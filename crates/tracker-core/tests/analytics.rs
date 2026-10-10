mod common;

use chrono::{FixedOffset, TimeZone};
use common::*;
use tracker_core::analytics::{limits_view, period_range, report, Filter, LimitState, Period, Range, Threshold};
use tracker_core::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool, UsageEvent};
use tracker_core::pricing::PriceBook;
use tracker_core::store::Store;

fn tz() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).unwrap()
}

fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
}

fn loaded() -> (Machine, Store) {
    let m = machine();
    let mut store = Store::open_in_memory().unwrap();
    run(&mut store, &m.env);
    (m, store)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn all_time_report_prices_every_known_model_and_flags_unknown_ones() {
    let (_m, store) = loaded();
    let book = PriceBook::default_book();
    let now = ms(2026, 9, 3, 13, 0);
    let range = period_range(Period::All, &tz(), now, store.first_event_ms().unwrap());
    let r = report(&store, &book, range, &Filter::default(), &tz()).unwrap();

    assert_eq!(r.totals.events, 9);
    // hand-computed from the fixtures and bundled prices (see fixture comments in ingest.rs)
    assert!(close(r.totals.cost_usd, 0.0404965), "{}", r.totals.cost_usd);
    assert_eq!(r.unpriced_models, vec!["codex-auto-review".to_string()]);
    assert_eq!(r.totals.unpriced_tokens, 110);
    assert_eq!(r.totals.unpriced_events, 1);
    // net cache saving: reads at (input − read) minus write premiums, per model (hand-computed)
    assert!(close(r.totals.cache_savings_usd, 0.0197275), "{}", r.totals.cache_savings_usd);
    let day1 = &r.daily[0];
    assert_eq!(day1.cache_read, 2000 + 3000 + 600 + 1400 + 800);
    assert_eq!(day1.prompt_tokens, day1.cache_read + 5 + 3 + 400 + 500 + 100 + 500 + 200 + 1000 + 200 + 100);

    // local days 2026-09-01 … 2026-09-03 (UTC+3)
    let days: Vec<&str> = r.daily.iter().map(|d| d.date.as_str()).collect();
    assert_eq!(days, vec!["2026-09-01", "2026-09-02", "2026-09-03"]);
    assert_eq!(r.active_days, 3);
    assert_eq!(r.peak_day.as_ref().unwrap().date, "2026-09-01");
    assert_eq!(r.daily.iter().map(|d| d.events).sum::<u64>(), 9);

    let tool_sum: u64 = r.by_tool.iter().map(|g| g.totals.total_tokens).sum();
    assert_eq!(tool_sum, r.totals.total_tokens);
    let proj: Vec<&str> = r.by_project.iter().map(|g| g.label.as_str()).collect();
    assert!(proj.contains(&"demo-app") && proj.contains(&"Cowork"));
    assert_eq!(r.by_accuracy.get("exact"), Some(&9));
    assert_eq!(r.heatmap.iter().flatten().sum::<u64>(), r.totals.total_tokens);

    // reuse counts Claude and the Codex models whose requests log cache writes
    let logs_writes = |g: &&tracker_core::analytics::Group| g.key.starts_with("claude") || g.totals.tokens.cache_write > 0;
    let counted: u64 = r.by_model.iter().filter(logs_writes).map(|g| g.totals.tokens.cache_read).sum();
    let skipped: u64 = r.by_model.iter().filter(|g| !logs_writes(g)).map(|g| g.totals.tokens.cache_read).sum();
    assert!(skipped > 0);
    assert_eq!(r.totals.cache_read_with_writes, counted);
}

#[test]
fn filters_by_tool_project_and_model() {
    let (_m, store) = loaded();
    let book = PriceBook::default_book();
    let range = Range { from_ms: 0, to_ms: i64::MAX / 2 };
    let codex = report(&store, &book, range, &Filter { tools: vec![Tool::Codex], ..Default::default() }, &tz()).unwrap();
    assert_eq!(codex.totals.events, 5);

    let demo = store.projects().unwrap().into_iter().find(|p| p.name == "demo-app").unwrap();
    let only_demo = report(&store, &book, range, &Filter { projects: vec![demo.id], ..Default::default() }, &tz()).unwrap();
    assert_eq!(only_demo.totals.events, 8); // everything except the Cowork session

    let sonnet = report(&store, &book, range, &Filter { models: vec!["claude-sonnet-5".into()], ..Default::default() }, &tz()).unwrap();
    assert_eq!(sonnet.totals.events, 1);
    assert_eq!(sonnet.totals.tokens, Tokens { input: 3, cache_read: 3000, cache_write: 200, cache_write_1h: 0, output: 50, reasoning: 0 });
}

#[test]
fn trend_compares_with_the_previous_period() {
    let (_m, store) = loaded();
    let book = PriceBook::default_book();
    // "today" = 2026-09-02 local; previous = 2026-09-01 local
    let range = period_range(Period::Today, &tz(), ms(2026, 9, 2, 12, 0), None);
    let r = report(&store, &book, range, &Filter::default(), &tz()).unwrap();
    assert_eq!(r.totals.events, 1);
    assert_eq!(r.previous.events, 7);
}

#[test]
fn hidden_projects_keep_their_data_but_are_flagged() {
    let (_m, store) = loaded();
    let book = PriceBook::default_book();
    let demo = store.projects().unwrap().into_iter().find(|p| p.name == "demo-app").unwrap();
    store.set_project_hidden(demo.id, true).unwrap();
    let r = report(&store, &book, Range { from_ms: 0, to_ms: i64::MAX / 2 }, &Filter::default(), &tz()).unwrap();
    let g = r.by_project.iter().find(|g| g.key == demo.id.to_string()).unwrap();
    assert!(g.hidden);
    assert_eq!(r.totals.events, 9);
}

fn ev(key: &str, ts: i64, tool: Tool, model: &str, project: &str, input: u64, output: u64) -> UsageEvent {
    UsageEvent {
        key: key.into(),
        ts_ms: ts,
        tool,
        client: None,
        model: model.into(),
        project_path: Some(project.into()),
        session_id: None,
        tokens: Tokens { input, output, ..Default::default() },
        request_input: input,
        web_search_requests: 0,
        speed: None,
        service_tier: None,
        inference_geo: None,
        request_id: None,
        accuracy: Accuracy::Exact,
        source: "test".into(),
        branch: None,
        agent: None,
        thread_id: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn snap(ts: i64, provider: Provider, tool: Tool, window: &str, used: f64, resets_at: Option<i64>, plan: Option<&str>, source: &str) -> LimitSnapshot {
    LimitSnapshot {
        ts_ms: ts,
        provider,
        tool,
        account: None,
        limit_id: if provider == Provider::OpenAI { "codex".into() } else { String::new() },
        window: window.into(),
        used_pct: Some(used),
        resets_at,
        status: None,
        plan: plan.map(Into::into),
        source: source.into(),
        accuracy: Accuracy::Exact,
    }
}

#[test]
fn limit_views_pick_the_newest_reading_and_split_it_across_projects() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    let resets = (now + 2 * 3_600_000) / 1000; // window ends in 2h → started 3h ago
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            // inside the window: A costs 3× B (same model)
            ev("a", now - 3_600_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\a", 3_000, 0),
            ev("b", now - 1_800_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\b", 1_000, 0),
            // before the window: ignored
            ev("old", now - 4 * 3_600_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\a", 99_000, 0),
            // other provider: ignored for the Codex window
            ev("c", now - 600_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\a", 5_000, 0),
        ])
        .unwrap();
        tx.insert_limits(&[
            snap(now - 7_200_000, Provider::OpenAI, Tool::Codex, "five_hour", 20.0, Some(resets), Some("plus"), "codex_rollout"),
            snap(now - 600_000, Provider::OpenAI, Tool::Codex, "five_hour", 40.0, Some(resets), Some("plus"), "codex_rollout"),
            // old monthly window from a previous plan → hidden
            snap(now - 90 * 86_400_000, Provider::OpenAI, Tool::Codex, "thirty_day", 100.0, Some((now - 80 * 86_400_000) / 1000), Some("go"), "codex_rollout"),
            // Claude: the desktop history reading is newer than the Cowork one → wins
            snap(now - 3_600_000, Provider::Anthropic, Tool::ClaudeCode, "five_hour", 47.0, None, None, "cowork_audit"),
            snap(now - 60_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 6.0, None, None, "claude_plan_history"),
            // Claude weekly reading whose reset already passed
            snap(now - 9 * 86_400_000, Provider::Anthropic, Tool::ClaudeCode, "seven_day", 80.0, Some((now - 86_400_000) / 1000), None, "cowork_audit"),
        ])
        .unwrap();
        tx.commit().unwrap();
    }
    let book = PriceBook::default_book();
    let views = limits_view(&store, &book, now, &[]).unwrap();
    let find = |p: Provider, w: &str| views.iter().find(|v| v.provider == p && v.window == w);

    let cx = find(Provider::OpenAI, "five_hour").unwrap();
    assert_eq!(cx.used_pct, Some(40.0));
    assert_eq!(cx.state, LimitState::Fresh);
    assert_eq!(cx.window_start_ms, Some(now - 3 * 3_600_000));
    assert_eq!(cx.window_usage.events, 2);
    assert_eq!(cx.projects.len(), 2);
    assert!((cx.projects[0].share - 0.75).abs() < 1e-9);
    assert!((cx.projects[0].estimated_pct.unwrap() - 30.0).abs() < 1e-9);
    assert!(find(Provider::OpenAI, "thirty_day").is_none());

    let cl = find(Provider::Anthropic, "five_hour").unwrap();
    assert_eq!((cl.used_pct, cl.source.as_str()), (Some(6.0), "claude_plan_history"));
    assert_eq!(cl.window_usage.events, 1);

    let wk = find(Provider::Anthropic, "seven_day").unwrap();
    assert_eq!(wk.state, LimitState::Reset);
    assert!(wk.projects.iter().all(|p| p.estimated_pct.is_none()));
}

#[test]
fn stale_reading_without_reset_time_is_marked_stale() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    {
        let mut tx = store.transaction().unwrap();
        tx.insert_limits(&[snap(now - 6 * 3_600_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 50.0, None, None, "claude_plan_history")])
            .unwrap();
        tx.commit().unwrap();
    }
    let v = limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap();
    assert_eq!(v[0].state, LimitState::Stale);
}

#[test]
fn user_thresholds_only_fill_windows_without_real_readings() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[ev("x", now - 60_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\a", 1_000_000, 0)]).unwrap();
        tx.commit().unwrap();
    }
    let th = [
        Threshold { provider: Provider::Anthropic, window: "five_hour".into(), tokens: None, cost_usd: Some(8.0) },
        Threshold { provider: Provider::OpenAI, window: "five_hour".into(), tokens: Some(1000), cost_usd: None },
    ];
    let v = limits_view(&store, &PriceBook::default_book(), now, &th).unwrap();
    let cl = v.iter().find(|v| v.provider == Provider::Anthropic).unwrap();
    assert_eq!(cl.accuracy, Accuracy::Estimated);
    assert_eq!(cl.source, "user_threshold");
    assert!((cl.used_pct.unwrap() - 25.0).abs() < 1e-9); // $2 of $8
    let cx = v.iter().find(|v| v.provider == Provider::OpenAI).unwrap();
    assert_eq!(cx.used_pct, Some(0.0));
}

#[test]
fn backup_merges_back_without_duplicates() {
    let (_m, store) = loaded();
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("backup.db");
    store.backup_to(&backup).unwrap();

    let mut fresh = Store::open(&dir.path().join("fresh.db")).unwrap();
    let (events, limits) = fresh.merge_from(&backup).unwrap();
    assert_eq!((events, limits), (9, 12));
    assert_eq!(fresh.projects().unwrap().len(), 2);
    // merging the same backup again changes nothing
    fresh.merge_from(&backup).unwrap();
    assert_eq!(fresh.event_count().unwrap(), 9);
    assert_eq!(fresh.limit_count().unwrap(), 12);
    // a non-tracker file is rejected
    std::fs::write(dir.path().join("junk.db"), b"").unwrap();
    assert!(fresh.merge_from(&dir.path().join("junk.db")).is_err());
    // the live file itself, under another spelling of its path, is refused before it is attached
    let other_spelling = dir.path().join(".").join("fresh.db");
    assert!(matches!(fresh.merge_from(&other_spelling), Err(tracker_core::rusqlite::Error::InvalidPath(_))));
}

#[test]
fn a_live_reading_with_an_impossible_reset_time_is_kept_without_it() {
    let mut store = Store::open_in_memory().unwrap();
    let now = 1_791_000_000_000;
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[snap(now - 60_000, Provider::OpenAI, Tool::Codex, "five_hour", 30.0, Some(i64::MAX), None, "codex_rollout")]).unwrap();
    tx.commit().unwrap();
    let stored: Option<i64> = store.conn().query_row("SELECT resets_at FROM limit_snapshot", [], |r| r.get(0)).unwrap();
    assert_eq!(stored, None);
    // a row an older version stored is read safely too
    store.conn().execute("UPDATE limit_snapshot SET resets_at = ?1", [i64::MAX]).unwrap();
    let v = limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap();
    let cx = v.iter().find(|l| l.provider == Provider::OpenAI).unwrap();
    assert_eq!((cx.used_pct, cx.resets_at), (Some(30.0), None));
}

#[test]
fn a_backup_with_impossible_times_brings_only_its_sound_rows() {
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("odd.db");
    {
        let mut odd = Store::open(&backup).unwrap();
        let mut tx = odd.transaction().unwrap();
        tx.insert_limits(&[
            snap(1_790_000_000_000, Provider::Anthropic, Tool::ClaudeCode, "five_hour", 20.0, Some(1_790_010_000), None, "test"),
            snap(1_790_000_100_000, Provider::Anthropic, Tool::ClaudeCode, "five_hour", 30.0, Some(i64::MAX), None, "test"),
            snap(-5, Provider::Anthropic, Tool::ClaudeCode, "five_hour", 30.0, None, None, "test"),
        ])
        .unwrap();
        tx.commit().unwrap();
        // insert_limits already drops such a time, so the damaged value is written directly
        odd.conn().execute("UPDATE limit_snapshot SET resets_at = ?1 WHERE used_pct = 30 AND ts_ms > 0", [i64::MAX]).unwrap();
    }
    let mut fresh = Store::open(&dir.path().join("fresh.db")).unwrap();
    assert_eq!(fresh.merge_from(&backup).unwrap(), (0, 1));
}

#[test]
fn an_impossible_window_length_is_dropped_on_every_path_and_never_breaks_the_limits() {
    let now = chrono::Utc::now().timestamp_millis();
    let reset = Some(now / 1000 + 3600);
    let book = PriceBook::default_book();
    let reply = |mins: i64| serde_json::json!({"rateLimits": {"primary": {"usedPercent": 40.0, "windowDurationMins": mins, "resetsAt": reset}}});
    for mins in [-1, 0, i64::MAX] {
        assert!(tracker_core::capture::codex_limits::parse_result(&reply(mins), now).is_empty(), "{mins}");
    }
    assert_eq!(tracker_core::capture::codex_limits::parse_result(&reply(90), now)[0].window, "90m");

    let mut store = Store::open_in_memory().unwrap();
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[snap(now - 60_000, Provider::OpenAI, Tool::Codex, "-1m", 40.0, reset, None, "codex_app_server")]).unwrap();
    tx.commit().unwrap();
    assert!(limits_view(&store, &book, now, &[]).unwrap().is_empty(), "the live path stores nothing");

    // rows an older version stored or a backup brings are read without a duration
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("odd.db");
    Store::open(&backup).unwrap();
    for (i, w) in ["-1m", "0m", "-60000m", "99999999999m", "90m"].iter().enumerate() {
        let conn = tracker_core::rusqlite::Connection::open(&backup).unwrap();
        conn.execute(
            "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status, plan, source, accuracy)
             VALUES(?1, 'openai', 'codex', '', 'codex', ?2, 40, ?3, NULL, NULL, 'codex_app_server', 'captured')",
            tracker_core::rusqlite::params![now - 60_000 - i as i64, w, reset],
        )
        .unwrap();
        store.conn().execute(
            "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status, plan, source, accuracy)
             VALUES(?1, 'openai', 'codex', '', 'codex', ?2, 40, ?3, NULL, NULL, 'codex_app_server', 'captured')",
            tracker_core::rusqlite::params![now - 60_000 - i as i64, w, reset],
        )
        .unwrap();
    }
    let views = limits_view(&store, &book, now, &[]).unwrap();
    assert!(views.iter().all(|v| v.forecast.is_none() || v.window == "90m"));
    let mut fresh = Store::open(&dir.path().join("fresh.db")).unwrap();
    assert_eq!(fresh.merge_from(&backup).unwrap(), (0, 1), "only the sound window is imported");
    limits_view(&fresh, &book, now, &[]).unwrap();
}

#[test]
fn a_reading_dated_in_the_future_never_hides_the_current_one() {
    let now = chrono::Utc::now().timestamp_millis();
    let book = PriceBook::default_book();
    let dir = tempfile::tempdir().unwrap();
    let history = dir.path().join("plan-usage-history.json");
    std::fs::write(&history, format!(r#"{{"version":2,"samples":[{{"t":{},"u":{{"fh":95}}}},{{"t":{},"u":{{"fh":5}}}}]}}"#, i64::MAX, now - 60_000)).unwrap();
    let out = tracker_core::sources::claude_plan::parse_file(&history).unwrap();
    assert_eq!((out.limits.len(), out.warning_count), (1, 1));

    let mut store = Store::open_in_memory().unwrap();
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[
        snap(i64::MAX, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 95.0, None, None, "claude_plan_history"),
        snap(now - 60_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 5.0, None, None, "claude_plan_history"),
    ])
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(store.conn().query_row("SELECT COUNT(*) FROM limit_snapshot", [], |r| r.get::<_, i64>(0)).unwrap(), 1);

    // a row an older version stored is passed over, and an hour of clock drift is still accepted
    store
        .conn()
        .execute(
            "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status, plan, source, accuracy)
             VALUES(?1, 'anthropic', 'claude_desktop', '', '', 'five_hour', 95, NULL, NULL, NULL, 'claude_plan_history', 'exact')",
            [i64::MAX],
        )
        .unwrap();
    let v = &limits_view(&store, &book, now, &[]).unwrap()[0];
    assert_eq!((v.used_pct, v.state), (Some(5.0), LimitState::Fresh));
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[snap(now + 3_600_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 7.0, None, None, "claude_plan_history")]).unwrap();
    tx.commit().unwrap();
    assert_eq!(limits_view(&store, &book, now, &[]).unwrap()[0].used_pct, Some(7.0));
}

#[test]
fn a_stored_reading_dated_before_1970_is_never_shown_as_current() {
    let now = chrono::Utc::now().timestamp_millis();
    let book = PriceBook::default_book();
    let mut store = Store::open_in_memory().unwrap();
    // rows an older version could store; written directly, past today's insert checks
    let legacy = |store: &Store, ts: i64, used: f64| {
        store
            .conn()
            .execute(
                "INSERT INTO limit_snapshot(ts_ms, provider, tool, account, limit_id, window, used_pct, resets_at, status, plan, source, accuracy)
                 VALUES(?1, 'anthropic', 'claude_desktop', '', '', 'five_hour', ?2, NULL, NULL, NULL, 'claude_plan_history', 'exact')",
                tracker_core::rusqlite::params![ts, used],
            )
            .unwrap();
    };
    legacy(&store, i64::MIN, 95.0);
    legacy(&store, -1, 90.0);
    assert!(limits_view(&store, &book, now, &[]).unwrap().is_empty());

    // the first moment of 1970 is a valid, very old reading: it shows as stale, never current
    legacy(&store, 0, 80.0);
    let v = &limits_view(&store, &book, now, &[]).unwrap()[0];
    assert_eq!((v.used_pct, v.state), (Some(80.0), LimitState::Stale));

    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[snap(now - 60_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 5.0, None, None, "claude_plan_history")]).unwrap();
    tx.commit().unwrap();
    let v = &limits_view(&store, &book, now, &[]).unwrap()[0];
    assert_eq!((v.used_pct, v.state), (Some(5.0), LimitState::Fresh));
}

#[test]
fn a_web_search_count_past_u32_is_never_imported_or_kept() {
    let dir = tempfile::tempdir().unwrap();
    let event = |conn: &tracker_core::rusqlite::Connection, key: &str, web: i64| {
        conn.execute(
            "INSERT INTO usage_event(key, ts_ms, tool, model, input, output, web_search, accuracy, source)
             VALUES(?1, 1790000000000, 'claude_code', 'claude-opus-5-5', 10, 1, ?2, 'exact', 'test')",
            tracker_core::rusqlite::params![key, web],
        )
        .unwrap();
    };
    let backup = dir.path().join("odd.db");
    {
        let odd = Store::open(&backup).unwrap();
        for (key, web) in [("max", u32::MAX as i64), ("over1", u32::MAX as i64 + 1), ("over2", u32::MAX as i64 + 2), ("neg", -1)] {
            event(odd.conn(), key, web);
        }
    }
    let mut fresh = Store::open(&dir.path().join("fresh.db")).unwrap();
    assert_eq!(fresh.merge_from(&backup).unwrap(), (1, 0), "only the count that fits u32 is imported");
    let rows = fresh.events_between(0, i64::MAX).unwrap();
    assert_eq!(rows.iter().map(|e| e.web_search).collect::<Vec<_>>(), [u32::MAX]);

    // a count an older version imported is dropped when the archive is opened; the row's tokens stay
    let old = dir.path().join("old.db");
    {
        let s = Store::open(&old).unwrap();
        event(s.conn(), "kept", 3);
        event(s.conn(), "wrapped", u32::MAX as i64 + 2);
        s.conn().execute_batch("PRAGMA user_version = 10;").unwrap();
    }
    let reopened = Store::open(&old).unwrap();
    let mut rows: Vec<(u32, u64)> = reopened.events_between(0, i64::MAX).unwrap().iter().map(|e| (e.web_search, e.tokens.input)).collect();
    rows.sort();
    assert_eq!(rows, [(0, 10), (3, 10)]);
}

#[test]
fn an_imported_backup_keeps_the_archives_projects_and_brings_their_spelling() {
    let dir = tempfile::tempdir().unwrap();
    let event = |s: &Store, key: &str, project: i64| {
        s.conn()
            .execute(
                "INSERT INTO usage_event(key, ts_ms, tool, model, project_id, input, output, accuracy, source)
                 VALUES(?1, 1790000000000, 'claude_code', 'claude-opus-5-5', ?2, 10, 1, 'exact', 'test')",
                tracker_core::rusqlite::params![key, project],
            )
            .unwrap();
    };
    let project = |s: &Store, path: &str, display: &str| -> i64 {
        s.conn().execute("INSERT INTO project(path, name, display_path) VALUES(?1, 'p', ?2)", [path, display]).unwrap();
        s.conn().last_insert_rowid()
    };
    let backup = dir.path().join("old.db");
    {
        let b = Store::open(&backup).unwrap();
        let wrong = project(&b, "c:\\old\\wrong", "C:\\Old\\Wrong");
        event(&b, "cc:1", wrong);
        let only = project(&b, "c:\\only\\here", "C:\\Only\\Here");
        event(&b, "cc:2", only);
    }
    let main = Store::open(&dir.path().join("main.db")).unwrap();
    let right = project(&main, "/right/project", "/right/project");
    event(&main, "cc:1", right);
    let mut main = main;
    main.merge_from(&backup).unwrap();
    let project_of = |key: &str| -> String {
        main.conn()
            .query_row("SELECT COALESCE(p.display_path, p.path) FROM usage_event e JOIN project p ON p.id = e.project_id WHERE e.key = ?1", [key], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(project_of("cc:1"), "/right/project", "the archive's project wins");
    assert_eq!(project_of("cc:2"), "C:\\Only\\Here", "a project only the backup knows keeps its spelling");
}

#[test]
fn a_reading_without_a_reset_time_takes_the_one_another_source_knows() {
    let now = chrono::Utc::now().timestamp_millis();
    let book = PriceBook::default_book();
    let mut store = Store::open_in_memory().unwrap();
    let reset_s = (now - 10 * 60_000) / 1000;
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[
        // Claude Code's read: 60 %, window ended ten minutes ago
        snap(now - 90 * 60_000, Provider::Anthropic, Tool::ClaudeCode, "five_hour", 60.0, Some(reset_s), None, "claude_usage"),
        // Claude desktop's sample, newer but taken before that reset and without a reset time
        snap(now - 40 * 60_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 80.0, None, None, "claude_plan_history"),
    ])
    .unwrap();
    tx.commit().unwrap();
    let v = limits_view(&store, &book, now, &[]).unwrap().into_iter().find(|l| l.window == "five_hour").unwrap();
    assert_eq!((v.used_pct, v.resets_at, v.state), (Some(80.0), Some(reset_s), LimitState::Reset));

    // a sample taken after that reset is a new window: the old end does not apply
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[snap(now - 5 * 60_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 4.0, None, None, "claude_plan_history")]).unwrap();
    tx.commit().unwrap();
    let v = limits_view(&store, &book, now, &[]).unwrap().into_iter().find(|l| l.window == "five_hour").unwrap();
    assert_eq!((v.used_pct, v.resets_at, v.state), (Some(4.0), None, LimitState::Fresh));
}

#[test]
fn a_failed_export_keeps_the_existing_file_and_the_archive_is_never_a_target() {
    use std::io::Write;
    use tracker_core::export::write_atomically;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("usage.csv");
    std::fs::write(&dest, "earlier export").unwrap();
    let failed: Result<(), String> = write_atomically(&dest, |f| {
        f.write_all(b"half a file").map_err(|e| e.to_string())?;
        Err("disk full".into())
    });
    assert_eq!(failed, Err("disk full".into()));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "earlier export");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temporary file is left behind");
    write_atomically(&dest, |f| f.write_all(b"new export").map_err(|e| e.to_string())).unwrap();
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new export");

    let db = dir.path().join("tracker.db");
    let store = Store::open(&db).unwrap();
    for p in [db.clone(), dir.path().join("tracker.db-wal"), dir.path().join(".").join("tracker.db")] {
        assert!(store.is_live_file(&p), "{}", p.display());
    }
    assert!(!store.is_live_file(&dest));
}

#[test]
fn csv_export_masks_hidden_projects_and_leaves_unpriced_cost_empty() {
    use tracker_core::export::{export, Format, Granularity};
    let (_m, store) = loaded();
    let demo = store.projects().unwrap().into_iter().find(|p| p.name == "demo-app").unwrap();
    store.set_project_hidden(demo.id, true).unwrap();
    let mut buf = Vec::new();
    let range = Range { from_ms: 0, to_ms: i64::MAX / 2 };
    let n = export(&store, &PriceBook::default_book(), range, &Filter::default(), &tz(), Granularity::Events, Format::Csv, false, &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    assert_eq!(n, 9);
    assert_eq!(text.lines().count(), 10);
    assert!(!text.contains("demo-app"));
    assert!(text.contains(&format!("project-{}", demo.id)));
    assert!(text.contains(",Cowork,"));
    let review = text.lines().find(|l| l.contains("codex-auto-review")).unwrap();
    assert!(review.contains(",,exact,"), "{review}");

    let mut daily = Vec::new();
    export(&store, &PriceBook::default_book(), range, &Filter::default(), &tz(), Granularity::Daily, Format::Json, true, &mut daily).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&daily).unwrap();
    let total: u64 = v.as_array().unwrap().iter().map(|r| r["events"].as_u64().unwrap()).sum();
    assert_eq!(total, 9);
    assert!(!String::from_utf8(daily).unwrap().contains("Cowork"));
}

#[test]
fn reading_followed_by_more_usage_is_marked_behind() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    let read_at = now - 3_600_000;
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            // before the reading: already in it
            ev("before", read_at - 60_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\a", 1_000, 0),
            // Codex use says nothing about the Claude window
            ev("cx", now - 1_200_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\a", 1_000, 0),
        ])
        .unwrap();
        tx.insert_limits(&[snap(read_at, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 6.0, None, None, "claude_plan_history")]).unwrap();
        tx.commit().unwrap();
    }
    let book = PriceBook::default_book();
    assert_eq!(limits_view(&store, &book, now, &[]).unwrap()[0].state, LimitState::Fresh);

    {
        let mut tx = store.transaction().unwrap();
        // five minutes after the reading: not in it, however long ago the reading was
        tx.upsert_events(&[ev("soon", read_at + 300_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\a", 1_000_000, 0)]).unwrap();
        tx.commit().unwrap();
    }
    // while the reading is recent a refresh is expected, so it still counts as current
    assert_eq!(limits_view(&store, &book, read_at + 360_000, &[]).unwrap()[0].state, LimitState::Fresh);
    let v = &limits_view(&store, &book, now, &[]).unwrap()[0];
    assert_eq!(v.state, LimitState::Behind);
    // the reading stays available, with what it does not include yet
    assert_eq!(v.used_pct, Some(6.0));
    assert_eq!((v.usage_since.events, v.usage_since.tokens.input), (1, 1_000_000));

    // a newer reading includes that use
    {
        let mut tx = store.transaction().unwrap();
        tx.insert_limits(&[snap(now - 60_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 30.0, None, None, "claude_plan_history")]).unwrap();
        tx.commit().unwrap();
    }
    let v = &limits_view(&store, &book, now, &[]).unwrap()[0];
    assert_eq!((v.state, v.used_pct, v.usage_since.events), (LimitState::Fresh, Some(30.0), 0));
}

#[test]
fn an_old_reading_inside_its_window_is_only_the_last_known_value() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    let day = 86_400_000;
    {
        let mut tx = store.transaction().unwrap();
        // read six days ago, no local use since: the web or another device may have used it
        tx.insert_limits(&[
            snap(now - 6 * day, Provider::Anthropic, Tool::ClaudeCode, "seven_day", 40.0, Some((now + day) / 1000), None, "claude_usage"),
            snap(now - 3 * 3_600_000, Provider::Anthropic, Tool::ClaudeCode, "five_hour", 20.0, Some((now + 3_600_000) / 1000), None, "claude_usage"),
        ])
        .unwrap();
        tx.commit().unwrap();
    }
    let views = limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap();
    let week = views.iter().find(|v| v.window == "seven_day").unwrap();
    assert_eq!((week.state, week.used_pct, week.usage_since.events), (LimitState::Behind, Some(40.0), 0));
    assert!(week.forecast.is_none());
    let five = views.iter().find(|v| v.window == "five_hour").unwrap();
    assert_eq!(five.state, LimitState::Fresh);
    assert!(five.forecast.is_some());
}

#[test]
fn per_model_windows_count_only_their_model_family() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            ev("o", now - 3_600_000, Tool::ClaudeCode, "claude-opus-5", r"C:\p\a", 1_000, 0),
            ev("s", now - 3_000_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\b", 9_000, 0),
            ev("s2", now - 1_200_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\b", 9_000, 0),
        ])
        .unwrap();
        tx.insert_limits(&[snap(now - 1_800_000, Provider::Anthropic, Tool::ClaudeCode, "seven_day_opus", 30.0, Some((now + 86_400_000) / 1000), None, "claude_usage")])
            .unwrap();
        tx.commit().unwrap();
    }
    let v = &limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap()[0];
    // Sonnet use after the reading does not make the Opus window outdated
    assert_eq!(v.state, LimitState::Fresh);
    assert_eq!(v.window_usage.events, 1);
    assert_eq!(v.projects.len(), 1);
    assert!((v.projects[0].share - 1.0).abs() < 1e-9);
}

#[test]
fn unpriced_use_keeps_its_share_of_a_limit() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            ev("a", now - 3_600_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\a", 1_000, 0),
            ev("b", now - 3_000_000, Tool::Codex, "codex-auto-review", r"C:\p\b", 3_000, 0),
        ])
        .unwrap();
        tx.insert_limits(&[snap(now - 60_000, Provider::OpenAI, Tool::Codex, "five_hour", 40.0, Some((now + 3_600_000) / 1000), None, "codex_rollout")])
            .unwrap();
        tx.commit().unwrap();
    }
    let v = &limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap()[0];
    // by tokens, since a cost share would give the unpriced project nothing
    assert_eq!(v.projects.len(), 2);
    assert!((v.projects[0].share - 0.75).abs() < 1e-9);
    assert!((v.projects[0].estimated_pct.unwrap() - 30.0).abs() < 1e-9);
}

#[test]
fn custom_ranges_are_ordered_bounded_and_never_overflow() {
    use chrono::NaiveDate;
    use tracker_core::analytics::MAX_RANGE_DAYS;
    let now = ms(2026, 10, 2, 12, 0);
    let day = 86_400_000;
    let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
    let r = period_range(Period::Custom { from: d(2026, 9, 5), to: d(2026, 9, 5) }, &tz(), now, None);
    assert_eq!(r.to_ms - r.from_ms, day);
    let r = period_range(Period::Custom { from: d(2026, 9, 5), to: d(2026, 9, 1) }, &tz(), now, None);
    assert_eq!(r.to_ms - r.from_ms, 5 * day);
    // far too wide: the newest ten years are kept
    let r = period_range(Period::Custom { from: d(1900, 1, 1), to: d(2026, 9, 30) }, &tz(), now, None);
    assert_eq!((r.to_ms - r.from_ms) / day, MAX_RANGE_DAYS as i64);
    assert_eq!(r.to_ms, period_range(Period::Custom { from: d(2026, 9, 30), to: d(2026, 9, 30) }, &tz(), now, None).to_ms);
    for (a, b) in [(NaiveDate::MIN, NaiveDate::MAX), (NaiveDate::MAX, NaiveDate::MAX), (NaiveDate::MIN, NaiveDate::MIN)] {
        let r = period_range(Period::Custom { from: a, to: b }, &tz(), now, None);
        assert!(r.from_ms < r.to_ms && (r.to_ms - r.from_ms) / day <= MAX_RANGE_DAYS as i64);
        let store = Store::open_in_memory().unwrap();
        let rep = report(&store, &PriceBook::default_book(), r, &Filter::default(), &tz()).unwrap();
        assert!(rep.daily.len() as u64 <= MAX_RANGE_DAYS);
    }
    // an event far in the past does not stretch "all time" without bound
    let r = period_range(Period::All, &tz(), now, Some(ms(1971, 1, 1, 0, 0)));
    assert_eq!((r.to_ms - r.from_ms) / day, MAX_RANGE_DAYS as i64);
}

#[test]
fn cache_reuse_counts_codex_models_that_log_their_writes() {
    let mut store = Store::open_in_memory().unwrap();
    let now = ms(2026, 10, 2, 12, 0);
    let cached = |key: &str, model: &str, read: u64, write: u64| {
        let mut e = ev(key, now - 3_600_000, Tool::Codex, model, r"C:\p\a", 100, 10);
        e.tokens.cache_read = read;
        e.tokens.cache_write = write;
        e
    };
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[cached("w", "gpt-6-sol", 0, 1_000), cached("r", "gpt-6-sol", 4_000, 0), cached("old", "gpt-5.3-codex", 9_000, 0)]).unwrap();
        tx.commit().unwrap();
    }
    let range = Range { from_ms: now - 86_400_000, to_ms: now };
    let r = report(&store, &PriceBook::default_book(), range, &Filter::default(), &tz()).unwrap();
    assert_eq!((r.totals.cache_read_with_writes, r.totals.cache_write_with_reads), (4_000, 1_000));
}

#[test]
fn antigravity_pools_count_only_their_own_models() {
    let now = 1_791_440_000_000;
    let resets = (now + 3_600_000) / 1000;
    let mut store = Store::open_in_memory().unwrap();
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            ev("g1", now - 3_600_000, Tool::Antigravity, "gemini-3.8-flash-n", r"C:\p\a", 4_000, 0),
            ev("g2", now - 1_800_000, Tool::Antigravity, "gemini-3-flash-a", r"C:\p\b", 1_000, 0),
            ev("c1", now - 1_200_000, Tool::Antigravity, "claude-opus-4-6-thinking", r"C:\p\a", 2_000, 0),
            // another provider's model with "gemini" nowhere near Antigravity
            ev("x", now - 600_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\a", 9_000, 0),
        ])
        .unwrap();
        let pool = |id: &str, used: f64| LimitSnapshot {
            limit_id: id.into(),
            ..snap(now - 60_000, Provider::Google, Tool::Antigravity, "five_hour", used, Some(resets), None, "antigravity_cli_usage")
        };
        tx.insert_limits(&[pool("gemini", 30.0), pool("3p", 10.0)]).unwrap();
        tx.commit().unwrap();
    }
    let views = limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap();
    let find = |id: &str| views.iter().find(|v| v.provider == Provider::Google && v.limit_id == id).unwrap();
    assert_eq!((find("gemini").used_pct, find("gemini").window_usage.events), (Some(30.0), 2));
    assert_eq!((find("3p").used_pct, find("3p").window_usage.events), (Some(10.0), 1));
    assert_eq!(find("3p").projects.len(), 1);

    // a budget the user set for Antigravity covers both pools
    let th = [Threshold { provider: Provider::Google, window: "seven_day".into(), tokens: Some(70_000), cost_usd: None }];
    let v = limits_view(&store, &PriceBook::default_book(), now, &th).unwrap();
    let b = v.iter().find(|v| v.provider == Provider::Google && v.window == "seven_day").unwrap();
    assert_eq!((b.window_usage.events, b.used_pct), (3, Some(10.0)));
}
