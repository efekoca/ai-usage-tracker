use crate::settings::{Settings, TraySettings};
use crate::tray_text::{needs_usage, shown_value, Data, Format, Reading, Usage};
use crate::state::AppState;
use crate::windows::{handle_menu, system_is_turkish};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tracker_core::analytics::{self, LimitState, Period};
use tracker_core::discovery::SourceId;
use tracker_core::insights::ForecastKind;
use tracker_core::model::{Provider, Tool};
use tracker_core::store::EventRow;

const ID: &str = "main-tray";
/// Reset times pass and readings age even when nothing new arrives.
const REFRESH: Duration = Duration::from_secs(20);

static DIRTY: AtomicBool = AtomicBool::new(true);

/// Last applied state, so the icon and menu are replaced only when they change.
#[derive(Default, PartialEq, Clone)]
struct Shown {
    icon: Option<(String, Level, u32)>,
    /// macOS: text without the icon.
    icon_hidden: bool,
    title: String,
    tooltip: String,
    menu: (bool, Option<String>, String),
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

pub fn refresh_soon() {
    DIRTY.store(true, Ordering::SeqCst);
}

pub(crate) fn turkish(s: &Settings) -> bool {
    s.language == "tr" || (s.language == "system" && system_is_turkish())
}

fn menu(app: &AppHandle, tr: bool, update: Option<&str>, hotkey: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let t = |en: &str, tr_: &str| if tr { tr_.to_owned() } else { en.to_owned() };
    let open = MenuItem::with_id(app, "tray:open", t("Open dashboard", "Paneli aç"), true, None::<&str>)?;
    let widget_text = t("Show/hide widget", "Widget'ı göster/gizle");
    // macOS draws the shortcut (⌃⌥⇧W) in the menu's key column; a combination the menu cannot
    // parse just goes without it, so it can never break the menu
    #[cfg(target_os = "macos")]
    let widget = MenuItem::with_id(app, "tray:widget", &widget_text, true, (!hotkey.is_empty()).then_some(hotkey))
        .or_else(|_| MenuItem::with_id(app, "tray:widget", &widget_text, true, None::<&str>))?;
    // elsewhere it goes in the accelerator column (after a tab) as plain text, for the same reason
    #[cfg(not(target_os = "macos"))]
    let widget = {
        // Linux desktops call the key Super, as stored
        let shown = if cfg!(windows) { hotkey.replace("Super", "Win") } else { hotkey.to_owned() };
        let text = if hotkey.is_empty() { widget_text } else { format!("{widget_text}\t{shown}") };
        MenuItem::with_id(app, "tray:widget", text, true, None::<&str>)?
    };
    let rescan = MenuItem::with_id(app, "tray:rescan", t("Rescan now", "Şimdi tara"), true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "app:quit", t("Quit", "Çık"), true, None::<&str>)?;
    let menu = match update {
        Some(v) => {
            let up = MenuItem::with_id(app, "tray:update", t(&format!("Update to v{v}…"), &format!("v{v} sürümüne güncelle…")), true, None::<&str>)?;
            Menu::with_items(app, &[&open, &widget, &rescan, &sep, &up, &sep2, &quit])?
        }
        None => Menu::with_items(app, &[&open, &widget, &rescan, &sep, &quit])?,
    };
    // Linux trays report no clicks, so the panel is opened from the menu
    #[cfg(target_os = "linux")]
    menu.insert(&MenuItem::with_id(app, "tray:panel", t("Control panel", "Kontrol paneli"), true, None::<&str>)?, 0)?;
    Ok(menu)
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let mut b = TrayIconBuilder::with_id(ID).tooltip("AI Usage Tracker").menu(&menu(app, turkish(&s), None, &s.widget.hotkey)?).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.on_menu_event(|app, e| handle_menu(app, e.id().as_ref()))
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = e {
                crate::panel::toggle(tray.app_handle(), Some(rect));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    Normal,
    Warn,
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Current {
    pub provider: Provider,
    pub window: String,
    pub used: f64,
    /// Used after the reading: the real value is at least `used`.
    pub behind: bool,
}

/// Every pool's current window; `None` when limits cannot be read right now, so the tray keeps
/// what it shows.
pub(crate) fn readings(app: &AppHandle, s: &Settings) -> Option<Vec<Reading>> {
    let state = app.state::<AppState>();
    let providers = crate::commands::enabled_providers(s);
    let views = {
        let store = state.db();
        let book = state.book.read().unwrap();
        analytics::limits_view(&store, &book, chrono::Utc::now().timestamp_millis(), &[]).ok()?
    };
    Some(
        views
            .into_iter()
            .filter(|v| providers.contains(&v.provider) && (v.window == "five_hour" || v.window == "seven_day"))
            .filter(|v| matches!(v.state, LimitState::Fresh | LimitState::Behind))
            .filter_map(|v| {
                let (fills_at_ms, at_reset) = match &v.forecast {
                    Some(f) if f.kind == ForecastKind::Fills => (f.fills_at_ms, None),
                    Some(f) if matches!(f.kind, ForecastKind::Safe | ForecastKind::Idle) => (None, f.at_reset_pct),
                    _ => (None, None),
                };
                Some(Reading {
                    provider: v.provider,
                    pool: v.limit_id,
                    window: v.window,
                    used: v.used_pct?,
                    behind: v.state == LimitState::Behind,
                    resets_at: v.resets_at,
                    fills_at_ms,
                    at_reset,
                })
            })
            .collect(),
    )
}

/// The fullest pool of each provider's window, for the icon and the tooltip.
fn current_limits(readings: &[Reading]) -> Vec<Current> {
    let mut out: Vec<Current> = Vec::new();
    for r in readings {
        let c = Current { provider: r.provider, window: r.window.clone(), used: r.used, behind: r.behind };
        match out.iter_mut().find(|x| x.provider == c.provider && x.window == c.window) {
            Some(x) if x.used >= c.used => {}
            Some(x) => *x = c,
            None => out.push(c),
        }
    }
    out
}

/// The shown tools' usage today, over 7 and over 30 days, and OpenCode's alone while its source is on.
pub(crate) fn usage(app: &AppHandle, s: &Settings) -> Option<([Usage; 3], Option<[Usage; 3]>)> {
    let state = app.state::<AppState>();
    let providers = crate::commands::enabled_providers(s);
    let opencode = s.enabled_sources.contains(&SourceId::OpenCode);
    let store = state.db();
    let book = state.book.read().unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let starts = [Period::Today, Period::Days7, Period::Month1].map(|p| analytics::period_range(p, &chrono::Local, now, None).from_ms);
    let mut out = [Usage::default(); 3];
    let mut oc = [Usage::default(); 3];
    let mut sessions: [std::collections::HashSet<String>; 3] = Default::default();
    let mut oc_sessions: [std::collections::HashSet<String>; 3] = Default::default();
    for e in store.events_between(starts[2], now + 1).ok()? {
        if !e.tool.provider().map_or(opencode, |p| providers.contains(&p)) {
            continue;
        }
        let cost = book.cost(&tracker_core::pricing::CostInput {
            model: &e.model,
            tokens: &e.tokens,
            request_input: e.request_input,
            web_search_requests: e.web_search,
            speed: e.speed.as_deref(),
            inference_geo: e.inference_geo.as_deref(),
            ts_ms: Some(e.ts_ms),
        });
        let c = cost.as_ref().map_or(0.0, |c| c.total());
        for i in (0..3).filter(|&i| e.ts_ms >= starts[i]) {
            add(&mut out[i], &mut sessions[i], &e, c);
            if e.tool == Tool::OpenCode {
                add(&mut oc[i], &mut oc_sessions[i], &e, c);
            }
        }
    }
    for (u, ids) in out.iter_mut().zip(&sessions).chain(oc.iter_mut().zip(&oc_sessions)) {
        u.sessions = ids.len() as u64;
    }
    Some((out, opencode.then_some(oc)))
}

fn add(u: &mut Usage, sessions: &mut std::collections::HashSet<String>, e: &EventRow, cost: f64) {
    u.tokens += e.tokens.total();
    u.cost_usd += cost;
    u.prompt += e.tokens.input + e.tokens.cache_read + e.tokens.cache_write;
    u.cache_read += e.tokens.cache_read;
    if let Some(id) = &e.session_id {
        sessions.insert(format!("{}:{id}", e.tool.as_str()));
    }
}

pub(crate) fn data(app: &AppHandle, s: &Settings, cfg: &TraySettings, readings: Vec<Reading>) -> Data {
    let (usage, opencode) = if needs_usage(cfg) { usage(app, s).map_or((None, None), |(u, o)| (Some(u), o)) } else { (None, None) };
    let scan_ms = app.state::<AppState>().status.lock().unwrap().last_scan_ms;
    Data { readings, usage, opencode, scan_ms }
}

pub(crate) fn format<'a>(s: &'a Settings, cfg: &'a TraySettings) -> Format<'a, chrono::Local> {
    Format {
        cfg,
        tr: turkish(s),
        app_remaining: s.limit_display == "remaining",
        warn_at: s.widget.warn_at,
        currency: &s.currency,
        fx: s.fx_rate,
        now_ms: chrono::Utc::now().timestamp_millis(),
        tz: &chrono::Local,
    }
}

/// The text next to the icon for `cfg`, as the tray shows it; empty on Windows.
pub(crate) fn title(app: &AppHandle, s: &Settings, cfg: &TraySettings, readings: &[Reading]) -> String {
    if !cfg.title || cfg!(windows) {
        return String::new();
    }
    format(s, cfg).title(&data(app, s, cfg, readings.to_vec()))
}

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


fn tooltip(limits: &[Current], s: &Settings, update: Option<&str>) -> String {
    let tr = turkish(s);
    let remaining = s.limit_display == "remaining";
    let mut lines = vec!["AI Usage Tracker".to_owned()];
    for (p, name) in [(Provider::Anthropic, "Claude"), (Provider::OpenAI, "Codex"), (Provider::Google, "Antigravity")] {
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
    let Some(readings) = readings(app, &s) else { return };
    let limits = current_limits(&readings);
    let update = app.state::<AppState>().updates.lock().unwrap().available.as_ref().map(|a| a.version.clone());
    let size = icon_size(app);
    let icon = if s.tray.show_percent && s.onboarded {
        pick(&limits, &s.tray.limit).map(|c| {
            (shown_value(c.used, s.limit_display == "remaining").to_string(), level(c.used, s.widget.warn_at, s.widget.high_at), size)
        })
    } else {
        None
    };
    let title = if s.onboarded { title(app, &s, &s.tray, &readings) } else { String::new() };
    // a menu bar item needs something to show: the icon stays while the text is empty
    let icon_hidden = cfg!(target_os = "macos") && s.tray.hide_icon && !title.is_empty();
    let next = Shown {
        icon,
        icon_hidden,
        title,
        tooltip: tooltip(&limits, &s, update.as_deref()),
        menu: (turkish(&s), update.clone(), s.widget.hotkey.clone()),
    };
    let mut shown = SHOWN.lock().unwrap();
    let prev = shown.clone().unwrap_or_default();
    if shown.is_some() && prev == next {
        return;
    }
    if shown.is_none() || prev.icon != next.icon || prev.icon_hidden != next.icon_hidden {
        let image = match &next.icon {
            _ if next.icon_hidden => None,
            Some((text, lvl, size)) => Some(Image::new_owned(badge(text, *size, *lvl), *size, *size)),
            None => app.default_window_icon().cloned(),
        };
        if let Err(e) = tray.set_icon(image) {
            log::warn!("tray icon: {e}");
        }
    }
    // Windows has no text next to a tray icon
    #[cfg(not(windows))]
    if shown.is_none() || prev.title != next.title {
        let _ = tray.set_title((!next.title.is_empty()).then_some(next.title.as_str()));
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

/// Windows uses the small-icon size, scaled by the primary monitor, for the tray.
fn icon_size(app: &AppHandle) -> u32 {
    let scale = app.primary_monitor().ok().flatten().map(|m| m.scale_factor()).unwrap_or(1.0);
    ((16.0 * scale).round() as u32).clamp(16, 64)
}

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

/// Whole-pixel scaling keeps the digits sharp.
pub fn badge(text: &str, size: u32, level: Level) -> Vec<u8> {
    let s = size as i64;
    let (bg, fg) = colors(level);
    let mut px = vec![0u8; (size * size * 4) as usize];
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
