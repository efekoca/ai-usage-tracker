//! Opens Claude's five-hour window at a set time of day. The window starts with the first request
//! and ends five hours later, so one small request early in the morning puts the reset in the
//! middle of the working day. The request goes through the user's own Claude Code.

use crate::settings::{OpenerSettings, Settings};
use crate::state::AppState;
use chrono::{DateTime, Datelike, Duration as Days, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
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

/// The day to run for now: today's time has come, not more than the grace ago, and today's run
/// is not done yet. One that failed (no network right after the computer woke, say) is tried
/// again a few minutes later, three times in all.
fn due<Tz: TimeZone>(o: &OpenerSettings, runs: &[Run], now: &DateTime<Tz>) -> Option<String> {
    let at = scheduled(o, now, 0)?.timestamp_millis();
    let day = day_of(now);
    let now = now.timestamp_millis();
    let today: Vec<&Run> = runs.iter().filter(|r| r.day.as_deref() == Some(&day)).collect();
    let again = today.len() < TRIES && today.iter().all(|r| r.outcome == "failed" && now - r.at_ms >= RETRY_MS);
    ((0..GRACE_MS).contains(&(now - at)) && again).then_some(day)
}

/// The next time it runs, looking a week ahead.
fn next_ms<Tz: TimeZone>(o: &OpenerSettings, runs: &[Run], now: &DateTime<Tz>) -> Option<i64> {
    let ran_today = runs.iter().any(|r| r.day.as_deref() == Some(&day_of(now)));
    (0..=7).filter_map(|d| scheduled(o, now, d)).map(|t| t.timestamp_millis()).find(|&t| t > now.timestamp_millis() - GRACE_MS && !(ran_today && t <= now.timestamp_millis()))
}

/// When Claude's window ends, if the readings show one open now.
fn open_until(limits: &[LimitSnapshot], now: i64) -> Option<i64> {
    limits.iter().filter(|l| l.provider == Provider::Anthropic && l.window == "five_hour").filter_map(LimitSnapshot::resets_at_ms).filter(|&r| r > now).max()
}

/// Sends the request, unless a scheduled run finds a window already open: then there is nothing to open.
fn run(app: &AppHandle, day: Option<String>) -> Run {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let at_ms = now_ms();
    let base = Run { at_ms, day: day.clone(), outcome: "failed".into(), resets_ms: None, tokens: None, error: None };
    if day.is_some()
        && let Some(resets) = open_until(&state.db().latest_limits().unwrap_or_default(), at_ms)
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
        Err(e) => Run { error: Some(e.to_string()), ..base },
    }
}

/// One run at a time; its outcome is kept and the limits are read again.
fn run_and_keep(app: &AppHandle, day: Option<String>) -> Option<Run> {
    let state = app.state::<AppState>();
    {
        let mut b = BOOK.lock().unwrap();
        load(&state, &mut b);
        if b.running {
            return None;
        }
        b.running = true;
    }
    let done = run(app, day);
    log::info!("window opener: {}", done.outcome);
    let mut b = BOOK.lock().unwrap();
    b.running = false;
    b.runs.push(done.clone());
    save(&state, &mut b);
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
}

fn info(state: &AppState) -> OpenerInfo {
    let s = state.settings.read().unwrap().clone();
    let mut b = BOOK.lock().unwrap();
    load(state, &mut b);
    OpenerInfo {
        available: crate::capture::claude_binary(&s).is_some(),
        running: b.running,
        next_ms: s.window_opener.enabled.then(|| next_ms(&s.window_opener, &b.runs, &Local::now())).flatten(),
        runs: b.runs.iter().rev().take(5).cloned().collect(),
    }
}

#[tauri::command]
pub async fn window_opener_info(state: State<'_, AppState>) -> Result<OpenerInfo, String> {
    Ok(info(&state))
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
        Run { at_ms: 0, day: Some(day.into()), outcome: "sent".into(), resets_ms: None, tokens: None, error: None }
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
        let never = OpenerSettings { days: Vec::new(), ..o };
        assert_eq!(next_ms(&never, &[], &at("2026-11-13T05:00:00+03:00")), None);
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
        let limits = [
            limit(Provider::Anthropic, "five_hour", Some(now / 1000 + 600)),
            limit(Provider::Anthropic, "seven_day", Some(now / 1000 + 9_000)),
            limit(Provider::OpenAI, "five_hour", Some(now / 1000 + 9_000)),
        ];
        assert_eq!(open_until(&limits, now), Some(now + 600_000));
        assert_eq!(open_until(&limits, now + 600_000), None, "it ended");
        assert_eq!(open_until(&[limit(Provider::Anthropic, "five_hour", None)], now), None, "no window is open");
    }
}
