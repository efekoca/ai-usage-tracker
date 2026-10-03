//! Loopback OTLP/HTTP-JSON receiver for Claude Code log events (opt-in).
//!
//! Only `api_request` events are used, and from them only counts and ids: model, token
//! counts, request/session ids, timestamp. Identity attributes that Claude Code also sends
//! (e-mail, account and organisation ids) are never read into a record. The receiver binds to
//! 127.0.0.1 only.
//!
//! A captured request that also appears in a transcript (same `request_id`) is hidden at query
//! time, so this source only *adds* calls the transcripts do not contain (helper models, etc.).

use crate::model::{parse_ts_ms, Accuracy, Tokens, Tool, UsageEvent};
use crate::store::Store;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

pub const SOURCE: &str = "claude_otel";
pub const DEFAULT_PORT: u16 = 43180;
const MAX_BODY: usize = 16 * 1024 * 1024;

/// OTLP JSON `AnyValue` → plain JSON (ints may arrive as strings per the OTLP JSON mapping).
fn any_value(v: &Value) -> Value {
    if let Some(s) = v.get("stringValue") {
        return s.clone();
    }
    if let Some(i) = v.get("intValue") {
        return match i {
            Value::String(s) => s.parse::<i64>().map(Value::from).unwrap_or(Value::Null),
            other => other.clone(),
        };
    }
    if let Some(d) = v.get("doubleValue") {
        return d.clone();
    }
    if let Some(b) = v.get("boolValue") {
        return b.clone();
    }
    Value::Null
}

fn attrs(record: &Value) -> HashMap<&str, Value> {
    record
        .get("attributes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|kv| Some((kv.get("key")?.as_str()?, any_value(kv.get("value")?))))
                .collect()
        })
        .unwrap_or_default()
}

fn num(a: &HashMap<&str, Value>, k: &str) -> u64 {
    match a.get(k) {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)).unwrap_or(0),
        Some(Value::String(s)) => s.parse::<f64>().map(|f| f.max(0.0) as u64).unwrap_or(0),
        _ => 0,
    }
}

fn text<'a>(a: &'a HashMap<&str, Value>, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Turns an OTLP logs export request into usage events (only `api_request`).
pub fn parse_logs(body: &Value) -> Vec<UsageEvent> {
    let mut out = Vec::new();
    for rl in body.get("resourceLogs").and_then(Value::as_array).into_iter().flatten() {
        for sl in rl.get("scopeLogs").and_then(Value::as_array).into_iter().flatten() {
            for rec in sl.get("logRecords").and_then(Value::as_array).into_iter().flatten() {
                let a = attrs(rec);
                let body_name = rec.get("body").map(any_value);
                let is_request = text(&a, "event.name") == Some("api_request")
                    || body_name.as_ref().and_then(Value::as_str) == Some("claude_code.api_request");
                if !is_request {
                    continue;
                }
                let request_id = text(&a, "request_id").map(str::to_owned);
                let session = text(&a, "session.id").map(str::to_owned);
                // without the provider's request id the transcript copy of the same request could
                // not be recognised, so the event would be counted twice: skip it
                let Some(rid) = &request_id else { continue };
                let key = format!("otel:{rid}");
                let ts_ms = text(&a, "event.timestamp").and_then(parse_ts_ms).or_else(|| {
                    rec.get("timeUnixNano").map(any_value).and_then(|v| match v {
                        Value::String(s) => s.parse::<i64>().ok(),
                        Value::Number(n) => n.as_i64(),
                        _ => None,
                    }).map(|ns| ns / 1_000_000)
                });
                let Some(ts_ms) = ts_ms else { continue };
                let tokens = Tokens {
                    input: num(&a, "input_tokens"),
                    cache_read: num(&a, "cache_read_tokens"),
                    cache_write: num(&a, "cache_creation_tokens"),
                    cache_write_1h: 0,
                    output: num(&a, "output_tokens"),
                    reasoning: 0,
                };
                out.push(UsageEvent {
                    key,
                    ts_ms,
                    tool: Tool::ClaudeCode,
                    client: text(&a, "query_source").map(|q| format!("otel:{q}")).or(Some("otel".into())),
                    model: text(&a, "model").unwrap_or("unknown").to_owned(),
                    project_path: None,
                    session_id: session,
                    request_input: tokens.input + tokens.cache_read + tokens.cache_write,
                    tokens,
                    web_search_requests: 0,
                    speed: text(&a, "speed").filter(|s| *s == "fast").map(str::to_owned),
                    service_tier: None,
                    inference_geo: None,
                    request_id,
                    accuracy: Accuracy::Captured,
                    source: SOURCE.into(),
                    branch: None,
                    agent: None,
                    thread_id: None,
                });
            }
        }
    }
    out
}

/// Live counters for the settings screen.
#[derive(Debug, Default)]
pub struct ReceiverStats {
    pub events: AtomicU64,
    pub last_event_ms: AtomicI64,
    pub requests: AtomicU64,
}

pub struct Receiver {
    server: Arc<tiny_http::Server>,
    stop: Arc<AtomicBool>,
    pub port: u16,
    pub stats: Arc<ReceiverStats>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Receiver {
    /// Binds 127.0.0.1:`port` and serves until dropped. `on_events` runs after each batch
    /// that stored at least one event (e.g. to refresh the UI).
    pub fn start(port: u16, db_path: PathBuf, on_events: impl Fn(usize) + Send + 'static) -> std::io::Result<Receiver> {
        let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| std::io::Error::other(e.to_string()))?;
        let server = Arc::new(server);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(ReceiverStats::default());
        let (srv, st, stp) = (server.clone(), stats.clone(), stop.clone());
        let thread = std::thread::Builder::new().name("otlp-receiver".into()).spawn(move || {
            let mut store = match Store::open(&db_path) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("otlp receiver cannot open database: {e}");
                    return;
                }
            };
            for mut req in srv.incoming_requests() {
                if stp.load(Ordering::SeqCst) {
                    break;
                }
                st.requests.fetch_add(1, Ordering::Relaxed);
                let path = req.url().split('?').next().unwrap_or("").to_owned();
                let is_json = req
                    .headers()
                    .iter()
                    .any(|h| h.field.equiv("Content-Type") && h.value.as_str().starts_with("application/json"));
                let reply = |code: u16| tiny_http::Response::from_string("{}").with_status_code(code).with_header(
                    "Content-Type: application/json".parse::<tiny_http::Header>().unwrap(),
                );
                if req.method() != &tiny_http::Method::Post {
                    let _ = req.respond(reply(405));
                    continue;
                }
                if path != "/v1/logs" {
                    // metrics/traces are not configured by us; accept and drop
                    let _ = req.respond(reply(200));
                    continue;
                }
                if !is_json {
                    let _ = req.respond(reply(415));
                    continue;
                }
                let mut body = Vec::new();
                let read = req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body);
                if read.is_err() || body.len() > MAX_BODY {
                    let _ = req.respond(reply(413));
                    continue;
                }
                let events = serde_json::from_slice::<Value>(&body).map(|v| parse_logs(&v)).unwrap_or_default();
                if !events.is_empty() {
                    let stored = store.transaction().and_then(|mut tx| {
                        tx.upsert_events(&events)?;
                        tx.commit()
                    });
                    match stored {
                        Ok(()) => {
                            st.events.fetch_add(events.len() as u64, Ordering::Relaxed);
                            st.last_event_ms.store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
                            on_events(events.len());
                        }
                        Err(e) => log::warn!("otlp receiver could not store events: {e}"),
                    }
                }
                let _ = req.respond(reply(200));
            }
        })?;
        Ok(Receiver { server, stop, port, stats, thread: Some(thread) })
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn sample(request_id: &str, model: &str, input: i64) -> Value {
        json!({"resourceLogs": [{"resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "claude-code"}}]},
          "scopeLogs": [{"scope": {"name": "com.anthropic.claude_code.events"}, "logRecords": [
            {"timeUnixNano": "1790961946622000000", "body": {"stringValue": "claude_code.plugin_loaded"},
             "attributes": [{"key": "event.name", "value": {"stringValue": "plugin_loaded"}}]},
            {"timeUnixNano": "1790961950000000000", "body": {"stringValue": "claude_code.api_request"},
             "attributes": [
               {"key": "user.email", "value": {"stringValue": "someone@example.com"}},
               {"key": "session.id", "value": {"stringValue": "sess-1"}},
               {"key": "event.name", "value": {"stringValue": "api_request"}},
               {"key": "event.timestamp", "value": {"stringValue": "2026-10-02T17:25:50.000Z"}},
               {"key": "event.sequence", "value": {"intValue": 7}},
               {"key": "model", "value": {"stringValue": model}},
               {"key": "input_tokens", "value": {"intValue": input}},
               {"key": "output_tokens", "value": {"intValue": "40"}},
               {"key": "cache_read_tokens", "value": {"intValue": 1000}},
               {"key": "cache_creation_tokens", "value": {"intValue": 200}},
               {"key": "cost_usd", "value": {"doubleValue": 0.01}},
               {"key": "request_id", "value": {"stringValue": request_id}},
               {"key": "query_source", "value": {"stringValue": "repl_main_thread"}},
               {"key": "speed", "value": {"stringValue": "normal"}}
             ]}]}]}]})
    }

    #[test]
    fn maps_api_requests_and_ignores_other_events_and_identity() {
        let ev = parse_logs(&sample("req_1", "claude-haiku-4-5", 12));
        assert_eq!(ev.len(), 1);
        let e = &ev[0];
        assert_eq!(e.key, "otel:req_1");
        assert_eq!(e.request_id.as_deref(), Some("req_1"));
        assert_eq!(e.session_id.as_deref(), Some("sess-1"));
        assert_eq!(e.tokens, Tokens { input: 12, cache_read: 1000, cache_write: 200, cache_write_1h: 0, output: 40, reasoning: 0 });
        assert_eq!(e.accuracy, Accuracy::Captured);
        assert_eq!(e.speed, None);
        assert_eq!(e.client.as_deref(), Some("otel:repl_main_thread"));
        let dump = serde_json::to_string(e).unwrap();
        assert!(!dump.contains("example.com"));
        assert_eq!(e.ts_ms, crate::model::parse_ts_ms("2026-10-02T17:25:50.000Z").unwrap());
    }

    #[test]
    fn garbage_yields_nothing() {
        assert!(parse_logs(&json!({"resourceLogs": "nope"})).is_empty());
        assert!(parse_logs(&json!(null)).is_empty());
    }

    #[test]
    fn requests_without_a_request_id_are_skipped() {
        // the transcript copy could not be matched, so keeping it would count the request twice
        let mut body = sample("req_x", "claude-haiku-4-5", 12);
        let attrs = body["resourceLogs"][0]["scopeLogs"][0]["logRecords"][1]["attributes"].as_array_mut().unwrap();
        attrs.retain(|a| a["key"] != "request_id");
        assert!(parse_logs(&body).is_empty());
    }
}
