# ADR-0004 — Rust for the engine, TypeScript for the visual surface

**Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 1

## Decision

The Nerve engine (core model, store, indexer, CLI, future local server) is **Rust**.
The future visual application is **TypeScript + React**, served by the Rust binary with its
built assets embedded.

## Rationale

- **Distribution.** Offline-first means a user should get a working `nerve` without installing
  a language runtime. Rust produces a single self-contained binary per platform. A Node
  implementation would require the user to have a compatible Node runtime, and a Python one
  would be worse still.
- **Tree-sitter.** The canonical implementation is C with maintained first-class Rust
  bindings; grammar crates compile the C grammar directly. The WASM route costs parse
  throughput and adds a runtime.
- **SQLite bundling.** `rusqlite`'s `bundled` feature statically links a SQLite we compile,
  fixing the version and feature set (notably FTS5) across every platform. This removes an
  entire class of "works on my machine" failures.
- **Predictable performance and memory** for whole-repository indexing, without GC pauses.

## Costs accepted

- Slower iteration than TypeScript, and a language boundary at the UI. Mitigated by keeping
  **all** query logic server-side so the UI is a thin client rather than a second
  implementation of the model.
- Contributors need a Rust toolchain. Pinned via `rust-toolchain.toml`.

## Alternatives considered

- **TypeScript/Node for everything.** One language end to end and faster iteration, but
  requires a user runtime, pushes us to WASM tree-sitter or a native addon, and makes SQLite
  feature availability host-dependent. Rejected on distribution grounds.
- **Go.** Good single-binary story, but tree-sitter and SQLite both arrive via cgo, which
  surrenders most of the cross-compilation benefit.

## Constraints

- **No async runtime in Slice 1.** Indexing is CPU/disk bound; Tokio would add surface with no
  benefit. Introduce only when `nerve serve` needs it.
- **No networking crate in the dependency tree**, enforced by test.
- Dependencies are added only with a stated reason and a recorded license.
