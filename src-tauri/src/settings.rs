//! Missing or unknown fields fall back to defaults so older or newer settings never break startup.

use serde::{Deserialize, Serialize};
use tracker_core::analytics::Threshold;
use tracker_core::discovery::{ExtraPaths, SourceId};
use tracker_core::store::Store;

const KEY: &str = "settings.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WidgetItem {
    /// `primary` (big number), `cost`, `limit_five_hour`, `limit_seven_day`, `opencode`, `tools`,
    /// `week_cost`, `month_cost`, `week_tokens`, `updated`
    pub kind: String,
    pub enabled: bool,
}

fn default_items() -> Vec<WidgetItem> {
    [
        ("primary", true),
        ("cost", true),
        ("limit_five_hour", true),
        ("limit_seven_day", true),
        ("opencode", true),
        ("tools", false),
        ("week_tokens", false),
        ("week_cost", false),
        ("month_cost", false),
        ("updated", false),
    ]
    .into_iter()
    .map(|(k, e)| WidgetItem { kind: k.into(), enabled: e })
    .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetSettings {
    pub visible: bool,
    pub opacity: f64,
    /// Context-menu preset "s" | "m" | "l"; the actual size is `scale`.
    pub size: String,
    pub scale: f64,
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Corner kept while resizing ("bottom-right", …); empty once the user drags the widget.
    pub anchor: String,
    pub auto_hide_fullscreen: bool,
    /// "horizontal" | "vertical" | "line"
    pub layout: String,
    pub items: Vec<WidgetItem>,
    /// Which providers to show; empty = all enabled ones.
    pub providers: Vec<String>,
    /// Order of the providers' limits, top or left first.
    pub provider_order: Vec<String>,
    /// Antigravity pool shown: "fullest" | "gemini" | "3p" | "all".
    pub antigravity_pool: String,
    /// Other Codex and Claude accounts shown beside each one's account in use, by digest.
    #[serde(alias = "codex_accounts")]
    pub accounts: Vec<String>,
    /// "today" | "days7" | "month1"
    pub primary_period: String,
    /// "tokens" | "cost"
    pub primary_metric: String,
    /// "ring" | "bar" | "text"
    pub limit_style: String,
    /// "system" | "light" | "dark"
    pub theme: String,
    /// `#rrggbb`, or empty for the system accent.
    pub accent: String,
    pub corner_radius: f64,
    pub border: bool,
    pub shadow: bool,
    pub show_labels: bool,
    /// Each tool's mark next to its limits.
    pub show_icons: bool,
    pub show_reset_time: bool,
    pub warn_at: f64,
    pub high_at: f64,
    pub always_on_top: bool,
    pub lock_position: bool,
    /// "open_dashboard" | "none"
    pub click_action: String,
    /// Empty = the app font (Inter).
    pub font_family: String,
    pub text_scale: f64,
    pub number_scale: f64,
    pub number_weight: u16,
    pub tabular_nums: bool,
    /// Empty = no shortcut.
    pub hotkey: String,
}

impl Default for WidgetSettings {
    fn default() -> Self {
        WidgetSettings {
            visible: true,
            opacity: 0.85,
            size: "m".into(),
            scale: 1.0,
            x: None,
            y: None,
            anchor: "bottom-right".into(),
            auto_hide_fullscreen: true,
            layout: "horizontal".into(),
            items: default_items(),
            providers: Vec::new(),
            provider_order: PROVIDER_ORDER.iter().map(|p| p.to_string()).collect(),
            antigravity_pool: "fullest".into(),
            accounts: Vec::new(),
            primary_period: "today".into(),
            primary_metric: "tokens".into(),
            limit_style: "ring".into(),
            theme: "system".into(),
            accent: String::new(),
            corner_radius: 14.0,
            border: true,
            shadow: false,
            show_labels: true,
            show_icons: false,
            show_reset_time: false,
            warn_at: 70.0,
            high_at: 90.0,
            always_on_top: true,
            lock_position: false,
            click_action: "open_dashboard".into(),
            font_family: String::new(),
            text_scale: 1.0,
            number_scale: 1.0,
            number_weight: 700,
            tabular_nums: true,
            hotkey: DEFAULT_HOTKEY.into(),
        }
    }
}

impl WidgetSettings {
    pub fn normalize(&mut self) {
        // an item added in a later version takes its default place and state
        let defaults = default_items();
        for (n, d) in defaults.iter().enumerate() {
            if !self.items.iter().any(|i| i.kind == d.kind) {
                let at = defaults[..n].iter().rev().find_map(|p| self.items.iter().position(|i| i.kind == p.kind)).map_or(0, |i| i + 1);
                self.items.insert(at, d.clone());
            }
        }
        self.items.retain(|i| default_items().iter().any(|d| d.kind == i.kind));
        // each item once: the widget keys its rows by kind
        let mut seen = Vec::new();
        self.items.retain(|i| !seen.contains(&i.kind) && {
            seen.push(i.kind.clone());
            true
        });
        // the accent is placed into CSS, so only a plain hex color is kept
        if !self.accent.is_empty() && !is_hex_color(&self.accent) {
            self.accent.clear();
        }
        let mut order: Vec<String> = Vec::new();
        for p in self.provider_order.iter().map(String::as_str).chain(PROVIDER_ORDER) {
            if PROVIDER_ORDER.contains(&p) && !order.iter().any(|o| o == p) {
                order.push(p.to_owned());
            }
        }
        self.provider_order = order;
        if !["fullest", "gemini", "3p", "all"].contains(&self.antigravity_pool.as_str()) {
            self.antigravity_pool = "fullest".into();
        }
        known_once(&mut self.accounts, MAX_SHOWN_ACCOUNTS, |a| is_digest(a), |a, b| a == b);
        self.opacity = self.opacity.clamp(0.3, 1.0);
        self.scale = if self.scale.is_finite() { self.scale.clamp(0.6, 2.0) } else { 1.0 };
        self.corner_radius = self.corner_radius.clamp(0.0, 28.0);
        self.warn_at = self.warn_at.clamp(1.0, 100.0);
        self.high_at = self.high_at.clamp(self.warn_at, 100.0);
        let factor = |v: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { 1.0 };
        self.text_scale = factor(self.text_scale, 0.8, 1.6);
        self.number_scale = factor(self.number_scale, 0.6, 2.0);
        self.number_weight = self.number_weight.clamp(300, 900) / 100 * 100;
        // the name is placed inside a CSS string; keep it a plain family name
        self.font_family = self
            .font_family
            .chars()
            .filter(|c| !matches!(c, '"' | '\'' | '\\' | ';' | '{' | '}' | '<' | '>'))
            .take(80)
            .collect::<String>()
            .trim()
            .to_string();
        self.hotkey = self.hotkey.trim().chars().take(64).collect();
    }
}

/// The widget's order before it could be changed.
const PROVIDER_ORDER: [&str; 3] = ["anthropic", "google", "openai"];

const MAX_SHOWN_ACCOUNTS: usize = 8;

/// A Codex or Claude account's id: 16 hex digits of its digest.
fn is_digest(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// An account is kept only on a Codex or Claude limit; elsewhere it is cleared.
fn keep_account(account: &mut String, provider: &str, limit_kind: bool) {
    if !(matches!(provider, "openai" | "anthropic") && limit_kind && is_digest(account)) {
        account.clear();
    }
}

/// Four keys, so it avoids combinations other programs or AltGr (Ctrl+Alt on many layouts) need.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Shift+W";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TraySettings {
    /// The chosen limit's percent drawn into the icon.
    pub show_percent: bool,
    /// "auto" (the fullest current limit) or "<provider>:<window>", e.g. "anthropic:five_hour".
    pub limit: String,
    /// Text next to the icon (macOS and Linux; Windows has no room for it).
    pub title: bool,
    /// What the text shows, in order.
    pub items: Vec<TrayItem>,
    /// "name" ("Claude 5h"), "short" ("Cl 5h") or "none".
    pub label: String,
    /// "app" (the app's used/remaining choice), "used" or "remaining".
    pub value: String,
    /// "none", "countdown" ("2h 14m") or "time" ("18:40").
    pub reset: String,
    /// "percent", "bar" ("▰▰▱▱▱"), "bar_percent" or "dot" ("◑").
    pub style: String,
    pub separator: String,
    pub prefix: String,
    pub suffix: String,
    /// "⚠︎" before a value past the warning threshold.
    pub warn_mark: bool,
    /// Text only, without the icon (macOS).
    pub hide_icon: bool,
}

/// One piece of the text next to the icon. `kind`: "limit", "forecast" (when a limit fills at
/// the current pace), "tokens", "cost", "sessions", "cache" (hit rate) or "scan" (last scan).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct TrayItem {
    pub kind: String,
    /// For usage kinds: empty for every tool, `opencode` for OpenCode alone.
    pub provider: String,
    /// "five_hour" or "seven_day".
    pub window: String,
    /// Antigravity's pool, "gemini" or "3p"; empty elsewhere.
    pub pool: String,
    /// "today", "days7" or "month1" for usage kinds.
    pub period: String,
    /// A Codex or Claude limit of this account (its digest); empty for the account in use.
    pub account: String,
}

impl TrayItem {
    /// Its place in the catalog, which lists the account in use only.
    pub fn key(&self) -> (&str, &str, &str, &str, &str) {
        (&self.kind, &self.provider, &self.window, &self.pool, &self.period)
    }
}

const POOLS: [(&str, &str); 4] = [("anthropic", ""), ("openai", ""), ("google", "gemini"), ("google", "3p")];
const WINDOWS: [&str; 2] = ["five_hour", "seven_day"];
const PERIODS: [&str; 3] = ["today", "days7", "month1"];

/// Every item the text next to the icon can show.
pub fn tray_catalog() -> Vec<TrayItem> {
    let mut out = Vec::new();
    for kind in ["limit", "forecast"] {
        for (provider, pool) in POOLS {
            for window in WINDOWS {
                out.push(TrayItem { kind: kind.into(), provider: provider.into(), window: window.into(), pool: pool.into(), ..TrayItem::default() });
            }
        }
    }
    for provider in ["", "opencode"] {
        for kind in ["tokens", "cost", "sessions", "cache"] {
            for period in PERIODS {
                out.push(TrayItem { kind: kind.into(), provider: provider.into(), period: period.into(), ..TrayItem::default() });
            }
        }
    }
    out.push(TrayItem { kind: "scan".into(), ..TrayItem::default() });
    out
}

pub const MAX_TRAY_ITEMS: usize = 12;

impl Default for TraySettings {
    fn default() -> Self {
        let limit = |provider: &str, window: &str| TrayItem { kind: "limit".into(), provider: provider.into(), window: window.into(), ..TrayItem::default() };
        TraySettings {
            show_percent: true,
            limit: "auto".into(),
            title: false,
            items: vec![limit("anthropic", "five_hour"), limit("anthropic", "seven_day"), limit("openai", "five_hour"), limit("openai", "seven_day")],
            label: "short".into(),
            value: "app".into(),
            reset: "none".into(),
            style: "percent".into(),
            separator: " · ".into(),
            prefix: String::new(),
            suffix: String::new(),
            warn_mark: true,
            hide_icon: false,
        }
    }
}

/// One line of plain text, at most `max` characters.
fn short_text(v: &mut String, max: usize) {
    *v = v.chars().filter(|c| !c.is_control()).take(max).collect();
}

/// Keeps the known entries of `list` once each, in order, at most `max`.
fn known_once<T>(list: &mut Vec<T>, max: usize, known: impl Fn(&T) -> bool, same: impl Fn(&T, &T) -> bool) {
    let mut kept: Vec<T> = Vec::new();
    for x in list.drain(..) {
        if kept.len() < max && known(&x) && !kept.iter().any(|k| same(k, &x)) {
            kept.push(x);
        }
    }
    *list = kept;
}

impl TraySettings {
    pub fn normalize(&mut self) {
        let catalog = tray_catalog();
        for i in &mut self.items {
            keep_account(&mut i.account, &i.provider, matches!(i.kind.as_str(), "limit" | "forecast"));
        }
        known_once(&mut self.items, MAX_TRAY_ITEMS, |i| catalog.iter().any(|k| k.key() == i.key()), |a, b| a == b);
        let pick = |v: &mut String, allowed: &[&str], fallback: &str| {
            if !allowed.contains(&v.as_str()) {
                *v = fallback.into();
            }
        };
        pick(&mut self.label, &["name", "short", "none"], "short");
        pick(&mut self.value, &["app", "used", "remaining"], "app");
        pick(&mut self.reset, &["none", "countdown", "time"], "none");
        pick(&mut self.style, &["percent", "bar", "bar_percent", "dot"], "percent");
        short_text(&mut self.separator, 6);
        if self.separator.is_empty() {
            self.separator = " · ".into();
        }
        short_text(&mut self.prefix, 12);
        short_text(&mut self.suffix, 12);
    }
}

/// The panel of tiles the tray icon opens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PanelSettings {
    pub modules: Vec<PanelModule>,
    /// "app" (the app's used/remaining choice), "used" or "remaining".
    pub value: String,
}

/// `kind`: "limit" (one window), "provider" (both windows of a pool), "usage" (a period, of every
/// tool or with `provider` "opencode" of OpenCode alone),
/// "forecast", "scan" or "action" (`action`: "open", "widget", "rescan", "settings").
/// `size`: "s" 1×1, "w" 2×1, "m" 2×2 or "l" 4×1 cells. `style`: "ring" or "bar" for limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct PanelModule {
    pub kind: String,
    pub provider: String,
    pub pool: String,
    pub window: String,
    pub period: String,
    pub action: String,
    pub size: String,
    pub style: String,
    /// A Codex or Claude limit of this account (its digest); empty for the account in use.
    pub account: String,
}

impl PanelModule {
    /// Its place in the catalog, which lists the account in use only.
    fn key(&self) -> (&str, &str, &str, &str, &str, &str) {
        (&self.kind, &self.provider, &self.pool, &self.window, &self.period, &self.action)
    }

    /// The sizes a kind can take, the first one being its default.
    pub fn sizes(kind: &str) -> &'static [&'static str] {
        match kind {
            "limit" => &["w", "s", "m"],
            "provider" => &["m", "w", "l"],
            "usage" => &["w", "s", "m"],
            "forecast" => &["w", "l"],
            "scan" => &["w"],
            "action" => &["s", "w"],
            _ => &[],
        }
    }
}

pub const MAX_PANEL_MODULES: usize = 16;

/// Every module the panel offers, at its default size.
pub fn panel_catalog() -> Vec<PanelModule> {
    let m = |kind: &str| PanelModule { kind: kind.into(), size: PanelModule::sizes(kind)[0].into(), ..PanelModule::default() };
    let mut out = Vec::new();
    for (provider, pool) in POOLS {
        out.push(PanelModule { provider: provider.into(), pool: pool.into(), style: "ring".into(), ..m("provider") });
    }
    for (provider, pool) in POOLS {
        for window in WINDOWS {
            out.push(PanelModule { provider: provider.into(), pool: pool.into(), window: window.into(), style: "ring".into(), ..m("limit") });
        }
    }
    for (provider, pool) in POOLS {
        for window in WINDOWS {
            out.push(PanelModule { provider: provider.into(), pool: pool.into(), window: window.into(), ..m("forecast") });
        }
    }
    for provider in ["", "opencode"] {
        for period in PERIODS {
            out.push(PanelModule { provider: provider.into(), period: period.into(), ..m("usage") });
        }
    }
    out.push(m("scan"));
    for action in ["open", "widget", "rescan", "settings"] {
        out.push(PanelModule { action: action.into(), ..m("action") });
    }
    out
}

impl Default for PanelSettings {
    fn default() -> Self {
        let find = |f: &dyn Fn(&PanelModule) -> bool| panel_catalog().into_iter().find(|x| f(x)).expect("in the catalog");
        PanelSettings {
            value: "app".into(),
            modules: vec![
                find(&|x| x.kind == "provider" && x.provider == "anthropic"),
                find(&|x| x.kind == "provider" && x.provider == "openai"),
                find(&|x| x.kind == "usage" && x.period == "today"),
                find(&|x| x.kind == "forecast" && x.provider == "anthropic" && x.window == "five_hour"),
                find(&|x| x.action == "open"),
                find(&|x| x.action == "widget"),
                find(&|x| x.action == "rescan"),
                find(&|x| x.action == "settings"),
            ],
        }
    }
}

impl PanelSettings {
    pub fn normalize(&mut self) {
        if !["app", "used", "remaining"].contains(&self.value.as_str()) {
            self.value = "app".into();
        }
        let catalog = panel_catalog();
        for m in &mut self.modules {
            keep_account(&mut m.account, &m.provider, matches!(m.kind.as_str(), "limit" | "provider" | "forecast"));
        }
        known_once(&mut self.modules, MAX_PANEL_MODULES, |m| catalog.iter().any(|k| k.key() == m.key()), |a, b| a.key() == b.key() && a.account == b.account);
        for m in &mut self.modules {
            let sizes = PanelModule::sizes(&m.kind);
            if !sizes.contains(&m.size.as_str()) {
                m.size = sizes[0].into();
            }
            let styled = matches!(m.kind.as_str(), "limit" | "provider");
            if !styled {
                m.style.clear();
            } else if m.style != "bar" {
                m.style = "ring".into();
            }
        }
    }
}

/// Limit reads default on (they change no files); methods that edit Claude Code's settings
/// file stay off until the user turns them on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    pub codex_poll: bool,
    pub codex_poll_minutes: u64,
    /// Empty = auto-detect.
    pub codex_path: String,
    pub statusline: bool,
    pub otel: bool,
    pub otel_port: u16,
    /// Reads plan limits through Claude Code's `get_usage` request.
    pub claude_poll: bool,
    pub claude_poll_minutes: u64,
    /// Empty = auto-detect.
    pub claude_path: String,
    /// Reads Antigravity's limits through the `agy` CLI's `/usage` command.
    pub antigravity_poll: bool,
    pub antigravity_poll_minutes: u64,
    /// Empty = auto-detect.
    pub antigravity_path: String,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            codex_poll: true,
            codex_poll_minutes: 5,
            codex_path: String::new(),
            statusline: false,
            otel: false,
            otel_port: tracker_core::capture::otlp::DEFAULT_PORT,
            claude_poll: true,
            claude_poll_minutes: 5,
            claude_path: String::new(),
            antigravity_poll: true,
            antigravity_poll_minutes: 5,
            antigravity_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub onboarded: bool,
    /// "system" | "tr" | "en"
    pub language: String,
    /// "system" | "light" | "dark"
    pub theme: String,
    #[serde(deserialize_with = "known")]
    pub enabled_sources: Vec<SourceId>,
    /// Sources this install has offered; one added by a later version starts enabled.
    #[serde(default = "sources_before_antigravity", deserialize_with = "known")]
    pub known_sources: Vec<SourceId>,
    pub extra_paths: ExtraPaths,
    /// Provider ("anthropic" / "openai") → plan id from plans.json.
    pub plans: std::collections::BTreeMap<String, String>,
    #[serde(deserialize_with = "known")]
    pub thresholds: Vec<Threshold>,
    pub hide_project_names: bool,
    /// Display currency; costs are always computed in USD.
    pub currency: String,
    /// Manual USD → `currency` rate (no network lookup).
    pub fx_rate: f64,
    pub widget: WidgetSettings,
    pub capture: CaptureSettings,
    pub autostart: bool,
    /// Opt-in network use: fetch a newer pricing/plan file.
    pub allow_config_updates: bool,
    /// "tokens" | "cost"
    pub primary_metric: String,
    /// A model that becomes unpriced again later warns again.
    pub dismissed_unpriced: Vec<String>,
    /// How limit percentages read: "used" (Claude's convention) or "remaining" (Codex's).
    pub limit_display: String,
    /// Monthly USD price per provider, when it differs from the list price or the plan has none.
    pub plan_prices: std::collections::BTreeMap<String, f64>,
    pub weekly_report_auto: bool,
    /// Empty = Documents\AI Usage Tracker.
    pub weekly_report_dir: String,
    pub tray: TraySettings,
    pub panel: PanelSettings,
    /// Only the version file is downloaded; nothing is sent. No effect until the build names a
    /// release location.
    pub update_check: bool,
    /// Provider → `#rrggbb` for its limit bars and mark; a missing one uses the accent color.
    pub provider_colors: std::collections::BTreeMap<String, String>,
    /// Marks take the tool's color too; off keeps them in the text color.
    pub tint_icons: bool,
    /// Each tool's mark next to its name in the app; the widget has its own switch.
    pub show_icons: bool,
    /// Codex or Claude account digest → the name the user gave it; an unnamed account is numbered.
    pub account_names: std::collections::BTreeMap<String, String>,
    /// Account digest → the plan chosen for it, while its provider has several accounts. An account
    /// without one takes the plan its readings name, else the provider's (`plans`).
    pub account_plans: std::collections::BTreeMap<String, String>,
    /// Account digest → monthly USD price, when it differs from the list price or the plan has none.
    pub account_prices: std::collections::BTreeMap<String, f64>,
}

/// Long enough for "Work (team)", short enough for a limit card's header.
const MAX_ACCOUNT_NAME: usize = 32;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            onboarded: false,
            language: "system".into(),
            theme: "system".into(),
            enabled_sources: SourceId::ALL.into_iter().filter(|s| s.supported()).collect(),
            known_sources: SourceId::ALL.to_vec(),
            extra_paths: ExtraPaths::default(),
            plans: Default::default(),
            thresholds: Vec::new(),
            hide_project_names: false,
            currency: "USD".into(),
            fx_rate: 1.0,
            widget: WidgetSettings::default(),
            capture: CaptureSettings::default(),
            autostart: false,
            allow_config_updates: false,
            primary_metric: "tokens".into(),
            dismissed_unpriced: Vec::new(),
            limit_display: "used".into(),
            plan_prices: Default::default(),
            weekly_report_auto: false,
            weekly_report_dir: String::new(),
            tray: TraySettings::default(),
            panel: PanelSettings::default(),
            update_check: true,
            provider_colors: Default::default(),
            tint_icons: false,
            show_icons: true,
            account_names: Default::default(),
            account_plans: Default::default(),
            account_prices: Default::default(),
        }
    }
}

/// `#rrggbb` only.
fn is_hex_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Entries this version does not know (a source or provider from a newer version, after a
/// downgrade) are dropped; otherwise one of them would make every setting fall back to its default.
fn known<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Vec::<serde_json::Value>::deserialize(d)?.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

fn sources_before_antigravity() -> Vec<SourceId> {
    vec![SourceId::ClaudeCode, SourceId::Cowork, SourceId::ClaudeDesktop, SourceId::Codex, SourceId::ChatgptDesktop]
}

impl Settings {
    pub fn load(store: &Store) -> Settings {
        match store.setting(KEY) {
            Ok(Some(s)) => serde_json::from_str::<Settings>(&s)
                .map(|mut s| {
                    s.widget.normalize();
                    s.tray.normalize();
                    s.panel.normalize();
                    s.adopt_new_sources();
                    s
                })
                .unwrap_or_else(|e| {
                    log::warn!("settings unreadable, using defaults: {e}");
                    Settings::default()
                }),
            _ => Settings::default(),
        }
    }

    /// A source the user switched off stays off; one they were never offered is switched on.
    fn adopt_new_sources(&mut self) {
        for id in SourceId::ALL {
            if !self.known_sources.contains(&id) {
                if id.supported() && !self.enabled_sources.contains(&id) {
                    self.enabled_sources.push(id);
                }
                self.known_sources.push(id);
            }
        }
    }

    pub fn normalize(&mut self) {
        self.widget.normalize();
        self.tray.normalize();
        self.panel.normalize();
        if !(self.fx_rate.is_finite() && self.fx_rate > 0.0) {
            self.fx_rate = 1.0;
        }
        // the value is placed into CSS, so only a plain hex color is kept
        self.provider_colors.retain(|p, c| ["anthropic", "openai", "google"].contains(&p.as_str()) && is_hex_color(c));
        self.account_names.retain(|id, _| is_digest(id));
        for name in self.account_names.values_mut() {
            *name = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_ACCOUNT_NAME).collect();
        }
        self.account_names.retain(|_, n| !n.is_empty());
        self.account_plans.retain(|id, plan| is_digest(id) && !plan.is_empty());
        self.account_prices.retain(|id, usd| is_digest(id) && usd.is_finite() && *usd > 0.0);
    }

    /// Normalizes in place first, so what callers keep and publish is exactly what was stored.
    pub fn save(&mut self, store: &Store) -> Result<(), String> {
        self.normalize();
        store.set_setting(KEY, &serde_json::to_string(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_reads_are_on_and_file_editing_methods_off_by_default() {
        let c = Settings::default().capture;
        assert!(c.claude_poll && c.codex_poll && c.antigravity_poll);
        assert!(!c.statusline && !c.otel);
    }

    #[test]
    fn settings_saved_by_an_older_version_get_the_new_defaults() {
        // written before the Claude limit read existed
        let s: Settings = serde_json::from_str(r#"{"onboarded":true,"capture":{"codex_poll":false}}"#).unwrap();
        assert!(s.capture.claude_poll && s.capture.antigravity_poll);
        assert!(!s.capture.codex_poll, "an explicit choice is kept");
        assert_eq!(s.limit_display, "used");
        assert!(s.tray.show_percent && s.update_check);
        assert_eq!(s.widget.hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn account_plans_and_prices_are_only_kept_for_account_digests() {
        let mut s: Settings = serde_json::from_str(
            r#"{"account_plans":{"3f9c1a7e52b0d846":"max5x","b71e04c9a3d25f18":"","not-a-digest":"pro"},"account_prices":{"3f9c1a7e52b0d846":120,"b71e04c9a3d25f18":0,"0123456789abcdef":-5}}"#,
        )
        .unwrap();
        s.normalize();
        assert_eq!(s.account_plans.into_iter().collect::<Vec<_>>(), [("3f9c1a7e52b0d846".to_owned(), "max5x".to_owned())]);
        assert_eq!(s.account_prices.into_iter().collect::<Vec<_>>(), [("3f9c1a7e52b0d846".to_owned(), 120.0)]);
    }

    #[test]
    fn account_names_are_tidied_and_only_kept_for_account_digests() {
        let mut s: Settings = serde_json::from_str(
            r#"{"account_names":{"3f9c1a7e52b0d846":"  İş   hesabı ","b71e04c9a3d25f18":"   ","not-a-digest":"x","0123456789abcdef":"a very long name that goes on and on and on"}}"#,
        )
        .unwrap();
        s.normalize();
        assert_eq!(s.account_names.get("3f9c1a7e52b0d846").map(String::as_str), Some("İş hesabı"));
        assert_eq!(s.account_names.get("0123456789abcdef").map(|n| n.chars().count()), Some(MAX_ACCOUNT_NAME));
        assert_eq!(s.account_names.len(), 2);
    }

    #[test]
    fn another_account_is_kept_only_on_codex_and_claude_limits() {
        let mut s: Settings = serde_json::from_str(
            r#"{
                "widget": {"codex_accounts": ["3f9c1a7e52b0d846", "x", "3f9c1a7e52b0d846", "b71e04c9a3d25f18"]},
                "tray": {"items": [
                    {"kind": "limit", "provider": "openai", "window": "five_hour"},
                    {"kind": "limit", "provider": "openai", "window": "five_hour", "account": "3f9c1a7e52b0d846"},
                    {"kind": "forecast", "provider": "openai", "window": "seven_day", "account": "not-a-digest"},
                    {"kind": "limit", "provider": "anthropic", "window": "five_hour", "account": "3f9c1a7e52b0d846"},
                    {"kind": "tokens", "period": "today", "account": "3f9c1a7e52b0d846"}
                ]},
                "panel": {"modules": [
                    {"kind": "provider", "provider": "openai", "account": "3f9c1a7e52b0d846", "size": "m"},
                    {"kind": "provider", "provider": "openai", "account": "3f9c1a7e52b0d846", "size": "w"},
                    {"kind": "usage", "period": "today", "account": "3f9c1a7e52b0d846"}
                ]}
            }"#,
        )
        .unwrap();
        s.normalize();
        assert_eq!(s.widget.accounts, ["3f9c1a7e52b0d846", "b71e04c9a3d25f18"], "the 0.8 pre-release name is read too");
        let accounts: Vec<&str> = s.tray.items.iter().map(|i| i.account.as_str()).collect();
        assert_eq!(accounts, ["", "3f9c1a7e52b0d846", "", "3f9c1a7e52b0d846", ""]);
        let modules: Vec<(&str, &str)> = s.panel.modules.iter().map(|m| (m.kind.as_str(), m.account.as_str())).collect();
        assert_eq!(modules, [("provider", "3f9c1a7e52b0d846"), ("usage", "")], "the same account's module once");
    }

    #[test]
    fn opencode_switched_off_in_0_6_0_stays_off() {
        let store = Store::open_in_memory().unwrap();
        let known = r#"["claude_code","cowork","claude_desktop","codex","antigravity","open_code","chatgpt_desktop"]"#;
        store.set_setting(KEY, &format!(r#"{{"onboarded":true,"enabled_sources":["claude_code","codex"],"known_sources":{known}}}"#)).unwrap();
        let s = Settings::load(&store);
        assert_eq!(s.enabled_sources, [SourceId::ClaudeCode, SourceId::Codex]);
        assert!(s.known_sources.contains(&SourceId::OpenCode));
        store.set_setting(KEY, r#"{"onboarded":true,"enabled_sources":["open_code"],"known_sources":[]}"#).unwrap();
        assert!(Settings::load(&store).enabled_sources.contains(&SourceId::OpenCode));
    }

    #[test]
    fn menu_bar_settings_from_older_versions_keep_their_icon_and_start_without_text() {
        let s: Settings = serde_json::from_str(r#"{"tray":{"show_percent":false,"limit":"openai:five_hour"}}"#).unwrap();
        assert_eq!((s.tray.show_percent, s.tray.limit.as_str(), s.tray.title), (false, "openai:five_hour", false));
        assert_eq!(s.tray.items.len(), 4);
        assert_eq!(s.panel, PanelSettings::default());

        let mut t: TraySettings = serde_json::from_str(
            r#"{"items":[{"kind":"limit","provider":"openai","window":"seven_day"},
                         {"kind":"limit","provider":"openai","window":"seven_day"},
                         {"kind":"limit","provider":"nobody","window":"five_hour"},
                         {"kind":"tokens","period":"days7"}],
                "label":"huge","value":"x","reset":"y","style":"z","separator":"","prefix":"a\tb"}"#,
        )
        .unwrap();
        t.normalize();
        assert_eq!(t.items.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(), ["limit", "tokens"], "duplicates and unknown items are dropped, the order is kept");
        assert_eq!((t.label.as_str(), t.value.as_str(), t.reset.as_str(), t.style.as_str()), ("short", "app", "none", "percent"));
        assert_eq!((t.separator.as_str(), t.prefix.as_str()), (" · ", "ab"));
        t.items = tray_catalog();
        t.normalize();
        assert_eq!(t.items.len(), MAX_TRAY_ITEMS);
    }

    #[test]
    fn panel_modules_keep_the_users_order_and_only_sizes_their_kind_allows() {
        let mut p: PanelSettings = serde_json::from_str(
            r#"{"modules":[{"kind":"usage","period":"days7","size":"l","style":"bar"},
                           {"kind":"provider","provider":"anthropic","size":"l","style":"bar"},
                           {"kind":"provider","provider":"anthropic","size":"s"},
                           {"kind":"action","action":"format_disk","size":"s"},
                           {"kind":"limit","provider":"google","pool":"gemini","window":"seven_day","size":"m","style":"neon"}]}"#,
        )
        .unwrap();
        p.normalize();
        let got: Vec<_> = p.modules.iter().map(|m| (m.kind.as_str(), m.size.as_str(), m.style.as_str())).collect();
        assert_eq!(got, [("usage", "w", ""), ("provider", "l", "bar"), ("limit", "m", "ring")]);

        assert_eq!(p.value, "app", "a panel saved without the choice follows the app");
        let mut p = PanelSettings { modules: panel_catalog(), value: "left".into() };
        p.normalize();
        assert_eq!((p.modules.len(), p.value.as_str()), (MAX_PANEL_MODULES, "app"));
        assert!(panel_catalog().iter().all(|m| PanelModule::sizes(&m.kind).first() == Some(&m.size.as_str())));
        let mut empty = PanelSettings { modules: vec![], value: "remaining".into() };
        empty.normalize();
        assert!(empty.modules.is_empty(), "an emptied panel stays empty");
    }

    #[test]
    fn a_source_added_by_an_update_starts_on_without_reviving_switched_off_ones() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(KEY, r#"{"onboarded":true,"enabled_sources":["claude_code","cowork"]}"#).unwrap();
        let mut s = Settings::load(&store);
        assert_eq!(s.enabled_sources, [SourceId::ClaudeCode, SourceId::Cowork, SourceId::Antigravity, SourceId::OpenCode]);
        assert!(SourceId::ALL.iter().all(|x| s.known_sources.contains(x)) && s.known_sources.len() == SourceId::ALL.len());

        s.enabled_sources.retain(|x| *x != SourceId::Antigravity);
        s.save(&store).unwrap();
        assert_eq!(Settings::load(&store).enabled_sources, [SourceId::ClaudeCode, SourceId::Cowork, SourceId::OpenCode], "an explicit choice is kept");
        assert!(Settings::default().enabled_sources.contains(&SourceId::Antigravity));
    }

    #[test]
    fn saving_normalizes_the_object_that_is_kept() {
        let store = Store::open_in_memory().unwrap();
        let mut s = Settings { fx_rate: f64::NAN, ..Settings::default() };
        s.widget.opacity = 5.0;
        s.save(&store).unwrap();
        assert_eq!(s.fx_rate, 1.0);
        assert_eq!(s.widget.opacity, 1.0);
        let stored = Settings::load(&store);
        assert_eq!(serde_json::to_value(&stored).unwrap(), serde_json::to_value(&s).unwrap());
    }

    #[test]
    fn only_plain_hex_colors_for_known_tools_are_kept() {
        let mut s: Settings = serde_json::from_str(
            r##"{"provider_colors":{"anthropic":"#D97757","openai":"red;}body{","google":"#12345","grok":"#000000"}}"##,
        )
        .unwrap();
        s.normalize();
        assert_eq!(s.provider_colors.into_iter().collect::<Vec<_>>(), [("anthropic".to_owned(), "#D97757".to_owned())]);
        let d = Settings::default();
        assert!(d.provider_colors.is_empty() && !d.tint_icons && !d.widget.show_icons);
    }

    #[test]
    fn entries_from_a_newer_version_are_dropped_without_losing_the_rest() {
        let s: Settings = serde_json::from_str(
            r#"{"onboarded":true,"plans":{"anthropic":"max5x"},"enabled_sources":["claude_code","grok_cli"],
                "known_sources":["claude_code","grok_cli"],"thresholds":[{"provider":"xai","window":"five_hour"},{"provider":"openai","window":"seven_day"}]}"#,
        )
        .unwrap();
        assert!(s.onboarded, "the user's settings are kept");
        assert_eq!(s.plans.get("anthropic").map(String::as_str), Some("max5x"));
        assert_eq!(s.enabled_sources, [SourceId::ClaudeCode]);
        assert_eq!(s.known_sources, [SourceId::ClaudeCode]);
        assert_eq!(s.thresholds.len(), 1);
    }

    #[test]
    fn the_opencode_widget_item_joins_after_the_limits() {
        let store = Store::open_in_memory().unwrap();
        let saved = r#"{"widget":{"items":[{"kind":"limit_seven_day","enabled":false},{"kind":"primary","enabled":true},{"kind":"cost","enabled":false},{"kind":"limit_five_hour","enabled":true},{"kind":"tools","enabled":true},{"kind":"week_tokens","enabled":false},{"kind":"week_cost","enabled":false},{"kind":"month_cost","enabled":false},{"kind":"updated","enabled":false}]}}"#;
        store.set_setting(KEY, saved).unwrap();
        let s = Settings::load(&store);
        let kinds: Vec<_> = s.widget.items.iter().map(|i| (i.kind.as_str(), i.enabled)).collect();
        assert_eq!(&kinds[..3], [("limit_seven_day", false), ("opencode", true), ("primary", true)], "the user's order is kept");
        assert_eq!(kinds.len(), default_items().len());
    }

    #[test]
    fn widget_items_appear_once_and_the_accent_is_a_plain_color() {
        let mut s: Settings = serde_json::from_str(r#"{"widget":{"items":[{"kind":"primary","enabled":true},{"kind":"primary","enabled":false}],"accent":"red;zoom:9"}}"#).unwrap();
        s.normalize();
        assert_eq!(s.widget.items.iter().filter(|i| i.kind == "primary").count(), 1);
        assert!(s.widget.items[0].enabled, "the first copy wins");
        assert_eq!(s.widget.accent, "");
        s.widget.accent = "#2a78d6".into();
        s.normalize();
        assert_eq!(s.widget.accent, "#2a78d6");
    }

    #[test]
    fn the_widget_keeps_a_complete_provider_order_and_a_known_pool() {
        let mut s: Settings = serde_json::from_str(r#"{"widget":{"provider_order":["openai","grok","openai"],"antigravity_pool":"x"}}"#).unwrap();
        s.normalize();
        assert_eq!(s.widget.provider_order, ["openai", "anthropic", "google"]);
        assert_eq!(s.widget.antigravity_pool, "fullest");
        let old: Settings = serde_json::from_str(r#"{"onboarded":true}"#).unwrap();
        assert!(old.show_icons, "icons stay on for settings saved before the switch");
        assert_eq!(old.widget.provider_order, ["anthropic", "google", "openai"]);
    }

    #[test]
    fn a_cleared_hotkey_stays_cleared() {
        let s: Settings = serde_json::from_str(r#"{"widget":{"hotkey":""}}"#).unwrap();
        assert_eq!(s.widget.hotkey, "");
    }
}
