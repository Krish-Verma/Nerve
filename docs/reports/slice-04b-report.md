# Slice 4b — the visual explorer · completion report

**Date:** 2026-07-31 · **Status:** Complete · **Plan:** `docs/plans/slice-04-visual-explorer.md` (4b column)

---

## Summary

Nerve has a user interface. `nerve serve` now serves a React SPA compiled into the binary — no
Node, no build step, no network at runtime. Six views answer six questions, and the evidence
inspector makes the product thesis legible: an assertion written as a sentence, each observation
carrying its extractor id and version, and **freshness shown as the arithmetic it is** — the hash
recorded at observation time beside the hash on disk right now, with a verdict.

## A real Slice 4a bug — and a gap in my own verification

**Static assets required the session token, so the interface could not load at all.** A browser
cannot attach a header to `<script src>` or `<link href>`. The 4a placeholder page had therefore
been served **unstyled since `d9958b3`** and nobody noticed, because nothing ever rendered it.

```
GET /assets/nerve.css  →  401 token_required
```

**This is a gap in my own 4a verification, not just the agent's.** I probed `/` and every
`/api/*` route with the token attached, and separately without it — but I never loaded the page
the way a browser does, with subresources fetched header-less. Endpoint probing is not the same
as exercising the product.

The fix is in `router.rs` (policy), not `guard.rs` (mechanism, and a protected file). It relaxes
**only the token**, and **only** for paths resolving in the fixed asset table. I verified the
load-bearing claim in source before accepting it: `Guard::check` applies **Host → Origin → Token**
in that order (`guard.rs:113-129`), so a `MissingToken`/`BadToken` verdict is *proof* both earlier
checks already passed.

Confirmed live against a release binary:

| Probe | Result |
|---|---|
| `/`, `/assets/nerve.js`, `/assets/nerve.css` without token | **200** — the UI loads |
| `/assets/nerve.js` with `Host: evil.test` | **403** |
| `/assets/nerve.js` with `Origin: http://evil.test` | **403** |
| `/api/overview`, `/api/search`, `/api/why` without token | **401** |
| `/api/nonexistent` without token | **401** — no route enumeration |
| `/assets/../api/overview` | **401** — not treated as an asset |
| `/api/overview` with token | 200 |

What is served token-free is build-constant: identical bytes in every copy of the binary, with no
repository, index or session content.

## Files changed

**New** — `src/main.tsx`, `src/App.tsx`, `src/ui/Omnibox.tsx`, views `Search`/`Entity`/`Evidence`/
`Graph`/`Path`/`Source`/`Gaps`, `src/search.ts` (+test), `src/vocab.ts`, `tools/lint.mjs`,
`tools/embed.mjs`, `package-lock.json`.

**Changed** — `api/types.ts`, `graph/layout.ts` (+`ringLabelPlacement`, 2 tests), `styles/nerve.css`,
`ui/parts.tsx` (two pre-existing type errors fixed), `views/Overview.tsx`, `package.json`
(**vite `7.1.14` → `7.3.6`; the scaffolded version does not exist**), `README.md`,
`crates/nerve-server/src/assets.rs` (real 4-asset table + 2 tests), `router.rs` (the fix above),
`tests/security.rs` (3 tests replacing 1), `third_party/LICENSES.md`.

**Untouched, verified:** `guard.rs`, `token.rs`, `respond.rs`, `fixtures/`, the four
orchestrator-owned docs, and the frozen extraction/schema modules.

## Verification — run by the orchestrator

```
npm typecheck                                           → clean
npm lint                                                → 23 files clean; runtime deps react + react-dom
npm test                                                → 15 passed, 0 failed
npm build                                               → 4 files, 287,885 B → crates/nerve-server/assets
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 435 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

432 → 435 Rust tests. No test weakened or deleted.

**T5:** `grep` for `dangerouslySetInnerHTML|.innerHTML|eval(|new Function(|document.write` across
`apps/nerve-web/src/` → **no matches**. Enforced by `tools/lint.mjs`, which the agent proved fires
by planting `document.body.innerHTML` and watching the build fail, then clean on removal.

**CSP:** zero violations across all 31 captured pages. The only console entries are the deliberate
403 and connection-refused in the two error states.

**Read-only:** `nerve.db` sha256 `0ad77dc9…` byte-identical before and after my own full UI
session. WAL empty throughout.

## Visual QA — 31 screenshots, `/tmp/nerve-slice4b-screenshots/`

31 pages × 4 repositories (small, empty, hostile-XSS, 485-entity) at 380px and 1600px.
**I reviewed the screenshots myself**, not only the agent's summary.

- **Evidence inspector** — `Circle` *defines* `Circle.area`, `supported`, `1 observation`,
  `ts-js-structural@1.1.0`, `AST_DIRECT`, `fresh`, `match quality: not applicable` (no fake
  confidence anywhere), `src/shapes.ts:38–40`, then *"is the source still what was read?"* with
  both hashes side by side and the verdict *"identical — src/shapes.ts still holds exactly the
  bytes this was extracted from"*, then the source lines. This is the thesis, rendered.
- **Graph** — on a 485-entity repository with a 122-relation hub: `145 MORE NOT DRAWN` on the
  boundary ring, `25 of 170 entities · 24 assertions · 0 on the outer ring` in the footer, node
  budget (25/60/150/500), hops (1–4), relation filters, dashed edges for unresolved ends.
  Bounded neighbourhood, explicit truncation, **not a hairball** — plan P4 satisfied.

Defects the screenshots revealed and the agent fixed: singular/plural agreement ("1 relationship
involve"), the verb rendered as `DEFINES` instead of Nerve's own voice, graph overflow past the
fold, ring labels colliding with nodes (fixed with a new pure `ringLabelPlacement` + 2 tests),
6-line reason-code wrapping in Gaps, 8 auto-opened observations causing 8 file re-reads on load,
a path picker that stayed open after resolving, and 1000px-wide empty-state text.

## Dependencies

**Distributed** (compiled into the binary): `react` 19.2.0, `react-dom` 19.2.0, `scheduler` 0.27.0
— all MIT. **Build-time only, not distributed**: 67 packages (vite 7.3.6 MIT, typescript 5.9.3
Apache-2.0, esbuild/rollup/postcss and platform binaries). All 70 permissive; no copyleft.
Recorded in `third_party/LICENSES.md` with the distributed/not-distributed split explicit.

## Not implemented

**Language breakdown on Overview.** No language aggregate exists anywhere — not in `StatusReport`,
not in the API. Adding one is a `nerve-store` query plus an `api.rs` field, and would drift the
server's shape from the CLI's `--json`, which `shapes.rs` explicitly forbids. Deriving it
client-side would fabricate precision. Overview surfaces extractor identity and version instead.
**Deferred as a small `nerve-store` follow-up, correctly refused in the UI layer.**

## Known limitations

- FTS matching is prefix-per-token: `Through` will never find `callThroughMissingImport`. The UI
  states the rule and offers real alternative queries rather than faking fuzzy results.
- The evidence inspector issues one `/api/source` per opened observation (bounded by ≤3 auto-open).
- Graph labels are small at 380px; structure reads but individual names need the tap target.
- `/api/why` has no `limit`; a very high-degree entity returns everything (pre-existing).
- Visual QA used Playwright — the `claude-in-chrome` extension failed to connect.

## Result

Slice 4 is complete. Nerve is no longer a CLI with a JSON API; it is a usable local product.
