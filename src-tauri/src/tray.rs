//! The tray icon (the notification area at the right end of the taskbar). Optionally it shows
//! a limit's percentage drawn into the icon; its tooltip lists every current limit. It follows
//! new readings, settings and the passing of reset times.

use crate::settings::Settings;
use crate::state::AppState;
use crate::windows::{handle_menu, show_main, system_is_turkish};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tracker_core::analytics::{self, LimitState};
use tracker_core::model::Provider;

const ID: &str = "main-tray";
/// Reset times pass and readings age even when nothing new arrives.
const REFRESH: Duration = Duration::from_secs(20);

static DIRTY: AtomicBool = AtomicBool::new(true);

/// What is on screen now, so the icon and menu are replaced only when they change.
#[derive(Default, PartialEq, Clone)]
struct Shown {
    icon: Option<(String, Level, u32)>,
    tooltip: String,
    menu: (bool, Option<String>, String),
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

/// Asks the tray to update at its next tick (new readings, settings, update status).
pub fn refresh_soon() {
    DIRTY.store(true, Ordering::SeqCst);
}

fn turkish(s: &Settings) -> bool {
    s.language == "tr" || (s.language == "system" && system_is_turkish())
}

fn menu(app: &AppHandle, tr: bool, update: Option<&str>, hotkey: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let t = |en: &str, tr_: &str| if tr { tr_.to_owned() } else { en.to_owned() };
    let open = MenuItem::with_id(app, "tray:open", t("Open dashboard", "Paneli aç"), true, None::<&str>)?;
    // the shortcut goes in the accelerator column (after a tab) as plain text, so a combination
    // the menu's own parser does not know can never break the menu
    let mut widget_text = t("Show/hide widget", "Widget'ı göster/gizle");
    if !hotkey.is_empty() {
        widget_text = format!("{widget_text}\t{}", hotkey.replace("Super", "Win"));
    }
    let widget = MenuItem::with_id(app, "tray:widget", widget_text, true, None::<&str>)?;
    let rescan = MenuItem::with_id(app, "tray:rescan", t("Rescan now", "Şimdi tara"), true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "app:quit", t("Quit", "Çık"), true, None::<&str>)?;
    match update {
        Some(v) => {
            let up = MenuItem::with_id(app, "tray:update", t(&format!("Update to v{v}…"), &format!("v{v} sürümüne güncelle…")), true, None::<&str>)?;
            Menu::with_items(app, &[&open, &widget, &rescan, &sep, &up, &sep2, &quit])
        }
        None => Menu::with_items(app, &[&open, &widget, &rescan, &sep, &quit]),
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let mut b = TrayIconBuilder::with_id(ID).tooltip("AI Usage Tracker").menu(&menu(app, turkish(&s), None, &s.widget.hotkey)?).show_menu_on_left_click(false);
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

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            let mut last = Instant::now() - REFRESH;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if app.state::<AppState>().quitting.load(Ordering::SeqCst) {
                    return;
                }
                if DIRTY.swap(false, Ordering::SeqCst) || last.elapsed() >= REFRESH {
                    last = Instant::now();
                    update(&app);
                }
            }
        })
        .expect("spawn tray updater");
}

// ------------------------------------------------------------------ what to show

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    Normal,
    Warn,
    High,
}

/// One current limit as the tray shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    pub provider: Provider,
    pub window: String,
    pub used: f64,
    /// Used after the reading: the real value is at least `used`.
    pub behind: bool,
}

/// Current five-hour and weekly limits of the enabled providers (fullest reading per window).
/// `None` when they cannot be read right now; the tray then keeps what it shows.
fn current_limits(app: &AppHandle, s: &Settings) -> Option<Vec<Current>> {
    let state = app.state::<AppState>();
    let providers = crate::commands::enabled_providers(s);
    let views = {
        let store = state.store.lock().ok()?;
        let book = state.book.read().unwrap();
        analytics::limits_view(&store, &book, chrono::Utc::now().timestamp_millis(), &[]).ok()?
    };
    let mut out: Vec<Current> = Vec::new();
    for v in views {
        if !providers.contains(&v.provider) || !(v.window == "five_hour" || v.window == "seven_day") {
            continue;
        }
        let (Some(used), true) = (v.used_pct, matches!(v.state, LimitState::Fresh | LimitState::Behind)) else { continue };
        let c = Current { provider: v.provider, window: v.window.clone(), used, behind: v.state == LimitState::Behind };
        match out.iter_mut().find(|x| x.provider == c.provider && x.window == c.window) {
            Some(x) if x.used >= c.used => {}
            Some(x) => *x = c,
            None => out.push(c),
        }
    }
    Some(out)
}

/// The limit the icon shows: the chosen one, or the fullest.
pub fn pick<'a>(limits: &'a [Current], choice: &str) -> Option<&'a Current> {
    match choice.split_once(':') {
        Some((p, w)) => limits.iter().find(|c| c.provider.as_str() == p && c.window == w),
        None => limits.iter().max_by(|a, b| a.used.total_cmp(&b.used)),
    }
}

pub fn level(used: f64, warn_at: f64, high_at: f64) -> Level {
    if used >= high_at {
        Level::High
    } else if used >= warn_at {
        Level::Warn
    } else {
        Level::Normal
    }
}

/// The number drawn into the icon: used or left, as the user chose.
pub fn shown_value(used: f64, remaining: bool) -> u32 {
    let v = if remaining { 100.0 - used } else { used };
    v.clamp(0.0, 100.0).round() as u32
}

fn tooltip(limits: &[Current], s: &Settings, update: Option<&str>) -> String {
    let tr = turkish(s);
    let remaining = s.limit_display == "remaining";
    let mut lines = vec!["AI Usage Tracker".to_owned()];
    for (p, name) in [(Provider::Anthropic, "Claude"), (Provider::OpenAI, "Codex")] {
        let mut parts = Vec::new();
        for (w, en, tr_) in [("five_hour", "5h", "5 sa"), ("seven_day", "week", "haftalık")] {
            let Some(c) = limits.iter().find(|c| c.provider == p && c.window == w) else { continue };
            let v = shown_value(c.used, remaining);
            // "at least" used means "at most" left
            let sign = if c.behind { if remaining { "≤" } else { "≥" } } else { "" };
            let pct = if tr { format!("{sign}%{v}") } else { format!("{sign}{v}%") };
            let left = if remaining { if tr { " kaldı" } else { " left" } } else { "" };
            parts.push(format!("{} {pct}{left}", if tr { tr_ } else { en }));
        }
        if !parts.is_empty() {
            lines.push(format!("{name} · {}", parts.join(" · ")));
        }
    }
    if limits.is_empty() {
        lines.push(if tr { "Güncel limit okuması yok".into() } else { "No current limit reading".into() });
    }
    if let Some(v) = update {
        lines.push(if tr { format!("Güncelleme hazır: v{v}") } else { format!("Update ready: v{v}") });
    }
    // Windows shows at most 127 characters
    let mut text = lines.join("\n");
    if text.chars().count() > 127 {
        text = text.chars().take(126).collect::<String>() + "…";
    }
    text
}

fn update(app: &AppHandle) {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let Some(tray) = app.tray_by_id(ID) else { return };
    let Some(limits) = current_limits(app, &s) else { return };
    let update = app.state::<AppState>().updates.lock().unwrap().available.as_ref().map(|a| a.version.clone());
    let size = icon_size(app);
    let icon = if s.tray.show_percent && s.onboarded {
        pick(&limits, &s.tray.limit).map(|c| {
            (shown_value(c.used, s.limit_display == "remaining").to_string(), level(c.used, s.widget.warn_at, s.widget.high_at), size)
        })
    } else {
        None
    };
    let next = Shown { icon, tooltip: tooltip(&limits, &s, update.as_deref()), menu: (turkish(&s), update.clone(), s.widget.hotkey.clone()) };
    let mut shown = SHOWN.lock().unwrap();
    let prev = shown.clone().unwrap_or_default();
    if shown.is_some() && prev == next {
        return;
    }
    if shown.is_none() || prev.icon != next.icon {
        let image = match &next.icon {
            Some((text, lvl, size)) => Some(Image::new_owned(badge(text, *size, *lvl), *size, *size)),
            None => app.default_window_icon().cloned(),
        };
        if let Err(e) = tray.set_icon(image) {
            log::warn!("tray icon: {e}");
        }
    }
    if shown.is_none() || prev.tooltip != next.tooltip {
        let _ = tray.set_tooltip(Some(&next.tooltip));
    }
    if shown.is_none() || prev.menu != next.menu {
        match menu(app, next.menu.0, next.menu.1.as_deref(), &next.menu.2) {
            Ok(m) => {
                let _ = tray.set_menu(Some(m));
            }
            Err(e) => log::warn!("tray menu: {e}"),
        }
    }
    *shown = Some(next);
}

/// The small-icon size Windows uses for the tray at the primary monitor's scale.
fn icon_size(app: &AppHandle) -> u32 {
    let scale = app.primary_monitor().ok().flatten().map(|m| m.scale_factor()).unwrap_or(1.0);
    ((16.0 * scale).round() as u32).clamp(16, 64)
}

// ------------------------------------------------------------------ drawing

/// 5×7 digits for one or two characters; rows top to bottom, bit 4 = left column.
const DIGITS_5X7: [[u8; 7]; 10] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
];
/// 4×7 digits for "100", so three fit in 16 pixels; bit 3 = left column.
const DIGITS_4X7: [[u8; 7]; 10] = [
    [0x6, 0x9, 0x9, 0x9, 0x9, 0x9, 0x6],
    [0x2, 0x6, 0x2, 0x2, 0x2, 0x2, 0x7],
    [0x6, 0x9, 0x1, 0x2, 0x4, 0x8, 0xF],
    [0xE, 0x1, 0x1, 0x6, 0x1, 0x1, 0xE],
    [0x2, 0x6, 0xA, 0xF, 0x2, 0x2, 0x2],
    [0xF, 0x8, 0xE, 0x1, 0x1, 0x9, 0x6],
    [0x6, 0x8, 0x8, 0xE, 0x9, 0x9, 0x6],
    [0xF, 0x1, 0x2, 0x2, 0x4, 0x4, 0x4],
    [0x6, 0x9, 0x9, 0x6, 0x9, 0x9, 0x6],
    [0x6, 0x9, 0x9, 0x7, 0x1, 0x1, 0x6],
];

fn colors(level: Level) -> ([u8; 3], [u8; 3]) {
    match level {
        // slate with white digits: readable on light and dark taskbars
        Level::Normal => ([0x33, 0x41, 0x55], [0xFF, 0xFF, 0xFF]),
        // amber with near-black digits (white on amber is too faint)
        Level::Warn => ([0xF5, 0x9E, 0x0B], [0x11, 0x11, 0x11]),
        Level::High => ([0xDC, 0x26, 0x26], [0xFF, 0xFF, 0xFF]),
    }
}

/// RGBA pixels of a `size`×`size` rounded badge with `text` (digits only) centred in it,
/// drawn with whole-pixel scaling so the digits stay sharp.
pub fn badge(text: &str, size: u32, level: Level) -> Vec<u8> {
    let s = size as i64;
    let (bg, fg) = colors(level);
    let mut px = vec![0u8; (size * size * 4) as usize];
    // rounded square, anti-aliased corners
    let r = (s as f64 * 0.22).max(2.0);
    for y in 0..s {
        for x in 0..s {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let cx = fx.clamp(r, s as f64 - r);
            let cy = fy.clamp(r, s as f64 - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let a = (r + 0.5 - d).clamp(0.0, 1.0);
            if a > 0.0 {
                let i = ((y * s + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&bg);
                px[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    let digits: Vec<usize> = text.chars().filter_map(|c| c.to_digit(10)).map(|d| d as usize).collect();
    if digits.is_empty() {
        return px;
    }
    let narrow = digits.len() >= 3;
    let (gw, gh) = if narrow { (4i64, 7i64) } else { (5, 7) };
    let n = digits.len() as i64;
    let units_w = n * gw + (n - 1);
    // largest whole-pixel scale that leaves a one-pixel margin
    let unit = (((s - 2) / units_w).min((s - 2) / gh)).max(1);
    let (w, h) = (units_w * unit, gh * unit);
    let (ox, oy) = ((s - w) / 2, (s - h) / 2);
    for (k, d) in digits.iter().enumerate() {
        for row in 0..gh {
            let bits = if narrow { DIGITS_4X7[*d][row as usize] } else { DIGITS_5X7[*d][row as usize] };
            for col in 0..gw {
                if bits >> (gw - 1 - col) & 1 == 0 {
                    continue;
                }
                let x0 = ox + (k as i64 * (gw + 1) + col) * unit;
                let y0 = oy + row * unit;
                for y in y0..y0 + unit {
                    for x in x0..x0 + unit {
                        if (0..s).contains(&x) && (0..s).contains(&y) {
                            let i = ((y * s + x) * 4) as usize;
                            px[i..i + 3].copy_from_slice(&fg);
                            px[i + 3] = 255;
                        }
                    }
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(px: &[u8], size: u32, fg: [u8; 3]) -> Vec<(u32, u32)> {
        (0..size * size).filter(|i| px[(*i * 4) as usize..(*i * 4 + 3) as usize] == fg && px[(*i * 4 + 3) as usize] == 255).map(|i| (i % size, i / size)).collect()
    }

    #[test]
    fn digits_are_centred_whole_pixels_with_a_margin() {
        for size in [16, 20, 24, 32, 40] {
            for text in ["7", "42", "100"] {
                let px = badge(text, size, Level::Normal);
                assert_eq!(px.len(), (size * size * 4) as usize);
                let on = lit(&px, size, colors(Level::Normal).1);
                assert!(!on.is_empty(), "{text} at {size}");
                let (minx, maxx) = (on.iter().map(|p| p.0).min().unwrap(), on.iter().map(|p| p.0).max().unwrap());
                let (miny, maxy) = (on.iter().map(|p| p.1).min().unwrap(), on.iter().map(|p| p.1).max().unwrap());
                assert!(minx >= 1 && miny >= 1 && maxx <= size - 2 && maxy <= size - 2, "{text} at {size}: {minx}-{maxx} {miny}-{maxy}");
                // the glyph boxes are centred; a narrow glyph (the 1) may sit a pixel or two off
                assert!((minx as i64 - (size - 1 - maxx) as i64).abs() <= 2, "{text} at {size}");
            }
        }
    }

    #[test]
    fn corners_are_transparent_and_the_middle_is_filled() {
        let px = badge("", 16, Level::High);
        assert_eq!(px[3], 0);
        let mid = ((8 * 16 + 8) * 4) as usize;
        assert_eq!(&px[mid..mid + 4], &[0xDC, 0x26, 0x26, 255]);
    }

    #[test]
    fn the_fullest_current_limit_is_shown_unless_one_is_chosen() {
        let l = vec![
            Current { provider: Provider::Anthropic, window: "five_hour".into(), used: 30.0, behind: false },
            Current { provider: Provider::OpenAI, window: "seven_day".into(), used: 68.0, behind: false },
        ];
        assert_eq!(pick(&l, "auto").unwrap().used, 68.0);
        assert_eq!(pick(&l, "anthropic:five_hour").unwrap().used, 30.0);
        assert!(pick(&l, "anthropic:seven_day").is_none());
        assert_eq!(shown_value(68.4, false), 68);
        assert_eq!(shown_value(68.4, true), 32);
        assert_eq!(level(69.0, 70.0, 90.0), Level::Normal);
        assert_eq!(level(70.0, 70.0, 90.0), Level::Warn);
        assert_eq!(level(95.0, 70.0, 90.0), Level::High);
    }

    #[test]
    fn the_tooltip_lists_every_limit_in_the_users_language() {
        let mut s = Settings { language: "tr".into(), ..Default::default() };
        let l = vec![
            Current { provider: Provider::Anthropic, window: "five_hour".into(), used: 41.0, behind: false },
            Current { provider: Provider::Anthropic, window: "seven_day".into(), used: 43.0, behind: true },
            Current { provider: Provider::OpenAI, window: "five_hour".into(), used: 7.0, behind: false },
        ];
        assert_eq!(tooltip(&l, &s, Some("0.3.0")), "AI Usage Tracker\nClaude · 5 sa %41 · haftalık ≥%43\nCodex · 5 sa %7\nGüncelleme hazır: v0.3.0");
        s.language = "en".into();
        s.limit_display = "remaining".into();
        assert_eq!(tooltip(&l, &s, None), "AI Usage Tracker\nClaude · 5h 59% left · week ≤57% left\nCodex · 5h 93% left");
        assert!(tooltip(&[], &s, None).ends_with("No current limit reading"));
    }
}
