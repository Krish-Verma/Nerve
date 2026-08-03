# `py-basic` — the Slice 9a Python structure corpus

Ground truth lives in `expected.json` and is read by `crates/nerve-index/tests/python.rs`.
It was written **before** the resolver, so the resolver was made to satisfy a specification
rather than the specification made to describe a resolver.

## What each file is for

| File | Case | What it proves |
|---|---|---|
| `app.py` | positive | absolute in-repo specifiers resolve to `pkg/util.py`, `pkg/__init__.py` and `pkg/sub/deep.py` |
| `pkg/__init__.py` | positive | a directory holding an `__init__.py` is a package, recorded on `meta`; `__all__` is the only statement of a public surface Python has |
| `pkg/core.py` | positive | functions, classes, methods, a nested function, `async`, and decorators as structural metadata |
| `pkg/util.py` | positive | a leaf resolution target, named so that a basename search would wrongly find it |
| `pkg/sub/deep.py` | positive + refusal | one- and two-dot relative imports resolve; four dots climb past the root and do not |
| `nspkg/orphan.py` | refusal | a directory with no `__init__.py` is a namespace package |
| `unsupported.py` | unsupported | wildcard, conditional and dynamic imports — the module still resolves, the bound names do not |
| `syspath.py` | refusal | a module that mutates `sys.path` cannot have its absolute specifiers resolved |
| `negative.py` | negative | four specifiers that must **not** resolve, each the plausible wrong answer |
| `broken.py` | malformed | a syntax error mid-file degrades to a partial parse and does not abort the index |

## Decisions this corpus pins

- **`from pkg import name` names `pkg`, never `pkg.name`.** Whether `name` is an attribute the
  package's `__init__.py` binds or a submodule is not decidable without cross-module reasoning,
  and that is Slice 9b's ground. `from pkg import core` in `negative.py` is the case, and the
  edge to `pkg/core.py` it would produce is listed as forbidden.
- **Package before module.** `pkg/util/__init__.py` would outrank `pkg/util.py`, because
  CPython's `FileFinder` consults its path hooks (directories) before its file loaders.
- **A resolver that reaches outside the repository is a resolver that guesses.** `os`,
  `functools`, `sys` and `importlib` are all real modules and none of them is here, so each is
  an `Unresolved` value with a reason.

## Deliberately absent

No framework decorators are interpreted (`@app.route` would be Slice 10). No `EXTENDS`,
`CALLS` or `REFERENCES` — `Turbo(Engine)` is in `pkg/core.py` precisely so that 9b has
something to add without editing the corpus. And nothing here is ever executed: `broken.py`
is parsed, not run, which is the whole of THREAT-MODEL T1.
