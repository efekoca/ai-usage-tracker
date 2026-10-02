//! Codex limit reads through the documented `codex app-server` JSON-RPC interface (opt-in).
//! Uses the user's own Codex install and login; this app never touches Codex credentials.
//!
//! Protocol (JSONL over stdio, `"jsonrpc"` omitted): `initialize` → `initialized` notification →
//! `account/rateLimits/read`, whose result holds `rateLimits` and `rateLimitsByLimitId` with
//! `primary`/`secondary` `{usedPercent, windowDurationMins, resetsAt (epoch s)}` and `planType`.

use crate::discovery::{codex_homes, Env, ExtraPaths};
use crate::model::{window_name, Accuracy, LimitSnapshot, Provider, Tool};
use crate::sources::{f64_at, i64_at, str_at};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const SOURCE: &str = "codex_app_server";

/// Where a usable `codex` binary may live, best first.
pub fn candidates(env: &Env, extra: &ExtraPaths, configured: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(p) = configured {
        v.push(p.to_owned());
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            v.push(dir.join("codex.exe"));
        }
    }
    for home in codex_homes(env, extra) {
        // the Codex desktop app keeps a runnable copy of the CLI here
        v.push(home.join(".sandbox-bin").join("codex.exe"));
    }
    if let Some(r) = &env.roaming {
        v.push(r.join("npm/node_modules/@openai/codex/vendor/x86_64-pc-windows-msvc/codex/codex.exe"));
    }
    v
}

pub fn find_codex(env: &Env, extra: &ExtraPaths, configured: Option<&Path>) -> Option<PathBuf> {
    candidates(env, extra, configured).into_iter().find(|p| p.is_file())
}

/// Maps an `account/rateLimits/read` result to snapshots.
pub fn parse_result(result: &Value, now_ms: i64) -> Vec<LimitSnapshot> {
    let mut snaps = Vec::new();
    let buckets: Vec<&Value> = match result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        Some(m) if !m.is_empty() => m.values().collect(),
        _ => result.get("rateLimits").into_iter().collect(),
    };
    for b in buckets {
        let limit_id = str_at(b, "limitId").unwrap_or("codex").to_owned();
        let plan = str_at(b, "planType").map(str::to_owned);
        for slot in ["primary", "secondary"] {
            let Some(w) = b.get(slot).filter(|w| w.is_object()) else { continue };
            // classify by duration, not slot: `secondary` can be absent
            let Some(minutes) = i64_at(w, "windowDurationMins") else { continue };
            snaps.push(LimitSnapshot {
                ts_ms: now_ms,
                provider: Provider::OpenAI,
                tool: Tool::Codex,
                account: None,
                limit_id: limit_id.clone(),
                window: window_name(minutes),
                used_pct: f64_at(w, "usedPercent"),
                resets_at: i64_at(w, "resetsAt"),
                status: str_at(b, "rateLimitReachedType").map(|s| format!("reached:{s}")),
                plan: plan.clone(),
                source: SOURCE.into(),
                accuracy: Accuracy::Captured,
            });
        }
    }
    snaps
}

/// Starts `codex app-server`, reads the limits and stops it again.
pub fn query(bin: &Path, timeout: Duration, now_ms: i64) -> Result<Vec<LimitSnapshot>, String> {
    let mut cmd = Command::new(bin);
    cmd.arg("app-server").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start codex: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && tx.send(v).is_err()
            {
                break;
            }
        }
    });
    let send = |stdin: &mut std::process::ChildStdin, v: Value| -> Result<(), String> {
        writeln!(stdin, "{v}").and_then(|_| stdin.flush()).map_err(|e| format!("codex closed its input: {e}"))
    };
    let deadline = Instant::now() + timeout;
    let wait_for = |id: i64| -> Result<Value, String> {
        loop {
            let left = deadline.checked_duration_since(Instant::now()).ok_or("codex did not answer in time")?;
            let msg = rx.recv_timeout(left).map_err(|_| "codex did not answer in time".to_string())?;
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(str_at(err, "message").unwrap_or("codex returned an error").to_owned());
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    };
    let result = (|| {
        send(&mut stdin, json!({"id": 0, "method": "initialize", "params": {"clientInfo": {"name": "ai_usage_tracker", "title": "AI Usage Tracker", "version": env!("CARGO_PKG_VERSION")}}}))?;
        wait_for(0)?;
        send(&mut stdin, json!({"method": "initialized", "params": {}}))?;
        send(&mut stdin, json!({"id": 1, "method": "account/rateLimits/read"}))?;
        wait_for(1)
    })();
    let _ = child.kill();
    let _ = child.wait();
    let result = result?;
    let snaps = parse_result(&result, now_ms);
    if snaps.is_empty() {
        return Err("codex returned no limit windows (not signed in with ChatGPT?)".into());
    }
    Ok(snaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_buckets_by_window_duration() {
        let r = json!({"rateLimits": {"limitId": "codex"}, "rateLimitsByLimitId": {"codex": {
            "limitId": "codex", "planType": "plus",
            "primary": {"usedPercent": 12, "windowDurationMins": 300, "resetsAt": 1790973694},
            "secondary": {"usedPercent": 66, "windowDurationMins": 10080, "resetsAt": 1791386558},
            "rateLimitReachedType": null}}});
        let s = parse_result(&r, 5);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].window.as_str(), s[0].used_pct, s[0].resets_at), ("five_hour", Some(12.0), Some(1790973694)));
        assert_eq!(s[1].window, "seven_day");
        assert!(s.iter().all(|x| x.plan.as_deref() == Some("plus") && x.accuracy == Accuracy::Captured && x.ts_ms == 5));
    }

    #[test]
    fn falls_back_to_the_single_bucket_and_tolerates_a_missing_secondary() {
        let r = json!({"rateLimits": {"primary": {"usedPercent": 3, "windowDurationMins": 10080}, "secondary": null}});
        let s = parse_result(&r, 1);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].window, "seven_day");
        assert!(parse_result(&json!({}), 1).is_empty());
    }

    #[test]
    fn a_missing_binary_is_an_error_not_a_panic() {
        assert!(query(Path::new("Z:/nope/codex.exe"), Duration::from_secs(1), 0).is_err());
    }
}
