//! Weekly summary as a PDF. The report page (`report.html`) is rendered in a hidden window and
//! printed with WebView2's own PrintToPdf: no print dialog and no extra software involved.

use crate::state::AppState;
use chrono::{Datelike, Days, NaiveDate};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "report";

/// One report window at a time (a manual save and the Monday save can meet).
static RENDERING: Mutex<()> = Mutex::new(());

/// Renders the summary of the local days `from..=to` (`YYYY-MM-DD`) into `out`.
pub fn render(app: &AppHandle, from: &str, to: &str, out: &Path) -> Result<(), String> {
    let valid = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok();
    if !valid(from) || !valid(to) || from > to {
        return Err("invalid_range".into());
    }
    let _one = RENDERING.lock().unwrap_or_else(|e| e.into_inner());
    // one report at a time
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
    let (tx, rx) = mpsc::channel::<()>();
    *app.state::<AppState>().report_ready.lock().unwrap() = Some(tx);
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(format!("report.html?from={from}&to={to}").into()))
        .title("AI Usage Tracker report")
        .visible(false)
        .skip_taskbar(true)
        .inner_size(820.0, 1160.0)
        .build()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        // the page says when its data is in and its charts have laid out
        rx.recv_timeout(Duration::from_secs(45)).map_err(|_| "report_timeout".to_string())?;
        print_to_pdf(&win, out)
    })();
    let _ = win.destroy();
    *app.state::<AppState>().report_ready.lock().unwrap() = None;
    result
}

/// Default folder for the Monday summaries: Documents\AI Usage Tracker.
pub fn default_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().document_dir().ok().map(|d| d.join("AI Usage Tracker"))
}

/// Monday to Sunday of the week before the one containing `today`.
pub fn last_full_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
    (monday - Days::new(7), monday - Days::new(1))
}

/// While enabled, saves last week's summary once (checked every half hour); the file name
/// carries the week, so an existing file means it is done.
pub fn start_weekly(app: AppHandle) {
    let _ = std::thread::Builder::new().name("weekly-report".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(90));
        loop {
            let state = app.state::<AppState>();
            if state.quitting.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let s = state.settings.read().unwrap().clone();
            if s.onboarded && s.weekly_report_auto {
                let dir = if s.weekly_report_dir.trim().is_empty() { default_dir(&app) } else { Some(PathBuf::from(s.weekly_report_dir.trim())) };
                if let Some(dir) = dir {
                    let (from, to) = last_full_week(chrono::Local::now().date_naive());
                    let file = dir.join(format!("AI-Usage_{from}_{to}.pdf"));
                    if !file.exists() {
                        let r = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| render(&app, &from.to_string(), &to.to_string(), &file));
                        match r {
                            Ok(()) => log::info!("weekly summary saved"),
                            Err(e) => log::warn!("weekly summary failed: {e}"),
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(30 * 60));
        }
    });
}


#[cfg(windows)]
fn print_to_pdf(win: &tauri::WebviewWindow, out: &Path) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment6, ICoreWebView2_2, ICoreWebView2_7, COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT,
    };
    use webview2_com::PrintToPdfCompletedHandler;
    use windows::core::{Interface, HSTRING};

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let path = HSTRING::from(out.as_os_str());
    win.with_webview(move |wv| {
        let done = tx.clone();
        // SAFETY: plain WebView2 COM calls on the window's own controller, on its UI thread.
        let started = unsafe {
            (|| -> windows::core::Result<()> {
                let core = wv.controller().CoreWebView2()?;
                let env: ICoreWebView2Environment6 = core.cast::<ICoreWebView2_2>()?.Environment()?.cast()?;
                let s = env.CreatePrintSettings()?;
                s.SetOrientation(COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT)?;
                // A4 with 12 mm margins; the page lays itself out for print media
                s.SetPageWidth(8.27)?;
                s.SetPageHeight(11.69)?;
                for set in [s.SetMarginTop(0.47), s.SetMarginBottom(0.47), s.SetMarginLeft(0.47), s.SetMarginRight(0.47)] {
                    set?;
                }
                s.SetShouldPrintBackgrounds(true)?;
                s.SetShouldPrintHeaderAndFooter(false)?;
                let handler = PrintToPdfCompletedHandler::create(Box::new(move |res, ok| {
                    let _ = done.send(match res {
                        Ok(()) if ok => Ok(()),
                        Ok(()) => Err("pdf_failed".into()),
                        Err(e) => Err(e.to_string()),
                    });
                    Ok(())
                }));
                core.cast::<ICoreWebView2_7>()?.PrintToPdf(&path, &s, &handler)
            })()
        };
        if let Err(e) = started {
            let _ = tx.send(Err(e.to_string()));
        }
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "pdf_timeout".to_string())?
}

#[cfg(not(windows))]
fn print_to_pdf(_win: &tauri::WebviewWindow, _out: &Path) -> Result<(), String> {
    Err("pdf_unsupported".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_full_week_is_monday_to_sunday_before_this_week() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        // Friday 2 Oct 2026 → 21–27 Sep
        assert_eq!(last_full_week(d(2026, 10, 2)), (d(2026, 9, 21), d(2026, 9, 27)));
        // on a Monday the week just ended
        assert_eq!(last_full_week(d(2026, 10, 5)), (d(2026, 9, 28), d(2026, 10, 4)));
        // Sunday still belongs to the current week
        assert_eq!(last_full_week(d(2026, 10, 4)), (d(2026, 9, 21), d(2026, 9, 27)));
    }
}
