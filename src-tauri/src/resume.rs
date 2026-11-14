//! Continuing a Claude Code conversation the usage limit stopped: the hook in Claude Code's
//! settings that waits in it, the moment its limit starts again, and the signal that wakes it.

use crate::settings::Settings;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tracker_core::capture::claude_settings as cs;
use tracker_core::capture::refresh_hook as rh;
use tracker_core::model::{LimitStop, Tool};
use tracker_core::store::Store;

const LOG_KEY: &str = "limit_resumes";
const KEEP_MS: i64 = 7 * 86_400_000;
const DAY_MS: i64 = 86_400_000;
/// The limit names its reset to the second; the conversation goes on a little after it.
const MARGIN_MS: i64 = 60_000;
/// How far the limit's own record of the stop may lie from the moment the hook saw it.
const NEAR_MS: i64 = 2 * 60_000;
/// A conversation that went on longer ago than this is not told about any more.
const TOLD_MS: i64 = 5 * 60_000;
/// A signal the hook did not take within this long is sent again.
const SIGNAL_MS: i64 = 90_000;
/// A conversation the limit stops again right after it went on is not continued sooner than this.
const APART_MS: i64 = 15 * 60_000;
/// More in a day than the limits' windows allow: something else stops the conversation.
const DAY_MAX: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Done {
    session_id: String,
    at_ms: i64,
}

#[derive(Default)]
struct Book {
    loaded: bool,
    /// Conversations continued, kept so one is not continued again and again.
    log: Vec<Done>,
    /// Session → when it was signalled, until the hook takes it.
    signaled: HashMap<String, i64>,
    /// Session and the time it stopped → the reset found for it and the limit's window.
    resets: HashMap<(String, i64), (i64, String)>,
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
    b.log = state.db().setting(LOG_KEY).ok().flatten().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
}

fn save(state: &AppState, b: &mut Book) {
    let keep = now_ms() - KEEP_MS;
    b.log.retain(|d| d.at_ms >= keep);
    if let Err(e) = state.db().set_setting(LOG_KEY, &serde_json::to_string(&b.log).unwrap_or_default()) {
        log::warn!("could not keep the continued conversations: {e}");
    }
}

/// The reset the limit itself named when it turned this conversation's request down, and its
/// window: the nearest of the conversation's own stops, and of two at the same distance the one
/// that resets later. Another conversation's stop, on this account or another, tells nothing
/// about this one; without a record of its own the reset is not known, and nothing is guessed.
fn rejection(stops: &[LimitStop], stopped_ms: i64) -> Option<(i64, String)> {
    stops
        .iter()
        .filter(|s| (s.ts_ms - stopped_ms).abs() <= NEAR_MS)
        .min_by_key(|s| ((s.ts_ms - stopped_ms).abs(), std::cmp::Reverse(s.resets_at)))
        .map(|s| (s.resets_at * 1000, s.window.clone()))
}

fn reset_of(store: &Store, b: &mut Book, session: &str, stopped_ms: i64) -> Option<(i64, String)> {
    let key = (session.to_owned(), stopped_ms);
    if let Some(found) = b.resets.get(&key) {
        return Some(found.clone());
    }
    let stops = store.limit_stops(session, stopped_ms - NEAR_MS, stopped_ms + NEAR_MS + 1).unwrap_or_else(|e| {
        log::warn!("the limit that stopped a conversation: {e}");
        Vec::new()
    });
    let found = rejection(&stops, stopped_ms)?;
    b.resets.insert(key, found.clone());
    Some(found)
}

fn last_done(log: &[Done], session: &str) -> Option<i64> {
    log.iter().filter(|d| d.session_id == session).map(|d| d.at_ms).max()
}

/// When a conversation whose limit starts again at `reset_ms` is continued.
fn due_ms(reset_ms: i64, last_done: Option<i64>) -> i64 {
    (reset_ms + MARGIN_MS).max(last_done.map_or(i64::MIN, |t| t + APART_MS))
}

fn held(log: &[Done], session: &str, now: i64) -> bool {
    log.iter().filter(|d| d.session_id == session && now - d.at_ms < DAY_MS).count() >= DAY_MAX
}

/// Whether Claude Code's settings hold our hook.
pub fn installed(state: &AppState) -> bool {
    cs::load_state(&state.data_dir).resume.is_some_and(|c| cs::refresh_hook_in_place(&c))
}

/// The project a conversation runs in, its branch and where it runs, as the caches name them.
fn named(state: &AppState, s: &Settings, session: &str) -> crate::caches::Cache {
    let store = state.db();
    let last = store.session_events(session).unwrap_or_default().into_iter().rev().find(|e| e.tool == Tool::ClaudeCode && e.agent.is_none());
    let project = last.as_ref().and_then(|e| e.project_id).and_then(|id| store.projects().unwrap_or_default().into_iter().find(|p| p.id == id));
    let hidden = s.hide_project_names || project.as_ref().is_none_or(|p| p.hidden);
    crate::caches::Cache {
        session_id: session.to_owned(),
        project: if hidden { String::new() } else { project.map(|p| p.name).unwrap_or_default() },
        branch: last.as_ref().and_then(|e| e.branch.clone()).filter(|_| !hidden),
        client: last.and_then(|e| e.client),
        ..Default::default()
    }
}

/// The conversations that went on since the last look, as their hooks wrote down; read from the
/// hooks' files, so one that went on just before the app closed is still counted. A signal not
/// taken in time is forgotten, so it is sent again while the hook still waits.
fn taken(b: &mut Book, dirs: &[PathBuf], now: i64) -> Vec<Done> {
    let mut went: Vec<Done> = dirs
        .iter()
        .flat_map(|d| rh::all_resumed(d))
        .map(|(session_id, at_ms)| Done { session_id, at_ms })
        .filter(|d| d.at_ms > now - KEEP_MS && !b.log.contains(d))
        .collect();
    went.sort_by_key(|d| d.at_ms);
    went.dedup();
    b.signaled.retain(|session, at| !went.iter().any(|d| &d.session_id == session && d.at_ms >= *at) && now - *at <= SIGNAL_MS);
    b.log.extend(went.iter().cloned());
    went
}

/// Wakes the waiting conversations whose limit has started again.
fn wake_due(store: &Store, b: &mut Book, dirs: &[PathBuf], now: i64, clock: SystemTime) {
    for dir in dirs {
        for session in rh::limit_waiters(dir, clock) {
            if b.signaled.contains_key(&session) || held(&b.log, &session, now) {
                continue;
            }
            let Some(stopped) = rh::stopped_at(dir, &session) else { continue };
            let Some((reset, _)) = reset_of(store, b, &session, stopped) else { continue };
            if now < due_ms(reset, last_done(&b.log, &session)) {
                continue;
            }
            // the hook wakes the conversation with words of its own
            match rh::signal(dir, rh::Wait::Limit, &session, "") {
                Ok(()) => {
                    log::info!("continuing a conversation after its limit reset");
                    b.signaled.insert(session, now);
                }
                Err(e) => log::warn!("could not continue a conversation: {e}"),
            }
        }
    }
}

/// Called with every tray refresh: notes the conversations that went on, and wakes those whose
/// limit has started again.
pub fn tick(app: &AppHandle, s: &Settings, now: i64) {
    if !s.limit_resume {
        return;
    }
    let state = app.state::<AppState>();
    let dirs = crate::refresh::hook_dirs(&state);
    let mut b = BOOK.lock().unwrap();
    load(&state, &mut b);
    let went = taken(&mut b, &dirs, now);
    if !went.is_empty() {
        save(&state, &mut b);
    }
    wake_due(&state.db(), &mut b, &dirs, now, SystemTime::now());
    drop(b);
    // what happened while the app was closed is noted, not told
    for d in went.iter().filter(|d| now - d.at_ms < TOLD_MS) {
        let label = crate::caches::label(&named(&state, s, &d.session_id), crate::tray::turkish(s));
        crate::alerts::resumed(app, s, &d.session_id, label);
    }
    if !went.is_empty() {
        let _ = app.emit("data-changed", ());
    }
}

#[derive(Debug, Serialize)]
pub struct Waiting {
    session_id: String,
    /// The project's name; empty while it is hidden or not known.
    project: String,
    branch: Option<String>,
    client: Option<String>,
    stopped_ms: i64,
    /// The limit that stopped it: `five_hour`, `seven_day`, …; `None` while its record is not read yet.
    window: Option<String>,
    /// When it goes on; `None` while the limit's reset is not known.
    resumes_ms: Option<i64>,
    /// Continued too often in a day to be continued again by itself.
    held: bool,
}

#[derive(Debug, Serialize)]
pub struct Resumed {
    session_id: String,
    project: String,
    branch: Option<String>,
    at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct ResumeInfo {
    /// Our hook is in Claude Code's settings.
    installed: bool,
    settings_file: String,
    /// Conversations the limit stopped whose hook waits for the reset, the soonest first.
    waiting: Vec<Waiting>,
    /// Those continued in the last day, the latest first.
    resumed: Vec<Resumed>,
}

fn info(state: &AppState) -> ResumeInfo {
    let s = state.settings.read().unwrap().clone();
    let now = now_ms();
    let dirs = crate::refresh::hook_dirs(state);
    let clock = SystemTime::now();
    let mut b = BOOK.lock().unwrap();
    load(state, &mut b);
    let mut waiting = Vec::new();
    for dir in &dirs {
        for session in rh::limit_waiters(dir, clock) {
            let Some(stopped_ms) = rh::stopped_at(dir, &session) else { continue };
            let found = reset_of(&state.db(), &mut b, &session, stopped_ms);
            let c = named(state, &s, &session);
            waiting.push(Waiting {
                project: c.project,
                branch: c.branch,
                client: c.client,
                stopped_ms,
                resumes_ms: found.as_ref().map(|(reset, _)| due_ms(*reset, last_done(&b.log, &session))),
                window: found.map(|(_, w)| w),
                held: held(&b.log, &session, now),
                session_id: session,
            });
        }
    }
    waiting.sort_by(|a, b| a.resumes_ms.unwrap_or(i64::MAX).cmp(&b.resumes_ms.unwrap_or(i64::MAX)).then_with(|| a.session_id.cmp(&b.session_id)));
    let mut done: Vec<Done> = b.log.iter().filter(|d| now - d.at_ms < DAY_MS).cloned().collect();
    drop(b);
    done.sort_by_key(|d| std::cmp::Reverse(d.at_ms));
    let resumed = done
        .into_iter()
        .map(|d| {
            let c = named(state, &s, &d.session_id);
            Resumed { session_id: d.session_id, project: c.project, branch: c.branch, at_ms: d.at_ms }
        })
        .collect();
    ResumeInfo { installed: installed(state), settings_file: crate::capture::claude_settings_file(&s).to_string_lossy().into_owned(), waiting, resumed }
}

#[tauri::command]
pub async fn limit_resume_info(state: State<'_, AppState>) -> Result<ResumeInfo, String> {
    Ok(info(&state))
}

/// Adds our hook to Claude Code's settings, or brings one already there up to date. A record of a
/// hook the user took out, or of one in another settings file, is replaced cleanly.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let _one_at_a_time = crate::capture::state_lock();
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
    let command = crate::capture::limit_hook_command(&crate::app_path()?);
    let file = crate::capture::claude_settings_file(&s);
    if st.resume.as_ref().is_some_and(|c| c.settings_file != file || !cs::refresh_hook_in_place(c)) {
        cs::uninstall_hook(&mut st, cs::Hook::Resume, &state.data_dir).map_err(|e| e.to_string())?;
    }
    if st.resume.is_none() {
        cs::install_hook(&mut st, cs::Hook::Resume, &state.data_dir, &file, &command, now_ms()).map_err(|e| e.to_string())?;
    } else {
        cs::refresh_hook(&mut st, cs::Hook::Resume, &state.data_dir, &command).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Turns continuing after a limit's reset on or off, and with it our hook in Claude Code's
/// settings. Only from the main window, which asks first. Turning it off also sends away the
/// hooks that wait.
#[tauri::command]
pub async fn set_limit_resume(window: WebviewWindow, app: AppHandle, on: bool) -> Result<ResumeInfo, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let state = app.state::<AppState>();
    if on {
        install(&app)?;
    } else {
        let guard = crate::capture::state_lock();
        let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
        cs::uninstall_hook(&mut st, cs::Hook::Resume, &state.data_dir).map_err(|e| e.to_string())?;
        drop(guard);
        // every hook goes, one not heard from lately too: it would be woken if the switch went on again
        for dir in crate::refresh::hook_dirs(&state) {
            for session in rh::limit_hooks(&dir) {
                rh::retire_limit(&dir, &session);
            }
        }
        BOOK.lock().unwrap().signaled.clear();
    }
    let s = state.update_settings(|s| s.limit_resume = on)?;
    let _ = app.emit("settings-changed", &s);
    let _ = app.emit("data-changed", ());
    Ok(info(&state))
}

/// Calls off one conversation: its hook goes, and it stays where the limit stopped it.
#[tauri::command]
pub async fn cancel_limit_resume(window: WebviewWindow, app: AppHandle, session_id: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    if !rh::valid_session(&session_id) {
        return Err("not_waiting".into());
    }
    let state = app.state::<AppState>();
    for dir in crate::refresh::hook_dirs(&state) {
        rh::retire_limit(&dir, &session_id);
    }
    BOOK.lock().unwrap().signaled.remove(&session_id);
    let _ = app.emit("data-changed", ());
    Ok(())
}

/// "Delete all data" takes the record of continued conversations too, and sends away the hooks
/// that wait: what told when their conversations stopped is gone.
pub fn forget(state: &AppState) {
    for dir in crate::refresh::hook_dirs(state) {
        for session in rh::limit_hooks(&dir) {
            rh::retire_limit(&dir, &session);
        }
    }
    let mut b = BOOK.lock().unwrap();
    *b = Book { loaded: true, ..Default::default() };
    save(state, &mut b);
}

/// Points the hook at this copy of the app when it was moved or reinstalled elsewhere. Debug
/// builds leave it alone, as with the status line.
pub fn follow_app(state: &AppState) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(exe) = crate::app_path() else { return };
    let _one_at_a_time = crate::capture::state_lock();
    let mut st = cs::load_state(&state.data_dir);
    match cs::refresh_hook(&mut st, cs::Hook::Resume, &state.data_dir, &crate::capture::limit_hook_command(&exe)) {
        Ok(true) => log::info!("limit hook brought up to date"),
        Ok(false) => {}
        Err(e) => log::warn!("could not update the limit hook: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stop(session: &str, ts_ms: i64, window: &str, resets_at: i64) -> LimitStop {
        LimitStop { session_id: session.into(), ts_ms, window: window.into(), resets_at }
    }

    fn keep(store: &mut Store, s: LimitStop) {
        let mut tx = store.transaction().unwrap();
        tx.insert_limit_stops(&[s]).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn the_reset_is_the_one_the_limit_named_when_it_stopped_the_conversation() {
        let stopped = 1_000_000_000;
        let stops = [stop("s", stopped - 90_000, "seven_day", 2_000_000), stop("s", stopped - 400, "five_hour", 1_018_000)];
        assert_eq!(rejection(&stops, stopped), Some((1_018_000_000, "five_hour".into())), "the nearest one");
        assert_eq!(rejection(&[stop("s", stopped - NEAR_MS - 1, "five_hour", 1_018_000)], stopped), None, "an older stop is another one");
        assert_eq!(rejection(&[], stopped), None);
        // both windows turned the same request down: it goes on once the later one starts again
        let both = [stop("s", stopped, "five_hour", 1_018_000), stop("s", stopped, "seven_day", 2_000_000)];
        assert_eq!(rejection(&both, stopped), Some((2_000_000_000, "seven_day".into())));
    }

    #[test]
    fn a_conversation_waits_for_its_own_limit_not_for_one_that_stopped_another() {
        use std::time::{Duration, Instant};
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("refresh");
        let dirs = [dir.clone()];
        let mut store = Store::open(&d.path().join("t.db")).unwrap();
        let t = 1_700_000_000_000;
        let (long, short) = (t + 4 * 3_600_000, t + 600_000);
        // two accounts stopped half a second apart; the other one's limit starts again far sooner
        keep(&mut store, stop("a", t - 1_000, "five_hour", long / 1000));
        keep(&mut store, stop("b", t + 500, "five_hour", short / 1000));
        let wait = |session: &'static str| {
            let dir = dir.clone();
            let hook = std::thread::spawn(move || rh::wait(&dir, rh::Wait::Limit, session, session, None, Duration::from_millis(10), Instant::now() + Duration::from_secs(10), || true).unwrap());
            while !rh::waiting(&dirs[0], rh::Wait::Limit, session, SystemTime::now()) {
                std::thread::sleep(Duration::from_millis(10));
            }
            hook
        };
        let (a, b, c) = (wait("a"), wait("b"), wait("c"));
        rh::stopped(&dir, "a", t);
        rh::stopped(&dir, "b", t + 500);
        // stopped at the same moment, with no stop of its own in the logs
        rh::stopped(&dir, "c", t + 200);
        let mut book = Book::default();
        wake_due(&store, &mut book, &dirs, short + MARGIN_MS, SystemTime::now());
        assert_eq!(book.signaled.keys().collect::<Vec<_>>(), ["b"]);
        assert_eq!(b.join().unwrap(), rh::Outcome::Wake(String::new()));
        wake_due(&store, &mut book, &dirs, long + MARGIN_MS, SystemTime::now());
        assert!(book.signaled.contains_key("a") && !book.signaled.contains_key("c"));
        assert_eq!(a.join().unwrap(), rh::Outcome::Wake(String::new()));
        rh::retire_limit(&dir, "c");
        assert_eq!(c.join().unwrap(), rh::Outcome::Moved);
    }

    #[test]
    fn a_waiting_conversation_is_woken_once_its_limit_started_again() {
        use std::time::{Duration, Instant};
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("refresh");
        let dirs = [dir.clone()];
        let mut store = Store::open(&d.path().join("t.db")).unwrap();
        let stopped = 1_700_000_000_000;
        let reset = stopped + 3 * 3_600_000;
        keep(&mut store, stop("s", stopped - 800, "five_hour", reset / 1000));
        let wait = |me: &'static str| {
            let dir = dir.clone();
            let hook = std::thread::spawn(move || rh::wait(&dir, rh::Wait::Limit, "s", me, None, Duration::from_millis(10), Instant::now() + Duration::from_secs(10), || true).unwrap());
            while !rh::waiting(&dirs[0], rh::Wait::Limit, "s", SystemTime::now()) {
                std::thread::sleep(Duration::from_millis(10));
            }
            hook
        };
        let mut b = Book::default();

        let hook = wait("a");
        // before the hook wrote down when it stopped, and before the reset, nothing is sent
        wake_due(&store, &mut b, &dirs, reset + MARGIN_MS, SystemTime::now());
        rh::stopped(&dir, "s", stopped);
        wake_due(&store, &mut b, &dirs, reset + MARGIN_MS - 1, SystemTime::now());
        assert!(b.signaled.is_empty());
        wake_due(&store, &mut b, &dirs, reset + MARGIN_MS, SystemTime::now());
        assert_eq!(hook.join().unwrap(), rh::Outcome::Wake(String::new()));
        assert!(taken(&mut b, &dirs, reset + MARGIN_MS + 5_000).is_empty() && b.signaled.contains_key("s"), "sent, and not yet known to have gone on");
        rh::resumed(&dir, "s", reset + MARGIN_MS + 6_000);
        let went = [Done { session_id: "s".into(), at_ms: reset + MARGIN_MS + 6_000 }];
        assert_eq!(taken(&mut b, &dirs, reset + MARGIN_MS + 20_000), went);
        assert!(b.signaled.is_empty() && b.log == went);
        assert!(taken(&mut b, &dirs, reset + MARGIN_MS + 40_000).is_empty(), "counted once");
        // an app that closed right after it sent the signal still counts it at its next start
        let mut restarted = Book::default();
        assert_eq!(taken(&mut restarted, &dirs, reset + DAY_MS), went);
        assert!(taken(&mut Book::default(), &dirs, reset + KEEP_MS + DAY_MS).is_empty(), "too long ago to keep");

        // the limit stops it again at once: the same reset is not a reason to go on again right away
        let again = reset + MARGIN_MS + 30_000;
        keep(&mut store, stop("s", again, "five_hour", reset / 1000));
        let hook = wait("b");
        rh::stopped(&dir, "s", again);
        wake_due(&store, &mut b, &dirs, again + APART_MS - 60_000, SystemTime::now());
        assert!(b.signaled.is_empty());
        // a signal the hook does not take is sent again
        b.signaled.insert("s".into(), again);
        assert!(taken(&mut b, &dirs, again + SIGNAL_MS + 1).is_empty() && b.signaled.is_empty());
        wake_due(&store, &mut b, &dirs, again + APART_MS + 10_000, SystemTime::now());
        assert_eq!(hook.join().unwrap(), rh::Outcome::Wake(String::new()));

        // one whose stop the logs do not hold is left alone
        let hook = wait("c");
        rh::stopped(&dir, "s", again + DAY_MS);
        b.signaled.clear();
        wake_due(&store, &mut b, &dirs, again + 9 * DAY_MS, SystemTime::now());
        assert!(b.signaled.is_empty());
        rh::retire_limit(&dir, "s");
        assert_eq!(hook.join().unwrap(), rh::Outcome::Moved);
    }

    #[test]
    fn a_conversation_goes_on_a_little_after_the_reset_and_not_again_and_again() {
        let reset = 10 * DAY_MS;
        assert_eq!(due_ms(reset, None), reset + MARGIN_MS);
        assert_eq!(due_ms(reset, Some(reset - DAY_MS)), reset + MARGIN_MS, "continued long before");
        assert_eq!(due_ms(reset, Some(reset + MARGIN_MS)), reset + MARGIN_MS + APART_MS, "stopped again right after it went on");
        let done = |at_ms| Done { session_id: "s".into(), at_ms };
        let now = 20 * DAY_MS;
        let mut log: Vec<Done> = (1..DAY_MAX as i64).map(|i| done(now - i * 3_600_000)).collect();
        log.push(done(now - DAY_MS - 1));
        log.push(Done { session_id: "other".into(), at_ms: now - 1 });
        assert!(!held(&log, "s", now), "one fewer than a day allows; older ones and other conversations do not count");
        assert_eq!(last_done(&log, "s"), Some(now - 3_600_000));
        log.push(done(now - 60_000));
        assert!(held(&log, "s", now));
        assert!(!held(&log, "s", now + DAY_MS));
    }
}
