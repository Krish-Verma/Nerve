//! The per-module extraction cache, and what it is for.
//!
//! Incremental indexing re-extracts one file. Extracting a file correctly needs information
//! about *other* files: [`crate::exports::ExportIndex`] spans the whole corpus, and
//! `app.ts` resolves `helper` only if the export maps of `barrel.ts` and `impl.ts` are known.
//! Those maps are inputs to extraction, not outputs of it, so they cannot be read back out of
//! the graph — the persisted `EXPORTS` edges are one hop deep, while the closure is not.
//!
//! [`ModuleFacts`] is therefore the minimum an unchanged module must remember so that a changed
//! module can be re-extracted without re-parsing it:
//!
//! - its **export map** (`exported name -> entity id`), already closure-free and final;
//! - its **re-export statements**, kept as *raw specifiers* rather than resolved paths, because
//!   adding or deleting a file can change what a specifier resolves to without the module
//!   itself changing at all;
//! - its **import specifiers**, for the same reason — they are what detects the case where a
//!   previously unresolved import starts resolving;
//! - its **symbols with a body digest**, which is the evidence an `IdentityLink` proposal rests
//!   on after a file moves;
//! - its **per-file counters**, so that `nerve index` can still report whole-repository totals
//!   after re-extracting a fraction of the repository.
//!
//! No source text is stored: identifiers, specifiers, entity ids, tags, and BLAKE3 digests only
//! (SECURITY.md, "no source text at rest"). A body digest is a hash, not the body.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::EntityKind;

use crate::error::{IndexError, Result};
use crate::extract::{ExportTarget, LocalExport, ModuleExtraction, ReExport, SymbolDef};
use crate::refs::ReferenceExtraction;

/// A re-export statement, kept in source order and unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedReExport {
    /// Specifier exactly as written.
    pub raw_specifier: String,
    /// Named re-exports as `(name, alias)`; `None` means `export *`.
    pub names: Option<Vec<(String, Option<String>)>>,
}

/// A declared symbol, reduced to what identity-link evidence needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedSymbol {
    /// Entity id it had in the module it was cached from.
    pub entity_id: String,
    /// Entity kind name.
    pub kind: String,
    /// Declared name.
    pub name: String,
    /// Enclosing lexical scope.
    pub scope_path: String,
    /// BLAKE3 of the declaration's source bytes. Never the bytes themselves.
    pub body_hash: String,
}

impl CachedSymbol {
    /// The tuple a move proposal matches on: shape, not name alone (ADR-0002).
    pub fn identity_key(&self) -> (&str, &str, &str, &str) {
        (
            self.kind.as_str(),
            self.name.as_str(),
            self.scope_path.as_str(),
            self.body_hash.as_str(),
        )
    }
}

/// Per-file tallies that `nerve index` reports over the whole repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedCounters {
    /// `import(expr)` calls that named no specifier.
    pub dynamic_imports_without_specifier: usize,
    /// Whether the parse produced any ERROR node.
    pub has_syntax_error: bool,
    /// Call and heritage sites whose form Nerve does not model.
    pub unmodelled_call_sites: usize,
    /// Breakdown of the above by form tag.
    pub unmodelled_by_form: BTreeMap<String, usize>,
}

/// Per-document tallies, kept for the same reason [`CachedCounters`] is: a run that re-extracts
/// one file must still be able to report what the whole repository's documents contain.
///
/// Holds counts and form tags only — no heading text, no prose, no source at rest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentCounters {
    /// Sections the document contributed.
    pub sections: usize,
    /// Whether the document was recognised as an ADR.
    pub is_adr: bool,
    /// Constructs the Markdown scanner refused, by form tag.
    pub unsupported: BTreeMap<String, usize>,
    /// Every link destination the document wrote, deduplicated and sorted.
    ///
    /// The document counterpart of [`ModuleFacts::import_specifiers`], cached for exactly the
    /// same reason: adding, deleting or moving a file changes what a destination resolves to
    /// without the document itself changing at all, and the comparison must not require
    /// re-scanning every document to notice. The **whole** destination is kept, fragment
    /// included, because the fragment is what says whether the document depends on the target
    /// file's contents as well as on its existence.
    ///
    /// `serde(default)` so a payload written by Slice 5a still parses. An unreadable payload is
    /// a cache miss, and a cache miss re-extracts the repository for nothing.
    #[serde(default)]
    pub destinations: Vec<String>,
}

/// Everything an unchanged module must remember for another module to be re-extracted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFacts {
    /// Exported name -> entity id, for symbols this module itself defines.
    pub exports: BTreeMap<String, String>,
    /// Re-export statements, in source order.
    pub re_exports: Vec<CachedReExport>,
    /// Every import specifier the module writes, deduplicated and sorted.
    pub import_specifiers: Vec<String>,
    /// Declared symbols, in source order.
    pub symbols: Vec<CachedSymbol>,
    /// Per-file tallies.
    pub counters: CachedCounters,
    /// Per-document tallies. Empty for source files.
    ///
    /// `serde(default)` so a cache payload written before Slice 5a still parses: an unreadable
    /// payload is a cache miss, and a cache miss re-extracts the whole repository for nothing.
    #[serde(default)]
    pub document: DocumentCounters,
}

impl ModuleFacts {
    /// The cache entry for a document. It imports nothing and exports nothing, so every
    /// cross-module field is empty by construction rather than by omission.
    pub fn from_document(extraction: &crate::docs::DocumentExtraction) -> ModuleFacts {
        ModuleFacts {
            document: DocumentCounters {
                sections: extraction.sections.len(),
                is_adr: extraction.adr.is_adr,
                unsupported: extraction.unsupported.clone(),
                destinations: crate::docref::cached_destinations(&extraction.links),
            },
            ..ModuleFacts::default()
        }
    }
}

/// Resolve a module's local export map exactly the way the graph builder and the export index
/// both do, so that a cached map and a freshly computed one cannot drift apart.
pub fn local_export_map(extraction: &ModuleExtraction) -> BTreeMap<String, String> {
    let mut exports = BTreeMap::new();
    for export in &extraction.local_exports {
        let entity_id = match &export.target {
            ExportTarget::Symbol(position) => Some(extraction.symbols[*position].entity_id.clone()),
            ExportTarget::LocalName(name) => extraction
                .top_level_symbol(name)
                .map(|position| extraction.symbols[position].entity_id.clone()),
        };
        if let Some(entity_id) = entity_id {
            exports.insert(export.exported_name.clone(), entity_id);
        }
    }
    exports
}

/// Reduce a freshly parsed module to the slice [`crate::exports::ExportIndex::build`] reads.
///
/// The export index is built from every module in the repository, so a run that re-parses one
/// file still needs an entry for all the others. Reconstructed entries come from
/// [`ModuleFacts::as_export_source`]; this is the same reduction applied to a module that *was*
/// parsed, so the index never holds a full `ModuleExtraction` for a module it does not need one
/// for. Cloning them instead would double peak memory on a full index for no benefit.
///
/// Both paths resolve exports through [`local_export_map`], so a parsed entry and a cached one
/// cannot disagree.
pub fn export_source_of(extraction: &ModuleExtraction) -> ModuleExtraction {
    reduced_export_source(
        &extraction.rel_path,
        &local_export_map(extraction),
        extraction.re_exports.iter().map(|re_export| ReExport {
            raw_specifier: re_export.raw_specifier.clone(),
            names: re_export.names.clone(),
            span: Span::NONE,
        }),
    )
}

/// Shared construction for both the parsed and the cached reduction.
fn reduced_export_source(
    rel_path: &str,
    exports: &BTreeMap<String, String>,
    re_exports: impl Iterator<Item = ReExport>,
) -> ModuleExtraction {
    let mut symbols = Vec::with_capacity(exports.len());
    let mut local_exports = Vec::with_capacity(exports.len());
    for (exported_name, entity_id) in exports {
        local_exports.push(LocalExport {
            exported_name: exported_name.clone(),
            target: ExportTarget::Symbol(symbols.len()),
            span: Span::NONE,
        });
        symbols.push(SymbolDef {
            entity_id: entity_id.clone(),
            kind: EntityKind::Function,
            name: String::new(),
            scope_path: String::new(),
            disambiguator: 0,
            span: Span::NONE,
            meta: None,
            owner_class: None,
        });
    }

    ModuleExtraction {
        rel_path: rel_path.to_string(),
        symbols,
        local_exports,
        re_exports: re_exports.collect(),
        ..ModuleExtraction::default()
    }
}

/// BLAKE3 over the declaration's source bytes.
///
/// A span that does not lie on a character boundary or runs past the end of the file yields an
/// empty digest input rather than a panic; extraction spans come from the parser and should
/// always be valid, but a hash is not worth a crash.
fn body_hash(source: &str, span: Span) -> String {
    let bytes = source.as_bytes();
    if span.end_byte > bytes.len() || span.start_byte > span.end_byte {
        return ids::content_hash(&[]);
    }
    ids::content_hash(&bytes[span.start_byte..span.end_byte])
}

impl ModuleFacts {
    /// Derive the cache entry from a freshly extracted module.
    pub fn from_extraction(
        extraction: &ModuleExtraction,
        references: &ReferenceExtraction,
        source: &str,
    ) -> ModuleFacts {
        let mut import_specifiers: BTreeSet<String> = BTreeSet::new();
        for import in &extraction.imports {
            import_specifiers.insert(import.raw_specifier.clone());
        }
        for re_export in &extraction.re_exports {
            import_specifiers.insert(re_export.raw_specifier.clone());
        }

        ModuleFacts {
            exports: local_export_map(extraction),
            re_exports: extraction
                .re_exports
                .iter()
                .map(|re_export| CachedReExport {
                    raw_specifier: re_export.raw_specifier.clone(),
                    names: re_export.names.clone(),
                })
                .collect(),
            import_specifiers: import_specifiers.into_iter().collect(),
            symbols: extraction
                .symbols
                .iter()
                .map(|symbol| CachedSymbol {
                    entity_id: symbol.entity_id.clone(),
                    kind: symbol.kind.as_str().to_string(),
                    name: symbol.name.clone(),
                    scope_path: symbol.scope_path.clone(),
                    body_hash: body_hash(source, symbol.span),
                })
                .collect(),
            counters: CachedCounters {
                dynamic_imports_without_specifier: extraction.dynamic_imports_without_specifier,
                has_syntax_error: extraction.has_syntax_error,
                unmodelled_call_sites: references.unmodelled_call_sites,
                unmodelled_by_form: references.unmodelled_by_form.clone(),
            },
            document: DocumentCounters::default(),
        }
    }

    /// Canonical JSON. `serde_json` objects are ordered maps, so the text is byte-stable.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|source| IndexError::ModuleCache(format!("cannot serialize: {source}")))
    }

    /// Parse a cached payload.
    ///
    /// Returns `None` when the payload is not readable by this build. The caller treats that as
    /// "this file must be re-extracted" — a cache is allowed to miss, and a cache that could
    /// fail an index would be worse than no cache.
    pub fn from_json(text: &str) -> Option<ModuleFacts> {
        serde_json::from_str(text).ok()
    }

    /// Rebuild the slice of [`ModuleExtraction`] that [`crate::exports::ExportIndex::build`]
    /// reads, without parsing the file.
    ///
    /// `ExportIndex::build` consumes exactly three things: `rel_path`, `local_exports` (looked up
    /// through `symbols`), and `re_exports`. The symbol list reconstructed here therefore carries
    /// entity ids and nothing else — the other fields are never read on this path, and are filled
    /// with inert values rather than with guesses. Building the index from a mixture of freshly
    /// parsed and reconstructed modules is what lets one file be re-extracted against the export
    /// closure of a repository that was not re-parsed.
    pub fn as_export_source(&self, rel_path: &str) -> ModuleExtraction {
        reduced_export_source(
            rel_path,
            &self.exports,
            self.re_exports.iter().map(|re_export| ReExport {
                raw_specifier: re_export.raw_specifier.clone(),
                names: re_export.names.clone(),
                span: Span::NONE,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::ExportIndex;
    use crate::extract::extract_module;
    use crate::lang::Language;
    use crate::refs::extract_references;

    const PID: &str = "00000000000000000000000000000001";

    fn corpus(modules: &[(&str, &str)]) -> (Vec<ModuleExtraction>, BTreeSet<String>) {
        let indexed: BTreeSet<String> = modules.iter().map(|(path, _)| path.to_string()).collect();
        let extractions = modules
            .iter()
            .map(|(path, source)| extract_module(PID, path, Language::TypeScript, source).unwrap())
            .collect();
        (extractions, indexed)
    }

    /// The whole design rests on this: an export index built from reconstructed modules must
    /// answer every question identically to one built from the real parse.
    #[test]
    fn a_reconstructed_export_source_resolves_identically_to_the_parse() {
        let modules = [
            (
                "src/impl.ts",
                "export function helper() {}\nexport default helper;\n",
            ),
            ("src/barrel.ts", "export * from './impl';\n"),
            (
                "src/index.ts",
                "export { helper as aid } from './barrel';\nexport class Local {}\n",
            ),
        ];
        let (extractions, indexed) = corpus(&modules);
        let direct = ExportIndex::build(&extractions, &indexed);

        let reconstructed: Vec<ModuleExtraction> = extractions
            .iter()
            .map(|extraction| {
                let facts = ModuleFacts::from_extraction(
                    extraction,
                    &ReferenceExtraction::default(),
                    "export function helper() {}\nexport default helper;\n",
                );
                let round_tripped = ModuleFacts::from_json(&facts.to_json().unwrap())
                    .expect("a payload this build wrote must be readable");
                round_tripped.as_export_source(&extraction.rel_path)
            })
            .collect();
        let cached = ExportIndex::build(&reconstructed, &indexed);

        // The same reduction applied to a module that *was* parsed. The pipeline mixes all
        // three, so all three must answer identically.
        let reduced: Vec<ModuleExtraction> = extractions.iter().map(export_source_of).collect();
        let parsed_but_reduced = ExportIndex::build(&reduced, &indexed);

        for (module, _) in modules {
            for index in [&cached, &parsed_but_reduced] {
                assert_eq!(
                    direct.names(module),
                    index.names(module),
                    "export names differ for {module}"
                );
                for name in direct.names(module) {
                    assert_eq!(
                        direct.resolve(module, &name),
                        index.resolve(module, &name),
                        "resolution differs for {module}#{name}"
                    );
                }
                assert_eq!(
                    direct.resolve(module, "default"),
                    index.resolve(module, "default")
                );
                assert_eq!(
                    direct.resolve(module, "absent"),
                    index.resolve(module, "absent")
                );
            }
        }
    }

    #[test]
    fn facts_round_trip_through_json() {
        let source =
            "import { a } from './a';\nexport * from './b';\nexport function f() { return a; }\n";
        let extraction = extract_module(PID, "src/f.ts", Language::TypeScript, source).unwrap();
        let indexed: BTreeSet<String> = ["src/f.ts".to_string()].into_iter().collect();
        let export_index = ExportIndex::build(std::slice::from_ref(&extraction), &indexed);
        let references = extract_references(
            PID,
            "src/f.ts",
            Language::TypeScript,
            source,
            &extraction,
            &export_index,
            &indexed,
        )
        .unwrap();

        let facts = ModuleFacts::from_extraction(&extraction, &references, source);
        assert_eq!(
            facts.import_specifiers,
            vec!["./a".to_string(), "./b".to_string()]
        );
        assert!(facts.exports.contains_key("f"));
        assert_eq!(facts.symbols.len(), 1);
        assert_eq!(facts.symbols[0].name, "f");

        let json = facts.to_json().unwrap();
        assert_eq!(ModuleFacts::from_json(&json).unwrap(), facts);
        assert_eq!(
            ModuleFacts::from_json(&json).unwrap().to_json().unwrap(),
            json
        );
    }

    #[test]
    fn a_body_digest_changes_with_the_body_but_not_with_the_name() {
        let one = extract_module(
            PID,
            "a.ts",
            Language::TypeScript,
            "function f() { return 1; }\n",
        )
        .unwrap();
        let two = extract_module(
            PID,
            "a.ts",
            Language::TypeScript,
            "function f() { return 2; }\n",
        )
        .unwrap();
        let facts_one = ModuleFacts::from_extraction(
            &one,
            &ReferenceExtraction::default(),
            "function f() { return 1; }\n",
        );
        let facts_two = ModuleFacts::from_extraction(
            &two,
            &ReferenceExtraction::default(),
            "function f() { return 2; }\n",
        );
        assert_eq!(facts_one.symbols[0].name, facts_two.symbols[0].name);
        assert_ne!(
            facts_one.symbols[0].body_hash, facts_two.symbols[0].body_hash,
            "a body digest that ignores the body would link unrelated functions"
        );
    }

    /// An unreadable cache entry must degrade to "re-extract", never to a wrong answer.
    #[test]
    fn an_unparseable_payload_is_a_cache_miss() {
        assert!(ModuleFacts::from_json("{").is_none());
        assert!(ModuleFacts::from_json("null").is_none());
        assert!(ModuleFacts::from_json("{\"exports\":\"not-a-map\"}").is_none());
    }
}
