//! Two scans the cross-repository registry needs, and neither is about one function.
//!
//! **One shared service.** Availability is derived in `nerve-index/src/registry.rs` and nowhere
//! else. This is the same guard shape `crates/nerve-cli/tests/history_wording.rs` established for
//! history wording, and it exists for the same measured reason: Slice 12b left four wording
//! functions inside the CLI binary, a second surface copied them, and the two were then free to
//! drift. A registry is worse than wording, because a second derivation of *"is this neighbour
//! readable?"* is a second **answer**, not a second phrasing.
//!
//! **No user-specific absolute path is tracked by Git.** `repo_registry.local_path` is the first
//! column in this schema that is both absolute and specific to one person's machine. It lives in
//! `.nerve/nerve.db`, which `.gitignore` covers — and §9.6 of the row plan requires that be a test
//! rather than a convention.
//!
//! Both scans read **bytes**, never `read_to_string`, for the reason `docs/CONTINUATION.md:324`
//! records: a literal NUL in a source file made `grep` skip it silently and hid a real defect for a
//! whole slice. A guard that can be switched off by adding a byte is not a guard.

use std::path::{Path, PathBuf};

use nerve_core::vocab::{ContractFreshness, RegistryEntryStatus};
use nerve_index::registry::{RegistryAvailability, RegistryRefusal};

/// The one file allowed to hold a vocabulary's note, relative to `crates/`.
const CORE_VOCAB: &str = "nerve-core/src/vocab.rs";
/// The one file allowed to hold the registry service's own sentences.
const INDEX_REGISTRY: &str = "nerve-index/src/registry.rs";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Undo Rust's line-continuation and quote escapes, so a re-wrapped copy is still found.
///
/// Lifted in shape from `history_wording.rs`, and load-bearing for the same reason: a note written
/// across three source lines is one string at run time, so a raw substring scan would find nothing
/// for most sentinels — and "found nothing" is indistinguishable from "scanned nothing".
fn joined(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            match bytes.get(index + 1) {
                Some(b'\\') => {
                    out.push(b'\\');
                    index += 2;
                    continue;
                }
                Some(b'"') => {
                    out.push(b'"');
                    index += 2;
                    continue;
                }
                _ => {}
            }
            let mut after = index + 1;
            if bytes.get(after) == Some(&b'\r') {
                after += 1;
            }
            if bytes.get(after) == Some(&b'\n') {
                after += 1;
                while matches!(bytes.get(after), Some(b' ' | b'\t' | b'\r')) {
                    after += 1;
                }
                index = after;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn collect_rs(base: &Path, dir: &Path, found: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()))
    {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            collect_rs(base, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
            found.push((
                path.strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned(),
                joined(&bytes),
            ));
        }
    }
}

/// Every `.rs` file of every crate, keyed by its `crates/`-relative path.
///
/// Read from the directory rather than listed, because `layering.rs` was a hand-written array until
/// replacing it revealed the array held 16 entries against 17 real files — one module had never been
/// scanned by any of the four invariants it was subject to.
fn scanned_crates() -> Vec<(String, Vec<u8>)> {
    let mut scanned = Vec::new();
    for crate_name in [
        "nerve-cli",
        "nerve-server",
        "nerve-core",
        "nerve-store",
        "nerve-index",
    ] {
        let src = workspace_root().join("crates").join(crate_name).join("src");
        let mut found = Vec::new();
        collect_rs(&src, &src, &mut found);
        found.sort();
        for (name, bytes) in found {
            scanned.push((format!("{crate_name}/src/{name}"), bytes));
        }
    }
    scanned.sort();
    scanned
}

fn owner_source(owner: &str) -> Vec<u8> {
    let path = workspace_root().join("crates").join(owner);
    joined(&std::fs::read(&path).unwrap_or_else(|error| panic!("{owner}: {error}")))
}

/// Every sentence the registry surfaces render, paired with the one file allowed to hold it.
///
/// Generated by calling the methods that actually ship, never retyped: a retyped sentinel is free to
/// drift from the product, which is the class of defect this file exists to close.
fn sentences() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for value in ContractFreshness::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in RegistryEntryStatus::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in RegistryRefusal::ALL {
        out.push((INDEX_REGISTRY, value.statement()));
    }
    for value in availability_values() {
        out.push((INDEX_REGISTRY, value.statement()));
    }
    out
}

/// One representative of every [`RegistryAvailability`] variant.
///
/// There is no `ALL` const because two variants carry a payload, so the list is built here — and
/// [`the_registry_sentences_exist_in_exactly_one_file`] asserts the count, so a seventh variant
/// added without a sentence fails rather than being silently unscanned.
fn availability_values() -> Vec<RegistryAvailability> {
    vec![
        RegistryAvailability::Available,
        RegistryAvailability::PartiallyIndexed,
        RegistryAvailability::EntryRemoved,
        RegistryAvailability::Missing(RegistryRefusal::PathDoesNotExist),
        RegistryAvailability::Moved {
            observed_repository_id: "repo_other".into(),
        },
        RegistryAvailability::Refused(RegistryRefusal::SymlinkEscape),
    ]
}

/// No file but the one that owns a registry sentence may contain it as a literal.
#[test]
fn the_registry_sentences_exist_in_exactly_one_file() {
    let sentences = sentences();
    assert_eq!(
        sentences.len(),
        ContractFreshness::ALL.len()
            + RegistryEntryStatus::ALL.len()
            + RegistryRefusal::ALL.len()
            + availability_values().len(),
        "a vocabulary lost a value between its list and its sentence"
    );
    assert!(
        sentences.len() >= 32,
        "found only {} sentences",
        sentences.len()
    );
    for owner in [CORE_VOCAB, INDEX_REGISTRY] {
        assert!(
            sentences.iter().any(|(holder, _)| *holder == owner),
            "no sentence is owned by {owner}, so the per-file rule is not being exercised"
        );
    }

    // Anti-vacuity 1: every sentinel exists in the file allowed to hold it. Renaming a sentence
    // without moving it would otherwise make every negative below pass for free.
    for (owner, sentence) in &sentences {
        assert!(
            contains(&owner_source(owner), sentence.as_bytes()),
            "{owner} does not contain {sentence:?}, so searching every other file for it proves \
             nothing"
        );
    }

    // Anti-vacuity 2: the walk really walked, and reached the files by name.
    let scanned = scanned_crates();
    assert!(
        scanned.len() >= 85,
        "expected to scan five crates, found {} files",
        scanned.len()
    );
    for required in [
        "nerve-cli/src/main.rs",
        "nerve-server/src/api/history.rs",
        "nerve-store/src/registry.rs",
        CORE_VOCAB,
        INDEX_REGISTRY,
    ] {
        assert!(
            scanned.iter().any(|(name, _)| name == required),
            "{required} was not scanned"
        );
    }

    for (name, bytes) in &scanned {
        for (owner, sentence) in &sentences {
            if name == owner {
                continue;
            }
            assert!(
                !contains(bytes, sentence.as_bytes()),
                "{name} contains a registry sentence as a literal: {sentence:?}. That sentence \
                 lives in {owner} — call the method. A second copy is free to drift from the rule \
                 it renders."
            );
        }
    }
}

/// **No surface decides for itself whether a neighbour is readable.**
///
/// The tokens are the shapes a second derivation would have to be written in: naming a
/// [`ContractFreshness`] or a [`RegistryAvailability`] variant, opening a neighbour's database, or
/// comparing a found repository id against the recorded one. Each is asserted present in the file
/// that owns it first, so a renamed API cannot make the whole scan vacuous.
#[test]
fn no_surface_derives_registry_availability_of_its_own() {
    // `nerve_store::open_read_only` is spelled in full rather than bare: `nerve-server` has a
    // private `open_read_only` for **its own** database, which is a different thing and always was.
    // A token that matched it would have made this scan a false positive on day one.
    const FORBIDDEN: [&str; 6] = [
        "ContractFreshness::",
        "RegistryAvailability::",
        "nerve_store::open_read_only",
        "!= entry.expected_repository_id",
        "expected_repository_id !=",
        "expected_repository_id ==",
    ];

    // Anti-vacuity: the owner really is written in these shapes. Without this, renaming
    // `open_read_only` would silently retire a third of the scan.
    let owner = owner_source(INDEX_REGISTRY);
    let present = FORBIDDEN
        .iter()
        .filter(|token| contains(&owner, token.as_bytes()))
        .count();
    assert!(
        present >= 4,
        "only {present} of the forbidden shapes exist in {INDEX_REGISTRY}; the scan below is \
         mostly searching for nothing"
    );

    let mut scanned = 0;
    for (name, bytes) in scanned_crates() {
        if !name.starts_with("nerve-cli/") && !name.starts_with("nerve-server/") {
            continue;
        }
        scanned += 1;
        for token in FORBIDDEN {
            assert!(
                !contains(&bytes, token.as_bytes()),
                "{name} derives registry availability itself: {token:?}. Call \
                 `nerve_index::registry` — a second derivation of whether a neighbour is readable \
                 is a second answer, not a second phrasing."
            );
        }
    }
    assert!(scanned >= 20, "only {scanned} surface files scanned");

    // The positive half. The CLI has to be *calling* the service it is forbidden to reimplement, or
    // this test is satisfied by a surface that answers nothing.
    let main = std::fs::read(workspace_root().join("crates/nerve-cli/src/main.rs")).unwrap();
    for required in [
        "nerve_index::availability_of",
        "nerve_index::list_registry",
        "nerve_index::add_registry_target",
        "nerve_index::relocate_registry_target",
        "nerve_index::remove_registry_target",
    ] {
        assert!(
            contains(&main, required.as_bytes()),
            "the CLI does not call {required}, so the shared service reaches nobody"
        );
    }
}

// ---- §9.6: no user-specific absolute path is tracked by Git ------------------------------------

/// Directories Git does not track, from `.gitignore` plus `.git` itself.
const UNTRACKED_DIRS: [&str; 7] = [
    ".git",
    "target",
    ".nerve",
    "node_modules",
    "dist",
    "__pycache__",
    ".pytest_cache",
];

/// Files that already carry a home-shaped path, each with the reason it is allowed to.
///
/// **This list is an exemption, not a licence.** Both remaining entries are *illustrative* — they
/// spell out an example home path precisely in order to explain why an absolute one must never be
/// recorded — and neither is a value a program wrote. A **new** file carrying one, of any kind,
/// fails.
///
/// Each entry is checked to still exist and to still contain a hit, so an exemption that stopped
/// being needed fails rather than quietly widening the rule.
///
/// **Two entries were removed rather than kept, 2026-08-08.** `docs/CONTINUATION.md` and
/// `docs/reports/restart-recovery-report.md` carried **this machine's real home path** — not an
/// illustration, the actual checkout — in session prose and in a recovery report's header table.
/// The guard was written with them exempted, which is the reasonable thing to do when the scan is
/// the new part and the files are old. It is still the wrong resolution: `CLAUDE.md` §8 says no
/// user-specific data is committed, and an exemption is a record that the rule is being broken
/// somewhere agreed in advance. The paths were replaced with `/path/to/Nerve` and `<checkout root>`,
/// which cost the documents nothing — neither was making a point about *that* directory — and the
/// exemptions went with them. **Prefer deleting the value over widening the list.**
///
/// **This test file is not on the list, and must never be.** Its own probes are assembled at run
/// time from fragments (see [`home_needle`]) rather than written as literals, because a guard that
/// had to exempt itself would be one edit away from exempting the thing it guards.
const ALLOWED: [(&str, &str); 2] = [
    (
        "docs/plans/slice-11b-python-tracer.md",
        "an illustrative example home path explaining why the tracer never records one",
    ),
    (
        "tracers/python/nerve_trace/paths.py",
        "a docstring whose subject is refusing to emit an absolute path",
    ),
];

/// Does `bytes` contain something shaped like a home directory?
///
/// Shape rather than a fixed name, because the value being guarded against is *the machine this ran
/// on*, which no committed pattern can know.
fn home_shaped(bytes: &[u8]) -> bool {
    for prefix in [b"/Users/".as_slice(), b"/home/".as_slice()] {
        let mut index = 0;
        while index + prefix.len() < bytes.len() {
            if bytes[index..].starts_with(prefix) {
                // A name, then another separator. The prefix followed by a user name and a
                // slash is a home directory; the bare prefix is not.
                let rest = &bytes[index + prefix.len()..];
                let name: Vec<u8> = rest
                    .iter()
                    .copied()
                    .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
                    .collect();
                if !name.is_empty() && rest.get(name.len()) == Some(&b'/') {
                    return true;
                }
            }
            index += 1;
        }
    }
    // Built rather than written, for the reason [`home_needle`] gives: a literal here would make
    // this file its own first hit.
    let windows = format!("C:{}Users{}", '\\', '\\');
    contains(bytes, windows.as_bytes())
}

/// Every file Git could be tracking, as `(repository-relative path, bytes)`.
fn tracked_like_files() -> Vec<(String, Vec<u8>)> {
    let root = workspace_root();
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if !UNTRACKED_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            if name == ".DS_Store" {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap_or_default();
            found.push((
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                bytes,
            ));
        }
    }
    found.sort();
    found
}

/// A probe for [`home_shaped`], assembled at run time so this file contains no literal home path.
///
/// The alternative is exempting this file from its own scan, which is how a guard stops guarding:
/// the exemption would then cover anything a later edit added here.
fn home_needle(root: &str, user: &str) -> Vec<u8> {
    format!("prefix {root}{user}/project/src/x.py suffix").into_bytes()
}

/// **§9.6.** No file Git tracks carries a path that names somebody's home directory.
///
/// `local_path` is the reason this test exists: it is absolute, it is specific to one machine, and
/// it is written by a command rather than typed into a document. It lives in `.nerve/nerve.db`,
/// which `.gitignore:2` covers — and this asserts that rather than trusting it.
#[test]
fn no_user_specific_absolute_path_is_tracked_by_git() {
    // Anti-vacuity 1: the matcher decides. A scan whose predicate always answered "no" would pass
    // over any repository at all.
    assert!(home_shaped(&home_needle("/Users", "/someone")));
    assert!(home_shaped(&home_needle("/home", "/ci-runner")));
    assert!(home_shaped(
        format!("C:{}Users{}dev", '\\', '\\').as_bytes()
    ));
    assert!(!home_shaped(b"/usr/local/share is not a home directory"));
    assert!(!home_shaped(b"crates/nerve-cli/src/main.rs"));

    let files = tracked_like_files();
    // Anti-vacuity 2: the walk really walked. 699 is the measured count; a floor, so the repository
    // may grow, but tight enough that a broken walk cannot pass.
    assert!(
        files.len() >= 600,
        "expected to scan the repository, found {} files",
        files.len()
    );
    for required in [
        "crates/nerve-store/src/registry.rs",
        "crates/nerve-index/src/registry.rs",
        "scripts/final_acceptance.sh",
        "Cargo.lock",
        ".gitignore",
    ] {
        assert!(
            files.iter().any(|(name, _)| name == required),
            "{required} was not scanned"
        );
    }

    // Anti-vacuity 3: every exemption is still earning its place.
    for (allowed, reason) in ALLOWED {
        let entry = files
            .iter()
            .find(|(name, _)| name == allowed)
            .unwrap_or_else(|| panic!("the exempted file {allowed} no longer exists ({reason})"));
        assert!(
            home_shaped(&entry.1),
            "{allowed} no longer carries a home-shaped path — delete its exemption ({reason})"
        );
    }

    for (name, bytes) in &files {
        if ALLOWED.iter().any(|(allowed, _)| allowed == name) {
            continue;
        }
        assert!(
            !home_shaped(bytes),
            "{name} carries a path naming somebody's home directory. A user-specific absolute \
             path belongs in .nerve/, which .gitignore covers, and nowhere Git can see."
        );
    }

    // And the mechanism that keeps `local_path` out of Git is the one being relied on.
    let gitignore = std::fs::read_to_string(workspace_root().join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|line| line.trim() == ".nerve/"),
        "`.nerve/` is not ignored, so the registry's absolute paths are tracked: {gitignore}"
    );
}

/// The registry writes its absolute path into `.nerve/` and nowhere else.
///
/// The scan above proves the repository is clean today. This proves the *mechanism*: run the command
/// that records an absolute path, then look for that exact path everywhere in the tree. It must be
/// findable in `.nerve/nerve.db` — otherwise the search is not capable of finding it — and nowhere
/// outside.
#[test]
fn a_registered_absolute_path_lands_only_inside_the_ignored_directory() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = workspace_root().join("fixtures");
    let build = |name: &str, fixture: &str| {
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        for entry in std::fs::read_dir(fixtures.join(fixture)).unwrap() {
            let from = entry.unwrap().path();
            if from.is_file() {
                std::fs::copy(&from, root.join(from.file_name().unwrap())).unwrap();
            }
        }
        nerve_index::init(&root).unwrap();
        nerve_index::index_repository(&root).unwrap();
        root
    };
    let a = build("a", "ts-basic");
    let b = build("b", "ts-resolution");

    let conn = nerve_store::open(&nerve_index::config::db_path(&a)).unwrap();
    let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    assert!(matches!(
        nerve_index::add_registry_target(&conn, &repo_id, &b, Some("n"), None).unwrap(),
        nerve_index::RegistryOutcome::Done(_)
    ));
    drop(conn);

    let needle = b.canonicalize().unwrap().to_string_lossy().into_owned();
    let mut inside = 0;
    let mut outside = Vec::new();
    let mut stack = vec![a.clone()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !contains(&std::fs::read(&path).unwrap_or_default(), needle.as_bytes()) {
                continue;
            }
            let relative = path
                .strip_prefix(&a)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if relative.starts_with(".nerve/") {
                inside += 1;
            } else {
                outside.push(relative);
            }
        }
    }
    assert!(
        inside > 0,
        "the registered path was not found even inside .nerve/, so this search proves nothing"
    );
    assert!(
        outside.is_empty(),
        "a user-specific absolute path was written outside the ignored directory: {outside:?}"
    );
    // And `nerve init` covers that directory from the inside as well as from the root .gitignore.
    assert_eq!(
        std::fs::read_to_string(a.join(".nerve/.gitignore")).unwrap(),
        "*\n"
    );
}
