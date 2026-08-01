//! Graph reads: bounded path traversal and evidence assembly.
//!
//! Everything `nerve path` and `nerve why` do lives here rather than in the CLI, so that the
//! Slice 4 server, the Slice 7 CI command and the Slice 8 MCP tools answer the same questions
//! the same way (ARCHITECTURE.md invariant 3).
//!
//! Two properties are load-bearing:
//!
//! - **Determinism.** Every statement carries an explicit `ORDER BY`, every intermediate
//!   collection is a `BTreeMap`/`BTreeSet`, and adjacency is sorted before it is walked.
//!   Identical inputs produce identical output, including the order of the paths returned.
//! - **Boundedness.** Enumerating simple paths is exponential in the general case, so the
//!   walk carries an explicit budget. When the budget stops the search the report says so:
//!   "no path" and "I gave up" are different answers and must not be rendered as the same one.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rusqlite::{params, Connection};

use nerve_core::vocab::Relation;

use crate::error::Result;
use crate::freshness::{FileProber, Freshness, FreshnessCache};
use crate::select::{EntityRef, ENTITY_COLUMNS, ENTITY_FROM};

/// Largest number of partial paths the walk will expand before giving up.
pub const MAX_EXPANSIONS: usize = 100_000;

/// Largest number of partial paths held in the frontier at once.
pub const MAX_FRONTIER: usize = 200_000;

/// How edges are followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Follow assertions from source to target only.
    Forward,
    /// Treat assertions as undirected.
    Any,
}

impl Direction {
    /// Canonical name used in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Any => "any",
        }
    }
}

/// Bounds and filters for a path search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathQuery {
    /// Maximum number of hops.
    pub max_depth: usize,
    /// Maximum number of distinct paths to return.
    pub limit: usize,
    /// Edge direction policy.
    pub direction: Direction,
    /// Relations to follow. Empty means every relation.
    pub relations: Vec<Relation>,
    /// Exclude edges whose `assertion_state.is_unresolved` is set.
    pub resolved_only: bool,
}

impl Default for PathQuery {
    fn default() -> Self {
        PathQuery {
            max_depth: 6,
            limit: 3,
            direction: Direction::Forward,
            relations: Vec::new(),
            resolved_only: false,
        }
    }
}

/// One traversed edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathHop {
    /// Entity the hop starts at, in traversal order.
    pub from: EntityRef,
    /// Entity the hop arrives at, in traversal order.
    pub to: EntityRef,
    /// Relation name.
    pub relation: String,
    /// Assertion backing the hop.
    pub assertion_id: String,
    /// True when `Direction::Any` walked the assertion against its recorded direction.
    pub traversed_backwards: bool,
    /// Derived `assertion_state.is_unresolved`.
    pub is_unresolved: bool,
    /// Derived `assertion_state.status`.
    pub status: Option<String>,
    /// Derived `assertion_state.strongest_source_type`.
    pub strongest_source_type: Option<String>,
    /// Derived `assertion_state.observation_count`.
    pub observation_count: i64,
    /// Representative observation path.
    pub file_path: Option<String>,
    /// Representative observation line.
    pub start_line: Option<i64>,
}

impl PathHop {
    /// `file:line` of the representative observation, or `-`.
    pub fn location(&self) -> String {
        match (&self.file_path, self.start_line) {
            (Some(file), Some(line)) => format!("{file}:{line}"),
            (Some(file), None) => file.clone(),
            _ => "-".to_string(),
        }
    }
}

/// A sequence of hops from the requested source to the requested target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphPath {
    /// Hops in traversal order. Empty when source and target are the same entity.
    pub hops: Vec<PathHop>,
}

impl GraphPath {
    /// Number of hops.
    pub fn length(&self) -> usize {
        self.hops.len()
    }

    /// True when any hop rests on an unresolved assertion.
    pub fn traverses_unresolved(&self) -> bool {
        self.hops.iter().any(|hop| hop.is_unresolved)
    }
}

/// What a path search found, and how hard it looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathReport {
    /// Requested source.
    pub from: EntityRef,
    /// Requested target.
    pub to: EntityRef,
    /// Paths found, shortest first, at most `PathQuery::limit`.
    pub paths: Vec<GraphPath>,
    /// Depth bound the search used.
    pub max_depth: usize,
    /// True when the search budget stopped the walk before it was exhaustive.
    pub truncated: bool,
    /// Partial paths expanded.
    pub expansions: usize,
}

/// One edge as read from the database during the walk.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    relation: String,
    neighbour: String,
    assertion_id: String,
    traversed_backwards: bool,
}

/// A path under construction.
#[derive(Debug, Clone)]
struct Partial {
    /// Entity ids visited, in order. Also the "already used" set: paths are simple.
    nodes: Vec<String>,
    edges: Vec<Edge>,
}

/// `AND a.relation IN (...)`, built from the closed [`Relation`] vocabulary.
///
/// The values are `&'static str` literals returned by [`Relation::as_str`], never user text,
/// so there is nothing here for a caller to inject through.
fn relation_clause(relations: &[Relation]) -> String {
    if relations.is_empty() {
        return String::new();
    }
    let names: Vec<String> = relations
        .iter()
        .map(|relation| format!("'{}'", relation.as_str()))
        .collect();
    format!(" AND a.relation IN ({})", names.join(", "))
}

fn adjacency_sql(query: &PathQuery, backwards: bool) -> String {
    let (anchor, neighbour) = if backwards {
        ("target_entity_id", "source_entity_id")
    } else {
        ("source_entity_id", "target_entity_id")
    };
    let relations = relation_clause(&query.relations);
    let resolved = if query.resolved_only {
        " AND COALESCE(s.is_unresolved, 0) = 0"
    } else {
        ""
    };
    format!(
        "SELECT a.relation, a.{neighbour}, a.assertion_id
           FROM assertion a
           LEFT JOIN assertion_state s ON s.assertion_id = a.assertion_id
          WHERE a.{anchor} = ?1{relations}{resolved}
          ORDER BY a.relation, a.{neighbour}, a.assertion_id"
    )
}

/// Find up to `query.limit` simple paths from `from_id` to `to_id`.
///
/// Breadth-first over partial paths, so paths come back shortest first. Paths are simple: a
/// path never revisits an entity, which also makes the walk terminate without a global visited
/// set that would suppress alternative routes.
pub fn find_paths(
    conn: &Connection,
    from_id: &str,
    to_id: &str,
    query: &PathQuery,
) -> Result<PathReport> {
    let mut entities: BTreeMap<String, EntityRef> = BTreeMap::new();
    let from = load_entity(conn, &mut entities, from_id)?;
    let to = load_entity(conn, &mut entities, to_id)?;

    let mut forward = conn.prepare(&adjacency_sql(query, false))?;
    let mut reverse = match query.direction {
        Direction::Any => Some(conn.prepare(&adjacency_sql(query, true))?),
        Direction::Forward => None,
    };

    let mut adjacency: BTreeMap<String, Vec<Edge>> = BTreeMap::new();
    let mut found: Vec<Vec<Edge>> = Vec::new();
    let mut expansions = 0usize;
    let mut truncated = false;

    if from_id == to_id {
        // The degenerate question has a degenerate answer, and it is not "no path".
        found.push(Vec::new());
    }

    let mut frontier: VecDeque<Partial> = VecDeque::new();
    if found.is_empty() && query.max_depth > 0 {
        frontier.push_back(Partial {
            nodes: vec![from_id.to_string()],
            edges: Vec::new(),
        });
    }

    while let Some(partial) = frontier.pop_front() {
        if found.len() >= query.limit {
            break;
        }
        if expansions >= MAX_EXPANSIONS {
            truncated = true;
            break;
        }
        expansions += 1;

        let last = partial.nodes.last().expect("a partial path has a head");
        if !adjacency.contains_key(last) {
            let mut edges = read_edges(&mut forward, last, false)?;
            if let Some(reverse) = reverse.as_mut() {
                edges.extend(read_edges(reverse, last, true)?);
            }
            edges.sort();
            edges.dedup();
            adjacency.insert(last.clone(), edges);
        }
        let edges = adjacency.get(last).expect("just inserted").clone();

        for edge in edges {
            if partial.nodes.contains(&edge.neighbour) {
                continue;
            }
            let mut nodes = partial.nodes.clone();
            nodes.push(edge.neighbour.clone());
            let mut walked = partial.edges.clone();
            walked.push(edge);

            if nodes.last().is_some_and(|node| node == to_id) {
                found.push(walked);
                if found.len() >= query.limit {
                    break;
                }
                continue;
            }
            if walked.len() >= query.max_depth {
                continue;
            }
            if frontier.len() >= MAX_FRONTIER {
                truncated = true;
                break;
            }
            frontier.push_back(Partial {
                nodes,
                edges: walked,
            });
        }
        if truncated {
            break;
        }
    }

    let mut paths = Vec::with_capacity(found.len());
    for edges in found {
        paths.push(hydrate(conn, &mut entities, from_id, &edges)?);
    }

    Ok(PathReport {
        from,
        to,
        paths,
        max_depth: query.max_depth,
        truncated,
        expansions,
    })
}

fn read_edges(
    stmt: &mut rusqlite::Statement<'_>,
    anchor: &str,
    traversed_backwards: bool,
) -> Result<Vec<Edge>> {
    let rows = stmt.query_map(params![anchor], |row| {
        Ok(Edge {
            relation: row.get(0)?,
            neighbour: row.get(1)?,
            assertion_id: row.get(2)?,
            traversed_backwards,
        })
    })?;
    let mut edges = Vec::new();
    for row in rows {
        edges.push(row?);
    }
    Ok(edges)
}

fn hydrate(
    conn: &Connection,
    entities: &mut BTreeMap<String, EntityRef>,
    from_id: &str,
    edges: &[Edge],
) -> Result<GraphPath> {
    let mut hops = Vec::with_capacity(edges.len());
    let mut current = from_id.to_string();
    for edge in edges {
        let state = assertion_state(conn, &edge.assertion_id)?;
        let (file_path, start_line) = representative_observation(conn, &edge.assertion_id)?;
        let from = load_entity(conn, entities, &current)?;
        let to = load_entity(conn, entities, &edge.neighbour)?;
        hops.push(PathHop {
            from,
            to,
            relation: edge.relation.clone(),
            assertion_id: edge.assertion_id.clone(),
            traversed_backwards: edge.traversed_backwards,
            is_unresolved: state.as_ref().is_some_and(|state| state.is_unresolved),
            status: state.as_ref().map(|state| state.status.clone()),
            strongest_source_type: state
                .as_ref()
                .map(|state| state.strongest_source_type.clone()),
            observation_count: state.as_ref().map_or(0, |state| state.observation_count),
            file_path,
            start_line,
        });
        current = edge.neighbour.clone();
    }
    Ok(GraphPath { hops })
}

struct DerivedState {
    status: String,
    strongest_source_type: String,
    observation_count: i64,
    is_unresolved: bool,
}

fn assertion_state(conn: &Connection, assertion_id: &str) -> Result<Option<DerivedState>> {
    let found = conn
        .query_row(
            "SELECT status, strongest_source_type, observation_count, is_unresolved
               FROM assertion_state WHERE assertion_id = ?1",
            params![assertion_id],
            |row| {
                Ok(DerivedState {
                    status: row.get(0)?,
                    strongest_source_type: row.get(1)?,
                    observation_count: row.get(2)?,
                    is_unresolved: row.get::<_, i64>(3)? != 0,
                })
            },
        )
        .ok();
    Ok(found)
}

fn representative_observation(
    conn: &Connection,
    assertion_id: &str,
) -> Result<(Option<String>, Option<i64>)> {
    let found = conn
        .query_row(
            "SELECT file_path, start_line FROM observation
              WHERE assertion_id = ?1
              ORDER BY file_path, start_line, end_line, observation_id LIMIT 1",
            params![assertion_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .ok();
    Ok(match found {
        Some((path, line)) => (Some(path), Some(line)),
        None => (None, None),
    })
}

fn load_entity(
    conn: &Connection,
    cache: &mut BTreeMap<String, EntityRef>,
    entity_id: &str,
) -> Result<EntityRef> {
    if let Some(entity) = cache.get(entity_id) {
        return Ok(entity.clone());
    }
    let sql = format!("SELECT {ENTITY_COLUMNS} {ENTITY_FROM} WHERE e.entity_id = ?1");
    let entity = conn.query_row(&sql, params![entity_id], EntityRef::read)?;
    cache.insert(entity_id.to_string(), entity.clone());
    Ok(entity)
}

// ---- neighbourhood -------------------------------------------------------------------------
//
// A graph view that renders "the repository" answers no question and cannot be drawn: a 200k
// entity repository is not a picture. Every graph this crate produces is therefore a **bounded
// neighbourhood of one focused entity**, and when the bound bites, the report says by how much.
// "Nothing else is connected" and "I stopped looking" are different answers.

/// Bounds and filters for a neighbourhood expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighbourhoodQuery {
    /// How many hops from the focus to expand.
    pub max_depth: usize,
    /// Largest number of nodes to admit, including the focus.
    pub max_nodes: usize,
    /// Edge direction policy. `Any` is the useful default for a picture.
    pub direction: Direction,
    /// Relations to follow. Empty means every relation.
    pub relations: Vec<Relation>,
    /// Exclude edges whose `assertion_state.is_unresolved` is set.
    pub resolved_only: bool,
}

impl Default for NeighbourhoodQuery {
    fn default() -> Self {
        NeighbourhoodQuery {
            max_depth: 1,
            max_nodes: 60,
            direction: Direction::Any,
            relations: Vec::new(),
            resolved_only: false,
        }
    }
}

/// One admitted node, with the hop distance at which it was first reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighbourNode {
    /// The entity.
    pub entity: EntityRef,
    /// Hops from the focus. The focus itself is 0.
    pub depth: usize,
}

/// One edge between two admitted nodes, in its recorded direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighbourEdge {
    /// Assertion backing the edge.
    pub assertion_id: String,
    /// Relation name.
    pub relation: String,
    /// Assertion source entity id.
    pub source_entity_id: String,
    /// Assertion target entity id.
    pub target_entity_id: String,
    /// Derived `assertion_state.is_unresolved`.
    pub is_unresolved: bool,
    /// Derived `assertion_state.status`.
    pub status: Option<String>,
    /// Derived `assertion_state.strongest_source_type`.
    pub strongest_source_type: Option<String>,
    /// Derived `assertion_state.observation_count`.
    pub observation_count: i64,
    /// Representative observation path.
    pub file_path: Option<String>,
    /// Representative observation line.
    pub start_line: Option<i64>,
}

/// A bounded neighbourhood, and an honest account of what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighbourhoodReport {
    /// Entity the neighbourhood is centred on.
    pub focus: EntityRef,
    /// Admitted nodes: the focus first, then by depth and identifier.
    pub nodes: Vec<NeighbourNode>,
    /// Edges whose both endpoints were admitted, in a stable order.
    pub edges: Vec<NeighbourEdge>,
    /// Depth bound used.
    pub max_depth: usize,
    /// Node budget used.
    pub max_nodes: usize,
    /// True when the **node budget** refused at least one neighbour.
    ///
    /// Deliberately not set merely because the depth bound was reached: a caller who asked for
    /// depth 1 and got all of depth 1 got a complete answer to the question they asked. What
    /// lies past the boundary is [`NeighbourhoodReport::frontier_nodes`], which is an invitation
    /// to expand, not a warning that something was dropped.
    pub truncated: bool,
    /// Distinct adjacent entities that were reached but refused a slot.
    ///
    /// This is the "N more not shown" number. It counts entities, not edges.
    pub omitted_nodes: usize,
    /// Nodes admitted at the depth bound whose own neighbours were never looked at.
    pub frontier_nodes: usize,
}

/// One adjacency row, in its recorded direction.
struct AdjacentRow {
    assertion_id: String,
    relation: String,
    source_entity_id: String,
    target_entity_id: String,
    status: Option<String>,
    strongest_source_type: Option<String>,
    observation_count: Option<i64>,
    is_unresolved: Option<i64>,
}

fn neighbourhood_sql(query: &NeighbourhoodQuery) -> String {
    let endpoint = match query.direction {
        Direction::Forward => "a.source_entity_id = ?1",
        Direction::Any => "a.source_entity_id = ?1 OR a.target_entity_id = ?1",
    };
    let relations = relation_clause(&query.relations);
    let resolved = if query.resolved_only {
        " AND COALESCE(s.is_unresolved, 0) = 0"
    } else {
        ""
    };
    format!(
        "SELECT a.assertion_id, a.relation, a.source_entity_id, a.target_entity_id,
                s.status, s.strongest_source_type, s.observation_count, s.is_unresolved
           FROM assertion a
           LEFT JOIN assertion_state s ON s.assertion_id = a.assertion_id
          WHERE ({endpoint}){relations}{resolved}
          ORDER BY a.relation, a.source_entity_id, a.target_entity_id, a.assertion_id"
    )
}

/// Expand a bounded neighbourhood around `focus_id`.
///
/// Breadth-first, admitting nodes in a deterministic order until the budget is spent. An edge is
/// reported only when **both** of its endpoints were admitted, so the returned graph is closed:
/// a renderer never has to invent a node to draw an edge it was handed.
pub fn neighbourhood(
    conn: &Connection,
    focus_id: &str,
    query: &NeighbourhoodQuery,
) -> Result<NeighbourhoodReport> {
    let mut entities: BTreeMap<String, EntityRef> = BTreeMap::new();
    let focus = load_entity(conn, &mut entities, focus_id)?;

    let mut depths: BTreeMap<String, usize> = BTreeMap::new();
    depths.insert(focus_id.to_string(), 0);
    let mut admitted_order: Vec<String> = vec![focus_id.to_string()];
    let mut omitted: BTreeSet<String> = BTreeSet::new();
    let mut rows: BTreeMap<String, AdjacentRow> = BTreeMap::new();
    let mut truncated = false;

    let mut stmt = conn.prepare(&neighbourhood_sql(query))?;
    let mut frontier: Vec<String> = vec![focus_id.to_string()];

    for depth in 1..=query.max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<String> = Vec::new();
        for anchor in &frontier {
            let found = stmt.query_map(params![anchor], |row| {
                Ok(AdjacentRow {
                    assertion_id: row.get(0)?,
                    relation: row.get(1)?,
                    source_entity_id: row.get(2)?,
                    target_entity_id: row.get(3)?,
                    status: row.get(4)?,
                    strongest_source_type: row.get(5)?,
                    observation_count: row.get(6)?,
                    is_unresolved: row.get(7)?,
                })
            })?;
            for row in found {
                let row = row?;
                let neighbour = if row.source_entity_id == *anchor {
                    row.target_entity_id.clone()
                } else {
                    row.source_entity_id.clone()
                };
                if !depths.contains_key(&neighbour) {
                    if depths.len() >= query.max_nodes {
                        omitted.insert(neighbour);
                        truncated = true;
                        continue;
                    }
                    depths.insert(neighbour.clone(), depth);
                    admitted_order.push(neighbour.clone());
                    next.push(neighbour.clone());
                }
                rows.entry(row.assertion_id.clone()).or_insert(row);
            }
        }
        frontier = next;
    }
    // Whatever is left in the frontier was admitted but never expanded: the depth bound stopped
    // there. That is the "expand" affordance, reported separately from the budget having bitten.
    let frontier_nodes = frontier.len();

    let mut nodes = Vec::with_capacity(admitted_order.len());
    for entity_id in &admitted_order {
        nodes.push(NeighbourNode {
            entity: load_entity(conn, &mut entities, entity_id)?,
            depth: depths[entity_id],
        });
    }
    // Deterministic regardless of the order the walk happened to discover things.
    nodes.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.entity.kind.cmp(&b.entity.kind))
            .then_with(|| a.entity.name.cmp(&b.entity.name))
            .then_with(|| a.entity.entity_id.cmp(&b.entity.entity_id))
    });

    let mut edges = Vec::new();
    for row in rows.into_values() {
        if !depths.contains_key(&row.source_entity_id)
            || !depths.contains_key(&row.target_entity_id)
        {
            continue;
        }
        let (file_path, start_line) = representative_observation(conn, &row.assertion_id)?;
        edges.push(NeighbourEdge {
            is_unresolved: row.is_unresolved.unwrap_or(0) != 0,
            status: row.status,
            strongest_source_type: row.strongest_source_type,
            observation_count: row.observation_count.unwrap_or(0),
            assertion_id: row.assertion_id,
            relation: row.relation,
            source_entity_id: row.source_entity_id,
            target_entity_id: row.target_entity_id,
            file_path,
            start_line,
        });
    }
    edges.sort_by(|a, b| {
        a.relation
            .cmp(&b.relation)
            .then_with(|| a.source_entity_id.cmp(&b.source_entity_id))
            .then_with(|| a.target_entity_id.cmp(&b.target_entity_id))
            .then_with(|| a.assertion_id.cmp(&b.assertion_id))
    });

    // An entity can be reached, refused a slot, and then admitted from another anchor.
    let omitted_nodes = omitted
        .iter()
        .filter(|entity_id| !depths.contains_key(*entity_id))
        .count();

    Ok(NeighbourhoodReport {
        focus,
        nodes,
        edges,
        max_depth: query.max_depth,
        max_nodes: query.max_nodes,
        truncated,
        omitted_nodes,
        frontier_nodes,
    })
}

// ---- why ---------------------------------------------------------------------------------

/// Which side of the subject entity an assertion sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    /// The subject is the assertion's source.
    Outgoing,
    /// The subject is the assertion's target.
    Incoming,
}

impl EdgeDirection {
    /// Canonical name used in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeDirection::Outgoing => "outgoing",
            EdgeDirection::Incoming => "incoming",
        }
    }
}

/// Which assertions around the subject to explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhyDirection {
    /// Both sides.
    Both,
    /// Only assertions where the subject is the source.
    Outgoing,
    /// Only assertions where the subject is the target.
    Incoming,
}

impl WhyDirection {
    /// Canonical name used in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            WhyDirection::Both => "both",
            WhyDirection::Outgoing => "outgoing",
            WhyDirection::Incoming => "incoming",
        }
    }
}

/// Filters for an evidence question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhyQuery {
    /// Side of the subject to report.
    pub direction: WhyDirection,
    /// Relations to report. Empty means every relation.
    pub relations: Vec<Relation>,
}

impl Default for WhyQuery {
    fn default() -> Self {
        WhyQuery {
            direction: WhyDirection::Both,
            relations: Vec::new(),
        }
    }
}

/// One observation, with its full evidence profile and computed freshness.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservationEvidence {
    /// Surrogate observation id.
    pub observation_id: i64,
    /// ADR-0003 source type.
    pub evidence_source_type: String,
    /// ADR-0003 directness.
    pub directness: String,
    /// Extractor that produced it.
    pub extractor_id: String,
    /// Version of that extractor.
    pub extractor_version: String,
    /// Match quality, only for extractors that perform matching.
    pub match_quality: Option<f64>,
    /// Repository state the run that produced this observation was made in.
    ///
    /// Read through `extractor_run`, not off the observation row: since ADR-0006 the state is a
    /// property of the run, not of the evidence.
    pub state_id: String,
    /// File the evidence points at.
    pub file_path: String,
    /// First line of the evidence span.
    pub start_line: i64,
    /// Last line of the evidence span.
    pub end_line: i64,
    /// Content hash recorded when the observation was made.
    pub content_hash: String,
    /// Execution environment, for execution evidence.
    pub environment: Option<String>,
    /// Extractor-specific evidence detail, as stored.
    pub details: Option<String>,
    /// When the observation was written.
    pub created_at: String,
    /// Whether the file still says what the observation recorded. Computed, never stored.
    pub freshness: Freshness,
}

impl ObservationEvidence {
    /// `file:line`.
    pub fn location(&self) -> String {
        format!("{}:{}", self.file_path, self.start_line)
    }
}

/// One assertion and every observation supporting it.
#[derive(Debug, Clone, PartialEq)]
pub struct AssertionEvidence {
    /// Assertion identifier.
    pub assertion_id: String,
    /// Relation name.
    pub relation: String,
    /// Assertion source.
    pub source: EntityRef,
    /// Assertion target.
    pub target: EntityRef,
    /// Side of the subject this assertion sits on.
    pub direction: EdgeDirection,
    /// Derived status.
    pub status: Option<String>,
    /// Derived unresolved flag.
    pub is_unresolved: bool,
    /// Derived observation count.
    pub observation_count: i64,
    /// Derived strongest source type.
    pub strongest_source_type: Option<String>,
    /// Every supporting observation, ordered by location.
    pub observations: Vec<ObservationEvidence>,
}

/// Everything `nerve why` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct WhyReport {
    /// Entity the question was asked about.
    pub subject: EntityRef,
    /// Second entity, when the question named one.
    pub object: Option<EntityRef>,
    /// Assertions, in a stable order.
    pub assertions: Vec<AssertionEvidence>,
    /// Distinct files re-hashed to compute freshness.
    pub files_probed: usize,
}

/// Assemble every assertion around `subject_id`, with all of its evidence.
///
/// `prober` supplies the query-time file reads freshness needs. It is a parameter rather than
/// something this crate does itself because opening a repository path is a path-safety
/// decision that belongs to the code owning the repository root.
pub fn explain(
    conn: &Connection,
    subject_id: &str,
    object_id: Option<&str>,
    query: &WhyQuery,
    prober: &dyn FileProber,
) -> Result<WhyReport> {
    let mut entities: BTreeMap<String, EntityRef> = BTreeMap::new();
    let subject = load_entity(conn, &mut entities, subject_id)?;
    let object = match object_id {
        Some(id) => Some(load_entity(conn, &mut entities, id)?),
        None => None,
    };

    let endpoint = match object_id {
        Some(_) => {
            "(a.source_entity_id = ?1 AND a.target_entity_id = ?2)
                 OR (a.source_entity_id = ?2 AND a.target_entity_id = ?1)"
        }
        None => "a.source_entity_id = ?1 OR a.target_entity_id = ?1",
    };
    let side = match query.direction {
        WhyDirection::Both => "",
        WhyDirection::Outgoing => " AND a.source_entity_id = ?1",
        WhyDirection::Incoming => " AND a.target_entity_id = ?1",
    };
    let relations = relation_clause(&query.relations);
    let sql = format!(
        "SELECT a.assertion_id, a.relation, a.source_entity_id, a.target_entity_id,
                s.status, s.strongest_source_type, s.observation_count, s.is_unresolved
           FROM assertion a
           LEFT JOIN assertion_state s ON s.assertion_id = a.assertion_id
          WHERE ({endpoint}){side}{relations}
          ORDER BY a.relation,
                   CASE WHEN a.source_entity_id = ?1 THEN 0 ELSE 1 END,
                   CASE WHEN a.source_entity_id = ?1
                        THEN a.target_entity_id ELSE a.source_entity_id END,
                   a.assertion_id"
    );

    struct Row {
        assertion_id: String,
        relation: String,
        source_entity_id: String,
        target_entity_id: String,
        status: Option<String>,
        strongest_source_type: Option<String>,
        observation_count: Option<i64>,
        is_unresolved: Option<i64>,
    }
    let read = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
        Ok(Row {
            assertion_id: row.get(0)?,
            relation: row.get(1)?,
            source_entity_id: row.get(2)?,
            target_entity_id: row.get(3)?,
            status: row.get(4)?,
            strongest_source_type: row.get(5)?,
            observation_count: row.get(6)?,
            is_unresolved: row.get(7)?,
        })
    };

    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<Row> = Vec::new();
    match object_id {
        Some(object_id) => {
            for row in stmt.query_map(params![subject_id, object_id], read)? {
                rows.push(row?);
            }
        }
        None => {
            for row in stmt.query_map(params![subject_id], read)? {
                rows.push(row?);
            }
        }
    }
    drop(stmt);

    let mut cache = FreshnessCache::new(prober);
    let mut assertions = Vec::with_capacity(rows.len());
    for row in rows {
        let observations = observations_for(conn, &row.assertion_id, &mut cache)?;
        assertions.push(AssertionEvidence {
            direction: if row.source_entity_id == subject_id {
                EdgeDirection::Outgoing
            } else {
                EdgeDirection::Incoming
            },
            source: load_entity(conn, &mut entities, &row.source_entity_id)?,
            target: load_entity(conn, &mut entities, &row.target_entity_id)?,
            assertion_id: row.assertion_id,
            relation: row.relation,
            status: row.status,
            is_unresolved: row.is_unresolved.unwrap_or(0) != 0,
            observation_count: row.observation_count.unwrap_or(0),
            strongest_source_type: row.strongest_source_type,
            observations,
        });
    }

    Ok(WhyReport {
        subject,
        object,
        assertions,
        files_probed: cache.files_probed(),
    })
}

fn observations_for(
    conn: &Connection,
    assertion_id: &str,
    cache: &mut FreshnessCache<'_>,
) -> Result<Vec<ObservationEvidence>> {
    let mut stmt = conn.prepare(
        "SELECT o.observation_id, o.evidence_source_type, o.directness, o.extractor_id,
                o.extractor_version, o.match_quality, r.state_id, o.file_path, o.start_line,
                o.end_line, o.content_hash, o.environment, o.details, o.created_at
           FROM observation o
           JOIN extractor_run r ON r.run_id = o.extractor_run_id
          WHERE o.assertion_id = ?1
          ORDER BY o.file_path, o.start_line, o.end_line, o.observation_id",
    )?;
    let rows = stmt.query_map(params![assertion_id], |row| {
        Ok(ObservationEvidence {
            observation_id: row.get(0)?,
            evidence_source_type: row.get(1)?,
            directness: row.get(2)?,
            extractor_id: row.get(3)?,
            extractor_version: row.get(4)?,
            match_quality: row.get(5)?,
            state_id: row.get(6)?,
            file_path: row.get(7)?,
            start_line: row.get(8)?,
            end_line: row.get(9)?,
            content_hash: row.get(10)?,
            environment: row.get(11)?,
            details: row.get(12)?,
            created_at: row.get(13)?,
            // Replaced below; the freshness cache is not available inside the row mapper.
            freshness: Freshness::Unreadable,
        })
    })?;
    let mut observations = Vec::new();
    for row in rows {
        let mut observation = row?;
        observation.freshness = cache.evaluate(&observation.file_path, &observation.content_hash);
        observations.push(observation);
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_clause_is_empty_for_no_filter() {
        assert_eq!(relation_clause(&[]), "");
    }

    #[test]
    fn relation_clause_lists_closed_vocabulary_names_only() {
        let clause = relation_clause(&[Relation::Calls, Relation::Defines]);
        assert_eq!(clause, " AND a.relation IN ('CALLS', 'DEFINES')");
        // Nothing user-supplied can reach the clause: it is built from `as_str` literals.
        for relation in Relation::ALL {
            assert!(!relation.as_str().contains('\''));
        }
    }

    #[test]
    fn adjacency_sql_flips_the_anchor_when_walking_backwards() {
        let query = PathQuery::default();
        assert!(adjacency_sql(&query, false).contains("WHERE a.source_entity_id = ?1"));
        assert!(adjacency_sql(&query, true).contains("WHERE a.target_entity_id = ?1"));
    }

    #[test]
    fn resolved_only_filters_on_the_derived_flag() {
        let query = PathQuery {
            resolved_only: true,
            ..PathQuery::default()
        };
        assert!(adjacency_sql(&query, false).contains("COALESCE(s.is_unresolved, 0) = 0"));
        assert!(!adjacency_sql(&PathQuery::default(), false).contains("is_unresolved"));
    }

    #[test]
    fn every_adjacency_query_is_ordered() {
        for backwards in [false, true] {
            assert!(adjacency_sql(&PathQuery::default(), backwards).contains("ORDER BY"));
        }
    }

    #[test]
    fn direction_and_edge_direction_names_are_the_json_contract() {
        assert_eq!(Direction::Forward.as_str(), "forward");
        assert_eq!(Direction::Any.as_str(), "any");
        assert_eq!(EdgeDirection::Outgoing.as_str(), "outgoing");
        assert_eq!(EdgeDirection::Incoming.as_str(), "incoming");
        assert_eq!(WhyDirection::Both.as_str(), "both");
    }

    #[test]
    fn defaults_match_the_documented_command_line() {
        let query = PathQuery::default();
        assert_eq!(query.max_depth, 6);
        assert_eq!(query.limit, 3);
        assert_eq!(query.direction, Direction::Forward);
        assert!(!query.resolved_only);
        assert!(query.relations.is_empty());
    }
}
