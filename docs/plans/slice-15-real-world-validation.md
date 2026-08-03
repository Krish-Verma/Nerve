# Real-world accuracy validation

**Date:** 2026-07-31 · **Status:** Approved, scheduled after the language slices
**Runs after:** Slice 9 (Python), so every resolver that exists is measured in one pass.

---

## 1. Why this exists

Nerve's only accuracy number today is **FP = 0, FN = 0 over 24 resolved edges** on hand-authored
fixtures, with 38.1% of call sites honestly unresolved. That number is real and it is gated, but
fixtures were written by the same mind that wrote the resolver, so it measures *self-consistency*,
not accuracy. It must never be presented as production accuracy, and this document exists so it
does not have to be.

The purpose is not a flattering headline number. It is to find the categories where Nerve is
**wrong**, and to publish them next to the categories where it is right.

## 2. Corpus — selected autonomously, per the brief

The brief forbids asking the user to choose repositories when safe permissive options exist. These
were chosen for **structural diversity**, not popularity, and none is a code-intelligence or
knowledge-graph product (clean-room: we index them as ordinary TypeScript, and read no competitor's
schema, database or output).

| Repository | Licence | Chosen because it exercises |
|---|---|---|
| `chalk/chalk` | MIT | Small pure-ESM JavaScript with `.d.ts` — the minimal end, and a JS-not-TS control |
| `reduxjs/redux` | MIT | Small clean TypeScript: higher-order functions, callbacks, function composition |
| `colinhacks/zod` | MIT | Medium TypeScript: classes, interfaces, deep generics, method chaining — the method-call case the resolver openly cannot type-resolve |
| `date-fns/date-fns` | MIT | Large, thousands of modules, **barrel files and a deep re-export closure** — the hardest case for module resolution, and the one Slice 2a's closure was written for |
| `vitest-dev/vitest` | MIT | **Monorepo workspaces and `tsconfig` path aliases** — multi-package resolution |

Every repository is public, permissively licensed, needs no authentication, and is acquired by a
pinned-commit checkout.

### Python, added after Slices 9a, 9b and 10a landed

The original table was TypeScript-only and said Python repositories "are added when Slice 9 lands".
It has. Chosen against what the Python extractors actually claim to do, and against what 9b measured
itself as unable to do — **42.3% of Python references unresolved, and `self.method()` deliberately not
resolved.** A corpus that avoided those constructs would flatter the number.

| Repository | Licence | Chosen because it exercises |
|---|---|---|
| `psf/requests` | Apache-2.0 | Small, flat, heavily-used package: plain functions, module-level imports, a few classes — the readable baseline, and the one where a bad number has no excuse |
| `pallets/click` | BSD-3-Clause | **Decorator-heavy by design.** Nested decorators, decorator factories, and `@group.command()` forms that look exactly like the framework registrations 10a resolves but are **not** HTTP routes. The false-positive control for `SERVED_BY` |
| `pallets/flask` | BSD-3-Clause | The **framework's own source**, which registers no routes itself. Any `Endpoint` found here is a false positive, and the framework code is the likeliest place for one |
| `tiangolo/fastapi` | MIT | Same control for the other supported framework, plus `typing`-heavy generics and `Annotated` forms |
| `encode/httpx` | BSD-3-Clause | Classes with deep inheritance and pervasive `self.method()` calls — **the case 9b openly refuses.** Its purpose is to size the refusal, not to pass |
| a small FastAPI/Flask **application** (TBD at run time, MIT/BSD, ≤5k lines) | — | The only repository in the corpus that *should* yield endpoints. A framework library and an application that uses it are different measurements and must not be averaged |

The last row is deliberately unresolved here: choosing an application repository requires checking
that it actually declares routes in a form 10a supports, which is a measurement, not a guess. It is
picked during the run and pinned in the manifest like every other.

### Manifest

`validation/corpus.toml` records, per repository and verified by hash at use time:

```
repository        canonical URL
commit            exact 40-character SHA, never a tag or branch
license           SPDX identifier, plus the path of the licence file as it exists at that commit
purpose           the structural property it is in the corpus for
included_paths    what is indexed
excluded_paths    what is not, and why (vendored code, generated output, fixtures)
tree_hash         so a re-run proves it measured the same bytes
acquisition       how it was fetched
```

Acquisition is a **development-time** activity, exactly as fetching crates is. It is not part of the
shipped product's runtime behaviour, and `no_network.rs` continues to hold: nothing in
`crates/*/src` gains a network client. The corpus is **not committed** — the manifest is, and the
checkout is reproducible from it.

## 3. Oracle

**The TypeScript compiler API** (`typescript`, Apache-2.0), driven from a Node script under
`validation/oracle/`. `ts.createProgram` plus `TypeChecker.getSymbolAtLocation` and
`getResolvedSignature` yield, for a call site, the declaration the compiler believes it reaches.
That is genuine independent ground truth: it is produced by a different implementation, from a
different parse, with full type information Nerve deliberately does not have.

**It is an oracle, never an engine.** It lives outside the Rust workspace, is never invoked by
product code, is not a dependency of any crate, and no compiler output is ingested into an index.
The no-subprocess invariant is unaffected because nothing in `crates/*/src` calls it.

**Recorded limitations, because an oracle with unstated limits is just a second opinion:**

- The compiler resolves through types Nerve has no access to; disagreement on a method call on a
  typed receiver is an expected Nerve *limitation*, not a bug, and must be reported as its own
  category rather than folded into "false negatives".
- `any`, declaration merging, ambient declarations and conditional types produce compiler answers
  that are themselves approximations.
- Dynamic dispatch, `eval`-shaped indirection and framework magic are invisible to both.

Where the compiler cannot decide either, the sample is reported as **oracle-undecided** and excluded
from precision and recall — never silently counted as agreement.

### 3.1 The Python oracle

**`jedi`** (MIT), driven from a script under `validation/oracle/python/`, in its own venv. Jedi is a
static analyser built for editors: a different implementation, a different parse, and — importantly —
one that also does not execute the code, so it fails on the same *class* of construct Nerve does while
disagreeing about the specifics. `pyright` was considered and rejected as the primary: it is a
type checker whose answers depend on annotations that most of this corpus does not carry, so it would
report "undecided" across exactly the cases that matter.

Same status as the TypeScript oracle: outside the workspace, never invoked by product code, not a
dependency of any crate, no output ingested.

**Limitations, stated because Python's are worse than TypeScript's:**

- Python name resolution is **not statically decidable in general** — a name can be rebound at runtime,
  a module can rewrite its own `__dict__`, and `getattr` is ordinary. Jedi approximates; so does Nerve.
  Where they disagree and neither can justify itself from the source, the sample is **oracle-undecided**.
- `self.method()` is the largest single category and Nerve **refuses** it. It must be reported as a
  measured refusal with its own line, not folded into false negatives. A refusal reported as a failure
  would make the honest choice look like the broken one.
- Star imports, conditional imports under `TYPE_CHECKING`, and `__init__.py` re-export chains are
  approximations on both sides.

### 3.2 The framework-endpoint oracle, and its awkward property

**The only complete oracle for "what routes does this application serve" is to import the application
and ask it** — `app.routes` for FastAPI, `app.url_map` for Flask. Which means **the oracle must execute
the repository's code, and that is precisely what Nerve refuses to do** (T1, and the whole of
`no_subprocess.rs`).

That is not a contradiction, and it is worth stating plainly rather than working around:

- The **harness is not the product.** It runs at development time, in a throwaway virtualenv, on
  repositories chosen and pinned by us. `crates/*/src/**` gains nothing.
- **The gap between the two is the measurement.** Nerve's framework rules are a static approximation of
  something only execution settles. Running the app tells us the true route set; Nerve's static answer
  tells us how much of it a deterministic rule recovers. Any other oracle — hand-labelling from the
  source, say — is not independent of Nerve, because a human reading the source is doing Nerve's job
  with Nerve's information and would reproduce Nerve's blind spots.
- **Recorded risk:** importing a repository's application module executes arbitrary code. Accepted only
  for repositories already pinned in the manifest, in a container or throwaway VM, and the report must
  name every repository whose code was executed. If a candidate application cannot be imported safely,
  it is dropped from the endpoint measurement and that is recorded — not replaced with a weaker oracle
  reported under the same heading.

Expected finding, recorded now so a bad number is not reinterpreted later: **recall below 1.0 is the
correct outcome.** 10a supports two frameworks and a closed set of registration forms, and
`framework_unsupported_by_form` already counts what it declines. A dynamically-built route — a loop over
a config table, `add_api_route` called from a factory — is a true endpoint that no static rule should
claim. It belongs in **unsupported**, not in false negatives.

### 3.3 Trace evidence (Slices 11a, 11b)

The oracle for a trace is **the trace**: the artifact states which frames called which, and a real
tracer run over a real test suite is ground truth about that run by construction. What validation
measures is therefore not the edges but **Nerve's resolution of them** — how often a recorded
`(file, line)` frame maps to the symbol a reader would name.

Measured per repository: records accepted, `caller-outside-any-symbol` and
`callee-outside-any-symbol` rates (module-level and comprehension frames make these irreducibly
non-zero), `producer-unresolved-frame`, and the declared `producer_limitations`. Requires 11b, and the
corpus repository must have a runnable test suite — which not all of the above do, so the trace
measurement covers a **subset**, named explicitly.

## 4. What gets reported

Separately, per category, never aggregated into one score:

**TypeScript/JavaScript:** `local calls · imported calls · method calls · references · extends ·
implements · module resolution · re-exports · barrel re-exports`

**Python** (Slices 9a, 9b): `local calls · imported calls · module resolution · `__init__.py`
re-exports · class inheritance · decorated definitions` — and, on its own line and never folded into
false negatives, `self.method() — refused by design`

**Framework endpoints** (Slice 10a, 10b): `FastAPI routes · Flask routes · Express routes ·
SERVED_BY targets` — plus `framework_unsupported_by_form`, broken out by form, and **false positives
on the two framework libraries and on `click`**, where the correct count is zero

**Trace resolution** (Slices 11a, 11b): `records accepted · caller-outside-any-symbol ·
callee-outside-any-symbol · producer-unresolved-frame · declared producer_limitations`

**Cross-language:** `document links · test coverage mappings`

For each: sample size · TP · FP · FN · precision · recall · unresolved rate · unsupported rate ·
oracle-undecided rate · Wilson score interval at 95% · and an explicit **fixture-versus-real-world**
label on every number.

**A refusal is reported as a refusal.** `self.method()`, `framework_unsupported_by_form`, an
unsupported registration form and an unindexed trace frame are Nerve declining to guess. Counting them
as false negatives would make every honest refusal indistinguishable from a defect, and would create a
standing incentive to guess — which is the one thing the evidence model exists to prevent.

Sampling is stratified by category with a recorded seed, because a uniform sample of a large
repository is dominated by trivial local calls and would hide exactly the categories that are weak.

## 5. Acceptance

1. `validation/corpus.toml` exists, every commit is a full SHA, every licence is verified against
   the file at that commit, and a re-run reproduces the same `tree_hash`.
2. Each oracle runs offline after its own install step (`npm ci` for TypeScript, `pip install` into a
   venv for Python), and every version is pinned and recorded.
3. Every category above has a number or an explicit "not applicable — no such construct in this
   corpus".
4. **No category is omitted because it scored badly.** A category Nerve is bad at is the most
   valuable line in the report.
5. `docs/reports/validation-report.md` states the fixture number and the real-world number side by
   side, and `docs/CONTINUATION.md`'s "recall is unmeasured" limitation is either removed or
   rewritten with the measured value.
6. No product-code change is required to run it. If one turns out to be, that is a finding.
7. **Zero endpoint false positives on `pallets/flask`, `tiangolo/fastapi` and `pallets/click`.** These
   three declare no routes; `click`'s decorator factories are the shape most likely to fool a rule that
   pattern-matches decorators. A false positive here is worse than a missed endpoint, because
   `SERVED_BY` is in `impact::DEFAULT_RELATIONS` and would widen a blast radius on evidence that does
   not exist.
8. **Every repository whose code the harness executed is named in the report**, with why, per §3.2. A
   validation harness that executes repository code while the product refuses to must say so where a
   reader will see it, not in a footnote.
9. The trace measurement names the **subset** of the corpus it covers and why the rest is excluded.
