# Slice 2 — Static relationship resolution

**Date:** 2026-07-31 · **Status:** Accepted, in progress · **Supersedes:** ROADMAP row 2 (scope split)

---

## Disagreements and Pushback

### P1 — Roadmap row 2 is two slices, not one

**Disputed assumption.** ROADMAP.md line 9 bundles `CALLS` + `REFERENCES` + `EXTENDS` +
`IMPLEMENTS` + module resolution + a precision harness + `nerve path` + `nerve why` into one
slice.

**Concern.** That is a resolver, a measurement apparatus, and two query surfaces. CLAUDE.md §4
requires bite-sized vertical slices. A single subagent task of that size is where scope creep
and unreviewed compromise enter.

**Evidence.** Slice 1 was 7,733 lines across 4 crates for a strictly smaller surface
(4 structural relations, no resolution, no measurement). The resolver alone needs a lexical
binding table, an import-binding model, a re-export closure, and a `this`-receiver rule — none
of which exist in the repository today (`crates/nerve-index/src/resolve.rs` resolves module
specifiers only, 172 lines).

**Decision.** Split into two sequential slices. Not deferred, not dropped — both run in this
session, back to back.

| | Scope |
|---|---|
| **2a** | Binding resolution, `CALLS` `REFERENCES` `EXTENDS` `IMPLEMENTS`, the `ts-js-reference` extractor, negative fixtures, measured precision gated in CI |
| **2b** | `nerve path`, `nerve why`, evidence packets over the graph 2a produces |

2a is not "infrastructure only" (§17): `nerve index`, `nerve status` and `nerve search`
all report the new relations, and the precision report is a committed artifact.

**Blocks the slice?** No — it sequences it.

### P2 — Slice 1 mislabels resolved imports as `AST_DIRECT`. That is a real defect.

**Disputed assumption.** That the Slice 1 evidence labelling is correct and untouchable.

**Concern.** `crates/nerve-index/src/pipeline.rs:633-679` gives a *resolved* import
`Directness::Resolved` but `GraphBuilder::observe` hardcodes
`EvidenceSourceType::AstDirect` (pipeline.rs:154). ADR-0003 defines `AST_RESOLVED` as
"resolved through import/module resolution" — which is literally what
`resolve::resolve()` did. The same applies to re-export `EXPORTS` edges (pipeline.rs:604).

**Why it matters.** The product thesis is that the evidence-type distinction is real and
queryable. An edge produced by module resolution and labelled "the syntax tree literally
contains this" falsifies that claim in the first slice. It also makes the Slice 2 precision
story incoherent: 2a's resolved edges would be `AST_RESOLVED` while Slice 1's equally-resolved
import edges stay `AST_DIRECT`.

**Recommendation.** Fix in 2a. Resolved `IMPORTS` and re-export `EXPORTS` become
`AST_RESOLVED`; unresolved `IMPORTS` stay `AST_DIRECT` (the tree literally states a specifier
and nothing was resolved). Bump `ts-js-structural` to `1.1.0` — it changes what the extractor
emits, which is exactly what the version field is for. Regenerate the golden file.

**Blocks the slice?** No. Corrective, local, covered by the golden diff.

### P3 — `this.method()` resolution is the highest false-positive risk in the slice

**Disputed assumption.** That `this.m()` inside class `C` means "calls `C.m`".

**Concern.** It does not, in two common cases: (a) inside a nested non-arrow `function`, `this`
is not the instance; (b) `m` may be inherited from a base class and not declared on `C`.
Resolving either would be a name match wearing a resolution label — precisely what
master plan §3.3 refused to ship in Slice 1.

**Decision.** Resolve `this.m()` only when **both** hold: the innermost `this`-binding scope is
a method or constructor of a class declared in this module (no non-arrow `function` boundary
crossed), and that class **itself declares** `m`. Otherwise `Unresolved`, with the reason
recorded. Both failure modes get negative fixtures.

**Blocks the slice?** No — it constrains a rule.

### P4 — `REFERENCES` must be bounded or precision becomes unmeasurable

**Disputed assumption.** That `REFERENCES` means "every identifier mention".

**Concern.** Emitting an edge per identifier makes the graph mostly local-variable noise, and
makes the precision denominator dominated by cases with no cross-boundary information.

**Decision.** `REFERENCES` is emitted **only for resolved targets** — a module-scope symbol of
this module, or an import-resolved entity in another indexed module. Unresolved identifiers
produce no `REFERENCES` edge.

`CALLS` is deliberately asymmetric: an unresolved *call* is emitted, because "this code invokes
something Nerve cannot name" is the fact `nerve gaps` exists to report. An unresolved bare
identifier is usually a local or a global and carries no such fact.

**Blocks the slice?** No.

### P5 — Barrel files require a transitive re-export closure

`fixtures/ts-basic/src/index.ts` is a barrel (`export { add as plus } from './math'`;
`export * from './shapes'`). Without a transitive export map, `import { plus } from './index'`
followed by `plus()` cannot resolve, and recall on any realistic TypeScript repository
collapses. The closure needs cycle protection — barrel cycles are common and must terminate.

**Blocks the slice?** No — it is required scope.

### P6 — Unresolved-call fan-out is a measurable risk, not a reason to cap silently

One `Unresolved` entity per `(file, callee text)` could bloat the graph on real repositories
(`console.log`, `Math.PI`, every method call on a typed parameter). §10 forbids silently
broadening or narrowing heuristics to make numbers look good.

**Decision.** Emit them, **measure the ratio**, and report it. If it is pathological, a query-time
evidence policy in a later slice filters it — the data stays honest.

**Blocks the slice?** No.

---

## Slice 2a

### Objective

Nerve resolves and records symbol-level relationships — `CALLS`, `REFERENCES`, `EXTENDS`,
`IMPLEMENTS` — through lexical scope and import resolution, with every resolved edge labelled
`AST_RESOLVED`, every unnameable target recorded as `Unresolved`, and a measured precision
number gated in CI.

### User value

After 2a, `nerve index` answers "what does this function call?" and "which class implements
this interface?" with edges a reader can verify, and states plainly which call sites it could
not resolve.

### Scope

1. **`crates/nerve-index/src/bind.rs`** — lexical binding table.
   - Scope chain: module, function, block, class, catch.
   - Binding kinds: `LocalSymbol{entity_id, kind}`, `ImportedNamed{specifier, imported}`,
     `ImportedDefault{specifier}`, `ImportedNamespace{specifier}`, `Opaque`.
   - `Opaque` covers parameters, plain variables, destructuring patterns, catch params,
     loop bindings — names Nerve knows exist but that are not Nerve entities. An `Opaque`
     binding **shadows** an outer one and blocks resolution. This is the shadowing guard.
   - `const X = require('./m')` binds `ImportedNamespace`.
2. **`crates/nerve-index/src/exports.rs`** — per-module export map plus transitive re-export
   closure with cycle protection. `export *` does not re-export `default`.
3. **`crates/nerve-index/src/refs.rs`** — the `ts-js-reference` extractor.
4. **Callee forms modelled** — everything else is counted as an unmodelled call site and
   produces no edge:

   | Form | Behaviour |
   |---|---|
   | `foo()` | resolve `foo` through the binding chain |
   | `new Foo()` | as above, `call_form: "new"`, target is the `Class` entity |
   | `this.m()` | P3 rule |
   | `ns.foo()` where `ns` is a namespace/require binding | resolve `foo` as an export of the target module |
   | `obj.foo()`, `obj?.foo()` for any other simple identifier | `Unresolved`, reason `receiver-not-resolvable` |
   | `a[b]()`, `f()()`, IIFE, tagged template, super calls | **unmodelled**, counted, no edge |

5. **Heritage** — `class A extends B`, `class A implements I, J`, `interface I extends J`.
   Generic arguments are stripped to the head identifier. `class A extends mixin(B)` is
   unmodelled and counted.
6. **`REFERENCES`** — resolved-only, per P4. Value positions and type positions. Excluded:
   call-callee position (that is `CALLS`), heritage clauses, import/export statements, and the
   declaration site itself.
7. **Source entity** — the innermost enclosing symbol entity; the module entity when a site
   sits at module top level or inside an unnamed function.
8. **P2 correction** to `ts-js-structural`, version → `1.1.0`.
9. **Two extractor runs per index**, one row each in `extractor_run`.
10. **`fixtures/ts-resolution/`** — the precision corpus, with `expected.json` ground truth.
11. **`nerve status`** reports every run for the current state, not only the last.

### Non-goals

No type inference. No `tsconfig` path aliases, `node_modules`, or workspace resolution
(`resolve.rs` stays relative-only). No cross-file flow analysis. No framework rules. No
`nerve path` / `nerve why` — that is 2b. No schema migration: `relation` and
`evidence_source_type` are `TEXT` over closed vocabularies that already contain every value
2a needs, so schema stays at v1.

### Acceptance criteria

1. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, `cargo build --release` all pass.
2. **Precision gate — no undeclared resolved edge.** Every `AST_RESOLVED` `CALLS`/`REFERENCES`/
   `EXTENDS`/`IMPLEMENTS` edge in `fixtures/ts-resolution` appears in `expected.json`.
   A resolved edge not in ground truth is a false positive and fails the build.
   **Gate: false positives = 0.**
3. **Recall gate.** Every edge in `expected.resolved` exists, with `AST_RESOLVED`.
   **Gate: false negatives = 0** on the corpus.
4. **Negative gate.** No edge in `expected.forbidden` exists in any form — resolved *or*
   unresolved.
5. **Unresolved gate.** Every entry in `expected.unresolved` exists as an edge to an
   `Unresolved` entity carrying the stated `reason`.
6. The precision harness **prints** TP / FP / FN / unresolved-rate per relation.
7. Golden dump regenerated and reviewed; determinism and idempotence tests still pass.
8. `ts-js-reference` emits nothing outside `[AST_DIRECT, AST_RESOLVED]`
   (`verify_declared_source_types` enforces it).
9. `assertion_state` is still a pure rebuild — the truncate-and-rebuild test passes with the
   new relations present.
10. No extractor writes `assertion_state`. `grep` proves the writer set is unchanged.

### Tests required

Unit: binding chain and shadowing · import-binding resolution · re-export closure incl. cycles ·
`this` rule both ways · callee-form classification · heritage with generics · unmodelled-form
counting.
Integration: precision harness over `fixtures/ts-resolution` · golden dump ·
determinism · idempotence · unresolved-reason coverage · two-extractor-run persistence ·
`assertion_state` rebuild equivalence · declared-source-type enforcement.

### Stop conditions

- Precision gate cannot reach zero false positives without deleting a legitimate negative
  fixture → **retract the offending rule**, do not weaken the fixture (§10.3).
- Resolution requires type inference to be correct at all → record the case as `Unresolved`
  with a reason and document the limit; do not guess.

---

## Slice 2b (scope only; planned after 2a lands)

`nerve path <from> <to>` — bounded-depth shortest paths over the assertion graph with relation
filters. `nerve why <from> [<to>]` — the evidence packet for an assertion: every observation,
its source type, directness, extractor id+version, `file:line`, content hash, and whether that
hash still matches the file on disk (freshness derived, not stored). Entity selectors resolve
by id or by qualified name; ambiguity is reported with candidates and exit code 10, never
silently resolved to one.
