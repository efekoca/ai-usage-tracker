//! Loopback OTLP/HTTP-JSON receiver for Claude Code log events (opt-in).
//!
//! Only `api_request` events are used, and from them only counts and ids: model, token
//! counts, request/session ids, timestamp, and the organisation id as a one-way digest, which
//! tells Claude accounts apart (see [`crate::accounts`]). The e-mail and the other identity
//! attributes Claude Code also sends are never read. The receiver binds to 127.0.0.1 only.
//!
//! A captured request that also appears in a transcript (same `request_id`) is hidden at query
//! time, so this source only *adds* calls the transcripts do not contain (helper models, etc.).

use crate::model::{claude_account, parse_ts_ms, Accuracy, Tokens, Tool, UsageEvent};
use crate::store::Store;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

pub const SOURCE: &str = "claude_otel";
pub const DEFAULT_PORT: u16 = 43180;
/// Sent by Claude Code via `OTEL_EXPORTER_OTLP_HEADERS`.
pub const AUTH_HEADER: &str = "x-aiut-token";
const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_HEAD: usize = 16 * 1024;
/// Connections served at once; more are told to come back later.
const MAX_CONNECTIONS: usize = 8;
/// How long one request may take to arrive whole: a sender that stalls is cut off.
const REQUEST_TIME: Duration = Duration::from_secs(30);
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
                    request_input: tokens.input.saturating_add(tokens.cache_read).saturating_add(tokens.cache_write),
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
                    limit_reset: None,
                    creator: text(&a, "organization.id").filter(|o| crate::model::is_uuid(o.trim())).map(claude_account),
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
    stop: Arc<AtomicBool>,
    pub port: u16,
    pub stats: Arc<ReceiverStats>,
    pub token: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// A body read whole, and where to send back the status it got.
struct Batch {
    body: Vec<u8>,
    done: mpsc::Sender<u16>,
}

/// Constant time, so the token cannot be guessed byte by byte from response times.
fn same_token(given: &str, expected: &str) -> bool {
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// One connection's bytes, read against a deadline. A minimal HTTP/1.1 reader: the receiver
/// takes one request per connection and closes it, so nothing is ever drained or kept alive.
struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
    deadline: Instant,
}

impl Conn {
    /// Reads more bytes; 408 once the deadline passes, 400 when the sender closed.
    fn fill(&mut self) -> Result<(), u16> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(408);
        }
        let _ = self.stream.set_read_timeout(Some(left));
        let mut chunk = [0u8; 8192];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err(400),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(())
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(408),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Ok(()),
            Err(_) => Err(400),
        }
    }

    /// The bytes up to `sep`, which is taken off too; at most `max` of them.
    fn until(&mut self, sep: &[u8], max: usize, too_long: u16) -> Result<Vec<u8>, u16> {
        loop {
            if let Some(i) = self.buf.windows(sep.len()).position(|w| w == sep) {
                let line = self.buf[..i].to_vec();
                self.buf.drain(..i + sep.len());
                return Ok(line);
            }
            if self.buf.len() > max {
                return Err(too_long);
            }
            self.fill()?;
        }
    }

    fn take(&mut self, n: usize) -> Result<Vec<u8>, u16> {
        while self.buf.len() < n {
            self.fill()?;
        }
        Ok(self.buf.drain(..n).collect())
    }
}

struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn get(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn read_head(c: &mut Conn) -> Result<Head, u16> {
    let raw = c.until(b"\r\n\r\n", MAX_HEAD, 431)?;
    let text = std::str::from_utf8(&raw).map_err(|_| 400u16)?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(path), Some(version)) = (first.next(), first.next(), first.next()) else { return Err(400) };
    if !version.starts_with("HTTP/1.") {
        return Err(400);
    }
    let headers = lines.map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).ok_or(400u16)).collect::<Result<_, _>>()?;
    Ok(Head { method: method.to_owned(), path: path.to_owned(), headers })
}

/// Checks everything that needs no body, so unauthenticated senders never get one read.
/// `Err(200)` takes a request that is not ours (metrics, traces) and drops it.
fn precheck(h: &Head, token: &str) -> Result<Option<usize>, u16> {
    if h.method != "POST" {
        return Err(405);
    }
    if !h.get(AUTH_HEADER).is_some_and(|g| same_token(g, token)) {
        return Err(401);
    }
    if h.path.split('?').next() != Some("/v1/logs") {
        return Err(200);
    }
    if !h.get("Content-Type").is_some_and(|v| v.starts_with("application/json")) {
        return Err(415);
    }
    let chunked = h.get("Transfer-Encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
    match (h.get("Content-Length"), chunked) {
        // both at once could be read two ways
        (Some(_), true) => Err(400),
        (None, true) => Ok(None),
        (None, false) => Err(411),
        (Some(n), false) => match n.parse::<u64>() {
            Ok(n) if n > MAX_BODY as u64 => Err(413),
            Ok(n) => Ok(Some(n as usize)),
            Err(_) => Err(400),
        },
    }
}

fn read_chunked(c: &mut Conn) -> Result<Vec<u8>, u16> {
    let mut body = Vec::new();
    loop {
        let line = c.until(b"\r\n", 1024, 400)?;
        let size = std::str::from_utf8(&line).ok().and_then(|l| usize::from_str_radix(l.split(';').next().unwrap_or_default().trim(), 16).ok()).ok_or(400u16)?;
        if size == 0 {
            // trailers, then the end
            while !c.until(b"\r\n", 1024, 400)?.is_empty() {}
            return Ok(body);
        }
        if body.len() + size > MAX_BODY {
            return Err(413);
        }
        body.extend(c.take(size)?);
        if !c.take(2)?.eq(b"\r\n") {
            return Err(400);
        }
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

fn answer(stream: &mut TcpStream, code: u16) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = write!(stream, "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}", reason(code));
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// The status for one request; a body read whole is stored by the receiver's own thread.
fn serve(c: &mut Conn, token: &str, st: &ReceiverStats, batches: &mpsc::Sender<Batch>) -> u16 {
    let head = match read_head(c) {
        Ok(h) => h,
        Err(code) => return code,
    };
    st.requests.fetch_add(1, Ordering::Relaxed);
    let length = match precheck(&head, token) {
        Ok(length) => length,
        Err(code) => {
            if code == 401 && st.rejected.fetch_add(1, Ordering::Relaxed) == 0 {
                log::warn!("otlp receiver: a request without the expected token was refused");
            }
            return code;
        }
    };
    if head.get("Expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let _ = c.stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let body = match length {
        Some(n) => c.take(n),
        None => read_chunked(c),
    };
    let body = match body {
        Ok(b) => b,
        Err(code) => return code,
    };
    let (done, result) = mpsc::channel();
    if batches.send(Batch { body, done }).is_err() {
        return 503;
    }
    result.recv().unwrap_or(503)
}

impl Receiver {
    /// Binds 127.0.0.1:`port` and serves until dropped. `on_events` runs after each batch
    /// that stored at least one event (e.g. to refresh the UI).
    pub fn start(port: u16, db_path: PathBuf, token: String, on_events: impl Fn(usize) + Send + 'static) -> std::io::Result<Receiver> {
        if token.trim().is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the receiver needs an access key"));
        }
        // opened first: a receiver that cannot store must not listen and leave requests waiting
        let mut store = Store::open(&db_path).map_err(|e| std::io::Error::other(e.to_string()))?;
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        // polled, so a stop is seen within POLL
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(ReceiverStats::default());
        let (st, stp, tok) = (stats.clone(), stop.clone(), token.clone());
        let thread = std::thread::Builder::new().name("otlp-receiver".into()).spawn(move || {
            let (batch_tx, batches) = mpsc::channel::<Batch>();
            let busy = Arc::new(AtomicUsize::new(0));
            while !stp.load(Ordering::SeqCst) {
                loop {
                    let mut stream = match listener.accept() {
                        Ok((s, _)) => s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => {
                            log::warn!("otlp receiver: {e}");
                            break;
                        }
                    };
                    // some systems hand the listener's non-blocking mode on
                    let _ = stream.set_nonblocking(false);
                    if busy.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                        answer(&mut stream, 503);
                        continue;
                    }
                    busy.fetch_add(1, Ordering::SeqCst);
                    let (st, tok, tx, held) = (st.clone(), tok.clone(), batch_tx.clone(), busy.clone());
                    let spawned = std::thread::Builder::new().name("otlp-request".into()).spawn(move || {
                        let mut c = Conn { stream, buf: Vec::new(), deadline: Instant::now() + REQUEST_TIME };
                        let code = serve(&mut c, &tok, &st, &tx);
                        answer(&mut c.stream, code);
                        held.fetch_sub(1, Ordering::SeqCst);
                    });
                    if spawned.is_err() {
                        busy.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                if let Ok(b) = batches.recv_timeout(POLL) {
                    let _ = b.done.send(store_batch(&mut store, &b.body, &st, &on_events));
                }
            }
        })?;
        Ok(Receiver { stop, port, stats, token, thread: Some(thread) })
    }
}

/// The HTTP status: 503 asks the exporter to send the batch again (the archive was busy), 400
/// tells it the body was not OTLP JSON.
fn store_batch(store: &mut Store, body: &[u8], st: &ReceiverStats, on_events: &impl Fn(usize)) -> u16 {
    let Ok(doc) = serde_json::from_slice::<Value>(body) else { return 400 };
    let events = parse_logs(&doc);
    if events.is_empty() {
        return 200;
    }
    let stored = store.transaction().and_then(|mut tx| {
        tx.upsert_events(&events)?;
        tx.commit()
    });
    match stored {
        Ok(()) => {
            // an event's organization names its request's account; nothing else would settle it
            // if no log or limit read follows
            if events.iter().any(|e| e.creator.is_some())
                && let Err(e) = crate::accounts::resolve_claude(store)
            {
                log::warn!("claude accounts not resolved after telemetry: {e}");
            }
            st.events.fetch_add(events.len() as u64, Ordering::Relaxed);
            st.last_event_ms.store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
            on_events(events.len());
            200
        }
        Err(e) => {
            log::warn!("otlp receiver could not store events: {e}");
            503
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
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
               {"key": "user.account_uuid", "value": {"stringValue": "0b5e2c1d-3f4a-4b6c-8d9e-0f1a2b3c4d5e"}},
               {"key": "organization.id", "value": {"stringValue": "7C8D9E0F-1A2B-4C3D-9E4F-5A6B7C8D9E0F"}},
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
        assert_eq!(e.creator, Some(crate::model::claude_account("7c8d9e0f-1a2b-4c3d-9e4f-5a6b7c8d9e0f")), "the organisation, as a digest");
        let dump = serde_json::to_string(e).unwrap();
        assert!(!dump.contains("example.com") && !dump.contains("0b5e2c1d") && !dump.to_lowercase().contains("7c8d9e0f-"));
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
    fn a_batch_that_was_not_stored_is_answered_so_the_exporter_knows() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        Store::open(&db).unwrap().conn().execute_batch("CREATE TRIGGER full AFTER INSERT ON usage_event BEGIN SELECT RAISE(ABORT, 'disk full'); END;").unwrap();
        let r = Receiver::start(0, db.clone(), "k".into(), |_| {}).unwrap();
        let auth = "x-aiut-token: k\r\n";
        assert!(post(r.port, auth, "not json").starts_with("HTTP/1.1 400"));
        assert!(post(r.port, auth, &sample("req_a", "claude-haiku-4-5", 3).to_string()).starts_with("HTTP/1.1 503"), "503 asks for the batch again");
        assert!(post(r.port, auth, r#"{"resourceLogs":[]}"#).starts_with("HTTP/1.1 200"));

        // an archive that cannot be opened is reported before anything listens
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();
        assert!(Receiver::start(0, file.join("t.db"), "k".into(), |_| {}).is_err());
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

    fn send(port: u16, raw: &[u8]) -> String {
        use std::io::Write;
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(raw).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn a_hostile_length_or_a_missing_body_neither_crashes_nor_holds_the_receiver() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let r = Receiver::start(0, db.clone(), "k".into(), |_| {}).unwrap();
        for length in ["4611686018427387904", "18446744073709551615", "99999999999999999999999"] {
            let raw = format!("POST /v1/logs HTTP/1.1\r\nContent-Type: application/json\r\nx-aiut-token: k\r\nContent-Length: {length}\r\n\r\n");
            let got = send(r.port, raw.as_bytes());
            assert!(got.starts_with("HTTP/1.1 413") || got.starts_with("HTTP/1.1 400"), "{length}: {got}");
        }
        // without the token nothing is read, so a body that never comes keeps nothing waiting
        let t = Instant::now();
        let got = send(r.port, b"POST /v1/logs HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 5000\r\n\r\n");
        assert!(got.starts_with("HTTP/1.1 401") && t.elapsed() < Duration::from_secs(2), "{got}");
        let got = send(r.port, b"GET /v1/logs HTTP/1.1\r\nContent-Length: 5000\r\n\r\n");
        assert!(got.starts_with("HTTP/1.1 405"), "{got}");
        let body = sample("req_c", "claude-haiku-4-5", 3).to_string();
        assert!(post(r.port, "x-aiut-token: k\r\n", &body).starts_with("HTTP/1.1 200"), "still serving");
        assert_eq!(count(&db), 1);
    }

    #[test]
    fn a_chunked_body_is_read_and_too_many_waiting_senders_are_told_to_come_back() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let r = Receiver::start(0, db.clone(), "k".into(), |_| {}).unwrap();
        let body = sample("req_d", "claude-haiku-4-5", 3).to_string();
        let (a, b) = body.split_at(body.len() / 2);
        let raw = format!(
            "POST /v1/logs HTTP/1.1\r\nContent-Type: application/json\r\nx-aiut-token: k\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{a}\r\n{:x}\r\n{b}\r\n0\r\n\r\n",
            a.len(),
            b.len()
        );
        assert!(send(r.port, raw.as_bytes()).starts_with("HTTP/1.1 200"));
        assert_eq!(count(&db), 1);

        let stalled: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut s = std::net::TcpStream::connect(("127.0.0.1", r.port)).unwrap();
                s.write_all(b"POST /v1/logs HTTP/1.1\r\n").unwrap();
                s
            })
            .collect();
        std::thread::sleep(Duration::from_millis(300));
        assert!(post(r.port, "x-aiut-token: k\r\n", &body).starts_with("HTTP/1.1 503"), "busy: asked to send again later");
        drop(stalled);
        std::thread::sleep(Duration::from_millis(300));
        assert!(post(r.port, "x-aiut-token: k\r\n", &body).starts_with("HTTP/1.1 200"));
        let t = Instant::now();
        drop(r);
        assert!(t.elapsed() < Duration::from_secs(2));
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
