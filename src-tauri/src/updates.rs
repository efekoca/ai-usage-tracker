//! The release location is fixed at build time in `config/updates.json`; without one nothing is
//! ever requested. A check downloads only `latest.json` and sends nothing about the user. An
//! installer runs only when the user asks and its signature matches the key in `tauri.conf.json`.

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

pub const UPDATES_JSON: &str = include_str!("../../config/updates.json");

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct UpdatesFile {
    github_repo: String,
    endpoint: String,
    check_every_hours: u64,
}

fn file() -> UpdatesFile {
    serde_json::from_str(UPDATES_JSON).unwrap_or_default()
}

fn valid_repo(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |s: Option<&str>| {
        s.is_some_and(|s| !s.chars().all(|c| c == '.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

pub fn endpoint() -> Option<String> {
    let f = file();
    let e = f.endpoint.trim();
    if e.starts_with("https://") {
        return Some(e.to_owned());
    }
    let repo = f.github_repo.trim();
    valid_repo(repo).then(|| format!("https://github.com/{repo}/releases/latest/download/latest.json"))
}

fn interval() -> Duration {
    Duration::from_secs(3600 * file().check_every_hours.clamp(1, 168))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Available {
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateStatus {
    pub current: String,
    /// This build names a release location.
    pub configured: bool,
    pub checking: bool,
    pub last_check_ms: Option<i64>,
    pub last_error: Option<String>,
    pub available: Option<Available>,
    pub installing: bool,
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub fn status(app: &AppHandle) -> UpdateStatus {
    let mut s = app.state::<AppState>().updates.lock().unwrap().clone();
    s.current = app.package_info().version.to_string();
    s.configured = endpoint().is_some();
    s
}

fn set(app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) {
    {
        let state = app.state::<AppState>();
        let mut s = state.updates.lock().unwrap();
        f(&mut s);
    }
    let _ = app.emit("update-status", status(app));
    crate::tray::refresh_soon();
}

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let url = endpoint().ok_or("updates_not_configured")?;
    let url = url.parse().map_err(|e| format!("{e}"))?;
    app.updater_builder().endpoints(vec![url]).map_err(|e| e.to_string())?.timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())
}

pub async fn check(app: &AppHandle) -> Result<UpdateStatus, String> {
    let u = updater(app)?;
    set(app, |s| {
        s.checking = true;
        s.last_error = None;
    });
    let result = match u.check().await {
        // releases without a build for this platform (e.g. macOS for now) mean no update, not a failure
        Err(tauri_plugin_updater::Error::TargetNotFound(_) | tauri_plugin_updater::Error::TargetsNotFound(_)) => Ok(None),
        r => r,
    };
    let now = chrono::Utc::now().timestamp_millis();
    match result {
        Ok(found) => {
            let available = found.map(|u| Available { version: u.version.clone(), notes: u.body.clone(), date: u.date.map(|d| d.to_string()) });
            if let Some(a) = &available {
                log::info!("update available: v{}", a.version);
            }
            set(app, |s| {
                s.checking = false;
                s.last_check_ms = Some(now);
                s.available = available;
            });
            Ok(status(app))
        }
        Err(e) => {
            let msg = e.to_string();
            log::warn!("update check failed: {msg}");
            set(app, |s| {
                s.checking = false;
                s.last_check_ms = Some(now);
                s.last_error = Some(msg.clone());
            });
            Err(msg)
        }
    }
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    let u = updater(app)?;
    let update = u.check().await.map_err(|e| e.to_string())?.ok_or("no_update")?;
    set(app, |s| {
        s.installing = true;
        s.downloaded = 0;
        s.total = None;
        s.last_error = None;
    });
    let handle = app.clone();
    let mut last_emit = std::time::Instant::now();
    let data_dir = app.state::<AppState>().data_dir.clone();
    let result = update
        .download(
            move |chunk, total| {
                let state = handle.state::<AppState>();
                let mut s = state.updates.lock().unwrap();
                s.downloaded += chunk as u64;
                s.total = total;
                drop(s);
                if last_emit.elapsed() > Duration::from_millis(200) {
                    last_emit = std::time::Instant::now();
                    let _ = handle.emit("update-status", status(&handle));
                }
            },
            || log::info!("update downloaded and verified; starting the installer"),
        )
        .await
        .map_err(|e| e.to_string())
        .and_then(|bytes| install_package(&update, &bytes, &data_dir));
    // on Windows a successful install exits the process before this point
    if let Err(msg) = result {
        log::warn!("update install failed: {msg}");
        set(app, |s| {
            s.installing = false;
            s.last_error = Some(msg.clone());
        });
        return Err(msg);
    }
    // macOS and Linux replace the app on disk but keep the old version running
    log::info!("update installed; restarting");
    let state = app.state::<AppState>();
    state.quitting.store(true, Ordering::SeqCst);
    state.worker.send(crate::worker::Msg::Shutdown);
    app.request_restart();
    Ok(())
}

/// Linux packages go through the distribution's own package tool, which also brings in new
/// dependencies. Without a working pkexec nothing else is tried: the updater plugin would ask
/// for the password in a plain dialog of its own.
#[cfg(target_os = "linux")]
fn install_package(update: &tauri_plugin_updater::Update, bytes: &[u8], data_dir: &std::path::Path) -> Result<(), String> {
    use tauri::utils::config::BundleType;
    let ext = match tauri::utils::platform::bundle_type() {
        Some(BundleType::Deb) => "deb",
        Some(BundleType::Rpm) => "rpm",
        _ => return update.install(bytes).map_err(|e| e.to_string()),
    };
    let dir = data_dir.join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("ai-usage-tracker.{ext}"));
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let has = |tool: &str| ["/usr/bin", "/bin", "/usr/sbin", "/sbin"].iter().any(|d| std::path::Path::new(d).join(tool).is_file());
    let args = package_command(ext, &file, has);
    log::info!("installing the update with pkexec {}", args.join(" "));
    let status = std::process::Command::new("pkexec").args(&args).status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(s) if s.success() => Ok(()),
        // 126: the user dismissed the prompt; 127: not authorized or no polkit agent
        Ok(s) => {
            log::warn!("package install ended with {s}");
            Err("update_install_manual".into())
        }
        Err(e) => {
            log::warn!("pkexec could not start: {e}");
            Err("update_install_manual".into())
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn install_package(update: &tauri_plugin_updater::Update, bytes: &[u8], _data_dir: &std::path::Path) -> Result<(), String> {
    update.install(bytes).map_err(|e| e.to_string())
}

#[cfg(any(target_os = "linux", test))]
fn package_command(ext: &str, file: &std::path::Path, has: impl Fn(&str) -> bool) -> Vec<String> {
    let file = file.to_string_lossy().into_owned();
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).chain([file.clone()]).collect();
    match ext {
        "deb" if has("apt-get") => v(&["apt-get", "install", "-y", "--allow-downgrades"]),
        "deb" => v(&["dpkg", "-i"]),
        _ if has("dnf") => v(&["dnf", "install", "-y"]),
        // the packages are not GPG-signed; the updater already checked their own signature
        _ if has("zypper") => v(&["zypper", "--non-interactive", "install", "--allow-unsigned-rpm"]),
        _ => v(&["rpm", "-U"]),
    }
}

pub fn start(app: AppHandle) {
    if endpoint().is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let state = app.state::<AppState>();
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            let on = state.settings.read().unwrap().update_check;
            let busy = state.updates.lock().unwrap().installing;
            if on && !busy {
                let _ = check(&app).await;
            }
            tokio::time::sleep(interval()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn packages_are_installed_with_the_distributions_tool() {
        let f = Path::new("/home/u/.local/share/AIUsageTracker/update/ai-usage-tracker.deb");
        let only = |tools: &'static [&'static str]| move |t: &str| tools.contains(&t);
        assert_eq!(package_command("deb", f, only(&["apt-get", "dpkg"])), ["apt-get", "install", "-y", "--allow-downgrades", f.to_str().unwrap()]);
        assert_eq!(package_command("deb", f, only(&["dpkg"]))[..2], ["dpkg", "-i"]);
        let r = Path::new("/x/ai-usage-tracker.rpm");
        assert_eq!(package_command("rpm", r, only(&["dnf", "rpm"]))[..3], ["dnf", "install", "-y"]);
        assert!(package_command("rpm", r, only(&["zypper", "rpm"])).contains(&"--allow-unsigned-rpm".to_string()));
        assert_eq!(package_command("rpm", r, only(&["rpm"])), ["rpm", "-U", "/x/ai-usage-tracker.rpm"]);
    }

    #[test]
    fn only_a_plain_owner_and_name_make_a_github_location() {
        assert!(valid_repo("someone/ai-usage-tracker"));
        assert!(valid_repo("a.b/c_d"));
        assert!(!valid_repo("someone"));
        assert!(!valid_repo("someone/x/y"));
        assert!(!valid_repo("some one/x"));
        assert!(!valid_repo("../x"));
        assert!(!valid_repo(""));
    }

    #[test]
    fn the_shipped_file_parses() {
        let f: UpdatesFile = serde_json::from_str(UPDATES_JSON).unwrap();
        assert!(f.check_every_hours >= 1);
    }
}
