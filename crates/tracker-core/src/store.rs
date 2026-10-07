//! Local SQLite archive (`%LOCALAPPDATA%\AIUsageTracker\tracker.db`). Records stay here even
//! after the source tools delete or rotate their logs. No content columns exist by design.

use crate::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool, ToolCall, UsageEvent};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE project (
        id      INTEGER PRIMARY KEY,
        path    TEXT NOT NULL UNIQUE,
        name    TEXT NOT NULL,
        hidden  INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE usage_event (
        key            TEXT PRIMARY KEY,
        ts_ms          INTEGER NOT NULL,
        tool           TEXT NOT NULL,
        client         TEXT,
        model          TEXT NOT NULL,
        project_id     INTEGER REFERENCES project(id),
        session_id     TEXT,
        input          INTEGER NOT NULL DEFAULT 0,
        cache_read     INTEGER NOT NULL DEFAULT 0,
        cache_write    INTEGER NOT NULL DEFAULT 0,
        cache_write_1h INTEGER NOT NULL DEFAULT 0,
        output         INTEGER NOT NULL DEFAULT 0,
        reasoning      INTEGER NOT NULL DEFAULT 0,
        request_input  INTEGER NOT NULL DEFAULT 0,
        web_search     INTEGER NOT NULL DEFAULT 0,
        speed          TEXT,
        service_tier   TEXT,
        inference_geo  TEXT,
        accuracy       TEXT NOT NULL,
        source         TEXT NOT NULL
    );
    CREATE INDEX usage_event_ts ON usage_event(ts_ms);
    CREATE TABLE limit_snapshot (
        id        INTEGER PRIMARY KEY,
        ts_ms     INTEGER NOT NULL,
        provider  TEXT NOT NULL,
        tool      TEXT NOT NULL,
        account   TEXT NOT NULL DEFAULT '',
        limit_id  TEXT NOT NULL DEFAULT '',
        window    TEXT NOT NULL,
        used_pct  REAL,
        resets_at INTEGER,
        status    TEXT,
        plan      TEXT,
        source    TEXT NOT NULL,
        accuracy  TEXT NOT NULL,
        UNIQUE(source, account, limit_id, window, ts_ms)
    );
    CREATE INDEX limit_snapshot_ts ON limit_snapshot(ts_ms);
    CREATE TABLE file_checkpoint (
        path         TEXT PRIMARY KEY,
        file_id      TEXT NOT NULL,
        parser       TEXT NOT NULL,
        size         INTEGER NOT NULL,
        mtime_ms     INTEGER NOT NULL,
        offset       INTEGER NOT NULL,
        state        TEXT,
        warnings     INTEGER NOT NULL DEFAULT 0,
        last_warning TEXT,
        updated_ms   INTEGER NOT NULL
    );
    CREATE TABLE setting (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // v2: request ids link captured events to their log copies; re-read Claude transcripts.
    r#"
    ALTER TABLE usage_event ADD COLUMN request_id TEXT;
    CREATE INDEX usage_event_req ON usage_event(request_id);
    CREATE INDEX usage_event_session ON usage_event(session_id);
    DELETE FROM file_checkpoint WHERE parser IN ('claude_code_jsonl', 'cowork_jsonl');
    "#,
    // v3: projects keyed by the session's first directory; re-read Claude transcripts.
    r#"
    ALTER TABLE project ADD COLUMN display_path TEXT;
    DELETE FROM file_checkpoint WHERE parser IN ('claude_code_jsonl', 'cowork_jsonl');
    "#,
    // v4: Cowork grouped into one project; re-read Codex for the original path spelling.
    r#"
    DELETE FROM file_checkpoint WHERE parser IN ('codex_rollout', 'cowork_jsonl');
    "#,
    // v5: re-read Claude logs to keep pricing modifiers from any copy and audit utilization.
    r#"
    DELETE FROM file_checkpoint WHERE parser IN ('claude_code_jsonl', 'cowork_jsonl', 'cowork_audit');
    "#,
    // v6: subagent transcripts belong to their session's launch directory; re-read Claude Code.
    r#"
    DELETE FROM file_checkpoint WHERE parser = 'claude_code_jsonl';
    "#,
    // v7: branch, subagent and tool calls (names only, never input or output); re-read logs.
    r#"
    ALTER TABLE usage_event ADD COLUMN branch TEXT;
    ALTER TABLE usage_event ADD COLUMN agent TEXT;
    ALTER TABLE usage_event ADD COLUMN thread_id TEXT;
    CREATE TABLE tool_call (
        key        TEXT PRIMARY KEY,
        ts_ms      INTEGER NOT NULL,
        tool       TEXT NOT NULL,
        session_id TEXT,
        project_id INTEGER REFERENCES project(id),
        agent      TEXT,
        name       TEXT NOT NULL,
        failed     INTEGER
    );
    CREATE INDEX tool_call_ts ON tool_call(ts_ms);
    DELETE FROM file_checkpoint WHERE parser IN ('claude_code_jsonl', 'cowork_jsonl', 'codex_rollout');
    "#,
    // v8: an older build that opens a v7 database re-reads the logs without filling the v7
    // columns and saves its checkpoints again; read once more so they are filled.
    r#"
    DELETE FROM file_checkpoint WHERE parser IN ('claude_code_jsonl', 'cowork_jsonl', 'codex_rollout');
    "#,
    // v9: Codex keys come from content, so copied fork history merges with its source.
    r#"
    INSERT INTO setting(key, value) VALUES('repair.codex_keys', '1') ON CONFLICT(key) DO UPDATE SET value = '1';
    DELETE FROM file_checkpoint WHERE parser = 'codex_rollout';
    "#,
    // v10: 0.2.4's Codex keys are replaced by fork-root keys; offset keys are replaced per file.
    r#"
    DELETE FROM usage_event WHERE tool = 'codex' AND source = 'codex_rollout' AND key GLOB 'cx:*:t*';
    DELETE FROM tool_call WHERE tool = 'codex' AND key GLOB 'cxt:*:i*';
    DELETE FROM setting WHERE key = 'repair.codex_keys';
    DELETE FROM file_checkpoint WHERE parser = 'codex_rollout';
    "#,
];

const MAX_IMPORTED_COUNT: i64 = 1_000_000_000_000;

/// A captured row is hidden when the same request also exists as an exact (log) row.
const NOT_SUPERSEDED: &str = "NOT (u.accuracy = 'captured' AND u.request_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM usage_event x WHERE x.request_id = u.request_id AND x.accuracy = 'exact'))";

const EVENT_COLUMNS: &str = "key, ts_ms, tool, client, model, project_id, session_id, input, cache_read, cache_write,
    cache_write_1h, output, reasoning, request_input, web_search, speed, service_tier, inference_geo, accuracy, source,
    request_id, branch, agent, thread_id";

/// Merge rule for a repeated event key: field-wise maximum (streaming duplicates), earliest
/// timestamp, pricing modifiers from whichever copy carries them (the first streaming copy of a
/// Claude message has no `speed`), and a log-derived (exact) copy upgrades a captured one.
/// Branch, subagent and thread are kept once known; a row that learns it belongs to a subagent
/// thread also takes that copy's session (a Codex subagent counts toward its parent session).
/// SQLite evaluates every right-hand side against the old row, so the order does not matter.
const UPSERT_EVENT_TAIL: &str = "ON CONFLICT(key) DO UPDATE SET
    ts_ms = MIN(ts_ms, excluded.ts_ms),
    model = CASE WHEN model = 'unknown' THEN excluded.model ELSE model END,
    input = MAX(input, excluded.input),
    cache_read = MAX(cache_read, excluded.cache_read),
    cache_write = MAX(cache_write, excluded.cache_write),
    cache_write_1h = MAX(cache_write_1h, excluded.cache_write_1h),
    output = MAX(output, excluded.output),
    reasoning = MAX(reasoning, excluded.reasoning),
    request_input = MAX(request_input, excluded.request_input),
    web_search = MAX(web_search, excluded.web_search),
    speed = COALESCE(excluded.speed, speed),
    service_tier = COALESCE(excluded.service_tier, service_tier),
    inference_geo = COALESCE(excluded.inference_geo, inference_geo),
    project_id = COALESCE(excluded.project_id, project_id),
    request_id = COALESCE(request_id, excluded.request_id),
    accuracy = CASE WHEN excluded.accuracy = 'exact' THEN 'exact' ELSE accuracy END,
    branch = COALESCE(branch, excluded.branch),
    agent = COALESCE(agent, excluded.agent),
    session_id = CASE WHEN thread_id IS NULL AND excluded.thread_id IS NOT NULL THEN excluded.session_id ELSE session_id END,
    thread_id = COALESCE(thread_id, excluded.thread_id)";

pub struct Store {
    conn: Connection,
    project_cache: RefCell<HashMap<String, i64>>,
}

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub file_id: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub offset: u64,
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub ts_ms: i64,
    pub tool: Tool,
    pub client: Option<String>,
    pub model: String,
    pub project_id: Option<i64>,
    pub session_id: Option<String>,
    pub tokens: Tokens,
    pub request_input: u64,
    pub web_search: u32,
    pub speed: Option<String>,
    pub inference_geo: Option<String>,
    pub accuracy: Accuracy,
    pub branch: Option<String>,
    pub agent: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRow {
    pub ts_ms: i64,
    pub tool: Tool,
    pub session_id: Option<String>,
    pub project_id: Option<i64>,
    pub agent: Option<String>,
    pub name: String,
    pub failed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub hidden: bool,
}

pub fn default_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("AIUsageTracker"))
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {}; COMMIT;", i + 1))?;
        }
        Ok(Store { conn, project_cache: RefCell::default() })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn transaction(&mut self) -> Result<StoreTx<'_>> {
        let tx = self.conn.transaction()?;
        Ok(StoreTx { tx, project_cache: self.project_cache.get_mut(), new_projects: HashMap::new() })
    }

    pub fn checkpoint(&self, path: &str) -> Result<Option<Checkpoint>> {
        self.conn
            .query_row(
                "SELECT file_id, size, mtime_ms, offset, state FROM file_checkpoint WHERE path = ?1",
                [path],
                |r| {
                    let state: Option<String> = r.get(4)?;
                    Ok(Checkpoint {
                        file_id: r.get(0)?,
                        size: r.get::<_, i64>(1)? as u64,
                        mtime_ms: r.get(2)?,
                        offset: r.get::<_, i64>(3)? as u64,
                        state: state.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null),
                    })
                },
            )
            .optional()
    }

    pub fn event_count(&self) -> Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0))
    }

    pub fn limit_count(&self) -> Result<i64> {
        self.conn.query_row("SELECT COUNT(*) FROM limit_snapshot", [], |r| r.get(0))
    }

    /// Events with `from_ms <= ts < to_ms`.
    pub fn events_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<EventRow>> {
        let mut st = self.conn.prepare_cached(
            &format!(
                "SELECT ts_ms, tool, client, model, project_id, session_id, input, cache_read, cache_write,
                        cache_write_1h, output, reasoning, request_input, web_search, speed, inference_geo, accuracy,
                        branch, agent, thread_id
                 FROM usage_event u WHERE ts_ms >= ?1 AND ts_ms < ?2 AND {NOT_SUPERSEDED} ORDER BY ts_ms"
            ),
        )?;
        let rows = st.query_map(params![from_ms, to_ms], |r| {
            let tool: String = r.get(1)?;
            let acc: String = r.get(16)?;
            Ok(EventRow {
                ts_ms: r.get(0)?,
                tool: Tool::parse(&tool).unwrap_or(Tool::ClaudeCode),
                client: r.get(2)?,
                model: r.get(3)?,
                project_id: r.get(4)?,
                session_id: r.get(5)?,
                tokens: Tokens {
                    input: r.get::<_, i64>(6)? as u64,
                    cache_read: r.get::<_, i64>(7)? as u64,
                    cache_write: r.get::<_, i64>(8)? as u64,
                    cache_write_1h: r.get::<_, i64>(9)? as u64,
                    output: r.get::<_, i64>(10)? as u64,
                    reasoning: r.get::<_, i64>(11)? as u64,
                },
                request_input: r.get::<_, i64>(12)? as u64,
                web_search: r.get::<_, i64>(13)? as u32,
                speed: r.get(14)?,
                inference_geo: r.get(15)?,
                accuracy: Accuracy::parse(&acc),
                branch: r.get(17)?,
                agent: r.get(18)?,
                thread_id: r.get(19)?,
            })
        })?;
        rows.collect()
    }

    /// Tool calls with `from_ms <= ts < to_ms`.
    pub fn tool_calls_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<ToolCallRow>> {
        let mut st = self.conn.prepare_cached(
            "SELECT ts_ms, tool, session_id, project_id, agent, name, failed FROM tool_call
             WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms",
        )?;
        let rows = st.query_map(params![from_ms, to_ms], |r| {
            let tool: String = r.get(1)?;
            Ok(ToolCallRow {
                ts_ms: r.get(0)?,
                tool: Tool::parse(&tool).unwrap_or(Tool::ClaudeCode),
                session_id: r.get(2)?,
                project_id: r.get(3)?,
                agent: r.get(4)?,
                name: r.get(5)?,
                failed: r.get::<_, Option<i64>>(6)?.map(|f| f != 0),
            })
        })?;
        rows.collect()
    }

    pub fn first_event_ms(&self) -> Result<Option<i64>> {
        self.conn.query_row("SELECT MIN(ts_ms) FROM usage_event", [], |r| r.get(0))
    }

    pub fn projects(&self) -> Result<Vec<ProjectRow>> {
        let mut st = self.conn.prepare("SELECT id, COALESCE(display_path, path), name, hidden FROM project ORDER BY name")?;
        let rows = st.query_map([], |r| {
            Ok(ProjectRow { id: r.get(0)?, path: r.get(1)?, name: r.get(2)?, hidden: r.get::<_, i64>(3)? != 0 })
        })?;
        rows.collect()
    }

    pub fn set_project_hidden(&self, id: i64, hidden: bool) -> Result<()> {
        self.conn.execute("UPDATE project SET hidden = ?2 WHERE id = ?1", params![id, hidden as i64])?;
        Ok(())
    }

    /// Latest snapshot per (provider, tool, account, limit_id, window).
    pub fn latest_limits(&self) -> Result<Vec<LimitSnapshot>> {
        let mut st = self.conn.prepare(
            "SELECT s.ts_ms, s.provider, s.tool, s.account, s.limit_id, s.window, s.used_pct, s.resets_at,
                    s.status, s.plan, s.source, s.accuracy
             FROM limit_snapshot s
             JOIN (SELECT provider, tool, account, limit_id, window, MAX(ts_ms) AS m
                   FROM limit_snapshot GROUP BY provider, tool, account, limit_id, window) g
               ON s.provider = g.provider AND s.tool = g.tool AND s.account = g.account
              AND s.limit_id = g.limit_id AND s.window = g.window AND s.ts_ms = g.m
             ORDER BY s.provider, s.window",
        )?;
        let rows = st.query_map([], row_to_limit)?;
        rows.collect()
    }

    pub fn limits_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<LimitSnapshot>> {
        let mut st = self.conn.prepare(
            "SELECT ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status, plan, source, accuracy
             FROM limit_snapshot WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms",
        )?;
        let rows = st.query_map(params![from_ms, to_ms], row_to_limit)?;
        rows.collect()
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO setting(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Files whose last parse produced warnings (for the "unsupported format" notice).
    pub fn files_with_warnings(&self) -> Result<Vec<(String, String, i64, String)>> {
        let mut st = self.conn.prepare(
            "SELECT path, parser, warnings, COALESCE(last_warning, '') FROM file_checkpoint WHERE warnings > 0 ORDER BY updated_ms DESC",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    }

    /// Removes projects that no longer have any usage (e.g. after re-assignment).
    pub fn prune_projects(&self) -> Result<usize> {
        self.project_cache.borrow_mut().clear();
        self.conn.execute(
            "DELETE FROM project WHERE id NOT IN (SELECT DISTINCT project_id FROM usage_event WHERE project_id IS NOT NULL)
               AND id NOT IN (SELECT DISTINCT project_id FROM tool_call WHERE project_id IS NOT NULL)",
            [],
        )
    }

    /// Another connection may have deleted projects, and their ids are reused.
    pub fn forget_projects(&mut self) {
        self.project_cache.get_mut().clear();
    }

    /// Deletes every usage record, snapshot and checkpoint (settings are kept).
    pub fn wipe_data(&self) -> Result<()> {
        self.project_cache.borrow_mut().clear();
        self.conn.execute_batch(
            "BEGIN; DELETE FROM usage_event; DELETE FROM tool_call; DELETE FROM limit_snapshot; DELETE FROM file_checkpoint;
             DELETE FROM project; COMMIT; VACUUM;",
        )
    }

    /// Merges another tracker database (e.g. a backup) into this one using the same de-dup
    /// rules as ingestion. Nothing is deleted. Returns (events touched, snapshots added).
    pub fn merge_from(&mut self, other: &Path) -> Result<(usize, usize)> {
        // by canonical path: another spelling of the live file must not be attached to itself
        if self.is_live_file(other) {
            return Err(rusqlite::Error::InvalidPath(other.to_owned()));
        }
        self.conn.execute("ATTACH DATABASE ?1 AS other", [other.to_string_lossy()])?;
        let result = (|| {
            let theirs: i64 = self.conn.query_row("PRAGMA other.user_version", [], |r| r.get(0))?;
            if theirs == 0 || theirs as usize > MIGRATIONS.len() {
                return Err(rusqlite::Error::InvalidQuery);
            }
            // backups made before v2 have no request_id column, before v7 no branch/agent/thread
            let their_req = if theirs >= 2 { "e.request_id" } else { "NULL" };
            let their_v7 = if theirs >= 7 { "e.branch, e.agent, e.thread_id" } else { "NULL, NULL, NULL" };
            // Codex rows from before v10 use other keys: take them only for sessions this database lacks
            self.conn.execute_batch(
                "DROP TABLE IF EXISTS temp.codex_sessions;
                 CREATE TEMP TABLE codex_sessions AS SELECT DISTINCT session_id FROM main.usage_event WHERE tool = 'codex' AND session_id IS NOT NULL;",
            )?;
            let old_codex = |t: &str| {
                if theirs >= 10 {
                    String::new()
                } else {
                    format!("AND NOT ({t}.tool = 'codex' AND {t}.session_id IN (SELECT session_id FROM temp.codex_sessions))")
                }
            };
            let (old_events, old_calls) = (old_codex("e"), old_codex("c"));
            // a damaged or hand-made file must not bring negative or absurd counts
            let valid_counts: String = ["input", "cache_read", "cache_write", "cache_write_1h", "output", "reasoning", "request_input", "web_search"]
                .iter()
                .map(|c| format!(" AND typeof(e.{c}) = 'integer' AND e.{c} BETWEEN 0 AND {MAX_IMPORTED_COUNT}"))
                .collect();
            let tx = self.conn.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO project(path, name, hidden) SELECT path, name, hidden FROM other.project WHERE true
                 ON CONFLICT(path) DO NOTHING",
                [],
            )?;
            let events = tx.execute(
                &format!(
                    "INSERT INTO usage_event({EVENT_COLUMNS})
                     SELECT e.key, e.ts_ms, e.tool, e.client, e.model, p.id, e.session_id, e.input, e.cache_read,
                            e.cache_write, e.cache_write_1h, e.output, e.reasoning, e.request_input, e.web_search,
                            e.speed, e.service_tier, e.inference_geo, e.accuracy, e.source, {their_req}, {their_v7}
                     FROM other.usage_event e
                     LEFT JOIN other.project op ON op.id = e.project_id
                     LEFT JOIN main.project p ON p.path = op.path
                     WHERE typeof(e.ts_ms) = 'integer' {valid_counts} {old_events}
                     {UPSERT_EVENT_TAIL}"
                ),
                [],
            )?;
            let limits = tx.execute(
                "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status,
                     plan, source, accuracy)
                 SELECT ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status, plan, source, accuracy
                 FROM other.limit_snapshot
                 WHERE typeof(ts_ms) = 'integer' AND typeof(resets_at) IN ('integer', 'null')
                   AND (used_pct IS NULL OR used_pct BETWEEN 0 AND 1000)
                 ON CONFLICT(source, account, limit_id, window, ts_ms) DO NOTHING",
                [],
            )?;
            if theirs >= 7 {
                tx.execute(
                    &format!(
                        "INSERT INTO tool_call(key, ts_ms, tool, session_id, project_id, agent, name, failed)
                         SELECT c.key, c.ts_ms, c.tool, c.session_id, p.id, c.agent, c.name, c.failed
                         FROM other.tool_call c
                         LEFT JOIN other.project op ON op.id = c.project_id
                         LEFT JOIN main.project p ON p.path = op.path
                         WHERE typeof(c.ts_ms) = 'integer' AND typeof(c.failed) IN ('integer', 'null') {old_calls}
                         ON CONFLICT(key) DO UPDATE SET failed = COALESCE(failed, excluded.failed)"
                    ),
                    [],
                )?;
            }
            tx.commit()?;
            Ok((events, limits))
        })();
        self.conn.execute("DETACH DATABASE other", [])?;
        let _ = self.conn.execute("DROP TABLE IF EXISTS temp.codex_sessions", []);
        self.project_cache.get_mut().clear();
        result
    }

    /// Works while the app is running; an existing `dest` is replaced only once the copy is complete.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if self.is_live_file(dest) {
            return Err(rusqlite::Error::InvalidPath(dest.to_owned()));
        }
        let name = dest.file_name().ok_or_else(|| rusqlite::Error::InvalidPath(dest.to_owned()))?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let tmp = dest.with_file_name(format!(".{}.{}-{nanos}.tmp", name.to_string_lossy(), std::process::id()));
        let written = self
            .conn
            .execute("VACUUM INTO ?1", [tmp.to_string_lossy()])
            .and_then(|_| std::fs::rename(&tmp, dest).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e))));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    fn is_live_file(&self, path: &Path) -> bool {
        let Some(live) = self.conn.path().filter(|p| !p.is_empty()).and_then(|p| canonical(Path::new(p))) else {
            return false;
        };
        let Some(path) = canonical(path) else { return false };
        ["", "-wal", "-shm", "-journal"].iter().any(|s| path == format!("{live}{s}"))
    }
}

/// Lower-cased, since Windows paths are case-insensitive.
fn canonical(path: &Path) -> Option<String> {
    let full = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => std::fs::canonicalize(path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))
            .ok()?
            .join(path.file_name()?),
    };
    Some(full.to_string_lossy().to_lowercase())
}

fn row_to_limit(r: &rusqlite::Row<'_>) -> rusqlite::Result<LimitSnapshot> {
    let provider: String = r.get(1)?;
    let tool: String = r.get(2)?;
    let account: String = r.get(3)?;
    let acc: String = r.get(11)?;
    Ok(LimitSnapshot {
        ts_ms: r.get(0)?,
        provider: Provider::parse(&provider).unwrap_or(Provider::Anthropic),
        tool: Tool::parse(&tool).unwrap_or(Tool::ClaudeCode),
        account: (!account.is_empty()).then_some(account),
        limit_id: r.get(4)?,
        window: r.get(5)?,
        used_pct: r.get(6)?,
        resets_at: r.get(7)?,
        status: r.get(8)?,
        plan: r.get(9)?,
        source: r.get(10)?,
        accuracy: Accuracy::parse(&acc),
    })
}

pub struct StoreTx<'a> {
    tx: Transaction<'a>,
    project_cache: &'a mut HashMap<String, i64>,
    /// Cached only once the transaction commits.
    new_projects: HashMap<String, i64>,
}

impl StoreTx<'_> {
    fn project_id(&mut self, path: &str) -> Result<i64> {
        let key = normalize_project_path(path);
        if let Some(id) = self.project_cache.get(&key).or_else(|| self.new_projects.get(&key)) {
            return Ok(*id);
        }
        let name = project_name(path);
        self.tx.execute(
            "INSERT INTO project(path, name, display_path) VALUES(?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET display_path = COALESCE(display_path, excluded.display_path)",
            params![key, name, path],
        )?;
        let id: i64 = self.tx.query_row("SELECT id FROM project WHERE path = ?1", [&key], |r| r.get(0))?;
        self.new_projects.insert(key, id);
        Ok(id)
    }

    /// A repeated key merges with the stored row per `UPSERT_EVENT_TAIL`.
    pub fn upsert_events(&mut self, events: &[UsageEvent]) -> Result<()> {
        for e in events {
            let pid = match &e.project_path {
                Some(p) if !p.is_empty() => Some(self.project_id(p)?),
                // captured events carry no working directory: borrow the project of a logged
                // event from the same session, if any
                _ => match &e.session_id {
                    Some(s) => self
                        .tx
                        .query_row(
                            "SELECT project_id FROM usage_event WHERE session_id = ?1 AND project_id IS NOT NULL LIMIT 1",
                            [s],
                            |r| r.get(0),
                        )
                        .optional()?,
                    None => None,
                },
            };
            let mut st = self.tx.prepare_cached(&format!(
                "INSERT INTO usage_event({EVENT_COLUMNS})
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)
                 {UPSERT_EVENT_TAIL}"
            ))?;
            st.execute(params![
                e.key,
                e.ts_ms,
                e.tool.as_str(),
                e.client,
                e.model,
                pid,
                e.session_id,
                e.tokens.input as i64,
                e.tokens.cache_read as i64,
                e.tokens.cache_write as i64,
                e.tokens.cache_write_1h as i64,
                e.tokens.output as i64,
                e.tokens.reasoning as i64,
                e.request_input as i64,
                e.web_search_requests as i64,
                e.speed,
                e.service_tier,
                e.inference_geo,
                e.accuracy.as_str(),
                e.source,
                e.request_id,
                e.branch,
                e.agent,
                e.thread_id,
            ])?;
        }
        Ok(())
    }

    /// Inserts tool calls; a repeated key (copies of a message) keeps the known outcome.
    pub fn upsert_tool_calls(&mut self, calls: &[ToolCall]) -> Result<()> {
        for c in calls {
            let pid = match &c.project_path {
                Some(p) if !p.is_empty() => Some(self.project_id(p)?),
                _ => None,
            };
            let mut st = self.tx.prepare_cached(
                "INSERT INTO tool_call(key, ts_ms, tool, session_id, project_id, agent, name, failed)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(key) DO UPDATE SET
                     ts_ms = MIN(ts_ms, excluded.ts_ms),
                     project_id = COALESCE(excluded.project_id, project_id),
                     agent = COALESCE(agent, excluded.agent),
                     failed = COALESCE(excluded.failed, failed)",
            )?;
            st.execute(params![c.key, c.ts_ms, c.tool.as_str(), c.session_id, pid, c.agent, c.name, c.failed.map(i64::from)])?;
        }
        Ok(())
    }

    /// Removes what an older version stored for a file now read again under new keys.
    pub fn drop_stale(&mut self, events: &[String], calls: &[String], limits: &[(String, i64)]) -> Result<()> {
        let mut e = self.tx.prepare_cached("DELETE FROM usage_event WHERE key = ?1")?;
        for k in events {
            e.execute([k])?;
        }
        let mut c = self.tx.prepare_cached("DELETE FROM tool_call WHERE key = ?1")?;
        for k in calls {
            c.execute([k])?;
        }
        let mut l = self.tx.prepare_cached("DELETE FROM limit_snapshot WHERE source = ?1 AND ts_ms = ?2")?;
        for (source, ts) in limits {
            l.execute(params![source, ts])?;
        }
        Ok(())
    }

    /// For a request copied from `thread`'s log: removes every row an older version stored for it
    /// under an offset key (`cx:<thread>:<digits>`, the original and earlier copies) and returns
    /// the earliest time.
    pub fn absorb_offset_copy(&mut self, thread: &str, t: &Tokens) -> Result<Option<i64>> {
        let lo = format!("cx:{thread}:");
        let hi = format!("cx:{thread};");
        let found: Vec<(String, i64)> = {
            let mut st = self.tx.prepare_cached(
                "SELECT key, ts_ms FROM usage_event WHERE key >= ?1 AND key < ?2 AND input = ?3 AND cache_read = ?4
                   AND cache_write = ?5 AND output = ?6 AND reasoning = ?7",
            )?;
            let rows = st.query_map(
                params![lo, hi, t.input as i64, t.cache_read as i64, t.cache_write as i64, t.output as i64, t.reasoning as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            rows.collect::<Result<_>>()?
        };
        let mut earliest = None;
        for (key, ts) in found.into_iter().filter(|(k, _)| k[lo.len()..].bytes().all(|b| b.is_ascii_digit())) {
            self.tx.execute("DELETE FROM usage_event WHERE key = ?1", [key])?;
            earliest = Some(earliest.map_or(ts, |e: i64| e.min(ts)));
        }
        Ok(earliest)
    }

    /// Records outcomes that arrived after their calls (Claude's `tool_result` lines).
    pub fn apply_tool_results(&mut self, results: &[(String, bool)]) -> Result<()> {
        let mut st = self.tx.prepare_cached("UPDATE tool_call SET failed = ?2 WHERE key = ?1")?;
        for (key, failed) in results {
            st.execute(params![key, i64::from(*failed)])?;
        }
        Ok(())
    }

    pub fn insert_limits(&mut self, limits: &[LimitSnapshot]) -> Result<()> {
        let mut st = self.tx.prepare_cached(
            "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status,
                 plan, source, accuracy)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(source, account, limit_id, window, ts_ms) DO UPDATE SET
                 used_pct = COALESCE(used_pct, excluded.used_pct),
                 resets_at = COALESCE(resets_at, excluded.resets_at)",
        )?;
        for l in limits {
            st.execute(params![
                l.ts_ms,
                l.provider.as_str(),
                l.tool.as_str(),
                l.account.clone().unwrap_or_default(),
                l.limit_id,
                l.window,
                l.used_pct,
                l.resets_at,
                l.status,
                l.plan,
                l.source,
                l.accuracy.as_str(),
            ])?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_checkpoint(
        &mut self,
        path: &str,
        file_id: &str,
        parser: &str,
        size: u64,
        mtime_ms: i64,
        offset: u64,
        state: &serde_json::Value,
        warnings: usize,
        last_warning: Option<&str>,
    ) -> Result<()> {
        self.tx.execute(
            "INSERT INTO file_checkpoint(path, file_id, parser, size, mtime_ms, offset, state, warnings, last_warning, updated_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(path) DO UPDATE SET file_id = excluded.file_id, parser = excluded.parser, size = excluded.size,
                 mtime_ms = excluded.mtime_ms, offset = excluded.offset, state = excluded.state,
                 warnings = excluded.warnings, last_warning = excluded.last_warning, updated_ms = excluded.updated_ms",
            params![
                path,
                file_id,
                parser,
                size as i64,
                mtime_ms,
                offset as i64,
                state.to_string(),
                warnings as i64,
                last_warning,
                chrono::Utc::now().timestamp_millis()
            ],
        )?;
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        self.tx.commit()?;
        self.project_cache.extend(self.new_projects);
        Ok(())
    }
}

/// Uniqueness key for a project directory. Windows paths are case-insensitive and tools
/// disagree on casing and separators, so they are folded.
pub fn normalize_project_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    let looks_windows = trimmed.as_bytes().get(1) == Some(&b':') || trimmed.starts_with(r"\\");
    if looks_windows { trimmed.replace('/', "\\").to_lowercase() } else { trimmed.to_owned() }
}

pub fn project_name(path: &str) -> String {
    path.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_uses_last_component() {
        assert_eq!(project_name(r"C:\work\my-app"), "my-app");
        assert_eq!(project_name(r"C:\work\my-app\"), "my-app");
        assert_eq!(project_name("/home/u/proj"), "proj");
        assert_eq!(project_name("C:"), "C:");
    }

    #[test]
    fn windows_project_paths_fold_case_and_separators() {
        assert_eq!(normalize_project_path(r"C:\Work\App\"), normalize_project_path("c:/work/app"));
        assert_eq!(normalize_project_path("/home/U/x/"), "/home/U/x");
    }
}
