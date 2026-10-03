//! Commands return counts, costs and metadata only, never content; hidden project names are masked here.

use crate::settings::Settings;
use crate::state::{load_price_book, AppState, ScanStatus};
use crate::windows;
use crate::worker::Msg;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};
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

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Res<Settings> {
    let old = state.settings.read().unwrap().clone();
    let mut settings = settings;
    // capture changes only via set_capture (it edits files outside the app), the shortcut only
    // via set_hotkey (it must register first)
    settings.capture = old.capture.clone();
    settings.widget.hotkey = old.widget.hotkey.clone();
    settings.widget.normalize();
    settings.save(&state.store.lock().unwrap())?;
    *state.settings.write().unwrap() = settings.clone();

    if old.enabled_sources != settings.enabled_sources
        || serde_json::to_string(&old.extra_paths).ok() != serde_json::to_string(&settings.extra_paths).ok()
        || old.onboarded != settings.onboarded
    {
        state.worker.send(Msg::Reconfigure);
    }
    if old.autostart != settings.autostart {
        windows::apply_autostart(&app, settings.autostart);
    }
    windows::apply_widget_settings(&app, &settings);
    if settings.onboarded && (!old.onboarded || old.enabled_sources != settings.enabled_sources) {
        crate::capture::wake_readers(&app);
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
pub fn parser_warnings(state: State<AppState>) -> Res<Vec<ParserWarning>> {
    let rows = state.store.lock().unwrap().files_with_warnings().map_err(err)?;
    Ok(rows.into_iter().map(|(path, parser, count, last)| ParserWarning { path, parser, count, last }).collect())
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
pub fn get_report(state: State<AppState>, period: Period, filter: Option<Filter>) -> Res<Report> {
    let store = state.store.lock().unwrap();
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
            SourceId::ChatgptDesktop => {}
        }
    }
    p
}

#[tauri::command]
pub fn get_limits(state: State<AppState>) -> Res<Vec<LimitView>> {
    let settings = state.settings.read().unwrap().clone();
    let store = state.store.lock().unwrap();
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

#[derive(Serialize)]
pub struct WidgetTool {
    tool: Tool,
    tokens: u64,
    cost_usd: f64,
}

#[derive(Serialize)]
pub struct WidgetLimit {
    provider: Provider,
    window: String,
    used_pct: Option<f64>,
    state: LimitState,
    accuracy: Accuracy,
    resets_at: Option<i64>,
    observed_ms: Option<i64>,
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
    updated_ms: i64,
}

fn widget_period(
    store: &tracker_core::store::Store,
    book: &PriceBook,
    period: Period,
    now: i64,
    providers: &HashSet<Provider>,
) -> Res<WidgetPeriod> {
    let range = analytics::period_range(period, &chrono::Local, now, None);
    let mut out = WidgetPeriod::default();
    let mut per: std::collections::BTreeMap<&'static str, WidgetTool> = Default::default();
    for e in store.events_between(range.from_ms, range.to_ms).map_err(err)? {
        if !providers.contains(&e.tool.provider()) {
            continue;
        }
        let cost = book.cost(&tracker_core::pricing::CostInput {
            model: &e.model,
            tokens: &e.tokens,
            request_input: e.request_input,
            web_search_requests: e.web_search,
            speed: e.speed.as_deref(),
            inference_geo: e.inference_geo.as_deref(),
        });
        let c = cost.map(|c| c.total()).unwrap_or(0.0);
        out.has_unpriced |= cost.is_none();
        out.tokens += e.tokens.total();
        out.cost_usd += c;
        let t = per.entry(e.tool.as_str()).or_insert(WidgetTool { tool: e.tool, tokens: 0, cost_usd: 0.0 });
        t.tokens += e.tokens.total();
        t.cost_usd += c;
    }
    out.tools = per.into_values().collect();
    Ok(out)
}

#[tauri::command]
pub fn get_widget_data(state: State<AppState>) -> Res<WidgetData> {
    let settings = state.settings.read().unwrap().clone();
    let providers = enabled_providers(&settings);
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    let now = now_ms();
    let limits = analytics::limits_view(&store, &book, now, &settings.thresholds)
        .map_err(err)?
        .into_iter()
        .filter(|l| providers.contains(&l.provider) && (l.window == "five_hour" || l.window == "seven_day"))
        .map(|l| WidgetLimit {
            provider: l.provider,
            window: l.window,
            used_pct: l.used_pct,
            state: l.state,
            accuracy: l.accuracy,
            resets_at: l.resets_at,
            observed_ms: l.observed_ms,
        })
        .collect();
    let mut provs: Vec<Provider> = providers.iter().copied().collect();
    provs.sort_by_key(|p| p.as_str());
    Ok(WidgetData {
        today: widget_period(&store, &book, Period::Today, now, &providers)?,
        days7: widget_period(&store, &book, Period::Days7, now, &providers)?,
        month1: widget_period(&store, &book, Period::Month1, now, &providers)?,
        limits,
        providers: provs,
        updated_ms: state.status.lock().unwrap().last_scan_ms.unwrap_or(now),
    })
}

#[tauri::command]
pub fn list_projects(state: State<AppState>) -> Res<Vec<ProjectRow>> {
    let hide = state.settings.read().unwrap().hide_project_names;
    let mut v = state.store.lock().unwrap().projects().map_err(err)?;
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
pub fn list_projects_for_settings(state: State<AppState>) -> Res<Vec<ProjectRow>> {
    state.store.lock().unwrap().projects().map_err(err)
}

#[tauri::command]
pub fn set_project_hidden(app: AppHandle, state: State<AppState>, id: i64, hidden: bool) -> Res<()> {
    state.store.lock().unwrap().set_project_hidden(id, hidden).map_err(err)?;
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub fn list_models(state: State<AppState>) -> Res<Vec<String>> {
    let store = state.store.lock().unwrap();
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

#[tauri::command]
pub fn save_pricing(app: AppHandle, state: State<AppState>, file: PricingFile) -> Res<()> {
    let json = serde_json::to_string_pretty(&file).map_err(err)?;
    let book = PriceBook::from_json(&json).map_err(err)?; // validate before writing
    std::fs::write(state.user_pricing_path(), json).map_err(err)?;
    *state.book.write().unwrap() = book;
    *state.pricing_origin.write().unwrap() = "user".into();
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub fn reset_pricing(app: AppHandle, state: State<AppState>) -> Res<()> {
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
pub fn export_data(
    state: State<AppState>,
    path: PathBuf,
    period: Period,
    filter: Option<Filter>,
    granularity: Granularity,
    format: Format,
) -> Res<usize> {
    let hide = state.settings.read().unwrap().hide_project_names;
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    let range = analytics::period_range(period, &chrono::Local, now_ms(), store.first_event_ms().map_err(err)?);
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).map_err(err)?);
    // UTF-8 BOM so Excel opens Turkish characters correctly
    if format == Format::Csv {
        std::io::Write::write_all(&mut f, b"\xEF\xBB\xBF").map_err(err)?;
    }
    export::export(&store, &book, range, &filter.unwrap_or_default(), &chrono::Local, granularity, format, hide, &mut f).map_err(err)
}

#[tauri::command]
pub fn backup_database(state: State<AppState>, path: PathBuf) -> Res<()> {
    state.store.lock().unwrap().backup_to(&path).map_err(err)
}

#[derive(Serialize)]
pub struct ImportResult {
    events: usize,
    limits: usize,
}

#[tauri::command]
pub fn import_database(app: AppHandle, state: State<AppState>, path: PathBuf) -> Res<ImportResult> {
    if path == state.db_path {
        return Err("cannot import the live database into itself".into());
    }
    let (events, limits) = state.store.lock().unwrap().merge_from(&path).map_err(|e| match e {
        tracker_core::rusqlite::Error::InvalidQuery => "not an AI Usage Tracker database".to_string(),
        e => e.to_string(),
    })?;
    let _ = app.emit("data-changed", ());
    Ok(ImportResult { events, limits })
}

/// Returns to onboarding so nothing is re-imported until the user confirms sources again.
#[tauri::command]
pub fn wipe_all_data(app: AppHandle, state: State<AppState>) -> Res<()> {
    let mut s = state.settings.read().unwrap().clone();
    s.onboarded = false;
    {
        let store = state.store.lock().unwrap();
        store.wipe_data().map_err(err)?;
        s.save(&store)?;
    }
    *state.settings.write().unwrap() = s.clone();
    state.worker.send(Msg::Reconfigure);
    let _ = app.emit("settings-changed", &s);
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[tauri::command]
pub fn open_data_folder(state: State<AppState>) -> Res<()> {
    std::process::Command::new("explorer").arg(&state.data_dir).spawn().map(|_| ()).map_err(err)
}

#[tauri::command]
pub fn open_url(url: String) -> Res<()> {
    if !url.starts_with("https://") {
        return Err("only https links can be opened".into());
    }
    std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).spawn().map(|_| ()).map_err(err)
}

#[tauri::command]
pub fn open_main(app: AppHandle) {
    windows::show_main(&app);
}

#[tauri::command]
pub fn set_widget_visible(app: AppHandle, state: State<AppState>, visible: bool) -> Res<()> {
    let mut s = state.settings.read().unwrap().clone();
    s.widget.visible = visible;
    save_settings(app, state, s).map(|_| ())
}

#[tauri::command]
pub async fn list_fonts() -> Vec<String> {
    crate::fonts::installed_families()
}

/// `remember` makes `corner` the anchor the widget keeps while resizing.
#[tauri::command]
pub fn place_widget(app: AppHandle, state: State<AppState>, corner: String, remember: Option<bool>) -> Res<()> {
    if let Some(w) = tauri::Manager::get_webview_window(&app, windows::WIDGET) {
        windows::place_widget(&w, &corner);
    }
    if remember.unwrap_or(true) {
        let mut s = state.settings.read().unwrap().clone();
        if s.widget.anchor != corner {
            s.widget.anchor = corner;
            s.save(&state.store.lock().unwrap())?;
            *state.settings.write().unwrap() = s.clone();
            let _ = app.emit("settings-changed", &s);
        }
    }
    Ok(())
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
pub async fn set_capture(app: AppHandle, kind: String, enabled: bool) -> Res<String> {
    crate::capture::set(&app, &kind, enabled)
}

#[tauri::command]
pub fn quit_app(app: AppHandle, state: State<AppState>) {
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
    let store = state.store.lock().unwrap();
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
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    insights::compare_models(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)
}

#[tauri::command]
pub async fn context_stats(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::ContextStats> {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    insights::context_stats(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default(), &chrono::Local).map_err(err)
}

#[tauri::command]
pub async fn plan_value(app: AppHandle) -> Res<insights::PlanValue> {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    insights::plan_value(&store, &book, range_for(&store, Period::Month1)?, &chrono::Local).map_err(err)
}

/// `from` and `to` are inclusive local days.
#[tauri::command]
pub async fn export_report(app: AppHandle, from: String, to: String, path: PathBuf) -> Res<()> {
    crate::pdf::render(&app, &from, &to, &path)?;
    *app.state::<AppState>().last_report.lock().unwrap() = Some(path);
    Ok(())
}

/// Called by the report page once its data and charts are in place.
#[tauri::command]
pub fn report_ready(state: State<AppState>) {
    if let Some(tx) = state.report_ready.lock().unwrap().take() {
        let _ = tx.send(());
    }
}

/// Opens only the PDF this app saved last, never an arbitrary path.
#[tauri::command]
pub fn open_last_report(state: State<AppState>) -> Res<()> {
    let path = state.last_report.lock().unwrap().clone().ok_or("no report yet")?;
    std::process::Command::new("explorer").arg(path).spawn().map(|_| ()).map_err(err)
}

#[tauri::command]
pub async fn get_branches(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<insights::Branches> {
    let state = app.state::<AppState>();
    let hide_all = state.settings.read().unwrap().hide_project_names;
    let store = state.store.lock().unwrap();
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
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    insights::agents_tools(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)
}

#[tauri::command]
pub async fn get_tips(app: AppHandle, period: Period, filter: Option<Filter>) -> Res<tracker_core::tips::Tips> {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    tracker_core::tips::tips(&store, &book, range_for(&store, period)?, &filter.unwrap_or_default()).map_err(err)
}

#[derive(Serialize)]
pub struct LimitHistoryView {
    history: tracker_core::history::LimitHistory,
    advice: Vec<tracker_core::history::PlanAdvice>,
    /// Provider → plan id inferred from the readings when the user chose none.
    detected_plans: std::collections::BTreeMap<String, String>,
}

/// Claude reports "max" for both Max plans, so only "pro" is taken from its readings.
fn detected_plan(store: &tracker_core::store::Store, provider: Provider, plans: &tracker_core::plans::PlansFile) -> Option<String> {
    let newest = store.latest_limits().ok()?.into_iter().filter(|s| s.provider == provider && s.plan.is_some()).max_by_key(|s| s.ts_ms)?;
    let plan = newest.plan?;
    match provider {
        Provider::OpenAI => plans.find(provider, &plan).map(|p| p.id.clone()),
        Provider::Anthropic => (plan == "pro").then_some(plan),
    }
}

#[tauri::command]
pub async fn get_limit_history(app: AppHandle, days: Option<i64>) -> Res<LimitHistoryView> {
    let state = app.state::<AppState>();
    let settings = state.settings.read().unwrap().clone();
    let providers = enabled_providers(&settings);
    let store = state.store.lock().unwrap();
    let book = state.book.read().unwrap();
    let now = now_ms();
    let mut history = tracker_core::history::limit_history(&store, now, days.unwrap_or(56).clamp(1, 3660)).map_err(err)?;
    history.series.retain(|s| providers.contains(&s.provider));
    tracker_core::history::add_local_usage(&mut history, &store, &book).map_err(err)?;
    let advice_history = tracker_core::history::limit_history(&store, now, tracker_core::history::ADVICE_DAYS).map_err(err)?;
    let plans = tracker_core::plans::PlansFile::bundled();
    let mut detected_plans = std::collections::BTreeMap::new();
    let mut ordered: Vec<Provider> = providers.into_iter().collect();
    ordered.sort();
    let advice = ordered
        .into_iter()
        .map(|p| {
            let chosen = settings.plans.get(p.as_str()).filter(|id| !id.is_empty()).cloned();
            let plan = chosen.or_else(|| {
                let d = detected_plan(&store, p, &plans)?;
                detected_plans.insert(p.as_str().to_owned(), d.clone());
                Some(d)
            });
            tracker_core::history::plan_advice(&advice_history, p, plan.as_deref(), &plans, now)
        })
        .collect();
    Ok(LimitHistoryView { history, advice, detected_plans })
}

#[derive(Serialize)]
pub struct HotkeyStatus {
    hotkey: String,
    error: Option<String>,
}

#[tauri::command]
pub fn hotkey_status(state: State<AppState>) -> HotkeyStatus {
    HotkeyStatus { hotkey: state.settings.read().unwrap().widget.hotkey.clone(), error: state.hotkey_error.lock().unwrap().clone() }
}

/// Empty clears the shortcut; one owned by another program is refused and the old one restored.
#[tauri::command]
pub async fn set_hotkey(app: AppHandle, hotkey: String) -> Res<HotkeyStatus> {
    let hotkey: String = hotkey.trim().chars().take(64).collect();
    let state = app.state::<AppState>();
    let old = state.settings.read().unwrap().widget.hotkey.clone();
    if let Err(e) = crate::hotkey::apply(&app, &hotkey) {
        let _ = crate::hotkey::apply(&app, &old);
        return Err(e);
    }
    let mut s = state.settings.read().unwrap().clone();
    s.widget.hotkey = hotkey;
    s.save(&state.store.lock().unwrap())?;
    *state.settings.write().unwrap() = s.clone();
    let _ = app.emit("settings-changed", &s);
    crate::tray::refresh_soon();
    Ok(hotkey_status(state))
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> crate::updates::UpdateStatus {
    crate::updates::status(&app)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Res<crate::updates::UpdateStatus> {
    crate::updates::check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Res<()> {
    crate::updates::install(&app).await
}

#[tauri::command]
pub async fn get_day_detail(app: AppHandle, date: String, filter: Option<Filter>) -> Res<insights::DayDetail> {
    let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(err)?;
    let state = app.state::<AppState>();
    let hide_all = state.settings.read().unwrap().hide_project_names;
    let store = state.store.lock().unwrap();
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
