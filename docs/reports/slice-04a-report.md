# Slice 4a — `nerve serve`: local HTTP API · completion report

**Date:** 2026-07-31 · **Status:** Complete, all criteria met
**Plan:** `docs/plans/slice-04-visual-explorer.md` (4a column) · **Gate:** `docs/THREAT-MODEL.md` T4/T5/T6

---

## Summary

`nerve serve` starts a loopback-only, read-only HTTP API over the evidence graph. Nine endpoints
expose overview, search, entity detail, bounded neighbourhood, path, evidence (`why`), source
snippets, unresolved relationships and partial parses — all calling the same `nerve-store`
functions the CLI uses.

The three blocking security controls are implemented and **verified by attacks the orchestrator
constructed independently**, not only by the shipped tests.

## Scope split

Slice 4 was first dispatched whole — server, security, SPA, embedding and screenshot QA — and the
agent stalled without producing a change. That was an **orchestrator decomposition error**: the
slice bundled a security surface and a design product into one review. Split into 4a (this) and
4b (the SPA). Recorded in the plan.

## Files changed

**New crate `crates/nerve-server`** — `lib.rs` (`serve`, loopback bind, worker pool, one
`query_only` connection per worker), `token.rs` (256-bit token from `/dev/urandom`, constant-time
compare, redacted `Debug`), `guard.rs` (host/origin/token), `respond.rs` (hardened JSON encoder +
security headers), `request.rs` (bounded target parsing), `router.rs`, `api.rs` (nine endpoints,
no SQL, no filesystem), `shapes.rs`, `assets.rs` + placeholder page, and 59 integration tests.

**Changed** — `nerve-store`: `query.rs` (+5 read-only queries), `graph.rs` (`neighbourhood()`),
`lib.rs`. `nerve-index`: `probe.rs` (factored a private `resolve()` so `probe` and the new
`read_snippet` share **one** path-safety implementation), new `inspect.rs`, `lib.rs`.
`nerve-cli`: `main.rs` (`serve` + SIGINT/SIGTERM), `tests/cli.rs`. `nerve-store/tests/graph.rs`.
`Cargo.toml`, `Cargo.lock`, `third_party/LICENSES.md`.

**Verified untouched** — `fixtures/`, the four orchestrator-owned docs, the frozen extraction
modules, and `schema.rs`/`derive.rs`/`prune.rs`. `git diff --stat` empty for all. Schema stays v3.

## Dependency decision — axum was rejected on evidence

The plan's P1 forbade Tokio + axum for a loopback-only, single-user, read-only server. Adopted
`tiny_http 0.12.0` (MIT OR Apache-2.0, `default-features = false`, no TLS).

**Measured transitive cost: 3 new crates** (`ascii`, `chunked_transfer`, `httpdate`) — better than
the ~10 estimated. `ctrlc` was measured and rejected (7 crates on macOS via `nix`/`objc2`) in
favour of `signal-hook`.

| Crate | Version | SPDX |
|---|---|---|
| `tiny_http` | 0.12.0 | MIT OR Apache-2.0 |
| `ascii` | 1.1.0 | Apache-2.0 OR MIT |
| `chunked_transfer` | 1.5.0 | MIT OR Apache-2.0 |
| `httpdate` | 1.0.3 | MIT OR Apache-2.0 |
| `signal-hook` | 0.4.4 | MIT OR Apache-2.0 |
| `signal-hook-registry` | 1.4.8 | MIT OR Apache-2.0 |

93 → 100 crates (6 external + our own `nerve-server`), all permissive, all recorded.
`no_network` passes **unmodified** — `tiny_http` is a listener, never a client.

## Verification — run by the orchestrator

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 427 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

306 → 427 tests. No test weakened or deleted.

### Attacks constructed by the orchestrator against a live release server

| Probe | Result |
|---|---|
| no token | **401** `token_required` |
| wrong token | **403** `token_invalid` |
| correct token | 200 |
| `Origin: http://evil.test` | **403** `origin_not_allowed` |
| `Host: evil.test` | **403** `host_not_allowed` (DNS-rebinding defence) |
| `POST` with valid token | **405** `method_not_allowed` |
| `../../../etc/passwd`, `src/../../../etc/passwd`, `/etc/passwd` | **403**, no leak |
| `.env` containing `SECRET_TOKEN=leakme` | **403**, and the secret appears in **no** response |
| own XSS payload in a symbol name, file name and specifier | **0 raw `<`, 0 raw `>`**, escaped `<`, round-trips losslessly after JSON decode |

The single `leakme` hit across all endpoints was traced: it is the caller's own query echoed as
`"query":"leakme"` with `count:0` and empty results — not a disclosure.

**Headers on the served page:** `Content-Security-Policy: default-src 'none'; script-src 'self'; …`
(**no `unsafe-inline`**), `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
`Referrer-Policy: no-referrer`.

**Read-only:** database sha256 `980ea7e4…` byte-identical before and after a full API session
including every refused probe. Enforced structurally too — `PRAGMA query_only=ON` per worker.

**Clean shutdown:** SIGTERM → port closed (`URLError`), and a subsequent `nerve status` reports
`healthy yes`, so no lock is left behind.

## Accepted risk recorded in the threat model

`tiny_http` reads request header lines without an upper bound, so a local process (including
another local UID, since loopback is reachable by any) can exhaust memory. Impact is
**availability of the user's own dev tool**, not confidentiality or integrity — the header is
consumed before routing, so nothing leaks. It cannot be intercepted at Nerve's layer, and
replacing it means hand-rolling a parser: a bad trade for a local tool. Everything Nerve controls
*is* bounded (8 KiB target, 32 parameters, bodies refused unread, `Content-Length` framing,
chunking disabled). Documented as **T11, accepted**, with explicit revisit conditions.

## Known limitations

- T11 above, plus no connection read timeout (a test pins that the server stays responsive).
- `Host: localhost:PORT` is refused — strictly correct, a usability wart. `serve` prints the
  `127.0.0.1` URL.
- The token appears in the printed URL and therefore in shell history. Unavoidable: a terminal's
  only channel to a browser is a URL. Per-session, never persisted.
- `overview` freshness re-hashes up to 5000 files per request, then reports `truncated: true`.
- No request log — a log would be a write, and a place for repository strings to escape JSON.
- No UI yet. `assets.rs` serves a placeholder; 4b drops in the built SPA.

## Result

All acceptance criteria for the 4a column met.

**Next:** 4b — `apps/nerve-web`, consuming this API.
