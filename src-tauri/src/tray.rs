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

pub(crate) const ID: &str = "main-tray";
/// Reset times pass and readings age even when nothing new arrives.
const REFRESH: Duration = Duration::from_secs(20);

static DIRTY: AtomicBool = AtomicBool::new(true);

/// Last applied state, so the icon and menu are replaced only when they change.
#[derive(Default, PartialEq, Clone)]
struct Shown {
    icon: Option<(Badge, u32)>,
    /// macOS: text without the icon.
    icon_hidden: bool,
    /// Windows: the plain mark follows the taskbar's light or dark look.
    light_taskbar: bool,
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

/// The app's mark without a badge: on macOS a template image, which the menu bar tints for its
/// light or dark look; on Windows the tray mark drawn for the taskbar's look.
fn plain_icon(app: &AppHandle) -> Option<Image<'static>> {
    #[cfg(target_os = "macos")]
    if let Ok(mark) = Image::from_bytes(include_bytes!("../icons/menu-bar-template.png")) {
        return Some(mark);
    }
    #[cfg(windows)]
    {
        let ico: &[u8] = if crate::windows::light_taskbar() {
            include_bytes!("../icons/tray-light-taskbar.ico")
        } else {
            include_bytes!("../icons/tray-dark-taskbar.ico")
        };
        if let Some(mark) = ico_entry(ico, icon_size(app)).and_then(|png| Image::from_bytes(png).ok()) {
            return Some(mark);
        }
    }
    app.default_window_icon().map(|i| i.clone().to_owned())
}

/// The PNG of an `.ico` drawn for `size`, else for the next size up, else its largest.
#[cfg(any(windows, test))]
fn ico_entry(ico: &[u8], size: u32) -> Option<&[u8]> {
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let mut entries: Vec<(u32, &[u8])> = (0..count)
        .filter_map(|i| {
            let e = ico.get(6 + 16 * i..22 + 16 * i)?;
            let side = if e[0] == 0 { 256 } else { e[0] as u32 };
            let len = u32::from_le_bytes(e[8..12].try_into().ok()?) as usize;
            let at = u32::from_le_bytes(e[12..16].try_into().ok()?) as usize;
            let data = ico.get(at..at.checked_add(len)?)?;
            data.starts_with(b"\x89PNG").then_some((side, data))
        })
        .collect();
    entries.sort_by_key(|e| e.0);
    entries.iter().find(|e| e.0 >= size).or(entries.last()).map(|e| e.1)
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let mut b = TrayIconBuilder::with_id(ID).tooltip("AI Usage Tracker").menu(&menu(app, turkish(&s), None, &s.widget.hotkey)?).show_menu_on_left_click(false);
    if let Some(icon) = plain_icon(app) {
        b = b.icon(icon).icon_as_template(cfg!(target_os = "macos"));
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

/// What the badge shows: the limit's percent or a tool's logo, by its key in `LOGOS`.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    Number(String),
    Logo(&'static str),
    /// The app's dial, its arc filled to this share (0–1) of the limit.
    Dial(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    pub mark: Mark,
    pub level: Level,
    /// The tool's color, shown while the limit is not close.
    pub tint: Option<[u8; 3]>,
}

/// Every pool's current window, of each known Codex and Claude account too; `None` when limits cannot be read
/// right now, so the tray keeps what it shows.
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
            .filter(|v| (v.active || v.account.is_some()) && providers.contains(&v.provider) && (v.window == "five_hour" || v.window == "seven_day"))
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
                    account: v.account,
                    active: v.active,
                })
            })
            .collect(),
    )
}

/// The fullest pool of each provider's window in use, for the icon and the tooltip.
pub(crate) fn current_limits(readings: &[Reading]) -> Vec<Current> {
    let mut out: Vec<Current> = Vec::new();
    for r in readings.iter().filter(|r| r.active) {
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
    let accounts = if cfg.items.iter().any(|i| !i.account.is_empty()) { account_tags(app, s) } else { Default::default() };
    Data { readings, usage, opencode, scan_ms, accounts }
}

/// Each account's short name: the user's, else "Account" and its number among its provider's
/// accounts in order of first use, as the Limits page numbers them.
fn account_tags(app: &AppHandle, s: &Settings) -> std::collections::HashMap<String, String> {
    let list = tracker_core::accounts::accounts(&app.state::<AppState>().db()).unwrap_or_default();
    let mut counts: std::collections::HashMap<Provider, usize> = std::collections::HashMap::new();
    list.into_iter()
        .filter_map(|a| Some((a.provider, a.id?)))
        .map(|(provider, id)| {
            let n = counts.entry(provider).or_default();
            *n += 1;
            let tag = s.account_names.get(&id).cloned().unwrap_or_else(|| format!("{} {n}", if turkish(s) { "Hesap" } else { "Account" }));
            (id, tag)
        })
        .collect()
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

/// The badge `cfg` draws, colored by its limit; `None` keeps the app icon.
pub(crate) fn badge(s: &Settings, cfg: &TraySettings, limits: &[Current]) -> Option<Badge> {
    if !cfg.show_percent || !s.onboarded {
        return None;
    }
    let c = pick(limits, &cfg.limit);
    let level = c.map_or(Level::Normal, |c| level(c.used, s.widget.warn_at, s.widget.high_at));
    let (mark, tool) = match cfg.mark.as_str() {
        "number" => {
            let c = c?;
            (Mark::Number(shown_value(c.used, s.limit_display == "remaining").to_string()), c.provider.as_str())
        }
        "tool" => (Mark::Logo(c?.provider.as_str()), c?.provider.as_str()),
        "dial" => {
            let c = c?;
            (Mark::Dial(shown_value(c.used, s.limit_display == "remaining") as f64 / 100.0), c.provider.as_str())
        }
        key => {
            let key = LOGOS.iter().find(|l| l.0 == key)?.0;
            (Mark::Logo(key), key)
        }
    };
    let tint = if cfg.tool_color { tool_color(s, tool) } else { None };
    Some(Badge { mark, level, tint })
}

/// The color chosen for a tool in the app, else the tool's own.
fn tool_color(s: &Settings, tool: &str) -> Option<[u8; 3]> {
    let chosen = s.provider_colors.get(tool).and_then(|c| u32::from_str_radix(c.strip_prefix('#')?, 16).ok());
    chosen.map(|v| [(v >> 16) as u8, (v >> 8) as u8, v as u8]).or_else(|| LOGOS.iter().find(|l| l.0 == tool).map(|l| l.2))
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
            // the last reading: use since then raises it, a reset the provider grants lowers it
            let sign = if c.behind { "~" } else { "" };
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
    let icon = badge(&s, &s.tray, &limits).map(|b| (b, size));
    let title = if s.onboarded { title(app, &s, &s.tray, &readings) } else { String::new() };
    // a menu bar item needs something to show: the icon stays while the text is empty
    let icon_hidden = cfg!(target_os = "macos") && s.tray.hide_icon && !title.is_empty();
    #[cfg(windows)]
    let light_taskbar = crate::windows::light_taskbar();
    #[cfg(not(windows))]
    let light_taskbar = false;
    let next = Shown {
        icon,
        icon_hidden,
        light_taskbar,
        title,
        tooltip: tooltip(&limits, &s, update.as_deref()),
        menu: (turkish(&s), update.clone(), s.widget.hotkey.clone()),
    };
    let mut shown = SHOWN.lock().unwrap();
    let prev = shown.clone().unwrap_or_default();
    if shown.is_some() && prev == next {
        return;
    }
    if shown.is_none() || prev.icon != next.icon || prev.icon_hidden != next.icon_hidden || prev.light_taskbar != next.light_taskbar {
        let image = match &next.icon {
            _ if next.icon_hidden => None,
            Some((b, size)) => Some(Image::new_owned(draw(b, *size), *size, *size)),
            None => plain_icon(app),
        };
        if let Err(e) = tray.set_icon(image) {
            log::warn!("tray icon: {e}");
        }
        // setting an icon clears the template flag
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos") && next.icon.as_ref().is_none_or(|(b, _)| is_template(b)));
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

/// The app's monochrome marks (24×24, even-odd), from lobe-icons, MIT License, Copyright (c) 2023 LobeHub
/// (github.com/lobehub/lobe-icons), with each tool's color. The marks and names belong to their owners.
const LOGOS: [(&str, &str, [u8; 3]); 4] = [
    ("anthropic", "M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z", [0xD9, 0x77, 0x57]),
    ("openai", "M8.086.457a6.105 6.105 0 013.046-.415c1.333.153 2.521.72 3.564 1.7a.117.117 0 00.107.029c1.408-.346 2.762-.224 4.061.366l.063.03.154.076c1.357.703 2.33 1.77 2.918 3.198.278.679.418 1.388.421 2.126a5.655 5.655 0 01-.18 1.631.167.167 0 00.04.155 5.982 5.982 0 011.578 2.891c.385 1.901-.01 3.615-1.183 5.14l-.182.22a6.063 6.063 0 01-2.934 1.851.162.162 0 00-.108.102c-.255.736-.511 1.364-.987 1.992-1.199 1.582-2.962 2.462-4.948 2.451-1.583-.008-2.986-.587-4.21-1.736a.145.145 0 00-.14-.032c-.518.167-1.04.191-1.604.185a5.924 5.924 0 01-2.595-.622 6.058 6.058 0 01-2.146-1.781c-.203-.269-.404-.522-.551-.821a7.74 7.74 0 01-.495-1.283 6.11 6.11 0 01-.017-3.064.166.166 0 00.008-.074.115.115 0 00-.037-.064 5.958 5.958 0 01-1.38-2.202 5.196 5.196 0 01-.333-1.589 6.915 6.915 0 01.188-2.132c.45-1.484 1.309-2.648 2.577-3.493.282-.188.55-.334.802-.438.286-.12.573-.22.861-.304a.129.129 0 00.087-.087A6.016 6.016 0 015.635 2.31C6.315 1.464 7.132.846 8.086.457zm-.804 7.85a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393zm5.446 6.24a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z", [0x10, 0xA3, 0x7F]),
    ("google", "M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z", [0x42, 0x85, 0xF4]),
    ("opencode", "M16 6H8v12h8V6zm4 16H4V2h16v20z", [0x52, 0x51, 0x4E]),
];

/// The square's color and the mark's: the tool's while the limit is not close, white on it unless it is too light.
pub(crate) fn colors(b: &Badge) -> ([u8; 3], [u8; 3]) {
    match (b.level, b.tint) {
        (Level::Normal, Some(bg)) => {
            let lin = |c: u8| {
                let c = c as f64 / 255.0;
                if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            let lum = 0.2126 * lin(bg[0]) + 0.7152 * lin(bg[1]) + 0.0722 * lin(bg[2]);
            // white while it keeps the 3:1 contrast icons need
            (bg, if 1.05 / (lum + 0.05) >= 3.0 { [0xFF, 0xFF, 0xFF] } else { [0x11, 0x11, 0x11] })
        }
        (level, _) => level_colors(level),
    }
}

fn level_colors(level: Level) -> ([u8; 3], [u8; 3]) {
    match level {
        // slate with white digits: readable on light and dark taskbars
        Level::Normal => ([0x33, 0x41, 0x55], [0xFF, 0xFF, 0xFF]),
        // amber with near-black digits (white on amber is too faint)
        Level::Warn => ([0xF5, 0x9E, 0x0B], [0x11, 0x11, 0x11]),
        Level::High => ([0xDC, 0x26, 0x26], [0xFF, 0xFF, 0xFF]),
    }
}

pub fn draw(b: &Badge, size: u32) -> Vec<u8> {
    let s = size as i64;
    let mut px = vec![0u8; (size * size * 4) as usize];
    if let Mark::Dial(used) = b.mark {
        let look = dial_look(b.level);
        if look == DialLook::Square {
            square(&mut px, s, NAVY);
        }
        dial(&mut px, s, used, b.level, look);
        return px;
    }
    let (bg, fg) = colors(b);
    square(&mut px, s, bg);
    match &b.mark {
        Mark::Number(text) => number(&mut px, s, text, fg),
        Mark::Logo(key) => {
            if let Some((_, d, _)) = LOGOS.iter().find(|l| l.0 == *key) {
                logo(&mut px, s, d, fg);
            }
        }
        Mark::Dial(_) => {}
    }
    px
}

/// The badge's rounded square.
fn square(px: &mut [u8], s: i64, bg: [u8; 3]) {
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
}

/// Whole-pixel scaling keeps the digits sharp.
fn number(px: &mut [u8], s: i64, text: &str, fg: [u8; 3]) {
    let digits: Vec<usize> = text.chars().filter_map(|c| c.to_digit(10)).map(|d| d as usize).collect();
    if digits.is_empty() {
        return;
    }
    let narrow = digits.len() >= 3;
    let (gw, gh) = if narrow { (4i64, 7i64) } else { (5, 7) };
    let n = digits.len() as i64;
    let units_w = n * gw + (n - 1);
    // largest whole-pixel scale that leaves a one-pixel margin; one digit is sized as two so "7" matches "28"
    let fit_w = n.max(2) * gw + (n.max(2) - 1);
    let unit = (((s - 2) / fit_w).min((s - 2) / gh)).max(1);
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
}

/// The mark fills most of the badge.
fn logo(px: &mut [u8], s: i64, d: &str, fg: [u8; 3]) {
    use kurbo::{Affine, BezPath, Shape};
    let Ok(mut path) = BezPath::from_svg(d) else { return };
    let b = path.bounding_box();
    let side = s as f64;
    let k = side * 0.72 / b.width().max(b.height());
    path.apply_affine(Affine::new([k, 0.0, 0.0, k, (side - b.width() * k) / 2.0 - b.x0 * k, (side - b.height() * k) / 2.0 - b.y0 * k]));
    fill(px, s, &path, fg, 1.0);
}

/// Paints `path` (even-odd) at `alpha` over `px`, anti-aliased: exact coverage across a row, 16
/// samples down a pixel.
fn fill(px: &mut [u8], s: i64, path: &kurbo::BezPath, rgb: [u8; 3], alpha: f64) {
    use kurbo::{PathEl, Point};
    const SUB: usize = 16;
    let side = s as f64;
    let mut edges: Vec<(Point, Point)> = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    // a fill closes every subpath, closed or not
    kurbo::flatten(path.iter(), 0.02, |el| match el {
        PathEl::MoveTo(p) => {
            if last != start {
                edges.push((last, start));
            }
            (start, last) = (p, p);
        }
        PathEl::LineTo(p) => {
            edges.push((last, p));
            last = p;
        }
        PathEl::ClosePath => {
            if last != start {
                edges.push((last, start));
            }
            last = start;
        }
        _ => {}
    });
    if last != start {
        edges.push((last, start));
    }
    let n = s as usize;
    let mut cover = vec![0f64; n * n];
    let mut xs = Vec::new();
    for row in 0..n * SUB {
        let y = (row as f64 + 0.5) / SUB as f64;
        xs.clear();
        xs.extend(edges.iter().filter(|(a, b)| (a.y <= y) != (b.y <= y)).map(|(a, b)| a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y)));
        xs.sort_by(f64::total_cmp);
        for [x0, x1] in xs.as_chunks::<2>().0 {
            let (x0, x1) = (x0.clamp(0.0, side), x1.clamp(0.0, side));
            for col in x0.floor() as usize..(x1.ceil() as usize).min(n) {
                let c = col as f64;
                cover[row / SUB * n + col] += (x1.min(c + 1.0) - x0.max(c)).max(0.0) / SUB as f64;
            }
        }
    }
    for (i, a) in cover.iter().enumerate() {
        let a = a.min(1.0) * alpha;
        if a <= 0.0 {
            continue;
        }
        let p = &mut px[i * 4..i * 4 + 4];
        let under = p[3] as f64 / 255.0 * (1.0 - a);
        let out = a + under;
        for c in 0..3 {
            p[c] = ((rgb[c] as f64 * a + p[c] as f64 * under) / out).round() as u8;
        }
        p[3] = (out * 255.0).round() as u8;
    }
}

/// The app icon's navy, its dial's track and its needle.
pub(crate) const NAVY: [u8; 3] = [0x16, 0x1E, 0x3E];
const TRACK: [u8; 3] = [0x28, 0x32, 0x56];
const NEEDLE: [u8; 3] = [0xF2, 0xF5, 0xFA];

#[derive(Debug, Clone, Copy, PartialEq)]
enum DialLook {
    /// macOS tints a template image for its light or dark menu bar.
    Template,
    /// On the icon's navy square: taskbars may be light or dark, and a warning needs its color.
    Square,
}

fn dial_look(level: Level) -> DialLook {
    if cfg!(target_os = "macos") && level == Level::Normal { DialLook::Template } else { DialLook::Square }
}

/// The badge is drawn for macOS to tint.
pub(crate) fn is_template(b: &Badge) -> bool {
    matches!(b.mark, Mark::Dial(_)) && dial_look(b.level) == DialLook::Template
}

/// The dial's arc: the icon's blue, the warning colors as the limit gets close.
pub(crate) fn arc_color(level: Level) -> [u8; 3] {
    match level {
        Level::Normal => [0x38, 0x84, 0xFF],
        level => level_colors(level).0,
    }
}

/// A gauge open at the bottom: the track, the arc to `used`, and the sparkle needle pointing at it.
fn dial(px: &mut [u8], s: i64, used: f64, level: Level, look: DialLook) {
    use kurbo::{Arc, BezPath, Cap, Point, Shape, Stroke, StrokeOpts, Vec2};
    use std::f64::consts::FRAC_PI_2;
    let side = s as f64;
    let used = used.clamp(0.0, 1.0);
    let (unit, track, track_alpha, arc, needle) = match look {
        DialLook::Template => (side / 24.0, [0; 3], 0.35, [0; 3], [0; 3]),
        DialLook::Square => (side * 0.84 / 24.0, TRACK, 1.0, arc_color(level), NEEDLE),
    };
    // the gap at the bottom leaves the ring looking high, so it sits a little lower
    let c = Point::new(side / 2.0, side / 2.0 + 0.6 * unit);
    let r = 8.0 * unit;
    // small icons get a thicker ring so it still reads
    let width = if side < 28.0 { 3.4 } else { 2.8 } * unit;
    let ring = |sweep: f64| {
        let arc = Arc::new(c, (r, r), 135f64.to_radians(), sweep.to_radians(), 0.0);
        kurbo::stroke(arc.path_elements(0.05), &Stroke::new(width).with_caps(Cap::Round), &StrokeOpts::default(), 0.05)
    };
    fill(px, s, &ring(270.0), track, track_alpha);
    if used > 0.0 {
        fill(px, s, &ring(270.0 * used), arc, 1.0);
    }
    let a = (135.0 + 270.0 * used).to_radians();
    let at = |angle: f64, len: f64| c + Vec2::new(angle.cos(), angle.sin()) * len * unit;
    let tip = |k: usize| at(a + k as f64 * FRAC_PI_2, if k.is_multiple_of(4) { 5.6 } else { 3.0 });
    let mut star = BezPath::new();
    star.move_to(tip(0));
    for k in 1..=4 {
        star.quad_to(at(a + (k as f64 - 0.5) * FRAC_PI_2, 0.9), tip(k));
    }
    star.close_path();
    fill(px, s, &star, needle, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(mark: Mark, level: Level) -> Badge {
        Badge { mark, level, tint: None }
    }

    fn lit(px: &[u8], size: u32, fg: [u8; 3]) -> Vec<(u32, u32)> {
        (0..size * size).filter(|i| px[(*i * 4) as usize..(*i * 4 + 3) as usize] == fg && px[(*i * 4 + 3) as usize] == 255).map(|i| (i % size, i / size)).collect()
    }

    #[test]
    fn digits_are_centred_whole_pixels_with_a_margin() {
        for size in [16, 20, 24, 32, 40] {
            for text in ["7", "42", "100"] {
                let px = draw(&plain(Mark::Number(text.into()), Level::Normal), size);
                assert_eq!(px.len(), (size * size * 4) as usize);
                let on = lit(&px, size, level_colors(Level::Normal).1);
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
    fn one_digit_is_as_tall_as_two() {
        for size in [16, 20, 24, 32, 40] {
            let height = |text: &str| {
                let on = lit(&draw(&plain(Mark::Number(text.into()), Level::Normal), size), size, level_colors(Level::Normal).1);
                on.iter().map(|p| p.1).max().unwrap() - on.iter().map(|p| p.1).min().unwrap()
            };
            assert_eq!(height("7"), height("28"), "at {size}");
        }
    }

    #[test]
    fn corners_are_transparent_and_the_middle_is_filled() {
        let px = draw(&plain(Mark::Number(String::new()), Level::High), 16);
        assert_eq!(px[3], 0);
        let mid = ((8 * 16 + 8) * 4) as usize;
        assert_eq!(&px[mid..mid + 4], &[0xDC, 0x26, 0x26, 255]);
    }

    #[test]
    fn each_logo_sits_in_the_middle_of_the_badge() {
        for (key, _, _) in LOGOS {
            for size in [16, 20, 24, 32, 40] {
                let px = draw(&plain(Mark::Logo(key), Level::Warn), size);
                let (bg, fg) = level_colors(Level::Warn);
                // pixels at least half covered by the logo
                let on: Vec<(u32, u32)> = (0..size * size)
                    .map(|i| (i, &px[(i * 4) as usize..(i * 4 + 4) as usize]))
                    .filter(|(_, p)| p[3] == 255 && p[0].abs_diff(fg[0]) < p[0].abs_diff(bg[0]))
                    .map(|(i, _)| (i % size, i / size))
                    .collect();
                assert!(!on.is_empty(), "{key} at {size}");
                let (minx, maxx) = (on.iter().map(|p| p.0).min().unwrap(), on.iter().map(|p| p.0).max().unwrap());
                let (miny, maxy) = (on.iter().map(|p| p.1).min().unwrap(), on.iter().map(|p| p.1).max().unwrap());
                assert!(minx >= 2 && miny >= 2 && maxx <= size - 3 && maxy <= size - 3, "{key} at {size}: {minx}-{maxx} {miny}-{maxy}");
                assert!((minx as i64 - (size - 1 - maxx) as i64).abs() <= 1 && (miny as i64 - (size - 1 - maxy) as i64).abs() <= 1, "{key} at {size}");
                assert!((maxx - minx).max(maxy - miny) + 1 >= size * 3 / 5, "{key} at {size} is too small");
                // the badge's corners stay as they were
                assert_eq!(px[3], 0);
            }
        }
    }

    #[test]
    fn the_logos_are_the_apps_own_marks() {
        let ui = include_str!("../../ui/src/components/BrandIcon.svelte");
        for (key, d, _) in LOGOS {
            assert!(ui.contains(&format!("{key}: '{d}'")), "{key}");
        }
    }

    #[test]
    fn the_badge_shows_the_chosen_mark_in_its_limits_color() {
        let mut s = Settings { onboarded: true, ..Default::default() };
        let l = vec![
            Current { provider: Provider::Anthropic, window: "five_hour".into(), used: 30.0, behind: false },
            Current { provider: Provider::OpenAI, window: "seven_day".into(), used: 96.0, behind: false },
        ];
        let shown = |s: &Settings, l: &[Current]| badge(s, &s.tray, l);
        assert_eq!(shown(&s, &l), Some(plain(Mark::Number("96".into()), Level::High)));
        s.tray.mark = "tool".into();
        assert_eq!(shown(&s, &l), Some(plain(Mark::Logo("openai"), Level::High)));
        s.tray.limit = "anthropic:five_hour".into();
        assert_eq!(shown(&s, &l), Some(plain(Mark::Logo("anthropic"), Level::Normal)));
        s.tray.mark = "opencode".into();
        assert_eq!(shown(&s, &l), Some(plain(Mark::Logo("opencode"), Level::Normal)));
        // a chosen logo stays without a reading; a number or the limit's tool needs one
        assert_eq!(shown(&s, &[]), Some(plain(Mark::Logo("opencode"), Level::Normal)));
        for m in ["number", "tool"] {
            s.tray.mark = m.into();
            assert_eq!(shown(&s, &[]), None, "{m}");
        }
        s.tray.show_percent = false;
        assert_eq!(shown(&s, &l), None);
    }

    #[test]
    fn the_square_takes_its_tools_color_until_the_limit_gets_close() {
        let mut s = Settings { onboarded: true, ..Default::default() };
        s.tray.tool_color = true;
        s.tray.limit = "anthropic:five_hour".into();
        let mut l = vec![Current { provider: Provider::Anthropic, window: "five_hour".into(), used: 30.0, behind: false }];
        let claude = [0xD9, 0x77, 0x57];
        for (m, mark) in [("number", Mark::Number("30".into())), ("tool", Mark::Logo("anthropic"))] {
            s.tray.mark = m.into();
            assert_eq!(badge(&s, &s.tray, &l), Some(Badge { mark, level: Level::Normal, tint: Some(claude) }), "{m}");
        }
        // a chosen logo takes its own tool's color, whichever limit colors it
        s.tray.mark = "opencode".into();
        assert_eq!(badge(&s, &s.tray, &l).unwrap().tint, Some([0x52, 0x51, 0x4E]));
        s.tray.mark = "anthropic".into();
        s.provider_colors.insert("anthropic".into(), "#F0E68C".into());
        let b = badge(&s, &s.tray, &l).unwrap();
        assert_eq!(colors(&b), ([0xF0, 0xE6, 0x8C], [0x11, 0x11, 0x11]), "a light color keeps a dark mark");
        let px = draw(&b, 16);
        let mid = ((8 * 16 + 1) * 4) as usize;
        assert_eq!(&px[mid..mid + 4], &[0xF0, 0xE6, 0x8C, 255]);
        s.provider_colors.clear();
        assert_eq!(colors(&badge(&s, &s.tray, &l).unwrap()), (claude, [0xFF, 0xFF, 0xFF]));
        l[0].used = 75.0;
        assert_eq!(colors(&badge(&s, &s.tray, &l).unwrap()), level_colors(Level::Warn), "the warning colors win");
        s.tray.tool_color = false;
        l[0].used = 30.0;
        assert_eq!(badge(&s, &s.tray, &l).unwrap().tint, None);
    }

    #[test]
    fn the_windows_tray_mark_takes_the_size_drawn_for_the_screen() {
        let side = |ico: &[u8], size: u32| {
            let png = ico_entry(ico, size).unwrap();
            u32::from_be_bytes(png[16..20].try_into().unwrap())
        };
        for ico in [&include_bytes!("../icons/tray-light-taskbar.ico")[..], &include_bytes!("../icons/tray-dark-taskbar.ico")[..]] {
            assert_eq!((side(ico, 16), side(ico, 20), side(ico, 28), side(ico, 32), side(ico, 64)), (16, 20, 32, 32, 48));
        }
        assert_eq!(ico_entry(b"\0\0\x01\0\x02\0", 16), None, "a damaged file draws nothing");
    }

    #[test]
    fn the_dial_fills_to_the_limit_and_its_needle_follows() {
        let mut s = Settings { onboarded: true, ..Default::default() };
        s.tray.mark = "dial".into();
        let l = vec![Current { provider: Provider::Anthropic, window: "five_hour".into(), used: 30.0, behind: false }];
        assert_eq!(badge(&s, &s.tray, &l), Some(plain(Mark::Dial(0.3), Level::Normal)));
        assert_eq!(badge(&s, &s.tray, &[]), None, "no reading keeps the app icon");
        s.limit_display = "remaining".into();
        assert_eq!(badge(&s, &s.tray, &l).unwrap().mark, Mark::Dial(0.7));

        let size = 32;
        let drawn = |used: f64, look: DialLook| {
            let mut px = vec![0u8; (size * size * 4) as usize];
            if look == DialLook::Square {
                square(&mut px, size as i64, NAVY);
            }
            dial(&mut px, size as i64, used, Level::Normal, look);
            px
        };
        let count = |px: &[u8], rgb: [u8; 3]| lit(px, size, rgb).len();
        let blue = arc_color(Level::Normal);
        assert!(count(&drawn(0.0, DialLook::Square), blue) == 0);
        assert!(count(&drawn(0.25, DialLook::Square), blue) < count(&drawn(0.75, DialLook::Square), blue));
        let solid = |px: &[u8]| px.chunks(4).filter(|p| p[3] == 255).count();
        assert!(solid(&drawn(0.25, DialLook::Template)) < solid(&drawn(0.75, DialLook::Template)), "the opaque arc grows");
        assert_eq!(drawn(0.5, DialLook::Template)[3], 0, "a template has no square");
        let needle_x = |used: f64| {
            let on = lit(&drawn(used, DialLook::Square), size, NEEDLE);
            on.iter().map(|p| p.0 as f64).sum::<f64>() / on.len() as f64
        };
        assert!(needle_x(0.0) + 1.0 < needle_x(1.0), "the needle turns from the start of the arc to its end");
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
        assert_eq!(tooltip(&l, &s, Some("0.3.0")), "AI Usage Tracker\nClaude · 5 sa %41 · haftalık ~%43\nCodex · 5 sa %7\nGüncelleme hazır: v0.3.0");
        s.language = "en".into();
        s.limit_display = "remaining".into();
        assert_eq!(tooltip(&l, &s, None), "AI Usage Tracker\nClaude · 5h 59% left · week ~57% left\nCodex · 5h 93% left");
        assert!(tooltip(&[], &s, None).ends_with("No current limit reading"));
    }
}
