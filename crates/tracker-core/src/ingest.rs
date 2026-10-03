//! Incremental, idempotent ingestion: each file is read from its last checkpointed byte
//! offset; re-running over unchanged files does nothing, and re-reading a file from scratch
//! (truncation, replacement, move) cannot double count because every record has a global key.

use crate::discovery::DiscoveredFile;
use crate::capture::statusline;
use crate::sources::{claude_code, claude_plan, codex, cowork_audit, ParseOutput, ParserKind};
use crate::store::Store;
use serde::Serialize;
use std::time::UNIX_EPOCH;

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestReport {
    pub files_seen: usize,
    pub files_read: usize,
    pub events_before: i64,
    pub events_after: i64,
    pub limits_before: i64,
    pub limits_after: i64,
    /// (path, message) — content-free.
    pub warnings: Vec<(String, String)>,
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
    let mut rep = IngestReport {
        files_seen: files.len(),
        events_before: store.event_count().unwrap_or(0),
        limits_before: store.limit_count().unwrap_or(0),
        ..Default::default()
    };
    for (i, f) in files.iter().enumerate() {
        match ingest_file(store, f) {
            Ok(Some(warns)) => {
                rep.files_read += 1;
                let p = f.path.to_string_lossy().into_owned();
                rep.warnings.extend(warns.into_iter().map(|w| (p.clone(), w)));
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("ingest failed for {}: {e}", f.path.display());
                rep.errors.push((f.path.to_string_lossy().into_owned(), e));
            }
        }
        on_progress(Progress { done: i + 1, total: files.len() });
    }
    if rep.files_read > 0 {
        let _ = store.prune_projects();
    }
    rep.events_after = store.event_count().unwrap_or(0);
    rep.limits_after = store.limit_count().unwrap_or(0);
    rep
}

/// Returns `Ok(None)` when the file was unchanged and skipped, otherwise the parse warnings.
pub fn ingest_file(store: &mut Store, f: &DiscoveredFile) -> Result<Option<Vec<String>>, String> {
    let meta = std::fs::metadata(&f.path).map_err(|e| e.to_string())?;
    let size = meta.len();
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let key = f.path.to_string_lossy().into_owned();
    let cp = store.checkpoint(&key).map_err(|e| e.to_string())?;

    let (offset, state) = match &cp {
        Some(c) if c.file_id == f.file_id && c.size == size && c.mtime_ms == mtime_ms => return Ok(None),
        Some(c) if f.parser.is_incremental() && c.file_id == f.file_id && size >= c.offset => (c.offset, c.state.clone()),
        _ => (0, serde_json::Value::Null), // new, replaced, truncated or whole-document file
    };

    let out: ParseOutput = match f.parser {
        ParserKind::ClaudeCodeJsonl => claude_code::parse_file(
            &f.path,
            offset,
            &state,
            &claude_code::ClaudeCtx { client_override: None, source: ParserKind::ClaudeCodeJsonl.as_str() },
        ),
        ParserKind::CoworkJsonl => claude_code::parse_file(
            &f.path,
            offset,
            &state,
            &claude_code::ClaudeCtx { client_override: Some("cowork"), source: ParserKind::CoworkJsonl.as_str() },
        ),
        ParserKind::CoworkAudit => cowork_audit::parse_file(&f.path, offset),
        ParserKind::ClaudePlanHistory => claude_plan::parse_file(&f.path),
        ParserKind::CodexRollout => codex::parse_file(&f.path, offset, &state),
        ParserKind::StatuslineCapture => statusline::parse_file(&f.path, offset),
    }
    .map_err(|e| e.to_string())?;

    let mut tx = store.transaction().map_err(|e| e.to_string())?;
    tx.upsert_events(&out.events).map_err(|e| e.to_string())?;
    tx.insert_limits(&out.limits).map_err(|e| e.to_string())?;
    tx.upsert_tool_calls(&out.tool_calls).map_err(|e| e.to_string())?;
    tx.apply_tool_results(&out.tool_results).map_err(|e| e.to_string())?;
    tx.set_checkpoint(
        &key,
        &f.file_id,
        f.parser.as_str(),
        // store the consumed size so a pending partial line is re-checked next time
        if out.next_offset < size && f.parser.is_incremental() { out.next_offset } else { size },
        mtime_ms,
        out.next_offset,
        &out.state,
        out.warnings.len(),
        out.warnings.last().map(String::as_str),
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(out.warnings))
}
