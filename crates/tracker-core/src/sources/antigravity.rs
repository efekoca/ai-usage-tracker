//! Google Antigravity conversation databases (`~/.gemini/<surface>/conversations/<id>.db`).
//! The Antigravity 2.0 app, the IDE and the `agy` CLI run the same agent engine and each keeps
//! its own data folder: `antigravity`, `antigravity-ide`, `antigravity-cli`.
//!
//! Format notes (SQLite in WAL mode, protobuf blobs):
//! * `gen_metadata(idx, data)`: one row per model generation. `data.1` holds `4` = usage summed
//!   over the generation's API calls (`1` model enum, `2` uncached input, `3` output, `4` cache
//!   write, `5` cache read, `6` API provider, `9` thinking, `10` response), `17.2` = each call's
//!   own usage, `19` = model id, `20` = `{key, value}` pairs incl. `last_step_index`.
//!   Generations that failed before the model answered carry no usage and are skipped.
//! * `steps(idx, status, metadata, error_details)`: `metadata.1` = creation timestamp,
//!   `metadata.4` = the tool call a tool step runs (`1` call id, `2` tool name). A generation
//!   writes step `last_step_index + 1`.
//! * `trajectory_metadata_blob.data`: `1` = workspace (`1` URI, `4` git branch), `2` = creation
//!   time, `7` = workspace URI.
//! * `<surface>/conversation_summaries.db` names a subagent's parent conversation and agent.
//!
//! The live database is never opened: it is copied with its WAL to a private temporary folder
//! and read there, so Antigravity's files are not touched or locked.

use super::ParseOutput;
use crate::model::{Accuracy, Tokens, Tool, ToolCall, UsageEvent};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SOURCE: &str = "antigravity_db";

/// Step statuses that end a tool call successfully; anything with error details failed.
const STEP_DONE: i64 = 3;
const STEP_FAILED: [i64; 2] = [6, 7];

#[derive(Clone, Copy)]
enum Val<'a> {
    Int(u64),
    Bytes(&'a [u8]),
    Fixed,
}

/// Protobuf wire-format fields of one message; stops at the first malformed byte.
struct Fields<'a> {
    buf: &'a [u8],
    pos: usize,
}

fn fields(buf: &[u8]) -> Fields<'_> {
    Fields { buf, pos: 0 }
}

impl<'a> Fields<'a> {
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u64, Val<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        let item = (|| {
            let key = self.varint()?;
            let (field, wire) = (key >> 3, key & 7);
            let val = match wire {
                0 => Val::Int(self.varint()?),
                1 | 5 => {
                    let n = if wire == 1 { 8 } else { 4 };
                    self.buf.get(self.pos..self.pos + n)?;
                    self.pos += n;
                    Val::Fixed
                }
                2 => {
                    let n = usize::try_from(self.varint()?).ok()?;
                    let b = self.buf.get(self.pos..self.pos.checked_add(n)?)?;
                    self.pos += n;
                    Val::Bytes(b)
                }
                _ => return None,
            };
            (field != 0).then_some((field, val))
        })();
        if item.is_none() {
            self.pos = self.buf.len();
        }
        item
    }
}

fn int(buf: &[u8], field: u64) -> Option<u64> {
    fields(buf).find_map(|(f, v)| match v {
        Val::Int(n) if f == field => Some(n),
        _ => None,
    })
}

fn msgs(buf: &[u8], field: u64) -> impl Iterator<Item = &[u8]> {
    fields(buf).filter_map(move |(f, v)| match v {
        Val::Bytes(b) if f == field => Some(b),
        _ => None,
    })
}

fn msg(buf: &[u8], field: u64) -> Option<&[u8]> {
    msgs(buf, field).next()
}

fn text(buf: &[u8], field: u64) -> Option<String> {
    msg(buf, field).and_then(|b| std::str::from_utf8(b).ok()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// `google.protobuf.Timestamp` → epoch milliseconds.
fn timestamp_ms(buf: &[u8]) -> Option<i64> {
    let secs = i64::try_from(int(buf, 1)?).ok()?;
    let nanos = int(buf, 2).unwrap_or(0).min(999_999_999) as i64;
    Some(secs * 1000 + nanos / 1_000_000)
}

fn usage_tokens(u: &[u8]) -> Tokens {
    let n = |f| int(u, f).unwrap_or(0);
    let thinking = n(9);
    Tokens {
        input: n(2),
        cache_read: n(5),
        cache_write: n(4),
        cache_write_1h: 0,
        output: n(3).max(thinking + n(10)),
        reasoning: thinking,
    }
}

/// `file:///c%3A/Users/x` → `c:\Users\x`; `file:///home/x` → `/home/x`.
pub fn file_uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8(out).ok()?;
    // a host part (`file://server/share`) is a UNC path
    let path = match path.strip_prefix('/') {
        Some(p) if p.len() >= 2 && p.as_bytes()[0].is_ascii_alphabetic() && p.as_bytes()[1] == b':' => p.replace('/', "\\"),
        Some(_) => path,
        None => format!("//{path}"),
    };
    let trimmed = path.trim_end_matches(['/', '\\']).to_owned();
    (!trimmed.is_empty()).then(|| if trimmed.ends_with(':') { path } else { trimmed })
}

/// A private copy of a live SQLite database and its WAL, removed when dropped.
struct Snapshot {
    dir: PathBuf,
    conn: Option<Connection>,
}

impl Snapshot {
    fn open(db: &Path) -> std::io::Result<Snapshot> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir()
            .join(format!("ai-usage-tracker-ag-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir)?;
        let mut snap = Snapshot { dir, conn: None };
        let copy = snap.dir.join("c.db");
        let mut wal = db.as_os_str().to_owned();
        wal.push("-wal");
        let wal = PathBuf::from(wal);
        let stamp = || -> Vec<Option<(u64, std::time::SystemTime)>> {
            [db, wal.as_path()].iter().map(|p| std::fs::metadata(p).ok().and_then(|m| Some((m.len(), m.modified().ok()?)))).collect()
        };
        // a copy taken while Antigravity checkpoints can mix old and new pages: take it again
        for attempt in 0.. {
            let before = stamp();
            std::fs::copy(db, &copy)?;
            // the WAL holds everything written since Antigravity's last checkpoint
            match std::fs::copy(&wal, snap.dir.join("c.db-wal")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                Err(_) => {
                    let _ = std::fs::remove_file(snap.dir.join("c.db-wal"));
                }
                Ok(_) => {}
            }
            if stamp() == before || attempt == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(std::io::Error::other)?;
        snap.conn = Some(conn);
        Ok(snap)
    }

    fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("opened")
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        drop(self.conn.take());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The surface a data folder belongs to, used as the event's client.
pub fn client_for(data_dir: &Path) -> String {
    let name = data_dir.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.is_empty() { "antigravity".into() } else { name.replace('-', "_") }
}

/// Parent conversation and agent name of a subagent conversation, from the surface's summary DB.
fn subagent_of(data_dir: &Path, id: &str) -> Option<(String, Option<String>)> {
    let db = data_dir.join("conversation_summaries.db");
    if !db.is_file() {
        return None;
    }
    let snap = Snapshot::open(&db).ok()?;
    let (parent, agent): (String, String) = snap
        .conn()
        .query_row(
            "SELECT parent_conversation_id, agent_name FROM conversation_summaries WHERE conversation_id = ?1",
            [id],
            |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default())),
        )
        .optional()
        .ok()??;
    let parent = parent.trim();
    (!parent.is_empty() && parent != id).then(|| (parent.to_owned(), Some(agent.trim().to_owned()).filter(|a| !a.is_empty())))
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

struct Step {
    ts_ms: Option<i64>,
    status: i64,
    failed: bool,
    call: Option<(Option<String>, String)>,
}

pub fn parse_file(path: &Path) -> std::io::Result<ParseOutput> {
    read(path).map_err(std::io::Error::other)
}

fn read(path: &Path) -> Result<ParseOutput, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = ParseOutput::default();
    let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let data_dir = path.parent().and_then(Path::parent).unwrap_or(Path::new(""));
    let client = client_for(data_dir);
    let snap = Snapshot::open(path)?;
    let conn = snap.conn();
    if !has_table(conn, "gen_metadata") || !has_table(conn, "steps") {
        out.warnings.push("not an Antigravity conversation database".into());
        return Ok(out);
    }

    let (mut project, mut branch, mut created_ms) = (None, None, None);
    if has_table(conn, "trajectory_metadata_blob") {
        let blob: Option<Vec<u8>> =
            conn.query_row("SELECT data FROM trajectory_metadata_blob WHERE id = 'main'", [], |r| r.get(0)).optional()?.flatten();
        if let Some(b) = blob {
            let ws = msg(&b, 1);
            project = ws.and_then(|w| text(w, 1)).or_else(|| text(&b, 7)).and_then(|u| file_uri_to_path(&u));
            branch = ws.and_then(|w| text(w, 4));
            created_ms = msg(&b, 2).and_then(timestamp_ms);
        }
    }
    let (session, thread, agent) = match subagent_of(data_dir, &id) {
        Some((parent, agent)) => (parent, Some(id.clone()), agent),
        None => (id.clone(), None, None),
    };

    let mut steps: HashMap<i64, Step> = HashMap::new();
    {
        let mut q = conn.prepare("SELECT idx, status, metadata, error_details IS NOT NULL AND length(error_details) > 0 FROM steps")?;
        let rows = q.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<Vec<u8>>>(2)?, r.get::<_, bool>(3)?))
        })?;
        for row in rows {
            let (idx, status, meta, has_error) = row?;
            let meta = meta.unwrap_or_default();
            let call = msg(&meta, 4).and_then(|c| Some((text(c, 1), text(c, 2)?)));
            let failed = has_error || STEP_FAILED.contains(&status);
            steps.insert(idx, Step { ts_ms: msg(&meta, 1).and_then(timestamp_ms), status, failed, call });
        }
    }

    let mut q = conn.prepare("SELECT idx, data FROM gen_metadata ORDER BY idx")?;
    let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)))?;
    for row in rows {
        let (idx, data) = row?;
        out.lines_total += 1;
        let Some(g) = data.as_deref().and_then(|d| msg(d, 1)) else { continue };
        let Some(usage) = msg(g, 4) else { continue };
        let tokens = usage_tokens(usage);
        if tokens.total() == 0 {
            continue;
        }
        let Some(model) = text(g, 19) else {
            out.warnings.push(format!("generation {idx}: no model id"));
            continue;
        };
        let last_step = msgs(g, 20)
            .find(|kv| text(kv, 1).as_deref() == Some("last_step_index"))
            .and_then(|kv| text(kv, 2))
            .and_then(|v| v.parse::<i64>().ok());
        let step_ts = |i: i64| steps.get(&i).and_then(|s| s.ts_ms);
        let ts_ms = last_step.and_then(|l| step_ts(l + 1).or_else(|| step_ts(l))).or(created_ms);
        let Some(ts_ms) = ts_ms else {
            out.warnings.push(format!("generation {idx}: no timestamp"));
            continue;
        };
        let prompt = |u: &[u8]| int(u, 2).unwrap_or(0) + int(u, 4).unwrap_or(0) + int(u, 5).unwrap_or(0);
        // a retried generation sums its calls; the long-context tier depends on one request's prompt
        let request_input =
            msgs(g, 17).filter_map(|c| msg(c, 2)).map(prompt).max().unwrap_or_else(|| prompt(usage));
        out.lines_recognised += 1;
        out.events.push(UsageEvent {
            key: format!("ag:{id}:{idx}"),
            ts_ms,
            tool: Tool::Antigravity,
            client: Some(client.clone()),
            model,
            project_path: project.clone(),
            session_id: Some(session.clone()),
            tokens,
            request_input,
            web_search_requests: 0,
            speed: None,
            service_tier: None,
            inference_geo: None,
            request_id: None,
            accuracy: Accuracy::Exact,
            source: SOURCE.into(),
            branch: branch.clone(),
            agent: agent.clone(),
            thread_id: thread.clone(),
        });
    }

    let mut calls: Vec<(&i64, &Step)> = steps.iter().filter(|(_, s)| s.call.is_some()).collect();
    calls.sort_by_key(|(i, _)| **i);
    for (idx, s) in calls {
        let Some((call_id, name)) = &s.call else { continue };
        let Some(ts_ms) = s.ts_ms.or(created_ms) else { continue };
        let failed = if s.failed { Some(true) } else { (s.status == STEP_DONE).then_some(false) };
        out.tool_calls.push(ToolCall {
            key: match call_id {
                Some(c) => format!("agt:{id}:{c}"),
                None => format!("agt:{id}:s{idx}"),
            },
            ts_ms,
            tool: Tool::Antigravity,
            session_id: Some(session.clone()),
            project_path: project.clone(),
            agent: agent.clone(),
            name: name.clone(),
            failed,
        });
    }
    out.next_offset = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_become_local_paths() {
        assert_eq!(file_uri_to_path("file:///c%3A/Users/Efe/Desktop/app").as_deref(), Some(r"c:\Users\Efe\Desktop\app"));
        assert_eq!(file_uri_to_path("file:///c:/xampp/htdocs/fuar/").as_deref(), Some(r"c:\xampp\htdocs\fuar"));
        assert_eq!(file_uri_to_path("file:///home/u/%C3%A7al%C4%B1%C5%9Fma").as_deref(), Some("/home/u/çalışma"));
        assert_eq!(file_uri_to_path("file:///Users/u/My%20App").as_deref(), Some("/Users/u/My App"));
        assert_eq!(file_uri_to_path("file://server/share/x").as_deref(), Some("//server/share/x"));
        assert_eq!(file_uri_to_path("file:///c%3A/").as_deref(), Some(r"c:\"));
        assert_eq!(file_uri_to_path("https://x"), None);
        assert_eq!(file_uri_to_path("file:///bad%"), Some("/bad%".into()));
    }

    #[test]
    fn malformed_protobuf_stops_without_panicking() {
        for bad in [&[0xff_u8][..], &[0x0a, 0x05, 1], &[0x08], &[0x0b], &[0x00, 0x01], &[0x80; 12]] {
            assert!(fields(bad).count() <= 1);
        }
        assert_eq!(int(&[0x08, 0x96, 0x01], 1), Some(150));
    }

    #[test]
    fn surface_comes_from_the_data_folder() {
        assert_eq!(client_for(Path::new("/h/.gemini/antigravity")), "antigravity");
        assert_eq!(client_for(Path::new("/h/.gemini/antigravity-ide")), "antigravity_ide");
        assert_eq!(client_for(Path::new("/h/.gemini/antigravity-cli")), "antigravity_cli");
    }
}
