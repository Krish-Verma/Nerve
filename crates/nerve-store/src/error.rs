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

    /// Filesystem operation around the database file failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, StoreError>;
