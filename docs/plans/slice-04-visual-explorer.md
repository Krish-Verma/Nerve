# Slice 4 — Initial visual explorer (`nerve serve`)

**Date:** 2026-07-31 · **Status:** Accepted, **split into 4a and 4b** · **Depends on:** Slices 1–3b
**Security gate:** `docs/THREAT-MODEL.md` T4, T5, T6 — **blocking**

> ## Scope split (2026-07-31, after a failed attempt)
>
> This slice was first dispatched as a single unit — HTTP server, read-only API, three blocking
> security controls, a full React SPA, asset embedding, and screenshot-driven visual QA. The
> implementation agent stalled without producing any change. That is a **task-decomposition
> error on the orchestrator's part**, not an implementation failure: the slice bundled a backend
> security surface and a complete frontend product into one reviewable unit, which is exactly the
> oversizing this project splits elsewhere.
>
> | | Scope | Verified by |
> |---|---|---|
> | **4a** | `crates/nerve-server`: blocking loopback HTTP, read-only JSON API, T4/T5/T6 controls, asset-embedding hook with a placeholder page | Rust tests — token, origin/host, XSS escaping, path safety, read-only, clean shutdown |
> | **4b** | `apps/nerve-web`: the React SPA, real asset embedding, product design, screenshot QA | Frontend gate + screenshot-driven review |
>
> 4a is a security surface that must be provably correct before any UI is worth building on it;
> 4b is a design product. They have different reviewers, different gates, and different failure
> modes. Both run sequentially. Everything in §Scope below is retained and divided between them.

---

## Disagreements and Pushback

### P1 — Do not adopt Tokio + axum. The requirement does not justify the surface.

The master plan said an async runtime arrives "only when `nerve serve` exists and needs it."
It exists now. **It does not need it.**

`nerve serve` is a **loopback-only, single-user, read-only** server with a handful of JSON
endpoints and some embedded static assets. Tokio + axum + tower + hyper pulls roughly 80–100
additional crates into a product that today has 93 total, every one of which must be recorded and
licence-reviewed under CLAUDE.md §1 — and it introduces an async execution model into a codebase
that is deliberately serial for determinism.

**Decision.** Use a **minimal blocking HTTP server** with a small thread pool. Candidate:
`tiny_http` (MIT/Apache-2.0, ~10 transitive crates). The subagent must verify the licence and the
actual transitive count before adopting, and **record it in `third_party/LICENSES.md`**.

If the chosen crate turns out to pull an unexpected tree, or cannot bind loopback-only, report it
and propose an alternative rather than silently reaching for axum. A hand-rolled HTTP/1.1 read-only
server is also acceptable if it proves smaller — but only with tests for malformed requests.

### P2 — Loopback binding is not a security control on its own

A web page the user visits can `fetch('http://127.0.0.1:PORT/api/...')`. Loopback is reachable
from the browser. Every "it's local so it's safe" local-server design has this hole.

**Decision — T4 controls are blocking, not optional:** per-session token generated at `serve`
start and required on every API request; **no permissive CORS**; `Origin`/`Host` validated
against the bound address (DNS-rebinding defence — an attacker domain resolving to 127.0.0.1
otherwise passes a naive host check); read-only API so a successful CSRF still cannot mutate.

### P3 — Repository content is hostile and *will* be rendered

`<img src=x onerror=alert(1)>` is a legal substring of a filename and of a TypeScript identifier.
Nerve indexes and displays both.

**Decision — T5 controls are blocking:** render every repository-derived string as **text, never
HTML**. No `dangerouslySetInnerHTML`. A strict CSP with no `unsafe-inline` and no remote origins.
A test must assert that a fixture containing an XSS payload in a symbol name is escaped in the
served output.

### P4 — "Graph canvas" must not mean "render every node"

A 200k-entity repository cannot be shown as a graph, and a 500-node hairball answers no question.
Build prompt §18: *the graph view should help answer questions, not display every node.*

**Decision.** The graph is **always a bounded neighbourhood of a focused entity** — default depth
1, expandable — never "the repository". Node budget enforced in code with an explicit
"N more not shown" affordance. No physics simulation over thousands of nodes.

### P5 — The frontend dependency tree cannot be fully licence-audited, so keep it tiny

An npm tree is thousands of packages. CLAUDE.md §1 requires every dependency recorded.

**Decision.** Build-time tooling (Vite, TypeScript) is **not distributed** — only the built assets
ship, embedded in the binary — and is recorded as such. **Runtime** dependencies are held to
`react` + `react-dom` and nothing else: no UI kit, no chart library, no graph library, no state
manager. Graph rendering is hand-rolled SVG. This is a licence-surface decision as much as an
aesthetic one.

---

## Objective

`nerve serve` starts a local application that makes the evidence graph explorable by a human:
find something, understand what it is, see what it connects to, and inspect *why Nerve believes
it* — including what Nerve does not know.

## Scope

1. **`crates/nerve-server`** — blocking HTTP on `127.0.0.1`, ephemeral or `--port`, per-session
   token, read-only JSON API, embedded SPA assets. **No business logic**: it calls the same
   `nerve-store` functions the CLI uses (ARCHITECTURE.md invariant 3).
2. **API** (read-only, token-gated): status/overview · search · entity detail with occurrences ·
   neighbourhood (bounded) · path · why/evidence · source snippet (T6-guarded).
3. **`apps/nerve-web`** — React + Vite + TypeScript SPA:
   - **Overview**: counts by kind and relation, unresolved totals, freshness, last run, schema.
   - **Search**: FTS over entities, kind filter, keyboard-navigable.
   - **Entity view**: what it is, where it lives, what defines it, what it calls, what calls it.
   - **Evidence inspector**: the `why` packet — source type, directness, extractor id+version,
     `file:line`, freshness — the product's whole thesis, made legible.
   - **Bounded neighbourhood graph** (P4).
   - **Unresolved and partial-parse surfaces**: what Nerve could not resolve, and which files
     parsed with errors. Absence of knowledge is first-class, not hidden.
   - Empty, loading, and error states for every view.
4. **Embedding** — built assets compiled into the binary so `nerve serve` needs no node at runtime.
5. **Security** — T4, T5, T6 implemented and tested.

## Non-goals

No editing, no mutation, no auth beyond the session token, no multi-repository, no remote access,
no telemetry, no external fonts or CDNs (CSP forbids them anyway), no framework beyond React.

## Acceptance criteria

1. Rust gate passes: fmt, clippy `-D warnings`, `cargo test --workspace`, release build.
2. Frontend gate passes: typecheck, lint, build. Frontend unit tests where logic warrants them.
3. **T4:** binds `127.0.0.1` only; a request without the token is rejected; an `Origin` or `Host`
   that is not the bound address is rejected. All three asserted by tests.
4. **T5:** a fixture with an XSS payload in a symbol name is escaped in served output — asserted
   by test. CSP header present with no `unsafe-inline`.
5. **T6:** the source endpoint refuses traversal, symlink escape, and deny-listed files, reusing
   the existing `canonical_child` choke point rather than a second implementation. Asserted by
   test, including a constructed symlink escape.
6. **Read-only:** the database is byte-identical before and after a full UI session.
7. No business logic in `nerve-server`: query logic lives in `nerve-store`.
8. Every runtime dependency recorded in `third_party/LICENSES.md` with version, SPDX and purpose,
   Rust and npm, with build-time-only tooling marked as not distributed.
9. **Screenshot-driven visual QA** across: empty index, small repository, large graph, long
   names, unresolved-heavy repository, narrow and wide viewports, loading and error states.
10. Keyboard navigation for search and primary navigation.
11. `nerve serve` shuts down cleanly and does not leave the database locked.

## Stop conditions

- If the chosen HTTP crate cannot bind loopback-only or drags in an unexpected dependency tree,
  stop and report rather than defaulting to axum.
- If any T4/T5/T6 control cannot be implemented, **do not ship the server**. A local server with
  a CSRF hole is worse than no server.
