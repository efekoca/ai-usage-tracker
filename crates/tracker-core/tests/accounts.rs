mod common;

use common::use_test_round;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracker_core::accounts::{accounts, resolve, NAMELESS};
use tracker_core::analytics::{account_key, limits_view, Filter, LimitState};
use tracker_core::discovery::{enumerate_files, Env, ExtraPaths, SourceId};
use tracker_core::history::limit_history;
use tracker_core::ingest::ingest;
use tracker_core::model::{account_digest, Accuracy, LimitSnapshot, Provider, Tool};
use tracker_core::pricing::PriceBook;
use tracker_core::store::Store;

/// 2026-09-01T00:00:00Z
const BASE: i64 = 1_788_220_800_000;
const H: i64 = 3_600_000;
const M: i64 = 60_000;
const DAY_S: i64 = 86_400;

fn at(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A weekly window: when it resets (epoch seconds) and how full it is.
#[derive(Clone, Copy)]
struct Win(i64, f64);

/// A weekly window with its five-hour window: (weekly reset, weekly %, five-hour reset, five-hour %).
#[derive(Clone, Copy)]
struct Both(i64, f64, i64, f64);

/// A Codex thread written the way Codex writes its rollout.
struct Thread {
    id: String,
    lines: Vec<String>,
    total: u64,
}

impl Thread {
    fn new(n: u32, started_ms: i64, creator: Option<&str>) -> Thread {
        let id = format!("0199aaaa-bbbb-7ccc-8ddd-{n:012}");
        let creator = creator.map_or(String::new(), |c| format!(r#","creator_account_id":"{c}""#));
        let meta = format!(
            r#"{{"timestamp":"{t}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{t}","cwd":"/work/app","originator":"Codex Desktop","cli_version":"0.164.0","source":"vscode"{creator}}}}}"#,
            t = at(started_ms)
        );
        let ctx = format!(r#"{{"timestamp":"{}","type":"turn_context","payload":{{"cwd":"/work/app","model":"gpt-5.6-sol"}}}}"#, at(started_ms));
        Thread { id, lines: vec![meta, ctx], total: 0 }
    }

    /// One request; `win` is the weekly window its line reports (`None`: no `rate_limits`).
    fn ask(&mut self, ms: i64, win: Option<Win>) -> &mut Thread {
        self.ask_limit(ms, win, "codex")
    }

    fn ask_limit(&mut self, ms: i64, win: Option<Win>, limit: &str) -> &mut Thread {
        self.ask_both(ms, win.map(|Win(reset, used)| Both(reset, used, ms / 1000 + 3 * 3600, used)), limit)
    }

    fn ask_both(&mut self, ms: i64, win: Option<Both>, limit: &str) -> &mut Thread {
        self.total += 100;
        let rl = match win {
            Some(Both(reset, used, five, five_used)) => format!(
                r#"{{"limit_id":"{limit}","primary":{{"used_percent":{five_used},"window_minutes":300,"resets_at":{five}}},"secondary":{{"used_percent":{used},"window_minutes":10080,"resets_at":{reset}}},"plan_type":"plus"}}"#
            ),
            None => "null".into(),
        };
        self.lines.push(format!(
            r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{t},"output_tokens":10}},"last_token_usage":{{"input_tokens":100,"output_tokens":10}}}},"rate_limits":{rl}}}}}"#,
            at(ms),
            t = self.total,
        ));
        self
    }

    fn write(&self, home: &Path) -> PathBuf {
        let dir = home.join(".codex/sessions/2026/09/01");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("rollout-2026-09-01T00-00-00-{}.jsonl", self.id));
        std::fs::write(&p, self.lines.join("\n") + "\n").unwrap();
        p
    }
}

struct Archive {
    _dir: tempfile::TempDir,
    home: PathBuf,
    store: Store,
}

impl Archive {
    fn new() -> Archive {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        Archive { _dir: dir, home, store: Store::open_in_memory().unwrap() }
    }

    fn add(&mut self, threads: &[&Thread]) {
        for t in threads {
            t.write(&self.home);
        }
        use_test_round();
        let env = Env { home: Some(self.home.clone()), ..Default::default() };
        let files = enumerate_files(&env, &ExtraPaths::default(), &HashSet::from([SourceId::Codex]));
        ingest(&mut self.store, &files, |_| {});
    }

    /// A live read of the signed-in account; `account` is `None` for a Codex that does not name it.
    fn live(&mut self, ms: i64, win: Win, account: Option<&str>) {
        let snap = |window: &str, reset: i64| LimitSnapshot {
            ts_ms: ms,
            provider: Provider::OpenAI,
            tool: Tool::Codex,
            account: account.map(account_digest),
            limit_id: "codex".into(),
            window: window.into(),
            used_pct: Some(win.1),
            resets_at: Some(reset),
            status: None,
            plan: Some("plus".into()),
            source: "codex_app_server".into(),
            accuracy: Accuracy::Captured,
        };
        let mut tx = self.store.transaction().unwrap();
        tx.insert_limits(&[snap("five_hour", ms / 1000 + 3 * 3600), snap("seven_day", win.0)]).unwrap();
        tx.commit().unwrap();
        resolve(&mut self.store).unwrap();
    }

    /// Requests per owner (`None`: no known account).
    fn owners(&self) -> Vec<(Option<String>, i64)> {
        let mut st = self.store.conn().prepare("SELECT owner, COUNT(*) FROM usage_event WHERE tool = 'codex' GROUP BY owner ORDER BY owner").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
    }

    fn owner_at(&self, ms: i64) -> Option<String> {
        self.store.conn().query_row("SELECT owner FROM usage_event WHERE tool = 'codex' AND ts_ms = ?1", [ms], |r| r.get(0)).unwrap()
    }
}

fn a() -> String {
    account_digest("acc-a")
}
fn b() -> String {
    account_digest("acc-b")
}

fn win_a() -> Win {
    Win(BASE / 1000 + 5 * DAY_S, 40.0)
}
fn win_b() -> Win {
    Win(BASE / 1000 + 3 * DAY_S, 10.0)
}

#[test]
fn one_account_changes_nothing() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 2 * M, None).ask(BASE + 3 * H, Some(win_a()));
    // an early reset by the provider starts a new window of the same account
    let mut u = Thread::new(2, BASE + 4 * H, None);
    u.ask(BASE + 4 * H + M, Some(Win(BASE / 1000 + 7 * DAY_S, 2.0)));
    x.add(&[&t, &u]);
    assert!(accounts(&x.store).unwrap().is_empty());
    assert_eq!(x.owners(), [(Some(a()), 4)]);
    let views = limits_view(&x.store, &PriceBook::default_book(), BASE + 5 * H, &[]).unwrap();
    assert!(views.iter().all(|v| v.active && v.account.is_none()), "no account is shown with one account");
    assert_eq!(views.iter().filter(|v| v.window == "seven_day").count(), 1);
}

#[test]
fn logs_that_name_no_account_stay_as_before() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, None);
    t.ask(BASE + M, Some(win_a())).ask(BASE + 2 * M, Some(win_a()));
    x.add(&[&t]);
    assert!(accounts(&x.store).unwrap().is_empty());
    assert_eq!(x.owners(), [(None, 2)]);
}

/// The friend's test: a thread started under A goes on under B after a desktop account switch,
/// and a new thread is started under B.
#[test]
fn a_thread_carried_over_to_another_account_counts_toward_it() {
    let mut x = Archive::new();
    let mut old = Thread::new(1, BASE, Some("acc-a"));
    old.ask(BASE + M, Some(win_a())).ask(BASE + 2 * M, Some(win_a()));
    old.ask(BASE + 20 * M, Some(win_b())).ask(BASE + 21 * M, Some(win_b()));
    let mut new = Thread::new(2, BASE + 19 * M, Some("acc-b"));
    new.ask(BASE + 19 * M + 30_000, Some(win_b()));
    x.add(&[&old, &new]);

    assert_eq!(x.owner_at(BASE + 2 * M), Some(a()));
    assert_eq!(x.owner_at(BASE + 20 * M), Some(b()), "the creator stops vouching once the window changes");
    assert_eq!(x.owner_at(BASE + 21 * M), Some(b()));
    let list = accounts(&x.store).unwrap();
    assert_eq!(list.iter().map(|l| (l.id.clone(), l.active)).collect::<Vec<_>>(), [(Some(a()), false), (Some(b()), true)]);

    let book = PriceBook::default_book();
    let views: Vec<_> = limits_view(&x.store, &book, BASE + 30 * M, &[]).unwrap().into_iter().filter(|v| v.window == "seven_day").collect();
    assert_eq!(views.len(), 2);
    assert!(views[0].active && views[0].account == Some(b()) && views[0].used_pct == Some(10.0));
    assert!(!views[1].active && views[1].account == Some(a()) && views[1].used_pct == Some(40.0));
    assert_eq!(views[0].window_usage.events, 3, "only B's requests fill B's window");
    assert_eq!(views[1].window_usage.events, 2);
}

#[test]
fn switching_back_returns_to_the_first_accounts_window() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + H, Some(win_b())).ask(BASE + 2 * H, Some(Win(win_a().0, 45.0)));
    let mut u = Thread::new(2, BASE + H, Some("acc-b"));
    u.ask(BASE + H + M, Some(win_b()));
    x.add(&[&t, &u]);
    assert_eq!(x.owner_at(BASE + H), Some(b()));
    assert_eq!(x.owner_at(BASE + 2 * H), Some(a()));
}

#[test]
fn a_request_without_a_window_takes_its_neighbours_account_only_when_they_agree() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 30 * M, None).ask(BASE + H, Some(win_a()));
    t.ask(BASE + 2 * H, None).ask(BASE + 3 * H, Some(win_b()));
    let mut u = Thread::new(2, BASE + 3 * H, Some("acc-b"));
    u.ask(BASE + 3 * H + M, Some(win_b()));
    x.add(&[&t, &u]);
    assert_eq!(x.owner_at(BASE + 30 * M), Some(a()));
    assert_eq!(x.owner_at(BASE + 2 * H), None, "between A and B nothing is guessed");
}

#[test]
fn a_window_no_evidence_names_stays_unknown_once_two_accounts_were_in_use() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 3 * H, Some(win_a()));
    // an old thread resumed under another account, used at the same time
    let mut old = Thread::new(2, BASE - 10 * DAY_S * 1000, Some("acc-a"));
    old.ask(BASE + H, Some(win_b())).ask(BASE + 2 * H, Some(win_b()));
    x.add(&[&t, &old]);
    assert_eq!(x.owner_at(BASE + H), None, "the creator does not vouch for a resumed thread");
    assert_eq!(x.owners(), [(None, 2), (Some(a()), 2)]);
    let list = accounts(&x.store).unwrap();
    assert_eq!(list.iter().map(|l| l.id.clone()).collect::<Vec<_>>(), [Some(a()), None]);
}

#[test]
fn a_live_read_names_the_window_of_the_signed_in_account() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 3 * H, Some(win_a()));
    let mut old = Thread::new(2, BASE - 10 * DAY_S * 1000, None);
    old.ask(BASE + H, Some(win_b())).ask(BASE + 2 * H, Some(win_b()));
    x.add(&[&t, &old]);
    assert_eq!(x.owner_at(BASE + H), None);
    x.live(BASE + 2 * H + M, win_b(), Some("acc-b"));
    assert_eq!(x.owner_at(BASE + H), Some(b()), "new evidence corrects the past");
}

#[test]
fn an_unused_window_read_live_makes_no_account() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 2 * M, Some(win_a()));
    x.add(&[&t]);
    // an idle account read by a Codex that does not name it: its end moves with every read
    for i in 1..20 {
        let ms = BASE + i * 5 * M + H;
        x.live(ms, Win(ms / 1000 + 7 * DAY_S, 0.0), None);
    }
    assert!(accounts(&x.store).unwrap().is_empty(), "still one account");
    let nameless: i64 = x.store.conn().query_row("SELECT COUNT(*) FROM limit_snapshot WHERE owner IS NOT ?1", [a()], |r| r.get(0)).unwrap();
    assert_eq!(nameless, 0, "with one account its unused reads are its own");
}

#[test]
fn unused_reads_of_an_unnamed_account_are_left_out_of_the_limits() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 3 * H, Some(win_a()));
    let mut u = Thread::new(2, BASE + H, Some("acc-b"));
    u.ask(BASE + H + M, Some(win_b())).ask(BASE + 2 * H, Some(win_b()));
    x.add(&[&t, &u]);
    let ms = BASE + 4 * H;
    x.live(ms, Win(ms / 1000 + 7 * DAY_S, 0.0), None);
    let owner: Option<String> = x.store.conn().query_row("SELECT owner FROM limit_snapshot WHERE ts_ms = ?1 LIMIT 1", [ms], |r| r.get(0)).unwrap();
    assert_eq!(owner.as_deref(), Some(NAMELESS));
    let views = limits_view(&x.store, &PriceBook::default_book(), ms + M, &[]).unwrap();
    let active: Vec<_> = views.iter().filter(|v| v.active && v.window == "seven_day").collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].account, Some(a()), "the account read last by a named reading");
}

#[test]
fn the_first_request_of_a_new_window_joins_it() {
    let mut x = Archive::new();
    let start = BASE + 2 * H;
    let reset = start / 1000 + 7 * DAY_S;
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a()));
    let mut u = Thread::new(2, BASE + H, Some("acc-b"));
    u.ask(BASE + H + M, Some(win_b()));
    // A's window was reset; a thread resumed under A starts the next one at 0 %
    let mut old = Thread::new(3, BASE - 10 * DAY_S * 1000, None);
    old.ask(start, Some(Win(reset, 0.0))).ask(start + 10 * M, Some(Win(reset, 3.0)));
    x.live(start + 11 * M, Win(reset, 3.0), Some("acc-a"));
    x.add(&[&t, &u, &old]);
    assert_eq!(x.owner_at(start), Some(a()));
}

#[test]
fn a_models_own_limit_does_not_look_like_another_account() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask_limit(BASE + 30 * M, Some(Win(BASE / 1000 + 6 * DAY_S, 20.0)), "codex_premium").ask(BASE + 3 * H, Some(win_a()));
    x.add(&[&t]);
    assert!(accounts(&x.store).unwrap().is_empty());
    assert_eq!(x.owners(), [(Some(a()), 3)]);
}

#[test]
fn history_series_and_the_usage_filter_follow_the_account() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a())).ask(BASE + 2 * M, Some(win_a()));
    let mut u = Thread::new(2, BASE + H, Some("acc-b"));
    u.ask(BASE + H + M, Some(win_b()));
    x.add(&[&t, &u]);
    let h = limit_history(&x.store, BASE + 2 * H, 7).unwrap();
    let mut weekly: Vec<_> = h.series.iter().filter(|s| s.window == "seven_day").map(|s| (s.account.clone(), s.active, s.windows.len())).collect();
    weekly.sort_by_key(|w| w.1);
    assert_eq!(weekly, [(Some(a()), false, 1), (Some(b()), true, 1)]);

    let rows = x.store.events_between(0, i64::MAX).unwrap();
    let only_b = Filter { accounts: vec![account_key(Provider::OpenAI, Some(&b()))], ..Default::default() };
    assert_eq!(rows.iter().filter(|e| only_b.matches(e)).count(), 1);
    let unknown = Filter { accounts: vec![account_key(Provider::OpenAI, None)], ..Default::default() };
    assert_eq!(rows.iter().filter(|e| unknown.matches(e)).count(), 0);
    let states: Vec<LimitState> = limits_view(&x.store, &PriceBook::default_book(), BASE + 2 * H, &[]).unwrap().iter().map(|v| v.state).collect();
    assert!(states.iter().all(|s| *s != LimitState::Reset));
}

#[test]
fn a_backup_brings_the_evidence_and_the_accounts_are_derived_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + M, Some(win_a()));
    let mut u = Thread::new(2, BASE + H, Some("acc-b"));
    u.ask(BASE + H + M, Some(win_b()));
    x.add(&[&t, &u]);
    let backup = dir.path().join("backup.db");
    x.store.backup_to(&backup).unwrap();

    let mut fresh = Store::open(&dir.path().join("fresh.db")).unwrap();
    fresh.merge_from(&backup).unwrap();
    let owned: i64 = fresh.conn().query_row("SELECT COUNT(*) FROM usage_event WHERE owner IS NOT NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(owned, 0, "derived owners are never imported");
    resolve(&mut fresh).unwrap();
    assert_eq!(accounts(&fresh).unwrap().len(), 2);
}

/// A reset the provider grants to everyone starts both accounts' next windows together.
fn shared_week() -> i64 {
    BASE / 1000 + 7 * DAY_S
}

#[test]
fn a_window_two_accounts_share_after_a_reset_for_everyone_names_neither() {
    let mut x = Archive::new();
    let five_a = BASE / 1000 + 5 * 3600;
    let five_b = BASE / 1000 + 2 * 3600 + 7 * 60;
    // A and B are used side by side; their weekly windows reset at the same second
    let mut a1 = Thread::new(1, BASE, Some("acc-a"));
    let mut b1 = Thread::new(2, BASE + M, Some("acc-b"));
    for i in 0..8 {
        a1.ask_both(BASE + i * 20 * M + 2 * M, Some(Both(shared_week(), 2.0 + i as f64, five_a, 10.0 + i as f64)), "codex");
        b1.ask_both(BASE + i * 20 * M + 12 * M, Some(Both(shared_week(), 30.0 + i as f64, five_b, 40.0 + i as f64)), "codex");
    }
    // a resumed thread no creator vouches for, in A's five-hour window
    let mut old = Thread::new(3, BASE - 10 * DAY_S * 1000, None);
    old.ask_both(BASE + 70 * M, Some(Both(shared_week(), 6.5, five_a, 14.0)), "codex");
    x.add(&[&a1, &b1, &old]);
    assert_eq!(x.owner_at(BASE + 2 * M), None, "in a shared week a switch changes no window, so the creator proves nothing");
    assert_eq!(x.owner_at(BASE + 12 * M), None);
    assert_eq!(x.owner_at(BASE + 70 * M), None, "the window is shared, so it names neither account");
    let wrong: i64 = x
        .store
        .conn()
        .query_row("SELECT COUNT(*) FROM usage_event e WHERE owner IS NOT NULL AND owner != (CASE WHEN session_id LIKE '%000000000002' THEN ?2 ELSE ?1 END)", [a(), b()], |r| r.get(0))
        .unwrap();
    assert_eq!(wrong, 0, "no request is given to the other account");
}

#[test]
fn a_shared_window_with_evidence_for_one_account_is_still_not_given_to_it() {
    let mut x = Archive::new();
    let mut a1 = Thread::new(1, BASE, Some("acc-a"));
    a1.ask_both(BASE + M, Some(Both(shared_week(), 5.0, BASE / 1000 + 5 * 3600, 20.0)), "codex");
    // B, carried over in an old thread, uses its own five-hour window at the same time
    let mut old = Thread::new(2, BASE - 10 * DAY_S * 1000, None);
    let mut a2 = Thread::new(3, BASE - 10 * DAY_S * 1000, None);
    for i in 0..6 {
        old.ask_both(BASE + i * 30 * M + 5 * M, Some(Both(shared_week(), 40.0 + i as f64, BASE / 1000 + 4 * 3600, 50.0 + i as f64)), "codex");
        a2.ask_both(BASE + i * 30 * M + 20 * M, Some(Both(shared_week(), 6.0 + i as f64, BASE / 1000 + 5 * 3600, 21.0 + i as f64)), "codex");
    }
    x.add(&[&a1, &old, &a2]);
    assert_eq!(x.owner_at(BASE + 5 * M), None);
    assert_eq!(x.owner_at(BASE + 20 * M), None, "the window is shared, so A's evidence does not cover it");
    assert_eq!(x.owner_at(BASE + M), None);
}

#[test]
fn in_a_week_two_accounts_share_the_creator_names_no_request() {
    let five_a = BASE / 1000 + 5 * 3600;
    // the thread goes on under B, after two hours or after five minutes; B's five-hour window
    // differs from A's, or lines up with it as well
    for (switch, five_b) in [(2 * H, BASE / 1000 + 4 * 3600), (2 * H, five_a), (5 * M, five_a)] {
        let mut x = Archive::new();
        let mut t = Thread::new(1, BASE, Some("acc-a"));
        t.ask_both(BASE + M, Some(Both(shared_week(), 5.0, five_a, 10.0)), "codex");
        t.ask_both(BASE + switch, Some(Both(shared_week(), 70.0, five_b, 80.0)), "codex");
        x.add(&[&t]);
        x.live(BASE + switch + M, Win(shared_week(), 70.0), Some("acc-b"));
        assert_eq!(x.owner_at(BASE + switch), None, "the creator does not name the account after a switch");
        assert_eq!(x.owner_at(BASE + M), None, "nor before it: nothing tells when the switch came");
    }
}

#[test]
fn a_creator_seen_on_another_window_at_the_time_names_none_of_its_threads_requests() {
    let mut x = Archive::new();
    x.live(BASE - 6 * DAY_S * 1000, Win(BASE / 1000 - DAY_S, 30.0), Some("acc-b"));
    x.live(BASE - 5 * M, win_a(), Some("acc-a"));
    // A starts the thread, B sends its requests: A was just read on its own window
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    t.ask(BASE + 2 * M, None).ask(BASE + 5 * M, Some(win_b()));
    x.add(&[&t]);
    assert_eq!(x.owner_at(BASE + 5 * M), None);
    assert_eq!(x.owner_at(BASE + 2 * M), None, "nor the request that reports no window");

    x.live(BASE + 6 * M, win_b(), Some("acc-b"));
    assert_eq!(x.owner_at(BASE + 5 * M), None, "the creator and the live read disagree on the window");
}

#[test]
fn a_new_window_from_a_reset_the_provider_grants_keeps_its_creator() {
    let mut x = Archive::new();
    x.live(BASE - 6 * DAY_S * 1000, Win(BASE / 1000 - DAY_S, 30.0), Some("acc-b"));
    let mut before = Thread::new(1, BASE, Some("acc-a"));
    before.ask(BASE + M, Some(win_a())).ask(BASE + 30 * M, Some(Win(win_a().0, 45.0)));
    // the reset starts A's next window at once, days before the old one would have ended
    let mut after = Thread::new(2, BASE + H + M, Some("acc-a"));
    after.ask(BASE + H + 2 * M, Some(Win((BASE + H + 2 * M) / 1000 + 7 * DAY_S, 2.0)));
    x.add(&[&before, &after]);
    assert_eq!(x.owner_at(BASE + 30 * M), Some(a()));
    assert_eq!(x.owner_at(BASE + H + 2 * M), Some(a()));
}

#[test]
fn two_accounts_read_at_the_same_moment_keep_their_own_readings() {
    let mut x = Archive::new();
    x.live(BASE, win_a(), Some("acc-a"));
    x.live(BASE, win_b(), Some("acc-b"));
    let wrong: i64 = x.store.conn().query_row("SELECT COUNT(*) FROM limit_snapshot WHERE owner IS NOT account", [], |r| r.get(0)).unwrap();
    assert_eq!(wrong, 0);
    let ids: HashSet<Option<String>> = accounts(&x.store).unwrap().into_iter().map(|a| a.id).collect();
    assert_eq!(ids, HashSet::from([Some(a()), Some(b())]));
}

#[test]
fn a_reset_the_provider_grants_inside_a_window_keeps_one_account() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    let week = win_a().0;
    for (i, used) in [40.0, 42.0, 0.0, 1.0, 3.0, 6.0].iter().enumerate() {
        t.ask(BASE + i as i64 * H + M, Some(Win(week, *used)));
    }
    x.add(&[&t]);
    assert!(accounts(&x.store).unwrap().is_empty());
    assert_eq!(x.owners(), [(Some(a()), 6)]);
}

#[test]
fn an_early_reset_with_late_reports_of_the_old_window_keeps_one_account() {
    let mut x = Archive::new();
    let mut t = Thread::new(1, BASE, Some("acc-a"));
    let mut u = Thread::new(2, BASE - 10 * DAY_S * 1000, None);
    let next = (BASE + 2 * H) / 1000 + 7 * DAY_S;
    t.ask(BASE + M, Some(win_a())).ask(BASE + H, Some(win_a())).ask(BASE + 2 * H, Some(Win(next, 1.0))).ask(BASE + 3 * H, Some(Win(next, 4.0)));
    // a request that was running when the window was reset reports the old one
    u.ask(BASE + 2 * H + 4 * M, Some(win_a()));
    x.add(&[&t, &u]);
    assert!(accounts(&x.store).unwrap().is_empty(), "a late report is not a second account");
    assert_eq!(x.owners(), [(Some(a()), 5)]);
}
