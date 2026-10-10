//! The status-line bridge runs as a separate short-lived process (`--statusline`, see `statusline_main`).

use crate::settings::Settings;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tracker_core::capture::claude_settings::{self as cs, RevertOutcome};
use tracker_core::capture::{antigravity_limits, claude_usage, codex_limits, otlp, statusline, Cli};
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
    claude_last_read_ms: Mutex<i64>,
    pub antigravity: Mutex<PollStatus>,
    antigravity_wake: Mutex<Option<Sender<()>>>,
    /// Bumped by "delete all data": a limit read started before it is not stored.
    wipes: AtomicU64,
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
            claude_last_read_ms: Mutex::new(0),
            antigravity: Mutex::new(PollStatus::default()),
            antigravity_wake: Mutex::new(None),
            wipes: AtomicU64::new(0),
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
#[cfg(windows)]
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
    // Scoop keeps only a shim on PATH; the install itself lives under its apps folder
    let scoop = std::env::var_os("SCOOP").map(PathBuf::from).or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("scoop")));
    c.extend(scoop.map(|s| s.join(r"apps\git\current\bin\bash.exe")));
    // like Claude Code itself: the bash.exe that belongs to the git.exe on PATH
    for dir in std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default() {
        if dir.join("git.exe").is_file() {
            c.push(dir.join("bash.exe"));
            c.extend(dir.parent().map(|g| g.join(r"bin\bash.exe")));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Elsewhere Claude Code runs status-line commands with `sh`.
#[cfg(not(windows))]
pub fn git_bash() -> Option<PathBuf> {
    None
}

/// Must work under both Git Bash and PowerShell on Windows, whichever Claude Code uses.
pub fn statusline_command(exe: &Path) -> String {
    quoted_command(&exe.to_string_lossy(), short_path(exe).as_deref(), !cfg!(windows) || git_bash().is_some())
}

/// Single quotes: inside double quotes both shells would still expand `$` and backticks.
fn quoted_command(full: &str, short: Option<&str>, bash: bool) -> String {
    // only Windows paths are turned into forward slashes; a backslash is an ordinary file name character elsewhere
    let fwd = |s: &str| if cfg!(windows) { s.replace('\\', "/") } else { s.to_owned() };
    let plain = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "/:._-~".contains(c));
    if let Some(short) = short.map(fwd).filter(|s| plain(s)) {
        return format!("{short} --statusline");
    }
    let full = fwd(full);
    if bash { format!("'{}' --statusline", full.replace('\'', r"'\''")) } else { format!("& '{}' --statusline", full.replace('\'', "''")) }
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
    // a bridge run as someone's previous command (another copy of the app) does not chain again,
    // so two bridges can never call each other without end
    let previous = state.statusline.and_then(|s| s.previous).filter(|_| std::env::var_os(CHAINED).is_none());
    let out = match previous.as_ref().and_then(|p| p.get("command")).and_then(|c| c.as_str()) {
        Some(cmd) => run_chained(cmd, input).unwrap_or_default(),
        None => statusline::render_default(windows.as_ref()),
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
    0
}

/// Set for the user's previous status-line command.
const CHAINED: &str = "AIUT_STATUSLINE_CHAINED";

fn run_chained(cmd: &str, input: Vec<u8>) -> Option<String> {
    run_within(cmd, input, Duration::from_secs(5))
}

/// I/O goes through helper threads, so a stalled child or its descendants cannot outlast the deadline.
fn run_within(cmd: &str, input: Vec<u8>, deadline: Duration) -> Option<String> {
    use std::process::{Command, Stdio};
    const MAX_OUT: u64 = 64 * 1024;
    let mut c = match git_bash() {
        Some(bash) => {
            let mut c = Command::new(bash);
            c.arg("-c").arg(cmd);
            c
        }
        None if !cfg!(windows) => {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(cmd);
            c
        }
        None => {
            let mut c = Command::new(crate::system_exe(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
            c.args(["-NoProfile", "-NonInteractive", "-Command", cmd]);
            c
        }
    };
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).env(CHAINED, "1");
    // its own process group, so a timeout also ends what the command started
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    #[cfg(unix)]
    let mut child = c.spawn().ok()?;
    #[cfg(windows)]
    let (mut child, job) = Job::spawn(&mut c).ok()?;
    if let Some(mut si) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = si.write_all(&input);
        });
    }
    let so = child.stdout.take()?;
    let (tx, rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = so.take(MAX_OUT).read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(deadline).ok();
    let left = Instant::now() + Duration::from_millis(200);
    while matches!(child.try_wait(), Ok(None)) && Instant::now() < left {
        std::thread::sleep(Duration::from_millis(20));
    }
    // output still open at the deadline means something the command started is still running
    if out.is_none() || matches!(child.try_wait(), Ok(None)) {
        // Windows: the job also reaches what was started by a parent that has already exited
        #[cfg(windows)]
        if let Some(job) = &job {
            job.terminate();
        }
        tracker_core::capture::kill_tree(&mut child);
    }
    out.map(|o| String::from_utf8_lossy(&o).into_owned())
}

/// A Job Object holding a command and every process it starts.
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Job {
    /// Starts the command suspended and lets it run only once it is in the job, so nothing it
    /// starts can escape. Without a job (failure is logged) it runs as before, ended by `taskkill`.
    fn spawn(c: &mut std::process::Command) -> std::io::Result<(std::process::Child, Option<Job>)> {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // SAFETY: an unnamed job with default security; the handle is closed on drop
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        let mut job = if handle.is_null() {
            Self::failed("create");
            None
        } else {
            Some(Job(handle))
        };
        let mut child = c.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED).spawn()?;
        // SAFETY: both handles stay open for the call
        if let Some(j) = &job
            && unsafe { AssignProcessToJobObject(j.0, child.as_raw_handle()) } == 0
        {
            Self::failed("assign");
            job = None;
        }
        if let Err(e) = resume_suspended(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        Ok((child, job))
    }

    fn terminate(&self) {
        // SAFETY: the handle is open until drop
        if unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) } == 0 {
            Self::failed("terminate");
        }
    }

    fn failed(step: &str) -> std::io::Error {
        Self::report(step, std::io::Error::last_os_error())
    }

    fn report(step: &str, e: std::io::Error) -> std::io::Error {
        log::warn!("status line job {step} failed: {e}");
        e
    }
}

/// Resumes the main thread of a process started suspended.
#[cfg(windows)]
fn resume_suspended(pid: u32) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    // SAFETY: a thread snapshot, closed below; THREADENTRY32 is plain data with its size set
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Err(Job::failed("resume"));
    }
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = false;
    // read right after the failing call; the scan and cleanup below overwrite the last error
    let mut error = None;
    let mut more = unsafe { Thread32First(snap, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: a handle to one of the child's threads, closed right after
            let t = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if t.is_null() {
                error = Some(std::io::Error::last_os_error());
            } else {
                if unsafe { ResumeThread(t) } == u32::MAX {
                    error = Some(std::io::Error::last_os_error());
                } else {
                    resumed = true;
                }
                unsafe { CloseHandle(t) };
            }
        }
        more = unsafe { Thread32Next(snap, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snap) };
    if resumed {
        return Ok(());
    }
    Err(Job::report("resume", error.unwrap_or_else(|| std::io::Error::other(format!("no thread of process {pid} found")))))
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: owned handle, closed once
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// `--revert-capture`, run by the uninstaller: undoes every change made outside the app's folder.
pub fn revert_all_main() -> i32 {
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return 0 };
    let mut state = match cs::read_state(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ai-usage-tracker: {e}");
            return 2;
        }
    };
    let had = (state.statusline.is_some(), state.otel.is_some());
    let statusline_ok = cs::uninstall_statusline(&mut state, &data_dir).inspect_err(|e| eprintln!("ai-usage-tracker: status line: {e}")).is_ok();
    let otel_ok = cs::uninstall_otel(&mut state, &data_dir).inspect_err(|e| eprintln!("ai-usage-tracker: telemetry: {e}")).is_ok();
    let reverted = Reverted { statusline: had.0 && statusline_ok, otel: had.1 && otel_ok, at_ms: now_ms() };
    if reverted.statusline || reverted.otel {
        let _ = std::fs::write(reverted_file(&data_dir), serde_json::to_string(&reverted).unwrap_or_default());
    }
    if let Ok(store) = Store::open(&data_dir.join("tracker.db")) {
        let mut s = Settings::load(&store);
        s.capture.statusline &= !statusline_ok;
        s.capture.otel &= !otel_ok;
        let _ = s.save(&store);
    }
    if statusline_ok && otel_ok { 0 } else { 1 }
}

/// What an uninstaller turned off. A manual upgrade runs the old uninstaller first, so a start
/// soon after turns the same captures back on.
#[derive(Debug, Serialize, Deserialize)]
struct Reverted {
    statusline: bool,
    otel: bool,
    at_ms: i64,
}

const REINSTALL_WINDOW_MS: i64 = 60 * 60_000;

fn reverted_file(data_dir: &Path) -> PathBuf {
    data_dir.join("capture").join("reverted-by-uninstall.json")
}

impl Reverted {
    fn to_restore(&self, now: i64) -> Vec<&'static str> {
        if !(0..REINSTALL_WINDOW_MS).contains(&(now - self.at_ms)) {
            return Vec::new();
        }
        [("statusline", self.statusline), ("otel", self.otel)].into_iter().filter(|k| k.1).map(|k| k.0).collect()
    }
}

fn restore_after_upgrade(app: &AppHandle) {
    let file = reverted_file(&app.state::<AppState>().data_dir);
    let Ok(raw) = std::fs::read_to_string(&file) else { return };
    let _ = std::fs::remove_file(&file);
    let Ok(reverted) = serde_json::from_str::<Reverted>(&raw) else { return };
    for kind in reverted.to_restore(now_ms()) {
        match set(app, kind, true) {
            Ok(r) => log::info!("{kind} turned back on after the upgrade: {r}"),
            Err(e) => log::warn!("{kind} could not be turned back on after the upgrade: {e}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureStatus {
    pub codex_poll: bool,
    pub codex: PollStatus,
    pub codex_candidates_found: bool,
    pub claude_poll: bool,
    pub claude: PollStatus,
    pub claude_candidates_found: bool,
    pub antigravity_poll: bool,
    pub antigravity: PollStatus,
    pub antigravity_candidates_found: bool,
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
        antigravity_poll: s.capture.antigravity_poll,
        antigravity: rt.antigravity.lock().unwrap().clone(),
        antigravity_candidates_found: agy_binary(&s).is_some(),
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
    codex_limits::find_codex(&Env::from_system(), configured.as_deref())
}

fn agy_binary(s: &Settings) -> Option<PathBuf> {
    let configured = (!s.capture.antigravity_path.trim().is_empty()).then(|| PathBuf::from(s.capture.antigravity_path.trim()));
    antigravity_limits::find_agy(&Env::from_system(), configured.as_deref())
}

/// Installs a CLI with its maker's installer, then reads its limits.
pub fn install_cli(app: &AppHandle, kind: &str) -> Result<String, String> {
    type Find = fn(&Settings) -> Option<PathBuf>;
    let (cli, find, code): (Cli, Find, &str) = match kind {
        "claude" => (Cli::Claude, claude_binary, "claude"),
        "codex" => (Cli::Codex, codex_binary, "codex"),
        "antigravity" => (Cli::Antigravity, agy_binary, "agy"),
        _ => return Err(format!("unknown capture kind {kind}")),
    };
    let state = app.state::<AppState>();
    if find(&state.settings.read().unwrap()).is_none() {
        tracker_core::capture::install(cli, Duration::from_secs(15 * 60))?;
    }
    find(&state.settings.read().unwrap()).ok_or(format!("{code}_not_found"))?;
    persist(app, |s| match cli {
        Cli::Claude => s.capture.claude_poll = true,
        Cli::Codex => s.capture.codex_poll = true,
        Cli::Antigravity => s.capture.antigravity_poll = true,
    })?;
    match cli {
        Cli::Claude => wake_claude(app),
        Cli::Codex => wake_codex(app),
        Cli::Antigravity => wake_antigravity(app),
    }
    Ok(format!("{code}_installed"))
}

/// Runs in an empty folder of the app's own, never in one of the user's projects.
fn read_antigravity(state: &AppState, bin: &Path) -> Result<Vec<tracker_core::model::LimitSnapshot>, String> {
    let work = state.data_dir.join("antigravity-usage");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    antigravity_limits::query(bin, &work, Duration::from_secs(30), now_ms())
}

fn claude_binary(s: &Settings) -> Option<PathBuf> {
    let configured = (!s.capture.claude_path.trim().is_empty()).then(|| PathBuf::from(s.capture.claude_path.trim()));
    claude_usage::find_claude(&Env::from_system(), configured.as_deref())
}

/// Runs in an empty folder of the app's own so no project settings, files or trust prompts apply.
/// Claude throttles `get_usage` server-side, so reads are kept this far apart wherever they start.
const CLAUDE_MIN_GAP_MS: i64 = 5 * 60_000;

/// Takes the next read slot unless the previous read is less than `gap_ms` old.
fn claim_read(last_ms: &Mutex<i64>, now: i64, gap_ms: i64) -> bool {
    let mut last = last_ms.lock().unwrap();
    if *last != 0 && now - *last < gap_ms {
        return false;
    }
    *last = now;
    true
}

fn read_claude(state: &AppState, s: &Settings, bin: &Path) -> Result<Vec<tracker_core::model::LimitSnapshot>, String> {
    if !claim_read(&state.capture.claude_last_read_ms, now_ms(), CLAUDE_MIN_GAP_MS) {
        return Err("claude_throttled".into());
    }
    let work = state.data_dir.join("claude-usage");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let env = Env::from_system();
    let cfg = claude_usage::config_dir(&env, &s.extra_paths);
    claude_usage::query(bin, &work, cfg.as_deref(), Duration::from_secs(30), now_ms()).map_err(|e| e.to_string())
}

fn persist(app: &AppHandle, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
    let s = app.state::<AppState>().update_settings(f)?;
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

fn managed_settings() -> Vec<serde_json::Value> {
    let mut docs = cs::managed_dir().map(|d| cs::managed_files(&d)).unwrap_or_default();
    docs.extend(policy_values().iter().filter_map(|s| serde_json::from_str(s).ok()));
    docs
}

#[cfg(windows)]
fn policy_values() -> Vec<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    let key: Vec<u16> = "SOFTWARE\\Policies\\ClaudeCode\0".encode_utf16().collect();
    let val: Vec<u16> = "Settings\0".encode_utf16().collect();
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut out = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut size = 0u32;
        // SAFETY: size query, then a read into a buffer of the reported size.
        let rc = unsafe { RegGetValueW(root, key.as_ptr(), val.as_ptr(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
        if rc != 0 || size == 0 {
            continue;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let rc = unsafe { RegGetValueW(root, key.as_ptr(), val.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        if rc == 0 {
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            out.push(String::from_utf16_lossy(&buf[..n]));
        }
    }
    out
}

/// MDM configuration profiles land as `com.anthropic.claudecode` managed preferences, per user or machine-wide.
#[cfg(target_os = "macos")]
fn policy_values() -> Vec<String> {
    let base = Path::new("/Library/Managed Preferences");
    let mut plists = Vec::new();
    if let Ok(user) = std::env::var("USER") {
        plists.push(base.join(user).join("com.anthropic.claudecode.plist"));
    }
    plists.push(base.join("com.anthropic.claudecode.plist"));
    plists
        .iter()
        .filter(|p| p.is_file())
        .filter_map(|p| std::process::Command::new("/usr/bin/plutil").args(["-convert", "json", "-o", "-"]).arg(p).output().ok())
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn policy_values() -> Vec<String> {
    Vec::new()
}

fn code(e: cs::SettingsError) -> String {
    match e {
        cs::SettingsError::TelemetryConflict(k) => format!("telemetry_settings_conflict:{k}"),
        cs::SettingsError::Unparseable(d) => format!("settings_unparseable:{d}"),
        cs::SettingsError::Changed => "settings_changed".into(),
        cs::SettingsError::StateUnreadable(d) => format!("state_unreadable:{d}"),
        cs::SettingsError::Io(e) => e.to_string(),
    }
}

pub fn set(app: &AppHandle, kind: &str, on: bool) -> Result<String, String> {
    let state = app.state::<AppState>();
    let data_dir = state.data_dir.clone();
    let settings = state.settings.read().unwrap().clone();
    let read_state = || cs::read_state(&data_dir).map_err(code);
    match (kind, on) {
        ("codex", true) => {
            let bin = codex_binary(&settings).ok_or("codex_not_found")?;
            let epoch = state.capture.wipes.load(Ordering::SeqCst);
            let snaps = codex_limits::query(&bin, Duration::from_secs(20), now_ms())?;
            store_limits(&state, epoch, &snaps)?;
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
        ("antigravity", true) => {
            let bin = agy_binary(&settings).ok_or("agy_not_found")?;
            let epoch = state.capture.wipes.load(Ordering::SeqCst);
            let snaps = read_antigravity(&state, &bin)?;
            store_limits(&state, epoch, &snaps)?;
            *state.capture.antigravity.lock().unwrap() =
                PollStatus { binary: Some(bin.to_string_lossy().into_owned()), last_ok_ms: Some(now_ms()), last_error: None };
            persist(app, |s| s.capture.antigravity_poll = true)?;
            wake_antigravity(app);
            let _ = app.emit("data-changed", ());
            Ok("enabled".into())
        }
        ("antigravity", false) => {
            persist(app, |s| s.capture.antigravity_poll = false)?;
            Ok("disabled".into())
        }
        ("claude", true) => {
            let bin = claude_binary(&settings).ok_or("claude_not_found")?;
            let epoch = state.capture.wipes.load(Ordering::SeqCst);
            match read_claude(&state, &settings, &bin) {
                Ok(snaps) => store_limits(&state, epoch, &snaps)?,
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
            let mut cstate = read_state()?;
            if cstate.statusline.is_some() {
                refresh_statusline(&data_dir);
                cstate = read_state()?;
                // replaced since (by the user or Claude Code's `/statusline`): install again below,
                // chaining the new command
                if cstate.statusline.as_ref().is_some_and(cs::statusline_in_place) {
                    persist(app, |s| s.capture.statusline = true)?;
                    return Ok("already".into());
                }
            }
            let exe = crate::app_path()?;
            let file = claude_settings_file(&settings);
            let ch = cs::install_statusline(&mut cstate, &data_dir, &file, &statusline_command(&exe), now_ms()).map_err(code)?;
            persist(app, |s| s.capture.statusline = true)?;
            Ok(if ch.previous.is_some() { "enabled_chained".into() } else { "enabled".into() })
        }
        ("statusline", false) => {
            let mut cstate = read_state()?;
            // a failed revert keeps both the record and the setting, so it can be retried
            let note = outcome_text(cs::uninstall_statusline(&mut cstate, &data_dir).map_err(code)?);
            persist(app, |s| s.capture.statusline = false)?;
            Ok(note.into())
        }
        ("otel", true) => {
            let conflict = cs::conflicting_process_env();
            if !conflict.is_empty() {
                return Err(format!("telemetry_env_conflict:{}", conflict.join(",")));
            }
            let managed = cs::managed_conflicts(&managed_settings());
            if !managed.is_empty() {
                return Err(format!("telemetry_settings_conflict:managed settings: {}", managed.join(", ")));
            }
            let mut cstate = read_state()?;
            let port = settings.capture.otel_port;
            if cstate.otel.as_ref().is_some_and(|c| c.token().is_none()) {
                let _ = cs::add_otel_token(&mut cstate, &data_dir, &cs::new_token());
            }
            let token = match &cstate.otel {
                Some(ch) => ch.token().map(str::to_owned).ok_or(TOKEN_MISSING)?,
                None => cs::new_token(),
            };
            start_receiver(app, port, token.clone()).map_err(|e| format!("port_busy:{port}:{e}"))?;
            if cstate.otel.is_none() {
                let file = claude_settings_file(&settings);
                if let Err(e) = cs::install_otel(&mut cstate, &data_dir, &file, port, &token, now_ms()) {
                    stop_receiver(app);
                    return Err(code(e));
                }
            }
            persist(app, |s| s.capture.otel = true)?;
            Ok("enabled".into())
        }
        ("otel", false) => {
            let mut cstate = read_state()?;
            let note = outcome_text(cs::uninstall_otel(&mut cstate, &data_dir).map_err(code)?);
            stop_receiver(app);
            persist(app, |s| s.capture.otel = false)?;
            Ok(note.into())
        }
        _ => Err("unknown capture kind".into()),
    }
}

/// A limit read finished after "delete all data" (or before onboarding) is dropped.
const DISCARDED: &str = "discarded";

fn store_limits(state: &AppState, epoch: u64, snaps: &[tracker_core::model::LimitSnapshot]) -> Result<(), String> {
    let _writing = state.writers.read().unwrap_or_else(|e| e.into_inner());
    if state.capture.wipes.load(Ordering::SeqCst) != epoch || !state.settings.read().unwrap().onboarded {
        return Err(DISCARDED.into());
    }
    let mut store = Store::open(&state.db_path).map_err(|e| e.to_string())?;
    let mut tx = store.transaction().map_err(|e| e.to_string())?;
    tx.insert_limits(snaps).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// A telemetry install whose settings could not take a token: the receiver stays off, since
/// without one it would accept any local sender. Turning the method off and on reinstalls it.
const TOKEN_MISSING: &str = "telemetry_token_missing";

fn start_receiver(app: &AppHandle, port: u16, token: String) -> std::io::Result<()> {
    let state = app.state::<AppState>();
    let mut slot = state.capture.receiver.lock().unwrap();
    if slot.as_ref().is_some_and(|r| r.port == port && r.token == token) {
        return Ok(());
    }
    *slot = None;
    let handle = app.clone();
    match otlp::Receiver::start(port, state.db_path.clone(), token, move |_| {
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

/// A pre-token install gets a token first, so the receiver only accepts Claude Code's requests.
fn start_otel(app: &AppHandle) {
    let state = app.state::<AppState>();
    let port = state.settings.read().unwrap().capture.otel_port;
    let mut cstate = cs::load_state(&state.data_dir);
    if cstate.otel.as_ref().is_some_and(|c| c.token().is_none())
        && let Err(e) = cs::add_otel_token(&mut cstate, &state.data_dir, &cs::new_token())
    {
        log::warn!("could not add a token to the telemetry settings: {e}");
    }
    let Some(token) = cstate.otel.as_ref().and_then(|c| c.token()).map(str::to_owned) else {
        log::warn!("otlp receiver not started: the telemetry settings have no token");
        *state.capture.receiver_error.lock().unwrap() = Some(TOKEN_MISSING.into());
        return;
    };
    if let Err(e) = start_receiver(app, port, token) {
        log::warn!("otlp receiver could not start: {e}");
    }
}

pub fn stop_receiver(app: &AppHandle) {
    let state = app.state::<AppState>();
    let r = state.capture.receiver.lock().unwrap().take();
    drop(r);
}

/// Part of "delete all data": stops every capture writer before the data goes.
pub fn pause(app: &AppHandle) -> std::io::Result<()> {
    let state = app.state::<AppState>();
    state.capture.wipes.fetch_add(1, Ordering::SeqCst);
    stop_receiver(app);
    *state.capture.codex.lock().unwrap() = PollStatus::default();
    *state.capture.claude.lock().unwrap() = PollStatus::default();
    *state.capture.antigravity.lock().unwrap() = PollStatus::default();
    statusline::set_paused(&state.data_dir, true)
}

pub fn resume(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Err(e) = statusline::set_paused(&state.data_dir, false) {
        log::warn!("could not resume the status-line capture: {e}");
    }
    if state.settings.read().unwrap().capture.otel {
        start_otel(app);
    }
    wake_readers(app);
}

/// No background reads before onboarding is confirmed, nor for providers the user disabled.
fn reads_allowed(s: &Settings, sources: &[SourceId]) -> bool {
    s.onboarded && sources.iter().any(|id| s.enabled_sources.contains(id))
}

pub fn wake_readers(app: &AppHandle) {
    wake_claude(app);
    wake_codex(app);
    wake_antigravity(app);
}

fn wake_antigravity(app: &AppHandle) {
    if let Some(tx) = app.state::<AppState>().capture.antigravity_wake.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// First reads right after start, a second apart, so the widget shows limits at once without
/// starting three CLIs together.
const FIRST_READ: [Duration; 3] = [Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(3)];
/// A failed first read (e.g. no network yet after login) is retried a minute later, a few times.
const FIRST_RETRY: Duration = Duration::from_secs(60);
const FIRST_RETRIES: u8 = 3;

fn start_antigravity_poller(app: &AppHandle) {
    let (tx, rx) = channel::<()>();
    *app.state::<AppState>().capture.antigravity_wake.lock().unwrap() = Some(tx);
    let app = app.clone();
    let _ = std::thread::Builder::new().name("antigravity-limits".into()).spawn(move || {
        let mut wait = FIRST_READ[1];
        let mut quick_retries = FIRST_RETRIES;
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
            wait = Duration::from_secs(60 * s.capture.antigravity_poll_minutes.clamp(1, 120));
            if !s.capture.antigravity_poll || !reads_allowed(&s, &[SourceId::Antigravity]) {
                continue;
            }
            let Some(bin) = agy_binary(&s) else {
                state.capture.antigravity.lock().unwrap().last_error = Some("agy_not_found".into());
                wait = Duration::from_secs(30 * 60);
                continue;
            };
            let epoch = state.capture.wipes.load(Ordering::SeqCst);
            match read_antigravity(&state, &bin).and_then(|snaps| store_limits(&state, epoch, &snaps)) {
                Ok(()) => {
                    quick_retries = 0;
                    let mut c = state.capture.antigravity.lock().unwrap();
                    c.binary = Some(bin.to_string_lossy().into_owned());
                    c.last_ok_ms = Some(now_ms());
                    c.last_error = None;
                    drop(c);
                    let _ = app.emit("data-changed", ());
                }
                Err(e) if e == DISCARDED => {}
                Err(e) => {
                    if quick_retries > 0 {
                        quick_retries -= 1;
                        wait = wait.min(FIRST_RETRY);
                    }
                    let mut c = state.capture.antigravity.lock().unwrap();
                    if c.last_error.as_deref() != Some(e.as_str()) {
                        log::warn!("antigravity limit read failed: {e}");
                    }
                    c.last_error = Some(e);
                }
            }
        }
    });
}

fn wake_claude(app: &AppHandle) {
    if let Some(tx) = app.state::<AppState>().capture.claude_wake.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// `tool = 'claude_code'` also covers Cowork logs.
fn latest_claude_event_ms(state: &AppState) -> Option<i64> {
    state
        .db()
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
            match rx.recv_timeout(if last_try_ms == 0 { FIRST_READ[2] } else { TICK }) {
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
            let epoch = state.capture.wipes.load(Ordering::SeqCst);
            match read_claude(&state, &s, &bin).and_then(|snaps| store_limits(&state, epoch, &snaps)) {
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
                Err(e) if e == "claude_throttled" || e == DISCARDED => {}
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

/// Points Claude Code at this copy of the app when it was moved or reinstalled elsewhere. Debug
/// builds leave it alone, so switching between a dev and a release build does not rewrite it each time.
fn refresh_statusline(data_dir: &Path) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(exe) = crate::app_path() else { return };
    let mut cstate = cs::load_state(data_dir);
    match cs::refresh_statusline(&mut cstate, data_dir, &statusline_command(&exe)) {
        Ok(true) => log::info!("status line now points at this copy of the app"),
        Ok(false) => {}
        Err(e) => log::warn!("could not refresh the status line: {e}"),
    }
}

pub fn start(app: &AppHandle) {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    if s.onboarded {
        // a wipe interrupted before it deleted anything leaves the pause behind
        if let Err(e) = statusline::set_paused(&app.state::<AppState>().data_dir, false) {
            log::warn!("could not resume the status-line capture: {e}");
        }
        if s.capture.otel {
            start_otel(app);
        }
        if s.capture.statusline {
            refresh_statusline(&app.state::<AppState>().data_dir);
        }
        restore_after_upgrade(app);
    }
    start_claude_poller(app);
    start_antigravity_poller(app);
    let (tx, rx) = channel::<()>();
    *app.state::<AppState>().capture.codex_wake.lock().unwrap() = Some(tx);
    let app = app.clone();
    let _ = std::thread::Builder::new().name("codex-limits".into()).spawn(move || {
        let mut wait = FIRST_READ[0];
        let mut quick_retries = FIRST_RETRIES;
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
            let epoch = state.capture.wipes.load(Ordering::SeqCst);
            match codex_limits::query(&bin, Duration::from_secs(20), now_ms()).and_then(|snaps| store_limits(&state, epoch, &snaps)) {
                Ok(()) => {
                    quick_retries = 0;
                    let mut c = state.capture.codex.lock().unwrap();
                    c.binary = Some(bin.to_string_lossy().into_owned());
                    c.last_ok_ms = Some(now_ms());
                    c.last_error = None;
                    drop(c);
                    let _ = app.emit("data-changed", ());
                }
                Err(e) if e == DISCARDED => {}
                Err(e) => {
                    if quick_retries > 0 {
                        quick_retries -= 1;
                        wait = wait.min(FIRST_RETRY);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chained_command_that_runs_too_long_is_ended_with_what_it_started() {
        // Windows runs it through Git Bash, as Claude Code does; CI must really run it
        if cfg!(windows) && git_bash().is_none() {
            assert!(std::env::var_os("CI").is_none(), "Git Bash is required for this test on CI");
            eprintln!("skipped: no Git Bash");
            return;
        }
        // the shell waits for its child; or exits at once, leaving an orphan holding the output open
        for (i, tail) in ["& wait", "&"].iter().enumerate() {
            let marker = std::env::temp_dir().join(format!("aiut-chained-{}-{i}", std::process::id()));
            let _ = std::fs::remove_file(&marker);
            let cmd = format!("(sleep 1; echo survived > '{}') {tail}", marker.display().to_string().replace('\\', "/"));
            let started = Instant::now();
            assert_eq!(run_within(&cmd, Vec::new(), Duration::from_millis(300)), None, "{tail}");
            assert!(started.elapsed() < Duration::from_secs(3), "{tail}");
            std::thread::sleep(Duration::from_millis(1500));
            assert!(!marker.exists(), "a process started by `{tail}` outlived the deadline");
        }
        assert_eq!(run_within("cat", b"ok".to_vec(), Duration::from_secs(5)).as_deref(), Some("ok"));
    }

    #[cfg(unix)]
    #[test]
    fn a_chained_command_knows_it_was_chained_so_a_bridge_never_chains_again() {
        assert_eq!(run_within("printf %s \"$AIUT_STATUSLINE_CHAINED\"", Vec::new(), Duration::from_secs(5)).as_deref(), Some("1"));
    }

    #[cfg(windows)]
    #[test]
    fn a_chained_command_runs_inside_its_job_from_the_start() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;
        let mut c = std::process::Command::new(crate::system_exe(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
        c.args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep 30"]);
        let (mut child, job) = Job::spawn(&mut c).unwrap();
        let job = job.expect("the command was put in a job");
        let mut inside = 0;
        // SAFETY: both handles are open
        assert_ne!(unsafe { IsProcessInJob(child.as_raw_handle(), job.0, &mut inside) }, 0);
        assert_ne!(inside, 0);
        let started = Instant::now();
        job.terminate();
        child.wait().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "the job ended the command");
    }

    #[test]
    fn only_a_recent_uninstall_turns_captures_back_on() {
        let r = Reverted { statusline: true, otel: false, at_ms: 1_000_000 };
        assert_eq!(r.to_restore(1_000_000 + 5 * 60_000), ["statusline"]);
        assert!(r.to_restore(1_000_000 + REINSTALL_WINDOW_MS).is_empty());
        // a clock set back is no reason to touch the user's settings
        assert!(r.to_restore(1_000_000 - 1).is_empty());
    }

    #[test]
    fn claude_reads_stay_five_minutes_apart() {
        let last = Mutex::new(0);
        assert!(claim_read(&last, 1_000_000, CLAUDE_MIN_GAP_MS));
        assert!(!claim_read(&last, 1_000_000 + 10_000, CLAUDE_MIN_GAP_MS));
        assert!(!claim_read(&last, 1_000_000 + CLAUDE_MIN_GAP_MS - 1, CLAUDE_MIN_GAP_MS));
        assert!(claim_read(&last, 1_000_000 + CLAUDE_MIN_GAP_MS, CLAUDE_MIN_GAP_MS));
        assert_eq!(*last.lock().unwrap(), 1_000_000 + CLAUDE_MIN_GAP_MS);
    }

    #[cfg(not(windows))]
    #[test]
    fn status_line_paths_are_quoted_literally_for_sh() {
        let app = "/Applications/AI Usage Tracker.app/Contents/MacOS/ai-usage-tracker";
        assert_eq!(statusline_command(Path::new(app)), format!("'{app}' --statusline"));
        let odd = r"/Users/o'neil/$HOME `x` a\b/ai-usage-tracker";
        assert_eq!(quoted_command(odd, None, true), r"'/Users/o'\''neil/$HOME `x` a\b/ai-usage-tracker' --statusline");
    }

    #[cfg(windows)]
    #[test]
    fn status_line_paths_are_quoted_literally_for_each_shell() {
        assert_eq!(quoted_command(r"C:\Users\A\x.exe", Some(r"C:\Users\A\x.exe"), true), "C:/Users/A/x.exe --statusline");
        let odd = r"C:\Users\O'Neil $HOME `x`\app.exe";
        assert_eq!(quoted_command(odd, None, true), r"'C:/Users/O'\''Neil $HOME `x`/app.exe' --statusline");
        assert_eq!(quoted_command(odd, Some(r"C:\Users\ONEIL$~1\app.exe"), false), "& 'C:/Users/O''Neil $HOME `x`/app.exe' --statusline");
    }
}
