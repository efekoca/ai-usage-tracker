//! Process-wide state shared by commands, the worker thread and windows.

use crate::settings::Settings;
use crate::worker::Worker;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock};
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
    pub quitting: AtomicBool,
    pub started_hidden: bool,
}

impl AppState {
    pub fn user_pricing_path(&self) -> PathBuf {
        self.data_dir.join("pricing.json")
    }
}

/// Loads the user's pricing override when present and valid, otherwise the bundled file.
pub fn load_price_book(data_dir: &std::path::Path) -> (PriceBook, String) {
    let p = data_dir.join("pricing.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        match PriceBook::from_json(&s) {
            Ok(b) => return (b, "user".into()),
            Err(e) => log::warn!("user pricing.json ignored: {e}"),
        }
    }
    (PriceBook::default_book(), "bundled".into())
}
