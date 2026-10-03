//! Opt-in, reversible edits to the user's Claude Code `settings.json`:
//! * `statusLine` → our bridge command (the previous value is kept and chained),
//! * `env` → OpenTelemetry *log* export to our loopback receiver.
//!
//! Rules: a backup is written before every change, unrelated keys and their order are kept,
//! nothing is installed over an existing telemetry setup or a file that changed meanwhile, and
//! a revert only touches values that are still exactly what we wrote.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("settings file is not valid JSON; it was not changed ({0})")]
    Unparseable(String),
    #[error("an existing telemetry configuration was found ({0}); it was not changed")]
    TelemetryConflict(String),
    #[error("the settings file changed while it was being edited; it was not changed")]
    Changed,
    #[error("the record of earlier changes is unreadable ({0}); nothing was changed")]
    StateUnreadable(String),
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

pub const HEADERS_KEY: &str = "OTEL_EXPORTER_OTLP_HEADERS";

impl OtelChange {
    /// `None` for installs made before tokens existed.
    pub fn token(&self) -> Option<&str> {
        self.added.get(HEADERS_KEY)?.strip_prefix(super::otlp::AUTH_HEADER)?.strip_prefix('=')
    }
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
    read_state(data_dir).unwrap_or_default()
}

/// An unreadable record is an error, so the undo information in it is never silently replaced.
pub fn read_state(data_dir: &Path) -> Result<CaptureState, SettingsError> {
    match fs::read_to_string(state_file(data_dir)) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| SettingsError::StateUnreadable(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CaptureState::default()),
        Err(e) => Err(SettingsError::StateUnreadable(e.to_string())),
    }
}

pub fn save_state(data_dir: &Path, st: &CaptureState) -> std::io::Result<()> {
    let p = state_file(data_dir);
    fs::create_dir_all(p.parent().unwrap())?;
    write_atomic(&p, &serde_json::to_string_pretty(st).unwrap_or_default())
}

pub fn settings_path(config_root: &Path) -> PathBuf {
    config_root.join("settings.json")
}

fn read_doc(path: &Path) -> Result<(Map<String, Value>, Option<String>), SettingsError> {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Map::new(), None)),
        Err(e) => return Err(e.into()),
    };
    let s = raw.trim_start_matches('\u{feff}');
    if s.trim().is_empty() {
        return Ok((Map::new(), Some(raw)));
    }
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Ok((m, Some(raw))),
        Ok(_) => Err(SettingsError::Unparseable("top level is not an object".into())),
        Err(e) => Err(SettingsError::Unparseable(e.to_string())),
    }
}

/// Refuses when the file no longer holds `original`, so an edit made meanwhile is not lost.
fn write_doc(path: &Path, doc: &Map<String, Value>, original: &Option<String>) -> Result<(), SettingsError> {
    let current = match fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if &current != original {
        return Err(SettingsError::Changed);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let target = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_owned(),
    };
    write_atomic(&target, &(serde_json::to_string_pretty(&Value::Object(doc.clone())).unwrap_or_default() + "\n"))?;
    Ok(())
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.{}-{}.aiut-tmp", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    let r = fs::write(&tmp, content).and_then(|_| fs::rename(&tmp, path));
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
}

fn backup(path: &Path, data_dir: &Path, now_ms: i64) -> std::io::Result<PathBuf> {
    let dir = data_dir.join("backups");
    fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("claude-settings-{now_ms}.json"));
    // read + write rather than copy, so the backup never inherits a read-only attribute
    match fs::read(path) {
        Ok(bytes) => fs::write(&dest, bytes)?,
        // nothing to back up: record that the file did not exist
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::write(&dest, "")?,
        Err(e) => return Err(e),
    }
    Ok(dest)
}

/// The record is rolled back if the write fails, so neither exists without the other.
fn record_then_write<T: Clone>(
    st: &mut CaptureState,
    data_dir: &Path,
    slot: fn(&mut CaptureState) -> &mut Option<T>,
    change: T,
    backup: &Path,
    write: impl FnOnce() -> Result<(), SettingsError>,
) -> Result<T, SettingsError> {
    let before = slot(st).replace(change.clone());
    let result = save_state(data_dir, st).map_err(SettingsError::from).and_then(|_| write());
    if let Err(e) = result {
        *slot(st) = before;
        let _ = save_state(data_dir, st);
        let _ = fs::remove_file(backup);
        return Err(e);
    }
    Ok(change)
}

pub fn statusline_value(command: &str) -> Value {
    serde_json::json!({ "type": "command", "command": command, "padding": 0 })
}

pub fn install_statusline(st: &mut CaptureState, data_dir: &Path, settings_file: &Path, command: &str, now_ms: i64) -> Result<StatuslineChange, SettingsError> {
    let (mut doc, original) = read_doc(settings_file)?;
    let installed = statusline_value(command);
    let previous = doc.get("statusLine").cloned().filter(|v| v != &installed);
    let backup = backup(settings_file, data_dir, now_ms)?;
    doc.insert("statusLine".into(), installed.clone());
    let change = StatuslineChange { settings_file: settings_file.to_owned(), previous, installed, backup: backup.clone(), installed_at_ms: now_ms };
    record_then_write(st, data_dir, |s| &mut s.statusline, change, &backup, || write_doc(settings_file, &doc, &original))
}

pub fn revert_statusline(change: &StatuslineChange) -> Result<RevertOutcome, SettingsError> {
    let (mut doc, original) = read_doc(&change.settings_file)?;
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
    write_doc(&change.settings_file, &doc, &original)?;
    Ok(RevertOutcome::Restored)
}

/// The record is dropped only once the revert worked, so a failed undo can be retried.
pub fn uninstall_statusline(st: &mut CaptureState, data_dir: &Path) -> Result<RevertOutcome, SettingsError> {
    let Some(ch) = st.statusline.clone() else { return Ok(RevertOutcome::NothingToDo) };
    let out = revert_statusline(&ch)?;
    st.statusline = None;
    save_state(data_dir, st)?;
    let _ = fs::remove_file(&ch.backup);
    Ok(out)
}

/// Log export only, with no content options; `token` tells Claude Code from other local programs.
pub fn otel_env(port: u16, token: &str) -> BTreeMap<String, String> {
    [
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1".to_string()),
        ("OTEL_LOGS_EXPORTER", "otlp".to_string()),
        ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json".to_string()),
        ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", format!("http://127.0.0.1:{port}/v1/logs")),
        (HEADERS_KEY, format!("{}={token}", super::otlp::AUTH_HEADER)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// std's `RandomState` is keyed from the OS random generator: enough for a loopback secret.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

fn is_telemetry_key(k: &str) -> bool {
    k == "CLAUDE_CODE_ENABLE_TELEMETRY" || k.starts_with("OTEL_")
}

/// Telemetry variables already present in the process environment (set by the user's shell or
/// system) — these would compete with ours, so installation is refused.
pub fn conflicting_process_env() -> Vec<String> {
    std::env::vars().map(|(k, _)| k).filter(|k| is_telemetry_key(k)).collect()
}

/// A headers helper counts too: Claude Code would send its output to our receiver.
fn telemetry_settings(doc: &Map<String, Value>) -> Vec<String> {
    let mut v: Vec<String> = doc.get("env").and_then(Value::as_object).map(|e| e.keys().filter(|k| is_telemetry_key(k)).cloned().collect()).unwrap_or_default();
    if doc.contains_key("otelHeadersHelper") {
        v.push("otelHeadersHelper".into());
    }
    v
}

/// Managed (organisation) settings take precedence over the user's file.
pub fn managed_conflicts(docs: &[Value]) -> Vec<String> {
    let mut v: Vec<String> = docs.iter().filter_map(Value::as_object).flat_map(telemetry_settings).collect();
    v.sort();
    v.dedup();
    v
}

pub fn managed_files(program_files: &Path) -> Vec<Value> {
    let dir = program_files.join("ClaudeCode");
    let mut paths = vec![dir.join("managed-settings.json")];
    let mut drop_ins: Vec<PathBuf> = fs::read_dir(dir.join("managed-settings.d"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    drop_ins.sort();
    paths.extend(drop_ins);
    paths.iter().filter_map(|p| fs::read_to_string(p).ok()).filter_map(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).collect()
}

pub fn install_otel(st: &mut CaptureState, data_dir: &Path, settings_file: &Path, port: u16, token: &str, now_ms: i64) -> Result<OtelChange, SettingsError> {
    let (mut doc, original) = read_doc(settings_file)?;
    if doc.get("env").is_some_and(|e| !e.is_object()) {
        return Err(SettingsError::Unparseable("`env` is not an object".into()));
    }
    let existing = telemetry_settings(&doc);
    if !existing.is_empty() {
        return Err(SettingsError::TelemetryConflict(existing.join(", ")));
    }
    let added = otel_env(port, token);
    let backup = backup(settings_file, data_dir, now_ms)?;
    let env = doc.entry("env").or_insert_with(|| Value::Object(Map::new())).as_object_mut().expect("checked above");
    for (k, v) in &added {
        env.insert(k.clone(), Value::String(v.clone()));
    }
    let change = OtelChange { settings_file: settings_file.to_owned(), port, added, backup: backup.clone(), installed_at_ms: now_ms };
    record_then_write(st, data_dir, |s| &mut s.otel, change, &backup, || write_doc(settings_file, &doc, &original))
}

/// Only while our entries are still exactly as written.
pub fn add_otel_token(st: &mut CaptureState, data_dir: &Path, token: &str) -> Result<bool, SettingsError> {
    let Some(ch) = st.otel.clone().filter(|c| c.token().is_none()) else { return Ok(false) };
    let (mut doc, original) = read_doc(&ch.settings_file)?;
    if doc.contains_key("otelHeadersHelper") {
        return Ok(false);
    }
    let Some(env) = doc.get_mut("env").and_then(Value::as_object_mut) else { return Ok(false) };
    let ours = ch.added.iter().all(|(k, v)| env.get(k).and_then(Value::as_str) == Some(v.as_str()));
    if !ours || env.contains_key(HEADERS_KEY) {
        return Ok(false);
    }
    let header = otel_env(ch.port, token).remove(HEADERS_KEY).unwrap_or_default();
    env.insert(HEADERS_KEY.into(), Value::String(header.clone()));
    let mut next = ch.clone();
    next.added.insert(HEADERS_KEY.into(), header);
    let before = st.otel.replace(next);
    let result = save_state(data_dir, st).map_err(SettingsError::from).and_then(|_| write_doc(&ch.settings_file, &doc, &original));
    if let Err(e) = result {
        st.otel = before;
        let _ = save_state(data_dir, st);
        return Err(e);
    }
    Ok(true)
}

pub fn revert_otel(change: &OtelChange) -> Result<RevertOutcome, SettingsError> {
    let (mut doc, original) = read_doc(&change.settings_file)?;
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
        write_doc(&change.settings_file, &doc, &original)?;
    }
    Ok(match (removed, kept) {
        (_, k) if k > 0 => RevertOutcome::LeftUserValue,
        (0, _) => RevertOutcome::NothingToDo,
        _ => RevertOutcome::Restored,
    })
}

pub fn uninstall_otel(st: &mut CaptureState, data_dir: &Path) -> Result<RevertOutcome, SettingsError> {
    let Some(ch) = st.otel.clone() else { return Ok(RevertOutcome::NothingToDo) };
    let out = revert_otel(&ch)?;
    st.otel = None;
    save_state(data_dir, st)?;
    let _ = fs::remove_file(&ch.backup);
    Ok(out)
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
        let mut st = CaptureState::default();
        let ch = install_statusline(&mut st, &data, &file, "aiut --statusline", 1).unwrap();
        assert_eq!(ch.previous, Some(user_sl.clone()));
        assert_eq!(read_state(&data).unwrap().statusline, Some(ch.clone()));
        let after = read(&file);
        assert_eq!(after["statusLine"]["command"], "aiut --statusline");
        let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["theme", "statusLine", "effortLevel"]);
        assert!(ch.backup.exists());

        assert_eq!(uninstall_statusline(&mut st, &data).unwrap(), RevertOutcome::Restored);
        assert_eq!(read(&file), json!({"theme": "dark", "statusLine": user_sl, "effortLevel": "high"}));
        assert_eq!(read_state(&data).unwrap(), CaptureState::default());
        assert!(!ch.backup.exists(), "the backup is not kept once the change is undone");
    }

    #[test]
    fn statusline_revert_removes_the_key_when_there_was_none_and_respects_user_edits() {
        let (_d, file, data) = setup(None);
        let mut st = CaptureState::default();
        install_statusline(&mut st, &data, &file, "aiut --statusline", 1).unwrap();
        assert_eq!(uninstall_statusline(&mut st, &data).unwrap(), RevertOutcome::Restored);
        assert!(read(&file).get("statusLine").is_none());

        let ch = install_statusline(&mut st, &data, &file, "aiut --statusline", 2).unwrap();
        let mut doc = read(&file);
        doc["statusLine"] = json!({"type": "command", "command": "user-changed"});
        fs::write(&file, doc.to_string()).unwrap();
        assert_eq!(revert_statusline(&ch).unwrap(), RevertOutcome::LeftUserValue);
        assert_eq!(read(&file)["statusLine"]["command"], "user-changed");
    }

    #[test]
    fn otel_install_refuses_existing_telemetry_and_reverts_only_its_own_keys() {
        let mut st = CaptureState::default();
        let (_d, file, data) = setup(Some(r#"{"env": {"OTEL_METRICS_EXPORTER": "console"}}"#));
        assert!(matches!(install_otel(&mut st, &data, &file, 43180, "t", 1), Err(SettingsError::TelemetryConflict(_))));
        assert_eq!(read(&file), json!({"env": {"OTEL_METRICS_EXPORTER": "console"}}));
        assert_eq!(st, CaptureState::default());

        let (_d, file, data) = setup(Some(r#"{"env": {"MY_VAR": "1"}, "model": "opus"}"#));
        let ch = install_otel(&mut st, &data, &file, 43180, "tok", 1).unwrap();
        assert_eq!(ch.token(), Some("tok"));
        let env = read(&file)["env"].clone();
        assert_eq!(env["MY_VAR"], "1");
        assert_eq!(env["OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"], "http://127.0.0.1:43180/v1/logs");
        assert_eq!(env[HEADERS_KEY], "x-aiut-token=tok");
        assert!(env.get("OTEL_LOG_USER_PROMPTS").is_none());
        assert_eq!(uninstall_otel(&mut st, &data).unwrap(), RevertOutcome::Restored);
        assert_eq!(read(&file), json!({"env": {"MY_VAR": "1"}, "model": "opus"}));
        assert!(st.otel.is_none());
    }

    #[test]
    fn a_headers_helper_is_a_conflict_in_the_users_file_and_in_managed_settings() {
        let mut st = CaptureState::default();
        let (_d, file, data) = setup(Some(r#"{"otelHeadersHelper": "/bin/gen-headers.sh"}"#));
        assert!(matches!(install_otel(&mut st, &data, &file, 43180, "t", 1), Err(SettingsError::TelemetryConflict(k)) if k == "otelHeadersHelper"));
        let docs = [json!({"env": {"OTEL_LOGS_EXPORTER": "otlp", "OTHER": "1"}}), json!({"otelHeadersHelper": "x"}), json!("not an object")];
        assert_eq!(managed_conflicts(&docs), ["OTEL_LOGS_EXPORTER", "otelHeadersHelper"]);
    }

    #[test]
    fn managed_files_are_read_with_their_drop_ins() {
        let dir = tempfile::tempdir().unwrap();
        let cc = dir.path().join("ClaudeCode");
        fs::create_dir_all(cc.join("managed-settings.d")).unwrap();
        fs::write(cc.join("managed-settings.json"), r#"{"a": 1}"#).unwrap();
        fs::write(cc.join("managed-settings.d").join("10-x.json"), "\u{feff}{\"b\": 2}").unwrap();
        fs::write(cc.join("managed-settings.d").join("notes.txt"), "{}").unwrap();
        assert_eq!(managed_files(dir.path()), vec![json!({"a": 1}), json!({"b": 2})]);
        assert!(managed_files(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn an_old_install_gets_a_token_only_while_its_entries_are_untouched() {
        let (_d, file, data) = setup(Some(r#"{"env": {"MY_VAR": "1"}}"#));
        let mut st = CaptureState::default();
        install_otel(&mut st, &data, &file, 43180, "old", 1).unwrap();
        // as an install from before tokens existed
        let legacy = {
            let mut doc = read(&file);
            doc["env"].as_object_mut().unwrap().remove(HEADERS_KEY);
            fs::write(&file, doc.to_string()).unwrap();
            st.otel.as_mut().unwrap().added.remove(HEADERS_KEY);
            st.clone()
        };
        assert!(add_otel_token(&mut st, &data, "new").unwrap());
        assert_eq!(st.otel.as_ref().unwrap().token(), Some("new"));
        assert_eq!(read(&file)["env"][HEADERS_KEY], "x-aiut-token=new");
        assert_eq!(read_state(&data).unwrap(), st);
        assert!(!add_otel_token(&mut st, &data, "newer").unwrap(), "already has one");

        let mut st = legacy;
        let mut doc = read(&file);
        doc["env"].as_object_mut().unwrap().remove(HEADERS_KEY);
        doc["env"]["OTEL_LOGS_EXPORTER"] = json!("console");
        fs::write(&file, doc.to_string()).unwrap();
        assert!(!add_otel_token(&mut st, &data, "new").unwrap(), "the user changed our entries");
        assert!(st.otel.as_ref().unwrap().token().is_none());
    }

    #[test]
    fn a_failed_revert_keeps_the_record() {
        let (_d, file, data) = setup(None);
        let mut st = CaptureState::default();
        install_statusline(&mut st, &data, &file, "aiut --statusline", 1).unwrap();
        fs::write(&file, "{ not json").unwrap();
        assert!(uninstall_statusline(&mut st, &data).is_err());
        assert!(st.statusline.is_some());
        assert!(read_state(&data).unwrap().statusline.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_write_leaves_no_record_and_no_backup() {
        let (_d, file, data) = setup(Some("{}"));
        let mut st = CaptureState::default();
        // Windows refuses to replace a read-only file
        let mut perm = fs::metadata(&file).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&file, perm.clone()).unwrap();
        let r = install_statusline(&mut st, &data, &file, "x", 1);
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&file, perm).unwrap();
        assert!(r.is_err());
        assert_eq!(st, CaptureState::default());
        assert_eq!(read_state(&data).unwrap(), CaptureState::default());
        assert_eq!(fs::read_dir(data.join("backups")).unwrap().count(), 0);
        assert_eq!(fs::read_to_string(&file).unwrap(), "{}");
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no temporary file is left");
    }

    #[test]
    fn unparseable_settings_are_never_touched() {
        let (_d, file, data) = setup(Some("{ // comments are not JSON\n }"));
        assert!(matches!(install_statusline(&mut CaptureState::default(), &data, &file, "x", 1), Err(SettingsError::Unparseable(_))));
        assert_eq!(fs::read_to_string(&file).unwrap(), "{ // comments are not JSON\n }");
    }

    #[test]
    fn a_file_changed_after_it_was_read_is_not_overwritten() {
        let (_d, file, _data) = setup(Some(r#"{"a": 1}"#));
        let (mut doc, original) = read_doc(&file).unwrap();
        fs::write(&file, r#"{"a": 2}"#).unwrap();
        doc.insert("b".into(), json!(1));
        assert!(matches!(write_doc(&file, &doc, &original), Err(SettingsError::Changed)));
        assert_eq!(fs::read_to_string(&file).unwrap(), r#"{"a": 2}"#);
    }

    #[test]
    fn state_round_trips_and_an_unreadable_record_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let st = CaptureState { statusline: None, otel: None };
        save_state(dir.path(), &st).unwrap();
        assert_eq!(load_state(dir.path()), st);
        assert_eq!(load_state(&dir.path().join("missing")), CaptureState::default());
        assert_eq!(read_state(&dir.path().join("missing")).unwrap(), CaptureState::default());
        fs::write(state_file(dir.path()), "{ broken").unwrap();
        assert!(matches!(read_state(dir.path()), Err(SettingsError::StateUnreadable(_))));
        assert_eq!(load_state(dir.path()), CaptureState::default());
    }

    #[test]
    fn tokens_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
