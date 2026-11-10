//! Keeps prices current between releases: the price list on the repository's main branch takes the
//! bundled list's place when it is newer. Only that file is downloaded and nothing is sent. The
//! user's own prices sit on top of whichever list is newest (see `tracker_core::pricing::base`).

use crate::state::{load_price_book, AppState, FETCHED_PRICING};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const EVERY: Duration = Duration::from_secs(12 * 3600);
/// The list is about 60 KB; anything far larger is not it.
const MAX_BYTES: usize = 2 << 20;

#[derive(Debug, Clone, Default, Serialize)]
pub struct PriceStatus {
    pub last_check_ms: Option<i64>,
    pub last_error: Option<String>,
}

pub fn start(app: AppHandle) {
    if crate::updates::prices_url().is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(90)).await;
        loop {
            let state = app.state::<AppState>();
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            if state.settings.read().unwrap().price_updates {
                let _ = check(&app).await;
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// Fetches the shared list and takes it when it is newer; `Ok(true)` when prices changed.
pub async fn check(app: &AppHandle) -> Result<bool, String> {
    let result = match fetch().await {
        Ok(text) => take(app, &text),
        Err(e) => Err(e),
    };
    let state = app.state::<AppState>();
    let mut st = state.prices.lock().unwrap();
    st.last_check_ms = Some(chrono::Utc::now().timestamp_millis());
    st.last_error = result.as_ref().err().cloned();
    if let Err(e) = &result {
        log::warn!("price list not updated: {e}");
    }
    result
}

async fn fetch() -> Result<String, String> {
    let url = crate::updates::prices_url().ok_or("no price list location")?;
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("AI-Usage-Tracker/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
    if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
        return Err("price list too large".into());
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("price list too large".into());
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
}

fn take(app: &AppHandle, text: &str) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let fetched = tracker_core::pricing::checked_list(text).map_err(|e| e.to_string())?;
    let path = state.data_dir.join(FETCHED_PRICING);
    // a list from the same day can still carry corrections, so the text decides
    let same = |kept: &str| kept.trim() == text.trim();
    if fetched.updated_at < crate::state::price_base(&state.data_dir).updated_at
        || std::fs::read_to_string(&path).is_ok_and(|kept| same(&kept))
        || (!path.exists() && same(tracker_core::pricing::DEFAULT_PRICING_JSON))
    {
        return Ok(false);
    }
    tracker_core::export::write_atomically(&path, |f| std::io::Write::write_all(f, text.as_bytes()).map_err(|e| e.to_string()))?;
    let (book, origin) = load_price_book(&state.data_dir);
    *state.book.write().unwrap() = book;
    *state.pricing_origin.write().unwrap() = origin;
    let _ = app.emit("data-changed", ());
    Ok(true)
}
