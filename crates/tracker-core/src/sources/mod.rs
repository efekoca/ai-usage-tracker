//! Source parsers. Each parser is tolerant: unknown fields are ignored, malformed lines are
//! counted as warnings and skipped, and nothing ever panics on unexpected input.

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
        }
    }
    /// Whole-file JSON documents are re-read completely when they change;
    /// JSONL files are read incrementally from a byte offset.
    pub fn is_incremental(self) -> bool {
        !matches!(self, ParserKind::ClaudePlanHistory)
    }
}

#[derive(Debug, Default)]
pub struct ParseOutput {
    pub events: Vec<UsageEvent>,
    pub limits: Vec<LimitSnapshot>,
    pub tool_calls: Vec<ToolCall>,
    /// Outcomes that arrive after their call (`(call key, failed)`); applied to stored calls.
    pub tool_results: Vec<(String, bool)>,
    /// Human-readable, content-free warnings (line numbers / offsets only).
    pub warnings: Vec<String>,
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

/// A trailing line without `\n` may still be being written, unless it is already whole JSON.
pub fn read_jsonl_from(path: &Path, offset: u64) -> std::io::Result<(Vec<Line>, u64)> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut r = BufReader::with_capacity(1 << 16, f);
    let mut pos = offset;
    let mut out = Vec::new();
    let mut buf = Vec::with_capacity(4096);
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        let mut slice: &[u8] = &buf;
        if pos == 0 && slice.starts_with(&[0xEF, 0xBB, 0xBF]) {
            slice = &slice[3..];
        }
        let trimmed = trim_ascii(slice);
        let value: Option<Value> = if trimmed.is_empty() { None } else { serde_json::from_slice(trimmed).ok() };
        if buf.last() != Some(&b'\n') && !value.as_ref().is_some_and(Value::is_object) {
            break;
        }
        if !trimmed.is_empty() {
            out.push(Line { offset: pos, value });
        }
        pos += n as u64;
    }
    Ok((out, pos))
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
