//! The history notes exist in **exactly one crate**, enforced rather than asserted in a comment.
//!
//! Slice 12b left four wording functions and one interpretation predicate inside the CLI binary
//! (`main.rs:2151`, `:2180`, `:2210`, `:2230`, `:2255`). Slice 12c-i moved the four notes to
//! `nerve-core`, beside the vocabularies they render, and the judgment to `nerve-store`, beside
//! `IngestRow`. This file is what stops a copy coming back: three further surfaces — HTTP, MCP and
//! the reference UI — are due to render the same values, and a surface that re-words
//! `shallow_boundary` slightly is a surface that has restated the invariant the historical model
//! exists to protect.
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
//! 1. **The sentinels are generated from the vocabulary**, not retyped. `notes()` calls `note()` on
//!    every value of all four vocabularies, so the thing searched for is the prose that actually
//!    ships. Retyping it would let the guard and the product drift apart, which is the exact class
//!    of defect the hoist exists to close.
//! 2. **The bytes are un-wrapped before searching.** Rust's `\<newline>` escape means a note's
//!    runtime text is almost never contiguous in its source, so a naive scan would find nothing for
//!    most of them — and "found nothing" is indistinguishable from "scanned nothing". Applying the
//!    escape's own rule to the source makes a re-wrapped copy fail too, which a raw substring scan
//!    could not do.
//! 3. **Test code is not stripped.** `layering.rs` strips it because a test proving `0.0.0.0` is
//!    refused must contain `0.0.0.0`. Nothing here needs to contain a note: a test that wants one
//!    asks `nerve_core` for it, which is the property being enforced.

use std::path::{Path, PathBuf};

use nerve_core::vocab::{ChangesEnumerated, ParentCompleteness, RenameAmbiguity, WalkTermination};

/// The prose the four hoisted notes render, taken from the vocabulary rather than retyped.
///
/// Generated over `ALL`, so a value added to any of the four vocabularies is searched for without
/// this file being edited.
fn notes() -> Vec<&'static str> {
    let mut out = Vec::new();
    for value in WalkTermination::ALL {
        out.push(value.note());
    }
    for value in ParentCompleteness::ALL {
        out.push(value.note());
    }
    for value in ChangesEnumerated::ALL {
        out.push(value.note());
    }
    for value in RenameAmbiguity::ALL {
        out.push(value.note());
    }
    out
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

fn vocab_source() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nerve-core/src/vocab.rs");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    joined(&bytes)
}

/// No crate outside `nerve-core` may contain a history note as a literal.
///
/// The two anti-vacuity halves come first, because a source scan that finds nothing is otherwise
/// indistinguishable from one that scanned nothing.
#[test]
fn the_history_notes_exist_in_exactly_one_crate() {
    let notes = notes();
    assert_eq!(
        notes.len(),
        WalkTermination::ALL.len()
            + ParentCompleteness::ALL.len()
            + ChangesEnumerated::ALL.len()
            + RenameAmbiguity::ALL.len(),
        "a vocabulary lost a value between `ALL` and its note"
    );
    assert!(notes.len() >= 18, "found only {} notes", notes.len());

    // Anti-vacuity 1: every sentinel must exist in the crate that is allowed to hold it. Renaming a
    // note without moving it would otherwise make every negative below pass for free.
    let vocab = vocab_source();
    for note in &notes {
        assert!(
            contains(&vocab, note.as_bytes()),
            "nerve-core/src/vocab.rs does not contain {note:?}, so searching other crates for it \
             proves nothing"
        );
    }

    // Anti-vacuity 2: the walk really walked. A floor rather than an equality so the crates may
    // grow; 20 is the measured count across the two `src/` trees.
    let mut scanned: Vec<(String, Vec<u8>)> = Vec::new();
    for crate_name in ["nerve-cli", "nerve-server"] {
        for (name, bytes) in sources(&crate_dir(crate_name)) {
            scanned.push((format!("{crate_name}/src/{name}"), bytes));
        }
    }
    assert!(
        scanned.len() >= 19,
        "expected to scan both surface crates, found {} files: {:?}",
        scanned.len(),
        scanned.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    // And the scan reached the file the notes were hoisted out of, by name.
    assert!(
        scanned
            .iter()
            .any(|(name, _)| name == "nerve-cli/src/main.rs"),
        "the file the four functions used to live in was not scanned"
    );

    for (name, bytes) in &scanned {
        for note in &notes {
            assert!(
                !contains(bytes, note.as_bytes()),
                "{name} contains a history note as a literal: {note:?}. The notes live on the \
                 vocabulary in nerve-core — call `value.note()`. A second copy is free to drift \
                 from the rule it renders."
            );
        }
    }
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
