//! Claude Code statusLine bridge (opt-in). Claude Code runs the configured command on every
//! status update with session JSON on stdin; we keep only the documented `rate_limits` windows
//! (`used_percentage` 0–100, `resets_at` epoch seconds) and append them to a local capture
//! file, which the ingest worker then reads like any other source.
//!
//! The bridge is transparent: if the user already had a status line, that command is run with
//! the same stdin and its output is printed unchanged.

use crate::model::{Accuracy, LimitSnapshot, Provider, Tool};
use crate::sources::{f64_at, i64_at, read_jsonl_from, ParseOutput};
use serde_json::{json, Map, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SOURCE: &str = "claude_statusline";
const ROTATE_BYTES: u64 = 4 * 1024 * 1024;
/// Write an unchanged reading again after this long, so the series shows it is still current.
const HEARTBEAT_MS: i64 = 5 * 60 * 1000;

pub fn capture_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("capture")
}
pub fn capture_file(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("statusline.jsonl")
}
pub fn rotated_file(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("statusline.old.jsonl")
}
fn last_file(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("statusline.last.json")
}

/// Extracts only the limit windows. Returns `None` when the input has no `rate_limits`
/// (non-subscribers, or before the first API response of a session).
pub fn extract_windows(input: &Value) -> Option<Map<String, Value>> {
    let rl = input.get("rate_limits")?.as_object()?;
    let mut out = Map::new();
    for (name, w) in rl {
        let Some(pct) = f64_at(w, "used_percentage") else { continue };
        out.insert(name.clone(), json!({ "used_percentage": pct, "resets_at": i64_at(w, "resets_at") }));
    }
    (!out.is_empty()).then_some(out)
}

/// Appends the windows to the capture file when they changed (or after a heartbeat).
/// Returns whether a line was written. Never panics on I/O problems.
pub fn record(data_dir: &Path, windows: &Map<String, Value>, now_ms: i64) -> bool {
    let _ = fs::create_dir_all(capture_dir(data_dir));
    let last: Option<Value> = fs::read_to_string(last_file(data_dir)).ok().and_then(|s| serde_json::from_str(&s).ok());
    let same = last.as_ref().and_then(|l| l.get("windows")) == Some(&Value::Object(windows.clone()));
    let recent = last.as_ref().and_then(|l| l.get("ts")).and_then(Value::as_i64).is_some_and(|t| now_ms - t < HEARTBEAT_MS);
    if same && recent {
        return false;
    }
    let line = json!({ "ts": now_ms, "windows": windows });
    let path = capture_file(data_dir);
    if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
        let _ = fs::rename(&path, rotated_file(data_dir));
    }
    let ok = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{line}"))
        .is_ok();
    if ok {
        let _ = fs::write(last_file(data_dir), line.to_string());
    }
    ok
}

/// One-line status shown when the user had no status line of their own.
pub fn render_default(windows: Option<&Map<String, Value>>) -> String {
    let Some(w) = windows else { return String::new() };
    let mut parts = Vec::new();
    for (key, label) in [("five_hour", "5h"), ("seven_day", "7d"), ("spend_limit", "spend")] {
        if let Some(p) = w.get(key).and_then(|v| f64_at(v, "used_percentage")) {
            parts.push(format!("{label} {p:.0}%"));
        }
    }
    parts.join(" · ")
}

/// Parser for the capture file (incremental JSONL).
pub fn parse_file(path: &Path, offset: u64) -> std::io::Result<ParseOutput> {
    let (lines, next) = read_jsonl_from(path, offset)?;
    let mut out = ParseOutput { next_offset: next, ..Default::default() };
    for line in &lines {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warnings.push(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        let (Some(ts), Some(windows)) = (i64_at(v, "ts"), v.get("windows").and_then(Value::as_object)) else { continue };
        for (name, w) in windows {
            out.limits.push(LimitSnapshot {
                ts_ms: ts,
                provider: Provider::Anthropic,
                tool: Tool::ClaudeCode,
                account: None,
                limit_id: String::new(),
                window: name.clone(),
                used_pct: f64_at(w, "used_percentage"),
                resets_at: i64_at(w, "resets_at"),
                status: None,
                plan: None,
                source: SOURCE.into(),
                accuracy: Accuracy::Captured,
            });
        }
        out.lines_recognised += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        json!({
            "session_id": "s", "model": {"display_name": "Opus"}, "workspace": {"current_dir": "C:/x"},
            "rate_limits": {
                "five_hour": {"used_percentage": 23.5, "resets_at": 1738425600},
                "seven_day": {"used_percentage": 41.2, "resets_at": 1738857600}
            }
        })
    }

    #[test]
    fn keeps_only_limit_windows() {
        let w = extract_windows(&input()).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(serde_json::to_string(&w).unwrap(), r#"{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},"seven_day":{"used_percentage":41.2,"resets_at":1738857600}}"#);
        assert!(extract_windows(&json!({"model": {}})).is_none());
        assert_eq!(render_default(Some(&w)), "5h 24% · 7d 41%");
    }

    #[test]
    fn records_only_changes_and_heartbeats_then_parses_back() {
        let dir = tempfile::tempdir().unwrap();
        let w = extract_windows(&input()).unwrap();
        assert!(record(dir.path(), &w, 1_000));
        assert!(!record(dir.path(), &w, 2_000)); // unchanged, recent
        assert!(record(dir.path(), &w, 1_000 + HEARTBEAT_MS + 1)); // heartbeat
        let mut changed = w.clone();
        changed.insert("five_hour".into(), json!({"used_percentage": 30.0, "resets_at": 1738425600}));
        assert!(record(dir.path(), &changed, 1_000 + HEARTBEAT_MS + 2));
        let out = parse_file(&capture_file(dir.path()), 0).unwrap();
        assert_eq!(out.limits.len(), 6);
        assert!(out.limits.iter().all(|l| l.accuracy == Accuracy::Captured && l.source == SOURCE));
        assert_eq!(out.limits.last().unwrap().used_pct, Some(41.2));
    }
}
