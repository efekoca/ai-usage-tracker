//! Source parsers. Each parser is tolerant: unknown fields are ignored, malformed lines are
//! counted as warnings and skipped, and nothing ever panics on unexpected input.

pub mod antigravity;
pub mod claude_code;
pub mod claude_plan;
pub mod codex;
pub mod cowork_audit;

use crate::model::{LimitSnapshot, ToolCall, UsageEvent};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParserKind {
    ClaudeCodeJsonl,
    CoworkJsonl,
    CoworkAudit,
    ClaudePlanHistory,
    CodexRollout,
    /// This app's own Claude Code status-line capture file.
    StatuslineCapture,
    AntigravityDb,
}

impl ParserKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ParserKind::ClaudeCodeJsonl => "claude_code_jsonl",
            ParserKind::CoworkJsonl => "cowork_jsonl",
            ParserKind::CoworkAudit => "cowork_audit",
            ParserKind::ClaudePlanHistory => "claude_plan_history",
            ParserKind::CodexRollout => "codex_rollout",
            ParserKind::StatuslineCapture => "statusline_capture",
            ParserKind::AntigravityDb => antigravity::SOURCE,
        }
    }
    /// Whole-file documents (JSON, SQLite) are re-read completely when they change;
    /// JSONL files are read incrementally from a byte offset.
    pub fn is_incremental(self) -> bool {
        !matches!(self, ParserKind::ClaudePlanHistory | ParserKind::AntigravityDb)
    }
}

#[derive(Debug, Default)]
pub struct ParseOutput {
    pub events: Vec<UsageEvent>,
    pub limits: Vec<LimitSnapshot>,
    pub tool_calls: Vec<ToolCall>,
    /// Outcomes that arrive after their call (`(call key, failed)`); applied to stored calls.
    pub tool_results: Vec<(String, bool)>,
    /// Human-readable, content-free warnings (line numbers / offsets only), at most
    /// `MAX_WARNING_DETAILS`; the last one is always the latest. Added with `warn`.
    pub warnings: Vec<String>,
    pub warning_count: usize,
    /// Byte offset up to which the file has been fully consumed.
    pub next_offset: u64,
    /// Parser-specific state carried to the next incremental read.
    pub state: Value,
    pub lines_total: u64,
    pub lines_recognised: u64,
    /// Keys an older version stored for this file's records; replaced when the file is re-read.
    pub stale_events: Vec<String>,
    pub stale_tool_calls: Vec<String>,
    /// (source, ts_ms) of readings an older version stored from copied history.
    pub stale_limits: Vec<(String, i64)>,
    /// Events copied from another thread's log: (index in `events`, that thread). The original
    /// may be stored under an older key while its own log is gone.
    pub copies: Vec<(usize, String)>,
}

/// One complete JSONL line with the byte offset at which it starts.
pub struct Line {
    pub offset: u64,
    pub value: Option<Value>,
}

/// Longer lines are skipped without their content ever being held in memory.
pub const MAX_LINE: usize = 64 << 20;

/// A file of damaged lines must not hold one message per line in memory.
pub const MAX_WARNING_DETAILS: usize = 100;
/// Records per read: a longer file is read in rounds, each stored with its own checkpoint.
pub const CHUNK_RECORDS: usize = 10_000;
static ROUND_RECORDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(CHUNK_RECORDS);

/// Tests run every ingest scenario with tiny rounds to prove they store the same result.
#[doc(hidden)]
pub fn set_round_records(n: usize) {
    ROUND_RECORDS.store(n.max(1), std::sync::atomic::Ordering::Relaxed);
}

impl ParseOutput {
    pub fn warn(&mut self, w: impl Into<String>) {
        self.warning_count += 1;
        if self.warnings.len() < MAX_WARNING_DETAILS {
            self.warnings.push(w.into());
        } else if let Some(last) = self.warnings.last_mut() {
            *last = w.into();
        }
    }

    /// The parser stops after the line that fills a round; the next round resumes from there.
    pub fn round_full(&self) -> bool {
        self.events.len() + self.limits.len() + self.tool_calls.len() + self.tool_results.len()
            >= ROUND_RECORDS.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Reads a JSONL file from a byte offset one line at a time, so only one line is in memory.
/// A trailing line without `\n` may still be being written, unless it is already whole JSON.
pub struct JsonlReader {
    r: BufReader<File>,
    pos: u64,
    buf: Vec<u8>,
    error: Option<std::io::Error>,
    done: bool,
}

impl JsonlReader {
    pub fn open(path: &Path, offset: u64) -> std::io::Result<JsonlReader> {
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(offset))?;
        Ok(JsonlReader { r: BufReader::with_capacity(1 << 16, f), pos: offset, buf: Vec::with_capacity(4096), error: None, done: false })
    }

    /// The offset up to which the file has been consumed, or the read error that stopped it.
    pub fn finish(self) -> std::io::Result<u64> {
        self.error.map_or(Ok(self.pos), Err)
    }

    /// Reads up to and including the next `\n`; returns the byte count and whether the line
    /// was too long to keep (its bytes are consumed but dropped).
    fn read_line(&mut self) -> std::io::Result<(usize, bool)> {
        self.buf.clear();
        let (mut n, mut over) = (0, false);
        loop {
            let chunk = self.r.fill_buf()?;
            if chunk.is_empty() {
                return Ok((n, over));
            }
            let (take, end) = match chunk.iter().position(|&b| b == b'\n') {
                Some(i) => (i + 1, true),
                None => (chunk.len(), false),
            };
            if !over && self.buf.len() + take <= MAX_LINE {
                self.buf.extend_from_slice(&chunk[..take]);
            } else {
                over = true;
                self.buf.clear();
            }
            self.r.consume(take);
            n += take;
            if end {
                if over {
                    self.buf.push(b'\n');
                }
                return Ok((n, over));
            }
        }
    }
}

impl Iterator for JsonlReader {
    type Item = Line;

    fn next(&mut self) -> Option<Line> {
        while !self.done {
            let (n, over) = match self.read_line() {
                Ok(r) => r,
                Err(e) => {
                    self.error = Some(e);
                    break;
                }
            };
            if n == 0 {
                break;
            }
            let mut slice: &[u8] = &self.buf;
            if self.pos == 0 && slice.starts_with(&[0xEF, 0xBB, 0xBF]) {
                slice = &slice[3..];
            }
            let trimmed = trim_ascii(slice);
            let value: Option<Value> = if trimmed.is_empty() || over { None } else { serde_json::from_slice(trimmed).ok() };
            if self.buf.last() != Some(&b'\n') && !value.as_ref().is_some_and(Value::is_object) {
                break;
            }
            let offset = self.pos;
            self.pos += n as u64;
            if over || !trimmed.is_empty() {
                return Some(Line { offset, value });
            }
        }
        self.done = true;
        None
    }
}

fn trim_ascii(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if first.is_ascii_whitespace() { s = rest } else { break }
    }
    while let [rest @ .., last] = s {
        if last.is_ascii_whitespace() { s = rest } else { break }
    }
    s
}

pub(crate) fn u64_at(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)).unwrap_or(0),
        _ => 0,
    }
}

pub(crate) fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub(crate) fn f64_at(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

pub(crate) fn i64_at(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_line_is_skipped_and_the_lines_after_it_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.jsonl");
        let mut text = String::from("{\"a\":1}\n{\"big\":\"");
        text.push_str(&"x".repeat(MAX_LINE));
        text.push_str("\"}\n{\"b\":2}\n{\"c\":");
        std::fs::write(&p, &text).unwrap();
        let mut r = JsonlReader::open(&p, 0).unwrap();
        let lines: Vec<Line> = r.by_ref().collect();
        let values: Vec<Option<Value>> = lines.iter().map(|l| l.value.clone()).collect();
        assert_eq!(values, [Some(serde_json::json!({"a": 1})), None, Some(serde_json::json!({"b": 2}))]);
        assert_eq!(lines[2].offset as usize, text.rfind("{\"b\"").unwrap());
        // the unfinished last line is left for the next read
        assert_eq!(r.finish().unwrap() as usize, text.rfind("{\"c\"").unwrap());
    }
}
