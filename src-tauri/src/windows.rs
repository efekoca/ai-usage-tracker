//! Main window (created on demand and destroyed on close to free WebView memory), the
//! always-on-top widget, its native context menu, the tray icon and full-screen auto-hide.

use crate::settings::Settings;
use crate::state::AppState;
use crate::worker::Msg;
use std::sync::atomic::Ordering;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const WIDGET: &str = "widget";

/// Windows 11 (build 22000+) supports Mica.
pub fn supports_mica() -> bool {
    #[cfg(windows)]
    {
        os_build() >= 22000
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn os_build() -> u32 {
    // RtlGetVersion is not subject to manifest-based version lies.
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    let mut v = OsVersionInfo { size: std::mem::size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
    unsafe { RtlGetVersion(&mut v) };
    v.build
}

/// The user's Windows accent color as `#rrggbb` (read-only registry lookup), if set.
pub fn accent_color() -> Option<String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let key: Vec<u16> = "Software\\Microsoft\\Windows\\DWM\0".encode_utf16().collect();
        let val: Vec<u16> = "AccentColor\0".encode_utf16().collect();
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let rc = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut data as *mut u32 as *mut _, &mut size)
        };
        // stored as 0xAABBGGRR
        (rc == 0).then(|| format!("#{:02x}{:02x}{:02x}", data & 0xff, (data >> 8) & 0xff, (data >> 16) & 0xff))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

// ------------------------------------------------------------------ main window

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let mut b = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("AI Usage Tracker")
        .inner_size(1180.0, 780.0)
        .min_inner_size(820.0, 560.0)
        .center()
        .visible(true);
    if supports_mica() {
        b = b.transparent(true).effects(EffectsBuilder::new().effect(Effect::Mica).build());
    }
    match b.build() {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => log::error!("cannot create main window: {e}"),
    }
}

// ------------------------------------------------------------------ widget

fn widget_size(size: &str) -> (f64, f64) {
    match size {
        "s" => (232.0, 104.0),
        "l" => (360.0, 156.0),
        _ => (288.0, 128.0),
    }
}

pub fn ensure_widget(app: &AppHandle, s: &Settings) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(WIDGET) {
        return Some(w);
    }
    let (w, h) = widget_size(&s.widget.size);
    let mut b = WebviewWindowBuilder::new(app, WIDGET, WebviewUrl::App("widget.html".into()))
        .title("AI Usage Tracker widget")
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false);
    if let (Some(x), Some(y)) = (s.widget.x, s.widget.y) {
        b = b.position(x as f64, y as f64);
    }
    match b.build() {
        Ok(win) => {
            if s.widget.x.is_none() {
                place_widget(&win, "top-right");
            } else if !on_any_monitor(&win, s.widget.x.unwrap_or(0), s.widget.y.unwrap_or(0)) {
                // the monitor it lived on is gone
                place_widget(&win, "top-right");
            }
            let handle = app.clone();
            win.on_window_event(move |e| {
                if let tauri::WindowEvent::Moved(p) = e {
                    remember_widget_position(&handle, p.x, p.y);
                }
            });
            Some(win)
        }
        Err(e) => {
            log::error!("cannot create widget: {e}");
            None
        }
    }
}

fn on_any_monitor(w: &WebviewWindow, x: i32, y: i32) -> bool {
    w.available_monitors().unwrap_or_default().iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        x >= p.x - 50 && y >= p.y - 50 && x < p.x + s.width as i32 && y < p.y + s.height as i32
    })
}

fn remember_widget_position(app: &AppHandle, x: i32, y: i32) {
    let state = app.state::<AppState>();
    let mut s = state.settings.write().unwrap();
    if s.widget.x == Some(x) && s.widget.y == Some(y) {
        return;
    }
    s.widget.x = Some(x);
    s.widget.y = Some(y);
    let snapshot = s.clone();
    drop(s);
    if let Ok(store) = state.store.try_lock() {
        let _ = snapshot.save(&store);
    }
}

/// Moves the widget to a corner of the monitor it is on (work area, so the taskbar is avoided).
pub fn place_widget(w: &WebviewWindow, corner: &str) {
    let Ok(Some(m)) = w.current_monitor().or_else(|_| w.primary_monitor()) else { return };
    let area = m.work_area();
    let Ok(size) = w.outer_size() else { return };
    let margin = (16.0 * m.scale_factor()) as i32;
    let (ax, ay, aw, ah) = (area.position.x, area.position.y, area.size.width as i32, area.size.height as i32);
    let (x, y) = match corner {
        "top-left" => (ax + margin, ay + margin),
        "bottom-left" => (ax + margin, ay + ah - size.height as i32 - margin),
        "bottom-right" => (ax + aw - size.width as i32 - margin, ay + ah - size.height as i32 - margin),
        _ => (ax + aw - size.width as i32 - margin, ay + margin),
    };
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

/// Applies visibility/size from settings and tells the widget to re-render.
pub fn apply_widget_settings(app: &AppHandle, s: &Settings) {
    if !s.onboarded {
        if let Some(w) = app.get_webview_window(WIDGET) {
            let _ = w.hide();
        }
        return;
    }
    if !s.widget.visible {
        if let Some(w) = app.get_webview_window(WIDGET) {
            let _ = w.hide();
        }
        return;
    }
    if let Some(w) = ensure_widget(app, s) {
        let (wd, ht) = widget_size(&s.widget.size);
        let _ = w.set_size(tauri::LogicalSize::new(wd, ht));
        if !FULLSCREEN_HIDDEN.load(Ordering::SeqCst) {
            let _ = w.show();
        }
        let _ = w.set_always_on_top(true);
    }
}

pub fn popup_widget_menu(app: &AppHandle) {
    let Some(w) = app.get_webview_window(WIDGET) else { return };
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let tr = s.language == "tr" || (s.language == "system" && system_is_turkish());
    let t = |en: &'static str, tr_: &'static str| if tr { tr_ } else { en };
    let build = || -> tauri::Result<Menu<tauri::Wry>> {
        let open = MenuItem::with_id(app, "w:open", t("Open dashboard", "Paneli aç"), true, None::<&str>)?;
        let op = |v: f64, label: &str| CheckMenuItem::with_id(app, format!("w:opacity:{v}"), label, true, (s.widget.opacity - v).abs() < 0.01, None::<&str>);
        let opacity = Submenu::with_items(app, t("Opacity", "Saydamlık"), true, &[&op(1.0, "100%")?, &op(0.85, "85%")?, &op(0.7, "70%")?, &op(0.55, "55%")?])?;
        let sz = |v: &str, label: &str| CheckMenuItem::with_id(app, format!("w:size:{v}"), label, true, s.widget.size == v, None::<&str>);
        let size = Submenu::with_items(app, t("Size", "Boyut"), true, &[&sz("s", t("Small", "Küçük"))?, &sz("m", t("Medium", "Orta"))?, &sz("l", t("Large", "Büyük"))?])?;
        let pos = |v: &str, label: &str| MenuItem::with_id(app, format!("w:pos:{v}"), label, true, None::<&str>);
        let position = Submenu::with_items(
            app,
            t("Position", "Konum"),
            true,
            &[&pos("top-left", t("Top left", "Sol üst"))?, &pos("top-right", t("Top right", "Sağ üst"))?, &pos("bottom-left", t("Bottom left", "Sol alt"))?, &pos("bottom-right", t("Bottom right", "Sağ alt"))?],
        )?;
        let autohide = CheckMenuItem::with_id(app, "w:autohide", t("Hide in full screen", "Tam ekranda gizle"), true, s.widget.auto_hide_fullscreen, None::<&str>)?;
        let hide = MenuItem::with_id(app, "w:hide", t("Hide widget", "Widget'ı gizle"), true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "app:quit", t("Quit", "Çık"), true, None::<&str>)?;
        Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?, &opacity, &size, &position, &autohide, &PredefinedMenuItem::separator(app)?, &hide, &quit])
    };
    match build() {
        Ok(menu) => {
            let _ = w.popup_menu(&menu);
        }
        Err(e) => log::warn!("widget menu: {e}"),
    }
}

pub fn system_is_turkish() -> bool {
    std::env::var("LANG").map(|l| l.starts_with("tr")).unwrap_or(false) || user_locale().starts_with("tr")
}

#[cfg(windows)]
fn user_locale() -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n > 1 { String::from_utf16_lossy(&buf[..(n - 1) as usize]) } else { String::new() }
}

#[cfg(not(windows))]
fn user_locale() -> String {
    String::new()
}

/// Handles ids from both the widget context menu and the tray menu.
pub fn handle_menu(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let mut s = state.settings.read().unwrap().clone();
    match id {
        "w:open" | "tray:open" => return show_main(app),
        "app:quit" => {
            state.quitting.store(true, Ordering::SeqCst);
            state.worker.send(Msg::Shutdown);
            app.exit(0);
            return;
        }
        "tray:rescan" => return state.worker.send(Msg::Scan),
        "w:hide" => s.widget.visible = false,
        "tray:widget" => s.widget.visible = !s.widget.visible,
        "w:autohide" => s.widget.auto_hide_fullscreen = !s.widget.auto_hide_fullscreen,
        other => {
            if let Some(v) = other.strip_prefix("w:opacity:").and_then(|v| v.parse::<f64>().ok()) {
                s.widget.opacity = v;
            } else if let Some(v) = other.strip_prefix("w:size:") {
                s.widget.size = v.to_owned();
            } else if let Some(corner) = other.strip_prefix("w:pos:") {
                if let Some(w) = app.get_webview_window(WIDGET) {
                    place_widget(&w, corner);
                }
                return;
            } else {
                return;
            }
        }
    }
    let _ = s.save(&state.store.lock().unwrap());
    *state.settings.write().unwrap() = s.clone();
    apply_widget_settings(app, &s);
    let _ = app.emit("settings-changed", &s);
}

// ------------------------------------------------------------------ tray

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let tr = s.language == "tr" || (s.language == "system" && system_is_turkish());
    let t = |en: &'static str, tr_: &'static str| if tr { tr_ } else { en };
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "tray:open", t("Open dashboard", "Paneli aç"), true, None::<&str>)?,
            &MenuItem::with_id(app, "tray:widget", t("Show/hide widget", "Widget'ı göster/gizle"), true, None::<&str>)?,
            &MenuItem::with_id(app, "tray:rescan", t("Rescan now", "Şimdi tara"), true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "app:quit", t("Quit", "Çık"), true, None::<&str>)?,
        ],
    )?;
    let mut b = TrayIconBuilder::with_id("main-tray").tooltip("AI Usage Tracker").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.on_menu_event(|app, e| handle_menu(app, e.id().as_ref()))
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

// ------------------------------------------------------------------ autostart

pub fn apply_autostart(app: &AppHandle, on: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let m = app.autolaunch();
    let r = if on { m.enable() } else { m.disable() };
    if let Err(e) = r {
        log::warn!("autostart change failed: {e}");
    }
}

// ------------------------------------------------------------------ full-screen auto-hide

static FULLSCREEN_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Polls the shell's "user notification state" every 2 s: when a full-screen app, game or
/// presentation is in front, the widget hides; it comes back afterwards.
pub fn start_fullscreen_watch(app: AppHandle) {
    std::thread::Builder::new()
        .name("fullscreen-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let state = app.state::<AppState>();
                if state.quitting.load(Ordering::SeqCst) {
                    return;
                }
                let s = state.settings.read().unwrap().clone();
                if !s.widget.visible || !s.onboarded {
                    continue;
                }
                let busy = s.widget.auto_hide_fullscreen && fullscreen_app_active();
                let was = FULLSCREEN_HIDDEN.swap(busy, Ordering::SeqCst);
                if busy != was
                    && let Some(w) = app.get_webview_window(WIDGET)
                {
                    let _ = if busy { w.hide() } else { w.show() };
                }
            }
        })
        .expect("spawn fullscreen watcher");
}

#[cfg(windows)]
fn fullscreen_app_active() -> bool {
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    let mut state = 0;
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    hr == 0 && (state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE)
}

#[cfg(not(windows))]
fn fullscreen_app_active() -> bool {
    false
}
