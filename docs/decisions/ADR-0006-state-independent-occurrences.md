# ADR-0006 — Occurrence and observation identity is state-independent

**Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 3b
**Amends:** ADR-0002 §2 (`OccurrenceId`) · **Touches:** ADR-0003 (schema, `assertion_state`)

## Context

ADR-0002 defined physical identity as

```
OccurrenceId = blake3(entity_id, state_id, rel_path, start_byte, end_byte)
```

and schema v1 denormalized `state_id` onto `occurrence` and `observation`. Both decisions were
made in Slice 1, when every index was a full index and the cost was invisible.

Slice 3 made re-indexing incremental and the cost became the dominant term. Because the
repository state participates in the identity of every occurrence, advancing to a new state
requires rewriting **every surviving row and every index entry over it**. Measured on a
520-module repository: 1330 ms of a 2900 ms incremental run, against 8 ms of actual extraction.
`docs/reports/slice-03-report.md` records the measurement and the miss it caused (24.9% of a
full index, against a < 20% target).

The restatement pass could not simply be deleted. `nerve-core::dump` put `state_id` on
occurrence rows, observation rows and `assertion_state` rows, so an unchanged file's rows
keeping an older state while a full index gave them a new one would make the two canonical
dumps differ — and the byte-identity of those dumps is the property Slice 3's correctness rests
on. The choice was therefore between weakening the dump and fixing the identity model.

**Weakening the dump was rejected.** A dump that omits a field it can observe is no longer a
determinism guard for that field; it converts a real divergence into an invisible one.

## Decision

### 1. An occurrence is a physical location fact

> Entity `E` appears at `rel_path:start_byte..end_byte`, and the file hashed to `content_hash`
> when that was recorded.

Nothing in that sentence depends on which index run observed it. Therefore:

```
OccurrenceId = blake3(entity_id, rel_path, start_byte, end_byte)
```

`state_id` is removed from `occurrence` entirely — from the identity tuple, from the table, and
from the canonical dump.

### 2. An observation is evidence produced by an extractor over a file at a content hash

`observation.state_id` was a denormalization of a fact already reachable through
`observation.extractor_run_id → extractor_run.state_id`. It is removed from the table and from
the canonical dump; the join is retained and is what `nerve why` now reads to report the state
an observation was made in. The output surface does not change.

### 3. `content_hash` is the freshness anchor

Freshness was never read off `state_id`. `nerve why` re-hashes the file on disk and compares it
with `observation.content_hash` (`nerve-store::freshness`), which detects changes that were
never indexed at all — strictly more than a stored state could. Removing the stamp therefore
changes no product semantics.

### 4. `assertion_state` no longer names a state

`assertion_state.state_id` and `assertion_state.last_seen_state_id` are removed. They were
derived from `observation.state_id`, so retaining them would have reintroduced exactly the
whole-repository write this ADR exists to remove: an unchanged claim would keep an old state
while a full index gave it a new one.

"Which state was this claim last observed in" is still answerable, by joining
`observation → extractor_run`. It is no longer a column.

### 5. The dump keeps one state, at the top

`CanonicalDump::state_ids` previously listed the states the rows referred to. It now carries the
repository state of the **most recent extractor run** — the state the database currently
describes. That value is `content_merkle`, a pure function of the file set and file contents, so
it remains a determinism guard: an incremental run and a from-scratch run over the same tree
must still agree on it.

### 6. Entity kind is a function of entity id — a standing invariant

Every canonical tuple in ADR-0002 begins with the kind string, and every entity id carries a
kind prefix. Therefore `entity.kind` cannot change for a fixed `entity_id`, and
`assertion_state.is_unresolved` — which depends only on the target entity's kind — is fixed per
`assertion_id`.

This is what makes the scoped derivation of `assertion_state` (Slice 3b) provably equal to the
whole-table rebuild: a row depends only on that assertion's own observations. **A future entity
kind introduced without the kind in its canonical tuple would silently break scoped derivation.**
Recorded here because the dependency is not local to the code that relies on it.

## Consequences and known defects — explicitly not solved

1. **An occurrence no longer records which run wrote it.** `occurrence` has no link to
   `extractor_run`. "Which state saw this occurrence" is answerable only for observations. This
   is an accepted loss: an occurrence is a location, and a location that has not changed has not
   changed. Restoring it would require either the stamp this ADR removes or an occurrence→run
   join table, which is a temporal-layer decision (Slice 12), not this slice's.

2. **A row can outlive the state it was written in with no marker.** That is the point, and it
   is why `content_hash` must stay on both tables. A row whose file was never re-read looks
   identical to a freshly written one until the file is re-hashed. Nerve re-reads and re-hashes
   the whole tree on every index run — the repository state *is* that Merkle — so the window in
   which a row can be silently stale is bounded by "between index runs", which is exactly the
   window `nerve why`'s query-time freshness check exists to cover.

3. **Migration from v1/v2 can collapse rows, and does so deliberately.** A Slice 1/2 database
   was insert-only: re-indexing appended a second `occurrence` row for the same entity at the
   same span under a new `state_id`, and a second `observation` row for the same claim. Under
   the new identity those are the *same* row. The v3 migration keeps the most recently written
   one (highest rowid) and discards the superseded duplicates. For a Slice 3 (v2) database this
   is a no-op, because the restatement pass had already collapsed every row onto one state. It
   is lossy only for the accumulated per-state history of a v1 database, which was never
   queryable as history — nothing read it — and a re-index restores the current truth exactly.

4. **File moves still change identity.** `rel_path` is still in the tuple, so ADR-0002's known
   defect 1 is unchanged: moving a file changes every occurrence id in it, and the
   `IdentityLink` bridge from Slice 3 remains the honest, evidence-bearing answer. This ADR
   makes occurrence identity *smaller*, not more stable.

5. **Renames, overloads and anonymous-function ordering** are exactly as unsolved as ADR-0002
   left them. **Identity is not solved.** Nothing here claims otherwise; this ADR removes a
   field that never helped with identity and only ever cost writes.

6. **The uniqueness rule for observations changed shape.** The unique index was
   `(assertion_id, state_id, extractor_id, extractor_version, evidence_source_type, file_path,
   start_line, end_line)`; it is now the same tuple without `state_id`. This is a *tightening*:
   the same evidence at the same place from the same extractor is now one row across states
   rather than one row per state. That is the intended meaning, and it is what stops an
   insert-only re-index from accumulating duplicate evidence forever.

## Migration policy

ADR-0002's policy stands: a change to a canonical tuple is a breaking change requiring a
migration. Schema **v3** performs it. `occurrence_id` is recomputed in Rust during the migration
(BLAKE3 is not available inside SQLite), rows are preserved except for the deliberate collapse
described above, and the step is appended rather than editing v1 or v2 in place.
