//! C1, C2 and C3: the three cross-repository contracts that can be read from a manifest with no new
//! dependency (Slices 13b and 13c).
//!
//! **C1** is an npm local or workspace dependency in `package.json`; **C3** is a Python local path
//! dependency in `pyproject.toml`; **C2** resolves an npm *import specifier* through the neighbour's
//! own `exports` map to a file inside it. C1 and C3 are *repository-to-repository*: neither end is
//! an entity Nerve models. C2 does name an entity on both ends — and it still emits no `Relation`,
//! no `assertion` and no `observation`, and still lives in `contract_link` alone.
//! `docs/plans/slice-13-cross-repository-contracts.md` §4.3 as corrected on 2026-08-08 is the
//! argument, and it is mechanical rather than stylistic: `assertion.target_entity_id` is
//! `NOT NULL REFERENCES entity(entity_id)` (`schema.rs:97`) with `PRAGMA foreign_keys=ON`
//! (`db.rs:37`), and C2's target has no row in **this** database. The question is not *does this
//! have entities on both ends* but *are both ends in this database*, and for all three the answer
//! is no.
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
//! # C2, and the four declarations one link rests on
//!
//! C2 is the one rule in this row that reaches a **file entity inside the target**, and every step
//! of the chain is a stated declaration in a file rather than an inference:
//!
//! 1. **This import specifier.** A module in *this* repository writes `import … from 'pkg-b/sub'`.
//!    The specifier is read from [`crate::facts::ModuleFacts::import_specifiers`], which indexing
//!    already cached, so no file is re-parsed and no source text is read.
//! 2. **This `file:` / `workspace:` dependency.** The specifier's *package part* must be the key of
//!    a C1 declaration this same scan resolved. That is the gate, and it is why `react` is not a C2
//!    declaration at all: its dependency is a registry range, which C1 declines, so there is no
//!    stated path for the specifier to travel along and nothing was read to be dropped.
//! 3. **This `name`.** The neighbour's own `package.json` must declare the package part as its
//!    `name`. A dependency key that disagrees with the target's declared name resolves to nothing —
//!    a key is what npm *aliases* with, and an alias is a name, and a name is not evidence
//!    (`CLAUDE.md` §3).
//! 4. **This `exports` entry.** The subpath is resolved through the neighbour's declared `exports`,
//!    or through `module` / `main` / `types` when the neighbour declares no `exports` at all.
//!
//! Only then is the resolved path looked up in the neighbour's index, **read-only**, to snapshot the
//! entity it names. A path that exists in the neighbour and has no entity is
//! [`nerve_core::vocab::ContractFreshness::TargetPartiallyIndexed`] — part of the target was never
//! looked at — and never a missing target, which is Slice 7c-i's `Stale` / `Unverified` distinction
//! in its fourth place.
//!
//! **The link is still not a local assertion.** `relation_semantics` records `REFERENCES` because
//! that is the honest name for what the manifest declares, and the string sits in a free-text column
//! of a table that is not the evidence graph. An ordinary `path` or `impact` query cannot reach it,
//! and `crates/nerve-index/tests/contracts.rs` asserts that negatively rather than by inspection.
//!
//! # What this module deliberately does not decide
//!
//! `expected_contract_version` and `observed_contract_version` are both recorded and neither is
//! compared. `^1.2.0` against `1.2.3` is a *range satisfaction* question, and answering it needs a
//! semantic-version resolver — a new dependency, or a parser of our own in the exact expression that
//! decides whether two repositories agree. `contract_version_mismatch` therefore has no producer in
//! this slice, and the evidence for one is stored rather than a verdict invented.
//!
//! Nor does C2 decide what a **wildcard** subpath means, what a `null` export blocks past the
//! blocking, what an unlisted subpath would be if the filesystem were probed for it, or which file a
//! condition Nerve does not support would choose. Each is declined with the form **named**, because
//! resolving any of them would be Nerve inventing a resolution rule and then measuring itself
//! against it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nerve_core::vocab::{ContractFreshness, ContractLinkStatus, ContractResolutionMethod};
use nerve_store::{Connection, ContractLinkRow, EntityRef, RegistryEntryRow};

use crate::config::Config;
use crate::discover::{canonical_child, canonical_root, discover_named};
use crate::error::Result;
use crate::facts::ModuleFacts;
use crate::lang::Language;
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

/// The largest number of entries one neighbour's `exports` map may state.
///
/// Exceeding it declines the **declaration** rather than truncating the map, for the same reason
/// [`MAX_DECLARATIONS_PER_MANIFEST`] refuses a whole manifest: an export map read halfway would
/// resolve the subpaths before the cut and silently miss the rest, and no row anywhere would say
/// which half was read.
pub const MAX_EXPORT_ENTRIES: usize = 512;

/// The deepest subpath C2 will carry into an `exports` lookup, counted in `/`-separated segments
/// after the package name.
///
/// `pkg/a/b` is depth 2. The bound exists because the subpath comes from repository content and is
/// used to build a filesystem path; a specifier deeper than this is declined by name rather than
/// walked.
pub const MAX_EXPORT_SUBPATH_DEPTH: usize = 8;

/// How deeply conditional exports may nest before C2 stops descending.
///
/// `{"import": {"node": {"default": "./x.js"}}}` is depth 3. Beyond the bound the entry is declined
/// with [`UnsupportedForm::NpmExportUnsupportedCondition`], because a condition tree Nerve stopped
/// reading is a condition tree Nerve cannot claim to have resolved.
pub const MAX_EXPORT_CONDITION_DEPTH: usize = 4;

/// The extractor id C3 stamps on every link it records.
pub const PYTHON_EXTRACTOR_ID: &str = "python-path-dependency";

/// The extractor id C2 stamps on every link it records.
pub const NPM_EXPORT_EXTRACTOR_ID: &str = "npm-export-resolution";

/// The export conditions C2 reads, **in the order it reads them**.
///
/// One documented order, applied at every nesting level. `import` comes first because
/// [`crate::facts::ModuleFacts::import_specifiers`] merges `import … from` and `require(…)`
/// specifiers into one set — the cache keeps the specifier, not the syntax that wrote it — so the
/// distinction is not available at this layer and the modern default is preferred rather than
/// guessed at per call site. That is a stated limitation of the rule, not a hidden one: it is why a
/// package whose `import` and `require` conditions name different files can be resolved to the ESM
/// half of the pair, and the condition actually taken is recorded in `evidence_details`.
///
/// Every other condition — `node`, `browser`, `types`, `development`, `production`, a custom one —
/// is **declined by name** as [`UnsupportedForm::NpmExportUnsupportedCondition`] rather than
/// silently falling through.
pub const EXPORT_CONDITION_ORDER: [&str; 3] = ["import", "require", "default"];

/// The legacy entry-point fields C2 reads when a package declares no `exports`, in order.
///
/// `module` first because it is the ESM entry and matches the condition order above; `main` second
/// because it is the field Node itself reads; `types` last, because a declaration file *describes*
/// the implementation rather than being it, and preferring it would point every link at a `.d.ts`.
pub const LEGACY_ENTRY_FIELDS: [&str; 3] = ["module", "main", "types"];

/// The semantic relation an export resolution states.
///
/// `REFERENCES` already means "this names that", and the rule that relation names are
/// endpoint-kind-agnostic says the cross-repository-ness belongs in the evidence rather than in the
/// name. This is **not** a member of [`nerve_core::vocab::Relation`] and never a row in `assertion`:
/// it is the description a response renders for a link in `contract_link`, which no local traversal
/// reads.
pub const EXPORT_REFERENCE_SEMANTICS: &str = "REFERENCES";

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
    /// C2 — an npm import specifier resolved through the neighbour's `exports` map.
    NpmExportResolution,
    /// C3 — a Python local path dependency in `pyproject.toml`.
    PythonPathDependency,
}

impl ContractRule {
    /// Every value, in declaration order.
    pub const ALL: [ContractRule; 3] = [
        ContractRule::NpmLocalDependency,
        ContractRule::NpmExportResolution,
        ContractRule::PythonPathDependency,
    ];

    /// The rules a scan reaches by **discovering a manifest**.
    ///
    /// C2 is absent, and its absence is the point. C2 is driven by the *imports* of a module this
    /// repository has already indexed, not by finding a file — it reads `package.json`, but only
    /// after a C1 declaration in that same file has named a neighbour. Letting a file name reach it
    /// would run it twice over one manifest and give the same declaration two rules.
    pub const MANIFEST_DRIVEN: [ContractRule; 2] = [
        ContractRule::NpmLocalDependency,
        ContractRule::PythonPathDependency,
    ];

    /// Canonical lower-case name. This is what `contract_link.contract_kind` stores.
    pub fn as_str(self) -> &'static str {
        match self {
            ContractRule::NpmLocalDependency => "npm_local_dependency",
            ContractRule::NpmExportResolution => "npm_export_resolution",
            ContractRule::PythonPathDependency => "python_path_dependency",
        }
    }

    /// The manifest file name this rule reads.
    pub fn manifest_file_name(self) -> &'static str {
        match self {
            ContractRule::NpmLocalDependency | ContractRule::NpmExportResolution => "package.json",
            ContractRule::PythonPathDependency => "pyproject.toml",
        }
    }

    /// The extractor id this rule stamps on a link.
    pub fn extractor_id(self) -> &'static str {
        match self {
            ContractRule::NpmLocalDependency => NPM_EXTRACTOR_ID,
            ContractRule::NpmExportResolution => NPM_EXPORT_EXTRACTOR_ID,
            ContractRule::PythonPathDependency => PYTHON_EXTRACTOR_ID,
        }
    }

    /// Which manifest-driven rule reads a file with this name, if any.
    pub fn for_file_name(name: &str) -> Option<ContractRule> {
        ContractRule::MANIFEST_DRIVEN
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
/// The set is closed and small on purpose. Every C1 and C3 member names a *path* — directly, or
/// through the source manifest's own `workspaces` array — because a path is the only thing in a
/// manifest that can be resolved to a repository without asking a network registry what a name
/// means. Every C2 member names a *file inside a repository already reached that way*.
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
    /// npm `import … from 'pkg-b/sub'` — the specifier itself, read in full.
    ///
    /// This is the form a C2 declaration carries **before** an export entry has been read, so it is
    /// what an [`UnresolvedDeclaration`] reports. A recorded link never carries it: by the time
    /// there is a link, the export declaration that produced it is known and is the more specific
    /// answer.
    NpmImportSpecifier,
    /// npm `"exports": "./src/index.ts"` — the whole map is one string, for the `.` subpath.
    NpmExportsString,
    /// npm `"exports": { "./sub": "./src/sub.ts" }` — a subpath map entry naming a file directly.
    NpmExportsSubpath,
    /// npm `"exports": { ".": { "import": "./src/esm.ts" } }` — resolved by
    /// [`EXPORT_CONDITION_ORDER`].
    NpmExportsConditional,
    /// npm `"module": "./src/index.ts"`, when the package declares no `exports`.
    NpmLegacyModule,
    /// npm `"main": "./src/index.js"`, when the package declares no `exports` and no `module`.
    NpmLegacyMain,
    /// npm `"types": "./src/index.d.ts"`, the last legacy field and the least direct.
    NpmLegacyTypes,
}

impl SupportedForm {
    /// Every value, in declaration order.
    pub const ALL: [SupportedForm; 15] = [
        SupportedForm::NpmFilePath,
        SupportedForm::NpmWorkspaceWildcard,
        SupportedForm::NpmWorkspaceCaret,
        SupportedForm::NpmWorkspaceTilde,
        SupportedForm::NpmWorkspaceVersion,
        SupportedForm::PythonDirectFileUrl,
        SupportedForm::PythonPoetryPath,
        SupportedForm::PythonUvSourcePath,
        SupportedForm::NpmImportSpecifier,
        SupportedForm::NpmExportsString,
        SupportedForm::NpmExportsSubpath,
        SupportedForm::NpmExportsConditional,
        SupportedForm::NpmLegacyModule,
        SupportedForm::NpmLegacyMain,
        SupportedForm::NpmLegacyTypes,
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
            SupportedForm::NpmImportSpecifier => "npm_import_specifier",
            SupportedForm::NpmExportsString => "npm_exports_string",
            SupportedForm::NpmExportsSubpath => "npm_exports_subpath",
            SupportedForm::NpmExportsConditional => "npm_exports_conditional",
            SupportedForm::NpmLegacyModule => "npm_legacy_module",
            SupportedForm::NpmLegacyMain => "npm_legacy_main",
            SupportedForm::NpmLegacyTypes => "npm_legacy_types",
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
            SupportedForm::NpmImportSpecifier
            | SupportedForm::NpmExportsString
            | SupportedForm::NpmExportsSubpath
            | SupportedForm::NpmExportsConditional
            | SupportedForm::NpmLegacyModule
            | SupportedForm::NpmLegacyMain
            | SupportedForm::NpmLegacyTypes => ContractRule::NpmExportResolution,
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
    /// | every C2 form | `export_map_resolved` | a subpath resolved through the target's own map |
    ///
    /// `export_map_resolved` is the value 13b deliberately left with no producer. C2 is its
    /// producer, and it is the only one: the six C2 forms differ in *which* declaration was read,
    /// which is `SupportedForm` itself, not in how the target was reached.
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
            SupportedForm::NpmImportSpecifier
            | SupportedForm::NpmExportsString
            | SupportedForm::NpmExportsSubpath
            | SupportedForm::NpmExportsConditional
            | SupportedForm::NpmLegacyModule
            | SupportedForm::NpmLegacyMain
            | SupportedForm::NpmLegacyTypes => ContractResolutionMethod::ExportMapResolved,
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
    /// An `exports` subpath that only a wildcard key such as `"./*"` would match. Nerve does not
    /// expand a pattern into a path.
    NpmExportWildcardSubpath,
    /// An `exports` entry whose value is `null` — a deliberate block, recorded as one.
    NpmExportBlocked,
    /// A conditions object naming none of [`EXPORT_CONDITION_ORDER`], or nesting deeper than
    /// [`MAX_EXPORT_CONDITION_DEPTH`].
    NpmExportUnsupportedCondition,
    /// An `exports` value that is neither a string, an object nor `null` — an array fallback, a
    /// number, a boolean.
    NpmExportNonStringTarget,
    /// A resolved export target that leaves the neighbour's root, lexically or through a symlink.
    NpmExportPathEscapesTarget,
    /// The package declares an `exports` map and that map declares no matching subpath. A closed
    /// map is a deliberate statement about what is *not* exported.
    NpmExportSubpathNotDeclared,
    /// An `exports` object mixing `"."`-prefixed subpath keys with condition keys, which is not a
    /// shape Node accepts and not one Nerve will guess at.
    NpmExportsMixedKeys,
    /// A subpath import against a package that declares no `exports`. Resolving it would mean
    /// probing the filesystem and guessing an extension — a resolution rule Nerve would be
    /// inventing.
    NpmLegacySubpathProbe,
    /// The neighbour's `exports` map states more entries than [`MAX_EXPORT_ENTRIES`].
    NpmExportMapTooLarge,
    /// The specifier's subpath is deeper than [`MAX_EXPORT_SUBPATH_DEPTH`].
    NpmExportSubpathTooDeep,
}

impl UnsupportedForm {
    /// Every value, in declaration order.
    pub const ALL: [UnsupportedForm; 23] = [
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
        UnsupportedForm::NpmExportWildcardSubpath,
        UnsupportedForm::NpmExportBlocked,
        UnsupportedForm::NpmExportUnsupportedCondition,
        UnsupportedForm::NpmExportNonStringTarget,
        UnsupportedForm::NpmExportPathEscapesTarget,
        UnsupportedForm::NpmExportSubpathNotDeclared,
        UnsupportedForm::NpmExportsMixedKeys,
        UnsupportedForm::NpmLegacySubpathProbe,
        UnsupportedForm::NpmExportMapTooLarge,
        UnsupportedForm::NpmExportSubpathTooDeep,
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
            UnsupportedForm::NpmExportWildcardSubpath => "npm_export_wildcard_subpath",
            UnsupportedForm::NpmExportBlocked => "npm_export_blocked",
            UnsupportedForm::NpmExportUnsupportedCondition => "npm_export_unsupported_condition",
            UnsupportedForm::NpmExportNonStringTarget => "npm_export_non_string_target",
            UnsupportedForm::NpmExportPathEscapesTarget => "npm_export_path_escapes_target",
            UnsupportedForm::NpmExportSubpathNotDeclared => "npm_export_subpath_not_declared",
            UnsupportedForm::NpmExportsMixedKeys => "npm_exports_mixed_keys",
            UnsupportedForm::NpmLegacySubpathProbe => "npm_legacy_subpath_probe",
            UnsupportedForm::NpmExportMapTooLarge => "npm_export_map_too_large",
            UnsupportedForm::NpmExportSubpathTooDeep => "npm_export_subpath_too_deep",
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
            UnsupportedForm::NpmExportWildcardSubpath
            | UnsupportedForm::NpmExportBlocked
            | UnsupportedForm::NpmExportUnsupportedCondition
            | UnsupportedForm::NpmExportNonStringTarget
            | UnsupportedForm::NpmExportPathEscapesTarget
            | UnsupportedForm::NpmExportSubpathNotDeclared
            | UnsupportedForm::NpmExportsMixedKeys
            | UnsupportedForm::NpmLegacySubpathProbe
            | UnsupportedForm::NpmExportMapTooLarge
            | UnsupportedForm::NpmExportSubpathTooDeep => ContractRule::NpmExportResolution,
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
    /// C2: the neighbour reached through the dependency does not declare the specifier's package
    /// part as its own `name`. The dependency key is an alias, and an alias is a name.
    PackageNameNotDeclared,
    /// C2: the neighbour has no `package.json` this build can read, so there is no `exports` map,
    /// no `name` and nothing to resolve against.
    TargetManifestUnreadable,
    /// C2: the neighbour declares no `exports`, no `module`, no `main` and no `types`. It states no
    /// entry point, so there is no file it says the specifier means.
    TargetDeclaresNoEntryPoint,
    /// C2: the export entry was read and names a path that is not in the neighbour's tree.
    ExportTargetMissing,
}

impl UnresolvedReason {
    /// Every value, in declaration order.
    pub const ALL: [UnresolvedReason; 10] = [
        UnresolvedReason::DeclaredPathMissing,
        UnresolvedReason::DeclaredPathInSameRepository,
        UnresolvedReason::DeclaredPathNotARepositoryRoot,
        UnresolvedReason::TargetNotRegistered,
        UnresolvedReason::RegistryEntryUnusable,
        UnresolvedReason::WorkspaceMemberNotDeclared,
        UnresolvedReason::PackageNameNotDeclared,
        UnresolvedReason::TargetManifestUnreadable,
        UnresolvedReason::TargetDeclaresNoEntryPoint,
        UnresolvedReason::ExportTargetMissing,
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
            UnresolvedReason::PackageNameNotDeclared => "package_name_not_declared",
            UnresolvedReason::TargetManifestUnreadable => "target_manifest_unreadable",
            UnresolvedReason::TargetDeclaresNoEntryPoint => "target_declares_no_entry_point",
            UnresolvedReason::ExportTargetMissing => "export_target_missing",
        }
    }
}

impl std::fmt::Display for UnresolvedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a manifest, a source file, or the rest of a scan, was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManifestRefusal {
    /// The file is larger than [`MAX_MANIFEST_BYTES`].
    ManifestTooLarge,
    /// The file states more than [`MAX_DECLARATIONS_PER_MANIFEST`] declarations.
    TooManyDeclarations,
    /// The file could not be read, or is not UTF-8.
    ManifestUnreadable,
    /// The file is not valid JSON or TOML, or its cached extraction payload is not readable by this
    /// build.
    ManifestUnparsable,
    /// The repository already holds [`MAX_LINKS_PER_REPOSITORY`] links.
    LinkBudgetExhausted,
    /// C2: a module has cached import specifiers and no entity at its path, so a link read from it
    /// would have no local end. The file is passed over **by name** rather than dropped, because a
    /// source entity that vanished is a fact about this repository's index rather than about the
    /// neighbour.
    SourceModuleNotIndexed,
}

impl ManifestRefusal {
    /// Every value, in declaration order.
    pub const ALL: [ManifestRefusal; 6] = [
        ManifestRefusal::ManifestTooLarge,
        ManifestRefusal::TooManyDeclarations,
        ManifestRefusal::ManifestUnreadable,
        ManifestRefusal::ManifestUnparsable,
        ManifestRefusal::LinkBudgetExhausted,
        ManifestRefusal::SourceModuleNotIndexed,
    ];

    /// Canonical lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            ManifestRefusal::ManifestTooLarge => "manifest_too_large",
            ManifestRefusal::TooManyDeclarations => "too_many_declarations",
            ManifestRefusal::ManifestUnreadable => "manifest_unreadable",
            ManifestRefusal::ManifestUnparsable => "manifest_unparsable",
            ManifestRefusal::LinkBudgetExhausted => "link_budget_exhausted",
            ManifestRefusal::SourceModuleNotIndexed => "source_module_not_indexed",
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
///
/// The three naming fields mean slightly different things per rule, and the difference is stated
/// rather than left to be inferred:
///
/// | rule | `manifest` | `section` | `identity` |
/// |---|---|---|---|
/// | C1 / C3 | the manifest read | the manifest section | the dependency key |
/// | C2 | the **importing module's** path, which is also `source_path` | the package name the specifier names | the import specifier as written |
///
/// C2's `(manifest, identity)` is the pair that identifies a declaration: the same specifier
/// written in two files is two declarations, and one file writing two specifiers for one package is
/// two as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedLink {
    /// Which rule read it.
    pub rule: ContractRule,
    /// Repository-relative path of the manifest, or of the importing module for C2.
    pub manifest: String,
    /// Which manifest section the declaration sits in, or the package name for C2.
    pub section: String,
    /// The dependency key, or the import specifier for C2. Untrusted repository content.
    pub identity: String,
    /// The declared form.
    pub form: SupportedForm,
    /// The registry entry the link resolved through.
    pub registry_id: String,
    /// The repository id found at the declared path.
    pub target_repository_id: String,
    /// Which stated declaration it was drawn from.
    pub resolution_method: ContractResolutionMethod,
    /// Where in the manifest, as `line:line`. For C2, the importing module's own span.
    pub source_span: String,
    /// The semantic relation the declaration states. `DEPENDS_ON` for C1 and C3, `REFERENCES` for
    /// C2. **Not** a [`nerve_core::vocab::Relation`] and never a row in `assertion`.
    pub relation_semantics: &'static str,
    /// The local entity the declaration sits in, when the contract has one. C2 always has one; C1
    /// and C3 never do, because a repository-to-repository dependency names no entity.
    pub source_entity_id: Option<String>,
    /// The target entity's id **in the neighbour's database**, when it was indexed there.
    ///
    /// `None` with [`RecordedLink::target_path`] set means the file is in the neighbour's tree and
    /// its index has no entity for it: `target_partially_indexed`, not a missing target.
    pub target_entity_id: Option<String>,
    /// The neighbour-relative path the specifier resolved to, when one was resolved.
    pub target_path: Option<String>,
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
    let names: Vec<&str> = ContractRule::MANIFEST_DRIVEN
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

    // Every C1 declaration, resolved or not, in the order the manifests were read. This is C2's
    // gate: an import specifier is a C2 declaration only when its package part is one of these
    // keys, which is what keeps `react` out of the scan entirely rather than in it as an
    // unresolved row nobody can act on.
    let mut npm_dependencies: Vec<NpmDependency> = Vec::new();

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
                if rule == ContractRule::NpmLocalDependency {
                    npm_dependencies.push(NpmDependency {
                        key: declaration.identity.clone(),
                        manifest: manifest.rel_path.clone(),
                        section: declaration.section.clone(),
                        verdict: Err(UnresolvedReason::WorkspaceMemberNotDeclared),
                    });
                }
                continue;
            };

            let verdict = resolutions
                .entry(target_root.clone())
                .or_insert_with(|| resolve_target(&root, &target_root, &neighbours))
                .clone();

            if rule == ContractRule::NpmLocalDependency {
                npm_dependencies.push(NpmDependency {
                    key: declaration.identity.clone(),
                    manifest: manifest.rel_path.clone(),
                    section: declaration.section.clone(),
                    verdict: verdict.clone(),
                });
            }

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

    // C2 runs last and only on what C1 already resolved. It reads no manifest of this repository
    // that C1 did not already read, and it reaches the neighbour only through a registry entry the
    // dependency named.
    resolve_export_contracts(
        &tx,
        repo_id,
        &source_state,
        &neighbours,
        &npm_dependencies,
        &mut scan,
        &mut budget,
    )?;

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
        relation_semantics: REPOSITORY_DEPENDENCY_SEMANTICS,
        source_entity_id: None,
        target_entity_id: None,
        target_path: None,
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
        // Both npm rules read the same field of the same file, so they share an arm rather than
        // duplicating it: `version` in `package.json` is one fact whichever rule asked.
        ContractRule::NpmLocalDependency | ContractRule::NpmExportResolution => {
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()?
                .get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        }
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

// ---- C2: the export map --------------------------------------------------------------------

/// One C1 declaration, kept so that C2 can decide whether a specifier has a stated path to travel.
///
/// This is the second link in C2's evidence chain and the reason `react` never becomes a C2
/// declaration: a registry range is not a supported C1 form, so it never reaches this table, so no
/// import specifier naming it is ever considered.
struct NpmDependency {
    /// The dependency key, which is what an import specifier must name.
    key: String,
    /// Repository-relative path of the `package.json` it was declared in.
    manifest: String,
    /// Which section of that manifest declared it.
    section: String,
    /// The registry entry it resolved through, or the reason it reached nothing.
    verdict: std::result::Result<String, UnresolvedReason>,
}

/// One neighbour's `package.json` and index, read at most once per scan.
///
/// Both reads are of the neighbour and both are bounded: the manifest through [`read_manifest`],
/// the index through [`crate::registry::open_target_index`], which is the only place in the product
/// that opens somebody else's database.
struct TargetPackage {
    /// The neighbour's parsed `package.json`, or `None` when it could not be read.
    manifest: Option<serde_json::Value>,
    /// The canonical root of the neighbour.
    root: PathBuf,
    /// The neighbour's `state_id` at resolution.
    state_id: Option<String>,
    /// The neighbour's `repo_id` as read at resolution.
    repository_id: String,
    /// The neighbour's index, read-only, or `None` when it could not be opened.
    index: Option<Connection>,
}

impl TargetPackage {
    fn read(target: &RegistryTarget) -> TargetPackage {
        let manifest = canonical_child(&target.root, Path::new("package.json"))
            .ok()
            .and_then(|path| read_manifest(&path).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        TargetPackage {
            manifest,
            root: target.root.clone(),
            state_id: target.state_id.clone(),
            repository_id: target.repository_id.clone(),
            index: crate::registry::open_target_index(&target.root).ok(),
        }
    }

    /// The `name` the neighbour declares for itself. The third link in the evidence chain.
    fn declared_name(&self) -> Option<&str> {
        self.manifest.as_ref()?.get("name")?.as_str()
    }

    /// The `version` the neighbour declares. Recorded, never compared — see the module header.
    fn declared_version(&self) -> Option<String> {
        Some(
            self.manifest
                .as_ref()?
                .get("version")?
                .as_str()?
                .to_string(),
        )
    }
}

/// What resolving one specifier through one neighbour's `exports` produced.
enum ExportOutcome {
    /// A neighbour-relative path, and which declaration named it.
    Resolved {
        form: SupportedForm,
        target: String,
        matched: String,
        condition: Option<String>,
    },
    /// A form Nerve declines to read, named.
    Declined(UnsupportedForm),
    /// A form Nerve read in full that reached nothing, named.
    Unresolved(UnresolvedReason),
}

/// What descending a conditions object produced.
enum ConditionOutcome {
    Resolved { target: String, condition: String },
    Declined(UnsupportedForm),
}

/// One link C2 is about to write, held until the ambiguity across its siblings is known.
struct PendingExportLink {
    registry_id: String,
    expected_repository_id: String,
    target_repository_id: String,
    target_state: Option<String>,
    form: SupportedForm,
    matched: String,
    condition: Option<String>,
    target_rel: String,
    target_entity: Option<EntityRef>,
    observed_version: Option<String>,
    declared_manifest: String,
    declared_section: String,
}

/// Resolve every import specifier that a C1 declaration gave a stated path to.
///
/// The order of operations is the evidence chain, and each step can only narrow what the next sees:
///
/// 1. Nothing runs at all unless C1 produced a declaration. An empty `dependencies` slice returns
///    immediately, which is why a repository with no registered npm neighbour pays nothing.
/// 2. Import specifiers come from [`crate::facts::ModuleFacts`], which indexing already wrote. No
///    file in this repository is re-read and no source text is loaded.
/// 3. A specifier is a C2 declaration only when its package part is a C1 dependency key.
/// 4. The neighbour must declare that package part as its own `name`.
/// 5. The subpath must be named by the neighbour's own `exports`, or by `module` / `main` / `types`
///    when it declares no `exports` at all.
///
/// Only then is the neighbour's index opened, read-only, to snapshot the entity at the resolved
/// path. A resolved path with no entity behind it is recorded **with the path** and without an
/// entity id, which is what makes `target_partially_indexed` a stored fact rather than a guess.
#[allow(clippy::too_many_arguments)]
fn resolve_export_contracts(
    conn: &Connection,
    repo_id: &str,
    source_state: &str,
    neighbours: &[Neighbour],
    dependencies: &[NpmDependency],
    scan: &mut ContractScan,
    budget: &mut usize,
) -> Result<()> {
    if dependencies.is_empty() {
        return Ok(());
    }
    let facts = nerve_store::load_module_facts(conn, repo_id)?;
    let mut packages: BTreeMap<String, TargetPackage> = BTreeMap::new();

    for (rel_path, row) in &facts {
        if !is_ts_js(&row.language) {
            continue;
        }
        let Some(module) = ModuleFacts::from_json(&row.facts) else {
            // A cached payload this build cannot read is a file whose imports were not examined.
            // Recorded by name rather than skipped: "we did not look" is not "there was nothing".
            scan.refusals
                .push((rel_path.clone(), ManifestRefusal::ManifestUnparsable));
            continue;
        };

        let candidates: Vec<(&str, &str, String)> = module
            .import_specifiers
            .iter()
            .filter_map(|specifier| {
                let (package, subpath) = split_specifier(specifier)?;
                dependencies
                    .iter()
                    .any(|dependency| dependency.key == package)
                    .then_some((specifier.as_str(), package, subpath))
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }

        let Some(source) = local_entity(conn, rel_path)? else {
            scan.refusals
                .push((rel_path.clone(), ManifestRefusal::SourceModuleNotIndexed));
            continue;
        };

        for (specifier, package, subpath) in candidates {
            scan.declarations += 1;
            let declared: Vec<&NpmDependency> = dependencies
                .iter()
                .filter(|dependency| dependency.key == package)
                .collect();

            // How many declarations named a usable entry, and which entries they were. Both
            // numbers are needed: one entry named twice is `declared_more_than_once`, and two
            // entries named once each is `conflicting_targets`.
            let named: Vec<&str> = declared
                .iter()
                .filter_map(|dependency| dependency.verdict.as_deref().ok())
                .collect();
            if named.is_empty() {
                let reason = declared
                    .iter()
                    .find_map(|dependency| dependency.verdict.as_ref().err().copied())
                    .expect("a declaration with no usable entry carries a reason");
                scan.unresolved.push(UnresolvedDeclaration {
                    rule: ContractRule::NpmExportResolution,
                    manifest: rel_path.clone(),
                    section: package.to_string(),
                    identity: specifier.to_string(),
                    form: SupportedForm::NpmImportSpecifier,
                    reason,
                });
                continue;
            }

            let mut distinct: Vec<&str> = named.clone();
            distinct.sort_unstable();
            distinct.dedup();

            let mut pending: Vec<PendingExportLink> = Vec::new();
            let mut declined: Vec<UnsupportedForm> = Vec::new();
            let mut unresolved: Vec<UnresolvedReason> = Vec::new();

            for registry_id in &distinct {
                let neighbour = neighbours
                    .iter()
                    .find(|candidate| candidate.entry.registry_id == **registry_id)
                    .expect("a resolved verdict names an entry in this table");
                let target = neighbour
                    .target
                    .as_ref()
                    .expect("a resolved verdict is only produced through a usable entry");
                let declaration = declared
                    .iter()
                    .find(|dependency| dependency.verdict.as_deref() == Ok(*registry_id))
                    .expect("the entry came from one of these declarations");
                let package_state = packages
                    .entry((*registry_id).to_string())
                    .or_insert_with(|| TargetPackage::read(target));

                if package_state.manifest.is_none() {
                    unresolved.push(UnresolvedReason::TargetManifestUnreadable);
                    continue;
                }
                if package_state.declared_name() != Some(package) {
                    unresolved.push(UnresolvedReason::PackageNameNotDeclared);
                    continue;
                }
                let manifest = package_state
                    .manifest
                    .as_ref()
                    .expect("checked immediately above");

                let (form, target_path, matched, condition) =
                    match resolve_export(manifest, &subpath) {
                        ExportOutcome::Resolved {
                            form,
                            target,
                            matched,
                            condition,
                        } => (form, target, matched, condition),
                        ExportOutcome::Declined(form) => {
                            declined.push(form);
                            continue;
                        }
                        ExportOutcome::Unresolved(reason) => {
                            unresolved.push(reason);
                            continue;
                        }
                    };

                let target_rel = match neighbour_relative(&package_state.root, &target_path) {
                    Ok(relative) => relative,
                    Err(ExportPathRefusal::Escapes) => {
                        declined.push(UnsupportedForm::NpmExportPathEscapesTarget);
                        continue;
                    }
                    Err(ExportPathRefusal::Missing) => {
                        unresolved.push(UnresolvedReason::ExportTargetMissing);
                        continue;
                    }
                };

                let target_entity = package_state
                    .index
                    .as_ref()
                    .and_then(|index| foreign_entity(index, &target_rel));

                pending.push(PendingExportLink {
                    registry_id: (*registry_id).to_string(),
                    expected_repository_id: neighbour.entry.expected_repository_id.clone(),
                    target_repository_id: package_state.repository_id.clone(),
                    target_state: package_state.state_id.clone(),
                    form,
                    matched,
                    condition,
                    target_rel,
                    target_entity,
                    observed_version: package_state.declared_version(),
                    declared_manifest: declaration.manifest.clone(),
                    declared_section: declaration.section.clone(),
                });
            }

            // Ambiguity is decided over the links actually produced, never over the declarations
            // that were merely written: reporting `conflicting_targets` on a lone link because a
            // second declaration declined would describe a conflict that does not exist.
            let ambiguity = if pending.len() > 1 {
                Some(Ambiguity::ConflictingTargets)
            } else if pending.len() == 1 && named.len() > 1 {
                Some(Ambiguity::DeclaredMoreThanOnce)
            } else {
                None
            };

            if pending.is_empty() {
                // Nothing resolved. Every refusal is reported, and a declined *form* is reported
                // ahead of an unresolved *reason* because "we do not read that shape" is the more
                // specific fact and the one the user can act on.
                if let Some(form) = declined.first().copied() {
                    scan.unsupported.push(UnsupportedDeclaration {
                        rule: ContractRule::NpmExportResolution,
                        manifest: rel_path.clone(),
                        section: package.to_string(),
                        identity: specifier.to_string(),
                        form,
                    });
                } else if let Some(reason) = unresolved.first().copied() {
                    scan.unresolved.push(UnresolvedDeclaration {
                        rule: ContractRule::NpmExportResolution,
                        manifest: rel_path.clone(),
                        section: package.to_string(),
                        identity: specifier.to_string(),
                        form: SupportedForm::NpmImportSpecifier,
                        reason,
                    });
                }
                continue;
            }

            for link in &pending {
                if *budget >= MAX_LINKS_PER_REPOSITORY {
                    if !scan
                        .refusals
                        .iter()
                        .any(|(_, refusal)| *refusal == ManifestRefusal::LinkBudgetExhausted)
                    {
                        scan.refusals
                            .push((rel_path.clone(), ManifestRefusal::LinkBudgetExhausted));
                    }
                    break;
                }
                let recorded = write_export_link(
                    conn,
                    repo_id,
                    source_state,
                    &source,
                    rel_path,
                    package,
                    specifier,
                    link,
                    ambiguity,
                )?;
                if recorded.inserted {
                    *budget += 1;
                }
                scan.links.push(recorded);
            }
        }
    }
    Ok(())
}

/// Record one C2 link, or re-observe the one already stored for this logical identity.
#[allow(clippy::too_many_arguments)]
fn write_export_link(
    conn: &Connection,
    repo_id: &str,
    source_state: &str,
    source: &EntityRef,
    source_path: &str,
    package: &str,
    specifier: &str,
    link: &PendingExportLink,
    ambiguity: Option<Ambiguity>,
) -> Result<RecordedLink> {
    let span = entity_span(source);
    let evidence = serde_json::json!({
        "rule": ContractRule::NpmExportResolution.as_str(),
        "specifier": specifier,
        "package": package,
        "form": link.form.as_str(),
        "exports_key": link.matched,
        "condition": link.condition,
        "declared_by": {
            "manifest": link.declared_manifest,
            "section": link.declared_section,
        },
        "target_path": link.target_rel,
        "target_indexed": link.target_entity.is_some(),
    })
    .to_string();

    let existing = nerve_store::contract_link_id(
        conn,
        repo_id,
        &nerve_store::ContractLinkIdentity {
            registry_entry_id: &link.registry_id,
            contract_kind: ContractRule::NpmExportResolution.as_str(),
            contract_identity: specifier,
            source_path,
            source_span: &span,
            resolution_method: ContractResolutionMethod::ExportMapResolved,
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
                // The source half is local and foreign-keyed. The target half below is a
                // snapshot and none of it is, which is the whole difference between this table
                // and `assertion` (`schema.rs:97`).
                source_entity_id: Some(source.entity_id.clone()),
                source_kind_snapshot: Some(source.kind.clone()),
                source_path: source_path.to_string(),
                source_span: span.clone(),
                registry_entry_id: link.registry_id.clone(),
                expected_target_repository_id: link.expected_repository_id.clone(),
                target_state_at_resolution: link.target_state.clone(),
                target_entity_id: link
                    .target_entity
                    .as_ref()
                    .map(|entity| entity.entity_id.clone()),
                target_kind_snapshot: link
                    .target_entity
                    .as_ref()
                    .map(|entity| entity.kind.clone()),
                target_name_snapshot: link
                    .target_entity
                    .as_ref()
                    .map(|entity| entity.name.clone()),
                // Set even when the entity is absent: the path is what makes "the neighbour has
                // this file and never indexed it" distinguishable from "the neighbour does not
                // have it", and the second is an unresolved declaration rather than a link.
                target_path_snapshot: Some(link.target_rel.clone()),
                target_span_snapshot: link.target_entity.as_ref().map(entity_span),
                relation_semantics: EXPORT_REFERENCE_SEMANTICS.to_string(),
                contract_kind: ContractRule::NpmExportResolution.as_str().to_string(),
                contract_identity: specifier.to_string(),
                expected_contract_version: None,
                observed_contract_version: link.observed_version.clone(),
                resolution_method: ContractResolutionMethod::ExportMapResolved,
                extractor_id: NPM_EXPORT_EXTRACTOR_ID.to_string(),
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
        rule: ContractRule::NpmExportResolution,
        manifest: source_path.to_string(),
        section: package.to_string(),
        identity: specifier.to_string(),
        form: link.form,
        registry_id: link.registry_id.clone(),
        target_repository_id: link.target_repository_id.clone(),
        resolution_method: ContractResolutionMethod::ExportMapResolved,
        source_span: span,
        relation_semantics: EXPORT_REFERENCE_SEMANTICS,
        source_entity_id: Some(source.entity_id.clone()),
        target_entity_id: link
            .target_entity
            .as_ref()
            .map(|entity| entity.entity_id.clone()),
        target_path: Some(link.target_rel.clone()),
        expected_contract_version: None,
        observed_contract_version: link.observed_version.clone(),
        ambiguity,
        inserted,
    })
}

/// `start:end` for an entity, which is where the link is quoted from.
fn entity_span(entity: &EntityRef) -> String {
    format!(
        "{}:{}",
        entity.start_line.unwrap_or(1),
        entity.end_line.unwrap_or(1)
    )
}

/// Is this the language tag of a file the `ts-js-*` extractors read?
///
/// Derived from [`Language`] rather than spelled as three string literals, so a language added to
/// the TS/JS family is answered here the day it exists.
fn is_ts_js(language: &str) -> bool {
    [Language::TypeScript, Language::Tsx, Language::JavaScript]
        .into_iter()
        .any(|candidate| candidate.as_str() == language)
}

/// The entity at a repository-relative path in **this** repository.
fn local_entity(conn: &Connection, rel_path: &str) -> Result<Option<EntityRef>> {
    match nerve_store::resolve_selector(conn, rel_path)? {
        nerve_store::Selection::Resolved { entity, .. } => Ok(Some(*entity)),
        _ => Ok(None),
    }
}

/// The entity at a repository-relative path in the **neighbour's** index, read-only.
///
/// Every outcome that is not exactly one entity is `None`, and `None` is not an error. It is the
/// evidence for `target_partially_indexed`: the resolved path was not something the neighbour's
/// index can name with one row, so nothing about that file is claimed. Reporting a *guess* here —
/// the first of an ambiguous pair, say — would put a snapshot on the link that the neighbour never
/// agreed to, and the whole point of the snapshot is that it is a copy of something real.
fn foreign_entity(index: &Connection, rel_path: &str) -> Option<EntityRef> {
    match nerve_store::resolve_selector(index, rel_path).ok()? {
        nerve_store::Selection::Resolved { entity, .. } => Some(*entity),
        _ => None,
    }
}

/// Why a resolved export path is not a file inside the neighbour.
enum ExportPathRefusal {
    /// It leaves the neighbour's root, lexically or through a symlink.
    Escapes,
    /// It names nothing in the neighbour's tree.
    Missing,
}

/// Turn a path an `exports` entry named into a neighbour-relative path, or refuse by name.
///
/// Two checks, in this order and for two different reasons. The **lexical** one runs first and
/// touches no filesystem, so `"../../elsewhere.ts"` is refused as an escape whether or not anything
/// is there — a path that leaves the root is an escape even when the escape fails. The
/// **canonical** one then runs [`canonical_child`], which is the same choke point discovery uses, so
/// a symlink pointing out of the neighbour is refused rather than followed.
///
/// `canonical_child` cannot distinguish an absent path from an escaping one, so the two are
/// separated afterwards exactly as [`crate::registry::probe_target`] separates them: something at
/// the path means a symlink escape, nothing at it means the target is missing.
fn neighbour_relative(
    root: &Path,
    declared: &str,
) -> std::result::Result<String, ExportPathRefusal> {
    if !lexically_inside(declared) {
        return Err(ExportPathRefusal::Escapes);
    }
    let resolved = match canonical_child(root, Path::new(declared)) {
        Ok(resolved) => resolved,
        Err(_) => {
            return Err(match root.join(declared).symlink_metadata() {
                Ok(_) => ExportPathRefusal::Escapes,
                Err(_) => ExportPathRefusal::Missing,
            })
        }
    };
    resolved
        .strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().into_owned())
        .ok_or(ExportPathRefusal::Escapes)
}

/// Does this relative path stay inside its root, reading the text alone?
fn lexically_inside(declared: &str) -> bool {
    if declared.is_empty() || declared.starts_with('/') || declared.contains('\\') {
        return false;
    }
    let mut depth: i32 = 0;
    for segment in declared.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

/// Split a bare npm specifier into its package part and its subpath.
///
/// `None` means *this is not a bare package specifier* and therefore not a C2 declaration at all: a
/// relative or absolute path names a file in this repository, `#internal` is npm's own subpath
/// import, and `node:fs` names a scheme. None of the three could ever be a cross-repository
/// contract, so none is counted as a declaration Nerve read and declined.
fn split_specifier(specifier: &str) -> Option<(&str, String)> {
    let specifier = specifier.trim();
    if specifier.is_empty()
        || specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.starts_with('#')
        || has_scheme(specifier)
    {
        return None;
    }
    let mut segments = specifier.split('/');
    let first = segments.next()?;
    let package_len = if let Some(scope) = first.strip_prefix('@') {
        if scope.is_empty() {
            return None;
        }
        let second = segments.next()?;
        if second.is_empty() {
            return None;
        }
        first.len() + 1 + second.len()
    } else {
        first.len()
    };
    if package_len == 0 {
        return None;
    }
    let (package, rest) = specifier.split_at(package_len);
    let subpath = match rest.strip_prefix('/') {
        None | Some("") => ".".to_string(),
        Some(tail) => format!("./{tail}"),
    };
    Some((package, subpath))
}

/// How many `/`-separated segments a subpath has after the package name. `.` is zero.
fn subpath_depth(subpath: &str) -> usize {
    match subpath.strip_prefix("./") {
        Some(tail) if !tail.is_empty() => tail.split('/').count(),
        _ => 0,
    }
}

/// Resolve one subpath against one neighbour's manifest. The whole of C2's supported syntax is this
/// function and the three below it.
fn resolve_export(manifest: &serde_json::Value, subpath: &str) -> ExportOutcome {
    if subpath_depth(subpath) > MAX_EXPORT_SUBPATH_DEPTH {
        return ExportOutcome::Declined(UnsupportedForm::NpmExportSubpathTooDeep);
    }
    match manifest.get("exports") {
        // No `exports` at all is the legacy shape, and it is the only case where `main`, `module`
        // and `types` are read. A package that declares `exports` has stated what it exports.
        None => legacy_entry(manifest, subpath),
        Some(serde_json::Value::Null) => ExportOutcome::Declined(UnsupportedForm::NpmExportBlocked),
        Some(serde_json::Value::String(target)) => {
            if subpath == "." {
                ExportOutcome::Resolved {
                    form: SupportedForm::NpmExportsString,
                    target: target.clone(),
                    matched: "exports".to_string(),
                    condition: None,
                }
            } else {
                ExportOutcome::Declined(UnsupportedForm::NpmExportSubpathNotDeclared)
            }
        }
        Some(serde_json::Value::Object(map)) => resolve_export_object(map, subpath),
        Some(_) => ExportOutcome::Declined(UnsupportedForm::NpmExportNonStringTarget),
    }
}

/// An `exports` object: either a subpath map, or a bare conditions object for `.`.
fn resolve_export_object(
    map: &serde_json::Map<String, serde_json::Value>,
    subpath: &str,
) -> ExportOutcome {
    if map.len() > MAX_EXPORT_ENTRIES {
        return ExportOutcome::Declined(UnsupportedForm::NpmExportMapTooLarge);
    }
    let subpath_keys = map.keys().filter(|key| key.starts_with('.')).count();

    if subpath_keys == 0 {
        // Every key is a condition, so the object states the `.` subpath and nothing else.
        if subpath != "." {
            return ExportOutcome::Declined(UnsupportedForm::NpmExportSubpathNotDeclared);
        }
        return match resolve_conditions(map, 1) {
            ConditionOutcome::Resolved { target, condition } => ExportOutcome::Resolved {
                form: SupportedForm::NpmExportsConditional,
                target,
                matched: ".".to_string(),
                condition: Some(condition),
            },
            ConditionOutcome::Declined(form) => ExportOutcome::Declined(form),
        };
    }
    if subpath_keys != map.len() {
        // Node rejects this outright, and guessing which half was meant would be inventing the
        // syntax rather than reading it.
        return ExportOutcome::Declined(UnsupportedForm::NpmExportsMixedKeys);
    }
    if let Some(value) = map.get(subpath) {
        return resolve_export_entry(value, subpath);
    }
    // An exact key is preferred over a pattern, which is Node's own rule, so a wildcard only
    // decides anything when nothing exact matched.
    if map.keys().any(|key| wildcard_matches(key, subpath)) {
        return ExportOutcome::Declined(UnsupportedForm::NpmExportWildcardSubpath);
    }
    ExportOutcome::Declined(UnsupportedForm::NpmExportSubpathNotDeclared)
}

/// One subpath map entry: a string, a conditions object, a deliberate `null`, or nothing Nerve
/// reads.
fn resolve_export_entry(value: &serde_json::Value, matched: &str) -> ExportOutcome {
    match value {
        serde_json::Value::Null => ExportOutcome::Declined(UnsupportedForm::NpmExportBlocked),
        serde_json::Value::String(target) => ExportOutcome::Resolved {
            form: SupportedForm::NpmExportsSubpath,
            target: target.clone(),
            matched: matched.to_string(),
            condition: None,
        },
        serde_json::Value::Object(map) => match resolve_conditions(map, 1) {
            ConditionOutcome::Resolved { target, condition } => ExportOutcome::Resolved {
                form: SupportedForm::NpmExportsConditional,
                target,
                matched: matched.to_string(),
                condition: Some(condition),
            },
            ConditionOutcome::Declined(form) => ExportOutcome::Declined(form),
        },
        _ => ExportOutcome::Declined(UnsupportedForm::NpmExportNonStringTarget),
    }
}

/// Descend a conditions object in [`EXPORT_CONDITION_ORDER`].
///
/// The **first** condition in that order that the object names decides the answer, including when
/// what it names is `null` or a shape Nerve declines. Falling through a `null` to a later condition
/// would step over a block the package wrote deliberately.
fn resolve_conditions(
    map: &serde_json::Map<String, serde_json::Value>,
    depth: usize,
) -> ConditionOutcome {
    if depth > MAX_EXPORT_CONDITION_DEPTH {
        return ConditionOutcome::Declined(UnsupportedForm::NpmExportUnsupportedCondition);
    }
    if map.len() > MAX_EXPORT_ENTRIES {
        return ConditionOutcome::Declined(UnsupportedForm::NpmExportMapTooLarge);
    }
    for condition in EXPORT_CONDITION_ORDER {
        let Some(value) = map.get(condition) else {
            continue;
        };
        return match value {
            serde_json::Value::Null => {
                ConditionOutcome::Declined(UnsupportedForm::NpmExportBlocked)
            }
            serde_json::Value::String(target) => ConditionOutcome::Resolved {
                target: target.clone(),
                condition: condition.to_string(),
            },
            serde_json::Value::Object(nested) => resolve_conditions(nested, depth + 1),
            _ => ConditionOutcome::Declined(UnsupportedForm::NpmExportNonStringTarget),
        };
    }
    ConditionOutcome::Declined(UnsupportedForm::NpmExportUnsupportedCondition)
}

/// `module`, then `main`, then `types` — read only when the package declares no `exports`.
fn legacy_entry(manifest: &serde_json::Value, subpath: &str) -> ExportOutcome {
    if subpath != "." {
        // Node would probe the filesystem and guess an extension here. That is a resolution rule
        // Nerve would be inventing, so the specifier is declined with the form named.
        return ExportOutcome::Declined(UnsupportedForm::NpmLegacySubpathProbe);
    }
    for field in LEGACY_ENTRY_FIELDS {
        let Some(target) = manifest.get(field).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let form = match field {
            "module" => SupportedForm::NpmLegacyModule,
            "main" => SupportedForm::NpmLegacyMain,
            _ => SupportedForm::NpmLegacyTypes,
        };
        return ExportOutcome::Resolved {
            form,
            target: target.to_string(),
            matched: field.to_string(),
            condition: None,
        };
    }
    ExportOutcome::Unresolved(UnresolvedReason::TargetDeclaresNoEntryPoint)
}

/// Would this `exports` key match this subpath **as a pattern**?
///
/// Used only to name the refusal. Nerve never expands the pattern: knowing that `"./*"` would have
/// matched is what turns a silent absence into [`UnsupportedForm::NpmExportWildcardSubpath`].
fn wildcard_matches(key: &str, subpath: &str) -> bool {
    let Some((prefix, suffix)) = key.split_once('*') else {
        return false;
    };
    if suffix.contains('*') {
        return false;
    }
    subpath.len() >= prefix.len() + suffix.len()
        && subpath.starts_with(prefix)
        && subpath.ends_with(suffix)
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
        // Unreachable by construction: the manifest loop only ever sees a rule that
        // [`ContractRule::for_file_name`] returned, and that function reads
        // [`ContractRule::MANIFEST_DRIVEN`], which C2 is deliberately not a member of.
        ContractRule::NpmExportResolution => unreachable!(
            "C2 is not manifest-driven; it is reached from the imports of an indexed module"
        ),
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
    // A resolved path with no entity behind it: the neighbour has the file and its index has never
    // looked at it. Read off the row alone, because that is where the evidence is — C1 and C3 links
    // carry no target path at all, so this can only fire for C2. Reported **before** the state
    // comparison, because "part of the target was never indexed" outranks "the target moved on":
    // reporting an unknown as a clean bill is Slice 7c-i's failure, and reporting it as a change is
    // 12b's.
    if link.target_path_snapshot.is_some() && link.target_entity_id.is_none() {
        return Some(ContractFreshness::TargetPartiallyIndexed);
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
        // `export_map_resolved` belongs to C2 and to nothing else. 13b left it with no producer;
        // 13c gives it exactly one rule's worth, and the partition is asserted in both directions
        // so that a C1 or C3 form drifting onto it fails here rather than in a precision table.
        for form in SupportedForm::ALL {
            assert_eq!(
                form.resolution_method() == ExportMapResolved,
                form.rule() == ContractRule::NpmExportResolution,
                "{form} disagrees with its own rule about export_map_resolved"
            );
        }
        assert!(SupportedForm::ALL
            .iter()
            .any(|form| form.resolution_method() == ExportMapResolved));
    }

    // ---- C2 ------------------------------------------------------------------------------------

    fn export_of(manifest: &str, subpath: &str) -> ExportOutcome {
        let value: serde_json::Value = serde_json::from_str(manifest).unwrap();
        resolve_export(&value, subpath)
    }

    fn resolved(manifest: &str, subpath: &str) -> (SupportedForm, String, Option<String>) {
        match export_of(manifest, subpath) {
            ExportOutcome::Resolved {
                form,
                target,
                condition,
                ..
            } => (form, target, condition),
            ExportOutcome::Declined(form) => panic!("{subpath} was declined: {form}"),
            ExportOutcome::Unresolved(reason) => panic!("{subpath} was unresolved: {reason}"),
        }
    }

    fn export_declined(manifest: &str, subpath: &str) -> UnsupportedForm {
        match export_of(manifest, subpath) {
            ExportOutcome::Declined(form) => form,
            ExportOutcome::Resolved { target, .. } => panic!("{subpath} resolved to {target}"),
            ExportOutcome::Unresolved(reason) => panic!("{subpath} was unresolved: {reason}"),
        }
    }

    /// The whole of C2's supported syntax, one assertion per form.
    #[test]
    fn the_export_syntax_is_exactly_the_six_forms() {
        assert_eq!(
            resolved(r#"{"exports": "./src/index.ts"}"#, "."),
            (
                SupportedForm::NpmExportsString,
                "./src/index.ts".to_string(),
                None
            )
        );
        assert_eq!(
            resolved(
                r#"{"exports": {".": "./src/index.ts", "./sub": "./src/sub.ts"}}"#,
                "./sub"
            ),
            (
                SupportedForm::NpmExportsSubpath,
                "./src/sub.ts".to_string(),
                None
            )
        );
        // The documented order: `import`, then `require`, then `default`, at every level.
        assert_eq!(
            resolved(
                r#"{"exports": {".": {"require": "./cjs.js", "import": "./esm.js", "default": "./d.js"}}}"#,
                "."
            ),
            (
                SupportedForm::NpmExportsConditional,
                "./esm.js".to_string(),
                Some("import".to_string())
            )
        );
        assert_eq!(
            resolved(
                r#"{"exports": {".": {"require": "./cjs.js", "default": "./d.js"}}}"#,
                "."
            ),
            (
                SupportedForm::NpmExportsConditional,
                "./cjs.js".to_string(),
                Some("require".to_string())
            )
        );
        assert_eq!(
            resolved(r#"{"exports": {".": {"default": "./d.js"}}}"#, "."),
            (
                SupportedForm::NpmExportsConditional,
                "./d.js".to_string(),
                Some("default".to_string())
            )
        );
        // A bare conditions object states `.` and nothing else.
        assert_eq!(
            resolved(r#"{"exports": {"import": "./esm.js"}}"#, "."),
            (
                SupportedForm::NpmExportsConditional,
                "./esm.js".to_string(),
                Some("import".to_string())
            )
        );
        assert_eq!(
            resolved(
                r#"{"exports": {".": {"import": {"default": "./deep.js"}}}}"#,
                "."
            ),
            (
                SupportedForm::NpmExportsConditional,
                "./deep.js".to_string(),
                Some("default".to_string())
            )
        );
        // Legacy, in the stated order, and only when there is no `exports` at all.
        assert_eq!(
            resolved(
                r#"{"module": "./m.ts", "main": "./x.js", "types": "./t.d.ts"}"#,
                "."
            ),
            (SupportedForm::NpmLegacyModule, "./m.ts".to_string(), None)
        );
        assert_eq!(
            resolved(r#"{"main": "src/index.ts", "types": "./t.d.ts"}"#, "."),
            (
                SupportedForm::NpmLegacyMain,
                "src/index.ts".to_string(),
                None
            )
        );
        assert_eq!(
            resolved(r#"{"types": "./t.d.ts"}"#, "."),
            (SupportedForm::NpmLegacyTypes, "./t.d.ts".to_string(), None)
        );
    }

    /// Everything else is declined **with the form named**, and the names are all different.
    #[test]
    fn every_other_export_shape_is_declined_with_its_form_named() {
        assert_eq!(
            export_declined(r#"{"exports": {"./*": "./src/*.ts"}}"#, "./deep"),
            UnsupportedForm::NpmExportWildcardSubpath
        );
        // An exact key still wins over a pattern that would also have matched.
        assert_eq!(
            resolved(
                r#"{"exports": {"./*": "./src/*.ts", "./sub": "./src/sub.ts"}}"#,
                "./sub"
            )
            .1,
            "./src/sub.ts"
        );
        assert_eq!(
            export_declined(r#"{"exports": {"./blocked": null}}"#, "./blocked"),
            UnsupportedForm::NpmExportBlocked
        );
        assert_eq!(
            export_declined(r#"{"exports": null}"#, "."),
            UnsupportedForm::NpmExportBlocked
        );
        // A `null` under the first condition in the order is a block, not a fall-through.
        assert_eq!(
            export_declined(
                r#"{"exports": {".": {"import": null, "default": "./d.js"}}}"#,
                "."
            ),
            UnsupportedForm::NpmExportBlocked
        );
        assert_eq!(
            export_declined(r#"{"exports": {".": {"browser": "./b.js"}}}"#, "."),
            UnsupportedForm::NpmExportUnsupportedCondition
        );
        // [`MAX_EXPORT_CONDITION_DEPTH`] nested objects resolve; one more is declined, so the bound
        // is asserted from both sides rather than only from the failing one.
        let nest = |levels: usize| {
            let mut text = "\"./x.js\"".to_string();
            for _ in 0..levels {
                text = format!("{{\"import\": {text}}}");
            }
            format!("{{\"exports\": {{\".\": {text}}}}}")
        };
        assert_eq!(resolved(&nest(MAX_EXPORT_CONDITION_DEPTH), ".").1, "./x.js");
        assert_eq!(
            export_declined(&nest(MAX_EXPORT_CONDITION_DEPTH + 1), "."),
            UnsupportedForm::NpmExportUnsupportedCondition
        );
        assert_eq!(
            export_declined(r#"{"exports": {".": ["./a.js", "./b.js"]}}"#, "."),
            UnsupportedForm::NpmExportNonStringTarget
        );
        assert_eq!(
            export_declined(r#"{"exports": 7}"#, "."),
            UnsupportedForm::NpmExportNonStringTarget
        );
        assert_eq!(
            export_declined(r#"{"exports": {".": "./a.ts", "import": "./b.ts"}}"#, "."),
            UnsupportedForm::NpmExportsMixedKeys
        );
        assert_eq!(
            export_declined(r#"{"exports": {".": "./src/index.ts"}}"#, "./sub"),
            UnsupportedForm::NpmExportSubpathNotDeclared
        );
        assert_eq!(
            export_declined(r#"{"exports": "./src/index.ts"}"#, "./sub"),
            UnsupportedForm::NpmExportSubpathNotDeclared
        );
        assert_eq!(
            export_declined(r#"{"main": "./src/index.ts"}"#, "./sub"),
            UnsupportedForm::NpmLegacySubpathProbe
        );
        // A package that declares nothing at all is unresolved rather than declined: there is no
        // form to name, only an absence.
        assert!(matches!(
            export_of(r#"{"name": "pkg"}"#, "."),
            ExportOutcome::Unresolved(UnresolvedReason::TargetDeclaresNoEntryPoint)
        ));
    }

    /// Both C2 bounds stop a resolution and name themselves.
    #[test]
    fn the_export_bounds_are_reachable_and_named() {
        let deep = format!("./{}", ["x"; MAX_EXPORT_SUBPATH_DEPTH + 1].join("/"));
        assert_eq!(
            export_declined(r#"{"exports": {".": "./src/index.ts"}}"#, &deep),
            UnsupportedForm::NpmExportSubpathTooDeep
        );
        let shallow = format!("./{}", ["x"; MAX_EXPORT_SUBPATH_DEPTH].join("/"));
        assert_eq!(
            export_declined(r#"{"exports": {".": "./src/index.ts"}}"#, &shallow),
            UnsupportedForm::NpmExportSubpathNotDeclared,
            "the bound must be off by nothing: exactly MAX_EXPORT_SUBPATH_DEPTH is inside it"
        );

        let entries: Vec<String> = (0..=MAX_EXPORT_ENTRIES)
            .map(|index| format!("\"./s{index}\": \"./src/s{index}.ts\""))
            .collect();
        let oversized = format!("{{\"exports\": {{{}}}}}", entries.join(","));
        assert_eq!(
            export_declined(&oversized, "./s0"),
            UnsupportedForm::NpmExportMapTooLarge
        );
    }

    /// A specifier is split into the two things npm resolves it by, and nothing else is a package.
    #[test]
    fn a_bare_specifier_splits_into_a_package_and_a_subpath() {
        let split = |specifier: &str| {
            split_specifier(specifier).map(|(package, subpath)| (package.to_string(), subpath))
        };
        assert_eq!(split("pkg"), Some(("pkg".into(), ".".into())));
        assert_eq!(split("pkg/"), Some(("pkg".into(), ".".into())));
        assert_eq!(split("pkg/sub"), Some(("pkg".into(), "./sub".into())));
        assert_eq!(split("pkg/a/b"), Some(("pkg".into(), "./a/b".into())));
        assert_eq!(split("@scope/pkg"), Some(("@scope/pkg".into(), ".".into())));
        assert_eq!(
            split("@scope/pkg/sub"),
            Some(("@scope/pkg".into(), "./sub".into()))
        );
        // Not packages, and therefore not declarations Nerve read and declined.
        for specifier in [
            "./local",
            "../up",
            "/abs",
            "#internal",
            "node:fs",
            "@scope",
            "",
        ] {
            assert_eq!(split(specifier), None, "{specifier} was read as a package");
        }
    }

    /// A resolved export path may not leave the neighbour, and the two ways it fails are different
    /// facts.
    #[test]
    fn an_export_path_that_leaves_the_target_is_refused_before_the_filesystem_is_touched() {
        for declared in ["../outside.ts", "../../outside.ts", "/etc/passwd", "a\\b"] {
            assert!(
                !lexically_inside(declared),
                "{declared} was accepted as inside the target"
            );
        }
        for declared in [
            "./src/index.ts",
            "src/index.ts",
            "./a/../b.ts",
            "./a/b/c.ts",
        ] {
            assert!(
                lexically_inside(declared),
                "{declared} was refused as an escape"
            );
        }
    }

    /// The pattern matcher exists only to name a refusal, so it must not claim matches it has not.
    #[test]
    fn a_wildcard_key_matches_only_what_it_covers() {
        assert!(wildcard_matches("./*", "./deep"));
        assert!(wildcard_matches("./features/*", "./features/a"));
        assert!(wildcard_matches("./*.js", "./a.js"));
        assert!(!wildcard_matches("./features/*", "./other/a"));
        assert!(!wildcard_matches("./sub", "./sub"));
        assert!(!wildcard_matches("./*/*", "./a/b"));
    }
}
