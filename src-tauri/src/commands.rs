//! Commands return counts, costs and metadata only, never content; hidden project names are masked here.

use crate::settings::Settings;
use crate::state::{load_price_book, AppState, ScanStatus};
use crate::windows;
use crate::worker::Msg;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tracker_core::analytics::{self, Filter, LimitState, LimitView, Period, Report};
use tracker_core::discovery::{self, Env, SourceId, SourceStatus};
use tracker_core::export::{self, Format, Granularity};
use tracker_core::insights;
use tracker_core::model::{Accuracy, Provider, Tool};
use tracker_core::pricing::{PriceBook, PricingFile};
use tracker_core::store::ProjectRow;

type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    data_dir: String,
    pricing_origin: String,
    pricing_updated_at: String,
    platform: &'static str,
    supports_mica: bool,
    accent_color: Option<String>,
    started_hidden: bool,
    reports_dir: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle, state: State<AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        data_dir: state.data_dir.to_string_lossy().into_owned(),
        pricing_origin: state.pricing_origin.read().unwrap().clone(),
        pricing_updated_at: state.book.read().unwrap().file().updated_at.clone(),
        platform: std::env::consts::OS,
        supports_mica: windows::supports_mica(),
        accent_color: windows::accent_color(),
        started_hidden: state.started_hidden,
        reports_dir: crate::pdf::default_dir(&app).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// Commands that change state or reveal what settings hide answer only the dashboard window.
fn main_only(w: &WebviewWindow) -> Res<()> {
    if w.label() == windows::MAIN { Ok(()) } else { Err("not_allowed".into()) }
}

// Commands that lock the store are async: a sync command runs on the event loop, and a long
// query holding the store would freeze every window until it ends.
/// `base` is what the page saw; only what it changed from there is saved.
#[tauri::command]
pub async fn save_settings(app: AppHandle, window: WebviewWindow, state: State<'_, AppState>, settings: Settings, base: Option<Settings>) -> Res<Settings> {
    main_only(&window)?;
    save_settings_inner(&app, &state, |old| match base {
        Some(base) => old.with_changes(&base, &settings),
        None => settings,
    })
}

/// `make` builds the new settings from the current ones while the store is locked.
fn save_settings_inner(app: &AppHandle, state: &AppState, make: impl FnOnce(&Settings) -> Settings) -> Res<Settings> {
    let store = state.db();
    let old = state.settings.read().unwrap().clone();
    let mut settings = make(&old);
    // capture changes only via set_capture (it edits files outside the app), the shortcut only
    // via set_hotkey (it must register first); a page's widget position can predate the last drag
    settings.capture = old.capture.clone();
    settings.known_sources = old.known_sources.clone();
    settings.widget.hotkey = old.widget.hotkey.clone();
    settings.widget.x = old.widget.x;
    settings.widget.y = old.widget.y;
    settings.widget.anchor = old.widget.anchor.clone();
    settings.save(&store)?;
    *state.settings.write().unwrap() = settings.clone();
    drop(store);

    if old.enabled_sources != settings.enabled_sources
        || serde_json::to_string(&old.extra_paths).ok() != serde_json::to_string(&settings.extra_paths).ok()
        || old.onboarded != settings.onboarded
    {
        state.worker.send(Msg::Reconfigure);
    }
    if old.autostart != settings.autostart {
        windows::apply_autostart(settings.autostart, true);
    }
    windows::apply_widget_settings(app, &settings);
    if old.theme != settings.theme {
        windows::apply_theme(app, &settings);
    }
    if settings.onboarded && !old.onboarded {
        crate::capture::resume(app);
    } else if settings.onboarded && old.enabled_sources != settings.enabled_sources {
        crate::capture::wake_readers(app);
    }
    let _ = app.emit("settings-changed", &settings);
    crate::tray::refresh_soon();
    Ok(settings)
}

#[derive(Serialize)]
pub struct SourceInfo {
    #[serde(flatten)]
    status: SourceStatus,
    enabled: bool,
}

#[tauri::command]
pub fn detect_sources(state: State<AppState>) -> Vec<SourceInfo> {
    let s = state.settings.read().unwrap().clone();
    discovery::detect(&Env::from_system(), &s.extra_paths)
        .into_iter()
        .map(|st| SourceInfo { enabled: s.enabled_sources.contains(&st.id), status: st })
        .collect()
}

#[tauri::command]
pub fn scan_status(state: State<AppState>) -> ScanStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn rescan(state: State<AppState>) {
    state.worker.send(Msg::Scan);
}

#[derive(Serialize)]
pub struct ParserWarning {
    path: String,
    parser: String,
    count: i64,
    last: String,
}

#[tauri::command]
pub async fn parser_warnings(window: WebviewWindow, state: State<'_, AppState>) -> Res<Vec<ParserWarning>> {
    main_only(&window)?;
    let hide = state.settings.read().unwrap().hide_project_names;
    let rows = state.db().files_with_warnings().map_err(err)?;
    Ok(rows.into_iter().map(|(path, parser, count, last)| ParserWarning { path: if hide { mask_project_folder(&path) } else { path }, parser, count, last }).collect())
}

/// Claude Code names each project's log folder after its path (`projects/-Users-me-app/…`).
fn mask_project_folder(path: &str) -> String {
    let mut after_projects = false;
    path.split_inclusive(['/', '\\'])
        .map(|part| {
            let name = part.trim_end_matches(['/', '\\']);
            let out = if after_projects { part.replacen(name, "…", 1) } else { part.to_owned() };
            after_projects = name == "projects";
            out
        })
        .collect()
}

fn mask_report(r: &mut Report, hide_all: bool) {
    for g in &mut r.by_project {
        if hide_all || g.hidden {
            g.label.clear();
            g.hidden = true;
        }
    }
}

fn mask_limits(v: &mut [LimitView], hide_all: bool) {
    for l in v {
        for p in &mut l.projects {
            if hide_all || p.hidden {
                p.name.clear();
                p.hidden = true;
            }
        }
    }
}

#[tauri::command]
pub async fn get_report(state: State<'_, AppState>, period: Period, filter: Option<Filter>) -> Res<Report> {
    let store = state.db();
    let book = state.book.read().unwrap();
    let range = analytics::period_range(period, &chrono::Local, now_ms(), store.first_event_ms().map_err(err)?);
    let mut r = analytics::report(&store, &book, range, &filter.unwrap_or_default(), &chrono::Local).map_err(err)?;
    mask_report(&mut r, state.settings.read().unwrap().hide_project_names);
    Ok(r)
}

pub(crate) fn enabled_providers(s: &Settings) -> HashSet<Provider> {
    let mut p = HashSet::new();
    for src in &s.enabled_sources {
        match src {
            SourceId::ClaudeCode | SourceId::Cowork | SourceId::ClaudeDesktop => {
                p.insert(Provider::Anthropic);
            }
            SourceId::Codex => {
                p.insert(Provider::OpenAI);
            }
            SourceId::Antigravity => {
                p.insert(Provider::Google);
            }
            // OpenCode counts toward no plan: its records do not say how a request was paid
            SourceId::ChatgptDesktop | SourceId::OpenCode => {}
        }
    }
    p
}

#[tauri::command]
pub async fn get_limits(state: State<'_, AppState>) -> Res<Vec<LimitView>> {
    let settings = state.settings.read().unwrap().clone();
    let store = state.db();
    let book = state.book.read().unwrap();
    let providers = enabled_providers(&settings);
    let mut v: Vec<LimitView> = analytics::limits_view(&store, &book, now_ms(), &settings.thresholds)
        .map_err(err)?
        .into_iter()
        .filter(|l| providers.contains(&l.provider))
        .collect();
    mask_limits(&mut v, settings.hide_project_names);
    Ok(v)
}

/// Codex and Claude accounts, each provider's only while it has more than one.
#[tauri::command]
pub async fn get_accounts(state: State<'_, AppState>) -> Res<Vec<tracker_core::accounts::Account>> {
    let providers = enabled_providers(&state.settings.read().unwrap());
    Ok(tracker_core::accounts::accounts(&state.db()).map_err(err)?.into_iter().filter(|a| providers.contains(&a.provider)).collect())
}

/// A known account, by which the unnamed ones are numbered per provider.
#[derive(Serialize)]
pub struct KnownAccount {
    provider: Provider,
    id: String,
}

/// Every provider's known accounts in order of first use.
pub(crate) fn known_accounts(store: &tracker_core::store::Store) -> Result<Vec<KnownAccount>, String> {
    Ok(tracker_core::accounts::accounts(store).map_err(err)?.into_iter().filter_map(|a| Some(KnownAccount { provider: a.provider, id: a.id? })).collect())
}

#[derive(Serialize)]
pub struct WidgetTool {
    tool: Tool,
    tokens: u64,
    cost_usd: f64,
    has_unpriced: bool,
}

#[derive(Serialize)]
pub struct WidgetLimit {
    provider: Provider,
    limit_id: String,
    window: String,
    used_pct: Option<f64>,
    state: LimitState,
    accuracy: Accuracy,
    resets_at: Option<i64>,
    observed_ms: Option<i64>,
    /// The Codex or Claude account, while there are several.
    account: Option<String>,
    active: bool,
}

#[derive(Serialize, Default)]
pub struct WidgetPeriod {
    tokens: u64,
    cost_usd: f64,
    has_unpriced: bool,
    tools: Vec<WidgetTool>,
}

#[derive(Serialize)]
pub struct WidgetData {
    today: WidgetPeriod,
    days7: WidgetPeriod,
    month1: WidgetPeriod,
    limits: Vec<WidgetLimit>,
    providers: Vec<Provider>,
    /// OpenCode's source is on and it was used in the last 30 days.
    opencode: bool,
    /// Known accounts in order of first use, which numbers the unnamed ones.
    accounts: Vec<KnownAccount>,
    updated_ms: i64,
}

fn widget_period(
    store: &tracker_core::store::Store,
    book: &PriceBook,
    period: Period,
    now: i64,
    shown: &impl Fn(Tool) -> bool,
) -> Res<WidgetPeriod> {
    let range = analytics::period_range(period, &chrono::Local, now, None);
    let mut out = WidgetPeriod::default();
    let mut per: std::collections::BTreeMap<&'static str, WidgetTool> = Default::default();
    for e in store.events_between(range.from_ms, range.to_ms).map_err(err)? {
        if !shown(e.tool) {
            continue;
        }
        let cost = book.cost(&tracker_core::pricing::CostInput {
            model: &e.model,
            tokens: &e.tokens,
            request_input: e.request_input,
            web_search_requests: e.web_search,
            speed: e.speed.as_deref(),
            inference_geo: e.inference_geo.as_deref(),
            ts_ms: Some(e.ts_ms),
        });
        let c = cost.map(|c| c.total()).unwrap_or(0.0);
        out.has_unpriced |= cost.is_none();
        out.tokens += e.tokens.total();
        out.cost_usd += c;
        let t = per.entry(e.tool.as_str()).or_insert(WidgetTool { tool: e.tool, tokens: 0, cost_usd: 0.0, has_unpriced: false });
        t.tokens += e.tokens.total();
        t.cost_usd += c;
        t.has_unpriced |= cost.is_none();
    }
    out.tools = per.into_values().collect();
    Ok(out)
}

#[tauri::command]
pub async fn get_widget_data(state: State<'_, AppState>) -> Res<WidgetData> {
    let settings = state.settings.read().unwrap().clone();
    let providers = enabled_providers(&settings);
    let store = state.db();
    let book = state.book.read().unwrap();
    let now = now_ms();
    let limits = analytics::limits_view(&store, &book, now, &settings.thresholds)
        .map_err(err)?
        .into_iter()
        .filter(|l| (l.active || l.account.is_some()) && providers.contains(&l.provider) && (l.window == "five_hour" || l.window == "seven_day"))
        .map(|l| WidgetLimit {
            provider: l.provider,
            limit_id: l.limit_id,
            window: l.window,
            used_pct: l.used_pct,
            state: l.state,
            accuracy: l.accuracy,
            resets_at: l.resets_at,
            observed_ms: l.observed_ms,
            account: l.account,
            active: l.active,
        })
        .collect();
    let accounts = known_accounts(&store)?;
    let opencode = settings.enabled_sources.contains(&SourceId::OpenCode);
    let shown = |t: Tool| t.provider().map_or(opencode, |p| providers.contains(&p));
    let mut provs: Vec<Provider> = providers.iter().copied().collect();
    provs.sort_by_key(|p| p.as_str());
    let month1 = widget_period(&store, &book, Period::Month1, now, &shown)?;
    Ok(WidgetData {
        today: widget_period(&store, &book, Period::Today, now, &shown)?,
        days7: widget_period(&store, &book, Period::Days7, now, &shown)?,
        opencode: month1.tools.iter().any(|t| t.tool == Tool::OpenCode && t.tokens > 0),
        month1,
        limits,
        providers: provs,
        accounts,
        updated_ms: state.status.lock().unwrap().last_scan_ms.unwrap_or(now),
    })
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Res<Vec<ProjectRow>> {
    let hide = state.settings.read().unwrap().hide_project_names;
    let mut v = state.db().projects().map_err(err)?;
    for p in &mut v {
        if hide || p.hidden {
            p.name.clear();
            p.path.clear();
            p.hidden = true;
        }
    }
    Ok(v)
}

/// Unmasked: the settings screen is where the user chooses what to hide.
#[tauri::command]
pub async fn list_projects_for_settings(window: WebviewWindow, state: State<'_, AppState>) -> Res<Vec<ProjectRow>> {
    main_only(&window)?;
    state.db().projects().map_err(err)
}

#[tauri::command]
pub async fn set_project_hidden(window: WebviewWindow, app: AppHandle, state: State<'_, AppState>, id: i64, hidden: bool) -> Res<()> {
    main_only(&window)?;
    state.db().set_project_hidden(id, hidden).map_err(err)?;
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> Res<Vec<String>> {
    let store = state.db();
    let mut st = store.conn().prepare("SELECT DISTINCT model FROM usage_event ORDER BY model").map_err(err)?;
    let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

#[derive(Serialize)]
pub struct PricingInfo {
    file: PricingFile,
    origin: String,
}

#[tauri::command]
pub fn get_pricing(state: State<AppState>) -> PricingInfo {
    PricingInfo { file: state.book.read().unwrap().file().clone(), origin: state.pricing_origin.read().unwrap().clone() }
}

/// OpenCode catalog prices of the models in `models` the price list does not know.
#[tauri::command]
pub fn catalog_prices(state: State<AppState>, models: Vec<String>) -> Vec<tracker_core::pricing::ModelPrice> {
    state.book.read().unwrap().catalog_entries(models.iter().map(String::as_str))
}

#[tauri::command]
pub fn save_pricing(window: WebviewWindow, app: AppHandle, state: State<AppState>, file: PricingFile) -> Res<()> {
    main_only(&window)?;
    let json = serde_json::to_string_pretty(&file).map_err(err)?;
    let book = PriceBook::from_json(&json).map_err(err)?.with_catalog(crate::state::opencode_catalog()); // validate before writing
    std::fs::write(state.user_pricing_path(), json).map_err(err)?;
    *state.book.write().unwrap() = book;
    *state.pricing_origin.write().unwrap() = "user".into();
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub fn reset_pricing(window: WebviewWindow, app: AppHandle, state: State<AppState>) -> Res<()> {
    main_only(&window)?;
    let p = state.user_pricing_path();
    if p.exists() {
        std::fs::remove_file(&p).map_err(err)?;
    }
    let (book, origin) = load_price_book(&state.data_dir);
    *state.book.write().unwrap() = book;
    *state.pricing_origin.write().unwrap() = origin;
    let _ = app.emit("data-changed", ());
    Ok(())
}

pub const PLANS_JSON: &str = tracker_core::plans::DEFAULT_PLANS_JSON;

#[tauri::command]
pub fn get_plans() -> serde_json::Value {
    serde_json::from_str(PLANS_JSON).unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
pub async fn export_data(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: PathBuf,
    period: Period,
    filter: Option<Filter>,
    granularity: Granularity,
    format: Format,
) -> Res<usize> {
    main_only(&window)?;
    let hide = state.settings.read().unwrap().hide_project_names;
    let store = state.db();
    let book = state.book.read().unwrap();
    let range = analytics::period_range(period, &chrono::Local, now_ms(), store.first_event_ms().map_err(err)?);
    crate::pdf::not_the_archive(&store, &path)?;
    export::write_atomically(&path, |f| {
        // UTF-8 BOM so Excel opens Turkish characters correctly
        if format == Format::Csv {
            std::io::Write::write_all(f, b"\xEF\xBB\xBF").map_err(err)?;
        }
        export::export(&store, &book, range, &filter.unwrap_or_default(), &chrono::Local, granularity, format, hide, f).map_err(err)
    })
}

#[tauri::command]
pub async fn backup_database(window: WebviewWindow, state: State<'_, AppState>, path: PathBuf) -> Res<()> {
    main_only(&window)?;
    state.db().backup_to(&path).map_err(err)
}

#[derive(Serialize)]
pub struct ImportResult {
    events: usize,
    limits: usize,
}

#[tauri::command]
pub async fn import_database(window: WebviewWindow, app: AppHandle, state: State<'_, AppState>, path: PathBuf) -> Res<ImportResult> {
    main_only(&window)?;
    let mut db = state.db();
    let (events, limits) = db.merge_from(&path).map_err(|e| match e {
        tracker_core::rusqlite::Error::InvalidQuery => "not an AI Usage Tracker database".to_string(),
        tracker_core::rusqlite::Error::InvalidPath(_) => "cannot import the live database into itself".to_string(),
        e => e.to_string(),
    })?;
    if let Err(e) = tracker_core::accounts::resolve(&mut db) {
        log::warn!("accounts not resolved after import: {e}");
    }
    drop(db);
    let _ = app.emit("data-changed", ());
    Ok(ImportResult { events, limits })
}

/// Returns to onboarding so nothing is re-imported until the user confirms sources again.
/// Async: waiting for a running scan or limit read must not block the window.
#[tauri::command]
pub async fn wipe_all_data(window: WebviewWindow, app: AppHandle) -> Res<()> {
    main_only(&window)?;
    let state = app.state::<AppState>();
    if let Err(e) = crate::capture::pause(&app) {
        crate::capture::resume(&app);
        return Err(err(e));
    }
    let wiped = (|| {
        let _writers = state.writers.write().unwrap_or_else(|e| e.into_inner());
        let s = {
            let store = state.db();
            let mut s = state.settings.read().unwrap().clone();
            s.onboarded = false;
            store.wipe_data().map_err(err)?;
            s.save(&store)?;
            *state.settings.write().unwrap() = s.clone();
            s
        };
        tracker_core::capture::statusline::delete_records(&state.data_dir).map_err(err)?;
        Ok::<_, String>(s)
    })();
    let s = match wiped {
        Ok(s) => s,
        Err(e) => {
            if state.settings.read().unwrap().onboarded {
                crate::capture::resume(&app);
            }
            return Err(e);
        }
    };
    state.worker.send(Msg::Reconfigure);
    // onboarding starts over: the widget hides and the tray drops the old limits
    crate::windows::apply_widget_settings(&app, &s);
    crate::tray::refresh_soon();
    let _ = app.emit("settings-changed", &s);
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub fn open_data_folder(window: WebviewWindow, state: State<AppState>) -> Res<()> {
    main_only(&window)?;
    crate::open_path(&state.data_dir).map_err(err)
}

#[tauri::command]
pub fn open_url(window: WebviewWindow, url: String) -> Res<()> {
    main_only(&window)?;
    if !url.starts_with("https://") {
        return Err("only https links can be opened".into());
    }
    crate::open_link(&url).map_err(err)
}

#[tauri::command]
pub fn open_main(app: AppHandle) {
    windows::show_main(&app);
}

#[tauri::command]
pub fn set_widget_visible(window: WebviewWindow, app: AppHandle, state: State<AppState>, visible: bool) -> Res<()> {
    main_only(&window)?;
    save_settings_inner(&app, &state, |old| {
        let mut s = old.clone();
        s.widget.visible = visible;
        s
    })
    .map(|_| ())
}

#[tauri::command]
pub async fn list_fonts(app: AppHandle) -> Vec<String> {
    crate::fonts::installed_families(&app)
}

/// `remember` makes `corner` the anchor the widget keeps while resizing.
#[tauri::command]
pub async fn place_widget(app: AppHandle, state: State<'_, AppState>, corner: String, remember: Option<bool>) -> Res<()> {
    if let Some(w) = tauri::Manager::get_webview_window(&app, windows::WIDGET) {
        windows::place_widget(&w, &corner);
    }
    if remember.unwrap_or(true) && state.settings.read().unwrap().widget.anchor != corner {
        let s = state.update_settings(|s| s.widget.anchor = corner)?;
        let _ = app.emit("settings-changed", &s);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct TrayPreview {
    title: String,
    /// Each item's own text in `items` and in the catalog; empty when it has no value.
    parts: Vec<String>,
    catalog: Vec<String>,
}

/// What the text next to the tray icon would show with `tray`, before it is saved.
#[tauri::command]
pub async fn tray_preview(app: AppHandle, tray: crate::settings::TraySettings) -> Res<TrayPreview> {
    let mut tray = tray;
    tray.normalize();
    let s = app.state::<AppState>().settings.read().unwrap().clone();
    let readings = crate::tray::readings(&app, &s).unwrap_or_default();
    let mut data = crate::tray::data(&app, &s, &tray, readings);
    // the catalog shows every kind's value
    if data.usage.is_none() {
        (data.usage, data.opencode) = crate::tray::usage(&app, &s).map_or((None, None), |(u, o)| (Some(u), o));
    }
    let f = crate::tray::format(&s, &tray);
    // catalog chips carry their own names
    let bare = crate::settings::TraySettings { label: "none".into(), ..tray.clone() };
    let g = crate::tray::format(&s, &bare);
    Ok(TrayPreview {
        title: if tray.title && !cfg!(windows) { f.title(&data) } else { String::new() },
        parts: tray.items.iter().map(|i| f.part(i, &data).unwrap_or_default()).collect(),
        catalog: crate::settings::tray_catalog().iter().map(|i| g.part(i, &data).unwrap_or_default()).collect(),
    })
}

#[tauri::command]
pub fn widget_menu(app: AppHandle) {
    windows::popup_widget_menu(&app);
}

#[tauri::command]
pub fn capture_status(app: AppHandle) -> crate::capture::CaptureStatus {
    crate::capture::status(&app)
}

/// Async: enabling Codex runs one limit read (up to ~20 s) and must not block the UI thread.
#[tauri::command]
pub async fn set_capture(window: WebviewWindow, app: AppHandle, kind: String, enabled: bool) -> Res<String> {
    main_only(&window)?;
    crate::capture::set(&app, &kind, enabled)
}

#[tauri::command]
pub async fn set_read_interval(window: WebviewWindow, app: AppHandle, kind: String, minutes: u64) -> Res<Settings> {
    main_only(&window)?;
    crate::capture::set_interval(&app, &kind, minutes)
}

#[tauri::command]
pub async fn install_cli(window: WebviewWindow, app: AppHandle, kind: String) -> Res<String> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(move || crate::capture::install_cli(&app, &kind)).await.map_err(err)?
}

#[tauri::command]
pub fn quit_app(window: WebviewWindow, app: AppHandle, state: State<AppState>) {
    if main_only(&window).is_err() {
        return;
    }
    state.quitting.store(true, Ordering::SeqCst);
    state.worker.send(Msg::Shutdown);
    app.exit(0);
}


// Insight commands are async so long ranges never block the window's event loop.

fn range_for(store: &tracker_core::store::Store, period: Period) -> Res<analytics::Range> {
    Ok(analytics::period_range(period, &chrono::Local, now_ms(), store.first_event_ms().map_err(err)?))
}

#[tauri::command]
pub async fn get_sessions(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::Sessions> {
    let state = app.state::<AppState>();
    let hide_all = state.settings.read().unwrap().hide_project_names;
    let store = state.db();
    let book = state.book.read().unwrap();
    let mut r = insights::sessions(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)?;
    for s in &mut r.sessions {
        if hide_all || s.hidden {
            s.project.clear();
            s.hidden = true;
        }
    }
    Ok(r)
}

#[tauri::command]
pub async fn compare_models(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::ModelCompare> {
    let state = app.state::<AppState>();
    let store = state.db();
    let book = state.book.read().unwrap();
    insights::compare_models(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)
}

#[tauri::command]
pub async fn context_stats(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::ContextStats> {
    let state = app.state::<AppState>();
    let store = state.db();
    let book = state.book.read().unwrap();
    insights::context_stats(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default(), &chrono::Local).map_err(err)
}

#[tauri::command]
pub async fn plan_value(app: AppHandle) -> Res<insights::PlanValue> {
    let state = app.state::<AppState>();
    let store = state.db();
    let book = state.book.read().unwrap();
    insights::plan_value(&store, &book, range_for(&store, Period::Month1)?, &chrono::Local).map_err(err)
}

/// `from` and `to` are inclusive local days.
#[tauri::command]
pub async fn export_report(window: WebviewWindow, app: AppHandle, from: String, to: String, path: PathBuf) -> Res<()> {
    main_only(&window)?;
    // rendering blocks for seconds while the report page calls async commands of its own
    let handle = app.clone();
    let out = path.clone();
    tauri::async_runtime::spawn_blocking(move || crate::pdf::render(&handle, &from, &to, &out)).await.map_err(err)??;
    *app.state::<AppState>().last_report.lock().unwrap() = Some(path);
    Ok(())
}

/// `!ok`: the page's data failed, so the export fails instead of printing the error page.
#[tauri::command]
pub fn report_ready(window: WebviewWindow, state: State<AppState>, ok: bool) {
    if window.label() != crate::pdf::LABEL {
        return;
    }
    if let Some(tx) = state.report_ready.lock().unwrap().take() {
        let _ = tx.send(ok);
    }
}

/// Opens only the PDF this app saved last, never an arbitrary path.
#[tauri::command]
pub fn open_last_report(window: WebviewWindow, state: State<AppState>) -> Res<()> {
    main_only(&window)?;
    let path = state.last_report.lock().unwrap().clone().ok_or("no report yet")?;
    crate::open_path(&path).map_err(err)
}

#[tauri::command]
pub async fn get_branches(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::Branches> {
    let state = app.state::<AppState>();
    let hide_all = state.settings.read().unwrap().hide_project_names;
    let store = state.db();
    let book = state.book.read().unwrap();
    let mut r = insights::branches(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)?;
    for b in &mut r.rows {
        if hide_all || b.hidden {
            b.project.clear();
            b.hidden = true;
            // a branch name can say as much as the project's (a client, a feature)
            b.branch_hidden = b.branch.take().is_some();
        }
    }
    Ok(r)
}

#[tauri::command]
pub async fn get_agents_tools(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::AgentsTools> {
    let state = app.state::<AppState>();
    let store = state.db();
    let book = state.book.read().unwrap();
    insights::agents_tools(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)
}

#[tauri::command]
pub async fn get_subagent_cache(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::SubagentCache> {
    let state = app.state::<AppState>();
    let store = state.db();
    let book = state.book.read().unwrap();
    insights::subagent_cache(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default(), &chrono::Local).map_err(err)
}

#[tauri::command]
pub async fn get_tips(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<tracker_core::tips::Tips> {
    let state = app.state::<AppState>();
    let store = state.db();
    let book = state.book.read().unwrap();
    tracker_core::tips::tips(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)
}

#[derive(Serialize)]
pub struct LimitHistoryView {
    history: tracker_core::history::LimitHistory,
    advice: Vec<tracker_core::history::PlanAdvice>,
}

/// Claude reports "max" for both Max plans, so only "pro" is taken from its readings.
fn detected_plan(store: &tracker_core::store::Store, provider: Provider, plans: &tracker_core::plans::PlansFile) -> Option<String> {
    let newest = store.latest_limits().ok()?.into_iter().filter(|s| s.provider == provider && s.plan.is_some()).max_by_key(|s| s.ts_ms)?;
    let plan = newest.plan?;
    match provider {
        Provider::OpenAI => plans.plans(provider).iter().find(|d| tracker_core::plans::fits(provider, &d.id, &plan)).map(|p| p.id.clone()),
        Provider::Anthropic => (plan == "pro").then_some(plan),
        Provider::Google => None,
    }
}

#[tauri::command]
pub async fn get_limit_history(app: AppHandle, days: Option<i64>) -> Res<LimitHistoryView> {
    let state = app.state::<AppState>();
    let settings = state.settings.read().unwrap().clone();
    let providers = enabled_providers(&settings);
    let store = state.db();
    let book = state.book.read().unwrap();
    let now = now_ms();
    let mut history = tracker_core::history::limit_history(&store, now, days.unwrap_or(56).clamp(1, 3660)).map_err(err)?;
    history.series.retain(|s| providers.contains(&s.provider));
    tracker_core::history::add_local_usage(&mut history, &store, &book).map_err(err)?;
    let advice_history = tracker_core::history::limit_history(&store, now, tracker_core::history::ADVICE_DAYS).map_err(err)?;
    let plans = tracker_core::plans::PlansFile::bundled();
    let accounts = tracker_core::accounts::accounts(&store).map_err(err)?;
    // a provider whose plans have no limit windows never reports readings to advise on
    let mut ordered: Vec<Provider> =
        providers.into_iter().filter(|p| plans.plans(*p).iter().any(|d| !d.windows.is_empty())).collect();
    ordered.sort();
    let mut advice = Vec::new();
    for p in ordered {
        let chosen = settings.plans.get(p.as_str()).filter(|id| !id.is_empty()).map(String::as_str);
        let several: Vec<(&str, Option<&str>, bool)> =
            accounts.iter().filter(|a| a.provider == p).filter_map(|a| Some((a.id.as_deref()?, a.read_plan.as_deref(), a.active))).collect();
        if several.is_empty() {
            let detected = if chosen.is_none() { detected_plan(&store, p, &plans) } else { None };
            let mut a = tracker_core::history::plan_advice(&advice_history, p, chosen.or(detected.as_deref()), &plans, now);
            a.detected = detected.is_some();
            advice.push(a);
            continue;
        }
        for (id, read, active) in several {
            let own = settings.account_plans.get(id).map(String::as_str).filter(|own| plans.find(p, own).is_some());
            let plan = plans.account_plan(p, own, read, chosen);
            let mut a = tracker_core::history::account_advice(&advice_history, p, id, plan.as_deref(), &plans, now);
            // another account not read in the advice period is not in use
            if a.observed_days == 0 && !active {
                continue;
            }
            a.detected = own.is_none() && plan.is_some() && plan.as_deref() != chosen;
            advice.push(a);
        }
    }
    Ok(LimitHistoryView { history, advice })
}

#[derive(Serialize)]
pub struct HotkeyStatus {
    hotkey: String,
    error: Option<String>,
    /// The desktop owns the shortcut (Wayland portal) and may have assigned other keys.
    desktop: bool,
}

#[tauri::command]
pub fn hotkey_status(state: State<AppState>) -> HotkeyStatus {
    #[cfg(target_os = "linux")]
    let desktop = crate::portal_shortcut::active();
    #[cfg(not(target_os = "linux"))]
    let desktop = false;
    HotkeyStatus { hotkey: state.settings.read().unwrap().widget.hotkey.clone(), error: state.hotkey_error.lock().unwrap().clone(), desktop }
}

/// Empty clears the shortcut; one owned by another program is refused and the old one restored.
#[tauri::command]
pub async fn set_hotkey(window: WebviewWindow, app: AppHandle, hotkey: String) -> Res<HotkeyStatus> {
    main_only(&window)?;
    let hotkey: String = hotkey.trim().chars().take(64).collect();
    let state = app.state::<AppState>();
    let old = state.settings.read().unwrap().widget.hotkey.clone();
    if let Err(e) = crate::hotkey::apply(&app, &hotkey, true) {
        let _ = crate::hotkey::apply(&app, &old, true);
        return Err(e);
    }
    let s = state.update_settings(|s| s.widget.hotkey = hotkey)?;
    let _ = app.emit("settings-changed", &s);
    crate::tray::refresh_soon();
    Ok(hotkey_status(state))
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> crate::updates::UpdateStatus {
    crate::updates::status(&app)
}

#[tauri::command]
pub async fn check_update(window: WebviewWindow, app: AppHandle) -> Res<crate::updates::UpdateStatus> {
    main_only(&window)?;
    crate::updates::check(&app).await
}

#[tauri::command]
pub async fn install_update(window: WebviewWindow, app: AppHandle) -> Res<()> {
    main_only(&window)?;
    crate::updates::install(&app).await
}

#[tauri::command]
pub async fn get_day_detail(app: AppHandle, date: String, filter: Option<Filter>) -> Res<insights::DayDetail> {
    let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(err)?;
    let state = app.state::<AppState>();
    let hide_all = state.settings.read().unwrap().hide_project_names;
    let store = state.db();
    let book = state.book.read().unwrap();
    let mut d = insights::day_detail(&store, &book, date, &filter.unwrap_or_default(), &chrono::Local).map_err(err)?;
    for g in &mut d.by_project {
        if hide_all || g.hidden {
            g.label.clear();
            g.hidden = true;
        }
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_project_name_stays_out_of_log_paths() {
        assert_eq!(
            mask_project_folder("/Users/me/.claude/projects/-Users-me-secret-app/0199-sess/subagents/agent-a.jsonl"),
            "/Users/me/.claude/projects/…/0199-sess/subagents/agent-a.jsonl"
        );
        assert_eq!(mask_project_folder(r"C:\Users\me\.claude\projects\C--Work-secret\s.jsonl"), r"C:\Users\me\.claude\projects\…\s.jsonl");
        assert_eq!(mask_project_folder("/Users/me/.codex/sessions/2026/10/rollout-1.jsonl"), "/Users/me/.codex/sessions/2026/10/rollout-1.jsonl");
    }
}
