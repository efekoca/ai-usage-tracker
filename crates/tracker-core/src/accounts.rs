//! Codex accounts. Codex logs the account that started a thread, not the one that sent each
//! request: a desktop account switch or a resume carries a thread over to another account. Every
//! account has its own limit windows, though, and each request reports the window it counted
//! toward, so requests are grouped by window and windows are tied to accounts by evidence:
//! * a live read through `codex app-server` (Codex ≥ 0.163), which names the signed-in account;
//! * the account that started a thread, for the requests it sent right after starting.
//!
//! A window no evidence names belongs to the only account seen, unless the windows show that
//! more than one account was in use; then it stays unknown. Nothing is guessed, and the result
//! is derived again from the stored evidence after every scan, so new evidence corrects the past.

use crate::analytics::window_minutes;
use crate::model::{Provider, Tool};
use crate::store::Store;
use rusqlite::{params, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Readings of one window name reset times this far apart (seconds): Codex rounds differently
/// from one response to the next.
pub const SAME_WINDOW_S: i64 = 120;
/// A reading may describe the window as it was when its request started: a request still running
/// when its window was reset reports the old one.
const STALE_MS: i64 = 10 * 60_000;
/// Below this a window is unused, and Codex reports one starting at the moment of the reading.
const USED_PCT: f64 = 1.0;
/// One account's weekly use never falls inside a window, except by a reset the provider grants,
/// and a reading at most `STALE_MS` old explains a smaller fall.
const FALL_PCT: f64 = 10.0;
/// Faster than any account climbs (this archive's fastest was half of it): back at the earlier
/// level sooner than this after a fall, the readings are of two accounts.
const CLIMB_PCT_PER_H: f64 = 100.0;
/// The limit every Codex account has. Other limits (a model's own pool) run their own windows
/// beside it, so only this one's windows tell accounts apart.
pub const MAIN_LIMIT: &str = "codex";
/// Owner of a reading that names nothing to tell its account by: an unused window read with a
/// Codex that does not name the account. Left out wherever accounts are told apart.
pub const NAMELESS: &str = "?";

/// Codex reports an unused window as one starting at the moment of the reading, so its end moves
/// with every reading until a request starts it.
fn unused(minutes: i64, reset_s: i64, used_pct: Option<f64>, ts_ms: i64) -> bool {
    used_pct.is_some_and(|u| u < USED_PCT) && reset_s.saturating_mul(1000) - ts_ms >= (minutes * 60 - SAME_WINDOW_S) * 1000
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Account {
    pub provider: Provider,
    /// Digest of the account id; `None` for requests no evidence ties to an account.
    pub id: Option<String>,
    pub first_ms: i64,
    pub last_ms: i64,
    pub requests: u64,
    /// The newest reading belongs to it.
    pub active: bool,
}

/// The Codex accounts in the archive, oldest first. Empty while the archive shows at most one
/// account, so a single account changes nothing in the app.
pub fn accounts(store: &Store) -> Result<Vec<Account>> {
    let mut seen: BTreeMap<Option<String>, Account> = BTreeMap::new();
    let mut note = |id: Option<String>, ts: i64, last: i64, n: u64| {
        let a = seen.entry(id.clone()).or_insert(Account { provider: Provider::OpenAI, id, first_ms: ts, last_ms: last, requests: 0, active: false });
        a.first_ms = a.first_ms.min(ts);
        a.last_ms = a.last_ms.max(last);
        a.requests += n;
    };
    let conn = store.conn();
    let mut st = conn.prepare("SELECT owner, MIN(ts_ms), MAX(ts_ms), COUNT(*) FROM usage_event WHERE tool = 'codex' GROUP BY owner")?;
    for row in st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)?)))? {
        let (id, first, last, n) = row?;
        note(id, first, last, n as u64);
    }
    let mut st = conn.prepare("SELECT owner, MIN(ts_ms), MAX(ts_ms) FROM limit_snapshot WHERE tool = 'codex' AND owner IS NOT ?1 GROUP BY owner")?;
    for row in st.query_map([NAMELESS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (id, first, last) = row?;
        note(id, first, last, 0);
    }
    let named = seen.keys().filter(|k| k.is_some()).count();
    if named + usize::from(seen.contains_key(&None)) < 2 || named == 0 {
        return Ok(Vec::new());
    }
    let newest: Option<Option<String>> = conn
        .query_row(
            "SELECT owner FROM limit_snapshot WHERE tool = 'codex' AND owner IS NOT ?1 AND ts_ms <= ?2 ORDER BY ts_ms DESC LIMIT 1",
            params![NAMELESS, chrono::Utc::now().timestamp_millis()],
            |r| r.get(0),
        )
        .ok();
    let mut out: Vec<Account> = seen.into_values().collect();
    for a in &mut out {
        a.active = newest.as_ref() == Some(&a.id);
    }
    // named accounts by first use; requests no evidence names last
    out.sort_by_key(|a| (a.id.is_none(), a.first_ms));
    Ok(out)
}

struct Request {
    rowid: i64,
    ts_ms: i64,
    thread: String,
    mark: Option<i64>,
    creator: Option<String>,
    owner: Option<String>,
}

/// One reading: the rows a source stored for one moment and limit. `mark` is when the longest
/// window of the main limit resets, `pct` how full it is, `short` when its shorter window in use
/// resets.
struct Reading {
    rowids: Vec<i64>,
    ts_ms: i64,
    source: String,
    main: bool,
    mark: Option<i64>,
    used: bool,
    pct: Option<f64>,
    short: Option<i64>,
    account: Option<String>,
    owner: Option<String>,
}

#[derive(Default)]
struct Window {
    first_ms: i64,
    last_ms: i64,
    /// Accounts named by live reads, and by thread creators.
    named: HashMap<String, u32>,
    vouched: HashMap<String, u32>,
}

/// The one account the evidence names; `Err` when it names two, which only happens when two
/// accounts' windows reset at the same time (as a reset the provider grants to everyone can make
/// them).
fn sole(votes: &HashMap<String, u32>) -> std::result::Result<Option<String>, ()> {
    let mut ids = votes.keys();
    match (ids.next(), ids.next()) {
        (Some(_), Some(_)) => Err(()),
        (one, _) => Ok(one.cloned()),
    }
}

/// One account uses one window at a time and never returns to an earlier one: two windows each
/// read well after the other began belong to different accounts. `spans` are (first, last) reads.
fn overlap(mut spans: Vec<(i64, i64)>) -> bool {
    spans.sort_unstable();
    spans.iter().enumerate().any(|(i, &(first, last))| spans[i + 1..].iter().take_while(|(f, _)| last - f > STALE_MS).any(|&(_, l)| l - first > STALE_MS))
}

/// Reset times chained within the tolerance.
fn chain(mut marks: Vec<i64>) -> Vec<(i64, i64)> {
    marks.sort_unstable();
    marks.dedup();
    let mut bounds: Vec<(i64, i64)> = Vec::new();
    for m in marks {
        match bounds.last_mut() {
            Some((_, hi)) if m - *hi <= SAME_WINDOW_S => *hi = m,
            _ => bounds.push((m, m)),
        }
    }
    bounds
}

/// A reading of a window in use: when, how full, and when its shorter window resets.
type Read = (i64, Option<f64>, Option<i64>);

/// Whether one window's readings are of two accounts whose windows happen to reset together:
/// their shorter windows run side by side, or the use falls and climbs back faster than any
/// account uses it.
fn shared(reads: &mut [Read]) -> bool {
    reads.sort_unstable_by_key(|r| r.0);
    let shorts = chain(reads.iter().filter_map(|r| r.2).collect());
    let mut spans: Vec<(i64, i64)> = shorts.iter().map(|_| (i64::MAX, i64::MIN)).collect();
    for &(ts, _, short) in reads.iter() {
        if let Some(reset) = short {
            let i = shorts.partition_point(|(_, hi)| *hi < reset);
            spans[i] = (spans[i].0.min(ts), spans[i].1.max(ts));
        }
    }
    if overlap(spans) {
        return true;
    }
    let (mut high, mut low) = (f64::MIN, None::<(f64, i64)>);
    for &(ts, pct, _) in reads.iter() {
        let Some(u) = pct else { continue };
        match low {
            Some((l, at)) if u >= high - FALL_PCT => {
                let hours = ((ts - at) as f64 / 3.6e6).max(1e-6);
                if (u - l) / hours > CLIMB_PCT_PER_H {
                    return true;
                }
                low = None;
            }
            Some((l, _)) if u < l => low = Some((u, ts)),
            None if u < high - FALL_PCT => low = Some((u, ts)),
            _ => {}
        }
        high = high.max(u);
    }
    false
}

/// Ties every Codex request and reading to its account (see the module notes) and stores the
/// result where it changed.
pub fn resolve(store: &mut Store) -> Result<()> {
    let (requests, readings) = load(store)?;

    // a window is known from readings of it in use, as reset times chained within the tolerance;
    // an unused reading and a request's own mark only join a known window
    let bounds = chain(readings.iter().filter(|r| r.used).filter_map(|r| r.mark).collect());
    let find = |mark: i64| {
        let i = bounds.partition_point(|(_, hi)| *hi + SAME_WINDOW_S < mark);
        bounds.get(i).is_some_and(|(lo, _)| mark >= lo - SAME_WINDOW_S).then_some(i)
    };
    let req_win: Vec<Option<usize>> = requests.iter().map(|r| r.mark.and_then(find)).collect();
    let read_win: Vec<Option<usize>> = readings.iter().map(|r| r.mark.and_then(find)).collect();

    let mut windows: Vec<Window> = bounds.iter().map(|_| Window { first_ms: i64::MAX, last_ms: i64::MIN, ..Default::default() }).collect();
    for (r, w) in requests.iter().zip(&req_win) {
        let Some(w) = *w else { continue };
        windows[w].first_ms = windows[w].first_ms.min(r.ts_ms);
        windows[w].last_ms = windows[w].last_ms.max(r.ts_ms);
        if let Some(c) = &r.creator {
            *windows[w].vouched.entry(c.clone()).or_default() += 1;
        }
    }
    for (r, w) in readings.iter().zip(&read_win) {
        let Some(w) = *w else { continue };
        windows[w].first_ms = windows[w].first_ms.min(r.ts_ms);
        windows[w].last_ms = windows[w].last_ms.max(r.ts_ms);
        if let Some(a) = &r.account {
            *windows[w].named.entry(a.clone()).or_default() += 1;
        }
    }

    let mut reads: Vec<Vec<Read>> = vec![Vec::new(); windows.len()];
    for (r, w) in readings.iter().zip(&read_win) {
        if let (Some(w), true) = (*w, r.main && r.used) {
            reads[w].push((r.ts_ms, r.pct, r.short));
        }
    }
    // a window two accounts share names neither: only its own evidence ties a request or reading
    let mut several = overlap(windows.iter().map(|w| (w.first_ms, w.last_ms)).collect());
    let mut owner: Vec<Option<String>> = Vec::with_capacity(windows.len());
    for (w, reads) in windows.iter().zip(reads.iter_mut()) {
        let named = if shared(reads) { Err(()) } else { sole(&w.named) };
        let own = match named {
            Ok(None) => sole(&w.vouched),
            other => other,
        };
        several |= own.is_err();
        owner.push(own.ok().flatten());
    }
    let mut known: HashSet<String> = owner.iter().flatten().cloned().collect();
    known.extend(requests.iter().filter_map(|r| r.creator.clone()));
    known.extend(readings.iter().filter_map(|r| r.account.clone()));
    let only = (known.len() == 1 && !several).then(|| known.iter().next().cloned()).flatten();
    if let Some(id) = &only {
        owner.iter_mut().for_each(|o| *o = Some(id.clone()));
    }

    let named = |i: usize| requests[i].creator.is_some() || req_win[i].is_some();
    let mut req_owner: Vec<Option<String>> =
        requests.iter().zip(&req_win).map(|(r, w)| r.creator.clone().or_else(|| w.and_then(|w| owner[w].clone()))).collect();
    // a request that names no window: from the requests around it in its thread, when both sides
    // agree
    let mut by_thread: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in requests.iter().enumerate() {
        by_thread.entry(&r.thread).or_default().push(i);
    }
    for idx in by_thread.values() {
        let around: Vec<Option<String>> = idx.iter().map(|&i| req_owner[i].clone()).collect();
        for (n, &i) in idx.iter().enumerate() {
            if named(i) {
                continue;
            }
            let before = (0..n).rev().find(|&k| named(idx[k])).map(|k| &around[k]);
            let after = (n + 1..idx.len()).find(|&k| named(idx[k])).map(|k| &around[k]);
            req_owner[i] = match (before, after) {
                (Some(Some(a)), Some(Some(b))) if a == b => Some(a.clone()),
                _ => only.clone(),
            };
        }
    }

    // a reading in no known window belongs with the main limit read at the same moment, or with
    // the request reported in the same line
    let mut at: HashMap<i64, HashSet<Option<&str>>> = HashMap::new();
    for (r, o) in requests.iter().zip(&req_owner) {
        at.entry(r.ts_ms).or_default().insert(o.as_deref());
    }
    let own = |r: &Reading, w: Option<usize>| r.account.clone().or_else(|| w.and_then(|w| owner[w].clone()));
    let main_at: HashMap<(i64, &str), Option<String>> = readings
        .iter()
        .zip(&read_win)
        .filter(|(r, w)| r.main && (w.is_some() || r.account.is_some()))
        .map(|(r, w)| ((r.ts_ms, r.source.as_str()), own(r, *w)))
        .collect();
    let read_owner: Vec<Option<String>> = readings
        .iter()
        .zip(&read_win)
        .map(|(r, w)| {
            if r.account.is_some() || w.is_some() {
                return own(r, *w);
            }
            if let Some(o) = main_at.get(&(r.ts_ms, r.source.as_str())) {
                return o.clone();
            }
            match at.get(&r.ts_ms) {
                Some(set) if set.len() == 1 => set.iter().next().copied().flatten().map(str::to_owned),
                _ => Some(only.clone().unwrap_or_else(|| NAMELESS.to_owned())),
            }
        })
        .collect();

    let tx = store.conn().unchecked_transaction()?;
    {
        let mut st = tx.prepare_cached("UPDATE usage_event SET owner = ?2 WHERE rowid = ?1")?;
        for (r, o) in requests.iter().zip(&req_owner) {
            if r.owner != *o {
                st.execute(params![r.rowid, o])?;
            }
        }
        let mut st = tx.prepare_cached("UPDATE limit_snapshot SET owner = ?2 WHERE rowid = ?1")?;
        for (r, o) in readings.iter().zip(&read_owner) {
            if r.owner != *o {
                for id in &r.rowids {
                    st.execute(params![id, o])?;
                }
            }
        }
    }
    tx.commit()
}

fn load(store: &Store) -> Result<(Vec<Request>, Vec<Reading>)> {
    let conn = store.conn();
    let mut st = conn.prepare(
        "SELECT rowid, ts_ms, COALESCE(session_id, '') || '/' || COALESCE(thread_id, ''), limit_reset, creator, owner
         FROM usage_event WHERE tool = ?1 ORDER BY ts_ms, rowid",
    )?;
    let requests = st
        .query_map([Tool::Codex.as_str()], |r| {
            Ok(Request { rowid: r.get(0)?, ts_ms: r.get(1)?, thread: r.get(2)?, mark: r.get(3)?, creator: r.get(4)?, owner: r.get(5)? })
        })?
        .collect::<Result<Vec<_>>>()?;

    let mut st = conn.prepare(
        "SELECT rowid, ts_ms, source, limit_id, window, resets_at, used_pct, account, owner
         FROM limit_snapshot WHERE tool = ?1 ORDER BY ts_ms, source, limit_id",
    )?;
    let mut readings: Vec<Reading> = Vec::new();
    let mut key: Option<(i64, String, String)> = None;
    let mut windows: Vec<(i64, i64, Option<f64>)> = Vec::new();
    for row in st.query_map([Tool::Codex.as_str()], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, Option<f64>>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, Option<String>>(8)?,
        ))
    })? {
        let (rowid, ts, source, limit_id, window, reset, used, account, owner) = row?;
        let k = (ts, source, limit_id);
        if key.as_ref() != Some(&k) {
            let main = k.2 == MAIN_LIMIT;
            readings.push(Reading {
                rowids: Vec::new(),
                ts_ms: ts,
                source: k.1.clone(),
                main,
                mark: None,
                used: false,
                pct: None,
                short: None,
                account: (!account.is_empty()).then_some(account),
                owner,
            });
            key = Some(k);
            windows.clear();
        }
        let r = readings.last_mut().expect("pushed above");
        r.rowids.push(rowid);
        if let (true, Some(m), Some(reset)) = (r.main, window_minutes(&window), reset) {
            windows.push((m, reset, used));
            windows.sort_unstable_by_key(|w| std::cmp::Reverse(w.0));
            let (m, reset, used) = windows[0];
            r.mark = Some(reset);
            r.used = !unused(m, reset, used, ts);
            r.pct = used;
            r.short = windows[1..].iter().rev().find(|(m, reset, used)| !unused(*m, *reset, *used, ts)).map(|w| w.1);
        }
    }
    Ok((requests, readings))
}
