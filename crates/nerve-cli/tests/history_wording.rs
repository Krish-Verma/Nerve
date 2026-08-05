//! The history notes exist in **exactly one place each**, enforced rather than asserted in a
//! comment.
//!
//! Slice 12b left four wording functions and one interpretation predicate inside the CLI binary
//! (`main.rs:2151`, `:2180`, `:2210`, `:2230`, `:2255`). Slice 12c-i moved the four notes to
//! `nerve-core`, beside the vocabularies they render, and the judgment to `nerve-store`, beside
//! `IngestRow`. Slice 12c-iii-a moved the four it could not — `FirstObservedKind::note`,
//! `FirstObservedKind::created_claim_note`, `HistoryFreshness::note` and
//! `EarlierHistoryUnavailable::note` — immediately before the HTTP surface became their second copy.
//! This file is what stops a copy coming back: two further surfaces — MCP and the reference UI — are
//! still due to render the same values, and a surface that re-words `shallow_boundary` slightly is a
//! surface that has restated the invariant the historical model exists to protect.
//!
//! # One crate is no longer the answer, so the guard records an owner per note
//!
//! Seven of the eight vocabularies live in `nerve-core`. [`EarlierHistoryUnavailable`] does not: it
//! composes [`ParentCompleteness`] and [`WalkTermination`] rather than being a third axis, so it
//! lives in `nerve-store` and its note lives beside it. A single global "only `nerve-core` may hold
//! a note" rule would therefore have had to exempt that note from every crate, which is not a rule
//! at all. [`notes`] pairs each sentinel with **the one file allowed to hold it**, and the scan
//! covers all four crates rather than only the two surfaces — so a note copied *into* `nerve-store`
//! from `nerve-core`, or the reverse, fails as loudly as one copied into a surface.
//!
//! # The scan, and why it is shaped this way
//!
//! It mirrors `crates/nerve-server/tests/layering.rs` (`ee7f124`), which reads its crate's `src/`
//! **dynamically**: that file was a hand-maintained `include_str!` array until replacing it revealed
//! the array held 16 entries against 17 real files, so `token.rs` had never been scanned by any of
//! the four invariants it was supposed to be subject to. A hand-written file list here would be one
//! new module away from the same hole.
//!
//! Three differences from `layering.rs`, each deliberate:
//!
//! 1. **The sentinels are generated from the vocabulary**, not retyped. [`notes`] calls the note
//!    methods on every value of all eight vocabularies, so the thing searched for is the prose that
//!    actually ships. Retyping it would let the guard and the product drift apart, which is the
//!    exact class of defect the hoist exists to close.
//! 2. **The bytes are un-wrapped before searching.** Rust's `\<newline>` escape means a note's
//!    runtime text is almost never contiguous in its source, so a naive scan would find nothing for
//!    most of them — and "found nothing" is indistinguishable from "scanned nothing". Applying the
//!    escape's own rule to the source makes a re-wrapped copy fail too, which a raw substring scan
//!    could not do.
//! 3. **Test code is not stripped.** `layering.rs` strips it because a test proving `0.0.0.0` is
//!    refused must contain `0.0.0.0`. Nothing here needs to contain a note: a test that wants one
//!    asks the vocabulary for it, which is the property being enforced.

use std::path::{Path, PathBuf};

use nerve_core::vocab::{
    ChangesEnumerated, FirstObservedKind, HistoryFreshness, ParentCompleteness, RenameAmbiguity,
    WalkTermination,
};
use nerve_store::EarlierHistoryUnavailable;

/// The one file allowed to hold a note, relative to `crates/`.
///
/// A path rather than a crate name, because "exactly one crate" stopped being expressible once a
/// note moved to `nerve-store`: the owner is the module the vocabulary is declared in, and every
/// other file in every scanned crate — including the owner's own crate — must be free of it.
const CORE_VOCAB: &str = "nerve-core/src/vocab.rs";
/// The one file allowed to hold [`EarlierHistoryUnavailable`]'s note.
const STORE_HISTORY: &str = "nerve-store/src/history.rs";

/// The prose the eight hoisted notes render, each paired with the file that owns it.
///
/// Generated over `ALL`, so a value added to any of the eight vocabularies is searched for without
/// this file being edited. [`FirstObservedKind`] contributes **two** notes per value, because the
/// answer and the permission to call it a creation are separate sentences and both ship.
fn notes() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for value in WalkTermination::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in ParentCompleteness::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in ChangesEnumerated::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in RenameAmbiguity::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in FirstObservedKind::ALL {
        out.push((CORE_VOCAB, value.note()));
        out.push((CORE_VOCAB, value.created_claim_note()));
    }
    for value in HistoryFreshness::ALL {
        out.push((CORE_VOCAB, value.note()));
    }
    for value in EarlierHistoryUnavailable::ALL {
        out.push((STORE_HISTORY, value.note()));
    }
    out
}

/// How many sentinels [`notes`] must produce, computed from the vocabularies rather than typed.
fn expected_note_count() -> usize {
    WalkTermination::ALL.len()
        + ParentCompleteness::ALL.len()
        + ChangesEnumerated::ALL.len()
        + RenameAmbiguity::ALL.len()
        + FirstObservedKind::ALL.len() * 2
        + HistoryFreshness::ALL.len()
        + EarlierHistoryUnavailable::ALL.len()
}

/// Undo the two source-level escapes that stand between a note's bytes on disk and its bytes at run
/// time: `\<newline><whitespace>`, and `\"` / `\\`.
///
/// The line continuation swallows the newline **and** the leading whitespace of the next line, which
/// is how a 100-character note is written across three source lines and is still one string at run
/// time. `ChangesEnumerated::Enumerated`'s note ends in the quoted words `"nothing changed"`, which
/// are `\"nothing changed\"` on disk. Without both, most sentinels would be absent from their own
/// source file — and a sentinel nothing can find makes every negative below pass for free, which is
/// why `the_history_notes_exist_in_exactly_one_crate` checks each one against `vocab.rs` first.
///
/// This is deliberately **not** a Rust lexer. It is applied to whole files, so a `'\n'` character
/// literal outside a string becomes a real newline; that is harmless for a substring scan and much
/// less likely to be wrong than parsing Rust here would be.
fn joined(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            match bytes.get(index + 1) {
                // An escaped backslash is one backslash, and it is handled before the continuation
                // so that `\\` followed by a real newline is not read as a continuation.
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

/// Does `haystack` contain `needle`? Bytes, not `str`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Every `.rs` file under `dir`, recursively, read as **bytes** and un-wrapped.
///
/// `std::fs::read`, never `read_to_string`. `docs/CONTINUATION.md:324` records a literal NUL byte in
/// a source file making `grep` skip that file silently, which hid a real defect for a whole slice;
/// `read_to_string` on invalid UTF-8 fails outright, and a guard that panics on one file or — worse —
/// skips it is a guard that can be disabled by adding a byte.
/// `the_scan_reads_bytes_so_a_hostile_byte_cannot_hide_a_copy` proves this rather than asserting it.
fn sources(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    collect(dir, dir, &mut found);
    found.sort();
    found
}

fn collect(base: &Path, dir: &Path, found: &mut Vec<(String, Vec<u8>)>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            collect(base, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
            let name = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            found.push((name, joined(&bytes)));
        }
    }
}

fn crate_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name)
        .join("src")
}

/// One owner file's bytes, un-wrapped, keyed by the `crates/`-relative path [`notes`] records.
fn owner_source(owner: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(owner);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    joined(&bytes)
}

/// Every `.rs` file of every scanned crate, keyed by its `crates/`-relative path.
fn scanned_workspace() -> Vec<(String, Vec<u8>)> {
    let mut scanned: Vec<(String, Vec<u8>)> = Vec::new();
    for crate_name in ["nerve-cli", "nerve-server", "nerve-core", "nerve-store"] {
        for (name, bytes) in sources(&crate_dir(crate_name)) {
            scanned.push((format!("{crate_name}/src/{name}"), bytes));
        }
    }
    scanned
}

/// No file but the one that owns a note may contain it as a literal.
///
/// The two anti-vacuity halves come first, because a source scan that finds nothing is otherwise
/// indistinguishable from one that scanned nothing.
#[test]
fn every_history_note_exists_in_exactly_one_file() {
    let notes = notes();
    assert_eq!(
        notes.len(),
        expected_note_count(),
        "a vocabulary lost a value between `ALL` and its note"
    );
    assert!(notes.len() >= 35, "found only {} notes", notes.len());
    // Both owners are actually used. A refactor that moved every note into one file would otherwise
    // make the per-owner machinery below vacuous while every assertion still passed.
    for owner in [CORE_VOCAB, STORE_HISTORY] {
        assert!(
            notes.iter().any(|(holder, _)| *holder == owner),
            "no note is owned by {owner}, so the per-file rule is not being exercised"
        );
    }

    // Anti-vacuity 1: every sentinel must exist in the file that is allowed to hold it. Renaming a
    // note without moving it would otherwise make every negative below pass for free.
    for (owner, note) in &notes {
        assert!(
            contains(&owner_source(owner), note.as_bytes()),
            "{owner} does not contain {note:?}, so searching every other file for it proves nothing"
        );
    }

    // Anti-vacuity 2: the walk really walked. A floor rather than an equality so the crates may
    // grow; 43 is the measured count across the four `src/` trees.
    let scanned = scanned_workspace();
    assert!(
        scanned.len() >= 40,
        "expected to scan four crates, found {} files: {:?}",
        scanned.len(),
        scanned.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    // And the scan reached the file the notes were hoisted out of, the file the HTTP surface renders
    // them from, and both owners — by name, so a renamed module cannot quietly leave the scan.
    for required in [
        "nerve-cli/src/main.rs",
        "nerve-server/src/api/history.rs",
        CORE_VOCAB,
        STORE_HISTORY,
    ] {
        assert!(
            scanned.iter().any(|(name, _)| name == required),
            "{required} was not scanned"
        );
    }

    for (name, bytes) in &scanned {
        for (owner, note) in &notes {
            if name == owner {
                continue;
            }
            assert!(
                !contains(bytes, note.as_bytes()),
                "{name} contains a history note as a literal: {note:?}. That note lives in \
                 {owner} — call the vocabulary's own method. A second copy is free to drift from \
                 the rule it renders."
            );
        }
    }
}

/// Every interface source file, read as **bytes**, keyed by its `apps/nerve-web/src/`-relative path.
///
/// The same byte read as [`sources`], for the same reason: `docs/CONTINUATION.md:324` records a
/// literal NUL in `views/Graph.tsx` making `grep` treat it as binary and report no matches at all,
/// which produced a false "dead code" finding that stood for a whole slice.
fn interface_sources() -> Vec<(String, Vec<u8>)> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/nerve-web/src");
    let mut found = Vec::new();
    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_source = path
                .extension()
                .is_some_and(|ext| ["ts", "tsx", "mjs", "js", "jsx"].iter().any(|k| ext == *k));
            if !is_source {
                continue;
            }
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
            let name = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            found.push((name, bytes));
        }
    }
    found.sort();
    found
}

/// The reference UI renders the notes the backend sends. It may not hold a copy of one.
///
/// Slice 12c-iv is the surface `every_history_note_exists_in_exactly_one_file`'s doc comment
/// predicted: the fourth renderer of these values, and the first one written in a different
/// language, where the guard scanning `crates/` could not see it. A gloss and a note are different
/// things — `apps/nerve-web/src/vocab.ts` carries a short reading of each *value*, which is the
/// interface's own voice and is required by `crates/nerve-server/tests/ui_vocabulary.rs` — but the
/// sentences are the vocabulary's, they arrive on the response as `kind_note`,
/// `changes_enumerated_note`, `walk_terminated_note` and the rest, and a copy pasted into
/// TypeScript would be free to drift from the rule it renders while every Rust test stayed green.
///
/// No `joined` here: Rust's line-continuation escape has no equivalent in a TypeScript string, so
/// the bytes on disk are the bytes. A copy re-wrapped across a template literal or a `+` would
/// escape this scan, which is why the positive half below matters — the interface must be shown to
/// be *rendering* the notes, or "no copy found" would be satisfied by a screen that shows none.
#[test]
fn no_interface_source_holds_a_copy_of_a_history_note() {
    let notes = notes();
    assert!(notes.len() >= 35, "found only {} notes", notes.len());

    let sources = interface_sources();
    assert!(
        sources.len() >= 20,
        "expected to scan the interface source, found {} files",
        sources.len()
    );
    // By name, so a renamed or deleted view cannot quietly leave the scan while it passes.
    for required in [
        "vocab.ts",
        "history.ts",
        "views/HistoryParts.tsx",
        "views/HistoryPath.tsx",
    ] {
        assert!(
            sources.iter().any(|(name, _)| name == required),
            "{required} was not scanned"
        );
    }

    for (name, bytes) in &sources {
        for (owner, note) in &notes {
            assert!(
                !contains(bytes, note.as_bytes()),
                "apps/nerve-web/src/{name} contains a history note as a literal: {note:?}. That \
                 note lives in {owner} and the API sends it on the response — render what was \
                 sent. A second copy is free to drift from the rule it renders."
            );
        }
    }

    // The positive half. The interface has to be rendering the sentences it is forbidden to copy,
    // or this test is satisfied by an interface that says nothing at all.
    let mut rendered = 0;
    for field in [
        "may_claim_created_note",
        "kind_note",
        "changes_enumerated_note",
        "walk_terminated_note",
        "parent_completeness_note",
        "ambiguity_note",
        "freshness_note",
        "earlier_history_unavailable_note",
    ] {
        assert!(
            sources
                .iter()
                .any(|(_, bytes)| contains(bytes, field.as_bytes())),
            "no interface source reads `{field}`, so the note it carries reaches nobody"
        );
        rendered += 1;
    }
    assert_eq!(rendered, 8);
}

/// The judgment moved too, and it is the one that matters most.
///
/// `earlier_changes_may_exist` is not wording: it is the single question every history surface must
/// agree on — whether history exists above what was read. Three surfaces each deriving it
/// independently is the drift the four notes are merely the visible half of. No crate outside
/// `nerve-store` may re-derive it, and the shape it would be re-derived in is a `WalkTermination`
/// comparison beside a `shallow` read.
#[test]
fn no_surface_re_derives_whether_earlier_changes_may_exist() {
    // Anti-vacuity: the definition exists where it is supposed to, and says what it is supposed to.
    let store = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nerve-store/src/history.rs");
    let definition =
        joined(&std::fs::read(&store).expect("nerve-store/src/history.rs is readable"));
    for required in [
        b"pub fn earlier_changes_may_exist(ingest: &IngestRow) -> bool".as_slice(),
        b"ingest.shallow || ingest.walk_terminated_by != WalkTermination::Exhausted".as_slice(),
    ] {
        assert!(
            contains(&definition, required),
            "nerve-store/src/history.rs no longer holds the definition this test guards: {:?}",
            String::from_utf8_lossy(required)
        );
    }

    let mut scanned = 0;
    for crate_name in ["nerve-cli", "nerve-server"] {
        for (name, bytes) in sources(&crate_dir(crate_name)) {
            scanned += 1;
            for forbidden in [
                b"WalkTermination::Exhausted".as_slice(),
                b"!= WalkTermination".as_slice(),
                b"walk_terminated_by !=".as_slice(),
            ] {
                assert!(
                    !contains(&bytes, forbidden),
                    "{crate_name}/src/{name} re-derives history availability: {:?}. Call \
                     `nerve_store::earlier_changes_may_exist` instead.",
                    String::from_utf8_lossy(forbidden)
                );
            }
        }
    }
    assert!(scanned >= 19, "only {scanned} files scanned");
}

/// **The scan reads bytes.** A file `read_to_string` cannot decode is still scanned.
///
/// This is the third anti-vacuity assertion and the least obvious one. A guard built on
/// `read_to_string` either panics on a file with a stray byte or, if it were written to skip
/// unreadable files, silently exempts it — and `docs/CONTINUATION.md:324` records exactly that: a
/// literal NUL in a source file made `grep` skip it without saying so, and a real defect stayed
/// hidden for a whole slice. So the guard is given a file that is *not* valid UTF-8, that contains a
/// NUL, and that hides a note behind both, and it has to find the note anyway.
#[test]
fn the_scan_reads_bytes_so_a_hostile_byte_cannot_hide_a_copy() {
    let note = ParentCompleteness::ShallowBoundary.note();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let planted = dir.path().join("nested").join("copy.rs");
    std::fs::create_dir_all(planted.parent().expect("a parent")).expect("create the nested dir");

    // Invalid UTF-8 (a lone 0xFF), a NUL, and the note split across a `\<newline>` continuation —
    // the three things that would each defeat a naive scan on their own.
    let mut bytes: Vec<u8> = b"// \xff\x00 hidden\nfn note() -> &'static str {\n    \"".to_vec();
    let split = note.len() / 2;
    bytes.extend_from_slice(&note.as_bytes()[..split]);
    bytes.extend_from_slice(b"\\\n     ");
    bytes.extend_from_slice(&note.as_bytes()[split..]);
    bytes.extend_from_slice(b"\"\n}\n");
    std::fs::write(&planted, &bytes).expect("write the planted copy");

    // The control: this file really cannot be read as a string, so the byte read is load-bearing
    // rather than a stylistic preference.
    assert!(
        std::fs::read_to_string(&planted).is_err(),
        "the planted file must be undecodable, or this test proves nothing about reading bytes"
    );

    let found = sources(dir.path());
    assert_eq!(found.len(), 1, "the walk must reach the nested file");
    assert!(
        contains(&found[0].1, note.as_bytes()),
        "the scan missed a note hidden behind a control byte, invalid UTF-8 and a line continuation"
    );

    // And the same file without the note is not a match, so `contains` is deciding rather than
    // always answering yes.
    let clean = dir.path().join("nested").join("clean.rs");
    std::fs::write(&clean, b"// \xff\x00 nothing to see\nfn f() {}\n").expect("write");
    let found = sources(dir.path());
    assert_eq!(found.len(), 2);
    assert_eq!(
        found
            .iter()
            .filter(|(_, bytes)| contains(bytes, note.as_bytes()))
            .count(),
        1,
        "exactly one of the two planted files holds the note"
    );
}
