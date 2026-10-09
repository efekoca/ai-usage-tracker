use crate::settings::Settings;
use crate::state::AppState;
use notify::{RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tracker_core::capture::statusline;
use tracker_core::discovery::{self, DiscoveredFile, Env, SourceId};
use tracker_core::sources::ParserKind;
use tracker_core::ingest::ingest;
use tracker_core::store::Store;

const DEBOUNCE: Duration = Duration::from_millis(1500);
const SAFETY_RESCAN: Duration = Duration::from_secs(300);
const REWATCH_GAP: Duration = Duration::from_secs(30);

pub enum Msg {
    Scan,
    Reconfigure,
    WatchFailed,
    Shutdown,
}

#[derive(Clone)]
pub struct Worker {
    tx: Sender<Msg>,
}

impl Worker {
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }
}

/// The thread starts later via [`start`], once `AppState` is managed.
pub fn channel_pair() -> (Worker, std::sync::mpsc::Receiver<Msg>) {
    let (tx, rx) = channel();
    (Worker { tx }, rx)
}

pub fn start(app: AppHandle, db_path: PathBuf, rx: std::sync::mpsc::Receiver<Msg>) {
    let tx = app.state::<AppState>().worker.tx.clone();
    std::thread::Builder::new()
        .name("ingest-worker".into())
        .spawn(move || run(app, db_path, rx, tx))
        .expect("spawn worker");
}

fn run(app: AppHandle, db_path: PathBuf, rx: std::sync::mpsc::Receiver<Msg>, tx: Sender<Msg>) {
    let mut store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("worker cannot open database: {e}");
            return;
        }
    };
    let mut watcher: Option<notify::RecommendedWatcher> = None;
    let mut watched: Vec<(PathBuf, bool)> = Vec::new();
    let mut pending = true;
    let mut rewatch = true;
    let mut last_watch = Instant::now();

    loop {
        if rewatch {
            let settings = app.state::<AppState>().settings.read().unwrap().clone();
            let roots = watch_roots(&Env::from_system(), &settings);
            if roots != watched || watcher.is_none() {
                watcher = make_watcher(&roots, tx.clone());
                watched = roots;
                last_watch = Instant::now();
            }
            rewatch = false;
        }
        if pending {
            let deadline = Instant::now() + DEBOUNCE;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(Msg::Shutdown) => return,
                    Ok(Msg::Reconfigure) => rewatch = true,
                    Ok(Msg::WatchFailed) => watch_failed(&mut watcher, &mut rewatch, &last_watch),
                    Ok(Msg::Scan) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            scan(&app, &mut store);
            pending = false;
            continue;
        }
        match rx.recv_timeout(SAFETY_RESCAN) {
            Ok(Msg::Scan) => pending = true,
            Ok(Msg::WatchFailed) => {
                watch_failed(&mut watcher, &mut rewatch, &last_watch);
                pending = true;
            }
            Ok(Msg::Reconfigure) => {
                rewatch = true;
                pending = true;
            }
            Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                // also picks up tool directories created after startup
                rewatch = true;
                pending = true;
            }
        }
    }
}

fn scan(app: &AppHandle, store: &mut Store) {
    let state = app.state::<AppState>();
    // "delete all data" waits for a running scan; one that waited for it sees onboarding undone
    let _writing = state.writers.read().unwrap_or_else(|e| e.into_inner());
    let settings = state.settings.read().unwrap().clone();
    if !settings.onboarded {
        return; // nothing is read until the user confirms sources in onboarding
    }
    let enabled: HashSet<SourceId> = settings.enabled_sources.iter().copied().collect();
    let mut files = discovery::enumerate_files(&Env::from_system(), &settings.extra_paths, &enabled);
    // this app's own status-line capture (older rotated file first)
    let own_capture = enabled.contains(&SourceId::ClaudeCode) && settings.capture.statusline;
    for p in [statusline::rotated_file(&state.data_dir), statusline::capture_file(&state.data_dir)] {
        if own_capture && p.is_file() {
            files.push(DiscoveredFile { file_id: discovery::file_identity(&p), path: p, parser: ParserKind::StatuslineCapture, source: SourceId::ClaudeCode });
        }
    }
    {
        let mut st = state.status.lock().unwrap();
        st.running = true;
        st.done = 0;
        st.total = files.len();
    }
    let big = files.len() > 20;
    let mut last_emit = Instant::now();
    let report = ingest(store, &files, |p| {
        if big && (last_emit.elapsed() > Duration::from_millis(150) || p.done == p.total) {
            last_emit = Instant::now();
            {
                let mut st = state.status.lock().unwrap();
                st.done = p.done;
                st.total = p.total;
            }
            let _ = app.emit("scan-progress", &p);
        }
    });
    for (path, w) in report.warnings.iter().take(20) {
        log::warn!("parse warning in {}: {w}", redact_home(path));
    }
    for (path, e) in &report.errors {
        log::warn!("ingest error in {}: {e}", redact_home(path));
    }
    let changed = report.new_events() > 0 || report.limits_after != report.limits_before || report.records_written > 0;
    {
        let mut st = state.status.lock().unwrap();
        st.running = false;
        st.done = files.len();
        st.last_scan_ms = Some(chrono::Utc::now().timestamp_millis());
        st.last_new_events = report.new_events();
        st.files_seen = report.files_seen;
        st.warnings = report.warning_count;
        st.errors = report.errors.len();
    }
    let _ = app.emit("scan-finished", state.status.lock().unwrap().clone());
    if changed {
        let _ = app.emit("data-changed", ());
    }
}

/// Keeps user names out of the app log.
fn redact_home(p: &str) -> String {
    match dirs_home() {
        Some(h) if p.len() >= h.len() && p.is_char_boundary(h.len()) && p[..h.len()].eq_ignore_ascii_case(&h) => {
            format!("~{}", &p[h.len()..])
        }
        _ => p.to_owned(),
    }
}

fn dirs_home() -> Option<String> {
    Env::from_system().home.map(|h| h.to_string_lossy().into_owned())
}

fn watch_roots(env: &Env, s: &Settings) -> Vec<(PathBuf, bool)> {
    let on = |id| s.enabled_sources.contains(&id);
    let mut v = Vec::new();
    if on(SourceId::ClaudeCode) {
        v.extend(discovery::claude_config_roots(env, &s.extra_paths).into_iter().map(|r| (r.join("projects"), true)));
    }
    for d in discovery::claude_desktop_dirs(env) {
        if on(SourceId::Cowork) {
            v.push((d.join("local-agent-mode-sessions"), true));
        }
        if on(SourceId::ClaudeDesktop) {
            v.push((d.clone(), false)); // only for plan-usage-history.json
        }
    }
    v.push((statusline::capture_dir(&crate_data_dir()), false));
    if on(SourceId::Codex) {
        for h in discovery::codex_homes(env, &s.extra_paths) {
            v.push((h.join("sessions"), true));
            v.push((h.join("archived_sessions"), true));
        }
    }
    if on(SourceId::Antigravity) {
        v.extend(discovery::antigravity_dirs(env).into_iter().map(|d| (d.join("conversations"), false)));
    }
    v.retain(|(p, _)| p.is_dir());
    v.sort();
    v.dedup();
    v
}

fn crate_data_dir() -> PathBuf {
    tracker_core::store::default_data_dir().unwrap_or_default()
}

fn is_relevant(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "jsonl" || e == "db" || e == "db-wal")
        || p.file_name().is_some_and(|n| n == "plan-usage-history.json")
}

fn make_watcher(roots: &[(PathBuf, bool)], tx: Sender<Msg>) -> Option<notify::RecommendedWatcher> {
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
        Ok(ev) if ev.paths.iter().any(|p| is_relevant(p)) => {
            let _ = tx.send(Msg::Scan);
        }
        Ok(_) => {}
        Err(_) => {
            let _ = tx.send(Msg::WatchFailed);
        }
    })
    .map_err(|e| log::warn!("file watcher unavailable, falling back to periodic scans: {e}"))
    .ok()?;
    for (r, recursive) in roots {
        let mode = if *recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        if let Err(e) = w.watch(r, mode) {
            log::warn!("cannot watch a source directory: {e}");
        }
    }
    Some(w)
}

/// Rebuilt at most once per [`REWATCH_GAP`], so a persistent error cannot spin.
fn watch_failed(watcher: &mut Option<notify::RecommendedWatcher>, rewatch: &mut bool, last_watch: &Instant) {
    if last_watch.elapsed() >= REWATCH_GAP && watcher.take().is_some() {
        log::warn!("file watcher reported an error; rebuilding it");
        *rewatch = true;
    }
}
