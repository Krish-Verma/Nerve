//! `nerve doctor` — *"something is wrong with my install, what?"*
//!
//! `check` judges one thing — is the index current? — and its product is an exit code for CI.
//! `doctor` inspects many things — is the installation, the database and the configuration
//! sound? — and its product is a readable report for a person whose tooling is misbehaving.
//!
//! Which means it must **run and produce a useful report when things are broken**: no database,
//! a corrupt database, a schema from the future, an unparseable config. Those are its subject
//! matter, not reasons to bail out. Every fatal path below records a finding and carries on with
//! the checks that are still answerable; nothing here panics or exits early.
//!
//! Three deliberate absences:
//!
//! - **No repair.** `doctor` diagnoses. Every remedy is a command the *user* runs, and the
//!   connection is opened `query_only` so no later edit can quietly start fixing things.
//! - **No index-freshness verdict.** That is `check`'s single question, `check` answers it with
//!   a sweep of the whole tree, and a second implementation here would be a second answer to
//!   drift from. `doctor` says so and points at `check`.
//! - **No network.** There is nothing to reach; no version check, no update check.

use std::path::{Path, PathBuf};

use serde_json::json;

use nerve_index::config;

use crate::exit;
use crate::Output;

/// The exit code a fatal finding carries.
///
/// Reused rather than minted. Exit `2` is documented as *"there is no index at the requested
/// path, **or it is not healthy enough to answer**"*, and every fatal finding below is exactly
/// one of those two: nothing is installed here, or what is installed cannot be read as a Nerve
/// index. A sixth exit code would say nothing the fourth does not.
const FATAL_EXIT: i32 = exit::NO_INDEX;

/// How bad one finding is.
///
/// Ordered worst-last so the summary can take a maximum. `Skipped` sits above `Ok` because a
/// check that could not run is not a check that passed — the same distinction between *none* and
/// *unknown* the rest of Nerve is built on — and below `Warning` because it is almost always the
/// downstream consequence of a warning or a failure already reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    /// Checked, and sound.
    Ok,
    /// Not checked, because something earlier made it unanswerable.
    Skipped,
    /// Checked, and wrong in a way one ordinary `nerve` command puts right.
    Warning,
    /// Checked, and this installation cannot be read as a Nerve index as it stands.
    Fatal,
}

impl Severity {
    /// Name used in `--json`.
    fn as_str(self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Skipped => "skipped",
            Severity::Warning => "warning",
            Severity::Fatal => "fatal",
        }
    }

    /// Fixed-width tag used in the human report.
    fn tag(self) -> &'static str {
        match self {
            Severity::Ok => "ok  ",
            Severity::Skipped => "skip",
            Severity::Warning => "warn",
            Severity::Fatal => "FAIL",
        }
    }
}

/// Which part of the installation a check belongs to. Grouping only; it carries no judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    /// The files on disk.
    Installation,
    /// The database as a database.
    Database,
    /// `config.toml` and what it and the database agree the repository is.
    Configuration,
    /// What has been indexed into it.
    Index,
}

impl Group {
    /// Name used in `--json`.
    fn as_str(self) -> &'static str {
        match self {
            Group::Installation => "installation",
            Group::Database => "database",
            Group::Configuration => "configuration",
            Group::Index => "index",
        }
    }

    /// Heading used in the human report.
    fn heading(self) -> &'static str {
        match self {
            Group::Installation => "Installation",
            Group::Database => "Database",
            Group::Configuration => "Configuration",
            Group::Index => "Index",
        }
    }
}

/// The closed vocabulary of checks `doctor` performs.
///
/// These identifiers are the machine-readable contract: a caller branches on `id` and reads
/// `severity`, so an id may be added but must never be renamed or silently repurposed. Exactly
/// one finding is emitted per id on every run — including when the check could not be performed,
/// which is reported as [`Severity::Skipped`] rather than by omitting the id, so a caller can
/// tell "sound" from "never established" without knowing which checks exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckId {
    /// `.nerve/` is present and is a directory.
    NerveDir,
    /// `.nerve/nerve.db` is present and its bytes can be read.
    DatabaseFile,
    /// `.nerve/nerve.db` and its sidecars are owner-only.
    DatabasePermissions,
    /// `PRAGMA integrity_check`.
    DatabaseIntegrity,
    /// The schema version on disk against the one this build writes.
    SchemaVersion,
    /// Every migration step up to that version was actually applied.
    MigrationHistory,
    /// The full-text index holds one document per entity.
    FtsConsistency,
    /// `.nerve/config.toml` is present and parses.
    ConfigFile,
    /// The root recorded at `init` still exists and is the one `doctor` was pointed at.
    RecordedRoot,
    /// Anything has ever been indexed.
    IndexPresent,
    /// No extractor run is missing its finish time.
    UnfinishedRuns,
}

impl CheckId {
    /// Every check, in report order. The vocabulary a `--json` caller may rely on.
    pub(crate) const ALL: [CheckId; 11] = [
        CheckId::NerveDir,
        CheckId::DatabaseFile,
        CheckId::DatabasePermissions,
        CheckId::DatabaseIntegrity,
        CheckId::SchemaVersion,
        CheckId::MigrationHistory,
        CheckId::FtsConsistency,
        CheckId::ConfigFile,
        CheckId::RecordedRoot,
        CheckId::IndexPresent,
        CheckId::UnfinishedRuns,
    ];

    /// Stable machine-readable name.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CheckId::NerveDir => "nerve_dir",
            CheckId::DatabaseFile => "database_file",
            CheckId::DatabasePermissions => "database_permissions",
            CheckId::DatabaseIntegrity => "database_integrity",
            CheckId::SchemaVersion => "schema_version",
            CheckId::MigrationHistory => "migration_history",
            CheckId::FtsConsistency => "fts_consistency",
            CheckId::ConfigFile => "config_file",
            CheckId::RecordedRoot => "recorded_root",
            CheckId::IndexPresent => "index_present",
            CheckId::UnfinishedRuns => "unfinished_runs",
        }
    }

    fn group(self) -> Group {
        match self {
            CheckId::NerveDir | CheckId::DatabaseFile | CheckId::DatabasePermissions => {
                Group::Installation
            }
            CheckId::DatabaseIntegrity
            | CheckId::SchemaVersion
            | CheckId::MigrationHistory
            | CheckId::FtsConsistency => Group::Database,
            CheckId::ConfigFile | CheckId::RecordedRoot => Group::Configuration,
            CheckId::IndexPresent | CheckId::UnfinishedRuns => Group::Index,
        }
    }

    /// What this check looks at. Printed for anything that is not sound, so the reader can see
    /// what was actually established rather than inferring it from the complaint.
    fn checked(self) -> &'static str {
        match self {
            CheckId::NerveDir => "whether .nerve/ exists and is a directory",
            CheckId::DatabaseFile => "whether .nerve/nerve.db exists and its bytes can be read",
            CheckId::DatabasePermissions => {
                "the file mode of .nerve/nerve.db and its write-ahead-log sidecars"
            }
            CheckId::DatabaseIntegrity => "PRAGMA integrity_check over the database",
            CheckId::SchemaVersion => {
                "the schema version on disk against the one this build writes"
            }
            CheckId::MigrationHistory => "whether every migration up to that version was applied",
            CheckId::FtsConsistency => {
                "the document count of the full-text index against the entity table"
            }
            CheckId::ConfigFile => "whether .nerve/config.toml exists and parses",
            CheckId::RecordedRoot => {
                "whether the root recorded at init still exists and is this directory"
            }
            CheckId::IndexPresent => "whether anything has ever been indexed",
            CheckId::UnfinishedRuns => "extractor runs with no recorded finish time",
        }
    }
}

/// One diagnosis: what was checked, what was found, how bad it is, and what to do about it.
pub(crate) struct Finding {
    id: CheckId,
    severity: Severity,
    found: String,
    /// The command or action that resolves it. `None` only when there is nothing to resolve.
    remedy: Option<String>,
}

/// Findings collected in check order, one per [`CheckId`].
struct Report {
    findings: Vec<Finding>,
}

impl Report {
    fn new() -> Self {
        Report {
            findings: Vec::new(),
        }
    }

    fn has(&self, id: CheckId) -> bool {
        self.findings.iter().any(|finding| finding.id == id)
    }

    fn record(&mut self, id: CheckId, severity: Severity, found: String, remedy: Option<&str>) {
        debug_assert!(!self.has(id), "{} recorded twice", id.as_str());
        self.findings.push(Finding {
            id,
            severity,
            found,
            remedy: remedy.map(str::to_string),
        });
    }

    fn ok(&mut self, id: CheckId, found: String) {
        self.record(id, Severity::Ok, found, None);
    }

    fn warn(&mut self, id: CheckId, found: String, remedy: &str) {
        self.record(id, Severity::Warning, found, Some(remedy));
    }

    fn fatal(&mut self, id: CheckId, found: String, remedy: &str) {
        self.record(id, Severity::Fatal, found, Some(remedy));
    }

    fn skip(&mut self, id: CheckId, cause: &str) {
        self.record(id, Severity::Skipped, cause.to_string(), None);
    }

    /// Mark every check not yet recorded as unanswerable, with the reason it is unanswerable.
    ///
    /// This is what makes an early exit honest: the checks downstream of a broken database are
    /// reported as *not established*, never quietly dropped and never reported as passing.
    fn skip_remaining(&mut self, cause: &str) {
        for id in CheckId::ALL {
            if !self.has(id) {
                self.skip(id, cause);
            }
        }
    }

    /// Findings in [`CheckId::ALL`] order.
    fn finish(mut self) -> Vec<Finding> {
        self.skip_remaining("this check was not reached");
        let mut ordered = Vec::with_capacity(CheckId::ALL.len());
        for id in CheckId::ALL {
            if let Some(position) = self.findings.iter().position(|finding| finding.id == id) {
                ordered.push(self.findings.remove(position));
            }
        }
        ordered
    }
}

/// The database path plus its write-ahead-log sidecars, whichever of them exist.
fn database_files(db_path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let candidate = if suffix.is_empty() {
            db_path.to_path_buf()
        } else {
            let mut name = db_path.as_os_str().to_os_string();
            name.push(suffix);
            PathBuf::from(name)
        };
        if candidate.exists() {
            files.push(candidate);
        }
    }
    files
}

/// Owner-only permissions on the database and its sidecars (SECURITY.md, "Data at rest").
///
/// A real security control, not a formality: the index stores repository-relative paths, symbol
/// names, scope paths and content hashes. It stores no source text, and it is still a map of the
/// repository that no other account should be able to read.
#[cfg(unix)]
fn check_permissions(report: &mut Report, db_path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for file in database_files(db_path) {
        let Ok(metadata) = std::fs::metadata(&file) else {
            continue;
        };
        checked += 1;
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o600 {
            let name = file
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| file.display().to_string());
            offenders.push(format!("{name} is {mode:04o}"));
        }
    }
    if checked == 0 {
        report.skip(
            CheckId::DatabasePermissions,
            "there is no database file to inspect",
        );
        return;
    }
    if offenders.is_empty() {
        report.ok(
            CheckId::DatabasePermissions,
            format!("all {checked} database file(s) are mode 0600, readable by the owner only"),
        );
        return;
    }
    let listed = offenders.join(", ");
    report.warn(
        CheckId::DatabasePermissions,
        format!("{listed}, and SECURITY.md requires owner-only 0600"),
        &format!("chmod 600 {}*", db_path.display()),
    );
}

#[cfg(not(unix))]
fn check_permissions(report: &mut Report, _db_path: &Path) {
    report.skip(
        CheckId::DatabasePermissions,
        "file modes are a Unix concept; this build does not inspect Windows ACLs",
    );
}

/// Everything `doctor` can establish about `root`.
///
/// `root` must already be canonical. Nothing here writes, and nothing here reads repository
/// content: the whole subject is `.nerve/`.
fn diagnose(root: &Path) -> Vec<Finding> {
    let mut report = Report::new();
    let nerve_dir = config::nerve_dir(root);
    let db_path = config::db_path(root);

    // ---- installation ----
    if !nerve_dir.exists() {
        report.fatal(
            CheckId::NerveDir,
            format!("there is no .nerve directory at {}", root.display()),
            "run `nerve init` here, or run doctor against the repository root if this is not it",
        );
        report.skip_remaining("there is no .nerve directory to inspect");
        return report.finish();
    }
    if !nerve_dir.is_dir() {
        report.fatal(
            CheckId::NerveDir,
            format!("{} exists but is not a directory", nerve_dir.display()),
            "move it aside, then run `nerve init`",
        );
        report.skip_remaining("the .nerve path is not a directory");
        return report.finish();
    }
    report.ok(CheckId::NerveDir, ".nerve/ is present".to_string());

    let database_present = db_path.is_file();
    if !database_present {
        report.fatal(
            CheckId::DatabaseFile,
            format!("{} is missing", db_path.display()),
            "run `nerve init`, then `nerve index`",
        );
    } else if let Err(err) = std::fs::File::open(&db_path) {
        report.fatal(
            CheckId::DatabaseFile,
            format!("{} cannot be opened for reading: {err}", db_path.display()),
            "check the ownership and permissions of .nerve/ and the file inside it",
        );
    } else {
        report.ok(
            CheckId::DatabaseFile,
            format!(
                "present and readable, {} bytes including the write-ahead-log sidecars",
                nerve_store::database_bytes(&db_path)
            ),
        );
    }
    check_permissions(&mut report, &db_path);

    // ---- configuration that does not need the database ----
    check_config(&mut report, root);

    if !database_present {
        report.skip_remaining("there is no database to read");
        return report.finish();
    }

    // ---- the database ----
    //
    // `query_only` is the whole "doctor never repairs" guarantee, made by construction rather
    // than by discipline, exactly as `check` makes it.
    let opened = nerve_store::open(&db_path)
        .map_err(|err| err.to_string())
        .and_then(|conn| {
            conn.pragma_update(None, "query_only", "ON")
                .map(|()| conn)
                .map_err(|err| err.to_string())
        });
    let conn = match opened {
        Ok(conn) => conn,
        Err(err) => {
            report.fatal(
                CheckId::DatabaseIntegrity,
                format!("SQLite could not open the file: {err}"),
                "Nerve cannot repair this. Move .nerve/nerve.db aside, then run `nerve init` \
                 and `nerve index` to rebuild it.",
            );
            report.skip_remaining("the database could not be opened");
            return report.finish();
        }
    };

    let facts = match nerve_store::diagnose(&conn) {
        Ok(facts) => facts,
        Err(err) => {
            report.fatal(
                CheckId::DatabaseIntegrity,
                format!("the integrity check could not be run: {err}"),
                "Nerve cannot repair this. Move .nerve/nerve.db aside, then run `nerve init` \
                 and `nerve index` to rebuild it.",
            );
            report.skip_remaining("the database could not be inspected");
            return report.finish();
        }
    };

    let sound = facts.integrity == ["ok"];
    if sound {
        report.ok(
            CheckId::DatabaseIntegrity,
            "PRAGMA integrity_check reports ok".to_string(),
        );
    } else {
        report.fatal(
            CheckId::DatabaseIntegrity,
            format!(
                "PRAGMA integrity_check reports: {}",
                if facts.integrity.is_empty() {
                    "nothing at all".to_string()
                } else {
                    facts.integrity.join("; ")
                }
            ),
            "Nerve cannot repair this. Move .nerve/nerve.db aside, then run `nerve init` and \
             `nerve index` to rebuild it.",
        );
    }

    check_schema(&mut report, &facts);
    check_index(&mut report, root, &facts);
    report.finish()
}

/// The schema version and the migration history behind it.
fn check_schema(report: &mut Report, facts: &nerve_store::DatabaseDiagnostics) {
    let supported = nerve_store::SCHEMA_VERSION;
    match facts.schema_version {
        None => {
            report.fatal(
                CheckId::SchemaVersion,
                "the database has no schema_version table, so it has never been migrated and \
                 may not be a Nerve database at all"
                    .to_string(),
                "run `nerve init` against this directory; if the file was not written by Nerve, \
                 move it aside first",
            );
            report.skip(
                CheckId::MigrationHistory,
                "the database records no migrations",
            );
        }
        // Ahead of this build. Reported as exactly that, and never as damage: the file is
        // intact, it is simply describing a schema this binary was written before.
        Some(version) if version > supported => {
            report.fatal(
                CheckId::SchemaVersion,
                format!(
                    "schema {version} on disk was written by a newer Nerve; this build \
                     understands {supported} and will not read a database from the future"
                ),
                &format!(
                    "upgrade Nerve to a build that writes schema {version} or later. Migrations \
                     run forward only, so this build cannot convert the database back."
                ),
            );
            report.skip(
                CheckId::MigrationHistory,
                "the migration history is one this build does not know",
            );
        }
        Some(version) if version < supported => {
            report.warn(
                CheckId::SchemaVersion,
                format!(
                    "schema {version} on disk, and this build writes {supported}: \
                     {} migration(s) are pending",
                    supported - version
                ),
                "run `nerve index`, which migrates before it extracts",
            );
            check_migration_history(report, facts, version);
        }
        Some(version) => {
            report.ok(
                CheckId::SchemaVersion,
                format!("schema {version} on disk, and this build writes {supported}"),
            );
            check_migration_history(report, facts, version);
        }
    }
}

/// Whether every migration step up to the recorded version was actually applied.
///
/// A gap is a real and otherwise silent failure: migrations are applied only above the recorded
/// maximum, so a database claiming version 4 with version 2 missing will never have version 2's
/// tables and no `nerve` command will ever add them.
fn check_migration_history(
    report: &mut Report,
    facts: &nerve_store::DatabaseDiagnostics,
    version: i64,
) {
    let missing: Vec<i64> = (1..=version)
        .filter(|step| !facts.applied_versions.contains(step))
        .collect();
    if missing.is_empty() {
        report.ok(
            CheckId::MigrationHistory,
            format!("migrations 1 to {version} were all applied"),
        );
        return;
    }
    let listed = missing
        .iter()
        .map(|step| step.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    report.fatal(
        CheckId::MigrationHistory,
        format!(
            "the database claims schema {version} but migration(s) {listed} were never applied, \
             so the tables they create are missing"
        ),
        "Nerve applies migrations only above the recorded version and cannot replay a skipped \
         step. Move .nerve/nerve.db aside, then run `nerve init` and `nerve index`.",
    );
}

/// What has been indexed, and what the database says the repository is.
fn check_index(report: &mut Report, root: &Path, facts: &nerve_store::DatabaseDiagnostics) {
    match (facts.entities, facts.fts_documents) {
        (Some(entities), Some(documents)) if entities == documents => report.ok(
            CheckId::FtsConsistency,
            format!("{entities} entities and {documents} documents in the full-text index"),
        ),
        (Some(entities), Some(documents)) => report.warn(
            CheckId::FtsConsistency,
            format!(
                "{entities} entities but {documents} document(s) in the full-text index, so \
                 `nerve search` will miss rows or return rows that no longer exist"
            ),
            "run `nerve index --full`; it rewrites every entity row, and the triggers that \
             maintain the full-text index fire with it",
        ),
        _ => report.skip(
            CheckId::FtsConsistency,
            "the entity table or the full-text index could not be counted",
        ),
    }

    match &facts.repository_root {
        None => report.warn(
            CheckId::RecordedRoot,
            "the database records no repository row, so it does not know which directory it \
             describes"
                .to_string(),
            "run `nerve init` here; it records the root without touching the existing graph",
        ),
        Some(recorded) => {
            let recorded_path = Path::new(recorded);
            match std::fs::canonicalize(recorded_path) {
                Err(_) => report.warn(
                    CheckId::RecordedRoot,
                    format!("the root recorded at init, {recorded}, no longer exists"),
                    "run `nerve init` here to record this directory as the root, then \
                     `nerve index`",
                ),
                Ok(canonical) if canonical == root => report.ok(
                    CheckId::RecordedRoot,
                    "the root recorded at init is this directory".to_string(),
                ),
                Ok(_) => report.warn(
                    CheckId::RecordedRoot,
                    format!(
                        "the root recorded at init is {recorded}, and doctor was invoked against \
                         {}: this repository was moved or copied",
                        root.display()
                    ),
                    "run `nerve init` here to re-record the root — it keeps the project_id, so \
                     no entity id changes — then `nerve index`",
                ),
            }
        }
    }

    let ever_indexed = facts.extractor_runs.unwrap_or(0) > 0;
    match (ever_indexed, facts.entities) {
        (false, _) => report.warn(
            CheckId::IndexPresent,
            "nothing has ever been indexed: the database is initialized and empty".to_string(),
            "run `nerve index`",
        ),
        (true, Some(entities)) => report.ok(
            CheckId::IndexPresent,
            format!(
                "{entities} entities, last extractor run {}",
                facts.last_run_at.as_deref().unwrap_or("at an unknown time")
            ),
        ),
        (true, None) => report.skip(
            CheckId::IndexPresent,
            "runs are recorded but the entity table could not be counted",
        ),
    }

    match facts.unfinished_runs {
        None => report.skip(
            CheckId::UnfinishedRuns,
            "the extractor_run table could not be read",
        ),
        Some(0) => report.ok(
            CheckId::UnfinishedRuns,
            "every extractor run recorded a finish time".to_string(),
        ),
        Some(open) => report.warn(
            CheckId::UnfinishedRuns,
            format!(
                "{open} extractor run(s) have no finish time: an index was interrupted, so the \
                 graph may be a half-written one"
            ),
            "run `nerve index --full` to rebuild it from a known state",
        ),
    }
}

/// Collapse a multi-line error into one line.
///
/// A `toml` parse error carries a source excerpt and a caret across three or four lines. That is
/// helpful in isolation and destroys a column-aligned report, so the report keeps it to a line
/// and the reader still gets the position and the reason.
fn one_line(err: &impl std::fmt::Display) -> String {
    err.to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `.nerve/config.toml`: present, and parseable.
///
/// Fatal rather than a warning when it is either missing or malformed, because it is not
/// decoration: `nerve index`, `nerve check` and `nerve why` all load it before they read a single
/// file — it carries the per-file size ceiling and the secret deny-list — and every one of them
/// fails outright without it.
fn check_config(report: &mut Report, root: &Path) {
    let path = config::config_path(root);
    if !path.is_file() {
        report.fatal(
            CheckId::ConfigFile,
            format!("{} is missing", path.display()),
            "run `nerve init`; it writes a fresh config and leaves the database alone",
        );
        return;
    }
    match config::Config::load(root) {
        Ok(config) => report.ok(
            CheckId::ConfigFile,
            format!(
                "parses; project_id {}, written for schema {}, max_file_bytes {}",
                config.project_id, config.schema_version, config.index.max_file_bytes
            ),
        ),
        Err(err) => report.fatal(
            CheckId::ConfigFile,
            format!("{} does not parse: {}", path.display(), one_line(&err)),
            "correct the file by hand. Deleting it makes `nerve init` mint a new project_id, \
             and every entity id in the database derives from that one (ADR-0002), so the \
             existing index would be orphaned.",
        ),
    }
}

/// Findings tallied by severity, worst first.
fn tally(findings: &[Finding]) -> (usize, usize, usize, usize) {
    let count = |wanted: Severity| {
        findings
            .iter()
            .filter(|finding| finding.severity == wanted)
            .count()
    };
    (
        count(Severity::Fatal),
        count(Severity::Warning),
        count(Severity::Skipped),
        count(Severity::Ok),
    )
}

/// Render the report and return its exit code.
fn render(output: &Output, root: &Path, findings: &[Finding]) -> i32 {
    let (fatal, warning, skipped, ok) = tally(findings);
    let code = if fatal > 0 { FATAL_EXIT } else { exit::SUCCESS };

    output.line(format!("Nerve doctor — {}", root.display()));
    let mut current: Option<Group> = None;
    for finding in findings {
        let group = finding.id.group();
        if current != Some(group) {
            output.line(String::new());
            output.line(format!("  {}", group.heading()));
            current = Some(group);
        }
        output.line(format!(
            "    {} {:<21} {}{}",
            finding.severity.tag(),
            finding.id.as_str(),
            if finding.severity == Severity::Skipped {
                "not checked — "
            } else {
                ""
            },
            finding.found
        ));
        // A sound check needs no elaboration, and a skipped one has already said why it was
        // skipped. Anything actionable gets what was looked at and what to do next, because a
        // finding a reader cannot act on is not worth printing.
        if matches!(finding.severity, Severity::Warning | Severity::Fatal) {
            output.line(format!("           checked  {}", finding.id.checked()));
            if let Some(remedy) = &finding.remedy {
                output.line(format!("           do       {remedy}"));
            }
        }
    }

    output.line(String::new());
    output.line(format!(
        "  summary        {} checks · {ok} ok · {warning} warning · {fatal} fatal · {skipped} \
         skipped",
        findings.len()
    ));
    // Said whether or not anyone asked, because a clean doctor report reads as "everything is
    // fine" and index currency is the one thing it did not look at.
    output.line(
        "  freshness      not judged here — `nerve check` asks whether the index still describes",
    );
    output.line("                 the working tree, and answers with an exit code");
    output.line(format!("  exit_code      {code}"));

    output.object(json!({
        "command": "doctor",
        "ok": code == exit::SUCCESS,
        "exit_code": code,
        "error": if fatal > 0 {
            json!(format!("{fatal} fatal finding(s)"))
        } else {
            serde_json::Value::Null
        },
        "root": root.display().to_string(),
        "nerve_dir": config::nerve_dir(root).display().to_string(),
        "database_path": config::db_path(root).display().to_string(),
        "counts": {
            "checks": findings.len(),
            "ok": ok,
            "warning": warning,
            "fatal": fatal,
            "skipped": skipped,
        },
        "findings": findings.iter().map(|finding| json!({
            "id": finding.id.as_str(),
            "group": finding.id.group().as_str(),
            "severity": finding.severity.as_str(),
            "checked": finding.id.checked(),
            "found": finding.found,
            "remedy": finding.remedy,
        })).collect::<Vec<_>>(),
    }));

    code
}

/// Diagnose the installation at `path` and report it.
pub(crate) fn run(output: &Output, path: &Path) -> i32 {
    let root = match std::fs::canonicalize(path) {
        Ok(root) if root.is_dir() => root,
        Ok(root) => {
            return output.failure(
                "doctor",
                exit::USAGE,
                &format!("{} is not a directory", root.display()),
            )
        }
        Err(err) => {
            return output.failure("doctor", exit::USAGE, &format!("{}: {err}", path.display()))
        }
    };
    let findings = diagnose(&root);
    render(output, &root, &findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `--json` contract. An id may be added; renaming one breaks every caller that branches
    /// on it, so the vocabulary is pinned here rather than left to drift.
    #[test]
    fn the_finding_id_vocabulary_is_closed_and_stable() {
        let names: Vec<&str> = CheckId::ALL.iter().map(|id| id.as_str()).collect();
        assert_eq!(
            names,
            [
                "nerve_dir",
                "database_file",
                "database_permissions",
                "database_integrity",
                "schema_version",
                "migration_history",
                "fts_consistency",
                "config_file",
                "recorded_root",
                "index_present",
                "unfinished_runs",
            ]
        );
    }

    #[test]
    fn severity_names_are_stable_and_ordered_worst_last() {
        assert_eq!(Severity::Ok.as_str(), "ok");
        assert_eq!(Severity::Skipped.as_str(), "skipped");
        assert_eq!(Severity::Warning.as_str(), "warning");
        assert_eq!(Severity::Fatal.as_str(), "fatal");
        assert!(Severity::Ok < Severity::Skipped);
        assert!(Severity::Skipped < Severity::Warning);
        assert!(Severity::Warning < Severity::Fatal);
    }

    /// Every check is reported on every run, whatever happened. A caller that branches on `id`
    /// can therefore tell "sound" from "never established" without knowing the check list.
    #[test]
    fn every_check_is_reported_exactly_once_however_broken_the_installation_is() {
        let empty = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(empty.path()).unwrap();
        let findings = diagnose(&root);
        let ids: Vec<&str> = findings.iter().map(|finding| finding.id.as_str()).collect();
        let expected: Vec<&str> = CheckId::ALL.iter().map(|id| id.as_str()).collect();
        assert_eq!(ids, expected, "in vocabulary order, with none dropped");
        assert!(
            findings
                .iter()
                .all(|finding| finding.severity != Severity::Ok),
            "there is nothing installed here for any check to pass"
        );
        assert_eq!(
            findings[0].severity,
            Severity::Fatal,
            "a missing .nerve directory is the diagnosis, not a reason to stop"
        );
    }

    /// A check that could not run is never reported as one that passed.
    #[test]
    fn unreached_checks_are_skipped_rather_than_dropped_or_passed() {
        let mut report = Report::new();
        report.ok(CheckId::NerveDir, "present".to_string());
        report.skip_remaining("nothing else was readable");
        let findings = report.finish();
        assert_eq!(findings.len(), CheckId::ALL.len());
        assert_eq!(findings[0].severity, Severity::Ok);
        assert!(findings[1..]
            .iter()
            .all(|finding| finding.severity == Severity::Skipped));
    }

    fn finding(severity: Severity) -> Finding {
        Finding {
            id: CheckId::NerveDir,
            severity,
            found: String::new(),
            remedy: None,
        }
    }

    /// The exit rule, asserted on its own: a warning is not a failure, and neither is a check
    /// that could not be run.
    #[test]
    fn only_a_fatal_finding_makes_doctor_exit_non_zero() {
        for severity in [Severity::Ok, Severity::Warning, Severity::Skipped] {
            let (fatal, _, _, _) = tally(&[finding(severity)]);
            assert_eq!(fatal, 0, "{severity:?} must not be counted as fatal");
        }
        let (fatal, _, _, _) = tally(&[finding(Severity::Fatal)]);
        assert_eq!(fatal, 1);
        assert_eq!(FATAL_EXIT, exit::NO_INDEX, "no new exit code was minted");
    }

    /// Every finding that is not sound must say what to do about it.
    #[test]
    fn every_actionable_finding_carries_a_remedy() {
        let empty = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(empty.path()).unwrap();
        for finding in diagnose(&root) {
            match finding.severity {
                Severity::Warning | Severity::Fatal => assert!(
                    finding.remedy.is_some(),
                    "{} is actionable and says nothing about what to do",
                    finding.id.as_str()
                ),
                Severity::Ok | Severity::Skipped => assert!(
                    finding.remedy.is_none(),
                    "{} has nothing to remedy",
                    finding.id.as_str()
                ),
            }
        }
    }
}
