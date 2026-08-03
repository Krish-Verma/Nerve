//! The `py-structural` extractor.
//!
//! Reads one Python module's syntax tree and reports what the tree literally says: which
//! functions, classes and methods are declared, which modules are imported, and which names
//! `__all__` publishes. It performs no resolution and emits no assertions — resolution and
//! assertion construction happen in [`crate::pipeline`], which is also where the evidence
//! profile is attached.
//!
//! # Why this is a module and not a branch in `extract.rs`
//!
//! Every observation records an extractor id, and Slice 5d-i was a corrective slice for exactly
//! the failure of stamping one extractor's name on another's claim: directory containment said
//! `ts-js-structural` in a repository holding no TypeScript. A Python observation must never
//! say `ts-js-structural` either, so Python gets its own id, its own version, its own declared
//! source types and its own batch, verified separately by the check the pipeline already runs.
//!
//! # What this extractor does not do
//!
//! No `CALLS`, `REFERENCES` or `EXTENDS` — those are Slice 9b's `py-reference`. And **no
//! `IMPLEMENTS` ever**: Python has no `implements` keyword, so `class C(SomeABC)` states
//! inheritance and nothing more.
//!
//! # What Python makes unknowable, and is therefore recorded rather than guessed
//!
//! An import statement makes two separable claims: *which module it names*, and *which names it
//! binds*. The first is resolved by [`crate::pyresolve`]. The second is **not statically
//! knowable** in three cases, and each is recorded as an `Unresolved` value with a reason
//! rather than dropped:
//!
//! - a wildcard `from x import *`, whose bound names live in a module's runtime namespace;
//! - a conditional import — inside `if`, `try`, a loop, a `with`, a function or a class body —
//!   which may or may not bind at module scope;
//! - a dynamic `importlib.import_module(...)` or `__import__(...)`, where even the module is
//!   chosen at runtime.
//!
//! And one case makes the *first* claim unsound too: a module that mutates `sys.path` has
//! changed where absolute specifiers point, so no absolute specifier in it may be resolved
//! against the repository layout. See [`PyModuleExtraction::sys_path_mutated`].
//!
//! # Unsupported, stated rather than silent
//!
//! Monkey patching, runtime attribute assignment, metaclasses, decorator-generated behaviour,
//! `__getattr__` and `globals()` mutation are all outside what a parse can see. Resolving them
//! would require executing repository code, which SECURITY.md T1 forbids absolutely. They are
//! unsupported, which is a stated state and not a gap this extractor pretends to have filled.

use std::collections::HashMap;

use tree_sitter::Node;

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::{EntityKind, EvidenceSourceType};

use crate::error::Result;
use crate::lang::Language;
use crate::pyresolve::PACKAGE_INIT;

/// Identifier of the Python structural extractor.
pub const EXTRACTOR_ID: &str = "py-structural";

/// Version of the extractor. Bump on any change to what it emits.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The evidence source types this extractor is permitted to emit (ADR-0003).
///
/// `AST_DIRECT` for what the tree literally states — containment, definitions, `__all__`
/// exports, unresolved imports — and `AST_RESOLVED` for the `IMPORTS` edges that survived
/// specifier resolution. Verified against every emitted observation by
/// [`nerve_core::model::GraphBatch::verify_declared_source_types`].
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 2] = [
    EvidenceSourceType::AstDirect,
    EvidenceSourceType::AstResolved,
];

/// Attribute names on `sys.path` whose call mutates the list.
///
/// A read (`sys.path.index`, `sys.path.count`) changes nothing and is deliberately absent.
const SYS_PATH_MUTATORS: [&str; 7] = [
    "append",
    "insert",
    "extend",
    "remove",
    "pop",
    "clear",
    "__setitem__",
];

/// The two callables that import a module named at runtime.
const DYNAMIC_IMPORT_CALLEES: [&str; 2] = ["importlib.import_module", "__import__"];

/// Where a symbol sits lexically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    /// A function body.
    Function,
    /// A class body.
    Class,
}

#[derive(Debug, Clone)]
struct ScopeFrame {
    token: String,
    kind: ScopeKind,
}

/// A declared symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PySymbol {
    /// Logical identifier.
    pub entity_id: String,
    /// Entity kind: `Function`, `Class` or `Method`.
    pub kind: EntityKind,
    /// Declared name.
    pub name: String,
    /// Enclosing lexical scope, joined with `.`.
    pub scope_path: String,
    /// Index among identically-named siblings, in source order.
    pub disambiguator: u32,
    /// Source range of the declaration, decorators included.
    pub span: Span,
    /// Canonical JSON metadata: `async`, `decorators`, and the count of decorator expressions
    /// whose form is not a dotted name.
    pub meta: Option<String>,
    /// Owning class, for methods.
    pub owner_class: Option<String>,
}

impl PySymbol {
    /// True when the symbol is declared at module top level.
    pub fn is_top_level(&self) -> bool {
        self.scope_path.is_empty()
    }
}

/// The syntactic form an import took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyImportForm {
    /// `import a.b` or `import a.b as c`.
    Import,
    /// `from a.b import c` or `from . import c`.
    FromImport,
    /// `from a.b import *`.
    Wildcard,
    /// `importlib.import_module(...)` or `__import__(...)`.
    Dynamic,
}

impl PyImportForm {
    /// Tag recorded in observation details.
    pub fn as_str(self) -> &'static str {
        match self {
            PyImportForm::Import => "import",
            PyImportForm::FromImport => "from-import",
            PyImportForm::Wildcard => "wildcard",
            PyImportForm::Dynamic => "dynamic",
        }
    }
}

/// One binding introduced by an import statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyBinding {
    /// Name in the imported module, or `*` for a wildcard.
    pub imported: Option<String>,
    /// Name bound locally.
    pub local: Option<String>,
}

/// An import site: a specifier exactly as written, plus everything the tree says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyImportSite {
    /// The specifier exactly as written, leading dots included. Empty for a dynamic import.
    pub raw_specifier: String,
    /// Syntactic form.
    pub form: PyImportForm,
    /// Bindings introduced, in source order.
    pub bindings: Vec<PyBinding>,
    /// The **outermost** enclosing construct when the statement is not at module top level.
    ///
    /// `Some("try")`, `Some("if")`, `Some("function")` and so on. `None` means the statement runs
    /// exactly once, unconditionally, when the module is imported.
    pub conditional: Option<&'static str>,
    /// For a dynamic import, which callable was used.
    pub dynamic_callee: Option<String>,
    /// For a dynamic import, the string literal argument when it had one.
    ///
    /// Recorded and **not resolved**: `importlib.import_module` is the runtime import hook, and a
    /// literal at one call site says nothing about the others. What the source wrote is evidence
    /// worth keeping; treating it as a specifier would be the guess this slice refuses.
    pub literal_argument: Option<String>,
    /// Source range of the statement.
    pub span: Span,
}

/// What a module's `__all__` says, when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllDeclaration {
    /// A list or tuple of string literals — the only form whose value a parse can read.
    Literal(Vec<String>),
    /// Present, but not a literal sequence of strings. The public surface is not knowable.
    Unsupported,
}

/// Everything one Python module's syntax tree literally states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyModuleExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Whether this module is a package's `__init__.py`.
    ///
    /// Recorded here and carried onto the `Module` entity's `meta`. Packageness is real, but a
    /// `Package` entity kind is a vocabulary change touching the UI mirror,
    /// `EntityKind::path_role` and every exhaustiveness test — for a fact that is already 1:1
    /// with a file Nerve indexes.
    pub is_package: bool,
    /// Whether the module mutates `sys.path`.
    ///
    /// Absolute import resolution reads `sys.path`, so once a module rewrites it, no absolute
    /// specifier in that module can be claimed to name a repository file. Relative imports are
    /// unaffected: they are resolved from `__package__`.
    pub sys_path_mutated: bool,
    /// Symbols declared, in source order.
    pub symbols: Vec<PySymbol>,
    /// Import sites, in source order.
    pub imports: Vec<PyImportSite>,
    /// What `__all__` said, when the module wrote one.
    pub all_declaration: Option<AllDeclaration>,
    /// Decorator expressions whose form is not a dotted name, and so were not recorded.
    pub unsupported_decorators: usize,
    /// Whether the parse produced any ERROR node.
    pub has_syntax_error: bool,
    /// Span covering the whole file.
    pub file_span: Span,
}

impl PyModuleExtraction {
    /// Index of the first module-scope symbol with this name, if any.
    pub fn top_level_symbol(&self, name: &str) -> Option<usize> {
        self.symbols
            .iter()
            .position(|symbol| symbol.is_top_level() && symbol.name == name)
    }

    /// The names `__all__` publishes, or an empty slice when it said nothing readable.
    pub fn exported_names(&self) -> &[String] {
        match &self.all_declaration {
            Some(AllDeclaration::Literal(names)) => names,
            _ => &[],
        }
    }
}

/// True when a repository-relative path is a package's `__init__.py`.
pub fn path_is_package_init(rel_path: &str) -> bool {
    rel_path == PACKAGE_INIT || rel_path.ends_with(&format!("/{PACKAGE_INIT}"))
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
    let children: Vec<Node<'tree>> = node.named_children(&mut cursor).collect();
    children
}

fn all_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    let children: Vec<Node<'tree>> = node.children(&mut cursor).collect();
    children
}

fn has_child_kind(node: Node, kind: &str) -> bool {
    all_children(node).iter().any(|child| child.kind() == kind)
}

/// The enclosing construct tag for a node kind that makes a statement conditional.
///
/// "Conditional" here means only *"not run exactly once at module import"*. A statement in a
/// function body may run many times or never; one in an `if` may not run at all; one in a class
/// body binds into the class namespace rather than the module's. All three make the set of
/// names bound at module scope something a parse cannot state.
fn conditional_tag(kind: &str) -> Option<&'static str> {
    match kind {
        "if_statement" => Some("if"),
        "try_statement" => Some("try"),
        "while_statement" => Some("while"),
        "for_statement" => Some("for"),
        "with_statement" => Some("with"),
        "match_statement" => Some("match"),
        "function_definition" => Some("function"),
        "class_definition" => Some("class"),
        _ => None,
    }
}

struct Extractor<'a> {
    source: &'a [u8],
    project_id: &'a str,
    rel_path: &'a str,
    scopes: Vec<ScopeFrame>,
    /// Enclosing constructs that make a statement conditional, outermost first.
    conditionals: Vec<&'static str>,
    disambiguators: HashMap<(EntityKind, String, String), u32>,
    out: PyModuleExtraction,
}

impl<'a> Extractor<'a> {
    fn text(&self, node: Node) -> String {
        std::str::from_utf8(&self.source[node.byte_range()])
            .unwrap_or_default()
            .to_string()
    }

    /// Value of a `string` node, when it is a plain literal.
    ///
    /// An f-string carries `interpolation` children whose value depends on runtime state, so it
    /// is not a literal and yields `None` rather than the text between the quotes.
    fn string_value(&self, node: Node) -> Option<String> {
        if node.kind() != "string" {
            return None;
        }
        let mut value = String::new();
        for child in named_children(node) {
            match child.kind() {
                "string_content" => value.push_str(&self.text(child)),
                "interpolation" => return None,
                _ => {}
            }
        }
        Some(value)
    }

    fn scope_path(&self) -> String {
        self.scopes
            .iter()
            .map(|frame| frame.token.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Scope token for a named declaration, given where it sits.
    ///
    /// A declaration directly inside a function body is not addressable from outside, so its
    /// token is wrapped: `outer.<local:inner>`. Class members and module-level declarations use
    /// their bare name. The rule is `ts-js-structural`'s, unchanged, so a qualified name means
    /// the same thing in both languages.
    fn token_for(&self, name: &str) -> String {
        match self.scopes.last().map(|frame| frame.kind) {
            Some(ScopeKind::Function) => format!("<local:{name}>"),
            _ => name.to_string(),
        }
    }

    fn define(
        &mut self,
        kind: EntityKind,
        name: &str,
        span: Span,
        meta: Option<String>,
        owner_class: Option<String>,
    ) -> Result<usize> {
        let scope_path = self.scope_path();
        let key = (kind, scope_path.clone(), name.to_string());
        let next = self.disambiguators.entry(key).or_insert(0);
        let disambiguator = *next;
        *next += 1;

        let entity_id = ids::symbol_id(
            kind,
            self.project_id,
            self.rel_path,
            &scope_path,
            name,
            disambiguator,
        )?;

        self.out.symbols.push(PySymbol {
            entity_id,
            kind,
            name: name.to_string(),
            scope_path,
            disambiguator,
            span,
            meta,
            owner_class,
        });
        Ok(self.out.symbols.len() - 1)
    }

    fn push_scope(&mut self, token: String, kind: ScopeKind) {
        self.scopes.push(ScopeFrame { token, kind });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn visit_children(&mut self, node: Node) -> Result<()> {
        for child in named_children(node) {
            self.visit(child)?;
        }
        Ok(())
    }

    /// Visit `node`'s children with `tag` recorded as an enclosing conditional construct.
    fn visit_children_conditional(&mut self, node: Node, tag: &'static str) -> Result<()> {
        self.conditionals.push(tag);
        let result = self.visit_children(node);
        self.conditionals.pop();
        result
    }

    fn conditional(&self) -> Option<&'static str> {
        self.conditionals.first().copied()
    }

    fn visit(&mut self, node: Node) -> Result<()> {
        match node.kind() {
            "decorated_definition" => self.visit_decorated(node, None),
            "function_definition" => self.visit_function(node, span_of(node), &[], None),
            "class_definition" => self.visit_class(node, span_of(node), &[]),
            "import_statement" => self.visit_import(node),
            "import_from_statement" | "future_import_statement" => self.visit_import_from(node),
            "call" => self.visit_call(node),
            "expression_statement" => self.visit_expression_statement(node),
            "assignment" | "augmented_assignment" => {
                self.note_sys_path(node);
                self.visit_children(node)
            }
            kind => match conditional_tag(kind) {
                Some(tag) => self.visit_children_conditional(node, tag),
                None => self.visit_children(node),
            },
        }
    }

    // ---- declarations ----------------------------------------------------------------------

    /// Dotted callee of an expression: `staticmethod`, `functools.lru_cache`, `app.route`.
    ///
    /// `None` for anything that is not a name, an attribute chain, or a call on one. That is a
    /// refusal rather than a fallback to source text: a decorator built by a subscript or a
    /// conditional expression names nothing a reader could look up.
    fn dotted_callee(&self, node: Node) -> Option<String> {
        match node.kind() {
            "identifier" => Some(self.text(node)),
            "attribute" => {
                let object = node.child_by_field_name("object")?;
                let attribute = node.child_by_field_name("attribute")?;
                Some(format!(
                    "{}.{}",
                    self.dotted_callee(object)?,
                    self.text(attribute)
                ))
            }
            "call" => self.dotted_callee(node.child_by_field_name("function")?),
            _ => None,
        }
    }

    /// The decorators on a `decorated_definition`, as dotted names.
    ///
    /// A decorator is **structural metadata on the decorated symbol**, not a call edge.
    /// `@app.route("/x")` says something about the function it decorates; what a framework does
    /// with it is a framework rule, and framework rules are Slice 10.
    fn decorators(&mut self, node: Node) -> Vec<String> {
        let mut names = Vec::new();
        for child in named_children(node) {
            if child.kind() != "decorator" {
                continue;
            }
            let Some(expression) = named_children(child).into_iter().next() else {
                self.out.unsupported_decorators += 1;
                continue;
            };
            match self.dotted_callee(expression) {
                Some(name) => names.push(name),
                None => self.out.unsupported_decorators += 1,
            }
        }
        names
    }

    fn visit_decorated(&mut self, node: Node, class_entity_id: Option<&str>) -> Result<()> {
        let decorators = self.decorators(node);
        let span = span_of(node);
        let Some(definition) = node.child_by_field_name("definition") else {
            return self.visit_children(node);
        };
        match definition.kind() {
            "function_definition" => {
                self.visit_function(definition, span, &decorators, class_entity_id)
            }
            "class_definition" => self.visit_class(definition, span, &decorators),
            _ => self.visit(definition),
        }
    }

    fn definition_meta(&self, node: Node, decorators: &[String]) -> String {
        serde_json::json!({
            "async": has_child_kind(node, "async"),
            "decorators": decorators,
        })
        .to_string()
    }

    /// A `def`. A `Method` when it is a direct member of a class body, a `Function` otherwise.
    fn visit_function(
        &mut self,
        node: Node,
        span: Span,
        decorators: &[String],
        class_entity_id: Option<&str>,
    ) -> Result<()> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return self.visit_children_conditional(node, "function");
        };
        let name = self.text(name_node);
        let meta = self.definition_meta(node, decorators);
        let kind = if class_entity_id.is_some() {
            EntityKind::Method
        } else {
            EntityKind::Function
        };
        self.define(
            kind,
            &name,
            span,
            Some(meta),
            class_entity_id.map(str::to_string),
        )?;

        let token = self.token_for(&name);
        self.push_scope(token, ScopeKind::Function);
        self.conditionals.push("function");
        let result = self.visit_children(node);
        self.conditionals.pop();
        self.pop_scope();
        result
    }

    fn visit_class(&mut self, node: Node, span: Span, decorators: &[String]) -> Result<()> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return self.visit_children_conditional(node, "class");
        };
        let name = self.text(name_node);
        let meta = self.definition_meta(node, decorators);
        let index = self.define(EntityKind::Class, &name, span, Some(meta), None)?;
        let class_entity_id = self.out.symbols[index].entity_id.clone();

        let token = self.token_for(&name);
        self.push_scope(token, ScopeKind::Class);
        self.conditionals.push("class");
        let mut result = Ok(());
        for child in named_children(node) {
            if child.kind() != "block" {
                // Superclass list and type parameters. Visited so that a call inside them — a
                // dynamic import in a base-class expression, say — is still seen.
                result = self.visit(child);
            } else {
                for member in named_children(child) {
                    result = match member.kind() {
                        "function_definition" => self.visit_function(
                            member,
                            span_of(member),
                            &[],
                            Some(&class_entity_id),
                        ),
                        "decorated_definition" => {
                            self.visit_decorated(member, Some(&class_entity_id))
                        }
                        _ => self.visit(member),
                    };
                    if result.is_err() {
                        break;
                    }
                }
            }
            if result.is_err() {
                break;
            }
        }
        self.conditionals.pop();
        self.pop_scope();
        result
    }

    // ---- `__all__` -------------------------------------------------------------------------

    /// A module-top-level `__all__ = [...]` is the only statement of a public surface Python has.
    ///
    /// Anything else — a computed list, a `+=`, an `__all__` inside a conditional — is recorded
    /// as [`AllDeclaration::Unsupported`], because the names it publishes are not readable from
    /// the tree and a reader deserves to know that rather than to see an empty answer.
    fn visit_expression_statement(&mut self, node: Node) -> Result<()> {
        for child in named_children(node) {
            let is_all_target = matches!(child.kind(), "assignment" | "augmented_assignment")
                && child.child_by_field_name("left").is_some_and(|left| {
                    left.kind() == "identifier" && self.text(left) == "__all__"
                });
            if is_all_target && self.scopes.is_empty() && self.conditionals.is_empty() {
                let declaration = self.read_all_declaration(child);
                // The first `__all__` wins; a second statement is a rebinding whose result is
                // not readable, so it downgrades the answer rather than replacing it.
                self.out.all_declaration = Some(match self.out.all_declaration.take() {
                    None => declaration,
                    Some(_) => AllDeclaration::Unsupported,
                });
            }
        }
        self.visit_children(node)
    }

    fn read_all_declaration(&self, assignment: Node) -> AllDeclaration {
        if assignment.kind() != "assignment" {
            return AllDeclaration::Unsupported;
        }
        let Some(right) = assignment.child_by_field_name("right") else {
            return AllDeclaration::Unsupported;
        };
        if !matches!(right.kind(), "list" | "tuple") {
            return AllDeclaration::Unsupported;
        }
        let mut names = Vec::new();
        for element in named_children(right) {
            match self.string_value(element) {
                Some(value) => names.push(value),
                None => return AllDeclaration::Unsupported,
            }
        }
        AllDeclaration::Literal(names)
    }

    // ---- imports ---------------------------------------------------------------------------

    /// `import a.b`, `import a.b as c`, `import a, b`.
    fn visit_import(&mut self, node: Node) -> Result<()> {
        let span = span_of(node);
        let conditional = self.conditional();
        for child in named_children(node) {
            let (specifier_node, alias) = match child.kind() {
                "dotted_name" => (Some(child), None),
                "aliased_import" => (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("alias").map(|n| self.text(n)),
                ),
                _ => (None, None),
            };
            let Some(specifier_node) = specifier_node else {
                continue;
            };
            let raw_specifier = self.text(specifier_node);
            // `import a.b` binds `a`; `import a.b as c` binds `c`. Recording which is which is
            // what lets a later slice tell a shadowed alias from a package name.
            let local = alias.clone().or_else(|| {
                raw_specifier
                    .split('.')
                    .next()
                    .map(str::to_string)
                    .filter(|first| !first.is_empty())
            });
            self.out.imports.push(PyImportSite {
                raw_specifier: raw_specifier.clone(),
                form: PyImportForm::Import,
                bindings: vec![PyBinding {
                    imported: Some(raw_specifier),
                    local,
                }],
                conditional,
                dynamic_callee: None,
                literal_argument: None,
                span,
            });
        }
        Ok(())
    }

    /// `from a.b import c`, `from . import c`, `from a import *`.
    fn visit_import_from(&mut self, node: Node) -> Result<()> {
        let span = span_of(node);
        let conditional = self.conditional();
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return Ok(());
        };
        let raw_specifier = self.text(module_node);

        let mut bindings = Vec::new();
        let mut wildcard = false;
        for child in named_children(node) {
            if child.id() == module_node.id() {
                continue;
            }
            match child.kind() {
                "wildcard_import" => wildcard = true,
                "dotted_name" => {
                    let name = self.text(child);
                    bindings.push(PyBinding {
                        imported: Some(name.clone()),
                        local: Some(name),
                    });
                }
                "aliased_import" => {
                    let imported = child.child_by_field_name("name").map(|n| self.text(n));
                    let alias = child.child_by_field_name("alias").map(|n| self.text(n));
                    bindings.push(PyBinding {
                        local: alias.or_else(|| imported.clone()),
                        imported,
                    });
                }
                _ => {}
            }
        }
        if wildcard {
            bindings.push(PyBinding {
                imported: Some("*".to_string()),
                local: None,
            });
        }

        self.out.imports.push(PyImportSite {
            raw_specifier,
            form: if wildcard {
                PyImportForm::Wildcard
            } else {
                PyImportForm::FromImport
            },
            bindings,
            conditional,
            dynamic_callee: None,
            literal_argument: None,
            span,
        });
        Ok(())
    }

    fn first_string_argument<'tree>(&self, node: Node<'tree>) -> Option<Node<'tree>> {
        let arguments = node.child_by_field_name("arguments")?;
        named_children(arguments)
            .into_iter()
            .find(|argument| argument.kind() == "string")
    }

    fn visit_call(&mut self, node: Node) -> Result<()> {
        self.note_sys_path(node);
        if let Some(function) = node.child_by_field_name("function") {
            if let Some(callee) = self.dotted_callee(function) {
                if DYNAMIC_IMPORT_CALLEES.contains(&callee.as_str()) {
                    let literal_argument = self
                        .first_string_argument(node)
                        .and_then(|argument| self.string_value(argument));
                    self.out.imports.push(PyImportSite {
                        raw_specifier: String::new(),
                        form: PyImportForm::Dynamic,
                        bindings: Vec::new(),
                        conditional: self.conditional(),
                        dynamic_callee: Some(callee),
                        literal_argument,
                        span: span_of(node),
                    });
                }
            }
        }
        self.visit_children(node)
    }

    // ---- `sys.path` ------------------------------------------------------------------------

    fn is_sys_path(&self, node: Node) -> bool {
        node.kind() == "attribute"
            && node
                .child_by_field_name("object")
                .is_some_and(|object| object.kind() == "identifier" && self.text(object) == "sys")
            && node
                .child_by_field_name("attribute")
                .is_some_and(|attribute| self.text(attribute) == "path")
    }

    /// Record a mutation of `sys.path`, if this node is one.
    ///
    /// Two forms are recognised, and both are writes: a mutating method call on `sys.path`, and
    /// an assignment to `sys.path` or to an element of it. `from sys import path` followed by
    /// `path.append(...)` is **not** recognised, and is one of the reasons this signal is a
    /// lower bound rather than a guarantee.
    fn note_sys_path(&mut self, node: Node) {
        match node.kind() {
            "call" => {
                let Some(function) = node.child_by_field_name("function") else {
                    return;
                };
                if function.kind() != "attribute" {
                    return;
                }
                let (Some(object), Some(attribute)) = (
                    function.child_by_field_name("object"),
                    function.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                if self.is_sys_path(object)
                    && SYS_PATH_MUTATORS.contains(&self.text(attribute).as_str())
                {
                    self.out.sys_path_mutated = true;
                }
            }
            "assignment" | "augmented_assignment" => {
                let Some(left) = node.child_by_field_name("left") else {
                    return;
                };
                let target = if left.kind() == "subscript" {
                    left.child_by_field_name("value")
                } else {
                    Some(left)
                };
                if target.is_some_and(|target| self.is_sys_path(target)) {
                    self.out.sys_path_mutated = true;
                }
            }
            _ => {}
        }
    }
}

/// Extract one Python module.
///
/// `source` must be valid UTF-8; the caller is responsible for that check because it also
/// decides how a non-UTF-8 file is tallied.
pub fn extract_module(
    project_id: &str,
    rel_path: &str,
    source: &str,
) -> Result<PyModuleExtraction> {
    let mut parser = Language::Python.parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::error::IndexError::Parser(format!("parse failed: {rel_path}")))?;
    let root = tree.root_node();

    let mut extractor = Extractor {
        source: source.as_bytes(),
        project_id,
        rel_path,
        scopes: Vec::new(),
        conditionals: Vec::new(),
        disambiguators: HashMap::new(),
        out: PyModuleExtraction {
            rel_path: rel_path.to_string(),
            is_package: path_is_package_init(rel_path),
            has_syntax_error: root.has_error(),
            file_span: span_of(root),
            ..Default::default()
        },
    };
    extractor.visit_children(root)?;
    Ok(extractor.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PID: &str = "00000000000000000000000000000001";

    fn extract(source: &str) -> PyModuleExtraction {
        extract_module(PID, "pkg/mod.py", source).unwrap()
    }

    fn names(extraction: &PyModuleExtraction) -> Vec<(EntityKind, String, String)> {
        extraction
            .symbols
            .iter()
            .map(|s| (s.kind, s.scope_path.clone(), s.name.clone()))
            .collect()
    }

    fn meta_of(extraction: &PyModuleExtraction, index: usize) -> serde_json::Value {
        serde_json::from_str(extraction.symbols[index].meta.as_ref().unwrap()).unwrap()
    }

    #[test]
    fn a_module_level_def_is_a_function() {
        let extraction = extract("def add(a, b):\n    return a + b\n");
        assert_eq!(
            names(&extraction),
            vec![(EntityKind::Function, String::new(), "add".to_string())]
        );
        assert_eq!(extraction.symbols[0].span.start_line, 1);
        assert_eq!(extraction.symbols[0].span.start_byte, 0);
        assert!(!extraction.has_syntax_error);
    }

    #[test]
    fn a_nested_def_names_its_enclosing_function() {
        let extraction = extract("def outer():\n    def inner():\n        pass\n");
        assert_eq!(
            names(&extraction),
            vec![
                (EntityKind::Function, String::new(), "outer".to_string()),
                (
                    EntityKind::Function,
                    "outer".to_string(),
                    "inner".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_def_in_a_class_body_is_a_method_owned_by_the_class() {
        let extraction = extract(
            "class Engine:\n\
             \x20   def __init__(self):\n\
             \x20       pass\n\
             \x20   async def start(self):\n\
             \x20       pass\n",
        );
        assert_eq!(
            names(&extraction),
            vec![
                (EntityKind::Class, String::new(), "Engine".to_string()),
                (
                    EntityKind::Method,
                    "Engine".to_string(),
                    "__init__".to_string()
                ),
                (
                    EntityKind::Method,
                    "Engine".to_string(),
                    "start".to_string()
                ),
            ]
        );
        assert_eq!(
            extraction.symbols[1].owner_class.as_deref(),
            Some(extraction.symbols[0].entity_id.as_str()),
            "__init__ is an ordinary method, owned by its class like any other"
        );
        assert_eq!(meta_of(&extraction, 1)["async"], false);
        assert_eq!(meta_of(&extraction, 2)["async"], true);
    }

    #[test]
    fn a_def_inside_a_method_is_local_to_the_method() {
        let extraction =
            extract("class C:\n    def m(self):\n        def helper():\n            pass\n");
        assert_eq!(
            names(&extraction).last().unwrap(),
            &(
                EntityKind::Function,
                "C.m".to_string(),
                "helper".to_string()
            )
        );
    }

    #[test]
    fn decorators_are_metadata_on_the_decorated_symbol_and_never_an_edge() {
        let extraction = extract(
            "import functools\n\n\
             @functools.lru_cache(maxsize=None)\n\
             def tune(v):\n    return v\n\n\
             class C:\n\
             \x20   @staticmethod\n\
             \x20   def make():\n        pass\n\
             \x20   @property\n\
             \x20   def label(self):\n        return 1\n",
        );
        assert_eq!(
            meta_of(&extraction, 0)["decorators"],
            serde_json::json!(["functools.lru_cache"])
        );
        assert_eq!(
            meta_of(&extraction, 2)["decorators"],
            serde_json::json!(["staticmethod"])
        );
        assert_eq!(
            meta_of(&extraction, 3)["decorators"],
            serde_json::json!(["property"])
        );
        // A decorator is not a call. The only imports here are the literal `import functools`.
        assert_eq!(extraction.imports.len(), 1);
        assert_eq!(extraction.imports[0].raw_specifier, "functools");
    }

    /// The decorated span starts at the `@`, so the evidence range covers what a reader sees.
    #[test]
    fn a_decorated_definitions_span_includes_its_decorators() {
        let extraction = extract("@deco\ndef f():\n    pass\n");
        assert_eq!(extraction.symbols[0].span.start_line, 1);
        assert_eq!(extraction.symbols[0].span.start_byte, 0);
    }

    #[test]
    fn a_decorator_that_is_not_a_dotted_name_is_counted_not_invented() {
        let extraction = extract("@registry[0]\ndef f():\n    pass\n");
        assert_eq!(
            meta_of(&extraction, 0)["decorators"],
            serde_json::json!([]),
            "a subscript names nothing a reader could look up"
        );
        assert_eq!(extraction.unsupported_decorators, 1);
    }

    #[test]
    fn same_name_in_the_same_scope_increments_the_disambiguator() {
        let extraction = extract("def dup():\n    pass\ndef dup():\n    pass\n");
        assert_eq!(extraction.symbols[0].disambiguator, 0);
        assert_eq!(extraction.symbols[1].disambiguator, 1);
        assert_ne!(
            extraction.symbols[0].entity_id,
            extraction.symbols[1].entity_id
        );
    }

    #[test]
    fn import_forms_are_recorded_exactly_as_written() {
        let extraction = extract(
            "import os\n\
             import os.path as p\n\
             import a, b\n\
             from pkg.util import scale, tune as t\n\
             from . import sibling\n\
             from ..core import Engine\n",
        );
        let specifiers: Vec<(&str, &str)> = extraction
            .imports
            .iter()
            .map(|import| (import.raw_specifier.as_str(), import.form.as_str()))
            .collect();
        assert_eq!(
            specifiers,
            vec![
                ("os", "import"),
                ("os.path", "import"),
                ("a", "import"),
                ("b", "import"),
                ("pkg.util", "from-import"),
                (".", "from-import"),
                ("..core", "from-import"),
            ]
        );
        assert_eq!(
            extraction.imports[1].bindings[0].local.as_deref(),
            Some("p")
        );
        assert_eq!(
            extraction.imports[0].bindings[0].local.as_deref(),
            Some("os"),
            "`import a.b` binds the top-level package name"
        );
        assert_eq!(
            extraction.imports[4].bindings[0].imported.as_deref(),
            Some("scale")
        );
        assert_eq!(
            extraction.imports[4].bindings[1].local.as_deref(),
            Some("t")
        );
        assert!(extraction
            .imports
            .iter()
            .all(|import| import.conditional.is_none()));
    }

    #[test]
    fn a_wildcard_import_is_its_own_form() {
        let extraction = extract("from pkg.util import *\n");
        assert_eq!(extraction.imports[0].form, PyImportForm::Wildcard);
        assert_eq!(extraction.imports[0].raw_specifier, "pkg.util");
        assert_eq!(
            extraction.imports[0].bindings[0].imported.as_deref(),
            Some("*")
        );
    }

    #[test]
    fn an_import_that_is_not_at_module_top_level_is_conditional() {
        let extraction = extract(
            "try:\n\
             \x20   from a import x\n\
             except ImportError:\n\
             \x20   from b import x\n\
             if FLAG:\n\
             \x20   import c\n\
             def f():\n\
             \x20   import d\n\
             class C:\n\
             \x20   import e\n\
             for item in items:\n\
             \x20   import g\n",
        );
        let tags: Vec<(&str, Option<&'static str>)> = extraction
            .imports
            .iter()
            .map(|import| (import.raw_specifier.as_str(), import.conditional))
            .collect();
        assert_eq!(
            tags,
            vec![
                ("a", Some("try")),
                ("b", Some("try")),
                ("c", Some("if")),
                ("d", Some("function")),
                ("e", Some("class")),
                ("g", Some("for")),
            ],
            "the outermost enclosing construct is the one reported"
        );
    }

    #[test]
    fn a_dynamic_import_names_its_callee_and_never_a_module() {
        let extraction = extract(
            "import importlib\n\
             def load(name):\n\
             \x20   return importlib.import_module(name)\n\
             def builtin():\n\
             \x20   return __import__('json')\n",
        );
        let dynamic: Vec<(&str, Option<&str>)> = extraction
            .imports
            .iter()
            .filter(|import| import.form == PyImportForm::Dynamic)
            .map(|import| {
                (
                    import.dynamic_callee.as_deref().unwrap(),
                    import.literal_argument.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            dynamic,
            vec![
                ("importlib.import_module", None),
                ("__import__", Some("json")),
            ]
        );
        for import in &extraction.imports {
            if import.form == PyImportForm::Dynamic {
                assert!(
                    import.raw_specifier.is_empty(),
                    "a dynamic import names no specifier, and must not be given one"
                );
            }
        }
    }

    #[test]
    fn all_is_read_only_when_it_is_a_literal_sequence_of_strings() {
        let extraction = extract("__all__ = ['a', 'b']\n");
        assert_eq!(
            extraction.all_declaration,
            Some(AllDeclaration::Literal(vec![
                "a".to_string(),
                "b".to_string()
            ]))
        );
        assert_eq!(
            extraction.exported_names(),
            ["a".to_string(), "b".to_string()]
        );

        assert_eq!(
            extract("__all__ = ('a',)\n").all_declaration,
            Some(AllDeclaration::Literal(vec!["a".to_string()])),
            "a tuple is as literal as a list"
        );
        for source in [
            "__all__ = names\n",
            "__all__ = [f'{prefix}_a']\n",
            "__all__ = ['a'] + extra\n",
            "__all__ += ['a']\n",
            "__all__ = ['a']\n__all__ = ['b']\n",
        ] {
            assert_eq!(
                extract(source).all_declaration,
                Some(AllDeclaration::Unsupported),
                "{source:?} is not a readable public surface"
            );
        }
        assert_eq!(extract("x = 1\n").all_declaration, None);
        assert!(extract("x = 1\n").exported_names().is_empty());
    }

    /// An `__all__` inside a conditional is not the module's public surface; it is one branch's.
    #[test]
    fn an_all_that_is_not_at_module_top_level_is_ignored() {
        let extraction = extract("if FLAG:\n    __all__ = ['a']\n");
        assert_eq!(extraction.all_declaration, None);
    }

    #[test]
    fn sys_path_writes_are_detected_and_reads_are_not() {
        for source in [
            "import sys\nsys.path.append('x')\n",
            "import sys\nsys.path.insert(0, 'x')\n",
            "import sys\nsys.path = ['x']\n",
            "import sys\nsys.path += ['x']\n",
            "import sys\nsys.path[0] = 'x'\n",
            "import sys\ndef widen():\n    sys.path.extend(['x'])\n",
        ] {
            assert!(
                extract(source).sys_path_mutated,
                "{source:?} mutates sys.path"
            );
        }
        for source in [
            "import sys\nprint(sys.path)\n",
            "import sys\nfound = sys.path.index('x')\n",
            "import os\nos.path.append('x')\n",
        ] {
            assert!(
                !extract(source).sys_path_mutated,
                "{source:?} does not mutate sys.path"
            );
        }
    }

    #[test]
    fn package_init_modules_are_recognised_by_path() {
        assert!(path_is_package_init("__init__.py"));
        assert!(path_is_package_init("pkg/__init__.py"));
        assert!(path_is_package_init("pkg/sub/__init__.py"));
        assert!(!path_is_package_init("pkg/core.py"));
        assert!(!path_is_package_init("my__init__.py"));
        assert!(
            extract_module(PID, "pkg/__init__.py", "")
                .unwrap()
                .is_package
        );
        assert!(!extract_module(PID, "pkg/core.py", "").unwrap().is_package);
    }

    #[test]
    fn a_syntax_error_is_flagged_and_the_symbols_before_it_survive() {
        let extraction = extract("def before():\n    return 1\n\ndef broken(:\n    return 2\n");
        assert!(extraction.has_syntax_error);
        assert!(
            extraction.symbols.iter().any(|s| s.name == "before"),
            "a partial parse must still yield what it could read: {:?}",
            names(&extraction)
        );
    }

    #[test]
    fn extraction_is_deterministic() {
        let source = "@deco\ndef a():\n    def b():\n        pass\n__all__ = ['a']\n";
        assert_eq!(extract(source), extract(source));
    }
}
