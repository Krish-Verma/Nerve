//! Path safety and exclusion rules (SECURITY.md).

mod common;

use std::path::{Path, PathBuf};

use nerve_index::config::{Config, IndexSettings, SecuritySettings, DEFAULT_MAX_FILE_BYTES};
use nerve_index::discover::{canonical_child, discover, relative_path};
use nerve_index::IndexError;

fn write(root: &Path, rel_path: &str, contents: &str) {
    let path = root.join(rel_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn config() -> Config {
    Config {
        schema_version: 1,
        project_id: common::TEST_PROJECT_ID.to_string(),
        created_at: "2026-07-31T00:00:00Z".to_string(),
        index: IndexSettings::default(),
        security: SecuritySettings::default(),
    }
}

fn discovered(root: &Path, config: &Config) -> Vec<String> {
    discover(root, config)
        .unwrap()
        .files
        .into_iter()
        .map(|file| file.rel_path)
        .collect()
}

// ---- path safety -------------------------------------------------------------------------

#[test]
fn traversal_out_of_the_root_is_refused() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    write(outer.path(), "outside.ts", "export const secret = 1;\n");
    let root = std::fs::canonicalize(&root).unwrap();

    let err = canonical_child(&root, Path::new("../outside.ts")).unwrap_err();
    assert!(matches!(err, IndexError::PathEscapesRoot(_)), "{err}");

    let err = canonical_child(&root, Path::new("a/../../outside.ts")).unwrap_err();
    assert!(matches!(err, IndexError::PathEscapesRoot(_)), "{err}");
}

#[test]
fn an_absolute_path_outside_the_root_is_refused() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    write(outer.path(), "outside.ts", "export const secret = 1;\n");
    let root = std::fs::canonicalize(&root).unwrap();

    let absolute = std::fs::canonicalize(outer.path().join("outside.ts")).unwrap();
    let err = canonical_child(&root, &absolute).unwrap_err();
    assert!(matches!(err, IndexError::PathEscapesRoot(_)), "{err}");
}

#[test]
fn a_nul_byte_in_a_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let crafted = PathBuf::from("a\0b.ts");
    let err = canonical_child(&root, &crafted).unwrap_err();
    assert!(matches!(err, IndexError::PathEscapesRoot(_)), "{err}");
}

#[test]
fn a_path_inside_the_root_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let child = canonical_child(&root, Path::new("src/a.ts")).unwrap();
    assert_eq!(relative_path(&root, &child).unwrap(), "src/a.ts");
}

#[cfg(unix)]
#[test]
fn a_symlink_escaping_the_root_is_not_followed() {
    let outer = tempfile::tempdir().unwrap();
    write(
        outer.path(),
        "outside.ts",
        "export function leakedSymbol() { return 1; }\n",
    );
    let root = outer.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "src/inside.ts", "export const inside = 1;\n");
    std::os::unix::fs::symlink(outer.path().join("outside.ts"), root.join("src/linked.ts"))
        .unwrap();

    let root = std::fs::canonicalize(&root).unwrap();
    let report = discover(&root, &config()).unwrap();
    let paths: Vec<String> = report.files.into_iter().map(|f| f.rel_path).collect();
    assert_eq!(paths, vec!["src/inside.ts".to_string()]);
    assert_eq!(report.skipped_symlinks, 1);
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_escaping_the_root_is_not_followed() {
    let outer = tempfile::tempdir().unwrap();
    write(outer.path(), "vendor/leak.ts", "export const leak = 1;\n");
    let root = outer.path().join("repo");
    std::fs::create_dir_all(root.join("src")).unwrap();
    write(&root, "src/inside.ts", "export const inside = 1;\n");
    std::os::unix::fs::symlink(outer.path().join("vendor"), root.join("src/vendor")).unwrap();

    let root = std::fs::canonicalize(&root).unwrap();
    let paths = discovered(&root, &config());
    assert_eq!(paths, vec!["src/inside.ts".to_string()]);
}

#[cfg(unix)]
#[test]
fn a_symlinked_escape_contributes_nothing_to_the_graph() {
    let outer = tempfile::tempdir().unwrap();
    write(
        outer.path(),
        "outside.ts",
        "export function leakedSymbol() { return 1; }\n",
    );
    let root = outer.path().join("repo");
    std::fs::create_dir_all(root.join("src")).unwrap();
    write(&root, "src/inside.ts", "export const inside = 1;\n");
    std::os::unix::fs::symlink(outer.path().join("outside.ts"), root.join("src/linked.ts"))
        .unwrap();

    nerve_index::init_with_project_id(&root, Some(common::TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();

    let conn = common::open_db(&root);
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity WHERE name = 'leakedSymbol'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits, 0);
}

// ---- exclusion rules ---------------------------------------------------------------------

#[test]
fn gitignore_is_respected_without_a_git_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".gitignore", "dist/\ngenerated.ts\n");
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    write(dir.path(), "src/generated.ts", "export const g = 1;\n");
    write(dir.path(), "dist/bundle.js", "export const b = 1;\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    assert_eq!(discovered(&root, &config()), vec!["src/a.ts".to_string()]);
}

#[test]
fn nested_and_negated_gitignore_patterns_are_respected() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".gitignore", "*.gen.ts\n!keep.gen.ts\n");
    write(dir.path(), "src/.gitignore", "local/\n");
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    write(dir.path(), "src/drop.gen.ts", "export const d = 1;\n");
    write(dir.path(), "src/keep.gen.ts", "export const k = 1;\n");
    write(dir.path(), "src/local/hidden.ts", "export const h = 1;\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    assert_eq!(
        discovered(&root, &config()),
        vec!["src/a.ts".to_string(), "src/keep.gen.ts".to_string()]
    );
}

#[test]
fn nerveignore_is_respected() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".nerveignore", "vendored/\n*.spec.ts\n");
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    write(dir.path(), "src/a.spec.ts", "export const s = 1;\n");
    write(dir.path(), "vendored/lib.ts", "export const l = 1;\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    assert_eq!(discovered(&root, &config()), vec!["src/a.ts".to_string()]);
}

#[test]
fn the_secret_deny_list_excludes_even_tracked_source_files() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    // A denied name that is also a perfectly parseable TypeScript file.
    write(
        dir.path(),
        "src/secrets.ts",
        "export const apiToken = 'nerve-fixture-token';\n",
    );
    write(dir.path(), ".env", "TOKEN=fixture\n");
    write(dir.path(), ".npmrc", "//registry:_authToken=fixture\n");
    write(dir.path(), "server.key", "fixture\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    let report = discover(&root, &config()).unwrap();
    let paths: Vec<String> = report.files.iter().map(|f| f.rel_path.clone()).collect();
    assert_eq!(paths, vec!["src/a.ts".to_string()]);
    assert_eq!(
        report.denied_secrets,
        vec![
            ".env".to_string(),
            ".npmrc".to_string(),
            "server.key".to_string(),
            "src/secrets.ts".to_string(),
        ]
    );
}

#[test]
fn a_denied_source_file_contributes_no_entities() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/a.ts", "export const a = 1;\n");
    write(
        root,
        "src/secrets.ts",
        "export function readApiToken() { return 'fixture'; }\n",
    );
    nerve_index::init_with_project_id(root, Some(common::TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(root).unwrap();

    let conn = common::open_db(root);
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity WHERE name = 'readApiToken'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits, 0);
}

#[test]
fn user_supplied_deny_patterns_are_honoured() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    write(dir.path(), "src/vault.ts", "export const v = 1;\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    let mut config = config();
    config.security.extra_deny_patterns = vec!["vault.*".to_string()];
    assert_eq!(discovered(&root, &config), vec!["src/a.ts".to_string()]);
}

#[test]
fn node_modules_dot_git_and_dot_nerve_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    write(
        dir.path(),
        "node_modules/pkg/index.js",
        "export const p = 1;\n",
    );
    write(
        dir.path(),
        ".git/hooks/pre-commit.js",
        "export const g = 1;\n",
    );
    write(dir.path(), ".nerve/cache/stale.ts", "export const c = 1;\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    assert_eq!(discovered(&root, &config()), vec!["src/a.ts".to_string()]);
}

#[test]
fn unsupported_extensions_are_skipped_not_failed() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.ts", "export const a = 1;\n");
    write(dir.path(), "README.md", "# hello\n");
    write(dir.path(), "data.json", "{}\n");
    let root = std::fs::canonicalize(dir.path()).unwrap();

    let report = discover(&root, &config()).unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.skipped_unsupported, 2);
}

#[test]
fn oversized_files_are_skipped_and_counted_as_failures() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/a.ts", "export const a = 1;\n");
    let filler = "// ".to_string() + &"x".repeat(4096) + "\n";
    let big = filler.repeat(600); // > 2 MiB
    assert!(big.len() as u64 > DEFAULT_MAX_FILE_BYTES);
    write(root, "src/big.ts", &big);

    nerve_index::init_with_project_id(root, Some(common::TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(root).unwrap();
    assert_eq!(outcome.files_processed, 1);
    assert_eq!(outcome.files_failed, 1);
    assert_eq!(outcome.status, nerve_index::RunStatus::Partial);
}

#[test]
fn non_utf8_files_are_skipped_and_counted_as_failures() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/a.ts", "export const a = 1;\n");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/latin1.ts"), [0xff, 0xfe, 0x41, 0x42]).unwrap();

    nerve_index::init_with_project_id(root, Some(common::TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(root).unwrap();
    assert_eq!(outcome.files_processed, 1);
    assert_eq!(outcome.files_failed, 1);
    assert_eq!(outcome.status, nerve_index::RunStatus::Partial);
}

#[test]
fn the_index_directory_is_git_ignored_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let outcome = nerve_index::init(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(outcome.nerve_dir.join(".gitignore")).unwrap(),
        "*\n"
    );
}
