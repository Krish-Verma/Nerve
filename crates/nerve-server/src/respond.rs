//! Turning an answer into bytes, safely (THREAT-MODEL T5).
//!
//! Everything this server serves that came from the repository is attacker-controlled:
//! `<img src=x onerror=alert(1)>.ts` is a legal file name, and Nerve indexes it, names an entity
//! after it, and will be asked to display it. Two independent controls apply.
//!
//! **Encoding.** Repository strings only ever leave as JSON string values, produced by
//! `serde_json`, never by string concatenation into markup. On top of that, [`to_json_bytes`]
//! escapes `<`, `>`, `&`, U+2028 and U+2029 as `\uXXXX`. None of those is JSON syntax, so the
//! value a parser sees is unchanged — but the bytes on the wire then contain no character that
//! can open a tag, close a script, or terminate a JavaScript line. That makes the response inert
//! even if a future consumer does something careless with it, and it is trivially testable: the
//! served bytes contain no `<` at all.
//!
//! **Headers.** Every response, including error responses, carries a `Content-Security-Policy`
//! with no `unsafe-inline` and no remote origin, plus `nosniff` so a browser cannot be talked
//! into treating `application/json` as HTML. There is no `Access-Control-Allow-Origin` header
//! anywhere in this crate.

use tiny_http::{Header, Response, StatusCode};

/// The policy every response carries.
///
/// `default-src 'none'` is the base: nothing is allowed unless it is named. Scripts and styles
/// come from this origin only — no `unsafe-inline`, so an injected `<script>` or `onerror=`
/// attribute cannot execute even if one reached the document. `connect-src 'self'` keeps the
/// page from exfiltrating anything it read. `frame-ancestors 'none'` stops framing, and
/// `form-action 'none'` stops a submitted form from carrying the session token anywhere.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; \
     script-src 'self'; \
     style-src 'self'; \
     img-src 'self' data:; \
     font-src 'self'; \
     connect-src 'self'; \
     base-uri 'none'; \
     form-action 'none'; \
     frame-ancestors 'none'; \
     object-src 'none'";

/// Headers applied to every response this server produces, in a fixed order.
///
/// `Cache-Control: no-store` matters more than it looks: a cached response would outlive the
/// session token that authorised it.
pub const SECURITY_HEADERS: [(&str, &str); 6] = [
    ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
    ("Referrer-Policy", "no-referrer"),
    ("Cache-Control", "no-store"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
];

/// Serialize a JSON value with every character that could open markup escaped.
///
/// `<`, `>` and `&` are not JSON syntax, so replacing them with their `\uXXXX` forms produces a
/// document that parses to exactly the same value. U+2028 and U+2029 are legal in a JSON string
/// but terminate a line in JavaScript source, which is the classic JSONP break; they go too.
pub fn to_json_bytes(value: &serde_json::Value) -> Vec<u8> {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out.into_bytes()
}

fn header(field: &str, value: &str) -> Option<Header> {
    Header::from_bytes(field.as_bytes(), value.as_bytes()).ok()
}

/// Attach the security headers and a content type to a byte body.
///
/// Chunked transfer encoding is disabled: every body here is already fully in memory, so a
/// `Content-Length` is always available, and a framing the client cannot mis-parse is worth more
/// than streaming a response that is at most a few hundred kilobytes.
fn build(status: u16, content_type: &str, body: Vec<u8>) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_chunked_threshold(usize::MAX);
    if let Some(header) = header("Content-Type", content_type) {
        response.add_header(header);
    }
    for (field, value) in SECURITY_HEADERS {
        if let Some(header) = header(field, value) {
            response.add_header(header);
        }
    }
    response
}

/// A JSON response.
pub fn json(status: u16, value: &serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    build(
        status,
        "application/json; charset=utf-8",
        to_json_bytes(value),
    )
}

/// A static asset response.
pub fn asset(content_type: &str, body: &'static [u8]) -> Response<std::io::Cursor<Vec<u8>>> {
    build(200, content_type, body.to_vec())
}

/// The one shape every failure takes.
///
/// `detail` is frequently something the caller sent, so it goes through the same hardened
/// encoder as everything else rather than being formatted into a message.
pub fn error(
    status: u16,
    code: &str,
    message: &str,
    detail: serde_json::Value,
) -> Response<std::io::Cursor<Vec<u8>>> {
    json(
        status,
        &serde_json::json!({
            "ok": false,
            "status": status,
            "error": { "code": code, "message": message, "detail": detail },
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn markup_characters_never_reach_the_wire() {
        let payload = "<img src=x onerror=alert(1)>&</script>";
        let bytes = to_json_bytes(&json!({ "name": payload }));
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains('<'), "{text}");
        assert!(!text.contains('>'), "{text}");
        assert!(!text.contains('&'), "{text}");
        assert!(text.contains("\\u003c"), "{text}");
    }

    #[test]
    fn escaping_does_not_change_the_parsed_value() {
        let payload = "<img src=x onerror=alert(1)>&\u{2028}\u{2029}";
        let value = json!({ "name": payload, "nested": [ { "path": payload } ] });
        let text = String::from_utf8(to_json_bytes(&value)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, value);
        assert_eq!(parsed["name"].as_str().unwrap(), payload);
    }

    #[test]
    fn javascript_line_terminators_are_escaped() {
        let text = String::from_utf8(to_json_bytes(&json!("a\u{2028}b\u{2029}c"))).unwrap();
        assert!(!text.contains('\u{2028}'));
        assert!(!text.contains('\u{2029}'));
        assert_eq!(text, "\"a\\u2028b\\u2029c\"");
    }

    #[test]
    fn the_policy_forbids_inline_and_remote_code() {
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-inline"));
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-eval"));
        assert!(!CONTENT_SECURITY_POLICY.contains("http://"));
        assert!(!CONTENT_SECURITY_POLICY.contains("https://"));
        assert!(!CONTENT_SECURITY_POLICY.contains('*'));
        assert!(CONTENT_SECURITY_POLICY.starts_with("default-src 'none'"));
    }

    #[test]
    fn no_response_helper_emits_a_cors_header() {
        for (field, _) in SECURITY_HEADERS {
            assert!(!field.to_ascii_lowercase().starts_with("access-control"));
        }
    }
}
