# Slice 3 — Incremental indexing · completion report

**Date:** 2026-07-31 · **Status:** Complete, with one acceptance criterion **NOT MET**
**Plan:** `docs/plans/slice-03-incremental.md`

---

## Summary

Re-indexing now costs time proportional to what changed. The correctness defect that motivated
the slice is fixed: **deletion works.** Before this slice the pipeline only ever
`INSERT OR IGNORE`d, so a deleted file's entities and edges survived forever and the graph was
simply wrong after any removal.

The load-bearing property — an incremental re-index produces a canonical dump byte-identical to
a full index of the same tree — **holds over a seeded 24-edit sequence, checked at every step**.

**Acceptance §5 (speed < 20% of a full index) is not met on the realistic corpus: 24.9%.**
The cause is architectural, diagnosed precisely, and corrected in Slice 3b.

## Files changed

**New** — `nerve-store/src/prune.rs` (deletion, orphan pruning, state restatement),
`nerve-store/src/facts.rs`, `nerve-index/src/facts.rs` (`ModuleFacts` cache),
`nerve-index/src/incremental.rs` (change classification, invalidation closure, move proposals),
`nerve-index/tests/incremental.rs` (13 tests + 1 opt-in speed test), `fixtures/ts-incremental/`.

**Modified** — `nerve-store`: `schema.rs` (v2, stepwise migrations), `write.rs`, `query.rs`
(`importers_of`), `dump.rs`, `lib.rs`, `tests/schema.rs` (+5 migration tests).
`nerve-index`: `pipeline.rs`, `error.rs`, `lib.rs`. `nerve-core`: `dump.rs`.
`nerve-cli`: `main.rs` (`--full`, removal reporting), `tests/cli.rs`.

**`fixtures/ts-basic/golden.json` — one line.** The orchestrator diffed it: the entire change is
`"schema_version": 1 → 2`. Every entity, occurrence, assertion, observation and derived-state row
is byte-identical, and the `state_id` is unchanged. Extraction behaviour did not move.

## Schema v2 — additive

New table `module_facts(repo_id, rel_path, content_hash, language, structural_version,
reference_version, facts)`, plus `idx_module_facts_hash` and a uniqueness index on
`identity_link`. V1 DDL untouched; `migrate` is now a stepwise list.

**Why it was unavoidable:** `ExportIndex` spans the whole corpus and is an *input* to extraction,
not an output. Persisted `EXPORTS` edges are one hop deep; the re-export closure is not.
Re-extracting `app.ts` needs `barrel.ts`'s and `impl.ts`'s export maps, so without a cache you
must re-parse everything — which is the cost incremental indexing exists to remove.

Migration tests: v1-with-rows → v2 preserves every table's rows and leaves v1 DDL intact; fresh
database reaches v2 directly; re-running is a no-op on both paths; identity-link uniqueness.

Audited: `module_facts` payloads contain names, specifiers, tags, entity ids and **body
digests** — hashes, not bodies. No source text at rest.

## Verification — run by the orchestrator

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 295 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

The 2 ignored are the pre-existing scale test and the new opt-in speed test.

### Equivalence property

Seed `0x5117_3E03_1CE7_5EED`, **24 edits** spanning all six kinds (modify / rename-export / add /
delete / move / add-dependency), compared against a from-scratch index at **every** step.
**No step diverged.** The subagent additionally verified 4 more seeds.

### Gate validity — orchestrator's own mutation probe

The subagent reported 5 probes. The orchestrator ran an independent one on the highest-risk
rule, the transitive invalidation closure (plan P2): truncating the fixpoint in
`incremental.rs::invalidation_set` so it stops at direct importers.

```
FAILED  editing_a_module_behind_a_barrel_re_extracts_its_importers_transitively
        assertion failed: impl, barrel and app — and nothing else
FAILED  incremental_and_full_agree_under_a_seeded_edit_sequence
        incremental and full disagree at step 7 (seed 0x51173e031ce75eed)
```

Both gates bite. Reverted and re-verified byte-identical; 13 passed after revert.

Worth recording: the subagent reported that this probe **initially passed**, which exposed a weak
edit generator (importers only named their direct dependency, so no resolution crossed a barrel).
They rewrote the generator and added `rename-export` edits — body edits cannot detect closure
loss, because they do not change identity. That is the correct response to a probe that fails to
fail, and it is why the probe bites now.

### Deletion, verified end-to-end

Deleting `src/shapes.ts` from a 15-file repository:

```
changed        0 modified, 0 added, 1 removed, 3 re-resolved
re-extracted   4 of 14 (10 skipped unchanged)
removed        79 observations, 34 occurrences, 41 assertions, 16 entities
  gone         src/shapes.ts
entities       85 → 75
```

Rows still naming the deleted path: **0**. `Circle` correctly degrades to an honest `unresolved`
entity in `heritage.ts` rather than dangling.

### Identity links

A file move proposes `1 moved_file` + `2 moved_symbol` links with evidence
(`matched=2/2`, `file_content_hash_equal=true`, body digests). The negative case — deleting a
file while adding a same-named, different-bodied one — proposes **0 links**. Links are proposed
with evidence and recorded; nothing is silently merged.

## Acceptance §5 — NOT MET, with diagnosis

520 modules, 3 runs each, measured by the orchestrator at machine load 24.6:

| corpus | run 1 | run 2 | run 3 | median | amplification |
|---|---|---|---|---|---|
| stubs (~12 lines) | 14.2% | 37.3% | 19.3% | **19.3%** | 1.00 |
| realistic (~230 lines) | 37.9% | 23.9% | 24.9% | **24.9%** | 1.00 |

Target < 20%. Stubs meet it; the realistic corpus does not. Amplification is **1.00** — exactly
one of 520 files is re-extracted, so invalidation is not over-firing.

**Root cause, confirmed architecturally rather than accepted on report.** Extraction is ~8 ms of
a ~2.9 s incremental run. The dominant cost is *state restatement*: `nerve-core::ids::occurrence_id`
takes `state_id` as a tuple field (`ids.rs:134`), and `state_id` is denormalized onto every
`occurrence` and `observation` row. Every index run must therefore rewrite every surviving row
to carry the new state — O(repository), not O(change). Removing that pass alone lands the ratio
near 22%.

This is a consequence of ADR-0002 and the v1 schema, not of the incremental implementation.
Fixing it changes occurrence identity and needs its own migration, so it is **Slice 3b** rather
than an unreviewed change bolted onto this one.

The opt-in speed test asserts the invalidation properties strictly, prints `NOT MET`, and holds
a measured ceiling of 60% so a real regression still trips. Run with:
`cargo test -p nerve-index --test incremental --release -- --ignored --nocapture`

## Deviations from the plan

1. **P4's tombstone half was wrong and is superseded.** Retaining an observation-less assertion
   as `DELETED` would make the incremental database differ from a fresh one — the very failure
   P4 objected to — and producing that row would require `assertion_state` to be written from
   something other than `observation`. P3 is the stronger invariant, so deletion is a hard
   delete. `derive.rs` is untouched and `AssertionStatus::Deleted` stays unreachable. The plan
   document records the supersession. **The orchestrator reviewed and accepts this reasoning.**
2. **Scope item 7 (`STALE`) not delivered**, on the same argument. `nerve why` computes freshness
   at query time, which is strictly better — it detects changes that were never indexed at all.
   "What was deleted, and when" belongs to the temporal layer (Slice 12).
3. **Acceptance §5 not met** (above). Not a stop condition: the plan's stop conditions were
   equivalence failure and unsafe deletion, and neither fired.

## Clean-room, dependencies

No new dependencies — `Cargo.toml`, `Cargo.lock`, `third_party/LICENSES.md` unchanged. The
seeded PRNG is a hand-rolled xorshift64\* in the test. Zero networking crates; `no_network`
green. No competitor reference; no competitor skill invoked.

## Known limitations

- **State restatement is O(repository) per run** — the §5 miss, addressed in Slice 3b.
- **A transient read error treats that file as removed**, deleting its rows until the next
  successful run. Recoverable but noisy.
- Move proposals require ≥ 50% symbol correspondence and a strictly-best candidate; ties refuse.
  Renames *within* a file are not proposed — only file-level moves.
- `identity_link` is absent from the canonical dump by design, so the equivalence property does
  not constrain it.
- `prune_orphans` uses whole-table anti-joins (~196 ms of a 2.9 s run); scoping them to touched
  ids is an easy future win.
- No file watching or daemon; no parallel parsing (deliberately deferred — one variable at a
  time while the equivalence property is new).

## Result

Acceptance criteria 1, 2, 3, 4, 6, 7, 8, 9 met. **Criterion 5 not met (24.9% vs < 20%)**, with a
verified architectural cause and a scheduled correction.

**Next slice:** 3b — normalize repository state out of `occurrence`/`observation` and out of
`occurrence_id`, removing the restatement pass.
