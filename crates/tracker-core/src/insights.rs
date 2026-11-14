//! Derived views over the archive, computed from stored counts only.

use crate::analytics::{local_date, Filter, Range, Totals};
use crate::model::{Accuracy, Provider, Tool};
use crate::pricing::{CostInput, PriceBook};
use crate::store::{EventRow, ProjectRow, Store};
use chrono::{Days, NaiveDate, TimeZone};
use rusqlite::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

fn input_of(e: &EventRow) -> CostInput<'_> {
    CostInput {
        model: &e.model,
        tokens: &e.tokens,
        request_input: e.request_input,
        web_search_requests: e.web_search,
        speed: e.speed.as_deref(),
        inference_geo: e.inference_geo.as_deref(),
        ts_ms: Some(e.ts_ms),
    }
}

fn add(t: &mut Totals, book: &PriceBook, e: &EventRow) {
    let i = input_of(e);
    t.add_event(e, book.cost(&i).as_ref(), book.cache_savings(&i).unwrap_or(0.0));
}

fn project_label(projects: &HashMap<i64, ProjectRow>, id: Option<i64>) -> (String, bool) {
    match id.and_then(|i| projects.get(&i)) {
        Some(p) => (p.name.clone(), p.hidden),
        None => (String::new(), false),
    }
}

/// Nearest-rank percentile of a sorted slice.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionModel {
    pub model: String,
    pub events: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub unpriced: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub session_id: String,
    pub tool: Tool,
    pub client: Option<String>,
    pub project_id: Option<i64>,
    pub project: String,
    pub hidden: bool,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub totals: Totals,
    pub models: Vec<SessionModel>,
    /// Largest prompt sent in one request (uncached input + cache read + cache write).
    pub max_context: u64,
    pub avg_context: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sessions {
    pub sessions: Vec<SessionRow>,
    /// Requests in the range that carry no session id (counted nowhere above).
    pub events_without_session: u64,
}

/// Sessions with activity in `range` (only their part inside the range), newest first.
pub fn sessions(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<Sessions> {
    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    struct Acc {
        tool: Tool,
        client: Option<String>,
        projects: HashMap<Option<i64>, u64>,
        started: i64,
        ended: i64,
        totals: Totals,
        models: BTreeMap<String, SessionModel>,
        max_ctx: u64,
        ctx_sum: u64,
        ctx_n: u64,
    }
    let mut map: HashMap<String, Acc> = HashMap::new();
    let mut without = 0u64;
    for e in store.events_between(range.from_ms, range.to_ms)?.iter().filter(|e| filter.matches(e)) {
        let Some(sid) = e.session_id.as_deref().filter(|s| !s.is_empty()) else {
            without += 1;
            continue;
        };
        let a = map.entry(format!("{}:{sid}", e.tool.as_str())).or_insert_with(|| Acc {
            tool: e.tool,
            client: e.client.clone(),
            projects: HashMap::new(),
            started: e.ts_ms,
            ended: e.ts_ms,
            totals: Totals::default(),
            models: BTreeMap::new(),
            max_ctx: 0,
            ctx_sum: 0,
            ctx_n: 0,
        });
        a.started = a.started.min(e.ts_ms);
        a.ended = a.ended.max(e.ts_ms);
        if a.client.is_none() {
            a.client = e.client.clone();
        }
        *a.projects.entry(e.project_id).or_default() += 1;
        add(&mut a.totals, book, e);
        let c = book.cost(&input_of(e));
        let m = a.models.entry(e.model.clone()).or_insert_with(|| SessionModel {
            model: e.model.clone(),
            events: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            unpriced: false,
        });
        m.events += 1;
        m.total_tokens = m.total_tokens.saturating_add(e.tokens.total());
        match c {
            Some(c) => m.cost_usd += c.total(),
            None => m.unpriced = true,
        }
        // an advisor's prompt is its own, not one the conversation sent
        if e.request_input > 0 && !e.advisor {
            a.max_ctx = a.max_ctx.max(e.request_input);
            a.ctx_sum = a.ctx_sum.saturating_add(e.request_input);
            a.ctx_n += 1;
        }
    }
    let mut sessions: Vec<SessionRow> = map
        .into_iter()
        .map(|(key, a)| {
            // the project most of the session's requests belong to
            let project_id = a.projects.iter().max_by_key(|(id, n)| (**n, id.unwrap_or(i64::MIN))).and_then(|(id, _)| *id);
            let (project, hidden) = project_label(&projects, project_id);
            let mut models: Vec<SessionModel> = a.models.into_values().collect();
            models.sort_by(|x, y| y.cost_usd.total_cmp(&x.cost_usd).then(y.total_tokens.cmp(&x.total_tokens)));
            SessionRow {
                session_id: key.split_once(':').map(|(_, s)| s.to_owned()).unwrap_or(key),
                tool: a.tool,
                client: a.client,
                project_id,
                project,
                hidden,
                started_ms: a.started,
                ended_ms: a.ended,
                totals: a.totals,
                models,
                max_context: a.max_ctx,
                avg_context: if a.ctx_n > 0 { a.ctx_sum as f64 / a.ctx_n as f64 } else { 0.0 },
            }
        })
        .collect();
    sessions.sort_by(|x, y| y.started_ms.cmp(&x.started_ms).then(x.session_id.cmp(&y.session_id)));
    Ok(Sessions { sessions, events_without_session: without })
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareTarget {
    pub model: String,
    pub provider: Provider,
    pub cost_usd: f64,
    /// `cost_usd - actual`; negative = cheaper.
    pub delta_usd: f64,
    pub verified_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCompare {
    /// Requests whose actual model has a price (the basis of every figure here).
    pub basis_events: u64,
    pub basis_tokens: u64,
    /// Requests left out because their own model has no price to compare with.
    pub excluded_events: u64,
    pub actual_cost_usd: f64,
    /// Actual cost per model used, most expensive first.
    pub actual_by_model: Vec<(String, f64)>,
    /// Every priced model, cheapest first.
    pub targets: Vec<CompareTarget>,
}

/// What the same requests (same token counts, same prompt sizes) would have cost on every
/// priced model. Token counts are taken as-is: models with a different tokenizer would count
/// the same text differently, so cross-family figures are indicative.
pub fn compare_models(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<ModelCompare> {
    let events: Vec<EventRow> = store.events_between(range.from_ms, range.to_ms)?.into_iter().filter(|e| filter.matches(e)).collect();
    let mut basis = Vec::new();
    let mut excluded = 0u64;
    let mut actual = 0.0;
    let mut by_model: BTreeMap<String, f64> = BTreeMap::new();
    for e in &events {
        match book.cost(&input_of(e)) {
            Some(c) => {
                actual += c.total();
                *by_model.entry(e.model.clone()).or_default() += c.total();
                basis.push(e);
            }
            None => excluded += 1,
        }
    }
    // the comparison offers the plan providers' models; other vendors have no place in it yet
    let mut targets: Vec<CompareTarget> = book
        .file()
        .models
        .iter()
        .filter_map(|m| Some((m, Provider::parse(&m.provider)?)))
        .map(|(m, provider)| {
            let cost: f64 = basis
                .iter()
                .map(|e| {
                    let mut i = input_of(e);
                    i.model = &m.id;
                    book.cost(&i).map(|c| c.total()).unwrap_or(0.0)
                })
                .sum();
            CompareTarget {
                model: m.id.clone(),
                provider,
                cost_usd: cost,
                delta_usd: cost - actual,
                verified_at: m.verified_at.clone(),
                notes: m.notes.clone(),
            }
        })
        .collect();
    targets.sort_by(|a, b| a.cost_usd.total_cmp(&b.cost_usd).then(a.model.cmp(&b.model)));
    let mut actual_by_model: Vec<(String, f64)> = by_model.into_iter().collect();
    actual_by_model.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(ModelCompare {
        basis_events: basis.len() as u64,
        basis_tokens: basis.iter().map(|e| e.tokens.total()).sum(),
        excluded_events: excluded,
        actual_cost_usd: actual,
        actual_by_model,
        targets,
    })
}

/// A main-conversation prompt that falls under half of the one before, from at least this size, is
/// a compacted context.
pub const COMPACT_MIN_PROMPT: u64 = 20_000;

#[derive(Debug, Clone, Serialize)]
pub struct TimelineRequest {
    pub ts_ms: i64,
    pub model: String,
    /// The subagent that sent it; none for the main conversation.
    pub agent: Option<String>,
    /// The subagent run it belongs to; none for the main conversation.
    pub thread: Option<String>,
    /// The prompt: uncached input, cache read and cache write.
    pub context: u64,
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    pub cost_usd: Option<f64>,
    /// Claude: context written to the cache again after a pause longer than the cache lifetime.
    pub rewritten: u64,
    /// What that rewrite cost above a warm cache.
    pub rewrite_extra_usd: Option<f64>,
    /// With a rewrite: the pause since the previous request of the same conversation and model.
    pub pause_ms: Option<i64>,
    pub compacted: bool,
    /// With a compaction: the context of the main conversation's request before, which a filter
    /// may have left out of the list.
    pub compacted_from: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineCall {
    pub ts_ms: i64,
    pub name: String,
    pub agent: Option<String>,
    pub failed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionTimeline {
    pub requests: Vec<TimelineRequest>,
    pub calls: Vec<TimelineCall>,
    /// The filter left out some of the session's requests, so its tool calls are not shown: a
    /// call does not record which request it came from, and the nearest one in time can be
    /// another account's, an advisor's or a subagent's.
    pub calls_hidden: bool,
}

/// One session's requests and tool calls between `from_ms` and `to_ms`, oldest first, kept to
/// the requests `filter` allows, as in the session list. Cache rewrites and compactions are found
/// over the whole session, since the cache and the context are shared by all of it.
pub fn session_timeline(store: &Store, book: &PriceBook, tool: Tool, session_id: &str, from_ms: i64, to_ms: i64, filter: &Filter) -> Result<SessionTimeline> {
    let ours = |t: Tool, s: Option<&str>| t == tool && s == Some(session_id);
    let mut events: Vec<EventRow> = store.events_between(from_ms, to_ms + 1)?.into_iter().filter(|e| ours(e.tool, e.session_id.as_deref())).collect();
    events.sort_by_key(|e| e.ts_ms);
    let prompt = |e: &EventRow| if e.request_input > 0 { e.request_input } else { e.tokens.input + e.tokens.cache_read + e.tokens.cache_write };

    // captured telemetry also holds helper requests and names no subagent or cache lifetime
    let logged = |e: &EventRow| e.accuracy != Accuracy::Captured;
    let mut rewritten = vec![0; events.len()];
    let mut pause = vec![None; events.len()];
    if tool.provider() == Some(Provider::Anthropic) {
        // the cache is per conversation and model, as in `tips::cache_rebuilds`
        let mut last: HashMap<(Option<&str>, &str), usize> = HashMap::new();
        for (i, e) in events.iter().enumerate().filter(|(_, e)| logged(e)) {
            if let Some(&p) = last.get(&(e.thread_id.as_deref(), e.model.as_str())) {
                rewritten[i] = crate::tips::rewritten_after_pause(&events[p], e).unwrap_or(0);
                pause[i] = (rewritten[i] > 0).then(|| e.ts_ms - events[p].ts_ms);
            }
            last.insert((e.thread_id.as_deref(), e.model.as_str()), i);
        }
    }
    let mut compacted_from = vec![None; events.len()];
    let mut before: Option<u64> = None;
    // a subagent without a name still has its own thread; an advisor reads a prompt of its own
    for (i, e) in events.iter().enumerate().filter(|(_, e)| e.agent.is_none() && e.thread_id.is_none() && !e.advisor && logged(e) && prompt(e) > 0) {
        compacted_from[i] = before.filter(|&b| b >= COMPACT_MIN_PROMPT && prompt(e) * 2 < b);
        before = Some(prompt(e));
    }

    let requests = events
        .iter()
        .enumerate()
        .filter(|(_, e)| filter.matches(e))
        .map(|(i, e)| TimelineRequest {
            ts_ms: e.ts_ms,
            model: e.model.clone(),
            agent: e.agent.clone(),
            thread: e.thread_id.clone(),
            context: prompt(e),
            input: e.tokens.input,
            cache_read: e.tokens.cache_read,
            cache_write: e.tokens.cache_write,
            output: e.tokens.output,
            cost_usd: book.cost(&input_of(e)).map(|c| c.total()),
            rewritten: rewritten[i],
            rewrite_extra_usd: (rewritten[i] > 0).then(|| crate::tips::rewrite_extra_usd(book, e, rewritten[i])).flatten(),
            pause_ms: pause[i],
            compacted: compacted_from[i].is_some(),
            compacted_from: compacted_from[i],
        })
        .collect();
    let mut calls: Vec<TimelineCall> = store
        .tool_calls_between(from_ms, to_ms + 1)?
        .into_iter()
        .filter(|c| ours(c.tool, c.session_id.as_deref()))
        .map(|c| TimelineCall { ts_ms: c.ts_ms, name: c.name, agent: c.agent, failed: c.failed })
        .collect();
    // decided over the whole session, not the row's span: a request that began before the row's
    // first one can still send a tool call inside it
    let calls_hidden = !calls.is_empty() && !filter.is_empty() && store.session_events(session_id)?.iter().any(|e| e.tool == tool && !filter.matches(e));
    if calls_hidden {
        calls.clear();
    }
    calls.sort_by_key(|c| c.ts_ms);
    Ok(SessionTimeline { requests, calls, calls_hidden })
}

/// Bucket edges for prompt sizes. 100K, 200K and 272K are where long-context pricing starts for
/// Claude Haiku 5.5, Gemini Pro and current OpenAI models.
pub const CONTEXT_EDGES: [u64; 7] = [10_000, 50_000, 100_000, 200_000, 272_000, 500_000, 1_000_000];

#[derive(Debug, Clone, Serialize)]
pub struct ContextBucket {
    pub from: u64,
    /// Exclusive; `None` for the open-ended last bucket.
    pub to: Option<u64>,
    pub requests: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextByModel {
    pub model: String,
    pub requests: u64,
    pub avg: f64,
    pub p90: u64,
    pub max: u64,
    /// The model's long-context threshold, if its price has one.
    pub threshold: Option<u64>,
    pub over_threshold: u64,
    pub long_context_extra_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextDay {
    pub date: String,
    pub requests: u64,
    pub avg: f64,
    pub p90: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ContextStats {
    pub requests: u64,
    pub avg: f64,
    pub median: u64,
    pub p90: u64,
    pub p99: u64,
    pub max: u64,
    pub buckets: Vec<ContextBucket>,
    /// Requests priced at a long-context tier, and what that tier added over the standard rates.
    pub long_context_requests: u64,
    pub long_context_extra_usd: f64,
    pub by_model: Vec<ContextByModel>,
    pub daily: Vec<ContextDay>,
}

/// One conversation's first request: when, and how large its prompt was.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Start {
    pub ts_ms: i64,
    pub tokens: u64,
}

/// A lasting change in a project's starting load: from the conversation at `at_ms` on, they start
/// larger or smaller than those before.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StartChange {
    pub at_ms: i64,
    pub before: u64,
    pub after: u64,
}

/// What every new conversation of a project begins with, where it runs: the system prompt, the
/// tool definitions, memory files and MCP servers, as the first request's prompt size tells.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StartingLoad {
    pub project_id: Option<i64>,
    pub project: String,
    pub hidden: bool,
    pub tool: Tool,
    pub client: Option<String>,
    pub conversations: u64,
    /// The median of the latest conversations' first requests: what one begins with now.
    pub typical: u64,
    pub smallest: u64,
    /// API-equivalent cost of such a first request; `None` when none of them has a price.
    pub cost_usd: Option<f64>,
    pub change: Option<StartChange>,
    /// The conversations' first requests, the oldest first, at most [`START_POINTS`] of the latest.
    pub starts: Vec<Start>,
}

/// How many of the latest conversations tell what one begins with now, and how many on each side
/// of a change are compared.
const START_RECENT: usize = 5;
pub const START_POINTS: usize = 60;
const START_RUN: usize = 3;
const START_STEP_TOKENS: u64 = 2_000;
const START_STEP_SHARE: f64 = 0.10;

fn median(values: &[u64]) -> u64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    percentile(&v, 50.0)
}

/// Whether the conversations from `i` on start clearly larger or smaller than those before: the
/// medians of up to five on each side differ by at least 2,000 tokens and a tenth, and the first
/// three after lie beyond every one of those before.
fn step_at(starts: &[Start], i: usize) -> Option<StartChange> {
    let before: Vec<u64> = starts[i.saturating_sub(START_RECENT)..i].iter().map(|s| s.tokens).collect();
    let after: Vec<u64> = starts[i..(i + START_RECENT).min(starts.len())].iter().map(|s| s.tokens).collect();
    let (was, is) = (median(&before), median(&after));
    let apart = was.abs_diff(is) >= START_STEP_TOKENS.max((was as f64 * START_STEP_SHARE) as u64);
    // three in a row beyond all of those before: a long prompt among the later ones hides nothing
    let run = &after[..after.len().min(START_RUN)];
    let (low, high) = (run.iter().min()?, run.iter().max()?);
    let clear = low > before.iter().max()? || high < before.iter().min()?;
    (apart && clear).then(|| StartChange { at_ms: starts[i].ts_ms, before: was, after: is })
}

/// The latest lasting change in where the conversations start (see [`step_at`]), at the
/// conversation it began with. Two before and three after are needed, so one conversation that
/// began with a long prompt is not taken for a change.
fn start_change(starts: &[Start]) -> Option<StartChange> {
    let mut at = (2..starts.len().saturating_sub(2)).rev().find(|&i| step_at(starts, i).is_some())?;
    // the conversations right after a step still pass against the older ones: the step is the first that does
    while at > 2 && step_at(starts, at - 1).is_some() {
        at -= 1;
    }
    step_at(starts, at)
}

/// The first main-conversation request of every conversation that began in `range`, the oldest
/// first: subagents, advisors and captured telemetry (which also holds helper requests) are left
/// out, and so are a conversation that began before the range and only goes on in it and one
/// that began with another's history.
fn first_requests(store: &Store, range: Range, filter: &Filter) -> Result<Vec<EventRow>> {
    firsts_among(store, store.events_between(range.from_ms, range.to_ms)?, range, filter)
}

/// [`first_requests`] of a range whose events, the oldest first, are already read.
fn firsts_among(store: &Store, events: Vec<EventRow>, range: Range, filter: &Filter) -> Result<Vec<EventRow>> {
    let mut firsts: BTreeMap<(&str, String), EventRow> = BTreeMap::new();
    for e in events {
        if e.agent.is_some() || e.advisor || e.accuracy == Accuracy::Captured || e.request_input == 0 {
            continue;
        }
        if let Some(session) = e.session_id.clone() {
            // events come oldest first
            firsts.entry((e.tool.as_str(), session)).or_insert(e);
        }
    }
    // a conversation resumed under a new id or forked begins with all the earlier one carried
    let carried = store.carried_sessions()?;
    let mut out = Vec::new();
    for ((tool, session), e) in firsts {
        if carried.contains(&(tool.to_owned(), session.clone())) {
            continue;
        }
        if filter.matches(&e) && !store.session_started_ms(&session)?.is_some_and(|t| t < range.from_ms) {
            out.push(e);
        }
    }
    out.sort_by_key(|e| e.ts_ms);
    Ok(out)
}

/// One side of a before-and-after comparison: the use in `from_ms <= ts < to_ms`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ExperimentSide {
    pub from_ms: i64,
    pub to_ms: i64,
    /// Conversations with a request in it.
    pub sessions: u64,
    pub requests: u64,
    /// API-equivalent cost of its priced requests.
    pub cost_usd: f64,
    /// Requests that used tokens of a model without a price: they are in no cost.
    pub unpriced_requests: u64,
    /// The cost of the conversations' priced requests ÷ the conversations that have one; `None`
    /// without such a conversation.
    pub cost_per_session: Option<f64>,
    /// Conversations that began in it, and the median of their first requests' prompt size.
    pub started: u64,
    pub starting_load: Option<u64>,
    /// Tool calls whose outcome is known, and those of them that failed.
    pub tool_calls: u64,
    pub tool_failed: u64,
    /// `None` without a call whose outcome is known.
    pub tool_error_pct: Option<f64>,
    /// Cache read ÷ prompt tokens; `None` without a prompt token.
    pub cache_hit_pct: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Experiment {
    pub before: ExperimentSide,
    pub after: ExperimentSide,
}

fn experiment_side(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<ExperimentSide> {
    let mut side = ExperimentSide { from_ms: range.from_ms, to_ms: range.to_ms, ..Default::default() };
    if range.to_ms <= range.from_ms {
        return Ok(side);
    }
    let mut sessions = std::collections::HashSet::new();
    // a conversation counts towards the cost per session only with a request that has a price,
    // so a model without one does not make conversations look cheaper
    let (mut priced, mut priced_usd) = (std::collections::HashSet::new(), 0.0);
    let mut share = PromptShare::default();
    let events = store.events_between(range.from_ms, range.to_ms)?;
    for e in events.iter().filter(|e| filter.matches(e)) {
        side.requests += 1;
        share.add(&e.tokens);
        let session = e.session_id.as_ref().map(|s| (e.tool.as_str(), s.clone()));
        match book.cost(&input_of(e)).map(|c| c.total()) {
            Some(usd) => {
                side.cost_usd += usd;
                if let Some(s) = session.clone() {
                    priced.insert(s);
                    priced_usd += usd;
                }
            }
            // a request that used no token, such as one that failed, would cost nothing at any price
            None if e.tokens.total() > 0 => side.unpriced_requests += 1,
            None => {}
        }
        sessions.extend(session);
    }
    side.sessions = sessions.len() as u64;
    side.cost_per_session = (!priced.is_empty()).then(|| priced_usd / priced.len() as f64);
    side.cache_hit_pct = (share.prompt > 0).then(|| share.cache_read as f64 * 100.0 / share.prompt as f64);
    let starts: Vec<u64> = firsts_among(store, events, range, filter)?.iter().map(|e| e.request_input).collect();
    side.started = starts.len() as u64;
    side.starting_load = (!starts.is_empty()).then(|| median(&starts));
    // tool calls carry a tool and a project, which is all a comparison is narrowed by
    for c in store.tool_calls_between(range.from_ms, range.to_ms)? {
        let wanted = (filter.tools.is_empty() || filter.tools.contains(&c.tool)) && (filter.projects.is_empty() || c.project_id.is_some_and(|p| filter.projects.contains(&p)));
        if let Some(failed) = c.failed.filter(|_| wanted) {
            side.tool_calls += 1;
            side.tool_failed += u64::from(failed);
        }
    }
    side.tool_error_pct = (side.tool_calls > 0).then(|| side.tool_failed as f64 * 100.0 / side.tool_calls as f64);
    Ok(side)
}

/// The use in the `days` days before `at_ms` against the same number of days from it on, as far
/// as they have passed by `now_ms`: what changed after something the user did at `at_ms`. Only
/// the tool and the project of `filter` narrow it.
pub fn experiment<Tz: TimeZone>(store: &Store, book: &PriceBook, at_ms: i64, days: u32, now_ms: i64, filter: &Filter, tz: &Tz) -> Result<Experiment> {
    let filter = Filter { tools: filter.tools.clone(), projects: filter.projects.clone(), ..Default::default() };
    Ok(Experiment {
        before: experiment_side(store, book, Range { from_ms: shift_days(tz, at_ms, -i64::from(days)), to_ms: at_ms }, &filter)?,
        after: experiment_side(store, book, Range { from_ms: at_ms, to_ms: shift_days(tz, at_ms, i64::from(days)).min(now_ms) }, &filter)?,
    })
}

/// `ms` moved by whole local calendar days, to the same local time: a day of 23 or 25 hours is
/// still one day. A time the clock skips on the day it lands on falls back to 24-hour days.
fn shift_days<Tz: TimeZone>(tz: &Tz, ms: i64, days: i64) -> i64 {
    let fixed = ms.saturating_add(days.saturating_mul(86_400_000));
    let Some(local) = tz.timestamp_millis_opt(ms).single().map(|d| d.naive_local()) else { return fixed };
    let moved = if days >= 0 { local.checked_add_days(Days::new(days.unsigned_abs())) } else { local.checked_sub_days(Days::new(days.unsigned_abs())) };
    moved.and_then(|d| tz.from_local_datetime(&d).earliest()).map_or(fixed, |t| t.timestamp_millis())
}

/// The starting load of each project, per tool and where it runs, from the conversations that
/// began in `range`; the projects with the most conversations first. A conversation's first
/// request is its first main-conversation one: subagents, advisors and captured telemetry (which
/// also holds helper requests) are left out.
pub fn starting_loads(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<Vec<StartingLoad>> {
    let firsts = first_requests(store, range, filter)?;
    type Key<'a> = (Option<i64>, &'a str, Option<String>);
    let mut groups: BTreeMap<Key, Vec<&EventRow>> = BTreeMap::new();
    for e in &firsts {
        groups.entry((e.project_id, e.tool.as_str(), e.client.clone())).or_default().push(e);
    }
    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    let mut out: Vec<StartingLoad> = groups
        .into_iter()
        .map(|((project_id, _, client), evs)| {
            let all: Vec<Start> = evs.iter().map(|e| Start { ts_ms: e.ts_ms, tokens: e.request_input }).collect();
            let recent = &evs[evs.len().saturating_sub(START_RECENT)..];
            let mut costs: Vec<f64> = recent.iter().filter_map(|e| book.cost(&input_of(e)).map(|c| c.total())).collect();
            costs.sort_by(f64::total_cmp);
            let (project, hidden) = project_label(&projects, project_id);
            StartingLoad {
                project_id,
                project,
                hidden,
                tool: evs[0].tool,
                client,
                conversations: all.len() as u64,
                typical: median(&recent.iter().map(|e| e.request_input).collect::<Vec<_>>()),
                smallest: all.iter().map(|s| s.tokens).min().unwrap_or(0),
                cost_usd: costs.get(costs.len().saturating_sub(1) / 2).copied(),
                change: start_change(&all),
                starts: all[all.len().saturating_sub(START_POINTS)..].to_vec(),
            }
        })
        .collect();
    out.sort_by(|a, b| b.conversations.cmp(&a.conversations).then_with(|| b.typical.cmp(&a.typical)));
    Ok(out)
}

/// Prompt size per request (`request_input`: uncached input + cache read + cache write).
pub fn context_stats<Tz: TimeZone>(store: &Store, book: &PriceBook, range: Range, filter: &Filter, tz: &Tz) -> Result<ContextStats> {
    let events: Vec<EventRow> =
        store.events_between(range.from_ms, range.to_ms)?.into_iter().filter(|e| filter.matches(e) && e.request_input > 0).collect();
    let mut out = ContextStats::default();
    let mut edges = vec![0u64];
    edges.extend(CONTEXT_EDGES);
    out.buckets = edges
        .iter()
        .enumerate()
        .map(|(i, &from)| ContextBucket { from, to: edges.get(i + 1).copied(), requests: 0, cost_usd: 0.0 })
        .collect();
    let mut all = Vec::with_capacity(events.len());
    let mut per_model: BTreeMap<String, (Vec<u64>, u64, f64)> = BTreeMap::new();
    let mut per_day: BTreeMap<NaiveDate, Vec<u64>> = BTreeMap::new();
    for e in &events {
        let size = e.request_input;
        all.push(size);
        let i = input_of(e);
        let cost = book.cost(&i).map(|c| c.total()).unwrap_or(0.0);
        let extra = book.long_context_premium(&i).unwrap_or(0.0);
        let b = out.buckets.iter_mut().rev().find(|b| size >= b.from).expect("first bucket starts at 0");
        b.requests += 1;
        b.cost_usd += cost;
        let m = per_model.entry(e.model.clone()).or_default();
        m.0.push(size);
        if book.is_long_context(&i) {
            m.1 += 1;
            out.long_context_requests += 1;
        }
        m.2 += extra;
        out.long_context_extra_usd += extra;
        per_day.entry(local_date(tz, e.ts_ms)).or_default().push(size);
    }
    all.sort_unstable();
    out.requests = all.len() as u64;
    if !all.is_empty() {
        out.avg = all.iter().sum::<u64>() as f64 / all.len() as f64;
        out.median = percentile(&all, 50.0);
        out.p90 = percentile(&all, 90.0);
        out.p99 = percentile(&all, 99.0);
        out.max = *all.last().unwrap_or(&0);
    }
    out.by_model = per_model
        .into_iter()
        .map(|(model, (mut v, over, extra))| {
            v.sort_unstable();
            let threshold = book.lookup(&model).and_then(|p| p.long_context.as_ref()).map(|lc| lc.threshold);
            ContextByModel {
                requests: v.len() as u64,
                avg: v.iter().sum::<u64>() as f64 / v.len().max(1) as f64,
                p90: percentile(&v, 90.0),
                max: *v.last().unwrap_or(&0),
                threshold,
                over_threshold: over,
                long_context_extra_usd: extra,
                model,
            }
        })
        .collect();
    out.by_model.sort_by(|a, b| b.requests.cmp(&a.requests).then(a.model.cmp(&b.model)));
    out.daily = per_day
        .into_iter()
        .map(|(d, mut v)| {
            v.sort_unstable();
            ContextDay {
                date: d.format("%Y-%m-%d").to_string(),
                requests: v.len() as u64,
                avg: v.iter().sum::<u64>() as f64 / v.len() as f64,
                p90: percentile(&v, 90.0),
                max: *v.last().unwrap_or(&0),
            }
        })
        .collect();
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderValue {
    pub provider: Provider,
    pub cost_usd: f64,
    pub events: u64,
    pub unpriced_events: u64,
    /// Running total of the API-equivalent cost per local day of the range.
    pub cumulative: Vec<f64>,
    /// Each account's share while the provider has several; `account` is `None` for use no
    /// evidence ties to one.
    pub accounts: Vec<AccountValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountValue {
    pub account: Option<String>,
    pub cost_usd: f64,
    pub events: u64,
    pub unpriced_events: u64,
    pub cumulative: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanValue {
    pub range: Range,
    pub dates: Vec<String>,
    pub providers: Vec<ProviderValue>,
}

/// A Claude Code conversation whose prompt cache still lasts.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LiveCache {
    pub session_id: String,
    pub project_id: Option<i64>,
    pub branch: Option<String>,
    pub model: String,
    /// How the conversation runs: "cli", "claude-desktop", "claude-vscode" and so on.
    pub client: Option<String>,
    /// The lifetime it was written with: 5 minutes or an hour.
    pub ttl_ms: i64,
    pub expires_ms: i64,
    /// The conversation's first request, which tells apart two of one project on the same branch.
    pub started_ms: i64,
    /// The prompt the last request left in the cache: what the next request reads.
    pub tokens: u64,
    /// API-equivalent cost of reading the cache once, and of writing it again after it runs out;
    /// `None` for a model without a price.
    pub refresh_usd: Option<f64>,
    pub rewrite_usd: Option<f64>,
}

const FIVE_MINUTES: i64 = 5 * 60_000;
const ONE_HOUR: i64 = 60 * 60_000;

/// A cache written partly for five minutes loses that part first.
pub(crate) fn cache_ttl_ms(written: u64, written_1h: u64) -> i64 {
    if written > written_1h { FIVE_MINUTES } else { ONE_HOUR }
}

/// Every Claude Code conversation whose cache still lasts, the soonest to run out first. A request
/// that reads a cache renews it, so it runs out its lifetime after the conversation's last request
/// that used it; one that wrote both lifetimes loses its five-minute part first. Subagents and
/// advisors keep caches of their own and are left out.
pub fn live_caches(store: &Store, book: &PriceBook, now_ms: i64) -> Result<Vec<LiveCache>> {
    let events = store.events_between(now_ms - ONE_HOUR, now_ms + 1)?;
    let mut sessions: BTreeMap<&str, Vec<&EventRow>> = BTreeMap::new();
    // captured telemetry also holds helper requests and does not tell the two cache lifetimes apart
    for e in events.iter().filter(|e| e.tool == Tool::ClaudeCode && e.agent.is_none() && !e.advisor && e.accuracy != Accuracy::Captured) {
        if let Some(s) = e.session_id.as_deref() {
            sessions.entry(s).or_default().push(e);
        }
    }
    let mut out = Vec::new();
    for (session_id, evs) in sessions {
        // a failed request logs no tokens and leaves the cache as it was
        let Some(last) = evs.iter().filter(|e| e.tokens.cache_read + e.tokens.cache_write > 0).max_by_key(|e| e.ts_ms) else { continue };
        // the lifetime is the one the cache was last written with, however long reads kept it since
        let Some((written, written_1h)) = store.last_cache_write(session_id, &last.model, last.ts_ms)? else { continue };
        let ttl_ms = cache_ttl_ms(written, written_1h);
        let expires_ms = last.ts_ms + ttl_ms;
        if expires_ms <= now_ms {
            continue;
        }
        let tokens = last.tokens.cache_read + last.tokens.cache_write;
        let cost = |t: crate::model::Tokens| book.cost(&CostInput { tokens: &t, request_input: tokens, web_search_requests: 0, ..input_of(last) }).map(|c| c.total());
        let one_hour = if ttl_ms == ONE_HOUR { tokens } else { 0 };
        out.push(LiveCache {
            session_id: session_id.to_owned(),
            project_id: last.project_id,
            branch: last.branch.clone(),
            model: last.model.clone(),
            client: last.client.clone(),
            ttl_ms,
            expires_ms,
            started_ms: store.session_started_ms(session_id)?.unwrap_or(last.ts_ms),
            tokens,
            refresh_usd: cost(crate::model::Tokens { cache_read: tokens, ..Default::default() }),
            rewrite_usd: cost(crate::model::Tokens { cache_write: tokens, cache_write_1h: one_hour, ..Default::default() }),
        });
    }
    out.sort_by(|a, b| a.expires_ms.cmp(&b.expires_ms).then_with(|| a.session_id.cmp(&b.session_id)));
    Ok(out)
}

/// Prompt cache misses Claude Code reported over a period: in all, day by day and project by project.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheMisses {
    /// Whether Claude Code has reported any here at all, in any period.
    pub reported: bool,
    /// Conversations with a miss in the period.
    pub sessions: u64,
    pub misses: u64,
    pub rebuilds: u64,
    pub recached_tokens: u64,
    /// How many misses had each cause, the most frequent first; one miss can have several.
    pub causes: Vec<MissCause>,
    /// Misses Claude Code gave no cause for.
    pub undetermined: u64,
    /// The days with a miss, the earliest first.
    pub days: Vec<MissDay>,
    /// The projects with a miss or a rebuild, the most misses first.
    pub projects: Vec<MissProject>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MissCause {
    pub cause: String,
    pub misses: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MissDay {
    /// The local date, `YYYY-MM-DD`.
    pub date: String,
    pub causes: Vec<MissCause>,
    pub undetermined: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MissProject {
    pub project_id: Option<i64>,
    pub project: String,
    pub hidden: bool,
    pub misses: u64,
    pub rebuilds: u64,
    pub recached_tokens: u64,
    /// The latest miss, or the latest rebuild when there was none.
    pub last_ms: i64,
    /// The cause behind most of its misses; `None` when that is none Claude Code could tell, or
    /// when it had rebuilds alone.
    pub cause: Option<String>,
    /// Its misses and rebuilds as they came, the latest first, at most [`MISS_EVENTS`].
    pub events: Vec<MissEvent>,
    /// How many more there were.
    pub more: u64,
}

/// What happened to a conversation's cache between two reports: usually one miss or one rebuild.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MissEvent {
    pub ts_ms: i64,
    pub session_id: String,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub misses: u64,
    pub rebuilds: u64,
    pub recached_tokens: u64,
    /// The causes Claude Code gave; empty when it could not tell, or for a rebuild alone.
    pub causes: Vec<String>,
}

/// The most misses a project lists one by one.
pub const MISS_EVENTS: usize = 50;
/// A report comes with the next status-line update after the miss; one this much later still
/// tells about a miss in the period.
const MISS_REPORT_LAG_MS: i64 = 7 * 86_400_000;
/// How far the log's time of a request may lie from the time Claude Code gives its miss.
const MISS_NEAR_MS: i64 = 60_000;
/// Requests this near a miss's time cannot be told apart as the one that missed.
const MISS_DRIFT_MS: i64 = 5_000;

/// The counts one window on a conversation has told so far: its latest report, and whether the
/// counts were seen at nothing after it, which ends the run.
struct MissRun<'a> {
    head: &'a crate::model::CacheMissReport,
    closed: bool,
}

/// The earlier report `r` goes on from. Two windows can hold one conversation, each with counts
/// of its own, and a conversation resumed begins its counts again; nothing in a report says which
/// it is, so a report joins the run it can be the next step of, and begins a new one when there
/// is none. `Some(None)` is a new run; `None` a state already told, which adds nothing.
fn miss_base<'a>(runs: &mut Vec<MissRun<'a>>, high: Option<i64>, r: &'a crate::model::CacheMissReport) -> Option<Option<&'a crate::model::CacheMissReport>> {
    // counts only go up within a run, each cause's and the rewritten tokens too: a report that
    // lacks what an earlier one told is another window's
    let no_fewer = |h: &crate::model::CacheMissReport| {
        let causes = h.causes.iter().all(|(c, n)| r.causes.iter().any(|(rc, rn)| rc == c && rn >= n));
        h.misses <= r.misses && h.rebuilds <= r.rebuilds && h.recached_tokens <= r.recached_tokens && causes
    };
    // misses told before: their latest is no later than one already told
    let told = r.misses > 0 && matches!((r.last_miss_ms, high), (Some(l), Some(h)) if l <= h);
    let fits = |run: &MissRun| {
        let h = run.head;
        if told {
            // the same run going on (a rebuild since, or the report once more)
            h.last_miss_ms == r.last_miss_ms && no_fewer(h)
        } else if r.misses == 0 {
            // rebuilds alone: no miss tells a new run from the old one, so none is assumed
            h.misses == 0 && no_fewer(h)
        } else if r.last_miss_ms.is_some() && h.last_miss_ms.is_some() {
            // a later miss is one more miss
            !run.closed && h.misses < r.misses && no_fewer(h)
        } else {
            !run.closed && no_fewer(h)
        }
    };
    // of several it could go on from, the one that leaves the fewest new: never more than there were
    let found = runs.iter().enumerate().filter(|(_, run)| fits(run)).max_by_key(|(_, run)| (run.head.misses, run.head.rebuilds)).map(|(i, _)| i);
    match found {
        Some(i) => {
            let base = runs[i].head;
            runs[i] = MissRun { head: r, closed: false };
            Some(Some(base))
        }
        None if told => None,
        None => {
            runs.push(MissRun { head: r, closed: false });
            Some(None)
        }
    }
}

/// The misses Claude Code reported, each at the time it happened and counted when the request
/// that missed (the main conversation's nearest to that time) passes the filter. Two reports in a row tell what happened in between; counts that start over begin a
/// new run.
pub fn cache_misses<Tz: TimeZone>(store: &Store, range: Range, filter: &Filter, tz: &Tz) -> Result<CacheMisses> {
    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    let mut out = CacheMisses { reported: store.has_cache_misses()?, ..CacheMisses::default() };
    let mut causes: BTreeMap<String, u64> = BTreeMap::new();
    let mut days: BTreeMap<NaiveDate, (BTreeMap<String, u64>, u64)> = BTreeMap::new();
    struct Acc {
        project: String,
        hidden: bool,
        misses: u64,
        rebuilds: u64,
        recached: u64,
        last_miss: Option<i64>,
        last: i64,
        causes: BTreeMap<String, u64>,
        undetermined: u64,
        events: Vec<MissEvent>,
    }
    let mut by_project: HashMap<Option<i64>, Acc> = HashMap::new();
    // a miss in the period is reported at or after it
    let reports = store.cache_miss_reports(range.from_ms, range.to_ms.saturating_add(MISS_REPORT_LAG_MS))?;
    for group in reports.chunk_by(|a, b| a.session_id == b.session_id) {
        let session_id = &group[0].session_id;
        let events: Vec<EventRow> = store.session_events(session_id)?.into_iter().filter(|e| e.tool == Tool::ClaudeCode).collect();
        // a conversation none of whose requests is in the archive has no project, model or account to count under
        if !events.iter().any(|e| filter.matches(e)) {
            continue;
        }
        let mut counts: HashMap<Option<i64>, u64> = HashMap::new();
        for e in &events {
            *counts.entry(e.project_id).or_default() += 1;
        }
        let project_id = counts.iter().max_by_key(|(id, n)| (**n, id.unwrap_or(i64::MIN))).and_then(|(id, _)| *id);
        // a miss is the main conversation's: a subagent's request tells nothing about it
        let main: Vec<&EventRow> = events.iter().filter(|e| e.agent.is_none() && !e.advisor).collect();
        let mut missed = false;
        let mut runs: Vec<MissRun> = Vec::new();
        // the latest miss told so far
        let mut high: Option<i64> = None;
        for r in group {
            if r.misses == 0 && r.rebuilds == 0 {
                runs.iter_mut().for_each(|run| run.closed = true);
                continue;
            }
            let Some(base) = miss_base(&mut runs, high, r) else { continue };
            high = high.max(r.last_miss_ms);
            let (misses, rebuilds) = (r.misses - base.map_or(0, |p| p.misses), r.rebuilds - base.map_or(0, |p| p.rebuilds));
            let at = if misses > 0 { r.last_miss_ms.unwrap_or(r.ts_ms) } else { r.ts_ms };
            if misses + rebuilds == 0 || at < range.from_ms || at >= range.to_ms {
                continue;
            }
            // Claude Code names the miss to the second and the log stamps the response on its own,
            // so any main request within a few seconds can be the one that missed. The latest of
            // the miss's own second is shown, else the nearest one within a minute, else the last before.
            let around: Vec<&&EventRow> = main.iter().filter(|e| (e.ts_ms - at).abs() <= MISS_DRIFT_MS).collect();
            let near = main
                .iter()
                .rev()
                .find(|e| (at..at.saturating_add(1000)).contains(&e.ts_ms))
                .or_else(|| main.iter().filter(|e| (e.ts_ms - at).abs() <= MISS_NEAR_MS).min_by_key(|e| (e.ts_ms - at).abs()))
                .or_else(|| main.iter().rev().find(|e| e.ts_ms <= at))
                .or(main.first());
            // a conversation can change model or account: a miss of a request the filter leaves out
            // is not its, and one the requests around it disagree on is counted under no filter
            let kept = if around.is_empty() { near.is_some_and(|e| filter.matches(e)) } else { around.iter().all(|e| filter.matches(e)) };
            if !filter.is_empty() && !kept {
                continue;
            }
            let before = |cause: &str| base.and_then(|p| p.causes.iter().find(|(c, _)| c == cause)).map_or(0, |(_, n)| *n);
            // Claude Code's own name for a miss it found no cause for is counted with the rest of those
            let new: Vec<(String, u64)> = r.causes.iter().filter(|(c, _)| c != "unknown").filter_map(|(c, n)| Some((c.clone(), n.checked_sub(before(c)).filter(|d| *d > 0)?))).collect();
            let recached = r.recached_tokens.saturating_sub(base.map_or(0, |p| p.recached_tokens));
            let undetermined = misses.saturating_sub(new.iter().map(|(_, n)| n).sum());
            let acc = by_project.entry(project_id).or_insert_with(|| {
                let (project, hidden) = project_label(&projects, project_id);
                Acc { project, hidden, misses: 0, rebuilds: 0, recached: 0, last_miss: None, last: 0, causes: BTreeMap::new(), undetermined: 0, events: Vec::new() }
            });
            acc.misses += misses;
            acc.rebuilds += rebuilds;
            acc.recached += recached;
            acc.undetermined += undetermined;
            acc.last = acc.last.max(at);
            if misses > 0 {
                acc.last_miss = Some(acc.last_miss.map_or(at, |l| l.max(at)));
                missed = true;
                let day = days.entry(local_date(tz, at)).or_default();
                day.1 += undetermined;
                for (c, n) in &new {
                    *day.0.entry(c.clone()).or_default() += n;
                }
            }
            for (c, n) in &new {
                *causes.entry(c.clone()).or_default() += n;
                *acc.causes.entry(c.clone()).or_default() += n;
            }
            out.misses += misses;
            out.rebuilds += rebuilds;
            out.recached_tokens += recached;
            out.undetermined += undetermined;
            acc.events.push(MissEvent {
                ts_ms: at,
                session_id: session_id.clone(),
                branch: near.and_then(|e| e.branch.clone()),
                model: near.map(|e| e.model.clone()),
                misses,
                rebuilds,
                recached_tokens: recached,
                causes: new.into_iter().map(|(c, _)| c).collect(),
            });
        }
        out.sessions += u64::from(missed);
    }
    let by_count = |m: BTreeMap<String, u64>| {
        let mut v: Vec<MissCause> = m.into_iter().map(|(cause, misses)| MissCause { cause, misses }).collect();
        v.sort_by(|a, b| b.misses.cmp(&a.misses).then_with(|| a.cause.cmp(&b.cause)));
        v
    };
    out.causes = by_count(causes);
    out.days = days.into_iter().map(|(d, (c, undetermined))| MissDay { date: d.format("%Y-%m-%d").to_string(), causes: by_count(c), undetermined }).collect();
    out.projects = by_project
        .into_iter()
        .map(|(project_id, mut a)| {
            a.events.sort_by(|x, y| y.ts_ms.cmp(&x.ts_ms).then_with(|| x.session_id.cmp(&y.session_id)));
            let more = a.events.len().saturating_sub(MISS_EVENTS) as u64;
            a.events.truncate(MISS_EVENTS);
            let top = by_count(a.causes).into_iter().next().filter(|c| c.misses >= a.undetermined);
            MissProject {
                project_id,
                project: a.project,
                hidden: a.hidden,
                misses: a.misses,
                rebuilds: a.rebuilds,
                recached_tokens: a.recached,
                last_ms: a.last_miss.unwrap_or(a.last),
                cause: top.map(|c| c.cause),
                events: a.events,
                more,
            }
        })
        .collect();
    out.projects.sort_by(|a, b| b.misses.cmp(&a.misses).then(b.last_ms.cmp(&a.last_ms)).then(a.project.cmp(&b.project)));
    Ok(out)
}

/// API-equivalent cost per provider over `range`, with a daily running total (unfiltered).
/// Over 401 days only the newest are shown; earlier use starts the first day's running total.
pub fn plan_value<Tz: TimeZone>(store: &Store, book: &PriceBook, range: Range, tz: &Tz) -> Result<PlanValue> {
    let last = local_date(tz, range.to_ms - 1);
    let first = local_date(tz, range.from_ms).max(last.checked_sub_days(chrono::Days::new(400)).unwrap_or(last));
    let mut dates = Vec::new();
    let mut d = first;
    while d <= last {
        dates.push(d);
        let Some(next) = d.succ_opt() else { break };
        d = next;
    }
    let idx: HashMap<NaiveDate, usize> = dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let several = crate::accounts::several(store)?;
    let mut acc: BTreeMap<Provider, (Totals, Vec<f64>)> = BTreeMap::new();
    let mut by_account: BTreeMap<(Provider, Option<String>), (Totals, Vec<f64>)> = BTreeMap::new();
    for e in store.events_between(range.from_ms, range.to_ms)? {
        // a plan's value counts only usage known to go through that plan
        let Some(p) = e.tool.provider() else { continue };
        let date = local_date(tz, e.ts_ms);
        let i = idx.get(&date).copied().or_else(|| (date < first && !dates.is_empty()).then_some(0));
        let cost = book.cost(&input_of(&e)).map(|c| c.total());
        let tally = |entry: &mut (Totals, Vec<f64>)| {
            add(&mut entry.0, book, &e);
            if let (Some(c), Some(i)) = (cost, i) {
                entry.1[i] += c;
            }
        };
        if several.contains(&p) {
            tally(by_account.entry((p, e.account.clone())).or_insert_with(|| (Totals::default(), vec![0.0; dates.len()])));
        }
        tally(acc.entry(p).or_insert_with(|| (Totals::default(), vec![0.0; dates.len()])));
    }
    let running = |daily: Vec<f64>| -> Vec<f64> {
        let mut run = 0.0;
        daily
            .into_iter()
            .map(|v| {
                run += v;
                run
            })
            .collect()
    };
    let providers = acc
        .into_iter()
        .map(|(provider, (t, daily))| {
            let accounts = by_account
                .iter()
                .filter(|((p, _), _)| *p == provider)
                .map(|((_, account), (t, daily))| AccountValue {
                    account: account.clone(),
                    cost_usd: t.cost_usd,
                    events: t.events,
                    unpriced_events: t.unpriced_events,
                    cumulative: running(daily.clone()),
                })
                .collect();
            ProviderValue { provider, cost_usd: t.cost_usd, events: t.events, unpriced_events: t.unpriced_events, cumulative: running(daily), accounts }
        })
        .collect();
    Ok(PlanValue { range, dates: dates.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect(), providers })
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchRow {
    pub project_id: Option<i64>,
    pub project: String,
    pub hidden: bool,
    /// `None`: outside a repository, or the tool did not report a branch.
    pub branch: Option<String>,
    /// The project's name is hidden, so its branch names are too (they can name clients or
    /// features); set by the caller that masks names.
    pub branch_hidden: bool,
    pub totals: Totals,
    pub sessions: u64,
    pub tools: Vec<Tool>,
    pub first_ms: i64,
    pub last_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Branches {
    /// Most expensive first.
    pub rows: Vec<BranchRow>,
    pub events_with_branch: u64,
    pub events_without_branch: u64,
}

/// Usage per project and git branch. The same branch name in two projects is two rows.
pub fn branches(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<Branches> {
    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    struct Acc {
        totals: Totals,
        sessions: std::collections::HashSet<String>,
        tools: std::collections::BTreeSet<&'static str>,
        first: i64,
        last: i64,
    }
    let mut map: HashMap<(Option<i64>, Option<String>), Acc> = HashMap::new();
    let (mut with, mut without) = (0u64, 0u64);
    for e in store.events_between(range.from_ms, range.to_ms)?.iter().filter(|e| filter.matches(e)) {
        if e.branch.is_some() {
            with += 1;
        } else {
            without += 1;
        }
        let a = map.entry((e.project_id, e.branch.clone())).or_insert_with(|| Acc {
            totals: Totals::default(),
            sessions: Default::default(),
            tools: Default::default(),
            first: e.ts_ms,
            last: e.ts_ms,
        });
        add(&mut a.totals, book, e);
        if let Some(s) = &e.session_id {
            a.sessions.insert(format!("{}:{s}", e.tool.as_str()));
        }
        a.tools.insert(e.tool.as_str());
        a.first = a.first.min(e.ts_ms);
        a.last = a.last.max(e.ts_ms);
    }
    let mut rows: Vec<BranchRow> = map
        .into_iter()
        .map(|((project_id, branch), a)| {
            let (project, hidden) = project_label(&projects, project_id);
            BranchRow {
                project_id,
                project,
                hidden,
                branch,
                branch_hidden: false,
                sessions: a.sessions.len() as u64,
                tools: a.tools.iter().filter_map(|t| Tool::parse(t)).collect(),
                first_ms: a.first,
                last_ms: a.last,
                totals: a.totals,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.totals
            .cost_usd
            .total_cmp(&a.totals.cost_usd)
            .then(b.totals.total_tokens.cmp(&a.totals.total_tokens))
            .then(a.project.cmp(&b.project))
            .then(a.branch.cmp(&b.branch))
    });
    Ok(Branches { rows, events_with_branch: with, events_without_branch: without })
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRow {
    pub tool: Tool,
    /// `None`: the main conversation.
    pub agent: Option<String>,
    pub totals: Totals,
    /// Separate subagent runs (conversations); 0 for the main conversation.
    pub runs: u64,
    pub sessions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolRow {
    pub tool: Tool,
    pub name: String,
    pub calls: u64,
    /// Calls whose outcome the log records.
    pub known: u64,
    pub failed: u64,
    pub by_subagents: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentsTools {
    /// Per tool, main conversation first, then subagent types by cost.
    pub agents: Vec<AgentRow>,
    /// Most used first.
    pub tools: Vec<ToolRow>,
    pub tool_calls: u64,
    /// The model or client filter applies to tool calls through their sessions.
    pub filtered_by_session: bool,
}

pub fn agents_tools(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<AgentsTools> {
    let events: Vec<EventRow> = store.events_between(range.from_ms, range.to_ms)?.into_iter().filter(|e| filter.matches(e)).collect();
    struct Acc {
        totals: Totals,
        runs: std::collections::HashSet<String>,
        sessions: std::collections::HashSet<String>,
    }
    let mut acc: BTreeMap<(&'static str, Option<String>), Acc> = BTreeMap::new();
    for e in &events {
        let a = acc
            .entry((e.tool.as_str(), e.agent.clone()))
            .or_insert_with(|| Acc { totals: Totals::default(), runs: Default::default(), sessions: Default::default() });
        add(&mut a.totals, book, e);
        if let Some(t) = &e.thread_id {
            a.runs.insert(t.clone());
        }
        if let Some(s) = &e.session_id {
            a.sessions.insert(s.clone());
        }
    }
    let mut agents: Vec<AgentRow> = acc
        .into_iter()
        .filter_map(|((tool, agent), a)| {
            Some(AgentRow { tool: Tool::parse(tool)?, runs: a.runs.len() as u64, sessions: a.sessions.len() as u64, agent, totals: a.totals })
        })
        .collect();
    agents.sort_by(|a, b| {
        a.tool
            .as_str()
            .cmp(b.tool.as_str())
            .then(a.agent.is_some().cmp(&b.agent.is_some()))
            .then(b.totals.cost_usd.total_cmp(&a.totals.cost_usd))
            .then(b.totals.total_tokens.cmp(&a.totals.total_tokens))
    });

    // tool calls carry no model, client or account: with such a filter, keep the calls of the
    // sessions the filtered requests belong to
    let by_session = !filter.models.is_empty() || !filter.clients.is_empty() || !filter.accounts.is_empty();
    let sessions: std::collections::HashSet<(&'static str, &str)> =
        events.iter().filter_map(|e| e.session_id.as_deref().map(|s| (e.tool.as_str(), s))).collect();
    let mut tools: BTreeMap<(&'static str, String), ToolRow> = BTreeMap::new();
    let mut total = 0u64;
    for c in store.tool_calls_between(range.from_ms, range.to_ms)? {
        if !filter.tools.is_empty() && !filter.tools.contains(&c.tool) {
            continue;
        }
        if !filter.projects.is_empty() && !c.project_id.is_some_and(|p| filter.projects.contains(&p)) {
            continue;
        }
        if by_session && !c.session_id.as_deref().is_some_and(|s| sessions.contains(&(c.tool.as_str(), s))) {
            continue;
        }
        total += 1;
        let r = tools.entry((c.tool.as_str(), c.name.clone())).or_insert_with(|| ToolRow {
            tool: c.tool,
            name: c.name.clone(),
            calls: 0,
            known: 0,
            failed: 0,
            by_subagents: 0,
        });
        r.calls += 1;
        if let Some(f) = c.failed {
            r.known += 1;
            r.failed += u64::from(f);
        }
        if c.agent.is_some() {
            r.by_subagents += 1;
        }
    }
    let mut tools: Vec<ToolRow> = tools.into_values().collect();
    tools.sort_by(|a, b| b.calls.cmp(&a.calls).then(a.tool.as_str().cmp(b.tool.as_str())).then(a.name.cmp(&b.name)));
    Ok(AgentsTools { agents, tools, tool_calls: total, filtered_by_session: by_session })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PromptShare {
    pub requests: u64,
    pub prompt: u64,
    pub cache_read: u64,
}

impl PromptShare {
    fn add(&mut self, t: &crate::model::Tokens) {
        self.requests += 1;
        self.prompt = self.prompt.saturating_add(t.input).saturating_add(t.cache_read).saturating_add(t.cache_write);
        self.cache_read = self.cache_read.saturating_add(t.cache_read);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheAgentRow {
    pub tool: Tool,
    pub agent: Option<String>,
    pub totals: Totals,
    pub runs: u64,
    pub first: PromptShare,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentCache {
    pub main: Totals,
    pub subagents: Totals,
    pub runs: u64,
    pub sessions: u64,
    pub first: PromptShare,
    /// Main requests in sessions with subagents: the first one after each run, and the rest.
    pub main_after_run: PromptShare,
    pub main_other: PromptShare,
    pub previous_main: PromptShare,
    pub previous_subagents: PromptShare,
    pub rows: Vec<CacheAgentRow>,
}

type SessionKey<'a> = (&'static str, &'a str);

pub fn subagent_cache<Tz: TimeZone>(store: &Store, book: &PriceBook, range: Range, filter: &Filter, tz: &Tz) -> Result<SubagentCache> {
    let events: Vec<EventRow> = store.events_between(range.from_ms, range.to_ms)?.into_iter().filter(|e| filter.matches(e)).collect();
    let (mut main, mut subagents) = (Totals::default(), Totals::default());
    struct Acc {
        totals: Totals,
        runs: std::collections::HashSet<String>,
        first: PromptShare,
    }
    let mut acc: BTreeMap<(&'static str, Option<String>), Acc> = BTreeMap::new();
    // events come sorted by time, so the last one seen ends the run
    let mut runs: HashMap<SessionKey, (SessionKey, i64)> = HashMap::new();
    let mut first = PromptShare::default();
    // reuse as the Cache page counts it
    let writers = crate::analytics::cache_writers(events.iter());
    let add = |t: &mut Totals, e: &EventRow| {
        let i = input_of(e);
        t.add_counted(e, book.cost(&i).as_ref(), book.cache_savings(&i).unwrap_or(0.0), writers(e));
    };
    for e in &events {
        let a = acc
            .entry((e.tool.as_str(), e.agent.clone()))
            .or_insert_with(|| Acc { totals: Totals::default(), runs: Default::default(), first: PromptShare::default() });
        add(&mut a.totals, e);
        if e.agent.is_none() {
            add(&mut main, e);
            continue;
        }
        add(&mut subagents, e);
        let Some(thread) = e.thread_id.as_deref() else { continue };
        if a.runs.insert(thread.to_owned()) {
            a.first.add(&e.tokens);
            first.add(&e.tokens);
        }
        if let Some(session) = e.session_id.as_deref() {
            runs.entry((e.tool.as_str(), thread)).or_insert(((e.tool.as_str(), session), e.ts_ms)).1 = e.ts_ms;
        }
    }

    // the conversation's own requests; an advisor's prompt and cache are its own
    let mut main_by_session: HashMap<SessionKey, Vec<&EventRow>> = HashMap::new();
    for e in events.iter().filter(|e| e.agent.is_none() && !e.advisor) {
        if let Some(s) = e.session_id.as_deref() {
            main_by_session.entry((e.tool.as_str(), s)).or_default().push(e);
        }
    }
    let mut after: std::collections::HashSet<(&'static str, &str, usize)> = std::collections::HashSet::new();
    let mut sessions: std::collections::HashSet<SessionKey> = std::collections::HashSet::new();
    for (session, end) in runs.values() {
        sessions.insert(*session);
        if let Some(evs) = main_by_session.get(session) {
            let i = evs.partition_point(|e| e.ts_ms <= *end);
            if i < evs.len() {
                after.insert((session.0, session.1, i));
            }
        }
    }
    let (mut main_after_run, mut main_other) = (PromptShare::default(), PromptShare::default());
    for (key, evs) in &main_by_session {
        if !sessions.contains(key) {
            continue;
        }
        for (i, e) in evs.iter().enumerate() {
            if after.contains(&(key.0, key.1, i)) { main_after_run.add(&e.tokens) } else { main_other.add(&e.tokens) }
        }
    }

    let prev = range.previous(tz);
    let (mut previous_main, mut previous_subagents) = (PromptShare::default(), PromptShare::default());
    for e in store.events_between(prev.from_ms, prev.to_ms)?.iter().filter(|e| filter.matches(e)) {
        if e.agent.is_none() { previous_main.add(&e.tokens) } else { previous_subagents.add(&e.tokens) }
    }

    let prompt = |t: &Totals| t.tokens.input + t.tokens.cache_read + t.tokens.cache_write;
    let mut rows: Vec<CacheAgentRow> = acc
        .into_iter()
        .filter_map(|((tool, agent), a)| Some(CacheAgentRow { tool: Tool::parse(tool)?, runs: a.runs.len() as u64, agent, first: a.first, totals: a.totals }))
        .filter(|r| prompt(&r.totals) > 0)
        .collect();
    rows.sort_by(|a, b| {
        a.tool
            .as_str()
            .cmp(b.tool.as_str())
            .then(a.agent.is_some().cmp(&b.agent.is_some()))
            .then(prompt(&b.totals).cmp(&prompt(&a.totals)))
            .then(a.agent.cmp(&b.agent))
    });
    Ok(SubagentCache {
        runs: rows.iter().map(|r| r.runs).sum(),
        sessions: sessions.len() as u64,
        main,
        subagents,
        first,
        main_after_run,
        main_other,
        previous_main,
        previous_subagents,
        rows,
    })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HourPoint {
    pub hour: u32,
    pub tokens: u64,
    pub cost_usd: f64,
    pub events: u64,
    pub by_tool: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayLimitPeak {
    pub provider: Provider,
    /// The account, while the provider has more than one (as in [`crate::analytics::LimitView`]);
    /// `None` there for readings no evidence ties to one.
    pub account: Option<String>,
    pub window: String,
    pub peak_pct: f64,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayDetail {
    pub date: String,
    pub totals: Totals,
    /// 24 local hours (a day with a DST change still has one entry per clock hour).
    pub hourly: Vec<HourPoint>,
    pub by_tool: Vec<crate::analytics::Group>,
    pub by_model: Vec<crate::analytics::Group>,
    pub by_project: Vec<crate::analytics::Group>,
    pub sessions: u64,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub limit_peaks: Vec<DayLimitPeak>,
}

fn groups(map: HashMap<String, (String, bool, Totals)>) -> Vec<crate::analytics::Group> {
    let mut v: Vec<crate::analytics::Group> =
        map.into_iter().map(|(key, (label, hidden, totals))| crate::analytics::Group { key, label, hidden, totals }).collect();
    v.sort_by(|a, b| {
        b.totals.cost_usd.total_cmp(&a.totals.cost_usd).then(b.totals.total_tokens.cmp(&a.totals.total_tokens)).then(a.key.cmp(&b.key))
    });
    v
}

pub fn day_detail<Tz: TimeZone>(store: &Store, book: &PriceBook, date: NaiveDate, filter: &Filter, tz: &Tz) -> Result<DayDetail> {
    use chrono::Timelike;
    let from = crate::analytics::start_of_day(tz, date);
    let to = crate::analytics::start_of_day(tz, date.succ_opt().unwrap_or(date));
    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    let mut totals = Totals::default();
    let mut hourly: Vec<HourPoint> = (0..24).map(|h| HourPoint { hour: h, ..Default::default() }).collect();
    let (mut by_tool, mut by_model, mut by_project) = (HashMap::new(), HashMap::new(), HashMap::new());
    let mut sessions = std::collections::HashSet::new();
    let (mut first, mut last) = (None::<i64>, None::<i64>);
    let events: Vec<EventRow> = store.events_between(from, to)?.into_iter().filter(|e| filter.matches(e)).collect();
    let writers = crate::analytics::cache_writers(events.iter());
    for e in &events {
        let i = input_of(e);
        let cost = book.cost(&i);
        let saving = book.cache_savings(&i).unwrap_or(0.0);
        let reuse = writers(e);
        totals.add_counted(e, cost.as_ref(), saving, reuse);
        let put = |m: &mut HashMap<String, (String, bool, Totals)>, key: String, label: String, hidden: bool| {
            m.entry(key).or_insert_with(|| (label, hidden, Totals::default())).2.add_counted(e, cost.as_ref(), saving, reuse);
        };
        put(&mut by_tool, e.tool.as_str().into(), e.tool.as_str().into(), false);
        put(&mut by_model, e.model.clone(), e.model.clone(), false);
        match e.project_id.and_then(|id| projects.get(&id)) {
            Some(p) => put(&mut by_project, p.id.to_string(), p.name.clone(), p.hidden),
            None => put(&mut by_project, "none".into(), String::new(), false),
        }
        if let Some(local) = tz.timestamp_millis_opt(e.ts_ms).single() {
            let h = &mut hourly[local.hour() as usize];
            h.tokens += e.tokens.total();
            h.cost_usd += cost.map(|c| c.total()).unwrap_or(0.0);
            h.events += 1;
            *h.by_tool.entry(e.tool.as_str().to_owned()).or_default() += e.tokens.total();
        }
        if let Some(s) = &e.session_id {
            sessions.insert(format!("{}:{s}", e.tool.as_str()));
        }
        first = Some(first.map_or(e.ts_ms, |f| f.min(e.ts_ms)));
        last = Some(last.map_or(e.ts_ms, |l| l.max(e.ts_ms)));
    }
    // each account has its own windows; a provider with one account keeps one peak per window
    let several = crate::accounts::several(store)?;
    let mut peaks: BTreeMap<(Provider, Option<String>, String), (f64, i64)> = BTreeMap::new();
    for (s, owner) in store.owned_limits_between(from, to)? {
        let (Some(u), true) = (s.used_pct, s.window == "five_hour" || s.window == "seven_day") else { continue };
        if !filter.accounts.is_empty() && !filter.accounts.contains(&crate::analytics::account_key(s.provider, owner.as_deref())) {
            continue;
        }
        let account = match several.contains(&s.provider) {
            true if owner.as_deref() == Some(crate::accounts::NAMELESS) => continue,
            true => owner,
            false => None,
        };
        let p = peaks.entry((s.provider, account, s.window.clone())).or_insert((u, s.ts_ms));
        if u > p.0 {
            *p = (u, s.ts_ms);
        }
    }
    Ok(DayDetail {
        date: date.format("%Y-%m-%d").to_string(),
        totals,
        hourly,
        by_tool: groups(by_tool),
        by_model: groups(by_model),
        by_project: groups(by_project),
        sessions: sessions.len() as u64,
        first_ms: first,
        last_ms: last,
        limit_peaks: peaks.into_iter().map(|((provider, account, window), (peak_pct, at_ms))| DayLimitPeak { provider, account, window, peak_pct, at_ms }).collect(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastKind {
    /// At the recent pace the window fills before it resets.
    Fills,
    /// At the recent pace it does not fill before the reset.
    Safe,
    /// No measurable increase over the recent readings.
    Idle,
    /// Too few readings, or too short a span, to say anything.
    Insufficient,
}

#[derive(Debug, Clone, Serialize)]
pub struct Forecast {
    pub kind: ForecastKind,
    /// Percentage points per hour: the window's average pace since it started (or since use
    /// started again from zero inside it, see [`restarted`]).
    pub rate_per_hour: Option<f64>,
    /// When the window would reach 100 % (`Fills` only).
    pub fills_at_ms: Option<i64>,
    /// Where the window would stand at its reset (`Safe` / `Idle`).
    pub at_reset_pct: Option<f64>,
    /// How long the window has been running, or the use since it restarted (the basis of the pace).
    pub basis_minutes: i64,
}

/// A fall below what a reading delayed by its request explains.
const FALL_PCT: f64 = 10.0;
/// How old a reading may be: a request still running reports the limit as it was when it began.
pub(crate) const STALE_MS: i64 = 10 * 60_000;

/// Each time use started again from zero inside a window: the provider granted a reset without
/// moving the window's end. Each is the index of the last reading before a fall that the readings
/// after it hold, with none back up in between, for longer than a delayed reading could explain;
/// use may fill up again later. `reads` are (time, %) of one window, oldest first.
pub(crate) fn restarts(reads: &[(i64, f64)]) -> Vec<usize> {
    (1..reads.len())
        .filter(|&j| {
            let before = reads[j - 1].1;
            let held = reads[j..].iter().take_while(|r| before - r.1 > FALL_PCT).last();
            held.is_some_and(|r| r.0 - reads[j].0 >= STALE_MS)
        })
        .map(|j| j - 1)
        .collect()
}

/// When use last started again from zero inside a window, if it did (see [`restarts`]).
pub fn restarted(reads: &[(i64, f64)]) -> Option<i64> {
    restarts(reads).last().map(|&i| reads[i].0)
}

/// Linear forecast at the window's average pace so far. Every window starts at 0 % when it
/// resets, so `current ÷ time since the window started` is a known average that includes idle
/// hours (nights, breaks); a burst of recent work is not extrapolated as if it never stopped.
/// Before a tenth of the window has passed the pace is not trusted. A reset the provider grants
/// inside the window (`restarted_ms`, see [`restarted`]) starts the pace again from there.
pub fn forecast(now_ms: i64, current: f64, resets_at_ms: i64, window_ms: i64, restarted_ms: Option<i64>) -> Forecast {
    let base = Forecast { kind: ForecastKind::Insufficient, rate_per_hour: None, fills_at_ms: None, at_reset_pct: None, basis_minutes: 0 };
    if window_ms <= 0 || !current.is_finite() {
        return base;
    }
    let started = resets_at_ms.saturating_sub(window_ms).max(restarted_ms.unwrap_or(i64::MIN));
    let elapsed = now_ms.saturating_sub(started).clamp(0, window_ms);
    let left_ms = resets_at_ms.saturating_sub(now_ms).max(0);
    let current = current.clamp(0.0, 100.0);
    let base = Forecast { basis_minutes: elapsed / 60_000, ..base };
    if elapsed < window_ms / 10 {
        return base;
    }
    let per_ms = current / elapsed as f64;
    let base = Forecast { rate_per_hour: Some(per_ms * 3_600_000.0), ..base };
    if current < 0.5 {
        return Forecast { kind: ForecastKind::Idle, at_reset_pct: Some(current), ..base };
    }
    let to_full_ms = ((100.0 - current) / per_ms) as i64;
    if to_full_ms < left_ms {
        Forecast { kind: ForecastKind::Fills, fills_at_ms: Some(now_ms.saturating_add(to_full_ms)), ..base }
    } else {
        Forecast { kind: ForecastKind::Safe, at_reset_pct: Some((current + per_ms * left_ms as f64).min(100.0)), ..base }
    }
}
