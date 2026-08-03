//! Nerve's machine-facing surfaces: a local HTTP server, and MCP over stdio.
//!
//! A **loopback-only, single-user, read-only** server that makes the evidence graph explorable
//! from a browser, plus [`mcp`], which answers the same questions for an agent over stdin and
//! stdout. Both are surfaces, not layers: every question either answers is answered by calling
//! the same `nerve-store` and `nerve-index` functions the CLI calls (ARCHITECTURE.md invariant
//! 3). If an endpoint needed a query that did not exist, the query was added to `nerve-store`;
//! none of it lives here.
//!
//! The two share [`api`] and [`shapes`] rather than describing the graph twice, so a client
//! cannot be told one thing over HTTP and a different thing over MCP. [`mcp`] adds only what is
//! specific to talking to an agent: JSON-RPC framing, argument validation, response bounds and
//! the untrusted-content label (THREAT-MODEL T7 and T8). It opens no socket and binds no port.
//!
//! ## Why there is no async runtime here
//!
//! `nerve serve` handles one human. Tokio plus a web framework is roughly as many crates again
//! as this entire product has, every one of which must be licence-reviewed
//! (`third_party/LICENSES.md`), and it introduces an async execution model into a codebase that
//! is deliberately serial for determinism. A blocking server with a small thread pool is the
//! shape of the requirement. See `docs/plans/slice-04-visual-explorer.md` §P1.
//!
//! ## Security posture
//!
//! Three controls are blocking gates (`docs/THREAT-MODEL.md`), and each has its own module:
//!
//! - **T4, cross-site request forgery and DNS rebinding.** Binding to `127.0.0.1` is not an
//!   access control — a page the user visits can reach loopback. See [`token`] and [`guard`]:
//!   a 256-bit per-session token from the OS CSPRNG, compared in constant time; `Host` pinned to
//!   the bound address, which is what defeats an attacker domain that resolves to `127.0.0.1`;
//!   `Origin` refused unless it is exactly ours; and no CORS header emitted anywhere.
//! - **T5, stored XSS from repository content.** `<img src=x onerror=alert(1)>` is a legal file
//!   name and Nerve indexes it. See [`respond`]: every answer is JSON, produced by a serializer,
//!   with `<`, `>`, `&`, U+2028 and U+2029 escaped so the served bytes cannot open markup; plus
//!   a `Content-Security-Policy` with no `unsafe-inline` and no remote origin. Nothing in
//!   [`assets`] is templated, so the served document has no injection point at all.
//! - **T6, serving files outside the indexed root.** See [`api::source`]: a path must already be
//!   in the index, *and* must survive `nerve-index`'s `canonical_child` choke point — the same
//!   one discovery uses, not a second implementation. The byte range is bounded.
//!
//! The whole API is `GET`-only and opens the database read-only, so a request that somehow
//! passed every check still could not change anything.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod api;
pub mod assets;
pub mod error;
pub mod guard;
pub mod mcp;
pub mod request;
pub mod respond;
pub mod router;
pub mod shapes;
pub mod token;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use nerve_index::RepositoryProber;

pub use error::{Result, ServerError};
pub use guard::{Guard, Rejection};
pub use token::SessionToken;

/// Default number of request-handling threads.
///
/// One human with a browser opens a handful of connections. The pool exists so that a slow
/// query does not stall the page, not to serve load.
pub const DEFAULT_WORKERS: usize = 4;

/// Largest worker count accepted. Each worker holds its own SQLite connection.
pub const MAX_WORKERS: usize = 16;

/// What to serve, and where.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Repository root. Must already contain an index.
    pub root: PathBuf,
    /// TCP port. `0` asks the operating system for a free one, which is the default.
    pub port: u16,
    /// Request-handling threads.
    pub workers: usize,
}

impl ServeConfig {
    /// Serve `root` on an ephemeral port with the default worker count.
    pub fn new(root: impl Into<PathBuf>) -> ServeConfig {
        ServeConfig {
            root: root.into(),
            port: 0,
            workers: DEFAULT_WORKERS,
        }
    }
}

/// A stop button that can be handed to a signal handler.
#[derive(Clone)]
pub struct ShutdownHandle {
    server: Arc<tiny_http::Server>,
    stopping: Arc<AtomicBool>,
    workers: usize,
}

impl ShutdownHandle {
    /// Ask every worker to finish its current request and stop.
    ///
    /// Idempotent, and safe to call from a signal-handling thread. `unblock` releases exactly
    /// one thread parked in `recv`, so it is called once per worker.
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for _ in 0..self.workers {
            self.server.unblock();
        }
    }
}

impl std::fmt::Debug for ShutdownHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownHandle")
            .field("workers", &self.workers)
            .field("stopping", &self.stopping.load(Ordering::SeqCst))
            .finish()
    }
}

/// A bound, running server.
#[derive(Debug)]
pub struct RunningServer {
    address: SocketAddr,
    token: SessionToken,
    shutdown: ShutdownHandle,
    workers: Vec<JoinHandle<()>>,
}

impl RunningServer {
    /// The loopback address actually bound, with the resolved port.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The session token every request must carry.
    pub fn token(&self) -> &SessionToken {
        &self.token
    }

    /// The URL to open, carrying the token so the page can bootstrap itself.
    pub fn url(&self) -> String {
        format!(
            "http://{}/?{}={}",
            self.address,
            token::TOKEN_QUERY,
            self.token.as_str()
        )
    }

    /// The base URL, without the token.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// A stop button for a signal handler.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.clone()
    }

    /// Block until every worker has stopped.
    pub fn join(self) {
        for worker in self.workers {
            let _ = worker.join();
        }
    }

    /// Stop, then block until every worker has stopped.
    pub fn shutdown_and_join(self) {
        self.shutdown.shutdown();
        self.join();
    }
}

/// Bind the loopback address and start serving.
///
/// Returns as soon as the socket is bound and the workers are running, so the caller can print
/// the URL before anything is served.
///
/// The address is constructed from [`Ipv4Addr::LOCALHOST`] directly rather than from a string
/// that a resolver would interpret. There is no code path in this crate that can produce any
/// other bind address, which is the strongest available form of "binds `127.0.0.1` only".
pub fn serve(config: ServeConfig) -> Result<RunningServer> {
    // Canonicalized through `nerve-index`'s own entry point rather than by calling the
    // filesystem here: the repository root is the anchor every path-safety decision is made
    // against, and there is exactly one definition of what it is.
    let root = nerve_index::discover::canonical_root(&config.root)
        .map_err(|_| ServerError::NoSuchRoot(config.root.clone()))?;
    let db_path = nerve_index::config::db_path(&root);
    if !db_path.exists() {
        return Err(ServerError::NotIndexed(root));
    }
    // Built once and shared: it canonicalizes the root and loads the deny-list, and every
    // worker must enforce exactly the same rules.
    let prober = Arc::new(RepositoryProber::new(&root)?);

    let token = SessionToken::generate().map_err(ServerError::Randomness)?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let server = tiny_http::Server::http(address).map_err(|err| ServerError::Bind {
        port: config.port,
        message: err.to_string(),
    })?;
    let address = server
        .server_addr()
        .to_ip()
        .expect("an IP server reports an IP address");
    debug_assert!(
        address.ip().is_loopback(),
        "nerve serve must be loopback-only"
    );

    let server = Arc::new(server);
    let stopping = Arc::new(AtomicBool::new(false));
    let worker_count = config.workers.clamp(1, MAX_WORKERS);
    let guard = Arc::new(Guard::new(token.clone(), address));

    let mut workers = Vec::with_capacity(worker_count);
    for index in 0..worker_count {
        let server = Arc::clone(&server);
        let stopping = Arc::clone(&stopping);
        let guard = Arc::clone(&guard);
        let prober = Arc::clone(&prober);
        let db_path = db_path.clone();
        let handle = std::thread::Builder::new()
            .name(format!("nerve-serve-{index}"))
            .spawn(move || worker(&server, &stopping, &guard, &prober, &db_path))
            .map_err(|err| ServerError::Bind {
                port: address.port(),
                message: format!("cannot start worker thread: {err}"),
            })?;
        workers.push(handle);
    }

    Ok(RunningServer {
        address,
        token,
        shutdown: ShutdownHandle {
            server,
            stopping,
            workers: worker_count,
        },
        workers,
    })
}

/// Open the index for reading only.
///
/// `PRAGMA query_only` makes read-onlyness a property SQLite enforces rather than one this
/// crate promises: no statement reachable from any handler can write, whatever a future
/// handler does by accident.
fn open_read_only(db_path: &Path) -> nerve_store::Result<nerve_store::Connection> {
    let conn = nerve_store::open(db_path)?;
    conn.pragma_update(None, "query_only", "ON")?;
    Ok(conn)
}

fn worker(
    server: &tiny_http::Server,
    stopping: &AtomicBool,
    guard: &Guard,
    prober: &RepositoryProber,
    db_path: &Path,
) {
    // One connection per worker: `rusqlite::Connection` is `Send` but not `Sync`, and reopening
    // it per request would put a file open on the latency path of every keystroke in search.
    let Ok(conn) = open_read_only(db_path) else {
        return;
    };
    let repo_id = nerve_store::repository(&conn)
        .ok()
        .flatten()
        .map(|repository| repository.repo_id);

    while !stopping.load(Ordering::SeqCst) {
        let Ok(request) = server.recv() else {
            // `recv` fails when the queue was unblocked for shutdown, or when the listener is
            // gone. Either way this worker is finished.
            break;
        };
        let ctx = api::Context {
            conn: &conn,
            prober,
            repo_id: repo_id.as_deref(),
            db_path,
        };
        router::handle(request, guard, &ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_configuration_asks_for_an_ephemeral_loopback_port() {
        let config = ServeConfig::new("/tmp/does-not-matter");
        assert_eq!(config.port, 0);
        assert_eq!(config.workers, DEFAULT_WORKERS);
    }

    #[test]
    fn serving_an_unindexed_directory_is_refused_before_anything_binds() {
        let dir = tempfile::tempdir().unwrap();
        let err = serve(ServeConfig::new(dir.path())).unwrap_err();
        assert!(matches!(err, ServerError::NotIndexed(_)), "{err}");
    }

    #[test]
    fn serving_a_missing_directory_is_refused() {
        let err = serve(ServeConfig::new("/nerve/does/not/exist")).unwrap_err();
        assert!(matches!(err, ServerError::NoSuchRoot(_)), "{err}");
    }
}
