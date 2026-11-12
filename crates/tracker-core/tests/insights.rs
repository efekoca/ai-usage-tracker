mod common;

use chrono::{FixedOffset, TimeZone};
use common::Dst;
use tracker_core::analytics::{account_key, Filter, Range};
use tracker_core::insights::{agents_tools, branches, compare_models, context_stats, day_detail, forecast, plan_value, restarted, session_timeline, sessions, subagent_cache, ForecastKind, PromptShare};
use tracker_core::insights::live_caches;
use tracker_core::tips::{cache_rebuilds, tips, Tip, Tips};
use tracker_core::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool, ToolCall, UsageEvent};
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
    branch: Option<&'static str>,
    agent: Option<&'static str>,
    thread: Option<&'static str>,
    speed: Option<&'static str>,
    accuracy: Accuracy,
}

#[allow(clippy::too_many_arguments)]
fn ev(key: &'static str, ts: i64, tool: Tool, model: &'static str, session: Option<&'static str>, project: &'static str, input: u64, cache_read: u64, output: u64) -> Ev {
    Ev {
        key,
        ts,
        tool,
        model,
        session,
        project,
        tokens: Tokens { input, cache_read, output, ..Default::default() },
        branch: None,
        agent: None,
        thread: None,
        speed: None,
        accuracy: Accuracy::Exact,
    }
}

impl Ev {
    fn branch(self, b: &'static str) -> Ev {
        Ev { branch: Some(b), ..self }
    }
    fn agent(self, a: &'static str, thread: &'static str) -> Ev {
        Ev { agent: Some(a), thread: Some(thread), ..self }
    }
    fn speed(self, s: &'static str) -> Ev {
        Ev { speed: Some(s), ..self }
    }
    fn captured(self) -> Ev {
        Ev { accuracy: Accuracy::Captured, ..self }
    }
    fn cache_write(mut self, all: u64, one_hour: u64) -> Ev {
        self.tokens.cache_write = all;
        self.tokens.cache_write_1h = one_hour;
        self
    }
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
            speed: e.speed.map(Into::into),
            service_tier: None,
            inference_geo: None,
            request_id: None,
            accuracy: e.accuracy,
            source: "test".into(),
            branch: e.branch.map(Into::into),
            agent: e.agent.map(Into::into),
            thread_id: e.thread.map(Into::into),
            limit_reset: None,
            creator: None,
        })
        .collect();
    let mut tx = store.transaction().unwrap();
    tx.upsert_events(&rows).unwrap();
    tx.commit().unwrap();
    store
}

#[test]
fn each_conversation_keeps_its_own_cache_counted_from_its_last_request() {
    let t = ms(2026, 11, 10, 12, 0);
    let book = PriceBook::default_book();
    let store = store_with(&[
        ev("a", t - 2 * HOUR, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "/p", 10, 0, 5).cache_write(1_000, 800),
        ev("b", t, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "/p", 10, 0, 5).cache_write(1_000, 800),
        ev("c", t + 2 * MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "/p", 10, 900, 5).branch("main"),
        ev("d", t + 3 * MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "/p", 10, 0, 5).agent("Explore", "x").cache_write(500, 0),
        ev("e", t + 3 * MIN, Tool::ClaudeCode, "<synthetic>", Some("s"), "/p", 0, 0, 0),
        ev("f", t + MIN, Tool::ClaudeCode, "claude-sonnet-5-5", Some("h"), "/q", 10, 0, 5).cache_write(2_000, 2_000),
        ev("g", t + MIN, Tool::Codex, "gpt-5.6-terra", Some("x"), "/p", 10, 400, 5),
        ev("i", t + MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("n"), "/p", 10, 0, 5),
    ]);
    let live = live_caches(&store, &book, t + 4 * MIN).unwrap();
    assert_eq!(live.iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), ["s", "h"], "soonest first; nothing cached, other tools and subagents left out");
    let s = &live[0];
    assert_eq!((s.expires_ms, s.ttl_ms), (t + 7 * MIN, 5 * MIN), "the read at +2 renews; a failed request and a subagent's do not");
    assert_eq!((s.tokens, s.branch.as_deref(), s.model.as_str()), (900, Some("main"), "claude-opus-5-5"));
    assert_eq!(s.started_ms, t - 2 * HOUR, "the conversation began before the last hour");
    let (read, write) = (s.refresh_usd.unwrap(), s.rewrite_usd.unwrap());
    assert!(read > 0.0 && write > 10.0 * read, "a refresh reads the cache for a fraction of writing it again");
    assert_eq!((live[1].expires_ms, live[1].ttl_ms, live[1].tokens), (t + MIN + HOUR, HOUR, 2_000));
    assert!(live[1].rewrite_usd.unwrap() > book.cost(&CostInput { model: "claude-sonnet-5-5", tokens: &Tokens { cache_write: 2_000, ..Default::default() }, request_input: 2_000, web_search_requests: 0, speed: None, inference_geo: None, ts_ms: None }).unwrap().total(), "an hour's cache costs more to write");
    assert_eq!(live_caches(&store, &book, t + 8 * MIN).unwrap().iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), ["h"], "run out");
}

#[test]
fn a_cache_keeps_the_lifetime_it_was_last_written_with() {
    let t = ms(2026, 11, 10, 12, 0);
    let book = PriceBook::default_book();
    let opus = "claude-opus-5-5";
    // a 5-minute cache only read for an hour; its write is older than the hour looked at
    let mut evs = vec![ev("w", t, Tool::ClaudeCode, opus, Some("r"), "/p", 10, 0, 5).cache_write(1_000, 0)];
    evs.extend((1..=15).map(|i| ev(Box::leak(format!("r{i}").into_boxed_str()), t + i * 4 * MIN, Tool::ClaudeCode, opus, Some("r"), "/p", 10, 1_000, 5)));
    evs.extend([
        // a 5-minute cache that ran out, then written again whole for an hour
        ev("x1", t, Tool::ClaudeCode, opus, Some("x"), "/p", 10, 0, 5).cache_write(1_000, 0),
        ev("x2", t + 10 * MIN, Tool::ClaudeCode, opus, Some("x"), "/p", 10, 0, 5).cache_write(2_000, 2_000),
        // the model switched and back: each model has a cache of its own
        ev("m1", t + 55 * MIN, Tool::ClaudeCode, opus, Some("m"), "/p", 10, 0, 5).cache_write(1_000, 0),
        ev("m2", t + 56 * MIN, Tool::ClaudeCode, "claude-sonnet-5-5", Some("m"), "/p", 10, 0, 5).cache_write(2_000, 2_000),
        ev("m3", t + 57 * MIN, Tool::ClaudeCode, opus, Some("m"), "/p", 10, 1_000, 5),
        // a helper request seen only through telemetry, which cannot tell the lifetimes apart
        ev("c1", t + 50 * MIN, Tool::ClaudeCode, opus, Some("c"), "/p", 10, 0, 5).cache_write(3_000, 3_000),
        ev("c2", t + 55 * MIN, Tool::ClaudeCode, "claude-haiku-5-5", Some("c"), "/p", 10, 900, 5).cache_write(500, 0).captured(),
    ]);
    let store = store_with(&evs);
    let live = live_caches(&store, &book, t + 61 * MIN).unwrap();
    let got: Vec<(&str, i64, i64)> = live.iter().map(|c| (c.session_id.as_str(), c.ttl_ms, c.expires_ms)).collect();
    assert_eq!(got, [("m", 5 * MIN, t + 62 * MIN), ("r", 5 * MIN, t + 65 * MIN), ("x", HOUR, t + 70 * MIN), ("c", HOUR, t + 110 * MIN)]);
    assert_eq!((live[0].model.as_str(), live[3].model.as_str()), (opus, opus));
}

fn all() -> Range {
    Range { from_ms: 0, to_ms: i64::MAX / 2 }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_fast_window_fills_before_the_reset() {
    let now = ms(2026, 10, 2, 12, 0);
    // 5-hour window, 2 h in, 60 % used = 30 %/h; 40 points left = 80 min < 3 h to the reset
    let f = forecast(now, 60.0, now + 3 * HOUR, 5 * HOUR, None);
    assert_eq!(f.kind, ForecastKind::Fills);
    assert!(close(f.rate_per_hour.unwrap(), 30.0));
    assert_eq!(f.fills_at_ms, Some(now + 80 * MIN));
    assert_eq!(f.basis_minutes, 120);
}

#[test]
fn a_slow_window_is_safe_and_says_where_it_ends() {
    let now = ms(2026, 10, 2, 12, 0);
    // 2 h in, 20 % = 10 %/h; 3 h left → 50 % at the reset
    let f = forecast(now, 20.0, now + 3 * HOUR, 5 * HOUR, None);
    assert_eq!(f.kind, ForecastKind::Safe);
    assert!(close(f.at_reset_pct.unwrap(), 50.0));
}

#[test]
fn the_weekly_pace_includes_idle_hours() {
    let now = ms(2026, 10, 2, 12, 0);
    let week = 7 * 24 * HOUR;
    // 4 days in, 37 % used: 9.25 %/day, 3 days left → 64.75 % at the reset (a burst today does
    // not make it fill tomorrow)
    let f = forecast(now, 37.0, now + 3 * 24 * HOUR, week, None);
    assert_eq!(f.kind, ForecastKind::Safe);
    assert!(close(f.at_reset_pct.unwrap(), 37.0 + 37.0 / 4.0 * 3.0));
}

#[test]
fn an_early_or_unused_window_is_not_extrapolated() {
    let now = ms(2026, 10, 2, 12, 0);
    // 20 minutes into a 5-hour window: under a tenth of it
    assert_eq!(forecast(now, 15.0, now + 4 * HOUR + 40 * MIN, 5 * HOUR, None).kind, ForecastKind::Insufficient);
    let idle = forecast(now, 0.0, now + 2 * HOUR, 5 * HOUR, None);
    assert_eq!(idle.kind, ForecastKind::Idle);
    assert!(close(idle.at_reset_pct.unwrap(), 0.0));
}

#[test]
fn a_reset_the_provider_grants_inside_a_window_restarts_the_pace() {
    let t = ms(2026, 10, 2, 9, 0);
    // 40 % and 45 %, then a reset to 2 % that every later reading confirms
    let reads = [(t, 40.0), (t + 30 * MIN, 45.0), (t + 40 * MIN, 2.0), (t + 55 * MIN, 5.0), (t + 70 * MIN, 6.0)];
    assert_eq!(restarted(&reads), Some(t + 30 * MIN));
    let now = t + 2 * HOUR + 30 * MIN;
    // 2 h since the last reading before the reset, 12 % used: 6 %/h; without it the window's
    // 5 h start would give 2.4 %/h
    let f = forecast(now, 12.0, t + 3 * HOUR, 5 * HOUR, restarted(&reads));
    assert!(close(f.rate_per_hour.unwrap(), 6.0));
    assert_eq!(f.basis_minutes, 120);
    // a pace measured over too short a time since the reset is not trusted either
    assert_eq!(forecast(t + 50 * MIN, 3.0, t + 3 * HOUR, 5 * HOUR, Some(t + 30 * MIN)).kind, ForecastKind::Insufficient);
}

#[test]
fn a_granted_reset_stays_a_reset_when_use_fills_the_window_again() {
    let t = ms(2026, 10, 2, 9, 0);
    let reads = [(t, 100.0), (t + 10 * MIN, 100.0), (t + 30 * MIN, 1.0), (t + 50 * MIN, 2.0), (t + 80 * MIN, 100.0)];
    assert_eq!(restarted(&reads), Some(t + 10 * MIN));
}

#[test]
fn a_delayed_reading_is_not_a_reset() {
    let t = ms(2026, 10, 2, 9, 0);
    // a request that ran for a while reports the limit as it was when it began
    assert_eq!(restarted(&[(t, 40.0), (t + 5 * MIN, 52.0), (t + 6 * MIN, 38.0), (t + 7 * MIN, 53.0), (t + 30 * MIN, 58.0)]), None);
    // a fall seen only in the newest few minutes waits for confirmation
    assert_eq!(restarted(&[(t, 40.0), (t + 30 * MIN, 45.0), (t + 32 * MIN, 1.0), (t + 35 * MIN, 2.0)]), None);
    assert_eq!(restarted(&[(t, 10.0), (t + HOUR, 30.0)]), None);
    assert_eq!(restarted(&[]), None);
}

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
    // the plan providers' models; other vendors are not offered in the comparison
    let plan_models = PriceBook::default_book().file().models.iter().filter(|m| Provider::parse(&m.provider).is_some()).count();
    assert_eq!(c.targets.len(), plan_models);
    assert!(c.targets.iter().all(|x| x.model != "deepseek-flash"));
}

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
    let at = CostInput { model: "gpt-5.5", tokens: &tokens, request_input: 272_000, web_search_requests: 0, speed: None, inference_geo: None, ts_ms: None };
    assert!(!book.is_long_context(&at));
    assert_eq!(book.long_context_premium(&at), Some(0.0));
    assert_eq!(book.long_context_premium(&CostInput { model: "no-such-model", ..at }), None);
}

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
    assert!(a.accounts.is_empty() && o.accounts.is_empty(), "one account each: no split");
}

#[test]
fn a_long_plan_value_range_keeps_its_newest_days_and_the_whole_total() {
    let tz = FixedOffset::east_opt(3 * 3600).unwrap();
    let from = tz.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap().timestamp_millis();
    let to = tz.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap().timestamp_millis();
    let s = store_with(&[
        // before the days shown: part of the first day's running total
        ev("o1", from + HOUR, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 1_000_000, 0, 0),
        ev("o2", to - HOUR, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 500_000, 0, 0),
    ]);
    let v = plan_value(&s, &PriceBook::default_book(), Range { from_ms: from, to_ms: to }, &tz).unwrap();
    assert_eq!(v.dates.len(), 401);
    assert_eq!(v.dates.last().map(String::as_str), Some("2026-05-31"));
    let a = &v.providers[0];
    assert!(close(a.cost_usd, 3.0));
    assert!(close(a.cumulative[0], 2.0) && close(*a.cumulative.last().unwrap(), a.cost_usd));
}

#[test]
fn branches_are_per_project_and_requests_without_one_are_kept_apart() {
    let t = ms(2026, 9, 10, 9, 0);
    let s = store_with(&[
        ev("b1", t, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 1_000_000, 0, 0).branch("main"),
        ev("b2", t + MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 500_000, 0, 0).branch("main"),
        ev("b3", t + 2 * MIN, Tool::Codex, "gpt-5.6-terra", Some("x1"), "C:/w/app", 100_000, 0, 0).branch("feature/login"),
        // the same branch name in another project is another branch
        ev("b4", t + 3 * MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s2"), "C:/w/site", 10, 0, 0).branch("main"),
        ev("b5", t + 4 * MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s3"), "C:/w/app", 10, 0, 0),
    ]);
    let b = branches(&s, &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    assert_eq!((b.events_with_branch, b.events_without_branch), (4, 1));
    assert_eq!(b.rows.len(), 4);
    let main = &b.rows[0]; // most expensive first: 1.5M Sonnet input = $3
    assert_eq!((main.project.as_str(), main.branch.as_deref(), main.totals.events, main.sessions), ("app", Some("main"), 2, 1));
    assert!(close(main.totals.cost_usd, 3.0));
    assert_eq!((main.first_ms, main.last_ms), (t, t + MIN));
    assert!(b.rows.iter().any(|r| r.project == "site" && r.branch.as_deref() == Some("main")));
    assert!(b.rows.iter().any(|r| r.project == "app" && r.branch.is_none()));
    let codex = b.rows.iter().find(|r| r.branch.as_deref() == Some("feature/login")).unwrap();
    assert_eq!(codex.tools, vec![Tool::Codex]);
}

/// (key, time, tool, name, subagent, failed)
type Call<'a> = (&'a str, i64, Tool, &'a str, Option<&'a str>, Option<bool>);

fn add_calls(store: &mut Store, calls: &[Call]) {
    let rows: Vec<ToolCall> = calls
        .iter()
        .map(|(key, ts, tool, name, agent, failed)| ToolCall {
            key: (*key).into(),
            ts_ms: *ts,
            tool: *tool,
            session_id: Some("s1".into()),
            project_path: Some("C:/w/app".into()),
            agent: agent.map(Into::into),
            name: (*name).into(),
            failed: *failed,
        })
        .collect();
    let mut tx = store.transaction().unwrap();
    tx.upsert_tool_calls(&rows).unwrap();
    tx.commit().unwrap();
}

#[test]
fn subagent_share_and_tool_calls_are_counted() {
    let t = ms(2026, 9, 10, 9, 0);
    let mut s = store_with(&[
        ev("m1", t, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 1_000_000, 0, 0),
        ev("a1", t + MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 500_000, 0, 0).agent("Explore", "agent-1"),
        ev("a2", t + 2 * MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 500_000, 0, 0).agent("Explore", "agent-2"),
        ev("g1", t + 3 * MIN, Tool::Codex, "codex-auto-review", Some("x1"), "C:/w/app", 100, 0, 0).agent("guardian", "x2"),
    ]);
    add_calls(
        &mut s,
        &[
            ("c1", t, Tool::ClaudeCode, "Bash", None, Some(true)),
            ("c2", t, Tool::ClaudeCode, "Bash", None, Some(false)),
            ("c3", t, Tool::ClaudeCode, "Bash", None, None),
            ("c4", t + MIN, Tool::ClaudeCode, "Grep", Some("Explore"), Some(false)),
            ("c5", t + 3 * MIN, Tool::Codex, "shell", None, Some(false)),
        ],
    );
    let r = agents_tools(&s, &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    let main = &r.agents[0];
    assert_eq!((main.tool, main.agent.as_deref(), main.totals.events, main.runs), (Tool::ClaudeCode, None, 1, 0));
    let explore = &r.agents[1];
    assert_eq!((explore.agent.as_deref(), explore.totals.events, explore.runs, explore.sessions), (Some("Explore"), 2, 2, 1));
    assert!(close(explore.totals.cost_usd, 2.0));
    assert!(r.agents.iter().any(|a| a.tool == Tool::Codex && a.agent.as_deref() == Some("guardian")));
    assert_eq!(r.tool_calls, 5);
    let bash = &r.tools[0];
    assert_eq!((bash.name.as_str(), bash.calls, bash.known, bash.failed, bash.by_subagents), ("Bash", 3, 2, 1, 0));
    let grep = r.tools.iter().find(|x| x.name == "Grep").unwrap();
    assert_eq!(grep.by_subagents, 1);

    // a model filter keeps the calls of the sessions that used the model
    let f = Filter { models: vec!["gpt-5.6-terra".into()], ..Default::default() };
    let r = agents_tools(&s, &PriceBook::default_book(), all(), &f).unwrap();
    assert_eq!((r.tool_calls, r.filtered_by_session), (0, true));
    let f = Filter { tools: vec![Tool::Codex], ..Default::default() };
    assert_eq!(agents_tools(&s, &PriceBook::default_book(), all(), &f).unwrap().tool_calls, 1);
}

#[test]
fn subagent_cache_separates_runs_and_the_main_request_after_each_run() {
    let t = ms(2026, 9, 10, 9, 0);
    let cc = |key, at, input, read, write| ev(key, at, Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", input, read, 10).cache_write(write, 0);
    let s = store_with(&[
        cc("m1", t, 1000, 0, 0),
        cc("a1", t + MIN, 0, 600, 400).agent("Explore", "agent-1"),
        cc("a2", t + 2 * MIN, 0, 900, 100).agent("Explore", "agent-1"),
        cc("m2", t + 3 * MIN, 950, 50, 0),
        cc("m3", t + 4 * MIN, 0, 1000, 0),
        cc("b1", t + 5 * MIN, 0, 0, 3000).agent("general-purpose", "agent-2"),
        ev("m4", t, Tool::ClaudeCode, "claude-sonnet-5", Some("s2"), "C:/w/app", 0, 500, 10),
        ev("p1", ms(2026, 9, 9, 12, 0), Tool::ClaudeCode, "claude-sonnet-5", Some("s0"), "C:/w/app", 100, 300, 10).agent("Explore", "agent-0"),
    ]);
    let day = Range { from_ms: ms(2026, 9, 10, 0, 0), to_ms: ms(2026, 9, 11, 0, 0) };
    let r = subagent_cache(&s, &PriceBook::default_book(), day, &Filter::default(), &FixedOffset::east_opt(0).unwrap()).unwrap();
    assert_eq!((r.runs, r.sessions), (2, 1));
    assert_eq!((r.main.events, r.subagents.events), (4, 3));
    assert_eq!(r.first, PromptShare { requests: 2, prompt: 4000, cache_read: 600 });
    assert_eq!(r.main_after_run, PromptShare { requests: 1, prompt: 1000, cache_read: 50 });
    assert_eq!(r.main_other, PromptShare { requests: 2, prompt: 2000, cache_read: 1000 });
    assert_eq!(r.previous_subagents, PromptShare { requests: 1, prompt: 400, cache_read: 300 });
    assert_eq!(r.previous_main.requests, 0);
    let rows: Vec<_> = r.rows.iter().map(|x| (x.agent.as_deref(), x.runs, x.totals.events)).collect();
    assert_eq!(rows, vec![(None, 0, 4), (Some("general-purpose"), 1, 1), (Some("Explore"), 1, 2)]);
    assert!(r.subagents.cost.cache_write > 0.0);
}

fn tip_kinds(t: &Tips) -> Vec<String> {
    t.tips.iter().map(|x| serde_json::to_value(x).unwrap()["kind"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn a_context_rewritten_after_the_cache_expired_is_priced_against_a_warm_read() {
    let t = ms(2026, 9, 10, 9, 0);
    // Opus 5.5: 1-hour write $8, read $0.20 per 1M
    let rebuild = |key, at| ev(key, at, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1, 20_000, 10).cache_write(80_000, 80_000);
    let s = store_with(&[
        ev("r0", t, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1, 0, 10).cache_write(100_000, 100_000),
        rebuild("r1", t + 2 * HOUR),
        ev("r2", t + 2 * HOUR + 5 * MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1, 100_000, 10).cache_write(500, 500),
        rebuild("r3", t + 4 * HOUR),
        rebuild("r4", t + 6 * HOUR),
        // 20 minutes: the 1-hour cache was still there, whatever caused this write
        rebuild("r5", t + 6 * HOUR + 20 * MIN),
        // another model has its own cache
        ev("r6", t + 9 * HOUR, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 1, 20_000, 10).cache_write(80_000, 80_000),
    ]);
    let book = PriceBook::default_book();
    let evs = s.events_between(0, i64::MAX / 2).unwrap();
    let r = cache_rebuilds(&evs, &book);
    assert_eq!((r.requests, r.sessions, r.tokens), (3, 1, 240_000));
    assert!(close(r.extra_usd, 3.0 * 80_000.0 * (8.0 - 0.2) / 1e6), "{}", r.extra_usd);
    let t = tips(&s, &book, all(), &Filter::default()).unwrap();
    assert_eq!(tip_kinds(&t), vec!["cache_rebuild"]);
}

#[test]
fn only_the_context_cached_before_counts_as_rebuilt() {
    let t = ms(2026, 9, 10, 9, 0);
    let s = store_with(&[
        // 100K cached (1-hour writes)
        ev("p0", t, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1, 0, 10).cache_write(100_000, 100_000),
        // an advisor on another model in between has its own cache
        ev("p0a", t + MIN, Tool::ClaudeCode, "claude-fable-5-1", Some("s"), "C:/w/app", 30_000, 0, 1_000),
        // two hours later 200K, all written: only the earlier 100K was warm
        ev("p1", t + 2 * HOUR, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1, 0, 10).cache_write(200_000, 200_000),
        // a pasted 150K document on top of a 10K cached prompt is new content, not a rebuild
        ev("q0", t, Tool::ClaudeCode, "claude-opus-5-5", Some("q"), "C:/w/app", 1, 0, 10).cache_write(10_000, 10_000),
        ev("q1", t + 2 * HOUR, Tool::ClaudeCode, "claude-opus-5-5", Some("q"), "C:/w/app", 1, 0, 10).cache_write(160_000, 160_000),
    ]);
    let evs = s.events_between(0, i64::MAX / 2).unwrap();
    let r = cache_rebuilds(&evs, &PriceBook::default_book());
    assert_eq!((r.requests, r.sessions, r.tokens), (1, 1, 100_000));
    // Opus 5.5: 1-hour write $8, read $0.20 per 1M
    assert!(close(r.extra_usd, 100_000.0 * (8.0 - 0.2) / 1e6), "{}", r.extra_usd);
}

#[test]
fn price_tier_surcharges_are_measured_against_the_standard_rate() {
    let t = ms(2026, 9, 10, 9, 0);
    let s = store_with(&[
        // gpt-5.5 over 272K: +$1.515 (see the context test)
        ev("l1", t, Tool::Codex, "gpt-5.5", Some("x"), "C:/w/app", 300_000, 0, 1_000),
        // Opus 5.5 fast mode doubles the price: 1M input $8 instead of $4
        ev("f1", t + MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1_000_000, 0, 0).speed("fast"),
        ev("f2", t + 2 * MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 10, 0, 0).speed("standard"),
    ]);
    let r = tips(&s, &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    assert_eq!(tip_kinds(&r), vec!["fast_mode", "long_context"], "largest amount first");
    match &r.tips[0] {
        Tip::FastMode { requests, extra_usd, .. } => assert!(*requests == 1 && close(*extra_usd, 4.0)),
        other => panic!("{other:?}"),
    }
    match &r.tips[1] {
        Tip::LongContext { requests, extra_usd, models, .. } => {
            assert_eq!((*requests, models.as_slice()), (1, ["gpt-5.5".to_string()].as_slice()));
            assert!(close(*extra_usd, 300_000.0 * 5.0 / 1e6 + 1_000.0 * 15.0 / 1e6));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn small_amounts_and_rare_tool_errors_make_no_tip() {
    let t = ms(2026, 9, 10, 9, 0);
    let mut evs = vec![
        // fast mode worth $0.000004: real but not worth a tip
        ev("f1", t, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 1, 0, 0).speed("fast"),
    ];
    // $40 of ordinary use
    evs.push(ev("n1", t + MIN, Tool::ClaudeCode, "claude-opus-5-5", Some("s"), "C:/w/app", 10_000_000, 0, 0));
    let mut s = store_with(&evs);
    let mut calls = Vec::new();
    let names = ["Read", "Bash", "Odd"];
    for i in 0..40 {
        // Read: 2 of 20 fail (10 %); Bash: 6 of 20 fail (30 %); Odd: 5 of 5 fail (too few calls)
        let (name, failed) = if i < 20 { (names[0], i < 2) } else { (names[1], i < 26) };
        calls.push((format!("k{i}"), name, failed));
    }
    for i in 0..5 {
        calls.push((format!("o{i}"), names[2], true));
    }
    let rows: Vec<Call> =
        calls.iter().map(|(k, n, f)| (k.as_str(), t, Tool::ClaudeCode, *n, None, Some(*f))).collect();
    add_calls(&mut s, &rows);
    let r = tips(&s, &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    assert_eq!(tip_kinds(&r), vec!["tool_errors"]);
    match &r.tips[0] {
        Tip::ToolErrors { name, calls, known, failed, rate_pct, .. } => assert_eq!((name.as_str(), *calls, *known, *failed, *rate_pct), ("Bash", 20, 20, 6, 30.0)),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.tool_calls, 45);
}

#[test]
fn large_prompts_are_reported_when_they_carry_most_of_the_cost() {
    let t = ms(2026, 9, 10, 9, 0);
    let keys: Vec<String> = (0..25).map(|i| format!("g{i}")).collect();
    let mut evs: Vec<Ev> =
        keys.iter().enumerate().map(|(i, k)| ev(k.clone().leak(), t + i as i64 * MIN, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 150_000, 0, 0)).collect();
    evs.push(ev("small", t + HOUR, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "C:/w/app", 1_000, 0, 0));
    let r = tips(&store_with(&evs), &PriceBook::default_book(), all(), &Filter::default()).unwrap();
    match &r.tips[..] {
        [Tip::LargeContexts { requests, cost_share_pct, threshold, .. }] => {
            assert_eq!((*requests, *threshold), (25, 100_000));
            assert!(*cost_share_pct > 99.0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_day_is_split_by_local_hour_with_its_sessions_and_limit_peaks() {
    let tz = FixedOffset::east_opt(3 * 3600).unwrap();
    let at = |d: u32, h: u32, m: u32| tz.with_ymd_and_hms(2026, 9, d, h, m, 0).unwrap().timestamp_millis();
    let mut s = store_with(&[
        // 23:50 the evening before: another day
        ev("d0", at(9, 23, 50), Tool::ClaudeCode, "claude-sonnet-5", Some("s0"), "C:/w/app", 1_000, 0, 0),
        ev("d1", at(10, 0, 10), Tool::ClaudeCode, "claude-sonnet-5", Some("s1"), "C:/w/app", 1_000_000, 0, 0),
        ev("d2", at(10, 0, 40), Tool::ClaudeCode, "claude-opus-5-5", Some("s1"), "C:/w/app", 10, 0, 0),
        ev("d3", at(10, 14, 5), Tool::Codex, "gpt-5.6-terra", Some("x1"), "C:/w/site", 100, 0, 0),
    ]);
    let snap = |ts: i64, window: &str, used: f64| LimitSnapshot {
        ts_ms: ts,
        provider: Provider::Anthropic,
        tool: Tool::ClaudeCode,
        account: None,
        limit_id: String::new(),
        window: window.into(),
        used_pct: Some(used),
        resets_at: None,
        status: None,
        plan: None,
        source: "test".into(),
        accuracy: Accuracy::Exact,
    };
    let mut tx = s.transaction().unwrap();
    tx.insert_limits(&[snap(at(10, 1, 0), "five_hour", 30.0), snap(at(10, 3, 0), "five_hour", 72.0), snap(at(10, 3, 0), "seven_day", 41.0), snap(at(11, 9, 0), "five_hour", 99.0)]).unwrap();
    tx.commit().unwrap();

    let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
    let d = day_detail(&s, &PriceBook::default_book(), date, &Filter::default(), &tz).unwrap();
    assert_eq!(d.date, "2026-09-10");
    assert_eq!(d.totals.events, 3);
    assert_eq!(d.hourly.len(), 24);
    assert_eq!((d.hourly[0].events, d.hourly[14].events, d.hourly[23].events), (2, 1, 0));
    assert_eq!(d.hourly[0].by_tool["claude_code"], 1_000_010);
    assert_eq!(d.sessions, 2);
    assert_eq!((d.first_ms, d.last_ms), (Some(at(10, 0, 10)), Some(at(10, 14, 5))));
    assert_eq!(d.by_model[0].key, "claude-sonnet-5", "most expensive first");
    assert_eq!(d.by_project.len(), 2);
    let peaks: Vec<(String, f64)> = d.limit_peaks.iter().map(|p| (p.window.clone(), p.peak_pct)).collect();
    assert_eq!(peaks, vec![("five_hour".to_string(), 72.0), ("seven_day".to_string(), 41.0)]);
    // a filter narrows the day like every other view
    let only_codex = Filter { tools: vec![Tool::Codex], ..Default::default() };
    assert_eq!(day_detail(&s, &PriceBook::default_book(), date, &only_codex, &tz).unwrap().totals.events, 1);
}

#[test]
fn a_session_timeline_marks_rebuilt_caches_compactions_subagents_and_failed_calls() {
    let t0 = ms(2026, 10, 2, 9, 0);
    let m = |n: i64| t0 + n * 60_000;
    let mut store = store_with(&[
        ev("t1", m(0), Tool::ClaudeCode, "claude-sonnet-4-6", Some("s1"), "C:/w/app", 0, 0, 100).cache_write(30_000, 0),
        ev("t2", m(1), Tool::ClaudeCode, "claude-sonnet-4-6", Some("s1"), "C:/w/app", 0, 30_000, 100).cache_write(30_000, 0),
        ev("t3", m(2), Tool::ClaudeCode, "claude-sonnet-4-6", Some("s1"), "C:/w/app", 10_000, 0, 100).agent("general-purpose", "a1"),
        // 18 minutes later only the shared start is still cached
        ev("t4", m(20), Tool::ClaudeCode, "claude-sonnet-4-6", Some("s1"), "C:/w/app", 0, 5_000, 100).cache_write(65_000, 0),
        ev("t5", m(21), Tool::ClaudeCode, "claude-sonnet-4-6", Some("s1"), "C:/w/app", 20_000, 0, 100),
        ev("other", m(3), Tool::ClaudeCode, "claude-sonnet-4-6", Some("s2"), "C:/w/app", 50_000, 0, 100),
    ]);
    let calls: Vec<ToolCall> = [("c1", m(1), "Bash", Some(true)), ("c2", m(2), "Read", Some(false)), ("c3", m(2), "Read", None)]
        .iter()
        .map(|(key, ts, name, failed)| ToolCall {
            key: (*key).into(),
            ts_ms: *ts,
            tool: Tool::ClaudeCode,
            session_id: Some(if *key == "c3" { "s2" } else { "s1" }.into()),
            project_path: Some("C:/w/app".into()),
            agent: None,
            name: (*name).into(),
            failed: *failed,
        })
        .collect();
    let mut tx = store.transaction().unwrap();
    tx.upsert_tool_calls(&calls).unwrap();
    tx.commit().unwrap();

    let t = session_timeline(&store, &PriceBook::default_book(), Tool::ClaudeCode, "s1", m(0), m(21), &Filter::default()).unwrap();
    let contexts: Vec<u64> = t.requests.iter().map(|r| r.context).collect();
    assert_eq!(contexts, [30_000, 60_000, 10_000, 70_000, 20_000], "only this session, oldest first");
    assert_eq!(t.requests[2].agent.as_deref(), Some("general-purpose"));
    let rewritten: Vec<u64> = t.requests.iter().map(|r| r.rewritten).collect();
    assert_eq!(rewritten, [0, 0, 0, 55_000, 0], "the main conversation's cache, rebuilt after the pause");
    assert_eq!(t.requests[3].pause_ms, Some(19 * MIN), "measured from the main conversation, not the subagent");
    assert_eq!(t.requests[2].thread.as_deref(), Some("a1"));
    let extra = t.requests[3].rewrite_extra_usd.unwrap();
    assert!(extra > 0.0 && t.requests.iter().filter(|r| r.rewrite_extra_usd.is_some()).count() == 1, "{extra}");
    let compacted: Vec<bool> = t.requests.iter().map(|r| r.compacted).collect();
    assert_eq!(compacted, [false, false, false, false, true], "the subagent's small prompt is not a compaction");
    assert!(t.requests.iter().all(|r| r.cost_usd.is_some()));
    let calls: Vec<(&str, Option<bool>)> = t.calls.iter().map(|c| (c.name.as_str(), c.failed)).collect();
    assert_eq!(calls, [("Bash", Some(true)), ("Read", Some(false))]);
}

#[test]
fn a_session_timeline_keeps_the_filter_of_the_session_list() {
    let m = |n: i64| ms(2026, 10, 2, 9, 0) + n * MIN;
    let mut store = store_with(&[
        ev("a1", m(0), Tool::ClaudeCode, "claude-sonnet-5-5", Some("s1"), "C:/w/app", 1_000, 0, 100),
        ev("b1", m(1), Tool::ClaudeCode, "claude-opus-5-5", Some("s1"), "C:/w/app", 2_000, 0, 100),
        ev("a2", m(2), Tool::ClaudeCode, "claude-sonnet-5-5", Some("s1"), "C:/w/app", 3_000, 0, 100),
    ]);
    let calls: Vec<ToolCall> = [("c1", m(0) + 1_000), ("c2", m(1) + 1_000), ("c3", m(2) + 1_000)]
        .iter()
        .map(|(key, ts)| ToolCall {
            key: (*key).into(),
            ts_ms: *ts,
            tool: Tool::ClaudeCode,
            session_id: Some("s1".into()),
            project_path: Some("C:/w/app".into()),
            agent: None,
            name: (*key).into(),
            failed: None,
        })
        .collect();
    let mut tx = store.transaction().unwrap();
    tx.upsert_tool_calls(&calls).unwrap();
    tx.commit().unwrap();

    let book = PriceBook::default_book();
    let only = Filter { models: vec!["claude-sonnet-5-5".into()], ..Default::default() };
    let t = session_timeline(&store, &book, Tool::ClaudeCode, "s1", m(0), m(3), &only).unwrap();
    let contexts: Vec<u64> = t.requests.iter().map(|r| r.context).collect();
    assert_eq!(contexts, [1_000, 3_000], "the other model's request in between stays out");
    assert!(t.calls.is_empty() && t.calls_hidden, "a call records no request, so none is guessed for the filter");

    let all = session_timeline(&store, &book, Tool::ClaudeCode, "s1", m(0), m(3), &Filter::default()).unwrap();
    assert_eq!((all.requests.len(), all.calls.len(), all.calls_hidden), (3, 3, false));
    let opus = Filter { models: vec!["claude-opus-5-5".into()], ..Default::default() };
    assert!(session_timeline(&store, &book, Tool::ClaudeCode, "s1", m(0), m(3), &opus).unwrap().calls_hidden);
}

#[test]
fn a_compaction_keeps_the_context_before_it_when_a_filter_hides_that_request() {
    let m = |n: i64| ms(2026, 10, 2, 9, 0) + n * MIN;
    let store = store_with(&[
        ev("a", m(0), Tool::ClaudeCode, "claude-sonnet-5-5", Some("s1"), "C:/w/app", 10_000, 0, 100),
        ev("b", m(1), Tool::ClaudeCode, "claude-opus-5-5", Some("s1"), "C:/w/app", 80_000, 0, 100),
        ev("c", m(2), Tool::ClaudeCode, "claude-sonnet-5-5", Some("s1"), "C:/w/app", 20_000, 0, 100),
    ]);
    let sonnet = Filter { models: vec!["claude-sonnet-5-5".into()], ..Default::default() };
    let t = session_timeline(&store, &PriceBook::default_book(), Tool::ClaudeCode, "s1", m(0), m(2), &sonnet).unwrap();
    let last = t.requests.last().unwrap();
    assert!(last.compacted);
    assert_eq!(last.compacted_from, Some(80_000), "the hidden request before it, not the visible 10K");
}

/// Another group's message that began before the row and sent a tool call inside the row's span.
fn a_call_from_before_the_row() -> (Store, i64, i64) {
    let t = ms(2026, 10, 2, 9, 0);
    let s = 1_000;
    let mut store = store_with(&[
        ev("b1", t, Tool::ClaudeCode, "claude-opus-5-5", Some("s1"), "C:/w/app", 1_000, 0, 100),
        ev("a1", t + 10 * s, Tool::ClaudeCode, "claude-sonnet-5-5", Some("s1"), "C:/w/app", 2_000, 0, 100),
        ev("a2", t + 30 * s, Tool::ClaudeCode, "claude-sonnet-5-5", Some("s1"), "C:/w/app", 3_000, 0, 100),
        ev("b2", t + 60 * s, Tool::ClaudeCode, "claude-opus-5-5", Some("s1"), "C:/w/app", 4_000, 0, 100),
    ]);
    let call = ToolCall {
        key: "b1-read".into(),
        ts_ms: t + 20 * s,
        tool: Tool::ClaudeCode,
        session_id: Some("s1".into()),
        project_path: Some("C:/w/app".into()),
        agent: None,
        name: "Read".into(),
        failed: Some(true),
    };
    let mut tx = store.transaction().unwrap();
    tx.upsert_tool_calls(&[call]).unwrap();
    tx.commit().unwrap();
    store.conn().execute_batch("UPDATE usage_event SET owner = 'b' WHERE key IN ('b1', 'b2'); UPDATE usage_event SET owner = 'a' WHERE key IN ('a1', 'a2');").unwrap();
    // the filtered row starts at a1 and ends at a2, as the session list gives it
    (store, t + 10 * s, t + 30 * s)
}

#[test]
fn a_call_from_a_message_that_began_before_the_row_is_not_shown_for_the_model() {
    let (store, from, to) = a_call_from_before_the_row();
    let book = PriceBook::default_book();
    let sonnet = Filter { models: vec!["claude-sonnet-5-5".into()], ..Default::default() };
    let t = session_timeline(&store, &book, Tool::ClaudeCode, "s1", from, to, &sonnet).unwrap();
    assert_eq!(t.requests.len(), 2);
    assert!(t.calls.is_empty() && t.calls_hidden, "the session has opus requests, even if none falls in the row");
    let all = session_timeline(&store, &book, Tool::ClaudeCode, "s1", from, to, &Filter::default()).unwrap();
    assert_eq!((all.calls.len(), all.calls_hidden), (1, false), "without a filter the call stays");
}

#[test]
fn a_call_from_another_accounts_message_does_not_show_for_the_account() {
    let (store, from, to) = a_call_from_before_the_row();
    let only_a = Filter { accounts: vec![account_key(Provider::Anthropic, Some("a"))], ..Default::default() };
    let t = session_timeline(&store, &PriceBook::default_book(), Tool::ClaudeCode, "s1", from, to, &only_a).unwrap();
    assert_eq!(t.requests.len(), 2);
    assert!(t.calls.is_empty() && t.calls_hidden);
}

#[test]
fn an_advisor_is_not_the_conversations_cache_context_or_compaction() {
    let t = ms(2026, 11, 10, 12, 0);
    let book = PriceBook::default_book();
    let (sonnet, opus) = ("claude-sonnet-5-5", "claude-opus-5-5");
    // logged in the line of the request that consulted it, so stored right after it at the same time
    let store = store_with(&[
        ev("cc:msg_0", t - MIN, Tool::ClaudeCode, sonnet, Some("s"), "/p", 10, 0, 5).cache_write(60_000, 0),
        ev("cc:msg_1", t, Tool::ClaudeCode, sonnet, Some("s"), "/p", 10, 60_000, 5).cache_write(1_000, 0),
        ev("cc:msg_1:claude-opus-5-5", t, Tool::ClaudeCode, opus, Some("s"), "/p", 2_000, 0, 50).cache_write(3_000, 3_000),
        ev("cc:msg_2", t + 2 * MIN, Tool::ClaudeCode, sonnet, Some("s"), "/p", 25_000, 0, 5),
    ]);

    let live = live_caches(&store, &book, t + 3 * MIN).unwrap();
    let got: Vec<(&str, &str, i64, i64, u64)> = live.iter().map(|c| (c.session_id.as_str(), c.model.as_str(), c.ttl_ms, c.expires_ms, c.tokens)).collect();
    assert_eq!(got, [("s", sonnet, 5 * MIN, t + 5 * MIN, 61_000)]);

    let timeline = session_timeline(&store, &book, Tool::ClaudeCode, "s", t - MIN, t + 2 * MIN, &Filter::default()).unwrap();
    let marks: Vec<(u64, bool)> = timeline.requests.iter().map(|r| (r.context, r.compacted)).collect();
    assert_eq!(marks, [(60_010, false), (61_010, false), (5_000, false), (25_000, true)], "the advisor's small prompt is no compaction and hides none");
    assert_eq!(timeline.requests[3].compacted_from, Some(61_010));

    let list = sessions(&store, &book, all(), &Filter::default()).unwrap();
    let s = &list.sessions[0];
    assert_eq!((s.totals.events, s.max_context), (4, 61_010), "the advisor is counted and priced, as its own model");
    assert!(close(s.avg_context, (60_010.0 + 61_010.0 + 25_000.0) / 3.0), "{}", s.avg_context);
}

#[test]
fn telemetry_rows_are_left_out_of_cache_rebuilds_and_compactions() {
    let t = ms(2026, 9, 10, 9, 0);
    let opus = "claude-opus-5-5";
    let store = store_with(&[
        ev("l0", t, Tool::ClaudeCode, opus, Some("s"), "/p", 1, 0, 10).cache_write(100_000, 100_000),
        // telemetry names no cache lifetime and no subagent, and also holds helper requests
        ev("o1", t + 20 * MIN, Tool::ClaudeCode, opus, Some("s"), "/p", 1, 0, 10).cache_write(100_000, 0).captured(),
        ev("o2", t + 30 * MIN, Tool::ClaudeCode, opus, Some("s"), "/p", 1, 0, 10).cache_write(100_000, 0).captured(),
        ev("o3", t + 35 * MIN, Tool::ClaudeCode, opus, Some("s"), "/p", 3_000, 0, 10).captured(),
        ev("l1", t + 40 * MIN, Tool::ClaudeCode, opus, Some("s"), "/p", 1, 100_000, 10).cache_write(500, 500),
    ]);
    let book = PriceBook::default_book();
    let timeline = session_timeline(&store, &book, Tool::ClaudeCode, "s", t, t + 40 * MIN, &Filter::default()).unwrap();
    assert_eq!(timeline.requests.len(), 5, "still listed");
    assert!(timeline.requests.iter().all(|r| r.rewritten == 0 && !r.compacted), "{:?}", timeline.requests.iter().map(|r| (r.rewritten, r.compacted)).collect::<Vec<_>>());
    let evs = store.events_between(0, i64::MAX / 2).unwrap();
    assert_eq!(cache_rebuilds(&evs, &book).requests, 0);
}

#[test]
fn a_day_keeps_each_accounts_limit_peaks_apart() {
    let at = |h: u32| ms(2026, 9, 10, h, 0);
    let (a, b) = ("aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb");
    let mut store = store_with(&[ev("x", at(1), Tool::Codex, "gpt-5.6-terra", Some("x1"), "/p", 100, 0, 0)]);
    let snap = |ts: i64, provider: Provider, used: f64| LimitSnapshot {
        ts_ms: ts,
        provider,
        tool: if provider == Provider::OpenAI { Tool::Codex } else { Tool::ClaudeCode },
        account: None,
        limit_id: if provider == Provider::OpenAI { "codex" } else { "" }.into(),
        window: "five_hour".into(),
        used_pct: Some(used),
        resets_at: None,
        status: None,
        plan: None,
        source: "test".into(),
        accuracy: Accuracy::Exact,
    };
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(&[snap(at(2), Provider::OpenAI, 30.0), snap(at(3), Provider::OpenAI, 60.0), snap(at(4), Provider::OpenAI, 80.0), snap(at(5), Provider::Anthropic, 40.0)]).unwrap();
    tx.commit().unwrap();
    let owner = |account: &str, ts: i64| store.conn().execute("UPDATE limit_snapshot SET owner = ?1 WHERE ts_ms = ?2", tracker_core::rusqlite::params![account, ts]).unwrap();
    owner(a, at(2));
    owner(a, at(3));
    owner(b, at(4));

    let (book, tz, date) = (PriceBook::default_book(), FixedOffset::east_opt(0).unwrap(), chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap());
    let peaks = |filter: &Filter| -> Vec<(Provider, Option<String>, f64)> {
        day_detail(&store, &book, date, filter, &tz).unwrap().limit_peaks.into_iter().map(|p| (p.provider, p.account, p.peak_pct)).collect()
    };
    assert_eq!(
        peaks(&Filter::default()),
        [(Provider::Anthropic, None, 40.0), (Provider::OpenAI, Some(a.to_owned()), 60.0), (Provider::OpenAI, Some(b.to_owned()), 80.0)],
        "one account's peak is not another's; a provider with one account is shown as before"
    );
    let only_a = Filter { accounts: vec![account_key(Provider::OpenAI, Some(a))], ..Default::default() };
    assert_eq!(peaks(&only_a), [(Provider::OpenAI, Some(a.to_owned()), 60.0)]);
}

#[test]
fn a_day_with_a_clock_change_keeps_every_local_hour_once() {
    let cet = Dst::europe();
    let local = |m: u32, d: u32, h: u32, mi: u32| cet.with_ymd_and_hms(2026, m, d, h, mi, 0).earliest().unwrap().timestamp_millis();
    let at = |key: &'static str, ts: i64| ev(key, ts, Tool::ClaudeCode, "claude-sonnet-5", Some("s"), "/p", 100, 0, 10);
    let store = store_with(&[
        at("before", local(3, 28, 23, 50)),
        at("s0", local(3, 29, 0, 10)),
        at("s1", local(3, 29, 1, 30)),
        at("s3", local(3, 29, 3, 30)),
        at("s23", local(3, 29, 23, 50)),
        at("next", local(3, 30, 0, 10)),
        // 02:30 came twice on 25 October
        at("a1", ms(2026, 10, 25, 0, 30)),
        at("a2", ms(2026, 10, 25, 1, 30)),
        at("a23", local(10, 25, 23, 50)),
        at("a_next", local(10, 26, 0, 10)),
    ]);
    let book = PriceBook::default_book();
    let day = |m: u32, d: u32| day_detail(&store, &book, chrono::NaiveDate::from_ymd_opt(2026, m, d).unwrap(), &Filter::default(), &cet).unwrap();
    let busy = |d: &tracker_core::insights::DayDetail| d.hourly.iter().filter(|h| h.events > 0).map(|h| (h.hour, h.events)).collect::<Vec<_>>();
    let spring = day(3, 29);
    assert_eq!((spring.totals.events, spring.hourly.len()), (4, 24), "23 hours long");
    assert_eq!(busy(&spring), [(0, 1), (1, 1), (3, 1), (23, 1)]);
    let autumn = day(10, 25);
    assert_eq!((autumn.totals.events, autumn.hourly.len()), (3, 24), "25 hours long");
    assert_eq!(busy(&autumn), [(2, 2), (23, 1)]);
}
