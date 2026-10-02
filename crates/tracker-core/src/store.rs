//! Local SQLite archive (`%LOCALAPPDATA%\AIUsageTracker\tracker.db`). Records stay here even
//! after the source tools delete or rotate their logs. No content columns exist by design.

use crate::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool, UsageEvent};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

const MIGRATIONS: &[&str] = &[
    // v1
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
];

pub struct Store {
    conn: Connection,
    project_cache: HashMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub file_id: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub offset: u64,
    pub state: serde_json::Value,
}

/// A usage row as read back for analysis.
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
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub hidden: bool,
}

/// Default database location for the current user.
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
        Ok(Store { conn, project_cache: HashMap::new() })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn transaction(&mut self) -> Result<StoreTx<'_>> {
        let tx = self.conn.transaction()?;
        Ok(StoreTx { tx, project_cache: &mut self.project_cache })
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
            "SELECT ts_ms, tool, client, model, project_id, session_id, input, cache_read, cache_write,
                    cache_write_1h, output, reasoning, request_input, web_search, speed, inference_geo, accuracy
             FROM usage_event WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms",
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
            })
        })?;
        rows.collect()
    }

    pub fn first_event_ms(&self) -> Result<Option<i64>> {
        self.conn.query_row("SELECT MIN(ts_ms) FROM usage_event", [], |r| r.get(0))
    }

    pub fn projects(&self) -> Result<Vec<ProjectRow>> {
        let mut st = self.conn.prepare("SELECT id, path, name, hidden FROM project ORDER BY name")?;
        let rows = st.query_map([], |r| {
            Ok(ProjectRow { id: r.get(0)?, path: r.get(1)?, name: r.get(2)?, hidden: r.get::<_, i64>(3)? != 0 })
        })?;
        rows.collect()
    }

    pub fn set_project_hidden(&self, id: i64, hidden: bool) -> Result<()> {
        self.conn.execute("UPDATE project SET hidden = ?2 WHERE id = ?1", params![id, hidden as i64])?;
        Ok(())
    }

    /// Latest snapshot per (provider, tool, account, limit_id, window), plus how old it is.
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

    /// Deletes every usage record, snapshot and checkpoint (settings are kept).
    pub fn wipe_data(&self) -> Result<()> {
        self.conn.execute_batch(
            "BEGIN; DELETE FROM usage_event; DELETE FROM limit_snapshot; DELETE FROM file_checkpoint; DELETE FROM project; COMMIT; VACUUM;",
        )
    }

    /// Consistent copy of the database (works while the app is running).
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        let _ = std::fs::remove_file(dest);
        self.conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
        Ok(())
    }
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
}

impl StoreTx<'_> {
    fn project_id(&mut self, path: &str) -> Result<i64> {
        let key = normalize_project_path(path);
        if let Some(id) = self.project_cache.get(&key) {
            return Ok(*id);
        }
        let name = project_name(path);
        self.tx.execute("INSERT INTO project(path, name) VALUES(?1, ?2) ON CONFLICT(path) DO NOTHING", params![key, name])?;
        let id: i64 = self.tx.query_row("SELECT id FROM project WHERE path = ?1", [&key], |r| r.get(0))?;
        self.project_cache.insert(key, id);
        Ok(id)
    }

    /// Inserts events; a repeated key keeps the field-wise maximum (streaming duplicates)
    /// and the earliest timestamp. A log-derived (exact) copy upgrades a captured one.
    pub fn upsert_events(&mut self, events: &[UsageEvent]) -> Result<()> {
        for e in events {
            let pid = match &e.project_path {
                Some(p) if !p.is_empty() => Some(self.project_id(p)?),
                _ => None,
            };
            let mut st = self.tx.prepare_cached(
                "INSERT INTO usage_event(key, ts_ms, tool, client, model, project_id, session_id, input, cache_read,
                     cache_write, cache_write_1h, output, reasoning, request_input, web_search, speed, service_tier,
                     inference_geo, accuracy, source)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
                 ON CONFLICT(key) DO UPDATE SET
                     ts_ms = MIN(ts_ms, excluded.ts_ms),
                     input = MAX(input, excluded.input),
                     cache_read = MAX(cache_read, excluded.cache_read),
                     cache_write = MAX(cache_write, excluded.cache_write),
                     cache_write_1h = MAX(cache_write_1h, excluded.cache_write_1h),
                     output = MAX(output, excluded.output),
                     reasoning = MAX(reasoning, excluded.reasoning),
                     request_input = MAX(request_input, excluded.request_input),
                     web_search = MAX(web_search, excluded.web_search),
                     project_id = COALESCE(project_id, excluded.project_id),
                     accuracy = CASE WHEN excluded.accuracy = 'exact' THEN 'exact' ELSE accuracy END",
            )?;
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
            ])?;
        }
        Ok(())
    }

    pub fn insert_limits(&mut self, limits: &[LimitSnapshot]) -> Result<()> {
        let mut st = self.tx.prepare_cached(
            "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status,
                 plan, source, accuracy)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(source, account, limit_id, window, ts_ms) DO NOTHING",
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
        self.tx.commit()
    }
}

/// Uniqueness key for a project directory. Windows paths are case-insensitive and tools
/// disagree on casing and separators, so they are folded.
pub fn normalize_project_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    let looks_windows = trimmed.as_bytes().get(1) == Some(&b':') || trimmed.starts_with(r"\\");
    if looks_windows { trimmed.replace('/', "\\").to_lowercase() } else { trimmed.to_owned() }
}

/// Display name for a project directory: its last path component.
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
