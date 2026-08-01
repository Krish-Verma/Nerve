//! Who is allowed to ask (THREAT-MODEL T4).
//!
//! Three checks, all of them required, none of them sufficient alone:
//!
//! 1. **`Host` must be the bound loopback address.** This is the DNS-rebinding defence. An
//!    attacker who controls `evil.test` can point it at `127.0.0.1`; the connection then really
//!    does arrive on loopback and a naive "is this localhost?" check passes. What the attacker
//!    cannot change is the `Host` header the browser sends, which carries *their* name. Refusing
//!    every `Host` that is not the address we bound closes it.
//! 2. **`Origin`, when present, must be exactly our own origin.** Browsers omit `Origin` on
//!    same-origin `GET`, and send it on cross-origin requests, so "absent or exactly ours" is
//!    the precise rule. There is deliberately **no** CORS response header anywhere in this
//!    crate: nothing is opted in, so nothing is allowed to read a response cross-origin.
//! 3. **The session token must be present and correct.** This is the control that survives an
//!    attacker who can forge headers from a non-browser client, where 1 and 2 are worthless.
//!
//! The API is read-only besides, so even a request that somehow passed all three could not
//! change anything — defence in depth, not the primary control.

use std::net::SocketAddr;

use crate::token::{SessionToken, TOKEN_HEADER};

/// Why a request was refused, and with what status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// `Host` was absent or named something other than the bound address.
    BadHost,
    /// `Origin` was present and was not this server's own origin.
    BadOrigin,
    /// No session token was supplied.
    MissingToken,
    /// A session token was supplied and it was wrong.
    BadToken,
}

impl Rejection {
    /// HTTP status this rejection is reported with.
    ///
    /// A missing credential is 401; a supplied-but-wrong credential and a forged host are 403,
    /// because retrying with different credentials is not going to help.
    pub fn status(self) -> u16 {
        match self {
            Rejection::MissingToken => 401,
            Rejection::BadHost | Rejection::BadOrigin | Rejection::BadToken => 403,
        }
    }

    /// Stable machine-readable code, part of the API contract.
    pub fn code(self) -> &'static str {
        match self {
            Rejection::BadHost => "host_not_allowed",
            Rejection::BadOrigin => "origin_not_allowed",
            Rejection::MissingToken => "token_required",
            Rejection::BadToken => "token_invalid",
        }
    }

    /// Message shown to a human. Deliberately says nothing an attacker did not already supply.
    pub fn message(self) -> &'static str {
        match self {
            Rejection::BadHost => {
                "request Host is not the loopback address this server is bound to"
            }
            Rejection::BadOrigin => "cross-origin requests are refused",
            Rejection::MissingToken => {
                "a session token is required; use the URL nerve serve printed"
            }
            Rejection::BadToken => "session token is not valid for this server",
        }
    }
}

/// The values a request must carry to be answered at all.
#[derive(Debug)]
pub struct Guard {
    token: SessionToken,
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
}

impl Guard {
    /// Build the guard for a server bound to `address`.
    ///
    /// Only the literal loopback IPv4 address is accepted, in the two spellings HTTP allows for
    /// it. `localhost` is **not** accepted: it is a name, names are resolved by a resolver we do
    /// not control, and the whole point of check 1 is to stop trusting names.
    pub fn new(token: SessionToken, address: SocketAddr) -> Guard {
        let port = address.port();
        let ip = address.ip().to_string();
        Guard {
            token,
            allowed_hosts: vec![format!("{ip}:{port}")],
            allowed_origins: vec![format!("http://{ip}:{port}")],
        }
    }

    /// The token this guard was built with.
    pub fn token(&self) -> &SessionToken {
        &self.token
    }

    /// Apply all three checks, in the order that leaks the least.
    ///
    /// `host` and `origin` are the raw header values, `supplied_token` whatever the request
    /// carried in [`TOKEN_HEADER`] or the token query parameter.
    pub fn check(
        &self,
        host: Option<&str>,
        origin: Option<&str>,
        supplied_token: Option<&str>,
    ) -> Result<(), Rejection> {
        let Some(host) = host else {
            return Err(Rejection::BadHost);
        };
        if !self.allowed_hosts.iter().any(|allowed| allowed == host) {
            return Err(Rejection::BadHost);
        }
        // `null` is what a sandboxed iframe or a `file://` page sends. It is not our origin.
        if let Some(origin) = origin {
            if !self.allowed_origins.iter().any(|allowed| allowed == origin) {
                return Err(Rejection::BadOrigin);
            }
        }
        match supplied_token {
            None => Err(Rejection::MissingToken),
            Some(candidate) if self.token.matches(candidate) => Ok(()),
            Some(_) => Err(Rejection::BadToken),
        }
    }
}

/// Name of the header a client supplies the token in, re-exported for callers building requests.
pub const TOKEN_HEADER_NAME: &str = TOKEN_HEADER;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn guard() -> Guard {
        Guard::new(
            SessionToken::from_hex(TOKEN),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 41234),
        )
    }

    #[test]
    fn the_happy_path_is_our_own_host_and_the_right_token() {
        let guard = guard();
        assert_eq!(
            guard.check(Some("127.0.0.1:41234"), None, Some(TOKEN)),
            Ok(())
        );
        assert_eq!(
            guard.check(
                Some("127.0.0.1:41234"),
                Some("http://127.0.0.1:41234"),
                Some(TOKEN)
            ),
            Ok(())
        );
    }

    #[test]
    fn a_rebound_dns_name_is_refused_even_though_it_reached_loopback() {
        let guard = guard();
        for host in [
            "evil.test",
            "evil.test:41234",
            "localhost:41234",
            "127.0.0.1",
            "127.0.0.1:41235",
            "[::1]:41234",
            "0.0.0.0:41234",
        ] {
            assert_eq!(
                guard.check(Some(host), None, Some(TOKEN)),
                Err(Rejection::BadHost),
                "{host}"
            );
        }
        assert_eq!(
            guard.check(None, None, Some(TOKEN)),
            Err(Rejection::BadHost)
        );
    }

    #[test]
    fn a_cross_origin_page_is_refused_before_its_token_is_even_considered() {
        let guard = guard();
        for origin in [
            "http://evil.test",
            "https://evil.test",
            "http://localhost:41234",
            "http://127.0.0.1:41235",
            "null",
            "",
        ] {
            assert_eq!(
                guard.check(Some("127.0.0.1:41234"), Some(origin), Some(TOKEN)),
                Err(Rejection::BadOrigin),
                "{origin}"
            );
        }
    }

    #[test]
    fn the_token_is_required_and_must_be_exact() {
        let guard = guard();
        assert_eq!(
            guard.check(Some("127.0.0.1:41234"), None, None),
            Err(Rejection::MissingToken)
        );
        for candidate in ["", "wrong", &TOKEN[..TOKEN.len() - 1], &format!("{TOKEN}0")] {
            assert_eq!(
                guard.check(Some("127.0.0.1:41234"), None, Some(candidate)),
                Err(Rejection::BadToken),
                "{candidate}"
            );
        }
    }

    #[test]
    fn statuses_and_codes_are_the_api_contract() {
        assert_eq!(Rejection::MissingToken.status(), 401);
        assert_eq!(Rejection::BadToken.status(), 403);
        assert_eq!(Rejection::BadHost.status(), 403);
        assert_eq!(Rejection::BadOrigin.status(), 403);
        assert_eq!(Rejection::BadHost.code(), "host_not_allowed");
        assert_eq!(Rejection::BadOrigin.code(), "origin_not_allowed");
    }
}
