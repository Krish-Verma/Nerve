//! The wire shapes.
//!
//! These are deliberately the **same field names the CLI's `--json` already emits**. A path hop
//! looks the same whether it arrived over stdout or over HTTP, because both surfaces render the
//! same `nerve-store` types and neither computes anything (ARCHITECTURE.md invariant 3). Two
//! surfaces that describe the same graph with different vocabulary would be two products.
//!
//! Nothing in this module reads the database. It converts values that were already fetched.

use serde_json::{json, Value};

use nerve_store::{
    AssertionEvidence, EntityRef, NeighbourEdge, NeighbourhoodReport, ObservationEvidence,
    OccurrenceRow, PathHop, PathReport, SearchHit, WhyReport,
};

/// An entity, with its qualified name and first location resolved.
pub fn entity(entity: &EntityRef) -> Value {
    json!({
        "entity_id": entity.entity_id,
        "kind": entity.kind,
        "name": entity.name,
        "scope_path": entity.scope_path,
        "qualified_name": entity.qualified_name(),
        "language": entity.language,
        "file_path": entity.file_path,
        "start_line": entity.start_line,
        "end_line": entity.end_line,
    })
}

/// One occurrence span.
pub fn occurrence(row: &OccurrenceRow) -> Value {
    json!({
        "occurrence_id": row.occurrence_id,
        "file_path": row.file_path,
        "start_byte": row.start_byte,
        "end_byte": row.end_byte,
        "start_line": row.start_line,
        "start_col": row.start_col,
        "end_line": row.end_line,
        "end_col": row.end_col,
        "content_hash": row.content_hash,
    })
}

/// One search hit.
pub fn search_hit(hit: &SearchHit) -> Value {
    json!({
        "entity_id": hit.entity_id,
        "kind": hit.kind,
        "name": hit.name,
        "scope_path": hit.scope_path,
        "language": hit.language,
        "file_path": hit.file_path,
        "start_line": hit.start_line,
        "end_line": hit.end_line,
        "score": hit.score,
    })
}

/// One traversed hop.
pub fn hop(hop: &PathHop) -> Value {
    json!({
        "relation": hop.relation,
        "assertion_id": hop.assertion_id,
        "from": entity(&hop.from),
        "to": entity(&hop.to),
        "traversed_backwards": hop.traversed_backwards,
        "is_unresolved": hop.is_unresolved,
        "status": hop.status,
        "strongest_source_type": hop.strongest_source_type,
        "observation_count": hop.observation_count,
        "file_path": hop.file_path,
        "start_line": hop.start_line,
    })
}

/// A whole path report, bounds and truncation included.
pub fn path_report(report: &PathReport) -> Value {
    json!({
        "from": entity(&report.from),
        "to": entity(&report.to),
        "max_depth": report.max_depth,
        "truncated": report.truncated,
        "expansions": report.expansions,
        "count": report.paths.len(),
        "paths": report.paths.iter().map(|found| json!({
            "length": found.length(),
            "traverses_unresolved": found.traverses_unresolved(),
            "hops": found.hops.iter().map(hop).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// One neighbourhood edge, in its recorded direction.
pub fn neighbour_edge(edge: &NeighbourEdge) -> Value {
    json!({
        "assertion_id": edge.assertion_id,
        "relation": edge.relation,
        "source_entity_id": edge.source_entity_id,
        "target_entity_id": edge.target_entity_id,
        "is_unresolved": edge.is_unresolved,
        "status": edge.status,
        "strongest_source_type": edge.strongest_source_type,
        "observation_count": edge.observation_count,
        "file_path": edge.file_path,
        "start_line": edge.start_line,
    })
}

/// A bounded neighbourhood, with the count it did not show.
pub fn neighbourhood(report: &NeighbourhoodReport) -> Value {
    json!({
        "focus": entity(&report.focus),
        "max_depth": report.max_depth,
        "max_nodes": report.max_nodes,
        "truncated": report.truncated,
        "omitted_nodes": report.omitted_nodes,
        "frontier_nodes": report.frontier_nodes,
        "node_count": report.nodes.len(),
        "edge_count": report.edges.len(),
        "nodes": report.nodes.iter().map(|node| json!({
            "depth": node.depth,
            "entity": entity(&node.entity),
        })).collect::<Vec<_>>(),
        "edges": report.edges.iter().map(neighbour_edge).collect::<Vec<_>>(),
    })
}

/// One observation, with its full evidence profile and computed freshness.
pub fn observation(observation: &ObservationEvidence) -> Value {
    json!({
        "observation_id": observation.observation_id,
        "evidence_source_type": observation.evidence_source_type,
        "directness": observation.directness,
        "extractor_id": observation.extractor_id,
        "extractor_version": observation.extractor_version,
        "match_quality": observation.match_quality,
        "state_id": observation.state_id,
        "file_path": observation.file_path,
        "start_line": observation.start_line,
        "end_line": observation.end_line,
        "content_hash": observation.content_hash,
        "environment": observation.environment,
        "details": observation.details.as_deref().map(details),
        "created_at": observation.created_at,
        "freshness": observation.freshness.as_str(),
    })
}

/// One assertion and every observation behind it.
pub fn assertion(assertion: &AssertionEvidence) -> Value {
    json!({
        "assertion_id": assertion.assertion_id,
        "relation": assertion.relation,
        "direction": assertion.direction.as_str(),
        "source": entity(&assertion.source),
        "target": entity(&assertion.target),
        "status": assertion.status,
        "is_unresolved": assertion.is_unresolved,
        "observation_count": assertion.observation_count,
        "strongest_source_type": assertion.strongest_source_type,
        "observations": assertion.observations.iter().map(observation).collect::<Vec<_>>(),
    })
}

/// The whole evidence packet.
pub fn why_report(report: &WhyReport) -> Value {
    json!({
        "subject": entity(&report.subject),
        "object": report.object.as_ref().map(entity),
        "files_probed": report.files_probed,
        "count": report.assertions.len(),
        "assertions": report.assertions.iter().map(assertion).collect::<Vec<_>>(),
    })
}

/// Render a stored `details` blob as JSON when it parses, as a string when it does not.
///
/// A `details` payload is extractor-written but lands in the database, which is a file on disk;
/// a value that does not parse is surfaced as the text it is rather than dropped or guessed at.
pub fn details(details: &str) -> Value {
    serde_json::from_str(details).unwrap_or_else(|_| json!(details))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EntityRef {
        EntityRef {
            entity_id: "meth_1".into(),
            kind: "method".into(),
            name: "area".into(),
            scope_path: "Circle".into(),
            language: Some("typescript".into()),
            file_path: Some("src/shapes.ts".into()),
            start_line: Some(9),
            end_line: Some(11),
        }
    }

    #[test]
    fn an_entity_carries_its_qualified_name_and_location() {
        let value = entity(&sample());
        assert_eq!(value["qualified_name"], "Circle.area");
        assert_eq!(value["file_path"], "src/shapes.ts");
        assert_eq!(value["start_line"], 9);
    }

    #[test]
    fn a_hostile_name_is_carried_as_data_not_markup() {
        let mut hostile = sample();
        hostile.name = "<img src=x onerror=alert(1)>".into();
        let value = entity(&hostile);
        assert_eq!(value["name"], "<img src=x onerror=alert(1)>");
        // The value is a JSON string. Rendering it safely is `respond::to_json_bytes`' job.
        assert!(value["name"].is_string());
    }

    #[test]
    fn unparseable_details_survive_as_text() {
        assert_eq!(details("{\"a\":1}"), json!({ "a": 1 }));
        assert_eq!(details("not json"), json!("not json"));
    }
}
