//! Limit history (windows from readings) and the plan advice built on it.

use tracker_core::history::{add_local_usage, build_windows, limit_history, plan_advice, AdviceKind, Reading};
use tracker_core::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool, UsageEvent};
use tracker_core::pricing::PriceBook;
use tracker_core::plans::PlansFile;
use tracker_core::store::Store;

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;
const FIVE: i64 = 5 * HOUR;
/// 2026-10-03 12:00 UTC
const NOW: i64 = 1_791_028_800_000;

fn r(ts: i64, used: f64, resets: Option<i64>) -> Reading {
    Reading { ts_ms: ts, used, resets_ms: resets, plan: None }
}

#[test]
fn readings_naming_almost_the_same_reset_are_one_window() {
    let reset = NOW - 2 * HOUR;
    let w = build_windows(
        &[r(reset - 4 * HOUR, 10.0, Some(reset)), r(reset - 3 * HOUR, 30.0, Some(reset - 1000)), r(reset - 10 * MIN, 42.0, Some(reset))],
        FIVE,
        NOW,
    );
    assert_eq!(w.len(), 1);
    assert_eq!((w[0].peak_pct, w[0].readings, w[0].resets_at_ms, w[0].start_ms), (42.0, 3, Some(reset), Some(reset - FIVE)));
    assert!(w[0].complete, "read 10 minutes before its reset");
    assert!(!w[0].full && !w[0].in_progress);
}

#[test]
fn a_window_not_read_near_its_end_is_incomplete_and_a_full_one_is_always_complete() {
    let a = NOW - 10 * HOUR;
    let b = NOW - 2 * HOUR;
    let w = build_windows(
        &[
            r(a - 4 * HOUR, 20.0, Some(a)), // last read 4 h before its reset
            r(b - 4 * HOUR, 60.0, Some(b)),
            r(b - 3 * HOUR, 100.0, Some(b)), // full 3 h before its reset
        ],
        FIVE,
        NOW,
    );
    assert_eq!(w.len(), 2);
    assert!(!w[0].complete);
    assert!(w[1].full && w[1].complete);
    assert_eq!(w[1].full_minutes, Some(180));
}

#[test]
fn a_window_reset_early_ends_where_the_next_one_begins() {
    // Codex named a new five-hour window an hour into the old one: the old one was reset then
    let a = NOW - 10 * HOUR;
    let b = a + HOUR;
    let w = build_windows(
        &[
            r(a - 5 * HOUR, 30.0, Some(a)),
            r(a - 4 * HOUR - 5 * MIN, 95.0, Some(a)),
            r(a - 4 * HOUR, 2.0, Some(b)),
            r(a - 3 * HOUR, 40.0, Some(b)),
            r(b - 10 * MIN, 61.0, Some(b)),
        ],
        FIVE,
        NOW,
    );
    assert_eq!(w.len(), 2);
    assert_eq!((w[0].resets_at_ms, w[0].end_ms), (Some(a), Some(a - 4 * HOUR)));
    assert!(w[0].complete, "read five minutes before the early reset");
    assert_eq!(w[0].peak_pct, 95.0);
    assert_eq!((w[1].end_ms, w[1].peak_pct), (Some(b), 61.0));
}

#[test]
fn unused_windows_with_a_moving_reset_are_dropped() {
    // Codex names "now + 5 h" as the reset of a window nobody used yet
    let w = build_windows(&[r(NOW - 20 * MIN, 0.0, Some(NOW - 20 * MIN + FIVE)), r(NOW - 15 * MIN, 0.0, Some(NOW - 15 * MIN + FIVE))], FIVE, NOW);
    assert!(w.is_empty());
}

#[test]
fn readings_without_a_reset_join_their_window_or_split_where_the_value_drops() {
    let reset = NOW - HOUR;
    let w = build_windows(
        &[
            // Claude desktop history: no reset time
            r(NOW - 30 * HOUR, 10.0, None),
            r(NOW - 29 * HOUR, 25.0, None),
            r(NOW - 28 * HOUR + 50 * MIN, 31.0, None), // last reading of that window
            r(NOW - 28 * HOUR + 55 * MIN, 0.0, None),  // reset in between: the value dropped
            r(NOW - 27 * HOUR, 5.0, None),
            // inside a window whose reset another source named
            r(reset - 2 * HOUR, 50.0, None),
            r(reset - 30 * MIN, 64.0, Some(reset)),
        ],
        FIVE,
        NOW,
    );
    assert_eq!(w.len(), 3, "{w:#?}");
    assert_eq!((w[0].peak_pct, w[0].resets_at_ms), (31.0, None));
    assert!(w[0].complete, "the next reading came 5 minutes after the last one");
    assert_eq!(w[1].peak_pct, 5.0);
    assert!(!w[1].complete, "nothing was read near its end");
    assert_eq!((w[2].peak_pct, w[2].readings, w[2].resets_at_ms), (64.0, 2, Some(reset)));
}

fn snap(ts: i64, provider: Provider, window: &str, used: f64, resets_ms: i64, plan: Option<&str>) -> LimitSnapshot {
    LimitSnapshot {
        ts_ms: ts,
        provider,
        tool: if provider == Provider::OpenAI { Tool::Codex } else { Tool::ClaudeCode },
        account: None,
        limit_id: if provider == Provider::OpenAI { "codex".into() } else { String::new() },
        window: window.into(),
        used_pct: Some(used),
        resets_at: Some(resets_ms / 1000),
        status: None,
        plan: plan.map(Into::into),
        source: "test".into(),
        accuracy: Accuracy::Exact,
    }
}

fn store_with(snaps: Vec<LimitSnapshot>) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&snaps).unwrap();
    tx.commit().unwrap();
    store
}

/// `weeks` weekly windows ending before NOW plus one five-hour window per day, each watched to
/// its end, at the given peaks.
fn claude_month(week_peak: f64, five_peak: f64, five_full_days: &[i64]) -> Vec<LimitSnapshot> {
    let mut v = Vec::new();
    for k in 0..4 {
        let reset = NOW - k * WEEK - DAY;
        v.push(snap(reset - 3 * DAY, Provider::Anthropic, "seven_day", week_peak / 2.0, reset, Some("max")));
        v.push(snap(reset - 2 * HOUR, Provider::Anthropic, "seven_day", week_peak, reset, Some("max")));
    }
    for d in 1..=27 {
        let reset = NOW - d * DAY;
        let peak = if five_full_days.contains(&d) { 100.0 } else { five_peak };
        v.push(snap(reset - 3 * HOUR, Provider::Anthropic, "five_hour", peak / 2.0, reset, Some("max")));
        v.push(snap(reset - 10 * MIN, Provider::Anthropic, "five_hour", peak, reset, Some("max")));
    }
    v
}

#[test]
fn history_covers_both_windows_and_counts_observed_days() {
    let store = store_with(claude_month(40.0, 30.0, &[]));
    let h = limit_history(&store, NOW, 28).unwrap();
    let five = h.series.iter().find(|s| s.window == "five_hour").unwrap();
    let week = h.series.iter().find(|s| s.window == "seven_day").unwrap();
    assert_eq!(five.windows.len(), 27);
    assert_eq!(week.windows.len(), 4);
    assert!(week.windows.iter().all(|w| w.complete && w.peak_pct == 40.0));
    assert!(h.observed_days[&Provider::Anthropic] >= 27);
}

#[test]
fn filled_limits_suggest_the_next_plan_with_its_published_ratio() {
    let store = store_with(claude_month(60.0, 50.0, &[2, 5, 9]));
    let h = limit_history(&store, NOW, 28).unwrap();
    let a = plan_advice(&h, Provider::Anthropic, Some("max5x"), &PlansFile::bundled(), NOW);
    assert_eq!(a.kind, AdviceKind::Upgrade);
    assert_eq!(a.suggested.as_deref(), Some("max20x"));
    assert_eq!(a.five_hour.full, 3);
    assert_eq!(a.five_hour.full_minutes, 30, "each was full for its last 10 minutes");
    assert_eq!(a.session_ratio, Some(4.0));
    assert_eq!(a.monthly_delta_usd, Some(100.0));
    assert!(!a.strong);
    // already the largest personal plan
    let top = plan_advice(&h, Provider::Anthropic, Some("max20x"), &PlansFile::bundled(), NOW);
    assert_eq!(top.kind, AdviceKind::AtTop);
}

#[test]
fn low_use_on_a_large_plan_suggests_the_one_below_only_with_enough_room() {
    // Max 20x at 15 % (five-hour) and 12 % (weekly): ×4 on Max 5x = 60 % and 48 %
    let store = store_with(claude_month(12.0, 15.0, &[]));
    let h = limit_history(&store, NOW, 28).unwrap();
    let a = plan_advice(&h, Provider::Anthropic, Some("max20x"), &PlansFile::bundled(), NOW);
    assert_eq!(a.kind, AdviceKind::Downgrade, "{a:#?}");
    assert_eq!(a.suggested.as_deref(), Some("max5x"));
    assert_eq!(a.projected_five_hour, Some(60.0));
    assert_eq!(a.projected_weekly_if_same_ratio, Some(48.0));
    assert_eq!(a.monthly_delta_usd, Some(-100.0));

    // at 25 % the plan below would be full (×4 = 100 %): the plan fits as it is
    let store = store_with(claude_month(12.0, 25.0, &[]));
    let h = limit_history(&store, NOW, 28).unwrap();
    let a = plan_advice(&h, Provider::Anthropic, Some("max20x"), &PlansFile::bundled(), NOW);
    assert_eq!(a.kind, AdviceKind::Fits);
    assert_eq!(a.projected_five_hour, Some(100.0));
}

#[test]
fn openai_gets_no_downgrade_because_no_ratio_is_published() {
    let mut v = Vec::new();
    for d in 1..=20 {
        let reset = NOW - d * DAY;
        v.push(snap(reset - 10 * MIN, Provider::OpenAI, "five_hour", 5.0, reset, Some("pro")));
    }
    let h = limit_history(&store_with(v), NOW, 28).unwrap();
    let a = plan_advice(&h, Provider::OpenAI, Some("pro"), &PlansFile::bundled(), NOW);
    assert_eq!(a.kind, AdviceKind::Fits);
    assert!(a.suggested.is_none() && a.session_ratio.is_none());
}

#[test]
fn plus_users_hitting_the_five_hour_limit_are_told_pro_has_none() {
    let mut v = Vec::new();
    for d in [1, 3, 6] {
        let reset = NOW - d * DAY;
        v.push(snap(reset - HOUR, Provider::OpenAI, "five_hour", 100.0, reset, Some("plus")));
    }
    // a window recorded under another plan is not counted
    v.push(snap(NOW - 2 * DAY - HOUR, Provider::OpenAI, "five_hour", 100.0, NOW - 2 * DAY, Some("go")));
    let h = limit_history(&store_with(v), NOW, 28).unwrap();
    let a = plan_advice(&h, Provider::OpenAI, Some("plus"), &PlansFile::bundled(), NOW);
    assert_eq!(a.kind, AdviceKind::Upgrade);
    assert_eq!(a.five_hour.full, 3);
    assert_eq!(a.suggested.as_deref(), Some("pro"));
    assert!(a.suggested_has_no_five_hour);
}

#[test]
fn without_enough_days_or_a_plan_nothing_is_recommended() {
    let v = vec![snap(NOW - HOUR, Provider::Anthropic, "five_hour", 40.0, NOW + HOUR, Some("max"))];
    let h = limit_history(&store_with(v), NOW, 28).unwrap();
    let plans = PlansFile::bundled();
    assert_eq!(plan_advice(&h, Provider::Anthropic, Some("max20x"), &plans, NOW).kind, AdviceKind::Insufficient);
    assert_eq!(plan_advice(&h, Provider::Anthropic, None, &plans, NOW).kind, AdviceKind::NoPlan);
    assert_eq!(plan_advice(&h, Provider::Anthropic, Some("team_premium"), &plans, NOW).kind, AdviceKind::NotApplicable);
}

fn usage(key: &str, ts: i64, input: u64) -> UsageEvent {
    UsageEvent {
        key: key.into(),
        ts_ms: ts,
        tool: Tool::ClaudeCode,
        client: None,
        model: "claude-sonnet-5".into(),
        project_path: None,
        session_id: None,
        tokens: Tokens { input, ..Default::default() },
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

#[test]
fn each_window_gets_the_local_use_inside_it_and_a_capacity_from_complete_windows() {
    // two weekly windows watched to their end: 20 % for $2 of use, 40 % for $3
    let w1 = NOW - 9 * DAY;
    let w2 = NOW - 2 * DAY;
    let mut store = store_with(vec![
        snap(w1 - 2 * HOUR, Provider::Anthropic, "seven_day", 20.0, w1, None),
        snap(w2 - 2 * HOUR, Provider::Anthropic, "seven_day", 40.0, w2, None),
        // still running: never part of the capacity
        snap(NOW - HOUR, Provider::Anthropic, "seven_day", 15.0, NOW + 5 * DAY, None),
    ]);
    let mut tx = store.transaction().unwrap();
    // Sonnet 5 input: $2 per 1M
    tx.upsert_events(&[
        usage("u1", w1 - 3 * DAY, 1_000_000),
        usage("u2", w1 - DAY, 0),
        usage("u2b", w1 - DAY + 1, 0),
        usage("u3", w2 - 4 * DAY, 1_500_000),
        usage("u4", w1 - 8 * DAY, 1_000_000), // before the first window
    ])
    .unwrap();
    tx.commit().unwrap();
    let mut h = limit_history(&store, NOW, 28).unwrap();
    add_local_usage(&mut h, &store, &PriceBook::default_book()).unwrap();
    let s = h.series.iter().find(|s| s.window == "seven_day").unwrap();
    let w: Vec<(u64, f64)> = s.windows.iter().map(|w| (w.local.requests, (w.local.cost_usd * 100.0).round() / 100.0)).collect();
    assert_eq!(w, vec![(3, 2.0), (1, 3.0), (0, 0.0)]);
    // $2 / 20 % × 100 = $10 and $3 / 40 % × 100 = $7.50 → median $8.75
    let c = s.capacity.as_ref().unwrap();
    assert_eq!(c.windows, 2);
    assert!((c.median_usd - 8.75).abs() < 1e-9 && (c.min_usd - 7.5).abs() < 1e-9 && (c.max_usd - 10.0).abs() < 1e-9);
}
