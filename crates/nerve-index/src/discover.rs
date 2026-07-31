//! File discovery, ignore rules and path safety.
//!
//! Repository content is untrusted input (SECURITY.md). Every path this module yields has
//! been canonicalized and proven to live under the repository root, and no symlink is
//! followed. Nothing here reads file contents.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;

use crate::config::{is_denied, Config, PRUNED_DIRECTORIES};
use crate::error::{IndexError, Result};
use crate::lang::Language;

/// A file selected for indexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    /// Repository-relative path, `/`-separated.
    pub rel_path: String,
    /// Canonical absolute path.
    pub abs_path: PathBuf,
    /// Grammar chosen from the extension.
    pub language: Language,
}

/// What discovery found and what it refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryReport {
    /// Files to index, sorted by `rel_path`.
    pub files: Vec<DiscoveredFile>,
    /// Directories containing at least one indexed file, sorted, `/`-separated.
    pub directories: Vec<String>,
    /// Paths refused by the secret deny-list.
    pub denied_secrets: Vec<String>,
    /// Paths skipped because their extension has no grammar.
    pub skipped_unsupported: usize,
    /// Entries skipped because they were symlinks.
    pub skipped_symlinks: usize,
}

/// Canonicalize the repository root, asserting it is a directory.
pub fn canonical_root(root: &Path) -> Result<PathBuf> {
    if !root.is_dir() {
        return Err(IndexError::NotADirectory(root.to_path_buf()));
    }
    Ok(std::fs::canonicalize(root)?)
}

/// Canonicalize `candidate` and prove it lives under `root`.
///
/// This is the single choke point for path safety. It rejects `..` traversal, absolute paths
/// pointing elsewhere, NUL bytes, non-UTF-8 names, and symlinks that resolve outside the root.
pub fn canonical_child(root: &Path, candidate: &Path) -> Result<PathBuf> {
    let as_str = candidate.to_str();
    if as_str.is_none() {
        return Err(IndexError::NonUtf8Path(candidate.to_path_buf()));
    }
    if as_str.is_some_and(|s| s.contains('\0')) {
        return Err(IndexError::PathEscapesRoot(candidate.to_path_buf()));
    }

    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };

    let canonical = std::fs::canonicalize(&joined)
        .map_err(|_| IndexError::PathEscapesRoot(candidate.to_path_buf()))?;

    if !canonical.starts_with(root) {
        return Err(IndexError::PathEscapesRoot(candidate.to_path_buf()));
    }
    Ok(canonical)
}

/// Repository-relative, `/`-separated form of a canonical path under `root`.
pub fn relative_path(root: &Path, canonical: &Path) -> Result<String> {
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| IndexError::PathEscapesRoot(canonical.to_path_buf()))?;
    let mut segments = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                let text = part
                    .to_str()
                    .ok_or_else(|| IndexError::NonUtf8Path(canonical.to_path_buf()))?;
                segments.push(text.to_string());
            }
            Component::CurDir => {}
            _ => return Err(IndexError::PathEscapesRoot(canonical.to_path_buf())),
        }
    }
    Ok(segments.join("/"))
}

fn parent_directory(rel_path: &str) -> Option<String> {
    rel_path
        .rfind('/')
        .map(|index| rel_path[..index].to_string())
}

/// Walk the repository and select files to index.
///
/// Ignore sources, in the order the `ignore` crate applies them: `.gitignore`, `.ignore`,
/// `.git/info/exclude`, and `.nerveignore`. Parent-directory ignore files are deliberately not
/// consulted, so a repository indexes the same way wherever it is checked out.
pub fn discover(root: &Path, config: &Config) -> Result<DiscoveryReport> {
    let root = canonical_root(root)?;
    let deny_patterns = config.deny_patterns();
    let mut report = DiscoveryReport::default();
    let mut directories: BTreeSet<String> = BTreeSet::new();

    let mut builder = WalkBuilder::new(&root);
    builder
        .follow_links(false)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .ignore(true)
        .add_custom_ignore_filename(".nerveignore")
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if !is_dir {
                return true;
            }
            entry
                .file_name()
                .to_str()
                .map(|name| !PRUNED_DIRECTORIES.contains(&name))
                .unwrap_or(false)
        });

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            // A directory we cannot read is not a fatal condition; it contributes nothing.
            Err(_) => continue,
        };
        let file_type = match entry.file_type() {
            Some(file_type) => file_type,
            None => continue,
        };
        if file_type.is_symlink() {
            report.skipped_symlinks += 1;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let name = match entry.file_name().to_str() {
            Some(name) => name.to_string(),
            None => continue,
        };

        // Deny-list runs before anything reads the file.
        if is_denied(&name, &deny_patterns) {
            if let Ok(canonical) = canonical_child(&root, entry.path()) {
                if let Ok(rel) = relative_path(&root, &canonical) {
                    report.denied_secrets.push(rel);
                }
            }
            continue;
        }

        let extension = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Some(language) = Language::from_extension(&extension) else {
            report.skipped_unsupported += 1;
            continue;
        };

        let canonical = match canonical_child(&root, entry.path()) {
            Ok(canonical) => canonical,
            Err(_) => continue,
        };
        let rel_path = relative_path(&root, &canonical)?;

        let mut ancestor = parent_directory(&rel_path);
        while let Some(directory) = ancestor {
            ancestor = parent_directory(&directory);
            directories.insert(directory);
        }

        report.files.push(DiscoveredFile {
            rel_path,
            abs_path: canonical,
            language,
        });
    }

    report.files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    report.denied_secrets.sort();
    report.directories = directories.into_iter().collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_directory_walks_up() {
        assert_eq!(parent_directory("a/b/c.ts"), Some("a/b".to_string()));
        assert_eq!(parent_directory("a"), None);
    }

    #[test]
    fn relative_path_uses_forward_slashes() {
        let root = Path::new("/tmp/root");
        let child = Path::new("/tmp/root/src/math.ts");
        assert_eq!(relative_path(root, child).unwrap(), "src/math.ts");
    }
}
