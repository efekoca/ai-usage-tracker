//! Codex rollout parser (`<CODEX_HOME>/{sessions,archived_sessions}/YYYY/MM/DD/rollout-*.jsonl`).
//! Used by both the Codex CLI and the Codex desktop app.
//!
//! Format notes (observed, Codex 0.142–0.152):
//! * `session_meta.payload`: `id`, `cwd`, `originator`, `cli_version`, `parent_thread_id`.
//! * `turn_context.payload.model`: the model for the following turn.
//! * `event_msg` with `payload.type == "token_count"`: `info.last_token_usage` (this request)
//!   and `info.total_token_usage` (cumulative). `input_tokens` *includes* cached input;
//!   `output_tokens` *includes* reasoning. Consecutive events can repeat an unchanged total
//!   (skipped). The cumulative total can drop after compaction, so we sum `last_token_usage`
//!   instead of diffing totals.
//! * `payload.rate_limits`: `primary`/`secondary` `{used_percent, window_minutes, resets_at}`,
//!   `plan_type`, `limit_id`. Windows are classified by `window_minutes`, not by slot.
//!
//! De-dup key: `cx:<session id>:<byte offset of the line>`. Rollouts are append-only, so the
//! key is stable across re-reads and when a file is moved to `archived_sessions`.

use super::{f64_at, i64_at, read_jsonl_from, str_at, u64_at, ParseOutput};
use crate::model::{parse_ts_ms, window_name, Accuracy, LimitSnapshot, Provider, Tokens, Tool, UsageEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const SOURCE: &str = "codex_rollout";

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    session_id: Option<String>,
    cwd: Option<String>,
    originator: Option<String>,
    model: Option<String>,
    last_total: Option<String>,
    cli_version: Option<String>,
}

pub fn parse_file(path: &Path, offset: u64, state: &Value) -> std::io::Result<ParseOutput> {
    let mut st: State = serde_json::from_value(state.clone()).unwrap_or_default();
    if st.session_id.is_none() {
        st.session_id = session_id_from_filename(path);
    }
    let (lines, next) = read_jsonl_from(path, offset)?;
    let mut out = ParseOutput { next_offset: next, ..Default::default() };
    let mut typed = 0u64;

    for line in &lines {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warnings.push(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        let Some(kind) = str_at(v, "type") else { continue };
        typed += 1;
        let p = v.get("payload").unwrap_or(&Value::Null);
        match kind {
            "session_meta" => {
                if let Some(id) = str_at(p, "id") {
                    st.session_id = Some(id.to_owned());
                }
                st.cwd = str_at(p, "cwd").map(str::to_owned).or(st.cwd.take());
                st.originator = str_at(p, "originator").map(str::to_owned).or(st.originator.take());
                st.cli_version = str_at(p, "cli_version").map(str::to_owned).or(st.cli_version.take());
                out.lines_recognised += 1;
            }
            "turn_context" => {
                if let Some(m) = str_at(p, "model") {
                    st.model = Some(m.to_owned());
                }
                if let Some(c) = str_at(p, "cwd") {
                    st.cwd = Some(c.to_owned());
                }
                out.lines_recognised += 1;
            }
            "event_msg" if str_at(p, "type") == Some("token_count") => {
                let ts_ms = str_at(v, "timestamp").and_then(parse_ts_ms);
                let mut recognised = false;
                if let (Some(info), Some(ts_ms)) = (p.get("info").filter(|i| !i.is_null()), ts_ms) {
                    let total_key = info.get("total_token_usage").map(|t| t.to_string());
                    if total_key.is_some() && total_key == st.last_total {
                        recognised = true; // repeated snapshot, nothing new
                    } else if let Some(last) = info.get("last_token_usage") {
                        st.last_total = total_key;
                        out.events.push(event(&st, path, line.offset, ts_ms, last));
                        recognised = true;
                    }
                }
                if let (Some(rl), Some(ts_ms)) = (p.get("rate_limits").filter(|r| !r.is_null()), ts_ms) {
                    rate_limits(rl, ts_ms, &mut out.limits);
                    recognised = true;
                }
                if recognised {
                    out.lines_recognised += 1;
                }
            }
            _ => {}
        }
    }
    if out.lines_total >= 20 && typed == 0 {
        out.warnings.push("unrecognised format: no line has a `type` field".into());
    }
    out.state = serde_json::to_value(&st).unwrap_or(Value::Null);
    Ok(out)
}

fn event(st: &State, path: &Path, offset: u64, ts_ms: i64, last: &Value) -> UsageEvent {
    let input_all = u64_at(last, "input_tokens");
    let cached = u64_at(last, "cached_input_tokens");
    let cache_write = u64_at(last, "cache_write_input_tokens");
    let output = u64_at(last, "output_tokens");
    let reasoning = u64_at(last, "reasoning_output_tokens").min(output);
    let session = st.session_id.clone().unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    UsageEvent {
        key: format!("cx:{session}:{offset}"),
        ts_ms,
        tool: Tool::Codex,
        client: st.originator.clone(),
        model: st.model.clone().unwrap_or_else(|| "unknown".into()),
        project_path: st.cwd.clone(),
        session_id: Some(session),
        tokens: Tokens {
            input: input_all.saturating_sub(cached).saturating_sub(cache_write),
            cache_read: cached,
            cache_write,
            cache_write_1h: 0,
            output,
            reasoning,
        },
        request_input: input_all,
        web_search_requests: 0,
        speed: None,
        service_tier: None,
        inference_geo: None,
        accuracy: Accuracy::Exact,
        source: SOURCE.into(),
    }
}

fn rate_limits(rl: &Value, ts_ms: i64, out: &mut Vec<LimitSnapshot>) {
    let plan = str_at(rl, "plan_type").map(str::to_owned);
    let limit_id = str_at(rl, "limit_id").unwrap_or("codex").to_owned();
    for slot in ["primary", "secondary"] {
        let Some(w) = rl.get(slot).filter(|w| w.is_object()) else { continue };
        let Some(minutes) = i64_at(w, "window_minutes") else { continue };
        out.push(LimitSnapshot {
            ts_ms,
            provider: Provider::OpenAI,
            tool: Tool::Codex,
            account: None,
            limit_id: limit_id.clone(),
            window: window_name(minutes),
            used_pct: f64_at(w, "used_percent"),
            resets_at: i64_at(w, "resets_at"),
            status: str_at(rl, "rate_limit_reached_type").map(|s| format!("reached:{s}")),
            plan: plan.clone(),
            source: SOURCE.into(),
            accuracy: Accuracy::Exact,
        });
    }
}

/// `rollout-2026-06-21T11-36-07-<uuid>.jsonl` → `<uuid>` (fallback before `session_meta` is seen).
fn session_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let rest = stem.strip_prefix("rollout-")?;
    // timestamp is 19 chars: YYYY-MM-DDTHH-MM-SS
    rest.get(20..).filter(|s| s.len() >= 32).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_taken_from_rollout_filename() {
        let p = Path::new("rollout-2026-06-21T11-36-07-019ee952-a384-76a3-bc8c-63d9865696b7.jsonl");
        assert_eq!(session_id_from_filename(p).as_deref(), Some("019ee952-a384-76a3-bc8c-63d9865696b7"));
        assert_eq!(session_id_from_filename(Path::new("other.jsonl")), None);
    }
}
