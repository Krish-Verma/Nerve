# Nerve — Continuation State

**Written:** 2026-07-31 · Keep this accurate at the end of every session.

---

## Where execution stopped

| | |
|---|---|
| **Current HEAD** | `8e732fe` — `fix: accept the repository positionally on nerve status` |
| **Branch** | `main` |
| **Working tree** | Clean |
| **Remote** | **None configured.** Nothing has been pushed. All work is local. |
| **Last completed slice** | **Slice 3** (`fec7f74`) — incremental indexing |
| **Next slice** | **Slice 3b** — normalize repository state out of `occurrence` / `observation` and out of `occurrence_id` |

## Verification state at HEAD

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 296 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

The 2 ignored are opt-in measurements, not skipped tests:

```bash
cargo test --release -p nerve-store --test scale -- --ignored --nocapture
cargo test --release -p nerve-index --test incremental -- --ignored --nocapture
```

**Cargo is not on `PATH` by default in this environment.** Prefix commands with
`export PATH="$HOME/.cargo/bin:$PATH";`.

## Commands to resume

```bash
cd /Users/krishverma/Documents/Nerve
export PATH="$HOME/.cargo/bin:$PATH"
git log --oneline -6
cargo test --workspace
```

Then read, in order: `CLAUDE.md`, `docs/ROADMAP.md`, `docs/reports/slice-03-report.md`,
`docs/plans/slice-03-incremental.md` (its P4 supersession note especially).

---

## Next objective — Slice 3b, exactly

**Problem.** Slice 3 missed acceptance criterion §5: a single-file edit re-indexes in **24.9%**
of a full-index wall time on a realistic 520-module corpus, against a **< 20%** target.
Invalidation amplification is 1.00 — exactly one file is re-extracted — so the cost is not
extraction. Extraction is ~8 ms of a ~2.9 s run.

**Cause.** `nerve_core::ids::occurrence_id` takes `state_id` as a canonical-tuple field
(`crates/nerve-core/src/ids.rs:134`), and `state_id` is denormalized onto every `occurrence` and
`observation` row. Every index run must therefore rewrite every surviving row to carry the new
state — O(repository), not O(change). Measured: restamp ~1330 ms, `rebuild_assertion_state`
~960 ms, prune ~196 ms, commit ~226 ms.

**Objective.** Remove the restatement pass by making occurrence identity independent of
repository state, so an unchanged file's rows are untouched by a re-index.

**Expected shape** (design it properly; this is a sketch, not a decision):
- Occurrence identity becomes `(entity_id, rel_path, start_byte, end_byte)` — a physical
  location, which does not depend on which state observed it.
- "Which states saw this occurrence" becomes a separate concern, or is dropped in favour of the
  content hash already stored per row.
- Schema **v3**, additive where possible, with v1→v3 and v2→v3 migration tests.
- **ADR-0002 must be amended**, since this changes an identity definition. Record the failure
  modes, do not claim identity is solved.

**Gates that must stay green and must not be weakened:**
1. The Slice 3 equivalence property — incremental ≡ full, byte-identical, every step.
2. The Slice 2a precision gate — FP=0, FN=0.
3. `fixtures/ts-basic/golden.json` will move (occurrence ids change). That is expected here,
   unlike in Slice 3. Diff it and confirm **only** occurrence ids and `schema_version` changed.
4. `rebuild_assertion_state` stays a pure function of `observation`.

**Acceptance:** the Slice 3 §5 ratio drops below 20% on the realistic corpus, measured ≥ 3 times
with all runs reported (this machine is noisy — see below).

---

## Decisions already made — do not relitigate

- **Slice 2 was split** into 2a (resolution) and 2b (query surface). Both complete.
- **Deletion is a hard delete.** The tombstone half of Slice 3's plan P4 is **superseded**:
  retaining an observation-less assertion as `DELETED` makes the incremental database differ
  from a fresh one, which the equivalence invariant forbids. `AssertionStatus::Deleted` and
  `Stale` remain unreachable by design. `derive.rs` is untouched. See the supersession note in
  `docs/plans/slice-03-incremental.md`.
- **Freshness is computed at query time**, by re-hashing the file, not stored. This is strictly
  better than a `STALE` flag: it detects changes that were never indexed at all.
- **Serial parsing.** Parallelism is deliberately deferred so that a future equivalence failure
  has one candidate cause, not two.
- **Slice 1 evidence-labelling defect fixed in 2a**: resolved `IMPORTS` and re-export `EXPORTS`
  are `AST_RESOLVED`, not `AST_DIRECT`.
- **Unresolved ids carry a `module` / `value` category**, because without it an unresolved module
  specifier and an unresolved value of the same name collided into one entity.
- **No remote, no push.** Do not add a remote or push without explicit user authorization.

## Open decisions requiring the user

1. **Git remote / publication.** No remote is configured. Whether Nerve becomes a public
   repository, under which account and licence, is a user decision. Nothing has been pushed.
2. **Slice 4 (visual explorer) scope.** The roadmap places `nerve serve` + a React SPA at Slice 4.
   `docs/SECURITY.md` requires a written threat model before document ingestion (Slice 5) and MCP
   (Slice 8), and a local HTTP surface deserves the same treatment. Confirm whether Slice 4 runs
   next or after 3b.
3. **Recall on real-world repositories is unmeasured.** Measuring it needs either a vendored
   permissively-licensed TypeScript corpus or a type checker to compare against. Both have cost
   and licensing implications. See "Known limitations" below.

## Environment notes for the next session

- **This machine was under sustained load average 25–51** during the Slice 2b and 3 sessions
  (external processes). Timing measurements are noisy: the scale test failed spuriously once at
  p95 1004 ms and passed at 38–120 ms minutes later on identical code. **Always run a timing
  measurement at least three times and report every run**, never a single flattering number.
- `rm` is aliased interactively; use `/bin/rm -f` in scripts.

## Known limitations carried forward

- Recall on real repositories is unmeasured (precision is measured, and gated at FP=0).
- 38.1% of call sites on the resolution corpus are honestly `Unresolved`; real repositories will
  be higher. Any method call on a typed receiver is unresolvable without type inference.
- State restatement is O(repository) — the Slice 3b subject.
- A transient file-read error treats that file as removed until the next successful run.
- Move proposals cover file-level moves only, not renames within a file.
- CommonJS `module.exports` is unmodelled, so those export surfaces are invisible.
- `nerve why` on a single entity has no `--limit`.
- The scale test is load-sensitive and can fail spuriously; it is `#[ignore]`d and does not gate CI.
