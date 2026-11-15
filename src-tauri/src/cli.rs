//! `ai-usage-tracker --json`: the usage and limits the app shows, printed for scripts and other tools.

use crate::commands::enabled_providers;
use crate::settings::Settings;
use chrono::{SecondsFormat, TimeZone};
use serde_json::{json, Value};
use std::fmt::Display;
use std::io::Write;
use std::path::Path;
use tracker_core::analytics::{self, Filter, LimitState, Period};
use tracker_core::pricing::PriceBook;
use tracker_core::store::Store;

pub(crate) const FLAG: &str = "--json";
/// The output's own version, raised only when a field changes meaning or goes away.
const SCHEMA: u32 = 1;
const PERIODS: [(&str, Period); 7] = [("today", Period::Today), ("7d", Period::Days7), ("30d", Period::Month1), ("3m", Period::Months3), ("6m", Period::Months6), ("1y", Period::Year1), ("all", Period::All)];

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn json_main() -> i32 {
    #[cfg(windows)]
    attach_console();
    let tr = crate::windows::system_is_turkish();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let period = match period_arg(&args) {
        Ok(p) => p,
        Err(name) => {
            let names = PERIODS.map(|(n, _)| n).join(", ");
            return fail(&if tr { format!("Bilinmeyen dönem: {name}. Şunlardan biri olabilir: {names}") } else { format!("Unknown period: {name}. Use one of: {names}") });
        }
    };
    let Some(data_dir) = tracker_core::store::default_data_dir() else { return fail(if tr { "Veri klasörü bulunamadı." } else { "No data folder found." }) };
    let db = data_dir.join("tracker.db");
    if !db.is_file() {
        return fail(if tr { "Henüz veri yok. AI Usage Tracker'ı bir kez açın." } else { "No data yet. Open AI Usage Tracker once." });
    }
    let store = match Store::open_read_only(&db) {
        Ok(s) => s,
        Err(e) => return fail(&e.to_string()),
    };
    match store.is_current() {
        Ok(true) => {}
        Ok(false) => return fail(if tr { "Veriler bu sürüme göre güncel değil. AI Usage Tracker'ı bir kez açıp yeniden deneyin." } else { "The data is not up to date for this version. Open AI Usage Tracker once and try again." }),
        Err(e) => return fail(&e.to_string()),
    }
    let settings = Settings::load(&store);
    let (book, _) = crate::state::load_price_book(&data_dir);
    match build(&store, &book, &settings, period, now_ms(), &chrono::Local) {
        Ok(v) => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            let _ = out.flush();
            0
        }
        Err(e) => fail(&e.to_string()),
    }
}

fn fail(message: &str) -> i32 {
    eprintln!("{message}");
    1
}

/// `--period 7d` or `--period=7d`; today when none is given.
fn period_arg(args: &[String]) -> Result<(&'static str, Period), String> {
    let mut it = args.iter();
    let mut name = None;
    while let Some(a) = it.next() {
        if a == "--period" {
            name = Some(it.next().cloned().unwrap_or_default());
        } else if let Some(v) = a.strip_prefix("--period=") {
            name = Some(v.to_owned());
        }
    }
    let Some(name) = name else { return Ok(PERIODS[0]) };
    PERIODS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).copied().ok_or(name)
}

pub(crate) fn build<Tz: TimeZone>(store: &Store, book: &PriceBook, settings: &Settings, (name, period): (&str, Period), now_ms: i64, tz: &Tz) -> analytics::Result<Value>
where
    Tz::Offset: Display,
{
    let at = |ms: i64| tz.timestamp_millis_opt(ms).single().map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true));
    let money = |usd: f64| (usd * 10_000.0).round() / 10_000.0;
    let range = analytics::period_range(period, tz, now_ms, store.first_event_ms()?);
    let r = analytics::report(store, book, range, &Filter::default(), tz, now_ms)?;
    let k = &r.totals.tokens;
    let by_tool: Vec<Value> = r
        .by_tool
        .iter()
        .map(|g| json!({ "tool": g.key, "requests": g.totals.events, "tokens": g.totals.total_tokens, "cost_usd": money(g.totals.cost_usd) }))
        .collect();
    // the account in use, as the widget and the menu bar show it
    let providers = enabled_providers(settings);
    let limits: Vec<Value> = analytics::limits_view(store, book, now_ms, &settings.thresholds)?
        .into_iter()
        .filter(|l| l.active && providers.contains(&l.provider))
        .map(|l| {
            json!({
                "provider": l.provider,
                "pool": (!l.limit_id.is_empty()).then_some(&l.limit_id),
                "window": l.window,
                "window_minutes": l.window_minutes,
                // a window that reset since the reading has an unknown use
                "used_pct": l.used_pct.filter(|_| l.state != LimitState::Reset),
                "resets_at": l.resets_at.and_then(|s| at(s.saturating_mul(1000))),
                "observed_at": l.observed_ms.and_then(at),
                "state": l.state,
                "fills_at": l.forecast.as_ref().and_then(|f| f.fills_at_ms).and_then(at),
                "blocked": l.blocked,
                "plan": l.plan,
            })
        })
        .collect();
    Ok(json!({
        "schema": SCHEMA,
        "generated_at": at(now_ms),
        "period": { "name": name, "from": at(range.from_ms), "to": at(range.to_ms) },
        "usage": {
            "requests": r.totals.events,
            "tokens": { "input": k.input, "output": k.output, "cache_read": k.cache_read, "cache_write": k.cache_write, "total": r.totals.total_tokens },
            "cost_usd": money(r.totals.cost_usd),
            "unpriced_requests": r.totals.unpriced_events,
            "by_tool": by_tool,
        },
        "limits": limits,
    }))
}

/// What to type in a terminal to run this copy: PowerShell on Windows, any POSIX shell elsewhere.
pub(crate) fn terminal_command(exe: &Path) -> String {
    let cmd = crate::capture::quoted_command(&exe.to_string_lossy(), None, !cfg!(windows), FLAG);
    // PowerShell waits for a windowed program only while it reads the program's output
    if cfg!(windows) { format!("{cmd} | Out-String") } else { cmd }
}

/// A windowed program has no console: piped output has a handle of its own, output typed in a
/// terminal goes to that terminal's console.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
    // SAFETY: plain Win32 calls with no pointers of ours
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracker_core::model::{Accuracy, LimitSnapshot, Provider, Tokens, Tool, UsageEvent};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_period_is_today_unless_another_is_named() {
        assert_eq!(period_arg(&args(&["--json"])), Ok(("today", Period::Today)));
        assert_eq!(period_arg(&args(&["--json", "--period", "7d"])), Ok(("7d", Period::Days7)));
        assert_eq!(period_arg(&args(&["--period=ALL", "--json"])), Ok(("all", Period::All)));
        assert_eq!(period_arg(&args(&["--json", "--period", "week"])), Err("week".into()));
        assert_eq!(period_arg(&args(&["--json", "--period"])), Err(String::new()));
    }

    fn event(key: &str, ts_ms: i64, tool: Tool, model: &str, input: u64, output: u64) -> UsageEvent {
        UsageEvent {
            key: key.into(),
            ts_ms,
            tool,
            client: Some("cli".into()),
            model: model.into(),
            project_path: Some("/p/notes".into()),
            session_id: Some("s".into()),
            request_input: input,
            tokens: Tokens { input, output, ..Default::default() },
            web_search_requests: 0,
            speed: None,
            service_tier: None,
            inference_geo: None,
            request_id: None,
            accuracy: Accuracy::Exact,
            source: "test".into(),
            branch: None,
            agent: None,
            thread_id: None,
            limit_reset: None,
            creator: None,
            turn: None,
        }
    }

    #[test]
    fn the_output_holds_the_periods_usage_and_the_limits_of_the_sources_in_use() {
        let tz = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        let now = tz.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap().timestamp_millis();
        let mut store = Store::open_in_memory().unwrap();
        let limit = |provider: Provider, tool: Tool, used: f64| LimitSnapshot {
            ts_ms: now - 60_000,
            provider,
            tool,
            account: None,
            limit_id: String::new(),
            window: "five_hour".into(),
            used_pct: Some(used),
            resets_at: Some(now / 1000 + 3600),
            status: None,
            plan: None,
            source: "test".into(),
            accuracy: Accuracy::Exact,
        };
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[
            event("a", now - 3_600_000, Tool::ClaudeCode, "claude-opus-5-5", 1_000, 200),
            event("b", now - 1_800_000, Tool::Codex, "gpt-5.6-terra", 500, 100),
            // yesterday, outside today
            event("c", now - 30 * 3_600_000, Tool::ClaudeCode, "claude-opus-5-5", 9_000, 900),
        ])
        .unwrap();
        tx.insert_limits(&[limit(Provider::Anthropic, Tool::ClaudeCode, 46.0), limit(Provider::Google, Tool::Antigravity, 10.0)]).unwrap();
        tx.commit().unwrap();
        let book = PriceBook::default_book();
        let settings = Settings::default();

        let v = build(&store, &book, &settings, ("today", Period::Today), now, &tz).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["generated_at"], "2026-11-15T12:00:00+03:00");
        assert_eq!((v["period"]["from"].as_str(), v["period"]["to"].as_str()), (Some("2026-11-15T00:00:00+03:00"), Some("2026-11-16T00:00:00+03:00")));
        assert_eq!((v["usage"]["requests"].as_u64(), v["usage"]["tokens"]["total"].as_u64()), (Some(2), Some(1_800)));
        let tools: Vec<&str> = v["usage"]["by_tool"].as_array().unwrap().iter().filter_map(|t| t["tool"].as_str()).collect();
        assert_eq!(tools.len(), 2, "{tools:?}");
        assert!(v["usage"]["cost_usd"].as_f64().unwrap() > 0.0);

        let limits = v["limits"].as_array().unwrap();
        assert_eq!(limits.len(), 1, "a source that is off shows no limits: {limits:?}");
        assert_eq!((limits[0]["provider"].as_str(), limits[0]["window"].as_str(), limits[0]["used_pct"].as_f64()), (Some("anthropic"), Some("five_hour"), Some(46.0)));
        assert_eq!(limits[0]["resets_at"], "2026-11-15T13:00:00+03:00");

        let week = build(&store, &book, &settings, ("7d", Period::Days7), now, &tz).unwrap();
        assert_eq!((week["period"]["name"].as_str(), week["usage"]["requests"].as_u64()), (Some("7d"), Some(3)));
    }

    #[test]
    fn a_read_only_archive_is_read_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tracker.db");
        let mut store = Store::open(&path).unwrap();
        let mut tx = store.transaction().unwrap();
        tx.upsert_events(&[event("a", now_ms() - 60_000, Tool::ClaudeCode, "claude-opus-5-5", 1_000, 200)]).unwrap();
        tx.commit().unwrap();
        drop(store);

        let ro = Store::open_read_only(&path).unwrap();
        assert!(ro.is_current().unwrap());
        let v = build(&ro, &PriceBook::default_book(), &Settings::load(&ro), PERIODS[0], now_ms(), &chrono::Utc).unwrap();
        assert_eq!(v["usage"]["requests"], 1);
        assert!(ro.conn().execute("DELETE FROM usage_event", []).is_err(), "nothing can be written");

        Store::open(&path).unwrap().conn().pragma_update(None, "user_version", 1).unwrap();
        assert!(!Store::open_read_only(&path).unwrap().is_current().unwrap(), "an archive an older build left");
    }
}
