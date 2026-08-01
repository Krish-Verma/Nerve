# Slice 3b — State normalization · completion report

**Date:** 2026-07-31 · **Status:** Complete, all acceptance criteria met
**Plan:** `docs/plans/slice-03b-state-normalization.md` · **ADR:** `ADR-0006`

---

## Summary

Repository state no longer lives on evidence rows. An occurrence is a **physical location fact**
and an observation is **evidence about a file at a content hash** — neither depends on which run
observed it. The O(repository) state-restatement pass is gone, and `assertion_state` derivation
and orphan pruning are now scoped to what actually moved.

**Slice 3's missed target is met with room to spare: 24.9% → 2.0%.**

More important than the ratio: a one-file leaf edit now writes **the same 52 rows in a 100-file
repository and in a 520-file one**, asserted by row count rather than by timing.

The slice also uncovered and fixed a **silent data-destruction bug** (below), which on its own
justified the work.

## Files changed

**New** — `docs/decisions/ADR-0006-state-independent-occurrences.md`.

**Changed** — `nerve-core`: `ids.rs` (`occurrence_id` drops `state_id`), `dump.rs` (state fields
removed from occurrence/observation/assertion_state). `nerve-store`: `schema.rs` (v3, stepwise
`Step::Sql | Step::Rust` migrations), `write.rs`, `derive.rs` (scoped derivation sharing one
statement with the whole-table oracle), `prune.rs` (`restamp_state` **deleted**, scoped pruning),
`dump.rs`, `graph.rs`, `facts.rs`, `lib.rs`. `nerve-index`: `pipeline.rs` (restamp removed,
scoped derivation/pruning, conditional directory containment, `rows_written` ledger,
**migrate-on-index**). `nerve-cli`: `main.rs`. Tests across store and index.
`ADR-0002` and `ADR-0003` gained amendment banners. `fixtures/ts-basic/golden.json` regenerated.

## ADR-0006 — the identity change

`occurrence_id = blake3(entity_id, rel_path, start_byte, end_byte)`. `state_id` leaves
`occurrence`, `observation` and `assertion_state` entirely; state linkage survives as
`observation.extractor_run_id → extractor_run.state_id`. `content_hash` is the freshness anchor,
which is already how `nerve why` worked — **product semantics are unchanged.**

Failure modes recorded honestly rather than glossed: an occurrence can no longer be attributed to
a run; a row can outlive its state with no marker, which is why `content_hash` must stay; v1→v3
deliberately collapses per-state duplicate rows and is one-way; and **identity is still not
solved** — moves and renames change ids exactly as ADR-0002 left them. This ADR removes a field
that never helped that.

## Verification — run by the orchestrator

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 306 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
cargo test -p nerve-index --test precision              → 5 passed; CALLS 13/0/0, REFERENCES 4/0/0,
                                                          EXTENDS 4/0/0, IMPLEMENTS 3/0/0
PRAGMA integrity_check                                  → ok
PRAGMA foreign_key_check                                → clean
```

296 → 306 tests. **No test was deleted or weakened**; `fixtures/ts-resolution/**` is untouched
(`git diff --stat` empty) and the precision gate still reads FP=0, FN=0.

### Golden diff — verified structurally, not by eye

```
schema_version    2 -> 3
state_ids         identical (same content Merkle)
entities    (48)  identical
assertions  (87)  identical
occurrences (46)  identical after dropping state_id
observations(89)  identical after dropping state_id
assert_state(87)  identical after dropping state_id, last_seen_state_id
keys added        none        keys removed  {state_id}
```

Every payload row is byte-identical. The change is confined to `schema_version` and the removed
state fields, exactly as the plan required.

### Adversarial probes — orchestrator's own

| Probe | Result |
|---|---|
| Remove `nerve_store::migrate` from the index path | **FAILED as required** — `indexing_migrates_a_database_that_is_not_at_the_current_schema_version` |
| Make directory containment unconditional (reintroducing an O(repository) write) | **FAILED as required** — `a one-file edit wrote 96 rows in a 100-file repository and 264 in a 520-file one` |

Both reverted, byte-verified, 306 tests green after revert.

### Counted-writes gate (the durable one)

| repository | files | rows written by a one-file leaf edit | full index |
|---|---|---|---|
| 10 clusters | 100 | **52** | 4,085 |
| 52 clusters | 520 | **52** | 21,221 |

Asserted as exact equality. Deterministic, CI-safe, and impossible to game with a fast machine.
`rows_written` counts Nerve's own model rows; the six fixed bookkeeping statements per run and
SQLite's FTS5 shadow-table maintenance are documented exclusions.

### Ratio — measured by the orchestrator, all runs

| run | load (1m) | stubs | realistic |
|---|---|---|---|
| 1 | 5.29 | 8.8% | **2.0%** |
| 2 | 5.67 | 10.3% | **2.0%** |
| 3 | 9.24 | 8.9% | **2.5%** |

Target < 20%. Slice 3 measured 24.9%. Amplification 1.00 in every run. The subagent's six runs
under loads 5.7–47.7 reported 1.8–2.5%, consistent with these. The speed test's former
`MEASURED_CEILING = 0.60` escape hatch is deleted; it now asserts the real 20% target.

## The data-destruction bug this slice found

Only `nerve init` ever migrated. On an un-migrated v2 database a v3 binary **silently destroyed
data**: `persist_batch`'s `INSERT OR IGNORE` swallows `NOT NULL` violations as readily as
duplicate keys, so every insert was dropped *after* the re-extracted files' rows had already been
deleted — 49 entities became 33, **exit code 0**.

Fixed by migrating on the index path, with a regression test. The orchestrator confirmed by
mutation that removing the line fails that test. This was outside the planned scope and was the
right call: shipping v3 without it would have been the worst outcome available.

End-to-end migration was also verified against a **genuine v2 database** built from the Slice 3
binary in a throwaway worktree: 2→3 in place, all counts preserved, integrity clean, 0 rows
written, and the resulting graph — including every recomputed `occurrence_id` — identical to a
from-scratch v3 index (365/365 rows).

## Scoped derivation — how equality is held

The whole-table rebuild remains the reference implementation and oracle, and **both paths share
one `insert_statement()`**, so they cannot drift by being two implementations. A derived row
depends only on that assertion's own observations plus its target's kind, and kind is a function
of entity id, so the scope is exact. Gated by a unit test and by a seeded 16-step mixed edit
sequence that, after **every** step, re-runs the whole-table rebuild and asserts the snapshot is
unchanged — and asserts the scoped path was actually exercised.

## Known limitations

- Read paths (`why`, `path`, `search`, `status`) do not migrate; they report `healthy: no` on a
  stale database, which is honest. Only the write path migrates — deliberately smaller blast
  radius.
- `rows_written` excludes FTS5 shadow-table maintenance (~19 rows at 100 files, ~43 at 520).
  Real work, but amortized segment merging rather than a per-run repository-proportional cost.
- A v1 database indexed more than once loses per-state duplicate history at migration. One-way;
  nothing ever read it.
- An occurrence can no longer be attributed to a run.
- The scoped pruner's completeness is checked empirically over a seeded sequence, not proved. If
  a future code path deletes observations outside `nerve-store::prune`, the scope silently
  becomes incomplete — **this is the invariant to guard when adding extractors.**
- Unchanged from Slice 3: transient read errors treat a file as removed; move proposals are
  file-level and need ≥50% symbol correspondence.

## Result

All ten acceptance criteria met, including the two Slice 3 missed.

**Next:** Slice 4 — visual explorer. Its blocking controls (T4 CSRF/token, T5 XSS, T6 source
serving) are specified in `docs/THREAT-MODEL.md`, written this session.
