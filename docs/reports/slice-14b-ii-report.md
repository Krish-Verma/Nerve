# Slice 14b-ii — `nerve memory`, and the empty pass deleted in the same commit

**Objective.** Surface 14b-i's transactional store as a command family, ship deterministic export,
and replace the acceptance script's existence-only `memory` row **in the landing commit** rather
than after it.

**Starting HEAD:** `a466c25` · **Ending HEAD:** `bdc87fa`
**Commits:** `a466c25` (export format decided, before code) · `bdc87fa` (implementation)

---

## User value

A human can now write a note, confirm it, replace it, retire it, cite a passage, search their notes
and back them all up — and every one of those is a deliberate local CLI act, which is the whole
control this row rests on. There is no `delete`, by design.

---

## What shipped

Ten verbs: `propose confirm supersede invalidate cite list show search events export`.

**No lifecycle logic in the CLI.** Every status change goes through `nerve-store`'s transactions,
so a surface cannot invent a state the store would refuse. The CLI's job is parsing, resolution,
rendering and refusal.

| decision | why |
|---|---|
| a supersession successor enters `proposed`, not `active` | `confirm_memory` is the only door into `active`; inserting past it would produce a confirmed record whose history never says it was confirmed |
| `--author` defaults to `local`, not `$USER` | a default that looks like an identity invites the reading the column exists to refuse. Said in the help text and in every human rendering |
| unknown `--scope` / `--status` parsed by hand, not `ValueEnum` | so a `--json` caller gets a JSON refusal object rather than clap text on stderr. Exit `USAGE` either way |
| `cite --file` stores a path and span with **no** entity id | 14a shipped citation snapshots and deferred the resolution verdict; naming an entity would be a claim nothing re-checks |

---

## The acceptance defect, fixed where the plan said to fix it

`scripts/final_acceptance.sh` awarded a **PASS for a command's mere existence**. It printed
`NOT BUILT` while `nerve memory` did not exist and would have printed `PASS — nerve memory exists`
the moment it did — the empty pass `2dc3a7d` had to delete for `history`, arriving on schedule for
row 14.

Deleted here, and replaced by a `REFUSED` row for `memory delete` plus **section 4f**, eight
behaviour checks: proposal persists · confirm changes the lifecycle and records it · supersede keeps
the predecessor **and all four of its events** · invalidation stays distinct from supersession in one
read · reads leave the database byte-identical · export contains what was written with its history ·
export byte-identical twice · a misspelled scope refused while a legal empty one is not.

**75 → 83 passed, 0 failed, 0 skipped.** The `REFUSED` row scores zero and was proved non-vacuous by
pointing it at `memory show`, which makes it FAIL.

---

## A 14b-i defect, found by building on it

`supersede_memory` reached the raw `insert_memory`, which appends nothing. So a record born by
supersession arrived with an **empty audit history**, while an otherwise identical record born by
`propose_memory` arrived with a `proposed` event. `nerve memory events <successor>` printed nothing,
and the record could not say it had ever been written.

§4's *"every mutating lifecycle operation appends a typed event"* carves out no exception for the
operation that **creates** a record. The successor's creating event is now appended inside the same
transaction: two records change in a supersession, so two events are recorded, and reading either
record's history tells the whole of what happened to it.

Four anti-vacuity event-count floors across three files moved by exactly one, each with the reason
written beside it. **The implementer found this and recorded it in a test asserting the empty
history rather than papering over it** — which is precisely why it was findable, and is the
behaviour to keep.

---

## Two documented claims that were false

The plan's §7.6 and `CONTINUATION.md` both said a CLI-surface test holds `nerve affected` and
`nerve trace-tests` refused and would fail if a forbidden command appeared. **There is no such
test.**

- `affected` appears in **no** test under `crates/nerve-cli/tests/`.
- `trace-tests` appears only in doc comments and in `scripts/final_acceptance.sh`.

So the only mechanism holding those two refusals is the acceptance script's `refused` helper — **a
script a developer may not run, not a gate.** Rather than inherit the assumption, 14b-ii built both
halves for its own verb: `there_is_no_delete_verb` (a real assertion rejecting `delete`, `remove`,
`purge` and `forget`, checking the help listing, and confirming the record survives) **and** the
acceptance row.

> **Carried defect.** `nerve affected` and `nerve trace-tests` are two of this product's most
> load-bearing refusals and are still held only by the acceptance script. They deserve the test this
> criterion assumed they already had. Recorded rather than fixed here, because widening a refusal
> guard is not this slice's subject.

---

## Export

One JSON document, `format_version` 1, deterministic by `serde_json`'s `BTreeMap` key ordering.

**Three deliberate omissions**, each a test rather than a note (`a466c25`):

1. **No `exported_at`** — §7.4e promised byte-identical twice, and a timestamp cannot coexist with
   that. The plan's own *"exported-at metadata if useful"* is declined in determinism's favour.
2. **No derived state** — `potentially_stale`, `conflicted`, `multiple_active`, subject resolution
   and `current_state_id` are query-time. Exporting one writes a query's answer into a file that
   outlives the query, undoing the stored-versus-derived split §3 spent its length establishing.
3. **No absolute path** — asserted by searching the output for the repository root.

`anchor_state_id` **is** exported, against the brief's sketch: it is a stored column, not a verdict,
and dropping it would make the one artefact protecting unrecoverable data lossy about the field
every staleness derivation is read against.

**Import is not shipped and is deferred explicitly**, per §6b and §6c. Export is the supported
backup mechanism; a half-safe importer is how a human's notes get overwritten by a file.

---

## Verification

Run by the orchestrator after the supersession fix, not quoted from the implementer:

```
cargo fmt --all -- --check                            → clean
cargo clippy --workspace --all-targets -- -D warnings → 0 warnings
cargo test --workspace --no-fail-fast                 → 1730 passed, 0 failed, 2 ignored (59 targets)
cargo build --release                                 → Finished
scripts/final_acceptance.sh                           → 83 passed, 0 failed, 0 skipped
Cargo.lock                                            → 106 packages, unchanged
```

Test count **1713 → 1730**. Acceptance **75 → 83**.

### Mutation probes

| probe | fails |
|---|---|
| the successor's creating event removed (the orchestrator's own fix) | `a_record_created_by_supersede_reports_its_own_creation` |
| `exported_at` added to the export document | `the_export_is_deterministic_and_contains_what_was_written` **and** `the_export_omits_the_three_things_that_would_make_it_a_claim` |

Each reverted and verified byte-identical afterwards.

---

## Known limitations

- Import is not built. Export is the backup mechanism.
- `nerve affected` / `nerve trace-tests` remain held only by the acceptance script (above).
- Citations still carry no live-target resolution verdict — 14a deferred it, and nothing reads a
  citation's live target yet.
- The five memory vocabularies are not yet covered by the wording single-copy guard, because there
  is no second surface copying them. That belongs with 14c, when there is one.

**Next:** Slice 14c — read-only HTTP and MCP, the T7 extension for memory text, and T13.
