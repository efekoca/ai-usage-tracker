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
