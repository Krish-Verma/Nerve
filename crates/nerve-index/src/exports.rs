//! Per-module export map, plus the transitive re-export closure.
//!
//! Barrel files are the normal shape of a TypeScript repository:
//! `export { add as plus } from './math'` and `export * from './shapes'`. Without a closure,
//! `import { plus } from './index'` followed by `plus()` cannot resolve and recall collapses
//! (plan P5).
//!
//! Two rules are load-bearing:
//!
//! - `export *` does **not** re-export `default`. That is the language rule, and getting it
//!   wrong invents an edge the code does not have.
//! - Barrel cycles are common (`a` re-exports `b`, `b` re-exports `a`) and must terminate. The
//!   closure carries a visited set keyed on `(module, name)`, which both breaks cycles and
//!   keeps the search linear: whether a module exports a name is a fixed answer, so a
//!   `(module, name)` pair already explored can never yield a different result on a second
//!   path.

use std::collections::{BTreeMap, BTreeSet};

use crate::extract::{ExportTarget, ModuleExtraction};
use crate::resolve;

/// Export maps for every indexed module.
#[derive(Debug, Default)]
pub struct ExportIndex {
    /// module -> exported name -> entity id defined in that module.
    local: BTreeMap<String, BTreeMap<String, String>>,
    /// module -> exported name -> (target module, name in the target module).
    named_re_exports: BTreeMap<String, BTreeMap<String, (String, String)>>,
    /// module -> `export *` targets, in source order.
    star_re_exports: BTreeMap<String, Vec<String>>,
}

impl ExportIndex {
    /// Build the index from every module's structural extraction.
    ///
    /// Specifiers are resolved with [`crate::resolve`], so a re-export of a module Nerve did
    /// not index contributes nothing rather than a guess.
    pub fn build(extractions: &[ModuleExtraction], indexed: &BTreeSet<String>) -> ExportIndex {
        let mut index = ExportIndex::default();

        for extraction in extractions {
            let mut local = BTreeMap::new();
            for export in &extraction.local_exports {
                let entity_id = match &export.target {
                    ExportTarget::Symbol(position) => {
                        Some(extraction.symbols[*position].entity_id.clone())
                    }
                    ExportTarget::LocalName(name) => extraction
                        .top_level_symbol(name)
                        .map(|position| extraction.symbols[position].entity_id.clone()),
                };
                if let Some(entity_id) = entity_id {
                    local.insert(export.exported_name.clone(), entity_id);
                }
            }
            index.local.insert(extraction.rel_path.clone(), local);

            let mut named = BTreeMap::new();
            let mut stars = Vec::new();
            for re_export in &extraction.re_exports {
                let Some(target) =
                    resolve::resolve(&extraction.rel_path, &re_export.raw_specifier, indexed)
                else {
                    continue;
                };
                match &re_export.names {
                    Some(names) => {
                        for (name, alias) in names {
                            let exported_as = alias.clone().unwrap_or_else(|| name.clone());
                            named.insert(exported_as, (target.clone(), name.clone()));
                        }
                    }
                    None => stars.push(target),
                }
            }
            index
                .named_re_exports
                .insert(extraction.rel_path.clone(), named);
            index
                .star_re_exports
                .insert(extraction.rel_path.clone(), stars);
        }

        index
    }

    /// Entity a module exports under `name`, following re-export chains.
    pub fn resolve(&self, module: &str, name: &str) -> Option<String> {
        let mut visited: BTreeSet<(String, String)> = BTreeSet::new();
        self.resolve_inner(module, name, &mut visited)
    }

    fn resolve_inner(
        &self,
        module: &str,
        name: &str,
        visited: &mut BTreeSet<(String, String)>,
    ) -> Option<String> {
        if !visited.insert((module.to_string(), name.to_string())) {
            return None;
        }

        if let Some(entity_id) = self.local.get(module).and_then(|names| names.get(name)) {
            return Some(entity_id.clone());
        }

        if let Some((target, source_name)) = self
            .named_re_exports
            .get(module)
            .and_then(|names| names.get(name))
        {
            if let Some(entity_id) = self.resolve_inner(target, source_name, visited) {
                return Some(entity_id);
            }
        }

        // `export *` forwards every name except `default`.
        if name != "default" {
            for target in self.star_re_exports.get(module).into_iter().flatten() {
                if let Some(entity_id) = self.resolve_inner(target, name, visited) {
                    return Some(entity_id);
                }
            }
        }

        None
    }

    /// Every name a module exports, following re-export chains. Sorted, cycle-safe.
    pub fn names(&self, module: &str) -> BTreeSet<String> {
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut out = BTreeSet::new();
        self.names_inner(module, true, &mut visited, &mut out);
        out
    }

    fn names_inner(
        &self,
        module: &str,
        include_default: bool,
        visited: &mut BTreeSet<String>,
        out: &mut BTreeSet<String>,
    ) {
        if !visited.insert(module.to_string()) {
            return;
        }
        for name in self.local.get(module).into_iter().flatten().map(|(k, _)| k) {
            if include_default || name != "default" {
                out.insert(name.clone());
            }
        }
        for name in self
            .named_re_exports
            .get(module)
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
        {
            out.insert(name.clone());
        }
        for target in self.star_re_exports.get(module).into_iter().flatten() {
            self.names_inner(target, false, visited, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_module;
    use crate::lang::Language;

    const PID: &str = "00000000000000000000000000000001";

    fn build(modules: &[(&str, &str)]) -> (ExportIndex, Vec<ModuleExtraction>) {
        let indexed: BTreeSet<String> = modules.iter().map(|(path, _)| path.to_string()).collect();
        let extractions: Vec<ModuleExtraction> = modules
            .iter()
            .map(|(path, source)| extract_module(PID, path, Language::TypeScript, source).unwrap())
            .collect();
        let index = ExportIndex::build(&extractions, &indexed);
        (index, extractions)
    }

    fn entity_of(extractions: &[ModuleExtraction], path: &str, name: &str) -> String {
        let extraction = extractions
            .iter()
            .find(|extraction| extraction.rel_path == path)
            .unwrap();
        let position = extraction.top_level_symbol(name).unwrap();
        extraction.symbols[position].entity_id.clone()
    }

    #[test]
    fn a_local_export_resolves_to_its_own_entity() {
        let (index, extractions) = build(&[("src/math.ts", "export function add() {}\n")]);
        assert_eq!(
            index.resolve("src/math.ts", "add"),
            Some(entity_of(&extractions, "src/math.ts", "add"))
        );
    }

    #[test]
    fn an_aliased_named_re_export_keeps_the_defining_entity() {
        let (index, extractions) = build(&[
            ("src/math.ts", "export function add() {}\n"),
            ("src/index.ts", "export { add as plus } from './math';\n"),
        ]);
        assert_eq!(
            index.resolve("src/index.ts", "plus"),
            Some(entity_of(&extractions, "src/math.ts", "add"))
        );
        assert_eq!(index.resolve("src/index.ts", "add"), None);
    }

    #[test]
    fn a_star_re_export_forwards_named_exports() {
        let (index, extractions) = build(&[
            ("src/shapes.ts", "export class Circle {}\n"),
            ("src/index.ts", "export * from './shapes';\n"),
        ]);
        assert_eq!(
            index.resolve("src/index.ts", "Circle"),
            Some(entity_of(&extractions, "src/shapes.ts", "Circle"))
        );
    }

    #[test]
    fn a_star_re_export_does_not_forward_default() {
        let (index, _) = build(&[
            (
                "src/math.ts",
                "export function add() {}\nexport default add;\n",
            ),
            ("src/index.ts", "export * from './math';\n"),
        ]);
        assert!(index.resolve("src/math.ts", "default").is_some());
        assert_eq!(
            index.resolve("src/index.ts", "default"),
            None,
            "`export *` must not re-export default"
        );
        assert!(index.resolve("src/index.ts", "add").is_some());
    }

    #[test]
    fn re_export_chains_are_transitive() {
        let (index, extractions) = build(&[
            ("src/math.ts", "export function add() {}\n"),
            ("src/mid.ts", "export * from './math';\n"),
            ("src/index.ts", "export { add as plus } from './mid';\n"),
        ]);
        assert_eq!(
            index.resolve("src/index.ts", "plus"),
            Some(entity_of(&extractions, "src/math.ts", "add"))
        );
    }

    #[test]
    fn a_re_export_cycle_terminates() {
        let (index, extractions) = build(&[
            (
                "src/a.ts",
                "export * from './b';\nexport function fromA() {}\n",
            ),
            (
                "src/b.ts",
                "export * from './a';\nexport function fromB() {}\n",
            ),
        ]);
        assert_eq!(
            index.resolve("src/a.ts", "fromB"),
            Some(entity_of(&extractions, "src/b.ts", "fromB"))
        );
        assert_eq!(
            index.resolve("src/b.ts", "fromA"),
            Some(entity_of(&extractions, "src/a.ts", "fromA"))
        );
        assert_eq!(index.resolve("src/a.ts", "nobody"), None);
    }

    #[test]
    fn a_self_referential_barrel_terminates() {
        let (index, _) = build(&[("src/a.ts", "export * from './a';\n")]);
        assert_eq!(index.resolve("src/a.ts", "anything"), None);
    }

    #[test]
    fn a_re_export_of_an_unindexed_module_contributes_nothing() {
        let (index, _) = build(&[("src/index.ts", "export * from './missing';\n")]);
        assert_eq!(index.resolve("src/index.ts", "anything"), None);
    }

    #[test]
    fn names_are_transitive_and_exclude_forwarded_defaults() {
        let (index, _) = build(&[
            (
                "src/math.ts",
                "export function add() {}\nexport default add;\n",
            ),
            ("src/index.ts", "export * from './math';\n"),
        ]);
        assert_eq!(
            index.names("src/index.ts").into_iter().collect::<Vec<_>>(),
            vec!["add".to_string()]
        );
        let math: Vec<String> = index.names("src/math.ts").into_iter().collect();
        assert_eq!(math, vec!["add".to_string(), "default".to_string()]);
    }
}
