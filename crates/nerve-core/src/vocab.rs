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
}

impl EntityKind {
    /// Every kind, in declaration order.
    ///
    /// Appended to, never inserted into. Nothing on disk encodes a position — `entity.kind` is
    /// `TEXT` — but `apps/nerve-web/src/api/types.ts` mirrors this array *in order* and
    /// `crates/nerve-server/tests/ui_vocabulary.rs` asserts the two match exactly, so the order
    /// is a contract with the interface even though it is not one with the database.
    pub const ALL: [EntityKind; 12] = [
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
}

impl Relation {
    /// Every relation, in declaration order.
    ///
    /// Appended to, never inserted into — `apps/nerve-web/src/api/types.ts` mirrors this array in
    /// order and `crates/nerve-server/tests/ui_vocabulary.rs` asserts the two match exactly.
    pub const ALL: [Relation; 10] = [
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
                "unres", "cov"
            ]
        );
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
    /// *symbols*. Listing all twelve, and checking the list is exhaustive over
    /// [`EntityKind::ALL`], makes a new kind fail here until someone states which it is.
    #[test]
    fn every_entity_kind_is_classified_as_a_symbol_or_not() {
        let pinned: [(EntityKind, bool); 12] = [
            // Named by a path, a tuple of its own, or a content hash — never by the symbol tuple.
            (EntityKind::Repository, false),
            (EntityKind::Directory, false),
            (EntityKind::File, false),
            (EntityKind::Module, false),
            (EntityKind::Document, false),
            (EntityKind::Section, false),
            (EntityKind::Unresolved, false),
            (EntityKind::CoverageRun, false),
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
