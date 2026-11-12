//! Refreshing a waiting Claude Code conversation's prompt cache: the hook in Claude Code's settings,
//! the signal to it, refreshes made automatically before a cache runs out, and their tally.

use crate::caches::Cache;
use crate::settings::Settings;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tracker_core::capture::claude_settings as cs;
use tracker_core::capture::refresh_hook as rh;

const LOG_KEY: &str = "cache_refreshes";
const SINCE_KEY: &str = "cache_refresh_since";
/// Long enough that a conversation's automatic refreshes all count until they reach their limit.
const KEEP_MS: i64 = 7 * 86_400_000;
/// A refresh the cache's time has not moved on from by then did not happen.
const PENDING_MS: i64 = 5 * 60_000;
/// A signal no hook took within this long is cleared.
const SIGNAL_MS: u64 = 90_000;
/// How long a failed refresh is shown on its row.
const FAILED_SHOWN_MS: i64 = 10 * 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Refresh {
    at_ms: i64,
    session_id: String,
    usd: Option<f64>,
    auto: bool,
    /// The cache's time moved on after it; until then it was only asked for.
    #[serde(default = "made")]
    done: bool,
}

fn made() -> bool {
    true
}

#[derive(Default)]
struct Book {
    loaded: bool,
    log: Vec<Refresh>,
    /// Session → when it was signalled and the expiry the refresh is to move on.
    pending: HashMap<String, (i64, i64)>,
    /// Session → when its last refresh came to nothing.
    failed: HashMap<String, i64>,
    /// Sessions whose automatic refresh was switched by hand, over the setting.
    auto: HashMap<String, bool>,
    /// Session → when it was first seen, for conversations with no prompt the hook saw: their
    /// automatic refreshes count towards the limit from then.
    since: HashMap<String, i64>,
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
    let read = |key| state.db().setting(key).ok().flatten();
    b.log = read(LOG_KEY).and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    // one still waiting when the app closed cannot be confirmed any more
    b.log.retain(|r| r.done);
    b.since = read(SINCE_KEY).and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
}

fn save(state: &AppState, b: &mut Book) {
    let since = now_ms() - KEEP_MS;
    b.log.retain(|r| r.at_ms >= since);
    b.since.retain(|_, t| *t >= since);
    let store = state.db();
    for (key, value) in [(LOG_KEY, serde_json::to_string(&b.log)), (SINCE_KEY, serde_json::to_string(&b.since))] {
        if let Err(e) = store.set_setting(key, &value.unwrap_or_default()) {
            log::warn!("could not keep the cache refreshes: {e}");
        }
    }
}

/// Where hooks leave their files: the app's own folder and, on Windows, the private copies of it
/// that a packaged app (the Claude desktop app) gives the programs it starts.
fn hook_dirs(state: &AppState) -> Vec<PathBuf> {
    let dirs = vec![rh::dir(&state.data_dir)];
    #[cfg(windows)]
    let dirs = dirs.into_iter().chain(package_copies(&state.data_dir)).collect();
    dirs
}

#[cfg(windows)]
fn package_copies(data_dir: &std::path::Path) -> Vec<PathBuf> {
    let (Some(folder), Some(local)) = (data_dir.file_name(), data_dir.parent()) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(local.join("Packages")) else { return Vec::new() };
    entries.flatten().map(|e| rh::dir(&e.path().join("LocalCache").join("Local").join(folder))).filter(|d| d.is_dir()).collect()
}

fn waiter_dir<'a>(dirs: &'a [PathBuf], session: &str) -> Option<&'a PathBuf> {
    let now = SystemTime::now();
    dirs.iter().find(|d| rh::waiting(d, session, now))
}

/// Whether Claude Code's settings hold our hook.
pub fn installed(state: &AppState) -> bool {
    cs::load_state(&state.data_dir).refresh.is_some_and(|c| cs::refresh_hook_in_place(&c))
}

fn auto_for(b: &Book, s: &Settings, session: &str) -> bool {
    b.auto.get(session).copied().unwrap_or(s.cache_auto)
}

/// A refresh counts once the cache's time moved on after it; one that did not within PENDING_MS
/// came to nothing and is taken out of the log. Returns whether the log changed.
fn settle(b: &mut Book, caches: &[Cache], now: i64) -> bool {
    let waiting: Vec<(String, i64, i64)> = b.pending.iter().map(|(s, &(at, expires))| (s.clone(), at, expires)).collect();
    let mut changed = false;
    for (session, at, expires) in waiting {
        let moved = caches.iter().any(|c| c.session_id == session && c.expires_ms > expires);
        if !moved && now - at <= PENDING_MS {
            continue;
        }
        b.pending.remove(&session);
        if moved {
            b.log.iter_mut().filter(|r| r.session_id == session && r.at_ms == at).for_each(|r| r.done = true);
            b.failed.remove(&session);
        } else {
            b.log.retain(|r| !(r.session_id == session && r.at_ms == at));
            b.failed.insert(session, now);
        }
        changed = true;
    }
    changed
}

/// Adds what the refresh side knows to each cache: whether it can be refreshed now, whether a
/// refresh is on its way or came to nothing, its automatic refresh and how many refreshes it had.
pub fn annotate(state: &AppState, s: &Settings, caches: &mut [Cache], now: i64) {
    let ready = installed(state);
    let dirs = hook_dirs(state);
    let mut b = BOOK.lock().unwrap();
    load(state, &mut b);
    let mut changed = settle(&mut b, caches, now);
    for c in caches.iter_mut() {
        c.refreshing = b.pending.contains_key(&c.session_id);
        c.refresh_failed = !c.refreshing && b.failed.get(&c.session_id).is_some_and(|&at| now - at < FAILED_SHOWN_MS);
        c.refreshable = ready && !c.refreshing && waiter_dir(&dirs, &c.session_id).is_some();
        c.auto = ready && auto_for(&b, s, &c.session_id);
        let made: Vec<&Refresh> = b.log.iter().filter(|r| r.done && r.session_id == c.session_id).collect();
        c.refreshes = made.len() as u32;
        c.refreshes_usd = made.iter().filter_map(|r| r.usd).sum();
        let prompt = dirs.iter().filter_map(|d| rh::last_prompt(d, &c.session_id)).max();
        c.auto_capped = c.auto && capped(&mut b, c, prompt, &mut changed);
    }
    if changed {
        save(state, &mut b);
    }
}

/// Automatic refreshes stop once, since the user's last own prompt, they would cost more than
/// writing the cache again; a model without a price, or priced at nothing, is not refreshed by
/// itself. Refreshes still on their way count too.
fn capped(b: &mut Book, c: &Cache, prompt: Option<i64>, changed: &mut bool) -> bool {
    let (Some(read), Some(write)) = (c.refresh_usd, c.rewrite_usd) else { return true };
    if read <= 0.0 || write <= 0.0 {
        return true;
    }
    let since = match prompt {
        Some(p) => p,
        // a conversation whose prompts the hook has not seen yet counts from when it was first seen
        None => *b.since.entry(c.session_id.clone()).or_insert_with(|| {
            *changed = true;
            c.expires_ms - c.ttl_ms
        }),
    };
    let spent: f64 = b.log.iter().filter(|r| r.session_id == c.session_id && r.auto && r.at_ms > since).filter_map(|r| r.usd).sum();
    spent + read > write
}

fn message(s: &Settings) -> &'static str {
    if crate::tray::turkish(s) {
        "Bu, önbelleğin süresi dolmasın diye AI Usage Tracker'ın gönderdiği otomatik bir mesaj. Hiçbir araç kullanma ve başka bir şey yapma; yalnızca şunu yaz: Önbellek tazelendi."
    } else {
        "This is an automatic message from AI Usage Tracker to keep the prompt cache from running out. Use no tools and do nothing else; reply with exactly: Cache refreshed."
    }
}

fn signal(app: &AppHandle, s: &Settings, c: &Cache, auto: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let dirs = hook_dirs(&state);
    let dir = waiter_dir(&dirs, &c.session_id).ok_or("not_waiting")?;
    rh::signal(dir, &c.session_id, message(s)).map_err(|e| e.to_string())?;
    log::info!("cache refresh {}", if auto { "(automatic)" } else { "(by hand)" });
    let mut b = BOOK.lock().unwrap();
    load(&state, &mut b);
    let now = now_ms();
    b.pending.insert(c.session_id.clone(), (now, c.expires_ms));
    b.failed.remove(&c.session_id);
    b.log.push(Refresh { at_ms: now, session_id: c.session_id.clone(), usd: c.refresh_usd, auto, done: false });
    save(&state, &mut b);
    drop(b);
    crate::tray::refresh_soon();
    let _ = app.emit("data-changed", ());
    Ok(())
}

/// Called with every tray refresh: refreshes the caches whose time has come, and clears signals
/// no waiter took.
pub fn tick(app: &AppHandle, s: &Settings, caches: &[Cache], now: i64) {
    for dir in hook_dirs(&app.state::<AppState>()) {
        rh::drop_stale_signals(&dir, SystemTime::now(), Duration::from_millis(SIGNAL_MS));
    }
    for c in caches.iter().filter(|c| c.auto && c.refreshable && !c.auto_capped) {
        // a 5-minute cache is refreshed in its last minute at most, so it is not refreshed twice as often as it lasts
        let lead = if c.ttl_ms <= 300_000 { 60_000 } else { i64::from(s.cache_lead) * 60_000 };
        let left = c.expires_ms - now;
        if left > 0
            && left <= lead
            && let Err(e) = signal(app, s, c, true)
        {
            log::warn!("automatic cache refresh: {e}");
        }
    }
}

/// Refreshes one conversation now: from its row, the panel or its notification.
pub fn refresh_now(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let caches = crate::caches::followed(&state, &s, now_ms());
    let c = caches.iter().find(|c| c.session_id == session_id).ok_or("cache_gone")?;
    if !installed(&state) {
        return Err("not_installed".into());
    }
    if !c.refreshable {
        return Err(if c.refreshing { "refreshing" } else { "not_waiting" }.into());
    }
    signal(app, &s, c, false)
}

/// From the main window and the panel; async, as it waits for the database.
#[tauri::command]
pub async fn refresh_cache(window: WebviewWindow, app: AppHandle, session_id: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN && window.label() != crate::panel::PANEL {
        return Err("not_allowed".into());
    }
    refresh_now(&app, &session_id)
}

/// Switches one conversation's automatic refresh, over the setting, until the app restarts.
#[tauri::command]
pub async fn set_cache_auto(window: WebviewWindow, app: AppHandle, session_id: String, on: bool) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    BOOK.lock().unwrap().auto.insert(session_id, on);
    crate::tray::refresh_soon();
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[derive(Serialize)]
pub struct RefreshInfo {
    /// Our hook is in Claude Code's settings.
    installed: bool,
    settings_file: String,
    /// Refreshes made today and their API-equivalent cost.
    today: u32,
    today_usd: f64,
}

fn info(state: &AppState) -> RefreshInfo {
    let s = state.settings.read().unwrap().clone();
    let mut b = BOOK.lock().unwrap();
    load(state, &mut b);
    let midnight = chrono::Local::now().date_naive().and_hms_opt(0, 0, 0).and_then(|t| t.and_local_timezone(chrono::Local).earliest()).map_or(0, |t| t.timestamp_millis());
    let today: Vec<&Refresh> = b.log.iter().filter(|r| r.done && r.at_ms >= midnight).collect();
    RefreshInfo {
        installed: installed(state),
        settings_file: crate::capture::claude_settings_file(&s).to_string_lossy().into_owned(),
        today: today.len() as u32,
        today_usd: today.iter().filter_map(|r| r.usd).sum(),
    }
}

#[tauri::command]
pub async fn cache_refresh_info(state: State<'_, AppState>) -> Result<RefreshInfo, String> {
    Ok(info(&state))
}

/// Adds our hook to Claude Code's settings, or brings one already there up to date. A record of a
/// hook the user took out, or of one in another settings file, is replaced cleanly.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
    let command = crate::capture::cache_hook_command(&crate::app_path()?);
    let file = crate::capture::claude_settings_file(&s);
    if st.refresh.as_ref().is_some_and(|c| c.settings_file != file || !cs::refresh_hook_in_place(c)) {
        cs::uninstall_refresh_hook(&mut st, &state.data_dir).map_err(|e| e.to_string())?;
    }
    if st.refresh.is_none() {
        cs::install_refresh_hook(&mut st, &state.data_dir, &file, &command, now_ms()).map_err(|e| e.to_string())?;
    } else {
        cs::refresh_refresh_hook(&mut st, &state.data_dir, &command).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Adds our hook to Claude Code's settings or takes it out again. Only from the main window, which
/// asks first. Taking it out also turns automatic refresh off, so adding it again for one refresh
/// does not start them all.
#[tauri::command]
pub async fn set_cache_refresh(window: WebviewWindow, app: AppHandle, on: bool) -> Result<RefreshInfo, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let state = app.state::<AppState>();
    if on {
        install(&app)?;
    } else {
        let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
        cs::uninstall_refresh_hook(&mut st, &state.data_dir).map_err(|e| e.to_string())?;
        if state.settings.read().unwrap().cache_auto {
            let s = state.update_settings(|s| s.cache_auto = false)?;
            let _ = app.emit("settings-changed", &s);
        }
    }
    crate::tray::refresh_soon();
    let _ = app.emit("data-changed", ());
    Ok(info(&state))
}

/// "Delete all data" takes the refresh log and the prompt times the hooks kept too.
pub fn forget(state: &AppState) {
    let mut b = BOOK.lock().unwrap();
    *b = Book { loaded: true, ..Default::default() };
    save(state, &mut b);
    drop(b);
    for dir in hook_dirs(state) {
        rh::forget_prompts(&dir);
    }
}

/// Points the hook at this copy of the app when it was moved or reinstalled elsewhere, and adds
/// what an older install lacks. Debug builds leave it alone, as with the status line.
pub fn follow_app(state: &AppState) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(exe) = crate::app_path() else { return };
    let mut st = cs::load_state(&state.data_dir);
    match cs::refresh_refresh_hook(&mut st, &state.data_dir, &crate::capture::cache_hook_command(&exe)) {
        Ok(true) => log::info!("cache refresh hook brought up to date"),
        Ok(false) => {}
        Err(e) => log::warn!("could not update the cache refresh hook: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(session: &str, expires_ms: i64) -> Cache {
        Cache { session_id: session.into(), ttl_ms: 3_600_000, expires_ms, refresh_usd: Some(0.25), rewrite_usd: Some(1.0), ..Cache::default() }
    }

    fn auto_at(b: &mut Book, session: &str, at_ms: i64) {
        b.log.push(Refresh { at_ms, session_id: session.into(), usd: Some(0.25), auto: true, done: true });
    }

    #[test]
    fn automatic_refreshes_stop_at_a_rewrites_cost_and_start_again_after_a_prompt() {
        let mut b = Book::default();
        // its last request was at 6_400_000
        let c = cache("s", 10_000_000);
        let mut changed = false;
        for at in [6_000_000, 7_000_000, 8_000_000] {
            auto_at(&mut b, "s", at);
        }
        assert!(!capped(&mut b, &c, None, &mut changed), "counted from when it was first seen: two, and the next costs less than a rewrite");
        assert!(changed, "when it was first seen is kept");
        // after a restart only what was saved is known
        let mut restarted = Book { since: b.since.clone(), log: b.log.clone(), ..Book::default() };
        auto_at(&mut restarted, "s", 9_000_000);
        assert!(!capped(&mut restarted, &c, None, &mut changed), "three and the next make exactly a rewrite");
        auto_at(&mut restarted, "s", 9_500_000);
        assert!(capped(&mut restarted, &c, None, &mut changed), "four and the next cost more, across the restart");
        assert!(!capped(&mut restarted, &c, Some(9_600_000), &mut changed), "the user wrote again");
        assert!(capped(&mut restarted, &c, Some(6_500_000), &mut changed), "an older prompt does not start the count again");
    }

    #[test]
    fn a_model_priced_at_nothing_is_not_refreshed_by_itself() {
        let mut changed = false;
        let free = Cache { refresh_usd: Some(0.0), rewrite_usd: Some(0.0), ..cache("s", 1) };
        assert!(capped(&mut Book::default(), &free, Some(0), &mut changed));
        let unknown = Cache { refresh_usd: None, ..cache("s", 1) };
        assert!(capped(&mut Book::default(), &unknown, Some(0), &mut changed));
    }

    #[test]
    fn a_refresh_counts_once_the_cache_moves_on_and_one_that_did_not_is_dropped() {
        let mut b = Book::default();
        for (s, at) in [("a", 1_000), ("b", 1_000)] {
            b.pending.insert(s.into(), (at, 50_000));
            b.log.push(Refresh { at_ms: at, session_id: s.into(), usd: Some(0.2), auto: true, done: false });
        }
        assert!(!settle(&mut b, &[cache("a", 50_000), cache("b", 50_000)], 2_000), "still on its way");
        assert!(settle(&mut b, &[cache("a", 3_650_000), cache("b", 50_000)], 3_000));
        assert!(b.log.iter().any(|r| r.session_id == "a" && r.done));
        assert!(settle(&mut b, &[cache("b", 50_000)], 1_000 + PENDING_MS + 1));
        assert!(b.log.iter().all(|r| r.session_id != "b"), "a refresh that came to nothing is not counted");
        assert!(b.failed.contains_key("b") && b.pending.is_empty());
    }
}
