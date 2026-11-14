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
use tracker_core::capture::{antigravity_limits, claude_signin, claude_usage, codex_limits, otlp, statusline, Cli};
use tracker_core::discovery::{self, Env, SourceId};
use tracker_core::model::Provider;
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
    quoted_command(&exe.to_string_lossy(), short_path(exe).as_deref(), !cfg!(windows) || git_bash().is_some(), "--statusline")
}

/// The cache refresh hook runs the same way: Claude Code picks the shell by the same rule.
pub fn cache_hook_command(exe: &Path) -> String {
    let cmd = quoted_command(&exe.to_string_lossy(), short_path(exe).as_deref(), !cfg!(windows) || git_bash().is_some(), "--cache-hook");
    // PowerShell would report a program's exit code 2, which wakes the conversation, as 1; in Git
    // Bash the unset variable leaves a bare `exit`, which keeps the code as it is
    if cfg!(windows) { format!("{cmd}; exit $LASTEXITCODE") } else { cmd }
}

/// The hook that waits for a limit's reset wakes the conversation the same way.
pub fn limit_hook_command(exe: &Path) -> String {
    let cmd = quoted_command(&exe.to_string_lossy(), short_path(exe).as_deref(), !cfg!(windows) || git_bash().is_some(), "--limit-hook");
    if cfg!(windows) { format!("{cmd}; exit $LASTEXITCODE") } else { cmd }
}

/// Single quotes: inside double quotes both shells would still expand `$` and backticks.
fn quoted_command(full: &str, short: Option<&str>, bash: bool, flag: &str) -> String {
    // only Windows paths are turned into forward slashes; a backslash is an ordinary file name character elsewhere
    let fwd = |s: &str| if cfg!(windows) { s.replace('\\', "/") } else { s.to_owned() };
    let plain = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "/:._-~".contains(c));
    if let Some(short) = short.map(fwd).filter(|s| plain(s)) {
        return format!("{short} {flag}");
    }
    let full = fwd(full);
    // PowerShell also ends a single-quoted string at the typographic single quotes
    let ps = |s: &str| s.chars().flat_map(|c| if matches!(c, '\'' | '\u{2018}'..='\u{201B}') { vec![c, c] } else { vec![c] }).collect::<String>();
    if bash { format!("'{}' {flag}", full.replace('\'', r"'\''")) } else { format!("& '{}' {flag}", ps(&full)) }
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
    statusline::record_cache_misses(&data_dir, &json, now_ms());
    let state = cs::load_state(&data_dir);
    // a bridge run as someone's previous command (another copy of the app) does not chain again,
    // so two bridges can never call each other without end
    let previous = state.statusline.and_then(|s| s.previous).filter(|_| std::env::var_os(CHAINED).is_none());
    let mut out = match previous.as_ref().and_then(|p| p.get("command")).and_then(|c| c.as_str()) {
        Some(cmd) => run_chained(cmd, input).unwrap_or_default(),
        None => statusline::render_default(windows.as_ref()),
    };
    // a static clock time: Claude Code redraws the line only when the conversation changes
    let options = statusline::read_options(&data_dir);
    let cache = (options.cache && std::env::var_os(CHAINED).is_none())
        .then(|| statusline::session_cache_expiry(&json, now_ms()))
        .flatten()
        .and_then(|at| statusline::cache_text(at, options.turkish, &chrono::Local));
    if let Some(part) = cache {
        out = statusline::append(&out, &part);
    }
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
    0
}

/// Run by Claude Code after each turn: waits in the background and, when the app asks, wakes the
/// conversation for one short turn, so its prompt cache is read before it runs out. Run on a new
/// prompt or at the end of a conversation, it sends the waiting one away at once.
pub fn cache_hook_main() -> i32 {
    use tracker_core::capture::refresh_hook as rh;
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return 0 };
    let mut input = Vec::new();
    let _ = std::io::stdin().take(1024 * 1024).read_to_end(&mut input);
    let json: serde_json::Value = serde_json::from_slice(&input).unwrap_or(serde_json::Value::Null);
    let Some(session) = json.get("session_id").and_then(|v| v.as_str()).filter(|s| rh::valid_session(s)) else { return 0 };
    let dir = rh::dir(&data_dir);
    // nothing is printed: a prompt hook's output would be added to the conversation
    match json.get("hook_event_name").and_then(|v| v.as_str()) {
        Some("UserPromptSubmit") => {
            rh::prompted(&dir, session, now_ms());
            return 0;
        }
        Some("SessionEnd") => {
            rh::retire(&dir, session);
            return 0;
        }
        _ if json.get("permission_mode").and_then(|v| v.as_str()) == Some("bypassPermissions") => {
            rh::unasked(&dir, session);
            return 0;
        }
        _ if scripted() => return 0,
        _ => {}
    }
    let transcript = json.get("transcript_path").and_then(|v| v.as_str()).map(PathBuf::from);
    let me = format!("{}-{}", std::process::id(), now_ms());
    // ends before Claude Code's own timeout, so the wait is never cut off mid-signal
    let until = Instant::now() + Duration::from_secs(cs::REFRESH_HOOK_TIMEOUT_SECS.saturating_sub(60));
    match rh::wait(&dir, rh::Wait::Cache, session, &me, transcript.as_deref(), Duration::from_secs(1), until, parent_alive()) {
        Ok(rh::Outcome::Wake(message)) => {
            rh::woke(&dir, session, now_ms());
            eprint!("{message}");
            2
        }
        _ => 0,
    }
}

fn resume_message(turkish: bool) -> &'static str {
    if turkish {
        "Bu, AI Usage Tracker'ın gönderdiği otomatik bir mesaj: bu konuşmayı durduran kullanım limiti yeniden başladı. Yarıda kalan işe kaldığın yerden devam et."
    } else {
        "This is an automatic message from AI Usage Tracker: the usage limit that stopped this conversation has started again. Continue the work from where it stopped."
    }
}

/// Run by Claude Code when the usage limit ends a turn: waits in the background until the app says
/// the limit started again, then wakes the conversation to continue. The words are the hook's own,
/// so nothing a file says reaches the conversation. A new prompt or the end of the conversation
/// sends the waiting one away at once.
pub fn limit_hook_main() -> i32 {
    use tracker_core::capture::refresh_hook as rh;
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return 0 };
    let mut input = Vec::new();
    let _ = std::io::stdin().take(1024 * 1024).read_to_end(&mut input);
    let json: serde_json::Value = serde_json::from_slice(&input).unwrap_or(serde_json::Value::Null);
    let Some(session) = json.get("session_id").and_then(|v| v.as_str()).filter(|s| rh::valid_session(s)) else { return 0 };
    let dir = rh::dir(&data_dir);
    let options = statusline::read_options(&data_dir);
    match json.get("hook_event_name").and_then(|v| v.as_str()) {
        Some("StopFailure") if json.get("error").and_then(|v| v.as_str()) == Some("rate_limit") && options.resume && !rh::refresh_turn(&dir, session, now_ms()) && !scripted() => {}
        Some("UserPromptSubmit" | "SessionEnd") => {
            rh::retire_limit(&dir, session);
            return 0;
        }
        _ => return 0,
    }
    rh::stopped(&dir, session, now_ms());
    let me = format!("{}-{}", std::process::id(), now_ms());
    let until = Instant::now() + Duration::from_secs(cs::RESUME_HOOK_TIMEOUT_SECS.saturating_sub(60));
    // the transcript is not watched: Claude Code may write to it while the conversation only waits
    match rh::wait(&dir, rh::Wait::Limit, session, &me, None, Duration::from_secs(2), until, parent_alive()) {
        Ok(rh::Outcome::Wake(_)) => {
            rh::resumed(&dir, session, now_ms());
            eprint!("{}", resume_message(options.turkish));
            2
        }
        _ => 0,
    }
}

/// `claude -p` waits for its hooks before it exits, so in a scripted run the hook must not wait (and
/// no one would see a refresh there). SDK runs say so in their entrypoint; a print run started from
/// inside another session inherits that session's, so the command line that started it is read too.
fn scripted() -> bool {
    if std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok_and(|e| e.starts_with("sdk")) {
        return true;
    }
    #[cfg(unix)]
    {
        let mut pid = std::os::unix::process::parent_id();
        // the hook may run under a shell that did not hand its place over
        for _ in 0..2 {
            let Some((args, parent)) = process_args(pid) else { break };
            if print_run(&args) {
                return true;
            }
            let shell = args.first().and_then(|a| Path::new(a.trim_start_matches('-')).file_name()).is_some_and(|n| ["sh", "bash", "zsh", "dash"].contains(&n.to_string_lossy().as_ref()));
            if !shell {
                break;
            }
            pid = parent;
        }
    }
    #[cfg(windows)]
    if ancestor_command_lines().iter().any(|l| print_run(&l.split_whitespace().map(|w| w.trim_matches('"').to_owned()).collect::<Vec<_>>())) {
        return true;
    }
    false
}

/// The command lines of the hook's parent and the two above it (the shell, then Claude Code). The
/// hook waits in the background, so the second or so this takes holds nothing up.
#[cfg(windows)]
fn ancestor_command_lines() -> Vec<String> {
    let script = format!(
        // no double quotes: they would not reach PowerShell intact through the command line
        "$p = {}; for ($i = 0; $i -lt 3; $i++) {{ $w = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $p); if (-not $w) {{ break }}; $p = $w.ParentProcessId; $q = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $p); if (-not $q) {{ break }}; [Console]::Out.WriteLine($q.CommandLine) }}",
        std::process::id()
    );
    let mut c = std::process::Command::new(crate::system_exe(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    run_command(c, Vec::new(), Duration::from_secs(10)).map(|o| o.lines().map(str::to_owned).collect()).unwrap_or_default()
}

fn print_run(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "-p" || a == "--print" || a.starts_with("--print="))
}

/// A process's command line and its parent.
#[cfg(target_os = "linux")]
fn process_args(pid: u32) -> Option<(Vec<String>, u32)> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args = raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // after the name in parentheses: state, then the parent
    let parent = stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()?;
    Some((args, parent))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_args(pid: u32) -> Option<(Vec<String>, u32)> {
    let out = std::process::Command::new("/bin/ps").args(["-o", "ppid=", "-o", "args=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut words = text.split_whitespace();
    let parent = words.next()?.parse().ok()?;
    Some((words.map(str::to_owned).collect(), parent))
}

/// Whether the Claude Code that started the hook still runs: an orphan gets a new parent.
#[cfg(unix)]
fn parent_alive() -> impl Fn() -> bool {
    let first = std::os::unix::process::parent_id();
    move || std::os::unix::process::parent_id() == first
}

/// Windows keeps no such link, so the Claude Code among the hook's ancestors is held open and asked
/// whether it still runs. When none is found, the hook's own time limit ends the wait.
#[cfg(windows)]
fn parent_alive() -> impl Fn() -> bool {
    let claude = ClaudeProcess::find();
    move || claude.as_ref().is_none_or(ClaudeProcess::running)
}

#[cfg(windows)]
struct ClaudeProcess(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ClaudeProcess {
    /// The nearest ancestor that is Claude Code itself (its own program, or Node running it); the
    /// shells between it and the hook are passed over.
    fn find() -> Option<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        // SAFETY: a process snapshot, closed below; PROCESSENTRY32W is plain data with its size set
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut processes = std::collections::HashMap::new();
        let mut more = unsafe { Process32FirstW(snap, &mut entry) } != 0;
        while more {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            processes.insert(entry.th32ProcessID, (entry.th32ParentProcessID, String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase()));
            more = unsafe { Process32NextW(snap, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snap) };
        let mut pid = processes.get(&std::process::id())?.0;
        for _ in 0..6 {
            let (parent, name) = processes.get(&pid)?;
            if name == "claude.exe" || name == "node.exe" {
                // SAFETY: only the right to wait on it; the handle is closed on drop
                let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
                return (!handle.is_null()).then_some(ClaudeProcess(handle));
            }
            pid = *parent;
        }
        None
    }

    fn running(&self) -> bool {
        use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
        // SAFETY: the handle is open until drop; a wait of no time only asks whether it ended
        unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
    }
}

#[cfg(windows)]
impl Drop for ClaudeProcess {
    fn drop(&mut self) {
        // SAFETY: owned handle, closed once
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(not(any(unix, windows)))]
fn parent_alive() -> impl Fn() -> bool {
    || true
}

/// Set for the user's previous status-line command.
const CHAINED: &str = "AIUT_STATUSLINE_CHAINED";

/// Telemetry variables the user's login shell exports. An app started from the Dock or at login
/// does not see them, but Claude Code started from a terminal does, so they would compete with ours.
#[cfg(unix)]
fn shell_telemetry_env() -> Vec<String> {
    const MARK: &str = "__aiut_env__";
    let Some(shell) = std::env::var("SHELL").ok().filter(|s| s.starts_with('/') && !s.contains('\'')) else { return Vec::new() };
    // interactive and login, so both profile and rc files are read; the mark skips what they print
    let out = run_within(&format!("'{shell}' -ilc 'printf \"\\n{MARK}\\n\"; env' </dev/null 2>/dev/null"), Vec::new(), Duration::from_secs(5)).unwrap_or_default();
    let Some((_, env)) = out.split_once(&format!("\n{MARK}\n")) else { return Vec::new() };
    env.lines().filter_map(|l| l.split_once('=')).map(|(k, _)| k).filter(|k| k.starts_with("OTEL_") || *k == "CLAUDE_CODE_ENABLE_TELEMETRY").map(str::to_owned).collect()
}

#[cfg(not(unix))]
fn shell_telemetry_env() -> Vec<String> {
    Vec::new()
}

fn run_chained(cmd: &str, input: Vec<u8>) -> Option<String> {
    run_within(cmd, input, Duration::from_secs(5))
}

fn run_within(cmd: &str, input: Vec<u8>, deadline: Duration) -> Option<String> {
    use std::process::Command;
    let c = match git_bash() {
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
    run_command(c, input, deadline)
}

/// I/O goes through helper threads, so a stalled child or its descendants cannot outlast the deadline.
fn run_command(mut c: std::process::Command, input: Vec<u8>, deadline: Duration) -> Option<String> {
    use std::process::Stdio;
    const MAX_OUT: u64 = 64 * 1024;
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
    let had = (state.statusline.is_some(), state.otel.is_some(), state.refresh.is_some(), state.resume.is_some());
    let statusline = cs::uninstall_statusline(&mut state, &data_dir).inspect_err(|e| eprintln!("ai-usage-tracker: status line: {e}"));
    let otel = cs::uninstall_otel(&mut state, &data_dir).inspect_err(|e| eprintln!("ai-usage-tracker: telemetry: {e}"));
    let refresh = cs::uninstall_refresh_hook(&mut state, &data_dir).inspect_err(|e| eprintln!("ai-usage-tracker: cache refresh: {e}"));
    let resume = cs::uninstall_hook(&mut state, cs::Hook::Resume, &data_dir).inspect_err(|e| eprintln!("ai-usage-tracker: limit hook: {e}"));
    let (statusline_ok, otel_ok, refresh_ok) = (statusline.is_ok(), otel.is_ok(), refresh.is_ok());
    // only what the uninstaller itself took out is put back; a value the user changed stays theirs
    let restored = |r: &Result<RevertOutcome, _>| matches!(r, Ok(RevertOutcome::Restored));
    let store = Store::open(&data_dir.join("tracker.db")).ok();
    let mut settings = store.as_ref().map(Settings::load);
    let auto = settings.as_ref().is_some_and(|s| s.cache_auto);
    let reverted = Reverted { statusline: had.0 && restored(&statusline), otel: had.1 && restored(&otel), refresh: had.2 && restored(&refresh), resume: had.3 && restored(&resume), auto, at_ms: now_ms() };
    if reverted.statusline || reverted.otel || reverted.refresh || reverted.resume {
        let _ = std::fs::write(reverted_file(&data_dir), serde_json::to_string(&reverted).unwrap_or_default());
    }
    if let (Some(store), Some(s)) = (&store, settings.as_mut()) {
        s.capture.statusline &= !statusline_ok;
        s.capture.otel &= !otel_ok;
        // a hook added again later must not start refreshing every conversation by itself; an
        // upgrade puts both back
        s.cache_auto &= !refresh_ok;
        // without its hook nothing is continued, so the switch goes with it
        s.limit_resume &= !resume.is_ok();
        let _ = s.save(store);
    }
    if statusline_ok && otel_ok && refresh_ok && resume.is_ok() { 0 } else { 1 }
}

/// What an uninstaller turned off. A manual upgrade runs the old uninstaller first, so a start
/// soon after turns the same captures back on.
#[derive(Debug, Serialize, Deserialize)]
struct Reverted {
    statusline: bool,
    otel: bool,
    #[serde(default)]
    refresh: bool,
    /// Automatic refresh was on with the hook.
    #[serde(default)]
    auto: bool,
    #[serde(default)]
    resume: bool,
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
        [("statusline", self.statusline), ("otel", self.otel), ("refresh", self.refresh), ("resume", self.resume)].into_iter().filter(|k| k.1).map(|k| k.0).collect()
    }
}

fn restore_auto(app: &AppHandle, auto: bool) -> Result<String, String> {
    if auto {
        let s = app.state::<AppState>().update_settings(|s| s.cache_auto = true)?;
        let _ = app.emit("settings-changed", &s);
    }
    Ok("enabled".into())
}

fn restore_after_upgrade(app: &AppHandle) {
    let file = reverted_file(&app.state::<AppState>().data_dir);
    let Ok(raw) = std::fs::read_to_string(&file) else { return };
    let _ = std::fs::remove_file(&file);
    let Ok(reverted) = serde_json::from_str::<Reverted>(&raw) else { return };
    for kind in reverted.to_restore(now_ms()) {
        let done = match kind {
            "refresh" => crate::refresh::install(app).and_then(|()| restore_auto(app, reverted.auto)),
            "resume" => crate::resume::install(app).and_then(|()| {
                let s = app.state::<AppState>().update_settings(|s| s.limit_resume = true)?;
                let _ = app.emit("settings-changed", &s);
                Ok("enabled".into())
            }),
            _ => set(app, kind, true),
        };
        match done {
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

/// What the last limit read says about a provider's sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignIn {
    Out,
    /// The last read worked, or the provider's limits are not being read.
    Clear,
    /// No read yet, or one that failed some other way, which says nothing either way.
    Unknown,
}

pub(crate) fn sign_ins(app: &AppHandle, s: &Settings) -> [(Provider, SignIn); 3] {
    let rt = &app.state::<AppState>().capture;
    let providers = crate::commands::enabled_providers(s);
    let of = |p: Provider, reads: bool, poll: &Mutex<PollStatus>, out: &str| {
        let poll = poll.lock().unwrap();
        let state = match (reads && providers.contains(&p), poll.last_error.as_deref(), poll.last_ok_ms) {
            (false, ..) => SignIn::Clear,
            (true, Some(e), _) if e == out => SignIn::Out,
            (true, None, Some(_)) => SignIn::Clear,
            _ => SignIn::Unknown,
        };
        (p, state)
    };
    [
        // an API plan has no limits to read
        of(Provider::Anthropic, s.capture.claude_poll && s.plans.get("anthropic").map(String::as_str) != Some("api"), &rt.claude, claude_usage::NOT_SIGNED_IN),
        of(Provider::OpenAI, s.capture.codex_poll, &rt.codex, codex_limits::NOT_SIGNED_IN),
        of(Provider::Google, s.capture.antigravity_poll, &rt.antigravity, antigravity_limits::NOT_SIGNED_IN),
    ]
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
    // a copy: looking for the binary can run it for up to 10 s, which must not hold the settings
    let settings = || state.settings.read().unwrap().clone();
    if find(&settings()).is_none() {
        tracker_core::capture::install(cli, Duration::from_secs(15 * 60))?;
    }
    find(&settings()).ok_or(format!("{code}_not_found"))?;
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

pub(crate) fn claude_binary(s: &Settings) -> Option<PathBuf> {
    let configured = (!s.capture.claude_path.trim().is_empty()).then(|| PathBuf::from(s.capture.claude_path.trim()));
    claude_usage::find_claude(&Env::from_system(), configured.as_deref())
}

/// Runs in an empty folder of the app's own so no project settings, files or trust prompts apply.
/// Claude throttles `get_usage` server-side, so reads are kept this far apart wherever they start.
const CLAUDE_MIN_GAP_MS: i64 = 5 * 60_000;

/// Takes the next read slot unless the previous read is less than `gap_ms` old.
fn claim_read(last_ms: &Mutex<i64>, now: i64, gap_ms: i64) -> bool {
    let mut last = last_ms.lock().unwrap();
    // a time ahead of the clock (the clock was set back) holds nothing up
    if *last != 0 && *last <= now && now - *last < gap_ms {
        return false;
    }
    *last = now;
    true
}

/// The last read survives a restart, so a relaunch within five minutes does not read again.
const CLAUDE_LAST_READ: &str = "capture.claude_last_read_ms";

fn read_claude(state: &AppState, s: &Settings, bin: &Path) -> Result<Vec<tracker_core::model::LimitSnapshot>, String> {
    let stored = state.db().setting(CLAUDE_LAST_READ).ok().flatten().and_then(|v| v.parse::<i64>().ok());
    if let Some(stored) = stored {
        let mut last = state.capture.claude_last_read_ms.lock().unwrap();
        *last = (*last).max(stored);
    }
    let now = now_ms();
    if !claim_read(&state.capture.claude_last_read_ms, now, CLAUDE_MIN_GAP_MS) {
        return Err("claude_throttled".into());
    }
    let _ = state.db().set_setting(CLAUDE_LAST_READ, &now.to_string());
    let work = state.data_dir.join("claude-usage");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let env = Env::from_system();
    let cfg = claude_usage::config_dir(&env, &s.extra_paths);
    // the sign-in the CLI reads with, when it is the same before and after the read
    let root = cfg.clone().or_else(|| env.home.as_ref().map(|h| h.join(".claude")));
    let file = root.as_deref().map(|r| claude_signin::config_file(&env, r));
    let signed_in = || file.as_deref().and_then(|f| claude_signin::signed_in(f).ok().flatten());
    let before = signed_in();
    // a token or key the CLI is given wins over the folder's sign-in
    let mut docs = managed_settings();
    docs.extend(root.as_deref().and_then(|r| cs::read_settings(&cs::settings_path(r))));
    let own = cs::own_sign_in_env() || cs::own_sign_in(&docs);
    let mut snaps = match claude_usage::query(bin, &work, cfg.as_deref(), Duration::from_secs(30), now_ms()) {
        // signing in would change nothing for a key the CLI is handed
        Err(claude_usage::UsageError::NotAvailable) if own => return Err("claude_own_key".into()),
        read => read.map_err(|e| e.to_string())?,
    };
    if !own && before.is_some() && before == signed_in() {
        snaps.iter_mut().for_each(|l| l.account = before.clone());
    }
    Ok(snaps)
}

/// What Claude Code sends its requests with, when the app starts it.
#[derive(Debug, Clone, PartialEq)]
pub enum CliSignIn {
    /// A token or key the CLI is handed, which wins over its folder's sign-in: such a request is
    /// billed by use and touches no plan window.
    OwnKey,
    /// Its config folder's sign-in.
    Account(String),
    /// The folder names no account.
    Unknown,
}

pub fn claude_cli_sign_in(s: &Settings) -> CliSignIn {
    let env = Env::from_system();
    let Some(root) = claude_usage::config_dir(&env, &s.extra_paths).or_else(|| env.home.as_ref().map(|h| h.join(".claude"))) else { return CliSignIn::Unknown };
    let mut docs = managed_settings();
    docs.extend(cs::read_settings(&cs::settings_path(&root)));
    if cs::own_sign_in_env() || cs::own_sign_in(&docs) {
        return CliSignIn::OwnKey;
    }
    claude_signin::signed_in(&claude_signin::config_file(&env, &root)).ok().flatten().map_or(CliSignIn::Unknown, CliSignIn::Account)
}

/// A settings file's size and change time, and the account it named then.
type SeenSignin = std::collections::HashMap<PathBuf, ((u64, Option<std::time::SystemTime>), Option<String>)>;

/// Notes which account each Claude config folder is signed in to (see `tracker_core::accounts`).
/// A settings file that has not changed since the last look is not read again. Returns whether
/// requests were tied anew (see `Store::note_signin`).
fn note_claude_signins(state: &AppState, s: &Settings, seen: &mut SeenSignin) -> bool {
    let _writing = state.writers.read().unwrap_or_else(|e| e.into_inner());
    // `s` may be from before "Delete all my data"; nothing is written into the emptied archive
    if !state.settings.read().unwrap().onboarded {
        return false;
    }
    let env = Env::from_system();
    let now = now_ms();
    let mut store: Option<Store> = None;
    let mut changed = false;
    for (root, file) in claude_signin::roots(&env, &s.extra_paths) {
        let Ok(meta) = std::fs::metadata(&file) else { continue };
        let stamp = (meta.len(), meta.modified().ok());
        let account = match seen.get(&file) {
            Some((at, account)) if *at == stamp => account.clone(),
            // a file caught while it is being written is read on the next look
            _ => match claude_signin::signed_in(&file) {
                Ok(account) => {
                    seen.insert(file.clone(), (stamp, account.clone()));
                    account
                }
                Err(_) => continue,
            },
        };
        if store.is_none() {
            match Store::open(&state.db_path) {
                Ok(st) => store = Some(st),
                Err(e) => {
                    log::warn!("claude sign-in not noted: {e}");
                    return false;
                }
            }
        }
        match store.as_mut().map(|st| st.note_signin(&root, account.as_deref(), now)) {
            Some(Ok(new)) => changed |= new,
            Some(Err(e)) => log::warn!("claude sign-in not noted: {e}"),
            None => {}
        }
    }
    if changed && let Some(st) = store.as_mut() && let Err(e) = tracker_core::accounts::resolve_claude(st) {
        log::warn!("claude accounts not resolved: {e}");
    }
    changed
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

/// Copies of the app's data folder that a packaged app (the Claude desktop app on Windows) gave the
/// programs it starts: their writes to the local app data folder land there instead.
#[cfg(windows)]
pub(crate) fn package_data_dirs(data_dir: &Path) -> Vec<PathBuf> {
    let (Some(folder), Some(local)) = (data_dir.file_name(), data_dir.parent()) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(local.join("Packages")) else { return Vec::new() };
    entries.flatten().map(|e| e.path().join("LocalCache").join("Local").join(folder)).filter(|d| d.is_dir()).collect()
}

#[cfg(not(windows))]
pub(crate) fn package_data_dirs(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

/// Changes to `capture/state.json` and Claude Code's settings go one at a time: each reads the
/// record, changes it and writes it back.
static STATE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set(app: &AppHandle, kind: &str, on: bool) -> Result<String, String> {
    let _one_at_a_time = matches!(kind, "statusline" | "otel").then(state_lock);
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
            let read = match read_claude(&state, &settings, &bin) {
                Ok(snaps) => {
                    store_limits(&state, epoch, &snaps)?;
                    true
                }
                // too soon to ask, so nothing is known yet about the sign-in: the status stays
                // as it was and the next cycle reads the limits
                Err(e) if e == "claude_throttled" => false,
                Err(e) => return Err(e),
            };
            if read {
                *state.capture.claude.lock().unwrap() =
                    PollStatus { binary: Some(bin.to_string_lossy().into_owned()), last_ok_ms: Some(now_ms()), last_error: None };
            }
            persist(app, |s| s.capture.claude_poll = true)?;
            wake_claude(app);
            let _ = app.emit("data-changed", ());
            Ok(if read { "enabled" } else { "claude_wait" }.into())
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
            let mut conflict = cs::conflicting_process_env();
            conflict.extend(shell_telemetry_env());
            conflict.sort();
            conflict.dedup();
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
            let _ = cs::logs_only_header(&mut cstate, &data_dir);
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
    tx.commit().map_err(|e| e.to_string())?;
    // a read may name the signed-in account, which settles whose windows are whose
    if snaps.iter().any(|s| s.tool == tracker_core::model::Tool::Codex)
        && let Err(e) = tracker_core::accounts::resolve_codex(&mut store)
    {
        log::warn!("codex accounts not resolved: {e}");
    }
    if snaps.iter().any(|s| s.provider == tracker_core::model::Provider::Anthropic)
        && let Err(e) = tracker_core::accounts::resolve_claude(&mut store)
    {
        log::warn!("claude accounts not resolved: {e}");
    }
    Ok(())
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
    let guard = state_lock();
    let mut cstate = cs::load_state(&state.data_dir);
    if cstate.otel.as_ref().is_some_and(|c| c.token().is_none())
        && let Err(e) = cs::add_otel_token(&mut cstate, &state.data_dir, &cs::new_token())
    {
        log::warn!("could not add a token to the telemetry settings: {e}");
    }
    if let Err(e) = cs::logs_only_header(&mut cstate, &state.data_dir) {
        log::warn!("could not limit the telemetry token to logs: {e}");
    }
    drop(guard);
    let Some(token) = cstate.otel.as_ref().and_then(|c| c.token()).map(str::to_owned) else {
        log::warn!("otlp receiver not started: the telemetry settings have no token");
        *state.capture.receiver_error.lock().unwrap() = Some(TOKEN_MISSING.into());
        return;
    };
    if let Err(e) = start_receiver(app, port, token.clone()) {
        log::warn!("otlp receiver could not start: {e}");
        let app = app.clone();
        std::thread::spawn(move || otel_port_taken(&app, port, token));
    }
}

/// Claude Code keeps sending to the port in its settings: while another program holds it, that
/// program would get the telemetry. After a short wait the export is taken out of the settings.
fn otel_port_taken(app: &AppHandle, port: u16, token: String) {
    for _ in 0..6 {
        std::thread::sleep(Duration::from_secs(5));
        let state = app.state::<AppState>();
        if state.quitting.load(Ordering::SeqCst) || !state.settings.read().unwrap().capture.otel {
            return;
        }
        if start_receiver(app, port, token.clone()).is_ok() {
            return;
        }
    }
    match set(app, "otel", false) {
        Ok(_) => {
            *app.state::<AppState>().capture.receiver_error.lock().unwrap() = Some(format!("port_busy_off:{port}"));
            let _ = app.emit("settings-changed", app.state::<AppState>().settings.read().unwrap().clone());
        }
        Err(e) => log::warn!("telemetry left on with its port taken: {e}"),
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

/// How often a limit reader runs. Claude's usage service answers at most every 5 minutes.
pub fn set_interval(app: &AppHandle, kind: &str, minutes: u64) -> Result<Settings, String> {
    match kind {
        "claude" => persist(app, |s| s.capture.claude_poll_minutes = minutes.clamp(5, 120)),
        // the Claude reader looks at its interval every 30 seconds; these two would wait out the old one
        "codex" => persist(app, |s| s.capture.codex_poll_minutes = minutes.clamp(1, 120)).inspect(|_| wake_codex(app)),
        "antigravity" => persist(app, |s| s.capture.antigravity_poll_minutes = minutes.clamp(1, 120)).inspect(|_| wake_antigravity(app)),
        _ => Err(format!("unknown capture kind: {kind}")),
    }
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

pub(crate) fn wake_claude(app: &AppHandle) {
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
        let mut signins = SeenSignin::new();
        let mut first = true;
        loop {
            // soon after start, then on the tick: with reads off, `last_try_ms` stays 0
            match rx.recv_timeout(if first && last_try_ms == 0 { FIRST_READ[2] } else { TICK }) {
                Ok(()) => forced = true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            first = false;
            let state = app.state::<AppState>();
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            let s = state.settings.read().unwrap().clone();
            if reads_allowed(&s, &[SourceId::ClaudeCode]) && note_claude_signins(&state, &s, &mut signins) {
                let _ = app.emit("data-changed", ());
            }
            if !s.capture.claude_poll || !reads_allowed(&s, &[SourceId::ClaudeCode, SourceId::Cowork, SourceId::ClaudeDesktop]) {
                forced = false;
                continue;
            }
            let now = now_ms();
            let active_gap = (60_000 * s.capture.claude_poll_minutes.clamp(5, 120) as i64).max(ACTIVE_GAP_MS);
            let latest = latest_claude_event_ms(&state);
            let new_activity = latest.is_some() && latest != seen_event_ms;
            // a clock set back since the last try must not hold reads off until it catches up
            let since = if now < last_try_ms { i64::MAX } else { now - last_try_ms };
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
    let _one_at_a_time = state_lock();
    let mut cstate = cs::load_state(data_dir);
    match cs::refresh_statusline(&mut cstate, data_dir, &statusline_command(&exe)) {
        Ok(true) => log::info!("status line now points at this copy of the app"),
        Ok(false) => {}
        Err(e) => log::warn!("could not refresh the status line: {e}"),
    }
}

/// The bridge runs as a process of its own, so what it adds is written down for it.
pub fn write_statusline_options(data_dir: &Path, s: &Settings) {
    let options = statusline::Options { cache: s.cache_timer && s.cache_statusline, turkish: crate::tray::turkish(s), resume: s.limit_resume };
    if let Err(e) = statusline::write_options(data_dir, &options) {
        log::warn!("could not save the status-line options: {e}");
    }
}

pub fn start(app: &AppHandle) {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    // the system language may have changed since the last run
    write_statusline_options(&app.state::<AppState>().data_dir, &s);
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
        crate::refresh::follow_app(&app.state::<AppState>());
        crate::resume::follow_app(&app.state::<AppState>());
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
    fn a_print_run_is_told_by_its_flag() {
        let args = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        assert!(print_run(&args("claude -p hello --model haiku")));
        assert!(print_run(&args("node /x/cli.js --print hi")));
        assert!(!print_run(&args("claude --model opus")));
        assert!(!print_run(&args("-p")), "the program's own name is not a flag");
        #[cfg(unix)]
        assert!(process_args(std::process::id()).is_some_and(|(a, p)| !a.is_empty() && p > 0), "this test's own command line");
    }

    #[test]
    fn only_a_recent_uninstall_turns_captures_back_on() {
        let r = Reverted { statusline: true, otel: false, refresh: true, auto: true, resume: false, at_ms: 1_000_000 };
        assert_eq!(r.to_restore(1_000_000 + 5 * 60_000), ["statusline", "refresh"]);
        let older: Reverted = serde_json::from_str(r#"{"statusline": false, "otel": true, "at_ms": 1000000}"#).unwrap();
        assert_eq!(older.to_restore(1_000_000), ["otel"], "a record written before the refresh hook existed");
        assert!(r.to_restore(1_000_000 + REINSTALL_WINDOW_MS).is_empty());
        // a clock set back is no reason to touch the user's settings
        assert!(r.to_restore(1_000_000 - 1).is_empty());
    }

    #[test]
    fn the_bridge_adds_the_cache_time_only_while_both_switches_are_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings { language: "tr".into(), cache_statusline: true, ..Settings::default() };
        write_statusline_options(dir.path(), &s);
        assert_eq!(statusline::read_options(dir.path()), statusline::Options { cache: true, turkish: true, resume: false });
        s.cache_timer = false;
        s.language = "en".into();
        write_statusline_options(dir.path(), &s);
        assert_eq!(statusline::read_options(dir.path()), statusline::Options { cache: false, turkish: false, resume: false });
    }

    #[test]
    fn claude_reads_stay_five_minutes_apart() {
        let last = Mutex::new(0);
        assert!(claim_read(&last, 1_000_000, CLAUDE_MIN_GAP_MS));
        assert!(!claim_read(&last, 1_000_000 + 10_000, CLAUDE_MIN_GAP_MS));
        assert!(!claim_read(&last, 1_000_000 + CLAUDE_MIN_GAP_MS - 1, CLAUDE_MIN_GAP_MS));
        assert!(claim_read(&last, 1_000_000 + CLAUDE_MIN_GAP_MS, CLAUDE_MIN_GAP_MS));
        assert_eq!(*last.lock().unwrap(), 1_000_000 + CLAUDE_MIN_GAP_MS);
        // a stored read ahead of the clock (the clock was set back) does not block reads
        let ahead = Mutex::new(5_000_000);
        assert!(claim_read(&ahead, 1_000_000, CLAUDE_MIN_GAP_MS));
    }

    #[cfg(not(windows))]
    #[test]
    fn status_line_paths_are_quoted_literally_for_sh() {
        let app = "/Applications/AI Usage Tracker.app/Contents/MacOS/ai-usage-tracker";
        assert_eq!(statusline_command(Path::new(app)), format!("'{app}' --statusline"));
        assert_eq!(cache_hook_command(Path::new(app)), format!("'{app}' --cache-hook"));
        let odd = r"/Users/o'neil/$HOME `x` a\b/ai-usage-tracker";
        assert_eq!(quoted_command(odd, None, true, "--statusline"), r"'/Users/o'\''neil/$HOME `x` a\b/ai-usage-tracker' --statusline");
        // the PowerShell form doubles every kind of single quote it would end a string at
        assert_eq!(quoted_command("/x/O\u{2019}Neil/O'K/app", None, false, "--statusline"), "& '/x/O\u{2019}\u{2019}Neil/O''K/app' --statusline");
    }

    #[cfg(windows)]
    #[test]
    fn status_line_paths_are_quoted_literally_for_each_shell() {
        assert_eq!(quoted_command(r"C:\Users\A\x.exe", Some(r"C:\Users\A\x.exe"), true, "--statusline"), "C:/Users/A/x.exe --statusline");
        assert_eq!(quoted_command(r"C:\Users\A\x.exe", Some(r"C:\Users\A\x.exe"), true, "--cache-hook"), "C:/Users/A/x.exe --cache-hook");
        assert!(cache_hook_command(Path::new(r"C:\Missing\app.exe")).ends_with("--cache-hook; exit $LASTEXITCODE"), "the wake's exit code survives PowerShell");
        let odd = r"C:\Users\O'Neil $HOME `x`\app.exe";
        assert_eq!(quoted_command(odd, None, true, "--statusline"), r"'C:/Users/O'\''Neil $HOME `x`/app.exe' --statusline");
        assert_eq!(quoted_command(odd, Some(r"C:\Users\ONEIL$~1\app.exe"), false, "--statusline"), "& 'C:/Users/O''Neil $HOME `x`/app.exe' --statusline");
    }
}
