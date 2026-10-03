//! End-to-end tests over a fake user profile built from synthetic fixtures.

mod common;

use common::*;
use std::fs;
use std::io::Write;
use tracker_core::discovery::{detect, enumerate_files, Env, ExtraPaths, SourceId};
use tracker_core::store::Store;

#[derive(Debug, PartialEq, Default)]
struct Sums {
    events: i64,
    input: i64,
    cache_read: i64,
    cache_write: i64,
    cache_write_1h: i64,
    output: i64,
    reasoning: i64,
    web_search: i64,
}

fn sums(store: &Store, tool: &str) -> Sums {
    store
        .conn()
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(input),0), COALESCE(SUM(cache_read),0), COALESCE(SUM(cache_write),0),
                    COALESCE(SUM(cache_write_1h),0), COALESCE(SUM(output),0), COALESCE(SUM(reasoning),0),
                    COALESCE(SUM(web_search),0)
             FROM usage_event WHERE tool = ?1 AND (?2 = '' OR source = ?2)",
            [tool.split(':').next().unwrap(), tool.split(':').nth(1).unwrap_or("")],
            |r| {
                Ok(Sums {
                    events: r.get(0)?,
                    input: r.get(1)?,
                    cache_read: r.get(2)?,
                    cache_write: r.get(3)?,
                    cache_write_1h: r.get(4)?,
                    output: r.get(5)?,
                    reasoning: r.get(6)?,
                    web_search: r.get(7)?,
                })
            },
        )
        .unwrap()
}

#[test]
fn detects_installed_tools_for_any_user_name() {
    let m = machine();
    let st = detect(&m.env, &ExtraPaths::default());
    let found = |id| st.iter().find(|s| s.id == id).unwrap();
    assert!(found(SourceId::ClaudeCode).found);
    assert_eq!(found(SourceId::ClaudeCode).file_count, 2);
    assert!(found(SourceId::Cowork).found);
    assert_eq!(found(SourceId::Cowork).file_count, 2); // transcript + audit
    assert!(found(SourceId::ClaudeDesktop).found);
    assert_eq!(found(SourceId::Codex).file_count, 2);
    assert!(!found(SourceId::ChatgptDesktop).found);
    assert!(!found(SourceId::ChatgptDesktop).supported);
}

#[test]
fn ingests_exact_totals_without_double_counting() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    let rep = run(&mut store, &m.env);
    assert!(rep.errors.is_empty(), "{:?}", rep.errors);

    // Claude Code: msg a1 (streamed twice, output 10→120), a2 (zeroed copy + real copy, and
    // again in the resumed session b), b1. Synthetic/error lines are ignored.
    assert_eq!(
        sums(&store, "claude_code:claude_code_jsonl"),
        Sums { events: 3, input: 15, cache_read: 5500, cache_write: 1200, cache_write_1h: 1000, output: 240, reasoning: 30, web_search: 1 }
    );
    // Cowork transcript only; the duplicate assistant line in audit.jsonl must not count.
    assert_eq!(
        sums(&store, "claude_code:cowork_jsonl"),
        Sums { events: 1, input: 2, cache_read: 100, cache_write: 50, output: 30, ..Default::default() }
    );
    // Codex: 4 distinct requests in the main rollout (one repeated snapshot skipped, totals
    // reset after compaction) + 1 in the archived sub-agent rollout.
    assert_eq!(
        sums(&store, "codex"),
        Sums { events: 5, input: 1700, cache_read: 2800, cache_write: 100, output: 440, reasoning: 110, ..Default::default() }
    );
    // Limits: 1 Claude Code rejection + 2 Cowork windows + 5 plan-history values + 4 Codex windows.
    assert_eq!(store.limit_count().unwrap(), 12);

    // the malformed line is reported, content-free
    assert!(rep.warnings.iter().any(|(p, w)| p.ends_with("session-a.jsonl") && w.starts_with("invalid JSON at byte")));
}

#[test]
fn codex_models_follow_turn_context_and_cached_input_is_split_out() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let rows: Vec<(String, i64, i64, i64)> = store
        .conn()
        .prepare("SELECT model, input, cache_read, request_input FROM usage_event WHERE tool='codex' ORDER BY ts_ms")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("gpt-5.6-sol".into(), 400, 600, 1000),
            ("gpt-5.6-sol".into(), 500, 1400, 2000),
            ("codex-auto-review".into(), 100, 0, 100),
            ("gpt-5.6-sol".into(), 500, 0, 500),
            ("gpt-5.6-terra".into(), 200, 800, 1000),
        ]
    );
}

#[test]
fn projects_are_merged_case_insensitively_and_keep_a_readable_name() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let names: Vec<String> = store.projects().unwrap().into_iter().map(|p| p.name).collect();
    // "C:\Projects\demo-app" and "c:/projects/demo-app" are one project
    assert_eq!(names, vec!["Cowork".to_string(), "demo-app".to_string()]);
}

#[test]
fn second_run_is_a_no_op() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let rep = run(&mut store, &m.env);
    assert_eq!(rep.files_read, 0);
    assert_eq!(rep.new_events(), 0);
    assert_eq!(rep.limits_after, rep.limits_before);
}

#[test]
fn appended_lines_are_read_incrementally_and_partial_lines_wait() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let file = m.home.join(".claude/projects/C--Projects-demo-app/session-b.jsonl");
    let line = r#"{"type":"assistant","sessionId":"s-b","uuid":"u-9","timestamp":"2026-09-02T11:00:00.000Z","cwd":"c:/projects/demo-app","message":{"id":"msg_test_b2","model":"claude-opus-5-5","usage":{"input_tokens":1,"output_tokens":9}}}"#;

    // write without the trailing newline: still being written → not consumed
    let (head, tail) = line.split_at(40);
    let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
    f.write_all(head.as_bytes()).unwrap();
    f.flush().unwrap();
    let rep = run(&mut store, &m.env);
    assert_eq!(rep.new_events(), 0);

    f.write_all(tail.as_bytes()).unwrap();
    f.write_all(b"\n").unwrap();
    f.flush().unwrap();
    let rep = run(&mut store, &m.env);
    assert_eq!(rep.files_read, 1);
    assert_eq!(rep.new_events(), 1);
    assert!(rep.warnings.is_empty(), "{:?}", rep.warnings);
    assert_eq!(sums(&store, "claude_code:claude_code_jsonl").output, 249);
}

#[test]
fn archiving_a_codex_rollout_does_not_double_count() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let before = sums(&store, "codex");
    let from = m.home.join(".codex/sessions/2026/09/01").join(CODEX_MAIN);
    let to = m.home.join(".codex/archived_sessions/2026/09/01").join(CODEX_MAIN);
    fs::rename(&from, &to).unwrap();
    let rep = run(&mut store, &m.env);
    assert_eq!(rep.files_read, 1); // new path is read …
    assert_eq!(sums(&store, "codex"), before); // … but nothing is added
}

#[test]
fn deleted_source_logs_stay_in_the_archive() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    fs::remove_dir_all(m.home.join(".claude")).unwrap();
    run(&mut store, &m.env);
    assert_eq!(sums(&store, "claude_code:claude_code_jsonl").events, 3);
}

#[test]
fn msix_alias_of_the_same_file_is_read_once() {
    let m = machine();
    // MSIX exposes %APPDATA%\Claude also under the package's LocalCache; simulate with a hard link.
    let local = m.env.local.clone().unwrap();
    let pkg = local.join("Packages").join("Claude_test123").join("LocalCache").join("Roaming").join("Claude");
    fs::create_dir_all(&pkg).unwrap();
    fs::hard_link(m.roaming.join("Claude").join("plan-usage-history.json"), pkg.join("plan-usage-history.json")).unwrap();
    let files = enumerate_files(&m.env, &ExtraPaths::default(), &all_sources());
    assert_eq!(files.iter().filter(|f| f.source == SourceId::ClaudeDesktop).count(), 1);
}

#[test]
fn claude_config_dir_overrides_the_default_location() {
    let m = machine();
    let alt = m.home.join("alt-claude");
    put("claude/session-b.jsonl", &alt.join("projects").join("x").join("only.jsonl"));
    let env = Env { claude_config_dir: Some(alt.to_string_lossy().into_owned()), ..m.env.clone() };
    let files = enumerate_files(&env, &ExtraPaths::default(), &[SourceId::ClaudeCode].into_iter().collect());
    assert_eq!(files.len(), 1);
    assert!(files[0].path.ends_with("only.jsonl"));
}

#[test]
fn user_added_paths_are_scanned() {
    let m = machine();
    let extra_dir = m.home.join("portable-codex");
    put(&format!("codex/{CODEX_SUB}"), &extra_dir.join("sessions").join("other").join(CODEX_SUB.replace("0002", "0003")));
    let extra = ExtraPaths { codex_homes: vec![extra_dir], ..Default::default() };
    let files = enumerate_files(&m.env, &extra, &[SourceId::Codex].into_iter().collect());
    assert_eq!(files.len(), 3);
}

#[test]
fn disabled_sources_are_not_read() {
    let m = machine();
    let files = enumerate_files(&m.env, &ExtraPaths::default(), &[SourceId::Codex].into_iter().collect());
    assert!(files.iter().all(|f| f.source == SourceId::Codex));
}

#[test]
fn a_machine_without_any_ai_tool_works() {
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        home: Some(dir.path().join("home")),
        roaming: Some(dir.path().join("roaming")),
        local: Some(dir.path().join("local")),
        ..Default::default()
    };
    assert!(detect(&env, &ExtraPaths::default()).iter().all(|s| !s.found && s.file_count == 0));
    let mut store = Store::open_in_memory().unwrap();
    let rep = run(&mut store, &env);
    assert_eq!((rep.files_seen, rep.new_events()), (0, 0));
    // and with no environment at all
    assert!(enumerate_files(&Env::default(), &ExtraPaths::default(), &all_sources()).is_empty());
}

#[test]
fn garbage_and_unknown_formats_warn_instead_of_failing() {
    let m = machine();
    let p = m.home.join(".claude/projects/junk/garbage.jsonl");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut s = String::new();
    for i in 0..25 {
        s.push_str(&format!("{{\"no_type_here\":{i}}}\n"));
    }
    s.push_str("\u{0}\u{1}binary?\n");
    fs::write(&p, s).unwrap();
    fs::write(m.roaming.join("Claude").join("plan-usage-history.json"), r#"{"version":99,"other":[]}"#).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    let rep = run(&mut store, &m.env);
    assert!(rep.errors.is_empty());
    assert!(rep.warnings.iter().any(|(_, w)| w.contains("unrecognised format")));
    assert!(rep.warnings.iter().any(|(_, w)| w.contains("unknown plan-usage-history version")));
    assert!(!store.files_with_warnings().unwrap().is_empty());
}

#[test]
fn archive_survives_reopen_and_backup_and_wipe() {
    let m = machine();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tracker.db");
    {
        let mut store = Store::open(&db).unwrap();
        run(&mut store, &m.env);
        store.backup_to(&dir.path().join("backup.db")).unwrap();
    }
    let store = Store::open(&db).unwrap();
    assert_eq!(store.event_count().unwrap(), 9);
    let backup = Store::open(&dir.path().join("backup.db")).unwrap();
    assert_eq!(backup.event_count().unwrap(), 9);
    store.wipe_data().unwrap();
    assert_eq!(store.event_count().unwrap(), 0);
    assert_eq!(store.limit_count().unwrap(), 0);
}

#[test]
fn a_session_that_changes_directory_stays_one_project() {
    let m = machine();
    let p = m.home.join(".claude/projects/C--Work-site/s.jsonl");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let line = |id: &str, cwd: &str| {
        format!(
            r#"{{"type":"assistant","sessionId":"s-cd","timestamp":"2026-09-05T10:00:00.000Z","cwd":"{cwd}","message":{{"id":"{id}","model":"claude-sonnet-5","usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
        )
    };
    fs::write(&p, format!("{}\n{}\n", line("msg_cd1", "C:/Work/Site"), line("msg_cd2", "C:/Work/Site/src/ui"))).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let names: Vec<(String, String)> = store.projects().unwrap().into_iter().map(|p| (p.name, p.path)).collect();
    assert!(names.contains(&("Site".to_string(), "C:/Work/Site".to_string())), "{names:?}");
    assert!(!names.iter().any(|(n, _)| n == "ui"));
}

#[test]
fn a_pricing_modifier_on_a_later_copy_of_a_message_is_kept() {
    let m = machine();
    let p = m.home.join(".claude/projects/C--Work-fast/f.jsonl");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    // the first streaming copy has no speed; only the final copy says "fast"
    let first = r#"{"type":"assistant","sessionId":"s-f","timestamp":"2026-09-05T10:00:00.000Z","cwd":"C:/Work/Fast","message":{"id":"msg_f1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":1}}}"#;
    let last = r#"{"type":"assistant","sessionId":"s-f","timestamp":"2026-09-05T10:00:01.000Z","cwd":"C:/Work/Fast","message":{"id":"msg_f1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":50,"speed":"fast","service_tier":"standard","inference_geo":"us"}}}"#;
    fs::write(&p, format!("{first}\n{last}\n")).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let row: (String, Option<String>, i64) = store
        .conn()
        .query_row("SELECT speed, inference_geo, output FROM usage_event WHERE key = 'cc:msg_f1'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(row, ("fast".to_string(), Some("us".to_string()), 50));
}

#[test]
fn a_subagent_started_in_a_sub_directory_belongs_to_the_session_project() {
    let m = machine();
    let dir = m.home.join(".claude/projects/C--Work-Site");
    let sub = dir.join("s-sub/subagents/agent-1.jsonl");
    fs::create_dir_all(sub.parent().unwrap()).unwrap();
    let line = |id: &str, cwd: &str| {
        format!(
            r#"{{"type":"assistant","sessionId":"s-sub","timestamp":"2026-09-05T10:00:00.000Z","cwd":"{cwd}","message":{{"id":"{id}","model":"claude-sonnet-5","usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#
        )
    };
    fs::write(dir.join("s-sub.jsonl"), format!("{}\n", line("msg_main", "C:/Work/Site"))).unwrap();
    // the agent was launched while the shell sat in a sub-directory
    fs::write(&sub, format!("{}\n", line("msg_agent", "C:/Work/Site/ui"))).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    let names: Vec<String> = store.projects().unwrap().into_iter().map(|p| p.name).collect();
    assert!(names.contains(&"Site".to_string()), "{names:?}");
    assert!(!names.contains(&"ui".to_string()), "{names:?}");
    let n: i64 = store.conn().query_row("SELECT count(*) FROM usage_event WHERE key = 'cc:msg_agent' AND project_id IS NOT NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1, "the subagent transcript is read");
}

// ---------------------------------------------------------------- branches, subagents, tools

fn claude_line(kind: &str, session: &str, id: &str, extra: &str, content: &str) -> String {
    let message = if kind == "assistant" {
        format!(r#"{{"id":"{id}","model":"claude-sonnet-5","content":{content},"usage":{{"input_tokens":1,"output_tokens":1}}}}"#)
    } else {
        format!(r#"{{"role":"user","content":{content}}}"#)
    };
    format!(r#"{{"type":"{kind}","sessionId":"{session}","timestamp":"2026-09-05T10:00:00.000Z","cwd":"C:/Work/Repo"{extra},"message":{message}}}"#)
}

#[test]
fn claude_branch_subagent_and_tool_calls_are_recorded_without_content() {
    let m = machine();
    let dir = m.home.join(".claude/projects/C--Work-Repo");
    fs::create_dir_all(dir.join("s-t/subagents")).unwrap();
    let branch = r#","gitBranch":"feature/login""#;
    let main = [
        claude_line("assistant", "s-t", "msg_t1", branch, r#"[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"secret"}}]"#),
        claude_line("user", "s-t", "", branch, r#"[{"type":"tool_result","tool_use_id":"toolu_1","is_error":true,"content":"secret"}]"#),
        claude_line("assistant", "s-t", "msg_t2", branch, r#"[{"type":"tool_use","id":"toolu_2","name":"mcp__browser__navigate","input":{}}]"#),
        claude_line("user", "s-t", "", branch, r#"[{"type":"tool_result","tool_use_id":"toolu_2","content":"ok"}]"#),
        // outside a repository the branch is empty
        claude_line("assistant", "s-t", "msg_t3", r#","gitBranch":"""#, "[]"),
    ];
    fs::write(dir.join("s-t.jsonl"), main.join("\n") + "\n").unwrap();
    // a resumed session copies the first call and its result
    fs::write(dir.join("s-copy.jsonl"), format!("{}\n{}\n", main[0].replace("\"s-t\"", "\"s-copy\""), main[1])).unwrap();
    let sub = claude_line(
        "assistant",
        "s-t",
        "msg_t4",
        r#","gitBranch":"feature/login","isSidechain":true,"agentId":"agent-7","attributionAgent":"Explore""#,
        r#"[{"type":"tool_use","id":"toolu_3","name":"Grep","input":{}}]"#,
    );
    fs::write(dir.join("s-t/subagents/agent-7.jsonl"), sub + "\n").unwrap();

    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    type Row = (Option<String>, Option<String>, Option<String>, Option<String>);
    let row = |key: &str| -> Row {
        store
            .conn()
            .query_row("SELECT branch, agent, thread_id, session_id FROM usage_event WHERE key = ?1", [key], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap()
    };
    assert_eq!(row("cc:msg_t2"), (Some("feature/login".into()), None, None, Some("s-t".into())));
    assert_eq!(row("cc:msg_t3").0, None, "no branch outside a repository");
    assert_eq!(row("cc:msg_t4"), (Some("feature/login".into()), Some("Explore".into()), Some("agent-7".into()), Some("s-t".into())));

    let calls: Vec<(String, String, Option<String>, Option<i64>)> = {
        let mut st = store.conn().prepare("SELECT key, name, agent, failed FROM tool_call ORDER BY key").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(
        calls,
        vec![
            ("cct:toolu_1".into(), "Bash".into(), None, Some(1)),
            ("cct:toolu_2".into(), "mcp__browser__navigate".into(), None, Some(0)),
            ("cct:toolu_3".into(), "Grep".into(), Some("Explore".into()), None),
        ],
        "one row per call however often it was copied; outcome from tool_result"
    );
    // nothing of the call's input or output is stored anywhere
    let dump: String = store
        .conn()
        .query_row("SELECT group_concat(key || name || COALESCE(agent,'') || COALESCE(session_id,''), '|') FROM tool_call", [], |r| r.get(0))
        .unwrap();
    assert!(!dump.contains("secret"));
    let projects: i64 = store.conn().query_row("SELECT count(*) FROM tool_call WHERE project_id IS NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(projects, 0, "tool calls belong to the session's project");
}

#[test]
fn codex_subagents_count_toward_their_parent_and_actions_are_recorded() {
    let m = machine();
    let dir = m.home.join(".codex/sessions/2026/09/05");
    fs::create_dir_all(&dir).unwrap();
    let name = "rollout-2026-09-05T10-00-00-0199aaaa-bbbb-7ccc-8ddd-eeeeffff0009.jsonl";
    let lines = [
        r#"{"timestamp":"2026-09-05T10:00:00.000Z","type":"session_meta","payload":{"id":"0199aaaa-bbbb-7ccc-8ddd-eeeeffff0009","cwd":"C:\\Work\\Repo","originator":"codex_cli_rs","source":"cli","git":{"commit_hash":"abc","branch":"main","repository_url":"https://example.invalid/r.git"}}}"#,
        r#"{"timestamp":"2026-09-05T10:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
        r#"{"timestamp":"2026-09-05T10:00:02.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":1},"last_token_usage":{"input_tokens":10,"output_tokens":1}}}}"#,
        r#"{"timestamp":"2026-09-05T10:00:03.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"i1","command":["secret"],"status":"failed","exit_code":1}}}"#,
        r#"{"timestamp":"2026-09-05T10:00:04.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","id":"i2","status":"completed","changes":{}}}}"#,
        r#"{"timestamp":"2026-09-05T10:00:05.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"McpToolCall","id":"i3","server":"node_repl","tool":"js","status":"completed","result":{"isError":false}}}}"#,
        r#"{"timestamp":"2026-09-05T10:00:06.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"Extension","id":"i4","kind":"web.search","query":"secret"}}}"#,
        r#"{"timestamp":"2026-09-05T10:00:07.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"Reasoning","id":"i5","summary_text":["secret"]}}}"#,
    ];
    fs::write(dir.join(name), lines.join("\n") + "\n").unwrap();

    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    // the fixture's guardian rollout counts toward its parent session
    let (session, agent, thread): (String, Option<String>, Option<String>) = store
        .conn()
        .query_row(
            "SELECT session_id, agent, thread_id FROM usage_event WHERE key LIKE 'cx:0199aaaa-bbbb-7ccc-8ddd-eeeeffff0002:%'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(session, "0199aaaa-bbbb-7ccc-8ddd-eeeeffff0001");
    assert_eq!(agent.as_deref(), Some("guardian"));
    assert_eq!(thread.as_deref(), Some("0199aaaa-bbbb-7ccc-8ddd-eeeeffff0002"));
    let branch: Option<String> = store
        .conn()
        .query_row("SELECT branch FROM usage_event WHERE key LIKE 'cx:0199aaaa-bbbb-7ccc-8ddd-eeeeffff0009:%'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(branch.as_deref(), Some("main"));

    let calls: Vec<(String, Option<i64>)> = {
        let mut st = store.conn().prepare("SELECT name, failed FROM tool_call WHERE tool = 'codex' ORDER BY ts_ms").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(
        calls,
        vec![("shell".into(), Some(1)), ("apply_patch".into(), Some(0)), ("mcp__node_repl__js".into(), Some(0)), ("web_search".into(), None)]
    );
}

#[test]
fn rows_read_before_this_version_learn_their_subagent_session_on_the_next_read() {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    // what an older version stored: the subagent's own id as the session, no agent or thread
    store
        .conn()
        .execute(
            "UPDATE usage_event SET session_id = '0199aaaa-bbbb-7ccc-8ddd-eeeeffff0002', agent = NULL, thread_id = NULL, branch = NULL
             WHERE key LIKE 'cx:0199aaaa-bbbb-7ccc-8ddd-eeeeffff0002:%'",
            [],
        )
        .unwrap();
    store.conn().execute("DELETE FROM file_checkpoint", []).unwrap();
    run(&mut store, &m.env);
    let (session, agent): (String, Option<String>) = store
        .conn()
        .query_row("SELECT session_id, agent FROM usage_event WHERE key LIKE 'cx:0199aaaa-bbbb-7ccc-8ddd-eeeeffff0002:%'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((session.as_str(), agent.as_deref()), ("0199aaaa-bbbb-7ccc-8ddd-eeeeffff0001", Some("guardian")));
    assert_eq!(sums(&store, "codex").events, 5, "re-reading adds nothing");
}
