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
pinned-commit checkout. Python repositories are added to this table when Slice 9 lands; the same
constraints apply.

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

## 4. What gets reported

Separately, per category, never aggregated into one score:

`local calls · imported calls · method calls · references · extends · implements · module
resolution · re-exports · barrel re-exports · framework relationships · document links · test
coverage mappings · Python categories`

For each: sample size · TP · FP · FN · precision · recall · unresolved rate · unsupported rate ·
oracle-undecided rate · Wilson score interval at 95% · and an explicit **fixture-versus-real-world**
label on every number.

Sampling is stratified by category with a recorded seed, because a uniform sample of a large
repository is dominated by trivial local calls and would hide exactly the categories that are weak.

## 5. Acceptance

1. `validation/corpus.toml` exists, every commit is a full SHA, every licence is verified against
   the file at that commit, and a re-run reproduces the same `tree_hash`.
2. The oracle runs offline after `npm ci`, and its version is pinned and recorded.
3. Every category above has a number or an explicit "not applicable — no such construct in this
   corpus".
4. **No category is omitted because it scored badly.** A category Nerve is bad at is the most
   valuable line in the report.
5. `docs/reports/validation-report.md` states the fixture number and the real-world number side by
   side, and `docs/CONTINUATION.md`'s "recall is unmeasured" limitation is either removed or
   rewritten with the measured value.
6. No product-code change is required to run it. If one turns out to be, that is a finding.
</content>
</invoke>
