//! Claude Code transcript parser (`<config>/projects/**/*.jsonl`), also used for Cowork sessions.
//!
//! Format notes (observed, Claude Code 2.1.x):
//! * Every `type: "assistant"` line carries `message.usage`. A single API response is written
//!   once per content block, so the same `message.id` appears several times; intermediate
//!   streaming copies can carry smaller output counts or zeroed top-level usage while
//!   `usage.iterations[]` holds the real values. We therefore take the field-wise maximum
//!   of the top-level usage and the iteration sum, and the store merges duplicates with max.
//! * Resumed/forked sessions copy earlier messages into a new file → de-dup must be global.
//! * `model: "<synthetic>"` lines are client-side error placeholders with zero usage.
//! * `quotaLimits` appears on a line when a plan limit rejected the request.

use super::{read_jsonl_from, str_at, u64_at, i64_at, ParseOutput};
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tokens, Tool, UsageEvent};
use serde_json::{json, Value};
use std::path::Path;

pub struct ClaudeCtx<'a> {
    /// Overrides the `entrypoint` client label (used for Cowork sessions).
    pub client_override: Option<&'a str>,
    pub source: &'a str,
}

pub fn parse_file(path: &Path, offset: u64, state: &Value, ctx: &ClaudeCtx) -> std::io::Result<ParseOutput> {
    let (lines, next) = read_jsonl_from(path, offset)?;
    let mut out = ParseOutput { next_offset: next, ..Default::default() };
    let mut max_version = str_at(state, "max_version").map(str::to_owned);
    let mut typed = 0u64;
    for line in &lines {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warnings.push(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        if v.get("type").is_some() {
            typed += 1;
        }
        if let Some(ver) = str_at(v, "version")
            && max_version.as_deref().is_none_or(|m| version_gt(ver, m))
        {
            max_version = Some(ver.to_owned());
        }
        let mut recognised = false;
        if let Some(ev) = usage_event(v, ctx) {
            out.events.push(ev);
            recognised = true;
        }
        if let Some(ls) = quota_limit(v, ctx) {
            out.limits.push(ls);
            recognised = true;
        }
        if recognised {
            out.lines_recognised += 1;
        }
    }
    if out.lines_total >= 20 && typed == 0 {
        out.warnings.push("unrecognised format: no line has a `type` field".into());
    }
    out.state = json!({ "max_version": max_version });
    Ok(out)
}

fn usage_tokens(u: &Value) -> Tokens {
    let cache_1h = u.get("cache_creation").map(|c| u64_at(c, "ephemeral_1h_input_tokens")).unwrap_or(0);
    let reasoning = u.get("output_tokens_details").map(|d| u64_at(d, "thinking_tokens")).unwrap_or(0);
    Tokens {
        input: u64_at(u, "input_tokens"),
        cache_read: u64_at(u, "cache_read_input_tokens"),
        cache_write: u64_at(u, "cache_creation_input_tokens"),
        cache_write_1h: cache_1h,
        output: u64_at(u, "output_tokens"),
        reasoning,
    }
}

pub(crate) fn usage_event(v: &Value, ctx: &ClaudeCtx) -> Option<UsageEvent> {
    if str_at(v, "type") != Some("assistant") {
        return None;
    }
    let msg = v.get("message")?;
    let usage = msg.get("usage")?.as_object()?;
    let usage = Value::Object(usage.clone());
    let model = str_at(msg, "model").unwrap_or("unknown");
    if model == "<synthetic>" || v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        return None;
    }

    let mut tokens = usage_tokens(&usage);
    if let Some(iters) = usage.get("iterations").and_then(Value::as_array) {
        let mut sum = Tokens::default();
        for it in iters {
            sum.add(&usage_tokens(it));
        }
        // iteration objects don't carry thinking details; keep the top-level figure
        sum.reasoning = tokens.reasoning;
        tokens = tokens.max(&sum);
    }
    tokens.cache_write_1h = tokens.cache_write_1h.min(tokens.cache_write);
    tokens.reasoning = tokens.reasoning.min(tokens.output);

    let key = if let Some(id) = str_at(msg, "id") {
        format!("cc:{id}")
    } else if let Some(rid) = str_at(v, "requestId") {
        format!("ccr:{rid}")
    } else {
        format!("ccu:{}", str_at(v, "uuid")?)
    };
    let ts_ms = str_at(v, "timestamp").and_then(parse_ts_ms)?;
    let web_search = usage.get("server_tool_use").map(|s| u64_at(s, "web_search_requests")).unwrap_or(0) as u32;

    Some(UsageEvent {
        key,
        ts_ms,
        tool: Tool::ClaudeCode,
        client: ctx.client_override.map(str::to_owned).or_else(|| str_at(v, "entrypoint").map(str::to_owned)),
        model: model.to_owned(),
        project_path: str_at(v, "cwd").map(str::to_owned),
        session_id: str_at(v, "sessionId").map(str::to_owned),
        request_input: tokens.input + tokens.cache_read + tokens.cache_write,
        tokens,
        web_search_requests: web_search,
        speed: str_at(&usage, "speed").map(str::to_owned),
        service_tier: str_at(&usage, "service_tier").map(str::to_owned),
        inference_geo: str_at(&usage, "inference_geo").map(str::to_owned),
        request_id: str_at(v, "requestId").map(str::to_owned),
        accuracy: Accuracy::Exact,
        source: ctx.source.to_owned(),
    })
}

fn quota_limit(v: &Value, ctx: &ClaudeCtx) -> Option<LimitSnapshot> {
    let q = v.get("quotaLimits")?;
    let status = str_at(q, "status").map(str::to_owned);
    let window = str_at(q, "rateLimitType").unwrap_or("unknown").to_owned();
    let ts_ms = str_at(v, "timestamp").and_then(parse_ts_ms)?;
    Some(LimitSnapshot {
        ts_ms,
        provider: Provider::Anthropic,
        tool: Tool::ClaudeCode,
        account: None,
        limit_id: String::new(),
        window,
        used_pct: if status.as_deref() == Some("rejected") { Some(100.0) } else { None },
        resets_at: i64_at(q, "resetsAt"),
        status,
        plan: None,
        source: ctx.source.to_owned(),
        accuracy: Accuracy::Exact,
    })
}

/// Compares dotted numeric versions ("2.1.280" > "2.1.97").
fn version_gt(a: &str, b: &str) -> bool {
    let p = |s: &str| s.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse::<u64>().ok()).collect::<Vec<_>>();
    p(a) > p(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(version_gt("2.1.280", "2.1.97"));
        assert!(!version_gt("2.1.9", "2.1.10"));
    }
}
