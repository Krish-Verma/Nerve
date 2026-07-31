//! Nerve core model.
//!
//! Identity computation, closed vocabularies, error types, and the canonical graph dump used
//! by golden and determinism tests. This crate has no I/O and no dependency on storage or
//! parsing — it is the shared vocabulary every other crate speaks.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod dump;
pub mod error;
pub mod ids;
pub mod model;
pub mod vocab;

pub use dump::CanonicalDump;
pub use error::{NerveError, Result};
pub use model::{
    AssertionRecord, EntityRecord, GraphBatch, ObservationRecord, OccurrenceRecord, Span,
};
pub use vocab::{
    AssertionStatus, Directness, EntityKind, EvidenceSourceType, Relation, UnresolvedCategory,
};
