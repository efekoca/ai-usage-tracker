//! Incremental, idempotent ingestion: each file is read from its last checkpointed byte
//! offset; re-running over unchanged files does nothing, and re-reading a file from scratch
//! (truncation, replacement, move) cannot double count because every record has a global key.

use crate::discovery::{file_key, DiscoveredFile};
use crate::capture::statusline;
use crate::sources::{antigravity, claude_code, claude_plan, claude_sessions, codex, cowork_audit, is_compressed, opencode, ParseOutput, ParserKind, MAX_WARNING_DETAILS};
use crate::store::Store;
use serde::Serialize;
use std::time::UNIX_EPOCH;

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestReport {
    pub files_seen: usize,
    pub files_read: usize,
    /// Usage records, readings and tool calls written; zero when only other lines were appended.
    pub records_written: usize,
    pub events_before: i64,
    pub events_after: i64,
    pub limits_before: i64,
    pub limits_after: i64,
    /// (path, message) — content-free; at most `MAX_WARNING_DETAILS` per file.
    pub warnings: Vec<(String, String)>,
    pub warning_count: usize,
    pub errors: Vec<(String, String)>,
}

impl IngestReport {
    pub fn new_events(&self) -> i64 {
        self.events_after - self.events_before
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

pub fn ingest(store: &mut Store, files: &[DiscoveredFile], mut on_progress: impl FnMut(Progress)) -> IngestReport {
    store.forget_projects();
    let mut rep = IngestReport {
        files_seen: files.len(),
        events_before: store.event_count().unwrap_or(0),
        limits_before: store.limit_count().unwrap_or(0),
        ..Default::default()
    };
    let (mut codex_read, mut claude_read) = (false, false);
    for (i, f) in files.iter().enumerate() {
        // one damaged file must not stop the scan; its open transaction rolls back on unwind
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ingest_file(store, f)))
            .unwrap_or_else(|_| Err("the parser stopped on unexpected data".into()));
        match read {
            Ok(Some(read)) => {
                rep.files_read += 1;
                codex_read |= f.parser == ParserKind::CodexRollout;
                claude_read |= f.parser.is_claude();
                rep.records_written += read.records;
                rep.warning_count += read.warning_count;
                let p = file_key(&f.path);
                rep.warnings.extend(read.warnings.into_iter().map(|w| (p.clone(), w)));
            }
            Ok(None) => {}
            Err(e) => {
                let p = file_key(&f.path);
                log::warn!("ingest failed for {p}: {e}");
                rep.errors.push((p, e));
            }
        }
        on_progress(Progress { done: i + 1, total: files.len() });
    }
    if rep.files_read > 0 {
        let _ = store.prune_projects();
    }
    if codex_read && let Err(e) = crate::accounts::resolve_codex(store) {
        log::warn!("codex accounts not resolved: {e}");
    }
    if claude_read && let Err(e) = crate::accounts::resolve_claude(store) {
        log::warn!("claude accounts not resolved: {e}");
    }
    rep.events_after = store.event_count().unwrap_or(0);
    rep.limits_after = store.limit_count().unwrap_or(0);
    rep
}

/// A record dated further ahead than this is damaged, not from a clock that ran fast.
const HOLD_AHEAD_MS: i64 = 30 * 86_400_000;

fn size_and_mtime(path: &std::path::Path) -> std::io::Result<(u64, i64)> {
    let meta = std::fs::metadata(path)?;
    let mtime_ms = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    Ok((meta.len(), mtime_ms))
}

/// What one changed file added: a sample of its warnings, how many there were, and records.
#[derive(Debug, Default)]
pub struct FileRead {
    pub warnings: Vec<String>,
    pub warning_count: usize,
    pub records: usize,
}

/// Returns `Ok(None)` when the file was unchanged and skipped. A long file is read in rounds of
/// `CHUNK_RECORDS`, each committed with the checkpoint after it, so memory stays bounded and an
/// interrupted scan resumes after the last stored round.
pub fn ingest_file(store: &mut Store, f: &DiscoveredFile) -> Result<Option<FileRead>, String> {
    let (mut size, mut mtime_ms) = size_and_mtime(&f.path).map_err(|e| e.to_string())?;
    if f.parser.has_wal() {
        // new rows land in the WAL until SQLite checkpoints them into the database file
        let mut wal = f.path.clone().into_os_string();
        wal.push("-wal");
        if let Ok((s, m)) = size_and_mtime(std::path::Path::new(&wal)) {
            size += s;
            mtime_ms = mtime_ms.max(m);
        }
    }
    let key = file_key(&f.path);
    let cp = store.checkpoint(&key).map_err(|e| e.to_string())?;
    // offsets count decompressed bytes, which the file size cannot be compared with; a changed
    // compressed file is read again from its start, and its keys merge with what was stored
    let packed = is_compressed(&f.path);

    let (mut offset, mut state) = match &cp {
        Some(c) if c.file_id == f.file_id && c.size == size && c.mtime_ms == mtime_ms => return Ok(None),
        Some(c) if f.parser.is_incremental() && !packed && c.file_id == f.file_id && size >= c.offset && same_tail(&f.path, c) => (c.offset, c.state.clone()),
        Some(c) if f.parser.keeps_state() && c.file_id == f.file_id => (0, c.state.clone()),
        _ => (0, serde_json::Value::Null), // new, replaced, truncated or whole-document file
    };

    let mut read = FileRead::default();
    // where the first record dated too far ahead was read: the file is read from there again
    // until the clock reaches it, instead of skipping it for good
    let mut hold: Option<(u64, serde_json::Value)> = None;
    loop {
        let mut out = parse(f, offset, &state).map_err(|e| e.to_string())?;
        let more = f.parser.is_incremental() && out.round_full() && out.next_offset > offset && (packed || out.next_offset < size);
        let latest = crate::store::latest_accepted_ms();
        let ahead = |ts: i64| ts > latest && ts - latest <= HOLD_AHEAD_MS;
        if hold.is_none() && (out.events.iter().any(|e| ahead(e.ts_ms)) || out.limits.iter().any(|l| ahead(l.ts_ms)) || out.tool_calls.iter().any(|c| ahead(c.ts_ms))) {
            hold = Some((offset, state.clone()));
            out.warn("records dated ahead of the clock are read once it reaches them");
        }

        let mut tx = store.transaction().map_err(|e| e.to_string())?;
        tx.drop_stale(&out.stale_events, &out.stale_tool_calls, &out.stale_limits).map_err(|e| e.to_string())?;
        for (i, thread) in &out.copies {
            if let Some(ts) = tx.absorb_offset_copy(thread, &out.events[*i].tokens).map_err(|e| e.to_string())? {
                out.events[*i].ts_ms = out.events[*i].ts_ms.min(ts);
            }
        }
        tx.upsert_events(&out.events).map_err(|e| e.to_string())?;
        tx.insert_limits(&out.limits).map_err(|e| e.to_string())?;
        tx.insert_session_accounts(&out.sessions).map_err(|e| e.to_string())?;
        if f.parser == ParserKind::ClaudeCodeJsonl
            && let Some((root, session)) = crate::accounts::claude_log_session(&f.path)
        {
            tx.note_session_root(crate::model::Tool::ClaudeCode, &session, &root).map_err(|e| e.to_string())?;
        }
        tx.upsert_tool_calls(&out.tool_calls).map_err(|e| e.to_string())?;
        tx.apply_tool_results(&out.tool_results).map_err(|e| e.to_string())?;
        read.warning_count += out.warning_count;
        for w in out.warnings {
            if read.warnings.len() >= MAX_WARNING_DETAILS {
                read.warnings.pop();
            }
            read.warnings.push(w);
        }
        // store the consumed size so a pending partial line or the next round is read next time
        let stored = match (packed, more) {
            (true, true) => 0,
            (true, false) => size,
            _ if out.next_offset < size && f.parser.is_incremental() => out.next_offset,
            _ => size,
        };
        let (stored, next, kept) = match &hold {
            // size 0 never matches, so the next scan reads from the held point
            Some((at, st)) => (0, *at, st),
            None => (stored, out.next_offset, &out.state),
        };
        let kept = if f.parser.is_incremental() && !packed { with_tail(kept, &f.path, next) } else { kept.clone() };
        tx.set_checkpoint(
            &key,
            &f.file_id,
            f.parser.as_str(),
            stored,
            mtime_ms,
            next,
            &kept,
            read.warning_count,
            read.warnings.last().map(String::as_str),
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        read.records += out.events.len() + out.limits.len() + out.tool_calls.len() + out.tool_results.len() + out.sessions.len();
        if !more {
            return Ok(Some(read));
        }
        offset = out.next_offset;
        state = out.state;
    }
}

/// Where a checkpoint keeps a digest of the bytes before its offset (never the bytes themselves).
const TAIL_KEY: &str = "_tail";

fn tail_digest(path: &std::path::Path, offset: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let from = offset.saturating_sub(64);
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity(64);
    f.take(offset - from).read_to_end(&mut buf).ok()?;
    // FNV-1a: stable across builds, unlike the standard library's hasher
    let h = buf.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    (buf.len() as u64 == offset - from).then(|| format!("{h:016x}"))
}

/// A file rewritten in place keeps its identity and may grow past the old offset; the bytes
/// before the offset tell it apart from one that was only appended to.
fn same_tail(path: &std::path::Path, c: &crate::store::Checkpoint) -> bool {
    match c.state.get(TAIL_KEY).and_then(serde_json::Value::as_str) {
        Some(t) => tail_digest(path, c.offset).as_deref() == Some(t),
        None => true,
    }
}

fn with_tail(state: &serde_json::Value, path: &std::path::Path, offset: u64) -> serde_json::Value {
    let mut state = state.clone();
    if offset > 0
        && let Some(t) = tail_digest(path, offset)
    {
        match &mut state {
            serde_json::Value::Object(m) => {
                m.insert(TAIL_KEY.into(), t.into());
            }
            serde_json::Value::Null => state = serde_json::json!({ TAIL_KEY: t }),
            _ => {}
        }
    }
    state
}

fn parse(f: &DiscoveredFile, offset: u64, state: &serde_json::Value) -> std::io::Result<ParseOutput> {
    match f.parser {
        ParserKind::ClaudeCodeJsonl => claude_code::parse_file(
            &f.path,
            offset,
            state,
            &claude_code::ClaudeCtx { client_override: None, source: ParserKind::ClaudeCodeJsonl.as_str() },
        ),
        ParserKind::CoworkJsonl => claude_code::parse_file(
            &f.path,
            offset,
            state,
            &claude_code::ClaudeCtx { client_override: Some("cowork"), source: ParserKind::CoworkJsonl.as_str() },
        ),
        ParserKind::CoworkAudit => cowork_audit::parse_file(&f.path, offset),
        ParserKind::ClaudePlanHistory => claude_plan::parse_file(&f.path),
        ParserKind::ClaudeDesktopSession => claude_sessions::parse_file(&f.path),
        ParserKind::CodexRollout => codex::parse_file(&f.path, offset, state),
        ParserKind::StatuslineCapture => statusline::parse_file(&f.path, offset),
        ParserKind::AntigravityDb => antigravity::parse_file(&f.path),
        ParserKind::OpenCodeDb => opencode::parse_file(&f.path, state),
    }
}
