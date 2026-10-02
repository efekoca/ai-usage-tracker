//! User settings, stored as one JSON document in the tracker database.
//! Unknown/missing fields fall back to defaults so older/newer settings never break startup.

use serde::{Deserialize, Serialize};
use tracker_core::analytics::Threshold;
use tracker_core::discovery::{ExtraPaths, SourceId};
use tracker_core::store::Store;

const KEY: &str = "settings.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetSettings {
    pub visible: bool,
    /// Background opacity 0.3–1.0.
    pub opacity: f64,
    /// "s" | "m" | "l"
    pub size: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub auto_hide_fullscreen: bool,
}

impl Default for WidgetSettings {
    fn default() -> Self {
        WidgetSettings { visible: true, opacity: 0.85, size: "m".into(), x: None, y: None, auto_hide_fullscreen: true }
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
    pub autostart: bool,
    /// Optional network use: allow fetching a newer pricing/plan file. Off by default.
    pub allow_config_updates: bool,
    /// Visual: show cost or tokens first.
    pub primary_metric: String,
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
            autostart: false,
            allow_config_updates: false,
            primary_metric: "tokens".into(),
        }
    }
}

impl Settings {
    pub fn load(store: &Store) -> Settings {
        match store.setting(KEY) {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("settings unreadable, using defaults: {e}");
                Settings::default()
            }),
            _ => Settings::default(),
        }
    }

    pub fn save(&self, store: &Store) -> Result<(), String> {
        let mut s = self.clone();
        s.widget.opacity = s.widget.opacity.clamp(0.3, 1.0);
        if !(s.fx_rate.is_finite() && s.fx_rate > 0.0) {
            s.fx_rate = 1.0;
        }
        store.set_setting(KEY, &serde_json::to_string(&s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}
