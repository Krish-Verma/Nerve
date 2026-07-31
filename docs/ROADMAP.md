# Nerve Roadmap

Authoritative slice list. Update the status column at the end of every slice.
**Never begin a slice without explicit approval.**

| # | Slice | Status |
|---|---|---|
| 1 | Indexing foundation — `init`/`index`/`status`/`search`, SQLite evidence schema, TS/JS entities, `CONTAINS`/`DEFINES`/`IMPORTS`/`EXPORTS` | ✅ Complete (2026-07-31) — 137 tests pass, ADR-0001 gate passed |
| 2 | Static relationship resolution — `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`, module resolution, negative fixtures + measured precision, `nerve path`, `nerve why` | ⬜ Not started — **recommended next** |
| 3 | Incremental indexing — content hashes, changed-file indexing, importer invalidation, moves/deletes, `IdentityLink` | ⬜ Not started |
| 4 | Initial visual explorer — `nerve serve`, overview, search, graph canvas, evidence inspector | ⬜ Not started |
| 5 | Markdown + ADR evidence — sections, citations, document↔code identity links | ⬜ Not started |
| 6 | Test evidence (**coverage only**) — `TEST_COVERS_SYMBOL`, freshness, affected-test experiment | ⬜ Not started |
| 7 | CLI + query expansion — `impact`, `gaps`, `check`, evidence packets | ⬜ Not started |
| 8 | MCP — one default investigation tool | ⬜ Not started |
| 9 | Python language support | ⬜ Not started |
| 10 | Framework rules (routes, events, DI) with negative fixtures | ⬜ Not started |
| 11 | **Test call tracing** — `TEST_OBSERVED_CALL`, distinct from coverage, with sampling metadata | ⬜ Not started |
| 12 | Git history / temporal layer | ⬜ Not started |
| 13 | Cross-repository contracts | ⬜ Not started |
| 14 | Human-confirmed memory | ⬜ Not started |

## Slice 1 — delivered

- 4-crate Rust workspace (`nerve-core`, `nerve-store`, `nerve-index`, `nerve-cli`).
- SQLite schema v1 with the eight-concept evidence model + FTS5 symbol search.
- Tree-sitter extraction for `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`.
- Entities: Repository, Directory, File, Module, Function, Method, Class, Interface, Unresolved.
- Relations: `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`. Unresolved imports retained as first-class entities.
- `assertion_state` proven to be a pure rebuild from observations.
- CLI: `init`, `index`, `status`, `search` with `--json`.
- Security: secret deny-list, `.gitignore`/`.nerveignore`, path-traversal and symlink-escape guards, `0600` DB.

## Deferred out of Slice 1 (deliberate)

`CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS` — deferred to Slice 2 so they ship with negative
fixtures and a measured precision number rather than as name matches. See master plan §3.3.
