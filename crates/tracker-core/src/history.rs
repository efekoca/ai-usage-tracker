//! Limit history and plan advice. Every five-hour and weekly window the stored readings cover
//! becomes one record (how full it got, whether it filled, whether it was watched to its end),
//! and the plan advice is built only on those records and the ratios providers publish.

use crate::analytics::window_minutes;
use crate::model::{LimitSnapshot, Provider};
use crate::plans::PlansFile;
use crate::store::Store;
use rusqlite::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MIN_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
/// A window at or above this counts as full (providers report whole percentages).
pub const FULL_PCT: f64 = 99.5;
/// Readings whose reset times differ by at most this much name the same window (sources
/// round differently: 12:59:59 and 13:00:00).
const SAME_RESET_MS: i64 = 2 * MIN_MS;
/// Windows that never reached this are unused (Codex reports a moving reset time for them).
const USED_PCT: f64 = 1.0;
/// The plan advice looks at this many days.
pub const ADVICE_DAYS: i64 = 28;

/// What the readings show about one limit window.
#[derive(Debug, Clone, Serialize)]
pub struct WindowRecord {
    /// Known only when a reading named the reset time.
    pub start_ms: Option<i64>,
    pub resets_at_ms: Option<i64>,
    /// When the window actually ended: its reset, or earlier when a newer window started
    /// before it (the provider reset it early). Known only with a reset time.
    pub end_ms: Option<i64>,
    pub first_ms: i64,
    pub last_ms: i64,
    /// Highest reading. When `complete` is false the window may have gone higher unseen.
    pub peak_pct: f64,
    pub readings: u32,
    /// Reached 100 %.
    pub full: bool,
    pub full_at_ms: Option<i64>,
    /// How long it stayed full until its reset (until now while it runs).
    pub full_minutes: Option<i64>,
    /// Watched until (near) its end, so `peak_pct` is how full it got.
    pub complete: bool,
    pub in_progress: bool,
    /// Plan named by the readings (Codex), when any.
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowSeries {
    pub provider: Provider,
    pub limit_id: String,
    pub window: String,
    pub window_minutes: i64,
    /// Oldest first.
    pub windows: Vec<WindowRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LimitHistory {
    pub from_ms: i64,
    pub to_ms: i64,
    pub series: Vec<WindowSeries>,
    /// Days (UTC) with at least one reading in the advice period, per provider.
    pub observed_days: BTreeMap<Provider, u32>,
}

#[derive(Debug, Clone)]
pub struct Reading {
    pub ts_ms: i64,
    pub used: f64,
    pub resets_ms: Option<i64>,
    pub plan: Option<String>,
}

impl Reading {
    fn of(s: &LimitSnapshot) -> Option<Reading> {
        Some(Reading { ts_ms: s.ts_ms, used: s.used_pct?.clamp(0.0, 100.0), resets_ms: s.resets_at.map(|r| r * 1000), plan: s.plan.clone() })
    }
}

/// How close to its end a window must have been read to know how full it got: the last tenth
/// of the window, at least 30 minutes.
fn tail_ms(dur_ms: i64) -> i64 {
    (dur_ms / 10).max(30 * MIN_MS)
}

/// The five-hour and weekly windows of the last `days` days (and the one running now).
pub fn limit_history(store: &Store, now_ms: i64, days: i64) -> Result<LimitHistory> {
    let from_ms = now_ms - days * DAY_MS;
    // a weekly window that ends inside the range started up to a week before it
    let snaps = store.limits_between(from_ms - 7 * DAY_MS, now_ms + 1)?;
    let mut groups: BTreeMap<(Provider, String, String), Vec<Reading>> = BTreeMap::new();
    let mut days_seen: HashMap<Provider, BTreeSet<i64>> = HashMap::new();
    for s in &snaps {
        if s.ts_ms >= now_ms - ADVICE_DAYS * DAY_MS {
            days_seen.entry(s.provider).or_default().insert(s.ts_ms.div_euclid(DAY_MS));
        }
        if s.window != "five_hour" && s.window != "seven_day" {
            continue;
        }
        if let Some(r) = Reading::of(s) {
            groups.entry((s.provider, s.limit_id.clone(), s.window.clone())).or_default().push(r);
        }
    }
    let series = groups
        .into_iter()
        .filter_map(|((provider, limit_id, window), readings)| {
            let minutes = window_minutes(&window)?;
            let windows: Vec<WindowRecord> = build_windows(&readings, minutes * MIN_MS, now_ms)
                .into_iter()
                .filter(|w| w.resets_at_ms.unwrap_or(w.last_ms) >= from_ms)
                .collect();
            (!windows.is_empty()).then_some(WindowSeries { provider, limit_id, window, window_minutes: minutes, windows })
        })
        .collect();
    let observed_days = days_seen.into_iter().map(|(p, d)| (p, d.len() as u32)).collect();
    Ok(LimitHistory { from_ms, to_ms: now_ms, series, observed_days })
}

/// Splits one limit's readings into windows.
/// 1. Readings that name a reset time are grouped by it.
/// 2. Readings without one (Claude desktop's history) join the known window covering them.
/// 3. The rest are split where the value drops (a reset happened in between) or a window's
///    length has passed; their reset time stays unknown.
///
/// A window ends at its reset, or earlier when readings of a newer window (a later reset)
/// begin before it: the provider then reset it early (Codex does this).
pub fn build_windows(readings: &[Reading], dur_ms: i64, now_ms: i64) -> Vec<WindowRecord> {
    struct Known<'a> {
        reset: i64,
        last_reset: i64,
        items: Vec<&'a Reading>,
    }
    let mut named: Vec<&Reading> = readings.iter().filter(|r| r.resets_ms.is_some()).collect();
    named.sort_by_key(|r| (r.resets_ms, r.ts_ms));
    let mut known: Vec<Known> = Vec::new();
    for r in named {
        let reset = r.resets_ms.unwrap_or_default();
        match known.last_mut() {
            Some(k) if reset - k.last_reset <= SAME_RESET_MS => {
                k.last_reset = reset;
                k.reset = k.reset.max(reset);
                k.items.push(r);
            }
            _ => known.push(Known { reset, last_reset: reset, items: vec![r] }),
        }
    }
    // the first reading of any later window cuts an earlier window short
    let ends: Vec<i64> = (0..known.len())
        .map(|i| {
            let next = known[i + 1..].iter().filter_map(|k| k.items.iter().map(|r| r.ts_ms).min()).min();
            next.map_or(known[i].reset, |n| n.min(known[i].reset))
        })
        .collect();
    let mut loose: Vec<&Reading> = Vec::new();
    for r in readings.iter().filter(|r| r.resets_ms.is_none()) {
        match known.iter_mut().zip(&ends).find(|(k, end)| r.ts_ms >= k.reset - dur_ms && r.ts_ms < **end).map(|(k, _)| k) {
            Some(k) => k.items.push(r),
            None => loose.push(r),
        }
    }
    loose.sort_by_key(|r| r.ts_ms);
    let mut segments: Vec<Vec<&Reading>> = Vec::new();
    for r in loose {
        let split = match segments.last() {
            Some(seg) => {
                let (first, last) = (seg[0], seg[seg.len() - 1]);
                r.used < last.used - 0.5 || r.ts_ms - first.ts_ms >= dur_ms
            }
            None => true,
        };
        if split {
            segments.push(vec![r]);
        } else if let Some(seg) = segments.last_mut() {
            seg.push(r);
        }
    }

    let mut out: Vec<WindowRecord> = known.iter().zip(&ends).map(|(k, end)| record(&k.items, Some((k.reset, *end)), dur_ms, now_ms, None)).collect();
    // the first reading after a segment (of any window) bounds when that segment's window reset
    let mut all_ts: Vec<i64> = readings.iter().map(|r| r.ts_ms).collect();
    all_ts.sort_unstable();
    for seg in &segments {
        let last = seg[seg.len() - 1].ts_ms;
        let next = all_ts.get(all_ts.partition_point(|&t| t <= last)).copied();
        out.push(record(seg, None, dur_ms, now_ms, next));
    }
    out.retain(|w| w.peak_pct >= USED_PCT);
    out.sort_by_key(|w| (w.resets_at_ms.unwrap_or(w.last_ms), w.first_ms));
    out
}

/// `reset`: (the named reset time, when the window actually ended).
fn record(items: &[&Reading], reset: Option<(i64, i64)>, dur_ms: i64, now_ms: i64, next_first: Option<i64>) -> WindowRecord {
    let first = items.iter().map(|r| r.ts_ms).min().unwrap_or(now_ms);
    let last = items.iter().map(|r| r.ts_ms).max().unwrap_or(now_ms);
    let peak = items.iter().map(|r| r.used).fold(0.0, f64::max);
    let full_at = items.iter().filter(|r| r.used >= FULL_PCT).map(|r| r.ts_ms).min();
    let in_progress = match reset {
        Some((_, end)) => end > now_ms,
        None => next_first.is_none() && now_ms - first < dur_ms,
    };
    let tail = tail_ms(dur_ms);
    let complete = full_at.is_some()
        || match reset {
            Some((_, end)) => !in_progress && last >= end - tail,
            // the reset fell between this window's last reading and the next reading
            None => next_first.is_some_and(|n| n - last <= tail),
        };
    let mut plans: HashMap<&str, u32> = HashMap::new();
    for r in items {
        if let Some(p) = &r.plan {
            *plans.entry(p.as_str()).or_default() += 1;
        }
    }
    let plan = plans.into_iter().max_by_key(|(p, n)| (*n, std::cmp::Reverse(*p))).map(|(p, _)| p.to_owned());
    WindowRecord {
        start_ms: reset.map(|(r, _)| r - dur_ms),
        resets_at_ms: reset.map(|(r, _)| r),
        end_ms: reset.map(|(_, end)| end),
        first_ms: first,
        last_ms: last,
        peak_pct: peak,
        readings: items.len() as u32,
        full: full_at.is_some(),
        full_at_ms: full_at,
        full_minutes: match (full_at, reset) {
            (Some(f), Some((_, end))) => Some((end.min(now_ms) - f).max(0) / MIN_MS),
            _ => None,
        },
        complete,
        in_progress,
        plan,
    }
}

// ---------------------------------------------------------------- plan advice

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdviceKind {
    /// No plan chosen (and none detected).
    NoPlan,
    /// Team, enterprise, API or an unknown plan: nothing to compare.
    NotApplicable,
    /// Too few days of readings to say anything beyond the limits that filled.
    Insufficient,
    /// Limits filled in the period; a larger plan exists.
    Upgrade,
    /// Limits filled, but this is already the largest personal plan.
    AtTop,
    /// The plan below would have covered every five-hour window with room to spare.
    Downgrade,
    /// No limit filled and the plan below would not clearly have been enough.
    Fits,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WindowStats {
    /// Windows that ended in the period or are running now.
    pub windows: u32,
    /// Ended windows watched until their end.
    pub complete: u32,
    pub full: u32,
    pub full_minutes: i64,
    /// Highest reading of the windows watched until their end.
    pub peak_complete: Option<f64>,
    /// Highest reading of any window, watched to its end or not.
    pub peak_seen: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanAdvice {
    pub provider: Provider,
    pub kind: AdviceKind,
    pub plan: Option<String>,
    pub suggested: Option<String>,
    /// Upgrade only: two or more weekly windows, or six five-hour windows, filled.
    pub strong: bool,
    pub days: i64,
    pub observed_days: u32,
    pub five_hour: WindowStats,
    pub weekly: WindowStats,
    /// Published per-session ratio between the current and the suggested plan (Claude only).
    pub session_ratio: Option<f64>,
    /// The highest five-hour reading scaled by that ratio: where it would have stood.
    pub projected_five_hour: Option<f64>,
    /// The highest weekly reading scaled by the same ratio. The weekly ratio is not published,
    /// so this holds only if the weekly allowance scales like the per-session one.
    pub projected_weekly_if_same_ratio: Option<f64>,
    /// The suggested plan has no five-hour limit at all (published: ChatGPT Pro).
    pub suggested_has_no_five_hour: bool,
    /// List-price difference per month (positive = the suggestion costs more).
    pub monthly_delta_usd: Option<f64>,
}

const MIN_DAYS: u32 = 7;
const DOWNGRADE_DAYS: u32 = 14;
const DOWNGRADE_WEEKLY: u32 = 2;
const DOWNGRADE_FIVE_HOUR: u32 = 5;
/// The plan below must have left at least this much room in every window.
const DOWNGRADE_MAX_PCT: f64 = 80.0;

fn stats(history: &LimitHistory, provider: Provider, window: &str, plan_filter: Option<&str>, since_ms: i64) -> WindowStats {
    let mut s = WindowStats::default();
    for series in history.series.iter().filter(|x| x.provider == provider && x.window == window) {
        for w in &series.windows {
            if w.resets_at_ms.unwrap_or(w.last_ms) < since_ms {
                continue;
            }
            // a window recorded under another plan says nothing about this one
            if let (Some(want), Some(had)) = (plan_filter, &w.plan)
                && want != had
            {
                continue;
            }
            s.windows += 1;
            s.peak_seen = Some(s.peak_seen.unwrap_or(0.0).max(w.peak_pct));
            if w.full {
                s.full += 1;
                s.full_minutes += w.full_minutes.unwrap_or(0);
            }
            if w.complete && !w.in_progress {
                s.complete += 1;
                s.peak_complete = Some(s.peak_complete.unwrap_or(0.0).max(w.peak_pct));
            }
        }
    }
    s
}

/// Plan advice for one provider from the last [`ADVICE_DAYS`] days of limit windows.
pub fn plan_advice(history: &LimitHistory, provider: Provider, plan_id: Option<&str>, plans: &PlansFile, now_ms: i64) -> PlanAdvice {
    let since = now_ms - ADVICE_DAYS * DAY_MS;
    // Codex readings name the plan exactly; Claude's say only "max", which fits both Max plans
    let filter = if provider == Provider::OpenAI { plan_id } else { None };
    let mut a = PlanAdvice {
        provider,
        kind: AdviceKind::NoPlan,
        plan: plan_id.map(str::to_owned),
        suggested: None,
        strong: false,
        days: ADVICE_DAYS,
        observed_days: history.observed_days.get(&provider).copied().unwrap_or(0),
        five_hour: stats(history, provider, "five_hour", filter, since),
        weekly: stats(history, provider, "seven_day", filter, since),
        session_ratio: None,
        projected_five_hour: None,
        projected_weekly_if_same_ratio: None,
        suggested_has_no_five_hour: false,
        monthly_delta_usd: None,
    };
    let Some(id) = plan_id else { return a };
    let Some(current) = plans.find(provider, id).filter(|p| p.ladder.is_some()) else {
        a.kind = AdviceKind::NotApplicable;
        return a;
    };
    let price_delta = |to: &crate::plans::PlanDef| match (current.monthly_usd, to.monthly_usd) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };

    if a.weekly.full >= 1 || a.five_hour.full >= 3 {
        a.strong = a.weekly.full >= 2 || a.five_hour.full >= 6;
        match plans.above(provider, current) {
            Some(up) => {
                a.kind = AdviceKind::Upgrade;
                a.suggested = Some(up.id.clone());
                a.monthly_delta_usd = price_delta(up);
                a.suggested_has_no_five_hour = !up.windows.iter().any(|w| w == "five_hour");
                a.session_ratio = match (current.session_multiple, up.session_multiple) {
                    (Some(c), Some(u)) if c > 0.0 => Some(u / c),
                    _ => None,
                };
            }
            None => a.kind = AdviceKind::AtTop,
        }
        return a;
    }
    if a.observed_days < MIN_DAYS {
        a.kind = AdviceKind::Insufficient;
        return a;
    }
    a.kind = AdviceKind::Fits;
    let Some(down) = plans.below(provider, current) else { return a };
    let (Some(cur_x), Some(down_x)) = (current.session_multiple, down.session_multiple) else { return a };
    if down_x <= 0.0 {
        return a;
    }
    let ratio = cur_x / down_x;
    a.session_ratio = Some(ratio);
    a.projected_five_hour = a.five_hour.peak_complete.map(|p| p * ratio);
    a.projected_weekly_if_same_ratio = a.weekly.peak_complete.map(|p| p * ratio);
    let enough = a.observed_days >= DOWNGRADE_DAYS && a.weekly.complete >= DOWNGRADE_WEEKLY && a.five_hour.complete >= DOWNGRADE_FIVE_HOUR;
    // every window, including the ones not watched to their end, must fit
    let seen_fits = |s: &WindowStats| s.peak_seen.is_none_or(|p| p * ratio <= DOWNGRADE_MAX_PCT);
    let fits = a.projected_five_hour.is_some_and(|p| p <= DOWNGRADE_MAX_PCT)
        && a.projected_weekly_if_same_ratio.is_some_and(|p| p <= DOWNGRADE_MAX_PCT)
        && seen_fits(&a.five_hour)
        && seen_fits(&a.weekly);
    if enough && fits {
        a.kind = AdviceKind::Downgrade;
        a.suggested = Some(down.id.clone());
        a.monthly_delta_usd = price_delta(down);
    }
    a
}
