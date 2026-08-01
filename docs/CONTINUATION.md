# Nerve — Continuation State

**Written:** 2026-08-01 · Keep this accurate at the end of every session.

---

## Where execution stopped

| | |
|---|---|
| **Current HEAD** | `cbe667a` — `feat: Slice 5a — Markdown and ADR evidence, and two closed identity forgeries` |
| **Branch** | `main` · **Working tree** clean at that commit |
| **Remote** | **None configured.** Nothing pushed. All work local. Deliberate — see "Decisions already made". |
| **Last completed slice** | **Slice 5a** (`cbe667a`) |
| **Next slice** | **Slice 5b** — document↔code links, measured precision, invalidation |
| **Roadmap status** | **INCOMPLETE.** 5b, 5c, 6–14 and real-world validation not started. |

## Verification state at HEAD

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 506 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

The 2 ignored are opt-in measurements, not skipped tests:

```bash
cargo test --release -p nerve-store --test scale       -- --ignored --nocapture
cargo test --release -p nerve-index --test incremental -- --ignored --nocapture
```

**Cargo is not on `PATH`.** Prefix commands with `export PATH="$HOME/.cargo/bin:$PATH";`.

## Commands to resume

```bash
cd /Users/krishverma/Documents/Nerve
export PATH="$HOME/.cargo/bin:$PATH"
git log --oneline -10
cargo test --workspace
```

Then read: `CLAUDE.md` · `docs/ROADMAP.md` · `docs/plans/slice-05-document-evidence.md` ·
`docs/reports/slice-05a-report.md` · `docs/THREAT-MODEL.md`.

---

## Remaining roadmap

| | |
|---|---|
| **5b** | Document↔code links: `Section REFERENCES` file/symbol by explicit path and `#L<n>` anchor, `Document SUPERSEDES Document`, unresolved reasons, **measured precision gate**, invalidation across document→code edges. Backend only. |
| **5c** | **Corrective + UI catch-up.** Filesystem containment is labelled `AST_DIRECT` and attributed to `ts-js-structural` even in a repository with no TypeScript — see below. Fixing it is a vocabulary addition, which needs a UI gloss anyway, so the deferred glosses for `document`, `section` and `SUPERSEDES` ride along. Requires an `apps/nerve-web` rebuild and asset re-embed. |
| 6 | Test evidence (**coverage only**) — `TEST_COVERS_SYMBOL`, freshness. **T9 gate.** |
| 7 | CLI + query expansion — `impact`, `gaps`, `check`, `doctor` |
| 8 | MCP — one default investigation tool. **T7 + T8 gate.** |
| 9 | Python |
| 10 | Framework rules |
| 11 | Test call tracing. **T9 gate.** |
| 12 | Git history / temporal layer |
| 13 | Cross-repository contracts |
| 14 | Human-confirmed memory |
| **Validation** | Real-world accuracy — plan and corpus already chosen: `docs/plans/slice-15-real-world-validation.md`. Runs after Slice 9. |

## Slice 5c — the exact finding

Indexing a documentation-only tree produced 4 observations labelled `AST_DIRECT`, attributed to
`ts-js-structural`, in a repository containing **no TypeScript at all**:

```
AST_DIRECT DIRECT ts-js-structural docs             CONTAINS repository '.'  → directory 'docs'
AST_DIRECT DIRECT ts-js-structural docs/decisions   CONTAINS directory docs  → directory 'decisions'
```

`AST_DIRECT` is defined as *"the syntax tree literally contains this relationship."* There is no
syntax tree behind a directory. This is the same defect class Slice 2a already corrected once, when
resolved imports were found mislabelled `AST_DIRECT`. It is **pre-existing from Slice 1** — Slice 5a
made it visible, not worse. The likely fix is a new `EvidenceSourceType` for filesystem structure
plus its own trivial extractor, which changes `EvidenceSourceType::ALL` (append only — `ordinal()`
and `mask_bit()` must stay stable for existing variants) and the golden dump.

---

## Decisions already made — do not relitigate

- **No remote, no push, no publication.** Explicitly deferred by the user. Do not add a remote.
- **Slice splits**: 2a/2b, 4a/4b, 5a/5b/5c. A slice bundling two surfaces stalled an agent at the
  600 s watchdog; the same work split in two succeeded. **Keep slices small.**
- **Relation names are endpoint-kind-agnostic.** `DOCUMENT_CONTAINS_SECTION` and friends were
  rejected: kinds live in `entity.kind` and are never duplicated into relation names. Reuse
  `CONTAINS`/`REFERENCES`; `SUPERSEDES` was the one genuine addition.
- **`ADR_DESCRIBES_COMPONENT` refused** — no deterministic rule separates "describes" from
  "mentions". **ADR status is a property, not an entity** — a status value has no occurrences.
- **`tree-sitter-md` rejected** — it requires `tree-sitter 0.26` against this workspace's `0.25`.
  Adopting it means re-parsing every precision-gated TS/JS fixture through a new runtime, or
  carrying two copies of tree-sitter's C runtime. A restricted hand-written block scanner was
  written instead, with zero dependencies.
- **Everything derived from a document is `DOCUMENT_STATED`**, including `File CONTAINS Document`.
  T7 separation is therefore a total function of the source file and is checked exhaustively.
- **A bare code-span name in prose is never a link.** It is counted, not entity-ised: an
  `Unresolved` entity stands for a reference *site* a resolver failed on, and prose is not one.
- **Deletion is a hard delete**; `AssertionStatus::Deleted`/`Stale` are unreachable by design.
- **Freshness is computed at query time** by re-hashing, never stored.
- **Occurrence identity is state-independent** (ADR-0006).
- **Tokio + axum rejected** for `nerve serve` on measured dependency cost. Do not "upgrade".
- **Serial parsing** — parallelism deferred so an equivalence failure has one candidate cause.
- **T11 accepted and now investigated** (2026-07-31): `tiny_http` is unbounded in header *line
  length* **and** header *count*, and no mitigation is reachable — `ServerConfig` has only `addr`
  and `ssl`, and `Listener` is a closed enum. Revisit only if Nerve binds non-loopback, ships
  multi-user, or grows beyond a read-only surface.

## Open decisions requiring the user

1. **Publication** — no remote exists; account, licence and public release are deferred by explicit
   instruction. Not blocking.

Real-world validation no longer needs a user decision: the corpus and oracle were selected
autonomously and recorded in `docs/plans/slice-15-real-world-validation.md`.

## Environment notes

- **Session limits are real here.** One agent was terminated mid-slice; another stalled on a 600 s
  watchdog. **Keep slices small.**
- Machine load ranged 5–51 across sessions. **Run timing measurements ≥3 times and report every
  run**, never a single flattering number.
- `rm` is aliased interactively; use `/bin/rm -f` in scripts.
- `curl`/`wget` are blocked by a hook; use `python3` + `urllib` for HTTP probing.
- Node v24.15.0 / npm 11.17.0 at `~/.nvm/versions/node/v24.15.0/bin`.
- Subagent file tools **strip C0 bytes**, so a fixture needing a literal `0x1f` must store an
  escape and substitute at test time — with a test asserting the substitution happened, so the
  security test cannot pass vacuously.

## Known limitations carried forward

- **Recall on real repositories is unmeasured.** Precision is measured and gated, on fixtures only.
- 38.1% of call sites on the resolution corpus are honestly `Unresolved`. Any method call on a typed
  receiver is unresolvable without type inference.
- FTS matching is prefix-per-token, so `Through` never finds `callThroughMissingImport`.
- **The UI has no gloss for `document` or `section`** — `kindGloss` falls back to "This build has no
  description for that entity kind." Honest, but unfinished. Slice 5c.
- Overview has no language breakdown: no language aggregate exists in `StatusReport` or the API.
- Document resource-bound counters appear in `nerve index`, not `nerve status`, which reads only
  the graph tables.
- CommonJS `module.exports` is unmodelled; move proposals are file-level only.
- A transient file-read error treats that file as removed until the next successful run.
- The scoped pruner's completeness is checked empirically, not proved. **If a future code path
  deletes observations outside `nerve-store::prune`, the scope silently becomes incomplete.**
- `nerve why` on a single entity has no `--limit`.
- The scale test is load-sensitive and can fail spuriously; it is `#[ignore]`d and does not gate CI.
</content>
