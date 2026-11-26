//! Opens Claude's five-hour window at a set time of day. The window starts with the first request
//! and ends five hours later, so one small request early in the morning puts the reset in the
//! middle of the working day. The request goes through the user's own Claude Code.

use crate::settings::{OpenerSettings, Settings};
use crate::state::AppState;
use chrono::{DateTime, Datelike, Duration as Days, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tracker_core::capture::claude_usage;
use tracker_core::discovery::Env;
use tracker_core::model::{LimitSnapshot, Provider};

const LOG_KEY: &str = "window_opener_runs";
const KEPT: usize = 30;
/// A time missed by more than this (the computer slept, the app was closed) is let go: the
/// window would open too late for what the time was chosen for.
const GRACE_MS: i64 = 15 * 60_000;
const TIMEOUT: Duration = Duration::from_secs(60);
const TRIES: usize = 3;
const RETRY_MS: i64 = 3 * 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Run {
    at_ms: i64,
    /// The local day it ran for, `YYYY-MM-DD`; `None` for one started by hand.
    day: Option<String>,
    /// "sent": the request went out; "open": a window was already open, so none was sent; "failed".
    outcome: String,
    /// When the window ends, as Claude named it.
    resets_ms: Option<i64>,
    /// The request's tokens, in and out.
    tokens: Option<u64>,
    error: Option<String>,
    /// The account the request went with, when its sign-in was known.
    #[serde(default)]
    account: Option<String>,
}

#[derive(Default)]
struct Book {
    loaded: bool,
    runs: Vec<Run>,
    running: bool,
}

static BOOK: LazyLock<Mutex<Book>> = LazyLock::new(Default::default);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn load(state: &AppState, b: &mut Book) {
    if b.loaded {
        return;
    }
    b.loaded = true;
    b.runs = state.db().setting(LOG_KEY).ok().flatten().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
}

fn save(state: &AppState, b: &mut Book) {
    let extra = b.runs.len().saturating_sub(KEPT);
    b.runs.drain(..extra);
    if let Err(e) = state.db().set_setting(LOG_KEY, &serde_json::to_string(&b.runs).unwrap_or_default()) {
        log::warn!("could not keep the window opener's runs: {e}");
    }
}

fn day_of<Tz: TimeZone>(t: &DateTime<Tz>) -> String {
    t.date_naive().format("%Y-%m-%d").to_string()
}

/// The moment the opener runs on the day `days` after `now`'s, if it runs that day. A time the
/// clock skips that day (daylight saving) has none.
fn scheduled<Tz: TimeZone>(o: &OpenerSettings, now: &DateTime<Tz>, days: i64) -> Option<DateTime<Tz>> {
    let date = now.date_naive() + Days::days(days);
    if !o.days.contains(&(date.weekday().number_from_monday() as u8)) {
        return None;
    }
    let time = NaiveTime::from_num_seconds_from_midnight_opt(u32::from(o.minute) * 60, 0)?;
    date.and_time(time).and_local_timezone(now.timezone()).earliest()
}

/// The tries already made for the run of the day of `at`.
fn tries_of<'a, Tz: TimeZone>(runs: &'a [Run], at: &DateTime<Tz>) -> Vec<&'a Run> {
    let day = day_of(at);
    runs.iter().filter(|r| r.day.as_deref() == Some(&day)).collect()
}

/// The day to run for now: its time has come, not more than the grace ago, and its run is not
/// done yet. One that failed (no network right after the computer woke, say) is tried again a
/// few minutes later, three times in all. A time just before midnight keeps both into the next day.
fn due<Tz: TimeZone>(o: &OpenerSettings, runs: &[Run], now: &DateTime<Tz>) -> Option<String> {
    let now_ms = now.timestamp_millis();
    [0, -1].into_iter().find_map(|d| {
        let at = scheduled(o, now, d)?;
        let tries = tries_of(runs, &at);
        let again = tries.len() < TRIES && tries.iter().all(|r| r.outcome == "failed" && now_ms - r.at_ms >= RETRY_MS);
        ((0..GRACE_MS).contains(&(now_ms - at.timestamp_millis())) && again).then(|| day_of(&at))
    })
}

/// The next time it runs, looking a week ahead, by the rule [`due`] goes by: a day's run is over
/// once one went out or found a window open, or after the last try; a try that failed is made
/// again a little later, while the time is not missed.
fn next_ms<Tz: TimeZone>(o: &OpenerSettings, runs: &[Run], now: &DateTime<Tz>) -> Option<i64> {
    let now_ms = now.timestamp_millis();
    (-1..=7).filter_map(|d| Some((d, scheduled(o, now, d)?))).find_map(|(d, at)| {
        let t = at.timestamp_millis();
        if d > 0 {
            return Some(t);
        }
        let tries = tries_of(runs, &at);
        let over = tries.len() >= TRIES || tries.iter().any(|r| r.outcome != "failed");
        let again = tries.iter().map(|r| r.at_ms + RETRY_MS).max().map_or(t, |a| a.max(t));
        (!over && now_ms - t < GRACE_MS && again - t < GRACE_MS).then_some(again)
    })
}

/// When the window the opener's own earlier request opened for `account` ends, if it is still open.
fn opened_until(runs: &[Run], account: Option<&str>, now: i64) -> Option<i64> {
    runs.iter().filter(|r| r.outcome == "sent" && account.is_some() && r.account.as_deref() == account).filter_map(|r| r.resets_ms).filter(|&r| r > now).max()
}

/// A failure in words the card can show in the user's language.
fn error_code(e: &str) -> String {
    match e {
        e if e.starts_with("claude did not answer in time") => "claude_timeout".into(),
        e if e.starts_with("claude stopped before answering") => "claude_stopped".into(),
        e if e.starts_with("cannot start claude") => "claude_not_started".into(),
        e => e.to_owned(),
    }
}

/// When the window of the account the request would go to ends, if its readings show one open
/// now. `target` is that account when its sign-in is known; another account's open window says
/// nothing about it. Without a known account the readings count only while they are all one
/// account's.
fn open_until(limits: &[(LimitSnapshot, Option<String>)], target: Option<&str>, now: i64) -> Option<i64> {
    let five: Vec<&(LimitSnapshot, Option<String>)> = limits.iter().filter(|(l, _)| l.provider == Provider::Anthropic && l.window == "five_hour").collect();
    let owners: std::collections::BTreeSet<Option<&str>> = five.iter().map(|(_, o)| o.as_deref()).collect();
    let counts = |owner: Option<&str>| match target {
        Some(t) => owner == Some(t),
        None => owners.len() == 1,
    };
    five.iter().filter(|(_, o)| counts(o.as_deref())).filter_map(|(l, _)| l.resets_at_ms()).filter(|&r| r > now).max()
}

/// Sends the request, unless a scheduled run finds a window already open: then there is nothing to open.
fn run(app: &AppHandle, day: Option<String>, earlier: &[Run]) -> Run {
    use crate::capture::CliSignIn;
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let at_ms = now_ms();
    let sign_in = crate::capture::claude_cli_sign_in(&s);
    let account = match &sign_in {
        CliSignIn::Account(a) => Some(a.clone()),
        _ => None,
    };
    let base = Run { at_ms, day: day.clone(), outcome: "failed".into(), resets_ms: None, tokens: None, error: None, account: account.clone() };
    // a request billed by use opens no plan window
    if sign_in == CliSignIn::OwnKey {
        return Run { error: Some("claude_own_key".into()), ..base };
    }
    let open = || open_until(&state.db().latest_owned_limits().unwrap_or_default(), account.as_deref(), at_ms).or_else(|| opened_until(earlier, account.as_deref(), at_ms));
    if day.is_some()
        && let Some(resets) = open()
    {
        return Run { outcome: "open".into(), resets_ms: Some(resets), ..base };
    }
    let Some(bin) = crate::capture::claude_binary(&s) else { return Run { error: Some("claude_not_found".into()), ..base } };
    let work = state.data_dir.join("claude-usage");
    if let Err(e) = std::fs::create_dir_all(&work) {
        return Run { error: Some(e.to_string()), ..base };
    }
    let config = claude_usage::config_dir(&Env::from_system(), &s.extra_paths);
    match claude_usage::open_window(&bin, &work, config.as_deref(), TIMEOUT) {
        Ok(o) => Run { outcome: "sent".into(), resets_ms: o.resets_at.map(|r| r * 1000), tokens: Some(o.tokens), ..base },
        Err(e) => Run { error: Some(error_code(&e.to_string())), ..base },
    }
}

/// One run at a time; its outcome is kept and the limits are read again.
fn run_and_keep(app: &AppHandle, day: Option<String>) -> Option<Run> {
    let state = app.state::<AppState>();
    let earlier = {
        let mut b = BOOK.lock().unwrap();
        load(&state, &mut b);
        if b.running {
            return None;
        }
        b.running = true;
        b.runs.clone()
    };
    let wipes = state.capture.wipes.load(std::sync::atomic::Ordering::SeqCst);
    let done = run(app, day, &earlier);
    log::info!("window opener: {}", done.outcome);
    let mut b = BOOK.lock().unwrap();
    b.running = false;
    // "Delete all data" came while it ran: its record goes with the rest
    if state.capture.wipes.load(std::sync::atomic::Ordering::SeqCst) == wipes {
        b.runs.push(done.clone());
        save(&state, &mut b);
    }
    drop(b);
    crate::capture::wake_claude(app);
    let _ = app.emit("data-changed", ());
    Some(done)
}

/// Called with every tray refresh: starts the day's run when its time has come.
pub fn tick(app: &AppHandle, s: &Settings, _now: i64) {
    if !s.onboarded || !s.window_opener.enabled {
        return;
    }
    let state = app.state::<AppState>();
    let day = {
        let mut b = BOOK.lock().unwrap();
        load(&state, &mut b);
        if b.running {
            return;
        }
        due(&s.window_opener, &b.runs, &Local::now())
    };
    if let Some(day) = day {
        let app = app.clone();
        // the request can take a while; the tray goes on meanwhile
        std::thread::spawn(move || run_and_keep(&app, Some(day)));
    }
}

#[derive(Debug, Serialize)]
pub struct OpenerInfo {
    /// Claude Code was found, so a request can be sent.
    available: bool,
    running: bool,
    /// The next time it runs; `None` while it is off.
    next_ms: Option<i64>,
    /// The latest runs, the newest first.
    runs: Vec<Run>,
    /// The hours Claude is used on the days it runs, and when to open a window for them.
    day: Option<tracker_core::insights::WorkingDay>,
}

fn info(state: &AppState) -> OpenerInfo {
    let s = state.settings.read().unwrap().clone();
    // the record only: the tray's tick waits on it, not on the reads below
    let (running, runs) = {
        let mut b = BOOK.lock().unwrap();
        load(state, &mut b);
        (b.running, b.runs.clone())
    };
    OpenerInfo {
        available: crate::capture::claude_binary(&s).is_some(),
        running,
        next_ms: s.window_opener.enabled.then(|| next_ms(&s.window_opener, &runs, &Local::now())).flatten(),
        runs: runs.iter().rev().take(5).cloned().collect(),
        day: tracker_core::insights::working_day(&state.db(), Local::now().timestamp_millis(), &Local, &s.window_opener.days).ok().flatten(),
    }
}

#[tauri::command]
pub async fn window_opener_info(app: AppHandle) -> Result<OpenerInfo, String> {
    tauri::async_runtime::spawn_blocking(move || info(&app.state::<AppState>())).await.map_err(|e| e.to_string())
}

/// Sends the request now, whatever the time: from the main window, to try it out.
#[tauri::command]
pub async fn open_window_now(window: WebviewWindow, app: AppHandle) -> Result<OpenerInfo, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || run_and_keep(&worker, None)).await.map_err(|e| e.to_string())?.ok_or("running")?;
    Ok(info(&app.state::<AppState>()))
}

/// "Delete all data" takes the record of runs too.
pub fn forget(state: &AppState) {
    let mut b = BOOK.lock().unwrap();
    b.runs.clear();
    b.loaded = true;
    save(state, &mut b);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use tracker_core::model::{Accuracy, Tool};

    fn at(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).unwrap()
    }

    fn ran(day: &str) -> Run {
        Run { at_ms: 0, day: Some(day.into()), outcome: "sent".into(), resets_ms: None, tokens: None, error: None, account: None }
    }

    #[test]
    fn it_runs_once_at_its_time_on_its_days_and_lets_a_missed_time_go() {
        let o = OpenerSettings { enabled: true, minute: 6 * 60, days: vec![1, 2, 3, 4, 5], ..Default::default() };
        // 2026-11-13 is a Friday
        assert_eq!(due(&o, &[], &at("2026-11-13T05:59:59+03:00")), None, "not yet");
        assert_eq!(due(&o, &[], &at("2026-11-13T06:00:00+03:00")).as_deref(), Some("2026-11-13"));
        assert_eq!(due(&o, &[], &at("2026-11-13T06:14:59+03:00")).as_deref(), Some("2026-11-13"), "a little late still counts");
        assert_eq!(due(&o, &[], &at("2026-11-13T06:15:00+03:00")), None, "the computer woke too late");
        assert_eq!(due(&o, &[ran("2026-11-13")], &at("2026-11-13T06:00:20+03:00")), None, "once a day");
        assert_eq!(due(&o, &[ran("2026-11-12")], &at("2026-11-13T06:00:20+03:00")).as_deref(), Some("2026-11-13"));
        assert_eq!(due(&o, &[Run { day: None, ..ran("") }], &at("2026-11-13T06:00:20+03:00")).as_deref(), Some("2026-11-13"), "one started by hand is not the day's");
        assert_eq!(due(&o, &[], &at("2026-11-14T06:00:00+03:00")), None, "not on a Saturday");
        // a run that failed is tried again a little later, three times in all
        let failed = |clock: &str| Run { at_ms: at(clock).timestamp_millis(), outcome: "failed".into(), ..ran("2026-11-13") };
        let first = failed("2026-11-13T06:00:10+03:00");
        assert_eq!(due(&o, std::slice::from_ref(&first), &at("2026-11-13T06:02:00+03:00")), None, "too soon");
        assert_eq!(due(&o, std::slice::from_ref(&first), &at("2026-11-13T06:03:10+03:00")).as_deref(), Some("2026-11-13"));
        let two = [first.clone(), failed("2026-11-13T06:03:20+03:00")];
        assert_eq!(due(&o, &two, &at("2026-11-13T06:06:20+03:00")).as_deref(), Some("2026-11-13"));
        let three = [two[0].clone(), two[1].clone(), failed("2026-11-13T06:06:30+03:00")];
        assert_eq!(due(&o, &three, &at("2026-11-13T06:09:30+03:00")), None, "three tries are enough");
        let sent = [first, Run { at_ms: at("2026-11-13T06:03:20+03:00").timestamp_millis(), ..ran("2026-11-13") }];
        assert_eq!(due(&o, &sent, &at("2026-11-13T06:09:30+03:00")), None, "the second try went out");
    }

    #[test]
    fn the_next_run_is_the_next_chosen_day() {
        let o = OpenerSettings { enabled: true, minute: 6 * 60 + 30, days: vec![1, 5], ..Default::default() };
        let ms = |t: &str| Some(at(t).timestamp_millis());
        assert_eq!(next_ms(&o, &[], &at("2026-11-13T05:00:00+03:00")), ms("2026-11-13T06:30:00+03:00"));
        assert_eq!(next_ms(&o, &[], &at("2026-11-13T06:35:00+03:00")), ms("2026-11-13T06:30:00+03:00"), "due now");
        assert_eq!(next_ms(&o, &[ran("2026-11-13")], &at("2026-11-13T06:35:00+03:00")), ms("2026-11-16T06:30:00+03:00"), "done for today: Monday");
        assert_eq!(next_ms(&o, &[], &at("2026-11-13T07:00:00+03:00")), ms("2026-11-16T06:30:00+03:00"), "missed today");
        assert_eq!(next_ms(&o, &[], &at("2026-11-16T06:30:00+03:00")), ms("2026-11-16T06:30:00+03:00"));
        let never = OpenerSettings { days: Vec::new(), ..o.clone() };
        assert_eq!(next_ms(&never, &[], &at("2026-11-13T05:00:00+03:00")), None);

        // the time moved to later after today's run went out: nothing more is sent today, so none is promised
        let afternoon = OpenerSettings { minute: 15 * 60, ..o.clone() };
        assert_eq!(next_ms(&afternoon, &[ran("2026-11-13")], &at("2026-11-13T10:00:00+03:00")), ms("2026-11-16T15:00:00+03:00"));
        assert_eq!(due(&afternoon, &[ran("2026-11-13")], &at("2026-11-13T15:00:10+03:00")), None);
        // a try that failed is made again in a few minutes, and the card says so
        let failed = |clock: &str| Run { at_ms: at(clock).timestamp_millis(), outcome: "failed".into(), ..ran("2026-11-13") };
        assert_eq!(next_ms(&o, &[failed("2026-11-13T06:30:20+03:00")], &at("2026-11-13T06:31:00+03:00")), ms("2026-11-13T06:33:20+03:00"));
        let thrice = [failed("2026-11-13T06:30:20+03:00"), failed("2026-11-13T06:33:30+03:00"), failed("2026-11-13T06:36:40+03:00")];
        assert_eq!(next_ms(&o, &thrice, &at("2026-11-13T06:40:00+03:00")), ms("2026-11-16T06:30:00+03:00"), "out of tries");
    }

    #[test]
    fn a_time_just_before_midnight_keeps_its_grace_and_its_tries_into_the_next_day() {
        let o = OpenerSettings { enabled: true, minute: 23 * 60 + 55, days: vec![1, 2, 3, 4, 5, 6, 7], ..Default::default() };
        let ms = |t: &str| Some(at(t).timestamp_millis());
        assert_eq!(due(&o, &[], &at("2026-11-14T00:05:00+03:00")).as_deref(), Some("2026-11-13"), "the computer woke ten minutes late");
        let failed = Run { at_ms: at("2026-11-13T23:57:00+03:00").timestamp_millis(), outcome: "failed".into(), ..ran("2026-11-13") };
        assert_eq!(next_ms(&o, std::slice::from_ref(&failed), &at("2026-11-13T23:58:00+03:00")), ms("2026-11-14T00:00:00+03:00"));
        assert_eq!(due(&o, std::slice::from_ref(&failed), &at("2026-11-14T00:00:10+03:00")).as_deref(), Some("2026-11-13"), "the try the card promised");
        assert_eq!(due(&o, &[ran("2026-11-13")], &at("2026-11-14T00:05:00+03:00")), None, "yesterday's went out");
        assert_eq!(due(&o, &[], &at("2026-11-14T00:10:00+03:00")), None, "too late for yesterday's");
        assert_eq!(next_ms(&o, &[ran("2026-11-13")], &at("2026-11-14T00:05:00+03:00")), ms("2026-11-14T23:55:00+03:00"));
    }

    #[test]
    fn the_openers_own_request_keeps_the_next_one_from_going_out_twice() {
        let now = 1_000_000_000;
        let sent = |account: Option<&str>, resets_ms| Run { outcome: "sent".into(), resets_ms, account: account.map(Into::into), ..ran("2026-11-13") };
        let runs = [sent(Some("me"), Some(now + 600_000)), sent(Some("other"), Some(now + 9_000_000)), sent(None, Some(now + 9_000_000))];
        assert_eq!(opened_until(&runs, Some("me"), now), Some(now + 600_000));
        assert_eq!(opened_until(&runs, Some("me"), now + 600_000), None, "it ended");
        assert_eq!(opened_until(&runs, None, now), None, "whose window it was is not known");
        assert_eq!(opened_until(&[Run { outcome: "failed".into(), ..sent(Some("me"), Some(now + 600_000)) }], Some("me"), now), None);
        assert_eq!(error_code("claude did not answer in time"), "claude_timeout");
        assert_eq!(error_code("claude stopped before answering"), "claude_stopped");
        assert_eq!(error_code("cannot start claude: No such file"), "claude_not_started");
        assert_eq!(error_code("claude_limit_full"), "claude_limit_full");
    }

    #[test]
    fn a_window_is_open_while_a_reading_names_a_reset_still_to_come() {
        let limit = |provider, window: &str, resets_at| LimitSnapshot {
            ts_ms: 0,
            provider,
            tool: Tool::ClaudeCode,
            account: None,
            limit_id: String::new(),
            window: window.into(),
            used_pct: Some(10.0),
            resets_at,
            status: None,
            plan: None,
            source: "claude_code_usage".into(),
            accuracy: Accuracy::Captured,
        };
        let now = 1_000_000_000;
        let mine = |l: LimitSnapshot| (l, Some("me".to_owned()));
        let limits = [
            mine(limit(Provider::Anthropic, "five_hour", Some(now / 1000 + 600))),
            mine(limit(Provider::Anthropic, "seven_day", Some(now / 1000 + 9_000))),
            mine(limit(Provider::OpenAI, "five_hour", Some(now / 1000 + 9_000))),
        ];
        assert_eq!(open_until(&limits, Some("me"), now), Some(now + 600_000));
        assert_eq!(open_until(&limits, None, now), Some(now + 600_000), "one account's readings need no name");
        assert_eq!(open_until(&limits, Some("me"), now + 600_000), None, "it ended");
        assert_eq!(open_until(&[mine(limit(Provider::Anthropic, "five_hour", None))], Some("me"), now), None, "no window is open");

        // another account's open window does not stand for the one the request goes to
        let other = (limit(Provider::Anthropic, "five_hour", Some(now / 1000 + 3_600)), Some("other".to_owned()));
        assert_eq!(open_until(std::slice::from_ref(&other), Some("me"), now), None);
        let two = [limits[0].clone(), other.clone()];
        assert_eq!(open_until(&two, Some("me"), now), Some(now + 600_000));
        assert_eq!(open_until(&two, Some("other"), now), Some(now + 3_600_000));
        assert_eq!(open_until(&two, None, now), None, "which of the two the request goes to is not known");
        let unowned = (limit(Provider::Anthropic, "five_hour", Some(now / 1000 + 600)), None);
        assert_eq!(open_until(std::slice::from_ref(&unowned), Some("me"), now), None, "a reading no account is tied to is not mine for sure");
        assert_eq!(open_until(&[unowned], None, now), Some(now + 600_000));
    }
}
