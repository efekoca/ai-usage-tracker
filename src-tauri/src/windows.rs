//! The main window is created on demand and destroyed on close to free WebView memory.

use crate::settings::Settings;
use crate::state::AppState;
use crate::worker::Msg;
use std::sync::atomic::Ordering;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const WIDGET: &str = "widget";

/// Build 22000 is Windows 11, the first with Mica.
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

/// Whether the taskbar has Windows' light look; the apps' own light or dark mode is set apart.
#[cfg(windows)]
pub fn light_taskbar() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0".encode_utf16().collect();
    let val: Vec<u16> = "SystemUsesLightTheme\0".encode_utf16().collect();
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut data as *mut u32 as *mut _, &mut size)
    };
    // missing on older Windows 10, whose taskbar is dark
    rc == 0 && data == 1
}

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
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSColor, NSColorSpace};
        let c = NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(format!("#{:02x}{:02x}{:02x}", byte(c.redComponent()), byte(c.greenComponent()), byte(c.blueComponent())))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Creating a WebView window waits for the event loop, which deadlocks on Windows when called
/// from it (sync IPC commands, menu and tray handlers), so it always runs on a helper thread.
fn off_main(f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name("window-builder".into()).spawn(f) {
        log::error!("cannot spawn window builder: {e}");
    }
}

pub fn show_main(app: &AppHandle) {
    show_main_at(app, None);
}

/// Shows the main window, on `view` when given.
pub fn show_main_at(app: &AppHandle, view: Option<&str>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(v) = view {
            let _ = app.emit_to(MAIN, "navigate", v);
        }
        return;
    }
    let (app, view) = (app.clone(), view.map(str::to_owned));
    off_main(move || build_main(&app, view.as_deref()));
}

/// Windows show only the app's own pages: anything else, such as Tauri's IPC address, is refused
/// and logged, so a window never ends up on a bare error page.
pub(crate) fn app_page(url: &tauri::Url) -> bool {
    let web = matches!(url.scheme(), "http" | "https");
    // a new webview may pass through a blank page before its first one
    let ours = url.scheme() == "tauri"
        || url.as_str() == "about:blank"
        || (web && url.host_str() == Some("tauri.localhost"))
        || (tauri::is_dev() && web && url.host_str() == Some("localhost"));
    if !ours {
        log::warn!("refused to open {}://{}{}", url.scheme(), url.host_str().unwrap_or_default(), url.path());
    }
    ours
}

fn build_main(app: &AppHandle, view: Option<&str>) {
    if app.get_webview_window(MAIN).is_some() {
        return; // a concurrent request already built it
    }
    let (w, h, min_w, min_h) = match app.primary_monitor() {
        Ok(Some(m)) => {
            let area = m.work_area().size;
            main_size(area.width as f64 / m.scale_factor(), area.height as f64 / m.scale_factor())
        }
        _ => (1360.0, 880.0, 860.0, 560.0),
    };
    // a new window reads its first page from the address; an event would arrive before it listens
    let url = view.map_or_else(|| "index.html".to_owned(), |v| format!("index.html#{v}"));
    let mut b = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App(url.into()))
        .title("AI Usage Tracker")
        .inner_size(w, h)
        .min_inner_size(min_w, min_h)
        .center()
        // macOS otherwise spends the first click on an inactive window just activating it
        .accept_first_mouse(true)
        .theme(window_theme(&app.state::<AppState>().settings.read().unwrap()))
        .on_navigation(app_page)
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

/// Mica and the title bar follow the window theme, not the page's.
pub(crate) fn window_theme(s: &Settings) -> Option<tauri::Theme> {
    match s.theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    }
}

/// The app's icon in the Dock and ⌘Tab; its windows stay open either way.
#[cfg(target_os = "macos")]
pub fn apply_dock(app: &AppHandle, s: &Settings) {
    if let Err(e) = app.set_dock_visibility(s.show_in_dock) {
        log::warn!("dock: {e}");
    }
}

pub fn apply_theme(app: &AppHandle, s: &Settings) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.set_theme(window_theme(s));
    }
    crate::panel::apply_theme(app, s);
}

/// Fits the work area minus the frame, or the page bottom is out of reach on small screens.
fn main_size(aw: f64, ah: f64) -> (f64, f64, f64, f64) {
    let (aw, ah) = ((aw - 16.0).max(320.0), (ah - 48.0).max(240.0));
    ((aw * 0.9).min(1360.0).max(aw.min(900.0)), (ah * 0.9).min(880.0).max(ah.min(600.0)), aw.min(860.0), ah.min(560.0))
}

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
    let b = WebviewWindowBuilder::new(app, WIDGET, WebviewUrl::App("widget.html".into()))
        .title("AI Usage Tracker widget")
        .on_navigation(app_page)
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .accept_first_mouse(true)
        .visible(false);
    match b.build() {
        Ok(win) => {
            // saved positions are physical (from Moved events), so they are restored as physical
            match (s.widget.anchor.as_str(), s.widget.x, s.widget.y) {
                ("", Some(x), Some(y)) if fits_a_monitor(&win, x, y) => {
                    LAST_PLACED_MS.store(chrono::Utc::now().timestamp_millis(), Ordering::SeqCst);
                    let _ = win.set_position(PhysicalPosition::new(x, y));
                }
                ("", _, _) => place_widget(&win, "bottom-right"), // never placed, or its monitor is gone
                (corner, _, _) => place_widget(&win, corner),
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

fn fits_a_monitor(w: &WebviewWindow, x: i32, y: i32) -> bool {
    let Ok(size) = w.outer_size() else { return false };
    let rects: Vec<(i32, i32, i32, i32)> = w
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.size().height as i32))
        .collect();
    rect_fits((x, y, size.width as i32, size.height as i32), &rects)
}

fn rect_fits((x, y, w, h): (i32, i32, i32, i32), monitors: &[(i32, i32, i32, i32)]) -> bool {
    const SLACK: i32 = 24;
    monitors.iter().any(|&(mx, my, mw, mh)| x >= mx - SLACK && y >= my - SLACK && x + w <= mx + mw + SLACK && y + h <= my + mh + SLACK)
}

/// When the app itself last moved the widget; Moved events right after it are ours, not a drag.
static LAST_PLACED_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static LAST_DRAG_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static DRAG_SAVE_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The last dragged position, applied again when it is saved: a settings update that copied the
/// settings before the drag would otherwise put the old position back. Taken and cleared only under
/// the settings lock, so a corner picked after the drag cannot be overwritten by its delayed save.
static DRAGGED_TO: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

fn apply_pending_drag(s: &mut Settings) {
    if let Some((x, y)) = DRAGGED_TO.lock().unwrap().take() {
        s.widget.x = Some(x);
        s.widget.y = Some(y);
        s.widget.anchor.clear();
    }
}

fn remember_widget_position(app: &AppHandle, x: i32, y: i32) {
    let now = chrono::Utc::now().timestamp_millis();
    if now - LAST_PLACED_MS.load(Ordering::SeqCst) < 600 {
        return;
    }
    let state = app.state::<AppState>();
    let mut s = state.settings.write().unwrap();
    if s.widget.x == Some(x) && s.widget.y == Some(y) {
        return;
    }
    // a user drag: drop the corner anchor
    s.widget.x = Some(x);
    s.widget.y = Some(y);
    s.widget.anchor.clear();
    drop(s);
    *DRAGGED_TO.lock().unwrap() = Some((x, y));
    LAST_DRAG_MS.store(now, Ordering::SeqCst);
    // a drag fires many Moved events: save and tell the UI once it has settled, off the event loop
    if DRAG_SAVE_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("widget-position".into()).spawn(move || {
        while chrono::Utc::now().timestamp_millis() - LAST_DRAG_MS.load(Ordering::SeqCst) < 400 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        DRAG_SAVE_PENDING.store(false, Ordering::SeqCst);
        match app.state::<AppState>().update_settings(apply_pending_drag) {
            Ok(s) => {
                let _ = app.emit("settings-changed", &s);
            }
            Err(e) => log::warn!("widget position not saved: {e}"),
        }
    });
    if spawned.is_err() {
        DRAG_SAVE_PENDING.store(false, Ordering::SeqCst);
    }
}

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
    LAST_PLACED_MS.store(chrono::Utc::now().timestamp_millis(), Ordering::SeqCst);
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

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
    // the widget page sizes its own window to fit its content
    let show = |w: &WebviewWindow, s: &Settings| {
        if !FULLSCREEN_HIDDEN.load(Ordering::SeqCst) {
            let _ = w.show();
        }
        let _ = w.set_always_on_top(s.widget.always_on_top);
        #[cfg(target_os = "macos")]
        join_all_spaces(w, !s.show_in_dock);
    };
    match app.get_webview_window(WIDGET) {
        Some(w) => show(&w, s),
        None => {
            let (app, s) = (app.clone(), s.clone());
            off_main(move || {
                if let Some(w) = ensure_widget(&app, &s) {
                    show(&w, &s);
                }
            });
        }
    }
}

/// macOS keeps a window on the Space it opened on. A full-screen app's Space takes only the helper
/// windows of an app without a Dock icon, so the widget can show over full-screen apps only while
/// `show_in_dock` is off; the full-screen watcher hides it there when asked to.
#[cfg(target_os = "macos")]
fn join_all_spaces(w: &WebviewWindow, over_full_screen: bool) {
    let Ok(ptr) = w.ns_window() else { return };
    let ptr = ptr as usize;
    let _ = w.run_on_main_thread(move || {
        use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
        // SAFETY: the widget's own NSWindow, used on the main thread.
        let win: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        let aux = NSWindowCollectionBehavior::FullScreenAuxiliary.0;
        let rest = (win.collectionBehavior().0 | NSWindowCollectionBehavior::CanJoinAllSpaces.0) & !aux;
        win.setCollectionBehavior(NSWindowCollectionBehavior(if over_full_screen { rest | aux } else { rest }));
    });
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
        let sep = || PredefinedMenuItem::separator(app);
        // Windows can tell a full-screen app is in front; macOS lets the widget show there only
        // without a Dock icon (see `join_all_spaces`)
        if cfg!(windows) || (cfg!(target_os = "macos") && !s.show_in_dock) {
            Menu::with_items(app, &[&open, &sep()?, &opacity, &size, &position, &autohide, &sep()?, &hide, &quit])
        } else {
            Menu::with_items(app, &[&open, &sep()?, &opacity, &size, &position, &sep()?, &hide, &quit])
        }
    };
    match build() {
        Ok(menu) => {
            let _ = w.popup_menu(&menu);
        }
        Err(e) => log::warn!("widget menu: {e}"),
    }
}

pub fn system_is_turkish() -> bool {
    #[cfg(not(any(windows, target_os = "macos")))]
    return user_locale().starts_with("tr");
    #[cfg(any(windows, target_os = "macos"))]
    return std::env::var("LANG").map(|l| l.starts_with("tr")).unwrap_or(false) || user_locale().starts_with("tr");
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

/// Apps opened from Finder or at login get no `LANG`, so the system's language list decides.
#[cfg(target_os = "macos")]
fn user_locale() -> String {
    objc2_foundation::NSLocale::preferredLanguages().firstObject().map(|l| l.to_string()).unwrap_or_default()
}

/// The message language the way gettext picks it: LANGUAGE (unless the locale is C), LC_ALL,
/// LC_MESSAGES, then LANG.
#[cfg(not(any(windows, target_os = "macos")))]
fn user_locale() -> String {
    locale_from(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

#[cfg(any(not(any(windows, target_os = "macos")), test))]
fn locale_from(var: impl Fn(&str) -> Option<String>) -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(&var).unwrap_or_default();
    let plain = locale.is_empty() || locale == "C" || locale == "POSIX" || locale.starts_with("C.");
    match var("LANGUAGE").and_then(|l| l.split(':').find(|x| !x.is_empty()).map(str::to_owned)) {
        Some(first) if !plain => first,
        _ => locale,
    }
}

pub fn handle_menu(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    match id {
        "w:open" | "tray:open" => return show_main(app),
        "tray:panel" => return crate::panel::toggle(app, None),
        "app:quit" => {
            state.quitting.store(true, Ordering::SeqCst);
            state.worker.send(Msg::Shutdown);
            app.exit(0);
            return;
        }
        "tray:rescan" => return state.worker.send(Msg::Scan),
        "tray:update" => return show_main_at(app, Some("settings")),
        _ => {}
    }
    if menu_change(id).is_none() {
        return;
    }
    // a long query may hold the store; waiting for it here would freeze the event loop
    let (app, id) = (app.clone(), id.to_owned());
    off_main(move || {
        let Some(change) = menu_change(&id) else { return };
        // applied to the current settings under the store lock, so a concurrent change is kept
        match app.state::<AppState>().update_settings(change) {
            Ok(s) => {
                if let Some(corner) = id.strip_prefix("w:pos:")
                    && let Some(w) = app.get_webview_window(WIDGET)
                {
                    place_widget(&w, corner);
                }
                // a quick second press may have saved after this one: show what is saved now
                let latest = app.state::<AppState>().settings.read().unwrap().clone();
                apply_widget_settings(&app, &latest);
                let _ = app.emit("settings-changed", &s);
                crate::tray::refresh_soon();
            }
            Err(e) => log::warn!("menu setting not saved: {e}"),
        }
    });
}

type SettingsChange<'a> = Box<dyn FnOnce(&mut Settings) + 'a>;

/// The settings change a widget or tray menu item makes, if it makes one.
fn menu_change(id: &str) -> Option<SettingsChange<'_>> {
    Some(match id {
        "w:hide" => Box::new(|s| s.widget.visible = false),
        "tray:widget" => Box::new(|s| s.widget.visible = !s.widget.visible),
        "w:autohide" => Box::new(|s| s.widget.auto_hide_fullscreen = !s.widget.auto_hide_fullscreen),
        other => {
            if let Some(v) = other.strip_prefix("w:opacity:").and_then(|v| v.parse::<f64>().ok()) {
                Box::new(move |s| s.widget.opacity = v)
            } else if let Some(v) = other.strip_prefix("w:size:") {
                Box::new(move |s| {
                    s.widget.size = v.to_owned();
                    s.widget.scale = match v {
                        "s" => 0.85,
                        "l" => 1.25,
                        _ => 1.0,
                    };
                })
            } else {
                let corner = other.strip_prefix("w:pos:")?;
                Box::new(move |s| {
                    DRAGGED_TO.lock().unwrap().take();
                    s.widget.anchor = corner.to_owned();
                })
            }
        }
    })
}

/// The uninstaller removes the same value.
#[cfg(windows)]
const AUTOSTART_NAME: &str = "AI Usage Tracker";

/// Quoted, so a folder name with spaces cannot be read as another program.
/// Logs and returns a failure, so a switch the system refused does not stay on.
pub fn apply_autostart(on: bool, user_choice: bool) -> Result<(), String> {
    #[cfg(windows)]
    let r = if on {
        std::env::current_exe().map_err(|e| e.to_string()).and_then(|exe| autostart::enable(AUTOSTART_NAME, &autostart_command(&exe), user_choice))
    } else {
        autostart::disable(AUTOSTART_NAME)
    };
    #[cfg(target_os = "macos")]
    let r = {
        let _ = user_choice;
        launch_agent::apply(on)
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let r = {
        let _ = user_choice;
        xdg_autostart::apply(on)
    };
    if let Err(e) = &r {
        log::warn!("autostart change failed: {e}");
    }
    r
}

#[cfg(any(windows, test))]
fn autostart_command(exe: &std::path::Path) -> String {
    format!("\"{}\" --autostart", exe.display())
}

/// An XDG autostart entry, which GNOME, KDE, XFCE and most other desktops start at login.
#[cfg(target_os = "linux")]
pub(crate) use xdg_autostart::exec_arg as desktop_exec_arg;

#[cfg(any(not(any(windows, target_os = "macos")), test))]
mod xdg_autostart {
    use std::path::Path;

    const FILE: &str = "ai-usage-tracker.desktop";

    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn apply(on: bool) -> Result<(), String> {
        use std::path::PathBuf;
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or("no config folder")?;
        apply_in(&config, &crate::app_path()?, on)
    }

    pub fn apply_in(config: &Path, exe: &Path, on: bool) -> Result<(), String> {
        let dir = config.join("autostart");
        let file = dir.join(FILE);
        if !on {
            return match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&file, entry(exe)).map_err(|e| e.to_string())
    }

    pub fn entry(exe: &Path) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=AI Usage Tracker\nExec={} --autostart\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exec_arg(&exe.to_string_lossy())
        )
    }

    /// Quoted per the Desktop Entry spec; the string-level escape doubles backslashes once more.
    pub(crate) fn exec_arg(s: &str) -> String {
        let quoted = s.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$");
        format!("\"{}\"", quoted.replace('\\', "\\\\")).replace('%', "%%")
    }
}

/// A per-user LaunchAgent; launchd starts it at the next login, so nothing is started now.
#[cfg(any(target_os = "macos", test))]
mod launch_agent {
    use std::path::Path;

    const LABEL: &str = "io.aiusagetracker.desktop";

    #[cfg(target_os = "macos")]
    pub fn apply(on: bool) -> Result<(), String> {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).ok_or("no home folder")?;
        apply_in(&home, &crate::app_path()?, on)
    }

    pub fn apply_in(home: &Path, exe: &Path, on: bool) -> Result<(), String> {
        let dir = home.join("Library").join("LaunchAgents");
        let file = dir.join(format!("{LABEL}.plist"));
        if !on {
            return match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&file, plist(exe)).map_err(|e| e.to_string())
    }

    pub fn plist(exe: &Path) -> String {
        let exe = xml_escape(&exe.to_string_lossy());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--autostart</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#
        )
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    }
}

#[cfg(windows)]
mod autostart {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn delete(root: HKEY, key: &str, name: &str) -> u32 {
        // SAFETY: NUL-terminated UTF-16 strings that outlive the call.
        let rc = unsafe { RegDeleteKeyValueW(root, wide(key).as_ptr(), wide(name).as_ptr()) };
        if rc == ERROR_FILE_NOT_FOUND { 0 } else { rc }
    }

    pub fn enable(name: &str, command: &str, user_choice: bool) -> Result<(), String> {
        let data = wide(command);
        // SAFETY: as above; the size is the byte length including the terminating NUL.
        let rc = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide(name).as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
        if rc != 0 {
            return Err(format!("registry error {rc}"));
        }
        if user_choice {
            delete(HKEY_CURRENT_USER, APPROVED, name);
        }
        // older versions could write a machine-wide entry when run elevated; it fails quietly otherwise
        delete(HKEY_LOCAL_MACHINE, RUN, name);
        Ok(())
    }

    pub fn disable(name: &str) -> Result<(), String> {
        delete(HKEY_LOCAL_MACHINE, RUN, name);
        delete(HKEY_CURRENT_USER, APPROVED, name);
        match delete(HKEY_CURRENT_USER, RUN, name) {
            0 => Ok(()),
            rc => Err(format!("registry error {rc}")),
        }
    }
}

static FULLSCREEN_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
                // with a Dock icon macOS keeps the widget off full-screen Spaces by itself
                let watch = s.widget.auto_hide_fullscreen && !(cfg!(target_os = "macos") && s.show_in_dock);
                let busy = watch && fullscreen_app_active(&app);
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

/// `QUNS_BUSY` is not trusted: always-on-top full-screen overlays (e.g. GPU overlays) report it
/// permanently, so the foreground window's rect is checked instead.
#[cfg(windows)]
fn fullscreen_app_active(_: &AppHandle) -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible};

    let mut state = 0;
    if unsafe { SHQueryUserNotificationState(&mut state) } == 0
        && (state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE)
    {
        return true;
    }
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || IsWindowVisible(hwnd) == 0 {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        // the desktop and the taskbar are "full screen" too, but never count
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// The front app is full screen on the widget's display when one of its windows covers that display.
/// Full-screen windows start under the menu bar strip (below a notch) and reach the bottom edge; a
/// zoomed window stops short of the Dock, or of the edge strip a hidden Dock keeps.
#[cfg(target_os = "macos")]
fn fullscreen_app_active(app: &AppHandle) -> bool {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::display::{CGDisplay, CGPoint, CGRect};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
    };
    // kCGMainMenuWindowLevel
    const MENU_BAR_LAYER: i64 = 24;

    let Some(w) = app.get_webview_window(WIDGET) else { return false };
    let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.outer_size(), w.scale_factor()) else {
        return false;
    };
    let center = CGPoint::new(
        (pos.x as f64 + size.width as f64 / 2.0) / scale,
        (pos.y as f64 + size.height as f64 / 2.0) / scale,
    );
    let Ok((ids, n)) = CGDisplay::displays_with_point(center, 1) else { return false };
    let Some(&id) = ids.first().filter(|_| n > 0) else { return false };
    let display = CGDisplay::new(id).bounds();
    let Some(list) = copy_window_info(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID) else {
        return false;
    };

    // SAFETY: the keys are CoreGraphics constants, and the list holds one dictionary per window.
    let key = |k: CFStringRef| unsafe { CFString::wrap_under_get_rule(k) };
    let (layer_key, pid_key, bounds_key) = unsafe { (key(kCGWindowLayer), key(kCGWindowOwnerPID), key(kCGWindowBounds)) };
    let me = i64::from(std::process::id());
    let (mut front, mut menu_bar, mut windows) = (None, 0.0, Vec::new());
    for item in list.iter() {
        let info: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_get_rule(*item as CFDictionaryRef) };
        let num = |k: &CFString| info.find(k).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64());
        let Some(rect) = info
            .find(&bounds_key)
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|b| CGRect::from_dict_representation(&b))
        else {
            continue;
        };
        match (num(&layer_key), num(&pid_key)) {
            (Some(0), Some(pid)) if pid != me => {
                // the list runs front to back, so the first app window belongs to the front app
                front.get_or_insert(pid);
                windows.push((pid, rect));
            }
            (Some(MENU_BAR_LAYER), _) if rect.origin.x == display.origin.x && rect.origin.y == display.origin.y => {
                menu_bar = rect.size.height;
            }
            _ => {}
        }
    }
    windows.iter().any(|(pid, r)| Some(*pid) == front && covers_display(r, &display, menu_bar))
}

#[cfg(target_os = "macos")]
fn covers_display(w: &core_graphics::display::CGRect, d: &core_graphics::display::CGRect, menu_bar: f64) -> bool {
    w.origin.x <= d.origin.x
        && w.origin.x + w.size.width >= d.origin.x + d.size.width
        && w.origin.y <= d.origin.y + menu_bar
        && w.origin.y + w.size.height >= d.origin.y + d.size.height
}

#[cfg(not(any(windows, target_os = "macos")))]
fn fullscreen_app_active(_: &AppHandle) -> bool {
    false
}

#[cfg(test)]
mod tests {

    #[test]
    fn windows_open_only_the_apps_own_pages() {
        let page = |u: &str| super::app_page(&u.parse().unwrap());
        assert!(page("tauri://localhost/index.html#limits") && page("tauri://localhost") && page("http://tauri.localhost/widget.html"));
        assert!(!page("ipc://localhost/plugin%3Aevent%7Cemit") && !page("ipc://tauri.localhost/x"), "Tauri's IPC address");
        assert!(!page("https://example.com/") && !page("file:///etc/hosts") && page("about:blank"));
        assert_eq!(page("http://localhost:5173/"), tauri::is_dev(), "the dev server only while developing");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_a_window_over_the_whole_display_counts_as_full_screen() {
        use core_graphics::display::{CGPoint, CGRect, CGSize};
        let r = |x, y, w, h| CGRect::new(&CGPoint::new(x, y), &CGSize::new(w, h));
        let (laptop, monitor) = (r(0.0, 0.0, 1512.0, 982.0), r(1512.0, -200.0, 2560.0, 1440.0));
        assert!(super::covers_display(&r(0.0, 33.0, 1512.0, 949.0), &laptop, 33.0), "below the notch");
        assert!(super::covers_display(&r(1512.0, -200.0, 2560.0, 1440.0), &monitor, 25.0));
        assert!(!super::covers_display(&r(0.0, 33.0, 1512.0, 867.0), &laptop, 33.0), "zoomed above the Dock");
        assert!(!super::covers_display(&r(0.0, 33.0, 1512.0, 945.0), &laptop, 33.0), "zoomed above a hidden Dock");
        assert!(!super::covers_display(&r(1516.0, -175.0, 2556.0, 1415.0), &monitor, 25.0), "zoomed beside a hidden Dock");
        assert!(!super::covers_display(&r(0.0, 33.0, 1512.0, 949.0), &laptop, 0.0), "no menu bar seen");
    }

    #[test]
    fn menu_items_change_only_their_own_setting() {
        let base = Settings::default();
        let after = |id: &str| {
            let mut s = base.clone();
            menu_change(id).expect(id)(&mut s);
            s
        };
        let s = after("w:size:l");
        assert_eq!((s.widget.size.as_str(), s.widget.scale), ("l", 1.25));
        assert_eq!(after("w:opacity:0.6").widget.opacity, 0.6);
        assert_eq!(after("tray:widget").widget.visible, !base.widget.visible);
        assert_eq!(serde_json::to_value(&after("w:autohide").capture).unwrap(), serde_json::to_value(&base.capture).unwrap());
        assert!(menu_change("tray:open").is_none() && menu_change("w:opacity:x").is_none());
    }

    #[test]
    fn a_corner_picked_after_a_drag_survives_the_delayed_drag_save() {
        let mut s = Settings::default();
        *DRAGGED_TO.lock().unwrap() = Some((40, 50));
        menu_change("w:pos:top-left").unwrap()(&mut s);
        apply_pending_drag(&mut s);
        assert_eq!((s.widget.anchor.as_str(), s.widget.x), ("top-left", None));

        // a drag after the corner still wins
        *DRAGGED_TO.lock().unwrap() = Some((40, 50));
        apply_pending_drag(&mut s);
        assert_eq!((s.widget.anchor.as_str(), s.widget.x, s.widget.y), ("", Some(40), Some(50)));
    }
    use super::*;

    #[test]
    fn the_language_follows_gettext_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_owned());
        assert_eq!(locale_from(env(&[("LANG", "tr_TR.UTF-8")])), "tr_TR.UTF-8");
        assert_eq!(locale_from(env(&[("LANG", "tr_TR.UTF-8"), ("LC_MESSAGES", "en_US.UTF-8")])), "en_US.UTF-8");
        assert_eq!(locale_from(env(&[("LANG", "en_US.UTF-8"), ("LC_ALL", "tr_TR.UTF-8"), ("LC_MESSAGES", "en_US.UTF-8")])), "tr_TR.UTF-8");
        assert_eq!(locale_from(env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "tr:en")])), "tr");
        // gettext ignores LANGUAGE under the C locale
        assert_eq!(locale_from(env(&[("LANG", "C"), ("LANGUAGE", "tr")])), "C");
    }

    #[test]
    fn the_main_window_fits_small_scaled_screens() {
        // 1366×768 at 150 %, taskbar taken off
        let (w, h, min_w, min_h) = main_size(910.0, 480.0);
        assert!(w <= 894.0 && min_w <= w && h <= 432.0 && min_h <= h);
        assert_eq!(main_size(1920.0, 1040.0), (1360.0, 880.0, 860.0, 560.0));
    }

    #[test]
    fn a_widget_must_lie_on_one_monitor() {
        let mons = [(0, 0, 1920, 1080), (-2560, -200, 2560, 1440)];
        assert!(rect_fits((1600, 900, 300, 150), &mons));
        assert!(rect_fits((-300, 100, 280, 120), &mons));
        assert!(!rect_fits((1800, 900, 300, 150), &mons), "half off the right edge");
        assert!(!rect_fits((4000, 100, 300, 150), &mons), "the monitor is gone");
    }

    #[test]
    fn the_autostart_path_is_quoted() {
        let cmd = autostart_command(std::path::Path::new(r"C:\Program Files\AI Usage Tracker\ai-usage-tracker.exe"));
        assert_eq!(cmd, r#""C:\Program Files\AI Usage Tracker\ai-usage-tracker.exe" --autostart"#);
    }

    #[test]
    fn the_autostart_entry_quotes_the_path_per_the_desktop_entry_spec() {
        let e = xdg_autostart::entry(std::path::Path::new(r"/opt/AI Usage/a\b$x`y%z/ai-usage-tracker"));
        assert!(e.contains(r#"Exec="/opt/AI Usage/a\\\\b\\$x\\`y%%z/ai-usage-tracker" --autostart"#), "{e}");
        assert!(e.starts_with("[Desktop Entry]\nType=Application\n"));

        let config = std::env::temp_dir().join(format!("aiut-xdg-{}", std::process::id()));
        let file = config.join("autostart/ai-usage-tracker.desktop");
        xdg_autostart::apply_in(&config, std::path::Path::new("/usr/bin/ai-usage-tracker"), true).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains(r#"Exec="/usr/bin/ai-usage-tracker" --autostart"#));
        xdg_autostart::apply_in(&config, std::path::Path::new("/usr/bin/ai-usage-tracker"), false).unwrap();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn the_launch_agent_is_written_and_removed() {
        let home = std::env::temp_dir().join(format!("aiut-launch-agent-{}", std::process::id()));
        let file = home.join("Library/LaunchAgents/io.aiusagetracker.desktop.plist");
        let exe = std::path::Path::new("/Applications/AI Usage Tracker.app/Contents/MacOS/ai-usage-tracker");
        launch_agent::apply_in(&home, exe, true).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("<string>/Applications/AI Usage Tracker.app/Contents/MacOS/ai-usage-tracker</string>"));
        launch_agent::apply_in(&home, exe, false).unwrap();
        assert!(!file.exists());
        launch_agent::apply_in(&home, exe, false).expect("removing twice is fine");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_launch_agent_escapes_the_app_path() {
        let p = launch_agent::plist(std::path::Path::new("/Applications/A & <B>.app/Contents/MacOS/ai-usage-tracker"));
        assert!(p.contains("<string>/Applications/A &amp; &lt;B&gt;.app/Contents/MacOS/ai-usage-tracker</string>"));
        assert!(p.contains("<string>--autostart</string>"));
    }
}
