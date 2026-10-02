//! Calculation layer: costs, breakdowns, daily series and plan-limit views.

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

    // breakdowns add up
    let tool_sum: u64 = r.by_tool.iter().map(|g| g.totals.total_tokens).sum();
    assert_eq!(tool_sum, r.totals.total_tokens);
    let proj: Vec<&str> = r.by_project.iter().map(|g| g.label.as_str()).collect();
    assert!(proj.contains(&"demo-app") && proj.contains(&"Cowork"));
    assert_eq!(r.by_accuracy.get("exact"), Some(&9));
    assert_eq!(r.heatmap.iter().flatten().sum::<u64>(), r.totals.total_tokens);

    // reuse counts only reads from tools that report writes (Codex logs reads, never writes)
    let claude_reads: u64 = r.by_tool.iter().filter(|g| g.key != "codex").map(|g| g.totals.tokens.cache_read).sum();
    let codex_reads: u64 = r.by_tool.iter().filter(|g| g.key == "codex").map(|g| g.totals.tokens.cache_read).sum();
    assert!(codex_reads > 0);
    assert_eq!(r.totals.cache_read_with_writes, claude_reads);
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

// ---------------------------------------------------------------- limits

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
    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            // a minute after the reading: the same request, within the grace period
            ev("same", now - 3_540_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\a", 1_000, 0),
            // Codex use says nothing about the Claude window
            ev("cx", now - 1_200_000, Tool::Codex, "gpt-5.6-terra", r"C:\p\a", 1_000, 0),
        ])
        .unwrap();
        tx.insert_limits(&[snap(now - 3_600_000, Provider::Anthropic, Tool::ClaudeDesktop, "five_hour", 6.0, None, None, "claude_plan_history")])
            .unwrap();
        tx.commit().unwrap();
    }
    let book = PriceBook::default_book();
    assert_eq!(limits_view(&store, &book, now, &[]).unwrap()[0].state, LimitState::Fresh);

    {
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[ev("later", now - 600_000, Tool::ClaudeCode, "claude-sonnet-5", r"C:\p\a", 1_000, 0)]).unwrap();
        tx.commit().unwrap();
    }
    let v = &limits_view(&store, &book, now, &[]).unwrap()[0];
    assert_eq!(v.state, LimitState::Behind);
    // the reading stays available, with what it does not include yet
    assert_eq!(v.used_pct, Some(6.0));
    assert_eq!(v.usage_since.events, 1);
}
