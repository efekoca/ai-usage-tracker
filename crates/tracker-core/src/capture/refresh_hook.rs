//! The cache refresh hook. Claude Code runs it after each turn (`--cache-hook`); it waits in the
//! background, and when the app asks, it wakes the waiting conversation for one short turn, which
//! reads the cache and starts its lifetime again. A new prompt or the end of the conversation sends
//! the waiting hook away, and a prompt's time is kept so the app can tell the user's own turns from
//! its refreshes. The app and the hook share only small files named by the session id; nothing of
//! the conversation is read.

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

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("refresh")
}

/// Claude Code's session ids are UUIDs; anything else never names a file.
pub fn valid_session(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn waiter_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("waiter-{session}"))
}

fn signal_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("signal-{session}"))
}

fn prompt_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("prompt-{session}"))
}

fn woke_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("woke-{session}"))
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
pub fn wait(dir: &Path, session: &str, me: &str, transcript: Option<&Path>, tick: Duration, until: Instant, alive: impl Fn() -> bool) -> io::Result<Outcome> {
    fs::create_dir_all(dir)?;
    let mine = waiter_file(dir, session);
    let signal = signal_file(dir, session);
    // a signal left from before this turn ended is out of date
    let _ = fs::remove_file(&signal);
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

/// Sends the waiter for `session` away: the conversation went on or ended.
pub fn retire(dir: &Path, session: &str) {
    let _ = fs::remove_file(waiter_file(dir, session));
}

/// Marks the turn a wake is about to start, so its prompt is not taken for the user's.
pub fn woke(dir: &Path, session: &str) {
    let _ = fs::write(woke_file(dir, session), "");
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
        if name.to_string_lossy().starts_with("prompt-") || name.to_string_lossy().starts_with("woke-") {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// When the user last submitted a prompt in `session`, as far as the hook saw.
pub fn last_prompt(dir: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(prompt_file(dir, session)).ok()?.trim().parse().ok()
}

/// Whether a waiter for `session` is there to be woken.
pub fn waiting(dir: &Path, session: &str, now: SystemTime) -> bool {
    modified(&waiter_file(dir, session)).is_some_and(|t| now.duration_since(t).unwrap_or_default() < STALE)
}

/// Asks the waiter for `session` to wake it with `message`.
pub fn signal(dir: &Path, session: &str, message: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let target = signal_file(dir, session);
    let tmp = dir.join(format!(".signal-{session}-{}", std::process::id()));
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
        let limit = if name.starts_with("signal-") || name.starts_with(".signal-") {
            max_age
        } else if name.starts_with("waiter-") || name.starts_with("woke-") {
            WAITER_GONE
        } else if name.starts_with("prompt-") {
            PROMPT_KEPT
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
            std::thread::spawn(move || wait(&dir, "s-1", "a", None, TICK, later(5_000), || true).unwrap())
        };
        while !waiting(&dir, "s-1", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        signal(&dir, "s-1", "refresh please").unwrap();
        assert_eq!(waiter.join().unwrap(), Outcome::Wake("refresh please".into()));
        assert!(!waiting(&dir, "s-1", SystemTime::now()));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_later_turn_takes_over_and_an_old_signal_is_not_taken() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_owned();
        signal(&dir, "s", "left over").unwrap();
        let first = {
            let dir = dir.clone();
            std::thread::spawn(move || wait(&dir, "s", "a", None, TICK, later(5_000), || true).unwrap())
        };
        while fs::read_to_string(waiter_file(&dir, "s")).ok().as_deref() != Some("a") {
            std::thread::sleep(TICK);
        }
        assert!(!signal_file(&dir, "s").exists(), "the signal from before the turn ended is gone");
        let gone = Arc::new(AtomicBool::new(false));
        let second = {
            let (dir, gone) = (dir.clone(), gone.clone());
            std::thread::spawn(move || wait(&dir, "s", "b", None, TICK, later(5_000), move || !gone.load(Ordering::SeqCst)).unwrap())
        };
        assert_eq!(first.join().unwrap(), Outcome::Moved);
        gone.store(true, Ordering::SeqCst);
        assert_eq!(second.join().unwrap(), Outcome::Gone, "Claude Code went away");
        assert!(!waiter_file(&dir, "s").exists());
        assert_eq!(wait(&dir, "t", "c", None, TICK, Instant::now(), || true).unwrap(), Outcome::Gone, "the time ran out");
    }

    #[test]
    fn a_prompt_sends_the_waiter_away_and_only_the_users_own_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_owned();
        let waiter = {
            let dir = dir.clone();
            std::thread::spawn(move || wait(&dir, "s", "a", None, TICK, later(5_000), || true).unwrap())
        };
        while !waiting(&dir, "s", SystemTime::now()) {
            std::thread::sleep(TICK);
        }
        prompted(&dir, "s", 1_000);
        assert_eq!(waiter.join().unwrap(), Outcome::Moved, "the conversation went on at once");
        assert_eq!(last_prompt(&dir, "s"), Some(1_000));
        woke(&dir, "s");
        prompted(&dir, "s", 2_000);
        assert_eq!(last_prompt(&dir, "s"), Some(1_000), "the turn a wake started is not the user's");
        prompted(&dir, "s", 3_000);
        assert_eq!(last_prompt(&dir, "s"), Some(3_000));
    }

    #[test]
    fn only_session_ids_name_files_and_stale_signals_are_cleared() {
        assert!(valid_session("1425e5e7-8bc3-4140-a831-c06cc4747644"));
        for bad in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            assert!(!valid_session(bad), "{bad}");
        }
        let d = tempfile::tempdir().unwrap();
        signal(d.path(), "s", "x").unwrap();
        drop_stale_signals(d.path(), SystemTime::now(), Duration::from_secs(60));
        assert!(signal_file(d.path(), "s").exists());
        fs::write(waiter_file(d.path(), "s"), "a").unwrap();
        drop_stale_signals(d.path(), SystemTime::now() + Duration::from_secs(120), Duration::from_secs(60));
        assert!(!signal_file(d.path(), "s").exists());
        assert!(waiter_file(d.path(), "s").exists(), "a waiter after sleep is not taken for gone");
        drop_stale_signals(d.path(), SystemTime::now() + WAITER_GONE * 2, Duration::from_secs(60));
        assert!(!waiter_file(d.path(), "s").exists());
    }
}
