//! The measured gate for `gitobj`, over `fixtures/gitobj`.
//!
//! Every per-object assertion here is read out of `fixtures/gitobj/inventory.json`, which is
//! **Git's** answer for the committed pack and loose objects, and every case is listed in
//! `fixtures/gitobj/expected.json`, which was written before the reader existed. Neither file is
//! produced by Nerve, so this suite cannot pass by the reader agreeing with itself.
//!
//! The corruption cases derive their bytes from the committed `.idx` and `.pack`, changing a named
//! handful of bytes each. That is deliberate: a corrupt fixture committed as bytes drifts away from
//! the format it is meant to corrupt, and a reviewer cannot see what makes it invalid. The bomb is in
//! `gitobj_bomb.rs`, in its own binary, because it measures peak allocation and nothing may run
//! beside it.

mod common;

use std::path::{Path, PathBuf};

use nerve_index::gitobj::{
    form, parse_commit, parse_tree, Error, ObjectKind, ObjectStore, Oid, PackIndex,
};

fn fixture_root() -> PathBuf {
    common::named_fixture_root("gitobj")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} must be valid JSON: {err}", path.display()))
}

/// What Git says the committed objects are.
fn inventory() -> serde_json::Value {
    read_json(&fixture_root().join("inventory.json"))
}

/// What the reader is required to do, written before the reader existed.
fn expected() -> serde_json::Value {
    read_json(&fixture_root().join("expected.json"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// A writable copy of the fixture, so a corruption case never touches the committed bytes.
fn fixture_copy() -> (tempfile::TempDir, PathBuf) {
    common::named_fixture_copy("gitobj")
}

fn packed_dir(root: &Path) -> PathBuf {
    root.join("packed")
}

fn idx_path(root: &Path) -> PathBuf {
    let name = inventory()["pack"]["idx"]
        .as_str()
        .expect("inventory names the .idx")
        .to_string();
    packed_dir(root).join("objects").join("pack").join(name)
}

fn pack_path(root: &Path) -> PathBuf {
    let name = inventory()["pack"]["pack"]
        .as_str()
        .expect("inventory names the .pack")
        .to_string();
    packed_dir(root).join("objects").join("pack").join(name)
}

/// Overwrite a file inside the fixture **copy**.
///
/// Git writes loose objects and packfiles read-only — mode `444` — and `fs::copy` preserves that, so
/// a corruption case has to make its own copy writable first. Worth a helper rather than a line in
/// each test: the failure it prevents is `PermissionDenied`, which looks like a broken test rather
/// than like a property of the format.
fn overwrite(path: &Path, bytes: &[u8]) {
    let mut permissions = std::fs::metadata(path)
        .unwrap_or_else(|err| panic!("{} must exist: {err}", path.display()))
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o644);
    }
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    {
        permissions.set_readonly(false);
    }
    std::fs::set_permissions(path, permissions).expect("the fixture copy must be writable");
    std::fs::write(path, bytes).expect("writing the corrupted copy");
}

fn oid_of(entry: &serde_json::Value) -> Oid {
    Oid::from_hex(entry["oid"].as_str().expect("an oid string")).expect("a 40-hex oid")
}

// ---- the fixture itself ------------------------------------------------------------------------

/// The fixture must be small, and it must actually contain what it claims to.
///
/// The delta assertions are the load-bearing ones: if `git gc` ever stopped producing deltas for this
/// content, every delta test below would pass over an empty set and report success. That is the
/// Slice 11a-i failure shape, and it is checked here rather than assumed.
#[test]
fn the_committed_pack_fixture_is_small_and_really_contains_deltas() {
    let root = fixture_root();
    let pack = std::fs::metadata(pack_path(&root)).expect("the .pack is committed");
    let index = std::fs::metadata(idx_path(&root)).expect("the .idx is committed");
    assert!(
        pack.len() + index.len() < 64 * 1024,
        "the committed pack fixture must stay small: {} + {} bytes",
        pack.len(),
        index.len()
    );

    let inventory = inventory();
    let entries = inventory["packed_objects"]
        .as_array()
        .expect("an object list");
    assert!(entries.len() >= 10, "only {} objects", entries.len());
    let deltas = entries
        .iter()
        .filter(|entry| entry["depth"].as_u64().unwrap_or(0) > 0)
        .count();
    assert!(deltas >= 1, "the pack contains no delta entry");
    assert_eq!(
        deltas,
        inventory["delta_entry_count"].as_u64().unwrap() as usize
    );
    assert!(
        inventory["max_delta_depth"].as_u64().unwrap() >= 2,
        "a depth-1-only pack does not exercise a delta *chain*"
    );

    let loose = inventory["loose_objects"].as_array().expect("a loose list");
    let mut kinds: Vec<&str> = loose
        .iter()
        .map(|entry| entry["type"].as_str().unwrap())
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, vec!["blob", "commit", "tag", "tree"]);
}

// ---- what it must read ------------------------------------------------------------------------

/// `real-pack-non-delta-entry`.
#[test]
fn every_non_delta_entry_git_recorded_reconstructs() {
    let store = ObjectStore::open(&packed_dir(&fixture_root())).unwrap();
    let inventory = inventory();
    let mut checked = 0;
    for entry in inventory["packed_objects"].as_array().unwrap() {
        if entry["depth"].as_u64().unwrap_or(0) != 0 {
            continue;
        }
        let oid = oid_of(entry);
        let object = store
            .read(&oid)
            .unwrap()
            .unwrap_or_else(|| panic!("{oid} is in the pack and must be readable"));
        assert_eq!(
            object.kind().as_str(),
            entry["type"].as_str().unwrap(),
            "{oid} came back the wrong type"
        );
        assert_eq!(
            object.data().len() as u64,
            entry["size"].as_u64().unwrap(),
            "{oid} came back the wrong length"
        );
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} whole entries were checked");
    assert_eq!(
        store.counters().total(),
        0,
        "a clean pack refused something"
    );
    assert_eq!(store.limits().packs_loaded, 1);
}

/// `real-pack-delta-entry`. The hardest half of the format, over the entries Git chose to delta.
#[test]
fn every_delta_entry_git_recorded_reconstructs() {
    let store = ObjectStore::open(&packed_dir(&fixture_root())).unwrap();
    let inventory = inventory();
    let mut checked = 0;
    let mut deepest = 0;
    for entry in inventory["packed_objects"].as_array().unwrap() {
        let depth = entry["depth"].as_u64().unwrap_or(0);
        if depth == 0 {
            continue;
        }
        let oid = oid_of(entry);
        let object = store
            .read(&oid)
            .unwrap()
            .unwrap_or_else(|| panic!("{oid} is a delta entry and must reconstruct"));
        assert_eq!(object.kind().as_str(), entry["type"].as_str().unwrap());
        assert_eq!(
            object.data().len() as u64,
            entry["size"].as_u64().unwrap(),
            "{oid} reconstructed to the wrong length"
        );
        // Content, not only length: a delta that copied the wrong range would often still produce
        // the right number of bytes.
        if object.kind() == ObjectKind::Blob && object.data().len() > 1000 {
            assert!(
                object
                    .data()
                    .starts_with(b"row 0000: the quick brown fox jumps over the lazy dog\n"),
                "{oid} reconstructed to the wrong bytes"
            );
            assert!(
                object.data().windows(8).any(|window| window == b"marker: "),
                "{oid} lost the marker line the fixture script wrote"
            );
        }
        deepest = deepest.max(depth);
        checked += 1;
    }
    assert!(checked >= 1, "no delta entry was checked");
    assert!(
        deepest >= 2,
        "no delta *chain* was followed, only single links"
    );
    assert_eq!(store.counters().total(), 0);
}

/// The `.idx` lookup must go through the fanout, and the table it searches must be sorted.
#[test]
fn the_committed_index_is_sorted_and_agrees_with_a_linear_scan() {
    let root = fixture_root();
    let index = PackIndex::parse(std::fs::read(idx_path(&root)).unwrap()).unwrap();
    let oids: Vec<Oid> = index.oids().collect();
    assert_eq!(oids.len(), index.len());

    let mut sorted = oids.clone();
    sorted.sort();
    assert_eq!(oids, sorted, "the id table must be ascending");

    for (position, oid) in oids.iter().enumerate() {
        assert_eq!(
            index.find(oid),
            Some(position),
            "{oid} was not found where the table holds it"
        );
    }
    // Every offset the index reports is the offset Git recorded.
    for entry in inventory()["packed_objects"].as_array().unwrap() {
        let oid = oid_of(entry);
        assert_eq!(
            index.offset_of(&oid).unwrap(),
            Some(entry["offset"].as_u64().unwrap()),
            "{oid} has the wrong offset"
        );
    }
    // An id whose first byte no object shares exercises an empty fanout bucket.
    assert_eq!(index.find(&Oid::from_hex(&"7e".repeat(20)).unwrap()), None);
}

/// `loose-all-four-types`.
#[test]
fn the_four_committed_loose_objects_parse() {
    let store = ObjectStore::open(&fixture_root().join("loose")).unwrap();
    let inventory = inventory();
    let mut kinds = Vec::new();
    for entry in inventory["loose_objects"].as_array().unwrap() {
        let oid = oid_of(entry);
        let object = store
            .read(&oid)
            .unwrap()
            .unwrap_or_else(|| panic!("{oid} is committed loose and must be readable"));
        assert_eq!(object.kind().as_str(), entry["type"].as_str().unwrap());
        assert_eq!(object.data().len() as u64, entry["size"].as_u64().unwrap());
        kinds.push(object.kind());
    }
    kinds.sort_unstable();
    let mut all = ObjectKind::ALL.to_vec();
    all.sort_unstable();
    assert_eq!(kinds, all, "all four object types must be covered");
    assert_eq!(store.counters().total(), 0);
    assert_eq!(store.limits().packs_loaded, 0, "there is no pack here");
}

/// The commit, tree and tag parsers, over objects Git actually wrote.
#[test]
fn the_committed_objects_parse_into_their_fields() {
    let store = ObjectStore::open(&fixture_root().join("loose")).unwrap();
    let inventory = inventory();
    let mut saw_commit = false;
    let mut saw_tree = false;
    let mut saw_tag = false;

    for entry in inventory["loose_objects"].as_array().unwrap() {
        let object = store.read(&oid_of(entry)).unwrap().unwrap();
        match object.kind() {
            ObjectKind::Commit => {
                let commit = parse_commit(object.data()).expect("a real commit parses");
                assert_eq!(commit.author.name, b"Nerve Fixture");
                assert_eq!(commit.author.email, b"fixture@nerve.invalid");
                assert_eq!(commit.author.timezone, "+0000");
                assert_eq!(commit.parents.len(), 1, "the fixture's HEAD has one parent");
                assert!(!commit.message.is_empty());
                saw_commit = true;
            }
            ObjectKind::Tree => {
                let entries = parse_tree(object.data()).expect("a real tree parses");
                let names: Vec<String> = entries
                    .iter()
                    .map(|entry| String::from_utf8_lossy(&entry.name).into_owned())
                    .collect();
                assert!(names.contains(&"README.md".to_string()), "{names:?}");
                assert!(names.contains(&"data.txt".to_string()), "{names:?}");
                assert!(names.contains(&"src".to_string()), "{names:?}");
                let src = entries.iter().find(|entry| entry.name == b"src").unwrap();
                assert!(src.is_tree());
                assert!(!src.is_gitlink());
                saw_tree = true;
            }
            ObjectKind::Tag => {
                // No tag parser exists in 12a, by decision: nothing consumes one yet. What is
                // asserted is that the object was read whole and is the tag Git wrote.
                let text = String::from_utf8_lossy(object.data());
                assert!(text.starts_with("object "), "{text}");
                assert!(text.contains("\ntype commit\n"), "{text}");
                assert!(text.contains("\ntag v1\n"), "{text}");
                saw_tag = true;
            }
            ObjectKind::Blob => {}
        }
    }
    assert!(saw_commit && saw_tree && saw_tag);
}

/// The committed tree's blobs must be reachable from the pack, which walks two object kinds and the
/// pack index in one step.
#[test]
fn the_committed_trees_resolve_to_blobs_in_the_pack() {
    let root = fixture_root();
    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    let inventory = inventory();
    let mut resolved = 0;
    for entry in inventory["packed_objects"].as_array().unwrap() {
        if entry["type"].as_str() != Some("tree") {
            continue;
        }
        let tree = store.read(&oid_of(entry)).unwrap().unwrap();
        for item in parse_tree(tree.data()).unwrap() {
            let child = store
                .read(&item.oid)
                .unwrap()
                .unwrap_or_else(|| panic!("{} is named by a tree and must be present", item.oid));
            if item.is_tree() {
                assert_eq!(child.kind(), ObjectKind::Tree);
            } else {
                assert_eq!(child.kind(), ObjectKind::Blob);
            }
            resolved += 1;
        }
    }
    assert!(resolved >= 5, "only {resolved} tree entries resolved");
}

/// `object-absent-from-the-store`.
#[test]
fn an_id_the_fixture_does_not_hold_is_ok_none() {
    let store = ObjectStore::open(&packed_dir(&fixture_root())).unwrap();
    let absent = Oid::from_hex(&"f".repeat(40)).unwrap();
    assert_eq!(store.read(&absent).unwrap(), None);
    assert!(!store.contains(&absent).unwrap());
    assert_eq!(
        store.counters().total(),
        0,
        "an absent object is not a refusal"
    );
    assert_eq!(store.limits().shallow, None);
    assert!(!store.limits().promisor);
}

// ---- what it must refuse ----------------------------------------------------------------------

/// `truncated-pack`.
#[test]
fn a_truncated_committed_pack_is_refused() {
    let (_dir, root) = fixture_copy();
    let path = pack_path(&root);
    let bytes = std::fs::read(&path).unwrap();
    // Cut inside the entry region: past the 12-byte header, well short of the end.
    overwrite(&path, &bytes[..bytes.len() / 2]);

    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    let inventory = inventory();
    let mut refused = 0;
    let mut read = 0;
    for entry in inventory["packed_objects"].as_array().unwrap() {
        match store.read(&oid_of(entry)) {
            Ok(Some(object)) => {
                // Anything that does come back must still be a whole object of the right length: a
                // truncated pack must never yield a partial one.
                assert_eq!(object.data().len() as u64, entry["size"].as_u64().unwrap());
                read += 1;
            }
            Ok(None) => {}
            Err(error) => {
                assert!(
                    error.form() == form::PACK_TRUNCATED
                        || error.form() == form::PACK_ENTRY_SIZE_DISAGREES
                        || error.form() == form::INFLATE_FAILED,
                    "unexpected refusal {}: {error}",
                    error.form()
                );
                assert_eq!(
                    error.form(),
                    form::PACK_TRUNCATED,
                    "a pack cut mid-entry must be reported as truncated"
                );
                refused += 1;
            }
        }
    }
    assert!(refused >= 1, "cutting the pack in half refused nothing");
    assert!(
        read >= 1,
        "cutting the pack in half broke everything, so the assertion above is weak"
    );
}

/// `corrupt-idx-bad-magic`. Not a v1 index, and must not be reported as one.
#[test]
fn a_committed_index_with_its_magic_overwritten_is_refused() {
    let (_dir, root) = fixture_copy();
    let path = idx_path(&root);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[..4].copy_from_slice(&[0xff; 4]);
    overwrite(&path, &bytes);

    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    assert_eq!(store.limits().packs_loaded, 0);
    assert_eq!(store.limits().packs_refused, 1);
    assert_eq!(store.counters().get(form::IDX_BAD_MAGIC), 1);
    assert!(
        store.limits().unsupported_index_versions.is_empty(),
        "a v2 index with its magic overwritten is not version 1, and saying so would be false"
    );
    // The store still opens, and simply has nothing.
    assert_eq!(
        store
            .read(&oid_of(&inventory()["packed_objects"][0]))
            .unwrap(),
        None
    );
}

/// `corrupt-idx-version-1`.
#[test]
fn a_version_one_index_is_refused_with_the_version_stated() {
    let (_dir, root) = fixture_copy();
    let pack_dir = packed_dir(&root).join("objects").join("pack");
    // A real, empty `.idx` v1: 256 zero fanout entries and the two 20-byte checksums, and no magic.
    let mut v1 = vec![0u8; 256 * 4];
    v1.extend_from_slice(&[0u8; 40]);
    std::fs::write(pack_dir.join("pack-legacy.idx"), &v1).unwrap();
    std::fs::copy(pack_path(&root), pack_dir.join("pack-legacy.pack")).unwrap();

    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    assert_eq!(store.limits().packs_loaded, 1, "the real pack still loads");
    assert_eq!(store.limits().packs_refused, 1);
    assert_eq!(store.counters().get(form::IDX_UNSUPPORTED_VERSION), 1);
    assert_eq!(store.limits().unsupported_index_versions, vec![1]);
}

/// `corrupt-idx-version-3`.
#[test]
fn a_version_three_index_is_refused_with_the_version_stated() {
    let (_dir, root) = fixture_copy();
    let path = idx_path(&root);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[4..8].copy_from_slice(&3u32.to_be_bytes());
    overwrite(&path, &bytes);

    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    assert_eq!(store.limits().packs_loaded, 0);
    assert_eq!(store.counters().get(form::IDX_UNSUPPORTED_VERSION), 1);
    assert_eq!(store.limits().unsupported_index_versions, vec![3]);
}

/// `corrupt-idx-fanout-not-monotonic`.
#[test]
fn a_committed_index_with_a_broken_fanout_is_refused() {
    let (_dir, root) = fixture_copy();
    let path = idx_path(&root);
    let mut bytes = std::fs::read(&path).unwrap();
    // Raise bucket 0 above bucket 255, which is the object count. Every later bucket is then lower
    // than its predecessor, so the search ranges are reversed.
    bytes[8..12].copy_from_slice(&0xffff_fff0u32.to_be_bytes());
    overwrite(&path, &bytes);

    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    assert_eq!(store.limits().packs_loaded, 0);
    assert_eq!(store.counters().get(form::IDX_FANOUT_NOT_MONOTONIC), 1);
}

/// `corrupt-idx-truncated`.
#[test]
fn a_truncated_committed_index_is_refused() {
    for fraction in [2usize, 4, 8] {
        let (_dir, root) = fixture_copy();
        let path = idx_path(&root);
        let bytes = std::fs::read(&path).unwrap();
        overwrite(&path, &bytes[..bytes.len() / fraction]);

        let store = ObjectStore::open(&packed_dir(&root)).unwrap();
        assert_eq!(store.limits().packs_loaded, 0, "cut to 1/{fraction}");
        assert_eq!(
            store.counters().get(form::IDX_TRUNCATED),
            1,
            "cutting the index to 1/{fraction} was not reported as truncated"
        );
    }

    // One trailing byte too many is equally refused: the length is the only way the size of the
    // 64-bit offset table is knowable, so it has to work out exactly.
    let (_dir, root) = fixture_copy();
    let path = idx_path(&root);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(0);
    overwrite(&path, &bytes);
    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    assert_eq!(store.counters().get(form::IDX_TRUNCATED), 1);
}

// ---- the dependency ----------------------------------------------------------------------------

/// **The `flate2` backend must be pure Rust, and a future edit must not be able to change that
/// quietly.**
///
/// Every backend `flate2` offers other than `rust_backend` is a C library with a build script.
/// `default-features = false` plus an explicit `rust_backend` is what keeps the C out, and this test
/// is what keeps the declaration honest — including by checking `Cargo.lock`, so that a C backend
/// arriving transitively is caught even if the feature list still reads correctly.
#[test]
fn the_inflate_backend_is_pure_rust() {
    let crate_manifest: toml::Value = toml::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap(),
    )
    .expect("crates/nerve-index/Cargo.toml must parse");
    let dependency = crate_manifest["dependencies"]
        .get("flate2")
        .expect("crates/nerve-index/Cargo.toml must declare flate2");
    assert_eq!(
        dependency.get("workspace").and_then(toml::Value::as_bool),
        Some(true),
        "flate2 is declared through the workspace, so the features live in the root manifest"
    );

    let workspace: toml::Value =
        toml::from_str(&std::fs::read_to_string(workspace_root().join("Cargo.toml")).unwrap())
            .expect("the workspace Cargo.toml must parse");
    let flate2 = &workspace["workspace"]["dependencies"]["flate2"];
    assert_eq!(
        flate2
            .get("default-features")
            .and_then(toml::Value::as_bool),
        Some(false),
        "flate2's default feature set selects a backend; it must be turned off explicitly"
    );
    let features: Vec<&str> = flate2["features"]
        .as_array()
        .expect("flate2 must select its backend explicitly")
        .iter()
        .map(|value| value.as_str().expect("a feature name"))
        .collect();
    assert_eq!(
        features,
        vec!["rust_backend"],
        "rust_backend is the only pure-Rust backend flate2 has"
    );

    // Nothing else may select a C backend, now or transitively.
    let lock = std::fs::read_to_string(workspace_root().join("Cargo.lock")).unwrap();
    for forbidden in [
        "libz-sys",
        "libz-ng-sys",
        "cloudflare-zlib-sys",
        "zlib-rs",
        "miniz-sys",
    ] {
        assert!(
            !lock.contains(&format!("name = \"{forbidden}\"")),
            "{forbidden} is in Cargo.lock, so a C or alternative zlib backend was pulled in"
        );
    }
    // And the pure-Rust one that must be there.
    assert!(lock.contains("name = \"miniz_oxide\""));
}

// ---- the constraints this slice is under -------------------------------------------------------

/// The fixture script runs `git`. **No Rust source may reference it.**
///
/// The needle is assembled from pieces so that this file does not contain the string it forbids;
/// otherwise the test would fail on itself, and the obvious repair — excluding this file from the
/// scan — is the kind of exception that later swallows a real one.
#[test]
fn no_rust_source_references_the_fixture_script() {
    let needle = ["make_", "gitobj_", "fixtures"].concat();
    let mut sources = Vec::new();
    collect_rs(&workspace_root().join("crates"), &mut sources);
    assert!(sources.len() > 40, "only {} sources scanned", sources.len());

    let mut offenders = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path).expect("a source file");
        if text.contains(&needle) {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "the fixture script is a development tool that runs `git`; product and test code must not \
         reach for it: {offenders:?}"
    );
}

/// Slice 12a creates no entity, no relation and no schema change. Asserted, not asserted-about.
///
/// **The literal moved from 5 to 6 in Slice 12b, from 6 to 7 in Slice 12c-ii, from 7 to 8 in
/// Slice 13a-i and from 8 to 9 in Slice 14a, and it is still a tripwire rather than a bookkeeping
/// value.** 12a was a reader with no schema change of its own, which is what this test was written
/// to pin. v6 belongs to 12b, v7 to 12c-ii, v8 to 13a-i and v9 to 14a, and all four live entirely in
/// `nerve-store`; the invariant that survives is that the *reader* is never what moves the version,
/// and what enforces that is the source scan below — no module under
/// `crates/nerve-index/src/gitobj` may name an entity kind, a relation, or the evidence tables.
/// Pinning the number keeps the next slice that touches the schema from doing it from inside the
/// reader without noticing.
#[test]
fn the_reader_touches_no_entity_kind_no_relation_and_no_schema_version() {
    assert_eq!(
        nerve_store::SCHEMA_VERSION,
        10,
        "the Git object reader must not be what migrates the schema"
    );

    let mut sources = Vec::new();
    collect_rs(
        &workspace_root().join("crates/nerve-index/src/gitobj"),
        &mut sources,
    );
    assert!(sources.len() >= 8, "only {} modules found", sources.len());
    for path in &sources {
        let text = std::fs::read_to_string(path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for forbidden in ["EntityKind", "Relation", "assertion", "observation"] {
                assert!(
                    !code.contains(forbidden),
                    "{}:{} mentions {forbidden}; 12a is a reader and writes nothing",
                    path.display(),
                    number + 1
                );
            }
        }
    }
}

/// Reading a repository must not write to it. A reader that left a file behind would be a reader
/// that had to be excluded from the read-only guarantees the rest of this crate is held to.
#[test]
fn reading_the_fixture_writes_nothing() {
    let (_dir, root) = fixture_copy();
    let before = tree_listing(&root);

    let store = ObjectStore::open(&packed_dir(&root)).unwrap();
    for entry in inventory()["packed_objects"].as_array().unwrap() {
        let _ = store.read(&oid_of(entry));
    }
    let loose = ObjectStore::open(&root.join("loose")).unwrap();
    for entry in inventory()["loose_objects"].as_array().unwrap() {
        let _ = loose.read(&oid_of(entry));
    }

    assert_eq!(before, tree_listing(&root), "reading changed the tree");
    assert!(!root.join(".nerve").exists());
}

/// Every case in `expected.json` must be asserted by a test that exists, and every test named here
/// must correspond to a case. A case nothing asserts is a claim in a fixture file and nothing more —
/// which is exactly the Slice 11a-i defect, in a different file.
#[test]
fn every_expected_case_is_asserted_by_a_named_test() {
    /// case name → the tests that assert it. Unit tests live in `crates/nerve-index/src/gitobj/`.
    const COVERAGE: [(&str, &[&str]); 26] = [
        (
            "real-pack-non-delta-entry",
            &["every_non_delta_entry_git_recorded_reconstructs"],
        ),
        (
            "real-pack-delta-entry",
            &["every_delta_entry_git_recorded_reconstructs"],
        ),
        (
            "loose-all-four-types",
            &[
                "the_four_committed_loose_objects_parse",
                "the_committed_objects_parse_into_their_fields",
            ],
        ),
        (
            "object-absent-from-the-store",
            &[
                "an_id_the_fixture_does_not_hold_is_ok_none",
                "a_loose_object_is_read_and_an_unknown_id_is_ok_none",
            ],
        ),
        (
            "truncated-pack",
            &[
                "a_truncated_committed_pack_is_refused",
                "a_pack_truncated_mid_entry_is_refused_with_no_partial_object",
            ],
        ),
        (
            "corrupt-idx-bad-magic",
            &[
                "a_committed_index_with_its_magic_overwritten_is_refused",
                "a_magic_less_file_that_is_not_a_v1_index_is_bad_magic_rather_than_version_one",
            ],
        ),
        (
            "corrupt-idx-version-1",
            &[
                "a_version_one_index_is_refused_with_the_version_stated",
                "a_real_version_one_index_is_reported_as_version_one",
            ],
        ),
        (
            "corrupt-idx-version-3",
            &[
                "a_version_three_index_is_refused_with_the_version_stated",
                "an_unsupported_version_is_refused_with_the_version_stated",
            ],
        ),
        (
            "corrupt-idx-fanout-not-monotonic",
            &[
                "a_committed_index_with_a_broken_fanout_is_refused",
                "a_non_monotonic_fanout_is_refused_rather_than_clamped",
            ],
        ),
        (
            "corrupt-idx-truncated",
            &[
                "a_truncated_committed_index_is_refused",
                "a_truncated_index_is_refused",
                "trailing_bytes_that_are_not_a_whole_number_of_large_offsets_are_refused",
            ],
        ),
        (
            "ref-delta-missing-base",
            &["a_ref_delta_naming_a_missing_base_is_ok_none_and_counted"],
        ),
        (
            "delta-chain-past-max-depth",
            &["a_delta_chain_past_the_depth_bound_is_refused"],
        ),
        (
            "delta-cycle",
            &[
                "a_delta_cycle_terminates_at_the_depth_bound",
                "an_ofs_delta_whose_base_offset_is_not_inside_the_pack_is_refused",
            ],
        ),
        (
            "decompression-bomb",
            &[
                "a_decompression_bomb_is_refused_during_inflate_with_bounded_peak_allocation",
                "a_bomb_is_refused_without_the_buffer_ever_exceeding_the_bound",
                "a_loose_bomb_is_refused_by_the_inflate_bound",
            ],
        ),
        (
            "loose-declared-size-disagrees",
            &["a_declared_size_that_disagrees_with_the_stream_is_refused_not_resolved"],
        ),
        (
            "loose-header-malformed",
            &[
                "a_header_with_no_nul_is_refused",
                "a_header_with_no_space_is_refused",
                "a_size_that_is_not_plain_decimal_is_refused",
                "an_unknown_type_word_is_refused_with_its_own_reason",
            ],
        ),
        (
            "pack-count-past-the-bound",
            &["packs_past_the_bound_are_refused_and_counted"],
        ),
        (
            "worktree-dot-git-is-a-file",
            &[
                "a_linked_worktree_reads_through_commondir",
                "a_malformed_commondir_is_counted_and_falls_back",
            ],
        ),
        (
            "alternates-one-entry",
            &[
                "one_alternate_inside_the_repository_is_followed",
                "a_relative_alternate_resolves_against_the_object_directory",
            ],
        ),
        (
            "alternates-chain",
            &[
                "an_alternates_chain_is_refused_at_the_second_hop_and_counted",
                "a_second_alternate_entry_is_refused_and_counted",
            ],
        ),
        (
            "alternates-outside-the-repository",
            &["an_alternate_outside_the_repository_root_is_refused_and_counted"],
        ),
        (
            "alternates-hostile-shape",
            &[
                "a_hostile_alternates_entry_is_refused_by_shape_and_counted",
                "a_comment_in_the_alternates_file_is_not_a_refusal",
            ],
        ),
        (
            "shallow-clone",
            &[
                "a_shallow_file_is_reported_as_a_boundary",
                "a_malformed_shallow_line_is_counted_and_the_rest_is_kept",
            ],
        ),
        (
            "no-shallow-file",
            &["no_shallow_file_means_not_shallow_rather_than_no_boundary"],
        ),
        (
            "promisor-partial-clone",
            &[
                "a_promisor_remote_or_a_promisor_pack_marker_is_reported",
                "extensions_partial_clone_is_reported",
            ],
        ),
        (
            "sha256-object-format",
            &[
                "a_sha256_repository_is_refused_with_the_format_named",
                "an_explicit_sha1_object_format_opens_normally",
            ],
        ),
    ];

    let expected = expected();
    let mut declared: Vec<String> = expected["cases"]
        .as_array()
        .expect("expected.json must list cases")
        .iter()
        .map(|case| case["case"].as_str().expect("a case name").to_string())
        .collect();
    declared.sort();
    let mut covered: Vec<String> = COVERAGE
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    covered.sort();
    assert_eq!(
        declared, covered,
        "expected.json and this test's coverage map must name the same cases"
    );

    // Every named test must exist, so the map cannot rot into a list of aspirations.
    let mut sources = Vec::new();
    collect_rs(
        &workspace_root().join("crates/nerve-index/src/gitobj"),
        &mut sources,
    );
    collect_rs(
        &workspace_root().join("crates/nerve-index/tests"),
        &mut sources,
    );
    let corpus: String = sources
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    for (case, tests) in COVERAGE {
        assert!(!tests.is_empty(), "{case} names no test");
        for test in tests {
            assert!(
                corpus.contains(&format!("fn {test}(")),
                "{case} names the test {test}, which does not exist"
            );
        }
    }

    // The bounds `expected.json` states must be the bounds the code uses.
    let bounds = &expected["bounds"];
    assert_eq!(
        bounds["MAX_OBJECT_BYTES"].as_u64().unwrap() as usize,
        nerve_index::gitobj::MAX_OBJECT_BYTES
    );
    assert_eq!(
        bounds["MAX_DELTA_DEPTH"].as_u64().unwrap() as usize,
        nerve_index::gitobj::MAX_DELTA_DEPTH
    );
    assert_eq!(
        bounds["MAX_PACK_COUNT"].as_u64().unwrap() as usize,
        nerve_index::gitobj::MAX_PACK_COUNT
    );
}

/// Opening something that is not a git directory is a refusal, not a panic and not an empty store.
#[test]
fn opening_a_non_git_directory_is_refused() {
    let root = fixture_root();
    match ObjectStore::open(&root.join("does-not-exist")).unwrap_err() {
        Error::NotADirectory(_) => {}
        other => panic!("expected NotADirectory, got {other:?}"),
    }
    // A directory with no `objects/` opens and simply has nothing, which is a different answer.
    let store = ObjectStore::open(&root).unwrap();
    assert_eq!(store.limits().packs_loaded, 0);
    assert_eq!(
        store
            .read(&Oid::from_hex(&"a".repeat(40)).unwrap())
            .unwrap(),
        None
    );
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

/// Every path under `root`, with its length, so a write of any kind shows up as a difference.
fn tree_listing(root: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                let length = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                out.push((relative, length));
            }
        }
    }
    out.sort();
    out
}
