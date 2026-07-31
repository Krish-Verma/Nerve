# Slice 2a — Static relationship resolution · completion report

**Date:** 2026-07-31 · **Status:** Complete · **Plan:** `docs/plans/slice-02-static-resolution.md`

---

## Summary

Nerve now resolves symbol-level relationships through lexical scope and import resolution.
`CALLS`, `REFERENCES`, `EXTENDS` and `IMPLEMENTS` are emitted by a second extractor,
`ts-js-reference 1.0.0`, with every resolved edge labelled `AST_RESOLVED` and every target it
could not name recorded as an `Unresolved` entity carrying a closed-vocabulary reason.

The measured result on the purpose-built corpus is **0 false positives and 0 false negatives
across 24 resolved edges**, with 38.1% of call sites honestly unresolved.

## Files changed

**New**

| Path | Purpose |
|---|---|
| `crates/nerve-index/src/bind.rs` | Lexical binding table — scope chain, 5 binding kinds, `Opaque` shadowing guard, `this` resolution with staticness |
| `crates/nerve-index/src/exports.rs` | Export map + transitive re-export closure, cycle-safe |
| `crates/nerve-index/src/refs.rs` | The `ts-js-reference` extractor and its `UnresolvedReason` vocabulary |
| `crates/nerve-index/tests/precision.rs` | The 5-gate precision harness plus evidence-labelling and rebuild tests |
| `fixtures/ts-resolution/` | 15-file precision corpus + `expected.json` ground truth + `README.md` |
| `docs/plans/slice-02-static-resolution.md` | Slice plan and pushback record |
| `docs/plans/slice-02b-query-surface.md` | Slice 2b specification |

**Modified** — `nerve-core`: `vocab.rs` (`UnresolvedCategory`), `ids.rs` (category discriminator),
`lib.rs`. `nerve-index`: `extract.rs` (→ 1.1.0), `pipeline.rs` (`Evidence`, second run,
`build_reference_graph`), `lib.rs`, `tests/common/mod.rs`, `tests/graph.rs`.
`nerve-store`: `query.rs` (`StatusReport.runs`). `nerve-cli`: `main.rs`, `tests/cli.rs`.
`docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `fixtures/ts-basic/golden.json`.

**Deliberately untouched** — `schema.rs`, `derive.rs`, `discover.rs`, `config.rs`,
`gitinfo.rs`, `init.rs`, `resolve.rs`.

## Architecture decisions

1. **A second extractor, not an extension of the first.** ADR-0003 makes measured precision a
   property of `(extractor_id, extractor_version)`. Slice 2a's resolution rules have entirely
   different precision characteristics from Slice 1's structural reading, so they get their own
   identity, their own declaration `[AST_DIRECT, AST_RESOLVED]`, and their own gate. Each index
   run now writes two `extractor_run` rows.
2. **`Evidence { source_type, directness }` as one value.** The two fields cannot drift apart,
   and a test asserts they never disagree.
3. **Table-first binding.** The scope tree is built before any reference is resolved, so
   hoisting is free and `scope_at` is a binary search over a pre-order layout.
4. **`Opaque` bindings shadow.** A parameter, plain variable, destructured field, catch binding,
   loop binding, type parameter, enum or namespace blocks resolution rather than falling through
   to a module-level symbol of the same name. This is the single load-bearing precision guard.
5. **Class type parameters bind `Opaque`.** Without this, `class Box<Shape> { x: Shape }`
   resolves `Shape` to a module interface — a real false positive, caught in review.
6. **`this.m()` requires matching staticness and rejects accessors**, in addition to the
   plan's own-class and no-crossed-boundary rules. `this.x()` on a getter invokes the accessor
   and calls *its result*; naming the accessor would be the wrong target.
7. **The whole callee expression is consumed by `CALLS`** and never also emits `REFERENCES`.
   Unmodelled callees are still walked, or `f()()` would drop the real inner call.
8. **P2 correction** (plan §P2): resolved `IMPORTS` and re-export `EXPORTS` are now
   `AST_RESOLVED`; unresolved `IMPORTS` correctly remain `AST_DIRECT`. `ts-js-structural` → 1.1.0.
9. **Identity discriminator** (orchestrator correction): the unresolved tuple is now
   `("unresolved", project_id, importer_rel_path, category, raw_name)` with
   `category ∈ {module, value}`. Without it, `import {x} from 'parse'` and a call to `parse()`
   in the same file collapsed into one entity.

## Verification — run by the orchestrator, not taken from the subagent

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 203 passed, 0 failed, 1 ignored
cargo build --release                                   → Finished in 3.90s
cargo test -p nerve-index --test precision -- --nocapture
```

Test totals by target: nerve-cli unit 3 · cli 15 · no_network 3 · nerve-core 20 ·
nerve-index lib 108 · graph 18 · precision 5 · safety 18 · nerve-store 4 · schema 9 ·
scale 0 (**1 ignored** — the opt-in scale test, unchanged from Slice 1).

### Measured precision

```
relation       TP   FP   FN unresolved  unresolved-rate
CALLS          13    0    0          8            38.1%
REFERENCES      4    0    0          0             0.0%
EXTENDS         4    0    0          0             0.0%
IMPLEMENTS      3    0    0          0             0.0%
totals: 32 modelled edges, 5 unmodelled call/heritage sites
        {call-result:1, computed-member:1, heritage-call:1, iife:1, require:1}
```

### Independent gate-validity check (mutation probes)

A gate that passes on the first try is weak evidence. The orchestrator mutated the
**implementation** — not the fixtures — and confirmed the gate fails with a diagnosable message,
then reverted both probes and re-verified byte-identical restoration:

| Probe | Result |
|---|---|
| Disabled the parameter-shadowing guard in `bind.rs::bind_pattern` | **FAILED as required** — `FALSE POSITIVE CALLS: shadow.ts#shadowedByParameter -> shadow.ts#target at src/shadow.ts:17` plus the matching `FORBIDDEN` line |
| Swapped `ThisReboundByNestedFunction` for `ThisNotInClassMethod` in `refs.rs` | **FAILED as required** — `MISSING UNRESOLVED` + `UNDECLARED UNRESOLVED` at `src/shapes.ts:29` |

The second probe changed only the *reason* on an otherwise-correct unresolved edge, and the
gate still caught it. The harness discriminates on evidence quality, not merely edge presence.

### Manual inspection

All 12 `ts-basic` `CALLS` edges were read by hand. Notable: the two `shared()` calls in
`ambiguous.ts` resolve to **different** entities (`fn_76bd10c8…` scoped to `outerA`,
`fn_07fadf65…` scoped to `outerB`) — the exact case Slice 1 refused to guess.
`legacyAdd → add` resolves through `const math = require('./math')`.
`describe → add` resolves through `import { add as plus }`.

P2 correction confirmed directly against the database:

| relation | AST_DIRECT | AST_RESOLVED |
|---|---|---|
| `IMPORTS` | 1 (the unresolved one) | 15 |
| `EXPORTS` | 33 (local) | 8 (re-export) |
| `CALLS` | 8 (unresolved) | 13 |

## Safety, security, clean-room

- `assertion_state` writer set unchanged — `grep` confirms `derive.rs` alone holds the
  `DELETE`/`INSERT`.
- **No source text at rest.** Observation `details` were dumped from the database and contain
  only identifiers, dotted member names and enumerated tags. Longest value (221 chars) is a
  Slice 1 import specifier list. SECURITY.md's rule holds by construction.
- **No new dependencies.** `Cargo.toml`, `Cargo.lock` and `third_party/LICENSES.md` are
  byte-identical. Zero networking or async crates in the tree; `no_network` tests green.
- **Clean-room:** no competitor name, URL, or network surface anywhere in the new code.
  No competitor skill invoked.
- No schema change; schema stays at v1. No migration needed.

## Golden diff

`fixtures/ts-basic/golden.json`: entities 44 → 48, assertions 68 → 87, observations 70 → 89.
New: 12 `CALLS`, 5 `REFERENCES`, 2 `IMPLEMENTS` (no `EXTENDS` — `ts-basic` has no extends
clause). Evidence split moves from 70 `AST_DIRECT` to 62 `AST_DIRECT` + 27 `AST_RESOLVED`;
12 of those are the P2 correction. Every unresolved-module entity id changed because of the new
category discriminator. 4 new `Unresolved` *value* entities.

## Known limitations

- **Recall is the weak axis, not precision.** Any method call on a typed receiver is
  `Unresolved` — 38.1% of `CALLS` on a corpus deliberately loaded with hard cases. Real
  repositories will be higher. This is plan §P6's measured-and-reported outcome.
- Recall is measured only against hand-authored ground truth. Recall on real-world repositories
  is **unmeasured** and cannot be measured without a type checker to compare against.
- `REFERENCES` is emitted for identifiers inside an unmodelled callee subtree (`a[b]()`).
- Object-literal shorthand yields no `REFERENCES`; `module.exports` is unmodelled, so CommonJS
  export surfaces are invisible.
- Destructuring default values (`function f({a = g()})`) are not walked; calls there are missed.
- Class field initializers do not bind `this`, so `class C { x = this.m() }` is `Unresolved`.
- `refs.rs` re-parses each file, roughly doubling parse cost. Deferred, not forced.
- `nerve status.runs` grows by two per re-index; `extractor_run` is a log and will want a
  `--all` flag.
- Two sites in one file failing on the same name share one `Unresolved` entity whose
  `meta.reason` is the first seen; per-site reasons are exact in observation `details`.

## Result

All ten acceptance criteria met. No rule was retracted; no negative fixture was weakened or
deleted; no gate threshold was lowered.

**Next slice:** 2b — `nerve path`, `nerve why`.
