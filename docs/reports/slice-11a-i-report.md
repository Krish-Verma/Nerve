# Slice 11a-i — the hostile fixtures were not attacking anything

`871aef3` · 2026-08-03 · corrective slice on 11a · **1169 tests** (1168 → 1169)

A corrective slice, opened because `docs/CONTINUATION.md` recorded three hostile trace artifacts that
produced no refusal despite `fixtures/trace-hostile/README.md` declaring one for each. Diagnosis found
**five**, four of them sharing one root cause.

---

## 1. The finding

`fixtures/trace-hostile/README.md` documents four payload placeholders and closes:

> Each substitution is derived from the bound constant it attacks, so tightening a bound cannot leave
> its attack testing nothing.

**No code performed any substitution.** `grep -rn --include='*.rs' 'PAD_ARTIFACT\|PAD_RECORD\|PAD_STRING\|INVALID_UTF8__' crates/`
returned nothing. The hostile artifacts were `std::fs::copy`d verbatim into the fixture tree, so
`__PAD_STRING__` reached the parser as fourteen ASCII bytes and `__INVALID_UTF8__` as perfectly valid
UTF-8.

Measured, with the release build, before any change:

| artifact | the fixture table required | what actually happened |
|---|---|---|
| `oversized-file.jsonl` | `artifact-too-large`, refused whole, zero edges | `malformed-json` ×1, **1 observation written** |
| `oversized-record.jsonl` | `record-too-large` on one line | `record-unknown-key` ×1 — from its own `"padding"` key |
| `oversized-string.jsonl` | `string-too-long` | **nothing refused, 2 observations** |
| `malformed-utf8.jsonl` | `invalid-utf8-line` | **nothing refused, 2 observations** |
| `duplicate-run-id.jsonl` | `run-id-conflict` | **nothing refused** |

**The parser was never wrong about any of the four bounds.** All fourteen forms in `trace::form::ALL`
have unit tests in `trace_tests.rs` and every one passed, before and after. What was untested was the
**end-to-end path** — artifact on disk, through `ingest_trace`, to a counted refusal — while the
fixture table read as though each row exercised the bound in its name.

This is the **fourth** instance of this defect class on this project: two vacuous T7 property tests in
Slice 8b, the `a_lambda_handler_has_no_symbol_to_serve` test in 10a whose walker never visited the node
the fixture existed to exercise, and now this. The pattern is consistent enough to name: **a test that
cannot fail is worse than a missing one, because it reports a guarantee nobody is keeping.**

## 2. Why a green suite hid it

`every_refusal_form_is_produced_by_some_fixture` asserted an **aggregate**:

```rust
assert!(produced.len() >= 6, "only {} refusal form(s) were produced across every fixture");
```

Nine working attacks satisfied that on their own. The four disarmed artifacts contributed nothing and
cost nothing — the threshold could not distinguish *"every case works"* from *"enough cases work"*, and
that distinction is the entire value of a hostile corpus.

Replaced by `each_hostile_artifact_produces_its_declared_refusal`, which asserts the fixture table row
by row and **in both directions**.

## 3. The one real implementation defect: `run-id-conflict`, and the scope was the error

11a detected a replayed `run_id` inside `merge_runs`, which compares the run about to be written against
the runs already stored **on that one call site**. `duplicate-run-id.jsonl` walks straight past it by
replaying the id on a *different* edge: no stored observation ever sees it.

11a's own documentation defended the scope —

> Detected where it could do harm: on a call site both artifacts describe. A replayed `run_id`
> overlapping no previously observed site is not detected, and cannot overwrite anything either.

— and that is true and beside the point. **The harm is not overwriting. It is that `run_id` stops
naming one run.** A reader asking what `run-bound-1` observed then receives the union of two different
runs, at every site either artifact touched, and is told nothing about it.

Run identity is a property of the repository, so the check is one too:
`nerve_store::environments_for_extractor` reads every environment this extractor wrote, and the import
compares `(run_id, artifact_content_hash)` — same id *and* same bytes is a re-import, which must stay a
silent no-op; same id, different bytes is a conflict. Counted **once per artifact**, because the
collision is one fact about one header, and reported even when no record survives resolution.
`merge_runs` now counts nothing and its unit test says why.

## 4. Four artifacts must produce *no* refusal, and that is now asserted

`fts5-syntax`, `prompt-injection`, `sql-injection` and `state-substitution` count no refusal, and that
is **correct**: they are **inert, not invalid**. FTS5 operators, SQL fragments and instruction text are
legal strings in a `run_id`. Refusing one would reject a legal artifact for looking dangerous, and
`docs/THREAT-MODEL.md` T7's claim about untrusted content has always been inertness rather than
rejection. `state-substitution`'s correct answer is a *binding* of `stale`.

Previously this was a note in the continuation state. It is now an assertion, so a future over-eager
guard fails a test instead of looking like a security improvement.

## 5. My own test was wrong again, and the reason is the same as three times in 11a

`a_replayed_run_id_is_reported_and_overwrites_nothing` is new — the fixture table made two claims about
the replay and only the first had any test. Its third assertion was:

```rust
assert!(!after.iter().any(|row| row.4.contains("9999")));
```

It failed, correctly. The replay names a *different* edge and plan §7 says it **imports**, so a row
carrying `count: 9999` is the honest record of a claim the artifact actually made. Banning the number
outright would have been the fifth over-assertion of this slice: forbidding the evidence instead of
bounding where it may land.

The assertion is now the boundary — that count may reach only the replay's own edge — and the reasoning
is in the test.

## 6. Files changed

| file | what |
|---|---|
| `crates/nerve-store/src/query.rs` | `environments_for_extractor`, with its table-scan cost stated rather than hidden |
| `crates/nerve-store/src/lib.rs` | re-export |
| `crates/nerve-index/src/trace_ingest.rs` | repository-wide `run_id_already_recorded`; `merge_runs` no longer counts; three unit tests updated |
| `crates/nerve-index/tests/trace.rs` | `stage_hostile`, `hostile_artifacts`, the per-artifact table, the new overwrite test; four copy sites routed through staging |
| `fixtures/trace-hostile/README.md` | the false paragraph corrected and kept as a record; two miscounts fixed (13→15 artifacts, 3→4 tokens) |
| `docs/plans/slice-11a-trace-ingestion.md` | §7 gains the scope correction |
| `docs/CONTINUATION.md` | the gap table closed |

## 7. Verification

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace --no-fail-fast                   → 1169 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
Cargo.lock                                              → 101 packages, unchanged
git diff no_subprocess.rs no_network.rs                 → empty
```

**Mutation probes**, both applied, confirmed, reverted, and the file checksummed back to its
pre-probe value:

| probe | result |
|---|---|
| drop `__PAD_STRING__` from the token table | `stage_hostile` **refuses to stage** `oversized-string.jsonl` by name, before the assertion runs — the guard fires at staging time, which is the right place |
| `if false && run_id_already_recorded(…)` | both run-id tests fail; *"the replay must be reported exactly once; got {}"* |

**CLI smoke test**, release binary, `fixtures/trace-basic` copied to a temp root:

| step | result |
|---|---|
| import `bound.jsonl` | `binding bound`, 6 edges, **18 rows written**, exit 0 |
| import it again | 6 restated, **0 rows written**, no conflict, exit 0 |
| import `duplicate-run-id.jsonl` | `run-id-conflict 1`, exit **3**, 3 rows written for its own edge |
| the graph afterwards | six legitimate edges unchanged; `run-bound-1` present against **two** artifact hashes with both paths named — the collision visible in the evidence, not resolved behind the reader's back |

Clean-room: no competitor named. Safety: nothing sensitive staged, no absolute path in any committed
file, `.nerve/` untouched.

## 8. What was not done

- **The `record-too-large` and `artifact-too-large` bounds are now exercised end to end, but their
  fixtures write 32 MiB and 8 KiB of padding at staging time.** The 32 MiB write happens twice per full
  suite run. Measured cost is acceptable; if it ever is not, the fix is a smaller `MAX_ARTIFACT_BYTES`
  in a test build, not a smaller attack.
- **`records-exceeded` still has no fixture** — it needs 500,000 records. Its unit test in
  `trace_tests.rs` covers it, and the per-artifact test's documentation says so explicitly rather than
  implying the fixture set is complete.
- Row 11 is **not** complete: 11b, the reference Python tracer, is not built.
