//! Lexical binding table: what a name means at a point in one module.
//!
//! This is the precision guard of Slice 2a. Resolution never asks "is there a symbol called
//! `foo` anywhere?" — it asks "what does `foo` bind to *here*?", and the answer is frequently
//! [`Binding::Opaque`]: a parameter, a plain variable, a destructured field, a catch binding, a
//! loop binding, a type parameter, an enum or a namespace. Those are names Nerve knows exist
//! but cannot name as entities. An `Opaque` binding **shadows** the outer one and blocks
//! resolution, which is what stops `function foo() {}; function bar(foo) { foo(); }` from being
//! reported as `bar` calling the module-level `foo`.
//!
//! The table is built once per module, before any reference is resolved, so hoisting is free:
//! a call to a function declared later in the same scope resolves without a second pass.

use std::collections::BTreeMap;

use tree_sitter::Node;

use nerve_core::vocab::EntityKind;

use crate::extract::{ImportForm, ModuleExtraction};

/// What a name in scope refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// A symbol declared in this module, with a Nerve entity.
    LocalSymbol {
        /// Entity identifier of the declaration.
        entity_id: String,
        /// Kind of the declared entity.
        kind: EntityKind,
        /// Index into [`ModuleExtraction::symbols`].
        symbol_index: usize,
    },
    /// `import { imported as local } from specifier`.
    ImportedNamed {
        /// Module specifier exactly as written.
        specifier: String,
        /// Name in the exporting module.
        imported: String,
    },
    /// `import local from specifier`.
    ImportedDefault {
        /// Module specifier exactly as written.
        specifier: String,
    },
    /// `import * as local from specifier`, or `const local = require(specifier)`.
    ImportedNamespace {
        /// Module specifier exactly as written.
        specifier: String,
    },
    /// A name Nerve knows exists but cannot name: parameters, plain variables, destructuring
    /// targets, catch parameters, loop bindings, type parameters, enums, namespaces.
    Opaque,
}

/// The kind of lexical region a scope covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// The module's top level.
    Module,
    /// A function, method, constructor or arrow body, including its parameters.
    Function,
    /// A block, loop head, or type-parameter list.
    Block,
    /// A class body.
    Class,
    /// A `catch` clause.
    Catch,
}

/// How `this` is bound by a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ThisBinding {
    /// The scope does not rebind `this`; look further out.
    Inherit,
    /// A method or constructor of a class declared in this module.
    Method {
        class_entity_id: String,
        is_static: bool,
    },
    /// A non-arrow function rebinds `this` to something Nerve cannot name.
    Rebound,
    /// Module top level.
    Module,
}

/// What `this` refers to at a point in the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThisResolution {
    /// `this` is the instance (or constructor) of a class declared in this module.
    Class {
        /// Entity id of the class.
        entity_id: String,
        /// True inside a `static` method.
        is_static: bool,
    },
    /// A non-arrow `function` boundary inside a class method rebound `this` (P3 case a).
    ReboundByNestedFunction,
    /// `this` is not bound to a class declared in this module at all.
    NotInClassMethod,
}

#[derive(Debug)]
struct ScopeNode {
    parent: Option<usize>,
    kind: ScopeKind,
    start_byte: usize,
    end_byte: usize,
    bindings: BTreeMap<String, Binding>,
    this_binding: ThisBinding,
}

/// The scope chain of one module.
#[derive(Debug)]
pub struct BindingTable {
    scopes: Vec<ScopeNode>,
}

/// True for the node kinds that introduce a function scope.
pub fn is_function_node(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "generator_function_declaration"
            | "function_signature"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
    )
}

fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn all_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn has_child_kind(node: Node, kind: &str) -> bool {
    all_children(node).iter().any(|child| child.kind() == kind)
}

struct Builder<'a> {
    source: &'a [u8],
    extraction: &'a ModuleExtraction,
    /// Declaration start byte -> index into `extraction.symbols`.
    symbol_at: BTreeMap<usize, usize>,
    scopes: Vec<ScopeNode>,
}

impl<'a> Builder<'a> {
    fn text(&self, node: Node) -> String {
        std::str::from_utf8(&self.source[node.byte_range()])
            .unwrap_or_default()
            .to_string()
    }

    fn push_scope(
        &mut self,
        kind: ScopeKind,
        node: Node,
        parent: usize,
        this_binding: ThisBinding,
    ) -> usize {
        self.scopes.push(ScopeNode {
            parent: Some(parent),
            kind,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            bindings: BTreeMap::new(),
            this_binding,
        });
        self.scopes.len() - 1
    }

    fn bind(&mut self, scope: usize, name: String, binding: Binding) {
        if name.is_empty() {
            return;
        }
        self.scopes[scope].bindings.insert(name, binding);
    }

    /// The binding a declaration node introduces, when the structural extractor named it.
    fn symbol_binding(&self, declaration_start: usize) -> Option<Binding> {
        let index = *self.symbol_at.get(&declaration_start)?;
        let symbol = &self.extraction.symbols[index];
        Some(Binding::LocalSymbol {
            entity_id: symbol.entity_id.clone(),
            kind: symbol.kind,
            symbol_index: index,
        })
    }

    /// Bind every identifier a binding pattern introduces as [`Binding::Opaque`].
    fn bind_pattern(&mut self, node: Node, scope: usize) {
        match node.kind() {
            "identifier" => {
                let name = self.text(node);
                self.bind(scope, name, Binding::Opaque);
            }
            "shorthand_property_identifier_pattern" => {
                let name = self.text(node);
                self.bind(scope, name, Binding::Opaque);
            }
            "pair_pattern" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.bind_pattern(value, scope);
                }
            }
            "assignment_pattern" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.bind_pattern(left, scope);
                }
            }
            "object_pattern" | "array_pattern" | "rest_pattern" => {
                for child in named_children(node) {
                    self.bind_pattern(child, scope);
                }
            }
            // TypeScript parameter wrappers carry the binding under `pattern`.
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind_pattern(pattern, scope);
                }
            }
            _ => {}
        }
    }

    fn bind_parameters(&mut self, node: Node, scope: usize) {
        if let Some(parameters) = node.child_by_field_name("parameters") {
            for child in named_children(parameters) {
                self.bind_pattern(child, scope);
            }
        }
        // `x => x` binds a single parameter without a `formal_parameters` node.
        if let Some(parameter) = node.child_by_field_name("parameter") {
            self.bind_pattern(parameter, scope);
        }
        // Type parameters are names in scope that are not Nerve entities.
        if let Some(type_parameters) = node.child_by_field_name("type_parameters") {
            for child in named_children(type_parameters) {
                if child.kind() == "type_parameter" {
                    if let Some(name) = child.child_by_field_name("name") {
                        let name = self.text(name);
                        self.bind(scope, name, Binding::Opaque);
                    }
                }
            }
        }
    }

    fn this_binding_for(&self, node: Node, enclosing_class: Option<&str>) -> ThisBinding {
        match node.kind() {
            // Arrow functions do not rebind `this`.
            "arrow_function" => ThisBinding::Inherit,
            "method_definition" => match enclosing_class {
                Some(class_entity_id) => ThisBinding::Method {
                    class_entity_id: class_entity_id.to_string(),
                    is_static: has_child_kind(node, "static"),
                },
                // An object-literal method, or a class Nerve did not name.
                None => ThisBinding::Rebound,
            },
            _ => ThisBinding::Rebound,
        }
    }

    /// Visit a function-like node: new scope, parameters bound, then the rest.
    fn function_scope(&mut self, node: Node, scope: usize, enclosing_class: Option<&str>) {
        let this_binding = self.this_binding_for(node, enclosing_class);
        let inner = self.push_scope(ScopeKind::Function, node, scope, this_binding);
        self.bind_parameters(node, inner);

        // A named function *expression* binds its own name inside itself. It is not addressable
        // from outside, so it is Opaque unless the structural extractor named it.
        if matches!(node.kind(), "function_expression" | "generator_function") {
            if let Some(name) = node.child_by_field_name("name") {
                let name = self.text(name);
                self.bind(inner, name, Binding::Opaque);
            }
        }

        for child in named_children(node) {
            self.visit(child, inner, None);
        }
    }

    fn visit_class(&mut self, node: Node, scope: usize) {
        let binding = self.symbol_binding(node.start_byte());
        let name_node = node.child_by_field_name("name");

        if let (Some(name_node), Some(binding)) = (name_node, binding.clone()) {
            let name = self.text(name_node);
            self.bind(scope, name, binding);
        }

        let inner = self.push_scope(ScopeKind::Class, node, scope, ThisBinding::Inherit);
        // A class type parameter named after a module symbol must shadow it, or
        // `class Box<Shape> { x: Shape }` would resolve `Shape` to the module's interface.
        self.bind_parameters(node, inner);
        // The class name is visible inside its own body.
        if let Some(name_node) = name_node {
            let name = self.text(name_node);
            let inner_binding = binding.clone().unwrap_or(Binding::Opaque);
            self.bind(inner, name, inner_binding);
        }
        let class_context = match &binding {
            Some(Binding::LocalSymbol { entity_id, .. }) => Some(entity_id.clone()),
            _ => None,
        };
        for child in named_children(node) {
            self.visit(child, inner, class_context.as_deref());
        }
    }

    fn visit_variable_declaration(&mut self, node: Node, scope: usize) {
        for declarator in named_children(node) {
            if declarator.kind() != "variable_declarator" {
                self.visit(declarator, scope, None);
                continue;
            }
            let name_node = declarator.child_by_field_name("name");
            let value = declarator.child_by_field_name("value");

            match name_node {
                Some(name_node) if name_node.kind() == "identifier" => {
                    let name = self.text(name_node);
                    let binding = if let Some(binding) =
                        self.symbol_binding(declarator.start_byte())
                    {
                        // A declarator-bound arrow or function expression: a real entity.
                        binding
                    } else if let Some(specifier) = value.and_then(|v| self.require_specifier(v)) {
                        Binding::ImportedNamespace { specifier }
                    } else {
                        Binding::Opaque
                    };
                    self.bind(scope, name, binding);
                }
                Some(pattern) => self.bind_pattern(pattern, scope),
                None => {}
            }

            for child in named_children(declarator) {
                self.visit(child, scope, None);
            }
        }
    }

    /// The literal specifier of `require('...')`, when that is exactly what the node is.
    fn require_specifier(&self, node: Node) -> Option<String> {
        if node.kind() != "call_expression" {
            return None;
        }
        let function = node.child_by_field_name("function")?;
        if function.kind() != "identifier" || self.text(function) != "require" {
            return None;
        }
        let arguments = node.child_by_field_name("arguments")?;
        let argument = named_children(arguments)
            .into_iter()
            .find(|argument| argument.kind() == "string")?;
        for child in named_children(argument) {
            if child.kind() == "string_fragment" {
                return Some(self.text(child));
            }
        }
        None
    }

    fn bind_opaque_name(&mut self, node: Node, scope: usize) {
        if let Some(name) = node.child_by_field_name("name") {
            if name.kind() == "identifier" {
                let name = self.text(name);
                self.bind(scope, name, Binding::Opaque);
            }
        }
    }

    fn visit(&mut self, node: Node, scope: usize, enclosing_class: Option<&str>) {
        match node.kind() {
            // Import bindings come from the structural extraction, not from a second walk.
            "import_statement" => {}

            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                if let (Some(name_node), Some(binding)) = (
                    node.child_by_field_name("name"),
                    self.symbol_binding(node.start_byte()),
                ) {
                    let name = self.text(name_node);
                    self.bind(scope, name, binding);
                }
                self.function_scope(node, scope, None);
            }

            "function_expression" | "generator_function" | "arrow_function" => {
                self.function_scope(node, scope, None);
            }

            "method_definition" => {
                self.function_scope(node, scope, enclosing_class);
            }

            "class_declaration" | "abstract_class_declaration" | "class" => {
                self.visit_class(node, scope);
            }

            "interface_declaration" => {
                if let (Some(name_node), Some(binding)) = (
                    node.child_by_field_name("name"),
                    self.symbol_binding(node.start_byte()),
                ) {
                    let name = self.text(name_node);
                    self.bind(scope, name, binding);
                }
                let inner = self.push_scope(ScopeKind::Block, node, scope, ThisBinding::Inherit);
                self.bind_parameters(node, inner);
                for child in named_children(node) {
                    self.visit(child, inner, None);
                }
            }

            // Names Nerve does not model as entities. They still shadow.
            "enum_declaration" | "internal_module" | "module" => {
                self.bind_opaque_name(node, scope);
                let inner = self.push_scope(ScopeKind::Block, node, scope, ThisBinding::Inherit);
                for child in named_children(node) {
                    self.visit(child, inner, None);
                }
            }
            "type_alias_declaration" => {
                self.bind_opaque_name(node, scope);
                let inner = self.push_scope(ScopeKind::Block, node, scope, ThisBinding::Inherit);
                self.bind_parameters(node, inner);
                for child in named_children(node) {
                    self.visit(child, inner, None);
                }
            }

            "lexical_declaration" | "variable_declaration" => {
                self.visit_variable_declaration(node, scope);
            }

            "statement_block" => {
                let inner = self.push_scope(ScopeKind::Block, node, scope, ThisBinding::Inherit);
                for child in named_children(node) {
                    self.visit(child, inner, enclosing_class);
                }
            }

            "catch_clause" => {
                let inner = self.push_scope(ScopeKind::Catch, node, scope, ThisBinding::Inherit);
                if let Some(parameter) = node.child_by_field_name("parameter") {
                    self.bind_pattern(parameter, inner);
                }
                for child in named_children(node) {
                    self.visit(child, inner, enclosing_class);
                }
            }

            "for_statement" | "for_in_statement" => {
                let inner = self.push_scope(ScopeKind::Block, node, scope, ThisBinding::Inherit);
                // `for (const item of xs)` binds `item` without a declarator node.
                if let Some(left) = node.child_by_field_name("left") {
                    self.bind_pattern(left, inner);
                }
                for child in named_children(node) {
                    self.visit(child, inner, enclosing_class);
                }
            }

            // An object literal in a class body is not the class: a method inside it rebinds
            // `this` to the object, so the class context must not leak into it.
            "object" => {
                for child in named_children(node) {
                    self.visit(child, scope, None);
                }
            }

            _ => {
                for child in named_children(node) {
                    self.visit(child, scope, enclosing_class);
                }
            }
        }
    }
}

impl BindingTable {
    /// Build the scope chain for one module.
    pub fn build(extraction: &ModuleExtraction, root: Node, source: &[u8]) -> BindingTable {
        let mut symbol_at: BTreeMap<usize, usize> = BTreeMap::new();
        for (index, symbol) in extraction.symbols.iter().enumerate() {
            // First declaration at a byte offset wins; identical offsets cannot occur for two
            // different declarations.
            symbol_at.entry(symbol.span.start_byte).or_insert(index);
        }

        let mut builder = Builder {
            source,
            extraction,
            symbol_at,
            scopes: vec![ScopeNode {
                parent: None,
                kind: ScopeKind::Module,
                // Deliberately 0, not `root.start_byte()`: leading trivia must still land in
                // the module scope rather than fall off the front of the search.
                start_byte: 0,
                end_byte: usize::MAX,
                bindings: BTreeMap::new(),
                this_binding: ThisBinding::Module,
            }],
        };

        // Import bindings are module-scoped and come from what the structural extractor read.
        for import in &extraction.imports {
            if import.form != ImportForm::Static {
                continue;
            }
            for specifier in &import.specifiers {
                let Some(local) = specifier.local.clone() else {
                    continue;
                };
                let binding = match specifier.kind {
                    "default" => Binding::ImportedDefault {
                        specifier: import.raw_specifier.clone(),
                    },
                    "namespace" => Binding::ImportedNamespace {
                        specifier: import.raw_specifier.clone(),
                    },
                    "named" => match &specifier.imported {
                        Some(imported) => Binding::ImportedNamed {
                            specifier: import.raw_specifier.clone(),
                            imported: imported.clone(),
                        },
                        None => continue,
                    },
                    _ => continue,
                };
                builder.bind(0, local, binding);
            }
        }

        for child in named_children(root) {
            builder.visit(child, 0, None);
        }

        BindingTable {
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
        // `binary_search_by` may land on any of several scopes starting at the same offset.
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

    /// Resolve a name from `scope` outwards. The first scope that binds it wins.
    pub fn lookup(&self, scope: usize, name: &str) -> Option<&Binding> {
        let mut current = Some(scope);
        while let Some(index) = current {
            if let Some(binding) = self.scopes[index].bindings.get(name) {
                return Some(binding);
            }
            current = self.scopes[index].parent;
        }
        None
    }

    /// What `this` refers to at `scope` (P3).
    pub fn this_at(&self, scope: usize) -> ThisResolution {
        let mut current = Some(scope);
        let mut crossed_function_boundary = false;
        while let Some(index) = current {
            match &self.scopes[index].this_binding {
                ThisBinding::Inherit => {}
                ThisBinding::Method {
                    class_entity_id,
                    is_static,
                } => {
                    return if crossed_function_boundary {
                        ThisResolution::ReboundByNestedFunction
                    } else {
                        ThisResolution::Class {
                            entity_id: class_entity_id.clone(),
                            is_static: *is_static,
                        }
                    };
                }
                ThisBinding::Rebound => crossed_function_boundary = true,
                ThisBinding::Module => return ThisResolution::NotInClassMethod,
            }
            current = self.scopes[index].parent;
        }
        ThisResolution::NotInClassMethod
    }

    /// Kind of a scope, for tests and diagnostics.
    pub fn kind_of(&self, scope: usize) -> ScopeKind {
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
    use crate::extract::extract_module;
    use crate::lang::Language;

    const PID: &str = "00000000000000000000000000000001";

    struct Fixture {
        extraction: ModuleExtraction,
        table: BindingTable,
        source: String,
    }

    fn build(source: &str) -> Fixture {
        build_as(source, "src/a.ts", Language::TypeScript)
    }

    fn build_as(source: &str, rel_path: &str, language: Language) -> Fixture {
        let extraction = extract_module(PID, rel_path, language, source).unwrap();
        let mut parser = language.parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        // The tree is dropped at the end of this function, so the table must not borrow it.
        let table = BindingTable::build(&extraction, tree.root_node(), source.as_bytes());
        Fixture {
            extraction,
            table,
            source: source.to_string(),
        }
    }

    /// Byte offset of the `needle`th occurrence (0-based) of `text`.
    fn offset_of(fixture: &Fixture, text: &str, occurrence: usize) -> usize {
        let mut from = 0;
        for _ in 0..=occurrence {
            let found = fixture.source[from..]
                .find(text)
                .unwrap_or_else(|| panic!("{text:?} occurrence {occurrence} not found"));
            from += found + 1;
        }
        from - 1
    }

    fn lookup_at<'a>(fixture: &'a Fixture, at: usize, name: &str) -> Option<&'a Binding> {
        let scope = fixture.table.scope_at(at);
        fixture.table.lookup(scope, name)
    }

    #[test]
    fn a_module_function_binds_at_module_scope() {
        let fixture = build("function add() {}\nadd();\n");
        let at = offset_of(&fixture, "add();", 0);
        assert!(matches!(
            lookup_at(&fixture, at, "add"),
            Some(Binding::LocalSymbol {
                kind: EntityKind::Function,
                ..
            })
        ));
    }

    #[test]
    fn a_call_resolves_to_a_function_declared_later_in_the_same_scope() {
        let fixture = build("function a() { b(); }\nfunction b() {}\n");
        let at = offset_of(&fixture, "b();", 0);
        assert!(matches!(
            lookup_at(&fixture, at, "b"),
            Some(Binding::LocalSymbol { .. })
        ));
    }

    #[test]
    fn a_parameter_shadows_a_module_function() {
        let fixture = build("function foo() {}\nfunction bar(foo) { foo(); }\n");
        let at = offset_of(&fixture, "foo();", 0);
        assert_eq!(lookup_at(&fixture, at, "foo"), Some(&Binding::Opaque));
        // Outside `bar`'s parameter list, the module function is still visible. A function
        // scope starts at the `function` keyword, so "outside" means module top level.
        assert!(matches!(
            lookup_at(&fixture, 0, "foo"),
            Some(Binding::LocalSymbol { .. })
        ));
    }

    #[test]
    fn a_const_shadows_an_import() {
        let fixture =
            build("import { add } from './math';\nfunction f() { const add = 1; add; }\n");
        let at = offset_of(&fixture, "add;", 0);
        assert_eq!(lookup_at(&fixture, at, "add"), Some(&Binding::Opaque));
        let outside = offset_of(&fixture, "function f", 0);
        assert!(matches!(
            lookup_at(&fixture, outside, "add"),
            Some(Binding::ImportedNamed { .. })
        ));
    }

    #[test]
    fn destructuring_catch_and_loop_bindings_are_opaque() {
        let fixture = build(
            "function f() {}\n\
             function g() { const { f } = o; f(); }\n\
             function h() { try {} catch (f) { f(); } }\n\
             function i() { for (const f of xs) { f(); } }\n\
             function j() { const [f] = xs; f(); }\n",
        );
        for occurrence in 0..4 {
            let at = offset_of(&fixture, "f();", occurrence);
            assert_eq!(
                lookup_at(&fixture, at, "f"),
                Some(&Binding::Opaque),
                "occurrence {occurrence} did not shadow"
            );
        }
    }

    #[test]
    fn type_parameters_shadow_a_module_type() {
        let fixture = build("interface T {}\nfunction f<T>(t: T): T { return t; }\n");
        let at = offset_of(&fixture, "return t", 0);
        assert_eq!(lookup_at(&fixture, at, "T"), Some(&Binding::Opaque));
    }

    #[test]
    fn class_type_parameters_shadow_a_module_type() {
        let fixture = build("interface Shape {}\nclass Box<Shape> { m(x: Shape) { return x; } }\n");
        let at = offset_of(&fixture, "return x", 0);
        assert_eq!(lookup_at(&fixture, at, "Shape"), Some(&Binding::Opaque));
    }

    #[test]
    fn import_forms_bind_distinctly() {
        let fixture = build(
            "import def, { a as b } from './math';\nimport * as ns from './shapes';\nconst m = require('./legacy');\n",
        );
        let at = fixture.source.len() - 1;
        assert_eq!(
            lookup_at(&fixture, at, "def"),
            Some(&Binding::ImportedDefault {
                specifier: "./math".into()
            })
        );
        assert_eq!(
            lookup_at(&fixture, at, "b"),
            Some(&Binding::ImportedNamed {
                specifier: "./math".into(),
                imported: "a".into()
            })
        );
        assert_eq!(
            lookup_at(&fixture, at, "ns"),
            Some(&Binding::ImportedNamespace {
                specifier: "./shapes".into()
            })
        );
        assert_eq!(
            lookup_at(&fixture, at, "m"),
            Some(&Binding::ImportedNamespace {
                specifier: "./legacy".into()
            })
        );
    }

    #[test]
    fn a_plain_const_is_opaque_even_when_it_is_a_call_result() {
        let fixture = build("const m = makeThing('./x');\nm.go();\n");
        let at = offset_of(&fixture, "m.go", 0);
        assert_eq!(lookup_at(&fixture, at, "m"), Some(&Binding::Opaque));
    }

    #[test]
    fn this_inside_a_method_binds_to_its_class() {
        let fixture = build("class C { m() { this.n(); } n() {} }\n");
        let at = offset_of(&fixture, "this.n", 0);
        let scope = fixture.table.scope_at(at);
        let class_id = &fixture.extraction.symbols[0].entity_id;
        assert_eq!(
            fixture.table.this_at(scope),
            ThisResolution::Class {
                entity_id: class_id.clone(),
                is_static: false
            }
        );
    }

    #[test]
    fn this_inside_a_nested_function_in_a_method_is_rebound() {
        let fixture = build("class C { m() { function inner() { this.n(); } } n() {} }\n");
        let at = offset_of(&fixture, "this.n", 0);
        let scope = fixture.table.scope_at(at);
        assert_eq!(
            fixture.table.this_at(scope),
            ThisResolution::ReboundByNestedFunction
        );
    }

    #[test]
    fn this_inside_an_arrow_in_a_method_still_binds_to_the_class() {
        let fixture = build("class C { m() { const f = () => this.n(); } n() {} }\n");
        let at = offset_of(&fixture, "this.n", 0);
        let scope = fixture.table.scope_at(at);
        assert!(matches!(
            fixture.table.this_at(scope),
            ThisResolution::Class {
                is_static: false,
                ..
            }
        ));
    }

    #[test]
    fn this_at_module_level_is_not_a_class() {
        let fixture = build("this.go();\n");
        let scope = fixture.table.scope_at(0);
        assert_eq!(
            fixture.table.this_at(scope),
            ThisResolution::NotInClassMethod
        );
    }

    #[test]
    fn this_inside_a_static_method_records_staticness() {
        let fixture = build("class C { static m() { this.n(); } static n() {} }\n");
        let at = offset_of(&fixture, "this.n", 0);
        let scope = fixture.table.scope_at(at);
        assert!(matches!(
            fixture.table.this_at(scope),
            ThisResolution::Class {
                is_static: true,
                ..
            }
        ));
    }

    #[test]
    fn javascript_classes_and_parameters_bind_the_same_way() {
        let fixture = build_as(
            "function foo() {}\nclass A extends B { m(foo) { foo(); } }\n",
            "src/a.js",
            Language::JavaScript,
        );
        let at = offset_of(&fixture, "foo();", 0);
        assert_eq!(lookup_at(&fixture, at, "foo"), Some(&Binding::Opaque));
    }

    #[test]
    fn scope_lookup_is_position_sensitive_across_siblings() {
        let fixture = build(
            "function shared() {}\n\
             function a() { function shared() {} shared(); }\n\
             function b() { shared(); }\n",
        );
        let inside_a = offset_of(&fixture, "shared();", 0);
        let inside_b = offset_of(&fixture, "shared();", 1);
        let a_binding = lookup_at(&fixture, inside_a, "shared").cloned();
        let b_binding = lookup_at(&fixture, inside_b, "shared").cloned();
        assert_ne!(a_binding, b_binding, "sibling scopes must not share");
    }

    #[test]
    fn a_class_name_is_visible_inside_its_own_body() {
        let fixture = build("class C { m() { return new C(); } }\n");
        let at = offset_of(&fixture, "new C", 0);
        assert!(matches!(
            lookup_at(&fixture, at, "C"),
            Some(Binding::LocalSymbol {
                kind: EntityKind::Class,
                ..
            })
        ));
    }

    #[test]
    fn enums_and_namespaces_shadow_without_becoming_entities() {
        let fixture = build("function E() {}\nenum E { A }\nE;\n");
        let at = fixture.source.len() - 2;
        assert_eq!(lookup_at(&fixture, at, "E"), Some(&Binding::Opaque));
    }

    /// `scope_at` binary-searches, which is only correct if scopes are pushed in pre-order.
    /// This asserts the invariant on a file that exercises every scope-creating construct.
    #[test]
    fn scopes_are_recorded_in_non_decreasing_start_order() {
        let fixture = build(
            "import { a } from './m';\n\
             enum E { X }\n\
             type Alias<T> = T;\n\
             interface I<T> { m(): T }\n\
             class C<T> extends B implements I<T> {\n\
               static s() { return 1; }\n\
               m(p = 1) { const q = () => { for (const r of []) { try { r(); } catch (e) { e; } } }; return q; }\n\
             }\n\
             namespace N { export const y = 1; }\n\
             function f(g = function inner() { return 1; }) { { let h = 2; } return g; }\n\
             const k = function* () { yield 1; };\n",
        );
        assert!(fixture.table.len() > 10, "not enough scopes to be a test");
        for pair in fixture.table.scopes.windows(2) {
            assert!(
                pair[0].start_byte <= pair[1].start_byte,
                "scopes are out of order: {:?} then {:?}",
                pair[0].start_byte,
                pair[1].start_byte
            );
        }
        // Every scope except the module scope is strictly inside its parent.
        for (index, scope) in fixture.table.scopes.iter().enumerate().skip(1) {
            let parent = &fixture.table.scopes[scope.parent.unwrap()];
            assert!(
                parent.start_byte <= scope.start_byte && scope.end_byte <= parent.end_byte,
                "scope {index} is not nested inside its parent"
            );
        }
    }

    #[test]
    fn building_the_table_twice_gives_the_same_answers() {
        let source = "import { a } from './m';\nfunction f(b) { return a(b); }\n";
        let first = build(source);
        let second = build(source);
        assert_eq!(first.table.len(), second.table.len());
        let at = offset_of(&first, "a(b)", 0);
        assert_eq!(
            lookup_at(&first, at, "a").cloned(),
            lookup_at(&second, at, "a").cloned()
        );
    }
}
