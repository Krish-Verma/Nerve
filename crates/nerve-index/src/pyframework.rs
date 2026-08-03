//! `py-framework` — HTTP routes that FastAPI and Flask declare, read from the source alone.
//!
//! # What this extractor claims, and what it refuses to claim
//!
//! One thing: **the source declares an endpoint at this address, and names this symbol as its
//! handler.** That is `Endpoint SERVED_BY Function|Method`, `FRAMEWORK_RULE` / `DIRECT`.
//!
//! It does not claim the route is reachable in production, that middleware permits access, that
//! dynamic configuration has not replaced it, that a decorator-generated wrapper preserves the
//! handler's runtime identity, or that two matching path strings denote one deployed endpoint. A
//! registration proves a table entry, not an execution — the same distinction ADR-0005 draws for
//! `COVERS`, and [`Relation::ServedBy`] must never be relabelled as a call.
//!
//! # Why the decorator's *name* is not enough
//!
//! `.get`, `.post` and `.route` are ordinary method names. `fixtures/py-framework/negative.py`
//! contains five decorators spelled exactly like route decorators, on a cache, on a class named
//! `Router`, and on a bare module-level function. A rule that matched the spelling would emit five
//! false positives.
//!
//! So the receiver is traced, through two ordinary binding lookups, neither of which is name
//! matching:
//!
//! 1. `app` must be bound **at module scope in this file** to a call whose callee resolves to a
//!    framework constructor;
//! 2. that constructor name must itself be bound by an import from the framework's own package.
//!
//! Aliases fall out of this for free — `from fastapi import FastAPI as WebApp` binds `WebApp` to
//! `fastapi.FastAPI`, and the lookup follows the binding rather than comparing text.
//!
//! # The stated lower bound
//!
//! Requirement 1 says *in this file*. `from .main import app` followed by `@app.get(...)` is a real
//! route that this extractor does not record, because proving `app` is a FastAPI instance would
//! mean tracking a value across modules rather than reading a binding. That is the same kind of
//! deliberate refusal as 9a's `sys.path` poisoning: it drops an edge that a more speculative rule
//! would emit. It is **counted** as `app-not-local`, so the limit is measured rather than invisible.
//!
//! # Where nothing is counted either
//!
//! When the receiver cannot be traced to a framework constructor at all, this extractor emits
//! nothing **and counts nothing**. Nerve does not know that `@cache.get("/x")` was meant to be a
//! route, so incrementing a missed-route tally there would itself be a false claim about the
//! repository. `negative.py` therefore contributes zero endpoints and zero unsupported counts, and
//! the fixture gate asserts both.
//!
//! # Executes nothing
//!
//! A decorator is read, never called. `fixtures/py-framework` has no hostile file, but
//! `crates/nerve-cli/tests/no_subprocess.rs` indexes a repository whose decorator factory, module
//! top level and `setup.py` all try to write marker files and spawn processes, and asserts zero
//! markers with a non-zero entity count.

use std::collections::{BTreeMap, BTreeSet};

use nerve_core::vocab::{Directness, EndpointKind, EntityKind, EvidenceSourceType, Relation};
use nerve_core::{ids, Span};
use tree_sitter::Node;

use crate::error::Result;
use crate::lang::Language;
use crate::pystruct::{PyImportForm, PyModuleExtraction, PySymbol};

/// Named children as a vector, so the cursor's lifetime does not leak into the walk.
///
/// Defined here rather than imported: `pystruct` and `pyrefs` each keep a private copy, and a
/// shared one would have to be public API on a module whose surface is deliberately narrow.
fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
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

/// Stable extractor id.
pub const EXTRACTOR_ID: &str = "py-framework";

/// Extractor version. Bumping this invalidates `module_facts.framework_version` for every Python
/// file, which is what makes a rule change take effect on an existing index.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The only evidence source type this extractor may emit.
///
/// `FRAMEWORK_RULE` has been declared in the vocabulary since Slice 1 and never emitted; this is
/// its first emitter. `Directness::Direct` rather than `Inferred` because the decorator and the
/// function it decorates are adjacent in one syntax tree — the rule reads a declaration, it does
/// not conclude one.
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::FrameworkRule];

/// The only relation this extractor may emit.
pub const DECLARED_RELATIONS: [Relation; 1] = [Relation::ServedBy];

/// A framework Nerve has a rule for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Framework {
    /// FastAPI: `FastAPI()` and `APIRouter()`.
    FastApi,
    /// Flask: `Flask()` and `Blueprint()`.
    Flask,
}

impl Framework {
    /// Every framework with a rule, in declaration order.
    pub const ALL: [Framework; 2] = [Framework::FastApi, Framework::Flask];

    /// Tag recorded in `entity.meta.framework` and in observation details.
    pub fn as_str(self) -> &'static str {
        match self {
            Framework::FastApi => "fastapi",
            Framework::Flask => "flask",
        }
    }

    /// The import package whose constructors this rule trusts.
    fn package(self) -> &'static str {
        match self {
            Framework::FastApi => "fastapi",
            Framework::Flask => "flask",
        }
    }

    /// Constructor names that produce a routable application object.
    fn constructors(self) -> &'static [&'static str] {
        match self {
            Framework::FastApi => &["FastAPI", "APIRouter"],
            Framework::Flask => &["Flask", "Blueprint"],
        }
    }

    /// Whether this framework's `route` decorator takes a `methods=` keyword.
    ///
    /// Flask's does. FastAPI has no `route` decorator at all, so the question never arises for it,
    /// and treating one as a route would invent an API the framework does not have.
    fn has_route_decorator(self) -> bool {
        matches!(self, Framework::Flask)
    }

    /// Rule id recorded on every endpoint, so an observation names the rule that produced it.
    ///
    /// Public because the pipeline stamps it into entity metadata and observation details.
    pub fn rule_id(self) -> &'static str {
        match self {
            Framework::FastApi => "fastapi-route-decorator",
            Framework::Flask => "flask-route-decorator",
        }
    }
}

/// HTTP methods a decorator name may carry directly.
///
/// Both frameworks spell these identically, and both derive them from the HTTP standard rather than
/// from anything framework-specific.
const METHOD_DECORATORS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

/// Flask's documented default when `@app.route(...)` carries no `methods=`.
///
/// The framework's default, not Nerve's guess: Flask's own documentation states that a view
/// function accepts `GET` unless told otherwise.
const FLASK_DEFAULT_METHOD: &str = "GET";

/// A construct the rule read and declined, named so the tally is auditable.
///
/// Every member is counted by form. A silently growing set fails the fixture gate — the discipline
/// Slice 9b introduced for unmodelled call sites, applied here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnsupportedForm {
    /// The application object is bound in another module.
    AppNotLocal,
    /// The path argument is not a plain string literal, or is absent.
    PathNotLiteral,
    /// The decorated thing is not a declared symbol — a lambda, or a bare call.
    HandlerNotASymbol,
    /// Flask's `methods=` is not a list of string literals.
    MethodsNotLiteral,
}

impl UnsupportedForm {
    /// Every form, in declaration order.
    pub const ALL: [UnsupportedForm; 4] = [
        UnsupportedForm::AppNotLocal,
        UnsupportedForm::PathNotLiteral,
        UnsupportedForm::HandlerNotASymbol,
        UnsupportedForm::MethodsNotLiteral,
    ];

    /// Tag used in the tally and in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            UnsupportedForm::AppNotLocal => "app-not-local",
            UnsupportedForm::PathNotLiteral => "path-not-literal",
            UnsupportedForm::HandlerNotASymbol => "handler-not-a-symbol",
            UnsupportedForm::MethodsNotLiteral => "methods-not-literal",
        }
    }
}

/// One declared endpoint and the symbol that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyEndpoint {
    /// Entity id from [`ids::endpoint_id`].
    pub entity_id: String,
    /// Canonical address, e.g. `GET /users/{user_id}`. This is the entity name, so FTS5 indexes it.
    pub address: String,
    /// HTTP method, upper case.
    pub method: String,
    /// Path **as declared**. No prefix composition.
    pub path: String,
    /// Which framework's rule produced it.
    pub framework: Framework,
    /// Entity id of the handler symbol.
    pub handler_entity_id: String,
    /// Source range of the decorator that declares it.
    pub span: Span,
}

/// What one module's framework rules found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyFrameworkExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Endpoints declared here, in source order.
    pub endpoints: Vec<PyEndpoint>,
    /// Constructs read and declined, counted by form.
    pub unsupported_by_form: BTreeMap<&'static str, usize>,
    /// Addresses declared more than once in this module, with their declaration count.
    ///
    /// Both edges are kept: the source really does say it twice, and choosing one would resolve an
    /// ambiguity the evidence does not resolve.
    pub ambiguous_addresses: BTreeMap<String, usize>,
}

impl PyFrameworkExtraction {
    fn count(&mut self, form: UnsupportedForm) {
        *self.unsupported_by_form.entry(form.as_str()).or_insert(0) += 1;
    }
}

/// Read the HTTP routes one Python module declares.
///
/// `extraction` is 9a's structural result for the same file, which supplies the symbol entities the
/// endpoints point at and the import sites the constructor lookup walks.
pub fn extract_framework(
    project_id: &str,
    rel_path: &str,
    source: &str,
    extraction: &PyModuleExtraction,
) -> Result<PyFrameworkExtraction> {
    let mut parser = Language::Python.parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::error::IndexError::Parser(format!("parse failed: {rel_path}")))?;
    let root = tree.root_node();
    let bytes = source.as_bytes();

    let mut out = PyFrameworkExtraction {
        rel_path: rel_path.to_string(),
        ..PyFrameworkExtraction::default()
    };

    let constructors = framework_constructors(extraction);
    let apps = application_objects(root, bytes, &constructors);
    let imported_names = imported_locals(extraction);

    let mut walker = Walker {
        source: bytes,
        symbols: &extraction.symbols,
        apps: &apps,
        imported: &imported_names,
        out: &mut out,
    };
    walker.visit(root, project_id, rel_path);

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for endpoint in &out.endpoints {
        *seen.entry(endpoint.address.clone()).or_insert(0) += 1;
    }
    out.ambiguous_addresses = seen.into_iter().filter(|(_, count)| *count > 1).collect();

    Ok(out)
}

/// Local names bound, by import, to a framework constructor: `local name -> framework`.
///
/// Only `from <package> import <Constructor>` is read. `import fastapi` followed by
/// `fastapi.FastAPI()` is handled separately, through the dotted callee.
fn framework_constructors(extraction: &PyModuleExtraction) -> BTreeMap<String, Framework> {
    let mut out = BTreeMap::new();
    for site in &extraction.imports {
        if site.form != PyImportForm::FromImport {
            continue;
        }
        let Some(framework) = Framework::ALL
            .into_iter()
            .find(|f| site.raw_specifier == f.package())
        else {
            continue;
        };
        for binding in &site.bindings {
            let (Some(imported), Some(local)) = (&binding.imported, &binding.local) else {
                continue;
            };
            if framework.constructors().contains(&imported.as_str()) {
                out.insert(local.clone(), framework);
            }
        }
    }
    out
}

/// Every local name a module imported anything under, whatever the form.
///
/// Used to tell `app-not-local` (a real route this rule declines) from an untraceable receiver
/// (something Nerve has no reason to think is a route at all). Only a name that came from an import
/// is counted as the former.
fn imported_locals(extraction: &PyModuleExtraction) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for site in &extraction.imports {
        for binding in &site.bindings {
            if let Some(local) = &binding.local {
                out.insert(local.clone());
            }
        }
    }
    out
}

/// Module-scope names bound to a framework application object: `name -> framework`.
///
/// Module scope only, and by design: a route decorator refers to a name, and only a module-scope
/// assignment is visible to every decorator in the file without tracking control flow.
fn application_objects(
    root: Node,
    source: &[u8],
    constructors: &BTreeMap<String, Framework>,
) -> BTreeMap<String, Framework> {
    let mut out = BTreeMap::new();
    for statement in named_children(root) {
        if statement.kind() != "expression_statement" {
            continue;
        }
        for child in named_children(statement) {
            if child.kind() != "assignment" {
                continue;
            }
            let (Some(left), Some(right)) = (
                child.child_by_field_name("left"),
                child.child_by_field_name("right"),
            ) else {
                continue;
            };
            if left.kind() != "identifier" || right.kind() != "call" {
                continue;
            }
            let Some(callee) = right
                .child_by_field_name("function")
                .and_then(|function| dotted_name(function, source))
            else {
                continue;
            };
            if let Some(framework) = framework_of_constructor(&callee, constructors) {
                out.insert(text(left, source), framework);
            }
        }
    }
    out
}

/// Which framework a constructor callee belongs to, if any.
///
/// Two spellings are accepted, and both are bindings rather than name matches: a local name the
/// module imported from the framework's package, or the dotted form `fastapi.FastAPI` whose head is
/// the package itself.
fn framework_of_constructor(
    callee: &str,
    constructors: &BTreeMap<String, Framework>,
) -> Option<Framework> {
    if let Some(framework) = constructors.get(callee) {
        return Some(*framework);
    }
    let (head, tail) = callee.rsplit_once('.')?;
    Framework::ALL
        .into_iter()
        .find(|f| head == f.package() && f.constructors().contains(&tail))
}

fn text(node: Node, source: &[u8]) -> String {
    std::str::from_utf8(&source[node.byte_range()])
        .unwrap_or_default()
        .to_string()
}

/// Dotted name of an expression, or `None` when it is not one.
///
/// Deliberately does **not** descend into a `call`: `@HANDLERS["get"]("/x")` and
/// `@make_router().get("/x")` are not dotted names, and reading through them would be the
/// speculation this rule refuses.
fn dotted_name(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source)),
        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;
            Some(format!(
                "{}.{}",
                dotted_name(object, source)?,
                text(attribute, source)
            ))
        }
        _ => None,
    }
}

/// Value of a `string` node when it is a plain literal.
///
/// An f-string carries `interpolation` children whose value depends on runtime state, so it is not
/// a literal. Same rule, and the same reason, as `pystruct`'s reader.
fn string_value(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut value = String::new();
    for child in named_children(node) {
        match child.kind() {
            "string_content" => value.push_str(&text(child, source)),
            "interpolation" => return None,
            _ => {}
        }
    }
    Some(value)
}

struct Walker<'a> {
    source: &'a [u8],
    symbols: &'a [PySymbol],
    apps: &'a BTreeMap<String, Framework>,
    imported: &'a BTreeSet<String>,
    out: &'a mut PyFrameworkExtraction,
}

impl Walker<'_> {
    fn visit(&mut self, node: Node, project_id: &str, rel_path: &str) {
        match node.kind() {
            "decorated_definition" => self.visit_decorated(node, project_id, rel_path),
            "call" => self.visit_registration_call(node, project_id, rel_path),
            _ => {}
        }
        for child in named_children(node) {
            self.visit(child, project_id, rel_path);
        }
    }

    /// The applied-decorator form: `app.get("/x")(handler)`.
    ///
    /// A decorator is sugar for exactly this, and both frameworks accept it written out. It matters
    /// because it is the form whose handler need not be a declared symbol — `app.get("/x")(lambda:
    /// [])` registers a callable that no entity names, which is the only way
    /// `handler-not-a-symbol` can arise.
    ///
    /// The single-call form `app.get("/x")` on its own is **not** a registration: it evaluates the
    /// decorator and discards it, binding no handler. Treating it as one would assert a route with
    /// no target.
    fn visit_registration_call(&mut self, node: Node, project_id: &str, rel_path: &str) {
        let Some(inner) = node.child_by_field_name("function") else {
            return;
        };
        if inner.kind() != "call" {
            return;
        }
        // The inner call must itself be a route decorator on a known application object.
        let Some(callee) = inner
            .child_by_field_name("function")
            .and_then(|function| dotted_name(function, self.source))
        else {
            return;
        };
        let Some((receiver, method_name)) = callee.rsplit_once('.') else {
            return;
        };
        let framework = match self.apps.get(receiver) {
            Some(framework) => *framework,
            None => {
                if self.imported.contains(receiver) && self.is_route_decorator_name(method_name) {
                    self.out.count(UnsupportedForm::AppNotLocal);
                }
                return;
            }
        };
        let Some(methods) = self.methods_for(framework, method_name, inner) else {
            return;
        };
        let Some(path) = self.path_of(inner) else {
            self.out.count(UnsupportedForm::PathNotLiteral);
            return;
        };

        // The outer call's first positional argument is the handler.
        let handler = node
            .child_by_field_name("arguments")
            .and_then(|arguments| {
                named_children(arguments)
                    .into_iter()
                    .find(|argument| argument.kind() != "comment")
            })
            .and_then(|argument| self.handler_from_argument(argument));

        let Some(handler_entity_id) = handler else {
            self.out.count(UnsupportedForm::HandlerNotASymbol);
            return;
        };

        let span = span_of(node);
        for method in methods {
            self.push_endpoint(
                project_id,
                rel_path,
                framework,
                &method,
                &path,
                &handler_entity_id,
                span,
            );
        }
    }

    /// The symbol an argument names, when it names one at all.
    ///
    /// A lambda, a call result, an attribute and a subscript all yield `None`: each is a callable
    /// the source does not give a declared name, so no entity can be the edge's target.
    fn handler_from_argument(&self, argument: Node) -> Option<String> {
        if argument.kind() != "identifier" {
            return None;
        }
        let wanted = text(argument, self.source);
        self.symbols
            .iter()
            .find(|symbol| symbol.is_top_level() && symbol.name == wanted)
            .map(|symbol| symbol.entity_id.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn push_endpoint(
        &mut self,
        project_id: &str,
        rel_path: &str,
        framework: Framework,
        method: &str,
        path: &str,
        handler_entity_id: &str,
        span: Span,
    ) {
        self.out.endpoints.push(PyEndpoint {
            entity_id: ids::endpoint_id(
                project_id,
                rel_path,
                EndpointKind::HttpRoute,
                method,
                path,
            ),
            address: format!("{method} {path}"),
            method: method.to_string(),
            path: path.to_string(),
            framework,
            handler_entity_id: handler_entity_id.to_string(),
            span,
        });
    }

    /// Read every decorator on one definition.
    ///
    /// Every decorator is examined, not only the outermost, so a route decorator stacked above or
    /// below an ordinary one is still found. Whether the ordinary one preserves the handler's
    /// runtime identity is unknowable from the source, which is exactly why `SERVED_BY` states a
    /// declaration rather than a dispatch.
    fn visit_decorated(&mut self, node: Node, project_id: &str, rel_path: &str) {
        let Some(definition) = named_children(node)
            .into_iter()
            .find(|child| matches!(child.kind(), "function_definition" | "class_definition"))
        else {
            return;
        };
        let Some(handler) = self.symbol_at(definition) else {
            return;
        };
        if handler.kind == EntityKind::Class {
            // A class decorated with a route decorator is not a handler in either framework.
            return;
        }

        for decorator in named_children(node) {
            if decorator.kind() != "decorator" {
                continue;
            }
            let Some(expression) = named_children(decorator).into_iter().next() else {
                continue;
            };
            self.visit_decorator(decorator, expression, &handler, project_id, rel_path);
        }
    }

    fn visit_decorator(
        &mut self,
        decorator: Node,
        expression: Node,
        handler: &PySymbol,
        project_id: &str,
        rel_path: &str,
    ) {
        // A route decorator is always a call: `@app.get("/x")`, never `@app.get`.
        if expression.kind() != "call" {
            return;
        }
        let Some(function) = expression.child_by_field_name("function") else {
            return;
        };
        let Some(callee) = dotted_name(function, self.source) else {
            // `@HANDLERS["get"](...)` — a subscript. Nothing is emitted and **nothing is counted**:
            // Nerve has no reason to believe a dict lookup was meant to register a route, and 9a
            // already counts decorator expressions whose form is not a dotted name. Counting it
            // here as a route Nerve missed would assert a route the source never states, which is a
            // false claim pointing the opposite way from a false positive.
            return;
        };
        let Some((receiver, method_name)) = callee.rsplit_once('.') else {
            // A bare `@get("/x")`. Nerve has no reason to think this is a route.
            return;
        };

        let framework = match self.apps.get(receiver) {
            Some(framework) => *framework,
            None => {
                // The receiver names something imported, and a route decorator was written on it.
                // That is a real route this rule declines — the stated cross-module lower bound.
                // Anything else is not known to be a route at all, so nothing is counted.
                if self.imported.contains(receiver) && self.is_route_decorator_name(method_name) {
                    self.out.count(UnsupportedForm::AppNotLocal);
                }
                return;
            }
        };

        let methods = match self.methods_for(framework, method_name, expression) {
            Some(methods) => methods,
            None => return,
        };

        let Some(path) = self.path_of(expression) else {
            self.out.count(UnsupportedForm::PathNotLiteral);
            return;
        };

        let span = span_of(decorator);
        for method in methods {
            self.push_endpoint(
                project_id,
                rel_path,
                framework,
                &method,
                &path,
                &handler.entity_id,
                span,
            );
        }
    }

    /// Whether a decorator name is one a route rule would recognise on a framework object.
    ///
    /// Used only to decide whether an untraceable receiver deserves an `app-not-local` count, so
    /// that `@app.middleware("http")` on an imported `app` is not reported as a missed route.
    fn is_route_decorator_name(&self, name: &str) -> bool {
        METHOD_DECORATORS.contains(&name) || name == "route"
    }

    /// The HTTP methods a decorator declares, or `None` when it declares no route at all.
    fn methods_for(
        &mut self,
        framework: Framework,
        method_name: &str,
        call: Node,
    ) -> Option<Vec<String>> {
        if METHOD_DECORATORS.contains(&method_name) {
            return Some(vec![method_name.to_uppercase()]);
        }
        if method_name != "route" || !framework.has_route_decorator() {
            return None;
        }
        match self.methods_keyword(call) {
            MethodsKeyword::Absent => Some(vec![FLASK_DEFAULT_METHOD.to_string()]),
            MethodsKeyword::Literal(methods) => Some(methods),
            MethodsKeyword::NotLiteral => {
                // Present but unreadable. Falling back to Flask's default would assert `GET` for a
                // route whose methods the source did not state — inventing an answer rather than
                // declining one.
                self.out.count(UnsupportedForm::MethodsNotLiteral);
                None
            }
        }
    }

    fn methods_keyword(&self, call: Node) -> MethodsKeyword {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return MethodsKeyword::Absent;
        };
        for argument in named_children(arguments) {
            if argument.kind() != "keyword_argument" {
                continue;
            }
            let Some(name) = argument.child_by_field_name("name") else {
                continue;
            };
            if text(name, self.source) != "methods" {
                continue;
            }
            let Some(value) = argument.child_by_field_name("value") else {
                return MethodsKeyword::NotLiteral;
            };
            if !matches!(value.kind(), "list" | "tuple" | "set") {
                return MethodsKeyword::NotLiteral;
            }
            let mut methods = Vec::new();
            for element in named_children(value) {
                match string_value(element, self.source) {
                    Some(method) => methods.push(method.to_uppercase()),
                    None => return MethodsKeyword::NotLiteral,
                }
            }
            if methods.is_empty() {
                return MethodsKeyword::NotLiteral;
            }
            return MethodsKeyword::Literal(methods);
        }
        MethodsKeyword::Absent
    }

    /// The first positional argument, when it is a plain string literal.
    fn path_of(&self, call: Node) -> Option<String> {
        let arguments = call.child_by_field_name("arguments")?;
        let first = named_children(arguments)
            .into_iter()
            .find(|argument| argument.kind() != "comment")?;
        string_value(first, self.source)
    }

    /// The symbol 9a extracted for this definition, matched by the name node's position.
    fn symbol_at(&self, definition: Node) -> Option<PySymbol> {
        let name = definition.child_by_field_name("name")?;
        let wanted = text(name, self.source);
        let start = definition.start_byte();
        let end = definition.end_byte();
        self.symbols
            .iter()
            .find(|symbol| {
                symbol.name == wanted
                    // A decorated definition's span includes its decorators, so 9a's span starts
                    // at or before the bare definition and ends at or after it.
                    && symbol.span.start_byte <= start
                    && symbol.span.end_byte >= end
            })
            .cloned()
    }
}

/// What a Flask `methods=` argument turned out to be.
enum MethodsKeyword {
    /// No `methods=` at all — the framework's documented default applies.
    Absent,
    /// A readable sequence of string literals.
    Literal(Vec<String>),
    /// Present, but its value is not statically readable.
    NotLiteral,
}

/// The evidence typing every framework observation carries.
pub fn directness() -> Directness {
    Directness::Direct
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pystruct::extract_module;

    const PID: &str = "00000000000000000000000000000001";

    fn run(rel_path: &str, source: &str) -> PyFrameworkExtraction {
        let structural = extract_module(PID, rel_path, source).expect("structural extraction");
        extract_framework(PID, rel_path, source, &structural).expect("framework extraction")
    }

    fn addresses(extraction: &PyFrameworkExtraction) -> Vec<String> {
        extraction
            .endpoints
            .iter()
            .map(|endpoint| endpoint.address.clone())
            .collect()
    }

    #[test]
    fn a_fastapi_decorator_declares_an_endpoint_served_by_the_decorated_function() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\n@app.get(\"/users\")\ndef list_users():\n    return []\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /users"]);
        let endpoint = &extraction.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert_eq!(endpoint.path, "/users");
        assert_eq!(endpoint.framework, Framework::FastApi);
        assert!(endpoint.handler_entity_id.starts_with("fn_"));
        assert!(extraction.unsupported_by_form.is_empty());
    }

    /// The whole reason the receiver is traced rather than the decorator name matched.
    #[test]
    fn a_decorator_spelled_like_a_route_on_a_non_framework_object_is_not_a_route() {
        let extraction = run(
            "cache.py",
            "class Cache:\n    def get(self, key):\n        return None\n\n\ncache = Cache()\n\n\n@cache.get(\"/not-a-route\")\ndef handler():\n    return None\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert!(
            extraction.unsupported_by_form.is_empty(),
            "Nerve has no reason to think this is a route, so counting it as one missed would \
             itself be a false claim: {:?}",
            extraction.unsupported_by_form
        );
    }

    #[test]
    fn an_aliased_constructor_import_still_resolves_through_the_binding() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI as WebApp\n\napi = WebApp()\n\n\n@api.get(\"/aliased\")\ndef handler():\n    return []\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /aliased"]);
    }

    #[test]
    fn a_dotted_constructor_call_resolves_through_the_package_head() {
        let extraction = run(
            "api.py",
            "import fastapi\n\napp = fastapi.FastAPI()\n\n\n@app.post(\"/x\")\ndef handler():\n    return []\n",
        );
        assert_eq!(addresses(&extraction), vec!["POST /x"]);
    }

    #[test]
    fn a_router_prefix_is_not_composed_into_the_declared_path() {
        let extraction = run(
            "api.py",
            "from fastapi import APIRouter\n\nrouter = APIRouter(prefix=\"/v1\")\n\n\n@router.get(\"/items\")\ndef handler():\n    return []\n",
        );
        assert_eq!(
            addresses(&extraction),
            vec!["GET /items"],
            "the prefix applies only if another file calls include_router, so composing it here \
             would produce a confidently wrong URL"
        );
    }

    #[test]
    fn flask_route_without_methods_uses_the_frameworks_documented_default() {
        let extraction = run(
            "app.py",
            "from flask import Flask\n\napp = Flask(__name__)\n\n\n@app.route(\"/health\")\ndef health():\n    return \"ok\"\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /health"]);
    }

    #[test]
    fn flask_methods_list_declares_one_endpoint_per_method() {
        let extraction = run(
            "app.py",
            "from flask import Flask\n\napp = Flask(__name__)\n\n\n@app.route(\"/both\", methods=[\"GET\", \"POST\"])\ndef both():\n    return \"ok\"\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /both", "POST /both"]);
    }

    /// Falling back to `GET` here would assert a method the source did not state.
    #[test]
    fn an_unreadable_methods_argument_declines_rather_than_defaulting() {
        let extraction = run(
            "app.py",
            "from flask import Flask\n\napp = Flask(__name__)\nMETHODS = [\"GET\"]\n\n\n@app.route(\"/x\", methods=METHODS)\ndef handler():\n    return \"ok\"\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert_eq!(
            extraction.unsupported_by_form.get("methods-not-literal"),
            Some(&1)
        );
    }

    /// FastAPI has no `route` decorator, so reading one would invent an API.
    #[test]
    fn fastapi_has_no_route_decorator() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\n@app.route(\"/x\")\ndef handler():\n    return []\n",
        );
        assert!(extraction.endpoints.is_empty());
    }

    #[test]
    fn a_computed_path_is_counted_not_guessed() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\nPREFIX = \"/p\"\n\n\n@app.get(PREFIX + \"/items\")\ndef handler():\n    return []\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert_eq!(
            extraction.unsupported_by_form.get("path-not-literal"),
            Some(&1)
        );
    }

    #[test]
    fn an_f_string_path_is_not_a_literal() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\nP = \"x\"\n\n\n@app.get(f\"/users/{P}\")\ndef handler():\n    return []\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert_eq!(
            extraction.unsupported_by_form.get("path-not-literal"),
            Some(&1)
        );
    }

    #[test]
    fn an_imported_application_object_is_counted_as_the_stated_lower_bound() {
        let extraction = run(
            "routes.py",
            "from .main import app\n\n\n@app.get(\"/imported\")\ndef handler():\n    return []\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert_eq!(
            extraction.unsupported_by_form.get("app-not-local"),
            Some(&1),
            "this is a real route the rule declines, so unlike an unknown receiver it is counted"
        );
    }

    /// A non-route decorator on an imported object must not be reported as a missed route.
    #[test]
    fn a_non_route_decorator_on_an_imported_object_is_not_counted() {
        let extraction = run(
            "routes.py",
            "from .main import app\n\n\n@app.middleware(\"http\")\ndef handler(request, call_next):\n    return call_next(request)\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert!(extraction.unsupported_by_form.is_empty());
    }

    #[test]
    fn a_route_decorator_stacked_with_an_ordinary_one_is_found_either_way() {
        let below = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\ndef trace(fn):\n    return fn\n\n\n@app.get(\"/below\")\n@trace\ndef handler():\n    return []\n",
        );
        assert_eq!(addresses(&below), vec!["GET /below"]);

        let above = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\ndef trace(fn):\n    return fn\n\n\n@trace\n@app.get(\"/above\")\ndef handler():\n    return []\n",
        );
        assert_eq!(addresses(&above), vec!["GET /above"]);
    }

    #[test]
    fn a_method_can_be_a_handler() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\nclass Views:\n    @app.get(\"/m\")\n    def as_method(self):\n        return []\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /m"]);
        assert!(extraction.endpoints[0]
            .handler_entity_id
            .starts_with("meth_"));
    }

    #[test]
    fn the_same_address_declared_twice_is_ambiguous_and_keeps_both_edges() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\n@app.get(\"/twice\")\ndef first():\n    return []\n\n\n@app.get(\"/twice\")\ndef second():\n    return []\n",
        );
        assert_eq!(extraction.endpoints.len(), 2);
        assert_eq!(extraction.ambiguous_addresses.get("GET /twice"), Some(&2));
        assert_eq!(
            extraction.endpoints[0].entity_id, extraction.endpoints[1].entity_id,
            "one declared address in one module is one endpoint, served by two symbols"
        );
    }

    #[test]
    fn a_syntax_error_does_not_lose_the_routes_above_it() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\n@app.get(\"/ok\")\ndef ok():\n    return []\n\n\ndef broken(:\n    return\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /ok"]);
    }

    /// The applied-decorator form, and the only way `handler-not-a-symbol` can arise.
    ///
    /// The first version of this test asserted only `endpoints.is_empty()`, and it **passed
    /// vacuously**: the walker visited `decorated_definition` alone, so `app.get(…)(…)` was never
    /// read and no tally moved. A reviewing agent caught it, and the probe confirmed
    /// `unsupported_by_form: {}` where the fixture requires 1. Asserting the count is what makes
    /// the check non-vacuous — the same trap that produced two false T7 passes on this project.
    #[test]
    fn an_applied_decorator_with_a_lambda_handler_is_counted_not_emitted() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\napp.get(\"/lambda\")(lambda: [])\n",
        );
        assert!(
            extraction.endpoints.is_empty(),
            "a lambda is not a declared symbol, so there is no entity the endpoint could serve"
        );
        assert_eq!(
            extraction.unsupported_by_form.get("handler-not-a-symbol"),
            Some(&1),
            "the form must be counted, or the fixture's tally has a member no code produces"
        );
    }

    /// The same form with a named handler is a real endpoint, not an unsupported one.
    ///
    /// Without this, `handler-not-a-symbol` could be produced by a rule that simply refuses every
    /// applied decorator, and the count above would still pass.
    #[test]
    fn an_applied_decorator_with_a_named_handler_declares_an_endpoint() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\n\ndef handler():\n    return []\n\n\napp.get(\"/named\")(handler)\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /named"]);
        assert!(extraction.endpoints[0].handler_entity_id.starts_with("fn_"));
        assert!(extraction.unsupported_by_form.is_empty());
    }

    /// Evaluating a decorator without applying it registers nothing.
    #[test]
    fn a_bare_decorator_call_is_not_a_registration() {
        let extraction = run(
            "api.py",
            "from fastapi import FastAPI\n\napp = FastAPI()\n\napp.get(\"/discarded\")\n",
        );
        assert!(
            extraction.endpoints.is_empty(),
            "the decorator is evaluated and discarded; no handler is bound"
        );
        assert!(extraction.unsupported_by_form.is_empty());
    }

    /// The tally is a closed vocabulary, and every member is reachable from a real construct.
    #[test]
    fn every_unsupported_form_has_a_distinct_tag() {
        let tags: Vec<&str> = UnsupportedForm::ALL.iter().map(|f| f.as_str()).collect();
        let unique: BTreeSet<&str> = tags.iter().copied().collect();
        assert_eq!(tags.len(), unique.len(), "duplicate unsupported-form tag");
        assert_eq!(UnsupportedForm::ALL.len(), 4);
    }

    #[test]
    fn the_extractor_declares_only_framework_rule_and_served_by() {
        assert_eq!(DECLARED_SOURCE_TYPES, [EvidenceSourceType::FrameworkRule]);
        assert_eq!(DECLARED_RELATIONS, [Relation::ServedBy]);
        assert_eq!(directness(), Directness::Direct);
        assert!(
            !DECLARED_RELATIONS.contains(&Relation::Calls),
            "a registration proves a table entry, not an execution"
        );
    }

    #[test]
    fn each_framework_states_its_package_and_constructors() {
        for framework in Framework::ALL {
            assert!(!framework.package().is_empty());
            assert!(!framework.constructors().is_empty());
            assert!(!framework.rule_id().is_empty());
        }
        assert!(Framework::Flask.has_route_decorator());
        assert!(!Framework::FastApi.has_route_decorator());
    }
}
