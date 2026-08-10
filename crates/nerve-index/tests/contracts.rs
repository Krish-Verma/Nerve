//! C1, C2 and C3 as behaviour: what the scan links, what it refuses, and what it must never invent
//! (Slices 13b and 13c).
//!
//! Precision is measured next door in `contract_precision.rs`, against ground truth written before
//! the resolver existed. This file asserts the properties that are not a precision number:
//! idempotency, the bounds, the absence of fuzzy linking, the absence of auto-registration, the
//! neighbour's bytes, the freshness a stored link reports once the world moves under it — and, for
//! C2, the two properties that only exist because it reaches a file entity in another repository:
//! **no local proxy is created for a foreign target**, and **no ordinary `path` or `impact` query
//! traverses the link**. The second is asserted negatively, by answering the same question with the
//! links present and with them deleted.
//!
//! Two habits, both inherited from `registry.rs` one slice over.
//!
//! **Every repository gets its own `project_id`.** `repo_id` derives from it
//! (`nerve_core::ids::repository_id`), and resolution is *by repository id* — so a fixture whose
//! halves shared an identity would make every resolution assertion pass for the wrong reason.
//!
//! **A negative is never asserted alone.** *"No link was made"* is satisfied by a scan that cannot
//! make links, so the fuzzy test asserts an available registered neighbour in the same breath, and
//! the byte test asserts the read produced links before it claims the bytes are unchanged.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::copy_tree;
use nerve_core::vocab::ContractFreshness;
use nerve_index::contracts::{
    link_freshness, scan_contracts, ContractRule, ContractScan, ManifestRefusal, ScanOutcome,
    UnresolvedReason, MAX_DECLARATIONS_PER_MANIFEST, MAX_LINKS_PER_REPOSITORY, MAX_MANIFEST_BYTES,
};
use nerve_index::registry::{
    add_registry_target, availability_of, probe_target, remove_registry_target, RegistryOutcome,
};

/// A distinct project id per repository, so no two checkouts share a `repo_id`.
fn project_id(seed: u8) -> String {
    format!("{:032x}", 0xb0000000u64 + u64::from(seed))
}

/// Copy one sub-tree of a contract fixture into `<base>/<name>`, without initialising it.
fn place(base: &Path, fixture: &str, name: &str) -> PathBuf {
    let root = base.join(name);
    copy_tree(
        &common::named_fixture_root(&format!("{fixture}/{name}")),
        &root,
    );
    root
}

fn initialise(root: &Path, seed: u8) {
    nerve_index::init_with_project_id(root, Some(&project_id(seed))).unwrap();
    nerve_index::index_repository(root).unwrap();
}

fn conn_for(root: &Path) -> nerve_store::Connection {
    nerve_store::open(&nerve_index::config::db_path(root)).unwrap()
}

fn repo_id_of(root: &Path) -> String {
    nerve_store::repository(&conn_for(root))
        .unwrap()
        .unwrap()
        .repo_id
}

fn register(source: &Path, target: &Path, id: &str) {
    let conn = conn_for(source);
    let repo_id = repo_id_of(source);
    match add_registry_target(&conn, &repo_id, target, Some(id), None).unwrap() {
        RegistryOutcome::Done(_) => {}
        RegistryOutcome::Refused(reason) => panic!("{id} was refused: {reason}"),
    }
}

fn scan(root: &Path) -> ContractScan {
    let conn = conn_for(root);
    let repo_id = repo_id_of(root);
    match scan_contracts(&conn, &repo_id, root).unwrap() {
        ScanOutcome::Done(scan) => *scan,
        ScanOutcome::Refused(reason) => panic!("the scan refused: {reason}"),
    }
}

fn stored_links(root: &Path) -> Vec<nerve_store::ContractLinkRow> {
    let conn = conn_for(root);
    let repo_id = repo_id_of(root);
    nerve_store::list_contract_links(&conn, &repo_id).unwrap()
}

/// BLAKE3 of a file, so "unchanged" is a hash comparison rather than a length comparison.
fn digest(path: &Path) -> String {
    nerve_core::ids::content_hash(&std::fs::read(path).unwrap())
}

/// `app` with `lib-core` and `lib-extra` registered, and `unregistered` deliberately not.
struct NpmWorld {
    _dir: tempfile::TempDir,
    app: PathBuf,
    core: PathBuf,
    extra: PathBuf,
}

fn npm_world() -> NpmWorld {
    let dir = tempfile::tempdir().unwrap();
    let app = place(dir.path(), "contracts-npm", "app");
    let core = place(dir.path(), "contracts-npm", "lib-core");
    let extra = place(dir.path(), "contracts-npm", "lib-extra");
    let unregistered = place(dir.path(), "contracts-npm", "unregistered");
    for (root, seed) in [(&app, 1), (&core, 2), (&extra, 3), (&unregistered, 4)] {
        initialise(root, seed);
    }
    register(&app, &core, "core");
    register(&app, &extra, "extra");
    NpmWorld {
        _dir: dir,
        app,
        core,
        extra,
    }
}

/// `service` with `pkg-core` and `pkg-extra` registered.
///
/// The PEP 621 direct reference is an **absolute** URL, so the fixture ships a placeholder and the
/// harness substitutes the built path. An absolute path cannot be committed: it would name the
/// machine the fixture was built on, which `crates/nerve-cli/tests/registry_guards.rs` fails on.
struct PythonWorld {
    _dir: tempfile::TempDir,
    service: PathBuf,
    core: PathBuf,
}

fn python_world() -> PythonWorld {
    let dir = tempfile::tempdir().unwrap();
    let service = place(dir.path(), "contracts-python", "service");
    let core = place(dir.path(), "contracts-python", "pkg-core");
    let extra = place(dir.path(), "contracts-python", "pkg-extra");
    let unregistered = place(dir.path(), "contracts-python", "unregistered");

    let manifest = service.join("pyproject.toml");
    let template = std::fs::read_to_string(&manifest).unwrap();
    assert!(
        template.contains("{{PKG_CORE_PATH}}"),
        "the fixture no longer carries the placeholder this harness substitutes"
    );
    let resolved = template.replace(
        "{{PKG_CORE_PATH}}",
        &core.canonicalize().unwrap().to_string_lossy(),
    );
    std::fs::write(&manifest, resolved).unwrap();

    for (root, seed) in [
        (&service, 11),
        (&core, 12),
        (&extra, 13),
        (&unregistered, 14),
    ] {
        initialise(root, seed);
    }
    register(&service, &core, "core");
    register(&service, &extra, "extra");
    PythonWorld {
        _dir: dir,
        service,
        core,
    }
}

// ---- §9.7: fuzzy linking is asserted absent ----------------------------------------------------

/// **§9.7.** Two adjacent checkouts with the same package name and no declaration produce nothing.
///
/// The registration is what stops this being vacuous: a scan that produced zero links because it had
/// no neighbour to link to would prove nothing about name matching. So the neighbour is registered,
/// re-derived as available, and *then* the answer is zero.
#[test]
fn same_named_packages_that_declare_nothing_produce_no_link() {
    let dir = tempfile::tempdir().unwrap();
    let left = place(dir.path(), "contracts-fuzzy", "left");
    let right = place(dir.path(), "contracts-fuzzy", "right");
    initialise(&left, 21);
    initialise(&right, 22);
    register(&left, &right, "neighbour");

    // Anti-vacuity 1: the two really do share both package names.
    for manifest in ["package.json", "pyproject.toml"] {
        let mine = std::fs::read_to_string(left.join(manifest)).unwrap();
        let theirs = std::fs::read_to_string(right.join(manifest)).unwrap();
        assert!(
            mine.contains("shared-name") && theirs.contains("shared-name"),
            "{manifest} no longer declares the shared name this test is about"
        );
    }
    // Anti-vacuity 2: the neighbour is registered and readable right now.
    let conn = conn_for(&left);
    let entries = nerve_store::list_registry_entries(&conn, &repo_id_of(&left)).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(availability_of(&entries[0]).is_usable());
    drop(conn);

    let scan = scan(&left);
    assert_eq!(
        scan.manifests_read, 2,
        "both manifests must be read, or the zero below is a zero of nothing"
    );
    assert!(scan.declarations >= 3, "{scan:?}");
    assert!(
        scan.links.is_empty(),
        "a link was made between same-named packages that declare nothing about each other: {:?}",
        scan.links
    );
    assert!(stored_links(&left).is_empty());
}

// ---- idempotency -------------------------------------------------------------------------------

/// Re-running the scan over an unchanged tree writes nothing new and duplicates nothing.
///
/// The unique index on the logical identity is what makes this possible rather than lucky: without
/// it the surrogate key would append a row on every run. `first_seen_at` must not move, because the
/// date something started is not re-datable by looking again.
#[test]
fn re_running_the_scan_records_nothing_new_and_duplicates_nothing() {
    let world = npm_world();

    let first = scan(&world.app);
    assert_eq!(first.inserted(), 5, "{:?}", first.links);
    assert_eq!(first.unchanged(), 0);
    let after_first = stored_links(&world.app);
    assert_eq!(after_first.len(), 5);

    let second = scan(&world.app);
    assert_eq!(second.inserted(), 0, "{:?}", second.links);
    assert_eq!(second.unchanged(), 5);
    let after_second = stored_links(&world.app);
    assert_eq!(after_second.len(), 5, "the scan appended a duplicate row");

    for (before, after) in after_first.iter().zip(after_second.iter()) {
        assert_eq!(before.link_id, after.link_id, "a link changed identity");
        assert_eq!(
            before.first_seen_at, after.first_seen_at,
            "first_seen_at was re-dated by looking again"
        );
        assert!(
            after.last_seen_at >= after.first_seen_at,
            "last_seen_at moved behind first_seen_at"
        );
    }

    // A third run over a re-indexed tree is still not a duplicate: the identity is the declaration,
    // not the state it was read at.
    nerve_index::index_repository(&world.app).unwrap();
    let third = scan(&world.app);
    assert_eq!(third.inserted(), 0);
    assert_eq!(stored_links(&world.app).len(), 5);
}

// ---- the three bounds, each exercised ----------------------------------------------------------

/// [`MAX_MANIFEST_BYTES`]: an oversized manifest is refused by name and read no further.
#[test]
fn an_oversized_manifest_is_refused_with_its_bound_named() {
    let world = npm_world();
    let directory = world.app.join("oversize");
    std::fs::create_dir_all(&directory).unwrap();
    let padding = "x".repeat(MAX_MANIFEST_BYTES as usize);
    std::fs::write(
        directory.join("package.json"),
        format!("{{\n  \"name\": \"{padding}\",\n  \"dependencies\": {{}}\n}}\n"),
    )
    .unwrap();

    let scan = scan(&world.app);
    assert!(
        scan.refusals
            .iter()
            .any(|(path, refusal)| path == "oversize/package.json"
                && *refusal == ManifestRefusal::ManifestTooLarge),
        "{:?}",
        scan.refusals
    );
    // The rest of the repository is still read: a bound stops one file, not the scan.
    assert_eq!(scan.manifests_read, 1);
    assert_eq!(scan.inserted(), 5);
}

/// [`MAX_DECLARATIONS_PER_MANIFEST`]: the whole manifest is refused, never half-read.
#[test]
fn a_manifest_with_too_many_declarations_is_refused_with_its_bound_named() {
    let world = npm_world();
    let directory = world.app.join("crowded");
    std::fs::create_dir_all(&directory).unwrap();
    let entries: Vec<String> = (0..=MAX_DECLARATIONS_PER_MANIFEST)
        .map(|index| format!("    \"dep-{index}\": \"^1.0.0\""))
        .collect();
    std::fs::write(
        directory.join("package.json"),
        format!(
            "{{\n  \"name\": \"crowded\",\n  \"dependencies\": {{\n{}\n  }}\n}}\n",
            entries.join(",\n")
        ),
    )
    .unwrap();

    let scan = scan(&world.app);
    assert!(
        scan.refusals
            .iter()
            .any(|(path, refusal)| path == "crowded/package.json"
                && *refusal == ManifestRefusal::TooManyDeclarations),
        "{:?}",
        scan.refusals
    );
    // Nothing from the refused manifest leaked into the tally: a refused file contributes no
    // declarations at all, which is what "refused rather than truncated" has to mean.
    assert!(
        !scan
            .unsupported
            .iter()
            .any(|row| row.manifest == "crowded/package.json"),
        "the refused manifest still contributed declarations"
    );
    assert_eq!(scan.inserted(), 5);
}

/// [`MAX_LINKS_PER_REPOSITORY`]: the scan stops at the bound and says so.
#[test]
fn the_link_budget_stops_the_scan_and_names_the_bound() {
    let world = npm_world();
    let directory = world.app.join("many");
    std::fs::create_dir_all(&directory).unwrap();
    let entries: Vec<String> = (0..MAX_LINKS_PER_REPOSITORY + 200)
        .map(|index| format!("    \"many-{index:04}\": \"file:../../lib-core\""))
        .collect();
    std::fs::write(
        directory.join("package.json"),
        format!(
            "{{\n  \"name\": \"many\",\n  \"dependencies\": {{\n{}\n  }}\n}}\n",
            entries.join(",\n")
        ),
    )
    .unwrap();

    let first = scan(&world.app);
    assert!(
        first
            .refusals
            .iter()
            .any(|(_, refusal)| *refusal == ManifestRefusal::LinkBudgetExhausted),
        "{:?}",
        first.refusals
    );
    assert_eq!(first.inserted(), MAX_LINKS_PER_REPOSITORY);
    assert_eq!(stored_links(&world.app).len(), MAX_LINKS_PER_REPOSITORY);

    // The bound is against the table, not against the run, so a second scan cannot grow past it.
    let again = scan(&world.app);
    assert_eq!(again.inserted(), 0);
    assert_eq!(stored_links(&world.app).len(), MAX_LINKS_PER_REPOSITORY);
}

// ---- what the scan must never do ---------------------------------------------------------------

/// A declared path reaching an unregistered repository is reported, never registered.
///
/// This is §1's "directory proximity" refusal at the resolution layer: `app` names
/// `../unregistered` with an explicit `file:` path, that directory really is an indexed Nerve
/// repository, and it is still not linked and still not registered.
#[test]
fn a_declared_path_to_an_unregistered_repository_is_never_auto_registered() {
    let world = npm_world();
    let scan = scan(&world.app);

    assert!(
        scan.unresolved
            .iter()
            .any(|row| row.identity == "lib-unregistered"
                && row.reason == UnresolvedReason::TargetNotRegistered),
        "{:?}",
        scan.unresolved
    );
    assert!(
        !scan
            .links
            .iter()
            .any(|link| link.identity == "lib-unregistered"),
        "an unregistered repository was linked"
    );

    let conn = conn_for(&world.app);
    let ids: BTreeSet<String> = nerve_store::list_registry_entries(&conn, &repo_id_of(&world.app))
        .unwrap()
        .into_iter()
        .map(|entry| entry.registry_id)
        .collect();
    assert_eq!(
        ids,
        ["core", "extra"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>(),
        "the scan registered something nobody named"
    );
}

/// **§9.3c and T12 control 2.** A scan adds no entity, no assertion, and no proxy for a foreign
/// target — and the neighbour's database is byte-identical afterwards.
#[test]
fn a_scan_adds_no_entity_no_assertion_and_leaves_the_neighbour_untouched() {
    let world = npm_world();
    let core_db = nerve_index::registry::target_database_path(&world.core);
    let extra_db = nerve_index::registry::target_database_path(&world.extra);
    let before = (digest(&core_db), digest(&extra_db));

    let conn = conn_for(&world.app);
    let entities_before = common::count(&conn, "entity");
    let assertions_before = common::count(&conn, "assertion");
    let observations_before = common::count(&conn, "observation");
    drop(conn);

    let scan = scan(&world.app);
    assert_eq!(
        scan.inserted(),
        5,
        "no link was written, so nothing is proven"
    );

    let conn = conn_for(&world.app);
    assert_eq!(common::count(&conn, "entity"), entities_before);
    assert_eq!(common::count(&conn, "assertion"), assertions_before);
    assert_eq!(common::count(&conn, "observation"), observations_before);

    // No proxy entity for a foreign target: every link's target columns are NULL, so there is
    // nothing that could be mistaken for a local row.
    for link in nerve_store::list_contract_links(&conn, &repo_id_of(&world.app)).unwrap() {
        assert_eq!(link.target_entity_id, None, "{link:?}");
        assert_eq!(link.target_path_snapshot, None, "{link:?}");
        assert_eq!(link.source_entity_id, None, "{link:?}");
        assert!(link.target_state_at_resolution.is_some(), "{link:?}");
    }
    drop(conn);

    assert_eq!(
        before,
        (digest(&core_db), digest(&extra_db)),
        "a neighbour's database changed while its manifests were read"
    );
}

/// A link is quoted from a place, and the place is a line that exists.
#[test]
fn every_link_names_a_line_of_the_manifest_it_was_read_from() {
    let world = npm_world();
    let scan = scan(&world.app);
    let manifest_lines = std::fs::read_to_string(world.app.join("package.json"))
        .unwrap()
        .lines()
        .count();
    let mut spans = BTreeSet::new();
    for link in &scan.links {
        let (start, end) = link.source_span.split_once(':').expect("line:line");
        let start: usize = start.parse().unwrap();
        assert_eq!(start.to_string(), end);
        assert!(start >= 1 && start <= manifest_lines, "{link:?}");
        spans.insert(link.source_span.clone());
    }
    assert_eq!(
        spans.len(),
        scan.links.len(),
        "two links share a span, so the manifest is not being read line by line"
    );
}

// ---- freshness, once the world moves under a stored link ---------------------------------------

/// What a stored link reports right now.
fn freshness_of(source: &Path, registry_id: &str) -> Vec<Option<ContractFreshness>> {
    let conn = conn_for(source);
    let repo_id = repo_id_of(source);
    let entry = nerve_store::registry_entry(&conn, &repo_id, registry_id)
        .unwrap()
        .unwrap();
    let availability = availability_of(&entry);
    let target_state = probe_target(Path::new(&entry.local_path))
        .ok()
        .and_then(|target| target.state_id);
    let source_state = nerve_store::status(&conn).unwrap().state_id.unwrap();
    let manifest_present = source.join("package.json").exists();

    nerve_store::contract_links_for_registry_entry(&conn, &repo_id, registry_id)
        .unwrap()
        .iter()
        .map(|link| {
            link_freshness(
                link,
                &availability,
                target_state.as_deref(),
                &source_state,
                manifest_present,
            )
        })
        .collect()
}

/// The stale fixture: a link resolved at one pair of states, then each side moves.
///
/// Four of the twelve states are reached here and each is distinguished from its neighbour —
/// unchanged is `None` rather than a state, which matters because "nothing is wrong" and "we did
/// not look" are the two answers Slice 7c-i exists to keep apart.
#[test]
fn a_stored_link_reports_which_side_moved() {
    let world = npm_world();
    assert_eq!(scan(&world.app).inserted(), 5);

    assert!(
        freshness_of(&world.app, "core")
            .iter()
            .all(|freshness| freshness.is_none()),
        "a freshly resolved link is already stale"
    );

    // The neighbour moves on.
    std::fs::write(
        world.core.join("src/added.ts"),
        "export function added(): number {\n  return 3;\n}\n",
    )
    .unwrap();
    nerve_index::index_repository(&world.core).unwrap();
    assert_eq!(
        freshness_of(&world.app, "core"),
        vec![
            Some(ContractFreshness::TargetChanged),
            Some(ContractFreshness::TargetChanged)
        ]
    );

    // And then this repository does too.
    std::fs::write(
        world.app.join("src/added.ts"),
        "export function added(): number {\n  return 4;\n}\n",
    )
    .unwrap();
    nerve_index::index_repository(&world.app).unwrap();
    assert_eq!(
        freshness_of(&world.app, "core"),
        vec![
            Some(ContractFreshness::BothChanged),
            Some(ContractFreshness::BothChanged)
        ]
    );

    // The manifest the link was quoted from is deleted.
    std::fs::remove_file(world.app.join("package.json")).unwrap();
    assert!(freshness_of(&world.app, "core")
        .iter()
        .all(|freshness| *freshness == Some(ContractFreshness::ContractFileMissing)));
}

/// Retiring the entry withdraws its links, and the report says which of the two facts it is.
///
/// `registry_entry_removed` wins over `contract_deleted` because the entry being retired is *why*
/// the link ended, and it is the answer with a remedy.
#[test]
fn retiring_an_entry_makes_its_links_report_the_entry_rather_than_the_declaration() {
    let world = npm_world();
    assert_eq!(scan(&world.app).inserted(), 5);

    let conn = conn_for(&world.app);
    let repo_id = repo_id_of(&world.app);
    assert!(matches!(
        remove_registry_target(&conn, &repo_id, "core").unwrap(),
        RegistryOutcome::Done(_)
    ));
    drop(conn);

    let reported = freshness_of(&world.app, "core");
    assert_eq!(reported.len(), 2);
    assert!(
        reported
            .iter()
            .all(|freshness| *freshness == Some(ContractFreshness::RegistryEntryRemoved)),
        "{reported:?}"
    );
    // The other entry is untouched: retiring one neighbour says nothing about another.
    assert!(freshness_of(&world.app, "extra")
        .iter()
        .all(Option::is_none));
}

/// A neighbour that has gone missing is not a neighbour that changed.
#[test]
fn a_missing_neighbour_is_reported_as_missing_and_not_as_changed() {
    let world = npm_world();
    assert_eq!(scan(&world.app).inserted(), 5);
    std::fs::remove_dir_all(&world.core).unwrap();

    assert!(freshness_of(&world.app, "core")
        .iter()
        .all(|freshness| *freshness == Some(ContractFreshness::TargetRepositoryMissing)));
    assert!(freshness_of(&world.app, "extra")
        .iter()
        .all(Option::is_none));
}

// ---- C3 -----------------------------------------------------------------------------------------

/// The Python rule reads all three of its sections, and the resolution method says which.
#[test]
fn the_python_rule_reads_pep621_poetry_and_uv() {
    let world = python_world();
    let scan = scan(&world.service);

    let sections: BTreeSet<&str> = scan
        .links_of(ContractRule::PythonPathDependency)
        .map(|link| link.section.as_str())
        .collect();
    assert_eq!(
        sections,
        [
            "project.dependencies",
            "tool.poetry.dependencies",
            "tool.uv.sources"
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
    assert_eq!(scan.inserted(), 5, "{:?}", scan.links);
    assert!(scan
        .links_of(ContractRule::NpmLocalDependency)
        .next()
        .is_none());

    // The neighbour's own version is recorded, so `contract_version_mismatch` has evidence to be
    // decided from later. It is not decided here: `^1.2.0` against `1.2.3` is range satisfaction.
    let core_version = scan
        .links
        .iter()
        .find(|link| link.registry_id == "core")
        .and_then(|link| link.observed_contract_version.clone());
    assert_eq!(core_version.as_deref(), Some("1.4.0"));
    assert!(world.core.join("pyproject.toml").exists());
}

// ---- C2: the export map, and the file entity on the far side -----------------------------------

/// `host` with five neighbours registered and `pkg-unregistered` deliberately not.
struct ExportWorld {
    _dir: tempfile::TempDir,
    host: PathBuf,
    map: PathBuf,
}

fn export_world() -> ExportWorld {
    let dir = tempfile::tempdir().unwrap();
    let host = place(dir.path(), "contracts-exports", "host");
    let mut roots = Vec::new();
    for (offset, name) in [
        "pkg-map",
        "pkg-string",
        "pkg-legacy",
        "twin-a",
        "twin-b",
        "pkg-unregistered",
    ]
    .into_iter()
    .enumerate()
    {
        let root = place(dir.path(), "contracts-exports", name);
        initialise(&root, 31 + offset as u8);
        roots.push((name, root));
    }
    initialise(&host, 30);
    for (name, root) in &roots[..5] {
        register(&host, root, name);
    }
    let map = roots[0].1.clone();
    ExportWorld {
        _dir: dir,
        host,
        map,
    }
}

/// Every entity id in a repository, so "this id is not local" is a scan rather than a claim.
fn entity_ids(root: &Path) -> BTreeSet<String> {
    let conn = conn_for(root);
    let mut stmt = conn.prepare("SELECT entity_id FROM entity").unwrap();
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    ids
}

/// **The link the row exists for.** `host/src/app.ts` REFERENCES `pkg-map/src/sub.ts`, by
/// `export_map_resolved`, with a local source entity and a foreign target that has no local row.
#[test]
fn an_export_link_names_a_file_entity_in_the_neighbour_and_no_local_proxy_for_it() {
    let world = export_world();
    let scan = scan(&world.host);

    let link = scan
        .links_of(ContractRule::NpmExportResolution)
        .find(|link| link.identity == "pkg-map/sub")
        .expect("pkg-map/sub produced no link");
    assert_eq!(link.manifest, "src/app.ts");
    assert_eq!(link.section, "pkg-map");
    assert_eq!(link.registry_id, "pkg-map");
    assert_eq!(link.resolution_method.as_str(), "export_map_resolved");
    assert_eq!(link.relation_semantics, "REFERENCES");
    assert_eq!(link.target_path.as_deref(), Some("src/sub.ts"));
    let target_entity = link
        .target_entity_id
        .clone()
        .expect("the neighbour indexed src/sub.ts, so the link must name its entity");
    let source_entity = link.source_entity_id.clone().expect("the source is local");

    // §9.3c, for the one rule that has a foreign entity to be tempted by: the source id is a row
    // in this repository and the target id is a row in the neighbour's, and neither database
    // gained the other's.
    let local = entity_ids(&world.host);
    assert!(local.contains(&source_entity));
    assert!(
        !local.contains(&target_entity),
        "a proxy entity was created for a target this repository never indexed"
    );
    assert!(entity_ids(&world.map).contains(&target_entity));

    // The stored row carries the whole snapshot, because a target that later moves must still be
    // nameable. `assertion` would have refused this row outright; `contract_link` is where it goes.
    let conn = conn_for(&world.host);
    let stored = nerve_store::list_contract_links(&conn, &repo_id_of(&world.host))
        .unwrap()
        .into_iter()
        .find(|row| row.contract_identity == "pkg-map/sub")
        .unwrap();
    assert_eq!(stored.relation_semantics, "REFERENCES");
    assert_eq!(stored.contract_kind, "npm_export_resolution");
    assert_eq!(stored.target_entity_id.as_deref(), Some(&*target_entity));
    assert_eq!(stored.target_kind_snapshot.as_deref(), Some("module"));
    assert_eq!(stored.target_path_snapshot.as_deref(), Some("src/sub.ts"));
    assert!(stored.target_span_snapshot.is_some());
    assert_eq!(stored.observed_contract_version.as_deref(), Some("3.1.0"));

    // And no assertion row names either end of it. `assertion.target_entity_id` is
    // `NOT NULL REFERENCES entity(entity_id)`, so this row could not have been inserted there —
    // the scan is asserted not to have tried a proxy instead.
    let named_in_assertions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assertion WHERE target_entity_id = ?1",
            [&target_entity],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(named_in_assertions, 0);
}

/// **§9.3b, asserted negatively.** A `path` and an `impact` query answer identically with the
/// contract links present and with them deleted.
///
/// Two anti-vacuity floors, because "the answers matched" is satisfied by two empty answers: there
/// must really be C2 links naming a foreign entity, and the local queries must really find
/// something. Only then does "the same result" mean the traversal did not cross.
#[test]
fn neither_path_nor_impact_traverses_a_contract_link() {
    let world = export_world();
    let scan = scan(&world.host);
    let foreign: BTreeSet<String> = scan
        .links_of(ContractRule::NpmExportResolution)
        .filter_map(|link| link.target_entity_id.clone())
        .collect();
    assert!(
        foreign.len() >= 5,
        "only {} C2 links name a foreign entity; the comparison below would be vacuous",
        foreign.len()
    );

    let conn = conn_for(&world.host);
    let app = match nerve_store::resolve_selector(&conn, "src/app.ts").unwrap() {
        nerve_store::Selection::Resolved { entity, .. } => *entity,
        other => panic!("src/app.ts did not resolve: {other:?}"),
    };
    let local = match nerve_store::resolve_selector(&conn, "src/local.ts").unwrap() {
        nerve_store::Selection::Resolved { entity, .. } => *entity,
        other => panic!("src/local.ts did not resolve: {other:?}"),
    };
    let prober = nerve_index::RepositoryProber::new(&world.host).unwrap();
    let path_query = nerve_store::PathQuery {
        direction: nerve_store::Direction::Any,
        ..nerve_store::PathQuery::default()
    };
    // `IMPORTS` rather than the default relation set: `src/app.ts` imports `src/local.ts`, so the
    // closure has something in it and the comparison below is about a real answer.
    let impact_query = nerve_store::ImpactQuery {
        relations: vec![nerve_core::vocab::Relation::Imports],
        ..nerve_store::ImpactQuery::default()
    };

    let path_before =
        nerve_store::find_paths(&conn, &app.entity_id, &local.entity_id, &path_query).unwrap();
    let impact_before =
        nerve_store::impact(&conn, &local.entity_id, &impact_query, &prober).unwrap();
    assert!(
        !path_before.paths.is_empty(),
        "the two local modules are not connected, so this comparison proves nothing"
    );
    assert!(
        impact_before.totals.entities > 0 || impact_before.results_total > 0,
        "the impact closure is empty, so this comparison proves nothing: {impact_before:?}"
    );

    // No foreign endpoint appears anywhere in either answer.
    for hop in path_before.paths.iter().flat_map(|path| &path.hops) {
        assert!(!foreign.contains(&hop.to.entity_id), "{hop:?}");
        assert!(!foreign.contains(&hop.from.entity_id), "{hop:?}");
    }
    for row in &impact_before.results {
        assert!(!foreign.contains(&row.entity.entity_id), "{row:?}");
    }

    // And the answers do not change when the links are gone, which is the strong form: a
    // traversal that read `contract_link` at all would have to answer differently.
    let deleted = conn.execute("DELETE FROM contract_link", []).unwrap();
    assert!(deleted >= 8, "only {deleted} links were deleted");
    let path_after =
        nerve_store::find_paths(&conn, &app.entity_id, &local.entity_id, &path_query).unwrap();
    let impact_after =
        nerve_store::impact(&conn, &local.entity_id, &impact_query, &prober).unwrap();
    assert_eq!(path_before, path_after, "`path` traversed a contract link");
    assert_eq!(
        impact_before, impact_after,
        "`impact` traversed a contract link"
    );
}

/// A file the neighbour has and never indexed is `target_partially_indexed`, not a missing target.
///
/// The two are the same absence from the *index* and different facts about the *world*, which is
/// Slice 7c-i's `Stale` / `Unverified` distinction in its fourth place. The fixture produces both
/// from one manifest so the pair is asserted together rather than one at a time.
#[test]
fn a_file_the_neighbour_never_indexed_is_partially_indexed_and_a_file_it_lacks_is_unresolved() {
    let world = export_world();
    let scan = scan(&world.host);

    let data = scan
        .links_of(ContractRule::NpmExportResolution)
        .find(|link| link.identity == "pkg-map/data")
        .expect("pkg-map/data produced no link");
    assert_eq!(data.target_path.as_deref(), Some("src/data.json"));
    assert_eq!(data.target_entity_id, None);
    assert!(
        world.map.join("src/data.json").exists(),
        "the file must really be there, or this is a missing target wearing the wrong name"
    );

    // `./src/gone.ts` is named by the same map and is not in the tree at all.
    let gone = scan
        .unresolved_of(ContractRule::NpmExportResolution)
        .find(|row| row.identity == "pkg-map/gone")
        .expect("pkg-map/gone was not reported");
    assert_eq!(gone.reason, UnresolvedReason::ExportTargetMissing);
    assert!(!world.map.join("src/gone.ts").exists());

    // And the stored link reports the distinction rather than a clean bill.
    let reported = freshness_of(&world.host, "pkg-map");
    assert!(
        reported
            .iter()
            .filter(|freshness| **freshness == Some(ContractFreshness::TargetPartiallyIndexed))
            .count()
            == 1,
        "{reported:?}"
    );
    assert!(
        reported
            .iter()
            .filter(|freshness| freshness.is_none())
            .count()
            >= 4,
        "every link is qualified, so `target_partially_indexed` is not distinguishing anything: \
         {reported:?}"
    );
}

/// Every export shape the rule declines is tallied with its own name, and the tally is complete.
#[test]
fn every_declined_export_shape_is_named_in_the_tally() {
    let world = export_world();
    let scan = scan(&world.host);
    let tally = scan.unsupported_tally();
    for form in [
        "npm_export_unsupported_condition",
        "npm_export_blocked",
        "npm_export_path_escapes_target",
        "npm_export_subpath_not_declared",
        "npm_legacy_subpath_probe",
        "npm_export_wildcard_subpath",
    ] {
        assert!(
            tally
                .iter()
                .any(|(declined, count)| declined.as_str() == form && *count >= 1),
            "{form} was not tallied: {tally:?}"
        );
    }
    // Nothing was read and then forgotten, per rule.
    for rule in [
        ContractRule::NpmLocalDependency,
        ContractRule::NpmExportResolution,
    ] {
        assert!(scan.links_of(rule).count() > 0, "{rule} produced nothing");
    }
    assert_eq!(
        scan.declarations,
        scan.links_of(ContractRule::NpmLocalDependency).count()
            + scan.links_of(ContractRule::NpmExportResolution).count()
            - 1 // `pkg-twin` is one declaration and two links; both are counted above, once here.
            + scan.unresolved.len()
            + scan.unsupported.len(),
        "a declaration was read and landed in none of the three buckets"
    );
}

/// Re-running writes nothing new, and the neighbour's bytes are the same afterwards.
#[test]
fn re_running_the_export_scan_records_nothing_new_and_leaves_the_neighbour_untouched() {
    let world = export_world();
    let map_db = nerve_index::registry::target_database_path(&world.map);

    let first = scan(&world.host);
    let exports_first = first.links_of(ContractRule::NpmExportResolution).count();
    assert_eq!(exports_first, 8, "{:?}", first.links);

    let before = digest(&map_db);
    let second = scan(&world.host);
    assert_eq!(second.inserted(), 0, "{:?}", second.links);
    assert_eq!(
        second.links_of(ContractRule::NpmExportResolution).count(),
        exports_first
    );
    assert_eq!(
        digest(&map_db),
        before,
        "the neighbour's database changed while its exports were read"
    );

    let stored = stored_links(&world.host);
    // The whole logical identity, exactly as `idx_contract_link_identity` spells it — one
    // dependency declared in two sections is two declarations and legitimately two rows, so a
    // narrower tuple here would fail on a fixture that is behaving correctly.
    let identities: BTreeSet<(String, String, String, String, String, &str)> = stored
        .iter()
        .map(|row| {
            (
                row.registry_entry_id.clone(),
                row.contract_kind.clone(),
                row.contract_identity.clone(),
                row.source_path.clone(),
                row.source_span.clone(),
                row.resolution_method.as_str(),
            )
        })
        .collect();
    assert_eq!(
        identities.len(),
        stored.len(),
        "two stored links share a logical identity"
    );
}

/// A cached module with no entity at its path is passed over **by name**, never silently.
///
/// Produced by writing a `module_facts` row for a path that has no entity, which is the only way
/// the two can disagree — and the reason the refusal exists rather than a silent `continue`.
#[test]
fn a_module_with_cached_imports_and_no_entity_is_refused_by_name() {
    let world = export_world();
    let conn = conn_for(&world.host);
    let repo_id = repo_id_of(&world.host);
    let facts = nerve_index::ModuleFacts {
        import_specifiers: vec!["pkg-map/sub".to_string()],
        ..nerve_index::ModuleFacts::default()
    };
    nerve_store::upsert_module_facts(
        &conn,
        &repo_id,
        &nerve_store::ModuleFactsRow {
            rel_path: "src/never-indexed.ts".to_string(),
            content_hash: nerve_core::ids::content_hash(b"never indexed"),
            language: "typescript".to_string(),
            structural_version: "0".to_string(),
            reference_version: "0".to_string(),
            framework_version: String::new(),
            facts: facts.to_json().unwrap(),
        },
    )
    .unwrap();
    drop(conn);

    let scan = scan(&world.host);
    assert!(
        scan.refusals.iter().any(|(path, refusal)| {
            path == "src/never-indexed.ts" && *refusal == ManifestRefusal::SourceModuleNotIndexed
        }),
        "{:?}",
        scan.refusals
    );
    // And nothing was written for it: a link with no local end is not storable and is not stored.
    assert!(stored_links(&world.host)
        .iter()
        .all(|row| row.source_path != "src/never-indexed.ts"));
}
