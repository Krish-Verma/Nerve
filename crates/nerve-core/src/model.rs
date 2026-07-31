//! In-memory records handed from the extraction pipeline to the store.
//!
//! Note what is absent: no `confidence: f64` anywhere, and no source text. Evidence is a
//! structured profile (ADR-0003) and source is read from disk when it is presented.

use crate::vocab::{Directness, EntityKind, EvidenceSourceType, Relation};

/// A source range. Bytes are 0-based; lines are 1-based; columns are 0-based byte offsets
/// within the line, as reported by the parser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start_byte: usize,
    /// Exclusive end byte offset.
    pub end_byte: usize,
    /// 1-based start line.
    pub start_line: usize,
    /// 0-based start column.
    pub start_col: usize,
    /// 1-based end line.
    pub end_line: usize,
    /// 0-based end column.
    pub end_col: usize,
}

impl Span {
    /// A zero-width span used for structural (filesystem) facts that have no source range.
    pub const NONE: Span = Span {
        start_byte: 0,
        end_byte: 0,
        start_line: 0,
        start_col: 0,
        end_line: 0,
        end_col: 0,
    };
}

/// A logical entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityRecord {
    /// Stable logical identifier.
    pub entity_id: String,
    /// Entity kind.
    pub kind: EntityKind,
    /// Display name.
    pub name: String,
    /// Lexical or filesystem scope, joined with `.` for symbols and `/` for paths.
    pub scope_path: String,
    /// Language tag, where one applies.
    pub language: Option<String>,
    /// Extractor-specific metadata as canonical JSON.
    pub meta: Option<String>,
}

/// A physical appearance of an entity in a repository state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccurrenceRecord {
    /// Physical identifier.
    pub occurrence_id: String,
    /// Entity that appears.
    pub entity_id: String,
    /// Repository-relative path.
    pub file_path: String,
    /// Source range.
    pub span: Span,
    /// BLAKE3 of the file's bytes at observation time.
    pub content_hash: String,
}

/// A claim that a relationship holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionRecord {
    /// Claim identifier.
    pub assertion_id: String,
    /// Source entity.
    pub source_entity_id: String,
    /// Relation.
    pub relation: Relation,
    /// Target entity.
    pub target_entity_id: String,
}

/// A piece of evidence supporting a claim. Extractors emit these and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservationRecord {
    /// Claim this evidence supports.
    pub assertion_id: String,
    /// How the evidence was obtained.
    pub evidence_source_type: EvidenceSourceType,
    /// How directly the artifact states it.
    pub directness: Directness,
    /// Producing extractor.
    pub extractor_id: String,
    /// Producing extractor version.
    pub extractor_version: String,
    /// Only meaningful for matching extractors; `None` otherwise.
    pub match_quality: Option<f64>,
    /// Repository-relative path of the evidence.
    pub file_path: String,
    /// 1-based first line of the evidence, or 0 for structural facts.
    pub start_line: usize,
    /// 1-based last line of the evidence, or 0 for structural facts.
    pub end_line: usize,
    /// BLAKE3 of what the source said at the time.
    pub content_hash: String,
    /// Execution environment, for execution evidence only.
    pub environment: Option<String>,
    /// Extractor-specific, human-readable evidence steps as canonical JSON.
    pub details: Option<String>,
}

/// Everything one extractor run produced, ready to persist in a single transaction.
#[derive(Debug, Default, Clone)]
pub struct GraphBatch {
    /// Entities discovered.
    pub entities: Vec<EntityRecord>,
    /// Occurrences discovered.
    pub occurrences: Vec<OccurrenceRecord>,
    /// Claims made.
    pub assertions: Vec<AssertionRecord>,
    /// Evidence for those claims.
    pub observations: Vec<ObservationRecord>,
}

impl GraphBatch {
    /// Verify that every observation carries a source type the extractor declared.
    ///
    /// ADR-0003: emitting outside the declaration is a hard error, not a warning.
    pub fn verify_declared_source_types(
        &self,
        extractor_id: &'static str,
        declared: &[EvidenceSourceType],
    ) -> crate::error::Result<()> {
        for observation in &self.observations {
            if !declared.contains(&observation.evidence_source_type) {
                return Err(crate::error::NerveError::UndeclaredEvidenceSourceType {
                    extractor_id,
                    source_type: observation.evidence_source_type.as_str(),
                    declared: declared
                        .iter()
                        .map(|d| d.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(source: EvidenceSourceType) -> ObservationRecord {
        ObservationRecord {
            assertion_id: "a".into(),
            evidence_source_type: source,
            directness: Directness::Direct,
            extractor_id: "x".into(),
            extractor_version: "1".into(),
            match_quality: None,
            file_path: "a.ts".into(),
            start_line: 1,
            end_line: 1,
            content_hash: "h".into(),
            environment: None,
            details: None,
        }
    }

    #[test]
    fn undeclared_source_type_is_a_hard_error() {
        let batch = GraphBatch {
            observations: vec![observation(EvidenceSourceType::LlmDerived)],
            ..Default::default()
        };
        let err = batch
            .verify_declared_source_types("t", &[EvidenceSourceType::AstDirect])
            .unwrap_err();
        assert!(err.to_string().contains("LLM_DERIVED"));
    }

    #[test]
    fn declared_source_type_passes() {
        let batch = GraphBatch {
            observations: vec![observation(EvidenceSourceType::AstDirect)],
            ..Default::default()
        };
        assert!(batch
            .verify_declared_source_types("t", &[EvidenceSourceType::AstDirect])
            .is_ok());
    }
}
