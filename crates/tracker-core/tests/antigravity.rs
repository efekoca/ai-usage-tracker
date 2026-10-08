mod common;

use common::*;
use std::fs;
use std::path::{Path, PathBuf};
use tracker_core::discovery::{detect, enumerate_files, Env, ExtraPaths, SourceId};
use tracker_core::pricing::{CostInput, PriceBook};
use tracker_core::rusqlite::{params, Connection};
use tracker_core::store::Store;

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn int(field: u64, v: u64) -> Vec<u8> {
    let mut o = Vec::new();
    varint(field << 3, &mut o);
    varint(v, &mut o);
    o
}

fn bytes(field: u64, b: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    varint(field << 3 | 2, &mut o);
    varint(b.len() as u64, &mut o);
    o.extend_from_slice(b);
    o
}

fn s(field: u64, v: &str) -> Vec<u8> {
    bytes(field, v.as_bytes())
}

fn m(field: u64, parts: &[Vec<u8>]) -> Vec<u8> {
    bytes(field, &parts.concat())
}

fn ts(field: u64, secs: u64) -> Vec<u8> {
    m(field, &[int(1, secs), int(2, 250_000_000)])
}

/// `(input, output, cache_read, thinking, response)`; zero fields are left out like Antigravity does.
fn usage(field: u64, u: (u64, u64, u64, u64, u64)) -> Vec<u8> {
    let mut p = vec![int(1, 1318)];
    for (f, v) in [(2, u.0), (3, u.1), (5, u.2), (9, u.3), (10, u.4)] {
        if v > 0 {
            p.push(int(f, v));
        }
    }
    p.push(int(6, 24));
    m(field, &p)
}

fn kv(k: &str, v: &str) -> Vec<u8> {
    m(20, &[s(1, k), s(2, v)])
}

fn generation(model: &str, total: (u64, u64, u64, u64, u64), calls: &[(u64, u64, u64, u64, u64)], last_step: i64) -> Vec<u8> {
    let mut inner = vec![int(3, 1318), usage(4, total)];
    for c in calls {
        inner.push(m(17, &[usage(2, *c)]));
    }
    inner.push(s(19, model));
    inner.push(kv("model_enum", "MODEL_PLACEHOLDER_M318"));
    inner.push(kv("last_step_index", &last_step.to_string()));
    m(1, &inner)
}

fn step_meta(secs: u64, call: Option<(&str, &str)>) -> Vec<u8> {
    let mut p = vec![ts(1, secs), int(3, 2)];
    if let Some((id, name)) = call {
        p.push(m(4, &[s(1, id), s(2, name), s(9, name)]));
    }
    p.concat()
}

const T0: u64 = 1_791_192_252; // 2026-10-05T09:24:12Z

fn workspace(uri: &str, branch: &str) -> Vec<u8> {
    [m(1, &[s(1, uri), s(2, uri), s(4, branch)]), ts(2, T0 - 60), s(7, uri)].concat()
}

/// A conversation database with the real schema, left open so new rows stay in the WAL.
fn conversation(dir: &Path, id: &str, ws: &[u8]) -> Connection {
    fs::create_dir_all(dir.join("conversations")).unwrap();
    let c = Connection::open(dir.join("conversations").join(format!("{id}.db"))).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
         CREATE TABLE `trajectory_meta` (`trajectory_id` text,`cascade_id` text,`trajectory_type` integer,`source` integer,PRIMARY KEY (`trajectory_id`));
         CREATE TABLE `steps` (`idx` integer,`step_type` integer NOT NULL DEFAULT 0,`status` integer NOT NULL DEFAULT 0,`has_subtrajectory` numeric NOT NULL DEFAULT false,`metadata` blob,`error_details` blob,`permissions` blob,`task_details` blob,`render_info` blob,`step_payload` blob,`step_format` integer NOT NULL DEFAULT 0,PRIMARY KEY (`idx`));
         CREATE TABLE `gen_metadata` (`idx` integer,`data` blob,`size` integer NOT NULL DEFAULT 0,PRIMARY KEY (`idx`));
         CREATE TABLE `trajectory_metadata_blob` (`id` text DEFAULT \"main\",`data` blob,PRIMARY KEY (`id`));",
    )
    .unwrap();
    c.execute("INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)", [ws]).unwrap();
    c
}

fn add_step(c: &Connection, idx: i64, step_type: i64, status: i64, meta: Vec<u8>, error: Option<&[u8]>) {
    c.execute(
        "INSERT INTO steps (idx, step_type, status, metadata, error_details) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![idx, step_type, status, meta, error],
    )
    .unwrap();
}

fn add_gen(c: &Connection, idx: i64, data: Vec<u8>) {
    c.execute("INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, ?3)", params![idx, data, 0]).unwrap();
}

struct Gemini {
    _dir: tempfile::TempDir,
    env: Env,
    gemini: PathBuf,
    /// Open connections: Antigravity is "running", so recent rows are still in the WALs.
    conns: Vec<Connection>,
}

const APP_ID: &str = "f607803a-0e2c-4833-a5c4-b4ed8cb94227";
const IDE_ID: &str = "31c31050-3302-4d51-b363-c7ca17ac4243";
const CLI_ID: &str = "0a1b2c3d-0000-4000-8000-000000000001";
const SUB_ID: &str = "0a1b2c3d-0000-4000-8000-000000000002";

fn gemini() -> Gemini {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Users").join("Başka Kullanıcı");
    let gemini = home.join(".gemini");

    // Antigravity 2.0: a Gemini conversation on Windows with a retried generation and three tool calls
    let app = conversation(&gemini.join("antigravity"), APP_ID, &workspace("file:///c%3A/Users/Ba%C5%9Fka/Desktop/app", "main"));
    add_step(&app, 0, 14, 3, step_meta(T0, None), None);
    add_step(&app, 1, 15, 3, step_meta(T0 + 5, None), None);
    add_step(&app, 2, 8, 3, step_meta(T0 + 6, Some(("toolu_1", "view_file"))), None);
    add_step(&app, 3, 21, 7, step_meta(T0 + 7, Some(("toolu_2", "run_command"))), Some(b"\x0a\x04fail"));
    add_step(&app, 4, 15, 3, step_meta(T0 + 20, None), None);
    add_step(&app, 5, 85, 2, step_meta(T0 + 21, Some(("toolu_3", "ask_question"))), None);
    add_gen(&app, 0, generation("gemini-3.8-flash-n", (12_463, 267, 0, 200, 67), &[], 0));
    add_gen(&app, 1, generation("gemini-3.8-flash-n", (5_000, 300, 30_000, 100, 200), &[(2_000, 100, 20_000, 40, 60), (3_000, 200, 10_000, 60, 140)], 3));
    // failed before the model answered: no usage
    add_gen(&app, 2, m(1, &[int(3, 1318), s(19, "gemini-3.8-flash-n"), kv("last_step_index", "5")]));

    // IDE: a Claude model on macOS; output reported only as thinking + response
    let ide = conversation(&gemini.join("antigravity-ide"), IDE_ID, &workspace("file:///Users/u/My%20App/", "feature/x"));
    add_step(&ide, 0, 14, 3, step_meta(T0 + 100, None), None);
    add_step(&ide, 1, 15, 3, step_meta(T0 + 104, None), None);
    add_gen(&ide, 0, generation("claude-opus-4-6-thinking", (1_532, 0, 18_164, 300, 118), &[], 0));

    // CLI: a conversation and a subagent it started
    let cli_dir = gemini.join("antigravity-cli");
    let cli = conversation(&cli_dir, CLI_ID, &workspace("file:///home/u/proj", ""));
    add_step(&cli, 0, 14, 3, step_meta(T0 + 200, None), None);
    add_step(&cli, 1, 15, 3, step_meta(T0 + 201, None), None);
    add_gen(&cli, 0, generation("gemini-3.6-flash", (1_000, 50, 0, 0, 50), &[], 0));
    let sub = conversation(&cli_dir, SUB_ID, &workspace("file:///home/u/proj", ""));
    add_step(&sub, 0, 14, 3, step_meta(T0 + 210, None), None);
    add_step(&sub, 1, 15, 3, step_meta(T0 + 211, None), None);
    add_gen(&sub, 0, generation("gemini-3.6-flash", (400, 20, 0, 0, 20), &[], 0));
    let summaries = Connection::open(cli_dir.join("conversation_summaries.db")).unwrap();
    summaries
        .execute_batch(&format!(
            "CREATE TABLE `conversation_summaries` (`conversation_id` text,`parent_conversation_id` text NOT NULL DEFAULT \"\",`agent_name` text NOT NULL DEFAULT \"\",PRIMARY KEY (`conversation_id`));
             INSERT INTO conversation_summaries VALUES ('{CLI_ID}', '', ''), ('{SUB_ID}', '{CLI_ID}', 'research');"
        ))
        .unwrap();
    drop(summaries);

    // a browser profile is not a data folder
    fs::create_dir_all(gemini.join("antigravity-browser-profile").join("Default")).unwrap();

    let env = Env { home: Some(home), ..Default::default() };
    Gemini { _dir: dir, env, gemini, conns: vec![app, ide, cli, sub] }
}

fn antigravity_only() -> std::collections::HashSet<SourceId> {
    [SourceId::Antigravity].into_iter().collect()
}

fn run_ag(store: &mut Store, env: &Env) -> tracker_core::ingest::IngestReport {
    let files = enumerate_files(env, &ExtraPaths::default(), &antigravity_only());
    tracker_core::ingest::ingest(store, &files, |_| {})
}

#[derive(Debug)]
struct Ev {
    key: String,
    ts_ms: i64,
    client: String,
    model: String,
    path: Option<String>,
    session: String,
    input: i64,
    cache_read: i64,
    cache_write: i64,
    output: i64,
    reasoning: i64,
    request_input: i64,
    branch: Option<String>,
    agent: Option<String>,
    thread: Option<String>,
}

fn rows(store: &Store) -> Vec<Ev> {
    let mut q = store
        .conn()
        .prepare(
            "SELECT e.key, e.ts_ms, e.client, e.model, p.display_path, e.session_id, e.input, e.cache_read, e.cache_write, e.output,
                    e.reasoning, e.request_input, e.branch, e.agent, e.thread_id
             FROM usage_event e LEFT JOIN project p ON p.id = e.project_id WHERE e.tool = 'antigravity' ORDER BY e.ts_ms",
        )
        .unwrap();
    q.query_map([], |r| {
        Ok(Ev {
            key: r.get(0)?,
            ts_ms: r.get(1)?,
            client: r.get(2)?,
            model: r.get(3)?,
            path: r.get(4)?,
            session: r.get(5)?,
            input: r.get(6)?,
            cache_read: r.get(7)?,
            cache_write: r.get(8)?,
            output: r.get(9)?,
            reasoning: r.get(10)?,
            request_input: r.get(11)?,
            branch: r.get(12)?,
            agent: r.get(13)?,
            thread: r.get(14)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn reads_the_app_ide_and_cli_with_exact_tokens() {
    let g = gemini();
    let mut store = Store::open_in_memory().unwrap();
    let rep = run_ag(&mut store, &g.env);
    assert_eq!(rep.errors, vec![]);
    assert_eq!(rep.files_seen, 4);
    let r = rows(&store);
    assert_eq!(r.len(), 5, "{r:#?}");
    let ms = |secs: u64| (secs * 1000 + 250) as i64;

    let e = &r[0];
    assert_eq!(e.key, format!("ag:{APP_ID}:0"));
    assert_eq!(e.ts_ms, ms(T0 + 5), "a generation is dated by the step it wrote");
    assert_eq!((e.client.as_str(), e.model.as_str()), ("antigravity", "gemini-3.8-flash-n"));
    assert_eq!(e.path.as_deref(), Some(r"c:\Users\Başka\Desktop\app"));
    assert_eq!(e.session, APP_ID);
    assert_eq!((e.input, e.cache_read, e.cache_write, e.output, e.reasoning, e.request_input), (12_463, 0, 0, 267, 200, 12_463));
    assert_eq!((e.branch.as_deref(), &e.agent, &e.thread), (Some("main"), &None, &None));

    let e = &r[1];
    assert_eq!(e.ts_ms, ms(T0 + 20));
    assert_eq!((e.input, e.cache_read, e.output, e.reasoning), (5_000, 30_000, 300, 100), "a retried generation counts every call");
    assert_eq!(e.request_input, 22_000, "the long-context tier follows the largest single request");

    let e = &r[2];
    assert_eq!((e.client.as_str(), e.model.as_str()), ("antigravity_ide", "claude-opus-4-6-thinking"));
    assert_eq!(e.path.as_deref(), Some("/Users/u/My App"));
    assert_eq!((e.input, e.cache_read, e.output, e.reasoning), (1_532, 18_164, 418, 300), "output falls back to thinking + response");
    assert_eq!(e.branch.as_deref(), Some("feature/x"));

    let e = &r[3];
    assert_eq!((e.client.as_str(), e.path.as_deref(), e.session.as_str()), ("antigravity_cli", Some("/home/u/proj"), CLI_ID));
    assert_eq!((&e.branch, &e.agent, &e.thread), (&None, &None, &None));

    let e = &r[4];
    assert_eq!(e.input, 400);
    assert_eq!(e.session, CLI_ID, "a subagent counts toward the conversation that started it");
    assert_eq!((e.agent.as_deref(), e.thread.as_deref()), (Some("research"), Some(SUB_ID)));
    drop(g.conns);
}

#[test]
fn tool_calls_keep_names_and_outcomes_only() {
    let g = gemini();
    let mut store = Store::open_in_memory().unwrap();
    run_ag(&mut store, &g.env);
    let mut q = store
        .conn()
        .prepare("SELECT key, name, failed FROM tool_call WHERE tool = 'antigravity' ORDER BY ts_ms")
        .unwrap();
    let calls: Vec<(String, String, Option<bool>)> =
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(
        calls,
        vec![
            (format!("agt:{APP_ID}:toolu_1"), "view_file".into(), Some(false)),
            (format!("agt:{APP_ID}:toolu_2"), "run_command".into(), Some(true)),
            (format!("agt:{APP_ID}:toolu_3"), "ask_question".into(), None),
        ]
    );
}

#[test]
fn rows_still_in_the_wal_are_read_once_and_new_ones_are_picked_up() {
    let g = gemini();
    let app_db = g.gemini.join("antigravity").join("conversations").join(format!("{APP_ID}.db"));
    let mut wal = app_db.clone().into_os_string();
    wal.push("-wal");
    assert!(fs::metadata(&wal).unwrap().len() > 0, "the fixture keeps its rows in the WAL");

    let mut store = Store::open_in_memory().unwrap();
    run_ag(&mut store, &g.env);
    assert_eq!(rows(&store).len(), 5);
    let again = run_ag(&mut store, &g.env);
    assert_eq!((again.files_read, again.records_written), (0, 0), "unchanged databases are skipped");

    add_step(&g.conns[0], 6, 15, 3, step_meta(T0 + 40, None), None);
    add_gen(&g.conns[0], 3, generation("gemini-3.8-flash-n", (700, 70, 0, 0, 70), &[], 5));
    let rep = run_ag(&mut store, &g.env);
    assert_eq!(rep.files_read, 1, "only the database that changed is read");
    let r = rows(&store);
    assert_eq!(r.len(), 6);
    assert_eq!(r.iter().map(|x| x.input).sum::<i64>(), 12_463 + 5_000 + 1_532 + 1_000 + 400 + 700);

    // a checkpoint moves the rows into the database file without changing them
    g.conns[0].execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    run_ag(&mut store, &g.env);
    assert_eq!(rows(&store).len(), 6);
}

#[test]
fn the_live_database_is_left_untouched() {
    let g = gemini();
    let dir = g.gemini.join("antigravity").join("conversations");
    let listing = |d: &Path| {
        let mut v: Vec<(String, u64)> =
            fs::read_dir(d).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().len())).collect();
        v.sort();
        v
    };
    let before = listing(&dir);
    let mut store = Store::open_in_memory().unwrap();
    run_ag(&mut store, &g.env);
    assert_eq!(listing(&dir), before);
}

#[test]
fn detection_finds_each_surface_and_ignores_the_browser_profile() {
    let g = gemini();
    let st = detect(&g.env, &ExtraPaths::default());
    let ag = st.iter().find(|s| s.id == SourceId::Antigravity).unwrap();
    assert!(ag.found && ag.supported);
    assert_eq!(ag.file_count, 4);
    let names: Vec<String> = ag.roots.iter().map(|r| r.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["antigravity", "antigravity-ide", "antigravity-cli"]);

    // a CLI that is set up but has no conversations yet still counts as installed
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".gemini").join("antigravity-cli")).unwrap();
    fs::write(dir.path().join(".gemini").join("antigravity-cli").join("settings.json"), "{}").unwrap();
    let env = Env { home: Some(dir.path().to_owned()), ..Default::default() };
    let ag = detect(&env, &ExtraPaths::default()).into_iter().find(|s| s.id == SourceId::Antigravity).unwrap();
    assert!(ag.found && ag.file_count == 0);

    let none = Env { home: Some(tempfile::tempdir().unwrap().path().to_owned()), ..Default::default() };
    let ag = detect(&none, &ExtraPaths::default()).into_iter().find(|s| s.id == SourceId::Antigravity).unwrap();
    assert!(!ag.found);
}

#[test]
fn a_custom_data_folder_is_scanned() {
    let g = gemini();
    let other = g._dir.path().join("elsewhere").join("agy-data");
    let c = conversation(&other, "11111111-0000-4000-8000-000000000000", &workspace("file:///srv/x", ""));
    add_step(&c, 0, 14, 3, step_meta(T0, None), None);
    add_step(&c, 1, 15, 3, step_meta(T0 + 1, None), None);
    add_gen(&c, 0, generation("gemini-3.6-flash", (10, 1, 0, 0, 1), &[], 0));
    let env = Env { antigravity_data_dir: Some(other.to_string_lossy().into_owned()), ..g.env.clone() };
    let mut store = Store::open_in_memory().unwrap();
    run_ag(&mut store, &env);
    let r = rows(&store);
    assert_eq!(r.len(), 6);
    assert!(r.iter().any(|x| x.client == "agy_data" && x.path.as_deref() == Some("/srv/x")));
}

#[test]
fn unreadable_or_foreign_databases_do_not_stop_the_scan() {
    let g = gemini();
    let conv = g.gemini.join("antigravity").join("conversations");
    fs::write(conv.join("broken.db"), b"not a database at all, just bytes").unwrap();
    Connection::open(conv.join("other.db")).unwrap().execute_batch("CREATE TABLE t (x);").unwrap();
    let mut store = Store::open_in_memory().unwrap();
    let rep = run_ag(&mut store, &g.env);
    assert_eq!(rep.errors, vec![]);
    let mut warned: Vec<&str> = rep.warnings.iter().map(|(p, _)| p.rsplit(['/', '\\']).next().unwrap()).collect();
    warned.sort();
    assert_eq!(warned, ["broken.db", "other.db"], "skipped with a warning and not re-read until they change");
    assert_eq!(run_ag(&mut store, &g.env).files_read, 0);
    assert_eq!(rows(&store).len(), 5);
}

#[test]
fn antigravity_models_are_priced_under_their_public_names() {
    let b = PriceBook::default_book();
    for (id, public) in [
        ("gemini-3.8-flash-n", "gemini-3.8-flash"),
        ("gemini-3-flash-a", "gemini-3.5-flash"),
        ("claude-opus-4-6-thinking", "claude-opus-4-6"),
        ("claude-sonnet-4-6", "claude-sonnet-4-6"),
        ("gemini-3.6-flash", "gemini-3.6-flash"),
    ] {
        assert_eq!(b.lookup(id).map(|p| p.id.as_str()), Some(public), "{id}");
    }
    let t = tracker_core::model::Tokens { input: 1_000_000, cache_read: 1_000_000, output: 1_000_000, ..Default::default() };
    let cost = |model, request_input| {
        let c = b.cost(&CostInput { model, tokens: &t, request_input, web_search_requests: 0, speed: None, inference_geo: None }).unwrap();
        (c.total() * 1e6).round() / 1e6
    };
    assert_eq!(cost("gemini-3.8-flash-n", 0), 0.75 + 0.075 + 3.75);
    assert_eq!(cost("gemini-3.1-pro-preview", 200_001), 4.0 + 0.4 + 18.0);
    let mut sample = Store::open_in_memory().unwrap();
    let g = gemini();
    run_ag(&mut sample, &g.env);
    let unpriced: Vec<String> = rows(&sample).into_iter().map(|r| r.model).filter(|m| b.lookup(m).is_none()).collect();
    assert_eq!(unpriced, Vec::<String>::new());
}

#[test]
fn the_full_machine_scan_includes_antigravity_next_to_the_other_tools() {
    let mut mc = machine();
    let g = gemini();
    let gem = mc.home.join(".gemini");
    copy_dir(&g.gemini, &gem);
    mc.env.home = Some(mc.home.clone());
    let mut store = Store::open_in_memory().unwrap();
    let rep = run(&mut store, &mc.env);
    assert_eq!(rep.errors, vec![]);
    let tools: Vec<String> = store
        .conn()
        .prepare("SELECT DISTINCT tool FROM usage_event ORDER BY tool")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(tools, ["antigravity", "claude_code", "codex"]);
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            fs::copy(e.path(), dst).unwrap();
        }
    }
}
