//! Developer tool: ingest this machine's real logs into a given database and print totals.
//! Usage: cargo run --release --example scan -- <db path>
//! Prints counts only — never content.

use std::collections::HashSet;
use std::time::Instant;
use tracker_core::discovery::{detect, enumerate_files, Env, ExtraPaths, SourceId};
use tracker_core::ingest::ingest;
use tracker_core::store::Store;

fn main() {
    let db = std::env::args().nth(1).expect("usage: scan <db path>");
    let env = Env::from_system();
    for s in detect(&env, &ExtraPaths::default()) {
        println!("{:<16} found={:<5} supported={:<5} files={}", s.id.as_str(), s.found, s.supported, s.file_count);
    }
    let enabled: HashSet<SourceId> = SourceId::ALL.into_iter().collect();
    let files = enumerate_files(&env, &ExtraPaths::default(), &enabled);
    let mut store = Store::open(std::path::Path::new(&db)).unwrap();
    let t = Instant::now();
    let rep = ingest(&mut store, &files, |_| {});
    println!(
        "files seen={} read={} new events={} limits={} warnings={} errors={} in {:?}",
        rep.files_seen,
        rep.files_read,
        rep.new_events(),
        rep.limits_after - rep.limits_before,
        rep.warnings.len(),
        rep.errors.len(),
        t.elapsed()
    );
    for (_, w) in rep.warnings.iter().take(5) {
        println!("  warning: {w}");
    }
    let mut st = store
        .conn()
        .prepare(
            "SELECT tool, source, COUNT(*), SUM(input), SUM(cache_read), SUM(cache_write), SUM(output), SUM(reasoning),
                    MIN(datetime(ts_ms/1000,'unixepoch')), MAX(datetime(ts_ms/1000,'unixepoch'))
             FROM usage_event GROUP BY tool, source",
        )
        .unwrap();
    let rows = st
        .query_map([], |r| {
            Ok(format!(
                "{:<12} {:<18} events={:<6} in={:<10} cr={:<12} cw={:<10} out={:<9} reas={:<9} {}..{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?
            ))
        })
        .unwrap();
    for r in rows {
        println!("{}", r.unwrap());
    }
    for l in store.latest_limits().unwrap() {
        println!(
            "limit {:<10} {:<14} {:<10} used={:?} resets_at={:?} status={:?} plan={:?} source={}",
            l.provider.as_str(),
            l.tool.as_str(),
            l.window,
            l.used_pct,
            l.resets_at,
            l.status,
            l.plan,
            l.source
        );
    }
}
