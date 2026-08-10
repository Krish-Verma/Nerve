//! Database file lifecycle.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

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

/// Open an existing database that belongs to **another repository**, read-only.
///
/// This is not [`open`] with a flag. [`open`] creates the file when it is absent, chmods it, and
/// then runs [`apply_pragmas`] — and `PRAGMA journal_mode=WAL` **writes to the database header**.
/// Pointed at a neighbour's index that is the opposite of what Slice 13a-ii's trust boundary
/// requires: Nerve would have modified the bytes of a repository the user named only as a
/// dependency, and the byte-identical-after-every-read property could not hold.
///
/// So three things are deliberately different here, and each closes one way in.
///
/// 1. **`SQLITE_OPEN_READ_ONLY` and no `SQLITE_OPEN_CREATE`.** A path that is not a database is an
///    error rather than a new empty database created inside somebody else's checkout.
/// 2. **No `SQLITE_OPEN_URI`.** With URI parsing on, a `local_path` ending
///    `…/nerve.db?mode=rwc` would re-open the connection writable — and `local_path` is a row in a
///    file on disk, which is untrusted input the moment it is written.
/// 3. **`PRAGMA query_only=ON` as well**, which is redundant against the flag by design: it is the
///    same construction `nerve check` and every read command already rely on, so a later edit that
///    reaches for a convenient repair fails at SQLite rather than at review.
///
/// No pragma that writes is applied, and the connection never runs a migration. A neighbour whose
/// schema this build does not support is refused by its caller, not upgraded — upgrading a database
/// Nerve does not own would be a write.
pub fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "query_only", "ON")?;
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
