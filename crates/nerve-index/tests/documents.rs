//! Document ingestion: structure, ADR recognition, resource bounds, and the T7 controls.
//!
//! The load-bearing test here is [`no_document_sourced_observation_escapes_document_stated`].
//! Everything else checks a mechanism; that one checks the property THREAT-MODEL.md §T7 gates
//! Slice 5 on, and it checks it by querying every row rather than by sampling.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::{copy_tree, count, named_fixture_root, open_db, TEST_PROJECT_ID};

use nerve_index::{AdrStatus, IndexOutcome};

// ---- harness -------------------------------------------------------------------------------

/// Control bytes `docs/hostile.md` names as escapes, and the bytes they name.
///
/// Substituted at materialization rather than committed raw: a C0 byte does not survive an
/// editor or a diff viewer, and a fixture nobody can read is not a fixture. That the
/// substitution happened is asserted, not assumed — see
/// [`the_hostile_document_really_does_contain_the_control_bytes_it_claims`].
const CONTROL_ESCAPES: [(&str, char); 3] =
    [("\\x1f", '\u{1f}'), ("\\x01", '\u{1}'), ("\\x0b", '\u{b}')];

fn materialize_control_bytes(root: &Path) {
    let path = root.join("docs/hostile.md");
    let mut text = std::fs::read_to_string(&path).unwrap();
    for (escape, byte) in CONTROL_ESCAPES {
        text = text.replace(escape, &byte.to_string());
    }
    std::fs::write(&path, text).unwrap();
}

/// Copy `md-docs` into a temporary root, materialize its control bytes, initialize and index.
fn indexed_documents() -> (tempfile::TempDir, PathBuf, IndexOutcome) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&named_fixture_root("md-docs"), &root);
    materialize_control_bytes(&root);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    (dir, root, outcome)
}

/// Initialize and index an arbitrary set of files written into a fresh temporary root.
fn indexed_tree(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, IndexOutcome) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    for (rel_path, contents) in files {
        let path = root.join(rel_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    (dir, root, outcome)
}

fn strings(conn: &nerve_store::Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn scalar(conn: &nerve_store::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Every `document` entity's `meta`, keyed by the path it occurs at.
fn document_meta(conn: &nerve_store::Connection) -> BTreeMap<String, serde_json::Value> {
    let mut stmt = conn
        .prepare(
            "SELECT o.file_path, e.meta FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'document'",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    rows.map(|row| {
        let (path, meta) = row.unwrap();
        (path, serde_json::from_str(&meta).unwrap())
    })
    .collect()
}

/// Section names in a document, in document order.
fn section_names(conn: &nerve_store::Connection, rel_path: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT e.name FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'section' AND o.file_path = ?1
              ORDER BY o.start_byte",
        )
        .unwrap();
    let rows = stmt
        .query_map([rel_path], |row| row.get::<_, String>(0))
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

// ---- T7: the separation invariant ----------------------------------------------------------

/// Every observation on a document path whose `(extractor_id, evidence_source_type)` pair is not
/// one of the two the amended T7 allows, named.
///
/// The allowed set is closed and keyed on **extractor id**, not on source type alone: an
/// `fs-structural` row saying `DOCUMENT_STATED` is as much a violation as an `md-structural` row
/// saying `AST_DIRECT`, because either one would mean an extractor emitted a label it does not
/// declare. Written once and used by both the invariant test and its mutation probe, so the probe
/// exercises the same query the gate does rather than a paraphrase of it.
fn t7_offenders(conn: &nerve_store::Connection) -> Vec<String> {
    strings(
        conn,
        "SELECT file_path || ' -> ' || evidence_source_type || ' (' || extractor_id || ')'
           FROM observation
          WHERE (file_path LIKE '%.md' OR file_path LIKE '%.markdown')
            AND (extractor_id, evidence_source_type) NOT IN (
                    VALUES ('md-structural', 'DOCUMENT_STATED'),
                           ('fs-structural', 'FILESYSTEM_OBSERVED'))
          ORDER BY 1",
    )
}

/// **THREAT-MODEL.md T7, the Slice 5 gate, as amended by Slice 5d-i.**
///
/// > Every observation whose `file_path` is a document carries `DOCUMENT_STATED`, except
/// > observations from `fs-structural`, which carries `FILESYSTEM_OBSERVED`. The allowed set is
/// > exactly those two, and it is checked exhaustively.
///
/// Stated as a property of the *file* rather than of each emission site, so that it is one query
/// over every row of `observation` rather than a rule each new call site is trusted to remember.
///
/// Until 5d-i the allowed set was a single value, held that way by having `md-structural` emit
/// `<directory> CONTAINS <document>` as `DOCUMENT_STATED`. That made the label depend on the file
/// extension: `docs/` containing `ROADMAP.md` was a document's claim while `src/` containing
/// `math.ts` was a syntax tree's, for the identical filesystem fact. 5d-i gave that claim to
/// `fs-structural`, and the invariant widened to exactly two values.
///
/// **Widening it is not weakening it**, and the reason is structural rather than a promise. T7
/// defends against *content* an attacker wrote inside a document; `fs-structural` cannot carry
/// document content anywhere, because it never reads any — see
/// `fs_structural_carries_no_file_content`, which proves it against a marker string. The one
/// attacker-influenced input it does touch is the path, which already passes the Slice 5a
/// `canonical_child` guard. The invariant stays total, exhaustively queryable and, per
/// `t7_names_every_offender_when_a_document_path_is_stamped_ast_direct`, mutation-verifiable.
#[test]
fn no_document_sourced_observation_escapes_document_stated() {
    let (_dir, root, _outcome) = indexed_documents();
    let conn = open_db(&root);

    let offenders = t7_offenders(&conn);
    assert!(
        offenders.is_empty(),
        "document-sourced observations escaped the allowed set: {offenders:?}"
    );

    // Both allowed pairs actually occur, so neither arm of the widened set is dead weight that
    // could be removed without any test noticing.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT extractor_id || '/' || evidence_source_type FROM observation
              WHERE file_path LIKE '%.md' OR file_path LIKE '%.markdown' ORDER BY 1"
        ),
        vec![
            "fs-structural/FILESYSTEM_OBSERVED".to_string(),
            "md-structural/DOCUMENT_STATED".to_string(),
        ]
    );

    // The query must have had something to prove. A tree with no document observations would
    // satisfy the assertion above vacuously.
    let document_observations = scalar(
        &conn,
        "SELECT count(*) FROM observation
          WHERE file_path LIKE '%.md' OR file_path LIKE '%.markdown'",
    );
    assert!(
        document_observations > 50,
        "only {document_observations} document observations — the invariant proved nothing"
    );

    // The converse: no source file may carry DOCUMENT_STATED either. The separation is total,
    // not one-directional.
    let leaked = strings(
        &conn,
        "SELECT DISTINCT file_path FROM observation
          WHERE evidence_source_type = 'DOCUMENT_STATED'
            AND file_path NOT LIKE '%.md'
            AND file_path NOT LIKE '%.markdown'",
    );
    assert!(
        leaked.is_empty(),
        "DOCUMENT_STATED evidence appeared on non-document paths: {leaked:?}"
    );

    // And the code side is still extracted the way it always was.
    assert!(
        scalar(
            &conn,
            "SELECT count(*) FROM observation
              WHERE file_path LIKE '%.ts' AND evidence_source_type = 'AST_DIRECT'"
        ) > 0,
        "the code half of the fixture produced no AST_DIRECT evidence"
    );

    // Every `DOCUMENT_STATED` observation is `md-structural`'s, and it emitted nothing else.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT extractor_id FROM observation
              WHERE evidence_source_type = 'DOCUMENT_STATED' ORDER BY 1"
        ),
        vec!["md-structural".to_string()]
    );
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT evidence_source_type || '/' || directness FROM observation
              WHERE extractor_id = 'md-structural' ORDER BY 1"
        ),
        vec!["DOCUMENT_STATED/DIRECT".to_string()]
    );

    // The same, both ways round, for the extractor the widened set admits.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT extractor_id FROM observation
              WHERE evidence_source_type = 'FILESYSTEM_OBSERVED' ORDER BY 1"
        ),
        vec!["fs-structural".to_string()]
    );
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT evidence_source_type || '/' || directness FROM observation
              WHERE extractor_id = 'fs-structural' ORDER BY 1"
        ),
        vec!["FILESYSTEM_OBSERVED/DIRECT".to_string()]
    );
}

/// The T7 query must still *fail* when the property is false, and must name every offender.
///
/// An exhaustive invariant that cannot fail is a comment. Slice 5d-i widened the allowed set, so
/// the probe is re-pointed at the widened query: stamping `AST_DIRECT` on a document path — the
/// exact promotion T7 exists to forbid — must be caught, and every stamped row must appear in the
/// message a maintainer would read.
#[test]
fn t7_names_every_offender_when_a_document_path_is_stamped_ast_direct() {
    let (_dir, root, _outcome) = indexed_documents();
    let conn = open_db(&root);
    assert!(t7_offenders(&conn).is_empty());

    // Promote three document observations to source-level evidence, exactly as a careless new
    // emission site would.
    let stamped = conn
        .execute(
            "UPDATE observation
                SET evidence_source_type = 'AST_DIRECT'
              WHERE rowid IN (
                    SELECT rowid FROM observation
                     WHERE file_path LIKE '%.md'
                       AND extractor_id = 'md-structural'
                     ORDER BY rowid LIMIT 3)",
            [],
        )
        .unwrap();
    assert_eq!(stamped, 3, "the mutation itself must have taken effect");

    let offenders = t7_offenders(&conn);
    assert_eq!(
        offenders.len(),
        3,
        "T7 named {} of 3 stamped rows: {offenders:?}",
        offenders.len()
    );
    for offender in &offenders {
        assert!(
            offender.contains("AST_DIRECT") && offender.contains("md-structural"),
            "an offender must say what escaped and who emitted it: {offender}"
        );
    }

    // The other direction: a filesystem row that claims to be a document's word is caught too,
    // because the allowed set is keyed on the pair and not on the source type alone.
    conn.execute(
        "UPDATE observation
            SET evidence_source_type = 'DOCUMENT_STATED'
          WHERE rowid IN (
                SELECT rowid FROM observation
                 WHERE file_path LIKE '%.md'
                   AND extractor_id = 'fs-structural'
                 ORDER BY rowid LIMIT 1)",
        [],
    )
    .unwrap();
    let offenders = t7_offenders(&conn);
    assert_eq!(offenders.len(), 4, "{offenders:?}");
}

/// **Schema v4, end to end, on a real database rather than a hand-built one.**
///
/// The `nerve-store` migration tests build their fixtures out of literal `INSERT`s, which is what
/// makes them precise — and also what makes them blind to anything the real indexer does that the
/// literals do not reproduce. Slice 3b learned that the expensive way: the v2 data-destruction bug
/// passed every unit-level migration test and was only ever caught by a test that ran the actual
/// indexer and compared the whole database afterwards. This is that form.
///
/// A real v3 database is produced by indexing the fixture with the current binary and then
/// putting the evidence labels back the way every build before Slice 5d-i wrote them — the `.md`
/// half as `md-structural` / `DOCUMENT_STATED`, the rest as `ts-js-structural` / `AST_DIRECT` —
/// re-deriving `assertion_state` from those labels and dropping the v4 marker. **Stated plainly
/// because it matters:** the tree is real and every row is the indexer's own, but the downgrade is
/// reconstructed rather than written by an old binary, which no longer exists to run. The schema
/// makes that sound: v4 changes no DDL, so a v3 and a v4 database differ in exactly the data this
/// reconstruction restores.
///
/// The assertion is the strongest available: after migrating, the canonical dump of the whole
/// database must equal the dump of a from-scratch index, byte for byte.
#[test]
fn a_real_v3_database_migrates_to_exactly_what_the_current_build_produces() {
    let (_dir, root, _outcome) = indexed_documents();
    let expected = {
        let conn = open_db(&root);
        nerve_store::canonical_dump(&conn).unwrap()
    };

    {
        let conn = open_db(&root);
        assert_eq!(
            nerve_store::schema_version(&conn).unwrap(),
            Some(nerve_store::SCHEMA_VERSION)
        );
        let rewound = conn
            .execute(
                "UPDATE observation
                    SET evidence_source_type =
                            CASE WHEN file_path LIKE '%.md' OR file_path LIKE '%.markdown'
                                 THEN 'DOCUMENT_STATED' ELSE 'AST_DIRECT' END,
                        extractor_id =
                            CASE WHEN file_path LIKE '%.md' OR file_path LIKE '%.markdown'
                                 THEN 'md-structural' ELSE 'ts-js-structural' END,
                        extractor_version = '1.1.0'
                  WHERE extractor_id = 'fs-structural'",
                [],
            )
            .unwrap();
        assert!(
            rewound > 10,
            "only {rewound} rows rewound — the fixture stopped exercising the migration"
        );
        // A v3 build derived its state from those labels, so the downgrade has to as well.
        nerve_store::rebuild_assertion_state(&conn).unwrap();
        // Rewind every marker above v3, and rewind v5's and v6's *shape* as well as their
        // markers: replaying `ALTER TABLE module_facts ADD COLUMN framework_version` against a
        // column that already exists is an error, and so is replaying `CREATE TABLE git_commit`
        // against a table that already exists. Both should be — a migration that tolerated
        // re-application would hide a real double-apply. So the downgrade has to be a real one.
        //
        // The v6, v7, v8 and v9 tables go in dependency order: `git_change`,
        // `git_rename_hypothesis` and v7's `git_rename_analysis` carry a foreign key onto
        // `git_commit`, v8's `contract_link` carries one onto `repo_registry`, and v9's
        // `memory_citation` and `memory_event` each carry one onto `memory`, so dropping any parent
        // first would leave a table referencing a table that is gone. v7's `summary_truncation`
        // column goes with `git_commit` itself, so there is no separate column to rewind.
        conn.execute("DELETE FROM schema_version WHERE version >= 4", [])
            .unwrap();
        conn.execute("ALTER TABLE module_facts DROP COLUMN framework_version", [])
            .unwrap();
        for table in [
            "git_change",
            "git_rename_hypothesis",
            "git_rename_analysis",
            "git_history_ingest",
            "git_commit",
            "contract_link",
            "repo_registry",
            "memory_citation",
            "memory_event",
            "memory",
        ] {
            conn.execute(&format!("DROP TABLE {table}"), []).unwrap();
        }
        assert_eq!(nerve_store::schema_version(&conn).unwrap(), Some(3));
        assert_ne!(
            nerve_store::canonical_dump(&conn).unwrap(),
            expected,
            "the downgrade changed nothing; the migration would prove nothing"
        );
    }

    let conn = open_db(&root);
    nerve_store::migrate(&conn).unwrap();
    assert_eq!(
        nerve_store::schema_version(&conn).unwrap(),
        Some(nerve_store::SCHEMA_VERSION)
    );
    assert_eq!(
        nerve_store::canonical_dump(&conn).unwrap(),
        expected,
        "a migrated v3 database is not what the current build produces from scratch"
    );

    // And T7 holds on the migrated database, not only on a freshly indexed one.
    assert!(t7_offenders(&conn).is_empty());
}

/// `fs-structural` never reads file bytes, proved against real files rather than by inspection.
///
/// This is the load-bearing premise of the amended T7 (`docs/plans/slice-05d-...` §4): the
/// filesystem extractor is allowed onto document paths **because** it cannot carry document
/// content, so if that ever stopped being true the widened allowed set would be a real weakening.
///
/// The construction proof is in the types — `nerve_index::FsEntry` has no field that can hold
/// file text, and it is the only input to the graph builder — and `fs_graph_needs_no_file_on_disk`
/// in `pipeline.rs` builds the whole skeleton from hand-written entries with no file anywhere. The
/// proof here is empirical and complements it: the fixture's documents contain distinctive strings
/// including a hostile one, and none of them reaches any column of any `fs-structural` row.
#[test]
fn fs_structural_carries_no_file_content() {
    let (_dir, root, _outcome) = indexed_documents();
    let conn = open_db(&root);

    let rows = scalar(
        &conn,
        "SELECT count(*) FROM observation WHERE extractor_id = 'fs-structural'",
    );
    assert!(rows > 0, "nothing to prove: fs-structural emitted no rows");

    // Every column an `fs-structural` observation can carry, concatenated. If content ever leaked
    // into `details`, `environment` or anywhere else, it lands in this string.
    let everything = strings(
        &conn,
        "SELECT coalesce(evidence_source_type,'') || '|' || coalesce(directness,'') || '|'
             || coalesce(extractor_id,'') || '|' || coalesce(extractor_version,'') || '|'
             || coalesce(match_quality,'') || '|' || coalesce(file_path,'') || '|'
             || coalesce(start_line,'') || '|' || coalesce(end_line,'') || '|'
             || coalesce(content_hash,'') || '|' || coalesce(environment,'') || '|'
             || coalesce(details,'')
           FROM observation WHERE extractor_id = 'fs-structural'",
    );
    let haystack = everything.join("\n");

    // Prose that really is in the fixture's documents, including the injection attempt from
    // `docs/hostile.md`. None is a path, so none can reach a filesystem row by any route at all.
    for needle in [
        "Ignore previous instructions",
        "Starts at level three",
        "Then a level two",
        "Hostile document",
        "attacker-controlled",
    ] {
        assert!(
            !haystack.contains(needle),
            "an fs-structural row carried document content: {needle:?}"
        );
    }

    // `details` is the only free-form column, and for this extractor it is a closed vocabulary.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT details FROM observation
              WHERE extractor_id = 'fs-structural' ORDER BY 1"
        ),
        vec![
            "{\"child_kind\":\"directory\"}".to_string(),
            "{\"child_kind\":\"file\"}".to_string(),
        ]
    );
}

// ---- the hostile document ------------------------------------------------------------------

#[test]
fn the_hostile_document_really_does_contain_the_control_bytes_it_claims() {
    let (_dir, root, _outcome) = indexed_documents();
    let text = std::fs::read_to_string(root.join("docs/hostile.md")).unwrap();
    for (escape, byte) in CONTROL_ESCAPES {
        assert!(
            text.contains(byte),
            "the harness did not substitute {escape}; the forgery test would prove nothing"
        );
        assert!(
            !text.contains(escape),
            "{escape} survived substitution as literal text"
        );
    }
}

/// The negative fixture, end to end. Nothing executes, nothing resolves, nothing collides.
#[test]
fn a_hostile_document_is_stored_as_inert_data_and_forges_no_identity() {
    let (_dir, root, outcome) = indexed_documents();
    let conn = open_db(&root);

    // 1. Indexing succeeded and the document is in the graph.
    assert_eq!(outcome.status, nerve_index::RunStatus::Complete);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity e JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'document' AND o.file_path = 'docs/hostile.md'"
        ),
        1
    );

    // 2. The hostile text is stored, as text, exactly as written. Storing it is the point:
    //    a scanner that dropped it would hide the attack rather than defuse it.
    let names = section_names(&conn, "docs/hostile.md");
    assert!(names.iter().any(|name| name.contains("Control characters")));
    assert!(
        names.iter().any(|name| name.contains('\u{1f}')),
        "the unit-separator heading was not stored: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains('\u{1}'))
            && names.iter().any(|name| name.contains('\u{b}')),
        "the NUL-adjacent control characters were not stored: {names:?}"
    );

    // 3. Every section of the hostile document has a distinct identity. Two of its headings are
    //    built to forge another section's tuple — one through `0x1f`, one through `>`.
    let ids = strings(
        &conn,
        "SELECT e.entity_id FROM entity e JOIN occurrence o ON o.entity_id = e.entity_id
          WHERE e.kind = 'section' AND o.file_path = 'docs/hostile.md'",
    );
    let unique: BTreeSet<&String> = ids.iter().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "two sections of the hostile document share an id"
    );

    // The specific pair the `0x1f` heading targets: a top-level `Parent<0x1f>Child`, and `Child`
    // nested under `Parent`. Unstripped, both encode to the same tuple bytes.
    let forged = nerve_core::ids::section_id(
        TEST_PROJECT_ID,
        "docs/hostile.md",
        &["Parent\u{1f}Child"],
        0,
    );
    let honest =
        nerve_core::ids::section_id(TEST_PROJECT_ID, "docs/hostile.md", &["Parent", "Child"], 0);
    assert_ne!(forged, honest);
    assert!(ids.contains(&forged) && ids.contains(&honest));

    // The `>` pair: `A>B / C` against `A / B / C`.
    assert!(ids.contains(&nerve_core::ids::section_id(
        TEST_PROJECT_ID,
        "docs/hostile.md",
        &["A>B", "C"],
        0
    )));
    assert!(ids.contains(&nerve_core::ids::section_id(
        TEST_PROJECT_ID,
        "docs/hostile.md",
        &["A", "B", "C"],
        0
    )));

    // 4. Nothing was executed, nothing was fetched and nothing escaped. `no_subprocess.rs` and
    //    `no_network.rs` prove the process-level half; here, the graph itself must show that
    //    every hostile link landed in the one place a hostile link may land — an `Unresolved`
    //    entity with a reason — and that no path outside the repository reached a row.
    //
    //    Slice 5c resolves links, so "a document may only CONTAIN" is no longer the property.
    //    The property is that a document may only CONTAIN and REFERENCE, and that **none** of
    //    this document's references resolves to anything.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT a.relation FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
              WHERE s.kind IN ('document', 'section') ORDER BY 1"
        ),
        vec!["CONTAINS".to_string(), "REFERENCES".to_string()]
    );
    let hostile_links = strings(
        &conn,
        "SELECT t.kind || ' ' || t.name FROM assertion a
           JOIN entity s ON s.entity_id = a.source_entity_id
           JOIN entity t ON t.entity_id = a.target_entity_id
           JOIN observation o ON o.assertion_id = a.assertion_id
          WHERE a.relation = 'REFERENCES' AND o.file_path = 'docs/hostile.md'
          ORDER BY 1",
    );
    assert_eq!(
        hostile_links,
        vec![
            // The traversal destination: refused before anything reached the filesystem.
            "unresolved ../../../etc/passwd".to_string(),
            // `[../../../etc/passwd](./real.md)` — the *destination* is what is read, and it
            // names no indexed file. The traversal-shaped link **text** produced nothing, which
            // is the point of writing it into the fixture that way.
            "unresolved ./real.md".to_string(),
        ],
        "a hostile link resolved to something, or the link text was read as a destination"
    );
    // `[click](javascript:alert(1))` is external. It is counted and never fetched, and it is not
    // an `Unresolved` entity: nothing failed. Nothing in the graph may name it.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE name LIKE 'javascript:%'"
        ),
        0,
        "a javascript: destination became an entity"
    );
    assert_eq!(
        document_meta(&conn)["docs/hostile.md"]["links"]["document_link_external"],
        1,
        "the external destination was not counted"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM observation WHERE file_path LIKE '%etc/passwd%'
                 OR file_path LIKE '/%' OR file_path LIKE '%..%'"
        ),
        0,
        "a traversal-shaped link text reached a file_path"
    );
    // `SUPERSEDES` is declared in 5a and emitted by nothing.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion WHERE relation = 'SUPERSEDES'"
        ),
        0
    );

    // 5. The scanner saw the raw HTML for what it was and counted it rather than descending.
    let meta = document_meta(&conn);
    let hostile = &meta["docs/hostile.md"];
    assert!(
        hostile["unsupported"]["html-block"].as_u64().unwrap() >= 3,
        "the script tag, the event handler and the inline HTML were not counted: {hostile}"
    );
    assert_eq!(hostile["adr"], false);
    assert_eq!(hostile["status"], serde_json::Value::Null);
}

// ---- structure -----------------------------------------------------------------------------

#[test]
fn documents_and_sections_are_entities_with_spans_and_content_hashes() {
    let (_dir, root, outcome) = indexed_documents();
    let conn = open_db(&root);

    assert_eq!(outcome.documents_processed, 8);
    assert!(outcome.document_sections > 20);
    assert_eq!(
        *outcome.entities_by_kind.get("document").unwrap(),
        8,
        "one document entity per discovered document"
    );

    // Every document and section has an occurrence with a real span and the file's content hash.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity e
              WHERE e.kind IN ('document','section')
                AND NOT EXISTS (SELECT 1 FROM occurrence o WHERE o.entity_id = e.entity_id)"
        ),
        0,
        "a document or section entity has no occurrence"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM occurrence o
               JOIN entity e ON e.entity_id = o.entity_id
              WHERE e.kind = 'section' AND (o.end_byte <= o.start_byte OR o.content_hash = '')"
        ),
        0,
        "a section has an empty or inverted span"
    );

    // `nerve search` finds a section by its heading text (acceptance criterion 2).
    let hits = nerve_store::search_entities(&conn, "Decision", None, 20).unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "section" && hit.name == "Decision"),
        "searching for a heading found no section: {hits:?}"
    );
}

#[test]
fn section_nesting_follows_heading_levels() {
    let (_dir, root, _outcome) = indexed_documents();
    let conn = open_db(&root);

    // A document whose first heading is level 3 still hangs it off the document.
    let parent_kind = |name: &str| -> String {
        strings(
            &conn,
            &format!(
                "SELECT s.kind FROM assertion a
                   JOIN entity s ON s.entity_id = a.source_entity_id
                   JOIN entity t ON t.entity_id = a.target_entity_id
                   JOIN occurrence o ON o.entity_id = t.entity_id
                  WHERE a.relation = 'CONTAINS' AND t.kind = 'section'
                    AND t.name = '{name}' AND o.file_path = 'docs/architecture.md'"
            ),
        )
        .remove(0)
    };
    assert_eq!(parent_kind("Starts at level three"), "document");
    assert_eq!(parent_kind("A child of a level-three parent"), "section");
    assert_eq!(parent_kind("Then a level two"), "document");

    // Setext headings nest exactly as ATX ones do.
    let names = section_names(&conn, "README.md");
    assert!(names.contains(&"Setext level one".to_string()));
    assert!(names.contains(&"Setext level two".to_string()));
    assert!(
        !names.iter().any(|name| name.contains("not a heading")),
        "text inside a fence or an indented block became a section: {names:?}"
    );

    // Two sibling sections with identical text are two sections.
    assert_eq!(
        names.iter().filter(|name| *name == "Repeated").count(),
        2,
        "identical sibling headings collapsed onto one section"
    );

    // Every containment edge into a section comes from a document or another section.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE t.kind = 'section' AND s.kind NOT IN ('document','section')"
        ),
        0
    );
}

/// Documents must not disturb what the TS/JS extractors do with the same tree.
#[test]
fn code_extraction_is_unchanged_by_the_presence_of_documents() {
    let (_dir, root, outcome) = indexed_documents();
    let conn = open_db(&root);

    assert_eq!(
        outcome.files_processed, 10,
        "8 documents and 2 source files"
    );
    assert_eq!(*outcome.entities_by_kind.get("module").unwrap(), 2);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion a
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE a.relation = 'IMPORTS' AND t.kind = 'module'"
        ),
        1,
        "app.ts imports util.ts and that is the only module import"
    );
    // No specifier may ever resolve to a document: a `Module` entity for a `.md` path would be
    // a dangling target, because no extractor creates one.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'module' AND (o.file_path LIKE '%.md' OR o.file_path LIKE '%.markdown')"
        ),
        0
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion a
              WHERE NOT EXISTS (SELECT 1 FROM entity e WHERE e.entity_id = a.target_entity_id)
                 OR NOT EXISTS (SELECT 1 FROM entity e WHERE e.entity_id = a.source_entity_id)"
        ),
        0,
        "an assertion names an entity that does not exist"
    );
}

// ---- ADR recognition -----------------------------------------------------------------------

#[test]
fn adr_recognition_and_status_on_the_fixture() {
    let (_dir, root, outcome) = indexed_documents();
    let conn = open_db(&root);
    let meta = document_meta(&conn);

    let header = &meta["docs/decisions/ADR-0001-header-status.md"];
    assert_eq!(header["adr"], true);
    assert_eq!(header["adr_id"], "ADR-0001");
    assert_eq!(header["status"], "Accepted");

    let section = &meta["docs/decisions/ADR-0002-status-section.md"];
    assert_eq!(section["adr"], true);
    assert_eq!(section["status"], "Superseded");

    let unparsed = &meta["docs/decisions/ADR-0003-unparsed-status.md"];
    assert_eq!(unparsed["adr"], true);
    assert_eq!(
        unparsed["status"], "unparsed",
        "an unrecognised status must never be coerced into the vocabulary"
    );

    let plain = &meta["docs/decisions/plain-note.md"];
    assert_eq!(plain["adr"], true, "a file in `decisions/` is an ADR");
    assert_eq!(plain["adr_id"], serde_json::Value::Null);
    assert_eq!(
        plain["status"],
        serde_json::Value::Null,
        "no status is not the same as an unreadable one"
    );

    assert_eq!(meta["README.md"]["adr"], false);
    assert_eq!(outcome.adr_documents, 4);

    // The raw text of an unparsed status is preserved on the observation, with its citation.
    let details: serde_json::Value = serde_json::from_str(
        &strings(
            &conn,
            "SELECT details FROM observation
              WHERE file_path = 'docs/decisions/ADR-0003-unparsed-status.md'
                AND details LIKE '%status_raw%' AND details LIKE '%child_kind\":\"document%'",
        )
        .remove(0),
    )
    .unwrap();
    assert_eq!(details["status_raw"], "Mostly agreed, pending review");
    assert_eq!(details["status_line"], 3);
    assert_eq!(details["status_form"], "header-line");
}

/// **Acceptance criterion 4.** Verified against the real files, not against a paraphrase of
/// them: `docs/decisions/` is this repository's own, and its header format is
/// `**Status:** Accepted · **Date:** …`.
#[test]
fn nerves_own_decision_records_are_recognised() {
    let decisions = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/decisions")
        .canonicalize()
        .expect("docs/decisions must exist");

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&decisions, &root.join("docs/decisions"));
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    let conn = open_db(&root);
    let meta = document_meta(&conn);

    assert!(
        outcome.documents_processed >= 6,
        "expected this repository's ADRs, found {}",
        outcome.documents_processed
    );
    assert_eq!(outcome.adr_documents, outcome.documents_processed);

    for (path, document) in &meta {
        assert_eq!(document["adr"], true, "{path} is in docs/decisions");
        let status = document["status"]
            .as_str()
            .unwrap_or_else(|| panic!("{path} records no status"));
        assert!(
            AdrStatus::parse(status).is_some() || status == "unparsed",
            "{path} recorded neither a vocabulary status nor `unparsed`: {status}"
        );
    }

    // ADR-0002 writes `**Status:** Accepted, with documented known defects`. That is not the
    // word `Accepted`, and reporting it as `Accepted` would drop the qualification the author
    // wrote deliberately. It is `unparsed`, with the raw text preserved for a human to read.
    let qualified = meta
        .iter()
        .find(|(path, _)| path.contains("ADR-0002"))
        .map(|(_, document)| document)
        .expect("ADR-0002 must be present");
    assert_eq!(
        qualified["status"], "unparsed",
        "a qualified status must not be coerced to the nearest vocabulary member"
    );

    let adr_0006 = meta
        .iter()
        .find(|(path, _)| path.contains("ADR-0006"))
        .map(|(_, document)| document)
        .expect("ADR-0006 must be present");
    assert_eq!(adr_0006["adr_id"], "ADR-0006");
    assert_eq!(adr_0006["status"], "Accepted");

    let path = meta
        .keys()
        .find(|path| path.contains("ADR-0006"))
        .unwrap()
        .clone();
    assert!(
        section_names(&conn, &path).contains(&"Decision".to_string()),
        "ADR-0006's `## Decision` heading is not a Section entity"
    );
}

// ---- resource bounds -----------------------------------------------------------------------

/// Every bound refuses and counts. Nothing is truncated silently, and every counter reaches the
/// index report, which is where `nerve index --json` and the human output read it from.
#[test]
fn every_resource_bound_fires_and_is_reported() {
    let mut too_many_headings = String::new();
    for heading in 0..nerve_index::markdown::MAX_HEADINGS_PER_DOCUMENT + 3 {
        too_many_headings.push_str(&format!("# H{heading}\n"));
    }

    let mut long_front_matter = String::from("---\n");
    for key in 0..nerve_index::markdown::MAX_FRONT_MATTER_LINES + 2 {
        long_front_matter.push_str(&format!("k{key}: v\n"));
    }
    long_front_matter.push_str("---\n\n# After\n");

    let (_dir, _root, outcome) = indexed_tree(&[
        ("docs/headings.md", &too_many_headings),
        ("docs/frontmatter.md", &long_front_matter),
        ("docs/fence.md", "# Real\n\n```\nnever closed\n"),
        ("docs/openmatter.md", "---\nkey: value\n\n# Real\n"),
        ("docs/deep.md", "####### seven hashes\n"),
        ("docs/nested.md", "> # quoted\n\n- # listed\n"),
    ]);

    let by_form = &outcome.unsupported_markdown_by_form;
    use nerve_index::markdown::form;
    assert_eq!(by_form[form::HEADINGS_EXCEEDED], 3);
    assert_eq!(by_form[form::FRONT_MATTER_TOO_LONG], 1);
    assert_eq!(by_form[form::UNTERMINATED_FENCE], 1);
    assert_eq!(by_form[form::UNTERMINATED_FRONT_MATTER], 1);
    assert_eq!(by_form[form::ATX_OVER_MAX_LEVEL], 1);
    assert_eq!(by_form[form::HEADING_IN_BLOCK_QUOTE], 1);
    assert_eq!(by_form[form::HEADING_IN_LIST_ITEM], 1);
    assert_eq!(
        outcome.unsupported_markdown,
        by_form.values().sum::<usize>()
    );

    // The heading bound refused the excess rather than accepting it, and the document kept
    // exactly its allowance.
    assert_eq!(
        outcome.document_sections,
        nerve_index::markdown::MAX_HEADINGS_PER_DOCUMENT + 3,
        "headings.md keeps its allowance; frontmatter, fence and openmatter contribute one each"
    );
}

/// A document past `index.max_file_bytes` is refused at the read boundary and counted against
/// `md-structural`'s own run, not against a TypeScript run that never saw it.
#[test]
fn an_oversized_document_is_refused_and_attributed_to_its_own_extractor() {
    let huge = "# Heading\n".repeat(300_000);
    assert!(huge.len() as u64 > nerve_index::config::DEFAULT_MAX_FILE_BYTES);
    let (_dir, root, outcome) = indexed_tree(&[
        ("docs/huge.md", &huge),
        ("docs/small.md", "# Small\n"),
        ("src/a.ts", "export const a = 1;\n"),
    ]);

    assert_eq!(outcome.files_failed, 1);
    assert_eq!(outcome.status, nerve_index::RunStatus::Partial);
    assert_eq!(outcome.documents_processed, 1);

    let conn = open_db(&root);
    let report = nerve_store::status(&conn).unwrap();
    let run = report
        .runs
        .iter()
        .find(|run| run.extractor_id == "md-structural")
        .unwrap();
    assert_eq!(run.files_processed, 1);
    assert_eq!(run.files_failed, 1);
    let structural = report
        .runs
        .iter()
        .find(|run| run.extractor_id == "ts-js-structural")
        .unwrap();
    assert_eq!(structural.files_failed, 0, "the .ts file read fine");
}

// ---- malformed input -----------------------------------------------------------------------

/// Acceptance criterion 8. None of these may fail an index, and none may panic.
#[test]
fn malformed_documents_never_fail_an_index() {
    let (_dir, root, outcome) = indexed_tree(&[
        ("docs/empty.md", ""),
        ("docs/no-headings.md", "just prose\nand more prose\n"),
        ("docs/lone-hash.md", "#\n"),
        ("docs/crlf.md", "# One\r\n## Two\r\n"),
        ("docs/mixed.md", "# One\r\n## Two\n### Three\r\n"),
        ("docs/fence.markdown", "```\n# hidden\n"),
        ("docs/matter.md", "---\nnope\n"),
        ("docs/blank-setext.md", "Title\n\n-----\n"),
        ("docs/third-level.md", "### Third\n"),
    ]);

    assert_eq!(outcome.status, nerve_index::RunStatus::Complete);
    assert_eq!(outcome.files_failed, 0);
    assert_eq!(outcome.documents_processed, 9);

    let conn = open_db(&root);
    assert!(section_names(&conn, "docs/empty.md").is_empty());
    assert!(section_names(&conn, "docs/no-headings.md").is_empty());
    assert_eq!(section_names(&conn, "docs/lone-hash.md"), vec![""]);
    assert_eq!(
        section_names(&conn, "docs/crlf.md"),
        vec!["One".to_string(), "Two".to_string()]
    );
    assert_eq!(
        section_names(&conn, "docs/mixed.md"),
        vec!["One".to_string(), "Two".to_string(), "Three".to_string()]
    );
    assert!(section_names(&conn, "docs/blank-setext.md").is_empty());
    assert_eq!(section_names(&conn, "docs/third-level.md"), vec!["Third"]);
    assert!(count(&conn, "assertion_state") > 0);
}

// ---- incremental ---------------------------------------------------------------------------

/// A document has no imports, so in Slice 5a it invalidates only itself.
#[test]
fn editing_a_document_re_extracts_only_that_document() {
    let (_dir, root, _outcome) = indexed_documents();

    std::fs::write(
        root.join("docs/architecture.md"),
        "# Rewritten\n\n## Child\n",
    )
    .unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.incremental.files_re_extracted, 1);
    assert_eq!(outcome.incremental.amplification(), Some(1.0));

    let conn = open_db(&root);
    assert_eq!(
        section_names(&conn, "docs/architecture.md"),
        vec!["Rewritten".to_string(), "Child".to_string()]
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity e JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'section' AND o.file_path = 'docs/architecture.md'
                AND e.name = 'Starts at level three'"
        ),
        0,
        "the superseded sections survived the edit"
    );
}

/// Adding or removing a document must not re-resolve a single module: `resolve` never sees a
/// document path, so a README cannot change what a specifier means.
#[test]
fn adding_a_document_does_not_disturb_module_resolution() {
    let (_dir, root, _outcome) = indexed_documents();

    std::fs::write(root.join("docs/new.md"), "# New\n").unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.incremental.files_added, 1);
    assert_eq!(
        outcome.incremental.files_resolution_changed, 0,
        "adding a document re-resolved a module's specifiers"
    );
    assert_eq!(outcome.incremental.files_re_extracted, 1);

    std::fs::remove_file(root.join("docs/new.md")).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.incremental.files_removed, 1);
    assert_eq!(outcome.incremental.files_resolution_changed, 0);
}

/// Re-indexing an unchanged tree with documents in it must write nothing at all.
#[test]
fn an_unchanged_tree_with_documents_re_extracts_nothing() {
    let (_dir, root, _outcome) = indexed_documents();
    let before = {
        let conn = open_db(&root);
        (
            count(&conn, "entity"),
            count(&conn, "observation"),
            count(&conn, "module_facts"),
        )
    };
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.incremental.files_re_extracted, 0);
    assert_eq!(outcome.incremental.rows_written, 0);

    let conn = open_db(&root);
    assert_eq!(
        (
            count(&conn, "entity"),
            count(&conn, "observation"),
            count(&conn, "module_facts")
        ),
        before
    );
}

// ---- links (Slice 5c) ------------------------------------------------------------------------

/// A tree with one module, one document that links into it, and nothing else.
fn linked_tree() -> (tempfile::TempDir, PathBuf) {
    let (dir, root, _) = indexed_tree(&[
        (
            "src/util.ts",
            "export function describe(): string {\n  return 'x';\n}\n",
        ),
        ("src/other.ts", "export const other = 1;\n"),
        (
            "docs/guide.md",
            "# Guide\n\nAn anchored link to [describe](../src/util.ts#L2), and a plain one to \
             [other](../src/other.ts).\n",
        ),
    ]);
    (dir, root)
}

/// The two edges an anchored link produces, and where each of them points.
#[test]
fn an_anchored_link_references_the_file_and_the_innermost_symbol() {
    let (_dir, root) = linked_tree();
    let conn = open_db(&root);

    let edges = strings(
        &conn,
        "SELECT t.kind || ' ' || t.name || ' [' || o.directness || ']'
           FROM assertion a
           JOIN entity t ON t.entity_id = a.target_entity_id
           JOIN observation o ON o.assertion_id = a.assertion_id
          WHERE a.relation = 'REFERENCES' AND o.file_path = 'docs/guide.md'
          ORDER BY 1",
    );
    assert_eq!(
        edges,
        vec![
            "file other.ts [RESOLVED]".to_string(),
            "file util.ts [RESOLVED]".to_string(),
            "function describe [RESOLVED]".to_string(),
        ]
    );

    // The target file's hash at resolution time, on the symbol edge and on nothing else.
    let details: serde_json::Value = serde_json::from_str(
        &strings(
            &conn,
            "SELECT details FROM observation
              WHERE file_path = 'docs/guide.md' AND details LIKE '%\"link_target\":\"symbol\"%'",
        )
        .remove(0),
    )
    .unwrap();
    let expected = nerve_core::ids::content_hash(&std::fs::read(root.join("src/util.ts")).unwrap());
    assert_eq!(details["target_content_hash"], expected);
    assert_eq!(details["resolved_path"], "src/util.ts");
    assert_eq!(details["anchor"]["start_line"], 2);
    assert_eq!(details["form"], "inline");
    assert_eq!(details["source_kind"], "section");
}

/// **The invalidation rule that makes the anchor honest.** Editing a file a document anchors
/// into must re-extract that document, because its edge records the hash it resolved against.
/// Editing a file the document merely links to must not: that edge depends on the file
/// existing, and nothing else.
#[test]
fn editing_an_anchor_target_re_extracts_the_document_and_editing_a_plain_target_does_not() {
    let (_dir, root) = linked_tree();

    std::fs::write(
        root.join("src/util.ts"),
        "export function describe(): string {\n  return 'edited';\n}\n",
    )
    .unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(
        outcome.incremental.files_resolution_changed, 1,
        "an anchored link survived an edit to the file it points into"
    );
    assert!(
        outcome.incremental.files_re_extracted >= 2,
        "the document and its anchor target must both be re-extracted, saw {}",
        outcome.incremental.files_re_extracted
    );

    let conn = open_db(&root);
    let details: serde_json::Value = serde_json::from_str(
        &strings(
            &conn,
            "SELECT details FROM observation
              WHERE file_path = 'docs/guide.md' AND details LIKE '%\"link_target\":\"symbol\"%'",
        )
        .remove(0),
    )
    .unwrap();
    assert_eq!(
        details["target_content_hash"],
        nerve_core::ids::content_hash(&std::fs::read(root.join("src/util.ts")).unwrap()),
        "the recorded hash still names the superseded bytes"
    );

    // The unanchored target now changes. Nothing about the document's claim moved.
    std::fs::write(root.join("src/other.ts"), "export const other = 2;\n").unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(
        outcome.incremental.files_resolution_changed, 0,
        "an unanchored link made a document depend on a file's contents"
    );
    assert_eq!(outcome.incremental.files_re_extracted, 1);
}

/// Deleting a link target turns the edge unresolved rather than deleting it, and restoring the
/// target turns it back. Unresolved is a value, not an omission.
#[test]
fn deleting_a_link_target_breaks_the_link_rather_than_removing_it() {
    let (_dir, root) = linked_tree();
    let broken = |root: &Path| -> Vec<String> {
        let conn = open_db(root);
        strings(
            &conn,
            "SELECT e.name || ' ' || json_extract(e.meta, '$.reason') FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'unresolved' AND o.file_path = 'docs/guide.md' ORDER BY 1",
        )
    };
    assert!(broken(&root).is_empty());

    std::fs::remove_file(root.join("src/other.ts")).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.incremental.files_resolution_changed, 1);
    assert_eq!(
        broken(&root),
        vec!["../src/other.ts document_link_target_not_indexed".to_string()]
    );

    std::fs::write(root.join("src/other.ts"), "export const other = 1;\n").unwrap();
    nerve_index::index_repository(&root).unwrap();
    assert!(
        broken(&root).is_empty(),
        "the broken-link entity outlived the file coming back"
    );
}
