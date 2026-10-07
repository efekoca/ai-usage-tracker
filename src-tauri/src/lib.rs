mod capture;
mod commands;
mod fonts;
mod hotkey;
#[cfg(target_os = "linux")]
mod gnome_shortcut;
#[cfg(target_os = "linux")]
mod kde_shortcut;
#[cfg(target_os = "linux")]
mod portal_shortcut;
mod pdf;
mod settings;
mod state;
mod tray;
mod updates;
mod windows;
mod worker;

use settings::Settings;
use state::{load_price_book, AppState, ScanStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use tauri::{Manager, RunEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tracker_core::store::Store;

/// Shows or hides the widget of the running app; the desktop shortcut on GNOME runs this.
pub(crate) const TOGGLE_WIDGET: &str = "--toggle-widget";

/// The file the app was started from, for commands that other programs run later.
pub(crate) fn app_path() -> Result<std::path::PathBuf, String> {
    // an AppImage runs from a temporary mount; the file it was started from stays put
    #[cfg(target_os = "linux")]
    if let Some(p) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).filter(|p| p.is_file()) {
        return Ok(p);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    if temporary_location(&exe) {
        return Err("app_not_installed".into());
    }
    Ok(exe)
}

/// A copy opened from the disk image or moved aside by App Translocation disappears later.
#[cfg(target_os = "macos")]
pub(crate) fn temporary_location(exe: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return true;
    }
    let Ok(path) = std::ffi::CString::new(exe.as_os_str().as_bytes()) else { return false };
    let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated and `fs` is a valid out-parameter
    unsafe { libc::statfs(path.as_ptr(), &mut fs) == 0 && fs.f_flags & libc::MNT_RDONLY as u32 != 0 }
}

/// Start at login, the status line and updates need the app where it stays.
#[cfg(target_os = "macos")]
fn warn_if_temporary(app: &tauri::AppHandle, s: &Settings) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    if !std::env::current_exe().is_ok_and(|e| temporary_location(&e)) {
        return;
    }
    let (title, text) = if tray::turkish(s) {
        ("Uygulamayı taşıyın", "AI Usage Tracker şu an disk görüntüsünden ya da geçici bir konumdan çalışıyor. Girişte başlatma, Claude Code durum satırı ve güncellemeler için uygulamayı Uygulamalar klasörüne taşıyıp oradan açın.")
    } else {
        ("Move the app", "AI Usage Tracker is running from the disk image or a temporary location. Move it to the Applications folder and open it from there so start at login, the Claude Code status line and updates work.")
    };
    let app = app.clone();
    std::thread::spawn(move || {
        // once the window is up: an alert without one opens off screen
        std::thread::sleep(std::time::Duration::from_secs(2));
        let Some(w) = app.get_webview_window(windows::MAIN) else { return };
        app.dialog().message(text).title(title).kind(MessageDialogKind::Warning).parent(&w).show(|_| {});
    });
}

/// Runs before the GUI so these modes never start it or hit the single-instance check.
pub fn cli_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--statusline") {
        return Some(capture::statusline_main());
    }
    if args.iter().any(|a| a == "--revert-capture") {
        return Some(capture::revert_all_main());
    }
    if relaunched_outside_package() {
        return Some(0);
    }
    None
}

/// A start from inside an MSIX app (e.g. the Claude desktop app) can inherit its file-system
/// virtualization, so %LOCALAPPDATA% writes land in a private copy. Relaunch via Explorer instead.
#[cfg(windows)]
fn relaunched_outside_package() -> bool {
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return false };
    if !writes_are_redirected(&data_dir) {
        return false;
    }
    // never loop: a relaunch that is still redirected carries on where it is
    let marker = data_dir.join(".relaunched");
    let recent = std::fs::metadata(&marker)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < std::time::Duration::from_secs(30));
    if recent {
        log_early("still redirected after a relaunch; continuing in place");
        return false;
    }
    let _ = std::fs::write(&marker, b"");
    let Ok(exe) = std::env::current_exe() else { return false };
    std::process::Command::new(system_exe("explorer.exe")).arg(exe).spawn().is_ok()
}

/// Shows a folder, or opens a file in its default app.
pub(crate) fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut c = std::process::Command::new(system_exe("explorer.exe"));
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("/usr/bin/open");
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut c = std::process::Command::new("xdg-open");
    #[cfg(target_os = "linux")]
    desktop_env(&mut c);
    c.arg(path).spawn().map(|_| ())
}

pub(crate) fn open_link(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut c = {
        let mut c = std::process::Command::new(system_exe(r"System32\rundll32.exe"));
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("/usr/bin/open");
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut c = std::process::Command::new("xdg-open");
    #[cfg(target_os = "linux")]
    desktop_env(&mut c);
    c.arg(url).spawn().map(|_| ())
}

/// By full path, so PATH and the current folder play no part in which file runs.
pub(crate) fn system_exe(relative: &str) -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join(relative)
}

#[cfg(windows)]
fn writes_are_redirected(data_dir: &std::path::Path) -> bool {
    let (Some(folder), Some(local)) = (data_dir.file_name(), data_dir.parent()) else { return false };
    if std::fs::create_dir_all(data_dir).is_err() {
        return false;
    }
    let name = format!(".where-{}", std::process::id());
    let probe = data_dir.join(&name);
    if std::fs::write(&probe, b"").is_err() {
        return false;
    }
    let found = std::fs::read_dir(local.join("Packages"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().join("LocalCache").join("Local").join(folder).join(&name).exists());
    let _ = std::fs::remove_file(&probe);
    found
}

#[cfg(windows)]
fn log_early(msg: &str) {
    eprintln!("ai-usage-tracker: {msg}");
}

#[cfg(not(windows))]
fn relaunched_outside_package() -> bool {
    false
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Wayland lets no app place its windows, keep one on top or grab a key for itself, so under a
/// Wayland session the app runs through XWayland, which allows all three. GTK falls back to
/// Wayland when XWayland cannot be reached, and `GDK_BACKEND` overrides the choice.
#[cfg(target_os = "linux")]
fn prefer_xwayland() {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("WAYLAND_DISPLAY") && set("DISPLAY") && !set("GDK_BACKEND") {
        // SAFETY: runs first in `run`, before any other thread exists
        unsafe { std::env::set_var("GDK_BACKEND", "x11,wayland") };
        CHOSE_GDK_BACKEND.store(true, Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
static CHOSE_GDK_BACKEND: AtomicBool = AtomicBool::new(false);

/// Programs started for the user (file manager, browser, PDF viewer, gsettings) get the desktop's
/// own setup back: not the app's XWayland choice, nor the GTK modules an AppImage brings along.
#[cfg(target_os = "linux")]
pub(crate) fn desktop_env(c: &mut std::process::Command) -> &mut std::process::Command {
    if CHOSE_GDK_BACKEND.load(Ordering::Relaxed) {
        c.env_remove("GDK_BACKEND");
    }
    let Some(appdir) = std::env::var_os("APPDIR").filter(|d| !d.is_empty() && std::env::var_os("APPIMAGE").is_some()) else { return c };
    let inside = |v: &std::ffi::OsStr| std::env::split_paths(v).any(|p| p.starts_with(&appdir));
    for k in ["GTK_PATH", "GTK_DATA_PREFIX", "GTK_EXE_PREFIX", "GTK_IM_MODULE_FILE", "GIO_MODULE_DIR", "GDK_PIXBUF_MODULE_FILE", "GSETTINGS_SCHEMA_DIR"] {
        if std::env::var_os(k).is_some_and(|v| inside(&v)) {
            c.env_remove(k);
        }
    }
    if let Some(dirs) = std::env::var_os("XDG_DATA_DIRS") {
        let kept: Vec<_> = std::env::split_paths(&dirs).filter(|p| !p.starts_with(&appdir)).collect();
        match std::env::join_paths(kept) {
            Ok(v) if !v.is_empty() => c.env("XDG_DATA_DIRS", v),
            _ => c.env_remove("XDG_DATA_DIRS"),
        };
    }
    c
}

/// The single-instance plugin panics on a session bus address it cannot parse; without one it
/// starts anyway and the instance lock below keeps a second copy out.
#[cfg(target_os = "linux")]
fn drop_unusable_session_bus() {
    use std::str::FromStr;
    const KEY: &str = "DBUS_SESSION_BUS_ADDRESS";
    if std::env::var(KEY).is_ok_and(|a| zbus::Address::from_str(&a).is_err()) || std::env::var_os(KEY).is_some_and(|a| a.to_str().is_none()) {
        // SAFETY: runs first in `run`, before any other thread exists
        unsafe { std::env::remove_var(KEY) };
    }
}

/// Kept for the life of the process. A restart after an update starts the new copy just before
/// the old one exits, so it waits a moment for the lock.
#[cfg(target_os = "linux")]
fn instance_lock(data_dir: &std::path::Path) -> bool {
    let Ok(file) = std::fs::File::options().create(true).truncate(false).write(true).open(data_dir.join("instance.lock")) else { return true };
    for _ in 0..20 {
        match file.try_lock() {
            Ok(()) => {
                std::mem::forget(file);
                return true;
            }
            Err(std::fs::TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(250)),
            Err(_) => return true,
        }
    }
    false
}

pub fn run() {
    #[cfg(target_os = "linux")]
    prefer_xwayland();
    #[cfg(target_os = "linux")]
    drop_unusable_session_bus();
    // a desktop shortcut that starts the app with --toggle-widget brings up just the widget
    let started_hidden = std::env::args().any(|a| a == "--autostart" || a == TOGGLE_WIDGET);
    let data_dir = tracker_core::store::default_data_dir().unwrap_or_else(|| std::env::temp_dir().join("AIUsageTracker"));
    let _ = std::fs::create_dir_all(&data_dir);
    // the database and the settings backups are the user's alone
    #[cfg(unix)]
    let _ = std::fs::set_permissions(&data_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));

    let app = tauri::Builder::default()
        // must be registered first: a second launch just focuses the running instance
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == TOGGLE_WIDGET) {
                windows::handle_menu(app, "tray:widget");
            } else {
                windows::show_main(app);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .target(Target::new(TargetKind::Folder { path: data_dir.join("logs"), file_name: Some("tracker".into()) }))
                .target(Target::new(TargetKind::Stdout))
                .max_file_size(1_000_000)
                .rotation_strategy(RotationStrategy::KeepOne)
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(move |app| {
            // the plugin above cannot tell a second copy apart without a session bus
            #[cfg(target_os = "linux")]
            if !instance_lock(&data_dir) {
                log::info!("already running; exiting");
                app.handle().cleanup_before_exit();
                std::process::exit(0);
            }
            let db_path = data_dir.join("tracker.db");
            let store = Store::open(&db_path)?;
            let settings = Settings::load(&store);
            let (book, origin) = load_price_book(&data_dir);
            let (worker, rx) = worker::channel_pair();
            app.manage(AppState {
                data_dir: data_dir.clone(),
                db_path: db_path.clone(),
                store: Mutex::new(store),
                book: RwLock::new(book),
                pricing_origin: RwLock::new(origin),
                settings: RwLock::new(settings.clone()),
                status: Mutex::new(ScanStatus::default()),
                worker,
                capture: capture::CaptureRuntime::new(),
                quitting: AtomicBool::new(false),
                writers: RwLock::new(()),
                started_hidden,
                report_ready: Mutex::new(None),
                last_report: Mutex::new(None),
                updates: Mutex::new(updates::UpdateStatus::default()),
                hotkey_error: Mutex::new(None),
            });
            worker::start(app.handle().clone(), db_path, rx);
            tray::build(app.handle())?;
            tray::start(app.handle().clone());
            // a combination another program owns is reported in settings, not fatal
            let _ = hotkey::apply(app.handle(), &settings.widget.hotkey, false);
            if !started_hidden || !settings.onboarded {
                windows::show_main(app.handle());
            }
            windows::apply_widget_settings(app.handle(), &settings);
            if settings.autostart {
                // rewrites an entry made by an older version (unquoted path) and follows a moved exe
                windows::apply_autostart(true, false);
            }
            windows::start_fullscreen_watch(app.handle().clone());
            capture::start(app.handle());
            pdf::start_weekly(app.handle().clone());
            updates::start(app.handle().clone());
            #[cfg(target_os = "macos")]
            warn_if_temporary(app.handle(), &settings);
            log::info!("started v{}", app.package_info().version);
            Ok(())
        })
        .on_menu_event(|app, e| windows::handle_menu(app, e.id().as_ref()))
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_settings,
            commands::save_settings,
            commands::detect_sources,
            commands::scan_status,
            commands::rescan,
            commands::parser_warnings,
            commands::get_report,
            commands::get_limits,
            commands::get_widget_data,
            commands::list_projects,
            commands::list_projects_for_settings,
            commands::set_project_hidden,
            commands::list_models,
            commands::get_pricing,
            commands::save_pricing,
            commands::reset_pricing,
            commands::get_plans,
            commands::export_data,
            commands::backup_database,
            commands::import_database,
            commands::wipe_all_data,
            commands::open_data_folder,
            commands::open_url,
            commands::open_main,
            commands::set_widget_visible,
            commands::widget_menu,
            commands::place_widget,
            commands::list_fonts,
            commands::get_sessions,
            commands::compare_models,
            commands::context_stats,
            commands::plan_value,
            commands::export_report,
            commands::report_ready,
            commands::open_last_report,
            commands::quit_app,
            commands::capture_status,
            commands::set_capture,
            commands::get_branches,
            commands::get_agents_tools,
            commands::get_tips,
            commands::get_limit_history,
            commands::get_day_detail,
            commands::hotkey_status,
            commands::set_hotkey,
            commands::update_status,
            commands::check_update,
            commands::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building AI Usage Tracker");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { api, code, .. } => {
            // closing the last window keeps the app in the tray; only an explicit quit exits
            let quitting = app.try_state::<AppState>().is_some_and(|s| s.quitting.load(Ordering::SeqCst));
            if code.is_none() && !quitting {
                api.prevent_exit();
            }
        }
        // a Dock click brings back the main window, even while only the widget is open
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => windows::show_main(app),
        _ => {}
    });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::temporary_location;
    use std::path::Path;

    #[test]
    fn disk_images_and_translocated_copies_are_temporary() {
        assert!(temporary_location(Path::new("/private/var/folders/x/T/AppTranslocation/ABC/d/AI Usage Tracker.app/Contents/MacOS/ai-usage-tracker")));
        // the sealed system volume is mounted read-only, like a disk image
        assert!(temporary_location(Path::new("/System/Applications/Calculator.app/Contents/MacOS/Calculator")));
        assert!(!temporary_location(&std::env::temp_dir()));
    }
}
