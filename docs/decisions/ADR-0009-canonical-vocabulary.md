# ADR-0009 — The canonical vocabulary mechanism is a drift guard, not a generator

**Status:** Accepted · **Date:** 2026-08-11 · **Slice:** functional UI parity

---

## Context

This project has repeatedly suffered frontend/backend vocabulary drift. Slice 5d-iii was a
corrective slice for it and found **120 real sites** rendering fallback text, plus one gloss for a
status the backend could not emit. Slice 12c-iv found **eight** vocabularies the interface did not
mirror — and, worse, for which the guard *could not fail*, because it did not know they existed.

The remaining-work brief requires that by final UI parity there be **one canonical vocabulary
mechanism**, and offers two candidate shapes:

1. an `/api/meta` or `/api/vocabulary` endpoint the interface reads at runtime; or
2. a generated frontend artifact produced from the Rust vocabularies at build time.

It also states the prohibition that actually matters: *do not maintain independent manually
handwritten Rust and frontend vocabularies **without an automated drift guard**.*

## Decision

**Neither candidate is adopted. The existing mechanism — `crates/nerve-server/tests/ui_vocabulary.rs`
— is the canonical mechanism, and it already satisfies the prohibition.**

It is a test that reads the interface's TypeScript gloss maps and fails when a Rust vocabulary
gains a member the interface does not gloss. Two lists carry the contract:

| list | meaning | current |
|---|---|---|
| `GLOSS_TABLES` | vocabularies the interface renders, each guarded | **36** |
| `DECLARED_NOT_RENDERED` | vocabularies declared in Rust and deliberately not rendered yet | **0** |

`DECLARED_NOT_RENDERED` being **empty** is the strongest available statement of parity: there is no
backend vocabulary the interface declares and does not render. It has not been empty before.

## Why not a generator or an endpoint

**The glosses are not derivable.** A gloss is UI-facing prose explaining what a value *means to a
reader*. Several vocabularies do carry a Rust `note()`, but those are written for the CLI and for
MCP, and they are deliberately not always the sentence a screen should show. Generating the
interface's text from `note()` would either force one wording onto two audiences or require a
second field in Rust whose only consumer is the generator — at which point the "single source" is
two fields again.

**An endpoint moves the failure from build time to run time.** `/api/vocabulary` would let a
missing gloss ship and fail in front of a user, whereas the guard fails in `cargo test`. It would
also add a route to a surface whose route table is deliberately small and parity-tested in both
directions.

**A generator adds a build step this project does not have.** There is no codegen infrastructure,
and introducing one to solve a problem a passing test already solves would be new machinery
carrying new failure modes.

**And the guard forbids the thing that actually hurts.** Drift is not "the wording differs" — it is
**absence**: a value the backend can emit that the interface renders as fallback text, or does not
know about at all. The guard makes absence a build failure while leaving wording free, which is the
correct split for two audiences.

## Evidence that it is load-bearing

It is not trusted on inspection. Every extension of it has been probe-verified, and two probes were
run on the day this ADR was written:

| probe | result |
|---|---|
| delete `process` from `MEMORY_SCOPE` | `every_memory_scope_is_glossed_and_ownership_is_not_one` FAILS — *"MemoryScope::ALL has 1 member(s) with no entry"* |
| delete `unverified` from `TRACE_BINDING` | `every_trace_repository_binding_is_glossed` FAILS by name |

Both reverted and verified byte-identical. Earlier slices probed it the same way: 12c-iv's removal
of one gloss fails `every_change_kind_is_glossed`, and 5d-iii's original version found the 120
sites rather than being written to pass over them.

It also guards **32 distinct `::ALL` vocabularies spanning three crates**, not just `nerve-core`'s
27 — so it is not scoped to one module's idea of what a vocabulary is.

There is a second guard beside it: the embedded bundle is checked against the interface source, so
a screen that was edited and never re-embedded fails rather than silently shipping the old build.

## Consequences

- Adding a member to any guarded Rust vocabulary **fails the build** until the interface glosses it.
  That is the intended cost and it has been paid five times in row 14 alone.
- A new vocabulary must be added to `GLOSS_TABLES` (or, with a stated reason, to
  `DECLARED_NOT_RENDERED`) in the **same change** that introduces it. 12c-iv is the precedent: a
  guard that does not know a vocabulary exists cannot fail for it, which is the failure mode that
  let eight drift.
- `DECLARED_NOT_RENDERED` should stay empty. A non-empty entry is a declared, dated gap — not a
  parking space.
- If a future networked or multi-client mode appears, revisit: an endpoint becomes attractive when
  the interface is not built from this repository. That is the condition, and it does not hold
  today.

## Rejected alternative, recorded

`/api/meta` returning every vocabulary with its glosses. Rejected for the run-time-versus-build-time
reason above, and because the one existing precedent — `/api/contracts/vocabulary` — serves a
*different* need: it returns contract-rule metadata a client cannot know statically, not display
prose the client already ships.
