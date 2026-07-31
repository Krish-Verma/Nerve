//! Canonical graph dump.
//!
//! Used by golden, determinism, and idempotence tests. The dump is deliberately **not** a
//! faithful database snapshot: it excludes everything that is not a property of the code.
//!
//! Excluded, and why:
//! - timestamps (`created_at`, `started_at`, `finished_at`) — wall-clock, not code
//! - autoincrement surrogate keys (`observation_id`, `run_id`, `link_id`) — insertion order
//! - absolute paths (`repository.root_path`) — machine-specific
//! - `occurrence_id` — a pure function of fields already present
//! - the `extractor_run` table — an audit log of runs, which grows by design on re-index
//!
//! Everything retained is a deterministic function of (project_id, file set, file contents).

use serde::{Deserialize, Serialize};

/// A canonical, sortable, machine-comparable view of the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalDump {
    /// Schema version the dump was taken at.
    pub schema_version: i64,
    /// Project identifier the entity ids were derived from.
    pub project_id: String,
    /// Every repository state present, sorted.
    pub state_ids: Vec<String>,
    /// Entities, sorted by `entity_id`.
    pub entities: Vec<DumpEntity>,
    /// Occurrences, sorted.
    pub occurrences: Vec<DumpOccurrence>,
    /// Assertions, sorted.
    pub assertions: Vec<DumpAssertion>,
    /// Observations, sorted.
    pub observations: Vec<DumpObservation>,
    /// Derived assertion states, sorted.
    pub assertion_states: Vec<DumpAssertionState>,
}

/// Canonical entity row.
///
/// `serde_json::Value` has no total order, so this type is sorted by an explicit key rather
/// than by a derived `Ord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpEntity {
    /// Logical identifier.
    pub entity_id: String,
    /// Entity kind.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Scope path.
    pub scope_path: String,
    /// Language tag.
    pub language: Option<String>,
    /// Canonicalized metadata JSON.
    pub meta: Option<serde_json::Value>,
}

/// Canonical occurrence row. `occurrence_id` is omitted: it is derived from these fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DumpOccurrence {
    /// Entity that appears.
    pub entity_id: String,
    /// Repository state.
    pub state_id: String,
    /// Repository-relative path.
    pub file_path: String,
    /// Inclusive start byte.
    pub start_byte: i64,
    /// Exclusive end byte.
    pub end_byte: i64,
    /// 1-based start line.
    pub start_line: i64,
    /// 0-based start column.
    pub start_col: i64,
    /// 1-based end line.
    pub end_line: i64,
    /// 0-based end column.
    pub end_col: i64,
    /// File content hash.
    pub content_hash: String,
}

/// Canonical assertion row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DumpAssertion {
    /// Claim identifier.
    pub assertion_id: String,
    /// Source entity.
    pub source_entity_id: String,
    /// Relation.
    pub relation: String,
    /// Target entity.
    pub target_entity_id: String,
}

/// Canonical observation row. Surrogate keys and `created_at` are omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DumpObservation {
    /// Claim supported.
    pub assertion_id: String,
    /// Evidence source type.
    pub evidence_source_type: String,
    /// Directness.
    pub directness: String,
    /// Producing extractor.
    pub extractor_id: String,
    /// Producing extractor version.
    pub extractor_version: String,
    /// Match quality, where meaningful.
    pub match_quality: Option<f64>,
    /// Repository state.
    pub state_id: String,
    /// Repository-relative path.
    pub file_path: String,
    /// 1-based first line, or 0 for structural facts.
    pub start_line: i64,
    /// 1-based last line, or 0 for structural facts.
    pub end_line: i64,
    /// Content hash of the evidence.
    pub content_hash: String,
    /// Execution environment.
    pub environment: Option<String>,
    /// Canonicalized evidence detail JSON.
    pub details: Option<serde_json::Value>,
}

impl Eq for DumpObservation {}

/// Canonical derived-state row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DumpAssertionState {
    /// Claim.
    pub assertion_id: String,
    /// Repository state the claim was last observed in.
    pub state_id: String,
    /// Derived status.
    pub status: String,
    /// Structurally strongest source type present.
    pub strongest_source_type: String,
    /// Bitmask of source types present.
    pub source_type_mask: i64,
    /// Number of supporting observations.
    pub observation_count: i64,
    /// 1 when the target is an `Unresolved` entity.
    pub is_unresolved: i64,
    /// Last state the claim was seen in.
    pub last_seen_state_id: String,
}

fn entity_sort_key(e: &DumpEntity) -> (String, String, String, String) {
    (
        e.entity_id.clone(),
        e.kind.clone(),
        e.scope_path.clone(),
        e.name.clone(),
    )
}

fn observation_sort_key(o: &DumpObservation) -> (String, String, i64, i64, String, String) {
    (
        o.assertion_id.clone(),
        o.file_path.clone(),
        o.start_line,
        o.end_line,
        o.evidence_source_type.clone(),
        o.details
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_default(),
    )
}

impl CanonicalDump {
    /// Sort every collection into its canonical order. Idempotent.
    pub fn sort(&mut self) {
        self.state_ids.sort();
        self.state_ids.dedup();
        self.entities.sort_by_key(entity_sort_key);
        self.occurrences.sort();
        self.assertions.sort();
        self.observations.sort_by_key(observation_sort_key);
        self.assertion_states.sort();
    }

    /// Sort, then render as pretty JSON with a trailing newline.
    ///
    /// `serde_json`'s object representation is a `BTreeMap`, so nested `meta` and `details`
    /// objects serialize with sorted keys. Struct fields serialize in declaration order.
    /// The output is therefore byte-stable for a given graph.
    pub fn to_canonical_json(&self) -> crate::error::Result<String> {
        let mut copy = self.clone();
        copy.sort();
        let mut json = serde_json::to_string_pretty(&copy)?;
        json.push('\n');
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: &str) -> DumpEntity {
        DumpEntity {
            entity_id: id.into(),
            kind: "function".into(),
            name: "n".into(),
            scope_path: String::new(),
            language: None,
            meta: None,
        }
    }

    #[test]
    fn sorting_is_deterministic_and_idempotent() {
        let mut a = CanonicalDump {
            schema_version: 1,
            project_id: "p".into(),
            state_ids: vec!["b".into(), "a".into(), "a".into()],
            entities: vec![entity("z"), entity("a")],
            occurrences: vec![],
            assertions: vec![],
            observations: vec![],
            assertion_states: vec![],
        };
        let mut b = a.clone();
        b.entities.reverse();
        b.state_ids.reverse();

        assert_eq!(
            a.to_canonical_json().unwrap(),
            b.to_canonical_json().unwrap()
        );

        a.sort();
        let once = a.to_canonical_json().unwrap();
        a.sort();
        assert_eq!(once, a.to_canonical_json().unwrap());
        assert_eq!(a.state_ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn canonical_json_round_trips() {
        let dump = CanonicalDump {
            schema_version: 1,
            project_id: "p".into(),
            state_ids: vec!["s".into()],
            entities: vec![entity("a")],
            occurrences: vec![],
            assertions: vec![],
            observations: vec![],
            assertion_states: vec![],
        };
        let json = dump.to_canonical_json().unwrap();
        let parsed: CanonicalDump = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, dump);
    }
}
