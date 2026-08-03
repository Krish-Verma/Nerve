//! The `py-reference` extractor: `CALLS`, `REFERENCES`, `EXTENDS`.
//!
//! Its own extractor id and its own version, for the reason Slice 5d-i established and 9a
//! restated: an observation names what produced it, so a Python edge must never claim
//! `ts-js-reference` read it. Nothing here is a branch inside [`crate::refs`], and a `.py` file
//! never reaches that module.
//!
//! # No `IMPLEMENTS`, in any slice
//!
//! Python has no `implements` keyword. `class C(SomeABC)` states inheritance and nothing more;
//! ABCs and protocols are inheritance or duck typing, and an `IMPLEMENTS` edge derived from a
//! base list would be a guess dressed as a syntax fact. `EXTENDS` is what the syntax states.
//!
//! # A receiver Nerve cannot name is unresolved, and that is the headline number
//!
//! `obj.method()` resolves only when the receiver is unambiguously **a module or a class Nerve
//! indexed**:
//!
//! - `pkg.util.scale()` after `import pkg.util` — the longest prefix of the callee that names an
//!   indexed module decides the target, so a basename is never matched anywhere in the tree;
//! - `Engine.start(...)` where `Engine` is bound to a class this repository declares, and that
//!   class **declares** `start` itself.
//!
//! Everything else is [`PyRefTarget::Unresolved`] with a reason. In particular **`self.m()` is
//! unresolved**, and that is the single largest contributor to Python's unresolved rate. The
//! reason is not squeamishness: `self` is a parameter name, not a language keyword — `def m(this)`
//! means exactly the same thing — so reading it as the enclosing class would be type inference by
//! convention, which CLAUDE.md §3 forbids and which is wrong whenever a subclass instance is
//! passed. TypeScript's `this` is a language binding and is treated differently for that reason.
//!
//! An inherited member is refused for a related reason: walking the MRO stops being a syntax fact
//! as soon as one base is unresolved, so it is refused uniformly rather than sometimes.
//!
//! # Three asymmetries carried over from `ts-js-reference`
//!
//! - An unresolved **call** is emitted. "This code invokes something Nerve cannot name" is worth
//!   recording and counting.
//! - An unresolved **reference** is not. A bare identifier that does not resolve is usually a
//!   local or a builtin, and one edge per identifier would drown the graph.
//! - Callee and base-class forms Nerve does not model produce **no edge at all** and are counted.
//!
//! # Decorators belong to `py-structural`
//!
//! 9a records a decorator as structural metadata on the decorated symbol. This extractor does not
//! also record it as a call: one piece of syntax with two differently-shaped claims about it would
//! make "what does Nerve say about `@app.route`?" have two answers. The decorator's *arguments*
//! are still walked, because a call written inside them is a call like any other.
//!
//! # What a wildcard import does not do
//!
//! `from x import *` binds a set of names that lives in the imported module's runtime namespace.
//! 9a records that set as unknowable, and this extractor does not contradict it: a name that only
//! a wildcard could have bound is `name-not-in-scope`. The measured cost is stated in
//! `fixtures/py-resolution/expected.json` as a known false negative rather than hidden.
//!
//! Observation `details` carry identifier and dotted-name **names** only. Names already live in
//! `entity.name`; arbitrary expression source does not, and SECURITY.md's "no source text at rest"
//! rule stays true by construction.

use std::collections::BTreeMap;

use tree_sitter::Node;

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::{EntityKind, EvidenceSourceType, Relation};

use crate::error::Result;
use crate::lang::Language;
use crate::pybind::{symbol_index_for, symbol_offsets, PyBinding, PyBindingTable};
use crate::pystruct::PyModuleExtraction;
use crate::pysurface::PySurfaceIndex;

/// Identifier of the Python reference extractor.
pub const EXTRACTOR_ID: &str = "py-reference";

/// Version of the extractor. Bump on any change to what it emits.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The evidence source types this extractor is permitted to emit (ADR-0003).
///
/// `AST_RESOLVED` for edges produced by binding, specifier and cross-module name resolution;
/// `AST_DIRECT` for edges whose target is an `Unresolved` entity, where the only thing the
/// evidence states is what the tree literally wrote.
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 2] = [
    EvidenceSourceType::AstDirect,
    EvidenceSourceType::AstResolved,
];

/// Why a Python reference could not be resolved. Closed vocabulary; recorded verbatim.
///
/// Every tag is one `ts-js-reference` already uses, and
/// [`tests::every_reason_tag_is_one_the_interface_already_glosses`] pins that. The vocabulary is
/// mirrored in `apps/nerve-web/src/vocab.ts`; a Python-only tag would arrive there unglossed, and
/// none of these distinctions needed a word the interface does not already have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PyUnresolvedReason {
    /// The receiver is not a module or a class Nerve indexed, so the member cannot be named.
    ReceiverNotResolvable,
    /// The name is bound, but to a parameter, variable, loop target or comprehension variable.
    LocalBindingNotASymbol,
    /// Nothing in the scope chain binds the name.
    NameNotInScope,
    /// The name is imported, but the specifier names no indexed module.
    ImportModuleUnresolved,
    /// The specifier names an indexed module, which provides no such name at top level.
    ImportNameNotProvided,
    /// The receiver is an indexed class that does not itself declare the member.
    MethodNotDeclaredOnClass,
}

impl PyUnresolvedReason {
    /// Every reason, in declaration order.
    pub const ALL: [PyUnresolvedReason; 6] = [
        PyUnresolvedReason::ReceiverNotResolvable,
        PyUnresolvedReason::LocalBindingNotASymbol,
        PyUnresolvedReason::NameNotInScope,
        PyUnresolvedReason::ImportModuleUnresolved,
        PyUnresolvedReason::ImportNameNotProvided,
        PyUnresolvedReason::MethodNotDeclaredOnClass,
    ];

    /// Canonical name recorded in evidence and in ground truth.
    pub fn as_str(self) -> &'static str {
        match self {
            PyUnresolvedReason::ReceiverNotResolvable => "receiver-not-resolvable",
            PyUnresolvedReason::LocalBindingNotASymbol => "local-binding-not-a-symbol",
            PyUnresolvedReason::NameNotInScope => "name-not-in-scope",
            PyUnresolvedReason::ImportModuleUnresolved => "import-module-unresolved",
            // Python has no export keyword, so "not exported" reads oddly and "not provided" is
            // what is meant: the module binds no such name at top level. The **tag** is shared
            // with `ts-js-reference` so the interface gloss keeps covering it.
            PyUnresolvedReason::ImportNameNotProvided => "import-name-not-exported",
            PyUnresolvedReason::MethodNotDeclaredOnClass => "method-not-declared-on-class",
        }
    }
}

/// Every form tag this extractor can record in `unmodelled_by_form`, in alphabetical order.
///
/// Each is already a member of [`crate::refs::UNMODELLED_FORMS`], and
/// [`tests::every_unmodelled_form_is_one_the_interface_already_glosses`] pins that, so Python
/// adds no vocabulary member to the UI mirror.
pub const PY_UNMODELLED_FORMS: [&str; 9] = [
    "call-result",
    "complex-receiver",
    "computed-member",
    "dynamic-import",
    "heritage-call",
    "heritage-other",
    "iife",
    "other",
    "super",
];

/// What a Python reference site points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PyRefTarget {
    /// A resolved entity in this repository.
    Resolved(String),
    /// A target Nerve could not name.
    Unresolved {
        /// The name as written: an identifier, or a dotted name.
        name: String,
        /// Why resolution failed.
        reason: PyUnresolvedReason,
    },
}

/// One `CALLS` / `REFERENCES` / `EXTENDS` site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyReferenceSite {
    /// Innermost enclosing symbol entity, or the module entity.
    pub source_entity_id: String,
    /// Relation asserted.
    pub relation: Relation,
    /// Where it points.
    pub target: PyRefTarget,
    /// Source range of the referring token.
    pub span: Span,
    /// Evidence detail. Names and enumerated tags only — never expression source.
    pub details: serde_json::Value,
}

/// Everything the Python reference extractor found in one module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyReferenceExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Reference sites, in source order.
    pub sites: Vec<PyReferenceSite>,
    /// Call sites and base-class expressions whose form Nerve does not model.
    pub unmodelled_call_sites: usize,
    /// Breakdown of the above by form tag.
    pub unmodelled_by_form: BTreeMap<String, usize>,
}

/// Outcome of resolving one name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    Entity(String),
    Failed(PyUnresolvedReason),
}

/// A resolution plus the evidence detail explaining how it was reached.
#[derive(Debug, Clone)]
struct Lookup {
    outcome: Resolution,
    binding: &'static str,
    resolved_module: Option<String>,
}

impl Lookup {
    fn entity(entity_id: String, binding: &'static str) -> Lookup {
        Lookup {
            outcome: Resolution::Entity(entity_id),
            binding,
            resolved_module: None,
        }
    }

    fn failed(reason: PyUnresolvedReason, binding: &'static str) -> Lookup {
        Lookup {
            outcome: Resolution::Failed(reason),
            binding,
            resolved_module: None,
        }
    }
}

/// The declaration a site sits inside.
#[derive(Debug, Clone)]
enum Owner {
    /// A named declaration with an entity.
    Entity(String),
    /// A `lambda`, which Nerve does not name; sites inside it belong to the module.
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
    extraction: &'a PyModuleExtraction,
    surfaces: &'a PySurfaceIndex,
    table: PyBindingTable,
    module_entity_id: String,
    symbol_at: BTreeMap<usize, usize>,
    owners: Vec<Owner>,
    out: PyReferenceExtraction,
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
        match symbol_index_for(&self.symbol_at, node) {
            Some(index) => Owner::Entity(self.extraction.symbols[index].entity_id.clone()),
            None => Owner::Anonymous,
        }
    }

    /// The dotted name an expression writes, when it is one.
    ///
    /// `None` for anything with a call, a subscript or a parenthesis in it — those need a value,
    /// not a name, and naming their target would be a guess.
    fn dotted_chain(&self, node: Node) -> Option<Vec<String>> {
        match node.kind() {
            "identifier" => Some(vec![self.text(node)]),
            "attribute" => {
                let object = node.child_by_field_name("object")?;
                let attribute = node.child_by_field_name("attribute")?;
                if attribute.kind() != "identifier" {
                    return None;
                }
                let mut chain = self.dotted_chain(object)?;
                chain.push(self.text(attribute));
                Some(chain)
            }
            _ => None,
        }
    }

    fn is_super_call(&self, node: Node) -> bool {
        node.kind() == "call"
            && node
                .child_by_field_name("function")
                .is_some_and(|function| {
                    function.kind() == "identifier" && self.text(function) == "super"
                })
    }

    // ---- resolution ------------------------------------------------------------------------

    fn resolve_module_specifier(&self, specifier: &str) -> Option<String> {
        self.surfaces.resolve_specifier(self.rel_path, specifier)
    }

    fn resolve_imported_name(&self, specifier: &str, imported: &str) -> Lookup {
        match self.resolve_module_specifier(specifier) {
            None => Lookup::failed(
                PyUnresolvedReason::ImportModuleUnresolved,
                "import-from-name",
            ),
            Some(module) => match self.surfaces.provides(&module, imported) {
                Some(entity_id) => Lookup {
                    outcome: Resolution::Entity(entity_id),
                    binding: "import-from-name",
                    resolved_module: Some(module),
                },
                None => Lookup {
                    outcome: Resolution::Failed(PyUnresolvedReason::ImportNameNotProvided),
                    binding: "import-from-name",
                    resolved_module: Some(module),
                },
            },
        }
    }

    fn resolve_class_member(
        &self,
        class_entity_id: &str,
        member: &str,
        binding: &'static str,
    ) -> Lookup {
        match self.surfaces.method_of(class_entity_id, member) {
            Some(entity_id) => Lookup::entity(entity_id, binding),
            // Inherited from a base, assigned as an attribute, or produced by a decorator.
            None => Lookup::failed(PyUnresolvedReason::MethodNotDeclaredOnClass, binding),
        }
    }

    /// Resolve a bare name as written at `byte`.
    fn resolve_name(&self, name: &str, byte: usize) -> Lookup {
        let scope = self.table.scope_at(byte);
        match self.table.lookup(scope, name) {
            None => Lookup::failed(PyUnresolvedReason::NameNotInScope, "none"),
            Some(PyBinding::Opaque) => {
                Lookup::failed(PyUnresolvedReason::LocalBindingNotASymbol, "opaque")
            }
            Some(PyBinding::LocalSymbol { entity_id, .. }) => {
                Lookup::entity(entity_id.clone(), "local")
            }
            Some(PyBinding::ImportedName {
                specifier,
                imported,
            }) => self.resolve_imported_name(specifier, imported),
            Some(PyBinding::ImportedModule { specifier }) => {
                match self.resolve_module_specifier(specifier) {
                    Some(module) => Lookup {
                        outcome: Resolution::Entity(ids::module_id(self.project_id, &module)),
                        binding: "import-module",
                        resolved_module: Some(module),
                    },
                    None => {
                        Lookup::failed(PyUnresolvedReason::ImportModuleUnresolved, "import-module")
                    }
                }
            }
        }
    }

    /// Resolve `a.b(.c)*` where `a` is bound to a module.
    ///
    /// The **longest** prefix of the chain that names an indexed module decides the target. That
    /// is deterministic, and it is what the language does: `pkg.util.scale` is an attribute of
    /// the module `pkg.util` exactly when that module exists. Anything left over after the module
    /// and one member is an attribute of a value, which needs a type Nerve does not have.
    fn resolve_through_module(&self, specifier: &str, chain: &[String]) -> Lookup {
        for extra in (0..chain.len() - 1).rev() {
            let mut candidate = specifier.to_string();
            for segment in &chain[1..1 + extra] {
                candidate.push('.');
                candidate.push_str(segment);
            }
            let Some(module) = self.resolve_module_specifier(&candidate) else {
                continue;
            };
            let rest = &chain[1 + extra..];
            if rest.len() != 1 {
                return Lookup {
                    outcome: Resolution::Failed(PyUnresolvedReason::ReceiverNotResolvable),
                    binding: "import-module",
                    resolved_module: Some(module),
                };
            }
            return match self.surfaces.provides(&module, &rest[0]) {
                Some(entity_id) => Lookup {
                    outcome: Resolution::Entity(entity_id),
                    binding: "import-module",
                    resolved_module: Some(module),
                },
                None => Lookup {
                    outcome: Resolution::Failed(PyUnresolvedReason::ImportNameNotProvided),
                    binding: "import-module",
                    resolved_module: Some(module),
                },
            };
        }
        Lookup::failed(PyUnresolvedReason::ImportModuleUnresolved, "import-module")
    }

    /// Resolve a dotted name, which only succeeds when the head is a module or an indexed class.
    fn resolve_qualified(&self, chain: &[String], byte: usize) -> Lookup {
        let scope = self.table.scope_at(byte);
        match self.table.lookup(scope, &chain[0]) {
            Some(PyBinding::ImportedModule { specifier }) => {
                self.resolve_through_module(specifier, chain)
            }
            Some(PyBinding::LocalSymbol {
                entity_id, kind, ..
            }) if *kind == EntityKind::Class && chain.len() == 2 => {
                self.resolve_class_member(entity_id, &chain[1], "local-class")
            }
            Some(PyBinding::ImportedName {
                specifier,
                imported,
            }) if chain.len() == 2 => {
                let head = self.resolve_imported_name(specifier, imported);
                match &head.outcome {
                    Resolution::Entity(entity_id) if self.surfaces.is_class(entity_id) => {
                        let mut member =
                            self.resolve_class_member(entity_id, &chain[1], "import-class");
                        member.resolved_module = head.resolved_module.clone();
                        member
                    }
                    // The head is a function or a variable: naming its attribute needs a value.
                    Resolution::Entity(_) => {
                        Lookup::failed(PyUnresolvedReason::ReceiverNotResolvable, "import-class")
                    }
                    // The head itself did not resolve; that is the finding worth reporting.
                    Resolution::Failed(_) => head,
                }
            }
            // A parameter, a local, `self`, a builtin, or a chain too long to name. The type of
            // `other` in `other.start()` is exactly the information a binding table lacks.
            _ => Lookup::failed(PyUnresolvedReason::ReceiverNotResolvable, "none"),
        }
    }

    // ---- emission --------------------------------------------------------------------------

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
                PyRefTarget::Resolved(entity_id)
            }
            Resolution::Failed(reason) => {
                // An unresolved bare reference states nothing worth an edge.
                if relation == Relation::References {
                    return;
                }
                details.insert("reason".into(), reason.as_str().into());
                PyRefTarget::Unresolved {
                    name: name.clone(),
                    reason,
                }
            }
        };

        let source_entity_id = self.source_entity_id();
        self.out.sites.push(PyReferenceSite {
            source_entity_id,
            relation,
            target,
            span,
            details: serde_json::Value::Object(details),
        });
    }

    // ---- calls -----------------------------------------------------------------------------

    fn visit_call(&mut self, node: Node) {
        let Some(callee) = node.child_by_field_name("function") else {
            self.unmodelled("other");
            self.visit_children(node);
            return;
        };

        let mut consumed_callee = true;
        match callee.kind() {
            "identifier" => {
                let name = self.text(callee);
                if name == "__import__" {
                    // `py-structural` already records this as an import finding; a second,
                    // differently-shaped claim about one statement would count it twice.
                    self.unmodelled("dynamic-import");
                } else if name == "super" {
                    self.unmodelled("super");
                } else {
                    let lookup = self.resolve_name(&name, callee.start_byte());
                    let details = call_details("call", "identifier", &name);
                    self.emit(Relation::Calls, lookup, name, span_of(callee), details);
                }
            }
            "attribute" => consumed_callee = self.visit_member_callee(callee),
            "subscript" => {
                self.unmodelled("computed-member");
                consumed_callee = false;
            }
            "call" => {
                self.unmodelled("call-result");
                consumed_callee = false;
            }
            "parenthesized_expression" | "lambda" => {
                self.unmodelled("iife");
                consumed_callee = false;
            }
            _ => {
                self.unmodelled("other");
                consumed_callee = false;
            }
        }

        // The callee is CALLS territory and never also a REFERENCES site. An unmodelled callee is
        // different: it can contain calls of its own, so its subtree is still walked.
        let skip = if consumed_callee {
            Some(callee.id())
        } else {
            None
        };
        for child in named_children(node) {
            if Some(child.id()) == skip {
                continue;
            }
            self.visit(child);
        }
    }

    /// Handle an `attribute` in callee position. Returns whether the callee was consumed.
    fn visit_member_callee(&mut self, callee: Node) -> bool {
        let Some(object) = callee.child_by_field_name("object") else {
            self.unmodelled("other");
            return false;
        };
        if self.is_super_call(object) {
            self.unmodelled("super");
            return true;
        }
        match self.dotted_chain(callee) {
            Some(chain) if chain.len() >= 2 => {
                let name = chain.join(".");
                if name == "importlib.import_module" {
                    self.unmodelled("dynamic-import");
                    return true;
                }
                let lookup = self.resolve_qualified(&chain, callee.start_byte());
                let callee_form = match lookup.binding {
                    "import-module" => "module-member",
                    "local-class" | "import-class" => "class-member",
                    _ => "member",
                };
                let details = call_details("call", callee_form, &name);
                self.emit(Relation::Calls, lookup, name, span_of(callee), details);
                true
            }
            // `f().g()`, `xs[0].g()`, `(a or b).g()`: naming the target would need a value.
            _ => {
                self.unmodelled("complex-receiver");
                false
            }
        }
    }

    // ---- base classes ----------------------------------------------------------------------

    fn visit_superclasses(&mut self, list: Node) {
        for child in named_children(list) {
            match child.kind() {
                "identifier" => {
                    let name = self.text(child);
                    let lookup = self.resolve_name(&name, child.start_byte());
                    self.emit_heritage(lookup, name, child, "identifier");
                }
                "attribute" => match self.dotted_chain(child) {
                    Some(chain) if chain.len() >= 2 => {
                        let name = chain.join(".");
                        let lookup = self.resolve_qualified(&chain, child.start_byte());
                        self.emit_heritage(lookup, name, child, "qualified");
                    }
                    _ => {
                        self.unmodelled("heritage-other");
                        self.visit(child);
                    }
                },
                // `class C(make_base())`: the base is computed. No EXTENDS edge is invented; the
                // call itself is a real call site and is still visited.
                "call" => {
                    self.unmodelled("heritage-call");
                    self.visit(child);
                }
                // `class C(metaclass=Meta)`: a keyword argument is not a base at all.
                "keyword_argument" => self.visit(child),
                "comment" => {}
                // `class C(Engine[int])`: a subscripted base is a call to `__class_getitem__`,
                // not an erased type argument to be stripped down to its head the way
                // TypeScript's `extends Base<number>` is.
                _ => {
                    self.unmodelled("heritage-other");
                    self.visit(child);
                }
            }
        }
    }

    fn emit_heritage(&mut self, lookup: Lookup, name: String, node: Node, form: &str) {
        let mut details = serde_json::Map::new();
        details.insert("heritage_form".into(), form.into());
        details.insert("heritage_name".into(), name.as_str().into());
        self.emit(Relation::Extends, lookup, name, span_of(node), details);
    }

    // ---- traversal -------------------------------------------------------------------------

    fn visit_children(&mut self, node: Node) {
        for child in named_children(node) {
            self.visit(child);
        }
    }

    fn visit_skipping(&mut self, node: Node, skip: Option<usize>) {
        for child in named_children(node) {
            if Some(child.id()) == skip {
                continue;
            }
            self.visit(child);
        }
    }

    fn visit_function(&mut self, node: Node, owner: Owner) {
        self.owners.push(owner);
        let name_id = node.child_by_field_name("name").map(|child| child.id());
        self.visit_skipping(node, name_id);
        self.owners.pop();
    }

    fn visit_class(&mut self, node: Node) {
        let owner = self.owner_for(node);
        self.owners.push(owner);
        if let Some(superclasses) = node.child_by_field_name("superclasses") {
            self.visit_superclasses(superclasses);
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_children(body);
        }
        self.owners.pop();
    }

    fn visit_decorated(&mut self, node: Node) {
        // `py-structural` gives a decorated definition a span that starts at the `@`, so the
        // decorator list belongs to the decorated symbol's occurrence. Sites inside it are
        // attributed the same way, rather than to the module the decorator is evaluated in.
        let owner = self.owner_for(node);
        self.owners.push(owner);
        for child in named_children(node) {
            if child.kind() != "decorator" {
                self.visit(child);
                continue;
            }
            // The decorator itself is `py-structural`'s claim, recorded on the decorated
            // symbol's `meta`. Its arguments are still walked.
            let Some(expression) = named_children(child).into_iter().next() else {
                continue;
            };
            if expression.kind() == "call" {
                if let Some(arguments) = expression.child_by_field_name("arguments") {
                    self.visit(arguments);
                }
            }
        }
        self.owners.pop();
    }

    fn visit_parameters(&mut self, node: Node) {
        for parameter in named_children(node) {
            match parameter.kind() {
                // The binding itself is not a reference to anything.
                "identifier"
                | "list_splat_pattern"
                | "dictionary_splat_pattern"
                | "positional_separator"
                | "keyword_separator" => {}
                "typed_parameter" | "typed_default_parameter" => {
                    if let Some(annotation) = parameter.child_by_field_name("type") {
                        self.visit(annotation);
                    }
                    if let Some(value) = parameter.child_by_field_name("value") {
                        self.visit(value);
                    }
                }
                "default_parameter" => {
                    if let Some(value) = parameter.child_by_field_name("value") {
                        self.visit(value);
                    }
                }
                _ => self.visit(parameter),
            }
        }
    }

    fn visit(&mut self, node: Node) {
        match node.kind() {
            // An import statement declares bindings; it references nothing.
            "import_statement" | "import_from_statement" | "future_import_statement" => {}
            // A `global` / `nonlocal` statement names a binding, not a use of one.
            "global_statement" | "nonlocal_statement" => {}

            "call" => self.visit_call(node),
            "class_definition" => self.visit_class(node),
            "decorated_definition" => self.visit_decorated(node),
            "function_definition" => {
                let owner = self.owner_for(node);
                self.visit_function(node, owner);
            }
            "lambda" => self.visit_function(node, Owner::Anonymous),
            "parameters" | "lambda_parameters" => self.visit_parameters(node),

            // Only the object of an attribute access is a name in scope. Nerve does not resolve
            // `mod.name` outside callee and base-class position: the sites that matter are those
            // two, and a third path would be a fourth answer to maintain.
            "attribute" => {
                if let Some(object) = node.child_by_field_name("object") {
                    self.visit(object);
                }
            }
            "keyword_argument" | "keyword_pattern" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.visit(value);
                }
            }
            // A `match` pattern binds names; a class pattern's own name is indistinguishable
            // from a capture without the runtime value, so none of it is a reference.
            "case_pattern" => {}

            "assignment" | "augmented_assignment" => {
                let left = node.child_by_field_name("left");
                let skip = left
                    .filter(|left| left.kind() == "identifier")
                    .map(|left| left.id());
                self.visit_skipping(node, skip);
            }
            "for_statement" | "for_in_clause" => {
                let skip = node.child_by_field_name("left").map(|left| left.id());
                self.visit_skipping(node, skip);
            }
            "as_pattern" => {
                let skip = node.child_by_field_name("alias").map(|alias| alias.id());
                self.visit_skipping(node, skip);
            }
            "named_expression" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.visit(value);
                }
            }

            "identifier" => {
                let name = self.text(node);
                let lookup = self.resolve_name(&name, node.start_byte());
                let position = match node.parent().map(|parent| parent.kind()) {
                    Some("type") => "type",
                    _ => "value",
                };
                let mut details = serde_json::Map::new();
                details.insert("reference_name".into(), name.as_str().into());
                details.insert("position".into(), position.into());
                self.emit(Relation::References, lookup, name, span_of(node), details);
            }

            _ => self.visit_children(node),
        }
    }
}

/// Extract every modelled Python reference from one module.
///
/// `extraction` must be the `py-structural` extraction of the same source; `surfaces` must cover
/// every indexed Python module, because a package `__init__` chain can reach any of them.
pub fn extract_references(
    project_id: &str,
    rel_path: &str,
    source: &str,
    extraction: &PyModuleExtraction,
    surfaces: &PySurfaceIndex,
) -> Result<PyReferenceExtraction> {
    let mut parser = Language::Python.parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::error::IndexError::Parser(format!("parse failed: {rel_path}")))?;
    let root = tree.root_node();

    let table = PyBindingTable::build(extraction, root, source.as_bytes());

    let mut walker = Walker {
        source: source.as_bytes(),
        project_id,
        rel_path,
        extraction,
        surfaces,
        table,
        module_entity_id: ids::module_id(project_id, rel_path),
        symbol_at: symbol_offsets(extraction),
        owners: Vec::new(),
        out: PyReferenceExtraction {
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
    use std::collections::BTreeSet;

    use super::*;
    use crate::pystruct::extract_module;
    use crate::pysurface::PyModuleSurface;

    const PID: &str = "00000000000000000000000000000001";

    struct Corpus {
        extractions: Vec<PyModuleExtraction>,
        references: Vec<PyReferenceExtraction>,
    }

    impl Corpus {
        fn module(&self, rel_path: &str) -> &PyReferenceExtraction {
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

        fn edges(&self, rel_path: &str, relation: Relation) -> Vec<&PyReferenceSite> {
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
                    PyRefTarget::Resolved(target) => {
                        Some((site.source_entity_id.clone(), target.clone()))
                    }
                    PyRefTarget::Unresolved { .. } => None,
                })
                .collect()
        }

        fn unresolved(
            &self,
            rel_path: &str,
            relation: Relation,
        ) -> Vec<(String, PyUnresolvedReason)> {
            self.edges(rel_path, relation)
                .into_iter()
                .filter_map(|site| match &site.target {
                    PyRefTarget::Unresolved { name, reason } => Some((name.clone(), *reason)),
                    PyRefTarget::Resolved(_) => None,
                })
                .collect()
        }
    }

    fn corpus(modules: &[(&str, &str)]) -> Corpus {
        let indexed: BTreeSet<String> = modules.iter().map(|(path, _)| path.to_string()).collect();
        let extractions: Vec<PyModuleExtraction> = modules
            .iter()
            .map(|(path, source)| extract_module(PID, path, source).unwrap())
            .collect();
        let surfaces = PySurfaceIndex::build(
            extractions
                .iter()
                .map(PyModuleSurface::from_extraction)
                .collect(),
            &indexed,
        );
        let references: Vec<PyReferenceExtraction> = modules
            .iter()
            .zip(extractions.iter())
            .map(|((path, source), extraction)| {
                extract_references(PID, path, source, extraction, &surfaces).unwrap()
            })
            .collect();
        Corpus {
            extractions,
            references,
        }
    }

    fn single(source: &str) -> Corpus {
        corpus(&[("app.py", source)])
    }

    #[test]
    fn a_same_module_call_resolves() {
        let corpus =
            single("def target():\n    return 1\n\n\ndef caller():\n    return target()\n");
        assert_eq!(
            corpus.resolved("app.py", Relation::Calls),
            vec![(
                corpus.entity("app.py", "", "caller"),
                corpus.entity("app.py", "", "target")
            )]
        );
    }

    #[test]
    fn a_parameter_shadowing_a_module_function_blocks_the_call() {
        let corpus =
            single("def target():\n    return 1\n\n\ndef caller(target):\n    return target()\n");
        assert!(corpus.resolved("app.py", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("app.py", Relation::Calls),
            vec![(
                "target".to_string(),
                PyUnresolvedReason::LocalBindingNotASymbol
            )]
        );
    }

    #[test]
    fn a_from_import_call_resolves() {
        let corpus = corpus(&[
            ("pkg/util.py", "def scale(v):\n    return v\n"),
            (
                "app.py",
                "from pkg.util import scale\n\n\ndef run(v):\n    return scale(v)\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("app.py", Relation::Calls),
            vec![(
                corpus.entity("app.py", "", "run"),
                corpus.entity("pkg/util.py", "", "scale")
            )]
        );
    }

    #[test]
    fn a_dotted_module_call_takes_the_longest_indexed_prefix() {
        let corpus = corpus(&[
            ("pkg/__init__.py", "\"\"\"pkg.\"\"\"\n"),
            ("pkg/util.py", "def scale(v):\n    return v\n"),
            (
                "app.py",
                "import pkg.util\n\n\ndef run(v):\n    return pkg.util.scale(v)\n",
            ),
        ]);
        assert_eq!(
            corpus.resolved("app.py", Relation::Calls),
            vec![(
                corpus.entity("app.py", "", "run"),
                corpus.entity("pkg/util.py", "", "scale")
            )]
        );
    }

    #[test]
    fn a_self_receiver_is_unresolved() {
        let corpus = single(
            "class C:\n    def a(self):\n        return 1\n\n    def b(self):\n        return self.a()\n",
        );
        assert!(corpus.resolved("app.py", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("app.py", Relation::Calls),
            vec![(
                "self.a".to_string(),
                PyUnresolvedReason::ReceiverNotResolvable
            )]
        );
    }

    #[test]
    fn an_explicit_class_receiver_resolves_only_for_a_declared_member() {
        let corpus = single(
            "class C:\n    def a(self):\n        return 1\n\n\nclass D(C):\n    def b(self):\n        return C.a(self)\n\n    def c(self):\n        return D.a(self)\n",
        );
        assert_eq!(
            corpus.resolved("app.py", Relation::Calls),
            vec![(
                corpus.entity("app.py", "D", "b"),
                corpus.entity("app.py", "C", "a")
            )]
        );
        assert_eq!(
            corpus.unresolved("app.py", Relation::Calls),
            vec![(
                "D.a".to_string(),
                PyUnresolvedReason::MethodNotDeclaredOnClass
            )]
        );
    }

    #[test]
    fn a_base_class_produces_extends_and_never_implements() {
        let corpus = single("class Base:\n    pass\n\n\nclass Derived(Base):\n    pass\n");
        assert_eq!(
            corpus.resolved("app.py", Relation::Extends),
            vec![(
                corpus.entity("app.py", "", "Derived"),
                corpus.entity("app.py", "", "Base")
            )]
        );
        assert!(corpus.edges("app.py", Relation::Implements).is_empty());
    }

    #[test]
    fn an_unresolved_base_is_recorded_rather_than_dropped() {
        let corpus = single("from abc import ABC\n\n\nclass A(ABC):\n    pass\n");
        assert_eq!(
            corpus.unresolved("app.py", Relation::Extends),
            vec![(
                "ABC".to_string(),
                PyUnresolvedReason::ImportModuleUnresolved
            )]
        );
    }

    #[test]
    fn a_wildcard_import_binds_nothing_a_call_may_use() {
        let corpus = corpus(&[
            ("pkg/util.py", "def scale(v):\n    return v\n"),
            (
                "app.py",
                "from pkg.util import *\n\n\ndef run(v):\n    return scale(v)\n",
            ),
        ]);
        assert!(corpus.resolved("app.py", Relation::Calls).is_empty());
        assert_eq!(
            corpus.unresolved("app.py", Relation::Calls),
            vec![("scale".to_string(), PyUnresolvedReason::NameNotInScope)]
        );
    }

    #[test]
    fn a_name_an_indexed_module_does_not_provide_is_distinguished() {
        let corpus = corpus(&[
            ("pkg/util.py", "def scale(v):\n    return v\n"),
            (
                "app.py",
                "from pkg.util import missing\n\n\ndef run():\n    return missing()\n",
            ),
        ]);
        assert_eq!(
            corpus.unresolved("app.py", Relation::Calls),
            vec![(
                "missing".to_string(),
                PyUnresolvedReason::ImportNameNotProvided
            )]
        );
    }

    #[test]
    fn unmodelled_callee_forms_produce_no_edge_and_are_counted() {
        let corpus = single(
            "class C:\n    def go(self, table, index, factory):\n        super().go()\n        table[index]()\n        factory()()\n        (lambda: 1)()\n        return 0\n",
        );
        let module = corpus.module("app.py");
        assert_eq!(module.unmodelled_by_form["super"], 1);
        assert_eq!(module.unmodelled_by_form["computed-member"], 1);
        assert_eq!(module.unmodelled_by_form["call-result"], 1);
        assert_eq!(module.unmodelled_by_form["iife"], 1);
        // `factory()()` still contains a real inner call to an opaque parameter.
        assert_eq!(
            corpus.unresolved("app.py", Relation::Calls),
            vec![(
                "factory".to_string(),
                PyUnresolvedReason::LocalBindingNotASymbol
            )]
        );
    }

    #[test]
    fn a_decorator_is_never_a_call_edge_but_its_arguments_are_walked() {
        let corpus = single(
            "def deco(**kw):\n    return kw\n\n\ndef compute():\n    return 1\n\n\n@deco(times=compute())\ndef tuned():\n    return 2\n",
        );
        let calls = corpus.resolved("app.py", Relation::Calls);
        assert_eq!(
            calls,
            vec![(
                corpus.entity("app.py", "", "tuned"),
                corpus.entity("app.py", "", "compute")
            )],
            "the decorator itself is py-structural's claim; the argument call is a call"
        );
    }

    #[test]
    fn references_are_resolved_only() {
        let corpus =
            single("def target():\n    return 1\n\n\nalias = target\nunknown = something_global\n");
        let references = corpus.edges("app.py", Relation::References);
        assert_eq!(references.len(), 1);
        assert_eq!(
            references[0].target,
            PyRefTarget::Resolved(corpus.entity("app.py", "", "target"))
        );
    }

    #[test]
    fn an_annotation_is_a_reference() {
        let corpus =
            single("class Shape:\n    pass\n\n\ndef area(s: Shape) -> Shape:\n    return s\n");
        let references = corpus.edges("app.py", Relation::References);
        assert_eq!(references.len(), 2);
        assert!(references
            .iter()
            .all(|site| site.details["position"] == "type"));
    }

    #[test]
    fn a_declaration_name_is_not_a_reference_to_itself() {
        let corpus = single("def add(add):\n    return 1\n");
        assert!(corpus.edges("app.py", Relation::References).is_empty());
    }

    #[test]
    fn extraction_is_deterministic() {
        let source = "from pkg.util import scale\n\n\nclass C:\n    def m(self):\n        return self.n(scale(1))\n";
        assert_eq!(
            single(source).module("app.py"),
            single(source).module("app.py")
        );
    }

    #[test]
    fn every_unresolved_reason_has_a_distinct_tag() {
        let mut tags: Vec<&str> = PyUnresolvedReason::ALL.iter().map(|r| r.as_str()).collect();
        let count = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), count);
    }

    /// Python adds no member to the reason vocabulary the interface mirrors.
    ///
    /// `apps/nerve-web/src/vocab.ts` glosses `refs::UnresolvedReason` and `docref::reason`. A
    /// Python-only tag would arrive in the UI unglossed, and this slice was not allowed to touch
    /// the mirror — so the reasons reuse tags that are already there, and this pins it.
    #[test]
    fn every_reason_tag_is_one_the_interface_already_glosses() {
        let glossed: BTreeSet<&str> = crate::refs::UnresolvedReason::ALL
            .iter()
            .map(|reason| reason.as_str())
            .collect();
        let ours: Vec<&str> = PyUnresolvedReason::ALL
            .iter()
            .map(|reason| reason.as_str())
            .filter(|tag| !glossed.contains(tag))
            .collect();
        assert!(
            ours.is_empty(),
            "py-reference introduced unglossed reason tag(s) {ours:?}; add them to \
             apps/nerve-web/src/vocab.ts and to ui_vocabulary.rs before shipping"
        );
    }

    /// The same guarantee for the unmodelled-form vocabulary.
    #[test]
    fn every_unmodelled_form_is_one_the_interface_already_glosses() {
        let glossed: BTreeSet<&str> = crate::refs::UNMODELLED_FORMS.iter().copied().collect();
        let ours: Vec<&&str> = PY_UNMODELLED_FORMS
            .iter()
            .filter(|form| !glossed.contains(*form))
            .collect();
        assert!(
            ours.is_empty(),
            "py-reference introduced unglossed form tag(s) {ours:?}"
        );
    }

    /// The declared list must match what the extractor can actually emit.
    ///
    /// The tags are bare literals at the site that recognises each shape, so nothing in the type
    /// system ties them to `PY_UNMODELLED_FORMS`. The list is checked against this module's own
    /// source, exactly as `ui_vocabulary.rs` checks `refs::UNMODELLED_FORMS` against its.
    #[test]
    fn the_unmodelled_form_list_matches_this_extractor_source() {
        let source = include_str!("pyrefs.rs");
        let declared: BTreeSet<&str> = PY_UNMODELLED_FORMS.iter().copied().collect();
        let mut emitted: BTreeSet<String> = BTreeSet::new();
        let mut rest = source;
        while let Some(offset) = rest.find("unmodelled(\"") {
            let after = &rest[offset + "unmodelled(\"".len()..];
            let close = after.find('"').expect("an unterminated form tag");
            emitted.insert(after[..close].to_string());
            rest = &after[close..];
        }
        assert!(!emitted.is_empty(), "the source scan found no form tags");
        let undeclared: Vec<&String> = emitted
            .iter()
            .filter(|tag| !declared.contains(tag.as_str()))
            .collect();
        assert!(
            undeclared.is_empty(),
            "PY_UNMODELLED_FORMS is missing form tag(s) emitted by the extractor: {undeclared:?}"
        );
    }
}
