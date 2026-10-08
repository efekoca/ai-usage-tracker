//! Claude plan-limit reads through Claude Code's own `get_usage` control request (opt-in).
//! Uses the user's own Claude Code install and login; this app never touches its credentials,
//! and no model request is made (the call reads plan usage only).
//!
//! Protocol: `claude -p --input-format stream-json --output-format stream-json --verbose`, then
//! one stdin line `{"type":"control_request","request_id":…,"request":{"subtype":"get_usage",
//! "skip_behaviors":true}}`. The matching `control_response` carries `subscription_type`,
//! `rate_limits_available` and `rate_limits.{five_hour,seven_day,seven_day_opus,
//! seven_day_sonnet}` = `{utilization (0–100), resets_at (ISO 8601)}`. Claude Code marks the
//! SDK wrapper of this request experimental, so the parser accepts missing fields and reports a
//! clear error instead of guessing.

use crate::discovery::{claude_config_roots, Env, ExtraPaths};
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tool};
use crate::sources::{f64_at, str_at};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const SOURCE: &str = "claude_code_usage";

/// Windows read from the answer; per-model buckets are left out (they change names often).
const WINDOWS: [&str; 4] = ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet"];

/// Documented opt-outs (code.claude.com/docs/en/data-usage), so a limit read neither reports
/// anything nor updates the install; `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` covers more.
pub const QUIET_ENV: [(&str, &str); 5] = [
    ("DISABLE_TELEMETRY", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
    ("DISABLE_FEEDBACK_COMMAND", "1"),
    ("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", "1"),
    ("DISABLE_AUTOUPDATER", "1"),
];

/// Where a usable `claude` binary may live, best first: an explicit path, the native
/// installer, PATH, the npm package, then the copy the Claude desktop app keeps for its Code tab.
pub fn candidates(env: &Env, configured: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(p) = configured {
        v.push(p.to_owned());
    }
    let claude = super::exe("claude");
    #[cfg(windows)]
    {
        if let Some(h) = &env.home {
            v.push(h.join(".local").join("bin").join(&claude));
        }
        v.extend(super::path_dirs().into_iter().map(|d| d.join(&claude)));
    }
    #[cfg(not(windows))]
    v.extend(super::unix_bin_dirs(env).into_iter().map(|d| d.join(&claude)));
    for root in super::npm_roots(env) {
        let bin = root.join("@anthropic-ai/claude-code/bin");
        v.push(bin.join("claude.exe"));
        if !cfg!(windows) {
            v.push(bin.join("claude"));
        }
    }
    if let Some(r) = &env.roaming {
        // newest version folder first
        let dir = r.join("Claude").join("claude-code");
        let mut versions: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        versions.sort_by_key(|p| std::cmp::Reverse(version_key(p)));
        for p in versions {
            v.push(p.join(&claude));
            // on macOS each version holds `<build>/claude.app`
            #[cfg(target_os = "macos")]
            v.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|b| b.path().join("claude.app/Contents/MacOS/claude")));
        }
    }
    v
}

fn version_key(p: &Path) -> Vec<u64> {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("").split('.').map(|x| x.parse().unwrap_or(0)).collect()
}

pub fn find_claude(env: &Env, configured: Option<&Path>) -> Option<PathBuf> {
    candidates(env, configured).into_iter().find(|p| p.is_file())
}

/// The Claude Code config folder the CLI will use (for `CLAUDE_CONFIG_DIR`, when the user set one).
pub fn config_dir(env: &Env, extra: &ExtraPaths) -> Option<PathBuf> {
    env.claude_config_dir.as_ref()?;
    claude_config_roots(env, extra).into_iter().find(|p| p.is_dir())
}

/// Why a read produced no limits, in words the UI can map.
#[derive(Debug, Clone, PartialEq)]
pub enum UsageError {
    /// Claude Code answered but has no plan limits: not signed in, signed in with an API key,
    /// or the login expired.
    NotAvailable,
    /// Plan limits apply but the usage service gave no numbers this time (it limits how often
    /// it is asked); the last reading stays valid and a later read succeeds.
    Throttled,
    Failed(String),
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsageError::NotAvailable => f.write_str("claude_no_plan_limits"),
            UsageError::Throttled => f.write_str("claude_throttled"),
            UsageError::Failed(e) => f.write_str(e),
        }
    }
}

pub fn parse_response(resp: &Value, now_ms: i64) -> Result<Vec<LimitSnapshot>, UsageError> {
    let available = resp.get("rate_limits_available").and_then(Value::as_bool).unwrap_or(false);
    if !available {
        return Err(UsageError::NotAvailable);
    }
    let Some(limits) = resp.get("rate_limits").filter(|v| v.is_object()) else {
        return Err(UsageError::Throttled);
    };
    let plan = str_at(resp, "subscription_type").map(str::to_owned);
    let mut out = Vec::new();
    for w in WINDOWS {
        let Some(win) = limits.get(w).filter(|v| v.is_object()) else { continue };
        let Some(used) = f64_at(win, "utilization") else { continue };
        out.push(LimitSnapshot {
            ts_ms: now_ms,
            provider: Provider::Anthropic,
            tool: Tool::ClaudeCode,
            account: None,
            limit_id: String::new(),
            window: w.to_owned(),
            used_pct: Some(used.clamp(0.0, 100.0)),
            resets_at: str_at(win, "resets_at").and_then(parse_ts_ms).map(|ms| ms / 1000),
            status: None,
            plan: plan.clone(),
            source: SOURCE.into(),
            accuracy: Accuracy::Captured,
        });
    }
    if out.is_empty() {
        return Err(UsageError::NotAvailable);
    }
    Ok(out)
}

/// Starts Claude Code headless, asks for plan usage and stops it again. `work_dir` should be an
/// empty folder of this app's own, so no project settings or files are involved.
pub fn query(bin: &Path, work_dir: &Path, config_dir: Option<&Path>, timeout: Duration, now_ms: i64) -> Result<Vec<LimitSnapshot>, UsageError> {
    let fail = |e: String| UsageError::Failed(e);
    let mut cmd = Command::new(bin);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        // no MCP servers and no hooks: only the usage answer is needed
        "--strict-mcp-config",
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ])
    .current_dir(work_dir)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .envs(QUIET_ENV);
    if let Some(dir) = config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    super::set_child_path(&mut cmd, bin);
    let mut child = cmd.spawn().map_err(|e| fail(format!("cannot start claude: {e}")))?;
    let mut stdin = child.stdin.take().ok_or_else(|| fail("no stdin".into()))?;
    let stdout = child.stdout.take().ok_or_else(|| fail("no stdout".into()))?;
    let rx = super::json_lines(stdout);
    let result = (|| {
        let req = json!({"type": "control_request", "request_id": "aut-usage", "request": {"subtype": "get_usage", "skip_behaviors": true}});
        writeln!(stdin, "{req}").and_then(|_| stdin.flush()).map_err(|e| fail(format!("claude closed its input: {e}")))?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.checked_duration_since(Instant::now()).ok_or_else(|| fail("claude did not answer in time".into()))?;
            let msg = rx.recv_timeout(left).map_err(|_| fail("claude did not answer in time".into()))?;
            let Some(r) = msg.get("response").filter(|_| str_at(&msg, "type") == Some("control_response")) else { continue };
            if str_at(r, "request_id") != Some("aut-usage") {
                continue;
            }
            if str_at(r, "subtype") == Some("error") {
                return Err(fail(str_at(r, "error").unwrap_or("claude returned an error").to_owned()));
            }
            return parse_response(r.get("response").unwrap_or(&Value::Null), now_ms);
        }
    })();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_windows_and_converts_reset_times() {
        let r = json!({"subscription_type": "max", "rate_limits_available": true, "rate_limits": {
            "five_hour": {"utilization": 42.5, "resets_at": "2026-10-02T20:00:00Z"},
            "seven_day": {"utilization": 31, "resets_at": "2026-10-07T09:00:00.000+00:00"},
            "seven_day_opus": null, "seven_day_oauth_apps": {"utilization": 3, "resets_at": null},
            "model_scoped": [{"display_name": "Fable", "utilization": 10, "resets_at": null}]}});
        let s = parse_response(&r, 7).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].window.as_str(), s[0].used_pct), ("five_hour", Some(42.5)));
        assert_eq!(s[0].resets_at, parse_ts_ms("2026-10-02T20:00:00Z").map(|m| m / 1000));
        assert_eq!(s[1].window, "seven_day");
        assert!(s.iter().all(|x| x.plan.as_deref() == Some("max") && x.accuracy == Accuracy::Captured && x.ts_ms == 7 && x.limit_id.is_empty()));
    }

    #[test]
    fn no_plan_limits_is_its_own_error() {
        // what an expired login or an API-key session answers
        let r = json!({"subscription_type": null, "rate_limits_available": false, "rate_limits": null});
        assert_eq!(parse_response(&r, 1), Err(UsageError::NotAvailable));
        assert_eq!(parse_response(&json!({}), 1), Err(UsageError::NotAvailable));
    }

    #[test]
    fn limits_that_apply_but_are_not_given_are_a_throttled_read() {
        // what Claude Code answers when asked again too soon
        let r = json!({"subscription_type": "max", "rate_limits_available": true, "rate_limits": null});
        assert_eq!(parse_response(&r, 1), Err(UsageError::Throttled));
    }

    #[test]
    fn a_missing_binary_is_an_error_not_a_panic() {
        let missing = std::env::temp_dir().join("aiut-nope").join(super::super::exe("claude"));
        let r = query(&missing, &std::env::temp_dir(), None, Duration::from_secs(1), 0);
        assert!(matches!(r, Err(UsageError::Failed(_))));
    }

    #[cfg(not(windows))]
    #[test]
    fn node_version_manager_installs_are_candidates_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_owned();
        for d in [".nvm/versions/node/v9.11.2/bin", ".nvm/versions/node/v22.3.0/bin", ".local/share/fnm/node-versions/v20.1.0/installation/bin"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let env = Env { home: Some(home.clone()), roaming: None, local: None, claude_config_dir: None, codex_home: None, antigravity_data_dir: None };
        let c = candidates(&env, None);
        let at = |p: &str| c.iter().position(|x| x == &home.join(p)).unwrap_or_else(|| panic!("{p} missing"));
        assert!(at(".nvm/versions/node/v22.3.0/bin/claude") < at(".nvm/versions/node/v9.11.2/bin/claude"));
        at(".volta/bin/claude");
        at(".bun/bin/claude");
        at(".local/share/fnm/node-versions/v20.1.0/installation/bin/claude");
    }

    #[cfg(not(windows))]
    #[test]
    fn native_homebrew_and_npm_claude_installs_are_candidates() {
        let home = std::env::temp_dir().join("aiut-home-unix");
        let env = Env { home: Some(home.clone()), roaming: None, local: None, claude_config_dir: None, codex_home: None, antigravity_data_dir: None };
        let c = candidates(&env, None);
        for p in [
            home.join(".local/bin/claude"),
            PathBuf::from("/opt/homebrew/bin/claude"),
            PathBuf::from("/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/bin/claude"),
        ] {
            assert!(c.contains(&p), "{} missing", p.display());
        }
        assert!(c.iter().all(|p| p.extension().is_none_or(|e| e == "exe")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_claude_desktop_apps_copy_is_a_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Claude/claude-code/2.1.288/48d54124d3c3/claude.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        let env = Env { home: None, roaming: Some(dir.path().to_owned()), local: None, claude_config_dir: None, codex_home: None, antigravity_data_dir: None };
        assert!(candidates(&env, None).contains(&app.join("claude")));
    }

    #[test]
    fn newest_desktop_copy_comes_first() {
        let mut v = [PathBuf::from("x/2.1.9"), PathBuf::from("x/2.1.280"), PathBuf::from("x/2.1.75")];
        v.sort_by_key(|p| std::cmp::Reverse(version_key(p)));
        assert_eq!(v[0], PathBuf::from("x/2.1.280"));
    }
}
