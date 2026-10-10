//! Renders `report.html` in a hidden window and prints it to PDF without a print dialog: WebView2's
//! PrintToPdf on Windows, a WKWebView print operation on macOS, a WebKitGTK one on Linux.

use crate::state::AppState;
use chrono::{Datelike, Days, NaiveDate};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "report";

/// One report window at a time (a manual save and the Monday save can meet).
static RENDERING: Mutex<()> = Mutex::new(());

pub fn render(app: &AppHandle, from: &str, to: &str, out: &Path) -> Result<(), String> {
    let valid = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok();
    if !valid(from) || !valid(to) || from > to {
        return Err("invalid_range".into());
    }
    // the PDF replaces its target (manual and Monday saves alike)
    not_the_archive(&app.state::<AppState>().db(), out)?;
    let _one = RENDERING.lock().unwrap_or_else(|e| e.into_inner());
    close(app)?;
    let (tx, rx) = mpsc::channel::<bool>();
    *app.state::<AppState>().report_ready.lock().unwrap() = Some(tx);
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(format!("report.html?from={from}&to={to}").into()))
        .title("AI Usage Tracker report")
        .visible(false)
        .skip_taskbar(true)
        .inner_size(820.0, 1160.0)
        .build()
        .map_err(|e| e.to_string())?;
    let partial = partial_path(out);
    let result = (|| {
        if !rx.recv_timeout(Duration::from_secs(45)).map_err(|_| "report_timeout".to_string())? {
            return Err("report_data_failed".to_string());
        }
        // printed beside the target, so a failed render never leaves a broken PDF in its place
        print_to_pdf(&win, &partial)?;
        std::fs::rename(&partial, out).map_err(|e| e.to_string())
    })();
    let _ = win.destroy();
    *app.state::<AppState>().report_ready.lock().unwrap() = None;
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    // the next render reuses the label, so the window must be gone before the lock is released
    if let Err(e) = close(app) {
        log::warn!("report window still open: {e}");
    }
    result
}

/// Closes the report window and waits until Tauri has let go of its label. Never called on the
/// main thread, which does the closing.
fn close(app: &AppHandle) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while let Some(w) = app.get_webview_window(LABEL) {
        if std::time::Instant::now() > deadline {
            return Err("report_busy".into());
        }
        let _ = w.destroy();
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

/// An export never writes over the open archive or its SQLite side files.
pub fn not_the_archive(store: &tracker_core::store::Store, out: &Path) -> Result<(), String> {
    if store.is_live_file(out) { Err("export_target_is_archive".into()) } else { Ok(()) }
}

fn partial_path(out: &Path) -> PathBuf {
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    out.with_file_name(format!("{stem}.partial-{}.pdf", std::process::id()))
}

pub fn default_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().document_dir().ok().map(|d| d.join("AI Usage Tracker"))
}

pub fn last_full_week(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
    (monday - Days::new(7), monday - Days::new(1))
}

/// The last week saved, so a file the user moved or deleted is not made again.
const LAST_WEEK_SAVED: &str = "weekly_report.last_week";

/// The file name carries the week, so an existing file also means that week is done.
pub fn start_weekly(app: AppHandle) {
    let _ = std::thread::Builder::new().name("weekly-report".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(90));
        // the first scan after login may still be reading the week's logs
        let scan_deadline = std::time::Instant::now() + Duration::from_secs(15 * 60);
        while app.state::<AppState>().status.lock().unwrap().last_scan_ms.is_none() && std::time::Instant::now() < scan_deadline {
            std::thread::sleep(Duration::from_secs(5));
        }
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
                    let saved = state.db().setting(LAST_WEEK_SAVED).ok().flatten().is_some_and(|w| w == from.to_string());
                    if !saved && !file.exists() {
                        let r = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| render(&app, &from.to_string(), &to.to_string(), &file));
                        match r {
                            Ok(()) => {
                                let _ = state.db().set_setting(LAST_WEEK_SAVED, &from.to_string());
                                log::info!("weekly summary saved");
                            }
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

#[cfg(target_os = "macos")]
fn print_to_pdf(win: &tauri::WebviewWindow, out: &Path) -> Result<(), String> {
    use objc2::runtime::ProtocolObject;
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::{NSPrintInfo, NSPrintJobSavingURL, NSPrintSaveJob, NSPrintingPaginationMode, NSWindow};
    use objc2_foundation::{NSSize, NSURL};
    use objc2_web_kit::WKWebView;

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let id = mac_print::start(tx.clone());
    let out_file = out.to_owned();
    win.with_webview(move |wv| {
        let fail = |e: &str| {
            mac_print::finish(id);
            let _ = tx.send(Err(e.into()));
        };
        let (Some(mtm), Some(url)) = (MainThreadMarker::new(), NSURL::from_file_path(&out_file)) else { return fail("pdf_failed") };
        // SAFETY: `inner` and `ns_window` are this window's own WKWebView and NSWindow, used on the main thread.
        let (web, window): (&WKWebView, &NSWindow) = unsafe { (&*wv.inner().cast(), &*wv.ns_window().cast()) };
        let info = NSPrintInfo::new();
        // A4 with 12 mm margins, like the Windows output; the page lays itself out for print media
        info.setPaperSize(NSSize::new(595.28, 841.89));
        info.setTopMargin(34.0);
        info.setBottomMargin(34.0);
        info.setLeftMargin(34.0);
        info.setRightMargin(34.0);
        info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
        info.setHorizontallyCentered(false);
        info.setVerticallyCentered(false);
        // SAFETY: AppKit constants and the print info's own attribute dictionary.
        unsafe {
            info.setJobDisposition(NSPrintSaveJob);
            info.dictionary().setObject_forKey(&url, ProtocolObject::from_ref(NSPrintJobSavingURL));
        }
        let delegate = mac_print::Delegate::new(mtm);
        // SAFETY: WebKit pages the document in another process, so the operation must run
        // asynchronously (the blocking `runOperation` never learns the page count).
        unsafe {
            let op = web.printOperationWithPrintInfo(&info);
            op.setShowsPrintPanel(false);
            op.setShowsProgressPanel(false);
            if let Some(v) = op.view() {
                v.setFrame(web.bounds());
            }
            op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                window,
                Some(&delegate),
                Some(sel!(printOperationDidRun:success:contextInfo:)),
                id as *mut std::ffi::c_void,
            );
        }
        mac_print::keep(delegate);
    })
    .map_err(|e| {
        mac_print::finish(id);
        e.to_string()
    })?;
    let r = rx.recv_timeout(Duration::from_secs(60));
    mac_print::finish(id);
    r.map_err(|_| "pdf_timeout".to_string())??;
    if out.is_file() {
        Ok(())
    } else {
        Err("pdf_failed".into())
    }
}

#[cfg(target_os = "macos")]
mod mac_print {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, NSObject};
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Mutex};

    /// The render waiting for an answer, by number: a late answer from an earlier, timed-out
    /// operation must not settle a newer one.
    static DONE: Mutex<Option<(usize, Answer)>> = Mutex::new(None);
    static NEXT: AtomicUsize = AtomicUsize::new(1);

    thread_local! {
        // AppKit does not retain the delegate while the operation runs; one per running operation,
        // so a new render never frees the delegate of one still in progress
        static ALIVE: RefCell<Vec<Retained<Delegate>>> = const { RefCell::new(Vec::new()) };
    }

    type Answer = mpsc::Sender<Result<(), String>>;

    pub fn start(tx: Answer) -> usize {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        *DONE.lock().unwrap() = Some((id, tx));
        id
    }

    /// The sender of render `id`, unless another render has taken its place.
    pub fn finish(id: usize) -> Option<Answer> {
        let mut done = DONE.lock().unwrap();
        if done.as_ref().is_some_and(|d| d.0 == id) { done.take().map(|d| d.1) } else { None }
    }

    pub fn keep(d: Retained<Delegate>) {
        ALIVE.with(|a| a.borrow_mut().push(d));
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "AIUsageTrackerPrintDelegate"]
        pub struct Delegate;

        impl Delegate {
            #[unsafe(method(printOperationDidRun:success:contextInfo:))]
            fn did_run(&self, _op: *mut AnyObject, success: Bool, ctx: *mut std::ffi::c_void) {
                if let Some(tx) = finish(ctx as usize) {
                    let _ = tx.send(if success.as_bool() { Ok(()) } else { Err("pdf_failed".into()) });
                }
                ALIVE.with(|a| a.borrow_mut().retain(|d| !std::ptr::eq(&**d, self)));
            }
        }
    );

    impl Delegate {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: a plain NSObject subclass without ivars.
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }
}

#[cfg(target_os = "linux")]
fn print_to_pdf(win: &tauri::WebviewWindow, out: &Path) -> Result<(), String> {
    use webkit2gtk::{PrintOperation, PrintOperationExt};

    let uri = gtk::glib::filename_to_uri(out, None).map_err(|e| e.to_string())?.to_string();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    win.with_webview(move |wv| {
        let settings = gtk::PrintSettings::new();
        // GTK's file printer has a translated name, so it is looked up in GTK's own catalog
        settings.set_printer(&gtk::glib::dgettext(Some("gtk30"), "Print to File"));
        settings.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
        settings.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(&uri));
        // A4 with 12 mm margins, like the other platforms; the page lays itself out for print media
        let setup = gtk::PageSetup::new();
        setup.set_paper_size(&gtk::PaperSize::new(Some(gtk::PAPER_NAME_A4)));
        setup.set_orientation(gtk::PageOrientation::Portrait);
        setup.set_top_margin(12.0, gtk::Unit::Mm);
        setup.set_bottom_margin(12.0, gtk::Unit::Mm);
        setup.set_left_margin(12.0, gtk::Unit::Mm);
        setup.set_right_margin(12.0, gtk::Unit::Mm);

        let op = PrintOperation::new(&wv.inner());
        op.set_print_settings(&settings);
        op.set_page_setup(&setup);
        let failed = tx.clone();
        // "failed" comes before "finished", so the first message decides
        op.connect_failed(move |_, e| {
            let _ = failed.send(Err(e.to_string()));
        });
        // the operation keeps itself alive until it finishes
        let keep = std::cell::RefCell::new(Some(op.clone()));
        op.connect_finished(move |_| {
            let _ = tx.send(Ok(()));
            keep.borrow_mut().take();
        });
        op.print();
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "pdf_timeout".to_string())??;
    if out.is_file() {
        Ok(())
    } else {
        Err("pdf_failed".into())
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn print_to_pdf(_win: &tauri::WebviewWindow, _out: &Path) -> Result<(), String> {
    Err("pdf_unsupported".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_is_refused_over_the_archive_and_its_side_files() {
        let dir = std::env::temp_dir().join(format!("aiut-target-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("tracker.db");
        let store = tracker_core::store::Store::open(&db).unwrap();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let target = PathBuf::from(format!("{}{suffix}", db.display()));
            assert_eq!(not_the_archive(&store, &target), Err("export_target_is_archive".into()), "{suffix}");
        }
        assert_eq!(not_the_archive(&store, &dir.join("AI-Usage.pdf")), Ok(()));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_full_week_is_monday_to_sunday_before_this_week() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        // a Friday
        assert_eq!(last_full_week(d(2026, 10, 2)), (d(2026, 9, 21), d(2026, 9, 27)));
        // on a Monday the week just ended
        assert_eq!(last_full_week(d(2026, 10, 5)), (d(2026, 9, 28), d(2026, 10, 4)));
        // Sunday still belongs to the current week
        assert_eq!(last_full_week(d(2026, 10, 4)), (d(2026, 9, 21), d(2026, 9, 27)));
    }

    #[test]
    fn the_partial_file_sits_beside_the_target_under_another_name() {
        let dir = std::env::temp_dir().join("reports");
        let out = dir.join("AI-Usage_2026-09-21_2026-09-27.pdf");
        let p = partial_path(&out);
        assert_eq!(p.parent(), Some(dir.as_path()));
        assert_ne!(p, out);
        assert_eq!(p.extension().unwrap(), "pdf");
    }
}

/// Guards the vendored glib fix (RUSTSEC-2024-0429): reading a string array must see what
/// GLib wrote, also in optimized builds.
#[cfg(all(test, target_os = "linux"))]
mod glib_patch {
    use gtk::glib::{ToVariant, Variant};

    #[test]
    fn a_string_array_reads_back_from_both_ends() {
        let v = Variant::array_from_iter::<String>(["a", "bc", "déf"].map(|s| s.to_variant()));
        let iter = v.array_iter_str().unwrap();
        assert_eq!(iter.collect::<Vec<_>>(), ["a", "bc", "déf"]);
        let mut iter = v.array_iter_str().unwrap();
        assert_eq!((iter.next_back(), iter.nth(1), iter.next()), (Some("déf"), Some("bc"), None));
    }
}
