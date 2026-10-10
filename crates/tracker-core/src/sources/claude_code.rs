//! Claude Code transcript parser (`<config>/projects/**/*.jsonl`), also used for Cowork sessions.
//!
//! Format notes (observed, Claude Code 2.1.x):
//! * Every `type: "assistant"` line carries `message.usage`. A single API response is written
//!   once per content block, so the same `message.id` appears several times; intermediate
//!   streaming copies can carry smaller output counts or zeroed top-level usage while
//!   `usage.iterations[]` holds the real values. We therefore take the field-wise maximum
//!   of the top-level usage and the sum of the response's own iterations, and the store merges
//!   duplicates with max. Advisor iterations are separate events priced as their own model.
//! * Resumed/forked sessions copy earlier messages into a new file → de-dup must be global.
//! * `model: "<synthetic>"` lines are client-side error placeholders with zero usage.
//! * `quotaLimits` appears on a line when a plan limit rejected the request.
//! * Transcripts live in `projects/<encoded launch dir>/`, where the launch directory has every
//!   non-alphanumeric character replaced by `-`. Subagent transcripts
//!   (`projects/<dir>/<session>/subagents/*.jsonl`) start in whatever directory the agent was in,
//!   so the project is anchored to the launch directory that folder name encodes.
//! * Lines inside a git repository carry `gitBranch`. Subagent lines are `isSidechain: true`
//!   with the agent's id (`agentId`) and type (`attributionAgent`, newer versions).
//! * Tool calls are `tool_use` content blocks (one block per line); the outcome is the matching
//!   `tool_result` block (`is_error`) in a later user line. Only names and outcomes are kept.

use super::{str_at, u64_at, i64_at, JsonlReader, ParseOutput};
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tokens, Tool, ToolCall, UsageEvent};
use serde_json::{json, Value};
use std::path::Path;

pub struct ClaudeCtx<'a> {
    /// Overrides the `entrypoint` client label (used for Cowork sessions).
    pub client_override: Option<&'a str>,
    pub source: &'a str,
}

/// `C:\Users\me\app` → `C--Users-me-app` (Claude Code's project folder naming).
fn encode_dir(p: &str) -> String {
    p.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn project_folder(path: &Path) -> Option<String> {
    let mut parts = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned());
    parts.by_ref().find(|c| c.eq_ignore_ascii_case("projects"))?;
    parts.next()
}

/// The directory a session was launched in: the deepest ancestor of `cwd` (itself included)
/// whose encoding is the transcript's project folder name. Falls back to `cwd`.
fn launch_dir(cwd: &str, folder: Option<&str>) -> String {
    if let Some(folder) = folder {
        for a in Path::new(cwd).ancestors() {
            let s = a.to_string_lossy();
            let s = s.trim_end_matches(['\\', '/']);
            if !s.is_empty() && encode_dir(s).eq_ignore_ascii_case(folder) {
                return s.to_owned();
            }
        }
    }
    cwd.to_owned()
}

pub fn parse_file(path: &Path, offset: u64, state: &Value, ctx: &ClaudeCtx) -> std::io::Result<ParseOutput> {
    let folder = project_folder(path);
    let mut lines = JsonlReader::open(path, offset)?;
    let mut out = ParseOutput::default();
    let mut max_version = str_at(state, "max_version").map(str::to_owned);
    // The session's first working directory is its project; later lines can carry
    // sub-directories the agent cd'd into, which would split one project into many.
    let mut root_cwd = str_at(state, "root_cwd").map(str::to_owned);
    let mut typed = 0u64;
    for line in lines.by_ref() {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warn(format!("invalid JSON at byte {}", line.offset));
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
        if root_cwd.is_none() {
            root_cwd = str_at(v, "cwd").filter(|s| !s.is_empty()).map(|c| launch_dir(c, folder.as_deref()));
        }
        let mut recognised = false;
        // each Cowork session runs in its own scratch folder; group them as one project
        let project = if ctx.client_override == Some("cowork") { Some("Cowork".to_owned()) } else { root_cwd.clone() };
        for mut ev in usage_events(v, ctx) {
            if project.is_some() {
                ev.project_path = project.clone();
            }
            out.events.push(ev);
            recognised = true;
        }
        for mut call in tool_calls(v) {
            if project.is_some() {
                call.project_path = project.clone();
            }
            out.tool_calls.push(call);
        }
        out.tool_results.extend(tool_results(v));
        if let Some(ls) = quota_limit(v, ctx) {
            out.limits.push(ls);
            recognised = true;
        }
        if recognised {
            out.lines_recognised += 1;
        }
        if out.round_full() {
            break;
        }
    }
    if out.lines_total >= 20 && typed == 0 {
        out.warn("unrecognised format: no line has a `type` field");
    }
    out.state = json!({ "max_version": max_version, "root_cwd": root_cwd });
    out.next_offset = lines.finish()?;
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

fn prompt_size(t: &Tokens) -> u64 {
    t.input + t.cache_read + t.cache_write
}

fn consulted_model(it: &Value) -> Option<&str> {
    (str_at(it, "type") == Some("advisor_message")).then(|| str_at(it, "model").unwrap_or("unknown"))
}

/// Every iteration re-sends the conversation, so the prompt size is the largest one, not the sum.
pub(crate) fn usage_events(v: &Value, ctx: &ClaudeCtx) -> Vec<UsageEvent> {
    if str_at(v, "type") != Some("assistant") {
        return Vec::new();
    }
    let Some(msg) = v.get("message") else { return Vec::new() };
    let Some(usage) = msg.get("usage").filter(|u| u.is_object()) else { return Vec::new() };
    let model = str_at(msg, "model").unwrap_or("unknown");
    if model == "<synthetic>" || v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    let key = if let Some(id) = str_at(msg, "id") {
        format!("cc:{id}")
    } else if let Some(rid) = str_at(v, "requestId") {
        format!("ccr:{rid}")
    } else if let Some(uuid) = str_at(v, "uuid") {
        format!("ccu:{uuid}")
    } else {
        return Vec::new();
    };
    let Some(ts_ms) = str_at(v, "timestamp").and_then(parse_ts_ms) else { return Vec::new() };

    let mut tokens = usage_tokens(usage);
    let mut prompt = prompt_size(&tokens);
    let mut consulted: Vec<(&str, Tokens, u64)> = Vec::new();
    if let Some(iters) = usage.get("iterations").and_then(Value::as_array) {
        let (mut own, mut own_prompt) = (Tokens::default(), 0);
        for it in iters.iter().filter(|it| it.is_object()) {
            let t = usage_tokens(it);
            match consulted_model(it) {
                None => {
                    own.add(&t);
                    own_prompt = own_prompt.max(prompt_size(&t));
                }
                Some(m) => match consulted.iter_mut().find(|c| c.0 == m) {
                    Some(c) => {
                        c.1.add(&t);
                        c.2 = c.2.max(prompt_size(&t));
                    }
                    None => consulted.push((m, t, prompt_size(&t))),
                },
            }
        }
        // iteration objects don't carry thinking details; keep the top-level figure
        own.reasoning = tokens.reasoning;
        tokens = tokens.max(&own);
        if own_prompt > 0 {
            prompt = own_prompt;
        }
    }

    let (agent, thread_id) = subagent(v);
    let event = |key: String, model: &str, mut tokens: Tokens, request_input: u64| {
        tokens.cache_write_1h = tokens.cache_write_1h.min(tokens.cache_write);
        tokens.reasoning = tokens.reasoning.min(tokens.output);
        UsageEvent {
            key,
            ts_ms,
            tool: Tool::ClaudeCode,
            client: ctx.client_override.map(str::to_owned).or_else(|| str_at(v, "entrypoint").map(str::to_owned)),
            model: model.to_owned(),
            project_path: str_at(v, "cwd").map(str::to_owned),
            session_id: str_at(v, "sessionId").map(str::to_owned),
            request_input,
            tokens,
            web_search_requests: 0,
            speed: None,
            service_tier: str_at(usage, "service_tier").map(str::to_owned),
            inference_geo: str_at(usage, "inference_geo").map(str::to_owned),
            request_id: str_at(v, "requestId").map(str::to_owned),
            accuracy: Accuracy::Exact,
            source: ctx.source.to_owned(),
            branch: branch(v),
            agent: agent.clone(),
            thread_id: thread_id.clone(),
        }
    };
    let searches = usage.get("server_tool_use").map(|s| u64_at(s, "web_search_requests")).unwrap_or(0);
    // a count past u32 means a damaged line; the whole record is dropped like an impossible token count
    let Ok(web_search_requests) = u32::try_from(searches) else {
        log::warn!("Claude record skipped: implausible web search count");
        return Vec::new();
    };
    let mut out = vec![UsageEvent {
        web_search_requests,
        speed: str_at(usage, "speed").map(str::to_owned),
        ..event(key.clone(), model, tokens, prompt)
    }];
    out.extend(consulted.into_iter().map(|(m, t, p)| event(format!("{key}:{m}"), m, t, p)));
    out
}

/// `gitBranch` as written on every line inside a repository (empty outside one).
fn branch(v: &Value) -> Option<String> {
    str_at(v, "gitBranch").map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned)
}

/// Subagent lines are marked `isSidechain` and carry the agent's id and, in newer versions,
/// its type (`attributionAgent`). Returns (agent type, agent id).
fn subagent(v: &Value) -> (Option<String>, Option<String>) {
    if v.get("isSidechain").and_then(Value::as_bool) != Some(true) {
        return (None, None);
    }
    let kind = str_at(v, "attributionAgent").map(str::trim).filter(|s| !s.is_empty()).unwrap_or("subagent");
    (Some(kind.to_owned()), str_at(v, "agentId").map(str::to_owned))
}

/// Tool calls in an assistant line: their names only, keyed by the tool-use id so copies of
/// a message (streaming duplicates, resumed sessions) count once.
fn tool_calls(v: &Value) -> Vec<ToolCall> {
    if str_at(v, "type") != Some("assistant") {
        return Vec::new();
    }
    let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) else { return Vec::new() };
    let Some(ts_ms) = str_at(v, "timestamp").and_then(parse_ts_ms) else { return Vec::new() };
    let (agent, _) = subagent(v);
    blocks
        .iter()
        .filter(|b| matches!(str_at(b, "type"), Some("tool_use" | "server_tool_use")))
        .filter_map(|b| {
            let (id, name) = (str_at(b, "id")?, str_at(b, "name")?.trim());
            (!name.is_empty()).then(|| ToolCall {
                key: format!("cct:{id}"),
                ts_ms,
                tool: Tool::ClaudeCode,
                session_id: str_at(v, "sessionId").map(str::to_owned),
                project_path: str_at(v, "cwd").map(str::to_owned),
                agent: agent.clone(),
                name: name.to_owned(),
                failed: None,
            })
        })
        .collect()
}

fn tool_results(v: &Value) -> Vec<(String, bool)> {
    if str_at(v, "type") != Some("user") {
        return Vec::new();
    }
    let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) else { return Vec::new() };
    blocks
        .iter()
        .filter(|b| str_at(b, "type") == Some("tool_result"))
        .filter_map(|b| {
            let id = str_at(b, "tool_use_id")?;
            Some((format!("cct:{id}"), b.get("is_error").and_then(Value::as_bool).unwrap_or(false)))
        })
        .collect()
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

    fn line(usage: Value) -> Value {
        json!({"type": "assistant", "sessionId": "s", "requestId": "req_1", "timestamp": "2026-09-01T08:00:00.000Z",
               "message": {"id": "msg_1", "model": "claude-sonnet-5", "usage": usage}})
    }

    fn events(usage: Value) -> Vec<UsageEvent> {
        usage_events(&line(usage), &ClaudeCtx { client_override: None, source: "test" })
    }

    #[test]
    fn a_web_search_count_past_u32_drops_the_record_instead_of_wrapping() {
        let ev = |n: u64| events(json!({"input_tokens": 1, "output_tokens": 1, "server_tool_use": {"web_search_requests": n}}));
        assert_eq!(ev(3)[0].web_search_requests, 3);
        assert_eq!(ev(u32::MAX as u64)[0].web_search_requests, u32::MAX);
        assert!(ev(u32::MAX as u64 + 2).is_empty());
    }

    #[test]
    fn advisor_iterations_are_a_separate_event_of_their_own_model() {
        // the documented shape: top-level totals hold the executor iterations only
        let ev = events(json!({
            "input_tokens": 1760, "cache_read_input_tokens": 412, "cache_creation_input_tokens": 0, "output_tokens": 531,
            "server_tool_use": {"web_search_requests": 1},
            "iterations": [
                {"type": "message", "input_tokens": 412, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "output_tokens": 89},
                {"type": "advisor_message", "model": "claude-opus-5", "input_tokens": 823, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "output_tokens": 1612},
                {"type": "advisor_message", "model": "claude-opus-5", "input_tokens": 900, "cache_read_input_tokens": 800, "cache_creation_input_tokens": 0, "output_tokens": 400},
                {"type": "message", "input_tokens": 1348, "cache_read_input_tokens": 412, "cache_creation_input_tokens": 0, "output_tokens": 442},
                "not an iteration"
            ]
        }));
        assert_eq!(ev.len(), 2);
        let (exec, adv) = (&ev[0], &ev[1]);
        assert_eq!((exec.key.as_str(), exec.model.as_str()), ("cc:msg_1", "claude-sonnet-5"));
        assert_eq!(exec.tokens, Tokens { input: 1760, cache_read: 412, output: 531, ..Default::default() });
        assert_eq!(exec.request_input, 1348 + 412, "the largest single prompt, not the sum");
        assert_eq!(exec.web_search_requests, 1);
        assert_eq!((adv.key.as_str(), adv.model.as_str()), ("cc:msg_1:claude-opus-5", "claude-opus-5"));
        assert_eq!(adv.tokens, Tokens { input: 1723, cache_read: 800, output: 2012, ..Default::default() });
        assert_eq!((adv.request_input, adv.web_search_requests), (1700, 0));
        assert_eq!((adv.session_id.as_deref(), adv.request_id.as_deref()), (Some("s"), Some("req_1")));
    }

    #[test]
    fn executor_only_iterations_keep_one_event_and_the_largest_prompt() {
        // two 120K prompts: neither request was over 200K
        let it = json!({"type": "message", "input_tokens": 1000, "cache_read_input_tokens": 119_000, "cache_creation_input_tokens": 0, "output_tokens": 10});
        let ev = events(json!({"input_tokens": 0, "output_tokens": 0, "iterations": [it.clone(), it]}));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].tokens, Tokens { input: 2000, cache_read: 238_000, output: 20, ..Default::default() });
        assert_eq!(ev[0].request_input, 120_000);
        // without iterations the top-level usage is the single prompt
        let ev = events(json!({"input_tokens": 5, "cache_read_input_tokens": 2000, "cache_creation_input_tokens": 1000, "output_tokens": 10}));
        assert_eq!((ev.len(), ev[0].request_input), (1, 3005));
    }

    #[test]
    fn an_advisor_iteration_without_a_model_is_left_unpriced() {
        let ev = events(json!({
            "input_tokens": 10, "output_tokens": 5,
            "iterations": [{"type": "message", "input_tokens": 10, "output_tokens": 5}, {"type": "advisor_message", "input_tokens": 300, "output_tokens": 50}]
        }));
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].tokens.input, 10);
        assert_eq!((ev[1].model.as_str(), ev[1].tokens.input), ("unknown", 300));
    }
}
