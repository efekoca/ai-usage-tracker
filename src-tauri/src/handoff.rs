//! The handoff note, as the app runs it: the hook in Claude Code's settings with its write rule and
//! the hook in Codex's `hooks.json`, the app's own limit readings for conversations whose own the
//! hook cannot read, the notes Codex left in the temp folder moved over, a notification once the
//! limit stops a conversation that wrote its note, and the other tool started on the note in a
//! terminal. The app never reads a note; it opens it or passes its path on.

use crate::settings::Settings;
use crate::state::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tracker_core::capture::claude_settings as cs;
use tracker_core::capture::codex_hooks as ch;
use tracker_core::capture::handoff as ho;
use tracker_core::discovery::Env;
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

/// Every project's latest note with the folder it is in.
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

/// Points the hooks at this copy of the app when it was moved; debug builds leave them alone.
/// Codex asks the user to trust a changed hook again, so its own is changed only then.
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
    let command = crate::capture::codex_handoff_hook_command(&exe);
    let Some(old) = st.codex_handoff.clone().filter(|c| c.command != command) else { return };
    match ch::retarget(&old, &command, now_ms()) {
        Ok(None) => {}
        Ok(Some(c)) => {
            st.codex_handoff = Some(c);
            match cs::save_state(&state.data_dir, &st) {
                Ok(()) => log::info!("codex handoff hook brought up to date"),
                Err(e) => log::warn!("could not record the codex handoff hook: {e}"),
            }
        }
        Err(e) => log::warn!("could not update the codex handoff hook: {e}"),
    }
}

/// The Codex folder in use: the first that is there, else where Codex would make it.
fn codex_home(s: &Settings) -> Option<PathBuf> {
    let homes = tracker_core::discovery::codex_homes(&Env::from_system(), &s.extra_paths);
    homes.iter().find(|h| h.is_dir()).or(homes.first()).cloned()
}

/// Codex is on this computer: its folder or its program.
fn codex_here(s: &Settings) -> bool {
    codex_home(s).is_some_and(|h| h.is_dir()) || crate::capture::codex_binary(s).is_some()
}

/// Adds our hook to Codex's `hooks.json`, or brings it up to date. Codex runs it only once the user
/// trusted it there. Returns how many of the user's own hooks moved up in a file it left, as
/// [`uninstall_codex`] does.
pub fn install_codex(app: &AppHandle) -> Result<usize, String> {
    let _one_at_a_time = crate::capture::state_lock();
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let file = codex_home(&s).map(|h| ch::hooks_file(&h)).ok_or("codex_not_found")?;
    let command = crate::capture::codex_handoff_hook_command(&crate::app_path()?);
    let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
    // a hook in another file goes first; one of another copy of the app is changed where it is
    let moved = match st.codex_handoff.clone().filter(|c| c.file != file) {
        Some(old) => ch::uninstall(&old).map_err(codex_error)?,
        None => 0,
    };
    let previous = st.codex_handoff.clone().filter(|c| c.file == file);
    st.codex_handoff = Some(ch::install(&file, &command, previous.as_ref(), now_ms()).map_err(codex_error)?);
    cs::save_state(&state.data_dir, &st).map_err(|e| e.to_string())?;
    Ok(moved)
}

fn codex_error(e: cs::SettingsError) -> String {
    match e {
        cs::SettingsError::Unparseable(d) => format!("codex_hooks_unparseable:{d}"),
        e => e.to_string(),
    }
}

/// Takes our hook out of Codex's `hooks.json`; the record goes once it is out. Returns how many
/// of the user's own hooks moved up, which Codex asks about again.
pub(crate) fn uninstall_codex(data_dir: &Path, st: &mut cs::CaptureState) -> Result<usize, String> {
    let Some(c) = st.codex_handoff.clone() else { return Ok(0) };
    let moved = ch::uninstall(&c).map_err(codex_error)?;
    st.codex_handoff = None;
    cs::save_state(data_dir, st).map_err(|e| e.to_string())?;
    Ok(moved)
}

/// The app's reading of the tool's main limit, while one account of it is known: a conversation
/// whose own reading the hook cannot see (Claude desktop, an IDE, Codex's desktop chat) is then
/// still on the same account.
fn shared_reading(state: &AppState, s: &Settings, agent: ho::Agent, now: i64) -> Option<ho::Reading> {
    let (provider, main) = match agent {
        ho::Agent::Claude => (Provider::Anthropic, ""),
        ho::Agent::Codex => (Provider::OpenAI, tracker_core::accounts::MAIN_LIMIT),
    };
    let store = state.db();
    if tracker_core::accounts::several(&store).map_or(true, |p| p.contains(&provider)) {
        return None;
    }
    let book = state.book.read().unwrap();
    let views = tracker_core::analytics::limits_view(&store, &book, now, &s.thresholds).ok()?;
    let mut windows = serde_json::Map::new();
    let mut ts = 0;
    for v in views.iter().filter(|v| v.provider == provider && v.limit_id == main && v.state == tracker_core::analytics::LimitState::Fresh) {
        let (Some(pct), Some(resets), Some(at)) = (v.used_pct, v.resets_at, v.observed_ms) else { continue };
        if v.window == "five_hour" || v.window == "seven_day" {
            windows.insert(v.window.clone(), serde_json::json!({ "used_percentage": pct, "resets_at": resets }));
            ts = ts.max(at);
        }
    }
    (!windows.is_empty()).then_some(ho::Reading { ts, windows })
}

/// Keeps the app's readings for the hooks, moves over the notes Codex left in the temp folder,
/// drops what is old, and tells the user once a conversation that wrote its note was stopped by
/// the limit.
pub fn tick(app: &AppHandle, s: &Settings, now: i64) {
    let state = app.state::<AppState>();
    let all = dirs(&state);
    for d in &all {
        ho::expire(d, SystemTime::now());
        ho::adopt_project_notes(d);
    }
    ho::clear_drops(SystemTime::now(), false);
    if !s.limit_handoff && !s.codex_handoff {
        return;
    }
    // the hook in a packaged app still reads the app's own folder
    for (agent, on) in [(ho::Agent::Claude, s.limit_handoff), (ho::Agent::Codex, s.codex_handoff)] {
        let reading = on.then(|| shared_reading(&state, s, agent, now)).flatten();
        if let Err(e) = ho::write_shared(&dir(&state), agent, reading.as_ref()) {
            log::warn!("handoff reading: {e}");
        }
    }
    for d in &all {
        let told: Vec<ho::Told> = ho::all_told(d).into_iter().filter(|t| !t.scripted).collect();
        // no hook may run after the tool call that wrote it, once the limit stops the turn or the
        // window resets
        for t in told.iter().filter(|t| t.agent == ho::Agent::Codex) {
            ho::collect(d, t);
        }
        for t in told.into_iter().filter(|t| t.resets_ms > now) {
            // a conversation stopped after the user went on in the other tool waits for no reset
            if t.handed {
                crate::resume::call_off(&state, &t.session);
                continue;
            }
            if t.notified {
                continue;
            }
            // the limit that stopped it may be the other window, with its own reset
            let stop = state.db().limit_stops(&t.session, t.told_ms, now).ok().and_then(|v| v.into_iter().max_by_key(|x| x.resets_at));
            let (true, Some(stop)) = (ho::answered(d, &t), stop) else { continue };
            let _ = ho::set_notified(d, &t);
            crate::alerts::handoff_ready(app, s, &t.session, &project_name(&state, s, &t.cwd), stop.resets_at.saturating_mul(1000), t.agent);
        }
    }
}

/// The project `cwd` is in, the innermost when projects nest. Compared as the archive keys
/// projects, so another spelling of the folder (letter case on Windows) is the same project.
fn project_for(projects: Vec<tracker_core::store::ProjectRow>, cwd: &str) -> Option<tracker_core::store::ProjectRow> {
    let key = tracker_core::store::normalize_project_path;
    let c = key(cwd);
    projects
        .into_iter()
        .filter(|p| {
            let k = key(&p.path);
            c == k || c.strip_prefix(k.as_str()).is_some_and(|rest| rest.starts_with(['/', '\\']))
        })
        .max_by_key(|p| p.path.len())
}

/// The project's name as the app shows it; empty while names are hidden. A folder inside a
/// project is that project, so a hidden one stays hidden.
fn project_name(state: &AppState, s: &Settings, cwd: &Path) -> String {
    if s.hide_project_names {
        return String::new();
    }
    match project_for(state.db().projects().unwrap_or_default(), &cwd.to_string_lossy()) {
        Some(p) if p.hidden => String::new(),
        Some(p) => p.name,
        None => cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

#[derive(Debug, Serialize)]
pub struct NoteView {
    /// The conversation that wrote it.
    id: String,
    /// Empty while project names are hidden.
    project: String,
    written_ms: i64,
    /// The tool that wrote it; the user goes on in the other one.
    agent: ho::Agent,
}

#[derive(Debug, Serialize)]
pub struct HandoffInfo {
    installed: bool,
    settings_file: String,
    /// The Claude Code setting that keeps the hook from running, if one does.
    hooks_off: Option<String>,
    /// The folder Claude may write to.
    folder: String,
    codex_found: bool,
    claude_found: bool,
    /// Codex's folder or program is on this computer.
    codex_here: bool,
    codex_installed: bool,
    codex_hooks_file: String,
    /// The hook ran in a Codex conversation since it was added, so the user trusted it.
    codex_seen: bool,
    /// Turning Codex off just now moved this many of the user's own hooks up in `hooks.json`, so
    /// Codex asks to trust them again; 0 in every other answer.
    codex_moved: usize,
    /// Where Codex writes before the note is moved over.
    codex_folder: String,
    notes: Vec<NoteView>,
}

fn info(state: &AppState) -> HandoffInfo {
    let s = state.settings.read().unwrap().clone();
    let notes = all_notes(state).into_iter().map(|(_, n)| NoteView { project: project_name(state, &s, &n.cwd), id: n.session, written_ms: n.written_ms, agent: n.agent }).collect();
    let codex = cs::load_state(&state.data_dir).codex_handoff;
    // a packaged Codex on Windows leaves it in its own copy of the folder
    let seen = dirs(state).iter().filter_map(|d| ho::codex_seen(d)).max();
    HandoffInfo {
        installed: installed(state),
        settings_file: crate::capture::claude_settings_file(&s).to_string_lossy().into_owned(),
        hooks_off: crate::capture::hooks_blocked(&s),
        folder: ho::note_path(&dir(state), "x").parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        codex_found: crate::capture::codex_binary(&s).is_some(),
        claude_found: crate::capture::claude_binary(&s).is_some(),
        codex_here: codex_here(&s),
        codex_installed: codex.as_ref().is_some_and(ch::in_place),
        codex_hooks_file: codex_home(&s).map(|h| ch::hooks_file(&h).to_string_lossy().into_owned()).unwrap_or_default(),
        codex_seen: codex.as_ref().zip(seen).is_some_and(|(c, at)| at >= c.installed_at_ms),
        codex_moved: 0,
        codex_folder: ho::drop_dir().to_string_lossy().into_owned(),
        notes,
    }
}

/// Off the async workers: finding Codex can run `codex --version`.
#[tauri::command]
pub async fn handoff_info(app: AppHandle) -> Result<HandoffInfo, String> {
    tauri::async_runtime::spawn_blocking(move || info(&app.state::<AppState>())).await.map_err(|e| e.to_string())
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

/// Turns the handoff note on or off for Codex, and with it our hook in Codex's `hooks.json`. Only
/// from the main window, which asks first.
#[tauri::command]
pub async fn set_codex_handoff(window: WebviewWindow, app: AppHandle, on: bool) -> Result<HandoffInfo, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let state = app.state::<AppState>();
    let moved = if on {
        install_codex(&app)?
    } else {
        let _one_at_a_time = crate::capture::state_lock();
        let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
        uninstall_codex(&state.data_dir, &mut st)?
    };
    let s = state.update_settings(|s| s.codex_handoff = on)?;
    crate::capture::write_statusline_options(&state.data_dir, &s);
    let _ = app.emit("settings-changed", &s);
    Ok(HandoffInfo { codex_moved: moved, ..info(&state) })
}

/// The note a conversation wrote and the folder it really is in.
fn note_of(state: &AppState, id: &str) -> Result<(PathBuf, ho::Note), String> {
    dirs(state).into_iter().filter_map(|d| ho::note(&d, id).map(|n| (d, n))).max_by_key(|(_, n)| n.written_ms).ok_or_else(|| "no_note".into())
}

pub fn open_note(app: &AppHandle, id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (dir, _) = note_of(&state, id)?;
    crate::open_path(&ho::note_path(&dir, id)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_handoff_note(window: WebviewWindow, app: AppHandle, id: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    open_note(&app, &id)
}

/// Starts Codex on the note in the project's folder.
pub fn continue_in_codex(app: &AppHandle, id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let (note_dir, note) = note_of(&state, id)?;
    let codex = crate::capture::codex_binary(&s).ok_or("codex_not_found")?;
    let prompt = ho::continue_prompt(&ho::note_path(&note_dir, id));
    let args = if tracker_core::capture::codex_limits::has_no_daemon(&codex) { vec!["--no-daemon".into(), prompt] } else { vec![prompt] };
    start(app, "codex", &codex, &note, &args)
}

/// Starts Claude Code on the note in the project's folder; the notes folder is added to the folders
/// it may read, so it does not ask first.
pub fn continue_in_claude(app: &AppHandle, id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let (note_dir, note) = note_of(&state, id)?;
    let claude = crate::capture::claude_binary(&s).ok_or("claude_not_found")?;
    let path = ho::note_path(&note_dir, id);
    let folder = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    // `--add-dir` takes every word after it, so the prompt goes first
    start(app, "claude", &claude, &note, &[ho::continue_prompt(&path), "--add-dir".into(), folder])
}

/// The other tool than the one that wrote the note.
pub fn continue_in_other(app: &AppHandle, id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    match note_of(&state, id)?.1.agent {
        ho::Agent::Claude => continue_in_codex(app, id),
        ho::Agent::Codex => continue_in_claude(app, id),
    }
}

/// Opens the terminal; the conversations asked for the project's notes no longer go on by
/// themselves at the reset, so two agents never work on the same files.
fn start(app: &AppHandle, tool: &str, bin: &Path, note: &ho::Note, args: &[String]) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !note.cwd.is_dir() {
        return Err("folder_missing".into());
    }
    launch(&dir(&state), &format!("{tool}-{}", note.session), bin, &note.cwd, args).map_err(|e| {
        log::warn!("{tool} terminal: {e}");
        "terminal_failed".to_owned()
    })?;
    // every conversation asked in this project, also one the limit stops only later
    for d in dirs(&state) {
        for session in ho::set_handed(&d, &note.key) {
            crate::resume::call_off(&state, &session);
        }
    }
    crate::resume::call_off(&state, &note.session);
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn continue_handoff(window: WebviewWindow, app: AppHandle, id: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    tauri::async_runtime::spawn_blocking(move || continue_in_other(&app, &id)).await.map_err(|e| e.to_string())?
}

/// "Delete all data" takes the notes and the records with it, and what Codex left in the temp folder.
pub fn forget(state: &AppState) {
    for d in dirs(state) {
        if let Err(e) = ho::forget(&d) {
            log::warn!("could not delete the handoff notes: {e}");
        }
    }
    ho::clear_drops(SystemTime::now(), true);
}

/// `'…'` for `sh`, which takes everything inside literally.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A `.command` file opens in the user's terminal without asking to control another app.
#[cfg(target_os = "macos")]
fn launch(dir: &Path, name: &str, bin: &Path, cwd: &Path, args: &[String]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join(format!("{name}.command"));
    // an npm-installed CLI needs its `node`, which a terminal's own PATH may lack
    // ahead of the terminal's own PATH, which keeps the user's other tools
    let path = tracker_core::capture::child_path(bin).map(|p| format!("PATH={}:\"$PATH\"; export PATH\n", sh_quote(&p.to_string_lossy()))).unwrap_or_default();
    let args: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
    let body = format!("#!/bin/sh\n{path}cd {} || exit 1\nexec {} {}\n", sh_quote(&cwd.to_string_lossy()), sh_quote(&bin.to_string_lossy()), args.join(" "));
    std::fs::create_dir_all(dir)?;
    std::fs::write(&script, body)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    crate::spawn_reaped(std::process::Command::new("/usr/bin/open").arg(&script)).map(|_| ())
}

/// A PowerShell window of its own, in the project's folder.
#[cfg(windows)]
fn launch(_: &Path, _: &str, bin: &Path, cwd: &Path, args: &[String]) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let ps = |s: &str| format!("'{}'", s.chars().flat_map(|c| if matches!(c, '\'' | '\u{2018}'..='\u{201B}') { vec![c, c] } else { vec![c] }).collect::<String>());
    let args: Vec<String> = args.iter().map(|a| ps(a)).collect();
    std::process::Command::new(crate::system_exe(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoExit", "-Command", &format!("& {} {}", ps(&bin.to_string_lossy()), args.join(" "))])
        .current_dir(cwd)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|_| ())
}

/// The first terminal found; the folder, the program and its words go in as arguments, never into
/// the script.
#[cfg(target_os = "linux")]
fn launch(_: &Path, _: &str, bin: &Path, cwd: &Path, args: &[String]) -> std::io::Result<()> {
    let script = r#"cd "$1" || exit 1; shift; exec "$@""#;
    let mut tail = vec!["sh".to_owned(), "-c".into(), script.into(), "sh".into(), cwd.to_string_lossy().into_owned(), bin.to_string_lossy().into_owned()];
    tail.extend(args.iter().cloned());
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal found");
    for (term, flag) in [("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e"), ("xfce4-terminal", "-x"), ("xterm", "-e")] {
        let mut c = std::process::Command::new(term);
        c.arg(flag).args(&tail);
        crate::desktop_env(&mut c);
        // an npm-installed CLI needs its `node`
        if let Some(p) = tracker_core::capture::child_path(bin) {
            c.env("PATH", p);
        }
        match crate::spawn_reaped(&mut c) {
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

    #[test]
    fn a_hidden_project_stays_hidden_in_another_spelling_of_its_folder() {
        let row = |path: &str, hidden| tracker_core::store::ProjectRow { id: 1, path: path.into(), name: "Proj".into(), hidden };
        assert!(super::project_for(vec![row(r"C:\Code\Proj", true)], r"c:\code\proj\sub").is_some_and(|p| p.hidden));
        assert!(super::project_for(vec![row(r"C:\Code\Proj", true)], r"c:\code\project").is_none(), "only a whole folder name counts");
        assert_eq!(super::project_for(vec![row("/w/a", false), row("/w/a/b", false)], "/w/a/b/c").map(|p| p.path), Some("/w/a/b".into()));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_the_app_starts_and_leaves_leaves_no_zombie() {
        let pid = crate::spawn_reaped(&mut std::process::Command::new("/usr/bin/true")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let ps = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
        assert!(!String::from_utf8_lossy(&ps.stdout).contains('Z'), "{}", String::from_utf8_lossy(&ps.stdout));
    }
}
