# Slice 3b — Normalize repository state out of the evidence rows

**Date:** 2026-07-31 · **Status:** Accepted, in progress · **Depends on:** Slice 3

---

## Disagreements and Pushback

### P1 — The stated cause is real but **not sufficient**. There are two O(repository) costs, not one.

The brief (§4) attributes the 24.9% miss to `occurrence_id` containing `state_id` and `state_id`
being denormalized onto every row. That is confirmed from source (`ids.rs:134`, schema v1
`occurrence.state_id` / `observation.state_id`). But it is only **half** the cost.

`nerve-store/src/derive.rs:102` runs, unconditionally, on every index:

```sql
DELETE FROM assertion_state;
INSERT INTO assertion_state (...) SELECT ... FROM observation o ...
```

That is a whole-table rebuild of the derived view — **O(repository), independent of what
changed** — and removing `state_id` from occurrence identity does nothing about it.

Measured breakdown of a ~2.9 s incremental run on the 520-module realistic corpus:

| phase | cost | O(repo)? | fixed by the brief's diagnosis? |
|---|---|---|---|
| state restatement | ~1330 ms | yes | yes |
| `rebuild_assertion_state` | ~960 ms | yes | **no** |
| prune orphans | ~196 ms | yes | no |
| commit | ~226 ms | partly | no |
| read + hash | ~66 ms | yes (cheap) | no |
| **extraction** | **~8 ms** | no | — |

Removing restatement alone lands near 14–15% on the measured full-index baseline, which would
pass. But it leaves a second whole-repository pass in the hot path that will dominate again as
soon as repositories get larger, and it would be dishonest to call the architecture fixed while
it remains.

**Decision.** Slice 3b addresses **both**: state normalization *and* scoped derivation. Prune
scoping is included if it falls out cheaply; it is not the headline.

### P2 — Removing state from the rows is blocked by the canonical dump, and that is the real design problem

`nerve-core/src/dump.rs` puts `state_id` on occurrence rows (:71), observation rows (:119) and
`assertion_state` (:142, :154). So today, if an unchanged file's rows kept an old `state_id`
while a full index gave them a new one, **the dumps would differ and the Slice 3 equivalence
property would fail.** The restatement pass exists precisely to prevent that.

This means the restamp cannot simply be deleted. Either the rows stop carrying state, or the
dump stops treating state as part of a row's canonical content. **Do not "fix" this by weakening
the dump** — a dump that hides a field is no longer a determinism guard for that field.

**Decision — the identity model is wrong, and that is what gets fixed.** An occurrence is a
*physical location fact*: "entity E appears at `path:start..end`, where the file hashed to H."
Nothing about that depends on which index run observed it. Likewise an observation is *evidence
produced by an extractor over a file at a content hash*. `state_id` on these rows is a
denormalized convenience that turned into an O(repository) write amplifier.

- `occurrence_id` becomes `blake3(entity_id, rel_path, start_byte, end_byte)` — no state.
- `content_hash`, already present on both tables, is the freshness anchor. This is already how
  `nerve why` computes freshness (it re-hashes the file), so the product semantics do not change.
- "Which state saw this" stays answerable through `observation.extractor_run_id → extractor_run.state_id`
  and through `repository_state`, rather than being stamped on every row.

This is an ADR-0002 amendment and requires **ADR-0006** before implementation.

### P3 — Scoped derivation must be proven equal to the full rebuild, not assumed

ADR-0003's load-bearing invariant is that `assertion_state` is a pure function of `observation`.
Recomputing only the affected rows does **not** violate purity — a pure function may be evaluated
lazily — *provided the result is identical*.

**Decision.** Keep the existing whole-table rebuild as the **reference implementation** and the
test oracle. Add a scoped path, and gate it with a test asserting
`scoped(edits) == full_rebuild()` after arbitrary edit sequences. If they ever differ, the
scoped path is wrong and the full path stands. The existing truncate-and-rebuild test stays
unmodified.

### P4 — The 20% target is a proxy; the real requirement is "no whole-repository write for a leaf edit"

A ratio is machine- and corpus-dependent, and this machine has been running at load average
25–51 all session. The brief's own §4 states the durable requirement better: **no
full-repository row rewrite for a one-file leaf edit.** That is a structural property, testable
by counting rows written, and it cannot be gamed by a fast machine.

**Decision.** Ship **both** gates: a counted-writes assertion (structural, deterministic, CI-safe)
and the ratio (reported over ≥ 5 runs with load recorded). The counted-writes gate is the one
that must never regress.

---

## Objective

An index run writes rows proportional to what changed, not to repository size, while remaining
canonically equivalent to a full index.

## Scope

1. **ADR-0006** — occurrence/observation identity is state-independent. Amends ADR-0002.
2. **Schema v3**, additive where possible: drop `state_id` from occurrence/observation identity
   and from their canonical dump representation; retain state linkage via `extractor_run`.
   Migration from **v1 and v2** with tests from both.
3. **`occurrence_id` = `blake3(entity_id, rel_path, start_byte, end_byte)`.**
4. **Remove the state-restatement pass.**
5. **Scoped `assertion_state` derivation**, gated by equality with the full rebuild (P3).
6. **Scoped orphan pruning** if it falls out cheaply.
7. **Counted-writes gate** (P4): a one-file leaf edit in a ≥ 500-file repository writes
   O(changed) rows, asserted by count.

## Non-goals

No parallelism. No query-surface change. No new language. No change to *what* is extracted —
the precision gate and `fixtures/ts-resolution` must stay green untouched.

## Acceptance criteria

1. Full gate passes.
2. Slice 3's seeded 24-edit equivalence property passes, unmodified.
3. Multi-hop invalidation, create/update/delete/move/re-export all still correct.
4. Migration v1→v3 and v2→v3, both with rows preserved; clean database reaches v3.
5. Determinism and idempotence hold.
6. `nerve why` freshness still correct (fresh / stale / file-missing / refused / unreadable).
7. `rebuild_assertion_state` remains a pure function of `observation`; scoped == full.
8. **Counted-writes: a one-file leaf edit writes no row for an unaffected file.**
9. Realistic incremental/full median ratio **< 20%**, ≥ 5 runs, every run reported, load recorded.
10. Security properties intact; database integrity check passes.

`fixtures/ts-basic/golden.json` **will** move (occurrence ids change, schema version bumps).
That is expected here. The diff must be confined to occurrence ids, state fields and
`schema_version` — every entity, assertion and observation payload must otherwise be identical.

## Stop conditions

- If state-independent occurrences cannot preserve equivalence, **stop and record the design
  conflict**; do not weaken the dump to force it through.
- If scoped derivation cannot be proven equal to the full rebuild, ship the full rebuild and
  report the ratio honestly.
