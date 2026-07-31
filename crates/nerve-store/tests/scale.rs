//! Scale and latency harness for ADR-0001's pre-registered falsification trigger.
//!
//! ADR-0001 says: revisit SQLite if p95 depth-4 traversal exceeds 200 ms on a 2,000,000
//! assertion synthetic graph, or FTS5 symbol lookup exceeds 20 ms at 200,000 entities.
//!
//! Two traversals are measured against the same budget: the recursive-CTE reachability count
//! from Slice 1, and Slice 2b's shipped [`find_paths`] walk, which is what `nerve path`
//! actually runs.
//!
//! Ignored by default because building the graph takes tens of seconds. Run it with:
//!
//! ```text
//! cargo test --workspace -- --ignored scale --nocapture
//! ```

use std::time::{Duration, Instant};

use nerve_store::{find_paths, migrate, open, PathQuery};

const ENTITIES: usize = 200_000;
const EDGES_PER_ENTITY: usize = 10;
const ASSERTIONS: usize = ENTITIES * EDGES_PER_ENTITY;
const TRAVERSAL_SAMPLES: usize = 200;
const FTS_SAMPLES: usize = 200;
/// The path walk is orders of magnitude more work per sample than the reachability count, so
/// it takes fewer samples. 50 still puts a real observation at the p95 index.
const PATH_SAMPLES: usize = 50;
const TRAVERSAL_DEPTH: usize = 4;

const TRAVERSAL_P95_BUDGET: Duration = Duration::from_millis(200);
const FTS_P95_BUDGET: Duration = Duration::from_millis(20);

/// Deterministic generator so the synthetic graph is reproducible across runs.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[test]
#[ignore = "scale harness: builds a 2,000,000 assertion graph"]
fn scale_bounded_traversal_and_fts_latency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scale.db");
    let conn = open(&path).unwrap();
    migrate(&conn).unwrap();

    // Bulk-load settings. Synthetic data we generate ourselves does not need referential
    // checking, and the indexes are rebuilt afterwards so the final schema is unchanged.
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.pragma_update(None, "synchronous", "OFF").unwrap();
    conn.pragma_update(None, "cache_size", -262_144i64).unwrap();
    conn.execute_batch(
        "DROP INDEX idx_assertion_source;
         DROP INDEX idx_assertion_target;
         DROP INDEX idx_assertion_repo_relation;",
    )
    .unwrap();

    let build_started = Instant::now();
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES ('scale-repo', 'scale', '/scale', 'now')",
        [],
    )
    .unwrap();

    {
        let tx = conn.unchecked_transaction().unwrap();
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language)
                     VALUES (?1, 'scale-repo', 'function', ?2, ?3, 'typescript')",
                )
                .unwrap();
            for index in 0..ENTITIES {
                stmt.execute(rusqlite_params(index)).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let entities_built = build_started.elapsed();

    {
        let tx = conn.unchecked_transaction().unwrap();
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO assertion
                         (assertion_id, repo_id, source_entity_id, relation, target_entity_id)
                     VALUES (?1, 'scale-repo', ?2, 'CALLS', ?3)",
                )
                .unwrap();
            let mut rng = Lcg(0x5EED_1234_ABCD_0001);
            let mut assertion_index = 0usize;
            for source in 0..ENTITIES {
                let source_id = format!("e{source:07}");
                for _ in 0..EDGES_PER_ENTITY {
                    let target = (rng.next() as usize) % ENTITIES;
                    stmt.execute(rusqlite::params![
                        format!("a{assertion_index:07}"),
                        source_id,
                        format!("e{target:07}")
                    ])
                    .unwrap();
                    assertion_index += 1;
                }
            }
            assert_eq!(assertion_index, ASSERTIONS);
        }
        tx.commit().unwrap();
    }

    conn.execute_batch(
        "CREATE INDEX idx_assertion_source        ON assertion(source_entity_id, relation);
         CREATE INDEX idx_assertion_target        ON assertion(target_entity_id, relation);
         CREATE INDEX idx_assertion_repo_relation ON assertion(repo_id, relation);
         ANALYZE;",
    )
    .unwrap();
    let build_total = build_started.elapsed();

    let entity_rows: i64 = conn
        .query_row("SELECT count(*) FROM entity", [], |row| row.get(0))
        .unwrap();
    let assertion_rows: i64 = conn
        .query_row("SELECT count(*) FROM assertion", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entity_rows as usize, ENTITIES);
    assert_eq!(assertion_rows as usize, ASSERTIONS);

    // ---- bounded traversal -------------------------------------------------------------
    let traversal_sql = format!(
        "WITH RECURSIVE reachable(entity_id, depth) AS (
             SELECT ?1, 0
             UNION
             SELECT a.target_entity_id, r.depth + 1
               FROM reachable r
               JOIN assertion a ON a.source_entity_id = r.entity_id
              WHERE r.depth < {TRAVERSAL_DEPTH}
         )
         SELECT count(*) FROM reachable"
    );
    let mut traversal = conn.prepare(&traversal_sql).unwrap();

    let mut rng = Lcg(0xC0FF_EE00_1234_5678);
    let mut reached_total = 0i64;
    let mut traversal_times = Vec::with_capacity(TRAVERSAL_SAMPLES);
    for _ in 0..TRAVERSAL_SAMPLES {
        let start = format!("e{:07}", (rng.next() as usize) % ENTITIES);
        let began = Instant::now();
        let reached: i64 = traversal.query_row([&start], |row| row.get(0)).unwrap();
        traversal_times.push(began.elapsed());
        reached_total += reached;
    }
    traversal_times.sort();

    // ---- the shipped path walk -------------------------------------------------------------
    //
    // The recursive CTE above measures reachability. `nerve path` does something strictly
    // harder: it enumerates simple paths and reconstructs them. Slice 2b's budget claim is
    // about that function, so that function is what is measured here. Most sampled pairs are
    // unreachable within four hops in a random 10-out-degree graph, which is the worst case:
    // the walk explores the whole depth-4 neighbourhood before answering "no path".
    let path_query = PathQuery {
        max_depth: TRAVERSAL_DEPTH,
        ..PathQuery::default()
    };
    let mut path_times = Vec::with_capacity(PATH_SAMPLES);
    let mut paths_found = 0usize;
    let mut expansions_total = 0usize;
    for _ in 0..PATH_SAMPLES {
        let from = format!("e{:07}", (rng.next() as usize) % ENTITIES);
        let to = format!("e{:07}", (rng.next() as usize) % ENTITIES);
        let began = Instant::now();
        let report = find_paths(&conn, &from, &to, &path_query).unwrap();
        path_times.push(began.elapsed());
        paths_found += report.paths.len();
        expansions_total += report.expansions;
        assert!(
            !report.truncated,
            "the depth-{TRAVERSAL_DEPTH} walk must fit inside its budget, not be cut off by it"
        );
    }
    path_times.sort();

    // ---- FTS lookup ----------------------------------------------------------------------
    let mut fts = conn
        .prepare(
            "SELECT e.entity_id, e.name
               FROM entity_fts
               JOIN entity e ON e.rowid = entity_fts.rowid
              WHERE entity_fts MATCH ?1
              LIMIT 20",
        )
        .unwrap();
    let mut fts_times = Vec::with_capacity(FTS_SAMPLES);
    let mut fts_hits = 0usize;
    for _ in 0..FTS_SAMPLES {
        let needle = format!("\"symbol_{}\"*", (rng.next() as usize) % ENTITIES);
        let began = Instant::now();
        let rows = fts
            .query_map([&needle], |row| row.get::<_, String>(0))
            .unwrap()
            .count();
        fts_times.push(began.elapsed());
        fts_hits += rows;
    }
    fts_times.sort();

    let database_bytes = nerve_store::database_bytes(&path);

    println!("\n=== Nerve scale harness (ADR-0001 falsification trigger) ===");
    println!("entities                {ENTITIES}");
    println!("assertions              {ASSERTIONS}");
    println!("database bytes          {database_bytes}");
    println!(
        "build time              {:.1} s (entities {:.1} s)",
        build_total.as_secs_f64(),
        entities_built.as_secs_f64()
    );
    println!(
        "traversal depth-{TRAVERSAL_DEPTH}       {TRAVERSAL_SAMPLES} samples, mean fan-out {} rows",
        reached_total / TRAVERSAL_SAMPLES as i64
    );
    println!(
        "  p50 {:8.2} ms   p95 {:8.2} ms   max {:8.2} ms   budget {:.0} ms",
        millis(percentile(&traversal_times, 0.50)),
        millis(percentile(&traversal_times, 0.95)),
        millis(*traversal_times.last().unwrap()),
        millis(TRAVERSAL_P95_BUDGET)
    );
    println!(
        "nerve path depth-{TRAVERSAL_DEPTH}      {PATH_SAMPLES} samples, {paths_found} path(s) \
         found, mean {} expansions",
        expansions_total / PATH_SAMPLES
    );
    println!(
        "  p50 {:8.2} ms   p95 {:8.2} ms   max {:8.2} ms   budget {:.0} ms",
        millis(percentile(&path_times, 0.50)),
        millis(percentile(&path_times, 0.95)),
        millis(*path_times.last().unwrap()),
        millis(TRAVERSAL_P95_BUDGET)
    );
    println!("fts lookup              {FTS_SAMPLES} samples, {fts_hits} rows returned");
    println!(
        "  p50 {:8.2} ms   p95 {:8.2} ms   max {:8.2} ms   budget {:.0} ms",
        millis(percentile(&fts_times, 0.50)),
        millis(percentile(&fts_times, 0.95)),
        millis(*fts_times.last().unwrap()),
        millis(FTS_P95_BUDGET)
    );
    println!();

    let traversal_p95 = percentile(&traversal_times, 0.95);
    let fts_p95 = percentile(&fts_times, 0.95);
    assert!(
        traversal_p95 < TRAVERSAL_P95_BUDGET,
        "ADR-0001 falsification trigger fired: p95 depth-{TRAVERSAL_DEPTH} traversal {:.2} ms \
         exceeds the {:.0} ms budget",
        millis(traversal_p95),
        millis(TRAVERSAL_P95_BUDGET)
    );
    let path_p95 = percentile(&path_times, 0.95);
    assert!(
        path_p95 < TRAVERSAL_P95_BUDGET,
        "ADR-0001 falsification trigger fired: p95 depth-{TRAVERSAL_DEPTH} `nerve path` walk \
         {:.2} ms exceeds the {:.0} ms budget",
        millis(path_p95),
        millis(TRAVERSAL_P95_BUDGET)
    );
    assert!(
        fts_p95 < FTS_P95_BUDGET,
        "ADR-0001 falsification trigger fired: p95 FTS lookup {:.2} ms exceeds the {:.0} ms \
         budget at {ENTITIES} entities",
        millis(fts_p95),
        millis(FTS_P95_BUDGET)
    );
}

fn rusqlite_params(index: usize) -> [String; 3] {
    [
        format!("e{index:07}"),
        format!("symbol_{index}"),
        format!("module_{}", index % 1000),
    ]
}
