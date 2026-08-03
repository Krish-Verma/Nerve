//! Closed vocabularies.
//!
//! Every vocabulary here is closed on purpose: values are parsed, never invented, and an
//! unknown string is an error rather than a silently-tolerated free-text tag.

use std::fmt;
use std::str::FromStr;

use crate::error::NerveError;

/// The kinds of entity Nerve can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityKind {
    /// The indexed repository itself.
    Repository,
    /// A directory inside the repository.
    Directory,
    /// A file inside the repository.
    File,
    /// A module. For TS/JS this is 1:1 with a file.
    Module,
    /// A free function (including declarator-bound arrow and function expressions).
    Function,
    /// A class method.
    Method,
    /// A class.
    Class,
    /// A TypeScript interface.
    Interface,
    /// A prose document. For Markdown this is 1:1 with a file.
    Document,
    /// A heading-delimited region of a document.
    Section,
    /// A reference target that could not be resolved. A value, never an omission.
    Unresolved,
    /// One coverage report, identified by its repository-relative path and its content hash.
    ///
    /// **Not a test.** LCOV carries no per-test attribution — its `TN:` field is empty and one
    /// report describes one whole run — so the only endpoint the evidence supports is the run
    /// that produced the report. See `docs/decisions/ADR-0008-coverage-evidence.md`.
    CoverageRun,
    /// A declared entry point at which something outside the indexed code can cause a symbol to
    /// run.
    ///
    /// **Not code.** An endpoint is a *declaration about* code — a decorator, a registration call
    /// — so it is not a symbol and is not addressed by a repository path. Which sort of entry
    /// point it is lives in `meta.endpoint_kind`, from the closed [`EndpointKind`] vocabulary;
    /// Slice 10 emits exactly one value, [`EndpointKind::HttpRoute`].
    ///
    /// Named `Endpoint` rather than `Route` because CLI commands, queue consumers and scheduled
    /// tasks are the same concept with a different address form, and three vocabulary members for
    /// one concept is the drift Slices 5d-iii and 7a-iii were corrective slices for. A general
    /// name is not a licence to emit general things: [`EndpointKind::ALL`] is what bounds it.
    Endpoint,
}

impl EntityKind {
    /// Every kind, in declaration order.
    ///
    /// Appended to, never inserted into. Nothing on disk encodes a position — `entity.kind` is
    /// `TEXT` — but `apps/nerve-web/src/api/types.ts` mirrors this array *in order* and
    /// `crates/nerve-server/tests/ui_vocabulary.rs` asserts the two match exactly, so the order
    /// is a contract with the interface even though it is not one with the database.
    pub const ALL: [EntityKind; 13] = [
        EntityKind::Repository,
        EntityKind::Directory,
        EntityKind::File,
        EntityKind::Module,
        EntityKind::Function,
        EntityKind::Method,
        EntityKind::Class,
        EntityKind::Interface,
        EntityKind::Document,
        EntityKind::Section,
        EntityKind::Unresolved,
        // Appended in Slice 6a.
        EntityKind::CoverageRun,
        // Appended in Slice 10a.
        EntityKind::Endpoint,
    ];

    /// Canonical lower-case name, used in the database and in canonical tuples.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Repository => "repository",
            EntityKind::Directory => "directory",
            EntityKind::File => "file",
            EntityKind::Module => "module",
            EntityKind::Function => "function",
            EntityKind::Method => "method",
            EntityKind::Class => "class",
            EntityKind::Interface => "interface",
            EntityKind::Document => "document",
            EntityKind::Section => "section",
            EntityKind::Unresolved => "unresolved",
            EntityKind::CoverageRun => "coverage_run",
            EntityKind::Endpoint => "endpoint",
        }
    }

    /// Identifier prefix, per ADR-0002.
    pub fn prefix(self) -> &'static str {
        match self {
            EntityKind::Repository => "repo",
            EntityKind::Directory => "dir",
            EntityKind::File => "file",
            EntityKind::Module => "mod",
            EntityKind::Function => "fn",
            EntityKind::Method => "meth",
            EntityKind::Class => "class",
            EntityKind::Interface => "iface",
            EntityKind::Document => "doc",
            EntityKind::Section => "sect",
            EntityKind::Unresolved => "unres",
            EntityKind::CoverageRun => "cov",
            EntityKind::Endpoint => "endp",
        }
    }

    /// True for kinds identified by the symbol tuple
    /// `(kind, project_id, module_rel_path, scope_path, name, disambiguator)`.
    pub fn is_symbol(self) -> bool {
        matches!(
            self,
            EntityKind::Function | EntityKind::Method | EntityKind::Class | EntityKind::Interface
        )
    }

    /// How this kind is located by a repository-relative path, if it is at all.
    ///
    /// This is the vocabulary's own answer to *"which entities does `docs/architecture.md`
    /// name?"*, and it is stated here rather than as a list in a SQL string because a kind added
    /// to [`EntityKind::ALL`] must be classified before it can be addressed — the drift that
    /// Slices 5d-iii and 7a-iii were corrective slices for.
    ///
    /// The two addressable roles differ in **where the path is stored**, which is why one
    /// predicate cannot serve both:
    ///
    /// - [`PathRole::Content`] — `scope_path` *is* the path (`Module`, `Document`).
    /// - [`PathRole::Container`] — the path is `scope_path` joined to `name` (`File`,
    ///   `Directory`).
    ///
    /// Everything else is [`PathRole::None`], and each for a reason rather than by default: a
    /// `Section`'s `scope_path` is the document that holds it, an `Unresolved`'s is the importer
    /// that failed to resolve it, a `CoverageRun` is named by a report path and a content hash
    /// rather than by a position in the tree, an `Endpoint` is named by the address a framework
    /// serves it at, the symbol kinds are named by the symbol tuple, and the `Repository` is the
    /// root every path is relative *to*.
    pub fn path_role(self) -> PathRole {
        match self {
            EntityKind::Module | EntityKind::Document => PathRole::Content,
            EntityKind::File | EntityKind::Directory => PathRole::Container,
            EntityKind::Repository
            | EntityKind::Function
            | EntityKind::Method
            | EntityKind::Class
            | EntityKind::Interface
            | EntityKind::Section
            | EntityKind::Unresolved
            | EntityKind::CoverageRun
            | EntityKind::Endpoint => PathRole::None,
        }
    }
}

/// What sort of entry point an [`EntityKind::Endpoint`] is.
///
/// Closed on purpose, and closed **in `nerve-core`** rather than in whichever extractor happens to
/// need one: `EntityKind::Endpoint` is deliberately a general name, and a general name with an
/// open discriminator is a free-text tag wearing a vocabulary's clothes. An extractor cannot
/// invent a member; adding one is a change to this file.
///
/// **Slice 10 emits exactly one value.** The others are not declared here in advance either —
/// declaring `cli_command` before a rule produces one would be the same over-claim in a different
/// place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EndpointKind {
    /// An HTTP route declared by a web framework: a method and a path, as the source declares
    /// them.
    ///
    /// **The declared address, never the deployed one.** Prefix composition
    /// (`APIRouter(prefix=…)`, `app.include_router(…)`, `register_blueprint(…)`) is not applied,
    /// because composing it needs cross-module value tracking and inventing a composed address
    /// would produce a confidently wrong URL.
    HttpRoute,
}

impl EndpointKind {
    /// Every endpoint kind, in declaration order.
    pub const ALL: [EndpointKind; 1] = [EndpointKind::HttpRoute];

    /// Canonical name, recorded in `entity.meta.endpoint_kind` and in identity tuples.
    pub fn as_str(self) -> &'static str {
        match self {
            EndpointKind::HttpRoute => "http_route",
        }
    }
}

impl fmt::Display for EndpointKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EndpointKind {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EndpointKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| NerveError::unknown("EndpointKind", s))
    }
}

/// Whether — and how — a repository-relative path names an entity of a given kind.
///
/// See [`EntityKind::path_role`]. The distinction between the two addressable roles is what lets
/// a path selector resolve to the content at that path while still reporting the container as a
/// second reading, rather than silently answering one of two questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathRole {
    /// Not addressable by a repository path.
    None,
    /// The entity is the content at that path; `scope_path` holds the path itself.
    Content,
    /// The entity is the container at that path; the path is `scope_path` joined to `name`.
    Container,
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntityKind {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EntityKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| NerveError::unknown("EntityKind", s))
    }
}

/// The relation vocabulary.
///
/// Slice 1 emits only [`Relation::SLICE1_EMITTED`]. The remaining variants are declared now so
/// that the vocabulary does not churn when Slice 2 adds resolved and inferred relationships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    /// Filesystem containment.
    Contains,
    /// Lexical definition.
    Defines,
    /// Module-level import dependency.
    Imports,
    /// Module-level export.
    Exports,
    /// Call relationship. Declared, not emitted in Slice 1.
    Calls,
    /// Non-call symbol reference. Declared, not emitted in Slice 1.
    References,
    /// Class/interface inheritance. Declared, not emitted in Slice 1.
    Extends,
    /// Interface implementation. Declared, not emitted in Slice 1.
    Implements,
    /// One document supersedes another. Declared in Slice 5a, emitted in Slice 5b.
    Supersedes,
    /// A coverage run executed at least one line inside a symbol.
    ///
    /// **Never a call relationship** (ADR-0005). Two symbols executing during one run says
    /// nothing about who invoked whom, and this relation must never be relabelled, aliased or
    /// presented as `CALLS`.
    ///
    /// Named `COVERS` rather than `TEST_COVERS_SYMBOL` for the reasons in
    /// `docs/decisions/ADR-0008-coverage-evidence.md`: this vocabulary names relationships, never
    /// their endpoints' kinds, and the source endpoint is a [`EntityKind::CoverageRun`] rather
    /// than a test, so a name containing `TEST_` would assert an attribution LCOV does not carry.
    ///
    /// Declared in Slice 6a, emitted in Slice 6b.
    Covers,
    /// An [`EntityKind::Endpoint`] declares that a symbol implements it.
    ///
    /// `Endpoint SERVED_BY Function|Method`. **The endpoint is the source**, and the direction is
    /// forced rather than chosen: `nerve impact X` is a *reverse* closure — it finds `A` where
    /// `A rel X` — so a handler stops looking like dead code only if the endpoint asserts the
    /// edge. Semantically that is also the right way round: change the handler and the endpoint's
    /// behaviour changes, so the endpoint depends on the handler.
    ///
    /// **Passive voice, deliberately.** Every other relation here is active. The active
    /// alternatives all over-claim: `DISPATCHES_TO` and `INVOKES` assert a runtime invocation from
    /// a static registration, and `ROUTES_TO` is HTTP-only, which contradicts the general kind.
    ///
    /// **Never a call relationship**, the same invariant ADR-0005 states for [`Relation::Covers`].
    /// A registration proves a table entry, not an execution: it does not prove the route is
    /// reachable in production, that middleware permits access, that dynamic configuration has not
    /// replaced it, that a decorator-generated wrapper preserves the handler's identity, or that
    /// two matching path strings denote one deployed endpoint.
    ///
    /// Declared and emitted in Slice 10a.
    ServedBy,
    /// A tracer observed one symbol call another while a test was executing.
    ///
    /// `Function|Method TEST_OBSERVED_CALL Function|Method`. **The endpoints are the two frames of
    /// the call, never the test.** For a stack `test_x → parse → lex` a tracer records two call
    /// events, and naming the test as the source of the second would assert a call `test_x` never
    /// made. Which test observed the edge is provenance and lives on the observation
    /// (`docs/plans/slice-11a-trace-ingestion.md` §2.1), which is where ADR-0003 puts provenance.
    ///
    /// **Existential, never universal.** It says one run, in one environment, took this edge. It
    /// does not say the edge is always taken, and the absence of it is absence of observation
    /// rather than absence of a call — the same rule this project applies to coverage and gaps.
    ///
    /// **Not `CALLS`, and never relabelled as it.** [`Relation::Calls`] is what the source says;
    /// this is what one execution did. The two are separate members of a closed vocabulary because
    /// a trace can hold an edge no source statement resolves (Python's measured 42.3% unresolved
    /// call rate) and the source can state an edge no run ever took.
    ///
    /// **Not `COVERS`.** Coverage says lines inside a symbol executed and has no attribution for
    /// who invoked whom (ADR-0005, ADR-0008); a trace states the caller outright. A `COVERS`
    /// observation may never become one of these, and a test asserts it over
    /// [`Relation::ALL`].
    ///
    /// Declared and emitted in Slice 11a.
    TestObservedCall,
}

impl Relation {
    /// Every relation, in declaration order.
    ///
    /// Appended to, never inserted into — `apps/nerve-web/src/api/types.ts` mirrors this array in
    /// order and `crates/nerve-server/tests/ui_vocabulary.rs` asserts the two match exactly.
    pub const ALL: [Relation; 12] = [
        Relation::Contains,
        Relation::Defines,
        Relation::Imports,
        Relation::Exports,
        Relation::Calls,
        Relation::References,
        Relation::Extends,
        Relation::Implements,
        Relation::Supersedes,
        // Appended in Slice 6a.
        Relation::Covers,
        // Appended in Slice 10a.
        Relation::ServedBy,
        // Appended in Slice 11a.
        Relation::TestObservedCall,
    ];

    /// The relations Slice 1 is permitted to emit.
    pub const SLICE1_EMITTED: [Relation; 4] = [
        Relation::Contains,
        Relation::Defines,
        Relation::Imports,
        Relation::Exports,
    ];

    /// Canonical upper-case name, used in the database and in canonical tuples.
    pub fn as_str(self) -> &'static str {
        match self {
            Relation::Contains => "CONTAINS",
            Relation::Defines => "DEFINES",
            Relation::Imports => "IMPORTS",
            Relation::Exports => "EXPORTS",
            Relation::Calls => "CALLS",
            Relation::References => "REFERENCES",
            Relation::Extends => "EXTENDS",
            Relation::Implements => "IMPLEMENTS",
            Relation::Supersedes => "SUPERSEDES",
            Relation::Covers => "COVERS",
            Relation::ServedBy => "SERVED_BY",
            Relation::TestObservedCall => "TEST_OBSERVED_CALL",
        }
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Relation {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Relation::ALL
            .into_iter()
            .find(|r| r.as_str() == s)
            .ok_or_else(|| NerveError::unknown("Relation", s))
    }
}

/// What an [`EntityKind::Unresolved`] entity stands in for.
///
/// This is a **domain discriminator**, not decoration. `import { parse } from 'parse'` and a
/// call to `parse()` in the same file are different unresolved things; without a category in
/// the identity tuple they would hash to one entity and Nerve would claim a module and a value
/// are the same thing. ADR-0002's tuples exist precisely to prevent that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnresolvedCategory {
    /// A module specifier that named no indexed module.
    Module,
    /// A value or type name that no binding in scope could resolve.
    Value,
    /// A document link destination — or its `#L<n>` anchor — that named nothing indexed.
    DocumentLink,
    /// A supersession marker in an ADR that named no single indexed ADR.
    DocumentSupersedes,
}

impl UnresolvedCategory {
    /// Every category, in declaration order.
    pub const ALL: [UnresolvedCategory; 4] = [
        UnresolvedCategory::Module,
        UnresolvedCategory::Value,
        UnresolvedCategory::DocumentLink,
        UnresolvedCategory::DocumentSupersedes,
    ];

    /// Canonical name, used in identity tuples and in entity metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            UnresolvedCategory::Module => "module",
            UnresolvedCategory::Value => "value",
            UnresolvedCategory::DocumentLink => "document_link",
            UnresolvedCategory::DocumentSupersedes => "document_supersedes",
        }
    }
}

impl fmt::Display for UnresolvedCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UnresolvedCategory {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        UnresolvedCategory::ALL
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| NerveError::unknown("UnresolvedCategory", s))
    }
}

/// How a piece of evidence was obtained (ADR-0003).
///
/// The declaration order is **not** a truth ranking — ADR-0003 is explicit that ranking is
/// supplied by an evidence policy at query time. It is used only as the default structural
/// ordering for [`EvidenceSourceType::ordinal`] and for the `source_type_mask` bit layout,
/// both of which must be stable for the lifetime of a schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceSourceType {
    /// The syntax tree literally contains this relationship.
    AstDirect,
    /// Resolved through import/module resolution.
    AstResolved,
    /// Name-based or otherwise ambiguous match.
    AstHeuristic,
    /// A type checker resolved it.
    TypeResolved,
    /// A deterministic framework rule inferred it.
    FrameworkRule,
    /// A test executed this symbol. Not a call relationship (ADR-0005).
    TestCoverage,
    /// A call observed during a test, via instrumentation.
    TestCallTrace,
    /// A call observed at runtime.
    RuntimeCallTrace,
    /// A document asserts it.
    DocumentStated,
    /// A human confirmed it.
    HumanConfirmed,
    /// A language model suggested it.
    LlmDerived,
    /// The filesystem contains this. Derived from a directory walk, never from file content.
    FilesystemObserved,
}

impl EvidenceSourceType {
    /// Every source type, in declaration order. Index in this array is the ordinal.
    pub const ALL: [EvidenceSourceType; 12] = [
        EvidenceSourceType::AstDirect,
        EvidenceSourceType::AstResolved,
        EvidenceSourceType::AstHeuristic,
        EvidenceSourceType::TypeResolved,
        EvidenceSourceType::FrameworkRule,
        EvidenceSourceType::TestCoverage,
        EvidenceSourceType::TestCallTrace,
        EvidenceSourceType::RuntimeCallTrace,
        EvidenceSourceType::DocumentStated,
        EvidenceSourceType::HumanConfirmed,
        EvidenceSourceType::LlmDerived,
        // Appended in Slice 5d-i. `ordinal()` is the index into this array and `mask_bit()` is
        // `1 << ordinal`, so `assertion_state.source_type_mask` — a **stored** integer — is a
        // function of these positions. Appending leaves every existing ordinal and every stored
        // bit correct; inserting anywhere else silently reinterprets every mask already on disk.
        // Append only. `evidence_source_type_ordinals_and_masks_are_stable` pins all of them.
        EvidenceSourceType::FilesystemObserved,
    ];

    /// Canonical database name.
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceSourceType::AstDirect => "AST_DIRECT",
            EvidenceSourceType::AstResolved => "AST_RESOLVED",
            EvidenceSourceType::AstHeuristic => "AST_HEURISTIC",
            EvidenceSourceType::TypeResolved => "TYPE_RESOLVED",
            EvidenceSourceType::FrameworkRule => "FRAMEWORK_RULE",
            EvidenceSourceType::TestCoverage => "TEST_COVERAGE",
            EvidenceSourceType::TestCallTrace => "TEST_CALL_TRACE",
            EvidenceSourceType::RuntimeCallTrace => "RUNTIME_CALL_TRACE",
            EvidenceSourceType::DocumentStated => "DOCUMENT_STATED",
            EvidenceSourceType::HumanConfirmed => "HUMAN_CONFIRMED",
            EvidenceSourceType::LlmDerived => "LLM_DERIVED",
            EvidenceSourceType::FilesystemObserved => "FILESYSTEM_OBSERVED",
        }
    }

    /// Stable ordinal, fixed for the lifetime of a schema version.
    pub fn ordinal(self) -> u32 {
        EvidenceSourceType::ALL
            .iter()
            .position(|s| *s == self)
            .expect("ALL contains every variant") as u32
    }

    /// The `source_type_mask` bit for this source type.
    pub fn mask_bit(self) -> i64 {
        1i64 << self.ordinal()
    }
}

impl fmt::Display for EvidenceSourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EvidenceSourceType {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EvidenceSourceType::ALL
            .into_iter()
            .find(|e| e.as_str() == s)
            .ok_or_else(|| NerveError::unknown("EvidenceSourceType", s))
    }
}

/// How directly the evidence states the relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Directness {
    /// The artifact literally states it.
    Direct,
    /// Derived through a resolution step (for example module resolution).
    Resolved,
    /// A rule concluded it.
    Inferred,
}

impl Directness {
    /// Every directness value, in declaration order.
    pub const ALL: [Directness; 3] = [
        Directness::Direct,
        Directness::Resolved,
        Directness::Inferred,
    ];

    /// Canonical database name.
    pub fn as_str(self) -> &'static str {
        match self {
            Directness::Direct => "DIRECT",
            Directness::Resolved => "RESOLVED",
            Directness::Inferred => "INFERRED",
        }
    }
}

impl fmt::Display for Directness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Directness {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Directness::ALL
            .into_iter()
            .find(|d| d.as_str() == s)
            .ok_or_else(|| NerveError::unknown("Directness", s))
    }
}

/// Derived status of an assertion in a repository state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssertionStatus {
    /// At least one observation supports it and its target resolved.
    Supported,
    /// Observations disagree. Requires multiple extractors; not reachable in Slice 1.
    Contradicted,
    /// Last observed in an older repository state. Requires incremental indexing.
    Stale,
    /// Supported, but the target is an `Unresolved` entity.
    Unresolved,
    /// Explicitly retracted. Not reachable in Slice 1.
    Deleted,
}

impl AssertionStatus {
    /// Every status, in declaration order.
    pub const ALL: [AssertionStatus; 5] = [
        AssertionStatus::Supported,
        AssertionStatus::Contradicted,
        AssertionStatus::Stale,
        AssertionStatus::Unresolved,
        AssertionStatus::Deleted,
    ];

    /// Canonical database name.
    pub fn as_str(self) -> &'static str {
        match self {
            AssertionStatus::Supported => "SUPPORTED",
            AssertionStatus::Contradicted => "CONTRADICTED",
            AssertionStatus::Stale => "STALE",
            AssertionStatus::Unresolved => "UNRESOLVED",
            AssertionStatus::Deleted => "DELETED",
        }
    }
}

impl fmt::Display for AssertionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AssertionStatus {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AssertionStatus::ALL
            .into_iter()
            .find(|a| a.as_str() == s)
            .ok_or_else(|| NerveError::unknown("AssertionStatus", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_kind_round_trips() {
        for kind in EntityKind::ALL {
            assert_eq!(kind.as_str().parse::<EntityKind>().unwrap(), kind);
        }
        assert!("nonsense".parse::<EntityKind>().is_err());
    }

    #[test]
    fn entity_kind_prefixes_are_unique_and_stable() {
        let prefixes: Vec<&str> = EntityKind::ALL.iter().map(|k| k.prefix()).collect();
        let mut sorted = prefixes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), prefixes.len(), "prefixes must be unique");
        assert_eq!(
            prefixes,
            vec![
                "repo", "dir", "file", "mod", "fn", "meth", "class", "iface", "doc", "sect",
                "unres", "cov", "endp"
            ]
        );
    }

    /// The two Slice 10a additions, named for exactly what the evidence supports.
    ///
    /// `SERVED_BY` is spelled passively because every active alternative asserts an execution the
    /// evidence does not carry, and it must never be relabelled as a call — the same invariant
    /// ADR-0005 states for `COVERS`. `Endpoint` is not a symbol: it is a declaration *about* code,
    /// named by the address a framework serves it at rather than by the symbol tuple.
    #[test]
    fn the_framework_vocabulary_states_only_what_a_registration_carries() {
        assert_eq!(Relation::ServedBy.as_str(), "SERVED_BY");
        assert_eq!("SERVED_BY".parse::<Relation>().unwrap(), Relation::ServedBy);
        // The names that would assert a runtime invocation from a static registration.
        for over_claim in ["DISPATCHES_TO", "INVOKES", "ROUTES_TO", "HANDLES"] {
            assert!(over_claim.parse::<Relation>().is_err());
        }
        assert_ne!(Relation::ServedBy, Relation::Calls);

        assert_eq!(EntityKind::Endpoint.as_str(), "endpoint");
        assert_eq!(
            "endpoint".parse::<EntityKind>().unwrap(),
            EntityKind::Endpoint
        );
        assert!(!EntityKind::Endpoint.is_symbol());
        assert_eq!(EntityKind::Endpoint.path_role(), PathRole::None);
        // `route` would be the narrow name this kind deliberately does not have.
        assert!("route".parse::<EntityKind>().is_err());
    }

    /// The endpoint discriminator is a closed vocabulary, and Slice 10 emits one member of it.
    ///
    /// `EntityKind::Endpoint` is a general name on purpose. A general name whose discriminator is
    /// free text is not a vocabulary at all, so the discriminator is parsed here and an extractor
    /// cannot invent a value: `"cli_command"` is a plausible future member and must not parse
    /// until someone writes the rule that produces it.
    #[test]
    fn the_endpoint_kind_vocabulary_is_closed_and_has_one_member() {
        assert_eq!(EndpointKind::ALL.len(), 1);
        assert_eq!(EndpointKind::HttpRoute.as_str(), "http_route");
        for kind in EndpointKind::ALL {
            assert_eq!(kind.as_str().parse::<EndpointKind>().unwrap(), kind);
            assert_eq!(kind.to_string(), kind.as_str());
        }
        for invented in [
            "cli_command",
            "queue_consumer",
            "scheduled_task",
            "route",
            "",
        ] {
            assert!(
                invented.parse::<EndpointKind>().is_err(),
                "{invented:?} parsed as an EndpointKind without a rule that emits it"
            );
        }
    }

    /// The two Slice 6a additions, named for exactly what the evidence supports.
    ///
    /// `COVERS` is spelled without either endpoint's kind, because the vocabulary never puts a
    /// kind in a relation name and because LCOV carries no per-test attribution to justify the
    /// `TEST_` the roadmap originally proposed (ADR-0008). `CoverageRun` is not a symbol kind:
    /// it is named by its report path and content hash, not by the symbol tuple.
    #[test]
    fn the_coverage_vocabulary_states_only_what_the_evidence_carries() {
        assert_eq!(Relation::Covers.as_str(), "COVERS");
        assert_eq!("COVERS".parse::<Relation>().unwrap(), Relation::Covers);
        assert!("TEST_COVERS_SYMBOL".parse::<Relation>().is_err());

        assert_eq!(EntityKind::CoverageRun.as_str(), "coverage_run");
        assert_eq!(
            "coverage_run".parse::<EntityKind>().unwrap(),
            EntityKind::CoverageRun
        );
        assert!(!EntityKind::CoverageRun.is_symbol());

        // Coverage is not a call graph (ADR-0005). The two are separate members of a closed
        // vocabulary, and neither parses as the other.
        assert_ne!(Relation::Covers, Relation::Calls);
    }

    /// The Slice 11a addition, named for exactly what one execution carries.
    ///
    /// `TEST_OBSERVED_CALL` is the whole of the vocabulary change: a trace run is provenance rather
    /// than an endpoint, so — unlike `COVERS` — it needs no `EntityKind` of its own. The names that
    /// would over-claim are refused: `TRACED_CALL` and `RUNTIME_CALL` drop the fact that a *test*
    /// was executing, `OBSERVED_CALL` drops it too, and `TEST_CALLS` would read as a call the test
    /// itself made, which is false for every frame below the test body.
    #[test]
    fn the_trace_vocabulary_states_only_what_one_execution_carries() {
        assert_eq!(Relation::TestObservedCall.as_str(), "TEST_OBSERVED_CALL");
        assert_eq!(
            "TEST_OBSERVED_CALL".parse::<Relation>().unwrap(),
            Relation::TestObservedCall
        );
        for over_claim in [
            "TRACED_CALL",
            "RUNTIME_CALL",
            "OBSERVED_CALL",
            "TEST_CALLS",
            "RUNTIME_OBSERVED_CALL",
            "FRAMEWORK_INFERRED_CALL",
        ] {
            assert!(
                over_claim.parse::<Relation>().is_err(),
                "{over_claim:?} parsed as a Relation without a rule that emits it"
            );
        }
        // Three separate members of one closed vocabulary. What the source says, what a run
        // executed, and what a run observed calling what are three different claims.
        assert_ne!(Relation::TestObservedCall, Relation::Calls);
        assert_ne!(Relation::TestObservedCall, Relation::Covers);

        // Appended, and the appending is the point: `ALL` is mirrored by index in
        // `apps/nerve-web/src/api/types.ts`.
        assert_eq!(Relation::ALL.len(), 12);
        assert_eq!(Relation::ALL[11], Relation::TestObservedCall);

        // Its source type has existed since Slice 1 and Slice 11a is its first emitter; no member
        // was added on that axis and no ordinal moved.
        assert_eq!(EvidenceSourceType::TestCallTrace.ordinal(), 6);
        assert_eq!(EvidenceSourceType::ALL.len(), 12);
    }

    /// A document and a section are named by their own tuples, not by the symbol tuple. If
    /// either ever became a symbol kind, `select.rs` would fold a heading into a dotted
    /// qualified name that appears nowhere in the repository.
    #[test]
    fn documents_and_sections_are_not_symbols() {
        assert!(!EntityKind::Document.is_symbol());
        assert!(!EntityKind::Section.is_symbol());
    }

    /// Every kind's classification, pinned one by one.
    ///
    /// [`EntityKind::is_symbol`] is a `matches!` over four variants, so a kind added to the
    /// vocabulary is silently classified as *not* a symbol — a default, not a decision. That
    /// default is load-bearing well beyond the symbol tuple it was written for: it now decides
    /// which kinds `symbol_kinds_sql` puts in an `IN (…)` clause, and therefore what
    /// `StatusReport::symbols_total` counts and what the interface prints beside the word
    /// *symbols*. Listing all thirteen, and checking the list is exhaustive over
    /// [`EntityKind::ALL`], makes a new kind fail here until someone states which it is.
    #[test]
    fn every_entity_kind_is_classified_as_a_symbol_or_not() {
        let pinned: [(EntityKind, bool); 13] = [
            // Named by a path, a tuple of its own, or a content hash — never by the symbol tuple.
            (EntityKind::Repository, false),
            (EntityKind::Directory, false),
            (EntityKind::File, false),
            (EntityKind::Module, false),
            (EntityKind::Document, false),
            (EntityKind::Section, false),
            (EntityKind::Unresolved, false),
            (EntityKind::CoverageRun, false),
            // An endpoint is a declaration *about* code, not code. Counting it as a symbol would
            // make `symbols_total` grow by one per route and the interface print a number of
            // "symbols" the repository does not contain.
            (EntityKind::Endpoint, false),
            // The symbol tuple's four kinds, and the whole of what "symbol" means here.
            (EntityKind::Function, true),
            (EntityKind::Method, true),
            (EntityKind::Class, true),
            (EntityKind::Interface, true),
        ];

        for (kind, expected) in pinned {
            assert_eq!(
                kind.is_symbol(),
                expected,
                "{kind} is classified against this list"
            );
        }

        let mut listed: Vec<EntityKind> = pinned.iter().map(|(kind, _)| *kind).collect();
        listed.sort_unstable();
        let mut all = EntityKind::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a kind was added to the vocabulary without a classification above"
        );

        assert_eq!(
            EntityKind::ALL.iter().filter(|k| k.is_symbol()).count(),
            4,
            "the symbol tuple names exactly four kinds"
        );
    }

    /// Every kind's path role, pinned one by one, for the same reason `is_symbol` is.
    ///
    /// [`EntityKind::path_role`] decides which entities a `<rel_path>` selector may return and
    /// which SQL shape finds them, so a kind added to the vocabulary without a role would be
    /// silently unaddressable — a default rather than a decision. Listing all thirteen, and
    /// checking the list is exhaustive over [`EntityKind::ALL`], makes a new kind fail here until
    /// someone states where it lives.
    #[test]
    fn every_entity_kind_states_how_a_path_names_it() {
        let pinned: [(EntityKind, PathRole); 13] = [
            // `scope_path` is the path itself.
            (EntityKind::Module, PathRole::Content),
            (EntityKind::Document, PathRole::Content),
            // The path is `scope_path` joined to `name`.
            (EntityKind::File, PathRole::Container),
            (EntityKind::Directory, PathRole::Container),
            // Named by something other than a position in the tree.
            (EntityKind::Repository, PathRole::None),
            (EntityKind::Function, PathRole::None),
            (EntityKind::Method, PathRole::None),
            (EntityKind::Class, PathRole::None),
            (EntityKind::Interface, PathRole::None),
            (EntityKind::Section, PathRole::None),
            (EntityKind::Unresolved, PathRole::None),
            (EntityKind::CoverageRun, PathRole::None),
            // An endpoint's `scope_path` is the module that declares it, exactly as a section's
            // is the document that holds it. `api/routes.py` names that module, never the routes
            // declared inside it.
            (EntityKind::Endpoint, PathRole::None),
        ];

        for (kind, expected) in pinned {
            assert_eq!(
                kind.path_role(),
                expected,
                "{kind} is pinned against this list"
            );
        }

        let mut listed: Vec<EntityKind> = pinned.iter().map(|(kind, _)| *kind).collect();
        listed.sort_unstable();
        let mut all = EntityKind::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a kind was added to the vocabulary without a path role above"
        );

        // A symbol is never addressed by a bare path: `src/app.ts` names the module or the file
        // there, never a function inside it. That is what keeps stage 2 from competing with the
        // `<rel_path>#<qualified_name>` stage.
        for kind in EntityKind::ALL {
            assert!(
                !(kind.is_symbol() && kind.path_role() != PathRole::None),
                "{kind} is both a symbol and path-addressable"
            );
        }
    }

    #[test]
    fn unresolved_category_round_trips() {
        for category in UnresolvedCategory::ALL {
            assert_eq!(
                category.as_str().parse::<UnresolvedCategory>().unwrap(),
                category
            );
        }
        assert!("nonsense".parse::<UnresolvedCategory>().is_err());
    }

    #[test]
    fn relation_round_trips() {
        for relation in Relation::ALL {
            assert_eq!(relation.as_str().parse::<Relation>().unwrap(), relation);
        }
    }

    /// Every ordinal and every mask bit, pinned one by one.
    ///
    /// `assertion_state.source_type_mask` is a **stored** integer whose bit layout is
    /// `1 << ordinal`, and `ordinal` is a position in [`EvidenceSourceType::ALL`]. A variant
    /// inserted anywhere but the end therefore reinterprets every mask in every database already
    /// on disk, silently and unrecoverably. Spot-checking the first and last variant would not
    /// catch an insertion in the middle, so each one is listed: this test is the thing that makes
    /// such an insertion fail loudly at the point it is written.
    #[test]
    fn evidence_source_type_ordinals_and_masks_are_stable() {
        let pinned: [(EvidenceSourceType, u32, i64, &str); 12] = [
            (EvidenceSourceType::AstDirect, 0, 1, "AST_DIRECT"),
            (EvidenceSourceType::AstResolved, 1, 2, "AST_RESOLVED"),
            (EvidenceSourceType::AstHeuristic, 2, 4, "AST_HEURISTIC"),
            (EvidenceSourceType::TypeResolved, 3, 8, "TYPE_RESOLVED"),
            (EvidenceSourceType::FrameworkRule, 4, 16, "FRAMEWORK_RULE"),
            (EvidenceSourceType::TestCoverage, 5, 32, "TEST_COVERAGE"),
            (EvidenceSourceType::TestCallTrace, 6, 64, "TEST_CALL_TRACE"),
            (
                EvidenceSourceType::RuntimeCallTrace,
                7,
                128,
                "RUNTIME_CALL_TRACE",
            ),
            (
                EvidenceSourceType::DocumentStated,
                8,
                256,
                "DOCUMENT_STATED",
            ),
            (
                EvidenceSourceType::HumanConfirmed,
                9,
                512,
                "HUMAN_CONFIRMED",
            ),
            (EvidenceSourceType::LlmDerived, 10, 1024, "LLM_DERIVED"),
            (
                EvidenceSourceType::FilesystemObserved,
                11,
                2048,
                "FILESYSTEM_OBSERVED",
            ),
        ];

        assert_eq!(
            pinned.len(),
            EvidenceSourceType::ALL.len(),
            "a source type was added without pinning its ordinal and mask bit"
        );
        for (index, (source, ordinal, mask, name)) in pinned.into_iter().enumerate() {
            assert_eq!(
                EvidenceSourceType::ALL[index],
                source,
                "{name} moved position in ALL"
            );
            assert_eq!(source.ordinal(), ordinal, "{name} ordinal moved");
            assert_eq!(source.mask_bit(), mask, "{name} mask bit moved");
            assert_eq!(source.as_str(), name, "{name} canonical name changed");
            assert_eq!(name.parse::<EvidenceSourceType>().unwrap(), source);
        }
    }

    #[test]
    fn directness_and_status_round_trip() {
        for d in Directness::ALL {
            assert_eq!(d.as_str().parse::<Directness>().unwrap(), d);
        }
        for s in AssertionStatus::ALL {
            assert_eq!(s.as_str().parse::<AssertionStatus>().unwrap(), s);
        }
    }
}
