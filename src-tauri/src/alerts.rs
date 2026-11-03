//! System notifications when a limit in use passes the warning threshold, fills up, and starts
//! again after filling, and when the monthly budget gets close or is exceeded. Clicking one opens
//! the page it is about.

use crate::settings::{NotifySettings, Settings};
use crate::tray_text::{name, shown_value, Format, Reading};
use chrono::TimeZone;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Manager};
use tracker_core::model::Provider;

/// Readings round their reset time differently; a larger move is a window that started again.
const MOVED_SECS: i64 = 15 * 60;

static SEEN: LazyLock<Mutex<HashMap<Limit, Seen>>> = LazyLock::new(Default::default);
static BUDGET: Mutex<Option<BudgetSeen>> = Mutex::new(None);

/// The budget sums the whole month, so it is checked only every few minutes.
const BUDGET_EVERY_MS: i64 = 5 * 60_000;
const BUDGET_CLOSE_PCT: f64 = 80.0;

/// What became of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Sent {
    Shown,
    /// macOS: the user did not allow the app's notifications.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Denied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Limit {
    provider: Provider,
    pool: String,
    window: String,
    /// Each account's limits are followed apart, so switching accounts is not a reset.
    account: Option<String>,
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

struct BudgetSeen {
    month: String,
    step: Step,
    budget_usd: f64,
    checked_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct BudgetAlert {
    over: bool,
    used_pct: f64,
    spent_usd: f64,
    budget_usd: f64,
    projected_usd: Option<f64>,
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
    let now_ms = chrono::Utc::now().timestamp_millis();
    let alerts = changes(&mut SEEN.lock().unwrap(), readings, early_at, now_ms / 1000);
    let budget = budget(app, s, now_ms);
    if !n.enabled {
        return;
    }
    let f = crate::tray::format(s, &s.tray);
    for a in alerts.iter().filter(|a| wanted(a, n)) {
        let (title, body) = text(a, &f);
        log::info!("limit notification: {}", a.limit().tag());
        send(app, &a.limit().tag(), &title, &body, |_| {});
    }
    if let Some(b) = budget.filter(|_| n.budget) {
        let (title, body) = budget_text(&b, &f);
        log::info!("budget notification");
        send(app, "budget", &title, &body, |_| {});
    }
}

fn budget(app: &AppHandle, s: &Settings, now_ms: i64) -> Option<BudgetAlert> {
    let mut seen = BUDGET.lock().unwrap();
    if s.monthly_budget_usd <= 0.0 {
        // a budget set later starts from where the month stands then
        *seen = None;
        return None;
    }
    if seen.as_ref().is_some_and(|b| now_ms - b.checked_ms < BUDGET_EVERY_MS) {
        return None;
    }
    let state = app.state::<crate::state::AppState>();
    let spend = {
        let store = state.db();
        let book = state.book.read().unwrap();
        tracker_core::analytics::month_spend(&store, &book, &chrono::Local, now_ms).ok()?
    };
    let used_pct = spend.spent_usd / s.monthly_budget_usd * 100.0;
    let step = budget_change(&mut seen, &spend.month, s.monthly_budget_usd, used_pct, now_ms)?;
    Some(BudgetAlert {
        over: step == Step::Full,
        used_pct,
        spent_usd: spend.spent_usd,
        budget_usd: s.monthly_budget_usd,
        projected_usd: spend.projected_usd,
    })
}

/// Like a limit, the budget is first only recorded, and a new month starts from below.
fn budget_change(seen: &mut Option<BudgetSeen>, month: &str, budget_usd: f64, used_pct: f64, now_ms: i64) -> Option<Step> {
    let now_step = if used_pct >= 100.0 {
        Step::Full
    } else if used_pct >= BUDGET_CLOSE_PCT {
        Step::Close
    } else {
        Step::Below
    };
    let Some(b) = seen else {
        *seen = Some(BudgetSeen { month: month.into(), step: now_step, budget_usd, checked_ms: now_ms });
        return None;
    };
    b.checked_ms = now_ms;
    if b.month != month {
        (b.month, b.step) = (month.into(), Step::Below);
    }
    // a raised budget can be reached again; a lowered one notifies at once
    if b.budget_usd != budget_usd {
        (b.budget_usd, b.step) = (budget_usd, b.step.min(now_step));
    }
    (now_step > b.step).then(|| {
        b.step = now_step;
        now_step
    })
}

/// Sends a sample notification and waits for how it went; on a Mac, until the user answers
/// whether the app may send notifications.
pub(crate) fn test(app: &AppHandle, s: &Settings) -> Sent {
    let (title, body) = if crate::tray::turkish(s) {
        ("Bildirimler açık", "Limitleriniz dolmaya yaklaşınca böyle bir bildirim alırsınız.")
    } else {
        ("Notifications are on", "You'll get a notification like this when a limit gets close.")
    };
    let (tx, rx) = std::sync::mpsc::channel();
    send(app, "test", title, body, move |r| {
        let _ = tx.send(r);
    });
    rx.recv_timeout(std::time::Duration::from_secs(300)).unwrap_or(Sent::Failed)
}

/// The budget is on the Overview; every other notification is about the Limits page.
fn open(app: &AppHandle, tag: &str) {
    crate::windows::show_main_at(app, Some(if tag == "budget" { "overview" } else { "limits" }));
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
/// account never notifies. Readings from before a reset are left out. An account not in use
/// resets quietly: its notification would read as if the limit in use were free again.
fn changes(seen: &mut HashMap<Limit, Seen>, readings: &[Reading], early_at: f64, now: i64) -> Vec<Alert> {
    let mut out = Vec::new();
    let in_use: HashSet<(Provider, Option<&str>)> = readings.iter().filter(|r| r.active).map(|r| (r.provider, r.account.as_deref())).collect();
    for (limit, s) in seen.iter_mut() {
        if s.resets_at.is_some_and(|at| at <= now) {
            if s.step == Step::Full && in_use.contains(&(limit.provider, limit.account.as_deref())) {
                out.push(Alert::Reset { limit: limit.clone() });
            }
            *s = Seen { step: Step::Below, resets_at: None };
        }
    }
    for r in readings.iter().filter(|r| r.active && r.resets_at.is_none_or(|at| at > now)) {
        let limit = Limit { provider: r.provider, pool: r.pool.clone(), window: r.window.clone(), account: r.account.clone() };
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

/// Lets notifications show while the app is in front too, and opens their page on a click.
#[cfg(target_os = "macos")]
pub(crate) fn init(app: &AppHandle) {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
    use objc2_user_notifications::{
        UNNotification, UNNotificationPresentationOptions, UNNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Delegate does not implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "AIUsageTrackerNotificationDelegate"]
        #[ivars = AppHandle]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(&self, _: &UNUserNotificationCenter, _: &UNNotification, done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
                done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
                open(self.ivars(), &response.notification().request().identifier().to_string());
                done.call(());
            }
        }
    );

    if !in_bundle() {
        return;
    }
    let this = Delegate::alloc().set_ivars(app.clone());
    let delegate: Retained<Delegate> = unsafe { msg_send![super(this), init] };
    UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // the center holds its delegate weakly
    std::mem::forget(delegate);
}

/// The notification center throws outside an app bundle, e.g. in a development build.
#[cfg(target_os = "macos")]
fn in_bundle() -> bool {
    objc2_foundation::NSBundle::mainBundle().bundlePath().to_string().ends_with(".app")
}

fn budget_text<Tz: TimeZone>(b: &BudgetAlert, f: &Format<'_, Tz>) -> (String, String) {
    let (spent, budget) = (f.money(b.spent_usd), f.money(b.budget_usd));
    if b.over {
        return if f.tr {
            ("Aylık bütçenizi aştınız".into(), format!("Bu ay {spent} / {budget}."))
        } else {
            ("You are over your monthly budget".into(), format!("{spent} of {budget} this month."))
        };
    }
    let pct = (b.used_pct.round() as u32).min(99);
    let title = if f.tr { format!("Aylık bütçe: %{pct} kullanıldı") } else { format!("Monthly budget: {pct}% used") };
    let pace = b.projected_usd.map(|p| f.money(p));
    let body = match (f.tr, pace) {
        (true, Some(p)) => format!("Bu ay {spent} / {budget}. Bu hızla ay sonunda {p}."),
        (true, None) => format!("Bu ay {spent} / {budget}."),
        (false, Some(p)) => format!("{spent} of {budget} this month. At this pace: {p} by the month's end."),
        (false, None) => format!("{spent} of {budget} this month."),
    };
    (title, body)
}

/// macOS answers in callbacks; the first answer counts.
#[cfg(target_os = "macos")]
type Done = std::sync::Arc<Mutex<Option<Box<dyn FnOnce(Sent) + Send>>>>;

#[cfg(target_os = "macos")]
fn finish(done: &Done, r: Sent) {
    if let Some(f) = done.lock().unwrap().take() {
        f(r);
    }
}

/// A newer notification of the same limit replaces the older one.
#[cfg(target_os = "macos")]
fn send(_: &AppHandle, tag: &str, title: &str, body: &str, done: impl FnOnce(Sent) + Send + 'static) {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter};

    let done: Done = std::sync::Arc::new(Mutex::new(Some(Box::new(done))));
    if !in_bundle() {
        log::info!("notifications need the app bundle");
        return finish(&done, Sent::Failed);
    }
    let (tag, title, body) = (tag.to_owned(), title.to_owned(), body.to_owned());
    // macOS asks once; later calls answer at once
    let then = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            // SAFETY: an error macOS hands over lives for this call
            let why = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            log::info!("notifications not allowed: {}", why.unwrap_or_default());
            return finish(&done, Sent::Denied);
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&tag), &content, None);
        let done = done.clone();
        let added = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: as above
            match unsafe { error.as_ref() } {
                Some(e) => {
                    log::warn!("notification: {}", e.localizedDescription());
                    finish(&done, Sent::Failed);
                }
                None => finish(&done, Sent::Shown),
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&added));
    });
    UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &then);
}

/// Toasts kept so a click on one still reaches the app while it is on screen.
#[cfg(windows)]
static TOASTS: LazyLock<Mutex<HashMap<String, windows::UI::Notifications::ToastNotification>>> = LazyLock::new(Default::default);

/// Shown under the app's name through the Start menu shortcut, which carries the app's identifier.
#[cfg(windows)]
fn send(app: &AppHandle, tag: &str, title: &str, body: &str, done: impl FnOnce(Sent) + Send + 'static) {
    use windows::core::{IInspectable, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
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
        let (clicked, page) = (app.clone(), tag.to_owned());
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
            open(&clicked, &page);
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app.config().identifier.as_str()))?.Show(&toast)?;
        TOASTS.lock().unwrap().insert(tag.to_owned(), toast);
        Ok(())
    };
    match show() {
        Ok(()) => done(Sent::Shown),
        Err(e) => {
            log::warn!("notification: {e}");
            done(Sent::Failed);
        }
    }
}

#[cfg(any(windows, test))]
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(target_os = "linux")]
const BUS: &str = "org.freedesktop.Notifications";
#[cfg(target_os = "linux")]
const BUS_PATH: &str = "/org/freedesktop/Notifications";

/// The server's id for each limit's newest notification, which the next one replaces.
#[cfg(target_os = "linux")]
static IDS: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(Default::default);

/// One connection for every notification and for the clicks: GNOME takes an app's
/// notifications away as soon as the connection that sent them closes.
#[cfg(target_os = "linux")]
static SESSION: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);

#[cfg(target_os = "linux")]
fn session() -> zbus::Result<zbus::blocking::Connection> {
    let mut held = SESSION.lock().unwrap();
    if let Some(c) = held.as_ref() {
        return Ok(c.clone());
    }
    let c = zbus::blocking::Connection::session()?;
    *held = Some(c.clone());
    Ok(c)
}

#[cfg(target_os = "linux")]
fn send(app: &AppHandle, tag: &str, title: &str, body: &str, done: impl FnOnce(Sent) + Send + 'static) {
    use zbus::zvariant::Value;
    static LISTENING: std::sync::Once = std::sync::Once::new();

    let replaces = IDS.lock().unwrap().get(tag).copied().unwrap_or(0);
    let show = || -> zbus::Result<u32> {
        let hints = HashMap::from([("desktop-entry", Value::from("ai-usage-tracker"))]);
        let reply = session()?.call_method(
            Some(BUS),
            BUS_PATH,
            Some(BUS),
            "Notify",
            &("AI Usage Tracker", replaces, "ai-usage-tracker", title, body, vec!["default", ""], hints, -1i32),
        )?;
        reply.body().deserialize::<u32>()
    };
    match show() {
        Ok(id) => {
            IDS.lock().unwrap().insert(tag.to_owned(), id);
            LISTENING.call_once(|| listen_for_clicks(app.clone()));
            done(Sent::Shown);
        }
        Err(e) => {
            log::warn!("notification: {e}");
            done(Sent::Failed);
        }
    }
}

#[cfg(target_os = "linux")]
fn listen_for_clicks(app: AppHandle) {
    let _ = std::thread::Builder::new().name("notification-clicks".into()).spawn(move || {
        let run = || -> zbus::Result<()> {
            let c = session()?;
            for m in zbus::blocking::Proxy::new(&c, BUS, BUS_PATH, BUS)?.receive_signal("ActionInvoked")? {
                let (id, key): (u32, String) = m.body().deserialize()?;
                let tag = IDS.lock().unwrap().iter().find(|(_, v)| **v == id).map(|(t, _)| t.clone());
                if let (Some(tag), "default") = (tag, key.as_str()) {
                    open(&app, &tag);
                }
            }
            Ok(())
        };
        if let Err(e) = run() {
            log::warn!("notification clicks: {e}");
        }
    });
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
    fn switching_to_another_account_is_not_a_reset() {
        let mut seen = HashMap::new();
        let on = |account: &str, used, resets_in| Reading { account: Some(account.into()), ..read(Provider::Anthropic, "five_hour", used, resets_in) };
        changes(&mut seen, &[on("a", 90.0, 3600)], 70.0, NOW);
        assert_eq!(kinds(&changes(&mut seen, &[on("a", 100.0, 3600)], 70.0, NOW)), ["full"]);
        assert!(changes(&mut seen, &[on("b", 75.0, 9000)], 70.0, NOW).is_empty(), "b is only recorded");
        assert_eq!(kinds(&changes(&mut seen, &[on("b", 100.0, 9000)], 70.0, NOW)), ["full"]);
        assert!(changes(&mut seen, &[on("b", 100.0, 9000)], 70.0, NOW + 3601).is_empty(), "a's reset while b is in use and full stays quiet");
        assert_eq!(kinds(&changes(&mut seen, &[on("a", 100.0, 20_000)], 70.0, NOW + 3700)), ["full"], "a fills again after switching back");
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
        let limit = |window: &str| Limit { provider: Provider::Anthropic, pool: String::new(), window: window.into(), account: None };
        assert!(wanted(&Alert::Close { limit: limit("five_hour"), used: 80.0, fills_at_ms: None, resets_at: None }, &n));
        assert!(!wanted(&Alert::Full { limit: limit("five_hour"), resets_at: None }, &n));
        assert!(wanted(&Alert::Reset { limit: limit("five_hour") }, &n));
        assert!(!wanted(&Alert::Reset { limit: limit("seven_day") }, &n));
    }

    #[test]
    fn the_budget_notifies_once_at_80_percent_and_once_past_it_each_month() {
        let mut seen = None;
        assert_eq!(budget_change(&mut seen, "2026-11-01", 500.0, 85.0, 0), None, "the first check only records");
        assert_eq!(budget_change(&mut seen, "2026-11-01", 500.0, 90.0, 1), None);
        assert_eq!(budget_change(&mut seen, "2026-11-01", 500.0, 101.0, 2), Some(Step::Full));
        assert_eq!(budget_change(&mut seen, "2026-11-01", 500.0, 120.0, 3), None);
        assert_eq!(budget_change(&mut seen, "2026-12-01", 500.0, 2.0, 4), None, "a new month starts from below");
        assert_eq!(budget_change(&mut seen, "2026-12-01", 500.0, 80.0, 5), Some(Step::Close));
    }

    #[test]
    fn a_raised_budget_notifies_again_and_a_lowered_one_at_once() {
        let mut seen = None;
        budget_change(&mut seen, "2026-11-01", 500.0, 50.0, 0);
        assert_eq!(budget_change(&mut seen, "2026-11-01", 500.0, 102.0, 1), Some(Step::Full));
        assert_eq!(budget_change(&mut seen, "2026-11-01", 1000.0, 51.0, 2), None, "raised: $510 of $1000");
        assert_eq!(budget_change(&mut seen, "2026-11-01", 1000.0, 80.0, 3), Some(Step::Close));
        assert_eq!(budget_change(&mut seen, "2026-11-01", 1000.0, 100.0, 4), Some(Step::Full));
        assert_eq!(budget_change(&mut seen, "2026-11-01", 2000.0, 60.0, 5), None);
        assert_eq!(budget_change(&mut seen, "2026-11-01", 1000.0, 120.0, 6), Some(Step::Full), "lowered under the spend");
    }

    #[test]
    fn texts_name_the_limit_and_avoid_suffixes_after_numbers() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let cfg = TraySettings::default();
        let f = |tr, remaining| Format { cfg: &cfg, tr, app_remaining: remaining, warn_at: 70.0, currency: "USD", fx: 1.0, now_ms: NOW * 1000, tz: &tz };
        let claude = Limit { provider: Provider::Anthropic, pool: String::new(), window: "five_hour".into(), account: None };
        let close = Alert::Close { limit: claude.clone(), used: 72.0, fills_at_ms: Some((NOW + 1800) * 1000), resets_at: Some(NOW + 3600) };
        let clock = |secs| f(true, false).clock(NOW + secs).unwrap();
        let (half, hour) = (clock(1800), clock(3600));
        assert_eq!(text(&close, &f(true, false)), ("Claude 5 saatlik limit: %72 kullanıldı".into(), format!("Bu hızla {half} civarında dolacak. Yenilenme: {hour}.")));
        assert_eq!(text(&close, &f(false, true)), ("Claude 5-hour limit: 28% left".into(), format!("At this pace it will fill around {half}. Resets: {hour}.")));
        let week = Limit { provider: Provider::OpenAI, pool: "codex".into(), window: "seven_day".into(), account: None };
        assert_eq!(text(&Alert::Full { limit: week.clone(), resets_at: None }, &f(true, false)), ("Codex haftalık limitiniz doldu".into(), String::new()));
        assert_eq!(text(&Alert::Reset { limit: week }, &f(false, false)).0, "Your Codex weekly limit has reset");
        assert_eq!(escape(r#"<a & "b">"#), "&lt;a &amp; &quot;b&quot;&gt;");

        let b = BudgetAlert { over: false, used_pct: 82.4, spent_usd: 412.0, budget_usd: 500.0, projected_usd: Some(610.0) };
        assert_eq!(budget_text(&b, &f(true, false)), ("Aylık bütçe: %82 kullanıldı".into(), "Bu ay $412,00 / $500,00. Bu hızla ay sonunda $610,00.".into()));
        let over = BudgetAlert { over: true, used_pct: 103.0, spent_usd: 515.0, projected_usd: None, ..b };
        assert_eq!(budget_text(&over, &f(false, false)), ("You are over your monthly budget".into(), "$515.00 of $500.00 this month.".into()));
    }
}
