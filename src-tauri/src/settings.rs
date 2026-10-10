//! Missing or unknown fields fall back to defaults so older or newer settings never break startup.

use serde::{Deserialize, Serialize};
use tracker_core::analytics::Threshold;
use tracker_core::discovery::{ExtraPaths, SourceId};
use tracker_core::store::Store;

const KEY: &str = "settings.v1";

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
        for d in default_items() {
            if !self.items.iter().any(|i| i.kind == d.kind) {
                self.items.push(WidgetItem { enabled: false, ..d });
            }
        }
        self.items.retain(|i| default_items().iter().any(|d| d.kind == i.kind));
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

/// Four keys, so it avoids combinations other programs or AltGr (Ctrl+Alt on many layouts) need.
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Shift+W";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TraySettings {
    pub show_percent: bool,
    /// "auto" (the fullest current limit) or "<provider>:<window>", e.g. "anthropic:five_hour".
    pub limit: String,
}

impl Default for TraySettings {
    fn default() -> Self {
        TraySettings { show_percent: true, limit: "auto".into() }
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
    pub enabled_sources: Vec<SourceId>,
    /// Sources this install has offered; one added by a later version starts enabled.
    #[serde(default = "sources_before_antigravity")]
    pub known_sources: Vec<SourceId>,
    pub extra_paths: ExtraPaths,
    /// Provider ("anthropic" / "openai") → plan id from plans.json.
    pub plans: std::collections::BTreeMap<String, String>,
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
    /// Only the version file is downloaded; nothing is sent. No effect until the build names a
    /// release location.
    pub update_check: bool,
    /// Provider → `#rrggbb` for its limit bars and mark; a missing one uses the accent color.
    pub provider_colors: std::collections::BTreeMap<String, String>,
    /// Marks take the tool's color too; off keeps them in the text color.
    pub tint_icons: bool,
    /// Each tool's mark next to its name in the app; the widget has its own switch.
    pub show_icons: bool,
}

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
            update_check: true,
            provider_colors: Default::default(),
            tint_icons: false,
            show_icons: true,
        }
    }
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
        if !(self.fx_rate.is_finite() && self.fx_rate > 0.0) {
            self.fx_rate = 1.0;
        }
        // the value is placed into CSS, so only a plain hex color is kept
        self.provider_colors.retain(|p, c| {
            ["anthropic", "openai", "google"].contains(&p.as_str()) && c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
        });
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
    fn a_source_added_by_an_update_starts_on_without_reviving_switched_off_ones() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(KEY, r#"{"onboarded":true,"enabled_sources":["claude_code","cowork"]}"#).unwrap();
        let mut s = Settings::load(&store);
        assert_eq!(s.enabled_sources, [SourceId::ClaudeCode, SourceId::Cowork, SourceId::Antigravity]);
        assert!(SourceId::ALL.iter().all(|x| s.known_sources.contains(x)) && s.known_sources.len() == SourceId::ALL.len());

        s.enabled_sources.retain(|x| *x != SourceId::Antigravity);
        s.save(&store).unwrap();
        assert_eq!(Settings::load(&store).enabled_sources, [SourceId::ClaudeCode, SourceId::Cowork], "an explicit choice is kept");
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
