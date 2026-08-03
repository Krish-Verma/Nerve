//! The blast-radius question: *if I change this symbol, what else might break?*
//!
//! A reverse dependency closure: start at one symbol, walk assertions **backwards** — from
//! target to source — and report everything that transitively depends on it, with the evidence
//! for the edge that reached it.
//!
//! Like [`crate::gaps`], the difficulty is not the traversal. It is that the obvious answer is
//! read as a stronger claim than the evidence supports.
//!
//! # The unresolved account is part of the answer, not a footnote
//!
//! Slice 2a measured **38.1% of call sites on the resolution corpus as honestly `Unresolved`**.
//! Any method call on a typed receiver — `shape.area()` — is unresolvable without type
//! inference, and Nerve has none. So a report that says
//!
//! ```text
//! 3 entities depend on parseConfig
//! ```
//!
//! will be read as *"only three things use this, it is safe to change"*, and if a third of the
//! repository's reference sites resolved to nothing that reading is unsupported. The command
//! would then have talked someone into a breaking change, which is worse than not answering.
//!
//! [`UnresolvedAccount`] is therefore a **field on the report, never an `Option`**, always
//! rendered and always serialized — including when every one of its counts is zero. Zero is a
//! measurement here and it is worth stating: it means nothing is hidden from this particular
//! answer by a failed resolution. The precedent is Slice 7a's `CoverageEvidence::Absent`, one
//! silence made into a value; this is a different silence given the same treatment.
//!
//! **What may be claimed.** *"N reference sites in this repository resolved to nothing; any of
//! them could reach this symbol and this answer cannot rule them out."*
//!
//! **What may not.** Matching an unresolved site's *name* against the subject's name and
//! presenting the result as a probable caller. That is identity by fuzzy name matching, which
//! this project forbids and which ADR-0002's tuples exist to prevent. Nothing in this module
//! compares a name to a name.
//!
//! # What the account counts, and why that denominator
//!
//! - **Unit: observations, not assertions.** An observation *is* a site — one `parse()` in one
//!   place. Two calls to the same unresolved name from one function collapse into one assertion
//!   but stay two observations, and "38.1% of call sites" is a per-site figure. Counting
//!   assertions would silently under-report exactly where a file leans hardest on one
//!   unresolvable name. [`UnresolvedAccount::assertions`] and
//!   [`UnresolvedAccount::targets`] are carried alongside because each is a distinct exact fact
//!   and all three fall out of one query.
//! - **Scope: the whole repository, restricted to the relations this query walked.**
//!   Repository-wide because a hidden edge can attach anywhere — to the subject, or to a symbol
//!   the closure already reached, or to one it has not — and narrowing that set without name
//!   matching or type inference is not possible. Relation-restricted because counting
//!   `SUPERSEDES` markers as potential hidden callers, when the walk never follows `SUPERSEDES`,
//!   would be an exact number that answers a different question.
//! - **Split by [`nerve_core::vocab::UnresolvedCategory`].** A broken Markdown link and an
//!   unresolvable method call are both "resolved to nothing" and are not the same warning. The
//!   split is reported so neither is read as the other.
//!
//! # Which relations are followed, and which are refused
//!
//! The default is `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS` — the symbol-level dependency
//! relations Slice 2a resolves and measures — plus `SERVED_BY` since Slice 10a.
//! [`DEFAULT_RELATIONS`] is the whole of it, and both the exclusions and the one inclusion that
//! looks like an exclusion are deliberate:
//!
//! - **`CONTAINS` and `DEFINES`** would walk from a function to its module, its file, its
//!   directory and the repository itself. Every symbol would "impact" the repository — true, and
//!   useless, and it would bury the four edges carrying the actual answer under structural noise.
//! - **`IMPORTS`** is the right closure for incremental invalidation ([`crate::importers_of`]),
//!   where a false positive costs a reparse. It is the wrong default for a person: if module A
//!   imports module B and I change a function in B that A never calls, A is not affected, and
//!   reporting it trains the reader to ignore the output. It remains available explicitly.
//! - **`COVERS`** would report "coverage run R covered this symbol", which is a freshness
//!   consequence of a change rather than a dependency on the symbol — and would put a
//!   `CoverageRun`, which is neither a symbol nor code, in a list of things that depend on your
//!   function.
//! - **`SERVED_BY` is included**, and the obvious objection is the `COVERS` one restated: an
//!   `Endpoint` is not a symbol and not code either. The distinction is real. A `CoverageRun` is
//!   an artifact **about** the code — a report of a past execution — and changing a symbol does
//!   not break a report, it makes it *stale*, which is a freshness matter and is exactly what the
//!   `COVERS` exclusion note says. An `Endpoint` is a declaration **in** the code: it exists
//!   because a decorator in a source file declares it, it is withdrawn when that file changes, and
//!   changing its handler changes what the endpoint does. It genuinely depends on the symbol.
//!   Excluding it would reproduce the defect Slice 10 exists to fix — a live HTTP handler and dead
//!   code producing a byte-identical *"nothing in the index depends on this"*.
//!
//!   What appears in the closure is still a **declaration**, never a proof of reachability. See
//!   [`nerve_core::vocab::Relation::ServedBy`] for the list of things a registration does not
//!   prove, which the CLI restates when an endpoint is in the answer.
//!
//! This is **not** `nerve affected`. That command is refused, not deferred (ADR-0008 §A.2: LCOV
//! carries no per-test attribution). If a test file appears in an impact set it is because code
//! depends on code; nothing here is test attribution and nothing here may be described as such.
//!
//! # Bounds
//!
//! Depth-bounded, and count-bounded on the rows only. The closure is computed in full within the
//! depth bound, so [`ImpactTotals`] and [`ImpactReport::results_total`] stay exact whatever
//! [`ImpactQuery::limit`] cuts, and [`ImpactReport::truncated`] says when it cut something. The
//! walk keeps a global visited set, so a cycle terminates and every entity appears exactly once —
//! unlike [`crate::graph::find_paths`], which enumerates alternative routes and must therefore
//! keep paths simple instead.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;

use nerve_core::vocab::{Relation, UnresolvedCategory};

use crate::error::Result;
use crate::freshness::{FileProber, Freshness, FreshnessCache};
use crate::graph::{
    adjacency_sql, assertion_state, load_entity, read_edges, relation_clause,
    representative_observation_row, Direction, EdgeDirection, PathQuery,
};
use crate::select::EntityRef;

/// The relations a reverse impact closure follows unless the caller names others.
///
/// See the module documentation for why `CONTAINS`, `DEFINES`, `IMPORTS` and `COVERS` are not
/// here, and why `SERVED_BY` is. Changing this array changes what `nerve impact` means.
pub const DEFAULT_RELATIONS: [Relation; 5] = [
    Relation::Calls,
    Relation::References,
    Relation::Extends,
    Relation::Implements,
    // Appended in Slice 10a. An endpoint depends on the symbol that implements it, and without
    // this a live route handler is indistinguishable from dead code.
    Relation::ServedBy,
];

/// What to ask, and how much of the answer to return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactQuery {
    /// How many hops from the subject to walk. Matches `nerve path`'s convention.
    pub max_depth: usize,
    /// Largest number of rows returned. The tallies are always exact regardless.
    pub limit: usize,
    /// Relations to follow, from the closed vocabulary.
    ///
    /// Empty means [`DEFAULT_RELATIONS`] — deliberately **not** "every relation", which is what
    /// empty means to [`PathQuery`]. An impact closure that quietly fell back to every relation
    /// on an empty list would follow `CONTAINS` and answer that every symbol impacts the
    /// repository. See [`ImpactQuery::effective_relations`].
    pub relations: Vec<Relation>,
}

impl Default for ImpactQuery {
    fn default() -> Self {
        ImpactQuery {
            max_depth: 6,
            limit: 50,
            relations: DEFAULT_RELATIONS.to_vec(),
        }
    }
}

impl ImpactQuery {
    /// The relation set actually walked: the caller's, or [`DEFAULT_RELATIONS`] when empty.
    pub fn effective_relations(&self) -> Vec<Relation> {
        if self.relations.is_empty() {
            DEFAULT_RELATIONS.to_vec()
        } else {
            self.relations.clone()
        }
    }
}

/// One entity that depends on the subject, and the edge that reached it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactRow {
    /// The dependent entity.
    pub entity: EntityRef,
    /// Hops from the subject. Never `0`: a symbol is not its own impact.
    pub depth: usize,
    /// Relation of the edge that **first** reached this entity.
    ///
    /// First, not only. Breadth-first means this is an edge on a shortest route from the
    /// subject; other edges may also connect it, and `nerve why` reports those.
    pub relation: String,
    /// Which side of that edge this row's entity sits on.
    ///
    /// Always [`EdgeDirection::Outgoing`], read against [`ImpactRow::entity`] exactly as
    /// [`crate::AssertionEvidence::direction`] is read against its subject: the dependent
    /// asserts the edge, the subject receives it. It is invariant because a reverse closure can
    /// only ever admit an entity through an edge that entity itself asserts, and it is reported
    /// anyway so that a row read on its own cannot be mistaken for a forward dependency.
    pub direction: EdgeDirection,
    /// The closure member this edge points at — already reached at `depth - 1`, or the subject.
    pub reached_entity_id: String,
    /// Assertion backing the edge.
    pub assertion_id: String,
    /// Derived `assertion_state.status`.
    pub status: Option<String>,
    /// Derived `assertion_state.strongest_source_type`.
    pub strongest_source_type: Option<String>,
    /// Derived `assertion_state.observation_count`.
    pub observation_count: i64,
    /// Derived `assertion_state.is_unresolved`.
    pub is_unresolved: bool,
    /// Representative observation path.
    pub file_path: Option<String>,
    /// Representative observation line.
    pub start_line: Option<i64>,
    /// Whether that observation still describes the file. Computed at query time, never stored.
    pub evidence_freshness: Option<Freshness>,
}

impl ImpactRow {
    /// `file:line` of the representative observation, or `-`.
    pub fn location(&self) -> String {
        match (&self.file_path, self.start_line) {
            (Some(file), Some(line)) => format!("{file}:{line}"),
            (Some(file), None) => file.clone(),
            _ => "-".to_string(),
        }
    }
}

/// The exact tally over the whole closure, before any row cap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImpactTotals {
    /// Distinct entities that depend on the subject. Excludes the subject itself.
    pub entities: usize,
    /// Entities per hop distance.
    pub by_depth: BTreeMap<usize, usize>,
    /// Entities per relation of the edge that reached them.
    pub by_relation: BTreeMap<String, usize>,
    /// Entities per entity kind.
    ///
    /// A `file`, `directory` or `repository` here would mean containment leaked into the walk.
    pub by_kind: BTreeMap<String, usize>,
    /// Entities reached through evidence that no longer matches the file it was taken from.
    pub stale: usize,
}

/// What this answer cannot see, counted exactly.
///
/// Never an `Option` and never omitted. See the module documentation for the unit, the scope and
/// the reason zero is still worth printing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnresolvedAccount {
    /// Reference **sites** — observations — on edges whose target resolved to nothing.
    pub sites: usize,
    /// Assertions those sites support.
    pub assertions: usize,
    /// Distinct `Unresolved` entities those assertions name.
    pub targets: usize,
    /// Sites per [`UnresolvedCategory`], so a broken document link is not read as a lost call.
    ///
    /// A category the closed vocabulary does not know is counted under
    /// [`UNCATEGORISED`] rather than dropped: the database is a file on disk Nerve does not own
    /// exclusively, and a site that cannot be classified is still a site.
    pub by_category: BTreeMap<String, usize>,
}

/// Bucket for an unresolved entity whose recorded category is absent or unrecognised.
pub const UNCATEGORISED: &str = "uncategorised";

impl UnresolvedAccount {
    /// Whether every reference site in scope resolved.
    pub fn is_empty(&self) -> bool {
        self.sites == 0
    }
}

/// Everything `nerve impact` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactReport {
    /// The symbol the question was asked about.
    pub subject: EntityRef,
    /// The relation set actually walked, after the empty-means-default rule.
    pub relations: Vec<Relation>,
    /// Depth bound used.
    pub max_depth: usize,
    /// The exact tally over the whole closure.
    pub totals: ImpactTotals,
    /// The rows, capped at [`ImpactQuery::limit`].
    pub results: Vec<ImpactRow>,
    /// How many rows the closure held before the cap.
    pub results_total: usize,
    /// Whether the cap cut anything off.
    pub truncated: bool,
    /// The cap that was applied.
    pub limit: usize,
    /// What this answer cannot see. Always present, including when it is all zeroes.
    pub unresolved: UnresolvedAccount,
    /// Distinct files re-hashed to compute freshness.
    pub files_probed: usize,
}

/// How one entity first entered the closure.
struct Reach {
    depth: usize,
    relation: String,
    assertion_id: String,
    reached_entity_id: String,
}

/// Read an unresolved entity's category out of its `meta` blob.
///
/// The value is parsed against the closed [`UnresolvedCategory`] vocabulary, so a category this
/// build does not know is bucketed as [`UNCATEGORISED`] rather than tallied under a name nobody
/// can interpret. A blob that does not parse at all lands in the same bucket instead of panicking
/// the query: `meta` is extractor-written but lives in a file on disk Nerve does not own.
fn read_category(meta: Option<&str>) -> String {
    meta.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .as_ref()
        .and_then(|value| value.get("category"))
        .and_then(serde_json::Value::as_str)
        .and_then(|name| name.parse::<UnresolvedCategory>().ok())
        .map(|category| category.as_str().to_string())
        .unwrap_or_else(|| UNCATEGORISED.to_string())
}

/// Count every reference site in the repository that resolved to nothing, over `relations`.
///
/// One row per assertion, so the site count is a `COUNT` over `observation` and the assertion and
/// target counts fall out of the same pass. The relation list is inlined from the closed
/// vocabulary through the same helper the traversal uses; no caller text reaches the statement.
fn unresolved_account(conn: &Connection, relations: &[Relation]) -> Result<UnresolvedAccount> {
    let relation_filter = relation_clause(relations);
    let sql = format!(
        "SELECT a.target_entity_id, target.meta, COUNT(o.observation_id)
           FROM assertion a
           INNER JOIN assertion_state s ON s.assertion_id = a.assertion_id
           INNER JOIN entity target ON target.entity_id = a.target_entity_id
           INNER JOIN observation o ON o.assertion_id = a.assertion_id
          WHERE s.is_unresolved = 1{relation_filter}
          GROUP BY a.assertion_id, a.target_entity_id, target.meta
          ORDER BY a.assertion_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut account = UnresolvedAccount::default();
    let mut targets: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        let (target_entity_id, meta, sites) = row?;
        let sites = usize::try_from(sites).unwrap_or(0);
        account.sites += sites;
        account.assertions += 1;
        targets.insert(target_entity_id);
        *account
            .by_category
            .entry(read_category(meta.as_deref()))
            .or_default() += sites;
    }
    account.targets = targets.len();
    Ok(account)
}

/// Answer *"if I change this, what else might break?"* — with what the answer cannot see.
///
/// Breadth-first over the reverse adjacency built by `graph::adjacency_sql`, which is
/// served by `idx_assertion_target(target_entity_id, relation)`. A global visited set makes the
/// walk terminate on a cycle and admits every entity exactly once, at the shortest distance from
/// which it was reached.
///
/// The closure is expanded in full within [`ImpactQuery::max_depth`] before
/// [`ImpactQuery::limit`] is applied, so the tallies describe the whole answer and not the page.
/// Freshness is computed by re-reading the repository through `prober`, which enforces the
/// repository's path rules on every path the database supplies.
pub fn impact(
    conn: &Connection,
    subject_id: &str,
    query: &ImpactQuery,
    prober: &dyn FileProber,
) -> Result<ImpactReport> {
    let relations = query.effective_relations();
    let mut entities: BTreeMap<String, EntityRef> = BTreeMap::new();
    let subject = load_entity(conn, &mut entities, subject_id)?;

    // The same statement builder `nerve path` uses, anchored on the target column. `limit` and
    // `direction` play no part in the SQL; they are set to the values that read least
    // surprisingly if this struct is ever printed in a debug message.
    let walk = PathQuery {
        max_depth: query.max_depth,
        limit: query.limit,
        direction: Direction::Forward,
        relations: relations.clone(),
        resolved_only: false,
    };
    let mut reverse = conn.prepare(&adjacency_sql(&walk, true))?;

    // The subject is seeded into the visited set and never reported: it is the question, not an
    // answer, and seeding it is also what makes a cycle back onto the subject terminate.
    let mut reached: BTreeMap<String, Reach> = BTreeMap::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(subject_id.to_string());
    let mut frontier: Vec<String> = vec![subject_id.to_string()];

    for depth in 1..=query.max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<String> = Vec::new();
        for anchor in &frontier {
            for edge in read_edges(&mut reverse, anchor, true)? {
                // First reach wins, and breadth-first means the first reach is the shortest.
                if !visited.insert(edge.neighbour.clone()) {
                    continue;
                }
                reached.insert(
                    edge.neighbour.clone(),
                    Reach {
                        depth,
                        relation: edge.relation,
                        assertion_id: edge.assertion_id,
                        reached_entity_id: anchor.clone(),
                    },
                );
                next.push(edge.neighbour);
            }
        }
        frontier = next;
    }

    let mut cache = FreshnessCache::new(prober);
    let mut totals = ImpactTotals {
        entities: reached.len(),
        ..ImpactTotals::default()
    };
    let mut rows: Vec<ImpactRow> = Vec::with_capacity(reached.len());
    for (entity_id, reach) in &reached {
        let state = assertion_state(conn, &reach.assertion_id)?;
        let observation = representative_observation_row(conn, &reach.assertion_id)?;
        let evidence_freshness = observation
            .as_ref()
            .map(|found| cache.evaluate(&found.file_path, &found.content_hash));
        let entity = load_entity(conn, &mut entities, entity_id)?;

        *totals.by_depth.entry(reach.depth).or_default() += 1;
        *totals
            .by_relation
            .entry(reach.relation.clone())
            .or_default() += 1;
        *totals.by_kind.entry(entity.kind.clone()).or_default() += 1;
        if evidence_freshness.is_some_and(|freshness| freshness != Freshness::Fresh) {
            totals.stale += 1;
        }

        rows.push(ImpactRow {
            entity,
            depth: reach.depth,
            relation: reach.relation.clone(),
            direction: EdgeDirection::Outgoing,
            reached_entity_id: reach.reached_entity_id.clone(),
            assertion_id: reach.assertion_id.clone(),
            status: state.as_ref().map(|state| state.status.clone()),
            strongest_source_type: state
                .as_ref()
                .map(|state| state.strongest_source_type.clone()),
            observation_count: state.as_ref().map_or(0, |state| state.observation_count),
            is_unresolved: state.as_ref().is_some_and(|state| state.is_unresolved),
            file_path: observation.as_ref().map(|found| found.file_path.clone()),
            start_line: observation.as_ref().map(|found| found.start_line),
            evidence_freshness,
        });
    }

    // Nearest first, then by kind and name. Deterministic regardless of the order the walk
    // happened to discover things, which the `BTreeMap` above has already fixed.
    rows.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.entity.kind.cmp(&b.entity.kind))
            .then_with(|| a.entity.qualified_name().cmp(&b.entity.qualified_name()))
            .then_with(|| a.entity.entity_id.cmp(&b.entity.entity_id))
    });

    let results_total = rows.len();
    let truncated = results_total > query.limit;
    rows.truncate(query.limit);

    Ok(ImpactReport {
        subject,
        unresolved: unresolved_account(conn, &relations)?,
        relations,
        max_depth: query.max_depth,
        totals,
        results: rows,
        results_total,
        truncated,
        limit: query.limit,
        files_probed: cache.files_probed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five the walk follows, and the six it must not.
    ///
    /// This is the difference between an answer and a truism. With `CONTAINS` in the set every
    /// symbol reaches its module, its file, its directory and the repository, so every symbol
    /// "impacts" the whole repository; with `COVERS` in it a `CoverageRun` — not a symbol, not
    /// code — turns up in a list of things that depend on your function.
    ///
    /// `SERVED_BY` is in the set for the reason the module documentation gives: an endpoint is a
    /// declaration *in* the code rather than a report *about* it, so it genuinely depends on its
    /// handler, and leaving it out makes a live route handler answer exactly as dead code does.
    /// The two membership decisions are asserted together, because they are the same argument
    /// pointing in opposite directions.
    #[test]
    fn the_default_relation_set_is_the_dependency_relations_plus_served_by() {
        assert_eq!(
            DEFAULT_RELATIONS.to_vec(),
            vec![
                Relation::Calls,
                Relation::References,
                Relation::Extends,
                Relation::Implements,
                Relation::ServedBy,
            ]
        );
        for refused in [
            Relation::Contains,
            Relation::Defines,
            Relation::Imports,
            Relation::Covers,
            Relation::Exports,
            Relation::Supersedes,
        ] {
            assert!(
                !DEFAULT_RELATIONS.contains(&refused),
                "{refused} must not be a default impact relation"
            );
        }
        assert!(
            DEFAULT_RELATIONS.contains(&Relation::ServedBy),
            "without SERVED_BY a live route handler and dead code give the same answer, which is \
             the measured defect Slice 10 exists to fix"
        );
        // Every member of the closed vocabulary is accounted for, so a relation added later
        // cannot be silently absent from both lists.
        assert_eq!(
            DEFAULT_RELATIONS.len() + 6,
            Relation::ALL.len(),
            "a relation was added to the vocabulary without a decision about `nerve impact`"
        );
    }

    /// An empty list means the default four, never "every relation".
    ///
    /// [`PathQuery`] reads an empty list as "no filter", and an impact closure that inherited
    /// that reading would follow `CONTAINS`. The two conventions differ on purpose, so the
    /// difference is pinned here.
    #[test]
    fn an_empty_relation_list_means_the_default_set_not_every_relation() {
        let query = ImpactQuery {
            relations: Vec::new(),
            ..ImpactQuery::default()
        };
        assert_eq!(query.effective_relations(), DEFAULT_RELATIONS.to_vec());
        assert!(!query.effective_relations().contains(&Relation::Contains));

        let explicit = ImpactQuery {
            relations: vec![Relation::Imports],
            ..ImpactQuery::default()
        };
        assert_eq!(explicit.effective_relations(), vec![Relation::Imports]);
    }

    #[test]
    fn defaults_match_the_documented_command_line() {
        let query = ImpactQuery::default();
        assert_eq!(query.max_depth, 6, "the same default as `nerve path`");
        assert_eq!(query.limit, 50);
        assert_eq!(query.relations, DEFAULT_RELATIONS.to_vec());
    }

    /// A category is read against the closed vocabulary, and nothing unreadable is dropped.
    #[test]
    fn an_unreadable_category_is_bucketed_rather_than_lost() {
        assert_eq!(read_category(Some(r#"{"category":"value"}"#)), "value");
        assert_eq!(
            read_category(Some(r#"{"category":"document_link"}"#)),
            "document_link"
        );
        assert_eq!(
            read_category(Some(r#"{"category":"novel"}"#)),
            UNCATEGORISED
        );
        assert_eq!(read_category(Some("not json")), UNCATEGORISED);
        assert_eq!(read_category(Some("[1,2]")), UNCATEGORISED);
        assert_eq!(read_category(None), UNCATEGORISED);
        for category in UnresolvedCategory::ALL {
            let meta = format!(r#"{{"category":"{}"}}"#, category.as_str());
            assert_eq!(read_category(Some(&meta)), category.as_str());
        }
    }

    /// Zero is a measurement, and `is_empty` must not be read as "no answer".
    #[test]
    fn an_account_with_no_sites_is_still_an_account() {
        let account = UnresolvedAccount::default();
        assert!(account.is_empty());
        assert_eq!(account.sites, 0);
        assert_eq!(account.assertions, 0);
        assert_eq!(account.targets, 0);
        assert!(account.by_category.is_empty());
    }

    /// The account's statement is scoped to the relations the walk followed.
    #[test]
    fn the_unresolved_filter_names_only_closed_vocabulary_relations() {
        let clause = relation_clause(&DEFAULT_RELATIONS);
        assert_eq!(
            clause,
            " AND a.relation IN ('CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS', 'SERVED_BY')"
        );
        assert!(!clause.contains("CONTAINS"));
    }
}
