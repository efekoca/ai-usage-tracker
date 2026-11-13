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
//! * Every `type: "user"` line carries `promptId`: the prompt, the tool results that follow it and
//!   the subagents it starts share one, in the subagents' own files too. Responses carry none and
//!   belong to the user line before them. `origin.kind` or `turnOrigin` says what began the turn
//!   (`human`, `task-notification`, `peer`, …); older versions wrote neither.

use super::{str_at, u64_at, i64_at, JsonlReader, ParseOutput};
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tokens, Tool, ToolCall, Turn, UsageEvent};
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

/// The folder after the last `projects`: the config root itself may sit under a `Projects` folder.
fn project_folder(path: &Path) -> Option<String> {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let at = parts.iter().rposition(|c| c.eq_ignore_ascii_case("projects"))?;
    parts.get(at + 1).cloned()
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
    // The session's first working directory is its project; later lines can carry
    // sub-directories the agent cd'd into, which would split one project into many.
    let mut root_cwd = str_at(state, "root_cwd").map(str::to_owned);
    let mut turn = str_at(state, "turn").map(str::to_owned);
    let mut typed = 0u64;
    let mut undated = 0u64;
    for line in lines.by_ref() {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warn(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        if v.get("type").is_some() {
            typed += 1;
        }
        if carries_records(v) && str_at(v, "timestamp").and_then(parse_ts_ms).is_none() {
            undated += 1;
        }
        if root_cwd.is_none() {
            root_cwd = str_at(v, "cwd").filter(|s| !s.is_empty()).map(|c| launch_dir(c, folder.as_deref()));
        }
        if let Some((id, start)) = turn_of(v) {
            out.turns.extend(start);
            turn = Some(id);
        }
        let mut recognised = false;
        // each Cowork session runs in its own scratch folder; group them as one project
        let project = if ctx.client_override == Some("cowork") { Some("Cowork".to_owned()) } else { root_cwd.clone() };
        for mut ev in usage_events(v, ctx) {
            if project.is_some() {
                ev.project_path = project.clone();
            }
            ev.turn = turn.clone();
            out.events.push(ev);
            recognised = true;
        }
        for mut call in tool_calls(v) {
            if project.is_some() {
                call.project_path = project.clone();
            }
            call.turn = turn.clone();
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
    out.warn_undated(undated);
    out.state = json!({ "root_cwd": root_cwd, "turn": turn });
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
    t.input.saturating_add(t.cache_read).saturating_add(t.cache_write)
}

fn consulted_model(it: &Value) -> Option<&str> {
    (str_at(it, "type") == Some("advisor_message")).then(|| str_at(it, "model").unwrap_or("unknown"))
}

/// SQL that is true for an advisor's event, keyed `<request key>:<model>` below; a request's own
/// key (`cc:<message id>`, `ccr:<request id>`, `ccu:<uuid>`) holds no `:` after its prefix.
pub(crate) const ADVISOR_SQL: &str = "(key GLOB 'cc:*:*' OR key GLOB 'ccr:*:*' OR key GLOB 'ccu:*:*')";

/// A line whose records need its time: a response's usage and tool calls, or a limit hit.
fn carries_records(v: &Value) -> bool {
    let usage = str_at(v, "type") == Some("assistant") && v.get("message").and_then(|m| m.get("usage")).is_some_and(Value::is_object);
    usage || v.get("quotaLimits").is_some_and(Value::is_object)
}

/// A Claude model under its Bedrock or Vertex name becomes Anthropic's id, so it is priced and
/// grouped as one model; every other id is kept as written.
fn model_id(model: &str) -> String {
    super::opencode::cloud_claude_id(model).unwrap_or_else(|| model.to_owned())
}

/// Every iteration re-sends the conversation, so the prompt size is the largest one, not the sum.
pub(crate) fn usage_events(v: &Value, ctx: &ClaudeCtx) -> Vec<UsageEvent> {
    if str_at(v, "type") != Some("assistant") {
        return Vec::new();
    }
    let Some(msg) = v.get("message") else { return Vec::new() };
    let Some(usage) = msg.get("usage").filter(|u| u.is_object()) else { return Vec::new() };
    let raw_model = str_at(msg, "model").unwrap_or("unknown");
    if raw_model == "<synthetic>" || v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    let model = model_id(raw_model);
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
            limit_reset: None,
            creator: None,
            turn: None,
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
        ..event(key.clone(), &model, tokens, prompt)
    }];
    // the key keeps the name as logged, so a record read before stays one record
    out.extend(consulted.into_iter().map(|(m, t, p)| event(format!("{key}:{m}"), &model_id(m), t, p)));
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
                turn: None,
            })
        })
        .collect()
}

fn has_tool_result(v: &Value) -> bool {
    let blocks = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array);
    blocks.is_some_and(|b| b.iter().any(|b| str_at(b, "type") == Some("tool_result")))
}

/// The turn a user line belongs to. A line of the main conversation that is not a tool result
/// also gives the turn's record: the earliest one stored is when and how the turn began.
fn turn_of(v: &Value) -> Option<(String, Option<Turn>)> {
    if str_at(v, "type") != Some("user") {
        return None;
    }
    let id = format!("cc:{}", str_at(v, "promptId").filter(|p| !p.is_empty())?);
    let flag = |k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
    if flag("isSidechain") || has_tool_result(v) {
        return Some((id, None));
    }
    let named = str_at(v, "turnOrigin").or_else(|| v.get("origin").and_then(|o| str_at(o, "kind")));
    let origin = match named.map(|o| o.replace('-', "_")).as_deref() {
        Some(o @ ("human" | "task_notification" | "peer" | "scheduled" | "sdk")) => o.to_owned(),
        // versions that named no origin wrote the user's own prompt as a plain line
        None if !flag("isMeta") && !flag("isCompactSummary") => "human".to_owned(),
        _ => "other".to_owned(),
    };
    let start = str_at(v, "timestamp").and_then(parse_ts_ms).map(|ts_ms| Turn {
        id: id.clone(),
        tool: Tool::ClaudeCode,
        session_id: str_at(v, "sessionId").map(str::to_owned),
        ts_ms,
        origin,
    });
    Some((id, start))
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
    let q = v.get("quotaLimits").filter(|q| q.is_object())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_folder_follows_the_last_projects_folder() {
        let p = Path::new("/x/Projects/cfg/projects/-p-app/sess/subagents/agent-a.jsonl");
        assert_eq!(project_folder(p).as_deref(), Some("-p-app"));
        assert_eq!(project_folder(Path::new("/home/u/.claude/projects/-p-app/s.jsonl")).as_deref(), Some("-p-app"));
    }

    #[test]
    fn a_huge_count_does_not_overflow_the_prompt_size() {
        let ev = events(json!({"input_tokens": u64::MAX, "cache_read_input_tokens": 1, "output_tokens": 1}));
        assert_eq!(ev[0].request_input, u64::MAX);
    }

    #[test]
    fn a_quota_field_that_is_not_an_object_is_no_reading() {
        let ctx = ClaudeCtx { client_override: None, source: "test" };
        for q in [Value::Null, json!("rejected"), json!([])] {
            assert!(quota_limit(&json!({"timestamp": "2026-09-01T08:00:00.000Z", "quotaLimits": q}), &ctx).is_none());
        }
        let ok = json!({"timestamp": "2026-09-01T08:00:00.000Z", "quotaLimits": {"status": "rejected", "rateLimitType": "five_hour"}});
        assert_eq!(quota_limit(&ok, &ctx).unwrap().used_pct, Some(100.0));
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
    fn the_archive_tells_an_advisor_event_apart_by_its_key() {
        let usage = json!({"input_tokens": 10, "output_tokens": 5, "iterations": [
            {"type": "message", "input_tokens": 10, "output_tokens": 5},
            {"type": "advisor_message", "model": "us.anthropic.claude-opus-5-v1:0", "input_tokens": 30, "output_tokens": 2}
        ]});
        // keyed by message id, by request id, and by the line's own id
        let lines = [
            json!({"type": "assistant", "sessionId": "s", "requestId": "req_1", "timestamp": "2026-09-01T08:00:00.000Z", "message": {"id": "msg_1", "model": "claude-sonnet-5", "usage": usage}}),
            json!({"type": "assistant", "sessionId": "s", "requestId": "req_2", "timestamp": "2026-09-01T08:01:00.000Z", "message": {"model": "claude-sonnet-5", "usage": usage}}),
            json!({"type": "assistant", "sessionId": "s", "uuid": "u-3", "timestamp": "2026-09-01T08:02:00.000Z", "message": {"model": "claude-sonnet-5", "usage": usage}}),
        ];
        let ctx = ClaudeCtx { client_override: None, source: "test" };
        let events: Vec<UsageEvent> = lines.iter().flat_map(|l| usage_events(l, &ctx)).collect();
        let keys: Vec<&str> = events.iter().map(|e| e.key.as_str()).collect();
        let advisor = "us.anthropic.claude-opus-5-v1:0";
        assert_eq!(keys, ["cc:msg_1".to_owned(), format!("cc:msg_1:{advisor}"), "ccr:req_2".into(), format!("ccr:req_2:{advisor}"), "ccu:u-3".into(), format!("ccu:u-3:{advisor}")]);
        let mut store = crate::store::Store::open_in_memory().unwrap();
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&events).unwrap();
        tx.commit().unwrap();
        let rows: Vec<(String, bool)> = store.events_between(0, i64::MAX).unwrap().into_iter().map(|e| (e.model, e.advisor)).collect();
        let (sonnet, opus) = (("claude-sonnet-5".to_owned(), false), ("claude-opus-5".to_owned(), true));
        assert_eq!(rows, [sonnet.clone(), opus.clone(), sonnet.clone(), opus.clone(), sonnet, opus], "the advisor's own model, under Anthropic's name");
    }

    #[test]
    fn only_bedrock_and_vertex_names_of_a_claude_model_are_spelt_anew() {
        // Anthropic's ids, placeholders and other vendors' models behind an Anthropic-style endpoint
        for id in [
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-3-5-haiku-20241022",
            "claude-3-7-sonnet-latest",
            "claude-opus-4-6",
            "claude-opus-4-6[1m]",
            "claude-sonnet-5-5",
            "claude-fable-5-1",
            "unknown",
            "glm-4.6",
            "glm-5.1",
            "kimi-k2.5",
            "kimi-k2-0905-preview",
            "deepseek-chat",
            "deepseek-v3.2",
            "qwen3-coder-plus",
            "MiniMax-M2.1",
            "gpt-5.4",
            "gemini-3.7-flash",
            "anthropic/claude-sonnet-4.5",
            "z-ai/glm-4.6",
            "mistral.devstral-2-v1:0",
            "gpt-5@2026",
        ] {
            assert_eq!(model_id(id), id, "{id}");
        }
        for (cloud, id) in [
            ("anthropic.claude-sonnet-4-5-20250929-v1:0", "claude-sonnet-4-5-20250929"),
            ("us.anthropic.claude-opus-4-1-20250805-v1:0", "claude-opus-4-1-20250805"),
            ("global.anthropic.claude-haiku-4-5-20251001-v1:0", "claude-haiku-4-5-20251001"),
            ("eu.anthropic.claude-opus-5", "claude-opus-5"),
            ("arn:aws:bedrock:us-east-1:123456789012:inference-profile/us.anthropic.claude-sonnet-4-5-20250929-v1:0", "claude-sonnet-4-5-20250929"),
            ("claude-sonnet-4-5@20250929", "claude-sonnet-4-5-20250929"),
            ("claude-opus-5@default", "claude-opus-5"),
        ] {
            assert_eq!(model_id(cloud), id, "{cloud}");
        }
        let ev = usage_events(
            &json!({"type": "assistant", "sessionId": "s", "timestamp": "2026-09-01T08:00:00.000Z", "message": {"id": "msg_1", "model": "claude-sonnet-4-5@20250929", "usage": {"input_tokens": 1, "output_tokens": 1}}}),
            &ClaudeCtx { client_override: None, source: "test" },
        );
        assert_eq!((ev[0].key.as_str(), ev[0].model.as_str()), ("cc:msg_1", "claude-sonnet-4-5-20250929"));
    }

    #[test]
    fn lines_whose_time_cannot_be_read_are_counted_in_one_warning() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        let lines = [
            json!({"type": "assistant", "sessionId": "s", "timestamp": "2026-09-01T08:00:00.000Z", "message": {"id": "msg_1", "model": "claude-sonnet-5", "usage": {"input_tokens": 1, "output_tokens": 1}}}),
            json!({"type": "assistant", "sessionId": "s", "timestamp": "01/09/2026 08:01", "message": {"id": "msg_2", "model": "claude-sonnet-5", "usage": {"input_tokens": 1, "output_tokens": 1}}}),
            json!({"type": "assistant", "sessionId": "s", "message": {"id": "msg_3", "model": "claude-sonnet-5", "usage": {"input_tokens": 1, "output_tokens": 1}}}),
            json!({"type": "system", "quotaLimits": {"status": "rejected", "rateLimitType": "five_hour"}}),
            // nothing to count on these
            json!({"type": "user", "sessionId": "s", "message": {"content": "hi"}}),
            json!({"type": "summary", "summary": "x"}),
        ];
        std::fs::write(&p, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        let out = parse_file(&p, 0, &Value::Null, &ClaudeCtx { client_override: None, source: "test" }).unwrap();
        assert_eq!((out.events.len(), out.limits.len()), (1, 0));
        assert_eq!(out.warning_count, 3);
        assert_eq!(out.warnings, ["3 lines skipped: their time could not be read"]);
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
