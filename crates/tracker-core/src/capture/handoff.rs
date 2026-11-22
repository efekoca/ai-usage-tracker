//! The handoff note. When a conversation's usage limit is nearly full, the hook asks Claude or Codex
//! to write what it did, what is left and the next steps to a note in the app's folder, one per
//! conversation, so the work can go on after the reset or in the other tool. The app never reads a
//! note; it only passes its path on. The hook and the app share small files: the latest limit
//! reading of each conversation, the app's own reading while one account of the tool is known, and
//! a record of each conversation told. Codex's sandbox keeps it out of the app's folder, so Codex writes to
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

/// Each conversation writes a note of its own, named after it, so the file a note is in says who
/// wrote it. Up to 1.7.2 a project had one, named after its key (see [`adopt_project_notes`]).
pub fn note_path(dir: &Path, session: &str) -> PathBuf {
    notes_dir(dir).join(format!("{session}.md"))
}

/// The project and the tool of a conversation's note. Up to 1.7.2, the project's record of whom
/// it asked, under the project's key.
fn meta_path(dir: &Path, name: &str) -> PathBuf {
    notes_dir(dir).join(format!("{name}.json"))
}

/// Up to 1.7.2, who wrote a project's note the app moved over from Codex.
fn writer_path(dir: &Path, key: &str) -> PathBuf {
    notes_dir(dir).join(format!("{key}.writer.json"))
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

/// The latest reading of Codex's main limit by the conversation's own thread, from the end of its
/// log and by the rules the app reads it with: the history a fork starts with is another
/// conversation's, and its readings are that one's. Another limit's reading (`premium`) carries no
/// windows and is passed over.
pub fn codex_reading(log: &Path) -> Option<Reading> {
    let len = fs::metadata(log).ok()?.len();
    let (own, forked) = codex_head(log);
    let start = if len > TAIL { line_start(log, len - TAIL)? } else { 0 };
    // a copy can run past where the read starts
    let limits = crate::sources::codex::own_limits(log, start, own.as_deref(), start > 0 && forked).ok()?;
    let main = |l: &&crate::model::LimitSnapshot| l.limit_id == crate::accounts::MAIN_LIMIT && l.used_pct.is_some() && l.resets_at.is_some();
    let ts = limits.iter().filter(main).map(|l| l.ts_ms).max()?;
    let windows = limits.iter().filter(main).filter(|l| l.ts_ms == ts).map(|l| (l.window.clone(), json!({ "used_percentage": l.used_pct, "resets_at": l.resets_at }))).collect();
    Some(Reading { ts, windows })
}

/// The log's own thread, and whether it starts with history copied from another: Codex names the
/// source in the first `session_meta`, and an older Codex copies the source's own right after it.
/// A start that cannot be read may be a fork.
fn codex_head(log: &Path) -> (Option<String>, bool) {
    let Ok(f) = fs::File::open(log) else { return (None, true) };
    let mut metas = std::io::BufRead::lines(std::io::BufReader::new(f.take(TAIL)))
        .take(2)
        .map_while(Result::ok)
        .map(|l| serde_json::from_str::<Value>(&l).ok().filter(|v| v.get("type").and_then(Value::as_str) == Some("session_meta")).and_then(|v| v.get("payload").cloned()));
    let Some(Some(first)) = metas.next() else { return (None, true) };
    let own = first.get("id").and_then(Value::as_str).map(str::to_owned);
    let forked = first.get("forked_from_id").is_some_and(|f| !f.is_null()) || metas.next().flatten().is_some_and(|p| p.get("id").and_then(Value::as_str) != own.as_deref());
    (own, forked)
}

/// Where the first whole line at or after `at` starts.
fn line_start(log: &Path, at: u64) -> Option<u64> {
    let mut f = fs::File::open(log).ok()?;
    let from = at.checked_sub(1)?;
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL + 1).read_to_end(&mut buf).ok()?;
    buf.iter().position(|b| *b == b'\n').map(|i| from + i as u64 + 1)
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

pub fn mark_told(dir: &Path, t: &Told) -> std::io::Result<()> {
    write_json(&told_file(dir, &t.session), t)?;
    if t.scripted {
        return Ok(());
    }
    write_meta(dir, t)
}

fn write_meta(dir: &Path, t: &Told) -> std::io::Result<()> {
    write_json(&meta_path(dir, &t.session), &json!({ "key": t.key, "cwd": t.cwd, "agent": t.agent }))
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
    /// The conversation that wrote it, which names it.
    pub session: String,
    pub key: String,
    pub cwd: PathBuf,
    pub written_ms: i64,
    pub agent: Agent,
}

pub fn note(dir: &Path, session: &str) -> Option<Note> {
    if !crate::capture::refresh_hook::valid_session(session) {
        return None;
    }
    let meta: Value = read_json(&meta_path(dir, session))?;
    Some(Note {
        key: meta.get("key")?.as_str().filter(|k| valid_key(k))?.to_owned(),
        cwd: PathBuf::from(meta.get("cwd")?.as_str()?),
        written_ms: modified_ms(&note_path(dir, session))?,
        agent: meta.get("agent").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default(),
        session: session.to_owned(),
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
        .filter_map(|s| note(dir, &s))
        .collect();
    out.sort_by_key(|n| std::cmp::Reverse(n.written_ms));
    out
}

/// The conversation wrote its note since it was asked this time. A note moved over from Codex
/// keeps when Codex wrote it, which can be a moment before the request.
pub fn answered(dir: &Path, t: &Told) -> bool {
    note(dir, &t.session).is_some_and(|n| n.written_ms + 2_000 >= t.told_ms)
}

/// A project's note from 1.7.2 or before goes to the conversation it was credited to then: the one
/// its writer record names while it is still that note; the one before the last request while the
/// note is older than that request; after a note moved over from Codex, the Claude conversation
/// asked last before it was changed in place; else the one asked last. A conversation's own newer
/// note stays.
pub fn adopt_project_notes(dir: &Path) {
    for e in fs::read_dir(notes_dir(dir)).into_iter().flatten().flatten() {
        let from = e.path();
        let Some(key) = from.file_stem().and_then(|k| k.to_str()).filter(|k| from.extension().is_some_and(|x| x == "md") && valid_key(k)) else { continue };
        let Some(meta) = read_json::<Value>(&meta_path(dir, key)).filter(|m| m.get("key").is_none()) else { continue };
        let (Some(written), Some(cwd)) = (modified_ms(&from), meta.get("cwd").and_then(Value::as_str)) else { continue };
        let named = read_json::<Value>(&writer_path(dir, key)).filter(|w| w.get("written_ms").and_then(Value::as_i64) == Some(written));
        let told = meta.get("told_ms").and_then(Value::as_i64).unwrap_or(0);
        let moved_over = writer_path(dir, key).exists();
        let codex_asked_last = meta.get("agent").and_then(Value::as_str) == Some("codex");
        let who = match (named, meta.get("before").filter(|b| b.is_object())) {
            (Some(w), _) => w,
            (None, Some(before)) if written + 2_000 < told => before.clone(),
            _ if moved_over && codex_asked_last => all_told(dir)
                .into_iter()
                .find(|t| t.key == key && t.agent == Agent::Claude && !t.scripted && t.told_ms <= written + 2_000)
                .map_or_else(|| meta.clone(), |c| json!({ "session": c.session, "agent": c.agent })),
            _ => meta.clone(),
        };
        let Some(session) = who.get("session").and_then(Value::as_str).filter(|s| crate::capture::refresh_hook::valid_session(s)) else { continue };
        if session == key || modified_ms(&note_path(dir, session)).is_some_and(|own| own >= written) {
            continue;
        }
        let agent = who.get("agent").and_then(|a| serde_json::from_value::<Agent>(a.clone()).ok()).unwrap_or_default();
        let t = Told { session: session.to_owned(), cwd: cwd.into(), key: key.to_owned(), agent, ..Default::default() };
        if write_meta(dir, &t).is_ok() {
            let _ = fs::rename(&from, note_path(dir, session));
        }
    }
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

/// Where a Codex conversation writes its note. None when the folder is not the user's alone: the
/// temp folder is shared on Linux, and Codex's sandbox can write there.
pub fn codex_drop(session: &str) -> Option<PathBuf> {
    let d = drop_dir();
    private_dir(&d).ok()?;
    Some(d.join(format!("{session}.md")))
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
    // 1.7.2 gave a project's conversations one drop: it is moved only for a request still in its
    // window, as then, so a conversation asked long before does not take a later one's note
    let now_ms = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    if from.file_stem().is_none_or(|s| s != t.session.as_str()) && t.resets_ms <= now_ms {
        return false;
    }
    let Ok(mut file) = open_note(from) else { return false };
    let Ok(m) = file.metadata() else { return false };
    #[cfg(unix)]
    let single = std::os::unix::fs::MetadataExt::nlink(&m) == 1;
    #[cfg(not(unix))]
    let single = true;
    let written = m.modified().ok().and_then(|w| w.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
    // a note from an earlier request stays where it is
    if !m.is_file() || !single || written.is_none_or(|w| w + 2_000 < t.told_ms) || m.len() > 1024 * 1024 {
        return false;
    }
    let mut text = Vec::new();
    if file.read_to_end(&mut text).is_err() || write_note(dir, &t.session, &text).is_err() {
        return false;
    }
    let _ = fs::remove_file(from);
    // moving it over is not writing it: the note keeps Codex's time
    if let Ok(w) = m.modified() {
        let _ = fs::File::options().write(true).open(note_path(dir, &t.session)).and_then(|f| f.set_modified(w));
    }
    let _ = write_meta(dir, t);
    true
}

fn modified_ms(path: &Path) -> Option<i64> {
    Some(fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

fn write_note(dir: &Path, session: &str, text: &[u8]) -> std::io::Result<()> {
    let to = note_path(dir, session);
    fs::create_dir_all(notes_dir(dir))?;
    let tmp = notes_dir(dir).join(format!(".{session}.{}", std::process::id()));
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
        fs::write(note_path(&dir, "s1"), "x").unwrap();
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
        assert!(!meta_path(d.path(), "s1").exists());
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
    fn a_fork_reads_only_its_own_limit_also_when_the_copy_runs_past_the_part_read() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = ("0199aaaa-aaaa-7aaa-8aaa-aaaaaaaaaaaa", "0199bbbb-bbbb-7bbb-8bbb-bbbbbbbbbbbb");
        let log = d.path().join(format!("rollout-2026-11-22T10-00-00-{b}.jsonl"));
        let t0 = crate::model::parse_ts_ms("2026-11-22T10:00:00.000Z").unwrap();
        let at = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let meta = |ms: i64, id: &str, from: Option<&str>| json!({"timestamp": at(ms), "type": "session_meta", "payload": {"id": id, "forked_from_id": from, "cwd": "/w"}});
        let limit = |ms: i64, pct: f64| json!({"timestamp": at(ms), "type": "event_msg", "payload": {"type": "token_count", "info": null, "rate_limits": {"limit_id": "codex", "primary": {"used_percent": pct, "window_minutes": 300, "resets_at": 1_894_228_269}, "secondary": {"used_percent": 41.0, "window_minutes": 10080, "resets_at": 1_894_650_450}}}});
        let filler = |ms: i64| json!({"timestamp": at(ms), "type": "response_item", "payload": {"type": "message", "text": "x".repeat(1000)}});
        let write = |lines: &[Value]| fs::write(&log, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        let pct = || codex_reading(&log).map(|r| r.windows["five_hour"]["used_percentage"].as_f64().unwrap());
        let parsed = || crate::sources::codex::parse_file(&log, 0, &Value::Null).unwrap().limits.iter().rfind(|l| l.window == "five_hour").and_then(|l| l.used_pct);

        // B starts from A's history, copied in one burst and ending with A's 96 %
        let mut lines = vec![meta(t0, b, Some(a)), meta(t0 + 1, a, None), limit(t0 + 2, 96.0)];
        write(&lines);
        assert_eq!(pct(), None, "only the copy has a reading");
        lines.truncate(2);
        lines.extend((0..700).map(|i| filler(t0 + 2 + i)));
        lines.push(limit(t0 + 702, 96.0));
        write(&lines);
        assert!(fs::metadata(&log).unwrap().len() > TAIL);
        assert_eq!((pct(), parsed()), (None, None), "a copy longer than the part read");
        lines.push(limit(t0 + 10_000, 40.0));
        write(&lines);
        assert_eq!((pct(), parsed()), (Some(40.0), Some(40.0)), "B's own, after the burst");
        lines.push(limit(t0 + 20_000, 96.0));
        write(&lines);
        assert_eq!(pct(), Some(96.0));
        // B goes on long enough that the part read starts in its own lines
        lines.extend((0..700).map(|i| filler(t0 + 30_000 + i * 2_000)));
        lines.push(limit(t0 + 30_000 + 700 * 2_000, 50.0));
        write(&lines);
        assert_eq!((pct(), parsed()), (Some(50.0), Some(50.0)));

        // an older Codex names no source and copies the source's own start after B's
        let old = [meta(t0, b, None), meta(t0 + 1, a, None), limit(t0 + 2, 96.0)];
        write(&old);
        assert_eq!(pct(), None);
    }

    #[test]
    fn a_note_codex_wrote_after_it_was_asked_is_moved_over_and_an_older_one_or_a_link_is_not() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let drop = d.path().join("drop");
        fs::create_dir_all(&drop).unwrap();
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let from = drop.join("s1.md");
        let t = Told { session: "s1".into(), told_ms: now - 1_000, key: "site-1".into(), agent: Agent::Codex, drop: Some(from.clone()), ..Default::default() };
        assert!(!collect(&dir, &t), "nothing written yet");
        fs::write(&from, "note").unwrap();
        assert!(!collect(&dir, &Told { told_ms: now + 60_000, ..t.clone() }), "written before the request");
        assert!(collect(&dir, &t));
        assert_eq!(fs::read_to_string(note_path(&dir, "s1")).unwrap(), "note");
        assert_eq!(note(&dir, "s1").map(|n| (n.key, n.agent)), Some(("site-1".to_owned(), Agent::Codex)), "moving it over records its project and tool too");
        assert!(!from.exists());
        #[cfg(unix)]
        {
            let secret = d.path().join("secret");
            fs::write(&secret, "x").unwrap();
            fs::remove_file(note_path(&dir, "s1")).unwrap();
            std::os::unix::fs::symlink(&secret, &from).unwrap();
            assert!(!collect(&dir, &t), "a link");
            fs::remove_file(&from).unwrap();
            fs::hard_link(&secret, &from).unwrap();
            assert!(!collect(&dir, &t), "a second name of another file");
            assert!(!note_path(&dir, "s1").exists());
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
    }

    fn set_ms(path: &Path, ms: i64) {
        fs::File::options().write(true).open(path).unwrap().set_modified(UNIX_EPOCH + std::time::Duration::from_millis(ms as u64)).unwrap();
    }

    #[test]
    fn a_note_is_credited_to_the_conversation_that_wrote_it_whoever_was_asked_after() {
        let now = now_ms();
        // (A's tool, B's tool, B's project): A is asked a minute ago, B half a minute ago, and only A writes
        for (a, b, b_key) in [(Agent::Codex, Agent::Codex, "site-1"), (Agent::Claude, Agent::Claude, "site-1"), (Agent::Claude, Agent::Codex, "site-1"), (Agent::Codex, Agent::Codex, "api-2")] {
            let d = tempfile::tempdir().unwrap();
            let dir = d.path().join("handoff");
            let told = |session: &str, ago: i64, agent: Agent, key: &str| Told {
                session: session.into(),
                told_ms: now - ago,
                resets_ms: now + 3_600_000,
                cwd: "/w/site".into(),
                key: key.into(),
                agent,
                drop: (agent == Agent::Codex).then(|| d.path().join(format!("{session}.md"))),
                ..Default::default()
            };
            let (ta, tb) = (told("a", 60_000, a, "site-1"), told("b", 30_000, b, b_key));
            mark_told(&dir, &ta).unwrap();
            mark_told(&dir, &tb).unwrap();
            fs::write(ta.drop.clone().unwrap_or_else(|| note_path(&dir, "a")), "a's").unwrap();
            for t in all_told(&dir) {
                collect(&dir, &t);
            }
            let listed: Vec<(String, Agent)> = notes(&dir).into_iter().map(|n| (n.session, n.agent)).collect();
            assert_eq!(listed, [("a".to_owned(), a)], "{a:?} then {b:?} in {b_key}");
            assert_eq!(fs::read_to_string(note_path(&dir, "a")).unwrap(), "a's");
            assert!((answered(&dir, &ta), answered(&dir, &tb)) == (true, false), "{a:?} then {b:?} in {b_key}");
        }
    }

    #[test]
    fn two_conversations_of_one_project_keep_a_note_each() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = now_ms();
        let drop = d.path().join("x.md");
        let claude = Told { session: "c".into(), told_ms: now - 300_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() };
        let codex = Told { session: "x".into(), told_ms: now - 200_000, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        mark_told(&dir, &claude).unwrap();
        mark_told(&dir, &codex).unwrap();
        // Codex writes first, Claude, asked first, writes last; the app moves Codex's over after both
        fs::write(&drop, "codex's").unwrap();
        set_ms(&drop, now - 150_000);
        fs::write(note_path(&dir, "c"), "claude's").unwrap();
        set_ms(&note_path(&dir, "c"), now - 100_000);
        assert!(collect(&dir, &codex));
        let listed: Vec<(String, Agent, i64)> = notes(&dir).into_iter().map(|n| (n.session, n.agent, n.written_ms)).collect();
        assert_eq!(listed, [("c".to_owned(), Agent::Claude, now - 100_000), ("x".to_owned(), Agent::Codex, now - 150_000)], "the latest first; a moved note keeps when Codex wrote it");
        assert_eq!(fs::read_to_string(note_path(&dir, "x")).unwrap(), "codex's");
        assert!(answered(&dir, &claude) && answered(&dir, &codex));
    }

    #[test]
    fn a_conversation_asked_again_has_answered_only_once_it_writes_again() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = now_ms();
        let first = Told { session: "c".into(), told_ms: now - 6 * 3_600_000, cwd: "/w/site".into(), key: "site-1".into(), ..Default::default() };
        mark_told(&dir, &first).unwrap();
        fs::write(note_path(&dir, "c"), "the first window's").unwrap();
        set_ms(&note_path(&dir, "c"), first.told_ms + 60_000);
        assert!(answered(&dir, &first));
        let again = Told { told_ms: now - 60_000, ..first.clone() };
        mark_told(&dir, &again).unwrap();
        assert!(!answered(&dir, &again), "the note of the window before");
        fs::write(note_path(&dir, "c"), "this window's").unwrap();
        assert!(answered(&dir, &again));
        // Codex's file time can be a moment before the request it answers
        let codex = Told { session: "x".into(), told_ms: now, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(d.path().join("x.md")), ..Default::default() };
        mark_told(&dir, &codex).unwrap();
        fs::write(d.path().join("x.md"), "x").unwrap();
        set_ms(&d.path().join("x.md"), now - 1_500);
        assert!(collect(&dir, &codex) && answered(&dir, &codex));
    }

    #[test]
    fn a_project_drop_of_1_7_2_is_moved_only_for_a_request_still_in_its_window() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let now = now_ms();
        let drop = d.path().join("site-1.md");
        let t = |session: &str, resets_ms: i64| Told { session: session.into(), told_ms: now - 60_000, resets_ms, cwd: "/w/site".into(), key: "site-1".into(), agent: Agent::Codex, drop: Some(drop.clone()), ..Default::default() };
        fs::write(&drop, "x2's").unwrap();
        assert!(!collect(&dir, &t("x1", now - 1)), "an older conversation whose window reset");
        assert!(collect(&dir, &t("x2", now + 3_600_000)));
        assert_eq!(fs::read_to_string(note_path(&dir, "x2")).unwrap(), "x2's");
        // a conversation's own drop is moved whenever it comes
        let own = d.path().join("x3.md");
        fs::write(&own, "x3's").unwrap();
        assert!(collect(&dir, &Told { session: "x3".into(), drop: Some(own), ..t("x3", now - 1) }));
    }

    #[test]
    fn a_project_note_of_1_7_2_goes_to_the_conversation_it_was_credited_to() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("handoff");
        let notes_at = notes_dir(&dir);
        fs::create_dir_all(&notes_at).unwrap();
        let now = now_ms();
        let old = |key: &str, meta: Value, written: i64| {
            fs::write(meta_path(&dir, key), meta.to_string()).unwrap();
            fs::write(notes_at.join(format!("{key}.md")), format!("{key}'s")).unwrap();
            set_ms(&notes_at.join(format!("{key}.md")), written);
        };
        // the conversation asked last; the one before it while the note is older than that request;
        // the writer Codex's move named
        old("site-1", json!({ "cwd": "/w/site", "session": "c1", "told_ms": now - 60_000, "agent": "claude", "before": null }), now - 30_000);
        old("api-2", json!({ "cwd": "/w/api", "session": "c2", "told_ms": now - 60_000, "agent": "claude", "before": { "session": "x1", "agent": "codex" } }), now - 90_000);
        old("web-3", json!({ "cwd": "/w/web", "session": "c3", "told_ms": now - 60_000, "agent": "claude", "before": null }), now - 40_000);
        fs::write(writer_path(&dir, "web-3"), json!({ "session": "x3", "agent": "codex", "written_ms": now - 40_000 }).to_string()).unwrap();
        // Codex was asked last and its note moved over, then the Claude conversation asked before
        // it wrote in place: the writer record no longer matches the note
        mark_told(&dir, &Told { session: "c5".into(), told_ms: now - 200_000, cwd: "/w/app".into(), key: "app-5".into(), ..Default::default() }).unwrap();
        old("app-5", json!({ "cwd": "/w/app", "session": "x5", "told_ms": now - 150_000, "agent": "codex", "before": { "session": "c5", "agent": "claude" } }), now - 50_000);
        fs::write(writer_path(&dir, "app-5"), json!({ "session": "x5", "agent": "codex", "written_ms": now - 120_000 }).to_string()).unwrap();
        // one whose conversation has written a newer note of its own since stays where it is
        old("doc-4", json!({ "cwd": "/w/doc", "session": "c4", "told_ms": now - 60_000, "agent": "claude", "before": null }), now - 30_000);
        mark_told(&dir, &Told { session: "c4".into(), told_ms: now - 20_000, cwd: "/w/doc".into(), key: "doc-4".into(), ..Default::default() }).unwrap();
        fs::write(note_path(&dir, "c4"), "c4's own").unwrap();
        assert_eq!(notes(&dir).len(), 1, "only the note of this version is listed before");
        adopt_project_notes(&dir);
        adopt_project_notes(&dir);
        let listed: Vec<(String, String, Agent, String)> = notes(&dir).into_iter().map(|n| (n.session.clone(), n.key, n.agent, fs::read_to_string(note_path(&dir, &n.session)).unwrap())).collect();
        assert_eq!(
            listed,
            [
                ("c4".to_owned(), "doc-4".to_owned(), Agent::Claude, "c4's own".to_owned()),
                ("c1".to_owned(), "site-1".to_owned(), Agent::Claude, "site-1's".to_owned()),
                ("x3".to_owned(), "web-3".to_owned(), Agent::Codex, "web-3's".to_owned()),
                ("c5".to_owned(), "app-5".to_owned(), Agent::Claude, "app-5's".to_owned()),
                ("x1".to_owned(), "api-2".to_owned(), Agent::Codex, "api-2's".to_owned()),
            ]
        );
        assert_eq!(note(&dir, "c1").unwrap().written_ms, now - 30_000, "moved, not written");
        assert!(notes_at.join("doc-4.md").exists());
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
        fs::write(note_path(d.path(), "s1"), "x").unwrap();
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
