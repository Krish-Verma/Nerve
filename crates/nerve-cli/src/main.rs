//! `nerve` — a local, offline code evidence graph.
//!
//! This binary is a thin adapter (ARCHITECTURE.md): it parses arguments, calls the
//! application layer, renders, and maps outcomes to exit codes. It contains no graph logic.

#![forbid(unsafe_code)]

mod doctor;
mod exit;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use nerve_core::vocab::{MemoryScope, MemoryStatus};
use nerve_core::Relation;
use nerve_index::config;
use nerve_index::error::IndexError;
// The one history *judgment* — whether an earlier change may exist and not be recorded — is derived
// in `nerve-store` beside `IngestRow` and never here. See the note above `print_refusals`.
use nerve_store::earlier_changes_may_exist;

/// Command-line surface.
#[derive(Debug, Parser)]
#[command(
    name = "nerve",
    version,
    about = "Local, offline code evidence graph",
    long_about = "Nerve builds a local evidence graph of a repository. It makes no network \
                  calls, sends no telemetry, and stores no source text — only ranges and \
                  content hashes."
)]
struct Cli {
    /// Emit a single JSON object instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Suppress human-readable output. Has no effect on --json.
    #[arg(long, global = true)]
    quiet: bool,

    /// Accepted for forward compatibility. Nerve emits no ANSI colour in this release.
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create .nerve/ with a config, database and migrations. Idempotent.
    Init {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
    },
    /// Parse the repository and persist the evidence graph.
    ///
    /// Only files that changed, and the files that import them transitively, are re-extracted.
    Index {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Re-extract every file, ignoring the change-detection cache.
        #[arg(long)]
        full: bool,
    },
    /// Ingest one LCOV coverage report into an existing index.
    ///
    /// Reads only the report you name. Nerve runs no tests, spawns no process and looks for no
    /// report of its own — a coverage report is something another tool produced, and an index
    /// that changed meaning because a test run left a file behind would be an index nobody could
    /// reason about.
    ///
    /// This is a separate command rather than a flag on `nerve index` on purpose: were it a flag,
    /// the ordinary post-edit `nerve index` — run without it, as it always is — would silently
    /// destroy every coverage edge in the repository.
    ///
    /// Editing a covered file does not delete its coverage. It makes it *stale*, which
    /// `nerve why` reports, and which is strictly more informative than silence.
    Coverage {
        /// Path to the LCOV report. Must be inside the repository.
        report: PathBuf,
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
    },
    /// Read a test-call-trace artifact your own tracer produced.
    ///
    /// **Nerve does not run your tests.** There is no `nerve trace-tests`, and its absence is a
    /// design position rather than a gap: `crates/nerve-cli/tests/no_subprocess.rs` forbids process
    /// creation in Nerve's product code, and running a test suite would need an exception to it. You
    /// run your suite under the tracer, in your own environment, with your own secrets; Nerve reads
    /// the artifact afterwards and spawns nothing.
    ///
    /// A trace is **existential evidence**: it says one run took an edge, not that every run does.
    /// `TEST_OBSERVED_CALL` is therefore not in `nerve impact`'s default relation set, and is asked
    /// for explicitly with `--relation TEST_OBSERVED_CALL`.
    #[command(subcommand)]
    Trace(TraceCommand),
    /// Read the repository's own commit history, and ask what it recorded.
    ///
    /// `sync` walks `.git` and records what the commit graph says; `log` and `file` read it back.
    /// Nothing here spawns `git`, and nothing here reaches the network: the object store is parsed
    /// directly, exactly as `nerve index` parses source.
    ///
    /// **An absent history is never described as an empty one.** A repository whose history has
    /// never been synced, one that was synced and holds no commits, one whose earliest visible
    /// commit is a shallow boundary, and one where Nerve stopped at its own commit budget are four
    /// different answers, and each says which it is. That distinction is the reason this surface
    /// exists — see `docs/plans/slice-12b-historical-model.md` §5.
    ///
    /// Times and timezones are always recorded. Author and committer **identities are not**, unless
    /// `--with-identity` asks for them: no question this surface answers asks *who*, so contributor
    /// names and email addresses would be third-party personal data in the index with no query
    /// behind them.
    #[command(subcommand)]
    History(HistoryCommand),
    /// Record which neighbouring repositories this one is allowed to be told about.
    ///
    /// **Nothing here is discovered.** A sibling checkout is a filesystem accident, not a
    /// declaration, so a repository becomes a neighbour only because `nerve repo add` named it —
    /// which is the "directory proximity" link `docs/plans/slice-13-cross-repository-contracts.md`
    /// §1 refuses, one layer down.
    ///
    /// This is the first command that reads a directory the user did not point Nerve at, and it is
    /// therefore a new trust boundary (`docs/THREAT-MODEL.md` T12). The neighbour's
    /// `.nerve/nerve.db` is opened **read-only** and nothing else over there is opened at all; its
    /// bytes are identical afterwards; the target is never indexed as a side effect; and the stored
    /// path is re-validated on every use, because a row naming a directory is untrusted input the
    /// moment it is written.
    ///
    /// `remove` **retires** an entry and never deletes it. There is no purge verb: a row that
    /// vanished from the table could not be reported as having ended, and a retired name stays
    /// taken.
    #[command(subcommand)]
    Repo(RepoCommand),
    /// Write down what a human knows about this repository, and read it back with its
    /// qualifications.
    ///
    /// **This is the human's surface, and it is the only one that writes.** Nerve has no accounts,
    /// no network and no identity provider, so nothing here can tell a human apart from an agent
    /// holding the same shell — and a control that cannot be enforced must not be claimed. What is
    /// enforceable is the *surface boundary*: the HTTP API and the MCP tools read memory and cannot
    /// write it, and their inability is `PRAGMA query_only`, not a check they could be talked out
    /// of. A human who hands an agent their shell has removed the boundary, and Nerve cannot
    /// detect that (`docs/THREAT-MODEL.md` T13).
    ///
    /// **Nothing here deletes.** `invalidate` records that a note stopped being true and
    /// `supersede` records that a later note replaced it — two different facts, kept apart — and
    /// both keep every earlier event readable. There is no `nerve memory delete`, because a delete
    /// verb is how *"history preserved"* stops being true; its absence is asserted by a test rather
    /// than left to discipline.
    ///
    /// A record's subject is a **snapshot**, never a live pointer: `entity` rows are pruned on
    /// every re-index, so a note about a file survives that file's deletion and reports its subject
    /// as `missing` rather than disappearing with it. Staleness is derived at read time against the
    /// repository state the record was anchored to, and is never stored as a score.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Report index counts, freshness and schema version.
    Status {
        /// Repository root. Defaults to the current directory.
        ///
        /// Accepted positionally, matching `init` and `index`, whose subject is also the
        /// repository. `--path` remains accepted so the query commands — whose positional
        /// arguments are the query, not the repository — keep one spelling across the surface.
        path: Option<PathBuf>,
        /// Repository root. Equivalent to the positional form.
        #[arg(long = "path", value_name = "PATH")]
        path_flag: Option<PathBuf>,
    },
    /// Judge whether this index can be trusted right now, and say so with an exit code.
    ///
    /// `status` reports; `check` **judges**, and its judgement is a process exit code another
    /// program branches on. It answers one question — *can I trust this index?* — for a CI job
    /// about to run other `nerve` commands, and for a pre-commit hook. The output is secondary.
    ///
    /// `0` the index is current · `2` there is no index · `3` the schema is behind or a run never
    /// finished · `4` the index is sound but describes a tree that has moved on · `10` bad
    /// arguments.
    ///
    /// It never repairs, re-indexes or migrates. A command that silently fixed the thing it was
    /// asked to judge could not be trusted to judge it, so the connection it reads through is
    /// opened `query_only`.
    ///
    /// It applies no policy: there is no `--max-unresolved` and no `--max-gaps`. `nerve gaps` and
    /// `nerve impact` already emit JSON a CI script can threshold for itself, and a `check` with
    /// policy flags would mean whatever its flags happened to say.
    Check {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Repository root. Equivalent to the positional form.
        #[arg(long = "path", value_name = "PATH")]
        path_flag: Option<PathBuf>,
        /// Report staleness as a warning and exit 0 anyway.
        ///
        /// For the pipeline that indexes and queries in one job and knows the tree cannot have
        /// moved between the two. The staleness is still reported, in the output and in `--json`.
        #[arg(long)]
        allow_stale: bool,
    },
    /// Diagnose the installation, the database and the configuration.
    ///
    /// `check` judges one thing — is the index current? — and answers with an exit code for CI.
    /// `doctor` inspects many things and answers in prose, for a person whose tooling is
    /// misbehaving. Each finding says what was checked, what was found, how bad it is, and what
    /// to do about it.
    ///
    /// It runs on a broken installation on purpose: no database, a corrupt database, a schema
    /// written by a newer Nerve and an unparseable config are its subject matter, not reasons to
    /// bail out. Exit `0` unless something fatal was found, `2` if it was — a warning is not a
    /// failure.
    ///
    /// It diagnoses and never repairs; there is no `--fix`. It makes no network call of any kind,
    /// and it does not judge index freshness — that is `nerve check`'s question.
    Doctor {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Repository root. Equivalent to the positional form.
        #[arg(long = "path", value_name = "PATH")]
        path_flag: Option<PathBuf>,
    },
    /// Full-text search over entity names and scope paths.
    Search {
        /// Search terms.
        query: String,
        /// Restrict to one entity kind.
        #[arg(long)]
        kind: Option<String>,
        /// Maximum results.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Report symbols no test is known to touch.
    ///
    /// Answers the question the coverage evidence exists for, and distinguishes the two things
    /// a naive implementation would render identically: a repository where coverage was
    /// ingested and this symbol has none, and a repository where **no coverage was ever
    /// ingested at all**. The second is not "your tests cover nothing", it is "Nerve has not
    /// been told anything about your tests", and it is reported as unanswerable rather than as
    /// a list of every symbol you have.
    ///
    /// Finding gaps is not a failure, so this exits 0 whatever it finds. CI-failing behaviour
    /// belongs to `nerve check`.
    Gaps {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Repository root. Equivalent to the positional form.
        #[arg(long = "path", value_name = "PATH")]
        path_flag: Option<PathBuf>,
        /// Restrict to this repository-relative file, or to anything under this directory.
        #[arg(long, value_name = "PATH")]
        under: Option<String>,
        /// Restrict to one symbol kind.
        #[arg(long)]
        kind: Option<String>,
        /// Also list partially covered symbols. They are counted either way, never as gaps.
        #[arg(long)]
        include_partial: bool,
        /// Maximum rows listed. The tallies stay exact.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Report what depends on a symbol, transitively, and what the answer cannot see.
    ///
    /// A reverse dependency closure: everything that reaches this symbol through `CALLS`,
    /// `REFERENCES`, `EXTENDS` or `IMPLEMENTS`, with the evidence for the edge that reached it.
    /// Containment is deliberately not followed — walking `CONTAINS` from a function reaches its
    /// file, its directory and the repository, so every symbol would "impact" everything.
    ///
    /// The count is never the whole answer. Nerve has no type inference, so a method call on a
    /// typed receiver is recorded as unresolved rather than guessed at, and the number of
    /// reference sites in this repository that resolved to nothing is printed with every result —
    /// including when it is zero. Any of those sites could reach this symbol and this command
    /// cannot rule them out.
    ///
    /// Finding nothing is not a failure, so this exits 0 whatever it finds.
    Impact {
        /// Subject selector: entity id, rel/path.ts, rel/path.ts#Name, or a unique name.
        selector: String,
        /// Maximum number of hops from the subject.
        #[arg(long, default_value_t = 6)]
        max_depth: usize,
        /// Follow only this relation. Repeatable. Default: CALLS, REFERENCES, EXTENDS, IMPLEMENTS.
        #[arg(long = "relation")]
        relations: Vec<String>,
        /// Maximum rows listed. The tallies stay exact.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Find how two entities are connected.
    Path {
        /// Source selector: entity id, rel/path.ts, rel/path.ts#Name, or a unique name.
        from: String,
        /// Target selector, same forms as `from`.
        to: String,
        /// Maximum number of hops.
        #[arg(long, default_value_t = 6)]
        max_depth: usize,
        /// Follow only this relation. Repeatable. Default: every relation.
        #[arg(long = "relation")]
        relations: Vec<String>,
        /// Maximum number of distinct paths.
        #[arg(long, default_value_t = 3)]
        limit: usize,
        /// `forward` follows assertions; `any` treats them as undirected.
        #[arg(long, value_enum, default_value_t = DirectionArg::Forward)]
        direction: DirectionArg,
        /// Exclude edges whose target Nerve could not resolve.
        #[arg(long)]
        resolved_only: bool,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Serve the evidence graph over a local, read-only HTTP API.
    ///
    /// Binds 127.0.0.1 only, mints a per-session token, and never writes to the index.
    Serve {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Repository root. Equivalent to the positional form.
        #[arg(long = "path", value_name = "PATH")]
        path_flag: Option<PathBuf>,
        /// TCP port on 127.0.0.1. The default asks the operating system for a free one.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Request-handling threads.
        #[arg(long, default_value_t = nerve_server::DEFAULT_WORKERS)]
        workers: usize,
    },
    /// Speak the Model Context Protocol on stdin and stdout, for an agent.
    ///
    /// Eight tools, listed by `tools/list` and pinned by `nerve_server::mcp::TOOL_NAMES`. The
    /// first was `nerve_investigate`, the MCP counterpart of `nerve why`; the rest were admitted
    /// one at a time, each having to show a materially different input/output contract rather than
    /// being an existing tool with a flag. Read-only, offline, stdio only — no socket, no port, no
    /// outbound connection.
    ///
    /// The count is deliberately not repeated as a list here. This comment said "One tool" from
    /// Slice 8a until 2026-08-11, four slices after that stopped being true, because a prose count
    /// beside a vocabulary is a second copy of it that nothing checks.
    ///
    /// Stdout is the protocol stream for the lifetime of the process, so nothing else is
    /// written to it. Responses are bounded and say when they were cut, and every value that
    /// came out of the repository is returned inside a field labelled untrusted content, so the
    /// consuming agent can apply its own policy to it (THREAT-MODEL T7 and T8).
    Mcp {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Repository root. Equivalent to the positional form.
        #[arg(long = "path", value_name = "PATH")]
        path_flag: Option<PathBuf>,
    },
    /// Show the evidence behind a relationship.
    Why {
        /// Subject selector: entity id, rel/path.ts, rel/path.ts#Name, or a unique name.
        from: String,
        /// Optional second selector. Without it, every assertion touching `from` is reported.
        to: Option<String>,
        /// Only assertions where the subject is the target.
        #[arg(long, conflicts_with = "outgoing")]
        incoming: bool,
        /// Only assertions where the subject is the source.
        #[arg(long)]
        outgoing: bool,
        /// Report only this relation. Repeatable. Default: every relation.
        #[arg(long = "relation")]
        relations: Vec<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

/// `nerve trace` subcommands.
///
/// One member, and the shape is deliberate. A bare `nerve trace <artifact>` would leave no room for
/// the verbs a trace surface plausibly grows — listing the runs a repository has ingested, or
/// withdrawing one — and, more importantly, `import` names what the command does. `nerve trace-tests`
/// is refused, not deferred: see the `Trace` variant's help.
#[derive(Debug, Subcommand)]
enum TraceCommand {
    /// Read one `nerve-trace/v1` artifact into an existing index.
    ///
    /// Reads only the artifact you name. Nerve runs no tests, spawns no process and looks for no
    /// artifact of its own — a trace is something your tracer produced, and an index that changed
    /// meaning because a test run left a file behind would be an index nobody could reason about.
    ///
    /// Editing a traced file does not delete its trace evidence. It makes it *stale*, which
    /// `nerve why` reports, and which is strictly more informative than silence.
    Import {
        /// Path to the artifact. Must be inside the repository.
        artifact: PathBuf,
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
    },
}

/// `nerve history` subcommands.
///
/// One writer and two readers, following `nerve trace`'s shape rather than a bare
/// `nerve history <path>`: the verbs are what the commands do, and a surface that grows a third
/// question later needs no rename.
#[derive(Debug, Subcommand)]
enum HistoryCommand {
    /// Walk `.git` and record the commit graph into an existing index.
    ///
    /// Requires `nerve init` and nothing more. History resolves nothing against the graph, so a
    /// repository that has never been indexed still has a history to read — the opposite of
    /// `nerve coverage`, which refuses without an index because every path in a report is resolved
    /// against what was indexed.
    ///
    /// Re-syncing is cheap and additive: a commit object is immutable, so a commit already recorded
    /// costs one lookup and no tree diff. The two columns that are *not* properties of the object —
    /// whether its parents were visible, and whether its changes could be enumerated — are
    /// re-examined every time, because `git fetch --unshallow` can turn "unavailable" into
    /// "available" and stale availability data is the one thing this surface must not keep.
    ///
    /// Exit `0` when everything the walk needed was read, `3` when anything was refused **or** the
    /// walk stopped at `--max-commits`. A bounded read is a partial read: a script that treated it
    /// as complete would read Nerve's own boundary as the repository's.
    Sync {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Commits this walk may read. Refused above the clamp, never silently lowered.
        #[arg(long, value_name = "N", default_value_t = nerve_index::MAX_HISTORY_COMMITS)]
        max_commits: usize,
        /// Also store author and committer identities.
        ///
        /// **Off by default, and that is a data-protection decision rather than a performance
        /// one.** Not one question this surface answers asks *who*, so storing contributor names
        /// and email addresses would put third-party personal data in the index that no query
        /// reads. Times and timezones are always stored, because *when* is asked repeatedly.
        ///
        /// With this flag, a name and an email are untrusted repository strings on exactly the same
        /// terms as a commit summary.
        #[arg(long)]
        with_identity: bool,
    },
    /// List recorded commits, newest committer time first.
    ///
    /// Reads only what `nerve history sync` recorded. It never opens `.git`, so it cannot invent a
    /// commit the last sync did not read — and it says how much of the history that sync saw, so a
    /// bounded or shallow ingest is not mistaken for the whole story.
    Log {
        /// Commits listed. The totals beside the list stay exact.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Commits skipped before listing.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Report every recorded commit that changed one path, and the rename hypotheses naming it.
    ///
    /// The argument is a **repository-relative path exactly as a commit's tree recorded it** — not a
    /// selector, and not resolved against the working tree. It is matched literally against
    /// `git_change.path`.
    ///
    /// That is deliberate and it is the only thing it can be. A historical path routinely does not
    /// exist on disk: it was deleted, or renamed, or it only ever existed before the shallow
    /// boundary. Nerve's path guard ends in a `canonicalize` call, so it can only validate a path
    /// that exists *now* — routed through it, this command would refuse every deleted path and
    /// report a repository with no renames rather than a broken guard
    /// (`docs/plans/slice-12b-historical-model.md` §8.4).
    ///
    /// A path no recorded commit touched is an empty answer and exit `0`. Absence is a finding.
    File {
        /// Repository-relative path as recorded in a tree, for example `src/app/main.rs`.
        #[arg(value_name = "PATH_IN_REPO")]
        tree_path: String,
        /// Commits listed. The totals beside the list stay exact.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// What changed between two recorded states, by **ancestry** and never by a time range.
    ///
    /// `history log` orders by `committer_time`, which is exactly what makes the wrong
    /// implementation the convenient one. A time range is not an ancestry range: a merge brings in
    /// commits whose committer time precedes it, and a rebase or a fabricated clock reorders them
    /// freely, so "the commits between two timestamps" answers a different question and fails
    /// silently. The walk here follows `parent_oids` from `--to` toward `--from`, pruned at
    /// `--from`'s own ancestors, which is what `from..to` means.
    ///
    /// **Four of the five outcomes are not diffs, and none of them prints as an empty one.** An
    /// endpoint Nerve never recorded, a `--from` that is not an ancestor of `--to`, an ancestry
    /// that could not be followed, and Nerve's own walk bound each name themselves and say which
    /// commit is at fault. An empty diff means one thing only: `--from` and `--to` have nothing
    /// between them.
    ///
    /// Every outcome exits `0`. None is a failure: a commit Nerve never read is a finding in the
    /// manner `nerve path` established for a route that does not exist, and a walk stopped by
    /// Nerve's own bound is a success carrying a qualification rather than an error.
    Diff {
        /// The older state, **excluded** from the range. Must be a recorded commit oid.
        #[arg(long, value_name = "OID")]
        from: String,
        /// The newer state, **included** in the range. Must be a recorded commit oid.
        #[arg(long, value_name = "OID")]
        to: String,
        /// Commits the range may carry. Truncation is reported as a fact, never inferred.
        #[arg(long, default_value_t = nerve_store::StateDiffLimits::DEFAULT.commits)]
        limit: usize,
        /// Commits the ancestry walk may visit.
        ///
        /// Three bounds rather than one, because they bound three different things and one number
        /// would leave two of them unbounded. This one is **Nerve's** boundary: a walk that stops
        /// here reports `walk_budget_exhausted` rather than an answer, since a prune set that is a
        /// floor would produce a range that is wrong rather than merely narrow.
        #[arg(long, value_name = "N", default_value_t = nerve_store::StateDiffLimits::DEFAULT.commits_walked)]
        max_walk: usize,
        /// Change rows the diff may carry.
        #[arg(long, value_name = "N", default_value_t = nerve_store::StateDiffLimits::DEFAULT.changes)]
        max_changes: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Which paths changed most often **in visible history**.
    ///
    /// It refuses to call the number a lifetime count, and both halves of that refusal are printed
    /// rather than documented. A shallow or bounded ingest makes every count a floor, so the
    /// availability block sits beside the list; and a merge enumerates no changes at all by the
    /// historical model's decision, so a merge-heavy repository undercounts against its own log and
    /// the merge count is stated for that reason.
    ///
    /// The order is count descending then path ascending. A count tie is the normal case — most
    /// paths change once — so the second key is part of the answer rather than decoration.
    Frequency {
        /// Paths listed. The total is counted separately, so truncation stays a fact.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Which paths were changed in the same commits as one path — an **observation**, not a
    /// dependency.
    ///
    /// This command refuses the inference its own numbers invite. Two paths changing together is
    /// equally consistent with coupling, with a formatting sweep, with a release-version bump, and
    /// with one commit that did two unrelated things, so the count is a raw shared-commit count
    /// rather than a normalised affinity — a normalised number would invite exactly the comparison
    /// the label forbids — and the sentence naming what it is not is carried on every answer rather
    /// than left to a footnote a consumer can drop.
    ///
    /// No relation is emitted and no assertion is written. Co-change exists only in this response.
    ///
    /// The argument is a repository-relative path exactly as a commit's tree recorded it, on the
    /// same terms as `history file`.
    Cochange {
        /// Repository-relative path as recorded in a tree, for example `src/app/main.rs`.
        #[arg(value_name = "PATH_IN_REPO")]
        tree_path: String,
        /// Pairs listed. The total is counted under the same restriction, so truncation stays a
        /// fact.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// What visible history is unavailable, and whether what was recorded is still current.
    ///
    /// Four freshness verdicts, and `unverifiable` is not a cosmetic fourth: a repository state
    /// that records no commit cannot be compared against the ingest's HEAD, and reporting *unknown*
    /// as *current* is how a truncated sweep becomes a clean bill of health. `nerve check` draws the
    /// same distinction between stale and unverified.
    ///
    /// Staleness is reported, never refused: `nerve check` is the only command that may exit on it,
    /// and every other command carries freshness beside its answer instead. A history that has never
    /// been synced is `no_history_ingested`, which is an absence rather than a failure, and it too
    /// exits `0`.
    Availability {
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

/// `nerve repo` subcommands.
///
/// Three writers, two readers and one re-pointer, following `nerve history`'s shape: the verbs are
/// what the commands do, and the repository the registry belongs to is always `--path`, because the
/// positional argument here is the *neighbour* rather than the subject.
///
/// The two readers are not one command with a flag. `list` answers *which neighbours are
/// registered*, which is a fact about this repository's registry; `links` answers *what this
/// repository declares about them and how much of that is still true*, which is a fact about two
/// repositories and is decided against the world as it is now.
#[derive(Debug, Subcommand)]
enum RepoCommand {
    /// Register a neighbouring repository, reading its identity from its own index.
    ///
    /// The target's `.nerve/nerve.db` is opened read-only to learn its `repo_id`, and that id is
    /// what every later check is made against — never the path. A path that no longer exists and a
    /// path that now holds a *different* repository are different facts with different remedies,
    /// and only the recorded id can tell them apart.
    ///
    /// Each refusal has its own name: the path does not exist, it is not a directory, it holds no
    /// `.nerve/nerve.db`, its database is newer than this build supports, it is a symlink leading
    /// out of the target root, it is this very repository, or it is already registered. None of
    /// them falls back to a narrower answer.
    Add {
        /// The neighbouring repository's root. Must already have been `nerve init`-ed.
        #[arg(value_name = "TARGET")]
        target: PathBuf,
        /// Local name for the entry. Defaults to the target directory's own name.
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// What to call it on a surface. Defaults to the target directory's own name.
        ///
        /// A directory name is repository content and is treated as untrusted on T7's terms: it is
        /// stored verbatim, never interpreted, and rendered inert.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Repository root whose registry this is. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// List every registered neighbour and what each one is right now.
    ///
    /// **Retired entries are listed**, marked as retired. That is not a courtesy: `registry_entry_removed`
    /// is a report made from the kept row, and a list that hid it would make the state unreportable
    /// at exactly the moment it becomes the answer.
    ///
    /// Availability is re-derived from the filesystem on every run rather than read out of the row,
    /// because what the row points at is free to change after it is written. Exit `0` whatever the
    /// verdicts are: `nerve check` is the only command that may exit on staleness.
    List {
        /// Repository root whose registry this is. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Read the recorded cross-repository links back, each with **its own** freshness.
    ///
    /// The read half of `repo scan`, and a different question from it. A scan reports what that run
    /// just wrote; this reports what is *stored*, re-decided against the world as it is now — the
    /// registry entry's availability, the neighbour's current state, and whether the manifest the
    /// declaration was quoted from is still here. Widening `scan` could not answer it, because a
    /// repository that has not been scanned since its neighbour moved has nothing to report from
    /// the scan it did not run.
    ///
    /// The verdict comes from the one service that decides it, which is the same call
    /// `/api/contracts` and the `nerve_contracts` MCP tool make. Nothing is re-derived here, so the
    /// four surfaces cannot answer this differently — `scripts/final_acceptance.sh` asserts the
    /// agreement rather than asserting the command exists.
    ///
    /// Withdrawn links and retired entries are listed, marked. A link that ended is exactly the one
    /// whose ending has to stay reportable.
    ///
    /// Human output carries each entry as its id, its name and its availability; `--json` carries
    /// the entry in full, and `nerve repo list` prints every entry whether a link rests on it or
    /// not. Exit `0` whatever the verdicts are: `nerve check` is the only command that may exit on
    /// staleness.
    Links {
        /// Links listed. The total is counted separately, so truncation stays a fact.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Repository root whose links these are. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Retire a registered neighbour and withdraw the links that resolved through it.
    ///
    /// A tombstone, never a delete, and the two writes happen in one transaction. The row keeps its
    /// id and its recorded repository id, which is the only reason a link that resolved through it
    /// can later be reported as `registry_entry_removed` rather than as a link pointing at nothing
    /// nameable. There is no purge verb in this release, and the retired name stays taken.
    Remove {
        /// The entry's local id, as `nerve repo list` prints it.
        #[arg(value_name = "REGISTRY_ID")]
        registry_id: String,
        /// Repository root whose registry this is. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Point an entry at a different path, **after** proving the recorded repository is there.
    ///
    /// The identity check is the whole command. Without it, relocation is not a convenience — it is
    /// precisely the silent re-pointing `target_repository_moved` exists to detect, performed by
    /// Nerve itself on request, and every link resolved through the entry afterwards would describe
    /// a repository nobody registered. A new path holding some other repository is refused with
    /// that reason named.
    Relocate {
        /// The entry's local id, as `nerve repo list` prints it.
        #[arg(value_name = "REGISTRY_ID")]
        registry_id: String,
        /// Where that repository now lives.
        #[arg(value_name = "NEW_PATH")]
        new_path: PathBuf,
        /// Repository root whose registry this is. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Read this repository's manifests and record the contracts they declare.
    ///
    /// Its own command rather than a step inside `nerve index`, for the reason `nerve coverage` is
    /// its own command: this one reads a **second repository**, and an ordinary re-index must not
    /// open a directory the user pointed at only as a dependency. Registration is the opt-in, and
    /// this is the command that acts on it.
    ///
    /// Two rules are read. `package.json` `dependencies`, `devDependencies` and `peerDependencies`
    /// with a `file:` or `workspace:` specifier; `pyproject.toml` PEP 621 direct `file://`
    /// references, Poetry `{ path = }` and uv `{ path = }` sources. Every other form — a registry
    /// range, a `git:` or `https:` specifier, an `npm:` alias — is **recorded as unsupported with
    /// its form named** and never fetched. Nerve does not run `npm`, `pip`, `poetry` or `git`.
    ///
    /// A declared path resolves to a neighbour by the repository id read out of that directory's
    /// own index, never by package name and never by directory proximity. A path that reaches no
    /// registered entry is reported with the reason named and is never auto-registered.
    Scan {
        /// Repository root to scan. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

/// `nerve memory` subcommands.
///
/// Five writers and five readers, and the split is the one thing about this surface that is
/// enforced rather than intended: the readers open the database `query_only`, so a read that
/// reached for a convenient repair would be refused by SQLite rather than by review.
///
/// The verbs are what they do. `supersede` and `invalidate` are **not** the same verb spelled two
/// ways — *"a later record replaced it"* and *"it stopped being true and nothing replaced it"* are
/// different facts, and the second is the one a returning reader most often needs, because there is
/// no successor to read instead. There is no `delete`: history is preserved, and a delete verb is
/// how that stops being true.
#[derive(Debug, Subcommand)]
enum MemoryCommand {
    /// Write down a note about one subject. It enters as `proposed`.
    ///
    /// The subject is resolved **now** and stored as a snapshot — its id, kind, name and
    /// repository-relative path, plus the selector exactly as you typed it. None of those is a
    /// foreign key, which is what lets the note outlive the file: entities are pruned on every
    /// re-index, and a note that vanished with its subject would be the one thing a memory feature
    /// must never do.
    ///
    /// The note is anchored to the repository state this index currently describes, and a
    /// repository nothing has indexed is **refused** rather than anchored to a state invented for
    /// the occasion: staleness is derived from the anchor at read time, so a note anchored to
    /// nothing could never be qualified.
    Propose {
        /// What the note is about. Any selector `nerve why` accepts.
        #[arg(long, value_name = "SELECTOR")]
        subject: String,
        /// Which facet of the subject the claim is about.
        ///
        /// One of `implementation`, `interface`, `operations`, `process`. Closed on purpose: the
        /// value is part of the grouping both derived views are decided over, so a free-form typo
        /// would silently suppress a conflict report and make `--scope opertions` answer *"there
        /// are no notes"* when it means *"there is no such scope"*.
        #[arg(long, value_name = "SCOPE")]
        scope: String,
        /// The note itself. Stored verbatim and never rewritten, including by supersession.
        #[arg(long, value_name = "TEXT")]
        content: String,
        /// What named question this note answers — `owner`, `deprecation-status`, `retry-policy`.
        ///
        /// Optional, and it is what makes a contradiction reportable at all: only records agreeing
        /// on subject, scope and claim key are ever reported as conflicting. Several notes about
        /// one subject with no claim key are `multiple_active`, which is ordinary.
        #[arg(long = "claim-key", value_name = "KEY")]
        claim_key: Option<String>,
        /// A local label recorded beside the note. **Not an identity.**
        ///
        /// Nerve has no accounts, no network and no identity provider, so this records what the
        /// caller said it was and nothing verified it. Defaults to `local`.
        #[arg(long, value_name = "LABEL")]
        author: Option<String>,
        /// The note's id. Defaults to a generated one.
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Confirm a proposal: `proposed` → `active`.
    ///
    /// The only transition into `active`, and the only one this product claims a human made. What
    /// makes it the human's act is the **surface it arrived on** — this command exists at the CLI
    /// and nowhere else — never an identity Nerve checked, because there is none to check.
    Confirm {
        /// The record's id, as `nerve memory list` prints it.
        #[arg(value_name = "MEMORY_ID")]
        memory_id: String,
        /// Anything to record in the audit history beside the change.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Replace an earlier note with a new one, retiring the earlier one in the same transaction.
    ///
    /// The predecessor keeps its content: superseding rewrites nothing and deletes nothing, so what
    /// was once believed stays readable with every event it accumulated. The successor inherits the
    /// predecessor's subject snapshot, and its scope and claim key unless you say otherwise — a
    /// replacement that answered a different question about a different subject would not be a
    /// replacement.
    ///
    /// The successor enters as `proposed`, like every other note, and the command prints the
    /// `confirm` line for it. `active` is reachable only through `confirm`, or a record would
    /// arrive already confirmed with nothing in its history saying who confirmed it.
    Supersede {
        /// The record being replaced.
        #[arg(value_name = "PREDECESSOR_ID")]
        predecessor_id: String,
        /// The replacement note.
        #[arg(long, value_name = "TEXT")]
        content: String,
        /// The successor's scope. Defaults to the predecessor's.
        #[arg(long, value_name = "SCOPE")]
        scope: Option<String>,
        /// The successor's claim key. Defaults to the predecessor's.
        #[arg(long = "claim-key", value_name = "KEY")]
        claim_key: Option<String>,
        /// A local label recorded beside the successor. **Not an identity.**
        #[arg(long, value_name = "LABEL")]
        author: Option<String>,
        /// The successor's id. Defaults to a generated one.
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Anything to record in the predecessor's audit history beside its retirement.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Record that a note stopped being true and **nothing replaced it**.
    ///
    /// Not a delete and not a supersession. The row keeps its content, keeps every event it ever
    /// had, and gains the moment it ended — so *"what did we once believe and no longer do, with no
    /// successor"* stays answerable, which is the question a returning reader actually asks.
    Invalidate {
        /// The record's id.
        #[arg(value_name = "MEMORY_ID")]
        memory_id: String,
        /// Why it stopped being true. Stored on the record itself.
        #[arg(long, value_name = "TEXT")]
        reason: Option<String>,
        /// Anything to record in the audit history beside the change.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Attach a passage to a note as its citation. Changes no status.
    ///
    /// The only operation here that is not a transition, so its event carries the status the record
    /// is already in on both sides. The cited passage is stored as a snapshot of a
    /// repository-relative path and an optional line span, for the reason the subject is one.
    Cite {
        /// The record's id.
        #[arg(value_name = "MEMORY_ID")]
        memory_id: String,
        /// The cited file, repository-relative.
        #[arg(long, value_name = "REPO_RELATIVE_PATH")]
        file: String,
        /// The cited lines, as `START:END`. Omit to cite the whole file.
        #[arg(long, value_name = "START:END")]
        span: Option<String>,
        /// Anything to record in the audit history beside the citation.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// List records, retired ones included.
    ///
    /// A retired record is listed and marked, for the reason `nerve repo list` prints a tombstone:
    /// hiding it would make the state unreportable at exactly the moment it becomes the answer.
    ///
    /// `--subject` takes a selector and resolves it against the **live** index, so a subject that
    /// has since been pruned can no longer be named that way. Those records have not gone anywhere:
    /// they are listed by this command without the filter, found by `nerve memory search`, and each
    /// one carries the snapshot of what it was about.
    List {
        /// Only this facet.
        #[arg(long, value_name = "SCOPE")]
        scope: Option<String>,
        /// Only records about this subject.
        #[arg(long, value_name = "SELECTOR")]
        subject: Option<String>,
        /// Only records in this stored lifecycle: `proposed`, `active`, `superseded`,
        /// `invalidated`.
        ///
        /// The stored lifecycle only. `potentially_stale`, `conflicted` and `multiple_active` are
        /// derived at read time and are reported beside every record rather than filtered on, so
        /// asking for one here is refused rather than silently answered with something else.
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Show one record in full: its lifecycle, its qualifications, its citations and its history.
    Show {
        /// The record's id.
        #[arg(value_name = "MEMORY_ID")]
        memory_id: String,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Find records whose content or claim key contains some text.
    ///
    /// A literal substring, case-insensitive for ASCII. `%` and `_` are characters here and not
    /// wildcards. The subject snapshot is deliberately **not** searched: a search that also matched
    /// paths would answer *"records near this file"* and *"records that say this"* with the same
    /// result, and the caller could not tell which question was answered.
    Search {
        /// The text to look for.
        #[arg(value_name = "QUERY")]
        query: String,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Print one record's audit history, oldest first.
    ///
    /// Every event ever appended, including the ones that precede a retirement. Nothing removes
    /// one, so this is the complete history by construction rather than by filtering.
    Events {
        /// The record's id.
        #[arg(value_name = "MEMORY_ID")]
        memory_id: String,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
    /// Write every record, its citations and its history out as one JSON document.
    ///
    /// Memory is the only thing in this database a human authored, so it is the only thing whose
    /// loss re-indexing cannot repair. The document is **deterministic**: the same database exports
    /// byte-identically twice, which is why it carries no timestamp of its own, no derived state
    /// and no absolute path.
    ///
    /// There is no `import` in this release, and that is a refusal rather than an omission: a
    /// half-safe import is how a human's notes get overwritten by a file.
    Export {
        /// Write to this file instead of standard output.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },
}

/// `--direction` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DirectionArg {
    /// Follow assertions from source to target.
    Forward,
    /// Treat assertions as undirected.
    Any,
}

impl DirectionArg {
    fn to_store(self) -> nerve_store::Direction {
        match self {
            DirectionArg::Forward => nerve_store::Direction::Forward,
            DirectionArg::Any => nerve_store::Direction::Any,
        }
    }
}

/// Rendering context threaded through the command handlers.
struct Output {
    json: bool,
    quiet: bool,
}

impl Output {
    fn line(&self, text: impl AsRef<str>) {
        if !self.json && !self.quiet {
            println!("{}", text.as_ref());
        }
    }

    fn object(&self, value: serde_json::Value) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
        }
    }

    fn failure(&self, command: &str, code: i32, message: &str) -> i32 {
        self.failure_detail(command, code, message, &[], json!({}))
    }

    /// A failure that carries structured detail: candidate lists and search suggestions.
    fn failure_detail(
        &self,
        command: &str,
        code: i32,
        message: &str,
        lines: &[String],
        detail: serde_json::Value,
    ) -> i32 {
        if self.json {
            let mut object = serde_json::Map::new();
            object.insert("command".into(), json!(command));
            object.insert("ok".into(), json!(false));
            object.insert("exit_code".into(), json!(code));
            object.insert("error".into(), json!(message));
            if let serde_json::Value::Object(extra) = detail {
                for (key, value) in extra {
                    object.insert(key, value);
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::Value::Object(object))
                    .unwrap_or_default()
            );
        } else {
            eprintln!("nerve {command}: {message}");
            for line in lines {
                eprintln!("{line}");
            }
        }
        code
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let usage_error = !matches!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelp
                    | clap::error::ErrorKind::DisplayVersion
                    | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            let _ = err.print();
            std::process::exit(if usage_error {
                exit::USAGE
            } else {
                exit::SUCCESS
            });
        }
    };

    let output = Output {
        json: cli.json,
        quiet: cli.quiet,
    };

    let code = match cli.command {
        Command::Init { path } => run_init(&output, path),
        Command::Index { path, full } => run_index(&output, path, full),
        Command::Coverage { report, path } => run_coverage(&output, path, report),
        Command::Trace(TraceCommand::Import { artifact, path }) => {
            run_trace_import(&output, path, artifact)
        }
        Command::History(HistoryCommand::Sync {
            path,
            max_commits,
            with_identity,
        }) => run_history_sync(&output, path, max_commits, with_identity),
        Command::History(HistoryCommand::Log {
            limit,
            offset,
            path,
        }) => run_history_log(&output, &path, limit, offset),
        Command::History(HistoryCommand::File {
            tree_path,
            limit,
            path,
        }) => run_history_file(&output, &path, &tree_path, limit),
        Command::History(HistoryCommand::Diff {
            from,
            to,
            limit,
            max_walk,
            max_changes,
            path,
        }) => run_history_diff(
            &output,
            &path,
            &from,
            &to,
            nerve_store::StateDiffLimits {
                commits: limit,
                commits_walked: max_walk,
                changes: max_changes,
            },
        ),
        Command::History(HistoryCommand::Frequency { limit, path }) => {
            run_history_frequency(&output, &path, limit)
        }
        Command::History(HistoryCommand::Cochange {
            tree_path,
            limit,
            path,
        }) => run_history_cochange(&output, &path, &tree_path, limit),
        Command::History(HistoryCommand::Availability { path }) => {
            run_history_availability(&output, &path)
        }
        Command::Repo(RepoCommand::Add {
            target,
            id,
            name,
            path,
        }) => run_repo_add(&output, &path, &target, id.as_deref(), name.as_deref()),
        Command::Repo(RepoCommand::List { path }) => run_repo_list(&output, &path),
        Command::Repo(RepoCommand::Links { limit, path }) => run_repo_links(&output, &path, limit),
        Command::Repo(RepoCommand::Remove { registry_id, path }) => {
            run_repo_remove(&output, &path, &registry_id)
        }
        Command::Repo(RepoCommand::Relocate {
            registry_id,
            new_path,
            path,
        }) => run_repo_relocate(&output, &path, &registry_id, &new_path),
        Command::Repo(RepoCommand::Scan { path }) => run_repo_scan(&output, &path),
        Command::Memory(MemoryCommand::Propose {
            subject,
            scope,
            content,
            claim_key,
            author,
            id,
            path,
        }) => run_memory_propose(
            &output,
            &path,
            MemoryProposeArguments {
                subject,
                scope,
                content,
                claim_key,
                author,
                id,
            },
        ),
        Command::Memory(MemoryCommand::Confirm {
            memory_id,
            note,
            path,
        }) => run_memory_confirm(&output, &path, &memory_id, note.as_deref()),
        Command::Memory(MemoryCommand::Supersede {
            predecessor_id,
            content,
            scope,
            claim_key,
            author,
            id,
            note,
            path,
        }) => run_memory_supersede(
            &output,
            &path,
            MemorySupersedeArguments {
                predecessor_id,
                content,
                scope,
                claim_key,
                author,
                id,
                note,
            },
        ),
        Command::Memory(MemoryCommand::Invalidate {
            memory_id,
            reason,
            note,
            path,
        }) => run_memory_invalidate(
            &output,
            &path,
            &memory_id,
            reason.as_deref(),
            note.as_deref(),
        ),
        Command::Memory(MemoryCommand::Cite {
            memory_id,
            file,
            span,
            note,
            path,
        }) => run_memory_cite(
            &output,
            &path,
            &memory_id,
            &file,
            span.as_deref(),
            note.as_deref(),
        ),
        Command::Memory(MemoryCommand::List {
            scope,
            subject,
            status,
            path,
        }) => run_memory_list(
            &output,
            &path,
            MemoryListArguments {
                scope,
                subject,
                status,
            },
        ),
        Command::Memory(MemoryCommand::Show { memory_id, path }) => {
            run_memory_show(&output, &path, &memory_id)
        }
        Command::Memory(MemoryCommand::Search { query, path }) => {
            run_memory_search(&output, &path, &query)
        }
        Command::Memory(MemoryCommand::Events { memory_id, path }) => {
            run_memory_events(&output, &path, &memory_id)
        }
        Command::Memory(MemoryCommand::Export { out, path }) => {
            run_memory_export(&output, &path, out.as_deref())
        }
        Command::Status { path, path_flag } => run_status(
            &output,
            &path.or(path_flag).unwrap_or_else(|| PathBuf::from(".")),
        ),
        Command::Check {
            path,
            path_flag,
            allow_stale,
        } => run_check(
            &output,
            &path.or(path_flag).unwrap_or_else(|| PathBuf::from(".")),
            allow_stale,
        ),
        Command::Doctor { path, path_flag } => doctor::run(
            &output,
            &path.or(path_flag).unwrap_or_else(|| PathBuf::from(".")),
        ),
        Command::Search {
            query,
            kind,
            limit,
            path,
        } => run_search(&output, &path, &query, kind.as_deref(), limit),
        Command::Gaps {
            path,
            path_flag,
            under,
            kind,
            include_partial,
            limit,
        } => {
            let arguments = GapArguments {
                under,
                kind,
                include_partial,
                limit,
            };
            run_gaps(
                &output,
                &path.or(path_flag).unwrap_or_else(|| PathBuf::from(".")),
                arguments,
            )
        }
        Command::Impact {
            selector,
            max_depth,
            relations,
            limit,
            path,
        } => {
            let arguments = ImpactArguments {
                selector,
                max_depth,
                relations,
                limit,
            };
            run_impact(&output, &path, arguments)
        }
        Command::Path {
            from,
            to,
            max_depth,
            relations,
            limit,
            direction,
            resolved_only,
            path,
        } => {
            let arguments = PathArguments {
                from,
                to,
                max_depth,
                relations,
                limit,
                direction,
                resolved_only,
            };
            run_path(&output, &path, arguments)
        }
        Command::Serve {
            path,
            path_flag,
            port,
            workers,
        } => run_serve(
            &output,
            path.or(path_flag).unwrap_or_else(|| PathBuf::from(".")),
            port,
            workers,
        ),
        Command::Mcp { path, path_flag } => run_mcp(
            &output,
            &path.or(path_flag).unwrap_or_else(|| PathBuf::from(".")),
        ),
        Command::Why {
            from,
            to,
            incoming,
            outgoing,
            relations,
            path,
        } => {
            let arguments = WhyArguments {
                from,
                to,
                incoming,
                outgoing,
                relations,
            };
            run_why(&output, &path, arguments)
        }
    };
    std::process::exit(code);
}

fn error_exit_code(err: &IndexError) -> i32 {
    match err {
        IndexError::NotADirectory(_) => exit::USAGE,
        IndexError::NotAFile(_) => exit::USAGE,
        IndexError::NotInitialized(_) => exit::NO_INDEX,
        IndexError::NotIndexed(_) => exit::NO_INDEX,
        IndexError::Config { .. } => exit::NO_INDEX,
        // A path the guard refused is a wrong argument, not an internal failure. `nerve coverage`
        // is the first command that takes a repository path as an argument and can therefore be
        // handed one that escapes the root; reporting that as exit 70 would tell a script that
        // Nerve broke when in fact the script asked for something Nerve refuses.
        IndexError::PathEscapesRoot(_)
        | IndexError::NonUtf8Path(_)
        | IndexError::ControlCharacterInPath(_) => exit::USAGE,
        _ => exit::INTERNAL,
    }
}

fn run_init(output: &Output, path: Option<PathBuf>) -> i32 {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    match nerve_index::init(&root) {
        Ok(outcome) => {
            output.line(format!(
                "Initialized Nerve index at {}",
                outcome.nerve_dir.display()
            ));
            output.line(format!("  project_id     {}", outcome.project_id));
            output.line(format!("  schema_version {}", outcome.schema_version));
            output.line(format!(
                "  state          {}",
                if outcome.created {
                    "created"
                } else {
                    "already initialized"
                }
            ));
            output.object(json!({
                "command": "init",
                "ok": true,
                "exit_code": exit::SUCCESS,
                "root": outcome.root.display().to_string(),
                "nerve_dir": outcome.nerve_dir.display().to_string(),
                "database_path": outcome.db_path.display().to_string(),
                "project_id": outcome.project_id,
                "schema_version": outcome.schema_version,
                "created": outcome.created,
            }));
            exit::SUCCESS
        }
        Err(err) => output.failure("init", error_exit_code(&err), &err.to_string()),
    }
}

/// What incremental indexing decided, re-extracted and removed.
fn incremental_json(step: &nerve_index::IncrementalReport) -> serde_json::Value {
    json!({
        "full": step.full,
        "files_unchanged": step.files_unchanged,
        "files_modified": step.files_modified,
        "files_added": step.files_added,
        "files_removed": step.files_removed,
        "files_resolution_changed": step.files_resolution_changed,
        "files_seeded": step.files_seeded,
        "files_re_extracted": step.files_re_extracted,
        "files_skipped_unchanged": step.files_skipped_unchanged,
        "files_changed": step.files_changed(),
        "amplification": step.amplification(),
        "removed_paths": step.removed_paths,
        "observations_removed": step.observations_removed,
        "occurrences_removed": step.occurrences_removed,
        "assertions_removed": step.assertions_removed,
        "entities_removed": step.entities_removed,
        "assertions_derived": step.assertions_derived,
        "rows_written": step.rows_written,
        "identity_links_proposed": step.identity_links_proposed,
        "identity_links_recorded": step.identity_links_recorded,
    })
}

fn run_index(output: &Output, path: Option<PathBuf>, full: bool) -> i32 {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    let options = nerve_index::IndexOptions { full };
    match nerve_index::index_repository_with(&root, options) {
        Ok(outcome) => {
            let partial = outcome.status == nerve_index::RunStatus::Partial;
            let code = if partial {
                exit::PARTIAL_INDEX
            } else {
                exit::SUCCESS
            };

            output.line(format!("Indexed {}", outcome.root.display()));
            output.line(format!("  state_id       {}", outcome.state_id));
            output.line(format!(
                "  git_commit     {}",
                outcome.git_commit.as_deref().unwrap_or("(none)")
            ));
            output.line(format!(
                "  files          {} parsed, {} failed, {} unsupported, {} denied",
                outcome.files_processed,
                outcome.files_failed,
                outcome.skipped_unsupported,
                outcome.denied_secrets.len()
            ));

            let step = &outcome.incremental;
            output.line(format!(
                "  mode           {}",
                if step.full { "full" } else { "incremental" }
            ));
            output.line(format!(
                "  changed        {} modified, {} added, {} removed, {} re-resolved",
                step.files_modified,
                step.files_added,
                step.files_removed,
                step.files_resolution_changed
            ));
            output.line(format!(
                "  re-extracted   {} of {} ({} skipped unchanged)",
                step.files_re_extracted, outcome.files_processed, step.files_skipped_unchanged
            ));
            output.line(format!(
                "  amplification  {}",
                match step.amplification() {
                    Some(factor) => format!(
                        "{factor:.2} files re-extracted per changed file ({} changed)",
                        step.files_changed()
                    ),
                    None => "n/a (nothing changed)".to_string(),
                }
            ));
            // Deletion is destructive, so it is reported whether or not anyone asked.
            output.line(format!(
                "  removed        {} observations, {} occurrences, {} assertions, {} entities",
                step.observations_removed,
                step.occurrences_removed,
                step.assertions_removed,
                step.entities_removed
            ));
            for path in &step.removed_paths {
                output.line(format!("    gone         {path}"));
            }
            // What the run cost the database. Work proportional to the change is the property
            // incremental indexing exists to have, so it is reported rather than assumed.
            output.line(format!(
                "  wrote          {} database rows ({} assertion states derived)",
                step.rows_written, step.assertions_derived
            ));
            if step.identity_links_proposed > 0 {
                output.line(format!(
                    "  identity       {} link(s) proposed, {} recorded",
                    step.identity_links_proposed, step.identity_links_recorded
                ));
            }

            output.line(format!("  entities       {}", outcome.entities_total));
            for (kind, count) in &outcome.entities_by_kind {
                output.line(format!("    {kind:<12} {count}"));
            }
            // `entities` counts every kind; `symbols` only the four the vocabulary calls symbols.
            output.line(format!("  symbols        {}", outcome.symbols_total));
            output.line(format!("  assertions     {}", outcome.assertions_total));
            for (relation, count) in &outcome.assertions_by_relation {
                output.line(format!("    {relation:<12} {count}"));
            }
            output.line(format!("  observations   {}", outcome.observations_total));
            output.line(format!(
                "  unresolved     {} entities, {} assertions",
                outcome.unresolved_entities, outcome.unresolved_assertions
            ));
            output.line(format!(
                "  unmodelled     {} call/heritage sites",
                outcome.unmodelled_call_sites
            ));
            for (form, count) in &outcome.unmodelled_by_form {
                output.line(format!("    {form:<18} {count}"));
            }
            output.line(format!(
                "  documents      {} scanned, {} ADRs, {} sections",
                outcome.documents_processed, outcome.adr_documents, outcome.document_sections
            ));
            // Every Markdown construct outside the supported subset, and every resource bound
            // that fired. Reported whether or not anyone asked: a bound that refused something
            // silently would be indistinguishable from a document that contained nothing.
            output.line(format!(
                "  md-unsupported {} constructs refused",
                outcome.unsupported_markdown
            ));
            for (form, count) in &outcome.unsupported_markdown_by_form {
                output.line(format!("    {form:<28} {count}"));
            }
            // A cycle is reported, never suppressed: each edge is individually evidenced by an
            // explicit statement in a real file, and deleting one to make the graph acyclic
            // would hide evidence. A contradiction is a document another document claims to have
            // replaced, whose own status still says `Accepted` — a string comparison.
            output.line(format!(
                "  supersession   {} edges, {} cycle(s) over {} documents, {} status contradiction(s)",
                outcome.supersession_edges,
                outcome.supersession_cycles,
                outcome.supersession_cycle_documents,
                outcome.supersession_contradictions
            ));
            output.line(format!("  duration_ms    {}", outcome.duration_ms));
            if partial {
                output.line(format!(
                    "  note           {} file(s) skipped as too large, unreadable or not UTF-8",
                    outcome.files_failed
                ));
            }

            output.object(json!({
                "command": "index",
                "ok": true,
                "exit_code": code,
                "root": outcome.root.display().to_string(),
                "state_id": outcome.state_id,
                "git_commit": outcome.git_commit,
                "status": outcome.status.as_str(),
                "files_processed": outcome.files_processed,
                "files_failed": outcome.files_failed,
                "files_with_syntax_errors": outcome.files_with_syntax_errors,
                "skipped_unsupported": outcome.skipped_unsupported,
                "skipped_symlinks": outcome.skipped_symlinks,
                "denied_secrets": outcome.denied_secrets,
                "dynamic_imports_without_specifier": outcome.dynamic_imports_without_specifier,
                "unmodelled_call_sites": outcome.unmodelled_call_sites,
                "unmodelled_by_form": outcome.unmodelled_by_form,
                "documents_processed": outcome.documents_processed,
                "adr_documents": outcome.adr_documents,
                "document_sections": outcome.document_sections,
                "unsupported_markdown": outcome.unsupported_markdown,
                "unsupported_markdown_by_form": outcome.unsupported_markdown_by_form,
                "supersession_edges": outcome.supersession_edges,
                "supersession_cycles": outcome.supersession_cycles,
                "supersession_cycle_documents": outcome.supersession_cycle_documents,
                "supersession_contradictions": outcome.supersession_contradictions,
                "entities_total": outcome.entities_total,
                "symbols_total": outcome.symbols_total,
                "entities_by_kind": outcome.entities_by_kind,
                "assertions_total": outcome.assertions_total,
                "assertions_by_relation": outcome.assertions_by_relation,
                "observations_total": outcome.observations_total,
                "unresolved_entities": outcome.unresolved_entities,
                "unresolved_assertions": outcome.unresolved_assertions,
                "duration_ms": outcome.duration_ms,
                "incremental": incremental_json(step),
            }));
            code
        }
        Err(err) => output.failure("index", error_exit_code(&err), &err.to_string()),
    }
}

/// Ingest one LCOV report the user named into an existing index.
///
/// Exit codes follow `nerve index`: 0 when every record in the report was believed, 3 when part
/// of it was refused — a path outside the repository, a file Nerve never indexed, a file whose
/// bytes have moved since it was, or anything the parser declined. Lines that map to no symbol
/// are *not* a partial ingestion: that is the documented lossiness of mapping lines onto symbols,
/// it is reported as a number, and treating it as a failure would make every real repository
/// exit 3 forever.
fn run_coverage(output: &Output, path: Option<PathBuf>, report: PathBuf) -> i32 {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    match nerve_index::ingest_coverage(&root, &report) {
        Ok(outcome) => {
            let partial = outcome.status == nerve_index::RunStatus::Partial;
            let code = if partial {
                exit::PARTIAL_INDEX
            } else {
                exit::SUCCESS
            };

            output.line(format!("Ingested coverage from {}", outcome.report_path));
            output.line(format!("  root           {}", outcome.root.display()));
            output.line(format!(
                "  report_hash    {}",
                outcome.report_content_hash.as_deref().unwrap_or("(unread)")
            ));
            output.line(format!(
                "  coverage_run   {}",
                outcome
                    .coverage_run_entity_id
                    .as_deref()
                    .unwrap_or("(none written)")
            ));
            output.line(format!("  state_id       {}", outcome.state_id));
            output.line(format!(
                "  files          {} in report, {} ingested, {} refused",
                outcome.files_in_report, outcome.files_ingested, outcome.files_refused
            ));
            // `partial` is a recorded value, never rounded, so it is reported as its own number
            // rather than folded into a percentage.
            output.line(format!(
                "  symbols        {} covered ({} fully, {} partially)",
                outcome.symbols_covered,
                outcome.symbols_fully_covered,
                outcome.symbols_partially_covered
            ));
            output.line(format!(
                "  lines          {} covered, {} instrumented and unexecuted",
                outcome.covered_lines, outcome.uncovered_lines
            ));
            output.line(format!(
                "  refused        {} in total",
                outcome.refused_total()
            ));
            for (form, count) in &outcome.refused {
                output.line(format!("    {form:<28} {count}"));
            }
            output.line(format!(
                "  superseded     {} observations, {} occurrences, {} assertions, {} entities",
                outcome.observations_removed,
                outcome.occurrences_removed,
                outcome.assertions_removed,
                outcome.entities_removed
            ));
            output.line(format!(
                "  wrote          {} database rows",
                outcome.rows_written
            ));
            output.line(format!("  duration_ms    {}", outcome.duration_ms));
            output.line(String::new());
            output.line(
                "  Coverage is not a call graph. The source of every edge is the coverage run,",
            );
            output.line(
                "  not a test: LCOV carries no per-test attribution, so \"which tests would my",
            );
            output.line("  change affect?\" is not answerable from this report.");

            output.object(json!({
                "command": "coverage",
                "ok": true,
                "exit_code": code,
                "root": outcome.root.display().to_string(),
                "report_path": outcome.report_path,
                "report_content_hash": outcome.report_content_hash,
                "coverage_run_entity_id": outcome.coverage_run_entity_id,
                "state_id": outcome.state_id,
                "status": outcome.status.as_str(),
                "files_in_report": outcome.files_in_report,
                "files_ingested": outcome.files_ingested,
                "files_refused": outcome.files_refused,
                "symbols_covered": outcome.symbols_covered,
                "symbols_fully_covered": outcome.symbols_fully_covered,
                "symbols_partially_covered": outcome.symbols_partially_covered,
                "covered_lines": outcome.covered_lines,
                "uncovered_lines": outcome.uncovered_lines,
                "refused": outcome.refused,
                "refused_total": outcome.refused_total(),
                "rows_written": outcome.rows_written,
                "observations_removed": outcome.observations_removed,
                "occurrences_removed": outcome.occurrences_removed,
                "assertions_removed": outcome.assertions_removed,
                "entities_removed": outcome.entities_removed,
                "duration_ms": outcome.duration_ms,
                "per_test_attribution": false,
            }));
            code
        }
        Err(err) => output.failure("coverage", error_exit_code(&err), &err.to_string()),
    }
}

/// Read one trace artifact the user named into an existing index.
///
/// Exit codes follow `nerve coverage`: 0 when the whole artifact was believed and the traced run
/// itself finished, 3 when anything was refused **or** the run did not finish. The second half is the
/// point — a partial run is a partial answer, and a script that only checked for refusals would
/// treat an interrupted suite's trace as a complete one.
fn run_trace_import(output: &Output, path: Option<PathBuf>, artifact: PathBuf) -> i32 {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    match nerve_index::ingest_trace(&root, &artifact) {
        Ok(outcome) => {
            let partial = outcome.status == nerve_index::RunStatus::Partial;
            let code = if partial {
                exit::PARTIAL_INDEX
            } else {
                exit::SUCCESS
            };

            output.line(format!("Imported trace from {}", outcome.artifact_path));
            output.line(format!("  root           {}", outcome.root.display()));
            output.line(format!(
                "  artifact_hash  {}",
                outcome
                    .artifact_content_hash
                    .as_deref()
                    .unwrap_or("(unread)")
            ));
            output.line(format!(
                "  run_id         {}",
                outcome.run_id.as_deref().unwrap_or("(no usable header)")
            ));
            output.line(format!("  state_id       {}", outcome.state_id));
            // Three values, never two: `unverified` is not `bound` and is not `stale`.
            output.line(format!(
                "  binding        {}",
                outcome
                    .binding
                    .map(|binding| binding.as_str())
                    .unwrap_or("(refused)")
            ));
            output.line(format!(
                "  run            {}{}",
                outcome
                    .completion_state
                    .map(|state| state.as_str())
                    .unwrap_or("(unknown)"),
                outcome
                    .partial_reason
                    .as_deref()
                    .map(|reason| format!(" — {reason}"))
                    .unwrap_or_default()
            ));
            output.line(format!(
                "  records        {} in artifact, {} accepted, {} unsupported",
                outcome.records_in_artifact, outcome.records_accepted, outcome.records_unsupported
            ));
            output.line(format!(
                "  edges          {} observed over {} call site(s), {} restated",
                outcome.edges_observed, outcome.observations_written, outcome.observations_merged
            ));
            output.line(format!(
                "  refused        {} in total",
                outcome.refused_total()
            ));
            for (form, count) in &outcome.refused {
                output.line(format!("    {form:<28} {count}"));
            }
            output.line(format!(
                "  limitations    {} record(s) the producer could not model",
                outcome.limitations_total()
            ));
            for (form, count) in &outcome.limitations {
                output.line(format!("    {form:<28} {count}"));
            }
            if !outcome.declared_limitations.is_empty() {
                output.line(format!(
                    "  declared       {}",
                    outcome.declared_limitations.join(", ")
                ));
            }
            output.line(format!(
                "  wrote          {} database rows",
                outcome.rows_written
            ));
            output.line(format!("  duration_ms    {}", outcome.duration_ms));
            output.line(String::new());
            output
                .line("  A trace is existential evidence: it says this run took these edges, not");
            output.line("  that every run does, and absence of an edge is absence of observation.");
            output
                .line("  The endpoints are the two frames of each call — never the test, which is");
            output.line("  recorded on the evidence instead. Nerve ran no tests to produce this.");

            output.object(json!({
                "command": "trace import",
                "ok": true,
                "exit_code": code,
                "root": outcome.root.display().to_string(),
                "artifact_path": outcome.artifact_path,
                "artifact_content_hash": outcome.artifact_content_hash,
                "state_id": outcome.state_id,
                "run_id": outcome.run_id,
                "repository_binding": outcome.binding.map(|binding| binding.as_str()),
                "completion_state": outcome.completion_state.map(|state| state.as_str()),
                "partial_reason": outcome.partial_reason,
                "declared_limitations": outcome.declared_limitations,
                "status": outcome.status.as_str(),
                "records_in_artifact": outcome.records_in_artifact,
                "records_accepted": outcome.records_accepted,
                "records_unsupported": outcome.records_unsupported,
                "edges_observed": outcome.edges_observed,
                "observations_written": outcome.observations_written,
                "observations_merged": outcome.observations_merged,
                "refused": outcome.refused,
                "refused_total": outcome.refused_total(),
                "limitations": outcome.limitations,
                "limitations_total": outcome.limitations_total(),
                "rows_written": outcome.rows_written,
                "duration_ms": outcome.duration_ms,
                "runs_tests": false,
            }));
            code
        }
        Err(err) => output.failure("trace import", error_exit_code(&err), &err.to_string()),
    }
}

/// An opened index: the repository root, the database path, and a connection.
struct OpenIndex {
    root: PathBuf,
    db_path: PathBuf,
    conn: nerve_store::Connection,
}

fn open_existing(path: &Path) -> Result<OpenIndex, String> {
    let root = std::fs::canonicalize(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(format!(
            "no Nerve index at {}; run `nerve init` first",
            root.display()
        ));
    }
    let conn = nerve_store::open(&db_path).map_err(|err| err.to_string())?;
    Ok(OpenIndex {
        root,
        db_path,
        conn,
    })
}

fn run_json(run: &nerve_store::ExtractorRunSummary) -> serde_json::Value {
    json!({
        "run_id": run.run_id,
        "state_id": run.state_id,
        "extractor_id": run.extractor_id,
        "extractor_version": run.extractor_version,
        "started_at": run.started_at,
        "finished_at": run.finished_at,
        "files_processed": run.files_processed,
        "files_failed": run.files_failed,
        "status": run.status,
    })
}

fn run_status(output: &Output, path: &Path) -> i32 {
    let OpenIndex { db_path, conn, .. } = match open_existing(path) {
        Ok(opened) => opened,
        Err(message) => return output.failure("status", exit::NO_INDEX, &message),
    };
    let report = match nerve_store::status(&conn) {
        Ok(report) => report,
        Err(err) => return output.failure("status", exit::INTERNAL, &err.to_string()),
    };
    let database_bytes = nerve_store::database_bytes(&db_path);
    let healthy = report.is_healthy();

    output.line(format!("Nerve index at {}", db_path.display()));
    output.line(format!(
        "  schema_version {}",
        report
            .schema_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "(none)".into())
    ));
    output.line(format!(
        "  project_id     {}",
        report.project_id.as_deref().unwrap_or("(none)")
    ));
    output.line(format!(
        "  root_path      {}",
        report.root_path.as_deref().unwrap_or("(none)")
    ));
    output.line(format!(
        "  state_id       {}",
        report.state_id.as_deref().unwrap_or("(none)")
    ));
    output.line(format!(
        "  git_commit     {}",
        report.git_commit.as_deref().unwrap_or("(none)")
    ));
    output.line(format!("  database_bytes {database_bytes}"));
    output.line(format!("  entities       {}", report.entities_total));
    for (kind, count) in &report.entities_by_kind {
        output.line(format!("    {kind:<12} {count}"));
    }
    // Reported next to `entities` because the two are routinely confused: `entities` counts
    // every kind, `symbols` only the four the vocabulary calls symbols.
    output.line(format!("  symbols        {}", report.symbols_total));
    output.line(format!("  assertions     {}", report.assertions_total));
    for (relation, count) in &report.assertions_by_relation {
        output.line(format!("    {relation:<12} {count}"));
    }
    output.line(format!("  occurrences    {}", report.occurrences_total));
    output.line(format!("  observations   {}", report.observations_total));
    output.line(format!(
        "  unresolved     {} entities, {} assertions",
        report.unresolved_entities, report.unresolved_assertions
    ));
    match &report.last_run {
        Some(run) => {
            output.line(format!(
                "  last_run       {} {} at {} ({}), {} processed, {} failed",
                run.extractor_id,
                run.extractor_version,
                run.finished_at.as_deref().unwrap_or(&run.started_at),
                run.status,
                run.files_processed,
                run.files_failed
            ));
        }
        None => output.line("  last_run       (never indexed)"),
    }
    output.line(format!("  runs           {}", report.runs.len()));
    for run in &report.runs {
        output.line(format!(
            "    {:<18} {:<7} {} ({}), {} processed, {} failed",
            run.extractor_id,
            run.extractor_version,
            run.finished_at.as_deref().unwrap_or(&run.started_at),
            run.status,
            run.files_processed,
            run.files_failed
        ));
    }
    output.line(format!(
        "  healthy        {}",
        if healthy { "yes" } else { "no" }
    ));

    output.object(json!({
        "command": "status",
        "ok": true,
        "exit_code": if healthy { exit::SUCCESS } else { exit::NO_INDEX },
        "healthy": healthy,
        "database_path": db_path.display().to_string(),
        "database_bytes": database_bytes,
        "schema_version": report.schema_version,
        "project_id": report.project_id,
        "root_path": report.root_path,
        "state_id": report.state_id,
        "git_commit": report.git_commit,
        "entities_total": report.entities_total,
        "symbols_total": report.symbols_total,
        "entities_by_kind": report.entities_by_kind,
        "assertions_total": report.assertions_total,
        "assertions_by_relation": report.assertions_by_relation,
        "occurrences_total": report.occurrences_total,
        "observations_total": report.observations_total,
        "assertion_states_total": report.assertion_states_total,
        "unresolved_entities": report.unresolved_entities,
        "unresolved_assertions": report.unresolved_assertions,
        "last_run": report.last_run.as_ref().map(run_json),
        "runs": report.runs.iter().map(run_json).collect::<Vec<_>>(),
    }));

    if healthy {
        exit::SUCCESS
    } else {
        exit::NO_INDEX
    }
}

// ---- check -----------------------------------------------------------------------------------
//
// One question — *can I trust this index right now?* — and no new analysis to answer it. Every
// fact below is already produced by `nerve_store::status`, `nerve_index::index_freshness` and
// `nerve_index::untracked_files`; what this section adds is the judgement over them and the exit
// code that carries it.

/// How many indexed files `nerve check` re-hashes before it reports a partial sweep.
///
/// The bound `/api/overview` already uses, for the same reason: a repository can hold a hundred
/// thousand files and this answer is wanted in a pre-commit hook. When the cap bites, the sweep
/// is deliberately **not** reported as clean — see [`judge_freshness`].
const CHECK_PROBE_CAP: usize = 5_000;

/// How many added paths the human output names before it stops and gives a count.
const CHECK_ADDED_SHOWN: usize = 10;

/// What `nerve check` decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Every indexed file still hashes to what was extracted, and nothing new is untracked.
    Current,
    /// There is nothing to judge: no database, no schema, or nothing ever indexed.
    NoIndex,
    /// An index exists but cannot be used as it stands: the schema is behind, or a run is open.
    Unusable,
    /// The index is internally sound and describes a tree that has moved on.
    Stale,
    /// The index is internally sound and the sweep could not establish whether it is current.
    ///
    /// Separate from [`Verdict::Stale`] because the evidence is different — nothing was observed
    /// to have changed, some of the tree was simply never looked at — and identical to it in exit
    /// code, because "I could not check" is not a clean bill of health.
    Unverified,
}

impl Verdict {
    /// Canonical name used in rendered and `--json` output.
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Current => "current",
            Verdict::NoIndex => "no_index",
            Verdict::Unusable => "unusable",
            Verdict::Stale => "stale",
            Verdict::Unverified => "unverified",
        }
    }

    /// The exit code this verdict carries. The only place the mapping exists.
    fn exit_code(self, allow_stale: bool) -> i32 {
        match self {
            Verdict::Current => exit::SUCCESS,
            Verdict::NoIndex => exit::NO_INDEX,
            Verdict::Unusable => exit::PARTIAL_INDEX,
            Verdict::Stale | Verdict::Unverified => {
                if allow_stale {
                    exit::SUCCESS
                } else {
                    exit::STALE_INDEX
                }
            }
        }
    }
}

/// Whether the schema on disk is one this build can read at all.
///
/// Answered on its own, and answered first, because every other question below reads a table:
/// a database written by an older or newer build may not have the tables `status` queries, and
/// reporting that as an internal error would tell a script Nerve broke when in fact the index
/// needs migrating.
fn judge_schema(schema_version: Option<i64>) -> Option<(Verdict, String)> {
    let Some(version) = schema_version else {
        return Some((
            Verdict::NoIndex,
            "the database has never been migrated; run `nerve init`".to_string(),
        ));
    };
    if version != nerve_store::SCHEMA_VERSION {
        return Some((
            Verdict::Unusable,
            format!(
                "schema version {version} is not the supported version {}; \
                 run `nerve index` to migrate",
                nerve_store::SCHEMA_VERSION
            ),
        ));
    }
    None
}

/// Whether the index is usable at all, before anything is compared against the disk.
///
/// `None` means there is an index worth measuring the tree against; anything else is a refusal
/// and the freshness sweep is not run, because re-hashing a tree to compare it with a graph that
/// cannot be read would be work in service of an answer nobody can use.
fn judge_index(
    schema_version: Option<i64>,
    ever_indexed: bool,
    runs_running: usize,
) -> Option<(Verdict, String)> {
    if let Some(refusal) = judge_schema(schema_version) {
        return Some(refusal);
    }
    if !ever_indexed {
        return Some((
            Verdict::NoIndex,
            "the database is initialized and nothing has been indexed; run `nerve index`"
                .to_string(),
        ));
    }
    if runs_running > 0 {
        return Some((
            Verdict::Unusable,
            format!(
                "{runs_running} extractor run(s) are still marked running; the last index did \
                 not finish, so the graph is a half-written one"
            ),
        ));
    }
    None
}

/// Whether the index still describes the tree, given the sweep and the untracked walk.
///
/// The five freshness counts and the added count fall into two families, and they are kept apart
/// because the evidence behind them is different:
///
/// - **observed divergence** — `stale` (the file changed), `missing` (the indexed file is gone)
///   and `added` (a file exists that no row describes). Each is a measurement, and any of them
///   means the graph and the tree disagree.
/// - **not established** — `refused` (the path-safety check would not read it), `unreadable`
///   (allowed but the bytes would not come) and `truncated` (the cap stopped the sweep). Nothing
///   here says the index is wrong; it says this run did not find out.
///
/// Both exit non-zero. The second family is the reason a truncated sweep can never report a clean
/// result: a partial sweep that returned `0` would be a clean bill of health issued without
/// looking, which is exactly the failure mode `check` exists to prevent.
fn judge_freshness(freshness: &nerve_index::IndexFreshness, added: usize) -> (Verdict, String) {
    if freshness.stale + freshness.missing + added > 0 {
        return (
            Verdict::Stale,
            format!(
                "{} indexed file(s) changed, {} no longer exist and {} file(s) are not indexed \
                 at all",
                freshness.stale, freshness.missing, added
            ),
        );
    }
    if freshness.truncated {
        return (
            Verdict::Unverified,
            format!(
                "the sweep compared {} of {} indexed file(s) before reaching its {CHECK_PROBE_CAP}-file \
                 cap; the rest were never looked at",
                freshness.files_probed, freshness.files_total
            ),
        );
    }
    if freshness.refused + freshness.unreadable > 0 {
        return (
            Verdict::Unverified,
            format!(
                "{} indexed file(s) were refused by the path-safety check and {} could not be \
                 read, so they were never compared",
                freshness.refused, freshness.unreadable
            ),
        );
    }
    (
        Verdict::Current,
        format!(
            "{} indexed file(s) still hash to what was extracted, and nothing in the tree is \
             untracked",
            freshness.fresh
        ),
    )
}

/// Runs left open, counted once whether they reach us through `runs` or through `last_run`.
fn runs_still_running(report: &nerve_store::StatusReport) -> usize {
    let mut count = report
        .runs
        .iter()
        .filter(|run| run.status == "running")
        .count();
    if let Some(last) = &report.last_run {
        if last.status == "running" && !report.runs.iter().any(|run| run.run_id == last.run_id) {
            count += 1;
        }
    }
    count
}

/// What the sweep measured, or nothing when there was no index to sweep.
struct CheckMeasurement {
    freshness: nerve_index::IndexFreshness,
    untracked: nerve_index::UntrackedFiles,
}

/// One complete answer from `nerve check`, in the shape the renderer needs it.
struct CheckAnswer<'a> {
    root: &'a Path,
    db_path: Option<&'a Path>,
    verdict: Verdict,
    reason: String,
    allow_stale: bool,
    schema_version: Option<i64>,
    runs_running: usize,
    measured: Option<CheckMeasurement>,
}

/// Render one verdict and return its exit code.
///
/// Every outcome takes the same shape, failures included: `check`'s product is a judgement, and a
/// script that must parse one object for a clean index and a different one for a stale index
/// would be a script that gets the stale case wrong. The report goes to stdout rather than being
/// duplicated on stderr, because it is an answer and not an error.
fn render_check(output: &Output, answer: &CheckAnswer<'_>) -> i32 {
    let root = answer.root;
    let db_path = answer.db_path;
    let verdict = answer.verdict;
    let reason = answer.reason.as_str();
    let allow_stale = answer.allow_stale;
    let schema_version = answer.schema_version;
    let runs_running = answer.runs_running;
    let measured = answer.measured.as_ref();

    let code = verdict.exit_code(allow_stale);
    let downgraded = code == exit::SUCCESS && verdict != Verdict::Current;

    match db_path {
        Some(path) => output.line(format!("Nerve index at {}", path.display())),
        None => output.line(format!("No Nerve index at {}", root.display())),
    }
    output.line(format!("  verdict        {}", verdict.as_str()));
    output.line(format!("  reason         {reason}"));
    output.line(format!(
        "  schema_version {} (supported {})",
        schema_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "(none)".into()),
        nerve_store::SCHEMA_VERSION
    ));
    output.line(format!("  runs_running   {runs_running}"));
    if let Some(measured) = measured {
        let freshness = &measured.freshness;
        output.line(format!(
            "  files          {} indexed, {} probed, sweep {}",
            freshness.files_total,
            freshness.files_probed,
            if freshness.truncated {
                "TRUNCATED"
            } else {
                "complete"
            }
        ));
        output.line(format!("  fresh          {}", freshness.fresh));
        output.line(format!("  changed        {}", freshness.stale));
        output.line(format!("  removed        {}", freshness.missing));
        output.line(format!(
            "  added          {}",
            measured.untracked.added.len()
        ));
        for path in measured.untracked.added.iter().take(CHECK_ADDED_SHOWN) {
            output.line(format!("    new          {path}"));
        }
        if measured.untracked.added.len() > CHECK_ADDED_SHOWN {
            output.line(format!(
                "    ...          {} more",
                measured.untracked.added.len() - CHECK_ADDED_SHOWN
            ));
        }
        output.line(format!(
            "  unverified     {} refused, {} unreadable",
            freshness.refused, freshness.unreadable
        ));
        // Reported whether or not anyone asked: these are files the tree has and the index will
        // never have, and silence about them would read as "nothing to see".
        output.line(format!(
            "  unindexable    {} file(s) the indexer could not read either",
            measured.untracked.unindexable
        ));
    }
    if downgraded {
        output.line(String::new());
        output.line(format!(
            "  --allow-stale was given, so this exits 0. The index is still {}.",
            verdict.as_str()
        ));
    }
    output.line(format!("  exit_code      {code}"));

    output.object(json!({
        "command": "check",
        "ok": code == exit::SUCCESS,
        "exit_code": code,
        // Tracks the exit code rather than the verdict, so `ok: true` is never accompanied by an
        // error. A downgraded verdict is still fully reported, in `verdict`, `reason`,
        // `downgraded` and the freshness counts — none of which `--allow-stale` touches.
        "error": if code == exit::SUCCESS { serde_json::Value::Null } else { json!(reason) },
        "verdict": verdict.as_str(),
        "reason": reason,
        "allow_stale": allow_stale,
        "downgraded": downgraded,
        "root": root.display().to_string(),
        "database_path": db_path.map(|path| path.display().to_string()),
        "schema_version": schema_version,
        "supported_schema_version": nerve_store::SCHEMA_VERSION,
        "runs_running": runs_running,
        "freshness": measured.map(|measured| json!({
            "files_total": measured.freshness.files_total,
            "files_probed": measured.freshness.files_probed,
            "fresh": measured.freshness.fresh,
            "stale": measured.freshness.stale,
            "missing": measured.freshness.missing,
            "refused": measured.freshness.refused,
            "unreadable": measured.freshness.unreadable,
            "truncated": measured.freshness.truncated,
        })),
        "added": measured.map(|measured| measured.untracked.added.len()),
        "added_paths": measured.map(|measured| measured.untracked.added.clone()).unwrap_or_default(),
        "unindexable": measured.map(|measured| measured.untracked.unindexable),
    }));

    code
}

/// Judge whether the index can be trusted, and exit with the answer.
fn run_check(output: &Output, path: &Path, allow_stale: bool) -> i32 {
    /// One unmeasured verdict: reached before, or instead of, the freshness sweep.
    fn unmeasured<'a>(
        root: &'a Path,
        db_path: Option<&'a Path>,
        verdict: Verdict,
        reason: String,
        allow_stale: bool,
        schema_version: Option<i64>,
        runs_running: usize,
    ) -> CheckAnswer<'a> {
        CheckAnswer {
            root,
            db_path,
            verdict,
            reason,
            allow_stale,
            schema_version,
            runs_running,
            measured: None,
        }
    }

    let root = match std::fs::canonicalize(path) {
        Ok(root) => root,
        Err(err) => {
            return render_check(
                output,
                &unmeasured(
                    path,
                    None,
                    Verdict::NoIndex,
                    format!("{}: {err}", path.display()),
                    allow_stale,
                    None,
                    0,
                ),
            )
        }
    };
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return render_check(
            output,
            &unmeasured(
                &root,
                None,
                Verdict::NoIndex,
                "there is no Nerve database here; run `nerve init` then `nerve index`".to_string(),
                allow_stale,
                None,
                0,
            ),
        );
    }
    // `query_only` is the whole "check never writes" guarantee, made by construction rather than
    // by discipline: SQLite refuses the write, so no future edit to this handler can quietly
    // repair the thing it was asked to judge.
    let conn = match nerve_store::open(&db_path) {
        Ok(conn) => match conn.pragma_update(None, "query_only", "ON") {
            Ok(()) => conn,
            Err(err) => return output.failure("check", exit::INTERNAL, &err.to_string()),
        },
        Err(err) => return output.failure("check", exit::INTERNAL, &err.to_string()),
    };

    let schema_version = match nerve_store::schema_version(&conn) {
        Ok(version) => version,
        Err(err) => return output.failure("check", exit::INTERNAL, &err.to_string()),
    };
    if let Some((verdict, reason)) = judge_schema(schema_version) {
        return render_check(
            output,
            &unmeasured(
                &root,
                Some(&db_path),
                verdict,
                reason,
                allow_stale,
                schema_version,
                0,
            ),
        );
    }

    let report = match nerve_store::status(&conn) {
        Ok(report) => report,
        Err(err) => return output.failure("check", exit::INTERNAL, &err.to_string()),
    };
    let runs_running = runs_still_running(&report);
    let repository = match nerve_store::repository(&conn) {
        Ok(repository) => repository,
        Err(err) => return output.failure("check", exit::INTERNAL, &err.to_string()),
    };
    let ever_indexed = report.last_run.is_some() && repository.is_some();
    if let Some((verdict, reason)) = judge_index(schema_version, ever_indexed, runs_running) {
        return render_check(
            output,
            &unmeasured(
                &root,
                Some(&db_path),
                verdict,
                reason,
                allow_stale,
                schema_version,
                runs_running,
            ),
        );
    }
    let repo_id = repository
        .expect("ever_indexed implies a repository row")
        .repo_id;

    // Freshness is computed by re-reading the repository, so the reader is built from the
    // repository root and enforces the Slice 1 path rules on every path the database supplies.
    let prober = match nerve_index::RepositoryProber::new(&root) {
        Ok(prober) => prober,
        Err(err) => return output.failure("check", error_exit_code(&err), &err.to_string()),
    };
    let freshness = match nerve_index::index_freshness(&conn, &repo_id, &prober, CHECK_PROBE_CAP) {
        Ok(freshness) => freshness,
        Err(err) => return output.failure("check", exit::INTERNAL, &err.to_string()),
    };
    // The sweep above walks the cache, so it can only ask about files the index already knows.
    // A file added since the last index has no row to compare and would otherwise be invisible.
    let untracked = match nerve_index::untracked_files(&root, &conn, &repo_id) {
        Ok(untracked) => untracked,
        Err(err) => return output.failure("check", error_exit_code(&err), &err.to_string()),
    };

    let (verdict, reason) = judge_freshness(&freshness, untracked.added.len());
    render_check(
        output,
        &CheckAnswer {
            root: &root,
            db_path: Some(&db_path),
            verdict,
            reason,
            allow_stale,
            schema_version,
            runs_running,
            measured: Some(CheckMeasurement {
                freshness,
                untracked,
            }),
        },
    )
}

fn run_search(output: &Output, path: &Path, query: &str, kind: Option<&str>, limit: usize) -> i32 {
    if let Some(kind) = kind {
        if kind.parse::<nerve_core::EntityKind>().is_err() {
            let allowed = nerve_core::EntityKind::ALL
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return output.failure(
                "search",
                exit::USAGE,
                &format!("unknown --kind {kind:?}; expected one of: {allowed}"),
            );
        }
    }

    let conn = match open_existing(path) {
        Ok(opened) => opened.conn,
        Err(message) => return output.failure("search", exit::NO_INDEX, &message),
    };

    let hits = match nerve_store::search_entities(&conn, query, kind, limit) {
        Ok(hits) => hits,
        Err(err) => return output.failure("search", exit::INTERNAL, &err.to_string()),
    };

    if hits.is_empty() {
        output.line(format!("No matches for {query:?}."));
    }
    for hit in &hits {
        output.line(format!(
            "{:<10} {:<32} {}",
            hit.kind,
            hit.qualified_name(),
            hit.location()
        ));
    }

    output.object(json!({
        "command": "search",
        "ok": true,
        "exit_code": exit::SUCCESS,
        "query": query,
        "kind": kind,
        "limit": limit,
        "count": hits.len(),
        "results": hits.iter().map(|hit| json!({
            "entity_id": hit.entity_id,
            "kind": hit.kind,
            "name": hit.name,
            "scope_path": hit.scope_path,
            "language": hit.language,
            "file_path": hit.file_path,
            "start_line": hit.start_line,
            "end_line": hit.end_line,
            "score": hit.score,
        })).collect::<Vec<_>>(),
    }));

    exit::SUCCESS
}

// ---- gaps ------------------------------------------------------------------------------------

/// Everything `nerve gaps` was asked for, minus the repository.
struct GapArguments {
    under: Option<String>,
    kind: Option<String>,
    include_partial: bool,
    limit: usize,
}

/// The symbol kinds a coverage gap can be about, for the `--kind` refusal message.
fn symbol_kind_names() -> String {
    nerve_core::EntityKind::ALL
        .iter()
        .filter(|kind| kind.is_symbol())
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn coverage_run_json(run: &nerve_store::CoverageRunRef) -> serde_json::Value {
    json!({
        "entity_id": run.entity_id,
        "report_path": run.report_path,
        "report_content_hash": run.report_content_hash,
        "freshness": run.freshness.map(|freshness| freshness.as_str()),
        "source_files_in_report": run.source_files_in_report,
    })
}

/// Report symbols no test is known to touch.
///
/// Exit 0 in every successful case, gaps or none. Reporting a fact is not a failure, and a
/// command that exited non-zero because a repository has untested code would be unusable as the
/// reporting tool it is; `nerve check` is where CI-failing behaviour belongs. The **unanswerable**
/// case exits 0 too, because there is nothing wrong with the index — but it says so in the first
/// line of human output and in `answerable` in `--json`, and its tallies are `null` rather than
/// zero, so a script cannot read "no coverage was ever ingested" as "no gaps".
fn run_gaps(output: &Output, path: &Path, arguments: GapArguments) -> i32 {
    if arguments.limit == 0 {
        return output.failure("gaps", exit::USAGE, "--limit must be at least 1");
    }
    let kind = match &arguments.kind {
        None => None,
        Some(name) => match name.parse::<nerve_core::EntityKind>() {
            Ok(kind) if kind.is_symbol() => Some(kind),
            _ => {
                return output.failure(
                    "gaps",
                    exit::USAGE,
                    &format!(
                        "unknown --kind {name:?}; a coverage gap is about a symbol, so expected \
                         one of: {}",
                        symbol_kind_names()
                    ),
                )
            }
        },
    };

    let OpenIndex { root, conn, .. } = match open_existing(path) {
        Ok(opened) => opened,
        Err(message) => return output.failure("gaps", exit::NO_INDEX, &message),
    };
    // Freshness is computed by re-reading the repository, so the reader is built from the
    // repository root and enforces the Slice 1 path rules on every path the database supplies.
    let prober = match nerve_index::RepositoryProber::new(&root) {
        Ok(prober) => prober,
        Err(err) => return output.failure("gaps", error_exit_code(&err), &err.to_string()),
    };

    let query = nerve_store::GapQuery {
        path_prefix: arguments.under.clone(),
        kind,
        include_partial: arguments.include_partial,
        limit: arguments.limit,
    };
    let report = match nerve_store::gaps(&conn, &query, &prober) {
        Ok(report) => report,
        Err(err) => return output.failure("gaps", exit::INTERNAL, &err.to_string()),
    };

    if report.coverage.is_answerable() {
        output.line(format!("Coverage gaps in {}", root.display()));
        output.line(format!(
            "  coverage       present · {} run(s) · {} file(s) re-hashed",
            report.runs.len(),
            report.files_probed
        ));
    } else {
        output.line(format!("No coverage evidence in {}", root.display()));
        output.line("  coverage       absent · the gap question is unanswerable here");
    }
    for run in &report.runs {
        output.line(format!(
            "    {:<28} {:<12} {} source file(s) in report",
            run.report_path.as_deref().unwrap_or("(no occurrence)"),
            run.freshness
                .map(|freshness| freshness.as_str())
                .unwrap_or("-"),
            run.source_files_in_report
                .map(|count| count.to_string())
                .unwrap_or_else(|| "?".to_string())
        ));
    }
    output.line(format!(
        "  symbols        {} in scope",
        report.symbols_in_scope
    ));
    if let Some(under) = &arguments.under {
        output.line(format!("  under          {under}"));
    }
    if let Some(kind) = kind {
        output.line(format!("  kind           {}", kind.as_str()));
    }

    match &report.totals {
        Some(totals) => {
            output.line(format!("  covered        {}", totals.covered));
            // `partial` is a recorded value and is never rounded into either neighbour.
            output.line(format!("  partial        {}", totals.partial));
            output.line(format!(
                "  uncovered      {}  (a run measured the file; no line in the symbol ran)",
                totals.uncovered
            ));
            output.line(format!(
                "  unmeasured     {}  (no coverage evidence names the file at all)",
                totals.unmeasured
            ));
            output.line(format!("  gaps           {}", totals.gaps()));
            output.line(format!(
                "  stale          {} symbol(s) over {} file(s) whose bytes have moved since \
                 the coverage was taken",
                totals.stale, totals.stale_files
            ));
            output.line(format!(
                "  measured       {} file(s) some coverage observation names",
                totals.measured_files
            ));
            output.line(String::new());

            if report.results.is_empty() {
                output.line("Every symbol in scope is covered by the ingested coverage.");
            }
            for row in &report.results {
                let lines = match (row.covered_lines, row.instrumented_lines) {
                    (Some(covered), Some(instrumented)) => {
                        format!("{covered}/{instrumented} lines")
                    }
                    _ => "-".to_string(),
                };
                output.line(format!(
                    "{:<11} {:<10} {:<34} {:<24} {:<12} {}",
                    row.state.as_str(),
                    row.entity.kind,
                    row.entity.qualified_name(),
                    row.entity.location(),
                    row.coverage_freshness
                        .map(|freshness| freshness.as_str())
                        .unwrap_or("-"),
                    lines
                ));
            }
            if report.truncated {
                output.line(String::new());
                output.line(format!(
                    "Listed {} of {} matching symbol(s); raise --limit to see the rest. The \
                     tallies above are exact.",
                    report.results.len(),
                    report.results_total
                ));
            }
        }
        None => {
            output.line(String::new());
            output.line(format!(
                "  {} symbol(s) are in scope and nothing is known about any of them.",
                report.symbols_in_scope
            ));
            output.line("  This is not \"no test covers your code\" — it is \"Nerve has not been");
            output.line("  told what your tests cover\". Those are different answers, and listing");
            output.line("  every symbol as a gap would report the second as the first.");
            output.line("  Run `nerve coverage <lcov-report>` first, then ask again.");
        }
    }

    output.object(json!({
        "command": "gaps",
        "ok": true,
        "exit_code": exit::SUCCESS,
        "root": root.display().to_string(),
        "coverage": report.coverage.as_str(),
        "answerable": report.coverage.is_answerable(),
        "under": arguments.under,
        "kind": kind.map(|kind| kind.as_str()),
        "include_partial": query.include_partial,
        "limit": report.limit,
        "runs": report.runs.iter().map(coverage_run_json).collect::<Vec<_>>(),
        "symbols_in_scope": report.symbols_in_scope,
        "totals": report.totals.as_ref().map(|totals| json!({
            "covered": totals.covered,
            "partial": totals.partial,
            "uncovered": totals.uncovered,
            "unmeasured": totals.unmeasured,
            "gaps": totals.gaps(),
            "stale": totals.stale,
            "measured_files": totals.measured_files,
            "stale_files": totals.stale_files,
        })),
        "count": report.results.len(),
        "results_total": report.results_total,
        "truncated": report.truncated,
        "files_probed": report.files_probed,
        "results": report.results.iter().map(|row| json!({
            "entity": entity_json(&row.entity),
            "state": row.state.as_str(),
            "coverage_freshness": row.coverage_freshness.map(|freshness| freshness.as_str()),
            "covered_lines": row.covered_lines,
            "instrumented_lines": row.instrumented_lines,
            "covered_by": row.covered_by,
        })).collect::<Vec<_>>(),
    }));

    exit::SUCCESS
}

// ---- history -------------------------------------------------------------------------------
//
// Three commands over schema v6. `sync` calls `nerve_index::ingest_history`, which is the only code
// here that opens `.git`; `log` and `file` ask `nerve_store::history` and open the database
// `query_only`, so a later edit cannot quietly make a read command write.
//
// Every handler below has one job beyond rendering: keep four kinds of silence apart. "Never
// synced" is not "synced and found nothing", a shallow boundary is not a root commit, Nerve's own
// commit budget is not the repository's boundary, and a commit with no change rows is one of four
// stated facts rather than a row count. Collapsing any of them produces an answer that looks
// reasonable and is wrong, which is what `docs/plans/slice-12b-historical-model.md` §5 and §6.1
// exist to prevent.

/// Render a repository-supplied string as inert single-line text.
///
/// A commit summary is the first free-form repository **prose** Nerve stores (plan §8.7). It is
/// attacker-influencable in any repository that accepts contributions, and this is a line-oriented
/// terminal surface, so two characters in it are dangerous for structural rather than semantic
/// reasons: a newline forges a second output line — which can be shaped to read
/// `  shallow        false` — and `ESC` begins an ANSI sequence, which can repaint or erase what
/// Nerve printed above it. Either turns repository data into something that looks like Nerve's own
/// answer.
///
/// So every control character becomes a visible `\u{..}` escape, and **the same escaped string is
/// what `--json` carries.** Emitting the raw byte into JSON would be valid JSON and would still
/// hand the problem to the next program — `jq -r` prints the value straight back to a terminal —
/// and one rendering is also the only way a terminal and a UI cannot disagree about what a commit
/// says.
///
/// What this deliberately does **not** do is neutralise meaning. `<script>` stays `<script>` and an
/// instruction-shaped sentence stays an instruction-shaped sentence: they are data, they are
/// labelled as the repository's, and HTML escaping belongs to whatever renders HTML. Bidirectional
/// format characters are also left alone, because they cannot forge a line or a control sequence
/// and escaping them would corrupt legitimate right-to-left prose; a *renderer* must still treat
/// them as hostile.
fn inert_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        if character.is_control() {
            out.push_str(&format!("\\u{{{:02x}}}", character as u32));
        } else {
            out.push(character);
        }
    }
    out
}

/// Open the index read-only for a history query.
///
/// `PRAGMA query_only` is the whole "a read command never writes" guarantee, made by construction
/// rather than by discipline: SQLite refuses the write, so the byte-identical-database test cannot
/// be broken by a later edit that reaches for a convenient repair.
fn open_query_only(path: &Path) -> Result<OpenIndex, String> {
    let opened = open_existing(path)?;
    opened
        .conn
        .pragma_update(None, "query_only", "ON")
        .map_err(|err| err.to_string())?;
    Ok(opened)
}

// The four wording functions and the one interpretation predicate that used to live here were
// hoisted in Slice 12c-i. The notes are now inherent methods on the vocabularies they render —
// `WalkTermination::note`, `ParentCompleteness::note`, `ChangesEnumerated::note`,
// `RenameAmbiguity::note` — so `ParentCompleteness::note` sits beside
// `may_claim_history_begins_here`, the rule and its rendering in one place. The judgment
// `earlier_changes_may_exist` moved to `nerve-store` beside `IngestRow`, because it takes one and
// `nerve-core` does not depend on `nerve-store`.
//
// This surface keeps **formatting only**. `crates/nerve-cli/tests/history_wording.rs` scans this
// crate and `nerve-server` for the note prose and fails if a copy returns, because a surface that
// re-words `shallow_boundary` slightly is a surface that has restated the invariant Slice 12b exists
// to protect.

/// Report a refused history argument as a refusal, with nothing answered.
///
/// The exit code is the one a refused selector already uses. No history question is asked, no
/// availability block is assembled and no path is suggested: every field a successful answer would
/// carry is absent, so a consumer cannot mistake this for a path with no history.
fn refuse_history_path(
    output: &Output,
    command: &'static str,
    argument: &str,
    refusal: nerve_store::HistoryPathRefusal,
) -> i32 {
    output.failure_detail(
        command,
        exit::USAGE,
        &format!(
            "{:?} is a symbol selector; {command} takes a path — {}",
            inert_text(argument),
            refusal.statement()
        ),
        &[
            "  nothing was looked up; this is a refusal, not an absence".to_string(),
            "  the path this selector names is not guessed at, because answering with it would be \
             the wrong claim rather than a narrower one"
                .to_string(),
        ],
        json!({
            "argument": inert_text(argument),
            "reason": refusal.as_str(),
            "reason_statement": refusal.statement(),
            "path_guessed": false,
        }),
    )
}

/// The refusal map, printed by form with its counts. Never summarised into one number alone.
fn print_refusals(output: &Output, refusals: &std::collections::BTreeMap<String, usize>) {
    output.line(format!(
        "  refused        {} in total",
        refusals.values().sum::<usize>()
    ));
    for (form, count) in refusals {
        output.line(format!("    {form:<32} {count}"));
    }
}

/// The one-line shallow verdict, with the boundary oids beneath it.
fn print_shallow(output: &Output, shallow: bool, boundary: &[String]) {
    if shallow {
        output.line(
            "  shallow        true — history before this point is unavailable to this repository",
        );
        for oid in boundary {
            output.line(format!(
                "    boundary     {oid} — earliest commit visible in this checkout"
            ));
        }
    } else {
        output.line("  shallow        false — this repository declares no shallow boundary");
    }
}

fn ingest_json(ingest: &nerve_store::IngestRow) -> serde_json::Value {
    json!({
        "head_oid": ingest.head_oid,
        "walked_from": ingest.walked_from,
        "commits_recorded": ingest.commits_recorded,
        "commit_budget": ingest.commit_budget,
        "walk_terminated_by": ingest.walk_terminated_by.as_str(),
        "walk_terminated_note": ingest.walk_terminated_by.note(),
        "shallow": ingest.shallow,
        "shallow_boundary": ingest.shallow_boundary,
        "promisor": ingest.promisor,
        "refusals": ingest.refusals,
        "refusals_total": ingest.refusals.values().sum::<usize>(),
        "reader_version": ingest.reader_version,
        "earlier_changes_may_exist": earlier_changes_may_exist(ingest),
    })
}

fn history_totals_json(totals: &nerve_store::HistoryTotals) -> serde_json::Value {
    json!({
        "commits": totals.commits,
        "changes": totals.changes,
        "renames": totals.renames,
        "merges": totals.merges,
        "changes_by_kind": totals.changes_by_kind.iter()
            .map(|(kind, count)| (kind.as_str().to_string(), *count))
            .collect::<std::collections::BTreeMap<String, i64>>(),
    })
}

/// One commit, with the count of its change rows beside the column that qualifies that count.
///
/// `changes` is `null` where the caller counted rows *for one path* rather than for the commit —
/// see [`print_commit`]. `null` is not `0`: the count was not taken, and a consumer that read an
/// absent count as an empty commit would make exactly the mistake `changes_enumerated` exists to
/// stop.
fn commit_json(commit: &nerve_store::CommitRow, changes: Option<usize>) -> serde_json::Value {
    let summary = inert_text(&commit.summary);
    json!({
        "commit_oid": commit.commit_oid,
        "tree_oid": commit.tree_oid,
        "parent_oids": commit.parent_oids,
        "is_merge": commit.is_merge,
        "parent_completeness": commit.parent_completeness.as_str(),
        "parent_completeness_note": commit.parent_completeness.note(),
        // Carried rather than left to the consumer: a UI that re-derived this from the string
        // would be the second copy of the rule, and the one likeliest to say "history begins here"
        // about a shallow boundary.
        "may_claim_history_begins_here":
            commit.parent_completeness.may_claim_history_begins_here(),
        "changes_enumerated": commit.changes_enumerated.as_str(),
        "changes_enumerated_note": commit.changes_enumerated.note(),
        "changes": changes,
        "author_time": commit.author_time,
        "author_tz": commit.author_tz,
        "committer_time": commit.committer_time,
        "committer_tz": commit.committer_tz,
        "author_ident": commit.author_ident.as_deref().map(inert_text),
        "committer_ident": commit.committer_ident.as_deref().map(inert_text),
        "summary": summary,
        "summary_escaped": summary != commit.summary,
        // Beside the summary and never apart from it. The repository-level tally in the ingest
        // record says *some* summary was cut; only this says whether **this** one was, and from the
        // text alone a reader cannot tell a short first line from a cut one — a first line of
        // exactly the bound is `complete`, so length cannot recover the answer.
        "summary_truncation": commit.summary_truncation.as_str(),
        "summary_truncation_note": commit.summary_truncation.note(),
    })
}

fn change_json(change: &nerve_store::ChangeRow) -> serde_json::Value {
    json!({
        "path": change.path,
        "change_kind": change.change_kind.as_str(),
        "blob_oid": change.blob_oid,
        "prev_blob_oid": change.prev_blob_oid,
        "mode": change.mode,
        "prev_mode": change.prev_mode,
    })
}

/// One commit's similarity analysis, as the surface carries it beside a hypothesis.
///
/// Everything a measurement is meaningless without: the method, its version, the **threshold** the
/// measurement was admitted against, and how much of the candidate set was measured at all. The
/// threshold is stored per run rather than assumed from a constant, so a row measured under an
/// older threshold still renders the number it was actually judged by.
fn rename_analysis_json(analysis: &nerve_store::AnalysisRow) -> serde_json::Value {
    json!({
        "commit_oid": analysis.commit_oid,
        "matcher_id": analysis.matcher_id,
        "matcher_version": analysis.matcher_version,
        // Two integers, like the measurement itself. A ratio of two thresholds is comparable and
        // rounds; `7 of 8` says what was required.
        "threshold_numerator": analysis.threshold_numerator,
        "threshold_denominator": analysis.threshold_denominator,
        "deletions_considered": analysis.deletions_considered,
        "additions_considered": analysis.additions_considered,
        "pairs_considered": analysis.pairs_considered,
        "pairs_measured": analysis.pairs_measured,
        "completeness": analysis.completeness.as_str(),
        "completeness_note": analysis.completeness.note(),
        "unmeasured": analysis.unmeasured.iter()
            .map(|(reason, count)| (reason.as_str().to_string(), *count))
            .collect::<std::collections::BTreeMap<String, i64>>(),
        // The reasons in words, because "blob-binary 1" beside a hypothesis reads as a defect and
        // is not one: an unmeasured pair is an unanswered question, never a negative answer.
        "unmeasured_notes": analysis.unmeasured.keys()
            .map(|reason| (reason.as_str().to_string(), reason.note().to_string()))
            .collect::<std::collections::BTreeMap<String, String>>(),
    })
}

/// A hypothesis, and every field without which its measurement would be a number from nowhere.
///
/// `analysis` is the per-commit candidate-set record, joined on rather than re-derived. `None` is
/// **not** rendered as an empty object: `analysis_absent_note` says which of the two cases it is,
/// in `nerve-store`'s words, because a blank here would be read as "the set was complete".
fn rename_json(
    rename: &nerve_store::RenameRow,
    analysis: Option<&nerve_store::AnalysisRow>,
) -> serde_json::Value {
    json!({
        "commit_oid": rename.commit_oid,
        "from_path": rename.from_path,
        "to_path": rename.to_path,
        "evidence": rename.evidence.as_str(),
        "evidence_note": rename.evidence.note(),
        // Two blob oids since schema v7, because a similarity pair has two. For an exact-content
        // hypothesis they are equal, and that identity is the evidence rather than a redundancy.
        "from_blob_oid": rename.from_blob_oid,
        "to_blob_oid": rename.to_blob_oid,
        // The producer of the row, and its measurement as two integers rather than one float. Both
        // measurement fields are null on an exact match, which computes no similarity at all.
        "matcher_id": rename.matcher_id,
        "matcher_version": rename.matcher_version,
        "match_numerator": rename.match_numerator,
        "match_denominator": rename.match_denominator,
        "ambiguity": rename.ambiguity.as_str(),
        "ambiguity_note": rename.ambiguity.note(),
        // Stated on every row rather than in a footnote a consumer can drop. Git records no rename;
        // this is a proposal drawn from content, and there is no score to sort it by.
        "is_hypothesis": true,
        "is_confirmed_rename": false,
        "analysis": analysis.map(rename_analysis_json),
        "analysis_absent_note": match analysis {
            Some(_) => serde_json::Value::Null,
            None => json!(nerve_store::rename_analysis_absence(rename.evidence)),
        },
    })
}

// ---- the derived questions (Slice 12c-i) -----------------------------------------------------
//
// Five store queries reach the terminal here, and the rendering below carries one obligation the
// counts do not: **only `may_claim_created` licenses the word "created"**, and the licence is read
// off the response rather than re-derived. `FirstObservedKind::may_claim_created` in `nerve-core`
// is the only copy of that rule in the workspace; `nerve_store::first_last_observed` carries its
// answer on the response for exactly this reason.
//
// Slice 12c-i left the prose for `FirstObservedKind`, `HistoryFreshness` and
// `EarlierHistoryUnavailable` in this file, because it was forbidden from editing `nerve-core`, and
// recorded it as a duplication waiting to happen. Slice 12c-iii-a performed that hoist before the
// HTTP surface could become the second copy: `FirstObservedKind::note`,
// `FirstObservedKind::created_claim_note` and `HistoryFreshness::note` are in `nerve-core`, and
// `EarlierHistoryUnavailable::note` is in `nerve-store` beside its enum. What is left here is
// formatting.

/// One end of the visible range, as one line.
fn path_change_line(change: &nerve_store::PathChange) -> String {
    format!(
        "{} · {} · committed {} {}",
        change.commit.commit_oid,
        change.change.change_kind.as_str(),
        change.commit.committer_time,
        change.commit.committer_tz
    )
}

fn path_change_json(change: &nerve_store::PathChange) -> serde_json::Value {
    json!({
        "commit": commit_json(&change.commit, None),
        "change": change_json(&change.change),
    })
}

fn first_last_observed_json(observed: &nerve_store::FirstLastObserved) -> serde_json::Value {
    json!({
        "path": inert_text(&observed.path),
        "kind": observed.kind.as_str(),
        "kind_note": observed.kind.note(),
        // Carried, never re-derived. `FirstObservedKind::may_claim_created` is the only copy of
        // this permission, exactly as `may_claim_history_begins_here` is for a commit.
        "may_claim_created": observed.may_claim_created,
        "may_claim_created_note": observed.kind.created_claim_note(),
        "first": observed.first.as_ref().map(path_change_json),
        "last": observed.last.as_ref().map(path_change_json),
        "changes_in_visible_history": observed.changes_in_visible_history,
        "additions_recorded": observed.additions_recorded,
        "merges_in_repository": observed.merges_in_repository,
        "earlier_history_unavailable":
            observed.earlier_history_unavailable.map(|reason| reason.as_str()),
        "earlier_history_unavailable_note":
            observed.earlier_history_unavailable.map(|reason| reason.note()),
        // The repository-level question, beside the path-level one above. They are different
        // scopes and a consumer that collapsed them would deny a creation the object graph proves.
        "earlier_changes_may_exist": observed.earlier_changes_may_exist,
        "walk_terminated_by": observed.walk_terminated_by.map(|value| value.as_str()),
        "walk_terminated_note": observed.walk_terminated_by.map(|value| value.note()),
        "shallow": observed.shallow,
        "current_tree": {
            "basis": observed.current_tree.basis,
            "index_exists": observed.current_tree.index_exists,
            "entities_at_path": observed.current_tree.entities_at_path,
        },
    })
}

/// One commit's similarity candidate-set record, printed the same way wherever it appears.
///
/// It appears in two places for one reason: a hypothesis needs the threshold it was admitted
/// against, and a commit with **no** hypothesis needs to say whether that silence is "nothing
/// moved" or "nothing could be measured". The two callers differ only in what they say when there
/// is no record at all, which is why `absent` is a parameter and never a sentence written here.
fn print_rename_analysis(
    output: &Output,
    analysis: Option<&nerve_store::AnalysisRow>,
    absent: &str,
) {
    let Some(analysis) = analysis else {
        output.line(format!("  candidates     {absent}"));
        return;
    };
    output.line(format!(
        "  threshold      {} of {} — admitted when numerator × {} ≥ {} × denominator",
        analysis.threshold_numerator,
        analysis.threshold_denominator,
        analysis.threshold_denominator,
        analysis.threshold_numerator
    ));
    output.line(format!(
        "  candidates     {} — {} pair(s) considered, {} measured, from {} deletion(s) × {} \
         addition(s)",
        analysis.completeness.as_str(),
        analysis.pairs_considered,
        analysis.pairs_measured,
        analysis.deletions_considered,
        analysis.additions_considered
    ));
    output.line(format!("                 {}", analysis.completeness.note()));
    // Each reason, in its own words. "blob-binary 1" beside a hypothesis reads as a defect and is
    // not one: an unmeasured pair is an unanswered question, never a negative answer.
    for (reason, count) in &analysis.unmeasured {
        output.line(format!(
            "  unmeasured     {} × {count} — {}",
            reason.as_str(),
            reason.note()
        ));
    }
}

/// Print the first/last-observed block.
///
/// Printed only where the header said the repository is answerable: with no ingest the header's
/// four lines are the whole answer, and a second block restating them in different words is the
/// drift this slice exists to remove. `--json` carries the block either way, with
/// `kind: "no_history_ingested"`.
fn print_first_observed(output: &Output, observed: &nerve_store::FirstLastObserved) {
    output.line(String::new());
    output.line(format!(
        "  {:<14} {} — {}",
        "first_seen",
        observed.kind.as_str(),
        observed.kind.note()
    ));
    output.line(format!(
        "  {:<14} {}",
        "created_claim",
        observed.kind.created_claim_note()
    ));
    output.line(format!(
        "  {:<14} {}",
        "first_change",
        observed
            .first
            .as_ref()
            .map(path_change_line)
            .unwrap_or_else(|| "(no change to this path is recorded)".to_string())
    ));
    output.line(format!(
        "  {:<14} {}",
        "last_change",
        observed
            .last
            .as_ref()
            .map(path_change_line)
            .unwrap_or_else(|| "(no change to this path is recorded)".to_string())
    ));
    output.line(format!(
        "  {:<14} {} commit(s) in visible history · {} addition(s) recorded",
        "observed_in", observed.changes_in_visible_history, observed.additions_recorded
    ));
    output.line(format!(
        "  {:<14} {} recorded in this repository — a merge enumerates no changes, so a creation \
         and a deletion inside merges are both unrecorded",
        "merges", observed.merges_in_repository
    ));
    output.line(match observed.earlier_history_unavailable {
        Some(reason) => format!(
            "  {:<14} {} — {}",
            "above_this",
            reason.as_str(),
            reason.note()
        ),
        None => format!(
            "  {:<14} nothing above what Nerve read of this path is hidden, and that is measured \
             rather than assumed",
            "above_this"
        ),
    });
    output.line(format!(
        "  {:<14} {} — this is the repository's ingest, a different scope from the line above",
        "earlier_repo", observed.earlier_changes_may_exist
    ));
    if let Some(terminated) = observed.walk_terminated_by {
        output.line(format!(
            "  {:<14} {} — {}",
            "walk_ended",
            terminated.as_str(),
            terminated.note()
        ));
    }
    output.line(format!("  {:<14} {}", "shallow_repo", observed.shallow));
    output.line(format!(
        "  {:<14} {} · index_exists {} · {} entity row(s) at this path",
        "current_tree",
        observed.current_tree.basis,
        observed.current_tree.index_exists,
        observed.current_tree.entities_at_path
    ));
}

/// Print one commit as a labelled block, with every state that qualifies it.
///
/// A block rather than a table row because the summary is repository prose bounded at 512 bytes:
/// in a column it would wrap and hide the fields beside it. The summary is printed **last** so
/// nothing Nerve says about the commit can be pushed off the line by it.
///
/// `changes` is `None` where the caller has no *whole-commit* count to show — `nerve history file`
/// holds one change row per commit because that is what it asked for, and printing `1` there would
/// state that the commit changed one path. The qualifying column is printed either way, because a
/// count is only readable next to it and its absence must not be readable as zero.
fn print_commit(output: &Output, commit: &nerve_store::CommitRow, changes: Option<usize>) {
    output.line(String::new());
    output.line(format!("commit {}", commit.commit_oid));
    output.line(format!(
        "  committed      {} {}",
        commit.committer_time, commit.committer_tz
    ));
    output.line(format!(
        "  authored       {} {}",
        commit.author_time, commit.author_tz
    ));
    output.line(format!(
        "  parents        {}",
        if commit.parent_oids.is_empty() {
            "(none in the commit object)".to_string()
        } else {
            commit.parent_oids.join(" ")
        }
    ));
    output.line(format!(
        "  availability   {} — {}",
        commit.parent_completeness.as_str(),
        commit.parent_completeness.note()
    ));
    match changes {
        Some(count) => output.line(format!(
            "  changes        {count} row(s) · {}",
            commit.changes_enumerated.as_str()
        )),
        None => output.line(format!(
            "  enumeration    {}",
            commit.changes_enumerated.as_str()
        )),
    }
    output.line(format!(
        "                 {}",
        commit.changes_enumerated.note()
    ));
    if let Some(ident) = &commit.author_ident {
        output.line(format!("  author         {}", inert_text(ident)));
    }
    if let Some(ident) = &commit.committer_ident {
        output.line(format!("  committer      {}", inert_text(ident)));
    }
    output.line(format!("  summary        {}", inert_text(&commit.summary)));
    // On the line under the summary, always — including `complete`. A flag printed only when a
    // summary was cut would make its absence the claim "nothing was cut", and `unknown` — what
    // every commit recorded before schema v7 carries — would then read as `complete`.
    output.line(format!(
        "  summary_state  {} — {}",
        commit.summary_truncation.as_str(),
        commit.summary_truncation.note()
    ));
}

/// Walk `.git` and record the commit graph.
///
/// The clamp is a **refusal**, not a silent correction: a caller that asked for more commits than
/// Nerve will ever walk in one sync must learn that its number was not the one used, because the
/// alternative is a bounded ingest that reads as the number the caller chose.
fn run_history_sync(
    output: &Output,
    path: Option<PathBuf>,
    max_commits: usize,
    with_identity: bool,
) -> i32 {
    if max_commits > nerve_index::MAX_HISTORY_COMMITS {
        return output.failure(
            "history sync",
            exit::USAGE,
            &format!(
                "--max-commits {max_commits} is above the clamp of {}; ask for at most {} \
                 commit(s). Honouring it silently would report a bounded read as the read you \
                 asked for.",
                nerve_index::MAX_HISTORY_COMMITS,
                nerve_index::MAX_HISTORY_COMMITS
            ),
        );
    }

    let root = path.unwrap_or_else(|| PathBuf::from("."));
    // Similarity limits are the shipped defaults here. They are a field on `HistoryOptions` so a
    // test can tighten them and observe a bound refusal end to end; exposing them as flags is a
    // presentation decision that belongs with the rest of 12c-ii's surface work.
    let options = nerve_index::HistoryOptions {
        max_commits,
        with_identity,
        ..nerve_index::HistoryOptions::default()
    };
    match nerve_index::ingest_history(&root, &options) {
        Ok(outcome) => {
            let refused_total = outcome.refused.values().sum::<usize>();
            // A budget-bounded walk is counted as a refusal by the ingester, so it lands here as a
            // partial read — which is what it is. `nerve coverage` and `nerve trace import` use the
            // same code for the same reason.
            let code = if outcome.status == nerve_index::RunStatus::Partial {
                exit::PARTIAL_INDEX
            } else {
                exit::SUCCESS
            };

            output.line(format!("Read history from {}", outcome.root.display()));
            output.line(format!("  git_dir        {}", outcome.git_dir.display()));
            output.line(format!(
                "  head           {}",
                outcome.head_oid.as_deref().unwrap_or(
                    "(unborn branch — HEAD names a ref that does not exist yet; this is a \
                     success, not an error)"
                )
            ));
            output.line(format!(
                "  budget         {} commit(s)",
                outcome.commit_budget
            ));
            output.line(format!(
                "  walked         {} examined · {} newly recorded · {} already recorded · {} \
                 re-examined",
                outcome.commits_walked,
                outcome.commits_recorded,
                outcome.commits_already_present,
                outcome.commits_repaired
            ));
            output.line(format!(
                "  changes        {} row(s)",
                outcome.changes_written
            ));
            // Two counts, on two lines, and **never a total**. They rest on different evidence:
            // one says *these two paths named the same bytes*, the other says *a named method
            // measured how much two different blobs share*. A single number would describe
            // neither, in the way the `py-framework` and `ts-js-framework` precision tables are
            // never summed.
            output.line(format!(
                "  renames        {} exact-content hypothesis(es)",
                outcome.renames_written
            ));
            output.line(format!(
                "                 {} similar-content hypothesis(es) — a different kind of \
                 evidence, never added to the line above",
                outcome.similar_renames_written
            ));
            output.line(format!(
                "  analyses       {} commit candidate-set record(s), one per newly recorded \
                 commit, including commits with no candidate pair at all",
                outcome.rename_analyses_written
            ));
            output.line(format!(
                "  stopped        {} — {}",
                outcome.walk_terminated_by.as_str(),
                outcome.walk_terminated_by.note()
            ));
            print_shallow(output, outcome.shallow, &outcome.shallow_boundary);
            output.line(format!("  promisor       {}", outcome.promisor));
            output.line(format!(
                "  availability   {}",
                tally_line(&outcome.completeness)
            ));
            output.line(format!(
                "  enumeration    {}",
                tally_line(&outcome.enumeration)
            ));
            output.line(format!(
                "  trees          {} read · {} subtree(s) skipped unread",
                outcome.trees_read, outcome.subtrees_skipped
            ));
            output.line(format!(
                "  summaries      {} truncated at {} bytes",
                outcome.summaries_truncated,
                nerve_index::MAX_SUMMARY_BYTES
            ));
            if with_identity {
                output.line(
                    "  identity       stored — author and committer names and email addresses are \
                     now in the index, as untrusted third-party strings",
                );
            } else {
                output.line(
                    "  identity       not stored — no question this surface answers asks who, so \
                     contributor names and",
                );
                output.line(
                    "                 email addresses are kept out of the index; times and \
                     timezones are always stored",
                );
            }
            print_refusals(output, &outcome.refused);
            output.line(format!("  duration_ms    {}", outcome.duration_ms));

            output.object(json!({
                "command": "history sync",
                "ok": true,
                "exit_code": code,
                "root": outcome.root.display().to_string(),
                "git_dir": outcome.git_dir.display().to_string(),
                "head_oid": outcome.head_oid,
                "walked_from": outcome.walked_from,
                "commit_budget": outcome.commit_budget,
                "commits_walked": outcome.commits_walked,
                "commits_recorded": outcome.commits_recorded,
                "commits_already_present": outcome.commits_already_present,
                "commits_repaired": outcome.commits_repaired,
                "changes_written": outcome.changes_written,
                // Three fields, never a fourth that adds two of them together. `renames_written`
                // counts the exact-content matcher's rows and keeps its Slice 12b name and meaning.
                "renames_written": outcome.renames_written,
                "similar_renames_written": outcome.similar_renames_written,
                "rename_analyses_written": outcome.rename_analyses_written,
                "rename_matchers": {
                    "exact": nerve_index::history::EXACT_MATCHER_ID,
                    "exact_version": nerve_index::history::EXACT_MATCHER_VERSION,
                    "similarity": nerve_index::similarity::MATCHER_ID,
                    "similarity_version": nerve_index::similarity::MATCHER_VERSION,
                },
                "similarity_threshold_numerator":
                    nerve_index::similarity::SIMILARITY_THRESHOLD_NUMERATOR,
                "similarity_threshold_denominator":
                    nerve_index::similarity::SIMILARITY_THRESHOLD_DENOMINATOR,
                "walk_terminated_by": outcome.walk_terminated_by.as_str(),
                "walk_terminated_note": outcome.walk_terminated_by.note(),
                "shallow": outcome.shallow,
                "shallow_boundary": outcome.shallow_boundary,
                "promisor": outcome.promisor,
                "with_identity": with_identity,
                "completeness": outcome.completeness.iter()
                    .map(|(value, count)| (value.as_str().to_string(), *count))
                    .collect::<std::collections::BTreeMap<String, usize>>(),
                "enumeration": outcome.enumeration.iter()
                    .map(|(value, count)| (value.as_str().to_string(), *count))
                    .collect::<std::collections::BTreeMap<String, usize>>(),
                "trees_read": outcome.trees_read,
                "subtrees_skipped": outcome.subtrees_skipped,
                "summaries_truncated": outcome.summaries_truncated,
                "max_summary_bytes": nerve_index::MAX_SUMMARY_BYTES,
                "refused": outcome.refused,
                "refused_total": refused_total,
                "reader_version": outcome.reader_version,
                "status": outcome.status.as_str(),
                "duration_ms": outcome.duration_ms,
                "reads_git_only": true,
            }));
            code
        }
        Err(err) => {
            // A directory with no Git repository in it is a wrong **argument**, not a crash and not
            // a missing index: there is no history there to read at all. It is deliberately not the
            // same answer as an unborn branch, which is a real repository whose HEAD names no
            // commit yet and which succeeds with `head_oid: null` — collapsing the two would report
            // "you pointed at the wrong directory" as "this project has no history".
            let message = match &err {
                IndexError::NotADirectory(missing)
                    if missing.file_name() == Some(std::ffi::OsStr::new(".git")) =>
                {
                    format!(
                        "no Git repository at {}: there is no history to read here. A repository \
                         whose HEAD names no commit yet is a different answer and succeeds with no \
                         commits.",
                        missing.display()
                    )
                }
                other => other.to_string(),
            };
            output.failure("history sync", error_exit_code(&err), &message)
        }
    }
}

/// Everything the two read commands need before they can answer.
struct HistoryRead {
    root: PathBuf,
    conn: nerve_store::Connection,
    repo_id: String,
    ingest: Option<nerve_store::IngestRow>,
    totals: Option<nerve_store::HistoryTotals>,
}

/// Open the index, find the repository, and read the ingest record.
///
/// `totals` is `Some` only when an ingest record exists, and that pairing is the point. A repository
/// that has never been synced has zero commits *and* zero of everything else, so reporting the
/// tallies would answer "this project has no history" to a question nobody asked — the
/// `nerve gaps` `totals: null` rule, in a second place.
fn open_history(output: &Output, command: &'static str, path: &Path) -> Result<HistoryRead, i32> {
    let OpenIndex { root, conn, .. } = open_query_only(path)
        .map_err(|message| output.failure(command, exit::NO_INDEX, &message))?;
    let repository = nerve_store::repository(&conn)
        .map_err(|err| output.failure(command, exit::INTERNAL, &err.to_string()))?;
    let repo_id = repository
        .ok_or_else(|| {
            output.failure(
                command,
                exit::NO_INDEX,
                &format!(
                    "no repository row at {}; run `nerve init` first",
                    root.display()
                ),
            )
        })?
        .repo_id;
    let ingest = nerve_store::history_ingest(&conn, &repo_id)
        .map_err(|err| output.failure(command, exit::INTERNAL, &err.to_string()))?;
    let totals = match &ingest {
        None => None,
        Some(_) => Some(
            nerve_store::history_totals(&conn, &repo_id)
                .map_err(|err| output.failure(command, exit::INTERNAL, &err.to_string()))?,
        ),
    };
    Ok(HistoryRead {
        root,
        conn,
        repo_id,
        ingest,
        totals,
    })
}

/// The header both read commands print, and the JSON both of them carry.
///
/// Returns `false` when there is no ingest record, in which case the caller has nothing to list:
/// **no history was ever read here**, which is not the same fact as a history with no commits in it
/// and must not print the same words.
fn print_history_header(output: &Output, read: &HistoryRead) -> bool {
    let Some(ingest) = &read.ingest else {
        output.line(format!("No history read in {}", read.root.display()));
        output.line("  history        never synced — `nerve history sync` has not been run here");
        output.line(String::new());
        output.line("  This is not \"this repository has no commits\". Nerve has not read its");
        output.line("  history at all, so it has nothing to say about it — and listing zero");
        output.line("  commits would report the second as the first. Run `nerve history sync`.");
        return false;
    };

    output.line(format!("History in {}", read.root.display()));
    let totals = read
        .totals
        .as_ref()
        .expect("totals accompany every ingest record");
    if totals.commits == 0 {
        // The other half of the distinction above, and the reason both halves are printed
        // explicitly: this repository *was* read and the walk found no commit to record.
        output.line(
            "  history        synced, and it recorded no commits — the walk ran and found none",
        );
    } else {
        output.line(format!(
            "  history        synced · reader {}",
            ingest.reader_version
        ));
    }
    output.line(format!(
        "  head           {}",
        ingest
            .head_oid
            .as_deref()
            .unwrap_or("(unborn branch at sync time — HEAD named a ref that did not exist yet)")
    ));
    output.line(format!(
        "  commits        {} recorded · {} change row(s) · {} rename hypothesis(es) · {} merge(s)",
        totals.commits, totals.changes, totals.renames, totals.merges
    ));
    output.line(format!(
        "  changes        {}",
        totals
            .changes_by_kind
            .iter()
            .map(|(kind, count)| format!("{kind} {count}"))
            .collect::<Vec<_>>()
            .join(" · ")
    ));
    output.line(format!(
        "  budget         {} commit(s)",
        ingest.commit_budget
    ));
    output.line(format!(
        "  stopped        {} — {}",
        ingest.walk_terminated_by.as_str(),
        ingest.walk_terminated_by.note()
    ));
    print_shallow(output, ingest.shallow, &ingest.shallow_boundary);
    output.line(format!("  promisor       {}", ingest.promisor));
    print_refusals(output, &ingest.refusals);
    if ingest
        .refusals
        .contains_key(nerve_index::history::form::SUMMARY_TRUNCATED)
    {
        output.line(format!(
            "                 a summary was cut at {} bytes. The flag is per repository, not per",
            nerve_index::MAX_SUMMARY_BYTES
        ));
        output.line(
            "                 commit, so a summary of exactly that length cannot be told from a \
             cut one",
        );
    }
    if earlier_changes_may_exist(ingest) {
        output.line(
            "  completeness   this ingest did not read the whole reachable history, so an earlier",
        );
        output.line("                 commit may exist and not be recorded below");
    }
    true
}

/// The JSON fields both read commands share.
fn history_context_json(read: &HistoryRead) -> serde_json::Map<String, serde_json::Value> {
    let mut object = serde_json::Map::new();
    object.insert("root".into(), json!(read.root.display().to_string()));
    // `false` means *nothing was ever read*, and both tallies are `null` in that case rather than
    // zero, so a script cannot read "never synced" as "no history".
    object.insert("answerable".into(), json!(read.ingest.is_some()));
    object.insert(
        "ingest".into(),
        read.ingest
            .as_ref()
            .map(ingest_json)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "totals".into(),
        read.totals
            .as_ref()
            .map(history_totals_json)
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "commits_total".into(),
        read.totals
            .as_ref()
            .map(|totals| json!(totals.commits))
            .unwrap_or(serde_json::Value::Null),
    );
    object.insert(
        "earlier_changes_may_exist".into(),
        read.ingest
            .as_ref()
            .map(|ingest| json!(earlier_changes_may_exist(ingest)))
            .unwrap_or(serde_json::Value::Null),
    );
    object
}

/// Emit one read command's answer, with the shared context merged into it.
fn history_object(
    output: &Output,
    command: &'static str,
    read: &HistoryRead,
    extra: serde_json::Value,
) {
    let mut object = serde_json::Map::new();
    object.insert("command".into(), json!(command));
    object.insert("ok".into(), json!(true));
    object.insert("exit_code".into(), json!(exit::SUCCESS));
    for (key, value) in history_context_json(read) {
        object.insert(key, value);
    }
    if let serde_json::Value::Object(fields) = extra {
        for (key, value) in fields {
            object.insert(key, value);
        }
    }
    output.object(serde_json::Value::Object(object));
}

fn run_history_log(output: &Output, path: &Path, limit: usize, offset: usize) -> i32 {
    if limit == 0 {
        return output.failure("history log", exit::USAGE, "--limit must be at least 1");
    }
    let read = match open_history(output, "history log", path) {
        Ok(read) => read,
        Err(code) => return code,
    };

    let commits = if read.ingest.is_none() {
        Vec::new()
    } else {
        match nerve_store::commit_log(&read.conn, &read.repo_id, limit, offset) {
            Ok(commits) => commits,
            Err(err) => return output.failure("history log", exit::INTERNAL, &err.to_string()),
        }
    };
    // The change *count* per commit, not the rows: `log` answers "what happened", and the count is
    // only readable next to `changes_enumerated`, which is why they are printed together.
    let mut counts = Vec::with_capacity(commits.len());
    for commit in &commits {
        match nerve_store::changes_for_commit(&read.conn, &read.repo_id, &commit.commit_oid) {
            Ok(changes) => counts.push(changes.len()),
            Err(err) => return output.failure("history log", exit::INTERNAL, &err.to_string()),
        }
    }

    let answerable = print_history_header(output, &read);
    let total = read.totals.as_ref().map(|totals| totals.commits);
    if answerable {
        // "20 of 95", never a bare "20": a reader who saw only the returned count would believe
        // that was all there was.
        output.line(format!(
            "  listed         {} of {} recorded commit(s), from offset {}",
            commits.len(),
            total.unwrap_or(0),
            offset
        ));
    }
    for (commit, changes) in commits.iter().zip(&counts) {
        print_commit(output, commit, Some(*changes));
    }

    history_object(
        output,
        "history log",
        &read,
        json!({
            "limit": limit,
            "offset": offset,
            "count": commits.len(),
            "truncated": total.unwrap_or(0) > i64::try_from(offset + commits.len()).unwrap_or(i64::MAX),
            "commits": commits.iter().zip(&counts)
                .map(|(commit, changes)| commit_json(commit, Some(*changes)))
                .collect::<Vec<_>>(),
        }),
    );
    exit::SUCCESS
}

fn run_history_file(output: &Output, path: &Path, tree_path: &str, limit: usize) -> i32 {
    if limit == 0 {
        return output.failure("history file", exit::USAGE, "--limit must be at least 1");
    }
    if let Some(refusal) = nerve_store::history_path_refusal(tree_path) {
        return refuse_history_path(output, "history file", tree_path, refusal);
    }
    let read = match open_history(output, "history file", path) {
        Ok(read) => read,
        Err(code) => return code,
    };

    // One row past the limit, then cut: this is the only way `truncated` can be a fact rather than
    // the guess "we got exactly as many as we asked for", which is false whenever the answer ends
    // on the boundary. There is no total for a path the way `commit_log` has one.
    let probe = limit.saturating_add(1);
    let (mut commits, mut renames) = if read.ingest.is_none() {
        (Vec::new(), Vec::new())
    } else {
        let commits =
            match nerve_store::commits_touching_path(&read.conn, &read.repo_id, tree_path, probe) {
                Ok(commits) => commits,
                Err(err) => {
                    return output.failure("history file", exit::INTERNAL, &err.to_string())
                }
            };
        let renames =
            match nerve_store::renames_touching_path(&read.conn, &read.repo_id, tree_path, probe) {
                Ok(renames) => renames,
                Err(err) => {
                    return output.failure("history file", exit::INTERNAL, &err.to_string())
                }
            };
        (commits, renames)
    };
    let commits_truncated = commits.len() > limit;
    let renames_truncated = renames.len() > limit;
    commits.truncate(limit);
    renames.truncate(limit);

    // The per-commit candidate-set record, joined onto the hypotheses above **and onto the commits
    // listed below**. Without it a similarity row is a ratio with no threshold and no statement of
    // whether the set it came from was measured in full — a percentage from nowhere, written as two
    // integers instead of one float.
    //
    // The listed commits are in the join for the case the hypotheses cannot cover: a commit whose
    // candidate set was refused by a bound, or whose pairs could not be measured, records **no**
    // similarity row, so there is no row for a per-row field to hang on. Reading that silence as
    // "nothing moved here" is the failure `RenameAnalysisCompleteness` exists to prevent, and it can
    // only be prevented where the commit itself carries its analysis.
    let mut analysis_oids: Vec<&str> = renames
        .iter()
        .map(|rename| rename.commit_oid.as_str())
        .chain(commits.iter().map(|(commit, _)| commit.commit_oid.as_str()))
        .collect();
    analysis_oids.sort_unstable();
    analysis_oids.dedup();
    let analyses = match nerve_store::rename_analysis_for_commits(
        &read.conn,
        &read.repo_id,
        &analysis_oids,
        nerve_index::similarity::MATCHER_ID,
    ) {
        Ok(analyses) => analyses,
        Err(err) => return output.failure("history file", exit::INTERNAL, &err.to_string()),
    };

    // Asked unconditionally, unlike the two lists above: with no ingest this answers
    // `no_history_ingested`, which is one of the four states §11 requires to stay distinct, and
    // short-circuiting it would collapse "never read" into "read, and nothing was found".
    let observed = match nerve_store::first_last_observed(&read.conn, &read.repo_id, tree_path) {
        Ok(observed) => observed,
        Err(err) => return output.failure("history file", exit::INTERNAL, &err.to_string()),
    };

    let answerable = print_history_header(output, &read);
    if answerable {
        output.line(format!(
            "  path           {} (matched as a tree recorded it, not resolved on disk)",
            inert_text(tree_path)
        ));
        output.line(format!(
            "  changed_in     {} commit(s){}, limit {limit}",
            commits.len(),
            if commits_truncated {
                " and more beyond the limit"
            } else {
                ""
            }
        ));
        output.line(format!(
            "  renames        {} hypothesis(es) naming this path{}",
            renames.len(),
            if renames_truncated {
                " and more beyond the limit"
            } else {
                ""
            }
        ));
        if commits.is_empty() && renames.is_empty() {
            output.line(String::new());
            output.line("  No recorded commit changed this path. That is an answer, not a");
            output.line("  failure: the path may never have existed under this spelling, or it");
            output.line("  may be recorded under another one — a tree records the bytes it was");
            output.line("  given, and this argument is matched against those bytes literally.");
            if read.ingest.as_ref().is_some_and(earlier_changes_may_exist) {
                output.line("  This ingest also did not read the whole reachable history, so a");
                output.line("  change to this path may exist in a commit it never saw.");
            }
        }
        print_first_observed(output, &observed);
    }

    for (commit, change) in &commits {
        print_commit(output, commit, None);
        output.line(format!(
            "  change         {} · blob {} ← {} · mode {} ← {}",
            change.change_kind.as_str(),
            change.blob_oid.as_deref().unwrap_or("(none — deleted)"),
            change.prev_blob_oid.as_deref().unwrap_or("(none — added)"),
            change
                .mode
                .map(|mode| format!("{mode:o}"))
                .unwrap_or_else(|| "-".to_string()),
            change
                .prev_mode
                .map(|mode| format!("{mode:o}"))
                .unwrap_or_else(|| "-".to_string()),
        ));
        // Beneath the change, because it qualifies the *absence* of a rename hypothesis for this
        // commit as much as the presence of one. A commit whose candidate set a bound refused
        // records no hypothesis, and without this line that empty result reads as "nothing moved".
        print_rename_analysis(
            output,
            analyses.get(&commit.commit_oid),
            nerve_store::COMMIT_NOT_ANALYSED,
        );
    }
    for rename in &renames {
        let analysis = nerve_store::analysis_of(&analyses, rename);
        output.line(String::new());
        // "Git recorded no rename" rather than "not a confirmed rename": a denial that contains
        // the phrase it denies is one careless `grep -q` away from being read as the claim, and
        // this line is the one sentence a reader takes the whole block's standing from.
        output.line("rename hypothesis — Git recorded no rename, and there is no score");
        output.line(format!("  commit         {}", rename.commit_oid));
        output.line(format!(
            "  from           {}",
            inert_text(&rename.from_path)
        ));
        output.line(format!("  to             {}", inert_text(&rename.to_path)));
        output.line(format!(
            "  evidence       {} · blob {} → {}",
            rename.evidence.as_str(),
            rename.from_blob_oid,
            rename.to_blob_oid
        ));
        output.line(format!("                 {}", rename.evidence.note()));
        // The producer, always. A measurement whose method is not named is not readable at all,
        // and an exact-content row names one too — it just measured nothing.
        output.line(format!(
            "  matcher        {} version {}",
            rename.matcher_id, rename.matcher_version
        ));
        // "18 of 20 line(s)", never "90%". Two integers a reader can check by hand, printed with
        // the threshold they were admitted against, because neither is readable without the other.
        match (rename.match_numerator, rename.match_denominator) {
            (Some(numerator), Some(denominator)) => {
                output.line(format!(
                    "  measurement    {numerator} of {denominator} line(s) shared, by the matcher \
                     above"
                ));
            }
            _ => output.line(
                "  measurement    none — this evidence computes no similarity, so there is no \
                 ratio and no perfect score either",
            ),
        }
        print_rename_analysis(
            output,
            analysis,
            nerve_store::rename_analysis_absence(rename.evidence),
        );
        output.line(format!(
            "  ambiguity      {} — {}",
            rename.ambiguity.as_str(),
            rename.ambiguity.note()
        ));
    }

    history_object(
        output,
        "history file",
        &read,
        json!({
            "path": inert_text(tree_path),
            "path_is_as_recorded_in_a_tree": true,
            "first_observed": first_last_observed_json(&observed),
            "limit": limit,
            "count": commits.len(),
            "truncated": commits_truncated,
            "commits": commits.iter().map(|(commit, change)| {
                let mut object = commit_json(commit, None);
                if let Some(fields) = object.as_object_mut() {
                    fields.insert("change".into(), change_json(change));
                    let analysis = analyses.get(&commit.commit_oid);
                    fields.insert(
                        "rename_analysis".into(),
                        analysis.map(rename_analysis_json).unwrap_or(serde_json::Value::Null),
                    );
                    fields.insert(
                        "rename_analysis_absent_note".into(),
                        match analysis {
                            Some(_) => serde_json::Value::Null,
                            None => json!(nerve_store::COMMIT_NOT_ANALYSED),
                        },
                    );
                }
                object
            }).collect::<Vec<_>>(),
            "renames_count": renames.len(),
            "renames_truncated": renames_truncated,
            "renames": renames.iter()
                .map(|rename| rename_json(rename, nerve_store::analysis_of(&analyses, rename)))
                .collect::<Vec<_>>(),
            "rename_analysis_matcher_id": nerve_index::similarity::MATCHER_ID,
        }),
    );
    exit::SUCCESS
}

/// What changed between two recorded states, by ancestry.
///
/// **Every outcome exits `0`, and four of the five are not diffs.** In `--json` the diff-shaped
/// fields are `null` rather than empty for those four, which is the property that stops a refusal
/// being read as "nothing changed": `commits: null` says no range was computed, and `commits: []`
/// says a range was computed and holds nothing.
fn run_history_diff(
    output: &Output,
    path: &Path,
    from: &str,
    to: &str,
    limits: nerve_store::StateDiffLimits,
) -> i32 {
    let limit = limits.commits;
    for (flag, bound) in [
        ("--limit", limits.commits),
        ("--max-walk", limits.commits_walked),
        ("--max-changes", limits.changes),
    ] {
        if bound == 0 {
            return output.failure(
                "history diff",
                exit::USAGE,
                &format!("{flag} must be at least 1"),
            );
        }
    }
    for (flag, oid) in [("--from", from), ("--to", to)] {
        if oid.trim().is_empty() {
            return output.failure(
                "history diff",
                exit::USAGE,
                &format!("{flag} needs a commit oid"),
            );
        }
    }
    let read = match open_history(output, "history diff", path) {
        Ok(read) => read,
        Err(code) => return code,
    };

    let diff = match nerve_store::state_diff(&read.conn, &read.repo_id, from, to, limits) {
        Ok(diff) => diff,
        Err(err) => return output.failure("history diff", exit::INTERNAL, &err.to_string()),
    };

    let answerable = print_history_header(output, &read);
    if answerable {
        output.line(format!("  {:<14} {from}", "from"));
        output.line(format!("  {:<14} {to}", "to"));
    }

    let detail = match &diff {
        nerve_store::StateDiff::StateNotRecorded {
            from,
            to,
            from_recorded,
            to_recorded,
        } => {
            if answerable {
                output.line(format!("  {:<14} state_not_recorded", "result"));
                for (label, oid, recorded) in
                    [("from", from, from_recorded), ("to", to, to_recorded)]
                {
                    output.line(format!(
                        "  {:<14} {oid} — {}",
                        format!("{label}_state"),
                        if *recorded {
                            "recorded"
                        } else {
                            "never recorded here, so nothing between the two states can be stated"
                        }
                    ));
                }
                output.line(String::new());
                output.line("  This is not an empty diff. Nerve never read the commit marked");
                output.line("  \"never recorded\", so it has no basis to say what lies between");
                output.line("  the two states — which is a different fact from nothing lying");
                output.line("  between them. `nerve history sync` may not have reached it.");
            }
            json!({
                "result": "state_not_recorded",
                "from_recorded": from_recorded,
                "to_recorded": to_recorded,
            })
        }
        nerve_store::StateDiff::NotAnAncestor {
            from,
            to,
            commits_walked,
        } => {
            if answerable {
                output.line(format!("  {:<14} not_an_ancestor", "result"));
                output.line(format!(
                    "  {:<14} {commits_walked} commit(s) before the ancestry ran out",
                    "walked"
                ));
                output.line(String::new());
                output.line(format!(
                    "  {from} is not an ancestor of {to}: the walk read every commit"
                ));
                output.line("  reachable from the newer state and never reached the older one,");
                output.line("  and nothing stopped it early. This is not an empty diff — there");
                output.line("  is no range between these two states to be empty.");
            }
            json!({ "result": "not_an_ancestor", "commits_walked": commits_walked })
        }
        nerve_store::StateDiff::AncestryIncomplete {
            from,
            to,
            stopped_at,
            parent_completeness,
            commits_walked,
        } => {
            if answerable {
                output.line(format!("  {:<14} ancestry_incomplete", "result"));
                output.line(format!(
                    "  {:<14} {stopped_at} — {} — {}",
                    "stopped_at",
                    parent_completeness.as_str(),
                    parent_completeness.note()
                ));
                output.line(format!(
                    "  {:<14} {commits_walked} commit(s) before stopping",
                    "walked"
                ));
                output.line(String::new());
                output.line(format!(
                    "  Whether {from} is an ancestor of {to} could not be"
                ));
                output.line("  decided: the walk reached a commit whose parents it could not");
                output.line("  follow. This is not an empty diff and it is not a \"no\" — the");
                output.line("  question was left undecided, and saying either would be a claim.");
            }
            json!({
                "result": "ancestry_incomplete",
                "stopped_at": stopped_at,
                "stopped_at_parent_completeness": parent_completeness.as_str(),
                "stopped_at_parent_completeness_note": parent_completeness.note(),
                "commits_walked": commits_walked,
            })
        }
        nerve_store::StateDiff::WalkBudgetExhausted {
            from,
            to,
            commits_walked,
            limit,
        } => {
            if answerable {
                output.line(format!("  {:<14} walk_budget_exhausted", "result"));
                output.line(format!(
                    "  {:<14} {commits_walked} commit(s) against a bound of {limit}",
                    "walked"
                ));
                output.line(String::new());
                output.line(format!(
                    "  Nerve's own walk bound stopped before {from} was reached from"
                ));
                output.line(format!(
                    "  {to}. That is Nerve's boundary and not the repository's,"
                ));
                output.line("  it is not an empty diff, and it is not \"not an ancestor\" —");
                output.line("  reporting either would state something never established.");
            }
            json!({
                "result": "walk_budget_exhausted",
                "commits_walked": commits_walked,
                "walk_limit": limit,
            })
        }
        nerve_store::StateDiff::Diff(report) => {
            if answerable {
                output.line(format!("  {:<14} diff", "result"));
                output.line(format!(
                    "  {:<14} {} listed of {} in range{}, limit {limit} · {} walked",
                    "commits",
                    report.commits.len(),
                    report.commits_in_range,
                    if report.commits_truncated {
                        " and more beyond the limit"
                    } else {
                        ""
                    },
                    report.commits_walked
                ));
                output.line(format!(
                    "  {:<14} {} row(s){}",
                    "changes",
                    report.changes.len(),
                    if report.changes_truncated {
                        " — not all the changes in this range"
                    } else {
                        ""
                    }
                ));
                output.line(format!(
                    "  {:<14} {} in range — a merge has several parents and its changes are not \
                     enumerated,",
                    "merges", report.merges_in_range
                ));
                output.line(
                    "                 so a merge-heavy range carrying few change rows is expected \
                     rather than quiet",
                );
                output.line(format!(
                    "  {:<14} {}",
                    "enumeration",
                    report
                        .changes_enumerated
                        .iter()
                        .map(|(value, count)| format!("{} {count}", value.as_str()))
                        .collect::<Vec<_>>()
                        .join(" · ")
                ));
                if let Some((oid, completeness)) = &report.ancestry_incomplete_at {
                    output.line(format!(
                        "  {:<14} {oid} — {} — {}",
                        "incomplete_at",
                        completeness.as_str(),
                        completeness.note()
                    ));
                    output.line(
                        "                 the older state was reached down one branch while \
                         another was cut off,",
                    );
                    output.line(
                        "                 so this range is a floor rather than the whole of it",
                    );
                }
            }
            for commit in &report.commits {
                // `None` rather than a count: the change list is bounded across the whole range,
                // so a count taken from it would state a property of one commit that the bound,
                // not the commit, decided.
                print_commit(output, commit, None);
                for change in report
                    .changes
                    .iter()
                    .filter(|change| change.commit_oid == commit.commit_oid)
                {
                    output.line(format!(
                        "  {:<14} {} · {}",
                        "change",
                        change.change_kind.as_str(),
                        inert_text(&change.path)
                    ));
                }
            }
            json!({
                "result": "diff",
                "commits": report.commits.iter()
                    .map(|commit| commit_json(commit, None)).collect::<Vec<_>>(),
                "commits_in_range": report.commits_in_range,
                "commits_truncated": report.commits_truncated,
                "commits_walked": report.commits_walked,
                "changes": report.changes.iter().map(|change| {
                    let mut object = change_json(change);
                    if let Some(fields) = object.as_object_mut() {
                        fields.insert("commit_oid".into(), json!(change.commit_oid));
                    }
                    object
                }).collect::<Vec<_>>(),
                "changes_truncated": report.changes_truncated,
                "merges_in_range": report.merges_in_range,
                "changes_enumerated": report.changes_enumerated.iter()
                    .map(|(value, count)| (value.as_str().to_string(), *count))
                    .collect::<std::collections::BTreeMap<String, usize>>(),
                "ancestry_incomplete_at": report.ancestry_incomplete_at.as_ref()
                    .map(|(oid, completeness)| json!({
                        "commit_oid": oid,
                        "parent_completeness": completeness.as_str(),
                        "parent_completeness_note": completeness.note(),
                    })),
            })
        }
    };

    // Every diff-shaped key exists on every outcome, and is `null` where no range was computed.
    // Without this a consumer reading `commits` would find the key absent on a refusal and an
    // empty array on an empty range, which are the two answers this command exists to keep apart.
    let mut fields = serde_json::Map::new();
    fields.insert("from".into(), json!(from));
    fields.insert("to".into(), json!(to));
    fields.insert("limit".into(), json!(limit));
    fields.insert("max_walk".into(), json!(limits.commits_walked));
    fields.insert("max_changes".into(), json!(limits.changes));
    fields.insert("ancestry_not_a_time_range".into(), json!(true));
    for key in [
        "from_recorded",
        "to_recorded",
        "commits_walked",
        "walk_limit",
        "stopped_at",
        "stopped_at_parent_completeness",
        "stopped_at_parent_completeness_note",
        "commits",
        "commits_in_range",
        "commits_truncated",
        "changes",
        "changes_truncated",
        "merges_in_range",
        "changes_enumerated",
        "ancestry_incomplete_at",
    ] {
        fields.insert(key.into(), serde_json::Value::Null);
    }
    if let serde_json::Value::Object(present) = detail {
        for (key, value) in present {
            fields.insert(key, value);
        }
    }
    history_object(
        output,
        "history diff",
        &read,
        serde_json::Value::Object(fields),
    );
    exit::SUCCESS
}

/// Which paths changed most often in visible history.
fn run_history_frequency(output: &Output, path: &Path, limit: usize) -> i32 {
    if limit == 0 {
        return output.failure(
            "history frequency",
            exit::USAGE,
            "--limit must be at least 1",
        );
    }
    let read = match open_history(output, "history frequency", path) {
        Ok(read) => read,
        Err(code) => return code,
    };
    let frequency = match nerve_store::change_frequency(&read.conn, &read.repo_id, limit) {
        Ok(frequency) => frequency,
        Err(err) => return output.failure("history frequency", exit::INTERNAL, &err.to_string()),
    };

    let answerable = print_history_header(output, &read);
    if answerable {
        output.line(format!(
            "  {:<14} {} listed of {} path(s) with a change row, limit {limit}",
            "frequency",
            frequency.rows.len(),
            frequency.paths_total
        ));
        output.line(format!(
            "  {:<14} {} — a fact against the counted total, never inferred from the page length",
            "truncated", frequency.truncated
        ));
        output.line(format!(
            "  {:<14} {} recorded — a merge enumerates no changes, so a repository with merges",
            "merges", frequency.merges
        ));
        output.line("                 counts fewer here than its own log would show");
        output.line(
            "  scope          changes within visible history, not lifetime changes; on a shallow \
             or",
        );
        output.line("                 bounded ingest every count below is a floor");
        if frequency.rows.is_empty() {
            output.line(String::new());
            output.line("  No recorded commit changed any path. That is an answer, not a");
            output.line("  failure — see the availability block above for what was read.");
        }
    }
    for row in &frequency.rows {
        output.line(format!("  {:>8}  {}", row.commits, inert_text(&row.path)));
    }

    history_object(
        output,
        "history frequency",
        &read,
        json!({
            "limit": limit,
            "count": frequency.rows.len(),
            "paths_total": frequency.paths_total,
            "truncated": frequency.truncated,
            "merges": frequency.merges,
            "counts_are_visible_history_only": true,
            "rows": frequency.rows.iter().map(|row| json!({
                "path": inert_text(&row.path),
                "commits": row.commits,
            })).collect::<Vec<_>>(),
        }),
    );
    exit::SUCCESS
}

/// Which paths were changed in the same commits as one path.
///
/// The disclaimer printed and carried below is [`nerve_store::COCHANGE_IS_NOT_A_DEPENDENCY`],
/// **taken from the store rather than written here**. A surface that paraphrased it would be a
/// second copy of the one sentence that stops a shared-commit count reading as a dependency, and
/// the paraphrase is where it would soften.
fn run_history_cochange(output: &Output, path: &Path, tree_path: &str, limit: usize) -> i32 {
    if limit == 0 {
        return output.failure(
            "history cochange",
            exit::USAGE,
            "--limit must be at least 1",
        );
    }
    if let Some(refusal) = nerve_store::history_path_refusal(tree_path) {
        return refuse_history_path(output, "history cochange", tree_path, refusal);
    }
    let read = match open_history(output, "history cochange", path) {
        Ok(read) => read,
        Err(code) => return code,
    };
    let cochange = match nerve_store::cochange(&read.conn, &read.repo_id, Some(tree_path), limit) {
        Ok(cochange) => cochange,
        Err(err) => return output.failure("history cochange", exit::INTERNAL, &err.to_string()),
    };

    let answerable = print_history_header(output, &read);
    if answerable {
        output.line(format!(
            "  {:<14} {} (matched as a tree recorded it, not resolved on disk)",
            "path",
            inert_text(tree_path)
        ));
        output.line(format!(
            "  {:<14} {} listed of {} pair(s) naming this path, limit {limit}",
            "cochange",
            cochange.rows.len(),
            cochange.pairs_total
        ));
        output.line(format!(
            "  {:<14} {} — a fact against the counted total, never inferred from the page length",
            "truncated", cochange.truncated
        ));
        output.line(format!(
            "  {:<14} {} recorded — a merge enumerates no changes, so it contributes no pair at all",
            "merges", cochange.merges
        ));
        output.line(format!("  {:<14} {}", "observation", cochange.disclaimer));
        if cochange.rows.is_empty() {
            output.line(String::new());
            output.line("  No recorded commit changed this path together with another. That is");
            output.line("  an answer, not a failure.");
        }
    }
    for row in &cochange.rows {
        output.line(format!(
            "  {:>8}  {}  +  {}",
            row.cochange_observations,
            inert_text(&row.path_a),
            inert_text(&row.path_b)
        ));
    }

    history_object(
        output,
        "history cochange",
        &read,
        json!({
            "path": inert_text(tree_path),
            "path_is_as_recorded_in_a_tree": true,
            "limit": limit,
            "count": cochange.rows.len(),
            "pairs_total": cochange.pairs_total,
            "truncated": cochange.truncated,
            "merges": cochange.merges,
            // The store's sentence, not this surface's. A paraphrase here would be the second copy.
            "disclaimer": cochange.disclaimer,
            "rows": cochange.rows.iter().map(|row| json!({
                "path_a": inert_text(&row.path_a),
                "path_b": inert_text(&row.path_b),
                // Named for what was observed, never for what it might imply.
                "cochange_observations": row.cochange_observations,
            })).collect::<Vec<_>>(),
        }),
    );
    exit::SUCCESS
}

/// What visible history is unavailable, and whether what was recorded is still current.
fn run_history_availability(output: &Output, path: &Path) -> i32 {
    let read = match open_history(output, "history availability", path) {
        Ok(read) => read,
        Err(code) => return code,
    };
    let freshness = match nerve_store::history_freshness(&read.conn, &read.repo_id) {
        Ok(freshness) => freshness,
        Err(err) => {
            return output.failure("history availability", exit::INTERNAL, &err.to_string())
        }
    };

    // Printed whether or not there is an ingest: `no_history_ingested` is one of the four verdicts
    // and exits `0`, so this command answers even where `log` and `file` have nothing to list.
    print_history_header(output, &read);
    output.line(format!(
        "  {:<14} {} — {}",
        "freshness",
        freshness.verdict.as_str(),
        freshness.verdict.note()
    ));
    output.line(format!(
        "  {:<14} {}",
        "ingest_head",
        freshness
            .ingest_head_oid
            .as_deref()
            .unwrap_or("(none — no ingest, or an unborn branch at sync time)")
    ));
    output.line(format!(
        "  {:<14} {}",
        "indexed_at",
        freshness
            .current_git_commit
            .as_deref()
            .unwrap_or("(none recorded — the indexed state names no commit)")
    ));
    output.line(format!(
        "  {:<14} {}",
        "indexed_state",
        freshness
            .current_state_id
            .as_deref()
            .unwrap_or("(none — no index has run here)")
    ));

    history_object(
        output,
        "history availability",
        &read,
        json!({
            "freshness": freshness.verdict.as_str(),
            "freshness_note": freshness.verdict.note(),
            "ingest_head_oid": freshness.ingest_head_oid,
            "current_git_commit": freshness.current_git_commit,
            "current_state_id": freshness.current_state_id,
        }),
    );
    exit::SUCCESS
}

// ---- the cross-repository registry -----------------------------------------------------------
//
// These four handlers render and nothing else. **Availability is never computed here.** It is
// derived once, in `nerve_index::registry`, because this is the first thing in Nerve whose answer
// depends on a *second* repository — and two surfaces deciding independently whether a neighbour is
// reachable is two answers to one question. `crates/nerve-cli/tests/registry_guards.rs` scans this
// crate and `nerve-server` for a second derivation, in the shape `history_wording.rs` established.
//
// The target path never appears in a `format!` that builds a filesystem path here either. The one
// file Nerve opens in a neighbour is named in `nerve_index::registry`, and a surface that spelled it
// again would be a second place to widen it.

/// Open the index for a command that needs this repository's own `repo_id`, and hand back both.
///
/// `writable` is the whole reason this is one helper rather than two: a command that answers a
/// question must not be able to write, so it takes the `query_only` connection every other read
/// command takes; a mutating command says so at the call site and is the only kind that gets a
/// writable connection.
///
/// Named for the registry because that is what first needed it. `nerve memory` needs the identical
/// pair — an open index and the repository the rows belong to — and calling this rather than
/// opening a second way is what keeps *"which repository is this?"* a question with one answer.
fn open_registry(
    output: &Output,
    command: &'static str,
    path: &Path,
    writable: bool,
) -> Result<(OpenIndex, String), i32> {
    let opened = if writable {
        open_existing(path)
    } else {
        open_query_only(path)
    }
    .map_err(|message| output.failure(command, exit::NO_INDEX, &message))?;
    let repository = nerve_store::repository(&opened.conn)
        .map_err(|err| output.failure(command, exit::INTERNAL, &err.to_string()))?;
    let repo_id = repository
        .ok_or_else(|| {
            output.failure(
                command,
                exit::NO_INDEX,
                &format!(
                    "no repository row at {}; run `nerve init` first",
                    opened.root.display()
                ),
            )
        })?
        .repo_id;
    Ok((opened, repo_id))
}

/// One entry and its verdict, as JSON.
///
/// `local_path` is in the response and is *not* in anything Git tracks: it lives in
/// `.nerve/nerve.db`, which `.gitignore` covers, and `crates/nerve-cli/tests/registry_guards.rs`
/// asserts that rather than trusting it. A user asking where their neighbour is recorded has to be
/// told.
fn registry_entry_json(view: &nerve_index::RegistryEntryView) -> serde_json::Value {
    let entry = &view.entry;
    let availability = &view.availability;
    json!({
        "registry_id": inert_text(&entry.registry_id),
        "expected_repository_id": entry.expected_repository_id,
        "display_name": inert_text(&entry.display_name),
        "local_path": inert_text(&entry.local_path),
        "added_at": entry.added_at,
        "status": entry.status.as_str(),
        "status_note": entry.status.note(),
        "withdrawn_at": entry.withdrawn_at,
        "last_seen_state": entry.last_seen_state,
        "last_seen_at": entry.last_seen_at,
        "availability_checked_at": entry.availability_checked_at,
        "availability": availability.as_str(),
        "availability_statement": availability.statement(),
        "refusal": availability.refusal().map(|reason| reason.as_str()),
        "refusal_statement": availability.refusal().map(|reason| reason.statement()),
        "observed_repository_id": availability.observed_repository_id(),
        "freshness": availability.freshness().map(|state| state.as_str()),
        "freshness_note": availability.freshness().map(|state| state.note()),
    })
}

/// One entry and its verdict, as lines.
///
/// Every repository-supplied string goes through [`inert_text`] for the reason a commit summary
/// does: a display name is content from a checkout that may have been cloned from anywhere, and a
/// newline in it would forge a second line of Nerve's own output.
fn print_registry_entry(output: &Output, view: &nerve_index::RegistryEntryView) {
    let entry = &view.entry;
    let availability = &view.availability;
    output.line(format!(
        "  {}  {}",
        inert_text(&entry.registry_id),
        inert_text(&entry.display_name)
    ));
    output.line(format!(
        "    {:<14} {}",
        "path",
        inert_text(&entry.local_path)
    ));
    output.line(format!(
        "    {:<14} {}",
        "repository_id", entry.expected_repository_id
    ));
    output.line(format!(
        "    {:<14} {} — {}",
        "status",
        entry.status.as_str(),
        entry.status.note()
    ));
    if let Some(withdrawn_at) = &entry.withdrawn_at {
        output.line(format!("    {:<14} {withdrawn_at}", "withdrawn_at"));
    }
    output.line(format!(
        "    {:<14} {} — {}",
        "availability",
        availability.as_str(),
        availability.statement()
    ));
    if let Some(reason) = availability.refusal() {
        output.line(format!(
            "    {:<14} {} — {}",
            "reason",
            reason.as_str(),
            reason.statement()
        ));
    }
    if let Some(found) = availability.observed_repository_id() {
        output.line(format!("    {:<14} {found}", "found_instead"));
    }
    match availability.freshness() {
        Some(state) => output.line(format!(
            "    {:<14} {} — {}",
            "freshness",
            state.as_str(),
            state.note()
        )),
        None => output.line(format!(
            "    {:<14} {}",
            "freshness", "(none — this entry puts no qualification on what resolves through it)"
        )),
    }
}

/// Report a named refusal, with nothing done and nothing guessed at.
fn refuse_registry(
    output: &Output,
    command: &'static str,
    subject: &str,
    reason: nerve_index::RegistryRefusal,
) -> i32 {
    output.failure_detail(
        command,
        exit::USAGE,
        &format!("{}: {}", inert_text(subject), reason.statement()),
        &[
            "  nothing was registered, moved or retired; this is a refusal, not a partial result"
                .to_string(),
        ],
        json!({
            "subject": inert_text(subject),
            "refusal": reason.as_str(),
            "refusal_statement": reason.statement(),
        }),
    )
}

/// The object every successful registry command prints under `--json`.
fn registry_object(
    output: &Output,
    command: &'static str,
    root: &Path,
    entries: Vec<serde_json::Value>,
) {
    output.object(json!({
        "command": command,
        "ok": true,
        "root": root.display().to_string(),
        "entries": entries,
    }));
}

fn run_repo_add(
    output: &Output,
    path: &Path,
    target: &Path,
    id: Option<&str>,
    name: Option<&str>,
) -> i32 {
    let (opened, repo_id) = match open_registry(output, "repo add", path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let outcome = match nerve_index::add_registry_target(&opened.conn, &repo_id, target, id, name) {
        Ok(outcome) => outcome,
        Err(err) => return output.failure("repo add", error_exit_code(&err), &err.to_string()),
    };
    let entry = match outcome {
        nerve_index::RegistryOutcome::Done(entry) => entry,
        nerve_index::RegistryOutcome::Refused(reason) => {
            return refuse_registry(output, "repo add", &target.display().to_string(), reason)
        }
    };
    let view = nerve_index::RegistryEntryView {
        availability: nerve_index::availability_of(&entry),
        entry,
    };
    output.line(format!(
        "Registered {} in {}",
        inert_text(&view.entry.registry_id),
        opened.root.display()
    ));
    print_registry_entry(output, &view);
    registry_object(
        output,
        "repo add",
        &opened.root,
        vec![registry_entry_json(&view)],
    );
    exit::SUCCESS
}

fn run_repo_list(output: &Output, path: &Path) -> i32 {
    let (opened, repo_id) = match open_registry(output, "repo list", path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let views = match nerve_index::list_registry(&opened.conn, &repo_id) {
        Ok(views) => views,
        Err(err) => return output.failure("repo list", error_exit_code(&err), &err.to_string()),
    };

    if views.is_empty() {
        output.line(format!(
            "No registered neighbours in {}",
            opened.root.display()
        ));
        output.line(
            "  registry       empty — no sibling directory is ever registered on its own; a \
             neighbour exists because `nerve repo add` named it",
        );
    } else {
        output.line(format!(
            "{} registered neighbour(s) in {}",
            views.len(),
            opened.root.display()
        ));
        for view in &views {
            print_registry_entry(output, view);
        }
    }
    registry_object(
        output,
        "repo list",
        &opened.root,
        views.iter().map(registry_entry_json).collect(),
    );
    exit::SUCCESS
}

fn run_repo_remove(output: &Output, path: &Path, registry_id: &str) -> i32 {
    let (opened, repo_id) = match open_registry(output, "repo remove", path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let outcome = match nerve_index::remove_registry_target(&opened.conn, &repo_id, registry_id) {
        Ok(outcome) => outcome,
        Err(err) => return output.failure("repo remove", error_exit_code(&err), &err.to_string()),
    };
    let entry = match outcome {
        nerve_index::RegistryOutcome::Done(entry) => entry,
        nerve_index::RegistryOutcome::Refused(reason) => {
            return refuse_registry(output, "repo remove", registry_id, reason)
        }
    };
    let view = nerve_index::RegistryEntryView {
        availability: nerve_index::availability_of(&entry),
        entry,
    };
    output.line(format!(
        "Retired {} — the entry is kept, not deleted, and stays listed",
        inert_text(&view.entry.registry_id)
    ));
    print_registry_entry(output, &view);
    registry_object(
        output,
        "repo remove",
        &opened.root,
        vec![registry_entry_json(&view)],
    );
    exit::SUCCESS
}

fn run_repo_relocate(output: &Output, path: &Path, registry_id: &str, new_path: &Path) -> i32 {
    let (opened, repo_id) = match open_registry(output, "repo relocate", path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let outcome = match nerve_index::relocate_registry_target(
        &opened.conn,
        &repo_id,
        registry_id,
        new_path,
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            return output.failure("repo relocate", error_exit_code(&err), &err.to_string())
        }
    };
    let entry = match outcome {
        nerve_index::RegistryOutcome::Done(entry) => entry,
        nerve_index::RegistryOutcome::Refused(reason) => {
            return refuse_registry(output, "repo relocate", registry_id, reason)
        }
    };
    let view = nerve_index::RegistryEntryView {
        availability: nerve_index::availability_of(&entry),
        entry,
    };
    output.line(format!(
        "Relocated {} — the repository recorded for this entry was found at the new path",
        inert_text(&view.entry.registry_id)
    ));
    print_registry_entry(output, &view);
    registry_object(
        output,
        "repo relocate",
        &opened.root,
        vec![registry_entry_json(&view)],
    );
    exit::SUCCESS
}

// ---- reading the stored links back -------------------------------------------------------------
//
// `repo scan` renders what a scan produced. Everything below renders what is **stored**, with the
// standing `nerve_index::contract_report` decided for it — the same call `/api/contracts` and the
// `nerve_contracts` MCP tool make, on the same connection discipline. No verdict is computed here,
// for the reason the registry handlers above give: a second derivation of a link's standing is a
// second answer, and `crates/nerve-cli/tests/registry_guards.rs` scans this crate for the shapes one
// would have to be written in.

/// What a link with no qualification on it is, said rather than left as a blank.
///
/// The absence of a verdict is *no qualification*, never "unknown". A reader meeting an empty field
/// would have that exactly backwards, which is why the state is printed in words.
const LINK_IS_CURRENT: &str = "current — its registry entry is available, both recorded states \
                               still match, and the manifest it was quoted from is still here";

/// Why a repository with no registered neighbour has no links.
///
/// Distinct from [`NOTHING_WAS_DECLARED`] on purpose. Nothing is ever discovered, so "no neighbour
/// was ever named" and "neighbours were named and no manifest declares a path into one" are
/// different situations with different next steps, and an answer that let one be read as the other
/// would be reporting *we never looked* as *we looked and there is nothing*.
const NO_NEIGHBOUR_REGISTERED: &str =
    "empty — no neighbour is registered, so no declaration in this repository could have resolved \
     to one. `nerve repo add <path> --id <name>` names one; no sibling directory is ever registered \
     on its own";

/// Why a repository that *has* neighbours can still have no links.
const NOTHING_WAS_DECLARED: &str =
    "run `nerve repo scan` if none has been run since those neighbours were registered; otherwise \
     no manifest in this repository declares a path into any of them";

/// Which of the three answers this is, named inside the answer rather than inferred from a count.
fn contract_links_result_kind(report: &nerve_index::ContractReport) -> &'static str {
    if !report.links.is_empty() {
        "contract_links"
    } else if report.entries.is_empty() {
        "no_registered_neighbours"
    } else {
        "no_contract_links"
    }
}

/// One stored link and its standing, as JSON.
///
/// The keys are `/api/contracts`'s keys, deliberately: the same row read through two surfaces has to
/// be recognisably the same row, and `scripts/final_acceptance.sh` compares the verdicts across them
/// rather than asserting this command exists.
///
/// Every string that came out of a manifest or out of the *neighbour's* index goes through
/// [`inert_text`] on T7's terms — a contract identity, a version, a manifest path and every target
/// snapshot field are repository content, and the target fields were read from a checkout this
/// repository does not control at all.
fn contract_link_view_json(view: &nerve_index::ContractLinkView) -> serde_json::Value {
    let link = &view.link;
    let entry = nerve_index::RegistryEntryView {
        entry: view.entry.clone(),
        availability: view.availability.clone(),
    };
    json!({
        "link_id": link.link_id,
        // What the declaration says, in the words the manifest used.
        "relation_semantics": inert_text(&link.relation_semantics),
        "contract_kind": inert_text(&link.contract_kind),
        "contract_identity": inert_text(&link.contract_identity),
        "resolution_method": link.resolution_method.as_str(),
        "resolution_method_note": link.resolution_method.note(),
        // Both versions, and never a verdict: deciding whether `1.2.3` satisfies `^1.2.0` is range
        // resolution, which needs a resolver this product does not have.
        "expected_contract_version": link.expected_contract_version.as_deref().map(inert_text),
        "observed_contract_version": link.observed_contract_version.as_deref().map(inert_text),
        // The local end. Every one of these is a fact about this database.
        "source_repository_id": link.source_repository_id,
        "source_state_at_resolution": link.source_state_at_resolution,
        "source_entity_id": link.source_entity_id,
        "source_kind_snapshot": link.source_kind_snapshot.as_deref().map(inert_text),
        "source_path": inert_text(&link.source_path),
        "source_span": inert_text(&link.source_span),
        "source_manifest_present": view.source_manifest_present,
        // The far end, and it is a snapshot rather than a pointer.
        "expected_target_repository_id": link.expected_target_repository_id,
        "target_state_at_resolution": link.target_state_at_resolution,
        "target_current_state": view.target_current_state,
        "target_entity_id": link.target_entity_id.as_deref().map(inert_text),
        "target_kind_snapshot": link.target_kind_snapshot.as_deref().map(inert_text),
        "target_name_snapshot": link.target_name_snapshot.as_deref().map(inert_text),
        "target_path_snapshot": link.target_path_snapshot.as_deref().map(inert_text),
        "target_span_snapshot": link.target_span_snapshot.as_deref().map(inert_text),
        // Who wrote the row, and what it could not resolve.
        "extractor_id": link.extractor_id,
        "extractor_version": link.extractor_version,
        "evidence_details": link.evidence_details.as_deref().map(inert_text),
        "ambiguity": link.ambiguity.as_deref().map(inert_text),
        "unsupported_reason": link.unsupported_reason.as_deref().map(inert_text),
        // Lifecycle. A withdrawn link is kept so that the ending can be reported at all.
        "status": link.status.as_str(),
        "status_note": link.status.note(),
        "first_seen_at": link.first_seen_at,
        "last_seen_at": link.last_seen_at,
        "withdrawn_at": link.withdrawn_at,
        // The verdict, carried off the service that decided it. `null` is *no qualification*, so
        // `is_current` states it rather than leaving a consumer to read an absent field as unknown.
        "freshness": view.freshness.map(|state| state.as_str()),
        "freshness_note": view.freshness.map(|state| state.note()),
        "is_current": view.freshness.is_none(),
        // The entry it came through, in full, through the one renderer that exists for an entry.
        "registry_entry": registry_entry_json(&entry),
    })
}

/// One stored link and its standing, as lines.
fn print_contract_link(output: &Output, view: &nerve_index::ContractLinkView) {
    let link = &view.link;
    output.line(format!(
        "  #{}  {}  {}",
        // A row read back out of the table always carries its key. Printing `0` for a `None` would
        // name a row that does not exist, so the absence is said instead.
        link.link_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unassigned".to_string()),
        inert_text(&link.contract_kind),
        inert_text(&link.contract_identity)
    ));
    // First, because it is the question the command exists to answer.
    match view.freshness {
        Some(state) => output.line(format!(
            "    {:<14} {} — {}",
            "freshness",
            state.as_str(),
            state.note()
        )),
        None => output.line(format!("    {:<14} {LINK_IS_CURRENT}", "freshness")),
    }
    output.line(format!(
        "    {:<14} {}",
        "relation",
        inert_text(&link.relation_semantics)
    ));
    output.line(format!(
        "    {:<14} {} — {}",
        "resolution",
        link.resolution_method.as_str(),
        link.resolution_method.note()
    ));
    output.line(format!(
        "    {:<14} {} — {}",
        "status",
        link.status.as_str(),
        link.status.note()
    ));
    if let Some(withdrawn_at) = &link.withdrawn_at {
        output.line(format!("    {:<14} {withdrawn_at}", "withdrawn_at"));
    }
    output.line(format!(
        "    {:<14} {}:{} — {}",
        "manifest",
        inert_text(&link.source_path),
        inert_text(&link.source_span),
        if view.source_manifest_present {
            "still in this repository"
        } else {
            "no longer in this repository"
        }
    ));
    output.line(format!(
        "    {:<14} expects {}, target declares {} (both recorded, never compared)",
        "versions",
        link.expected_contract_version
            .as_deref()
            .map(inert_text)
            .unwrap_or_else(|| "(none)".to_string()),
        link.observed_contract_version
            .as_deref()
            .map(inert_text)
            .unwrap_or_else(|| "(none)".to_string()),
    ));
    output.line(format!(
        "    {:<14} {}",
        "target_repo", link.expected_target_repository_id
    ));
    if let Some(path) = &link.target_path_snapshot {
        output.line(format!(
            "    {:<14} {} — {}",
            "target",
            inert_text(path),
            match &link.target_entity_id {
                Some(entity) => format!("entity {}", inert_text(entity)),
                None => "the neighbour has the file and its index has never read it".to_string(),
            }
        ));
    }
    output.line(format!(
        "    {:<14} source {} · target recorded {} · target now {}",
        "states",
        link.source_state_at_resolution,
        link.target_state_at_resolution
            .as_deref()
            .unwrap_or("(none)"),
        view.target_current_state.as_deref().unwrap_or("(not read)"),
    ));
    if let Some(ambiguity) = &link.ambiguity {
        output.line(format!("    {:<14} {}", "ambiguity", inert_text(ambiguity)));
    }
    if let Some(reason) = &link.unsupported_reason {
        output.line(format!("    {:<14} {}", "unsupported", inert_text(reason)));
    }
    output.line(format!(
        "    {:<14} {}  {}",
        "entry",
        inert_text(&view.entry.registry_id),
        inert_text(&view.entry.display_name)
    ));
    output.line(format!(
        "    {:<14} {} — {}",
        "entry_state",
        view.availability.as_str(),
        view.availability.statement()
    ));
}

/// `nerve repo links` — read the stored links back, each with its freshness.
///
/// Bounded, and `truncated` is a comparison against a counted total rather than the guess
/// `returned == limit`, which is false exactly when the answer ends on the boundary.
fn run_repo_links(output: &Output, path: &Path, limit: usize) -> i32 {
    if limit == 0 {
        return output.failure("repo links", exit::USAGE, "--limit must be at least 1");
    }
    let (opened, repo_id) = match open_registry(output, "repo links", path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let report = match nerve_index::contract_report(&opened.conn, &repo_id, &opened.root) {
        Ok(report) => report,
        Err(err) => return output.failure("repo links", error_exit_code(&err), &err.to_string()),
    };

    let total = report.links.len();
    let shown = total.min(limit);
    let truncated = shown < total;
    let result_kind = contract_links_result_kind(&report);

    if total == 0 {
        output.line(format!("No contract links in {}", opened.root.display()));
        if report.entries.is_empty() {
            output.line(format!("  {:<14} {NO_NEIGHBOUR_REGISTERED}", "registry"));
        } else {
            output.line(format!(
                "  {:<14} {} registered neighbour(s), and no declaration resolved through any of \
                 them",
                "registry",
                report.entries.len()
            ));
            output.line(format!("  {:<14} {NOTHING_WAS_DECLARED}", "next"));
        }
    } else {
        output.line(format!(
            "{total} contract link(s) in {}",
            opened.root.display()
        ));
        output.line(format!(
            "  {:<14} {}",
            "source_state",
            report.source_state.as_deref().unwrap_or("(never indexed)")
        ));
        output.line(format!(
            "  {:<14} {} registered neighbour(s)",
            "registry",
            report.entries.len()
        ));
        output.line(format!(
            "  {:<14} {shown} of {total}, limit {limit}{}",
            "shown",
            if truncated {
                " — more beyond the limit"
            } else {
                ""
            }
        ));
        // Structurally zero, and reported rather than assumed: a link dropped from an answer could
        // not be reported as having been dropped.
        if report.links_without_registry_entry > 0 {
            output.line(format!(
                "  {:<14} {} link(s) whose registry entry could not be found",
                "unkeyed", report.links_without_registry_entry
            ));
        }
        for view in report.links.iter().take(limit) {
            print_contract_link(output, view);
        }
    }

    output.object(json!({
        "command": "repo links",
        "ok": true,
        "root": opened.root.display().to_string(),
        "result_kind": result_kind,
        "source_state": report.source_state,
        "registry_entries_total": report.entries.len(),
        "links_total": total,
        "links_without_registry_entry": report.links_without_registry_entry,
        "returned": shown,
        "limit": limit,
        "truncated": truncated,
        "links": report
            .links
            .iter()
            .take(limit)
            .map(contract_link_view_json)
            .collect::<Vec<_>>(),
    }));
    exit::SUCCESS
}

/// One recorded link, as JSON.
///
/// Every string that came out of a manifest — the identity, the section — is repository content and
/// goes through [`inert_text`] on exactly T7's terms. The resolution method, the form and the rule
/// are closed vocabularies rendered by the service that owns them, never spelled here.
fn contract_link_json(link: &nerve_index::RecordedLink) -> serde_json::Value {
    json!({
        "rule": link.rule.as_str(),
        "manifest": inert_text(&link.manifest),
        "section": inert_text(&link.section),
        "identity": inert_text(&link.identity),
        "form": link.form.as_str(),
        "registry_id": inert_text(&link.registry_id),
        "target_repository_id": link.target_repository_id,
        "resolution_method": link.resolution_method.as_str(),
        "source_span": link.source_span,
        // The two halves of a cross-repository link, and they are not symmetric. The source id is
        // a foreign key into this database; the target id is a snapshot of a row in another one,
        // and a target path with no target id means the neighbour has the file and has not
        // indexed it.
        "relation_semantics": link.relation_semantics,
        "source_entity_id": link.source_entity_id,
        "target_entity_id": link.target_entity_id,
        "target_path": link.target_path.as_deref().map(inert_text),
        "expected_contract_version": link.expected_contract_version.as_deref().map(inert_text),
        "observed_contract_version": link.observed_contract_version.as_deref().map(inert_text),
        "ambiguity": link.ambiguity.map(|value| value.as_str()),
        "inserted": link.inserted,
    })
}

/// `nerve repo scan` — read the manifests, resolve what they declare, record the links.
///
/// The output is a **tally**, not a list to read by eye. §9.1 of the row plan requires that an
/// unsupported form be recorded with its form named rather than silently dropped, *asserted by a
/// tally*, so the counts per form are the point of the response rather than a summary of it.
fn run_repo_scan(output: &Output, path: &Path) -> i32 {
    let (opened, repo_id) = match open_registry(output, "repo scan", path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let outcome = match nerve_index::scan_contracts(&opened.conn, &repo_id, &opened.root) {
        Ok(outcome) => outcome,
        Err(err) => return output.failure("repo scan", error_exit_code(&err), &err.to_string()),
    };
    let scan = match outcome {
        nerve_index::ScanOutcome::Done(scan) => scan,
        nerve_index::ScanOutcome::Refused(reason) => {
            return output.failure_detail(
                "repo scan",
                exit::NO_INDEX,
                reason.statement(),
                &["  nothing was scanned and nothing was written".to_string()],
                json!({
                    "refusal": reason.as_str(),
                    "refusal_statement": reason.statement(),
                }),
            )
        }
    };

    output.line(format!(
        "Scanned {} manifest(s) in {} — {} declaration(s)",
        scan.manifests_read,
        opened.root.display(),
        scan.declarations
    ));
    output.line(format!(
        "  {:<14} {} recorded, {} already stored",
        "links",
        scan.inserted(),
        scan.unchanged()
    ));
    for link in &scan.links {
        output.line(format!(
            "    {} {} -> {} ({})",
            inert_text(&link.section),
            inert_text(&link.identity),
            inert_text(&link.registry_id),
            link.resolution_method.as_str()
        ));
    }
    for (form, count) in scan.unsupported_tally() {
        output.line(format!("  {:<14} {} x {}", "unsupported", count, form));
    }
    for (reason, count) in scan.unresolved_tally() {
        output.line(format!("  {:<14} {} x {}", "unresolved", count, reason));
    }
    for (manifest, refusal) in &scan.refusals {
        output.line(format!(
            "  {:<14} {} — {}",
            "refused",
            inert_text(manifest),
            refusal
        ));
    }

    output.object(json!({
        "command": "repo scan",
        "ok": true,
        "root": opened.root.display().to_string(),
        "source_state": scan.source_state,
        "manifests_read": scan.manifests_read,
        "declarations": scan.declarations,
        "links_recorded": scan.inserted(),
        "links_unchanged": scan.unchanged(),
        "links": scan.links.iter().map(contract_link_json).collect::<Vec<_>>(),
        "unsupported": scan
            .unsupported
            .iter()
            .map(|row| json!({
                "rule": row.rule.as_str(),
                "manifest": inert_text(&row.manifest),
                "section": inert_text(&row.section),
                "identity": inert_text(&row.identity),
                "form": row.form.as_str(),
            }))
            .collect::<Vec<_>>(),
        "unsupported_tally": scan
            .unsupported_tally()
            .into_iter()
            .map(|(form, count)| json!({ "form": form.as_str(), "count": count }))
            .collect::<Vec<_>>(),
        "unresolved": scan
            .unresolved
            .iter()
            .map(|row| json!({
                "rule": row.rule.as_str(),
                "manifest": inert_text(&row.manifest),
                "section": inert_text(&row.section),
                "identity": inert_text(&row.identity),
                "form": row.form.as_str(),
                "reason": row.reason.as_str(),
            }))
            .collect::<Vec<_>>(),
        "refusals": scan
            .refusals
            .iter()
            .map(|(manifest, refusal)| json!({
                "manifest": inert_text(manifest),
                "refusal": refusal.as_str(),
            }))
            .collect::<Vec<_>>(),
    }));
    exit::SUCCESS
}

// ---- human-confirmed memory --------------------------------------------------------------------
//
// These handlers render and map exit codes. **No lifecycle logic lives here.** Every status change
// and the event that records it happen inside one `nerve-store` transaction — `propose_memory`,
// `confirm_memory`, `supersede_memory`, `invalidate_memory`, `cite_memory` — because a surface that
// flipped a status and then appended an event would leave, on any failure between the two, a record
// that changed with no record of changing. That is the failure the audit history exists to prevent,
// and it is not a failure a surface gets to re-introduce.
//
// The one derived value this surface reads directly is the anchor state, and it reads it from
// `nerve_store::current_repository_state` rather than deriving it: a second answer to "which state
// does this database describe?" is the duplication Slice 12c-i-a existed to remove.

/// The admitted values of a closed vocabulary, for a refusal that names the whole set.
///
/// Generated from `ALL` rather than typed out, so a value added to a vocabulary is offered by the
/// refusal the day it exists.
fn admitted(values: impl IntoIterator<Item = &'static str>) -> String {
    values.into_iter().collect::<Vec<_>>().join(", ")
}

/// Parse `--scope`, or refuse naming the admitted set.
///
/// A refusal rather than a filter that returns nothing, and the distinction is the reason the
/// vocabulary is closed at all: against a free-form column `--scope opertions` answers *"there are
/// no notes"* when what is true is *"there is no such scope"*. Absence is not zero.
fn parse_memory_scope(value: &str) -> Result<MemoryScope, String> {
    value.parse::<MemoryScope>().map_err(|_| {
        format!(
            "unknown --scope {value:?}; expected one of: {}",
            admitted(MemoryScope::ALL.iter().map(|scope| scope.as_str()))
        )
    })
}

/// Parse `--status`, or refuse naming the admitted set.
///
/// **The stored lifecycle only.** A caller asking for `potentially_stale` is asking to filter on a
/// value nothing ever wrote, so the refusal names the derived views separately rather than letting
/// the request fall through to an empty list that would read as *"nothing is stale"*.
fn parse_memory_status(value: &str) -> Result<MemoryStatus, String> {
    value.parse::<MemoryStatus>().map_err(|_| {
        format!(
            "unknown --status {value:?}; expected one of: {}. \
             {} are derived at read time, never stored, and are reported beside every record \
             rather than filtered on",
            admitted(MemoryStatus::ALL.iter().map(|status| status.as_str())),
            admitted(
                nerve_core::vocab::MemoryView::ALL
                    .iter()
                    .map(|v| v.as_str())
            )
        )
    })
}

/// A memory id for a caller who did not supply one.
///
/// The clock rather than a hash of the content: two identical notes written a minute apart are two
/// notes, and a content-derived id would silently make the second one a primary-key collision. If
/// two calls did land in the same nanosecond the `INSERT` fails loudly — which is the behaviour
/// this schema chose over `INSERT OR IGNORE` everywhere else, for the reason Slice 3b measured.
fn generated_memory_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    format!("mem_{nanos:016x}")
}

/// Map a storage failure to an exit code.
///
/// [`nerve_store::StoreError::Memory`] is a **refusal**, not a fault: superseding an invalidated
/// record, confirming something that is already active, naming an id that is not here. Each of
/// those is a wrong command line and exits [`exit::USAGE`], and the store's own sentence — which
/// names the status the record is actually in — is what the caller is shown, because a surface that
/// re-worded it would be a second statement of the same rule.
fn memory_exit_code(err: &nerve_store::StoreError) -> i32 {
    match err {
        nerve_store::StoreError::Memory(_) => exit::USAGE,
        _ => exit::INTERNAL,
    }
}

/// The repository state a new record anchors to, or a refusal.
///
/// A repository nothing has indexed is refused rather than anchored to a state invented for the
/// occasion. Staleness is `active` plus *"the anchor is not the current state"*, so a note anchored
/// to nothing could never be qualified — and a qualification that can never be computed is worse
/// than a refusal, because it looks like an answer.
fn memory_anchor(
    output: &Output,
    command: &'static str,
    conn: &nerve_store::Connection,
    repo_id: &str,
) -> Result<String, i32> {
    match nerve_store::current_repository_state(conn, repo_id) {
        Ok(Some(state)) => Ok(state),
        Ok(None) => Err(output.failure_detail(
            command,
            exit::NO_INDEX,
            "this repository has not been indexed, so there is no repository state to anchor a \
             note to",
            &[
                "  run `nerve index` first".to_string(),
                "  nothing was written; a note anchored to nothing has no staleness to derive, and \
                 inventing an anchor would make every later reading of it a guess"
                    .to_string(),
            ],
            json!({
                "anchor_state_id": serde_json::Value::Null,
                "reason": "repository_not_indexed",
            }),
        )),
        Err(err) => Err(output.failure(command, memory_exit_code(&err), &err.to_string())),
    }
}

/// One record with everything a read surface must be able to show.
///
/// The citations and the events are read separately rather than joined into
/// [`nerve_store::MemoryReport`], because they are lists and the report is one row. Assembling them
/// here keeps every read verb rendering the same shape: a `list` that showed less than a `show`
/// would be two answers to one question.
struct MemoryDetail {
    report: nerve_store::MemoryReport,
    citations: Vec<nerve_store::MemoryCitationRow>,
    events: Vec<nerve_store::MemoryEventRow>,
}

fn memory_details(
    conn: &nerve_store::Connection,
    repo_id: &str,
    reports: Vec<nerve_store::MemoryReport>,
) -> Result<Vec<MemoryDetail>, nerve_store::StoreError> {
    let mut out = Vec::with_capacity(reports.len());
    for report in reports {
        let memory_id = report.row.memory_id.clone();
        out.push(MemoryDetail {
            citations: nerve_store::memory_citations(conn, repo_id, &memory_id)?,
            events: nerve_store::memory_events(conn, repo_id, &memory_id)?,
            report,
        });
    }
    Ok(out)
}

/// Read one record, or render the refusal that says it is not here.
fn memory_detail(
    output: &Output,
    command: &'static str,
    conn: &nerve_store::Connection,
    repo_id: &str,
    memory_id: &str,
) -> Result<MemoryDetail, i32> {
    let report = match nerve_store::read_memory(conn, repo_id, memory_id) {
        Ok(Some(report)) => report,
        Ok(None) => {
            return Err(output.failure_detail(
                command,
                exit::NO_INDEX,
                &format!(
                    "no memory record {} in this repository",
                    inert_text(memory_id)
                ),
                &["  `nerve memory list` prints every record, retired ones included".to_string()],
                json!({ "memory_id": inert_text(memory_id) }),
            ))
        }
        Err(err) => return Err(output.failure(command, memory_exit_code(&err), &err.to_string())),
    };
    let mut details = memory_details(conn, repo_id, vec![report])
        .map_err(|err| output.failure(command, memory_exit_code(&err), &err.to_string()))?;
    Ok(details.remove(0))
}

/// The stored subject snapshot, as JSON. **Every field is a copy, and none is a live pointer.**
fn memory_subject_json(subject: &nerve_store::MemorySubject) -> serde_json::Value {
    json!({
        "entity_id": subject.entity_id,
        "kind": inert_text(&subject.kind),
        "name": inert_text(&subject.name),
        "path": inert_text(&subject.path),
        "selector": inert_text(&subject.selector),
    })
}

fn memory_citation_json(citation: &nerve_store::MemoryCitationRow) -> serde_json::Value {
    json!({
        "citation_id": citation.citation_id,
        "cited_entity_id": citation.cited_entity_id,
        "cited_kind": citation.cited_kind.as_deref().map(inert_text),
        "cited_name": citation.cited_name.as_deref().map(inert_text),
        "cited_path": inert_text(&citation.cited_path),
        "cited_span": citation.cited_span.as_deref().map(inert_text),
        "cited_at_state": citation.cited_at_state,
        "created_at": citation.created_at,
    })
}

fn memory_event_json(event: &nerve_store::MemoryEventRow) -> serde_json::Value {
    json!({
        "event_id": event.event_id,
        "at": event.at,
        "operation": event.operation.as_str(),
        "operation_note": event.operation.note(),
        "changes_status": event.operation.changes_status(),
        "from_status": event.from_status.map(|status| status.as_str()),
        "to_status": event.to_status.as_str(),
        "note": event.note.as_deref().map(inert_text),
    })
}

/// One record, everything stored about it and everything derived, as JSON.
///
/// **The two kinds are never mixed.** `status` is the stored lifecycle and `views` are query-time
/// qualifications, under different keys, each rendered through its own vocabulary's `note` so no
/// surface restates the rule in its own words. `superseded_by_memory_id` is marked derived for the
/// same reason: there is no such column, and a consumer that wrote one back would be storing a
/// second, independently writable copy of a fact the schema keeps in one direction.
fn memory_json(detail: &MemoryDetail) -> serde_json::Value {
    let report = &detail.report;
    let row = &report.row;
    let scope = row.scope.parse::<MemoryScope>().ok();
    json!({
        "memory_id": inert_text(&row.memory_id),
        "status": row.status.as_str(),
        "status_note": row.status.note(),
        "views": report
            .views
            .iter()
            .map(|view| json!({ "view": view.as_str(), "note": view.note() }))
            .collect::<Vec<_>>(),
        "views_are_derived": true,
        "subject": memory_subject_json(&row.subject),
        "subject_resolution": report.subject.resolution.as_str(),
        "subject_resolution_note": report.subject.resolution.note(),
        "subject_live_entity_ids": report.subject.live_entity_ids,
        "scope": inert_text(&row.scope),
        "scope_note": scope.map(|scope| scope.note()),
        "claim_key": row.claim_key.as_deref().map(inert_text),
        "anchor_state_id": row.anchor_state_id,
        "current_state_id": report.current_state_id,
        "content": inert_text(&row.content),
        "author_label": inert_text(&row.author_label),
        "author_label_is_an_identity": false,
        "created_at": row.created_at,
        "supersedes_memory_id": row.supersedes_memory_id.as_deref().map(inert_text),
        "superseded_by_memory_id": report.superseded_by.as_deref().map(inert_text),
        "superseded_by_is_derived": true,
        "invalidated_at": row.invalidated_at,
        "invalidation_reason": row.invalidation_reason.as_deref().map(inert_text),
        "citations": detail.citations.iter().map(memory_citation_json).collect::<Vec<_>>(),
        "events": detail.events.iter().map(memory_event_json).collect::<Vec<_>>(),
    })
}

/// One record as lines, carrying the same facts as [`memory_json`] in the same order.
///
/// Every human-supplied and repository-derived string goes through [`inert_text`], for the reason a
/// commit summary does: a newline inside a note would forge a second line of Nerve's own output.
fn print_memory(output: &Output, detail: &MemoryDetail) {
    let report = &detail.report;
    let row = &report.row;
    output.line(format!(
        "  {}  {} — {}",
        inert_text(&row.memory_id),
        row.status.as_str(),
        row.status.note()
    ));
    for view in &report.views {
        output.line(format!(
            "    {:<14} {} — {} (derived at read time, never stored)",
            "view",
            view.as_str(),
            view.note()
        ));
    }
    output.line(format!(
        "    {:<14} {} — {} {} at {}",
        "subject",
        inert_text(&row.subject.selector),
        inert_text(&row.subject.kind),
        inert_text(&row.subject.name),
        if row.subject.path.is_empty() {
            "(no file — the repository itself)".to_string()
        } else {
            inert_text(&row.subject.path)
        }
    ));
    output.line(format!(
        "    {:<14} {} — {}",
        "resolution",
        report.subject.resolution.as_str(),
        report.subject.resolution.note()
    ));
    for live in &report.subject.live_entity_ids {
        output.line(format!("    {:<14} {live}", "reaches"));
    }
    match row.scope.parse::<MemoryScope>() {
        Ok(scope) => output.line(format!(
            "    {:<14} {} — {}",
            "scope",
            scope.as_str(),
            scope.note()
        )),
        Err(_) => output.line(format!("    {:<14} {}", "scope", inert_text(&row.scope))),
    }
    output.line(format!(
        "    {:<14} {}",
        "claim_key",
        match &row.claim_key {
            Some(key) => inert_text(key),
            None => "(none — this record answers no named claim and competes with nothing)".into(),
        }
    ));
    output.line(format!(
        "    {:<14} {} (current {})",
        "anchor_state",
        row.anchor_state_id,
        report
            .current_state_id
            .as_deref()
            .unwrap_or("none — nothing has been indexed here")
    ));
    if let Some(predecessor) = &row.supersedes_memory_id {
        output.line(format!(
            "    {:<14} {}",
            "supersedes",
            inert_text(predecessor)
        ));
    }
    if let Some(successor) = &report.superseded_by {
        output.line(format!(
            "    {:<14} {} (derived from the one stored direction, never a second column)",
            "superseded_by",
            inert_text(successor)
        ));
    }
    if let Some(at) = &row.invalidated_at {
        output.line(format!("    {:<14} {at}", "invalidated_at"));
    }
    output.line(format!(
        "    {:<14} {}",
        "reason",
        match &row.invalidation_reason {
            Some(reason) => inert_text(reason),
            None => "(none)".to_string(),
        }
    ));
    output.line(format!(
        "    {:<14} {} — a local label, not an identity: Nerve has no accounts and nothing \
         verified this",
        "author",
        inert_text(&row.author_label)
    ));
    output.line(format!("    {:<14} {}", "created_at", row.created_at));
    output.line(format!(
        "    {:<14} {}",
        "content",
        inert_text(&row.content)
    ));
    if detail.citations.is_empty() {
        output.line(format!(
            "    {:<14} {}",
            "citations", "(none — this record cites no passage)"
        ));
    }
    for citation in &detail.citations {
        output.line(format!(
            "    {:<14} #{}  {}{}  at {}",
            "citation",
            citation.citation_id.unwrap_or_default(),
            inert_text(&citation.cited_path),
            match &citation.cited_span {
                Some(span) => format!(":{}", inert_text(span)),
                None => " (whole file)".to_string(),
            },
            citation.cited_at_state
        ));
    }
    if detail.events.is_empty() {
        output.line(format!(
            "    {:<14} {}",
            "events", "(none recorded — this record's history begins with what is above)"
        ));
    }
    for event in &detail.events {
        output.line(format!(
            "    {:<14} #{}  {:<12} {} → {}  {}",
            "event",
            event.event_id.unwrap_or_default(),
            event.operation.as_str(),
            event
                .from_status
                .map(|status| status.as_str())
                .unwrap_or("(none)"),
            event.to_status.as_str(),
            match &event.note {
                Some(note) => inert_text(note),
                None => String::new(),
            }
        ));
    }
}

/// The object every memory command prints under `--json`.
fn memory_object(
    output: &Output,
    command: &'static str,
    root: &Path,
    records: Vec<serde_json::Value>,
) {
    output.object(json!({
        "command": command,
        "ok": true,
        "root": root.display().to_string(),
        "count": records.len(),
        "records": records,
    }));
}

/// Print a whole answer: the records, then the JSON object carrying the same facts.
fn render_memory(output: &Output, command: &'static str, root: &Path, details: &[MemoryDetail]) {
    for detail in details {
        print_memory(output, detail);
    }
    memory_object(
        output,
        command,
        root,
        details.iter().map(memory_json).collect(),
    );
}

/// `nerve memory propose`'s arguments.
struct MemoryProposeArguments {
    subject: String,
    scope: String,
    content: String,
    claim_key: Option<String>,
    author: Option<String>,
    id: Option<String>,
}

/// `nerve memory supersede`'s arguments.
struct MemorySupersedeArguments {
    predecessor_id: String,
    content: String,
    scope: Option<String>,
    claim_key: Option<String>,
    author: Option<String>,
    id: Option<String>,
    note: Option<String>,
}

/// `nerve memory list`'s filters.
struct MemoryListArguments {
    scope: Option<String>,
    subject: Option<String>,
    status: Option<String>,
}

/// The default `author_label`.
///
/// Deliberately not the operating-system user name. Nerve has no accounts, and a field defaulting
/// to something identity-shaped invites being read as authentication — which is the reading the
/// column's own documentation exists to refuse.
const DEFAULT_AUTHOR_LABEL: &str = "local";

/// Refuse a value the schema would refuse anyway, but at the point of entry and by name.
///
/// A `CHECK` failing is a correct outcome with an unhelpful sentence: *"CHECK constraint failed"*
/// does not say which field was empty or why an empty one is not a value.
fn refuse_empty(
    output: &Output,
    command: &'static str,
    field: &str,
    value: &str,
    because: &str,
) -> Option<i32> {
    if !value.trim().is_empty() {
        return None;
    }
    Some(output.failure_detail(
        command,
        exit::USAGE,
        &format!("{field} is empty, and {because}"),
        &["  nothing was written; this is a refusal, not a partial result".to_string()],
        json!({ "field": field }),
    ))
}

fn run_memory_propose(output: &Output, path: &Path, arguments: MemoryProposeArguments) -> i32 {
    const COMMAND: &str = "memory propose";
    let scope = match parse_memory_scope(&arguments.scope) {
        Ok(scope) => scope,
        Err(message) => return output.failure(COMMAND, exit::USAGE, &message),
    };
    if let Some(code) = refuse_empty(
        output,
        COMMAND,
        "--content",
        &arguments.content,
        "a note that says nothing is not a note",
    ) {
        return code;
    }
    if let Some(key) = &arguments.claim_key {
        if let Some(code) = refuse_empty(
            output,
            COMMAND,
            "--claim-key",
            key,
            "an empty key would gather every keyless record into one competing claim and report \
             ordinary notes as contradictions",
        ) {
            return code;
        }
    }

    let (opened, repo_id) = match open_registry(output, COMMAND, path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let anchor = match memory_anchor(output, COMMAND, &opened.conn, &repo_id) {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    let resolution = match resolve_one(output, COMMAND, &opened.conn, "subject", &arguments.subject)
    {
        Ok(resolution) => resolution,
        Err(code) => return code,
    };

    let entity = &resolution.entity;
    let row = nerve_store::MemoryRow {
        memory_id: arguments.id.unwrap_or_else(generated_memory_id),
        subject: nerve_store::MemorySubject {
            entity_id: entity.entity_id.clone(),
            kind: entity.kind.clone(),
            name: entity.name.clone(),
            // Empty for an entity no path names — the repository itself is not a file, and the
            // column records that honestly rather than inventing a location for it.
            path: entity.repository_path().unwrap_or_default(),
            // Verbatim. Re-deriving it from the fields above would silently rewrite what the human
            // asked about.
            selector: arguments.subject.clone(),
        },
        anchor_state_id: anchor,
        scope: scope.as_str().to_string(),
        claim_key: arguments.claim_key,
        content: arguments.content,
        author_label: arguments
            .author
            .unwrap_or_else(|| DEFAULT_AUTHOR_LABEL.to_string()),
        created_at: String::new(),
        // Ignored by `propose_memory`, which enters the record at `proposed` because that is what
        // proposing is. Stated here rather than left to a default so the two agree in the source.
        status: MemoryStatus::Proposed,
        supersedes_memory_id: None,
        invalidated_at: None,
        invalidation_reason: None,
    };

    let written = match nerve_store::propose_memory(&opened.conn, &repo_id, &row) {
        Ok(written) => written,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    let detail = match memory_detail(output, COMMAND, &opened.conn, &repo_id, &written.memory_id) {
        Ok(detail) => detail,
        Err(code) => return code,
    };
    output.line(format!(
        "Proposed {} in {}",
        inert_text(&written.memory_id),
        opened.root.display()
    ));
    output.line(format!(
        "  confirm it with `nerve memory confirm {}` — nothing treats a proposal as settled",
        inert_text(&written.memory_id)
    ));
    render_memory(output, COMMAND, &opened.root, &[detail]);
    exit::SUCCESS
}

fn run_memory_confirm(output: &Output, path: &Path, memory_id: &str, note: Option<&str>) -> i32 {
    const COMMAND: &str = "memory confirm";
    let (opened, repo_id) = match open_registry(output, COMMAND, path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    if let Err(err) = nerve_store::confirm_memory(&opened.conn, &repo_id, memory_id, note) {
        return output.failure(COMMAND, memory_exit_code(&err), &err.to_string());
    }
    let detail = match memory_detail(output, COMMAND, &opened.conn, &repo_id, memory_id) {
        Ok(detail) => detail,
        Err(code) => return code,
    };
    output.line(format!(
        "Confirmed {} in {}",
        inert_text(memory_id),
        opened.root.display()
    ));
    render_memory(output, COMMAND, &opened.root, &[detail]);
    exit::SUCCESS
}

fn run_memory_supersede(output: &Output, path: &Path, arguments: MemorySupersedeArguments) -> i32 {
    const COMMAND: &str = "memory supersede";
    if let Some(code) = refuse_empty(
        output,
        COMMAND,
        "--content",
        &arguments.content,
        "a replacement that says nothing replaces nothing",
    ) {
        return code;
    }
    let scope = match arguments
        .scope
        .as_deref()
        .map(parse_memory_scope)
        .transpose()
    {
        Ok(scope) => scope,
        Err(message) => return output.failure(COMMAND, exit::USAGE, &message),
    };

    let (opened, repo_id) = match open_registry(output, COMMAND, path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let anchor = match memory_anchor(output, COMMAND, &opened.conn, &repo_id) {
        Ok(anchor) => anchor,
        Err(code) => return code,
    };
    // Read first, because the successor inherits the predecessor's subject: a replacement about a
    // different subject would not be a replacement. The store reads it again inside its own
    // transaction and refuses on what it finds there, so this read decides nothing.
    let predecessor = match nerve_store::memory(&opened.conn, &repo_id, &arguments.predecessor_id) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return output.failure_detail(
                COMMAND,
                exit::USAGE,
                &format!(
                    "no memory record {} in this repository",
                    inert_text(&arguments.predecessor_id)
                ),
                &["  nothing was written; this is a refusal, not a partial result".to_string()],
                json!({ "memory_id": inert_text(&arguments.predecessor_id) }),
            )
        }
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };

    let successor = nerve_store::MemoryRow {
        memory_id: arguments.id.unwrap_or_else(generated_memory_id),
        subject: predecessor.subject.clone(),
        anchor_state_id: anchor,
        scope: scope
            .map(|scope| scope.as_str().to_string())
            .unwrap_or_else(|| predecessor.scope.clone()),
        claim_key: arguments
            .claim_key
            .or_else(|| predecessor.claim_key.clone()),
        content: arguments.content,
        author_label: arguments
            .author
            .unwrap_or_else(|| DEFAULT_AUTHOR_LABEL.to_string()),
        created_at: String::new(),
        // The successor enters `proposed`. `confirm_memory` is the only door into `active`, and a
        // record inserted straight into it would be confirmed with nothing in its history saying
        // so.
        status: MemoryStatus::Proposed,
        supersedes_memory_id: Some(arguments.predecessor_id.clone()),
        invalidated_at: None,
        invalidation_reason: None,
    };

    let written = match nerve_store::supersede_memory(
        &opened.conn,
        &repo_id,
        &successor,
        nerve_core::vocab::MemoryOperation::Superseded,
        arguments.note.as_deref(),
    ) {
        Ok(written) => written,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };

    // Both records are shown, in the order the change happened: what was retired, then what
    // replaced it. Printing only the successor would leave the caller to check for themselves that
    // the predecessor is intact, which is the property this verb is most often distrusted about.
    let mut reports = Vec::new();
    for id in [&arguments.predecessor_id, &written.memory_id] {
        match nerve_store::read_memory(&opened.conn, &repo_id, id) {
            // The write committed, so both rows are there. A `None` here would mean the database
            // moved under the transaction that just returned, which is reported rather than
            // quietly leaving one of the two records out of the answer.
            Ok(Some(report)) => reports.push(report),
            Ok(None) => {
                return output.failure(
                    COMMAND,
                    exit::INTERNAL,
                    &format!("{} was written and could not be read back", inert_text(id)),
                )
            }
            Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
        }
    }
    let details = match memory_details(&opened.conn, &repo_id, reports) {
        Ok(details) => details,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    output.line(format!(
        "Superseded {} with {} in {}",
        inert_text(&arguments.predecessor_id),
        inert_text(&written.memory_id),
        opened.root.display()
    ));
    output.line(format!(
        "  confirm it with `nerve memory confirm {}` — the successor enters as a proposal, like \
         every other note",
        inert_text(&written.memory_id)
    ));
    render_memory(output, COMMAND, &opened.root, &details);
    exit::SUCCESS
}

fn run_memory_invalidate(
    output: &Output,
    path: &Path,
    memory_id: &str,
    reason: Option<&str>,
    note: Option<&str>,
) -> i32 {
    const COMMAND: &str = "memory invalidate";
    let (opened, repo_id) = match open_registry(output, COMMAND, path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    if let Err(err) =
        nerve_store::invalidate_memory(&opened.conn, &repo_id, memory_id, reason, note)
    {
        return output.failure(COMMAND, memory_exit_code(&err), &err.to_string());
    }
    let detail = match memory_detail(output, COMMAND, &opened.conn, &repo_id, memory_id) {
        Ok(detail) => detail,
        Err(code) => return code,
    };
    output.line(format!(
        "Invalidated {} in {} — it stopped being true and nothing replaced it",
        inert_text(memory_id),
        opened.root.display()
    ));
    render_memory(output, COMMAND, &opened.root, &[detail]);
    exit::SUCCESS
}

/// Refuse a `--file` that is not a repository-relative path.
///
/// An absolute path would put one machine's directory layout inside a record meant to be exported
/// and read elsewhere, and a `..` segment names something outside the repository the citation
/// claims to be about. Neither is narrowed to a guess.
fn check_cited_path(output: &Output, command: &'static str, file: &str) -> Option<i32> {
    let refusal = if file.trim().is_empty() {
        Some("a citation with no place is not a citation")
    } else if Path::new(file).is_absolute() || file.starts_with('/') {
        Some(
            "a citation is repository-relative; an absolute path records one machine's layout \
             inside a record meant to outlive it",
        )
    } else if Path::new(file)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        Some("a `..` segment names something outside the repository this record is about")
    } else {
        None
    }?;
    Some(output.failure_detail(
        command,
        exit::USAGE,
        &format!("{} is refused: {refusal}", inert_text(file)),
        &["  nothing was written; this is a refusal, not a partial result".to_string()],
        json!({ "file": inert_text(file), "reason": refusal }),
    ))
}

/// Parse `--span`, which is `START:END` and nothing else.
fn parse_cited_span(span: &str) -> Result<String, String> {
    let malformed =
        || format!("--span {span:?} is not START:END, where both are line numbers from 1");
    let (start, end) = span.split_once(':').ok_or_else(malformed)?;
    let start: u32 = start.trim().parse().map_err(|_| malformed())?;
    let end: u32 = end.trim().parse().map_err(|_| malformed())?;
    if start == 0 || end < start {
        return Err(format!(
            "--span {span:?} does not name a range: lines are numbered from 1 and the end may not \
             precede the start"
        ));
    }
    Ok(format!("{start}:{end}"))
}

fn run_memory_cite(
    output: &Output,
    path: &Path,
    memory_id: &str,
    file: &str,
    span: Option<&str>,
    note: Option<&str>,
) -> i32 {
    const COMMAND: &str = "memory cite";
    if let Some(code) = check_cited_path(output, COMMAND, file) {
        return code;
    }
    let span = match span.map(parse_cited_span).transpose() {
        Ok(span) => span,
        Err(message) => return output.failure(COMMAND, exit::USAGE, &message),
    };

    let (opened, repo_id) = match open_registry(output, COMMAND, path, true) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let state = match memory_anchor(output, COMMAND, &opened.conn, &repo_id) {
        Ok(state) => state,
        Err(code) => return code,
    };

    // The cited entity is deliberately left unnamed. Slice 14a ships the citation's durable
    // snapshot and defers its resolution verdict, so naming an entity here would be a claim no
    // reader could re-check — a path and a span are what the human pointed at, and they are what is
    // recorded.
    let citation = nerve_store::MemoryCitationRow {
        citation_id: None,
        memory_id: memory_id.to_string(),
        cited_entity_id: None,
        cited_kind: None,
        cited_name: None,
        cited_path: file.to_string(),
        cited_span: span,
        cited_at_state: state,
        created_at: String::new(),
    };
    let citation_id = match nerve_store::cite_memory(&opened.conn, &repo_id, &citation, note) {
        Ok(citation_id) => citation_id,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    let detail = match memory_detail(output, COMMAND, &opened.conn, &repo_id, memory_id) {
        Ok(detail) => detail,
        Err(code) => return code,
    };
    output.line(format!(
        "Cited {} on {} — a citation changes no status",
        inert_text(file),
        inert_text(memory_id)
    ));
    output.line(format!("  citation_id    #{citation_id}"));
    render_memory(output, COMMAND, &opened.root, &[detail]);
    exit::SUCCESS
}

fn run_memory_list(output: &Output, path: &Path, arguments: MemoryListArguments) -> i32 {
    const COMMAND: &str = "memory list";
    let scope = match arguments
        .scope
        .as_deref()
        .map(parse_memory_scope)
        .transpose()
    {
        Ok(scope) => scope,
        Err(message) => return output.failure(COMMAND, exit::USAGE, &message),
    };
    let status = match arguments
        .status
        .as_deref()
        .map(parse_memory_status)
        .transpose()
    {
        Ok(status) => status,
        Err(message) => return output.failure(COMMAND, exit::USAGE, &message),
    };

    let (opened, repo_id) = match open_registry(output, COMMAND, path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let subject = match arguments.subject.as_deref() {
        Some(selector) => match resolve_one(output, COMMAND, &opened.conn, "subject", selector) {
            Ok(resolution) => Some(resolution.entity.entity_id),
            Err(code) => return code,
        },
        None => None,
    };

    let reports = match (&subject, scope) {
        (Some(entity_id), _) => {
            nerve_store::read_memory_for_subject(&opened.conn, &repo_id, entity_id)
        }
        (None, Some(scope)) => {
            nerve_store::read_memory_in_scope(&opened.conn, &repo_id, scope.as_str())
        }
        (None, None) => nerve_store::read_memory_all(&opened.conn, &repo_id),
    };
    let mut reports = match reports {
        Ok(reports) => reports,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    // The remaining filters are applied over the rows the store returned. Filtering a list is
    // rendering; the queries themselves stay in `nerve-store`.
    if let Some(scope) = scope {
        reports.retain(|report| report.row.scope == scope.as_str());
    }
    if let Some(status) = status {
        reports.retain(|report| report.row.status == status);
    }

    let details = match memory_details(&opened.conn, &repo_id, reports) {
        Ok(details) => details,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    if details.is_empty() {
        output.line(format!(
            "No memory records in {} matching this question",
            opened.root.display()
        ));
        output.line(
            "  memory         empty — nothing is written here except by `nerve memory propose`; \
             this is an absence, and every filter above was accepted",
        );
    } else {
        output.line(format!(
            "{} memory record(s) in {}",
            details.len(),
            opened.root.display()
        ));
    }
    render_memory(output, COMMAND, &opened.root, &details);
    exit::SUCCESS
}

fn run_memory_show(output: &Output, path: &Path, memory_id: &str) -> i32 {
    const COMMAND: &str = "memory show";
    let (opened, repo_id) = match open_registry(output, COMMAND, path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let detail = match memory_detail(output, COMMAND, &opened.conn, &repo_id, memory_id) {
        Ok(detail) => detail,
        Err(code) => return code,
    };
    render_memory(output, COMMAND, &opened.root, &[detail]);
    exit::SUCCESS
}

fn run_memory_search(output: &Output, path: &Path, query: &str) -> i32 {
    const COMMAND: &str = "memory search";
    let (opened, repo_id) = match open_registry(output, COMMAND, path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let reports = match nerve_store::search_memory(&opened.conn, &repo_id, query) {
        Ok(reports) => reports,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    let details = match memory_details(&opened.conn, &repo_id, reports) {
        Ok(details) => details,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    if details.is_empty() {
        output.line(format!(
            "No memory record in {} contains {}",
            opened.root.display(),
            inert_text(query)
        ));
        output.line(
            "  search         literal substring over content and claim key; the subject snapshot \
             is not searched, so a path finds nothing here by design",
        );
    } else {
        output.line(format!(
            "{} memory record(s) contain {}",
            details.len(),
            inert_text(query)
        ));
    }
    render_memory(output, COMMAND, &opened.root, &details);
    exit::SUCCESS
}

fn run_memory_events(output: &Output, path: &Path, memory_id: &str) -> i32 {
    const COMMAND: &str = "memory events";
    let (opened, repo_id) = match open_registry(output, COMMAND, path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let detail = match memory_detail(output, COMMAND, &opened.conn, &repo_id, memory_id) {
        Ok(detail) => detail,
        Err(code) => return code,
    };
    output.line(format!(
        "{} event(s) for {} — every one ever appended, oldest first",
        detail.events.len(),
        inert_text(memory_id)
    ));
    render_memory(output, COMMAND, &opened.root, &[detail]);
    exit::SUCCESS
}

/// The exported form of one record.
///
/// **Stored columns only.** `potentially_stale`, `conflicted`, `multiple_active`, the subject's
/// current resolution and the repository's current state are all query-time verdicts, and exporting
/// one as though it were stored is exactly the confusion the read/stored split exists to prevent: a
/// file saying `potentially_stale` would be a derived value with no query behind it, kept true by
/// nothing.
///
/// `anchor_state_id` **is** here, because it is a column rather than a verdict — and it is the one
/// the staleness of every re-imported record would be re-derived against.
fn memory_export_record(detail: &MemoryDetail) -> serde_json::Value {
    let row = &detail.report.row;
    json!({
        "memory_id": row.memory_id,
        "subject": {
            "entity_id": row.subject.entity_id,
            "kind": row.subject.kind,
            "name": row.subject.name,
            "path": row.subject.path,
            "selector": row.subject.selector,
        },
        "anchor_state_id": row.anchor_state_id,
        "scope": row.scope,
        "claim_key": row.claim_key,
        "content": row.content,
        "author_label": row.author_label,
        "created_at": row.created_at,
        "status": row.status.as_str(),
        "supersedes_memory_id": row.supersedes_memory_id,
        "invalidated_at": row.invalidated_at,
        "invalidation_reason": row.invalidation_reason,
        "citations": detail
            .citations
            .iter()
            .map(|citation| json!({
                "citation_id": citation.citation_id,
                "cited_entity_id": citation.cited_entity_id,
                "cited_kind": citation.cited_kind,
                "cited_name": citation.cited_name,
                "cited_path": citation.cited_path,
                "cited_span": citation.cited_span,
                "cited_at_state": citation.cited_at_state,
                "created_at": citation.created_at,
            }))
            .collect::<Vec<_>>(),
        "events": detail
            .events
            .iter()
            .map(|event| json!({
                "event_id": event.event_id,
                "at": event.at,
                "operation": event.operation.as_str(),
                "from_status": event.from_status.map(|status| status.as_str()),
                "to_status": event.to_status.as_str(),
                "note": event.note,
            }))
            .collect::<Vec<_>>(),
    })
}

/// The whole export, as one document.
///
/// Three omissions, each deliberate and each with a test behind it.
///
/// 1. **No timestamp of the export itself.** The same database must export byte-identically twice,
///    and an `exported_at` field would break exactly that — a determinism claim that fails on the
///    only field nobody needed.
/// 2. **No derived state.** See [`memory_export_record`].
/// 3. **No absolute path.** Every path here is a repository-relative snapshot, so the document says
///    nothing about the machine it was written on and can be read on another.
///
/// Key order is not sorted by hand: `serde_json`'s map is a `BTreeMap` in this build, so keys
/// serialise in sorted order, and a test asserts that rather than trusting it. Records are ordered
/// by `memory_id`, citations by `citation_id` and events by `event_id` — all three of which the
/// store's own ordering already guarantees.
fn memory_export_document(repo_id: &str, details: &[MemoryDetail]) -> serde_json::Value {
    json!({
        "format": "nerve-memory-export",
        "format_version": 1,
        "schema_version": nerve_store::SCHEMA_VERSION,
        "repo_id": repo_id,
        "record_count": details.len(),
        "records": details.iter().map(memory_export_record).collect::<Vec<_>>(),
    })
}

fn run_memory_export(output: &Output, path: &Path, out: Option<&Path>) -> i32 {
    const COMMAND: &str = "memory export";
    let (opened, repo_id) = match open_registry(output, COMMAND, path, false) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let reports = match nerve_store::read_memory_all(&opened.conn, &repo_id) {
        Ok(reports) => reports,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    let details = match memory_details(&opened.conn, &repo_id, reports) {
        Ok(details) => details,
        Err(err) => return output.failure(COMMAND, memory_exit_code(&err), &err.to_string()),
    };
    let document = memory_export_document(&repo_id, &details);
    let Ok(text) = serde_json::to_string_pretty(&document) else {
        return output.failure(
            COMMAND,
            exit::INTERNAL,
            "the export could not be serialised",
        );
    };

    match out {
        Some(file) => {
            if let Err(err) = std::fs::write(file, format!("{text}\n")) {
                return output.failure(
                    COMMAND,
                    exit::INTERNAL,
                    &format!("{}: {err}", file.display()),
                );
            }
            output.line(format!(
                "Exported {} record(s) from {} to {}",
                details.len(),
                opened.root.display(),
                file.display()
            ));
            output.object(json!({
                "command": COMMAND,
                "ok": true,
                "root": opened.root.display().to_string(),
                "out": file.display().to_string(),
                "format": "nerve-memory-export",
                "format_version": 1,
                "schema_version": nerve_store::SCHEMA_VERSION,
                "repo_id": repo_id,
                "record_count": details.len(),
            }));
        }
        // With no `--out` the document **is** the answer, in both modes. There is no second,
        // human-readable rendering of an export: one would be a second format nothing could read
        // back, and the whole point of this command is that its output is exact.
        None => println!("{text}"),
    }
    exit::SUCCESS
}

// ---- graph commands ------------------------------------------------------------------------
//
// These handlers parse arguments, ask `nerve-store` the question, render the answer and map the
// outcome to an exit code. There is no traversal, no evidence assembly and no SQL here; see
// ARCHITECTURE.md invariant 3.

fn entity_json(entity: &nerve_store::EntityRef) -> serde_json::Value {
    json!({
        "entity_id": entity.entity_id,
        "kind": entity.kind,
        "name": entity.name,
        "scope_path": entity.scope_path,
        "qualified_name": entity.qualified_name(),
        "language": entity.language,
        "file_path": entity.file_path,
        "start_line": entity.start_line,
        "end_line": entity.end_line,
    })
}

fn entity_line(entity: &nerve_store::EntityRef) -> String {
    format!(
        "{:<10} {:<34} {}",
        entity.kind,
        entity.qualified_name(),
        entity.location()
    )
}

/// Parse `--relation` values against the closed relation vocabulary.
fn parse_relations(values: &[String]) -> Result<Vec<Relation>, String> {
    let mut relations = Vec::new();
    for value in values {
        match value.parse::<Relation>() {
            Ok(relation) => {
                if !relations.contains(&relation) {
                    relations.push(relation);
                }
            }
            Err(_) => {
                let allowed = Relation::ALL
                    .iter()
                    .map(|relation| relation.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "unknown --relation {value:?}; expected one of: {allowed}"
                ));
            }
        }
    }
    Ok(relations)
}

fn relation_names(relations: &[Relation]) -> Vec<&'static str> {
    relations.iter().map(|relation| relation.as_str()).collect()
}

/// One selector, resolved, and what a stated rule passed over on the way.
struct Resolution {
    role: String,
    selector: String,
    entity: nerve_store::EntityRef,
    matched_by: nerve_store::SelectorKind,
    alternatives: Vec<nerve_store::EntityRef>,
}

impl Resolution {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "role": self.role,
            "selector": self.selector,
            "matched_by": self.matched_by.as_str(),
            "alternatives": self.alternatives.iter().map(entity_json).collect::<Vec<_>>(),
        })
    }
}

fn selectors_json(resolutions: &[&Resolution]) -> serde_json::Value {
    json!(resolutions
        .iter()
        .map(|resolution| resolution.to_json())
        .collect::<Vec<_>>())
}

/// `nerve why`'s optional second selector, without building a `Vec` at the call site.
fn why_selectors_json(subject: &Resolution, object: Option<&Resolution>) -> serde_json::Value {
    match object {
        Some(object) => selectors_json(&[subject, object]),
        None => selectors_json(&[subject]),
    }
}

/// Resolve one selector, or render the refusal and return its exit code.
///
/// Ambiguity is exit 10 with the candidate list; nothing is chosen on the user's behalf. So are
/// the two refusals Slice 8b-i separates out of "matches no indexed entity": a **malformed**
/// selector and a **refused** traversal-shaped one are wrong command lines, which is what
/// [`exit::USAGE`] means and what an ambiguous selector already returned. `NotFound` keeps
/// [`exit::NO_INDEX`] unchanged — it is a stretch, it is pre-existing, and correcting it is a
/// slice of its own rather than a side effect of this one.
///
/// A resolution that passed over a second reading says so on the spot, before the answer, rather
/// than leaving the caller to infer from silence that `src/app.ts` had two readings.
fn resolve_one(
    output: &Output,
    command: &str,
    conn: &nerve_store::Connection,
    role: &str,
    selector: &str,
) -> Result<Resolution, i32> {
    match nerve_store::resolve_selector(conn, selector) {
        Ok(nerve_store::Selection::Resolved {
            entity,
            matched_by,
            alternatives,
        }) => {
            for passed_over in &alternatives {
                let addressable = passed_over
                    .repository_path()
                    .map(|path| format!("{}:{path}", passed_over.kind))
                    .unwrap_or_else(|| passed_over.entity_id.clone());
                output.line(format!(
                    "note     {selector:?} ({role}) named the {}; the {} at that path is \
                     also indexed, as {addressable}",
                    entity.kind, passed_over.kind
                ));
            }
            Ok(Resolution {
                role: role.to_string(),
                selector: selector.to_string(),
                entity: *entity,
                matched_by,
                alternatives,
            })
        }
        Ok(nerve_store::Selection::Refused { reason }) => Err(output.failure_detail(
            command,
            exit::USAGE,
            &format!("{selector:?} ({role}) is refused: {}", reason.statement()),
            &["  nothing was looked up; this is a refusal, not an absence".to_string()],
            json!({
                "selector": selector,
                "selector_role": role,
                "reason": reason.as_str(),
            }),
        )),
        Ok(nerve_store::Selection::Invalid { reason }) => {
            let accepted = nerve_store::qualifiers().join(", ");
            Err(output.failure_detail(
                command,
                exit::USAGE,
                &format!(
                    "{selector:?} ({role}) is not a selector ({})",
                    reason.as_str()
                ),
                &[format!("  qualifiers: {accepted}")],
                json!({
                    "selector": selector,
                    "selector_role": role,
                    "reason": reason.as_str(),
                    "accepted_qualifiers": nerve_store::qualifiers(),
                }),
            ))
        }
        Ok(nerve_store::Selection::Ambiguous {
            candidates,
            matched_by,
        }) => {
            let message = format!(
                "{selector:?} ({role}) matches {} entities; nothing was chosen — repeat the \
                 command with one of these ids, or a path#name selector",
                candidates.len()
            );
            let lines: Vec<String> = candidates
                .iter()
                .map(|candidate| format!("  {}  {}", entity_line(candidate), candidate.entity_id))
                .collect();
            Err(output.failure_detail(
                command,
                exit::USAGE,
                &message,
                &lines,
                json!({
                    "selector": selector,
                    "selector_role": role,
                    "matched_by": matched_by.as_str(),
                    "candidates": candidates.iter().map(entity_json).collect::<Vec<_>>(),
                }),
            ))
        }
        Ok(nerve_store::Selection::NotFound {
            qualifier,
            excluded,
            suggestions,
        }) => {
            let message = match qualifier {
                Some(qualifier) => format!(
                    "{selector:?} ({role}) matches no indexed {} entity",
                    qualifier.as_str()
                ),
                None => format!("{selector:?} ({role}) matches no indexed entity"),
            };
            let mut lines = Vec::new();
            // What the qualifier ruled out, so the refusal is "no module there — there is a
            // document" rather than a bare miss the caller has to go and disprove.
            if !excluded.is_empty() {
                lines.push("  the selector does name these, which the qualifier excluded:".into());
                for entity in &excluded {
                    lines.push(format!("    {}  {}", entity_line(entity), entity.entity_id));
                }
            }
            if suggestions.is_empty() {
                lines.push("  no near matches; try `nerve search`".to_string());
            } else {
                lines.push("  did you mean:".to_string());
                for hit in &suggestions {
                    lines.push(format!(
                        "    {:<10} {:<34} {}",
                        hit.kind,
                        hit.qualified_name(),
                        hit.location()
                    ));
                }
            }
            Err(output.failure_detail(
                command,
                exit::NO_INDEX,
                &message,
                &lines,
                json!({
                    "selector": selector,
                    "selector_role": role,
                    "qualifier": qualifier.map(nerve_store::Qualifier::as_str),
                    "excluded": excluded.iter().map(entity_json).collect::<Vec<_>>(),
                    "suggestions": suggestions.iter().map(|hit| json!({
                        "entity_id": hit.entity_id,
                        "kind": hit.kind,
                        "name": hit.name,
                        "scope_path": hit.scope_path,
                        "qualified_name": hit.qualified_name(),
                        "file_path": hit.file_path,
                        "start_line": hit.start_line,
                    })).collect::<Vec<_>>(),
                }),
            ))
        }
        Err(err) => Err(output.failure(command, exit::INTERNAL, &err.to_string())),
    }
}

/// Everything `nerve path` was asked for.
struct PathArguments {
    from: String,
    to: String,
    max_depth: usize,
    relations: Vec<String>,
    limit: usize,
    direction: DirectionArg,
    resolved_only: bool,
}

/// Largest `--max-depth` accepted. Beyond this the answer stops being a useful explanation.
const MAX_DEPTH_CEILING: usize = 32;

fn run_path(output: &Output, path: &Path, arguments: PathArguments) -> i32 {
    if arguments.max_depth == 0 || arguments.max_depth > MAX_DEPTH_CEILING {
        return output.failure(
            "path",
            exit::USAGE,
            &format!("--max-depth must be between 1 and {MAX_DEPTH_CEILING}"),
        );
    }
    if arguments.limit == 0 {
        return output.failure("path", exit::USAGE, "--limit must be at least 1");
    }
    let relations = match parse_relations(&arguments.relations) {
        Ok(relations) => relations,
        Err(message) => return output.failure("path", exit::USAGE, &message),
    };

    let conn = match open_existing(path) {
        Ok(opened) => opened.conn,
        Err(message) => return output.failure("path", exit::NO_INDEX, &message),
    };

    let from = match resolve_one(output, "path", &conn, "from", &arguments.from) {
        Ok(resolution) => resolution,
        Err(code) => return code,
    };
    let to = match resolve_one(output, "path", &conn, "to", &arguments.to) {
        Ok(resolution) => resolution,
        Err(code) => return code,
    };

    let query = nerve_store::PathQuery {
        max_depth: arguments.max_depth,
        limit: arguments.limit,
        direction: arguments.direction.to_store(),
        relations: relations.clone(),
        resolved_only: arguments.resolved_only,
    };
    let report = match nerve_store::find_paths(
        &conn,
        &from.entity.entity_id,
        &to.entity.entity_id,
        &query,
    ) {
        Ok(report) => report,
        Err(err) => return output.failure("path", exit::INTERNAL, &err.to_string()),
    };

    output.line(format!("from  {}", entity_line(&report.from)));
    output.line(format!("to    {}", entity_line(&report.to)));
    output.line(String::new());

    if report.paths.is_empty() {
        output.line(format!(
            "No path found within depth {} ({}, {} partial path(s) explored).",
            report.max_depth,
            query.direction.as_str(),
            report.expansions
        ));
        if report.truncated {
            output.line(
                "  The search budget stopped the walk before the graph was exhausted, so a \
                 longer path may exist.",
            );
        }
    }

    for (index, graph_path) in report.paths.iter().enumerate() {
        output.line(format!(
            "path {} of {} · {} hop(s){}",
            index + 1,
            report.paths.len(),
            graph_path.length(),
            if graph_path.traverses_unresolved() {
                " · traverses an unresolved edge"
            } else {
                ""
            }
        ));
        output.line(format!("  {}", entity_line(&report.from)));
        for hop in &graph_path.hops {
            let arrow = if hop.traversed_backwards {
                format!("<-[{}]-", hop.relation)
            } else {
                format!("-[{}]->", hop.relation)
            };
            output.line(format!(
                "    {:<20} {:<14} {} obs  {}{}",
                arrow,
                hop.strongest_source_type.as_deref().unwrap_or("-"),
                hop.observation_count,
                hop.location(),
                if hop.is_unresolved {
                    "  [unresolved]"
                } else {
                    ""
                }
            ));
            output.line(format!("  {}", entity_line(&hop.to)));
        }
        output.line(String::new());
    }

    if report.truncated && !report.paths.is_empty() {
        output.line("Search budget reached; there may be further paths.");
    }

    output.object(json!({
        "command": "path",
        "ok": true,
        "exit_code": exit::SUCCESS,
        "selectors": selectors_json(&[&from, &to]),
        "from": entity_json(&report.from),
        "to": entity_json(&report.to),
        "max_depth": report.max_depth,
        "limit": query.limit,
        "direction": query.direction.as_str(),
        "relations": relation_names(&relations),
        "resolved_only": query.resolved_only,
        "truncated": report.truncated,
        "expansions": report.expansions,
        "count": report.paths.len(),
        "paths": report.paths.iter().map(|graph_path| json!({
            "length": graph_path.length(),
            "traverses_unresolved": graph_path.traverses_unresolved(),
            "hops": graph_path.hops.iter().map(|hop| json!({
                "relation": hop.relation,
                "assertion_id": hop.assertion_id,
                "from": entity_json(&hop.from),
                "to": entity_json(&hop.to),
                "traversed_backwards": hop.traversed_backwards,
                "is_unresolved": hop.is_unresolved,
                "status": hop.status,
                "strongest_source_type": hop.strongest_source_type,
                "observation_count": hop.observation_count,
                "file_path": hop.file_path,
                "start_line": hop.start_line,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }));

    exit::SUCCESS
}

// ---- impact ----------------------------------------------------------------------------------

/// Everything `nerve impact` was asked for, minus the repository.
struct ImpactArguments {
    selector: String,
    max_depth: usize,
    relations: Vec<String>,
    limit: usize,
}

/// A tally rendered as `name N · name N`, or `-` when there is nothing in it.
fn tally_line<K: std::fmt::Display>(tally: &std::collections::BTreeMap<K, usize>) -> String {
    if tally.is_empty() {
        return "-".to_string();
    }
    tally
        .iter()
        .map(|(key, count)| format!("{key} {count}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Report what depends on a symbol, and state what the answer cannot see.
///
/// Exit 0 in every successful case, including the empty one. "Nothing resolved depends on this"
/// is a finding, not an error (Slice 2b), and it is the finding that most needs the unresolved
/// caveat attached — which is why the caveat is printed unconditionally, before the exit code is
/// ever reached, whether or not it has anything to report.
///
/// This is not `nerve affected`. That command is refused, not deferred: LCOV carries no per-test
/// attribution (ADR-0008 §A.2). Nothing here is test attribution, and a test file in an impact
/// set is there because code depends on code.
fn run_impact(output: &Output, path: &Path, arguments: ImpactArguments) -> i32 {
    if arguments.max_depth == 0 || arguments.max_depth > MAX_DEPTH_CEILING {
        return output.failure(
            "impact",
            exit::USAGE,
            &format!("--max-depth must be between 1 and {MAX_DEPTH_CEILING}"),
        );
    }
    if arguments.limit == 0 {
        return output.failure("impact", exit::USAGE, "--limit must be at least 1");
    }
    let relations = match parse_relations(&arguments.relations) {
        Ok(relations) => relations,
        Err(message) => return output.failure("impact", exit::USAGE, &message),
    };

    let OpenIndex { root, conn, .. } = match open_existing(path) {
        Ok(opened) => opened,
        Err(message) => return output.failure("impact", exit::NO_INDEX, &message),
    };
    // Freshness is computed by re-reading the repository, so the reader is built from the
    // repository root and enforces the Slice 1 path rules on every path the database supplies.
    let prober = match nerve_index::RepositoryProber::new(&root) {
        Ok(prober) => prober,
        Err(err) => return output.failure("impact", error_exit_code(&err), &err.to_string()),
    };

    let subject = match resolve_one(output, "impact", &conn, "selector", &arguments.selector) {
        Ok(resolution) => resolution,
        Err(code) => return code,
    };

    let query = nerve_store::ImpactQuery {
        max_depth: arguments.max_depth,
        limit: arguments.limit,
        relations,
    };
    let report = match nerve_store::impact(&conn, &subject.entity.entity_id, &query, &prober) {
        Ok(report) => report,
        Err(err) => return output.failure("impact", exit::INTERNAL, &err.to_string()),
    };
    let walked = relation_names(&report.relations);

    output.line(format!("subject  {}", entity_line(&report.subject)));
    output.line(format!("  relations      {}", walked.join(", ")));
    output.line(format!("  max-depth      {}", report.max_depth));
    output.line(format!(
        "  entities       {} depend on this, transitively",
        report.totals.entities
    ));
    output.line(format!(
        "  by depth       {}",
        tally_line(&report.totals.by_depth)
    ));
    output.line(format!(
        "  by relation    {}",
        tally_line(&report.totals.by_relation)
    ));
    output.line(format!(
        "  by kind        {}",
        tally_line(&report.totals.by_kind)
    ));
    output.line(format!(
        "  stale          {} reached through evidence that no longer matches its file",
        report.totals.stale
    ));
    output.line(format!("  files re-hashed {}", report.files_probed));
    output.line(String::new());

    if report.results.is_empty() {
        output.line(format!(
            "Nothing in the index depends on this through {} within depth {}.",
            walked.join(", "),
            report.max_depth
        ));
    }
    for row in &report.results {
        output.line(format!(
            "{:<5} {:<11} {:<10} {:<32} {:<24} {:<14} {}",
            row.depth,
            row.relation,
            row.entity.kind,
            row.entity.qualified_name(),
            row.entity.location(),
            row.strongest_source_type.as_deref().unwrap_or("-"),
            row.evidence_freshness
                .map(|freshness| freshness.as_str())
                .unwrap_or("-")
        ));
        output.line(format!(
            "      {} {} obs at {}{}",
            row.direction.as_str(),
            row.observation_count,
            row.location(),
            if row.is_unresolved {
                "  [unresolved]"
            } else {
                ""
            }
        ));
    }
    if report.truncated {
        output.line(String::new());
        output.line(format!(
            "Listed {} of {} dependent entities; raise --limit to see the rest. The tallies \
             above are exact.",
            report.results.len(),
            report.results_total
        ));
    }

    // Printed whether or not it has anything to report. A small impact set with no caveat reads
    // as "safe to change", and on a repository where a third of the reference sites resolve to
    // nothing that reading is unsupported.
    output.line(String::new());
    let account = &report.unresolved;
    output.line(format!(
        "  unresolved     {} reference site(s) in this repository resolved to nothing",
        account.sites
    ));
    output.line(format!("                 over {}", walked.join(", ")));
    if account.is_empty() {
        // Not "every reference site resolved". Zero **failed** resolutions is what is measured,
        // and in a repository that indexed no reference site at all the old wording was
        // vacuously true while reading as a coverage claim — which is the one thing this block
        // exists to avoid. A construct Nerve declines to model, or a language whose reference
        // extractor does not exist yet, contributes nothing to either side of the count.
        output.line("  No reference site under those relations failed to resolve. That is a count");
        output.line("  of failed resolutions, not of coverage: a construct Nerve does not model —");
        output.line("  or a language it does not yet resolve — contributes no site to count.");
    } else {
        output.line(format!(
            "                 {} assertion(s), {} distinct target(s) · {}",
            account.assertions,
            account.targets,
            tally_line(&account.by_category)
        ));
        output.line("  Any of them could reach this symbol, and this answer cannot rule them out.");
        output.line("  Nerve has no type inference, so a method call on a typed receiver is");
        output.line("  recorded as unresolved rather than guessed at.");
        output.line("  This is not a list of suspects: matching an unresolved name against this");
        output.line("  symbol's name would be identity by coincidence, which Nerve does not do.");
    }

    output.object(json!({
        "command": "impact",
        "ok": true,
        "exit_code": exit::SUCCESS,
        "selectors": selectors_json(&[&subject]),
        "root": root.display().to_string(),
        "subject": entity_json(&report.subject),
        "relations": walked,
        "max_depth": report.max_depth,
        "limit": report.limit,
        "totals": {
            "entities": report.totals.entities,
            // An array rather than an object: JSON object keys are strings, and `"10"` sorting
            // before `"2"` would put the tally in an order no reader expects.
            "by_depth": report.totals.by_depth.iter().map(|(depth, count)| json!({
                "depth": depth,
                "entities": count,
            })).collect::<Vec<_>>(),
            "by_relation": report.totals.by_relation,
            "by_kind": report.totals.by_kind,
            "stale": report.totals.stale,
        },
        "unresolved": {
            "sites": account.sites,
            "assertions": account.assertions,
            "targets": account.targets,
            "by_category": account.by_category,
        },
        "count": report.results.len(),
        "results_total": report.results_total,
        "truncated": report.truncated,
        "files_probed": report.files_probed,
        "results": report.results.iter().map(|row| json!({
            "entity": entity_json(&row.entity),
            "depth": row.depth,
            "relation": row.relation,
            "direction": row.direction.as_str(),
            "reached_entity_id": row.reached_entity_id,
            "assertion_id": row.assertion_id,
            "status": row.status,
            "strongest_source_type": row.strongest_source_type,
            "observation_count": row.observation_count,
            "is_unresolved": row.is_unresolved,
            "file_path": row.file_path,
            "start_line": row.start_line,
            "evidence_freshness": row.evidence_freshness.map(|freshness| freshness.as_str()),
        })).collect::<Vec<_>>(),
    }));

    exit::SUCCESS
}

/// Everything `nerve why` was asked for.
struct WhyArguments {
    from: String,
    to: Option<String>,
    incoming: bool,
    outgoing: bool,
    relations: Vec<String>,
}

fn run_why(output: &Output, path: &Path, arguments: WhyArguments) -> i32 {
    let relations = match parse_relations(&arguments.relations) {
        Ok(relations) => relations,
        Err(message) => return output.failure("why", exit::USAGE, &message),
    };
    let direction = match (arguments.incoming, arguments.outgoing) {
        (true, false) => nerve_store::WhyDirection::Incoming,
        (false, true) => nerve_store::WhyDirection::Outgoing,
        _ => nerve_store::WhyDirection::Both,
    };

    let OpenIndex { root, conn, .. } = match open_existing(path) {
        Ok(opened) => opened,
        Err(message) => return output.failure("why", exit::NO_INDEX, &message),
    };
    // Freshness is computed by re-reading the repository, so the reader is built from the
    // repository root and enforces the Slice 1 path rules on every path the database supplies.
    let prober = match nerve_index::RepositoryProber::new(&root) {
        Ok(prober) => prober,
        Err(err) => return output.failure("why", error_exit_code(&err), &err.to_string()),
    };

    let subject = match resolve_one(output, "why", &conn, "from", &arguments.from) {
        Ok(resolution) => resolution,
        Err(code) => return code,
    };
    let object = match &arguments.to {
        Some(selector) => match resolve_one(output, "why", &conn, "to", selector) {
            Ok(resolution) => Some(resolution),
            Err(code) => return code,
        },
        None => None,
    };

    let query = nerve_store::WhyQuery {
        direction,
        relations: relations.clone(),
    };
    let report = match nerve_store::explain(
        &conn,
        &subject.entity.entity_id,
        object
            .as_ref()
            .map(|resolution| resolution.entity.entity_id.as_str()),
        &query,
        &prober,
    ) {
        Ok(report) => report,
        Err(err) => return output.failure("why", exit::INTERNAL, &err.to_string()),
    };

    output.line(format!("subject  {}", entity_line(&report.subject)));
    if let Some(object) = &report.object {
        output.line(format!("object   {}", entity_line(object)));
    }
    output.line(format!(
        "assertions {} · files re-hashed {}",
        report.assertions.len(),
        report.files_probed
    ));
    output.line(String::new());

    if report.assertions.is_empty() {
        output.line("No assertion matches that question.");
    }

    for (index, assertion) in report.assertions.iter().enumerate() {
        let other = match assertion.direction {
            nerve_store::EdgeDirection::Outgoing => &assertion.target,
            nerve_store::EdgeDirection::Incoming => &assertion.source,
        };
        let arrow = match assertion.direction {
            nerve_store::EdgeDirection::Outgoing => "->",
            nerve_store::EdgeDirection::Incoming => "<-",
        };
        output.line(format!(
            "{}. {} {} {}",
            index + 1,
            assertion.relation,
            arrow,
            entity_line(other)
        ));
        output.line(format!(
            "   status {} · unresolved {} · observations {} · strongest {}",
            assertion.status.as_deref().unwrap_or("(none)"),
            if assertion.is_unresolved { "yes" } else { "no" },
            assertion.observation_count,
            assertion.strongest_source_type.as_deref().unwrap_or("-")
        ));
        for observation in &assertion.observations {
            output.line(format!(
                "   - {} / {}  {} {}  {}  freshness {}",
                observation.evidence_source_type,
                observation.directness,
                observation.extractor_id,
                observation.extractor_version,
                observation.location(),
                observation.freshness
            ));
            output.line(format!(
                "     environment {} · match_quality {} · state {}",
                observation.environment.as_deref().unwrap_or("-"),
                observation
                    .match_quality
                    .map(|quality| quality.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                observation.state_id
            ));
            output.line(format!(
                "     details {}",
                observation.details.as_deref().unwrap_or("-")
            ));
        }
        output.line(String::new());
    }

    output.object(json!({
        "command": "why",
        "ok": true,
        "exit_code": exit::SUCCESS,
        "selectors": why_selectors_json(&subject, object.as_ref()),
        "subject": entity_json(&report.subject),
        "object": report.object.as_ref().map(entity_json),
        "direction": query.direction.as_str(),
        "relations": relation_names(&relations),
        "files_probed": report.files_probed,
        "count": report.assertions.len(),
        "assertions": report.assertions.iter().map(|assertion| json!({
            "assertion_id": assertion.assertion_id,
            "relation": assertion.relation,
            "direction": assertion.direction.as_str(),
            "source": entity_json(&assertion.source),
            "target": entity_json(&assertion.target),
            "status": assertion.status,
            "is_unresolved": assertion.is_unresolved,
            "observation_count": assertion.observation_count,
            "strongest_source_type": assertion.strongest_source_type,
            "observations": assertion.observations.iter().map(|observation| json!({
                "observation_id": observation.observation_id,
                "evidence_source_type": observation.evidence_source_type,
                "directness": observation.directness,
                "extractor_id": observation.extractor_id,
                "extractor_version": observation.extractor_version,
                "match_quality": observation.match_quality,
                "state_id": observation.state_id,
                "file_path": observation.file_path,
                "start_line": observation.start_line,
                "end_line": observation.end_line,
                "content_hash": observation.content_hash,
                "environment": observation.environment,
                "details": observation.details.as_deref().map(details_json),
                "created_at": observation.created_at,
                "freshness": observation.freshness.as_str(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }));

    exit::SUCCESS
}

/// Render a stored `details` blob as JSON when it parses, as a string when it does not.
fn details_json(details: &str) -> serde_json::Value {
    serde_json::from_str(details).unwrap_or_else(|_| json!(details))
}

// ---- serve ---------------------------------------------------------------------------------

fn serve_exit_code(err: &nerve_server::ServerError) -> i32 {
    match err {
        nerve_server::ServerError::NoSuchRoot(_) => exit::USAGE,
        nerve_server::ServerError::NotIndexed(_) => exit::NO_INDEX,
        _ => exit::INTERNAL,
    }
}

/// Start the local server, print how to reach it, and block until it is asked to stop.
///
/// The URL carries the session token because the terminal's only channel to a browser is a URL,
/// and without the token nothing — not even the page — is served. The token is minted per run
/// and never written to disk.
fn run_serve(output: &Output, root: PathBuf, port: u16, workers: usize) -> i32 {
    let config = nerve_server::ServeConfig {
        root,
        port,
        workers,
    };
    let server = match nerve_server::serve(config) {
        Ok(server) => server,
        Err(err) => return output.failure("serve", serve_exit_code(&err), &err.to_string()),
    };

    output.line(format!("Nerve is serving on {}", server.base_url()));
    output.line(String::new());
    output.line(format!("  open  {}", server.url()));
    output.line(String::new());
    output.line("  The session token is required on every request, as the X-Nerve-Token");
    output.line("  header or a token query parameter. It is not written to disk and it dies");
    output.line("  with this process. Requests from another origin, or naming another host,");
    output.line("  are refused. The API is read-only.");
    output.line(String::new());
    output.line("  Press Ctrl-C to stop.");

    output.object(json!({
        "command": "serve",
        "ok": true,
        "exit_code": exit::SUCCESS,
        "address": server.address().to_string(),
        "port": server.address().port(),
        "base_url": server.base_url(),
        "url": server.url(),
        "token": server.token().as_str(),
        "token_header": nerve_server::token::TOKEN_HEADER,
        "workers": workers,
        "routes": nerve_server::router::ROUTES,
    }));

    install_shutdown(server.shutdown_handle());
    server.join();
    output.line("Stopped.");
    exit::SUCCESS
}

// ---- mcp -----------------------------------------------------------------------------------

/// Speak MCP on stdin and stdout until the client closes the stream.
///
/// **Stdout belongs to the protocol.** Nothing is printed there while the session runs: not a
/// banner, not a `--json` summary, not a progress line. A stray line would desynchronise the
/// client's parser, and a client that cannot parse the stream cannot report why.
///
/// A failure to open the index is reported the way every other command reports one, before the
/// loop starts and therefore before anything has been framed. There is no signal handler here
/// and none is wanted: the session ends when its stdin ends, which is how a stdio server is
/// supposed to stop, and the connection is read-only so there is nothing to unwind.
fn run_mcp(output: &Output, path: &Path) -> i32 {
    let mut session = match nerve_server::mcp::McpSession::open(path) {
        Ok(session) => session,
        Err(err) => return output.failure("mcp", serve_exit_code(&err), &err.to_string()),
    };
    match nerve_server::mcp::serve_stdio(&mut session) {
        Ok(()) => exit::SUCCESS,
        // A broken pipe is the client hanging up, which is how these sessions usually end.
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => exit::SUCCESS,
        Err(err) => output.failure("mcp", exit::INTERNAL, &err.to_string()),
    }
}

/// Turn SIGINT and SIGTERM into a graceful stop.
///
/// The alternative is the default disposition, which kills the process mid-response. That is
/// survivable here — the server never writes and holds only `query_only` connections — but
/// "survivable" is not the same as "clean", and a tool that has to be killed teaches its user
/// that being killed is normal.
///
/// If the handler cannot be installed the server still runs; the default disposition applies,
/// which is exactly the behaviour we would have had anyway.
fn install_shutdown(handle: nerve_server::ShutdownHandle) {
    use signal_hook::consts::{SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) else {
        return;
    };
    std::thread::Builder::new()
        .name("nerve-serve-signals".to_string())
        .spawn(move || {
            if signals.forever().next().is_some() {
                handle.shutdown();
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn exit_codes_are_the_documented_contract() {
        assert_eq!(exit::SUCCESS, 0);
        assert_eq!(exit::NO_INDEX, 2);
        assert_eq!(exit::PARTIAL_INDEX, 3);
        assert_eq!(exit::STALE_INDEX, 4);
        assert_eq!(exit::USAGE, 10);
        assert_eq!(exit::INTERNAL, 70);
    }

    fn swept(fresh: usize) -> nerve_index::IndexFreshness {
        nerve_index::IndexFreshness {
            files_total: fresh,
            files_probed: fresh,
            fresh,
            ..nerve_index::IndexFreshness::default()
        }
    }

    #[test]
    fn every_verdict_maps_to_exactly_one_exit_code() {
        assert_eq!(Verdict::Current.exit_code(false), exit::SUCCESS);
        assert_eq!(Verdict::NoIndex.exit_code(false), exit::NO_INDEX);
        assert_eq!(Verdict::Unusable.exit_code(false), exit::PARTIAL_INDEX);
        assert_eq!(Verdict::Stale.exit_code(false), exit::STALE_INDEX);
        assert_eq!(Verdict::Unverified.exit_code(false), exit::STALE_INDEX);
    }

    /// `--allow-stale` downgrades staleness and nothing else. A missing index is still a missing
    /// index however confident the caller is that the tree has not moved.
    #[test]
    fn allow_stale_downgrades_only_the_two_freshness_verdicts() {
        assert_eq!(Verdict::Stale.exit_code(true), exit::SUCCESS);
        assert_eq!(Verdict::Unverified.exit_code(true), exit::SUCCESS);
        assert_eq!(Verdict::NoIndex.exit_code(true), exit::NO_INDEX);
        assert_eq!(Verdict::Unusable.exit_code(true), exit::PARTIAL_INDEX);
        assert_eq!(Verdict::Current.exit_code(true), exit::SUCCESS);
    }

    #[test]
    fn an_index_that_matches_the_tree_is_current() {
        let (verdict, _) = judge_freshness(&swept(12), 0);
        assert_eq!(verdict, Verdict::Current);
        assert_eq!(verdict.exit_code(false), exit::SUCCESS);
    }

    /// Changed, deleted and added are three different observations and one verdict. Each is
    /// asserted on its own so that dropping any single one from the sum is a test failure.
    #[test]
    fn changed_deleted_and_added_each_make_the_index_stale() {
        let mut changed = swept(12);
        changed.fresh = 11;
        changed.stale = 1;
        assert_eq!(judge_freshness(&changed, 0).0, Verdict::Stale);

        let mut deleted = swept(12);
        deleted.fresh = 11;
        deleted.missing = 1;
        assert_eq!(judge_freshness(&deleted, 0).0, Verdict::Stale);

        assert_eq!(
            judge_freshness(&swept(12), 1).0,
            Verdict::Stale,
            "an added file is not in the cache the sweep walks, so only the untracked walk sees it"
        );
    }

    /// The rule the whole command rests on: a sweep that stopped early has not seen the tree, so
    /// it cannot certify it. Exercised here rather than end to end because forcing the cap needs
    /// a repository larger than the probe cap.
    #[test]
    fn a_truncated_sweep_is_never_a_clean_result() {
        let truncated = nerve_index::IndexFreshness {
            files_total: CHECK_PROBE_CAP * 2,
            files_probed: CHECK_PROBE_CAP,
            fresh: CHECK_PROBE_CAP,
            truncated: true,
            ..nerve_index::IndexFreshness::default()
        };
        let (verdict, reason) = judge_freshness(&truncated, 0);
        assert_ne!(verdict, Verdict::Current);
        assert_eq!(verdict, Verdict::Unverified);
        assert_ne!(verdict.exit_code(false), exit::SUCCESS);
        assert_eq!(verdict.exit_code(false), exit::STALE_INDEX);
        assert!(reason.contains("never looked at"), "{reason}");
    }

    /// A file the sweep was not allowed to read, or could not, is not a fresh file.
    #[test]
    fn a_file_the_sweep_could_not_compare_is_not_counted_as_fresh() {
        let mut refused = swept(12);
        refused.fresh = 11;
        refused.refused = 1;
        assert_eq!(judge_freshness(&refused, 0).0, Verdict::Unverified);

        let mut unreadable = swept(12);
        unreadable.fresh = 11;
        unreadable.unreadable = 1;
        assert_eq!(judge_freshness(&unreadable, 0).0, Verdict::Unverified);
    }

    /// Observed divergence outranks "could not tell": the caller can act on the first.
    #[test]
    fn observed_staleness_outranks_an_incomplete_sweep() {
        let mut both = swept(12);
        both.fresh = 10;
        both.stale = 1;
        both.refused = 1;
        both.truncated = true;
        assert_eq!(judge_freshness(&both, 0).0, Verdict::Stale);
    }

    /// The schema gate is the one question answered before any table is read, so it is asserted
    /// on its own as well as through [`judge_index`].
    #[test]
    fn the_schema_is_judged_before_any_table_is_queried() {
        assert_eq!(judge_schema(Some(nerve_store::SCHEMA_VERSION)), None);
        assert_eq!(judge_schema(None).unwrap().0, Verdict::NoIndex);
        assert_eq!(
            judge_schema(Some(nerve_store::SCHEMA_VERSION + 1))
                .unwrap()
                .0,
            Verdict::Unusable,
            "a database from a newer build is unusable, not merely stale"
        );
    }

    #[test]
    fn an_index_is_judged_before_the_tree_is_swept() {
        assert_eq!(
            judge_index(Some(nerve_store::SCHEMA_VERSION), true, 0),
            None
        );

        let (verdict, _) = judge_index(None, false, 0).expect("no schema is not judgeable");
        assert_eq!(verdict, Verdict::NoIndex);

        let (verdict, reason) = judge_index(Some(nerve_store::SCHEMA_VERSION - 1), true, 0)
            .expect("an old schema is not judgeable");
        assert_eq!(verdict, Verdict::Unusable);
        assert_eq!(verdict.exit_code(false), exit::PARTIAL_INDEX);
        assert!(reason.contains("migrate"), "{reason}");

        let (verdict, _) = judge_index(Some(nerve_store::SCHEMA_VERSION), false, 0)
            .expect("an empty index is not judgeable");
        assert_eq!(verdict, Verdict::NoIndex);
        assert_eq!(verdict.exit_code(false), exit::NO_INDEX);

        let (verdict, reason) = judge_index(Some(nerve_store::SCHEMA_VERSION), true, 1)
            .expect("an open run is not judgeable");
        assert_eq!(verdict, Verdict::Unusable);
        assert!(reason.contains("did not finish"), "{reason}");
    }

    #[test]
    fn verdict_names_are_distinct_and_stable() {
        let names = [
            Verdict::Current.as_str(),
            Verdict::NoIndex.as_str(),
            Verdict::Unusable.as_str(),
            Verdict::Stale.as_str(),
            Verdict::Unverified.as_str(),
        ];
        assert_eq!(
            names,
            ["current", "no_index", "unusable", "stale", "unverified"]
        );
    }

    #[test]
    fn not_initialized_maps_to_no_index() {
        assert_eq!(
            error_exit_code(&IndexError::NotInitialized(PathBuf::from("/x"))),
            exit::NO_INDEX
        );
        assert_eq!(
            error_exit_code(&IndexError::NotADirectory(PathBuf::from("/x"))),
            exit::USAGE
        );
    }
}
