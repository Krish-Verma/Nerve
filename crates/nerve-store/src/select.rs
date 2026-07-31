//! Turning what a human typed into exactly one entity, or refusing to.
//!
//! Resolution order, first stage that matches anything wins:
//!
//! 1. an exact `entity_id`
//! 2. `<rel_path>` — the `Module` entity for that file
//! 3. `<rel_path>#<qualified_name>` — a symbol in that file
//! 4. a bare `<qualified_name>` or `<name>`, unique across the repository
//!
//! A stage that matches more than one entity is **ambiguous** and stops resolution there. It
//! does not fall through to the next stage and it never picks one: silently choosing is the
//! failure mode that makes a tool untrustworthy in exactly the situation where the user most
//! needs it to be right.

use rusqlite::{params, Connection};

use nerve_core::vocab::EntityKind;

use crate::error::Result;
use crate::query::{search_entities, SearchHit};

/// How many `nerve search` suggestions accompany a selector that matched nothing.
pub const SUGGESTION_LIMIT: usize = 5;

/// Shortest prefix a suggestion search will fall back to.
pub const MIN_SUGGESTION_PREFIX: usize = 3;

/// SQL list of the kinds whose `scope_path` names enclosing symbols.
///
/// For every other kind `scope_path` holds a repository path — a module's own file, a file's
/// parent directory, the importer that failed to resolve — so folding it into a dotted
/// qualified name would produce a name that does not exist anywhere in the source. The list is
/// generated from the closed vocabulary so it cannot drift from [`EntityKind::is_symbol`].
fn symbol_kinds_sql() -> String {
    EntityKind::ALL
        .iter()
        .filter(|kind| kind.is_symbol())
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

    /// `scope.name` for a scoped symbol, `name` otherwise.
    ///
    /// A `Module`'s `scope_path` is its own file path and an `Unresolved`'s is the importer
    /// that failed to resolve it, so neither is folded in: the result would be a dotted name
    /// that appears nowhere in the source.
    pub fn qualified_name(&self) -> String {
        let scoped = self
            .kind
            .parse::<EntityKind>()
            .map(EntityKind::is_symbol)
            .unwrap_or(false);
        if self.scope_path.is_empty() || !scoped {
            self.name.clone()
        } else {
            format!("{}.{}", self.scope_path, self.name)
        }
    }

    /// `file:line`, or `-` when the entity has no occurrence.
    pub fn location(&self) -> String {
        match (&self.file_path, self.start_line) {
            (Some(file), Some(line)) => format!("{file}:{line}"),
            (Some(file), None) => file.clone(),
            _ => "-".to_string(),
        }
    }
}

/// Which resolution stage produced a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind {
    /// An exact `entity_id`.
    EntityId,
    /// A repository-relative path naming a module.
    ModulePath,
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
            SelectorKind::ModulePath => "module_path",
            SelectorKind::PathQualified => "path_qualified",
            SelectorKind::Name => "name",
        }
    }
}

/// What resolving a selector produced.
#[derive(Debug, Clone)]
pub enum Selection {
    /// Exactly one entity matched.
    Resolved {
        /// The entity.
        entity: Box<EntityRef>,
        /// Stage that matched.
        matched_by: SelectorKind,
    },
    /// More than one entity matched at the same stage. Nothing is chosen.
    Ambiguous {
        /// Every candidate, in a stable order.
        candidates: Vec<EntityRef>,
        /// Stage that matched.
        matched_by: SelectorKind,
    },
    /// Nothing matched at any stage.
    NotFound {
        /// Nearest `nerve search` hits, as a starting point.
        suggestions: Vec<SearchHit>,
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

/// Resolve a selector to exactly one entity, a candidate list, or nothing.
pub fn resolve_selector(conn: &Connection, selector: &str) -> Result<Selection> {
    for stage in [
        SelectorKind::EntityId,
        SelectorKind::ModulePath,
        SelectorKind::PathQualified,
        SelectorKind::Name,
    ] {
        let mut candidates = match stage {
            SelectorKind::EntityId => lookup(conn, "e.entity_id = ?1", params![selector])?,
            SelectorKind::ModulePath => lookup(
                conn,
                "e.kind = 'module' AND e.scope_path = ?1",
                params![selector],
            )?,
            SelectorKind::PathQualified => path_qualified(conn, selector)?,
            SelectorKind::Name => lookup(
                conn,
                &format!(
                    "e.name = ?1 OR (e.scope_path <> '' AND e.kind IN ({})
                                     AND e.scope_path || '.' || e.name = ?1)",
                    symbol_kinds_sql()
                ),
                params![selector],
            )?,
        };
        let matched_by = stage;
        match candidates.len() {
            0 => continue,
            1 => {
                return Ok(Selection::Resolved {
                    entity: Box::new(candidates.remove(0)),
                    matched_by,
                })
            }
            _ => {
                return Ok(Selection::Ambiguous {
                    candidates,
                    matched_by,
                })
            }
        }
    }

    Ok(Selection::NotFound {
        suggestions: suggestions(conn, selector)?,
    })
}

/// Nearest `nerve search` hits for a selector that matched nothing.
///
/// FTS5 gives prefix matching, not fuzzy matching, so a typo in the final characters — which
/// is where typos live — finds nothing at all. Retrying with progressively shorter prefixes
/// turns `normalise` into `normali`, which does find `normalize`. The floor keeps the list a
/// suggestion rather than a dump of whatever the repository happens to contain.
fn suggestions(conn: &Connection, selector: &str) -> Result<Vec<SearchHit>> {
    // Search the symbol part of a `path#name` selector: the path part would be AND-ed into the
    // FTS expression and suppress every useful hit.
    let needle = selector.rsplit('#').next().unwrap_or(selector);
    let mut prefix: Vec<char> = needle.chars().collect();
    while prefix.len() >= MIN_SUGGESTION_PREFIX {
        let candidate: String = prefix.iter().collect();
        let hits = search_entities(conn, &candidate, None, SUGGESTION_LIMIT)?;
        if !hits.is_empty() {
            return Ok(hits);
        }
        prefix.pop();
    }
    Ok(Vec::new())
}

fn path_qualified(conn: &Connection, selector: &str) -> Result<Vec<EntityRef>> {
    let Some((rel_path, qualified)) = selector.split_once('#') else {
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
                                  AND e.scope_path || '.' || e.name = ?2))",
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

        let module = EntityRef {
            kind: "module".into(),
            name: "shapes".into(),
            scope_path: "src/shapes.ts".into(),
            ..method.clone()
        };
        assert_eq!(module.qualified_name(), "shapes");

        let unresolved = EntityRef {
            kind: "unresolved".into(),
            name: "console.log".into(),
            scope_path: "src/globals.ts".into(),
            ..method.clone()
        };
        assert_eq!(unresolved.qualified_name(), "console.log");

        let orphan = EntityRef {
            file_path: None,
            start_line: None,
            scope_path: String::new(),
            ..method
        };
        assert_eq!(orphan.qualified_name(), "area");
        assert_eq!(orphan.location(), "-");
    }

    #[test]
    fn only_symbol_kinds_fold_their_scope_into_a_qualified_name() {
        let list = symbol_kinds_sql();
        for kind in EntityKind::ALL {
            let quoted = format!("'{}'", kind.as_str());
            assert_eq!(list.contains(&quoted), kind.is_symbol(), "{kind} in {list}");
        }
    }

    #[test]
    fn selector_kind_names_are_the_json_contract() {
        assert_eq!(SelectorKind::EntityId.as_str(), "entity_id");
        assert_eq!(SelectorKind::ModulePath.as_str(), "module_path");
        assert_eq!(SelectorKind::PathQualified.as_str(), "path_qualified");
        assert_eq!(SelectorKind::Name.as_str(), "name");
    }
}
