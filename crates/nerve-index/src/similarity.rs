//! The `nerve-line-multiset` rename matcher: a **second** evidence value, never a blend.
//!
//! Slice 12b ships [`RenameEvidence::ExactContent`] — one blob oid, two paths, one commit. This
//! module adds the case where content changed *and* moved, and `docs/plans/slice-12c-historical-questions.md`
//! §6 is its specification. Four things in it are load-bearing enough to restate beside the code.
//!
//! # 1. Two integers, and not one float
//!
//! `CLAUDE.md` §3 forbids a generic `confidence: float`. The measurement here is
//! `numerator / denominator` kept **as two integers**, and the admission test is
//! `numerator × threshold_denominator >= threshold_numerator × denominator`. No float is computed
//! anywhere on this path — not to compare, not to report, not to sort. A float would be comparable
//! against anything, would round, and would not say what was counted; *"18 of 20 lines"* is a
//! measurement a reader can check by hand.
//!
//! # 2. A similarity row and an exact-content row are never the same row
//!
//! A candidate pair whose two blob oids are **equal** is skipped outright. That pair is the exact
//! matcher's, and emitting it here as well would put one pairing on disk under two evidence values —
//! the blend §6's first constraint forbids. The schema's `CHECK` would refuse it anyway
//! (`from_blob_oid <> to_blob_oid` for `similar_content`), which is the point: the rule is
//! structural rather than conventional.
//!
//! # 3. A copy is not a move, and that falls out of the candidate rule
//!
//! A candidate is a path **deleted** paired with a path **added** in the same commit. A copy leaves
//! its source in the tree, so the source is not a deletion, so the pair is never a candidate. The
//! requirement that a move be evidenced by a deletion *and* an addition is therefore satisfied by
//! the shape of the input rather than by a check somebody could delete.
//!
//! # 4. An absence is always explained
//!
//! Every commit gets exactly one [`AnalysisRow`] — including a commit with no candidates at all.
//! Zero similarity rows has four different causes ([`RenameAnalysisCompleteness`]) and a reader must
//! never have to guess which: *nothing to pair*, *some pair unmeasurable*, *a bound refused the
//! whole set*, *the diff was never enumerated*. That is the same failure
//! [`nerve_core::vocab::ChangesEnumerated`] exists to prevent one table over.
//!
//! # Two properties of the method, stated because a reader will assume the opposite
//!
//! A multiset **cannot see line order**, so a file whose lines were reordered measures `20/20`.
//! That is a real property of the method and `fixtures/history-similar`'s commit 8 documents it
//! rather than patching it. And it **cannot tell shared content from shared boilerplate** — a
//! licence header is lines like any other. The answer to that is the threshold, chosen so that the
//! measured false-positive count is zero, and not a heuristic.

use std::collections::BTreeMap;

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, RenameAmbiguity, RenameAnalysisCompleteness, RenameEvidence,
    SimilarityUnmeasured,
};
use nerve_store::{AnalysisRow, ChangeRow, RenameRow};

use crate::gitobj::{self, Object, ObjectStore, Oid};

/// The producer named on every similarity rename hypothesis.
///
/// Named in full rather than left implicit, because schema v7 admits more than one matcher: a row
/// that did not name its own producer would have to be attributed by a join a caller might forget,
/// and *"84% similar"* without *"by which method"* is a percentage from nowhere.
pub const MATCHER_ID: &str = "nerve-line-multiset";

/// Version of [`MATCHER_ID`].
///
/// Changing what the matcher counts — the split rule, the normalisation, the threshold — is a bump
/// here, never a silent redefinition of rows already on disk. Rows written by version 1 keep
/// meaning what version 1 meant.
pub const MATCHER_VERSION: &str = "1";

/// Numerator of the admission threshold: 7 of 8.
///
/// **An output of measurement, not a preference.** The gate `docs/plans/slice-12c-historical-questions.md`
/// §6.5 sets is *false positives = 0*, and the cases that decide the number are the ones where two
/// unrelated files share a lot of text: `fixtures/history-similar`'s licence-header pair measures
/// `16/20` and its YAML-scaffold pair measures `14/20`. Any threshold at or below `4/5` admits the
/// licence pair and the false-positive count stops being zero. `7/8 = 0.875` clears it with margin
/// and still admits a 20-line file with two lines edited (`18/20 = 0.9`). Recall is whatever that
/// costs, and it is reported rather than optimised.
pub const SIMILARITY_THRESHOLD_NUMERATOR: i64 = 7;

/// Denominator of the admission threshold. See [`SIMILARITY_THRESHOLD_NUMERATOR`].
pub const SIMILARITY_THRESHOLD_DENOMINATOR: i64 = 8;

/// Bytes of one blob this matcher will turn into lines.
///
/// 1 MiB, sixty-four times **beneath** 12a's [`gitobj::MAX_OBJECT_BYTES`], because a rename matcher
/// has a different appetite from an object reader: a source file that moved is kilobytes, and a
/// megabyte file is a vendored bundle, a minified artifact or a lockfile, where a line ratio is
/// noise even when it is computable. The tighter ceiling is the matcher's own rather than trust in
/// the one below it.
///
/// **A recorded limitation.** [`ObjectStore::read`] has no size preview, so the blob is inflated —
/// under 12a's own bound, its delta-depth limit and its declared-size checks — and *then* measured
/// against this one. This bound therefore caps what the matcher **holds as lines**, not what the
/// reader beneath it inflates.
pub const MAX_SIMILARITY_BLOB_BYTES: usize = 1024 * 1024;

/// Lines of one blob this matcher will hold in memory.
///
/// A 1 MiB blob of nothing but newlines is a million lines, so the byte bound above does not imply
/// this one: the distinct-line map is the allocation a repository chooses, and 50 000 lines is
/// already far past any file a human moved. See [`SimilarityLimits::max_lines`] for why exceeding
/// it refuses the commit rather than the pair.
pub const MAX_SIMILARITY_LINES: usize = 50_000;

/// Deletions in one commit this matcher will consider.
///
/// The left-hand side of the quadratic. 64 is generous for a commit that moved files and small
/// enough that the pair count below stays bounded by something a reader can hold in their head; a
/// licence sweep or a vendored-directory import exceeds it and is refused **loudly**, with
/// [`RenameAnalysisCompleteness::RefusedBound`] recorded, rather than partially paired.
pub const MAX_SIMILARITY_DELETIONS: usize = 64;

/// Additions in one commit this matcher will consider. The right-hand side of the quadratic; same
/// reasoning as [`MAX_SIMILARITY_DELETIONS`].
pub const MAX_SIMILARITY_ADDITIONS: usize = 64;

/// `deletions × additions` this matcher will pair.
///
/// **The bound that actually matters**, because the cost is the product and not either factor: 64
/// deletions and 64 additions is 4 096 pairs, each a multiset intersection. 256 is the ceiling on
/// that product, so the two factor bounds above are the coarse guard and this one is the real
/// budget. Applied to the raw product **before** equal-oid pairs are filtered out, because
/// filtering is itself a pass over the product.
pub const MAX_SIMILARITY_PAIRS: usize = 256;

/// Similarity rows one commit may record.
///
/// Distinct from the pair bound: 256 pairs can admit at most 256 rows, and a commit that admitted
/// more than 64 of them is one where the answer is *"everything looks like everything"* rather than
/// *"here are the renames"*. Recording a hundred mutually ambiguous hypotheses would be publishing
/// noise with the authority of a table.
pub const MAX_SIMILARITY_ROWS_PER_COMMIT: usize = 64;

/// Lines beneath which a ratio is not a measurement.
///
/// Two one-line files that agree measure `1/1`, and that says nothing whatever: at one or two lines
/// the ratio is dominated by whether the file has a shebang. Three is the floor at which a multiset
/// starts to carry information. A blob beneath it is [`SimilarityUnmeasured::BlobTooSmall`] — an
/// unanswered question, never a negative answer.
pub const MIN_SIMILARITY_LINES: usize = 3;

/// What one similarity run is allowed to read and record.
///
/// Shaped like [`crate::gitobj::StoreLimits`] and for the same reason: every bound is a field a
/// test can tighten, so **every bound is exercisable end to end** rather than only in theory. Slice
/// 12c-i-b was a corrective slice for a bound that could not be reached from outside, and a bound
/// that cannot be exercised cannot be tested.
///
/// The admission threshold is deliberately **not** a field here. It is
/// [`SIMILARITY_THRESHOLD_NUMERATOR`] / [`SIMILARITY_THRESHOLD_DENOMINATOR`] and nothing else: a
/// tunable threshold would let a test lower the gate until its corpus passed, which is exactly the
/// trade `docs/plans/slice-12c-historical-questions.md` §6.5 refuses. Bounds decide how much work
/// is done; the threshold decides what is claimed, and only one of those is a knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimilarityLimits {
    /// [`MAX_SIMILARITY_BLOB_BYTES`]. Exceeding it is [`SimilarityUnmeasured::BlobTooLarge`] for
    /// that pair, not a refusal of the commit: one oversized blob among many says nothing about the
    /// others.
    pub max_blob_bytes: usize,
    /// [`MAX_SIMILARITY_LINES`]. Exceeding it refuses the **commit**
    /// ([`RenameAnalysisCompleteness::RefusedBound`]), unlike every other per-blob condition.
    ///
    /// That asymmetry is deliberate and it is a gap in the plan made visible rather than papered
    /// over: [`SimilarityUnmeasured`] is a closed vocabulary with no value naming a line ceiling,
    /// and recording this as [`SimilarityUnmeasured::BlobTooLarge`] would attach a stored reason
    /// whose own text says *"exceeded the matcher's own byte bound"* to a blob that did not. A
    /// refusal that is true beats a reason that is nearly right.
    pub max_lines: usize,
    /// [`MAX_SIMILARITY_DELETIONS`].
    pub max_deletions: usize,
    /// [`MAX_SIMILARITY_ADDITIONS`].
    pub max_additions: usize,
    /// [`MAX_SIMILARITY_PAIRS`], applied to `deletions × additions`.
    pub max_pairs: usize,
    /// [`MAX_SIMILARITY_ROWS_PER_COMMIT`].
    pub max_rows_per_commit: usize,
    /// [`MIN_SIMILARITY_LINES`]. A blob of **zero** lines is too small whatever this is set to,
    /// because a ratio needs a denominator.
    pub min_lines: usize,
}

impl Default for SimilarityLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: MAX_SIMILARITY_BLOB_BYTES,
            max_lines: MAX_SIMILARITY_LINES,
            max_deletions: MAX_SIMILARITY_DELETIONS,
            max_additions: MAX_SIMILARITY_ADDITIONS,
            max_pairs: MAX_SIMILARITY_PAIRS,
            max_rows_per_commit: MAX_SIMILARITY_ROWS_PER_COMMIT,
            min_lines: MIN_SIMILARITY_LINES,
        }
    }
}

/// What one commit's similarity run produced: the rows, and the account of the candidate set.
///
/// The two are returned together and written together, because the account is what makes an empty
/// [`SimilarityAnalysis::rows`] readable. Splitting them would let a caller record hypotheses
/// without recording what they are a subset of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarityAnalysis {
    /// Admitted hypotheses, ordered by `(from_path, to_path)`. Empty whenever the completeness is
    /// [`RenameAnalysisCompleteness::RefusedBound`] — that is the whole meaning of the value.
    pub rows: Vec<RenameRow>,
    /// Exactly one row per commit, whatever happened. Written even for a commit with no candidates
    /// at all, because an absence must be explained rather than interpreted.
    pub analysis: AnalysisRow,
}

/// Whether a measurement meets the shipped threshold, in integers only.
///
/// `numerator × threshold_denominator >= threshold_numerator × denominator`. Exposed because every
/// surface that renders a measurement has to say whether it cleared the gate, and each surface
/// re-deriving that from two integers is how a `>` becomes a `>=` in one place and not another.
///
/// A `denominator` of zero is not admissible: there is no ratio to compare, and returning `true`
/// for it would admit a pair that was never measured.
pub fn admits(numerator: i64, denominator: i64) -> bool {
    if denominator <= 0 || numerator < 0 {
        return false;
    }
    numerator.saturating_mul(SIMILARITY_THRESHOLD_DENOMINATOR)
        >= SIMILARITY_THRESHOLD_NUMERATOR.saturating_mul(denominator)
}

/// Measure one commit's rename candidates and account for the whole candidate set.
///
/// `changes` is the commit's enumerated diff and `enumeration` is what
/// [`nerve_core::vocab::ChangesEnumerated`] says about it. When the diff was **not** enumerated the
/// answer is [`RenameAnalysisCompleteness::NotAttempted`] with zeroes: there was no candidate set,
/// which says nothing at all about whether the commit renamed anything.
///
/// Each candidate blob is inflated **once per commit** and cached as its line multiset, so a
/// one-to-many commit reads one deleted blob once rather than once per pair.
pub fn analyse(
    store: &ObjectStore,
    commit_oid: &str,
    changes: &[ChangeRow],
    enumeration: ChangesEnumerated,
    limits: &SimilarityLimits,
) -> SimilarityAnalysis {
    let mut run = Run::new(commit_oid);
    if enumeration != ChangesEnumerated::Enumerated {
        return run.not_attempted();
    }

    let mut deletions: Vec<Side<'_>> = Vec::new();
    let mut additions: Vec<Side<'_>> = Vec::new();
    for change in changes {
        match change.change_kind {
            ChangeKind::Deleted => {
                if let Some(blob) = change.prev_blob_oid.as_deref() {
                    deletions.push(Side {
                        path: &change.path,
                        blob,
                    });
                }
            }
            ChangeKind::Added => {
                if let Some(blob) = change.blob_oid.as_deref() {
                    additions.push(Side {
                        path: &change.path,
                        blob,
                    });
                }
            }
            ChangeKind::Modified | ChangeKind::ModeChanged => {}
        }
    }
    run.deletions = deletions.len();
    run.additions = additions.len();

    // One empty side is a **complete** answer of zero, and it is decided before the bounds. A
    // commit that adds five hundred files and deletes none cannot contain a move whatever the
    // bounds say, so refusing it would report *"Nerve did not look"* about a question that has a
    // provable answer — the opposite of what `RefusedBound` is for.
    let product = deletions.len().saturating_mul(additions.len());
    if product == 0 {
        return run.complete(Vec::new(), 0, BTreeMap::new());
    }

    // The two factor bounds, then the product. Checked before any blob is read, because the point
    // of the bound is not to do the work.
    if deletions.len() > limits.max_deletions
        || additions.len() > limits.max_additions
        || product > limits.max_pairs
    {
        return run.refused(product, 0, BTreeMap::new());
    }

    // A pair whose blobs are equal is the exact matcher's row. Skipped here rather than measured
    // and discarded, so it never costs an inflation either.
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    for (from, deletion) in deletions.iter().enumerate() {
        for (to, addition) in additions.iter().enumerate() {
            if deletion.blob != addition.blob {
                candidates.push((from, to));
            }
        }
    }
    run.considered = candidates.len();
    if candidates.is_empty() {
        return run.complete(Vec::new(), 0, BTreeMap::new());
    }

    // ---- read each candidate blob exactly once ------------------------------------------------
    let mut cache: BTreeMap<&str, Result<Lines, BlobRefusal>> = BTreeMap::new();
    for (from, to) in &candidates {
        for blob in [deletions[*from].blob, additions[*to].blob] {
            cache
                .entry(blob)
                .or_insert_with(|| read_lines(store, blob, limits));
        }
    }
    if cache
        .values()
        .any(|entry| matches!(entry, Err(BlobRefusal::LineBound)))
    {
        // The one per-blob condition that refuses the whole commit. See
        // [`SimilarityLimits::max_lines`] for why it cannot honestly be a per-pair reason.
        return run.refused(candidates.len(), 0, BTreeMap::new());
    }

    // ---- measure -------------------------------------------------------------------------------
    let mut unmeasured: BTreeMap<SimilarityUnmeasured, i64> = BTreeMap::new();
    let mut measured = 0usize;
    let mut admitted: Vec<Admitted<'_>> = Vec::new();
    for (from, to) in &candidates {
        let deletion = deletions[*from];
        let addition = additions[*to];
        // One reason per unmeasured pair, the deleted side first, so the counts in `unmeasured` sum
        // to exactly `pairs_considered - pairs_measured` and a reader can check that by hand.
        let (Ok(left), Ok(right)) = (&cache[deletion.blob], &cache[addition.blob]) else {
            let reason = match (&cache[deletion.blob], &cache[addition.blob]) {
                (Err(BlobRefusal::Unmeasured(reason)), _)
                | (_, Err(BlobRefusal::Unmeasured(reason))) => *reason,
                // Unreachable: the line-bound case returned above. Recorded as unreadable rather
                // than by panicking, because a matcher must not be the thing that stops a sync.
                _ => SimilarityUnmeasured::BlobUnreadable,
            };
            *unmeasured.entry(reason).or_insert(0) += 1;
            continue;
        };
        measured += 1;
        let numerator = left.intersection(right);
        let denominator = as_i64(left.total.max(right.total));
        if admits(numerator, denominator) {
            admitted.push(Admitted {
                from: deletion,
                to: addition,
                numerator,
                denominator,
            });
        }
    }

    if admitted.len() > limits.max_rows_per_commit {
        // Measured, then refused. The counts stay true — they describe work that happened — and the
        // completeness says the rows are absent by refusal rather than by there being none.
        return run.refused(candidates.len(), measured, unmeasured);
    }

    let rows = rows_from(commit_oid, &admitted);
    if unmeasured.is_empty() {
        run.complete(rows, measured, unmeasured)
    } else {
        run.partial(rows, measured, unmeasured)
    }
}

/// One side of a candidate pair: a path and the blob it named.
#[derive(Debug, Clone, Copy)]
struct Side<'a> {
    path: &'a str,
    blob: &'a str,
}

/// A pair that cleared the threshold, before ambiguity is known.
///
/// Ambiguity is a property of the **admitted** set rather than of the candidate set, so it cannot be
/// decided while pairs are still being measured.
#[derive(Debug, Clone, Copy)]
struct Admitted<'a> {
    from: Side<'a>,
    to: Side<'a>,
    numerator: i64,
    denominator: i64,
}

/// Turn the admitted set into rows, deciding ambiguity over that set.
///
/// The shape mirrors the exact matcher's exactly: `to_count` is how many admitted pairs share this
/// deleted path, `from_count` is how many share this added path, and `(1, 1)` is the only
/// unambiguous answer. **No pairing is promoted**, here as there — one deleted file that plausibly
/// became two is two rows carrying [`RenameAmbiguity::ManyTo`], not a winner.
fn rows_from(commit_oid: &str, admitted: &[Admitted<'_>]) -> Vec<RenameRow> {
    let mut by_from: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_to: BTreeMap<&str, usize> = BTreeMap::new();
    for pair in admitted {
        *by_from.entry(pair.from.path).or_insert(0) += 1;
        *by_to.entry(pair.to.path).or_insert(0) += 1;
    }

    let mut rows: Vec<RenameRow> = admitted
        .iter()
        .map(|pair| {
            let to_count = by_from[pair.from.path];
            let from_count = by_to[pair.to.path];
            let ambiguity = match (from_count > 1, to_count > 1) {
                (false, false) => RenameAmbiguity::Unique,
                (false, true) => RenameAmbiguity::ManyTo,
                (true, false) => RenameAmbiguity::ManyFrom,
                (true, true) => RenameAmbiguity::ManyBoth,
            };
            RenameRow {
                commit_oid: commit_oid.to_string(),
                from_path: pair.from.path.to_string(),
                to_path: pair.to.path.to_string(),
                evidence: RenameEvidence::SimilarContent,
                from_blob_oid: pair.from.blob.to_string(),
                to_blob_oid: pair.to.blob.to_string(),
                matcher_id: MATCHER_ID.to_string(),
                matcher_version: MATCHER_VERSION.to_string(),
                match_numerator: Some(pair.numerator),
                match_denominator: Some(pair.denominator),
                ambiguity,
            }
        })
        .collect();
    rows.sort_by(|left, right| {
        (&left.from_path, &left.to_path).cmp(&(&right.from_path, &right.to_path))
    });
    rows
}

/// The mutable tally one commit's run accumulates, and the four ways it can end.
///
/// A struct rather than six loose locals because every exit has to fill the *same* row: an
/// [`AnalysisRow`] built ad hoc at four return sites is how one of them ends up with a threshold of
/// zero or a matcher id that does not match the hypotheses beside it.
struct Run<'a> {
    commit_oid: &'a str,
    deletions: usize,
    additions: usize,
    considered: usize,
}

impl<'a> Run<'a> {
    fn new(commit_oid: &'a str) -> Self {
        Self {
            commit_oid,
            deletions: 0,
            additions: 0,
            considered: 0,
        }
    }

    fn row(
        &self,
        considered: usize,
        measured: usize,
        completeness: RenameAnalysisCompleteness,
        unmeasured: BTreeMap<SimilarityUnmeasured, i64>,
    ) -> AnalysisRow {
        AnalysisRow {
            commit_oid: self.commit_oid.to_string(),
            matcher_id: MATCHER_ID.to_string(),
            matcher_version: MATCHER_VERSION.to_string(),
            threshold_numerator: SIMILARITY_THRESHOLD_NUMERATOR,
            threshold_denominator: SIMILARITY_THRESHOLD_DENOMINATOR,
            deletions_considered: as_i64(self.deletions),
            additions_considered: as_i64(self.additions),
            pairs_considered: as_i64(considered),
            pairs_measured: as_i64(measured),
            completeness,
            unmeasured,
        }
    }

    fn not_attempted(&self) -> SimilarityAnalysis {
        SimilarityAnalysis {
            rows: Vec::new(),
            analysis: self.row(
                0,
                0,
                RenameAnalysisCompleteness::NotAttempted,
                BTreeMap::new(),
            ),
        }
    }

    /// A bound refused. **No row is written for this commit**, whatever was measured before the
    /// refusal, and `pairs_considered` is the set that was too big rather than the part that fitted.
    fn refused(
        &self,
        considered: usize,
        measured: usize,
        unmeasured: BTreeMap<SimilarityUnmeasured, i64>,
    ) -> SimilarityAnalysis {
        SimilarityAnalysis {
            rows: Vec::new(),
            analysis: self.row(
                considered,
                measured,
                RenameAnalysisCompleteness::RefusedBound,
                unmeasured,
            ),
        }
    }

    fn complete(
        &self,
        rows: Vec<RenameRow>,
        measured: usize,
        unmeasured: BTreeMap<SimilarityUnmeasured, i64>,
    ) -> SimilarityAnalysis {
        SimilarityAnalysis {
            rows,
            analysis: self.row(
                self.considered,
                measured,
                RenameAnalysisCompleteness::Complete,
                unmeasured,
            ),
        }
    }

    fn partial(
        &self,
        rows: Vec<RenameRow>,
        measured: usize,
        unmeasured: BTreeMap<SimilarityUnmeasured, i64>,
    ) -> SimilarityAnalysis {
        SimilarityAnalysis {
            rows,
            analysis: self.row(
                self.considered,
                measured,
                RenameAnalysisCompleteness::Partial,
                unmeasured,
            ),
        }
    }
}

/// Why one blob produced no lines.
///
/// Two shapes rather than one, because they have different consequences: an
/// [`BlobRefusal::Unmeasured`] reason is a *pair* that goes unmeasured and is counted in the
/// analysis row, while [`BlobRefusal::LineBound`] refuses the **commit**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlobRefusal {
    /// A per-pair reason, recorded in `git_rename_analysis.unmeasured`.
    Unmeasured(SimilarityUnmeasured),
    /// The blob had more lines than [`SimilarityLimits::max_lines`].
    LineBound,
}

/// One blob's line multiset.
///
/// Keyed by the **raw line bytes**, so two lines are the same line exactly when their bytes are.
/// Not by a hash: a hash would introduce a collision question this matcher would then have to
/// reason about, and the whole content is already in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lines {
    counts: BTreeMap<Vec<u8>, usize>,
    total: usize,
}

impl Lines {
    /// `Σ over distinct lines of min(count_here, count_there)` — the multiset intersection.
    ///
    /// Iterates whichever map has fewer distinct lines, which changes the cost and not the answer:
    /// `min(a, b)` is symmetric, and a line absent from one side contributes `min(n, 0) = 0`.
    fn intersection(&self, other: &Lines) -> i64 {
        let (small, large) = if self.counts.len() <= other.counts.len() {
            (self, other)
        } else {
            (other, self)
        };
        let mut shared = 0usize;
        for (line, count) in &small.counts {
            if let Some(mirror) = large.counts.get(line) {
                shared += *count.min(mirror);
            }
        }
        as_i64(shared)
    }
}

/// Inflate one blob and split it into its line multiset.
fn read_lines(
    store: &ObjectStore,
    blob: &str,
    limits: &SimilarityLimits,
) -> Result<Lines, BlobRefusal> {
    let Some(oid) = Oid::from_hex(blob) else {
        // A stored oid that is not 40 hex characters cannot name an object at all. Unreadable
        // rather than absent: absence is a claim about the store, and nothing was asked of it.
        return Err(BlobRefusal::Unmeasured(
            SimilarityUnmeasured::BlobUnreadable,
        ));
    };
    let body = match store.read(&oid) {
        Ok(Some(Object::Blob(body))) => body,
        // The tree said blob and the store answered otherwise. Not measurable, and not a lie the
        // matcher is willing to tell by treating a tree's bytes as text.
        Ok(Some(_)) => {
            return Err(BlobRefusal::Unmeasured(
                SimilarityUnmeasured::BlobUnreadable,
            ))
        }
        Ok(None) => return Err(BlobRefusal::Unmeasured(SimilarityUnmeasured::BlobAbsent)),
        Err(gitobj::Error::ObjectTooLarge { .. }) => {
            return Err(BlobRefusal::Unmeasured(SimilarityUnmeasured::BlobTooLarge))
        }
        Err(_) => {
            return Err(BlobRefusal::Unmeasured(
                SimilarityUnmeasured::BlobUnreadable,
            ))
        }
    };
    if body.len() > limits.max_blob_bytes {
        return Err(BlobRefusal::Unmeasured(SimilarityUnmeasured::BlobTooLarge));
    }
    // Binary content has no lines, so a ratio over it is a number without a meaning. `NUL` is the
    // same test Git itself uses to decide a file is binary, and it is applied to the whole blob
    // rather than to a prefix, because a prefix test is a guess.
    if body.contains(&0) {
        return Err(BlobRefusal::Unmeasured(SimilarityUnmeasured::BlobBinary));
    }

    // Split on `\n`, dropping a **single** trailing empty segment so that `"a\nb\n"` and `"a\nb"`
    // are both two lines. No trimming, no case folding, no line-ending normalisation: the bytes are
    // compared as Git stored them, and a matcher that normalised would be measuring a file nobody
    // committed.
    let mut segments: Vec<&[u8]> = body.split(|byte| *byte == b'\n').collect();
    if segments.last().is_some_and(|last| last.is_empty()) {
        segments.pop();
    }
    if segments.len() > limits.max_lines {
        return Err(BlobRefusal::LineBound);
    }
    // Zero lines is too small whatever the floor is set to: there is no denominator.
    if segments.len() < limits.min_lines || segments.is_empty() {
        return Err(BlobRefusal::Unmeasured(SimilarityUnmeasured::BlobTooSmall));
    }

    let mut counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for line in &segments {
        *counts.entry(line.to_vec()).or_insert(0) += 1;
    }
    Ok(Lines {
        counts,
        total: segments.len(),
    })
}

/// A count as the schema stores it. Saturating rather than panicking: every input is already
/// bounded far beneath [`i64::MAX`], and a matcher must not be the thing that stops a sync.
fn as_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::gitobj::ObjectKind;

    /// Where a loose object lives: `objects/<first two hex>/<remaining thirty-eight>`.
    fn loose_path(objects_dir: &Path, oid: &str) -> PathBuf {
        objects_dir.join(&oid[..2]).join(&oid[2..])
    }

    /// Write a loose blob under an **invented** id, which is sound because 12a deliberately does not
    /// verify content against its object id (a non-check it states on `StoreLimits`).
    fn write_blob(objects_dir: &Path, oid: &str, body: &[u8]) {
        let path = loose_path(objects_dir, oid);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut raw = format!("{} {}\0", ObjectKind::Blob.as_str(), body.len()).into_bytes();
        raw.extend_from_slice(body);
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        std::fs::write(path, encoder.finish().unwrap()).unwrap();
    }

    fn oid(seed: u8) -> String {
        format!("{seed:02x}").repeat(20)
    }

    fn store_with(blobs: Vec<(String, Vec<u8>)>) -> (tempfile::TempDir, ObjectStore) {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(git.join("objects")).unwrap();
        for (id, body) in &blobs {
            write_blob(&git.join("objects"), id, body);
        }
        let store = ObjectStore::open(&git).unwrap();
        (dir, store)
    }

    fn deleted(path: &str, blob: &str) -> ChangeRow {
        ChangeRow {
            commit_oid: "c".to_string(),
            path: path.to_string(),
            change_kind: ChangeKind::Deleted,
            blob_oid: None,
            prev_blob_oid: Some(blob.to_string()),
            mode: None,
            prev_mode: None,
        }
    }

    fn added(path: &str, blob: &str) -> ChangeRow {
        ChangeRow {
            commit_oid: "c".to_string(),
            path: path.to_string(),
            change_kind: ChangeKind::Added,
            blob_oid: Some(blob.to_string()),
            prev_blob_oid: None,
            mode: None,
            prev_mode: None,
        }
    }

    /// `n` distinct lines with `prefix`, numbered from `first`.
    fn lines(prefix: &str, first: usize, count: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for index in first..first + count {
            out.extend_from_slice(format!("{prefix} line {index:02}\n").as_bytes());
        }
        out
    }

    fn run(changes: &[ChangeRow], blobs: Vec<(String, Vec<u8>)>) -> SimilarityAnalysis {
        let (_dir, store) = store_with(blobs);
        analyse(
            &store,
            "c",
            changes,
            ChangesEnumerated::Enumerated,
            &SimilarityLimits::default(),
        )
    }

    /// **The measurement, asserted as exact integers.** 18 shared lines of 20 is `18/20`, and the
    /// row carries those two numbers rather than anything derived from them.
    #[test]
    fn a_measurement_is_two_integers_and_the_threshold_is_integer_arithmetic() {
        let mut moved = lines("alpha", 1, 18);
        moved.extend_from_slice(&lines("alpha-new", 19, 2));
        let analysis = run(
            &[deleted("a.txt", &oid(1)), added("b.txt", &oid(2))],
            vec![(oid(1), lines("alpha", 1, 20)), (oid(2), moved)],
        );
        assert_eq!(analysis.rows.len(), 1);
        let row = &analysis.rows[0];
        assert_eq!(row.match_numerator, Some(18));
        assert_eq!(row.match_denominator, Some(20));
        assert_eq!(row.evidence, RenameEvidence::SimilarContent);
        assert_eq!(row.matcher_id, MATCHER_ID);
        assert_eq!(row.matcher_version, MATCHER_VERSION);
        assert_eq!(row.ambiguity, RenameAmbiguity::Unique);
        assert_eq!(row.from_blob_oid, oid(1));
        assert_eq!(row.to_blob_oid, oid(2));
        assert_eq!(
            analysis.analysis.completeness,
            RenameAnalysisCompleteness::Complete
        );
        assert_eq!(analysis.analysis.pairs_considered, 1);
        assert_eq!(analysis.analysis.pairs_measured, 1);
        assert!(analysis.analysis.unmeasured.is_empty());

        // The gate itself, at and around the boundary, in integers.
        assert!(admits(7, 8));
        assert!(admits(875, 1000));
        assert!(!admits(874, 1000));
        assert!(!admits(3, 4));
        assert!(!admits(4, 5));
        assert!(!admits(1, 0), "no ratio, no admission");
    }

    /// A multiset cannot see order, and that is documented rather than patched.
    #[test]
    fn the_same_lines_reordered_measure_whole() {
        let forward = lines("ord", 1, 20);
        let mut reversed: Vec<Vec<u8>> = forward
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(<[u8]>::to_vec)
            .collect();
        reversed.reverse();
        let mut shuffled = Vec::new();
        for line in reversed {
            shuffled.extend_from_slice(&line);
            shuffled.push(b'\n');
        }
        assert_ne!(forward, shuffled, "the bytes really do differ");

        let analysis = run(
            &[deleted("a.txt", &oid(1)), added("b.txt", &oid(2))],
            vec![(oid(1), forward), (oid(2), shuffled)],
        );
        assert_eq!(analysis.rows.len(), 1);
        assert_eq!(analysis.rows[0].match_numerator, Some(20));
        assert_eq!(analysis.rows[0].match_denominator, Some(20));
    }

    /// `"a\nb\n"` and `"a\nb"` are the same two lines, and a second trailing newline is a real
    /// empty line rather than a second thing to drop.
    #[test]
    fn one_trailing_empty_segment_is_dropped_and_only_one() {
        let limits = SimilarityLimits::default();
        let (_dir, store) = store_with(vec![
            (oid(1), b"a\nb\nc\n".to_vec()),
            (oid(2), b"a\nb\nc".to_vec()),
            (oid(3), b"a\nb\nc\n\n".to_vec()),
        ]);
        let three = read_lines(&store, &oid(1), &limits).unwrap();
        assert_eq!(three.total, 3);
        assert_eq!(read_lines(&store, &oid(2), &limits).unwrap(), three);
        let four = read_lines(&store, &oid(3), &limits).unwrap();
        assert_eq!(four.total, 4, "the second newline ends an empty line");
    }

    /// A blob the store cannot return is [`SimilarityUnmeasured::BlobAbsent`] and the commit is
    /// [`RenameAnalysisCompleteness::Partial`] — **never a silent skip**, which would be
    /// indistinguishable from the two paths being unrelated.
    #[test]
    fn an_absent_blob_is_reported_and_the_commit_is_partial() {
        let analysis = run(
            &[deleted("a.txt", &oid(1)), added("b.txt", &oid(2))],
            vec![(oid(1), lines("alpha", 1, 20))],
        );
        assert!(analysis.rows.is_empty());
        assert_eq!(
            analysis.analysis.completeness,
            RenameAnalysisCompleteness::Partial
        );
        assert_eq!(analysis.analysis.pairs_considered, 1);
        assert_eq!(analysis.analysis.pairs_measured, 0);
        assert_eq!(
            analysis.analysis.unmeasured,
            BTreeMap::from([(SimilarityUnmeasured::BlobAbsent, 1)])
        );
    }

    /// Every unmeasured pair contributes exactly one reason, so the counts sum to
    /// `pairs_considered - pairs_measured` and a reader can check the row by hand.
    #[test]
    fn the_unmeasured_counts_sum_to_the_unmeasured_pairs() {
        let analysis = run(
            &[
                deleted("a.txt", &oid(1)),
                deleted("small.txt", &oid(3)),
                added("b.txt", &oid(2)),
            ],
            vec![
                (oid(1), lines("alpha", 1, 20)),
                (oid(2), lines("beta", 1, 20)),
                (oid(3), b"one\ntwo\n".to_vec()),
            ],
        );
        let row = &analysis.analysis;
        assert_eq!(row.pairs_considered, 2);
        assert_eq!(row.pairs_measured, 1);
        assert_eq!(
            row.unmeasured,
            BTreeMap::from([(SimilarityUnmeasured::BlobTooSmall, 1)])
        );
        assert_eq!(
            row.unmeasured.values().sum::<i64>(),
            row.pairs_considered - row.pairs_measured
        );
        assert_eq!(row.completeness, RenameAnalysisCompleteness::Partial);
    }

    /// A pair whose blobs are equal is the **exact** matcher's row and is never a candidate here.
    #[test]
    fn an_equal_blob_pair_is_not_a_similarity_candidate() {
        let analysis = run(
            &[deleted("a.txt", &oid(1)), added("b.txt", &oid(1))],
            vec![(oid(1), lines("alpha", 1, 20))],
        );
        assert!(analysis.rows.is_empty());
        assert_eq!(analysis.analysis.pairs_considered, 0);
        assert_eq!(
            analysis.analysis.completeness,
            RenameAnalysisCompleteness::Complete,
            "nothing to measure is complete, not partial"
        );
    }

    /// A copy leaves its source in the tree, so it is never a candidate — structurally, rather than
    /// by a check somebody could remove.
    #[test]
    fn a_copy_produces_no_candidate_because_the_source_was_not_deleted() {
        let mut duplicate = lines("copy", 1, 19);
        duplicate.extend_from_slice(&lines("copy-extra", 20, 1));
        let analysis = run(
            &[added("duplicate.txt", &oid(2))],
            vec![(oid(1), lines("copy", 1, 20)), (oid(2), duplicate)],
        );
        assert!(analysis.rows.is_empty());
        assert_eq!(analysis.analysis.pairs_considered, 0);
        assert_eq!(analysis.analysis.deletions_considered, 0);
        assert_eq!(analysis.analysis.additions_considered, 1);
        assert_eq!(
            analysis.analysis.completeness,
            RenameAnalysisCompleteness::Complete
        );

        // And with the addition bound set beneath the addition count, it is **still** complete: an
        // empty deleted side is a provable zero, not a question Nerve declined to answer.
        let (_dir, store) = store_with(vec![(oid(2), lines("copy", 1, 20))]);
        let bounded = analyse(
            &store,
            "c",
            &[added("duplicate.txt", &oid(2))],
            ChangesEnumerated::Enumerated,
            &SimilarityLimits {
                max_additions: 0,
                ..SimilarityLimits::default()
            },
        );
        assert_eq!(
            bounded.analysis.completeness,
            RenameAnalysisCompleteness::Complete,
            "no deletion means no candidate, whatever the bounds say"
        );
    }

    /// One deleted path admitted against two added paths keeps **both** pairings.
    #[test]
    fn ambiguity_is_decided_over_the_admitted_set() {
        let mut one_a = lines("one", 1, 18);
        one_a.extend_from_slice(&lines("one-a", 19, 2));
        let mut one_b = lines("one", 1, 18);
        one_b.extend_from_slice(&lines("one-b", 19, 2));
        let many_to = run(
            &[
                deleted("one.txt", &oid(1)),
                added("one-a.txt", &oid(2)),
                added("one-b.txt", &oid(3)),
            ],
            vec![
                (oid(1), lines("one", 1, 20)),
                (oid(2), one_a),
                (oid(3), one_b),
            ],
        );
        assert_eq!(many_to.rows.len(), 2);
        assert!(many_to
            .rows
            .iter()
            .all(|row| row.ambiguity == RenameAmbiguity::ManyTo));

        let mut left = lines("merged", 1, 18);
        left.extend_from_slice(&lines("left", 19, 2));
        let mut right = lines("merged", 3, 18);
        right.extend_from_slice(&lines("right", 1, 2));
        let many_from = run(
            &[
                deleted("left.txt", &oid(1)),
                deleted("right.txt", &oid(2)),
                added("merged.txt", &oid(3)),
            ],
            vec![
                (oid(1), left),
                (oid(2), right),
                (oid(3), lines("merged", 1, 20)),
            ],
        );
        assert_eq!(many_from.rows.len(), 2);
        assert!(many_from
            .rows
            .iter()
            .all(|row| row.ambiguity == RenameAmbiguity::ManyFrom));

        // And the symmetric shape, which no fixture produces but the vocabulary has.
        let mut second = lines("merged", 1, 18);
        second.extend_from_slice(&lines("second", 19, 2));
        let many_both = run(
            &[
                deleted("left.txt", &oid(1)),
                deleted("right.txt", &oid(2)),
                added("merged.txt", &oid(3)),
                added("second.txt", &oid(4)),
            ],
            vec![
                (oid(1), lines("merged", 1, 20)),
                (oid(2), lines("merged", 1, 20)),
                (oid(3), lines("merged", 1, 20)),
                (oid(4), second),
            ],
        );
        assert_eq!(many_both.rows.len(), 4, "2x2 pairings, not one winner");
        assert!(many_both
            .rows
            .iter()
            .all(|row| row.ambiguity == RenameAmbiguity::ManyBoth));
    }

    /// A diff that was never enumerated is [`RenameAnalysisCompleteness::NotAttempted`] with
    /// zeroes, which says nothing about whether the commit renamed anything.
    #[test]
    fn an_unenumerated_diff_is_not_attempted() {
        let (_dir, store) = store_with(Vec::new());
        for enumeration in [
            ChangesEnumerated::MergeNotEnumerated,
            ChangesEnumerated::ParentUnavailable,
            ChangesEnumerated::Refused,
        ] {
            let analysis = analyse(
                &store,
                "c",
                &[deleted("a.txt", &oid(1)), added("b.txt", &oid(2))],
                enumeration,
                &SimilarityLimits::default(),
            );
            assert!(analysis.rows.is_empty());
            assert_eq!(
                analysis.analysis.completeness,
                RenameAnalysisCompleteness::NotAttempted
            );
            assert_eq!(analysis.analysis.pairs_considered, 0);
            assert_eq!(analysis.analysis.deletions_considered, 0);
        }
    }

    /// Every bound, exercised through a tight [`SimilarityLimits`], and each one refuses the commit
    /// with **no** rows rather than pairing part of the set.
    #[test]
    fn every_set_bound_refuses_the_commit_and_writes_nothing() {
        let mut one_a = lines("one", 1, 18);
        one_a.extend_from_slice(&lines("one-a", 19, 2));
        let mut one_b = lines("one", 1, 18);
        one_b.extend_from_slice(&lines("one-b", 19, 2));
        let changes = [
            deleted("one.txt", &oid(1)),
            added("one-a.txt", &oid(2)),
            added("one-b.txt", &oid(3)),
        ];
        let (_dir, store) = store_with(vec![
            (oid(1), lines("one", 1, 20)),
            (oid(2), one_a),
            (oid(3), one_b),
        ]);

        let tight = |limits: SimilarityLimits| {
            analyse(
                &store,
                "c",
                &changes,
                ChangesEnumerated::Enumerated,
                &limits,
            )
        };
        let base = SimilarityLimits::default();
        assert_eq!(tight(base).rows.len(), 2, "the control admits two");

        for (name, limits, considered) in [
            (
                "deletions",
                SimilarityLimits {
                    max_deletions: 0,
                    ..base
                },
                2,
            ),
            (
                "additions",
                SimilarityLimits {
                    max_additions: 1,
                    ..base
                },
                2,
            ),
            (
                "pairs",
                SimilarityLimits {
                    max_pairs: 1,
                    ..base
                },
                2,
            ),
            (
                "lines",
                SimilarityLimits {
                    max_lines: 5,
                    ..base
                },
                2,
            ),
            (
                "rows",
                SimilarityLimits {
                    max_rows_per_commit: 1,
                    ..base
                },
                2,
            ),
        ] {
            let analysis = tight(limits);
            assert!(analysis.rows.is_empty(), "{name} still wrote rows");
            assert_eq!(
                analysis.analysis.completeness,
                RenameAnalysisCompleteness::RefusedBound,
                "{name}"
            );
            assert_eq!(analysis.analysis.pairs_considered, considered, "{name}");
        }
    }

    /// The two per-blob ceilings are pair-level reasons rather than commit-level refusals.
    #[test]
    fn a_blob_over_the_byte_bound_or_under_the_line_floor_is_a_pair_level_reason() {
        let mut moved = lines("alpha", 1, 18);
        moved.extend_from_slice(&lines("alpha-new", 19, 2));
        let (_dir, store) = store_with(vec![(oid(1), lines("alpha", 1, 20)), (oid(2), moved)]);
        let changes = [deleted("a.txt", &oid(1)), added("b.txt", &oid(2))];
        let base = SimilarityLimits::default();

        let too_large = analyse(
            &store,
            "c",
            &changes,
            ChangesEnumerated::Enumerated,
            &SimilarityLimits {
                max_blob_bytes: 8,
                ..base
            },
        );
        assert!(too_large.rows.is_empty());
        assert_eq!(
            too_large.analysis.completeness,
            RenameAnalysisCompleteness::Partial
        );
        assert_eq!(
            too_large.analysis.unmeasured,
            BTreeMap::from([(SimilarityUnmeasured::BlobTooLarge, 1)])
        );

        let too_small = analyse(
            &store,
            "c",
            &changes,
            ChangesEnumerated::Enumerated,
            &SimilarityLimits {
                min_lines: 21,
                ..base
            },
        );
        assert_eq!(
            too_small.analysis.unmeasured,
            BTreeMap::from([(SimilarityUnmeasured::BlobTooSmall, 1)])
        );
        assert_eq!(
            too_small.analysis.completeness,
            RenameAnalysisCompleteness::Partial
        );
    }

    /// A blob with a `NUL` byte has no lines, so no ratio is computed over it.
    #[test]
    fn a_binary_blob_is_refused_rather_than_measured() {
        let mut binary = lines("alpha", 1, 20);
        binary[3] = 0;
        let analysis = run(
            &[deleted("a.txt", &oid(1)), added("b.txt", &oid(2))],
            vec![(oid(1), lines("alpha", 1, 20)), (oid(2), binary)],
        );
        assert!(analysis.rows.is_empty());
        assert_eq!(
            analysis.analysis.unmeasured,
            BTreeMap::from([(SimilarityUnmeasured::BlobBinary, 1)])
        );
    }

    /// The threshold is the shipped one and the constants say so. A test that read the threshold
    /// out of a mutable option would be measuring whatever the test chose.
    #[test]
    fn the_analysis_row_records_the_shipped_threshold_and_matcher() {
        let analysis = run(&[], Vec::new());
        assert_eq!(analysis.analysis.matcher_id, MATCHER_ID);
        assert_eq!(analysis.analysis.matcher_version, MATCHER_VERSION);
        assert_eq!(analysis.analysis.threshold_numerator, 7);
        assert_eq!(analysis.analysis.threshold_denominator, 8);
        assert_eq!(
            analysis.analysis.completeness,
            RenameAnalysisCompleteness::Complete
        );
    }

    /// **No float on this path.** The module's own source is read and asserted to contain no
    /// floating-point type, cast or literal, because the arithmetic being integral is a property of
    /// the code rather than of any one measurement a test happened to check.
    ///
    /// Comments are stripped before the search rather than searched: English prose contains `as f`
    /// (*"has four"*, *"has fewer"*) and decimal numbers, and a check that tripped over its own
    /// documentation would be turned off rather than fixed. The test module is excluded because it
    /// names the forbidden tokens in order to forbid them.
    #[test]
    fn the_matcher_source_contains_no_floating_point_arithmetic() {
        let product = include_str!("similarity.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a beginning");
        let code: String = product
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains("pub fn analyse") && code.contains("fn intersection"),
            "the comment stripping removed the code as well, so the search proves nothing"
        );
        for token in ["f32", "f64", "as f", "float", "sqrt", "powf", "0.0", "1.0"] {
            assert!(
                !code.contains(token),
                "similarity.rs contains {token:?}: the measurement must stay integral"
            );
        }
    }
}
