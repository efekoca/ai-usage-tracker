//! The status-line bridge runs as a separate short-lived process (`--statusline`, see `statusline_main`).

use crate::settings::Settings;
use crate::state::AppState;
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tracker_core::capture::claude_settings::{self as cs, CaptureState, RevertOutcome};
use tracker_core::capture::{claude_usage, codex_limits, otlp, statusline};
use tracker_core::discovery::{self, Env, SourceId};
use tracker_core::store::Store;

#[derive(Debug, Clone, Default, Serialize)]
pub struct PollStatus {
    pub binary: Option<String>,
    pub last_ok_ms: Option<i64>,
    pub last_error: Option<String>,
}

pub struct CaptureRuntime {
    pub receiver: Mutex<Option<otlp::Receiver>>,
    pub receiver_error: Mutex<Option<String>>,
    pub codex: Mutex<PollStatus>,
    codex_wake: Mutex<Option<Sender<()>>>,
    pub claude: Mutex<PollStatus>,
    claude_wake: Mutex<Option<Sender<()>>>,
}

impl CaptureRuntime {
    pub fn new() -> Self {
        CaptureRuntime {
            receiver: Mutex::new(None),
            receiver_error: Mutex::new(None),
            codex: Mutex::new(PollStatus::default()),
            codex_wake: Mutex::new(None),
            claude: Mutex::new(PollStatus::default()),
            claude_wake: Mutex::new(None),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn claude_settings_file(s: &Settings) -> PathBuf {
    let env = Env::from_system();
    let roots = discovery::claude_config_roots(&env, &s.extra_paths);
    let root = roots.iter().find(|r| r.is_dir()).cloned().or_else(|| roots.first().cloned()).unwrap_or_else(|| PathBuf::from(".claude"));
    cs::settings_path(&root)
}

#[cfg(windows)]
fn short_path(p: &Path) -> Option<String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
    (n > 0 && (n as usize) < buf.len()).then(|| std::ffi::OsString::from_wide(&buf[..n as usize]).to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn short_path(_: &Path) -> Option<String> {
    None
}

/// Claude Code prefers Git Bash for status-line commands on Windows.
pub fn git_bash() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CLAUDE_CODE_GIT_BASH_PATH")
        && Path::new(&p).is_file()
    {
        return Some(PathBuf::from(p));
    }
    let mut c: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            c.push(base.join("Git").join("bin").join("bash.exe"));
            c.push(base.join("Programs").join("Git").join("bin").join("bash.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Must work under both Git Bash and PowerShell, whichever Claude Code uses.
pub fn statusline_command(exe: &Path) -> String {
    let fwd = |s: &str| s.replace('\\', "/");
    if let Some(short) = short_path(exe).filter(|s| !s.contains(' ')) {
        return format!("{} --statusline", fwd(&short));
    }
    let full = fwd(&exe.to_string_lossy());
    if git_bash().is_some() { format!("\"{full}\" --statusline") } else { format!("& \"{full}\" --statusline") }
}

/// Run by Claude Code as its status-line command; chains to the user's previous one if any.
pub fn statusline_main() -> i32 {
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return 0 };
    let mut input = Vec::new();
    let _ = std::io::stdin().take(4 * 1024 * 1024).read_to_end(&mut input);
    let json: serde_json::Value = serde_json::from_slice(&input).unwrap_or(serde_json::Value::Null);
    let windows = statusline::extract_windows(&json);
    if let Some(w) = &windows {
        statusline::record(&data_dir, w, now_ms());
    }
    let state = cs::load_state(&data_dir);
    let previous = state.statusline.and_then(|s| s.previous);
    let out = match previous.as_ref().and_then(|p| p.get("command")).and_then(|c| c.as_str()) {
        Some(cmd) => run_chained(cmd, &input).unwrap_or_default(),
        None => statusline::render_default(windows.as_ref()),
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
    0
}

fn run_chained(cmd: &str, input: &[u8]) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut c = match git_bash() {
        Some(bash) => {
            let mut c = Command::new(bash);
            c.arg("-c").arg(cmd);
            c
        }
        None => {
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-Command", cmd]);
            c
        }
    };
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.spawn().ok()?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(input);
    }
    let mut out = String::new();
    let mut so = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let _ = so.read_to_string(&mut out);
        out
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                break;
            }
        }
    }
    reader.join().ok()
}

/// `--revert-capture`, run by the uninstaller: undoes every change made outside the app's folder.
pub fn revert_all_main() -> i32 {
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return 0 };
    let mut state = cs::load_state(&data_dir);
    if let Some(ch) = state.statusline.take() {
        let _ = cs::revert_statusline(&ch);
    }
    if let Some(ch) = state.otel.take() {
        let _ = cs::revert_otel(&ch);
    }
    let _ = cs::save_state(&data_dir, &state);
    if let Ok(store) = Store::open(&data_dir.join("tracker.db")) {
        let mut s = Settings::load(&store);
        s.capture.statusline = false;
        s.capture.otel = false;
        s.capture.codex_poll = false;
        s.capture.claude_poll = false;
        let _ = s.save(&store);
    }
    0
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureStatus {
    pub codex_poll: bool,
    pub codex: PollStatus,
    pub codex_candidates_found: bool,
    pub claude_poll: bool,
    pub claude: PollStatus,
    pub claude_candidates_found: bool,
    pub statusline: bool,
    pub statusline_file: String,
    pub statusline_chained: bool,
    pub statusline_last_ms: Option<i64>,
    pub otel: bool,
    pub otel_port: u16,
    pub otel_listening: bool,
    pub otel_events: u64,
    pub otel_last_ms: Option<i64>,
    pub otel_error: Option<String>,
    pub settings_file: String,
}

pub fn status(app: &AppHandle) -> CaptureStatus {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let st = cs::load_state(&state.data_dir);
    let rt = &state.capture;
    let recv = rt.receiver.lock().unwrap();
    let last_line = std::fs::metadata(statusline::capture_file(&state.data_dir)).and_then(|m| m.modified()).ok();
    CaptureStatus {
        codex_poll: s.capture.codex_poll,
        codex: rt.codex.lock().unwrap().clone(),
        codex_candidates_found: codex_binary(&s).is_some(),
        claude_poll: s.capture.claude_poll,
        claude: rt.claude.lock().unwrap().clone(),
        claude_candidates_found: claude_binary(&s).is_some(),
        statusline: s.capture.statusline,
        statusline_file: st.statusline.as_ref().map(|c| c.settings_file.to_string_lossy().into_owned()).unwrap_or_default(),
        statusline_chained: st.statusline.as_ref().is_some_and(|c| c.previous.is_some()),
        statusline_last_ms: last_line.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64),
        otel: s.capture.otel,
        otel_port: s.capture.otel_port,
        otel_listening: recv.is_some(),
        otel_events: recv.as_ref().map(|r| r.stats.events.load(Ordering::Relaxed)).unwrap_or(0),
        otel_last_ms: recv.as_ref().map(|r| r.stats.last_event_ms.load(Ordering::Relaxed)).filter(|v| *v > 0),
        otel_error: rt.receiver_error.lock().unwrap().clone(),
        settings_file: claude_settings_file(&s).to_string_lossy().into_owned(),
    }
}

fn codex_binary(s: &Settings) -> Option<PathBuf> {
    let configured = (!s.capture.codex_path.trim().is_empty()).then(|| PathBuf::from(s.capture.codex_path.trim()));
    codex_limits::find_codex(&Env::from_system(), &s.extra_paths, configured.as_deref())
}

fn claude_binary(s: &Settings) -> Option<PathBuf> {
    let configured = (!s.capture.claude_path.trim().is_empty()).then(|| PathBuf::from(s.capture.claude_path.trim()));
    claude_usage::find_claude(&Env::from_system(), configured.as_deref())
}

/// Runs in an empty folder of the app's own so no project settings, files or trust prompts apply.
fn read_claude(state: &AppState, s: &Settings, bin: &Path) -> Result<Vec<tracker_core::model::LimitSnapshot>, String> {
    let work = state.data_dir.join("claude-usage");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let env = Env::from_system();
    let cfg = claude_usage::config_dir(&env, &s.extra_paths);
    claude_usage::query(bin, &work, cfg.as_deref(), Duration::from_secs(30), now_ms()).map_err(|e| e.to_string())
}

fn persist(app: &AppHandle, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let state = app.state::<AppState>();
    let mut s = state.settings.read().unwrap().clone();
    f(&mut s);
    s.save(&state.store.lock().unwrap())?;
    *state.settings.write().unwrap() = s.clone();
    let _ = app.emit("settings-changed", &s);
    Ok(s)
}

fn outcome_text(o: RevertOutcome) -> &'static str {
    match o {
        RevertOutcome::Restored => "restored",
        RevertOutcome::LeftUserValue => "left_user_value",
        RevertOutcome::NothingToDo => "nothing_to_do",
    }
}

pub fn set(app: &AppHandle, kind: &str, on: bool) -> Result<String, String> {
    let state = app.state::<AppState>();
    let data_dir = state.data_dir.clone();
    let settings = state.settings.read().unwrap().clone();
    let mut cstate: CaptureState = cs::load_state(&data_dir);
    match (kind, on) {
        ("codex", true) => {
            let bin = codex_binary(&settings).ok_or("codex_not_found")?;
            let snaps = codex_limits::query(&bin, Duration::from_secs(20), now_ms())?;
            store_limits(&state.db_path, &snaps)?;
            *state.capture.codex.lock().unwrap() =
                PollStatus { binary: Some(bin.to_string_lossy().into_owned()), last_ok_ms: Some(now_ms()), last_error: None };
            persist(app, |s| s.capture.codex_poll = true)?;
            wake_codex(app);
            let _ = app.emit("data-changed", ());
            Ok("enabled".into())
        }
        ("codex", false) => {
            persist(app, |s| s.capture.codex_poll = false)?;
            Ok("disabled".into())
        }
        ("claude", true) => {
            let bin = claude_binary(&settings).ok_or("claude_not_found")?;
            match read_claude(&state, &settings, &bin) {
                Ok(snaps) => store_limits(&state.db_path, &snaps)?,
                // signed in, just asked too soon: the next cycle reads the limits
                Err(e) if e == "claude_throttled" => {}
                Err(e) => return Err(e),
            }
            *state.capture.claude.lock().unwrap() =
                PollStatus { binary: Some(bin.to_string_lossy().into_owned()), last_ok_ms: Some(now_ms()), last_error: None };
            persist(app, |s| s.capture.claude_poll = true)?;
            wake_claude(app);
            let _ = app.emit("data-changed", ());
            Ok("enabled".into())
        }
        ("claude", false) => {
            persist(app, |s| s.capture.claude_poll = false)?;
            Ok("disabled".into())
        }
        ("statusline", true) => {
            if cstate.statusline.is_some() {
                persist(app, |s| s.capture.statusline = true)?;
                return Ok("already".into());
            }
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let file = claude_settings_file(&settings);
            let ch = cs::install_statusline(&file, &data_dir, &statusline_command(&exe), now_ms()).map_err(|e| e.to_string())?;
            let chained = ch.previous.is_some();
            cstate.statusline = Some(ch);
            cs::save_state(&data_dir, &cstate).map_err(|e| e.to_string())?;
            persist(app, |s| s.capture.statusline = true)?;
            Ok(if chained { "enabled_chained".into() } else { "enabled".into() })
        }
        ("statusline", false) => {
            let note = match cstate.statusline.take() {
                Some(ch) => outcome_text(cs::revert_statusline(&ch).map_err(|e| e.to_string())?),
                None => "nothing_to_do",
            };
            cs::save_state(&data_dir, &cstate).map_err(|e| e.to_string())?;
            persist(app, |s| s.capture.statusline = false)?;
            Ok(note.into())
        }
        ("otel", true) => {
            let conflict = cs::conflicting_process_env();
            if !conflict.is_empty() {
                return Err(format!("telemetry_env_conflict:{}", conflict.join(",")));
            }
            let port = settings.capture.otel_port;
            start_receiver(app, port).map_err(|e| format!("port_busy:{port}:{e}"))?;
            if cstate.otel.is_none() {
                let file = claude_settings_file(&settings);
                match cs::install_otel(&file, &data_dir, port, now_ms()) {
                    Ok(ch) => cstate.otel = Some(ch),
                    Err(e) => {
                        stop_receiver(app);
                        return Err(match e {
                            cs::SettingsError::TelemetryConflict(k) => format!("telemetry_settings_conflict:{k}"),
                            other => other.to_string(),
                        });
                    }
                }
                cs::save_state(&data_dir, &cstate).map_err(|e| e.to_string())?;
            }
            persist(app, |s| s.capture.otel = true)?;
            Ok("enabled".into())
        }
        ("otel", false) => {
            let note = match cstate.otel.take() {
                Some(ch) => outcome_text(cs::revert_otel(&ch).map_err(|e| e.to_string())?),
                None => "nothing_to_do",
            };
            cs::save_state(&data_dir, &cstate).map_err(|e| e.to_string())?;
            stop_receiver(app);
            persist(app, |s| s.capture.otel = false)?;
            Ok(note.into())
        }
        _ => Err("unknown capture kind".into()),
    }
}

fn store_limits(db: &Path, snaps: &[tracker_core::model::LimitSnapshot]) -> Result<(), String> {
    let mut store = Store::open(db).map_err(|e| e.to_string())?;
    let mut tx = store.transaction().map_err(|e| e.to_string())?;
    tx.insert_limits(snaps).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn start_receiver(app: &AppHandle, port: u16) -> std::io::Result<()> {
    let state = app.state::<AppState>();
    let mut slot = state.capture.receiver.lock().unwrap();
    if slot.as_ref().is_some_and(|r| r.port == port) {
        return Ok(());
    }
    *slot = None;
    let handle = app.clone();
    match otlp::Receiver::start(port, state.db_path.clone(), move |_| {
        let _ = handle.emit("data-changed", ());
    }) {
        Ok(r) => {
            *slot = Some(r);
            *state.capture.receiver_error.lock().unwrap() = None;
            Ok(())
        }
        Err(e) => {
            *state.capture.receiver_error.lock().unwrap() = Some(e.to_string());
            Err(e)
        }
    }
}

pub fn stop_receiver(app: &AppHandle) {
    let state = app.state::<AppState>();
    let r = state.capture.receiver.lock().unwrap().take();
    drop(r);
}

/// No background reads before onboarding is confirmed, nor for providers the user disabled.
fn reads_allowed(s: &Settings, sources: &[SourceId]) -> bool {
    s.onboarded && sources.iter().any(|id| s.enabled_sources.contains(id))
}

pub fn wake_readers(app: &AppHandle) {
    wake_claude(app);
    wake_codex(app);
}

fn wake_claude(app: &AppHandle) {
    if let Some(tx) = app.state::<AppState>().capture.claude_wake.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// `tool = 'claude_code'` also covers Cowork logs.
fn latest_claude_event_ms(state: &AppState) -> Option<i64> {
    let store = state.store.lock().ok()?;
    store
        .conn()
        .query_row("SELECT MAX(ts_ms) FROM usage_event WHERE tool = 'claude_code'", [], |r| r.get::<_, Option<i64>>(0))
        .ok()
        .flatten()
}

/// Claude's usage service throttles callers, so reads stay >= 5 min apart. Idle reads still run
/// every 15 min because web and desktop chat use leaves no local log.
fn start_claude_poller(app: &AppHandle) {
    let (tx, rx) = channel::<()>();
    *app.state::<AppState>().capture.claude_wake.lock().unwrap() = Some(tx);
    let app = app.clone();
    let _ = std::thread::Builder::new().name("claude-limits".into()).spawn(move || {
        const TICK: Duration = Duration::from_secs(30);
        const ACTIVE_GAP_MS: i64 = 5 * 60_000;
        const IDLE_GAP_MS: i64 = 15 * 60_000;
        const MAX_BACKOFF_MS: i64 = 30 * 60_000;
        let mut last_try_ms: i64 = 0;
        // consecutive real failures (not throttling): 5, 10, 20, 30 minutes apart
        let mut failures: u32 = 0;
        let mut seen_event_ms: Option<i64> = None;
        let mut forced = false;
        loop {
            match rx.recv_timeout(if last_try_ms == 0 { Duration::from_secs(25) } else { TICK }) {
                Ok(()) => forced = true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let state = app.state::<AppState>();
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            let s = state.settings.read().unwrap().clone();
            if !s.capture.claude_poll || !reads_allowed(&s, &[SourceId::ClaudeCode, SourceId::Cowork, SourceId::ClaudeDesktop]) {
                forced = false;
                continue;
            }
            let now = now_ms();
            let active_gap = (60_000 * s.capture.claude_poll_minutes.clamp(5, 120) as i64).max(ACTIVE_GAP_MS);
            let latest = latest_claude_event_ms(&state);
            let new_activity = latest.is_some() && latest != seen_event_ms;
            let since = now - last_try_ms;
            let due = if failures > 0 {
                forced || since >= (ACTIVE_GAP_MS << (failures - 1).min(3)).min(MAX_BACKOFF_MS)
            } else {
                forced || since >= IDLE_GAP_MS.max(active_gap) || (new_activity && since >= active_gap)
            };
            if !due {
                continue;
            }
            forced = false;
            last_try_ms = now;
            seen_event_ms = latest;
            let Some(bin) = claude_binary(&s) else {
                failures = failures.saturating_add(1);
                state.capture.claude.lock().unwrap().last_error = Some("claude_not_found".into());
                continue;
            };
            match read_claude(&state, &s, &bin).and_then(|snaps| store_limits(&state.db_path, &snaps)) {
                Ok(()) => {
                    failures = 0;
                    let mut c = state.capture.claude.lock().unwrap();
                    c.binary = Some(bin.to_string_lossy().into_owned());
                    c.last_ok_ms = Some(now_ms());
                    c.last_error = None;
                    drop(c);
                    let _ = app.emit("data-changed", ());
                }
                // throttled: keep the last reading, no error shown
                Err(e) if e == "claude_throttled" => {}
                Err(e) => {
                    failures = failures.saturating_add(1);
                    let mut c = state.capture.claude.lock().unwrap();
                    if c.last_error.as_deref() != Some(e.as_str()) {
                        log::warn!("claude limit read failed: {e}");
                    }
                    c.last_error = Some(e);
                }
            }
        }
    });
}

fn wake_codex(app: &AppHandle) {
    if let Some(tx) = app.state::<AppState>().capture.codex_wake.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

pub fn start(app: &AppHandle) {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    if s.capture.otel
        && let Err(e) = start_receiver(app, s.capture.otel_port)
    {
        log::warn!("otlp receiver could not start: {e}");
    }
    start_claude_poller(app);
    let (tx, rx) = channel::<()>();
    *app.state::<AppState>().capture.codex_wake.lock().unwrap() = Some(tx);
    let app = app.clone();
    let _ = std::thread::Builder::new().name("codex-limits".into()).spawn(move || {
        let mut wait = Duration::from_secs(20);
        loop {
            match rx.recv_timeout(wait) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let state = app.state::<AppState>();
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            let s = state.settings.read().unwrap().clone();
            wait = Duration::from_secs(60 * s.capture.codex_poll_minutes.clamp(1, 120));
            if !s.capture.codex_poll || !reads_allowed(&s, &[SourceId::Codex]) {
                continue;
            }
            let Some(bin) = codex_binary(&s) else {
                state.capture.codex.lock().unwrap().last_error = Some("codex_not_found".into());
                wait = Duration::from_secs(30 * 60);
                continue;
            };
            match codex_limits::query(&bin, Duration::from_secs(20), now_ms()).and_then(|snaps| store_limits(&state.db_path, &snaps)) {
                Ok(()) => {
                    let mut c = state.capture.codex.lock().unwrap();
                    c.binary = Some(bin.to_string_lossy().into_owned());
                    c.last_ok_ms = Some(now_ms());
                    c.last_error = None;
                    drop(c);
                    let _ = app.emit("data-changed", ());
                }
                Err(e) => {
                    let mut c = state.capture.codex.lock().unwrap();
                    if c.last_error.as_deref() != Some(e.as_str()) {
                        log::warn!("codex limit read failed: {e}");
                    }
                    c.last_error = Some(e);
                }
            }
        }
    });
}
