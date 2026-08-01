//! Errors raised while starting the server.

use std::path::PathBuf;

/// Why `nerve serve` could not start.
#[derive(Debug)]
pub enum ServerError {
    /// The repository root does not exist or is not a directory.
    NoSuchRoot(PathBuf),
    /// There is no index at that root.
    NotIndexed(PathBuf),
    /// The index could not be opened.
    Store(nerve_store::StoreError),
    /// The repository could not be read for path-safety configuration.
    Index(nerve_index::IndexError),
    /// The operating system would not supply randomness for the session token.
    Randomness(std::io::Error),
    /// The loopback address could not be bound.
    Bind {
        /// Port that was requested. `0` means "any free port".
        port: u16,
        /// Underlying failure, as reported by the HTTP server.
        message: String,
    },
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::NoSuchRoot(path) => {
                write!(f, "{} is not a readable directory", path.display())
            }
            ServerError::NotIndexed(path) => write!(
                f,
                "no Nerve index at {}; run `nerve init` and `nerve index` first",
                path.display()
            ),
            ServerError::Store(err) => write!(f, "{err}"),
            ServerError::Index(err) => write!(f, "{err}"),
            ServerError::Randomness(err) => {
                write!(f, "cannot generate a session token: {err}")
            }
            ServerError::Bind { port, message } => write!(
                f,
                "cannot bind 127.0.0.1:{port}: {message}; try a different --port"
            ),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<nerve_store::StoreError> for ServerError {
    fn from(err: nerve_store::StoreError) -> ServerError {
        ServerError::Store(err)
    }
}

impl From<nerve_index::IndexError> for ServerError {
    fn from(err: nerve_index::IndexError) -> ServerError {
        ServerError::Index(err)
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, ServerError>;
