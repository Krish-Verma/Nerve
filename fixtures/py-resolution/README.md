# `fixtures/py-resolution`

The corpus the Slice 9b precision gate measures `py-reference` against.
`crates/nerve-index/tests/py_precision.rs` reads `expected.json` and reports FP, FN and the
unresolved rate for `CALLS`, `REFERENCES` and `EXTENDS` — **separately from the TypeScript
numbers in `fixtures/ts-resolution`**. A combined figure would let a strong TypeScript result
hide a weak Python one.

`expected.json` was written **before** the resolver, exactly as `fixtures/ts-resolution` and
`fixtures/py-basic` were. When the two disagreed, the fixture won unless the fixture was shown
to be wrong about Python.

## What each file is for

| file | what it pins |
|---|---|
| `pkg/util.py` | the simplest edge: a same-module call |
| `pkg/__init__.py` | a package that re-exports a name, so `from pkg import Engine` has a chain |
| `pkg/core.py` | `EXTENDS`, an explicit class receiver that resolves, and a `self.` receiver that does not |
| `app.py` | every cross-module import form: from-import, package re-export, dotted module, module alias |
| `negative.py` | a shadowed name, a same-named local, a same-named attribute on another class, and both directions of the class-body scope rule |
| `scoping.py` | comprehension scopes, `global`, `nonlocal` |
| `unmodelled.py` | callee and base-class shapes that are counted rather than guessed |
| `wildcard.py` | a wildcard import, whose bindings stay unknowable |
| `external.py` | names that leave the repository, and one that names an indexed module without naming anything in it |

## The three rules that decide the numbers

**A class body is not an enclosing scope for its methods.** `negative.py#Scoped` writes
`helper` twice: once in the class body, where the class attribute wins, and once inside a
method, where the module function wins because the class scope is skipped. Getting this wrong
produces a false positive in one direction and a false negative in the other, so both are
pinned.

**Python binds a name for the whole function body.** `negative.py#assigned_anywhere_is_local`
reads `scale` on the line *above* `scale = 2`. CPython raises `UnboundLocalError` there; it does
not reach the import. A shadowing guard that only looks backwards from the use would resolve it
and be wrong.

**A receiver Nerve cannot name is unresolved, never guessed.** `self.start()`,
`other.start(1)` and `Turbo.start(...)` are all recorded as unresolved with distinct reasons.
The first two are the reason Python's unresolved rate is higher than TypeScript's, and the
correct response to that number is to print it.

## `known_false_negatives`

Three edges a semantically complete resolver would produce and Nerve deliberately does not.
They are listed, must be **absent**, and are what the FN column counts — so the reported FN is
a measurement rather than a zero asserted into existence. If one of them ever starts being
produced, the gate fails and tells you to promote it to `resolved`.

## No `IMPLEMENTS`

There is no `IMPLEMENTS` section and there will not be one. Python has no `implements` keyword.
`class Abstract(ABC)` states inheritance; `EXTENDS` is what the syntax states, and the edge is
recorded as unresolved because `abc` is not in the index.
