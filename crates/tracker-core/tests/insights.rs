use chrono::{FixedOffset, TimeZone};
use tracker_core::analytics::{Filter, Range};
use tracker_core::insights::{agents_tools, branches, compare_models, context_stats, day_detail, forecast, plan_value, sessions, ForecastKind};
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
            accuracy: Accuracy::Exact,
            source: "test".into(),
            branch: e.branch.map(Into::into),
            agent: e.agent.map(Into::into),
            thread_id: e.thread.map(Into::into),
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
    assert_eq!(c.targets.len(), PriceBook::default_book().file().models.len());
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
    let at = CostInput { model: "gpt-5.5", tokens: &tokens, request_input: 272_000, web_search_requests: 0, speed: None, inference_geo: None };
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
