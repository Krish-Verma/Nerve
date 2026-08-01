//! The per-session capability token (THREAT-MODEL T4).
//!
//! Binding to `127.0.0.1` is **not** an access control. Any web page the user visits can issue
//! `fetch('http://127.0.0.1:PORT/api/...')`, and loopback is reachable from the browser. The
//! control that actually separates "the user's own tab" from "some page the user happened to
//! open" is a secret the attacker's page cannot read: a token minted at `serve` start, printed
//! once to the terminal, and required on every request.
//!
//! Properties that make it a control rather than a decoration:
//!
//! - **256 bits from the operating system's CSPRNG.** Not a PID, not a timestamp, not a hash of
//!   the port. Guessing it is not a strategy.
//! - **Per session.** It is never written to disk and never reused across runs, so a token
//!   leaked in a shell history is dead as soon as the server stops.
//! - **Compared in constant time.** A byte-at-a-time comparison against a local attacker who
//!   can issue unlimited requests is a timing oracle, and this one is cheap to close.
//!
//! `nerve init` already reads `/dev/urandom` directly rather than adopting `rand`
//! (`third_party/LICENSES.md`, "Dependencies deliberately absent"). The same interface is used
//! here for the same reason.

use std::fmt;

/// Bytes of entropy behind a session token.
pub const TOKEN_BYTES: usize = 32;

/// Header a client supplies the token in.
pub const TOKEN_HEADER: &str = "X-Nerve-Token";

/// Query parameter a client may supply the token in instead.
///
/// The browser has to learn the token somehow, and the only channel a terminal has to a browser
/// is the URL. The page then reads it from `location` and sends the header on API calls. A token
/// in a URL is visible in shell history and in the browser's own history, which is why it is a
/// per-session secret with no persistence rather than a durable credential.
pub const TOKEN_QUERY: &str = "token";

/// A 256-bit session capability, held as its hex rendering.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken {
    hex: String,
}

impl SessionToken {
    /// Mint a fresh token from the operating system's randomness source.
    pub fn generate() -> std::io::Result<SessionToken> {
        let bytes = os_random_bytes()?;
        let mut hex = String::with_capacity(TOKEN_BYTES * 2);
        for byte in bytes {
            hex.push_str(&format!("{byte:02x}"));
        }
        Ok(SessionToken { hex })
    }

    /// Build a token from a known value. Tests only — never call this from product code.
    #[cfg(test)]
    pub fn from_hex(hex: &str) -> SessionToken {
        SessionToken {
            hex: hex.to_string(),
        }
    }

    /// The token as it appears in a header or URL.
    pub fn as_str(&self) -> &str {
        &self.hex
    }

    /// Whether `candidate` is this token, compared without an early exit.
    ///
    /// Length is compared first and non-secretly: the token's length is public, and a
    /// variable-length loop would leak nothing useful but complicate the constant-time claim.
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.hex.as_bytes();
        let supplied = candidate.as_bytes();
        if expected.len() != supplied.len() {
            return false;
        }
        let mut difference: u8 = 0;
        for (a, b) in expected.iter().zip(supplied.iter()) {
            difference |= a ^ b;
        }
        difference == 0
    }
}

/// Deliberately opaque: a token that prints itself into a log is not a secret.
impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(redacted)")
    }
}

#[cfg(unix)]
fn os_random_bytes() -> std::io::Result<[u8; TOKEN_BYTES]> {
    use std::io::Read;
    let mut buffer = [0u8; TOKEN_BYTES];
    let mut source = std::fs::File::open("/dev/urandom")?;
    source.read_exact(&mut buffer)?;
    Ok(buffer)
}

#[cfg(not(unix))]
fn os_random_bytes() -> std::io::Result<[u8; TOKEN_BYTES]> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no supported OS randomness source on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_256_bits_of_hex() {
        let token = SessionToken::generate().unwrap();
        assert_eq!(token.as_str().len(), TOKEN_BYTES * 2);
        assert!(token.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokens_do_not_repeat() {
        let a = SessionToken::generate().unwrap();
        let b = SessionToken::generate().unwrap();
        assert_ne!(a.as_str(), b.as_str());
    }

    #[test]
    fn comparison_accepts_only_the_exact_token() {
        let token = SessionToken::generate().unwrap();
        let value = token.as_str().to_string();
        assert!(token.matches(&value));
        assert!(!token.matches(""));
        assert!(!token.matches(&value[..value.len() - 1]));
        assert!(!token.matches(&format!("{value}a")));

        let mut flipped: Vec<char> = value.chars().collect();
        flipped[0] = if flipped[0] == 'a' { 'b' } else { 'a' };
        assert!(!token.matches(&flipped.into_iter().collect::<String>()));
    }

    #[test]
    fn a_token_never_prints_itself() {
        let token = SessionToken::generate().unwrap();
        let rendered = format!("{token:?}");
        assert_eq!(rendered, "SessionToken(redacted)");
        assert!(!rendered.contains(token.as_str()));
    }
}
