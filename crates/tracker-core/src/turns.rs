//! What each prompt cost: every request, subagent and tool call it set off, as one line. A turn
//! is the tool's own grouping (see [`crate::model::Turn`]), so nothing here is inferred from
//! timing, and no prompt is ever read.

use crate::analytics::{Filter, Range};
use crate::model::Tool;
use crate::pricing::{CostInput, PriceBook};
use crate::store::{EventRow, ProjectRow, Store};
use rusqlite::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The main conversation's share of a turn, or one kind of subagent's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnPart {
    /// The subagent's type; `None` for the main conversation.
    pub agent: Option<String>,
    /// How many subagents of this type ran (0 for the main conversation).
    pub threads: u64,
    pub requests: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnCost {
    pub id: String,
    pub tool: Tool,
    pub client: Option<String>,
    pub session_id: Option<String>,
    /// The turn's place in its conversation, 1 for the first.
    pub ordinal: u32,
    /// What began it: `human`, `task_notification`, `peer`, `scheduled`, `sdk` or `other`.
    pub origin: String,
    pub project_id: Option<i64>,
    pub project: String,
    pub hidden: bool,
    pub started_ms: i64,
    /// When its last request was made.
    pub ended_ms: i64,
    /// Requests of the main conversation and its subagents; an advisor's are part of the cost only.
    pub requests: u64,
    pub agents: u64,
    pub tool_calls: u64,
    pub tool_failed: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    /// Requests of a model with no price, which the cost leaves out.
    pub unpriced_requests: u64,
    /// The models it used, the one that cost the most first.
    pub models: Vec<String>,
    /// The main conversation first, then the subagent types by cost.
    pub parts: Vec<TurnPart>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Turns {
    /// Newest first.
    pub turns: Vec<TurnCost>,
    /// Whether any log read so far names its turns.
    pub tracked: bool,
    /// Requests in the range, of the tools that name turns, that belong to no turn listed
    /// anywhere: their log named none, or the log that recorded the turn is gone.
    pub requests_outside: u64,
}

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

/// The tools whose logs name turns.
const TURN_TOOLS: [Tool; 2] = [Tool::ClaudeCode, Tool::Codex];

#[derive(Default)]
struct Part {
    threads: BTreeSet<String>,
    requests: u64,
    cost: f64,
}

#[derive(Default)]
struct Acc {
    client: Option<String>,
    projects: HashMap<Option<i64>, u64>,
    sessions: HashMap<String, u64>,
    ended: i64,
    requests: u64,
    tokens: u64,
    cost: f64,
    unpriced: u64,
    models: BTreeMap<String, f64>,
    parts: BTreeMap<Option<String>, Part>,
    calls: u64,
    failed: u64,
}

/// The turns that began in `range`, each with everything it set off, whenever that ran: a
/// subagent still working after the range ended counts toward the turn that started it. A turn
/// none of whose requests pass `filter` is left out.
pub fn turns(store: &Store, book: &PriceBook, range: Range, filter: &Filter) -> Result<Turns> {
    let mut map: HashMap<String, Acc> = HashMap::new();
    for e in store.turn_events(range.from_ms, range.to_ms)?.iter().filter(|e| filter.matches(e)) {
        let Some(turn) = e.turn.as_deref() else { continue };
        let a = map.entry(turn.to_owned()).or_default();
        if a.client.is_none() {
            a.client = e.client.clone();
        }
        *a.projects.entry(e.project_id).or_default() += 1;
        if let Some(s) = &e.session_id {
            *a.sessions.entry(s.clone()).or_default() += 1;
        }
        a.ended = a.ended.max(e.ts_ms);
        a.tokens = a.tokens.saturating_add(e.tokens.total());
        let cost = book.cost(&input_of(e)).map(|c| c.total());
        match cost {
            Some(c) => {
                a.cost += c;
                *a.models.entry(e.model.clone()).or_default() += c;
            }
            None => {
                a.models.entry(e.model.clone()).or_default();
                if e.tokens.total() > 0 {
                    a.unpriced += 1;
                }
            }
        }
        // a thread with no type is still a subagent's
        let agent = e.agent.clone().or_else(|| e.thread_id.as_ref().map(|_| "subagent".to_owned()));
        let part = a.parts.entry(agent).or_default();
        part.cost += cost.unwrap_or(0.0);
        if let Some(t) = &e.thread_id {
            part.threads.insert(t.clone());
        }
        if !e.advisor {
            a.requests += 1;
            part.requests += 1;
        }
    }
    for c in store.turn_calls(range.from_ms, range.to_ms)? {
        let passes = (filter.tools.is_empty() || filter.tools.contains(&c.tool)) && (filter.projects.is_empty() || c.project_id.is_some_and(|p| filter.projects.contains(&p)));
        if let (true, Some(a)) = (passes, map.get_mut(&c.turn)) {
            a.calls += 1;
            a.failed += u64::from(c.failed == Some(true));
        }
    }

    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    let mut out: Vec<TurnCost> = store
        .turns_between(range.from_ms, range.to_ms)?
        .into_iter()
        .filter_map(|t| {
            let a = map.remove(&t.id)?;
            // the project most of the turn's requests belong to
            let project_id = a.projects.iter().max_by_key(|(id, n)| (**n, id.unwrap_or(i64::MIN))).and_then(|(id, _)| *id);
            let (project, hidden) = project_id.and_then(|i| projects.get(&i)).map_or((String::new(), false), |p| (p.name.clone(), p.hidden));
            // the session its requests are filed under: a resumed conversation copies the prompt's
            // line into a log of another name
            let session_id = a.sessions.iter().max_by(|x, y| x.1.cmp(y.1).then_with(|| y.0.cmp(x.0))).map(|(s, _)| s.clone()).or(t.session_id);
            let mut models: Vec<(String, f64)> = a.models.into_iter().collect();
            models.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
            let agents = a.parts.iter().filter(|(k, _)| k.is_some()).map(|(_, p)| p.threads.len() as u64).sum();
            let mut parts: Vec<TurnPart> = a
                .parts
                .into_iter()
                .map(|(agent, p)| TurnPart { agent, threads: p.threads.len() as u64, requests: p.requests, cost_usd: p.cost })
                .collect();
            parts.sort_by(|x, y| x.agent.is_some().cmp(&y.agent.is_some()).then(y.cost_usd.total_cmp(&x.cost_usd)).then_with(|| x.agent.cmp(&y.agent)));
            Some(TurnCost {
                id: t.id,
                tool: t.tool,
                client: a.client,
                session_id,
                ordinal: t.ordinal,
                origin: t.origin,
                project_id,
                project,
                hidden,
                started_ms: t.ts_ms,
                // a request can be stamped just before the line that names its turn
                ended_ms: a.ended.max(t.ts_ms),
                requests: a.requests,
                agents,
                tool_calls: a.calls,
                tool_failed: a.failed,
                total_tokens: a.tokens,
                cost_usd: a.cost,
                unpriced_requests: a.unpriced,
                models: models.into_iter().map(|(m, _)| m).collect(),
                parts,
            })
        })
        .collect();
    out.reverse();

    let unrecorded = store.turns_without_record(range.from_ms, range.to_ms)?;
    let requests_outside = store
        .events_between(range.from_ms, range.to_ms)?
        .iter()
        .filter(|e| e.turn.as_ref().is_none_or(|t| unrecorded.contains(t)) && !e.advisor && TURN_TOOLS.contains(&e.tool) && filter.matches(e))
        .count() as u64;
    Ok(Turns { turns: out, tracked: store.has_turns()?, requests_outside })
}
