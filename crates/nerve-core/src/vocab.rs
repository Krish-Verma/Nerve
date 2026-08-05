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

/// What one commit did to one path, as a tree diff against a single parent states it.
///
/// Closed, and **four**-valued rather than three. `mode_changed` is a real change to a tracked
/// file whose bytes are identical, so folding it into [`ChangeKind::Modified`] would claim content
/// moved when it did not, and omitting it would report the commit as touching nothing.
///
/// There is deliberately **no `renamed` member**. Git records no rename; a rename is *detected*,
/// and in Nerve it is a hypothesis with its own evidence and ambiguity
/// ([`RenameEvidence`], [`RenameAmbiguity`]) rather than a change kind. A `renamed` value here
/// would state as fact the one thing about history that is a guess.
///
/// Added in Slice 12b. `git_change.change_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    /// The path is in this commit's tree and not in the parent's.
    Added,
    /// The path is in both trees with different content.
    Modified,
    /// The path is in the parent's tree and not in this commit's.
    Deleted,
    /// The path is in both trees with the same content and a different file mode.
    ModeChanged,
}

impl ChangeKind {
    /// Every change kind, in declaration order.
    pub const ALL: [ChangeKind; 4] = [
        ChangeKind::Added,
        ChangeKind::Modified,
        ChangeKind::Deleted,
        ChangeKind::ModeChanged,
    ];

    /// Canonical lower-case name, stored in `git_change.change_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::ModeChanged => "mode_changed",
        }
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChangeKind {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChangeKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| NerveError::unknown("ChangeKind", s))
    }
}

/// Why a commit has the parents it has — and, when it has none visible, *which* reason that is.
///
/// This is the vocabulary the historical model exists to get right. Two of its members mean
/// "cannot see further" and they are kept apart because one is declared and expected while the
/// other is a fault: collapsing them would report a corrupt repository as a shallow one, and
/// collapsing either into [`ParentCompleteness::Root`] would state "the project's history begins
/// here" about a checkout that merely cannot see past its own boundary.
///
/// See `docs/plans/slice-12b-historical-model.md` §5.1 for the derivation of each member and
/// [`ParentCompleteness::may_claim_history_begins_here`] for the one consequence that must not be
/// got wrong. Added in Slice 12b. `git_commit.parent_completeness`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParentCompleteness {
    /// No parents in the commit object, **and** not a shallow boundary. This is the beginning.
    Root,
    /// Listed in `.git/shallow`. The commit object may name parents; they are absent by
    /// declaration, so "earliest commit visible in this checkout" is all that may be said.
    ShallowBoundary,
    /// Has parents, all present in the object store.
    ParentsAvailable,
    /// Has parents, at least one absent, and the shallow declaration was read cleanly and does
    /// not list this commit. Promisor, or corrupt — unexpected, and distinct from shallow.
    ParentsMissing,
    /// Has parents, at least one absent, and Nerve **could not establish** whether that absence
    /// was declared.
    ///
    /// Exists because `.git/shallow` can be present and unreadable, over a size bound, or have a
    /// line dropped while the rest is kept — so "not shallow" and "shallow, but we could not
    /// tell" are different facts. Without this member the undecidable case would be labelled
    /// [`ParentCompleteness::ParentsMissing`], which reports a shallow repository as corrupt.
    /// It may not be called corrupt and it may not be called shallow.
    ParentsUnverifiable,
}

impl ParentCompleteness {
    /// Every value, in declaration order.
    pub const ALL: [ParentCompleteness; 5] = [
        ParentCompleteness::Root,
        ParentCompleteness::ShallowBoundary,
        ParentCompleteness::ParentsAvailable,
        ParentCompleteness::ParentsMissing,
        ParentCompleteness::ParentsUnverifiable,
    ];

    /// Canonical lower-case name, stored in `git_commit.parent_completeness`.
    pub fn as_str(self) -> &'static str {
        match self {
            ParentCompleteness::Root => "root",
            ParentCompleteness::ShallowBoundary => "shallow_boundary",
            ParentCompleteness::ParentsAvailable => "parents_available",
            ParentCompleteness::ParentsMissing => "parents_missing",
            ParentCompleteness::ParentsUnverifiable => "parents_unverifiable",
        }
    }

    /// May a consumer state that the project's history begins at this commit?
    ///
    /// **True for [`ParentCompleteness::Root`] and nothing else.** This is stated as a method
    /// rather than left in prose because it is the single claim the historical model is built to
    /// avoid making wrongly, and because a value added to [`ParentCompleteness::ALL`] without an
    /// answer here would default to whatever a `matches!` happened to say — a default, not a
    /// decision, which is the drift `EntityKind::path_role` was written for.
    ///
    /// A commit with available parents answers `false` for the plain reason that its history
    /// demonstrably does not begin there; the two boundary values and the undecidable one answer
    /// `false` because the absence in front of them is an absence of *visibility*.
    pub fn may_claim_history_begins_here(self) -> bool {
        matches!(self, ParentCompleteness::Root)
    }

    /// What this commit's parent situation means, and whether "history begins here" may be said.
    ///
    /// **The permitted case is taken from [`ParentCompleteness::may_claim_history_begins_here`] and
    /// is never re-derived here.** That method is true for [`ParentCompleteness::Root`] and nothing
    /// else; a `matches!` written out again in a renderer would be a second copy of the one rule
    /// this vocabulary exists to get right, free to drift from the first.
    ///
    /// This lives in `nerve-core`, beside the rule it renders, because Slice 12b left it inside the
    /// CLI binary and three further surfaces were about to each write their own — and a surface that
    /// re-words [`ParentCompleteness::ShallowBoundary`] slightly is a surface that has restated the
    /// invariant the historical model exists to protect. `crates/nerve-cli/tests/history_wording.rs`
    /// enforces the single copy by scanning for this prose outside this crate.
    pub fn note(self) -> &'static str {
        if self.may_claim_history_begins_here() {
            return "no parents in the commit object and no shallow boundary, so the project's \
                    history begins here";
        }
        match self {
            ParentCompleteness::Root => unreachable!("root is the permitted case above"),
            ParentCompleteness::ParentsAvailable => "every parent this commit names is present",
            ParentCompleteness::ShallowBoundary => {
                "earliest commit visible in this checkout; history before this point is \
                 unavailable to this repository"
            }
            ParentCompleteness::ParentsMissing => {
                "a parent this commit names is absent and was not declared absent, a fault in \
                 this repository rather than a shallow boundary; history before this point is \
                 unavailable to this repository"
            }
            ParentCompleteness::ParentsUnverifiable => {
                "a parent this commit names is absent and Nerve could not establish whether the \
                 absence was declared, so neither shallow nor corrupt may be asserted"
            }
        }
    }
}

impl fmt::Display for ParentCompleteness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ParentCompleteness {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ParentCompleteness::ALL
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| NerveError::unknown("ParentCompleteness", s))
    }
}

/// Whether a commit's changes were enumerated, and if not, why not.
///
/// A commit with zero `git_change` rows is never ambiguous: this value says which of the four
/// reasons it is. That is the whole purpose of the column — "no rows" is a fact about Nerve's
/// reading of the commit, not about the commit, and inferring one from the other is how an
/// unreadable parent becomes "nothing changed".
///
/// Added in Slice 12b. `git_commit.changes_enumerated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangesEnumerated {
    /// The tree diff against the single parent ran to completion. Zero rows means the commit
    /// genuinely changed nothing.
    Enumerated,
    /// A merge. Change enumeration is defined against one parent and a merge has several, so
    /// nothing was enumerated by decision rather than by failure.
    MergeNotEnumerated,
    /// The parent tree could not be read — a shallow boundary, or a missing object — so there was
    /// nothing to diff against. Not a claim that every path in this tree is new.
    ParentUnavailable,
    /// A bound was hit while enumerating. The count is in `git_history_ingest.refusals`.
    Refused,
}

impl ChangesEnumerated {
    /// Every value, in declaration order.
    pub const ALL: [ChangesEnumerated; 4] = [
        ChangesEnumerated::Enumerated,
        ChangesEnumerated::MergeNotEnumerated,
        ChangesEnumerated::ParentUnavailable,
        ChangesEnumerated::Refused,
    ];

    /// Canonical lower-case name, stored in `git_commit.changes_enumerated`.
    pub fn as_str(self) -> &'static str {
        match self {
            ChangesEnumerated::Enumerated => "enumerated",
            ChangesEnumerated::MergeNotEnumerated => "merge_not_enumerated",
            ChangesEnumerated::ParentUnavailable => "parent_unavailable",
            ChangesEnumerated::Refused => "refused",
        }
    }

    /// Which of the four silences a commit with no change rows is, in words.
    ///
    /// Printing a count without this is the defect the column exists to prevent: `0` alone reads as
    /// "nothing changed" in all four cases, and only [`ChangesEnumerated::Enumerated`] means that.
    /// Hoisted out of the CLI in Slice 12c-i so that every surface says the same sentence.
    pub fn note(self) -> &'static str {
        match self {
            ChangesEnumerated::Enumerated => {
                "the diff against the single parent ran to completion, so a zero here really is \
                 \"nothing changed\""
            }
            ChangesEnumerated::MergeNotEnumerated => {
                "a merge has several parents and a change is only defined against one, so none \
                 were enumerated — not an empty commit"
            }
            ChangesEnumerated::ParentUnavailable => {
                "the parent tree could not be read, so nothing was enumerated — not an empty commit"
            }
            ChangesEnumerated::Refused => {
                "a bound refused this commit's diff, so nothing was enumerated — see the refusals \
                 above"
            }
        }
    }
}

impl fmt::Display for ChangesEnumerated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChangesEnumerated {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChangesEnumerated::ALL
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| NerveError::unknown("ChangesEnumerated", s))
    }
}

/// Why the history walk stopped.
///
/// A concept distinct from history *availability*: [`WalkTermination::CommitBudget`] means the
/// history was present on disk and Nerve declined to read all of it. It is the one boundary
/// reason that is Nerve's own doing, and a derived "first observed" must be qualified by it as
/// well as by shallow state or a bounded ingest silently becomes a claim about the project's
/// origin.
///
/// Added in Slice 12b. `git_history_ingest.walk_terminated_by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WalkTermination {
    /// No unvisited parent remained. The walk saw everything reachable from its tips.
    Exhausted,
    /// The commit budget was reached. Nerve stopped; the history did not.
    CommitBudget,
    /// A declared shallow boundary was reached.
    ShallowBoundary,
    /// A parent object was absent from the store.
    MissingObject,
    /// A refusal — a bound or an unreadable object — stopped the walk. Counted, never silent.
    Refused,
}

impl WalkTermination {
    /// Every value, in declaration order.
    pub const ALL: [WalkTermination; 5] = [
        WalkTermination::Exhausted,
        WalkTermination::CommitBudget,
        WalkTermination::ShallowBoundary,
        WalkTermination::MissingObject,
        WalkTermination::Refused,
    ];

    /// Canonical lower-case name, stored in `git_history_ingest.walk_terminated_by`.
    pub fn as_str(self) -> &'static str {
        match self {
            WalkTermination::Exhausted => "exhausted",
            WalkTermination::CommitBudget => "commit_budget",
            WalkTermination::ShallowBoundary => "shallow_boundary",
            WalkTermination::MissingObject => "missing_object",
            WalkTermination::Refused => "refused",
        }
    }

    /// Why the walk stopped, in words, with Nerve's own boundary kept apart from the repository's.
    ///
    /// Hoisted out of the CLI in Slice 12c-i. [`WalkTermination::CommitBudget`] is the sentence that
    /// matters: it is Nerve declining to read further and says nothing whatever about how far the
    /// repository goes back, so a surface that re-worded it could turn a bound into a claim about
    /// the project's origin.
    pub fn note(self) -> &'static str {
        match self {
            WalkTermination::Exhausted => "every commit reachable from the walked tips was read",
            // The distinction this whole surface exists for. `commit_budget` is Nerve declining to
            // read further; it says nothing whatever about how far the repository goes back.
            WalkTermination::CommitBudget => {
                "Nerve stopped at its own commit budget, so the repository has more history than \
                 this ingest read"
            }
            WalkTermination::ShallowBoundary => {
                "the walk reached a declared shallow boundary; history before it is unavailable to \
                 this repository"
            }
            WalkTermination::MissingObject => {
                "an object the walk needed was absent; a fault in this repository, not a declared \
                 boundary"
            }
            WalkTermination::Refused => "a bound refused an object the walk needed",
        }
    }
}

impl fmt::Display for WalkTermination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WalkTermination {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        WalkTermination::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| NerveError::unknown("WalkTermination", s))
    }
}

/// What a rename hypothesis rests on.
///
/// **Slice 12b has exactly one member.** [`RenameEvidence::ExactContent`] means a path was
/// deleted and another added in the same commit with the *same blob oid* — the oids were already
/// in hand from the tree diff, so there is no similarity computation and no threshold.
///
/// **Slice 12c adds `SimilarContent` as a second value of this vocabulary, and the two are never
/// blended into a score.** Content similarity is a different kind of evidence, not a weaker
/// amount of this one: a blended number would let an exact match and a 60%-similar match arrive
/// at the same figure and become indistinguishable. Which evidence a hypothesis has, and how
/// ambiguous the pairing is ([`RenameAmbiguity`]), are separate columns because they are separate
/// facts.
///
/// Added in Slice 12b. `git_rename_hypothesis.evidence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenameEvidence {
    /// The deleted path and the added path name the same blob oid. Byte-identical content.
    ExactContent,
}

impl RenameEvidence {
    /// Every value, in declaration order.
    pub const ALL: [RenameEvidence; 1] = [RenameEvidence::ExactContent];

    /// Canonical lower-case name, stored in `git_rename_hypothesis.evidence`.
    pub fn as_str(self) -> &'static str {
        match self {
            RenameEvidence::ExactContent => "exact_content",
        }
    }
}

impl fmt::Display for RenameEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RenameEvidence {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RenameEvidence::ALL
            .into_iter()
            .find(|e| e.as_str() == s)
            .ok_or_else(|| NerveError::unknown("RenameEvidence", s))
    }
}

/// How many ways a rename hypothesis could have been drawn.
///
/// This is the point of the hypothesis being a hypothesis. Files with identical content — an
/// empty file, a copied licence header, a re-exported barrel — split and merge constantly, so
/// when one deleted blob matches several added paths **every pairing is recorded and none is
/// promoted**. There is no threshold, no tie-break, and no single confidence number.
///
/// Added in Slice 12b. `git_rename_hypothesis.ambiguity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenameAmbiguity {
    /// One deleted path, one added path, one blob. The only unambiguous shape.
    Unique,
    /// Several deleted paths share this blob with one added path.
    ManyFrom,
    /// One deleted path shares this blob with several added paths.
    ManyTo,
    /// Several on both sides.
    ManyBoth,
}

impl RenameAmbiguity {
    /// Every value, in declaration order.
    pub const ALL: [RenameAmbiguity; 4] = [
        RenameAmbiguity::Unique,
        RenameAmbiguity::ManyFrom,
        RenameAmbiguity::ManyTo,
        RenameAmbiguity::ManyBoth,
    ];

    /// Canonical lower-case name, stored in `git_rename_hypothesis.ambiguity`.
    pub fn as_str(self) -> &'static str {
        match self {
            RenameAmbiguity::Unique => "unique",
            RenameAmbiguity::ManyFrom => "many_from",
            RenameAmbiguity::ManyTo => "many_to",
            RenameAmbiguity::ManyBoth => "many_both",
        }
    }

    /// How ambiguous a rename hypothesis is, in words. There is no score, and none is invented.
    ///
    /// Hoisted out of the CLI in Slice 12c-i. Every value's sentence says that no pairing is
    /// promoted, including [`RenameAmbiguity::Unique`], which is still a hypothesis.
    pub fn note(self) -> &'static str {
        match self {
            RenameAmbiguity::Unique => {
                "one deleted path, one added path, one blob — the only unambiguous shape, and \
                 still a hypothesis"
            }
            RenameAmbiguity::ManyFrom => {
                "several deleted paths share this blob — every pairing is recorded and none is \
                 promoted"
            }
            RenameAmbiguity::ManyTo => {
                "several added paths share this blob — every pairing is recorded and none is \
                 promoted"
            }
            RenameAmbiguity::ManyBoth => {
                "several paths on both sides share this blob — every pairing is recorded and none \
                 is promoted"
            }
        }
    }
}

impl fmt::Display for RenameAmbiguity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RenameAmbiguity {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RenameAmbiguity::ALL
            .into_iter()
            .find(|a| a.as_str() == s)
            .ok_or_else(|| NerveError::unknown("RenameAmbiguity", s))
    }
}

/// What "when was this path first observed" actually answers — and whether *created* is one of them.
///
/// The earliest `git_change` row for a path is **not** when the path was created; it is the earliest
/// change Nerve can see. Reporting one as the other is the defect
/// [`ParentCompleteness::may_claim_history_begins_here`] exists to prevent, one query layer up. Six
/// values, and exactly one of them may be rendered as creation — see
/// [`FirstObservedKind::may_claim_created`].
///
/// The last three values are the ones a happy-path draft omits, and each is a different fact:
///
/// - [`FirstObservedKind::PresentBeforeVisibleHistory`] is the **common** case on a shallow clone,
///   where every unchanged file has zero change rows. Without it the answer is an empty result,
///   which reads as "this file has no history".
/// - [`FirstObservedKind::CurrentTreeUnknown`] exists because `nerve history sync` requires only
///   `nerve init`, not an index. Telling "exists now, never changed" from "does not exist now"
///   requires knowing the current tree, and the only thing that knows it is the entity table; with
///   no index Nerve genuinely cannot tell them apart, and collapsing them either way is a claim it
///   has no evidence for.
/// - [`FirstObservedKind::NoHistoryIngested`] is the absence of a `git_history_ingest` row, which is
///   not a failure and not "this project has no history".
///
/// Derived in Slice 12c-i. Stored nowhere: this vocabulary exists in responses only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FirstObservedKind {
    /// The path was created at the change Nerve can see, and the claim rests on no date.
    ///
    /// Three facts, all established by the store layer and none of them a timestamp: the earliest
    /// recorded change is an **addition**, so the path was absent from the tree it was diffed
    /// against; **nothing is hidden above it**, which is reachable only when the walk ran out of
    /// commits rather than stopping at a boundary, a missing object, a refusal or Nerve's own budget;
    /// and **exactly one addition is recorded** for the path, so it was created once — a path
    /// created, deleted and re-created records two.
    ///
    /// The third fact is what makes this clock-independent, and it exists because the second alone is
    /// not enough: change order is `committer_time` order, which a rebase or a fabricated clock can
    /// reorder freely, so "the earliest *dated* change is an addition" does not establish that it is
    /// the topologically first one.
    ///
    /// One residual, reported as data rather than hidden in prose: a path created inside one merge
    /// and deleted inside another has both events unrecorded, because 12b enumerates no changes for a
    /// merge. The response carries the repository's merge count so a consumer can see whether that
    /// possibility exists at all.
    CreatedInVisibleHistory,
    /// Changes exist, and this is the earliest one Nerve can see. It may or may not be the first, and
    /// the response always names which of the five reasons puts history above it out of reach.
    EarliestVisibleChange,
    /// Zero change rows, and the path is an indexed entity: it exists now and was never touched in
    /// visible history.
    PresentBeforeVisibleHistory,
    /// Zero change rows, the path is not an indexed entity, and an index exists — so the current
    /// tree was consulted and does not contain it.
    AbsentFromVisibleHistory,
    /// Zero change rows and **no index**, so the current tree could not be consulted at all.
    CurrentTreeUnknown,
    /// No `git_history_ingest` row: history has never been read here.
    NoHistoryIngested,
}

impl FirstObservedKind {
    /// Every value, in declaration order.
    pub const ALL: [FirstObservedKind; 6] = [
        FirstObservedKind::CreatedInVisibleHistory,
        FirstObservedKind::EarliestVisibleChange,
        FirstObservedKind::PresentBeforeVisibleHistory,
        FirstObservedKind::AbsentFromVisibleHistory,
        FirstObservedKind::CurrentTreeUnknown,
        FirstObservedKind::NoHistoryIngested,
    ];

    /// Canonical lower-case name, carried in every response that reports a first-observed answer.
    pub fn as_str(self) -> &'static str {
        match self {
            FirstObservedKind::CreatedInVisibleHistory => "created_in_visible_history",
            FirstObservedKind::EarliestVisibleChange => "earliest_visible_change",
            FirstObservedKind::PresentBeforeVisibleHistory => "present_before_visible_history",
            FirstObservedKind::AbsentFromVisibleHistory => "absent_from_visible_history",
            FirstObservedKind::CurrentTreeUnknown => "current_tree_unknown",
            FirstObservedKind::NoHistoryIngested => "no_history_ingested",
        }
    }

    /// May a consumer render this answer as *"the path was created then"*?
    ///
    /// **True for [`FirstObservedKind::CreatedInVisibleHistory`] and nothing else, and this is the
    /// only copy of that permission in the workspace.** It is a method rather than prose for the
    /// same reason [`ParentCompleteness::may_claim_history_begins_here`] is: it is the single claim
    /// the derived history layer is built to avoid making wrongly, and a seventh value added to
    /// [`FirstObservedKind::ALL`] without an answer here would inherit whatever a `matches!`
    /// happened to say — a default, not a decision.
    ///
    /// [`FirstObservedKind::EarliestVisibleChange`] answers `false` even when the change is an
    /// `added` row, because an addition seen above an unavailable parent is an addition *to what
    /// Nerve can see*: the five reasons visible history stops — a shallow boundary, a missing parent,
    /// an unverifiable one, Nerve's own commit budget, and Nerve's own refusal of an object the walk
    /// needed — each mean an earlier row may exist and not be recorded. It also answers `false` for a
    /// second, clock-shaped reason: a path with more than one recorded addition was created more than
    /// once, and which addition came first is a question about ancestry that `committer_time` order
    /// cannot answer.
    pub fn may_claim_created(self) -> bool {
        matches!(self, FirstObservedKind::CreatedInVisibleHistory)
    }

    /// What this answer means, in one sentence, with the creation permission as the gate.
    ///
    /// **The permission is read from [`FirstObservedKind::may_claim_created`], never re-derived**,
    /// which is what makes "created" impossible to say about the other five answers without deleting
    /// a branch. The mirror arm below is [`unreachable`] for the same reason
    /// [`ParentCompleteness::note`]'s is: the permitted case is handled once, above the match.
    ///
    /// Hoisted out of the CLI binary in Slice 12c-iii-a. Slice 12c-i-b was forbidden from editing
    /// this crate and so wrote the prose inside `nerve-cli`, recording that three further surfaces
    /// were about to copy it. `crates/nerve-cli/tests/history_wording.rs` scans for this text outside
    /// this crate and fails a copy by name.
    pub fn note(self) -> &'static str {
        if self.may_claim_created() {
            return "the path was created at this change: the earliest recorded change is an \
                    addition, nothing above it is hidden, and exactly one addition is recorded, so \
                    the claim rests on no clock";
        }
        match self {
            FirstObservedKind::CreatedInVisibleHistory => {
                unreachable!("creation is the permitted case above")
            }
            FirstObservedKind::EarliestVisibleChange => {
                "the earliest change Nerve can see, which is not established as the first one; the \
                 reason history above it is out of reach is named below"
            }
            FirstObservedKind::PresentBeforeVisibleHistory => {
                "present before visible history: the path is in the current tree and no recorded \
                 commit touched it, so it predates everything Nerve read"
            }
            FirstObservedKind::AbsentFromVisibleHistory => {
                "absent from visible history: no recorded commit touched it, and the current tree \
                 was consulted and does not hold it"
            }
            FirstObservedKind::CurrentTreeUnknown => {
                "no recorded commit touched it, and with no index the current tree could not be \
                 consulted at all, so whether it exists now is unknown rather than no"
            }
            FirstObservedKind::NoHistoryIngested => {
                "history has never been read here, which is not the same fact as a path with no \
                 history"
            }
        }
    }

    /// Whether the word *created* may be used of this answer, in words.
    ///
    /// The gate is [`FirstObservedKind::may_claim_created`] and the refusals say *why* rather than
    /// repeating one sentence: a path with changes is refused for an ordering reason, a path without
    /// any is refused because there is nothing that could be a creation, and a repository with no
    /// ingest is refused because nothing was read at all. One sentence for all three would have said
    /// something false about two of them.
    ///
    /// Hoisted with [`FirstObservedKind::note`] and for the same reason: it is prose gated on the
    /// one permission this vocabulary exists to hold, and every surface that renders the answer
    /// renders it.
    pub fn created_claim_note(self) -> &'static str {
        if self.may_claim_created() {
            return "permitted — this is the one answer of six that licenses it";
        }
        match self {
            FirstObservedKind::CreatedInVisibleHistory => {
                unreachable!("creation is the permitted case above")
            }
            FirstObservedKind::EarliestVisibleChange => {
                "not permitted — the earliest recorded change is not established as the first one, \
                 so this answer may only be rendered as the earliest change Nerve can see"
            }
            FirstObservedKind::PresentBeforeVisibleHistory
            | FirstObservedKind::AbsentFromVisibleHistory
            | FirstObservedKind::CurrentTreeUnknown => {
                "not permitted — no change to this path is recorded at all, so there is nothing \
                 here that could be a creation"
            }
            FirstObservedKind::NoHistoryIngested => {
                "not permitted — no history has been read here, so no claim about this path can be \
                 made either way"
            }
        }
    }
}

impl fmt::Display for FirstObservedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FirstObservedKind {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FirstObservedKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| NerveError::unknown("FirstObservedKind", s))
    }
}

/// Whether the recorded history still describes the repository's current HEAD.
///
/// `git_history_ingest.head_oid` against the current `repository_state.git_commit`. Four verdicts,
/// and [`HistoryFreshness::Unverifiable`] is not a cosmetic fourth: reporting *unknown* as
/// *current* is how a truncated sweep becomes a clean bill of health, which is the distinction
/// `nerve check` already draws between `Freshness::Stale` and `Freshness::Unverified` (Slice 7c-i).
/// A history whose freshness cannot be established has not been shown to be fresh.
///
/// [`HistoryFreshness::Stale`] is a qualification rather than an error. The recorded facts are true
/// of an older HEAD, which is exactly what makes a `last_observed` answer bounded above by the
/// ingest rather than by the repository.
///
/// Derived in Slice 12c-i. Stored nowhere: this vocabulary exists in responses only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HistoryFreshness {
    /// The ingest's HEAD equals the current repository state's commit.
    Current,
    /// They differ. The recorded history describes an older HEAD; both oids are named.
    Stale,
    /// The current repository state records no commit, so the comparison cannot be made. **Not
    /// [`HistoryFreshness::Current`].**
    Unverifiable,
    /// No `git_history_ingest` row. There is nothing whose freshness could be judged.
    NoHistoryIngested,
}

impl HistoryFreshness {
    /// Every value, in declaration order.
    pub const ALL: [HistoryFreshness; 4] = [
        HistoryFreshness::Current,
        HistoryFreshness::Stale,
        HistoryFreshness::Unverifiable,
        HistoryFreshness::NoHistoryIngested,
    ];

    /// Canonical lower-case name, carried in every response that reports freshness.
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryFreshness::Current => "current",
            HistoryFreshness::Stale => "stale",
            HistoryFreshness::Unverifiable => "unverifiable",
            HistoryFreshness::NoHistoryIngested => "no_history_ingested",
        }
    }

    /// What each verdict means, in words.
    ///
    /// [`HistoryFreshness::Unverifiable`] gets the longest sentence because it is the one a reader is
    /// most likely to file under [`HistoryFreshness::Current`], and that filing is how a truncated
    /// sweep becomes a clean bill of health.
    ///
    /// Hoisted out of the CLI binary in Slice 12c-iii-a, beside the vocabulary it renders, so the
    /// HTTP, MCP and UI surfaces say the same four sentences rather than four paraphrases each.
    pub fn note(self) -> &'static str {
        match self {
            HistoryFreshness::Current => {
                "the ingest's HEAD is the commit the newest indexed state records, so the recorded \
                 history describes what is indexed now"
            }
            HistoryFreshness::Stale => {
                "the ingest's HEAD is not the commit the newest indexed state records: every \
                 historical fact here is true of the older HEAD, which is a qualification rather \
                 than an error"
            }
            HistoryFreshness::Unverifiable => {
                "the newest indexed state records no commit, so the comparison could not be made. \
                 This is not \"current\": a history whose freshness cannot be established has not \
                 been shown to be fresh, and reporting unknown as current is how a truncated sweep \
                 becomes a clean bill of health"
            }
            HistoryFreshness::NoHistoryIngested => {
                "history has never been read here, so there is nothing whose freshness could be \
                 judged — an absence, and not a failure"
            }
        }
    }
}

impl fmt::Display for HistoryFreshness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for HistoryFreshness {
    type Err = NerveError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        HistoryFreshness::ALL
            .into_iter()
            .find(|f| f.as_str() == s)
            .ok_or_else(|| NerveError::unknown("HistoryFreshness", s))
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

    // ---- Slice 12b: the historical vocabularies -----------------------------------------------

    /// Every change kind's canonical name, pinned one by one, and the one name that must not exist.
    ///
    /// `git_change.change_kind` is `TEXT`, so the vocabulary is closed in Rust and nowhere else: a
    /// variant added without a name here would be stored as whatever `as_str` happened to return.
    /// `renamed` is refused deliberately — Git records no rename, so a change kind saying otherwise
    /// would state as fact the one thing about history that is inferred.
    #[test]
    fn every_change_kind_states_its_canonical_name() {
        let pinned: [(ChangeKind, &str); 4] = [
            (ChangeKind::Added, "added"),
            (ChangeKind::Modified, "modified"),
            (ChangeKind::Deleted, "deleted"),
            // A tracked file whose bytes are identical and whose mode moved. Its own value, so
            // that neither `modified` over-claims nor an omission under-reports the commit.
            (ChangeKind::ModeChanged, "mode_changed"),
        ];

        for (kind, name) in pinned {
            assert_eq!(kind.as_str(), name, "{kind:?} is pinned against this list");
            assert_eq!(name.parse::<ChangeKind>().unwrap(), kind);
            assert_eq!(kind.to_string(), name);
        }

        let mut listed: Vec<ChangeKind> = pinned.iter().map(|(kind, _)| *kind).collect();
        listed.sort_unstable();
        let mut all = ChangeKind::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a change kind was added to the vocabulary without a canonical name above"
        );

        for invented in ["renamed", "copied", "unmerged", "type_changed", ""] {
            assert!(
                invented.parse::<ChangeKind>().is_err(),
                "{invented:?} parsed as a ChangeKind without a diff rule that produces it"
            );
        }
    }

    /// Every parent-completeness value, and the one consequence each is allowed to have.
    ///
    /// The second column is the claim this whole vocabulary exists to control. Exactly one value
    /// permits *"the project's history begins here"*, and a value added to
    /// [`ParentCompleteness::ALL`] without an answer here fails this test rather than inheriting
    /// one — the same reason `EntityKind::path_role` is pinned kind by kind.
    #[test]
    fn every_parent_completeness_states_whether_history_may_begin_there() {
        let pinned: [(ParentCompleteness, &str, bool); 5] = [
            // The beginning, and the only value that is.
            (ParentCompleteness::Root, "root", true),
            // Declared and expected: "earliest commit visible in this checkout".
            (
                ParentCompleteness::ShallowBoundary,
                "shallow_boundary",
                false,
            ),
            // Its history is demonstrably in front of it.
            (
                ParentCompleteness::ParentsAvailable,
                "parents_available",
                false,
            ),
            // A fault, not a boundary. Never to be reported as shallow.
            (ParentCompleteness::ParentsMissing, "parents_missing", false),
            // Undecidable. Neither shallow nor corrupt may be asserted.
            (
                ParentCompleteness::ParentsUnverifiable,
                "parents_unverifiable",
                false,
            ),
        ];

        for (value, name, may_claim) in pinned {
            assert_eq!(
                value.as_str(),
                name,
                "{value:?} is pinned against this list"
            );
            assert_eq!(name.parse::<ParentCompleteness>().unwrap(), value);
            assert_eq!(value.to_string(), name);
            assert_eq!(
                value.may_claim_history_begins_here(),
                may_claim,
                "{name} changed what a consumer may claim about the start of history"
            );
        }

        let mut listed: Vec<ParentCompleteness> =
            pinned.iter().map(|(value, _, _)| *value).collect();
        listed.sort_unstable();
        let mut all = ParentCompleteness::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a parent-completeness value was added without stating what it permits"
        );

        assert_eq!(
            ParentCompleteness::ALL
                .iter()
                .filter(|c| c.may_claim_history_begins_here())
                .count(),
            1,
            "exactly one value means the history genuinely begins here"
        );

        // The three ways this vocabulary would be wrong if it were collapsed. A shallow boundary
        // is not a root, a fault is not a boundary, and an undecidable case is neither.
        assert_ne!(
            ParentCompleteness::Root,
            ParentCompleteness::ShallowBoundary
        );
        assert_ne!(
            ParentCompleteness::ShallowBoundary,
            ParentCompleteness::ParentsMissing
        );
        assert_ne!(
            ParentCompleteness::ParentsUnverifiable,
            ParentCompleteness::ParentsMissing
        );

        for invented in ["shallow", "corrupt", "orphan", "unknown", ""] {
            assert!(
                invented.parse::<ParentCompleteness>().is_err(),
                "{invented:?} parsed as a ParentCompleteness"
            );
        }
    }

    /// Every reason a commit can have zero change rows, pinned one by one.
    ///
    /// The vocabulary is what stops "no rows" being read as "nothing changed", so the names that
    /// would reintroduce that ambiguity — `empty`, `none`, `no_changes` — must not parse.
    #[test]
    fn every_changes_enumerated_value_states_which_silence_it_is() {
        let pinned: [(ChangesEnumerated, &str); 4] = [
            // Zero rows here, and only here, means the commit changed nothing.
            (ChangesEnumerated::Enumerated, "enumerated"),
            (
                ChangesEnumerated::MergeNotEnumerated,
                "merge_not_enumerated",
            ),
            (ChangesEnumerated::ParentUnavailable, "parent_unavailable"),
            (ChangesEnumerated::Refused, "refused"),
        ];

        for (value, name) in pinned {
            assert_eq!(
                value.as_str(),
                name,
                "{value:?} is pinned against this list"
            );
            assert_eq!(name.parse::<ChangesEnumerated>().unwrap(), value);
            assert_eq!(value.to_string(), name);
        }

        let mut listed: Vec<ChangesEnumerated> = pinned.iter().map(|(value, _)| *value).collect();
        listed.sort_unstable();
        let mut all = ChangesEnumerated::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a changes-enumerated value was added to the vocabulary without a name above"
        );

        // A merge with no rows and an empty commit with no rows are different facts.
        assert_ne!(
            ChangesEnumerated::MergeNotEnumerated,
            ChangesEnumerated::Enumerated
        );
        // An unreadable parent and a completed diff are different facts.
        assert_ne!(
            ChangesEnumerated::ParentUnavailable,
            ChangesEnumerated::Enumerated
        );

        for invented in ["empty", "none", "no_changes", "unknown", ""] {
            assert!(
                invented.parse::<ChangesEnumerated>().is_err(),
                "{invented:?} parsed as a ChangesEnumerated and would restore the ambiguity"
            );
        }
    }

    /// Every walk-termination reason, pinned one by one.
    ///
    /// `commit_budget` is the member that makes this vocabulary worth having: it is Nerve's own
    /// decision to stop reading, and it must never be confused with the repository being unable to
    /// go further. `complete` is refused because it would read as a claim about the history rather
    /// than about the walk.
    #[test]
    fn every_walk_termination_reason_states_who_stopped_the_walk() {
        let pinned: [(WalkTermination, &str); 5] = [
            (WalkTermination::Exhausted, "exhausted"),
            // Nerve stopped. The history did not.
            (WalkTermination::CommitBudget, "commit_budget"),
            (WalkTermination::ShallowBoundary, "shallow_boundary"),
            (WalkTermination::MissingObject, "missing_object"),
            (WalkTermination::Refused, "refused"),
        ];

        for (value, name) in pinned {
            assert_eq!(
                value.as_str(),
                name,
                "{value:?} is pinned against this list"
            );
            assert_eq!(name.parse::<WalkTermination>().unwrap(), value);
            assert_eq!(value.to_string(), name);
        }

        let mut listed: Vec<WalkTermination> = pinned.iter().map(|(value, _)| *value).collect();
        listed.sort_unstable();
        let mut all = WalkTermination::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a walk-termination reason was added to the vocabulary without a name above"
        );

        // A bounded ingest of a complete repository is not a shallow one.
        assert_ne!(
            WalkTermination::CommitBudget,
            WalkTermination::ShallowBoundary
        );
        assert_ne!(WalkTermination::CommitBudget, WalkTermination::Exhausted);

        for invented in ["complete", "budget", "error", "truncated", ""] {
            assert!(
                invented.parse::<WalkTermination>().is_err(),
                "{invented:?} parsed as a WalkTermination"
            );
        }
    }

    /// The rename vocabulary claims exactly what 12b's evidence carries, and no more.
    ///
    /// `similar_content` is a plausible future member and **must not parse until Slice 12c writes
    /// the rule that produces it** — the same discipline `EndpointKind` applies to `cli_command`.
    /// Evidence and ambiguity are two vocabularies rather than one number, because "what this rests
    /// on" and "how many ways it could have been drawn" are separate facts and a single score
    /// would make an exact match indistinguishable from a similar one.
    #[test]
    fn the_rename_vocabulary_keeps_evidence_and_ambiguity_apart() {
        assert_eq!(RenameEvidence::ALL.len(), 1);
        assert_eq!(RenameEvidence::ExactContent.as_str(), "exact_content");
        for evidence in RenameEvidence::ALL {
            assert_eq!(
                evidence.as_str().parse::<RenameEvidence>().unwrap(),
                evidence
            );
            assert_eq!(evidence.to_string(), evidence.as_str());
        }
        for not_yet in ["similar_content", "similarity", "content_similarity", ""] {
            assert!(
                not_yet.parse::<RenameEvidence>().is_err(),
                "{not_yet:?} parsed as a RenameEvidence without a rule that emits it"
            );
        }

        let pinned: [(RenameAmbiguity, &str); 4] = [
            // The only unambiguous shape.
            (RenameAmbiguity::Unique, "unique"),
            (RenameAmbiguity::ManyFrom, "many_from"),
            (RenameAmbiguity::ManyTo, "many_to"),
            (RenameAmbiguity::ManyBoth, "many_both"),
        ];
        for (value, name) in pinned {
            assert_eq!(
                value.as_str(),
                name,
                "{value:?} is pinned against this list"
            );
            assert_eq!(name.parse::<RenameAmbiguity>().unwrap(), value);
            assert_eq!(value.to_string(), name);
        }
        let mut listed: Vec<RenameAmbiguity> = pinned.iter().map(|(value, _)| *value).collect();
        listed.sort_unstable();
        let mut all = RenameAmbiguity::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a rename-ambiguity value was added to the vocabulary without a name above"
        );

        // Three of the four values mean "do not promote this pairing".
        assert_eq!(
            RenameAmbiguity::ALL
                .iter()
                .filter(|a| **a != RenameAmbiguity::Unique)
                .count(),
            3
        );

        // A score is exactly what this design does not have. No name suggesting one may parse as
        // either half of it.
        for scored in ["confidence", "score", "probable", "likely"] {
            assert!(scored.parse::<RenameEvidence>().is_err());
            assert!(scored.parse::<RenameAmbiguity>().is_err());
        }
    }

    // ---- Slice 12c-i: the derived historical vocabularies --------------------------------------

    /// Every first-observed value, and the one consequence each is allowed to have.
    ///
    /// The third column is the claim this vocabulary exists to control. Exactly one value permits
    /// *"the path was created then"*, and a value added to [`FirstObservedKind::ALL`] without an
    /// answer here fails this test rather than inheriting one — the same discipline
    /// `every_parent_completeness_states_whether_history_may_begin_there` applies one layer down.
    #[test]
    fn every_first_observed_kind_states_whether_creation_may_be_claimed() {
        let pinned: [(FirstObservedKind, &str, bool); 6] = [
            // The only value that may say "created", and it borrows its licence from
            // `ParentCompleteness::may_claim_history_begins_here`.
            (
                FirstObservedKind::CreatedInVisibleHistory,
                "created_in_visible_history",
                true,
            ),
            // An `added` row above an unavailable parent is an addition to what Nerve can see.
            (
                FirstObservedKind::EarliestVisibleChange,
                "earliest_visible_change",
                false,
            ),
            // The common case on a shallow clone: in the tree now, never touched in visible history.
            (
                FirstObservedKind::PresentBeforeVisibleHistory,
                "present_before_visible_history",
                false,
            ),
            // Not in the current tree, and the current tree was genuinely consulted.
            (
                FirstObservedKind::AbsentFromVisibleHistory,
                "absent_from_visible_history",
                false,
            ),
            // History syncs without an index, so the current tree may be unknowable.
            (
                FirstObservedKind::CurrentTreeUnknown,
                "current_tree_unknown",
                false,
            ),
            // Absence of an ingest is not absence of history.
            (
                FirstObservedKind::NoHistoryIngested,
                "no_history_ingested",
                false,
            ),
        ];

        for (value, name, may_claim) in pinned {
            assert_eq!(
                value.as_str(),
                name,
                "{value:?} is pinned against this list"
            );
            assert_eq!(name.parse::<FirstObservedKind>().unwrap(), value);
            assert_eq!(value.to_string(), name);
            assert_eq!(
                value.may_claim_created(),
                may_claim,
                "{name} changed what a consumer may claim about the creation of a path"
            );
        }

        // Exhaustiveness. A seventh value cannot be classified by a `_` arm somewhere else, because
        // it fails here first.
        let mut listed: Vec<FirstObservedKind> =
            pinned.iter().map(|(value, _, _)| *value).collect();
        listed.sort_unstable();
        let mut all = FirstObservedKind::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a first-observed value was added without stating what it permits"
        );

        assert_eq!(
            FirstObservedKind::ALL
                .iter()
                .filter(|kind| kind.may_claim_created())
                .count(),
            1,
            "exactly one value may be rendered as creation"
        );

        // The three pairs whose collapse is the whole class of defect this vocabulary guards.
        // "Never touched in visible history" is not "absent", and neither is "we could not look".
        assert_ne!(
            FirstObservedKind::PresentBeforeVisibleHistory,
            FirstObservedKind::AbsentFromVisibleHistory
        );
        assert_ne!(
            FirstObservedKind::CurrentTreeUnknown,
            FirstObservedKind::AbsentFromVisibleHistory
        );
        assert_ne!(
            FirstObservedKind::NoHistoryIngested,
            FirstObservedKind::AbsentFromVisibleHistory
        );

        // The names that would restore the ambiguity, and the one that would over-claim.
        for invented in [
            "created",
            "first_commit",
            "not_in_visible_history",
            "unknown",
            "",
        ] {
            assert!(
                invented.parse::<FirstObservedKind>().is_err(),
                "{invented:?} parsed as a FirstObservedKind"
            );
        }
    }

    /// Four freshness verdicts, and `unverifiable` is not `current`.
    ///
    /// Slice 7c-i is an entire slice about the difference between *stale* and *unverified*: reporting
    /// "unknown" as "current" is how a truncated sweep becomes a clean bill of health. The verdict
    /// for a repository state with no recorded commit is therefore its own value.
    #[test]
    fn every_history_freshness_verdict_keeps_unknown_apart_from_current() {
        let pinned: [(HistoryFreshness, &str); 4] = [
            (HistoryFreshness::Current, "current"),
            (HistoryFreshness::Stale, "stale"),
            // Not `current`. The comparison could not be made.
            (HistoryFreshness::Unverifiable, "unverifiable"),
            (HistoryFreshness::NoHistoryIngested, "no_history_ingested"),
        ];

        for (value, name) in pinned {
            assert_eq!(
                value.as_str(),
                name,
                "{value:?} is pinned against this list"
            );
            assert_eq!(name.parse::<HistoryFreshness>().unwrap(), value);
            assert_eq!(value.to_string(), name);
        }

        let mut listed: Vec<HistoryFreshness> = pinned.iter().map(|(value, _)| *value).collect();
        listed.sort_unstable();
        let mut all = HistoryFreshness::ALL.to_vec();
        all.sort_unstable();
        assert_eq!(
            listed, all,
            "a freshness verdict was added to the vocabulary without a name above"
        );

        assert_ne!(HistoryFreshness::Unverifiable, HistoryFreshness::Current);
        assert_ne!(HistoryFreshness::Unverifiable, HistoryFreshness::Stale);
        assert_ne!(
            HistoryFreshness::NoHistoryIngested,
            HistoryFreshness::Unverifiable
        );

        for invented in ["fresh", "unverified", "unknown", "ok", ""] {
            assert!(
                invented.parse::<HistoryFreshness>().is_err(),
                "{invented:?} parsed as a HistoryFreshness"
            );
        }
    }

    /// Every value of the four rendered vocabularies has a note, and no two share one.
    ///
    /// The notes were four functions inside the CLI binary until Slice 12c-i moved them here. This
    /// test is what makes the move a property rather than a tidy-up: a value added without a
    /// sentence would fall into a `match` arm that does not exist and fail to compile, and a value
    /// given a *copy* of its neighbour's sentence fails on the uniqueness check below — which is the
    /// drift the single-copy source scan in `crates/nerve-cli/tests/history_wording.rs` cannot see,
    /// because a duplicate inside this file is still inside this file.
    #[test]
    fn every_rendered_history_value_has_its_own_note() {
        let mut notes: Vec<&'static str> = Vec::new();
        for value in WalkTermination::ALL {
            notes.push(value.note());
        }
        for value in ParentCompleteness::ALL {
            notes.push(value.note());
        }
        for value in ChangesEnumerated::ALL {
            notes.push(value.note());
        }
        for value in RenameAmbiguity::ALL {
            notes.push(value.note());
        }
        assert_eq!(
            notes.len(),
            WalkTermination::ALL.len()
                + ParentCompleteness::ALL.len()
                + ChangesEnumerated::ALL.len()
                + RenameAmbiguity::ALL.len()
        );
        for note in &notes {
            assert!(
                note.len() > 20,
                "{note:?} is too short to be an explanation"
            );
        }
        let mut unique = notes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            notes.len(),
            "two values share one sentence, so one of them is described by the other's rule"
        );

        // The one sentence that must exist, and the four that must not be said of a boundary.
        assert!(ParentCompleteness::Root
            .note()
            .contains("the project's history begins here"));
        for value in ParentCompleteness::ALL {
            if value.may_claim_history_begins_here() {
                continue;
            }
            for forbidden in [
                "history begins here",
                "first commit in project",
                "beginning of repository history",
            ] {
                assert!(
                    !value.note().contains(forbidden),
                    "{value} claims {forbidden:?}: {:?}",
                    value.note()
                );
            }
        }
        // Nerve's own boundary must never read as the repository's.
        assert!(WalkTermination::CommitBudget
            .note()
            .contains("more history than this ingest read"));
        assert!(!WalkTermination::CommitBudget.note().contains("shallow"));
        // Three of the four silences must say they are not emptiness.
        assert_eq!(
            ChangesEnumerated::ALL
                .iter()
                .filter(|value| value.note().contains("not an empty commit"))
                .count(),
            2,
            "the merge and the unreadable parent each say they are not an empty commit"
        );
        assert!(ChangesEnumerated::Enumerated
            .note()
            .contains("\"nothing changed\""));
    }
}
