//! Claude Code statusLine bridge (opt-in). Claude Code runs the configured command on every
//! status update with session JSON on stdin; we keep only the documented `rate_limits` windows
//! (`used_percentage` 0–100, `resets_at` epoch seconds) and append them to a local capture
//! file, which the ingest worker then reads like any other source. Each conversation's
//! `prompt_cache` miss counts and causes are kept the same way, once each time they change.
//!
//! The bridge is transparent: if the user already had a status line, that command is run with
//! the same stdin and its output is printed unchanged. When the user asks for it, the time the
//! conversation's prompt cache runs out is added at the end.

use crate::model::{Accuracy, CacheMissReport, LimitSnapshot, Provider, Tokens, Tool};
use crate::sources::claude_code::{usage_events, ClaudeCtx};
use crate::sources::{f64_at, i64_at, str_at, u64_at, JsonlReader, ParseOutput};
use chrono::{TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const SOURCE: &str = "claude_statusline";
const ROTATE_BYTES: u64 = 4 * 1024 * 1024;
/// Write an unchanged reading again after this long, so the series shows it is still current.
const HEARTBEAT_MS: i64 = 5 * 60 * 1000;
/// A rotation takes milliseconds; a lock this old was left by a bridge that ended mid-way.
const STALE_LOCK: std::time::Duration = std::time::Duration::from_secs(60);

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
fn misses_file(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("statusline.misses.json")
}
fn options_file(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("statusline.options.json")
}
fn paused_file(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("paused")
}
fn rotate_lock(data_dir: &Path) -> PathBuf {
    capture_dir(data_dir).join("statusline.rotate.lock")
}

/// While paused the bridge still shows the status line but records nothing.
pub fn set_paused(data_dir: &Path, paused: bool) -> std::io::Result<()> {
    if paused {
        fs::create_dir_all(capture_dir(data_dir))?;
        fs::write(paused_file(data_dir), b"")
    } else {
        match fs::remove_file(paused_file(data_dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

pub fn delete_records(data_dir: &Path) -> std::io::Result<()> {
    for p in [capture_file(data_dir), rotated_file(data_dir), last_file(data_dir), misses_file(data_dir)] {
        match fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
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
    if paused_file(data_dir).exists() {
        return false;
    }
    let _ = fs::create_dir_all(capture_dir(data_dir));
    let last: Option<Value> = fs::read_to_string(last_file(data_dir)).ok().and_then(|s| serde_json::from_str(&s).ok());
    let same = last.as_ref().and_then(|l| l.get("windows")) == Some(&Value::Object(windows.clone()));
    let recent = last.as_ref().and_then(|l| l.get("ts")).and_then(Value::as_i64).is_some_and(|t| now_ms - t < HEARTBEAT_MS);
    if same && recent {
        return false;
    }
    let line = json!({ "ts": now_ms, "windows": windows }).to_string();
    let ok = append_line(data_dir, &line);
    if ok {
        let _ = fs::write(last_file(data_dir), line);
    }
    ok
}

/// Conversations whose miss counts were last written, the newest last.
const MISSES_KEPT: usize = 64;
/// Claude Code names a cause in lower case with underscores; anything else is not kept.
fn cause_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 48 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Notes the cache misses Claude Code reports for the conversation, once each time their counts
/// change. Returns whether a line was written.
pub fn record_cache_misses(data_dir: &Path, input: &Value, now_ms: i64) -> bool {
    let session = str_at(input, "session_id").filter(|s| crate::capture::refresh_hook::valid_session(s));
    let (Some(session), Some(cache)) = (session, input.get("prompt_cache").filter(|c| c.is_object())) else { return false };
    let (misses, rebuilds) = (u64_at(cache, "misses"), u64_at(cache, "expected_rebuilds"));
    if (misses == 0 && rebuilds == 0) || paused_file(data_dir).exists() {
        return false;
    }
    let mut seen: Vec<(String, u64, u64)> = fs::read(misses_file(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    if seen.iter().any(|(s, m, r)| s == session && *m == misses && *r == rebuilds) {
        return false;
    }
    let causes: Map<String, Value> = cache
        .get("miss_causes")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter(|(k, v)| cause_name(k) && v.as_u64().is_some_and(|n| n > 0)).take(32).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let last = i64_at(cache, "last_miss_at").and_then(|s| s.checked_mul(1000));
    let report = json!({ "requests": u64_at(cache, "requests"), "misses": misses, "rebuilds": rebuilds, "recached": u64_at(cache, "miss_recache_tokens"), "last": last, "causes": causes });
    let _ = fs::create_dir_all(capture_dir(data_dir));
    if !append_line(data_dir, &json!({ "ts": now_ms, "session": session, "cache": report }).to_string()) {
        return false;
    }
    seen.retain(|(s, _, _)| s != session);
    seen.push((session.to_owned(), misses, rebuilds));
    let drop = seen.len().saturating_sub(MISSES_KEPT);
    seen.drain(..drop);
    // several bridges write it at once; a note lost to another only writes the line once more
    let tmp = capture_dir(data_dir).join(format!(".statusline.misses.{}", std::process::id()));
    if serde_json::to_vec(&seen).ok().is_some_and(|b| fs::write(&tmp, b).is_ok()) && fs::rename(&tmp, misses_file(data_dir)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
    true
}

fn append_line(data_dir: &Path, line: &str) -> bool {
    let path = capture_file(data_dir);
    if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
        rotate(data_dir, &path);
    }
    // one write per line: several sessions append at once and must not interleave
    OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(format!("{line}\n").as_bytes())).is_ok()
}

/// Several bridges run at once: without the lock a second one that also saw the full file would
/// move a fresh one over the rotated file before it was read.
fn rotate(data_dir: &Path, path: &Path) {
    let lock = rotate_lock(data_dir);
    if OpenOptions::new().write(true).create_new(true).open(&lock).is_err() {
        // a bridge that ended while holding it must not stop rotation for good
        let stale = fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE_LOCK);
        if stale {
            let _ = fs::remove_file(&lock);
        }
        return;
    }
    if fs::metadata(path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
        let _ = fs::rename(path, rotated_file(data_dir));
    }
    let _ = fs::remove_file(&lock);
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

/// What the app asks the bridge to add to the status line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// The time the conversation's prompt cache runs out.
    pub cache: bool,
    pub turkish: bool,
}

/// A missing or unreadable file adds nothing.
pub fn read_options(data_dir: &Path) -> Options {
    fs::read(options_file(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn write_options(data_dir: &Path, options: &Options) -> std::io::Result<()> {
    if read_options(data_dir) == *options {
        return Ok(());
    }
    fs::create_dir_all(capture_dir(data_dir))?;
    let tmp = capture_dir(data_dir).join(format!(".statusline.options.{}", std::process::id()));
    // written whole first, so a bridge never reads half of it
    fs::write(&tmp, serde_json::to_vec(options)?)?;
    fs::rename(&tmp, options_file(data_dir)).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// How much of the transcript's end is read, a larger part only when a smaller one does not tell.
const TAIL_BYTES: [u64; 3] = [256 * 1024, 2 * 1024 * 1024, 8 * 1024 * 1024];

/// When the prompt cache of the conversation the status line is drawn for runs out. Claude Code
/// 2.1.251 and later send it as `prompt_cache.expires_at` (`null` once it went cold); for an older
/// one the transcript tells.
pub fn session_cache_expiry(input: &Value, now_ms: i64) -> Option<i64> {
    if let Some(cache) = input.get("prompt_cache").filter(|c| c.is_object()) {
        let at = cache.get("expires_at").and_then(Value::as_i64)?.checked_mul(1000)?;
        return (at > now_ms && cache.get("warm").and_then(Value::as_bool) != Some(false)).then_some(at);
    }
    cache_expiry(Path::new(input.get("transcript_path")?.as_str()?), now_ms)
}

/// When the prompt cache of the conversation in `transcript` runs out, by the rule of the app's
/// live caches: the lifetime it was last written with, after the last request that used it.
/// `None` once it ran out, or when the end of the transcript does not tell.
pub fn cache_expiry(transcript: &Path, now_ms: i64) -> Option<i64> {
    let mut file = File::open(transcript).ok()?;
    let len = file.metadata().ok()?.len();
    for size in TAIL_BYTES {
        let from = len.saturating_sub(size);
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(from)).ok()?;
        (&mut file).take(len - from).read_to_end(&mut bytes).ok()?;
        // a read that starts mid-file starts mid-line
        let text = String::from_utf8_lossy(&bytes);
        if let Some(at) = expiry_in(text.lines().skip(usize::from(from > 0))) {
            return (at > now_ms).then_some(at);
        }
        if from == 0 {
            break;
        }
    }
    None
}

fn expiry_in<'a>(lines: impl Iterator<Item = &'a str>) -> Option<i64> {
    let ctx = ClaudeCtx { client_override: None, source: SOURCE };
    // a response is written once per content block; its copies merge like they do in the store
    let mut requests: HashMap<String, (i64, String, Tokens)> = HashMap::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        // the first event is the request itself, any others the advisors it consulted;
        // subagents keep caches of their own
        let Some(e) = usage_events(&v, &ctx).into_iter().next().filter(|e| e.agent.is_none()) else { continue };
        requests.entry(e.key).and_modify(|r| r.2 = r.2.max(&e.tokens)).or_insert((e.ts_ms, e.model, e.tokens));
    }
    let last = requests.values().filter(|r| r.2.cache_read + r.2.cache_write > 0).max_by_key(|r| r.0)?;
    let written = requests.values().filter(|r| r.1 == last.1 && r.0 <= last.0 && r.2.cache_write > 0).max_by_key(|r| r.0)?;
    Some(last.0 + crate::insights::cache_ttl_ms(written.2.cache_write, written.2.cache_write_1h))
}

/// "cache until 14:32" by the local clock, rounded down so it never promises time it lacks.
pub fn cache_text<Tz: TimeZone>(expires_ms: i64, turkish: bool, tz: &Tz) -> Option<String> {
    let at = tz.timestamp_millis_opt(expires_ms).single()?;
    let hm = format!("{:02}:{:02}", at.hour(), at.minute());
    Some(if turkish { format!("önbellek {hm} silinir") } else { format!("cache until {hm}") })
}

/// `part` at the end of the status line's last line, after that line's own colors are reset.
pub fn append(out: &str, part: &str) -> String {
    let body = out.trim_end_matches(['\r', '\n']);
    if body.trim().is_empty() {
        return part.to_owned();
    }
    let reset = if body.contains('\u{1b}') { "\u{1b}[0m" } else { "" };
    format!("{body}{reset} · {part}")
}

pub fn parse_file(path: &Path, offset: u64) -> std::io::Result<ParseOutput> {
    let mut lines = JsonlReader::open(path, offset)?;
    let mut out = ParseOutput::default();
    for line in lines.by_ref() {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warn(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        let Some(ts) = i64_at(v, "ts") else { continue };
        if let (Some(session), Some(c)) = (str_at(v, "session"), v.get("cache").filter(|c| c.is_object())) {
            let causes = c.get("causes").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, n)| Some((k.clone(), n.as_u64()?))).collect()).unwrap_or_default();
            out.cache_misses.push(CacheMissReport {
                session_id: session.to_owned(),
                ts_ms: ts,
                last_miss_ms: i64_at(c, "last"),
                requests: u64_at(c, "requests"),
                misses: u64_at(c, "misses"),
                rebuilds: u64_at(c, "rebuilds"),
                recached_tokens: u64_at(c, "recached"),
                causes,
            });
            out.lines_recognised += 1;
            continue;
        }
        let Some(windows) = v.get("windows").and_then(Value::as_object) else { continue };
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
        if out.round_full() {
            break;
        }
    }
    out.next_offset = lines.finish()?;
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

    #[test]
    fn a_full_file_is_rotated_once_by_the_bridge_holding_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (data, path, old, lock) = (dir.path(), capture_file(dir.path()), rotated_file(dir.path()), rotate_lock(dir.path()));
        let mut w = extract_windows(&input()).unwrap();
        let mut next = |at: i64| {
            w.insert("five_hour".into(), json!({"used_percentage": at as f64, "resets_at": 1738425600}));
            record(data, &w, at)
        };
        let fill = || OpenOptions::new().write(true).open(&path).unwrap().set_len(ROTATE_BYTES + 1).unwrap();
        assert!(next(1));
        fill();
        fs::write(&lock, b"").unwrap();
        assert!(next(2));
        assert!(!old.exists(), "another bridge is rotating");
        fs::remove_file(&lock).unwrap();
        assert!(next(3));
        assert!(fs::metadata(&old).unwrap().len() > ROTATE_BYTES && !lock.exists());
        // a bridge that saw the full file before the rotation leaves the fresh one where it is
        rotate(data, &path);
        assert!(fs::metadata(&old).unwrap().len() > ROTATE_BYTES);
        assert_eq!(parse_file(&path, 0).unwrap().lines_recognised, 1);

        fill();
        let held = fs::File::create(&lock).unwrap();
        held.set_modified(std::time::SystemTime::now() - 2 * STALE_LOCK).unwrap();
        drop(held);
        assert!(next(4));
        assert!(!lock.exists(), "a lock left behind is cleared");
        assert!(next(5));
        assert_eq!(parse_file(&path, 0).unwrap().lines_recognised, 1);
    }

    #[test]
    fn a_paused_bridge_records_nothing_and_records_can_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let w = extract_windows(&input()).unwrap();
        assert!(record(dir.path(), &w, 1_000));
        delete_records(dir.path()).unwrap();
        assert!(!capture_file(dir.path()).exists());
        set_paused(dir.path(), true).unwrap();
        assert!(!record(dir.path(), &w, 2_000));
        assert!(!capture_file(dir.path()).exists());
        set_paused(dir.path(), false).unwrap();
        set_paused(dir.path(), false).unwrap();
        assert!(record(dir.path(), &w, 3_000));
    }

    fn request(id: &str, at: &str, read: u64, write_5m: u64, write_1h: u64) -> Value {
        json!({"type": "assistant", "sessionId": "s", "timestamp": format!("2026-11-13T{at}.000Z"), "message": {"id": id, "model": "claude-opus-5-5", "usage": {
            "input_tokens": 3, "output_tokens": 10, "cache_read_input_tokens": read, "cache_creation_input_tokens": write_5m + write_1h,
            "cache_creation": {"ephemeral_5m_input_tokens": write_5m, "ephemeral_1h_input_tokens": write_1h}}}})
    }

    fn ms(at: &str) -> i64 {
        crate::model::parse_ts_ms(&format!("2026-11-13T{at}.000Z")).unwrap()
    }

    fn transcript(lines: &[Value]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        (dir, path)
    }

    #[test]
    fn the_cache_lasts_its_written_lifetime_after_the_last_request_that_used_it() {
        let mut streaming = request("msg_b", "10:10:00", 0, 0, 0);
        streaming["message"]["usage"] = json!({"input_tokens": 0, "output_tokens": 0});
        let mut subagent = request("msg_c", "10:20:00", 900, 100, 0);
        subagent["isSidechain"] = json!(true);
        let mut failed = request("msg_d", "10:25:00", 0, 0, 0);
        failed["message"]["model"] = json!("<synthetic>");
        let mut lines = vec![request("msg_a", "10:00:00", 1000, 0, 500), streaming, request("msg_b", "10:10:00", 1500, 0, 0), subagent, failed];
        let (_dir, path) = transcript(&lines);
        // the hour the cache was written for, from the last request that read it
        assert_eq!(cache_expiry(&path, ms("10:30:00")), Some(ms("11:10:00")));
        assert_eq!(cache_expiry(&path, ms("11:10:00")), None, "it ran out");

        lines.push(request("msg_e", "10:30:00", 2000, 300, 0));
        let (_dir, path) = transcript(&lines);
        assert_eq!(cache_expiry(&path, ms("10:31:00")), Some(ms("10:35:00")), "written partly for five minutes");
        let (_dir, path) = transcript(&[request("msg_f", "10:30:00", 0, 0, 0)]);
        assert_eq!(cache_expiry(&path, ms("10:31:00")), None, "no cache in use");
        assert_eq!(cache_expiry(Path::new("/no/such/transcript.jsonl"), 0), None);
    }

    #[test]
    fn claude_codes_own_cache_time_comes_first() {
        let (_dir, path) = transcript(&[request("msg_a", "10:00:00", 1000, 0, 500)]);
        let t = path.to_str().unwrap();
        let input = |cache: Value| json!({"transcript_path": t, "prompt_cache": cache});
        let at = ms("10:04:00");
        assert_eq!(session_cache_expiry(&input(json!({"warm": true, "ttl": "5m", "expires_at": at / 1000})), ms("10:01:00")), Some(at));
        assert_eq!(session_cache_expiry(&input(json!({"warm": false, "expires_at": null})), ms("10:01:00")), None, "cold, whatever the transcript says");
        assert_eq!(session_cache_expiry(&input(json!({"warm": true, "expires_at": at / 1000})), at), None);
        // an older Claude Code sends no prompt_cache
        assert_eq!(session_cache_expiry(&json!({"transcript_path": t}), ms("10:01:00")), Some(ms("11:00:00")));
        assert_eq!(session_cache_expiry(&json!({}), 0), None);
    }

    #[test]
    fn a_larger_end_of_the_transcript_is_read_when_the_last_write_is_further_back() {
        let tool_result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t", "content": "x".repeat(400 * 1024)}]}});
        let (_dir, path) = transcript(&[request("msg_a", "10:00:00", 1000, 0, 500), tool_result, request("msg_b", "10:05:00", 1500, 0, 0)]);
        assert_eq!(cache_expiry(&path, ms("10:06:00")), Some(ms("11:05:00")));
    }

    #[test]
    fn the_cache_time_is_a_local_clock_time_at_the_end_of_the_line() {
        let tz = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        let at = ms("10:32:59");
        assert_eq!(cache_text(at, false, &tz).unwrap(), "cache until 13:32");
        assert_eq!(cache_text(at, true, &tz).unwrap(), "önbellek 13:32 silinir");
        assert_eq!(append("5h 24% · 7d 41%", "cache until 13:32"), "5h 24% · 7d 41% · cache until 13:32");
        assert_eq!(append("", "cache until 13:32"), "cache until 13:32");
        assert_eq!(append("first\nsecond\n", "x"), "first\nsecond · x");
        assert_eq!(append("\u{1b}[32mmine\n", "x"), "\u{1b}[32mmine\u{1b}[0m · x");
    }

    #[test]
    fn options_are_read_back_and_a_missing_file_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_options(dir.path()), Options::default());
        let on = Options { cache: true, turkish: true };
        write_options(dir.path(), &on).unwrap();
        assert_eq!(read_options(dir.path()), on);
        fs::write(options_file(dir.path()), b"{").unwrap();
        assert_eq!(read_options(dir.path()), Options::default());
    }

    #[test]
    fn cache_misses_are_written_once_each_time_they_change_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let report = |misses: u64, rebuilds: u64, causes: Value| json!({"session_id": "s-1", "prompt_cache": {"requests": 14, "misses": misses, "expected_rebuilds": rebuilds, "miss_recache_tokens": 310_200, "last_miss_at": 1_738_425_230, "miss_causes": causes}});
        assert!(!record_cache_misses(dir.path(), &report(0, 0, json!({})), 1_000), "nothing to tell");
        assert!(record_cache_misses(dir.path(), &report(1, 0, json!({"tools_changed": 1})), 2_000));
        assert!(!record_cache_misses(dir.path(), &report(1, 0, json!({"tools_changed": 1})), 3_000), "unchanged");
        assert!(record_cache_misses(dir.path(), &report(2, 1, json!({"tools_changed": 1, "ttl_expired_5m": 1, "Bad Name": 3, "negative": -1})), 4_000));
        assert!(!record_cache_misses(dir.path(), &json!({"session_id": "s-1"}), 5_000), "an older Claude Code sends none");
        assert!(!record_cache_misses(dir.path(), &json!({"session_id": "../x", "prompt_cache": {"misses": 1}}), 5_000));

        let out = parse_file(&capture_file(dir.path()), 0).unwrap();
        assert_eq!(out.cache_misses.len(), 2);
        let last = &out.cache_misses[1];
        assert_eq!((last.session_id.as_str(), last.ts_ms, last.requests, last.misses, last.rebuilds, last.recached_tokens), ("s-1", 4_000, 14, 2, 1, 310_200));
        assert_eq!(last.last_miss_ms, Some(1_738_425_230_000));
        assert_eq!(last.causes, [("tools_changed".to_owned(), 1), ("ttl_expired_5m".to_owned(), 1)], "only names as Claude Code writes them");
        assert!(out.limits.is_empty());

        set_paused(dir.path(), true).unwrap();
        assert!(!record_cache_misses(dir.path(), &report(3, 1, json!({})), 6_000));
        set_paused(dir.path(), false).unwrap();
        delete_records(dir.path()).unwrap();
        assert!(record_cache_misses(dir.path(), &report(2, 1, json!({})), 7_000), "deleted with the records");
    }
}
