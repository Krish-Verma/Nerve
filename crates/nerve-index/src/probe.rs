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

/// What resolving a repository-relative path produced, before any bytes were read.
enum Resolution {
    /// The path passed every check; this is the canonical absolute path.
    Allowed(PathBuf),
    /// Nothing exists there.
    Missing,
    /// A safety check refused it. Nothing was opened.
    Refused,
    /// Allowed, but not a readable ordinary file within the size ceiling.
    Unreadable,
}

/// How much source one snippet request may return, whatever it asks for.
///
/// The threat model requires the served byte range to be bounded (T6). It is bounded here, in
/// the code that owns the repository root, rather than in the surface that happens to ask.
pub const MAX_SNIPPET_BYTES: usize = 256 * 1024;

/// Largest number of lines one snippet request may return.
pub const MAX_SNIPPET_LINES: usize = 2_000;

/// A bounded, safety-checked read of part of an indexed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSnippet {
    /// Lines `start_line..=end_line` of the file, 1-based and inclusive.
    Text {
        /// The requested lines, joined with `\n`.
        text: String,
        /// First line returned, 1-based.
        start_line: usize,
        /// Last line returned, 1-based.
        end_line: usize,
        /// Total lines in the file.
        total_lines: usize,
        /// True when the byte or line ceiling cut the range short.
        truncated: bool,
        /// BLAKE3 of the whole file as it is now, so a caller can compare freshness.
        content_hash: String,
    },
    /// Nothing exists at that path any more.
    Missing,
    /// A path-safety check refused it. Nothing was opened, nothing leaked.
    Refused,
    /// Allowed, but the bytes could not be obtained or are not UTF-8.
    Unreadable,
}

impl RepositoryProber {
    /// The one place a repository-relative path becomes an absolute path that may be opened.
    ///
    /// Both [`FileProber::probe`] and [`RepositoryProber::read_snippet`] go through this, so
    /// there is exactly one implementation of the rules and no second one to drift from it
    /// (SECURITY.md, "Path safety"; THREAT-MODEL T2 and T6).
    fn resolve(&self, rel_path: &str) -> Resolution {
        if rel_path.is_empty() || rel_path.contains('\0') {
            return Resolution::Refused;
        }
        let candidate = Path::new(rel_path);
        if candidate.is_absolute() {
            return Resolution::Refused;
        }
        // `..` and `.` never appear in a path Nerve recorded, so their presence means the row
        // did not come from this indexer. Refuse before touching the filesystem.
        if !candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Resolution::Refused;
        }
        let name = match candidate.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => return Resolution::Refused,
        };
        if is_denied(name, &self.deny_patterns) {
            return Resolution::Refused;
        }

        let joined = self.root.join(candidate);
        match std::fs::symlink_metadata(&joined) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Resolution::Missing,
            Err(_) => return Resolution::Unreadable,
            // Discovery never indexes a symlink, so an observation path that is one now has
            // been swapped since indexing. Refuse it rather than follow it.
            Ok(metadata) if metadata.file_type().is_symlink() => return Resolution::Refused,
            Ok(metadata) if !metadata.is_file() => return Resolution::Unreadable,
            Ok(metadata) if metadata.len() > self.max_file_bytes => return Resolution::Unreadable,
            Ok(_) => {}
        }

        // The single choke point: canonicalize, and prove the result is inside the root. This
        // is what catches a symlinked *parent* directory pointing out of the repository.
        match canonical_child(&self.root, candidate) {
            Ok(canonical) => Resolution::Allowed(canonical),
            Err(_) => Resolution::Refused,
        }
    }

    /// Read lines `start_line..=end_line` of an indexed file, bounded and safety-checked.
    ///
    /// Lines are 1-based and the range is inclusive, matching every span Nerve stores. The
    /// caller's range is clamped to [`MAX_SNIPPET_LINES`] and the result to
    /// [`MAX_SNIPPET_BYTES`]; a range that had to be cut is reported as `truncated` rather than
    /// silently shortened.
    ///
    /// The path is resolved through [`RepositoryProber::resolve`], so traversal, symlink escape,
    /// deny-listed names and anything outside the root are refused here exactly as they are for
    /// freshness probes. Nothing outside the repository root is ever opened.
    pub fn read_snippet(
        &self,
        rel_path: &str,
        start_line: usize,
        end_line: usize,
    ) -> SourceSnippet {
        let canonical = match self.resolve(rel_path) {
            Resolution::Allowed(path) => path,
            Resolution::Missing => return SourceSnippet::Missing,
            Resolution::Refused => return SourceSnippet::Refused,
            Resolution::Unreadable => return SourceSnippet::Unreadable,
        };

        let bytes = match std::fs::read(&canonical) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return SourceSnippet::Missing
            }
            Err(_) => return SourceSnippet::Unreadable,
        };
        let content_hash = content_hash(&bytes);
        let Ok(text) = String::from_utf8(bytes) else {
            return SourceSnippet::Unreadable;
        };

        let lines: Vec<&str> = text.lines().collect();
        let total_lines = lines.len();
        if total_lines == 0 {
            return SourceSnippet::Text {
                text: String::new(),
                start_line: 0,
                end_line: 0,
                total_lines: 0,
                truncated: false,
                content_hash,
            };
        }

        let first = start_line.max(1).min(total_lines);
        let requested_last = end_line.max(first).min(total_lines);
        let capped_last = requested_last.min(first + MAX_SNIPPET_LINES - 1);
        let mut truncated = capped_last < requested_last;

        let mut collected = String::new();
        let mut last = first;
        for (offset, line) in lines[first - 1..capped_last].iter().enumerate() {
            if !collected.is_empty() && collected.len() + line.len() + 1 > MAX_SNIPPET_BYTES {
                truncated = true;
                break;
            }
            if !collected.is_empty() {
                collected.push('\n');
            }
            collected.push_str(line);
            last = first + offset;
        }

        SourceSnippet::Text {
            text: collected,
            start_line: first,
            end_line: last,
            total_lines,
            truncated,
            content_hash,
        }
    }
}

impl FileProber for RepositoryProber {
    fn probe(&self, rel_path: &str) -> FileProbe {
        let canonical = match self.resolve(rel_path) {
            Resolution::Allowed(path) => path,
            Resolution::Missing => return FileProbe::Missing,
            Resolution::Refused => return FileProbe::Refused,
            Resolution::Unreadable => return FileProbe::Unreadable,
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

    fn snippet_repository() -> (tempfile::TempDir, RepositoryProber) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::init(root).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lines.ts"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        let prober = RepositoryProber::new(root).unwrap();
        (dir, prober)
    }

    #[test]
    fn a_snippet_returns_the_requested_inclusive_line_range() {
        let (_dir, prober) = snippet_repository();
        match prober.read_snippet("src/lines.ts", 2, 4) {
            SourceSnippet::Text {
                text,
                start_line,
                end_line,
                total_lines,
                truncated,
                ..
            } => {
                assert_eq!(text, "two\nthree\nfour");
                assert_eq!((start_line, end_line, total_lines), (2, 4, 5));
                assert!(!truncated);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_snippet_range_is_clamped_to_the_file() {
        let (_dir, prober) = snippet_repository();
        match prober.read_snippet("src/lines.ts", 0, 9_999) {
            SourceSnippet::Text {
                start_line,
                end_line,
                ..
            } => assert_eq!((start_line, end_line), (1, 5)),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_snippet_refuses_every_path_the_freshness_probe_refuses() {
        let (dir, prober) = snippet_repository();
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        for path in [
            "../outside.ts",
            "src/../../outside.ts",
            "/etc/passwd",
            "src/lines\0.ts",
            "",
            "./src/lines.ts",
            ".env",
        ] {
            assert_eq!(
                prober.read_snippet(path, 1, 10),
                SourceSnippet::Refused,
                "{path:?} must not be read"
            );
            assert_eq!(prober.probe(path), FileProbe::Refused, "{path:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_snippet_refuses_a_symlink_escaping_the_root() {
        let (dir, prober) = snippet_repository();
        let outside = dir.path().parent().unwrap().join("nerve-probe-outside.ts");
        std::fs::write(&outside, "export const secret = 1;\n").unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("src/linked.ts")).unwrap();
        assert_eq!(
            prober.read_snippet("src/linked.ts", 1, 10),
            SourceSnippet::Refused
        );
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn a_missing_file_is_missing_not_refused_for_snippets_too() {
        let (_dir, prober) = snippet_repository();
        assert_eq!(
            prober.read_snippet("src/gone.ts", 1, 10),
            SourceSnippet::Missing
        );
    }
}
