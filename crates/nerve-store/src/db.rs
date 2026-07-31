//! Database file lifecycle.

use std::path::Path;

use rusqlite::Connection;

use crate::error::Result;

/// Open (creating if absent) the Nerve database and apply the standard pragmas.
///
/// A newly created file is chmod'd to `0600` on Unix before any data is written to it
/// (SECURITY.md, "Data at rest").
pub fn open(path: &Path) -> Result<Connection> {
    let is_new = !path.exists();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    if is_new {
        restrict_permissions(path)?;
    }
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Open an in-memory database. Used by tests and by the scale harness.
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

fn apply_pragmas(conn: &Connection) -> Result<()> {
    // journal_mode returns a row, so it must be queried rather than executed.
    let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    // Windows ACL handling is out of scope for Slice 1; the file inherits directory ACLs.
    Ok(())
}

/// Size of the database on disk, including the WAL and shared-memory sidecar files.
pub fn database_bytes(path: &Path) -> u64 {
    let mut total = 0;
    for suffix in ["", "-wal", "-shm"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            std::path::PathBuf::from(name)
        };
        if let Ok(meta) = std::fs::metadata(&candidate) {
            total += meta.len();
        }
    }
    total
}
