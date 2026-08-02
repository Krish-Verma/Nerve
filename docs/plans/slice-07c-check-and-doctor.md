# Slice 7c — `nerve check` and `nerve doctor`

**Status:** planned 2026-08-02. **Split into 7c-i and 7c-ii before implementation.**

---

## Why this row is split

Roadmap row 7c is one row covering two commands with different consumers, different output
contracts and different failure semantics. `docs/CONTINUATION.md` records that a slice bundling
two surfaces has now cost **five** agents, and that the same work split in two succeeded every
time. The 2a/2b, 4a/4b, 5a/5b/5c and 5d-i/ii/iii splits were all made on this seam.

- **7c-i — `nerve check`.** One question, answered with an exit code, for CI.
- **7c-ii — `nerve doctor`.** Many questions, answered in prose, for a human whose install is
  misbehaving.

---

## 7c-i — `nerve check`

### The question

*"Can I trust this index right now?"* — asked by a CI job before it runs any other `nerve`
command, and by a pre-commit hook. The answer is an exit code, and the output is secondary.

### Why it is not `nerve status`

`status` reports. `check` **judges**, and its judgement is a process exit code another program
branches on. `StatusReport::is_healthy()` already covers schema currency and no run left
`running`, but it says nothing about the thing that actually goes wrong in CI: the index
describing a tree that has since moved on. An index that is internally healthy and six commits
stale will answer every query confidently and wrongly.

### What it checks

Reuses `nerve_index::index_freshness(conn, repo_id, prober, cap)`, which already reports
`files_total / files_probed / fresh / stale / missing / refused / unreadable / truncated`, plus
`StatusReport::is_healthy()`. No new analysis.

| condition | exit |
|---|---|
| index present, schema current, no run `running`, nothing stale or missing | `0` |
| no index at all | `2` (existing `NO_INDEX`) |
| schema behind, or a run still `running` | `3` (existing `PARTIAL_INDEX`) |
| index present and healthy but **stale** — files changed, added or removed since indexing | **`4`, new `STALE_INDEX`** |
| bad arguments | `10` (existing `USAGE`) |

One new exit code. Codes are a contract; adding one is a deliberate act and it is recorded here.

`--allow-stale` downgrades staleness to a reported warning and exits `0`, for the pipeline that
indexes and queries in one job and knows the tree cannot have moved.

### Deliberate non-goals

- **No policy thresholds.** `--max-unresolved`, `--max-gaps`, "fail the build if coverage drops"
  are a policy engine, a different feature with a different design problem, and they would make
  `check` mean whatever its flags happened to say. `nerve gaps` and `nerve impact` already emit
  JSON that a CI script can threshold itself.
- **No writing.** `check` never repairs and never re-indexes. A command that silently fixed the
  thing it was asked to judge could not be trusted to judge it.
- **`truncated` is not `fresh`.** If the freshness sweep hit its probe cap, `check` has not
  examined the whole tree and must say so rather than report a clean bill from a partial sweep.

### Acceptance criteria

1. The five exit codes above, each covered by a test that asserts the code and not merely the text.
2. A stale index is detected: index, mutate a file, `check` exits `4`.
3. A deleted and an added file both count as stale.
4. `--allow-stale` exits `0` and still reports the staleness.
5. A truncated sweep never reports a clean result.
6. `--json` carries the verdict, the reason, and the freshness counts.
7. `check` writes nothing — database byte-identical before and after.
8. Mutation probe: make staleness exit `0` and confirm a test fails.

---

## 7c-ii — `nerve doctor`

### The question

*"Something is wrong with my install — what?"* Diagnostics for a human, in prose, with a
suggested next action per finding. Exit `0` unless the environment is actually broken.

### Candidate checks (confirm against the code before building)

Database present, readable, `0600`; schema version vs supported; migration state; `PRAGMA
integrity_check`; `.nerve/config.toml` parses; root path recorded still exists; extractor runs
with no `finished_at`; orphaned rows; FTS index consistency; disk space for the database;
whether the recorded root matches the invocation root.

Each finding carries: what was checked, what was found, whether it is fatal, and what to do.

### Deliberate non-goals

- No repair. `doctor` diagnoses; a `--fix` mode is a separate decision with its own risks.
- No network check of any kind. Nerve is offline-first; there is nothing to reach.

### Gate

7c-ii is planned but **not** started until 7c-i is committed and verified.
