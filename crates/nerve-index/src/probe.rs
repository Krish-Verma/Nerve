//! Query-time file reads, under the Slice 1 path-safety rules.
//!
//! `nerve why` computes freshness by re-hashing the file an observation points at. That makes
//! it the first command that opens a repository file at query time, and the path it opens
//! comes out of the database — which is a file on disk, not a trusted channel. Every rule
//! discovery applies (SECURITY.md "Path safety") therefore applies again here, using the same
//! [`canonical_child`] choke point rather than a second implementation of it:
//!
//! - the path must be relative and contain only ordinary components (no `..`, no NUL)
//! - it must not be a symlink, and must not resolve outside the repository root
//! - it must not match the secret deny-list
//! - it must not exceed the configured file-size ceiling
//!
//! Anything else is refused and reported as refused. Nothing is guessed and nothing outside
//! the root is ever opened.

use std::path::{Component, Path, PathBuf};

use nerve_core::ids::content_hash;
use nerve_store::{FileProbe, FileProber};

use crate::config::{is_denied, Config};
use crate::discover::{canonical_child, canonical_root};
use crate::error::Result;

/// Reads repository files at query time, applying the repository's path-safety rules.
#[derive(Debug, Clone)]
pub struct RepositoryProber {
    root: PathBuf,
    deny_patterns: Vec<String>,
    max_file_bytes: u64,
}

impl RepositoryProber {
    /// Canonicalize `root` and load the settings that govern what may be read.
    pub fn new(root: &Path) -> Result<Self> {
        let root = canonical_root(root)?;
        let config = Config::load(&root)?;
        Ok(RepositoryProber {
            deny_patterns: config.deny_patterns(),
            max_file_bytes: config.index.max_file_bytes,
            root,
        })
    }

    /// The canonical repository root nothing may be read from outside of.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl FileProber for RepositoryProber {
    fn probe(&self, rel_path: &str) -> FileProbe {
        if rel_path.is_empty() || rel_path.contains('\0') {
            return FileProbe::Refused;
        }
        let candidate = Path::new(rel_path);
        if candidate.is_absolute() {
            return FileProbe::Refused;
        }
        // `..` and `.` never appear in a path Nerve recorded, so their presence means the row
        // did not come from this indexer. Refuse before touching the filesystem.
        if !candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return FileProbe::Refused;
        }
        let name = match candidate.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => return FileProbe::Refused,
        };
        if is_denied(name, &self.deny_patterns) {
            return FileProbe::Refused;
        }

        let joined = self.root.join(candidate);
        match std::fs::symlink_metadata(&joined) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return FileProbe::Missing,
            Err(_) => return FileProbe::Unreadable,
            // Discovery never indexes a symlink, so an observation path that is one now has
            // been swapped since indexing. Refuse it rather than follow it.
            Ok(metadata) if metadata.file_type().is_symlink() => return FileProbe::Refused,
            Ok(metadata) if !metadata.is_file() => return FileProbe::Unreadable,
            Ok(metadata) if metadata.len() > self.max_file_bytes => return FileProbe::Unreadable,
            Ok(_) => {}
        }

        // The single choke point: canonicalize, and prove the result is inside the root. This
        // is what catches a symlinked *parent* directory pointing out of the repository.
        let Ok(canonical) = canonical_child(&self.root, candidate) else {
            return FileProbe::Refused;
        };

        match std::fs::read(&canonical) {
            Ok(bytes) => FileProbe::Hash(content_hash(&bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => FileProbe::Missing,
            Err(_) => FileProbe::Unreadable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> (tempfile::TempDir, RepositoryProber) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::init(root).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "export const a = 1;\n").unwrap();
        let prober = RepositoryProber::new(root).unwrap();
        (dir, prober)
    }

    #[test]
    fn a_real_file_hashes_to_its_contents() {
        let (_dir, prober) = repository();
        assert_eq!(
            prober.probe("src/a.ts"),
            FileProbe::Hash(content_hash(b"export const a = 1;\n"))
        );
    }

    #[test]
    fn a_vanished_file_is_missing_not_refused() {
        let (_dir, prober) = repository();
        assert_eq!(prober.probe("src/gone.ts"), FileProbe::Missing);
    }

    #[test]
    fn traversal_absolute_and_nul_paths_are_refused() {
        let (_dir, prober) = repository();
        for path in [
            "../outside.ts",
            "src/../../outside.ts",
            "/etc/passwd",
            "src/a\0.ts",
            "",
            "./src/a.ts",
        ] {
            assert_eq!(prober.probe(path), FileProbe::Refused, "{path:?}");
        }
    }

    #[test]
    fn deny_listed_names_are_never_read() {
        let (dir, prober) = repository();
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        assert_eq!(prober.probe(".env"), FileProbe::Refused);
    }

    #[test]
    fn a_directory_is_not_readable_as_a_file() {
        let (_dir, prober) = repository();
        assert_eq!(prober.probe("src"), FileProbe::Unreadable);
    }
}
