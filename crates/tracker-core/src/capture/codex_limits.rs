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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const SOURCE: &str = "codex_app_server";

/// Install locations only: a Codex home added as a log folder is never run from.
pub fn candidates(env: &Env, configured: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(p) = configured {
        v.push(p.to_owned());
    }
    let codex = super::exe("codex");
    #[cfg(windows)]
    v.extend(super::path_dirs().into_iter().map(|d| d.join(&codex)));
    #[cfg(not(windows))]
    v.extend(super::unix_bin_dirs(env).into_iter().map(|d| d.join(&codex)));
    if let Some(home) = codex_homes(env, &ExtraPaths::default()).into_iter().next() {
        // the Codex desktop app keeps a runnable copy of the CLI here
        v.push(home.join(".sandbox-bin").join(&codex));
    }
    #[cfg(target_os = "macos")]
    {
        // the Codex desktop app bundles the CLI
        let bundled = "Codex.app/Contents/Resources/codex";
        v.push(Path::new("/Applications").join(bundled));
        v.extend(env.home.iter().map(|h| h.join("Applications").join(bundled)));
    }
    for root in super::npm_roots(env) {
        v.push(root.join("@openai/codex/vendor").join(CODEX_TARGET).join("codex").join(&codex));
    }
    v
}

/// The platform folder inside the npm package's `vendor`.
const CODEX_TARGET: &str = if cfg!(windows) {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "aarch64-apple-darwin"
} else if cfg!(target_os = "macos") {
    "x86_64-apple-darwin"
} else if cfg!(target_arch = "aarch64") {
    "aarch64-unknown-linux-musl"
} else {
    "x86_64-unknown-linux-musl"
};

pub fn find_codex(env: &Env, configured: Option<&Path>) -> Option<PathBuf> {
    candidates(env, configured).into_iter().find(|p| p.is_file())
}

pub fn parse_result(result: &Value, now_ms: i64) -> Vec<LimitSnapshot> {
    let mut snaps = Vec::new();
    let buckets: Vec<(Option<&str>, &Value)> = match result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        Some(m) if !m.is_empty() => m.iter().map(|(k, b)| (Some(k.as_str()), b)).collect(),
        _ => result.get("rateLimits").map(|b| (None, b)).into_iter().collect(),
    };
    for (key, b) in buckets {
        let limit_id = str_at(b, "limitId").filter(|s| !s.is_empty()).or(key).unwrap_or("codex").to_owned();
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

pub fn query(bin: &Path, timeout: Duration, now_ms: i64) -> Result<Vec<LimitSnapshot>, String> {
    let mut cmd = Command::new(bin);
    // even when the user's config.toml turns analytics on
    cmd.args(["-c", "analytics.enabled=false", "app-server"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    super::set_child_path(&mut cmd, bin);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start codex: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let rx = super::json_lines(stdout);
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
    fn a_bucket_without_its_own_id_takes_the_map_key() {
        let w = json!({"usedPercent": 1, "windowDurationMins": 300});
        let r = json!({"rateLimitsByLimitId": {"codex": {"limitId": "codex", "primary": w}, "codex_other": {"limitId": null, "primary": w}}});
        let mut ids: Vec<String> = parse_result(&r, 1).into_iter().map(|s| s.limit_id).collect();
        ids.sort();
        assert_eq!(ids, ["codex", "codex_other"]);
    }

    #[test]
    fn only_the_default_codex_home_is_an_executable_location() {
        let home = std::env::temp_dir().join("aiut-home");
        let env = Env { home: Some(home.clone()), roaming: None, local: None, claude_config_dir: None, codex_home: None };
        let c = candidates(&env, None);
        let sandbox: Vec<&PathBuf> = c.iter().filter(|p| p.starts_with(home.join(".codex"))).collect();
        assert_eq!(sandbox, [&home.join(".codex").join(".sandbox-bin").join(super::super::exe("codex"))]);
        assert!(c.iter().all(|p| p.is_absolute()));
    }

    #[test]
    fn a_missing_binary_is_an_error_not_a_panic() {
        let missing = std::env::temp_dir().join("aiut-nope").join(super::super::exe("codex"));
        assert!(query(&missing, Duration::from_secs(1), 0).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn homebrew_and_npm_codex_installs_are_candidates() {
        let home = std::env::temp_dir().join("aiut-home-unix");
        let env = Env { home: Some(home.clone()), roaming: None, local: None, claude_config_dir: None, codex_home: None };
        let c = candidates(&env, None);
        for p in [
            home.join(".local/bin/codex"),
            PathBuf::from("/opt/homebrew/bin/codex"),
            PathBuf::from("/usr/local/bin/codex"),
            PathBuf::from(format!("/opt/homebrew/lib/node_modules/@openai/codex/vendor/{CODEX_TARGET}/codex/codex")),
            home.join(format!(".npm-global/lib/node_modules/@openai/codex/vendor/{CODEX_TARGET}/codex/codex")),
            #[cfg(target_os = "macos")]
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
        ] {
            assert!(c.contains(&p), "{} missing", p.display());
        }
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(CODEX_TARGET, "aarch64-apple-darwin");
        }
    }
}
