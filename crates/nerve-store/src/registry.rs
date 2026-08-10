//! The cross-repository registry and its contract links: `repo_registry` and `contract_link`
//! (schema v8, Slice 13a-i).
//!
//! These two tables are **not** part of the evidence graph and are deliberately absent from the
//! canonical dump. They record one repository's *stated view of its neighbours*: which other
//! checkouts the user registered, and which declarations in this repository's own manifests name
//! something inside one of them. [`crate::schema`]'s `V8` doc comment carries the placement
//! argument beside the DDL.
//!
//! Four properties of this module are load-bearing.
//!
//! 1. **A link's target is a snapshot, never a live pointer.** `contract_link.target_entity_id` and
//!    every `*_snapshot` column name rows in a database this one does not own, so none of them is a
//!    foreign key — the contrast is `assertion.target_entity_id`, which *is* one
//!    (`schema.rs:97`, `PRAGMA foreign_keys=ON` at `db.rs:37`). The snapshot is what lets a renamed
//!    or deleted target still be *named*, which is the difference between `contract_deleted`,
//!    `target_changed` and `contract_file_missing`.
//! 2. **Nothing here deletes.** [`tombstone_registry_entry`] and [`withdraw_contract_link`] write a
//!    status and a timestamp, because a row that vanished from the table cannot be reported as
//!    having ended, and `registry_entry_removed` is a report made from the kept row. Hard deletion
//!    is a separate, explicit purge and is not implemented here.
//! 3. **Plain `INSERT`, never `INSERT OR IGNORE`.** Slice 3b shipped a silent data-destruction bug
//!    in which `INSERT OR IGNORE` swallowed `NOT NULL` violations, the graph shrank, and the process
//!    exited zero (`crates/nerve-index/src/pipeline.rs:654-666`). A dropped registry entry would
//!    read as a repository the user never registered, and a dropped link as a declaration that was
//!    never made. `insert_commit`'s narrow licence to ignore a duplicate does not extend here:
//!    nothing in these tables is named by the hash of an immutable object.
//! 4. **No filesystem, no second database, no path validation.** This module is storage. Opening
//!    the registered repository is a new trust boundary and belongs to Slice 13a-ii, which owns the
//!    read-only open, the byte verification and the re-validation of a path that is untrusted input
//!    the moment it is written.

use nerve_core::vocab::{ContractLinkStatus, ContractResolutionMethod, RegistryEntryStatus};
use rusqlite::{params, Connection, Row};

use crate::error::Result;

/// One registered neighbour, as this repository recorded it.
///
/// `expected_repository_id` is the identity every later check is made against. Re-validating a
/// registry entry by its **path** is precisely the silent re-pointing that would make every link
/// through the entry describe the wrong repository, so the recorded repository id is what
/// `target_repository_moved` is detected with.
///
/// `local_path` is user-specific and absolute. It lives only in `.nerve/nerve.db`, which
/// `.gitignore` already covers, and no surface may put it anywhere Git can see.
///
/// `display_name` is untrusted repository content on the terms T7 already sets: it is never
/// interpreted, and a surface confines it to the envelope T7 defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntryRow {
    /// Stable local id for this entry. Survives tombstoning, because that is what makes
    /// `registry_entry_removed` answerable.
    pub registry_id: String,
    /// The target's own `repo_id`, recorded at registration.
    pub expected_repository_id: String,
    /// What to call it on a surface. Untrusted repository content.
    pub display_name: String,
    /// Absolute path to the target checkout. Never tracked by Git.
    pub local_path: String,
    /// When the entry was registered. Stamped by [`insert_registry_entry`].
    pub added_at: String,
    /// The target's `state_id` the last time it was read, or `None` if it never has been.
    pub last_seen_state: Option<String>,
    /// When that read happened. `None` exactly when [`RegistryEntryRow::last_seen_state`] is.
    pub last_seen_at: Option<String>,
    /// When the target's availability was last checked, whatever the answer was.
    pub availability_checked_at: Option<String>,
    /// Whether the entry still counts.
    pub status: RegistryEntryStatus,
    /// When it was tombstoned. `None` exactly while it is active.
    pub withdrawn_at: Option<String>,
}

/// One recorded cross-repository link.
///
/// **The source half is local and the target half is a snapshot**, and the two halves are not
/// symmetric on purpose. `source_entity_id` and `source_state_at_resolution` are foreign keys into
/// this database. `target_entity_id`, `target_state_at_resolution` and every `*_snapshot` field name
/// rows in the neighbour's database and are stored as copies, because the neighbour cannot be held
/// still and a dangling pointer with nothing left to name is the failure this shape prevents.
///
/// `expected_contract_version` and `observed_contract_version` are two columns because
/// `contract_version_mismatch` is a disagreement, and one value cannot disagree with itself.
///
/// `unsupported_reason` names a form Nerve declined to resolve. A declined form is recorded, never
/// silently dropped, and a row that carries one may not also carry a resolved target — the schema's
/// `CHECK` enforces that rather than a code review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractLinkRow {
    /// Surrogate key, assigned by SQLite. `None` before the row is written.
    pub link_id: Option<i64>,
    /// This repository's own id, denormalised so a response can state both ends.
    pub source_repository_id: String,
    /// The local `repository_state` the link was resolved at. A foreign key.
    pub source_state_at_resolution: String,
    /// The local entity the declaration sits in, when the contract has one. A foreign key.
    pub source_entity_id: Option<String>,
    /// That entity's kind at resolution.
    pub source_kind_snapshot: Option<String>,
    /// Repository-relative path of the manifest the declaration was read from.
    pub source_path: String,
    /// Where in that file, as `start_line:end_line`. Never empty — a link is quoted from a place.
    pub source_span: String,
    /// Which registry entry the link resolved through.
    pub registry_entry_id: String,
    /// The repository id the target was expected to have. Checked against, never re-derived.
    pub expected_target_repository_id: String,
    /// The target's `state_id` at resolution. **Not** a foreign key: another database's row.
    pub target_state_at_resolution: Option<String>,
    /// The target's `entity_id` at resolution. **Not** a foreign key: another database's row.
    pub target_entity_id: Option<String>,
    /// The target entity's kind at resolution.
    pub target_kind_snapshot: Option<String>,
    /// The target entity's name at resolution.
    pub target_name_snapshot: Option<String>,
    /// The target entity's path at resolution.
    pub target_path_snapshot: Option<String>,
    /// The target entity's span at resolution.
    pub target_span_snapshot: Option<String>,
    /// The semantic relation the declaration states, for a response to render. **Not** a row in the
    /// local assertion graph, and never traversed by an ordinary `path` or `impact` query.
    pub relation_semantics: String,
    /// Which contract this is — the manifest family, not the relation.
    pub contract_kind: String,
    /// What the contract calls itself. Untrusted repository content.
    pub contract_identity: String,
    /// The version this repository's declaration asks for.
    pub expected_contract_version: Option<String>,
    /// The version the target declares.
    pub observed_contract_version: Option<String>,
    /// Which stated declaration the link was drawn from.
    pub resolution_method: ContractResolutionMethod,
    /// The extractor that read the declaration.
    pub extractor_id: String,
    /// Its version.
    pub extractor_version: String,
    /// Anything else the extractor recorded, as a JSON object.
    pub evidence_details: Option<String>,
    /// How ambiguous the resolution was, when it was.
    pub ambiguity: Option<String>,
    /// The named form Nerve declined to resolve, when it declined one.
    pub unsupported_reason: Option<String>,
    /// When the link was first recorded. Stamped by [`insert_contract_link`].
    pub first_seen_at: String,
    /// When it was last re-observed. Stamped alongside `first_seen_at` on insert.
    pub last_seen_at: String,
    /// When it was withdrawn. `None` exactly while it is active.
    pub withdrawn_at: Option<String>,
    /// Whether the declaration is still claimed.
    pub status: ContractLinkStatus,
}

/// Every column of `repo_registry`, in the order [`registry_entry_from_row`] reads them.
const REGISTRY_COLUMNS: &str = "registry_id, expected_repository_id, display_name, local_path, \
                                added_at, last_seen_state, last_seen_at, availability_checked_at, \
                                status, withdrawn_at";

fn registry_entry_from_row(row: &Row<'_>) -> Result<RegistryEntryRow> {
    let status: String = row.get(8)?;
    Ok(RegistryEntryRow {
        registry_id: row.get(0)?,
        expected_repository_id: row.get(1)?,
        display_name: row.get(2)?,
        local_path: row.get(3)?,
        added_at: row.get(4)?,
        last_seen_state: row.get(5)?,
        last_seen_at: row.get(6)?,
        availability_checked_at: row.get(7)?,
        status: status.parse()?,
        withdrawn_at: row.get(9)?,
    })
}

/// Every column of `contract_link`, in the order [`contract_link_from_row`] reads them.
const LINK_COLUMNS: &str = "link_id, source_repository_id, source_state_at_resolution, \
                            source_entity_id, source_kind_snapshot, source_path, source_span, \
                            registry_entry_id, expected_target_repository_id, \
                            target_state_at_resolution, target_entity_id, target_kind_snapshot, \
                            target_name_snapshot, target_path_snapshot, target_span_snapshot, \
                            relation_semantics, contract_kind, contract_identity, \
                            expected_contract_version, observed_contract_version, \
                            resolution_method, extractor_id, extractor_version, evidence_details, \
                            ambiguity, unsupported_reason, first_seen_at, last_seen_at, \
                            withdrawn_at, status";

fn contract_link_from_row(row: &Row<'_>) -> Result<ContractLinkRow> {
    let resolution_method: String = row.get(20)?;
    let status: String = row.get(29)?;
    Ok(ContractLinkRow {
        link_id: row.get(0)?,
        source_repository_id: row.get(1)?,
        source_state_at_resolution: row.get(2)?,
        source_entity_id: row.get(3)?,
        source_kind_snapshot: row.get(4)?,
        source_path: row.get(5)?,
        source_span: row.get(6)?,
        registry_entry_id: row.get(7)?,
        expected_target_repository_id: row.get(8)?,
        target_state_at_resolution: row.get(9)?,
        target_entity_id: row.get(10)?,
        target_kind_snapshot: row.get(11)?,
        target_name_snapshot: row.get(12)?,
        target_path_snapshot: row.get(13)?,
        target_span_snapshot: row.get(14)?,
        relation_semantics: row.get(15)?,
        contract_kind: row.get(16)?,
        contract_identity: row.get(17)?,
        expected_contract_version: row.get(18)?,
        observed_contract_version: row.get(19)?,
        resolution_method: resolution_method.parse()?,
        extractor_id: row.get(21)?,
        extractor_version: row.get(22)?,
        evidence_details: row.get(23)?,
        ambiguity: row.get(24)?,
        unsupported_reason: row.get(25)?,
        first_seen_at: row.get(26)?,
        last_seen_at: row.get(27)?,
        withdrawn_at: row.get(28)?,
        status: status.parse()?,
    })
}

// ---- registry entries --------------------------------------------------------------------------

/// Register one neighbour. Returns the entry as it was written, with `added_at` filled in.
///
/// **Plain `INSERT`.** Registering the same `registry_id` twice is a primary-key conflict and an
/// error, because the second registration is a different statement about the same name — a
/// different path, a different expected repository — and silently keeping either one would leave
/// the user's registry saying something they did not ask for.
///
/// `added_at` is stamped here rather than supplied, in the manner of every other `created_at` in
/// this schema. `status` and `withdrawn_at` are not parameters: a new entry is
/// [`RegistryEntryStatus::Active`] by construction, and the schema's `CHECK` refuses the pairing
/// that would say otherwise.
///
/// **Nothing about `local_path` is checked here.** It is not resolved, not canonicalised and not
/// opened; no filesystem call is made by this module. Registration-time validation and the
/// read-only open of the target belong to Slice 13a-ii, which owns that trust boundary.
pub fn insert_registry_entry(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
    expected_repository_id: &str,
    display_name: &str,
    local_path: &str,
) -> Result<RegistryEntryRow> {
    conn.execute(
        "INSERT INTO repo_registry
             (repo_id, registry_id, expected_repository_id, display_name, local_path, added_at,
              last_seen_state, last_seen_at, availability_checked_at, status, withdrawn_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                 NULL, NULL, NULL, ?6, NULL)",
        params![
            repo_id,
            registry_id,
            expected_repository_id,
            display_name,
            local_path,
            RegistryEntryStatus::Active.as_str(),
        ],
    )?;
    registry_entry(conn, repo_id, registry_id)?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.into())
}

/// One registry entry by its id, tombstoned or not.
///
/// A tombstoned entry is returned like any other. Filtering it out here would make
/// `registry_entry_removed` unanswerable at exactly the moment it becomes the answer.
pub fn registry_entry(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
) -> Result<Option<RegistryEntryRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REGISTRY_COLUMNS} FROM repo_registry
          WHERE repo_id = ?1 AND registry_id = ?2"
    ))?;
    let mut rows = stmt.query(params![repo_id, registry_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(registry_entry_from_row(row)?)),
        None => Ok(None),
    }
}

/// Every registry entry for a repository, tombstones included, ordered by `registry_id`.
///
/// Tombstones are included for the reason [`registry_entry`] returns them: a caller reporting on a
/// link needs the entry it rested on whether or not the entry still counts. A surface that wants
/// only the live ones filters on [`RegistryEntryRow::status`], which is a decision the surface makes
/// and states rather than one the store makes silently.
///
/// Ordered by `registry_id`, which is unique per repository, so the order is total.
pub fn list_registry_entries(conn: &Connection, repo_id: &str) -> Result<Vec<RegistryEntryRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REGISTRY_COLUMNS} FROM repo_registry WHERE repo_id = ?1 ORDER BY registry_id"
    ))?;
    let mut rows = stmt.query(params![repo_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(registry_entry_from_row(row)?);
    }
    Ok(out)
}

/// Retire a registry entry without destroying it. `Ok(true)` if a row changed.
///
/// **This is a tombstone and never a `DELETE`.** The `registry_id` and the recorded
/// `expected_repository_id` survive, which is the only reason a link that resolved through this
/// entry can later be reported as `registry_entry_removed` rather than as a link pointing at
/// nothing nameable. Hard deletion is a separate, explicit purge and is not implemented here.
///
/// Re-tombstoning an already-tombstoned entry changes nothing and returns `Ok(false)`: the
/// `WHERE` clause matches only an active row, so the original `withdrawn_at` is never overwritten
/// with a later moment. The date something ended is not re-datable by asking again.
///
/// **The entry's links are not touched.** Withdrawing them is [`withdraw_links_for_registry_entry`],
/// and the two are separate calls a caller makes inside one transaction, so that a caller wanting
/// the entry retired and the links kept as they were can have that.
pub fn tombstone_registry_entry(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE repo_registry
            SET status = ?3, withdrawn_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE repo_id = ?1 AND registry_id = ?2 AND status = ?4",
        params![
            repo_id,
            registry_id,
            RegistryEntryStatus::Tombstoned.as_str(),
            RegistryEntryStatus::Active.as_str(),
        ],
    )?;
    Ok(changed > 0)
}

/// Record that the target was read, and what state it was in. `Ok(true)` if a row changed.
///
/// Three columns move together because the schema's `CHECK` says two of them must:
/// `last_seen_state` and `last_seen_at` are both written or both cleared, since a state observed at
/// no time and a time with no state are each half a fact. `availability_checked_at` is stamped
/// either way — *the target was looked at* and *the target had an indexed state* are different
/// observations, and a target that has been read and found unindexed must not look like one that
/// was never read.
///
/// `state_id` is **not** a foreign key and cannot be: it names a row in the neighbour's database.
/// Passing `None` is the honest record for a neighbour that has been initialised and never indexed.
///
/// Only an active entry is stamped. A tombstone records what was true when it was retired, and
/// nothing in this slice re-reads a retired entry's target — reading a directory on behalf of an
/// entry the user withdrew is a read the user did not ask for.
pub fn record_registry_observation(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
    state_id: Option<&str>,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE repo_registry
            SET last_seen_state = ?3,
                last_seen_at = CASE WHEN ?3 IS NULL THEN NULL
                                    ELSE strftime('%Y-%m-%dT%H:%M:%fZ','now') END,
                availability_checked_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE repo_id = ?1 AND registry_id = ?2 AND status = ?4",
        params![
            repo_id,
            registry_id,
            state_id,
            RegistryEntryStatus::Active.as_str(),
        ],
    )?;
    Ok(changed > 0)
}

/// Point an active registry entry at a different path. `Ok(true)` if a row changed.
///
/// Only the path moves. `registry_id` and `expected_repository_id` are deliberately not parameters:
/// relocation says *the same repository is now somewhere else*, and letting the expected identity be
/// rewritten in the same call would make relocation the silent re-pointing that
/// `target_repository_moved` exists to catch — performed by Nerve itself, on request.
///
/// **Nothing here verifies that the new path holds the expected repository.** That check needs a
/// read-only open of the target and lives at the second-repository trust boundary, which is
/// `nerve_index::registry::relocate_registry_target` (Slice 13a-ii); this function is the storage
/// half and **must not be called without it**. Called bare it *is* the silent re-pointing
/// `target_repository_moved` exists to catch, performed by Nerve itself on request, so
/// `crates/nerve-index/tests/registry.rs` asserts the verified path is the only route a surface
/// has to it.
///
/// A tombstoned entry is not relocated: the `WHERE` clause matches only an active row, so a retired
/// entry stays where it was when it was retired, and returns `Ok(false)`.
pub fn relocate_registry_entry(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
    local_path: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE repo_registry
            SET local_path = ?3, availability_checked_at = NULL
          WHERE repo_id = ?1 AND registry_id = ?2 AND status = ?4",
        params![
            repo_id,
            registry_id,
            local_path,
            RegistryEntryStatus::Active.as_str(),
        ],
    )?;
    Ok(changed > 0)
}

// ---- contract links ----------------------------------------------------------------------------

/// Record one cross-repository link. Returns its assigned `link_id`.
///
/// **Plain `INSERT`, never `INSERT OR IGNORE`**, for the measured reason in this module's header: a
/// silently dropped link is indistinguishable from a declaration that was never made, which is the
/// one thing this table exists to be able to state.
///
/// The unique index on the logical identity is what makes re-recording an unchanged link a conflict
/// rather than a duplicate — the surrogate key is an autoincrement integer, so without it a
/// re-index of an unchanged tree would append a row on every run. A caller re-observing a link it
/// has already stored updates the existing row rather than inserting a second one.
///
/// `first_seen_at` and `last_seen_at` are both stamped here, from one `strftime` whose value SQLite
/// fixes for the whole statement, so the schema's `last_seen_at >= first_seen_at` holds by
/// construction on a new row. `status` and `withdrawn_at` are taken from the row so that a caller
/// recording an already-withdrawn historical link can do so; the `CHECK` refuses the pairings that
/// would make the two disagree.
pub fn insert_contract_link(
    conn: &Connection,
    repo_id: &str,
    row: &ContractLinkRow,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO contract_link
             (repo_id, source_repository_id, source_state_at_resolution, source_entity_id,
              source_kind_snapshot, source_path, source_span, registry_entry_id,
              expected_target_repository_id, target_state_at_resolution, target_entity_id,
              target_kind_snapshot, target_name_snapshot, target_path_snapshot,
              target_span_snapshot, relation_semantics, contract_kind, contract_identity,
              expected_contract_version, observed_contract_version, resolution_method,
              extractor_id, extractor_version, evidence_details, ambiguity, unsupported_reason,
              first_seen_at, last_seen_at, withdrawn_at, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26,
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'), ?27, ?28)",
        params![
            repo_id,
            row.source_repository_id,
            row.source_state_at_resolution,
            row.source_entity_id,
            row.source_kind_snapshot,
            row.source_path,
            row.source_span,
            row.registry_entry_id,
            row.expected_target_repository_id,
            row.target_state_at_resolution,
            row.target_entity_id,
            row.target_kind_snapshot,
            row.target_name_snapshot,
            row.target_path_snapshot,
            row.target_span_snapshot,
            row.relation_semantics,
            row.contract_kind,
            row.contract_identity,
            row.expected_contract_version,
            row.observed_contract_version,
            row.resolution_method.as_str(),
            row.extractor_id,
            row.extractor_version,
            row.evidence_details,
            row.ambiguity,
            row.unsupported_reason,
            row.withdrawn_at,
            row.status.as_str(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Every contract link for a repository, withdrawn ones included, ordered by `link_id`.
///
/// Withdrawn links are included for the same reason tombstoned entries are: a withdrawn link is the
/// evidence that something used to be declared, and a read that hid it would make `contract_deleted`
/// unreportable. Ordered by the surrogate key, which is unique, so the order is total.
pub fn list_contract_links(conn: &Connection, repo_id: &str) -> Result<Vec<ContractLinkRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LINK_COLUMNS} FROM contract_link WHERE repo_id = ?1 ORDER BY link_id"
    ))?;
    let mut rows = stmt.query(params![repo_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(contract_link_from_row(row)?);
    }
    Ok(out)
}

/// Every contract link recorded through one registry entry, ordered by `link_id`.
pub fn contract_links_for_registry_entry(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
) -> Result<Vec<ContractLinkRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LINK_COLUMNS} FROM contract_link
          WHERE repo_id = ?1 AND registry_entry_id = ?2 ORDER BY link_id"
    ))?;
    let mut rows = stmt.query(params![repo_id, registry_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(contract_link_from_row(row)?);
    }
    Ok(out)
}

/// What makes a contract link *the same link*: exactly the columns `idx_contract_link_identity` is
/// built on.
///
/// A struct rather than six positional parameters, because the whole value of the type is that it
/// cannot drift from the index. If a column joins or leaves the unique index, this declaration is
/// where the change is made and every caller is recompiled against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractLinkIdentity<'a> {
    /// The registry entry the link resolves through.
    pub registry_entry_id: &'a str,
    /// The manifest family the declaration was read from.
    pub contract_kind: &'a str,
    /// What the contract calls itself.
    pub contract_identity: &'a str,
    /// Repository-relative path of the manifest.
    pub source_path: &'a str,
    /// Where in that manifest, as `start_line:end_line`.
    pub source_span: &'a str,
    /// Which stated declaration the link was drawn from.
    pub resolution_method: ContractResolutionMethod,
}

/// The `link_id` of an already-recorded link with this logical identity, if there is one.
///
/// Re-running extraction over an unchanged tree must not append a second row, and the unique index
/// is what makes a second row a conflict rather than a duplicate. A caller looks the identity up,
/// [`touch_contract_link`]s what it finds and [`insert_contract_link`]s what it does not.
///
/// Withdrawn rows are found too. A withdrawn row still occupies the identity — the index carries no
/// `status` column — so a caller that skipped it would insert and hit the conflict.
pub fn contract_link_id(
    conn: &Connection,
    repo_id: &str,
    identity: &ContractLinkIdentity<'_>,
) -> Result<Option<i64>> {
    let mut stmt = conn.prepare(
        "SELECT link_id FROM contract_link
          WHERE repo_id = ?1 AND registry_entry_id = ?2 AND contract_kind = ?3
            AND contract_identity = ?4 AND source_path = ?5 AND source_span = ?6
            AND resolution_method = ?7",
    )?;
    let mut rows = stmt.query(params![
        repo_id,
        identity.registry_entry_id,
        identity.contract_kind,
        identity.contract_identity,
        identity.source_path,
        identity.source_span,
        identity.resolution_method.as_str(),
    ])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// Record that an already-stored link was observed again. `Ok(true)` if a row changed.
///
/// Only `last_seen_at` moves, and only on an active row. `first_seen_at` is the date the
/// declaration was first read and is not re-datable by reading it again — the same rule
/// [`tombstone_registry_entry`] applies to `withdrawn_at`, one column over. The schema's
/// `last_seen_at >= first_seen_at` therefore holds by construction: the new value is *now*, and
/// `first_seen_at` was stamped by an earlier `now`.
///
/// A withdrawn row is deliberately not touched. *"This declaration is still being made"* and
/// *"this declaration ended"* are contradictory claims, and a caller that finds a withdrawn row for
/// a declaration it is reading again has a lifecycle decision to make rather than a timestamp to
/// bump.
pub fn touch_contract_link(conn: &Connection, repo_id: &str, link_id: i64) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE contract_link
            SET last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE repo_id = ?1 AND link_id = ?2 AND status = ?3",
        params![repo_id, link_id, ContractLinkStatus::Active.as_str()],
    )?;
    Ok(changed > 0)
}

/// Retire one contract link without destroying it. `Ok(true)` if a row changed.
///
/// The tombstone discipline of [`tombstone_registry_entry`], one table over: the row is kept so the
/// ending can be reported, and re-withdrawing an already-withdrawn link changes nothing rather than
/// re-dating its ending.
pub fn withdraw_contract_link(conn: &Connection, repo_id: &str, link_id: i64) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE contract_link
            SET status = ?3, withdrawn_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE repo_id = ?1 AND link_id = ?2 AND status = ?4",
        params![
            repo_id,
            link_id,
            ContractLinkStatus::Withdrawn.as_str(),
            ContractLinkStatus::Active.as_str(),
        ],
    )?;
    Ok(changed > 0)
}

/// Withdraw every active link recorded through one registry entry. Returns how many changed.
///
/// The companion to [`tombstone_registry_entry`]: retiring an entry means the declarations that
/// resolved through it no longer resolve, and a caller does both inside one transaction. They are
/// two calls rather than one so that the store never decides on the caller's behalf that a link
/// ended — a link withdrawn without its entry being retired, and an entry retired with its links
/// left as they were, are both things a caller may legitimately want to record.
pub fn withdraw_links_for_registry_entry(
    conn: &Connection,
    repo_id: &str,
    registry_id: &str,
) -> Result<usize> {
    let changed = conn.execute(
        "UPDATE contract_link
            SET status = ?3, withdrawn_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE repo_id = ?1 AND registry_entry_id = ?2 AND status = ?4",
        params![
            repo_id,
            registry_id,
            ContractLinkStatus::Withdrawn.as_str(),
            ContractLinkStatus::Active.as_str(),
        ],
    )?;
    Ok(changed)
}
