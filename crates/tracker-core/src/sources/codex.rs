//! Codex rollout parser (`<CODEX_HOME>/{sessions,archived_sessions}/YYYY/MM/DD/rollout-*.jsonl`).
//! Used by both the Codex CLI and the Codex desktop app.
//!
//! Format notes:
//! * `session_meta.payload`: `id`, `cwd`, `originator`, `git.branch`, `source`, `parent_thread_id`,
//!   `forked_from_id`, `session_id` (the root thread). Subagent rollouts have
//!   `source: {"subagent": …}`; their requests count toward the root session.
//! * Only the first `session_meta` belongs to the file. A fork or a subagent started with forked
//!   context first copies the source thread's history (its `session_meta`, `token_count`,
//!   `item_completed`, …) in one burst with new timestamps. `thread_settings_applied` names the
//!   thread that owns what follows (Codex ≥ 0.152); older logs end a copy at the first pause.
//! * `token_count`: `info.last_token_usage` (this request) and `info.total_token_usage`
//!   (cumulative). `input_tokens` includes cached input, `output_tokens` includes reasoning.
//! * De-dup keys come from content, so copied history, rewritten and moved files all map to the
//!   same rows: `cx:<fork root>:t<cumulative totals>` for usage, `cxt:i<call id>` for actions.
//!   Versions before 0.2.5 keyed rows by byte offset; a file read from its start reports those
//!   keys as stale so they are replaced in the same transaction.
//! * `rate_limits`: `primary`/`secondary` `{used_percent, window_minutes, resets_at}`,
//!   `plan_type`, `limit_id`. Windows are classified by `window_minutes`, not by slot.

use super::{f64_at, i64_at, str_at, u64_at, JsonlReader, ParseOutput};
use crate::model::{parse_ts_ms, window_name, Accuracy, LimitSnapshot, Provider, Tokens, Tool, ToolCall, UsageEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const SOURCE: &str = "codex_rollout";

/// Copied history is written in one burst; a line this long after the previous one comes from
/// the file's own thread.
const COPY_BURST_MS: i64 = 1_000;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    own: Option<String>,
    owner: Option<String>,
    threads: BTreeMap<String, Thread>,
    model: Option<String>,
    last_total: Option<String>,
    last_line_ms: Option<i64>,
    /// The session id the offset keys of versions before 0.2.5 used.
    legacy: Option<String>,
    /// The owner was named by the log itself (`thread_settings_applied`, `item_completed`).
    marked: bool,
    /// The last thread whose `session_meta` had no `forked_from_id`: older Codex starts a copy
    /// with the source's own `session_meta`, which names the source.
    unsourced: Option<String>,
    /// A read from the start stopped after a round; the next round still replays.
    replaying: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Thread {
    cwd: Option<String>,
    originator: Option<String>,
    branch: Option<String>,
    agent: Option<String>,
    parent: Option<String>,
    root: Option<String>,
    forked_from: Option<String>,
}

struct Who {
    thread: String,
    session: String,
    agent: Option<String>,
    cwd: Option<String>,
    branch: Option<String>,
    originator: Option<String>,
}

impl State {
    fn own_id(&self, path: &Path) -> String {
        self.own.clone().or_else(|| session_id_from_filename(path)).unwrap_or_default()
    }

    fn owner_id(&self, path: &Path) -> String {
        self.owner.clone().unwrap_or_else(|| self.own_id(path))
    }

    fn in_copy(&self, path: &Path) -> bool {
        self.owner.as_ref().is_some_and(|o| *o != self.own_id(path))
    }

    /// Only the file's own thread or one whose `session_meta` was copied here can own lines;
    /// `/review` forwards a child's items with the child's id into the parent's log.
    fn take_owner(&mut self, id: &str, path: &Path) {
        if id == self.own_id(path) || self.threads.contains_key(id) {
            self.owner = Some(id.to_owned());
            self.marked = true;
        }
    }

    /// A marked owner keeps its own id, so sibling forks stay apart; otherwise the fork root,
    /// which every copy of the line agrees on even when the copy hides where it ended.
    fn key_thread(&self, id: &str) -> String {
        if self.marked { id.to_owned() } else { self.fork_root(id) }
    }

    /// The first thread of the fork chain: a request keeps this key in every copy of its history.
    fn fork_root(&self, id: &str) -> String {
        let mut cur = id.to_owned();
        for _ in 0..64 {
            match self.threads.get(&cur).and_then(|t| t.forked_from.clone()) {
                Some(next) if next != cur => cur = next,
                _ => break,
            }
        }
        cur
    }

    fn who(&self, path: &Path) -> Who {
        let own = self.own_id(path);
        let id = self.owner_id(path);
        let base = self.threads.get(&own).cloned().unwrap_or_default();
        let t = self.threads.get(&id).cloned().unwrap_or_default();
        let session = match &t.agent {
            Some(_) => t.root.clone().filter(|r| *r != id).or(t.parent.clone()).unwrap_or_else(|| id.clone()),
            None => id.clone(),
        };
        Who {
            session,
            agent: t.agent,
            cwd: t.cwd.or(base.cwd),
            branch: t.branch.or(base.branch),
            originator: t.originator.or(base.originator),
            thread: id,
        }
    }
}

pub fn parse_file(path: &Path, offset: u64, state: &Value) -> std::io::Result<ParseOutput> {
    let mut st: State = serde_json::from_value(state.clone()).unwrap_or_default();
    if st.legacy.is_none() {
        st.legacy = session_id_from_filename(path).or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()));
    }
    let replay = offset == 0 || st.replaying;
    let mut lines = JsonlReader::open(path, offset)?;
    let mut out = ParseOutput::default();
    let mut typed = 0u64;
    let mut stopped = false;

    for line in lines.by_ref() {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warn(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        let Some(kind) = str_at(v, "type") else { continue };
        typed += 1;
        let p = v.get("payload").unwrap_or(&Value::Null);
        let ts_ms = str_at(v, "timestamp").and_then(parse_ts_ms);
        if let (Some(t), Some(l)) = (ts_ms, st.last_line_ms)
            && t - l >= COPY_BURST_MS
            && st.in_copy(path)
        {
            st.owner = None;
            st.marked = false;
        }
        match (kind, str_at(p, "type")) {
            ("session_meta", _) => {
                if let Some(id) = str_at(p, "id") {
                    let t = thread_of(p);
                    if let Some(src) = st.unsourced.take().filter(|s| s != id)
                        && let Some(prev) = st.threads.get_mut(&src)
                    {
                        prev.forked_from = Some(id.to_owned());
                    }
                    if t.forked_from.is_none() && t.agent.is_none() {
                        st.unsourced = Some(id.to_owned());
                    }
                    st.threads.entry(id.to_owned()).or_insert(t);
                    if st.own.is_none() {
                        st.own = Some(id.to_owned());
                    }
                    st.owner = Some(id.to_owned());
                    st.marked = false;
                    st.legacy = Some(id.to_owned());
                }
                out.lines_recognised += 1;
            }
            ("turn_context", _) => {
                if let Some(m) = str_at(p, "model") {
                    st.model = Some(m.to_owned());
                }
                if let Some(c) = str_at(p, "cwd") {
                    let id = st.owner_id(path);
                    st.threads.entry(id).or_default().cwd = Some(c.to_owned());
                }
                out.lines_recognised += 1;
            }
            ("event_msg", Some("thread_settings_applied")) => {
                if let Some(id) = str_at(p, "thread_id") {
                    st.take_owner(id, path);
                }
                if st.model.is_none()
                    && let Some(m) = p.get("thread_settings").and_then(|s| str_at(s, "model"))
                {
                    st.model = Some(m.to_owned());
                }
            }
            ("event_msg", Some("token_count")) => {
                if replay {
                    out.stale_events.push(format!("cx:{}:{}", st.legacy.as_deref().unwrap_or_default(), line.offset));
                }
                let mut recognised = false;
                if let (Some(info), Some(ts_ms)) = (p.get("info").filter(|i| !i.is_null()), ts_ms) {
                    let total = info.get("total_token_usage").filter(|t| t.is_object()).map(totals_key);
                    if total.is_some() && total == st.last_total {
                        recognised = true;
                    } else if let Some(last) = info.get("last_token_usage") {
                        st.last_total = total.clone();
                        if u64_at(last, "input_tokens") + u64_at(last, "output_tokens") > 0 {
                            let who = st.who(path);
                            let key = match total {
                                Some(t) => format!("cx:{}:t{t}", st.key_thread(&who.thread)),
                                None => format!("cx:{}:o{}", who.thread, line.offset),
                            };
                            if st.in_copy(path) {
                                out.copies.push((out.events.len(), st.legacy.clone().unwrap_or_default()));
                            }
                            out.events.push(event(&who, &st, key, ts_ms, last));
                        }
                        recognised = true;
                    }
                }
                if let (Some(rl), Some(ts_ms)) = (p.get("rate_limits").filter(|r| !r.is_null()), ts_ms) {
                    if !st.in_copy(path) {
                        rate_limits(rl, ts_ms, &mut out.limits);
                    } else if replay {
                        out.stale_limits.push((SOURCE.to_owned(), ts_ms));
                    }
                    recognised = true;
                }
                if recognised {
                    out.lines_recognised += 1;
                }
            }
            ("event_msg", Some("item_completed")) => {
                if replay {
                    out.stale_tool_calls.push(format!("cxt:{}:{}", st.legacy.as_deref().unwrap_or_default(), line.offset));
                }
                if let Some(id) = str_at(p, "thread_id") {
                    st.take_owner(id, path);
                }
                let completed = i64_at(p, "completed_at_ms").filter(|&ms| ms > 0).or(ts_ms);
                if let (Some(item), Some(ts_ms)) = (p.get("item"), completed)
                    && let Some(call) = action(&st.who(path), line.offset, ts_ms, item)
                {
                    out.tool_calls.push(call);
                    out.lines_recognised += 1;
                }
            }
            _ => {}
        }
        if ts_ms.is_some() {
            st.last_line_ms = ts_ms;
        }
        if out.round_full() {
            stopped = true;
            break;
        }
    }
    st.replaying = replay && stopped;
    if out.lines_total >= 20 && typed == 0 {
        out.warn("unrecognised format: no line has a `type` field");
    }
    out.state = serde_json::to_value(&st).unwrap_or(Value::Null);
    out.next_offset = lines.finish()?;
    Ok(out)
}

fn thread_of(p: &Value) -> Thread {
    let id = |k: &str| str_at(p, k).filter(|s| !s.is_empty()).map(str::to_owned);
    Thread {
        cwd: id("cwd"),
        originator: id("originator"),
        branch: p.get("git").and_then(|g| str_at(g, "branch")).map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned),
        agent: p.get("source").and_then(subagent_label),
        parent: id("parent_thread_id"),
        root: id("session_id"),
        forked_from: id("forked_from_id"),
    }
}

/// The cumulative totals: identical in a thread's own log and in any copy of it.
fn totals_key(t: &Value) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        u64_at(t, "input_tokens"),
        u64_at(t, "cached_input_tokens"),
        u64_at(t, "cache_write_input_tokens"),
        u64_at(t, "output_tokens"),
        u64_at(t, "reasoning_output_tokens")
    )
}

fn event(who: &Who, st: &State, key: String, ts_ms: i64, last: &Value) -> UsageEvent {
    let input_all = u64_at(last, "input_tokens");
    let cached = u64_at(last, "cached_input_tokens");
    let cache_write = u64_at(last, "cache_write_input_tokens");
    let output = u64_at(last, "output_tokens");
    let reasoning = u64_at(last, "reasoning_output_tokens").min(output);
    UsageEvent {
        key,
        ts_ms,
        tool: Tool::Codex,
        client: who.originator.clone(),
        model: st.model.clone().unwrap_or_else(|| "unknown".into()),
        project_path: who.cwd.clone(),
        session_id: Some(who.session.clone()),
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
        request_id: None,
        accuracy: Accuracy::Exact,
        source: SOURCE.into(),
        branch: who.branch.clone(),
        agent: who.agent.clone(),
        thread_id: who.agent.is_some().then(|| who.thread.clone()),
    }
}

/// `{"subagent": "review"}`, `{"subagent": {"other": "guardian"}}`,
/// `{"subagent": {"thread_spawn": {…, "agent_role": "explorer"}}}` → the subagent's type.
/// A plain source (`"cli"`, `"vscode"`, …) is the main agent.
fn subagent_label(source: &Value) -> Option<String> {
    let sub = source.get("subagent")?;
    let label = match sub {
        Value::String(s) => s.clone(),
        Value::Object(o) => match (o.get("other"), o.get("thread_spawn")) {
            (Some(Value::String(s)), _) => s.clone(),
            (_, Some(spawn)) => str_at(spawn, "agent_role").unwrap_or("thread_spawn").to_owned(),
            _ => o.keys().next().cloned().unwrap_or_default(),
        },
        _ => String::new(),
    };
    let label = label.trim();
    Some(if label.is_empty() { "subagent".to_owned() } else { label.to_owned() })
}

/// One completed action (command, file change, MCP tool call, web search, …) by name and
/// outcome. Messages, reasoning and compaction items are not actions and are skipped.
fn action(who: &Who, offset: u64, ts_ms: i64, item: &Value) -> Option<ToolCall> {
    let status = str_at(item, "status");
    let (name, failed) = match str_at(item, "type")? {
        // a non-zero exit code counts as an error, as Claude Code reports it for its shell
        "CommandExecution" => ("shell".to_owned(), Some(status == Some("failed") || i64_at(item, "exit_code").is_some_and(|c| c != 0))),
        "FileChange" => ("apply_patch".to_owned(), Some(matches!(status, Some("failed" | "declined")))),
        "McpToolCall" => {
            let server = str_at(item, "server").unwrap_or("unknown");
            let tool = str_at(item, "tool").unwrap_or("unknown");
            let is_error = item.get("result").and_then(|r| r.get("isError")).and_then(Value::as_bool) == Some(true);
            (format!("mcp__{server}__{tool}"), Some(status == Some("failed") || is_error))
        }
        "WebSearch" => ("web_search".to_owned(), None),
        "Extension" => (str_at(item, "kind").map(|k| k.replace('.', "_")).unwrap_or_else(|| "extension".into()), None),
        "ImageView" => ("view_image".to_owned(), None),
        _ => return None,
    };
    let key = match str_at(item, "id") {
        Some(id) => format!("cxt:i{id}"),
        None => format!("cxt:{}:o{offset}", who.thread),
    };
    Some(ToolCall {
        key,
        ts_ms,
        tool: Tool::Codex,
        session_id: Some(who.session.clone()),
        project_path: who.cwd.clone(),
        agent: who.agent.clone(),
        name,
        failed,
    })
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
