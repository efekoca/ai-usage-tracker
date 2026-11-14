mod common;

use common::*;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use tracker_core::analytics::{Filter, Range};
use tracker_core::model::Tool;
use tracker_core::pricing::PriceBook;
use tracker_core::store::Store;
use tracker_core::turns::{turns, TurnCost, TurnPart};

const DAY: Range = Range { from_ms: 1_788_480_000_000, to_ms: 1_788_566_400_000 }; // 2026-09-04 UTC

fn at(time: &str) -> String {
    format!("2026-09-04T{time}.000Z")
}

/// A user line as Claude Code writes it; `origin` is what began the turn, when the line says.
fn user(session: &str, time: &str, prompt: &str, origin: Option<&str>) -> Value {
    let mut v = json!({"type": "user", "sessionId": session, "timestamp": at(time), "cwd": "C:/Work/Turns", "promptId": prompt,
                       "message": {"role": "user", "content": "never stored"}});
    if let Some(o) = origin {
        v["origin"] = json!({"kind": o});
    }
    v
}

fn result(session: &str, time: &str, prompt: &str, call: &str, failed: bool) -> Value {
    json!({"type": "user", "sessionId": session, "timestamp": at(time), "cwd": "C:/Work/Turns", "promptId": prompt,
           "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": call, "is_error": failed}]}})
}

/// A response costing $0.30 (100K input and 10K output of claude-sonnet-5), with one tool call when named.
fn reply(session: &str, time: &str, id: &str, call: Option<&str>) -> Value {
    let content = call.map(|c| json!([{"type": "tool_use", "id": c, "name": "Bash"}])).unwrap_or(json!([{"type": "text"}]));
    json!({"type": "assistant", "sessionId": session, "timestamp": at(time), "cwd": "C:/Work/Turns",
           "message": {"id": id, "model": "claude-sonnet-5", "content": content, "usage": {"input_tokens": 100_000, "output_tokens": 10_000}}})
}

fn side(mut v: Value, agent: &str) -> Value {
    v["isSidechain"] = json!(true);
    v["agentId"] = json!(agent);
    v["attributionAgent"] = json!("Explore");
    v
}

fn write(path: &std::path::Path, lines: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
}

fn append(path: &std::path::Path, lines: &[Value]) {
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

fn of(store: &Store, session: &str) -> Vec<TurnCost> {
    let mut all = turns(store, &PriceBook::default_book(), DAY, &Filter::default()).unwrap().turns;
    all.retain(|t| t.session_id.as_deref() == Some(session));
    all.reverse();
    all
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_prompt_gathers_its_requests_subagents_and_tool_calls_into_one_turn() {
    let m = machine();
    let dir = m.home.join(".claude/projects/C--Work-Turns");
    let main = dir.join("s-turn.jsonl");
    let sub = dir.join("s-turn/subagents/agent-a1.jsonl");
    write(
        &main,
        &[
            user("s-turn", "10:00:00", "p1", Some("human")),
            reply("s-turn", "10:00:05", "msg_1", Some("toolu_task")),
            // the conversation is compacted on its own in the middle of the turn; the summary's
            // line is dated when an earlier summary was made
            {
                let mut c = user("s-turn", "07:03:00", "p1", None);
                c["isCompactSummary"] = json!(true);
                c
            },
            result("s-turn", "10:04:00", "p1", "toolu_task", false),
            reply("s-turn", "10:04:10", "msg_2", None),
            user("s-turn", "10:20:00", "p2", Some("task-notification")),
            reply("s-turn", "10:20:05", "msg_3", None),
        ],
    );
    write(
        &sub,
        &[
            side(user("s-turn", "10:00:10", "p1", None), "a1"),
            side(reply("s-turn", "10:00:20", "msg_a1", Some("toolu_sh")), "a1"),
            side(result("s-turn", "10:00:30", "p1", "toolu_sh", true), "a1"),
            // still working after the next turn began: it is the first prompt's work all the same
            side(reply("s-turn", "10:25:00", "msg_a2", None), "a1"),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);

    let got = of(&store, "s-turn");
    assert_eq!(got.len(), 2);
    let (first, second) = (&got[0], &got[1]);
    assert_eq!((first.origin.as_str(), first.ordinal, first.tool), ("human", 1, Tool::ClaudeCode), "a compaction inside the turn does not rename it");
    assert_eq!((first.requests, first.agents, first.tool_calls, first.tool_failed), (4, 1, 2, 1));
    assert!(close(first.cost_usd, 1.2), "{}", first.cost_usd);
    assert_eq!(first.total_tokens, 440_000);
    assert_eq!((first.started_ms - DAY.from_ms, first.ended_ms - DAY.from_ms), (10 * 3_600_000, 10 * 3_600_000 + 25 * 60_000));
    assert_eq!((first.project.as_str(), first.models.as_slice()), ("Turns", &["claude-sonnet-5".to_owned()][..]));
    assert_eq!(
        first.parts,
        [
            TurnPart { agent: None, threads: 0, requests: 2, cost_usd: first.parts[0].cost_usd },
            TurnPart { agent: Some("Explore".into()), threads: 1, requests: 2, cost_usd: first.parts[1].cost_usd },
        ]
    );
    assert!(close(first.parts[0].cost_usd, 0.6) && close(first.parts[1].cost_usd, 0.6));
    assert_eq!((second.origin.as_str(), second.ordinal, second.requests, second.agents, second.tool_calls), ("task_notification", 2, 1, 0, 0));
    assert!(close(second.cost_usd, 0.3));

    // a response appended later, with no user line before it in what is read, stays in the open turn
    append(&main, &[reply("s-turn", "10:21:00", "msg_4", None)]);
    run(&mut store, &m.env);
    let got = of(&store, "s-turn");
    assert_eq!((got[0].requests, got[1].requests), (4, 2));

    let prompts: i64 = store.conn().query_row("SELECT COUNT(*) FROM turn WHERE id IN ('cc:p1', 'cc:p2')", [], |r| r.get(0)).unwrap();
    assert_eq!(prompts, 2, "one record a turn, whatever lines repeat its id");
}

#[test]
fn what_began_a_turn_is_read_from_its_first_line() {
    let m = machine();
    let p = m.home.join(".claude/projects/C--Work-Turns/s-kind.jsonl");
    let mut scheduled = user("s-kind", "09:20:00", "k3", None);
    scheduled["turnOrigin"] = json!("scheduled");
    let mut meta = user("s-kind", "09:30:00", "k4", None);
    meta["isMeta"] = json!(true);
    let mut peer = user("s-kind", "09:40:00", "k5", Some("peer"));
    peer["isMeta"] = json!(true);
    let mut unnamed = user("s-kind", "09:50:00", "k6", Some("something-new"));
    unnamed["isMeta"] = json!(true);
    write(
        &p,
        &[
            // a version that named no origin wrote the prompt as a plain line
            user("s-kind", "09:00:00", "k1", None),
            reply("s-kind", "09:00:05", "msg_k1", None),
            // a later line of the same turn does not rename it
            {
                let mut later = user("s-kind", "09:01:00", "k1", None);
                later["isMeta"] = json!(true);
                later
            },
            reply("s-kind", "09:01:05", "msg_k1b", None),
            scheduled,
            reply("s-kind", "09:20:05", "msg_k3", None),
            meta,
            reply("s-kind", "09:30:05", "msg_k4", None),
            peer,
            reply("s-kind", "09:40:05", "msg_k5", None),
            unnamed,
            reply("s-kind", "09:50:05", "msg_k6", None),
            // a note written just before the prompt carries its id but not what began it
            {
                let mut note = user("s-kind", "09:52:00", "k8", None);
                note["isMeta"] = json!(true);
                note
            },
            user("s-kind", "09:52:01", "k8", Some("task-notification")),
            reply("s-kind", "09:52:05", "msg_k8", None),
            // a prompt nothing answered is no turn to show, and takes no place in the order
            user("s-kind", "09:53:00", "k9", Some("human")),
            user("s-kind", "09:55:00", "k7", Some("human")),
            reply("s-kind", "09:55:05", "msg_k7", Some("toolu_k7")),
            // a tool result with no prompt id leaves the turn open
            {
                let mut bare = result("s-kind", "09:55:30", "", "toolu_k7", false);
                bare.as_object_mut().unwrap().remove("promptId");
                bare
            },
            reply("s-kind", "09:55:35", "msg_k7b", None),
            // a prompt with no prompt id ends it: what answers that prompt belongs to no turn
            {
                let mut bare = user("s-kind", "09:56:00", "", Some("human"));
                bare.as_object_mut().unwrap().remove("promptId");
                bare
            },
            reply("s-kind", "09:56:05", "msg_k10", None),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let got: Vec<(String, u32, u64)> = of(&store, "s-kind").into_iter().map(|t| (t.origin, t.ordinal, t.requests)).collect();
    let want = [("human", 1, 2), ("scheduled", 2, 1), ("other", 3, 1), ("peer", 4, 1), ("other", 5, 1), ("task_notification", 6, 1), ("human", 7, 2)];
    assert_eq!(got, want.map(|(o, n, r)| (o.to_owned(), n, r)));
    let project = store.projects().unwrap().into_iter().find(|p| p.name == "Turns").unwrap().id;
    let r = turns(&store, &PriceBook::default_book(), DAY, &Filter { projects: vec![project], ..Default::default() }).unwrap();
    assert_eq!(r.requests_outside, 1, "the request after the prompt that named no turn");
}

#[test]
fn requests_of_a_log_that_names_no_turn_are_counted_apart() {
    let m = machine();
    let p = m.home.join(".claude/projects/C--Work-Turns/s-old.jsonl");
    write(&p, &[reply("s-old", "08:00:00", "msg_o1", None), reply("s-old", "08:01:00", "msg_o2", None), user("s-old", "08:02:00", "o1", Some("human")), reply("s-old", "08:02:05", "msg_o3", None)]);
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let project = store.projects().unwrap().into_iter().find(|p| p.name == "Turns").unwrap().id;
    let only = Filter { projects: vec![project], ..Default::default() };
    let r = turns(&store, &PriceBook::default_book(), DAY, &only).unwrap();
    assert_eq!((r.turns.len(), r.turns[0].requests, r.requests_outside, r.tracked), (1, 1, 2, true));
    let other = Filter { projects: vec![i64::MIN], ..Default::default() };
    let r = turns(&store, &PriceBook::default_book(), DAY, &other).unwrap();
    assert_eq!((r.turns.len(), r.requests_outside), (0, 0), "a turn none of whose requests pass the filter is left out");
    assert!(!turns(&Store::open_in_memory().unwrap(), &PriceBook::default_book(), DAY, &Filter::default()).unwrap().tracked);
}

fn cx(time: &str, payload: Value) -> Value {
    json!({"timestamp": at(time), "type": "event_msg", "payload": payload})
}

fn cx_tokens(time: &str, total: u64) -> Value {
    let usage = |n: u64| json!({"input_tokens": n, "cached_input_tokens": 0, "output_tokens": 10, "reasoning_output_tokens": 0, "total_tokens": n + 10});
    cx(time, json!({"type": "token_count", "info": {"total_token_usage": usage(total), "last_token_usage": usage(1000)}}))
}

fn cx_started(time: &str, turn: &str, root: &str, secs: i64, trigger: &str) -> Value {
    cx(time, json!({"type": "task_started", "turn_id": turn, "root_turn_id": root, "started_at": secs, "turn_attribution": {"turn_trigger": trigger}}))
}

#[test]
fn a_codex_subagent_counts_toward_the_turn_that_started_it() {
    let m = machine();
    let (main_id, sub_id) = ("0199cccc-0000-7000-8000-000000000001", "0199cccc-0000-7000-8000-000000000002");
    let dir = m.home.join(".codex/sessions/2026/09/04");
    let began = DAY.from_ms / 1000 + 11 * 3600;
    let shell = |time: &str, id: &str, exit: i64| {
        cx(time, json!({"type": "item_completed", "thread_id": main_id, "turn_id": "t1", "item": {"type": "CommandExecution", "id": id, "status": "completed", "exit_code": exit}}))
    };
    write(
        &dir.join(format!("rollout-2026-09-04T11-00-00-{main_id}.jsonl")),
        &[
            json!({"timestamp": at("11:00:00"), "type": "session_meta", "payload": {"id": main_id, "cwd": "C:\\Work\\Turns", "originator": "codex_cli_rs", "source": "cli"}}),
            json!({"timestamp": at("11:00:00"), "type": "turn_context", "payload": {"model": "gpt-5.3-codex"}}),
            // the line itself is dated later than the turn began, as in a copied history
            cx_started("11:00:09", "t1", "t1", began, "composer"),
            cx_tokens("11:00:10", 1000),
            shell("11:00:20", "call_1", 0),
            shell("11:00:30", "call_2", 1),
            cx_tokens("11:00:40", 2000),
            cx("11:00:50", json!({"type": "task_complete", "turn_id": "t1"})),
            cx_started("11:10:00", "t2", "t2", began + 600, "edit_user_message"),
            cx_tokens("11:10:10", 3000),
            cx("11:10:20", json!({"type": "turn_aborted", "turn_id": "t2"})),
            // nothing says which turn a request made between two of them belongs to
            cx_tokens("11:11:00", 4000),
        ],
    );
    write(
        &dir.join(format!("rollout-2026-09-04T11-00-15-{sub_id}.jsonl")),
        &[
            json!({"timestamp": at("11:00:15"), "type": "session_meta", "payload": {"id": sub_id, "session_id": main_id, "cwd": "C:\\Work\\Turns", "originator": "codex_cli_rs",
                   "source": {"subagent": {"other": "guardian"}}, "parent_thread_id": main_id}}),
            json!({"timestamp": at("11:00:15"), "type": "turn_context", "payload": {"model": "gpt-5.3-codex"}}),
            cx_started("11:00:16", "t9", "t1", began + 16, "guardian_review"),
            cx_tokens("11:00:18", 1000),
            cx("11:00:19", json!({"type": "task_complete", "turn_id": "t9"})),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);

    let got = of(&store, main_id);
    assert_eq!(got.len(), 2);
    let first = &got[0];
    assert_eq!((first.tool, first.origin.as_str(), first.ordinal), (Tool::Codex, "human", 1));
    assert_eq!(first.started_ms, began * 1000, "when the turn began, not when its line was written");
    assert_eq!((first.requests, first.agents, first.tool_calls, first.tool_failed), (3, 1, 2, 1));
    assert_eq!(first.parts.iter().map(|p| (p.agent.as_deref(), p.threads, p.requests)).collect::<Vec<_>>(), [(None, 0, 2), (Some("guardian"), 1, 1)]);
    assert_eq!((got[1].ordinal, got[1].requests, got[1].origin.as_str()), (2, 1, "human"));
    let codex = Filter { tools: vec![Tool::Codex], ..Default::default() };
    let all = turns(&store, &PriceBook::default_book(), DAY, &codex).unwrap();
    assert_eq!(all.requests_outside, 1);

    // a subagent whose main thread's log is gone names a turn nothing recorded
    let orphan = "0199cccc-0000-7000-8000-000000000003";
    write(
        &dir.join(format!("rollout-2026-09-04T12-00-00-{orphan}.jsonl")),
        &[
            json!({"timestamp": at("12:00:00"), "type": "session_meta", "payload": {"id": orphan, "session_id": "gone", "cwd": "C:\\Work\\Turns", "originator": "codex_cli_rs",
                   "source": {"subagent": {"other": "guardian"}}, "parent_thread_id": "gone"}}),
            json!({"timestamp": at("12:00:00"), "type": "turn_context", "payload": {"model": "gpt-5.3-codex"}}),
            cx_started("12:00:01", "t8", "t-gone", began + 3600, "guardian_review"),
            cx_tokens("12:00:02", 1000),
        ],
    );
    run(&mut store, &m.env);
    let all = turns(&store, &PriceBook::default_book(), DAY, &codex).unwrap();
    assert_eq!((all.turns.len(), all.requests_outside), (2, 2));
}

#[test]
fn turns_survive_a_backup_and_go_with_the_data() {
    let m = machine();
    write(&m.home.join(".claude/projects/C--Work-Turns/s-keep.jsonl"), &[user("s-keep", "12:00:00", "b1", Some("human")), reply("s-keep", "12:00:05", "msg_b1", Some("toolu_b"))]);
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("backup.db");
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let mut tx = store.transaction().unwrap();
    let report = tracker_core::model::CacheMissReport { session_id: "s-keep".into(), ts_ms: DAY.from_ms, last_miss_ms: None, requests: 3, misses: 1, rebuilds: 0, recached_tokens: 10, causes: vec![("tools_changed".into(), 1)] };
    tx.insert_cache_miss_reports(&[report]).unwrap();
    tx.insert_limit_stops(&[tracker_core::model::LimitStop { session_id: "s-keep".into(), ts_ms: DAY.from_ms, window: "five_hour".into(), resets_at: 1_788_500_000 }]).unwrap();
    tx.note_carried(&[(Tool::ClaudeCode, "s-keep".into())]).unwrap();
    tx.commit().unwrap();
    store.backup_to(&backup).unwrap();

    let mut fresh = Store::open_in_memory().unwrap();
    fresh.merge_from(&backup).unwrap();
    let got = of(&fresh, "s-keep");
    assert_eq!((got.len(), got[0].requests, got[0].tool_calls, got[0].origin.as_str()), (1, 1, 1, "human"));
    // the same backup again changes nothing
    fresh.merge_from(&backup).unwrap();
    assert_eq!(of(&fresh, "s-keep").len(), 1);
    assert_eq!(fresh.cache_miss_reports(0, i64::MAX).unwrap().len(), 1, "kept nowhere else once the status-line file is rotated away");
    assert_eq!(fresh.limit_stops("s-keep", 0, i64::MAX).unwrap().len(), 1);
    assert!(fresh.carried_sessions().unwrap().contains(&("claude_code".to_owned(), "s-keep".to_owned())));

    fresh.wipe_data().unwrap();
    assert!(!turns(&fresh, &PriceBook::default_book(), DAY, &Filter::default()).unwrap().tracked);
}

/// Takes an archive back to what the version before turns kept.
fn before_turns(path: &std::path::Path) {
    let s = Store::open(path).unwrap();
    s.conn()
        .execute_batch(
            "DROP INDEX usage_event_turn; DROP INDEX tool_call_turn; ALTER TABLE usage_event DROP COLUMN turn; ALTER TABLE tool_call DROP COLUMN turn;
             ALTER TABLE tool_call DROP COLUMN request; DROP TABLE turn; DROP TABLE limit_stop; DROP TABLE session_carried; PRAGMA user_version = 15;",
        )
        .unwrap();
}

#[test]
fn an_archive_from_before_turns_learns_them_from_the_logs_still_on_disk() {
    let m = machine();
    let log = m.home.join(".claude/projects/C--Work-Turns/s-up.jsonl");
    write(&log, &[user("s-up", "13:00:00", "u1", Some("human")), reply("s-up", "13:00:05", "msg_u1", Some("toolu_u")), reply("s-up", "13:00:09", "msg_u2", None)]);
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tracker.db");
    {
        let mut store = Store::open(&db).unwrap();
        run(&mut store, &m.env);
    }
    before_turns(&db);

    let mut store = Store::open(&db).unwrap();
    let events = store.event_count().unwrap();
    assert!(of(&store, "s-up").is_empty(), "nothing is known until the logs are read again");
    let rep = run(&mut store, &m.env);
    assert!(rep.files_read > 0, "the logs that name turns are read again");
    assert_eq!(store.event_count().unwrap(), events, "and nothing is counted twice");
    let got = of(&store, "s-up");
    assert_eq!((got.len(), got[0].requests, got[0].tool_calls), (1, 2, 1));

    // a log deleted before the upgrade leaves its requests in the archive, with no turn
    let gone = m.home.join(".claude/projects/C--Work-Turns/s-gone.jsonl");
    write(&gone, &[user("s-gone", "14:00:00", "g1", Some("human")), reply("s-gone", "14:00:05", "msg_g1", None)]);
    run(&mut store, &m.env);
    drop(store);
    before_turns(&db);
    std::fs::remove_file(&gone).unwrap();
    let mut store = Store::open(&db).unwrap();
    run(&mut store, &m.env);
    assert!(of(&store, "s-gone").is_empty());
    let project = store.projects().unwrap().into_iter().find(|p| p.name == "Turns").unwrap().id;
    let r = turns(&store, &PriceBook::default_book(), DAY, &Filter { projects: vec![project], ..Default::default() }).unwrap();
    assert_eq!((r.turns.len(), r.requests_outside), (1, 1));
}

#[test]
fn a_backup_from_before_turns_brings_its_requests_without_them() {
    let m = machine();
    write(&m.home.join(".claude/projects/C--Work-Turns/s-old-backup.jsonl"), &[user("s-old-backup", "15:00:00", "ob1", Some("human")), reply("s-old-backup", "15:00:05", "msg_ob1", Some("toolu_ob"))]);
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("old.db");
    {
        let mut store = Store::open(&backup).unwrap();
        run(&mut store, &m.env);
    }
    before_turns(&backup);

    let mut fresh = Store::open_in_memory().unwrap();
    let (events, _) = fresh.merge_from(&backup).unwrap();
    assert!(events > 0);
    let r = turns(&fresh, &PriceBook::default_book(), DAY, &Filter::default()).unwrap();
    assert_eq!((r.turns.len(), r.tracked), (0, false));
    assert!(r.requests_outside >= 1);
    // the logs, read afterwards, give the same requests their turn
    run(&mut fresh, &m.env);
    let got = of(&fresh, "s-old-backup");
    assert_eq!((got.len(), got[0].requests, got[0].tool_calls), (1, 1, 1));
}

#[test]
fn a_turn_the_filter_splits_keeps_the_tool_calls_of_the_requests_it_keeps() {
    let m = machine();
    let mut opus = reply("s-mix", "16:00:20", "msg_m2", Some("toolu_m2"));
    opus["message"]["model"] = json!("claude-opus-5-5");
    write(
        &m.home.join(".claude/projects/C--Work-Turns/s-mix.jsonl"),
        &[
            user("s-mix", "16:00:00", "m1", Some("human")),
            reply("s-mix", "16:00:05", "msg_m1", Some("toolu_m1")),
            result("s-mix", "16:00:10", "m1", "toolu_m1", false),
            opus,
            result("s-mix", "16:00:30", "m1", "toolu_m2", true),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let book = PriceBook::default_book();
    let all = of(&store, "s-mix");
    assert_eq!((all[0].requests, all[0].tool_calls, all[0].tool_failed, all[0].calls_hidden), (2, 2, 1, false));

    // each call counts with the request that made it: the other model's failure is not this one's
    let only = |model: &str, store: &Store| {
        let got = turns(store, &book, DAY, &Filter { models: vec![model.into()], ..Default::default() }).unwrap().turns;
        let t = got.into_iter().find(|t| t.session_id.as_deref() == Some("s-mix")).unwrap();
        (t.requests, t.tool_calls, t.tool_failed, t.calls_hidden)
    };
    assert_eq!(only("claude-sonnet-5", &store), (1, 1, 0, false));
    assert_eq!(only("claude-opus-5-5", &store), (1, 1, 1, false));

    // a call whose log does not say which request made it cannot be split: none is shown
    let mut tx = store.transaction().unwrap();
    tx.upsert_tool_calls(&[tracker_core::model::ToolCall {
        key: "cct:untied".into(),
        ts_ms: DAY.from_ms + 16 * 3_600_000 + 40_000,
        tool: Tool::ClaudeCode,
        session_id: Some("s-mix".into()),
        project_path: Some("C:/Work/Turns".into()),
        agent: None,
        name: "Bash".into(),
        failed: Some(true),
        turn: Some("cc:m1".into()),
        request: None,
    }])
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(only("claude-sonnet-5", &store), (1, 0, 0, true));
    assert_eq!(of(&store, "s-mix")[0].tool_calls, 3, "with every request kept, every call counts");

    // a filter every request of the turn passes hides nothing
    let project = store.projects().unwrap().into_iter().find(|p| p.name == "Turns").unwrap().id;
    let both = Filter { projects: vec![project], tools: vec![Tool::ClaudeCode], ..Default::default() };
    let got = turns(&store, &book, DAY, &both).unwrap().turns;
    let t = got.iter().find(|t| t.session_id.as_deref() == Some("s-mix")).unwrap();
    assert_eq!((t.tool_calls, t.tool_failed, t.calls_hidden), (3, 2, false));
}

#[test]
fn lines_written_again_under_a_later_prompts_id_do_not_move_its_start() {
    let m = machine();
    let line = |time: &str, prompt: &str, uuid: &str| {
        let mut v = user("s-again", time, prompt, Some("human"));
        v["uuid"] = json!(uuid);
        v
    };
    write(
        &m.home.join(".claude/projects/C--Work-Turns/s-again.jsonl"),
        &[
            line("10:00:00", "w1", "u-1"),
            reply("s-again", "10:00:05", "msg_w1", None),
            // the desktop app writes the earlier lines once more, now under the next prompt's id
            line("10:00:00", "w2", "u-1"),
            reply("s-again", "10:00:05", "msg_w1", None),
            line("12:00:00", "w2", "u-2"),
            reply("s-again", "12:00:05", "msg_w2", None),
            // and again after it
            line("10:00:00", "w2", "u-1"),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let got: Vec<(i64, u32, u64)> = of(&store, "s-again").iter().map(|t| ((t.started_ms - DAY.from_ms) / 3_600_000, t.ordinal, t.requests)).collect();
    assert_eq!(got, [(10, 1, 1), (12, 2, 1)]);
}

#[test]
fn a_codex_subagent_whose_log_names_no_root_turn_is_no_turn_of_its_own() {
    let m = machine();
    let (main_id, sub_id) = ("0199dddd-0000-7000-8000-000000000001", "0199dddd-0000-7000-8000-000000000002");
    let dir = m.home.join(".codex/sessions/2026/09/04");
    let began = DAY.from_ms / 1000 + 17 * 3600;
    write(
        &dir.join(format!("rollout-2026-09-04T17-00-00-{main_id}.jsonl")),
        &[
            json!({"timestamp": at("17:00:00"), "type": "session_meta", "payload": {"id": main_id, "cwd": "C:\\Work\\Turns", "originator": "codex_cli_rs", "source": "cli"}}),
            json!({"timestamp": at("17:00:00"), "type": "turn_context", "payload": {"model": "gpt-5.3-codex"}}),
            // an older version: the turn names itself only
            cx("17:00:01", json!({"type": "task_started", "turn_id": "o1", "started_at": began})),
            cx_tokens("17:00:10", 1000),
            cx("17:00:20", json!({"type": "task_complete", "turn_id": "o1"})),
        ],
    );
    write(
        &dir.join(format!("rollout-2026-09-04T17-00-05-{sub_id}.jsonl")),
        &[
            json!({"timestamp": at("17:00:05"), "type": "session_meta", "payload": {"id": sub_id, "session_id": main_id, "cwd": "C:\\Work\\Turns", "originator": "codex_cli_rs",
                   "source": {"subagent": {"other": "guardian"}}, "parent_thread_id": main_id}}),
            json!({"timestamp": at("17:00:05"), "type": "turn_context", "payload": {"model": "gpt-5.3-codex"}}),
            cx("17:00:06", json!({"type": "task_started", "turn_id": "o9", "started_at": began + 6})),
            cx_tokens("17:00:08", 1000),
        ],
    );
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let got = of(&store, main_id);
    assert_eq!((got.len(), got[0].requests, got[0].agents, got[0].origin.as_str()), (1, 1, 0, "human"), "the main thread's own turn");
    let r = turns(&store, &PriceBook::default_book(), DAY, &Filter { tools: vec![Tool::Codex], ..Default::default() }).unwrap();
    assert_eq!(r.requests_outside, 1, "the subagent's request belongs to no turn the log names");
}
