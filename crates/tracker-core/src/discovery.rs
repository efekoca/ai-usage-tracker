//! Finds the data sources installed on *this* machine for *this* user. Nothing is hardcoded:
//! roots come from Known Folders (`dirs`), tool-specific environment variables
//! (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`) and user-added paths. MSIX packages expose the same
//! folder under two paths, so files are de-duplicated by their NTFS file id.

use crate::sources::ParserKind;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceId {
    ClaudeCode,
    Cowork,
    ClaudeDesktop,
    Codex,
    ChatgptDesktop,
}

impl SourceId {
    pub const ALL: [SourceId; 5] =
        [SourceId::ClaudeCode, SourceId::Cowork, SourceId::ClaudeDesktop, SourceId::Codex, SourceId::ChatgptDesktop];

    pub fn as_str(self) -> &'static str {
        match self {
            SourceId::ClaudeCode => "claude_code",
            SourceId::Cowork => "cowork",
            SourceId::ClaudeDesktop => "claude_desktop",
            SourceId::Codex => "codex",
            SourceId::ChatgptDesktop => "chatgpt_desktop",
        }
    }
    /// Whether usage can actually be read (ChatGPT desktop is detection-only).
    pub fn supported(self) -> bool {
        !matches!(self, SourceId::ChatgptDesktop)
    }
}

/// Everything discovery needs from the outside world — injectable for tests.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    /// `%APPDATA%` (Roaming)
    pub roaming: Option<PathBuf>,
    /// `%LOCALAPPDATA%`
    pub local: Option<PathBuf>,
    pub claude_config_dir: Option<String>,
    pub codex_home: Option<String>,
}

impl Env {
    pub fn from_system() -> Env {
        let var = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
        Env {
            home: dirs::home_dir(),
            roaming: dirs::data_dir(),
            local: dirs::data_local_dir(),
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            codex_home: var("CODEX_HOME"),
        }
    }
}

/// User-added roots from settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtraPaths {
    #[serde(default)]
    pub claude_config_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub codex_homes: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStatus {
    pub id: SourceId,
    pub found: bool,
    pub supported: bool,
    pub roots: Vec<PathBuf>,
    pub file_count: usize,
    /// Cowork with no local logs but with cloud sessions: their token counts live on the server.
    pub cloud_only: bool,
}

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub parser: ParserKind,
    pub source: SourceId,
    /// Stable identity of the underlying file (volume + file index on NTFS).
    pub file_id: String,
}

/// Claude Code config roots: `CLAUDE_CONFIG_DIR` (comma/semicolon separated) or `~/.claude`,
/// plus the legacy `~/.config/claude`, plus user extras.
pub fn claude_config_roots(env: &Env, extra: &ExtraPaths) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(list) = &env.claude_config_dir {
        // a folder whose name holds a comma (`C:\Users\Doe, John\.claude`) is one root, not two
        if Path::new(list.trim()).is_dir() {
            v.push(PathBuf::from(list.trim()));
        } else {
            v.extend(list.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from));
        }
    } else if let Some(h) = &env.home {
        v.push(h.join(".claude"));
        v.push(h.join(".config").join("claude"));
    }
    v.extend(extra.claude_config_dirs.iter().cloned());
    v
}

pub fn codex_homes(env: &Env, extra: &ExtraPaths) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = &env.codex_home {
        v.push(PathBuf::from(h));
    } else if let Some(h) = &env.home {
        v.push(h.join(".codex"));
    }
    v.extend(extra.codex_homes.iter().cloned());
    v
}

/// `%APPDATA%\Claude` and the MSIX-virtualised `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude`.
pub fn claude_desktop_dirs(env: &Env) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(r) = &env.roaming {
        v.push(r.join("Claude"));
    }
    if let Some(l) = &env.local {
        for pkg in packages_matching(l, "Claude_") {
            v.push(pkg.join("LocalCache").join("Roaming").join("Claude"));
        }
    }
    v
}

fn packages_matching(local: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(local.join("Packages")) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn chatgpt_desktop_dirs(env: &Env) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(l) = &env.local {
        v.extend(packages_matching(l, "OpenAI.ChatGPT"));
        v.push(l.join("Programs").join("ChatGPT"));
    }
    if let Some(r) = &env.roaming {
        v.push(r.join("ChatGPT"));
    }
    v
}

fn jsonl_under(dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "jsonl"))
        .map(|e| e.into_path())
}

/// Lists every file to ingest for the enabled sources, de-duplicated by physical file identity.
pub fn enumerate_files(env: &Env, extra: &ExtraPaths, enabled: &HashSet<SourceId>) -> Vec<DiscoveredFile> {
    let mut candidates: Vec<(PathBuf, ParserKind, SourceId)> = Vec::new();

    if enabled.contains(&SourceId::ClaudeCode) {
        for root in claude_config_roots(env, extra) {
            for p in jsonl_under(&root.join("projects")) {
                candidates.push((p, ParserKind::ClaudeCodeJsonl, SourceId::ClaudeCode));
            }
        }
    }
    let desktop = claude_desktop_dirs(env);
    if enabled.contains(&SourceId::Cowork) {
        for d in &desktop {
            let base = d.join("local-agent-mode-sessions");
            for e in WalkDir::new(&base).max_depth(4).into_iter().filter_map(Result::ok) {
                let name = e.file_name().to_string_lossy();
                if e.file_type().is_dir() && name == ".claude" {
                    for p in jsonl_under(&e.path().join("projects")) {
                        candidates.push((p, ParserKind::CoworkJsonl, SourceId::Cowork));
                    }
                } else if e.file_type().is_file() && name == "audit.jsonl" {
                    candidates.push((e.into_path(), ParserKind::CoworkAudit, SourceId::Cowork));
                }
            }
        }
    }
    if enabled.contains(&SourceId::ClaudeDesktop) {
        for d in &desktop {
            let p = d.join("plan-usage-history.json");
            if p.is_file() {
                candidates.push((p, ParserKind::ClaudePlanHistory, SourceId::ClaudeDesktop));
            }
        }
    }
    if enabled.contains(&SourceId::Codex) {
        for home in codex_homes(env, extra) {
            for sub in ["sessions", "archived_sessions"] {
                for p in jsonl_under(&home.join(sub)) {
                    candidates.push((p, ParserKind::CodexRollout, SourceId::Codex));
                }
            }
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(candidates.len());
    for (path, parser, source) in candidates {
        let id = file_identity(&path);
        if seen.insert(id.clone()) {
            out.push(DiscoveredFile { path, parser, source, file_id: id });
        }
    }
    out
}

/// Physical identity of a file; falls back to the lower-cased path when unavailable.
pub fn file_identity(path: &Path) -> String {
    match file_id::get_file_id(path) {
        Ok(id) => format!("{id:?}"),
        Err(_) => format!("path:{}", path.to_string_lossy().to_lowercase()),
    }
}

/// Onboarding view: which tools exist on this machine, regardless of enabled state.
pub fn detect(env: &Env, extra: &ExtraPaths) -> Vec<SourceStatus> {
    let all: HashSet<SourceId> = SourceId::ALL.into_iter().collect();
    let files = enumerate_files(env, extra, &all);
    let count = |s: SourceId| files.iter().filter(|f| f.source == s).count();
    let existing = |v: Vec<PathBuf>| v.into_iter().filter(|p| p.exists()).collect::<Vec<_>>();
    let desktop = existing(claude_desktop_dirs(env));

    SourceId::ALL
        .into_iter()
        .map(|id| {
            let roots = match id {
                SourceId::ClaudeCode => existing(claude_config_roots(env, extra)),
                SourceId::Cowork => existing(desktop.iter().map(|d| d.join("local-agent-mode-sessions")).collect()),
                SourceId::ClaudeDesktop => desktop.clone(),
                SourceId::Codex => existing(codex_homes(env, extra)),
                SourceId::ChatgptDesktop => existing(chatgpt_desktop_dirs(env)),
            };
            let file_count = count(id);
            // A tool counts as found if its data directory exists, even with no usage yet.
            let found = !roots.is_empty();
            let cloud_only = id == SourceId::Cowork && file_count == 0 && roots.iter().any(|r| has_cloud_sessions(r));
            SourceStatus { id, found, supported: id.supported(), roots, file_count, cloud_only }
        })
        .collect()
}

/// The desktop app lists cloud Cowork sessions in `<org>/<user>/remote-session-spaces.json`.
fn has_cloud_sessions(sessions_dir: &Path) -> bool {
    WalkDir::new(sessions_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.file_name() == "remote-session-spaces.json")
        .any(|e| {
            let Ok(text) = std::fs::read_to_string(e.path()) else { return false };
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("entries")?.as_array().map(|a| !a.is_empty()))
                .unwrap_or(false)
        })
}
