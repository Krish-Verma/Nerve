//! `nerve` — a local, offline code evidence graph.
//!
//! This binary is a thin adapter (ARCHITECTURE.md): it parses arguments, calls the
//! application layer, renders, and maps outcomes to exit codes. It contains no graph logic.

#![forbid(unsafe_code)]

mod exit;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use nerve_core::Relation;
use nerve_index::config;
use nerve_index::error::IndexError;

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
        Command::Status { path, path_flag } => run_status(
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
        let location = match (&hit.file_path, hit.start_line) {
            (Some(file), Some(line)) => format!("{file}:{line}"),
            _ => "-".to_string(),
        };
        let qualified = if hit.scope_path.is_empty() {
            hit.name.clone()
        } else {
            format!("{}.{}", hit.scope_path, hit.name)
        };
        output.line(format!("{:<10} {qualified:<32} {location}", hit.kind));
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

/// Resolve one selector, or render the refusal and return its exit code.
///
/// Ambiguity is exit 10 with the candidate list; nothing is chosen on the user's behalf.
fn resolve_one(
    output: &Output,
    command: &str,
    conn: &nerve_store::Connection,
    role: &str,
    selector: &str,
) -> Result<nerve_store::EntityRef, i32> {
    match nerve_store::resolve_selector(conn, selector) {
        Ok(nerve_store::Selection::Resolved { entity, .. }) => Ok(*entity),
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
        Ok(nerve_store::Selection::NotFound { suggestions }) => {
            let message = format!("{selector:?} ({role}) matches no indexed entity");
            let mut lines = Vec::new();
            if suggestions.is_empty() {
                lines.push("  no near matches; try `nerve search`".to_string());
            } else {
                lines.push("  did you mean:".to_string());
                for hit in &suggestions {
                    let qualified = if hit.scope_path.is_empty() {
                        hit.name.clone()
                    } else {
                        format!("{}.{}", hit.scope_path, hit.name)
                    };
                    let location = match (&hit.file_path, hit.start_line) {
                        (Some(file), Some(line)) => format!("{file}:{line}"),
                        _ => "-".to_string(),
                    };
                    lines.push(format!("    {:<10} {qualified:<34} {location}", hit.kind));
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
                    "suggestions": suggestions.iter().map(|hit| json!({
                        "entity_id": hit.entity_id,
                        "kind": hit.kind,
                        "name": hit.name,
                        "scope_path": hit.scope_path,
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
        Ok(entity) => entity,
        Err(code) => return code,
    };
    let to = match resolve_one(output, "path", &conn, "to", &arguments.to) {
        Ok(entity) => entity,
        Err(code) => return code,
    };

    let query = nerve_store::PathQuery {
        max_depth: arguments.max_depth,
        limit: arguments.limit,
        direction: arguments.direction.to_store(),
        relations: relations.clone(),
        resolved_only: arguments.resolved_only,
    };
    let report = match nerve_store::find_paths(&conn, &from.entity_id, &to.entity_id, &query) {
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
        Ok(entity) => entity,
        Err(code) => return code,
    };

    let query = nerve_store::ImpactQuery {
        max_depth: arguments.max_depth,
        limit: arguments.limit,
        relations,
    };
    let report = match nerve_store::impact(&conn, &subject.entity_id, &query, &prober) {
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
        output.line("  Every reference site Nerve indexed under those relations resolved, so no");
        output.line("  failed resolution is hiding a dependency from this answer.");
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
        Ok(entity) => entity,
        Err(code) => return code,
    };
    let object = match &arguments.to {
        Some(selector) => match resolve_one(output, "why", &conn, "to", selector) {
            Ok(entity) => Some(entity),
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
        &subject.entity_id,
        object.as_ref().map(|entity| entity.entity_id.as_str()),
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
        assert_eq!(exit::USAGE, 10);
        assert_eq!(exit::INTERNAL, 70);
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
