//! User settings, stored as one JSON document in the tracker database.
//! Unknown/missing fields fall back to defaults so older/newer settings never break startup.

use serde::{Deserialize, Serialize};
use tracker_core::analytics::Threshold;
use tracker_core::discovery::{ExtraPaths, SourceId};
use tracker_core::store::Store;

const KEY: &str = "settings.v1";

/// One line/block the widget can show, in the user's order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WidgetItem {
    /// `primary` (big number), `cost`, `limit_five_hour`, `limit_seven_day`, `tools`,
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
    /// Background opacity 0.3–1.0.
    pub opacity: f64,
    /// Size preset kept for the context menu: "s" | "m" | "l" (maps to `scale`).
    pub size: String,
    /// Free zoom factor 0.6–2.0; the window follows the content size.
    pub scale: f64,
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// Corner the widget sticks to while it resizes ("bottom-right", …); empty once the user
    /// drags it somewhere else.
    pub anchor: String,
    pub auto_hide_fullscreen: bool,
    /// "horizontal" | "vertical" | "line"
    pub layout: String,
    pub items: Vec<WidgetItem>,
    /// Which providers to show; empty = all enabled ones.
    pub providers: Vec<String>,
    /// Range of the big number: "today" | "days7" | "month1"
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
    pub show_reset_time: bool,
    pub warn_at: f64,
    pub high_at: f64,
    pub always_on_top: bool,
    pub lock_position: bool,
    /// "open_dashboard" | "none"
    pub click_action: String,
    /// CSS family name of an installed font; empty = the app font (Inter).
    pub font_family: String,
    /// Size factor for labels and small values, 0.8–1.6 (independent of `scale`).
    pub text_scale: f64,
    /// Size factor of the big number, 0.6–2.0.
    pub number_scale: f64,
    /// Weight of the big number and limit values, 300–900.
    pub number_weight: u16,
    /// Fixed-width digits so values do not jitter as they change.
    pub tabular_nums: bool,
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
            primary_period: "today".into(),
            primary_metric: "tokens".into(),
            limit_style: "ring".into(),
            theme: "system".into(),
            accent: String::new(),
            corner_radius: 14.0,
            border: true,
            shadow: false,
            show_labels: true,
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
        }
    }
}

impl WidgetSettings {
    /// Fills in items added by newer versions and clamps values.
    pub fn normalize(&mut self) {
        for d in default_items() {
            if !self.items.iter().any(|i| i.kind == d.kind) {
                self.items.push(WidgetItem { enabled: false, ..d });
            }
        }
        self.items.retain(|i| default_items().iter().any(|d| d.kind == i.kind));
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
    }
}

/// Opt-in live capture switches (all off by default).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    pub codex_poll: bool,
    pub codex_poll_minutes: u64,
    /// Explicit path to `codex.exe`; empty = auto-detect.
    pub codex_path: String,
    pub statusline: bool,
    pub otel: bool,
    pub otel_port: u16,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            codex_poll: false,
            codex_poll_minutes: 5,
            codex_path: String::new(),
            statusline: false,
            otel: false,
            otel_port: tracker_core::capture::otlp::DEFAULT_PORT,
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
    pub enabled_sources: Vec<SourceId>,
    pub extra_paths: ExtraPaths,
    /// Plan ids from plans.json, per provider ("anthropic" / "openai").
    pub plans: std::collections::BTreeMap<String, String>,
    pub thresholds: Vec<Threshold>,
    /// Mask every project name in the UI and exports.
    pub hide_project_names: bool,
    /// Display currency; costs are always computed in USD.
    pub currency: String,
    /// Manual USD → `currency` rate (no network lookup).
    pub fx_rate: f64,
    pub widget: WidgetSettings,
    pub capture: CaptureSettings,
    pub autostart: bool,
    /// Optional network use: allow fetching a newer pricing/plan file. Off by default.
    pub allow_config_updates: bool,
    /// Visual: show cost or tokens first.
    pub primary_metric: String,
    /// Unpriced models whose warning the user dismissed; a model that becomes unpriced later
    /// warns again.
    pub dismissed_unpriced: Vec<String>,
    /// How limit percentages read: "used" (Claude's convention) or "remaining" (Codex's).
    pub limit_display: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            onboarded: false,
            language: "system".into(),
            theme: "system".into(),
            enabled_sources: SourceId::ALL.into_iter().filter(|s| s.supported()).collect(),
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
        }
    }
}

impl Settings {
    pub fn load(store: &Store) -> Settings {
        match store.setting(KEY) {
            Ok(Some(s)) => serde_json::from_str::<Settings>(&s)
                .map(|mut s| {
                    s.widget.normalize();
                    s
                })
                .unwrap_or_else(|e| {
                    log::warn!("settings unreadable, using defaults: {e}");
                    Settings::default()
                }),
            _ => Settings::default(),
        }
    }

    pub fn save(&self, store: &Store) -> Result<(), String> {
        let mut s = self.clone();
        s.widget.normalize();
        if !(s.fx_rate.is_finite() && s.fx_rate > 0.0) {
            s.fx_rate = 1.0;
        }
        store.set_setting(KEY, &serde_json::to_string(&s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}
