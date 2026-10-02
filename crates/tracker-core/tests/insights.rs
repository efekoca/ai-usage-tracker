//! Sessions, model comparison, context analysis, plan value and limit forecasts.

use chrono::{FixedOffset, TimeZone};
use tracker_core::analytics::{Filter, Range};
use tracker_core::insights::{compare_models, context_stats, forecast, plan_value, sessions, ForecastKind};
use tracker_core::model::{Accuracy, Provider, Tokens, Tool, UsageEvent};
use tracker_core::pricing::{CostInput, PriceBook};
use tracker_core::store::Store;

const MIN: i64 = 60_000;
const HOUR: i64 = 3_600_000;

fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
}

struct Ev {
    key: &'static str,
    ts: i64,
    tool: Tool,
    model: &'static str,
    session: Option<&'static str>,
    project: &'static str,
    tokens: Tokens,
}

#[allow(clippy::too_many_arguments)]
fn ev(key: &'static str, ts: i64, tool: Tool, model: &'static str, session: Option<&'static str>, project: &'static str, input: u64, cache_read: u64, output: u64) -> Ev {
    Ev { key, ts, tool, model, session, project, tokens: Tokens { input, cache_read, output, ..Default::default() } }
}

fn store_with(evs: &[Ev]) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let rows: Vec<UsageEvent> = evs
        .iter()
        .map(|e| UsageEvent {
            key: e.key.into(),
            ts_ms: e.ts,
            tool: e.tool,
            client: Some("cli".into()),
            model: e.model.into(),
            project_path: Some(e.project.into()),
            session_id: e.session.map(Into::into),
            request_input: e.tokens.input + e.tokens.cache_read + e.tokens.cache_write,
            tokens: e.tokens,
            web_search_requests: 0,
            speed: None,
            service_tier: None,
            inference_geo: None,
            request_id: None,
            accuracy: Accuracy::Exact,
            source: "test".into(),
        })
        .collect();
    let mut tx = store.transaction().unwrap();
    tx.upsert_events(&rows).unwrap();
    tx.commit().unwrap();
    store
}

fn all() -> Range {
    Range { from_ms: 0, to_ms: i64::MAX / 2 }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---------------------------------------------------------------- forecast

#[test]
fn a_fast_window_fills_before_the_reset() {
    let now = ms(2026, 10, 2, 12, 0);
    // 5-hour window, 2 h in, 60 % used = 30 %/h; 40 points left = 80 min < 3 h to the reset
    let f = forecast(now, 60.0, now + 3 * HOUR, 5 * HOUR);
    assert_eq!(f.kind, ForecastKind::Fills);
    assert!(close(f.rate_per_hour.unwrap(), 30.0));
    assert_eq!(f.fills_at_ms, Some(now + 80 * MIN));
    assert_eq!(f.basis_minutes, 120);
}

#[test]
fn a_slow_window_is_safe_and_says_where_it_ends() {
    let now = ms(2026, 10, 2, 12, 0);
    // 2 h in, 20 % = 10 %/h; 3 h left → 50 % at the reset
    let f = forecast(now, 20.0, now + 3 * HOUR, 5 * HOUR);
    assert_eq!(f.kind, ForecastKind::Safe);
    assert!(close(f.at_reset_pct.unwrap(), 50.0));
}

#[test]
fn the_weekly_pace_includes_idle_hours() {
    let now = ms(2026, 10, 2, 12, 0);
    let week = 7 * 24 * HOUR;
    // 4 days in, 37 % used: 9.25 %/day, 3 days left → 64.75 % at the reset (a burst today does
    // not make it fill tomorrow)
    let f = forecast(now, 37.0, now + 3 * 24 * HOUR, week);
    assert_eq!(f.kind, ForecastKind::Safe);
    assert!(close(f.at_reset_pct.unwrap(), 37.0 + 37.0 / 4.0 * 3.0));
}

#[test]
fn an_early_or_unused_window_is_not_extrapolated() {
    let now = ms(2026, 10, 2, 12, 0);
    // 20 minutes into a 5-hour window: under a tenth of it
    assert_eq!(forecast(now, 15.0, now + 4 * HOUR + 40 * MIN, 5 * HOUR).kind, ForecastKind::Insufficient);
    let idle = forecast(now, 0.0, now + 2 * HOUR, 5 * HOUR);
    assert_eq!(idle.kind, ForecastKind::Idle);
    assert!(close(idle.at_reset_pct.unwrap(), 0.0));
}

// ---------------------------------------------------------------- sessions

#[test]
fn sessions_group_by_tool_and_id_and_keep_their_largest_prompt() {
    let t = ms(2026, 9, 10, 9, 0);
    let s = store_with(&[
        ev("a1", t, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 100, 50_000, 10),
        ev("a2", t + 10 * MIN, Tool::ClaudeCode, "claude-haiku-4-5", Some("s1"), "C:/w/app", 10, 120_000, 5),
        ev("a3", t + 12 * MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/other", 1, 0, 1),
        // same id string from another tool is another session
        ev("b1", t + HOUR, Tool::Codex, "gpt-5.6-terra", Some("s1"), "C:/w/app", 1000, 0, 100),
        ev("c1", t + 2 * HOUR, Tool::Codex, "gpt-5.6-terra", None, "C:/w/app", 5, 0, 5),
    ]);
    let r = sessions(&s, &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    assert_eq!(r.events_without_session, 1);
    assert_eq!(r.sessions.len(), 2);
    let codex = &r.sessions[0]; // newest first
    assert_eq!((codex.tool, codex.totals.events), (Tool::Codex, 1));
    let cl = &r.sessions[1];
    assert_eq!((cl.session_id.as_str(), cl.totals.events), ("s1", 3));
    assert_eq!((cl.started_ms, cl.ended_ms), (t, t + 12 * MIN));
    assert_eq!(cl.project, "app", "the project most requests belong to");
    assert_eq!(cl.max_context, 120_010);
    assert_eq!(cl.models.len(), 2);
    assert!(close(cl.models.iter().map(|m| m.cost_usd).sum::<f64>(), cl.totals.cost_usd));
}

// ---------------------------------------------------------------- model comparison

#[test]
fn every_priced_model_is_compared_on_the_same_requests() {
    let t = ms(2026, 9, 10, 9, 0);
    let s = store_with(&[
        // Sonnet 5: 1M input × $2 + 100K output × $10 = $3.00
        ev("p1", t, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 1_000_000, 0, 100_000),
        // no price: left out of the comparison
        ev("u1", t + MIN, Tool::Codex, "codex-auto-review", Some("x"), "C:/w/app", 500, 0, 5),
    ]);
    let c = compare_models(&s, &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    assert_eq!((c.basis_events, c.excluded_events), (1, 1));
    assert!(close(c.actual_cost_usd, 3.0));
    let opus = c.targets.iter().find(|x| x.model == "claude-opus-5").unwrap();
    // Opus 5: 1M × $5 + 100K × $25 = $7.50
    assert!(close(opus.cost_usd, 7.5));
    assert!(close(opus.delta_usd, 4.5));
    let same = c.targets.iter().find(|x| x.model == "claude-sonnet-5").unwrap();
    assert!(close(same.delta_usd, 0.0));
    assert!(c.targets.windows(2).all(|w| w[0].cost_usd <= w[1].cost_usd), "cheapest first");
    assert_eq!(c.targets.len(), PriceBook::default_book().file().models.len());
}

// ---------------------------------------------------------------- context

#[test]
fn context_sizes_are_bucketed_and_long_context_premiums_counted() {
    let t = ms(2026, 9, 10, 9, 0);
    let s = store_with(&[
        ev("k1", t, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 5_000, 0, 1),
        ev("k2", t + MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 1_000, 60_000, 1),
        // over gpt-5.5's 272K threshold
        ev("k3", t + 2 * MIN, Tool::Codex, "gpt-5.5", Some("x"), "C:/w/app", 300_000, 0, 1_000),
    ]);
    let book = PriceBook::default_book();
    let tz = FixedOffset::east_opt(3 * 3600).unwrap();
    let c = context_stats(&s, &book, all(), &Filter::default(), &tz).unwrap();
    assert_eq!(c.requests, 3);
    assert_eq!(c.max, 300_000);
    assert_eq!(c.median, 61_000);
    let counts: Vec<u64> = c.buckets.iter().map(|b| b.requests).collect();
    assert_eq!(counts, vec![1, 0, 1, 0, 0, 1, 0, 0]);
    assert_eq!(c.long_context_requests, 1);
    // gpt-5.5 long tier: input $10 vs $5, output $45 vs $30
    let expected = 300_000.0 * 5.0 / 1e6 + 1_000.0 * 15.0 / 1e6;
    assert!(close(c.long_context_extra_usd, expected), "{}", c.long_context_extra_usd);
    let g = c.by_model.iter().find(|m| m.model == "gpt-5.5").unwrap();
    assert_eq!((g.threshold, g.over_threshold), (Some(272_000), 1));
    assert_eq!(c.daily.len(), 1);
}

#[test]
fn long_context_premium_is_zero_at_the_threshold() {
    let book = PriceBook::default_book();
    let tokens = Tokens { input: 272_000, output: 10, ..Default::default() };
    let at = CostInput { model: "gpt-5.5", tokens: &tokens, request_input: 272_000, web_search_requests: 0, speed: None, inference_geo: None };
    assert!(!book.is_long_context(&at));
    assert_eq!(book.long_context_premium(&at), Some(0.0));
    assert_eq!(book.long_context_premium(&CostInput { model: "no-such-model", ..at }), None);
}

// ---------------------------------------------------------------- plan value

#[test]
fn plan_value_runs_a_daily_total_per_provider() {
    let tz = FixedOffset::east_opt(3 * 3600).unwrap();
    let d1 = tz.with_ymd_and_hms(2026, 9, 10, 10, 0, 0).unwrap().timestamp_millis();
    let d3 = tz.with_ymd_and_hms(2026, 9, 12, 10, 0, 0).unwrap().timestamp_millis();
    let s = store_with(&[
        ev("v1", d1, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 1_000_000, 0, 0),
        ev("v2", d3, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 500_000, 0, 0),
        // under the 272K long-context threshold: 200K × $2
        ev("v3", d3, Tool::Codex, "gpt-5.6-terra", Some("x"), "C:/w/app", 200_000, 0, 0),
    ]);
    let from = tz.with_ymd_and_hms(2026, 9, 10, 0, 0, 0).unwrap().timestamp_millis();
    let to = tz.with_ymd_and_hms(2026, 9, 13, 0, 0, 0).unwrap().timestamp_millis();
    let v = plan_value(&s, &PriceBook::default_book(), Range { from_ms: from, to_ms: to }, &tz).unwrap();
    assert_eq!(v.dates, vec!["2026-09-10", "2026-09-11", "2026-09-12"]);
    let a = v.providers.iter().find(|p| p.provider == Provider::Anthropic).unwrap();
    assert!(close(a.cost_usd, 3.0));
    assert_eq!(a.cumulative.len(), 3);
    assert!(close(a.cumulative[0], 2.0) && close(a.cumulative[1], 2.0) && close(a.cumulative[2], 3.0));
    let o = v.providers.iter().find(|p| p.provider == Provider::OpenAI).unwrap();
    assert!(close(o.cost_usd, 0.4));
}
