//! C1 and C3: the two cross-repository contracts that can be read from a manifest with no new
//! dependency (Slice 13b).
//!
//! **C1** is an npm local or workspace dependency in `package.json`; **C3** is a Python local path
//! dependency in `pyproject.toml`. Both are *repository-to-repository*: neither end is an entity
//! Nerve models, so neither emits a `Relation`, an `assertion` or an `observation`, and both live in
//! `contract_link` alone. `docs/plans/slice-13-cross-repository-contracts.md` §4.3 as corrected on
//! 2026-08-08 is the argument: the question is not *does this have entities on both ends* but *are
//! both ends in this database*, and here the answer is no.
//!
//! Six properties are what this module is.
//!
//! 1. **A link comes from an explicit stated declaration and from nothing else.** The supported
//!    syntax is enumerated in [`SupportedForm`] and everything outside it is recorded as
//!    [`UnsupportedForm`] with the form **named**. Nothing is fetched, nothing is guessed, and a
//!    specifier Nerve declines is a tally entry rather than an absence.
//! 2. **Resolution is by the target's recorded `repo_id` and never by name.** A declared path is
//!    opened through [`crate::registry::probe_target`], the repository id found there is compared
//!    against [`RegistryEntryRow::expected_repository_id`], and a target that matches no registered
//!    entry is [`UnresolvedReason::TargetNotRegistered`] — never auto-registered, never matched by
//!    package name, never matched by directory proximity. `CLAUDE.md` §3: identity is never
//!    established by fuzzy name matching alone.
//! 3. **Availability is not re-derived here.** [`crate::registry::availability_of`] is the one place
//!    that decides whether a neighbour is readable, and
//!    `crates/nerve-cli/tests/registry_guards.rs` enforces it. This module asks that function and
//!    renders its answer; it never compares a path against a row.
//! 4. **Nothing is executed and nothing is fetched.** A `git:`, `github:`, `https:` or `npm:`
//!    specifier names a network resolution, and Nerve records that it saw one. `npm`, `pip`,
//!    `poetry` and `git` are never run — `crates/nerve-cli/tests/no_subprocess.rs` and
//!    `crates/nerve-cli/tests/no_network.rs` are untouched by this slice, and a registry is not a
//!    package manager.
//! 5. **Every bound is exercisable.** [`MAX_MANIFEST_BYTES`], [`MAX_DECLARATIONS_PER_MANIFEST`] and
//!    [`MAX_LINKS_PER_REPOSITORY`] each stop the scan with a named [`ManifestRefusal`], and
//!    `crates/nerve-index/tests/contracts.rs` reaches all three. A bound that cannot be exercised
//!    cannot be tested, which is the correction 12c-i-b already had to make.
//! 6. **Re-running is idempotent.** The logical identity is the unique index
//!    `idx_contract_link_identity`, so a re-scan of an unchanged tree finds every row it would have
//!    written and touches it instead. Nothing is deleted and nothing is duplicated.
//!
//! # Why `workspace:` needs the `workspaces` array, and why that is not name matching
//!
//! `workspace:*` carries **no path**. Resolving it at all therefore needs a second stated
//! declaration, and the only honest one is the source manifest's own `workspaces` array: *this
//! repository states that its workspace members live at these paths.* A member is then identified by
//! the `name` its own `package.json` declares, and the repository behind it is confirmed by
//! `repo_id`. Every step is a declaration in a file — the path from the source manifest, the name
//! from the member's manifest, the identity from the target's index — which is what separates it
//! from the "package name with no registry and version context" §1 of the row plan refuses.
//!
//! A `workspaces` entry containing a glob is **not** expanded. Globbing is a resolution rule Nerve
//! would be inventing, so it is recorded as [`UnsupportedForm::NpmWorkspaceGlobPattern`] and named.
//!
//! # What this module deliberately does not decide
//!
//! `expected_contract_version` and `observed_contract_version` are both recorded and neither is
//! compared. `^1.2.0` against `1.2.3` is a *range satisfaction* question, and answering it needs a
//! semantic-version resolver — a new dependency, or a parser of our own in the exact expression that
//! decides whether two repositories agree. `contract_version_mismatch` therefore has no producer in
//! this slice, and the evidence for one is stored rather than a verdict invented.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nerve_core::vocab::{ContractFreshness, ContractLinkStatus, ContractResolutionMethod};
use nerve_store::{Connection, ContractLinkRow, RegistryEntryRow};

use crate::config::Config;
use crate::discover::{canonical_child, canonical_root, discover_named};
use crate::error::Result;
use crate::registry::{availability_of, probe_target, RegistryAvailability, RegistryTarget};

/// The largest manifest this module will read, in bytes.
///
/// A manifest is a declaration file a human wrote; a megabyte of it is already two orders of
/// magnitude past anything real. The bound is a stop with a name rather than a truncation, because a
/// manifest read halfway would report the dependencies before the cut and silently omit the rest.
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// The largest number of dependency declarations one manifest may state.
///
/// Exceeding it refuses the **whole manifest** rather than the surplus. Reading the first
/// [`MAX_DECLARATIONS_PER_MANIFEST`] and dropping the rest would put a link in the database for one
/// half of a file and nothing at all for the other, with no row anywhere saying which half.
pub const MAX_DECLARATIONS_PER_MANIFEST: usize = 2_000;

/// The largest number of contract links one repository may hold.
///
/// Counted against the table, not against the run, so a scan cannot grow the table past the bound by
/// being run twice. Re-observing an already-stored link is not growth and is never refused.
pub const MAX_LINKS_PER_REPOSITORY: usize = 1_000;

/// The extractor id C1 stamps on every link it records.
pub const NPM_EXTRACTOR_ID: &str = "npm-local-dependency";

/// The extractor id C3 stamps on every link it records.
pub const PYTHON_EXTRACTOR_ID: &str = "python-path-dependency";

/// The version both contract rules stamp on every link they record.
pub const CONTRACT_EXTRACTOR_VERSION: &str = "1";

/// The semantic relation a repository-to-repository dependency states.
///
/// This is **not** a member of [`nerve_core::vocab::Relation`] and never a row in `assertion`. Row
/// 13's plan refuted a `DEPENDS_ON` *relation* because there is no entity on either end to be its
/// endpoint (§10.3), and that refutation stands: this string is the description a response renders
/// for a link, stored in a free-text column of a table that is not the evidence graph. An ordinary
/// `path` or `impact` query cannot reach it.
pub const REPOSITORY_DEPENDENCY_SEMANTICS: &str = "DEPENDS_ON";

/// Which contract rule read a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContractRule {
    /// C1 — an npm local or workspace dependency in `package.json`.
    NpmLocalDependency,
    /// C3 — a Python local path dependency in `pyproject.toml`.
    PythonPathDependency,
}

impl ContractRule {
    /// Every value, in declaration order.
    pub const ALL: [ContractRule; 2] = [
        ContractRule::NpmLocalDependency,
        ContractRule::PythonPathDependency,
    ];

    /// Canonical lower-case name. This is what `contract_link.contract_kind` stores.
    pub fn as_str(self) -> &'static str {
        match self {
            ContractRule::NpmLocalDependency => "npm_local_dependency",
            ContractRule::PythonPathDependency => "python_path_dependency",
        }
    }

    /// The manifest file name this rule reads.
    pub fn manifest_file_name(self) -> &'static str {
        match self {
            ContractRule::NpmLocalDependency => "package.json",
            ContractRule::PythonPathDependency => "pyproject.toml",
        }
    }

    /// The extractor id this rule stamps on a link.
    pub fn extractor_id(self) -> &'static str {
        match self {
            ContractRule::NpmLocalDependency => NPM_EXTRACTOR_ID,
            ContractRule::PythonPathDependency => PYTHON_EXTRACTOR_ID,
        }
    }

    /// Which rule reads a file with this name, if any.
    pub fn for_file_name(name: &str) -> Option<ContractRule> {
        ContractRule::ALL
            .into_iter()
            .find(|rule| rule.manifest_file_name() == name)
    }
}

impl std::fmt::Display for ContractRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A declaration shape Nerve reads, as a closed vocabulary.
///
/// The set is closed and small on purpose. Every member names a *path* — directly, or through the
/// source manifest's own `workspaces` array — because a path is the only thing in a manifest that
/// can be resolved to a repository without asking a network registry what a name means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SupportedForm {
    /// npm `"pkg": "file:../somewhere"`.
    NpmFilePath,
    /// npm `"pkg": "workspace:*"`.
    NpmWorkspaceWildcard,
    /// npm `"pkg": "workspace:^"`.
    NpmWorkspaceCaret,
    /// npm `"pkg": "workspace:~"`.
    NpmWorkspaceTilde,
    /// npm `"pkg": "workspace:1.2.3"` — any other non-empty remainder, kept as the expected version.
    NpmWorkspaceVersion,
    /// PEP 621 `dependencies = ["pkg @ file:///somewhere"]`.
    PythonDirectFileUrl,
    /// Poetry `[tool.poetry.dependencies] pkg = { path = "../somewhere" }`.
    PythonPoetryPath,
    /// uv / PEP 735 `[tool.uv.sources] pkg = { path = "../somewhere" }`.
    PythonUvSourcePath,
}

impl SupportedForm {
    /// Every value, in declaration order.
    pub const ALL: [SupportedForm; 8] = [
        SupportedForm::NpmFilePath,
        SupportedForm::NpmWorkspaceWildcard,
        SupportedForm::NpmWorkspaceCaret,
        SupportedForm::NpmWorkspaceTilde,
        SupportedForm::NpmWorkspaceVersion,
        SupportedForm::PythonDirectFileUrl,
        SupportedForm::PythonPoetryPath,
        SupportedForm::PythonUvSourcePath,
    ];

    /// Canonical lower-case name, carried on every response that reports a link.
    pub fn as_str(self) -> &'static str {
        match self {
            SupportedForm::NpmFilePath => "npm_file_path",
            SupportedForm::NpmWorkspaceWildcard => "npm_workspace_wildcard",
            SupportedForm::NpmWorkspaceCaret => "npm_workspace_caret",
            SupportedForm::NpmWorkspaceTilde => "npm_workspace_tilde",
            SupportedForm::NpmWorkspaceVersion => "npm_workspace_version",
            SupportedForm::PythonDirectFileUrl => "python_direct_file_url",
            SupportedForm::PythonPoetryPath => "python_poetry_path",
            SupportedForm::PythonUvSourcePath => "python_uv_source_path",
        }
    }

    /// Which rule this form belongs to.
    pub fn rule(self) -> ContractRule {
        match self {
            SupportedForm::NpmFilePath
            | SupportedForm::NpmWorkspaceWildcard
            | SupportedForm::NpmWorkspaceCaret
            | SupportedForm::NpmWorkspaceTilde
            | SupportedForm::NpmWorkspaceVersion => ContractRule::NpmLocalDependency,
            SupportedForm::PythonDirectFileUrl
            | SupportedForm::PythonPoetryPath
            | SupportedForm::PythonUvSourcePath => ContractRule::PythonPathDependency,
        }
    }

    /// Which stated declaration a link from this form was drawn from.
    ///
    /// The mapping is the whole of §2's instruction, written once:
    ///
    /// | form | `resolution_method` | because |
    /// |---|---|---|
    /// | `file:` | `manifest_declared` | the manifest states the path itself |
    /// | `workspace:` | `workspace_declared` | the path comes from the `workspaces` array |
    /// | `pkg @ file://` | `manifest_declared` | the manifest states the URL itself |
    /// | Poetry / uv `{ path = }` | `path_dependency_resolved` | a path *table*, resolved to a root |
    ///
    /// `export_map_resolved` has no producer here: it belongs to C2, which is 13c.
    pub fn resolution_method(self) -> ContractResolutionMethod {
        match self {
            SupportedForm::NpmFilePath | SupportedForm::PythonDirectFileUrl => {
                ContractResolutionMethod::ManifestDeclared
            }
            SupportedForm::NpmWorkspaceWildcard
            | SupportedForm::NpmWorkspaceCaret
            | SupportedForm::NpmWorkspaceTilde
            | SupportedForm::NpmWorkspaceVersion => ContractResolutionMethod::WorkspaceDeclared,
            SupportedForm::PythonPoetryPath | SupportedForm::PythonUvSourcePath => {
                ContractResolutionMethod::PathDependencyResolved
            }
        }
    }
}

impl std::fmt::Display for SupportedForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A declaration shape Nerve declines to resolve, **named**.
///
/// Every member of this vocabulary is a form that was read, recognised and refused. None of them is
/// a silent drop, and none of them is fetched: a `git:` specifier is a network resolution and Nerve
/// records that it saw one rather than performing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnsupportedForm {
    /// A version range resolved from a package registry: `^1.2.3`, `~1.0`, `1.x`, `*`, `latest`.
    NpmRegistryRange,
    /// A Git specifier: `git:`, `git+ssh:`, `github:`, `owner/repo`.
    NpmGitSpecifier,
    /// An `http:` or `https:` tarball.
    NpmUrlSpecifier,
    /// An alias: `npm:other-package@1.0.0`.
    NpmAliasSpecifier,
    /// Any other protocol prefix, including `link:` and a bare `workspace:`.
    NpmUnsupportedProtocol,
    /// A `dependencies` value that is not a string at all.
    NpmNonStringSpecifier,
    /// A `workspaces` entry containing a glob, which Nerve does not expand.
    NpmWorkspaceGlobPattern,
    /// A PEP 508 requirement with a version specifier and no direct reference: `requests>=2.31`.
    PythonVersionSpecifier,
    /// A direct reference that is not an absolute, unescaped `file://` URL.
    PythonUnsupportedDirectReference,
    /// A Poetry or uv source table with `git =`.
    PythonGitSource,
    /// A Poetry or uv source table with `url =`.
    PythonUrlSource,
    /// A uv source table with `workspace = true`, which names no path.
    PythonWorkspaceSource,
    /// A source table with none of `path`, `git`, `url` or `workspace`.
    PythonUnsupportedSource,
}

impl UnsupportedForm {
    /// Every value, in declaration order.
    pub const ALL: [UnsupportedForm; 13] = [
        UnsupportedForm::NpmRegistryRange,
        UnsupportedForm::NpmGitSpecifier,
        UnsupportedForm::NpmUrlSpecifier,
        UnsupportedForm::NpmAliasSpecifier,
        UnsupportedForm::NpmUnsupportedProtocol,
        UnsupportedForm::NpmNonStringSpecifier,
        UnsupportedForm::NpmWorkspaceGlobPattern,
        UnsupportedForm::PythonVersionSpecifier,
        UnsupportedForm::PythonUnsupportedDirectReference,
        UnsupportedForm::PythonGitSource,
        UnsupportedForm::PythonUrlSource,
        UnsupportedForm::PythonWorkspaceSource,
        UnsupportedForm::PythonUnsupportedSource,
    ];

    /// Canonical lower-case name, carried on every tally that reports a declined form.
    pub fn as_str(self) -> &'static str {
        match self {
            UnsupportedForm::NpmRegistryRange => "npm_registry_range",
            UnsupportedForm::NpmGitSpecifier => "npm_git_specifier",
            UnsupportedForm::NpmUrlSpecifier => "npm_url_specifier",
            UnsupportedForm::NpmAliasSpecifier => "npm_alias_specifier",
            UnsupportedForm::NpmUnsupportedProtocol => "npm_unsupported_protocol",
            UnsupportedForm::NpmNonStringSpecifier => "npm_non_string_specifier",
            UnsupportedForm::NpmWorkspaceGlobPattern => "npm_workspace_glob_pattern",
            UnsupportedForm::PythonVersionSpecifier => "python_version_specifier",
            UnsupportedForm::PythonUnsupportedDirectReference => {
                "python_unsupported_direct_reference"
            }
            UnsupportedForm::PythonGitSource => "python_git_source",
            UnsupportedForm::PythonUrlSource => "python_url_source",
            UnsupportedForm::PythonWorkspaceSource => "python_workspace_source",
            UnsupportedForm::PythonUnsupportedSource => "python_unsupported_source",
        }
    }

    /// Which rule read the declaration this form was found in.
    pub fn rule(self) -> ContractRule {
        match self {
            UnsupportedForm::NpmRegistryRange
            | UnsupportedForm::NpmGitSpecifier
            | UnsupportedForm::NpmUrlSpecifier
            | UnsupportedForm::NpmAliasSpecifier
            | UnsupportedForm::NpmUnsupportedProtocol
            | UnsupportedForm::NpmNonStringSpecifier
            | UnsupportedForm::NpmWorkspaceGlobPattern => ContractRule::NpmLocalDependency,
            UnsupportedForm::PythonVersionSpecifier
            | UnsupportedForm::PythonUnsupportedDirectReference
            | UnsupportedForm::PythonGitSource
            | UnsupportedForm::PythonUrlSource
            | UnsupportedForm::PythonWorkspaceSource
            | UnsupportedForm::PythonUnsupportedSource => ContractRule::PythonPathDependency,
        }
    }
}

impl std::fmt::Display for UnsupportedForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A supported declaration that reached no registered neighbour, with the reason **named**.
///
/// The distinction from [`UnsupportedForm`] is load bearing. An unsupported form is *syntax Nerve
/// declines to read*; an unresolved declaration is syntax Nerve read fully and a target it could not
/// reach. Collapsing the two would make "we do not support this" and "you have not registered that"
/// the same message, and only the second has a remedy the user can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnresolvedReason {
    /// Nothing exists at the declared path.
    DeclaredPathMissing,
    /// The declared path is inside the repository being scanned, so it is not a *cross*-repository
    /// contract at all.
    DeclaredPathInSameRepository,
    /// Something is at the declared path and it is not the root of a Nerve-initialised repository.
    DeclaredPathNotARepositoryRoot,
    /// A Nerve repository is at the declared path and no registry entry records its `repo_id`.
    TargetNotRegistered,
    /// A registry entry records the repository, and the entry is not usable right now.
    RegistryEntryUnusable,
    /// A `workspace:` dependency whose name no declared workspace member claims.
    WorkspaceMemberNotDeclared,
}

impl UnresolvedReason {
    /// Every value, in declaration order.
    pub const ALL: [UnresolvedReason; 6] = [
        UnresolvedReason::DeclaredPathMissing,
        UnresolvedReason::DeclaredPathInSameRepository,
        UnresolvedReason::DeclaredPathNotARepositoryRoot,
        UnresolvedReason::TargetNotRegistered,
        UnresolvedReason::RegistryEntryUnusable,
        UnresolvedReason::WorkspaceMemberNotDeclared,
    ];

    /// Canonical lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            UnresolvedReason::DeclaredPathMissing => "declared_path_missing",
            UnresolvedReason::DeclaredPathInSameRepository => "declared_path_in_same_repository",
            UnresolvedReason::DeclaredPathNotARepositoryRoot => {
                "declared_path_not_a_repository_root"
            }
            UnresolvedReason::TargetNotRegistered => "target_not_registered",
            UnresolvedReason::RegistryEntryUnusable => "registry_entry_unusable",
            UnresolvedReason::WorkspaceMemberNotDeclared => "workspace_member_not_declared",
        }
    }
}

impl std::fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a manifest, or the rest of a scan, was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManifestRefusal {
    /// The file is larger than [`MAX_MANIFEST_BYTES`].
    ManifestTooLarge,
    /// The file states more than [`MAX_DECLARATIONS_PER_MANIFEST`] declarations.
    TooManyDeclarations,
    /// The file could not be read, or is not UTF-8.
    ManifestUnreadable,
    /// The file is not valid JSON or TOML.
    ManifestUnparsable,
    /// The repository already holds [`MAX_LINKS_PER_REPOSITORY`] links.
    LinkBudgetExhausted,
}

impl ManifestRefusal {
    /// Every value, in declaration order.
    pub const ALL: [ManifestRefusal; 5] = [
        ManifestRefusal::ManifestTooLarge,
        ManifestRefusal::TooManyDeclarations,
        ManifestRefusal::ManifestUnreadable,
        ManifestRefusal::ManifestUnparsable,
        ManifestRefusal::LinkBudgetExhausted,
    ];

    /// Canonical lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            ManifestRefusal::ManifestTooLarge => "manifest_too_large",
            ManifestRefusal::TooManyDeclarations => "too_many_declarations",
            ManifestRefusal::ManifestUnreadable => "manifest_unreadable",
            ManifestRefusal::ManifestUnparsable => "manifest_unparsable",
            ManifestRefusal::LinkBudgetExhausted => "link_budget_exhausted",
        }
    }
}

impl std::fmt::Display for ManifestRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a whole scan refused before it read anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScanRefusal {
    /// The repository has no indexed state, so a link would have nothing to be resolved *at*.
    SourceNotIndexed,
}

impl ScanRefusal {
    /// Every value, in declaration order.
    pub const ALL: [ScanRefusal; 1] = [ScanRefusal::SourceNotIndexed];

    /// Canonical lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            ScanRefusal::SourceNotIndexed => "source_not_indexed",
        }
    }

    /// What was refused, and why, in words.
    pub fn statement(self) -> &'static str {
        match self {
            ScanRefusal::SourceNotIndexed => {
                "this repository has no indexed state, so a link recorded now could not say which \
                 state of this repository declared it — run `nerve index` first"
            }
        }
    }
}

impl std::fmt::Display for ScanRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How ambiguous a resolution was, when it was.
///
/// Recorded on **every** link of the ambiguous identity, and none of them is promoted over another.
/// That is 12c's `many_from` / `many_to` discipline in a new place: the evidence is that the
/// repository declared the same thing twice, and picking a winner would be Nerve inventing the
/// answer the manifest declined to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ambiguity {
    /// One identity, more than one supported declaration, all agreeing on the target.
    DeclaredMoreThanOnce,
    /// One identity, more than one supported declaration, naming **different** targets.
    ConflictingTargets,
}

impl Ambiguity {
    /// Every value, in declaration order.
    pub const ALL: [Ambiguity; 2] = [
        Ambiguity::DeclaredMoreThanOnce,
        Ambiguity::ConflictingTargets,
    ];

    /// Canonical lower-case name. This is what `contract_link.ambiguity` stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Ambiguity::DeclaredMoreThanOnce => "declared_more_than_once",
            Ambiguity::ConflictingTargets => "conflicting_targets",
        }
    }
}

impl std::fmt::Display for Ambiguity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---- what a scan produces ----------------------------------------------------------------------

/// One link this scan recorded or re-observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedLink {
    /// Which rule read it.
    pub rule: ContractRule,
    /// Repository-relative path of the manifest.
    pub manifest: String,
    /// Which manifest section the declaration sits in.
    pub section: String,
    /// The dependency key. Untrusted repository content.
    pub identity: String,
    /// The declared form.
    pub form: SupportedForm,
    /// The registry entry the link resolved through.
    pub registry_id: String,
    /// The repository id found at the declared path.
    pub target_repository_id: String,
    /// Which stated declaration it was drawn from.
    pub resolution_method: ContractResolutionMethod,
    /// Where in the manifest, as `line:line`.
    pub source_span: String,
    /// The version this repository asks for, when the form states one.
    pub expected_contract_version: Option<String>,
    /// The version the target's own manifest declares, when it was readable.
    pub observed_contract_version: Option<String>,
    /// How ambiguous the resolution was, when it was.
    pub ambiguity: Option<Ambiguity>,
    /// `true` when the row was inserted by this scan, `false` when it already existed.
    pub inserted: bool,
}

/// One supported declaration that reached no registered neighbour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedDeclaration {
    /// Which rule read it.
    pub rule: ContractRule,
    /// Repository-relative path of the manifest.
    pub manifest: String,
    /// Which manifest section the declaration sits in.
    pub section: String,
    /// The dependency key. Untrusted repository content.
    pub identity: String,
    /// The declared form, which was fully understood.
    pub form: SupportedForm,
    /// Why it reached nothing.
    pub reason: UnresolvedReason,
}

/// One declaration whose form Nerve declined to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedDeclaration {
    /// Which rule read it.
    pub rule: ContractRule,
    /// Repository-relative path of the manifest.
    pub manifest: String,
    /// Which manifest section the declaration sits in.
    pub section: String,
    /// The dependency key, or the `workspaces` entry's position. Untrusted repository content.
    pub identity: String,
    /// The form Nerve declined, **named**.
    pub form: UnsupportedForm,
}

/// Everything one scan read, recorded and refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractScan {
    /// The local state every link in this scan was resolved at.
    pub source_state: String,
    /// How many manifests were read to the end.
    pub manifests_read: usize,
    /// How many declarations were seen, supported or not.
    pub declarations: usize,
    /// Every link recorded or re-observed, ordered by manifest then section then identity.
    pub links: Vec<RecordedLink>,
    /// Every supported declaration that reached no registered neighbour.
    pub unresolved: Vec<UnresolvedDeclaration>,
    /// Every declaration whose form was declined, with the form named.
    pub unsupported: Vec<UnsupportedDeclaration>,
    /// Every manifest, and every bound, that stopped the scan.
    pub refusals: Vec<(String, ManifestRefusal)>,
}

impl ContractScan {
    /// How many links this scan inserted.
    pub fn inserted(&self) -> usize {
        self.links.iter().filter(|link| link.inserted).count()
    }

    /// How many links this scan found already stored and re-observed.
    pub fn unchanged(&self) -> usize {
        self.links.iter().filter(|link| !link.inserted).count()
    }

    /// Declined forms and how many of each, so §9.1's tally is a count rather than an inspection.
    pub fn unsupported_tally(&self) -> BTreeMap<UnsupportedForm, usize> {
        let mut tally = BTreeMap::new();
        for entry in &self.unsupported {
            *tally.entry(entry.form).or_insert(0) += 1;
        }
        tally
    }

    /// Unresolved reasons and how many of each.
    pub fn unresolved_tally(&self) -> BTreeMap<UnresolvedReason, usize> {
        let mut tally = BTreeMap::new();
        for entry in &self.unresolved {
            *tally.entry(entry.reason).or_insert(0) += 1;
        }
        tally
    }

    /// Every link one rule produced. Precision is measured **per rule** and never summed.
    pub fn links_of(&self, rule: ContractRule) -> impl Iterator<Item = &RecordedLink> {
        self.links.iter().filter(move |link| link.rule == rule)
    }

    /// Every unresolved declaration one rule produced.
    pub fn unresolved_of(
        &self,
        rule: ContractRule,
    ) -> impl Iterator<Item = &UnresolvedDeclaration> {
        self.unresolved.iter().filter(move |row| row.rule == rule)
    }

    /// Every declined declaration one rule produced.
    pub fn unsupported_of(
        &self,
        rule: ContractRule,
    ) -> impl Iterator<Item = &UnsupportedDeclaration> {
        self.unsupported.iter().filter(move |row| row.rule == rule)
    }
}

/// The outcome of a scan: what it read, or the reason it read nothing.
///
/// A refusal is a value rather than an error, in the manner of
/// [`crate::registry::RegistryOutcome`]: a real storage failure is still an `Err`, and a refusal is
/// something Nerve decided and can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    /// The scan ran, and this is what it found.
    Done(Box<ContractScan>),
    /// The scan refused before reading anything.
    Refused(ScanRefusal),
}

// ---- the scan ----------------------------------------------------------------------------------

/// A neighbour this repository registered, as it stands right now.
struct Neighbour {
    entry: RegistryEntryRow,
    /// `None` when the entry is not usable, which is itself the answer.
    target: Option<RegistryTarget>,
}

/// Read every manifest in `root`, resolve what it declares, and record the links.
///
/// The order of operations is the order the guarantees depend on:
///
/// 1. The repository's current state is read. Without one there is nothing to resolve *at*, and the
///    scan refuses rather than writing a link with an invented state.
/// 2. Every registry entry is asked [`crate::registry::availability_of`] once — tombstones
///    included, which answer without anything being opened. That function is the only place
///    availability is decided, here as everywhere.
/// 3. Manifests are found by [`crate::discover::discover_named`], so the ignore rules, the pruned
///    directories and `canonical_child` are the same ones indexing uses — `node_modules` is pruned,
///    which is what keeps a vendored `package.json` from being read as this repository's own
///    declaration.
/// 4. Each manifest is size-checked, read, parsed and classified.
/// 5. Each supported declaration is resolved against the neighbour table **by repository id**.
/// 6. Ambiguity is computed per manifest, then the links are written.
///
/// Nothing is written for a declaration that resolved to nothing: `contract_link.registry_entry_id`
/// is `NOT NULL` with a foreign key into `repo_registry` (`schema.rs:618-665`), so a row for a
/// declaration that named no registered repository is not storable. Those declarations are reported
/// in [`ContractScan::unresolved`] and [`ContractScan::unsupported`] instead, which is what §9.1's
/// tally is made from.
pub fn scan_contracts(conn: &Connection, repo_id: &str, root: &Path) -> Result<ScanOutcome> {
    let root = canonical_root(root)?;
    let source_state = match nerve_store::status(conn)?.state_id {
        Some(state) => state,
        None => return Ok(ScanOutcome::Refused(ScanRefusal::SourceNotIndexed)),
    };

    let neighbours = neighbour_table(conn, repo_id)?;
    let config = Config::load(&root)?;
    let names: Vec<&str> = ContractRule::ALL
        .into_iter()
        .map(ContractRule::manifest_file_name)
        .collect();
    let manifests = discover_named(&root, &config, &names)?;

    let mut scan = ContractScan {
        source_state: source_state.clone(),
        ..ContractScan::default()
    };
    let mut budget = nerve_store::list_contract_links(conn, repo_id)?.len();

    // Two caches, both scoped to one scan. A scan is one moment, so resolving the same declared
    // path twice inside it cannot produce two answers — and the alternative is opening the same
    // neighbour's database once per dependency, which is a read of somebody else's repository
    // performed for no new information.
    let mut resolutions: BTreeMap<PathBuf, std::result::Result<String, UnresolvedReason>> =
        BTreeMap::new();
    let mut versions: BTreeMap<(ContractRule, PathBuf), Option<String>> = BTreeMap::new();

    // One transaction for the whole write phase. A scan is a single observation of what this
    // repository declares, and half of one recorded is a registry that says something the manifest
    // never did.
    let tx = conn
        .unchecked_transaction()
        .map_err(nerve_store::StoreError::from)?;

    for manifest in manifests {
        let Some(rule) = manifest
            .abs_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(ContractRule::for_file_name)
        else {
            continue;
        };

        let text = match read_manifest(&manifest.abs_path) {
            Ok(text) => text,
            Err(refusal) => {
                scan.refusals.push((manifest.rel_path.clone(), refusal));
                continue;
            }
        };

        let parsed = match parse_manifest(rule, &text) {
            Ok(parsed) => parsed,
            Err(refusal) => {
                scan.refusals.push((manifest.rel_path.clone(), refusal));
                continue;
            }
        };
        if parsed.declarations.len() > MAX_DECLARATIONS_PER_MANIFEST {
            scan.refusals.push((
                manifest.rel_path.clone(),
                ManifestRefusal::TooManyDeclarations,
            ));
            continue;
        }

        scan.manifests_read += 1;
        scan.declarations += parsed.declarations.len() + parsed.declined_workspace_globs;
        let manifest_dir = manifest
            .abs_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.clone());

        for index in 0..parsed.declined_workspace_globs {
            scan.unsupported.push(UnsupportedDeclaration {
                rule,
                manifest: manifest.rel_path.clone(),
                section: "workspaces".to_string(),
                identity: format!("workspaces[{index}]"),
                form: UnsupportedForm::NpmWorkspaceGlobPattern,
            });
        }

        // Workspace members are probed once per manifest, not once per dependency: the same
        // directory would otherwise be opened as many times as it is depended upon.
        let mut members: Option<Vec<WorkspaceMember>> = None;
        let mut resolved: Vec<(Declaration, String)> = Vec::new();

        for declaration in parsed.declarations {
            let form = match declaration.outcome {
                DeclarationOutcome::Unsupported(form) => {
                    scan.unsupported.push(UnsupportedDeclaration {
                        rule,
                        manifest: manifest.rel_path.clone(),
                        section: declaration.section.clone(),
                        identity: declaration.identity.clone(),
                        form,
                    });
                    continue;
                }
                DeclarationOutcome::Supported { form, .. } => form,
            };

            let declared = match &declaration.outcome {
                DeclarationOutcome::Supported { path, .. } => path.clone(),
                DeclarationOutcome::Unsupported(_) => unreachable!("handled above"),
            };

            let target_root = match declared {
                DeclaredPath::Stated(ref stated) => Some(manifest_dir.join(stated)),
                DeclaredPath::WorkspaceMember => {
                    let members = members.get_or_insert_with(|| {
                        workspace_members(&manifest_dir, &parsed.workspace_paths)
                    });
                    members
                        .iter()
                        .find(|member| member.package_name == declaration.identity)
                        .map(|member| member.path.clone())
                }
            };

            let Some(target_root) = target_root else {
                scan.unresolved.push(UnresolvedDeclaration {
                    rule,
                    manifest: manifest.rel_path.clone(),
                    section: declaration.section.clone(),
                    identity: declaration.identity.clone(),
                    form,
                    reason: UnresolvedReason::WorkspaceMemberNotDeclared,
                });
                continue;
            };

            let verdict = resolutions
                .entry(target_root.clone())
                .or_insert_with(|| resolve_target(&root, &target_root, &neighbours))
                .clone();

            match verdict {
                Ok(registry_id) => resolved.push((declaration, registry_id)),
                Err(reason) => scan.unresolved.push(UnresolvedDeclaration {
                    rule,
                    manifest: manifest.rel_path.clone(),
                    section: declaration.section.clone(),
                    identity: declaration.identity.clone(),
                    form,
                    reason,
                }),
            }
        }

        let ambiguity = ambiguity_by_identity(&resolved);
        for (declaration, registry_id) in resolved {
            if budget >= MAX_LINKS_PER_REPOSITORY {
                if !scan
                    .refusals
                    .iter()
                    .any(|(_, refusal)| *refusal == ManifestRefusal::LinkBudgetExhausted)
                {
                    scan.refusals.push((
                        manifest.rel_path.clone(),
                        ManifestRefusal::LinkBudgetExhausted,
                    ));
                }
                break;
            }
            let neighbour = neighbours
                .iter()
                .find(|candidate| candidate.entry.registry_id == registry_id)
                .expect("resolve_target returns a registry id from this table");
            let target_root = neighbour
                .target
                .as_ref()
                .expect("a link is only written through a usable entry")
                .root
                .clone();
            let observed_version = versions
                .entry((rule, target_root.clone()))
                .or_insert_with(|| target_declared_version(rule, &target_root))
                .clone();
            let recorded = write_link(
                &tx,
                repo_id,
                &source_state,
                rule,
                &manifest.rel_path,
                &declaration,
                neighbour,
                observed_version,
                ambiguity.get(&declaration.identity).copied(),
            )?;
            if recorded.inserted {
                budget += 1;
            }
            scan.links.push(recorded);
        }
    }

    tx.commit().map_err(nerve_store::StoreError::from)?;
    Ok(ScanOutcome::Done(Box::new(scan)))
}

/// Every active registry entry, and the target behind it when the entry is usable.
///
/// [`crate::registry::availability_of`] decides usability. A second probe follows it for the entries
/// that are usable, because the verdict does not carry the target's identity and re-deriving that
/// identity here — by comparing a path, say — is exactly the second answer
/// `crates/nerve-cli/tests/registry_guards.rs` exists to prevent.
fn neighbour_table(conn: &Connection, repo_id: &str) -> Result<Vec<Neighbour>> {
    let mut out = Vec::new();
    for entry in nerve_store::list_registry_entries(conn, repo_id)? {
        let availability = availability_of(&entry);
        let usable = matches!(
            availability,
            RegistryAvailability::Available | RegistryAvailability::PartiallyIndexed
        );
        let target = if usable {
            probe_target(Path::new(&entry.local_path)).ok()
        } else {
            None
        };
        out.push(Neighbour { entry, target });
    }
    Ok(out)
}

/// Resolve a declared path onto a registered neighbour, **by the target's recorded `repo_id`**.
///
/// The order matters. A path inside the repository being scanned is answered first and by name,
/// because an intra-repository dependency is not a cross-repository contract at all and reporting it
/// as an unregistered neighbour would send the user to register their own repository. Only then is
/// the registry consulted, and only ever by identity.
fn resolve_target(
    source_root: &Path,
    declared: &Path,
    neighbours: &[Neighbour],
) -> std::result::Result<String, UnresolvedReason> {
    match probe_target(declared) {
        Ok(target) => {
            if target.root == source_root {
                return Err(UnresolvedReason::DeclaredPathInSameRepository);
            }
            let matched = neighbours
                .iter()
                .find(|neighbour| neighbour.entry.expected_repository_id == target.repository_id);
            match matched {
                Some(neighbour) if neighbour.target.is_some() => {
                    Ok(neighbour.entry.registry_id.clone())
                }
                Some(_) => Err(UnresolvedReason::RegistryEntryUnusable),
                None => Err(UnresolvedReason::TargetNotRegistered),
            }
        }
        Err(crate::registry::RegistryRefusal::PathDoesNotExist) => {
            Err(UnresolvedReason::DeclaredPathMissing)
        }
        Err(_) => match declared.canonicalize() {
            Ok(canonical) if canonical.starts_with(source_root) => {
                Err(UnresolvedReason::DeclaredPathInSameRepository)
            }
            Ok(_) | Err(_) => Err(UnresolvedReason::DeclaredPathNotARepositoryRoot),
        },
    }
}

/// One workspace member: a declared path that holds a package with a declared name.
struct WorkspaceMember {
    package_name: String,
    path: PathBuf,
}

/// Every literal `workspaces` path that holds a `package.json` with a `name`.
///
/// The member's own manifest is read for its `name` and nothing else. That read is bounded by
/// [`MAX_MANIFEST_BYTES`] like every other, and a member whose manifest is absent, oversized or
/// nameless simply is not a member — which makes the dependency
/// [`UnresolvedReason::WorkspaceMemberNotDeclared`] rather than a guess.
fn workspace_members(manifest_dir: &Path, declared: &[String]) -> Vec<WorkspaceMember> {
    let mut out = Vec::new();
    for entry in declared {
        let path = manifest_dir.join(entry);
        let Ok(root) = path.canonicalize() else {
            continue;
        };
        let Ok(member_manifest) = canonical_child(&root, Path::new("package.json")) else {
            continue;
        };
        let Ok(text) = read_manifest(&member_manifest) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(name) = value.get("name").and_then(serde_json::Value::as_str) {
            out.push(WorkspaceMember {
                package_name: name.to_string(),
                path: root,
            });
        }
    }
    out
}

/// Which identities were declared more than once, and whether the declarations agree.
fn ambiguity_by_identity(resolved: &[(Declaration, String)]) -> BTreeMap<String, Ambiguity> {
    let mut seen: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (declaration, registry_id) in resolved {
        seen.entry(declaration.identity.as_str())
            .or_default()
            .push(registry_id.as_str());
    }
    seen.into_iter()
        .filter(|(_, targets)| targets.len() > 1)
        .map(|(identity, targets)| {
            let agree = targets.iter().all(|target| *target == targets[0]);
            (
                identity.to_string(),
                if agree {
                    Ambiguity::DeclaredMoreThanOnce
                } else {
                    Ambiguity::ConflictingTargets
                },
            )
        })
        .collect()
}

/// Record one link, or re-observe the one already stored for this logical identity.
///
/// Both halves go through `nerve-store`. The lookup uses exactly the columns
/// `idx_contract_link_identity` is built on, so "already stored" here and "a conflict" in SQLite are
/// the same question asked twice, and re-running the scan neither duplicates nor errors.
#[allow(clippy::too_many_arguments)]
fn write_link(
    conn: &Connection,
    repo_id: &str,
    source_state: &str,
    rule: ContractRule,
    manifest: &str,
    declaration: &Declaration,
    neighbour: &Neighbour,
    observed_version: Option<String>,
    ambiguity: Option<Ambiguity>,
) -> Result<RecordedLink> {
    let (form, expected_version) = match &declaration.outcome {
        DeclarationOutcome::Supported { form, version, .. } => (*form, version.clone()),
        DeclarationOutcome::Unsupported(_) => unreachable!("only supported forms are written"),
    };
    let span = format!("{}:{}", declaration.line, declaration.line);
    let target = neighbour
        .target
        .as_ref()
        .expect("a link is only written through a usable entry");

    let evidence = serde_json::json!({
        "rule": rule.as_str(),
        "section": declaration.section,
        "form": form.as_str(),
        "specifier": declaration.specifier,
    })
    .to_string();

    let existing = nerve_store::contract_link_id(
        conn,
        repo_id,
        &nerve_store::ContractLinkIdentity {
            registry_entry_id: &neighbour.entry.registry_id,
            contract_kind: rule.as_str(),
            contract_identity: &declaration.identity,
            source_path: manifest,
            source_span: &span,
            resolution_method: form.resolution_method(),
        },
    )?;

    let inserted = match existing {
        Some(link_id) => {
            nerve_store::touch_contract_link(conn, repo_id, link_id)?;
            false
        }
        None => {
            let row = ContractLinkRow {
                link_id: None,
                source_repository_id: repo_id.to_string(),
                source_state_at_resolution: source_state.to_string(),
                source_entity_id: None,
                source_kind_snapshot: None,
                source_path: manifest.to_string(),
                source_span: span.clone(),
                registry_entry_id: neighbour.entry.registry_id.clone(),
                expected_target_repository_id: neighbour.entry.expected_repository_id.clone(),
                target_state_at_resolution: target.state_id.clone(),
                // Every target snapshot column stays NULL, and that is the honest record rather
                // than an omission: a repository-to-repository dependency names no entity in the
                // neighbour, so there is no kind, name, path or span of one to snapshot. The
                // schema's CHECK ties the four together for exactly the case where there is one.
                target_entity_id: None,
                target_kind_snapshot: None,
                target_name_snapshot: None,
                target_path_snapshot: None,
                target_span_snapshot: None,
                relation_semantics: REPOSITORY_DEPENDENCY_SEMANTICS.to_string(),
                contract_kind: rule.as_str().to_string(),
                contract_identity: declaration.identity.clone(),
                expected_contract_version: expected_version.clone(),
                observed_contract_version: observed_version.clone(),
                resolution_method: form.resolution_method(),
                extractor_id: rule.extractor_id().to_string(),
                extractor_version: CONTRACT_EXTRACTOR_VERSION.to_string(),
                evidence_details: Some(evidence),
                ambiguity: ambiguity.map(|value| value.as_str().to_string()),
                unsupported_reason: None,
                first_seen_at: String::new(),
                last_seen_at: String::new(),
                withdrawn_at: None,
                status: ContractLinkStatus::Active,
            };
            nerve_store::insert_contract_link(conn, repo_id, &row)?;
            true
        }
    };

    Ok(RecordedLink {
        rule,
        manifest: manifest.to_string(),
        section: declaration.section.clone(),
        identity: declaration.identity.clone(),
        form,
        registry_id: neighbour.entry.registry_id.clone(),
        target_repository_id: target.repository_id.clone(),
        resolution_method: form.resolution_method(),
        source_span: span,
        expected_contract_version: expected_version,
        observed_contract_version: observed_version,
        ambiguity,
        inserted,
    })
}

/// The version the target's own manifest declares, when it is readable.
///
/// One file in the neighbour, chosen by the rule that produced the link, resolved through
/// [`canonical_child`] so a symlinked manifest is refused rather than followed. `None` is returned
/// for every failure: a version that could not be read is absent, and absent is not a mismatch.
fn target_declared_version(rule: ContractRule, target_root: &Path) -> Option<String> {
    let path = canonical_child(target_root, Path::new(rule.manifest_file_name())).ok()?;
    let text = read_manifest(&path).ok()?;
    match rule {
        ContractRule::NpmLocalDependency => serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        ContractRule::PythonPathDependency => {
            let value: toml::Value = toml::from_str(&text).ok()?;
            let project = value
                .get("project")
                .and_then(|table| table.get("version"))
                .and_then(toml::Value::as_str);
            let poetry = value
                .get("tool")
                .and_then(|tool| tool.get("poetry"))
                .and_then(|poetry| poetry.get("version"))
                .and_then(toml::Value::as_str);
            project.or(poetry).map(str::to_string)
        }
    }
}

/// Read a manifest, refusing an oversized or non-UTF-8 one by name.
///
/// The size is taken from the directory entry **before** the file is opened, which is the same order
/// `coverage` and `trace` read their artifacts in: a bound checked after reading is not a bound.
fn read_manifest(path: &Path) -> std::result::Result<String, ManifestRefusal> {
    let metadata = std::fs::metadata(path).map_err(|_| ManifestRefusal::ManifestUnreadable)?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestRefusal::ManifestTooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| ManifestRefusal::ManifestUnreadable)?;
    String::from_utf8(bytes).map_err(|_| ManifestRefusal::ManifestUnreadable)
}

// ---- parsing -----------------------------------------------------------------------------------

/// Where a supported declaration says its target is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeclaredPath {
    /// The declaration states the path itself, relative to the manifest's directory or absolute.
    Stated(String),
    /// The declaration states no path; the source manifest's `workspaces` array does.
    WorkspaceMember,
}

/// What a declaration turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeclarationOutcome {
    /// A form Nerve reads, with the path it names and the version it asks for.
    Supported {
        form: SupportedForm,
        path: DeclaredPath,
        version: Option<String>,
    },
    /// A form Nerve declines, named.
    Unsupported(UnsupportedForm),
}

/// One dependency declaration, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Declaration {
    section: String,
    identity: String,
    specifier: String,
    line: usize,
    outcome: DeclarationOutcome,
}

/// One manifest, parsed.
struct ParsedManifest {
    declarations: Vec<Declaration>,
    workspace_paths: Vec<String>,
    declined_workspace_globs: usize,
}

fn parse_manifest(
    rule: ContractRule,
    text: &str,
) -> std::result::Result<ParsedManifest, ManifestRefusal> {
    match rule {
        ContractRule::NpmLocalDependency => parse_package_json(text),
        ContractRule::PythonPathDependency => parse_pyproject(text),
    }
}

/// The three `package.json` sections C1 reads.
const NPM_SECTIONS: [&str; 3] = ["dependencies", "devDependencies", "peerDependencies"];

fn parse_package_json(text: &str) -> std::result::Result<ParsedManifest, ManifestRefusal> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| ManifestRefusal::ManifestUnparsable)?;
    let lines = json_key_lines(text, &NPM_SECTIONS);

    let mut workspace_paths = Vec::new();
    let mut declined_workspace_globs = 0usize;
    if let Some(entries) = workspace_entries(&value) {
        for entry in entries {
            if entry.contains(['*', '?', '[', '{']) {
                declined_workspace_globs += 1;
            } else {
                workspace_paths.push(entry);
            }
        }
    }

    let mut declarations = Vec::new();
    for section in NPM_SECTIONS {
        let Some(table) = value.get(section).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, specifier) in table {
            let line = lines
                .get(&(section.to_string(), name.clone()))
                .or_else(|| lines.get(&(String::new(), section.to_string())))
                .copied()
                .unwrap_or(1);
            let (specifier_text, outcome) = match specifier.as_str() {
                Some(text) => (text.to_string(), classify_npm(text)),
                None => (
                    specifier.to_string(),
                    DeclarationOutcome::Unsupported(UnsupportedForm::NpmNonStringSpecifier),
                ),
            };
            declarations.push(Declaration {
                section: section.to_string(),
                identity: name.clone(),
                specifier: specifier_text,
                line,
                outcome,
            });
        }
    }

    Ok(ParsedManifest {
        declarations,
        workspace_paths,
        declined_workspace_globs,
    })
}

/// The `workspaces` array, in either of the two shapes npm and yarn accept.
fn workspace_entries(value: &serde_json::Value) -> Option<Vec<String>> {
    let workspaces = value.get("workspaces")?;
    let array = match workspaces {
        serde_json::Value::Array(array) => array,
        serde_json::Value::Object(object) => object.get("packages")?.as_array()?,
        _ => return None,
    };
    Some(
        array
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

/// Classify one npm specifier. The whole of C1's supported syntax is this function.
fn classify_npm(specifier: &str) -> DeclarationOutcome {
    let trimmed = specifier.trim();

    if let Some(path) = trimmed.strip_prefix("file:") {
        if path.is_empty() {
            return DeclarationOutcome::Unsupported(UnsupportedForm::NpmUnsupportedProtocol);
        }
        return DeclarationOutcome::Supported {
            form: SupportedForm::NpmFilePath,
            path: DeclaredPath::Stated(path.to_string()),
            version: None,
        };
    }

    if let Some(rest) = trimmed.strip_prefix("workspace:") {
        let form = match rest {
            "*" => SupportedForm::NpmWorkspaceWildcard,
            "^" => SupportedForm::NpmWorkspaceCaret,
            "~" => SupportedForm::NpmWorkspaceTilde,
            // A `workspace:` remainder that names a path is not one of the four forms §1 lists, and
            // guessing which of the two meanings it has would be inventing the syntax.
            "" => return DeclarationOutcome::Unsupported(UnsupportedForm::NpmUnsupportedProtocol),
            other if other.starts_with('.') || other.starts_with('/') => {
                return DeclarationOutcome::Unsupported(UnsupportedForm::NpmUnsupportedProtocol)
            }
            _ => SupportedForm::NpmWorkspaceVersion,
        };
        return DeclarationOutcome::Supported {
            form,
            path: DeclaredPath::WorkspaceMember,
            version: Some(rest.to_string()),
        };
    }

    if trimmed.starts_with("npm:") {
        return DeclarationOutcome::Unsupported(UnsupportedForm::NpmAliasSpecifier);
    }
    for prefix in [
        "git:",
        "git+",
        "github:",
        "gitlab:",
        "bitbucket:",
        "gist:",
        "ssh://",
    ] {
        if trimmed.starts_with(prefix) {
            return DeclarationOutcome::Unsupported(UnsupportedForm::NpmGitSpecifier);
        }
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return DeclarationOutcome::Unsupported(UnsupportedForm::NpmUrlSpecifier);
    }
    if has_scheme(trimmed) {
        return DeclarationOutcome::Unsupported(UnsupportedForm::NpmUnsupportedProtocol);
    }
    // `owner/repo` with no scheme is npm's GitHub shorthand. It is a network resolution, so it is
    // named as one rather than lumped in with a version range.
    if trimmed.contains('/') && !trimmed.starts_with('.') && !trimmed.starts_with('/') {
        return DeclarationOutcome::Unsupported(UnsupportedForm::NpmGitSpecifier);
    }
    DeclarationOutcome::Unsupported(UnsupportedForm::NpmRegistryRange)
}

/// Does this specifier begin with `scheme:`?
fn has_scheme(specifier: &str) -> bool {
    match specifier.find(':') {
        Some(index) if index > 0 => specifier[..index]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
}

/// The PEP 621 array C3 reads.
const PEP621_SECTION: &str = "project.dependencies";
/// The Poetry table C3 reads.
const POETRY_SECTION: &str = "tool.poetry.dependencies";
/// The uv / PEP 735 table C3 reads.
const UV_SECTION: &str = "tool.uv.sources";

fn parse_pyproject(text: &str) -> std::result::Result<ParsedManifest, ManifestRefusal> {
    let value: toml::Value =
        toml::from_str(text).map_err(|_| ManifestRefusal::ManifestUnparsable)?;
    let lines = toml_lines(text);
    let mut declarations = Vec::new();

    if let Some(array) = value
        .get("project")
        .and_then(|project| project.get("dependencies"))
        .and_then(toml::Value::as_array)
    {
        for entry in array {
            let Some(requirement) = entry.as_str() else {
                declarations.push(Declaration {
                    section: PEP621_SECTION.to_string(),
                    identity: entry.to_string(),
                    specifier: entry.to_string(),
                    line: lines.header_line("project"),
                    outcome: DeclarationOutcome::Unsupported(
                        UnsupportedForm::PythonUnsupportedSource,
                    ),
                });
                continue;
            };
            let (identity, outcome) = classify_pep508(requirement);
            declarations.push(Declaration {
                section: PEP621_SECTION.to_string(),
                identity,
                specifier: requirement.to_string(),
                line: lines.text_line(requirement, "project"),
                outcome,
            });
        }
    }

    for (section, table) in [
        (POETRY_SECTION, poetry_dependencies(&value)),
        (UV_SECTION, uv_sources(&value)),
    ] {
        let Some(table) = table else { continue };
        for (name, entry) in table {
            declarations.push(Declaration {
                section: section.to_string(),
                identity: name.clone(),
                specifier: entry.to_string(),
                line: lines.key_line(section, name),
                outcome: classify_python_source(entry, section),
            });
        }
    }

    Ok(ParsedManifest {
        declarations,
        workspace_paths: Vec::new(),
        declined_workspace_globs: 0,
    })
}

fn poetry_dependencies(value: &toml::Value) -> Option<&toml::Table> {
    value
        .get("tool")?
        .get("poetry")?
        .get("dependencies")?
        .as_table()
}

fn uv_sources(value: &toml::Value) -> Option<&toml::Table> {
    value.get("tool")?.get("uv")?.get("sources")?.as_table()
}

/// Classify one PEP 508 requirement string.
///
/// Only a direct reference to an **absolute, unescaped** `file://` URL is supported. A percent-escape
/// would need a decoder of our own in the expression that decides which directory is opened, and T11
/// already records this project refusing that trade.
fn classify_pep508(requirement: &str) -> (String, DeclarationOutcome) {
    let (name_part, reference) = match requirement.split_once('@') {
        Some((name, reference)) => (name, Some(reference.trim())),
        None => (requirement, None),
    };
    let identity = pep508_name(name_part);

    let Some(reference) = reference else {
        return (identity, {
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonVersionSpecifier)
        });
    };
    let Some(path) = reference.strip_prefix("file://") else {
        return (
            identity,
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonUnsupportedDirectReference),
        );
    };
    if !path.starts_with('/') || path.contains('%') {
        return (
            identity,
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonUnsupportedDirectReference),
        );
    }
    (
        identity,
        DeclarationOutcome::Supported {
            form: SupportedForm::PythonDirectFileUrl,
            path: DeclaredPath::Stated(path.to_string()),
            version: None,
        },
    )
}

/// The distribution name at the head of a PEP 508 requirement.
fn pep508_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// Classify one Poetry or uv dependency table.
fn classify_python_source(entry: &toml::Value, section: &str) -> DeclarationOutcome {
    if entry.as_str().is_some() {
        return DeclarationOutcome::Unsupported(UnsupportedForm::PythonVersionSpecifier);
    }
    let Some(table) = entry.as_table() else {
        return DeclarationOutcome::Unsupported(UnsupportedForm::PythonUnsupportedSource);
    };
    if let Some(path) = table.get("path").and_then(toml::Value::as_str) {
        let form = if section == UV_SECTION {
            SupportedForm::PythonUvSourcePath
        } else {
            SupportedForm::PythonPoetryPath
        };
        return DeclarationOutcome::Supported {
            form,
            path: DeclaredPath::Stated(path.to_string()),
            version: table
                .get("version")
                .and_then(toml::Value::as_str)
                .map(str::to_string),
        };
    }
    if table.contains_key("git") {
        return DeclarationOutcome::Unsupported(UnsupportedForm::PythonGitSource);
    }
    if table.contains_key("url") {
        return DeclarationOutcome::Unsupported(UnsupportedForm::PythonUrlSource);
    }
    if table.contains_key("workspace") {
        return DeclarationOutcome::Unsupported(UnsupportedForm::PythonWorkspaceSource);
    }
    if table.contains_key("version") {
        return DeclarationOutcome::Unsupported(UnsupportedForm::PythonVersionSpecifier);
    }
    DeclarationOutcome::Unsupported(UnsupportedForm::PythonUnsupportedSource)
}

// ---- locating a declaration in its file --------------------------------------------------------

/// The line each `(section, key)` pair is written on, from the raw JSON text.
///
/// `serde_json` carries no spans, so the line is found by a second pass that tracks string
/// boundaries, escapes and brace depth. It is not a second parser: it decides nothing about the
/// document's meaning, and everything the link records comes from `serde_json`'s value. Its only
/// job is to answer *which line was that written on*, so a link is quoted from a place.
///
/// A minified manifest puts every key on line 1, which is not a failure — that is genuinely where
/// they are.
fn json_key_lines(text: &str, sections: &[&str]) -> BTreeMap<(String, String), usize> {
    let mut out = BTreeMap::new();
    let bytes = text.as_bytes();
    let mut line = 1usize;
    let mut depth = 0usize;
    let mut index = 0usize;
    let mut section: Option<String> = None;
    let mut pending: Option<(String, usize)> = None;

    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                line += 1;
                index += 1;
            }
            b'"' => {
                let start_line = line;
                let mut cursor = index + 1;
                let mut literal = String::new();
                while cursor < bytes.len() {
                    match bytes[cursor] {
                        b'\\' => {
                            if bytes.get(cursor + 1) == Some(&b'\n') {
                                line += 1;
                            }
                            cursor += 2;
                        }
                        b'"' => break,
                        b'\n' => {
                            line += 1;
                            literal.push('\n');
                            cursor += 1;
                        }
                        byte => {
                            literal.push(byte as char);
                            cursor += 1;
                        }
                    }
                }
                index = cursor + 1;
                pending = Some((literal, start_line));
            }
            b':' => {
                if let Some((key, key_line)) = pending.take() {
                    if depth == 1 {
                        out.insert((String::new(), key.clone()), key_line);
                        section = sections.contains(&key.as_str()).then_some(key);
                    } else if depth == 2 {
                        if let Some(current) = &section {
                            out.insert((current.clone(), key), key_line);
                        }
                    }
                }
                index += 1;
            }
            b'{' | b'[' => {
                depth += 1;
                pending = None;
                index += 1;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                if depth <= 1 {
                    section = None;
                }
                pending = None;
                index += 1;
            }
            b',' => {
                pending = None;
                index += 1;
            }
            _ => index += 1,
        }
    }
    out
}

/// The line each TOML header and key is written on.
struct TomlLines {
    headers: BTreeMap<String, usize>,
    keys: BTreeMap<(String, String), usize>,
    lines: Vec<String>,
}

impl TomlLines {
    fn header_line(&self, header: &str) -> usize {
        self.headers.get(header).copied().unwrap_or(1)
    }

    fn key_line(&self, header: &str, key: &str) -> usize {
        self.keys
            .get(&(header.to_string(), key.to_string()))
            .copied()
            .unwrap_or_else(|| self.header_line(header))
    }

    /// The first line containing `needle`, or the header's line.
    ///
    /// PEP 621 dependencies are array *entries* rather than keys, so they are located by their own
    /// text. An entry written twice resolves to the first occurrence, which is the same line the
    /// first of the two duplicates is on.
    fn text_line(&self, needle: &str, header: &str) -> usize {
        self.lines
            .iter()
            .position(|line| line.contains(needle))
            .map(|index| index + 1)
            .unwrap_or_else(|| self.header_line(header))
    }
}

fn toml_lines(text: &str) -> TomlLines {
    let mut headers = BTreeMap::new();
    let mut keys = BTreeMap::new();
    let mut current = String::new();

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.starts_with('[') {
            let inner = trimmed
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or_default()
                .trim()
                .replace('"', "");
            headers.entry(inner.clone()).or_insert(line);
            current = inner;
            continue;
        }
        if current.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=') {
            let key = key.trim().trim_matches('"').to_string();
            if !key.is_empty() {
                keys.entry((current.clone(), key)).or_insert(line);
            }
        }
    }

    TomlLines {
        headers,
        keys,
        lines: text.lines().map(str::to_string).collect(),
    }
}

// ---- freshness ---------------------------------------------------------------------------------

/// What a stored link is right now, given what the registry says and what the two states are.
///
/// **Availability is not re-derived**: `availability` is the verdict
/// [`crate::registry::availability_of`] produced, and its own
/// [`RegistryAvailability::freshness`] answer wins whenever there is one. Only when the entry is
/// usable does this function have anything of its own to say, and what it says is a comparison of
/// two recorded states against two current ones.
///
/// `contract_version_mismatch` is **not** produced here. Deciding whether `1.2.3` satisfies `^1.2.0`
/// is range resolution, and Nerve has no resolver and will not invent one; the two versions are
/// recorded on the row so the question stays answerable by whoever implements it.
pub fn link_freshness(
    link: &nerve_store::ContractLinkRow,
    availability: &RegistryAvailability,
    target_state: Option<&str>,
    source_state: &str,
    source_manifest_present: bool,
) -> Option<ContractFreshness> {
    // Availability first, and `registry_entry_removed` therefore wins over `contract_deleted` for a
    // link withdrawn by `nerve repo remove`. That order is the useful one: the entry being retired
    // is *why* the link ended, and the more specific answer is the one with a remedy. A withdrawn
    // link whose entry is still active ended for some other reason, and that is `contract_deleted`.
    if let Some(freshness) = availability.freshness() {
        return Some(freshness);
    }
    if link.status == ContractLinkStatus::Withdrawn {
        return Some(ContractFreshness::ContractDeleted);
    }
    if !source_manifest_present {
        return Some(ContractFreshness::ContractFileMissing);
    }
    let source_changed = link.source_state_at_resolution != source_state;
    let target_changed = link.target_state_at_resolution.as_deref() != target_state;
    match (source_changed, target_changed) {
        (true, true) => Some(ContractFreshness::BothChanged),
        (true, false) => Some(ContractFreshness::SourceChanged),
        (false, true) => Some(ContractFreshness::TargetChanged),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supported(specifier: &str) -> SupportedForm {
        match classify_npm(specifier) {
            DeclarationOutcome::Supported { form, .. } => form,
            other => panic!("{specifier} was not supported: {other:?}"),
        }
    }

    fn declined(specifier: &str) -> UnsupportedForm {
        match classify_npm(specifier) {
            DeclarationOutcome::Unsupported(form) => form,
            other => panic!("{specifier} was supported: {other:?}"),
        }
    }

    /// Every npm form §1 lists as supported, and each of the four `workspace:` shapes.
    #[test]
    fn the_npm_supported_syntax_is_exactly_the_five_forms() {
        assert_eq!(supported("file:../lib"), SupportedForm::NpmFilePath);
        assert_eq!(supported("file:/abs/lib"), SupportedForm::NpmFilePath);
        assert_eq!(
            supported("workspace:*"),
            SupportedForm::NpmWorkspaceWildcard
        );
        assert_eq!(supported("workspace:^"), SupportedForm::NpmWorkspaceCaret);
        assert_eq!(supported("workspace:~"), SupportedForm::NpmWorkspaceTilde);
        assert_eq!(
            supported("workspace:1.2.3"),
            SupportedForm::NpmWorkspaceVersion
        );
        assert_eq!(
            supported("workspace:^1.2.0"),
            SupportedForm::NpmWorkspaceVersion
        );
    }

    /// Everything else is declined **with the form named**, never silently dropped.
    #[test]
    fn every_other_npm_specifier_is_declined_with_its_form_named() {
        assert_eq!(declined("^1.2.3"), UnsupportedForm::NpmRegistryRange);
        assert_eq!(declined("~1.0"), UnsupportedForm::NpmRegistryRange);
        assert_eq!(declined("1.x"), UnsupportedForm::NpmRegistryRange);
        assert_eq!(declined("*"), UnsupportedForm::NpmRegistryRange);
        assert_eq!(declined("latest"), UnsupportedForm::NpmRegistryRange);
        assert_eq!(declined("git:whatever"), UnsupportedForm::NpmGitSpecifier);
        assert_eq!(
            declined("git+https://example.invalid/x.git"),
            UnsupportedForm::NpmGitSpecifier
        );
        assert_eq!(
            declined("github:owner/repo"),
            UnsupportedForm::NpmGitSpecifier
        );
        assert_eq!(declined("owner/repo"), UnsupportedForm::NpmGitSpecifier);
        assert_eq!(
            declined("https://example.invalid/x.tgz"),
            UnsupportedForm::NpmUrlSpecifier
        );
        assert_eq!(
            declined("npm:other@1.0.0"),
            UnsupportedForm::NpmAliasSpecifier
        );
        assert_eq!(
            declined("link:../lib"),
            UnsupportedForm::NpmUnsupportedProtocol
        );
        assert_eq!(
            declined("workspace:"),
            UnsupportedForm::NpmUnsupportedProtocol
        );
        assert_eq!(
            declined("workspace:../lib"),
            UnsupportedForm::NpmUnsupportedProtocol
        );
        assert_eq!(declined("file:"), UnsupportedForm::NpmUnsupportedProtocol);
    }

    #[test]
    fn a_pep508_direct_file_url_is_supported_and_nothing_else_is() {
        let (name, outcome) = classify_pep508("pkg-core @ file:///srv/pkg-core");
        assert_eq!(name, "pkg-core");
        assert_eq!(
            outcome,
            DeclarationOutcome::Supported {
                form: SupportedForm::PythonDirectFileUrl,
                path: DeclaredPath::Stated("/srv/pkg-core".into()),
                version: None,
            }
        );

        for requirement in [
            "requests>=2.31",
            "flask",
            "pkg[extra]==1.0",
            "pkg @ https://example.invalid/pkg.tar.gz",
            "pkg @ git+https://example.invalid/pkg.git",
            // A percent-escape needs a decoder of our own in the expression that chooses a
            // directory. Refused rather than decoded.
            "pkg @ file:///srv/a%20b",
            // A relative file URL is not an absolute one.
            "pkg @ file://./relative",
        ] {
            let (_, outcome) = classify_pep508(requirement);
            assert!(
                matches!(outcome, DeclarationOutcome::Unsupported(_)),
                "{requirement} was accepted"
            );
        }
        assert_eq!(pep508_name("pkg[extra]==1.0"), "pkg");
    }

    #[test]
    fn a_python_source_table_is_read_by_its_key() {
        let table: toml::Value = toml::from_str(
            r#"
            path_dep = { path = "../core" }
            git_dep = { git = "https://example.invalid/x.git" }
            url_dep = { url = "https://example.invalid/x.tar.gz" }
            ws_dep = { workspace = true }
            ver_dep = { version = "^1" }
            str_dep = "^2"
            odd_dep = { markers = "sys_platform == 'linux'" }
            "#,
        )
        .unwrap();
        let read =
            |key: &str, section: &str| classify_python_source(table.get(key).unwrap(), section);

        assert_eq!(
            read("path_dep", POETRY_SECTION),
            DeclarationOutcome::Supported {
                form: SupportedForm::PythonPoetryPath,
                path: DeclaredPath::Stated("../core".into()),
                version: None,
            }
        );
        assert_eq!(
            read("path_dep", UV_SECTION),
            DeclarationOutcome::Supported {
                form: SupportedForm::PythonUvSourcePath,
                path: DeclaredPath::Stated("../core".into()),
                version: None,
            }
        );
        assert_eq!(
            read("git_dep", POETRY_SECTION),
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonGitSource)
        );
        assert_eq!(
            read("url_dep", POETRY_SECTION),
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonUrlSource)
        );
        assert_eq!(
            read("ws_dep", UV_SECTION),
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonWorkspaceSource)
        );
        assert_eq!(
            read("ver_dep", POETRY_SECTION),
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonVersionSpecifier)
        );
        assert_eq!(
            read("str_dep", POETRY_SECTION),
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonVersionSpecifier)
        );
        assert_eq!(
            read("odd_dep", POETRY_SECTION),
            DeclarationOutcome::Unsupported(UnsupportedForm::PythonUnsupportedSource)
        );
    }

    /// A link is quoted from a place, and two sections declaring one name are two places.
    #[test]
    fn a_json_key_is_located_on_its_own_line() {
        let text = "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"a\": \"file:../a\"\n  },\n  \"devDependencies\": {\n    \"a\": \"^1\"\n  }\n}\n";
        let lines = json_key_lines(text, &NPM_SECTIONS);
        assert_eq!(lines.get(&("dependencies".into(), "a".into())), Some(&4));
        assert_eq!(lines.get(&("devDependencies".into(), "a".into())), Some(&7));
        // A key at the top level is recorded under the empty section, which is how a declaration
        // whose own line could not be found still lands on its section rather than on line 1.
        assert_eq!(lines.get(&(String::new(), "dependencies".into())), Some(&3));
    }

    #[test]
    fn a_toml_key_is_located_under_its_own_header() {
        let text = "[project]\nname = \"s\"\ndependencies = [\n  \"pkg @ file:///x\",\n]\n\n[tool.poetry.dependencies]\npkg = { path = \"../pkg\" }\n";
        let lines = toml_lines(text);
        assert_eq!(lines.header_line("project"), 1);
        assert_eq!(lines.key_line("tool.poetry.dependencies", "pkg"), 8);
        assert_eq!(lines.text_line("pkg @ file:///x", "project"), 4);
        // An unknown key falls back to its header, which is a real place in the file.
        assert_eq!(lines.key_line("tool.poetry.dependencies", "absent"), 7);
    }

    #[test]
    fn every_vocabulary_name_is_distinct_and_lower_case() {
        let mut names: Vec<&str> = Vec::new();
        names.extend(SupportedForm::ALL.iter().map(|form| form.as_str()));
        names.extend(UnsupportedForm::ALL.iter().map(|form| form.as_str()));
        names.extend(UnresolvedReason::ALL.iter().map(|form| form.as_str()));
        names.extend(ManifestRefusal::ALL.iter().map(|form| form.as_str()));
        names.extend(ScanRefusal::ALL.iter().map(|form| form.as_str()));
        names.extend(Ambiguity::ALL.iter().map(|form| form.as_str()));
        names.extend(ContractRule::ALL.iter().map(|rule| rule.as_str()));
        let total = names.len();
        assert!(total >= 35, "only {total} names");
        for name in &names {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name} is not a canonical lower-case name"
            );
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two vocabulary values share a name");
    }

    /// The mapping §2 asks to be documented, asserted rather than described.
    #[test]
    fn each_supported_form_maps_to_one_resolution_method() {
        use ContractResolutionMethod::*;
        assert_eq!(
            SupportedForm::NpmFilePath.resolution_method(),
            ManifestDeclared
        );
        assert_eq!(
            SupportedForm::NpmWorkspaceWildcard.resolution_method(),
            WorkspaceDeclared
        );
        assert_eq!(
            SupportedForm::NpmWorkspaceVersion.resolution_method(),
            WorkspaceDeclared
        );
        assert_eq!(
            SupportedForm::PythonDirectFileUrl.resolution_method(),
            ManifestDeclared
        );
        assert_eq!(
            SupportedForm::PythonPoetryPath.resolution_method(),
            PathDependencyResolved
        );
        assert_eq!(
            SupportedForm::PythonUvSourcePath.resolution_method(),
            PathDependencyResolved
        );
        // `export_map_resolved` belongs to C2, which is 13c. No form here produces it.
        assert!(SupportedForm::ALL
            .iter()
            .all(|form| form.resolution_method() != ExportMapResolved));
    }
}
