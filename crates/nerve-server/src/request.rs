//! Splitting a request target into a path and parameters.
//!
//! The request target is attacker-controlled and arrives before any check has run, so it is
//! parsed defensively: bounded length, bounded parameter count, strict percent-decoding, and no
//! filesystem semantics anywhere. A path is only ever compared against a fixed table of routes;
//! it is never joined onto a directory. That is why traversal in the *route* is a non-issue —
//! the only path that ever reaches the filesystem is the `path` parameter of the source
//! endpoint, and that one goes through `nerve-index`'s `canonical_child` choke point.

use std::collections::BTreeMap;

/// Longest request target this server will parse. Anything longer is refused with 414.
pub const MAX_TARGET_BYTES: usize = 8 * 1024;

/// Largest number of query parameters accepted. More than this is a probe, not a query.
pub const MAX_PARAMETERS: usize = 32;

/// Why a request target was not parseable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    /// Longer than [`MAX_TARGET_BYTES`].
    TooLong,
    /// More than [`MAX_PARAMETERS`] parameters.
    TooManyParameters,
    /// Percent-encoding was malformed, or decoded to something that is not UTF-8.
    Malformed,
}

/// A parsed request target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The path, percent-decoded, always starting with `/`.
    pub path: String,
    /// Query parameters, last value wins for a repeated key.
    pub parameters: BTreeMap<String, String>,
}

impl Target {
    /// Parse a raw request target such as `/api/search?q=area&limit=5`.
    pub fn parse(raw: &str) -> Result<Target, TargetError> {
        if raw.len() > MAX_TARGET_BYTES {
            return Err(TargetError::TooLong);
        }
        let (path_part, query_part) = match raw.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (raw, None),
        };
        // A fragment is never sent by a conformant client; if one arrives, drop it rather than
        // letting `#` become part of a route name.
        let path_part = path_part.split('#').next().unwrap_or("");
        let path = decode(path_part)?;
        if !path.starts_with('/') {
            return Err(TargetError::Malformed);
        }

        let mut parameters = BTreeMap::new();
        if let Some(query) = query_part {
            let query = query.split('#').next().unwrap_or("");
            for (index, pair) in query.split('&').filter(|p| !p.is_empty()).enumerate() {
                if index >= MAX_PARAMETERS {
                    return Err(TargetError::TooManyParameters);
                }
                let (key, value) = match pair.split_once('=') {
                    Some((key, value)) => (key, value),
                    None => (pair, ""),
                };
                parameters.insert(decode(key)?, decode(value)?);
            }
        }
        Ok(Target { path, parameters })
    }

    /// A parameter as text, absent when it was not supplied or was empty.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.parameters
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// A parameter parsed as `usize` and clamped to `max`.
    ///
    /// A value that is not a number is an error rather than a silent default: a surface that
    /// quietly reinterprets `limit=abc` as 20 teaches its caller nothing.
    pub fn bounded(&self, key: &str, default: usize, max: usize) -> Result<usize, &'static str> {
        match self.get(key) {
            None => Ok(default.min(max)),
            Some(text) => match text.parse::<usize>() {
                Ok(value) if value >= 1 => Ok(value.min(max)),
                _ => Err(key_error(key)),
            },
        }
    }

    /// A parameter parsed as `usize`, allowing zero, clamped to `max`.
    pub fn bounded_from_zero(
        &self,
        key: &str,
        default: usize,
        max: usize,
    ) -> Result<usize, &'static str> {
        match self.get(key) {
            None => Ok(default.min(max)),
            Some(text) => match text.parse::<usize>() {
                Ok(value) => Ok(value.min(max)),
                Err(_) => Err(key_error(key)),
            },
        }
    }

    /// A parameter read as a boolean flag: `1` or `true` is set, absent or anything else is not.
    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some("1") | Some("true"))
    }

    /// A repeatable parameter supplied as a comma-separated list.
    pub fn list(&self, key: &str) -> Vec<&str> {
        match self.get(key) {
            None => Vec::new(),
            Some(text) => text.split(',').filter(|item| !item.is_empty()).collect(),
        }
    }
}

fn key_error(key: &str) -> &'static str {
    // A `&'static str` keeps the error allocation-free; the set of keys is closed.
    match key {
        "limit" => "limit must be a positive integer",
        "depth" => "depth must be a positive integer",
        "max_depth" => "max_depth must be a positive integer",
        "max_nodes" => "max_nodes must be a positive integer",
        "offset" => "offset must be a non-negative integer",
        "start_line" => "start_line must be a positive integer",
        "end_line" => "end_line must be a positive integer",
        _ => "parameter must be an integer",
    }
}

/// Percent-decode one component, rejecting anything malformed.
///
/// `+` is decoded to a space, matching how a browser encodes a form-style query string.
fn decode(raw: &str) -> Result<String, TargetError> {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(TargetError::Malformed);
                }
                let high = hex_value(bytes[index + 1]).ok_or(TargetError::Malformed)?;
                let low = hex_value(bytes[index + 2]).ok_or(TargetError::Malformed)?;
                out.push(high << 4 | low);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    // A NUL survives percent-decoding and would otherwise reach a path comparison. Refuse it
    // here rather than relying on every downstream check to notice.
    if out.contains(&0) {
        return Err(TargetError::Malformed);
    }
    String::from_utf8(out).map_err(|_| TargetError::Malformed)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_has_no_parameters() {
        let target = Target::parse("/api/overview").unwrap();
        assert_eq!(target.path, "/api/overview");
        assert!(target.parameters.is_empty());
    }

    #[test]
    fn parameters_are_percent_decoded() {
        let target = Target::parse("/api/search?q=get%20area&kind=method").unwrap();
        assert_eq!(target.get("q"), Some("get area"));
        assert_eq!(target.get("kind"), Some("method"));
        assert_eq!(target.get("missing"), None);
    }

    #[test]
    fn a_selector_survives_encoding_intact() {
        let target = Target::parse("/api/entity?selector=src%2Fshapes.ts%23Circle.area").unwrap();
        assert_eq!(target.get("selector"), Some("src/shapes.ts#Circle.area"));
    }

    #[test]
    fn a_markup_payload_decodes_to_itself_without_interpretation() {
        let target =
            Target::parse("/api/source?path=src%2F%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E.ts")
                .unwrap();
        assert_eq!(
            target.get("path"),
            Some("src/<img src=x onerror=alert(1)>.ts")
        );
    }

    #[test]
    fn malformed_encoding_is_an_error_not_a_guess() {
        for raw in ["/a?x=%", "/a?x=%zz", "/a?x=%2", "/a?x=%ff%fe", "/a?x=%00"] {
            assert_eq!(Target::parse(raw), Err(TargetError::Malformed), "{raw}");
        }
    }

    #[test]
    fn an_oversized_target_is_refused_before_it_is_parsed() {
        let raw = format!("/api/search?q={}", "a".repeat(MAX_TARGET_BYTES));
        assert_eq!(Target::parse(&raw), Err(TargetError::TooLong));
    }

    #[test]
    fn too_many_parameters_is_refused() {
        let query: Vec<String> = (0..MAX_PARAMETERS + 1).map(|i| format!("k{i}=1")).collect();
        assert_eq!(
            Target::parse(&format!("/a?{}", query.join("&"))),
            Err(TargetError::TooManyParameters)
        );
    }

    #[test]
    fn bounds_clamp_rather_than_trusting_the_caller() {
        let target = Target::parse("/a?limit=100000&depth=abc").unwrap();
        assert_eq!(target.bounded("limit", 20, 200), Ok(200));
        assert!(target.bounded("depth", 1, 4).is_err());
        assert_eq!(target.bounded("absent", 20, 200), Ok(20));
        assert!(Target::parse("/a?limit=0")
            .unwrap()
            .bounded("limit", 1, 5)
            .is_err());
        assert_eq!(
            Target::parse("/a?offset=0")
                .unwrap()
                .bounded_from_zero("offset", 0, 100),
            Ok(0)
        );
    }

    #[test]
    fn flags_and_lists_read_the_documented_spellings() {
        let target = Target::parse("/a?resolved_only=true&relation=CALLS,DEFINES").unwrap();
        assert!(target.flag("resolved_only"));
        assert!(!target.flag("absent"));
        assert_eq!(target.list("relation"), vec!["CALLS", "DEFINES"]);
        assert!(target.list("absent").is_empty());
    }

    #[test]
    fn a_fragment_never_becomes_part_of_a_route() {
        let target = Target::parse("/api/overview#/entity").unwrap();
        assert_eq!(target.path, "/api/overview");
    }
}
