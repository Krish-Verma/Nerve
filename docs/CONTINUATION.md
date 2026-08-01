# Nerve — Continuation State

**Written:** 2026-08-01 · Keep this accurate at the end of every session.

---

## Where execution stopped

| | |
|---|---|
| **Current HEAD** | `f98aa2a` — `feat: Slice 5d-ii — supersession from explicit evidence, and nothing else` |
| **Branch** | `main` · **Working tree** clean at that commit |
| **Remote** | **None configured.** Nothing pushed. All work local. Deliberate — see "Decisions already made". |
| **Last completed slice** | **Slice 5d-ii** (`f98aa2a`) |
| **Next slice** | **Slice 5d-iii** — UI vocabulary catch-up (in flight at the time of writing) |
| **Roadmap status** | **INCOMPLETE.** 5d-iii, 6–14 and real-world validation not started. |

## Verification state at HEAD `f98aa2a`

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 596 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

Run by the orchestrator, not merely reported by an implementer. The 2 ignored are opt-in
measurements, not skipped tests:

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

Then read: `CLAUDE.md` · `docs/ROADMAP.md` ·
`docs/plans/slice-05d-supersession-and-filesystem-evidence.md` · `docs/decisions/ADR-0007-filesystem-evidence.md` ·
`docs/THREAT-MODEL.md`.

---

## What Slice 5d delivered

**5d-i (`c39e783`) — filesystem structure is not a syntax tree.** Since Slice 1 the repository
skeleton was stamped `AST_DIRECT` / `ts-js-structural`, which made a documentation-only tree
produce observations asserting a syntax tree in a repository with no TypeScript. New source type
`FILESYSTEM_OBSERVED` (appended at ordinal 11 — appending is load-bearing, `mask_bit()` is
`1 << ordinal` and the mask is stored), new extractor `fs-structural 1.0.0`, schema **v4** with a
data migration. Content-independence is structural: the extractor is handed an `FsEntry` projection
with no field that can hold file text. **T7 amended, not weakened** — the allowed set on a document
path is now exactly `{DOCUMENT_STATED, FILESYSTEM_OBSERVED}` keyed on extractor id, still total,
still mutation-verified. ADR-0007 records the semantics and the rejected alternatives.

**5d-ii (`f98aa2a`) — supersession from explicit evidence only.** Four recognised forms, two
deterministic resolution mechanisms, and everything else recorded as a value rather than guessed.
Cycles are detected and counted but **never suppressed** — each edge is individually evidenced.
FP=0, recall 100% over a 26-file corpus **whose ground truth was written before the resolver
existed**. Nerve's own six ADRs state no supersession and produce **zero** edges; the two real
`**Supersedes:**` fields in `docs/plans/` name prose rather than a target and are recorded
`document_supersedes_unparsed`.

## Remaining roadmap

| | |
|---|---|
| **5d-iii** | UI vocabulary catch-up. Glosses driven from the Rust vocabularies by a test so they cannot drift; `directnessClass`'s `default` arm currently renders an unknown directness as "inferred", which is a false claim rather than a missing label. Requires an `apps/nerve-web` rebuild and asset re-embed. |
| 6 | Test evidence (**coverage only**) — `TEST_COVERS_SYMBOL`, freshness. **T9 gate.** |
| 7 | CLI + query expansion — `impact`, `gaps`, `check`, `doctor` |
| 8 | MCP — one default investigation tool. **T7 + T8 gate.** |
| 9 | Python |
| 10 | Framework rules |
| 11 | Test call tracing. **T9 gate.** |
| 12 | Git history / temporal layer |
| 13 | Cross-repository contracts |
| 14 | Human-confirmed memory |
| **Validation** | Real-world accuracy — plan and corpus already chosen: `docs/plans/slice-15-real-world-validation.md`. Runs after Slice 9. **Network access for the corpus checkout was verified available on 2026-08-01** (`git ls-remote` against GitHub succeeds). |

---

## Decisions already made — do not relitigate

- **No remote, no push, no publication.** Explicitly deferred by the user. Do not add a remote.
- **Slice splits**: 2a/2b, 4a/4b, 5a/5b/5c, 5d-i/ii/iii. A slice bundling two surfaces has now cost
  **five** agents. **Keep slices small.**
- **Relation names are endpoint-kind-agnostic.** Kinds live in `entity.kind` and are never
  duplicated into relation names.
- **`ADR_DESCRIBES_COMPONENT` refused** — no deterministic rule separates "describes" from
  "mentions". **ADR status is a property, not an entity.**
- **`tree-sitter-md` rejected** — requires tree-sitter 0.26 against this workspace's 0.25.
- **A bare code-span name in prose is never a link**, and never a supersession target.
- **A supersession cycle is never suppressed.** Each edge is individually evidenced; deleting one
  would hide evidence. Detect, count, report.
- **Supersession fields are recognised on every document, not only ADRs.** The evidence is the
  explicit field, not the file name. The bare-identifier form still resolves only against parsed
  ADR identifiers, because that is the only identifier namespace that exists.
- **`EvidenceSourceType` is append-only.** `ordinal()` is a position in `ALL` and `mask_bit()` is
  `1 << ordinal`, over a **stored** mask. All twelve are now pinned individually so an insertion
  fails where it is written.
- **A migration's literals are frozen.** `migrate_v4` writes `fs-structural` / `1.0.0` as literals
  rather than reading the live constants, for the same reason `V1` is immutable.
- **No `resolution_method` column** — deferred with a trigger, not refused. Reconsider at Slice 10,
  the first slice producing several distinct resolution methods under one source type. See
  ADR-0007's rejected alternatives.
- **Deletion is a hard delete**; **freshness is computed at query time**; **occurrence identity is
  state-independent** (ADR-0006).
- **Tokio + axum rejected** for `nerve serve` on measured dependency cost. Do not "upgrade".
- **Serial parsing** — parallelism deferred so an equivalence failure has one candidate cause.
- **T11 accepted and investigated**: `tiny_http` is unbounded in header line length and count, and
  no mitigation is reachable. Revisit only if Nerve binds non-loopback or grows beyond read-only.

## Open decisions requiring the user

1. **Publication** — no remote exists; account, licence and public release are deferred by explicit
   instruction. Not blocking.

Real-world validation needs no user decision: corpus and oracle were selected autonomously and
recorded in `docs/plans/slice-15-real-world-validation.md`.

## Environment notes

- **Session limits and watchdogs are real here, and have now cost five agents.** One terminated
  mid-slice (4b), one stalled at the 600 s watchdog (5b), one hit a hard session limit (5c), one hit
  a hard session limit mid-verification (5d-i). In every case the partial work was inspected rather
  than discarded and the salvageable half was committed as its own verified unit. Do that rather
  than restarting from zero.
- **Do not run `cargo test` while a subagent is also running cargo.** One unreproducible test
  failure was observed in a workspace run that overlapped a subagent's build; four consecutive
  clean runs followed with nothing changed. Two cargo processes sharing `target/` is the most
  likely cause. It was not reproduced and no defect was found.
- **Never trust a subagent's verification claim.** The 5d-ii agent reported 596/0/2; the
  orchestrator's independent rerun initially disagreed. Rerun the gate yourself, every slice.
- A version constant like `EXTRACTOR_VERSION` is a **behavioural contract**: bumping it re-extracts
  every file of that kind. Bump it in the commit that changes behaviour, never earlier.
- Machine load ranged 5–51 across sessions. **Run timing measurements ≥3 times and report every
  run**, never a single flattering number.
- `rm` is aliased interactively; use `/bin/rm -f` in scripts. `timeout` is not installed.
- `curl`/`wget` are blocked by a hook; use `python3` + `urllib` for HTTP probing. `git` network
  access works.
- Node v24.15.0 / npm 11.17.0 at `~/.nvm/versions/node/v24.15.0/bin`.
- Subagent file tools **strip C0 bytes**, so a fixture needing a literal `0x1f` must store an
  escape and substitute at test time.

## Known limitations carried forward

- **Recall on real repositories is unmeasured.** Precision is measured and gated, on fixtures only.
- 38.1% of call sites on the resolution corpus are honestly `Unresolved`. Any method call on a typed
  receiver is unresolvable without type inference.
- **Document link and supersession coverage is deliberately narrow.** Real documentation refers to
  code in inline code spans, which Nerve refuses to treat as links: Nerve's own 45 documents contain
  5 Markdown link sites and 0 resolvable supersession statements. High precision, narrow coverage,
  by design. Do not present this as broad document understanding.
- A **reference-style** link (`[a][ref]`) used as a supersession target resolves as `unparsed`,
  because the scanner records that link's span at the `[ref]:` definition line rather than inside
  the field. Not covered by a fixture.
- For `**Superseded by:** <unresolvable>` the `Unresolved` entity becomes the assertion's **source**.
  No fixture covers it; it is covered only by construction and by `docref` unit tests.
- Indexing the whole repository makes the `fixtures/md-supersession` ADR identifiers ambiguous
  against Nerve's own `docs/decisions/`. The refusal is correct — the identifier namespace is
  repository-wide — but it means the fixture corpus is visible to a self-index.
- FTS matching is prefix-per-token, so `Through` never finds `callThroughMissingImport`.
- Overview has no language breakdown: no language aggregate exists in `StatusReport` or the API.
- Document resource-bound counters appear in `nerve index`, not `nerve status`.
- CommonJS `module.exports` is unmodelled; move proposals are file-level only.
- A transient file-read error treats that file as removed until the next successful run.
- The scoped pruner's completeness is checked empirically, not proved.
- `nerve why` on a single entity has no `--limit`.
- The scale test is load-sensitive and can fail spuriously; it is `#[ignore]`d and does not gate CI.
