//! A long file is read in rounds; each round is stored with its own checkpoint.

use std::fs;
use std::path::Path;
use tracker_core::discovery::{file_identity, DiscoveredFile, SourceId};
use tracker_core::ingest::ingest_file;
use tracker_core::sources::{set_round_records, ParserKind};
use tracker_core::store::Store;

const ROUND: usize = 10;

fn line(i: usize) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"s","uuid":"u-{i}","timestamp":"2026-09-01T08:{:02}:{:02}.000Z","cwd":"/p","message":{{"id":"msg_{i}","model":"claude-opus-5-5","usage":{{"input_tokens":{i},"output_tokens":1}}}}}}"#,
        i / 60,
        i % 60
    )
}

fn session(dir: &Path, lines: &[String]) -> DiscoveredFile {
    let path = dir.join("s.jsonl");
    fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    DiscoveredFile { file_id: file_identity(&path), path, parser: ParserKind::ClaudeCodeJsonl, source: SourceId::ClaudeCode }
}

fn totals(store: &Store) -> (i64, i64) {
    store.conn().query_row("SELECT COUNT(*), COALESCE(SUM(input), 0) FROM usage_event", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
}

fn offset(store: &Store, f: &DiscoveredFile) -> u64 {
    store.checkpoint(&f.path.to_string_lossy()).unwrap().map_or(0, |c| c.offset)
}

#[test]
fn a_failed_round_keeps_the_rounds_before_it_and_the_next_scan_finishes_without_double_counting() {
    set_round_records(ROUND);
    let dir = tempfile::tempdir().unwrap();
    let lines: Vec<String> = (1..=35).map(line).collect();
    let f = session(dir.path(), &lines);
    let mut store = Store::open_in_memory().unwrap();
    // the third round cannot be written
    store
        .conn()
        .execute_batch(
            "CREATE TEMP TRIGGER fail_round AFTER INSERT ON usage_event WHEN NEW.key LIKE '%msg_25%'
             BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .unwrap();

    assert!(ingest_file(&mut store, &f).is_err());
    let stored_two_rounds: u64 = lines[..2 * ROUND].iter().map(|l| l.len() as u64 + 1).sum();
    assert_eq!(totals(&store), (20, (1..=20).sum()));
    assert_eq!(offset(&store, &f), stored_two_rounds, "the checkpoint stops after the last stored round");

    store.conn().execute_batch("DROP TRIGGER fail_round").unwrap();
    let read = ingest_file(&mut store, &f).unwrap().unwrap();
    assert_eq!(read.records, 15);
    assert_eq!(totals(&store), (35, (1..=35).sum()));
    assert_eq!(offset(&store, &f), fs::metadata(&f.path).unwrap().len());
    assert!(ingest_file(&mut store, &f).unwrap().is_none(), "an unchanged file is not read again");

    // a rescan from the start (a replaced file) overwrites by key instead of adding
    store.conn().execute("DELETE FROM file_checkpoint", []).unwrap();
    ingest_file(&mut store, &f).unwrap().unwrap();
    assert_eq!(totals(&store), (35, (1..=35).sum()));
}

#[test]
fn damaged_lines_are_all_counted_but_only_a_sample_is_kept() {
    set_round_records(ROUND);
    let dir = tempfile::tempdir().unwrap();
    let mut lines: Vec<String> = (0..1_000).map(|_| r#"{"type":"assistant","message":{"usage":"#.to_owned()).collect();
    lines.push(line(1));
    let f = session(dir.path(), &lines);
    let mut store = Store::open_in_memory().unwrap();

    let read = ingest_file(&mut store, &f).unwrap().unwrap();
    assert_eq!(read.warning_count, 1_000);
    assert_eq!(read.warnings.len(), tracker_core::sources::MAX_WARNING_DETAILS);
    let last_bad = lines[..999].iter().map(|l| l.len() + 1).sum::<usize>();
    assert_eq!(read.warnings.last().unwrap(), &format!("invalid JSON at byte {last_bad}"), "the latest warning is kept");
    assert_eq!(totals(&store), (1, 1));
    let (count, last): (i64, String) =
        store.conn().query_row("SELECT warnings, last_warning FROM file_checkpoint", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((count, last.as_str()), (1_000, read.warnings.last().unwrap().as_str()));
}
