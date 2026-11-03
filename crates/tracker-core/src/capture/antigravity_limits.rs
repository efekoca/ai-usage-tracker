//! Antigravity limit reads through the `agy` CLI's own `/usage` command in print mode
//! (`agy -p /usage --output-format json`). The command makes no model request and starts no
//! conversation; it uses the user's own CLI and Google sign-in.
//!
//! Output: `command.data.groups[]` (`name`, `buckets[]`), each bucket
//! `{id, window: "5h" | "weekly", remaining_fraction, reset_time (RFC 3339)}`. Gemini models
//! share one pool (`gemini-5h`, `gemini-weekly`), Claude and GPT-OSS another (`3p-…`).

use crate::discovery::Env;
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tool};
use crate::sources::{f64_at, str_at};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const SOURCE: &str = "antigravity_cli_usage";
/// `limit_id` of the pool Gemini models draw from; any other pool holds the remaining models.
pub const GEMINI_POOL: &str = "gemini";

pub fn candidates(env: &Env, configured: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(p) = configured {
        v.push(p.to_owned());
    }
    let agy = super::exe("agy");
    #[cfg(windows)]
    {
        v.extend(env.local.iter().map(|l| l.join("agy").join("bin").join(&agy)));
        v.extend(super::path_dirs().into_iter().map(|d| d.join(&agy)));
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            v.push(PathBuf::from(pf).join("Google").join("antigravity-cli").join(&agy));
        }
    }
    #[cfg(not(windows))]
    v.extend(super::unix_bin_dirs(env).into_iter().map(|d| d.join(&agy)));
    v.retain(|p| p.is_absolute());
    v
}

pub fn find_agy(env: &Env, configured: Option<&Path>) -> Option<PathBuf> {
    candidates(env, configured).into_iter().find(|p| p.is_file())
}

fn window_of(w: &str) -> Option<&'static str> {
    match w.to_ascii_lowercase().as_str() {
        "5h" | "five_hour" => Some("five_hour"),
        "weekly" | "7d" | "seven_day" => Some("seven_day"),
        "daily" | "1d" => Some("one_day"),
        _ => None,
    }
}

/// The pool a bucket belongs to: its id without the window (`gemini-5h` → `gemini`).
fn pool_of(bucket: &Value, group: &Value) -> String {
    let id = str_at(bucket, "id").unwrap_or("");
    match id.rsplit_once('-') {
        Some((pool, _)) if !pool.is_empty() => pool.to_ascii_lowercase(),
        _ => str_at(group, "name").unwrap_or("antigravity").to_ascii_lowercase().replace(' ', "_"),
    }
}

pub fn parse_output(v: &Value, now_ms: i64) -> Vec<LimitSnapshot> {
    let groups = v.pointer("/command/data/groups").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for g in &groups {
        for b in g.get("buckets").and_then(Value::as_array).into_iter().flatten() {
            let Some(window) = str_at(b, "window").and_then(window_of) else { continue };
            let Some(left) = f64_at(b, "remaining_fraction").filter(|f| f.is_finite()) else { continue };
            out.push(LimitSnapshot {
                ts_ms: now_ms,
                provider: Provider::Google,
                tool: Tool::Antigravity,
                account: None,
                limit_id: pool_of(b, g),
                window: window.into(),
                used_pct: Some(((1.0 - left.clamp(0.0, 1.0)) * 100.0 * 100.0).round() / 100.0),
                resets_at: str_at(b, "reset_time").and_then(parse_ts_ms).map(|ms| ms / 1000),
                status: None,
                plan: None,
                source: SOURCE.into(),
                accuracy: Accuracy::Captured,
            });
        }
    }
    out
}

/// The JSON document in the CLI's output; log lines around it are ignored.
fn json_document(stdout: &str) -> Option<Value> {
    stdout.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).find(Value::is_object)
}

/// Without a Google sign-in agy prints `authentication required` to stderr.
pub const NOT_SIGNED_IN: &str = "agy_not_signed_in";
/// It ended with no limits and no word on why: no network, an option it no longer takes, an empty
/// answer. Nothing here says the sign-in is gone.
pub const NO_USAGE: &str = "agy_no_usage";

fn read_output(stdout: &str, stderr: &str, now_ms: i64) -> Result<Vec<LimitSnapshot>, String> {
    let doc = json_document(stdout);
    let failed = doc.as_ref().and_then(|d| str_at(d, "status")).filter(|s| !s.eq_ignore_ascii_case("success"));
    let snaps = doc.as_ref().filter(|_| failed.is_none()).map(|d| parse_output(d, now_ms)).unwrap_or_default();
    if !snaps.is_empty() {
        return Ok(snaps);
    }
    let told = doc.as_ref().and_then(|d| str_at(d, "error")).filter(|e| !e.is_empty());
    if [Some(stderr), told].into_iter().flatten().any(|t| t.to_ascii_lowercase().contains("authentication required")) {
        return Err(NOT_SIGNED_IN.into());
    }
    Err(failed.map_or(NO_USAGE.into(), |status| format!("agy reported {}", told.unwrap_or(status))))
}

pub fn query(bin: &Path, work_dir: &Path, timeout: Duration, now_ms: i64) -> Result<Vec<LimitSnapshot>, String> {
    let mut cmd = Command::new(bin);
    cmd.args(["-p", "/usage", "--output-format", "json", "--print-timeout"])
        .arg(format!("{}s", timeout.as_secs().max(1)))
        .current_dir(work_dir)
        // a read must not update the user's install behind their back
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    super::set_child_path(&mut cmd, bin);
    // its own process group, so ending it also ends what the CLI started
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start agy: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let mut stderr = child.stderr.take().ok_or("no stderr")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stdout).take(4 << 20).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let (err_tx, err_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stderr).take(1 << 16).read_to_end(&mut buf);
        let _ = err_tx.send(buf);
    });
    let out = rx.recv_timeout(timeout + Duration::from_secs(5));
    super::kill_tree(&mut child);
    let out = out.map_err(|_| "agy did not answer in time".to_string())?;
    let errors = err_rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    read_output(&String::from_utf8_lossy(&out), &String::from_utf8_lossy(&errors), now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({"conversation_id": "", "status": "SUCCESS", "num_turns": 0, "command": {"name": "usage", "data": {"groups": [
            {"name": "Gemini Models", "buckets": [
                {"id": "gemini-weekly", "window": "weekly", "remaining_fraction": 0.9950135350227356, "reset_time": "2026-10-12T08:57:30Z"},
                {"id": "gemini-5h", "window": "5h", "remaining_fraction": 0.25, "reset_time": "2026-10-08T11:09:06Z"}]},
            {"name": "Claude and GPT models", "buckets": [
                {"id": "3p-weekly", "window": "weekly", "remaining_fraction": 1, "reset_time": "2026-10-15T06:09:44Z"},
                {"id": "3p-5h", "window": "5h", "remaining_fraction": 1}]}]}}})
    }

    #[test]
    fn each_pool_and_window_becomes_a_reading() {
        let s = parse_output(&sample(), 7);
        let got: Vec<(&str, &str, Option<f64>, Option<i64>)> =
            s.iter().map(|x| (x.limit_id.as_str(), x.window.as_str(), x.used_pct, x.resets_at)).collect();
        assert_eq!(
            got,
            [
                ("gemini", "seven_day", Some(0.5), Some(1_791_795_450)),
                ("gemini", "five_hour", Some(75.0), Some(1_791_457_746)),
                ("3p", "seven_day", Some(0.0), Some(1_792_044_584)),
                ("3p", "five_hour", Some(0.0), None),
            ]
        );
        assert!(s.iter().all(|x| x.provider == Provider::Google && x.accuracy == Accuracy::Captured && x.ts_ms == 7));
    }

    #[test]
    fn unknown_windows_and_odd_values_are_skipped_or_clamped() {
        let v = json!({"command": {"data": {"groups": [{"name": "New Pool", "buckets": [
            {"id": "x", "window": "monthly", "remaining_fraction": 0.5},
            {"window": "5h", "remaining_fraction": -0.2},
            {"id": "p-5h", "window": "5h"}]}]}}});
        let s = parse_output(&v, 1);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].limit_id.as_str(), s[0].used_pct), ("new_pool", Some(100.0)));
        assert!(parse_output(&json!({"status": "SUCCESS"}), 1).is_empty());
    }

    #[test]
    fn only_agys_own_words_count_as_a_sign_out() {
        // what agy 2026-11 prints without a sign-in
        let out = r#"{"conversation_id":"","status":"ERROR","response":"","error":"authentication failed or timed out"}"#;
        let err = "Error: authentication required. Run 'agy' to log in, then retry.\nerror: authentication failed or timed out\n";
        assert_eq!(read_output(out, err, 1), Err(NOT_SIGNED_IN.to_owned()));
        assert_eq!(read_output(out, "error: authentication failed or timed out\n", 1), Err("agy reported authentication failed or timed out".to_owned()), "it may have timed out");
        assert_eq!(read_output("", "error: unknown flag: --output-format\n", 1), Err(NO_USAGE.to_owned()));
        assert_eq!(read_output("", "", 1), Err(NO_USAGE.to_owned()), "no network, no word on why");
        assert_eq!(read_output(r#"{"status":"SUCCESS","command":{"data":{"groups":[]}}}"#, "", 1), Err(NO_USAGE.to_owned()), "signed in with no limits to show");
        assert_eq!(read_output(r#"{"status":"QUOTA"}"#, "", 1), Err("agy reported QUOTA".to_owned()));
        assert_eq!(read_output(&sample().to_string(), "Error: authentication required", 1).map(|s| s.len()), Ok(4), "limits win over a stray line");
    }

    #[cfg(unix)]
    #[test]
    fn a_cli_that_says_so_on_stderr_is_signed_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("agy");
        std::fs::write(&bin, "#!/bin/sh\necho 'Error: authentication required.' >&2\necho '{\"status\":\"ERROR\"}'\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(query(&bin, dir.path(), Duration::from_secs(5), 0), Err(NOT_SIGNED_IN.to_owned()));
    }

    #[test]
    fn log_lines_around_the_document_are_ignored() {
        let text = format!("Fetching usage...\n{}\nbye\n", sample());
        assert_eq!(json_document(&text).unwrap()["status"], "SUCCESS");
        assert!(json_document("no json here").is_none());
    }

    #[test]
    fn install_locations_are_absolute() {
        let home = std::env::temp_dir().join("aiut-home-agy");
        let env = Env { home: Some(home.clone()), local: Some(home.join("AppData").join("Local")), ..Default::default() };
        let c = candidates(&env, None);
        assert!(c.iter().all(|p| p.is_absolute()));
        #[cfg(windows)]
        assert_eq!(c[0], home.join("AppData").join("Local").join("agy").join("bin").join("agy.exe"));
        #[cfg(not(windows))]
        assert!(c.contains(&home.join(".local").join("bin").join("agy")));
    }

    #[test]
    fn a_missing_binary_is_an_error_not_a_panic() {
        let missing = std::env::temp_dir().join("aiut-nope").join(super::super::exe("agy"));
        assert!(query(&missing, &std::env::temp_dir(), Duration::from_secs(1), 0).is_err());
    }
}
