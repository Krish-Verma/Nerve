//! `nerve` — a local, offline code evidence graph.
//!
//! This binary is a thin adapter (ARCHITECTURE.md): it parses arguments, calls the
//! application layer, renders, and maps outcomes to exit codes. It contains no graph logic.

#![forbid(unsafe_code)]

mod exit;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde_json::json;

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
    Index {
        /// Repository root. Defaults to the current directory.
        path: Option<PathBuf>,
    },
    /// Report index counts, freshness and schema version.
    Status {
        /// Repository root. Defaults to the current directory.
        #[arg(long, default_value = ".")]
        path: PathBuf,
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
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "command": command,
                    "ok": false,
                    "exit_code": code,
                    "error": message,
                }))
                .unwrap_or_default()
            );
        } else {
            eprintln!("nerve {command}: {message}");
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
        Command::Index { path } => run_index(&output, path),
        Command::Status { path } => run_status(&output, &path),
        Command::Search {
            query,
            kind,
            limit,
            path,
        } => run_search(&output, &path, &query, kind.as_deref(), limit),
    };
    std::process::exit(code);
}

fn error_exit_code(err: &IndexError) -> i32 {
    match err {
        IndexError::NotADirectory(_) => exit::USAGE,
        IndexError::NotInitialized(_) => exit::NO_INDEX,
        IndexError::Config { .. } => exit::NO_INDEX,
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

fn run_index(output: &Output, path: Option<PathBuf>) -> i32 {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    match nerve_index::index_repository(&root) {
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
            output.line(format!("  entities       {}", outcome.entities_total));
            for (kind, count) in &outcome.entities_by_kind {
                output.line(format!("    {kind:<12} {count}"));
            }
            output.line(format!("  assertions     {}", outcome.assertions_total));
            for (relation, count) in &outcome.assertions_by_relation {
                output.line(format!("    {relation:<12} {count}"));
            }
            output.line(format!("  observations   {}", outcome.observations_total));
            output.line(format!(
                "  unresolved     {} entities, {} assertions",
                outcome.unresolved_entities, outcome.unresolved_assertions
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
                "entities_total": outcome.entities_total,
                "entities_by_kind": outcome.entities_by_kind,
                "assertions_total": outcome.assertions_total,
                "assertions_by_relation": outcome.assertions_by_relation,
                "observations_total": outcome.observations_total,
                "unresolved_entities": outcome.unresolved_entities,
                "unresolved_assertions": outcome.unresolved_assertions,
                "duration_ms": outcome.duration_ms,
            }));
            code
        }
        Err(err) => output.failure("index", error_exit_code(&err), &err.to_string()),
    }
}

fn open_existing(path: &Path) -> Result<(PathBuf, nerve_store::Connection), String> {
    let root = std::fs::canonicalize(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(format!(
            "no Nerve index at {}; run `nerve init` first",
            root.display()
        ));
    }
    let conn = nerve_store::open(&db_path).map_err(|err| err.to_string())?;
    Ok((db_path, conn))
}

fn run_status(output: &Output, path: &Path) -> i32 {
    let (db_path, conn) = match open_existing(path) {
        Ok(pair) => pair,
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
        "entities_by_kind": report.entities_by_kind,
        "assertions_total": report.assertions_total,
        "assertions_by_relation": report.assertions_by_relation,
        "occurrences_total": report.occurrences_total,
        "observations_total": report.observations_total,
        "assertion_states_total": report.assertion_states_total,
        "unresolved_entities": report.unresolved_entities,
        "unresolved_assertions": report.unresolved_assertions,
        "last_run": report.last_run.as_ref().map(|run| json!({
            "run_id": run.run_id,
            "state_id": run.state_id,
            "extractor_id": run.extractor_id,
            "extractor_version": run.extractor_version,
            "started_at": run.started_at,
            "finished_at": run.finished_at,
            "files_processed": run.files_processed,
            "files_failed": run.files_failed,
            "status": run.status,
        })),
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

    let (_, conn) = match open_existing(path) {
        Ok(pair) => pair,
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
