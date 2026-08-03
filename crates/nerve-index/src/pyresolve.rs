//! Python module specifier resolution.
//!
//! The counterpart of [`crate::resolve`], and it holds the same line: a specifier resolves only
//! when it names a file that is **actually in the index**, and everything else becomes an
//! `Unresolved` entity, which is a value rather than an omission (ADR-0003).
//!
//! # What a Python specifier is
//!
//! A specifier here is written exactly as the source writes it after `from` or `import`:
//! `os`, `pkg.util`, `.util`, `..core`, `.`, `....`. Leading dots are the relative level;
//! CPython counts them from the importer's own package, so **one** dot is that package and each
//! further dot climbs one level. The rest is a dotted name, resolved as directory segments.
//!
//! # What is deliberately not modelled
//!
//! - **`sys.path`.** Absolute resolution consults it, so a module that rewrites it has made
//!   every absolute specifier in itself unsound. That is handled by the caller
//!   ([`crate::pystruct::PyModule::sys_path_mutated`]) rather than here, because it is a
//!   property of the importing module and not of the specifier.
//! - **Source roots.** A `src/` layout, an installed distribution, a `PYTHONPATH` entry and an
//!   editable install all make `import mypkg` mean a file this walk never sees. Guessing a root
//!   would produce edges that look resolved and are not, which is exactly the failure this
//!   product exists to avoid.
//! - **Submodules named by `from pkg import name`.** Whether `name` is an attribute the
//!   package's `__init__.py` binds or the `pkg.name` submodule needs cross-module reasoning,
//!   which is Slice 9b's ground. The statement names `pkg`, and that is what is recorded.

use std::collections::BTreeSet;

/// The file that makes a directory a package.
pub const PACKAGE_INIT: &str = "__init__.py";

/// The extension of a Python module file.
pub const MODULE_SUFFIX: &str = ".py";

/// True when the specifier is relative — that is, written with at least one leading dot.
pub fn is_relative(specifier: &str) -> bool {
    specifier.starts_with('.')
}

/// Split a raw specifier into its leading dot count and the dotted name that follows.
///
/// `"..pkg.mod"` is `(2, "pkg.mod")`; `"."` is `(1, "")`; `"os.path"` is `(0, "os.path")`.
pub fn split_dots(specifier: &str) -> (usize, &str) {
    let dots = specifier.chars().take_while(|c| *c == '.').count();
    (dots, &specifier[dots..])
}

/// The directory the specifier is resolved against, as path segments.
///
/// `None` when a relative specifier climbs above the repository root — refused rather than
/// clamped, exactly as [`crate::resolve`] refuses `../../etc/passwd`.
fn base_segments(importer_rel_path: &str, specifier: &str) -> Option<Vec<String>> {
    let (dots, name) = split_dots(specifier);

    let mut segments: Vec<String> = if dots == 0 {
        // Absolute: resolved from the repository root and nowhere else.
        Vec::new()
    } else {
        let mut own: Vec<String> = match importer_rel_path.rfind('/') {
            Some(index) => importer_rel_path[..index]
                .split('/')
                .map(str::to_string)
                .collect(),
            None => Vec::new(),
        };
        // One dot is the importer's own package; each further dot climbs one level.
        for _ in 1..dots {
            own.pop()?;
        }
        own
    };

    for part in name.split('.') {
        if part.is_empty() {
            continue;
        }
        segments.push(part.to_string());
    }
    Some(segments)
}

/// Every repository-relative path the specifier could name, in resolution order.
///
/// The package comes first. That is CPython's order, not a preference invented here:
/// `FileFinder` consults its path hooks — which recognise directories — before its file
/// loaders, so `pkg/util/__init__.py` shadows `pkg/util.py` when both exist.
pub fn candidates(importer_rel_path: &str, specifier: &str) -> Vec<String> {
    let Some(segments) = base_segments(importer_rel_path, specifier) else {
        return Vec::new();
    };
    let joined = segments.join("/");
    if joined.is_empty() {
        // `from . import x` written in a module at the repository root. The "package" is the
        // root itself, whose init module would be `__init__.py`.
        return vec![PACKAGE_INIT.to_string()];
    }
    vec![
        format!("{joined}/{PACKAGE_INIT}"),
        format!("{joined}{MODULE_SUFFIX}"),
    ]
}

/// Resolve a specifier to an indexed module, or `None`.
pub fn resolve(
    importer_rel_path: &str,
    specifier: &str,
    indexed: &BTreeSet<String>,
) -> Option<String> {
    candidates(importer_rel_path, specifier)
        .into_iter()
        .find(|candidate| indexed.contains(candidate))
}

/// True when a relative specifier climbs above the repository root.
pub fn climbs_above_root(importer_rel_path: &str, specifier: &str) -> bool {
    is_relative(specifier) && base_segments(importer_rel_path, specifier).is_none()
}

/// True when the specifier names a directory that holds indexed files and has no `__init__.py`.
///
/// That is a **namespace package**: Python assembles it at import time from every `sys.path`
/// entry carrying a directory of the same name, so its contents are not a property of this
/// repository and Nerve must not claim to have found them. Distinguishing it from "names
/// nothing at all" is the whole point — the two have different fixes.
pub fn is_namespace_package(
    importer_rel_path: &str,
    specifier: &str,
    indexed: &BTreeSet<String>,
) -> bool {
    let Some(segments) = base_segments(importer_rel_path, specifier) else {
        return false;
    };
    let joined = segments.join("/");
    if joined.is_empty() {
        return false;
    }
    let init = format!("{joined}/{PACKAGE_INIT}");
    if indexed.contains(&init) {
        return false;
    }
    let prefix = format!("{joined}/");
    indexed.range(prefix.clone()..).next().is_some_and(|first| {
        // `range` from the prefix lands on the first path at or after it; a directory exists
        // when that path is inside it.
        first.starts_with(&prefix)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn dots_are_counted_and_stripped() {
        assert_eq!(split_dots("os.path"), (0, "os.path"));
        assert_eq!(split_dots(".util"), (1, "util"));
        assert_eq!(split_dots("..pkg.mod"), (2, "pkg.mod"));
        assert_eq!(split_dots("."), (1, ""));
        assert_eq!(split_dots("...."), (4, ""));
        assert!(!is_relative("os"));
        assert!(is_relative(".util"));
    }

    #[test]
    fn absolute_specifiers_resolve_from_the_repository_root() {
        let set = indexed(&["pkg/util.py", "pkg/__init__.py", "app.py"]);
        assert_eq!(
            resolve("app.py", "pkg.util", &set),
            Some("pkg/util.py".to_string())
        );
        assert_eq!(
            resolve("app.py", "pkg", &set),
            Some("pkg/__init__.py".to_string())
        );
    }

    /// CPython checks directories before files, so a package outranks a module of the same name.
    #[test]
    fn a_package_outranks_a_module_with_the_same_name() {
        let set = indexed(&["pkg/util.py", "pkg/util/__init__.py", "app.py"]);
        assert_eq!(
            resolve("app.py", "pkg.util", &set),
            Some("pkg/util/__init__.py".to_string())
        );
    }

    #[test]
    fn one_dot_is_the_importers_own_package() {
        let set = indexed(&["pkg/__init__.py", "pkg/core.py", "pkg/util.py"]);
        assert_eq!(
            resolve("pkg/core.py", ".util", &set),
            Some("pkg/util.py".to_string())
        );
        assert_eq!(
            resolve("pkg/core.py", ".", &set),
            Some("pkg/__init__.py".to_string())
        );
    }

    #[test]
    fn each_further_dot_climbs_one_level() {
        let set = indexed(&[
            "pkg/__init__.py",
            "pkg/core.py",
            "pkg/util.py",
            "pkg/sub/__init__.py",
            "pkg/sub/deep.py",
        ]);
        assert_eq!(
            resolve("pkg/sub/deep.py", "..util", &set),
            Some("pkg/util.py".to_string())
        );
        assert_eq!(
            resolve("pkg/sub/deep.py", ".", &set),
            Some("pkg/sub/__init__.py".to_string())
        );
        assert_eq!(
            resolve("pkg/sub/deep.py", "..", &set),
            Some("pkg/__init__.py".to_string())
        );
    }

    #[test]
    fn climbing_above_the_root_is_refused() {
        let set = indexed(&["pkg/sub/deep.py"]);
        assert_eq!(resolve("pkg/sub/deep.py", "....", &set), None);
        assert!(candidates("pkg/sub/deep.py", "....").is_empty());
        assert!(climbs_above_root("pkg/sub/deep.py", "...."));
        assert!(!climbs_above_root("pkg/sub/deep.py", ".."));
        assert!(!climbs_above_root("app.py", "os"));
    }

    /// A bare name must never be resolved by searching for a matching basename anywhere in the
    /// tree. `pkg/util.py` exists; `util` at the repository root does not.
    #[test]
    fn a_basename_that_exists_inside_a_package_does_not_resolve_at_the_root() {
        let set = indexed(&["pkg/util.py", "pkg/__init__.py", "negative.py"]);
        assert_eq!(resolve("negative.py", "util", &set), None);
        assert_eq!(resolve("negative.py", "core", &set), None);
    }

    #[test]
    fn a_specifier_naming_a_symbol_rather_than_a_module_does_not_resolve() {
        let set = indexed(&["pkg/util.py", "pkg/__init__.py", "negative.py"]);
        assert_eq!(resolve("negative.py", "pkg.util.scale", &set), None);
    }

    #[test]
    fn unindexed_targets_never_resolve() {
        let set = indexed(&["app.py"]);
        assert_eq!(resolve("app.py", "os", &set), None);
        assert_eq!(resolve("app.py", ".missing", &set), None);
    }

    #[test]
    fn resolution_never_leaves_the_indexed_set() {
        let set = indexed(&["pkg/__init__.py", "pkg/core.py", "app.py"]);
        for specifier in ["pkg", "pkg.core", ".", "..", "os", "....", ".core"] {
            if let Some(hit) = resolve("app.py", specifier, &set) {
                assert!(set.contains(&hit), "{specifier} escaped the index");
            }
        }
    }

    #[test]
    fn a_directory_without_an_init_is_a_namespace_package() {
        let set = indexed(&[
            "nspkg/orphan.py",
            "pkg/__init__.py",
            "pkg/util.py",
            "app.py",
        ]);
        assert!(is_namespace_package("app.py", "nspkg", &set));
        assert!(
            !is_namespace_package("app.py", "pkg", &set),
            "pkg has an __init__.py, so it is an ordinary package"
        );
        assert!(
            !is_namespace_package("app.py", "absent", &set),
            "naming nothing at all is not the same finding as naming a namespace package"
        );
        assert!(
            !is_namespace_package("app.py", "pkg.util", &set),
            "pkg/util.py is a module, not a directory"
        );
    }

    /// The prefix scan must not mistake a sibling whose name merely starts with the same
    /// characters for a directory.
    #[test]
    fn a_prefix_match_is_not_a_directory() {
        let set = indexed(&["nspkgs.py", "app.py"]);
        assert!(!is_namespace_package("app.py", "nspkg", &set));
    }
}
