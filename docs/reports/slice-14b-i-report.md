# Slice 14b-i — schema v10, and a rebuild v7's precedent could not stand in for

**Objective.** Close the two columns 14a left as open strings — `memory.scope` and
`memory_event.operation` — in Rust *and* in SQL, and add the transactional lifecycle writes 14a had
no room for.

**Starting HEAD:** `f1ee790` · **Ending HEAD:** `dd55152`
**Commits:** `169f37a` (plan correction, before any code) · `dd55152` (implementation)

---

## User value

A human's note can no longer be filed under a misspelled scope. That matters because `scope` sits in
both derived-view grouping keys, so a typo would have silently suppressed a conflict report — two
records answering the same named claim reported as unrelated notes, with no test able to catch it
because both spellings are legal.

---

## Scope, and what was deliberately not done

| | |
|---|---|
| **In** | `MemoryScope`, `MemoryOperation`, schema **v10**, `propose_memory`, `confirm_memory`, `invalidate_memory`, `cite_memory`, the closed `operation` parameter on `supersede_memory` |
| **Out** | the `nerve memory` command family, citations at the CLI, export, the acceptance rows — all **14b-ii**. No CLI, HTTP, MCP or UI file was touched |

---

## The decision made before the code

`scope` had no defined domain; 14a recorded that 14b decides it. Committed as `169f37a`:

**The argument for closing it is not the one that closed `status`.** `status` was closed in SQL
because a *derived* view (`potentially_stale`) could be *stored*. That confusion does not exist for
`scope`, so that argument does not transfer and was not reused. What applies instead:

- `multiple_active` groups by `(repo, subject, scope)`; `conflicted` by that plus `claim_key`. A
  typo therefore splits one group in two and **suppresses a contradiction the product would
  otherwise report**.
- `--scope opertions` returning zero records reads as *there are no notes* rather than *there is no
  such scope* — `absence is not zero`, the rule 7c-ii's `doctor` and 7b's unresolved account exist
  to enforce.

**The values are on an axis the neighbouring fields do not occupy, and the first draft duplicated
both.** Not the subject's kind (`subject_kind_snapshot` holds that — and 14a's test values `"file"`
and `"repository"` *are* that redundancy, which is why they are placeholders rather than a
vocabulary). Not the question answered (`claim_key` holds that). What remains is **which facet of
the subject the claim is about**, forced by the plan's own `owner` example: *"the payments team owns
this code"* and *"platform owns its deployment"* share a subject and a claim key and are **not** a
contradiction.

Four values, each admitted only because a real note in this repository already needs it — Slice
10's rule that a vocabulary member with no producer is documentation rather than a gate:
`implementation` · `interface` · `operations` · `process`. `ownership` was refused as a value
because *owner* is a `claim_key` in the plan's own text, and one concept may not sit on two axes.

Residual, stated rather than hidden: a retry policy is `implementation` coded and `operations`
configured, and a human may reasonably file it either way.

---

## The migration is the slice

SQLite has no `ALTER TABLE … ADD CONSTRAINT`, so a `CHECK` arrives only by create-copy-drop-rename.
`V7` did exactly that for `git_rename_hypothesis` — **and it could only because nothing referenced
that table.** `memory` is referenced by `memory_citation`, by `memory_event`, and by *itself*
through `supersedes_memory_id`.

Three facts, each **measured rather than assumed**, close off v7's shape:

1. `PRAGMA foreign_keys` is a documented **no-op inside a transaction**, and every step of
   `MIGRATIONS` runs in one — it still reads `1` after being set to `OFF` inside `BEGIN`.
2. With foreign keys on, `DROP TABLE` performs an implicit `DELETE FROM` first, orphaning every
   child row.
3. **`PRAGMA defer_foreign_keys=ON` does not rescue it**, and this is the finding worth keeping.
   Deferring moves the failure to `COMMIT`: the implicit delete increments SQLite's
   deferred-violation counter and renaming the replacement table never decrements it, so the commit
   fails with `FOREIGN KEY constraint failed` **while `PRAGMA foreign_key_check` reports zero rows**.
   The database is consistent and the commit is refused anyway — the worst available failure, landing
   outside the step that caused it, where the obvious diagnostic cannot see it.

**The procedure actually used** keeps enforcement immediate and orders statements so each leaves the
database consistent at its own conclusion: park all three tables in `TEMP`; empty the children, then
`memory` itself (**unqualified on purpose** — only a whole-table delete satisfies the
self-reference); drop; **re-create under their own names**; re-insert; drop the staging.

There is no `ALTER TABLE … RENAME` anywhere in the step, which avoids a second measured trap: since
SQLite 3.25 a rename rewrites references in *other* tables, so renaming `memory` aside silently
repoints `memory_citation`'s foreign key at the scratch table that is about to be dropped.

### Out-of-domain rows are refused, not repaired

v9 stored `scope` opaque and 14a's own tests used `"file"` and `"repository"`, so a database on disk
may genuinely hold one. `migrate_v10` checks both columns **before any DDL** and refuses with the
offending distinct values named. A migration narrowing a column has three honest options — drop the
rows, rewrite them to a default, or refuse — and for a table a **human authored** only the third is
available: memory is the one thing in this database re-indexing cannot rebuild. That check is Rust,
which is why v10 is a `Step::Rust`.

---

## Two refusals the implementation added, and why they are right

- **A proposal may not carry `supersedes_memory_id`.** The unique index would give the predecessor
  its one successor, so no later call could ever retire it. That is *unrecoverable*, not merely
  wrong.
- **Invalidation is refused from `superseded`**, in both directions. *"It stopped being true and
  nothing replaced it"* and *"this replaced it"* are contradictory claims about one record; accepting
  the second after the first would quietly turn one into the other. The pair is what keeps the two
  statuses distinguishable at all.

---

## Verification

Run by the orchestrator on a stable tree, not quoted from the implementer:

```
cargo fmt --all -- --check                            → clean
cargo clippy --workspace --all-targets -- -D warnings → 0 warnings
cargo test --workspace --no-fail-fast                 → 1713 passed, 0 failed, 2 ignored (58 targets)
cargo build --release                                 → Finished
Cargo.lock                                            → 106 packages, unchanged
```

Test count **1700 → 1713**. `SCHEMA_VERSION` **9 → 10**. `EvidenceSourceType::ALL` still **12**.

`fixtures/ts-basic/golden.json` moves by **exactly one line**, `schema_version` 9 → 10 — which is
the evidence that the rebuild perturbs nothing in the evidence graph, memory being deliberately
absent from the canonical dump.

### Mutation probes

Three, each applied, each failing a **named** test for the intended reason, each reverted and
verified byte-identical afterwards:

| probe | fails |
|---|---|
| the `scope IN (…)` CHECK deleted from `V10` | `the_database_itself_refuses_a_scope_outside_the_vocabulary` |
| `Superseded` admitted to `invalidate_memory`'s allowed set | `invalidated_and_superseded_are_refused_in_both_directions` |
| `refuse_out_of_domain` removed from `migrate_v10` | `a_scope_outside_the_v10_domain_refuses_the_migration_and_changes_nothing` |

### Anti-vacuity

The upgrade test builds a **genuine v9 database** from a hand-written v9 DDL and puts child rows in
it — a memory row, a citation, an event, and a second memory row superseding the first — because a
v10 test against an empty `memory` table would prove exactly the thing that was never in doubt. It
asserts surrogate keys (`citation_id`, `event_id`) survive unrenumbered, that the self-supersession
still resolves *through the schema* rather than by reading the column, and that the child foreign
keys still **enforce** afterwards.

The two domain-refusal tests are separate rather than one fixture violating both columns, because
the checks run in sequence and a combined fixture would pass while the second check did nothing.

---

## Safety, privacy, clean-room

- No new dependency; `Cargo.lock` unchanged at **106**.
- No new `EvidenceSourceType`, no new `Relation`. `assertion` / `observation` / `occurrence` /
  `assertion_state` byte-identical across every new operation, asserted by
  `no_lifecycle_operation_moves_a_single_byte_of_the_evidence_tables`.
- No network, no subprocess, no repository code executed. Guard files untouched.
- Clean-room: no competitor source consulted; the migration procedure is derived from SQLite's own
  documented behaviour, measured locally.

---

## Process deviations

1. **The implementation subagent was killed twice.** First silently mid-slice (recovered by
   resuming it), then terminally by an **organisation monthly spend limit**. The recorded fallback
   applied both times: inspect, preserve valid work, verify in the orchestrator. Everything after
   this slice is orchestrator-direct.
2. **This machine SIGKILLed several heavy `cargo` runs.** `nerve-store --test prune` aborted with no
   result line during one parallel workspace run and passes 6/6 in isolation and in the clean
   re-run. Attributed to resource pressure, not to this slice. One earlier full-gate run was
   **contaminated** by concurrent agent edits and discarded rather than reported.
3. **A remote was added and the repository published**, on direct user instruction of 2026-08-10.
   See `CONTINUATION.md`; this supersedes the standing local-only decision.

---

## Known limitations

- A fifth *stored* status or a fifth scope needs a table rebuild in a later migration. That is the
  intended price, stated in the DDL.
- `memory_citation` carries durable snapshots but still has **no resolution verdict** — 14a deferred
  it because nothing reads a citation's live target yet, and 14b-ii is where a reader appears.
- Subject resolution follows exactly one identity link, forwards only. A subject moved twice reports
  `missing`.

**Next:** Slice 14b-ii — the `nerve memory` command family, citations, export, and the acceptance
rows that replace `final_acceptance.sh`'s existence-only loop.
