//! Deeper views over the archive: sessions, "what would another model cost", context size
//! analysis, plan value and limit forecasts. Everything is computed from stored counts only.

use crate::analytics::{local_date, Filter, Range, Totals};
use crate::model::{Provider, Tool};
use crate::pricing::{CostInput, PriceBook};
use crate::store::{EventRow, ProjectRow, Store};
use chrono::{NaiveDate, TimeZone};
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

// ---------------------------------------------------------------- sessions

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
        m.total_tokens += e.tokens.total();
        match c {
            Some(c) => m.cost_usd += c.total(),
            None => m.unpriced = true,
        }
        if e.request_input > 0 {
            a.max_ctx = a.max_ctx.max(e.request_input);
            a.ctx_sum += e.request_input;
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

// ---------------------------------------------------------------- model comparison

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
    let mut targets: Vec<CompareTarget> = book
        .file()
        .models
        .iter()
        .map(|m| {
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
                provider: m.provider,
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

// ---------------------------------------------------------------- context size

/// Bucket edges for prompt sizes. 200K and 272K are where long-context pricing starts for
/// older Claude models and current OpenAI models.
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

// ---------------------------------------------------------------- plan value

#[derive(Debug, Clone, Serialize)]
pub struct ProviderValue {
    pub provider: Provider,
    pub cost_usd: f64,
    pub events: u64,
    pub unpriced_events: u64,
    /// Running total of the API-equivalent cost per local day of the range.
    pub cumulative: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanValue {
    pub range: Range,
    pub dates: Vec<String>,
    pub providers: Vec<ProviderValue>,
}

/// API-equivalent cost per provider over `range`, with a daily running total (unfiltered).
pub fn plan_value<Tz: TimeZone>(store: &Store, book: &PriceBook, range: Range, tz: &Tz) -> Result<PlanValue> {
    let first = local_date(tz, range.from_ms);
    let last = local_date(tz, range.to_ms - 1);
    let mut dates = Vec::new();
    let mut d = first;
    while d <= last {
        dates.push(d);
        d = d.succ_opt().unwrap_or(d);
        if dates.len() > 400 {
            break;
        }
    }
    let idx: HashMap<NaiveDate, usize> = dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut acc: BTreeMap<Provider, (Totals, Vec<f64>)> = BTreeMap::new();
    for e in store.events_between(range.from_ms, range.to_ms)? {
        let p = e.tool.provider();
        let entry = acc.entry(p).or_insert_with(|| (Totals::default(), vec![0.0; dates.len()]));
        add(&mut entry.0, book, &e);
        if let (Some(c), Some(&i)) = (book.cost(&input_of(&e)), idx.get(&local_date(tz, e.ts_ms))) {
            entry.1[i] += c.total();
        }
    }
    let providers = acc
        .into_iter()
        .map(|(provider, (t, daily))| {
            let mut run = 0.0;
            let cumulative = daily
                .into_iter()
                .map(|v| {
                    run += v;
                    run
                })
                .collect();
            ProviderValue { provider, cost_usd: t.cost_usd, events: t.events, unpriced_events: t.unpriced_events, cumulative }
        })
        .collect();
    Ok(PlanValue { range, dates: dates.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect(), providers })
}

// ---------------------------------------------------------------- limit forecast

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
    /// Percentage points per hour: the window's average pace since it started.
    pub rate_per_hour: Option<f64>,
    /// When the window would reach 100 % (`Fills` only).
    pub fills_at_ms: Option<i64>,
    /// Where the window would stand at its reset (`Safe` / `Idle`).
    pub at_reset_pct: Option<f64>,
    /// How long the window has been running (the basis of the pace).
    pub basis_minutes: i64,
}

/// Linear forecast at the window's average pace so far. Every window starts at 0 % when it
/// resets, so `current ÷ time since the window started` is a known average that includes idle
/// hours (nights, breaks); a burst of recent work is not extrapolated as if it never stopped.
/// Before a tenth of the window has passed the pace is not trusted.
pub fn forecast(now_ms: i64, current: f64, resets_at_ms: i64, window_ms: i64) -> Forecast {
    let started = resets_at_ms - window_ms;
    let elapsed = (now_ms - started).clamp(0, window_ms);
    let left_ms = (resets_at_ms - now_ms).max(0);
    let current = current.clamp(0.0, 100.0);
    let base = Forecast { kind: ForecastKind::Insufficient, rate_per_hour: None, fills_at_ms: None, at_reset_pct: None, basis_minutes: elapsed / 60_000 };
    if window_ms <= 0 || elapsed < window_ms / 10 {
        return base;
    }
    let per_ms = current / elapsed as f64;
    let base = Forecast { rate_per_hour: Some(per_ms * 3_600_000.0), ..base };
    if current < 0.5 {
        return Forecast { kind: ForecastKind::Idle, at_reset_pct: Some(current), ..base };
    }
    let to_full_ms = ((100.0 - current) / per_ms) as i64;
    if to_full_ms < left_ms {
        Forecast { kind: ForecastKind::Fills, fills_at_ms: Some(now_ms + to_full_ms), ..base }
    } else {
        Forecast { kind: ForecastKind::Safe, at_reset_pct: Some((current + per_ms * left_ms as f64).min(100.0)), ..base }
    }
}
