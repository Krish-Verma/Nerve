# Final acceptance

```
scripts/final_acceptance.sh
```

Exits 0 only if every check passes. Takes a few minutes, because it runs the full test suite and a
release build rather than trusting a previous one.

**The roadmap is authoritative about completeness** (`docs/ROADMAP.md`). This script gates *what is
built*; a green run is not a claim that Nerve is finished. As of this writing rows 1–11 are done and
rows 12–14, real-world validation and the final audit are not.

---

## 1. The distinction the whole script is organised around

A command that does not exist has four possible reasons, and collapsing them is how a report starts
lying. The script prints a different word for each:

| outcome | meaning |
|---|---|
| `PASS` | ran and succeeded |
| `FAIL` | ran and failed |
| `REFUSED` | **absent by decision**, with the decision named. Not a gap |
| `NOT BUILT` | absent because its slice is not done. A gap, and named as one |
| `SKIPPED` | could not run here, with the reason. **Never counted as a pass** |

Two commands are `REFUSED`, and both would be easy to mistake for missing features:

- **`nerve affected`** — ADR-0008. An LCOV report carries no per-test attribution, so *"which tests
  would my change affect?"* cannot be answered from coverage evidence. The command is absent because
  the honest answer is unavailable, not because nobody wrote it. Building it would mean inventing
  per-test attribution the report does not contain.
- **`nerve trace-tests`** — Nerve must not run a repository's test runner (THREAT-MODEL T1,
  `crates/nerve-cli/tests/no_subprocess.rs`, whose module documentation names *"no test runners"*
  explicitly). Traces are **ingested** from an artifact a user's own tracer produced. The producer
  ships in `tracers/python/` and runs in the user's process, never in Nerve's.

A future version of this script must not "fix" either row. If one of them ever exists, the script
fails — deliberately — so the security boundary cannot be crossed quietly.

`nerve history` (Slice 12) and `nerve memory` (Slice 14) are `NOT BUILT`. If they appear, the script
passes and asks to be updated.

## 2. What it checks

**1. Verification gate** — `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -D
warnings`, `cargo build --release`, and `cargo test --workspace --no-fail-fast`.

`--no-fail-fast` is not a preference. Measured on this project in Slice 7b: without it the run
reported **3** failures where there were **16**, because the first failing target stopped everything
after it. A gate that under-reports failures is worse than no gate.

**2. Security invariants** — the `no_subprocess` and `no_network` suites by name, and a scan asserting
no Rust source references the trace producer. Product code that knows the tracer's name is one step
from knowing how to launch it.

**3. Command surface** — all 13 commands exist; the two refusals do not; the two unbuilt commands are
reported as gaps.

**4. End to end on a clean checkout** — `git archive HEAD` into a temporary directory, then `init`,
`index`, `status`, `doctor`, `search`, `gaps`, `impact`, `why`, `path`, `check`. It never touches your
working tree's `.nerve/`.

Plus: **read-only queries do not mutate the database**, byte-compared with SHA-256 around three
queries.

The selectors are **TypeScript**, and the reason is worth knowing. The first version of this script
queried `parse_trace`, a Rust function, and `impact` and `why` both refused — correctly. Nerve indexes
TypeScript, JavaScript and Python, and **does not index Rust**. Nerve's own Rust source cannot serve as
a self-test subject for a symbol query; `apps/nerve-web` is what makes this repository able to index
itself at all. (The refusals were exemplary, incidentally: each listed candidate alternatives rather
than reporting an empty result.)

**5. Supply chain** — the `Cargo.lock` package count, that `third_party/LICENSES.md` is populated, and
that no copyleft-only dependency is recorded.

That last check is not a bare grep for `GPL`. The first version was, and it failed on a fine
dependency: `r-efi` offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later` and the record says *"we take
MIT"*. A disjunction containing a copyleft option is not a copyleft dependency. The rule is now: any
line naming GPL/AGPL/SSPL must also name a permissive licence — that is, must be a choice we can take.

Plus a clean-room scan for named competitor products.

## 3. What it cannot check, printed every run

The script prints these as `MANUAL` rather than omitting them, because a checklist that silently drops
what it cannot automate reads as though nothing were missing.

- **Real-world accuracy.** `docs/plans/slice-15-real-world-validation.md` needs a pinned corpus and two
  oracles (the TypeScript compiler; `jedi` for Python), and acquiring them needs network.
- **The Python tracer end to end.** `scripts/trace_python_e2e.sh` needs pytest in a virtualenv, which
  needs network. Its produced artifact is committed as a fixture so that *ingestion* is gated on every
  `cargo test`, but the production step is not automated and no document may describe it as if it were.
- **Visual QA of `apps/nerve-web`.** The frontend is frozen and owned by the user; see
  `docs/UI-BACKEND-HANDOFF.md`.

## 4. Last recorded result

```
passed  34
failed  0
skipped 1     no Rust source references the trace producer — tracers/ did not exist yet
```

The skip is the honest kind: the check has nothing to examine until Slice 11b lands, and it says so
rather than passing on an absent directory.
