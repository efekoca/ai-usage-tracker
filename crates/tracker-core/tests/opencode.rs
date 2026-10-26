//! OpenCode: a database with OpenCode 1.19.4's own schema and rows shaped like the ones it wrote
//! against a local mock model (see `sources/opencode.rs`).

mod common;

use common::use_test_round;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracker_core::discovery::{detect, enumerate_files, Env, ExtraPaths, SourceId};
use tracker_core::model::Tool;
use tracker_core::pricing::PriceBook;
use tracker_core::store::{EventRow, Store};

const SCHEMA: &str = "
CREATE TABLE `session` (`id` text PRIMARY KEY, `project_id` text NOT NULL, `workspace_id` text, `parent_id` text,
  `slug` text NOT NULL, `directory` text NOT NULL, `path` text, `title` text NOT NULL, `version` text NOT NULL,
  `share_url` text, `summary_additions` integer, `summary_deletions` integer, `summary_files` integer,
  `summary_diffs` text, `metadata` text, `cost` real DEFAULT 0 NOT NULL, `tokens_input` integer DEFAULT 0 NOT NULL,
  `tokens_output` integer DEFAULT 0 NOT NULL, `tokens_reasoning` integer DEFAULT 0 NOT NULL,
  `tokens_cache_read` integer DEFAULT 0 NOT NULL, `tokens_cache_write` integer DEFAULT 0 NOT NULL, `revert` text,
  `permission` text, `agent` text, `model` text, `time_created` integer NOT NULL, `time_updated` integer NOT NULL,
  `time_compacting` integer, `time_archived` integer);
CREATE INDEX `session_parent_idx` ON `session` (`parent_id`);
CREATE TABLE `message` (`id` text PRIMARY KEY, `session_id` text NOT NULL, `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL, `data` text NOT NULL);
CREATE INDEX `message_session_time_created_id_idx` ON `message` (`session_id`,`time_created`,`id`);
CREATE TABLE `part` (`id` text PRIMARY KEY, `message_id` text NOT NULL, `session_id` text NOT NULL,
  `time_created` integer NOT NULL, `time_updated` integer NOT NULL, `data` text NOT NULL);
CREATE INDEX `part_message_id_id_idx` ON `part` (`message_id`,`id`);
CREATE INDEX `part_session_idx` ON `part` (`session_id`);
";

const T0: i64 = 1_793_000_000_000;
const MAIN: &str = "ses_main";
const CHILD: &str = "ses_child";
const OLD: &str = "ses_old";

struct Db {
    _dir: tempfile::TempDir,
    env: Env,
    path: PathBuf,
    conn: Connection,
}

fn session(c: &Connection, id: &str, parent: Option<&str>, version: &str, dir: &str) {
    c.execute(
        "INSERT INTO session(id, project_id, parent_id, slug, directory, title, version, time_created, time_updated)
         VALUES(?1, 'p1', ?2, 's', ?3, 't', ?4, ?5, ?5)",
        params![id, parent, dir, version, T0],
    )
    .unwrap();
}

/// An assistant message with one finished step, like OpenCode writes after each model request.
#[allow(clippy::too_many_arguments)]
fn step(c: &Connection, n: i64, ses: &str, provider: &str, model: &str, agent: &str, tokens: [u64; 5], extra: &str) {
    let [input, output, reasoning, read, write] = tokens;
    let at = T0 + n * 1000;
    let msg = format!("msg_{n:03}");
    c.execute(
        "INSERT INTO message VALUES(?1, ?2, ?3, ?3, ?4)",
        params![
            msg,
            ses,
            at,
            format!(
                r#"{{"role":"assistant","time":{{"created":{at},"completed":{}}},"parentID":"msg_u","modelID":"{model}","providerID":"{provider}","mode":"{agent}","agent":"{agent}","path":{{"cwd":"/w","root":"/w"}},"cost":0.5,"tokens":{{"total":0,"input":{input},"output":{output},"reasoning":{reasoning},"cache":{{"write":{write},"read":{read}}}}}{extra}}}"#,
                at + 400
            )
        ],
    )
    .unwrap();
    c.execute(
        "INSERT INTO part VALUES(?1, ?2, ?3, ?4, ?4, ?5)",
        params![
            format!("prt_{n:03}_step"),
            msg,
            ses,
            at + 500,
            format!(r#"{{"type":"step-finish","reason":"stop","cost":0.5,"tokens":{{"total":0,"input":{input},"output":{output},"reasoning":{reasoning},"cache":{{"write":{write},"read":{read}}}}}}}"#)
        ],
    )
    .unwrap();
    c.execute(
        "INSERT INTO part VALUES(?1, ?2, ?3, ?4, ?4, ?5)",
        params![format!("prt_{n:03}_text"), msg, ses, at + 100, r#"{"type":"text","text":"never read"}"#],
    )
    .unwrap();
}

fn tool(c: &Connection, n: i64, ses: &str, name: &str, status: &str) {
    let msg = format!("msg_{n:03}");
    c.execute(
        "INSERT INTO part VALUES(?1, ?2, ?3, ?4, ?4, ?5)",
        params![
            format!("prt_{n:03}_tool"),
            msg,
            ses,
            T0 + n * 1000 + 200,
            format!(r#"{{"type":"tool","tool":"{name}","callID":"c{n}","state":{{"status":"{status}","input":{{}},"time":{{"start":{}}}}}}}"#, T0 + n * 1000 + 200)
        ],
    )
    .unwrap();
}

fn opencode() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let data = home.join(".local").join("share").join("opencode");
    std::fs::create_dir_all(&data).unwrap();
    let path = data.join("opencode.db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
    c.execute_batch(SCHEMA).unwrap();
    session(&c, MAIN, None, "1.19.4", "/work/app");
    session(&c, CHILD, Some(MAIN), "1.19.4", "/work/app");
    session(&c, OLD, None, "1.3.15", "/work/old");
    c.execute("INSERT INTO message VALUES('msg_u', ?1, ?2, ?2, '{\"role\":\"user\",\"time\":{\"created\":1}}')", params![MAIN, T0]).unwrap();
    // OpenAI-compatible: 1200 prompt (800 cached), 300 completion (100 reasoning)
    step(&c, 1, MAIN, "openai", "gpt-5", "build", [400, 200, 100, 800, 0], "");
    tool(&c, 1, MAIN, "task", "completed");
    // the subagent's request, made while the parent waited on its task call
    step(&c, 2, CHILD, "openai", "gpt-5", "general", [2000, 70, 0, 1000, 0], "");
    tool(&c, 2, CHILD, "read", "error");
    step(&c, 3, MAIN, "openrouter", "anthropic/claude-sonnet-4.5", "build", [50, 120, 0, 700, 400], "");
    step(&c, 4, MAIN, "anthropic", "claude-sonnet-4-5", "compaction", [900, 300, 0, 0, 0], r#","summary":true"#);
    // an aborted request: OpenCode keeps zeros
    step(&c, 5, MAIN, "anthropic", "claude-sonnet-4-5", "build", [0, 0, 0, 0, 0], "");
    step(&c, 6, OLD, "anthropic", "claude-sonnet-4-5", "build", [5000, 10, 0, 0, 0], "");
    let env = Env { home: Some(home), ..Default::default() };
    Db { _dir: dir, env, path, conn: c }
}

fn only_opencode() -> HashSet<SourceId> {
    [SourceId::OpenCode].into_iter().collect()
}

fn run(store: &mut Store, env: &Env) -> tracker_core::ingest::IngestReport {
    use_test_round();
    let files = enumerate_files(env, &ExtraPaths::default(), &only_opencode());
    tracker_core::ingest::ingest(store, &files, |_| {})
}

fn events(store: &Store) -> Vec<EventRow> {
    store.events_between(0, i64::MAX).unwrap()
}

#[test]
fn each_finished_step_is_one_request_in_this_apps_token_meaning() {
    let db = opencode();
    let mut store = Store::open_in_memory().unwrap();
    let rep = run(&mut store, &db.env);
    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    assert!(rep.warnings.iter().any(|(_, w)| w.contains("1 sessions from OpenCode before 1.3.16")), "{:?}", rep.warnings);

    let ev = events(&store);
    let got: Vec<_> = ev
        .iter()
        .map(|e| {
            let t = e.tokens;
            (e.model.as_str(), e.client.as_deref().unwrap(), e.session_id.as_deref().unwrap(), e.agent.as_deref(), e.thread_id.as_deref(), [t.input, t.cache_read, t.cache_write, t.output, t.reasoning], e.request_input)
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("gpt-5", "openai", MAIN, None, None, [400, 800, 0, 300, 100], 1200),
            ("gpt-5", "openai", MAIN, Some("general"), Some(CHILD), [2000, 1000, 0, 70, 0], 3000),
            ("claude-sonnet-4-5", "openrouter", MAIN, None, None, [50, 700, 400, 120, 0], 1150),
            ("claude-sonnet-4-5", "anthropic", MAIN, Some("compact"), None, [900, 0, 0, 300, 0], 900),
        ]
    );
    assert!(ev.iter().all(|e| e.tool == Tool::OpenCode && e.tool.provider().is_none()));
    // the request's start, not the moment its step was written after the tool calls ran
    assert_eq!(ev[0].ts_ms, T0 + 1000);
    let projects = store.projects().unwrap();
    assert_eq!(projects.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["/work/app"]);

    // priced from this app's list under the vendor's model name, not from OpenCode's own cost
    let book = PriceBook::default_book();
    assert!(ev.iter().all(|e| book.lookup(&e.model).is_some()), "{:?}", ev.iter().map(|e| &e.model).collect::<Vec<_>>());

    let calls = store.tool_calls_between(0, i64::MAX).unwrap();
    let calls: Vec<_> = calls.iter().map(|c| (c.name.as_str(), c.session_id.as_deref().unwrap(), c.agent.as_deref(), c.failed)).collect();
    assert_eq!(calls, vec![("task", MAIN, None, Some(false)), ("read", MAIN, Some("general"), Some(true))]);
}

#[test]
fn only_sessions_with_new_rows_are_read_again_and_nothing_counts_twice() {
    let db = opencode();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &db.env);
    assert_eq!(events(&store).len(), 4);
    assert_eq!(run(&mut store, &db.env).files_read, 0, "an unchanged database is not read");

    // text streamed into an existing part changes the file but adds no request
    db.conn.execute("UPDATE part SET data = '{\"type\":\"text\",\"text\":\"more\"}' WHERE id = 'prt_001_text'", []).unwrap();
    let rep = run(&mut store, &db.env);
    assert_eq!((rep.files_read, rep.records_written), (1, 0));

    step(&db.conn, 7, MAIN, "anthropic", "claude-opus-5", "build", [10, 20, 0, 30, 40], "");
    run(&mut store, &db.env);
    // a revert removes parts; new ones written before the next read leave the count unchanged
    db.conn.execute("DELETE FROM part WHERE id IN ('prt_007_step', 'prt_007_text')", []).unwrap();
    step(&db.conn, 8, MAIN, "anthropic", "claude-haiku-4-5", "build", [10, 20, 0, 30, 40], "");
    run(&mut store, &db.env);
    assert!(events(&store).iter().any(|e| e.model == "claude-haiku-4-5"), "a request after a revert is read");
    let ev = events(&store);
    assert_eq!(ev.len(), 6, "a reverted request stays counted: its tokens were spent");
    assert_eq!(ev.iter().filter(|e| e.model == "claude-opus-5").count(), 1);

    // a replaced database (new file identity) is read from scratch without double counting
    drop(db.conn);
    let copy = db.path.with_file_name("copy.db");
    std::fs::copy(&db.path, &copy).unwrap();
    std::fs::remove_file(&db.path).unwrap();
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{ext}", db.path.display())));
    }
    std::fs::rename(&copy, &db.path).unwrap();
    run(&mut store, &db.env);
    assert_eq!(events(&store).len(), 6);
}

/// What OpenCode's `Session.fork` does: every message and part copied under new ids into a new
/// top-level session, their data (times, model, step tokens, call ids) unchanged.
fn fork(c: &Connection, from: &str, to: &str, version: &str) {
    c.execute(
        "INSERT INTO session(id, project_id, slug, directory, title, version, time_created, time_updated)
         SELECT ?2, project_id, slug, directory, title || ' (fork #1)', ?3, ?4, ?4 FROM session WHERE id = ?1",
        params![from, to, version, T0 + 90_000],
    )
    .unwrap();
    c.execute(
        "INSERT INTO message SELECT id || '_f', ?2, time_created + 90000, time_updated, data FROM message WHERE session_id = ?1",
        params![from, to],
    )
    .unwrap();
    c.execute(
        "INSERT INTO part SELECT id || '_f', message_id || '_f', ?2, time_created + 90000, time_updated, data FROM part WHERE session_id = ?1",
        params![from, to],
    )
    .unwrap();
}

#[test]
fn a_forked_session_does_not_count_its_copied_history_again() {
    let db = opencode();
    fork(&db.conn, MAIN, "ses_fork", "1.19.4");
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &db.env);
    let ev = events(&store);
    assert_eq!(ev.len(), 4, "the copies are the same requests");
    assert!(ev.iter().all(|e| e.session_id.as_deref() == Some(MAIN)), "they stay with the session they were made in");
    assert_eq!(store.tool_calls_between(0, i64::MAX).unwrap().len(), 2);

    // a request made after the fork is the fork's own
    step(&db.conn, 9, "ses_fork", "anthropic", "claude-opus-5", "build", [10, 20, 0, 30, 40], "");
    run(&mut store, &db.env);
    let ev = events(&store);
    assert_eq!(ev.len(), 5);
    assert_eq!(ev.iter().find(|e| e.model == "claude-opus-5").unwrap().session_id.as_deref(), Some("ses_fork"));
}

#[test]
fn a_fork_made_by_a_newer_version_does_not_bring_requests_from_before_1_3_16() {
    let db = opencode();
    let before = 1_775_000_000_000i64; // March 2026
    db.conn.execute("UPDATE message SET data = json_set(data, '$.time.created', ?1) WHERE session_id = ?2", params![before, OLD]).unwrap();
    fork(&db.conn, OLD, "ses_old_fork", "1.19.4");
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &db.env);
    let ev = events(&store);
    assert!(ev.iter().all(|e| e.session_id.as_deref() != Some("ses_old_fork")));
    assert_eq!(ev.len(), 4);
}

#[test]
fn the_reader_never_writes_to_opencodes_database() {
    let db = opencode();
    db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    let before = std::fs::read(&db.path).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &db.env);
    assert_eq!(std::fs::read(&db.path).unwrap(), before);
    assert_eq!(events(&store).len(), 4);
}

#[test]
fn opencode_is_found_in_its_xdg_folder_its_channel_databases_and_opencode_db() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let xdg = dir.path().join("xdg");
    let mk = |p: &Path| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        Connection::open(p).unwrap().execute_batch(SCHEMA).unwrap();
    };
    mk(&home.join(".local/share/opencode/opencode.db"));
    mk(&xdg.join("opencode/opencode.db"));
    mk(&xdg.join("opencode/opencode-dev.db"));
    mk(&dir.path().join("elsewhere/custom.db"));
    std::fs::write(xdg.join("opencode/opencode.db-wal"), b"").unwrap();
    std::fs::write(xdg.join("opencode/other.db"), b"").unwrap();

    let env = Env {
        home: Some(home.clone()),
        xdg_data_home: Some(xdg.to_string_lossy().into_owned()),
        opencode_db: Some(dir.path().join("elsewhere/custom.db").to_string_lossy().into_owned()),
        ..Default::default()
    };
    let mut found: Vec<String> = enumerate_files(&env, &ExtraPaths::default(), &only_opencode())
        .iter()
        .map(|f| f.path.strip_prefix(dir.path()).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    found.sort();
    assert_eq!(found, ["elsewhere/custom.db", "home/.local/share/opencode/opencode.db", "xdg/opencode/opencode-dev.db", "xdg/opencode/opencode.db"]);

    let relative = Env { opencode_db: Some("opencode-dev.db".into()), xdg_data_home: None, ..env.clone() };
    assert!(enumerate_files(&relative, &ExtraPaths::default(), &only_opencode()).iter().all(|f| !f.path.ends_with("custom.db")));
    let memory = Env { opencode_db: Some(":memory:".into()), ..env.clone() };
    assert_eq!(enumerate_files(&memory, &ExtraPaths::default(), &only_opencode()).len(), 3);

    let status = detect(&env, &ExtraPaths::default()).into_iter().find(|s| s.id == SourceId::OpenCode).unwrap();
    assert!(status.found && status.supported);
    let none = Env { home: Some(dir.path().join("nobody")), ..Default::default() };
    assert!(!detect(&none, &ExtraPaths::default()).into_iter().find(|s| s.id == SourceId::OpenCode).unwrap().found);
}

#[test]
fn a_file_that_is_not_opencodes_database_is_reported_not_read() {
    let db = opencode();
    drop(db.conn);
    std::fs::remove_file(&db.path).unwrap();
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{ext}", db.path.display())));
    }
    Connection::open(&db.path).unwrap().execute_batch("CREATE TABLE other(x);").unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let rep = run(&mut store, &db.env);
    assert!(rep.errors.is_empty());
    assert!(rep.warnings.iter().any(|(_, w)| w.contains("not an OpenCode database")));
    assert!(events(&store).is_empty());
}
