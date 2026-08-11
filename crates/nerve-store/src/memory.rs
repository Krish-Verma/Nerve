//! Human-confirmed project memory: `memory`, `memory_citation` and `memory_event`
//! (schema v9, Slice 14a; the closed vocabularies and the lifecycle writes, schema v10,
//! Slice 14b-i).
//!
//! These three tables are **not** part of the evidence graph and are deliberately absent from the
//! canonical dump. A memory record is a human sentence *about* one subject, which is not the shape
//! of an `assertion`: an assertion is a relation between two entities, and `assertion_state` is
//! defined as a pure function of machine observations. Putting a human's sentence in that table
//! would be *"an unstructured agent-memory store that silently becomes truth"*, arrived at through
//! the schema instead of through a feature — so memory is offered *beside* evidence and never mixed
//! into it, the same placement decision `git_commit` took in 12b and `contract_link` in 13a-i.
//! [`crate::schema`]'s `V9` doc comment carries the argument beside the DDL.
//!
//! **No `EvidenceSourceType`, no `Relation`, no row in `assertion_state`.** Inventing a relation for
//! this (`HUMAN_NOTED_ABOUT`) would repeat the mistake `ADR_DESCRIBES_COMPONENT` was refused for.
//! The invariant is stated as a test rather than as a convention: `assertion`, `observation`,
//! `occurrence` and `assertion_state` are byte-identical across every operation in this module.
//!
//! Five properties of this module are load-bearing.
//!
//! 1. **A record's subject is a snapshot, never a live pointer.** `entity` rows are pruned on every
//!    re-index ([`crate::prune::prune_orphans`], `prune.rs:376`), so a foreign key would either
//!    block re-indexing a file a human wrote a note about, or let a routine re-index silently
//!    destroy the note. The live subject is resolved at read time and the verdict is *reported* —
//!    [`MemorySubjectResolution`] — never guessed.
//! 2. **A moved subject re-attaches only when an `identity_link` says so.** `CLAUDE.md` §3 forbids
//!    establishing identity by fuzzy name matching, and a subject re-attached by resemblance would
//!    transfer a human's sentence onto a different file without saying it had.
//! 3. **Supersession is stored in one direction.** `memory.supersedes_memory_id` is the only column
//!    that holds it; [`superseded_by`] derives the inverse. Two independently writable directions of
//!    one fact can disagree with nothing to notice — the shape row 13 §4.1 rejected.
//! 4. **Nothing here deletes.** There is no delete verb, and `memory_event` is append-only: this
//!    module contains no `DELETE` and no `UPDATE` against it, and a source scan asserts that.
//! 5. **A lifecycle write is one transaction, or it is nothing.** [`propose_memory`],
//!    [`confirm_memory`], [`invalidate_memory`], [`cite_memory`] and [`supersede_memory`] each
//!    change a status and append the event that records the change inside a single transaction. A
//!    crash or a refusal between the two halves would leave a record that changed with no record of
//!    changing — the failure the audit history exists to prevent. Slice 14a left [`insert_memory`]
//!    and [`append_memory_event`] as the only writers and appended no creating event; those two
//!    are still public, because supersession and the schema tests need them, but a product
//!    lifecycle caller must use the wrappers or an event is skipped.
//!
//! Slice 14b-i also closes the two vocabularies 14a deliberately left open. `operation` is a
//! [`MemoryOperation`] rather than a string, because the store no longer has a reason to accept an
//! arbitrary verb: 14b owns the verbs, and 14d's guard can only guard a vocabulary that exists.
//! `scope` stays a `String` on [`MemoryRow`] and is enumerated by the schema — see
//! [`crate::schema`]'s `V10` doc comment for why the column is closed and what it cost.

use std::collections::BTreeMap;

use nerve_core::vocab::{MemoryOperation, MemoryStatus, MemorySubjectResolution, MemoryView};
use rusqlite::{params, Connection, Row};

use crate::error::{Result, StoreError};

/// The subject of a memory record, as it was when the record was written.
///
/// **Every field is a copy, and none of them is a foreign key.** Together they are what lets a
/// record whose subject has been pruned still *name* what it was about: without the kind, name,
/// path and selector, a note about a deleted file would be a note about nothing.
///
/// `selector` is the string the human actually typed. It is kept verbatim rather than re-derived,
/// because re-deriving it from the other fields would silently rewrite what the human asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySubject {
    /// The subject's `entity_id` at the moment the record was written. **Not** a foreign key.
    pub entity_id: String,
    /// Its `EntityKind` then.
    pub kind: String,
    /// Its name then.
    pub name: String,
    /// Its repository-relative path then. Empty for the repository entity, which is no file.
    pub path: String,
    /// The selector the human named it with, verbatim.
    pub selector: String,
}

/// One human-confirmed memory record.
///
/// `author_label` is a **local label, not an identity**. Nerve has no accounts, no network and no
/// identity provider, so the column records what the caller said it was and nothing verified it.
/// It is untrusted repository-adjacent content on T7's terms, exactly like
/// [`crate::registry::RegistryEntryRow::display_name`].
///
/// `claim_key` is optional and it is what makes a conflict reportable. Only records agreeing on
/// repository + subject + scope + `claim_key` may be reported [`MemoryView::Conflicted`]; records
/// without one are [`MemoryView::MultipleActive`], because several notes about one file is ordinary
/// and calling it a contradiction would be a claim the evidence — two English sentences — cannot
/// support.
///
/// `status` holds one of exactly four stored values. `potentially_stale`, `conflicted` and
/// `multiple_active` are [`MemoryView`]s computed by [`read_memory`] and never written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    /// Stable local id. Supplied by the caller, in the manner of `repo_registry.registry_id`.
    pub memory_id: String,
    /// What the record is about, as it was then.
    pub subject: MemorySubject,
    /// The repository state the record was confirmed against. A foreign key: nothing deletes one.
    pub anchor_state_id: String,
    /// A caller-supplied grouping label. The store never interprets it.
    pub scope: String,
    /// What named claim this record answers, if it answers one.
    pub claim_key: Option<String>,
    /// The human's own sentence. Never rewritten, including by supersession.
    pub content: String,
    /// What the caller said it was. **A label, not an identity.**
    pub author_label: String,
    /// When the record was written. Stamped by [`insert_memory`].
    pub created_at: String,
    /// The stored lifecycle. Four values.
    pub status: MemoryStatus,
    /// The record this one replaces. The **only** stored direction; the inverse is derived.
    pub supersedes_memory_id: Option<String>,
    /// When it stopped being true. `Some` exactly when `status` is [`MemoryStatus::Invalidated`].
    pub invalidated_at: Option<String>,
    /// Why it stopped being true, when a reason was given.
    pub invalidation_reason: Option<String>,
}

/// One passage a memory record cites.
///
/// The **same durable-snapshot treatment as the subject**, for the identical reason: a citation into
/// an entity that has since been pruned would otherwise be a dangling pointer with nothing left to
/// name what was cited. `cited_entity_id_snapshot` is optional — a citation may name a path and a
/// span and no entity at all — and the schema refuses an entity id that arrives without its
/// snapshot beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCitationRow {
    /// Surrogate key, assigned by SQLite. `None` before the row is written.
    pub citation_id: Option<i64>,
    /// The record that makes the citation.
    pub memory_id: String,
    /// The cited entity's id then, when the citation names a thing rather than only a place.
    pub cited_entity_id: Option<String>,
    /// Its kind then. Present exactly when `cited_entity_id` is.
    pub cited_kind: Option<String>,
    /// Its name then. Present exactly when `cited_entity_id` is.
    pub cited_name: Option<String>,
    /// Repository-relative path of the cited passage. Never empty.
    pub cited_path: String,
    /// Where in that file, as `start_line:end_line`, or `None` for a whole file.
    pub cited_span: Option<String>,
    /// The repository state the citation was taken at. A foreign key.
    pub cited_at_state: String,
    /// When the citation was recorded. Stamped by [`insert_memory_citation`].
    pub created_at: String,
}

/// One entry in a record's audit history.
///
/// **Append-only, and never deleted — including on invalidation.** Nothing in this module updates or
/// deletes a `memory_event` row, and that absence is the enforcement: a trigger was considered and
/// declined because it can be dropped by a later migration, whereas a source scan for a `DELETE` or
/// an `UPDATE` against this table cannot be satisfied by anything except the code not existing.
///
/// `operation` is a closed [`MemoryOperation`] as of Slice 14b-i. 14a left it an open string
/// because *"14b's commands own the verbs"*; they exist now, and an open string would leave 14d's
/// vocabulary guard with nothing it could fail against — the shape 12c-iv found eight times over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEventRow {
    /// Surrogate key, assigned by SQLite. `None` before the row is written.
    pub event_id: Option<i64>,
    /// The record the event is about.
    pub memory_id: String,
    /// When it happened. Stamped by [`append_memory_event`].
    pub at: String,
    /// What was done. One of five values, and only [`MemoryOperation::Cited`] leaves the status be.
    pub operation: MemoryOperation,
    /// The status before. `None` on the event that created the record.
    pub from_status: Option<MemoryStatus>,
    /// The status after. Equal to `from_status` for an event that changed no status.
    pub to_status: MemoryStatus,
    /// Anything the caller wanted recorded alongside.
    pub note: Option<String>,
}

/// What became of a record's subject, and which live entity or entities it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySubjectReport {
    /// The verdict. Reported, never guessed.
    pub resolution: MemorySubjectResolution,
    /// The live entity ids the subject resolves to, ordered.
    ///
    /// One for [`MemorySubjectResolution::Resolved`] and
    /// [`MemorySubjectResolution::ResolvedThroughIdentityLink`], several for
    /// [`MemorySubjectResolution::Ambiguous`] — **every candidate, none promoted** — and empty for
    /// the two verdicts that reach nothing.
    pub live_entity_ids: Vec<String>,
}

/// One memory record with everything that is true of it *now*.
///
/// The stored row is `row`; everything else on this struct is computed by the read and stored
/// nowhere. That split is the point of the type: a surface renders qualifications it did not have
/// to keep true, and no writer exists that could let one go stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReport {
    /// The stored row, exactly as written.
    pub row: MemoryRow,
    /// What became of its subject.
    pub subject: MemorySubjectReport,
    /// Query-time qualifications, in [`MemoryView`] declaration order, without repeats.
    pub views: Vec<MemoryView>,
    /// The record that replaced this one, derived from the single stored direction.
    pub superseded_by: Option<String>,
    /// The repository state the record was compared against, or `None` if there is none.
    pub current_state_id: Option<String>,
}

/// Every column of `memory`, in the order [`memory_from_row`] reads them.
const MEMORY_COLUMNS: &str = "memory_id, subject_entity_id_snapshot, subject_kind_snapshot, \
                              subject_name_snapshot, subject_path_snapshot, \
                              subject_selector_snapshot, anchor_state_id, scope, claim_key, \
                              content, author_label, created_at, status, supersedes_memory_id, \
                              invalidated_at, invalidation_reason";

fn memory_from_row(row: &Row<'_>) -> Result<MemoryRow> {
    let status: String = row.get(12)?;
    Ok(MemoryRow {
        memory_id: row.get(0)?,
        subject: MemorySubject {
            entity_id: row.get(1)?,
            kind: row.get(2)?,
            name: row.get(3)?,
            path: row.get(4)?,
            selector: row.get(5)?,
        },
        anchor_state_id: row.get(6)?,
        scope: row.get(7)?,
        claim_key: row.get(8)?,
        content: row.get(9)?,
        author_label: row.get(10)?,
        created_at: row.get(11)?,
        status: status.parse()?,
        supersedes_memory_id: row.get(13)?,
        invalidated_at: row.get(14)?,
        invalidation_reason: row.get(15)?,
    })
}

/// Every column of `memory_citation`, in the order [`citation_from_row`] reads them.
const CITATION_COLUMNS: &str = "citation_id, memory_id, cited_entity_id_snapshot, \
                                cited_kind_snapshot, cited_name_snapshot, cited_path_snapshot, \
                                cited_span_snapshot, cited_at_state, created_at";

fn citation_from_row(row: &Row<'_>) -> Result<MemoryCitationRow> {
    Ok(MemoryCitationRow {
        citation_id: row.get(0)?,
        memory_id: row.get(1)?,
        cited_entity_id: row.get(2)?,
        cited_kind: row.get(3)?,
        cited_name: row.get(4)?,
        cited_path: row.get(5)?,
        cited_span: row.get(6)?,
        cited_at_state: row.get(7)?,
        created_at: row.get(8)?,
    })
}

/// Every column of `memory_event`, in the order [`event_from_row`] reads them.
const EVENT_COLUMNS: &str = "event_id, memory_id, at, operation, from_status, to_status, note";

fn event_from_row(row: &Row<'_>) -> Result<MemoryEventRow> {
    let operation: String = row.get(3)?;
    let from_status: Option<String> = row.get(4)?;
    let to_status: String = row.get(5)?;
    Ok(MemoryEventRow {
        event_id: row.get(0)?,
        memory_id: row.get(1)?,
        at: row.get(2)?,
        operation: operation.parse()?,
        from_status: from_status.map(|value| value.parse()).transpose()?,
        to_status: to_status.parse()?,
        note: row.get(6)?,
    })
}

// ---- writing -----------------------------------------------------------------------------------

/// Record one memory. Returns the row as it was written, with `created_at` filled in.
///
/// **Plain `INSERT`, never `INSERT OR IGNORE`.** Slice 3b shipped a silent data-destruction bug in
/// which `INSERT OR IGNORE` swallowed constraint violations and the process exited zero
/// (`nerve-index/src/pipeline.rs:654-666`). A dropped memory row would read as a note the human
/// never wrote, and memory is the only thing in this database that re-indexing cannot rebuild.
///
/// `created_at` is stamped here rather than supplied, in the manner of every other timestamp in this
/// schema; [`MemoryRow::created_at`] is ignored on the way in and correct on the way out. Everything
/// else is taken from the row, including `status`, so that a caller restoring an already-retired
/// record can — the schema's `CHECK`s refuse the pairings that would make a status and its
/// timestamps disagree.
///
/// **This is the raw writer and it appends no event.** It stays public because [`supersede_memory`]
/// builds on it and the schema tests need a way to put an arbitrary well-formed row on disk. A
/// product lifecycle caller must use [`propose_memory`] instead, which is the same insert with the
/// creating event attached to it inside one transaction — otherwise a record exists whose audit
/// history does not say it was ever written.
pub fn insert_memory(conn: &Connection, repo_id: &str, row: &MemoryRow) -> Result<MemoryRow> {
    conn.execute(
        "INSERT INTO memory
             (memory_id, repo_id, subject_entity_id_snapshot, subject_kind_snapshot,
              subject_name_snapshot, subject_path_snapshot, subject_selector_snapshot,
              anchor_state_id, scope, claim_key, content, author_label, created_at, status,
              supersedes_memory_id, invalidated_at, invalidation_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'), ?13, ?14, ?15, ?16)",
        params![
            row.memory_id,
            repo_id,
            row.subject.entity_id,
            row.subject.kind,
            row.subject.name,
            row.subject.path,
            row.subject.selector,
            row.anchor_state_id,
            row.scope,
            row.claim_key,
            row.content,
            row.author_label,
            row.status.as_str(),
            row.supersedes_memory_id,
            row.invalidated_at,
            row.invalidation_reason,
        ],
    )?;
    memory(conn, repo_id, &row.memory_id)?
        .ok_or_else(|| StoreError::from(rusqlite::Error::QueryReturnedNoRows))
}

/// Record one citation. Returns its assigned `citation_id`.
///
/// The cited passage is stored as a snapshot for the reason the subject is: the entity may be
/// pruned, and a citation that could no longer name what it cited would be worse than no citation.
pub fn insert_memory_citation(
    conn: &Connection,
    repo_id: &str,
    row: &MemoryCitationRow,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO memory_citation
             (repo_id, memory_id, cited_entity_id_snapshot, cited_kind_snapshot,
              cited_name_snapshot, cited_path_snapshot, cited_span_snapshot, cited_at_state,
              created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            repo_id,
            row.memory_id,
            row.cited_entity_id,
            row.cited_kind,
            row.cited_name,
            row.cited_path,
            row.cited_span,
            row.cited_at_state,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Append one event to a record's audit history. Returns its assigned `event_id`.
///
/// The only way anything is ever written to `memory_event`. There is no counterpart that removes or
/// rewrites one: an audit history a later operation can edit is not an audit history, and *"history
/// preserved"* stops being true the moment a delete verb exists.
///
/// `at` is stamped here; [`MemoryEventRow::at`] and [`MemoryEventRow::event_id`] are ignored on the
/// way in.
///
/// **This is the raw appender and it changes no status.** Like [`insert_memory`] it stays public
/// because the lifecycle wrappers and the schema tests need it, but a product caller that appends
/// an event here rather than through [`confirm_memory`], [`invalidate_memory`], [`cite_memory`] or
/// [`supersede_memory`] has written half of a change: the history would record a transition the row
/// did not make.
pub fn append_memory_event(conn: &Connection, repo_id: &str, row: &MemoryEventRow) -> Result<i64> {
    conn.execute(
        "INSERT INTO memory_event
             (repo_id, memory_id, at, operation, from_status, to_status, note)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now'), ?3, ?4, ?5, ?6)",
        params![
            repo_id,
            row.memory_id,
            row.operation.as_str(),
            row.from_status.map(|status| status.as_str()),
            row.to_status.as_str(),
            row.note,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Write `successor` and retire the record it replaces, **in one transaction**.
///
/// The atomicity is the point rather than a convenience. A crash between the status change and the
/// event append would leave a superseded record with no record of being superseded — a retirement
/// that happened and cannot be reported, which is the failure the audit history exists to prevent.
///
/// Three refusals, and each is a fact the schema cannot reach on its own because a `CHECK` sees one
/// row:
///
/// - a successor that names no predecessor is not a supersession;
/// - a predecessor that is not in this repository does not exist for this call;
/// - an **invalidated** predecessor may not be superseded. *"It stopped being true and nothing
///   replaced it"* and *"this replaced it"* are contradictory claims about the same record, and
///   accepting both would quietly turn one into the other.
///
/// Superseding an already-superseded record is refused by `idx_memory_supersedes` instead, as a
/// uniqueness conflict: at most one record may replace any given record, which is what makes
/// [`superseded_by`] single-valued.
///
/// `operation` was a `&str` in Slice 14a, on the ground that the store must not name the product's
/// verbs. 14b names them, so the parameter is a [`MemoryOperation`]: an arbitrary verb is no longer
/// a thing the store has any reason to accept, and the schema stopped accepting one at v10. It
/// stays a parameter rather than becoming [`MemoryOperation::Superseded`] outright because the
/// caller decides what it is doing, and the caller is the only one that knows.
pub fn supersede_memory(
    conn: &Connection,
    repo_id: &str,
    successor: &MemoryRow,
    operation: MemoryOperation,
    note: Option<&str>,
) -> Result<MemoryRow> {
    let predecessor_id = successor.supersedes_memory_id.clone().ok_or_else(|| {
        StoreError::Memory(format!(
            "{} supersedes nothing, so there is no record to retire",
            successor.memory_id
        ))
    })?;

    let tx = conn.unchecked_transaction()?;

    let predecessor = memory(&tx, repo_id, &predecessor_id)?.ok_or_else(|| {
        StoreError::Memory(format!(
            "no memory record {predecessor_id} in this repository"
        ))
    })?;
    if predecessor.status == MemoryStatus::Invalidated {
        return Err(StoreError::Memory(format!(
            "{predecessor_id} was invalidated, so nothing replaced it; superseding it now would \
             turn one claim into the other"
        )));
    }

    let written = insert_memory(&tx, repo_id, successor)?;

    // The successor gets its **own** creating event, and it is not optional.
    //
    // Slice 14b-ii found this missing and it was a real hole: this function reached
    // [`insert_memory`] — the raw writer, which appends nothing — so a record born by supersession
    // arrived with an **empty audit history**, while an otherwise identical record born by
    // [`propose_memory`] arrived with a `proposed` event. `nerve memory events <successor>` then
    // printed nothing, and the record could not say it had ever been written. That is the same
    // defect as a status change without its event, one table over, and §4's *"every mutating
    // lifecycle operation appends a typed event"* does not carve out the operation that creates a
    // record.
    //
    // `from_status` is `None` for the reason it is in [`propose_memory`]: there was no prior
    // status. The event is the successor's, and the `superseded` event appended below is the
    // predecessor's — two records change here, so two events are recorded, and reading either
    // record's history now tells the whole of what happened to it.
    append_memory_event(
        &tx,
        repo_id,
        &MemoryEventRow {
            event_id: None,
            memory_id: successor.memory_id.clone(),
            at: String::new(),
            operation,
            from_status: None,
            to_status: written.status,
            note: note.map(str::to_string),
        },
    )?;

    let changed = tx.execute(
        "UPDATE memory SET status = ?3
          WHERE repo_id = ?1 AND memory_id = ?2 AND status = ?4",
        params![
            repo_id,
            predecessor_id,
            MemoryStatus::Superseded.as_str(),
            predecessor.status.as_str(),
        ],
    )?;
    if changed == 0 {
        return Err(StoreError::Memory(format!(
            "{predecessor_id} changed status while it was being superseded"
        )));
    }

    append_memory_event(
        &tx,
        repo_id,
        &MemoryEventRow {
            event_id: None,
            memory_id: predecessor_id,
            at: String::new(),
            operation,
            from_status: Some(predecessor.status),
            to_status: MemoryStatus::Superseded,
            note: note.map(str::to_string),
        },
    )?;

    tx.commit()?;
    Ok(written)
}

// ---- the lifecycle, each write one transaction -------------------------------------------------

/// Write a new record as [`MemoryStatus::Proposed`] and append its creating event, **in one
/// transaction**.
///
/// [`MemoryRow::status`] is ignored and the record enters at `proposed`, because that is what
/// proposing is. [`insert_memory`] stays permissive about the status on purpose — a caller
/// restoring an already-retired row needs it to be — and this is the lifecycle door, where the
/// status is the verb's rather than the caller's. The event carries `from_status = None`: there was
/// no prior status, and writing one would invent a state the record was never in.
///
/// **A proposal may not name a predecessor.** A record carrying `supersedes_memory_id` while its
/// predecessor stays active is precisely the half-applied state [`supersede_memory`]'s transaction
/// exists to prevent — and worse, it is unrecoverable: the unique index means the predecessor now
/// has its one successor, so no later call could retire it. Superseding is one operation with one
/// door.
pub fn propose_memory(conn: &Connection, repo_id: &str, row: &MemoryRow) -> Result<MemoryRow> {
    if row.supersedes_memory_id.is_some() {
        return Err(StoreError::Memory(format!(
            "{} names a predecessor, and a proposal cannot retire one; use supersede_memory, \
             which changes both records together",
            row.memory_id
        )));
    }

    let tx = conn.unchecked_transaction()?;
    let proposed = MemoryRow {
        status: MemoryStatus::Proposed,
        invalidated_at: None,
        invalidation_reason: None,
        ..row.clone()
    };
    let written = insert_memory(&tx, repo_id, &proposed)?;
    append_memory_event(
        &tx,
        repo_id,
        &MemoryEventRow {
            event_id: None,
            memory_id: written.memory_id.clone(),
            at: String::new(),
            operation: MemoryOperation::Proposed,
            from_status: None,
            to_status: MemoryStatus::Proposed,
            note: None,
        },
    )?;
    tx.commit()?;
    Ok(written)
}

/// Confirm a proposal: [`MemoryStatus::Proposed`] → [`MemoryStatus::Active`], with the event, **in
/// one transaction**.
///
/// Confirming is the only transition this product claims a human made, and §1 of row 14's plan is
/// explicit about what that claim rests on: Nerve has no accounts and no identity provider, so what
/// makes this the human's act is the **surface it arrived on** and never an identity it checked.
/// This function is reachable from the CLI and from nothing else.
///
/// **Refused from every other status**, and the message says which one it actually is — a refusal
/// that only said "not proposed" would send a reader back to the database to find out whether they
/// had already confirmed it or someone had retired it.
pub fn confirm_memory(
    conn: &Connection,
    repo_id: &str,
    memory_id: &str,
    note: Option<&str>,
) -> Result<MemoryRow> {
    transition(
        conn,
        repo_id,
        memory_id,
        &[MemoryStatus::Proposed],
        MemoryStatus::Active,
        MemoryOperation::Confirmed,
        note,
        None,
    )
}

/// Record that a memory stopped being true and **nothing replaced it**, with the event, **in one
/// transaction**.
///
/// Sets `invalidated_at` — an ending is a status and a moment, and the schema's `CHECK` refuses one
/// without the other — and records `reason` when a reason was given.
///
/// **Refused from [`MemoryStatus::Superseded`].** *"It stopped being true and nothing replaced it"*
/// and *"this record replaced it"* are contradictory claims about one record, and accepting the
/// second after the first would quietly turn one into the other: the successor would still be
/// active, still naming a predecessor, and the predecessor would now say nothing succeeded it. That
/// is the mirror of the refusal [`supersede_memory`] already makes in the other direction, and the
/// pair is what keeps the two statuses distinguishable at all.
///
/// **Refused from [`MemoryStatus::Invalidated`]**, because a second ending would move
/// `invalidated_at` and overwrite the reason — the audit history would keep both events and the row
/// would keep only the later one, so the row and its history would disagree.
pub fn invalidate_memory(
    conn: &Connection,
    repo_id: &str,
    memory_id: &str,
    reason: Option<&str>,
    note: Option<&str>,
) -> Result<MemoryRow> {
    transition(
        conn,
        repo_id,
        memory_id,
        &[MemoryStatus::Proposed, MemoryStatus::Active],
        MemoryStatus::Invalidated,
        MemoryOperation::Invalidated,
        note,
        Some(reason),
    )
}

/// Attach a citation to a record and append the event that says so, **in one transaction**.
///
/// The only lifecycle operation that changes no status, so its event carries
/// `from_status == to_status == the record's status right now` —
/// [`MemoryOperation::changes_status`] is what states that in the vocabulary, and this is where it
/// is honoured. A citation is not a transition and pretending otherwise would put a status change
/// in the history that the row never made.
///
/// The status is read inside the transaction rather than taken from the caller, because a caller
/// holding a [`MemoryRow`] read a moment ago would stamp the event with a status the record may
/// have left. Returns the citation's assigned `citation_id`.
pub fn cite_memory(
    conn: &Connection,
    repo_id: &str,
    citation: &MemoryCitationRow,
    note: Option<&str>,
) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;

    let record = memory(&tx, repo_id, &citation.memory_id)?.ok_or_else(|| {
        StoreError::Memory(format!(
            "no memory record {} in this repository",
            citation.memory_id
        ))
    })?;

    let citation_id = insert_memory_citation(&tx, repo_id, citation)?;
    append_memory_event(
        &tx,
        repo_id,
        &MemoryEventRow {
            event_id: None,
            memory_id: citation.memory_id.clone(),
            at: String::new(),
            operation: MemoryOperation::Cited,
            from_status: Some(record.status),
            to_status: record.status,
            note: note.map(str::to_string),
        },
    )?;

    tx.commit()?;
    Ok(citation_id)
}

/// One status change plus its event, in one transaction. The body [`confirm_memory`] and
/// [`invalidate_memory`] share.
///
/// `from` is the set of statuses the transition is admissible from; anything else is refused and
/// the message names the status the record is **actually** in. `invalidation` is `Some(reason)`
/// exactly when the target status is [`MemoryStatus::Invalidated`], which is what pairs the
/// timestamp with the status the schema's `CHECK` requires them paired in.
///
/// The `UPDATE` re-states the expected status in its `WHERE` clause even though the row was just
/// read inside this transaction. That is not redundant: `unchecked_transaction` does not promise
/// serialisability against another connection to the same file, and a zero-row update is a fact
/// worth reporting rather than a silent no-op that would leave an event behind describing a change
/// that did not happen.
#[allow(clippy::too_many_arguments)]
fn transition(
    conn: &Connection,
    repo_id: &str,
    memory_id: &str,
    from: &[MemoryStatus],
    to: MemoryStatus,
    operation: MemoryOperation,
    note: Option<&str>,
    invalidation: Option<Option<&str>>,
) -> Result<MemoryRow> {
    let tx = conn.unchecked_transaction()?;

    let record = memory(&tx, repo_id, memory_id)?.ok_or_else(|| {
        StoreError::Memory(format!("no memory record {memory_id} in this repository"))
    })?;

    if !from.contains(&record.status) {
        let admitted = from
            .iter()
            .map(|status| status.as_str())
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(StoreError::Memory(format!(
            "{memory_id} is {}, and {operation} is only admissible from {admitted}",
            record.status
        )));
    }

    let changed = match invalidation {
        Some(reason) => tx.execute(
            "UPDATE memory
                SET status = ?3,
                    invalidated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                    invalidation_reason = ?5
              WHERE repo_id = ?1 AND memory_id = ?2 AND status = ?4",
            params![
                repo_id,
                memory_id,
                to.as_str(),
                record.status.as_str(),
                reason,
            ],
        )?,
        None => tx.execute(
            "UPDATE memory SET status = ?3
              WHERE repo_id = ?1 AND memory_id = ?2 AND status = ?4",
            params![repo_id, memory_id, to.as_str(), record.status.as_str()],
        )?,
    };
    if changed == 0 {
        return Err(StoreError::Memory(format!(
            "{memory_id} changed status while it was being {operation}"
        )));
    }

    append_memory_event(
        &tx,
        repo_id,
        &MemoryEventRow {
            event_id: None,
            memory_id: memory_id.to_string(),
            at: String::new(),
            operation,
            from_status: Some(record.status),
            to_status: to,
            note: note.map(str::to_string),
        },
    )?;

    let written = memory(&tx, repo_id, memory_id)?
        .ok_or_else(|| StoreError::from(rusqlite::Error::QueryReturnedNoRows))?;
    tx.commit()?;
    Ok(written)
}

// ---- reading the stored rows -------------------------------------------------------------------

/// One memory record by its id, whatever its status.
///
/// A retired record is returned like any other. Filtering one out here would make *"what did we once
/// believe and no longer do"* unanswerable at exactly the moment it becomes the answer — the same
/// rule [`crate::registry::registry_entry`] applies to a tombstone.
pub fn memory(conn: &Connection, repo_id: &str, memory_id: &str) -> Result<Option<MemoryRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MEMORY_COLUMNS} FROM memory WHERE repo_id = ?1 AND memory_id = ?2"
    ))?;
    let mut rows = stmt.query(params![repo_id, memory_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(memory_from_row(row)?)),
        None => Ok(None),
    }
}

/// Every memory record in a repository, retired ones included, ordered by `memory_id`.
pub fn list_memory(conn: &Connection, repo_id: &str) -> Result<Vec<MemoryRow>> {
    collect_memory(
        conn,
        &format!("SELECT {MEMORY_COLUMNS} FROM memory WHERE repo_id = ?1 ORDER BY memory_id"),
        params![repo_id],
    )
}

/// Every record written about one subject, by the subject's **snapshot** id.
///
/// The lookup is on the snapshot rather than on a live entity, which is what makes it keep working
/// after the subject is pruned. Callers holding a live entity id that the subject moved *to* will
/// not find records written about the id it moved *from*; [`read_memory_for_subject`] reports the
/// link verdict, and re-attaching by name is refused outright.
pub fn memory_for_subject(
    conn: &Connection,
    repo_id: &str,
    subject_entity_id: &str,
) -> Result<Vec<MemoryRow>> {
    collect_memory(
        conn,
        &format!(
            "SELECT {MEMORY_COLUMNS} FROM memory
              WHERE repo_id = ?1 AND subject_entity_id_snapshot = ?2 ORDER BY memory_id"
        ),
        params![repo_id, subject_entity_id],
    )
}

/// Every record in one scope, ordered by `memory_id`.
pub fn memory_in_scope(conn: &Connection, repo_id: &str, scope: &str) -> Result<Vec<MemoryRow>> {
    collect_memory(
        conn,
        &format!(
            "SELECT {MEMORY_COLUMNS} FROM memory
              WHERE repo_id = ?1 AND scope = ?2 ORDER BY memory_id"
        ),
        params![repo_id, scope],
    )
}

/// The `LIKE` pattern that matches one literal substring.
///
/// `%` and `_` are `LIKE`'s two wildcards and `\` is the escape character the statement names, so
/// all three are escaped here. A human searching for `100%` means those four characters, and a
/// pattern that let the `%` through would match **every** record and report the lot as hits — a
/// false positive produced by punctuation, which is worse than no search at all.
fn like_contains_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for character in query.chars() {
        if matches!(character, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(character);
    }
    out.push('%');
    out
}

/// Every record whose `content` or `claim_key` contains `query`, each with its qualifications.
///
/// A substring match over the two columns a human wrote, and deliberately not over the subject
/// snapshot: a search that also matched paths would answer *"records near this file"* with the same
/// result set as *"records that say this"*, and the caller could not tell which question was
/// answered. Subject is already its own lookup — [`read_memory_for_subject`].
///
/// **Case-insensitive for ASCII only.** That is SQLite's `LIKE` without ICU, and the limit is
/// stated rather than hidden: a query containing non-ASCII letters matches case-sensitively.
/// Nothing here folds case itself, because `lower()` is ASCII-only too and doing it twice would
/// look like a promise this can keep.
///
/// Ordered by `memory_id`, like every other read here, and retired records are included for the
/// reason [`memory`] returns them: *"what did we once believe"* is exactly the question a search
/// over a superseded sentence answers.
pub fn search_memory(conn: &Connection, repo_id: &str, query: &str) -> Result<Vec<MemoryReport>> {
    let rows = collect_memory(
        conn,
        &format!(
            "SELECT {MEMORY_COLUMNS} FROM memory
              WHERE repo_id = ?1
                AND (content LIKE ?2 ESCAPE '\\' OR claim_key LIKE ?2 ESCAPE '\\')
              ORDER BY memory_id"
        ),
        params![repo_id, like_contains_pattern(query)],
    )?;
    reports(conn, repo_id, rows)
}

fn collect_memory(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<Vec<MemoryRow>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(args)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(memory_from_row(row)?);
    }
    Ok(out)
}

/// Every citation a record makes, ordered by `citation_id`.
pub fn memory_citations(
    conn: &Connection,
    repo_id: &str,
    memory_id: &str,
) -> Result<Vec<MemoryCitationRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CITATION_COLUMNS} FROM memory_citation
          WHERE repo_id = ?1 AND memory_id = ?2 ORDER BY citation_id"
    ))?;
    let mut rows = stmt.query(params![repo_id, memory_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(citation_from_row(row)?);
    }
    Ok(out)
}

/// A record's whole audit history, oldest first.
///
/// Every event ever appended, including the ones that precede an invalidation. Nothing removes one,
/// so this is the complete history by construction rather than by filtering.
pub fn memory_events(
    conn: &Connection,
    repo_id: &str,
    memory_id: &str,
) -> Result<Vec<MemoryEventRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EVENT_COLUMNS} FROM memory_event
          WHERE repo_id = ?1 AND memory_id = ?2 ORDER BY event_id"
    ))?;
    let mut rows = stmt.query(params![repo_id, memory_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(event_from_row(row)?);
    }
    Ok(out)
}

/// The record that replaced this one, **derived** from the one stored direction.
///
/// There is no `superseded_by` column and there must not be: two independently writable directions
/// of one fact can disagree with nothing in the schema to notice. `idx_memory_supersedes` is unique,
/// so this query returns at most one row and the inverse is a function rather than a set.
pub fn superseded_by(conn: &Connection, repo_id: &str, memory_id: &str) -> Result<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT memory_id FROM memory WHERE repo_id = ?1 AND supersedes_memory_id = ?2")?;
    let mut rows = stmt.query(params![repo_id, memory_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

// ---- the derived read --------------------------------------------------------------------------

/// The repository state the database currently describes, if it describes one.
///
/// Read off the most recent extractor run, which is where [`crate::history::history_freshness`]
/// reads it from too: since ADR-0006 no graph row carries a state, so there is nothing else to take
/// it from. `None` means nothing has been indexed, and every subject verdict then becomes
/// [`MemorySubjectResolution::RepositoryStateUnavailable`] rather than
/// [`MemorySubjectResolution::Missing`] — unknown is not an absence.
///
/// **Public because the surfaces need the anchor and must not re-derive it.** `nerve memory
/// propose` stamps this value into `anchor_state_id`, and a second derivation on the CLI side
/// would be a second answer to *"which state does this database describe?"* — the duplication
/// Slice 12c-i-a existed to remove. A caller that gets `None` has a repository nothing has indexed,
/// and a note anchored to nothing has no staleness to derive, so proposing one is refused there
/// rather than anchored to a state invented here.
pub fn current_repository_state(conn: &Connection, repo_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT s.state_id
               FROM extractor_run r
               JOIN repository_state s ON s.state_id = r.state_id
              WHERE r.repo_id = ?1
              ORDER BY r.run_id DESC LIMIT 1",
            params![repo_id],
            |row| row.get::<_, String>(0),
        )
        .ok())
}

/// How many active records share each `(subject, scope)` and each `(subject, scope, claim_key)`.
///
/// Two aggregates, computed once per read rather than once per record, so a list of a hundred
/// records costs the same two queries as one.
struct ActiveGroups {
    by_subject_scope: BTreeMap<(String, String), i64>,
    by_claim: BTreeMap<(String, String, String), i64>,
}

impl ActiveGroups {
    fn load(conn: &Connection, repo_id: &str) -> Result<ActiveGroups> {
        let mut by_subject_scope = BTreeMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT subject_entity_id_snapshot, scope, count(*)
                   FROM memory WHERE repo_id = ?1 AND status = ?2
                  GROUP BY subject_entity_id_snapshot, scope",
            )?;
            let mut rows = stmt.query(params![repo_id, MemoryStatus::Active.as_str()])?;
            while let Some(row) = rows.next()? {
                by_subject_scope.insert((row.get(0)?, row.get(1)?), row.get(2)?);
            }
        }

        let mut by_claim = BTreeMap::new();
        {
            // `claim_key IS NOT NULL` is the whole gate. A record answering no named claim
            // competes with nothing, so it may never be counted into a conflict group.
            let mut stmt = conn.prepare(
                "SELECT subject_entity_id_snapshot, scope, claim_key, count(*)
                   FROM memory
                  WHERE repo_id = ?1 AND status = ?2 AND claim_key IS NOT NULL
                  GROUP BY subject_entity_id_snapshot, scope, claim_key",
            )?;
            let mut rows = stmt.query(params![repo_id, MemoryStatus::Active.as_str()])?;
            while let Some(row) = rows.next()? {
                by_claim.insert((row.get(0)?, row.get(1)?, row.get(2)?), row.get(3)?);
            }
        }

        Ok(ActiveGroups {
            by_subject_scope,
            by_claim,
        })
    }

    /// The qualifications true of one record right now, in declaration order and without repeats.
    ///
    /// Only an **active** record gets any of them. A proposed record is not yet a claim about the
    /// world, and a superseded or invalidated one has already been answered — reporting either as
    /// stale or as conflicting would be a qualification on a statement nobody is currently making.
    ///
    /// [`MemoryView::Conflicted`] and [`MemoryView::MultipleActive`] can both hold, and when they do
    /// both are reported: *several records are about this subject* and *two of them answer the same
    /// named claim* are both true, and collapsing them would lose which one the reader is looking at.
    fn views(&self, row: &MemoryRow, current_state: Option<&str>) -> Vec<MemoryView> {
        let mut views = Vec::new();
        if row.status != MemoryStatus::Active {
            return views;
        }

        // Only comparable against a state there is. With nothing indexed, "the anchor is not the
        // current state" has no second operand, and reporting it anyway would be inventing one.
        if let Some(current) = current_state {
            if row.anchor_state_id != current {
                views.push(MemoryView::PotentiallyStale);
            }
        }

        let subject_scope = (row.subject.entity_id.clone(), row.scope.clone());
        if self
            .by_subject_scope
            .get(&subject_scope)
            .copied()
            .unwrap_or(0)
            > 1
        {
            views.push(MemoryView::MultipleActive);
        }

        if let Some(claim_key) = &row.claim_key {
            let claim = (
                row.subject.entity_id.clone(),
                row.scope.clone(),
                claim_key.clone(),
            );
            if self.by_claim.get(&claim).copied().unwrap_or(0) > 1 {
                views.push(MemoryView::Conflicted);
            }
        }

        views.sort_unstable();
        views.dedup();
        views
    }
}

/// What became of one subject snapshot, resolved against the live index.
///
/// The order of the checks is the design:
///
/// 1. **No indexed state at all** → [`MemorySubjectResolution::RepositoryStateUnavailable`]. There
///    is nothing to check the subject against, so *"the subject is gone"* has not been established
///    and must not be reported. This is Slice 7c-i's `Stale` / `Unverified` separation.
/// 2. **The snapshot's id is in `entity`** → [`MemorySubjectResolution::Resolved`].
/// 3. **An `identity_link` reaches live successors** → one is
///    [`MemorySubjectResolution::ResolvedThroughIdentityLink`], several are
///    [`MemorySubjectResolution::Ambiguous`] with every candidate reported and none promoted.
/// 4. Otherwise [`MemorySubjectResolution::Missing`] — and the record is still readable, which is
///    the property the whole snapshot design exists to give.
///
/// **Exactly one link is followed, and links are followed forwards only.** A link records the
/// identity before a move on the left and after it on the right, so walking right-to-left would
/// attach a note about a new path to an entity that predates it. Chains are deliberately not
/// chased: a subject moved twice across two separate indexing runs reports `missing` rather than
/// being reached through a sequence of proposals whose combined strength nothing here measures.
/// That bound is stated rather than hidden, and `missing` remains a true statement about the
/// snapshot — the id genuinely is not in the index.
///
/// **No name matching anywhere.** `CLAUDE.md` §3: identity is never established by fuzzy name
/// matching alone, and a subject re-attached by resemblance would move a human's sentence onto a
/// different file without saying it had.
pub fn resolve_memory_subject(
    conn: &Connection,
    repo_id: &str,
    subject_entity_id: &str,
) -> Result<MemorySubjectReport> {
    let current = current_repository_state(conn, repo_id)?;
    resolve_with_state(conn, repo_id, subject_entity_id, current.as_deref())
}

fn resolve_with_state(
    conn: &Connection,
    repo_id: &str,
    subject_entity_id: &str,
    current_state: Option<&str>,
) -> Result<MemorySubjectReport> {
    if current_state.is_none() {
        return Ok(MemorySubjectReport {
            resolution: MemorySubjectResolution::RepositoryStateUnavailable,
            live_entity_ids: Vec::new(),
        });
    }

    let present: i64 = conn.query_row(
        "SELECT count(*) FROM entity WHERE repo_id = ?1 AND entity_id = ?2",
        params![repo_id, subject_entity_id],
        |row| row.get(0),
    )?;
    if present > 0 {
        return Ok(MemorySubjectReport {
            resolution: MemorySubjectResolution::Resolved,
            live_entity_ids: vec![subject_entity_id.to_string()],
        });
    }

    let mut stmt = conn.prepare(
        "SELECT DISTINCT l.right_entity_id
           FROM identity_link l
           JOIN entity e ON e.entity_id = l.right_entity_id AND e.repo_id = ?1
          WHERE l.repo_id = ?1 AND l.left_entity_id = ?2
          ORDER BY 1",
    )?;
    let mut rows = stmt.query(params![repo_id, subject_entity_id])?;
    let mut live_entity_ids = Vec::new();
    while let Some(row) = rows.next()? {
        live_entity_ids.push(row.get::<_, String>(0)?);
    }

    let resolution = match live_entity_ids.len() {
        0 => MemorySubjectResolution::Missing,
        1 => MemorySubjectResolution::ResolvedThroughIdentityLink,
        _ => MemorySubjectResolution::Ambiguous,
    };
    Ok(MemorySubjectReport {
        resolution,
        live_entity_ids,
    })
}

/// One record with every query-time qualification computed.
pub fn read_memory(
    conn: &Connection,
    repo_id: &str,
    memory_id: &str,
) -> Result<Option<MemoryReport>> {
    let Some(row) = memory(conn, repo_id, memory_id)? else {
        return Ok(None);
    };
    let current = current_repository_state(conn, repo_id)?;
    let groups = ActiveGroups::load(conn, repo_id)?;
    Ok(Some(report(
        conn,
        repo_id,
        row,
        &groups,
        current.as_deref(),
    )?))
}

/// Every record in the repository, each with its qualifications. Ordered by `memory_id`.
pub fn read_memory_all(conn: &Connection, repo_id: &str) -> Result<Vec<MemoryReport>> {
    reports(conn, repo_id, list_memory(conn, repo_id)?)
}

/// Every record about one subject snapshot, each with its qualifications.
pub fn read_memory_for_subject(
    conn: &Connection,
    repo_id: &str,
    subject_entity_id: &str,
) -> Result<Vec<MemoryReport>> {
    reports(
        conn,
        repo_id,
        memory_for_subject(conn, repo_id, subject_entity_id)?,
    )
}

/// Every record in one scope, each with its qualifications.
pub fn read_memory_in_scope(
    conn: &Connection,
    repo_id: &str,
    scope: &str,
) -> Result<Vec<MemoryReport>> {
    reports(conn, repo_id, memory_in_scope(conn, repo_id, scope)?)
}

fn reports(conn: &Connection, repo_id: &str, rows: Vec<MemoryRow>) -> Result<Vec<MemoryReport>> {
    let current = current_repository_state(conn, repo_id)?;
    let groups = ActiveGroups::load(conn, repo_id)?;
    // One resolution per *distinct* subject, so a hundred notes about one file cost one lookup.
    let mut resolved: BTreeMap<String, MemorySubjectReport> = BTreeMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let subject = match resolved.get(&row.subject.entity_id) {
            Some(cached) => cached.clone(),
            None => {
                let fresh =
                    resolve_with_state(conn, repo_id, &row.subject.entity_id, current.as_deref())?;
                resolved.insert(row.subject.entity_id.clone(), fresh.clone());
                fresh
            }
        };
        let views = groups.views(&row, current.as_deref());
        let superseded_by = superseded_by(conn, repo_id, &row.memory_id)?;
        out.push(MemoryReport {
            row,
            subject,
            views,
            superseded_by,
            current_state_id: current.clone(),
        });
    }
    Ok(out)
}

fn report(
    conn: &Connection,
    repo_id: &str,
    row: MemoryRow,
    groups: &ActiveGroups,
    current: Option<&str>,
) -> Result<MemoryReport> {
    let subject = resolve_with_state(conn, repo_id, &row.subject.entity_id, current)?;
    let views = groups.views(&row, current);
    let superseded_by = superseded_by(conn, repo_id, &row.memory_id)?;
    Ok(MemoryReport {
        row,
        subject,
        views,
        superseded_by,
        current_state_id: current.map(str::to_string),
    })
}
