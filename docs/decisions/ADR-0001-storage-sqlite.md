# ADR-0001 — SQLite as the local store

**Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 1

## Context

Nerve needs local, offline, single-file, crash-safe storage supporting incremental upserts,
full-text symbol search, bounded graph traversal, source locations, evidence observations,
repository states, and schema migrations.

## Decision

Use **SQLite** via `rusqlite` with the `bundled` feature (SQLite compiled from source and
statically linked), WAL journaling, and an FTS5 external-content virtual table for symbol search.

## Rationale

- **Bundling removes platform variance.** The SQLite version and compiled feature set are
  fixed by our build, not by whatever the host OS ships. FTS5 in particular is a compile-time
  option; relying on the system library would make availability machine-dependent.
- **The workload is relational, not graph-shaped.** Our dominant queries are *bounded*
  traversal (2–4 hops) combined with heavy predicate filtering on evidence source type,
  repository state, and extractor version. Predicate filtering over indexed columns is what a
  relational engine does best. Recursive CTEs cover the traversal.
- **Operational properties we need for free:** WAL gives concurrent readers with one writer;
  `PRAGMA integrity_check` gives corruption detection; the backup API and single-file layout
  give trivial portability and backup.
- **Zero install cost** for the end user, which offline-first demands.

## Alternatives rejected

- **Embedded graph databases.** Rejected for Slice 1 on maturity and distribution grounds:
  the leading MIT-licensed embedded property-graph engine in this space is archived, and its
  successor is pre-1.0 with a single maintainer and a native-addon build. Neither risk is
  worth taking before we have measured that SQLite is insufficient.
- **DuckDB.** Columnar and analytics-shaped; poor fit for the high-frequency small-row upserts
  that incremental indexing produces.
- **Server databases (Postgres, Neo4j).** Require a server process. Disqualified by offline-first.

## Consequences

- Graph traversal is our own code (recursive CTE or iterative frontier expansion), not a
  database feature. Acceptable at our depth bounds.
- If traversal becomes the bottleneck we add a materialized adjacency table before we consider
  changing engines.

## Falsification trigger (pre-registered)

Revisit this decision if the Slice 1 scale test shows **p95 depth-4 traversal > 200 ms on a
2,000,000-assertion synthetic graph**, or FTS5 symbol lookup > 20 ms at 200,000 entities.

## Verification

`nerve-store` includes a test asserting FTS5 is available in the bundled build
(`CREATE VIRTUAL TABLE ... USING fts5` succeeds) — availability is proven, not assumed.

### Measured result — Slice 1, 2026-07-31

`cargo test -p nerve-store --release -- --ignored --nocapture` on Apple Silicon, macOS 25.5:

```
entities            200,000
assertions        2,000,000
database bytes    464,005,824
build time              7.2 s
traversal depth-4  200 samples, mean fan-out 10,834 rows
  p50 11.28 ms   p95 23.05 ms   max 33.17 ms   budget 200 ms   PASS
fts lookup         200 samples, 339 rows returned
  p50  0.12 ms   p95  0.27 ms   max  1.67 ms   budget  20 ms   PASS
```

**The falsification trigger did not fire.** Traversal p95 is 8.7× under budget and FTS p95 is
74× under. The decision stands; revisit only if a future measurement crosses the thresholds
above. Note the database is ~464 MB at 2M assertions (~230 bytes/assertion), which is the
figure to watch for monorepo-scale repositories.
