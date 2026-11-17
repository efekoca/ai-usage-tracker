//! Our hook in Codex's own `hooks.json`. Codex runs a hook only after the user trusted it, and it
//! keeps that trust by the hook's place (`event:group:index`), so ours always goes last and only
//! our own entries are ever taken out. Codex's `config.toml` is never touched.

use super::claude_settings::{write_atomic, SettingsError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "PostToolUse"];
pub const TIMEOUT_SECS: u64 = 10;

/// What we added, so it can be taken out again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexHookChange {
    pub file: PathBuf,
    pub command: String,
    /// The file was not there before, so it goes when nothing but our hook was in it.
    #[serde(default)]
    pub created: bool,
    pub installed_at_ms: i64,
}

pub fn hooks_file(codex_home: &Path) -> PathBuf {
    codex_home.join("hooks.json")
}

fn read(file: &Path) -> Result<Option<Map<String, Value>>, SettingsError> {
    let raw = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SettingsError::Io(e)),
    };
    if raw.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    match serde_json::from_str(&raw) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        Ok(_) => Err(SettingsError::Unparseable("not a JSON object".into())),
        Err(e) => Err(SettingsError::Unparseable(e.to_string())),
    }
}

fn write(file: &Path, doc: &Map<String, Value>) -> Result<(), SettingsError> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(SettingsError::Io)?;
    }
    let text = serde_json::to_string_pretty(doc).map_err(|e| SettingsError::Unparseable(e.to_string()))?;
    // a file kept elsewhere and linked here (a dotfiles folder) stays where it is
    let target = match fs::symlink_metadata(file) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(file).map_err(SettingsError::Io)?,
        _ => file.to_owned(),
    };
    write_atomic(&target, &format!("{text}\n")).map_err(SettingsError::Io)
}

fn ours(group: &Value, command: &str) -> bool {
    group.get("hooks").and_then(Value::as_array).is_some_and(|h| h.iter().any(|x| x.get("command").and_then(Value::as_str) == Some(command)))
}

/// Adds our group to the end of each event's list, where it is missing. A command of an earlier
/// copy of the app in the same file is changed where it is, so the hooks after it keep their place.
/// A file that is not valid JSON, or whose hooks are not where Codex reads them, is left alone.
pub fn install(file: &Path, command: &str, previous: Option<&CodexHookChange>, now_ms: i64) -> Result<CodexHookChange, SettingsError> {
    let previous = previous.filter(|p| p.file == file);
    let existing = read(file)?;
    let created = previous.map_or(existing.is_none(), |p| p.created);
    let mut doc = existing.unwrap_or_default();
    let hooks = doc.entry("hooks").or_insert_with(|| json!({})).as_object_mut().ok_or_else(|| SettingsError::Unparseable("`hooks` is not an object".into()))?;
    let mut changed = false;
    for event in EVENTS {
        let list = hooks.entry(event).or_insert_with(|| json!([])).as_array_mut().ok_or_else(|| SettingsError::Unparseable(format!("`{event}` is not a list")))?;
        if let Some(old) = previous.map(|p| p.command.as_str()).filter(|old| *old != command) {
            for h in list.iter_mut().filter_map(|g| g.get_mut("hooks")).filter_map(Value::as_array_mut).flatten() {
                if h.get("command").and_then(Value::as_str) == Some(old) {
                    h["command"] = json!(command);
                    changed = true;
                }
            }
        }
        if !list.iter().any(|g| ours(g, command)) {
            list.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": TIMEOUT_SECS }] }));
            changed = true;
        }
    }
    if changed {
        write(file, &doc)?;
    }
    // Codex asks to trust only a hook that is new or changed
    let installed_at_ms = match previous {
        Some(p) if !changed && p.command == command => p.installed_at_ms,
        _ => now_ms,
    };
    Ok(CodexHookChange { file: file.to_owned(), command: command.to_owned(), created, installed_at_ms })
}

/// Our hook is in every event it should be.
pub fn in_place(c: &CodexHookChange) -> bool {
    let Ok(Some(doc)) = read(&c.file) else { return false };
    let hooks = doc.get("hooks").and_then(Value::as_object);
    EVENTS.iter().all(|e| hooks.and_then(|h| h.get(*e)).and_then(Value::as_array).is_some_and(|l| l.iter().any(|g| ours(g, &c.command))))
}

/// Takes out our entries only: a group or a list left empty by that goes too, and the file when
/// we made it and nothing else is in it.
pub fn uninstall(c: &CodexHookChange) -> Result<(), SettingsError> {
    let Some(mut doc) = read(&c.file)? else { return Ok(()) };
    let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) else { return Ok(()) };
    let mut changed = false;
    let mut emptied = Vec::new();
    for (event, list) in hooks.iter_mut().filter_map(|(k, v)| Some((k.clone(), v.as_array_mut()?))) {
        let mut took = false;
        list.retain_mut(|group| {
            let Some(h) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let n = h.len();
            h.retain(|x| x.get("command").and_then(Value::as_str) != Some(c.command.as_str()));
            took |= h.len() != n;
            !(h.is_empty() && n > 0)
        });
        if took && list.is_empty() {
            emptied.push(event);
        }
        changed |= took;
    }
    if !changed {
        return Ok(());
    }
    for event in emptied {
        hooks.shift_remove(&event);
    }
    if hooks.is_empty() {
        doc.shift_remove("hooks");
    }
    if c.created && doc.is_empty() {
        return match fs::remove_file(&c.file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(SettingsError::Io(e)),
            _ => Ok(()),
        };
    }
    write(&c.file, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "'/Applications/AI Usage Tracker.app/Contents/MacOS/aiut' --codex-handoff-hook";

    fn other() -> Value {
        json!({"hooks": {"SessionStart": [{"hooks": [{"command": "'/Users/u/.local/bin/jcode' setup-hotkey", "timeout": 5, "type": "command"}], "matcher": "startup|resume"}]}})
    }

    #[test]
    fn our_group_goes_after_the_users_own_and_only_ours_comes_out() {
        let d = tempfile::tempdir().unwrap();
        let file = hooks_file(d.path());
        fs::write(&file, serde_json::to_string_pretty(&other()).unwrap()).unwrap();
        let c = install(&file, CMD, None, 7).unwrap();
        assert!(!c.created && in_place(&c));
        let doc: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc["hooks"]["SessionStart"][0], other()["hooks"]["SessionStart"][0], "the user's group keeps its place");
        for e in EVENTS {
            let list = doc["hooks"][e].as_array().unwrap();
            assert_eq!(list.last().unwrap(), &json!({"hooks": [{"type": "command", "command": CMD, "timeout": 10}]}), "{e}");
        }
        let again = install(&file, CMD, Some(&c), 9).unwrap();
        let doc2: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc2, doc, "a second install adds nothing");
        assert_eq!(again.installed_at_ms, 7, "nothing for Codex to ask about again");
        uninstall(&again).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after, other());
        assert!(!in_place(&c));
    }

    #[test]
    fn a_file_we_made_goes_with_our_hook_and_one_with_more_in_it_stays() {
        let d = tempfile::tempdir().unwrap();
        let file = hooks_file(&d.path().join("codex"));
        let c = install(&file, CMD, None, 1).unwrap();
        assert!(c.created);
        uninstall(&c).unwrap();
        assert!(!file.exists());
        let c = install(&file, CMD, None, 1).unwrap();
        let mut doc: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        doc["hooks"]["Stop"] = json!([{"hooks": [{"type": "command", "command": "notify-me"}]}]);
        fs::write(&file, doc.to_string()).unwrap();
        uninstall(&c).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after, json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "notify-me"}]}]}}));
    }

    #[test]
    fn a_moved_app_changes_its_command_where_it_is_and_a_linked_file_stays_a_link() {
        let d = tempfile::tempdir().unwrap();
        let kept = d.path().join("dotfiles-hooks.json");
        let mut doc = other();
        doc["hooks"]["PostToolUse"] = json!([]);
        fs::write(&kept, doc.to_string()).unwrap();
        let file = hooks_file(&d.path().join("codex"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&kept, &file).unwrap();
        #[cfg(not(unix))]
        fs::copy(&kept, &file).unwrap();
        let c = install(&file, CMD, None, 1).unwrap();
        let mut doc: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        doc["hooks"]["PostToolUse"].as_array_mut().unwrap().push(json!({"hooks": [{"type": "command", "command": "later"}]}));
        fs::write(&file, doc.to_string()).unwrap();
        let moved = install(&file, "'/new/aiut' --codex-handoff-hook", Some(&c), 5).unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after["hooks"]["PostToolUse"][0]["hooks"][0]["command"], "'/new/aiut' --codex-handoff-hook");
        assert_eq!(after["hooks"]["PostToolUse"][1]["hooks"][0]["command"], "later", "keeps its place");
        assert_eq!(moved.installed_at_ms, 5);
        #[cfg(unix)]
        assert!(fs::symlink_metadata(&file).unwrap().file_type().is_symlink());
    }

    #[test]
    fn a_file_codex_cannot_read_is_never_changed() {
        let d = tempfile::tempdir().unwrap();
        let file = hooks_file(d.path());
        fs::write(&file, "{ not json").unwrap();
        assert!(matches!(install(&file, CMD, None, 1), Err(SettingsError::Unparseable(_))));
        fs::write(&file, r#"{"hooks": []}"#).unwrap();
        assert!(install(&file, CMD, None, 1).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), r#"{"hooks": []}"#);
    }
}
