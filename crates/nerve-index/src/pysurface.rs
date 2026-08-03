//! What one Python module makes available to another, and which class declares which method.
//!
//! The counterpart of [`crate::exports::ExportIndex`], and deliberately **not** that type:
//! Python has no export keyword, so the question "what does `from m import n` bind?" is answered
//! by what `m` defines at top level, not by what it publishes. `__all__` is not consulted at all
//! here — it governs `from m import *`, which Nerve refuses to resolve, and using it to gate a
//! plain `from m import n` would refuse edges Python allows.
//!
//! One chain is followed: a module-scope, unconditional, non-wildcard `from X import Y` in the
//! target module. `pkg/__init__.py` writing `from .core import Engine` makes `pkg.Engine` **be**
//! `pkg.core.Engine`, which is a Python fact rather than a name coincidence. Conditional imports
//! are excluded — whether `try: from .x import Y` bound anything is not statically decidable, and
//! 9a already records that as a finding.
//!
//! Cycles are normal in Python packages (`pkg/__init__` imports `pkg.core`, which imports `pkg`),
//! so the walk carries a visited set keyed on `(module, name)`.

use std::collections::{BTreeMap, BTreeSet};

use crate::pyresolve;
use crate::pystruct::{PyImportForm, PyModuleExtraction};

/// One module's contribution to cross-module resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyModuleSurface {
    /// Repository-relative path.
    pub rel_path: String,
    /// Whether the module rewrites `sys.path`, which poisons its own absolute specifiers.
    pub sys_path_mutated: bool,
    /// Top-level `def` and `class` names -> entity id.
    pub defines: BTreeMap<String, String>,
    /// Module-scope unconditional `from X import Y [as Z]`: `Z` -> (`X`, `Y`).
    pub rebinds: BTreeMap<String, (String, String)>,
    /// Every class entity this module declares, at any nesting depth.
    pub classes: BTreeSet<String>,
    /// (class entity id, method name) -> method entity id.
    pub methods: BTreeMap<(String, String), String>,
}

impl PyModuleSurface {
    /// Read a module's surface straight off a fresh extraction.
    pub fn from_extraction(extraction: &PyModuleExtraction) -> PyModuleSurface {
        let mut surface = PyModuleSurface {
            rel_path: extraction.rel_path.clone(),
            sys_path_mutated: extraction.sys_path_mutated,
            ..PyModuleSurface::default()
        };

        for symbol in &extraction.symbols {
            if symbol.is_top_level() {
                surface
                    .defines
                    .entry(symbol.name.clone())
                    .or_insert_with(|| symbol.entity_id.clone());
            }
            if symbol.kind == nerve_core::vocab::EntityKind::Class {
                surface.classes.insert(symbol.entity_id.clone());
            }
            if let Some(owner) = &symbol.owner_class {
                surface
                    .methods
                    .entry((owner.clone(), symbol.name.clone()))
                    .or_insert_with(|| symbol.entity_id.clone());
            }
        }

        for site in &extraction.imports {
            if site.form != PyImportForm::FromImport || site.conditional.is_some() {
                continue;
            }
            for binding in &site.bindings {
                let (Some(imported), Some(local)) = (&binding.imported, &binding.local) else {
                    continue;
                };
                if imported == "*" {
                    continue;
                }
                surface
                    .rebinds
                    .entry(local.clone())
                    .or_insert_with(|| (site.raw_specifier.clone(), imported.clone()));
            }
        }

        surface
    }
}

/// Cross-module name resolution for Python, plus the class-member map.
#[derive(Debug, Default)]
pub struct PySurfaceIndex {
    by_path: BTreeMap<String, PyModuleSurface>,
    indexed: BTreeSet<String>,
}

impl PySurfaceIndex {
    /// Build the index from every indexed Python module's surface.
    pub fn build(surfaces: Vec<PyModuleSurface>, indexed: &BTreeSet<String>) -> PySurfaceIndex {
        let mut by_path = BTreeMap::new();
        for surface in surfaces {
            by_path.insert(surface.rel_path.clone(), surface);
        }
        PySurfaceIndex {
            by_path,
            indexed: indexed.clone(),
        }
    }

    /// Resolve `specifier` written in `importer`, honouring that module's `sys.path` rewrite.
    ///
    /// The rewrite poisons absolute specifiers only: a relative import is resolved from
    /// `__package__` and `sys.path` never enters it. Same rule as 9a's `IMPORTS` resolution, and
    /// stated in one place so the two cannot drift.
    pub fn resolve_specifier(&self, importer: &str, specifier: &str) -> Option<String> {
        let relative = pyresolve::is_relative(specifier);
        let poisoned = !relative
            && self
                .by_path
                .get(importer)
                .is_some_and(|surface| surface.sys_path_mutated);
        if poisoned {
            return None;
        }
        pyresolve::resolve(importer, specifier, &self.indexed)
    }

    /// The entity a module provides under `name`, following one kind of chain.
    pub fn provides(&self, module: &str, name: &str) -> Option<String> {
        let mut visited: BTreeSet<(String, String)> = BTreeSet::new();
        self.provides_inner(module, name, &mut visited)
    }

    fn provides_inner(
        &self,
        module: &str,
        name: &str,
        visited: &mut BTreeSet<(String, String)>,
    ) -> Option<String> {
        if !visited.insert((module.to_string(), name.to_string())) {
            return None;
        }
        let surface = self.by_path.get(module)?;
        if let Some(entity_id) = surface.defines.get(name) {
            return Some(entity_id.clone());
        }
        let (specifier, imported) = surface.rebinds.get(name)?;
        let target = self.resolve_specifier(module, specifier)?;
        self.provides_inner(&target, imported, visited)
    }

    /// Whether an entity id names a class Nerve indexed.
    pub fn is_class(&self, entity_id: &str) -> bool {
        self.by_path
            .values()
            .any(|surface| surface.classes.contains(entity_id))
    }

    /// The method a class declares under `name`, if it declares one at all.
    ///
    /// Declares, not inherits. Walking the MRO stops being a syntax fact as soon as one base is
    /// unresolved, so it is refused uniformly rather than sometimes.
    pub fn method_of(&self, class_entity_id: &str, name: &str) -> Option<String> {
        let key = (class_entity_id.to_string(), name.to_string());
        self.by_path
            .values()
            .find_map(|surface| surface.methods.get(&key).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pystruct::extract_module;

    const PID: &str = "00000000000000000000000000000001";

    fn index(modules: &[(&str, &str)]) -> PySurfaceIndex {
        let indexed: BTreeSet<String> = modules.iter().map(|(path, _)| path.to_string()).collect();
        let surfaces = modules
            .iter()
            .map(|(path, source)| {
                PyModuleSurface::from_extraction(&extract_module(PID, path, source).unwrap())
            })
            .collect();
        PySurfaceIndex::build(surfaces, &indexed)
    }

    #[test]
    fn a_top_level_def_is_provided_under_its_own_name() {
        let index = index(&[("pkg/util.py", "def scale(v):\n    return v\n")]);
        assert!(index.provides("pkg/util.py", "scale").is_some());
        assert!(index.provides("pkg/util.py", "missing").is_none());
    }

    #[test]
    fn a_nested_def_is_not_provided() {
        let index = index(&[(
            "pkg/util.py",
            "def outer():\n    def inner():\n        return 1\n    return inner\n",
        )]);
        assert!(index.provides("pkg/util.py", "inner").is_none());
    }

    #[test]
    fn a_package_init_re_export_is_followed() {
        let index = index(&[
            ("pkg/__init__.py", "from .core import Engine\n"),
            ("pkg/core.py", "class Engine:\n    pass\n"),
        ]);
        let through_package = index.provides("pkg/__init__.py", "Engine");
        let direct = index.provides("pkg/core.py", "Engine");
        assert!(direct.is_some());
        assert_eq!(through_package, direct, "pkg.Engine is pkg.core.Engine");
    }

    #[test]
    fn a_conditional_re_export_is_not_followed() {
        let index = index(&[
            (
                "pkg/__init__.py",
                "try:\n    from .core import Engine\nexcept ImportError:\n    Engine = None\n",
            ),
            ("pkg/core.py", "class Engine:\n    pass\n"),
        ]);
        assert!(
            index.provides("pkg/__init__.py", "Engine").is_none(),
            "whether a conditional import bound anything is not statically decidable"
        );
    }

    #[test]
    fn a_wildcard_re_export_is_not_followed() {
        let index = index(&[
            ("pkg/__init__.py", "from .core import *\n"),
            ("pkg/core.py", "class Engine:\n    pass\n"),
        ]);
        assert!(index.provides("pkg/__init__.py", "Engine").is_none());
    }

    #[test]
    fn a_re_export_cycle_terminates() {
        let index = index(&[
            ("a.py", "from b import name\n"),
            ("b.py", "from a import name\n"),
        ]);
        assert!(index.provides("a.py", "name").is_none());
    }

    #[test]
    fn a_sys_path_rewrite_poisons_absolute_specifiers_only() {
        let index = index(&[
            (
                "pkg/__init__.py",
                "import sys\n\nsys.path.append('vendor')\n\nfrom pkg.core import Engine\nfrom .util import scale\n",
            ),
            ("pkg/core.py", "class Engine:\n    pass\n"),
            ("pkg/util.py", "def scale(v):\n    return v\n"),
        ]);
        assert!(index.provides("pkg/__init__.py", "Engine").is_none());
        assert!(index.provides("pkg/__init__.py", "scale").is_some());
    }

    #[test]
    fn a_class_declares_its_own_methods_and_not_its_bases() {
        let index = index(&[(
            "pkg/core.py",
            "class Engine:\n    def start(self):\n        return 1\n\n\nclass Turbo(Engine):\n    def boost(self):\n        return 2\n",
        )]);
        let engine = index.provides("pkg/core.py", "Engine").unwrap();
        let turbo = index.provides("pkg/core.py", "Turbo").unwrap();
        assert!(index.is_class(&engine));
        assert!(index.is_class(&turbo));
        assert!(index.method_of(&engine, "start").is_some());
        assert!(
            index.method_of(&turbo, "start").is_none(),
            "Turbo inherits start; it does not declare it"
        );
        assert!(index.method_of(&turbo, "boost").is_some());
    }
}
