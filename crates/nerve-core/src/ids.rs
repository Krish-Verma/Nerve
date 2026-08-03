//! Identity computation (ADR-0002).
//!
//! Every identifier is a BLAKE3 digest over a **domain-tagged canonical tuple**: the fields
//! joined with the ASCII unit separator `\x1f`, with the kind string first. The separator is
//! not a legal character in any field we produce, so the encoding is injective and two
//! different tuples cannot collide by concatenation.

use crate::error::NerveError;
use crate::vocab::{EntityKind, Relation, UnresolvedCategory};

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

/// `("document", project_id, rel_path)`
///
/// Mirrors [`module_id`]: a document is 1:1 with a file, and its identity is the path it lives
/// at, never its title. A title is prose and prose is rewritten.
pub fn document_id(project_id: &str, rel_path: &str) -> String {
    prefixed(
        EntityKind::Document,
        &[EntityKind::Document.as_str(), project_id, rel_path],
    )
}

/// Remove every byte below `0x20` from attacker-controlled text.
///
/// Heading text comes from a repository, which is untrusted input (THREAT-MODEL.md, A1). The
/// canonical tuple encoding is injective **only because no field can contain the separator**,
/// so a heading holding a literal `0x1f` would let its author choose where one tuple field ends
/// and the next begins, and thereby forge the identity of a section somewhere else in the same
/// document. Stripping the whole C0 range rather than just `0x1f` avoids relying on the rest of
/// the pipeline to be careful with the other control characters.
///
/// This is a *stripping* rule, not an escaping one: the result is used for identity only, never
/// for display, so it does not have to be reversible.
pub fn strip_control(text: &str) -> String {
    text.chars().filter(|c| (*c as u32) >= 0x20).collect()
}

/// `("section", project_id, rel_path, <heading chain…>, sibling_ordinal)`
///
/// `heading_path` is the chain of heading texts from the outermost enclosing heading down to
/// and including this section's own heading. Every segment enters the tuple as **its own
/// field**, and every segment is passed through [`strip_control`] first.
///
/// Both of those are load-bearing, and the second alone is not enough. Slice 5a's plan specified
/// a single `>`-joined field; that is forgeable, because `>` is ordinary heading text. In one
/// document,
///
/// ```text
/// # A>B          # A
/// ## C           ## B
///                ### C
/// ```
///
/// both `C` sections have the chain `A>B>C` and sibling ordinal 0, so a joined field collides
/// them — the very attack the control-character strip exists to prevent, carried by a printable
/// byte instead. Spreading the chain across tuple fields removes the class: the unit separator
/// cannot appear in a segment, so the encoding is injective in the number of segments as well
/// as in their contents.
///
/// `sibling_ordinal` disambiguates two sections with identical heading text under the same
/// parent. It is scoped to the parent, so inserting a section elsewhere in the document does not
/// churn the ids of unrelated sections.
pub fn section_id(
    project_id: &str,
    rel_path: &str,
    heading_path: &[&str],
    sibling_ordinal: u32,
) -> String {
    let sanitized: Vec<String> = heading_path
        .iter()
        .map(|part| strip_control(part))
        .collect();
    let ordinal = sibling_ordinal.to_string();
    let mut fields: Vec<&str> = Vec::with_capacity(heading_path.len() + 4);
    fields.push(EntityKind::Section.as_str());
    fields.push(project_id);
    fields.push(rel_path);
    fields.extend(sanitized.iter().map(String::as_str));
    fields.push(&ordinal);
    prefixed(EntityKind::Section, &fields)
}

/// `("coverage_run", project_id, rel_path, content_hash)`
///
/// A coverage run is one report file, and its identity is the path it was read from **plus the
/// bytes it contained** (ADR-0008). The content hash is in the tuple deliberately, and it is the
/// one identity in Nerve that carries content: a report is not a durable artifact whose meaning
/// survives an edit the way a function's does — it is a measurement, and re-running the suite
/// produces a *different* measurement that happens to live at the same path. Excluding the hash
/// would let a new run silently inherit the old run's edges.
pub fn coverage_run_id(project_id: &str, rel_path: &str, content_hash: &str) -> String {
    prefixed(
        EntityKind::CoverageRun,
        &[
            EntityKind::CoverageRun.as_str(),
            project_id,
            rel_path,
            content_hash,
        ],
    )
}

/// `("endpoint", project_id, module_rel_path, endpoint_kind, method, path)`
///
/// The declaring module is in the tuple because Slice 10 records the **declared** address, not the
/// deployed one: two modules that both declare `GET /items` are two declarations, and merging them
/// would assert that one deployed endpoint exists where the evidence says two files each register
/// something. Within one module the same address declared twice *is* one address — that is the
/// ambiguity `py-framework` counts as `duplicate-address` and reports with both edges kept.
///
/// `method` and `path` are separate fields rather than one joined address for the reason
/// [`section_id`] spreads its heading chain: a route path is repository text an attacker writes,
/// and a joined field would let `("GET", "/a /b")` and `("GET /a", "/b")` collide. Both are passed
/// through [`strip_control`] first, for the same reason a heading is.
pub fn endpoint_id(
    project_id: &str,
    module_rel_path: &str,
    endpoint_kind: crate::vocab::EndpointKind,
    method: &str,
    path: &str,
) -> String {
    let method = strip_control(method);
    let path = strip_control(path);
    prefixed(
        EntityKind::Endpoint,
        &[
            EntityKind::Endpoint.as_str(),
            project_id,
            module_rel_path,
            endpoint_kind.as_str(),
            &method,
            &path,
        ],
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

/// `("unresolved", project_id, importer_rel_path, category, raw_name)`
///
/// `category` is load-bearing, not descriptive. A file containing both
/// `import { parse } from 'parse'` and a call to `parse()` has two distinct unresolved things
/// with the same name; without the discriminator they would collide onto one entity and Nerve
/// would silently claim a module and a value are the same thing.
pub fn unresolved_id(
    project_id: &str,
    importer_rel_path: &str,
    category: UnresolvedCategory,
    raw_name: &str,
) -> String {
    prefixed(
        EntityKind::Unresolved,
        &[
            EntityKind::Unresolved.as_str(),
            project_id,
            importer_rel_path,
            category.as_str(),
            raw_name,
        ],
    )
}

/// `blake3(entity_id, rel_path, start_byte, end_byte)`, full hex.
///
/// Physical identity: one row per appearance of an entity at a byte span in a file.
///
/// **The repository state is deliberately absent** (ADR-0006). An occurrence is a location
/// fact, and a location does not depend on which index run happened to notice it. Including the
/// state made every re-index rewrite every surviving row and every index entry over it — an
/// O(repository) write for an O(change) edit. What the file said at the time is recorded by
/// `occurrence.content_hash`, which is what freshness is computed from.
pub fn occurrence_id(
    entity_id: &str,
    rel_path: &str,
    start_byte: usize,
    end_byte: usize,
) -> String {
    let start = start_byte.to_string();
    let end = end_byte.to_string();
    canonical_digest(&[entity_id, rel_path, &start, &end])
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
            (
                unresolved_id(PID, "src/a.ts", UnresolvedCategory::Module, "./missing"),
                "unres",
            ),
            (document_id(PID, "docs/README.md"), "doc"),
            (section_id(PID, "docs/README.md", &["Title"], 0), "sect"),
            (coverage_run_id(PID, "coverage/lcov.info", "abc"), "cov"),
            (
                endpoint_id(
                    PID,
                    "api/routes.py",
                    crate::vocab::EndpointKind::HttpRoute,
                    "GET",
                    "/users",
                ),
                "endp",
            ),
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
    fn unresolved_modules_and_values_with_the_same_name_are_distinct() {
        // A file that both imports from `parse` and calls `parse()` must not merge the two.
        let module = unresolved_id(PID, "src/a.ts", UnresolvedCategory::Module, "parse");
        let value = unresolved_id(PID, "src/a.ts", UnresolvedCategory::Value, "parse");
        assert_ne!(module, value);
        // The discriminator must not be forgeable by moving text between fields either.
        assert_ne!(
            unresolved_id(PID, "src/a.ts", UnresolvedCategory::Value, "parse"),
            unresolved_id(PID, "src/a.tsvalue", UnresolvedCategory::Value, "parse")
        );
    }

    /// The document counterpart of
    /// [`unresolved_modules_and_values_with_the_same_name_are_distinct`]: heading text is
    /// attacker-controlled, and the attack is to make one section's tuple *become* another's.
    ///
    /// Both separators are tried. `0x1f` is the canonical field separator and is stripped;
    /// `>` is the separator the Slice 5a plan proposed for the joined heading path, and it is
    /// printable, so it cannot be stripped — the chain must therefore not be joined at all.
    #[test]
    fn a_heading_cannot_forge_another_sections_identity() {
        // 1. A literal unit separator inside a heading must not merge two tuple fields.
        let honest = section_id(PID, "docs/a.md", &["Parent", "Child"], 0);
        let forged = section_id(PID, "docs/a.md", &["Parent\u{1f}Child"], 0);
        assert_ne!(
            honest, forged,
            "a 0x1f in a heading forged a nested section"
        );

        // The strip must not be reversible either: two different forgeries that only differ in
        // control characters land on the same id as the plain text they decorate, which is the
        // point — the control characters are simply not part of identity.
        assert_eq!(
            section_id(PID, "docs/a.md", &["Plain"], 0),
            section_id(PID, "docs/a.md", &["Pl\u{1f}ain"], 0)
        );
        assert_eq!(strip_control("a\u{0}b\u{1f}c\n"), "abc");

        // 2. A `>` inside a heading must not forge a differently nested section. `#A>B / ##C`
        //    and `#A / ##B / ###C` are different structures in the same document.
        assert_ne!(
            section_id(PID, "docs/a.md", &["A>B", "C"], 0),
            section_id(PID, "docs/a.md", &["A", "B", "C"], 0),
            "a `>` in a heading forged a section at another nesting level"
        );

        // 3. The ordinal is a field of its own and cannot be absorbed into the heading either.
        assert_ne!(
            section_id(PID, "docs/a.md", &["Repeat"], 1),
            section_id(PID, "docs/a.md", &["Repeat", "1"], 0)
        );
    }

    #[test]
    fn section_id_varies_with_every_tuple_field() {
        let base = section_id(PID, "docs/a.md", &["Top", "Inner"], 0);
        for other in [
            section_id("other", "docs/a.md", &["Top", "Inner"], 0),
            section_id(PID, "docs/b.md", &["Top", "Inner"], 0),
            section_id(PID, "docs/a.md", &["Top", "Other"], 0),
            section_id(PID, "docs/a.md", &["Other", "Inner"], 0),
            section_id(PID, "docs/a.md", &["Inner"], 0),
            section_id(PID, "docs/a.md", &["Top", "Inner"], 1),
        ] {
            assert_ne!(base, other);
        }
    }

    /// A document is identified by its path, exactly as a module is, and the two must not
    /// collide even though their tuples carry the same path.
    #[test]
    fn document_and_module_identities_are_domain_separated() {
        assert_ne!(
            document_id(PID, "docs/README.md")[4..],
            module_id(PID, "docs/README.md")[4..]
        );
        assert_ne!(document_id(PID, "docs/a.md"), document_id(PID, "docs/b.md"));
    }

    /// Two runs of the same suite land at the same path. They are different measurements, and
    /// the new one must not inherit the old one's edges.
    #[test]
    fn a_coverage_run_is_identified_by_its_path_and_its_bytes() {
        let base = coverage_run_id(PID, "coverage/lcov.info", "h1");
        for other in [
            coverage_run_id("other", "coverage/lcov.info", "h1"),
            coverage_run_id(PID, "coverage/other.info", "h1"),
            coverage_run_id(PID, "coverage/lcov.info", "h2"),
        ] {
            assert_ne!(base, other);
        }
        assert_eq!(base, coverage_run_id(PID, "coverage/lcov.info", "h1"));
        // Four fields, so the tuple cannot be forged by moving text across a separator.
        assert_ne!(
            coverage_run_id(PID, "coverage/lcov.info", "h1"),
            coverage_run_id(PID, "coverage/lcov.infoh1", "")
        );
    }

    /// A route path is repository text an attacker writes, so the endpoint tuple gets the same
    /// treatment a heading does: every field separate, control characters stripped.
    #[test]
    fn an_endpoint_address_cannot_forge_another_endpoints_identity() {
        use crate::vocab::EndpointKind::HttpRoute;

        let base = endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/users");
        for other in [
            endpoint_id("other", "api/routes.py", HttpRoute, "GET", "/users"),
            endpoint_id(PID, "api/other.py", HttpRoute, "GET", "/users"),
            endpoint_id(PID, "api/routes.py", HttpRoute, "POST", "/users"),
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/items"),
        ] {
            assert_ne!(base, other);
        }
        assert_eq!(
            base,
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/users"),
            "the same declared address in the same module is the same endpoint"
        );

        // A space is ordinary in neither field, but it is the character the canonical name joins
        // them with, so a joined tuple would collide these two.
        assert_ne!(
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/a /b"),
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET /a", "/b")
        );
        // And the unit separator cannot be smuggled through either field.
        assert_ne!(
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/users"),
            endpoint_id(
                PID,
                "api/routes.py\u{1f}http_route",
                HttpRoute,
                "GET",
                "/users"
            )
        );
        assert_eq!(
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/us\u{1f}ers"),
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/users")
        );

        // An endpoint and a module at the same path are domain-separated.
        assert_ne!(
            endpoint_id(PID, "api/routes.py", HttpRoute, "GET", "/users")[5..],
            module_id(PID, "api/routes.py")[4..]
        );
    }

    #[test]
    fn unresolved_id_varies_with_importer_and_name() {
        let base = unresolved_id(PID, "src/a.ts", UnresolvedCategory::Value, "parse");
        assert_ne!(
            base,
            unresolved_id(PID, "src/b.ts", UnresolvedCategory::Value, "parse")
        );
        assert_ne!(
            base,
            unresolved_id(PID, "src/a.ts", UnresolvedCategory::Value, "other")
        );
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
            occurrence_id("e", "a.ts", 1, 2),
            occurrence_id("e", "a.ts", 1, 2)
        );
        assert_eq!(
            assertion_id("a", Relation::Defines, "b"),
            assertion_id("a", Relation::Defines, "b")
        );
    }

    /// ADR-0006. The repository state is not an input, and every remaining field is.
    #[test]
    fn occurrence_id_is_state_independent_and_varies_with_every_field() {
        let base = occurrence_id("e", "a.ts", 1, 2);
        for other in [
            occurrence_id("other", "a.ts", 1, 2),
            occurrence_id("e", "b.ts", 1, 2),
            occurrence_id("e", "a.ts", 0, 2),
            occurrence_id("e", "a.ts", 1, 3),
        ] {
            assert_ne!(base, other);
        }
        // The tuple is four fields, so it cannot be forged by moving text across a separator.
        assert_ne!(occurrence_id("e", "a.ts", 1, 2), canonical_digest(&["e"]));
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
