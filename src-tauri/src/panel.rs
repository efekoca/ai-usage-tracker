//! The panel of tiles the tray icon opens: placed against the icon, closed when it
//! loses focus.

use crate::state::AppState;
use crate::tray_text::Usage;
use crate::windows::{handle_menu, show_main_at, supports_mica, window_theme};
use serde::Serialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{AppHandle, LogicalPosition, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracker_core::analytics::{self, LimitState};
use tracker_core::insights::Forecast;
use tracker_core::model::Provider;

pub const PANEL: &str = "panel";
const WIDTH: f64 = 304.0;
/// Logical pixels between the icon and the panel, and the panel and the screen edge.
const GAP: f64 = 6.0;

/// `(x, y, w, h)`, from the top left.
type Area = (f64, f64, f64, f64);

/// The icon the panel opened from; `None` places it top right.
#[derive(Clone, Copy)]
enum Anchor {
    /// Physical pixels: Windows and Linux lay every screen on one pixel grid.
    Pixels(Area),
    /// macOS lays screens out in points, and a screen's pixels line up only within it: the icon, its
    /// screen and the screen's work area, in points from the main screen's top left.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Points { icon: Area, screen: Area, work: Area },
}

static ANCHOR: Mutex<Option<Anchor>> = Mutex::new(None);
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
    #[cfg(target_os = "macos")]
    let points = icon_points(app);
    #[cfg(not(target_os = "macos"))]
    let points = None;
    *ANCHOR.lock().unwrap() = points.or_else(|| {
        rect.map(|r| {
            let p = r.position.to_physical::<f64>(1.0);
            let s = r.size.to_physical::<f64>(1.0);
            Anchor::Pixels((p.x, p.y, s.width, s.height))
        })
    });
    let Some(w) = ensure(app) else { return };
    place(&w);
    let _ = w.show();
    let _ = w.set_focus();
}

/// The menu bar icon's window and its screen, as AppKit lays them out.
#[cfg(target_os = "macos")]
fn icon_points(app: &AppHandle) -> Option<Anchor> {
    let tray = app.tray_by_id(crate::tray::ID)?;
    tray.with_inner_tray_icon(|t| {
        use objc2_app_kit::NSScreen;
        use objc2_foundation::NSRect;
        let mtm = objc2::MainThreadMarker::new()?;
        let window = t.ns_status_item()?.button(mtm)?.window()?;
        let screen = window.screen()?;
        // AppKit counts up from the main screen's bottom edge
        let main = NSScreen::screens(mtm).firstObject()?.frame();
        let flip = |r: NSRect| (r.origin.x, main.size.height - r.origin.y - r.size.height, r.size.width, r.size.height);
        Some(Anchor::Points { icon: flip(window.frame()), screen: flip(screen.frame()), work: flip(screen.visibleFrame()) })
    })
    .ok()
    .flatten()
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
        .on_navigation(crate::windows::app_page)
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
    let anchor = *ANCHOR.lock().unwrap();
    if let Some(Anchor::Points { icon, screen, work }) = anchor {
        let scale = w.scale_factor().unwrap_or(1.0);
        let (x, y) = spot((size.width as f64 / scale, size.height as f64 / scale), Some(icon), screen, work, GAP);
        let _ = w.set_position(LogicalPosition::new(x, y));
        return;
    }
    let icon = match anchor {
        Some(Anchor::Pixels(r)) => Some(r),
        _ => None,
    };
    let monitors = w.available_monitors().unwrap_or_default();
    let monitor = icon
        .and_then(|(x, y, aw, ah)| {
            let (cx, cy) = (x + aw / 2.0, y + ah / 2.0);
            monitors.iter().find(|m| {
                let (p, s) = (m.position(), m.size());
                cx >= p.x as f64 && cx < p.x as f64 + s.width as f64 && cy >= p.y as f64 && cy < p.y as f64 + s.height as f64
            })
        })
        .cloned()
        .or_else(|| w.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let (p, s, a) = (m.position(), m.size(), m.work_area());
    let screen = (p.x as f64, p.y as f64, s.width as f64, s.height as f64);
    let work = (a.position.x as f64, a.position.y as f64, a.size.width as f64, a.size.height as f64);
    let (x, y) = spot((size.width as f64, size.height as f64), icon, screen, work, (GAP * m.scale_factor()).round());
    let _ = w.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// The panel's top left: centred below an icon in the top half of its screen (macOS menu bar),
/// above one lower down, or top right without one; always `gap` inside the work area.
fn spot((pw, ph): (f64, f64), icon: Option<Area>, (_, sy, _, sh): Area, (left, top, ww, wh): Area, gap: f64) -> (f64, f64) {
    let (right, bottom) = (left + ww, top + wh);
    let (x, y) = match icon {
        Some((ax, ay, aw, ah)) => {
            let below = ay + ah / 2.0 < sy + sh / 2.0;
            (ax + aw / 2.0 - pw / 2.0, if below { ay + ah + gap } else { ay - ph - gap })
        }
        None => (right - pw - gap, top + gap),
    };
    (x.clamp(left + gap, (right - pw - gap).max(left + gap)), y.clamp(top + gap, (bottom - ph - gap).max(top + gap)))
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
    /// The Codex or Claude account, while there are several.
    account: Option<String>,
    active: bool,
}

#[derive(Serialize)]
pub struct PanelData {
    limits: Vec<PanelLimit>,
    /// Today, the last 7 and the last 30 days.
    usage: Option<[Usage; 3]>,
    /// The same periods for OpenCode alone; `None` while its source is off.
    opencode: Option<[Usage; 3]>,
    last_scan_ms: Option<i64>,
    scanning: bool,
    widget_visible: bool,
    update: Option<String>,
    providers: Vec<Provider>,
    /// Signed out of the tool the limits are read through, with no current reading left.
    signed_out: Vec<Provider>,
    /// Known accounts in order of first use, which numbers the unnamed ones.
    accounts: Vec<crate::commands::KnownAccount>,
}

/// Everything the panel's modules show; the Menu bar page's preview reads it too.
#[tauri::command]
pub async fn panel_data(app: AppHandle) -> Result<PanelData, String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().unwrap().clone();
    let providers = crate::commands::enabled_providers(&s);
    let (limits, accounts) = {
        let store = state.db();
        let book = state.book.read().unwrap();
        // another account's limits show only in modules the user added for it
        let limits = analytics::limits_view(&store, &book, now_ms(), &s.thresholds).map_err(|e| e.to_string())?.into_iter().filter(|l| l.active || l.account.is_some()).collect::<Vec<_>>();
        (limits, crate::commands::known_accounts(&store)?)
    };
    let live = |p: Provider| limits.iter().any(|l| l.active && l.provider == p && l.used_pct.is_some() && matches!(l.state, LimitState::Fresh | LimitState::Behind));
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
        .map(|l| PanelLimit {
            provider: l.provider,
            limit_id: l.limit_id,
            window: l.window,
            used_pct: l.used_pct,
            state: l.state,
            resets_at: l.resets_at,
            forecast: l.forecast,
            account: l.account,
            active: l.active,
        })
        .collect();
    let (last_scan_ms, scanning) = {
        let st = state.status.lock().unwrap();
        (st.last_scan_ms, st.running)
    };
    let mut order: Vec<Provider> = providers.into_iter().collect();
    order.sort_by_key(|p| p.as_str());
    let (usage, opencode) = crate::tray::usage(&app, &s).map_or((None, None), |(u, o)| (Some(u), o));
    Ok(PanelData {
        limits,
        usage,
        opencode,
        last_scan_ms,
        scanning,
        widget_visible: s.widget.visible,
        update: state.updates.lock().unwrap().available.as_ref().map(|a| a.version.clone()),
        providers: order,
        signed_out,
        accounts,
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
        "limits" => show_main_at(&app, Some("limits")),
        "update" => handle_menu(&app, "tray:update"),
        "close" => {}
        _ => return Err("unknown_action".into()),
    }
    hide(&window);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_opens_against_the_icon_inside_its_screen() {
        let panel = (304.0, 420.0);
        // a screen above the main one, in points: macOS gives it negative coordinates
        let upper = (-204.0, -1080.0, 1920.0, 1080.0);
        let upper_work = (-204.0, -1056.0, 1920.0, 1056.0);
        assert_eq!(spot(panel, Some((1000.0, -1080.0, 30.0, 24.0)), upper, upper_work, 6.0), (863.0, -1050.0));
        assert_eq!(spot(panel, Some((1690.0, -1080.0, 20.0, 24.0)), upper, upper_work, 6.0), (1406.0, -1050.0), "kept inside near the edge");
        // a taskbar at the bottom: above the icon, clear of the taskbar
        let screen = (0.0, 0.0, 1920.0, 1080.0);
        let work = (0.0, 0.0, 1920.0, 1040.0);
        assert_eq!(spot(panel, Some((1700.0, 1048.0, 24.0, 24.0)), screen, work, 6.0), (1560.0, 614.0));
        assert_eq!(spot(panel, None, screen, work, 6.0), (1610.0, 6.0), "top right without an icon");
    }
}
