//! The `fs-structural` extractor: the repository, its directories, its files, and containment.
//!
//! Everything this extractor produces is derivable from a **directory walk**. Nothing here needs
//! a grammar, a heading scanner, or a single byte of any file's contents. Before Slice 5d-i these
//! same claims were emitted by `ts-js-structural` as `AST_DIRECT`, which meant indexing a
//! documentation-only tree produced observations asserting that a syntax tree contained something
//! in a repository with no TypeScript in it. An evidence label that can say that is decoration,
//! so the label moved to one that is true: [`EvidenceSourceType::FilesystemObserved`].
//!
//! The defining property, which is what makes the amended THREAT-MODEL.md **T7** sound, is that
//! **`fs-structural` never reads file bytes**. That is enforced by construction rather than by
//! discipline: the graph builder's only input is `&[FsEntry]`, and [`FsEntry`] has no field that
//! can hold file contents. There is no `source`, no text, no line, no excerpt — the walk's
//! metadata and a digest, and nothing else. A test builds the whole skeleton from hand-written
//! `FsEntry` values with no file on disk anywhere in it, and a second test indexes a repository
//! whose files carry a unique marker string and asserts the marker appears in no `fs-structural`
//! row.
//!
//! T7 defends against *content* an attacker wrote inside a document. This extractor cannot carry
//! document content anywhere, because it never reads any; the one attacker-influenced input it
//! does touch is the path, which already passes the Slice 5a `canonical_child` guard.

use nerve_core::vocab::{Directness, EvidenceSourceType};

use crate::lang::FileKind;

/// Extractor identity, recorded on every observation and on its `extractor_run` row.
pub const EXTRACTOR_ID: &str = "fs-structural";

/// Extractor version. A change here re-states every filesystem claim, by design.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The only evidence source type this extractor may emit (ADR-0003, THREAT-MODEL.md T7).
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::FilesystemObserved];

/// How directly the filesystem states the structure this extractor reads out of it.
///
/// `Direct`: the directory walk literally found the entry. No resolution step occurred, so
/// ADR-0003 gives it `DIRECT` for the same reason a syntax node's own children are `DIRECT`.
pub const DIRECTNESS: Directness = Directness::Direct;

/// One entry from the discovery walk, and the **only** thing `fs-structural` is allowed to see.
///
/// This type is the content-independence proof. Every field is metadata about a file rather than
/// anything from inside one:
///
/// - `rel_path` — where the walk found it. Already through the Slice 5a path guard.
/// - `kind` — decided by the extension, not by the bytes.
/// - `size_bytes` — `std::fs::metadata`'s length.
/// - `content_hash` — a BLAKE3 digest. A fixed-width fingerprint that identifies a version of the
///   file for freshness checks; it carries no recoverable content, and the extractor never
///   inspects it, only copies it onto the row.
///
/// Adding a field that could hold file text would break the guarantee, which is why the set is
/// small, closed, and documented here rather than left implicit at the call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEntry {
    /// Repository-relative path, `/`-separated.
    pub rel_path: String,
    /// What the walk decided the file is, from its extension.
    pub kind: FileKind,
    /// Size on disk in bytes.
    pub size_bytes: u64,
    /// BLAKE3 digest of the file, carried through to the observation for freshness.
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_only_the_filesystem_source_type() {
        assert_eq!(DECLARED_SOURCE_TYPES.len(), 1);
        assert_eq!(
            DECLARED_SOURCE_TYPES[0],
            EvidenceSourceType::FilesystemObserved
        );
        assert_eq!(DECLARED_SOURCE_TYPES[0].as_str(), "FILESYSTEM_OBSERVED");
        assert_eq!(DIRECTNESS, Directness::Direct);
    }

    /// An `FsEntry` is fully determined by walk metadata: two files with wildly different
    /// contents and the same metadata produce the same entry, and the type offers nowhere to put
    /// the difference.
    #[test]
    fn an_entry_is_only_metadata() {
        let entry = FsEntry {
            rel_path: "docs/ROADMAP.md".to_string(),
            kind: FileKind::Doc,
            size_bytes: 42,
            content_hash: "deadbeef".to_string(),
        };
        assert_eq!(entry.kind.as_str(), "markdown");
        assert_eq!(
            entry,
            FsEntry {
                rel_path: "docs/ROADMAP.md".to_string(),
                kind: FileKind::Doc,
                size_bytes: 42,
                content_hash: "deadbeef".to_string(),
            }
        );
    }
}
