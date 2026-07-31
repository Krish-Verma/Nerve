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
    /// A reference target that could not be resolved. A value, never an omission.
    Unresolved,
}

impl EntityKind {
    /// Every kind, in declaration order.
    pub const ALL: [EntityKind; 9] = [
        EntityKind::Repository,
        EntityKind::Directory,
        EntityKind::File,
        EntityKind::Module,
        EntityKind::Function,
        EntityKind::Method,
        EntityKind::Class,
        EntityKind::Interface,
        EntityKind::Unresolved,
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
            EntityKind::Unresolved => "unresolved",
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
            EntityKind::Unresolved => "unres",
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
}

impl Relation {
    /// Every relation, in declaration order.
    pub const ALL: [Relation; 8] = [
        Relation::Contains,
        Relation::Defines,
        Relation::Imports,
        Relation::Exports,
        Relation::Calls,
        Relation::References,
        Relation::Extends,
        Relation::Implements,
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
}

impl EvidenceSourceType {
    /// Every source type, in declaration order. Index in this array is the ordinal.
    pub const ALL: [EvidenceSourceType; 11] = [
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
            vec!["repo", "dir", "file", "mod", "fn", "meth", "class", "iface", "unres"]
        );
    }

    #[test]
    fn relation_round_trips() {
        for relation in Relation::ALL {
            assert_eq!(relation.as_str().parse::<Relation>().unwrap(), relation);
        }
    }

    #[test]
    fn evidence_source_type_ordinals_and_masks_are_stable() {
        assert_eq!(EvidenceSourceType::AstDirect.ordinal(), 0);
        assert_eq!(EvidenceSourceType::AstDirect.mask_bit(), 1);
        assert_eq!(EvidenceSourceType::LlmDerived.ordinal(), 10);
        assert_eq!(EvidenceSourceType::LlmDerived.mask_bit(), 1024);
        for source in EvidenceSourceType::ALL {
            assert_eq!(
                source.as_str().parse::<EvidenceSourceType>().unwrap(),
                source
            );
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
