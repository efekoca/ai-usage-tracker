//! UI aggregations. Periods and daily buckets use the user's local time zone.

use crate::capture::antigravity_limits::GEMINI_POOL;
use crate::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool};
use crate::pricing::{Cost, CostInput, PriceBook};
use crate::store::{EventRow, ProjectRow, Store};
use chrono::{Datelike, Days, Months, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Period {
    Today,
    Days7,
    Month1,
    Months3,
    Months6,
    Year1,
    All,
    Custom { from: NaiveDate, to: NaiveDate },
}

/// Half-open interval `[from_ms, to_ms)` in UTC epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub from_ms: i64,
    pub to_ms: i64,
}

impl Range {
    /// The same number of local calendar days right before this range (DST-safe: a day that
    /// is 23 or 25 hours long still counts as one day).
    pub fn previous<Tz: TimeZone>(&self, tz: &Tz) -> Range {
        let first = local_date(tz, self.from_ms);
        let days = (local_date(tz, self.to_ms - 1) - first).num_days().max(0) as u64 + 1;
        Range { from_ms: start_of_day(tz, first.checked_sub_days(Days::new(days)).unwrap_or(first)), to_ms: self.from_ms }
    }
}

/// Local midnight of `d` (DST-safe: the earliest instant of that local date).
pub fn start_of_day<Tz: TimeZone>(tz: &Tz, d: NaiveDate) -> i64 {
    let naive = d.and_hms_opt(0, 0, 0).unwrap();
    match tz.from_local_datetime(&naive).earliest() {
        Some(t) => t.timestamp_millis(),
        // midnight skipped by a DST jump: take 01:00
        None => tz.from_local_datetime(&(naive + chrono::Duration::hours(1))).earliest().map(|t| t.timestamp_millis()).unwrap_or(0),
    }
}

pub fn local_date<Tz: TimeZone>(tz: &Tz, ms: i64) -> NaiveDate {
    tz.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

/// Ten years, so the dense daily series stays bounded.
pub const MAX_RANGE_DAYS: u64 = 3660;

pub fn period_range<Tz: TimeZone>(period: Period, tz: &Tz, now_ms: i64, first_event_ms: Option<i64>) -> Range {
    let today = local_date(tz, now_ms);
    let end = start_of_day(tz, today + Days::new(1));
    let since = |d: NaiveDate| Range { from_ms: start_of_day(tz, d), to_ms: end };
    let months_back = |m: u32| today.checked_sub_months(Months::new(m)).map(|d| d + Days::new(1)).unwrap_or(today);
    let oldest = |last: NaiveDate| last - Days::new(MAX_RANGE_DAYS - 1);
    match period {
        Period::Today => since(today),
        Period::Days7 => since(today - Days::new(6)),
        // "last 30 days" (not a calendar month, which would be 28–31 days)
        Period::Month1 => since(today - Days::new(29)),
        Period::Months3 => since(months_back(3)),
        Period::Months6 => since(months_back(6)),
        Period::Year1 => since(months_back(12)),
        Period::All => since(first_event_ms.map(|ms| local_date(tz, ms)).unwrap_or(today).min(today).max(oldest(today))),
        Period::Custom { from, to } => {
            let bounds = |d: NaiveDate| d.clamp(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(), NaiveDate::from_ymd_opt(9999, 12, 30).unwrap());
            let (a, b) = (bounds(from.min(to)), bounds(from.max(to)));
            Range { from_ms: start_of_day(tz, a.max(oldest(b))), to_ms: start_of_day(tz, b + Days::new(1)) }
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Filter {
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub clients: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub projects: Vec<i64>,
}

impl Filter {
    pub fn matches(&self, e: &EventRow) -> bool {
        (self.tools.is_empty() || self.tools.contains(&e.tool))
            && (self.clients.is_empty() || e.client.as_ref().is_some_and(|c| self.clients.contains(c)))
            && (self.models.is_empty() || self.models.contains(&e.model))
            && (self.projects.is_empty() || e.project_id.is_some_and(|p| self.projects.contains(&p)))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub events: u64,
    pub tokens: Tokens,
    pub total_tokens: u64,
    /// API-equivalent cost of the priced part.
    pub cost: Cost,
    pub cost_usd: f64,
    pub unpriced_events: u64,
    pub unpriced_tokens: u64,
    /// Net API-equivalent saving from prompt caching (reads minus write premium), priced part.
    pub cache_savings_usd: f64,
    /// Reads and writes of requests whose model logs its cache writes: Claude always, an OpenAI
    /// model only when its requests in the period report any (older Codex logs carry reads only).
    pub cache_read_with_writes: u64,
    pub cache_write_with_reads: u64,
}

impl Totals {
    /// Adds one request with its cost (`None` = unpriced) and net cache saving.
    pub fn add_event(&mut self, e: &EventRow, cost: Option<&Cost>, savings: f64) {
        self.add(e, cost, savings)
    }

    fn add(&mut self, e: &EventRow, cost: Option<&Cost>, savings: f64) {
        self.add_counted(e, cost, savings, e.tool.provider() == Provider::Anthropic)
    }

    fn add_counted(&mut self, e: &EventRow, cost: Option<&Cost>, savings: f64, reuse: bool) {
        self.events += 1;
        self.cache_savings_usd += savings;
        self.tokens.add(&e.tokens);
        self.total_tokens += e.tokens.total();
        if reuse {
            self.cache_read_with_writes += e.tokens.cache_read;
            self.cache_write_with_reads += e.tokens.cache_write;
        }
        match cost {
            Some(c) => {
                self.cost.add(c);
                self.cost_usd += c.total();
            }
            None => {
                self.unpriced_events += 1;
                self.unpriced_tokens += e.tokens.total();
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub key: String,
    pub label: String,
    /// Project groups only: the user asked to hide this project's name.
    pub hidden: bool,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayPoint {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub events: u64,
    pub by_tool: BTreeMap<String, u64>,
    pub cost_by_tool: BTreeMap<String, f64>,
    /// Prompt tokens (uncached input + cache read + cache write) and the cached part.
    pub prompt_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub range: Range,
    pub totals: Totals,
    /// Same-length period immediately before `range`, for trend arrows.
    pub previous: Totals,
    pub by_tool: Vec<Group>,
    pub by_client: Vec<Group>,
    pub by_model: Vec<Group>,
    pub by_project: Vec<Group>,
    pub daily: Vec<DayPoint>,
    /// `[weekday Mon=0..Sun=6][hour 0..23]` total tokens, local time.
    pub heatmap: Vec<Vec<u64>>,
    pub peak_day: Option<DayPoint>,
    pub peak_hour: Option<u32>,
    pub peak_weekday: Option<u32>,
    pub days_in_range: u32,
    pub active_days: u32,
    pub avg_daily_tokens: f64,
    pub avg_daily_cost: f64,
    pub unpriced_models: Vec<String>,
    pub by_accuracy: BTreeMap<String, u64>,
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

fn cost_of(book: &PriceBook, e: &EventRow) -> Option<Cost> {
    book.cost(&input_of(e))
}

fn savings_of(book: &PriceBook, e: &EventRow) -> f64 {
    book.cache_savings(&input_of(e)).unwrap_or(0.0)
}

fn cache_writers<'a>(events: impl Iterator<Item = &'a EventRow>) -> impl Fn(&EventRow) -> bool {
    let models: HashSet<String> = events.filter(|e| e.tokens.cache_write > 0).map(|e| e.model.clone()).collect();
    move |e| e.tool.provider() == Provider::Anthropic || models.contains(&e.model)
}

fn sorted_groups(map: HashMap<String, (String, bool, Totals)>) -> Vec<Group> {
    let mut v: Vec<Group> =
        map.into_iter().map(|(key, (label, hidden, totals))| Group { key, label, hidden, totals }).collect();
    v.sort_by(|a, b| {
        b.totals.cost_usd.total_cmp(&a.totals.cost_usd).then(b.totals.total_tokens.cmp(&a.totals.total_tokens)).then(a.key.cmp(&b.key))
    });
    v
}

pub fn report<Tz: TimeZone>(store: &Store, book: &PriceBook, range: Range, filter: &Filter, tz: &Tz) -> Result<Report> {
    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    let events = store.events_between(range.from_ms, range.to_ms)?;

    let mut totals = Totals::default();
    let (mut by_tool, mut by_client, mut by_model, mut by_project) =
        (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());
    // per local day: totals, tokens per tool, cost per tool
    type DayAcc = (Totals, BTreeMap<String, u64>, BTreeMap<String, f64>);
    let mut days: BTreeMap<NaiveDate, DayAcc> = BTreeMap::new();
    let mut heat = vec![vec![0u64; 24]; 7];
    let mut unpriced = BTreeMap::new();
    let mut by_accuracy = BTreeMap::new();

    let writers = cache_writers(events.iter().filter(|e| filter.matches(e)));
    for e in events.iter().filter(|e| filter.matches(e)) {
        let c = cost_of(book, e);
        let sv = savings_of(book, e);
        let reuse = writers(e);
        totals.add_counted(e, c.as_ref(), sv, reuse);
        let add = |m: &mut HashMap<String, (String, bool, Totals)>, key: String, label: String, hidden: bool| {
            m.entry(key).or_insert_with(|| (label, hidden, Totals::default())).2.add_counted(e, c.as_ref(), sv, reuse);
        };
        add(&mut by_tool, e.tool.as_str().into(), e.tool.as_str().into(), false);
        let client = e.client.clone().unwrap_or_else(|| "unknown".into());
        add(&mut by_client, format!("{}:{client}", e.tool.as_str()), client, false);
        add(&mut by_model, e.model.clone(), e.model.clone(), false);
        match e.project_id.and_then(|id| projects.get(&id)) {
            Some(p) => add(&mut by_project, p.id.to_string(), p.name.clone(), p.hidden),
            None => add(&mut by_project, "none".into(), String::new(), false),
        }
        if c.is_none() {
            *unpriced.entry(e.model.clone()).or_insert(0u64) += 1;
        }
        *by_accuracy.entry(e.accuracy.as_str().to_owned()).or_insert(0u64) += 1;

        if let Some(local) = tz.timestamp_millis_opt(e.ts_ms).single() {
            let d = days.entry(local.date_naive()).or_default();
            d.0.add_counted(e, c.as_ref(), sv, reuse);
            *d.1.entry(e.tool.as_str().to_owned()).or_insert(0) += e.tokens.total();
            *d.2.entry(e.tool.as_str().to_owned()).or_insert(0.0) += c.as_ref().map(Cost::total).unwrap_or(0.0);
            heat[local.weekday().num_days_from_monday() as usize][local.hour() as usize] += e.tokens.total();
        }
    }

    let prev_range = range.previous(tz);
    let mut previous = Totals::default();
    let prev_events = store.events_between(prev_range.from_ms, prev_range.to_ms)?;
    let prev_writers = cache_writers(prev_events.iter().filter(|e| filter.matches(e)));
    for e in prev_events.iter().filter(|e| filter.matches(e)) {
        previous.add_counted(e, cost_of(book, e).as_ref(), savings_of(book, e), prev_writers(e));
    }

    // dense daily series over the whole range
    let first = local_date(tz, range.from_ms);
    let last = local_date(tz, range.to_ms - 1);
    let mut daily = Vec::new();
    let mut d = first;
    while d <= last {
        let (t, by_tool, cost_by_tool) = days.remove(&d).unwrap_or_default();
        daily.push(DayPoint {
            date: d.to_string(),
            tokens: t.total_tokens,
            cost_usd: t.cost_usd,
            events: t.events,
            by_tool,
            cost_by_tool,
            prompt_tokens: t.tokens.input + t.tokens.cache_read + t.tokens.cache_write,
            cache_read: t.tokens.cache_read,
            cache_write: t.tokens.cache_write,
            cache_savings_usd: t.cache_savings_usd,
        });
        let Some(next) = d.succ_opt() else { break };
        d = next;
    }
    let days_in_range = daily.len() as u32;
    let active_days = daily.iter().filter(|p| p.events > 0).count() as u32;
    let peak_day = daily.iter().filter(|p| p.tokens > 0).max_by_key(|p| p.tokens).cloned();
    let hour_sums: Vec<u64> = (0..24).map(|h| heat.iter().map(|row| row[h]).sum()).collect();
    let peak_hour = hour_sums.iter().enumerate().filter(|(_, v)| **v > 0).max_by_key(|(_, v)| **v).map(|(h, _)| h as u32);
    let peak_weekday =
        heat.iter().enumerate().map(|(i, r)| (i, r.iter().sum::<u64>())).filter(|(_, v)| *v > 0).max_by_key(|(_, v)| *v).map(|(i, _)| i as u32);
    let denom = days_in_range.max(1) as f64;

    Ok(Report {
        range,
        avg_daily_tokens: totals.total_tokens as f64 / denom,
        avg_daily_cost: totals.cost_usd / denom,
        totals,
        previous,
        by_tool: sorted_groups(by_tool),
        by_client: sorted_groups(by_client),
        by_model: sorted_groups(by_model),
        by_project: sorted_groups(by_project),
        daily,
        heatmap: heat,
        peak_day,
        peak_hour,
        peak_weekday,
        days_in_range,
        active_days,
        unpriced_models: unpriced.into_keys().collect(),
        by_accuracy,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitState {
    /// Observation is inside its current window.
    Fresh,
    /// The window has reset since the observation; current use is unknown (≥ 0).
    Reset,
    /// No reset time known and the observation is older than one window.
    Stale,
    /// Used after the reading, or the reading is over a day old: the real value is at least the reading.
    Behind,
}

/// Live readers refresh every few minutes, so a reading this recent still counts as current.
pub const BEHIND_GRACE_MS: i64 = 10 * 60_000;
/// Use on another device or the web fills the same limit without reaching the local logs.
pub const MAX_FRESH_AGE_MS: i64 = 24 * 3_600_000;

/// User-defined budget for a window, used only when no real limit reading exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Threshold {
    pub provider: Provider,
    pub window: String,
    #[serde(default)]
    pub tokens: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectShare {
    pub project_id: Option<i64>,
    pub name: String,
    pub hidden: bool,
    pub totals: Totals,
    /// Fraction (0–1) of the window's usage, by cost (by tokens if any request is unpriced).
    pub share: f64,
    /// `share × used_pct` — always an estimate.
    pub estimated_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LimitView {
    pub provider: Provider,
    pub limit_id: String,
    pub window: String,
    pub window_minutes: Option<i64>,
    pub used_pct: Option<f64>,
    pub resets_at: Option<i64>,
    pub observed_ms: Option<i64>,
    pub source: String,
    pub status: Option<String>,
    pub plan: Option<String>,
    pub accuracy: Accuracy,
    pub state: LimitState,
    pub window_start_ms: Option<i64>,
    /// Usage recorded in local logs since the window started (exact counts).
    pub window_usage: Totals,
    /// Usage after the reading (`Behind` only): what the reading does not include yet.
    pub usage_since: Totals,
    /// Pace of the current window from its recent readings (current readings only).
    pub forecast: Option<crate::insights::Forecast>,
    pub projects: Vec<ProjectShare>,
}

pub fn window_minutes(window: &str) -> Option<i64> {
    match window {
        "five_hour" => Some(300),
        "one_day" => Some(1440),
        "seven_day" | "seven_day_opus" | "seven_day_sonnet" => Some(10080),
        "thirty_day" => Some(43200),
        w => w.strip_suffix('m').and_then(|n| n.parse().ok()),
    }
}

fn provider_tools(p: Provider) -> &'static [Tool] {
    match p {
        Provider::Anthropic => &[Tool::ClaudeCode],
        Provider::OpenAI => &[Tool::Codex],
        Provider::Google => &[Tool::Antigravity],
    }
}

pub(crate) fn counts_toward(provider: Provider, limit_id: &str, window: &str, e: &EventRow) -> bool {
    let model = e.model.to_ascii_lowercase();
    let family = match window {
        "seven_day_opus" => Some("opus"),
        "seven_day_sonnet" => Some("sonnet"),
        _ => None,
    };
    // Antigravity's Gemini models and its other models (Claude, GPT-OSS) have separate pools
    let pool = match (provider, limit_id) {
        (Provider::Google, "") => true,
        (Provider::Google, GEMINI_POOL) => model.starts_with("gemini"),
        (Provider::Google, _) => !model.starts_with("gemini"),
        _ => true,
    };
    provider_tools(provider).contains(&e.tool) && pool && family.is_none_or(|f| model.contains(f))
}

pub fn limits_view(store: &Store, book: &PriceBook, now_ms: i64, thresholds: &[Threshold]) -> Result<Vec<LimitView>> {
    // newest reading per (provider, limit_id, window) across all sources: Claude's desktop
    // history, Cowork and Claude Code readings describe the same account-wide pool
    let mut newest: BTreeMap<(String, String, String), LimitSnapshot> = BTreeMap::new();
    for s in store.latest_limits()? {
        let k = (s.provider.as_str().to_owned(), s.limit_id.clone(), s.window.clone());
        match newest.get(&k) {
            Some(cur) if cur.ts_ms >= s.ts_ms => {}
            _ => {
                newest.insert(k, s);
            }
        }
    }
    // drop windows that belong to a plan the user no longer has (e.g. a monthly window on an old plan)
    let mut current_plan: HashMap<Provider, (i64, String)> = HashMap::new();
    for s in newest.values() {
        if let Some(plan) = &s.plan {
            let e = current_plan.entry(s.provider).or_insert((s.ts_ms, plan.clone()));
            if s.ts_ms > e.0 {
                *e = (s.ts_ms, plan.clone());
            }
        }
    }
    let snaps: Vec<LimitSnapshot> = newest
        .into_values()
        .filter(|s| match (&s.plan, current_plan.get(&s.provider)) {
            (Some(p), Some((_, cur))) => p == cur,
            _ => true,
        })
        .collect();

    let projects: HashMap<i64, ProjectRow> = store.projects()?.into_iter().map(|p| (p.id, p)).collect();
    let mut out = Vec::new();
    for s in &snaps {
        let minutes = window_minutes(&s.window);
        let dur_ms = minutes.map(|m| m * 60_000);
        let reset_ms = s.resets_at_ms();
        let mut state = match (reset_ms, dur_ms) {
            (Some(r), _) if r <= now_ms => LimitState::Reset,
            (None, Some(d)) if now_ms - s.ts_ms > d => LimitState::Stale,
            _ => LimitState::Fresh,
        };
        let mut usage_since = Totals::default();
        if state == LimitState::Fresh {
            let age = now_ms - s.ts_ms;
            if age >= BEHIND_GRACE_MS {
                for e in store.events_between(s.ts_ms + 1, now_ms + 1)?.iter().filter(|e| counts_toward(s.provider, &s.limit_id, &s.window, e)) {
                    usage_since.add(e, cost_of(book, e).as_ref(), savings_of(book, e));
                }
            }
            if usage_since.events > 0 || age > MAX_FRESH_AGE_MS {
                state = LimitState::Behind;
            }
        }
        let start = dur_ms.map(|d| match reset_ms {
            Some(r) if r > now_ms => r - d,
            _ => now_ms - d,
        });
        // project shares of a limit only make sense against a current reading
        let used = if state == LimitState::Fresh { s.used_pct } else { None };
        let forecast = match (state, reset_ms, dur_ms, s.used_pct) {
            (LimitState::Fresh, Some(r), Some(d), Some(cur)) => Some(crate::insights::forecast(now_ms, cur, r, d)),
            _ => None,
        };
        let (usage, shares) = match start {
            Some(st) => window_usage(store, book, &projects, s.provider, &s.limit_id, &s.window, st, now_ms, used)?,
            None => (Totals::default(), Vec::new()),
        };
        out.push(LimitView {
            provider: s.provider,
            limit_id: s.limit_id.clone(),
            window: s.window.clone(),
            window_minutes: minutes,
            used_pct: s.used_pct,
            resets_at: reset_ms.map(|ms| ms / 1000),
            observed_ms: Some(s.ts_ms),
            source: s.source.clone(),
            status: s.status.clone(),
            plan: s.plan.clone(),
            accuracy: s.accuracy,
            state,
            window_start_ms: start,
            window_usage: usage,
            usage_since,
            forecast,
            projects: shares,
        });
    }

    // user thresholds fill in windows without any real reading
    for t in thresholds {
        if out.iter().any(|v| v.provider == t.provider && v.window == t.window) {
            continue;
        }
        let Some(minutes) = window_minutes(&t.window) else { continue };
        let start = now_ms - minutes * 60_000;
        let (usage, mut shares) = window_usage(store, book, &projects, t.provider, "", &t.window, start, now_ms, None)?;
        let pct = match (t.cost_usd, t.tokens) {
            (Some(c), _) if c > 0.0 => Some(usage.cost_usd / c * 100.0),
            (_, Some(n)) if n > 0 => Some(usage.total_tokens as f64 / n as f64 * 100.0),
            _ => None,
        };
        for sh in &mut shares {
            sh.estimated_pct = pct.map(|p| p * sh.share);
        }
        out.push(LimitView {
            provider: t.provider,
            limit_id: String::new(),
            window: t.window.clone(),
            window_minutes: Some(minutes),
            used_pct: pct,
            resets_at: None,
            observed_ms: None,
            source: "user_threshold".into(),
            status: None,
            plan: None,
            accuracy: Accuracy::Estimated,
            state: LimitState::Fresh,
            usage_since: Totals::default(),
            forecast: None,
            window_start_ms: Some(start),
            window_usage: usage,
            projects: shares,
        });
    }
    out.sort_by(|a, b| {
        (a.provider.as_str(), a.window_minutes.unwrap_or(i64::MAX), &a.limit_id).cmp(&(
            b.provider.as_str(),
            b.window_minutes.unwrap_or(i64::MAX),
            &b.limit_id,
        ))
    });
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn window_usage(
    store: &Store,
    book: &PriceBook,
    projects: &HashMap<i64, ProjectRow>,
    provider: Provider,
    limit_id: &str,
    window: &str,
    from_ms: i64,
    to_ms: i64,
    used_pct: Option<f64>,
) -> Result<(Totals, Vec<ProjectShare>)> {
    let mut total = Totals::default();
    let mut per: HashMap<Option<i64>, Totals> = HashMap::new();
    for e in store.events_between(from_ms, to_ms + 1)?.iter().filter(|e| counts_toward(provider, limit_id, window, e)) {
        let c = cost_of(book, e);
        let sv = savings_of(book, e);
        total.add(e, c.as_ref(), sv);
        per.entry(e.project_id).or_default().add(e, c.as_ref(), sv);
    }
    let by_cost = total.cost_usd > 0.0 && total.unpriced_events == 0;
    let mut shares: Vec<ProjectShare> = per
        .into_iter()
        .map(|(pid, t)| {
            let share = if by_cost {
                t.cost_usd / total.cost_usd
            } else if total.total_tokens > 0 {
                t.total_tokens as f64 / total.total_tokens as f64
            } else {
                0.0
            };
            let p = pid.and_then(|id| projects.get(&id));
            ProjectShare {
                project_id: pid,
                name: p.map(|p| p.name.clone()).unwrap_or_default(),
                hidden: p.is_some_and(|p| p.hidden),
                totals: t,
                share,
                estimated_pct: used_pct.map(|u| u * share),
            }
        })
        .collect();
    shares.sort_by(|a, b| b.share.total_cmp(&a.share));
    Ok((total, shares))
}

pub fn today<Tz: TimeZone>(tz: &Tz, now_ms: i64) -> Range {
    period_range(Period::Today, tz, now_ms, None)
}

pub fn age_minutes(now_ms: i64, ts_ms: i64) -> i64 {
    (now_ms - ts_ms).max(0) / 60_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const DAY_MS: i64 = 86_400_000;

    #[test]
    fn periods_are_local_calendar_days() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        // 2026-10-02 00:30 local == 2026-10-01 21:30 UTC
        let now = tz.with_ymd_and_hms(2026, 10, 2, 0, 30, 0).unwrap().timestamp_millis();
        let r = period_range(Period::Today, &tz, now, None);
        assert_eq!(r.from_ms, tz.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap().timestamp_millis());
        assert_eq!(r.to_ms - r.from_ms, DAY_MS);
        let r7 = period_range(Period::Days7, &tz, now, None);
        assert_eq!((r7.to_ms - r7.from_ms) / DAY_MS, 7);
        let m1 = period_range(Period::Month1, &tz, now, None);
        assert_eq!(local_date(&tz, m1.from_ms).to_string(), "2026-09-03");
        let y1 = period_range(Period::Year1, &tz, now, None);
        assert_eq!(local_date(&tz, y1.from_ms).to_string(), "2025-10-03");
        let custom = period_range(
            Period::Custom { from: NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), to: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap() },
            &tz,
            now,
            None,
        );
        assert_eq!((custom.to_ms - custom.from_ms) / DAY_MS, 5);
        assert_eq!(r.previous(&tz).to_ms, r.from_ms);
        assert_eq!(r.previous(&tz).to_ms - r.previous(&tz).from_ms, DAY_MS);
        // the 30-day period is 30 days at the end of a long month too
        let oct31 = tz.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap().timestamp_millis();
        let m = period_range(Period::Month1, &tz, oct31, None);
        assert_eq!(local_date(&tz, m.from_ms).to_string(), "2026-10-02");
        assert_eq!((m.to_ms - m.from_ms) / DAY_MS, 30);
    }

    #[test]
    fn window_names_map_to_minutes() {
        assert_eq!(window_minutes("five_hour"), Some(300));
        assert_eq!(window_minutes("seven_day"), Some(10080));
        assert_eq!(window_minutes("90m"), Some(90));
        assert_eq!(window_minutes("so"), None);
    }
}
