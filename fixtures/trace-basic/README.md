# trace-basic fixture

A small Python tree plus four **hand-written** `nerve-trace/v1` artifacts, used by
`crates/nerve-index/tests/trace.rs` to measure what the `test-trace` extractor emits against what a
human read out of the artifacts.

Nerve does not run tests. `tests/test_parse.py` is here as *source to index*, not as a suite to
execute — `crates/nerve-cli/tests/no_subprocess.rs` forbids process creation in product code, and
`docs/plans/slice-11a-trace-ingestion.md` §1 records why that invariant is not being weakened for a
tracer. The artifacts are written by hand for the same reason `fixtures/ts-coverage/coverage/lcov.info`
is: a real tracer cannot emit a traversal path, an oversized record, malformed UTF-8 or a
prompt-injection payload, so generating them would make the security half of the slice untestable.
The hostile half lives in `fixtures/trace-hostile/`.

## The tree

```
src/lex.py           tokenize 4-6 · classify 9-13 · unobserved 16-18
src/parse.py         parse 8-11 · reload_lex 14-18 · Parser 21-26 · Parser.parse_all 24-26
tests/test_parse.py  test_basic 6-8 · test_method 11-13 · test_lazy_import 16-18 · test_partial 21-23
```

Three shapes are deliberate:

- **`parse` calls `tokenize` and `classify`.** So `test_basic → parse → tokenize` is a depth-2 edge,
  and the assertion for it must have **`parse`** as its source. Attributing it to `test_basic` would
  assert a call the test never made; that is the defect Slice 11a's plan §2.1 corrects, and
  `expected.json`'s `no_edge` list pins the absence rather than leaving it unstated.
- **`Parser.parse_all` is nested inside `Parser`.** Line 26 is inside both extents, so the
  line→symbol mapping's *innermost* rule is exercised on a real tie-break: `parse_all`'s byte span
  is the shorter one.
- **`src/parse.py:5` is a module-level call.** No symbol contains it, so it produces an unresolved
  frame rather than being attached to the function below it.

## The artifacts

| file | what it is for |
|---|---|
| `trace/bound.jsonl` | the whole shape: depth-1, depth-2 and depth-3 edges, two tests sharing one call site, an unresolved frame on each side, a producer-unresolved frame, an unknown record key, and one record per member of the closed limitation vocabulary |
| `trace/partial.jsonl` | `completion_state: "partial"` with a null `completed_at` and a stated reason |
| `trace/unverified.jsonl` | both state fields `null` — the third binding value, never reported as `bound` |
| `trace/stale.jsonl` | a well-formed content merkle of 64 zeros — reported as `stale`, not refused |

### `__CONTENT_MERKLE__` is a placeholder, and has to be

`bound.jsonl` and `partial.jsonl` carry `"content_merkle": "__CONTENT_MERKLE__"`. A hand-written file
cannot contain a hash computed at index time, and hard-coding the merkle of the tree as committed
would break the fixture the next time anything under it is edited — including this README, which the
merkle covers. The test substitutes the merkle the index recorded, exactly as a real tracer computes
it at run time.

The placeholder is not valid 64-character hex, so the committed file is **refused** as
`header-invalid` if it is imported unmodified. That is deliberate: a placeholder that happened to
parse would let a forgotten substitution read as a genuine `stale` binding, which is a silent
misreading rather than a loud one.

## A disagreement with plan §2.1, recorded rather than smoothed over

Plan §2.1 says *"Two tests observing one edge produce two observations on one assertion, which is
exactly what `observation_count` is for."* **On the shipped schema that is false.**
`crates/nerve-store/src/schema.rs:257-260` defines

```sql
CREATE UNIQUE INDEX idx_observation_identity ON observation(
    assertion_id, extractor_id, extractor_version,
    evidence_source_type, file_path, start_line, end_line);
```

There is no column for `environment`. Two tests reaching the same callee from the same caller line
agree on every one of those seven columns, so they are **one row**, and `INSERT OR IGNORE` — which is
what makes re-indexing free — would drop the second test's identity without a word.

Slice 11a forbids a schema change, so the ingestion reads what is stored and restates the **union**:
one observation per `(caller, callee, caller_file, caller_line)`, whose `environment.runs[]` names
every run and every test that reached that site. Nothing is lost and nothing is silent. Two
*different* call sites are still two observations, because they are two pieces of evidence.

`expected.json`'s `bound.edges[1]` is the case: `observed_count` 3 from two records and two tests,
on one observation.
