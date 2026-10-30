//! OpenCode (`anomalyco/opencode`, formerly `sst/opencode`). Since 1.2 every session lives in
//! one SQLite database, `opencode.db` in its data folder (`$XDG_DATA_HOME/opencode`, else
//! `~/.local/share/opencode` on every system).
//!
//! * `part` rows of type `step-finish` are written once per model request with that request's
//!   `tokens {input, output, reasoning, cache {read, write}}`. They are the billable unit: the
//!   message keeps only the last step's tokens, so the two are never added together.
//! * Since 1.3.16 `input` excludes cache reads and writes and `output` excludes reasoning, for
//!   every provider. Sessions written by an older version used other meanings and are skipped.
//! * `message.data` names the provider (`providerID`, kept as the client), model (`modelID`) and
//!   agent; the request started when the message was created. A step's part is written after its
//!   tool calls ran.
//! * A subagent is a child session (`parent_id`) with its own requests; compaction is a real
//!   request (`summary: true`). Title generation is not recorded by OpenCode at all.
//! * The stored `cost` is not used: it is 0 for subscription logins and in Copilot units.
//! * Ids encode their creation time in 48 bits that wrap every ~795 days, so they cannot mark
//!   how far the database was read. Rows are updated in place, so offsets cannot either.
//! * Forking a session copies every message and part under new ids, times and counts kept. Keys
//!   are therefore built from what a copy keeps, and the oldest session is read first, so a
//!   request counts once, for the session it was made in.
//!
//! The database is opened read-only (`query_only`) and read in one transaction. Only sessions
//! whose parts or messages changed since the last read are read again.

use super::ParseOutput;
use crate::model::{Accuracy, Tokens, Tool, ToolCall, UsageEvent};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;

pub const SOURCE: &str = "opencode_db";

/// The first version whose token fields mean what this reader expects.
const MIN_VERSION: [u64; 3] = [1, 3, 16];
/// When 1.3.16 was released (2026-04-04 UTC); used for sessions whose version cannot be read.
const MIN_VERSION_MS: i64 = 1_775_260_800_000;
/// Counts beyond this are a damaged record, as elsewhere in the archive.
const MAX_COUNT: f64 = 1e12;

struct Session {
    parent: Option<String>,
    directory: Option<String>,
    counted: bool,
    created: i64,
}

pub fn parse_file(path: &Path, state: &Value) -> std::io::Result<ParseOutput> {
    read(path, state).map_err(std::io::Error::other)
}

fn read(path: &Path, state: &Value) -> Result<ParseOutput, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = ParseOutput::default();
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA query_only = ON; BEGIN")?;
    if !["session", "message", "part"].iter().all(|t| has_table(&conn, t)) {
        out.warn("not an OpenCode database");
        out.state = state.clone();
        return Ok(out);
    }

    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut skipped = 0usize;
    {
        let mut q = conn.prepare("SELECT id, parent_id, directory, version, time_created FROM session")?;
        let rows = q.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<i64>>(4)?))
        })?;
        for row in rows {
            let (id, parent, directory, version, created) = row?;
            let counted = supported(version.as_deref(), created);
            skipped += usize::from(!counted);
            let directory = directory.filter(|d| !d.is_empty()).map(native_path);
            sessions.insert(id, Session { parent: parent.filter(|p| !p.is_empty()), directory, counted, created: created.unwrap_or_default() });
        }
    }
    if skipped > 0 {
        out.warn(format!("{skipped} sessions from OpenCode before 1.3.16 are not counted"));
    }

    // Both come from indexes. Parts alone miss a revert followed by as many new parts (SQLite
    // hands the removed row ids out again); a message written after a revert is always newer.
    let mut marks: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        let mut q = conn.prepare("SELECT session_id, count(*) FROM part GROUP BY session_id")?;
        for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (id, parts) = row?;
            marks.entry(id).or_default().0 = parts;
        }
        let mut q = conn.prepare("SELECT session_id, count(*), max(time_created) FROM message GROUP BY session_id")?;
        for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)))? {
            let (id, messages, newest) = row?;
            let m = marks.entry(id).or_default();
            (m.1, m.2) = (messages, newest.unwrap_or_default());
        }
    }
    let before = state.get("sessions").and_then(Value::as_object);
    let mut seen = Map::new();
    let mut changed = Vec::new();
    for (id, (parts, messages, newest)) in marks {
        let mark = Value::from(format!("{parts}:{messages}:{newest}"));
        if sessions.get(&id).is_some_and(|s| s.counted) && before.and_then(|b| b.get(&id)) != Some(&mark) {
            changed.push(id.clone());
        }
        seen.insert(id, mark);
    }

    changed.sort_by_key(|id| (sessions.get(id).map_or(0, |s| s.created), id.clone()));
    for id in &changed {
        read_session(&conn, &sessions, id, &mut out)?;
    }
    conn.execute_batch("COMMIT")?;
    out.state = json!({ "sessions": seen });
    Ok(out)
}

fn read_session(conn: &Connection, sessions: &HashMap<String, Session>, id: &str, out: &mut ParseOutput) -> rusqlite::Result<()> {
    let root = root_of(sessions, id);
    let child = root != id;
    let project = sessions.get(id).and_then(|s| s.directory.clone()).or_else(|| sessions.get(root).and_then(|s| s.directory.clone()));
    let mut q = conn.prepare(
        "SELECT p.id, p.time_created, m.time_created,
                json_extract(p.data, '$.type'),
                json_extract(p.data, '$.tokens.input'), json_extract(p.data, '$.tokens.output'),
                json_extract(p.data, '$.tokens.reasoning'), json_extract(p.data, '$.tokens.cache.read'),
                json_extract(p.data, '$.tokens.cache.write'),
                json_extract(p.data, '$.tool'), json_extract(p.data, '$.state.status'),
                json_extract(p.data, '$.state.time.start'),
                json_extract(m.data, '$.providerID'), json_extract(m.data, '$.modelID'),
                json_extract(m.data, '$.agent'), json_extract(m.data, '$.summary'),
                json_extract(m.data, '$.time.created'),
                row_number() OVER (PARTITION BY p.message_id, json_extract(p.data, '$.type') ORDER BY p.time_created, p.id),
                json_extract(p.data, '$.callID')
         FROM part p JOIN message m ON m.id = p.message_id
         WHERE p.session_id = ?1 AND json_extract(p.data, '$.type') IN ('step-finish', 'tool')
         ORDER BY p.time_created, p.id",
    )?;
    let rows = q.query_map([id], |r| {
        Ok(Row {
            part: r.get(0)?,
            part_ms: r.get(1)?,
            message_ms: r.get(2)?,
            kind: r.get(3)?,
            counts: [r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?],
            tool: r.get(9)?,
            status: r.get(10)?,
            tool_start: r.get(11)?,
            provider: r.get(12)?,
            model: r.get(13)?,
            agent: r.get(14)?,
            summary: r.get::<_, Option<i64>>(15)?.is_some_and(|v| v != 0),
            created: r.get(16)?,
            step: r.get(17)?,
            call: r.get(18)?,
        })
    })?;
    for row in rows {
        out.lines_total += 1;
        // a field of an unexpected type damages only its own row
        let row = match row {
            Err(rusqlite::Error::InvalidColumnType(..) | rusqlite::Error::FromSqlConversionFailure(..) | rusqlite::Error::IntegralValueOutOfRange(..)) => {
                out.warn(format!("session {id}: a part with unexpected fields was skipped"));
                continue;
            }
            r => r?,
        };
        // a subagent's requests count toward the session that started it, as for Claude Code
        let agent = if child {
            row.agent.clone().filter(|a| !a.is_empty()).or(Some("subagent".into()))
        } else if row.summary || row.agent.as_deref() == Some("compaction") {
            Some("compact".into())
        } else {
            None
        };
        match row.kind.as_deref() {
            Some("step-finish") => {
                let Some(tokens) = tokens(&row.counts) else {
                    out.warn(format!("step {}: implausible token count", row.part));
                    continue;
                };
                // a request that failed before its usage arrived is stored with zeros; one made
                // before 1.3.16 existed reached a newer session only as a copy, with the old meanings
                if tokens.total() == 0 || row.created.is_some_and(|c| c < MIN_VERSION_MS) {
                    continue;
                }
                out.lines_recognised += 1;
                let provider = row.provider.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| "unknown".into());
                let key = match row.created {
                    Some(at) => {
                        let t = &tokens;
                        let model = row.model.as_deref().unwrap_or("");
                        format!("oc:{at}:{provider}:{model}:{}:{}:{}:{}:{}:{}", row.step, t.input, t.output, t.reasoning, t.cache_read, t.cache_write)
                    }
                    None => format!("oc:{}", row.part),
                };
                out.events.push(UsageEvent {
                    key,
                    ts_ms: row.created.or(row.message_ms).or(row.part_ms).unwrap_or_default(),
                    tool: Tool::OpenCode,
                    client: Some(provider),
                    model: row.model.as_deref().map(normalize_model).filter(|m| !m.is_empty()).unwrap_or_else(|| "unknown".into()),
                    project_path: project.clone(),
                    session_id: Some(root.to_owned()),
                    request_input: tokens.input.saturating_add(tokens.cache_read).saturating_add(tokens.cache_write),
                    tokens,
                    web_search_requests: 0,
                    speed: None,
                    service_tier: None,
                    inference_geo: None,
                    request_id: None,
                    accuracy: Accuracy::Exact,
                    source: SOURCE.into(),
                    branch: None,
                    agent,
                    thread_id: child.then(|| id.to_owned()),
                    limit_reset: None,
                    creator: None,
                });
            }
            Some("tool") => {
                let Some(name) = row.tool.clone().filter(|n| !n.is_empty()) else { continue };
                out.lines_recognised += 1;
                let ts_ms = row.tool_start.or(row.part_ms).unwrap_or_default();
                out.tool_calls.push(ToolCall {
                    key: match row.call.as_deref().filter(|c| !c.is_empty()) {
                        Some(call) => format!("oct:{call}:{ts_ms}"),
                        None => format!("oct:{}", row.part),
                    },
                    ts_ms,
                    tool: Tool::OpenCode,
                    session_id: Some(root.to_owned()),
                    project_path: project.clone(),
                    agent: if child { agent } else { None },
                    name,
                    failed: match row.status.as_deref() {
                        Some("completed") => Some(false),
                        Some("error") => Some(true),
                        _ => None,
                    },
                });
            }
            _ => {}
        }
    }
    Ok(())
}

struct Row {
    part: String,
    part_ms: Option<i64>,
    message_ms: Option<i64>,
    kind: Option<String>,
    counts: [Option<f64>; 5],
    tool: Option<String>,
    status: Option<String>,
    tool_start: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
    agent: Option<String>,
    summary: bool,
    created: Option<i64>,
    /// Position among the message's parts of the same type; a copy keeps the order.
    step: i64,
    call: Option<String>,
}

/// `[input, output, reasoning, cache read, cache write]` in OpenCode's meaning (output without
/// reasoning) to this app's (output with reasoning); `None` for a negative, fractional or absurd count.
fn tokens(counts: &[Option<f64>; 5]) -> Option<Tokens> {
    let mut v = [0u64; 5];
    for (i, c) in counts.iter().enumerate() {
        let c = c.unwrap_or(0.0);
        if !(0.0..=MAX_COUNT).contains(&c) || c.fract() != 0.0 {
            return None;
        }
        v[i] = c as u64;
    }
    let [input, output, reasoning, cache_read, cache_write] = v;
    Some(Tokens { input, cache_read, cache_write, cache_write_1h: 0, output: output + reasoning, reasoning })
}

/// The top-level session of a subagent chain; a broken or circular chain stops where it is.
fn root_of<'a>(sessions: &'a HashMap<String, Session>, id: &'a str) -> &'a str {
    let mut cur = id;
    for _ in 0..64 {
        match sessions.get(cur).and_then(|s| s.parent.as_deref()) {
            Some(p) if sessions.contains_key(p) => cur = p,
            _ => break,
        }
    }
    cur
}

fn supported(version: Option<&str>, created_ms: Option<i64>) -> bool {
    match version.and_then(parse_version) {
        Some(v) => v >= MIN_VERSION,
        None => created_ms.is_some_and(|c| c >= MIN_VERSION_MS),
    }
}

/// `1.19.4`, `1.3.16-beta.2` → `[1, 19, 4]`, `[1, 3, 16]`.
fn parse_version(s: &str) -> Option<[u64; 3]> {
    let mut parts = s.trim().trim_start_matches('v').split(['.', '-', '+']);
    let mut v = [0u64; 3];
    for slot in &mut v {
        *slot = parts.next()?.parse().ok()?;
    }
    Some(v)
}

/// One model under the names providers give it, so it is priced and grouped like the same model
/// from its own vendor: `anthropic/claude-sonnet-4.5` (OpenRouter), `claude-sonnet-4.6`
/// (Copilot), `claude-sonnet-4-5@20250929` (Vertex), `eu.anthropic.claude-opus-5` (Bedrock).
pub fn normalize_model(model: &str) -> String {
    let mut m = model.trim();
    if let Some((_, last)) = m.rsplit_once('/') {
        m = last;
    }
    for suffix in [":thinking", ":beta"] {
        m = m.strip_suffix(suffix).unwrap_or(m);
    }
    let mut s = bedrock_base(m).unwrap_or(m).to_owned();
    if let Some((base, tag)) = s.split_once('@') {
        s = if tag.len() == 8 && tag.bytes().all(|b| b.is_ascii_digit()) { format!("{base}-{tag}") } else { base.to_owned() };
    }
    if s.to_ascii_lowercase().starts_with("claude-") {
        // Anthropic writes versions with dashes; a dot between digits is another provider's spelling
        let b = s.as_bytes();
        s = s.char_indices().map(|(i, c)| if c == '.' && i > 0 && b[i - 1].is_ascii_digit() && b.get(i + 1).is_some_and(u8::is_ascii_digit) { '-' } else { c }).collect();
    }
    s
}

/// Bedrock ids: `[region.]vendor.model[-vN[:M]]` → `model`.
fn bedrock_base(m: &str) -> Option<&str> {
    const VENDORS: [&str; 8] = ["anthropic.", "openai.", "meta.", "amazon.", "mistral.", "cohere.", "deepseek.", "qwen."];
    let rest = match m.split_once('.') {
        Some((region, rest)) if !VENDORS.iter().any(|v| m.starts_with(v)) && region.len() <= 8 && region.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') => rest,
        _ => m,
    };
    let model = VENDORS.iter().find_map(|v| rest.strip_prefix(v))?;
    let model = match model.rsplit_once("-v") {
        Some((base, ver)) if !ver.is_empty() && ver.split(':').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) => base,
        _ => model,
    };
    Some(model)
}

/// OpenCode stores Windows paths with forward slashes (`C:/Users/…`); other tools use the
/// system's own, and one folder must be one project.
fn native_path(p: String) -> String {
    let b = p.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/';
    if drive || p.starts_with("//") { p.replace('/', "\\") } else { p }
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(())).optional().ok().flatten().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_spellings_of_a_model_become_its_vendor_name() {
        for (raw, want) in [
            ("claude-sonnet-4-5", "claude-sonnet-4-5"),
            ("anthropic/claude-sonnet-4.5", "claude-sonnet-4-5"),
            ("claude-sonnet-4.6", "claude-sonnet-4-6"),
            ("claude-opus-4.1", "claude-opus-4-1"),
            ("claude-sonnet-4-5@20250929", "claude-sonnet-4-5-20250929"),
            ("claude-opus-5@default", "claude-opus-5"),
            ("eu.anthropic.claude-opus-5", "claude-opus-5"),
            ("anthropic.claude-sonnet-4-5-20250929-v1:0", "claude-sonnet-4-5-20250929"),
            ("global.anthropic.claude-haiku-4-5", "claude-haiku-4-5"),
            ("us-gov.anthropic.claude-sonnet-4-5-20250929-v1:0", "claude-sonnet-4-5-20250929"),
            ("openai.gpt-5.4", "gpt-5.4"),
            ("openai/gpt-5-nano", "gpt-5-nano"),
            ("gpt-5.4-nano", "gpt-5.4-nano"),
            ("google/gemini-3.7-flash", "gemini-3.7-flash"),
            ("anthropic/claude-sonnet-4.5:thinking", "claude-sonnet-4-5"),
            ("qwen/qwen3-coder:free", "qwen3-coder:free"),
            ("big-pickle", "big-pickle"),
        ] {
            assert_eq!(normalize_model(raw), want, "{raw}");
        }
    }

    #[test]
    fn windows_folders_get_backslashes_like_the_other_tools() {
        assert_eq!(native_path("C:/Users/Ada/app".into()), "C:\\Users\\Ada\\app");
        assert_eq!(native_path("//server/share/app".into()), "\\\\server\\share\\app");
        assert_eq!(native_path("/Users/ada/app".into()), "/Users/ada/app");
    }

    #[test]
    fn only_sessions_from_1_3_16_on_are_counted() {
        assert!(supported(Some("1.19.4"), None));
        assert!(supported(Some("1.3.16"), None));
        assert!(supported(Some("v1.4.0-beta.1"), None));
        assert!(!supported(Some("1.3.15"), Some(MIN_VERSION_MS + 1)));
        assert!(!supported(Some("1.2.0"), None));
        assert!(supported(Some("local"), Some(MIN_VERSION_MS)));
        assert!(!supported(Some("local"), Some(MIN_VERSION_MS - 1)));
        assert!(!supported(None, None));
    }

    #[test]
    fn reasoning_is_added_back_into_output_and_damaged_counts_are_refused() {
        let t = tokens(&[Some(400.0), Some(200.0), Some(100.0), Some(800.0), None]).unwrap();
        assert_eq!((t.input, t.output, t.reasoning, t.cache_read, t.cache_write), (400, 300, 100, 800, 0));
        assert!(tokens(&[Some(-1.0), None, None, None, None]).is_none());
        assert!(tokens(&[Some(1.5), None, None, None, None]).is_none());
        assert!(tokens(&[Some(2e12), None, None, None, None]).is_none());
    }
}
