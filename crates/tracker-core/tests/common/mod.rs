#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracker_core::discovery::{enumerate_files, Env, ExtraPaths, SourceId};
use tracker_core::ingest::ingest;
use tracker_core::store::Store;

pub const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
pub const CODEX_MAIN: &str = "rollout-2026-09-01T10-00-00-0199aaaa-bbbb-7ccc-8ddd-eeeeffff0001.jsonl";
pub const CODEX_SUB: &str = "rollout-2026-09-01T10-02-00-0199aaaa-bbbb-7ccc-8ddd-eeeeffff0002.jsonl";

pub struct Machine {
    pub _dir: tempfile::TempDir,
    pub env: Env,
    pub home: PathBuf,
    pub roaming: PathBuf,
}

pub fn put(src: &str, dst: &Path) {
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::copy(Path::new(FIX).join(src), dst).unwrap();
}

/// A profile with a different user name (non-ASCII + space) and every supported tool.
pub fn machine() -> Machine {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Users").join("Başka Kullanıcı");
    let roaming = home.join("AppData").join("Roaming");
    let local = home.join("AppData").join("Local");
    fs::create_dir_all(&local).unwrap();

    let proj = home.join(".claude").join("projects").join("C--Projects-demo-app");
    put("claude/session-a.jsonl", &proj.join("session-a.jsonl"));
    put("claude/session-b.jsonl", &proj.join("session-b.jsonl"));

    let cw = roaming.join("Claude").join("local-agent-mode-sessions").join("org-1").join("user-1").join("local_s1");
    put("cowork/c1.jsonl", &cw.join(".claude").join("projects").join("D--Work-notes").join("c1.jsonl"));
    put("cowork/audit.jsonl", &cw.join("audit.jsonl"));
    put("claude_desktop/plan-usage-history.json", &roaming.join("Claude").join("plan-usage-history.json"));

    let codex = home.join(".codex");
    put(&format!("codex/{CODEX_MAIN}"), &codex.join("sessions/2026/09/01").join(CODEX_MAIN));
    put(&format!("codex/{CODEX_SUB}"), &codex.join("archived_sessions/2026/09/01").join(CODEX_SUB));

    let env = Env { home: Some(home.clone()), roaming: Some(roaming.clone()), local: Some(local), ..Default::default() };
    Machine { _dir: dir, env, home, roaming }
}

pub fn all_sources() -> HashSet<SourceId> {
    SourceId::ALL.into_iter().collect()
}

/// `TRACKER_TEST_ROUND=1` reads every file one record per round (CI runs the suites both ways).
pub fn use_test_round() {
    if let Some(n) = std::env::var("TRACKER_TEST_ROUND").ok().and_then(|v| v.parse().ok()) {
        tracker_core::sources::set_round_records(n);
    }
}

pub fn run(store: &mut Store, env: &Env) -> tracker_core::ingest::IngestReport {
    use_test_round();
    let files = enumerate_files(env, &ExtraPaths::default(), &all_sources());
    ingest(store, &files, |_| {})
}

/// A time zone with daylight saving time: `std` seconds east of UTC, and `dst` from `start` until
/// `end` (UTC seconds). Tests need no time zone database this way.
#[derive(Debug, Clone, Copy)]
pub struct Dst {
    pub std: i32,
    pub dst: i32,
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct DstOffset {
    zone: Dst,
    fixed: chrono::FixedOffset,
}

impl chrono::Offset for DstOffset {
    fn fix(&self) -> chrono::FixedOffset {
        self.fixed
    }
}

impl Dst {
    /// Central European time in 2026: clocks go forward at 02:00 on 29 March and back at 03:00
    /// on 25 October.
    pub fn europe() -> Dst {
        Dst { std: 3600, dst: 7200, start: 1_774_746_000, end: 1_792_890_000 }
    }

    fn offset_at(&self, utc_secs: i64) -> i32 {
        if (self.start..self.end).contains(&utc_secs) { self.dst } else { self.std }
    }

    fn offset(&self, secs: i32) -> DstOffset {
        DstOffset { zone: *self, fixed: chrono::FixedOffset::east_opt(secs).unwrap() }
    }
}

impl chrono::TimeZone for Dst {
    type Offset = DstOffset;

    fn from_offset(offset: &DstOffset) -> Dst {
        offset.zone
    }

    fn offset_from_local_date(&self, local: &chrono::NaiveDate) -> chrono::MappedLocalTime<DstOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &chrono::NaiveDateTime) -> chrono::MappedLocalTime<DstOffset> {
        let wall = local.and_utc().timestamp();
        // the offsets under which this wall time exists, the earliest instant first
        let mut fits: Vec<i32> = [self.std, self.dst].into_iter().filter(|&o| self.offset_at(wall - i64::from(o)) == o).collect();
        fits.sort_unstable_by(|a, b| b.cmp(a));
        fits.dedup();
        match fits[..] {
            [] => chrono::MappedLocalTime::None,
            [o] => chrono::MappedLocalTime::Single(self.offset(o)),
            [a, b, ..] => chrono::MappedLocalTime::Ambiguous(self.offset(a), self.offset(b)),
        }
    }

    fn offset_from_utc_date(&self, utc: &chrono::NaiveDate) -> DstOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, utc: &chrono::NaiveDateTime) -> DstOffset {
        self.offset(self.offset_at(utc.and_utc().timestamp()))
    }
}

