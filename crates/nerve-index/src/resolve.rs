//! Module specifier resolution.
//!
//! Slice 1 resolves **relative specifiers only**, and only to files that are actually in the
//! index. Bare package specifiers, `tsconfig` path aliases, workspace layouts and
//! `node_modules` semantics are not modelled, and pretending otherwise would produce edges
//! that look resolved and are not. Everything that does not resolve becomes an `Unresolved`
//! entity, which is a value, not an omission (ADR-0003).

use std::collections::BTreeSet;

/// Extensions tried against a bare specifier, in order.
pub const RESOLUTION_EXTENSIONS: [&str; 6] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

/// Directory index files tried against a bare specifier, in order.
pub const RESOLUTION_INDEX_FILES: [&str; 4] =
    ["/index.ts", "/index.tsx", "/index.js", "/index.jsx"];

/// True when the specifier is relative and therefore in scope for Slice 1 resolution.
pub fn is_relative(specifier: &str) -> bool {
    specifier.starts_with("./") || specifier.starts_with("../")
}

/// Normalize `<dir of importer> + specifier` into a repository-relative path.
///
/// Returns `None` when the specifier climbs above the repository root.
fn normalize(importer_rel_path: &str, specifier: &str) -> Option<String> {
    let mut segments: Vec<&str> = match importer_rel_path.rfind('/') {
        Some(index) => importer_rel_path[..index].split('/').collect(),
        None => Vec::new(),
    };
    if segments == [""] {
        segments.clear();
    }

    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

/// Every candidate path a specifier could name, in resolution order.
pub fn candidates(importer_rel_path: &str, specifier: &str) -> Vec<String> {
    let Some(base) = normalize(importer_rel_path, specifier) else {
        return Vec::new();
    };
    if base.is_empty() {
        return Vec::new();
    }
    let mut out =
        Vec::with_capacity(1 + RESOLUTION_EXTENSIONS.len() + RESOLUTION_INDEX_FILES.len());
    out.push(base.clone());
    for extension in RESOLUTION_EXTENSIONS {
        out.push(format!("{base}{extension}"));
    }
    for index in RESOLUTION_INDEX_FILES {
        out.push(format!("{base}{index}"));
    }
    out
}

/// Resolve a specifier to an indexed module, or `None`.
pub fn resolve(
    importer_rel_path: &str,
    specifier: &str,
    indexed: &BTreeSet<String>,
) -> Option<String> {
    if !is_relative(specifier) {
        return None;
    }
    candidates(importer_rel_path, specifier)
        .into_iter()
        .find(|candidate| indexed.contains(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn resolves_sibling_typescript() {
        let set = indexed(&["src/math.ts", "src/app.ts"]);
        assert_eq!(
            resolve("src/app.ts", "./math", &set),
            Some("src/math.ts".to_string())
        );
    }

    #[test]
    fn prefers_an_exact_path_over_an_extension_guess() {
        let set = indexed(&["src/math", "src/math.ts"]);
        assert_eq!(
            resolve("src/app.ts", "./math", &set),
            Some("src/math".to_string())
        );
    }

    #[test]
    fn extension_preference_order_is_ts_first() {
        let set = indexed(&["src/math.js", "src/math.ts"]);
        assert_eq!(
            resolve("src/app.ts", "./math", &set),
            Some("src/math.ts".to_string())
        );
    }

    #[test]
    fn resolves_parent_relative_paths() {
        let set = indexed(&["src/lib/util.ts", "src/feature/app.ts"]);
        assert_eq!(
            resolve("src/feature/app.ts", "../lib/util", &set),
            Some("src/lib/util.ts".to_string())
        );
    }

    #[test]
    fn resolves_directory_index_files() {
        let set = indexed(&["src/shapes/index.ts", "src/app.ts"]);
        assert_eq!(
            resolve("src/app.ts", "./shapes", &set),
            Some("src/shapes/index.ts".to_string())
        );
    }

    #[test]
    fn resolves_explicit_extensions() {
        let set = indexed(&["src/math.ts", "src/app.ts"]);
        assert_eq!(
            resolve("src/app.ts", "./math.ts", &set),
            Some("src/math.ts".to_string())
        );
    }

    #[test]
    fn bare_specifiers_never_resolve() {
        let set = indexed(&["react.ts", "src/app.ts"]);
        assert_eq!(resolve("src/app.ts", "react", &set), None);
        assert_eq!(resolve("src/app.ts", "@scope/pkg", &set), None);
    }

    #[test]
    fn unindexed_targets_never_resolve() {
        let set = indexed(&["src/app.ts"]);
        assert_eq!(resolve("src/app.ts", "./missing", &set), None);
    }

    #[test]
    fn climbing_above_the_root_is_refused() {
        let set = indexed(&["src/app.ts"]);
        assert_eq!(resolve("src/app.ts", "../../etc/passwd", &set), None);
        assert!(candidates("src/app.ts", "../../../x").is_empty());
    }

    #[test]
    fn resolution_never_leaves_the_indexed_set() {
        let set = indexed(&["src/app.ts"]);
        for specifier in ["./x", "../x", "./x/y", "../../x"] {
            if let Some(hit) = resolve("src/app.ts", specifier, &set) {
                assert!(set.contains(&hit));
            }
        }
    }
}
