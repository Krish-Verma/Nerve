# Slice 9 — Python

**Status:** planned 2026-08-02. **Split into 9a and 9b before implementation.**
Follows Slice 8b-ii (`b40458d`).

---

## Why the row is split

Row 9 is "Python language support" — for TypeScript that was **Slices 1 and 2a**: structure first,
then resolution with a measured precision gate. Doing both at once for a second language repeats
the mistake Slice 2 was split to avoid, and would land call resolution without the negative
fixtures that make its precision number mean anything.

- **9a — structure.** `py-structural`: modules, packages, imports, functions, classes, methods.
- **9b — references.** `py-reference`: `CALLS`, `REFERENCES`, `EXTENDS`, with a precision gate.

9b does not start until 9a is committed and verified.

---

## The dependency, checked before planning

`tree-sitter-python 0.25.0`, **MIT**, from the same `tree-sitter` organisation as the two grammars
already vendored, and its `0.25` line matches the workspace's `tree-sitter = "0.25"` — so no
version bump for the existing grammars. Verified via `cargo info` on 2026-08-02.

**`tree-sitter-stack-graphs-python` is rejected on clean-room grounds.** Stack-graphs is a
name-resolution / code-navigation engine. CLAUDE.md §1 forbids depending on a competing
code-intelligence engine; a bare grammar is a parser, which §1 explicitly permits. Nerve resolves
names itself, as it already does for TS/JS.

One new direct dependency, recorded in `third_party/LICENSES.md`. The implementer must report the
exact transitive count after adding it; the workspace is 100 crates today.

---

## What Python is not allowed to inherit

**A Python observation must never claim to come from `ts-js-structural`.** Every observation
records an extractor id and version, and Slice 5d-i was a corrective slice for exactly this —
directory containment was stamped `ts-js-structural` in a repository with no TypeScript. So:

- new extractor ids `py-structural` and `py-reference`, each versioned independently;
- new modules, not new branches inside `extract.rs` / `refs.rs`;
- `DECLARED_SOURCE_TYPES` per extractor, verified by the existing
  `verify_declared_source_types` check the pipeline already runs.

**Python accuracy is measured separately.** `crates/nerve-index/tests/precision.rs` measures
`ts-js-reference` over `fixtures/ts-resolution`. Python gets its own fixture corpus and its own
harness. A combined number would let a strong TS result hide a weak Python one, and the assignment
forbids it explicitly.

---

## 9a — structure

### Entities and relations

`Module` (1:1 with a `.py` file), `Function`, `Class`, `Method`, plus the `File`/`Directory`
skeleton `fs-structural` already produces for every file regardless of language.

`CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`.

### Python-specific decisions that must be *decided*, not defaulted

| question | the decision, and why |
|---|---|
| **Packages** | A directory with `__init__.py` is a package. Nerve has no `Package` entity kind and **must not gain one in this slice** — a new `EntityKind` is a vocabulary change touching the UI mirror, `path_role`, and every exhaustiveness test. The `__init__.py` module carries the package identity; record packageness in its `meta`. If the implementer finds this untenable, that is a finding to report, not a kind to add. |
| **Nested functions** | Extracted, with the enclosing function in `scope_path` — TS/JS already does this and `EntityKind::is_symbol` already covers `Function`. |
| **Decorators** | Recorded as **structural metadata on the decorated symbol**, in `meta`. A decorator is not a call edge in 9a. `@app.route("/x")` is a framework fact and belongs to Slice 10. |
| **`__init__`** | A `Method` like any other. Nothing special. |
| **Relative imports** | `from . import x`, `from ..pkg import y` — resolved, mirroring `resolve.rs`'s relative handling. Dot-count maps to parent traversal. |
| **Absolute in-repo imports** | `from pkg.mod import x` resolved against `pkg/mod.py` and `pkg/mod/__init__.py`, **only when the target is in the index**. |
| **Everything else** | `Unresolved`, with a closed-vocabulary reason. |

### What must be recorded as unresolved rather than guessed

This is the honest core of the slice. Each of these is a **value**, never an omission:

- wildcard `from x import *` — the names it binds are not knowable statically;
- conditional imports inside `if` / `try`;
- `importlib.import_module(...)` and `__import__`;
- imports resolving outside the repository (stdlib, site-packages);
- namespace packages (a directory with no `__init__.py`);
- `sys.path` manipulation.

`UnresolvedCategory` is a closed vocabulary. The implementer must check whether its existing
members cover these and **report** if a new member is needed rather than reusing an ill-fitting
one.

### Explicitly out of scope for 9a, and named in the report

Monkey patching, runtime attribute assignment, metaclasses, decorator-generated behaviour,
`__getattr__`, `globals()` mutation. Nerve is a static index; these are not resolvable without
executing code, and **executing repository code is forbidden** (SECURITY.md, T1). They are
unsupported, which is a stated state, not a silent gap.

### Fixtures

New `fixtures/py-basic`: positive, **negative** (a name that must *not* resolve), ambiguous,
malformed (a syntax error mid-file), and unsupported (wildcard, dynamic import) cases. Written
**before** the resolver, as Slice 5d-ii's ground truth was.

### 9a acceptance criteria

1. `.py` indexed; `Module`, `Function`, `Class`, `Method` entities with correct spans.
2. Relative and in-repo absolute imports resolve; a test asserts a specific expected edge set.
3. Every unsupported form appears as an `Unresolved` entity with a reason — asserted, not assumed.
4. **Zero `ts-js-*` observations in a Python-only repository**, asserted over the extractor id.
   This is the 5d-i invariant, restated for a new language.
5. A malformed file degrades to a partial parse, does not abort the index, and is counted in
   parse health.
6. Incremental: editing a `.py` file invalidates and rebuilds equivalently to a full index — the
   existing full-vs-incremental equivalence harness extended to Python, not a new one.
7. No repository code executed; the T1 no-subprocess loop covers a hostile Python repository
   (`setup.py` with side effects, a module with top-level `os.system`).
8. Determinism: two indexes of the same tree produce byte-identical graphs.
9. Mutation probes: stamp Python observations with `ts-js-structural` → criterion 4 fails;
   resolve a wildcard import to a guessed target → a negative-fixture test fails.

---

## 9b — references, with a measured gate

`py-reference`: `CALLS`, `REFERENCES`, `EXTENDS`. **No `IMPLEMENTS`** — Python has no `implements`
keyword; ABCs and protocols are inheritance or duck typing, and asserting `IMPLEMENTS` from
`class C(SomeABC)` would be a guess. `EXTENDS` is what the syntax states.

Lexical binding with the same shadowing discipline as `bind.rs`, including Python's scoping rules
(`global`, `nonlocal`, comprehension scopes, the fact that a class body is **not** an enclosing
scope for methods).

**A method call on a receiver (`obj.method()`) is unresolved** unless the receiver is
unambiguously a module or class Nerve indexed. Nerve has no type inference. TS/JS measured **38.1 %
of call sites unresolved** and reported it; Python's rate will be higher and must be reported, not
reduced by guessing.

### 9b acceptance criteria

1. `crates/nerve-index/tests/py_precision.rs` — a separate gate over a Python corpus, reporting
   FP, FN, and unresolved rate **independently of the TS/JS numbers**.
2. FP = 0 on the fixture corpus. FN reported honestly; a non-zero FN is acceptable and stated,
   a non-zero FP is not.
3. Negative fixtures: a shadowed name, a same-named local, a same-named attribute on a different
   class — none produce an edge.
4. The unresolved rate is **printed** by the harness, as the TS/JS one is.
5. Mutation probes: relax the shadowing guard → a negative-fixture test fails; resolve a
   method call by name matching → an FP appears and the gate fails.

---

## Non-goals for both

- No framework rules (Slice 10).
- No type inference. Ever, without evidence.
- No `EntityKind` addition unless proven necessary and reported first.
- No schema change or migration expected; if one is needed, that is a finding.
- No `apps/nerve-web/` change beyond a vocabulary mirror if a vocabulary actually gains a member.
- No change to TS/JS behaviour. The existing precision gate must still read FP=0, FN=0.

## Verification gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
```

Baseline **970 passed / 0 failed / 2 ignored**.
