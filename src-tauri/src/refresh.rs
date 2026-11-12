//! Refreshing a waiting Claude Code conversation's prompt cache: the hook in Claude Code's settings,
//! the signal to it, refreshes made automatically before a cache runs out, and their tally.

use crate::caches::Cache;
use crate::settings::Settings;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tracker_core::capture::claude_settings as cs;
use tracker_core::capture::refresh_hook as rh;

const LOG_KEY: &str = "cache_refreshes";
const KEEP_MS: i64 = 2 * 86_400_000;
/// A refresh that has not reached the logs by then did not happen.
const PENDING_MS: i64 = 90_000;
/// A request this soon after a signal is the refresh's own turn, not the user's.
const OWN_TURN_MS: i64 = 120_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Refresh {
    at_ms: i64,
    session_id: String,
    usd: Option<f64>,
    auto: bool,
}

#[derive(Default)]
struct Book {
    loaded: bool,
    log: Vec<Refresh>,
    /// Session → when it was signalled and the expiry the refresh is to move on.
    pending: HashMap<String, (i64, i64)>,
    /// Sessions whose automatic refresh was switched by hand, over the setting.
    auto: HashMap<String, bool>,
    /// Session → its user's last own request; automatic refreshes after it count towards the limit.
    user_since: HashMap<String, i64>,
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
    let raw = state.db().setting(LOG_KEY).ok().flatten();
    b.log = raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
}

fn save(state: &AppState, b: &mut Book) {
    let since = now_ms() - KEEP_MS;
    b.log.retain(|r| r.at_ms >= since);
    if let Err(e) = state.db().set_setting(LOG_KEY, &serde_json::to_string(&b.log).unwrap_or_default()) {
        log::warn!("could not keep the cache refreshes: {e}");
    }
}

/// Whether Claude Code's settings hold our hook.
pub fn installed(state: &AppState) -> bool {
    cs::load_state(&state.data_dir).refresh.is_some_and(|c| cs::refresh_hook_in_place(&c))
}

fn auto_for(b: &Book, s: &Settings, session: &str) -> bool {
    b.auto.get(session).copied().unwrap_or(s.cache_auto)
}

/// Adds what the refresh side knows to each cache: whether it can be refreshed now, whether a
/// refresh is on its way, its automatic refresh and how many refreshes it had.
pub fn annotate(state: &AppState, s: &Settings, caches: &mut [Cache], now: i64) {
    let ready = installed(state);
    let dir = rh::dir(&state.data_dir);
    let mut b = BOOK.lock().unwrap();
    load(state, &mut b);
    for c in caches.iter_mut() {
        if b.pending.get(&c.session_id).is_some_and(|&(at, expires)| c.expires_ms > expires || now - at > PENDING_MS) {
            b.pending.remove(&c.session_id);
        }
        c.refreshing = b.pending.contains_key(&c.session_id);
        c.refreshable = ready && !c.refreshing && rh::waiting(&dir, &c.session_id, SystemTime::now());
        c.auto = ready && auto_for(&b, s, &c.session_id);
        let mine: Vec<&Refresh> = b.log.iter().filter(|r| r.session_id == c.session_id).collect();
        c.refreshes = mine.len() as u32;
        c.refreshes_usd = mine.iter().filter_map(|r| r.usd).sum();
        c.auto_capped = c.auto && capped(&mut b, c);
    }
}

/// Automatic refreshes stop once, since the user's last own request, they would cost more than
/// writing the cache again; a model without a price is not refreshed by itself.
fn capped(b: &mut Book, c: &Cache) -> bool {
    let (Some(read), Some(write)) = (c.refresh_usd, c.rewrite_usd) else { return true };
    let last = c.expires_ms - c.ttl_ms;
    let own = b.log.iter().any(|r| r.session_id == c.session_id && r.at_ms <= last && last - r.at_ms < OWN_TURN_MS);
    let since = b.user_since.entry(c.session_id.clone()).or_insert(last);
    if !own && last > *since {
        *since = last;
    }
    let since = *since;
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
    rh::signal(&rh::dir(&state.data_dir), &c.session_id, message(s)).map_err(|e| e.to_string())?;
    log::info!("cache refresh {}", if auto { "(automatic)" } else { "(by hand)" });
    let mut b = BOOK.lock().unwrap();
    load(&state, &mut b);
    let now = now_ms();
    b.pending.insert(c.session_id.clone(), (now, c.expires_ms));
    b.log.push(Refresh { at_ms: now, session_id: c.session_id.clone(), usd: c.refresh_usd, auto });
    save(&state, &mut b);
    drop(b);
    crate::tray::refresh_soon();
    let _ = app.emit("data-changed", ());
    Ok(())
}

/// Called with every tray refresh: refreshes the caches whose time has come, and clears signals
/// no waiter took.
pub fn tick(app: &AppHandle, s: &Settings, caches: &[Cache], now: i64) {
    rh::drop_stale_signals(&rh::dir(&app.state::<AppState>().data_dir), SystemTime::now(), Duration::from_millis(PENDING_MS as u64));
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

#[tauri::command]
pub fn refresh_cache(app: AppHandle, session_id: String) -> Result<(), String> {
    refresh_now(&app, &session_id)
}

/// Switches one conversation's automatic refresh, over the setting, until the app restarts.
#[tauri::command]
pub fn set_cache_auto(app: AppHandle, session_id: String, on: bool) -> Result<(), String> {
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
    let today: Vec<&Refresh> = b.log.iter().filter(|r| r.at_ms >= midnight).collect();
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

/// Adds our hook to Claude Code's settings or takes it out again. Only from the main window, which
/// asks first.
#[tauri::command]
pub async fn set_cache_refresh(window: WebviewWindow, app: AppHandle, on: bool) -> Result<RefreshInfo, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let mut st = cs::read_state(&state.data_dir).map_err(|e| e.to_string())?;
    if on {
        let exe = crate::app_path()?;
        let file = crate::capture::claude_settings_file(&s);
        if st.refresh.as_ref().is_some_and(|c| c.settings_file != file || !cs::refresh_hook_in_place(c)) {
            // a hook the user took out, or one in another settings file, is replaced cleanly
            cs::uninstall_refresh_hook(&mut st, &state.data_dir).map_err(|e| e.to_string())?;
        }
        if st.refresh.is_none() {
            cs::install_refresh_hook(&mut st, &state.data_dir, &file, &crate::capture::cache_hook_command(&exe), now_ms()).map_err(|e| e.to_string())?;
        }
    } else {
        cs::uninstall_refresh_hook(&mut st, &state.data_dir).map_err(|e| e.to_string())?;
    }
    crate::tray::refresh_soon();
    let _ = app.emit("data-changed", ());
    Ok(info(&state))
}

/// Points the hook at this copy of the app when it was moved or reinstalled elsewhere. Debug builds
/// leave it alone, as with the status line.
pub fn follow_app(state: &AppState) {
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(exe) = crate::app_path() else { return };
    let mut st = cs::load_state(&state.data_dir);
    match cs::refresh_refresh_hook(&mut st, &state.data_dir, &crate::capture::cache_hook_command(&exe)) {
        Ok(true) => log::info!("cache refresh hook now points at this copy of the app"),
        Ok(false) => {}
        Err(e) => log::warn!("could not update the cache refresh hook: {e}"),
    }
}
