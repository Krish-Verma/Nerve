//! The façade: worktree resolution, alternates, pack discovery, and loose-then-pack lookup.
//!
//! # What `open` has to work out before it can read anything
//!
//! A `.git` directory is not one thing. Four cases have to be distinguished, and getting any of them
//! wrong produces a store that opens successfully and finds nothing — the worst available failure,
//! because it reads as "this repository has no history":
//!
//! 1. **An ordinary repository.** `objects/` is right there.
//! 2. **A linked worktree.** `.git` is a *file* containing `gitdir: …`, which
//!    [`crate::gitinfo::git_dir`] already resolves and which is reused rather than reimplemented.
//!    But the directory it names has **no `objects/` of its own** — a worktree's private directory
//!    holds `HEAD`, `index` and `commondir`, and the objects live in the main repository. So
//!    `commondir` must be followed as well. A store that resolved only the path would open and read
//!    nothing at all.
//! 3. **A shared object store**, named by `objects/info/alternates`. Followed **one hop**, guarded,
//!    and counted either way.
//! 4. **A repository that cannot show Nerve everything** — shallow, or a partial clone with a
//!    promisor remote. Reported through [`StoreLimits`] rather than left to be inferred from an
//!    empty result.
//!
//! # The alternates guard, stated plainly
//!
//! `objects/info/alternates` is repository content and therefore attacker-controlled (THREAT-MODEL
//! A1). It names a directory, and following it means reading files from wherever it points. Four
//! rules apply, and all four count on refusal:
//!
//! - **Shape.** Non-empty, no C0 control character — the same rule
//!   [`crate::discover::canonical_child`] applies, and for the same reason: a control byte in a path
//!   attacks identity rather than containment. It must resolve to an existing directory, and it must
//!   not name the store's own object directory.
//! - **Containment.** It must resolve **inside the repository root**, which is derived from the
//!   common git directory rather than passed in — [`ObjectStore::open`] takes only a git directory,
//!   so there is no root parameter to guard against. Nerve does not read another repository's object
//!   store because a file in this one asked it to.
//! - **One hop.** An alternate's own alternates are refused and counted. An unbounded chain is a
//!   path-traversal surface aimed at arbitrary directories.
//! - **A count.** Entries past [`MAX_ALTERNATES_ENTRIES`] are refused, not walked.
//!
//! The containment rule has a consequence worth stating rather than discovering later: a repository
//! cloned with `git clone --shared` or `--reference` has an alternate pointing **outside** its own
//! root, so that alternate is refused and counted. The objects it holds then read as `Ok(None)` and
//! [`StoreLimits::alternates_refused`] says why. That is the honest answer — *"I cannot see
//! further"* — and it is the same shape of answer as a shallow boundary.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::inflate::MAX_OBJECT_BYTES;
use super::loose::{loose_path, read_loose};
use super::oid::Oid;
use super::pack::{apply_delta, PackEntry, PackFile, MAX_DELTA_DEPTH};
use super::packidx::PackIndex;
use super::{form, Error, Object, Result, StoreCounters};

/// Packs one store will load, across its own object directory and any alternate.
///
/// Bounded at 256. A repository Git has ever run `gc` on has a handful; a directory of thousands of
/// `.idx` files is a directory someone built to make a reader walk it. Applied across the whole store
/// rather than per directory, so that following an alternate cannot double the budget.
pub const MAX_PACK_COUNT: usize = 256;

/// Entries one `objects/info/alternates` may contribute.
pub const MAX_ALTERNATES_ENTRIES: usize = 32;

/// Object ids `.git/shallow` may list.
pub const MAX_SHALLOW_ENTRIES: usize = 65_536;

/// Bytes of `.git/config` this reader will read.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Bytes of `commondir`, `shallow` or `alternates` this reader will read.
///
/// `shallow` is the largest of the three: [`MAX_SHALLOW_ENTRIES`] ids at 41 bytes each, plus slack.
const MAX_POINTER_FILE_BYTES: u64 = (MAX_SHALLOW_ENTRIES as u64) * 41 + 4096;

/// Bytes of a `.idx` this reader will load.
///
/// A `.idx` is read whole because a lookup needs random access to its tables, so its size is an
/// allocation an attacker chooses. 512 MiB is roughly 18 million objects — more than twice the
/// Linux kernel's object count — so it refuses nothing real and bounds what one file can cost.
pub const MAX_IDX_BYTES: u64 = 512 * 1024 * 1024;

/// What this store **cannot** see, and why.
///
/// The point of the type. An empty result from [`ObjectStore::read`] has several possible causes and
/// a caller must never have to guess which: a shallow clone's history genuinely ends at a boundary,
/// a promisor remote holds objects Nerve is forbidden to fetch, an unreadable `.idx` means a pack's
/// worth of objects is invisible, and a refused alternate means a whole object store is.
///
/// # A limitation recorded here on purpose
///
/// **Object content is never verified against its object id.** Git verifies; this reader does not,
/// because doing so would mean adding a SHA-1 implementation to detect corruption `git fsck` exists
/// for, in a repository Git itself is managing. The declared-size checks on loose objects, pack
/// entries and deltas catch the cases that would otherwise produce silently wrong bytes. It is
/// stated on this type because an unstated non-check is exactly the kind of thing a later reader
/// assumes was done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreLimits {
    /// The boundary commits from `.git/shallow`, when the repository is shallow.
    ///
    /// `None` means **not shallow**. `Some(vec![])` would mean "shallow with no boundary", which is
    /// a different claim and is never produced. A boundary commit is where history stops being
    /// visible; it is **not** a root commit and must never be treated as one.
    pub shallow: Option<Vec<Oid>>,
    /// Whether this is a partial clone, so a missing object may exist on a remote.
    ///
    /// Fetching it would need the network `CLAUDE.md` §2 forbids, so the honest report is that this
    /// store cannot see everything.
    pub promisor: bool,
    /// `.idx` versions found and refused, in discovery order. Usually `[1]` when non-empty.
    pub unsupported_index_versions: Vec<u32>,
    /// Alternates followed. At most one, by decision.
    pub alternates_followed: usize,
    /// Alternates refused — shape, containment, chain, or the entry bound.
    pub alternates_refused: usize,
    /// Packs this store is serving.
    pub packs_loaded: usize,
    /// Packs refused: an unreadable `.idx`, an unreadable `.pack`, or the pack bound.
    pub packs_refused: usize,
}

/// One object directory and the packs inside it.
#[derive(Debug)]
struct ObjectSource {
    objects_dir: PathBuf,
    packs: Vec<LoadedPack>,
}

#[derive(Debug)]
struct LoadedPack {
    index: PackIndex,
    pack: PackFile,
}

/// A read-only view of the Git objects one repository can see.
#[derive(Debug)]
pub struct ObjectStore {
    git_dir: PathBuf,
    common_dir: PathBuf,
    repository_root: PathBuf,
    sources: Vec<ObjectSource>,
    limits: StoreLimits,
    counters: RefCell<StoreCounters>,
}

impl ObjectStore {
    /// Open the object store for a resolved git directory.
    ///
    /// `git_dir` is the directory itself — [`crate::gitinfo::git_dir`] resolves the
    /// `.git`-is-a-file worktree case and that resolution is reused rather than reimplemented. This
    /// function then follows `commondir`, because a linked worktree's private directory has no
    /// `objects/`.
    ///
    /// Refuses, rather than opening, when `extensions.objectFormat` names a hash this reader does
    /// not implement. There is no `StoreLimits` to report that through — a store that refuses to
    /// exist cannot carry limits — so the format is named in the error instead.
    pub fn open(git_dir: &Path) -> Result<Self> {
        if !git_dir.is_dir() {
            return Err(Error::NotADirectory(git_dir.to_path_buf()));
        }
        let git_dir = std::fs::canonicalize(git_dir)?;
        let mut counters = StoreCounters::default();
        let mut limits = StoreLimits::default();

        let common_dir = resolve_commondir(&git_dir, &mut counters);
        // The containment root for alternates. Derived rather than passed: `open` takes a git
        // directory, and in the worktree case that directory is itself outside the repository.
        let repository_root = common_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| common_dir.clone());

        let config = read_config(&common_dir, &mut counters);
        if let Some(format) = &config.object_format {
            if format != "sha1" {
                return Err(Error::UnsupportedObjectFormat(format.clone()));
            }
        }

        let objects_dir = common_dir.join("objects");
        limits.shallow = read_shallow(&common_dir, &mut counters);
        limits.promisor = config.partial_clone || has_promisor_marker(&objects_dir);

        let mut budget = MAX_PACK_COUNT;
        let mut sources = vec![load_source(
            objects_dir.clone(),
            &mut budget,
            &mut counters,
            &mut limits,
        )];

        for alternate in read_alternates(
            &objects_dir,
            &objects_dir,
            &repository_root,
            &mut counters,
            &mut limits,
        ) {
            // One hop. The alternate's own alternates are refused and counted, never followed.
            refuse_second_hop(&alternate, &mut counters, &mut limits);
            limits.alternates_followed += 1;
            sources.push(load_source(
                alternate,
                &mut budget,
                &mut counters,
                &mut limits,
            ));
        }

        Ok(Self {
            git_dir,
            common_dir,
            repository_root,
            sources,
            limits,
            counters: RefCell::new(counters),
        })
    }

    /// The git directory this store was opened on.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The common git directory — the same as [`Self::git_dir`] unless this is a linked worktree.
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    /// The directory alternates are required to resolve inside.
    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    /// What this store cannot see, and why. Never inferred by the caller from an empty result.
    pub fn limits(&self) -> &StoreLimits {
        &self.limits
    }

    /// What this store has refused, by [`form`] tag.
    ///
    /// A snapshot: refusals accumulate as objects are read, so this is the total so far rather than
    /// a fixed property of the store.
    pub fn counters(&self) -> StoreCounters {
        self.counters.borrow().clone()
    }

    /// Read one object.
    ///
    /// `Ok(None)` means **not present in this store** — a different fact from an error and from a
    /// refusal. A partial clone makes it the ordinary case rather than an exception, and a delta
    /// whose base is absent produces it with [`form::DELTA_BASE_MISSING`] counted.
    pub fn read(&self, oid: &Oid) -> Result<Option<Object>> {
        self.read_at_depth(oid, 0)
    }

    /// Whether any source in this store holds `oid`, without reconstructing it.
    ///
    /// Cheaper than [`Self::read`] for a delta, because no chain is followed. It answers "is it
    /// here", which is not the same question as "can it be reconstructed" — an entry whose base is
    /// missing is present and unreadable.
    pub fn contains(&self, oid: &Oid) -> Result<bool> {
        for source in &self.sources {
            if loose_path(&source.objects_dir, oid).exists() {
                return Ok(true);
            }
            for pack in &source.packs {
                if pack.index.offset_of(oid)?.is_some() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn read_at_depth(&self, oid: &Oid, depth: usize) -> Result<Option<Object>> {
        if depth > MAX_DELTA_DEPTH {
            return Err(Error::DeltaDepthExceeded);
        }
        for source in &self.sources {
            // Loose before packed, within each source, which is the order Git itself looks: a
            // freshly written object is loose and a packed copy of it, if any, is identical.
            if let Some(object) = read_loose(&loose_path(&source.objects_dir, oid))? {
                return Ok(Some(object));
            }
            for pack in &source.packs {
                if let Some(offset) = pack.index.offset_of(oid)? {
                    return self.resolve(pack, offset, depth);
                }
            }
        }
        Ok(None)
    }

    /// Reconstruct the entry at `offset` in `pack`, following any delta.
    fn resolve(&self, pack: &LoadedPack, offset: u64, depth: usize) -> Result<Option<Object>> {
        if depth > MAX_DELTA_DEPTH {
            return Err(Error::DeltaDepthExceeded);
        }
        match pack.pack.read_entry(offset)? {
            PackEntry::Object { kind, data } => Ok(Some(Object::new(kind, data))),
            PackEntry::OfsDelta { base_offset, delta } => {
                let Some(base) = self.resolve(pack, base_offset, depth + 1)? else {
                    self.counters.borrow_mut().count(form::DELTA_BASE_MISSING);
                    return Ok(None);
                };
                self.combine(base, &delta)
            }
            PackEntry::RefDelta { base, delta } => {
                let Some(base) = self.read_at_depth(&base, depth + 1)? else {
                    self.counters.borrow_mut().count(form::DELTA_BASE_MISSING);
                    return Ok(None);
                };
                self.combine(base, &delta)
            }
        }
    }

    /// Apply `delta` to `base`, keeping the base's type.
    ///
    /// A delta carries no type of its own: the reconstructed object is whatever the base was. That
    /// is also why a chain must be resolved from the base outwards rather than guessed at.
    fn combine(&self, base: Object, delta: &[u8]) -> Result<Option<Object>> {
        let kind = base.kind();
        let data = apply_delta(base.data(), delta)?;
        if data.len() > MAX_OBJECT_BYTES {
            return Err(Error::ObjectTooLarge {
                limit: MAX_OBJECT_BYTES,
                at_least: data.len(),
            });
        }
        Ok(Some(Object::new(kind, data)))
    }
}

/// Follow `commondir` if this git directory is a linked worktree's.
///
/// Falls back to `git_dir` itself, with the refusal counted, so a malformed `commondir` produces a
/// store that reports why it is empty rather than one that silently is.
fn resolve_commondir(git_dir: &Path, counters: &mut StoreCounters) -> PathBuf {
    let path = git_dir.join("commondir");
    let Some(text) = read_pointer_file(&path) else {
        return git_dir.to_path_buf();
    };
    let target = text.trim();
    if target.is_empty() || has_control_character(target) {
        counters.count(form::COMMONDIR_REFUSED);
        return git_dir.to_path_buf();
    }
    let joined = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        git_dir.join(target)
    };
    match std::fs::canonicalize(&joined) {
        Ok(resolved) if resolved.is_dir() => resolved,
        _ => {
            counters.count(form::COMMONDIR_REFUSED);
            git_dir.to_path_buf()
        }
    }
}

/// Read a small pointer file, bounded, returning `None` when it is absent or over the bound.
fn read_pointer_file(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_POINTER_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    // Lossy rather than refusing: a path that is not UTF-8 will fail the guard below on its own,
    // and a lossy replacement character is not a valid path component either.
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn has_control_character(text: &str) -> bool {
    text.chars().any(|character| (character as u32) < 0x20)
}

/// The three `.git/config` facts this reader needs.
#[derive(Debug, Default, PartialEq, Eq)]
struct GitConfig {
    /// `extensions.objectFormat`, lowercased.
    object_format: Option<String>,
    /// `extensions.partialClone` naming a remote, or `remote.<name>.promisor = true`.
    partial_clone: bool,
}

fn read_config(common_dir: &Path, counters: &mut StoreCounters) -> GitConfig {
    let path = common_dir.join("config");
    let Ok(metadata) = std::fs::metadata(&path) else {
        return GitConfig::default();
    };
    if metadata.len() > MAX_CONFIG_BYTES {
        counters.count(form::CONFIG_TOO_LARGE);
        return GitConfig::default();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return GitConfig::default();
    };
    parse_config(&String::from_utf8_lossy(&bytes))
}

/// Parse the small part of Git's config format this reader needs.
///
/// Deliberately partial. It looks for `extensions.objectFormat`, `extensions.partialClone` and
/// `remote.<name>.promisor`, and understands nothing else — an includes directive, a conditional
/// include, or a value continued across lines is not interpreted. That is a stated limitation
/// rather than an oversight: implementing Git's configuration language would be a second parser
/// whose failure mode is disagreeing with Git about what a repository is.
fn parse_config(text: &str) -> GitConfig {
    let mut config = GitConfig::default();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let body = rest.split(']').next().unwrap_or("");
            // `[remote "origin"]` and `[extensions]`; the subsection is not needed by any key here.
            section = body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches('"').trim();
        match (section.as_str(), key.as_str()) {
            ("extensions", "objectformat") => {
                config.object_format = Some(value.to_ascii_lowercase());
            }
            ("extensions", "partialclone") if !value.is_empty() => config.partial_clone = true,
            ("remote", "promisor")
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "yes" | "on" | "1"
                ) =>
            {
                config.partial_clone = true;
            }
            _ => {}
        }
    }
    config
}

/// A partial clone marks each pack it fetched with a `.promisor` file beside it.
fn has_promisor_marker(objects_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(objects_dir.join("pack")) else {
        return false;
    };
    entries.filter_map(std::result::Result::ok).any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "promisor")
    })
}

/// Read `.git/shallow`, if it exists.
///
/// `None` means *not shallow*. `Some(vec![])` is never produced, because a shallow repository
/// without a boundary is not a state Git creates and inventing it would be a claim.
fn read_shallow(common_dir: &Path, counters: &mut StoreCounters) -> Option<Vec<Oid>> {
    let text = read_pointer_file(&common_dir.join("shallow"))?;
    let mut boundary = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if boundary.len() >= MAX_SHALLOW_ENTRIES {
            counters.count(form::SHALLOW_ENTRIES_EXCEEDED);
            break;
        }
        match Oid::from_hex(line) {
            Some(oid) => boundary.push(oid),
            None => counters.count(form::SHALLOW_ENTRY_UNPARSED),
        }
    }
    Some(boundary)
}

/// Read and guard `objects/info/alternates`, returning the directories that may be followed.
///
/// At most one is returned: the plan's one-hop rule is enforced by taking the first entry that
/// passes every guard, and counting the rest.
fn read_alternates(
    objects_dir: &Path,
    own_objects_dir: &Path,
    repository_root: &Path,
    counters: &mut StoreCounters,
    limits: &mut StoreLimits,
) -> Vec<PathBuf> {
    let Some(text) = read_pointer_file(&objects_dir.join("info").join("alternates")) else {
        return Vec::new();
    };
    let mut followed = Vec::new();
    let mut seen = 0usize;
    for line in text.lines() {
        // Git's own comment syntax. Not a refusal: a comment is not an entry.
        if line.starts_with('#') {
            continue;
        }
        if seen >= MAX_ALTERNATES_ENTRIES {
            counters.count(form::ALTERNATES_ENTRIES_EXCEEDED);
            limits.alternates_refused += 1;
            continue;
        }
        seen += 1;

        if !followed.is_empty() {
            // One hop means one alternate. A second entry is refused rather than silently ignored.
            counters.count(form::ALTERNATE_CHAIN_REFUSED);
            limits.alternates_refused += 1;
            continue;
        }
        match guard_alternate(line, objects_dir, own_objects_dir, repository_root) {
            Ok(path) => followed.push(path),
            Err(tag) => {
                counters.count(tag);
                limits.alternates_refused += 1;
            }
        }
    }
    followed
}

/// Guard one alternates entry, returning the [`form`] tag to count on refusal.
fn guard_alternate(
    line: &str,
    objects_dir: &Path,
    own_objects_dir: &Path,
    repository_root: &Path,
) -> std::result::Result<PathBuf, &'static str> {
    let target = line.trim();
    if target.is_empty() || has_control_character(target) {
        return Err(form::ALTERNATE_REFUSED_SHAPE);
    }
    // Git resolves a relative alternate against the object directory that named it.
    let joined = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        objects_dir.join(target)
    };
    let Ok(resolved) = std::fs::canonicalize(&joined) else {
        return Err(form::ALTERNATE_REFUSED_SHAPE);
    };
    if !resolved.is_dir() {
        return Err(form::ALTERNATE_REFUSED_SHAPE);
    }
    if resolved == own_objects_dir {
        // Naming itself is not a hop; following it would double every lookup for nothing.
        return Err(form::ALTERNATE_REFUSED_SHAPE);
    }
    if !resolved.starts_with(repository_root) {
        return Err(form::ALTERNATE_ESCAPES_REPOSITORY_ROOT);
    }
    Ok(resolved)
}

/// Count an alternate that carries alternates of its own. The second hop is never followed.
fn refuse_second_hop(alternate: &Path, counters: &mut StoreCounters, limits: &mut StoreLimits) {
    let Some(text) = read_pointer_file(&alternate.join("info").join("alternates")) else {
        return;
    };
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        counters.count(form::ALTERNATE_CHAIN_REFUSED);
        limits.alternates_refused += 1;
    }
}

/// Discover and open every usable pack in one object directory.
fn load_source(
    objects_dir: PathBuf,
    budget: &mut usize,
    counters: &mut StoreCounters,
    limits: &mut StoreLimits,
) -> ObjectSource {
    let mut packs = Vec::new();
    let mut candidates: Vec<PathBuf> = match std::fs::read_dir(objects_dir.join("pack")) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "idx"))
            .collect(),
        Err(_) => Vec::new(),
    };
    // Sorted, so which packs a store serves does not depend on directory order.
    candidates.sort();

    for idx_path in candidates {
        if *budget == 0 {
            counters.count(form::PACK_COUNT_EXCEEDED);
            limits.packs_refused += 1;
            continue;
        }
        match load_pack(&idx_path) {
            Ok(loaded) => {
                *budget -= 1;
                limits.packs_loaded += 1;
                packs.push(loaded);
            }
            Err(error) => {
                if let Error::IdxUnsupportedVersion(version) = &error {
                    limits.unsupported_index_versions.push(*version);
                }
                counters.count(error.form());
                limits.packs_refused += 1;
            }
        }
    }

    ObjectSource { objects_dir, packs }
}

fn load_pack(idx_path: &Path) -> Result<LoadedPack> {
    let metadata = std::fs::metadata(idx_path)?;
    if metadata.len() > MAX_IDX_BYTES {
        return Err(Error::IdxTooLarge {
            limit: MAX_IDX_BYTES,
            length: metadata.len(),
        });
    }
    let index = PackIndex::parse(std::fs::read(idx_path)?)?;
    let pack = PackFile::open(&idx_path.with_extension("pack"))?;
    Ok(LoadedPack { index, pack })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::gitobj::testpack::{self, fake_oid};
    use crate::gitobj::ObjectKind;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).expect("writing to a Vec");
        encoder.finish().expect("finishing a Vec encoder")
    }

    /// Write a loose object into `objects_dir` under `oid`, with content `data`.
    ///
    /// The id is invented, which is sound because nothing verifies content against its id.
    fn write_loose(objects_dir: &Path, oid: &Oid, kind: ObjectKind, data: &[u8]) {
        let path = loose_path(objects_dir, oid);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut raw = format!("{} {}\0", kind.as_str(), data.len()).into_bytes();
        raw.extend_from_slice(data);
        std::fs::write(path, deflate(&raw)).unwrap();
    }

    /// A minimal git directory: `objects/`, and nothing else unless a test adds it.
    fn git_dir(root: &Path) -> PathBuf {
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("objects").join("pack")).unwrap();
        git
    }

    fn size_varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return out;
            }
        }
    }

    fn delta_of(base_size: u64, result_size: u64, instructions: &[u8]) -> Vec<u8> {
        let mut out = size_varint(base_size);
        out.extend_from_slice(&size_varint(result_size));
        out.extend_from_slice(instructions);
        out
    }

    /// Write a synthetic pack plus a matching `.idx` into a git directory.
    fn install_pack(git: &Path, name: &str, entries: &[testpack::Entry], oids: &[Oid]) {
        let pack_dir = git.join("objects").join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        let pack_path = pack_dir.join(format!("{name}.pack"));
        let built = testpack::build(&pack_path, entries);
        let table: Vec<(Oid, u64)> = oids.iter().copied().zip(built.offsets.clone()).collect();
        std::fs::write(
            pack_dir.join(format!("{name}.idx")),
            testpack::idx_bytes(&table),
        )
        .unwrap();
    }

    // ---- what it can read ----------------------------------------------------------------------

    #[test]
    fn a_loose_object_is_read_and_an_unknown_id_is_ok_none() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let oid = fake_oid(1);
        write_loose(&git.join("objects"), &oid, ObjectKind::Blob, b"hello");

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(
            store.read(&oid).unwrap(),
            Some(Object::Blob(b"hello".to_vec()))
        );
        assert_eq!(store.read(&fake_oid(2)).unwrap(), None);
        assert!(store.contains(&oid).unwrap());
        assert!(!store.contains(&fake_oid(2)).unwrap());
        assert_eq!(store.counters().total(), 0);
        assert_eq!(store.limits(), &StoreLimits::default());
    }

    #[test]
    fn a_packed_object_is_read_through_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let blob = fake_oid(1);
        let commit = fake_oid(2);
        install_pack(
            &git,
            "pack-test",
            &[
                testpack::Entry::Object(ObjectKind::Blob, b"packed".to_vec()),
                testpack::Entry::Object(ObjectKind::Commit, b"tree x\n".to_vec()),
            ],
            &[blob, commit],
        );

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().packs_loaded, 1);
        assert_eq!(
            store.read(&blob).unwrap(),
            Some(Object::Blob(b"packed".to_vec()))
        );
        assert_eq!(
            store.read(&commit).unwrap(),
            Some(Object::Commit(b"tree x\n".to_vec()))
        );
    }

    #[test]
    fn an_ofs_delta_is_reconstructed_and_keeps_the_base_type() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let base = fake_oid(1);
        let delta = fake_oid(2);
        install_pack(
            &git,
            "pack-test",
            &[
                testpack::Entry::Object(ObjectKind::Tree, b"0123456789".to_vec()),
                testpack::Entry::OfsDelta {
                    base_index: 0,
                    delta: delta_of(10, 4, &[0x91, 0x03, 0x04]),
                },
            ],
            &[base, delta],
        );

        let store = ObjectStore::open(&git).unwrap();
        let object = store.read(&delta).unwrap().expect("the delta resolves");
        assert_eq!(
            object.kind(),
            ObjectKind::Tree,
            "a delta has no type of its own"
        );
        assert_eq!(object.data(), b"3456");
    }

    #[test]
    fn a_ref_delta_resolves_against_a_loose_base_in_the_same_store() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let base = fake_oid(1);
        let delta = fake_oid(2);
        write_loose(&git.join("objects"), &base, ObjectKind::Blob, b"0123456789");
        install_pack(
            &git,
            "pack-test",
            &[testpack::Entry::RefDelta {
                base,
                delta: delta_of(10, 4, &[0x91, 0x03, 0x04]),
            }],
            &[delta],
        );

        let store = ObjectStore::open(&git).unwrap();
        let object = store.read(&delta).unwrap().expect("the delta resolves");
        assert_eq!(object, Object::Blob(b"3456".to_vec()));
    }

    /// Loose wins inside a source, which is the order Git looks in.
    #[test]
    fn a_loose_object_is_preferred_to_a_packed_one() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let oid = fake_oid(1);
        write_loose(&git.join("objects"), &oid, ObjectKind::Blob, b"loose");
        install_pack(
            &git,
            "pack-test",
            &[testpack::Entry::Object(
                ObjectKind::Blob,
                b"packed".to_vec(),
            )],
            &[oid],
        );
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(
            store.read(&oid).unwrap(),
            Some(Object::Blob(b"loose".to_vec()))
        );
    }

    // ---- what it refuses ------------------------------------------------------------------------

    #[test]
    fn a_ref_delta_naming_a_missing_base_is_ok_none_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let delta = fake_oid(2);
        install_pack(
            &git,
            "pack-test",
            &[testpack::Entry::RefDelta {
                base: fake_oid(99),
                delta: delta_of(10, 4, &[0x91, 0x03, 0x04]),
            }],
            &[delta],
        );

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(
            store.read(&delta).unwrap(),
            None,
            "a missing base is Ok(None), never a partially reconstructed object"
        );
        assert_eq!(store.counters().get(form::DELTA_BASE_MISSING), 1);
        // Present but unreadable: `contains` and `read` answer different questions.
        assert!(store.contains(&delta).unwrap());
    }

    #[test]
    fn a_delta_chain_past_the_depth_bound_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());

        // One base plus MAX_DELTA_DEPTH + 1 chained deltas, each copying the whole of its base.
        let mut entries = vec![testpack::Entry::Object(ObjectKind::Blob, b"0123".to_vec())];
        for index in 0..=MAX_DELTA_DEPTH {
            entries.push(testpack::Entry::OfsDelta {
                base_index: index,
                delta: delta_of(4, 4, &[0x91, 0x00, 0x04]),
            });
        }
        let oids: Vec<Oid> = (0..entries.len()).map(|i| fake_oid(i as u8)).collect();
        install_pack(&git, "pack-test", &entries, &oids);

        let store = ObjectStore::open(&git).unwrap();
        // The last delta is MAX_DELTA_DEPTH + 1 links from the base.
        let error = store.read(oids.last().unwrap()).unwrap_err();
        assert_eq!(error.form(), form::DELTA_DEPTH_EXCEEDED);

        // Exactly at the bound still resolves, so the bound refuses one chain rather than all.
        let at_bound = store.read(&oids[oids.len() - 2]).unwrap();
        assert_eq!(at_bound, Some(Object::Blob(b"0123".to_vec())));
    }

    /// A `REF_DELTA` whose base is itself. Caught by the depth bound, which is the only control.
    #[test]
    fn a_delta_cycle_terminates_at_the_depth_bound() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let oid = fake_oid(7);
        install_pack(
            &git,
            "pack-test",
            &[testpack::Entry::RefDelta {
                base: oid,
                delta: delta_of(4, 4, &[0x91, 0x00, 0x04]),
            }],
            &[oid],
        );
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(
            store.read(&oid).unwrap_err().form(),
            form::DELTA_DEPTH_EXCEEDED
        );
    }

    #[test]
    fn an_unreadable_index_costs_its_pack_and_not_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let good = fake_oid(1);
        install_pack(
            &git,
            "pack-good",
            &[testpack::Entry::Object(ObjectKind::Blob, b"good".to_vec())],
            &[good],
        );
        // A real `.idx` v1: no magic, and the v1 layout, so the version can honestly be stated.
        let pack_dir = git.join("objects").join("pack");
        std::fs::write(
            pack_dir.join("pack-legacy.idx"),
            testpack::idx_v1_bytes(&[(fake_oid(2), 12)]),
        )
        .unwrap();
        std::fs::write(pack_dir.join("pack-legacy.pack"), vec![0u8; 100]).unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().packs_loaded, 1);
        assert_eq!(store.limits().packs_refused, 1);
        assert_eq!(store.limits().unsupported_index_versions, vec![1]);
        assert_eq!(store.counters().get(form::IDX_UNSUPPORTED_VERSION), 1);
        assert_eq!(
            store.read(&good).unwrap(),
            Some(Object::Blob(b"good".to_vec()))
        );
    }

    #[test]
    fn packs_past_the_bound_are_refused_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let oid = fake_oid(1);
        install_pack(
            &git,
            "pack-000",
            &[testpack::Entry::Object(ObjectKind::Blob, b"x".to_vec())],
            &[oid],
        );
        let pack_dir = git.join("objects").join("pack");
        let idx = std::fs::read(pack_dir.join("pack-000.idx")).unwrap();
        let pack = std::fs::read(pack_dir.join("pack-000.pack")).unwrap();
        for index in 1..=MAX_PACK_COUNT {
            std::fs::write(pack_dir.join(format!("pack-{index:04}.idx")), &idx).unwrap();
            std::fs::write(pack_dir.join(format!("pack-{index:04}.pack")), &pack).unwrap();
        }

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().packs_loaded, MAX_PACK_COUNT);
        assert_eq!(store.limits().packs_refused, 1);
        assert_eq!(store.counters().get(form::PACK_COUNT_EXCEEDED), 1);
    }

    #[test]
    fn an_oversized_index_is_refused_without_being_read() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let pack_dir = git.join("objects").join("pack");
        let path = pack_dir.join("pack-huge.idx");
        let file = std::fs::File::create(&path).unwrap();
        // Sparse: the length is what the guard reads, and no bytes are ever loaded.
        file.set_len(MAX_IDX_BYTES + 1).unwrap();
        drop(file);

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().packs_loaded, 0);
        assert_eq!(store.limits().packs_refused, 1);
        assert_eq!(store.counters().get(form::IDX_TOO_LARGE), 1);
    }

    // ---- what it says it cannot see -------------------------------------------------------------

    #[test]
    fn a_shallow_file_is_reported_as_a_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let boundary = fake_oid(9);
        std::fs::write(git.join("shallow"), format!("{}\n", boundary.to_hex())).unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().shallow, Some(vec![boundary]));
    }

    /// `None` is *not shallow*; `Some(vec![])` would be a different claim.
    #[test]
    fn no_shallow_file_means_not_shallow_rather_than_no_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().shallow, None);
    }

    #[test]
    fn a_malformed_shallow_line_is_counted_and_the_rest_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let good = fake_oid(9);
        std::fs::write(
            git.join("shallow"),
            format!("not-an-oid\n{}\n\n{}\n", good.to_hex(), "a".repeat(64)),
        )
        .unwrap();
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().shallow, Some(vec![good]));
        assert_eq!(store.counters().get(form::SHALLOW_ENTRY_UNPARSED), 2);
    }

    #[test]
    fn a_promisor_remote_or_a_promisor_pack_marker_is_reported() {
        for setup in ["config", "marker"] {
            let dir = tempfile::tempdir().unwrap();
            let git = git_dir(dir.path());
            if setup == "config" {
                std::fs::write(
                    git.join("config"),
                    "[remote \"origin\"]\n\tpromisor = true\n\tpartialclonefilter = blob:none\n",
                )
                .unwrap();
            } else {
                std::fs::write(git.join("objects/pack/pack-x.promisor"), b"").unwrap();
            }
            let store = ObjectStore::open(&git).unwrap();
            assert!(store.limits().promisor, "{setup} was not detected");
        }
    }

    #[test]
    fn extensions_partial_clone_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        std::fs::write(
            git.join("config"),
            "[extensions]\n\tpartialClone = origin\n",
        )
        .unwrap();
        assert!(ObjectStore::open(&git).unwrap().limits().promisor);
    }

    // ---- object format --------------------------------------------------------------------------

    #[test]
    fn a_sha256_repository_is_refused_with_the_format_named() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        std::fs::write(
            git.join("config"),
            "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectFormat = sha256\n",
        )
        .unwrap();
        match ObjectStore::open(&git).unwrap_err() {
            Error::UnsupportedObjectFormat(format) => assert_eq!(format, "sha256"),
            other => panic!("expected the format to be named, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_sha1_object_format_opens_normally() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        std::fs::write(git.join("config"), "[extensions]\n\tobjectFormat = sha1\n").unwrap();
        assert!(ObjectStore::open(&git).is_ok());
    }

    #[test]
    fn an_oversized_config_is_not_parsed_and_is_counted() {
        let dir = tempfile::tempdir().unwrap();
        let git = git_dir(dir.path());
        let mut config = b"[extensions]\n\tobjectFormat = sha256\n".to_vec();
        config.resize(MAX_CONFIG_BYTES as usize + 1, b'\n');
        std::fs::write(git.join("config"), config).unwrap();
        // Refusing to *parse* it must not mean silently accepting a sha256 repository as sha1: the
        // store opens, and the refusal is on the record.
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.counters().get(form::CONFIG_TOO_LARGE), 1);
    }

    // ---- worktrees ------------------------------------------------------------------------------

    /// A linked worktree's private directory has no `objects/`. Following `commondir` is what makes
    /// the store find anything at all.
    #[test]
    fn a_linked_worktree_reads_through_commondir() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let git = git_dir(&main);
        let oid = fake_oid(3);
        write_loose(&git.join("objects"), &oid, ObjectKind::Blob, b"shared");

        let worktree_git = git.join("worktrees").join("feature");
        std::fs::create_dir_all(&worktree_git).unwrap();
        std::fs::write(worktree_git.join("commondir"), "../..\n").unwrap();
        std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/feature\n").unwrap();

        // The `.git` file a worktree checkout carries, resolved by gitinfo rather than here.
        let checkout = dir.path().join("feature");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();
        let resolved = crate::gitinfo::git_dir(&checkout).expect("gitinfo resolves the .git file");
        assert_eq!(
            std::fs::canonicalize(&resolved).unwrap(),
            std::fs::canonicalize(&worktree_git).unwrap()
        );

        let store = ObjectStore::open(&resolved).unwrap();
        assert_eq!(
            store.common_dir(),
            std::fs::canonicalize(&git).unwrap(),
            "commondir must reach the main repository's git directory"
        );
        assert_eq!(
            store.read(&oid).unwrap(),
            Some(Object::Blob(b"shared".to_vec())),
            "a worktree that cannot see the shared objects would read as an empty history"
        );
    }

    #[test]
    fn a_malformed_commondir_is_counted_and_falls_back() {
        for contents in ["", "   ", "no/such/directory", "a\u{1}b"] {
            let dir = tempfile::tempdir().unwrap();
            let git = git_dir(dir.path());
            std::fs::write(git.join("commondir"), contents).unwrap();
            let store = ObjectStore::open(&git).unwrap();
            assert_eq!(
                store.counters().get(form::COMMONDIR_REFUSED),
                1,
                "{contents:?} was not counted"
            );
            assert_eq!(store.common_dir(), std::fs::canonicalize(&git).unwrap());
        }
    }

    #[test]
    fn a_path_that_is_not_a_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            ObjectStore::open(&file).unwrap_err().form(),
            form::NOT_A_DIRECTORY
        );
        assert_eq!(
            ObjectStore::open(&dir.path().join("absent"))
                .unwrap_err()
                .form(),
            form::NOT_A_DIRECTORY
        );
    }

    // ---- alternates -----------------------------------------------------------------------------

    /// One hop, followed, with an object only the alternate holds.
    #[test]
    fn one_alternate_inside_the_repository_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        let shared = root.join("shared-objects");
        std::fs::create_dir_all(&shared).unwrap();
        let oid = fake_oid(4);
        write_loose(&shared, &oid, ObjectKind::Blob, b"only in the alternate");

        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        std::fs::write(
            git.join("objects").join("info").join("alternates"),
            format!("{}\n", shared.display()),
        )
        .unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().alternates_followed, 1);
        assert_eq!(store.limits().alternates_refused, 0);
        assert_eq!(
            store.read(&oid).unwrap(),
            Some(Object::Blob(b"only in the alternate".to_vec()))
        );
    }

    #[test]
    fn a_relative_alternate_resolves_against_the_object_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        let shared = git.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        let oid = fake_oid(4);
        write_loose(&shared, &oid, ObjectKind::Blob, b"relative");

        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        std::fs::write(
            git.join("objects").join("info").join("alternates"),
            "../shared\n",
        )
        .unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().alternates_followed, 1);
        assert_eq!(
            store.read(&oid).unwrap(),
            Some(Object::Blob(b"relative".to_vec()))
        );
    }

    #[test]
    fn a_second_alternate_entry_is_refused_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let only_in_second = fake_oid(5);
        write_loose(&second, &only_in_second, ObjectKind::Blob, b"unreachable");

        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        std::fs::write(
            git.join("objects").join("info").join("alternates"),
            format!("{}\n{}\n", first.display(), second.display()),
        )
        .unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().alternates_followed, 1);
        assert_eq!(store.limits().alternates_refused, 1);
        assert_eq!(store.counters().get(form::ALTERNATE_CHAIN_REFUSED), 1);
        assert_eq!(store.read(&only_in_second).unwrap(), None);
    }

    /// The chain case: the alternate has alternates of its own. The second hop is never followed.
    #[test]
    fn an_alternates_chain_is_refused_at_the_second_hop_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        let hop_one = root.join("hop-one");
        let hop_two = root.join("hop-two");
        std::fs::create_dir_all(hop_one.join("info")).unwrap();
        std::fs::create_dir_all(&hop_two).unwrap();
        let far = fake_oid(6);
        write_loose(&hop_two, &far, ObjectKind::Blob, b"two hops away");

        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        std::fs::write(
            git.join("objects").join("info").join("alternates"),
            format!("{}\n", hop_one.display()),
        )
        .unwrap();
        std::fs::write(
            hop_one.join("info").join("alternates"),
            format!("{}\n", hop_two.display()),
        )
        .unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().alternates_followed, 1);
        assert_eq!(store.limits().alternates_refused, 1);
        assert_eq!(store.counters().get(form::ALTERNATE_CHAIN_REFUSED), 1);
        assert_eq!(
            store.read(&far).unwrap(),
            None,
            "an object two hops away must not be readable"
        );
    }

    #[test]
    fn an_alternate_outside_the_repository_root_is_refused_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        let elsewhere = dir.path().join("other-repository");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let oid = fake_oid(7);
        write_loose(&elsewhere, &oid, ObjectKind::Blob, b"another repository");

        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        std::fs::write(
            git.join("objects").join("info").join("alternates"),
            format!("{}\n", elsewhere.display()),
        )
        .unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().alternates_followed, 0);
        assert_eq!(store.limits().alternates_refused, 1);
        assert_eq!(
            store
                .counters()
                .get(form::ALTERNATE_ESCAPES_REPOSITORY_ROOT),
            1
        );
        assert_eq!(store.read(&oid).unwrap(), None);
    }

    #[test]
    fn a_hostile_alternates_entry_is_refused_by_shape_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        let own_objects = git.join("objects");

        let lines = format!(
            "\n   \n/no/such/directory/anywhere\n{}\n{}\na\u{1}b\n",
            own_objects.display(),
            root.join("README.md").display()
        );
        std::fs::write(git.join("objects").join("info").join("alternates"), lines).unwrap();
        std::fs::write(root.join("README.md"), b"not a directory").unwrap();

        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.limits().alternates_followed, 0);
        assert_eq!(store.counters().get(form::ALTERNATE_REFUSED_SHAPE), 6);
        assert_eq!(store.limits().alternates_refused, 6);
    }

    #[test]
    fn a_comment_in_the_alternates_file_is_not_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        std::fs::write(
            git.join("objects").join("info").join("alternates"),
            "# a comment Git itself ignores\n",
        )
        .unwrap();
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.counters().total(), 0);
        assert_eq!(store.limits().alternates_refused, 0);
    }

    #[test]
    fn alternates_entries_past_the_bound_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        std::fs::create_dir_all(git.join("objects").join("info")).unwrap();
        let mut lines = String::new();
        for index in 0..MAX_ALTERNATES_ENTRIES + 3 {
            lines.push_str(&format!("{}\n", root.join(format!("d{index}")).display()));
        }
        std::fs::write(git.join("objects").join("info").join("alternates"), lines).unwrap();
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(store.counters().get(form::ALTERNATES_ENTRIES_EXCEEDED), 3);
    }

    #[test]
    fn the_repository_root_is_the_parent_of_the_common_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let git = git_dir(&root);
        let store = ObjectStore::open(&git).unwrap();
        assert_eq!(
            store.repository_root(),
            std::fs::canonicalize(&root).unwrap()
        );
        assert_eq!(store.git_dir(), std::fs::canonicalize(&git).unwrap());
    }

    // ---- the config parser ----------------------------------------------------------------------

    #[test]
    fn the_config_parser_reads_only_what_it_claims_to() {
        let config = parse_config(
            "# a comment\n; another\n[core]\n\trepositoryformatversion = 1\n\
             [extensions]\n\tobjectFormat = SHA256\n\tpartialClone = origin\n\
             [remote \"origin\"]\n\tpromisor = true\n\turl = https://example.invalid/x\n",
        );
        assert_eq!(config.object_format.as_deref(), Some("sha256"));
        assert!(config.partial_clone);

        let empty = parse_config("");
        assert_eq!(empty, GitConfig::default());

        // A promisor value that is not truthy is not a partial clone.
        let off = parse_config("[remote \"origin\"]\n\tpromisor = false\n");
        assert!(!off.partial_clone);
    }
}
