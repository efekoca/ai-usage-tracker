//! AI Usage Tracker desktop shell: wires tracker-core to a tray app with a dashboard window
//! and an always-on-top widget.

mod capture;
mod commands;
mod fonts;
mod settings;
mod state;
mod windows;
mod worker;

use settings::Settings;
use state::{load_price_book, AppState, ScanStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use tauri::{Manager, RunEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tracker_core::store::Store;

/// Command-line modes that must not start the GUI (or the single-instance check).
pub fn cli_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--statusline") {
        return Some(capture::statusline_main());
    }
    if args.iter().any(|a| a == "--revert-capture") {
        return Some(capture::revert_all_main());
    }
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started_hidden = std::env::args().any(|a| a == "--autostart");
    let data_dir = tracker_core::store::default_data_dir().unwrap_or_else(|| std::env::temp_dir().join("AIUsageTracker"));
    let _ = std::fs::create_dir_all(&data_dir);

    let app = tauri::Builder::default()
        // must be registered first: a second launch just focuses the running instance
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| windows::show_main(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--autostart"])))
        .plugin(tauri_plugin_dialog::init())
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
                started_hidden,
            });
            worker::start(app.handle().clone(), db_path, rx);
            windows::build_tray(app.handle())?;
            if !started_hidden || !settings.onboarded {
                windows::show_main(app.handle());
            }
            windows::apply_widget_settings(app.handle(), &settings);
            windows::start_fullscreen_watch(app.handle().clone());
            capture::start(app.handle());
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
            commands::quit_app,
            commands::capture_status,
            commands::set_capture,
        ])
        .build(tauri::generate_context!())
        .expect("error while building AI Usage Tracker");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // closing the last window keeps the app in the tray; only an explicit quit exits
            let quitting = app.try_state::<AppState>().is_some_and(|s| s.quitting.load(Ordering::SeqCst));
            if code.is_none() && !quitting {
                api.prevent_exit();
            }
        }
    });
}
