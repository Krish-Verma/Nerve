//! Indexing errors.

use std::path::PathBuf;

/// Errors raised by the indexing pipeline.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// Storage failed.
    #[error(transparent)]
    Store(#[from] nerve_store::StoreError),

    /// A model invariant was violated.
    #[error(transparent)]
    Core(#[from] nerve_core::NerveError),

    /// Filesystem access failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// The requested root is not an existing directory.
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),

    /// `nerve init` has not been run for this tree.
    #[error("no Nerve index at {0}; run `nerve init` first")]
    NotInitialized(PathBuf),

    /// A path escaped the repository root and was refused.
    #[error("path escapes repository root: {0}")]
    PathEscapesRoot(PathBuf),

    /// A path could not be represented as UTF-8.
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),

    /// A path contained a C0 control character and was refused.
    ///
    /// Reported distinctly from [`IndexError::PathEscapesRoot`] because it is a different
    /// finding: the path did not escape anything, it attacked identity. See the comment on
    /// `canonical_child`.
    #[error("path contains a control character: {0:?}")]
    ControlCharacterInPath(PathBuf),

    /// `.nerve/config.toml` is missing, unreadable, or malformed.
    #[error("config error at {path}: {message}")]
    Config {
        /// Config file involved.
        path: PathBuf,
        /// What was wrong.
        message: String,
    },

    /// A tree-sitter grammar could not be loaded.
    #[error("parser error: {0}")]
    Parser(String),

    /// A cached module-facts payload could not be written.
    ///
    /// Reading one back is deliberately *not* an error: a payload this build cannot parse means
    /// the file must be re-extracted, which is a recoverable outcome, not a failed index.
    #[error("module cache: {0}")]
    ModuleCache(String),

    /// No source of operating-system randomness was available.
    #[error("could not obtain randomness for project_id: {0}")]
    Randomness(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, IndexError>;
