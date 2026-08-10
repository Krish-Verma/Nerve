//! The read-only cross-repository contract API.
//!
//! Three endpoints over the two tables Slice 13a writes and the resolver Slices 13b and 13c built.
//! **Not one of them decides anything.** Availability is
//! [`nerve_index::availability_of`]'s verdict, freshness is [`nerve_index::link_freshness`]'s, and
//! both arrive already decided on [`nerve_index::ContractLinkView`] —
//! `crates/nerve-cli/tests/registry_guards.rs::no_surface_derives_registry_availability_of_its_own`
//! scans this crate for the shapes a second derivation would have to be written in, and a second
//! derivation of *"is this neighbour readable?"* is a second **answer**, not a second phrasing.
//!
//! Every sentence in an answer is a vocabulary's own `note()` or `statement()`, fetched through
//! [`nerve_index::contract_vocabulary`]. Nothing here spells one.
//!
//! # What every answer carries
//!
//! One block, assembled in exactly one function — [`block`] — so no endpoint can answer without
//! saying what the answer rests on:
//!
//! ```text
//! repository_id · source_state · result_kind
//! registry_entries_total · links_total · links_without_registry_entry
//! truncation · continuation · boundary · limitations
//! ```
//!
//! # Bounds
//!
//! Every list is bounded and **truncation is a comparison against a counted total**, never
//! `len() == limit` — that guess is false whenever an answer ends exactly on the boundary, which is
//! the case a caller most needs to be right. Both list endpoints do offer a real `offset`, and that
//! is a difference from `/api/history*` rather than a departure from it: the store hands back the
//! **complete** ordered list here — `contract_link` is capped at
//! [`nerve_index::contracts::MAX_LINKS_PER_REPOSITORY`] rows when it is written — so a window over
//! it is the next page exactly, where paging a query that was already bounded would have re-run the
//! bound and returned a different set.
//!
//! # Repository text
//!
//! A display name, a contract identity, a version string, a manifest path and every target snapshot
//! field are repository prose: attacker-influencable wherever contributions are accepted, and never
//! interpreted. They are carried as JSON **string values**, exactly as `shapes::entity` carries an
//! entity name, and never as an object key, a vocabulary field or a code.
//!
//! # This surface is read-only, and registration is not a missing button
//!
//! `nerve repo add`, `nerve repo relocate`, `nerve repo remove` and `nerve repo scan` all write.
//! They are therefore command-line verbs, and this API answers with the exact command rather than
//! offering a route that would have to mutate to be useful. [`BOUNDARY`] is that statement, said
//! once and carried on every answer.

use serde_json::{json, Value};

use nerve_index::{ContractLinkView, ContractReport, ContractTerm};

use super::{Answer, ApiError, Context};
use crate::request::Target;

/// Largest page of contract links one request may ask for.
pub const MAX_CONTRACT_LINK_LIMIT: usize = 500;
/// Contract links returned when no limit is given.
pub const DEFAULT_CONTRACT_LINK_LIMIT: usize = 100;
/// Largest page of registry entries one request may ask for.
pub const MAX_REGISTRY_LIMIT: usize = 500;
/// Registry entries returned when no limit is given.
pub const DEFAULT_REGISTRY_LIMIT: usize = 100;

/// Why the vocabulary endpoint offers no continuation.
///
/// Not the same statement as a bounded list's: this answer is a compile-time constant of this
/// build, returned whole. There is no page after it, rather than a page this endpoint declines to
/// assemble.
pub const VOCABULARY_IS_COMPLETE: &str =
    "this answer is every value of every closed contract vocabulary in this build, returned in \
     full. It is a build constant rather than a page of repository data, so nothing was cut and \
     there is no continuation to offer";

/// The one statement about where a registry is changed, said once and carried everywhere.
///
/// A disabled button would imply an implementation is pending. Nothing is pending: `nerve serve`
/// is read-only and is proven so on the database bytes, so registering a neighbour and scanning
/// this repository's manifests are commands rather than controls — and the exact command is more
/// useful than a control that cannot exist.
pub const BOUNDARY: &str =
    "registering a neighbour, re-pointing one, retiring one and re-reading this repository's \
     manifests all write to this index. This API is read-only and every route on it is a GET, so \
     each of those is a command you run rather than a control on a page. Nothing is pending: a \
     button here would imply an implementation that is deliberately absent";

/// The commands that change what these endpoints report, in the order a user meets them.
pub const BOUNDARY_COMMANDS: [&str; 5] = [
    "nerve repo add <path> --id <name>",
    "nerve repo list",
    "nerve repo relocate <id> <path>",
    "nerve repo remove <id>",
    "nerve repo scan",
];

/// Why a link is one-sided, which is a property of the model rather than of this answer.
pub const LINK_IS_ONE_SIDED: &str =
    "a contract link is this repository's stated view of a neighbour, recorded in this \
     repository's index. The neighbour does not know it is depended upon until it registers this \
     repository in turn, and nothing here writes into a repository that was named only as a \
     dependency";

/// Why two recorded versions never produce a verdict.
pub const NO_VERSION_VERDICT: &str =
    "the version this repository expects and the version the neighbour declares are both recorded \
     and neither is compared. Deciding whether 1.2.3 satisfies ^1.2.0 is range resolution, which \
     needs a semantic-version resolver this product does not have and will not invent, so the \
     evidence is stored and no verdict is derived from it";

/// Why an ordinary graph query cannot reach any of this.
pub const NOT_IN_THE_GRAPH: &str =
    "no contract link is a row in the assertion graph, and no path or impact query traverses one. \
     Crossing into another repository is opt-in at this surface, never a side effect of an \
     ordinary traversal, because an answer about this repository must not be assembled from facts \
     whose freshness this repository cannot vouch for";

/// A bounded list's truncation, as a fact.
///
/// The same shape `/api/history*` uses, and for the same reason: `total` is what the store handed
/// over before the window was taken, so `truncated` is a comparison rather than the guess
/// `len() == limit`, which is false whenever a page ends exactly on the boundary.
struct Truncation {
    returned: usize,
    total: usize,
    truncated: bool,
    limit: usize,
    offset: usize,
}

impl Truncation {
    /// Window a complete ordered list, and report the cut as a comparison.
    fn window<T>(rows: Vec<T>, limit: usize, offset: usize) -> (Vec<T>, Truncation) {
        let total = rows.len();
        let kept: Vec<T> = rows.into_iter().skip(offset).take(limit).collect();
        let truncation = Truncation {
            returned: kept.len(),
            total,
            truncated: offset + kept.len() < total,
            limit,
            offset,
        };
        (kept, truncation)
    }

    fn next_offset(&self) -> Option<usize> {
        self.truncated.then_some(self.offset + self.returned)
    }

    fn value(&self) -> Value {
        json!({
            "returned": self.returned,
            "total": self.total,
            "truncated": self.truncated,
            "limit": self.limit,
        })
    }
}

/// A continuation the endpoint honours, or the statement that there is none.
fn continuation(offset: Option<usize>, next: Option<usize>, statement: Option<&str>) -> Value {
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
            "statement": statement,
        }),
    }
}

/// Read the registry and every stored link, or refuse with the reason.
///
/// A missing repository row is **not** an empty registry: it means this index has never recorded
/// which repository it describes, so there is nothing to key a neighbour on. Answering it as "no
/// neighbours" would report "we never looked" as "we looked and there are none".
fn read(ctx: &Context<'_>) -> Result<ContractReport, ApiError> {
    let Some(repo_id) = ctx.repo_id else {
        return Err(ApiError::with_detail(
            409,
            "repository_unknown",
            "this index records no repository, so no neighbour can be keyed to one",
            json!({ "registry_entries": Value::Null }),
        ));
    };
    nerve_index::contract_report(ctx.conn, repo_id, ctx.prober.root()).map_err(ApiError::internal)
}

/// The block every contract answer carries, assembled in **one** place.
fn block(
    ctx: &Context<'_>,
    report: &ContractReport,
    result_kind: &str,
    truncation: Option<&Truncation>,
    continuation: Value,
) -> Value {
    json!({
        "repository_id": ctx.repo_id,
        "source_state": report.source_state,
        "result_kind": result_kind,
        "registry_entries_total": report.entries.len(),
        "links_total": report.links.len(),
        // Structurally zero, and reported rather than assumed: a link dropped from an answer could
        // not be reported as having been dropped.
        "links_without_registry_entry": report.links_without_registry_entry,
        "truncation": truncation.map(Truncation::value),
        "continuation": continuation,
        "boundary": boundary(),
        "limitations": {
            "link_is_directional_and_one_sided": LINK_IS_ONE_SIDED,
            "contract_version_verdict_is_not_derived": NO_VERSION_VERDICT,
            "no_link_is_reachable_from_a_local_graph_query": NOT_IN_THE_GRAPH,
        },
    })
}

/// Where a registry is changed, and how.
fn boundary() -> Value {
    json!({
        "read_only": true,
        "statement": BOUNDARY,
        "commands": BOUNDARY_COMMANDS,
    })
}

/// One closed-vocabulary member, with whatever it owns beside its name.
fn term(term: &ContractTerm) -> Value {
    json!({ "name": term.name, "note": term.note, "rule": term.rule })
}

fn terms(values: &[ContractTerm]) -> Value {
    json!(values.iter().map(term).collect::<Vec<_>>())
}

/// One registry entry and what it is right now.
///
/// `local_path` is the one user-specific absolute value in this schema. It is served here because
/// this API is loopback-only, token-gated and describes the machine it is running on — the property
/// `crates/nerve-cli/tests/registry_guards.rs` enforces is that it is never *tracked by Git*, which
/// a response body is not.
fn entry(
    entry: &nerve_store::RegistryEntryRow,
    availability: &nerve_index::RegistryAvailability,
) -> Value {
    json!({
        "registry_id": entry.registry_id,
        "expected_repository_id": entry.expected_repository_id,
        "display_name": entry.display_name,
        "local_path": entry.local_path,
        "added_at": entry.added_at,
        "status": entry.status.as_str(),
        "status_note": entry.status.note(),
        "withdrawn_at": entry.withdrawn_at,
        "last_seen_state": entry.last_seen_state,
        "last_seen_at": entry.last_seen_at,
        "availability_checked_at": entry.availability_checked_at,
        // Carried, never re-derived. This is `nerve_index::availability_of`'s verdict, and it is
        // the only one in the product.
        "availability": availability.as_str(),
        "availability_statement": availability.statement(),
        "refusal": availability.refusal().map(|reason| reason.as_str()),
        "refusal_statement": availability.refusal().map(|reason| reason.statement()),
        "observed_repository_id": availability.observed_repository_id(),
        "usable": availability.is_usable(),
        // The qualification this entry puts on everything resolved through it. `null` means the
        // entry puts none, which is not the same as an entry nothing was concluded about — the
        // verdict beside it is what tells those apart, which is why both are carried.
        "freshness": availability.freshness().map(|state| state.as_str()),
        "freshness_note": availability.freshness().map(|state| state.note()),
    })
}

/// One stored link, with every column a consumer needs to tell current from moved.
fn link(view: &ContractLinkView) -> Value {
    let link = &view.link;
    json!({
        "link_id": link.link_id,
        // What the declaration says, in the words the manifest used.
        "relation_semantics": link.relation_semantics,
        "contract_kind": link.contract_kind,
        "contract_identity": link.contract_identity,
        "resolution_method": link.resolution_method.as_str(),
        "resolution_method_note": link.resolution_method.note(),
        // Both, and never a verdict. `NO_VERSION_VERDICT` on the block says why.
        "expected_contract_version": link.expected_contract_version,
        "observed_contract_version": link.observed_contract_version,
        // The local end. Every one of these is a fact about this database.
        "source_repository_id": link.source_repository_id,
        "source_state_at_resolution": link.source_state_at_resolution,
        "source_entity_id": link.source_entity_id,
        "source_kind_snapshot": link.source_kind_snapshot,
        "source_path": link.source_path,
        "source_span": link.source_span,
        "source_manifest_present": view.source_manifest_present,
        // The far end, and it is a **snapshot** rather than a pointer: the target may move, change
        // kind or vanish, and without these `contract_deleted`, `target_changed` and
        // `contract_file_missing` would be indistinguishable.
        "expected_target_repository_id": link.expected_target_repository_id,
        "target_state_at_resolution": link.target_state_at_resolution,
        "target_current_state": view.target_current_state,
        "target_entity_id": link.target_entity_id,
        "target_kind_snapshot": link.target_kind_snapshot,
        "target_name_snapshot": link.target_name_snapshot,
        "target_path_snapshot": link.target_path_snapshot,
        "target_span_snapshot": link.target_span_snapshot,
        // Who wrote the row, and what it could not resolve.
        "extractor_id": link.extractor_id,
        "extractor_version": link.extractor_version,
        "evidence_details": link.evidence_details,
        "ambiguity": link.ambiguity,
        "unsupported_reason": link.unsupported_reason,
        // Lifecycle. A withdrawn link is kept so that the ending can be reported at all.
        "status": link.status.as_str(),
        "status_note": link.status.note(),
        "first_seen_at": link.first_seen_at,
        "last_seen_at": link.last_seen_at,
        "withdrawn_at": link.withdrawn_at,
        // The verdict, carried off `nerve_index::link_freshness` and never re-derived here.
        // `null` is *no qualification*: the entry is available, both states still match, and the
        // manifest is still there. `is_current` states that rather than leaving it to be inferred
        // from a null, because a client reading an absent field as "unknown" would have it exactly
        // backwards.
        "freshness": view.freshness.map(|state| state.as_str()),
        "freshness_note": view.freshness.map(|state| state.note()),
        "is_current": view.freshness.is_none(),
        // The entry it came through, in full. A link without it is a claim about a repository the
        // reader cannot identify.
        "registry_entry": entry(&view.entry, &view.availability),
    })
}

// ---- /api/contracts ---------------------------------------------------------------------------

/// Every recorded cross-repository link, bounded, each with its entry and its standing.
pub fn links(ctx: &Context<'_>, target: &Target) -> Answer {
    let report = read(ctx)?;
    let limit = target
        .bounded(
            "limit",
            DEFAULT_CONTRACT_LINK_LIMIT,
            MAX_CONTRACT_LINK_LIMIT,
        )
        .map_err(ApiError::bad_request)?;
    let offset = target
        .bounded_from_zero("offset", 0, usize::MAX)
        .map_err(ApiError::bad_request)?;

    // One optional filter, and it is an exact registry id rather than a search: a link belongs to
    // exactly one entry, so naming the entry is the only narrowing this endpoint can do without
    // inventing a match rule.
    let wanted = target.get("registry_id");
    let selected: Vec<&ContractLinkView> = report
        .links
        .iter()
        .filter(|view| wanted.is_none_or(|id| view.link.registry_entry_id == id))
        .collect();
    let matched = selected.len();
    let (page, truncation) = Truncation::window(selected, limit, offset);

    let mut value = block(
        ctx,
        &report,
        if report.links.is_empty() {
            "no_contract_links"
        } else {
            "contract_links"
        },
        Some(&truncation),
        continuation(Some(offset), truncation.next_offset(), None),
    );
    value["registry_id"] = json!(wanted);
    value["links_matching_filter"] = json!(matched);
    value["links"] = json!(page.into_iter().map(link).collect::<Vec<_>>());
    Ok(value)
}

// ---- /api/contracts/registry ------------------------------------------------------------------

/// Every registered neighbour and what it is right now — tombstones included.
///
/// A tombstoned entry is listed. That is not a convenience: `registry_entry_removed` is a report
/// made from the kept row, and a list that hid it would make the state unreportable at exactly the
/// moment it becomes the answer.
pub fn registry(ctx: &Context<'_>, target: &Target) -> Answer {
    let report = read(ctx)?;
    let limit = target
        .bounded("limit", DEFAULT_REGISTRY_LIMIT, MAX_REGISTRY_LIMIT)
        .map_err(ApiError::bad_request)?;
    let offset = target
        .bounded_from_zero("offset", 0, usize::MAX)
        .map_err(ApiError::bad_request)?;

    let rows: Vec<&_> = report.entries.iter().collect();
    let (page, truncation) = Truncation::window(rows, limit, offset);

    // How many links each entry carries, so a reader can tell an entry nothing rests on from one
    // that every link in the repository resolves through. Counted over the whole report rather than
    // over the page, which is why it is exact whatever the window cut.
    let counted: Vec<Value> = page
        .into_iter()
        .map(|view| {
            let mut object = entry(&view.entry, &view.availability);
            let links = report
                .links
                .iter()
                .filter(|link| link.link.registry_entry_id == view.entry.registry_id)
                .count();
            object["links_through_this_entry"] = json!(links);
            object
        })
        .collect();

    let mut value = block(
        ctx,
        &report,
        if report.entries.is_empty() {
            "no_registered_neighbours"
        } else {
            "registry"
        },
        Some(&truncation),
        continuation(Some(offset), truncation.next_offset(), None),
    );
    // Absence with a reason. No sibling directory is ever registered on its own: a neighbour exists
    // in this table because somebody named it, which is the "directory proximity" link the row's
    // plan refuses, one layer down.
    value["nothing_is_auto_registered"] = json!(true);
    value["entries"] = json!(counted);
    Ok(value)
}

// ---- /api/contracts/vocabulary ----------------------------------------------------------------

/// Every closed contract vocabulary this build knows, with each value's own sentence.
///
/// A route of its own rather than a field on the two above, for two reasons. It is a **build
/// constant** — nothing in it depends on the repository — so carrying it on every links answer
/// would spend a hundred strings per request to say the same thing. And §9.1 requires an
/// unsupported form to be reported *with its form named*: a surface that shows a tally has to be
/// able to name the forms that scored zero as well as the ones that did not, and the set of names
/// has to come from the service that owns them rather than from a list typed into a view.
pub fn vocabulary(ctx: &Context<'_>) -> Answer {
    let report = read(ctx)?;
    let vocabulary = nerve_index::contract_vocabulary();
    let mut value = block(
        ctx,
        &report,
        "vocabulary",
        None,
        continuation(None, None, Some(VOCABULARY_IS_COMPLETE)),
    );
    value["vocabulary"] = json!({
        "rules": terms(&vocabulary.rules),
        "resolution_methods": terms(&vocabulary.resolution_methods),
        "link_statuses": terms(&vocabulary.link_statuses),
        "registry_entry_statuses": terms(&vocabulary.registry_entry_statuses),
        "freshness": terms(&vocabulary.freshness),
        "availability": terms(&vocabulary.availability),
        "registry_refusals": terms(&vocabulary.registry_refusals),
        "ambiguity": terms(&vocabulary.ambiguity),
        "supported_forms": terms(&vocabulary.supported_forms),
        "unsupported_forms": terms(&vocabulary.unsupported_forms),
        "unresolved_reasons": terms(&vocabulary.unresolved_reasons),
        "manifest_refusals": terms(&vocabulary.manifest_refusals),
        "scan_refusals": terms(&vocabulary.scan_refusals),
    });
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceilings_are_the_documented_contract() {
        assert_eq!(MAX_CONTRACT_LINK_LIMIT, 500);
        assert_eq!(MAX_REGISTRY_LIMIT, 500);
        for (default, ceiling) in [
            (DEFAULT_CONTRACT_LINK_LIMIT, MAX_CONTRACT_LINK_LIMIT),
            (DEFAULT_REGISTRY_LIMIT, MAX_REGISTRY_LIMIT),
        ] {
            assert!(default <= ceiling, "{default} > {ceiling}");
        }
    }

    /// The case `len() == limit` gets wrong in both directions.
    #[test]
    fn truncation_is_a_comparison_against_the_whole_list_rather_than_the_page_length() {
        let (page, exact) = Truncation::window(vec![1, 2, 3, 4, 5], 5, 0);
        assert_eq!(page.len(), 5);
        assert_eq!(exact.value()["truncated"], false);
        assert_eq!(exact.value()["total"], 5);
        assert_eq!(exact.next_offset(), None);

        let (page, cut) = Truncation::window(vec![1, 2, 3, 4, 5, 6, 7, 8, 9], 5, 0);
        assert_eq!(page, vec![1, 2, 3, 4, 5]);
        assert_eq!(cut.value()["truncated"], true);
        assert_eq!(cut.next_offset(), Some(5));

        // A window past the end is an ordinary answer, not a cut.
        let (page, past) = Truncation::window(vec![1, 2, 3], 5, 10);
        assert!(page.is_empty());
        assert_eq!(past.value()["truncated"], false);
        assert_eq!(past.value()["total"], 3);
    }

    #[test]
    fn a_continuation_is_an_offset_the_endpoint_honours_or_a_statement() {
        let offered = continuation(Some(10), Some(20), None);
        assert_eq!(offered["supported"], true);
        assert_eq!(offered["next_offset"], 20);
        assert_eq!(offered["statement"], Value::Null);

        let declined = continuation(None, None, Some(VOCABULARY_IS_COMPLETE));
        assert_eq!(declined["supported"], false);
        assert_eq!(declined["statement"], VOCABULARY_IS_COMPLETE);
    }

    /// Every command the boundary names is a `nerve repo` verb, and none of them is a route.
    #[test]
    fn the_boundary_names_commands_rather_than_offering_a_control() {
        assert_eq!(boundary()["read_only"], true);
        for command in BOUNDARY_COMMANDS {
            assert!(command.starts_with("nerve repo "), "{command}");
        }
        assert!(BOUNDARY.contains("read-only"));
    }
}
