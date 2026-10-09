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
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub const SOURCE: &str = "claude_otel";
pub const DEFAULT_PORT: u16 = 43180;
/// Sent by Claude Code via `OTEL_EXPORTER_OTLP_HEADERS`.
pub const AUTH_HEADER: &str = "x-aiut-token";
const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_READERS: usize = 4;
const POLL: Duration = Duration::from_millis(50);

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

/// A plain JSON scalar (decimal string or number), not an `AnyValue`; 0 means unknown.
fn unix_nano_ms(v: Option<&Value>) -> Option<i64> {
    let ns = match v? {
        Value::String(s) => s.parse::<i64>().ok()?,
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u.min(i64::MAX as u64) as i64))?,
        _ => return None,
    };
    (ns > 0).then_some(ns / 1_000_000)
}

fn text<'a>(a: &'a HashMap<&str, Value>, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

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
                let ts_ms = text(&a, "event.timestamp")
                    .and_then(parse_ts_ms)
                    .or_else(|| unix_nano_ms(rec.get("timeUnixNano")))
                    .or_else(|| unix_nano_ms(rec.get("observedTimeUnixNano")));
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

#[derive(Debug, Default)]
pub struct ReceiverStats {
    pub events: AtomicU64,
    pub last_event_ms: AtomicI64,
    pub requests: AtomicU64,
    pub rejected: AtomicU64,
}

pub struct Receiver {
    server: Arc<tiny_http::Server>,
    stop: Arc<AtomicBool>,
    pub port: u16,
    pub stats: Arc<ReceiverStats>,
    pub token: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

type ReadBody = (tiny_http::Request, Result<Vec<u8>, u16>);

fn reply(code: u16) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string("{}")
        .with_status_code(code)
        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap())
}

fn header<'a>(req: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// Constant time, so the token cannot be guessed byte by byte from response times.
fn same_token(given: &str, expected: &str) -> bool {
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Checks everything that needs no body, so unauthenticated senders never get a reader.
fn precheck(req: &tiny_http::Request, token: &str) -> Result<(), u16> {
    if req.method() != &tiny_http::Method::Post {
        return Err(405);
    }
    if !header(req, AUTH_HEADER).is_some_and(|g| same_token(g, token)) {
        return Err(401);
    }
    if req.url().split('?').next() != Some("/v1/logs") {
        // metrics/traces are not configured by us; accept and drop
        return Err(200);
    }
    if !header(req, "Content-Type").is_some_and(|v| v.starts_with("application/json")) {
        return Err(415);
    }
    if req.body_length().is_some_and(|n| n > MAX_BODY) {
        return Err(413);
    }
    Ok(())
}

impl Receiver {
    /// Binds 127.0.0.1:`port` and serves until dropped. `on_events` runs after each batch
    /// that stored at least one event (e.g. to refresh the UI).
    pub fn start(port: u16, db_path: PathBuf, token: String, on_events: impl Fn(usize) + Send + 'static) -> std::io::Result<Receiver> {
        if token.trim().is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the receiver needs an access key"));
        }
        let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| std::io::Error::other(e.to_string()))?;
        let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(port);
        let server = Arc::new(server);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(ReceiverStats::default());
        let (srv, st, stp, tok) = (server.clone(), stats.clone(), stop.clone(), token.clone());
        let thread = std::thread::Builder::new().name("otlp-receiver".into()).spawn(move || {
            let mut store = match Store::open(&db_path) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("otlp receiver cannot open database: {e}");
                    return;
                }
            };
            // tiny_http has no read deadline, so bodies are read off the serving thread
            let (done_tx, done_rx) = mpsc::channel::<ReadBody>();
            let readers = Arc::new(AtomicUsize::new(0));
            while !stp.load(Ordering::SeqCst) {
                while let Ok((req, body)) = done_rx.try_recv() {
                    let code = match body {
                        Ok(body) => {
                            store_batch(&mut store, &body, &st, &on_events);
                            200
                        }
                        Err(code) => code,
                    };
                    let _ = req.respond(reply(code));
                }
                let req = match srv.recv_timeout(POLL) {
                    Ok(Some(r)) => r,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                st.requests.fetch_add(1, Ordering::Relaxed);
                if let Err(code) = precheck(&req, &tok) {
                    if code == 401 && st.rejected.fetch_add(1, Ordering::Relaxed) == 0 {
                        log::warn!("otlp receiver: a request without the expected token was refused");
                    }
                    let _ = req.respond(reply(code));
                    continue;
                }
                if readers.load(Ordering::SeqCst) >= MAX_READERS {
                    let _ = req.respond(reply(503));
                    continue;
                }
                readers.fetch_add(1, Ordering::SeqCst);
                let (tx, busy) = (done_tx.clone(), readers.clone());
                let spawned = std::thread::Builder::new().name("otlp-body".into()).spawn(move || {
                    let mut req = req;
                    let mut body = Vec::new();
                    let read = req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body);
                    busy.fetch_sub(1, Ordering::SeqCst);
                    let body = match read {
                        Err(_) => Err(400),
                        Ok(_) if body.len() > MAX_BODY => Err(413),
                        Ok(_) => Ok(body),
                    };
                    // after shutdown nobody listens; dropping the request answers it with 500
                    let _ = tx.send((req, body));
                });
                if spawned.is_err() {
                    readers.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })?;
        Ok(Receiver { server, stop, port, stats, token, thread: Some(thread) })
    }
}

fn store_batch(store: &mut Store, body: &[u8], st: &ReceiverStats, on_events: &impl Fn(usize)) {
    let events = serde_json::from_slice::<Value>(body).map(|v| parse_logs(&v)).unwrap_or_default();
    if events.is_empty() {
        return;
    }
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
    fn the_record_time_is_the_fallback_timestamp() {
        let mut body = sample("req_t", "claude-haiku-4-5", 1);
        let attrs = body["resourceLogs"][0]["scopeLogs"][0]["logRecords"][1]["attributes"].as_array_mut().unwrap();
        attrs.retain(|a| a["key"] != "event.timestamp");
        assert_eq!(parse_logs(&body)[0].ts_ms, 1_790_961_950_000);
        let rec = &mut body["resourceLogs"][0]["scopeLogs"][0]["logRecords"][1];
        rec["timeUnixNano"] = json!("0");
        rec["observedTimeUnixNano"] = json!(1_790_961_951_000_000_000u64);
        assert_eq!(parse_logs(&body)[0].ts_ms, 1_790_961_951_000);
    }

    fn post(port: u16, headers: &str, body: &str) -> String {
        use std::io::Write;
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        write!(s, "POST /v1/logs HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn count(db: &std::path::Path) -> i64 {
        Store::open(db).unwrap().conn().query_row("SELECT COUNT(*) FROM usage_event", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn only_requests_with_the_token_are_stored() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        assert!(Receiver::start(0, db.clone(), " ".into(), |_| {}).is_err(), "an empty key would let any sender in");
        let r = Receiver::start(0, db.clone(), "secret-1".into(), |_| {}).unwrap();
        let body = sample("req_a", "claude-haiku-4-5", 3).to_string();
        assert!(post(r.port, "", &body).starts_with("HTTP/1.1 401"));
        assert!(post(r.port, "x-aiut-token: secret-2\r\n", &body).starts_with("HTTP/1.1 401"));
        assert_eq!(count(&db), 0);
        assert!(post(r.port, "X-AIUT-Token: secret-1\r\n", &body).starts_with("HTTP/1.1 200"));
        assert_eq!(count(&db), 1);
        assert_eq!(r.stats.rejected.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn an_unfinished_body_does_not_block_other_requests_or_shutdown() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let r = Receiver::start(0, db.clone(), "k".into(), |_| {}).unwrap();
        let mut stalled = std::net::TcpStream::connect(("127.0.0.1", r.port)).unwrap();
        write!(stalled, "POST /v1/logs HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nx-aiut-token: k\r\nContent-Length: 1000\r\n\r\n{{\"res").unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let body = sample("req_b", "claude-haiku-4-5", 3).to_string();
        assert!(post(r.port, "x-aiut-token: k\r\n", &body).starts_with("HTTP/1.1 200"));
        let t = std::time::Instant::now();
        drop(r);
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(count(&db), 1);
        drop(stalled);
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
