//! Shared fixtures and a deliberately dumb HTTP client.
//!
//! The client speaks raw HTTP/1.1 over a `TcpStream` rather than using a library, for two
//! reasons. Adding an HTTP client to the dependency tree to test a server that exists partly to
//! keep that tree small would be self-defeating. More importantly, the security tests need to
//! send headers a well-behaved client refuses to send — a forged `Host`, an `Origin` from
//! another site, no token at all — and a polite client is exactly the wrong tool for proving
//! that an impolite one is refused.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nerve_server::{RunningServer, ServeConfig};

/// Project id pinned so entity ids are stable across runs.
pub const TEST_PROJECT_ID: &str = "00000000000000000000000000000001";

/// A file name that is legal on disk and is also an XSS payload.
///
/// Repository content is attacker-controlled (THREAT-MODEL T5) and this is what that means in
/// practice: Nerve will index this file, name a `file` entity after it, name a `module` entity
/// after its stem, and put the whole path in `scope_path`, `occurrence.file_path` and every
/// observation's `details`.
pub const HOSTILE_FILE: &str = "src/<img src=x onerror=alert(1)>.ts";

/// An import specifier that does not resolve, so its name becomes an `unresolved` entity's.
pub const HOSTILE_SPECIFIER: &str = "./</script><svg onload=alert(2)>";

/// The literal payload the tests look for in decoded JSON.
pub const PAYLOAD: &str = "<img src=x onerror=alert(1)>";

// ---- fixtures ------------------------------------------------------------------------------

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    let mut entries: Vec<_> = std::fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name() == ".nerve" {
            continue;
        }
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// Copy a committed fixture into a temporary directory. The fixture itself is never touched.
pub fn fixture_copy(name: &str) -> (tempfile::TempDir, PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&source, &root);
    (dir, root)
}

/// Write a file, creating parents.
pub fn write(root: &Path, rel_path: &str, contents: &str) {
    let path = root.join(rel_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Add the hostile file to a repository before it is indexed.
pub fn add_hostile_content(root: &Path) {
    write(
        root,
        HOSTILE_FILE,
        &format!(
            "import {{ thing }} from '{HOSTILE_SPECIFIER}';\n\
             export function payloadCarrier() {{ return thing; }}\n"
        ),
    );
}

/// Initialize and index a repository in place.
pub fn index(root: &Path) {
    nerve_index::init_with_project_id(root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(root).unwrap();
}

/// A copy of `ts-resolution`, plus hostile content, indexed and served.
pub fn served() -> (tempfile::TempDir, PathBuf, Session) {
    let (dir, root) = fixture_copy("ts-resolution");
    add_hostile_content(&root);
    index(&root);
    let session = Session::start(&root);
    (dir, root, session)
}

// ---- cross-repository contract fixtures --------------------------------------------------------

/// A display name that is legal to register and is also an XSS payload.
///
/// `display_name` is untrusted repository content on exactly T7's terms — a neighbour is a checkout
/// that may have been cloned from anywhere — so the contract surfaces have to carry it inside the
/// envelope like any other repository string. It embeds [`PAYLOAD`] so the existing leak probe's
/// marker works unchanged.
pub const HOSTILE_DISPLAY_NAME: &str = "<img src=x onerror=alert(1)> neighbour";

/// Six repositories, five registered in the first, with the contract links scanned.
pub struct ContractWorld {
    /// Kept so the temporary directory outlives the roots.
    pub dir: tempfile::TempDir,
    /// The repository the questions are asked of.
    pub host: PathBuf,
    /// The neighbour registered as `pkg-map`, and the one C2 reaches a **file entity** inside.
    ///
    /// Its database must be byte-identical after every read.
    pub map: PathBuf,
}

/// The neighbours this world registers, in the order they are registered.
///
/// `pkg-unregistered` is deliberately absent from the tree as well as from the registry, so a
/// declaration naming it resolves to nothing — which is what keeps "declared and unregistered"
/// from being confused with "not declared".
pub const CONTRACT_WORLD_NEIGHBOURS: [&str; 5] =
    ["pkg-map", "pkg-string", "pkg-legacy", "twin-a", "twin-b"];

/// Links this world records once the neighbours are registered and the manifests are scanned.
///
/// Asserted where the world is built, so a fixture change that quietly halved it fails here rather
/// than in whichever test happened to count.
pub const CONTRACT_WORLD_LINKS: usize = 15;

/// Build `fixtures/contracts-exports` as six separate repositories, register five of them in the
/// sixth, and scan.
///
/// **Each repository gets its own `project_id`**, because `repo_id` derives from it and resolution
/// is *by repository id*: two checkouts sharing an identity would make every resolution assertion
/// pass for the wrong reason. That habit is inherited from `crates/nerve-index/tests/contracts.rs`.
///
/// This fixture rather than `contracts-npm`, because it is the only one that produces a **C2**
/// link: an import specifier resolved through a neighbour's own export map to a file entity inside
/// it. That is the only link with a non-null target snapshot, and a target snapshot is what the
/// whole surface exists to render — a link whose snapshot fields were all null would let a renderer
/// omit them and still pass.
pub fn contract_world(display_name: &str) -> ContractWorld {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/contracts-exports");
    let dir = tempfile::tempdir().unwrap();
    let host = dir.path().join("host");
    let build = |name: &str, seed: usize| -> PathBuf {
        let root = dir.path().join(name);
        copy_tree(&fixtures.join(name), &root);
        nerve_index::init_with_project_id(
            &root,
            Some(&format!("{:032x}", 0xc0000000u64 + seed as u64)),
        )
        .unwrap();
        nerve_index::index_repository(&root).unwrap();
        root
    };
    let mut roots = Vec::new();
    for (offset, name) in CONTRACT_WORLD_NEIGHBOURS.into_iter().enumerate() {
        roots.push((name, build(name, offset + 1)));
    }
    // A real, indexed, adjacent Nerve repository that is deliberately **not** registered, so that
    // "a package a manifest names" and "a package Nerve may link to" stay distinguishable on this
    // surface as well as in the precision fixture. Nothing auto-registers it.
    build("pkg-unregistered", CONTRACT_WORLD_NEIGHBOURS.len() + 1);
    copy_tree(&fixtures.join("host"), &host);
    nerve_index::init_with_project_id(&host, Some("000000000000000000000000c0000000")).unwrap();
    nerve_index::index_repository(&host).unwrap();

    let conn = nerve_store::open(&nerve_index::config::db_path(&host)).unwrap();
    let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    for (name, root) in &roots {
        // Only the first neighbour carries the caller's display name. One is enough to prove the
        // envelope holds, and giving every entry the same string would make a leak indistinguishable
        // from a different entry's legitimate value.
        let display = if *name == CONTRACT_WORLD_NEIGHBOURS[0] {
            display_name
        } else {
            name
        };
        match nerve_index::add_registry_target(&conn, &repo_id, root, Some(name), Some(display))
            .unwrap()
        {
            nerve_index::RegistryOutcome::Done(_) => {}
            nerve_index::RegistryOutcome::Refused(reason) => {
                panic!("the neighbour {name} was refused: {reason}")
            }
        }
    }
    match nerve_index::scan_contracts(&conn, &repo_id, &host).unwrap() {
        nerve_index::ScanOutcome::Done(scan) => assert_eq!(
            scan.inserted(),
            CONTRACT_WORLD_LINKS,
            "the fixture no longer records the links this world is built on"
        ),
        nerve_index::ScanOutcome::Refused(reason) => panic!("the scan refused: {reason}"),
    }
    drop(conn);

    let map = roots[0].1.clone();
    ContractWorld { dir, host, map }
}

/// A contract world with a server already running over the host repository.
pub fn served_contracts(display_name: &str) -> (ContractWorld, Session) {
    let world = contract_world(display_name);
    let session = Session::start(&world.host);
    (world, session)
}

/// A repository with an index and **no** registry at all.
///
/// "Nobody has registered a neighbour" and "a neighbour was registered and nothing declares it" are
/// different answers, and a surface that rendered the first as the second would be reporting an
/// absence as a finding.
pub fn served_without_contracts() -> (tempfile::TempDir, PathBuf, Session) {
    let (dir, root) = fixture_copy("ts-basic");
    index(&root);
    let session = Session::start(&root);
    (dir, root, session)
}

/// BLAKE3 of a file, so "unchanged" is a hash comparison rather than a length comparison.
pub fn digest(path: &Path) -> String {
    nerve_core::ids::content_hash(&std::fs::read(path).unwrap())
}

// ---- history fixtures ------------------------------------------------------------------------

/// A history fixture, copied and turned back into a repository.
///
/// The fixtures ship their git directory as `gitdir/` so that `cargo package`, `git add` and every
/// tool that special-cases `.git` leave it alone. Renaming it on the copy is what makes it a
/// repository again; the fixture itself is never touched.
pub fn history_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = fixture_copy(name);
    std::fs::rename(root.join("gitdir"), root.join(".git"))
        .expect("the fixture must carry a gitdir/");
    (dir, root)
}

/// The fixture's own `inventory.json`, whose every value is **Git's** answer rather than Nerve's.
///
/// Assertions are written against this rather than against numbers typed into a test, so a
/// disagreement is a Nerve defect rather than a stale expectation.
pub fn history_inventory(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .join("inventory.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("the inventory must be readable"))
        .expect("the inventory must parse")
}

/// Every distinct path Git's own inventory says this fixture's commits changed.
pub fn inventory_changed_paths(name: &str) -> Vec<String> {
    let inventory = history_inventory(name);
    let mut paths = std::collections::BTreeSet::new();
    for commit in inventory["commits"].as_array().unwrap() {
        for change in commit["changes"].as_array().unwrap_or(&Vec::new()) {
            paths.insert(change["path"].as_str().unwrap().to_string());
        }
    }
    paths.into_iter().collect()
}

/// A history fixture, indexed, with its history ingested, and served.
///
/// The index is required because `nerve serve` refuses a directory that has none. The ingest is a
/// separate step for the reason `nerve history sync` is a separate command: history resolves
/// nothing against the graph, so the two are independent facts and the endpoints must be tested
/// against a repository that has one, both or neither.
pub fn served_history(name: &str) -> (tempfile::TempDir, PathBuf, Session) {
    let (dir, root) = history_fixture(name);
    index(&root);
    nerve_index::ingest_history(&root, &nerve_index::HistoryOptions::default())
        .expect("the fixture's history must ingest");
    let session = Session::start(&root);
    (dir, root, session)
}

/// A history fixture, indexed, with its history ingested, and **not** served.
///
/// The MCP surface speaks stdio and binds nothing, so it takes the repository directly. Starting a
/// server for it would test the HTTP guard a second time and the tool surface not at all.
pub fn history_repository(name: &str) -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = history_fixture(name);
    index(&root);
    nerve_index::ingest_history(&root, &nerve_index::HistoryOptions::default())
        .expect("the fixture's history must ingest");
    (dir, root)
}

/// A history fixture, indexed, with **no** history ingested and no server.
///
/// "History has never been read here" is one of the four states the historical model requires to
/// stay distinct from "read, and nothing found", and the MCP surface has to keep them apart too.
pub fn history_repository_without_history(name: &str) -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = history_fixture(name);
    index(&root);
    (dir, root)
}

/// A history fixture, indexed and served, with **no** history ingested.
///
/// "History has never been read here" is one of the four states the historical model requires to
/// stay distinct from "read, and nothing found", so it needs a repository of its own.
pub fn served_without_history(name: &str) -> (tempfile::TempDir, PathBuf, Session) {
    let (dir, root) = history_fixture(name);
    index(&root);
    let session = Session::start(&root);
    (dir, root, session)
}

// ---- server session ------------------------------------------------------------------------

/// A running server plus the token needed to talk to it.
pub struct Session {
    server: Option<RunningServer>,
    address: SocketAddr,
    token: String,
}

impl Session {
    /// Start a server on an ephemeral loopback port.
    pub fn start(root: &Path) -> Session {
        let server = nerve_server::serve(ServeConfig::new(root)).expect("server must bind");
        Session {
            address: server.address(),
            token: server.token().as_str().to_string(),
            server: Some(server),
        }
    }

    /// The bound address.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The session token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The correct `Host` header value.
    pub fn host(&self) -> String {
        self.address.to_string()
    }

    /// The correct `Origin` header value.
    pub fn origin(&self) -> String {
        format!("http://{}", self.address)
    }

    /// A `GET` carrying the correct host and a valid token.
    pub fn get(&self, target: &str) -> HttpResponse {
        self.raw(
            "GET",
            target,
            &[
                ("Host", &self.host()),
                (nerve_server::token::TOKEN_HEADER, &self.token),
            ],
        )
    }

    /// A `GET` whose JSON body must have parsed and reported success.
    pub fn json(&self, target: &str) -> serde_json::Value {
        let response = self.get(target);
        assert_eq!(response.status, 200, "{target}\n{}", response.body);
        let value = response.parse_json();
        assert_eq!(value["ok"], true, "{target}\n{}", response.body);
        value
    }

    /// A fully controlled request. Nothing is added that the caller did not ask for.
    pub fn raw(&self, method: &str, target: &str, headers: &[(&str, &str)]) -> HttpResponse {
        request(self.address, method, target, headers)
    }

    /// Stop the server and wait for its workers.
    pub fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.shutdown_and_join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- raw HTTP ------------------------------------------------------------------------------

/// A parsed response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// Status code from the status line.
    pub status: u16,
    /// Headers, with field names lower-cased.
    pub headers: Vec<(String, String)>,
    /// Body as received.
    pub body: String,
    /// The complete response, headers included, exactly as it arrived.
    pub raw: String,
}

impl HttpResponse {
    /// One header value, by lower-case name.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, value)| value.as_str())
    }

    /// The body, parsed as JSON.
    pub fn parse_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("body is not JSON ({err}): {}", self.body))
    }
}

/// Send a request with exactly the headers given, plus `Connection: close`.
pub fn request(
    address: SocketAddr,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
) -> HttpResponse {
    let mut stream = TcpStream::connect(address).expect("server must accept a connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(20)))
        .unwrap();

    let mut wire = format!("{method} {target} HTTP/1.1\r\n");
    for (field, value) in headers {
        wire.push_str(&format!("{field}: {value}\r\n"));
    }
    wire.push_str("Connection: close\r\n\r\n");
    stream.write_all(wire.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    parse_response(&bytes)
}

/// Send bytes verbatim, without composing a request. For malformed-input tests.
///
/// The read timeout is short on purpose: a truncated request is one the server is entitled to
/// leave unanswered while it waits for the rest, so "no reply" is a pass, and waiting twenty
/// seconds to establish that would make the security suite unusable.
pub fn send_raw(address: SocketAddr, wire: &[u8]) -> Option<HttpResponse> {
    let mut stream = TcpStream::connect(address).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(1_500)))
        .unwrap();
    stream.write_all(wire).ok()?;
    stream.flush().ok()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(parse_response(&bytes))
}

fn parse_response(bytes: &[u8]) -> HttpResponse {
    let raw = String::from_utf8_lossy(bytes).to_string();
    let (head, body) = match raw.split_once("\r\n\r\n") {
        Some((head, body)) => (head, body.to_string()),
        None => (raw.as_str(), String::new()),
    };
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("no status in {status_line:?}"));
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(field, value)| (field.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    HttpResponse {
        status,
        headers,
        body,
        raw,
    }
}
