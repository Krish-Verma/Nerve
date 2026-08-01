# Slice 3 — Incremental indexing

**Date:** 2026-07-31 · **Status:** Accepted, in progress · **Depends on:** Slices 1, 2a, 2b

---

## Objective

Re-indexing after a change costs time proportional to **what changed**, not to repository size —
without weakening determinism, and without losing the identity of symbols that merely moved.

## User value

`nerve why` already reports `stale`. Today the only remedy is a full re-index. After this slice
the remedy is cheap, which is what makes the freshness signal actionable rather than merely
informative.

---

## Disagreements and Pushback

### P1 — "Content hashes" are already done; the real work is invalidation and deletion

ROADMAP row 3 lists "content hashes" as slice content. They exist since Slice 1:
`pipeline.rs` hashes every file with BLAKE3 and `repository_state.content_merkle` is a Merkle
over the sorted `(rel_path, content_hash)` pairs. Re-stating it as new scope would inflate the
slice with finished work.

The genuinely unsolved problems are:

1. **What must be re-extracted when file X changes.** Not just X: any module that imports X can
   have its resolution outcomes change, because Slice 2a resolves through the export map and the
   transitive re-export closure. A barrel-file edit can change resolution in modules that never
   name the edited file.
2. **Deletion.** Slice 1 and 2 only ever `INSERT OR IGNORE`. Nothing removes an entity,
   assertion or observation. Re-indexing a tree where a file was deleted currently leaves its
   entities and edges in the database forever — the graph is monotonically growing and, after a
   deletion, **wrong**. This is the most important correctness defect this slice fixes.
3. **Identity across moves** (`IdentityLink`), which master plan §3.5 names as a known defect:
   `EntityId` includes the module-relative path, so moving a file changes every symbol id in it.

**Decision.** Scope the slice around invalidation, deletion and identity links. Content hashing
is a dependency, not a deliverable.

### P2 — The importer-invalidation set must be computed from the graph, not guessed

The naive rule "re-extract X and everything that imports X" is **not sufficient** for Slice 2a's
resolver. `exports.rs` computes a transitive re-export closure, so:

```
app.ts  imports { helper } from './barrel'
barrel.ts  export * from './impl'
impl.ts  export function helper() {}
```

Editing `impl.ts` changes what `app.ts` resolves, and `app.ts` never mentions `impl.ts`.

**Decision.** The invalidation set is the **reverse-reachable set over `IMPORTS`** from the
changed files, computed from the stored graph, not from a re-parse. `IMPORTS` edges are already
persisted and indexed (`idx_assertion_target`). Depth is unbounded but the set is capped by
repository size; measure it and report the amplification factor on the fixtures.

Anything short of this silently produces stale resolution, which is worse than a slow re-index
because it is invisible.

### P3 — Determinism must be proven *equal to a full index*, not merely stable

ARCHITECTURE.md invariant 5 currently says two full indexes are byte-identical. That is no
longer sufficient. The load-bearing property becomes:

> For any sequence of edits, an incremental re-index produces a canonical dump **byte-identical
> to a full index of the same final tree.**

This is the single most important test in the slice and it must be property-style, not one
hand-picked case: apply a randomized-but-seeded sequence of edits (modify, add, delete, move)
and compare against a from-scratch index at every step.

If incremental and full ever disagree, incremental is wrong by definition. There is no
interpretation in which the cheap path is the correct one.

### P4 — Deletion must not silently destroy evidence history

Deleting rows is the first destructive operation in the product (CLAUDE.md §13, build prompt
§13). Two options:

| Option | Behaviour |
|---|---|
| **Hard delete** | Remove entities/assertions/observations for vanished files |
| **Tombstone** | Keep rows; mark `assertion_state.status = DELETED`, which the vocabulary already reserves |

`AssertionStatus::Deleted` exists in the vocabulary already and is documented as
"explicitly retracted. Not reachable in Slice 1." The evidence model's whole premise is that
observations are the record and derived state is disposable.

**Decision.** Observations for a vanished file are **removed** (they assert something about a
file that no longer exists at this state, and keeping them would make freshness meaningless),
but the derived `assertion_state` is rebuilt and an assertion with no surviving observation
becomes `DELETED` rather than vanishing silently. Entities with no remaining occurrence are
removed. This keeps `rebuild_assertion_state` a pure function of `observation` — the invariant
must not be weakened to accommodate deletion.

> **SUPERSEDED during implementation — the tombstone half of this decision was wrong.**
>
> P4's hybrid contradicts P3. A full index of the final tree never creates the retained
> assertion row at all, so keeping a `DELETED` tombstone makes the incremental database differ
> from a fresh one — which is precisely the "monotonically growing and wrong" failure P4 itself
> objected to. Worse, producing a `DELETED` row for an assertion with **no** observation would
> require `assertion_state` to be written from something other than the observation table.
>
> P3 is the stronger invariant: *if incremental and full disagree, incremental is wrong by
> definition.* **Deletion is therefore a hard delete.** `derive.rs` is untouched,
> `AssertionStatus::Deleted` remains unreachable, and removals are reported loudly by
> `nerve index` instead of being recorded as tombstones.
>
> Scope item 7 (`STALE`) falls to the same argument: reaching it requires retaining
> observations at superseded states, which is exactly what breaks equivalence. `nerve why`
> already computes freshness by re-hashing at query time, which is a strictly better signal —
> it detects changes that have not been indexed at all. "What was deleted, and when" is a
> temporal-layer question (Slice 12), not a current-state-graph question.

`nerve index` must **report** what it removed. Silent deletion is not acceptable output.

### P5 — Parallel parsing is still not this slice's job

Master plan §3.4 / open decision 4 deferred parallelism to "Slice 3 with an ordered merge".
Incremental indexing already delivers the speed the roadmap wanted, and introducing parallelism
in the same slice as deletion and invalidation would confound determinism failures between two
new causes. If P3's equivalence property is ever going to catch a real bug, it needs one
variable at a time.

**Decision.** Serial. Parallelism becomes its own slice, gated on the equivalence property
already being green.

---

## Scope

1. **Change detection** — compare the previous `repository_state`'s `(rel_path, content_hash)`
   set against the current scan. Classify each file: `unchanged`, `modified`, `added`, `removed`.
2. **Invalidation set** — changed files plus their reverse-reachable closure over `IMPORTS`
   (P2), computed from the stored graph.
3. **Selective extraction** — parse and extract only the invalidation set. Both extractors
   (`ts-js-structural`, `ts-js-reference`) run over that set.
4. **Deletion** — remove observations, occurrences, orphaned entities and assertions belonging
   to removed files and to re-extracted files' superseded rows. Rebuild `assertion_state`;
   assertions with no surviving observation become `DELETED` (P4).
5. **`IdentityLink`** — populate the table created-but-unused since Slice 1. A move is proposed
   when a removed file and an added file yield symbols with matching
   `(kind, name, scope_path, body-shape)` evidence. Links are **proposed with evidence, never
   silently merged** (ARCHITECTURE.md extension point 3). `link_kind` and the evidence JSON
   record why.
6. **CLI** — `nerve index` reports files re-extracted, skipped-unchanged, removed, the
   invalidation amplification factor, and identity links proposed. A `--full` flag forces a
   complete re-index.
7. **Freshness/staleness** — `assertion_state.status = STALE` becomes reachable for assertions
   last seen in an older state, which is what `last_seen_state_id` was built for.

## Non-goals

No parallel parsing (P5). No file watching / daemon. No cross-repository work. No new language.
No schema change **unless** change detection requires persisting the previous file set — if the
existing `occurrence` + `repository_state` tables are insufficient, a migration is in scope, but
it must be additive and shipped with migration tests from v1.

## Acceptance criteria

1. Full verification gate passes.
2. **Equivalence property (P3).** For a seeded sequence of ≥ 20 mixed edits (modify / add /
   delete / move), an incremental re-index produces a canonical dump byte-identical to a full
   index of the same tree, checked at **every** step.
3. **Deletion correctness.** After deleting a file, no entity, assertion or observation
   belonging to it remains queryable, and `nerve status` counts drop accordingly. Pinned test.
4. **Invalidation soundness.** A test with a barrel-file chain (`app → barrel → impl`) proves
   that editing `impl` re-resolves `app` (P2). A resolution that changes must not survive.
5. **Speed.** On a synthetic repository of ≥ 500 files, a single-file edit re-indexes in
   **< 20%** of the full-index wall time. Report the measured ratio and the amplification factor.
6. **Idempotence preserved.** Re-indexing an unchanged tree does no extraction work and does not
   grow any table.
7. **Identity links.** A file move proposes an `IdentityLink` with evidence; a coincidental
   name match across unrelated files does **not**. Negative fixture required.
8. `rebuild_assertion_state` remains a pure function of `observation` — the existing
   truncate-and-rebuild test still passes, unmodified.
9. Precision and golden tests unchanged: this slice must not alter *what* is extracted, only
   *when*. `fixtures/ts-basic/golden.json` must not move.

## Stop conditions

- If incremental and full indexes disagree and the cause is not quickly identifiable, **ship the
  full-index path as the default** and keep incremental behind a flag until it is proven. A fast
  wrong answer is the worst possible outcome for this product.
- If deletion cannot be made safe without violating the pure-rebuild invariant, stop and record
  the design conflict rather than weakening the invariant.
