//! The cache refresh hook. Claude Code runs it after each turn (`--cache-hook`); it waits in the
//! background, and when the app asks, it wakes the waiting conversation for one short turn, which
//! reads the cache and starts its lifetime again. A new prompt or the end of the conversation sends
//! the waiting hook away, and a prompt's time is kept so the app can tell the user's own turns from
//! its refreshes. A turn the usage limit stopped is waited on the same way, by a hook of its own
//! (`--limit-hook`), and woken once the limit starts again. The app and the hooks share only small
//! files named by the session id; nothing of the conversation is read.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// A waiter touches its file this often, so the app can tell a live one from one that is gone.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
const STALE: Duration = Duration::from_secs(45);
/// Claude Code may still write to the transcript just after a turn ends.
const SETTLE: Duration = Duration::from_secs(5);
/// A prompt this soon after a wake is the refresh's own turn.
const WOKE_GRACE: Duration = Duration::from_secs(30);
/// Left by a hook that was killed; well past the hook's own time limit.
const WAITER_GONE: Duration = Duration::from_secs(2 * 3600);
/// A prompt's time is kept as long as the app keeps its refresh log.
const PROMPT_KEPT: Duration = Duration::from_secs(7 * 86_400);
/// A refresh's turn is one short reply; a limit turns it down within seconds.
const REFRESH_TURN_MS: i64 = 10 * 60_000;
/// A weekly limit starts again within seven days; what a limit wait leaves is older than any by then.
const LIMIT_GONE: Duration = Duration::from_secs(9 * 86_400);

/// What a hook waits for: a cache refresh after a turn, or the reset of the limit that stopped one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    Cache,
    Limit,
}

impl Wait {
    fn waiter(self) -> &'static str {
        match self {
            Wait::Cache => "waiter",
            Wait::Limit => "limit",
        }
    }

    fn signal(self) -> &'static str {
        match self {
            Wait::Cache => "signal",
            Wait::Limit => "resume",
        }
    }
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("refresh")
}

/// Claude Code's session ids are UUIDs; anything else never names a file.
pub fn valid_session(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn waiter_file(dir: &Path, kind: Wait, session: &str) -> PathBuf {
    dir.join(format!("{}-{session}", kind.waiter()))
}

fn signal_file(dir: &Path, kind: Wait, session: &str) -> PathBuf {
    dir.join(format!("{}-{session}", kind.signal()))
}

fn stopped_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("stopped-{session}"))
}

fn resumed_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("resumed-{session}"))
}

fn prompt_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("prompt-{session}"))
}

fn woke_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("woke-{session}"))
}

fn took_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("took-{session}"))
}

fn unasked_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("unasked-{session}"))
}

/// The conversation lets every tool run without asking, so the hook does not wait in it: a refresh
/// turn must not have that power. Kept so the app can say why it cannot be refreshed.
pub fn unasked(dir: &Path, session: &str) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(unasked_file(dir, session), "");
}

pub fn runs_unasked(dir: &Path, session: &str) -> bool {
    unasked_file(dir, session).exists()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Asked to refresh: the message to wake the conversation with.
    Wake(String),
    /// The conversation went on by itself (a later turn waits in its place).
    Moved,
    /// Claude Code went away, or the time ran out.
    Gone,
}

fn modified(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn remove_if_mine(file: &Path, me: &str) {
    if fs::read_to_string(file).ok().as_deref() == Some(me) {
        let _ = fs::remove_file(file);
    }
}

/// Waits for the app's signal for `session`. `me` tells this waiter from a later one;
/// `transcript` going on (a new message) ends the wait as well; `alive` reports whether
/// Claude Code is still there.
#[allow(clippy::too_many_arguments)]
pub fn wait(dir: &Path, kind: Wait, session: &str, me: &str, transcript: Option<&Path>, tick: Duration, until: Instant, alive: impl Fn() -> bool) -> io::Result<Outcome> {
    fs::create_dir_all(dir)?;
    let mine = waiter_file(dir, kind, session);
    let signal = signal_file(dir, kind, session);
    // a signal left from before this turn ended is out of date
    let _ = fs::remove_file(&signal);
    if kind == Wait::Cache {
        let _ = fs::remove_file(unasked_file(dir, session));
    }
    fs::write(&mine, me)?;
    let started = Instant::now();
    let mut seen: Option<Option<SystemTime>> = None;
    let mut beat = Instant::now();
    loop {
        if fs::read_to_string(&mine).ok().as_deref() != Some(me) {
            return Ok(Outcome::Moved);
        }
        if let Ok(text) = fs::read_to_string(&signal) {
            // removing it claims it, so two waiters never both wake the conversation
            if fs::remove_file(&signal).is_ok() {
                remove_if_mine(&mine, me);
                return Ok(Outcome::Wake(text));
            }
        }
        if let Some(t) = transcript.filter(|_| started.elapsed() >= SETTLE) {
            match seen {
                None => seen = Some(modified(t)),
                Some(before) if modified(t) != before => {
                    remove_if_mine(&mine, me);
                    return Ok(Outcome::Moved);
                }
                Some(_) => {}
            }
        }
        if Instant::now() >= until || !alive() {
            remove_if_mine(&mine, me);
            return Ok(Outcome::Gone);
        }
        if beat.elapsed() >= HEARTBEAT {
            if let Ok(f) = fs::File::options().write(true).open(&mine) {
                let _ = f.set_modified(SystemTime::now());
            }
            beat = Instant::now();
        }
        std::thread::sleep(tick);
    }
}

/// Sends the waiters for `session` away: the conversation went on or ended.
pub fn retire(dir: &Path, session: &str) {
    for kind in [Wait::Cache, Wait::Limit] {
        let _ = fs::remove_file(waiter_file(dir, kind, session));
    }
}

/// Sends away only the waiter for the limit's reset: the user does not want it continued.
pub fn retire_limit(dir: &Path, session: &str) {
    let _ = fs::remove_file(waiter_file(dir, Wait::Limit, session));
}

/// Keeps when the usage limit stopped `session`, so the app can find the limit's reset.
pub fn stopped(dir: &Path, session: &str, now_ms: i64) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(stopped_file(dir, session), now_ms.to_string());
}

pub fn stopped_at(dir: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(stopped_file(dir, session)).ok()?.trim().parse().ok()
}

/// Marks the turn that continues `session` after its limit started again, so its prompt is not
/// taken for the user's, and keeps when, so the app knows the conversation went on.
pub fn resumed(dir: &Path, session: &str, now_ms: i64) {
    let _ = fs::write(woke_file(dir, session), "");
    let _ = fs::write(resumed_file(dir, session), now_ms.to_string());
}

pub fn resumed_at(dir: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(resumed_file(dir, session)).ok()?.trim().parse().ok()
}

/// Every conversation a limit wait continued, with the time, as far as the files still tell.
pub fn all_resumed(dir: &Path) -> Vec<(String, i64)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("resumed-").map(str::to_owned))
        .filter(|s| valid_session(s))
        .filter_map(|s| Some((resumed_at(dir, &s)?, s)))
        .map(|(at, s)| (s, at))
        .collect()
}

/// Whether the turn that just ended was a cache refresh's own: the hook took a refresh signal
/// shortly before, and the user has not written since. A limit that turns such a turn down
/// stopped no work, so there is nothing to continue.
pub fn refresh_turn(dir: &Path, session: &str, now_ms: i64) -> bool {
    took(dir, session).is_some_and(|t| (0..REFRESH_TURN_MS).contains(&(now_ms - t)) && last_prompt(dir, session).is_none_or(|p| p < t))
}

/// Every conversation with a limit hook's file, whether the hook was heard from lately or not:
/// one that slept with the computer has not written yet when it wakes.
pub fn limit_hooks(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> = entries.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_prefix("limit-").map(str::to_owned)).filter(|s| valid_session(s)).collect();
    out.sort();
    out
}

/// The conversations a limit stopped whose hook still waits for the reset.
pub fn limit_waiters(dir: &Path, now: SystemTime) -> Vec<String> {
    limit_hooks(dir).into_iter().filter(|s| waiting(dir, Wait::Limit, s, now)).collect()
}

/// Marks the turn a wake is about to start, so its prompt is not taken for the user's, and keeps
/// when the signal was taken, so the app counts only refreshes that woke the conversation.
pub fn woke(dir: &Path, session: &str, now_ms: i64) {
    let _ = fs::write(woke_file(dir, session), "");
    let _ = fs::write(took_file(dir, session), now_ms.to_string());
}

/// When the hook last took a signal for `session`.
pub fn took(dir: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(took_file(dir, session)).ok()?.trim().parse().ok()
}

/// A prompt was submitted: the waiter goes, and unless it is the turn a wake just started, its time
/// is kept as the user's latest request.
pub fn prompted(dir: &Path, session: &str, now_ms: i64) {
    retire(dir, session);
    let woke = woke_file(dir, session);
    let own = modified(&woke).is_some_and(|t| t.elapsed().unwrap_or_default() < WOKE_GRACE);
    let _ = fs::remove_file(&woke);
    if !own {
        let _ = fs::create_dir_all(dir);
        let _ = fs::write(prompt_file(dir, session), now_ms.to_string());
    }
}

/// Removes every kept prompt time and wake mark.
pub fn forget_prompts(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        if ["prompt-", "woke-", "took-", "unasked-", "stopped-", "resumed-"].iter().any(|p| name.to_string_lossy().starts_with(p)) {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// When the user last submitted a prompt in `session`, as far as the hook saw.
pub fn last_prompt(dir: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(prompt_file(dir, session)).ok()?.trim().parse().ok()
}

/// Whether a waiter for `session` is there to be woken.
pub fn waiting(dir: &Path, kind: Wait, session: &str, now: SystemTime) -> bool {
    modified(&waiter_file(dir, kind, session)).is_some_and(|t| now.duration_since(t).unwrap_or_default() < STALE)
}

/// Asks the waiter for `session` to wake it; a cache refresh is woken with `message`, a limit
/// wait with words of the hook's own.
pub fn signal(dir: &Path, kind: Wait, session: &str, message: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let target = signal_file(dir, kind, session);
    let tmp = dir.join(format!(".{}-{session}-{}", kind.signal(), std::process::id()));
    // written whole first, so a waiter never reads half of it
    fs::write(&tmp, message)?;
    fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// A signal no waiter took within `max_age` will not be taken; it is removed, as are files left by
/// hooks long gone. A live waiter's file is kept even when it looks old: after the computer sleeps,
/// its next heartbeat can come a little later.
pub fn drop_stale_signals(dir: &Path, now: SystemTime, max_age: Duration) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let age = e.metadata().and_then(|m| m.modified()).map_or(Duration::ZERO, |t| now.duration_since(t).unwrap_or_default());
        let name = e.file_name();
        let name = name.to_string_lossy();
        let limit = if ["signal-", ".signal-", "resume-", ".resume-"].iter().any(|p| name.starts_with(p)) {
            max_age
        } else if ["waiter-", "woke-", "took-", "unasked-"].iter().any(|p| name.starts_with(p)) {
            WAITER_GONE
        } else if name.starts_with("prompt-") {
            PROMPT_KEPT
        } else if ["limit-", "stopped-", "resumed-"].iter().any(|p| name.starts_with(p)) {
            LIMIT_GONE
        } else {
            continue;
        };
        if age > limit {
            let _ = fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    const TICK: Duration = Duration::from_millis(10);

    fn later(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn a_waiter_wakes_once_on_the_signal_and_leaves_nothing_behind() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_owned();
        let waiter = {
            let dir = dir.clone();
            std::thread::spawn(move || wait(&dir, Wait::Cache, "s-1", "a", None, TICK, later(5_000), || true).unwrap())
        };
        while !waiting(&dir, Wait::Cache, "s-1", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        signal(&dir, Wait::Cache, "s-1", "refresh please").unwrap();
        assert_eq!(waiter.join().unwrap(), Outcome::Wake("refresh please".into()));
        assert!(!waiting(&dir, Wait::Cache, "s-1", SystemTime::now()));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_later_turn_takes_over_and_an_old_signal_is_not_taken() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_owned();
        signal(&dir, Wait::Cache, "s", "left over").unwrap();
        let first = {
            let dir = dir.clone();
            std::thread::spawn(move || wait(&dir, Wait::Cache, "s", "a", None, TICK, later(5_000), || true).unwrap())
        };
        while fs::read_to_string(waiter_file(&dir, Wait::Cache, "s")).ok().as_deref() != Some("a") {
            std::thread::sleep(TICK);
        }
        assert!(!signal_file(&dir, Wait::Cache, "s").exists(), "the signal from before the turn ended is gone");
        let gone = Arc::new(AtomicBool::new(false));
        let second = {
            let (dir, gone) = (dir.clone(), gone.clone());
            std::thread::spawn(move || wait(&dir, Wait::Cache, "s", "b", None, TICK, later(5_000), move || !gone.load(Ordering::SeqCst)).unwrap())
        };
        assert_eq!(first.join().unwrap(), Outcome::Moved);
        gone.store(true, Ordering::SeqCst);
        assert_eq!(second.join().unwrap(), Outcome::Gone, "Claude Code went away");
        assert!(!waiter_file(&dir, Wait::Cache, "s").exists());
        assert_eq!(wait(&dir, Wait::Cache, "t", "c", None, TICK, Instant::now(), || true).unwrap(), Outcome::Gone, "the time ran out");
    }

    #[test]
    fn a_prompt_sends_the_waiter_away_and_only_the_users_own_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_owned();
        let waiter = {
            let dir = dir.clone();
            std::thread::spawn(move || wait(&dir, Wait::Cache, "s", "a", None, TICK, later(5_000), || true).unwrap())
        };
        while !waiting(&dir, Wait::Cache, "s", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        prompted(&dir, "s", 1_000);
        assert_eq!(waiter.join().unwrap(), Outcome::Moved, "the conversation went on at once");
        assert_eq!(last_prompt(&dir, "s"), Some(1_000));
        unasked(&dir, "s");
        assert!(runs_unasked(&dir, "s"));
        woke(&dir, "s", 1_500);
        assert_eq!(took(&dir, "s"), Some(1_500));
        prompted(&dir, "s", 2_000);
        assert_eq!(last_prompt(&dir, "s"), Some(1_000), "the turn a wake started is not the user's");
        prompted(&dir, "s", 3_000);
        assert_eq!(last_prompt(&dir, "s"), Some(3_000));
    }

    #[test]
    fn a_limit_wait_is_woken_apart_from_a_cache_refresh_and_a_prompt_ends_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_owned();
        let start = |me: &'static str| {
            let dir = dir.clone();
            std::thread::spawn(move || wait(&dir, Wait::Limit, "s", me, None, TICK, later(5_000), || true).unwrap())
        };
        let waiter = start("a");
        while !waiting(&dir, Wait::Limit, "s", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        stopped(&dir, "s", 1_000);
        assert_eq!(limit_waiters(&dir, SystemTime::now()), ["s"]);
        assert!(!waiting(&dir, Wait::Cache, "s", SystemTime::now()), "nothing to refresh in it");
        signal(&dir, Wait::Cache, "s", "refresh").unwrap();
        std::thread::sleep(TICK * 5);
        assert!(waiting(&dir, Wait::Limit, "s", SystemTime::now()), "a cache refresh does not wake it");
        signal(&dir, Wait::Limit, "s", "").unwrap();
        assert_eq!(waiter.join().unwrap(), Outcome::Wake(String::new()));
        assert!(limit_waiters(&dir, SystemTime::now()).is_empty());
        resumed(&dir, "s", 2_000);
        assert_eq!((stopped_at(&dir, "s"), resumed_at(&dir, "s")), (Some(1_000), Some(2_000)));
        assert_eq!(all_resumed(&dir), [("s".to_owned(), 2_000)]);
        prompted(&dir, "s", 2_500);
        assert_eq!(last_prompt(&dir, "s"), None, "the turn that continues it is not the user's");
        // a cache refresh the limit turned down stopped no work
        assert!(!refresh_turn(&dir, "s", 3_000), "no refresh was taken");
        woke(&dir, "s", 2_800);
        assert!(refresh_turn(&dir, "s", 3_000));
        assert!(!refresh_turn(&dir, "s", 2_800 + REFRESH_TURN_MS), "long ago");
        fs::write(prompt_file(&dir, "s"), "2900").unwrap();
        assert!(!refresh_turn(&dir, "s", 3_000), "the user wrote since");
        fs::remove_file(prompt_file(&dir, "s")).unwrap();
        let _ = fs::remove_file(woke_file(&dir, "s"));

        let waiter = start("b");
        while !waiting(&dir, Wait::Limit, "s", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        prompted(&dir, "s", 3_000);
        assert_eq!(waiter.join().unwrap(), Outcome::Moved, "the user went on by themself");
        let waiter = start("c");
        while !waiting(&dir, Wait::Limit, "s", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        retire_limit(&dir, "s");
        assert_eq!(waiter.join().unwrap(), Outcome::Moved, "the user called it off");
        drop_stale_signals(&dir, SystemTime::now() + WAITER_GONE * 2, Duration::from_secs(60));
        assert_eq!(stopped_at(&dir, "s"), Some(1_000), "kept past a refresh wait's lifetime");
        drop_stale_signals(&dir, SystemTime::now() + LIMIT_GONE * 2, Duration::from_secs(60));
        assert_eq!(stopped_at(&dir, "s"), None);
    }

    #[test]
    fn only_session_ids_name_files_and_stale_signals_are_cleared() {
        assert!(valid_session("1425e5e7-8bc3-4140-a831-c06cc4747644"));
        for bad in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            assert!(!valid_session(bad), "{bad}");
        }
        let d = tempfile::tempdir().unwrap();
        signal(d.path(), Wait::Cache, "s", "x").unwrap();
        drop_stale_signals(d.path(), SystemTime::now(), Duration::from_secs(60));
        assert!(signal_file(d.path(), Wait::Cache, "s").exists());
        fs::write(waiter_file(d.path(), Wait::Cache, "s"), "a").unwrap();
        drop_stale_signals(d.path(), SystemTime::now() + Duration::from_secs(120), Duration::from_secs(60));
        assert!(!signal_file(d.path(), Wait::Cache, "s").exists());
        assert!(waiter_file(d.path(), Wait::Cache, "s").exists(), "a waiter after sleep is not taken for gone");
        drop_stale_signals(d.path(), SystemTime::now() + WAITER_GONE * 2, Duration::from_secs(60));
        assert!(!waiter_file(d.path(), Wait::Cache, "s").exists());
    }
}
