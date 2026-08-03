# Slice 9b — Python references, and a measured gate of their own

2026-08-03. Plan: `docs/plans/slice-09-python.md` §9b. Follows Slice 9a (`4e53d82`).
**Row 9 is now complete.**

---

## Objective

`py-reference`: `CALLS`, `REFERENCES`, `EXTENDS` for Python, with precision measured and reported
**independently of the TypeScript numbers**.

## Measured accuracy — the point of the slice

`fixtures/py-resolution`, 8 files, ground truth written **before** the resolver.

```
relation       TP   FP   FN unresolved  unresolved-rate
CALLS          15    0    3         11            42.3%
REFERENCES      7    0    0          0             0.0%
EXTENDS         3    0    0          1            25.0%
(all)          25    0    3         12            32.4%
IMPLEMENTS      -    -    -          - not a Python relation
totals: 37 modelled edges, 8 unmodelled call/base-class sites
  {call-result:1, computed-member:1, dynamic-import:2, heritage-call:1,
   heritage-other:1, iife:1, super:1}
```

**FP = 0**, as the gate requires. **FN = 3, declared rather than hidden**, each pinned in
`expected.known_false_negatives` with its reason and each required to stay *absent* — so nobody can
"fix" an FN by guessing without the gate telling them to promote it deliberately.

TS/JS re-run by me, unchanged — this is the regression that mattered, since the slice touches
shared pipeline code:

```
CALLS 13/0/0 · 38.1%   REFERENCES 4/0/0   EXTENDS 4/0/0   IMPLEMENTS 3/0/0
```

**Two languages, two tables, no combined number.** A merged figure would let TypeScript's result
hide Python's.

### Why 42.3%, and why that is the correct answer

`self.method()` is unresolved, and that single decision is most of the gap. The reasoning survived
my attempt to break it.

My first reaction was that it is over-conservative: `self` is determinable from the AST as a
method's first parameter, and `@staticmethod` / `@classmethod` are detectable because 9a already
records decorators as metadata. So Nerve *could* know what `self` is.

The fixture's stated reason defeats that — **"a subclass instance is what is usually passed."**
Knowing that `self` is an instance of `Engine` does not tell you what `self.start()` calls: if
`Turbo(Engine)` overrides `start`, it calls `Turbo.start`. Resolving to `Engine.start` would be a
**false positive**, not a harmless over-approximation. Dynamic dispatch makes the target genuinely
unknowable.

TypeScript's `this` is a *language binding*; Python's `self` is a parameter name — `def m(this)`
means the same thing. That is why the two languages differ, and why the gap is a property of Python
rather than a weakness in the extractor.

### Unmodelled sites are counted, not dropped

`super`, `iife`, `heritage-call`, `computed-member`, `dynamic-import`, `call-result`,
`heritage-other` — 8 sites, tallied by form and asserted as **gate 7**. The denominator of the
precision figure is therefore auditable, and a silently growing set of unmodelled forms fails the
build. "Absence is not zero" applied to the measurement apparatus itself.

## The upgrade-path bug this slice found, which no test would have caught

The implementer bumped `py-structural` 1.0.0 → 1.1.0 and argued the bump was *required*, not
cosmetic. **I verified it by experiment rather than by reading the argument.**

The Python cache-hit check is
`row.structural_version == pystruct::VERSION && row.reference_version == pyrefs::VERSION`
(`pipeline.rs:1512`). Before this slice a Python row stored `("1.0.0", "1.0.0")` — the reference
column was written from `pystruct::EXTRACTOR_VERSION`. After it, the reference column comes from
`pyrefs::EXTRACTOR_VERSION`, which is *also* `"1.0.0"`.

So without the structural bump, an existing index would compare `("1.0.0","1.0.0")` against
`("1.0.0","1.0.0")`, hit the cache on every unchanged `.py` file, and **produce zero Python
reference edges forever**.

I simulated a 9a-era index by rewriting the cache rows and re-indexing:

```
before:  structural_version | reference_version  =  1.0.0 | 1.0.0     (simulated 9a index)
re-index: re-extracted 9 of 10 (1 skipped unchanged)
after:   structural_version | reference_version  =  1.1.0 | 1.0.0
py-reference edges: 37
```

**Correctly a cache miss.** Every test in the suite builds a fresh index, so this defect would have
been invisible to all of them and would have shipped as "Python calls silently missing after
upgrade". The pin now reads `nerve_index::PYTHON_EXTRACTOR_VERSION` rather than a literal, so it
cannot rot again.

## Python scoping, modelled rather than approximated

`pybind.rs` implements four rules, each of which changes an answer:

1. **A binding covers the whole function body**, not the text after it. `def f(): total = scale;
   scale = 2` makes `scale` local on *both* lines — CPython raises `UnboundLocalError` on the
   first. A backwards-looking guard would resolve it and be wrong.
2. **A class body is not an enclosing scope for its methods.** `class C: x = 1; def m(self): return x`
   reads the *module's* `x`.
3. **`global` / `nonlocal` redirect the walk.**
4. **Comprehensions get their own scope.**

`__all__` is deliberately **not** consulted when resolving `from m import n`. It governs
`from m import *`, which Nerve refuses; using it to gate a plain named import would refuse edges
Python actually allows.

Cross-module re-export chains are followed one hop: `pkg/__init__.py` writing
`from .core import Engine` makes `pkg.Engine` *be* `pkg.core.Engine` — a Python fact, not a name
coincidence. Conditional imports are excluded, because whether `try: from .x import Y` bound
anything is not statically decidable and 9a already records that as a finding.

## Files changed

**New:** `crates/nerve-index/src/pyrefs.rs`, `src/pybind.rs`, `src/pysurface.rs`,
`tests/py_precision.rs`, `fixtures/py-resolution/` (8 files + `expected.json` + `README.md`).

**Modified:** `src/pipeline.rs`, `src/facts.rs`, `src/lib.rs`, `src/pystruct.rs`,
`crates/nerve-cli/src/main.rs`, and four test files.

**`nerve-store`, `nerve-core` and `apps/nerve-web/` untouched.** No schema change, no migration,
no new `EntityKind` / `Relation` / `UnresolvedCategory`, no dependency — `Cargo.lock` still **101**.

## Tests

**1058 passed / 0 failed / 2 ignored**, up from 1012. **+46.**

## Verification

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace --no-fail-fast                  → 1058 passed, 0 failed, 2 ignored
cargo build --release                                  → Finished, exit 0
```

## Mutation probes — the implementer's two, and a third of mine

| probe | result |
|---|---|
| relax the shadowing guard | gate fails with 10 problems, `FORBIDDEN` reported **first**. Both the read *above* and *below* the assignment fired — the whole-function-body rule, which a backwards-looking guard gets wrong. 4 `pybind` unit tests also fail |
| resolve a method call by name matching | gate fails with 5. FP appears as required — and note the second one: `self.start()` in `pkg/core.py` resolved to a method **on a different class in a different file** |
| **mine — make a class body an enclosing scope for its methods** | **2 targets fail.** `Scoped.use` calling `helper()` loses its correct edge to the *module* function and becomes unresolved; rate moves 42.3% → 46.2%. Precise failure: *"a class body is not an enclosing scope for its methods, so the module function wins"* |

All reverted; `pybind.rs` diffed byte-identical afterwards; gate re-run at 1058.

## The `nerve impact` wording defect, fixed

9a's report flagged that `nerve impact` on a Python symbol printed *"every reference site Nerve
indexed under those relations resolved"* — true only while there were zero Python reference sites.
The implementer **built the actual zero case** (a Python repository with definitions and no calls)
and confirmed it still printed that over zero sites: vacuously true, reads as coverage. Now:

> No reference site under those relations failed to resolve. That is a count of failed
> resolutions, not of coverage: a construct Nerve does not model — or a language it does not yet
> resolve — contributes no site to count.

## Deviation from CLAUDE.md §4

The implementation agent was terminated by an **org monthly spend limit** mid-slice — the second
time this has happened on this project (Slice 7b was the first). Its work was inspected rather than
discarded and it was resumed from its transcript. At the point of interruption four tests were
failing on mechanical pins and a scratch AST-dumping file (`zz_scratch_ast.rs`) was present; both
were finished on resume, and the scratch file is deleted. The orchestrator wrote no implementation
code but ran the third mutation probe and the upgrade-path experiment directly.

## Decisions recorded

- **No `IMPLEMENTS` for Python.** No such keyword; ABCs and protocols are inheritance or duck
  typing, and asserting it from `class C(SomeABC)` would be a guess.
- **A decorator is not a call edge** — 9a records it as metadata — **but its arguments are walked.**
  `@deco(times=compute())` yields `CALLS compute` attributed to the decorated symbol, not the
  module. Pinned by a unit test and by the fixture. This is the implementer's decision, not the
  plan's; reversing it would be deliberate, not a bug fix.
- **Reason tags are borrowed from `ts-js-reference`**, so the UI gloss stays complete with no mirror
  edit. Cost: `import-name-not-exported` reads oddly for a language with no export keyword. The
  distinction is right, the word is borrowed.

## Known limitations

- **`self.method()` is unresolved**, and that is correct (above). It is the largest single
  contributor to 42.3%.
- **Inherited members are unresolved** — walking the MRO stops at the same dynamic-dispatch wall.
- **Wildcard-bound names are unresolved**, per 9a.
- **`importlib.import_module("literal")` remains unresolved** — the open decision recorded in 9a,
  unchanged here.
- **The corpus is 8 files.** These are fixture numbers and a regression gate, **not** a claim about
  real-world Python accuracy. Real-world measurement is the validation phase.

## UI backend handoff changes

**None.** No vocabulary gained a member; reason tags are reused, so no TypeScript mirror changed.

## Commit

`6e0412e` — *feat: Slice 9b — Python references, and a cache key that would have hidden them*.

## Next slice

**10 — Framework rules.** 9a's decorator metadata is the input it will consume.
