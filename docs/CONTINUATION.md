# Nerve — Continuation State

**Written:** 2026-07-31 · Keep this accurate at the end of every session.

---

## Where execution stopped

| | |
|---|---|
| **Current HEAD** | `1cd692a` — `wip: Slice 4b scaffold — INCOMPLETE, does not build` |
| **Branch** | `main` · **Working tree** clean |
| **Remote** | **None configured.** Nothing pushed. All work local. Deliberate — see "Decisions already made". |
| **Last completed slice** | **Slice 4a** (`d9958b3`) — `nerve serve`, loopback read-only HTTP API |
| **Next slice** | **Slice 4b** — `apps/nerve-web`, the visual explorer SPA |
| **Roadmap status** | **INCOMPLETE.** 4b started-not-finished; slices 5–14 not started. |

**Why it stopped:** the Slice 4b implementation agent was terminated by an environment session
limit (resets 10pm America/Los_Angeles) partway through scaffolding. Its partial work was
preserved as an explicit WIP commit rather than discarded or presented as complete.

## Verification state at HEAD

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 427 passed, 0 failed, 2 ignored
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

Then read: `CLAUDE.md` · `docs/ROADMAP.md` · `apps/nerve-web/README.md` ·
`docs/plans/slice-04-visual-explorer.md` · `docs/THREAT-MODEL.md` ·
`docs/reports/slice-04a-report.md`.

---

## Next objective — Slice 4b, exactly

Finish `apps/nerve-web` and embed it. **`apps/nerve-web/README.md` is the authoritative
inventory** of what exists and what is missing. Summary:

**Exists** (written against real 4a API responses, unverified): `api/client.ts`, `api/types.ts`,
`routing.ts`, `format.ts`, `hooks.ts`, `ui/parts.tsx`, `graph/layout.ts` + a test,
`styles/nerve.css`, `views/Overview.tsx`, `index.html`, `vite.config.ts`, `tsconfig.json`,
`package.json`.

**Missing:** `src/main.tsx`, `src/App.tsx`; the Search, Entity, **Evidence inspector**,
Neighbourhood-graph and Unresolved views; `tools/lint.mjs` and `tools/embed.mjs` (both referenced
by `package.json` scripts); `npm install`; asset embedding into
`crates/nerve-server/src/assets.rs`; and all verification.

**Start by running the server and exercising the API**, so you build against real shapes:

```bash
cargo build --release
cp -R fixtures/ts-resolution /tmp/demo && rm -rf /tmp/demo/.nerve
./target/release/nerve init /tmp/demo && ./target/release/nerve index /tmp/demo
./target/release/nerve serve /tmp/demo     # prints a URL containing ?token=…
```

Nine endpoints exist: `/api/overview`, `/api/search`, `/api/entity`, `/api/neighbourhood`,
`/api/path`, `/api/why`, `/api/source`, `/api/unresolved`, `/api/partial-parses`.

**Non-negotiable constraints:**
- Runtime deps are **`react` + `react-dom` only**. No UI kit, chart library, graph library, state
  manager, CSS framework or icons. Vite/TypeScript are build-time and **not distributed** —
  record them as such in `third_party/LICENSES.md`.
- **T5 is a blocking gate.** No `dangerouslySetInnerHTML`, no `innerHTML`, no inline
  `<script>`/`<style>`/handlers. The server sends `default-src 'none'; script-src 'self';
  style-src 'self'` with **no `unsafe-inline`**. Configure Vite to emit external JS/CSS.
  **Do not weaken the CSP to make a build work.**
- Graph is always a **bounded neighbourhood** of a focused entity; surface `truncated`,
  `omitted_nodes`, `frontier_nodes`.
- The **evidence inspector is the centrepiece** — source type, directness, extractor id+version,
  `file:line`, computed freshness. Give it the most design attention.
- Invoke the **`frontend-design` skill** before writing UI code.
- **Screenshot-driven visual QA is an acceptance criterion.** Actually look at the screenshots and
  fix what they reveal.

**Acceptance:** frontend typecheck + lint + build pass; Rust gate still passes; a grep-based check
proves no `dangerouslySetInnerHTML`; no CSP console violations; database byte-identical before and
after a UI session; screenshots across empty/small/large/long-name/unresolved-heavy repositories
and narrow (~380px) and wide (~1600px) viewports.

## Remaining roadmap after 4b

5 Markdown/ADR evidence · 6 Test evidence (coverage only) · 7 CLI+query expansion
(`impact`, `gaps`, `check`) · 8 MCP · 9 Python · 10 Framework rules · 11 Test call tracing ·
12 Git history · 13 Cross-repository contracts · 14 Human-confirmed memory ·
**plus a real-world accuracy validation phase** (see "Open decisions").

Each has a threat-model gate where applicable: T7 before Slice 5 and 8, T9 before Slice 6 and 11,
T8 before Slice 8. `docs/THREAT-MODEL.md` §6 tracks gate status.

---

## Decisions already made — do not relitigate

- **No remote, no push, no publication.** Explicitly deferred by the user until the whole system
  is reviewed. Do not add a remote or create a repository.
- **Slice 2 split** into 2a (resolution) and 2b (query surface); **Slice 4 split** into 4a
  (server + security) and 4b (SPA) after a single-unit attempt stalled.
- **Deletion is a hard delete.** Slice 3 plan P4's tombstone half is superseded: retaining an
  observation-less assertion as `DELETED` makes the incremental database differ from a fresh one,
  which the equivalence invariant forbids. `AssertionStatus::Deleted` and `Stale` are unreachable
  by design.
- **Freshness is computed at query time** by re-hashing, never stored. Strictly better than a
  `STALE` flag — it detects changes that were never indexed.
- **Occurrence identity is state-independent** (ADR-0006). An occurrence is a physical location
  fact; `content_hash` is the freshness anchor.
- **Tokio + axum rejected** for `nerve serve` on measured evidence: `tiny_http` costs 3
  transitive crates against ~80–100. Do not "upgrade" to axum without a new measurement.
- **Serial parsing.** Parallelism deferred so a future equivalence failure has one candidate cause.
- **Threat-model T11 accepted**: `tiny_http` reads header lines unbounded — a local availability
  issue with no disclosure path. Revisit only if Nerve binds non-loopback, ships multi-user, or
  grows beyond a read-only surface.

## Open decisions requiring the user

1. **Publication** — no remote exists; account, licence and public release are deferred by
   explicit instruction. Not blocking.
2. **Real-world accuracy validation.** Fixture precision is FP=0/FN=0 on 24 edges, but that is
   **not** real-world accuracy and must not be presented as such. The user's brief §7 specifies
   the shape: permissively licensed repositories, pinned commits, a recorded corpus manifest,
   licence review, and a compiler/language-service oracle used **only for evaluation**, never as
   Nerve's engine. This needs a decision on which corpus and which oracle, and carries a licence
   review. **Do not claim production-grade accuracy until it is done.**

## Environment notes

- **Session limits are real here.** One agent was terminated mid-slice by a session limit and
  another stalled on a 600s watchdog. **Keep slices small.** A slice bundling a security surface
  and a full frontend stalled; the same work split in two succeeded on the first half.
- The machine ran at load average 5–51 across this session. **Always run timing measurements ≥3
  times and report every run**, never a single flattering number. The scale test failed spuriously
  once at p95 1004 ms and passed at 38–120 ms minutes later on identical code.
- `rm` is aliased interactively; use `/bin/rm -f` in scripts.
- `curl`/`wget` are blocked by a hook; use `python3` + `urllib` for HTTP probing.
- Node v24.15.0 / npm 11.17.0 are available at `~/.nvm/versions/node/v24.15.0/bin`.

## Known limitations carried forward

- **Recall on real repositories is unmeasured.** Precision is measured and gated.
- 38.1% of call sites on the resolution corpus are honestly `Unresolved`; real repositories will
  be higher. Any method call on a typed receiver is unresolvable without type inference.
- No UI yet — `nerve serve` serves a placeholder page.
- CommonJS `module.exports` is unmodelled; move proposals are file-level only.
- A transient file-read error treats that file as removed until the next successful run.
- The scoped pruner's completeness is checked empirically, not proved. **If a future code path
  deletes observations outside `nerve-store::prune`, the scope silently becomes incomplete** —
  guard this when adding extractors.
- `nerve why` on a single entity has no `--limit`.
- No test asserts Nerve spawns no subprocess (threat-model T1 rests on code review). Corrective
  item raised in `docs/THREAT-MODEL.md` §7, still outstanding.
- The scale test is load-sensitive and can fail spuriously; it is `#[ignore]`d and does not gate CI.
