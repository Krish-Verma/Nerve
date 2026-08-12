//! *Can I trust this index right now?* — the one question that has to be answered before any
//! other answer on this server means anything.
//!
//! `nerve check` has answered it since Slice 7c-i and only a shell could ask. Every other surface
//! could return a confident, well-shaped, thoroughly-qualified answer drawn from a graph built
//! against a tree that has since moved on, and nothing on the wire said so. This route closes that:
//! the same five-valued verdict, over the same two measurements, reachable by an agent before it
//! asks anything else.
//!
//! **Nothing is judged here.** The verdict, the reason and the counts are
//! [`nerve_index::trust`]'s, which is what `nerve check` calls, so the command line and this route
//! cannot answer differently (ARCHITECTURE.md invariant 3). What this file adds is the JSON.
//!
//! # A stale index is a successful answer to the question that was asked
//!
//! **The HTTP status never encodes the verdict.** Every one of the five is `200`.
//!
//! The temptation is to return a 4xx for `stale` or `no_index`, and it is wrong in a way that
//! destroys the endpoint's value. A `4xx` says *your request was wrong*. "This index is stale" is
//! not a wrong request — it is the correct, complete, hard-won answer to exactly the question the
//! caller asked, and it is the answer they most need to be able to read. Encoding it as a failure
//! would make it indistinguishable from a bad token, a malformed target, or a route that does not
//! exist, and a client with a blanket `if (!response.ok) throw` — which is most clients — would
//! turn the most important answer this server can give into an exception with no verdict in it.
//!
//! The verdict is the payload. A refusal here is reserved for what a refusal means everywhere else
//! on this surface: the request could not be served at all.
//!
//! # `unverified` is not `stale`, on this surface as on every other
//!
//! Both mean *do not rely on this index*, and they are two values because the evidence is
//! different: one is a measurement of divergence, the other is the absence of a measurement. The
//! CLI gives them one exit code because a shell has one way to say "do not proceed"; a JSON body
//! has room to say which, so it does. `evidence` splits the counts into the two families under
//! their own keys, and neither family's tally is ever folded into the other's.
//!
//! # The walk is a read
//!
//! Answering costs a re-hash of every indexed file and a discovery walk of the tree — both reads,
//! neither a write. The connection is `query_only` like every other route here, and
//! `scripts/final_acceptance.sh` §4i proves it on the database bytes across a session that calls
//! this route rather than on the absence of a write statement.

use serde_json::{json, Value};

use nerve_index::{TrustReport, Verdict};

use super::{Answer, ApiError, Context};

/// Largest number of added paths one answer names.
///
/// The count is exact whatever this cuts. A repository that was initialized and never indexed has
/// every file untracked, so the list is repository-sized and the tally is not — bounding the list
/// while reporting the true total is the same trade `/api/gaps` makes, and the alternative is an
/// answer whose size is the size of the thing it is describing.
pub const MAX_ADDED_PATHS: usize = 200;

/// The one statement about what a `200` means here, said once and carried on every answer.
pub const STATUS_IS_NOT_THE_VERDICT: &str =
    "every verdict is served with HTTP 200, including the four that say this index cannot be \
     relied on. The status describes the request; the verdict describes the index. A 4xx for a \
     stale index would make `the index is stale` indistinguishable from `your request was wrong`, \
     and a client that branches on the status alone would throw away the answer it asked for. \
     Branch on `verdict`";

/// Why the two unreliable-for-different-reasons verdicts are two values.
pub const STALE_IS_NOT_UNVERIFIED: &str =
    "`stale` and `unverified` both mean do not rely on this index, and they are not the same \
     finding. `stale` is a measurement: a file changed, a file the index describes is gone, or a \
     file exists that no row describes. `unverified` is the absence of a measurement: part of the \
     tree was never looked at, because the sweep reached its cap or a path could not be read. \
     `nerve check` gives them one exit code because a shell has one way to say do not proceed; \
     that is a property of exit codes and not of the evidence, and this answer keeps them apart";

/// What a bounded sweep does and does not establish.
pub const SWEEP_IS_BOUNDED: &str =
    "the sweep re-hashes at most `sweep.probe_cap` indexed files. When the cap bites, \
     `sweep.truncated` is true and the verdict is `unverified` rather than `current` — a partial \
     sweep reported as clean would be a clean bill of health issued without looking, which is the \
     failure this endpoint exists to prevent";

/// Why an added file needs a second measurement at all.
pub const ADDED_IS_A_SEPARATE_MEASUREMENT: &str =
    "the sweep compares the files the index has a row for, so a file added since the last \
     index has nothing to compare and is invisible to it: a repository can grow a hundred modules \
     with every recorded hash still matching. `tree.added` comes from walking the repository and \
     subtracting what the index knows, which is the only measurement that can see one";

/// Why an unreadable new file is counted apart from an added one.
pub const UNINDEXABLE_IS_NOT_ADDED: &str =
    "`tree.unindexable` counts files the tree has that the indexer could not have read either — \
     over the size ceiling, unreadable, or not UTF-8. They are counted rather than reported as \
     additions because re-indexing would not produce a row for them, so calling them additions \
     would make this answer say `stale` forever with nothing its reader could do about it";

/// What the verdict is, and is not, in time.
pub const VERDICT_IS_A_MOMENT: &str =
    "this verdict was measured while answering this request, by reading the repository again \
     rather than from a cache. It describes the tree as it was at that moment and nothing keeps it \
     true afterwards: a file saved a second later makes a `current` answer stale, and this server \
     has no way to know";

/// The one statement about where the remedy is run.
pub const BOUNDARY: &str =
    "re-indexing writes to this index. This API is read-only and every route on it is a GET, so \
     bringing the index back up to date is a command you run rather than a control on a page. \
     Nothing is pending: a button here would imply an implementation that is deliberately absent";

/// What a caller runs to make a non-`current` verdict `current`.
///
/// One command, because there is one. `nerve index` migrates a schema that is behind, finishes an
/// index that did not, re-extracts a file that changed, drops a file that is gone and adds a file
/// that is new — every verdict below `current` has the same remedy, and offering a menu would
/// imply the caller has to work out which of them applies.
pub const REMEDY_COMMAND: &str = "nerve index";

/// The command that produces this same verdict at a shell, named so the two are known to be one.
pub const VERDICT_COMMAND: &str = "nerve check";

// ---- /api/check --------------------------------------------------------------------------------

/// Judge this index against the tree it describes, and answer with the verdict.
///
/// **Read-only, and `200` whatever the verdict.** See the module documentation: the status code
/// describes the request and the verdict describes the index, and collapsing the two would make
/// "your index is stale" arrive at a client as an exception.
///
/// The judgement is [`nerve_index::trust`]'s. This function chooses no verdict, computes no count
/// and re-derives nothing — including `trustworthy`, which is [`Verdict::is_current`] rather than a
/// second reading of the same enum.
pub fn verdict(ctx: &Context<'_>) -> Answer {
    // The root the sweep reads from is the prober's, which is the canonical root this session was
    // opened on. Taking it from anywhere else would be a second definition of the repository root.
    let report = nerve_index::trust(ctx.conn, ctx.prober.root(), nerve_index::TRUST_PROBE_CAP)
        .map_err(ApiError::internal)?;
    Ok(answer(ctx, &report))
}

/// The whole payload, assembled in one place so no branch can answer without its qualifications.
fn answer(ctx: &Context<'_>, report: &TrustReport) -> Value {
    json!({
        // Constant rather than varying with the verdict, and that is the same decision `nerve
        // check` made about its own output: every outcome takes one shape, so a script parses one
        // object for a clean index and for a stale one. A `result_kind` that tracked the verdict
        // would be a second copy of `verdict`, free to disagree with it.
        "result_kind": "trust_verdict",
        "verdict": report.verdict.as_str(),
        "verdict_note": report.verdict.note(),
        "reason": report.reason,
        // Read off the vocabulary, never from `verdict == "current"` spelled again here.
        "trustworthy": report.verdict.is_current(),
        "http_status": 200,
        "status_is_not_the_verdict": STATUS_IS_NOT_THE_VERDICT,
        "measured_at_this_request": true,
        "repository": repository(ctx),
        "schema": {
            "version": report.schema_version,
            "supported_version": nerve_store::SCHEMA_VERSION,
            "readable": report.schema_version == Some(nerve_store::SCHEMA_VERSION),
        },
        "runs_running": report.runs_running,
        // Present rather than implied: `sweep: null` and a sweep of zero files are different
        // answers, and a client that could not tell them apart would report an unjudged index as
        // an empty one.
        "swept": report.measured.is_some(),
        "sweep": sweep(report),
        "tree": tree(report),
        "evidence": evidence(report),
        "remedy": remedy(report),
        "boundary": boundary(),
        "vocabulary": vocabulary(),
        "limitations": {
            "stale_is_not_unverified": STALE_IS_NOT_UNVERIFIED,
            "sweep_is_bounded": SWEEP_IS_BOUNDED,
            "added_is_a_separate_measurement": ADDED_IS_A_SEPARATE_MEASUREMENT,
            "unindexable_is_not_added": UNINDEXABLE_IS_NOT_ADDED,
            "verdict_is_a_moment": VERDICT_IS_A_MOMENT,
        },
    })
}

/// Which repository this verdict is about, off `nerve-store`'s own status row.
fn repository(ctx: &Context<'_>) -> Value {
    let report = nerve_store::status(ctx.conn).ok();
    json!({
        "repository_id": ctx.repo_id,
        "project_id": report.as_ref().and_then(|report| report.project_id.clone()),
        "root_path": report.as_ref().and_then(|report| report.root_path.clone()),
        "state_id": report.as_ref().and_then(|report| report.state_id.clone()),
        "git_commit": report.as_ref().and_then(|report| report.git_commit.clone()),
        "database_bytes": nerve_store::database_bytes(ctx.db_path),
    })
}

/// The re-hash of every file the index has a row for. `null` when no sweep ran.
///
/// Null rather than a row of zeroes, for the reason `/api/gaps` reports a null `totals`: a zeroed
/// sweep is a measurement that came back clean, and no sweep at all is not a measurement.
fn sweep(report: &TrustReport) -> Value {
    let Some(measured) = &report.measured else {
        return Value::Null;
    };
    let freshness = &measured.freshness;
    json!({
        "files_total": freshness.files_total,
        "files_probed": freshness.files_probed,
        "fresh": freshness.fresh,
        "stale": freshness.stale,
        "missing": freshness.missing,
        "refused": freshness.refused,
        "unreadable": freshness.unreadable,
        "truncated": freshness.truncated,
        "probe_cap": report.probe_cap,
    })
}

/// What the repository holds that the index has never seen. `null` when no walk ran.
fn tree(report: &TrustReport) -> Value {
    let Some(measured) = &report.measured else {
        return Value::Null;
    };
    let untracked = &measured.untracked;
    let named: Vec<&String> = untracked.added.iter().take(MAX_ADDED_PATHS).collect();
    json!({
        // Exact whatever the list below was cut to.
        "added": untracked.added.len(),
        "added_paths": named,
        "added_paths_returned": named.len(),
        "added_paths_truncated": untracked.added.len() > named.len(),
        "added_paths_limit": MAX_ADDED_PATHS,
        "unindexable": untracked.unindexable,
    })
}

/// The two families of evidence, under their own keys, never summed together.
///
/// This is the block that makes `stale` and `unverified` readable as different findings rather
/// than as one number a client rounds to "bad". `observed` counts divergence that was measured;
/// `not_established` counts the tree that was never looked at.
fn evidence(report: &TrustReport) -> Value {
    let Some(measured) = &report.measured else {
        return json!({
            "observed": Value::Null,
            "not_established": Value::Null,
            "statement": "no sweep ran, so neither family has a tally. Null is not zero: nothing \
                          was measured, so nothing can be reported as diverged or as unchecked",
            "families_are_separate": STALE_IS_NOT_UNVERIFIED,
        });
    };
    let freshness = &measured.freshness;
    let added = measured.untracked.added.len();
    let observed = freshness.stale + freshness.missing + added;
    let unchecked = freshness.refused + freshness.unreadable;
    json!({
        "observed": {
            "changed": freshness.stale,
            "removed": freshness.missing,
            "added": added,
            "total": observed,
        },
        "not_established": {
            "refused": freshness.refused,
            "unreadable": freshness.unreadable,
            "never_probed": freshness.files_total.saturating_sub(freshness.files_probed),
            "truncated": freshness.truncated,
            "total": unchecked,
        },
        "statement": if observed > 0 {
            "divergence was observed. The index and the tree disagree about a file that was read"
        } else if freshness.truncated || unchecked > 0 {
            "nothing was observed to have changed, and part of the tree was never looked at. This \
             is the absence of a measurement rather than a finding of staleness"
        } else {
            "the whole of the index was compared against the tree and nothing diverged"
        },
        "families_are_separate": STALE_IS_NOT_UNVERIFIED,
    })
}

/// What to run, and whether there is anything to run.
///
/// `required` is `false` for exactly one verdict. Naming the command on a `current` answer as well
/// — with `required: false` — is deliberate: a client that only learned the command when something
/// was wrong would have to hold its own copy for the case where it wants to offer it anyway.
fn remedy(report: &TrustReport) -> Value {
    json!({
        "required": !report.verdict.is_current(),
        "command": REMEDY_COMMAND,
        "verdict_command": VERDICT_COMMAND,
        "statement": match report.verdict {
            Verdict::Current => "nothing needs running. This index describes the tree it was \
                                 built from.",
            Verdict::NoIndex => "there is nothing to judge yet. `nerve init` creates the database \
                                 and `nerve index` builds the graph.",
            Verdict::Unusable => "the index cannot be used as it stands. `nerve index` migrates a \
                                  schema that is behind and finishes an index that did not.",
            Verdict::Stale => "the tree has moved on. `nerve index` re-extracts what changed.",
            Verdict::Unverified => "part of the tree was never compared. `nerve index` re-extracts \
                                    the repository, after which the sweep has a full set of hashes \
                                    to compare against.",
        },
    })
}

/// Where the remedy is run, and why it is not a control here.
fn boundary() -> Value {
    json!({
        "read_only": true,
        "statement": BOUNDARY,
        "commands": [REMEDY_COMMAND, VERDICT_COMMAND],
    })
}

/// The closed set of verdicts, with what each means.
///
/// Generated from [`Verdict::ALL`] rather than typed here, so a sixth verdict is offered by this
/// answer the day it exists. Carried on every answer rather than served from a route of its own:
/// it is five short names, and a client rendering a verdict needs all five to render one it has
/// not met before as something other than a bare token.
fn vocabulary() -> Value {
    json!({
        "verdicts": Verdict::ALL
            .iter()
            .map(|verdict| json!({
                "verdict": verdict.as_str(),
                "note": verdict.note(),
                "trustworthy": verdict.is_current(),
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerve_index::{IndexFreshness, TrustMeasurement, UntrackedFiles};

    fn measured(freshness: IndexFreshness, untracked: UntrackedFiles) -> TrustReport {
        let (verdict, reason) = nerve_index::trust::judge_freshness(
            &freshness,
            untracked.added.len(),
            nerve_index::TRUST_PROBE_CAP,
        );
        TrustReport {
            verdict,
            reason,
            schema_version: Some(nerve_store::SCHEMA_VERSION),
            runs_running: 0,
            probe_cap: nerve_index::TRUST_PROBE_CAP,
            measured: Some(TrustMeasurement {
                freshness,
                untracked,
            }),
        }
    }

    fn swept(fresh: usize) -> IndexFreshness {
        IndexFreshness {
            files_total: fresh,
            files_probed: fresh,
            fresh,
            ..IndexFreshness::default()
        }
    }

    /// The distinction the whole route exists to carry, asserted on the payload rather than on the
    /// enum: a client reading only these two blocks must be able to tell the two apart.
    #[test]
    fn a_stale_answer_and_an_unverified_answer_are_different_documents() {
        let mut changed = swept(12);
        changed.fresh = 11;
        changed.stale = 1;
        let stale = measured(changed, UntrackedFiles::default());

        let mut refused = swept(12);
        refused.fresh = 11;
        refused.refused = 1;
        let unverified = measured(refused, UntrackedFiles::default());

        assert_eq!(stale.verdict, Verdict::Stale);
        assert_eq!(unverified.verdict, Verdict::Unverified);

        let one = evidence(&stale);
        let other = evidence(&unverified);
        assert_eq!(one["observed"]["total"], 1);
        assert_eq!(one["not_established"]["total"], 0);
        assert_eq!(other["observed"]["total"], 0);
        assert_eq!(other["not_established"]["total"], 1);
        assert_ne!(one["statement"], other["statement"]);
        // Neither family is ever folded into the other: the two totals are separate keys, and the
        // one that is zero is present rather than omitted.
        assert!(one["not_established"]["refused"].is_number());
        assert!(other["observed"]["changed"].is_number());
    }

    /// An added file is the case the tree walk exists for, and the sweep says nothing about it.
    #[test]
    fn an_added_file_is_reported_by_the_walk_while_the_sweep_reports_nothing_wrong() {
        let report = measured(
            swept(12),
            UntrackedFiles {
                added: vec!["src/brandnew.ts".to_string()],
                unindexable: 2,
            },
        );
        assert_eq!(report.verdict, Verdict::Stale);
        assert_eq!(sweep(&report)["stale"], 0);
        assert_eq!(sweep(&report)["fresh"], 12);
        assert_eq!(tree(&report)["added"], 1);
        assert_eq!(tree(&report)["added_paths"][0], "src/brandnew.ts");
        assert_eq!(tree(&report)["unindexable"], 2);
        assert_eq!(evidence(&report)["observed"]["added"], 1);
    }

    /// The list is cut and the tally is not.
    #[test]
    fn the_added_list_is_bounded_and_the_count_stays_exact() {
        let added: Vec<String> = (0..MAX_ADDED_PATHS * 2)
            .map(|index| format!("src/new{index}.ts"))
            .collect();
        let report = measured(
            swept(0),
            UntrackedFiles {
                added: added.clone(),
                unindexable: 0,
            },
        );
        let tree = tree(&report);
        assert_eq!(tree["added"], added.len());
        assert_eq!(tree["added_paths_returned"], MAX_ADDED_PATHS);
        assert_eq!(
            tree["added_paths"].as_array().unwrap().len(),
            MAX_ADDED_PATHS
        );
        assert_eq!(tree["added_paths_truncated"], true);
        assert_eq!(tree["added_paths_limit"], MAX_ADDED_PATHS);
    }

    /// No sweep is not an empty sweep.
    #[test]
    fn an_unjudged_index_reports_null_tallies_rather_than_zeroes() {
        let report = TrustReport {
            verdict: Verdict::NoIndex,
            reason: "nothing has been indexed".to_string(),
            schema_version: None,
            runs_running: 0,
            probe_cap: nerve_index::TRUST_PROBE_CAP,
            measured: None,
        };
        assert_eq!(sweep(&report), Value::Null);
        assert_eq!(tree(&report), Value::Null);
        let evidence = evidence(&report);
        assert_eq!(evidence["observed"], Value::Null);
        assert_eq!(evidence["not_established"], Value::Null);
        assert!(evidence["statement"]
            .as_str()
            .unwrap()
            .contains("Null is not zero"));
        assert_eq!(remedy(&report)["required"], true);
        assert!(remedy(&report)["statement"]
            .as_str()
            .unwrap()
            .contains("nerve init"));
    }

    /// One remedy, named on every answer, required on four of the five.
    #[test]
    fn the_remedy_is_named_on_every_verdict_and_required_on_the_four_that_are_not_current() {
        let mut required = 0;
        for verdict in Verdict::ALL {
            let report = TrustReport {
                verdict,
                reason: String::new(),
                schema_version: Some(nerve_store::SCHEMA_VERSION),
                runs_running: 0,
                probe_cap: nerve_index::TRUST_PROBE_CAP,
                measured: None,
            };
            let remedy = remedy(&report);
            assert_eq!(remedy["command"], REMEDY_COMMAND);
            assert!(!remedy["statement"].as_str().unwrap().is_empty());
            if remedy["required"] == json!(true) {
                required += 1;
            }
        }
        assert_eq!(required, 4, "exactly one verdict needs nothing run");
        assert_eq!(REMEDY_COMMAND, "nerve index");
    }

    /// The vocabulary is the vocabulary, not a list typed beside it.
    #[test]
    fn the_verdict_vocabulary_is_generated_and_complete() {
        let vocabulary = vocabulary();
        let verdicts = vocabulary["verdicts"].as_array().unwrap();
        assert_eq!(verdicts.len(), Verdict::ALL.len());
        let trustworthy: Vec<&Value> = verdicts
            .iter()
            .filter(|entry| entry["trustworthy"] == json!(true))
            .collect();
        assert_eq!(trustworthy.len(), 1);
        assert_eq!(trustworthy[0]["verdict"], "current");
        for entry in verdicts {
            assert!(!entry["note"].as_str().unwrap().is_empty());
        }
    }

    /// The statement that keeps a client from branching on the status code.
    #[test]
    fn the_answer_says_the_status_is_not_the_verdict() {
        assert!(STATUS_IS_NOT_THE_VERDICT.contains("200"));
        assert!(STATUS_IS_NOT_THE_VERDICT.contains("Branch on `verdict`"));
        assert!(STALE_IS_NOT_UNVERIFIED.contains("absence of a measurement"));
    }
}
