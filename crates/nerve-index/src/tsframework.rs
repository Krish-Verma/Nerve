//! `ts-js-framework` — HTTP routes Express declares, read from the source alone.
//!
//! The TypeScript/JavaScript counterpart of [`crate::pyframework`], and deliberately a **separate
//! extractor with its own id and version** rather than a branch inside it: an observation carries
//! the id of whatever produced it, so a TypeScript fact stamped `py-framework` would be a false
//! statement about where the evidence came from. Slice 5d-i was a corrective slice for exactly that.
//!
//! # The same claim, and the same refusals
//!
//! One thing is claimed: **the source declares an endpoint at this address, and names this symbol as
//! its handler.** `Endpoint SERVED_BY Function|Method`, `FRAMEWORK_RULE` / `DIRECT`. Not that the
//! route is reachable, that middleware permits it, or that configuration has not replaced it.
//!
//! # Why the call's spelling is not enough
//!
//! Express registers with an ordinary method call, so there is no decorator to key on and `.get` is
//! about as common a method name as exists. `fixtures/ts-framework/negative.ts` holds a cache, a
//! local class named `Router`, an object literal and a `Map`, all called with a string first
//! argument. A rule matching the spelling would emit five false positives.
//!
//! So the receiver is traced, through two ordinary binding lookups:
//!
//! 1. the receiver must be bound **at module scope in this file** to a call whose callee resolves to
//!    an Express constructor — `express()`, `express.Router()`, or a `Router` imported from
//!    `express`;
//! 2. that callee must itself be bound by an import from `express`.
//!
//! Default, named and aliased imports all fall out of this, because the lookup follows the binding
//! rather than comparing text.
//!
//! # Two refusals worth naming
//!
//! **`app.all("/x", handler)` is not a route.** `all` matches every HTTP method, and expanding it
//! into eight endpoints would assert eight declarations where the source made one. Recording it as a
//! ninth pseudo-method would put a value in `meta.method` that is not an HTTP method.
//!
//! **A one-argument call is not a registration.** `registry.get("/x")` is a lookup. A registration
//! binds a handler, so the second argument is required.
//!
//! # Executes nothing
//!
//! No module is imported, no callback invoked, no `package.json` script run.
//! `crates/nerve-cli/tests/no_subprocess.rs` indexes a repository whose `package.json` scripts and
//! module top level would write a marker if anything ever ran them.

use std::collections::{BTreeMap, BTreeSet};

use nerve_core::vocab::{Directness, EndpointKind, EntityKind, EvidenceSourceType, Relation};
use nerve_core::{ids, Span};
use tree_sitter::Node;

use crate::error::Result;
use crate::extract::{ModuleExtraction, SymbolDef};
use crate::lang::Language;
use crate::pyframework::UnsupportedForm;

/// Stable extractor id.
pub const EXTRACTOR_ID: &str = "ts-js-framework";

/// Extractor version. Bumping this invalidates `module_facts.framework_version` for every TS/JS
/// file, which is what makes a rule change take effect on an existing index.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The only evidence source type this extractor may emit.
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::FrameworkRule];

/// The only relation this extractor may emit.
pub const DECLARED_RELATIONS: [Relation; 1] = [Relation::ServedBy];

/// The framework tag recorded on every endpoint this extractor produces.
pub const FRAMEWORK: &str = "express";

/// The rule id recorded on every endpoint, so an observation names the rule that produced it.
pub const RULE_ID: &str = "express-route-registration";

/// The import package whose constructors this rule trusts.
const PACKAGE: &str = "express";

/// Names that produce a routable application object when called.
///
/// `express` itself is the default export and is callable; `Router` is a named export. Both are
/// reached by binding, never by matching the text.
const CONSTRUCTORS: [&str; 2] = ["express", "Router"];

/// HTTP methods a registration call may name.
///
/// `all` is deliberately absent — see the module documentation.
const METHOD_CALLS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace",
];

/// One declared endpoint and the symbol that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsEndpoint {
    /// Entity id from [`ids::endpoint_id`].
    pub entity_id: String,
    /// Canonical address, e.g. `GET /users/:id`. This is the entity name, so FTS5 indexes it.
    pub address: String,
    /// HTTP method, upper case.
    pub method: String,
    /// Path **as declared**. No mount-prefix composition.
    pub path: String,
    /// Entity id of the handler symbol.
    pub handler_entity_id: String,
    /// Source range of the registration call.
    pub span: Span,
}

/// What one module's framework rules found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsFrameworkExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Endpoints declared here, in source order.
    pub endpoints: Vec<TsEndpoint>,
    /// Constructs read and declined, counted by form.
    ///
    /// The **same** closed vocabulary `py-framework` uses ([`UnsupportedForm`]), so a reader
    /// comparing two languages compares the same tags. `methods-not-literal` is Flask-specific and
    /// simply never fires here.
    pub unsupported_by_form: BTreeMap<&'static str, usize>,
    /// Addresses declared more than once in this module, with their declaration count.
    pub ambiguous_addresses: BTreeMap<String, usize>,
}

impl TsFrameworkExtraction {
    fn count(&mut self, form: UnsupportedForm) {
        *self.unsupported_by_form.entry(form.as_str()).or_insert(0) += 1;
    }
}

/// Read the HTTP routes one TypeScript or JavaScript module declares.
pub fn extract_framework(
    project_id: &str,
    rel_path: &str,
    source: &str,
    language: Language,
    extraction: &ModuleExtraction,
) -> Result<TsFrameworkExtraction> {
    let mut parser = language.parser()?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| crate::error::IndexError::Parser(format!("parse failed: {rel_path}")))?;
    let root = tree.root_node();
    let bytes = source.as_bytes();

    let mut out = TsFrameworkExtraction {
        rel_path: rel_path.to_string(),
        ..TsFrameworkExtraction::default()
    };

    let constructors = framework_constructors(extraction);
    let apps = application_objects(root, bytes, &constructors);
    let imported = imported_locals(extraction);

    let mut walker = Walker {
        source: bytes,
        symbols: &extraction.symbols,
        apps: &apps,
        imported: &imported,
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

/// Local names bound, by import from `express`, to something that constructs an application.
///
/// Covers the default import (`import express from "express"`), a named one
/// (`import { Router } from "express"`), and either aliased.
fn framework_constructors(extraction: &ModuleExtraction) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for site in &extraction.imports {
        if site.raw_specifier != PACKAGE {
            continue;
        }
        for specifier in &site.specifiers {
            let Some(local) = &specifier.local else {
                continue;
            };
            // A default or namespace import binds the module's callable export whatever it is named
            // locally, so the imported name is irrelevant. A named import must name a known
            // constructor — matching the *export* name, which is a fact about the package, not a
            // guess about the local one.
            let accepted = match specifier.kind {
                "default" | "namespace" => true,
                "named" => specifier
                    .imported
                    .as_deref()
                    .is_some_and(|imported| CONSTRUCTORS.contains(&imported)),
                _ => false,
            };
            if accepted {
                out.insert(local.clone());
            }
        }
    }
    out
}

/// Every local name a module imported anything under.
///
/// Distinguishes `app-not-local` — a real route this rule declines — from an untraceable receiver,
/// which is not known to be a route at all and is therefore not counted.
fn imported_locals(extraction: &ModuleExtraction) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for site in &extraction.imports {
        for specifier in &site.specifiers {
            if let Some(local) = &specifier.local {
                out.insert(local.clone());
            }
        }
    }
    out
}

/// Module-scope names bound to an Express application or router.
fn application_objects(
    root: Node,
    source: &[u8],
    constructors: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for statement in named_children(root) {
        // `const app = express()` is a lexical declaration; `app = express()` an assignment.
        let declarators: Vec<Node> = match statement.kind() {
            "lexical_declaration" | "variable_declaration" => named_children(statement)
                .into_iter()
                .filter(|child| child.kind() == "variable_declarator")
                .collect(),
            _ => Vec::new(),
        };
        for declarator in declarators {
            let (Some(name), Some(value)) = (
                declarator.child_by_field_name("name"),
                declarator.child_by_field_name("value"),
            ) else {
                continue;
            };
            if name.kind() != "identifier" || value.kind() != "call_expression" {
                continue;
            }
            let Some(callee) = value
                .child_by_field_name("function")
                .and_then(|function| dotted_name(function, source))
            else {
                continue;
            };
            if is_framework_constructor(&callee, constructors) {
                out.insert(text(name, source));
            }
        }
    }
    out
}

/// Whether a callee constructs an Express application or router.
///
/// Two spellings, both bindings rather than name matches: a local name imported from `express`, or
/// a member access whose object is such a name — `express.Router()`.
fn is_framework_constructor(callee: &str, constructors: &BTreeSet<String>) -> bool {
    if constructors.contains(callee) {
        return true;
    }
    match callee.rsplit_once('.') {
        Some((head, tail)) => constructors.contains(head) && CONSTRUCTORS.contains(&tail),
        None => false,
    }
}

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

fn text(node: Node, source: &[u8]) -> String {
    std::str::from_utf8(&source[node.byte_range()])
        .unwrap_or_default()
        .to_string()
}

/// Dotted name of an expression, or `None` when it is not one.
///
/// Does not descend into a call, so `makeRouter().get(...)` is not read — that would be
/// speculation about a value this rule cannot follow.
fn dotted_name(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source)),
        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;
            Some(format!(
                "{}.{}",
                dotted_name(object, source)?,
                text(property, source)
            ))
        }
        _ => None,
    }
}

/// Value of a string node when it is a plain literal.
///
/// A template literal with a substitution depends on runtime state and is not a literal; one
/// without a substitution is, and is read.
fn string_value(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "string" => {
            let mut value = String::new();
            for child in named_children(node) {
                match child.kind() {
                    "string_fragment" => value.push_str(&text(child, source)),
                    "escape_sequence" => return None,
                    _ => {}
                }
            }
            Some(value)
        }
        "template_string" => {
            let mut value = String::new();
            for child in named_children(node) {
                match child.kind() {
                    "string_fragment" => value.push_str(&text(child, source)),
                    "template_substitution" | "escape_sequence" => return None,
                    _ => {}
                }
            }
            Some(value)
        }
        _ => None,
    }
}

struct Walker<'a> {
    source: &'a [u8],
    symbols: &'a [SymbolDef],
    apps: &'a BTreeSet<String>,
    imported: &'a BTreeSet<String>,
    out: &'a mut TsFrameworkExtraction,
}

impl Walker<'_> {
    fn visit(&mut self, node: Node, project_id: &str, rel_path: &str) {
        if node.kind() == "call_expression" {
            self.visit_call(node, project_id, rel_path);
        }
        for child in named_children(node) {
            self.visit(child, project_id, rel_path);
        }
    }

    fn visit_call(&mut self, call: Node, project_id: &str, rel_path: &str) {
        let Some(function) = call.child_by_field_name("function") else {
            return;
        };
        let Some(callee) = dotted_name(function, self.source) else {
            return;
        };
        let Some((receiver, method_name)) = callee.rsplit_once('.') else {
            return;
        };
        if !self.apps.contains(receiver) {
            // A route-shaped call on something imported is a real route this rule declines — the
            // stated cross-module lower bound. Anything else is not known to be a route at all, so
            // nothing is counted.
            if self.imported.contains(receiver) && METHOD_CALLS.contains(&method_name) {
                self.out.count(UnsupportedForm::AppNotLocal);
            }
            return;
        }
        if !METHOD_CALLS.contains(&method_name) {
            // `app.all`, `app.use`, `app.listen`, `app.set` — not a single-method declaration.
            return;
        }

        let arguments: Vec<Node> = call
            .child_by_field_name("arguments")
            .map(|node| {
                named_children(node)
                    .into_iter()
                    .filter(|argument| argument.kind() != "comment")
                    .collect()
            })
            .unwrap_or_default();

        // A registration binds a handler. One argument is a lookup, not a declaration.
        if arguments.len() < 2 {
            return;
        }

        let Some(path) = string_value(arguments[0], self.source) else {
            self.out.count(UnsupportedForm::PathNotLiteral);
            return;
        };

        // Express accepts a chain of middleware before the handler. The **last** argument is the
        // handler; anything before it is middleware, which this rule does not model.
        let handler = arguments
            .last()
            .and_then(|argument| self.handler_from_argument(*argument));
        let Some(handler_entity_id) = handler else {
            self.out.count(UnsupportedForm::HandlerNotASymbol);
            return;
        };

        let method = method_name.to_uppercase();
        let span = span_of(call);
        self.out.endpoints.push(TsEndpoint {
            entity_id: ids::endpoint_id(
                project_id,
                rel_path,
                EndpointKind::HttpRoute,
                &method,
                &path,
            ),
            address: format!("{method} {path}"),
            method,
            path,
            handler_entity_id,
            span,
        });
    }

    /// The symbol an argument names, when it names one at all.
    ///
    /// A bare identifier bound to a module-scope symbol. An arrow function, a function expression,
    /// a call result and a member expression all yield `None` — each is a callable the source does
    /// not give a locally declared name, so no entity can be the edge's target.
    ///
    /// `Views.handle` is the most defensible refusal in the set: it *is* a declared static method,
    /// but resolving it means resolving the receiver first, and `py-framework` declines the same
    /// shape. Doing it on one language only would make `SERVED_BY` mean different things per
    /// language.
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
}

/// The evidence typing every framework observation carries.
pub fn directness() -> Directness {
    Directness::Direct
}

/// Whether an entity kind may be a route handler.
pub fn is_handler_kind(kind: EntityKind) -> bool {
    matches!(kind, EntityKind::Function | EntityKind::Method)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_module;

    const PID: &str = "00000000000000000000000000000001";

    fn run(rel_path: &str, source: &str) -> TsFrameworkExtraction {
        let language = Language::TypeScript;
        let structural = extract_module(PID, rel_path, language, source).expect("structural");
        extract_framework(PID, rel_path, source, language, &structural).expect("framework")
    }

    fn addresses(extraction: &TsFrameworkExtraction) -> Vec<String> {
        extraction
            .endpoints
            .iter()
            .map(|endpoint| endpoint.address.clone())
            .collect()
    }

    #[test]
    fn an_express_registration_declares_an_endpoint_served_by_the_named_handler() {
        let extraction = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\nexport function listUsers() {}\n\napp.get(\"/users\", listUsers);\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /users"]);
        assert!(extraction.endpoints[0].handler_entity_id.starts_with("fn_"));
        assert!(extraction.unsupported_by_form.is_empty());
    }

    /// The whole reason the receiver is traced rather than the call name matched.
    #[test]
    fn a_call_spelled_like_a_registration_on_a_non_framework_object_is_not_a_route() {
        let extraction = run(
            "cache.ts",
            "class Cache {\n  get(k: string, f: unknown) { return f; }\n}\n\nconst cache = new Cache();\n\nexport function handler() {}\n\ncache.get(\"/not-a-route\", handler);\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert!(
            extraction.unsupported_by_form.is_empty(),
            "Nerve has no reason to think this is a route, so counting it as missed would itself \
             be a false claim: {:?}",
            extraction.unsupported_by_form
        );
    }

    #[test]
    fn a_default_import_under_any_local_name_resolves_through_the_binding() {
        let extraction = run(
            "routes.ts",
            "import makeServer from \"express\";\n\nconst site = makeServer();\n\nexport function h() {}\n\nsite.get(\"/aliased\", h);\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /aliased"]);
    }

    #[test]
    fn a_named_router_import_and_a_member_constructor_both_resolve() {
        let named = run(
            "a.ts",
            "import { Router } from \"express\";\n\nconst api = Router();\n\nexport function h() {}\n\napi.post(\"/x\", h);\n",
        );
        assert_eq!(addresses(&named), vec!["POST /x"]);

        let member = run(
            "b.ts",
            "import express from \"express\";\n\nconst api = express.Router();\n\nexport function h() {}\n\napi.put(\"/y\", h);\n",
        );
        assert_eq!(addresses(&member), vec!["PUT /y"]);
    }

    #[test]
    fn a_router_mount_prefix_is_not_composed_into_the_declared_path() {
        let extraction = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\nconst router = express.Router();\n\nexport function h() {}\n\nrouter.get(\"/items\", h);\napp.use(\"/v1\", router);\n",
        );
        assert_eq!(
            addresses(&extraction),
            vec!["GET /items"],
            "the mount prefix is a separate statement, and composing it would produce a \
             confidently wrong URL"
        );
    }

    /// `all` matches every method; expanding it would assert eight declarations for one statement.
    #[test]
    fn app_all_is_not_a_route() {
        let extraction = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\nexport function h() {}\n\napp.all(\"/anything\", h);\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert!(extraction.unsupported_by_form.is_empty());
    }

    /// A one-argument call is a lookup, not a registration.
    #[test]
    fn a_single_argument_call_is_not_a_registration() {
        let extraction = run(
            "registry.ts",
            "import express from \"express\";\n\nconst app = express();\n\napp.get(\"/lookup\");\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert!(extraction.unsupported_by_form.is_empty());
    }

    #[test]
    fn an_inline_handler_is_counted_not_emitted() {
        let arrow = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\napp.get(\"/inline\", (req, res) => { void req; void res; });\n",
        );
        assert!(arrow.endpoints.is_empty());
        assert_eq!(
            arrow.unsupported_by_form.get("handler-not-a-symbol"),
            Some(&1)
        );

        let expression = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\napp.post(\"/fn\", function (req, res) { void req; void res; });\n",
        );
        assert_eq!(
            expression.unsupported_by_form.get("handler-not-a-symbol"),
            Some(&1)
        );
    }

    #[test]
    fn a_computed_or_interpolated_path_is_counted_not_guessed() {
        let concatenated = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\nconst P = \"/p\";\n\nexport function h() {}\n\napp.get(P + \"/items\", h);\n",
        );
        assert!(concatenated.endpoints.is_empty());
        assert_eq!(
            concatenated.unsupported_by_form.get("path-not-literal"),
            Some(&1)
        );

        let interpolated = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\nconst P = \"p\";\n\nexport function h() {}\n\napp.get(`/users/${P}`, h);\n",
        );
        assert_eq!(
            interpolated.unsupported_by_form.get("path-not-literal"),
            Some(&1)
        );
    }

    /// A template literal with no substitution is a literal, and is read.
    #[test]
    fn a_template_literal_without_a_substitution_is_a_literal() {
        let extraction = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\nexport function h() {}\n\napp.get(`/plain`, h);\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /plain"]);
    }

    #[test]
    fn an_imported_application_object_is_counted_as_the_stated_lower_bound() {
        let extraction = run(
            "more.ts",
            "import { app } from \"./routes\";\n\nexport function h() {}\n\napp.get(\"/imported\", h);\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert_eq!(
            extraction.unsupported_by_form.get("app-not-local"),
            Some(&1)
        );
    }

    #[test]
    fn a_non_route_call_on_an_imported_object_is_not_counted() {
        let extraction = run(
            "more.ts",
            "import { app } from \"./routes\";\n\nexport function h() {}\n\napp.use(\"/mount\", h);\n",
        );
        assert!(extraction.endpoints.is_empty());
        assert!(extraction.unsupported_by_form.is_empty());
    }

    /// Express accepts middleware before the handler; the last argument is the handler.
    #[test]
    fn the_last_argument_is_the_handler_when_middleware_precedes_it() {
        let extraction = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\nexport function auth() {}\nexport function h() {}\n\napp.get(\"/guarded\", auth, h);\n",
        );
        assert_eq!(addresses(&extraction), vec!["GET /guarded"]);
        let handler = &extraction.endpoints[0].handler_entity_id;
        let expected = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\nexport function auth() {}\nexport function h() {}\n\napp.get(\"/only\", h);\n",
        );
        assert_eq!(handler, &expected.endpoints[0].handler_entity_id);
    }

    #[test]
    fn the_same_address_declared_twice_is_ambiguous_and_keeps_both_edges() {
        let extraction = run(
            "routes.ts",
            "import express from \"express\";\n\nconst app = express();\n\nexport function first() {}\nexport function second() {}\n\napp.get(\"/twice\", first);\napp.get(\"/twice\", second);\n",
        );
        assert_eq!(extraction.endpoints.len(), 2);
        assert_eq!(extraction.ambiguous_addresses.get("GET /twice"), Some(&2));
        assert_eq!(
            extraction.endpoints[0].entity_id, extraction.endpoints[1].entity_id,
            "one declared address in one module is one endpoint, served by two symbols"
        );
    }

    #[test]
    fn the_extractor_declares_only_framework_rule_and_served_by() {
        assert_eq!(DECLARED_SOURCE_TYPES, [EvidenceSourceType::FrameworkRule]);
        assert_eq!(DECLARED_RELATIONS, [Relation::ServedBy]);
        assert_eq!(directness(), Directness::Direct);
        assert_ne!(EXTRACTOR_ID, crate::pyframework::EXTRACTOR_ID);
    }

    /// The unsupported vocabulary is shared with `py-framework`, so two languages report the same
    /// tags. `methods-not-literal` is Flask-specific and never fires here, which is why the shared
    /// vocabulary is a superset rather than a per-language list.
    #[test]
    fn the_unsupported_vocabulary_is_the_one_py_framework_uses() {
        let shared: BTreeSet<&str> = UnsupportedForm::ALL
            .iter()
            .map(|form| form.as_str())
            .collect();
        for tag in ["app-not-local", "path-not-literal", "handler-not-a-symbol"] {
            assert!(
                shared.contains(tag),
                "{tag} must be in the shared vocabulary"
            );
        }
    }
}
