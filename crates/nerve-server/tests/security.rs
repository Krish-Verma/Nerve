//! The three blocking gates from `docs/THREAT-MODEL.md`, exercised against a running server.
//!
//! These are attack constructions, not shape assertions. Each one issues the request an
//! adversary would issue and checks that nothing came back.

mod common;

use std::path::Path;

use common::{HOSTILE_FILE, PAYLOAD};

// ---- T4 · cross-site request forgery and DNS rebinding --------------------------------------

/// The socket the operating system reports must be loopback. This is the property the rest of
/// T4 is defence in depth for.
#[test]
fn the_server_binds_loopback_only() {
    let (_dir, _root, session) = common::served();
    let address = session.address();
    assert!(address.ip().is_loopback(), "{address}");
    assert_eq!(address.ip().to_string(), "127.0.0.1");
    assert_ne!(address.ip().to_string(), "0.0.0.0");
}

/// Nothing that was read out of the repository is reachable without the session token.
///
/// Every advertised route is exercised, not a sample of them, so a route added later cannot be
/// added ungated without this failing. An unknown path is included too: it must also be refused
/// rather than 404, or an unauthorised caller could enumerate which routes exist.
#[test]
fn no_api_route_answers_without_a_token() {
    let (_dir, _root, session) = common::served();
    let mut targets: Vec<String> = nerve_server::router::ROUTES
        .iter()
        .map(|route| route.to_string())
        .collect();
    targets.push("/api/search?q=area".to_string());
    targets.push("/api/entity?selector=Circle".to_string());
    targets.push("/no/such/route".to_string());

    for target in targets {
        let response = session.raw("GET", &target, &[("Host", &session.host())]);
        assert_eq!(response.status, 401, "{target}: {}", response.body);
        assert_eq!(response.parse_json()["error"]["code"], "token_required");
        assert!(!response.body.contains("entities_total"), "{target}");
        assert!(!response.body.contains(PAYLOAD), "{target}");
    }
}

/// The interface's own files load without the token, and carry nothing that is not public.
///
/// This is a deliberate, narrow relaxation and it is pinned here so it cannot widen by accident.
/// A browser cannot attach a header to a `<script src>` or a `<link href>`, so requiring the
/// token on them would make the interface unloadable. What is served instead is build-constant:
/// identical in every copy of this binary, containing no repository content, no index content
/// and no session state. The API stays gated — see `no_api_route_answers_without_a_token`.
#[test]
fn the_interface_loads_without_a_token_and_carries_nothing_private() {
    let (_dir, root, session) = common::served();
    let root_path = root.to_string_lossy().to_string();

    for target in ["/", "/index.html", "/assets/nerve.css", "/assets/nerve.js"] {
        let response = session.raw("GET", target, &[("Host", &session.host())]);
        assert_eq!(response.status, 200, "{target}: {}", response.body);

        // The three things that would make serving these ungated a disclosure. Client-side field
        // names such as `entities_total` legitimately appear in the bundle — they are the shape
        // of the API, which is public — so what is asserted is the absence of *values*: this
        // session's token, this repository's content, and this repository's location on disk.
        assert!(
            !response.body.contains(session.token()),
            "{target} carries the session token"
        );
        assert!(
            !response.body.contains(PAYLOAD),
            "{target} carries repository content"
        );
        assert!(
            !response.body.contains(HOSTILE_FILE),
            "{target} carries an indexed path"
        );
        assert!(
            !response.body.contains(&root_path),
            "{target} carries the repository root"
        );
    }
}

/// Relaxing the token on the interface files did not relax `Host` or `Origin` on them.
///
/// The guard applies those two checks *before* the token, so they are still the DNS-rebinding
/// defence and the cross-origin refusal for every route on the server, assets included.
#[test]
fn the_interface_files_still_refuse_a_forged_host_or_a_foreign_origin() {
    let (_dir, _root, session) = common::served();
    for target in ["/", "/assets/nerve.js", "/assets/nerve.css"] {
        let rebound = session.raw("GET", target, &[("Host", "evil.test")]);
        assert_eq!(rebound.status, 403, "{target}: {}", rebound.body);
        assert_eq!(rebound.parse_json()["error"]["code"], "host_not_allowed");

        let foreign = session.raw(
            "GET",
            target,
            &[("Host", &session.host()), ("Origin", "http://evil.test")],
        );
        assert_eq!(foreign.status, 403, "{target}: {}", foreign.body);
        assert_eq!(foreign.parse_json()["error"]["code"], "origin_not_allowed");
    }
}

#[test]
fn a_request_with_the_wrong_token_is_refused() {
    let (_dir, _root, session) = common::served();
    let almost = {
        let mut token: Vec<char> = session.token().chars().collect();
        token[0] = if token[0] == 'a' { 'b' } else { 'a' };
        token.into_iter().collect::<String>()
    };
    for candidate in [
        "".to_string(),
        "wrong".to_string(),
        almost,
        format!("{}0", session.token()),
        session.token()[..session.token().len() - 1].to_string(),
    ] {
        let response = session.raw(
            "GET",
            "/api/overview",
            &[
                ("Host", &session.host()),
                (nerve_server::token::TOKEN_HEADER, &candidate),
            ],
        );
        assert!(
            response.status == 401 || response.status == 403,
            "{candidate:?} produced {}",
            response.status
        );
        assert!(!response.body.contains("entities_total"), "{candidate:?}");
    }
}

#[test]
fn the_right_token_is_accepted_in_the_header_or_the_query() {
    let (_dir, _root, session) = common::served();
    assert_eq!(session.get("/api/overview").status, 200);

    let via_query = session.raw(
        "GET",
        &format!("/api/overview?token={}", session.token()),
        &[("Host", &session.host())],
    );
    assert_eq!(via_query.status, 200, "{}", via_query.body);
    assert_eq!(via_query.parse_json()["ok"], true);
}

/// The DNS-rebinding construction. The connection genuinely arrives on loopback — an attacker
/// domain can be pointed there — so the only thing that distinguishes it is the `Host` header
/// the browser sends, which carries the attacker's name.
#[test]
fn a_host_header_that_is_not_the_bound_address_is_refused() {
    let (_dir, _root, session) = common::served();
    let port = session.address().port();
    for host in [
        "evil.test".to_string(),
        format!("evil.test:{port}"),
        format!("rebind.attacker.example:{port}"),
        format!("localhost:{port}"),
        "127.0.0.1".to_string(),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
        format!("0.0.0.0:{port}"),
        format!("[::1]:{port}"),
    ] {
        let response = session.raw(
            "GET",
            "/api/overview",
            &[
                ("Host", &host),
                (nerve_server::token::TOKEN_HEADER, session.token()),
            ],
        );
        assert_eq!(response.status, 403, "Host: {host} was not refused");
        assert_eq!(response.parse_json()["error"]["code"], "host_not_allowed");
        assert!(!response.body.contains("entities_total"), "{host}");
    }
}

#[test]
fn a_request_with_no_host_header_at_all_is_refused() {
    let (_dir, _root, session) = common::served();
    let response = session.raw(
        "GET",
        "/api/overview",
        &[(nerve_server::token::TOKEN_HEADER, session.token())],
    );
    assert_eq!(response.status, 403);
    assert_eq!(response.parse_json()["error"]["code"], "host_not_allowed");
}

#[test]
fn a_cross_origin_request_is_refused_even_with_a_valid_token() {
    let (_dir, _root, session) = common::served();
    let port = session.address().port();
    for origin in [
        "http://evil.test".to_string(),
        "https://evil.test".to_string(),
        format!("http://evil.test:{port}"),
        format!("http://localhost:{port}"),
        format!("https://127.0.0.1:{port}"),
        "null".to_string(),
    ] {
        let response = session.raw(
            "GET",
            "/api/overview",
            &[
                ("Host", &session.host()),
                ("Origin", &origin),
                (nerve_server::token::TOKEN_HEADER, session.token()),
            ],
        );
        assert_eq!(response.status, 403, "Origin: {origin} was not refused");
        assert_eq!(response.parse_json()["error"]["code"], "origin_not_allowed");
        assert!(!response.body.contains("entities_total"), "{origin}");
    }
}

#[test]
fn our_own_origin_is_accepted() {
    let (_dir, _root, session) = common::served();
    let response = session.raw(
        "GET",
        "/api/overview",
        &[
            ("Host", &session.host()),
            ("Origin", &session.origin()),
            (nerve_server::token::TOKEN_HEADER, session.token()),
        ],
    );
    assert_eq!(response.status, 200, "{}", response.body);
}

/// No CORS header is emitted anywhere. A response a cross-origin script cannot read is a
/// response it cannot exfiltrate, whatever else it manages.
#[test]
fn no_response_ever_carries_a_cors_header() {
    let (_dir, _root, session) = common::served();
    let responses = [
        session.get("/api/overview"),
        session.get("/api/search?q=area"),
        session.get("/"),
        session.get("/api/nope"),
        session.raw("GET", "/api/overview", &[("Host", "evil.test")]),
        session.raw("OPTIONS", "/api/overview", &[("Host", &session.host())]),
    ];
    for response in responses {
        for (field, _) in &response.headers {
            assert!(
                !field.starts_with("access-control"),
                "{field} in {}",
                response.raw
            );
        }
    }
}

/// Even a request that passed every check cannot mutate: nothing but `GET` is routed.
#[test]
fn no_method_but_get_is_served() {
    let (_dir, _root, session) = common::served();
    for method in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD"] {
        let response = session.raw(
            method,
            "/api/overview",
            &[
                ("Host", &session.host()),
                (nerve_server::token::TOKEN_HEADER, session.token()),
            ],
        );
        assert_eq!(response.status, 405, "{method} was served");
    }
}

// ---- T5 · stored XSS through repository content ---------------------------------------------

/// The fixture really does carry the payload, or the rest of this file proves nothing.
#[test]
fn the_hostile_fixture_reaches_the_graph() {
    let (_dir, root, session) = common::served();
    assert!(root.join(HOSTILE_FILE).is_file());

    let search = session.json("/api/search?q=payloadCarrier");
    assert!(
        search["count"].as_u64().unwrap() > 0,
        "{}",
        serde_json::to_string_pretty(&search).unwrap()
    );

    // The payload is in an entity name, not merely in a path we happened to echo back.
    let hostile = session.json(&format!("/api/entity?selector={}", urlencode(HOSTILE_FILE)));
    let name = hostile["entity"]["name"].as_str().unwrap();
    assert!(name.contains(PAYLOAD), "{name}");
}

/// The whole T5 claim in one assertion: no byte a browser could read as markup ever leaves.
#[test]
fn no_response_body_contains_a_raw_angle_bracket() {
    let (_dir, _root, session) = common::served();
    let module = urlencode(HOSTILE_FILE);
    for target in [
        "/api/overview".to_string(),
        "/api/search?q=payloadCarrier".to_string(),
        "/api/search?q=img".to_string(),
        format!("/api/entity?selector={module}"),
        format!("/api/neighbourhood?selector={module}&depth=2"),
        format!("/api/why?subject={module}"),
        format!("/api/source?path={module}"),
        "/api/unresolved".to_string(),
        "/api/partial-parses".to_string(),
    ] {
        let response = session.get(&target);
        assert_eq!(response.status, 200, "{target}: {}", response.body);
        assert!(
            !response.body.contains('<'),
            "{target} served a raw '<':\n{}",
            response.body
        );
        assert!(
            !response.body.contains('>'),
            "{target} served a raw '>':\n{}",
            response.body
        );
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8"),
            "{target}"
        );
    }
}

/// Escaping must not have quietly damaged the data. The payload survives a JSON round trip
/// exactly, which is what makes the escaping a rendering decision rather than a lossy filter.
#[test]
fn the_escaped_payload_still_decodes_to_the_original_bytes() {
    let (_dir, _root, session) = common::served();
    let response = session.get(&format!("/api/entity?selector={}", urlencode(HOSTILE_FILE)));
    assert!(response.body.contains("\\u003c"), "{}", response.body);
    let value = response.parse_json();
    assert_eq!(
        value["entity"]["scope_path"].as_str().unwrap(),
        HOSTILE_FILE
    );
    assert!(value["entity"]["name"]
        .as_str()
        .unwrap()
        .contains("<img src=x onerror=alert(1)>"));
}

/// An unresolved specifier is repository text too, and it is displayed as prominently as
/// anything else.
#[test]
fn a_hostile_unresolved_specifier_is_escaped() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/unresolved?limit=500");
    assert_eq!(response.status, 200);
    assert!(!response.body.contains('<'), "{}", response.body);

    let value = response.parse_json();
    let text = serde_json::to_string(&value).unwrap();
    assert!(
        text.contains("svg onload=alert(2)"),
        "the hostile specifier never reached the unresolved list: {text}"
    );
}

/// A refusal echoes what the caller sent. That echo is an injection point unless it goes
/// through the same encoder.
#[test]
fn an_error_message_quoting_the_caller_is_escaped_too() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/entity?selector=%3Cscript%3Ealert(1)%3C%2Fscript%3E");
    assert!(response.status >= 400, "{}", response.body);
    assert!(!response.body.contains('<'), "{}", response.body);
    assert!(!response.body.contains('>'), "{}", response.body);
    assert_eq!(
        response.header("content-type"),
        Some("application/json; charset=utf-8")
    );
}

#[test]
fn every_response_carries_the_security_headers() {
    let (_dir, _root, session) = common::served();
    let responses = [
        session.get("/api/overview"),
        session.get("/"),
        session.get("/assets/nerve.css"),
        session.get("/api/nope"),
        session.raw("GET", "/api/overview", &[("Host", "evil.test")]),
    ];
    for response in responses {
        let policy = response
            .header("content-security-policy")
            .unwrap_or_else(|| panic!("no CSP on:\n{}", response.raw));
        assert!(policy.starts_with("default-src 'none'"), "{policy}");
        assert!(!policy.contains("unsafe-inline"), "{policy}");
        assert!(!policy.contains("unsafe-eval"), "{policy}");
        assert!(!policy.contains("http://"), "{policy}");
        assert!(!policy.contains('*'), "{policy}");
        assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(response.header("x-frame-options"), Some("DENY"));
        assert_eq!(response.header("referrer-policy"), Some("no-referrer"));
        assert_eq!(response.header("cache-control"), Some("no-store"));
    }
}

/// The embedded page must not need the exception the policy refuses to grant.
#[test]
fn the_served_page_has_no_inline_script_or_style() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    // `script-src 'self'; style-src 'self'` with no `unsafe-inline` permits an external script
    // and an external stylesheet. What it forbids is anything the document itself carries: a
    // script body, a style block, a style attribute, an event handler.
    for fragment in response.body.split("<script").skip(1) {
        let (open, rest) = fragment
            .split_once('>')
            .expect("an unterminated <script tag");
        assert!(open.contains("src="), "a <script> with no src: {open}");
        let body = rest.split("</script>").next().unwrap_or("");
        assert!(body.trim().is_empty(), "an inline <script> body: {body}");
    }

    let html = response.body.to_ascii_lowercase();
    assert!(!html.contains("<style"), "{html}");
    assert!(!html.contains(" style="), "{html}");
    assert!(!html.contains(" onerror"), "{html}");
    assert!(!html.contains(" onload"), "{html}");
    assert!(!html.contains(" onclick"), "{html}");
    assert!(!html.contains("javascript:"), "{html}");
    // Nothing repository-derived, and nothing templated, is in the page at all.
    assert!(!response.body.contains(PAYLOAD));
    assert!(!response.body.contains(session.token()));
}

// ---- T6 · serving files outside the indexed root ---------------------------------------------

#[test]
fn the_source_endpoint_serves_an_indexed_file() {
    let (_dir, _root, session) = common::served();
    let value = session.json("/api/source?path=src/math.ts&start_line=1&end_line=5");
    assert_eq!(value["path"], "src/math.ts");
    assert_eq!(value["start_line"], 1);
    assert!(value["text"].as_str().unwrap().contains("export"));
    assert!(value["total_lines"].as_u64().unwrap() >= 1);
}

#[test]
fn the_source_endpoint_refuses_traversal_and_absolute_paths() {
    let (_dir, root, session) = common::served();
    let outside = root.parent().unwrap().join("outside.ts");
    std::fs::write(&outside, "export const secret = 'leaked';\n").unwrap();

    for path in [
        "../outside.ts".to_string(),
        "src/../../outside.ts".to_string(),
        "../../../../etc/passwd".to_string(),
        "/etc/passwd".to_string(),
        outside.to_string_lossy().to_string(),
        "src/./math.ts".to_string(),
    ] {
        let response = session.get(&format!("/api/source?path={}", urlencode(&path)));
        assert!(
            response.status == 403 || response.status == 404,
            "{path} produced {}: {}",
            response.status,
            response.body
        );
        assert!(!response.body.contains("secret"), "{path} leaked content");
        assert!(!response.body.contains("root:"), "{path} leaked content");
        assert!(response.parse_json()["text"].is_null(), "{path}");
    }
}

/// A path that *was* indexed and has since been replaced by a symlink pointing out of the
/// repository. This is the vector `crates/nerve-index/tests/safety.rs` constructs for the
/// indexer, applied to the serving path.
#[cfg(unix)]
#[test]
fn the_source_endpoint_refuses_a_symlink_escape_constructed_after_indexing() {
    let (_dir, root) = common::fixture_copy("ts-resolution");
    common::index(&root);

    let outside = root.parent().unwrap().join("outside.ts");
    std::fs::write(&outside, "export const leakedSecret = 'pwned';\n").unwrap();
    // src/math.ts is genuinely indexed, so the "is it in the index?" gate passes and the path
    // guard is the only thing standing between the request and a file outside the root.
    let indexed = root.join("src/math.ts");
    std::fs::remove_file(&indexed).unwrap();
    std::os::unix::fs::symlink(&outside, &indexed).unwrap();

    let session = common::Session::start(&root);
    let response = session.get("/api/source?path=src/math.ts");
    assert_eq!(response.status, 403, "{}", response.body);
    let value = response.parse_json();
    assert_eq!(value["error"]["code"], "path_refused");
    assert_eq!(value["error"]["detail"]["reason"], "refused");
    assert!(!response.body.contains("leakedSecret"), "{}", response.body);
    assert!(!response.body.contains("pwned"), "{}", response.body);
}

/// A symlinked parent directory. The file name looks ordinary; the escape is one level up.
#[cfg(unix)]
#[test]
fn the_source_endpoint_refuses_a_symlinked_parent_directory() {
    let (dir, root) = common::fixture_copy("ts-resolution");
    common::index(&root);

    let vendor = dir.path().join("vendor");
    std::fs::create_dir_all(&vendor).unwrap();
    std::fs::write(vendor.join("math.ts"), "export const leakedSecret = 1;\n").unwrap();
    std::fs::remove_dir_all(root.join("src")).unwrap();
    std::os::unix::fs::symlink(&vendor, root.join("src")).unwrap();

    let session = common::Session::start(&root);
    let response = session.get("/api/source?path=src/math.ts");
    assert_eq!(response.status, 403, "{}", response.body);
    assert_eq!(response.parse_json()["error"]["code"], "path_refused");
    assert!(!response.body.contains("leakedSecret"));
}

/// The deny-list is re-applied at serving time. The file was indexed before the pattern
/// existed; adding the pattern must take it out of reach immediately.
#[test]
fn the_source_endpoint_refuses_a_deny_listed_file() {
    let (_dir, root) = common::fixture_copy("ts-resolution");
    common::index(&root);

    let mut config = nerve_index::Config::load(&root).unwrap();
    config.security.extra_deny_patterns = vec!["math.ts".to_string()];
    config.save(&root).unwrap();

    let session = common::Session::start(&root);
    let response = session.get("/api/source?path=src/math.ts");
    assert_eq!(response.status, 403, "{}", response.body);
    assert_eq!(response.parse_json()["error"]["code"], "path_refused");
    assert!(response.parse_json()["text"].is_null());
}

/// A secret that was never indexed is refused before the filesystem is touched.
#[test]
fn the_source_endpoint_refuses_a_path_that_was_never_indexed() {
    let (_dir, root, session) = common::served();
    common::write(&root, ".env", "TOKEN=super-secret-value\n");
    common::write(&root, "src/id_rsa", "-----BEGIN PRIVATE KEY-----\n");
    common::write(&root, "notes.md", "nothing to see\n");

    for path in [".env", "src/id_rsa", "notes.md", "package.json"] {
        let response = session.get(&format!("/api/source?path={}", urlencode(path)));
        assert_eq!(response.status, 403, "{path}: {}", response.body);
        assert_eq!(response.parse_json()["error"]["code"], "not_indexed");
        assert!(!response.body.contains("super-secret-value"), "{path}");
        assert!(!response.body.contains("PRIVATE KEY"), "{path}");
    }
}

#[test]
fn a_source_range_is_bounded_and_says_when_it_was_cut() {
    let (_dir, root) = common::fixture_copy("ts-resolution");
    let long: String = (1..=nerve_index::MAX_SNIPPET_LINES + 500)
        .map(|line| format!("export const v{line} = {line};\n"))
        .collect();
    common::write(&root, "src/long.ts", &long);
    common::index(&root);

    let session = common::Session::start(&root);
    let value = session.json("/api/source?path=src/long.ts&start_line=1&end_line=999999");
    assert_eq!(value["truncated"], true);
    let returned = value["end_line"].as_u64().unwrap() - value["start_line"].as_u64().unwrap() + 1;
    assert!(
        returned <= nerve_index::MAX_SNIPPET_LINES as u64,
        "{returned} lines returned"
    );
    assert!(value["text"].as_str().unwrap().len() <= nerve_index::MAX_SNIPPET_BYTES);
}

/// An asset route is a table lookup, not a filesystem read; traversal cannot express anything.
#[test]
fn asset_routes_never_reach_the_filesystem() {
    let (_dir, _root, session) = common::served();
    for target in [
        "/../../Cargo.toml",
        "/assets/../../../Cargo.toml",
        "/assets/%2e%2e%2f%2e%2e%2fCargo.toml",
        "/etc/passwd",
        "/src/math.ts",
    ] {
        let response = session.get(target);
        assert_eq!(response.status, 404, "{target}: {}", response.body);
        assert_eq!(response.parse_json()["error"]["code"], "no_such_route");
    }
}

// ---- malformed input ------------------------------------------------------------------------

#[test]
fn a_malformed_or_oversized_request_target_is_refused_not_crashed() {
    let (_dir, _root, session) = common::served();
    let long = "a".repeat(nerve_server::request::MAX_TARGET_BYTES + 10);
    let cases: [(String, u16); 5] = [
        ("/api/search?q=%".to_string(), 400),
        ("/api/search?q=%zz".to_string(), 400),
        ("/api/search?q=%00".to_string(), 400),
        (format!("/api/search?q={long}"), 414),
        (
            format!(
                "/api/search?{}",
                (0..nerve_server::request::MAX_PARAMETERS + 5)
                    .map(|index| format!("k{index}=1"))
                    .collect::<Vec<_>>()
                    .join("&")
            ),
            400,
        ),
    ];
    for (target, expected) in cases {
        let response = session.get(&target);
        assert_eq!(response.status, expected, "{target}: {}", response.body);
    }
    // Still alive.
    assert_eq!(session.get("/api/overview").status, 200);
}

#[test]
fn a_request_carrying_a_body_is_refused_without_reading_it() {
    let (_dir, _root, session) = common::served();
    let wire = format!(
        "GET /api/overview HTTP/1.1\r\nHost: {}\r\n{}: {}\r\nContent-Length: 12\r\n\
         Connection: close\r\n\r\nhello world!",
        session.host(),
        nerve_server::token::TOKEN_HEADER,
        session.token()
    );
    let response = common::send_raw(session.address(), wire.as_bytes()).expect("a response");
    assert_eq!(response.status, 413, "{}", response.body);
    assert_eq!(session.get("/api/overview").status, 200);
}

#[test]
fn a_truncated_or_garbage_request_does_not_take_the_server_down() {
    let (_dir, _root, session) = common::served();
    for wire in [
        b"GET".to_vec(),
        b"GET /api/overview".to_vec(),
        b"GET /api/overview HTTP/1.1\r\n".to_vec(),
        b"GET /api/overview HTTP/1.1\r\nHost: 127".to_vec(),
        b"\x00\x01\x02\x03\r\n\r\n".to_vec(),
        b"NOTAMETHOD / HTTP/1.1\r\nHost: x\r\n\r\n".to_vec(),
        vec![b'A'; 64 * 1024],
    ] {
        // Some of these get no reply at all, which is a correct outcome for a request that was
        // never completed. What must not happen is the server stopping.
        let _ = common::send_raw(session.address(), &wire);
    }
    assert_eq!(session.get("/api/overview").status, 200);
    assert_eq!(session.get("/api/search?q=area").status, 200);
}

// ---- helpers ---------------------------------------------------------------------------------

/// Percent-encode everything that is not unreserved, so a payload survives the query string.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Keeps `Path` imported for the fixture helpers used above.
#[allow(dead_code)]
fn _path_marker(path: &Path) -> bool {
    path.exists()
}
