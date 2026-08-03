//! Turning what a human typed into exactly one entity, or refusing to.
//!
//! ```text
//! selector  := [ qualifier ":" ] body
//! qualifier := <entity-kind>      every EntityKind::as_str(), generated
//!            | "symbol"           alias: any kind where is_symbol()
//!            | "adr"              alias: a Document whose meta.adr is true
//! body      := <entity_id>
//!            | <rel_path>
//!            | <rel_path> "#" <qualified_name>
//!            | <name> | <qualified_name>
//! ```
//!
//! Resolution order, first stage that matches anything wins:
//!
//! 1. an exact `entity_id`
//! 2. `<rel_path>` — every entity **at** that path
//! 3. `<rel_path>#<qualified_name>` — a symbol in that file
//! 4. a bare `<qualified_name>` or `<name>`, unique across the repository
//!
//! A stage that matches more than one entity is **ambiguous** and stops resolution there. It
//! does not fall through to the next stage and it never picks one: silently choosing is the
//! failure mode that makes a tool untrustworthy in exactly the situation where the user most
//! needs it to be right.
//!
//! # Where the two-tier path rule is *not* a guess
//!
//! Stage 2 can match a content entity and a container entity at one path — `src/app.ts` has a
//! `Module` and a `File`; `docs/architecture.md` has a `Document` and a `File`. It resolves to
//! the content entity and returns the container as an **alternative**.
//!
//! That is a rule, not a coin toss, and the difference is load-bearing. Two functions named
//! `parse` are indistinguishable to Nerve, so it must refuse. A `File` and the `Document` inside
//! it are distinguishable by a fixed, total, stated rule ([`EntityKind::path_role`]); the answer
//! reports that the rule fired (`matched_by = "path"`), lists what it passed over, and the
//! passed-over entity stays directly addressable as `file:<path>`. Two matches **inside the
//! deciding tier** is still ambiguous, so the refusal is intact where the ambiguity is real.
//!
//! # A qualifier constrains kinds; it does not change the stages
//!
//! A qualifier is recognised only when the selector's first `:` comes before its first `/` and
//! its first `#`, so a repository path containing a colon below the root — `docs/a:b.md` — is
//! still a path rather than a malformed qualifier. A prefix that *is* in qualifier position and
//! is not a qualifier is an **invalid selector**, not a bare name: `banana:foo` is a malformed
//! request, and answering "no such entity" would assert a search that never happened.
//!
//! `adr:` is the one qualifier that also widens a stage's predicate, because an ADR's identifier
//! lives in `meta.adr_id` rather than in its name. It is stated here rather than hidden in the
//! stage.
//!
//! # The traversal refusal is syntactic, and it is shared
//!
//! [`selector_shape`] refuses a traversal-shaped selector before any stage runs. Nothing here
//! touches the filesystem — this is string shape, not a path resolution — and the authoritative
//! guard remains `nerve-index`'s `canonical_child` choke point, which screens the paths the
//! *database* hands back. The check lives here so that the CLI, the HTTP API and the MCP tool
//! cannot answer the same question three different ways: THREAT-MODEL T2's rule is that a
//! refusal is reported as a refusal and never disguised as *missing*, and until Slice 8b-i two
//! of the three surfaces said "matches no indexed entity" — asserting a check they never ran.

use std::path::Path;

use rusqlite::{params, Connection};

use nerve_core::vocab::{EntityKind, PathRole};

use crate::error::Result;
use crate::query::{search_entities, SearchHit};

/// How many `nerve search` suggestions accompany a selector that matched nothing.
pub const SUGGESTION_LIMIT: usize = 5;

/// Shortest prefix a suggestion search will fall back to.
pub const MIN_SUGGESTION_PREFIX: usize = 3;

/// The symbol kinds as a quoted, comma-separated SQL list, for an `IN (…)` clause.
///
/// Every kind for which [`EntityKind::is_symbol`] holds, and only those. The list is built from
/// the closed compile-time vocabulary and **never** from caller text, so it is not an injection
/// site, and it cannot drift from [`EntityKind::is_symbol`]: a kind added to the vocabulary is
/// included the moment the vocabulary says it is a symbol, and there is no second list to fall
/// out of step with.
///
/// One helper rather than one copy per question. Four questions in this crate need exactly this
/// list, for four different reasons, and each call site states its own:
///
/// - selector resolution, below — a scope is folded into a dotted qualified name only for a
///   symbol;
/// - [`crate::query::symbol_spans_in_file`] — coverage lines are mapped onto symbols only;
/// - [`crate::gaps`] — the gap question is asked of symbols only;
/// - [`crate::query::status`] — `symbols_total` counts symbols only.
pub(crate) fn symbol_kinds_sql() -> String {
    kinds_sql(|kind| kind.is_symbol())
}

/// The kinds with one [`PathRole`], as a quoted SQL list, for an `IN (…)` clause.
///
/// Built from the same closed vocabulary and for the same reason as [`symbol_kinds_sql`]: the
/// path stage must ask about exactly the kinds the vocabulary says a path names, and a second
/// hand-written list is how 5d-iii and 7a-iii went wrong.
pub(crate) fn path_kinds_sql(role: PathRole) -> String {
    kinds_sql(|kind| kind.path_role() == role)
}

fn kinds_sql(include: impl Fn(EntityKind) -> bool) -> String {
    EntityKind::ALL
        .iter()
        .copied()
        .filter(|kind| include(*kind))
        .map(|kind| format!("'{}'", kind.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Columns every entity lookup in this crate reads, in order.
pub(crate) const ENTITY_COLUMNS: &str =
    "e.entity_id, e.kind, e.name, e.scope_path, e.language, o.file_path, o.start_line, o.end_line";

/// `FROM` clause pairing an entity with its first occurrence, deterministically chosen.
pub(crate) const ENTITY_FROM: &str = "FROM entity e
     LEFT JOIN occurrence o ON o.occurrence_id = (
          SELECT occurrence_id FROM occurrence
           WHERE entity_id = e.entity_id
           ORDER BY file_path, start_byte, end_byte LIMIT 1)";

/// Stable ordering for any list of entities shown to a human.
pub(crate) const ENTITY_ORDER: &str = "ORDER BY e.kind, e.scope_path, e.name, e.entity_id";

/// `scope.name` for a scoped symbol, `name` otherwise. **The one implementation.**
///
/// A `Module`'s and a `Document`'s `scope_path` is its own file path, a `File`'s and a
/// `Directory`'s is its parent directory, and an `Unresolved`'s is the importer that failed to
/// resolve it, so none of them is folded in: the result would be a dotted name that appears
/// nowhere in the source *and cannot be typed back as a selector*. Two private copies of this
/// fold without the [`EntityKind::is_symbol`] guard — one in `nerve search`, one in the
/// suggestion list a failed selector prints — are what made `nerve why` suggest
/// `docs.architecture.md`, which resolves to nothing. They are gone; this is the only fold.
pub(crate) fn fold_scope(kind: &str, scope_path: &str, name: &str) -> String {
    let scoped = kind
        .parse::<EntityKind>()
        .map(EntityKind::is_symbol)
        .unwrap_or(false);
    if scope_path.is_empty() || !scoped {
        name.to_string()
    } else {
        format!("{scope_path}.{name}")
    }
}

/// `file:line`, or `-` when the entity has no occurrence. The one implementation.
pub(crate) fn format_location(file_path: Option<&str>, start_line: Option<i64>) -> String {
    match (file_path, start_line) {
        (Some(file), Some(line)) => format!("{file}:{line}"),
        (Some(file), None) => file.to_string(),
        _ => "-".to_string(),
    }
}

/// An entity plus enough context to recognise it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityRef {
    /// Content-derived identifier.
    pub entity_id: String,
    /// Entity kind.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Scope path: enclosing symbols for a symbol, the file path for a module.
    pub scope_path: String,
    /// Language tag.
    pub language: Option<String>,
    /// First occurrence path.
    pub file_path: Option<String>,
    /// First occurrence start line.
    pub start_line: Option<i64>,
    /// First occurrence end line.
    pub end_line: Option<i64>,
}

impl EntityRef {
    pub(crate) fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntityRef> {
        Ok(EntityRef {
            entity_id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            scope_path: row.get(3)?,
            language: row.get(4)?,
            file_path: row.get(5)?,
            start_line: row.get(6)?,
            end_line: row.get(7)?,
        })
    }

    /// `scope.name` for a scoped symbol, `name` otherwise. See [`fold_scope`].
    pub fn qualified_name(&self) -> String {
        fold_scope(&self.kind, &self.scope_path, &self.name)
    }

    /// `file:line`, or `-` when the entity has no occurrence.
    pub fn location(&self) -> String {
        format_location(self.file_path.as_deref(), self.start_line)
    }

    /// The repository-relative path this entity is *at*, when a path names it at all.
    ///
    /// The two addressable roles store the path differently — see [`EntityKind::path_role`] —
    /// so this is the Rust counterpart of the path stage's SQL, and the reason a selector like
    /// `file:docs/architecture.md` can be offered back to a caller verbatim.
    pub fn repository_path(&self) -> Option<String> {
        match self.kind.parse::<EntityKind>().ok()?.path_role() {
            PathRole::Content => Some(self.scope_path.clone()),
            PathRole::Container if self.scope_path.is_empty() => Some(self.name.clone()),
            PathRole::Container => Some(format!("{}/{}", self.scope_path, self.name)),
            PathRole::None => None,
        }
    }
}

/// The optional `<qualifier>:` prefix, which constrains which kinds a stage may return.
///
/// The kind arm is **generated** from [`EntityKind::ALL`] rather than listed, so a kind added to
/// the vocabulary is addressable the day it exists and no second vocabulary can drift from the
/// first. `symbol` and `adr` are aliases *over* that vocabulary rather than members of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qualifier {
    /// Exactly one entity kind.
    Kind(EntityKind),
    /// Any kind for which [`EntityKind::is_symbol`] holds.
    Symbol,
    /// A `Document` whose `meta.adr` is true; the body also matches `meta.adr_id`.
    Adr,
}

impl Qualifier {
    /// The aliases, which are not entity kinds.
    pub const ALIASES: [&'static str; 2] = ["symbol", "adr"];

    /// Parse a qualifier, or `None` when the text is not one.
    pub fn parse(text: &str) -> Option<Qualifier> {
        if text == "symbol" {
            return Some(Qualifier::Symbol);
        }
        if text == "adr" {
            return Some(Qualifier::Adr);
        }
        text.parse::<EntityKind>().ok().map(Qualifier::Kind)
    }

    /// The text that names this qualifier.
    pub fn as_str(self) -> &'static str {
        match self {
            Qualifier::Kind(kind) => kind.as_str(),
            Qualifier::Symbol => "symbol",
            Qualifier::Adr => "adr",
        }
    }

    /// The single kind this qualifier admits, when it admits exactly one.
    ///
    /// Used to narrow the suggestion search; the aliases admit several kinds and answer `None`.
    pub fn sole_kind(self) -> Option<EntityKind> {
        match self {
            Qualifier::Kind(kind) => Some(kind),
            Qualifier::Symbol => None,
            Qualifier::Adr => Some(EntityKind::Document),
        }
    }

    /// The SQL predicate this qualifier adds to every stage.
    ///
    /// Every literal here comes from the closed compile-time vocabulary. No caller text is ever
    /// interpolated: the body is bound as a parameter, always.
    fn sql(self) -> String {
        match self {
            Qualifier::Kind(kind) => format!("e.kind = '{}'", kind.as_str()),
            Qualifier::Symbol => format!("e.kind IN ({})", symbol_kinds_sql()),
            Qualifier::Adr => format!(
                "e.kind = '{}' AND json_extract(e.meta, '$.adr') = 1",
                EntityKind::Document.as_str()
            ),
        }
    }
}

/// Every qualifier a selector may carry, in vocabulary order then alias order.
pub fn qualifiers() -> Vec<&'static str> {
    EntityKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .chain(Qualifier::ALIASES)
        .collect()
}

/// Why a string is not a selector at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSelector {
    /// A prefix in qualifier position that is neither an entity kind nor an alias.
    UnknownQualifier,
    /// `:body` — there is a colon in qualifier position and nothing before it.
    EmptyQualifier,
    /// `kind:` or `""` — there is nothing to resolve.
    EmptyBody,
}

impl InvalidSelector {
    /// Canonical name used in `--json` output and in API detail.
    pub fn as_str(self) -> &'static str {
        match self {
            InvalidSelector::UnknownQualifier => "unknown_qualifier",
            InvalidSelector::EmptyQualifier => "empty_qualifier",
            InvalidSelector::EmptyBody => "empty_body",
        }
    }
}

/// Why a selector was refused without being looked for.
///
/// Syntactic, and never a statement about the filesystem: see the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorRefusal {
    /// A path outside the repository root, or one containing `..` or a dot segment.
    Traversal,
}

impl SelectorRefusal {
    /// Canonical name. Deliberately the code Slice 8a already used on the MCP surface.
    pub fn as_str(self) -> &'static str {
        match self {
            SelectorRefusal::Traversal => "path_refused",
        }
    }

    /// What the refusal means, in one sentence a caller can be shown.
    pub fn statement(self) -> &'static str {
        match self {
            SelectorRefusal::Traversal => {
                "a path outside the repository root, or one containing `..`, is never resolved"
            }
        }
    }
}

/// What a selector is, before anything is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorShape<'a> {
    /// Traversal-shaped. Refused, not searched for.
    Refused(SelectorRefusal),
    /// Malformed. Not a miss.
    Invalid(InvalidSelector),
    /// A qualifier, if one was given, and the body to resolve.
    Usable {
        /// The `<qualifier>:` prefix, if there was one.
        qualifier: Option<Qualifier>,
        /// Everything after it.
        body: &'a str,
    },
}

/// Decide what a selector is, without a database, a filesystem or a network.
///
/// **The one shared traversal refusal.** [`resolve_selector`] calls it, so the CLI and the HTTP
/// API refuse through it; `nerve-server`'s MCP argument validation calls it directly, so that a
/// refused argument is a JSON-RPC `-32602` before the index is touched, exactly as Slice 8a
/// shipped it. There is no second copy.
pub fn selector_shape(selector: &str) -> SelectorShape<'_> {
    let (qualifier, body) = match split_qualifier(selector) {
        Ok(split) => split,
        Err(invalid) => return SelectorShape::Invalid(invalid),
    };
    if body.is_empty() {
        return SelectorShape::Invalid(InvalidSelector::EmptyBody);
    }
    if let Some(refusal) = traversal_refusal(body) {
        return SelectorShape::Refused(refusal);
    }
    SelectorShape::Usable { qualifier, body }
}

/// Split `<qualifier>:<body>`, or answer that the whole string is the body.
///
/// The colon only introduces a qualifier when it precedes the first `/` and the first `#`. A
/// path below the root may legally contain a colon on Unix, and `docs/a:b.md` must stay a path
/// rather than become a malformed qualifier; a `path#Name:with:colons` selector likewise keeps
/// its symbol part intact.
fn split_qualifier(
    selector: &str,
) -> std::result::Result<(Option<Qualifier>, &str), InvalidSelector> {
    let Some(colon) = selector.find(':') else {
        return Ok((None, selector));
    };
    let before_separator = selector
        .find(['/', '#'])
        .map(|separator| colon < separator)
        .unwrap_or(true);
    if !before_separator {
        return Ok((None, selector));
    }
    let (prefix, body) = selector.split_at(colon);
    let body = &body[1..];
    if prefix.is_empty() {
        return Err(InvalidSelector::EmptyQualifier);
    }
    match Qualifier::parse(prefix) {
        Some(qualifier) => Ok((Some(qualifier), body)),
        None => Err(InvalidSelector::UnknownQualifier),
    }
}

/// The shapes `nerve-index`'s path choke point exists to refuse, recognised from the string.
///
/// No filesystem call is made and no path is resolved. Only the part before `#` can be
/// path-shaped, so only that part is examined.
///
/// Three things are refused, and nothing else:
///
/// 1. a leading `/` or `\`, or a [`Path::is_absolute`] path — a root other than the repository's;
/// 2. a `..` segment anywhere, splitting on **both** `/` and `\`;
/// 3. nothing else. In particular a `.` segment is not a refusal.
///
/// The two corrections over the check Slice 8a shipped, both measured against the release binary
/// and both consequences of asking `std::path::Components` a question it does not answer:
///
/// - **`./docs/architecture.md` was refused.** `Components` keeps a *leading* `CurDir`, which is
///   not `Component::Normal`, so a legal relative path was reported as an attempt to escape the
///   root. Refusing what a user may legitimately type is the same class of untruth as failing to
///   refuse what they may not: the message asserted an intent the string does not carry.
/// - **`..\..\windows` and `a\..\b` were not refused.** On Unix `\` is not a separator, so
///   `Components` sees one `Normal` and the `..` never becomes a component. They came back as
///   "matches no indexed entity" — the disguise T2 forbids.
///
/// `a\b.ts` stays **usable**: on Unix that is one legal filename that could genuinely be indexed,
/// and there is no `..` in it to refuse.
fn traversal_refusal(body: &str) -> Option<SelectorRefusal> {
    let path_part = body.split('#').next().unwrap_or_default();
    let escapes = path_part.starts_with('/')
        || path_part.starts_with('\\')
        || Path::new(path_part).is_absolute()
        || path_part.split(['/', '\\']).any(|segment| segment == "..");
    escapes.then_some(SelectorRefusal::Traversal)
}

/// Which resolution stage produced a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind {
    /// An exact `entity_id`.
    EntityId,
    /// A repository-relative path, naming whatever is at it.
    Path,
    /// `<rel_path>#<qualified_name>`.
    PathQualified,
    /// A bare qualified name or name.
    Name,
}

impl SelectorKind {
    /// Canonical name used in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            SelectorKind::EntityId => "entity_id",
            SelectorKind::Path => "path",
            SelectorKind::PathQualified => "path_qualified",
            SelectorKind::Name => "name",
        }
    }
}

/// Every stage, in the order they are tried.
const STAGES: [SelectorKind; 4] = [
    SelectorKind::EntityId,
    SelectorKind::Path,
    SelectorKind::PathQualified,
    SelectorKind::Name,
];

/// What resolving a selector produced.
#[derive(Debug, Clone)]
pub enum Selection {
    /// Exactly one entity was chosen.
    Resolved {
        /// The entity.
        entity: Box<EntityRef>,
        /// Stage that matched.
        matched_by: SelectorKind,
        /// Entities the same selector also names, which a stated rule passed over.
        ///
        /// Empty for every selector that has no second reading, so an answer that was never
        /// ambiguous is unchanged in shape. Non-empty only from the path stage's two-tier rule,
        /// and every entity in it is addressable as `<kind>:<path>`.
        alternatives: Vec<EntityRef>,
    },
    /// More than one entity matched inside the deciding tier of one stage. Nothing is chosen.
    Ambiguous {
        /// Every candidate, in a stable order.
        candidates: Vec<EntityRef>,
        /// Stage that matched.
        matched_by: SelectorKind,
    },
    /// Nothing matched at any stage.
    NotFound {
        /// The qualifier that was applied, if any.
        qualifier: Option<Qualifier>,
        /// What a stage did match and the qualifier then excluded.
        ///
        /// This is what lets a refusal say *"no module at `docs/architecture.md` — there is a
        /// document"* rather than a bare miss. Empty when no qualifier was given.
        excluded: Vec<EntityRef>,
        /// Nearest `nerve search` hits, as a starting point.
        suggestions: Vec<SearchHit>,
    },
    /// The string is not a selector. Distinct from a selector that found nothing.
    Invalid {
        /// What is wrong with it.
        reason: InvalidSelector,
    },
    /// The selector was refused before anything was looked for.
    Refused {
        /// Why.
        reason: SelectorRefusal,
    },
}

fn lookup(
    conn: &Connection,
    where_clause: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<EntityRef>> {
    let sql = format!("SELECT {ENTITY_COLUMNS} {ENTITY_FROM} WHERE {where_clause} {ENTITY_ORDER}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(args, EntityRef::read)?;
    let mut found = Vec::new();
    for row in rows {
        found.push(row?);
    }
    Ok(found)
}

/// Load one entity by its exact identifier.
pub fn entity_by_id(conn: &Connection, entity_id: &str) -> Result<Option<EntityRef>> {
    Ok(lookup(conn, "e.entity_id = ?1", params![entity_id])?
        .into_iter()
        .next())
}

/// Resolve a selector to exactly one entity, a candidate list, or a stated refusal.
pub fn resolve_selector(conn: &Connection, selector: &str) -> Result<Selection> {
    let (qualifier, body) = match selector_shape(selector) {
        SelectorShape::Refused(reason) => return Ok(Selection::Refused { reason }),
        SelectorShape::Invalid(reason) => return Ok(Selection::Invalid { reason }),
        SelectorShape::Usable { qualifier, body } => (qualifier, body),
    };

    // What a stage matched and the qualifier then excluded, from the first stage that had any.
    // Only asked for on a miss, and only when a qualifier could have caused it.
    let mut excluded: Vec<EntityRef> = Vec::new();
    for stage in STAGES {
        let candidates = stage_matches(conn, stage, body, qualifier)?;
        if candidates.is_empty() {
            if qualifier.is_some() && excluded.is_empty() {
                excluded = stage_matches(conn, stage, body, None)?;
            }
            continue;
        }
        return Ok(decide(stage, candidates));
    }

    Ok(Selection::NotFound {
        qualifier,
        excluded,
        suggestions: suggestions(conn, body, qualifier)?,
    })
}

/// Everything one stage matches, with the qualifier's constraint applied.
fn stage_matches(
    conn: &Connection,
    stage: SelectorKind,
    body: &str,
    qualifier: Option<Qualifier>,
) -> Result<Vec<EntityRef>> {
    let constraint = match qualifier {
        Some(qualifier) => format!(" AND ({})", qualifier.sql()),
        None => String::new(),
    };
    match stage {
        SelectorKind::EntityId => lookup(
            conn,
            &format!("e.entity_id = ?1{constraint}"),
            params![body],
        ),
        // Every entity **at** that path. The two addressable roles store the path in different
        // columns — `EntityKind::path_role` says which — so each needs its own predicate. A
        // `Section`'s `scope_path` is also a repository path, which is exactly why the kind list
        // is generated from the vocabulary rather than being "whatever has a path-shaped scope".
        SelectorKind::Path => lookup(
            conn,
            &format!(
                "((e.kind IN ({content}) AND e.scope_path = ?1)
                  OR (e.kind IN ({container})
                      AND CASE WHEN e.scope_path = '' THEN e.name
                               ELSE e.scope_path || '/' || e.name END = ?1)){constraint}",
                content = path_kinds_sql(PathRole::Content),
                container = path_kinds_sql(PathRole::Container),
            ),
            params![body],
        ),
        SelectorKind::PathQualified => path_qualified(conn, body, &constraint),
        // A scope is folded into a dotted qualified name only for a symbol. For every other
        // kind `scope_path` holds a repository path — a module's own file, a file's parent
        // directory, the importer that failed to resolve — so folding it in would produce a
        // name that does not exist anywhere in the source.
        SelectorKind::Name => lookup(
            conn,
            &format!(
                "(e.name = ?1 OR (e.scope_path <> '' AND e.kind IN ({symbols})
                                  AND e.scope_path || '.' || e.name = ?1){adr}){constraint}",
                symbols = symbol_kinds_sql(),
                adr = adr_id_clause(qualifier),
            ),
            params![body],
        ),
    }
}

/// The one predicate a qualifier adds rather than merely constrains.
///
/// An ADR is identified by `ADR-0001`, which is in `meta.adr_id` and in no name column. Widening
/// the name stage is stated here rather than buried in the stage, and it is reached only when
/// the caller asked for `adr:` — an unqualified `ADR-0001` still means what it always meant.
fn adr_id_clause(qualifier: Option<Qualifier>) -> &'static str {
    match qualifier {
        Some(Qualifier::Adr) => " OR json_extract(e.meta, '$.adr_id') = ?1",
        _ => "",
    }
}

/// Choose, refuse, or apply the path stage's two-tier rule.
fn decide(stage: SelectorKind, candidates: Vec<EntityRef>) -> Selection {
    let (mut deciding, alternatives) = match stage {
        SelectorKind::Path => split_tiers(candidates),
        _ => (candidates, Vec::new()),
    };
    if deciding.len() == 1 {
        Selection::Resolved {
            entity: Box::new(deciding.remove(0)),
            matched_by: stage,
            alternatives,
        }
    } else {
        Selection::Ambiguous {
            candidates: deciding,
            matched_by: stage,
        }
    }
}

/// Content first, container second; the empty tier never decides.
///
/// Returns `(deciding, passed over)`. The caller may still find the deciding tier ambiguous —
/// the rule chooses between *tiers*, never between members of one.
fn split_tiers(candidates: Vec<EntityRef>) -> (Vec<EntityRef>, Vec<EntityRef>) {
    let (content, container): (Vec<EntityRef>, Vec<EntityRef>) =
        candidates.into_iter().partition(|candidate| {
            candidate
                .kind
                .parse::<EntityKind>()
                .map(|kind| kind.path_role() == PathRole::Content)
                .unwrap_or(false)
        });
    if content.is_empty() {
        (container, Vec::new())
    } else {
        (content, container)
    }
}

/// Nearest `nerve search` hits for a selector that matched nothing.
///
/// FTS5 gives prefix matching, not fuzzy matching, so a typo in the final characters — which
/// is where typos live — finds nothing at all. Retrying with progressively shorter prefixes
/// turns `normalise` into `normali`, which does find `normalize`. The floor keeps the list a
/// suggestion rather than a dump of whatever the repository happens to contain.
fn suggestions(
    conn: &Connection,
    body: &str,
    qualifier: Option<Qualifier>,
) -> Result<Vec<SearchHit>> {
    // Search the symbol part of a `path#name` selector: the path part would be AND-ed into the
    // FTS expression and suppress every useful hit.
    let needle = body.rsplit('#').next().unwrap_or(body);
    // A qualifier that admits exactly one kind narrows the suggestions too: offering a module
    // to someone who asked for `document:` would be answering a question they did not ask.
    let kind = qualifier.and_then(Qualifier::sole_kind);
    let kind = kind.map(EntityKind::as_str);
    let mut prefix: Vec<char> = needle.chars().collect();
    while prefix.len() >= MIN_SUGGESTION_PREFIX {
        let candidate: String = prefix.iter().collect();
        let hits = search_entities(conn, &candidate, kind, SUGGESTION_LIMIT)?;
        if !hits.is_empty() {
            return Ok(hits);
        }
        prefix.pop();
    }
    Ok(Vec::new())
}

/// `<rel_path>#<qualified_name>` — a name, or a folded scope, recorded inside one file.
///
/// The scope is folded for symbol kinds only, for the same reason as the `Name` stage above: for
/// any other kind `scope_path` is a repository path, and `path.name` names nothing in the source.
fn path_qualified(conn: &Connection, body: &str, constraint: &str) -> Result<Vec<EntityRef>> {
    let Some((rel_path, qualified)) = body.split_once('#') else {
        return Ok(Vec::new());
    };
    if rel_path.is_empty() || qualified.is_empty() {
        return Ok(Vec::new());
    }
    lookup(
        conn,
        &format!(
            "EXISTS (SELECT 1 FROM occurrence oc
                      WHERE oc.entity_id = e.entity_id AND oc.file_path = ?1)
             AND (e.name = ?2 OR (e.scope_path <> '' AND e.kind IN ({})
                                  AND e.scope_path || '.' || e.name = ?2)){constraint}",
            symbol_kinds_sql()
        ),
        params![rel_path, qualified],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_name_folds_scope_for_symbols_only() {
        let method = EntityRef {
            entity_id: "meth_1".into(),
            kind: "method".into(),
            name: "area".into(),
            scope_path: "Circle".into(),
            language: None,
            file_path: Some("src/shapes.ts".into()),
            start_line: Some(9),
            end_line: Some(11),
        };
        assert_eq!(method.qualified_name(), "Circle.area");
        assert_eq!(method.location(), "src/shapes.ts:9");
        assert_eq!(method.repository_path(), None);

        let module = EntityRef {
            kind: "module".into(),
            name: "shapes".into(),
            scope_path: "src/shapes.ts".into(),
            ..method.clone()
        };
        assert_eq!(module.qualified_name(), "shapes");
        assert_eq!(module.repository_path().as_deref(), Some("src/shapes.ts"));

        let unresolved = EntityRef {
            kind: "unresolved".into(),
            name: "console.log".into(),
            scope_path: "src/globals.ts".into(),
            ..method.clone()
        };
        assert_eq!(unresolved.qualified_name(), "console.log");
        assert_eq!(unresolved.repository_path(), None);

        let orphan = EntityRef {
            file_path: None,
            start_line: None,
            scope_path: String::new(),
            ..method
        };
        assert_eq!(orphan.qualified_name(), "area");
        assert_eq!(orphan.location(), "-");
    }

    /// A container entity's path is assembled, not stored, and the root case has no separator.
    #[test]
    fn a_container_entity_reports_the_path_it_sits_at() {
        let file = EntityRef {
            entity_id: "file_1".into(),
            kind: "file".into(),
            name: "architecture.md".into(),
            scope_path: "docs".into(),
            language: None,
            file_path: Some("docs/architecture.md".into()),
            start_line: Some(1),
            end_line: Some(40),
        };
        assert_eq!(
            file.repository_path().as_deref(),
            Some("docs/architecture.md")
        );

        let at_root = EntityRef {
            name: "README.md".into(),
            scope_path: String::new(),
            ..file.clone()
        };
        assert_eq!(at_root.repository_path().as_deref(), Some("README.md"));

        let directory = EntityRef {
            kind: "directory".into(),
            name: "decisions".into(),
            scope_path: "docs".into(),
            ..file
        };
        assert_eq!(
            directory.repository_path().as_deref(),
            Some("docs/decisions")
        );
    }

    /// The one list four queries share must be exactly the vocabulary's own answer.
    ///
    /// Scope folding, symbol spans, the gap question and `symbols_total` all read this helper, so
    /// a drift here is not one wrong query but four — and one of them is a number the interface
    /// prints beside the word *symbols*. Membership is checked kind by kind, and the whole string
    /// is compared against an independently built one so that a kind cannot be present twice, or
    /// quoted differently, or ordered other than as the vocabulary declares it.
    #[test]
    fn the_shared_symbol_kind_list_is_exactly_what_the_vocabulary_calls_a_symbol() {
        let list = symbol_kinds_sql();
        for kind in EntityKind::ALL {
            let quoted = format!("'{}'", kind.as_str());
            assert_eq!(list.contains(&quoted), kind.is_symbol(), "{kind} in {list}");
        }

        let expected = EntityKind::ALL
            .iter()
            .filter(|kind| kind.is_symbol())
            .map(|kind| format!("'{}'", kind.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(list, expected);
        assert_eq!(list.matches('\'').count() / 2, 4, "four kinds, quoted once");
    }

    /// The path stage's two kind lists, for the same reason and by the same construction.
    #[test]
    fn the_path_kind_lists_are_exactly_what_the_vocabulary_calls_addressable() {
        // Declaration order, which is `EntityKind::ALL`'s and not the tier's.
        assert_eq!(path_kinds_sql(PathRole::Content), "'module', 'document'");
        assert_eq!(path_kinds_sql(PathRole::Container), "'directory', 'file'");

        for role in [PathRole::Content, PathRole::Container, PathRole::None] {
            let list = path_kinds_sql(role);
            for kind in EntityKind::ALL {
                let quoted = format!("'{}'", kind.as_str());
                assert_eq!(
                    list.contains(&quoted),
                    kind.path_role() == role,
                    "{kind} in {list}"
                );
            }
        }
        // No kind is in both lists, so a path never counts one entity twice.
        for kind in EntityKind::ALL {
            let quoted = format!("'{}'", kind.as_str());
            assert!(
                !(path_kinds_sql(PathRole::Content).contains(&quoted)
                    && path_kinds_sql(PathRole::Container).contains(&quoted)),
                "{kind} is in both tiers"
            );
        }
    }

    #[test]
    fn selector_kind_names_are_the_json_contract() {
        assert_eq!(SelectorKind::EntityId.as_str(), "entity_id");
        assert_eq!(SelectorKind::Path.as_str(), "path");
        assert_eq!(SelectorKind::PathQualified.as_str(), "path_qualified");
        assert_eq!(SelectorKind::Name.as_str(), "name");
    }

    /// Every entity kind is a qualifier, generated rather than listed, plus exactly two aliases.
    #[test]
    fn the_qualifier_vocabulary_is_generated_from_the_entity_kinds() {
        for kind in EntityKind::ALL {
            assert_eq!(
                Qualifier::parse(kind.as_str()),
                Some(Qualifier::Kind(kind)),
                "{kind} must be a qualifier"
            );
        }
        assert_eq!(Qualifier::parse("symbol"), Some(Qualifier::Symbol));
        assert_eq!(Qualifier::parse("adr"), Some(Qualifier::Adr));
        assert_eq!(Qualifier::parse("banana"), None);
        assert_eq!(Qualifier::parse(""), None);
        assert_eq!(Qualifier::parse("Module"), None, "the vocabulary is exact");

        let listed = qualifiers();
        assert_eq!(
            listed.len(),
            EntityKind::ALL.len() + Qualifier::ALIASES.len()
        );
        for name in &listed {
            assert!(Qualifier::parse(name).is_some(), "{name} must parse back");
        }
        // An alias is never also a kind, so a qualifier has exactly one meaning.
        for alias in Qualifier::ALIASES {
            assert!(
                alias.parse::<EntityKind>().is_err(),
                "{alias} shadows a kind"
            );
        }
    }

    #[test]
    fn a_qualifier_is_only_read_before_the_first_path_separator() {
        assert_eq!(
            selector_shape("symbol:parse"),
            SelectorShape::Usable {
                qualifier: Some(Qualifier::Symbol),
                body: "parse",
            }
        );
        assert_eq!(
            selector_shape("file:docs/architecture.md"),
            SelectorShape::Usable {
                qualifier: Some(Qualifier::Kind(EntityKind::File)),
                body: "docs/architecture.md",
            }
        );
        // A colon below the root is part of the path, not a malformed qualifier.
        assert_eq!(
            selector_shape("docs/a:b.md"),
            SelectorShape::Usable {
                qualifier: None,
                body: "docs/a:b.md",
            }
        );
        // A colon after the `#` belongs to the symbol part.
        assert_eq!(
            selector_shape("src/a.ts#Foo:bar"),
            SelectorShape::Usable {
                qualifier: None,
                body: "src/a.ts#Foo:bar",
            }
        );
        assert_eq!(
            selector_shape("Circle.area"),
            SelectorShape::Usable {
                qualifier: None,
                body: "Circle.area",
            }
        );
    }

    #[test]
    fn a_malformed_selector_is_invalid_rather_than_a_bare_name() {
        assert_eq!(
            selector_shape("banana:foo"),
            SelectorShape::Invalid(InvalidSelector::UnknownQualifier)
        );
        assert_eq!(
            selector_shape(":foo"),
            SelectorShape::Invalid(InvalidSelector::EmptyQualifier)
        );
        assert_eq!(
            selector_shape("module:"),
            SelectorShape::Invalid(InvalidSelector::EmptyBody)
        );
        assert_eq!(
            selector_shape(""),
            SelectorShape::Invalid(InvalidSelector::EmptyBody)
        );
        assert_eq!(
            InvalidSelector::UnknownQualifier.as_str(),
            "unknown_qualifier"
        );
        assert_eq!(InvalidSelector::EmptyQualifier.as_str(), "empty_qualifier");
        assert_eq!(InvalidSelector::EmptyBody.as_str(), "empty_body");
    }

    /// The shape check refuses an escape whatever wrapping it arrives in.
    ///
    /// Including behind a qualifier: `file:/etc/passwd` must be refused, which a check that ran
    /// before the qualifier was stripped would miss.
    #[test]
    fn a_traversal_selector_is_refused_however_it_is_spelled() {
        for selector in [
            "../../etc/passwd",
            "../secrets.env",
            "/etc/passwd",
            "/etc/passwd#thing",
            "src/../../../etc/passwd#Circle.area",
            "\\\\server\\share",
            "./../x",
            "a/../b.ts",
            "file:/etc/passwd",
            "symbol:../../etc/passwd",
            // Repeated separators do not smuggle a `..` past the check.
            "src//../../etc/passwd",
            "//etc/passwd",
            // Backslash forms. On Unix `\` is not a separator, so `std::path::Components` sees
            // one component and the escape is invisible to it. Splitting on both separators is
            // what makes these refusals rather than misses.
            "..\\..\\windows\\system32",
            "a\\..\\b",
            "docs\\..\\..\\etc\\passwd",
            "\\etc\\passwd",
            // Mixed separators, which is how a naive check is usually defeated.
            "src\\../../etc/passwd",
            "src/..\\..\\etc",
        ] {
            assert_eq!(
                selector_shape(selector),
                SelectorShape::Refused(SelectorRefusal::Traversal),
                "{selector} must be refused"
            );
        }
        assert_eq!(SelectorRefusal::Traversal.as_str(), "path_refused");
    }

    /// A legal selector is never refused, including the shapes that merely look alarming.
    #[test]
    fn an_ordinary_selector_is_not_refused() {
        for selector in [
            "src/shapes.ts",
            "src/shapes.ts#Circle.area",
            "Circle.area",
            "meth_0123456789abcdef",
            "src/a.b.c/thing.ts",
            // `..` inside a segment is not a parent-directory component.
            "a..b.ts",
            "src/a..b/c.ts",
            "docs/..hidden/notes.md",
            // Unicode, including a path that is not ASCII at all.
            "docs/архитектура.md",
            "src/日本語.ts#クラス.メソッド",
            "café",
            // A repeated separator with nothing to smuggle is simply a path that will miss.
            "src//app.ts",
            // A dot segment is not an escape in **any** position. `./x` means `x`; refusing it
            // would tell a user their legal selector tried to leave the repository.
            "./x",
            "./docs/architecture.md",
            "a/./b.ts",
            "docs/./architecture.md",
            ".",
            // One legal Unix filename that happens to contain a backslash. There is no `..` in
            // it, so there is nothing to refuse — and a file named this could be indexed.
            "a\\b.ts",
            "weird\\name.ts",
        ] {
            assert!(
                matches!(selector_shape(selector), SelectorShape::Usable { .. }),
                "{selector} must be usable, got {:?}",
                selector_shape(selector)
            );
        }
    }
}
