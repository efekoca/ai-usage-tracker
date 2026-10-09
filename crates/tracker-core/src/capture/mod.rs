//! Opt-in live capture. Everything here is off by default, changes outside this app's own
//! folder are recorded in [`claude_settings::CaptureState`] and can be reverted in one step.

pub mod antigravity_limits;
pub mod claude_settings;
pub mod claude_usage;
pub mod codex_limits;
pub mod otlp;
pub mod statusline;

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_LINE: usize = 1024 * 1024;

/// Lines are capped and the queue is bounded, so a misbehaving child cannot exhaust memory.
pub(crate) fn json_lines(out: impl Read + Send + 'static) -> mpsc::Receiver<Value> {
    let (tx, rx) = mpsc::sync_channel::<Value>(64);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(out);
        let mut line = Vec::new();
        loop {
            line.clear();
            match (&mut reader).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            if line.len() > MAX_LINE && line.last() != Some(&b'\n') {
                // an oversized line: skip the rest of it
                let mut rest = Vec::new();
                loop {
                    rest.clear();
                    match (&mut reader).take(MAX_LINE as u64).read_until(b'\n', &mut rest) {
                        Ok(0) | Err(_) => return,
                        Ok(_) if rest.last() == Some(&b'\n') => break,
                        Ok(_) => {}
                    }
                }
                continue;
            }
            if let Ok(v) = serde_json::from_slice::<Value>(&line)
                && tx.send(v).is_err()
            {
                return;
            }
        }
    });
    rx
}

/// A relative PATH entry would resolve against the current folder, which is not trusted.
/// A CLI this app reads limits through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cli {
    Claude,
    Codex,
    Antigravity,
}

impl Cli {
    /// The maker's documented installer: (Unix script, the shell it runs in, Windows script).
    /// None of them asks anything; each puts the CLI where this app looks for it and leaves an
    /// existing install alone.
    fn installer(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Cli::Claude => ("https://claude.ai/install.sh", "bash", "https://claude.ai/install.ps1"),
            Cli::Codex => ("https://chatgpt.com/codex/install.sh", "sh", "https://chatgpt.com/codex/install.ps1"),
            Cli::Antigravity => ("https://antigravity.google/cli/install.sh", "bash", "https://antigravity.google/cli/install.ps1"),
        }
    }
}

fn installer_command(cli: Cli) -> Command {
    let (unix, shell, windows) = cli.installer();
    #[cfg(windows)]
    let mut c = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = (unix, shell);
        let mut c = Command::new("powershell.exe");
        // older Windows PowerShell may not offer TLS 1.2 on its own
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"]).arg(format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072; irm {windows} | iex"
        ));
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let _ = windows;
        let mut c = Command::new("bash");
        c.args(["-c", &format!("set -o pipefail; {{ curl -fsSL {unix} || wget -qO- {unix}; }} | {shell}")]);
        c
    };
    c.env("CODEX_NON_INTERACTIVE", "1");
    c
}

pub fn install(cli: Cli, timeout: Duration) -> Result<(), String> {
    run_installer(installer_command(cli), timeout)
}

fn run_installer(mut cmd: Command, timeout: Duration) -> Result<(), String> {
    // its own process group, so a timeout also ends the downloads and installers it started
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let deadline = Instant::now() + timeout;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("install_failed:{e}"))?;
    let mut stderr = child.stderr.take().ok_or("install_failed:no stderr")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stderr).take(64 << 10).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                kill_tree(&mut child);
                return Err("install_failed:timed out".into());
            }
            Err(e) => {
                kill_tree(&mut child);
                return Err(format!("install_failed:{e}"));
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    // something the installer left running may still hold its error output open
    let err = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    let err = String::from_utf8_lossy(&err);
    let last = err.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect::<String>();
    Err(format!("install_failed:{}", if last.is_empty() { status.to_string() } else { last }))
}

/// Ends the installer and everything it started.
fn kill_tree(child: &mut std::process::Child) {
    // a direct call: procps-ng 4.0.2's `kill -KILL -<group>` (Debian 12) exits 0 and signals nothing
    #[cfg(unix)]
    if let Ok(group) = libc::pid_t::try_from(child.id()) {
        // SAFETY: plain syscall; the group is the installer's own, created by `process_group(0)`
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let taskkill = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join(r"System32\taskkill.exe");
        let _ = Command::new(taskkill).args(["/T", "/F", "/PID", &child.id().to_string()]).creation_flags(CREATE_NO_WINDOW).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).collect()).unwrap_or_default()
}

pub(crate) fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Apps started from Finder or at login get only the system PATH, without Homebrew or ~/.local/bin.
#[cfg(not(windows))]
pub(crate) fn unix_bin_dirs(env: &crate::discovery::Env) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = env.home.iter().map(|h| h.join(".local").join("bin")).collect();
    v.extend(path_dirs());
    v.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    v.extend(node_manager_bins(env));
    v.dedup();
    v
}

/// Where Node version managers put `node` and globally installed CLIs, newest Node first.
#[cfg(not(windows))]
fn node_manager_bins(env: &crate::discovery::Env) -> Vec<PathBuf> {
    let Some(home) = &env.home else { return Vec::new() };
    let newest_first = |dir: PathBuf, bin: &str| {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        let key = |p: &PathBuf| -> Vec<u64> {
            p.file_name().and_then(|n| n.to_str()).unwrap_or("").trim_start_matches('v').split('.').map(|x| x.parse().unwrap_or(0)).collect()
        };
        versions.sort_by_key(|p| std::cmp::Reverse(key(p)));
        versions.into_iter().map(|p| p.join(bin)).collect::<Vec<_>>()
    };
    let mut v = vec![home.join(".volta/bin"), home.join(".bun/bin")];
    v.extend(newest_first(home.join(".nvm/versions/node"), "bin"));
    for fnm in [home.join(".local/share/fnm"), home.join("Library/Application Support/fnm")] {
        v.extend(newest_first(fnm.join("node-versions"), "installation/bin"));
    }
    v
}

/// Global npm package folders: `%APPDATA%\npm` on Windows, the Homebrew or system prefix elsewhere.
pub(crate) fn npm_roots(env: &crate::discovery::Env) -> Vec<PathBuf> {
    if cfg!(windows) {
        return env.roaming.iter().map(|r| r.join("npm/node_modules")).collect();
    }
    let mut v: Vec<PathBuf> = ["/opt/homebrew/lib/node_modules", "/usr/local/lib/node_modules"].map(PathBuf::from).into();
    v.extend(env.home.iter().map(|h| h.join(".npm-global/lib/node_modules")));
    v
}

/// The npm-installed CLIs are node scripts, which need `node` on the child's PATH; the CLI's own
/// folder comes first, so a copy under a version manager runs with that manager's `node`.
#[cfg(not(windows))]
pub(crate) fn set_child_path(cmd: &mut std::process::Command, bin: &std::path::Path) {
    let own = bin.parent().filter(|d| d.is_absolute()).map(|d| d.to_path_buf());
    if let Ok(p) = std::env::join_paths(own.into_iter().chain(unix_bin_dirs(&crate::discovery::Env::from_system()))) {
        cmd.env("PATH", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn a_failing_installer_reports_its_last_error_line() {
        let run = |script: &str, secs| {
            let mut c = Command::new("sh");
            c.args(["-c", script]);
            run_installer(c, Duration::from_secs(secs))
        };
        assert_eq!(run("echo fetching >&2; echo 'Error: unsupported architecture' >&2; exit 1", 5), Err("install_failed:Error: unsupported architecture".into()));
        assert_eq!(run("exit 0", 5), Ok(()));
        assert_eq!(run("sleep 30", 1), Err("install_failed:timed out".into()));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_timeout_ends_an_installer_that_closed_its_output_and_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("mark");
        let mut c = Command::new("sh");
        c.args(["-c", &format!("(sleep 2; touch '{}') & exec 2>&-; sleep 30", mark.display())]);
        let started = Instant::now();
        assert_eq!(run_installer(c, Duration::from_millis(500)), Err("install_failed:timed out".into()));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(3));
        assert!(!mark.exists(), "a process the installer started outlived the timeout");
    }

    #[test]
    fn each_cli_installs_with_its_makers_documented_script() {
        let script = |cli| format!("{:?}", installer_command(cli));
        for (cli, host) in [(Cli::Claude, "claude.ai/install"), (Cli::Codex, "chatgpt.com/codex/install"), (Cli::Antigravity, "antigravity.google/cli/install")] {
            let s = script(cli);
            assert!(s.contains(&format!("https://{host}.{}", if cfg!(windows) { "ps1" } else { "sh" })), "{s}");
        }
        #[cfg(not(windows))]
        assert!(script(Cli::Codex).contains("| sh") && script(Cli::Claude).contains("| bash"));
    }

    #[test]
    fn oversized_lines_are_skipped_and_the_rest_is_read() {
        let mut data = vec![b'x'; MAX_LINE + 10];
        data.extend_from_slice(b"\n{\"a\":1}\nnot json\n{\"b\":2}");
        let rx = json_lines(std::io::Cursor::new(data));
        let got: Vec<Value> = std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(5)).ok()).collect();
        assert_eq!(got, vec![serde_json::json!({"a": 1}), serde_json::json!({"b": 2})]);
    }
}
