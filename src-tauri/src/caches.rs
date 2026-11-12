//! Claude Code conversations whose prompt cache still lasts, as the app shows them: named, priced
//! and kept to the projects the user follows.

use crate::settings::Settings;
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use tracker_core::insights;
use tracker_core::model::Tool;

/// Conversations the user stopped following, until the app restarts.
static DROPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

const MONTH_MS: i64 = 30 * 86_400_000;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Cache {
    pub session_id: String,
    /// The project's name; empty while it is hidden or not known.
    pub project: String,
    /// The branch, left out with a hidden project's name.
    pub branch: Option<String>,
    pub model: String,
    pub client: Option<String>,
    pub ttl_ms: i64,
    pub expires_ms: i64,
    pub started_ms: i64,
    pub tokens: u64,
    pub refresh_usd: Option<f64>,
    pub rewrite_usd: Option<f64>,
    /// A hook waits in this conversation, so it can be refreshed now.
    pub refreshable: bool,
    /// A refresh was asked for and has not reached the logs yet.
    pub refreshing: bool,
    /// The last refresh came to nothing: the cache's time did not move on.
    pub refresh_failed: bool,
    /// It lets every tool run without asking, so it is never refreshed.
    pub unasked: bool,
    /// It is refreshed by itself before it runs out.
    pub auto: bool,
    /// Its automatic refreshes stopped: more would cost more than writing the cache again.
    pub auto_capped: bool,
    /// Refreshes in the last two days and their API-equivalent cost.
    pub refreshes: u32,
    pub refreshes_usd: f64,
}

/// The caches the user follows, the soonest to run out first; none while the feature is off.
pub fn followed(state: &AppState, s: &Settings, now_ms: i64) -> Vec<Cache> {
    if !s.cache_timer {
        return Vec::new();
    }
    let (live, projects) = {
        let store = state.db();
        let book = state.book.read().unwrap();
        let live = insights::live_caches(&store, &book, now_ms).unwrap_or_else(|e| {
            log::warn!("live caches: {e}");
            Vec::new()
        });
        (live, store.projects().unwrap_or_default())
    };
    let projects: HashMap<i64, _> = projects.into_iter().map(|p| (p.id, p)).collect();
    let dropped = DROPPED.lock().unwrap();
    let mut out: Vec<Cache> = live
        .into_iter()
        .filter(|c| !dropped.contains(&c.session_id))
        .filter_map(|c| {
            let p = c.project_id.and_then(|id| projects.get(&id));
            if s.cache_projects == "picked" && !p.is_some_and(|p| s.cache_picked.contains(&p.path)) {
                return None;
            }
            let hidden = s.hide_project_names || p.is_none_or(|p| p.hidden);
            Some(Cache {
                session_id: c.session_id,
                project: if hidden { String::new() } else { p.map(|p| p.name.clone()).unwrap_or_default() },
                branch: c.branch.filter(|_| !hidden),
                model: c.model,
                client: c.client,
                ttl_ms: c.ttl_ms,
                expires_ms: c.expires_ms,
                started_ms: c.started_ms,
                tokens: c.tokens,
                refresh_usd: c.refresh_usd,
                rewrite_usd: c.rewrite_usd,
                ..Cache::default()
            })
        })
        .collect();
    drop(dropped);
    crate::refresh::annotate(state, s, &mut out, now_ms);
    out
}

/// Where a conversation runs, for a notification: "Terminal", "Claude desktop" and so on.
pub fn client_name(client: Option<&str>, tr: bool) -> Option<&'static str> {
    Some(match (client?, tr) {
        ("cli", _) => "Terminal",
        ("claude-desktop", true) => "Claude masaüstü",
        ("claude-desktop", false) => "Claude desktop",
        ("claude-vscode", _) => "VS Code",
        _ => return None,
    })
}

/// "notes · main": the project and what tells the conversation apart; `None` without a name.
pub fn label(c: &Cache, tr: bool) -> Option<String> {
    if c.project.is_empty() {
        return None;
    }
    let second = c.branch.clone().or_else(|| client_name(c.client.as_deref(), tr).map(str::to_owned));
    Some(second.map_or(c.project.clone(), |b| format!("{} · {b}", c.project)))
}

#[tauri::command]
pub async fn live_caches(state: State<'_, AppState>) -> Result<Vec<Cache>, String> {
    let s = state.settings.read().unwrap().clone();
    Ok(followed(&state, &s, chrono::Utc::now().timestamp_millis()))
}

/// Stops following one conversation; the others and new ones are still followed.
#[tauri::command]
pub async fn drop_cache(window: WebviewWindow, app: AppHandle, session_id: String) -> Result<(), String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let mut dropped = DROPPED.lock().unwrap();
    if !dropped.contains(&session_id) {
        dropped.push(session_id);
    }
    drop(dropped);
    crate::tray::refresh_soon();
    let _ = app.emit("data-changed", ());
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ProjectChoice {
    pub id: i64,
    pub path: String,
    /// Empty while project names are hidden or this one is.
    pub name: String,
    pub hidden: bool,
}

/// The projects to pick from: those Claude Code was used in over the last 30 days, the most recent
/// first, then any picked earlier.
#[tauri::command]
pub async fn cache_project_choices(window: WebviewWindow, state: State<'_, AppState>) -> Result<Vec<ProjectChoice>, String> {
    if window.label() != crate::windows::MAIN {
        return Err("not_allowed".into());
    }
    let (picked, hide_all) = {
        let s = state.settings.read().unwrap();
        (s.cache_picked.clone(), s.hide_project_names)
    };
    let store = state.db();
    let recent = store.projects_used_since(Tool::ClaudeCode, chrono::Utc::now().timestamp_millis() - MONTH_MS).map_err(|e| e.to_string())?;
    let projects: HashMap<i64, _> = store.projects().map_err(|e| e.to_string())?.into_iter().map(|p| (p.id, p)).collect();
    let choice = |p: &tracker_core::store::ProjectRow| {
        let hidden = hide_all || p.hidden;
        ProjectChoice { id: p.id, path: p.path.clone(), name: if hidden { String::new() } else { p.name.clone() }, hidden }
    };
    let mut out: Vec<ProjectChoice> = recent.iter().filter_map(|id| projects.get(id)).map(choice).collect();
    for p in projects.values() {
        if picked.contains(&p.path) && !out.iter().any(|o| o.path == p.path) {
            out.push(choice(p));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(project: &str, branch: Option<&str>, client: Option<&str>) -> Cache {
        Cache { session_id: "s".into(), project: project.into(), branch: branch.map(Into::into), client: client.map(Into::into), ..Cache::default() }
    }

    #[test]
    fn a_cache_is_named_by_its_project_and_branch_or_where_it_runs() {
        assert_eq!(label(&cache("notes", Some("main"), Some("cli")), true).as_deref(), Some("notes · main"));
        assert_eq!(label(&cache("notes", None, Some("claude-desktop")), true).as_deref(), Some("notes · Claude masaüstü"));
        assert_eq!(label(&cache("notes", None, Some("claude-desktop")), false).as_deref(), Some("notes · Claude desktop"));
        assert_eq!(label(&cache("notes", None, Some("sdk-ts")), false).as_deref(), Some("notes"));
        assert_eq!(label(&cache("", Some("main"), Some("cli")), false), None, "a hidden project");
    }
}
