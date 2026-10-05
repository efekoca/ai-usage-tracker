//! Opt-in live capture. Everything here is off by default, changes outside this app's own
//! folder are recorded in [`claude_settings::CaptureState`] and can be reverted in one step.

pub mod claude_settings;
pub mod claude_usage;
pub mod codex_limits;
pub mod otlp;
pub mod statusline;

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::sync::mpsc;

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
    v.dedup();
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

/// The npm-installed CLIs are node scripts, which need `node` on the child's PATH.
#[cfg(not(windows))]
pub(crate) fn set_child_path(cmd: &mut std::process::Command) {
    if let Ok(p) = std::env::join_paths(unix_bin_dirs(&crate::discovery::Env::from_system())) {
        cmd.env("PATH", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn oversized_lines_are_skipped_and_the_rest_is_read() {
        let mut data = vec![b'x'; MAX_LINE + 10];
        data.extend_from_slice(b"\n{\"a\":1}\nnot json\n{\"b\":2}");
        let rx = json_lines(std::io::Cursor::new(data));
        let got: Vec<Value> = std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(5)).ok()).collect();
        assert_eq!(got, vec![serde_json::json!({"a": 1}), serde_json::json!({"b": 2})]);
    }
}
