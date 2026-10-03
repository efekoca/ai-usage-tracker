//! Suggestions drawn only from the user's own records. Each one names what was measured, how
//! many requests it covers and, where the price list allows, the exact API-equivalent amount
//! involved. Every rule has a fixed threshold, so a tip appears only when the effect is real.

use crate::analytics::{Filter, Range};
use crate::model::{Provider, Tokens, Tool};
use crate::pricing::{CostInput, PriceBook};
use crate::store::{EventRow, Store};
use rusqlite::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

const MIN_MS: i64 = 60_000;

/// A cache write this large after a pause is a rebuilt context, not new conversation.
pub const REBUILD_MIN_PROMPT: u64 = 20_000;
pub const LARGE_CONTEXT: u64 = 100_000;
/// Tool error rates are judged on at least this many calls with a known outcome.
pub const TOOL_MIN_CALLS: u64 = 20;
pub const TOOL_ERROR_RATE: f64 = 0.25;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tip {
    /// Claude: after a pause longer than the cache lifetime the whole context was written to
    /// the cache again. `extra_usd` = what those writes cost beyond reading the same tokens.
    CacheRebuild { requests: u64, sessions: u64, tokens: u64, extra_usd: f64, cost_share_pct: f64 },
    /// Requests over a model's long-context threshold; `extra_usd` = the tier's surcharge.
    LongContext { requests: u64, extra_usd: f64, cost_share_pct: f64, models: Vec<String> },
    /// Fast mode; `extra_usd` = the cost above the same requests at standard speed.
    FastMode { requests: u64, extra_usd: f64, cost_share_pct: f64 },
    /// Data residency (e.g. US-only inference); `extra_usd` = the regional surcharge.
    Residency { requests: u64, extra_usd: f64, cost_share_pct: f64 },
    LargeContexts { requests: u64, requests_pct: f64, cost_usd: f64, cost_share_pct: f64, threshold: u64 },
    ToolErrors { tool: Tool, name: String, calls: u64, failed: u64, rate_pct: f64 },
}

impl Tip {
    fn weight(&self) -> f64 {
        match self {
            Tip::CacheRebuild { extra_usd, .. }
            | Tip::LongContext { extra_usd, .. }
            | Tip::FastMode { extra_usd, .. }
            | Tip::Residency { extra_usd, .. } => *extra_usd,
            Tip::LargeContexts { .. } | Tip::ToolErrors { .. } => -1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Tips {
    pub tips: Vec<Tip>,
    /// Requests examined (the filter applied).
    pub requests: u64,
    pub cost_usd: f64,
    pub tool_calls: u64,
}

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

fn cost(book: &PriceBook, c: &CostInput) -> Option<f64> {
    book.cost(c).map(|c| c.total())
}

/// An amount is worth a tip when it is at least $0.50 and 1 % of the period's cost.
fn material(extra: f64, total: f64) -> bool {
    extra >= 0.5 && extra >= total * 0.01
}

fn pct(part: f64, whole: f64) -> f64 {
    if whole > 0.0 { part / whole * 100.0 } else { 0.0 }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rebuilds {
    pub requests: u64,
    pub sessions: u64,
    pub tokens: u64,
    pub extra_usd: f64,
}

/// Claude requests that wrote their whole context to the cache again after a pause longer
/// than the cache lifetime (5 minutes, or an hour for conversations writing 1-hour entries).
/// A request counts when at least 40 % of its prompt was written again (the shared system
/// prompt usually stays cached, the conversation after it does not). The first request of a
/// conversation, a model switch (the cache is per model) and small prompts are not counted.
/// `extra_usd` is what those writes cost beyond reading the same tokens from a warm cache.
pub fn cache_rebuilds(events: &[EventRow], book: &PriceBook) -> Rebuilds {
    let mut threads: HashMap<(&str, &str), Vec<&EventRow>> = HashMap::new();
    for e in events.iter().filter(|e| e.tool.provider() == Provider::Anthropic) {
        if let Some(s) = e.session_id.as_deref() {
            threads.entry((s, e.thread_id.as_deref().unwrap_or(""))).or_default().push(e);
        }
    }
    let mut r = Rebuilds::default();
    let mut sessions = HashSet::new();
    for (key, mut evs) in threads {
        evs.sort_by_key(|e| e.ts_ms);
        for w in evs.windows(2) {
            let (prev, cur) = (w[0], w[1]);
            let ttl = if prev.tokens.cache_write_1h > 0 || cur.tokens.cache_write_1h > 0 { 60 * MIN_MS } else { 5 * MIN_MS };
            let t = &cur.tokens;
            let rebuilt = cur.ts_ms - prev.ts_ms >= ttl
                && prev.model == cur.model
                && cur.request_input >= REBUILD_MIN_PROMPT
                && t.cache_write * 5 >= cur.request_input * 2;
            if !rebuilt {
                continue;
            }
            let warm = Tokens { cache_read: t.cache_read + t.cache_write, cache_write: 0, cache_write_1h: 0, ..*t };
            let (Some(actual), Some(if_warm)) = (cost(book, &input_of(cur)), cost(book, &CostInput { tokens: &warm, ..input_of(cur) })) else {
                continue;
            };
            r.requests += 1;
            r.tokens += t.cache_write;
            r.extra_usd += actual - if_warm;
            sessions.insert(key.0);
        }
    }
    r.sessions = sessions.len() as u64;
    r
}

pub fn tips(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<Tips> {
    let events: Vec<EventRow> = store.events_between(range.from_ms, range.to_ms)?.into_iter().filter(|e| filter.matches(e)).collect();
    let total: f64 = events.iter().filter_map(|e| cost(book, &input_of(e))).sum();
    let mut out = Vec::new();

    // Claude caches explicitly, so its cache writes are logged
    {
        let r = cache_rebuilds(&events, book);
        if r.requests >= 3 && material(r.extra_usd, total) {
            out.push(Tip::CacheRebuild { requests: r.requests, sessions: r.sessions, tokens: r.tokens, extra_usd: r.extra_usd, cost_share_pct: pct(r.extra_usd, total) });
        }
    }

    // each surcharge is measured against the same request at the standard rate
    {
        let (mut lc_n, mut lc_extra) = (0u64, 0.0);
        let mut lc_models: BTreeMap<String, f64> = BTreeMap::new();
        let (mut fast_n, mut fast_extra) = (0u64, 0.0);
        let (mut geo_n, mut geo_extra) = (0u64, 0.0);
        for e in &events {
            let i = input_of(e);
            let Some(actual) = cost(book, &i) else { continue };
            if let Some(p) = book.long_context_premium(&i).filter(|p| *p > 0.0) {
                lc_n += 1;
                lc_extra += p;
                *lc_models.entry(e.model.clone()).or_default() += p;
            }
            if e.speed.is_some()
                && let Some(standard) = cost(book, &CostInput { speed: None, ..i })
                && actual - standard > 1e-12
            {
                fast_n += 1;
                fast_extra += actual - standard;
            }
            if e.inference_geo.is_some()
                && let Some(global) = cost(book, &CostInput { inference_geo: None, ..i })
                && actual - global > 1e-12
            {
                geo_n += 1;
                geo_extra += actual - global;
            }
        }
        if lc_n > 0 && material(lc_extra, total) {
            let mut models: Vec<(String, f64)> = lc_models.into_iter().collect();
            models.sort_by(|a, b| b.1.total_cmp(&a.1));
            out.push(Tip::LongContext {
                requests: lc_n,
                extra_usd: lc_extra,
                cost_share_pct: pct(lc_extra, total),
                models: models.into_iter().map(|(m, _)| m).collect(),
            });
        }
        if fast_n > 0 && material(fast_extra, total) {
            out.push(Tip::FastMode { requests: fast_n, extra_usd: fast_extra, cost_share_pct: pct(fast_extra, total) });
        }
        if geo_n > 0 && material(geo_extra, total) {
            out.push(Tip::Residency { requests: geo_n, extra_usd: geo_extra, cost_share_pct: pct(geo_extra, total) });
        }
    }

    // most of the cost of long sessions is re-sending their context
    {
        let priced: Vec<(u64, f64)> = events.iter().filter_map(|e| cost(book, &input_of(e)).map(|c| (e.request_input, c))).collect();
        let large: Vec<&(u64, f64)> = priced.iter().filter(|(size, _)| *size > LARGE_CONTEXT).collect();
        let large_cost: f64 = large.iter().map(|(_, c)| c).sum();
        let share = pct(large_cost, total);
        if large.len() >= 20 && share >= 50.0 && material(large_cost, total) {
            out.push(Tip::LargeContexts {
                requests: large.len() as u64,
                requests_pct: pct(large.len() as f64, priced.len() as f64),
                cost_usd: large_cost,
                cost_share_pct: share,
                threshold: LARGE_CONTEXT,
            });
        }
    }

    // each tool error usually costs another request
    let mut tool_calls = 0u64;
    {
        let by_session = !filter.models.is_empty() || !filter.clients.is_empty();
        let sessions: HashSet<(&'static str, &str)> = events.iter().filter_map(|e| e.session_id.as_deref().map(|s| (e.tool.as_str(), s))).collect();
        let mut per: BTreeMap<(&'static str, String), (Tool, u64, u64, u64)> = BTreeMap::new();
        for c in store.tool_calls_between(range.from_ms, range.to_ms)? {
            if (!filter.tools.is_empty() && !filter.tools.contains(&c.tool))
                || (!filter.projects.is_empty() && !c.project_id.is_some_and(|p| filter.projects.contains(&p)))
                || (by_session && !c.session_id.as_deref().is_some_and(|s| sessions.contains(&(c.tool.as_str(), s))))
            {
                continue;
            }
            tool_calls += 1;
            let r = per.entry((c.tool.as_str(), c.name.clone())).or_insert((c.tool, 0, 0, 0));
            r.1 += 1;
            if let Some(f) = c.failed {
                r.2 += 1;
                r.3 += u64::from(f);
            }
        }
        let mut flagged: Vec<Tip> = per
            .into_iter()
            .filter(|(_, (_, _, known, failed))| *known >= TOOL_MIN_CALLS && (*failed as f64) / (*known as f64) >= TOOL_ERROR_RATE)
            .map(|((_, name), (tool, calls, known, failed))| Tip::ToolErrors { tool, name, calls, failed, rate_pct: pct(failed as f64, known as f64) })
            .collect();
        flagged.sort_by(|a, b| match (a, b) {
            (Tip::ToolErrors { failed: x, .. }, Tip::ToolErrors { failed: y, .. }) => y.cmp(x),
            _ => std::cmp::Ordering::Equal,
        });
        out.extend(flagged.into_iter().take(3));
    }

    // money first (largest amount first), then the facts without an amount, in rule order
    out.sort_by(|a, b| b.weight().total_cmp(&a.weight()));
    Ok(Tips { tips: out, requests: events.len() as u64, cost_usd: total, tool_calls })
}
