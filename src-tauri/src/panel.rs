//! The panel of tiles the tray icon opens: placed against the icon, closed when it
//! loses focus.

use crate::state::AppState;
use crate::tray_text::Usage;
use crate::windows::{handle_menu, show_main_at, supports_mica, window_theme};
use serde::Serialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracker_core::analytics::{self, LimitState};
use tracker_core::insights::Forecast;
use tracker_core::model::Provider;

pub const PANEL: &str = "panel";
const WIDTH: f64 = 304.0;
/// Logical pixels between the icon and the panel, and the panel and the screen edge.
const GAP: f64 = 6.0;

/// The icon the panel opened from, physical `(x, y, w, h)`; `None` places it top right.
static ANCHOR: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
static HIDDEN_MS: AtomicI64 = AtomicI64::new(0);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn toggle(app: &AppHandle, rect: Option<Rect>) {
    if let Some(w) = app.get_webview_window(PANEL)
        && w.is_visible().unwrap_or(false)
    {
        hide(&w);
        return;
    }
    // the click on the icon took focus away and so already closed the panel
    if now_ms() - HIDDEN_MS.load(Ordering::SeqCst) < 300 {
        return;
    }
    *ANCHOR.lock().unwrap() = rect.map(|r| {
        let p = r.position.to_physical::<i32>(1.0);
        let s = r.size.to_physical::<i32>(1.0);
        (p.x, p.y, s.width, s.height)
    });
    let Some(w) = ensure(app) else { return };
    place(&w);
    let _ = w.show();
    let _ = w.set_focus();
}

fn hide(w: &WebviewWindow) {
    HIDDEN_MS.store(now_ms(), Ordering::SeqCst);
    let _ = w.hide();
}

fn ensure(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(PANEL) {
        return Some(w);
    }
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let native = cfg!(target_os = "macos") || supports_mica();
    let mut b = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("panel.html".into()))
        .title("AI Usage Tracker")
        .inner_size(WIDTH, 420.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        // the system draws the shadow and rounds the corners only around its own material
        .shadow(native)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .theme(window_theme(&s))
        .visible(false);
    if cfg!(target_os = "macos") {
        b = b.effects(EffectsBuilder::new().effect(Effect::Popover).state(EffectState::Active).radius(18.0).build());
    } else if supports_mica() {
        b = b.effects(EffectsBuilder::new().effect(Effect::Mica).build());
    }
    match b.build() {
        Ok(w) => {
            let win = w.clone();
            w.on_window_event(move |e| match e {
                tauri::WindowEvent::Focused(false) => hide(&win),
                // the page sizes the window to its modules; it stays against the icon
                tauri::WindowEvent::Resized(_) => place(&win),
                _ => {}
            });
            Some(w)
        }
        Err(e) => {
            log::error!("cannot create the panel: {e}");
            None
        }
    }
}

fn place(w: &WebviewWindow) {
    let Ok(size) = w.outer_size() else { return };
    let (pw, ph) = (size.width as i32, size.height as i32);
    let anchor = *ANCHOR.lock().unwrap();
    let monitors = w.available_monitors().unwrap_or_default();
    let monitor = anchor
        .and_then(|(x, y, aw, ah)| {
            let (cx, cy) = (x + aw / 2, y + ah / 2);
            monitors.iter().find(|m| {
                let (p, s) = (m.position(), m.size());
                cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
            })
        })
        .cloned()
        .or_else(|| w.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let gap = (GAP * m.scale_factor()).round() as i32;
    let area = m.work_area();
    let (left, top) = (area.position.x, area.position.y);
    let (right, bottom) = (left + area.size.width as i32, top + area.size.height as i32);
    let (x, y) = match anchor {
        Some((ax, ay, aw, ah)) => {
            let x = ax + aw / 2 - pw / 2;
            // below an icon in the top half of the screen (macOS menu bar), above one lower down
            let below = ay + ah / 2 < m.position().y + m.size().height as i32 / 2;
            (x, if below { ay + ah + gap } else { ay - ph - gap })
        }
        None => (right - pw - gap, top + gap),
    };
    let x = x.clamp(left + gap, (right - pw - gap).max(left + gap));
    let y = y.clamp(top + gap, (bottom - ph - gap).max(top + gap));
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

pub fn apply_theme(app: &AppHandle, s: &crate::settings::Settings) {
    if let Some(w) = app.get_webview_window(PANEL) {
        let _ = w.set_theme(window_theme(s));
    }
}

#[derive(Serialize)]
pub struct PanelLimit {
    provider: Provider,
    limit_id: String,
    window: String,
    used_pct: Option<f64>,
    state: LimitState,
    resets_at: Option<i64>,
    forecast: Option<Forecast>,
}

#[derive(Serialize)]
pub struct PanelData {
    limits: Vec<PanelLimit>,
    /// Today, the last 7 and the last 30 days.
    usage: Option<[Usage; 3]>,
    last_scan_ms: Option<i64>,
    scanning: bool,
    widget_visible: bool,
    update: Option<String>,
    providers: Vec<Provider>,
    /// Signed out of the tool the limits are read through, with no current reading left.
    signed_out: Vec<Provider>,
}

/// Everything the panel's modules show; the Menu bar page's preview reads it too.
#[tauri::command]
pub async fn panel_data(app: AppHandle) -> Result<PanelData, String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let providers = crate::commands::enabled_providers(&s);
    let limits = {
        let store = state.db();
        let book = state.book.read().unwrap();
        analytics::limits_view(&store, &book, now_ms(), &s.thresholds).map_err(|e| e.to_string())?
    };
    let live = |p: Provider| limits.iter().any(|l| l.provider == p && l.used_pct.is_some() && matches!(l.state, LimitState::Fresh | LimitState::Behind));
    let cap = crate::capture::status(&app);
    let signed_out: Vec<Provider> = [
        (Provider::Anthropic, cap.claude_poll && cap.claude.last_error.as_deref() == Some("claude_no_plan_limits") && s.plans.get("anthropic").map(String::as_str) != Some("api")),
        (Provider::OpenAI, cap.codex_poll && cap.codex.last_error.as_deref() == Some("codex_not_signed_in")),
        (Provider::Google, cap.antigravity_poll && cap.antigravity.last_error.as_deref() == Some("agy_not_signed_in")),
    ]
    .into_iter()
    .filter(|&(p, out)| out && providers.contains(&p) && !live(p))
    .map(|(p, _)| p)
    .collect();
    let limits = limits
        .into_iter()
        .filter(|l| providers.contains(&l.provider) && (l.window == "five_hour" || l.window == "seven_day"))
        .map(|l| PanelLimit { provider: l.provider, limit_id: l.limit_id, window: l.window, used_pct: l.used_pct, state: l.state, resets_at: l.resets_at, forecast: l.forecast })
        .collect();
    let (last_scan_ms, scanning) = {
        let st = state.status.lock().unwrap();
        (st.last_scan_ms, st.running)
    };
    let mut order: Vec<Provider> = providers.into_iter().collect();
    order.sort_by_key(|p| p.as_str());
    Ok(PanelData {
        limits,
        usage: crate::tray::usage(&app, &s),
        last_scan_ms,
        scanning,
        widget_visible: s.widget.visible,
        update: state.updates.lock().unwrap().available.as_ref().map(|a| a.version.clone()),
        providers: order,
        signed_out,
    })
}

/// A module's button. Opening another window closes the panel; switches and rescans keep it.
#[tauri::command]
pub fn panel_action(app: AppHandle, window: WebviewWindow, action: String) -> Result<(), String> {
    if window.label() != PANEL {
        return Err("not_allowed".into());
    }
    match action.as_str() {
        "widget" | "rescan" => {
            handle_menu(&app, &format!("tray:{action}"));
            return Ok(());
        }
        "open" => show_main_at(&app, None),
        "settings" => show_main_at(&app, Some("settings")),
        "edit" => show_main_at(&app, Some("menubar")),
        "limits" => show_main_at(&app, Some("limits")),
        "update" => handle_menu(&app, "tray:update"),
        "close" => {}
        _ => return Err("unknown_action".into()),
    }
    hide(&window);
    Ok(())
}
