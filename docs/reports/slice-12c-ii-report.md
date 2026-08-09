# Slice 12c-ii — similarity rename hypotheses, and schema v7

**Status:** complete. Row 12 is closed.
**Commits:** `2517736` (plan correction) · `d20abfd` (Pass A, storage) · `aedf952` (Pass B, matcher)
· `8f91cf7` (Pass C, surfaces) · this report and the narrow-width fix.

---

## 1. What this slice is, in one paragraph

12b shipped one kind of rename evidence: a path deleted and a path added in one commit naming the
**same blob oid**. 12c-ii adds the second kind — content that changed *and* moved — measured by a
named, versioned, bounded method, recorded so that a reader can reproduce it, and rendered on every
surface with the threshold it was judged against. The two kinds are never blended, and that is now
enforced by a `CHECK` constraint rather than by convention.

---

## 2. The committed plan was wrong, and one column proved it

The 12c plan asserted at §3 that similarity renames needed **no schema change**, because
`git_rename_hypothesis.evidence` already carried the comment *"exact_content (12b); similar_content
added in 12c"*.

That is false by inspection. The column beside it is `blob_oid TEXT NOT NULL` — **one** blob column,
which exists precisely because an exact-content hypothesis is *defined* by both paths naming the
same blob. A similarity hypothesis is defined by the opposite: two different objects. No value that
column can take is honest. Writing the to-blob makes one column mean two things depending on a
sibling column; a sentinel makes it mean nothing. Both are a measurement hidden inside a label,
which `CLAUDE.md` §3 forbids.

The claim was also too narrow about what similarity evidence *is*: `evidence` and `ambiguity` can
record that a hypothesis is similarity-based and how ambiguous the pairing is, but not the method,
its version, the measured value, the threshold, or whether the candidate set was complete. A
hypothesis a reader cannot reproduce is not inspectable evidence.

§6 was rewritten from a policy into a specification and committed **before** any code was written.

---

## 3. Schema v7

`V1`–`V6` are byte-identical, verified by extracting each raw SQL string from `HEAD` and from the
working tree and comparing — not by eye.

- **`git_rename_hypothesis` rebuilt** (SQLite create-copy-drop-rename, inside the step's
  transaction): `blob_oid` → `from_blob_oid` + `to_blob_oid`, plus `matcher_id`, `matcher_version`,
  `match_numerator`, `match_denominator`. Lossless: every v6 row copies with both oids set to the
  old one and no measurement, which is exactly what an exact-content row already meant.
- **The measurement is two integers, never a float.** `18 / 20` says what was counted and can be
  checked by hand; `0.9` is comparable against anything and rounds away its own meaning.
- **A `CHECK` constraint** makes an exact row carrying a measurement, or a similar row omitting one,
  a hard `ConstraintViolation`. Proved by mutation probe (§7).
- **`git_rename_analysis`** is per commit and per matcher, because the decisive case has no row to
  carry a flag: when a bound refuses, the commit records *no* hypothesis, and an absence would have
  to be interpreted — the failure `changes_enumerated` exists to prevent.
- **Exact-content renames get no analysis row**, and that is a claim rather than an omission: the
  exact matcher reads no blob content, so it is complete exactly when the diff was enumerated.
- **`git_commit.summary_truncation`** is a three-value vocabulary (`complete`/`truncated`/`unknown`)
  rather than a boolean. A v6 row cannot be backfilled, and length is not the answer — a CLI smoke
  test on a 600-byte subject stored exactly 512 bytes, so `length(summary) = 512 ⟹ truncated` would
  call an untruncated summary cut. The writer uses `>` not `>=`, and a test pins both sides.

Rows 13 and 14 renumbered to **v8** and **v9** in their own plans.

---

## 4. The matcher

`nerve-line-multiset` version `1`. Split both blobs on `\n`, drop a single trailing empty segment,
multiset the raw line bytes (compared as bytes, not hashes, so there is no collision question),
`numerator = Σ min(count_from, count_to)`, `denominator = max(lines_from, lines_to)`. Admission is
`numerator × 8 ≥ 7 × denominator` — integer arithmetic, and a test greps the module's own source to
assert no float appears on the path.

Two properties are documented rather than patched: it cannot see line **order**, so a reordered file
measures 1/1; and it cannot tell shared content from shared boilerplate — a licence header is lines
like any other. §5 is how the second is handled, and the answer is the threshold, not a heuristic.

**A copy is never called a move, structurally.** A copy leaves the source in the tree, so the source
is not a deletion, so the pair is never a candidate. No check to remove.

Bounds are a `SimilarityLimits` struct with `Default` equal to the shipped constants, so every bound
is exercisable end to end by a test constructing tight limits — a bound that cannot be exercised
cannot be tested.

---

## 5. Precision: the threshold is an output of measurement

Ground truth is `fixtures/history-similar/ground_truth.json`, **hand-written before the matcher
existed**, keyed by commit summary rather than object id (an id could only be obtained by running
the generator, which would make the oracle a derived artifact). The generator copies it aside and
back, and hard-errors if it is missing, so the script that builds the corpus structurally cannot
overwrite the file that grades it.

```
similar-content   nerve-line-multiset v1, threshold 7/8
  candidate pairs 13 · admitted 6 · true positives 6 · FALSE POSITIVES 0
  correctly rejected 4 · false negatives 1 · unmeasurable 2
  recall 6/7 over measurable ground-truth renames
  ambiguity {many_from: 2, many_to: 2, unique: 2}

exact-content     git-blob-oid v1, no threshold and no measurement
  candidate pairs 1 · admitted 1 · false positives 0
```

**Two tables. Never summed, never averaged.**

The deciding case is `c5`: two unrelated documents sharing a sixteen-line licence header measure
**16/20, which is 4/5 exactly**, so any threshold at or below 0.80 admits it and FP = 0 becomes
unreachable. 7/8 rejects it (`128 < 140`) and still admits `c1`'s 18/20 (`144 ≥ 140`).

The recall costs a real case and the test **asserts it is reported**: `c2` is a genuine move with
twelve of twenty lines rewritten, measures 8/20, and is published as a false negative. Asserting its
presence is what stops a later change lowering the threshold to flatter the number.

**Independently verified by the orchestrator**: every measurement was recomputed from the raw git
objects (`git cat-file` the blobs, split, multiset-intersect in Python) and matched the oracle case
for case.

### The residual, stated

The method cannot distinguish a boilerplate-dominated pair from a rename *above* the threshold. FP =
0 holds on the measured corpus; a pair whose shared content exceeds 7/8 without being a rename is
outside what a pairwise line method can separate. That is why the output is a **hypothesis carrying
its measurement and method**, never a conclusion.

---

## 6. Surfaces

Every surface carries: hypothesis-not-confirmed-rename, evidence kind, `matcher_id`,
`matcher_version`, the measurement as numerator-of-denominator, the **threshold**, ambiguity,
candidate-set completeness, both paths, both blob oids, the commit, and the limitations.

The threshold is read from `git_rename_analysis` rather than from a build constant, so a row
measured under an older threshold renders the number it was actually judged by.

**The analysis join covers listed commits, not only commits with hypotheses.** A commit whose
candidate set a bound refused records no hypothesis, so there is no row for a per-row field to hang
on; without the join, its silence would read as "nothing moved here".

`analysis` is null for two different reasons, and `analysis_absent_note` names which — from one
`nerve-store` function, so the CLI, HTTP and MCP cannot paraphrase it into three sentences.

**§6.7 is met**: no surface renders a commit summary without `summary_truncation`. The HTTP fix
covers all five API call sites and the whole MCP surface, because MCP wraps the API JSON into
`repository_content` — so the flag travels inside the trust envelope with the prose it qualifies.

---

## 7. Mutation probes

Each applied, each failing a **named** test for the intended reason, each reverted and confirmed
byte-identical by `md5`, each followed by a green re-run.

| probe | test that failed |
|---|---|
| threshold 7/8 → 4/5 | precision gate: `false positives must be zero, and these were admitted: ["c5 legal/old.txt -> legal/new.txt"]` |
| exact hypothesis given a `1/1` measurement | SQLite `ConstraintViolation` (extended 275) — blending refused by the schema |
| `history` exposed beside the MCP envelope | `no_repository_derived_string_appears_outside_the_untrusted_field` |
| `summary_truncation` dropped from HTTP | `every_commit_this_api_returns_says_whether_its_summary_was_cut` (naming the 600-byte hostile summary it would have rendered complete) |
| an exported gloss table on neither list | `every_gloss_table_the_source_declares_is_on_exactly_one_list` |

---

## 8. Browser QA — and the narrow-width gap is closed

**The obvious approach was a trap.** `--window-size=380,700` writes a 380px PNG, but
`window.innerWidth` reports **500**: Chrome clamps a headless window to a platform minimum, so the
page lays out at 500 and the screenshot is cropped. That is the identical failure to the extension's
`resize_window` that row 7a-ii recorded on 2026-08-02 — trusting it would have re-recorded the same
false claim with a new tool.

The working mechanism is CDP `Emulation.setDeviceMetricsOverride` with `mobile: false`, driven over
Node 22+'s **built-in** `WebSocket` — no new dependency, no network, no install. It **asserts** the
resulting `innerWidth` rather than assuming it. Committed as `scripts/viewport_qa.mjs`.

Measured against the release binary serving `fixtures/history-similar`, at **1600px and 380px**,
across `history/path`, `commits`, `cochange`, `frequency`, `diff`:

- `viewportHonoured: true` at both widths
- **no page-level horizontal overflow** anywhere
- **zero console messages, zero exceptions, zero 4xx** on every route
- the database **byte-identical** across the whole browser session (an earlier apparent difference
  was traced to a second `history sync` the orchestrator had run, not to the browser)
- every §6.6 field visible at 380px, including `18 of 20 lines shared` and `threshold 7 of 8`
- a symbol selector is refused **as a refusal** (`400 · refused_history_path`), not answered with
  the containing file

### One real defect found and fixed

`div.row--wrap` measured `scrollWidth 379` against `clientWidth 304` at 380px. `.chip` is
`white-space: nowrap` — right for a stored value, since `similar_content` split across two lines
reads as two values — and wrong for a chip carrying a 49-character **sentence**, which `.row--wrap`
cannot fix because it wraps *between* chips, never inside one. Fixed with a `chip--prose` modifier
applied to the three sentence-carrying chips, rather than by changing `.chip`, so the tokens keep
not wrapping. Re-measured: only the two `nav` elements overflow, both `overflow-x: auto`, which is
the correct scroll-inside-its-own-container pattern.

**This defect was introduced by this slice and could not have been found before**, because no
previous session had a mechanism that controlled the viewport.

### Not covered

- **Systematic keyboard navigation.** Not attempted; carried into the functional UI parity phase.
- **Corrupt-history UI** through a deliberately damaged object store. Server-side behaviour is
  covered by tests; the UI rendering of it is not.

---

## 9. Verification

```
cargo fmt --all -- --check                        clean
cargo clippy --workspace --all-targets -D warnings clean
cargo test --workspace --no-fail-fast             1532 passed, 0 failed, 2 ignored
cargo build --release                             passed
npm run typecheck / lint / test (apps/nerve-web)  clean, clean, 34 passed
scripts/final_acceptance.sh                       50 passed, 0 failed, 0 skipped
python3 -m unittest discover -s tracers/python    115 ran, OK (1 skipped)
Cargo.lock                                        106 packages, unchanged
SCHEMA_VERSION                                    7
```

Test count 1492 → 1532. Acceptance 43 → 50, and every added check exercises behaviour: it reads
expected numerators and thresholds out of the oracle, asserts the measurement fields are integers,
asserts the CLI never prints "confirmed rename", asserts a blended total is **absent**, and asserts
the database is byte-identical after the reads.

**Safety and clean room.** No new dependency. No `unsafe`, no network, no subprocess in product
code. `no_subprocess.rs` and `no_network.rs` untouched. Server remains loopback-only and read-only.
No competitor product consulted, referenced, or read.

---

## 10. Deviations and defects recorded

1. **Two subagents were killed by session limits** — Pass B after delivering its report, Pass C on
   its final mutation probe. Both times the orchestrator inspected the tree for live mutations,
   verified, and continued. A green suite is the proof, since every probe is built to fail a named
   test.
2. **`fixtures/ts-basic/golden.json`** moved `6 → 7` on its one embedded version line, regenerated
   through the path its own test documents. No entity, assertion, observation or state changed.
3. **A latent FK bug from Pass A**, found by Pass B: `git_rename_analysis` carries the same
   `git_commit` foreign key with no `ON DELETE CASCADE`, but the history repair deleted only two of
   three dependent tables. The first analysed commit would have made `history sync` fail. Fixed and
   asserted.
4. **`make_history_fixtures.sh` determinism refuted** for `fixtures/history-hostile/README.md` — see
   `CONTINUATION.md`. Recorded, not silently fixed.
5. Open defects 4–6 in `CONTINUATION.md` (line-cap vocabulary, post-inflation size check, untested
   `blob-unreadable`) carried forward.

---

## 11. Recommended next slice

**Row 13a — the local registry and revised schema v8.** The Row 13 plan was corrected on
2026-08-08 (`2b91a25`) before implementation: C2 cannot be a local assertion because
`assertion.target_entity_id` is a hard foreign key into the local `entity` table, so every
cross-repository link lives in `contract_link`; the link now carries target snapshots and a
lifecycle; the state set is twelve, not thirteen, with `generated_client_stale` removed as
unreachable. Read that plan's correction blocks before planning.
