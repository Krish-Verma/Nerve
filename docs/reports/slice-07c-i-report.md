# Slice 7c-i — `nerve check`, and the staleness the freshness sweep could not see

2026-08-02. Plan: `docs/plans/slice-07c-check-and-doctor.md` §7c-i. Follows Slice 7b (`45d0b77`).

---

## Objective

`nerve check` — *"can I trust this index right now?"*, answered with a process exit code for CI.

## User value

`status` reports; `check` **judges**, and its judgement is an exit code another program branches
on. `is_healthy()` already covered schema currency and open runs, but said nothing about what
actually goes wrong in CI: an index that is internally sound and several commits stale will answer
every query confidently and wrongly.

## Architecture decisions

**Five verdicts, five exit codes, one mapping.** `Verdict::{Current, NoIndex, Unusable, Stale,
Unverified}` → `0 / 2 / 3 / 4 / 4`. `exit_code()` is the only place the mapping exists, and a unit
test asserts every verdict maps to exactly one code. One new code, `STALE_INDEX = 4`; exit codes
are a contract and adding one is a deliberate act.

**`Unverified` is separate from `Stale` and identical in exit code.** Nothing was *observed* to
have changed — part of the tree was never looked at. Different evidence, same instruction to the
caller: do not trust this index. Distinguishing them in the `verdict` field rather than minting a
sixth exit code keeps the contract small while keeping the epistemics honest. A truncated sweep is
`Unverified`, never `Current`: a clean bill issued without looking is exactly the failure mode
`check` exists to prevent.

**Two evidence families, kept apart.** *Observed divergence* — `stale` (bytes moved), `missing`
(indexed file gone), `added`. *Not established* — `refused`, `unreadable`, `truncated`. Observed
divergence outranks "could not tell", because the caller can act on the first.

**Read-only by construction, not by discipline.** The connection is opened `query_only=ON`, so
SQLite refuses any write a later edit might introduce. The test proves it on the bytes.

## The pushback that changed the slice

**The brief said "reuse `index_freshness`, no new analysis" and also required an added file to
count as stale. Those two cannot both hold, and the implementer proved it rather than asserting
it.** `index_freshness` iterates `module_facts` — files the index already has a row for. An added
file has no row, is never probed, and is invisible: a repository can grow a hundred modules while
every recorded hash still matches, and `check` would report `current`.

The fix is `nerve_index::untracked_files` — `discover(root) − module_facts(repo_id)`, about 40
lines, no new SQL, no new table, and it reads no file contents during the walk. The test
`an_added_file_is_invisible_to_freshness_and_visible_to_untracked_files` pins the reason the
function exists.

**One subtlety, correctly handled.** Files the pipeline's loader refuses — over
`max_file_bytes`, unreadable, not UTF-8 — also have no `module_facts` row, so naively they would
read as permanent additions and pin `check` at exit 4 forever with nothing the user could do about
it. They are counted as `unindexable` and excluded, applying the loader's own three conditions.

## Files changed

| file | why |
|---|---|
| `crates/nerve-cli/src/exit.rs` | `STALE_INDEX = 4` |
| `crates/nerve-cli/src/main.rs` | `Check` command, `Verdict`, the three judges, rendering, 10 unit tests |
| `crates/nerve-index/src/inspect.rs` | `untracked_files` + `UntrackedFiles` + `indexable`, 3 unit tests |
| `crates/nerve-index/src/lib.rs` | re-export |
| `crates/nerve-cli/tests/cli.rs` | 11 end-to-end tests |
| `crates/nerve-cli/tests/no_subprocess.rs` | `check` added to the T1 loop — it reads repository bytes of its own now |
| `README.md` | exit-code line gained `4` |

No schema change, no migration, no dependency, no `apps/nerve-web/` file.

## Tests

**795 passed / 0 failed / 2 ignored**, up from 771. +24.

Each of the five exit codes asserted by **code**, not text. Modified, deleted and added files each
proven to produce exit 4 independently. `--allow-stale` exits 0 and still reports the staleness.
A truncated sweep never reports clean. `check` writes nothing — BLAKE3 of the database before and
after, using the project's own hasher rather than adding a `sha2` dependency.

## Mutation probes

**Implementer's** — `Verdict::Stale => exit::SUCCESS`. 7 tests failed across 2 targets, all
assertion failures (`left: 0, right: 4`), no compile errors.

**Orchestrator's**, deliberately different, aimed at the new discovery code — make
`untracked_files` never record an addition. **3 tests failed**, no compile errors:

```
check_exits_four_when_a_file_was_added
inspect::tests::an_added_document_counts_as_an_addition
inspect::tests::an_added_file_is_invisible_to_freshness_and_visible_to_untracked_files
```

Reverted; `inspect.rs` is +130/−4, insertions only against the pre-slice text. Gate re-run green.

## Verification

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace                                 → 795 passed, 0 failed, 2 ignored, exit 0
cargo build --release                                  → Finished, exit 0
```

**Orchestrator smoke tests with the release binary, beyond what the implementer ran:**

*Zero false positives across **all seven** fixtures* — `md-docs`, `md-links`, `md-supersession`,
`ts-basic`, `ts-coverage`, `ts-incremental`, `ts-resolution` — each indexed then checked:
`verdict current`, `added 0`, `unindexable 0`, **exit 0**. This was the claim most worth testing
independently, because `untracked_files` is new discovery logic and a false positive would make
`check` useless on day one. Document fixtures matter here: `module_facts` carries a row for
documents too (the `is_doc()` branch records `md-structural`'s version in both version columns),
so `.md` files are not seen as permanent additions.

*Unsupported file types do not trigger staleness.* Adding `thing.py`, `notes.txt` and a binary
`blob.bin` to an indexed fixture each left `verdict current`, **exit 0** — `discover()` filters by
supported extension, so a file indexing would never take is not reported as an addition. Adding a
real `brandnew.ts` moved it to `verdict stale`, `added 1`, **exit 4**. This was the failure mode I
most expected to find and it is not present. (When Slice 9 adds Python, `.py` becomes discoverable
and this behaviour changes correctly, on its own.)

*State transitions:* clean → 0 · modify → `stale`, 4 · `--allow-stale` → 0 · delete → `stale`, 4.
*Read-only:* sha256 of `.nerve/nerve.db` **identical after six `check` runs** including stale ones.

## Security / privacy / clean-room / dependency review

`check` reads; it never writes, and `query_only=ON` enforces that structurally. The discovery walk
reuses the Slice 1 path rules, so `.gitignore`/`.nerveignore`, the secret deny-list and
symlink-escape refusal all apply unchanged; a refused path is reported as *not established* rather
than silently treated as absent. `check` was added to the T1 no-subprocess loop because it now
reads repository bytes of its own. No network, no telemetry. No dependency added
(`Cargo.lock`/`third_party/` untouched). Independent implementation.

## Deviations

**One, argued and accepted:** the brief's "no new analysis" was wrong, and ~40 lines of new
discovery were required for the brief's own acceptance criterion 3 to be satisfiable. The
implementer proved the conflict with a test before writing the code. `/api/check` and all policy
flags were declined as briefed.

## Known limitations

- **`indexable()` restates the pipeline loader's three conditions rather than calling it.** If the
  loader's rules change, `check` misreports an addition it will not actually index. The duplication
  is documented at the function, but nothing enforces the correspondence — the same class of risk
  as the triplicated symbol-kind list Slice 7a-iii consolidated. Worth a shared helper when the
  loader next changes.
- **Truncation is unit-tested, not end-to-end.** Forcing the 5,000-file probe cap needs a
  repository larger than the cap, and indexing 5,001 files would dominate suite runtime.
  `judge_freshness` is a pure function and is tested as one. An `#[ignore]`d scale test is the
  option if end-to-end coverage is wanted.
- **`check` judges one repository.** Multi-repository CI is Slice 13's problem.

## UI backend handoff changes

**None.** `check` is a CI command whose product is an exit code; no endpoint, no view. `/api/check`
was explicitly declined — an HTTP status is not an exit code.

## Commit

*feat: Slice 7c-i — nerve check, and the staleness the freshness sweep could not see*

Hash recorded in `docs/CONTINUATION.md` by the follow-up docs commit.

## Next slice

**7c-ii — `nerve doctor`**, now unblocked by its gate.
