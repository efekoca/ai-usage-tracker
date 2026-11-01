//! System notifications when a limit in use passes the warning threshold, fills up, and starts
//! again after filling.

use crate::settings::{NotifySettings, Settings};
use crate::tray_text::{name, shown_value, Format, Reading};
use chrono::TimeZone;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tauri::AppHandle;
use tracker_core::model::Provider;

/// Readings round their reset time differently; a larger move is a window that started again.
const MOVED_SECS: i64 = 15 * 60;

static SEEN: LazyLock<Mutex<HashMap<Limit, Seen>>> = LazyLock::new(Default::default);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Limit {
    provider: Provider,
    pool: String,
    window: String,
}

impl Limit {
    fn tag(&self) -> String {
        format!("{}-{}-{}", self.provider.as_str(), self.pool, self.window)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Below,
    Close,
    Full,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    /// The highest step already told in this window.
    step: Step,
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
enum Alert {
    Close { limit: Limit, used: f64, fills_at_ms: Option<i64>, resets_at: Option<i64> },
    Full { limit: Limit, resets_at: Option<i64> },
    Reset { limit: Limit },
}

impl Alert {
    fn limit(&self) -> &Limit {
        match self {
            Alert::Close { limit, .. } | Alert::Full { limit, .. } | Alert::Reset { limit } => limit,
        }
    }
}

pub(crate) fn check(app: &AppHandle, s: &Settings, readings: &[Reading]) {
    if !s.onboarded {
        return;
    }
    let n = &s.notify;
    let early_at = match n.early.as_str() {
        "high" => s.widget.high_at,
        "off" => f64::INFINITY,
        _ => s.widget.warn_at,
    };
    // every limit is followed even while its notifications are off, so turning them on never
    // reports what happened before
    let alerts = changes(&mut SEEN.lock().unwrap(), readings, early_at, chrono::Utc::now().timestamp());
    if !n.enabled {
        return;
    }
    let f = crate::tray::format(s, &s.tray);
    for a in alerts.iter().filter(|a| wanted(a, n)) {
        let (title, body) = text(a, &f);
        log::info!("limit notification: {}", a.limit().tag());
        send(app, &a.limit().tag(), &title, &body);
    }
}

fn wanted(a: &Alert, n: &NotifySettings) -> bool {
    let kind = match a {
        Alert::Close { .. } => true,
        Alert::Full { .. } => n.full,
        Alert::Reset { .. } => n.reset,
    };
    let window = match a.limit().window.as_str() {
        "five_hour" => n.five_hour,
        _ => n.seven_day,
    };
    kind && window
}

fn step(used: f64, early_at: f64) -> Step {
    if shown_value(used, false) >= 100 {
        Step::Full
    } else if used >= early_at {
        Step::Close
    } else {
        Step::Below
    }
}

/// A limit seen for the first time only records where it stands, so starting the app or a new
/// account never notifies. Readings from before a reset are left out.
fn changes(seen: &mut HashMap<Limit, Seen>, readings: &[Reading], early_at: f64, now: i64) -> Vec<Alert> {
    let mut out = Vec::new();
    for (limit, s) in seen.iter_mut() {
        if s.resets_at.is_some_and(|at| at <= now) {
            if s.step == Step::Full {
                out.push(Alert::Reset { limit: limit.clone() });
            }
            *s = Seen { step: Step::Below, resets_at: None };
        }
    }
    for r in readings.iter().filter(|r| r.active && r.resets_at.is_none_or(|at| at > now)) {
        let limit = Limit { provider: r.provider, pool: r.pool.clone(), window: r.window.clone() };
        let now_step = step(r.used, early_at);
        let Some(s) = seen.get_mut(&limit) else {
            seen.insert(limit, Seen { step: now_step, resets_at: r.resets_at });
            continue;
        };
        // e.g. a Claude week that starts again early
        if let (Some(was), Some(at)) = (s.resets_at, r.resets_at)
            && (was - at).abs() > MOVED_SECS
        {
            if s.step == Step::Full {
                out.push(Alert::Reset { limit: limit.clone() });
            }
            s.step = Step::Below;
        }
        s.resets_at = r.resets_at.or(s.resets_at);
        if now_step > s.step {
            s.step = now_step;
            out.push(match now_step {
                Step::Full => Alert::Full { limit, resets_at: r.resets_at },
                _ => Alert::Close { limit, used: r.used, fills_at_ms: r.fills_at_ms, resets_at: r.resets_at },
            });
        }
    }
    out
}

/// Title and body. Numbers and times carry no Turkish suffix, which would depend on how they read.
fn text<Tz: TimeZone>(a: &Alert, f: &Format<'_, Tz>) -> (String, String) {
    let tr = f.tr;
    let limit = |l: &Limit| {
        let window = match (l.window.as_str(), tr) {
            ("five_hour", true) => "5 saatlik",
            ("five_hour", false) => "5-hour",
            (_, true) => "haftalık",
            (_, false) => "weekly",
        };
        format!("{} {window}", name(l.provider, &l.pool, tr))
    };
    let resets = |at: Option<i64>| at.and_then(|at| f.clock(at)).map(|c| if tr { format!("Yenilenme: {c}.") } else { format!("Resets: {c}.") });
    match a {
        Alert::Close { limit: l, used, fills_at_ms, resets_at } => {
            let v = shown_value(*used, f.app_remaining);
            let title = match (tr, f.app_remaining) {
                (true, false) => format!("{} limit: %{v} kullanıldı", limit(l)),
                (true, true) => format!("{} limit: %{v} kaldı", limit(l)),
                (false, false) => format!("{} limit: {v}% used", limit(l)),
                (false, true) => format!("{} limit: {v}% left", limit(l)),
            };
            let fills = fills_at_ms
                .map(|ms| ms / 1000)
                .filter(|at| resets_at.is_none_or(|r| *at < r))
                .and_then(|at| f.clock(at))
                .map(|c| if tr { format!("Bu hızla {c} civarında dolacak.") } else { format!("At this pace it will fill around {c}.") });
            (title, [fills, resets(*resets_at)].into_iter().flatten().collect::<Vec<_>>().join(" "))
        }
        Alert::Full { limit: l, resets_at } => {
            let title = if tr { format!("{} limitiniz doldu", limit(l)) } else { format!("Your {} limit is full", limit(l)) };
            (title, resets(*resets_at).unwrap_or_default())
        }
        Alert::Reset { limit: l } => {
            let title = if tr { format!("{} limitiniz yenilendi", limit(l)) } else { format!("Your {} limit has reset", limit(l)) };
            (title, if tr { "Kullanmaya başlayabilirsiniz." } else { "You can start using it." }.to_owned())
        }
    }
}

/// A newer notification of the same limit replaces the older one.
#[cfg(target_os = "macos")]
fn send(_: &AppHandle, tag: &str, title: &str, body: &str) {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter};

    // the notification center throws outside an app bundle, e.g. in a development build
    if !NSBundle::mainBundle().bundlePath().to_string().ends_with(".app") {
        log::info!("notifications need the app bundle");
        return;
    }
    let (tag, title, body) = (tag.to_owned(), title.to_owned(), body.to_owned());
    // macOS asks once; later calls answer at once
    let then = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            // SAFETY: an error macOS hands over lives for this call
            let why = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            log::info!("notifications not allowed: {}", why.unwrap_or_default());
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&tag), &content, None);
        let added = RcBlock::new(|error: *mut NSError| {
            // SAFETY: as above
            if let Some(e) = unsafe { error.as_ref() } {
                log::warn!("notification: {}", e.localizedDescription());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&added));
    });
    UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &then);
}

/// Shown under the app's name through the Start menu shortcut, which carries the app's identifier.
#[cfg(windows)]
fn send(app: &AppHandle, tag: &str, title: &str, body: &str) {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    let show = || -> windows::core::Result<()> {
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(format!(
            r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
            escape(title),
            escape(body)
        )))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        toast.SetTag(&HSTRING::from(tag))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app.config().identifier.as_str()))?.Show(&toast)
    };
    if let Err(e) = show() {
        log::warn!("notification: {e}");
    }
}

#[cfg(any(windows, test))]
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(target_os = "linux")]
fn send(_: &AppHandle, _: &str, title: &str, body: &str) {
    use zbus::zvariant::Value;
    let show = || -> zbus::Result<()> {
        let hints = HashMap::from([("desktop-entry", Value::from("ai-usage-tracker"))]);
        zbus::blocking::Connection::session()?.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("AI Usage Tracker", 0u32, "ai-usage-tracker", title, body, Vec::<&str>::new(), hints, -1i32),
        )?;
        Ok(())
    };
    if let Err(e) = show() {
        log::warn!("notification: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::TraySettings;
    use chrono::FixedOffset;

    const NOW: i64 = 1_800_000_000;

    fn read(provider: Provider, window: &str, used: f64, resets_in: i64) -> Reading {
        Reading {
            provider,
            pool: if provider == Provider::OpenAI { "codex".into() } else { String::new() },
            window: window.into(),
            used,
            behind: false,
            resets_at: Some(NOW + resets_in),
            fills_at_ms: None,
            at_reset: None,
            account: None,
            active: true,
        }
    }

    fn kinds(alerts: &[Alert]) -> Vec<&'static str> {
        alerts
            .iter()
            .map(|a| match a {
                Alert::Close { .. } => "close",
                Alert::Full { .. } => "full",
                Alert::Reset { .. } => "reset",
            })
            .collect()
    }

    #[test]
    fn a_limit_notifies_once_per_step_and_again_after_it_resets() {
        let mut seen = HashMap::new();
        let at = |used| [read(Provider::Anthropic, "five_hour", used, 3600)];
        assert!(changes(&mut seen, &at(80.0), 70.0, NOW).is_empty(), "the first reading only records");
        assert!(changes(&mut seen, &at(85.0), 70.0, NOW).is_empty(), "already past the threshold");
        assert_eq!(kinds(&changes(&mut seen, &at(100.0), 70.0, NOW + 60)), ["full"]);
        assert!(changes(&mut seen, &at(100.0), 70.0, NOW + 120).is_empty());
        assert_eq!(kinds(&changes(&mut seen, &at(100.0), 70.0, NOW + 3600)), ["reset"], "at the reset time, before a newer reading");
        assert!(changes(&mut seen, &[], 70.0, NOW + 3700).is_empty());
        let next = [read(Provider::Anthropic, "five_hour", 75.0, 4 * 3600)];
        assert_eq!(kinds(&changes(&mut seen, &next, 70.0, NOW)), ["close"], "the new window starts from below");
    }

    #[test]
    fn a_week_that_starts_again_early_counts_as_a_reset() {
        let mut seen = HashMap::new();
        changes(&mut seen, &[read(Provider::Anthropic, "seven_day", 60.0, 3 * 86_400)], 70.0, NOW);
        assert_eq!(kinds(&changes(&mut seen, &[read(Provider::Anthropic, "seven_day", 100.0, 3 * 86_400 + 30)], 70.0, NOW)), ["full"]);
        let restarted = [read(Provider::Anthropic, "seven_day", 2.0, 7 * 86_400)];
        assert_eq!(kinds(&changes(&mut seen, &restarted, 70.0, NOW)), ["reset"]);
    }

    #[test]
    fn only_limits_in_use_and_readings_from_the_current_window_count() {
        let mut seen = HashMap::new();
        let mut other = read(Provider::OpenAI, "five_hour", 10.0, 3600);
        other.active = false;
        let stale = read(Provider::Anthropic, "five_hour", 10.0, -60);
        changes(&mut seen, &[other.clone(), stale.clone()], 70.0, NOW);
        assert!(seen.is_empty());
        other.used = 100.0;
        assert!(changes(&mut seen, &[other], 70.0, NOW).is_empty());
    }

    #[test]
    fn the_settings_pick_the_kinds_and_windows_that_notify() {
        let mut seen = HashMap::new();
        let at = |used, window| [read(Provider::Anthropic, window, used, 3600)];
        changes(&mut seen, &at(10.0, "five_hour"), f64::INFINITY, NOW);
        assert_eq!(kinds(&changes(&mut seen, &at(95.0, "five_hour"), f64::INFINITY, NOW)), Vec::<&str>::new(), "no early warning");
        assert_eq!(kinds(&changes(&mut seen, &at(100.0, "five_hour"), f64::INFINITY, NOW)), ["full"]);

        let n = NotifySettings { full: false, seven_day: false, ..Default::default() };
        let limit = |window: &str| Limit { provider: Provider::Anthropic, pool: String::new(), window: window.into() };
        assert!(wanted(&Alert::Close { limit: limit("five_hour"), used: 80.0, fills_at_ms: None, resets_at: None }, &n));
        assert!(!wanted(&Alert::Full { limit: limit("five_hour"), resets_at: None }, &n));
        assert!(wanted(&Alert::Reset { limit: limit("five_hour") }, &n));
        assert!(!wanted(&Alert::Reset { limit: limit("seven_day") }, &n));
    }

    #[test]
    fn texts_name_the_limit_and_avoid_suffixes_after_numbers() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let cfg = TraySettings::default();
        let f = |tr, remaining| Format { cfg: &cfg, tr, app_remaining: remaining, warn_at: 70.0, currency: "USD", fx: 1.0, now_ms: NOW * 1000, tz: &tz };
        let claude = Limit { provider: Provider::Anthropic, pool: String::new(), window: "five_hour".into() };
        let close = Alert::Close { limit: claude.clone(), used: 72.0, fills_at_ms: Some((NOW + 1800) * 1000), resets_at: Some(NOW + 3600) };
        let clock = |secs| f(true, false).clock(NOW + secs).unwrap();
        let (half, hour) = (clock(1800), clock(3600));
        assert_eq!(text(&close, &f(true, false)), ("Claude 5 saatlik limit: %72 kullanıldı".into(), format!("Bu hızla {half} civarında dolacak. Yenilenme: {hour}.")));
        assert_eq!(text(&close, &f(false, true)), ("Claude 5-hour limit: 28% left".into(), format!("At this pace it will fill around {half}. Resets: {hour}.")));
        let week = Limit { provider: Provider::OpenAI, pool: "codex".into(), window: "seven_day".into() };
        assert_eq!(text(&Alert::Full { limit: week.clone(), resets_at: None }, &f(true, false)), ("Codex haftalık limitiniz doldu".into(), String::new()));
        assert_eq!(text(&Alert::Reset { limit: week }, &f(false, false)).0, "Your Codex weekly limit has reset");
        assert_eq!(escape(r#"<a & "b">"#), "&lt;a &amp; &quot;b&quot;&gt;");
    }
}
