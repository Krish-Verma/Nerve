//! Static assets, compiled into the binary.
//!
//! `nerve serve` must need no Node runtime, no build step and no files on disk beyond the
//! repository it is indexing, so the interface ships inside the executable.
//!
//! **This is the hook the explorer slice fills.** Built assets are dropped into
//! `crates/nerve-server/assets/` and listed in [`ASSETS`]; nothing else about the server needs
//! to change. Two properties of this module are load-bearing and must survive that change:
//!
//! - **Lookup is an exact match against a fixed table.** No path is ever joined onto a directory
//!   and no filesystem call is made, so there is no traversal surface here at all — a request
//!   for `../../etc/passwd` simply fails to match a table entry.
//! - **Nothing is templated.** Bytes are served exactly as compiled in. There is no substitution
//!   point where a repository string, a session token, or an error message could be interpolated
//!   into markup, which is what makes the served document XSS-free by construction rather than
//!   by escaping (THREAT-MODEL T5).
//!
//! The placeholder page carries no inline `<style>` or `<script>`, because the
//! `Content-Security-Policy` this server sends has no `unsafe-inline`. A page that needs an
//! exception to the policy is a page that has weakened it.

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
        path: "assets/nerve.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../assets/nerve.css"),
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

    #[test]
    fn the_placeholder_page_needs_no_csp_exception() {
        let html = std::str::from_utf8(ASSETS[0].bytes).unwrap();
        assert!(!html.contains("<script"), "no inline or external script");
        assert!(!html.contains("<style"), "no inline style block");
        assert!(!html.to_ascii_lowercase().contains(" onload"));
        assert!(!html.to_ascii_lowercase().contains(" onerror"));
        assert!(!html.contains("http://"), "no remote origin");
        assert!(!html.contains("https://"), "no remote origin");
        assert!(html.contains("/assets/nerve.css"));
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
