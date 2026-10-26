use crate::capture::CaptureRuntime;
use crate::settings::Settings;
use crate::worker::Worker;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, RwLock};
use tracker_core::pricing::PriceBook;
use tracker_core::store::Store;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub last_scan_ms: Option<i64>,
    pub last_new_events: i64,
    pub files_seen: usize,
    pub warnings: usize,
    pub errors: usize,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    /// Connection used by UI commands (the worker has its own).
    pub store: Mutex<Store>,
    pub book: RwLock<PriceBook>,
    /// "bundled" or "user"
    pub pricing_origin: RwLock<String>,
    pub settings: RwLock<Settings>,
    pub status: Mutex<ScanStatus>,
    pub worker: Worker,
    pub capture: CaptureRuntime,
    /// Exclusive for "delete all data", so nothing read before a wipe is written after it.
    pub writers: RwLock<()>,
    pub quitting: AtomicBool,
    pub started_hidden: bool,
    pub report_ready: Mutex<Option<std::sync::mpsc::Sender<bool>>>,
    pub last_report: Mutex<Option<PathBuf>>,
    pub updates: Mutex<crate::updates::UpdateStatus>,
    pub hotkey_error: Mutex<Option<String>>,
}

impl AppState {
    pub fn user_pricing_path(&self) -> PathBuf {
        self.data_dir.join("pricing.json")
    }

    /// An open transaction rolls back on drop, so a poisoned lock still holds a consistent SQLite.
    pub fn db(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Holds the store for the whole read-change-save, so concurrent updates cannot undo each other.
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let store = self.db();
        let mut s = self.settings.read().unwrap().clone();
        f(&mut s);
        s.save(&store)?;
        *self.settings.write().unwrap() = s.clone();
        Ok(s)
    }
}

pub fn load_price_book(data_dir: &std::path::Path) -> (PriceBook, String) {
    let p = data_dir.join("pricing.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        match PriceBook::from_user_json(&s) {
            Ok(b) => return (b.with_catalog(opencode_catalog()), "user".into()),
            Err(e) => log::warn!("user pricing.json ignored: {e}"),
        }
    }
    (PriceBook::default_book().with_catalog(opencode_catalog()), "bundled".into())
}

/// OpenCode keeps its model catalog in `$XDG_CACHE_HOME/opencode`, else `~/.cache/opencode`, on
/// every system; an app started from the Dock may not see the shell's variable.
pub fn opencode_catalog() -> std::collections::HashMap<String, tracker_core::pricing::ModelPrice> {
    let mut dirs = Vec::new();
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        dirs.push(x);
    }
    if let Some(h) = tracker_core::discovery::Env::from_system().home {
        dirs.push(h.join(".cache"));
    }
    dirs.iter()
        .find_map(|d| std::fs::read_to_string(d.join("opencode").join("models.json")).ok())
        .map(|s| tracker_core::pricing::catalog_prices(&s))
        .unwrap_or_default()
}
