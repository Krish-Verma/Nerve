//! Reading the git HEAD commit without shelling out.
//!
//! SECURITY.md forbids executing anything from the repository during indexing. That includes
//! `git` itself: a repository can ship `.git/config` with `core.fsmonitor` or alias hooks, and
//! spawning a subprocess in a directory we do not trust is a needless surface. We read the
//! plumbing files directly instead. Every failure degrades to `None`; a missing or exotic git
//! layout is not an indexing error.

use std::path::{Path, PathBuf};

/// Resolve the real git directory for `root`, handling worktrees where `.git` is a file.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let metadata = std::fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git);
    }
    if metadata.is_file() {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text.strip_prefix("gitdir:")?.trim();
        let path = Path::new(target);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if resolved.is_dir() {
            return Some(resolved);
        }
    }
    None
}

fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn read_packed_ref(git_dir: &Path, ref_name: &str) -> Option<String> {
    let text = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let (object_id, name) = line.split_once(' ')?;
        if name == ref_name && is_object_id(object_id) {
            return Some(object_id.to_string());
        }
    }
    None
}

/// Resolve the commit `HEAD` points at, or `None` when there is no readable git state.
pub fn head_commit(root: &Path) -> Option<String> {
    let git_dir = git_dir(root)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    if let Some(ref_name) = head.strip_prefix("ref:") {
        let ref_name = ref_name.trim();
        if ref_name.contains("..") || ref_name.starts_with('/') {
            return None;
        }
        let ref_path = git_dir.join(ref_name);
        if let Ok(text) = std::fs::read_to_string(&ref_path) {
            let object_id = text.trim();
            if is_object_id(object_id) {
                return Some(object_id.to_string());
            }
        }
        return read_packed_ref(&git_dir, ref_name);
    }

    // Detached HEAD.
    is_object_id(head).then(|| head.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn absent_git_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(head_commit(dir.path()), None);
    }

    #[test]
    fn reads_symbolic_head_through_a_loose_ref() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "0123456789abcdef0123456789abcdef01234567";
        write(&dir.path().join(".git/HEAD"), "ref: refs/heads/main\n");
        write(
            &dir.path().join(".git/refs/heads/main"),
            &format!("{sha}\n"),
        );
        assert_eq!(head_commit(dir.path()), Some(sha.to_string()));
    }

    #[test]
    fn falls_back_to_packed_refs() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        write(&dir.path().join(".git/HEAD"), "ref: refs/heads/main\n");
        write(
            &dir.path().join(".git/packed-refs"),
            &format!("# pack-refs with: peeled fully-peeled sorted \n{sha} refs/heads/main\n"),
        );
        assert_eq!(head_commit(dir.path()), Some(sha.to_string()));
    }

    #[test]
    fn reads_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "fedcba9876543210fedcba9876543210fedcba98";
        write(&dir.path().join(".git/HEAD"), &format!("{sha}\n"));
        assert_eq!(head_commit(dir.path()), Some(sha.to_string()));
    }

    #[test]
    fn rejects_traversal_in_a_crafted_head() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".git/HEAD"),
            "ref: ../../../../etc/passwd\n",
        );
        assert_eq!(head_commit(dir.path()), None);
    }

    #[test]
    fn ignores_a_non_object_id() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".git/HEAD"), "not-a-sha\n");
        assert_eq!(head_commit(dir.path()), None);
    }
}
