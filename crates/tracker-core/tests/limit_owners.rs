use chrono::TimeZone;
use std::path::Path;
use tracker_core::accounts::{resolve, resolve_claude, root_key};
use tracker_core::analytics::limits_view;
use tracker_core::model::{claude_account, Accuracy, LimitSnapshot, Provider, Tool};
use tracker_core::pricing::PriceBook;
use tracker_core::store::Store;

const M: i64 = 60_000;
const H: i64 = 60 * M;
const DAY: i64 = 24 * H;

fn ms(d: u32, h: u32, mi: u32) -> i64 {
    chrono::Utc.with_ymd_and_hms(2026, 10, d, h, mi, 0).unwrap().timestamp_millis()
}

fn snap(ts: i64, window: &str, used: f64, resets_s: Option<i64>, plan: Option<&str>, source: &str, account: Option<String>) -> LimitSnapshot {
    LimitSnapshot {
        ts_ms: ts,
        provider: Provider::Anthropic,
        tool: if source == "claude_plan_history" { Tool::ClaudeDesktop } else { Tool::ClaudeCode },
        account,
        limit_id: String::new(),
        window: window.into(),
        used_pct: Some(used),
        resets_at: resets_s,
        status: None,
        plan: plan.map(Into::into),
        source: source.into(),
        accuracy: Accuracy::Captured,
    }
}

fn put(store: &mut Store, s: &[LimitSnapshot]) {
    let mut tx = store.transaction().unwrap();
    tx.insert_limits(s).unwrap();
    tx.commit().unwrap();
}

#[test]
fn a_retired_window_comes_back_while_another_source_is_newest() {
    let mut seen = Vec::new();
    for (label, newer) in [("status line", "claude_statusline"), ("desktop history", "claude_plan_history")] {
        let mut store = Store::open_in_memory().unwrap();
        let now = ms(2, 12, 0);
        let (five, week) = (Some((now + H) / 1000), Some((now + 3 * DAY) / 1000));
        let usage = |at, window, reset| snap(at, window, 30.0, reset, Some("max"), "claude_code_usage", None);
        put(&mut store, &[usage(now - 3 * H, "five_hour", five), usage(now - 3 * H, "seven_day", week), usage(now - 3 * H, "seven_day_opus", Some((now - H) / 1000))]);
        put(&mut store, &[usage(now - 10 * M, "five_hour", five), usage(now - 10 * M, "seven_day", week)]);
        let book = PriceBook::default_book();
        let opus = |s: &Store| limits_view(s, &book, now, &[]).unwrap().into_iter().find(|v| v.window == "seven_day_opus").map(|v| v.state);
        assert_eq!(opus(&store), None, "{label}: gone after the newer full read");
        let timed = newer != "claude_plan_history";
        put(&mut store, &[snap(now - 5 * M, "five_hour", 31.0, five.filter(|_| timed), None, newer, None), snap(now - 5 * M, "seven_day", 31.0, week.filter(|_| timed), None, newer, None)]);
        let back = opus(&store);
        seen.push(back);
    }
    assert_eq!(seen, [None, None]);
}

#[test]
fn the_use_kept_with_a_reset_is_not_the_newest_one() {
    let mut out = Vec::new();
    // the status line's reset a second later or equal, stored before or after the newer reading
    for (sl_off, sl_first) in [(1, true), (0, true), (0, false)] {
        let mut store = Store::open_in_memory().unwrap();
        let now = ms(2, 12, 20);
        let r = ms(2, 15, 0) / 1000;
        let sl = snap(ms(2, 10, 0), "five_hour", 15.0, Some(r + sl_off), None, "claude_statusline", None);
        let us = snap(ms(2, 12, 0), "five_hour", 80.0, Some(r), Some("max"), "claude_code_usage", None);
        if sl_first {
            put(&mut store, &[sl]);
            put(&mut store, &[us]);
        } else {
            put(&mut store, &[us]);
            put(&mut store, &[sl]);
        }
        // the window started again early: the desktop history reads 10 % with no reset
        put(&mut store, &[snap(ms(2, 12, 16), "five_hour", 10.0, None, None, "claude_plan_history", None)]);
        let v = limits_view(&store, &PriceBook::default_book(), now, &[]).unwrap().into_iter().find(|v| v.window == "five_hour").unwrap();
        out.push(v.resets_at);
    }
    assert_eq!(out, [None, None, None], "use fell from 80 % to 10 %: a new window");
}

fn org(n: u8) -> String {
    claude_account(&format!("{n:08x}-0000-4000-8000-{n:012x}"))
}

fn sign_in(store: &mut Store, folder: &str, account: &str, from: i64, to: i64) {
    let mut t = from;
    while t <= to {
        store.note_signin(folder, Some(account), t).unwrap();
        t += 30_000;
    }
}

fn owner(store: &Store, ts: i64, source: &str, window: &str) -> Option<String> {
    store.conn().query_row("SELECT owner FROM limit_snapshot WHERE ts_ms = ?1 AND source = ?2 AND window = ?3", rusqlite::params![ts, source, window], |r| r.get(0)).unwrap()
}

/// Two Claude accounts the app knows: A in one folder, B in another.
fn two_accounts(base: i64) -> (Store, String, String) {
    let mut store = Store::open_in_memory().unwrap();
    let (a, b) = (org(1), org(2));
    sign_in(&mut store, &root_key(Path::new("/home/u/.claude")), &a, base - 4 * H, base + 3 * H);
    sign_in(&mut store, &root_key(Path::new("/home/u/claude-work")), &b, base - 4 * H, base + 3 * H);
    (store, a, b)
}

#[test]
fn a_sibling_window_without_an_owner_gets_its_siblings_owner() {
    let base = ms(27, 0, 0);
    let (mut store, a, _) = two_accounts(base);
    let t = base - 3 * H;
    let week = base / 1000 + 3 * 86_400;
    put(&mut store, &[snap(t, "five_hour", 20.0, Some(t / 1000 + 3600), Some("max"), "claude_code_usage", Some(a.clone())), snap(t, "seven_day", 20.0, Some(week), Some("max"), "claude_code_usage", Some(a.clone()))]);
    resolve_claude(&mut store).unwrap();
    assert_eq!((owner(&store, t, "claude_code_usage", "five_hour"), owner(&store, t, "claude_code_usage", "seven_day")), (Some(a.clone()), Some(a.clone())));
    // what an import of the same moment would leave: one window's row without an owner
    store.conn().execute("UPDATE limit_snapshot SET owner = NULL WHERE ts_ms = ?1 AND window = 'seven_day'", [t]).unwrap();
    resolve_claude(&mut store).unwrap();
    let got = (owner(&store, t, "claude_code_usage", "five_hour"), owner(&store, t, "claude_code_usage", "seven_day"));
    assert_eq!(got, (Some(a.clone()), Some(a)));
}

#[test]
fn a_reset_far_past_3000_in_an_old_row() {
    let mut store = Store::open_in_memory().unwrap();
    let t = ms(2, 10, 0);
    put(&mut store, &[snap(t, "seven_day", 20.0, Some(t / 1000 + 86_400), Some("max"), "claude_code_usage", None)]);
    store.conn().execute("UPDATE limit_snapshot SET resets_at = ?1", [i64::MAX]).unwrap();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| resolve(&mut store)));
    assert!(matches!(r, Ok(Ok(()))));
}

#[test]
fn a_status_line_shown_after_its_five_hour_reset_with_two_accounts() {
    let mut seen = Vec::new();
    for old_week in [22.0, 10.0] {
        seen.push(m14(old_week));
    }
    assert_eq!(seen, [true, true]);
}

fn m14(old_week: f64) -> bool {
    let base = ms(27, 0, 0);
    let (mut store, a, _) = two_accounts(base);
    let week = base / 1000 + 3 * 86_400;
    let (f0, f1) = (base / 1000 + 3600, base / 1000 + 6 * 3600);
    let status = |ts: i64, five: i64, fp: f64, wp: f64| [snap(ts, "five_hour", fp, Some(five), None, "claude_statusline", None), snap(ts, "seven_day", wp, Some(week), None, "claude_statusline", None)];
    // the app's own read names A in this week
    put(&mut store, &[snap(base - 3 * H, "seven_day", 20.0, Some(week), Some("max"), "claude_code_usage", Some(a.clone()))]);
    for (ts, five, fp, wp) in [
        (base - 3 * H + M, f0, 30.0, 20.0),
        (base - 2 * H, f0, 40.0, 22.0),
        (base + H + 10 * M, f1, 5.0, 25.0),
        (base + 2 * H, f1, 12.0, 27.0),
        // an idle terminal shows the window that ended at f0 again, with its old weekly figure
        (base + 2 * H + 5 * M, f0, 40.0, old_week),
        (base + 2 * H + 10 * M, f1, 13.0, 27.0),
    ] {
        put(&mut store, &status(ts, five, fp, wp));
    }
    resolve_claude(&mut store).unwrap();
    let got = owner(&store, base + 2 * H, "claude_statusline", "seven_day");
    got == Some(a)
}
