//! Normalised records shared by every source parser, the store and the query layer.
//! Nothing here ever holds prompt or response content — only metadata and counts.

use serde::{Deserialize, Serialize};

/// The product that produced the usage. Cowork sessions are Claude Code under the hood,
/// so they share `ClaudeCode` and are told apart by [`UsageEvent::client`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    ClaudeCode,
    Codex,
    ClaudeDesktop,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "claude_code",
            Tool::Codex => "codex",
            Tool::ClaudeDesktop => "claude_desktop",
        }
    }
    pub fn parse(s: &str) -> Option<Tool> {
        match s {
            "claude_code" => Some(Tool::ClaudeCode),
            "codex" => Some(Tool::Codex),
            "claude_desktop" => Some(Tool::ClaudeDesktop),
            _ => None,
        }
    }
    pub fn provider(self) -> Provider {
        match self {
            Tool::ClaudeCode | Tool::ClaudeDesktop => Provider::Anthropic,
            Tool::Codex => Provider::OpenAI,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAI,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAI => "openai",
        }
    }
    pub fn parse(s: &str) -> Option<Provider> {
        match s {
            "anthropic" => Some(Provider::Anthropic),
            "openai" => Some(Provider::OpenAI),
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
    pub fn parse(s: &str) -> Accuracy {
        match s {
            "estimated" => Accuracy::Estimated,
            "captured" => Accuracy::Captured,
            _ => Accuracy::Exact,
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
    /// Output tokens, reasoning included.
    pub output: u64,
    /// Subset of `output` spent on reasoning/thinking, when the tool reports it.
    pub reasoning: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.output
    }
    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.cache_write_1h += o.cache_write_1h;
        self.output += o.output;
        self.reasoning += o.reasoning;
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
    pub accuracy: Accuracy,
    /// Parser that produced the record (`claude_code_jsonl`, `codex_rollout`, …).
    pub source: String,
}

/// A point-in-time reading of a plan limit window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitSnapshot {
    pub ts_ms: i64,
    pub provider: Provider,
    pub tool: Tool,
    /// Account / organisation identifier when the source exposes one.
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

/// Maps a window length in minutes to the canonical window name.
pub fn window_name(minutes: i64) -> String {
    match minutes {
        300 => "five_hour".into(),
        1440 => "one_day".into(),
        10080 => "seven_day".into(),
        43200 => "thirty_day".into(), // seen on the ChatGPT Go plan
        m => format!("{m}m"),
    }
}

/// Parses an RFC 3339 timestamp into epoch milliseconds.
pub fn parse_ts_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}
