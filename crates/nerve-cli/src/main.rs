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
        Command::Index { path } => run_index(&output, path),
        Command::Status { path } => run_status(&output, &path),
        Command::Search {
            query,
            kind,
            limit,
            path,
        } => run_search(&output, &path, &query, kind.as_deref(), limit),
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
            output.line(format!(
                "  unmodelled     {} call/heritage sites",
                outcome.unmodelled_call_sites
            ));
            for (form, count) in &outcome.unmodelled_by_form {
                output.line(format!("    {form:<18} {count}"));
            }
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
