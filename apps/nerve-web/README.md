# `nerve-web` — INCOMPLETE SCAFFOLD, NOT A SHIPPED SLICE

> ## ⚠️ This directory does not build. It is preserved work-in-progress, not a delivered feature.

Slice 4b (the visual explorer SPA) was **started and not finished**. The implementation agent was
terminated by an environment session limit partway through scaffolding. What is here was
preserved rather than discarded because it is real work against the real API shapes, but **none
of it has been built, type-checked, linted, rendered, or reviewed.**

`nerve serve` currently serves the **Slice 4a placeholder page**, not this app. The Rust
workspace is unaffected — `apps/` is not a Cargo workspace member, and `cargo test --workspace`
passes at 427 tests with this directory present.

## What exists

```
index.html · package.json · tsconfig.json · vite.config.ts
src/api/{client.ts,types.ts}     API client and wire types, written against real 4a responses
src/{routing,format,hooks}.ts    routing, formatting, data hooks
src/ui/parts.tsx                 shared presentational parts
src/graph/{layout.ts,layout.test.mjs}   hand-rolled SVG graph layout + a test
src/styles/nerve.css             styling
src/views/Overview.tsx           1 of 6 views
public/assets/favicon.svg
```

## What is missing

- **No entry point** — `src/main.tsx` and `src/App.tsx` do not exist.
- **Five of six views** — Search, Entity, Evidence inspector, Neighbourhood graph, Unresolved.
- **`tools/lint.mjs` and `tools/embed.mjs`** — both referenced by `package.json` scripts, neither
  written. `npm run lint` and `npm run build` therefore fail.
- **`npm install` has never been run.** No lockfile, no `node_modules`.
- **Asset embedding is not wired.** `crates/nerve-server/src/assets.rs` still serves the
  placeholder.
- **No verification of any kind**: no typecheck, no lint, no build, no screenshots, no CSP check,
  no read-only check.

## Constraints the finished slice must still honour

Carried from `docs/plans/slice-04-visual-explorer.md` and `docs/THREAT-MODEL.md`:

- **Runtime dependencies are `react` and `react-dom` only.** No UI kit, chart library, graph
  library, state manager, CSS framework or icon package. `package.json` currently honours this —
  keep it that way. Vite and TypeScript are build-time only and are **not distributed**.
- **T5 is a blocking gate.** No `dangerouslySetInnerHTML`, no `innerHTML`, no inline `<script>`,
  no inline `<style>`, no inline event handlers. The 4a server sends
  `default-src 'none'; script-src 'self'; style-src 'self'` with **no `unsafe-inline`** — Vite
  must be configured to emit external JS and CSS only, or the page will not run.
  Do not weaken the CSP to make a build work.
- The graph is **always a bounded neighbourhood** of a focused entity. The API already returns
  `truncated`, `omitted_nodes` and `frontier_nodes`; surface them.
- The **evidence inspector is the centrepiece** — it is where Nerve's thesis becomes legible.
- Screenshot-driven visual QA is an acceptance criterion, not an optional extra.

## To resume

See `docs/CONTINUATION.md`, "Next objective".
