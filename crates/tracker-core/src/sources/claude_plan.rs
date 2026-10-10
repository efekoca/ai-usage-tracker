//! Claude desktop `plan-usage-history.json` — the app's own record of plan utilisation.
//!
//! Observed shape (version 2): `{version, samples:[{t: epoch ms, org: id, u:{fh: %, sd: %}}]}`
//! where `fh` = five-hour window and `sd` = seven-day window, both 0–100. Unknown keys in `u`
//! are kept under their raw name so new windows appear without a code change.

use super::{i64_at, str_at, ParseOutput};
use crate::model::{Accuracy, LimitSnapshot, Provider, Tool, MAX_CLOCK_SKEW_MS};
use serde_json::Value;
use std::path::Path;

pub const SOURCE: &str = "claude_plan_history";
const KNOWN_VERSIONS: &[i64] = &[1, 2];

pub fn parse_file(path: &Path) -> std::io::Result<ParseOutput> {
    let bytes = std::fs::read(path)?;
    let mut out = ParseOutput { next_offset: bytes.len() as u64, ..Default::default() };
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let doc: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => {
            out.warn("invalid JSON document");
            return Ok(out);
        }
    };
    match i64_at(&doc, "version") {
        Some(v) if KNOWN_VERSIONS.contains(&v) => {}
        other => out.warn(format!("unknown plan-usage-history version {other:?}; parsing best-effort")),
    }
    let Some(samples) = doc.get("samples").and_then(Value::as_array) else {
        out.warn("unrecognised format: no `samples` array");
        return Ok(out);
    };
    let latest = chrono::Utc::now().timestamp_millis() + MAX_CLOCK_SKEW_MS;
    for s in samples {
        out.lines_total += 1;
        let (Some(t), Some(u)) = (i64_at(s, "t"), s.get("u").and_then(Value::as_object)) else { continue };
        if !(0..=latest).contains(&t) {
            out.warn(format!("sample at {t} skipped: time before 1970 or ahead of the clock"));
            continue;
        }
        let account = str_at(s, "org").map(str::to_owned);
        for (k, val) in u {
            let Some(pct) = val.as_f64() else { continue };
            let window = match k.as_str() {
                "fh" => "five_hour",
                "sd" => "seven_day",
                other => other,
            };
            out.limits.push(LimitSnapshot {
                ts_ms: t,
                provider: Provider::Anthropic,
                tool: Tool::ClaudeDesktop,
                account: account.clone(),
                limit_id: String::new(),
                window: window.to_owned(),
                used_pct: Some(pct),
                resets_at: None,
                status: None,
                plan: None,
                source: SOURCE.into(),
                accuracy: Accuracy::Exact,
            });
        }
        out.lines_recognised += 1;
    }
    Ok(out)
}
