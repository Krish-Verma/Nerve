//! Shared domain errors.

use std::fmt;

/// Errors raised by the Nerve model layer.
#[derive(Debug, thiserror::Error)]
pub enum NerveError {
    /// A vocabulary string was not a member of its closed vocabulary.
    #[error("unknown {vocabulary} value: {value:?}")]
    UnknownVocabularyValue {
        /// The vocabulary that was being parsed.
        vocabulary: &'static str,
        /// The offending value.
        value: String,
    },

    /// An identity function was called with a kind it does not serve.
    #[error("entity kind {kind} is not valid for identity constructor {constructor}")]
    InvalidIdentityKind {
        /// Kind that was supplied.
        kind: &'static str,
        /// Constructor that rejected it.
        constructor: &'static str,
    },

    /// An extractor emitted an evidence source type it did not declare.
    #[error(
        "extractor {extractor_id} emitted undeclared evidence source type {source_type}; \
         declared: {declared}"
    )]
    UndeclaredEvidenceSourceType {
        /// Offending extractor.
        extractor_id: &'static str,
        /// The source type it tried to emit.
        source_type: &'static str,
        /// Comma-separated declared set.
        declared: String,
    },

    /// A canonical dump could not be serialized.
    #[error("canonical serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, NerveError>;

impl NerveError {
    /// Build an [`NerveError::UnknownVocabularyValue`].
    pub fn unknown(vocabulary: &'static str, value: impl fmt::Display) -> Self {
        NerveError::UnknownVocabularyValue {
            vocabulary,
            value: value.to_string(),
        }
    }
}
