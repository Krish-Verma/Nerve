//! Python's lexical binding table: what a name means at a point in one module.
//!
//! The counterpart of [`crate::bind`], and it holds the same line — resolution never asks "is
//! there a symbol called `scale` anywhere?", it asks "what does `scale` bind to *here*?" — but
//! Python's answer is reached by different rules, and three of them decide whether the answer
//! is right:
//!
//! 1. **A binding covers the whole function body, not the text after it.** `def f(): total =
//!    scale; scale = 2` makes `scale` local to `f` on *both* lines; CPython raises
//!    `UnboundLocalError` on the first rather than reaching an import. A guard that only looked
//!    backwards from the use would resolve it and be wrong, so every name a function assigns is
//!    collected before the body is walked.
//! 2. **A class body is not an enclosing scope for the methods defined in it.** `class C: x = 1;
//!    def m(self): return x` reads the *module's* `x`, not the class attribute. Only code
//!    running directly in the class body sees class-level names, so [`PyBindingTable::lookup`]
//!    consults the scope it starts in and then skips every enclosing [`PyScopeKind::Class`].
//! 3. **`global` and `nonlocal` redirect the walk.** `global x` sends the lookup straight to
//!    module scope past any enclosing function that happens to bind `x`; `nonlocal x` sends it
//!    to the nearest enclosing *function* scope and forbids it from reaching module scope.
//!
//! Comprehensions and generator expressions get their own scope, so `[target for target in xs]`
//! shadows a module-level `target` inside the comprehension and nowhere else.
//!
//! Everything the table cannot name is [`PyBinding::Opaque`] — parameters, plain variables,
//! loop and `with` and `except` targets, comprehension variables, `match` captures. An `Opaque`
//! binding **shadows** the outer one and blocks resolution, which is the whole point.

use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::Node;

use nerve_core::vocab::EntityKind;

use crate::pystruct::PyModuleExtraction;

/// What a Python name in scope refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PyBinding {
    /// A `def` or `class` in this module, with a Nerve entity.
    LocalSymbol {
        /// Entity identifier of the declaration.
        entity_id: String,
        /// Kind of the declared entity.
        kind: EntityKind,
        /// Index into [`PyModuleExtraction::symbols`].
        symbol_index: usize,
    },
    /// `from <specifier> import <imported> [as local]`.
    ImportedName {
        /// Specifier exactly as written, leading dots included.
        specifier: String,
        /// Name in the imported module.
        imported: String,
    },
    /// `import a.b` (binding `a` to module `a`) or `import a.b as c` (binding `c` to `a.b`).
    ImportedModule {
        /// Dotted module name this local name refers to.
        specifier: String,
    },
    /// A name Nerve knows exists but cannot name: a parameter, a variable, a loop or `with` or
    /// `except` target, a comprehension variable, a `match` capture, a wildcard-polluted name.
    Opaque,
}

/// The kind of lexical region a Python scope covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyScopeKind {
    /// The module's top level.
    Module,
    /// A `def`, `async def` or `lambda`, including its parameters.
    Function,
    /// A class body.
    Class,
    /// A comprehension or generator expression.
    Comprehension,
}

#[derive(Debug)]
struct ScopeNode {
    parent: Option<usize>,
    kind: PyScopeKind,
    start_byte: usize,
    end_byte: usize,
    bindings: BTreeMap<String, PyBinding>,
    globals: BTreeSet<String>,
    nonlocals: BTreeSet<String>,
}

/// The scope chain of one Python module.
#[derive(Debug)]
pub struct PyBindingTable {
    scopes: Vec<ScopeNode>,
}

fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// True for the node kinds that introduce a comprehension scope.
fn is_comprehension(kind: &str) -> bool {
    matches!(
        kind,
        "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    )
}

/// True for the node kinds that introduce a function scope.
pub fn is_function_node(kind: &str) -> bool {
    matches!(kind, "function_definition" | "lambda")
}

struct Builder<'a> {
    source: &'a [u8],
    extraction: &'a PyModuleExtraction,
    /// Declaration start byte -> index into `extraction.symbols`.
    symbol_at: BTreeMap<usize, usize>,
    scopes: Vec<ScopeNode>,
}

impl Builder<'_> {
    fn text(&self, node: Node) -> String {
        std::str::from_utf8(&self.source[node.byte_range()])
            .unwrap_or_default()
            .to_string()
    }

    fn push_scope(&mut self, kind: PyScopeKind, node: Node, parent: usize) -> usize {
        self.scopes.push(ScopeNode {
            parent: Some(parent),
            kind,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            bindings: BTreeMap::new(),
            globals: BTreeSet::new(),
            nonlocals: BTreeSet::new(),
        });
        self.scopes.len() - 1
    }

    fn bind(&mut self, scope: usize, name: String, binding: PyBinding) {
        if name.is_empty() {
            return;
        }
        self.scopes[scope].bindings.insert(name, binding);
    }

    /// The binding a declaration node introduces, when `py-structural` named it.
    ///
    /// A decorated definition's span starts at the `@`, so a `function_definition` whose parent
    /// is a `decorated_definition` is looked up under the parent's offset.
    fn symbol_binding(&self, node: Node) -> Option<PyBinding> {
        let index = symbol_index_for(&self.symbol_at, node)?;
        let symbol = &self.extraction.symbols[index];
        Some(PyBinding::LocalSymbol {
            entity_id: symbol.entity_id.clone(),
            kind: symbol.kind,
            symbol_index: index,
        })
    }

    // ---- collection: every name a scope binds, gathered before the body is walked ----------

    /// Bind every identifier a target pattern introduces as [`PyBinding::Opaque`].
    ///
    /// Deliberately generous: an `attribute` or `subscript` target binds no name and is skipped,
    /// but anything else that holds identifiers has them bound. Over-binding costs recall and
    /// can only ever suppress an edge; under-binding invents one.
    fn bind_target(&mut self, node: Node, scope: usize) {
        match node.kind() {
            "identifier" => {
                let name = self.text(node);
                self.bind(scope, name, PyBinding::Opaque);
            }
            // `obj.attr = 1` and `xs[0] = 1` write through a value; they bind no name.
            "attribute" | "subscript" => {}
            "as_pattern" => {
                if let Some(alias) = node.child_by_field_name("alias") {
                    self.bind_target(alias, scope);
                }
            }
            _ => {
                for child in named_children(node) {
                    self.bind_target(child, scope);
                }
            }
        }
    }

    /// Record the bindings an `import` / `from … import` statement introduces.
    fn collect_import(&mut self, node: Node, scope: usize) {
        match node.kind() {
            "import_statement" => {
                for child in named_children(node) {
                    let (target, alias) = match child.kind() {
                        "dotted_name" => (Some(child), None),
                        "aliased_import" => (
                            child.child_by_field_name("name"),
                            child.child_by_field_name("alias"),
                        ),
                        _ => (None, None),
                    };
                    let Some(target) = target else { continue };
                    let dotted = self.text(target);
                    match alias {
                        // `import a.b as c` binds `c` to the module `a.b`.
                        Some(alias) => {
                            let local = self.text(alias);
                            self.bind(
                                scope,
                                local,
                                PyBinding::ImportedModule { specifier: dotted },
                            )
                        }
                        // `import a.b` binds `a` to the module `a`, never to `a.b`.
                        None => {
                            let Some(head) = dotted.split('.').next().map(str::to_string) else {
                                continue;
                            };
                            self.bind(
                                scope,
                                head.clone(),
                                PyBinding::ImportedModule { specifier: head },
                            );
                        }
                    }
                }
            }
            "import_from_statement" | "future_import_statement" => {
                let Some(module_node) = node.child_by_field_name("module_name") else {
                    return;
                };
                let specifier = self.text(module_node);
                for child in named_children(node) {
                    if child.id() == module_node.id() {
                        continue;
                    }
                    match child.kind() {
                        // `from x import *` binds a set of names that lives in the imported
                        // module's runtime namespace. 9a records that as an unknowable value;
                        // binding a guess here would contradict it.
                        "wildcard_import" => {}
                        "dotted_name" => {
                            let name = self.text(child);
                            self.bind(
                                scope,
                                name.clone(),
                                PyBinding::ImportedName {
                                    specifier: specifier.clone(),
                                    imported: name,
                                },
                            );
                        }
                        "aliased_import" => {
                            let (Some(imported), Some(alias)) = (
                                child.child_by_field_name("name"),
                                child.child_by_field_name("alias"),
                            ) else {
                                continue;
                            };
                            let imported = self.text(imported);
                            let local = self.text(alias);
                            self.bind(
                                scope,
                                local,
                                PyBinding::ImportedName {
                                    specifier: specifier.clone(),
                                    imported,
                                },
                            );
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Gather every name `scope` binds, without descending into the scopes nested inside it.
    ///
    /// `in_comprehension` suppresses collection of a `for … in` target: that target belongs to
    /// the comprehension's own scope, while a walrus written inside the same comprehension binds
    /// in the enclosing one (PEP 572).
    fn collect(&mut self, node: Node, scope: usize, in_comprehension: bool) {
        for child in named_children(node) {
            match child.kind() {
                // A nested scope. Its name is a binding here; its body is not.
                "function_definition" | "class_definition" => {
                    if let Some(name) = child.child_by_field_name("name") {
                        let name = self.text(name);
                        self.bind(scope, name, PyBinding::Opaque);
                    }
                }
                "decorated_definition" => {
                    if let Some(definition) = child.child_by_field_name("definition") {
                        if let Some(name) = definition.child_by_field_name("name") {
                            let name = self.text(name);
                            self.bind(scope, name, PyBinding::Opaque);
                        }
                    }
                }
                // A lambda's parameters and its own walrus targets are the lambda's.
                "lambda" => {}

                "global_statement" => {
                    for name in named_children(child) {
                        let name = self.text(name);
                        self.scopes[scope].globals.insert(name);
                    }
                }
                "nonlocal_statement" => {
                    for name in named_children(child) {
                        let name = self.text(name);
                        self.scopes[scope].nonlocals.insert(name);
                    }
                }

                "assignment" | "augmented_assignment" => {
                    if let Some(left) = child.child_by_field_name("left") {
                        self.bind_target(left, scope);
                    }
                    self.collect(child, scope, in_comprehension);
                }
                "for_statement" => {
                    if let Some(left) = child.child_by_field_name("left") {
                        self.bind_target(left, scope);
                    }
                    self.collect(child, scope, in_comprehension);
                }
                "for_in_clause" => {
                    if !in_comprehension {
                        if let Some(left) = child.child_by_field_name("left") {
                            self.bind_target(left, scope);
                        }
                    }
                    self.collect(child, scope, in_comprehension);
                }
                "as_pattern" => {
                    if let Some(alias) = child.child_by_field_name("alias") {
                        self.bind_target(alias, scope);
                    }
                    self.collect(child, scope, in_comprehension);
                }
                "named_expression" => {
                    if let Some(name) = child.child_by_field_name("name") {
                        self.bind_target(name, scope);
                    }
                    self.collect(child, scope, in_comprehension);
                }
                // A `match` capture is a binding, and telling a capture from a class pattern's
                // name needs the runtime value. Every identifier in the pattern is bound, which
                // can only ever suppress an edge.
                "case_pattern" => self.bind_target(child, scope),

                "import_statement" | "import_from_statement" | "future_import_statement" => {
                    self.collect_import(child, scope);
                }

                kind if is_comprehension(kind) => self.collect(child, scope, true),

                _ => self.collect(child, scope, in_comprehension),
            }
        }

        // `global x` and `nonlocal x` say the assignments in this scope are not local bindings.
        // Applied after collection so that the declaration may sit anywhere in the body.
        let declared: Vec<String> = self.scopes[scope]
            .globals
            .iter()
            .chain(self.scopes[scope].nonlocals.iter())
            .cloned()
            .collect();
        for name in declared {
            self.scopes[scope].bindings.remove(&name);
        }
    }

    // ---- the scope tree --------------------------------------------------------------------

    fn bind_parameters(&mut self, node: Node, scope: usize) {
        for field in ["parameters", "lambda_parameters"] {
            if let Some(parameters) = node.child_by_field_name(field) {
                for parameter in named_children(parameters) {
                    match parameter.kind() {
                        "identifier" => self.bind_target(parameter, scope),
                        "typed_parameter"
                        | "default_parameter"
                        | "typed_default_parameter"
                        | "list_splat_pattern"
                        | "dictionary_splat_pattern" => {
                            // The binding is the first identifier; the annotation and the
                            // default are expressions and bind nothing.
                            if let Some(name) = parameter.child_by_field_name("name") {
                                self.bind_target(name, scope);
                            } else if let Some(first) = named_children(parameter).first() {
                                if first.kind() == "identifier" {
                                    self.bind_target(*first, scope);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn visit_function(&mut self, node: Node, scope: usize) {
        if let (Some(name_node), Some(binding)) =
            (node.child_by_field_name("name"), self.symbol_binding(node))
        {
            let name = self.text(name_node);
            self.bind(scope, name, binding);
        }
        let inner = self.push_scope(PyScopeKind::Function, node, scope);
        self.bind_parameters(node, inner);
        if let Some(body) = node.child_by_field_name("body") {
            self.collect(body, inner, false);
        }
        for child in named_children(node) {
            self.visit(child, inner);
        }
    }

    fn visit_class(&mut self, node: Node, scope: usize) {
        if let (Some(name_node), Some(binding)) =
            (node.child_by_field_name("name"), self.symbol_binding(node))
        {
            let name = self.text(name_node);
            self.bind(scope, name, binding);
        }

        // The base list is evaluated before the class body exists, in the enclosing scope. It is
        // walked first so that scope start offsets stay non-decreasing, which is what
        // `scope_at`'s binary search rests on.
        if let Some(superclasses) = node.child_by_field_name("superclasses") {
            self.visit(superclasses, scope);
        }

        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        // The scope covers the body only, and the class name is not bound *in* it: a class body
        // is executed before the name exists, so binding it here would model a scope Python does
        // not have. The enclosing scope still binds it, which is where a class body that names
        // its own class finds it — and where CPython raises `NameError` instead, for a reason
        // (execution order) that a lexical table has no way to see.
        let inner = self.push_scope(PyScopeKind::Class, body, scope);
        self.collect(body, inner, false);
        for child in named_children(body) {
            self.visit(child, inner);
        }
    }

    fn visit(&mut self, node: Node, scope: usize) {
        match node.kind() {
            // Import bindings were collected with the rest of the scope's names.
            "import_statement" | "import_from_statement" | "future_import_statement" => {}

            "function_definition" | "lambda" => self.visit_function(node, scope),
            "class_definition" => self.visit_class(node, scope),

            kind if is_comprehension(kind) => {
                let inner = self.push_scope(PyScopeKind::Comprehension, node, scope);
                for child in named_children(node) {
                    if child.kind() == "for_in_clause" {
                        if let Some(left) = child.child_by_field_name("left") {
                            self.bind_target(left, inner);
                        }
                    }
                }
                for child in named_children(node) {
                    self.visit(child, inner);
                }
            }

            _ => {
                for child in named_children(node) {
                    self.visit(child, scope);
                }
            }
        }
    }
}

/// Index into `symbols` for the declaration starting at `node`, decorators included.
pub fn symbol_index_for(symbol_at: &BTreeMap<usize, usize>, node: Node) -> Option<usize> {
    if let Some(index) = symbol_at.get(&node.start_byte()) {
        return Some(*index);
    }
    let parent = node.parent()?;
    if parent.kind() != "decorated_definition" {
        return None;
    }
    symbol_at.get(&parent.start_byte()).copied()
}

/// Declaration start byte -> index into `extraction.symbols`.
pub fn symbol_offsets(extraction: &PyModuleExtraction) -> BTreeMap<usize, usize> {
    let mut symbol_at: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, symbol) in extraction.symbols.iter().enumerate() {
        symbol_at.entry(symbol.span.start_byte).or_insert(index);
    }
    symbol_at
}

impl PyBindingTable {
    /// Build the scope chain for one Python module.
    pub fn build(extraction: &PyModuleExtraction, root: Node, source: &[u8]) -> PyBindingTable {
        let mut builder = Builder {
            source,
            extraction,
            symbol_at: symbol_offsets(extraction),
            scopes: vec![ScopeNode {
                parent: None,
                kind: PyScopeKind::Module,
                // Deliberately 0, not `root.start_byte()`: leading trivia must still land in the
                // module scope rather than fall off the front of the search.
                start_byte: 0,
                end_byte: usize::MAX,
                bindings: BTreeMap::new(),
                globals: BTreeSet::new(),
                nonlocals: BTreeSet::new(),
            }],
        };

        builder.collect(root, 0, false);
        for child in named_children(root) {
            builder.visit(child, 0);
        }

        PyBindingTable {
            scopes: builder.scopes,
        }
    }

    /// Innermost scope containing `byte`.
    ///
    /// Scopes are pushed in pre-order, so their start offsets are non-decreasing; the deepest
    /// candidate is the last one starting at or before `byte`, and the parent chain from there
    /// reaches the first that actually contains it.
    pub fn scope_at(&self, byte: usize) -> usize {
        let mut candidate = match self
            .scopes
            .binary_search_by(|scope| scope.start_byte.cmp(&byte))
        {
            Ok(index) => index,
            Err(0) => return 0,
            Err(index) => index - 1,
        };
        while candidate + 1 < self.scopes.len() && self.scopes[candidate + 1].start_byte <= byte {
            candidate += 1;
        }
        loop {
            let scope = &self.scopes[candidate];
            if scope.start_byte <= byte && byte < scope.end_byte {
                return candidate;
            }
            match scope.parent {
                Some(parent) => candidate = parent,
                None => return 0,
            }
        }
    }

    /// Resolve a name from `scope` outwards, by Python's rules.
    ///
    /// The starting scope is always consulted. Every **enclosing** class scope is skipped: only
    /// code running directly in a class body sees class-level names. `global` jumps to module
    /// scope; `nonlocal` continues in the enclosing function scopes and never reaches module
    /// scope.
    pub fn lookup(&self, scope: usize, name: &str) -> Option<&PyBinding> {
        let mut current = scope;
        let mut first = true;
        loop {
            let node = &self.scopes[current];
            if node.globals.contains(name) {
                return self.scopes[0].bindings.get(name);
            }
            if node.nonlocals.contains(name) {
                return self.lookup_nonlocal(node.parent?, name);
            }
            if first || node.kind != PyScopeKind::Class {
                if let Some(binding) = node.bindings.get(name) {
                    return Some(binding);
                }
            }
            first = false;
            current = node.parent?;
        }
    }

    /// The `nonlocal` walk: enclosing function scopes only, never the module.
    fn lookup_nonlocal(&self, from: usize, name: &str) -> Option<&PyBinding> {
        let mut current = from;
        loop {
            let node = &self.scopes[current];
            if node.kind == PyScopeKind::Module {
                return None;
            }
            if node.kind != PyScopeKind::Class {
                if let Some(binding) = node.bindings.get(name) {
                    return Some(binding);
                }
            }
            current = node.parent?;
        }
    }

    /// Kind of a scope, for tests and diagnostics.
    pub fn kind_of(&self, scope: usize) -> PyScopeKind {
        self.scopes[scope].kind
    }

    /// Number of scopes, for tests.
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    /// Always false: the module scope always exists.
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Language;
    use crate::pystruct::extract_module;

    const PID: &str = "00000000000000000000000000000001";

    struct Fixture {
        extraction: PyModuleExtraction,
        table: PyBindingTable,
        source: String,
    }

    impl Fixture {
        /// Entity id of the symbol declared at `scope_path` under `name`.
        fn entity(&self, scope_path: &str, name: &str) -> String {
            self.extraction
                .symbols
                .iter()
                .find(|symbol| symbol.scope_path == scope_path && symbol.name == name)
                .unwrap_or_else(|| panic!("{scope_path}.{name} was not extracted"))
                .entity_id
                .clone()
        }
    }

    fn build(source: &str) -> Fixture {
        let extraction = extract_module(PID, "pkg/mod.py", source).unwrap();
        let mut parser = Language::Python.parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let table = PyBindingTable::build(&extraction, tree.root_node(), source.as_bytes());
        Fixture {
            extraction,
            table,
            source: source.to_string(),
        }
    }

    /// Byte offset of the `occurrence`th (0-based) appearance of `text`.
    fn offset_of(fixture: &Fixture, text: &str, occurrence: usize) -> usize {
        let mut from = 0;
        for _ in 0..=occurrence {
            let found = fixture.source[from..]
                .find(text)
                .unwrap_or_else(|| panic!("{text:?} does not occur {} times", occurrence + 1));
            from += found + 1;
        }
        from - 1
    }

    fn lookup_at(fixture: &Fixture, name: &str, occurrence: usize) -> Option<PyBinding> {
        let byte = offset_of(fixture, name, occurrence);
        let scope = fixture.table.scope_at(byte);
        fixture.table.lookup(scope, name).cloned()
    }

    fn is_symbol(binding: &Option<PyBinding>) -> bool {
        matches!(binding, Some(PyBinding::LocalSymbol { .. }))
    }

    #[test]
    fn a_module_level_def_binds_at_module_scope() {
        let fixture = build("def scale(v):\n    return v\n\n\ndef run():\n    return scale(1)\n");
        assert!(is_symbol(&lookup_at(&fixture, "scale", 1)));
    }

    #[test]
    fn a_parameter_shadows_a_module_function() {
        let fixture =
            build("def scale(v):\n    return v\n\n\ndef run(scale):\n    return scale(1)\n");
        assert_eq!(lookup_at(&fixture, "scale", 2), Some(PyBinding::Opaque));
    }

    /// The rule a backwards-looking guard gets wrong: Python binds for the whole body.
    #[test]
    fn an_assignment_anywhere_in_a_body_makes_the_name_local_everywhere_in_it() {
        let fixture = build(
            "def scale(v):\n    return v\n\n\ndef run():\n    total = scale\n    scale = 2\n    return total + scale\n",
        );
        assert_eq!(
            lookup_at(&fixture, "scale", 1),
            Some(PyBinding::Opaque),
            "the read above the assignment is still a read of the local"
        );
    }

    #[test]
    fn a_class_body_is_not_an_enclosing_scope_for_its_methods() {
        let fixture = build(
            "def helper():\n    return 1\n\n\nclass C:\n    helper = 2\n    doubled = helper * 2\n\n    def use(self):\n        return helper()\n",
        );
        // In the class body the attribute wins.
        assert_eq!(lookup_at(&fixture, "helper", 2), Some(PyBinding::Opaque));
        // Inside the method the module function wins.
        assert!(is_symbol(&lookup_at(&fixture, "helper", 3)));
    }

    /// The class scope holds the class body's own names and nothing else.
    ///
    /// The class name itself is found in the **enclosing** scope, which is where a lexical table
    /// can find it. CPython raises `NameError` for `class C: alias = C` instead, and that is a
    /// fact about execution order rather than about scoping — stated here so the difference is
    /// recorded rather than discovered later.
    #[test]
    fn the_class_scope_holds_the_class_body_and_not_the_class_name() {
        let fixture = build("class C:\n    limit = 1\n    doubled = limit * 2\n    alias = C\n");
        assert_eq!(lookup_at(&fixture, "limit", 1), Some(PyBinding::Opaque));
        assert!(is_symbol(&lookup_at(&fixture, "C", 1)));
        let byte = offset_of(&fixture, "alias = C", 0);
        assert_eq!(
            fixture.table.kind_of(fixture.table.scope_at(byte)),
            PyScopeKind::Class
        );
    }

    #[test]
    fn a_comprehension_variable_shadows_only_inside_the_comprehension() {
        let fixture = build(
            "def target():\n    return 1\n\n\ndef run(items):\n    return [target for target in items]\n\n\ndef other():\n    return target()\n",
        );
        assert_eq!(lookup_at(&fixture, "target", 1), Some(PyBinding::Opaque));
        assert!(is_symbol(&lookup_at(&fixture, "target", 3)));
    }

    #[test]
    fn global_sends_the_lookup_past_an_enclosing_function_that_binds_the_name() {
        let fixture = build(
            "def target():\n    return 1\n\n\ndef outer():\n    target = 2\n\n    def inner():\n        global target\n        return target()\n\n    return inner()\n",
        );
        // The fourth `target` is the call inside `inner`, which declared the name global.
        assert!(is_symbol(&lookup_at(&fixture, "target", 3)));
        // Without the declaration, `outer`'s local would have won.
        assert_eq!(lookup_at(&fixture, "target", 1), Some(PyBinding::Opaque));
    }

    #[test]
    fn nonlocal_binds_the_enclosing_function_and_never_the_module() {
        let fixture = build(
            "def helper():\n    return 0\n\n\ndef outer():\n    def helper():\n        return 1\n\n    def replace():\n        nonlocal helper\n        value = helper()\n        helper = None\n        return value\n\n    return replace()\n",
        );
        let byte = offset_of(&fixture, "value = helper()", 0) + "value = ".len();
        let scope = fixture.table.scope_at(byte);
        let Some(PyBinding::LocalSymbol { entity_id, .. }) = fixture.table.lookup(scope, "helper")
        else {
            panic!("nonlocal must reach the enclosing function's def");
        };
        assert_eq!(
            *entity_id,
            fixture.entity("outer", "helper"),
            "nonlocal reached the wrong `helper`"
        );
        assert_ne!(*entity_id, fixture.entity("", "helper"));
    }

    #[test]
    fn import_forms_bind_what_they_actually_bind() {
        let fixture = build(
            "import pkg.util\nimport pkg.core as core\nfrom pkg.util import scale\nfrom pkg.util import scale as s\nfrom pkg.util import *\n",
        );
        let scope = 0;
        assert_eq!(
            fixture.table.lookup(scope, "pkg"),
            Some(&PyBinding::ImportedModule {
                specifier: "pkg".to_string()
            }),
            "`import a.b` binds `a`, not `a.b`"
        );
        assert_eq!(
            fixture.table.lookup(scope, "core"),
            Some(&PyBinding::ImportedModule {
                specifier: "pkg.core".to_string()
            })
        );
        assert_eq!(
            fixture.table.lookup(scope, "scale"),
            Some(&PyBinding::ImportedName {
                specifier: "pkg.util".to_string(),
                imported: "scale".to_string()
            })
        );
        assert_eq!(
            fixture.table.lookup(scope, "s"),
            Some(&PyBinding::ImportedName {
                specifier: "pkg.util".to_string(),
                imported: "scale".to_string()
            })
        );
        assert_eq!(
            fixture.table.lookup(scope, "anything"),
            None,
            "a wildcard binds nothing this file can state"
        );
    }

    #[test]
    fn loop_with_and_except_targets_shadow() {
        let fixture = build(
            "def scale(v):\n    return v\n\n\ndef run(xs):\n    for scale in xs:\n        pass\n    return scale\n",
        );
        assert_eq!(lookup_at(&fixture, "scale", 2), Some(PyBinding::Opaque));

        let fixture = build(
            "def err(v):\n    return v\n\n\ndef run():\n    try:\n        pass\n    except ValueError as err:\n        return err\n",
        );
        assert_eq!(lookup_at(&fixture, "err", 2), Some(PyBinding::Opaque));

        let fixture = build(
            "def fh(v):\n    return v\n\n\ndef run():\n    with open('x') as fh:\n        return fh\n",
        );
        assert_eq!(lookup_at(&fixture, "fh", 2), Some(PyBinding::Opaque));
    }

    #[test]
    fn a_match_capture_shadows() {
        let fixture = build(
            "def other(v):\n    return v\n\n\ndef run(command):\n    match command:\n        case other:\n            return other\n",
        );
        assert_eq!(lookup_at(&fixture, "other", 2), Some(PyBinding::Opaque));
    }

    #[test]
    fn a_decorated_definition_is_still_bound_to_its_symbol() {
        let fixture = build(
            "import functools\n\n\n@functools.cache\ndef tune(v):\n    return v\n\n\ndef run():\n    return tune(1)\n",
        );
        assert!(is_symbol(&lookup_at(&fixture, "tune", 1)));
    }

    #[test]
    fn scope_starts_are_non_decreasing() {
        let fixture = build(
            "class C(dict, metaclass=type):\n    def m(self):\n        return [x for x in (lambda: [])()]\n",
        );
        let mut previous = 0usize;
        for index in 0..fixture.table.len() {
            let start = fixture.table.scopes[index].start_byte;
            assert!(
                start >= previous,
                "scope {index} starts at {start}, before {previous}"
            );
            previous = start;
        }
    }
}
