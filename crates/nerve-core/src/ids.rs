//! Identity computation (ADR-0002).
//!
//! Every identifier is a BLAKE3 digest over a **domain-tagged canonical tuple**: the fields
//! joined with the ASCII unit separator `\x1f`, with the kind string first. The separator is
//! not a legal character in any field we produce, so the encoding is injective and two
//! different tuples cannot collide by concatenation.

use crate::error::NerveError;
use crate::vocab::{EntityKind, Relation};

/// ASCII unit separator, the canonical tuple field delimiter.
pub const UNIT_SEPARATOR: u8 = 0x1f;

/// Number of hex characters retained in an [`EntityKind`]-prefixed entity id (128 bits).
pub const ENTITY_ID_HEX_LEN: usize = 32;

/// BLAKE3 over the canonical tuple encoding, returned as full 64-character hex.
///
/// This is the single place the canonical tuple encoding is defined. Every identifier in the
/// system routes through it.
pub fn canonical_digest(fields: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            hasher.update(&[UNIT_SEPARATOR]);
        }
        hasher.update(field.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// BLAKE3 of arbitrary bytes as full hex. Used for file content hashes.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn prefixed(kind: EntityKind, fields: &[&str]) -> String {
    let digest = canonical_digest(fields);
    format!("{}_{}", kind.prefix(), &digest[..ENTITY_ID_HEX_LEN])
}

/// `("repository", project_id)`
pub fn repository_id(project_id: &str) -> String {
    prefixed(
        EntityKind::Repository,
        &[EntityKind::Repository.as_str(), project_id],
    )
}

/// `("directory", project_id, rel_path)`
pub fn directory_id(project_id: &str, rel_path: &str) -> String {
    prefixed(
        EntityKind::Directory,
        &[EntityKind::Directory.as_str(), project_id, rel_path],
    )
}

/// `("file", project_id, rel_path)`
pub fn file_id(project_id: &str, rel_path: &str) -> String {
    prefixed(
        EntityKind::File,
        &[EntityKind::File.as_str(), project_id, rel_path],
    )
}

/// `("module", project_id, rel_path)`
pub fn module_id(project_id: &str, rel_path: &str) -> String {
    prefixed(
        EntityKind::Module,
        &[EntityKind::Module.as_str(), project_id, rel_path],
    )
}

/// `("<kind>", project_id, module_rel_path, scope_path, name, disambiguator)`
///
/// Valid only for [`EntityKind::is_symbol`] kinds.
///
/// Body content is deliberately excluded: editing a function must not change its identity.
pub fn symbol_id(
    kind: EntityKind,
    project_id: &str,
    module_rel_path: &str,
    scope_path: &str,
    name: &str,
    disambiguator: u32,
) -> Result<String, NerveError> {
    if !kind.is_symbol() {
        return Err(NerveError::InvalidIdentityKind {
            kind: kind.as_str(),
            constructor: "symbol_id",
        });
    }
    let disambiguator = disambiguator.to_string();
    Ok(prefixed(
        kind,
        &[
            kind.as_str(),
            project_id,
            module_rel_path,
            scope_path,
            name,
            &disambiguator,
        ],
    ))
}

/// `("unresolved", project_id, importer_rel_path, raw_specifier)`
pub fn unresolved_id(project_id: &str, importer_rel_path: &str, raw_specifier: &str) -> String {
    prefixed(
        EntityKind::Unresolved,
        &[
            EntityKind::Unresolved.as_str(),
            project_id,
            importer_rel_path,
            raw_specifier,
        ],
    )
}

/// `blake3(entity_id, state_id, rel_path, start_byte, end_byte)`, full hex.
///
/// Physical identity: one row per appearance of an entity in a specific repository state.
pub fn occurrence_id(
    entity_id: &str,
    state_id: &str,
    rel_path: &str,
    start_byte: usize,
    end_byte: usize,
) -> String {
    let start = start_byte.to_string();
    let end = end_byte.to_string();
    canonical_digest(&[entity_id, state_id, rel_path, &start, &end])
}

/// `blake3(source_entity_id, relation, target_entity_id)`, full hex.
///
/// Claim identity: deduplicates claims so many observations can support one claim.
pub fn assertion_id(source_entity_id: &str, relation: Relation, target_entity_id: &str) -> String {
    canonical_digest(&[source_entity_id, relation.as_str(), target_entity_id])
}

/// BLAKE3 Merkle over sorted `(rel_path, content_hash)` pairs.
///
/// The caller supplies pairs in any order; this function sorts by `rel_path` so the result is
/// a pure function of the indexed file set and its contents.
pub fn content_merkle(pairs: &mut [(String, String)]) -> String {
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (rel_path, hash) in pairs.iter() {
        hasher.update(rel_path.as_bytes());
        hasher.update(&[UNIT_SEPARATOR]);
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PID: &str = "00000000000000000000000000000001";

    #[test]
    fn entity_ids_have_kind_prefix_and_fixed_length() {
        let cases = [
            (repository_id(PID), "repo"),
            (directory_id(PID, "src"), "dir"),
            (file_id(PID, "src/math.ts"), "file"),
            (module_id(PID, "src/math.ts"), "mod"),
            (unresolved_id(PID, "src/a.ts", "./missing"), "unres"),
        ];
        for (id, prefix) in cases {
            assert!(id.starts_with(&format!("{prefix}_")), "{id} lacks {prefix}");
            let hex = &id[prefix.len() + 1..];
            assert_eq!(hex.len(), ENTITY_ID_HEX_LEN);
            assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn symbol_id_rejects_non_symbol_kinds() {
        assert!(symbol_id(EntityKind::File, PID, "a.ts", "", "x", 0).is_err());
        assert!(symbol_id(EntityKind::Function, PID, "a.ts", "", "x", 0).is_ok());
    }

    #[test]
    fn symbol_id_varies_with_every_tuple_field() {
        let base = symbol_id(EntityKind::Function, PID, "a.ts", "outer", "inner", 0).unwrap();
        let others = [
            symbol_id(EntityKind::Method, PID, "a.ts", "outer", "inner", 0).unwrap(),
            symbol_id(EntityKind::Function, "other", "a.ts", "outer", "inner", 0).unwrap(),
            symbol_id(EntityKind::Function, PID, "b.ts", "outer", "inner", 0).unwrap(),
            symbol_id(EntityKind::Function, PID, "a.ts", "elsewhere", "inner", 0).unwrap(),
            symbol_id(EntityKind::Function, PID, "a.ts", "outer", "other", 0).unwrap(),
            symbol_id(EntityKind::Function, PID, "a.ts", "outer", "inner", 1).unwrap(),
        ];
        for other in others {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn unit_separator_prevents_field_concatenation_collisions() {
        // ("a", "bc") must not equal ("ab", "c").
        assert_ne!(
            canonical_digest(&["a", "bc"]),
            canonical_digest(&["ab", "c"])
        );
    }

    #[test]
    fn project_id_domain_separates_kinds_with_identical_paths() {
        assert_ne!(
            file_id(PID, "src/math.ts")[5..],
            module_id(PID, "src/math.ts")[4..]
        );
    }

    #[test]
    fn ids_are_deterministic() {
        assert_eq!(
            symbol_id(EntityKind::Class, PID, "a.ts", "", "C", 3).unwrap(),
            symbol_id(EntityKind::Class, PID, "a.ts", "", "C", 3).unwrap()
        );
        assert_eq!(
            occurrence_id("e", "s", "a.ts", 1, 2),
            occurrence_id("e", "s", "a.ts", 1, 2)
        );
        assert_eq!(
            assertion_id("a", Relation::Defines, "b"),
            assertion_id("a", Relation::Defines, "b")
        );
    }

    #[test]
    fn assertion_id_is_relation_sensitive_and_directed() {
        assert_ne!(
            assertion_id("a", Relation::Defines, "b"),
            assertion_id("a", Relation::Contains, "b")
        );
        assert_ne!(
            assertion_id("a", Relation::Defines, "b"),
            assertion_id("b", Relation::Defines, "a")
        );
    }

    #[test]
    fn content_merkle_is_order_independent_and_content_sensitive() {
        let mut forward = vec![
            ("a.ts".to_string(), "h1".to_string()),
            ("b.ts".to_string(), "h2".to_string()),
        ];
        let mut reverse = vec![
            ("b.ts".to_string(), "h2".to_string()),
            ("a.ts".to_string(), "h1".to_string()),
        ];
        assert_eq!(content_merkle(&mut forward), content_merkle(&mut reverse));

        let mut changed = vec![
            ("a.ts".to_string(), "h1".to_string()),
            ("b.ts".to_string(), "h3".to_string()),
        ];
        assert_ne!(content_merkle(&mut forward), content_merkle(&mut changed));
    }
}
