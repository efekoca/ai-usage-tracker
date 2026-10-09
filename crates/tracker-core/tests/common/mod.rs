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

