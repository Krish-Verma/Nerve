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
