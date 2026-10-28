//! The text next to the tray icon, built from the user's `TraySettings`.
//! Pure, so the Menu bar page's preview and the tray show exactly the same text.

use crate::settings::{TrayItem, TraySettings};
use chrono::{DateTime, Datelike, TimeZone, Timelike};
use tracker_core::model::Provider;

/// One current limit window of one pool.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub provider: Provider,
    /// Antigravity's pool (`gemini`, `3p`); empty elsewhere.
    pub pool: String,
    pub window: String,
    pub used: f64,
    /// Used after the reading: the real value is at least `used`.
    pub behind: bool,
    /// Unix seconds.
    pub resets_at: Option<i64>,
    /// When the window fills at its pace so far.
    pub fills_at_ms: Option<i64>,
    /// Where it would stand at the reset when it does not fill.
    pub at_reset: Option<f64>,
    /// The Codex account it belongs to, while there are several.
    pub account: Option<String>,
    /// The provider's account in use.
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
pub struct Usage {
    pub tokens: u64,
    pub cost_usd: f64,
    pub sessions: u64,
    /// Input, cache reads and cache writes: what a cache hit rate is out of.
    pub prompt: u64,
    pub cache_read: u64,
}

pub const PERIODS: [&str; 3] = ["today", "days7", "month1"];

/// Everything the items can show. `usage` follows `PERIODS` and is read only when an item needs it.
#[derive(Debug, Clone, Default)]
pub struct Data {
    pub readings: Vec<Reading>,
    pub usage: Option<[Usage; 3]>,
    /// OpenCode alone, for items whose `provider` is `opencode`; `None` while its source is off.
    pub opencode: Option<[Usage; 3]>,
    pub scan_ms: Option<i64>,
    /// Each Codex account's short name, by digest, for items of an account.
    pub accounts: std::collections::HashMap<String, String>,
}

pub fn needs_usage(cfg: &TraySettings) -> bool {
    cfg.items.iter().any(|i| matches!(i.kind.as_str(), "tokens" | "cost" | "sessions" | "cache"))
}

pub struct Format<'a, Tz: TimeZone> {
    pub cfg: &'a TraySettings,
    pub tr: bool,
    /// The app's own used/remaining choice, for `value: "app"`.
    pub app_remaining: bool,
    pub warn_at: f64,
    pub currency: &'a str,
    pub fx: f64,
    pub now_ms: i64,
    pub tz: &'a Tz,
}

impl<Tz: TimeZone> Format<'_, Tz> {
    fn remaining(&self) -> bool {
        match self.cfg.value.as_str() {
            "used" => false,
            "remaining" => true,
            _ => self.app_remaining,
        }
    }

    /// The menu bar text; empty when nothing chosen has a value.
    pub fn title(&self, data: &Data) -> String {
        let parts: Vec<String> = self.cfg.items.iter().filter_map(|i| self.part(i, data)).collect();
        if parts.is_empty() {
            return String::new();
        }
        format!("{}{}{}", self.cfg.prefix, parts.join(&self.cfg.separator), self.cfg.suffix)
    }

    /// One item's piece of the menu bar text.
    pub fn part(&self, item: &TrayItem, data: &Data) -> Option<String> {
        let body = match item.kind.as_str() {
            "limit" => {
                let r = self.reading(item, data)?;
                let mut v = self.value(r, &self.cfg.style);
                if let Some(at) = self.reset_text(r) {
                    v = format!("{v} ↻{at}");
                }
                v
            }
            "forecast" => {
                let r = self.reading(item, data)?;
                match (r.fills_at_ms, r.at_reset) {
                    (Some(at), _) => format!("⇡{}", self.clock(at / 1000)?),
                    (None, Some(p)) => format!("→{}", self.pct(&shown_value(p, self.remaining()).to_string())),
                    _ => return None,
                }
            }
            "scan" => self.clock(data.scan_ms? / 1000)?,
            _ => self.usage(item, data)?,
        };
        let label = self.label(item, data);
        Some(if label.is_empty() { body } else { format!("{label} {body}") })
    }

    fn reading<'d>(&self, item: &TrayItem, data: &'d Data) -> Option<&'d Reading> {
        find(&data.readings, item)
    }

    fn usage(&self, item: &TrayItem, data: &Data) -> Option<String> {
        let tr = self.tr;
        let set = if item.provider == "opencode" { data.opencode } else { data.usage };
        let u = set?[PERIODS.iter().position(|p| *p == item.period)?];
        Some(match item.kind.as_str() {
            "tokens" => self.tokens(u.tokens),
            "cost" => self.money(u.cost_usd),
            "sessions" if tr => format!("{} oturum", u.sessions),
            "sessions" => format!("{} {}", u.sessions, if u.sessions == 1 { "session" } else { "sessions" }),
            "cache" if u.prompt == 0 => return None,
            "cache" => {
                let p = self.pct(&((u.cache_read as f64 / u.prompt as f64 * 100.0).round() as u32).to_string());
                if tr { format!("önbellek {p}") } else { format!("cache {p}") }
            }
            _ => return None,
        })
    }

    fn label(&self, item: &TrayItem, data: &Data) -> String {
        let tr = self.tr;
        let short = self.cfg.label == "short";
        let account = |who: String| match item.account.as_str() {
            "" => who,
            id => format!("{who} {}", data.accounts.get(id).map_or("?", String::as_str)),
        };
        match (self.cfg.label.as_str(), item.kind.as_str()) {
            ("none", _) => String::new(),
            (_, "scan") => if tr { "Tarama" } else { "Scan" }.into(),
            (_, "tokens" | "cost" | "sessions" | "cache") if item.provider == "opencode" => {
                let period = period_label(&item.period, tr, short);
                if short { format!("OC {period}") } else { format!("OpenCode {}", period.to_lowercase()) }
            }
            (_, "tokens" | "cost" | "sessions" | "cache") => period_label(&item.period, tr, short).into(),
            ("name", _) => {
                let window = match (item.window.as_str(), tr) {
                    ("five_hour", true) => "5 sa",
                    ("five_hour", false) => "5h",
                    (_, true) => "hafta",
                    (_, false) => "week",
                };
                format!("{} {window}", account(name(provider(&item.provider), &item.pool, tr)))
            }
            _ => {
                let window = match (item.window.as_str(), tr) {
                    ("five_hour", true) => "5s",
                    ("five_hour", false) => "5h",
                    (_, true) => "H",
                    (_, false) => "W",
                };
                let who = match (item.provider.as_str(), item.pool.as_str()) {
                    ("anthropic", _) => "Cl",
                    ("openai", _) => "Cx",
                    (_, "gemini") => "Gm",
                    _ => "AG",
                };
                format!("{} {window}", account(who.into()))
            }
        }
    }

    fn warned(&self, r: &Reading) -> bool {
        self.cfg.warn_mark && r.used >= self.warn_at
    }

    fn pct(&self, v: &str) -> String {
        if self.tr { format!("%{v}") } else { format!("{v}%") }
    }

    fn value(&self, r: &Reading, style: &str) -> String {
        let remaining = self.remaining();
        let v = shown_value(r.used, remaining);
        // the last reading: use since then raises it, a reset the provider grants lowers it
        let sign = if r.behind { "~" } else { "" };
        let warn = if self.warned(r) { "⚠︎" } else { "" };
        let pct = format!("{sign}{}", self.pct(&v.to_string()));
        match style {
            "bar" => format!("{warn}{}", bar(v)),
            "bar_percent" => format!("{warn}{} {pct}", bar(v)),
            "dot" => format!("{warn}{}", dot(v)),
            _ => format!("{warn}{pct}"),
        }
    }

    fn reset_text(&self, r: &Reading) -> Option<String> {
        match self.cfg.reset.as_str() {
            "countdown" => self.countdown(r),
            "time" => self.time(r),
            _ => None,
        }
    }

    fn left_secs(&self, r: &Reading) -> Option<i64> {
        r.resets_at.map(|at| at - self.now_ms / 1000).filter(|s| *s > 0)
    }

    fn countdown(&self, r: &Reading) -> Option<String> {
        let s = self.left_secs(r)?;
        let (d, h, m) = (s / 86_400, s % 86_400 / 3600, ((s % 3600 + 59) / 60).max(1));
        let (dd, hh, mm) = if self.tr { ("g", "sa", "dk") } else { ("d", "h", "m") };
        Some(if d > 0 {
            format!("{d}{dd}{h}{hh}")
        } else if h > 0 {
            format!("{h}{hh}{m}{mm}")
        } else {
            format!("{m}{mm}")
        })
    }

    fn time(&self, r: &Reading) -> Option<String> {
        self.left_secs(r)?;
        self.clock(r.resets_at?)
    }

    /// "18:40" within a day of now, "Sun 08:53" further away.
    fn clock(&self, at_secs: i64) -> Option<String> {
        let at: DateTime<Tz> = self.tz.timestamp_opt(at_secs, 0).single()?;
        let hm = format!("{:02}:{:02}", at.hour(), at.minute());
        if (at_secs - self.now_ms / 1000).abs() < 86_400 {
            return Some(hm);
        }
        const TR: [&str; 7] = ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"];
        const EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let day = at.weekday().num_days_from_monday() as usize;
        Some(format!("{} {hm}", if self.tr { TR[day] } else { EN[day] }))
    }

    fn tokens(&self, n: u64) -> String {
        let (units, dec): (&[&str; 3], char) = if self.tr { (&[" B", " Mn", " Mr"], ',') } else { (&["K", "M", "B"], '.') };
        let n = n as f64;
        let (v, unit) = if n >= 1e9 {
            (n / 1e9, units[2])
        } else if n >= 1e6 {
            (n / 1e6, units[1])
        } else if n >= 1e3 {
            (n / 1e3, units[0])
        } else {
            return format!("{n}");
        };
        let s = if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}").trim_end_matches(".0").to_owned() };
        format!("{}{unit}", s.replace('.', &dec.to_string()))
    }

    fn money(&self, usd: f64) -> String {
        let v = usd * self.fx;
        let symbol = match self.currency {
            "TRY" => "₺",
            "EUR" => "€",
            "GBP" => "£",
            _ => "$",
        };
        let text = if v >= 1000.0 { group(v.round() as u64, if self.tr { '.' } else { ',' }) } else { format!("{v:.2}") };
        let text = if self.tr && v < 1000.0 { text.replace('.', ",") } else { text };
        format!("{symbol}{text}")
    }
}

/// Five cells, one per 20 %.
fn bar(v: u32) -> String {
    let full = ((f64::from(v) / 20.0).round() as usize).min(5);
    "▰".repeat(full) + &"▱".repeat(5 - full)
}

fn dot(v: u32) -> &'static str {
    ["○", "◔", "◑", "◕", "●"][((v + 12) / 25).min(4) as usize]
}

fn group(n: u64, sep: char) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

fn period_label(period: &str, tr: bool, short: bool) -> &'static str {
    match (period, tr, short) {
        ("today", true, _) => "Bugün",
        ("today", false, _) => "Today",
        ("days7", true, true) => "7g",
        ("days7", true, false) => "7 gün",
        ("days7", false, true) => "7d",
        ("days7", false, false) => "7 days",
        (_, true, true) => "30g",
        (_, true, false) => "30 gün",
        (_, false, true) => "30d",
        (_, false, false) => "30 days",
    }
}

fn provider(s: &str) -> Provider {
    Provider::parse(s).unwrap_or(Provider::Anthropic)
}

fn name(p: Provider, pool: &str, tr: bool) -> String {
    match (p, pool) {
        (Provider::Anthropic, _) => "Claude".into(),
        (Provider::OpenAI, _) => "Codex".into(),
        (Provider::Google, "gemini") => "Antigravity Gemini".into(),
        (Provider::Google, _) => if tr { "Antigravity Claude ve GPT" } else { "Antigravity Claude and GPT" }.into(),
    }
}

/// The item's reading; Antigravity's other pool is any pool that is not Gemini. An item without an
/// account follows the account in use.
fn find<'a>(readings: &'a [Reading], item: &TrayItem) -> Option<&'a Reading> {
    let p = Provider::parse(&item.provider)?;
    readings
        .iter()
        .filter(|r| r.provider == p && r.window == item.window)
        .filter(|r| if item.account.is_empty() { r.active } else { r.account.as_deref() == Some(item.account.as_str()) })
        .filter(|r| p != Provider::Google || (item.pool == "gemini") == (r.pool == "gemini"))
        .max_by(|a, b| a.used.total_cmp(&b.used))
}

pub fn shown_value(used: f64, remaining: bool) -> u32 {
    let v = if remaining { 100.0 - used } else { used };
    v.clamp(0.0, 100.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const NOW: i64 = 1_793_000_000_000; // a Monday, 07:33 UTC

    fn read(p: Provider, pool: &str, window: &str, used: f64, in_secs: i64) -> Reading {
        Reading { provider: p, pool: pool.into(), window: window.into(), used, behind: false, resets_at: Some(NOW / 1000 + in_secs), fills_at_ms: None, at_reset: None, account: None, active: true }
    }

    fn data() -> Data {
        let readings = vec![
            read(Provider::Anthropic, "", "five_hour", 42.4, 2 * 3600 + 14 * 60),
            read(Provider::Anthropic, "", "seven_day", 86.0, 3 * 86_400 + 5 * 3600),
            read(Provider::OpenAI, "codex", "five_hour", 12.0, 600),
            read(Provider::Google, "gemini", "five_hour", 18.0, 3600),
            read(Provider::Google, "3p", "five_hour", 7.0, 3600),
        ];
        Data { readings, ..Data::default() }
    }

    fn limit(provider: &str, window: &str, pool: &str) -> TrayItem {
        TrayItem { kind: "limit".into(), provider: provider.into(), window: window.into(), pool: pool.into(), ..TrayItem::default() }
    }

    fn usage(kind: &str, period: &str) -> TrayItem {
        TrayItem { kind: kind.into(), period: period.into(), ..TrayItem::default() }
    }

    fn cfg(items: Vec<TrayItem>, f: impl FnOnce(&mut TraySettings)) -> TraySettings {
        let mut c = TraySettings { items, ..TraySettings::default() };
        f(&mut c);
        c
    }

    fn fmt<'a>(c: &'a TraySettings, tr: bool, tz: &'a FixedOffset) -> Format<'a, FixedOffset> {
        Format { cfg: c, tr, app_remaining: false, warn_at: 80.0, currency: "USD", fx: 1.0, now_ms: NOW, tz }
    }

    #[test]
    fn short_labels_values_and_the_warning_mark_in_both_languages() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let c = cfg(vec![limit("anthropic", "five_hour", ""), limit("anthropic", "seven_day", "")], |_| {});
        assert_eq!(fmt(&c, true, &tz).title(&data()), "Cl 5s %42 · Cl H ⚠︎%86");
        assert_eq!(fmt(&c, false, &tz).title(&data()), "Cl 5h 42% · Cl W ⚠︎86%");
        let c = cfg(c.items.clone(), |c| c.warn_mark = false);
        assert_eq!(fmt(&c, true, &tz).title(&data()), "Cl 5s %42 · Cl H %86");
    }

    #[test]
    fn names_remaining_separator_prefix_and_a_reading_taken_before_newer_use() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let c = cfg(vec![limit("anthropic", "five_hour", ""), limit("google", "five_hour", "gemini")], |c| {
            c.label = "name".into();
            c.value = "remaining".into();
            c.separator = " | ".into();
            c.prefix = "[".into();
            c.suffix = "]".into();
        });
        let mut d = data();
        d.readings[0].behind = true;
        assert_eq!(fmt(&c, true, &tz).title(&d), "[Claude 5 sa ~%58 | Antigravity Gemini 5 sa %82]");
        assert_eq!(fmt(&c, true, &tz).title(&Data::default()), "", "no brackets around nothing");
    }

    #[test]
    fn countdown_and_clock_reset_times() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let mut c = cfg(vec![limit("anthropic", "five_hour", ""), limit("anthropic", "seven_day", ""), limit("openai", "five_hour", "")], |c| {
            c.label = "none".into();
            c.reset = "countdown".into();
        });
        assert_eq!(fmt(&c, true, &tz).title(&data()), "%42 ↻2sa14dk · ⚠︎%86 ↻3g5sa · %12 ↻10dk");
        c.reset = "time".into();
        // 07:33 UTC is 10:33 at +03:00; the weekly reset is Thursday 15:33
        assert_eq!(fmt(&c, false, &tz).title(&data()), "42% ↻12:47 · ⚠︎86% ↻Thu 15:33 · 12% ↻10:43");
    }

    #[test]
    fn order_and_missing_readings() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let c = cfg(vec![limit("google", "five_hour", "3p"), limit("openai", "seven_day", ""), limit("openai", "five_hour", "")], |_| {});
        assert_eq!(fmt(&c, false, &tz).title(&data()), "AG 5h 7% · Cx 5h 12%");
        assert_eq!(fmt(&cfg(vec![], |_| {}), false, &tz).title(&data()), "");
    }

    #[test]
    fn another_codex_account_by_its_name_beside_the_account_in_use() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let (work, home) = ("3f9c1a7e52b0d846", "b71e04c9a3d25f18");
        let mut d = data();
        d.readings[2].account = Some(home.into());
        d.readings.push(Reading { used: 100.0, account: Some(work.into()), active: false, ..d.readings[2].clone() });
        d.accounts = [(work.to_owned(), "İş".to_owned()), (home.to_owned(), "2".to_owned())].into();
        let of = |id: &str| TrayItem { account: id.into(), ..limit("openai", "five_hour", "") };
        let c = cfg(vec![of(""), of(work), of("0123456789abcdef")], |_| {});
        assert_eq!(fmt(&c, false, &tz).title(&d), "Cx 5h 12% · Cx İş 5h ⚠︎100%", "an account without a reading shows nothing");
        let c = cfg(vec![of(work)], |c| c.label = "name".into());
        assert_eq!(fmt(&c, true, &tz).title(&d), "Codex İş 5 sa ⚠︎%100");

        // Claude accounts the same way
        let personal = "5d2e8a14c07b9f63";
        d.readings.push(Reading { account: Some(personal.into()), active: false, used: 64.0, ..d.readings[0].clone() });
        d.accounts.insert(personal.into(), "Kişisel".into());
        let claude = |id: &str| TrayItem { account: id.into(), ..limit("anthropic", "five_hour", "") };
        let c = cfg(vec![claude(""), claude(personal)], |_| {});
        assert_eq!(fmt(&c, false, &tz).title(&d), "Cl 5h 42% · Cl Kişisel 5h 64%");
    }

    #[test]
    fn bars_and_circles() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let mut c = cfg(vec![limit("anthropic", "five_hour", ""), limit("anthropic", "seven_day", "")], |c| c.style = "bar".into());
        assert_eq!(fmt(&c, true, &tz).title(&data()), "Cl 5s ▰▰▱▱▱ · Cl H ⚠︎▰▰▰▰▱");
        c.style = "bar_percent".into();
        assert_eq!(fmt(&c, true, &tz).title(&data()), "Cl 5s ▰▰▱▱▱ %42 · Cl H ⚠︎▰▰▰▰▱ %86");
        c.style = "dot".into();
        c.value = "remaining".into();
        assert_eq!(fmt(&c, true, &tz).title(&data()), "Cl 5s ◑ · Cl H ⚠︎◔", "the circle shows what is left");
    }

    #[test]
    fn usage_over_three_periods_in_the_chosen_currency() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let items = vec![usage("tokens", "today"), usage("cost", "today"), usage("sessions", "days7"), usage("cache", "month1")];
        let c = cfg(items, |_| {});
        let today = Usage { tokens: 12_430_000, cost_usd: 3.2, sessions: 2, prompt: 0, cache_read: 0 };
        let week = Usage { sessions: 1, ..today };
        let month = Usage { prompt: 1000, cache_read: 870, ..today };
        let d = Data { usage: Some([today, week, month]), ..Data::default() };
        assert_eq!(fmt(&c, false, &tz).title(&d), "Today 12.4M · Today $3.20 · 7d 1 session · 30d cache 87%");
        let mut f = fmt(&c, true, &tz);
        f.currency = "TRY";
        f.fx = 41.5;
        assert_eq!(f.title(&d), "Bugün 12,4 Mn · Bugün ₺132,80 · 7g 1 oturum · 30g önbellek %87");
        let d = Data { usage: Some([Usage { tokens: 950, cost_usd: 30.0, ..Usage::default() }; 3]), ..Data::default() };
        assert_eq!(f.title(&d), "Bugün 950 · Bugün ₺1.245 · 7g 0 oturum", "no hit rate without prompts");
        assert_eq!(f.title(&Data::default()), "", "usage is shown only when it was read");
    }

    #[test]
    fn opencode_usage_on_its_own() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let oc = |kind: &str, period: &str| TrayItem { provider: "opencode".into(), ..usage(kind, period) };
        let mut c = cfg(vec![usage("tokens", "today"), oc("tokens", "today"), oc("cost", "days7")], |_| {});
        let all = Usage { tokens: 5_000_000, cost_usd: 9.0, ..Usage::default() };
        let mine = Usage { tokens: 1_200_000, cost_usd: 2.5, ..Usage::default() };
        let d = Data { usage: Some([all; 3]), opencode: Some([mine; 3]), ..Data::default() };
        assert_eq!(fmt(&c, false, &tz).title(&d), "Today 5M · OC Today 1.2M · OC 7d $2.50");
        c.label = "name".into();
        assert_eq!(fmt(&c, true, &tz).title(&d), "Bugün 5 Mn · OpenCode bugün 1,2 Mn · OpenCode 7 gün $2,50");
        let off = Data { opencode: None, ..d };
        assert_eq!(fmt(&c, false, &tz).title(&off), "Today 5M", "nothing while OpenCode is off");
    }

    #[test]
    fn forecasts_and_the_last_scan() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let items = vec![
            TrayItem { kind: "forecast".into(), ..limit("anthropic", "five_hour", "") },
            TrayItem { kind: "forecast".into(), ..limit("anthropic", "seven_day", "") },
            TrayItem { kind: "forecast".into(), ..limit("openai", "five_hour", "") },
            usage("scan", ""),
        ];
        let c = cfg(items, |_| {});
        let mut d = data();
        d.readings[0].fills_at_ms = Some(NOW + 2 * 3_600_000);
        d.readings[1].at_reset = Some(64.0);
        d.scan_ms = Some(NOW - 600_000);
        assert_eq!(fmt(&c, true, &tz).title(&d), "Cl 5s ⇡12:33 · Cl H →%64 · Tarama 10:23", "Codex has no forecast yet");
    }

}
