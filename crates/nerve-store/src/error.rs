//! Storage errors.

/// Errors raised by the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A SQLite call failed.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A model-layer invariant was violated.
    #[error(transparent)]
    Core(#[from] nerve_core::NerveError),

    /// JSON stored in a `meta` or `details` column could not be parsed.
    #[error("invalid JSON in column {column}: {source}")]
    Json {
        /// Column the JSON came from.
        column: &'static str,
        /// Underlying parse error.
        source: serde_json::Error,
    },

    /// The database is newer than this build understands.
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew {
        /// Version found on disk.
        found: i64,
        /// Version this build supports.
        supported: i64,
    },

    /// A memory operation was asked for something the stored rows cannot support.
    ///
    /// Distinct from [`StoreError::Sqlite`] because these are refusals the *schema* cannot state:
    /// superseding a record that was invalidated, or a supersession whose successor names no
    /// predecessor. A `CHECK` sees one row and cannot reach either fact.
    #[error("memory: {0}")]
    Memory(String),

    /// A migration refused to run because rows on disk sit outside the domain it introduces.
    ///
    /// The refusal is the point, and it is not a fallback for a rewrite that was hard. A migration
    /// that narrows a column has three honest options — drop the offending rows, rewrite them to a
    /// default, or refuse — and for a table a **human authored** only the third is available:
    /// memory is the one thing in this database re-indexing cannot rebuild, so a migration that
    /// silently edited a note would be refused on the same ground as a delete verb. The offending
    /// values are named so the human can correct them and run again.
    #[error(
        "migration to schema v{version} refused: {table}.{column} holds {found}, which v{version} \
         does not admit (it admits {admitted}). Nothing was dropped or rewritten — these rows were \
         written by a human and re-indexing cannot rebuild them. Correct them and migrate again"
    )]
    MigrationDomain {
        /// The version whose domain the rows violate.
        version: i64,
        /// Table holding them.
        table: &'static str,
        /// Column whose domain they are outside.
        column: &'static str,
        /// The offending distinct values, quoted and comma-separated.
        found: String,
        /// The values the new schema does admit, quoted and comma-separated.
        admitted: String,
    },

    /// Filesystem operation around the database file failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, StoreError>;
