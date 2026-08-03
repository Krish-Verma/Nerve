//! Reading Git objects from `.git`, without a subprocess and without a network client.
//!
//! This module is the whole of Slice 12a. It **reads**, and does nothing else: it creates no
//! entity, writes no database row, changes no schema, adds no CLI command and answers no user
//! question. That is the same split [`crate::coverage`] and [`crate::coverage_ingest`] are in, and
//! for the same reason — Slice 12b builds the historical model on top, and building it against a
//! moving reader is the mistake Slices 8b and 9 were split to avoid.
//!
//! # Why Nerve reads the format itself
//!
//! `docs/plans/slice-12-git-object-access-analysis.md` settles this and it is not relitigated here.
//! In one paragraph: a decompressor is mandatory because every Git object is zlib-deflated;
//! packed objects are mandatory because `git clone` always writes a packfile, so a loose-only
//! reader would pass this repository and fail every real one; `gix` and `git2` were rejected on
//! measured dependency delta and on keeping a network-capable Git implementation out of an
//! offline-first product's tree; and shelling out to `git cat-file` is refused by
//! `crates/nerve-cli/tests/no_subprocess.rs`, which names the `git` binary explicitly. What is left
//! is `flate2` with `rust_backend` — pure Rust, no C — plus a packfile reader written here.
//!
//! # `.git` is attacker-controlled input, and this is a new threat surface
//!
//! Until this module existed Nerve read `.git` only for two plain-text ref files
//! ([`crate::gitinfo`]). Compressed data whose output size is self-described is a classic
//! amplification vector, and a delta chain is a graph a hostile pack can make cyclic. Four bounds
//! answer that, and each has a named test that fails if the bound is removed:
//!
//! | bound | value | what it stops |
//! |---|---|---|
//! | [`MAX_OBJECT_BYTES`] | 64 MiB | one object's inflated size. Applied **while inflating** — see [`inflate`] |
//! | [`MAX_DELTA_DEPTH`] | 64 | an over-long chain, and a cyclic one, by the same mechanism |
//! | [`MAX_PACK_COUNT`] | 256 | a directory of thousands of `.idx` files |
//! | declared-size disagreement | refuse | a loose object whose header and stream disagree: **neither value is trusted** |
//!
//! Git's own default `pack.depth` is 50, so 64 admits every pack Git writes and refuses the chains
//! it does not. A cyclic chain is caught by the depth bound rather than by cycle detection, because
//! the bound is needed anyway and two mechanisms for one hazard is one too many.
//!
//! # Three answers, not two
//!
//! [`ObjectStore::read`] distinguishes:
//!
//! - `Ok(Some(object))` — here it is.
//! - `Ok(None)` — **not present in this store.** A partial clone makes this ordinary rather than
//!   exceptional, and a `REF_DELTA` naming an absent base produces it with a counted reason.
//! - `Err(_)` — the store found something and **refuses** it. Every refusal carries a tag from
//!   [`form`], a closed vocabulary, so a reader can enumerate every way this module can decline to
//!   believe a repository.
//!
//! [`StoreLimits`] is the fourth answer and the one that makes the other three honest: *"there are
//! no more commits"* and *"I cannot see further"* are different statements. A shallow clone's
//! history genuinely ends at a boundary; a promisor remote means a missing object may exist
//! somewhere Nerve is forbidden to look. Both are reported rather than being left for a caller to
//! infer from an empty result — the same three-valued discipline as `bound`/`stale`/`unverified` in
//! Slice 11a and `CoverageEvidence::Absent` in Slice 6b.
//!
//! # What this module deliberately does not do
//!
//! - **It does not verify an object's content against its id.** Git does; Nerve does not, and the
//!   reason is scope: it would mean adding a SHA-1 implementation to detect corruption that
//!   `git fsck` exists for, in a repository Git itself is managing. This is stated here and on
//!   [`StoreLimits`] because an unstated non-check is exactly the kind of thing a later reader
//!   assumes was done. The declared-size checks catch the cases that would otherwise yield
//!   silently wrong bytes.
//! - **It does not read a multi-pack-index or a commit-graph.** Both are caches of what the objects
//!   already say; reading them would be an optimisation whose failure mode is disagreeing with the
//!   source.
//! - **It does not support `.idx` v1.** Superseded in 2007, and refused with the version *stated*
//!   through [`StoreLimits::unsupported_index_versions`] rather than skipped silently.
//! - **It does not support SHA-256 repositories.** Detected from `extensions.objectFormat` and
//!   refused by [`ObjectStore::open`] with the format named.

pub mod commit;
pub mod inflate;
pub mod loose;
pub mod oid;
pub mod pack;
pub mod packidx;
pub mod store;
#[cfg(test)]
mod testpack;
pub mod tree;

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use commit::{parse_commit, Commit, Identity, MAX_COMMIT_PARENTS};
pub use inflate::{inflate_bounded, MAX_OBJECT_BYTES};
pub use loose::{loose_path, read_loose};
pub use oid::{Oid, OID_BYTES, OID_HEX_CHARS};
pub use pack::{apply_delta, PackEntry, PackFile, MAX_DELTA_DEPTH};
pub use packidx::{PackIndex, SUPPORTED_IDX_VERSION};
pub use store::{
    ObjectStore, StoreLimits, MAX_ALTERNATES_ENTRIES, MAX_CONFIG_BYTES, MAX_IDX_BYTES,
    MAX_PACK_COUNT, MAX_SHALLOW_ENTRIES,
};
pub use tree::{parse_tree, TreeEntry};

/// Convenience alias for this module's own error type.
///
/// Deliberately **not** [`crate::Result`]: nothing in Slice 12a is reachable from the indexing
/// pipeline, so bridging into [`crate::IndexError`] would add a public error variant that no code
/// path can produce. Slice 12b adds that bridge when it has a caller for it.
pub type Result<T> = std::result::Result<T, Error>;

/// One of the four Git object types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    /// A commit.
    Commit,
    /// A tree.
    Tree,
    /// A blob.
    Blob,
    /// An annotated tag.
    Tag,
}

impl ObjectKind {
    /// The type word as it appears in a loose object's header.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Tree => "tree",
            Self::Blob => "blob",
            Self::Tag => "tag",
        }
    }

    /// Parse a loose object header's type word. `None` for anything else — never guessed at.
    pub fn from_word(word: &[u8]) -> Option<Self> {
        match word {
            b"commit" => Some(Self::Commit),
            b"tree" => Some(Self::Tree),
            b"blob" => Some(Self::Blob),
            b"tag" => Some(Self::Tag),
            _ => None,
        }
    }

    /// Parse a packfile entry header's 3-bit type field.
    ///
    /// `1..=4` are the object types. `0` is invalid, `5` is reserved and has never been assigned,
    /// and `6`/`7` are the two delta forms — which are not object *types* and are therefore not
    /// this function's answer.
    pub const fn from_pack_type(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Commit),
            2 => Some(Self::Tree),
            3 => Some(Self::Blob),
            4 => Some(Self::Tag),
            _ => None,
        }
    }

    /// Every kind, in declaration order.
    pub const ALL: [Self; 4] = [Self::Commit, Self::Tree, Self::Blob, Self::Tag];
}

/// A Git object: its type, and its bytes exactly as Git stored them.
///
/// The payload is the object's content **without** the loose-object header, which is the form every
/// Git object id is computed over and the form [`parse_commit`] and [`parse_tree`] expect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Object {
    /// A commit object's bytes.
    Commit(Vec<u8>),
    /// A tree object's bytes.
    Tree(Vec<u8>),
    /// A blob object's bytes.
    Blob(Vec<u8>),
    /// An annotated tag object's bytes.
    Tag(Vec<u8>),
}

impl Object {
    /// Build an object of `kind` from its content bytes.
    pub fn new(kind: ObjectKind, data: Vec<u8>) -> Self {
        match kind {
            ObjectKind::Commit => Self::Commit(data),
            ObjectKind::Tree => Self::Tree(data),
            ObjectKind::Blob => Self::Blob(data),
            ObjectKind::Tag => Self::Tag(data),
        }
    }

    /// Which of the four types this is.
    pub const fn kind(&self) -> ObjectKind {
        match self {
            Self::Commit(_) => ObjectKind::Commit,
            Self::Tree(_) => ObjectKind::Tree,
            Self::Blob(_) => ObjectKind::Blob,
            Self::Tag(_) => ObjectKind::Tag,
        }
    }

    /// The content bytes.
    pub fn data(&self) -> &[u8] {
        match self {
            Self::Commit(data) | Self::Tree(data) | Self::Blob(data) | Self::Tag(data) => data,
        }
    }

    /// Consume the object and take its bytes.
    pub fn into_data(self) -> Vec<u8> {
        match self {
            Self::Commit(data) | Self::Tree(data) | Self::Blob(data) | Self::Tag(data) => data,
        }
    }
}

/// Refusal tags. **Closed**, so a reader can enumerate every way this module declines to believe a
/// repository — the same convention as [`crate::coverage::form`] and [`crate::trace::form`].
pub mod form {
    /// An object's inflated size exceeded [`super::MAX_OBJECT_BYTES`].
    pub const OBJECT_TOO_LARGE: &str = "object-too-large";
    /// A stream was not valid zlib, or ended mid-stream.
    pub const INFLATE_FAILED: &str = "inflate-failed";
    /// A loose object's header had no NUL terminator, no space, or a size that is not a number.
    pub const LOOSE_HEADER_MALFORMED: &str = "loose-header-malformed";
    /// A loose object's header named a type word that is not one of the four.
    pub const LOOSE_UNKNOWN_TYPE: &str = "loose-unknown-type";
    /// A loose object's declared size and its inflated stream disagree. **Neither is trusted.**
    pub const LOOSE_DECLARED_SIZE_DISAGREES: &str = "loose-declared-size-disagrees";
    /// A `.idx` did not begin with the v2+ magic, which is what a v1 index looks like.
    pub const IDX_BAD_MAGIC: &str = "idx-bad-magic";
    /// A `.idx` declared a version this reader does not support. The version is stated.
    pub const IDX_UNSUPPORTED_VERSION: &str = "idx-unsupported-version";
    /// A `.idx` fanout table was not non-decreasing, so its search ranges are meaningless.
    pub const IDX_FANOUT_NOT_MONOTONIC: &str = "idx-fanout-not-monotonic";
    /// A `.idx`'s length and its declared object count do not agree.
    pub const IDX_TRUNCATED: &str = "idx-truncated";
    /// A `.idx` was larger than [`super::MAX_IDX_BYTES`]. It is never read.
    pub const IDX_TOO_LARGE: &str = "idx-too-large";
    /// A `.idx` offset named an entry past the end of the 64-bit overflow table.
    pub const IDX_LARGE_OFFSET_OUT_OF_RANGE: &str = "idx-large-offset-out-of-range";
    /// A `.pack` did not begin with `PACK`.
    pub const PACK_BAD_MAGIC: &str = "pack-bad-magic";
    /// A `.pack` declared a version other than 2 or 3.
    pub const PACK_UNSUPPORTED_VERSION: &str = "pack-unsupported-version";
    /// A read ran past the end of a `.pack`.
    pub const PACK_TRUNCATED: &str = "pack-truncated";
    /// A pack entry header named type 0 or the reserved type 5.
    pub const PACK_ENTRY_UNKNOWN_TYPE: &str = "pack-entry-unknown-type";
    /// A pack entry's declared size and its inflated stream disagree.
    pub const PACK_ENTRY_SIZE_DISAGREES: &str = "pack-entry-size-disagrees";
    /// An `OFS_DELTA`'s backward offset landed outside the pack, or on itself.
    pub const OFS_DELTA_BAD_OFFSET: &str = "ofs-delta-bad-offset";
    /// A delta chain was longer than [`super::MAX_DELTA_DEPTH`], or cyclic.
    pub const DELTA_DEPTH_EXCEEDED: &str = "delta-depth-exceeded";
    /// A delta instruction stream was malformed — a zero opcode, or a copy outside the base.
    pub const DELTA_MALFORMED: &str = "delta-malformed";
    /// A delta's declared result size and what it produced disagree.
    pub const DELTA_RESULT_SIZE_DISAGREES: &str = "delta-result-size-disagrees";
    /// A delta's declared base size and the base it was applied to disagree.
    pub const DELTA_BASE_SIZE_DISAGREES: &str = "delta-base-size-disagrees";
    /// A delta named a base object no source in this store holds. Counted; the read is `Ok(None)`.
    pub const DELTA_BASE_MISSING: &str = "delta-base-missing";
    /// A commit object's headers were not the shape the format defines.
    pub const COMMIT_HEADER_MALFORMED: &str = "commit-header-malformed";
    /// A commit named more than [`super::MAX_COMMIT_PARENTS`] parents.
    pub const COMMIT_PARENTS_EXCEEDED: &str = "commit-parents-exceeded";
    /// A tree entry was truncated, or named an invalid mode or name.
    pub const TREE_ENTRY_MALFORMED: &str = "tree-entry-malformed";
    /// `extensions.objectFormat` named a hash this reader does not implement.
    pub const UNSUPPORTED_OBJECT_FORMAT: &str = "unsupported-object-format";
    /// The path given to [`super::ObjectStore::open`] is not a directory.
    pub const NOT_A_DIRECTORY: &str = "not-a-directory";
    /// The filesystem refused a read for a reason of its own.
    pub const IO: &str = "io";
    /// `.idx` files past [`super::MAX_PACK_COUNT`]. Counted once per skipped pack.
    pub const PACK_COUNT_EXCEEDED: &str = "pack-count-exceeded";
    /// An alternates entry that is empty, carries a control character, does not exist, is not a
    /// directory, or names the store's own object directory.
    pub const ALTERNATE_REFUSED_SHAPE: &str = "alternate-refused-shape";
    /// An alternates entry that resolved outside the repository root.
    pub const ALTERNATE_ESCAPES_REPOSITORY_ROOT: &str = "alternate-escapes-repository-root";
    /// An alternate that carried alternates of its own. The second hop is refused.
    pub const ALTERNATE_CHAIN_REFUSED: &str = "alternate-chain-refused";
    /// Alternates entries past the per-file bound.
    pub const ALTERNATES_ENTRIES_EXCEEDED: &str = "alternates-entries-exceeded";
    /// `.git/shallow` lines past the bound.
    pub const SHALLOW_ENTRIES_EXCEEDED: &str = "shallow-entries-exceeded";
    /// A `.git/shallow` line that is not a 40-character hex object id.
    pub const SHALLOW_ENTRY_UNPARSED: &str = "shallow-entry-unparsed";
    /// `.git/config` was larger than this reader will read. It is not parsed.
    pub const CONFIG_TOO_LARGE: &str = "config-too-large";
    /// A worktree's `commondir` was empty, carried a control character, or did not resolve.
    pub const COMMONDIR_REFUSED: &str = "commondir-refused";

    /// Every tag in this module, in declaration order.
    pub const ALL: [&str; 37] = [
        OBJECT_TOO_LARGE,
        INFLATE_FAILED,
        LOOSE_HEADER_MALFORMED,
        LOOSE_UNKNOWN_TYPE,
        LOOSE_DECLARED_SIZE_DISAGREES,
        IDX_BAD_MAGIC,
        IDX_UNSUPPORTED_VERSION,
        IDX_FANOUT_NOT_MONOTONIC,
        IDX_TRUNCATED,
        IDX_TOO_LARGE,
        IDX_LARGE_OFFSET_OUT_OF_RANGE,
        PACK_BAD_MAGIC,
        PACK_UNSUPPORTED_VERSION,
        PACK_TRUNCATED,
        PACK_ENTRY_UNKNOWN_TYPE,
        PACK_ENTRY_SIZE_DISAGREES,
        OFS_DELTA_BAD_OFFSET,
        DELTA_DEPTH_EXCEEDED,
        DELTA_MALFORMED,
        DELTA_RESULT_SIZE_DISAGREES,
        DELTA_BASE_SIZE_DISAGREES,
        DELTA_BASE_MISSING,
        COMMIT_HEADER_MALFORMED,
        COMMIT_PARENTS_EXCEEDED,
        TREE_ENTRY_MALFORMED,
        UNSUPPORTED_OBJECT_FORMAT,
        NOT_A_DIRECTORY,
        IO,
        PACK_COUNT_EXCEEDED,
        ALTERNATE_REFUSED_SHAPE,
        ALTERNATE_ESCAPES_REPOSITORY_ROOT,
        ALTERNATE_CHAIN_REFUSED,
        ALTERNATES_ENTRIES_EXCEEDED,
        SHALLOW_ENTRIES_EXCEEDED,
        SHALLOW_ENTRY_UNPARSED,
        CONFIG_TOO_LARGE,
        COMMONDIR_REFUSED,
    ];
}

/// A refusal. Every variant maps to exactly one tag in [`form`], via [`Error::form`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The filesystem refused a read.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// The path given to [`ObjectStore::open`] is not a directory.
    #[error("not a git directory: {0}")]
    NotADirectory(PathBuf),

    /// `extensions.objectFormat` named a hash this reader does not implement.
    ///
    /// Refused rather than misread. Reading a SHA-256 repository as if it were SHA-1 would produce
    /// 20-byte prefixes of real 32-byte ids and treat them as identities.
    #[error("unsupported extensions.objectFormat: {0}")]
    UnsupportedObjectFormat(String),

    /// An object's inflated size exceeded the bound. Refused **during** inflate.
    #[error("object exceeds the {limit}-byte bound (at least {at_least} bytes)")]
    ObjectTooLarge {
        /// The bound that was exceeded.
        limit: usize,
        /// The smallest size known to exceed the bound: either the bytes produced before the
        /// refusal fired — never the whole object — or, when a header or delta declared a size
        /// before anything was inflated, the size it declared.
        at_least: usize,
    },

    /// A zlib stream was invalid or truncated.
    #[error("inflate failed: {0}")]
    Inflate(String),

    /// A loose object's header was not `<type> <size>\0`.
    #[error("loose object header malformed")]
    LooseHeaderMalformed,

    /// A loose object's header named a type word that is not one of the four.
    #[error("loose object type is not one of commit/tree/blob/tag")]
    LooseUnknownType,

    /// A loose object's declared size and its inflated stream disagree.
    #[error("loose object header declares {declared} bytes, stream has {actual}")]
    LooseDeclaredSizeDisagrees {
        /// What the header said.
        declared: u64,
        /// What the stream actually held.
        actual: usize,
    },

    /// A `.idx` did not begin with the v2+ magic, so it is a v1 index or not an index at all.
    #[error("pack index has no v2 magic")]
    IdxBadMagic,

    /// A `.idx` declared an unsupported version.
    #[error("pack index version {0} is not supported")]
    IdxUnsupportedVersion(u32),

    /// A `.idx` fanout table was not non-decreasing.
    #[error("pack index fanout table is not monotonic")]
    IdxFanoutNotMonotonic,

    /// A `.idx`'s length and its declared object count do not agree.
    #[error("pack index truncated or over-long: {0}")]
    IdxTruncated(String),

    /// A `.idx` was larger than this reader will load.
    ///
    /// A `.idx` is read whole, because a lookup needs random access to its tables, so its length is
    /// an allocation the repository chooses. The length is checked from the file's metadata, so an
    /// over-long index costs no bytes at all.
    #[error("pack index is {length} bytes, over the {limit}-byte bound")]
    IdxTooLarge {
        /// The bound.
        limit: u64,
        /// The file's length, from its metadata.
        length: u64,
    },

    /// A `.idx` offset named an entry past the end of the 64-bit overflow table.
    #[error("pack index large-offset entry {0} is out of range")]
    IdxLargeOffsetOutOfRange(u32),

    /// A `.pack` did not begin with `PACK`.
    #[error("pack has no PACK magic")]
    PackBadMagic,

    /// A `.pack` declared a version other than 2 or 3.
    #[error("pack version {0} is not supported")]
    PackUnsupportedVersion(u32),

    /// A read ran past the end of a `.pack`.
    #[error("pack truncated: wanted {wanted} bytes at offset {offset}, pack is {length} bytes")]
    PackTruncated {
        /// Where the read started.
        offset: u64,
        /// How much it wanted.
        wanted: usize,
        /// How long the pack is.
        length: u64,
    },

    /// A pack entry header named type 0 or the reserved type 5.
    #[error("pack entry type {0} is not a Git object type")]
    PackEntryUnknownType(u8),

    /// A pack entry's declared size and its inflated stream disagree.
    #[error("pack entry declares {declared} bytes, stream has {actual}")]
    PackEntrySizeDisagrees {
        /// What the entry header said.
        declared: u64,
        /// What the stream actually held.
        actual: usize,
    },

    /// An `OFS_DELTA`'s backward offset landed outside the pack, or on itself.
    #[error("OFS_DELTA at offset {offset} names a base offset outside the pack")]
    OfsDeltaBadOffset {
        /// The delta entry's own offset.
        offset: u64,
    },

    /// A delta chain was longer than the bound, or cyclic.
    #[error("delta chain deeper than {MAX_DELTA_DEPTH}")]
    DeltaDepthExceeded,

    /// A delta instruction stream was malformed.
    #[error("delta instruction stream malformed: {0}")]
    DeltaMalformed(String),

    /// A delta's declared result size and what it produced disagree.
    #[error("delta declares a {declared}-byte result, produced {actual}")]
    DeltaResultSizeDisagrees {
        /// What the delta header said.
        declared: u64,
        /// What applying it produced.
        actual: usize,
    },

    /// A delta's declared base size and the base it was applied to disagree.
    #[error("delta declares a {declared}-byte base, base is {actual} bytes")]
    DeltaBaseSizeDisagrees {
        /// What the delta header said.
        declared: u64,
        /// How big the base actually is.
        actual: usize,
    },

    /// A commit object's headers were not the shape the format defines.
    #[error("commit header malformed: {0}")]
    CommitHeaderMalformed(String),

    /// A commit named more than [`MAX_COMMIT_PARENTS`] parents.
    #[error("commit names more than {MAX_COMMIT_PARENTS} parents")]
    CommitParentsExceeded,

    /// A tree entry was truncated, or named an invalid mode or name.
    #[error("tree entry malformed: {0}")]
    TreeEntryMalformed(String),
}

impl Error {
    /// The [`form`] tag for this refusal.
    pub const fn form(&self) -> &'static str {
        match self {
            Self::Io(_) => form::IO,
            Self::NotADirectory(_) => form::NOT_A_DIRECTORY,
            Self::UnsupportedObjectFormat(_) => form::UNSUPPORTED_OBJECT_FORMAT,
            Self::ObjectTooLarge { .. } => form::OBJECT_TOO_LARGE,
            Self::Inflate(_) => form::INFLATE_FAILED,
            Self::LooseHeaderMalformed => form::LOOSE_HEADER_MALFORMED,
            Self::LooseUnknownType => form::LOOSE_UNKNOWN_TYPE,
            Self::LooseDeclaredSizeDisagrees { .. } => form::LOOSE_DECLARED_SIZE_DISAGREES,
            Self::IdxBadMagic => form::IDX_BAD_MAGIC,
            Self::IdxUnsupportedVersion(_) => form::IDX_UNSUPPORTED_VERSION,
            Self::IdxFanoutNotMonotonic => form::IDX_FANOUT_NOT_MONOTONIC,
            Self::IdxTruncated(_) => form::IDX_TRUNCATED,
            Self::IdxTooLarge { .. } => form::IDX_TOO_LARGE,
            Self::IdxLargeOffsetOutOfRange(_) => form::IDX_LARGE_OFFSET_OUT_OF_RANGE,
            Self::PackBadMagic => form::PACK_BAD_MAGIC,
            Self::PackUnsupportedVersion(_) => form::PACK_UNSUPPORTED_VERSION,
            Self::PackTruncated { .. } => form::PACK_TRUNCATED,
            Self::PackEntryUnknownType(_) => form::PACK_ENTRY_UNKNOWN_TYPE,
            Self::PackEntrySizeDisagrees { .. } => form::PACK_ENTRY_SIZE_DISAGREES,
            Self::OfsDeltaBadOffset { .. } => form::OFS_DELTA_BAD_OFFSET,
            Self::DeltaDepthExceeded => form::DELTA_DEPTH_EXCEEDED,
            Self::DeltaMalformed(_) => form::DELTA_MALFORMED,
            Self::DeltaResultSizeDisagrees { .. } => form::DELTA_RESULT_SIZE_DISAGREES,
            Self::DeltaBaseSizeDisagrees { .. } => form::DELTA_BASE_SIZE_DISAGREES,
            Self::CommitHeaderMalformed(_) => form::COMMIT_HEADER_MALFORMED,
            Self::CommitParentsExceeded => form::COMMIT_PARENTS_EXCEEDED,
            Self::TreeEntryMalformed(_) => form::TREE_ENTRY_MALFORMED,
        }
    }
}

/// What a store refused, and how often.
///
/// The same shape as [`crate::coverage::CoverageCounters`] and [`crate::markdown::ScanCounters`],
/// deliberately: one counting convention for every reader of attacker-controlled input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreCounters {
    /// Refusals by [`form`] tag.
    pub refused: BTreeMap<String, usize>,
}

impl StoreCounters {
    /// Count one refusal under `tag`.
    pub fn count(&mut self, tag: &'static str) {
        *self.refused.entry(tag.to_string()).or_insert(0) += 1;
    }

    /// How many times `tag` was counted.
    pub fn get(&self, tag: &str) -> usize {
        self.refused.get(tag).copied().unwrap_or(0)
    }

    /// Total refusals across every form.
    pub fn total(&self) -> usize {
        self.refused.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_tag_is_distinct_and_non_empty() {
        let mut sorted = form::ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), form::ALL.len(), "two form tags collide");
        assert!(form::ALL.iter().all(|tag| !tag.is_empty()));
    }

    /// Every refusal a caller can receive must be nameable from the closed vocabulary. A variant
    /// whose tag is not in `ALL` would be a refusal nobody can enumerate.
    #[test]
    fn every_error_variant_maps_into_the_closed_vocabulary() {
        let samples = [
            Error::Io(std::io::Error::other("x")),
            Error::NotADirectory(PathBuf::from("/nowhere")),
            Error::UnsupportedObjectFormat("sha256".to_string()),
            Error::ObjectTooLarge {
                limit: 1,
                at_least: 2,
            },
            Error::Inflate("x".to_string()),
            Error::LooseHeaderMalformed,
            Error::LooseUnknownType,
            Error::LooseDeclaredSizeDisagrees {
                declared: 1,
                actual: 2,
            },
            Error::IdxBadMagic,
            Error::IdxUnsupportedVersion(1),
            Error::IdxFanoutNotMonotonic,
            Error::IdxTruncated("x".to_string()),
            Error::IdxTooLarge {
                limit: 1,
                length: 2,
            },
            Error::IdxLargeOffsetOutOfRange(0),
            Error::PackBadMagic,
            Error::PackUnsupportedVersion(9),
            Error::PackTruncated {
                offset: 0,
                wanted: 1,
                length: 0,
            },
            Error::PackEntryUnknownType(5),
            Error::PackEntrySizeDisagrees {
                declared: 1,
                actual: 2,
            },
            Error::OfsDeltaBadOffset { offset: 0 },
            Error::DeltaDepthExceeded,
            Error::DeltaMalformed("x".to_string()),
            Error::DeltaResultSizeDisagrees {
                declared: 1,
                actual: 2,
            },
            Error::DeltaBaseSizeDisagrees {
                declared: 1,
                actual: 2,
            },
            Error::CommitHeaderMalformed("x".to_string()),
            Error::CommitParentsExceeded,
            Error::TreeEntryMalformed("x".to_string()),
        ];
        for error in &samples {
            assert!(
                form::ALL.contains(&error.form()),
                "{} is not in form::ALL",
                error.form()
            );
            // A refusal must also say something; an empty message is not a reason.
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn object_kind_round_trips_through_its_header_word() {
        for kind in ObjectKind::ALL {
            assert_eq!(ObjectKind::from_word(kind.as_str().as_bytes()), Some(kind));
        }
        assert_eq!(ObjectKind::from_word(b"COMMIT"), None);
        assert_eq!(ObjectKind::from_word(b""), None);
        assert_eq!(ObjectKind::from_word(b"commitx"), None);
    }

    /// Types 0 and 5 are not object types, and 6/7 are deltas rather than types. A reader that
    /// mapped 5 onto anything would be inventing a type the format has never assigned.
    #[test]
    fn pack_type_five_and_zero_are_not_object_types() {
        assert_eq!(ObjectKind::from_pack_type(1), Some(ObjectKind::Commit));
        assert_eq!(ObjectKind::from_pack_type(2), Some(ObjectKind::Tree));
        assert_eq!(ObjectKind::from_pack_type(3), Some(ObjectKind::Blob));
        assert_eq!(ObjectKind::from_pack_type(4), Some(ObjectKind::Tag));
        for value in [0u8, 5, 6, 7, 8, 255] {
            assert_eq!(ObjectKind::from_pack_type(value), None, "type {value}");
        }
    }

    #[test]
    fn object_carries_its_kind_and_its_bytes() {
        for kind in ObjectKind::ALL {
            let object = Object::new(kind, b"payload".to_vec());
            assert_eq!(object.kind(), kind);
            assert_eq!(object.data(), b"payload");
            assert_eq!(object.clone().into_data(), b"payload".to_vec());
        }
    }

    #[test]
    fn counters_count_and_total() {
        let mut counters = StoreCounters::default();
        assert_eq!(counters.total(), 0);
        assert_eq!(counters.get(form::PACK_TRUNCATED), 0);
        counters.count(form::PACK_TRUNCATED);
        counters.count(form::PACK_TRUNCATED);
        counters.count(form::DELTA_BASE_MISSING);
        assert_eq!(counters.get(form::PACK_TRUNCATED), 2);
        assert_eq!(counters.get(form::DELTA_BASE_MISSING), 1);
        assert_eq!(counters.total(), 3);
    }
}
