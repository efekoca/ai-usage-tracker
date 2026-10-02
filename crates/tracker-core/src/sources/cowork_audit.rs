//! Cowork `audit.jsonl` — read **only** for `rate_limit_event` lines. Assistant lines in this
//! file duplicate the session transcript and are deliberately ignored to avoid double counting.
//!
//! Observed shape: `{type:"rate_limit_event", timestamp, rate_limit_info:{status, resetsAt,
//! rateLimitType, unifiedWindows:{five_hour:{utilization 0..1, resetsAt}, seven_day:{…}}}}`.

use super::{f64_at, i64_at, read_jsonl_from, str_at, ParseOutput};
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tool};
use serde_json::Value;
use std::path::Path;

pub const SOURCE: &str = "cowork_audit";

pub fn parse_file(path: &Path, offset: u64) -> std::io::Result<ParseOutput> {
    let (lines, next) = read_jsonl_from(path, offset)?;
    let mut out = ParseOutput { next_offset: next, ..Default::default() };
    for line in &lines {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warnings.push(format!("invalid JSON at byte {}", line.offset));
            continue;
        };
        if str_at(v, "type") != Some("rate_limit_event") {
            continue;
        }
        let Some(info) = v.get("rate_limit_info") else { continue };
        let Some(ts_ms) = str_at(v, "timestamp").or_else(|| str_at(v, "_audit_timestamp")).and_then(parse_ts_ms) else {
            continue;
        };
        let status = str_at(info, "status").map(str::to_owned);
        let mut any = false;
        if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
            for (name, w) in windows {
                let Some(util) = f64_at(w, "utilization") else { continue };
                // utilization is a 0..1 fraction; tolerate a future switch to percentages
                let pct = if util <= 1.5 { util * 100.0 } else { util };
                out.limits.push(snapshot(ts_ms, name, Some(pct), i64_at(w, "resetsAt"), None));
                any = true;
            }
        }
        if !any {
            if let Some(kind) = str_at(info, "rateLimitType") {
                let pct = (status.as_deref() == Some("rejected")).then_some(100.0);
                out.limits.push(snapshot(ts_ms, kind, pct, i64_at(info, "resetsAt"), status.clone()));
                any = true;
            }
        } else if let (Some(kind), Some(st)) = (str_at(info, "rateLimitType"), &status) {
            // attach the status to the window it refers to
            if let Some(s) = out.limits.iter_mut().rev().find(|s| s.ts_ms == ts_ms && s.window == kind) {
                s.status = Some(st.clone());
            }
        }
        if any {
            out.lines_recognised += 1;
        }
    }
    Ok(out)
}

fn snapshot(ts_ms: i64, window: &str, used_pct: Option<f64>, resets_at: Option<i64>, status: Option<String>) -> LimitSnapshot {
    LimitSnapshot {
        ts_ms,
        provider: Provider::Anthropic,
        tool: Tool::ClaudeCode,
        account: None,
        limit_id: String::new(),
        window: window.to_owned(),
        used_pct,
        resets_at,
        status,
        plan: None,
        source: SOURCE.into(),
        accuracy: Accuracy::Exact,
    }
}
