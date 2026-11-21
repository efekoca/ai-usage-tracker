//! The handoff note. When a conversation's usage limit is nearly full, the hook asks Claude or Codex
//! to write what it did, what is left and the next steps to a note in the app's folder, one per
//! project, so the work can go on after the reset or in the other tool. The app never reads a note;
//! it only passes its path on. The hook and the app share small files: the latest limit reading of
//! each conversation, the app's own reading while one account of the tool is known, and a record
//! of each conversation told. Codex's sandbox keeps it out of the app's folder, so Codex writes to
//! a folder of the user's own in the temp folder and the note is moved over from there.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A limit at or above this share asks for the note.
pub const THRESHOLD_PCT: f64 = 95.0;
/// A reading older than this says nothing about now.
const READING_MS: i64 = 15 * 60_000;
/// Notes and the records around them are kept this long.
pub const KEEP_MS: i64 = 14 * 86_400_000;
const WINDOWS: [&str; 2] = ["five_hour", "seven_day"];
/// How much of the end of a Codex log is read for its latest limit reading.
const TAIL: u64 = 512 * 1024;

/// The tool a conversation runs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[default]
    Claude,
    Codex,
}

impl Agent {
    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
        }
    }
}

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

fn shared_file(dir: &Path, agent: Agent) -> PathBuf {
    match agent {
        Agent::Claude => dir.join("limits.json"),
        Agent::Codex => dir.join("codex-limits.json"),
    }
}

fn seen_file(dir: &Path) -> PathBuf {
    dir.join("codex-seen")
}

/// The right to write the notes folder only. An Edit rule covers every tool that writes a file;
/// Claude Code matches no Write rule and warns about one at every start. It reads `//` as the root
/// of the file system and matches Windows paths in POSIX form, `C:\x` as `/c/x`.
pub fn permission_rules(dir: &Path) -> Vec<String> {
    vec![format!("Edit({}/**)", rule_path(&notes_dir(dir)))]
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

/// Who wrote a note the app moved over from Codex: a file of its own, since a hook may rewrite the
/// project's record at the same moment.
fn writer_path(dir: &Path, key: &str) -> PathBuf {
    notes_dir(dir).join(format!("{key}.writer.json"))
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
    // two threads of the app may write the same record at once
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = parent.join(format!(".{}.{}.{n}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
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

/// The app's own reading, for conversations whose own the hook cannot read; only while one account
/// of the tool is known, so it is never another account's.
pub fn write_shared(dir: &Path, agent: Agent, reading: Option<&Reading>) -> std::io::Result<()> {
    let file = shared_file(dir, agent);
    match reading {
        Some(r) if read_json::<Reading>(&file).as_ref() != Some(r) => write_json(&file, r),
        Some(_) => Ok(()),
        None => match fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

pub fn shared(dir: &Path, agent: Agent) -> Option<Reading> {
    read_json(&shared_file(dir, agent))
}

/// The latest reading of Codex's main limit in a conversation's log, read from the end only. Another
/// limit's reading (`premium`) carries no windows and is passed over.
pub fn codex_reading(log: &Path) -> Option<Reading> {
    let mut f = fs::File::open(log).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL).read_to_end(&mut buf).ok()?;
    String::from_utf8_lossy(&buf).lines().rev().filter(|l| l.contains("\"rate_limits\"")).find_map(|line| {
        let v: Value = serde_json::from_str(line).ok()?;
        let p = v.get("payload")?;
        let rl = p.get("rate_limits").filter(|_| p.get("type").and_then(Value::as_str) == Some("token_count"))?;
        let ts = crate::model::parse_ts_ms(v.get("timestamp")?.as_str()?)?;
        let windows: Map<String, Value> = crate::sources::codex::main_windows(rl, ts)
            .into_iter()
            .map(|(w, pct, resets)| (w, json!({ "used_percentage": pct, "resets_at": resets })))
            .collect();
        (!windows.is_empty()).then_some(Reading { ts, windows })
    })
}

/// Left by the Codex hook when a conversation starts or a prompt is sent: Codex runs a hook only
/// once the user trusted it, so this tells the hook really runs.
pub fn mark_codex_seen(dir: &Path, now_ms: i64) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(seen_file(dir), now_ms.to_string())
}

pub fn codex_seen(dir: &Path) -> Option<i64> {
    fs::read_to_string(seen_file(dir)).ok()?.trim().parse().ok()
}

/// What the conversation was told and where it was to write.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// The user went on in the other tool, so the conversation is not continued at the reset too.
    #[serde(default)]
    pub handed: bool,
    #[serde(default)]
    pub agent: Agent,
    /// Where Codex was told to write; the note is moved over from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop: Option<PathBuf>,
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
    let path = d.join(format!("{session}.{resets_ms}.claim"));
    let take = || fs::OpenOptions::new().write(true).create_new(true).open(&path).is_ok();
    if take() {
        return true;
    }
    // a hook stopped between its claim and its record (Claude Code ends one at its timeout) would
    // hold the window for good: the record, not the claim, says the conversation was asked
    let Some(written) = fs::metadata(&path).and_then(|m| m.modified()).ok() else { return false };
    if written.elapsed().is_ok_and(|age| age <= std::time::Duration::from_secs(60)) || told_record(dir, session).is_some_and(|t| t.resets_ms == resets_ms) {
        return false;
    }
    // of two hooks that both find it stale only the one that marks this very claim takes it over
    let stamp = written.duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    let mark = d.join(format!("{session}.{resets_ms}.claim.{stamp}.take"));
    fs::OpenOptions::new().write(true).create_new(true).open(mark).is_ok() && fs::remove_file(&path).is_ok() && take()
}

/// The project's record names the conversation told last and, while its note is not written yet,
/// the one before it, whose note the project still has.
pub fn mark_told(dir: &Path, t: &Told) -> std::io::Result<()> {
    // whoever wrote the note there now, not whoever was asked last: that one may never have written.
    // Read before this request's own record replaces the conversation's earlier one.
    let before = (!t.scripted).then(|| note(dir, &t.key).map(|n| json!({ "session": n.session, "agent": n.agent }))).flatten();
    write_json(&told_file(dir, &t.session), t)?;
    if t.scripted {
        return Ok(());
    }
    write_json(&meta_path(dir, &t.key), &json!({ "cwd": t.cwd, "session": t.session, "told_ms": t.told_ms, "agent": t.agent, "before": before }))
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

/// Read again first: the user may have gone on in the other tool since `t` was read, and a record
/// of a later request is not this one's to mark.
pub fn set_notified(dir: &Path, t: &Told) -> std::io::Result<()> {
    match told_record(dir, &t.session) {
        Some(now) if now.told_ms == t.told_ms => write_json(&told_file(dir, &t.session), &Told { notified: true, ..now }),
        _ => Ok(()),
    }
}

/// Marks the conversations asked for this project's note as gone on in the other tool.
pub fn set_handed(dir: &Path, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in all_told(dir).into_iter().filter(|t| t.key == key && !t.scripted) {
        // read again right before writing, as in `set_notified`: a newer request's record stays
        let Some(now) = told_record(dir, &t.session).filter(|n| n.told_ms == t.told_ms) else { continue };
        if write_json(&told_file(dir, &t.session), &Told { handed: true, ..now }).is_ok() {
            out.push(t.session);
        }
    }
    out
}

/// A note Claude or Codex wrote, without its contents.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    pub key: String,
    pub cwd: PathBuf,
    pub session: String,
    pub written_ms: i64,
    pub agent: Agent,
}

pub fn note(dir: &Path, key: &str) -> Option<Note> {
    let meta: Value = read_json(&meta_path(dir, key))?;
    let written_ms = modified_ms(&note_path(dir, key))?;
    // a note moved over from Codex names its writer while it is still that note
    let named = read_json::<Value>(&writer_path(dir, key));
    let writer = named.clone().filter(|w| w.get("written_ms").and_then(Value::as_i64) == Some(written_ms));
    // a note older than the last request is still the one the conversation before wrote
    let told = meta.get("told_ms").and_then(Value::as_i64).unwrap_or(0);
    let codex_asked_last = meta.get("agent").and_then(|a| serde_json::from_value::<Agent>(a.clone()).ok()) == Some(Agent::Codex);
    let who = match (writer, meta.get("before").filter(|b| b.is_object())) {
        (Some(w), _) => w,
        (None, Some(before)) if written_ms + 2_000 < told => before.clone(),
        // Codex writes only through its drop and a moved note names its writer: one changed in
        // place since is a Claude conversation's, also when Codex was asked last
        _ if named.is_some() && codex_asked_last => claude_asked_before(dir, key, written_ms).map_or_else(|| meta.clone(), |c| json!({ "session": c.session, "agent": c.agent })),
        _ => meta.clone(),
    };
    Some(Note {
        key: key.to_owned(),
        cwd: PathBuf::from(meta.get("cwd")?.as_str()?),
        session: who.get("session").and_then(Value::as_str).unwrap_or_default().to_owned(),
        written_ms,
        agent: who.get("agent").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default(),
    })
}

/// The notes written, the latest first.
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

/// Drops what is older than [`KEEP_MS`], notes too; the sign that Codex runs the hook stays.
pub fn expire(dir: &Path, now: std::time::SystemTime) {
    let keep = std::time::Duration::from_millis(KEEP_MS as u64);
    for d in [notes_dir(dir), readings_dir(dir), told_dir(dir), dir.to_owned()] {
        for e in fs::read_dir(&d).into_iter().flatten().flatten().filter(|e| e.path() != seen_file(dir)) {
            let old = e.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > keep);
            if old {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Everything the feature left, when it is turned off or all data is deleted: also the notes Codex
/// left in a temp folder, wherever its own temp folder was.
pub fn forget(dir: &Path) -> std::io::Result<()> {
    for t in all_told(dir) {
        if let Some(drop) = t.drop.filter(|d| d.parent().is_some_and(is_private)) {
            let _ = fs::remove_file(drop);
        }
    }
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The user's own folder in the temp folder, where Codex writes its note.
pub fn drop_dir() -> PathBuf {
    #[cfg(unix)]
    // SAFETY: getuid has no preconditions and cannot fail
    let name = format!("ai-usage-tracker-handoff-{}", unsafe { libc::getuid() });
    #[cfg(not(unix))]
    let name = "ai-usage-tracker-handoff".to_owned();
    std::env::temp_dir().join(name)
}

/// Where Codex writes this project's note. None when the folder is not the user's alone: the temp
/// folder is shared on Linux, and Codex's sandbox can write there.
pub fn codex_drop(key: &str) -> Option<PathBuf> {
    let d = drop_dir();
    private_dir(&d).ok()?;
    Some(d.join(format!("{key}.md")))
}

fn private_dir(d: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match fs::DirBuilder::new().mode(0o700).create(d) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(d)?;
    if is_private(d) { Ok(()) } else { Err(std::io::Error::other("the note folder is not private")) }
}

/// A folder, not a link, and on Unix the user's own that no one else may open.
fn is_private(d: &Path) -> bool {
    let Ok(m) = fs::symlink_metadata(d) else { return false };
    #[cfg(unix)]
    // SAFETY: getuid has no preconditions and cannot fail
    let mine = std::os::unix::fs::MetadataExt::uid(&m) == unsafe { libc::getuid() } && std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o077 == 0;
    #[cfg(not(unix))]
    let mine = true;
    m.is_dir() && mine
}

/// The note file itself, never what a link points to.
fn open_note(from: &Path) -> std::io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut o, libc::O_NOFOLLOW);
    #[cfg(windows)]
    {
        if fs::symlink_metadata(from)?.file_type().is_symlink() {
            return Err(std::io::Error::other("a link"));
        }
    }
    o.open(from)
}

/// Copies the note Codex wrote after it was asked over to the notes folder and takes it out of the
/// temp folder. Read through its own handle: Codex can change that folder between two steps.
pub fn collect(dir: &Path, t: &Told) -> bool {
    let Some(from) = &t.drop else { return false };
    let Ok(mut file) = open_note(from) else { return false };
    let Ok(m) = file.metadata() else { return false };
    #[cfg(unix)]
    let single = std::os::unix::fs::MetadataExt::nlink(&m) == 1;
    #[cfg(not(unix))]
    let single = true;
    let written = m.modified().ok().and_then(|w| w.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
    // an older note of the same project from an earlier request stays where it is
    if !m.is_file() || !single || written.is_none_or(|w| w + 2_000 < t.told_ms) || m.len() > 1024 * 1024 {
        return false;
    }
    // a note an asked Claude conversation wrote in place after Codex wrote this one is the newer:
    // the project keeps one note, and Codex's is left for the temp folder's own clearing. A time
    // ahead of the clock tells nothing.
    let now_ms = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    if let (Some(w), Some(here)) = (written, modified_ms(&note_path(dir, &t.key)))
        && here > w
        && here <= now_ms + 60_000
        && let Some(c) = claude_asked_before(dir, &t.key, here)
    {
        // named once: a later request of another conversation must not change who wrote it
        let named = read_json::<Value>(&writer_path(dir, &t.key)).is_some_and(|n| n.get("written_ms").and_then(Value::as_i64) == Some(here));
        if !named {
            let _ = write_json(&writer_path(dir, &t.key), &json!({ "session": c.session, "agent": c.agent, "written_ms": here }));
        }
        return false;
    }
    let mut text = Vec::new();
    if file.read_to_end(&mut text).is_err() || write_note(dir, &t.key, &text).is_err() {
        return false;
    }
    let _ = fs::remove_file(from);
    // moving it over is not writing it: the note keeps Codex's time and names its writer, since a
    // conversation asked meanwhile would otherwise look like the one that wrote it
    let to = note_path(dir, &t.key);
    if let Ok(w) = m.modified() {
        let _ = fs::File::options().write(true).open(&to).and_then(|f| f.set_modified(w));
    }
    if let Some(at) = modified_ms(&to) {
        let _ = write_json(&writer_path(dir, &t.key), &json!({ "session": t.session, "agent": t.agent, "written_ms": at }));
    }
    true
}

/// The Claude conversation asked for this project's note last before `written_ms`.
fn claude_asked_before(dir: &Path, key: &str, written_ms: i64) -> Option<Told> {
    all_told(dir).into_iter().find(|t| t.key == key && t.agent == Agent::Claude && !t.scripted && t.told_ms <= written_ms + 2_000)
}

fn modified_ms(path: &Path) -> Option<i64> {
    Some(fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

fn write_note(dir: &Path, key: &str, text: &[u8]) -> std::io::Result<()> {
    let to = note_path(dir, key);
    fs::create_dir_all(notes_dir(dir))?;
    let tmp = notes_dir(dir).join(format!(".{key}.{}", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &to).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Drops notes Codex left in the temp folder and nobody moved, and with `all` every one of them.
pub fn clear_drops(now: std::time::SystemTime, all: bool) {
    let d = drop_dir();
    if !is_private(&d) {
        return;
    }
    let keep = std::time::Duration::from_millis(KEEP_MS as u64);
    for e in fs::read_dir(&d).into_iter().flatten().flatten() {
        let old = all || e.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > keep);
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Told when the conversation starts, so a request in the middle of a turn is expected and trusted.
pub fn intro(note: &Path, agent: Agent) -> String {
    format!(
        "AI Usage Tracker, an app the user installed, watches the user's {} usage limits. If it reports during this conversation \
         that a usage limit is nearly full, write a short handoff note to {} (what you did, what is unfinished, the next steps, and the \
         files involved) right away, then go on with the task. The note lets the user carry on after the limit resets or in another tool.",
        agent.name(),
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

/// What the other tool is started with.
pub fn continue_prompt(note: &Path) -> String {
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
        let t = Told { session: "s1".into(), window: "five_hour".into(), resets_ms: 2_000, told_ms: 1_000, cwd: "/work/site".into(), key: key.clone(), ..Default::default() };
        mark_told(&dir, &t).unwrap();
        assert!(told(&dir, "s1", 1_500), "the other window or another reading does not ask again");
        assert!(!told(&dir, "s1", 2_000), "once the window resets it may ask again");
        assert!(notes(&dir).is_empty(), "nothing written yet");
        fs::write(note_path(&dir, &key), "x").unwrap();
        let listed = notes(&dir);
        assert_eq!((listed[0].key.as_str(), listed[0].session.as_str(), listed[0].cwd.as_path(), listed[0].agent), (key.as_str(), "s1", Path::new("/work/site"), Agent::Claude));
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
        let left = told_dir(d.path()).join("s1.2000.claim");
        fs::File::options().write(true).open(&left).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(120)).unwrap();
        assert!(claim(d.path(), "s1", 2_000), "a claim a stopped hook left without its record");
        assert!(!claim(d.path(), "s1", 2_000), "taken again just now");
        // another hook is taking over the same stale claim
        let left = told_dir(d.path()).join("s2.2000.claim");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        fs::File::options().write(true).open(&left).unwrap().set_modified(old).unwrap();
        let stamp = fs::metadata(&left).unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_millis();
        fs::write(told_dir(d.path()).join(format!("s2.2000.claim.{stamp}.take")), "").unwrap();
        assert!(!claim(d.path(), "s2", 2_000), "only one of two takes it over");
    }

    #[test]
    fn a_scripted_run_is_noted_without_a_note_to_list() {
        let d = tempfile::tempdir().unwrap();
        let t = Told { session: "s1".into(), window: "five_hour".into(), resets_ms: 2_000, told_ms: 1_000, cwd: "/w".into(), key: "w-1".into(), scripted: true, ..Default::default() };
        mark_told(d.path(), &t).unwrap();
        assert!(told(d.path(), "s1", 1_500));
        assert!(!meta_path(d.path(), "w-1").exists());
        assert!(set_handed(d.path(), "w-1").is_empty());
    }

    #[test]
    fn the_shared_reading_goes_away_when_more_than_one_account_is_known() {
        let d = tempfile::tempdir().unwrap();
        let r = reading(1, 50.0, 50.0, 10);
        write_shared(d.path(), Agent::Claude, Some(&r)).unwrap();
        assert_eq!(shared(d.path(), Agent::Claude), Some(r));
        assert_eq!(shared(d.path(), Agent::Codex), None, "each tool has its own");
        write_shared(d.path(), Agent::Claude, None).unwrap();
        assert_eq!(shared(d.path(), Agent::Claude), None);
        write_shared(d.path(), Agent::Claude, None).unwrap();
    }

    #[test]
    fn a_codex_reading_is_the_latest_main_limit_line_at_the_end_of_its_log() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("rollout.jsonl");
        let line = |ts: &str, rl: Value| json!({"timestamp": ts, "type": "event_msg", "payload": {"type": "token_count", "info": null, "rate_limits": rl}}).to_string();
        let main = |five: f64| json!({"limit_id": "codex", "primary": {"used_percent": five, "window_minutes": 300, "resets_at": 1_794_228_269}, "secondary": {"used_percent": 41.0, "window_minutes": 10080, "resets_at": 1_794_650_450}});
        let body = [
            "x".repeat(TAIL as usize),
            line("2026-11-09T09:40:00.000Z", main(80.0)),
            line("2026-11-09T09:53:16.000Z", main(96.0)),
            line("2026-11-09T09:53:16.100Z", json!({"limit_id": "premium", "primary": null, "secondary": null})),
            json!({"timestamp": "2026-11-09T09:53:17.000Z", "type": "response_item", "payload": {"type": "message"}}).to_string(),
        ]
        .join("\n");
        fs::write(&log, body).unwrap();
        let r = codex_reading(&log).unwrap();
        assert_eq!(r.ts, crate::model::parse_ts_ms("2026-11-09T09:53:16.000Z").unwrap());
        assert_eq!(r.windows["five_hour"], json!({"used_percentage": 96.0, "resets_at": 1_794_228_269}));
        assert_eq!(nearly_full(&r, r.ts).map(|f| (f.window, f.resets_ms)), Some(("five_hour".into(), 1_794_228_269_000)));
        assert_eq!(codex_reading(&d.path().join("none.jsonl")), None);
    }

    #[test]
    fn a_note_codex_wrote_after_it_was_asked_is_moved_over_and_an_older_one_or_a_link_is_not() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let drop = d.path().join("drop");
        fs::create_dir_all(&drop).unwrap();
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let from = drop.join("site-1.md");
        let t = Told { session: "s1".into(), told_ms: now - 1_000, key: "site-1".into(), agent: Agent::Codex, drop: Some(from.clone()), ..Default::default() };
        assert!(!collect(&dir, &t), "nothing written yet");
        fs::write(&from, "note").unwrap();
        assert!(!collect(&dir, &Told { told_ms: now + 60_000, ..t.clone() }), "written before the request");
        assert!(collect(&dir, &t));
        assert_eq!(fs::read_to_string(note_path(&dir, "site-1")).unwrap(), "note");
        assert!(!from.exists());
        #[cfg(unix)]
        {
            let secret = d.path().join("secret");
            fs::write(&secret, "x").unwrap();
            fs::remove_file(note_path(&dir, "site-1")).unwrap();
            std::os::unix::fs::symlink(&secret, &from).unwrap();
            assert!(!collect(&dir, &t), "a link");
            fs::remove_file(&from).unwrap();
            fs::hard_link(&secret, &from).unwrap();
            assert!(!collect(&dir, &t), "a second name of another file");
            assert!(!note_path(&dir, "site-1").exists());
        }
    }

    #[test]
    fn a_note_is_listed_under_the_tool_that_wrote_it_until_the_next_one_is_written() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let claude = Told { session: "c1".into(), told_ms: now - 120_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() };
        mark_told(&dir, &claude).unwrap();
        fs::write(note_path(&dir, "site-1"), "claude's").unwrap();
        let written = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        fs::File::options().write(true).open(note_path(&dir, "site-1")).unwrap().set_modified(written).unwrap();
        let drop = d.path().join("site-1.md");
        let codex = Told { session: "x1".into(), told_ms: now - 30_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c1"), "asked, not written yet");
        fs::write(&drop, "codex's").unwrap();
        assert!(collect(&dir, &codex));
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Codex, "x1"));
    }

    #[test]
    fn a_note_stays_with_its_writer_when_a_conversation_asked_after_it_never_wrote() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let told = |session: &str, ago: i64, agent: Agent| Told { session: session.into(), told_ms: now - ago, cwd: "/w/site".into(), key: "site-1".into(), agent, ..Default::default() };
        mark_told(&dir, &told("c1", 120_000, Agent::Claude)).unwrap();
        fs::write(note_path(&dir, "site-1"), "claude's").unwrap();
        fs::File::options().write(true).open(note_path(&dir, "site-1")).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(90)).unwrap();
        mark_told(&dir, &told("x1", 60_000, Agent::Codex)).unwrap();
        mark_told(&dir, &told("c2", 30_000, Agent::Claude)).unwrap();
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c1"), "Codex was asked but never wrote it");
    }

    #[test]
    fn a_codex_note_moved_over_after_a_later_request_stays_codexs() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let ago = |s: u64| now - std::time::Duration::from_secs(s);
        mark_told(&dir, &Told { session: "c1".into(), told_ms: ms - 300_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() }).unwrap();
        fs::write(note_path(&dir, "site-1"), "claude's").unwrap();
        fs::File::options().write(true).open(note_path(&dir, "site-1")).unwrap().set_modified(ago(250)).unwrap();
        let drop = d.path().join("site-1.md");
        let codex = Told { session: "x1".into(), told_ms: ms - 200_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        fs::write(&drop, "codex's").unwrap();
        fs::File::options().write(true).open(&drop).unwrap().set_modified(ago(150)).unwrap();
        // Claude is asked before the app gets to move Codex's note over
        mark_told(&dir, &Told { session: "c2".into(), told_ms: ms - 100_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() }).unwrap();
        assert!(collect(&dir, &codex));
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Codex, "x1"));
        assert!((n.written_ms - (ms - 150_000)).abs() < 1_000, "when Codex wrote it, not when it was moved");
        // a later note written in place is the asked conversation's again
        fs::write(note_path(&dir, "site-1"), "claude's again").unwrap();
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c2"));
    }

    #[test]
    fn a_note_written_after_codexs_is_not_replaced_by_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let ago = |s: u64| now - std::time::Duration::from_secs(s);
        let drop = d.path().join("site-1.md");
        let codex = Told { session: "x1".into(), told_ms: ms - 200_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        fs::write(&drop, "codex's").unwrap();
        fs::File::options().write(true).open(&drop).unwrap().set_modified(ago(60)).unwrap();
        // Claude is asked and writes its note in place before the app moves Codex's over
        mark_told(&dir, &Told { session: "c2".into(), told_ms: ms - 50_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() }).unwrap();
        fs::write(note_path(&dir, "site-1"), "claude's, newer").unwrap();
        assert!(!collect(&dir, &codex));
        assert_eq!(fs::read_to_string(note_path(&dir, "site-1")).unwrap(), "claude's, newer");
        assert!(drop.exists(), "left for the temp folder's own clearing, not deleted");
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c2"));
    }

    #[test]
    fn a_note_claude_writes_after_codex_was_asked_is_claudes() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let ago = |s: u64| now - std::time::Duration::from_secs(s);
        let claude = Told { session: "c1".into(), told_ms: ms - 300_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() };
        mark_told(&dir, &claude).unwrap();
        let drop = d.path().join("site-1.md");
        let codex = Told { session: "x1".into(), told_ms: ms - 200_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        fs::write(&drop, "codex's").unwrap();
        fs::File::options().write(true).open(&drop).unwrap().set_modified(ago(150)).unwrap();
        // the app is closed: Claude, asked first, writes in place before Codex's note is moved
        fs::write(note_path(&dir, "site-1"), "claude's").unwrap();
        fs::File::options().write(true).open(note_path(&dir, "site-1")).unwrap().set_modified(ago(100)).unwrap();
        assert!(!collect(&dir, &codex));
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c1"));

        // the app is running: Codex's note is moved first, then Claude writes in place
        fs::remove_file(note_path(&dir, "site-1")).unwrap();
        assert!(collect(&dir, &codex));
        assert_eq!(note(&dir, "site-1").map(|n| n.agent), Some(Agent::Codex));
        fs::write(note_path(&dir, "site-1"), "claude's after").unwrap();
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c1"));
    }

    #[test]
    fn claudes_credit_holds_when_it_is_asked_again_and_while_codexs_drop_waits() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let ago = |s: u64| now - std::time::Duration::from_secs(s);
        let at = |path: &Path, s: u64| fs::File::options().write(true).open(path).unwrap().set_modified(ago(s)).unwrap();
        let told = |session: &str, s: i64, agent: Agent, drop: Option<PathBuf>| Told { session: session.into(), told_ms: ms - s * 1000, cwd: "/w/site".into(), key: "site-1".into(), agent, drop, ..Default::default() };
        mark_told(&dir, &told("c0", 900, Agent::Claude, None)).unwrap();
        mark_told(&dir, &told("c1", 600, Agent::Claude, None)).unwrap();
        let drop = d.path().join("site-1.md");
        let codex = told("x1", 500, Agent::Codex, Some(drop.clone()));
        mark_told(&dir, &codex).unwrap();
        fs::write(&drop, "codex's").unwrap();
        at(&drop, 450);
        fs::write(note_path(&dir, "site-1"), "claude's").unwrap();
        at(&note_path(&dir, "site-1"), 400);
        assert!(!collect(&dir, &codex), "held back for c1's newer note");
        // c1 is asked again in its next window while Codex's drop still waits
        mark_told(&dir, &told("c1", 100, Agent::Claude, None)).unwrap();
        assert!(!collect(&dir, &codex));
        let n = note(&dir, "site-1").unwrap();
        assert_eq!((n.agent, n.session.as_str()), (Agent::Claude, "c1"));
    }

    #[test]
    fn a_note_changed_in_place_by_no_asked_claude_does_not_hold_codexs_back() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let drop = d.path().join("site-1.md");
        let codex = Told { session: "x1".into(), told_ms: ms - 200_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        fs::write(&drop, "codex's").unwrap();
        fs::File::options().write(true).open(&drop).unwrap().set_modified(now - std::time::Duration::from_secs(150)).unwrap();
        fs::write(note_path(&dir, "site-1"), "edited by hand").unwrap();
        assert!(collect(&dir, &codex), "no Claude conversation was asked, so Codex's note is the one asked for");
        assert_eq!(fs::read_to_string(note_path(&dir, "site-1")).unwrap(), "codex's");
    }

    #[test]
    fn a_note_dated_in_the_future_does_not_hold_codexs_back() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = std::time::SystemTime::now();
        let ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let drop = d.path().join("site-1.md");
        let codex = Told { session: "x1".into(), told_ms: ms - 200_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        fs::write(note_path(&dir, "site-1"), "from a clock set ahead").unwrap();
        fs::File::options().write(true).open(note_path(&dir, "site-1")).unwrap().set_modified(now + std::time::Duration::from_secs(86_400)).unwrap();
        fs::write(&drop, "codex's").unwrap();
        assert!(collect(&dir, &codex));
        assert_eq!(fs::read_to_string(note_path(&dir, "site-1")).unwrap(), "codex's");
    }

    #[test]
    fn sending_the_notification_keeps_a_handoff_made_meanwhile() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let t = Told { session: "c1".into(), told_ms: 1, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() };
        mark_told(&dir, &t).unwrap();
        let read_before = told_record(&dir, "c1").unwrap();
        assert_eq!(set_handed(&dir, "site-1"), vec!["c1".to_string()]);
        set_notified(&dir, &read_before).unwrap();
        let now = told_record(&dir, "c1").unwrap();
        assert!(now.notified && now.handed);
    }

    #[cfg(unix)]
    #[test]
    fn notes_left_in_a_folder_that_is_not_the_users_alone_are_never_touched() {
        let d = tempfile::tempdir().unwrap();
        let other = d.path().join("other");
        fs::create_dir(&other).unwrap();
        let link = d.path().join("drop");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(!is_private(&link));
        let dir = d.path().join("handoff");
        let kept = other.join("x.md");
        fs::write(&kept, "x").unwrap();
        mark_told(&dir, &Told { session: "s1".into(), key: "x".into(), drop: Some(link.join("x.md")), ..Default::default() }).unwrap();
        forget(&dir).unwrap();
        assert!(kept.exists());
    }

    #[cfg(unix)]
    #[test]
    fn codex_writes_only_to_a_folder_of_the_user_alone() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let mine = d.path().join("mine");
        private_dir(&mine).unwrap();
        assert_eq!(fs::metadata(&mine).unwrap().permissions().mode() & 0o777, 0o700);
        private_dir(&mine).unwrap();
        let open = d.path().join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&open).is_err());
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&mine, &link).unwrap();
        assert!(private_dir(&link).is_err());
    }

    #[test]
    fn a_codex_note_says_so_in_its_record() {
        let d = tempfile::tempdir().unwrap();
        let t = Told { session: "s1".into(), told_ms: 1, cwd: "/w/api".into(), key: "api-1".into(), agent: Agent::Codex, ..Default::default() };
        mark_told(d.path(), &t).unwrap();
        fs::write(note_path(d.path(), "api-1"), "x").unwrap();
        assert_eq!(notes(d.path())[0].agent, Agent::Codex);
        assert_eq!(told_record(d.path(), "s1").unwrap().agent, Agent::Codex);
        assert!(intro(Path::new("/n.md"), Agent::Codex).contains("Codex usage limits"));
    }

    #[test]
    fn the_hook_output_carries_the_text_for_its_event() {
        let v: Value = serde_json::from_str(&hook_output("PostToolUse", "hi")).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], "hi");
    }
}
