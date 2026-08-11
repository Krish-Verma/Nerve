//! The read-only human-confirmed memory API.
//!
//! Two endpoints over the three tables Slice 14a added and the lifecycle Slice 14b-i writes.
//! **Neither of them computes anything and neither of them can change anything.** Every stored
//! column is carried off `nerve-store`'s row, every query-time qualification is
//! [`nerve_store::read_memory`]'s verdict rather than a second derivation of it, and every sentence
//! is a vocabulary's own `note()` (ARCHITECTURE.md invariant 3). The four values that make a
//! record readable at all — its stored lifecycle, its derived views, what became of its subject,
//! and whether something replaced it — are decided in `nerve-store` and rendered here.
//!
//! # There is no write route, and that is the control rather than a gap
//!
//! `nerve serve` has been read-only since Slice 4a: one `PRAGMA query_only` connection per worker,
//! every method but `GET` refused before routing, and the promise proved on the database bytes
//! rather than asserted. Row 14 does not change that. Proposing, confirming, superseding,
//! invalidating and citing are therefore **commands**, and [`BOUNDARY`] carries the exact ones on
//! every answer — the same decision `api::contracts` took for registering a neighbour, and for the
//! same reason: a disabled control would imply an implementation that is deliberately absent.
//!
//! Making this surface writable would relax the one property that makes `query_only` provable, for
//! a mutation whose entire value is that it was deliberate. `docs/plans/
//! slice-14-human-confirmed-memory.md` §5.
//!
//! # What every answer carries
//!
//! One block, assembled in exactly one function — [`block`] — so no endpoint can answer without
//! saying what the answer rests on:
//!
//! ```text
//! repository_id · current_repository_state · requested
//! result_kind · records_in_repository · records_matching
//! truncation · continuation · boundary · vocabulary · limitations
//! ```
//!
//! # A refused filter is not an empty answer
//!
//! An unknown `scope` or `status` is a **400 naming the admitted set**, never a list that came back
//! empty. Against a closed vocabulary `scope=opertions` returning `[]` reads as *there are no
//! notes* when what is true is *there is no such scope*, which is the `absence is not zero` rule
//! 7c-ii's `doctor` and 7b's unresolved account already enforce, and which is the reason
//! [`nerve_core::vocab::MemoryScope`] is closed at all. `status` refuses one thing more: a caller
//! asking for `potentially_stale` is asking to filter on a value nothing ever wrote, so the refusal
//! names the derived views separately instead of answering *nothing is stale*.
//!
//! # Repository text
//!
//! A note's content, its author label, its claim key, its invalidation reason, an event's note, a
//! citation's path and every field of the subject snapshot are text a human typed or a repository
//! carried. They are attacker-influencable wherever contributions are accepted, and never
//! interpreted. They are carried as JSON **string values**, exactly as `shapes::entity` carries an
//! entity name, and never as an object key, a vocabulary field or a code. Making the bytes safe to
//! render is `respond::to_json_bytes`' job and is tested there.

use serde_json::{json, Value};

use nerve_core::vocab::{MemoryScope, MemoryStatus, MemoryView};
use nerve_store::{MemoryCitationRow, MemoryEventRow, MemoryReport, MemorySubject};

use super::{Answer, ApiError, Context, Resolution};
use crate::request::Target;

/// Largest page of memory records one request may ask for.
///
/// Lower than `/api/history*`'s 500 because a record is not a row: each one carries its whole
/// citation list and its whole audit history, and both are read per record on the page. The window
/// is taken before either is fetched, so the ceiling bounds the work as well as the answer.
pub const MAX_MEMORY_LIMIT: usize = 200;

/// Records returned when no limit is given.
pub const DEFAULT_MEMORY_LIMIT: usize = 50;

/// The one statement about where memory is written, said once and carried everywhere.
pub const BOUNDARY: &str =
    "writing a note, confirming one, replacing one, ending one and attaching a citation all write \
     to this index. This API is read-only and every route on it is a GET, so each of those is a \
     command you run rather than a control on a page. Nothing is pending: a button here would \
     imply an implementation that is deliberately absent, and what makes a confirmation the \
     human's act is the surface it arrived on rather than an identity Nerve checked, because there \
     is none to check";

/// Every `nerve memory` verb, in name order.
///
/// Not only the writers. The readers are here because this list is what a reader who cannot press
/// a button is given instead, and a caller told only about the writers would be told there is no
/// way to ask the question offline.
///
/// `crates/nerve-server/tests/memory.rs` compares this list against the subcommands the CLI
/// actually declares, in both directions, so a verb added without a name here — or a name here with
/// no verb behind it — fails rather than drifting into an instruction that does not work.
pub const BOUNDARY_COMMANDS: [&str; 10] = [
    "nerve memory cite <memory-id> --file <path>",
    "nerve memory confirm <memory-id>",
    "nerve memory events <memory-id>",
    "nerve memory export",
    "nerve memory invalidate <memory-id> --reason <why>",
    "nerve memory list",
    "nerve memory propose --subject <selector> --scope <scope> --content <text>",
    "nerve memory search <query>",
    "nerve memory show <memory-id>",
    "nerve memory supersede <memory-id> --content <text>",
];

/// Why there is no delete verb to name above.
pub const NO_DELETE_VERB: &str =
    "there is no command that removes a memory record or an event, and its absence is the design \
     rather than an omission. `invalidate` records that a note stopped being true and nothing \
     replaced it, and it keeps every event the record ever had; a delete verb is how *history is \
     preserved* stops being true";

/// Why a status and a view are different kinds, said on every answer that carries either.
pub const VIEWS_ARE_DERIVED: &str =
    "`status` is stored and holds one of four values. `potentially_stale`, `conflicted` and \
     `multiple_active` are computed when the record is read, from the anchor state and from what \
     else is active on the same subject, and nothing writes one — so they are reported beside a \
     record and can never be filtered on as though a column held them";

/// Why `superseded_by_memory_id` is reported and never stored.
pub const SUPERSESSION_IS_ONE_DIRECTIONAL: &str =
    "supersession is stored in one direction only, on the successor. The inverse reported here is \
     derived from that single column, because two independently writable copies of one fact can \
     disagree with nothing in the schema to notice";

/// Why an author label is not a caller.
pub const AUTHOR_LABEL_IS_NOT_AN_IDENTITY: &str =
    "`author_label` records what the caller said it was and nothing verified it. Nerve has no \
     accounts, no network and no identity provider, so it is a local label and never an \
     authentication of who wrote the note";

/// Why an empty list with every filter accepted is an absence rather than a finding.
pub const NO_MATCH_STATEMENT: &str =
    "every filter on this request was accepted and matched no record. That is an absence rather \
     than a refusal: an unknown scope or status is a 400 naming the admitted set, so an empty list \
     here means the filters were understood and nothing in this repository answers them";

/// Why an empty list on a repository with no records at all is a different absence.
pub const NOTHING_RECORDED_STATEMENT: &str =
    "no memory record has ever been written in this repository. Nothing is ever discovered here — \
     a record exists because a human wrote one at the command line — so this is an absence rather \
     than a claim that there is nothing worth knowing about this project";

/// What a record's subject snapshot is, and is not.
pub const SUBJECT_IS_A_SNAPSHOT: &str =
    "a record's subject is a copy of what it named when it was written, and never a pointer into \
     the graph. Entities are pruned on every re-index, so a foreign key would either block \
     re-indexing a file somebody wrote a note about or let a routine re-index destroy the note. \
     What the snapshot reaches now is `subject_resolution`, which is reported and never guessed: a \
     moved subject is re-attached only where an identity link says so, never by name resemblance";

/// What a memory record is not.
pub const NOT_EVIDENCE: &str =
    "a memory record is a human sentence about one subject. It is not an assertion, it carries no \
     evidence profile, it is not in the graph, and no path, impact or why query traverses one. It \
     is offered beside the evidence and never mixed into it";

/// A bounded list's truncation, as a fact.
///
/// The same shape `/api/contracts` uses, and for the same reason: `total` is what the store handed
/// over before the window was taken, so `truncated` is a comparison rather than the guess
/// `len() == limit`, which is false whenever a page ends exactly on the boundary — the case a
/// caller most needs to be right.
struct Truncation {
    returned: usize,
    total: usize,
    truncated: bool,
    limit: usize,
    offset: usize,
}

impl Truncation {
    /// Window a complete ordered list, and report the cut as a comparison.
    fn window<T>(rows: Vec<T>, limit: usize, offset: usize) -> (Vec<T>, Truncation) {
        let total = rows.len();
        let kept: Vec<T> = rows.into_iter().skip(offset).take(limit).collect();
        let truncation = Truncation {
            returned: kept.len(),
            total,
            truncated: offset + kept.len() < total,
            limit,
            offset,
        };
        (kept, truncation)
    }

    fn next_offset(&self) -> Option<usize> {
        self.truncated.then_some(self.offset + self.returned)
    }

    fn value(&self) -> Value {
        json!({
            "returned": self.returned,
            "total": self.total,
            "truncated": self.truncated,
            "limit": self.limit,
        })
    }
}

/// Why the single-record route offers no continuation.
///
/// Not a page this endpoint declines to assemble: one record is the whole answer, so there is
/// nothing after it.
pub const ONE_RECORD_IS_COMPLETE: &str =
    "this answer is one record, returned whole with every citation and every event it has. There \
     is no page after it rather than a page this endpoint declines to assemble";

/// What the memory tables answered for one repository, read once per request.
struct Read {
    repo_id: String,
    /// The repository state this index currently describes, which staleness is derived against.
    current_state_id: Option<String>,
    /// Every record here, filters or no filters. A denominator rather than a list.
    records_in_repository: usize,
}

/// Open memory for this repository, or refuse with the reason.
///
/// A missing repository row is **not** an empty memory: it means this index has never recorded
/// which repository it describes, so there is nothing to key a note to. Answering it as "no notes"
/// would report *we never looked* as *we looked and there are none*.
fn read(ctx: &Context<'_>) -> Result<Read, ApiError> {
    let Some(repo_id) = ctx.repo_id else {
        return Err(ApiError::with_detail(
            409,
            "repository_unknown",
            "this index records no repository, so no memory record can be keyed to one",
            json!({ "records": Value::Null }),
        ));
    };
    // Read off `nerve-store` rather than re-derived here: a second answer to "which state does this
    // database describe?" is the duplication Slice 12c-i-a existed to remove, and it is the value
    // every `potentially_stale` verdict was decided against.
    let current_state_id =
        nerve_store::current_repository_state(ctx.conn, repo_id).map_err(ApiError::internal)?;
    let records_in_repository = nerve_store::list_memory(ctx.conn, repo_id)
        .map_err(ApiError::internal)?
        .len();
    Ok(Read {
        repo_id: repo_id.to_string(),
        current_state_id,
        records_in_repository,
    })
}

/// The block every memory answer carries, assembled in **one** place.
///
/// Splitting it across the two endpoints is how one of them ends up answering without its
/// qualification. `requested` is what the caller asked for, echoed verbatim, so an answer can never
/// be read as though a filter had been applied that was not.
fn block(
    read: &Read,
    requested: Value,
    result_kind: &str,
    records_matching: usize,
    truncation: Option<&Truncation>,
    continuation: Value,
) -> Value {
    json!({
        "repository_id": read.repo_id,
        "current_repository_state": read.current_state_id,
        "requested": requested,
        "result_kind": result_kind,
        // Both tallies, because they answer different questions. A repository with records that
        // this question does not match is not a repository with no records, and a client that
        // could only see the second number would report the two absences as one.
        "records_in_repository": read.records_in_repository,
        "records_matching": records_matching,
        "truncation": truncation.map(Truncation::value),
        "continuation": continuation,
        "boundary": boundary(),
        "vocabulary": vocabulary(),
        "limitations": {
            "views_are_derived": VIEWS_ARE_DERIVED,
            "superseded_by_is_derived": SUPERSESSION_IS_ONE_DIRECTIONAL,
            "author_label_is_not_an_identity": AUTHOR_LABEL_IS_NOT_AN_IDENTITY,
            "subject_is_a_snapshot": SUBJECT_IS_A_SNAPSHOT,
            "no_delete_verb": NO_DELETE_VERB,
            "memory_is_not_evidence": NOT_EVIDENCE,
        },
    })
}

/// Where memory is written, and how.
fn boundary() -> Value {
    json!({
        "read_only": true,
        "statement": BOUNDARY,
        "commands": BOUNDARY_COMMANDS,
    })
}

/// The three closed sets a caller may filter on or meet in an answer, named rather than implied.
///
/// Carried on every answer rather than served from a route of its own — unlike
/// `/api/contracts/vocabulary`, which is a hundred glosses — because this is eleven short names and
/// a client building a filter control needs all three of them to build one that cannot be wrong.
/// The prose belongs to each value and travels on the record that carries it.
fn vocabulary() -> Value {
    json!({
        "scopes": scope_vocabulary(),
        "stored_statuses": status_vocabulary(),
        "derived_views": view_vocabulary(),
    })
}

/// Every scope a caller may filter on, for a schema or a refusal.
///
/// Generated from `ALL` rather than typed out, so a value added to the vocabulary is offered by
/// every surface the day it exists.
pub fn scope_vocabulary() -> Vec<&'static str> {
    MemoryScope::ALL
        .iter()
        .map(|scope| scope.as_str())
        .collect()
}

/// Every **stored** status a caller may filter on.
pub fn status_vocabulary() -> Vec<&'static str> {
    MemoryStatus::ALL
        .iter()
        .map(|status| status.as_str())
        .collect()
}

/// Every derived view, which is reported and never filtered on.
pub fn view_vocabulary() -> Vec<&'static str> {
    MemoryView::ALL.iter().map(|view| view.as_str()).collect()
}

/// A continuation the endpoint honours, or the statement that there is none.
fn continuation(offset: Option<usize>, next: Option<usize>, statement: Option<&str>) -> Value {
    match offset {
        Some(offset) => json!({
            "supported": true,
            "offset": offset,
            "next_offset": next,
            "statement": Value::Null,
        }),
        None => json!({
            "supported": false,
            "offset": Value::Null,
            "next_offset": Value::Null,
            "statement": statement,
        }),
    }
}

// ---- one record, rendered ----------------------------------------------------------------------

/// The stored subject snapshot. **Every field is a copy, and none of them is a live pointer.**
fn subject(subject: &MemorySubject) -> Value {
    json!({
        "entity_id": subject.entity_id,
        "kind": subject.kind,
        "name": subject.name,
        "path": subject.path,
        "selector": subject.selector,
    })
}

fn citation(citation: &MemoryCitationRow) -> Value {
    json!({
        "citation_id": citation.citation_id,
        "cited_entity_id": citation.cited_entity_id,
        "cited_kind": citation.cited_kind,
        "cited_name": citation.cited_name,
        "cited_path": citation.cited_path,
        "cited_span": citation.cited_span,
        "cited_at_state": citation.cited_at_state,
        "created_at": citation.created_at,
    })
}

fn event(event: &MemoryEventRow) -> Value {
    json!({
        "event_id": event.event_id,
        "at": event.at,
        "operation": event.operation.as_str(),
        "operation_note": event.operation.note(),
        // Carried off the vocabulary, never inferred from the two statuses being equal: an event
        // that changed no status and an event whose status happened to come back the same would be
        // indistinguishable, and only one of them is a citation.
        "changes_status": event.operation.changes_status(),
        "from_status": event.from_status.map(|status| status.as_str()),
        "to_status": event.to_status.as_str(),
        "note": event.note,
    })
}

/// One record: everything stored about it, and everything true of it right now.
///
/// **The two kinds are never mixed.** `status` is the stored lifecycle and `views` are query-time
/// qualifications, under different keys, each rendered through its own vocabulary's `note` so no
/// surface restates a rule in its own words. `superseded_by_memory_id` is marked derived for the
/// same reason: there is no such column, and a consumer that wrote one back would be storing a
/// second, independently writable copy of a fact the schema keeps in one direction.
///
/// The key set is `nerve memory show --json`'s, field for field. That is not a coincidence and it
/// is not decoration: the two surfaces answer the same question, and
/// `crates/nerve-server/tests/memory.rs` reads the CLI's renderer out of its own source and
/// compares the two sets, so a field added to one and forgotten in the other fails rather than
/// leaving a client reading a different answer depending on where it asked.
fn record(
    report: &MemoryReport,
    citations: &[MemoryCitationRow],
    events: &[MemoryEventRow],
) -> Value {
    let row = &report.row;
    // The scope is stored as text and enumerated by the schema, so a value that somehow is not in
    // the vocabulary still renders — as itself, with no note — rather than failing the whole read.
    let scope = row.scope.parse::<MemoryScope>().ok();
    json!({
        "memory_id": row.memory_id,
        "status": row.status.as_str(),
        "status_note": row.status.note(),
        "views": report.views.iter()
            .map(|view| json!({ "view": view.as_str(), "note": view.note() }))
            .collect::<Vec<_>>(),
        "views_are_derived": true,
        "subject": subject(&row.subject),
        // What the snapshot reaches now, carried off `nerve_store::read_memory` and never
        // re-derived: whether a note still has a subject is one answer, and a second one here
        // would be a second answer.
        "subject_resolution": report.subject.resolution.as_str(),
        "subject_resolution_note": report.subject.resolution.note(),
        "subject_live_entity_ids": report.subject.live_entity_ids,
        "scope": row.scope,
        "scope_note": scope.map(|scope| scope.note()),
        "claim_key": row.claim_key,
        "anchor_state_id": row.anchor_state_id,
        "current_state_id": report.current_state_id,
        "content": row.content,
        "author_label": row.author_label,
        "author_label_is_an_identity": false,
        "created_at": row.created_at,
        "supersedes_memory_id": row.supersedes_memory_id,
        "superseded_by_memory_id": report.superseded_by,
        "superseded_by_is_derived": true,
        "invalidated_at": row.invalidated_at,
        "invalidation_reason": row.invalidation_reason,
        "citations": citations.iter().map(citation).collect::<Vec<_>>(),
        "events": events.iter().map(event).collect::<Vec<_>>(),
    })
}

/// Fetch each record's citations and its whole audit history, for the page only.
///
/// After the window rather than before it, so the two extra reads per record are bounded by the
/// page rather than by the repository.
fn detailed(
    ctx: &Context<'_>,
    repo_id: &str,
    reports: &[&MemoryReport],
) -> Result<Vec<Value>, ApiError> {
    let mut out = Vec::with_capacity(reports.len());
    for report in reports {
        let id = &report.row.memory_id;
        let citations =
            nerve_store::memory_citations(ctx.conn, repo_id, id).map_err(ApiError::internal)?;
        let events =
            nerve_store::memory_events(ctx.conn, repo_id, id).map_err(ApiError::internal)?;
        out.push(record(report, &citations, &events));
    }
    Ok(out)
}

// ---- filters -----------------------------------------------------------------------------------

/// The `scope` filter, refused by name against the closed vocabulary.
fn scope_filter(target: &Target) -> Result<Option<MemoryScope>, ApiError> {
    let Some(raw) = target.get("scope") else {
        return Ok(None);
    };
    match raw.parse::<MemoryScope>() {
        Ok(scope) => Ok(Some(scope)),
        Err(_) => Err(ApiError::with_detail(
            400,
            "unknown_scope",
            format!("unknown scope {raw:?}"),
            json!({
                "parameter": "scope",
                "argument": raw,
                "allowed": scope_vocabulary(),
                // Said rather than implied: an empty list would have been a different claim, and
                // this refusal exists precisely so that claim is never made by accident.
                "nothing_was_looked_up": true,
                "this_is_not_an_empty_list": true,
            }),
        )),
    }
}

/// The `status` filter, refused by name against the **stored** vocabulary.
///
/// A caller naming a derived view is refused with that named separately, because filtering on
/// `potentially_stale` is asking for a value nothing ever wrote — and letting it fall through to an
/// empty list would answer *nothing is stale* to a question about a column that does not exist.
fn status_filter(target: &Target) -> Result<Option<MemoryStatus>, ApiError> {
    let Some(raw) = target.get("status") else {
        return Ok(None);
    };
    match raw.parse::<MemoryStatus>() {
        Ok(status) => Ok(Some(status)),
        Err(_) => Err(ApiError::with_detail(
            400,
            "unknown_status",
            format!("unknown status {raw:?}"),
            json!({
                "parameter": "status",
                "argument": raw,
                "allowed": status_vocabulary(),
                "derived_views": view_vocabulary(),
                "named_a_derived_view": raw.parse::<MemoryView>().is_ok(),
                "views_are_derived": VIEWS_ARE_DERIVED,
                "nothing_was_looked_up": true,
                "this_is_not_an_empty_list": true,
            }),
        )),
    }
}

/// What the caller asked for, echoed verbatim beside the answer.
fn requested(
    scope: Option<MemoryScope>,
    status: Option<MemoryStatus>,
    query: Option<&str>,
    subject: Option<&Resolution>,
    memory_id: Option<&str>,
) -> Value {
    json!({
        "memory_id": memory_id,
        "scope": scope.map(MemoryScope::as_str),
        "status": status.map(MemoryStatus::as_str),
        "query": query,
        "subject": subject.map(|resolution| resolution.entity.entity_id.clone()),
    })
}

// ---- /api/memory -------------------------------------------------------------------------------

/// Every record this question matches, bounded, each with everything true of it now.
///
/// Four filters, and each one narrows through the query in `nerve-store` that owns it: `q` is
/// [`nerve_store::search_memory`]'s literal substring over content and claim key, `subject` is a
/// lookup on the stored snapshot id, and `scope` is its own query. Where two are given the wider
/// query runs and the rest are applied as **equality comparisons over the rows it returned** — a
/// comparison is rendering, whereas re-implementing the search predicate here would be a second
/// answer to *"which records say this?"*.
///
/// The order is `nerve-store`'s throughout: by `memory_id`, ascending, retired records included.
/// A read that hid a superseded record would make *"what did we once believe and no longer do"*
/// unanswerable at exactly the moment it becomes the question.
pub fn list(ctx: &Context<'_>, target: &Target) -> Answer {
    let read = read(ctx)?;
    let scope = scope_filter(target)?;
    let status = status_filter(target)?;
    let query = target.get("q").map(str::to_string);
    // Resolved through the one resolver every surface uses, so an ambiguous selector is a refusal
    // carrying its candidates rather than a record chosen on the caller's behalf.
    let subject = match target.get("subject") {
        Some(_) => Some(super::resolve(ctx, target, "subject")?),
        None => None,
    };
    let limit = target
        .bounded("limit", DEFAULT_MEMORY_LIMIT, MAX_MEMORY_LIMIT)
        .map_err(ApiError::bad_request)?;
    let offset = target
        .bounded_from_zero("offset", 0, usize::MAX)
        .map_err(ApiError::bad_request)?;

    let mut reports = match (&query, &subject, scope) {
        (Some(query), _, _) => nerve_store::search_memory(ctx.conn, &read.repo_id, query),
        (None, Some(subject), _) => {
            nerve_store::read_memory_for_subject(ctx.conn, &read.repo_id, &subject.entity.entity_id)
        }
        (None, None, Some(scope)) => {
            nerve_store::read_memory_in_scope(ctx.conn, &read.repo_id, scope.as_str())
        }
        (None, None, None) => nerve_store::read_memory_all(ctx.conn, &read.repo_id),
    }
    .map_err(ApiError::internal)?;

    if let Some(subject) = &subject {
        reports.retain(|report| report.row.subject.entity_id == subject.entity.entity_id);
    }
    if let Some(scope) = scope {
        reports.retain(|report| report.row.scope == scope.as_str());
    }
    if let Some(status) = status {
        reports.retain(|report| report.row.status == status);
    }

    let matching = reports.len();
    let (page, truncation) = Truncation::window(reports.iter().collect::<Vec<_>>(), limit, offset);
    let records = detailed(ctx, &read.repo_id, &page)?;

    // Three answers, not two. "Nothing has ever been written here" and "records exist and this
    // question matches none of them" have different next steps, and a client that could not tell
    // them apart would report the first as the second.
    let (kind, statement) = match (matching, read.records_in_repository) {
        (0, 0) => ("no_memory_recorded", Some(NOTHING_RECORDED_STATEMENT)),
        (0, _) => ("no_memory_matches", Some(NO_MATCH_STATEMENT)),
        _ => ("memory_records", None),
    };

    let mut value = block(
        &read,
        requested(scope, status, query.as_deref(), subject.as_ref(), None),
        kind,
        matching,
        Some(&truncation),
        continuation(Some(offset), truncation.next_offset(), None),
    );
    value["absence_statement"] = json!(statement);
    value["records"] = Value::Array(records);
    if let Some(subject) = &subject {
        super::note_selectors(&mut value, &[("subject", subject)]);
    }
    Ok(value)
}

// ---- /api/memory/record ------------------------------------------------------------------------

/// One record in full, with every citation and every event it has.
///
/// **A record that is not here is a refusal, never an empty one.** "There is no such note" and
/// "the note says nothing" are different answers, and only a `404` says the first.
///
/// The id is a query parameter rather than a path segment, which is a deliberate departure from the
/// `/api/memory/<id>` shape row 14's plan sketches. Every route this server serves is an exact
/// match against a fixed table, and `tests/api.rs` compares that table against the dispatch in both
/// directions by reading the source; a path parameter cannot be written as a table entry, so it
/// would have to be matched by a prefix arm the parity test cannot see — an unadvertised route,
/// which is the specific hole that test exists to close. A `memory_id` is also caller-supplied text
/// that may hold any character, including `/`, and a path segment could not carry one.
pub fn one(ctx: &Context<'_>, target: &Target) -> Answer {
    let read = read(ctx)?;
    let memory_id = target
        .get("memory_id")
        .ok_or_else(|| ApiError::bad_request("memory_id is required"))?;

    let Some(report) =
        nerve_store::read_memory(ctx.conn, &read.repo_id, memory_id).map_err(ApiError::internal)?
    else {
        return Err(ApiError::with_detail(
            404,
            "memory_record_not_found",
            format!("{memory_id:?} is not a memory record in this repository"),
            json!({
                "memory_id": memory_id,
                "records_in_repository": read.records_in_repository,
                "this_is_not_an_empty_record": true,
                "every_record_is_listed": "/api/memory lists every record, retired ones included",
            }),
        ));
    };

    let records = detailed(ctx, &read.repo_id, &[&report])?;
    let mut value = block(
        &read,
        requested(None, None, None, None, Some(memory_id)),
        "memory_record",
        1,
        None,
        continuation(None, None, Some(ONE_RECORD_IS_COMPLETE)),
    );
    value["absence_statement"] = Value::Null;
    value["records"] = Value::Array(records);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceilings_are_the_documented_contract() {
        assert_eq!(MAX_MEMORY_LIMIT, 200);
        assert_eq!(DEFAULT_MEMORY_LIMIT, 50);
        assert_eq!(
            DEFAULT_MEMORY_LIMIT,
            DEFAULT_MEMORY_LIMIT.min(MAX_MEMORY_LIMIT),
            "the default page is larger than the ceiling"
        );
    }

    /// The case `returned == limit` gets wrong in both directions.
    #[test]
    fn truncation_is_a_comparison_against_the_whole_list_rather_than_the_page_length() {
        let (page, exact) = Truncation::window(vec![1, 2, 3], 3, 0);
        assert_eq!(page.len(), 3);
        assert_eq!(exact.value()["truncated"], false);
        assert_eq!(exact.next_offset(), None);

        let (page, cut) = Truncation::window(vec![1, 2, 3, 4, 5], 2, 0);
        assert_eq!(page, vec![1, 2]);
        assert_eq!(cut.value()["truncated"], true);
        assert_eq!(cut.next_offset(), Some(2));

        // A window past the end is an ordinary answer, not a cut.
        let (page, past) = Truncation::window(vec![1, 2, 3], 5, 10);
        assert!(page.is_empty());
        assert_eq!(past.value()["truncated"], false);
        assert_eq!(past.value()["total"], 3);
    }

    /// A misspelling is refused with the admitted set, and a legal value is not.
    #[test]
    fn an_unknown_scope_or_status_is_refused_by_name_rather_than_filtered_to_nothing() {
        let target = Target::parse("/api/memory?scope=opertions").unwrap();
        let error = scope_filter(&target).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(error.code, "unknown_scope");
        assert_eq!(error.detail["this_is_not_an_empty_list"], true);
        assert_eq!(
            error.detail["allowed"],
            json!(["implementation", "interface", "operations", "process"])
        );
        assert_eq!(
            scope_filter(&Target::parse("/api/memory?scope=process").unwrap()).unwrap(),
            Some(MemoryScope::Process)
        );
        assert_eq!(
            scope_filter(&Target::parse("/api/memory").unwrap()).unwrap(),
            None
        );

        // A derived view is refused *and named as one*, which is the case an empty list would have
        // answered `nothing is stale` to.
        let target = Target::parse("/api/memory?status=potentially_stale").unwrap();
        let error = status_filter(&target).unwrap_err();
        assert_eq!(error.code, "unknown_status");
        assert_eq!(error.detail["named_a_derived_view"], true);
        assert_eq!(
            error.detail["derived_views"],
            json!(["potentially_stale", "conflicted", "multiple_active"])
        );
        let target = Target::parse("/api/memory?status=banana").unwrap();
        assert_eq!(
            status_filter(&target).unwrap_err().detail["named_a_derived_view"],
            false
        );
        assert_eq!(
            status_filter(&Target::parse("/api/memory?status=active").unwrap()).unwrap(),
            Some(MemoryStatus::Active)
        );
    }

    /// Every command the boundary names is a `nerve memory` verb, and none of them is a route.
    #[test]
    fn the_boundary_names_commands_rather_than_offering_a_control() {
        assert_eq!(boundary()["read_only"], true);
        let mut sorted = BOUNDARY_COMMANDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, BOUNDARY_COMMANDS.to_vec(), "keep the list ordered");
        for command in BOUNDARY_COMMANDS {
            assert!(command.starts_with("nerve memory "), "{command}");
            assert!(!command.contains("delete"), "{command}");
        }
        assert!(BOUNDARY.contains("read-only"));
        assert!(NO_DELETE_VERB.contains("invalidate"));
    }

    /// The three closed sets are the vocabularies themselves rather than a list typed here.
    #[test]
    fn the_vocabulary_block_is_generated_from_the_closed_sets() {
        let vocabulary = vocabulary();
        assert_eq!(
            vocabulary["scopes"].as_array().unwrap().len(),
            MemoryScope::ALL.len()
        );
        assert_eq!(
            vocabulary["stored_statuses"].as_array().unwrap().len(),
            MemoryStatus::ALL.len()
        );
        assert_eq!(
            vocabulary["derived_views"].as_array().unwrap().len(),
            MemoryView::ALL.len()
        );
        // The two kinds never share a name, which is what makes refusing a view as a status a
        // statement about the vocabulary rather than about a spelling.
        for view in MemoryView::ALL {
            assert!(
                view.as_str().parse::<MemoryStatus>().is_err(),
                "{view} is both stored and derived"
            );
        }
    }
}
