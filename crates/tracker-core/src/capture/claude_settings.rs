//! Opt-in, reversible edits to the user's Claude Code `settings.json`:
//! * `statusLine` → our bridge command (the previous value is kept and chained),
//! * `env` → OpenTelemetry *log* export to our loopback receiver.
//!
//! Rules: a backup is written before every change, unrelated keys and their order are kept,
//! nothing is installed over an existing telemetry setup, and a revert only touches values
//! that are still exactly what we wrote (if the user changed them since, they are left alone).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings file is not valid JSON; it was not changed ({0})")]
    Unparseable(String),
    #[error("an existing telemetry configuration was found ({0}); it was not changed")]
    TelemetryConflict(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatuslineChange {
    pub settings_file: PathBuf,
    /// The user's own `statusLine` before we installed ours (chained by the bridge).
    pub previous: Option<Value>,
    pub installed: Value,
    pub backup: PathBuf,
    pub installed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OtelChange {
    pub settings_file: PathBuf,
    pub port: u16,
    /// Exactly the env entries we added.
    pub added: BTreeMap<String, String>,
    pub backup: PathBuf,
    pub installed_at_ms: i64,
}

/// What this app has changed outside its own folder. Persisted as `capture/state.json` so the
/// status-line bridge and the uninstaller can read it without the database.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CaptureState {
    #[serde(default)]
    pub statusline: Option<StatuslineChange>,
    #[serde(default)]
    pub otel: Option<OtelChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevertOutcome {
    Restored,
    /// The user edited the value after we installed it, so it was left as is.
    LeftUserValue,
    NothingToDo,
}

pub fn state_file(data_dir: &Path) -> PathBuf {
    data_dir.join("capture").join("state.json")
}

pub fn load_state(data_dir: &Path) -> CaptureState {
    fs::read_to_string(state_file(data_dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_state(data_dir: &Path, st: &CaptureState) -> std::io::Result<()> {
    let p = state_file(data_dir);
    fs::create_dir_all(p.parent().unwrap())?;
    write_atomic(&p, &serde_json::to_string_pretty(st).unwrap_or_default())
}

/// Claude Code's user settings file for a config root (`CLAUDE_CONFIG_DIR` or `~/.claude`).
pub fn settings_path(config_root: &Path) -> PathBuf {
    config_root.join("settings.json")
}

fn read_doc(path: &Path) -> Result<Map<String, Value>, SettingsError> {
    match fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Ok(Map::new()),
        Ok(s) => {
            let s = s.trim_start_matches('\u{feff}');
            match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(m)) => Ok(m),
                Ok(_) => Err(SettingsError::Unparseable("top level is not an object".into())),
                Err(e) => Err(SettingsError::Unparseable(e.to_string())),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.into()),
    }
}

fn write_doc(path: &Path, doc: &Map<String, Value>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_atomic(path, &(serde_json::to_string_pretty(&Value::Object(doc.clone())).unwrap_or_default() + "\n"))
}

fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("aiut-tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

fn backup(path: &Path, data_dir: &Path, now_ms: i64) -> std::io::Result<PathBuf> {
    let dir = data_dir.join("backups");
    fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("claude-settings-{now_ms}.json"));
    match fs::copy(path, &dest) {
        Ok(_) => {}
        // nothing to back up: record that the file did not exist
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::write(&dest, "")?,
        Err(e) => return Err(e),
    }
    Ok(dest)
}

/// The `statusLine` value we install.
pub fn statusline_value(command: &str) -> Value {
    serde_json::json!({ "type": "command", "command": command, "padding": 0 })
}

pub fn install_statusline(settings_file: &Path, data_dir: &Path, command: &str, now_ms: i64) -> Result<StatuslineChange, SettingsError> {
    let mut doc = read_doc(settings_file)?;
    let installed = statusline_value(command);
    let previous = doc.get("statusLine").cloned().filter(|v| v != &installed);
    let backup = backup(settings_file, data_dir, now_ms)?;
    doc.insert("statusLine".into(), installed.clone());
    write_doc(settings_file, &doc)?;
    Ok(StatuslineChange { settings_file: settings_file.to_owned(), previous, installed, backup, installed_at_ms: now_ms })
}

pub fn revert_statusline(change: &StatuslineChange) -> Result<RevertOutcome, SettingsError> {
    let mut doc = read_doc(&change.settings_file)?;
    match doc.get("statusLine") {
        Some(v) if v == &change.installed => {}
        Some(_) => return Ok(RevertOutcome::LeftUserValue),
        None => return Ok(RevertOutcome::NothingToDo),
    }
    match &change.previous {
        Some(prev) => {
            doc.insert("statusLine".into(), prev.clone());
        }
        None => {
            doc.remove("statusLine");
        }
    }
    write_doc(&change.settings_file, &doc)?;
    Ok(RevertOutcome::Restored)
}

/// The env entries for OTLP/HTTP-JSON *log* export to `127.0.0.1:port`. Metrics/traces and
/// every content option (prompts, tool details, responses) are left off.
pub fn otel_env(port: u16) -> BTreeMap<String, String> {
    [
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1".to_string()),
        ("OTEL_LOGS_EXPORTER", "otlp".to_string()),
        ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json".to_string()),
        ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", format!("http://127.0.0.1:{port}/v1/logs")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

fn is_telemetry_key(k: &str) -> bool {
    k == "CLAUDE_CODE_ENABLE_TELEMETRY" || k.starts_with("OTEL_")
}

/// Telemetry variables already present in the process environment (set by the user's shell or
/// system) — these would compete with ours, so installation is refused.
pub fn conflicting_process_env() -> Vec<String> {
    std::env::vars().map(|(k, _)| k).filter(|k| is_telemetry_key(k)).collect()
}

pub fn install_otel(settings_file: &Path, data_dir: &Path, port: u16, now_ms: i64) -> Result<OtelChange, SettingsError> {
    let mut doc = read_doc(settings_file)?;
    let env = doc.entry("env").or_insert_with(|| Value::Object(Map::new()));
    let Some(env) = env.as_object_mut() else {
        return Err(SettingsError::Unparseable("`env` is not an object".into()));
    };
    let existing: Vec<String> = env.keys().filter(|k| is_telemetry_key(k)).cloned().collect();
    if !existing.is_empty() {
        return Err(SettingsError::TelemetryConflict(existing.join(", ")));
    }
    let added = otel_env(port);
    let backup = backup(settings_file, data_dir, now_ms)?;
    for (k, v) in &added {
        env.insert(k.clone(), Value::String(v.clone()));
    }
    write_doc(settings_file, &doc)?;
    Ok(OtelChange { settings_file: settings_file.to_owned(), port, added, backup, installed_at_ms: now_ms })
}

pub fn revert_otel(change: &OtelChange) -> Result<RevertOutcome, SettingsError> {
    let mut doc = read_doc(&change.settings_file)?;
    let Some(env) = doc.get_mut("env").and_then(Value::as_object_mut) else { return Ok(RevertOutcome::NothingToDo) };
    let mut removed = 0;
    let mut kept = 0;
    for (k, v) in &change.added {
        match env.get(k) {
            Some(Value::String(cur)) if cur == v => {
                env.remove(k);
                removed += 1;
            }
            Some(_) => kept += 1,
            None => {}
        }
    }
    if env.is_empty() {
        doc.remove("env");
    }
    if removed > 0 {
        write_doc(&change.settings_file, &doc)?;
    }
    Ok(match (removed, kept) {
        (_, k) if k > 0 => RevertOutcome::LeftUserValue,
        (0, _) => RevertOutcome::NothingToDo,
        _ => RevertOutcome::Restored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup(initial: Option<&str>) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join(".claude");
        fs::create_dir_all(&cfg).unwrap();
        let file = settings_path(&cfg);
        if let Some(s) = initial {
            fs::write(&file, s).unwrap();
        }
        let data = dir.path().join("data");
        (dir, file, data)
    }

    fn read(p: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn statusline_install_keeps_other_keys_in_order_and_reverts_to_the_users_own() {
        let user_sl = json!({"type": "command", "command": "~/.claude/mine.sh"});
        let initial = json!({"theme": "dark", "statusLine": user_sl, "effortLevel": "high"}).to_string();
        let (_d, file, data) = setup(Some(&initial));
        let ch = install_statusline(&file, &data, "aiut --statusline", 1).unwrap();
        assert_eq!(ch.previous, Some(user_sl.clone()));
        let after = read(&file);
        assert_eq!(after["statusLine"]["command"], "aiut --statusline");
        let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["theme", "statusLine", "effortLevel"]);
        assert!(ch.backup.exists());

        assert_eq!(revert_statusline(&ch).unwrap(), RevertOutcome::Restored);
        assert_eq!(read(&file), json!({"theme": "dark", "statusLine": user_sl, "effortLevel": "high"}));
    }

    #[test]
    fn statusline_revert_removes_the_key_when_there_was_none_and_respects_user_edits() {
        let (_d, file, data) = setup(None);
        let ch = install_statusline(&file, &data, "aiut --statusline", 1).unwrap();
        assert_eq!(revert_statusline(&ch).unwrap(), RevertOutcome::Restored);
        assert!(read(&file).get("statusLine").is_none());

        let ch = install_statusline(&file, &data, "aiut --statusline", 2).unwrap();
        let mut doc = read(&file);
        doc["statusLine"] = json!({"type": "command", "command": "user-changed"});
        fs::write(&file, doc.to_string()).unwrap();
        assert_eq!(revert_statusline(&ch).unwrap(), RevertOutcome::LeftUserValue);
        assert_eq!(read(&file)["statusLine"]["command"], "user-changed");
    }

    #[test]
    fn otel_install_refuses_existing_telemetry_and_reverts_only_its_own_keys() {
        let (_d, file, data) = setup(Some(r#"{"env": {"OTEL_METRICS_EXPORTER": "console"}}"#));
        assert!(matches!(install_otel(&file, &data, 43180, 1), Err(SettingsError::TelemetryConflict(_))));
        assert_eq!(read(&file), json!({"env": {"OTEL_METRICS_EXPORTER": "console"}}));

        let (_d, file, data) = setup(Some(r#"{"env": {"MY_VAR": "1"}, "model": "opus"}"#));
        let ch = install_otel(&file, &data, 43180, 1).unwrap();
        let env = read(&file)["env"].clone();
        assert_eq!(env["MY_VAR"], "1");
        assert_eq!(env["OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"], "http://127.0.0.1:43180/v1/logs");
        assert!(env.get("OTEL_LOG_USER_PROMPTS").is_none());
        assert_eq!(revert_otel(&ch).unwrap(), RevertOutcome::Restored);
        assert_eq!(read(&file), json!({"env": {"MY_VAR": "1"}, "model": "opus"}));
    }

    #[test]
    fn unparseable_settings_are_never_touched() {
        let (_d, file, data) = setup(Some("{ // comments are not JSON\n }"));
        assert!(matches!(install_statusline(&file, &data, "x", 1), Err(SettingsError::Unparseable(_))));
        assert_eq!(fs::read_to_string(&file).unwrap(), "{ // comments are not JSON\n }");
    }

    #[test]
    fn state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let st = CaptureState { statusline: None, otel: None };
        save_state(dir.path(), &st).unwrap();
        assert_eq!(load_state(dir.path()), st);
        assert_eq!(load_state(&dir.path().join("missing")), CaptureState::default());
    }
}
