//! `nerve init`.

use std::path::{Path, PathBuf};

use nerve_core::ids;

use crate::config::{self, Config, IndexSettings, SecuritySettings};
use crate::discover::canonical_root;
use crate::error::{IndexError, Result};

/// What `nerve init` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOutcome {
    /// Canonical repository root.
    pub root: PathBuf,
    /// `.nerve` directory.
    pub nerve_dir: PathBuf,
    /// Database path.
    pub db_path: PathBuf,
    /// Stable project identifier, existing or newly generated.
    pub project_id: String,
    /// False when a usable index was already present.
    pub created: bool,
    /// Schema version after migration.
    pub schema_version: i64,
}

/// 32 bytes from the operating system's randomness source.
///
/// No `rand` crate: `/dev/urandom` is the OS interface this needs and adding a dependency for
/// one read would widen the dependency surface for nothing.
#[cfg(unix)]
fn os_random_bytes() -> Result<[u8; 32]> {
    use std::io::Read;
    let mut buffer = [0u8; 32];
    let mut source = std::fs::File::open("/dev/urandom")
        .map_err(|err| IndexError::Randomness(format!("/dev/urandom: {err}")))?;
    source
        .read_exact(&mut buffer)
        .map_err(|err| IndexError::Randomness(format!("/dev/urandom: {err}")))?;
    Ok(buffer)
}

#[cfg(not(unix))]
fn os_random_bytes() -> Result<[u8; 32]> {
    Err(IndexError::Randomness(
        "no supported OS randomness source on this platform".to_string(),
    ))
}

/// Generate a fresh project identifier: 32 hex characters of BLAKE3 over 32 OS-random bytes.
///
/// Deliberately not derived from the absolute path, so re-cloning or moving the repository
/// does not change any entity id (ADR-0002).
pub fn generate_project_id() -> Result<String> {
    let bytes = os_random_bytes()?;
    Ok(ids::content_hash(&bytes)[..32].to_string())
}

/// Civil date from a day count since the Unix epoch (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Current UTC time as an RFC 3339 string. No clock dependency beyond the standard library.
fn timestamp() -> String {
    let Ok(since_epoch) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return "1970-01-01T00:00:00Z".to_string();
    };
    let seconds = since_epoch.as_secs() as i64;
    let (days, seconds_of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Create or refresh `.nerve/` for `root`.
///
/// Idempotent: an existing `config.toml` keeps its `project_id`, so re-running `init` never
/// invalidates an existing index.
pub fn init(root: &Path) -> Result<InitOutcome> {
    init_with_project_id(root, None)
}

/// [`init`], with an explicit project identifier.
///
/// Tests use this to pin entity ids so that golden and determinism comparisons are meaningful.
pub fn init_with_project_id(root: &Path, project_id: Option<&str>) -> Result<InitOutcome> {
    let root = canonical_root(root)?;
    let nerve_dir = config::nerve_dir(&root);
    let db_path = config::db_path(&root);
    let config_path = config::config_path(&root);
    let already_initialized = config_path.exists() && db_path.exists();

    std::fs::create_dir_all(nerve_dir.join("cache"))?;
    std::fs::create_dir_all(nerve_dir.join("logs"))?;

    // The index is never committed (SECURITY.md).
    std::fs::write(nerve_dir.join(".gitignore"), "*\n")?;

    let config = if config_path.exists() {
        let mut existing = Config::load(&root)?;
        if let Some(project_id) = project_id {
            if existing.project_id != project_id {
                return Err(IndexError::Config {
                    path: config_path,
                    message: format!(
                        "refusing to change project_id from {} to {}: every entity id derives \
                         from it (ADR-0002)",
                        existing.project_id, project_id
                    ),
                });
            }
        }
        existing.schema_version = nerve_store::SCHEMA_VERSION;
        existing
    } else {
        Config {
            schema_version: nerve_store::SCHEMA_VERSION,
            project_id: match project_id {
                Some(value) => value.to_string(),
                None => generate_project_id()?,
            },
            created_at: timestamp(),
            index: IndexSettings::default(),
            security: SecuritySettings::default(),
        }
    };
    config.save(&root)?;

    let conn = nerve_store::open(&db_path)?;
    nerve_store::migrate(&conn)?;
    nerve_store::upsert_repository(
        &conn,
        &nerve_store::RepositoryRow {
            repo_id: ids::repository_id(&config.project_id),
            project_id: config.project_id.clone(),
            root_path: root.to_string_lossy().to_string(),
        },
    )?;

    Ok(InitOutcome {
        root,
        nerve_dir,
        db_path,
        project_id: config.project_id,
        created: !already_initialized,
        schema_version: nerve_store::SCHEMA_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_project_ids_are_hex_and_unique() {
        let a = generate_project_id().unwrap();
        let b = generate_project_id().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "project ids must not repeat");
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
        let now = timestamp();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.ends_with('Z'));
        assert!(now.starts_with("20"), "{now}");
    }

    #[test]
    fn init_is_idempotent_and_preserves_project_id() {
        let dir = tempfile::tempdir().unwrap();
        let first = init(dir.path()).unwrap();
        assert!(first.created);
        let second = init(dir.path()).unwrap();
        assert!(!second.created);
        assert_eq!(first.project_id, second.project_id);
    }

    #[test]
    fn init_writes_the_documented_layout() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = init(dir.path()).unwrap();
        assert!(outcome.nerve_dir.join("cache").is_dir());
        assert!(outcome.nerve_dir.join("logs").is_dir());
        assert!(outcome.db_path.is_file());
        assert_eq!(
            std::fs::read_to_string(outcome.nerve_dir.join(".gitignore")).unwrap(),
            "*\n"
        );
    }

    #[test]
    fn init_refuses_a_non_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(matches!(init(&file), Err(IndexError::NotADirectory(_))));
    }

    #[test]
    fn init_refuses_to_change_an_existing_project_id() {
        let dir = tempfile::tempdir().unwrap();
        init_with_project_id(dir.path(), Some("00000000000000000000000000000001")).unwrap();
        assert!(
            init_with_project_id(dir.path(), Some("00000000000000000000000000000002")).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_is_created_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let outcome = init(dir.path()).unwrap();
        let mode = std::fs::metadata(&outcome.db_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "nerve.db must be 0600");
    }
}
