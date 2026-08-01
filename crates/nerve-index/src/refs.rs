//! The `ts-js-reference` extractor: `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`.
//!
//! Every edge it produces is either **resolved** — a binding in [`crate::bind`] plus, where the
//! binding is an import, [`crate::resolve`] and [`crate::exports`] — or an explicit
//! [`RefTarget::Unresolved`] carrying a reason. There is no third category in which a name
//! match is presented as a resolution.
//!
//! Three deliberate asymmetries:
//!
//! - An unresolved **call** is emitted. "This code invokes something Nerve cannot name" is a
//!   fact worth recording and counting.
//! - An unresolved **reference** is not. A bare identifier that does not resolve is usually a
//!   local or a global and carries no such fact; emitting one per identifier would drown the
//!   graph and make precision unmeasurable (plan P4).
//! - Callee forms Nerve does not model — `a[b]()`, `f()()`, an IIFE, a tagged template, a
//!   `super` call, and heritage clauses that are not a plain name — produce **no edge at all**
//!   and are counted instead. A count is honest; an invented target is not.
//!
//! Observation `details` carry identifier and dotted-member **names** only. Names already live
//! in `entity.name`; arbitrary expression source does not, and SECURITY.md's "no source text at
//! rest" rule is kept true by construction rather than by review.

use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::Node;

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::{EntityKind, EvidenceSourceType, Relation};

use crate::bind::{Binding, BindingTable, ThisResolution};
use crate::error::Result;
use crate::exports::ExportIndex;
use crate::extract::ModuleExtraction;
use crate::lang::Language;
use crate::resolve;

/// Identifier of the reference extractor.
pub const EXTRACTOR_ID: &str = "ts-js-reference";

/// Version of the reference extractor. Bump on any change to what it emits.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The evidence source types this extractor is permitted to emit (ADR-0003).
///
/// `AST_RESOLVED` for edges produced by binding, module and export resolution; `AST_DIRECT` for
/// edges whose target is an `Unresolved` entity, because in that case the only thing the
/// evidence states is what the tree literally wrote.
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 2] = [
    EvidenceSourceType::AstDirect,
    EvidenceSourceType::AstResolved,
];

/// Why a reference could not be resolved. Closed vocabulary; recorded verbatim in evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnresolvedReason {
    /// The receiver of a member call is not a namespace binding, so the member cannot be named.
    ReceiverNotResolvable,
    /// `this` is not bound to a class declared in this module.
    ThisNotInClassMethod,
    /// A non-arrow `function` boundary rebound `this` between the site and the method.
    ThisReboundByNestedFunction,
    /// `this` is a class of this module, but that class does not itself declare the member.
    MethodNotDeclaredOnClass,
    /// The name is bound, but to a parameter, variable, pattern, type parameter or namespace.
    LocalBindingNotASymbol,
    /// Nothing in the scope chain binds the name.
    NameNotInScope,
    /// The name is imported, but the specifier names no indexed module.
    ImportModuleUnresolved,
    /// The name is imported from an indexed module that does not export it.
    ImportNameNotExported,
}

impl UnresolvedReason {
    /// Every reason, in declaration order.
    pub const ALL: [UnresolvedReason; 8] = [
        UnresolvedReason::ReceiverNotResolvable,
        UnresolvedReason::ThisNotInClassMethod,
        UnresolvedReason::ThisReboundByNestedFunction,
        UnresolvedReason::MethodNotDeclaredOnClass,
        UnresolvedReason::LocalBindingNotASymbol,
        UnresolvedReason::NameNotInScope,
        UnresolvedReason::ImportModuleUnresolved,
        UnresolvedReason::ImportNameNotExported,
    ];

    /// Canonical name recorded in evidence and in ground truth.
    pub fn as_str(self) -> &'static str {
        match self {
            UnresolvedReason::ReceiverNotResolvable => "receiver-not-resolvable",
            UnresolvedReason::ThisNotInClassMethod => "this-not-in-class-method",
            UnresolvedReason::ThisReboundByNestedFunction => "this-rebound-by-nested-function",
            UnresolvedReason::MethodNotDeclaredOnClass => "method-not-declared-on-class",
            UnresolvedReason::LocalBindingNotASymbol => "local-binding-not-a-symbol",
            UnresolvedReason::NameNotInScope => "name-not-in-scope",
            UnresolvedReason::ImportModuleUnresolved => "import-module-unresolved",
            UnresolvedReason::ImportNameNotExported => "import-name-not-exported",
        }
    }
}

/// Every form tag this extractor can record in `unmodelled_by_form`, in alphabetical order.
///
/// These are the call and heritage shapes Nerve declines to model, counted rather than guessed.
/// They are emitted as bare literals at the site that recognises each shape, so — unlike
/// [`UnresolvedReason`] — the set is not an enum and cannot be derived from the type system.
/// Listing it here makes it enumerable for consumers that must cover it, and
/// `ui_vocabulary.rs` reads this module's own source back to prove the list stayed complete.
pub const UNMODELLED_FORMS: [&str; 11] = [
    "call-result",
    "complex-receiver",
    "computed-member",
    "dynamic-import",
    "heritage-call",
    "heritage-other",
    "iife",
    "other",
    "require",
    "super",
    "tagged-template",
];

/// What a reference site points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    /// A resolved entity in this repository.
    Resolved(String),
    /// A target Nerve could not name.
    Unresolved {
        /// The name as written: an identifier, or `receiver.member`.
        name: String,
        /// Why resolution failed.
        reason: UnresolvedReason,
    },
}

/// One `CALLS` / `REFERENCES` / `EXTENDS` / `IMPLEMENTS` site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceSite {
    /// Innermost enclosing symbol entity, or the module entity.
    pub source_entity_id: String,
    /// Relation asserted.
    pub relation: Relation,
    /// Where it points.
    pub target: RefTarget,
    /// Source range of the referring token.
    pub span: Span,
    /// Evidence detail. Names and enumerated tags only — never expression source.
    pub details: serde_json::Value,
}

/// Everything the reference extractor found in one module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Reference sites, in source order.
    pub sites: Vec<ReferenceSite>,
    /// Call sites and heritage clauses whose form Nerve does not model. No edge was produced.
    pub unmodelled_call_sites: usize,
    /// Breakdown of the above by form tag, for reporting and tests.
    pub unmodelled_by_form: BTreeMap<String, usize>,
}

/// Outcome of resolving one name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    Entity(String),
    Failed(UnresolvedReason),
}

/// A resolution plus the evidence detail explaining how it was reached.
#[derive(Debug, Clone)]
struct Lookup {
    outcome: Resolution,
    binding: &'static str,
    resolved_module: Option<String>,
}

/// The declaration a site sits inside.
#[derive(Debug, Clone)]
enum Owner {
    /// A named declaration with an entity.
    Entity(String),
    /// A function literal Nerve did not name; sites inside it belong to the module.
    Anonymous,
}

fn span_of(node: Node) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: start.row + 1,
        start_col: start.column,
        end_line: end.row + 1,
        end_col: end.column,
    }
}

fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn all_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

/// Evidence detail shared by every `CALLS` site. Names and enumerated tags only.
fn call_details(
    call_form: &str,
    callee_form: &str,
    callee_name: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut details = serde_json::Map::new();
    details.insert("call_form".into(), call_form.into());
    details.insert("callee_form".into(), callee_form.into());
    details.insert("callee_name".into(), callee_name.into());
    details
}

struct Walker<'a> {
    source: &'a [u8],
    project_id: &'a str,
    rel_path: &'a str,
    extraction: &'a ModuleExtraction,
    exports: &'a ExportIndex,
    indexed: &'a BTreeSet<String>,
    table: BindingTable,
    module_entity_id: String,
    /// Declaration start byte -> index into `extraction.symbols`.
    symbol_at: BTreeMap<usize, usize>,
    /// (class entity id, member name) -> (method entity id, is_static).
    class_members: BTreeMap<(String, String), (String, bool)>,
    owners: Vec<Owner>,
    out: ReferenceExtraction,
}

impl Walker<'_> {
    fn text(&self, node: Node) -> String {
        std::str::from_utf8(&self.source[node.byte_range()])
            .unwrap_or_default()
            .to_string()
    }

    fn source_entity_id(&self) -> String {
        match self.owners.last() {
            Some(Owner::Entity(entity_id)) => entity_id.clone(),
            _ => self.module_entity_id.clone(),
        }
    }

    fn unmodelled(&mut self, form: &str) {
        self.out.unmodelled_call_sites += 1;
        *self
            .out
            .unmodelled_by_form
            .entry(form.to_string())
            .or_insert(0) += 1;
    }

    fn owner_for(&self, node: Node) -> Owner {
        match self.symbol_at.get(&node.start_byte()) {
            Some(index) => Owner::Entity(self.extraction.symbols[*index].entity_id.clone()),
            None => Owner::Anonymous,
        }
    }

    // ---- resolution ----------------------------------------------------------------------

    fn resolve_export(&self, specifier: &str, name: &str) -> Lookup {
        let binding = "import";
        match resolve::resolve(self.rel_path, specifier, self.indexed) {
            None => Lookup {
                outcome: Resolution::Failed(UnresolvedReason::ImportModuleUnresolved),
                binding,
                resolved_module: None,
            },
            Some(module) => match self.exports.resolve(&module, name) {
                Some(entity_id) => Lookup {
                    outcome: Resolution::Entity(entity_id),
                    binding,
                    resolved_module: Some(module),
                },
                None => Lookup {
                    outcome: Resolution::Failed(UnresolvedReason::ImportNameNotExported),
                    binding,
                    resolved_module: Some(module),
                },
            },
        }
    }

    fn resolve_binding(&self, binding: &Binding) -> Lookup {
        match binding {
            Binding::LocalSymbol { entity_id, .. } => Lookup {
                outcome: Resolution::Entity(entity_id.clone()),
                binding: "local",
                resolved_module: None,
            },
            Binding::ImportedNamed {
                specifier,
                imported,
            } => Lookup {
                binding: "import-named",
                ..self.resolve_export(specifier, imported)
            },
            Binding::ImportedDefault { specifier } => Lookup {
                binding: "import-default",
                ..self.resolve_export(specifier, "default")
            },
            Binding::ImportedNamespace { specifier } => {
                // A namespace binding names the module itself.
                match resolve::resolve(self.rel_path, specifier, self.indexed) {
                    Some(module) => Lookup {
                        outcome: Resolution::Entity(ids::module_id(self.project_id, &module)),
                        binding: "import-namespace",
                        resolved_module: Some(module),
                    },
                    None => Lookup {
                        outcome: Resolution::Failed(UnresolvedReason::ImportModuleUnresolved),
                        binding: "import-namespace",
                        resolved_module: None,
                    },
                }
            }
            Binding::Opaque => Lookup {
                outcome: Resolution::Failed(UnresolvedReason::LocalBindingNotASymbol),
                binding: "opaque",
                resolved_module: None,
            },
        }
    }

    /// Resolve a bare name as written at `byte`.
    fn resolve_name(&self, name: &str, byte: usize) -> Lookup {
        let scope = self.table.scope_at(byte);
        match self.table.lookup(scope, name) {
            Some(binding) => self.resolve_binding(binding),
            None => Lookup {
                outcome: Resolution::Failed(UnresolvedReason::NameNotInScope),
                binding: "none",
                resolved_module: None,
            },
        }
    }

    /// Resolve `receiver.member`, which only succeeds when `receiver` is a namespace binding.
    fn resolve_qualified(&self, receiver: &str, member: &str, byte: usize) -> Lookup {
        let scope = self.table.scope_at(byte);
        match self.table.lookup(scope, receiver) {
            Some(Binding::ImportedNamespace { specifier }) => Lookup {
                binding: "import-namespace",
                ..self.resolve_export(specifier, member)
            },
            // Any other receiver would require knowing its type. Nerve does not, and will not
            // guess: the type of `shape` in `shape.area()` is exactly the information a static
            // binding table does not have.
            _ => Lookup {
                outcome: Resolution::Failed(UnresolvedReason::ReceiverNotResolvable),
                binding: "none",
                resolved_module: None,
            },
        }
    }

    // ---- emission ------------------------------------------------------------------------

    fn emit(
        &mut self,
        relation: Relation,
        lookup: Lookup,
        name: String,
        span: Span,
        mut details: serde_json::Map<String, serde_json::Value>,
    ) {
        details.insert("binding".into(), lookup.binding.into());
        details.insert(
            "resolved_module".into(),
            match &lookup.resolved_module {
                Some(module) => module.as_str().into(),
                None => serde_json::Value::Null,
            },
        );

        let target = match lookup.outcome {
            Resolution::Entity(entity_id) => {
                details.insert("reason".into(), serde_json::Value::Null);
                RefTarget::Resolved(entity_id)
            }
            Resolution::Failed(reason) => {
                // P4: an unresolved bare reference states nothing worth an edge.
                if relation == Relation::References {
                    return;
                }
                details.insert("reason".into(), reason.as_str().into());
                RefTarget::Unresolved {
                    name: name.clone(),
                    reason,
                }
            }
        };

        let source_entity_id = self.source_entity_id();
        self.out.sites.push(ReferenceSite {
            source_entity_id,
            relation,
            target,
            span,
            details: serde_json::Value::Object(details),
        });
    }

    // ---- calls ---------------------------------------------------------------------------

    fn visit_call(&mut self, node: Node) {
        let Some(callee) = node.child_by_field_name("function") else {
            self.unmodelled("other");
            self.visit_children_skipping(node, &[]);
            return;
        };
        let arguments = node.child_by_field_name("arguments");

        // `tag`x`` is a call whose arguments are a template, not an argument list.
        if arguments.map(|node| node.kind()) == Some("template_string") {
            self.unmodelled("tagged-template");
            self.visit_children_skipping(node, &[]);
            return;
        }

        let mut consumed_callee = true;
        match callee.kind() {
            "identifier" => {
                let name = self.text(callee);
                // `require('./x')` and `import('./x')` are module-system operators; the
                // structural extractor already records them as IMPORTS. Re-reporting them as
                // calls to a global named `require` would be noise, so they are counted.
                if name == "require" && self.has_string_argument(node) {
                    self.unmodelled("require");
                } else {
                    let lookup = self.resolve_name(&name, callee.start_byte());
                    let details = call_details("call", "identifier", &name);
                    self.emit(Relation::Calls, lookup, name, span_of(callee), details);
                }
            }
            "import" => self.unmodelled("dynamic-import"),
            "super" => self.unmodelled("super"),
            "member_expression" => consumed_callee = self.visit_member_callee(callee, "call"),
            "subscript_expression" => {
                self.unmodelled("computed-member");
                consumed_callee = false;
            }
            "call_expression" => {
                self.unmodelled("call-result");
                consumed_callee = false;
            }
            "parenthesized_expression"
            | "function_expression"
            | "arrow_function"
            | "generator_function" => {
                self.unmodelled("iife");
                consumed_callee = false;
            }
            _ => {
                self.unmodelled("other");
                consumed_callee = false;
            }
        }

        // The callee is CALLS territory and never also a REFERENCES site. An unmodelled callee
        // is different: it can contain calls of its own, so its subtree is still walked.
        let skip = if consumed_callee {
            vec![callee.id()]
        } else {
            vec![]
        };
        for child in named_children(node) {
            if skip.contains(&child.id()) {
                continue;
            }
            self.visit(child);
        }
    }

    fn visit_new(&mut self, node: Node) {
        let Some(callee) = node.child_by_field_name("constructor") else {
            self.unmodelled("other");
            self.visit_children_skipping(node, &[]);
            return;
        };

        let mut consumed_callee = true;
        match callee.kind() {
            "identifier" => {
                let name = self.text(callee);
                let lookup = self.resolve_name(&name, callee.start_byte());
                let details = call_details("new", "identifier", &name);
                self.emit(Relation::Calls, lookup, name, span_of(callee), details);
            }
            "member_expression" => consumed_callee = self.visit_member_callee(callee, "new"),
            _ => {
                self.unmodelled("other");
                consumed_callee = false;
            }
        }

        let skip = if consumed_callee {
            vec![callee.id()]
        } else {
            vec![]
        };
        for child in named_children(node) {
            if skip.contains(&child.id()) {
                continue;
            }
            self.visit(child);
        }
    }

    /// Handle a `member_expression` in callee position. Returns whether the callee was consumed.
    fn visit_member_callee(&mut self, callee: Node, call_form: &str) -> bool {
        let (Some(object), Some(property)) = (
            callee.child_by_field_name("object"),
            callee.child_by_field_name("property"),
        ) else {
            self.unmodelled("other");
            return false;
        };
        let member = self.text(property);

        match object.kind() {
            "this" => {
                let name = format!("this.{member}");
                let lookup = self.resolve_this_member(&member, callee.start_byte());
                let details = call_details(call_form, "this-member", &name);
                self.emit(Relation::Calls, lookup, name, span_of(callee), details);
                true
            }
            "identifier" => {
                let receiver = self.text(object);
                let name = format!("{receiver}.{member}");
                let lookup = self.resolve_qualified(&receiver, &member, object.start_byte());
                let callee_form = if lookup.binding == "import-namespace" {
                    "namespace-member"
                } else {
                    "member"
                };
                let details = call_details(call_form, callee_form, &name);
                self.emit(Relation::Calls, lookup, name, span_of(callee), details);
                true
            }
            // `a.b.c()`, `f().g()`, `this.x.y()`: naming the target would require a type.
            _ => {
                self.unmodelled("complex-receiver");
                false
            }
        }
    }

    /// The P3 rule: `this.m()` resolves only inside a method of a class that itself declares
    /// `m`, with no non-arrow `function` boundary in between and matching staticness.
    fn resolve_this_member(&self, member: &str, byte: usize) -> Lookup {
        let scope = self.table.scope_at(byte);
        match self.table.this_at(scope) {
            ThisResolution::NotInClassMethod => Lookup {
                outcome: Resolution::Failed(UnresolvedReason::ThisNotInClassMethod),
                binding: "this",
                resolved_module: None,
            },
            ThisResolution::ReboundByNestedFunction => Lookup {
                outcome: Resolution::Failed(UnresolvedReason::ThisReboundByNestedFunction),
                binding: "this",
                resolved_module: None,
            },
            ThisResolution::Class {
                entity_id,
                is_static,
            } => match self.class_members.get(&(entity_id, member.to_string())) {
                Some((method_entity_id, member_is_static)) if *member_is_static == is_static => {
                    Lookup {
                        outcome: Resolution::Entity(method_entity_id.clone()),
                        binding: "this",
                        resolved_module: None,
                    }
                }
                // Inherited from a base class, declared as a field, or a staticness mismatch.
                _ => Lookup {
                    outcome: Resolution::Failed(UnresolvedReason::MethodNotDeclaredOnClass),
                    binding: "this",
                    resolved_module: None,
                },
            },
        }
    }

    fn has_string_argument(&self, node: Node) -> bool {
        node.child_by_field_name("arguments")
            .map(|arguments| {
                named_children(arguments)
                    .iter()
                    .any(|argument| argument.kind() == "string")
            })
            .unwrap_or(false)
    }

    // ---- heritage ------------------------------------------------------------------------

    fn visit_class_heritage(&mut self, heritage: Node) {
        for child in all_children(heritage) {
            match child.kind() {
                "extends_clause" => {
                    let generic = child.child_by_field_name("type_arguments").is_some();
                    match child.child_by_field_name("value") {
                        Some(value) => self.heritage_value(value, Relation::Extends, generic),
                        None => self.unmodelled("heritage-other"),
                    }
                }
                "implements_clause" => {
                    for entry in named_children(child) {
                        self.heritage_type(entry, Relation::Implements);
                    }
                }
                // JavaScript has no `extends_clause`: the superclass sits under class_heritage.
                other if child.is_named() && other != "comment" => {
                    self.heritage_value(child, Relation::Extends, false)
                }
                _ => {}
            }
        }
    }

    /// A heritage entry written as an expression (`class A extends B`).
    fn heritage_value(&mut self, node: Node, relation: Relation, generic: bool) {
        match node.kind() {
            "identifier" | "type_identifier" => {
                let name = self.text(node);
                let lookup = self.resolve_name(&name, node.start_byte());
                self.emit_heritage(relation, lookup, name, node, "identifier", generic);
            }
            "member_expression" => match (
                node.child_by_field_name("object"),
                node.child_by_field_name("property"),
            ) {
                (Some(object), Some(property)) if object.kind() == "identifier" => {
                    let receiver = self.text(object);
                    let member = self.text(property);
                    let name = format!("{receiver}.{member}");
                    let lookup = self.resolve_qualified(&receiver, &member, object.start_byte());
                    self.emit_heritage(relation, lookup, name, node, "qualified", generic);
                }
                _ => self.unmodelled("heritage-other"),
            },
            // `class A extends mixin(B)`: the superclass is computed. No EXTENDS edge is
            // invented; the call itself is still a call site and is visited.
            "call_expression" => {
                self.unmodelled("heritage-call");
                self.visit(node);
            }
            _ => {
                self.unmodelled("heritage-other");
                self.visit(node);
            }
        }
    }

    /// A heritage entry written as a type (`implements I`, `interface I extends J`).
    fn heritage_type(&mut self, node: Node, relation: Relation) {
        match node.kind() {
            "type_identifier" | "identifier" => self.heritage_value(node, relation, false),
            "generic_type" => match node.child_by_field_name("name") {
                // Generic arguments are stripped to the head identifier.
                Some(name) => self.heritage_type_head(name, relation),
                None => self.unmodelled("heritage-other"),
            },
            "nested_type_identifier" => self.heritage_type_head(node, relation),
            "comment" => {}
            _ => self.unmodelled("heritage-other"),
        }
    }

    fn heritage_type_head(&mut self, node: Node, relation: Relation) {
        match node.kind() {
            "type_identifier" | "identifier" => {
                let name = self.text(node);
                let lookup = self.resolve_name(&name, node.start_byte());
                self.emit_heritage(relation, lookup, name, node, "identifier", true);
            }
            "nested_type_identifier" => {
                match (
                    node.child_by_field_name("module"),
                    node.child_by_field_name("name"),
                ) {
                    (Some(module), Some(name)) if module.kind() == "identifier" => {
                        let receiver = self.text(module);
                        let member = self.text(name);
                        let qualified = format!("{receiver}.{member}");
                        let lookup =
                            self.resolve_qualified(&receiver, &member, module.start_byte());
                        self.emit_heritage(relation, lookup, qualified, node, "qualified", true);
                    }
                    _ => self.unmodelled("heritage-other"),
                }
            }
            _ => self.unmodelled("heritage-other"),
        }
    }

    fn emit_heritage(
        &mut self,
        relation: Relation,
        lookup: Lookup,
        name: String,
        node: Node,
        form: &str,
        generic: bool,
    ) {
        let mut details = serde_json::Map::new();
        details.insert("heritage_form".into(), form.into());
        details.insert("heritage_name".into(), name.as_str().into());
        details.insert("generic".into(), generic.into());
        self.emit(relation, lookup, name, span_of(node), details);
    }

    // ---- traversal -----------------------------------------------------------------------

    fn visit_children_skipping(&mut self, node: Node, skip_fields: &[&str]) {
        let skip: Vec<usize> = skip_fields
            .iter()
            .filter_map(|field| node.child_by_field_name(field))
            .map(|child| child.id())
            .collect();
        for child in named_children(node) {
            if skip.contains(&child.id()) {
                continue;
            }
            self.visit(child);
        }
    }

    fn visit_owned(&mut self, node: Node, owner: Owner, skip_fields: &[&str]) {
        self.owners.push(owner);
        self.visit_children_skipping(node, skip_fields);
        self.owners.pop();
    }

    fn visit_parameters(&mut self, node: Node) {
        for parameter in named_children(node) {
            match parameter.kind() {
                "required_parameter" | "optional_parameter" => {
                    self.visit_children_skipping(parameter, &["pattern"]);
                }
                // A default value is a real expression; the binding target is not a reference.
                "assignment_pattern" => {
                    if let Some(right) = parameter.child_by_field_name("right") {
                        self.visit(right);
                    }
                }
                "identifier" | "object_pattern" | "array_pattern" | "rest_pattern" => {}
                _ => self.visit(parameter),
            }
        }
    }

    fn visit_class(&mut self, node: Node) {
        let owner = self.owner_for(node);
        self.owners.push(owner);
        let name_id = node.child_by_field_name("name").map(|child| child.id());
        for child in named_children(node) {
            if Some(child.id()) == name_id {
                continue;
            }
            if child.kind() == "class_heritage" {
                self.visit_class_heritage(child);
            } else {
                self.visit(child);
            }
        }
        self.owners.pop();
    }

    fn visit_interface(&mut self, node: Node) {
        let owner = self.owner_for(node);
        self.owners.push(owner);
        let name_id = node.child_by_field_name("name").map(|child| child.id());
        for child in named_children(node) {
            if Some(child.id()) == name_id {
                continue;
            }
            if child.kind() == "extends_type_clause" {
                for entry in named_children(child) {
                    self.heritage_type(entry, Relation::Extends);
                }
            } else {
                self.visit(child);
            }
        }
        self.owners.pop();
    }

    fn visit_variable_declarator(&mut self, node: Node) {
        match self.symbol_at.get(&node.start_byte()) {
            Some(index) => {
                let owner = Owner::Entity(self.extraction.symbols[*index].entity_id.clone());
                self.owners.push(owner);
                // The declarator-bound function literal is this entity's body, not a separate
                // anonymous one, so it is walked transparently.
                let value = node.child_by_field_name("value");
                let name_id = node.child_by_field_name("name").map(|child| child.id());
                for child in named_children(node) {
                    if Some(child.id()) == name_id {
                        continue;
                    }
                    if Some(child.id()) == value.map(|value| value.id())
                        && crate::bind::is_function_node(child.kind())
                    {
                        self.visit_children_skipping(child, &["name", "parameter"]);
                    } else {
                        self.visit(child);
                    }
                }
                self.owners.pop();
            }
            None => self.visit_children_skipping(node, &["name"]),
        }
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            // An import statement declares bindings; it references nothing.
            "import_statement" => {}

            "export_statement" => {
                if let Some(declaration) = node.child_by_field_name("declaration") {
                    self.visit(declaration);
                    return;
                }
                for child in named_children(node) {
                    match child.kind() {
                        // `export { a as b }`, `export * from './x'`, `export default foo`.
                        "export_clause" | "string" | "identifier" => {}
                        _ => self.visit(child),
                    }
                }
            }

            "call_expression" => self.visit_call(node),
            "new_expression" => self.visit_new(node),

            "class_declaration" | "abstract_class_declaration" | "class" => self.visit_class(node),
            "interface_declaration" => self.visit_interface(node),

            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                let owner = self.owner_for(node);
                self.visit_owned(node, owner, &["name"]);
            }
            "method_definition" => {
                let owner = self.owner_for(node);
                self.visit_owned(node, owner, &["name"]);
            }
            "function_expression" | "generator_function" | "arrow_function" => {
                self.visit_owned(node, Owner::Anonymous, &["name", "parameter"]);
            }

            "variable_declarator" => self.visit_variable_declarator(node),
            "formal_parameters" => self.visit_parameters(node),
            "type_parameter" => self.visit_children_skipping(node, &["name"]),

            "enum_declaration" | "type_alias_declaration" | "internal_module" | "module" => {
                self.visit_children_skipping(node, &["name"]);
            }
            "public_field_definition"
            | "property_signature"
            | "method_signature"
            | "abstract_method_signature" => {
                self.visit_children_skipping(node, &["name"]);
            }

            // The closing tag repeats the opening tag's name.
            "jsx_closing_element" => {}
            "labeled_statement" => self.visit_children_skipping(node, &["label"]),

            "identifier" => {
                let name = self.text(node);
                let lookup = self.resolve_name(&name, node.start_byte());
                let position = match node.parent().map(|parent| parent.kind()) {
                    Some("jsx_opening_element") | Some("jsx_self_closing_element") => "jsx",
                    _ => "value",
                };
                let mut details = serde_json::Map::new();
                details.insert("reference_name".into(), name.as_str().into());
                details.insert("position".into(), position.into());
                self.emit(Relation::References, lookup, name, span_of(node), details);
            }

            "type_identifier" => {
                let name = self.text(node);
                let lookup = self.resolve_name(&name, node.start_byte());
                let mut details = serde_json::Map::new();
                details.insert("reference_name".into(), name.as_str().into());
                details.insert("position".into(), "type".into());
                self.emit(Relation::References, lookup, name, span_of(node), details);
            }

            "nested_type_identifier" => {
                match (
                    node.child_by_field_name("module"),
                    node.child_by_field_name("name"),
                ) {
                    (Some(module), Some(name)) if module.kind() == "identifier" => {
                        let receiver = self.text(module);
                        let member = self.text(name);
                        let qualified = format!("{receiver}.{member}");
                        let lookup =
                            self.resolve_qualified(&receiver, &member, module.start_byte());
                        let mut details = serde_json::Map::new();
                        details.insert("reference_name".into(), qualified.as_str().into());
                        details.insert("position".into(), "type".into());
                        self.emit(
                            Relation::References,
                            lookup,
                            qualified,
                            span_of(node),
                            details,
                        );
                    }
                    _ => {}
                }
            }

            _ => self.visit_children_skipping(node, &[]),
        }
    }
}

/// Extract every modelled reference from one module.
///
/// `extraction` must be the structural extraction of the same source; `exports` must cover
/// every indexed module, because a barrel chain can reach any of them.
pub fn extract_references(
    project_id: &str,
    rel_path: &str,
    language: Language,
    source: &str,
    extraction: &ModuleExtraction,
    exports: &ExportIndex,
    indexed: &BTreeSet<String>,
) -> Result<ReferenceExtraction> {
    let mut parser = language.parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::error::IndexError::Parser(format!("parse failed: {rel_path}")))?;
    let root = tree.root_node();

    let mut symbol_at: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, symbol) in extraction.symbols.iter().enumerate() {
        symbol_at.entry(symbol.span.start_byte).or_insert(index);
    }

    let mut class_members: BTreeMap<(String, String), (String, bool)> = BTreeMap::new();
    for symbol in &extraction.symbols {
        if symbol.kind != EntityKind::Method {
            continue;
        }
        let Some(class_entity_id) = symbol.owner_class.clone() else {
            continue;
        };
        let meta: Option<serde_json::Value> = symbol
            .meta
            .as_deref()
            .and_then(|meta| serde_json::from_str(meta).ok());
        // `this.x()` where `x` is a getter invokes the accessor and then calls **its result**.
        // Naming the accessor as the callee would be the wrong target, so accessors are not
        // candidates for the `this.m()` rule at all.
        let is_accessor = meta
            .as_ref()
            .and_then(|meta| meta.get("accessor"))
            .is_some_and(|accessor| !accessor.is_null());
        if is_accessor {
            continue;
        }
        let is_static = meta
            .as_ref()
            .and_then(|meta| meta.get("static").and_then(|value| value.as_bool()))
            .unwrap_or(false);
        class_members
            .entry((class_entity_id, symbol.name.clone()))
            .or_insert((symbol.entity_id.clone(), is_static));
    }

    let table = BindingTable::build(extraction, root, source.as_bytes());

    let mut walker = Walker {
        source: source.as_bytes(),
        project_id,
        rel_path,
        extraction,
        exports,
        indexed,
        table,
        module_entity_id: ids::module_id(project_id, rel_path),
        symbol_at,
        class_members,
        owners: Vec::new(),
        out: ReferenceExtraction {
            rel_path: rel_path.to_string(),
            ..Default::default()
        },
    };

    for child in named_children(root) {
        walker.visit(child);
    }

    Ok(walker.out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_module;

    const PID: &str = "00000000000000000000000000000001";

    struct Corpus {
        extractions: Vec<ModuleExtraction>,
        references: Vec<ReferenceExtraction>,
    }

    impl Corpus {
        fn module(&self, rel_path: &str) -> &ReferenceExtraction {
            self.references
                .iter()
                .find(|reference| reference.rel_path == rel_path)
                .expect("module was not extracted")
        }

        fn entity(&self, rel_path: &str, scope_path: &str, name: &str) -> String {
            let extraction = self
                .extractions
                .iter()
                .find(|extraction| extraction.rel_path == rel_path)
                .expect("module was not extracted");
            extraction
                .symbols
                .iter()
                .find(|symbol| symbol.scope_path == scope_path && symbol.name == name)
                .unwrap_or_else(|| panic!("{rel_path}#{scope_path}.{name} not found"))
                .entity_id
                .clone()
        }

        fn edges(&self, rel_path: &str, relation: Relation) -> Vec<&ReferenceSite> {
            self.module(rel_path)
                .sites
                .iter()
                .filter(|site| site.relation == relation)
                .collect()
        }

        fn resolved(&self, rel_path: &str, relation: Relation) -> Vec<(String, String)> {
            self.edges(rel_path, relation)
                .into_iter()
                .filter_map(|site| match &site.target {
                    RefTarget::Resolved(target) => {
                        Some((site.source_entity_id.clone(), target.clone()))
                    }
                    RefTarget::Unresolved { .. } => None,
                })
                .collect()
        }

        fn unresolved(
            &self,
            rel_path: &str,
            relation: Relation,
        ) -> Vec<(String, UnresolvedReason)> {
            self.edges(rel_path, relation)
                .into_iter()
                .filter_map(|site| match &site.target {
                    RefTarget::Unresolved { name, reason } => Some((name.clone(), *reason)),
                    RefTarget::Resolved(_) => None,
                })
                .collect()
        }
    }

    fn corpus(modules: &[(&str, &str)]) -> Corpus {
        let indexed: BTreeSet<String> = modules.iter().map(|(path, _)| path.to_string()).collect();
        let extractions: Vec<ModuleExtraction> = modules
            .iter()
            .map(|(path, source)| {
                let language = if path.ends_with(".tsx") {
                    Language::Tsx
                } else if path.ends_with(".ts") {
                    Language::TypeScript
                } else {
                    Language::JavaScript
                };
                extract_module(PID, path, language, source).unwrap()
            })
            .collect();
        let exports = ExportIndex::build(&extractions, &indexed);
        let references: Vec<ReferenceExtraction> = modules
            .iter()
            .zip(extractions.iter())
            .map(|((path, source), extraction)| {
                let language = if path.ends_with(".tsx") {
                    Language::Tsx
                } else if path.ends_with(".ts") {
                    Language::TypeScript
                } else {
                    Language::JavaScript
                };
                extract_references(PID, path, language, source, extraction, &exports, &indexed)
                    .unwrap()
            })
            .collect();
        Corpus {
            extractions,
            references,
        }
    }

    fn single(source: &str) -> Corpus {
        corpus(&[("src/a.ts", source)])
    }

    #[test]
    fn a_same_module_call_resolves() {
        let corpus = single("function target() {}\nfunction caller() { target(); }\n");
        assert_eq!(
            corpus.resolved("src/a.ts", Relation::Calls),
            vec![(
                corpus.entity("src/a.ts", "", "caller"),
                corpus.entity("src/a.ts", "", "target")
            )]
        );
    }

    #[test]
    fn a_parameter_shadowing_a_module_function_blocks_the_call() {
        let corpus = single("function foo() {}\nfunction bar(foo) { foo(); }\n");
        assert!(corpus.resolved("src/a.ts", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![("foo".to_string(), UnresolvedReason::LocalBindingNotASymbol)]
        );
    }

    #[test]
    fn an_aliased_named_import_call_resolves_through_the_barrel() {
        let corpus = corpus(&[
            ("src/math.ts", "export function add() {}\n"),
            ("src/index.ts", "export { add as plus } from './math';\n"),
            (
                "src/app.ts",
                "import { plus } from './index';\nexport function run() { plus(); }\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("src/app.ts", Relation::Calls),
            vec![(
                corpus.entity("src/app.ts", "", "run"),
                corpus.entity("src/math.ts", "", "add")
            )]
        );
    }

    #[test]
    fn a_default_import_call_resolves() {
        let corpus = corpus(&[
            (
                "src/math.ts",
                "export function add() {}\nexport default add;\n",
            ),
            (
                "src/app.ts",
                "import add from './math';\nexport function run() { add(); }\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("src/app.ts", Relation::Calls),
            vec![(
                corpus.entity("src/app.ts", "", "run"),
                corpus.entity("src/math.ts", "", "add")
            )]
        );
    }

    #[test]
    fn a_namespace_member_call_resolves() {
        let corpus = corpus(&[
            ("src/math.ts", "export function add() {}\n"),
            (
                "src/app.ts",
                "import * as math from './math';\nexport function run() { math.add(); }\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("src/app.ts", Relation::Calls),
            vec![(
                corpus.entity("src/app.ts", "", "run"),
                corpus.entity("src/math.ts", "", "add")
            )]
        );
    }

    #[test]
    fn a_require_namespace_member_call_resolves() {
        let corpus = corpus(&[
            ("src/math.ts", "export function add() {}\n"),
            (
                "src/app.js",
                "const math = require('./math');\nfunction run() { math.add(); }\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("src/app.js", Relation::Calls),
            vec![(
                corpus.entity("src/app.js", "", "run"),
                corpus.entity("src/math.ts", "", "add")
            )]
        );
        // The `require(...)` call itself is a module-system operator, not a modelled call.
        assert_eq!(corpus.module("src/app.js").unmodelled_by_form["require"], 1);
    }

    #[test]
    fn a_namespace_member_that_is_not_exported_is_unresolved() {
        let corpus = corpus(&[
            ("src/math.ts", "export function add() {}\n"),
            (
                "src/app.ts",
                "import * as math from './math';\nexport function run() { math.nope(); }\n",
            ),
        ]);
        assert_eq!(
            corpus.unresolved("src/app.ts", Relation::Calls),
            vec![(
                "math.nope".to_string(),
                UnresolvedReason::ImportNameNotExported
            )]
        );
    }

    #[test]
    fn new_from_an_import_resolves_to_the_class() {
        let corpus = corpus(&[
            ("src/shapes.ts", "export class Circle {}\n"),
            (
                "src/app.ts",
                "import { Circle } from './shapes';\nexport function run() { return new Circle(); }\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("src/app.ts", Relation::Calls),
            vec![(
                corpus.entity("src/app.ts", "", "run"),
                corpus.entity("src/shapes.ts", "", "Circle")
            )]
        );
    }

    #[test]
    fn this_member_resolves_only_when_the_class_declares_it() {
        let corpus = single("class C { m() { this.n(); } n() {} }\n");
        assert_eq!(
            corpus.resolved("src/a.ts", Relation::Calls),
            vec![(
                corpus.entity("src/a.ts", "C", "m"),
                corpus.entity("src/a.ts", "C", "n")
            )]
        );
    }

    #[test]
    fn this_member_in_a_nested_function_is_unresolved() {
        let corpus = single("class C { m() { function inner() { this.n(); } } n() {} }\n");
        assert!(corpus.resolved("src/a.ts", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "this.n".to_string(),
                UnresolvedReason::ThisReboundByNestedFunction
            )]
        );
    }

    #[test]
    fn this_member_that_is_a_getter_is_unresolved() {
        let corpus = single("class C { get n() { return 1; } m() { this.n(); } }\n");
        assert!(corpus.resolved("src/a.ts", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "this.n".to_string(),
                UnresolvedReason::MethodNotDeclaredOnClass
            )]
        );
    }

    #[test]
    fn this_member_with_a_staticness_mismatch_is_unresolved() {
        let corpus = single("class C { static n() {} m() { this.n(); } }\n");
        assert!(corpus.resolved("src/a.ts", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "this.n".to_string(),
                UnresolvedReason::MethodNotDeclaredOnClass
            )]
        );
    }

    #[test]
    fn this_member_declared_only_on_a_base_class_is_unresolved() {
        let corpus = single("class Base { n() {} }\nclass C extends Base { m() { this.n(); } }\n");
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "this.n".to_string(),
                UnresolvedReason::MethodNotDeclaredOnClass
            )]
        );
    }

    #[test]
    fn a_typed_parameter_receiver_is_unresolved() {
        let corpus = single("export function area(shape: Shape) { return shape.area(); }\n");
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "shape.area".to_string(),
                UnresolvedReason::ReceiverNotResolvable
            )]
        );
    }

    #[test]
    fn a_global_member_call_is_unresolved() {
        let corpus = single("export function log() { console.log(1); }\n");
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "console.log".to_string(),
                UnresolvedReason::ReceiverNotResolvable
            )]
        );
    }

    #[test]
    fn a_call_through_an_unresolved_import_is_unresolved() {
        let corpus =
            single("import { parse } from 'external';\nexport function run() { parse(); }\n");
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![(
                "parse".to_string(),
                UnresolvedReason::ImportModuleUnresolved
            )]
        );
    }

    #[test]
    fn unmodelled_callee_forms_produce_no_edge_and_are_counted() {
        let corpus = single(
            "export function run(a: any, b: any, f: any, tag: any) {\n\
               a[b]();\n\
               f()();\n\
               (function () {})();\n\
               tag`x`;\n\
             }\n",
        );
        let module = corpus.module("src/a.ts");
        assert_eq!(module.unmodelled_by_form["computed-member"], 1);
        assert_eq!(module.unmodelled_by_form["call-result"], 1);
        assert_eq!(module.unmodelled_by_form["iife"], 1);
        assert_eq!(module.unmodelled_by_form["tagged-template"], 1);
        // `f()()` still contains a real inner call to `f`, which is an opaque parameter.
        assert_eq!(
            corpus.unresolved("src/a.ts", Relation::Calls),
            vec![("f".to_string(), UnresolvedReason::LocalBindingNotASymbol)]
        );
    }

    #[test]
    fn heritage_resolves_locally_and_across_modules() {
        let corpus = corpus(&[
            (
                "src/base.ts",
                "export class Base {}\nexport interface I {}\n",
            ),
            (
                "src/app.ts",
                "import { Base, I } from './base';\nexport class A extends Base implements I {}\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("src/app.ts", Relation::Extends),
            vec![(
                corpus.entity("src/app.ts", "", "A"),
                corpus.entity("src/base.ts", "", "Base")
            )]
        );
        assert_eq!(
            corpus.resolved("src/app.ts", Relation::Implements),
            vec![(
                corpus.entity("src/app.ts", "", "A"),
                corpus.entity("src/base.ts", "", "I")
            )]
        );
    }

    #[test]
    fn generic_heritage_is_stripped_to_the_head_identifier() {
        let corpus = single(
            "export class Base<T> {}\nexport interface J {}\nexport interface I extends J {}\nexport class A extends Base<number> implements I {}\n",
        );
        assert_eq!(
            corpus.resolved("src/a.ts", Relation::Extends),
            vec![
                (
                    corpus.entity("src/a.ts", "", "I"),
                    corpus.entity("src/a.ts", "", "J")
                ),
                (
                    corpus.entity("src/a.ts", "", "A"),
                    corpus.entity("src/a.ts", "", "Base")
                ),
            ]
        );
    }

    #[test]
    fn a_computed_superclass_produces_no_extends_edge() {
        let corpus =
            single("export class Base {}\nfunction mixin(b: any) { return b; }\nexport class Mixed extends mixin(Base) {}\n");
        assert!(corpus.edges("src/a.ts", Relation::Extends).is_empty());
        assert_eq!(
            corpus.module("src/a.ts").unmodelled_by_form["heritage-call"],
            1
        );
        // The mixin call itself is a genuine call site and is still recorded.
        assert_eq!(
            corpus.resolved("src/a.ts", Relation::Calls),
            vec![(
                corpus.entity("src/a.ts", "", "Mixed"),
                corpus.entity("src/a.ts", "", "mixin")
            )]
        );
    }

    #[test]
    fn references_are_resolved_only() {
        let corpus = single("function target() {}\nexport const alias = target;\nexport const unknown = somethingGlobal;\n");
        let references = corpus.edges("src/a.ts", Relation::References);
        assert_eq!(references.len(), 1);
        assert_eq!(
            references[0].target,
            RefTarget::Resolved(corpus.entity("src/a.ts", "", "target"))
        );
    }

    #[test]
    fn a_type_position_reference_resolves() {
        let corpus =
            single("export interface Shape {}\nexport function area(s: Shape) { return s; }\n");
        let references = corpus.edges("src/a.ts", Relation::References);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].details["position"], "type");
        assert_eq!(
            references[0].target,
            RefTarget::Resolved(corpus.entity("src/a.ts", "", "Shape"))
        );
    }

    #[test]
    fn a_declaration_name_is_not_a_reference_to_itself() {
        let corpus = single("export function add(add: number) { return add; }\n");
        assert!(corpus.edges("src/a.ts", Relation::References).is_empty());
    }

    #[test]
    fn a_local_shadowing_an_exported_name_elsewhere_does_not_resolve() {
        let corpus = corpus(&[
            ("src/math.ts", "export function add() {}\n"),
            (
                "src/app.ts",
                "export function run() { const add = 1; return add; }\n",
            ),
        ]);
        assert!(corpus.edges("src/app.ts", Relation::References).is_empty());
        assert!(corpus.edges("src/app.ts", Relation::Calls).is_empty());
    }

    #[test]
    fn a_site_inside_an_unnamed_function_belongs_to_the_module() {
        let corpus =
            single("function target() {}\nexport function run() { [1].map(() => target()); }\n");
        let calls = corpus.resolved("src/a.ts", Relation::Calls);
        let module_entity = ids::module_id(PID, "src/a.ts");
        assert_eq!(
            calls,
            vec![(module_entity, corpus.entity("src/a.ts", "", "target"))]
        );
    }

    #[test]
    fn extraction_is_deterministic() {
        let source = "import { a } from './m';\nexport function f() { a(); this.g(); }\n";
        assert_eq!(
            single(source).module("src/a.ts"),
            single(source).module("src/a.ts")
        );
    }

    #[test]
    fn every_unresolved_reason_has_a_distinct_tag() {
        let mut tags: Vec<&str> = UnresolvedReason::ALL.iter().map(|r| r.as_str()).collect();
        let count = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), count);
    }
}
