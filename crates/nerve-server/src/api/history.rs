//! The read-only history API.
//!
//! Seven endpoints over the two tables Slice 12b writes and the five derived queries Slice 12c-i
//! added. **Not one of them computes anything.** Every number comes from `nerve-store`, every
//! sentence comes from a vocabulary's own note method, and every permission — `may_claim_created`,
//! `may_claim_history_begins_here`, `earlier_changes_may_exist` — is *carried* off the store's
//! answer rather than re-derived here (ARCHITECTURE.md invariant 3, and
//! `docs/plans/slice-12c-historical-questions.md` §9.2).
//!
//! That is the whole difficulty of this surface. The queries were written in 12c-i and the CLI
//! renders them; a second renderer is exactly where "a shallow boundary hides history" becomes "the
//! project starts here" by paraphrase. `crates/nerve-cli/tests/history_wording.rs` scans this file
//! for note prose and fails a copy by name, and `crates/nerve-server/tests/layering.rs` scans it for
//! SQL and for a second graph reader.
//!
//! # What every answer carries
//!
//! One block, assembled in exactly one function — [`block`] — so no endpoint can quietly answer
//! without saying what the answer could not see:
//!
//! ```text
//! repository_id · current_repository_state · requested_subject
//! history_ingested · shallow · shallow_boundary · promisor
//! walk_terminated_by · commits_recorded · commit_budget
//! refusals · reader_version
//! result_kind · freshness · truncation · continuation · limitations
//! ```
//!
//! # Bounds
//!
//! Every list is bounded, and **truncation is a field the store or this module computed, never
//! `len() == limit`** — that guess is false whenever an answer ends exactly on the boundary, which
//! is the case a caller most needs to be right. Where the store counts a total, truncation is a
//! comparison against it; where it cannot (a path's own history has no total), one row past the
//! limit is fetched and cut, which is the CLI's method and is a fact rather than an inference.
//!
//! # Repository text
//!
//! A commit summary and a tree path are repository prose: attacker-influencable wherever
//! contributions are accepted, and never interpreted. They are carried as JSON **string values**,
//! exactly as `shapes::entity` carries an entity name, and never as an object key, a vocabulary
//! field or a code. Making the bytes safe to render is `respond::to_json_bytes`' job and is tested
//! there.

use serde_json::{json, Value};

use nerve_store::{
    ChangeRow, CommitRow, EarlierHistoryUnavailable, FirstLastObserved, HistoryFreshnessReport,
    HistoryTotals, IngestRow, PathChange, RenameRow, StateDiff, StateDiffLimits, StateDiffReport,
};

use super::{Answer, ApiError, Context};
use crate::request::Target;

/// Largest page of commits one request may ask for.
pub const MAX_HISTORY_COMMIT_LIMIT: usize = 500;
/// Commits returned when no limit is given.
pub const DEFAULT_HISTORY_COMMIT_LIMIT: usize = 50;
/// Largest number of change rows one commit's answer may carry.
pub const MAX_HISTORY_CHANGE_LIMIT: usize = 5_000;
/// Change rows returned for one commit when no limit is given.
pub const DEFAULT_HISTORY_CHANGE_LIMIT: usize = 500;
/// Largest page of one path's commits, and of the rename hypotheses naming it.
pub const MAX_HISTORY_PATH_LIMIT: usize = 500;
/// Commits returned for one path when no limit is given.
pub const DEFAULT_HISTORY_PATH_LIMIT: usize = 50;
/// Largest page of change-frequency rows.
pub const MAX_HISTORY_FREQUENCY_LIMIT: usize = 500;
/// Frequency rows returned when no limit is given.
pub const DEFAULT_HISTORY_FREQUENCY_LIMIT: usize = 50;
/// Largest page of co-change pairs.
pub const MAX_HISTORY_COCHANGE_LIMIT: usize = 500;
/// Co-change pairs returned when no limit is given.
pub const DEFAULT_HISTORY_COCHANGE_LIMIT: usize = 50;
/// Largest number of commits one state diff may return.
pub const MAX_HISTORY_DIFF_COMMITS: usize = 500;
/// Largest number of commits a state diff's ancestry pass may visit.
pub const MAX_HISTORY_DIFF_WALK: usize = 5_000;
/// Largest number of change rows one state diff may return.
pub const MAX_HISTORY_DIFF_CHANGES: usize = 5_000;

/// Why an endpoint offers no continuation, said once.
///
/// Slice 8b-ii's recorded decision: a continuation is an offset the query honours, or `null` **with
/// a statement**. Only the commit log's query takes an offset, so the other six say so rather than
/// paging in this module — a second page assembled here would re-run a bounded query and return a
/// different set, not the next one.
pub const CONTINUATION_NOT_OFFERED: &str =
    "this answer is a single bounded page: the query behind it takes a limit and no offset, so \
     there is no continuation to offer. Paging it here would re-run the bound and return a \
     different page rather than the next one";

/// What the history tables answered for one repository, read once per request.
struct Read {
    repo_id: String,
    ingest: Option<IngestRow>,
    totals: Option<HistoryTotals>,
    freshness: HistoryFreshnessReport,
}

/// Open the history for this repository, or refuse with the reason.
///
/// A missing repository row is **not** an absent history: it means this index has never recorded
/// which repository it describes, so there is nothing to key a history question on. Answering it as
/// "no history" would collapse two of the four states §11 requires to stay distinct, so it is a
/// refusal that names itself instead.
///
/// [`nerve_store::history_totals`] is asked only when there is an ingest, for the same reason the
/// CLI does it: a tally over an un-ingested repository is a row of zeroes that reads as "this
/// project has no history".
fn read(ctx: &Context<'_>) -> Result<Read, ApiError> {
    let Some(repo_id) = ctx.repo_id else {
        return Err(ApiError::with_detail(
            409,
            "repository_unknown",
            "this index records no repository, so no history question can be keyed to one",
            json!({ "history_ingested": Value::Null }),
        ));
    };
    let ingest = nerve_store::history_ingest(ctx.conn, repo_id).map_err(ApiError::internal)?;
    let totals = match &ingest {
        Some(_) => {
            Some(nerve_store::history_totals(ctx.conn, repo_id).map_err(ApiError::internal)?)
        }
        None => None,
    };
    let freshness =
        nerve_store::history_freshness(ctx.conn, repo_id).map_err(ApiError::internal)?;
    Ok(Read {
        repo_id: repo_id.to_string(),
        ingest,
        totals,
        freshness,
    })
}

/// A bounded list's truncation, as a fact.
///
/// `total` is `None` where no total exists to compare against — one path's own history has none —
/// and `truncated` is then established by fetching one row past the limit and cutting it. Neither
/// form is `len() == limit`.
struct Truncation {
    returned: usize,
    total: Option<i64>,
    truncated: bool,
    limit: usize,
}

impl Truncation {
    fn value(&self) -> Value {
        json!({
            "returned": self.returned,
            "total": self.total,
            "truncated": self.truncated,
            "limit": self.limit,
        })
    }
}

/// A continuation the query honours, or the statement that there is none.
fn continuation(offset: Option<usize>, next: Option<usize>) -> Value {
    match offset {
        Some(offset) => json!({
            "supported": true,
            "offset": offset,
            "next_offset": next,
            "statement": Value::Null,
        }),
        None => json!({
            "supported": false,
            "offset": Value::Null,
            "next_offset": Value::Null,
            "statement": CONTINUATION_NOT_OFFERED,
        }),
    }
}

/// The availability block every history answer carries, assembled in **one** place.
///
/// Splitting this across endpoints is how one of them ends up answering without its qualification,
/// which is the whole failure mode `docs/plans/slice-12c-historical-questions.md` §9 exists to
/// close. Each caller supplies only what is its own: the subject it was asked about, which of the
/// answers it produced, its truncation and its continuation.
///
/// `limitations` is **structured rather than prose**. Every sentence a history surface may say now
/// belongs to a vocabulary in `nerve-core` or `nerve-store`; a paragraph invented here would be a
/// ninth note nothing owns, and the guard could not tell it from a paraphrase of one that is owned.
fn block(
    read: &Read,
    subject: Value,
    result_kind: &str,
    truncation: Option<&Truncation>,
    continuation: Value,
) -> Value {
    let merges = read.totals.as_ref().map(|totals| totals.merges);
    json!({
        "repository_id": read.repo_id,
        "current_repository_state": {
            "state_id": read.freshness.current_state_id,
            "git_commit": read.freshness.current_git_commit,
        },
        "requested_subject": subject,
        // `false` is "history has never been read here", which is not "this project has no
        // history" and is not a failure. Both tallies below are `null` in that case rather than
        // zero, so a client cannot read "never synced" as "nothing found".
        "history_ingested": read.ingest.is_some(),
        "shallow": read.ingest.as_ref().map(|row| json!(row.shallow)),
        "shallow_boundary": read.ingest.as_ref().map(|row| json!(row.shallow_boundary)),
        "promisor": read.ingest.as_ref().map(|row| json!(row.promisor)),
        "walk_terminated_by": read.ingest.as_ref()
            .map(|row| json!(row.walk_terminated_by.as_str())),
        "walk_terminated_note": read.ingest.as_ref().map(|row| json!(row.walk_terminated_by.note())),
        "commits_recorded": read.ingest.as_ref().map(|row| json!(row.commits_recorded)),
        "commit_budget": read.ingest.as_ref().map(|row| json!(row.commit_budget)),
        "refusals": read.ingest.as_ref().map(|row| json!(row.refusals)),
        "refusals_total": read.ingest.as_ref()
            .map(|row| json!(row.refusals.values().sum::<usize>())),
        "reader_version": read.ingest.as_ref().map(|row| json!(row.reader_version)),
        "totals": read.totals.as_ref().map(totals),
        "result_kind": result_kind,
        "freshness": read.freshness.verdict.as_str(),
        "freshness_note": read.freshness.verdict.note(),
        "ingest_head_oid": read.freshness.ingest_head_oid,
        "truncation": truncation.map(Truncation::value),
        "continuation": continuation,
        "limitations": {
            // The one judgment every history surface must agree on, taken from `nerve-store` and
            // never re-derived. `null` where there is no ingest, because the question is about one.
            "earlier_changes_may_exist": read.ingest.as_ref()
                .map(|row| json!(nerve_store::earlier_changes_may_exist(row))),
            // Data rather than a sentence: a merge enumerates no changes by Slice 12b's decision, so
            // every count derived from `git_change` is a floor against the repository's own log by
            // exactly this many commits, and zero here removes the possibility entirely.
            "merges_in_repository": merges,
            "merges_enumerate_no_changes": true,
            "counts_are_visible_history_only": true,
        },
    })
}

fn totals(totals: &HistoryTotals) -> Value {
    json!({
        "commits": totals.commits,
        "changes": totals.changes,
        "renames": totals.renames,
        "merges": totals.merges,
        "changes_by_kind": totals.changes_by_kind.iter()
            .map(|(kind, count)| (kind.as_str().to_string(), *count))
            .collect::<std::collections::BTreeMap<String, i64>>(),
    })
}

/// One commit. `changes` is `null` where the caller counted rows for one path rather than for the
/// commit — `null` is not `0`, and a client reading an absent count as an empty commit would make
/// exactly the mistake `changes_enumerated` exists to stop.
fn commit(row: &CommitRow, changes: Option<usize>) -> Value {
    json!({
        "commit_oid": row.commit_oid,
        "tree_oid": row.tree_oid,
        "parent_oids": row.parent_oids,
        "is_merge": row.is_merge,
        "parent_completeness": row.parent_completeness.as_str(),
        "parent_completeness_note": row.parent_completeness.note(),
        // Carried, never re-derived. A client that recomputed this from the string would be the
        // second copy of the rule, and the one likeliest to say "history begins here" about a
        // shallow boundary.
        "may_claim_history_begins_here": row.parent_completeness.may_claim_history_begins_here(),
        "changes_enumerated": row.changes_enumerated.as_str(),
        "changes_enumerated_note": row.changes_enumerated.note(),
        "changes": changes,
        "author_time": row.author_time,
        "author_tz": row.author_tz,
        "committer_time": row.committer_time,
        "committer_tz": row.committer_tz,
        "author_ident": row.author_ident,
        "committer_ident": row.committer_ident,
        // Repository prose, carried as a string value and never interpreted.
        "summary": row.summary,
    })
}

fn change(row: &ChangeRow) -> Value {
    json!({
        "path": row.path,
        "change_kind": row.change_kind.as_str(),
        "blob_oid": row.blob_oid,
        "prev_blob_oid": row.prev_blob_oid,
        "mode": row.mode,
        "prev_mode": row.prev_mode,
    })
}

fn rename(row: &RenameRow) -> Value {
    json!({
        "commit_oid": row.commit_oid,
        "from_path": row.from_path,
        "to_path": row.to_path,
        "evidence": row.evidence.as_str(),
        // Two blob oids since schema v7, because a similarity pair has two. For an exact-content
        // hypothesis they are equal, and that identity is the evidence rather than a redundancy.
        "from_blob_oid": row.from_blob_oid,
        "to_blob_oid": row.to_blob_oid,
        // The producer of the row, and its measurement as two integers. A ratio without the
        // method that computed it is a percentage from nowhere, so they travel together or not
        // at all; both measurement fields are null on an exact match, which carries none.
        "matcher_id": row.matcher_id,
        "matcher_version": row.matcher_version,
        "match_numerator": row.match_numerator,
        "match_denominator": row.match_denominator,
        "ambiguity": row.ambiguity.as_str(),
        "ambiguity_note": row.ambiguity.note(),
        // On every row rather than in a footnote a client can drop. Git records no rename; this is a
        // proposal drawn from identical content, and there is no score to sort it by.
        "is_hypothesis": true,
    })
}

fn path_change(observed: &PathChange) -> Value {
    json!({
        "commit": commit(&observed.commit, None),
        "change": change(&observed.change),
    })
}

/// The first/last-observed block, with every permission carried off the store's answer.
fn first_observed(observed: &FirstLastObserved) -> Value {
    json!({
        "path": observed.path,
        "kind": observed.kind.as_str(),
        "kind_note": observed.kind.note(),
        // `FirstObservedKind::may_claim_created` is the only copy of this permission in the
        // workspace, and the store already applied it. Branching on the kind here would be a second.
        "may_claim_created": observed.may_claim_created,
        "may_claim_created_note": observed.kind.created_claim_note(),
        "first": observed.first.as_ref().map(path_change),
        "last": observed.last.as_ref().map(path_change),
        "changes_in_visible_history": observed.changes_in_visible_history,
        "additions_recorded": observed.additions_recorded,
        "merges_in_repository": observed.merges_in_repository,
        "earlier_history_unavailable": observed.earlier_history_unavailable
            .map(EarlierHistoryUnavailable::as_str),
        "earlier_history_unavailable_note": observed.earlier_history_unavailable
            .map(EarlierHistoryUnavailable::note),
        // The repository-level question, beside the path-level one above. Different scopes: a
        // shallow clone can hold a genuine root, so a path created at it has nothing hidden above it
        // while the repository still reports that earlier commits may exist.
        "earlier_changes_may_exist": observed.earlier_changes_may_exist,
        "walk_terminated_by": observed.walk_terminated_by.map(|value| value.as_str()),
        "walk_terminated_note": observed.walk_terminated_by.map(|value| value.note()),
        "shallow": observed.shallow,
        "current_tree": {
            "basis": observed.current_tree.basis,
            "index_exists": observed.current_tree.index_exists,
            "entities_at_path": observed.current_tree.entities_at_path,
        },
    })
}

/// A path parameter, refused rather than answered when it names a symbol.
///
/// `docs/plans/slice-12c-historical-questions.md` §2.1 is a hard gate on **every** surface, not only
/// the command line: `git_change` is keyed on a path, every symbol kind answers `PathRole::None`,
/// and the only dates Nerve could return for `src/app.rs#parse` are `src/app.rs`'s — a different
/// claim wearing the same words.
///
/// The rule is `nerve_store::history_path_refusal`, which Slice 12c-iii-a hoisted out of the CLI
/// binary for this endpoint to call. Writing it again here is the failure the hoist exists to
/// prevent, and **the path the caller probably meant is not guessed**: the refusal says why, never
/// what instead, because answering with the containing file is the wrong claim rather than a
/// narrower one.
fn tree_path<'a>(target: &'a Target, key: &str) -> Result<&'a str, ApiError> {
    let raw = target
        .get(key)
        .ok_or_else(|| ApiError::bad_request(format!("{key} is required")))?;
    match nerve_store::history_path_refusal(raw) {
        None => Ok(raw),
        Some(refusal) => Err(ApiError::with_detail(
            400,
            "refused_history_path",
            format!("{raw:?} is a symbol selector; history takes a path"),
            json!({
                "parameter": key,
                "argument": raw,
                "reason": refusal.as_str(),
                "reason_statement": refusal.statement(),
                "path_guessed": false,
                "nothing_was_looked_up": true,
            }),
        )),
    }
}

// ---- /api/history ---------------------------------------------------------------------------

/// What visible history is unavailable, and whether what was recorded is still current.
///
/// The availability block on its own — the question `nerve history availability` answers. It is a
/// success whatever it finds: an un-ingested repository answers `history_ingested: false` and
/// `freshness: "no_history_ingested"`, which is an absence rather than a failure (§11).
pub fn availability(ctx: &Context<'_>) -> Answer {
    let read = read(ctx)?;
    let kind = match read.ingest {
        Some(_) => "availability",
        None => "no_history_ingested",
    };
    Ok(block(
        &read,
        Value::Null,
        kind,
        None,
        continuation(None, None),
    ))
}

// ---- /api/history/commits -------------------------------------------------------------------

/// The recorded commit log, newest committer time first, bounded and offsettable.
///
/// The one history endpoint whose store query takes an offset, so the one that offers a
/// continuation rather than a statement about not having one.
pub fn commits(ctx: &Context<'_>, target: &Target) -> Answer {
    let read = read(ctx)?;
    let limit = target
        .bounded(
            "limit",
            DEFAULT_HISTORY_COMMIT_LIMIT,
            MAX_HISTORY_COMMIT_LIMIT,
        )
        .map_err(ApiError::bad_request)?;
    let offset = target
        .bounded_from_zero("offset", 0, usize::MAX)
        .map_err(ApiError::bad_request)?;

    let rows = match read.ingest {
        None => Vec::new(),
        Some(_) => nerve_store::commit_log(ctx.conn, &read.repo_id, limit, offset)
            .map_err(ApiError::internal)?,
    };
    // The change *count* per commit, never the rows: a count is only readable next to
    // `changes_enumerated`, which every commit carries.
    let mut counts = Vec::with_capacity(rows.len());
    for row in &rows {
        counts.push(
            nerve_store::changes_for_commit(ctx.conn, &read.repo_id, &row.commit_oid)
                .map_err(ApiError::internal)?
                .len(),
        );
    }

    let total = read.totals.as_ref().map(|totals| totals.commits);
    let consumed = i64::try_from(offset + rows.len()).unwrap_or(i64::MAX);
    let truncation = Truncation {
        returned: rows.len(),
        total,
        truncated: total.is_some_and(|total| total > consumed),
        limit,
    };
    let next = truncation.truncated.then_some(offset + rows.len());

    let mut value = block(
        &read,
        Value::Null,
        if read.ingest.is_some() {
            "commit_log"
        } else {
            "no_history_ingested"
        },
        Some(&truncation),
        continuation(Some(offset), next),
    );
    value["commits"] = json!(rows
        .iter()
        .zip(&counts)
        .map(|(row, changes)| commit(row, Some(*changes)))
        .collect::<Vec<_>>());
    Ok(value)
}

// ---- /api/history/commit --------------------------------------------------------------------

/// What one commit did, or the fact that Nerve never read it.
///
/// **A commit that is not recorded is a refusal, never an empty change list.** "We never read that
/// commit" and "that commit changed nothing" are different answers, and only the second one is an
/// empty list — which itself means one of four things, named by `changes_enumerated`.
pub fn commit_changes(ctx: &Context<'_>, target: &Target) -> Answer {
    let read = read(ctx)?;
    let oid = target
        .get("commit")
        .ok_or_else(|| ApiError::bad_request("commit is required"))?;
    let limit = target
        .bounded(
            "limit",
            DEFAULT_HISTORY_CHANGE_LIMIT,
            MAX_HISTORY_CHANGE_LIMIT,
        )
        .map_err(ApiError::bad_request)?;

    let Some(row) =
        nerve_store::commit_by_oid(ctx.conn, &read.repo_id, oid).map_err(ApiError::internal)?
    else {
        return Err(ApiError::with_detail(
            404,
            "commit_not_recorded",
            format!("{oid:?} is not a recorded commit"),
            json!({
                "commit": oid,
                "history_ingested": read.ingest.is_some(),
                "this_is_not_an_empty_commit": true,
            }),
        ));
    };

    let mut rows = nerve_store::changes_for_commit(ctx.conn, &read.repo_id, &row.commit_oid)
        .map_err(ApiError::internal)?;
    let recorded = rows.len();
    let truncation = Truncation {
        returned: recorded.min(limit),
        total: Some(i64::try_from(recorded).unwrap_or(i64::MAX)),
        truncated: recorded > limit,
        limit,
    };
    rows.truncate(limit);

    let mut value = block(
        &read,
        json!({ "commit": row.commit_oid }),
        "commit_changes",
        Some(&truncation),
        continuation(None, None),
    );
    value["commit"] = commit(&row, Some(recorded));
    value["changes"] = json!(rows.iter().map(change).collect::<Vec<_>>());
    Ok(value)
}

// ---- /api/history/path ----------------------------------------------------------------------

/// One path's history, including the first/last-observed block.
///
/// The path is matched **as a tree recorded it**, literally, and never resolved on disk: a
/// historical path is frequently one that no longer exists, and routing it through a guard that
/// ends in canonicalisation would refuse every deleted path while counting each refusal as
/// path-safety coverage. `nerve-store` opens no path, which is the structural form of that
/// property rather than a promise.
///
/// `first_observed` is asked unconditionally, including where there is no ingest: it answers
/// `no_history_ingested` there, which is one of the four states §11 requires to stay distinct from
/// "the path is unknown" and from "the path is known and nothing touched it".
pub fn path(ctx: &Context<'_>, target: &Target) -> Answer {
    let path = tree_path(target, "path")?;
    let read = read(ctx)?;
    let limit = target
        .bounded("limit", DEFAULT_HISTORY_PATH_LIMIT, MAX_HISTORY_PATH_LIMIT)
        .map_err(ApiError::bad_request)?;

    // One row past the limit, then cut. There is no total for a path the way the commit log has
    // one, so this is the only way truncation can be a fact rather than the guess "we got exactly
    // as many as we asked for", which is false whenever the answer ends on the boundary.
    let probe = limit.saturating_add(1);
    let (mut commits, mut renames) = match read.ingest {
        None => (Vec::new(), Vec::new()),
        Some(_) => (
            nerve_store::commits_touching_path(ctx.conn, &read.repo_id, path, probe)
                .map_err(ApiError::internal)?,
            nerve_store::renames_touching_path(ctx.conn, &read.repo_id, path, probe)
                .map_err(ApiError::internal)?,
        ),
    };
    let commits_truncated = commits.len() > limit;
    let renames_truncated = renames.len() > limit;
    commits.truncate(limit);
    renames.truncate(limit);

    let observed = nerve_store::first_last_observed(ctx.conn, &read.repo_id, path)
        .map_err(ApiError::internal)?;

    let truncation = Truncation {
        returned: commits.len(),
        total: None,
        truncated: commits_truncated,
        limit,
    };
    let mut value = block(
        &read,
        json!({ "path": path, "path_is_as_recorded_in_a_tree": true }),
        if read.ingest.is_some() {
            "path_history"
        } else {
            "no_history_ingested"
        },
        Some(&truncation),
        continuation(None, None),
    );
    value["path"] = json!(path);
    value["first_observed"] = first_observed(&observed);
    value["commits"] = json!(commits
        .iter()
        .map(|(row, row_change)| {
            let mut object = commit(row, None);
            if let Some(fields) = object.as_object_mut() {
                fields.insert("change".into(), change(row_change));
            }
            object
        })
        .collect::<Vec<_>>());
    value["renames"] = json!(renames.iter().map(rename).collect::<Vec<_>>());
    value["renames_count"] = json!(renames.len());
    value["renames_truncated"] = json!(renames_truncated);
    Ok(value)
}

// ---- /api/history/diff ----------------------------------------------------------------------

/// What changed between two recorded states, by **ancestry** and never by a time range.
///
/// A time range is not an ancestry range and answering one for the other fails silently: a merge
/// brings in commits whose committer time precedes it, and a rebase reorders them freely.
///
/// **Four of the five outcomes are not diffs, and none of them is an empty one.** The diff-shaped
/// fields are `null` on those four rather than empty, which is what stops a refusal reading as
/// "nothing changed": `commits: null` says no range was computed, `commits: []` says a range was
/// computed and holds nothing. Every outcome is a `200`, because none of them is an error.
pub fn diff(ctx: &Context<'_>, target: &Target) -> Answer {
    let read = read(ctx)?;
    let from = target
        .get("from")
        .ok_or_else(|| ApiError::bad_request("from is required"))?;
    let to = target
        .get("to")
        .ok_or_else(|| ApiError::bad_request("to is required"))?;
    let limits = StateDiffLimits {
        commits: target
            .bounded(
                "limit",
                DEFAULT_HISTORY_COMMIT_LIMIT,
                MAX_HISTORY_DIFF_COMMITS,
            )
            .map_err(ApiError::bad_request)?,
        commits_walked: target
            .bounded("max_walk", MAX_HISTORY_DIFF_WALK, MAX_HISTORY_DIFF_WALK)
            .map_err(ApiError::bad_request)?,
        changes: target
            .bounded(
                "max_changes",
                DEFAULT_HISTORY_CHANGE_LIMIT,
                MAX_HISTORY_DIFF_CHANGES,
            )
            .map_err(ApiError::bad_request)?,
    };

    let outcome = nerve_store::state_diff(ctx.conn, &read.repo_id, from, to, limits)
        .map_err(ApiError::internal)?;

    let (kind, truncation, detail) = match &outcome {
        StateDiff::StateNotRecorded {
            from_recorded,
            to_recorded,
            ..
        } => (
            "state_not_recorded",
            None,
            json!({
                "from_recorded": from_recorded,
                "to_recorded": to_recorded,
                "this_is_not_an_empty_diff": true,
            }),
        ),
        StateDiff::NotAnAncestor { commits_walked, .. } => (
            "not_an_ancestor",
            None,
            json!({
                "commits_walked": commits_walked,
                "this_is_not_an_empty_diff": true,
            }),
        ),
        StateDiff::AncestryIncomplete {
            stopped_at,
            parent_completeness,
            commits_walked,
            ..
        } => (
            "ancestry_incomplete",
            None,
            json!({
                "stopped_at": stopped_at,
                "stopped_at_parent_completeness": parent_completeness.as_str(),
                "stopped_at_parent_completeness_note": parent_completeness.note(),
                "commits_walked": commits_walked,
                "this_is_not_an_empty_diff": true,
            }),
        ),
        StateDiff::WalkBudgetExhausted {
            commits_walked,
            limit,
            ..
        } => (
            "walk_budget_exhausted",
            None,
            json!({
                "commits_walked": commits_walked,
                "walk_limit": limit,
                "this_is_not_an_empty_diff": true,
            }),
        ),
        StateDiff::Diff(report) => (
            "diff",
            Some(Truncation {
                returned: report.commits.len(),
                total: Some(i64::try_from(report.commits_in_range).unwrap_or(i64::MAX)),
                truncated: report.commits_truncated,
                limit: limits.commits,
            }),
            diff_report(report),
        ),
    };

    let mut value = block(
        &read,
        json!({ "from": from, "to": to }),
        kind,
        truncation.as_ref(),
        continuation(None, None),
    );
    value["from"] = json!(from);
    value["to"] = json!(to);
    value["max_walk"] = json!(limits.commits_walked);
    value["max_changes"] = json!(limits.changes);
    value["ancestry_not_a_time_range"] = json!(true);
    // Every diff-shaped key exists on every outcome and is `null` where no range was computed.
    // Without this a client would find the key absent on a refusal and an empty array on an empty
    // range, which are the two answers this endpoint exists to keep apart.
    for key in [
        "from_recorded",
        "to_recorded",
        "commits_walked",
        "walk_limit",
        "stopped_at",
        "stopped_at_parent_completeness",
        "stopped_at_parent_completeness_note",
        "commits",
        "commits_in_range",
        "commits_truncated",
        "changes",
        "changes_truncated",
        "merges_in_range",
        "changes_enumerated",
        "ancestry_incomplete_at",
        "this_is_not_an_empty_diff",
    ] {
        value[key] = Value::Null;
    }
    if let Value::Object(fields) = detail {
        for (key, field) in fields {
            value[key] = field;
        }
    }
    Ok(value)
}

fn diff_report(report: &StateDiffReport) -> Value {
    json!({
        "commits": report.commits.iter().map(|row| commit(row, None)).collect::<Vec<_>>(),
        "commits_in_range": report.commits_in_range,
        "commits_truncated": report.commits_truncated,
        "commits_walked": report.commits_walked,
        "changes": report.changes.iter().map(|row| {
            let mut object = change(row);
            if let Some(fields) = object.as_object_mut() {
                fields.insert("commit_oid".into(), json!(row.commit_oid));
            }
            object
        }).collect::<Vec<_>>(),
        "changes_truncated": report.changes_truncated,
        // Not decoration: a merge enumerates no changes, so a merge-heavy range carrying few change
        // rows is expected rather than quiet.
        "merges_in_range": report.merges_in_range,
        "changes_enumerated": report.changes_enumerated.iter()
            .map(|(value, count)| (value.as_str().to_string(), *count))
            .collect::<std::collections::BTreeMap<String, usize>>(),
        "ancestry_incomplete_at": report.ancestry_incomplete_at.as_ref()
            .map(|(oid, completeness)| json!({
                "commit_oid": oid,
                "parent_completeness": completeness.as_str(),
                "parent_completeness_note": completeness.note(),
            })),
    })
}

// ---- /api/history/frequency -----------------------------------------------------------------

/// Which paths changed most often in visible history.
///
/// Every count is a **floor**: changes within what Nerve read, not lifetime changes, and merges
/// contribute none. Both facts are on the availability block, carried rather than documented.
pub fn frequency(ctx: &Context<'_>, target: &Target) -> Answer {
    let read = read(ctx)?;
    let limit = target
        .bounded(
            "limit",
            DEFAULT_HISTORY_FREQUENCY_LIMIT,
            MAX_HISTORY_FREQUENCY_LIMIT,
        )
        .map_err(ApiError::bad_request)?;
    let report = nerve_store::change_frequency(ctx.conn, &read.repo_id, limit)
        .map_err(ApiError::internal)?;

    let truncation = Truncation {
        returned: report.rows.len(),
        total: Some(report.paths_total),
        // The store's own comparison against a counted total.
        truncated: report.truncated,
        limit: report.limit,
    };
    let mut value = block(
        &read,
        Value::Null,
        if read.ingest.is_some() {
            "change_frequency"
        } else {
            "no_history_ingested"
        },
        Some(&truncation),
        continuation(None, None),
    );
    value["paths_total"] = json!(report.paths_total);
    value["rows"] = json!(report
        .rows
        .iter()
        .map(|row| json!({ "path": row.path, "commits": row.commits }))
        .collect::<Vec<_>>());
    Ok(value)
}

// ---- /api/history/cochange ------------------------------------------------------------------

/// Which paths were changed in the same commits as one path — an **observation**, never a
/// dependency.
///
/// Two files changing together is equally consistent with coupling, with a formatting sweep, with a
/// release-version bump, and with one commit that did two unrelated things. The count is a raw
/// shared-commit count rather than a normalised affinity, because a normalised number invites
/// exactly the comparison the label forbids; the field is `cochange_observations`; no relation is
/// emitted and no assertion is written.
///
/// The disclaimer is [`nerve_store::COCHANGE_IS_NOT_A_DEPENDENCY`], **taken from the store rather
/// than written here**. A paraphrase on this surface would be a second copy of the one sentence
/// that stops a shared-commit count reading as a dependency, and the paraphrase is where it softens.
pub fn cochange(ctx: &Context<'_>, target: &Target) -> Answer {
    let path = tree_path(target, "path")?;
    let read = read(ctx)?;
    let limit = target
        .bounded(
            "limit",
            DEFAULT_HISTORY_COCHANGE_LIMIT,
            MAX_HISTORY_COCHANGE_LIMIT,
        )
        .map_err(ApiError::bad_request)?;
    let report = nerve_store::cochange(ctx.conn, &read.repo_id, Some(path), limit)
        .map_err(ApiError::internal)?;

    let truncation = Truncation {
        returned: report.rows.len(),
        total: Some(report.pairs_total),
        truncated: report.truncated,
        limit: report.limit,
    };
    let mut value = block(
        &read,
        json!({ "path": path, "path_is_as_recorded_in_a_tree": true }),
        if read.ingest.is_some() {
            "cochange"
        } else {
            "no_history_ingested"
        },
        Some(&truncation),
        continuation(None, None),
    );
    value["path"] = json!(path);
    value["pairs_total"] = json!(report.pairs_total);
    value["disclaimer"] = json!(report.disclaimer);
    value["rows"] = json!(report
        .rows
        .iter()
        .map(|row| json!({
            "path_a": row.path_a,
            "path_b": row.path_b,
            // Named for what was observed, never for what it might imply.
            "cochange_observations": row.cochange_observations,
        }))
        .collect::<Vec<_>>());
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceilings_are_the_documented_contract() {
        assert_eq!(MAX_HISTORY_COMMIT_LIMIT, 500);
        assert_eq!(MAX_HISTORY_PATH_LIMIT, 500);
        assert_eq!(MAX_HISTORY_FREQUENCY_LIMIT, 500);
        assert_eq!(MAX_HISTORY_COCHANGE_LIMIT, 500);
        assert_eq!(MAX_HISTORY_DIFF_COMMITS, 500);
        assert_eq!(MAX_HISTORY_DIFF_WALK, 5_000);
        assert_eq!(MAX_HISTORY_DIFF_CHANGES, 5_000);
        for (default, ceiling) in [
            (DEFAULT_HISTORY_COMMIT_LIMIT, MAX_HISTORY_COMMIT_LIMIT),
            (DEFAULT_HISTORY_CHANGE_LIMIT, MAX_HISTORY_CHANGE_LIMIT),
            (DEFAULT_HISTORY_PATH_LIMIT, MAX_HISTORY_PATH_LIMIT),
            (DEFAULT_HISTORY_FREQUENCY_LIMIT, MAX_HISTORY_FREQUENCY_LIMIT),
            (DEFAULT_HISTORY_COCHANGE_LIMIT, MAX_HISTORY_COCHANGE_LIMIT),
        ] {
            assert!(default <= ceiling, "{default} > {ceiling}");
        }
    }

    #[test]
    fn a_symbol_shaped_path_is_refused_and_nothing_is_guessed() {
        for raw in [
            "path=src%2Fapp.ts%23Circle.area",
            "path=function%3Aparse",
            "path=symbol%3Aparse",
        ] {
            let target = Target::parse(&format!("/api/history/path?{raw}")).unwrap();
            let error = tree_path(&target, "path").unwrap_err();
            assert_eq!(error.status, 400, "{raw}");
            assert_eq!(error.code, "refused_history_path", "{raw}");
            assert_eq!(error.detail["path_guessed"], false);
            // The containing path must appear nowhere in the refusal: answering with it is the
            // failure this gate exists to prevent, and echoing it is one edit away from doing so.
            assert!(
                !error.detail["reason_statement"]
                    .as_str()
                    .unwrap()
                    .contains("src/app.ts"),
                "{raw}"
            );
        }
        // And a path that merely holds a colon below the root stays a path.
        let target = Target::parse("/api/history/path?path=docs%2Fa%3Ab.md").unwrap();
        assert_eq!(tree_path(&target, "path").unwrap(), "docs/a:b.md");
    }

    #[test]
    fn a_continuation_is_an_offset_the_query_honours_or_a_statement() {
        let offered = continuation(Some(10), Some(20));
        assert_eq!(offered["supported"], true);
        assert_eq!(offered["next_offset"], 20);
        assert_eq!(offered["statement"], Value::Null);

        let declined = continuation(None, None);
        assert_eq!(declined["supported"], false);
        assert_eq!(declined["offset"], Value::Null);
        assert_eq!(declined["statement"], CONTINUATION_NOT_OFFERED);
    }

    #[test]
    fn truncation_is_reported_rather_than_inferred_from_the_page_length() {
        // The case `len() == limit` gets wrong in both directions.
        let exact = Truncation {
            returned: 5,
            total: Some(5),
            truncated: false,
            limit: 5,
        };
        assert_eq!(exact.value()["truncated"], false);
        assert_eq!(exact.value()["total"], 5);
        let cut = Truncation {
            returned: 5,
            total: Some(9),
            truncated: true,
            limit: 5,
        };
        assert_eq!(cut.value()["truncated"], true);
    }
}
