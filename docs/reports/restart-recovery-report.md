# Restart recovery — 2026-08-01

**Reason:** the machine underwent a software update and restarted while a Nerve session was in
progress. The prior conversation's last confirmed report named HEAD `f344310`; a later session had
continued past it. This report reconstructs the actual state rather than trusting either.

---

## 1. Repository located

| | |
|---|---|
| Path | `<checkout root>` |
| Toplevel | `<checkout root>` (same — not a subdirectory) |
| Branch | `main` |
| **HEAD found** | **`a8a2d5d`** — `docs: Slice 6 plan — and the question that gates it` |
| Working tree | **clean** — `git status --porcelain=v2` returned no entries |
| Untracked | none, including with `--untracked-files=all` |
| Stash | empty |
| Worktrees | one, the main checkout |
| Remote | **none configured** (unchanged, as intended) |

## 2. Interrupted Git operations — none

Checked and **absent**: `.git/MERGE_HEAD`, `.git/CHERRY_PICK_HEAD`, `.git/REVERT_HEAD`,
`.git/BISECT_LOG`, `.git/index.lock`, `.git/rebase-merge/`, `.git/rebase-apply/`.

No lock file existed, so none was removed. No recovery command was run — no `reset --hard`, no
`clean`, no `stash drop`, no `reflog expire`, no `gc`.

## 3. Commits found after `f344310`

`f344310` was a **lower bound, not the tip**. Five commits followed it, all before the restart:

| Commit | Subject |
|---|---|
| `c39e783` | fix: Slice 5d-i — filesystem structure is not a syntax tree |
| `f98aa2a` | feat: Slice 5d-ii — supersession from explicit evidence, and nothing else |
| `61fe997` | docs: Slice 5d report and continuation state |
| `947013c` | feat: Slice 5d-iii — the interface can name what the backend stores |
| `a8a2d5d` | docs: Slice 6 plan — and the question that gates it |

Resetting to `f344310` would have destroyed three completed slices. It was not done.

## 4. One dangling object, inspected and not recovered

`git fsck` reported one dangling commit, `cb1ab00`:

```
WIP on main: f344310 feat: Slice 5c — document link resolution…
Krish Verma · Sat Aug 1 12:23:53 2026 · three parents
```

Three parents and that subject line make it a **`git stash` commit** whose entry was later dropped
— the ordinary end of a stash that was popped and applied. It is not orphaned work from the crash;
it predates the restart by nine hours and predates `c39e783`.

Its content was checked against HEAD rather than assumed:

- Its headline change, `FILESYSTEM_OBSERVED` in `crates/nerve-core/src/vocab.rs`, **is present at
  HEAD** — that work landed as Slice 5d-i.
- `git diff cb1ab00 HEAD` over its files shows HEAD **ahead by 445 insertions against 15
  deletions**, i.e. HEAD is a superset, not a divergent branch.

**Conclusion: superseded, nothing to salvage.** It was left in place (dangling objects are harmless
and `gc` was deliberately not run).

## 5. Interrupted work: a Slice 6 subagent, which left nothing on disk

Session notes show the pre-restart session had committed the Slice 6 *plan* (`a8a2d5d`) and
dispatched an implementation subagent, which was killed by the restart mid-run. The working tree is
clean and contains no Slice 6 source, tests or fixtures, so **that agent had not written to disk**.
Nothing was preserved because nothing existed; nothing was discarded.

Its one durable output was a research finding — that Node.js LCOV carries no per-test attribution —
which is the gating question of the Slice 6 plan §2.1. **That finding is treated as a hypothesis to
re-derive empirically, not as an established result**, because it exists only in session notes and
no artefact in the repository supports it.

## 6. Toolchain after the software update — intact

| Tool | Version | Expected |
|---|---|---|
| rustc | 1.97.1 (8bab26f4f 2026-07-14) | `rust-toolchain.toml` pins `1.97.1` — **match** |
| cargo | 1.97.1 (c980f4866 2026-06-30) | match |
| node | v24.15.0 | continuation notes record v24.15.0 — **match** |
| npm | 11.17.0 | continuation notes record 11.17.0 — **match** |
| git | 2.50.1 (Apple Git-155) | works |

**Nothing was upgraded, downgraded or repaired.** No edition change, no lockfile change, no package
manager change. `Cargo.lock` is unmodified. Cargo remains absent from the default `PATH`; commands
are still prefixed with `export PATH="$HOME/.cargo/bin:$PATH"`.

## 7. Verification run after recovery

Run by the orchestrator at `a8a2d5d`, not carried over from a prior report:

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → Finished, 0 warnings
cargo test --workspace                                  → 610 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

610 matches the count `docs/ROADMAP.md` records for Slice 5d-iii, so the committed work is
independently confirmed rather than assumed. The 2 ignored are the opt-in scale and incremental
measurements, unchanged.

## 8. Recovery state and next work

**State B — new commits exist and are complete.** No file was preserved, removed, edited or
reverted during recovery; the only change in this commit is documentation, because the repository
needed no repair.

One real inconsistency was found and fixed: **`docs/CONTINUATION.md` was stale.** It recorded HEAD
`f98aa2a` and named Slice 5d-iii as "next, in flight", but 5d-iii was committed at `947013c` and the
Slice 6 plan at `a8a2d5d`. A future session reading it would have redone a completed slice. Updated
in the same commit as this report.

**Next incomplete slice: Slice 6 — test evidence (coverage only).** Its plan is committed at
`a8a2d5d` (`docs/plans/slice-06-test-evidence.md`) and its first task is the §2.1 gating question,
to be answered empirically before any emission path is written.
