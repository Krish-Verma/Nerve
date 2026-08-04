//! Reading the git HEAD commit without shelling out.
//!
//! SECURITY.md forbids executing anything from the repository during indexing. That includes
//! `git` itself: a repository can ship `.git/config` with `core.fsmonitor` or alias hooks, and
//! spawning a subprocess in a directory we do not trust is a needless surface. We read the
//! plumbing files directly instead. Every failure degrades to `None`; a missing or exotic git
//! layout is not an indexing error.

use std::path::{Path, PathBuf};

/// Resolve the real git directory for `root`, handling worktrees where `.git` is a file.
///
/// Public since Slice 12a: [`crate::gitobj::ObjectStore::open`] takes a resolved git directory, and
/// this is the resolution it reuses rather than reimplementing. A linked worktree's `.git` is a file
/// containing `gitdir: …`, and there is one correct way to read that.
pub fn git_dir(root: &Path) -> Option<PathBuf> {
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

/// Bytes of a `commondir` pointer file this module will read.
///
/// [`crate::gitobj::store`] bounds its own pointer-file reads and that discipline is **matched**
/// here rather than shared: the bound lives in a private constant of that module, and widening its
/// visibility would export an implementation detail of the object reader into a ref reader that has
/// no other use for it. 4 KiB is far more than the one relative path Git writes — `../..` — so a
/// `commondir` larger than this is a file Nerve declines to read rather than a repository it has to
/// understand. `read_to_string` on an unbounded path is what this exists to avoid.
const MAX_COMMONDIR_BYTES: u64 = 4096;

fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolve `<git_dir>/commondir`, which exists only in a linked worktree's private git directory.
///
/// `None` when there is no `commondir`, when it is over [`MAX_COMMONDIR_BYTES`], when it is empty or
/// carries a control character, or when it does not resolve to a directory — every one of those
/// degrading to `None` in the manner of the rest of this module, because a missing or exotic git
/// layout is not an indexing error.
///
/// The content is almost always relative (`../..`), and it is resolved against the git directory
/// that named it, which is what Git does.
fn common_dir(git_dir: &Path) -> Option<PathBuf> {
    let path = git_dir.join("commondir");
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_COMMONDIR_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    // Lossy rather than refusing outright: a target that is not UTF-8 will fail the guard below on
    // its own, and a replacement character is not a valid path component either.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let target = text.trim();
    if target.is_empty() || target.chars().any(|c| (c as u32) < 0x20) {
        return None;
    }
    let joined = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        git_dir.join(target)
    };
    let resolved = std::fs::canonicalize(&joined).ok()?;
    resolved.is_dir().then_some(resolved)
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
///
/// # `commondir` is followed, and that is a fix rather than a feature
///
/// A linked worktree keeps its own `HEAD` but **not** the branch that `HEAD` names.
/// `git worktree add` writes `refs/heads/<branch>` into the *common* git directory, and the
/// worktree's private directory has no `refs/heads/<branch>` and no `packed-refs` at all — measured
/// on a real `git worktree add`. A reader that looked only beside `HEAD` therefore answered `None`
/// for every linked worktree, and this function's one production caller feeds
/// `repository_state.git_commit`, so indexing a linked worktree recorded no commit for the state.
/// [`crate::gitobj::ObjectStore::open`] already learned this lesson for `objects/`; this is the same
/// lesson one layer up.
///
/// The private directory is consulted **first**, because Git keeps a handful of refs per worktree
/// there and a worktree's own `HEAD` must win over the common directory's.
pub fn head_commit(root: &Path) -> Option<String> {
    let git_dir = git_dir(root)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    if let Some(ref_name) = head.strip_prefix("ref:") {
        let ref_name = ref_name.trim();
        if ref_name.contains("..") || ref_name.starts_with('/') {
            return None;
        }
        let mut directories = vec![git_dir.clone()];
        if let Some(common) = common_dir(&git_dir) {
            if common != git_dir {
                directories.push(common);
            }
        }
        for directory in &directories {
            if let Ok(text) = std::fs::read_to_string(directory.join(ref_name)) {
                let object_id = text.trim();
                if is_object_id(object_id) {
                    return Some(object_id.to_string());
                }
            }
            if let Some(object_id) = read_packed_ref(directory, ref_name) {
                return Some(object_id);
            }
        }
        return None;
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

    /// The shape `git worktree add` produces: `HEAD` beside `commondir`, and the branch it names
    /// living **only** in the common directory.
    ///
    /// Before `commondir` was followed this returned `None`, which reads as "this worktree has no
    /// history" and left `repository_state.git_commit` NULL for every linked worktree.
    #[test]
    fn resolves_a_ref_that_lives_only_in_the_common_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "0f0e0d0c0b0a09080706050403020100fedcba98";
        // The common git directory, and the worktree's private one two levels below it.
        write(
            &dir.path().join("common/refs/heads/feat"),
            &format!("{sha}\n"),
        );
        std::fs::create_dir_all(dir.path().join("common/worktrees/linked")).unwrap();
        let worktree_root = dir.path().join("checkout");
        std::fs::create_dir_all(&worktree_root).unwrap();
        write(
            &worktree_root.join(".git"),
            &format!(
                "gitdir: {}\n",
                dir.path().join("common/worktrees/linked").display()
            ),
        );
        write(
            &dir.path().join("common/worktrees/linked/HEAD"),
            "ref: refs/heads/feat\n",
        );
        write(
            &dir.path().join("common/worktrees/linked/commondir"),
            "../..\n",
        );

        // The private directory has neither the loose ref nor a `packed-refs`, which is exactly
        // what a real worktree looks like. If either existed this test would prove nothing.
        assert!(!dir
            .path()
            .join("common/worktrees/linked/refs/heads/feat")
            .exists());
        assert!(!dir
            .path()
            .join("common/worktrees/linked/packed-refs")
            .exists());

        assert_eq!(head_commit(&worktree_root), Some(sha.to_string()));
    }

    /// `packed-refs` lives in the common directory too, and a worktree has none of its own.
    #[test]
    fn falls_back_to_packed_refs_in_the_common_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "1234567890abcdef1234567890abcdef12345678";
        write(
            &dir.path().join("common/packed-refs"),
            &format!("# pack-refs with: peeled fully-peeled sorted \n{sha} refs/heads/feat\n"),
        );
        std::fs::create_dir_all(dir.path().join("common/worktrees/linked")).unwrap();
        let worktree_root = dir.path().join("checkout");
        std::fs::create_dir_all(&worktree_root).unwrap();
        write(
            &worktree_root.join(".git"),
            &format!(
                "gitdir: {}\n",
                dir.path().join("common/worktrees/linked").display()
            ),
        );
        write(
            &dir.path().join("common/worktrees/linked/HEAD"),
            "ref: refs/heads/feat\n",
        );
        write(
            &dir.path().join("common/worktrees/linked/commondir"),
            "../..\n",
        );

        assert_eq!(head_commit(&worktree_root), Some(sha.to_string()));
    }

    /// A `commondir` that is empty, carries a control character, or does not resolve is ignored,
    /// and the traversal refusal on the ref name still applies with a `commondir` present.
    #[test]
    fn a_malformed_commondir_is_ignored_and_the_ref_guard_still_holds() {
        for body in ["\n", "\u{1}evil\n", "../nowhere-at-all\n"] {
            let dir = tempfile::tempdir().unwrap();
            write(&dir.path().join(".git/HEAD"), "ref: refs/heads/feat\n");
            write(&dir.path().join(".git/commondir"), body);
            assert_eq!(head_commit(dir.path()), None, "commondir {body:?}");
        }

        // A resolvable `commondir` does not license a traversing ref name.
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".git/HEAD"),
            "ref: ../../../../etc/passwd\n",
        );
        write(&dir.path().join(".git/commondir"), "..\n");
        assert_eq!(head_commit(dir.path()), None);
    }

    /// A `commondir` over the bound is not read at all, so it cannot be an allocation a repository
    /// chooses.
    #[test]
    fn an_oversized_commondir_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let sha = "aaaabbbbccccddddeeeeffff0000111122223333";
        write(
            &dir.path().join("common/refs/heads/feat"),
            &format!("{sha}\n"),
        );
        std::fs::create_dir_all(dir.path().join("common/worktrees/linked")).unwrap();
        let private = dir.path().join("common/worktrees/linked");
        let worktree_root = dir.path().join("checkout");
        std::fs::create_dir_all(&worktree_root).unwrap();
        write(
            &worktree_root.join(".git"),
            &format!("gitdir: {}\n", private.display()),
        );
        write(&private.join("HEAD"), "ref: refs/heads/feat\n");

        // Under the bound the ref resolves; one byte over it, the pointer is not read and the ref
        // is unreachable. Both directions, so the bound cannot be satisfied by never resolving.
        let padded = format!("../..{}\n", " ".repeat(16));
        write(&private.join("commondir"), &padded);
        assert_eq!(head_commit(&worktree_root), Some(sha.to_string()));

        let oversized = format!("../..{}\n", " ".repeat(MAX_COMMONDIR_BYTES as usize + 1));
        write(&private.join("commondir"), &oversized);
        assert_eq!(head_commit(&worktree_root), None);
    }
}
