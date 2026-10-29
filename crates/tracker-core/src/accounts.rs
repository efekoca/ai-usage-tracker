//! Codex and Claude accounts. Ids are kept only as digests ([`crate::model::account_digest`]).
//! Nothing is guessed: what no evidence ties to an account stays unknown once the archive shows
//! more than one, and the result is derived again from the stored evidence after every scan, so
//! new evidence corrects the past.
//!
//! **Codex** logs the account that started a thread, not the one that sent each request: a
//! desktop account switch or a resume carries a thread over to another account. Every account has
//! its own limit windows, though, and each request reports the window it counted toward, so
//! requests are grouped by window and windows are tied to accounts by evidence:
//! * a live read through `codex app-server` (Codex ≥ 0.163), which names the signed-in account;
//! * the account that started a thread, for the requests it sent right after starting, and while
//!   the thread stays in the same weekly window; not in a week two accounts share (a reset for
//!   everyone can line them up), where a switch changes no window, nor when the evidence puts that
//!   account on another window at the time (a switch before the thread's first request).
//!
//! A window no evidence names belongs to the only account seen, unless the windows show that
//! more than one account was in use; then it stays unknown.
//!
//! **Claude** limits belong to the plan's organisation (a personal plan has its own; a Team seat is
//! another one, even with the same login), and every Claude source names it, so a Claude account
//! is the organisation. Claude Code's logs name none and report no window, so a request is tied
//! to one by evidence, strongest first:
//! * its telemetry event, while the local telemetry receiver is on;
//! * the Claude desktop app's session list, for its Code-tab sessions (the desktop app runs them
//!   with its own sign-in, whatever Claude Code itself is signed in to);
//! * the sign-in of the Claude config folder its log lies in, when the app has only ever seen that
//!   folder signed in to one account (one folder per account, as with `CLAUDE_CONFIG_DIR`), and
//!   saw it without a break from the session's first request to this one. Claude Code keeps the
//!   sign-in it loaded until it next renews it, and logs nothing when a session starts, so after a
//!   change in the same folder no request of a session without telemetry is known for certain.
//!   A backup's folders come in under names of their own, so their sign-ins speak only for the
//!   sessions logged there.
//!
//! Readings: the app's own read names the sign-in it ran with, the desktop history its
//! organisation; any other reading joins the weekly window it shares with a named one.

use crate::analytics::window_minutes;
use crate::model::{Provider, Tool};
use crate::store::Store;
use rusqlite::{params, OptionalExtension, Result};
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

/// Starts the name a folder of another computer gets when its sign-in records are imported.
pub const IMPORTED_ROOT: &str = "import:";

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
    /// The account in use ([`in_use`]).
    pub active: bool,
    /// The plan its newest reading names, as the provider names it (Claude: "pro", "max").
    pub read_plan: Option<String>,
}

/// Which rows of a provider are its requests and its readings.
fn rows_of(provider: Provider) -> (&'static str, &'static str) {
    match provider {
        Provider::Anthropic => ("tool = 'claude_code'", "provider = 'anthropic'"),
        _ => ("tool = 'codex'", "tool = 'codex'"),
    }
}

/// The providers whose accounts are told apart, in the order the app lists providers.
pub const PROVIDERS: [Provider; 2] = [Provider::Anthropic, Provider::OpenAI];

/// Every provider's accounts: Claude first, then Codex, each oldest first. A provider is left out
/// while its archive shows at most one account, so a single account changes nothing in the app.
pub fn accounts(store: &Store) -> Result<Vec<Account>> {
    let mut out = Vec::new();
    for p in PROVIDERS {
        out.extend(provider_accounts(store, p)?);
    }
    Ok(out)
}

/// The providers with more than one account.
pub fn several(store: &Store) -> Result<HashSet<Provider>> {
    Ok(accounts(store)?.into_iter().map(|a| a.provider).collect())
}

fn provider_accounts(store: &Store, provider: Provider) -> Result<Vec<Account>> {
    let mut seen: BTreeMap<Option<String>, Account> = BTreeMap::new();
    let mut note = |id: Option<String>, ts: i64, last: i64, n: u64| {
        let a = seen.entry(id.clone()).or_insert(Account { provider, id, first_ms: ts, last_ms: last, requests: 0, active: false, read_plan: None });
        a.first_ms = a.first_ms.min(ts);
        a.last_ms = a.last_ms.max(last);
        a.requests += n;
    };
    let (requests, readings) = rows_of(provider);
    let conn = store.conn();
    let mut st = conn.prepare(&format!(
        "SELECT owner, MIN(ts_ms), MAX(ts_ms), COUNT(*) FROM usage_event u WHERE {requests} AND {} GROUP BY owner",
        crate::store::NOT_SUPERSEDED
    ))?;
    for row in st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)?)))? {
        let (id, first, last, n) = row?;
        note(id, first, last, n as u64);
    }
    let mut st = conn.prepare(&format!("SELECT owner, MIN(ts_ms), MAX(ts_ms) FROM limit_snapshot WHERE {readings} AND owner IS NOT ?1 GROUP BY owner"))?;
    for row in st.query_map([NAMELESS], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (id, first, last) = row?;
        note(id, first, last, 0);
    }
    let named = seen.keys().filter(|k| k.is_some()).count();
    if named + usize::from(seen.contains_key(&None)) < 2 || named == 0 {
        return Ok(Vec::new());
    }
    // SQLite takes the bare `plan` from the row holding the newest reading; rows dated ahead of the clock are ignored
    let mut st = conn.prepare(&format!(
        "SELECT owner, plan, MAX(ts_ms) FROM limit_snapshot WHERE {readings} AND plan IS NOT NULL AND owner IS NOT NULL AND ts_ms BETWEEN 0 AND ?1 GROUP BY owner"
    ))?;
    for row in st.query_map([crate::store::latest_accepted_ms()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, plan) = row?;
        if let Some(a) = seen.get_mut(&Some(id)) {
            a.read_plan = Some(plan);
        }
    }
    let active = in_use(store, provider, chrono::Utc::now().timestamp_millis())?;
    let mut out: Vec<Account> = seen.into_values().collect();
    for a in &mut out {
        a.active = active.as_ref() == Some(&a.id);
    }
    // named accounts by first use; requests no evidence names last
    out.sort_by_key(|a| (a.id.is_none(), a.first_ms));
    Ok(out)
}

/// Claude's own reads can pause for hours (a long read interval, failures, a throttled service);
/// a newer reading from elsewhere only takes over after this long.
const OWN_READ_LAG_MS: i64 = 24 * 3_600_000;

/// The account in use (`None` inside: an unknown account): the one whose reading is newest. For
/// Claude, the account Claude Code is signed in to right now, once it has readings, and else the
/// app's own read: the desktop app's history can describe another sign-in, and would make the two
/// take turns. Folders signed in to different accounts at once also leave it to the app's own read.
pub fn in_use(store: &Store, provider: Provider, now_ms: i64) -> Result<Option<Option<String>>> {
    let (_, readings) = rows_of(provider);
    if provider == Provider::Anthropic {
        // each folder's latest look; an older sign-in no longer counts
        let signed_in: Vec<String> = store
            .conn()
            .prepare(
                &format!(
                    "SELECT DISTINCT s.account FROM signin_span s
                     WHERE s.account IS NOT NULL AND s.to_ms BETWEEN ?1 AND ?2 AND s.root NOT LIKE '{IMPORTED_ROOT}%'
                       AND s.from_ms = (SELECT max(from_ms) FROM signin_span t WHERE t.root = s.root)"
                ),
            )?
            .query_map(params![now_ms - crate::store::SIGNIN_GAP_MS, now_ms], |r| r.get(0))?
            .collect::<Result<_>>()?;
        if let [account] = signed_in.as_slice()
            && store.conn().query_row("SELECT EXISTS (SELECT 1 FROM limit_snapshot WHERE provider = 'anthropic' AND owner = ?1)", [account], |r| r.get(0))?
        {
            return Ok(Some(Some(account.clone())));
        }
    }
    let newest = |extra: &str| -> Result<Option<(i64, Option<String>)>> {
        store
            .conn()
            .query_row(
                &format!("SELECT ts_ms, owner FROM limit_snapshot WHERE {readings} {extra} AND owner IS NOT ?1 AND ts_ms <= ?2 ORDER BY ts_ms DESC LIMIT 1"),
                params![NAMELESS, now_ms],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
    };
    let any = newest("")?;
    if provider == Provider::Anthropic
        && let (Some(own), Some(any)) = (newest(&format!("AND source = '{}'", crate::capture::claude_usage::SOURCE))?, &any)
        && own.0 >= any.0 - OWN_READ_LAG_MS
    {
        return Ok(Some(own.1));
    }
    Ok(any.map(|n| n.1))
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
/// window of the main limit resets and `began` when it started, `pct` how full it is, `short` when
/// its shorter window in use resets.
struct Reading {
    rowids: Vec<i64>,
    ts_ms: i64,
    source: String,
    main: bool,
    mark: Option<i64>,
    began: Option<i64>,
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
    began_ms: i64,
    /// Accounts named by live reads, and by thread creators.
    named: HashMap<String, u32>,
    vouched: HashMap<String, u32>,
    /// When each of them last showed up here.
    seen: HashMap<String, i64>,
}

impl Window {
    fn saw(&mut self, account: &str, ts_ms: i64) {
        let s = self.seen.entry(account.to_owned()).or_insert(ts_ms);
        *s = (*s).max(ts_ms);
    }
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

/// Ties every request and reading to its account (see the module notes).
pub fn resolve(store: &mut Store) -> Result<()> {
    resolve_codex(store)?;
    resolve_claude(store)
}

/// Ties every Codex request and reading to its account and stores the result where it changed.
pub fn resolve_codex(store: &mut Store) -> Result<()> {
    let tx = settling(store)?;
    let (requests, readings) = (load_codex_requests(store)?, load_readings(store, "tool = 'codex'", |limit, _| limit == MAIN_LIMIT, 0)?);

    // a window is known from readings of it in use, as reset times chained within the tolerance;
    // an unused reading and a request's own mark only join a known window
    let bounds = chain(readings.iter().filter(|r| r.used).filter_map(|r| r.mark).collect());
    let find = |mark: i64| {
        let i = bounds.partition_point(|(_, hi)| *hi + SAME_WINDOW_S < mark);
        bounds.get(i).is_some_and(|(lo, _)| mark >= lo - SAME_WINDOW_S).then_some(i)
    };
    let req_win: Vec<Option<usize>> = requests.iter().map(|r| r.mark.and_then(find)).collect();
    let read_win: Vec<Option<usize>> = readings.iter().map(|r| r.mark.and_then(find)).collect();

    let mut windows: Vec<Window> = bounds.iter().map(|_| Window { first_ms: i64::MAX, last_ms: i64::MIN, began_ms: i64::MAX, ..Default::default() }).collect();
    for (r, w) in requests.iter().zip(&req_win) {
        let Some(w) = *w else { continue };
        windows[w].first_ms = windows[w].first_ms.min(r.ts_ms);
        windows[w].last_ms = windows[w].last_ms.max(r.ts_ms);
        if let Some(c) = &r.creator {
            *windows[w].vouched.entry(c.clone()).or_default() += 1;
            windows[w].saw(c, r.ts_ms);
        }
    }
    for (r, w) in readings.iter().zip(&read_win) {
        let Some(w) = *w else { continue };
        windows[w].first_ms = windows[w].first_ms.min(r.ts_ms);
        windows[w].last_ms = windows[w].last_ms.max(r.ts_ms);
        if let Some(a) = &r.account {
            *windows[w].named.entry(a.clone()).or_default() += 1;
            windows[w].saw(a, r.ts_ms);
        }
        if let (true, Some(b)) = (r.used, r.began) {
            windows[w].began_ms = windows[w].began_ms.min(b);
        }
    }
    // one account's windows follow one another, each beginning after its last sign on the one
    // before: a creator the evidence shows on another window at the time switched before its
    // thread's first request, so its word for this one is set aside
    let follow = |w: usize, o: usize, c: &str| {
        windows[w].seen[c] < windows[o].began_ms.saturating_add(STALE_MS) || windows[o].seen[c] < windows[w].began_ms.saturating_add(STALE_MS)
    };
    let mut belied: HashSet<(usize, &str)> = HashSet::new();
    for (w, win) in windows.iter().enumerate() {
        for c in win.vouched.keys().filter(|c| !win.named.contains_key(*c)) {
            if windows.iter().enumerate().any(|(o, other)| o != w && other.seen.contains_key(c) && !follow(w, o, c)) {
                belied.insert((w, c.as_str()));
            }
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
    let mut contested: Vec<bool> = Vec::with_capacity(windows.len());
    for (i, (w, reads)) in windows.iter().zip(reads.iter_mut()).enumerate() {
        let named = if shared(reads) { Err(()) } else { sole(&w.named) };
        let own = match named {
            // a creator set aside names no window, but still shows another account may use it
            Ok(None) => match sole(&w.vouched) {
                Ok(Some(c)) if belied.contains(&(i, c.as_str())) => Ok(None),
                other => other,
            },
            // a thread's creator in a window a live read gives to another account: both use it
            Ok(Some(a)) if w.vouched.keys().any(|v| *v != a) => Err(()),
            other => other,
        };
        several |= own.is_err();
        contested.push(own.is_err());
        owner.push(own.ok().flatten());
    }
    let mut known: HashSet<String> = owner.iter().flatten().cloned().collect();
    known.extend(requests.iter().filter_map(|r| r.creator.clone()));
    known.extend(readings.iter().filter_map(|r| r.account.clone()));
    let only = (known.len() == 1 && !several).then(|| known.iter().next().cloned()).flatten();
    if let Some(id) = &only {
        owner.iter_mut().for_each(|o| *o = Some(id.clone()));
    }

    // the creator is the account picked when the thread started and vouches for it while the weekly
    // window stays the same, which in a week two accounts share proves nothing: a switch can come
    // at any request, even the first ones, and change neither window. A thread's creator speaks
    // for all of its requests or for none.
    let doubted: HashSet<&str> = requests
        .iter()
        .zip(&req_win)
        .filter_map(|(r, w)| Some((r, (*w)?, r.creator.as_deref()?)))
        .filter(|&(_, w, c)| contested[w] || belied.contains(&(w, c)))
        .map(|(r, _, _)| r.thread.as_str())
        .collect();
    let mut by_thread: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in requests.iter().enumerate() {
        by_thread.entry(&r.thread).or_default().push(i);
    }
    // a windowless request before a window its creator stopped vouching for may follow a switch
    let mut switched: HashSet<usize> = HashSet::new();
    for idx in by_thread.values() {
        let mut later = None;
        for &i in idx.iter().rev() {
            match req_win[i] {
                Some(_) => later = Some(requests[i].creator.is_some()),
                None if later == Some(false) => _ = switched.insert(i),
                None => {}
            }
        }
    }
    let creator = |i: usize| requests[i].creator.clone().filter(|_| !doubted.contains(requests[i].thread.as_str()) && !switched.contains(&i));
    let named = |i: usize| creator(i).is_some() || req_win[i].is_some();
    let mut req_owner: Vec<Option<String>> = (0..requests.len()).map(|i| creator(i).or_else(|| req_win[i].and_then(|w| owner[w].clone()))).collect();
    // a request that names no window: from the requests around it in its thread, when both sides
    // agree
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
    let mut main_at: HashMap<(i64, &str), HashSet<Option<String>>> = HashMap::new();
    for (r, w) in readings.iter().zip(&read_win).filter(|(r, w)| r.main && (w.is_some() || r.account.is_some())) {
        main_at.entry((r.ts_ms, r.source.as_str())).or_default().insert(own(r, *w));
    }
    let read_owner: Vec<Option<String>> = readings
        .iter()
        .zip(&read_win)
        .map(|(r, w)| {
            if r.account.is_some() || w.is_some() {
                return own(r, *w);
            }
            // two accounts read at the same moment leave it unknown which one this belongs to
            if let Some(set) = main_at.get(&(r.ts_ms, r.source.as_str())) {
                return only_one_of(set);
            }
            match at.get(&r.ts_ms) {
                Some(set) if set.len() == 1 => set.iter().next().copied().flatten().map(str::to_owned),
                _ => Some(only.clone().unwrap_or_else(|| NAMELESS.to_owned())),
            }
        })
        .collect();

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

/// Claude sessions the desktop app runs with its own sign-in (its Code tab, its local agent,
/// Cowork) and remote ones.
fn desktop_run(client: &str) -> bool {
    matches!(client, "claude-desktop" | "claude-desktop-3p" | "local-agent" | "cowork") || client.starts_with("remote")
}

/// Sessions whose sign-in may not be the folder's: the desktop app's, and the Agent SDK's, which
/// is often given a key or token of its own.
fn own_sign_in(client: &str) -> bool {
    desktop_run(client) || client.starts_with("sdk")
}

/// Notes the folder a session's log lies in. A session logged in two folders (continued under
/// another config folder) gets none: `MANY_ROOTS`, which no sign-in names.
pub(crate) fn note_root(roots: &mut HashMap<String, String>, session: String, root: String) {
    roots.entry(session).and_modify(|r| if *r != root { MANY_ROOTS.clone_into(r) }).or_insert(root);
}

/// The folder kept for a session logged in more than one.
pub const MANY_ROOTS: &str = "";

/// One spelling of a folder, as its logs' paths and the sign-in records name it.
pub fn root_key(dir: &std::path::Path) -> String {
    dir.components().collect::<std::path::PathBuf>().to_string_lossy().into_owned()
}

/// Holds the archive's write lock from the first read, so a resolve running elsewhere at the same
/// time cannot store owners worked out from what it read before this one wrote.
fn settling(store: &Store) -> Result<rusqlite::Transaction<'_>> {
    rusqlite::Transaction::new_unchecked(store.conn(), rusqlite::TransactionBehavior::Immediate)
}

fn only_one_of(set: &HashSet<Option<String>>) -> Option<String> {
    (set.len() == 1).then(|| set.iter().next().cloned().flatten()).flatten()
}

/// The one value of a set, if it has exactly one.
fn only_one(set: Option<&HashSet<String>>) -> Option<String> {
    let set = set?;
    (set.len() == 1).then(|| set.iter().next().cloned()).flatten()
}

/// The Claude config folder and session of a Claude Code log: `<root>/projects/<project>/<session>.jsonl`,
/// or `…/<session>/subagents/<agent>.jsonl` for a subagent's.
pub fn claude_log_session(path: &std::path::Path) -> Option<(String, String)> {
    let dir = path.parent()?;
    let (session, project_dir) = if dir.file_name()? == "subagents" {
        (dir.parent()?.file_name()?.to_str()?.to_owned(), dir.parent()?.parent()?)
    } else {
        let name = path.file_name()?.to_str()?;
        (name.split_once(".jsonl").map_or(name, |(stem, _)| stem).to_owned(), dir)
    };
    let projects = project_dir.parent()?;
    (projects.file_name()? == "projects").then(|| Some((root_key(projects.parent()?), session))).flatten()
}

struct ClaudeRequest {
    rowid: i64,
    ts_ms: i64,
    session: Option<String>,
    request_id: Option<String>,
    client: Option<String>,
    creator: Option<String>,
    owner: Option<String>,
}

/// Ties every Claude request and reading to its account (see the module notes) and stores the
/// result where it changed.
pub fn resolve_claude(store: &mut Store) -> Result<()> {
    let tx = settling(store)?;
    let conn = store.conn();
    // only a weekly window marks an account's window: a five-hour reading alone (the line Claude
    // Code logs at a five-hour limit) would look like a window of its own
    let readings = load_readings(store, "provider = 'anthropic'", |limit, window| limit.is_empty() && matches!(window, "five_hour" | "seven_day"), 7 * 24 * 60)?;
    let bounds = chain(readings.iter().filter(|r| r.used).filter_map(|r| r.mark).collect());
    let find = |mark: i64| {
        let i = bounds.partition_point(|(_, hi)| *hi + SAME_WINDOW_S < mark);
        bounds.get(i).is_some_and(|(lo, _)| mark >= lo - SAME_WINDOW_S).then_some(i)
    };
    let read_win: Vec<Option<usize>> = readings.iter().map(|r| r.mark.and_then(find)).collect();
    let mut windows: Vec<Window> = bounds.iter().map(|_| Window { first_ms: i64::MAX, last_ms: i64::MIN, ..Default::default() }).collect();
    let mut reads: Vec<Vec<Read>> = vec![Vec::new(); windows.len()];
    for (r, w) in readings.iter().zip(&read_win) {
        let Some(w) = *w else { continue };
        windows[w].first_ms = windows[w].first_ms.min(r.ts_ms);
        windows[w].last_ms = windows[w].last_ms.max(r.ts_ms);
        if let Some(a) = &r.account {
            *windows[w].named.entry(a.clone()).or_default() += 1;
        }
        if r.used {
            reads[w].push((r.ts_ms, r.pct, r.short));
        }
    }
    // a shared window names neither; an idle session's old reading must not make one account two
    let mut owner: Vec<Option<String>> = Vec::with_capacity(windows.len());
    for (w, reads) in windows.iter().zip(reads.iter_mut()) {
        owner.push(if shared(reads) { None } else { sole(&w.named).ok().flatten() });
    }

    // every account any evidence names
    let distinct = |sql: &str| -> Result<Vec<String>> { conn.prepare(sql)?.query_map([], |r| r.get(0))?.collect() };
    let mut known: HashSet<String> = owner.iter().flatten().cloned().collect();
    known.extend(readings.iter().filter_map(|r| r.account.clone()));
    known.extend(distinct("SELECT DISTINCT creator FROM usage_event WHERE tool = 'claude_code' AND creator IS NOT NULL")?);
    known.extend(distinct("SELECT DISTINCT account FROM account_session WHERE tool = 'claude_code'")?);
    known.extend(distinct("SELECT DISTINCT account FROM signin_span WHERE account IS NOT NULL")?);
    if known.len() == 1
        && let Some(id) = known.iter().next()
    {
        tx.execute("UPDATE usage_event SET owner = ?1 WHERE tool = 'claude_code' AND owner IS NOT ?1", [id])?;
        tx.execute("UPDATE limit_snapshot SET owner = ?1 WHERE provider = 'anthropic' AND owner IS NOT ?1", [id])?;
        return tx.commit();
    }

    let req_owner = claude_requests(store)?;
    {
        let mut st = tx.prepare_cached("UPDATE usage_event SET owner = ?2 WHERE rowid = ?1")?;
        for (rowid, was, o) in &req_owner {
            if was != o {
                st.execute(params![rowid, o])?;
            }
        }
        let mut st = tx.prepare_cached("UPDATE limit_snapshot SET owner = ?2 WHERE rowid = ?1")?;
        for (r, w) in readings.iter().zip(&read_win) {
            let o = r.account.clone().or_else(|| w.and_then(|w| owner[w].clone()));
            if r.owner != o {
                for id in &r.rowids {
                    st.execute(params![id, o])?;
                }
            }
        }
    }
    tx.commit()
}

/// A request's row, the owner stored for it, and the owner its evidence names.
type Owned = (i64, Option<String>, Option<String>);

/// Every Claude request, owned by its own evidence.
fn claude_requests(store: &Store) -> Result<Vec<Owned>> {
    let conn = store.conn();
    let requests: Vec<ClaudeRequest> = conn
        .prepare("SELECT rowid, ts_ms, session_id, request_id, client, creator, owner FROM usage_event WHERE tool = 'claude_code' ORDER BY ts_ms, rowid")?
        .query_map([], |r| {
            Ok(ClaudeRequest { rowid: r.get(0)?, ts_ms: r.get(1)?, session: r.get(2)?, request_id: r.get(3)?, client: r.get(4)?, creator: r.get(5)?, owner: r.get(6)? })
        })?
        .collect::<Result<_>>()?;

    let mut told: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut listed: HashMap<String, HashSet<String>> = HashMap::new();
    for row in conn.prepare("SELECT session_id, account FROM account_session WHERE tool = 'claude_code'")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (session, account): (String, String) = row?;
        listed.entry(session).or_default().insert(account);
    }
    // sessions that may not run on their folder's sign-in, and when each one's first request was sent
    let mut desktop: HashSet<&str> = listed.keys().map(String::as_str).collect();
    let mut first: HashMap<&str, i64> = HashMap::new();
    for r in &requests {
        if let (Some(rid), Some(c)) = (&r.request_id, &r.creator) {
            told.entry(rid).or_default().insert(c.clone());
        }
        if let Some(s) = &r.session {
            first.entry(s).or_insert(r.ts_ms);
            if r.client.as_deref().is_some_and(own_sign_in) {
                desktop.insert(s);
            }
        }
    }
    // the folder each session's log lies in (logs read before it was kept: by their checkpoint)
    let mut root_of: HashMap<String, String> = HashMap::new();
    for path in conn.prepare("SELECT path FROM file_checkpoint WHERE parser = 'claude_code_jsonl'")?.query_map([], |r| r.get::<_, String>(0))? {
        if let Some((root, session)) = claude_log_session(std::path::Path::new(&path?)) {
            note_root(&mut root_of, session, root);
        }
    }
    for row in conn.prepare("SELECT session_id, root FROM session_root WHERE tool = 'claude_code'")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (session, root): (String, String) = row?;
        note_root(&mut root_of, session, root);
    }
    let mut spans: HashMap<String, Vec<(i64, i64, Option<String>)>> = HashMap::new();
    for row in conn.prepare("SELECT root, from_ms, to_ms, account FROM signin_span ORDER BY root, from_ms")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))? {
        let (root, from, to, account): (String, i64, i64, Option<String>) = row?;
        spans.entry(root).or_default().push((from, to, account));
    }
    // folders only ever seen signed in to one account
    let single: HashMap<&str, &str> = spans
        .iter()
        .filter_map(|(root, list)| {
            let ids: HashSet<&str> = list.iter().filter_map(|s| s.2.as_deref()).collect();
            (ids.len() == 1).then(|| ids.into_iter().next().map(|id| (root.as_str(), id))).flatten()
        })
        .collect();
    // telemetry on another account shows the session runs on a token of its own, not the folder's
    for r in &requests {
        if let (Some(s), Some(c)) = (&r.session, &r.creator)
            && root_of.get(s).and_then(|root| single.get(root.as_str())).is_some_and(|only| *only != c)
        {
            desktop.insert(s);
        }
    }
    // that account, seen without a break from the session's first request to this one
    let signed_in = |session: &str, ts: i64| -> Option<String> {
        let root = root_of.get(session)?;
        let only = *single.get(root.as_str())?;
        let list = spans.get(root)?;
        let i = list.partition_point(|s| s.0 <= ts).checked_sub(1)?;
        let (from, to, account) = &list[i];
        (ts <= *to && *first.get(session)? >= *from && account.as_deref() == Some(only)).then(|| only.to_owned())
    };

    Ok(requests
        .iter()
        .map(|r| {
            let own = r
                .creator
                .clone()
                .or_else(|| only_one(r.request_id.as_deref().and_then(|rid| told.get(rid))))
                // the desktop app's list speaks for the requests it ran, not for a resume elsewhere
                .or_else(|| only_one(r.session.as_ref().filter(|_| r.client.as_deref().is_some_and(desktop_run)).and_then(|s| listed.get(s))))
                .or_else(|| r.session.as_deref().filter(|s| !desktop.contains(s)).and_then(|s| signed_in(s, r.ts_ms)));
            (r.rowid, r.owner.clone(), own)
        })
        .collect())
}

fn load_codex_requests(store: &Store) -> Result<Vec<Request>> {
    let mut st = store.conn().prepare(
        "SELECT rowid, ts_ms, COALESCE(session_id, '') || '/' || COALESCE(thread_id, ''), limit_reset, creator, owner
         FROM usage_event WHERE tool = ?1 ORDER BY ts_ms, rowid",
    )?;
    st.query_map([Tool::Codex.as_str()], |r| {
        Ok(Request { rowid: r.get(0)?, ts_ms: r.get(1)?, thread: r.get(2)?, mark: r.get(3)?, creator: r.get(4)?, owner: r.get(5)? })
    })?
    .collect()
}

/// The readings matching `rows`, one per moment, source, limit and named account. `main` picks
/// the windows that tell accounts apart; a reading marks its window only by one at least
/// `mark_minutes` long.
fn load_readings(store: &Store, rows: &str, main: fn(&str, &str) -> bool, mark_minutes: i64) -> Result<Vec<Reading>> {
    let mut st = store.conn().prepare(&format!(
        "SELECT rowid, ts_ms, source, limit_id, window, resets_at, used_pct, account, owner
         FROM limit_snapshot WHERE {rows} ORDER BY ts_ms, source, limit_id, account"
    ))?;
    let mut readings: Vec<Reading> = Vec::new();
    let mut key: Option<(i64, String, String, String)> = None;
    let mut windows: Vec<(i64, i64, Option<f64>)> = Vec::new();
    for row in st.query_map([], |r| {
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
        let is_main = main(&limit_id, &window);
        let k = (ts, source, limit_id, account);
        if key.as_ref() != Some(&k) {
            readings.push(Reading {
                rowids: Vec::new(),
                ts_ms: ts,
                source: k.1.clone(),
                main: false,
                mark: None,
                began: None,
                used: false,
                pct: None,
                short: None,
                account: (!k.3.is_empty()).then(|| k.3.clone()),
                owner,
            });
            key = Some(k);
            windows.clear();
        }
        let r = readings.last_mut().expect("pushed above");
        r.rowids.push(rowid);
        r.main |= is_main;
        if let (true, Some(m), Some(reset)) = (is_main, window_minutes(&window), reset) {
            windows.push((m, reset, used));
            windows.sort_unstable_by_key(|w| std::cmp::Reverse(w.0));
            let (m, reset, used) = windows[0];
            if m >= mark_minutes {
                r.mark = Some(reset);
                r.began = Some(reset * 1000 - m * 60_000);
                r.used = !unused(m, reset, used, ts);
                r.pct = used;
            }
            r.short = windows[1..].iter().rev().find(|(m, reset, used)| !unused(*m, *reset, *used, ts)).map(|w| w.1);
        }
    }
    Ok(readings)
}
