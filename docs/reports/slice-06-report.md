# Slice 6 — test evidence (coverage only)

**Commits:** `70dc416` (6a — vocabulary + parser) · `<6b>` (6b — ingestion)
**Gate:** THREAT-MODEL **T9** · ADR-0005 · ADR-0008

---

## 1. The gating question, answered before anything was written

The plan made one question blocking: **does any common JavaScript coverage format carry per-test
attribution?** The roadmap's relation name presumed it did. Measured on Node v24.15.0's built-in
coverage and LCOV reporter — no dependency, no network — with two source files and two test files,
each test exercising exactly one source file.

| Probe | Result |
|---|---|
| One run over both test files | `TN:` **empty**; one record set per source file; both tests merged, every `DA:` a hit count with no test dimension |
| One run over `alpha.test.js` alone | `src/beta.js` **absent entirely** — attribution exists only *per run* |
| Concatenating the two single-test reports | Both records' `TN:` still **blank** — the merge workaround cannot recover it either |

**LCOV is aggregate.** The design follows the finding rather than the roadmap.

## 2. What that changed

**The source endpoint is a `CoverageRun`, not a test** — identified by the report's
repository-relative path and content hash, with a real occurrence at that real path. This is
structural, not a convention: it is impossible to state *"test X covers symbol Y"* because no such
endpoint exists to state it with. **Affected-test analysis is recorded as unsupported**, naming the
exact input that would support it (one report per test, from N separate runs, with the test's
identity carried outside the report — the format cannot carry it).

**The relation ships as `COVERS`, not `TEST_COVERS_SYMBOL`.** Two reasons, the second load-bearing:
this vocabulary never puts an endpoint kind in a relation name (Slice 5a rejected
`DOCUMENT_CONTAINS_SECTION` for the same reason), and after the finding a `TEST_` prefix would
assert an attribution the format does not carry — the same defect class Slice 5d-i corrected when
filesystem containment was labelled `AST_DIRECT`.

**This reversed an explicit prohibition, and says so.** ADR-0005's Enforcement read *"the relation
name is `TEST_COVERS_SYMBOL`, **not `COVERS`** and not `CALLS`."* `COVERS` was named and forbidden.
ADR-0008 names that sentence and reverses only the half the measurement falsifies: ADR-0005 was
written before any format had been measured, and it bundled *don't call it `CALLS`* with *do call it
`TEST_COVERS_SYMBOL`*. `COVERS` was rejected then for being vaguer; after the finding
`TEST_COVERS_SYMBOL` is not more precise, it is **precisely wrong**. The half carrying the argument
— *not `CALLS`* — stands, and is now enforced by a test rather than a spelling.

`CLAUDE.md` §3, ADR-0005, THREAT-MODEL T9, the roadmap and the master build plan were all corrected
in `70dc416` so no document still asserts the old name authoritatively.

## 3. Delivered

**6a — vocabulary and parser.** `EntityKind::CoverageRun`, `Relation::Covers`, extractor
`coverage 1.0.0` declaring `[TestCoverage]` and nothing else, `Directness::Inferred` (a line hit is
not a statement that a symbol is covered — a mapping step concludes it). The parser is **total**:
`&[u8] → CoverageReport`, no `Result`, no panic path, so there is nowhere for a syscall to hide.
`DA:n,0` is preserved as evidence of an uncovered line. `SF:` paths are carried byte-for-byte,
control bytes included, so 6b's guard sees exactly what the report said. Duplicate records merge by
**max, never sum** — with `TN:` empty there is no way to tell two runs from one recorded twice. A
non-empty `TN:` is counted, so a producer that ever populates it contradicts the finding with a
number rather than with memory. Three resource bounds, each refusing **whole** and counting rather
than truncating: a silently truncated report reads as "those lines never executed", a false negative
in a positive-evidence store.

**6b — ingestion.** `nerve coverage <report> [repo]`, a standalone command rather than a flag on
`nerve index`, because the ordinary post-edit `nerve index` would otherwise silently destroy every
coverage edge. Paths resolve through `discover::canonical_child`, the same choke point everything
else uses. Lines map to symbols through `docref::innermost_covering` — the same function Slice 5c
uses for `#L<n>` anchors, so the two cannot disagree. `covered`/`partial` is a **recorded value,
never rounded**; a symbol with no covered lines gets **no edge**, because absence is the answer and
a `NOT_COVERED` edge would put a negative claim in a positive-evidence store.

## 4. Two findings that changed the design mid-slice

**Coverage surviving a re-index required splitting the pipeline's withdrawal in two.** The plan
assumed the existing orphan pruner would suffice. It does not: an index run withdrew *everything*
recorded against a re-extracted file, so the first `nerve index` after any edit destroyed every
coverage edge. A **re-extracted** file now loses only the evidence `INDEX_EXTRACTOR_IDS` names; a
**removed** file still loses everything, because evidence about a file that no longer exists is
evidence about nothing.

That list is hand-maintained, and nothing failed if a future slice forgot it — a future extractor's
observations would go un-withdrawn on every re-extraction, leaving stale evidence surviving an edit
forever, silently. The orchestrator added
`every_extractor_an_index_run_writes_is_one_it_also_withdraws`, which checks the list against what an
index run **actually wrote** (`SELECT DISTINCT extractor_id FROM observation` over a fixture
exercising all four) rather than against a second list someone must remember to update.

**A file changed since indexing is refused, not mapped.** Not in the plan. Extents come from the
last index, so mapping a report onto a file whose bytes have moved produces a row derived from stale
spans and then stamps it with the *current* hash — a row that reads `fresh` and is wrong. Refusing
is the only honest option; the message tells the user to re-index.

**A deviation from §3.4, recorded in Addendum §A.4:** a source edit does **not** delete coverage
edges. Freshness makes them visibly stale, which is strictly more informative than deletion —
deleting would destroy the evidence that coverage was ever ingested and leave silence in its place.
Residual case: a symbol deleted by an *edit* keeps its `COVERS` edge, visibly stale, until the
report is re-ingested.

## 5. Verification

Run by the orchestrator, not taken from the implementers.

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 692 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

610 → 648 (6a) → 691 (6b) → **692** (orchestrator's completeness pin). Zero new dependencies;
`Cargo.toml`, `Cargo.lock` and `third_party/` unchanged. No schema migration — `entity.kind` and
`assertion.relation` are `TEXT`, verified against the schema rather than assumed.

### T9, by attack

| Attack | Observed |
|---|---|
| `../../../../etc/passwd`, `../outside.ts`, `/etc/passwd` | `path-refused` ×3, 0 edges; every text column scanned for the payload → **0 rows** |
| Symlinked file and symlinked parent directory escaping the root | `path-refused` ×2, 0 occurrences |
| A file that is not indexed | `file-not-indexed`; **no `File` entity created** — not trusted into existence |
| Line inside no symbol (incl. `u64::MAX`) | `line-outside-any-symbol`; the real line still produced its edge |
| Covered file changed since indexing | `file-changed-since-index` — refused, not mapped through stale extents |
| `.env` named in the report | refused, 0 observations |
| Each 6a bound exceeded | refused **whole**; an over-size report leaves previous evidence untouched |
| 12 malformed reports | all `Ok`, no panic, index still healthy |
| Report path outside the repository | `PathEscapesRoot`, **exit 10** |

### Adversarial mutation probes

Each confirmed applied, confirmed failing for the intended reason, reverted, and the tree confirmed
restored by SHA-256.

| Mutation | Caught by |
|---|---|
| Parser discards `DA:n,0` | 8 tests, incl. `da_records_both_hit_and_explicitly_unhit_lines` |
| Delete the `coverage_run` interface gloss | `every_entity_kind_is_glossed` — 5d-iii's anti-drift guard covers the new vocabulary |
| **Coverage extractor emits `CALLS` instead of `COVERS`** | `the_coverage_extractor_emits_no_call_shaped_relation_at_all` — ADR-0005's actual control |
| Path guard **clamps** an escaping path instead of refusing | 2 T9 tests, incl. the symlink-escape one |
| Drop `md-structural` from `INDEX_EXTRACTOR_IDS` | The new completeness pin, naming the exact offender |

### Independent end-to-end check on the release binary

`init` → `index` → `coverage` → `why`, on `fixtures/ts-coverage`: **6 `COVERS` edges and nothing
else** attributable to the coverage extractor. `nerve why src/math.ts#add --relation COVERS` reports
`COVERS <- coverage_run lcov.info`, `TEST_COVERAGE / INFERRED coverage 1.0.0`, `freshness fresh`,
with `covered_file_content_hash`, `report_content_hash`, `covered_lines`, `instrumented_lines` and
the mapping rule in `details`. After appending a function to the covered file and re-indexing, the
same assertion reads **`freshness stale`** and all 6 edges **survive** — the two behaviours the
withdrawal split and the freshness anchor exist to produce.

### Fixture precision — a regression gate, not an accuracy claim

`fixtures/ts-coverage`: 8 symbols, 6 expected edges, **FP = 0, FN = 0**. Every symbol must appear in
`covers` or `no_edge`, so a forgotten symbol cannot pass as a silent absence. One hand-built corpus
of 8 symbols says nothing about decorators, generated code or source maps. It says the mapping
cannot change without someone noticing.

## 6. Known limitations

- **Affected-test analysis is unsupported**, and will remain so for aggregate reports. This is a
  property of the format, not of Nerve.
- Coverage says a symbol was **entered**, not that it was fully executed. `partial` records this;
  it is never rounded.
- A symbol deleted by an edit keeps a stale `COVERS` edge until the report is re-ingested.
- Only LCOV is supported. Istanbul `coverage-final.json` is not parsed.
- Precision is measured on one 8-symbol fixture. **Real-world coverage accuracy is unmeasured.**
