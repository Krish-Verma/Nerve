# Slice 9a — Python structure

2026-08-02. Plan: `docs/plans/slice-09-python.md` §9a. Follows Slice 8b-ii (`b40458d`).

---

## Objective

Index Python — `Module`, `Function`, `Class`, `Method`, and `CONTAINS` / `DEFINES` / `IMPORTS` /
`EXPORTS` — resolving what can be resolved and recording the rest as explicit `Unresolved` values.

## The dependency, and its measured cost

`tree-sitter-python 0.25.0`, **MIT**, from the same organisation as the two grammars already
present. **`Cargo.lock` went 100 → 101 — exactly one new package.** Its dependencies
(`tree-sitter-language 0.1.7`, `cc 1.4.0`) were already in the tree, and its `0.25` line matches
`tree-sitter 0.25.10`, so no existing grammar was bumped. Third-party total 95 → 96. Verified by
`git diff Cargo.lock`: one added `name =` line.

`tree-sitter-stack-graphs-python` was rejected on clean-room grounds and the rejection is recorded
in `third_party/LICENSES.md` with the distinction CLAUDE.md §1 draws: a bare grammar is a parser
and answers no question about what a name *means*; stack-graphs is a name-resolution engine, and
depending on one would make Nerve's answers someone else's.

## The invariant this slice existed to protect

**Zero `ts-js-*` observations in a Python repository.** Slice 5d-i was a corrective slice for
directory containment stamped `ts-js-structural` in a repository with no TypeScript. Verified by me
on the real database:

```
fs-structural | 15      md-structural | 5      py-structural | 60
```

New modules (`pystruct.rs`, `pyresolve.rs`), new extractor id `py-structural 1.0.0`, not a branch
inside `extract.rs`.

## The design decision worth keeping: an import site makes two separable claims

`from pkg.util import *` says two things, and they have different epistemic status:

- **which module it names** — `pkg/util.py`, perfectly resolvable;
- **which names it binds** — unknowable without running the module.

The implementer recorded both. Verified on the real database:

```
module      | util               | pkg/util.py        ← the true edge, kept
unresolved  | wildcard:pkg.util  | unsupported.py     ← the unknowable part, recorded
```

Same for conditional imports: `try: from pkg.core import Engine` yields a resolved
`module → pkg/core.py` **and** `unresolved:conditional:pkg.core`.

The obvious alternative — refuse the module edge too, because "the import is unsupported" — deletes
a true fact to express an unrelated doubt. **I probed exactly that** (below); it is pinned.

`UnresolvedCategory` needed no new member: `Module` for a specifier naming nothing indexed, `Value`
for the bindings. The `wildcard:` / `conditional:` / `dynamic:` prefixes keep two findings about one
specifier distinct.

## Decisions recorded

- **No new `EntityKind`.** A package is the `__init__.py` module with packageness in `meta`.
- **`EXPORTS` comes only from `__all__`.** Python has no export keyword; promoting underscore-free
  top-level names would be a convention, not evidence.
- **`from pkg import name` names `pkg`, never `pkg.name`.** Attribute-vs-submodule needs
  cross-module reasoning. Listed as `forbidden` in the fixture so nobody "fixes" it by guessing.
- **`sys.path` mutation poisons absolute resolution in that module only.** This deliberately drops
  an edge that would otherwise resolve. Detection is a stated lower bound —
  `from sys import path; path.append(...)` is not caught.
- **No `IMPLEMENTS`.** Python has no such keyword; that would be 9b's guess to make and it will not.

## The open question the implementer raised, and where it actually lands

The plan put `importlib.import_module("literal")` unconditionally in the unresolved bucket. The
implementer flagged that `ts-js-structural` already treats `import('./x')` **with a string literal**
as a real import (`ImportForm::DynamicLiteral`) and asked whether that is an unjustified asymmetry.

**The observation is right and sharper than it looks.** Reading `extract.rs:712–736`, there are
three levels, not two:

| form | how it is recognised | assumption |
|---|---|---|
| `import('./x')` | `function.kind() == "import"` — a **grammar node** | none; it cannot be anything else |
| `require('x')` | `function.kind() == "identifier"` **and the text is `require`** | one: that `require` is CommonJS's |
| `importlib.import_module('x')` | attribute access on an imported name | two: that `importlib` is the real one, and `.import_module` is |

So Nerve *already* accepts one level of name-based assumption, and Python's case is one level
further out — where CLAUDE.md §3's "identity is never established by fuzzy name matching alone"
starts to bite.

**The conservative choice was the right one to ship, and not only because the plan said so: it is
the reversible one.** The implementer preserved the literal in `details.literal_argument`, so the
evidence survives and the decision can be made later without re-deriving it. Adding an edge later
is easy; withdrawing one that has been trusted is not. **Recorded as an open decision, not a
settled one.**

## Ground truth first, and it won

`fixtures/py-basic/expected.json` was written before the resolver. The first implementation emitted
both `dynamic:` and `conditional:` entities for `importlib.import_module(name)` inside a function;
the ground truth listed only `dynamic:`. **The fixture won** — a dynamic import is an *expression*
that binds no name at module scope, so "does this binding take effect?" is a question it never
raises. Same discipline as Slice 5d-ii.

## Files changed

**New:** `crates/nerve-index/src/pystruct.rs`, `src/pyresolve.rs`, `tests/python.rs`,
`fixtures/py-basic/` (11 `.py` files + `expected.json` + `README.md`).

**Modified:** `Cargo.toml`, `Cargo.lock`, `crates/nerve-index/Cargo.toml`, `src/lang.rs`,
`src/lib.rs`, `src/pipeline.rs`, `src/facts.rs`, `src/incremental.rs`, `third_party/LICENSES.md`,
and four test files.

**`nerve-store` untouched — no schema change, no migration.** No `apps/nerve-web/` edit. No new
`EntityKind`, `Relation` or `UnresolvedCategory`.

## Tests

**1012 passed / 0 failed / 2 ignored**, up from 970. **+42.**

## Verification

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace --no-fail-fast                  → 1012 passed, 0 failed, 2 ignored
cargo build --release                                  → Finished, exit 0
```

**TS/JS precision gate re-run by me and unchanged** — the slice touched shared pipeline code, so
this is the regression that mattered:

```
relation       TP   FP   FN unresolved  unresolved-rate
CALLS          13    0    0          8            38.1%
REFERENCES      4    0    0          0             0.0%
EXTENDS         4    0    0          0             0.0%
IMPLEMENTS      3    0    0          0             0.0%
```

### Orchestrator smoke test, release binary, fixture copy

68 entities (11 module, 14 function, 5 method, 2 class, 15 unresolved, plus the tree), 21 symbols,
0 files failed. The unresolved set reads as a list of honest refusals rather than a gap:

```
....                              conditional:pkg          conditional:pkg.core
dynamic:__import__                dynamic:importlib.import_module
wildcard:pkg.util                 nspkg (namespace package)
os · sys · functools · importlib (stdlib, outside the repository)
```

### T1 attacked directly, and the check is not vacuous

A hostile repository whose `setup.py` writes a marker, calls `os.system` and `subprocess.run` at
parse time, plus a module doing the same at top level, plus a `conftest.py`:

```
markers created: 0
python entities indexed: 9  →  Bomb, build, detonate all extracted
```

Nerve read the hostile code without executing any of it.

## Mutation probes — the implementer's three, and a fourth of mine

| probe | result |
|---|---|
| stamp Python observations `ts-js-structural` | criterion 4 fails: *"a repository with no TypeScript in it produced [("ts-js-structural", 6)]"* |
| guess a wildcard's bindings | negative test fails: `missing: ["unsupported.py -> value:wildcard:pkg.util"]` |
| **implementer's extra** — guess `from pkg import name` → `pkg/name.py` | **found a never-firing test.** The `forbidden` list sat behind a set-equality assertion and was unreachable. Reordered to run first; the probe then failed with the named wrong answer |
| **mine** — drop the resolved module edge for a wildcard (the plausible-but-wrong conservative alternative) | **2 tests fail**, diff shows the true edge `unsupported.py -> pkg/util.py` replaced by a false `module:pkg.util` refusal |

The third is the one I most value: the implementer ran a probe it was not asked for, *because* the
brief warned about never-firing tests, and it found one. That is the failure mode this project has
been bitten by before.

All probes reverted; `pipeline.rs` diffed byte-identical afterwards; gate re-run at 1012.

## Unsupported, and named rather than silently absent

Monkey patching, runtime attribute assignment, metaclasses, decorator-generated behaviour,
`__getattr__`, `globals()` mutation. All require execution, which SECURITY.md T1 forbids. Recorded
in the `pystruct.rs` module doc and the fixture README.

Two smaller asymmetries with TS/JS, stated rather than hidden: `.pyi` stubs are not indexed (a stub
would put a second `Module` on one import target), and `name = lambda ...` is not a `Function`,
where TS's declarator-bound arrows are.

## Known limitations

- **`nerve impact` on a Python symbol prints "every reference site … resolved"** in a Python-only
  repository. Literally true — there are zero Python reference sites until 9b — but a reader could
  take it as coverage. **9b must fix the wording.**
- **`incremental::classify` does not model the `sys.path` refusal**, so it can over-seed a
  re-extraction and never under-seed. Argued in a comment at the site.
- **`importlib.import_module("literal")` is unresolved** — open decision, above.
- **`sys.path` detection is a lower bound.**

## UI backend handoff changes

**None.** No vocabulary gained a member, so no TypeScript mirror changed.

## Commit

`4e53d82` — *feat: Slice 9a — Python structure, and an import site that makes two claims*.

## Next slice

**9b — `py-reference`**, with its own `py_precision.rs` gate reporting FP, FN and the unresolved
rate **independently of the TS/JS numbers**.
