//! Static assets, compiled into the binary.
//!
//! `nerve serve` must need no Node runtime, no build step and no files on disk beyond the
//! repository it is indexing, so the interface ships inside the executable.
//!
//! The bytes here are the output of `apps/nerve-web`, copied in by `npm run build` (which runs
//! `apps/nerve-web/tools/embed.mjs` after Vite). **The directory layout mirrors the route layout
//! exactly** — `assets/nerve.js` on disk is `/assets/nerve.js` over HTTP — so the `include_bytes!`
//! paths and the [`ASSETS`] table cannot drift apart without the mismatch being obvious.
//!
//! Two properties of this module are load-bearing:
//!
//! - **Lookup is an exact match against a fixed table.** No path is ever joined onto a directory
//!   and no filesystem call is made, so there is no traversal surface here at all — a request
//!   for `../../etc/passwd` simply fails to match a table entry.
//! - **Nothing is templated.** Bytes are served exactly as compiled in. There is no substitution
//!   point where a repository string, a session token, or an error message could be interpolated
//!   into markup, which is what makes the served document XSS-free by construction rather than
//!   by escaping (THREAT-MODEL T5).
//!
//! The served document carries no inline `<style>` and no inline `<script>` body — only external
//! references to files in this same table — because the `Content-Security-Policy` this server
//! sends has no `unsafe-inline`. A page that needs an exception to the policy is a page that has
//! weakened it. `embed.mjs` re-reads the emitted HTML and refuses to copy it in if that stops
//! being true, and [`the_served_page_needs_no_csp_exception`] asserts it again from Rust, against
//! the bytes that actually shipped.
//!
//! [`the_served_page_needs_no_csp_exception`]: tests::the_served_page_needs_no_csp_exception

/// One embedded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// Route-relative path, with no leading slash.
    pub path: &'static str,
    /// Exact `Content-Type` to serve it with. Never sniffed.
    pub content_type: &'static str,
    /// The bytes, verbatim.
    pub bytes: &'static [u8],
}

/// Path served for `/`.
pub const INDEX_PATH: &str = "index.html";

/// Every embedded asset.
pub const ASSETS: &[Asset] = &[
    Asset {
        path: INDEX_PATH,
        content_type: "text/html; charset=utf-8",
        bytes: include_bytes!("../assets/index.html"),
    },
    Asset {
        path: "assets/nerve.js",
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../assets/assets/nerve.js"),
    },
    Asset {
        path: "assets/nerve.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../assets/assets/nerve.css"),
    },
    Asset {
        path: "assets/favicon.svg",
        content_type: "image/svg+xml",
        bytes: include_bytes!("../assets/assets/favicon.svg"),
    },
];

/// Find an asset by its route path.
///
/// `/` and `/index.html` both resolve to the index. Everything else must match a table entry
/// exactly — no prefix matching, no directory listing, no fallback to the filesystem.
pub fn lookup(path: &str) -> Option<&'static Asset> {
    let wanted = match path {
        "/" | "" => INDEX_PATH,
        other => other.strip_prefix('/').unwrap_or(other),
    };
    ASSETS.iter().find(|asset| asset.path == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_serves_the_index() {
        assert_eq!(lookup("/").map(|a| a.path), Some(INDEX_PATH));
        assert_eq!(lookup("/index.html").map(|a| a.path), Some(INDEX_PATH));
    }

    #[test]
    fn nothing_outside_the_table_resolves() {
        for path in [
            "/../Cargo.toml",
            "/assets/../../Cargo.toml",
            "/assets/",
            "/assets",
            "/etc/passwd",
            "/index.html/",
            "/INDEX.HTML",
        ] {
            assert!(lookup(path).is_none(), "{path} must not resolve");
        }
    }

    /// The served document must run under `script-src 'self'; style-src 'self'` with no
    /// `unsafe-inline`. That permits an external script and an external stylesheet, and forbids
    /// an inline body, an inline style block, and an event-handler attribute.
    ///
    /// This asserts the property against the bytes that actually shipped, which is the point:
    /// `embed.mjs` checks the same thing in the frontend build, and neither check trusts the
    /// other or trusts the Vite configuration that is supposed to make both pass.
    #[test]
    fn the_served_page_needs_no_csp_exception() {
        let html = std::str::from_utf8(ASSETS[0].bytes).unwrap();
        let lower = html.to_ascii_lowercase();

        // Every `<script>` must be a reference, never a body: `<script ... src=...></script>`.
        for fragment in html.split("<script").skip(1) {
            let (open, rest) = fragment
                .split_once('>')
                .expect("an unterminated <script tag");
            assert!(open.contains("src="), "a <script> with no src: {open}");
            let body = rest.split("</script>").next().unwrap_or("");
            assert!(body.trim().is_empty(), "an inline <script> body: {body}");
        }

        assert!(!lower.contains("<style"), "no inline style block");
        assert!(!lower.contains(" style="), "no style attribute");
        assert!(!lower.contains(" onload"));
        assert!(!lower.contains(" onerror"));
        assert!(!lower.contains(" onclick"));
        assert!(!html.contains("http://"), "no remote origin");
        assert!(!html.contains("https://"), "no remote origin");

        // And every URL it names must be an asset this binary can actually serve.
        for path in [
            "/assets/nerve.js",
            "/assets/nerve.css",
            "/assets/favicon.svg",
        ] {
            assert!(
                html.contains(path),
                "the document does not reference {path}"
            );
            assert!(
                lookup(path).is_some(),
                "{path} is referenced but not embedded"
            );
        }
    }

    /// The interface is compiled in, so an empty or truncated build must not reach a release.
    #[test]
    fn the_interface_is_actually_embedded() {
        let script = lookup("/assets/nerve.js").expect("the script is embedded");
        assert!(
            script.bytes.len() > 20_000,
            "the embedded bundle is {} bytes, which is not a built application",
            script.bytes.len()
        );
        let styles = lookup("/assets/nerve.css").expect("the stylesheet is embedded");
        assert!(
            styles.bytes.len() > 2_000,
            "the embedded stylesheet is too small to be real"
        );
    }

    #[test]
    fn every_asset_declares_a_content_type_and_has_bytes() {
        for asset in ASSETS {
            assert!(!asset.content_type.is_empty(), "{}", asset.path);
            assert!(!asset.bytes.is_empty(), "{}", asset.path);
            assert!(!asset.path.starts_with('/'), "{}", asset.path);
        }
    }
}
