# Slice 7c-ii — `nerve doctor`, which has to work when everything else does not

2026-08-02. Plan: `docs/plans/slice-07c-check-and-doctor.md` §7c-ii. Follows Slice 7c-i (`c2cf8fa`).

---

## Objective

`nerve doctor` — *"something is wrong with my install, what?"* Diagnostics for a human, in prose,
with a suggested next action per finding.

## The constraint that defines the slice

`check` judges one thing and returns an exit code for CI. `doctor` inspects many things and returns
a readable report for a person whose tooling is misbehaving — which means **it must produce a
useful report when things are broken.** No database, corrupt database, schema from the future,
unparseable config are its subject matter, not reasons to bail. A `doctor` that panics on a corrupt
database is useless exactly when it is needed.

## Architecture decisions

**Eleven checks, one finding per check, every run, in a fixed order.** A check that could not run
is `severity: "skipped"` **with its cause**, never omitted and never reported as passing —
otherwise a caller cannot distinguish *sound* from *never established*. That is the same
absence-is-not-zero principle Slice 7a applied to coverage and Slice 7b to unresolved references.

Severities: `ok` / `skipped` / `warning` / `fatal`. Closed id vocabulary, pinned by two tests:
`nerve_dir`, `database_file`, `database_permissions`, `database_integrity`, `schema_version`,
`migration_history`, `fts_consistency`, `config_file`, `recorded_root`, `index_present`,
`unfinished_runs`.

**The SQL went to `nerve-store`, and the guard was widened.** `doctor` needs queries, and
`cli.rs` has a test forbidding SQL in the CLI — but it only scanned `main.rs`, so a new module
would have slipped past it. The implementer put the queries in a new `nerve_store::diagnose` and
**widened the guard to scan every module under `nerve-cli/src`**. That is the right instinct:
a guard that a new file can evade by existing is not a guard.

**No new exit code.** Fatal → `2` (`NO_INDEX`), whose documented meaning is already *"there is no
index at the requested path, **or it is not healthy enough to answer**"* — precisely the fatal rule
adopted. Everything else (schema behind, never indexed, interrupted run, wrong permissions, moved
root, FTS drift) is a warning and exits `0`. A warning is not a failure.

**`doctor` does not answer `check`'s question.** Index freshness is neither reimplemented nor
called; `doctor` prints a line pointing at `nerve check`. One question, one command, one code path.

## Two findings that came out of building it

**`SELECT count(*) FROM entity_fts` cannot detect FTS drift.** It reads the *content* table and
returns the entity count even after the index has diverged from it — so the obvious consistency
check is guaranteed to report agreement. The implementer established this by probe, not by
reasoning, and used the `entity_fts_docsize` shadow table instead, which holds one row per indexed
document. The test deletes from the FTS index and asserts `entities = 1, fts_documents = 0`: real
drift, really detected. `None` → `skipped` if that shadow table is ever absent.

**FTS5's own `integrity-check` is an `INSERT` command** and is blocked by `query_only`. Correctly
not used.

## Files changed

| file | why |
|---|---|
| `crates/nerve-store/src/diagnose.rs` | **new** — read-only facts, every field independently optional so a broken database still yields a report; 5 unit tests |
| `crates/nerve-store/src/lib.rs` | module + re-export |
| `crates/nerve-cli/src/doctor.rs` | **new** — severities, id vocabulary, judgement, rendering, `--json`; 6 unit tests |
| `crates/nerve-cli/src/main.rs` | `mod doctor;`, the subcommand, dispatch — 26 lines |
| `crates/nerve-cli/tests/cli.rs` | 15 end-to-end tests; the no-SQL guard widened to the whole crate |

No schema change, no migration, no dependency, no `apps/nerve-web/` file, no new exit code.

## Checks dropped, with reasons

- **Free disk space** — needs `statvfs`/`libc`. The brief said drop it rather than add a crate.
  Dropped; database size folded into `database_file`.
- **FTS5 `integrity-check`** — an `INSERT`, blocked by `query_only`. See above.
- **Index freshness** — that is `check`'s question. Deliberately not answered twice.

## Tests

**821 passed / 0 failed / 2 ignored**, up from 795. +26.

Every broken state is **constructed, not mocked**: no `.nerve/`, corrupt database, schema 99 from
the future, skipped migration, unparseable config, interrupted run, copied repository. `--json`
ids pinned by two tests. `doctor` writes nothing — BLAKE3 of the bytes after human, `--json` and
`--quiet` runs against a deliberately damaged index.

## Mutation probes

**Implementer's** — `FATAL_EXIT = exit::SUCCESS` (a fatal finding reports healthy). 7 failures
across 2 targets, all assertion failures, no compile errors.

**Orchestrator's**, deliberately different, aimed at migration-gap detection — make
`applied_versions` synthesise a complete version list. **3 failures**, no compile errors:

```
diagnose::tests::a_skipped_migration_shows_as_a_gap_in_the_applied_versions
diagnose::tests::a_database_with_no_nerve_tables_still_reports
doctor_reports_a_migration_that_was_never_applied
```

Reverted; `git status` shows only the five intended files; full gate re-run green.

## Verification

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace                                 → 821 passed, 0 failed, 2 ignored, exit 0
cargo build --release                                  → Finished, exit 0
```

**Orchestrator adversarial smoke tests**, beyond the implementer's, all with the release binary.
The property under test is *does it report rather than panic*:

| shape | result |
|---|---|
| healthy repository | `11 checks · 11 ok`, **exit 0** |
| no `.nerve/` at all | `FAIL nerve_dir`, 10 skipped, **exit 2**, no panic |
| **zero-byte** `nerve.db` | SQLite accepts it as an empty database, so integrity passes and `FAIL schema_version` fires — "may not be a Nerve database at all". **exit 2**, honest |
| `.nerve` is a **file**, not a directory | `FAIL nerve_dir` — "exists but is not a directory", **exit 2** |
| `nerve.db` is a **directory** | `FAIL database_file`, **exit 2** — but the wording is wrong, see below |
| `nerve.db` symlinked to `/etc/passwd` | `FAIL database_integrity` — "file is not a database". **exit 2, and no file content in the output.** No disclosure |

The symlink case is the security-relevant one and it is clean.

## Security / privacy / clean-room / dependency review

`query_only` on the connection: read-only by construction, proven on the bytes. `doctor` reads only
`.nerve/`, never repository content — which is why it was **not** added to the T1 no-subprocess
loop the way `check` was. No network of any kind: no version check, no update check; the
no-outbound-client test still passes. No telemetry. No `--fix`. No dependency added. Independent
implementation.

## Known limitations

- **One wording defect, found by the orchestrator.** When `nerve.db` exists but is a *directory*,
  `database_file` reports it as *"is missing"*. The verdict (fatal, exit 2) and the remedy are
  right; the sentence is not, and `nerve_dir` gets the analogous case right ("exists but is not a
  directory"). Cosmetic, but it is a diagnostic tool telling a small untruth about what it found,
  which is worth correcting the next time this file is opened. Reproduce: `mkdir .nerve/nerve.db`.
- **`PRAGMA integrity_check` is bounded to 5 reported problems** for affordability, but on a large
  database the check is still a full scan and `doctor` takes as long as it takes. Not timed.
- **`doctor` diagnoses one repository.** Multi-repository is Slice 13's problem.

## Out of scope, reported not fixed

`fixtures/ts-basic/.nerve/` exists in the working tree at **schema 1** with 44 entities — a
gitignored leftover from an old example run. **Verified untracked**: `git check-ignore` confirms
`.gitignore:2:.nerve/` covers it, and the only tracked path matching `.nerve` anywhere under
`fixtures/` is `fixtures/ts-incremental/.nerveignore`, which is a legitimate fixture. Harmless to
the repository, but it means `cp -R fixtures/ts-basic` carries a stale schema-1 index, which is why
the test helper `copy_tree` skips `.nerve`. Left in place: it is a local regenerable artifact in the
user's working tree, not something to delete unasked.

## UI backend handoff changes

**None.** `/api/doctor` was explicitly declined — diagnostics of a local installation are not a
read-only graph query.

## Commit

*feat: Slice 7c-ii — nerve doctor, which has to work when everything else does not*

Hash recorded in `docs/CONTINUATION.md` by the follow-up docs commit.

## Next slice

**Slice 8 — MCP.** T7 + T8 gate.
