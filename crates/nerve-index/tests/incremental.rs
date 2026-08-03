//! Incremental indexing: invalidation, deletion, identity links, and the property that makes
//! all three trustworthy.
//!
//! The load-bearing test here is [`incremental_and_full_agree_under_a_seeded_edit_sequence`].
//! Everything else in this file checks a mechanism; that one checks the outcome. If an
//! incremental re-index and a from-scratch index of the same tree ever disagree, the incremental
//! one is wrong by definition — there is no reading in which the cheap path is the correct one.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::{copy_tree, count, named_fixture_copy, named_fixture_root, open_db, TEST_PROJECT_ID};

use nerve_index::{IndexOptions, IndexOutcome};

// ---- helpers -------------------------------------------------------------------------------

fn dump_json(root: &Path) -> String {
    let conn = open_db(root);
    nerve_store::canonical_dump(&conn)
        .unwrap()
        .to_canonical_json()
        .unwrap()
}

fn index(root: &Path) -> IndexOutcome {
    nerve_index::index_repository(root).unwrap()
}

fn index_full(root: &Path) -> IndexOutcome {
    nerve_index::index_repository_with(root, IndexOptions { full: true }).unwrap()
}

/// Index a copy of `tree` into a brand-new database and return its canonical dump.
///
/// This is the reference every incremental run is measured against: what Nerve would say about
/// the tree if it had never seen anything else.
fn dump_of_a_from_scratch_index(tree: &Path) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(tree, &root);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();
    let dump = dump_json(&root);
    (dir, dump)
}

fn indexed_incremental_fixture() -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = named_fixture_copy("ts-incremental");
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();
    (dir, root)
}

fn write(root: &Path, rel_path: &str, contents: &str) {
    let path = root.join(rel_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn remove(root: &Path, rel_path: &str) {
    std::fs::remove_file(root.join(rel_path)).unwrap();
}

fn paths_touched(conn: &nerve_store::Connection, table: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT DISTINCT file_path FROM {table}"))
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn scalar(conn: &nerve_store::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Where a `CALLS` edge from a named source symbol lands, as `scope_path#name`.
fn call_targets(conn: &nerve_store::Connection, source_name: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.scope_path || '#' || t.name
               FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE a.relation = 'CALLS' AND s.name = ?1
              ORDER BY 1",
        )
        .unwrap();
    stmt.query_map([source_name], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

// ---- idempotence ---------------------------------------------------------------------------

/// Acceptance criterion 6: re-indexing an unchanged tree does no extraction work.
#[test]
fn an_unchanged_tree_re_extracts_nothing_and_grows_no_table() {
    let (_dir, root) = indexed_incremental_fixture();
    let tables = [
        "entity",
        "occurrence",
        "assertion",
        "observation",
        "assertion_state",
        "repository_state",
        "module_facts",
        "identity_link",
    ];
    let before: BTreeMap<&str, i64> = {
        let conn = open_db(&root);
        tables.iter().map(|t| (*t, count(&conn, t))).collect()
    };
    let dump_before = dump_json(&root);

    let outcome = index(&root);

    assert_eq!(outcome.incremental.files_re_extracted, 0, "nothing changed");
    assert_eq!(
        outcome.incremental.files_skipped_unchanged,
        outcome.files_processed
    );
    assert_eq!(outcome.incremental.files_changed(), 0);
    assert_eq!(outcome.incremental.amplification(), None);
    assert!(outcome.incremental.removed_paths.is_empty());
    assert_eq!(outcome.incremental.observations_removed, 0);
    assert_eq!(outcome.incremental.entities_removed, 0);

    let conn = open_db(&root);
    for table in tables {
        assert_eq!(count(&conn, table), before[table], "{table} grew");
    }
    assert_eq!(dump_json(&root), dump_before, "the graph changed");
}

/// A re-index that does no extraction must still leave a database equal to a fresh one.
#[test]
fn an_unchanged_re_index_still_equals_a_from_scratch_index() {
    let (_dir, root) = indexed_incremental_fixture();
    index(&root);
    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

// ---- invalidation soundness ----------------------------------------------------------------

/// Acceptance criterion 4, and the whole reason the invalidation set is a graph walk.
///
/// `app.ts` imports `./barrel`; `barrel.ts` does `export * from './impl'`. `app.ts` never names
/// `impl.ts`, so "the changed file and its direct importers" would stop at `barrel.ts` and leave
/// `app.ts` holding a resolution that no longer exists.
#[test]
fn editing_a_module_behind_a_barrel_re_extracts_its_importers_transitively() {
    let (_dir, root) = indexed_incremental_fixture();

    {
        let conn = open_db(&root);
        // `aid` is an alias; the resolved target keeps its defining module's identity, so the
        // call lands on `assist` itself (ADR-0002: barrel files do not clone entities).
        assert_eq!(call_targets(&conn, "run"), vec!["#assist", "#helper"]);
    }

    // Rename the symbol the barrel forwards. `app.ts` must stop resolving it.
    write(
        &root,
        "src/impl.ts",
        "export function renamedHelper(): number {\n  return 41;\n}\n\nexport function secondary(): number {\n  return renamedHelper() + 1;\n}\n",
    );

    let outcome = index(&root);
    assert_eq!(
        outcome.incremental.files_re_extracted, 3,
        "impl, barrel and app — and nothing else"
    );
    assert_eq!(outcome.incremental.files_changed(), 1);
    assert_eq!(outcome.incremental.amplification(), Some(3.0));

    let conn = open_db(&root);
    // The stale resolution is gone, not merely superseded.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE name = 'helper' AND kind = 'function'"
        ),
        0,
        "the renamed function's old entity survived"
    );
    // `helper` is now an unresolved *value* in app.ts, scoped to the importing file — an
    // honest "Nerve cannot name this", not a dangling edge to a symbol that no longer exists.
    let targets = call_targets(&conn, "run");
    assert!(
        targets.contains(&"src/app.ts#helper".to_string()),
        "app must record `helper` as unresolved — got {targets:?}"
    );
    assert!(targets.contains(&"#assist".to_string()));

    // The island is untouched, which is what makes this incremental rather than a re-index.
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM entity WHERE name = 'alone'"),
        1
    );

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(
        dump_json(&root),
        reference,
        "a barrel-chain edit left the incremental graph different from a full one"
    );
}

/// The control: a module nothing imports invalidates only itself.
#[test]
fn editing_an_island_re_extracts_only_that_file() {
    let (_dir, root) = indexed_incremental_fixture();
    write(
        &root,
        "src/island.ts",
        "export function alone(): number {\n  return 2;\n}\n",
    );
    let outcome = index(&root);
    assert_eq!(outcome.incremental.files_re_extracted, 1);
    assert_eq!(outcome.incremental.amplification(), Some(1.0));

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

/// Adding a file can make a previously unresolved specifier resolve, in a module whose bytes did
/// not change and whose only `IMPORTS` edge points at an `Unresolved` entity. The graph walk
/// cannot find that module; specifier re-resolution must.
#[test]
fn adding_a_file_that_satisfies_a_dangling_import_re_resolves_the_importer() {
    let (_dir, root) = indexed_incremental_fixture();
    write(
        &root,
        "src/consumer.ts",
        "import { future } from './future';\n\nexport function useIt(): number {\n  return future();\n}\n",
    );
    index(&root);
    {
        let conn = open_db(&root);
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM entity WHERE kind='unresolved' AND name='./future'"
            ),
            1,
            "the import should start out unresolved"
        );
    }

    write(
        &root,
        "src/future.ts",
        "export function future(): number {\n  return 5;\n}\n",
    );
    let outcome = index(&root);

    assert!(
        outcome.incremental.files_resolution_changed >= 1,
        "the importer must be seeded by specifier re-resolution, got {:?}",
        outcome.incremental
    );

    let conn = open_db(&root);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE kind='unresolved' AND name='./future'"
        ),
        0,
        "the unresolved entity must be gone once the module exists"
    );
    assert_eq!(call_targets(&conn, "useIt"), vec!["#future"]);

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

// ---- deletion ------------------------------------------------------------------------------

/// Acceptance criterion 3. Before Slice 3 this was the product's worst defect: nothing removed a
/// row, so a deleted file's graph was not merely stale, it was wrong.
#[test]
fn deleting_a_file_removes_its_entities_assertions_and_observations() {
    let (_dir, root) = indexed_incremental_fixture();

    let defined_helper = "SELECT count(*) FROM entity WHERE name='helper' AND kind='function'";
    let before = {
        let conn = open_db(&root);
        assert!(paths_touched(&conn, "occurrence").contains("src/impl.ts"));
        assert_eq!(scalar(&conn, defined_helper), 1);
        nerve_store::status(&conn).unwrap()
    };

    remove(&root, "src/impl.ts");
    let outcome = index(&root);

    assert_eq!(outcome.incremental.files_removed, 1);
    assert_eq!(outcome.incremental.removed_paths, vec!["src/impl.ts"]);
    assert!(
        outcome.incremental.observations_removed > 0,
        "a deletion that removed no evidence removed nothing"
    );
    assert!(outcome.incremental.entities_removed > 0);

    let conn = open_db(&root);
    for table in ["occurrence", "observation"] {
        assert!(
            !paths_touched(&conn, table).contains("src/impl.ts"),
            "{table} still holds rows for the deleted file"
        );
    }
    assert_eq!(
        scalar(&conn, defined_helper),
        0,
        "the deleted file's symbols are still queryable"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE name='secondary' AND kind='function'"
        ),
        0
    );

    let after = nerve_store::status(&conn).unwrap();
    assert!(
        after.entities_total < before.entities_total,
        "status counts must fall: {} -> {}",
        before.entities_total,
        after.entities_total
    );
    assert!(after.assertions_total < before.assertions_total);
    assert!(after.observations_total < before.observations_total);

    // No assertion may survive without evidence, and no entity without a place or an edge.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion a
              WHERE NOT EXISTS (SELECT 1 FROM observation o WHERE o.assertion_id = a.assertion_id)"
        ),
        0
    );

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

/// Deleting every file leaves the repository entity and nothing else — which is exactly what a
/// from-scratch index of an empty tree produces.
#[test]
fn deleting_everything_leaves_what_a_fresh_index_of_an_empty_tree_would() {
    let (_dir, root) = indexed_incremental_fixture();
    // `README.md` goes too: since Slice 5a it is an indexed document, and "every file" means
    // every file.
    for file in [
        "README.md",
        "src/app.ts",
        "src/barrel.ts",
        "src/impl.ts",
        "src/assist.ts",
        "src/island.ts",
        "src/movable.ts",
    ] {
        remove(&root, file);
    }
    let outcome = index(&root);
    assert_eq!(outcome.files_processed, 0);
    assert_eq!(outcome.incremental.files_removed, 7);

    let conn = open_db(&root);
    assert_eq!(count(&conn, "assertion"), 0);
    assert_eq!(count(&conn, "observation"), 0);
    assert_eq!(count(&conn, "occurrence"), 0);
    assert_eq!(count(&conn, "assertion_state"), 0);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM entity"),
        1,
        "only the repository entity should survive"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE kind = 'repository'"
        ),
        1
    );
    assert_eq!(
        count(&conn, "module_facts"),
        0,
        "the extraction cache must forget files that no longer exist"
    );

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

/// No surviving row may describe a superseded version of its file.
///
/// This replaces the Slice 3 test that asserted every row carried the current `state_id`. Since
/// ADR-0006 no graph row carries a state at all, so that property is not expressible — but the
/// thing it was protecting against, a database quietly describing two trees at once, is. The
/// check here is on `content_hash`, which is the freshness anchor the product actually uses, and
/// it is **stronger**: a restamped row could be at the current state and still describe bytes
/// that no longer exist, and this would catch that where the state check could not.
#[test]
fn no_row_is_left_describing_a_superseded_version_of_its_file() {
    let (_dir, root) = indexed_incremental_fixture();
    write(
        &root,
        "src/assist.ts",
        "export function assist(): number {\n  return 8;\n}\n",
    );
    index(&root);

    let conn = open_db(&root);
    let mut checked = 0usize;
    for table in ["occurrence", "observation"] {
        let rows: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT DISTINCT file_path, content_hash FROM {table} ORDER BY 1, 2"
                ))
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(|row| row.unwrap())
                .collect()
        };
        for (file_path, content_hash) in rows {
            let absolute = root.join(&file_path);
            // Directory containment rows quote a directory, whose "content" is its own path.
            if !absolute.is_file() {
                continue;
            }
            let current = nerve_core::ids::content_hash(&std::fs::read(&absolute).unwrap());
            assert_eq!(
                content_hash, current,
                "{table} row for {file_path} describes bytes that are no longer there"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the check inspected nothing");

    // And the state the database says it describes is the one this run observed.
    let dump = nerve_store::canonical_dump(&conn).unwrap();
    assert_eq!(dump.state_ids.len(), 1, "{:?}", dump.state_ids);
}

/// **Slice 3b, ADR-0003 purity.** The scoped derivation and the scoped pruner are lazy
/// evaluations of the whole-table statements, and lazy evaluation is only legitimate while the
/// answer is identical. This runs a mixed edit sequence and, after every step, re-runs both
/// whole-table versions and asserts they change nothing.
///
/// A regression in the scope — an assertion whose evidence moved but which was left out — shows
/// up here as a derived row that the rebuild disagrees with, or as an orphan the scoped pruner
/// failed to reach.
#[test]
fn scoped_derivation_and_pruning_equal_the_whole_table_versions() {
    const SEED: u64 = 0x5C09_ED03_B1DE_A115;
    const STEPS: usize = 16;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();

    let mut rng = Rng(SEED);
    let mut modules = starting_modules();
    materialize(&root, &modules);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    index(&root);

    let mut scoped_runs = 0usize;
    for step in 0..STEPS {
        let Some(edit) = plan_edit(&mut rng, &modules, step) else {
            continue;
        };
        let description = apply(edit, &mut modules, &mut rng);
        materialize(&root, &modules);
        let outcome = index(&root);
        if outcome.incremental.files_re_extracted < outcome.files_processed {
            scoped_runs += 1;
        }

        let conn = open_db(&root);
        let derived = assertion_states(&conn);
        nerve_store::rebuild_assertion_state(&conn).unwrap();
        assert_eq!(
            derived,
            assertion_states(&conn),
            "step {step} ({description}): scoped derivation != whole-table rebuild"
        );

        let leftovers = nerve_store::prune_orphans(&conn).unwrap();
        assert_eq!(
            leftovers,
            nerve_store::RemovalCounts::default(),
            "step {step} ({description}): the scoped pruner left orphans behind"
        );
    }

    assert!(
        scoped_runs >= 5,
        "the sequence never exercised the scoped path, so it proved nothing ({scoped_runs} runs)"
    );
}

/// Every derived row, rendered so two derivations can be compared as text.
fn assertion_states(conn: &nerve_store::Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT assertion_id || '|' || status || '|' || strongest_source_type || '|' ||
                    source_type_mask || '|' || observation_count || '|' || is_unresolved
               FROM assertion_state ORDER BY assertion_id",
        )
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

// ---- --full --------------------------------------------------------------------------------

#[test]
fn full_re_extracts_everything_and_lands_in_the_same_place() {
    let (_dir, root) = indexed_incremental_fixture();
    write(
        &root,
        "src/island.ts",
        "export function alone(): number {\n  return 3;\n}\n",
    );
    index(&root);
    let incremental = dump_json(&root);

    let outcome = index_full(&root);
    assert!(outcome.incremental.full);
    assert_eq!(
        outcome.incremental.files_re_extracted,
        outcome.files_processed
    );
    assert_eq!(outcome.incremental.files_skipped_unchanged, 0);
    assert_eq!(
        dump_json(&root),
        incremental,
        "--full disagreed with the incremental result it was checking"
    );

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

// ---- identity links ------------------------------------------------------------------------

fn identity_links(conn: &nerve_store::Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT link_kind, evidence FROM identity_link ORDER BY link_id")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

/// Acceptance criterion 7, positive half.
#[test]
fn a_moved_file_proposes_an_identity_link_with_evidence() {
    let (_dir, root) = indexed_incremental_fixture();
    let moved = std::fs::read_to_string(root.join("src/movable.ts")).unwrap();
    remove(&root, "src/movable.ts");
    write(&root, "src/relocated/movable.ts", &moved);

    let outcome = index(&root);
    assert_eq!(outcome.incremental.files_removed, 1);
    assert_eq!(outcome.incremental.files_added, 1);
    assert!(
        outcome.incremental.identity_links_proposed >= 3,
        "one file link and one per matched symbol, got {}",
        outcome.incremental.identity_links_proposed
    );

    let conn = open_db(&root);
    let links = identity_links(&conn);
    let kinds: BTreeSet<&str> = links.iter().map(|(kind, _)| kind.as_str()).collect();
    assert!(kinds.contains("moved_file"));
    assert!(kinds.contains("moved_symbol"));

    let file_link = links
        .iter()
        .find(|(kind, _)| kind == "moved_file")
        .expect("a moved file must be linked");
    let evidence: serde_json::Value = serde_json::from_str(&file_link.1).unwrap();
    assert_eq!(evidence["from_path"], "src/movable.ts");
    assert_eq!(evidence["to_path"], "src/relocated/movable.ts");
    assert_eq!(evidence["matched_symbols"], 2);
    assert_eq!(evidence["file_content_hash_equal"], true);

    let symbol_link = links
        .iter()
        .find(|(kind, _)| kind == "moved_symbol")
        .unwrap();
    let evidence: serde_json::Value = serde_json::from_str(&symbol_link.1).unwrap();
    assert!(evidence["body_hash"]
        .as_str()
        .is_some_and(|h| h.len() == 64));
    assert!(evidence["name"].as_str().is_some());

    // A proposal is not a merge: both identities exist independently, and the old one is gone
    // from the graph because its file is gone.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM occurrence WHERE file_path = 'src/movable.ts'"
        ),
        0
    );

    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(
        dump_json(&root),
        reference,
        "identity links must not change what the graph claims about the tree"
    );

    // Re-proposing the same move must not duplicate the proposal.
    let before = count(&conn, "identity_link");
    drop(conn);
    index_full(&root);
    let conn = open_db(&root);
    assert_eq!(count(&conn, "identity_link"), before);
}

/// Acceptance criterion 7, negative half — the fixture that must stay failing if the rule is
/// ever loosened to a name match.
#[test]
fn a_coincidental_name_match_across_unrelated_files_proposes_no_link() {
    let (_dir, root) = indexed_incremental_fixture();
    let impostor = std::fs::read_to_string(
        named_fixture_root("ts-incremental").join("candidates/impostor.ts"),
    )
    .unwrap();

    remove(&root, "src/movable.ts");
    write(&root, "src/impostor.ts", &impostor);

    let outcome = index(&root);
    assert_eq!(outcome.incremental.files_removed, 1);
    assert_eq!(outcome.incremental.files_added, 1);
    assert_eq!(
        outcome.incremental.identity_links_proposed, 0,
        "`relocate` and `annotate` match by name only; that is not evidence of identity"
    );

    let conn = open_db(&root);
    assert_eq!(count(&conn, "identity_link"), 0);
}

// ---- the equivalence property --------------------------------------------------------------

/// A deterministic 64-bit PRNG, so a failure reproduces exactly from the seed printed with it.
///
/// Hand-rolled rather than pulled in as a dependency: the product ships no random number
/// generator, and adding one to the tree for a test would be a real dependency for a fake need.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// One import statement: the module named in the specifier, and the module that actually
/// declares the symbol.
///
/// They differ whenever the import goes **through a barrel**, and that difference is the whole
/// point. If every import named the module that declares the symbol, no resolution would ever
/// cross a re-export, and a one-hop invalidation set would be indistinguishable from a correct
/// one. A generator that cannot tell those apart proves nothing about the closure.
#[derive(Debug, Clone)]
struct SyntheticImport {
    /// Module named in the specifier.
    via: String,
    /// Module that declares the imported symbol.
    symbol_from: String,
}

/// A module in the synthetic repository the property test mutates.
#[derive(Debug, Clone)]
struct SyntheticModule {
    rel_path: String,
    /// Symbols this module imports and calls, possibly through a barrel.
    imports: Vec<SyntheticImport>,
    /// Paths this module re-exports with `export *`.
    re_exports: Vec<String>,
    salt: u64,
    /// When true the module exports `head_<id>_alt` instead of `head_<id>`.
    ///
    /// Importers always name `head_<id>`, so toggling this is a **resolution-visible** edit: it
    /// changes what every transitive importer resolves, while leaving their bytes untouched. An
    /// edit that only changes a literal inside a function body does not — entity identity
    /// excludes body content by design — so a generator without this would never notice an
    /// invalidation set that stops one hop short.
    renamed: bool,
    /// Extra declarations, so a module can be sized like a real source file rather than a stub.
    bulk: usize,
}

fn stem(rel_path: &str) -> &str {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    name.strip_suffix(".ts").unwrap_or(name)
}

fn identifier(rel_path: &str) -> String {
    rel_path
        .trim_end_matches(".ts")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A relative specifier from `from` to `to`, both repository-relative.
fn specifier(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = from.split('/').collect();
    let to_parts: Vec<&str> = to.split('/').collect();
    let from_dir = &from_dir[..from_dir.len() - 1];
    let to_dir = &to_parts[..to_parts.len() - 1];

    let shared = from_dir
        .iter()
        .zip(to_dir.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut segments: Vec<String> = Vec::new();
    for _ in shared..from_dir.len() {
        segments.push("..".to_string());
    }
    for part in &to_dir[shared..] {
        segments.push((*part).to_string());
    }
    segments.push(stem(to).to_string());
    let joined = segments.join("/");
    if joined.starts_with("..") {
        joined
    } else {
        format!("./{joined}")
    }
}

impl SyntheticModule {
    fn exported_name(&self) -> String {
        let me = identifier(&self.rel_path);
        if self.renamed {
            format!("head_{me}_alt")
        } else {
            format!("head_{me}")
        }
    }

    fn render(&self) -> String {
        let me = identifier(&self.rel_path);
        let head = self.exported_name();
        let mut source = String::new();
        for import in &self.imports {
            source.push_str(&format!(
                "import {{ head_{} }} from '{}';\n",
                identifier(&import.symbol_from),
                specifier(&self.rel_path, &import.via)
            ));
        }
        for re_export in &self.re_exports {
            source.push_str(&format!(
                "export * from '{}';\n",
                specifier(&self.rel_path, re_export)
            ));
        }
        source.push_str(&format!("\nexport function {head}(): number {{\n"));
        source.push_str(&format!("  let total = {};\n", self.salt));
        for import in &self.imports {
            source.push_str(&format!(
                "  total += head_{}();\n",
                identifier(&import.symbol_from)
            ));
        }
        source.push_str("  return total;\n}\n\n");
        source.push_str(&format!(
            "export class Shape_{me} {{\n  area(): number {{\n    return {head}() * {};\n  }}\n}}\n",
            self.salt % 7 + 1
        ));
        for extra in 0..self.bulk {
            source.push_str(&format!(
                "\nexport function detail_{me}_{extra}(input: number): number {{\n  \
                 const scaled = input * {extra} + {};\n  \
                 const clamped = scaled > 0 ? scaled : -scaled;\n  \
                 return clamped + {head}();\n}}\n\n\
                 interface Shaped_{me}_{extra} {{\n  width: number;\n  height: number;\n}}\n\n\
                 class Helper_{me}_{extra} {{\n  \
                 private state: number = {extra};\n  \
                 scale(by: number): number {{\n    \
                 return detail_{me}_{extra}(this.state) * by;\n  }}\n  \
                 reset(): void {{\n    this.state = {extra};\n  }}\n}}\n",
                self.salt
            ));
        }
        source
    }
}

fn materialize(root: &Path, modules: &[SyntheticModule]) {
    let wanted: BTreeSet<String> = modules.iter().map(|m| m.rel_path.clone()).collect();
    let mut existing: Vec<PathBuf> = Vec::new();
    collect_typescript(&root.join("src"), &mut existing);
    for path in existing {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !wanted.contains(&rel) {
            std::fs::remove_file(root.join(&rel)).unwrap();
        }
    }
    for module in modules {
        write(root, &module.rel_path, &module.render());
    }
}

fn collect_typescript(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_by_extension(dir, "ts", out);
}

fn collect_by_extension(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.map(|e| e.unwrap()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_by_extension(&path, extension, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

// ---- documents in the property tree --------------------------------------------------------

/// A Markdown document the property test mutates alongside the modules.
///
/// Documents matter to the equivalence property for a reason the modules cannot cover: a
/// section's identity is a function of its **heading text and its position among its siblings**,
/// not of a path. Renaming a heading therefore changes an entity id in a way no source edit
/// does, and a stale section that an incremental run failed to delete shows up as a dump that
/// a from-scratch index would never produce.
#[derive(Debug, Clone)]
struct SyntheticDoc {
    rel_path: String,
    /// Top-level heading texts, each with one child.
    headings: Vec<String>,
    /// Changes the body without changing any heading, so no identity moves.
    salt: u64,
    /// When true the first heading is spelled `<text> (revised)`.
    renamed: bool,
}

impl SyntheticDoc {
    fn render(&self) -> String {
        let mut text = format!("---\ntitle: {}\n---\n\n", self.rel_path);
        for (position, heading) in self.headings.iter().enumerate() {
            let shown = if position == 0 && self.renamed {
                format!("{heading} (revised)")
            } else {
                heading.clone()
            };
            text.push_str(&format!("# {shown}\n\nBody {} of {shown}.\n\n", self.salt));
            text.push_str(&format!(
                "## Detail\n\n```ts\n# not a heading {}\n```\n\n",
                self.salt
            ));
            text.push_str("### Deeper\n\nText.\n\n");
        }
        text
    }
}

fn starting_docs() -> Vec<SyntheticDoc> {
    vec![
        SyntheticDoc {
            rel_path: "docs/overview.md".to_string(),
            headings: vec!["Overview".to_string(), "Overview".to_string()],
            salt: 1,
            renamed: false,
        },
        SyntheticDoc {
            rel_path: "docs/decisions/ADR-0001-first.md".to_string(),
            headings: vec!["ADR-0001".to_string()],
            salt: 2,
            renamed: false,
        },
    ]
}

fn materialize_docs(root: &Path, docs: &[SyntheticDoc]) {
    let wanted: BTreeSet<String> = docs.iter().map(|doc| doc.rel_path.clone()).collect();
    let mut existing: Vec<PathBuf> = Vec::new();
    collect_by_extension(&root.join("docs"), "md", &mut existing);
    for path in existing {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !wanted.contains(&rel) {
            std::fs::remove_file(root.join(&rel)).unwrap();
        }
    }
    for doc in docs {
        write(root, &doc.rel_path, &doc.render());
    }
}

/// Apply one document edit and name it. Draws from its own generator so that the code edit
/// sequence — and every assertion already made about it — is bit-for-bit unchanged.
fn apply_doc_edit(rng: &mut Rng, docs: &mut Vec<SyntheticDoc>, step: usize) -> &'static str {
    match rng.below(5) {
        0 => {
            let victim = rng.below(docs.len());
            docs[victim].salt = rng.next() % 1000;
            "doc-modify"
        }
        1 => {
            let victim = rng.below(docs.len());
            docs[victim].renamed = !docs[victim].renamed;
            "doc-rename-heading"
        }
        2 => {
            docs.push(SyntheticDoc {
                rel_path: format!("docs/added_{step}.md"),
                headings: vec![format!("Added {step}"), "Shared".to_string()],
                salt: rng.next() % 1000,
                renamed: false,
            });
            "doc-add"
        }
        3 if docs.len() > 2 => {
            let victim = rng.below(docs.len());
            docs.remove(victim);
            "doc-delete"
        }
        _ if docs.len() > 1 => {
            let victim = rng.below(docs.len());
            let name = docs[victim]
                .rel_path
                .rsplit('/')
                .next()
                .unwrap()
                .to_string();
            let destination = format!("docs/moved/{name}");
            if docs.iter().any(|doc| doc.rel_path == destination) {
                return "doc-move-skipped";
            }
            docs[victim].rel_path = destination;
            "doc-move"
        }
        _ => "doc-move-skipped",
    }
}

fn module(
    rel_path: &str,
    imports: &[(&str, &str)],
    re_exports: &[&str],
    salt: u64,
) -> SyntheticModule {
    SyntheticModule {
        rel_path: rel_path.into(),
        imports: imports
            .iter()
            .map(|(via, symbol_from)| SyntheticImport {
                via: (*via).to_string(),
                symbol_from: (*symbol_from).to_string(),
            })
            .collect(),
        re_exports: re_exports.iter().map(|p| (*p).to_string()).collect(),
        salt,
        renamed: false,
        bulk: 0,
    }
}

/// The starting tree: two leaves behind a barrel, behind a second barrel, behind an app.
///
/// `app.ts` names `./mid` in its specifier but resolves a symbol declared in `leaf_a.ts`, three
/// re-export hops away. A closure that stops short by even one hop leaves `app.ts` resolving a
/// symbol that no longer exists.
fn starting_modules() -> Vec<SyntheticModule> {
    vec![
        module("src/leaf_a.ts", &[], &[], 1),
        module("src/leaf_b.ts", &[], &[], 2),
        module("src/barrel.ts", &[], &["src/leaf_a.ts", "src/leaf_b.ts"], 3),
        module(
            "src/mid.ts",
            &[("src/barrel.ts", "src/leaf_a.ts")],
            &["src/barrel.ts"],
            4,
        ),
        module(
            "src/app.ts",
            &[
                ("src/mid.ts", "src/leaf_a.ts"),
                ("src/barrel.ts", "src/leaf_b.ts"),
            ],
            &[],
            5,
        ),
        module(
            "src/nested/util.ts",
            &[("src/leaf_b.ts", "src/leaf_b.ts")],
            &[],
            6,
        ),
    ]
}

#[derive(Debug)]
enum Edit {
    /// Change a function body. Identity is unaffected, so nothing downstream re-resolves.
    Modify(String),
    /// Rename an exported symbol. Every transitive importer's resolution changes.
    RenameExport(String),
    Add(String),
    Delete(String),
    Move(String, String),
    AddDependency(String, String),
}

fn plan_edit(rng: &mut Rng, modules: &[SyntheticModule], step: usize) -> Option<Edit> {
    let choice = rng.below(12);
    let victim = modules[rng.below(modules.len())].rel_path.clone();
    match choice {
        0..=2 => Some(Edit::Modify(victim)),
        3..=5 => Some(Edit::RenameExport(victim)),
        6..=7 => {
            let directory = if rng.below(2) == 0 {
                "src"
            } else {
                "src/nested"
            };
            Some(Edit::Add(format!("{directory}/added_{step}.ts")))
        }
        8 if modules.len() > 3 => Some(Edit::Delete(victim)),
        9 if modules.len() > 3 => {
            let destination = format!("src/moved/{}", stem(&victim));
            Some(Edit::Move(victim, format!("{destination}.ts")))
        }
        _ => {
            let target = modules[rng.below(modules.len())].rel_path.clone();
            if target == victim {
                None
            } else {
                Some(Edit::AddDependency(victim, target))
            }
        }
    }
}

fn apply(edit: Edit, modules: &mut Vec<SyntheticModule>, rng: &mut Rng) -> String {
    match edit {
        Edit::Modify(path) => {
            if let Some(module) = modules.iter_mut().find(|m| m.rel_path == path) {
                module.salt = rng.next() % 1000;
            }
            format!("modify {path}")
        }
        Edit::RenameExport(path) => {
            if let Some(module) = modules.iter_mut().find(|m| m.rel_path == path) {
                module.renamed = !module.renamed;
            }
            format!("rename-export {path}")
        }
        Edit::Add(path) => {
            let dependency = &modules[rng.below(modules.len())];
            let import = import_through(dependency, rng);
            modules.push(SyntheticModule {
                rel_path: path.clone(),
                imports: vec![import],
                re_exports: vec![],
                salt: rng.next() % 1000,
                renamed: false,
                bulk: 0,
            });
            format!("add {path}")
        }
        Edit::Delete(path) => {
            modules.retain(|m| m.rel_path != path);
            format!("delete {path}")
        }
        Edit::Move(from, to) => {
            if let Some(module) = modules.iter_mut().find(|m| m.rel_path == from) {
                module.rel_path = to.clone();
            }
            format!("move {from} -> {to}")
        }
        Edit::AddDependency(from, to) => {
            let import = match modules.iter().find(|m| m.rel_path == to) {
                Some(target) => import_through(target, rng),
                None => return format!("depend {from} -> {to} (absent)"),
            };
            if let Some(module) = modules.iter_mut().find(|m| m.rel_path == from) {
                let already = module
                    .imports
                    .iter()
                    .any(|existing| existing.symbol_from == import.symbol_from);
                if !already && !module.re_exports.contains(&to) {
                    module.imports.push(import);
                }
            }
            format!("depend {from} -> {to}")
        }
    }
}

/// Import from `target`, naming a symbol `target` re-exports rather than declares when it can.
fn import_through(target: &SyntheticModule, rng: &mut Rng) -> SyntheticImport {
    let symbol_from = if target.re_exports.is_empty() || rng.below(3) == 0 {
        target.rel_path.clone()
    } else {
        target.re_exports[rng.below(target.re_exports.len())].clone()
    };
    SyntheticImport {
        via: target.rel_path.clone(),
        symbol_from,
    }
}

/// **Acceptance criterion 2**, and Slice 5a's acceptance criterion 7. For a seeded sequence of
/// mixed edits over both source and documents, an incremental re-index must produce a canonical
/// dump byte-identical to a from-scratch index of the same tree, at *every* step.
///
/// The seed is fixed so a failure reproduces exactly. Divergence is reported with the step, the
/// edit that caused it, and the first differing line — an equality assertion on two 100 kB JSON
/// documents is otherwise unreadable.
///
/// Documents are driven by a **separate generator**, so the source edit sequence and every
/// assertion already made about it are bit-for-bit what they were before Slice 5a. One document
/// edit is applied per step, which makes the property strictly harder to satisfy rather than
/// differently satisfied: every step now mixes a source change with a document change.
#[test]
fn incremental_and_full_agree_under_a_seeded_edit_sequence() {
    const SEED: u64 = 0x5117_3E03_1CE7_5EED;
    const DOC_SEED: u64 = 0x00D0_C5EE_D0C5_EED1;
    const STEPS: usize = 24;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();

    let mut rng = Rng(SEED);
    let mut doc_rng = Rng(DOC_SEED);
    let mut modules = starting_modules();
    let mut docs = starting_docs();
    materialize(&root, &modules);
    materialize_docs(&root, &docs);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    index(&root);

    let mut applied = Vec::new();
    let mut amplifications = Vec::new();
    let mut kinds: BTreeSet<&'static str> = BTreeSet::new();

    for step in 0..STEPS {
        let Some(edit) = plan_edit(&mut rng, &modules, step) else {
            continue;
        };
        let description = apply(edit, &mut modules, &mut rng);
        kinds.insert(match description.split(' ').next().unwrap() {
            "modify" => "modify",
            "rename-export" => "rename-export",
            "add" => "add",
            "delete" => "delete",
            "move" => "move",
            _ => "depend",
        });
        kinds.insert(apply_doc_edit(&mut doc_rng, &mut docs, step));
        applied.push(description.clone());
        materialize(&root, &modules);
        materialize_docs(&root, &docs);

        let outcome = index(&root);
        if let Some(factor) = outcome.incremental.amplification() {
            amplifications.push(factor);
        }

        let actual = dump_json(&root);
        let (_reference_dir, expected) = dump_of_a_from_scratch_index(&root);
        if actual != expected {
            let divergence = actual
                .lines()
                .zip(expected.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(line, (a, b))| {
                    format!("line {line}:\n  incremental: {a}\n  full:        {b}")
                })
                .unwrap_or_else(|| {
                    format!(
                        "lengths differ: incremental {} lines, full {} lines",
                        actual.lines().count(),
                        expected.lines().count()
                    )
                });
            panic!(
                "incremental and full disagree at step {step} (seed {SEED:#x})\n\
                 edit: {description}\n\
                 history: {applied:?}\n{divergence}"
            );
        }
    }

    // ---- the document-link sequence ---------------------------------------------------------
    //
    // Slice 5c gives a document edges that rest on files it does not contain: the indexed path
    // set, and — under a `#L<n>` anchor — the target file's bytes. There are exactly five ways
    // that dependency can move, and they are scripted rather than sampled, because a sampler
    // that happened not to draw one of them would report a property it never tested.
    //
    // The target carries a line anchor throughout, which is the strictly harder case: an
    // unanchored link survives an edit to its target by design, an anchored one must not.
    const TARGET: &str = "src/link_target.ts";
    const MOVED_TARGET: &str = "src/moved/link_target.ts";
    const DOCUMENT: &str = "docs/links.md";
    let target_source = |body: &str| format!("export function linked(): number {{\n  {body}\n}}\n");
    let document_source = |links: &str| {
        format!("---\ntitle: links\n---\n\n# Links\n\n## Body\n\n{links}\n\n### Deeper\n\nText.\n")
    };

    // A document with no links, so that "link added" is an edit to an existing document rather
    // than the arrival of a new one.
    write(&root, TARGET, &target_source("return 1;"));
    write(&root, DOCUMENT, &document_source("No links yet."));
    index(&root);

    let mut link_steps: Vec<&'static str> = Vec::new();
    let check = |label: &'static str, applied: &[String], steps: &mut Vec<&'static str>| {
        let outcome = index(&root);
        let actual = dump_json(&root);
        let (_reference_dir, expected) = dump_of_a_from_scratch_index(&root);
        assert_eq!(
            actual, expected,
            "incremental and full disagree after {label}\n\
             re-extracted {} file(s), resolution-changed {}\n\
             history: {applied:?}",
            outcome.incremental.files_re_extracted, outcome.incremental.files_resolution_changed
        );
        steps.push(label);
    };

    // 1. A link is added to a document whose bytes are the only thing that changed.
    write(
        &root,
        DOCUMENT,
        &document_source("A link to [the target](../src/link_target.ts#L2)."),
    );
    check("doc-link-add", &applied, &mut link_steps);

    // 2. The target is edited. The document's bytes are unchanged, and its anchor edge records
    //    the target's content hash, so it must be re-extracted anyway.
    write(&root, TARGET, &target_source("return 2;"));
    check("doc-link-target-edited", &applied, &mut link_steps);

    // 3. The target is deleted. The link becomes unresolved rather than disappearing.
    remove(&root, TARGET);
    check("doc-link-target-deleted", &applied, &mut link_steps);

    // 4. The target comes back. The unresolved entity must go, and the edge must resolve again.
    write(&root, TARGET, &target_source("return 2;"));
    check("doc-link-target-restored", &applied, &mut link_steps);

    // 5. The target moves. Nothing in the document changed and its link is now broken, which is
    //    precisely the stale-documentation signal this slice exists to produce.
    remove(&root, TARGET);
    write(&root, MOVED_TARGET, &target_source("return 2;"));
    check("doc-link-target-moved", &applied, &mut link_steps);

    // The graph must actually show the break, not merely agree with a from-scratch index that is
    // equally wrong.
    {
        let conn = open_db(&root);
        assert_eq!(
            scalar(
                &conn,
                "SELECT count(*) FROM entity e
                   JOIN occurrence o ON o.entity_id = e.entity_id
                  WHERE e.kind = 'unresolved' AND o.file_path = 'docs/links.md'"
            ),
            1,
            "the moved target left no broken-link entity behind"
        );
    }

    assert_eq!(
        link_steps,
        vec![
            "doc-link-add",
            "doc-link-target-edited",
            "doc-link-target-deleted",
            "doc-link-target-restored",
            "doc-link-target-moved",
        ]
    );

    // ---- the supersession sequence ----------------------------------------------------------
    //
    // Slice 5d-ii gives a document one dependency a link does not: a **bare** `ADR-<digits>`
    // target resolves against the identifiers parsed from every indexed document's file name, so
    // adding or deleting an unrelated document can move the answer — including into and out of
    // ambiguity — while the citing document's bytes never change once. Scripted rather than
    // sampled, for the same reason the link sequence is: a sampler that happened not to draw the
    // ambiguity transition would report a property it never tested.
    const SUPERSEDER: &str = "docs/decisions/ADR-0900-head.md";
    const SUPERSEDED: &str = "docs/decisions/ADR-0901-target.md";
    const DUPLICATE: &str = "notes/ADR-0901-duplicate.md";
    let adr =
        |title: &str, field: &str| format!("# {title}\n\n**Status:** Accepted{field}\n\nB.\n");

    // Both documents, and no field yet, so that "field added" is an edit to an existing document
    // rather than the arrival of a new one.
    write(&root, SUPERSEDER, &adr("ADR-0900", ""));
    write(&root, SUPERSEDED, &adr("ADR-0901", ""));
    index(&root);

    let mut supersession_steps: Vec<&'static str> = Vec::new();
    let check_supersession = |label: &'static str,
                              steps: &mut Vec<&'static str>|
     -> nerve_index::IndexOutcome {
        let outcome = index(&root);
        let actual = dump_json(&root);
        let (_reference_dir, expected) = dump_of_a_from_scratch_index(&root);
        assert_eq!(
            actual, expected,
            "incremental and full disagree after {label}\n\
                 re-extracted {} file(s), resolution-changed {}",
            outcome.incremental.files_re_extracted, outcome.incremental.files_resolution_changed
        );
        steps.push(label);
        outcome
    };

    // 1. The field is added. One document changed, and it resolves.
    write(
        &root,
        SUPERSEDER,
        &adr("ADR-0900", " · **Supersedes:** ADR-0901"),
    );
    let outcome = check_supersession("supersedes-add", &mut supersession_steps);
    assert_eq!(outcome.supersession_edges, 1);

    // 2. The target is deleted. `ADR-0900-head.md` is byte-identical to the run before, so only
    //    the identifier re-resolution can seed it — and if it is not seeded, the graph keeps an
    //    edge into a document that no longer exists.
    remove(&root, SUPERSEDED);
    let outcome = check_supersession("supersedes-target-deleted", &mut supersession_steps);
    assert_eq!(outcome.supersession_edges, 0);
    assert!(
        outcome.incremental.files_resolution_changed >= 1,
        "the citing document was not seeded by identifier re-resolution"
    );

    // 3. The target comes back. The unresolved entity must go and the edge must resolve again.
    write(&root, SUPERSEDED, &adr("ADR-0901", ""));
    let outcome = check_supersession("supersedes-target-restored", &mut supersession_steps);
    assert_eq!(outcome.supersession_edges, 1);

    // 4. A second document carries the same identifier. The target is now ambiguous, and Nerve
    //    must withdraw the edge rather than keep the one it already had.
    write(&root, DUPLICATE, &adr("ADR-0901", ""));
    let outcome = check_supersession("supersedes-target-ambiguous", &mut supersession_steps);
    assert_eq!(
        outcome.supersession_edges, 0,
        "an identifier two documents carry must not keep resolving to the first of them"
    );
    assert!(outcome.incremental.files_resolution_changed >= 1);

    // 5. The duplicate goes away. Ambiguity is not a sticky state.
    remove(&root, DUPLICATE);
    let outcome = check_supersession("supersedes-target-unambiguous", &mut supersession_steps);
    assert_eq!(outcome.supersession_edges, 1);

    assert_eq!(
        supersession_steps,
        vec![
            "supersedes-add",
            "supersedes-target-deleted",
            "supersedes-target-restored",
            "supersedes-target-ambiguous",
            "supersedes-target-unambiguous",
        ]
    );

    // ---- the Python sequence (Slice 9a, acceptance criterion 6) ------------------------------
    //
    // Extending this harness rather than starting a second one is the point: the property that
    // matters is *"an incremental re-index equals a from-scratch index of the same tree"*, and it
    // is one property over one database, not one per language. The Python tree is written into
    // the same repository the source and document sequences already built, so every step below
    // re-indexes a mixed tree.
    //
    // Scripted rather than sampled, for the reason the link sequence is: Python's dependency on
    // the file set has a shape TypeScript's does not. `pkg/util.py` and `pkg/util/__init__.py`
    // are two different answers to one specifier, and a package outranks a module — so creating
    // an `__init__.py` moves an already-resolved import without touching the importer's bytes.
    const PY_PKG_INIT: &str = "pysrc/__init__.py";
    const PY_UTIL: &str = "pysrc/util.py";
    const PY_APP: &str = "pysrc/app.py";
    const PY_UTIL_PACKAGE: &str = "pysrc/util/__init__.py";

    write(&root, PY_PKG_INIT, "\"\"\"pysrc.\"\"\"\n");
    write(&root, PY_UTIL, "def scale(value):\n    return value * 2\n");
    write(
        &root,
        PY_APP,
        "from .util import scale\nimport pysrc.absent\n\n\ndef run(value):\n    return scale(value)\n",
    );
    index(&root);

    let mut python_steps: Vec<&'static str> = Vec::new();
    let check_python = |label: &'static str, steps: &mut Vec<&'static str>| {
        let outcome = index(&root);
        let actual = dump_json(&root);
        let (_reference_dir, expected) = dump_of_a_from_scratch_index(&root);
        assert_eq!(
            actual, expected,
            "incremental and full disagree after {label}\n\
             re-extracted {} file(s), resolution-changed {}",
            outcome.incremental.files_re_extracted, outcome.incremental.files_resolution_changed
        );
        steps.push(label);
    };

    // 1. The imported module is edited. Only its own bytes moved.
    write(&root, PY_UTIL, "def scale(value):\n    return value * 3\n");
    check_python("py-target-edited", &mut python_steps);

    // 2. A file appears that satisfies a specifier which was resolving to nothing. `pysrc/app.py`
    //    is byte-identical to the run before and its only edge to `pysrc.absent` points at an
    //    `Unresolved` entity, so the graph walk cannot reach it — specifier re-resolution must.
    write(&root, "pysrc/absent.py", "MARKER = 1\n");
    check_python("py-dangling-import-satisfied", &mut python_steps);

    // 3. A package appears that outranks the module a specifier already resolved to. Nothing in
    //    the importer changed and its answer must move all the same.
    write(
        &root,
        PY_UTIL_PACKAGE,
        "\"\"\"pysrc.util, now a package.\"\"\"\n",
    );
    check_python("py-package-outranks-module", &mut python_steps);

    // 4. The package goes away. Outranking is not a sticky state.
    remove(&root, PY_UTIL_PACKAGE);
    check_python("py-package-removed", &mut python_steps);

    // 5. The imported module is deleted. The import becomes unresolved rather than vanishing.
    remove(&root, PY_UTIL);
    check_python("py-target-deleted", &mut python_steps);

    // 6. A Python module moves. Its symbols keep their shape and their path does not.
    write(
        &root,
        "pysrc/moved.py",
        "def scale(value):\n    return value * 3\n",
    );
    check_python("py-target-moved", &mut python_steps);

    assert_eq!(
        python_steps,
        vec![
            "py-target-edited",
            "py-dangling-import-satisfied",
            "py-package-outranks-module",
            "py-package-removed",
            "py-target-deleted",
            "py-target-moved",
        ]
    );

    // The Python half of the tree must actually be in the graph, or every equality above holds
    // between two empty answers.
    {
        let conn = open_db(&root);
        assert!(
            scalar(
                &conn,
                "SELECT count(*) FROM observation WHERE extractor_id = 'py-structural'"
            ) > 0,
            "py-structural contributed nothing, so the equivalence above proves nothing about it"
        );
    }

    assert!(
        applied.len() >= 20,
        "the property must be exercised over at least 20 edits, applied {}",
        applied.len()
    );
    // A sequence that never deleted or moved anything would prove much less than it appears to.
    // The document half must cover the same ground, plus the heading rename, which is the only
    // edit in either half that moves an entity id without moving a path.
    for required in [
        "modify",
        "rename-export",
        "add",
        "delete",
        "move",
        "doc-modify",
        "doc-rename-heading",
        "doc-add",
        "doc-delete",
        "doc-move",
    ] {
        assert!(
            kinds.contains(required),
            "the seeded sequence never performed a {required}; it exercised {kinds:?}"
        );
    }
    let mean = amplifications.iter().sum::<f64>() / amplifications.len().max(1) as f64;
    println!(
        "equivalence held over {} edits (seed {SEED:#x}); kinds {kinds:?}; \
         mean amplification {mean:.2}\nhistory: {applied:?}",
        applied.len()
    );
}

/// The same property with `--full` in the loop: a forced full run over a database that already
/// holds a previous tree must also equal a from-scratch index. Without deletion and restatement
/// it would not, because `--full` inherits whatever the last run left behind.
#[test]
fn a_forced_full_run_over_an_existing_database_equals_a_from_scratch_index() {
    let (_dir, root) = indexed_incremental_fixture();
    remove(&root, "src/assist.ts");
    write(
        &root,
        "src/impl.ts",
        "export function helper(): number {\n  return 99;\n}\n",
    );
    write(
        &root,
        "src/extra.ts",
        "import { helper } from './impl';\nexport function extra(): number {\n  return helper();\n}\n",
    );

    index_full(&root);
    let (_reference_dir, reference) = dump_of_a_from_scratch_index(&root);
    assert_eq!(dump_json(&root), reference);
}

// ---- schema upgrade on the write path ------------------------------------------------------

/// `nerve index` must bring the schema up to date before it writes anything.
///
/// Only `nerve init` used to migrate, so a database written by an older build and then indexed
/// by a newer one kept its old shape. Until schema v3 that failed loudly, on a missing table.
/// With v3 it failed **silently and destructively**: `persist_batch` inserts `OR IGNORE`, which
/// swallows a `NOT NULL` violation exactly as readily as a duplicate key, so writing the v3
/// column list into a v2 `occurrence` discarded every insert *after* the re-extracted files' rows
/// had been deleted — a smaller graph and a zero exit code.
///
/// The database here is emptied rather than downgraded, because a v2 schema cannot be
/// reconstructed from a v3 build. It exercises the same line: a database that is not at the
/// current version when `index_repository` opens it.
#[test]
fn indexing_migrates_a_database_that_is_not_at_the_current_schema_version() {
    let (_dir, root) = indexed_incremental_fixture();
    let expected = dump_json(&root);

    // An unmigrated database: the file exists, so the repository is still "initialized", but it
    // has no schema at all.
    let db_path = nerve_index::config::db_path(&root);
    std::fs::remove_file(&db_path).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db_path.display()));
    }
    {
        let conn = nerve_store::open(&db_path).unwrap();
        assert_eq!(nerve_store::schema_version(&conn).unwrap(), None);
    }

    index_full(&root);

    let conn = open_db(&root);
    assert_eq!(
        nerve_store::schema_version(&conn).unwrap(),
        Some(nerve_store::SCHEMA_VERSION),
        "indexing left the database at an older schema version"
    );
    assert!(
        nerve_store::status(&conn).unwrap().is_healthy(),
        "indexing left the database unhealthy"
    );
    drop(conn);
    assert_eq!(
        dump_json(&root),
        expected,
        "indexing an unmigrated database produced a different graph"
    );
}

// ---- work proportional to the change -------------------------------------------------------

const CLUSTERS: usize = 52;
const PER_CLUSTER: usize = 10;
const REPEATS: usize = 3;

/// `clusters` short import chains of [`PER_CLUSTER`] modules each.
///
/// Realistic locality: editing the deepest module of one cluster invalidates that cluster and
/// nothing else, so the invalidation set is bounded by chain depth rather than repository size.
fn cluster_modules(clusters: usize, bulk: usize) -> Vec<SyntheticModule> {
    let mut modules = Vec::new();
    for cluster in 0..clusters {
        for depth in 0..PER_CLUSTER {
            let rel_path = format!("src/c{cluster}/m{depth}.ts");
            let imports = if depth == 0 {
                vec![]
            } else {
                let previous = format!("src/c{cluster}/m{}.ts", depth - 1);
                vec![SyntheticImport {
                    via: previous.clone(),
                    symbol_from: previous,
                }]
            };
            modules.push(SyntheticModule {
                rel_path,
                imports,
                re_exports: vec![],
                salt: (cluster * PER_CLUSTER + depth) as u64,
                renamed: false,
                bulk,
            });
        }
    }
    modules
}

/// What one leaf edit cost a repository of `clusters * PER_CLUSTER` files.
struct EditCost {
    files: usize,
    rows_written: usize,
    re_extracted: usize,
    full_rows_written: usize,
}

/// Build a clustered repository, index it, edit one leaf, and re-index.
fn cost_of_one_leaf_edit(clusters: usize) -> EditCost {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();

    let mut modules = cluster_modules(clusters, 0);
    materialize(&root, &modules);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let full = index(&root);

    let victim = modules
        .iter_mut()
        .find(|m| m.rel_path == format!("src/c0/m{}.ts", PER_CLUSTER - 1))
        .unwrap();
    victim.salt = 999_999;
    let rendered = victim.render();
    let victim_path = victim.rel_path.clone();
    write(&root, &victim_path, &rendered);

    let incremental = index(&root);
    EditCost {
        files: full.files_processed,
        rows_written: incremental.incremental.rows_written,
        re_extracted: incremental.incremental.files_re_extracted,
        full_rows_written: full.incremental.rows_written,
    }
}

/// **The durable Slice 3b gate.** A one-file leaf edit must write a number of database rows
/// that does not depend on how large the repository is.
///
/// Asserted by counting rows written, not by timing: a ratio is machine- and load-dependent and
/// can be flattered by a fast machine, whereas this is a structural property of the write path.
/// If a whole-repository pass ever returns — a state restamp, an unconditional
/// `DELETE FROM assertion_state`, an unscoped re-derivation of directory containment — the two
/// numbers separate and this test says so immediately.
///
/// The comparison is between a 100-file and a 520-file repository with **identical local
/// structure** around the edited file, so the expected answer is not "similar", it is *equal*.
#[test]
fn a_one_file_leaf_edit_writes_the_same_rows_whatever_the_repository_size() {
    let small = cost_of_one_leaf_edit(10);
    let large = cost_of_one_leaf_edit(CLUSTERS);

    println!(
        "one-file leaf edit: {} files wrote {} rows (full index wrote {}); \
         {} files wrote {} rows (full index wrote {})",
        small.files,
        small.rows_written,
        small.full_rows_written,
        large.files,
        large.rows_written,
        large.full_rows_written
    );

    assert_eq!(small.files, 10 * PER_CLUSTER);
    assert_eq!(large.files, CLUSTERS * PER_CLUSTER);
    assert_eq!(
        small.re_extracted, 1,
        "a leaf edit must re-extract the leaf"
    );
    assert_eq!(large.re_extracted, 1);

    assert_eq!(
        small.rows_written, large.rows_written,
        "a one-file edit wrote {} rows in a {}-file repository and {} rows in a {}-file one; \
         the write path is proportional to repository size, not to the change",
        small.rows_written, small.files, large.rows_written, large.files
    );

    // And the edit is genuinely cheap in absolute terms, not merely size-independent: a full
    // index of the same repository writes orders of magnitude more.
    assert!(
        large.full_rows_written > large.rows_written * 50,
        "a full index wrote {} rows and a one-file edit wrote {}; the gate is not measuring \
         anything",
        large.full_rows_written,
        large.rows_written
    );
}

// ---- speed ---------------------------------------------------------------------------------

/// Outcome of one speed measurement.
struct SpeedResult {
    files: usize,
    bytes: u64,
    full_runs: Vec<f64>,
    incremental_runs: Vec<f64>,
    re_extracted: usize,
    amplification: f64,
}

impl SpeedResult {
    fn full_min(&self) -> f64 {
        self.full_runs.iter().cloned().fold(f64::INFINITY, f64::min)
    }

    fn incremental_min(&self) -> f64 {
        self.incremental_runs
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min)
    }

    /// Ratio of each run's incremental time to the full time measured beside it.
    ///
    /// Preferred over `min(incremental) / min(full)`. Under contention both members of a pair are
    /// inflated together, so their ratio is stable, whereas taking the best of each series
    /// independently pairs a lucky incremental run with an unlucky full one and reports a figure
    /// no run achieved.
    fn per_run_ratios(&self) -> Vec<f64> {
        self.full_runs
            .iter()
            .zip(self.incremental_runs.iter())
            .map(|(full, incremental)| incremental / full)
            .collect()
    }

    /// Median per-run ratio — the figure to quote.
    fn ratio(&self) -> f64 {
        let mut ratios = self.per_run_ratios();
        ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ratios[ratios.len() / 2]
    }

    fn report(&self, label: &str) {
        println!("--- {label} ---");
        println!("  files               {}", self.files);
        println!(
            "  source bytes        {} ({} bytes/file)",
            self.bytes,
            self.bytes / self.files.max(1) as u64
        );
        for (index, (full, incremental)) in self
            .full_runs
            .iter()
            .zip(self.incremental_runs.iter())
            .enumerate()
        {
            println!(
                "  run {}               full {full:.1} ms · incremental {incremental:.1} ms · \
                 ratio {:.1}%",
                index + 1,
                100.0 * incremental / full
            );
        }
        println!("  full min            {:.1} ms", self.full_min());
        println!("  incremental min     {:.1} ms", self.incremental_min());
        println!(
            "  ratio min/min       {:.1}%  (biased under load; reported for completeness)",
            100.0 * self.incremental_min() / self.full_min()
        );
        println!(
            "  ratio median        {:.1}%  <- the figure to quote",
            self.ratio() * 100.0
        );
        println!("  files re-extracted  {}", self.re_extracted);
        println!("  amplification       {:.2}", self.amplification);
    }
}

/// Build a `CLUSTERS x PER_CLUSTER` repository, then time a full index against a one-file edit.
///
/// `bulk` sets how much each module declares beyond its import glue, which is the variable that
/// decides the answer: the fraction of a full index that is *parsing* is the fraction an
/// incremental run can avoid.
fn measure_single_file_edit(bulk: usize) -> SpeedResult {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();

    let mut modules = cluster_modules(CLUSTERS, bulk);
    materialize(&root, &modules);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();

    let bytes: u64 = modules.iter().map(|m| m.render().len() as u64).sum();
    let mut result = SpeedResult {
        files: modules.len(),
        bytes,
        full_runs: Vec::new(),
        incremental_runs: Vec::new(),
        re_extracted: 0,
        amplification: 0.0,
    };

    for repeat in 0..REPEATS {
        let started = std::time::Instant::now();
        let full = index_full(&root);
        let full_elapsed = started.elapsed();
        assert_eq!(full.files_processed, result.files);

        // Edit the deepest module of one cluster: its importers are the rest of that cluster.
        let victim = modules
            .iter_mut()
            .find(|m| m.rel_path == format!("src/c0/m{}.ts", PER_CLUSTER - 1))
            .unwrap();
        victim.salt = 10_000 + repeat as u64;
        let rendered = victim.render();
        let victim_path = victim.rel_path.clone();
        write(&root, &victim_path, &rendered);

        let started = std::time::Instant::now();
        let incremental = index(&root);
        let incremental_elapsed = started.elapsed();

        result.re_extracted = incremental.incremental.files_re_extracted;
        result.amplification = incremental.incremental.amplification().unwrap_or_default();
        result.full_runs.push(full_elapsed.as_secs_f64() * 1000.0);
        result
            .incremental_runs
            .push(incremental_elapsed.as_secs_f64() * 1000.0);
    }

    result
}

/// Acceptance criterion 5, opt-in.
///
/// Ignored by default because it builds and indexes a 520-module repository six times. Run it
/// with:
///
/// ```text
/// cargo test -p nerve-index --test incremental --release -- --ignored --nocapture
/// ```
///
/// Every measurement is repeated and **all** runs are printed. Wall-clock timing under
/// contention is an upper bound on the true cost, so the minimum across runs is the tightest
/// honest figure; a single number from a busy machine is not evidence.
///
/// # This test is the weaker of the two gates, deliberately
///
/// A ratio is machine-, load- and corpus-dependent. The durable requirement is
/// [`a_one_file_leaf_edit_writes_the_same_rows_whatever_the_repository_size`], which counts rows
/// and cannot be flattered by fast hardware. This one exists because a ratio is what a user
/// feels, and because it catches costs that write no rows at all.
///
/// # What the measurement said before and after Slice 3b
///
/// Slice 3 measured 24.9% on the realistic corpus, against a < 20% target. Extraction was 8 ms
/// of a 2900 ms incremental run; the rest was database maintenance proportional to repository
/// size:
///
/// | phase | Slice 3 | Slice 3b |
/// |---|---|---|
/// | read + hash every file | 66 ms | unchanged — the state is a Merkle over contents |
/// | extract the invalidation set | 8 ms | unchanged — this is what incremental saves |
/// | restate every surviving row to the new state | 1330 ms | **gone** (ADR-0006) |
/// | rebuild `assertion_state` whole-table | 960 ms | scoped to the assertions that moved |
/// | prune orphans whole-table | 196 ms | scoped to the rows that could have been orphaned |
///
/// Both corpora are reported. With stub modules, parsing is only a small part of a full index,
/// so the ratio is bounded below by the read-and-hash pass however good invalidation is; with
/// realistic modules parsing dominates a full index, which is where incremental indexing pays.
#[test]
#[ignore = "builds and indexes a 520-module repository six times; opt in with --ignored"]
fn a_single_file_edit_costs_a_fraction_of_a_full_index() {
    /// Acceptance criterion 5.
    const TARGET_RATIO: f64 = 0.20;

    let stubs = measure_single_file_edit(0);
    stubs.report("stub modules (~12 lines each)");

    let realistic = measure_single_file_edit(24);
    realistic.report("realistic modules (~230 lines each)");

    println!(
        "\ntarget {:.0}% · measured {:.1}% (stubs) and {:.1}% (realistic)",
        TARGET_RATIO * 100.0,
        stubs.ratio() * 100.0,
        realistic.ratio() * 100.0
    );

    // The invalidation rule is gated strictly: a one-file edit must not reach outside the
    // changed module's import cluster.
    for result in [&stubs, &realistic] {
        assert!(
            result.re_extracted <= PER_CLUSTER,
            "a one-file edit invalidated {} files; the closure is not bounded by the import graph",
            result.re_extracted
        );
        assert_eq!(result.amplification, result.re_extracted as f64);
    }
    assert_eq!(
        realistic.re_extracted, 1,
        "a leaf edit must re-extract exactly the leaf"
    );

    assert!(
        realistic.ratio() < TARGET_RATIO,
        "a single-file edit cost {:.1}% of a full index; acceptance criterion 5 wants < {:.0}%",
        realistic.ratio() * 100.0,
        TARGET_RATIO * 100.0
    );
}

/// Every extractor an index run writes must be one it also withdraws.
///
/// Slice 6b split the withdrawal in two: a **re-extracted** file loses only the evidence
/// [`nerve_index::INDEX_EXTRACTOR_IDS`] names, while a **removed** file loses everything. That
/// split is what stops an ordinary post-edit `nerve index` from destroying coverage, which is
/// ingested by a separate command and never replaced by an index run.
///
/// It also creates a silent failure mode. `INDEX_EXTRACTOR_IDS` is a hand-maintained list, and a
/// future slice that adds an extractor without adding it here would leave that extractor's
/// observations un-withdrawn on every re-extraction — stale evidence surviving an edit forever,
/// with nothing failing to say so.
///
/// So the list is checked against what an index run *actually wrote*, not against a second list
/// someone would have to remember to update. The tree is `md-docs` — TypeScript and Markdown —
/// plus one Python module written in here rather than committed into the fixture, so that every
/// extractor fires on one tree without changing what `documents.rs` measures over the same
/// corpus.
///
/// The module has to contain a *call*, not just a definition: Slice 9b added `py-reference`, and
/// a Python file with no reference site in it makes that extractor write nothing, which would
/// leave the equality below failing for a reason that has nothing to do with withdrawal.
#[test]
fn every_extractor_an_index_run_writes_is_one_it_also_withdraws() {
    let (_dir, root) = named_fixture_copy("md-docs");
    write(
        &root,
        "src/tool.py",
        "\"\"\"One Python module, so both Python extractors write something here too.\"\"\"\n\
         \n\
         import os\n\
         \n\
         \n\
         def scale(value):\n\
         \x20   return value * 2\n\
         \n\
         \n\
         def run(value):\n\
         \x20   return scale(value)\n",
    );
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    index(&root);
    let conn = open_db(&root);

    let mut written: BTreeSet<String> = BTreeSet::new();
    let mut statement = conn
        .prepare("SELECT DISTINCT extractor_id FROM observation ORDER BY 1")
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap();
    for row in rows {
        written.insert(row.unwrap());
    }

    let withdrawn: BTreeSet<String> = nerve_index::INDEX_EXTRACTOR_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect();

    let unwithdrawn: Vec<&String> = written.difference(&withdrawn).collect();
    assert!(
        unwithdrawn.is_empty(),
        "an index run wrote observations from {unwithdrawn:?}, which INDEX_EXTRACTOR_IDS does not \
         name. Re-extracting a file will not withdraw them, so an edit will leave stale evidence \
         behind. Add them to INDEX_EXTRACTOR_IDS in crates/nerve-index/src/pipeline.rs."
    );

    // The tree must keep exercising every extractor, or the check above passes vacuously for
    // whichever one stopped firing.
    assert_eq!(
        written, withdrawn,
        "the tree no longer exercises every extractor in INDEX_EXTRACTOR_IDS, so this test would \
         stop protecting the ones that went missing"
    );
}
