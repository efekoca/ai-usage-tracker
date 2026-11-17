//! The handoff note, as the app runs it: the hook in Claude Code's settings with its write rule, the
//! app's own limit reading for conversations the status line does not reach, a notification once
//! the limit stops a conversation that wrote its note, and Codex started on the note in a terminal.
//! The app never reads a note; it opens it or passes its path on.

use crate::settings::Settings;
use crate::state::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tracker_core::capture::claude_settings as cs;
use tracker_core::capture::handoff as ho;
use tracker_core::capture::refresh_hook as rh;
use tracker_core::model::Provider;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn dir(state: &AppState) -> PathBuf {
    ho::dir(&state.data_dir)
}

/// The app's folder, then the copies a packaged app (Claude desktop on Windows) gives the programs
/// it starts: the hook's records and the notes Claude writes from there land in those.
fn dirs(state: &AppState) -> Vec<PathBuf> {
    std::iter::once(state.data_dir.clone()).chain(crate::capture::package_data_dirs(&state.data_dir)).map(|d| ho::dir(&d)).collect()
}

/// Every note with the folder it is in, one per project: the latest written.
fn all_notes(state: &AppState) -> Vec<(PathBuf, ho::Note)> {
    let mut out: Vec<(PathBuf, ho::Note)> = Vec::new();
    for d in dirs(state) {
        for n in ho::notes(&d) {
            match out.iter_mut().find(|(_, o)| o.key == n.key) {
                Some(o) if o.1.written_ms < n.written_ms => *o = (d.clone(), n),
                Some(_) => {}
                None => out.push((d.clone(), n)),
            }
        }
    }
    out.sort_by_key(|(_, n)| std::cmp::Reverse(n.written_ms));
    out
}

pub fn installed(state: &AppState) -> bool {
    cs::load_state(&state.data_dir).handoff.as_ref().is_some_and(cs::refresh_hook_in_place)
}

/// Adds our hook and its write rule to Claude Code's settings, or brings them up to date. A record
/// of a hook the user took part of out, or of one in another settings file, is replaced cleanly.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let _one_at_a_time = crate::capture::state_lock();
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
    let command = crate::capture::handoff_hook_command(&crate::app_path()?);
    let file = crate::capture::claude_settings_file(&s);
    if st.handoff.as_ref().is_some_and(|c| c.settings_file != file || !cs::refresh_hook_in_place(c)) {
        cs::uninstall_hook(&mut st, cs::Hook::Handoff, &state.data_dir).map_err(|e| e.to_string())?;
    }
    if st.handoff.is_none() {
        cs::install_hook(&mut st, cs::Hook::Handoff, &state.data_dir, &file, &command, now_ms()).map_err(|e| e.to_string())?;
    } else {
        cs::refresh_hook(&mut st, cs::Hook::Handoff, &state.data_dir, &command).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Points the hook at this copy of the app when it was moved; debug builds leave it alone.
pub fn follow_app(state: &AppState) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(exe) = crate::app_path() else { return };
    let _one_at_a_time = crate::capture::state_lock();
    let mut st = cs::load_state(&state.data_dir);
    match cs::refresh_hook(&mut st, cs::Hook::Handoff, &state.data_dir, &crate::capture::handoff_hook_command(&exe)) {
        Ok(true) => log::info!("handoff hook brought up to date"),
        Ok(false) => {}
        Err(e) => log::warn!("could not update the handoff hook: {e}"),
    }
}

/// The app's Claude reading, while one Claude account is known: a conversation whose status line
/// the app does not see (Claude desktop, an IDE) is then still on the same account.
fn shared_reading(state: &AppState, s: &Settings, now: i64) -> Option<ho::Reading> {
    let store = state.db();
    if tracker_core::accounts::several(&store).map_or(true, |p| p.contains(&Provider::Anthropic)) {
        return None;
    }
    let book = state.book.read().unwrap();
    let views = tracker_core::analytics::limits_view(&store, &book, now, &s.thresholds).ok()?;
    let mut windows = serde_json::Map::new();
    let mut ts = 0;
    for v in views.iter().filter(|v| v.provider == Provider::Anthropic && v.limit_id.is_empty() && v.state == tracker_core::analytics::LimitState::Fresh) {
        let (Some(pct), Some(resets), Some(at)) = (v.used_pct, v.resets_at, v.observed_ms) else { continue };
        if v.window == "five_hour" || v.window == "seven_day" {
            windows.insert(v.window.clone(), serde_json::json!({ "used_percentage": pct, "resets_at": resets }));
            ts = ts.max(at);
        }
    }
    (!windows.is_empty()).then_some(ho::Reading { ts, windows })
}

/// Keeps the app's reading for the hook, drops what is old, and tells the user once a conversation
/// that wrote its note was stopped by the limit.
pub fn tick(app: &AppHandle, s: &Settings, now: i64) {
    let state = app.state::<AppState>();
    let all = dirs(&state);
    for d in &all {
        ho::expire(d, SystemTime::now());
    }
    if !s.limit_handoff {
        return;
    }
    // the hook in a packaged app still reads the app's own folder
    if let Err(e) = ho::write_shared(&dir(&state), shared_reading(&state, s, now).as_ref()) {
        log::warn!("handoff reading: {e}");
    }
    for d in &all {
        for t in ho::all_told(d).into_iter().filter(|t| !t.scripted && t.resets_ms > now) {
            // a conversation stopped after the user went on in Codex waits for no reset
            if t.handed {
                crate::resume::call_off(&state, &t.session);
                continue;
            }
            if t.notified {
                continue;
            }
            let written = ho::note(d, &t.key).is_some_and(|n| n.written_ms >= t.told_ms);
            // the limit that stopped it may be the other window, with its own reset
            let stop = state.db().limit_stops(&t.session, t.told_ms, now).ok().and_then(|v| v.into_iter().max_by_key(|x| x.resets_at));
            let (true, Some(stop)) = (written, stop) else { continue };
            let _ = ho::set_notified(d, &t);
            crate::alerts::handoff_ready(app, s, &t.key, &project_name(&state, s, &t.cwd), stop.resets_at.saturating_mul(1000));
        }
    }
}

/// The project's name as the app shows it; empty while names are hidden. A folder inside a
/// project is that project, so a hidden one stays hidden.
fn project_name(state: &AppState, s: &Settings, cwd: &Path) -> String {
    if s.hide_project_names {
        return String::new();
    }
    let within = state.db().projects().unwrap_or_default().into_iter().filter(|p| cwd.starts_with(&p.path)).max_by_key(|p| p.path.len());
    match within {
        Some(p) if p.hidden => String::new(),
        Some(p) => p.name,
        None => cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

#[derive(Debug, Serialize)]
pub struct NoteView {
    key: String,
    /// Empty while project names are hidden.
    project: String,
    written_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct HandoffInfo {
    installed: bool,
    settings_file: String,
    /// The folder Claude may write to.
    folder: String,
    codex_found: bool,
    notes: Vec<NoteView>,
}

fn info(state: &AppState) -> HandoffInfo {
    let s = state.settings.read().unwrap().clone();
    let notes = all_notes(state).into_iter().map(|(_, n)| NoteView { project: project_name(state, &s, &n.cwd), key: n.key, written_ms: n.written_ms }).collect();
    HandoffInfo {
        installed: installed(state),
        settings_file: crate::capture::claude_settings_file(&s).to_string_lossy().into_owned(),
        folder: ho::note_path(&dir(state), "x").parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        codex_found: crate::capture::codex_binary(&s).is_some(),
        notes,
    }
}

#[tauri::command]
pub async fn handoff_info(state: State<'_, AppState>) -> Result<HandoffInfo, String> {
    Ok(info(&state))
}

/// Turns the handoff note on or off, and with it our hook and its write rule in Claude Code's
/// settings. Only from the main window, which asks first. The notes stay until they expire.
#[tauri::command]
pub async fn set_limit_handoff(window: WebviewWindow, app: AppHandle, on: bool) -> Result<HandoffInfo, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let state = app.state::<AppState>();
    if on {
        install(&app)?;
    } else {
        let _one_at_a_time = crate::capture::state_lock();
        let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
        cs::uninstall_hook(&mut st, cs::Hook::Handoff, &state.data_dir).map_err(|e| e.to_string())?;
    }
    let s = state.update_settings(|s| s.limit_handoff = on)?;
    crate::capture::write_statusline_options(&state.data_dir, &s);
    let _ = app.emit("settings-changed", &s);
    Ok(info(&state))
}

/// The note and the folder it really is in.
fn note_of(state: &AppState, key: &str) -> Result<(PathBuf, ho::Note), String> {
    if !ho::valid_key(key) {
        return Err("no_note".into());
    }
    all_notes(state).into_iter().find(|(_, n)| n.key == key).ok_or_else(|| "no_note".into())
}

pub fn open_note(app: &AppHandle, key: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (dir, _) = note_of(&state, key)?;
    crate::open_path(&ho::note_path(&dir, key)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_handoff_note(window: WebviewWindow, app: AppHandle, key: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    open_note(&app, &key)
}

/// Starts Codex on the note in the project's folder. The Claude conversation that wrote it no
/// longer goes on by itself at the reset, so two agents never work on the same files.
pub fn continue_in_codex(app: &AppHandle, key: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let (note_dir, note) = note_of(&state, key)?;
    let codex = crate::capture::codex_binary(&s).ok_or("codex_not_found")?;
    if !note.cwd.is_dir() {
        return Err("folder_missing".into());
    }
    let prompt = ho::codex_prompt(&ho::note_path(&note_dir, key));
    launch(&dir(&state), key, &codex, &note.cwd, &prompt).map_err(|e| {
        log::warn!("codex terminal: {e}");
        "terminal_failed".to_owned()
    })?;
    // every conversation asked for this project's note, also one the limit stops only later
    for d in dirs(&state) {
        for session in ho::set_handed(&d, key) {
            crate::resume::call_off(&state, &session);
        }
    }
    if rh::valid_session(&note.session) {
        crate::resume::call_off(&state, &note.session);
    }
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn continue_handoff_in_codex(window: WebviewWindow, app: AppHandle, key: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    continue_in_codex(&app, &key)
}

/// "Delete all data" takes the notes and the records with it.
pub fn forget(state: &AppState) {
    for d in dirs(state) {
        if let Err(e) = ho::forget(&d) {
            log::warn!("could not delete the handoff notes: {e}");
        }
    }
}

/// `'…'` for `sh`, which takes everything inside literally.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A `.command` file opens in the user's terminal without asking to control another app.
#[cfg(target_os = "macos")]
fn launch(dir: &Path, key: &str, codex: &Path, cwd: &Path, prompt: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join(format!("codex-{key}.command"));
    // an npm-installed Codex needs its `node`, which a terminal's own PATH may lack
    let path = tracker_core::capture::child_path(codex).map(|p| format!("PATH={}; export PATH\n", sh_quote(&p.to_string_lossy()))).unwrap_or_default();
    let body = format!("#!/bin/sh\n{path}cd {} || exit 1\nexec {} {}\n", sh_quote(&cwd.to_string_lossy()), sh_quote(&codex.to_string_lossy()), sh_quote(prompt));
    std::fs::create_dir_all(dir)?;
    std::fs::write(&script, body)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    std::process::Command::new("/usr/bin/open").arg(&script).spawn().map(|_| ())
}

/// A PowerShell window of its own, in the project's folder.
#[cfg(windows)]
fn launch(_: &Path, _: &str, codex: &Path, cwd: &Path, prompt: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let ps = |s: &str| s.chars().flat_map(|c| if matches!(c, '\'' | '\u{2018}'..='\u{201B}') { vec![c, c] } else { vec![c] }).collect::<String>();
    std::process::Command::new(crate::system_exe(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoExit", "-Command", &format!("& '{}' '{}'", ps(&codex.to_string_lossy()), ps(prompt))])
        .current_dir(cwd)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|_| ())
}

/// The first terminal found; the folder and the program go in as arguments, never into the script.
#[cfg(target_os = "linux")]
fn launch(_: &Path, _: &str, codex: &Path, cwd: &Path, prompt: &str) -> std::io::Result<()> {
    let script = r#"cd "$1" || exit 1; exec "$2" "$3""#;
    let tail = ["sh".to_owned(), "-c".into(), script.into(), "sh".into(), cwd.to_string_lossy().into_owned(), codex.to_string_lossy().into_owned(), prompt.to_owned()];
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal found");
    for (term, flag) in [("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e"), ("xfce4-terminal", "-x"), ("xterm", "-e")] {
        let mut c = std::process::Command::new(term);
        c.arg(flag).args(&tail);
        crate::desktop_env(&mut c);
        // an npm-installed Codex needs its `node`
        if let Some(p) = tracker_core::capture::child_path(codex) {
            c.env("PATH", p);
        }
        match c.spawn() {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn a_quoted_argument_keeps_quotes_and_dollar_signs_literal() {
        let q = super::sh_quote("it's $HOME `x`");
        let out = std::process::Command::new("sh").args(["-c", &format!("printf %s {q}")]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "it's $HOME `x`");
    }
}
