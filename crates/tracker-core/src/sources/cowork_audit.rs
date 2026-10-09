//! Cowork `audit.jsonl` — read **only** for `rate_limit_event` lines. Assistant lines in this
//! file duplicate the session transcript and are deliberately ignored to avoid double counting.
//!
//! Observed shape: `{type:"rate_limit_event", timestamp, rate_limit_info:{status, resetsAt,
//! rateLimitType, unifiedWindows:{five_hour:{utilization 0..1, resetsAt}, seven_day:{…}}}}`.

use super::{f64_at, i64_at, str_at, JsonlReader, ParseOutput};
use crate::model::{parse_ts_ms, Accuracy, LimitSnapshot, Provider, Tool};
use serde_json::Value;
use std::path::Path;

pub const SOURCE: &str = "cowork_audit";

pub fn parse_file(path: &Path, offset: u64) -> std::io::Result<ParseOutput> {
    let mut lines = JsonlReader::open(path, offset)?;
    let mut out = ParseOutput::default();
    for line in lines.by_ref() {
        out.lines_total += 1;
        let Some(v) = &line.value else {
            out.warn(format!("invalid JSON at byte {}", line.offset));
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
                // a warning carries its own utilization; a rejection means the window is full
                let pct = f64_at(info, "utilization")
                    .map(|u| if u <= 1.5 { u * 100.0 } else { u })
                    .or_else(|| (status.as_deref() == Some("rejected")).then_some(100.0));
                out.limits.push(snapshot(ts_ms, kind, pct, i64_at(info, "resetsAt"), status.clone()));
                any = true;
            }
        } else if let (Some(kind), Some(st)) = (str_at(info, "rateLimitType"), &status)
            && let Some(s) = out.limits.iter_mut().rev().find(|s| s.ts_ms == ts_ms && s.window == kind)
        {
            s.status = Some(st.clone());
        }
        if any {
            out.lines_recognised += 1;
        }
        if out.round_full() {
            break;
        }
    }
    out.next_offset = lines.finish()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_keep_their_utilization_and_rejections_mean_full() {
        let p = std::env::temp_dir().join(format!("aut-cowork-audit-{}.jsonl", std::process::id()));
        std::fs::write(
            &p,
            concat!(
                r#"{"type":"rate_limit_event","timestamp":"2026-08-24T10:00:00.000Z","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day","utilization":0.99,"resetsAt":1787900000}}"#, "\n",
                r#"{"type":"rate_limit_event","timestamp":"2026-08-24T11:00:00.000Z","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour","resetsAt":1787910000}}"#, "\n",
                r#"{"type":"assistant","message":{"id":"m","usage":{"input_tokens":5}}}"#, "\n",
            ),
        )
        .unwrap();
        let out = parse_file(&p, 0).unwrap();
        std::fs::remove_file(&p).ok();
        let got: Vec<(&str, Option<f64>, Option<&str>)> = out.limits.iter().map(|l| (l.window.as_str(), l.used_pct, l.status.as_deref())).collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "seven_day");
        assert!((got[0].1.unwrap() - 99.0).abs() < 1e-9);
        assert_eq!(got[0].2, Some("allowed_warning"));
        assert_eq!(got[1], ("five_hour", Some(100.0), Some("rejected")));
        assert!(out.events.is_empty());
    }
}
