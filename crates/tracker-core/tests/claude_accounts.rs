//! Claude accounts: which evidence ties a request or a reading to an account, and what stays
//! unknown. The scenarios follow the account-switch tests run on a real two-account setup.

mod common;

use common::use_test_round;
use std::path::{Path, PathBuf};
use tracker_core::accounts::{accounts, in_use, resolve, resolve_claude, root_key};
use tracker_core::analytics::{account_key, limits_view, Filter};
use tracker_core::model::{claude_account, Accuracy, LimitSnapshot, Provider, SessionAccount, Tokens, Tool, UsageEvent};
use tracker_core::pricing::PriceBook;
use tracker_core::store::{Store, SIGNIN_GAP_MS};

/// 2026-10-27T00:00:00Z
const BASE: i64 = 1_793_059_200_000;
const M: i64 = 60_000;
const H: i64 = 60 * M;

fn org(n: u8) -> String {
    claude_account(&format!("{n:08x}-0000-4000-8000-{n:012x}"))
}

/// Spelt as the app stores folders, which on Windows differs from the literal.
fn root() -> String {
    root_key(Path::new("/home/u/.claude"))
}

/// A second Claude config folder, as one folder per account is set up.
fn other_root() -> String {
    root_key(Path::new("/home/u/claude-work"))
}

struct Claude {
    store: Store,
}

impl Claude {
    fn new() -> Claude {
        Claude { store: Store::open_in_memory().unwrap() }
    }

    fn event(key: String, ts: i64, session: &str, rid: &str, client: &str, accuracy: Accuracy, creator: Option<String>) -> UsageEvent {
        UsageEvent {
            key,
            ts_ms: ts,
            tool: Tool::ClaudeCode,
            client: Some(client.into()),
            model: "claude-sonnet-5-5".into(),
            project_path: Some("/work/app".into()),
            session_id: Some(session.into()),
            tokens: Tokens { input: 10, output: 5, ..Tokens::default() },
            request_input: 10,
            web_search_requests: 0,
            speed: None,
            service_tier: None,
            inference_geo: None,
            request_id: Some(rid.into()),
            accuracy,
            source: if accuracy == Accuracy::Exact { "claude_code_jsonl" } else { "claude_otel" }.into(),
            branch: None,
            agent: None,
            thread_id: None,
            limit_reset: None,
            creator,
        }
    }

    /// One request as Claude Code logs it, in a session whose log lies in `root()`.
    fn ask(&mut self, session: &str, client: &str, ts: i64, rid: &str) -> &mut Claude {
        self.ask_in(&root(), session, client, ts, rid)
    }

    fn ask_in(&mut self, folder: &str, session: &str, client: &str, ts: i64, rid: &str) -> &mut Claude {
        let log = PathBuf::from(folder).join("projects").join("-work-app").join(format!("{session}.jsonl"));
        let mut tx = self.store.transaction().unwrap();
        tx.upsert_events(&[Self::event(format!("cc:{rid}"), ts, session, rid, client, Accuracy::Exact, None)]).unwrap();
        tx.set_checkpoint(&log.to_string_lossy(), "id", "claude_code_jsonl", 1, ts, 1, &serde_json::Value::Null, 0, None).unwrap();
        tx.commit().unwrap();
        self
    }

    /// The telemetry event of a request, naming the organisation it ran on.
    fn told(&mut self, session: &str, ts: i64, rid: &str, account: Option<String>) -> &mut Claude {
        let mut tx = self.store.transaction().unwrap();
        tx.upsert_events(&[Self::event(format!("otel:{rid}"), ts, session, rid, "otel:repl_main_thread", Accuracy::Captured, account)]).unwrap();
        tx.commit().unwrap();
        self
    }

    /// The app looking at the sign-in every 30 seconds from `from` to `to`.
    fn signed_in(&mut self, account: Option<&str>, from: i64, to: i64) -> &mut Claude {
        self.signed_in_at(&root(), account, from, to)
    }

    fn signed_in_at(&mut self, folder: &str, account: Option<&str>, from: i64, to: i64) -> &mut Claude {
        let mut t = from;
        while t <= to {
            self.store.note_signin(folder, account, t).unwrap();
            t += 30_000;
        }
        self
    }

    fn desktop(&mut self, session: &str, account: &str) -> &mut Claude {
        let mut tx = self.store.transaction().unwrap();
        tx.insert_session_accounts(&[SessionAccount { tool: Tool::ClaudeCode, session_id: session.into(), account: account.into() }]).unwrap();
        tx.commit().unwrap();
        self
    }

    /// A reading of the five-hour and weekly windows; `week` is when the weekly one resets.
    fn read(&mut self, ts: i64, source: &str, account: Option<String>, week: i64, used: f64) -> &mut Claude {
        self.read_on("max", ts, source, account, week, used)
    }

    /// [`Claude::read`] on another plan.
    fn read_on(&mut self, plan: &str, ts: i64, source: &str, account: Option<String>, week: i64, used: f64) -> &mut Claude {
        let snap = |window: &str, reset: Option<i64>| LimitSnapshot {
            ts_ms: ts,
            provider: Provider::Anthropic,
            tool: if source == "claude_plan_history" { Tool::ClaudeDesktop } else { Tool::ClaudeCode },
            account: account.clone(),
            limit_id: String::new(),
            window: window.into(),
            used_pct: Some(used),
            resets_at: reset,
            status: None,
            plan: Some(plan.into()),
            source: source.into(),
            accuracy: Accuracy::Captured,
        };
        let five = (source != "claude_plan_history").then_some(ts / 1000 + 3600);
        let weekly = (source != "claude_plan_history").then_some(week);
        let mut tx = self.store.transaction().unwrap();
        tx.insert_limits(&[snap("five_hour", five), snap("seven_day", weekly)]).unwrap();
        tx.commit().unwrap();
        self
    }

    fn resolve(&mut self) -> &mut Claude {
        resolve_claude(&mut self.store).unwrap();
        self
    }

    fn owner(&self, rid: &str) -> Option<String> {
        self.store.conn().query_row("SELECT owner FROM usage_event WHERE key = ?1", [format!("cc:{rid}")], |r| r.get(0)).unwrap()
    }

    fn reading_owner(&self, ts: i64, source: &str) -> Option<String> {
        self.store
            .conn()
            .query_row("SELECT owner FROM limit_snapshot WHERE ts_ms = ?1 AND source = ?2 AND window = 'seven_day'", rusqlite::params![ts, source], |r| r.get(0))
            .unwrap()
    }

    /// A status-line reading, which names no account: (five-hour reset, weekly reset) in seconds.
    fn status(&mut self, ts: i64, five_reset: i64, week_reset: i64, five_pct: f64, week_pct: f64) -> &mut Claude {
        let snap = |window: &str, reset: i64, pct: f64| LimitSnapshot {
            ts_ms: ts,
            provider: Provider::Anthropic,
            tool: Tool::ClaudeCode,
            account: None,
            limit_id: String::new(),
            window: window.into(),
            used_pct: Some(pct),
            resets_at: Some(reset),
            status: None,
            plan: None,
            source: "claude_statusline".into(),
            accuracy: Accuracy::Captured,
        };
        let mut tx = self.store.transaction().unwrap();
        tx.insert_limits(&[snap("five_hour", five_reset, five_pct), snap("seven_day", week_reset, week_pct)]).unwrap();
        tx.commit().unwrap();
        self
    }

    fn claude_accounts(&self) -> Vec<Option<String>> {
        accounts(&self.store).unwrap().into_iter().filter(|a| a.provider == Provider::Anthropic).map(|a| a.id).collect()
    }
}

#[test]
fn one_account_owns_everything_and_shows_no_accounts() {
    let mut c = Claude::new();
    let a = org(1);
    c.signed_in(Some(&a), BASE, BASE + H).ask("s1", "cli", BASE - 3 * H, "r1").ask("s2", "claude-desktop", BASE + 10 * M, "r2");
    c.read(BASE + 20 * M, "claude_plan_history", Some(a.clone()), 0, 12.0).resolve();
    assert_eq!((c.owner("r1"), c.owner("r2")), (Some(a.clone()), Some(a.clone())), "nothing to tell apart: the only account");
    assert!(accounts(&c.store).unwrap().is_empty());
}

#[test]
fn telemetry_names_each_request_even_across_a_login_in_the_session() {
    // the real test: /login to B inside a running session; every later event named B
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + 3 * M).signed_in(Some(&b), BASE + 3 * M + 30_000, BASE + 10 * M);
    for (ts, rid, who) in [(BASE + M, "r1", &a), (BASE + 2 * M, "r2", &a), (BASE + 5 * M, "r3", &b)] {
        c.ask("s1", "cli", ts, rid).told("s1", ts, rid, Some(who.clone()));
    }
    // a helper request only telemetry saw
    c.told("s1", BASE + 6 * M, "r4", Some(b.clone())).resolve();
    assert_eq!([c.owner("r1"), c.owner("r2"), c.owner("r3")], [Some(a.clone()), Some(a.clone()), Some(b.clone())]);
    let helper: Option<String> = c.store.conn().query_row("SELECT owner FROM usage_event WHERE key = 'otel:r4'", [], |r| r.get(0)).unwrap();
    assert_eq!(helper, Some(b));
    // a request that telemetry and the log both hold counts once
    let counts: Vec<u64> = accounts(&c.store).unwrap().iter().map(|x| x.requests).collect();
    assert_eq!(counts, [2, 2]);
}

#[test]
fn without_telemetry_a_folder_signed_in_to_two_accounts_names_none() {
    // the real behaviour: a running Claude Code keeps the sign-in it loaded until it renews it, and
    // a session's log begins only at its first prompt, so after a /login in the same folder no
    // request of a session is known for certain
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + H).signed_in(Some(&b), BASE + H + 30_000, BASE + 3 * H);
    c.ask("s1", "cli", BASE + 10 * M, "r1").ask("s1", "cli", BASE + H + 10 * M, "r2");
    // opened under A, idle while another terminal signed in to B, first prompt afterwards
    c.ask("s2", "cli", BASE + H + 20 * M, "r3").resolve();
    assert_eq!([c.owner("r1"), c.owner("r2"), c.owner("r3")], [None, None, None]);
}

#[test]
fn one_folder_per_account_names_its_sessions() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + 3 * H).signed_in_at(&other_root(), Some(&b), BASE, BASE + 3 * H);
    c.ask("s1", "cli", BASE + 10 * M, "r1").ask("s1", "claude-vscode", BASE + 2 * H, "r2");
    c.ask_in(&other_root(), "s2", "cli", BASE + 20 * M, "r3");
    // a session from before the app watched the folder
    c.ask("s0", "cli", BASE - H, "r4").ask("s0", "cli", BASE + 30 * M, "r5");
    // between two looks the folder was not watched
    c.ask("s3", "cli", BASE + 3 * H + 10 * M, "r6").resolve();
    assert_eq!([c.owner("r1"), c.owner("r2")], [Some(a.clone()), Some(a.clone())]);
    assert_eq!(c.owner("r3"), Some(b));
    assert_eq!([c.owner("r4"), c.owner("r5"), c.owner("r6")], [None, None, None]);
}

#[test]
fn the_desktop_app_runs_its_sessions_with_its_own_sign_in() {
    let mut c = Claude::new();
    let (a, d) = (org(1), org(4));
    c.signed_in(Some(&a), BASE, BASE + 2 * H);
    // Claude Code is signed in to A; the desktop app to D
    c.ask("cli1", "cli", BASE + 10 * M, "r1").ask("desk1", "claude-desktop", BASE + 20 * M, "r2").ask("desk2", "claude-desktop", BASE + 30 * M, "r3");
    // the desktop app's session resumed in a terminal runs on Claude Code's own sign-in
    c.ask("desk1", "cli", BASE + 40 * M, "r4");
    c.desktop("desk1", &d).resolve();
    assert_eq!(c.owner("r1"), Some(a));
    assert_eq!(c.owner("r2"), Some(d), "the desktop app's list names it");
    assert_eq!(c.owner("r3"), None, "a desktop session it no longer lists says nothing about Claude Code's sign-in");
    assert_eq!(c.owner("r4"), None);
}

#[test]
fn the_sdk_and_a_signed_out_folder_name_no_account() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + H).signed_in(None, BASE + H + 30_000, BASE + 2 * H).signed_in(Some(&a), BASE + 2 * H + 30_000, BASE + 3 * H);
    c.signed_in_at(&other_root(), Some(&b), BASE, BASE + 3 * H);
    // an Agent SDK run may carry its own key; a signed-out folder runs on an API key, if anything
    c.ask("s1", "sdk-ts", BASE + 10 * M, "r1").ask("s2", "cli", BASE + H + 10 * M, "r2");
    c.ask("s3", "cli", BASE + 2 * H + 10 * M, "r3").resolve();
    assert_eq!([c.owner("r1"), c.owner("r2")], [None, None]);
    assert_eq!(c.owner("r3"), Some(a), "signing out and back in to the same account changes nothing");
}

#[test]
fn a_gap_in_watching_breaks_the_sign_in() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    // the app was closed for longer than the gap allows; it could have changed meanwhile
    c.signed_in(Some(&a), BASE, BASE + H).signed_in(Some(&a), BASE + H + SIGNIN_GAP_MS + M, BASE + 3 * H);
    c.signed_in_at(&other_root(), Some(&b), BASE, BASE + 3 * H);
    c.ask("s1", "cli", BASE + 30 * M, "r1").ask("s1", "cli", BASE + 2 * H, "r2").ask("s2", "cli", BASE + 2 * H, "r3").resolve();
    assert_eq!(c.owner("r1"), Some(a.clone()));
    assert_eq!(c.owner("r2"), None);
    assert_eq!(c.owner("r3"), Some(a));
    let spans: i64 = c.store.conn().query_row("SELECT COUNT(*) FROM signin_span WHERE root = ?1", [root()], |r| r.get(0)).unwrap();
    assert_eq!(spans, 2);
}

#[test]
fn a_clock_set_back_notes_nothing_until_it_is_past_the_last_look() {
    let mut c = Claude::new();
    let a = org(1);
    c.signed_in(Some(&a), BASE, BASE + H);
    assert!(!c.store.note_signin(&root(), Some(&org(2)), BASE + 10 * M).unwrap());
    assert!(!c.store.note_signin(&root(), Some(&a), BASE + H + 30_000).unwrap(), "the span goes on");
    let spans: Vec<(i64, i64)> = c.store.conn().prepare("SELECT from_ms, to_ms FROM signin_span").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(spans, [(BASE, BASE + H + 30_000)]);
}

#[test]
fn a_five_hour_reading_alone_is_not_another_account() {
    // Claude Code logs a five-hour line when that limit is hit; it is no window of its own
    let mut c = Claude::new();
    let a = org(1);
    let week = BASE / 1000 + 5 * 86_400;
    c.signed_in(Some(&a), BASE, BASE + 3 * H).ask("s0", "cli", BASE - 2 * H, "r0");
    for i in 0..12 {
        c.read(BASE + i * 15 * M, "claude_code_usage", Some(a.clone()), week, 20.0 + i as f64);
    }
    let mut tx = c.store.transaction().unwrap();
    tx.insert_limits(&[LimitSnapshot {
        ts_ms: BASE + 70 * M,
        provider: Provider::Anthropic,
        tool: Tool::ClaudeCode,
        account: None,
        limit_id: String::new(),
        window: "five_hour".into(),
        used_pct: Some(100.0),
        resets_at: Some(BASE / 1000 + 2 * 3600),
        status: Some("rejected".into()),
        plan: None,
        source: "claude_code_jsonl".into(),
        accuracy: Accuracy::Exact,
    }])
    .unwrap();
    tx.commit().unwrap();
    c.resolve();
    assert!(accounts(&c.store).unwrap().is_empty());
    assert_eq!(c.owner("r0"), Some(a));
}

#[test]
fn readings_follow_their_sign_in_and_share_a_named_window() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    let (week_a, week_b) = (BASE / 1000 + 4 * 86_400, BASE / 1000 + 6 * 86_400 + 1234);
    c.read(BASE, "claude_code_usage", Some(a.clone()), week_a, 10.0).read(BASE + 5 * M, "claude_code_usage", Some(b.clone()), week_b, 40.0);
    // the status line names no account, but reports B's week (one second off, as Claude rounds)
    c.read(BASE + 6 * M, "claude_statusline", None, week_b + 1, 41.0);
    // the desktop history names its organisation
    c.read(BASE + 7 * M, "claude_plan_history", Some(a.clone()), 0, 11.0);
    c.read(BASE + 8 * M, "claude_code_usage", Some(b.clone()), week_b, 42.0).resolve();
    assert_eq!(c.reading_owner(BASE, "claude_code_usage"), Some(a.clone()));
    assert_eq!(c.reading_owner(BASE + 6 * M, "claude_statusline"), Some(b.clone()));
    assert_eq!(c.reading_owner(BASE + 7 * M, "claude_plan_history"), Some(a.clone()));

    let list = accounts(&c.store).unwrap();
    let ids: Vec<(Provider, Option<String>, bool)> = list.iter().map(|x| (x.provider, x.id.clone(), x.active)).collect();
    assert_eq!(ids, [(Provider::Anthropic, Some(a.clone()), false), (Provider::Anthropic, Some(b.clone()), true)]);
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + H).unwrap(), Some(Some(b.clone())));

    let views = limits_view(&c.store, &PriceBook::default_book(), BASE + 9 * M, &[]).unwrap();
    let weekly: Vec<(Option<String>, bool, Option<f64>)> = views.iter().filter(|v| v.window == "seven_day").map(|v| (v.account.clone(), v.active, v.used_pct)).collect();
    assert_eq!(weekly, [(Some(b.clone()), true, Some(42.0)), (Some(a.clone()), false, Some(11.0))]);
}

#[test]
fn the_desktop_history_does_not_take_the_place_of_the_apps_own_read() {
    let mut c = Claude::new();
    let (a, d) = (org(1), org(4));
    c.read(BASE, "claude_code_usage", Some(a.clone()), BASE / 1000 + 86_400, 5.0).read(BASE + 10 * M, "claude_plan_history", Some(d.clone()), 0, 30.0).resolve();
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + H).unwrap(), Some(Some(a.clone())), "Claude Code's sign-in is the one in use");
    // once the app has not read for a while, the newest reading tells
    c.read(BASE + 2 * H, "claude_plan_history", Some(d.clone()), 0, 31.0).resolve();
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 3 * H).unwrap(), Some(Some(a.clone())), "a pause in reading is no change");
    c.read(BASE + 30 * H, "claude_plan_history", Some(d.clone()), 0, 32.0).resolve();
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 31 * H).unwrap(), Some(Some(d.clone())));
    // what Claude Code is signed in to right now counts first, once its account has readings
    c.signed_in(Some(&a), BASE + 31 * H, BASE + 31 * H);
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 31 * H).unwrap(), Some(Some(a)));
    c.signed_in(Some(&org(9)), BASE + 31 * H + 30_000, BASE + 31 * H + 30_000);
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 31 * H + 30_000).unwrap(), Some(Some(d)), "an account without readings yet");
}

#[test]
fn folders_signed_in_to_two_accounts_at_once_leave_the_account_in_use_to_the_apps_own_read() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.read(BASE, "claude_code_usage", Some(a.clone()), BASE / 1000 + 4 * 86_400, 10.0).read(BASE + M, "claude_plan_history", Some(b.clone()), 0, 30.0);
    // B's folder was signed in after A's, and both are in use
    c.signed_in(Some(&a), BASE + 2 * M, BASE + 20 * M).signed_in_at(&other_root(), Some(&b), BASE + 10 * M, BASE + 20 * M).resolve();
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 20 * M).unwrap(), Some(Some(a.clone())));
    // A's folder is no longer watched
    c.signed_in_at(&other_root(), Some(&b), BASE + 20 * M + 30_000, BASE + 40 * M);
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 40 * M).unwrap(), Some(Some(b.clone())));
    // a /login to B in A's folder: both folders name B
    c.signed_in(Some(&b), BASE + 40 * M, BASE + 40 * M);
    assert_eq!(in_use(&c.store, Provider::Anthropic, BASE + 40 * M).unwrap(), Some(Some(b)));
}

#[test]
fn the_usage_filter_keeps_one_claude_account() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.ask("s1", "cli", BASE, "r1").told("s1", BASE, "r1", Some(a.clone())).ask("s2", "cli", BASE + M, "r2").told("s2", BASE + M, "r2", Some(b.clone()));
    c.ask("s3", "cli", BASE + 2 * M, "r3").resolve();
    let rows = c.store.events_between(0, i64::MAX).unwrap();
    let only = |key: String| rows.iter().filter(|e| Filter { accounts: vec![key.clone()], ..Default::default() }.matches(e)).count();
    assert_eq!(only(account_key(Provider::Anthropic, Some(&a))), 1);
    assert_eq!(only(account_key(Provider::Anthropic, None)), 1, "the request no evidence names");
    assert_eq!(only(account_key(Provider::OpenAI, Some(&a))), 0);
}

#[test]
fn ids_kept_raw_by_an_older_version_become_digests_and_imports_keep_the_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let raw = "0b5e2c1d-3f4a-4b6c-8d9e-0f1a2b3c4d5e";
    let old = dir.path().join("old.db");
    {
        let mut s = Store::open(&old).unwrap();
        let mut tx = s.transaction().unwrap();
        let reading = |ts: i64, account: &str| LimitSnapshot {
            ts_ms: ts,
            provider: Provider::Anthropic,
            tool: Tool::ClaudeDesktop,
            account: Some(account.into()),
            limit_id: String::new(),
            window: "five_hour".into(),
            used_pct: Some(9.0),
            resets_at: None,
            status: None,
            plan: None,
            source: "claude_plan_history".into(),
            accuracy: Accuracy::Exact,
        };
        // as an older version stored it, and the same moment once more with the digest
        tx.insert_limits(&[reading(BASE, raw), reading(BASE + M, raw), reading(BASE + M, &claude_account(raw))]).unwrap();
        tx.insert_session_accounts(&[SessionAccount { tool: Tool::ClaudeCode, session_id: "desk1".into(), account: org(4) }]).unwrap();
        tx.commit().unwrap();
        s.note_signin(&root(), Some(&org(1)), BASE).unwrap();
    }
    let reopened = Store::open(&old).unwrap();
    let kept: Vec<String> = reopened.conn().prepare("SELECT account FROM limit_snapshot ORDER BY ts_ms").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(kept, [claude_account(raw), claude_account(raw)]);
    drop(reopened);

    let mut fresh = Store::open(&dir.path().join("fresh.db")).unwrap();
    fresh.merge_from(&old).unwrap();
    let count = |sql: &str| -> i64 { fresh.conn().query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM account_session"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM signin_span WHERE root NOT LIKE 'import:%'"), 0, "a sign-in belongs to the computer that saw it");
    assert_eq!(count("SELECT COUNT(*) FROM limit_snapshot WHERE length(account) = 16"), 2);
    resolve(&mut fresh).unwrap();
}

#[test]
fn logs_the_desktop_session_list_and_the_sign_in_tie_requests_end_to_end() {
    use tracker_core::capture::claude_signin;
    use tracker_core::discovery::{enumerate_files, Env, ExtraPaths, SourceId};
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let support = dir.path().join("support");
    let env = Env { home: Some(home.clone()), roaming: Some(support.clone()), ..Default::default() };
    let (org_a, org_d) = ("0a0a0a0a-0000-4000-8000-00000000000a", "0d0d0d0d-0000-4000-8000-00000000000d");
    let line = |sid: &str, n: u32, ts: i64, entry: &str| {
        let t = chrono::DateTime::from_timestamp_millis(ts).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        format!(
            r#"{{"type":"assistant","sessionId":"{sid}","uuid":"u-{n}","requestId":"req_{n}","timestamp":"{t}","cwd":"/work/app","version":"2.1.310","entrypoint":"{entry}","message":{{"id":"msg_{n}","model":"claude-sonnet-5-5","role":"assistant","content":[],"usage":{{"input_tokens":5,"output_tokens":10}}}}}}"#
        )
    };
    let write = |p: PathBuf, body: String| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    let projects = home.join(".claude").join("projects").join("-work-app");
    write(projects.join("cli-1.jsonl"), line("cli-1", 1, BASE + 10 * M, "cli"));
    write(projects.join("desk-1.jsonl"), line("desk-1", 2, BASE + 20 * M, "claude-desktop"));
    write(projects.join("cli-1").join("subagents").join("agent-x.jsonl"), line("cli-1", 3, BASE + 12 * M, "cli"));
    write(
        support.join("Claude").join("claude-code-sessions").join("1d1d1d1d-0000-4000-8000-00000000001d").join(org_d).join("local_9.json"),
        r#"{"sessionId":"local_9","cliSessionId":"desk-1","title":"t"}"#.into(),
    );
    write(home.join(".claude.json"), format!(r#"{{"oauthAccount":{{"emailAddress":"someone@example.com","organizationUuid":"{org_a}"}}}}"#));

    let mut store = Store::open_in_memory().unwrap();
    for (root, file) in claude_signin::roots(&env, &ExtraPaths::default()) {
        let account = claude_signin::signed_in(&file).ok().flatten();
        for t in (BASE..=BASE + H).step_by(30_000) {
            store.note_signin(&root, account.as_deref(), t).unwrap();
        }
    }
    let all: std::collections::HashSet<SourceId> = SourceId::ALL.into_iter().collect();
    use_test_round();
    tracker_core::ingest::ingest(&mut store, &enumerate_files(&env, &ExtraPaths::default(), &all), |_| {});
    let owner = |rid: &str| -> Option<String> { store.conn().query_row("SELECT owner FROM usage_event WHERE request_id = ?1", [rid], |r| r.get(0)).unwrap() };
    assert_eq!(owner("req_1"), Some(claude_account(org_a)), "the sign-in held through the session");
    assert_eq!(owner("req_3"), Some(claude_account(org_a)), "a subagent's log lies in its session's folder");
    assert_eq!(owner("req_2"), Some(claude_account(org_d)), "the desktop app's own sign-in");
    let list = accounts(&store).unwrap();
    assert_eq!(list.iter().filter(|a| a.provider == Provider::Anthropic && a.id.is_some()).count(), 2);
    let raw: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM file_checkpoint WHERE path LIKE '%1d1d1d1d%' OR path LIKE ?1", [format!("%{org_d}%")], |r| r.get(0))
        .unwrap();
    assert_eq!(raw, 0, "the account ids the desktop app names its folders after are kept as digests");
    let again = tracker_core::ingest::ingest(&mut store, &enumerate_files(&env, &ExtraPaths::default(), &all), |_| {});
    assert_eq!(again.files_read, 0, "and found again under them");

    // the folder a session lies in is kept, so read-again checkpoints and deleted logs lose nothing
    store.conn().execute("DELETE FROM file_checkpoint", []).unwrap();
    store.conn().execute("UPDATE usage_event SET owner = NULL", []).unwrap();
    resolve_claude(&mut store).unwrap();
    let owner = |rid: &str| -> Option<String> { store.conn().query_row("SELECT owner FROM usage_event WHERE request_id = ?1", [rid], |r| r.get(0)).unwrap() };
    assert_eq!(owner("req_1"), Some(claude_account(org_a)));
}

#[test]
fn telemetry_received_live_ties_its_request_before_the_app_is_told() {
    use std::io::{Read, Write};
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tracker.db");
    let mut c = Claude { store: Store::open(&db).unwrap() };
    let (a, b) = (org(1), org(2));
    c.read(BASE, "claude_code_usage", Some(a), BASE / 1000 + 4 * 86_400, 10.0).read(BASE + M, "claude_code_usage", Some(b.clone()), BASE / 1000 + 6 * 86_400, 20.0);
    c.ask("s1", "cli", BASE + 2 * M, "r-live").resolve();
    assert_eq!(c.owner("r-live"), None);
    let (tx, rx) = std::sync::mpsc::channel();
    let receiver = tracker_core::capture::otlp::Receiver::start(0, db, "token".into(), move |_| {
        let _ = tx.send(());
    })
    .unwrap();
    let body = serde_json::json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{
        "timeUnixNano": ((BASE + 2 * M) * 1_000_000).to_string(),
        "body": {"stringValue": "claude_code.api_request"},
        "attributes": [
            {"key": "request_id", "value": {"stringValue": "r-live"}},
            {"key": "session.id", "value": {"stringValue": "s1"}},
            {"key": "model", "value": {"stringValue": "claude-sonnet-5-5"}},
            {"key": "organization.id", "value": {"stringValue": "00000002-0000-4000-8000-000000000002"}},
            {"key": "input_tokens", "value": {"intValue": "10"}},
            {"key": "output_tokens", "value": {"intValue": "5"}}
        ]
    }]}]}]})
    .to_string();
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", receiver.port)).unwrap();
    socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    write!(socket, "POST /v1/logs HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nx-aiut-token: token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(c.owner("r-live"), Some(b));
}

#[test]
fn two_accounts_read_at_the_same_moment_keep_their_own_readings() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.read(BASE, "claude_plan_history", Some(a.clone()), 0, 10.0).read(BASE, "claude_plan_history", Some(b.clone()), 0, 70.0).resolve();
    let wrong: i64 = c.store.conn().query_row("SELECT COUNT(*) FROM limit_snapshot WHERE owner IS NOT account", [], |r| r.get(0)).unwrap();
    assert_eq!(wrong, 0);
    let ids: Vec<Option<String>> = accounts(&c.store).unwrap().into_iter().map(|x| x.id).collect();
    assert!(ids.contains(&Some(a)) && ids.contains(&Some(b)));
}

#[test]
fn a_backup_keeps_the_accounts_its_own_folders_named() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + H).signed_in_at(&other_root(), Some(&b), BASE, BASE + H);
    c.ask("sa", "cli", BASE + M, "ra").ask_in(&other_root(), "sb", "cli", BASE + 2 * M, "rb").resolve();
    assert_eq!((c.owner("ra"), c.owner("rb")), (Some(a.clone()), Some(b.clone())));
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("backup.db");
    c.store.backup_to(&backup).unwrap();

    // another computer, whose folder of the same path is signed in to B
    let mut other = Claude::new();
    other.signed_in(Some(&b), BASE, BASE + H).ask("here", "cli", BASE + 3 * M, "rh");
    other.read(BASE + H - 2 * M, "claude_code_usage", Some(b.clone()), BASE / 1000 + 6 * 86_400, 20.0);
    other.read(BASE + H - M, "claude_code_usage", Some(a.clone()), BASE / 1000 + 4 * 86_400, 10.0);
    other.store.merge_from(&backup).unwrap();
    other.resolve();
    assert_eq!((other.owner("ra"), other.owner("rb")), (Some(a.clone()), Some(b.clone())));
    assert_eq!(other.owner("rh"), Some(b.clone()), "the backup's sign-ins say nothing about this computer's folders");
    assert_eq!(in_use(&other.store, Provider::Anthropic, BASE + H).unwrap(), Some(Some(b.clone())), "nor about what it is signed in to now");
    let spans = |st: &Store| -> i64 { st.conn().query_row("SELECT COUNT(*) FROM signin_span", [], |r| r.get(0)).unwrap() };
    let before = spans(&other.store);
    other.store.merge_from(&backup).unwrap();
    assert_eq!(spans(&other.store), before, "importing it again adds nothing");

    // the computer that made it, after its archive was lost and its logs were read again
    let mut again = Claude::new();
    again.ask("sa", "cli", BASE + M, "ra").ask_in(&other_root(), "sb", "cli", BASE + 2 * M, "rb");
    again.signed_in(Some(&a), BASE + 2 * H, BASE + 2 * H);
    again.store.merge_from(&backup).unwrap();
    again.resolve();
    assert_eq!((again.owner("ra"), again.owner("rb")), (Some(a), Some(b)));
    let imported: i64 = again.store.conn().query_row("SELECT COUNT(*) FROM signin_span WHERE root LIKE 'import:%'", [], |r| r.get(0)).unwrap();
    assert_eq!(imported, 0, "a session both archives read from one folder shows the backup was made here");
}

#[test]
fn a_newer_backup_of_the_same_archive_brings_its_longer_sign_ins() {
    let mut source = Claude::new();
    let (a, b) = (org(1), org(2));
    source.signed_in(Some(&a), BASE, BASE + 10 * M).signed_in_at(&other_root(), Some(&b), BASE, BASE + H);
    source.ask("sa", "cli", BASE + M, "ra").ask_in(&other_root(), "sb", "cli", BASE + 2 * M, "rb").resolve();
    let dir = tempfile::tempdir().unwrap();
    let (early, later) = (dir.path().join("early.db"), dir.path().join("later.db"));
    source.store.backup_to(&early).unwrap();
    let mut target = Claude::new();
    target.store.merge_from(&early).unwrap();
    target.resolve();
    assert_eq!(target.owner("ra"), Some(a.clone()));

    source.signed_in(Some(&a), BASE + 10 * M + 30_000, BASE + H).ask("sa", "cli", BASE + 30 * M, "ra-later").resolve();
    assert_eq!(source.owner("ra-later"), Some(a.clone()));
    source.store.backup_to(&later).unwrap();
    target.store.merge_from(&later).unwrap();
    target.resolve();
    assert_eq!(target.owner("ra-later"), Some(a));
    // importing the earlier one again shortens nothing
    target.store.merge_from(&early).unwrap();
    target.resolve();
    assert_eq!(target.owner("ra-later"), source.owner("ra-later"));
}

#[test]
fn archives_that_name_two_accounts_for_one_moment_leave_the_folder_naming_neither() {
    let (a, b) = (org(1), org(2));
    let mut here = Claude::new();
    here.signed_in(Some(&a), BASE, BASE + H).signed_in_at(&other_root(), Some(&b), BASE, BASE + H).ask("sa", "cli", BASE + M, "ra").resolve();
    assert_eq!(here.owner("ra"), Some(a));
    // a damaged or hand-made backup of this computer's archive with B at the same moment
    let mut odd = Claude::new();
    odd.signed_in(Some(&b), BASE, BASE + H).ask("sa", "cli", BASE + M, "ra");
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("odd.db");
    odd.store.backup_to(&backup).unwrap();
    here.store.merge_from(&backup).unwrap();
    here.resolve();
    assert_eq!(here.owner("ra"), None);
}

#[test]
fn a_look_that_covers_a_request_logged_before_it_ties_the_request() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE).signed_in_at(&other_root(), Some(&b), BASE, BASE);
    c.ask("s1", "cli", BASE + 10_000, "r1").resolve();
    assert_eq!(c.owner("r1"), None, "no look has covered it yet");
    assert!(c.store.note_signin(&root(), Some(&a), BASE + 30_000).unwrap(), "the look covers a request no account was found for");
    c.resolve();
    assert_eq!(c.owner("r1"), Some(a.clone()));
    assert!(!c.store.note_signin(&root(), Some(&a), BASE + 60_000).unwrap(), "nothing is left to tie");
}

#[test]
fn the_account_filter_keeps_only_that_accounts_tool_calls() {
    use tracker_core::analytics::Range;
    use tracker_core::model::ToolCall;
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.ask("sa", "cli", BASE, "ra").told("sa", BASE, "ra", Some(a.clone()));
    c.ask("sb", "cli", BASE + M, "rb").told("sb", BASE + M, "rb", Some(b)).resolve();
    let call = |name: &str, session: &str| ToolCall {
        key: name.into(),
        ts_ms: BASE + 2 * M,
        tool: Tool::ClaudeCode,
        session_id: Some(session.into()),
        project_path: None,
        agent: None,
        name: name.into(),
        failed: Some(false),
    };
    let mut tx = c.store.transaction().unwrap();
    tx.upsert_tool_calls(&[call("Read", "sa"), call("Write", "sb")]).unwrap();
    tx.commit().unwrap();
    let f = Filter { accounts: vec![account_key(Provider::Anthropic, Some(&a))], ..Default::default() };
    let got = tracker_core::insights::agents_tools(&c.store, &PriceBook::default_book(), Range { from_ms: BASE, to_ms: BASE + H }, &f).unwrap();
    assert_eq!(got.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Read"]);
    assert!(got.filtered_by_session);
    let tips = tracker_core::tips::tips(&c.store, &PriceBook::default_book(), Range { from_ms: BASE, to_ms: BASE + H }, &f).unwrap();
    assert_eq!(tips.tool_calls, 1, "the tips count the same calls");
}

#[test]
fn plan_advice_counts_the_days_of_the_account_in_use() {
    use tracker_core::history::{limit_history, plan_advice, AdviceKind};
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    for d in 0..7 {
        c.read(BASE - (7 - d) * 86_400_000, "claude_code_usage", Some(a.clone()), BASE / 1000 + 86_400, 10.0);
    }
    c.read(BASE, "claude_code_usage", Some(b), BASE / 1000 + 5 * 86_400, 100.0).resolve();
    let h = limit_history(&c.store, BASE + M, 28).unwrap();
    assert_eq!(h.observed_days.get(&Provider::Anthropic), Some(&1), "B, in use, was read on one day");
    let advice = plan_advice(&h, Provider::Anthropic, Some("max5x"), &tracker_core::plans::PlansFile::bundled(), BASE + M);
    assert!(matches!(advice.kind, AdviceKind::Insufficient), "A's days do not make B's data enough: {advice:?}");
}

#[test]
fn each_account_gets_advice_on_its_own_plan_windows_and_days() {
    use tracker_core::history::{account_advice, limit_history, AdviceKind};
    let plans = tracker_core::plans::PlansFile::bundled();
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    let week = (BASE + H) / 1000;
    // a week of daily readings: A, on Pro, ends it full; B, on Max, stays low
    for d in 0..7 {
        let ts = BASE - (7 - d) * 86_400_000 + 2 * H;
        c.read_on("pro", ts, "claude_code_usage", Some(a.clone()), week, if d == 6 { 100.0 } else { 10.0 * d as f64 });
        c.read(ts + H, "claude_code_usage", Some(b.clone()), week + 3600, 8.0);
    }
    c.resolve();
    let listed: Vec<(Option<String>, Option<String>)> =
        accounts(&c.store).unwrap().into_iter().map(|x| (x.id, x.read_plan)).collect();
    assert_eq!(listed, [(Some(a.clone()), Some("pro".into())), (Some(b.clone()), Some("max".into()))]);
    // an older version's row dated far ahead of the clock does not decide the plan
    c.read_on("max20x-typo", BASE - H, "claude_code_usage", Some(b.clone()), week + 3600, 8.0);
    c.store.conn().execute("UPDATE limit_snapshot SET ts_ms = ?1 WHERE plan = 'max20x-typo'", [BASE + 50 * 365 * 86_400_000]).unwrap();
    assert_eq!(accounts(&c.store).unwrap()[1].read_plan.as_deref(), Some("max"));
    let h = limit_history(&c.store, BASE, 28).unwrap();
    let pro = account_advice(&h, Provider::Anthropic, &a, Some("pro"), &plans, BASE);
    assert!(matches!(pro.kind, AdviceKind::Upgrade), "{pro:?}");
    assert_eq!((pro.account.as_deref(), pro.suggested.as_deref(), pro.observed_days), (Some(a.as_str()), Some("max5x"), 7));
    let max = account_advice(&h, Provider::Anthropic, &b, Some("max5x"), &plans, BASE);
    assert!(matches!(max.kind, AdviceKind::Fits), "{max:?}");
    assert_eq!((max.weekly.full, max.weekly.windows), (0, 1), "A's full week is not B's");
    let unchosen = account_advice(&h, Provider::Anthropic, &b, None, &plans, BASE);
    assert!(matches!(unchosen.kind, AdviceKind::NoPlan));
}

#[test]
fn plan_value_splits_a_providers_use_between_its_accounts() {
    use chrono::FixedOffset;
    use tracker_core::analytics::Range;
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.read(BASE - H, "claude_code_usage", Some(a.clone()), BASE / 1000 + 86_400, 10.0);
    c.read(BASE - H + M, "claude_code_usage", Some(b.clone()), BASE / 1000 + 2 * 86_400, 10.0);
    for (ts, rid, who) in [(BASE + M, "r1", Some(&a)), (BASE + 2 * M, "r2", Some(&a)), (BASE + 3 * M, "r3", Some(&b)), (BASE + 4 * M, "r4", None)] {
        c.ask("s1", "cli", ts, rid).told("s1", ts, rid, who.cloned());
    }
    c.resolve();
    let tz = FixedOffset::east_opt(0).unwrap();
    let v = tracker_core::insights::plan_value(&c.store, &PriceBook::default_book(), Range { from_ms: BASE, to_ms: BASE + H }, &tz).unwrap();
    let claude = v.providers.iter().find(|p| p.provider == Provider::Anthropic).unwrap();
    let split: std::collections::BTreeMap<Option<String>, u64> = claude.accounts.iter().map(|x| (x.account.clone(), x.events)).collect();
    assert_eq!(split, [(None, 1), (Some(a.clone()), 2), (Some(b.clone()), 1)].into());
    assert_eq!(claude.events, 4, "the total keeps use no account is known for");
    let parts: f64 = claude.accounts.iter().map(|x| x.cumulative.last().unwrap()).sum();
    assert!((parts - claude.cumulative.last().unwrap()).abs() < 1e-12 && claude.cost_usd > 0.0);
}

#[test]
fn a_folder_is_one_folder_however_its_path_is_spelt() {
    use tracker_core::accounts::claude_log_session;
    assert_eq!(root_key(Path::new("/x/claude/")), root_key(Path::new("/x/claude")));
    let log = Path::new("/x/claude/projects/-w/abc.jsonl.zst");
    assert_eq!(claude_log_session(log), Some((root_key(Path::new("/x/claude")), "abc".into())));
    let sub = Path::new("/x/claude/projects/-w/abc/subagents/agent-1.jsonl");
    assert_eq!(claude_log_session(sub), Some((root_key(Path::new("/x/claude")), "abc".into())));
    assert_eq!(claude_log_session(Path::new("/x/claude/elsewhere/-w/abc.jsonl")), None);
}

#[test]
fn telemetry_on_another_account_leaves_its_sessions_folder_naming_none_of_it() {
    // the folder only ever named A, but the session runs on B's token; one event reached the app
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + 3 * H);
    c.ask("s1", "cli", BASE + 10 * M, "r1");
    c.ask("s1", "cli", BASE + 20 * M, "r2").told("s1", BASE + 20 * M, "r2", Some(b.clone()));
    c.ask("s1", "cli", BASE + 30 * M, "r3").resolve();
    assert_eq!(c.owner("r2"), Some(b));
    assert_eq!([c.owner("r1"), c.owner("r3")], [None, None]);
}

#[test]
fn a_session_waking_with_its_older_reading_does_not_make_one_account_look_like_two() {
    let a = org(1);
    let (five, week) = (BASE / 1000 + 2 * 3600, BASE / 1000 + 3 * 86_400);
    // the same week, lower than just read
    let mut c = Claude::new();
    c.signed_in(Some(&a), BASE, BASE + 3 * H).ask("s1", "cli", BASE + 10 * M, "r1");
    c.status(BASE + 20 * M, five, week, 40.0, 50.0).status(BASE + 25 * M, five, week, 10.0, 30.0);
    c.status(BASE + 30 * M, five, week, 41.0, 51.0).resolve();
    assert_eq!(c.claude_accounts(), Vec::<Option<String>>::new());
    let views = limits_view(&c.store, &PriceBook::default_book(), BASE + 31 * M, &[]).unwrap();
    let weekly: Vec<(Option<String>, u64)> = views.iter().filter(|v| v.window == "seven_day").map(|v| (v.account.clone(), v.window_usage.events)).collect();
    assert_eq!(weekly, [(None, 1)]);

    // a five-hour window that ended hours ago
    let mut c = Claude::new();
    let (f0, f1) = (BASE / 1000 + 3600, BASE / 1000 + 6 * 3600);
    c.signed_in(Some(&a), BASE - 4 * H, BASE + 3 * H).ask("s1", "cli", BASE - 3 * H, "r1");
    c.status(BASE - 3 * H, f0, week, 30.0, 20.0).status(BASE - 2 * H, f0, week, 40.0, 22.0);
    c.status(BASE + H + 10 * M, f1, week, 5.0, 25.0).status(BASE + 2 * H, f1, week, 12.0, 27.0);
    c.status(BASE + 2 * H + 5 * M, f0, week, 40.0, 22.0).status(BASE + 2 * H + 10 * M, f1, week, 13.0, 27.0).resolve();
    assert_eq!(c.claude_accounts(), Vec::<Option<String>>::new());

    // last week, after the new one began
    let mut c = Claude::new();
    let (w1, w2) = (BASE / 1000, BASE / 1000 + 7 * 86_400);
    c.signed_in(Some(&a), BASE - 3 * H, BASE + 2 * H).ask("s1", "cli", BASE - 2 * H + M, "r1");
    c.status(BASE - 2 * H, BASE / 1000 - 3600, w1, 30.0, 90.0).status(BASE - H, BASE / 1000 + 3600, w1, 50.0, 95.0);
    c.status(BASE + 10 * M, BASE / 1000 + 5 * 3600, w2, 2.0, 1.5).status(BASE + 40 * M, BASE / 1000 + 5 * 3600, w2, 6.0, 3.0);
    c.status(BASE + 45 * M, BASE / 1000 + 3600, w1, 50.0, 95.0).status(BASE + 60 * M, BASE / 1000 + 5 * 3600, w2, 8.0, 4.0).resolve();
    assert_eq!(c.claude_accounts(), Vec::<Option<String>>::new());
    assert_eq!(c.owner("r1"), Some(a));
}

#[test]
fn a_backup_with_a_sign_in_dated_ahead_leaves_this_computers_looks_counting() {
    let (a, b) = (org(1), org(2));
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("ahead.db");
    {
        let mut ahead = Claude::new();
        ahead.signed_in(Some(&a), BASE, BASE + 10 * M).ask("sa", "cli", BASE + M, "ra");
        ahead.store.conn().execute("INSERT INTO signin_span(root, account, from_ms, to_ms) VALUES(?1, ?2, ?3, ?3)", rusqlite::params![root(), a, 32_000_000_000_000i64]).unwrap();
        ahead.store.backup_to(&backup).unwrap();
    }
    let mut c = Claude::new();
    c.signed_in_at(&other_root(), Some(&b), BASE, BASE + 3 * H);
    c.signed_in(Some(&a), BASE, BASE + H).ask("sa", "cli", BASE + M, "ra");
    c.store.merge_from(&backup).unwrap();
    c.signed_in(Some(&a), BASE + H + 30_000, BASE + 3 * H).ask("s2", "cli", BASE + 2 * H, "r2").resolve();
    assert_eq!(c.owner("r2"), Some(a));
}

#[test]
fn a_session_continued_in_another_config_folder_is_tied_by_neither_folder() {
    let mut c = Claude::new();
    let (a, b) = (org(1), org(2));
    c.signed_in(Some(&a), BASE, BASE + 3 * H).signed_in_at(&other_root(), Some(&b), BASE, BASE + 3 * H);
    c.ask("s1", "cli", BASE + 10 * M, "r1").resolve();
    assert_eq!(c.owner("r1"), Some(a));
    // its log copied into the other folder and resumed there
    c.ask_in(&other_root(), "s1", "cli", BASE + H, "r2").resolve();
    assert_eq!([c.owner("r1"), c.owner("r2")], [None, None]);

    // the same, known only by the folders kept while reading its logs
    c.ask("s2", "cli", BASE + 20 * M, "r3");
    let mut tx = c.store.transaction().unwrap();
    tx.note_session_root(Tool::ClaudeCode, "s2", &root()).unwrap();
    tx.note_session_root(Tool::ClaudeCode, "s2", &other_root()).unwrap();
    tx.commit().unwrap();
    c.store.conn().execute("DELETE FROM file_checkpoint", []).unwrap();
    c.resolve();
    assert_eq!(c.owner("r3"), None);
}

#[test]
fn a_backup_that_saw_a_session_in_two_folders_keeps_it_tied_by_neither() {
    let (a, b) = (org(1), org(2));
    let mut source = Claude::new();
    source.signed_in(Some(&a), BASE, BASE + H).signed_in_at(&other_root(), Some(&b), BASE, BASE + H);
    let noted = |c: &mut Claude, folder: &str| {
        let mut tx = c.store.transaction().unwrap();
        tx.note_session_root(Tool::ClaudeCode, "s1", folder).unwrap();
        tx.commit().unwrap();
    };
    source.ask("s1", "cli", BASE + M, "r1");
    noted(&mut source, &root());
    let dir = tempfile::tempdir().unwrap();
    let file = |n: &str| dir.path().join(n);
    source.store.backup_to(&file("early.db")).unwrap();
    // resumed in the other folder, where telemetry names one request's account
    source.ask_in(&other_root(), "s1", "cli", BASE + 20 * M, "r2").ask_in(&other_root(), "s1", "cli", BASE + 30 * M, "r3");
    source.told("s1", BASE + 30 * M, "r3", Some(a.clone()));
    noted(&mut source, &other_root());
    source.resolve();
    let owners = |c: &Claude| [c.owner("r1"), c.owner("r2"), c.owner("r3")];
    let want = [None, None, Some(a.clone())];
    assert_eq!(owners(&source), want);
    source.store.backup_to(&file("later.db")).unwrap();
    // the same backup knowing the folders only by what was noted while reading
    source.store.backup_to(&file("bare.db")).unwrap();
    rusqlite::Connection::open(file("bare.db")).unwrap().execute("DELETE FROM file_checkpoint", []).unwrap();

    let imported = |backups: &[&str]| {
        let mut c = Claude::new();
        for b in backups {
            c.store.merge_from(&file(b)).unwrap();
            c.resolve();
        }
        c
    };
    assert_eq!(imported(&["early.db"]).owner("r1"), Some(a.clone()));
    for order in [["early.db", "later.db"], ["later.db", "early.db"], ["bare.db", "early.db"], ["early.db", "bare.db"]] {
        assert_eq!(owners(&imported(&order)), want, "{order:?}");
    }
    // a hand-made one giving the mark sign-ins of its own
    source.store.backup_to(&file("odd.db")).unwrap();
    let odd = rusqlite::Connection::open(file("odd.db")).unwrap();
    odd.execute("INSERT INTO signin_span(root, account, from_ms, to_ms) VALUES('', ?1, ?2, ?3)", rusqlite::params![a, BASE, BASE + H]).unwrap();
    drop(odd);
    let again = imported(&["early.db", "later.db", "later.db", "odd.db"]);
    assert_eq!(owners(&again), want);
    again.store.backup_to(&file("again.db")).unwrap();
    let mut reopened = Claude { store: Store::open(&file("again.db")).unwrap() };
    assert_eq!(owners(reopened.resolve()), want);

    // this computer, which read the session in the first folder only
    for backup in ["later.db", "bare.db"] {
        let mut here = Claude::new();
        here.signed_in(Some(&a), BASE, BASE + H).ask("s1", "cli", BASE + M, "r1").resolve();
        assert_eq!(here.owner("r1"), Some(a.clone()));
        here.store.merge_from(&file(backup)).unwrap();
        assert_eq!(owners(here.resolve()), want, "{backup}");
    }
}

#[test]
fn a_file_without_a_physical_id_keeps_no_account_id_in_its_fallback_id() {
    let dir = tempfile::tempdir().unwrap();
    let (login, org) = ("0b5e2c1d-3f4a-4b6c-8d9e-0f1a2b3c4d5e", "7c8d9e0f-1a2b-4c3d-9e4f-5a6b7c8d9e0f");
    let p = dir.path().join("claude-code-sessions").join(login).join(org).join("local_a.json");
    // a file not there yet has no physical id: the fallback the file system can also force
    let fallback = tracker_core::discovery::file_identity(&p);
    assert!(fallback.starts_with("path:"));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, r#"{"cliSessionId":"local-session"}"#).unwrap();
    let f = tracker_core::discovery::DiscoveredFile {
        path: p,
        file_id: fallback,
        parser: tracker_core::sources::ParserKind::ClaudeDesktopSession,
        source: tracker_core::discovery::SourceId::ClaudeDesktop,
    };
    let mut store = Store::open_in_memory().unwrap();
    assert!(tracker_core::ingest::ingest(&mut store, std::slice::from_ref(&f), |_| {}).errors.is_empty());
    let (path, id): (String, String) = store.conn().query_row("SELECT path, file_id FROM file_checkpoint", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(![&path, &id].iter().any(|v| v.contains(login) || v.contains(org)));
    assert_eq!(tracker_core::ingest::ingest(&mut store, &[f], |_| {}).files_read, 0);
}
