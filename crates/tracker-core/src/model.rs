//! Normalised usage records. Never holds prompt or response content, only metadata and counts.

use serde::{Deserialize, Serialize};

/// The product that produced the usage. Cowork sessions are Claude Code under the hood,
/// so they share `ClaudeCode` and are told apart by [`UsageEvent::client`]; the Antigravity
/// app, IDE and CLI likewise share `Antigravity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    ClaudeCode,
    Codex,
    ClaudeDesktop,
    Antigravity,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "claude_code",
            Tool::Codex => "codex",
            Tool::ClaudeDesktop => "claude_desktop",
            Tool::Antigravity => "antigravity",
        }
    }
    pub fn parse(s: &str) -> Option<Tool> {
        match s {
            "claude_code" => Some(Tool::ClaudeCode),
            "codex" => Some(Tool::Codex),
            "claude_desktop" => Some(Tool::ClaudeDesktop),
            "antigravity" => Some(Tool::Antigravity),
            _ => None,
        }
    }
    pub fn provider(self) -> Provider {
        match self {
            Tool::ClaudeCode | Tool::ClaudeDesktop => Provider::Anthropic,
            Tool::Codex => Provider::OpenAI,
            Tool::Antigravity => Provider::Google,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAI,
    Google,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
            Provider::Google => "google",
        }
    }
    pub fn parse(s: &str) -> Option<Provider> {
        match s {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAI),
            "google" => Some(Provider::Google),
            _ => None,
        }
    }
}

/// How trustworthy a number is. Shown to the user next to every figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accuracy {
    /// Written by the tool itself into its own log.
    Exact,
    /// Derived or computed by us (e.g. a user-defined threshold, a share of a window).
    Estimated,
    /// Recorded by this app's own opt-in live capture.
    Captured,
}

impl Accuracy {
    pub fn as_str(self) -> &'static str {
        match self {
            Accuracy::Exact => "exact",
            Accuracy::Estimated => "estimated",
            Accuracy::Captured => "captured",
        }
    }
    /// An unrecognised value is never trusted as exact.
    pub fn parse(s: &str) -> Accuracy {
        match s {
            "exact" => Accuracy::Exact,
            "captured" => Accuracy::Captured,
            _ => Accuracy::Estimated,
        }
    }
}

/// Token counts normalised so that the categories never overlap:
/// `input` is *uncached* prompt input only, `cache_read`/`cache_write` are separate,
/// and `output` includes reasoning/thinking (which is billed as output by both providers).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub cache_read: u64,
    /// All cache-write tokens, regardless of TTL.
    pub cache_write: u64,
    /// Subset of `cache_write` written with a 1-hour TTL (Anthropic only).
    pub cache_write_1h: u64,
    pub output: u64,
    /// Subset of `output` spent on reasoning/thinking, when the tool reports it.
    pub reasoning: u64,
}

impl Tokens {
    // saturating, so an absurd count from a damaged log cannot overflow
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.cache_read).saturating_add(self.cache_write).saturating_add(self.output)
    }
    pub fn add(&mut self, o: &Tokens) {
        self.input = self.input.saturating_add(o.input);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
        self.cache_write_1h = self.cache_write_1h.saturating_add(o.cache_write_1h);
        self.output = self.output.saturating_add(o.output);
        self.reasoning = self.reasoning.saturating_add(o.reasoning);
    }
    /// Field-wise maximum — used to merge repeated streaming snapshots of one response.
    pub fn max(&self, o: &Tokens) -> Tokens {
        Tokens {
            input: self.input.max(o.input),
            cache_read: self.cache_read.max(o.cache_read),
            cache_write: self.cache_write.max(o.cache_write),
            cache_write_1h: self.cache_write_1h.max(o.cache_write_1h),
            output: self.output.max(o.output),
            reasoning: self.reasoning.max(o.reasoning),
        }
    }
}

/// One billable model response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Globally unique de-duplication key (e.g. `cc:<message id>`, `cx:<session>:<offset>`).
    pub key: String,
    /// Unix epoch milliseconds, UTC.
    pub ts_ms: i64,
    pub tool: Tool,
    /// Finer-grained origin: Claude Code `entrypoint`, Codex `originator`, or `cowork`.
    pub client: Option<String>,
    pub model: String,
    /// Working directory of the session; kept locally only.
    pub project_path: Option<String>,
    pub session_id: Option<String>,
    pub tokens: Tokens,
    /// Size of the full prompt for this request (selects long-context price tiers).
    pub request_input: u64,
    pub web_search_requests: u32,
    pub speed: Option<String>,
    pub service_tier: Option<String>,
    pub inference_geo: Option<String>,
    /// Provider request id (Anthropic `req_…`) — links a captured event to its log copy.
    pub request_id: Option<String>,
    pub accuracy: Accuracy,
    /// Parser that produced the record (`claude_code_jsonl`, `codex_rollout`, …).
    pub source: String,
    /// Git branch the tool reported for this request (Claude Code: per request; Codex: at the
    /// start of the session). `None` outside a repository or when the tool does not say.
    pub branch: Option<String>,
    /// Subagent type when a subagent made the request (`general-purpose`, `guardian`, …);
    /// `None` for the main conversation.
    pub agent: Option<String>,
    /// The subagent's own conversation id. `session_id` is always the parent session, so a
    /// subagent's requests count toward the session that started it.
    pub thread_id: Option<String>,
}

/// One tool call or tool action, by name only (never its input or output).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Globally unique de-duplication key (`cct:<tool_use id>`, `cxt:<thread>:<offset>`).
    pub key: String,
    pub ts_ms: i64,
    pub tool: Tool,
    pub session_id: Option<String>,
    pub project_path: Option<String>,
    pub agent: Option<String>,
    /// Tool name as the tool reports it (`Bash`, `Edit`, `mcp__<server>__<tool>`, `shell`, …).
    pub name: String,
    /// Whether the call returned an error, when the log says (`None` = not known yet).
    pub failed: Option<bool>,
}

/// A point-in-time reading of a plan limit window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitSnapshot {
    pub ts_ms: i64,
    pub provider: Provider,
    pub tool: Tool,
    pub account: Option<String>,
    /// Provider-specific limit family (Codex `limit_id`), empty when not applicable.
    pub limit_id: String,
    /// `five_hour`, `seven_day`, or `<n>m` for unrecognised durations.
    pub window: String,
    /// 0–100, `None` when only a status is known.
    pub used_pct: Option<f64>,
    /// Unix epoch seconds.
    pub resets_at: Option<i64>,
    /// `allowed`, `allowed_warning`, `rejected`, … when reported.
    pub status: Option<String>,
    pub plan: Option<String>,
    pub source: String,
    pub accuracy: Accuracy,
}

/// The latest time accepted from any source: the year 3000, in epoch seconds.
pub const MAX_EPOCH_SECS: i64 = 32_503_680_000;

/// How far ahead of the clock a reading may be dated (clock drift between processes and machines).
pub const MAX_CLOCK_SKEW_MS: i64 = 86_400_000;

impl LimitSnapshot {
    /// A reading dated before 1970 or ahead of the clock, or with an impossible window, is damaged:
    /// stored, it would outrank every real reading as the newest one.
    pub fn is_damaged(&self, now_ms: i64) -> bool {
        let window = self.window.strip_suffix('m').and_then(|n| n.parse::<i64>().ok());
        let pct = self.used_pct.is_some_and(|p| !(0.0..=1000.0).contains(&p));
        !(0..=now_ms.saturating_add(MAX_CLOCK_SKEW_MS)).contains(&self.ts_ms) || window.is_some_and(|m| !(1..=MAX_WINDOW_MINUTES).contains(&m)) || pct
    }

    /// The reset time in milliseconds; `None` for a time outside 1970–3000 (a damaged record).
    pub fn resets_at_ms(&self) -> Option<i64> {
        self.resets_at.filter(|r| (0..=MAX_EPOCH_SECS).contains(r)).map(|r| r * 1000)
    }
}

/// The longest limit window accepted: a year. Anything else is a damaged reading.
pub const MAX_WINDOW_MINUTES: i64 = 366 * 1440;

/// `None` for a duration no limit window has (zero, negative or over a year).
pub fn window_name(minutes: i64) -> Option<String> {
    Some(match minutes {
        300 => "five_hour".into(),
        1440 => "one_day".into(),
        10080 => "seven_day".into(),
        43200 => "thirty_day".into(), // seen on the ChatGPT Go plan
        m if (1..=MAX_WINDOW_MINUTES).contains(&m) => format!("{m}m"),
        _ => return None,
    })
}

pub fn parse_ts_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_accuracy_is_not_exact() {
        for a in [Accuracy::Exact, Accuracy::Estimated, Accuracy::Captured] {
            assert_eq!(Accuracy::parse(a.as_str()), a);
        }
        assert_eq!(Accuracy::parse("EXACT"), Accuracy::Estimated);
        assert_eq!(Accuracy::parse(""), Accuracy::Estimated);
    }
}
