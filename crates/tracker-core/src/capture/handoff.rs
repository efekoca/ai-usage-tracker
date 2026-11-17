//! The handoff note. When a conversation's usage limit is nearly full, the hook asks Claude to write
//! what it did, what is left and the next steps to a note in the app's folder, one per project, so
//! the work can go on after the reset or in Codex. The app never reads a note; it only passes its
//! path on. The hook and the app share small files: the latest limit reading of each conversation,
//! the app's own reading while one Claude account is known, and a record of each conversation told.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// A limit at or above this share asks for the note.
pub const THRESHOLD_PCT: f64 = 95.0;
/// A reading older than this says nothing about now.
const READING_MS: i64 = 15 * 60_000;
/// Notes and the records around them are kept this long.
pub const KEEP_MS: i64 = 14 * 86_400_000;
const WINDOWS: [&str; 2] = ["five_hour", "seven_day"];

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("handoff")
}

fn notes_dir(dir: &Path) -> PathBuf {
    dir.join("notes")
}

fn readings_dir(dir: &Path) -> PathBuf {
    dir.join("readings")
}

fn told_dir(dir: &Path) -> PathBuf {
    dir.join("told")
}

fn shared_file(dir: &Path) -> PathBuf {
    dir.join("limits.json")
}

/// Write and edit rights for the notes folder only. Claude Code reads `//` as the root of the file
/// system and matches Windows paths in POSIX form, `C:\x` as `/c/x`.
pub fn permission_rules(dir: &Path) -> Vec<String> {
    let path = rule_path(&notes_dir(dir));
    vec![format!("Write({path}/**)"), format!("Edit({path}/**)")]
}

fn rule_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = match s.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => format!("/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
        _ => s,
    };
    format!("/{}", s.trim_end_matches('/'))
}

/// One note per project folder: its name, then a hash of the whole path, so two folders of the
/// same name keep their own.
pub fn note_key(cwd: &Path) -> String {
    let name: String = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(40)
        .collect();
    let name = if name.trim_matches('-').is_empty() { "project".to_owned() } else { name };
    format!("{name}-{:08x}", fnv1a(cwd.to_string_lossy().as_bytes()) as u32)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn note_path(dir: &Path, key: &str) -> PathBuf {
    notes_dir(dir).join(format!("{key}.md"))
}

fn meta_path(dir: &Path, key: &str) -> PathBuf {
    notes_dir(dir).join(format!("{key}.json"))
}

/// Limit windows as Claude Code's status line gives them: `used_percentage` 0 to 100 and
/// `resets_at` in epoch seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub ts: i64,
    pub windows: Map<String, Value>,
}

/// The window that asks for a note.
#[derive(Debug, Clone, PartialEq)]
pub struct Full {
    pub window: String,
    pub pct: f64,
    pub resets_ms: i64,
}

/// The fullest window at or above the threshold, from a recent reading whose window has not reset
/// since.
pub fn nearly_full(r: &Reading, now_ms: i64) -> Option<Full> {
    if !fresh(r, now_ms) {
        return None;
    }
    WINDOWS
        .iter()
        .filter_map(|w| {
            let v = r.windows.get(*w)?;
            let pct = v.get("used_percentage")?.as_f64()?;
            let resets_ms = v.get("resets_at")?.as_i64()?.checked_mul(1000)?;
            (pct >= THRESHOLD_PCT && resets_ms > now_ms).then(|| Full { window: (*w).to_owned(), pct, resets_ms })
        })
        .max_by(|a, b| a.pct.total_cmp(&b.pct))
}

pub fn fresh(r: &Reading, now_ms: i64) -> bool {
    now_ms - r.ts <= READING_MS && r.ts <= now_ms + 60_000
}

fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{}.{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    // written whole first, so the hook never reads half of it
    fs::write(&tmp, serde_json::to_vec(value)?)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The latest windows of one conversation, which tell its own account's limits.
pub fn record_reading(dir: &Path, session: &str, windows: &Map<String, Value>, now_ms: i64) -> std::io::Result<()> {
    let path = readings_dir(dir).join(format!("{session}.json"));
    let r = Reading { ts: now_ms, windows: windows.clone() };
    if read_json::<Reading>(&path).is_some_and(|old| old.windows == r.windows && now_ms - old.ts < 60_000) {
        return Ok(());
    }
    write_json(&path, &r)
}

pub fn reading(dir: &Path, session: &str) -> Option<Reading> {
    read_json(&readings_dir(dir).join(format!("{session}.json")))
}

/// The app's own reading, for conversations whose status line the app does not see; only while
/// one Claude account is known, so it is never another account's.
pub fn write_shared(dir: &Path, reading: Option<&Reading>) -> std::io::Result<()> {
    match reading {
        Some(r) if read_json::<Reading>(&shared_file(dir)).as_ref() != Some(r) => write_json(&shared_file(dir), r),
        Some(_) => Ok(()),
        None => match fs::remove_file(shared_file(dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

pub fn shared(dir: &Path) -> Option<Reading> {
    read_json(&shared_file(dir))
}

/// What the conversation was told and where it was to write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Told {
    pub session: String,
    pub window: String,
    pub resets_ms: i64,
    pub told_ms: i64,
    pub cwd: PathBuf,
    pub key: String,
    /// The app sent its notification about this note.
    #[serde(default)]
    pub notified: bool,
    /// A scripted run: not asked, only noted, so the next tool call need not check again.
    #[serde(default)]
    pub scripted: bool,
    /// The user went on in Codex, so the conversation is not continued at the reset too.
    #[serde(default)]
    pub handed: bool,
}

fn told_file(dir: &Path, session: &str) -> PathBuf {
    told_dir(dir).join(format!("{session}.json"))
}

pub fn told_record(dir: &Path, session: &str) -> Option<Told> {
    read_json(&told_file(dir, session))
}

/// Asked once until the window it was asked in resets, whichever window comes near the limit
/// next and whichever reading tells it.
pub fn told(dir: &Path, session: &str, now_ms: i64) -> bool {
    told_record(dir, session).is_some_and(|t| t.resets_ms > now_ms)
}

/// Two tool calls at once both pass [`told`]; only the one that claims the window goes on.
pub fn claim(dir: &Path, session: &str, resets_ms: i64) -> bool {
    let d = told_dir(dir);
    let _ = fs::create_dir_all(&d);
    fs::OpenOptions::new().write(true).create_new(true).open(d.join(format!("{session}.{resets_ms}.claim"))).is_ok()
}

pub fn mark_told(dir: &Path, t: &Told) -> std::io::Result<()> {
    write_json(&told_file(dir, &t.session), t)?;
    if t.scripted {
        return Ok(());
    }
    write_json(&meta_path(dir, &t.key), &json!({ "cwd": t.cwd, "session": t.session, "told_ms": t.told_ms }))
}

/// Every conversation told, the latest first.
pub fn all_told(dir: &Path) -> Vec<Told> {
    let mut out: Vec<Told> = fs::read_dir(told_dir(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| read_json::<Told>(&e.path()))
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.told_ms));
    out
}

pub fn set_notified(dir: &Path, t: &Told) -> std::io::Result<()> {
    write_json(&told_file(dir, &t.session), &Told { notified: true, ..t.clone() })
}

/// Marks the conversations asked for this project's note as gone on in Codex.
pub fn set_handed(dir: &Path, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in all_told(dir).into_iter().filter(|t| t.key == key && !t.scripted) {
        if write_json(&told_file(dir, &t.session), &Told { handed: true, ..t.clone() }).is_ok() {
            out.push(t.session);
        }
    }
    out
}

/// A note Claude wrote, without its contents.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    pub key: String,
    pub cwd: PathBuf,
    pub session: String,
    pub written_ms: i64,
}

pub fn note(dir: &Path, key: &str) -> Option<Note> {
    let meta: Value = read_json(&meta_path(dir, key))?;
    let written_ms = fs::metadata(note_path(dir, key)).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some(Note {
        key: key.to_owned(),
        cwd: PathBuf::from(meta.get("cwd")?.as_str()?),
        session: meta.get("session").and_then(Value::as_str).unwrap_or_default().to_owned(),
        written_ms,
    })
}

/// The notes Claude wrote, the latest first.
pub fn notes(dir: &Path) -> Vec<Note> {
    let mut out: Vec<Note> = fs::read_dir(notes_dir(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().is_some_and(|x| x == "md")).then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned())).flatten()
        })
        .filter(|k| valid_key(k))
        .filter_map(|k| note(dir, &k))
        .collect();
    out.sort_by_key(|n| std::cmp::Reverse(n.written_ms));
    out
}

/// Drops what is older than [`KEEP_MS`], notes too.
pub fn expire(dir: &Path, now: std::time::SystemTime) {
    let keep = std::time::Duration::from_millis(KEEP_MS as u64);
    for d in [notes_dir(dir), readings_dir(dir), told_dir(dir), dir.to_owned()] {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let old = e.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > keep);
            if old {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Everything the feature left, when it is turned off or all data is deleted.
pub fn forget(dir: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Told when the conversation starts, so a request in the middle of a turn is expected and trusted.
pub fn intro(note: &Path) -> String {
    format!(
        "AI Usage Tracker, an app the user installed, watches the user's Claude usage limits. If it reports during this conversation \
         that a usage limit is nearly full, write a short handoff note to {} (what you did, what is unfinished, the next steps, and the \
         files involved) right away, then go on with the task. The note lets the user carry on after the limit resets or in another tool.",
        note.display()
    )
}

/// The request itself, in the middle of a turn or with a prompt.
pub fn trigger(note: &Path, full: &Full) -> String {
    let window = if full.window == "five_hour" { "5-hour" } else { "weekly" };
    format!(
        "AI Usage Tracker: the {window} usage limit is at {}%. Write the handoff note to {} now, replacing any older one, then go on.",
        full.pct.floor() as u32,
        note.display()
    )
}

/// What Claude Code adds to the conversation from a hook.
pub fn hook_output(event: &str, text: &str) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": event, "additionalContext": text } }).to_string()
}

/// What Codex is started with.
pub fn codex_prompt(note: &Path) -> String {
    format!("Read the handoff note at {} and continue the work it describes in this folder.", note.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(ts: i64, five: f64, week: f64, resets_s: i64) -> Reading {
        let mut w = Map::new();
        w.insert("five_hour".into(), json!({"used_percentage": five, "resets_at": resets_s}));
        w.insert("seven_day".into(), json!({"used_percentage": week, "resets_at": resets_s + 86_400}));
        Reading { ts, windows: w }
    }

    #[test]
    fn a_window_asks_for_the_note_at_95_percent_from_a_recent_reading_before_it_resets() {
        let now = 1_800_000_000_000;
        let resets = now / 1000 + 3600;
        assert_eq!(nearly_full(&reading(now, 94.9, 50.0, resets), now), None);
        assert_eq!(nearly_full(&reading(now, 95.0, 50.0, resets), now).map(|f| f.window), Some("five_hour".into()));
        assert_eq!(nearly_full(&reading(now, 96.0, 99.0, resets), now).map(|f| f.window), Some("seven_day".into()), "the fuller one");
        assert_eq!(nearly_full(&reading(now - READING_MS - 1, 99.0, 50.0, resets), now), None, "an old reading");
        assert_eq!(nearly_full(&reading(now, 99.0, 50.0, now / 1000 - 1), now), None, "a window that reset since");
    }

    #[test]
    fn the_rule_names_the_notes_folder_from_the_root() {
        assert_eq!(permission_rules(Path::new("/Users/a b/Library/Application Support/x/handoff")), vec![
            "Write(//Users/a b/Library/Application Support/x/handoff/notes/**)".to_owned(),
            "Edit(//Users/a b/Library/Application Support/x/handoff/notes/**)".to_owned(),
        ]);
        assert_eq!(rule_path(Path::new(r"C:\Users\A\AppData\Local\x\handoff")), "//c/Users/A/AppData/Local/x/handoff");
    }

    #[test]
    fn each_project_folder_has_its_own_stable_note() {
        let a = note_key(Path::new("/work/site"));
        assert_eq!(a, note_key(Path::new("/work/site")));
        assert_ne!(a, note_key(Path::new("/other/site")));
        assert!(a.starts_with("site-") && valid_key(&a));
        assert!(valid_key(&note_key(Path::new("/work/Çok güzel proje!"))));
        assert!(note_key(Path::new("/")).starts_with("project-"));
    }

    #[test]
    fn a_conversation_is_told_once_a_window_and_its_note_is_listed_without_reading_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        assert!(!told(&dir, "s1", 1_000));
        let key = note_key(Path::new("/work/site"));
        let t = Told { session: "s1".into(), window: "five_hour".into(), resets_ms: 2_000, told_ms: 1_000, cwd: "/work/site".into(), key: key.clone(), notified: false, scripted: false, handed: false };
        mark_told(&dir, &t).unwrap();
        assert!(told(&dir, "s1", 1_500), "the other window or another reading does not ask again");
        assert!(!told(&dir, "s1", 2_000), "once the window resets it may ask again");
        assert!(notes(&dir).is_empty(), "nothing written yet");
        fs::write(note_path(&dir, &key), "x").unwrap();
        let listed = notes(&dir);
        assert_eq!((listed[0].key.as_str(), listed[0].session.as_str(), listed[0].cwd.as_path()), (key.as_str(), "s1", Path::new("/work/site")));
        set_notified(&dir, &t).unwrap();
        assert!(all_told(&dir)[0].notified);
        assert_eq!(set_handed(&dir, &key), ["s1"]);
        assert!(all_told(&dir)[0].handed && all_told(&dir)[0].notified);
    }

    #[test]
    fn only_one_of_two_tool_calls_at_once_claims_the_window() {
        let d = tempfile::tempdir().unwrap();
        assert!(claim(d.path(), "s1", 2_000));
        assert!(!claim(d.path(), "s1", 2_000));
        assert!(claim(d.path(), "s1", 9_000), "the next window");
        assert!(claim(d.path(), "s2", 2_000), "another conversation");
    }

    #[test]
    fn a_scripted_run_is_noted_without_a_note_to_list() {
        let d = tempfile::tempdir().unwrap();
        let t = Told { session: "s1".into(), window: "five_hour".into(), resets_ms: 2_000, told_ms: 1_000, cwd: "/w".into(), key: "w-1".into(), notified: false, scripted: true, handed: false };
        mark_told(d.path(), &t).unwrap();
        assert!(told(d.path(), "s1", 1_500));
        assert!(!meta_path(d.path(), "w-1").exists());
        assert!(set_handed(d.path(), "w-1").is_empty());
    }

    #[test]
    fn the_shared_reading_goes_away_when_more_than_one_account_is_known() {
        let d = tempfile::tempdir().unwrap();
        let r = reading(1, 50.0, 50.0, 10);
        write_shared(d.path(), Some(&r)).unwrap();
        assert_eq!(shared(d.path()), Some(r));
        write_shared(d.path(), None).unwrap();
        assert_eq!(shared(d.path()), None);
        write_shared(d.path(), None).unwrap();
    }

    #[test]
    fn the_hook_output_carries_the_text_for_its_event() {
        let v: Value = serde_json::from_str(&hook_output("PostToolUse", "hi")).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], "hi");
    }
}
