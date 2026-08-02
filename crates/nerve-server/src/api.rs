//! The read-only API.
//!
//! Every handler here does the same three things: validate and bound its arguments, ask
//! `nerve-store` (or `nerve-index`, for anything that touches the filesystem) the question, and
//! shape the answer. **No handler contains graph logic, SQL, traversal or path resolution.**
//! When an endpoint needed a query that did not exist, the query was added to `nerve-store`,
//! not written here (ARCHITECTURE.md invariant 3) — which is what keeps the CLI, this server
//! and the future MCP tools answering identically.
//!
//! Every argument is bounded. Depth, node budgets, result limits, byte ranges and line counts
//! all have ceilings enforced before the query runs, so no request can ask this process to do
//! unbounded work (THREAT-MODEL T8's discipline, applied early).

use std::path::Path;

use serde_json::{json, Value};

use nerve_core::vocab::{EntityKind, Relation};
use nerve_index::{RepositoryProber, SourceSnippet};
use nerve_store::{Connection, Direction, EntityRef, Selection, WhyDirection};

use crate::request::Target;
use crate::shapes;

/// Largest number of search results one request may ask for.
pub const MAX_SEARCH_LIMIT: usize = 200;
/// Largest neighbourhood depth. Beyond this a picture stops being a picture.
pub const MAX_NEIGHBOURHOOD_DEPTH: usize = 4;
/// Largest number of nodes one neighbourhood may admit.
pub const MAX_NEIGHBOURHOOD_NODES: usize = 500;
/// Largest `max_depth` a path search may use. Matches the CLI ceiling.
pub const MAX_PATH_DEPTH: usize = 32;
/// Largest number of distinct paths one request may ask for.
pub const MAX_PATH_LIMIT: usize = 25;
/// Largest page of unresolved entities.
pub const MAX_UNRESOLVED_LIMIT: usize = 500;
/// Largest page of coverage-gap rows. The tallies are exact whatever this cuts.
pub const MAX_GAPS_LIMIT: usize = 500;
/// How many files the overview will re-hash before it reports a partial sweep.
pub const FRESHNESS_PROBE_CAP: usize = 5_000;

/// A refusal, in the one shape every failure takes.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// HTTP status.
    pub status: u16,
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// Structured detail: candidates, suggestions, the refused value.
    pub detail: Value,
}

impl ApiError {
    /// A refusal with no structured detail.
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
            detail: json!({}),
        }
    }

    /// A refusal carrying structured detail.
    pub fn with_detail(
        status: u16,
        code: &'static str,
        message: impl Into<String>,
        detail: Value,
    ) -> ApiError {
        ApiError {
            detail,
            ..ApiError::new(status, code, message)
        }
    }

    fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(400, "bad_request", message)
    }

    fn internal(error: impl std::fmt::Display) -> ApiError {
        // The underlying message can quote a path or a SQL fragment. It is still ours, not the
        // repository's, so it is reported — an opaque 500 makes a local tool undebuggable.
        ApiError::new(500, "internal", error.to_string())
    }
}

/// What every handler receives.
pub struct Context<'a> {
    /// A read-only connection to the index.
    pub conn: &'a Connection,
    /// The repository reader, which owns every path-safety decision.
    pub prober: &'a RepositoryProber,
    /// Repository identifier, absent when the index has never been written.
    pub repo_id: Option<&'a str>,
    /// Database path, for the size report.
    pub db_path: &'a Path,
}

/// A handled request.
pub type Answer = Result<Value, ApiError>;

// ---- overview ------------------------------------------------------------------------------

/// Counts, unresolved totals, last run, schema version, and how much of it is still true.
pub fn overview(ctx: &Context<'_>) -> Answer {
    let report = nerve_store::status(ctx.conn).map_err(ApiError::internal)?;
    let freshness = match ctx.repo_id {
        Some(repo_id) => {
            let measured =
                nerve_index::index_freshness(ctx.conn, repo_id, ctx.prober, FRESHNESS_PROBE_CAP)
                    .map_err(ApiError::internal)?;
            json!({
                "files_total": measured.files_total,
                "files_probed": measured.files_probed,
                "fresh": measured.fresh,
                "stale": measured.stale,
                "missing": measured.missing,
                "refused": measured.refused,
                "unreadable": measured.unreadable,
                "truncated": measured.truncated,
            })
        }
        None => Value::Null,
    };

    Ok(json!({
        "schema_version": report.schema_version,
        "supported_schema_version": nerve_store::SCHEMA_VERSION,
        "healthy": report.is_healthy(),
        "project_id": report.project_id,
        "root_path": report.root_path,
        "state_id": report.state_id,
        "git_commit": report.git_commit,
        "database_bytes": nerve_store::database_bytes(ctx.db_path),
        "entities_total": report.entities_total,
        "symbols_total": report.symbols_total,
        "entities_by_kind": report.entities_by_kind,
        "assertions_total": report.assertions_total,
        "assertions_by_relation": report.assertions_by_relation,
        "occurrences_total": report.occurrences_total,
        "observations_total": report.observations_total,
        "assertion_states_total": report.assertion_states_total,
        "unresolved_entities": report.unresolved_entities,
        "unresolved_assertions": report.unresolved_assertions,
        "last_run": report.last_run.as_ref().map(run),
        "runs": report.runs.iter().map(run).collect::<Vec<_>>(),
        "freshness": freshness,
    }))
}

fn run(run: &nerve_store::ExtractorRunSummary) -> Value {
    json!({
        "run_id": run.run_id,
        "state_id": run.state_id,
        "extractor_id": run.extractor_id,
        "extractor_version": run.extractor_version,
        "started_at": run.started_at,
        "finished_at": run.finished_at,
        "files_processed": run.files_processed,
        "files_failed": run.files_failed,
        "status": run.status,
    })
}

// ---- search --------------------------------------------------------------------------------

/// FTS5 search over entity names and scope paths.
pub fn search(ctx: &Context<'_>, target: &Target) -> Answer {
    let query = target
        .get("q")
        .ok_or_else(|| ApiError::bad_request("q is required"))?;
    let kind = match target.get("kind") {
        None => None,
        Some(kind) => {
            if kind.parse::<EntityKind>().is_err() {
                return Err(ApiError::with_detail(
                    400,
                    "unknown_kind",
                    format!("unknown kind {kind:?}"),
                    json!({
                        "kind": kind,
                        "allowed": EntityKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
                    }),
                ));
            }
            Some(kind)
        }
    };
    let limit = target
        .bounded("limit", 20, MAX_SEARCH_LIMIT)
        .map_err(ApiError::bad_request)?;

    let hits =
        nerve_store::search_entities(ctx.conn, query, kind, limit).map_err(ApiError::internal)?;
    Ok(json!({
        "query": query,
        "kind": kind,
        "limit": limit,
        "count": hits.len(),
        "results": hits.iter().map(shapes::search_hit).collect::<Vec<_>>(),
    }))
}

// ---- entity --------------------------------------------------------------------------------

/// One entity: what it is, where it lives, and what defines or contains it.
pub fn entity(ctx: &Context<'_>, target: &Target) -> Answer {
    let subject = resolve(ctx, target, "selector")?;
    let occurrences =
        nerve_store::occurrences_of(ctx.conn, &subject.entity_id).map_err(ApiError::internal)?;
    let counts = nerve_store::entity_relation_counts(ctx.conn, &subject.entity_id)
        .map_err(ApiError::internal)?;

    // "What defines this" is a one-hop question over the structural relations, answered by the
    // same bounded expansion the graph view uses rather than by a second traversal.
    let structural = nerve_store::NeighbourhoodQuery {
        max_depth: 1,
        max_nodes: MAX_NEIGHBOURHOOD_NODES,
        direction: Direction::Any,
        relations: vec![Relation::Defines, Relation::Contains],
        resolved_only: false,
    };
    let defining = nerve_store::neighbourhood(ctx.conn, &subject.entity_id, &structural)
        .map_err(ApiError::internal)?;

    Ok(json!({
        "entity": shapes::entity(&subject),
        "occurrence_count": occurrences.len(),
        "occurrences": occurrences.iter().map(shapes::occurrence).collect::<Vec<_>>(),
        "relation_counts": {
            "outgoing": counts.outgoing,
            "incoming": counts.incoming,
        },
        "defining_edges": shapes::neighbourhood(&defining),
    }))
}

// ---- neighbourhood -------------------------------------------------------------------------

/// A bounded neighbourhood around one entity, with an explicit "not shown" count.
pub fn neighbourhood(ctx: &Context<'_>, target: &Target) -> Answer {
    let focus = resolve(ctx, target, "selector")?;
    let query = nerve_store::NeighbourhoodQuery {
        max_depth: target
            .bounded("depth", 1, MAX_NEIGHBOURHOOD_DEPTH)
            .map_err(ApiError::bad_request)?,
        max_nodes: target
            .bounded("max_nodes", 60, MAX_NEIGHBOURHOOD_NODES)
            .map_err(ApiError::bad_request)?,
        direction: direction(target)?,
        relations: relations(target)?,
        resolved_only: target.flag("resolved_only"),
    };
    let report = nerve_store::neighbourhood(ctx.conn, &focus.entity_id, &query)
        .map_err(ApiError::internal)?;
    let mut value = shapes::neighbourhood(&report);
    value["direction"] = json!(query.direction.as_str());
    value["relations"] = json!(relation_names(&query.relations));
    value["resolved_only"] = json!(query.resolved_only);
    Ok(value)
}

// ---- path ----------------------------------------------------------------------------------

/// How two entities are connected, bounded and honest about giving up.
pub fn path(ctx: &Context<'_>, target: &Target) -> Answer {
    let from = resolve(ctx, target, "from")?;
    let to = resolve(ctx, target, "to")?;
    let query = nerve_store::PathQuery {
        max_depth: target
            .bounded("max_depth", 6, MAX_PATH_DEPTH)
            .map_err(ApiError::bad_request)?,
        limit: target
            .bounded("limit", 3, MAX_PATH_LIMIT)
            .map_err(ApiError::bad_request)?,
        direction: direction(target)?,
        relations: relations(target)?,
        resolved_only: target.flag("resolved_only"),
    };
    let report = nerve_store::find_paths(ctx.conn, &from.entity_id, &to.entity_id, &query)
        .map_err(ApiError::internal)?;
    let mut value = shapes::path_report(&report);
    value["limit"] = json!(query.limit);
    value["direction"] = json!(query.direction.as_str());
    value["relations"] = json!(relation_names(&query.relations));
    value["resolved_only"] = json!(query.resolved_only);
    Ok(value)
}

// ---- why -----------------------------------------------------------------------------------

/// The evidence packet: every assertion around a subject, with observations and freshness.
pub fn why(ctx: &Context<'_>, target: &Target) -> Answer {
    let subject = resolve(ctx, target, "subject")?;
    let object = match target.get("object") {
        Some(_) => Some(resolve(ctx, target, "object")?),
        None => None,
    };
    let direction = match target.get("direction") {
        None | Some("both") => WhyDirection::Both,
        Some("outgoing") => WhyDirection::Outgoing,
        Some("incoming") => WhyDirection::Incoming,
        Some(other) => {
            return Err(ApiError::with_detail(
                400,
                "unknown_direction",
                format!("unknown direction {other:?}"),
                json!({ "direction": other, "allowed": ["both", "outgoing", "incoming"] }),
            ))
        }
    };
    let query = nerve_store::WhyQuery {
        direction,
        relations: relations(target)?,
    };
    // Freshness is computed by re-reading the repository through the prober, which enforces the
    // path rules on every path the database hands it.
    let report = nerve_store::explain(
        ctx.conn,
        &subject.entity_id,
        object.as_ref().map(|entity| entity.entity_id.as_str()),
        &query,
        ctx.prober,
    )
    .map_err(ApiError::internal)?;

    let mut value = shapes::why_report(&report);
    value["direction"] = json!(query.direction.as_str());
    value["relations"] = json!(relation_names(&query.relations));
    Ok(value)
}

// ---- source --------------------------------------------------------------------------------

/// A bounded source snippet for an **indexed** path (THREAT-MODEL T6).
///
/// Two independent gates, in this order:
///
/// 1. The path must already appear in the index. A client-supplied path that Nerve never
///    indexed is refused before the filesystem is touched at all.
/// 2. The read goes through `nerve-index`'s repository prober, which resolves the path with the
///    same `canonical_child` choke point discovery uses. That is what catches a path that *was*
///    indexed and has since been replaced by a symlink, a deny-listed name, or a `..` component
///    that a corrupted database could contain.
///
/// Neither gate is sufficient alone, and neither is skipped. The byte range is clamped by the
/// prober, not here, because the ceiling belongs with the code that owns the repository root.
pub fn source(ctx: &Context<'_>, target: &Target) -> Answer {
    let path = target
        .get("path")
        .ok_or_else(|| ApiError::bad_request("path is required"))?;
    let start_line = target
        .bounded("start_line", 1, usize::MAX)
        .map_err(ApiError::bad_request)?;
    let end_line = target
        .bounded("end_line", start_line + 39, usize::MAX)
        .map_err(ApiError::bad_request)?;

    let indexed = nerve_store::path_is_indexed(ctx.conn, path).map_err(ApiError::internal)?;
    if !indexed {
        return Err(ApiError::with_detail(
            403,
            "not_indexed",
            "source is served only for paths that are in the index",
            json!({ "path": path, "reason": "not_indexed" }),
        ));
    }

    match ctx.prober.read_snippet(path, start_line, end_line) {
        SourceSnippet::Text {
            text,
            start_line,
            end_line,
            total_lines,
            truncated,
            content_hash,
        } => Ok(json!({
            "path": path,
            "start_line": start_line,
            "end_line": end_line,
            "total_lines": total_lines,
            "truncated": truncated,
            "content_hash": content_hash,
            "max_lines": nerve_index::MAX_SNIPPET_LINES,
            "max_bytes": nerve_index::MAX_SNIPPET_BYTES,
            "text": text,
        })),
        // "Refused" is never disguised as "missing": the output always says which check fired.
        SourceSnippet::Refused => Err(ApiError::with_detail(
            403,
            "path_refused",
            "the repository path guard refused this path; nothing was read",
            json!({ "path": path, "reason": "refused" }),
        )),
        SourceSnippet::Missing => Err(ApiError::with_detail(
            404,
            "file_missing",
            "the indexed file no longer exists",
            json!({ "path": path, "reason": "missing" }),
        )),
        SourceSnippet::Unreadable => Err(ApiError::with_detail(
            409,
            "file_unreadable",
            "the file exists but could not be read as UTF-8 text within the size ceiling",
            json!({ "path": path, "reason": "unreadable" }),
        )),
    }
}

// ---- absence of knowledge ------------------------------------------------------------------

/// What Nerve could not resolve. Absence is a value, not an omission.
pub fn unresolved(ctx: &Context<'_>, target: &Target) -> Answer {
    let limit = target
        .bounded("limit", 100, MAX_UNRESOLVED_LIMIT)
        .map_err(ApiError::bad_request)?;
    let offset = target
        .bounded_from_zero("offset", 0, usize::MAX)
        .map_err(ApiError::bad_request)?;
    let report = nerve_store::status(ctx.conn).map_err(ApiError::internal)?;
    let rows =
        nerve_store::unresolved_entities(ctx.conn, limit, offset).map_err(ApiError::internal)?;

    Ok(json!({
        "limit": limit,
        "offset": offset,
        "unresolved_entities_total": report.unresolved_entities,
        "unresolved_assertions_total": report.unresolved_assertions,
        "count": rows.len(),
        "results": rows.iter().map(|row| json!({
            "entity_id": row.entity_id,
            "name": row.name,
            "scope_path": row.scope_path,
            "meta": row.meta.as_deref().map(shapes::details),
            "referencing_assertions": row.referencing_assertions,
        })).collect::<Vec<_>>(),
    }))
}

/// Which symbols no test is known to touch — and whether that question can be answered here.
///
/// The whole point of the endpoint is the state it carries. A repository that never ingested
/// coverage answers `coverage: "absent"`, `answerable: false`, `totals: null` and an empty row
/// list, rather than every symbol it has. Serving the naive answer would tell a client "your
/// tests cover nothing" when the truth is "Nerve has not been told anything about your tests".
///
/// The query itself lives in `nerve-store` and is the same one `nerve gaps` calls, so the two
/// surfaces cannot drift into different answers (ARCHITECTURE.md invariant 3).
pub fn gaps(ctx: &Context<'_>, target: &Target) -> Answer {
    let kind = match target.get("kind") {
        None => None,
        Some(name) => match name.parse::<EntityKind>() {
            Ok(kind) if kind.is_symbol() => Some(kind),
            _ => {
                return Err(ApiError::with_detail(
                    400,
                    "unknown_kind",
                    format!("unknown symbol kind {name:?}"),
                    json!({
                        "kind": name,
                        "allowed": EntityKind::ALL.iter()
                            .filter(|kind| kind.is_symbol())
                            .map(|kind| kind.as_str())
                            .collect::<Vec<_>>(),
                    }),
                ))
            }
        },
    };
    let query = nerve_store::GapQuery {
        path_prefix: target.get("under").map(str::to_string),
        kind,
        include_partial: target.flag("include_partial"),
        limit: target
            .bounded("limit", 100, MAX_GAPS_LIMIT)
            .map_err(ApiError::bad_request)?,
    };
    // Freshness is computed by re-reading the repository through the prober, which enforces the
    // path rules on every path the database hands it.
    let report = nerve_store::gaps(ctx.conn, &query, ctx.prober).map_err(ApiError::internal)?;

    let mut value = shapes::gap_report(&report);
    value["under"] = json!(query.path_prefix);
    value["kind"] = json!(kind.map(|kind| kind.as_str()));
    value["include_partial"] = json!(query.include_partial);
    Ok(value)
}

/// Which files parsed with errors, so what came out of them can be read with suspicion.
pub fn partial_parses(ctx: &Context<'_>) -> Answer {
    let Some(repo_id) = ctx.repo_id else {
        return Ok(json!({ "count": 0, "results": [] }));
    };
    let rows = nerve_index::partial_parses(ctx.conn, repo_id).map_err(ApiError::internal)?;
    Ok(json!({
        "count": rows.len(),
        "results": rows.iter().map(|row| json!({
            "rel_path": row.rel_path,
            "language": row.language,
            "content_hash": row.content_hash,
            "dynamic_imports_without_specifier": row.dynamic_imports_without_specifier,
            "unmodelled_call_sites": row.unmodelled_call_sites,
            "unmodelled_by_form": row.unmodelled_by_form,
        })).collect::<Vec<_>>(),
    }))
}

// ---- shared argument handling ---------------------------------------------------------------

/// Resolve one selector parameter to exactly one entity, or refuse.
///
/// Ambiguity is a refusal carrying every candidate. Nothing is chosen on the caller's behalf:
/// silently picking one is the failure mode that makes a tool untrustworthy in exactly the
/// situation where the caller most needs it to be right.
fn resolve(ctx: &Context<'_>, target: &Target, key: &str) -> Result<EntityRef, ApiError> {
    let selector = target
        .get(key)
        .ok_or_else(|| ApiError::bad_request(format!("{key} is required")))?;
    match nerve_store::resolve_selector(ctx.conn, selector).map_err(ApiError::internal)? {
        Selection::Resolved { entity, .. } => Ok(*entity),
        Selection::Ambiguous {
            candidates,
            matched_by,
        } => Err(ApiError::with_detail(
            409,
            "ambiguous_selector",
            format!("{selector:?} matches {} entities", candidates.len()),
            json!({
                "parameter": key,
                "selector": selector,
                "matched_by": matched_by.as_str(),
                "candidates": candidates.iter().map(shapes::entity).collect::<Vec<_>>(),
            }),
        )),
        Selection::NotFound { suggestions } => Err(ApiError::with_detail(
            404,
            "selector_not_found",
            format!("{selector:?} matches no indexed entity"),
            json!({
                "parameter": key,
                "selector": selector,
                "suggestions": suggestions.iter().map(shapes::search_hit).collect::<Vec<_>>(),
            }),
        )),
    }
}

fn direction(target: &Target) -> Result<Direction, ApiError> {
    match target.get("direction") {
        None => Ok(Direction::Any),
        Some("any") => Ok(Direction::Any),
        Some("forward") => Ok(Direction::Forward),
        Some(other) => Err(ApiError::with_detail(
            400,
            "unknown_direction",
            format!("unknown direction {other:?}"),
            json!({ "direction": other, "allowed": ["forward", "any"] }),
        )),
    }
}

/// Parse `relation=` against the closed relation vocabulary.
///
/// Only names from [`Relation::ALL`] ever reach the store, which is why the relation filter can
/// be inlined into SQL there without becoming an injection surface.
fn relations(target: &Target) -> Result<Vec<Relation>, ApiError> {
    let mut parsed = Vec::new();
    for value in target.list("relation") {
        match value.parse::<Relation>() {
            Ok(relation) if !parsed.contains(&relation) => parsed.push(relation),
            Ok(_) => {}
            Err(_) => {
                return Err(ApiError::with_detail(
                    400,
                    "unknown_relation",
                    format!("unknown relation {value:?}"),
                    json!({
                        "relation": value,
                        "allowed": Relation::ALL.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                    }),
                ))
            }
        }
    }
    Ok(parsed)
}

fn relation_names(relations: &[Relation]) -> Vec<&'static str> {
    relations.iter().map(|relation| relation.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_defaults_to_undirected_and_rejects_anything_else() {
        assert_eq!(
            direction(&Target::parse("/a").unwrap()).unwrap(),
            Direction::Any
        );
        assert_eq!(
            direction(&Target::parse("/a?direction=forward").unwrap()).unwrap(),
            Direction::Forward
        );
        let err = direction(&Target::parse("/a?direction=sideways").unwrap()).unwrap_err();
        assert_eq!(err.status, 400);
        assert_eq!(err.code, "unknown_direction");
    }

    #[test]
    fn only_closed_vocabulary_relations_are_accepted() {
        let target = Target::parse("/a?relation=CALLS,DEFINES,CALLS").unwrap();
        assert_eq!(
            relations(&target).unwrap(),
            vec![Relation::Calls, Relation::Defines]
        );
        let err = relations(&Target::parse("/a?relation=DROP+TABLE").unwrap()).unwrap_err();
        assert_eq!(err.code, "unknown_relation");
        assert_eq!(err.status, 400);
    }

    #[test]
    fn ceilings_are_the_documented_contract() {
        assert_eq!(MAX_PATH_DEPTH, 32);
        assert_eq!(MAX_PATH_LIMIT, 25);
        assert_eq!(MAX_SEARCH_LIMIT, 200);
        assert_eq!(MAX_NEIGHBOURHOOD_DEPTH, 4);
        assert_eq!(MAX_NEIGHBOURHOOD_NODES, 500);
        assert_eq!(MAX_UNRESOLVED_LIMIT, 500);
    }
}
