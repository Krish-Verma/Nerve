//! The second-repository trust boundary: registering, re-validating and reading a neighbour
//! (Slice 13a-ii).
//!
//! Every row before this one read the repository the user named on the command line. This module
//! reads a directory the user named **once**, in the past, and whose contents have been free to
//! change ever since. That is a new trust boundary rather than a wider version of an old one, and
//! `docs/THREAT-MODEL.md` T12 states it — T2 is about paths *within* one repository and has nothing
//! to say about a second one.
//!
//! Six properties are what this module is.
//!
//! 1. **Registration is explicit and nothing is discovered.** There is no scan of sibling
//!    directories anywhere in this crate. A neighbour exists because a mutating command named it,
//!    which is the "directory proximity" link `docs/plans/slice-13-cross-repository-contracts.md`
//!    §1 refuses, one layer down.
//! 2. **The target opens read-only.** [`nerve_store::open_read_only`] is not [`nerve_store::open`]
//!    with a flag — the ordinary open creates, chmods and sets `journal_mode=WAL`, and that last one
//!    writes the database header. The neighbour's bytes are identical after every read this module
//!    performs, which is a test rather than a claim.
//! 3. **The path is re-validated at every use.** `repo_registry.local_path` is a row in a file on
//!    disk; the moment it is written it is untrusted input, because what it points at can be
//!    replaced. [`availability_of`] therefore re-runs the whole guard chain rather than trusting the
//!    row, and identity is compared against the **recorded repository id** and never against the
//!    path — a path comparison is precisely the silent re-pointing that would make every link
//!    through the entry describe the wrong repository.
//! 4. **The database, and nothing else.** This module opens exactly one file in the neighbour:
//!    `.nerve/nerve.db`. It does not index the target, does not walk its tree, writes no row into
//!    it and modifies no file it finds there. Manifest reading arrives with contract detection in
//!    13b/13c and will be a second named read, not a widening of this one. **One residual, measured
//!    rather than assumed** — see *SQLite's sidecars*, below.
//! 5. **Everything read out is untrusted repository content**, on exactly T7's terms.
//!    [`RegistryTarget::display_name`] is a directory name from a checkout that may have been cloned
//!    from anywhere; it is stored verbatim, interpreted never, and rendered inert by the surface.
//! 6. **One service, and every surface reads it.** Freshness and availability are derived here and
//!    nowhere else. `crates/nerve-cli/tests/registry_guards.rs` scans the surfaces for a second
//!    derivation, in the shape `crates/nerve-cli/tests/history_wording.rs` already established for
//!    history wording.
//!
//! # SQLite's sidecars, and why they are accepted rather than avoided
//!
//! Measured, not assumed: a Nerve index runs in WAL mode, and **a read-only connection to a
//! WAL-mode database makes SQLite create `nerve.db-shm` and a zero-length `nerve.db-wal` beside it**
//! if they are absent. A read-only connection also cannot delete them on close, so they persist.
//! The database file itself is byte-identical — `crates/nerve-index/tests/registry.rs` hashes it —
//! and no file that was already in the neighbour is modified or removed, but two coordination files
//! do appear inside its `.nerve/`, which `nerve init` has already covered with a `*` gitignore.
//!
//! `file:…?immutable=1` avoids them entirely and was rejected on two counts. It needs
//! `SQLITE_OPEN_URI` plus a hand-written percent-encoder for a path that came out of the database —
//! a parser of our own in the exact expression that decides whether the connection is read-only, and
//! T11 already records this project refusing that trade. And `immutable=1` tells SQLite the file
//! cannot change, so a neighbour that is being indexed right now, or whose WAL was never
//! checkpointed, would be read **without its WAL**: a stale answer presented as a current one, which
//! is the one failure this whole row exists to prevent. Two empty coordination files in an ignored
//! directory is the smaller cost, and it is stated in `docs/THREAT-MODEL.md` T12 rather than left to
//! be discovered.
//!
//! # What this module can and cannot say about freshness
//!
//! [`RegistryAvailability::freshness`] maps an availability verdict onto
//! [`ContractFreshness`] where one exists. Four of the twelve are reachable here —
//! `target_repository_missing`, `target_repository_moved`, `target_partially_indexed` and
//! `registry_entry_removed` — because those four are properties of a *registry entry*. The other
//! eight are properties of a *contract link*, and there are no contract links until 13b/13c
//! extracts one. A state that cannot be produced from a fixture is not claimed as covered.
//!
//! **A refusal maps to no freshness value at all**, and that is deliberate. `target_repository_missing`
//! means *nothing is there*; a path Nerve declined to follow is a different fact, and rendering the
//! second as the first is the T2 honesty failure Slice 8b-i had to amend one boundary over — a
//! refusal disguised as a miss. So the availability verdict is what a surface renders, and
//! `freshness` is `None` for a refusal rather than the nearest-looking neighbour.

use std::path::{Path, PathBuf};

use nerve_core::vocab::{ContractFreshness, RegistryEntryStatus};
use nerve_store::{Connection, RegistryEntryRow};

use crate::config;
use crate::discover::{canonical_child, canonical_root};
use crate::error::{IndexError, Result};

/// The neighbour's database, relative to its root. The only file this module opens over there.
const TARGET_DB_RELATIVE: &str = ".nerve/nerve.db";

/// Why a registry command refused, as a closed vocabulary.
///
/// A refusal is a finding with a name, never an absence and never a silent fallback. The set is
/// closed so a surface can count refusals by form rather than collapse them into "it did not work",
/// which is the discipline [`crate::discover::TreeNameRefusal`] and `nerve_store::HistoryPathRefusal`
/// already apply to their own boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegistryRefusal {
    /// Nothing exists at the path given.
    PathDoesNotExist,
    /// Something exists at the path and it is not a directory.
    PathIsNotADirectory,
    /// The directory exists and holds no `.nerve/nerve.db`.
    NoNerveDatabase,
    /// The path, or the database inside it, is a symlink leading out of the target root.
    SymlinkEscape,
    /// The target's database records a schema version this build does not support.
    TargetSchemaTooNew,
    /// The database is there and could not be read as a Nerve index.
    TargetUnreadable,
    /// The target is the repository the command was run from.
    SameRepository,
    /// This repository's registry already holds an entry under that name.
    AlreadyRegistered,
    /// The repository found at the path is not the one the entry records.
    TargetRepositoryMoved,
    /// No entry with that id has ever been registered here.
    NoSuchRegistryEntry,
    /// The entry exists and was tombstoned.
    RegistryEntryTombstoned,
    /// No usable local identifier was given, and none could be derived from the directory name.
    UnusableRegistryId,
}

impl RegistryRefusal {
    /// Every value, in declaration order.
    pub const ALL: [RegistryRefusal; 12] = [
        RegistryRefusal::PathDoesNotExist,
        RegistryRefusal::PathIsNotADirectory,
        RegistryRefusal::NoNerveDatabase,
        RegistryRefusal::SymlinkEscape,
        RegistryRefusal::TargetSchemaTooNew,
        RegistryRefusal::TargetUnreadable,
        RegistryRefusal::SameRepository,
        RegistryRefusal::AlreadyRegistered,
        RegistryRefusal::TargetRepositoryMoved,
        RegistryRefusal::NoSuchRegistryEntry,
        RegistryRefusal::RegistryEntryTombstoned,
        RegistryRefusal::UnusableRegistryId,
    ];

    /// Canonical lower-case name, carried on every response that reports a refusal.
    pub fn as_str(self) -> &'static str {
        match self {
            RegistryRefusal::PathDoesNotExist => "path_does_not_exist",
            RegistryRefusal::PathIsNotADirectory => "path_is_not_a_directory",
            RegistryRefusal::NoNerveDatabase => "no_nerve_database",
            RegistryRefusal::SymlinkEscape => "symlink_escape",
            RegistryRefusal::TargetSchemaTooNew => "target_schema_too_new",
            RegistryRefusal::TargetUnreadable => "target_unreadable",
            RegistryRefusal::SameRepository => "same_repository",
            RegistryRefusal::AlreadyRegistered => "already_registered",
            RegistryRefusal::TargetRepositoryMoved => "target_repository_moved",
            RegistryRefusal::NoSuchRegistryEntry => "no_such_registry_entry",
            RegistryRefusal::RegistryEntryTombstoned => "registry_entry_tombstoned",
            RegistryRefusal::UnusableRegistryId => "unusable_registry_id",
        }
    }

    /// What was refused, and why, in words.
    pub fn statement(self) -> &'static str {
        match self {
            RegistryRefusal::PathDoesNotExist => {
                "nothing exists at that path, so there is no repository to register — this is a \
                 refusal rather than an empty registration"
            }
            RegistryRefusal::PathIsNotADirectory => {
                "something exists at that path and it is not a directory; a repository is a \
                 directory, and guessing at a nearby one would be inventing the argument"
            }
            RegistryRefusal::NoNerveDatabase => {
                "that directory holds no .nerve/nerve.db, so it has never been initialised as a \
                 Nerve index and there is no repository identity to record — run `nerve init` \
                 there first"
            }
            RegistryRefusal::SymlinkEscape => {
                "the registered path, or the database inside it, is a symlink leading outside the \
                 target root. It is refused rather than followed, by the same guard that refuses a \
                 symlink escape inside one repository"
            }
            RegistryRefusal::TargetSchemaTooNew => {
                "that repository's index records a schema version this build does not support. It \
                 is refused rather than migrated, because migrating a database Nerve does not own \
                 would be a write into a repository the user named only as a neighbour"
            }
            RegistryRefusal::TargetUnreadable => {
                "the database is there and could not be read as a Nerve index. Nothing was \
                 written, and nothing is assumed about what it holds"
            }
            RegistryRefusal::SameRepository => {
                "that is the repository this command was run from. A registry records neighbours, \
                 and an entry pointing at itself would make every link through it circular"
            }
            RegistryRefusal::AlreadyRegistered => {
                "this repository's registry already holds an entry under that name. It may be a \
                 tombstone: removal retires an entry and never deletes it, and there is no purge \
                 verb, so a retired name stays taken"
            }
            RegistryRefusal::TargetRepositoryMoved => {
                "the repository at that path is not the one this entry records. Identity is \
                 checked against the recorded repository id rather than against the path, because \
                 accepting it would be exactly the silent re-pointing that makes every link \
                 through the entry describe the wrong repository"
            }
            RegistryRefusal::NoSuchRegistryEntry => {
                "no entry with that id has ever been registered here. A retired entry is still an \
                 entry and would have been found; this name has never been used"
            }
            RegistryRefusal::RegistryEntryTombstoned => {
                "that entry was removed. The row is kept so its ending can be reported, and a \
                 retired entry is not relocated or removed again — the date something ended is not \
                 re-datable by asking twice"
            }
            RegistryRefusal::UnusableRegistryId => {
                "no usable local identifier was given and none could be derived from the directory \
                 name. An id names an entry in this repository's own registry, so it is asked for \
                 rather than invented"
            }
        }
    }
}

impl std::fmt::Display for RegistryRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a read-only open of a neighbour's index established about it.
///
/// Every string here is **untrusted repository content** on T7's terms. None of it is interpreted,
/// and a surface confines it to the envelope T7 defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryTarget {
    /// The canonical root of the target checkout.
    pub root: PathBuf,
    /// The target's own `repo_id`. This is the identity every later check is made against.
    pub repository_id: String,
    /// The target's `project_id`.
    pub project_id: String,
    /// The schema version its database records.
    pub schema_version: i64,
    /// The state its last index ran at, or `None` if it has never been indexed.
    pub state_id: Option<String>,
    /// How many entities it has indexed.
    pub entities_total: i64,
    /// The target directory's own name, offered as a default display name.
    pub default_display_name: String,
}

impl RegistryTarget {
    /// Whether the target holds an index, as opposed to merely a database.
    ///
    /// A neighbour that was initialised and never indexed is *unknown*, not *unchanged*: nothing
    /// was observed to change there because nothing was ever looked at. That is Slice 7c-i's
    /// `Stale` / `Unverified` distinction, and it is why this is the evidence for
    /// [`ContractFreshness::TargetPartiallyIndexed`] rather than for a clean bill of health.
    pub fn is_indexed(&self) -> bool {
        self.state_id.is_some() && self.entities_total > 0
    }
}

/// What a registry entry is right now, on evidence, re-derived from the filesystem every time.
///
/// This is the one place availability is decided. A surface renders [`RegistryAvailability::as_str`]
/// and, where there is one, the [`ContractFreshness`] it implies; it does not compute either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryAvailability {
    /// Opened read-only, the recorded repository id was found, and the target holds an index.
    Available,
    /// Opened read-only and identity-confirmed, and the target holds no indexed state.
    PartiallyIndexed,
    /// The entry was tombstoned. **Nothing was opened**: reading a directory on behalf of an entry
    /// the user retired is a read the user did not ask for.
    EntryRemoved,
    /// Nothing readable is at the registered path, with the named absence.
    Missing(RegistryRefusal),
    /// Something is at the registered path and it is a **different** repository.
    Moved {
        /// The repository id actually found there.
        observed_repository_id: String,
    },
    /// The path was refused before the target was read, with the named refusal.
    Refused(RegistryRefusal),
}

impl RegistryAvailability {
    /// Canonical lower-case name, carried on every response that reports an entry.
    pub fn as_str(&self) -> &'static str {
        match self {
            RegistryAvailability::Available => "available",
            RegistryAvailability::PartiallyIndexed => "partially_indexed",
            RegistryAvailability::EntryRemoved => "entry_removed",
            RegistryAvailability::Missing(_) => "missing",
            RegistryAvailability::Moved { .. } => "moved",
            RegistryAvailability::Refused(_) => "refused",
        }
    }

    /// What the verdict means, in words. One sentence per verdict, held in this file only.
    ///
    /// [`RegistryAvailability::Available`] needs a sentence as much as the failures do: *"nothing is
    /// wrong"* is a claim about a read that happened, and a surface printing a bare word cannot say
    /// which read it was.
    pub fn statement(&self) -> &'static str {
        match self {
            RegistryAvailability::Available => {
                "the registered path was re-checked, the repository found there is the one this \
                 entry records, and it holds an index"
            }
            RegistryAvailability::PartiallyIndexed => {
                "the repository found there is the one this entry records, and none of it has been \
                 indexed. Nothing was observed to change; nothing was observed at all"
            }
            RegistryAvailability::EntryRemoved => {
                "this entry was retired. It is still listed, because an entry that vanished could \
                 not be reported as having ended, and nothing at its path was opened"
            }
            RegistryAvailability::Missing(_) => {
                "there is nothing readable at the registered path any more, and the named reason \
                 says which absence it is"
            }
            RegistryAvailability::Moved { .. } => {
                "a repository is at the registered path and it is not the one this entry records. \
                 Nothing resolved through this entry may be reported until it is re-pointed or \
                 retired"
            }
            RegistryAvailability::Refused(_) => {
                "the path was refused before the target was read. This is a refusal and not an \
                 absence: what is there was not examined, so nothing is claimed about it"
            }
        }
    }

    /// The qualification this availability puts on anything resolved through the entry.
    ///
    /// `None` has two meanings and they are both honest. For [`RegistryAvailability::Available`] it
    /// means *no qualification* — the target is there, it is the recorded one, and it has been
    /// indexed. For [`RegistryAvailability::Refused`] it means *this vocabulary has no value for
    /// this fact*: a path Nerve declined to follow is not a repository that has gone missing, and
    /// reporting the second for the first would be a refusal disguised as a miss. The verdict is
    /// what distinguishes them, which is why a surface renders the verdict and not only this.
    pub fn freshness(&self) -> Option<ContractFreshness> {
        match self {
            RegistryAvailability::Available => None,
            RegistryAvailability::PartiallyIndexed => {
                Some(ContractFreshness::TargetPartiallyIndexed)
            }
            RegistryAvailability::EntryRemoved => Some(ContractFreshness::RegistryEntryRemoved),
            RegistryAvailability::Missing(_) => Some(ContractFreshness::TargetRepositoryMissing),
            RegistryAvailability::Moved { .. } => Some(ContractFreshness::TargetRepositoryMoved),
            RegistryAvailability::Refused(_) => None,
        }
    }

    /// The named refusal behind this verdict, where there is one.
    pub fn refusal(&self) -> Option<RegistryRefusal> {
        match self {
            RegistryAvailability::Missing(reason) | RegistryAvailability::Refused(reason) => {
                Some(*reason)
            }
            RegistryAvailability::Moved { .. } => Some(RegistryRefusal::TargetRepositoryMoved),
            _ => None,
        }
    }

    /// The repository id actually found at the path, when one was read and it was the wrong one.
    pub fn observed_repository_id(&self) -> Option<&str> {
        match self {
            RegistryAvailability::Moved {
                observed_repository_id,
            } => Some(observed_repository_id.as_str()),
            _ => None,
        }
    }

    /// Whether anything resolved through this entry may be reported without a qualification.
    pub fn is_usable(&self) -> bool {
        matches!(self, RegistryAvailability::Available)
    }
}

/// One registry entry and what it is right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntryView {
    /// The stored row, tombstone included.
    pub entry: RegistryEntryRow,
    /// The verdict, re-derived from the filesystem rather than read out of the row.
    pub availability: RegistryAvailability,
}

/// The outcome of a mutating registry command.
///
/// A refusal is a value rather than an error, in the manner of [`crate::coverage_ingest`]'s
/// outcome: a real storage failure is still an `Err`, and a refusal is something Nerve decided and
/// can name.
/// The variants differ in size by a row's worth of `String`s, and `clippy::large_enum_variant`
/// suggests boxing the larger one. It is not boxed: this value is constructed **once per invocation
/// of a command a human typed**, never in a loop, so the allocation buys nothing measurable and
/// costs every caller a `Box::new` and a deref at the one place the row is handed back.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryOutcome {
    /// The command did what was asked, and this is the row as it now stands.
    Done(RegistryEntryRow),
    /// The command refused, with the reason named.
    Refused(RegistryRefusal),
}

/// Validate a path and read the neighbour's identity, read-only.
///
/// The guard chain, in the order the checks are cheapest and most conclusive:
///
/// 1. **The final component must not be a symlink.** `symlink_metadata` decides this without
///    following it, which is the rule [`crate::probe`] already applies to a query-time read. Only
///    the final component: intermediate ones are the user's own filesystem layout — on macOS
///    `/var` is a symlink to `/private/var`, so refusing an intermediate link would refuse every
///    temporary directory on the platform, and it would refuse it for a reason that has nothing to
///    do with the target.
/// 2. **The path must be an existing directory**, canonicalized by [`canonical_root`].
/// 3. **`.nerve/nerve.db` must resolve inside that root**, by [`canonical_child`] — the same single
///    choke point discovery and query-time reads use, so a `.nerve` symlinked at somebody else's
///    database is refused rather than followed. This is the check that makes control 6 real.
/// 4. **The database opens read-only** and its schema version is read *before* anything else. A
///    version this build does not support is refused rather than migrated.
/// 5. **The repository row is read.** Its `repo_id` is the identity every later check is made
///    against.
///
/// Nothing is written at any step, and no file other than the database is opened.
pub fn probe_target(path: &Path) -> std::result::Result<RegistryTarget, RegistryRefusal> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(RegistryRefusal::SymlinkEscape)
        }
        Ok(metadata) if !metadata.is_dir() => return Err(RegistryRefusal::PathIsNotADirectory),
        Ok(_) => {}
        Err(_) => return Err(RegistryRefusal::PathDoesNotExist),
    }

    let root = canonical_root(path).map_err(|err| match err {
        IndexError::NotADirectory(_) => RegistryRefusal::PathIsNotADirectory,
        _ => RegistryRefusal::PathDoesNotExist,
    })?;

    let db_path = match canonical_child(&root, Path::new(TARGET_DB_RELATIVE)) {
        Ok(resolved) => resolved,
        Err(IndexError::PathEscapesRoot(_)) => {
            // Two different facts wear the same error, so they are separated here rather than
            // reported as one: `canonical_child` cannot canonicalize a path that does not exist, so
            // an absent database and a database symlinked out of the root both arrive as an escape.
            // Reporting a missing index as a symlink escape would be a security control claiming a
            // hit it did not get.
            return Err(match root.join(TARGET_DB_RELATIVE).symlink_metadata() {
                Ok(_) => RegistryRefusal::SymlinkEscape,
                Err(_) => RegistryRefusal::NoNerveDatabase,
            });
        }
        Err(_) => return Err(RegistryRefusal::NoNerveDatabase),
    };

    let conn =
        nerve_store::open_read_only(&db_path).map_err(|_| RegistryRefusal::TargetUnreadable)?;

    // The version is read before anything else, and a version this build cannot support stops the
    // read here. Every query below names tables and columns whose shape is only guaranteed at or
    // below `SCHEMA_VERSION`, so asking them of a newer database would be guessing at its layout.
    let schema_version = match nerve_store::schema_version(&conn) {
        Ok(Some(version)) if version > nerve_store::SCHEMA_VERSION => {
            return Err(RegistryRefusal::TargetSchemaTooNew)
        }
        Ok(Some(version)) => version,
        Ok(None) | Err(_) => return Err(RegistryRefusal::TargetUnreadable),
    };

    let repository = nerve_store::repository(&conn)
        .map_err(|_| RegistryRefusal::TargetUnreadable)?
        .ok_or(RegistryRefusal::TargetUnreadable)?;
    let status = nerve_store::status(&conn).map_err(|_| RegistryRefusal::TargetUnreadable)?;

    let default_display_name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| repository.repo_id.clone());

    Ok(RegistryTarget {
        root,
        repository_id: repository.repo_id,
        project_id: repository.project_id,
        schema_version,
        state_id: status.state_id,
        entities_total: status.entities_total,
        default_display_name,
    })
}

/// Derive what a stored registry entry is right now.
///
/// **The row is not trusted.** Everything but the recorded identity is re-established from the
/// filesystem: the path is re-validated through the same guard chain [`probe_target`] applies at
/// registration, the database is re-opened read-only, and the repository id found there is compared
/// against [`RegistryEntryRow::expected_repository_id`]. Comparing paths instead would make a
/// checkout swapped underneath the entry invisible, which is the one failure §6 of the row plan
/// calls the dangerous case.
pub fn availability_of(entry: &RegistryEntryRow) -> RegistryAvailability {
    if entry.status == RegistryEntryStatus::Tombstoned {
        return RegistryAvailability::EntryRemoved;
    }
    match probe_target(Path::new(&entry.local_path)) {
        Ok(target) if target.repository_id != entry.expected_repository_id => {
            RegistryAvailability::Moved {
                observed_repository_id: target.repository_id,
            }
        }
        Ok(target) if !target.is_indexed() => RegistryAvailability::PartiallyIndexed,
        Ok(_) => RegistryAvailability::Available,
        Err(reason @ RegistryRefusal::PathDoesNotExist)
        | Err(reason @ RegistryRefusal::PathIsNotADirectory)
        | Err(reason @ RegistryRefusal::NoNerveDatabase) => RegistryAvailability::Missing(reason),
        Err(reason) => RegistryAvailability::Refused(reason),
    }
}

/// Every registry entry for this repository and what each one is right now, tombstones included.
///
/// A tombstoned entry is listed. That is not a convenience: `registry_entry_removed` is a report
/// made from the kept row, and a list that hid it would make the state unreportable at exactly the
/// moment it becomes the answer.
pub fn list_registry(conn: &Connection, repo_id: &str) -> Result<Vec<RegistryEntryView>> {
    let mut out = Vec::new();
    for entry in nerve_store::list_registry_entries(conn, repo_id)? {
        let availability = availability_of(&entry);
        out.push(RegistryEntryView {
            entry,
            availability,
        });
    }
    Ok(out)
}

/// Register a neighbouring repository.
///
/// `registry_id` and `display_name` default to the target directory's own name. The default name is
/// untrusted repository content and is stored verbatim; the default id is the same name reduced to
/// a local identifier, and if nothing usable survives that reduction the command refuses rather
/// than inventing one.
pub fn add_registry_target(
    conn: &Connection,
    repo_id: &str,
    path: &Path,
    registry_id: Option<&str>,
    display_name: Option<&str>,
) -> Result<RegistryOutcome> {
    let target = match probe_target(path) {
        Ok(target) => target,
        Err(reason) => return Ok(RegistryOutcome::Refused(reason)),
    };

    if target.repository_id == repo_id {
        return Ok(RegistryOutcome::Refused(RegistryRefusal::SameRepository));
    }

    let id = match registry_id {
        Some(given) => match usable_registry_id(given) {
            Some(id) => id,
            None => {
                return Ok(RegistryOutcome::Refused(
                    RegistryRefusal::UnusableRegistryId,
                ))
            }
        },
        None => match usable_registry_id(&target.default_display_name) {
            Some(id) => id,
            None => {
                return Ok(RegistryOutcome::Refused(
                    RegistryRefusal::UnusableRegistryId,
                ))
            }
        },
    };

    // Both halves of "already registered" are checked, because they are different mistakes with the
    // same remedy: the same *name* used twice, and the same *repository* registered twice under two
    // names. Neither is a primary-key error the user should have to read a SQL message to
    // understand, and a tombstone counts for both — removal retires an entry and there is no purge.
    let existing = nerve_store::list_registry_entries(conn, repo_id)?;
    if existing
        .iter()
        .any(|row| row.registry_id == id || row.expected_repository_id == target.repository_id)
    {
        return Ok(RegistryOutcome::Refused(RegistryRefusal::AlreadyRegistered));
    }

    let name = display_name.unwrap_or(&target.default_display_name);
    let written = nerve_store::insert_registry_entry(
        conn,
        repo_id,
        &id,
        &target.repository_id,
        name,
        &target.root.to_string_lossy(),
    )?;
    nerve_store::record_registry_observation(conn, repo_id, &id, target.state_id.as_deref())?;
    Ok(RegistryOutcome::Done(
        nerve_store::registry_entry(conn, repo_id, &id)?.unwrap_or(written),
    ))
}

/// Point an existing entry at a different path, **after** proving the recorded repository is there.
///
/// This is the function `nerve_store::relocate_registry_entry`'s doc comment refuses to be called
/// without. Without the identity check, relocation is not a convenience — it is the silent
/// re-pointing `target_repository_moved` exists to detect, performed by Nerve itself on request,
/// and every link resolved through the entry afterwards would describe a repository nobody
/// registered.
pub fn relocate_registry_target(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
    new_path: &Path,
) -> Result<RegistryOutcome> {
    let entry = match nerve_store::registry_entry(conn, repo_id, registry_id)? {
        Some(entry) => entry,
        None => {
            return Ok(RegistryOutcome::Refused(
                RegistryRefusal::NoSuchRegistryEntry,
            ))
        }
    };
    if entry.status == RegistryEntryStatus::Tombstoned {
        return Ok(RegistryOutcome::Refused(
            RegistryRefusal::RegistryEntryTombstoned,
        ));
    }

    let target = match probe_target(new_path) {
        Ok(target) => target,
        Err(reason) => return Ok(RegistryOutcome::Refused(reason)),
    };
    if target.repository_id != entry.expected_repository_id {
        return Ok(RegistryOutcome::Refused(
            RegistryRefusal::TargetRepositoryMoved,
        ));
    }

    nerve_store::relocate_registry_entry(
        conn,
        repo_id,
        registry_id,
        &target.root.to_string_lossy(),
    )?;
    nerve_store::record_registry_observation(
        conn,
        repo_id,
        registry_id,
        target.state_id.as_deref(),
    )?;
    Ok(RegistryOutcome::Done(
        nerve_store::registry_entry(conn, repo_id, registry_id)?.unwrap_or(entry),
    ))
}

/// Retire an entry and withdraw the links that resolved through it. **Never a delete.**
///
/// Both writes happen inside one transaction, because an entry retired with its links still active
/// and links withdrawn with their entry still active are each a half-recorded fact. The row itself
/// survives: `registry_entry_removed` is a report made from the kept row, and there is no purge verb
/// in this slice.
pub fn remove_registry_target(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
) -> Result<RegistryOutcome> {
    let entry = match nerve_store::registry_entry(conn, repo_id, registry_id)? {
        Some(entry) => entry,
        None => {
            return Ok(RegistryOutcome::Refused(
                RegistryRefusal::NoSuchRegistryEntry,
            ))
        }
    };
    if entry.status == RegistryEntryStatus::Tombstoned {
        return Ok(RegistryOutcome::Refused(
            RegistryRefusal::RegistryEntryTombstoned,
        ));
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(nerve_store::StoreError::from)?;
    nerve_store::withdraw_links_for_registry_entry(&tx, repo_id, registry_id)?;
    nerve_store::tombstone_registry_entry(&tx, repo_id, registry_id)?;
    tx.commit().map_err(nerve_store::StoreError::from)?;

    Ok(RegistryOutcome::Done(
        nerve_store::registry_entry(conn, repo_id, registry_id)?.unwrap_or(entry),
    ))
}

/// Reduce a name to a local registry identifier, or refuse to.
///
/// The id is a primary-key field that a surface prints and a later command hands back as an
/// argument, so it is kept to a conservative set rather than accepting whatever a directory happens
/// to be called: ASCII letters, digits, `-` and `_`, with every run of anything else becoming a
/// single `-`, then trimmed. A directory whose name reduces to nothing is **refused**, never
/// replaced with a generated id the user never chose and would have to be told.
///
/// **`.` is not in the keep set**, and that is the one non-obvious choice. Keeping it looked
/// harmless until the unit test below showed `a/../b` reducing to `a-..-b`: an id that is not a path
/// and cannot be used as one, but reads like a traversal wherever it is printed. An id has no
/// business resembling a path, so the dot goes and `service.io` becomes `service-io`.
fn usable_registry_id(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            out.push(character.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The database path a registered target's index lives at, for a caller that needs to hash it.
///
/// Exposed so that a test can verify the neighbour's bytes are unchanged without rebuilding the
/// path itself — a second spelling of it is a second thing to get wrong.
pub fn target_database_path(root: &Path) -> PathBuf {
    config::db_path(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registry_id_keeps_what_is_safe_and_refuses_what_is_left() {
        assert_eq!(usable_registry_id("service-b"), Some("service-b".into()));
        assert_eq!(usable_registry_id("Service B"), Some("service-b".into()));
        // Nothing an id produces may read like a path. This case is why `.` is not kept: with it,
        // the reduction was `a-..-b`, which is not a path and looks like one.
        assert_eq!(usable_registry_id("a/../b"), Some("a-b".into()));
        assert_eq!(usable_registry_id(".hidden"), Some("hidden".into()));
        assert_eq!(usable_registry_id("service.io"), Some("service-io".into()));
        assert_eq!(usable_registry_id("///"), None);
        assert_eq!(usable_registry_id("++"), None);
        assert_eq!(usable_registry_id("  "), None);
        assert_eq!(usable_registry_id(""), None);
    }

    #[test]
    fn every_refusal_has_a_distinct_name_and_a_statement() {
        let mut names: Vec<&str> = RegistryRefusal::ALL.iter().map(|r| r.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two refusals share a name");
        for refusal in RegistryRefusal::ALL {
            assert!(refusal.statement().len() > 40, "{refusal} has no statement");
        }
    }

    /// The four states this slice can reach, and the eight it cannot — asserted, not described.
    #[test]
    fn availability_reaches_exactly_four_of_the_twelve_freshness_states() {
        let reachable = [
            RegistryAvailability::Available,
            RegistryAvailability::PartiallyIndexed,
            RegistryAvailability::EntryRemoved,
            RegistryAvailability::Missing(RegistryRefusal::PathDoesNotExist),
            RegistryAvailability::Moved {
                observed_repository_id: "repo-other".into(),
            },
            RegistryAvailability::Refused(RegistryRefusal::SymlinkEscape),
        ];
        let mut produced: Vec<ContractFreshness> =
            reachable.iter().filter_map(|a| a.freshness()).collect();
        produced.sort_unstable();
        produced.dedup();
        assert_eq!(
            produced,
            vec![
                ContractFreshness::TargetRepositoryMissing,
                ContractFreshness::TargetRepositoryMoved,
                ContractFreshness::TargetPartiallyIndexed,
                ContractFreshness::RegistryEntryRemoved,
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
            "the registry alone reaches four of the twelve; the other eight are properties of a \
             contract link and arrive with 13b/13c"
        );
        assert_eq!(ContractFreshness::ALL.len(), 12);
    }

    /// A refusal is never rendered as a missing repository.
    #[test]
    fn a_refusal_carries_no_freshness_value() {
        for reason in [
            RegistryRefusal::SymlinkEscape,
            RegistryRefusal::TargetSchemaTooNew,
            RegistryRefusal::TargetUnreadable,
        ] {
            let availability = RegistryAvailability::Refused(reason);
            assert_eq!(availability.freshness(), None);
            assert_eq!(availability.as_str(), "refused");
            assert_eq!(availability.refusal(), Some(reason));
        }
    }
}
