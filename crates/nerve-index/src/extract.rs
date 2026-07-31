//! The `ts-js-structural` extractor.
//!
//! Reads one module's syntax tree and reports what the tree literally says: which symbols are
//! declared, which specifiers are imported, and which names are exported. It performs no
//! resolution and emits no assertions — resolution and assertion construction happen in
//! [`crate::pipeline`], which is also where the evidence profile is attached.
//!
//! Slice 1 deliberately does **not** extract `CALLS`, `REFERENCES`, `EXTENDS` or `IMPLEMENTS`.
//! Those relations exist in the vocabulary and are emitted by nothing.

use std::collections::HashMap;

use tree_sitter::Node;

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::{EntityKind, EvidenceSourceType};

use crate::error::Result;
use crate::lang::Language;

/// Identifier of the only extractor in Slice 1.
pub const EXTRACTOR_ID: &str = "ts-js-structural";

/// Version of the extractor. Bump on any change to what it emits.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The evidence source types this extractor is permitted to emit (ADR-0003).
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::AstDirect];

/// Where a symbol sits lexically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    /// A function or method body.
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
pub struct SymbolDef {
    /// Logical identifier.
    pub entity_id: String,
    /// Entity kind.
    pub kind: EntityKind,
    /// Declared name.
    pub name: String,
    /// Enclosing lexical scope, joined with `.`.
    pub scope_path: String,
    /// Index among identically-named siblings, in source order.
    pub disambiguator: u32,
    /// Source range of the declaration.
    pub span: Span,
    /// Canonical JSON metadata.
    pub meta: Option<String>,
    /// Owning class, for methods.
    pub owner_class: Option<String>,
}

impl SymbolDef {
    /// True when the symbol is declared at module top level.
    pub fn is_top_level(&self) -> bool {
        self.scope_path.is_empty()
    }
}

/// The syntactic form an import took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportForm {
    /// `import ... from '...'` or `import '...'`.
    Static,
    /// `export ... from '...'` — a re-export also creates a module dependency.
    ReExport,
    /// `require('...')`.
    Require,
    /// `import('...')` with a string-literal argument.
    DynamicLiteral,
}

impl ImportForm {
    /// Tag recorded in observation details.
    pub fn as_str(self) -> &'static str {
        match self {
            ImportForm::Static => "static",
            ImportForm::ReExport => "re-export",
            ImportForm::Require => "require",
            ImportForm::DynamicLiteral => "dynamic-literal",
        }
    }
}

/// One binding introduced by an import statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecifierDetail {
    /// `default`, `named`, `namespace`, `side-effect`, or `re-export`.
    pub kind: &'static str,
    /// Name in the exporting module.
    pub imported: Option<String>,
    /// Name bound locally.
    pub local: Option<String>,
}

/// An import site: a literal specifier plus where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSite {
    /// The specifier exactly as written.
    pub raw_specifier: String,
    /// Syntactic form.
    pub form: ImportForm,
    /// `import type ...`.
    pub type_only: bool,
    /// Bindings introduced.
    pub specifiers: Vec<SpecifierDetail>,
    /// Source range of the statement.
    pub span: Span,
}

/// What an export names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    /// A symbol declared by this very export statement.
    Symbol(usize),
    /// A module-scope name to resolve within this module.
    LocalName(String),
}

/// An export of something defined in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalExport {
    /// Name the module exports it under.
    pub exported_name: String,
    /// What is exported.
    pub target: ExportTarget,
    /// Source range of the export statement.
    pub span: Span,
}

/// `export { x } from './y'` or `export * from './y'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReExport {
    /// Specifier exactly as written.
    pub raw_specifier: String,
    /// Named re-exports as `(name, alias)`; `None` means `export *`.
    pub names: Option<Vec<(String, Option<String>)>>,
    /// Source range of the export statement.
    pub span: Span,
}

/// Everything one module's syntax tree literally states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Symbols declared, in source order.
    pub symbols: Vec<SymbolDef>,
    /// Import sites, in source order.
    pub imports: Vec<ImportSite>,
    /// Exports of locally defined symbols.
    pub local_exports: Vec<LocalExport>,
    /// Re-exports from another module.
    pub re_exports: Vec<ReExport>,
    /// `import(expr)` calls whose argument was not a string literal.
    pub dynamic_imports_without_specifier: usize,
    /// Whether the parse produced any ERROR node.
    pub has_syntax_error: bool,
    /// Span covering the whole file.
    pub file_span: Span,
}

impl ModuleExtraction {
    /// Index of the first module-scope symbol with this name, if any.
    pub fn top_level_symbol(&self, name: &str) -> Option<usize> {
        self.symbols
            .iter()
            .position(|symbol| symbol.is_top_level() && symbol.name == name)
    }
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

struct Extractor<'a> {
    source: &'a [u8],
    project_id: &'a str,
    rel_path: &'a str,
    scopes: Vec<ScopeFrame>,
    anonymous_counter: u32,
    disambiguators: HashMap<(EntityKind, String, String), u32>,
    out: ModuleExtraction,
}

impl<'a> Extractor<'a> {
    fn text(&self, node: Node) -> String {
        std::str::from_utf8(&self.source[node.byte_range()])
            .unwrap_or_default()
            .to_string()
    }

    /// Value of a `string` node, without its quotes.
    fn string_value(&self, node: Node) -> Option<String> {
        if node.kind() != "string" {
            return None;
        }
        for child in named_children(node) {
            if child.kind() == "string_fragment" {
                return Some(self.text(child));
            }
        }
        // An empty string literal has no fragment child.
        Some(String::new())
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
    /// token is wrapped: `outer.<local:inner>`. Class members and module-level declarations
    /// use their bare name.
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

        self.out.symbols.push(SymbolDef {
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

    fn visit(&mut self, node: Node) -> Result<()> {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                self.visit_function_declaration(node)
            }
            "class_declaration" | "abstract_class_declaration" => self.visit_class(node),
            "interface_declaration" => self.visit_interface(node),
            "lexical_declaration" | "variable_declaration" => self.visit_variable_declaration(node),
            "arrow_function" | "function_expression" | "generator_function" => {
                self.visit_anonymous_function(node)
            }
            "import_statement" => self.visit_import(node),
            "export_statement" => self.visit_export(node),
            "call_expression" => self.visit_call(node),
            _ => self.visit_children(node),
        }
    }

    fn function_meta(&self, node: Node, form: &str) -> Option<String> {
        let generator = node.kind().starts_with("generator_") || has_child_kind(node, "*");
        let value = serde_json::json!({
            "async": has_child_kind(node, "async"),
            "form": form,
            "generator": generator,
        });
        Some(value.to_string())
    }

    fn visit_function_declaration(&mut self, node: Node) -> Result<()> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n))
            // `export default function () {}` declares a function whose exported name is
            // `default`; naming it so is what the module system does.
            .unwrap_or_else(|| "default".to_string());
        let meta = self.function_meta(node, "declaration");
        self.define(EntityKind::Function, &name, span_of(node), meta, None)?;

        let token = self.token_for(&name);
        self.push_scope(token, ScopeKind::Function);
        let result = self.visit_children(node);
        self.pop_scope();
        result
    }

    fn visit_class(&mut self, node: Node) -> Result<()> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n))
            .unwrap_or_else(|| "default".to_string());
        let meta = serde_json::json!({
            "abstract": node.kind() == "abstract_class_declaration",
        })
        .to_string();
        let index = self.define(EntityKind::Class, &name, span_of(node), Some(meta), None)?;
        let class_entity_id = self.out.symbols[index].entity_id.clone();

        let token = self.token_for(&name);
        self.push_scope(token, ScopeKind::Class);
        let mut result = Ok(());
        for child in named_children(node) {
            if child.kind() == "class_body" {
                for member in named_children(child) {
                    result = if member.kind() == "method_definition" {
                        self.visit_method(member, &class_entity_id)
                    } else {
                        self.visit(member)
                    };
                    if result.is_err() {
                        break;
                    }
                }
            } else {
                result = self.visit(child);
            }
            if result.is_err() {
                break;
            }
        }
        self.pop_scope();
        result
    }

    fn visit_method(&mut self, node: Node, class_entity_id: &str) -> Result<()> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return self.visit_children(node);
        };
        let name = self.text(name_node);
        let accessor = if has_child_kind(node, "get") {
            Some("get")
        } else if has_child_kind(node, "set") {
            Some("set")
        } else {
            None
        };
        let meta = serde_json::json!({
            "accessor": accessor,
            "async": has_child_kind(node, "async"),
            "generator": has_child_kind(node, "*"),
            "static": has_child_kind(node, "static"),
        })
        .to_string();
        self.define(
            EntityKind::Method,
            &name,
            span_of(node),
            Some(meta),
            Some(class_entity_id.to_string()),
        )?;

        let token = self.token_for(&name);
        self.push_scope(token, ScopeKind::Function);
        let result = self.visit_children(node);
        self.pop_scope();
        result
    }

    fn visit_interface(&mut self, node: Node) -> Result<()> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let name = self.text(name_node);
        self.define(EntityKind::Interface, &name, span_of(node), None, None)?;
        // Interface members are not entities in Slice 1.
        Ok(())
    }

    fn visit_variable_declaration(&mut self, node: Node) -> Result<()> {
        for declarator in named_children(node) {
            if declarator.kind() != "variable_declarator" {
                self.visit(declarator)?;
                continue;
            }
            let name_node = declarator.child_by_field_name("name");
            let value = declarator.child_by_field_name("value");
            let bound_function = matches!(
                value.map(|v| v.kind()),
                Some("arrow_function") | Some("function_expression") | Some("generator_function")
            );
            let is_plain_identifier = name_node.map(|n| n.kind()) == Some("identifier");

            if let (true, true, Some(name_node), Some(value)) =
                (bound_function, is_plain_identifier, name_node, value)
            {
                let name = self.text(name_node);
                let form = if value.kind() == "arrow_function" {
                    "arrow"
                } else {
                    "expression"
                };
                let meta = self.function_meta(value, form);
                // The span covers the binding, not only the function literal, so the evidence
                // range includes the name a reader would search for.
                self.define(EntityKind::Function, &name, span_of(declarator), meta, None)?;
                let token = self.token_for(&name);
                self.push_scope(token, ScopeKind::Function);
                let result = self.visit_children(value);
                self.pop_scope();
                result?;
            } else {
                self.visit_children(declarator)?;
            }
        }
        Ok(())
    }

    fn visit_anonymous_function(&mut self, node: Node) -> Result<()> {
        // No entity: Slice 1 names only declarator-bound functions. The scope token still has
        // to be allocated so that anything declared inside gets a stable scope path.
        let index = self.anonymous_counter;
        self.anonymous_counter += 1;
        let token = if node.kind() == "arrow_function" {
            format!("<anon:arrow@{index}>")
        } else {
            format!("<anon:fn@{index}>")
        };
        self.push_scope(token, ScopeKind::Function);
        let result = self.visit_children(node);
        self.pop_scope();
        result
    }

    fn parse_import_clause(&self, clause: Node) -> Vec<SpecifierDetail> {
        let mut specifiers = Vec::new();
        for child in named_children(clause) {
            match child.kind() {
                "identifier" => specifiers.push(SpecifierDetail {
                    kind: "default",
                    imported: Some("default".to_string()),
                    local: Some(self.text(child)),
                }),
                "namespace_import" => {
                    let local = named_children(child).first().map(|n| self.text(*n));
                    specifiers.push(SpecifierDetail {
                        kind: "namespace",
                        imported: Some("*".to_string()),
                        local,
                    });
                }
                "named_imports" => {
                    for specifier in named_children(child) {
                        if specifier.kind() != "import_specifier" {
                            continue;
                        }
                        let name = specifier.child_by_field_name("name").map(|n| self.text(n));
                        let alias = specifier.child_by_field_name("alias").map(|n| self.text(n));
                        specifiers.push(SpecifierDetail {
                            kind: "named",
                            local: alias.clone().or_else(|| name.clone()),
                            imported: name,
                        });
                    }
                }
                _ => {}
            }
        }
        specifiers
    }

    fn visit_import(&mut self, node: Node) -> Result<()> {
        let Some(source) = node.child_by_field_name("source") else {
            return self.visit_children(node);
        };
        let Some(raw_specifier) = self.string_value(source) else {
            return self.visit_children(node);
        };

        let mut specifiers = Vec::new();
        for child in named_children(node) {
            if child.kind() == "import_clause" {
                specifiers.extend(self.parse_import_clause(child));
            }
        }
        if specifiers.is_empty() {
            specifiers.push(SpecifierDetail {
                kind: "side-effect",
                imported: None,
                local: None,
            });
        }

        self.out.imports.push(ImportSite {
            raw_specifier,
            form: ImportForm::Static,
            type_only: has_child_kind(node, "type"),
            specifiers,
            span: span_of(node),
        });
        Ok(())
    }

    fn parse_export_clause(&self, clause: Node) -> Vec<(String, Option<String>)> {
        let mut names = Vec::new();
        for specifier in named_children(clause) {
            if specifier.kind() != "export_specifier" {
                continue;
            }
            let Some(name) = specifier.child_by_field_name("name").map(|n| self.text(n)) else {
                continue;
            };
            let alias = specifier.child_by_field_name("alias").map(|n| self.text(n));
            names.push((name, alias));
        }
        names
    }

    fn visit_export(&mut self, node: Node) -> Result<()> {
        let span = span_of(node);

        // `export ... from '...'` — a re-export, which is also a module dependency.
        if let Some(source) = node.child_by_field_name("source") {
            let Some(raw_specifier) = self.string_value(source) else {
                return self.visit_children(node);
            };
            let clause = named_children(node)
                .into_iter()
                .find(|child| child.kind() == "export_clause");
            let names = clause.map(|clause| self.parse_export_clause(clause));
            let is_star = names.is_none() && has_child_kind(node, "*");

            let specifiers = match &names {
                Some(list) => list
                    .iter()
                    .map(|(name, alias)| SpecifierDetail {
                        kind: "re-export",
                        imported: Some(name.clone()),
                        local: Some(alias.clone().unwrap_or_else(|| name.clone())),
                    })
                    .collect(),
                None => vec![SpecifierDetail {
                    kind: "re-export",
                    imported: Some("*".to_string()),
                    local: None,
                }],
            };

            self.out.imports.push(ImportSite {
                raw_specifier: raw_specifier.clone(),
                form: ImportForm::ReExport,
                type_only: has_child_kind(node, "type"),
                specifiers,
                span,
            });
            if is_star || names.is_some() {
                self.out.re_exports.push(ReExport {
                    raw_specifier,
                    names,
                    span,
                });
            }
            return Ok(());
        }

        // `export <declaration>`
        if let Some(declaration) = node.child_by_field_name("declaration") {
            let before = self.out.symbols.len();
            self.visit(declaration)?;
            for index in before..self.out.symbols.len() {
                if self.out.symbols[index].is_top_level() {
                    self.out.local_exports.push(LocalExport {
                        exported_name: self.out.symbols[index].name.clone(),
                        target: ExportTarget::Symbol(index),
                        span,
                    });
                }
            }
            return Ok(());
        }

        // `export default ...`
        if has_child_kind(node, "default") {
            let value = node
                .child_by_field_name("value")
                .or_else(|| named_children(node).into_iter().next());
            if let Some(value) = value {
                if value.kind() == "identifier" {
                    self.out.local_exports.push(LocalExport {
                        exported_name: "default".to_string(),
                        target: ExportTarget::LocalName(self.text(value)),
                        span,
                    });
                } else {
                    let before = self.out.symbols.len();
                    self.visit(value)?;
                    for index in before..self.out.symbols.len() {
                        if self.out.symbols[index].is_top_level() {
                            self.out.local_exports.push(LocalExport {
                                exported_name: "default".to_string(),
                                target: ExportTarget::Symbol(index),
                                span,
                            });
                        }
                    }
                }
            }
            return Ok(());
        }

        // `export { a, b as c }`
        if let Some(clause) = named_children(node)
            .into_iter()
            .find(|child| child.kind() == "export_clause")
        {
            for (name, alias) in self.parse_export_clause(clause) {
                self.out.local_exports.push(LocalExport {
                    exported_name: alias.unwrap_or_else(|| name.clone()),
                    target: ExportTarget::LocalName(name),
                    span,
                });
            }
            return Ok(());
        }

        self.visit_children(node)
    }

    fn first_string_argument<'tree>(&self, node: Node<'tree>) -> Option<Node<'tree>> {
        let arguments = node.child_by_field_name("arguments")?;
        named_children(arguments)
            .into_iter()
            .find(|argument| argument.kind() == "string")
    }

    fn visit_call(&mut self, node: Node) -> Result<()> {
        if let Some(function) = node.child_by_field_name("function") {
            if function.kind() == "identifier" && self.text(function) == "require" {
                if let Some(argument) = self.first_string_argument(node) {
                    if let Some(raw_specifier) = self.string_value(argument) {
                        self.out.imports.push(ImportSite {
                            raw_specifier,
                            form: ImportForm::Require,
                            type_only: false,
                            specifiers: vec![SpecifierDetail {
                                kind: "require",
                                imported: None,
                                local: None,
                            }],
                            span: span_of(node),
                        });
                    }
                }
            } else if function.kind() == "import" {
                match self.first_string_argument(node) {
                    Some(argument) => {
                        if let Some(raw_specifier) = self.string_value(argument) {
                            self.out.imports.push(ImportSite {
                                raw_specifier,
                                form: ImportForm::DynamicLiteral,
                                type_only: false,
                                specifiers: vec![SpecifierDetail {
                                    kind: "dynamic",
                                    imported: None,
                                    local: None,
                                }],
                                span: span_of(node),
                            });
                        }
                    }
                    None => {
                        // `import(expr)` states no specifier. Recording an `Unresolved` entity
                        // keyed on the expression text would be inventing a specifier that the
                        // source never wrote, so this is counted and left as no edge.
                        self.out.dynamic_imports_without_specifier += 1;
                    }
                }
            }
        }
        self.visit_children(node)
    }
}

/// Extract one module.
///
/// `source` must be valid UTF-8; the caller is responsible for that check because it also
/// decides how a non-UTF-8 file is tallied.
pub fn extract_module(
    project_id: &str,
    rel_path: &str,
    language: Language,
    source: &str,
) -> Result<ModuleExtraction> {
    let mut parser = language.parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::error::IndexError::Parser(format!("parse failed: {rel_path}")))?;
    let root = tree.root_node();

    let mut extractor = Extractor {
        source: source.as_bytes(),
        project_id,
        rel_path,
        scopes: Vec::new(),
        anonymous_counter: 0,
        disambiguators: HashMap::new(),
        out: ModuleExtraction {
            rel_path: rel_path.to_string(),
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

    fn extract(source: &str) -> ModuleExtraction {
        extract_module(PID, "src/a.ts", Language::TypeScript, source).unwrap()
    }

    fn names(extraction: &ModuleExtraction) -> Vec<(EntityKind, String, String)> {
        extraction
            .symbols
            .iter()
            .map(|s| (s.kind, s.scope_path.clone(), s.name.clone()))
            .collect()
    }

    #[test]
    fn function_declaration() {
        let extraction = extract("function add(a: number) { return a; }\n");
        assert_eq!(
            names(&extraction),
            vec![(EntityKind::Function, String::new(), "add".to_string())]
        );
        assert_eq!(extraction.symbols[0].span.start_line, 1);
        assert_eq!(extraction.symbols[0].span.start_byte, 0);
    }

    #[test]
    fn nested_named_function_uses_local_marker() {
        let extraction = extract("function outer() { function inner() {} }\n");
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
    fn declarator_bound_arrow_and_function_expression_are_named() {
        let extraction =
            extract("const mul = (a, b) => a * b;\nconst anon = function (x) { return x; };\n");
        assert_eq!(
            names(&extraction),
            vec![
                (EntityKind::Function, String::new(), "mul".to_string()),
                (EntityKind::Function, String::new(), "anon".to_string()),
            ]
        );
        let meta: serde_json::Value =
            serde_json::from_str(extraction.symbols[0].meta.as_ref().unwrap()).unwrap();
        assert_eq!(meta["form"], "arrow");
    }

    #[test]
    fn unbound_anonymous_functions_get_scope_tokens_but_no_entity() {
        let extraction = extract("[1].map(x => { function inner() {} });\n");
        assert_eq!(
            names(&extraction),
            vec![(
                EntityKind::Function,
                "<anon:arrow@0>".to_string(),
                "inner".to_string()
            )]
        );
    }

    #[test]
    fn class_with_methods() {
        let extraction = extract(
            "class Circle { constructor(r) { this.r = r; } area() { return 1; } static make() {} get radius() { return 1; } }\n",
        );
        assert_eq!(
            names(&extraction),
            vec![
                (EntityKind::Class, String::new(), "Circle".to_string()),
                (
                    EntityKind::Method,
                    "Circle".to_string(),
                    "constructor".to_string()
                ),
                (EntityKind::Method, "Circle".to_string(), "area".to_string()),
                (EntityKind::Method, "Circle".to_string(), "make".to_string()),
                (
                    EntityKind::Method,
                    "Circle".to_string(),
                    "radius".to_string()
                ),
            ]
        );
        let make: serde_json::Value =
            serde_json::from_str(extraction.symbols[3].meta.as_ref().unwrap()).unwrap();
        assert_eq!(make["static"], true);
        let radius: serde_json::Value =
            serde_json::from_str(extraction.symbols[4].meta.as_ref().unwrap()).unwrap();
        assert_eq!(radius["accessor"], "get");
        assert_eq!(
            extraction.symbols[2].owner_class.as_deref(),
            Some(extraction.symbols[0].entity_id.as_str())
        );
    }

    #[test]
    fn function_inside_a_method_is_local_to_the_method() {
        let extraction = extract("class C { m() { function helper() {} } }\n");
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
    fn interface_declaration_without_members() {
        let extraction = extract("interface Shape { area(): number; }\n");
        assert_eq!(
            names(&extraction),
            vec![(EntityKind::Interface, String::new(), "Shape".to_string())]
        );
    }

    #[test]
    fn same_name_in_different_scopes_keeps_disambiguator_zero() {
        let extraction =
            extract("function a() { function dup() {} }\nfunction b() { function dup() {} }\n");
        let dups: Vec<&SymbolDef> = extraction
            .symbols
            .iter()
            .filter(|s| s.name == "dup")
            .collect();
        assert_eq!(dups.len(), 2);
        assert_eq!(dups[0].disambiguator, 0);
        assert_eq!(dups[1].disambiguator, 0);
        assert_ne!(dups[0].entity_id, dups[1].entity_id);
    }

    #[test]
    fn same_name_in_the_same_scope_increments_the_disambiguator() {
        let extraction = extract("function dup() {}\nfunction dup() {}\n");
        assert_eq!(extraction.symbols[0].disambiguator, 0);
        assert_eq!(extraction.symbols[1].disambiguator, 1);
        assert_ne!(
            extraction.symbols[0].entity_id,
            extraction.symbols[1].entity_id
        );
    }

    #[test]
    fn import_forms() {
        let extraction = extract(
            "import def, { a as b, c } from './math';\n\
             import * as ns from './shapes';\n\
             import type { T } from './types';\n\
             import './side-effect';\n",
        );
        let specifiers: Vec<&str> = extraction.imports[0]
            .specifiers
            .iter()
            .map(|s| s.kind)
            .collect();
        assert_eq!(specifiers, vec!["default", "named", "named"]);
        assert_eq!(
            extraction.imports[0].specifiers[1].imported.as_deref(),
            Some("a")
        );
        assert_eq!(
            extraction.imports[0].specifiers[1].local.as_deref(),
            Some("b")
        );
        assert_eq!(extraction.imports[1].specifiers[0].kind, "namespace");
        assert!(extraction.imports[2].type_only);
        assert_eq!(extraction.imports[3].specifiers[0].kind, "side-effect");
        assert_eq!(
            extraction
                .imports
                .iter()
                .map(|i| i.raw_specifier.as_str())
                .collect::<Vec<_>>(),
            vec!["./math", "./shapes", "./types", "./side-effect"]
        );
    }

    #[test]
    fn export_forms() {
        let extraction = extract(
            "export function add() {}\n\
             const mul = () => 1;\n\
             export { mul as times };\n\
             export default add;\n\
             export { x } from './math';\n\
             export * from './shapes';\n",
        );
        let exports: Vec<(&str, &ExportTarget)> = extraction
            .local_exports
            .iter()
            .map(|e| (e.exported_name.as_str(), &e.target))
            .collect();
        assert_eq!(exports[0].0, "add");
        assert!(matches!(exports[0].1, ExportTarget::Symbol(_)));
        assert_eq!(exports[1].0, "times");
        assert_eq!(exports[1].1, &ExportTarget::LocalName("mul".to_string()));
        assert_eq!(exports[2].0, "default");
        assert_eq!(exports[2].1, &ExportTarget::LocalName("add".to_string()));

        assert_eq!(extraction.re_exports.len(), 2);
        assert_eq!(
            extraction.re_exports[0].names,
            Some(vec![("x".to_string(), None)])
        );
        assert_eq!(extraction.re_exports[1].names, None, "export * is a star");
        // Both re-exports also register a module dependency.
        assert_eq!(extraction.imports.len(), 2);
        assert!(extraction
            .imports
            .iter()
            .all(|i| i.form == ImportForm::ReExport));
    }

    #[test]
    fn require_with_a_string_literal_is_an_import() {
        let extraction = extract_module(
            PID,
            "src/legacy.cjs",
            Language::JavaScript,
            "const m = require('./math');\n",
        )
        .unwrap();
        assert_eq!(extraction.imports.len(), 1);
        assert_eq!(extraction.imports[0].form, ImportForm::Require);
        assert_eq!(extraction.imports[0].raw_specifier, "./math");
    }

    #[test]
    fn dynamic_import_with_a_literal_is_an_import() {
        let extraction = extract("const p = import('./math');\n");
        assert_eq!(extraction.imports.len(), 1);
        assert_eq!(extraction.imports[0].form, ImportForm::DynamicLiteral);
        assert_eq!(extraction.dynamic_imports_without_specifier, 0);
    }

    #[test]
    fn dynamic_import_without_a_literal_produces_no_import() {
        let extraction = extract("const name = './x';\nconst p = import(name);\n");
        assert!(extraction.imports.is_empty());
        assert_eq!(extraction.dynamic_imports_without_specifier, 1);
    }

    #[test]
    fn tsx_components_and_jsx_parse() {
        let extraction = extract_module(
            PID,
            "src/widget.tsx",
            Language::Tsx,
            "export function Widget() { return <div className=\"w\">hi</div>; }\n",
        )
        .unwrap();
        assert!(!extraction.has_syntax_error);
        assert_eq!(
            names(&extraction),
            vec![(EntityKind::Function, String::new(), "Widget".to_string())]
        );
    }

    #[test]
    fn javascript_files_have_no_interfaces() {
        let extraction =
            extract_module(PID, "a.js", Language::JavaScript, "class C { m() {} }\n").unwrap();
        assert_eq!(extraction.symbols[0].kind, EntityKind::Class);
        assert_eq!(extraction.symbols[1].kind, EntityKind::Method);
    }

    #[test]
    fn syntax_errors_are_flagged_not_fatal() {
        let extraction = extract("function broken( {\n");
        assert!(extraction.has_syntax_error);
    }

    #[test]
    fn extraction_is_deterministic() {
        let source = "export function a() { const b = () => 1; }\nexport default a;\n";
        assert_eq!(extract(source), extract(source));
    }
}
