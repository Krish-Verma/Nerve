//! The interface must be able to name everything the backend can store.
//!
//! Nerve's vocabularies are declared in Rust and glossed in TypeScript, and between Slice 5a and
//! Slice 5d-iii the two drifted: the backend learned `document`, `section`, `SUPERSEDES`,
//! `FILESYSTEM_OBSERVED` and seven document-reference reasons, and the interface answered "this
//! build has no description for that entity kind" for every one of them. Nobody noticed because
//! nothing checked.
//!
//! This is the thing that checks. It reads the shipped TypeScript **as text** — no Node, no
//! bundler, no runtime — extracts the keys of each gloss table, and asserts that every member of
//! the corresponding Rust vocabulary has an entry. Adding a variant in Rust without a gloss now
//! fails here, at the point the variant is written.
//!
//! Reading source text is a deliberate choice over the alternatives. Generating the TypeScript
//! from Rust would put English prose in the wrong crate and make the interface's own voice a
//! build artifact; running the TypeScript would put a Node runtime in `cargo test`. Parsing an
//! object literal for its keys is a small, total job, and every table this file reads is a flat
//! `Record<string, …>` written by hand for exactly this purpose.
//!
//! The one thing it deliberately does **not** assert is that a gloss is *good*. No test can. It
//! asserts only that one exists, is not empty, and is not the fallback sentence.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nerve_core::vocab::{
    AssertionStatus, Directness, EntityKind, EvidenceSourceType, Relation, UnresolvedCategory,
};
use nerve_index::docref;
use nerve_index::docs::{AdrStatus, STATUS_UNPARSED};
use nerve_index::refs::{UnresolvedReason, UNMODELLED_FORMS};
use nerve_store::Freshness;

// ---- locating and reading the interface source ----------------------------------------------

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root must exist")
}

fn read(relative: &str) -> String {
    let path = repository_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()))
}

fn vocab_ts() -> String {
    read("apps/nerve-web/src/vocab.ts")
}

fn format_ts() -> String {
    read("apps/nerve-web/src/format.ts")
}

fn types_ts() -> String {
    read("apps/nerve-web/src/api/types.ts")
}

/// The gloss tables this file checks, by declaring file and constant name.
///
/// One list, used by both the source-side vocabulary tests' companion below and the
/// bundle-staleness check, so a new gloss table cannot be added to one and forgotten by the other.
const GLOSS_TABLES: [(&str, &str); 11] = [
    ("format.ts", "FRESHNESS"),
    ("format.ts", "SOURCE_TYPES"),
    ("format.ts", "DIRECTNESS"),
    ("format.ts", "RELATION_VERB"),
    ("format.ts", "STATUS_GLOSS"),
    ("vocab.ts", "UNRESOLVED_REASON"),
    ("vocab.ts", "UNMODELLED_FORM"),
    ("vocab.ts", "KIND_GLOSS"),
    ("vocab.ts", "COVERAGE_STATE"),
    ("vocab.ts", "UNRESOLVED_CATEGORY"),
    ("vocab.ts", "ADR_STATUS"),
];

/// The interface compiled into the binary must be built from the interface source.
///
/// `crates/nerve-server/assets/assets/nerve.js` is a **tracked build artifact**, compiled in with
/// `include_bytes!` so `nerve serve` needs no Node. Nothing checked that it corresponded to the
/// TypeScript beside it, and it did not: the committed bundle was missing `is served by`, `was
/// observed calling` and `was observed called by` — the glosses for `SERVED_BY` (Slice 10a) and
/// `TEST_OBSERVED_CALL` (Slice 11a). Both vocabularies were added to the source, and the assets
/// were never re-embedded, so the shipped interface rendered fallback text for two relations the
/// backend emits.
///
/// Every vocabulary test in this file reads the TypeScript **source**, which is why they all passed
/// while the binary shipped something else. That is precisely the defect Slice 5d-iii existed to
/// close — it found 120 sites rendering fallback text — recurring one layer down, in the artifact
/// rather than in the source.
///
/// The check is a containment test rather than a rebuild: `cargo test` cannot run Vite, and should
/// not try. Every prose gloss the source declares must appear as a literal in the bundle. A
/// minifier renames identifiers but does not rewrite string contents, which is what makes this
/// sound — and it is exactly the class of drift that goes unnoticed, because adding a gloss and
/// forgetting `npm run build` leaves every other test green.
#[test]
fn the_embedded_bundle_carries_every_gloss_the_source_declares() {
    let bundle_bytes =
        std::fs::read(repository_root().join("crates/nerve-server/assets/assets/nerve.js"))
            .expect("the embedded bundle must exist — run `npm run build` in apps/nerve-web");
    let bundle = String::from_utf8_lossy(&bundle_bytes);

    let mut checked = 0;
    let mut missing = Vec::new();
    for (file, table) in GLOSS_TABLES {
        let source = read(&format!("apps/nerve-web/src/{file}"));
        for phrase in quoted_prose_in_table(&source, table) {
            checked += 1;
            if !bundle.contains(&phrase) {
                missing.push(format!("{file}/{table}: {phrase:?}"));
            }
        }
    }

    // Anti-vacuity. A parser that returned nothing would make this pass by checking nothing, which
    // is the whole failure mode being closed.
    assert!(
        checked >= 60,
        "expected to check the gloss tables, found only {checked} phrases"
    );
    assert!(
        missing.is_empty(),
        "{} gloss(es) are in the interface source but not in the embedded bundle — \
         run `npm run build` in apps/nerve-web and commit crates/nerve-server/assets:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

/// Prose string literals inside one object literal, at any depth.
///
/// "Prose" means it contains a space: a gloss is a sentence, while a key, a CSS class and a tone
/// name are single tokens. Restricting to one named table keeps the check away from strings that
/// tree-shaking may legitimately drop.
fn quoted_prose_in_table(source: &str, name: &str) -> Vec<String> {
    let start = body_start(source, name, "{");
    let chars: Vec<char> = source[start..].chars().collect();
    let mut found = Vec::new();
    let mut depth = 1usize;
    let mut index = 0usize;

    while index < chars.len() {
        if let Some(next) = skip_comment(&chars, index) {
            index = next;
            continue;
        }
        match chars[index] {
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            quote @ ('\'' | '"') => {
                let mut cursor = index + 1;
                let mut text = String::new();
                while cursor < chars.len() && chars[cursor] != quote {
                    if chars[cursor] == '\\' {
                        cursor += 1;
                    }
                    if let Some(ch) = chars.get(cursor) {
                        text.push(*ch);
                    }
                    cursor += 1;
                }
                if text.contains(' ') {
                    found.push(text);
                }
                index = cursor;
            }
            _ => {}
        }
        index += 1;
    }
    found
}

/// No interface source file may contain a raw C0 control byte.
///
/// `views/Graph.tsx` and `search.ts` each used a **literal NUL byte** as an in-band separator —
/// a composite sort key in one, a temporary split marker for camel-case words in the other. The
/// technique is sound; writing the byte instead of the `\u0000` escape is not, for three reasons
/// that all bit at once:
///
/// 1. **`grep` treats a file containing NUL as binary and prints no matches.** An audit of this
///    repository concluded that `views/Path.tsx` was dead code, because `grep -rn PathFinder`
///    found nothing — while `Graph.tsx:31` imports it. `file(1)` called the file "data". A
///    silent false negative from the tool an audit trusts most is worse than a loud defect.
/// 2. **Subagent file tools strip C0 bytes** — recorded in `docs/CONTINUATION.md` under
///    environment notes. Any agent asked to edit either file would have removed the separator and
///    silently changed what the code means.
/// 3. Slice 5a closed an identity-forgery vector by making `discover::canonical_child` refuse the
///    **entire C0 range** in a path, because a literal `0x1f` could merge two entities onto one
///    identity. Refusing those bytes in repository input while writing them into our own source
///    is not a position this project can hold.
///
/// Tab, newline and carriage return are permitted: they are C0 and they are what text is made of.
#[test]
fn no_interface_source_file_contains_a_raw_control_byte() {
    let src = repository_root().join("apps/nerve-web/src");
    let mut scanned = 0;
    let mut offenders = Vec::new();

    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("apps/nerve-web/src must be readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_source = path.extension().is_some_and(|ext| {
                ["ts", "tsx", "mjs", "js", "jsx", "css"]
                    .iter()
                    .any(|k| ext == *k)
            });
            if !is_source {
                continue;
            }
            scanned += 1;
            let bytes = std::fs::read(&path).expect("a source file must be readable");
            for (offset, byte) in bytes.iter().enumerate() {
                if *byte < 0x20 && !matches!(byte, b'\t' | b'\n' | b'\r') {
                    let line = bytes[..offset].iter().filter(|b| **b == b'\n').count() + 1;
                    offenders.push(format!(
                        "{}:{line} holds byte 0x{byte:02x} — write it as an escape",
                        path.strip_prefix(&src).unwrap_or(&path).display()
                    ));
                }
            }
        }
    }

    // Anti-vacuity: a walk that found nothing would pass by scanning nothing.
    assert!(
        scanned >= 20,
        "expected to scan the interface source, found {scanned} files"
    );
    assert!(
        offenders.is_empty(),
        "raw control bytes in interface source:\n  {}",
        offenders.join("\n  ")
    );
}

// ---- a very small object-literal reader ------------------------------------------------------

/// Skip a `//` or `/* */` comment starting at `index`; returns the index just past it.
fn skip_comment(chars: &[char], index: usize) -> Option<usize> {
    if chars.get(index) != Some(&'/') {
        return None;
    }
    match chars.get(index + 1) {
        Some('/') => {
            let mut cursor = index + 2;
            while cursor < chars.len() && chars[cursor] != '\n' {
                cursor += 1;
            }
            Some(cursor)
        }
        Some('*') => {
            let mut cursor = index + 2;
            while cursor + 1 < chars.len() && !(chars[cursor] == '*' && chars[cursor + 1] == '/') {
                cursor += 1;
            }
            Some((cursor + 2).min(chars.len()))
        }
        _ => None,
    }
}

/// Where the body of `const <name> … = <open>` starts, just past the opening bracket.
fn body_start(source: &str, name: &str, open: &str) -> usize {
    let declaration = format!("const {name}");
    let at = source
        .find(&declaration)
        .unwrap_or_else(|| panic!("`{declaration}` is not declared in the interface source"));
    let opener = format!("= {open}");
    let offset = source[at..]
        .find(&opener)
        .unwrap_or_else(|| panic!("`{declaration}` has no `{opener}`"));
    at + offset + opener.len()
}

/// The top-level keys of a flat object literal declared as `const <name> … = { … }`.
///
/// Depth-tracked rather than line-matched, so a nested value object (`FRESHNESS` holds one)
/// contributes none of its own keys. Values are never mistaken for keys because a key is only
/// read where one is syntactically expected: at the start of the literal, or just after a comma
/// at depth one.
fn object_keys(source: &str, name: &str) -> Vec<String> {
    let chars: Vec<char> = source[body_start(source, name, "{")..].chars().collect();
    let mut keys = Vec::new();
    let mut depth = 1usize;
    let mut expecting_key = true;
    let mut index = 0usize;

    while index < chars.len() {
        if let Some(next) = skip_comment(&chars, index) {
            index = next;
            continue;
        }
        let current = chars[index];
        match current {
            '{' | '[' | '(' => {
                depth += 1;
                expecting_key = false;
                index += 1;
            }
            '}' | ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                expecting_key = false;
                index += 1;
            }
            ',' | ';' => {
                if depth == 1 {
                    expecting_key = true;
                }
                index += 1;
            }
            '\'' | '"' | '`' => {
                let quote = current;
                let mut cursor = index + 1;
                let mut text = String::new();
                while cursor < chars.len() && chars[cursor] != quote {
                    if chars[cursor] == '\\' {
                        cursor += 1;
                    }
                    if cursor < chars.len() {
                        text.push(chars[cursor]);
                    }
                    cursor += 1;
                }
                if depth == 1 && expecting_key {
                    keys.push(text);
                }
                expecting_key = false;
                index = cursor + 1;
            }
            c if c.is_whitespace() => index += 1,
            c if c.is_alphanumeric() || c == '_' || c == '$' => {
                let mut cursor = index;
                let mut text = String::new();
                while cursor < chars.len()
                    && (chars[cursor].is_alphanumeric()
                        || chars[cursor] == '_'
                        || chars[cursor] == '$'
                        || chars[cursor] == '-')
                {
                    text.push(chars[cursor]);
                    cursor += 1;
                }
                if depth == 1 && expecting_key {
                    keys.push(text);
                }
                expecting_key = false;
                index = cursor;
            }
            _ => {
                expecting_key = false;
                index += 1;
            }
        }
    }
    keys
}

/// The string elements of a `const <name> = [ … ]` literal, in order.
fn array_strings(source: &str, name: &str) -> Vec<String> {
    let chars: Vec<char> = source[body_start(source, name, "[")..].chars().collect();
    let mut values = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if let Some(next) = skip_comment(&chars, index) {
            index = next;
            continue;
        }
        match chars[index] {
            ']' => break,
            quote @ ('\'' | '"') => {
                let mut cursor = index + 1;
                let mut text = String::new();
                while cursor < chars.len() && chars[cursor] != quote {
                    text.push(chars[cursor]);
                    cursor += 1;
                }
                values.push(text);
                index = cursor + 1;
            }
            _ => index += 1,
        }
    }
    values
}

/// The literals of every `case '…':` arm inside a named function.
fn case_arms(source: &str, function: &str) -> Vec<String> {
    let signature = format!("export function {function}");
    let at = source
        .find(&signature)
        .unwrap_or_else(|| panic!("`{signature}` is not declared in the interface source"));
    let tail = &source[at..];
    let end = tail[1..]
        .find("\nexport ")
        .map(|offset| offset + 1)
        .unwrap_or(tail.len());
    let body = &tail[..end];

    let mut arms = Vec::new();
    let mut rest = body;
    while let Some(offset) = rest.find("case '") {
        let after = &rest[offset + "case '".len()..];
        let close = after.find('\'').expect("an unterminated case literal");
        arms.push(after[..close].to_string());
        rest = &after[close..];
    }
    arms
}

/// Assert every member of a Rust vocabulary has an entry in a named interface table.
fn covers(kind: &str, table: &str, expected: &[String], actual: &[String]) {
    let present: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    let missing: Vec<&str> = expected
        .iter()
        .map(String::as_str)
        .filter(|value| !present.contains(value))
        .collect();
    assert!(
        missing.is_empty(),
        "{kind} has {} member(s) with no entry in `{table}`: {missing:?}\n\
         The backend can emit these and the interface would fall back to \"this build has no \
         description\". Add a gloss in the interface's own voice — one sentence, saying what the \
         value means to a reader who has not read the schema.",
        missing.len()
    );
}

// ---- the vocabularies ------------------------------------------------------------------------

#[test]
fn every_entity_kind_is_glossed() {
    let expected: Vec<String> = EntityKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();
    covers(
        "EntityKind::ALL",
        "KIND_GLOSS (apps/nerve-web/src/vocab.ts)",
        &expected,
        &object_keys(&vocab_ts(), "KIND_GLOSS"),
    );
}

#[test]
fn every_relation_has_a_verb_in_both_directions() {
    let source = format_ts();
    let expected: Vec<String> = Relation::ALL
        .iter()
        .map(|relation| relation.as_str().to_string())
        .collect();
    covers(
        "Relation::ALL",
        "RELATION_VERB (apps/nerve-web/src/format.ts)",
        &expected,
        &object_keys(&source, "RELATION_VERB"),
    );

    // A verb pair is `[outgoing, incoming]`. One of them missing renders the relation from the
    // wrong side, which reads as a true sentence about a relationship that does not exist.
    for relation in Relation::ALL {
        let needle = format!("{}: [", relation.as_str());
        let at = source
            .find(&needle)
            .unwrap_or_else(|| panic!("{relation} has no verb pair"));
        let pair = &source[at..at + source[at..].find(']').expect("an unterminated verb pair")];
        assert_eq!(
            pair.matches('\'').count(),
            4,
            "{relation} must have both an outgoing and an incoming verb: {pair}"
        );
    }
}

#[test]
fn every_evidence_source_type_is_glossed() {
    let expected: Vec<String> = EvidenceSourceType::ALL
        .iter()
        .map(|source| source.as_str().to_string())
        .collect();
    covers(
        "EvidenceSourceType::ALL",
        "SOURCE_TYPES (apps/nerve-web/src/format.ts)",
        &expected,
        &object_keys(&format_ts(), "SOURCE_TYPES"),
    );
}

/// Directness must be glossed **and** must have its own visual class.
///
/// The second half is the one that mattered: `directnessClass` used to end in a `default` arm
/// returning `obs--inferred`, so an unrecognised directness was drawn as though a rule had
/// concluded it. That is not a missing gloss, it is a false claim about the evidence — the
/// strongest thing this interface says is exactly what it must never invent.
#[test]
fn every_directness_is_glossed_and_has_its_own_class() {
    let source = format_ts();
    let expected: Vec<String> = Directness::ALL
        .iter()
        .map(|directness| directness.as_str().to_string())
        .collect();
    covers(
        "Directness::ALL",
        "DIRECTNESS (apps/nerve-web/src/format.ts)",
        &expected,
        &object_keys(&source, "DIRECTNESS"),
    );

    let arms = case_arms(&source, "directnessClass");
    covers(
        "Directness::ALL",
        "directnessClass (apps/nerve-web/src/format.ts)",
        &expected,
        &arms,
    );

    let function_at = source
        .find("export function directnessClass")
        .expect("directnessClass must exist");
    let body = &source[function_at..];
    let end = body.find("\n}").expect("directnessClass must be closed");
    assert!(
        !body[..end].contains("default:"),
        "`directnessClass` must not have a `default:` arm. A value outside `Directness::ALL` has \
         to render as visibly unknown; silently reusing a real vocabulary member's class states \
         something about the evidence that was never observed."
    );
}

#[test]
fn every_assertion_status_is_glossed() {
    let expected: Vec<String> = AssertionStatus::ALL
        .iter()
        .map(|status| status.as_str().to_string())
        .collect();
    covers(
        "AssertionStatus::ALL",
        "STATUS_GLOSS (apps/nerve-web/src/format.ts)",
        &expected,
        &object_keys(&format_ts(), "STATUS_GLOSS"),
    );
}

#[test]
fn every_freshness_value_is_glossed() {
    let expected: Vec<String> = Freshness::ALL
        .iter()
        .map(|value| value.as_str().to_string())
        .collect();
    covers(
        "Freshness::ALL",
        "FRESHNESS (apps/nerve-web/src/format.ts)",
        &expected,
        &object_keys(&format_ts(), "FRESHNESS"),
    );
}

/// The reason vocabulary is not one enum, so it is assembled from every place that owns part of it.
#[test]
fn every_unresolved_reason_is_glossed() {
    let mut expected: Vec<String> = UnresolvedReason::ALL
        .iter()
        .map(|reason| reason.as_str().to_string())
        .collect();
    expected.extend(
        docref::reason::ALL
            .iter()
            .map(|reason| (*reason).to_string()),
    );

    covers(
        "the unresolved-reason vocabulary (refs::UnresolvedReason::ALL + docref::reason::ALL)",
        "UNRESOLVED_REASON (apps/nerve-web/src/vocab.ts)",
        &expected,
        &object_keys(&vocab_ts(), "UNRESOLVED_REASON"),
    );
}

#[test]
fn every_unresolved_category_is_glossed() {
    let expected: Vec<String> = UnresolvedCategory::ALL
        .iter()
        .map(|category| category.as_str().to_string())
        .collect();
    covers(
        "UnresolvedCategory::ALL",
        "UNRESOLVED_CATEGORY (apps/nerve-web/src/vocab.ts)",
        &expected,
        &object_keys(&vocab_ts(), "UNRESOLVED_CATEGORY"),
    );
}

#[test]
fn every_unmodelled_form_is_glossed() {
    let expected: Vec<String> = UNMODELLED_FORMS.iter().map(|f| (*f).to_string()).collect();
    covers(
        "refs::UNMODELLED_FORMS",
        "UNMODELLED_FORM (apps/nerve-web/src/vocab.ts)",
        &expected,
        &object_keys(&vocab_ts(), "UNMODELLED_FORM"),
    );
}

/// `UNMODELLED_FORMS` is a hand-written list, so it needs its own guard.
///
/// The tags are bare literals at the site that recognises each shape; nothing in the type system
/// ties them to the list. So the list is checked against the extractor's own source: a new
/// `self.unmodelled("…")` that nobody added to `UNMODELLED_FORMS` fails here, and the interface
/// coverage test above then keeps the gloss honest in turn.
#[test]
fn the_unmodelled_form_list_matches_the_extractor_source() {
    let source = read("crates/nerve-index/src/refs.rs");
    let declared: BTreeSet<&str> = UNMODELLED_FORMS.iter().copied().collect();

    let mut emitted = BTreeSet::new();
    let mut rest = source.as_str();
    while let Some(offset) = rest.find("unmodelled(\"") {
        let after = &rest[offset + "unmodelled(\"".len()..];
        let close = after.find('"').expect("an unterminated form tag");
        emitted.insert(after[..close].to_string());
        rest = &after[close..];
    }

    assert!(
        !emitted.is_empty(),
        "the source scan found no form tags at all"
    );
    let undeclared: Vec<&String> = emitted
        .iter()
        .filter(|tag| !declared.contains(tag.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "`refs::UNMODELLED_FORMS` is missing form tag(s) emitted by the extractor: {undeclared:?}"
    );
}

/// The ADR status vocabulary, including the value that means "an ADR said something else".
#[test]
fn every_adr_status_is_glossed() {
    let mut expected: Vec<String> = AdrStatus::ALL
        .iter()
        .map(|status| status.as_str().to_string())
        .collect();
    expected.push(STATUS_UNPARSED.to_string());
    covers(
        "AdrStatus::ALL + STATUS_UNPARSED",
        "ADR_STATUS (apps/nerve-web/src/vocab.ts)",
        &expected,
        &object_keys(&vocab_ts(), "ADR_STATUS"),
    );
}

/// The two vocabularies the interface mirrors as data, not as prose.
///
/// `ENTITY_KINDS` drives the search filter and `RELATIONS` the graph filter, so a missing member
/// is not a missing sentence — it is a value the user cannot ask about at all. Order is asserted
/// too: these lists are rendered in array order, and matching the Rust declaration order keeps
/// the interface's ordering a fact rather than a preference.
#[test]
fn the_mirrored_vocabularies_match_rust_exactly() {
    let source = types_ts();

    let kinds: Vec<String> = EntityKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();
    assert_eq!(
        array_strings(&source, "ENTITY_KINDS"),
        kinds,
        "`ENTITY_KINDS` in apps/nerve-web/src/api/types.ts must mirror `EntityKind::ALL` exactly"
    );

    let relations: Vec<String> = Relation::ALL
        .iter()
        .map(|relation| relation.as_str().to_string())
        .collect();
    assert_eq!(
        array_strings(&source, "RELATIONS"),
        relations,
        "`RELATIONS` in apps/nerve-web/src/api/types.ts must mirror `Relation::ALL` exactly"
    );
}

/// A gloss that exists but says nothing is the same defect wearing a different hat.
#[test]
fn no_gloss_is_empty_or_the_fallback_sentence() {
    for (source, table) in [
        (vocab_ts(), "KIND_GLOSS"),
        (vocab_ts(), "UNRESOLVED_REASON"),
        (vocab_ts(), "UNRESOLVED_CATEGORY"),
        (vocab_ts(), "UNMODELLED_FORM"),
        (vocab_ts(), "ADR_STATUS"),
        (format_ts(), "SOURCE_TYPES"),
        (format_ts(), "DIRECTNESS"),
        (format_ts(), "STATUS_GLOSS"),
    ] {
        let start = body_start(&source, table, "{");
        let mut depth = 1usize;
        let body: String = source[start..]
            .chars()
            .take_while(|c| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                depth > 0
            })
            .collect();
        assert!(
            !body.contains("''") && !body.contains("\"\""),
            "`{table}` contains an empty gloss"
        );
        assert!(
            !body.contains("This build has no description"),
            "`{table}` uses the fallback sentence as an entry, which hides a missing gloss"
        );
    }
}

// ---- the JSON API is presentation-free -------------------------------------------------------

/// The gloss is a presentation layer, and this is the line it must not cross.
///
/// An API consumer receives `FILESYSTEM_OBSERVED`, not "the filesystem contains this". Every
/// vocabulary-bearing field of every response is read back and parsed with the *Rust* `FromStr`,
/// which accepts canonical names and nothing else — so a gloss substituted anywhere in the
/// serialization path fails here rather than being discovered by whoever wrote the first
/// third-party client.
///
/// The exact set of vocabulary values the fixture produces is pinned byte-for-byte. Timestamps,
/// entity ids and the database path are excluded deliberately: they are not vocabulary, and a
/// golden document containing them would fail for reasons that have nothing to do with the
/// property under test.
#[test]
fn the_json_api_speaks_only_the_canonical_vocabulary() {
    let (_dir, root) = common::fixture_copy("md-supersession");
    common::index(&root);
    let session = common::Session::start(&root);

    // Chosen by path rather than by rank, so the test does not depend on FTS ordering.
    let hits = session.json("/api/search?q=ADR&limit=200");
    let hits = hits["results"].as_array().expect("search returns results");
    let pick = |kind: &str, path: &str| -> String {
        hits.iter()
            .find(|hit| hit["kind"] == kind && hit["file_path"] == path)
            .unwrap_or_else(|| panic!("no {kind} hit for {path}"))["entity_id"]
            .as_str()
            .expect("an entity id")
            .to_string()
    };
    let document = pick("document", "docs/decisions/ADR-0002-replacement.md");
    let file = pick("file", "docs/decisions/ADR-0002-replacement.md");

    let bodies: Vec<serde_json::Value> = vec![
        session.json("/api/overview"),
        session.json(&format!("/api/why?subject={document}")),
        session.json(&format!("/api/why?subject={file}")),
        session.json("/api/unresolved?limit=50"),
    ];

    // Every value of every vocabulary field, wherever it appears in the tree.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    fn walk(value: &serde_json::Value, seen: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    if let Some(text) = item.as_str() {
                        match key.as_str() {
                            "kind" => {
                                text.parse::<EntityKind>()
                                    .unwrap_or_else(|_| panic!("`{text}` is not an EntityKind"));
                                seen.insert(format!("kind={text}"));
                            }
                            "relation" => {
                                text.parse::<Relation>()
                                    .unwrap_or_else(|_| panic!("`{text}` is not a Relation"));
                                seen.insert(format!("relation={text}"));
                            }
                            "evidence_source_type" | "strongest_source_type" => {
                                text.parse::<EvidenceSourceType>().unwrap_or_else(|_| {
                                    panic!("`{text}` is not an EvidenceSourceType")
                                });
                                seen.insert(format!("source_type={text}"));
                            }
                            "directness" => {
                                text.parse::<Directness>()
                                    .unwrap_or_else(|_| panic!("`{text}` is not a Directness"));
                                seen.insert(format!("directness={text}"));
                            }
                            "status" if map.contains_key("assertion_id") => {
                                text.parse::<AssertionStatus>().unwrap_or_else(|_| {
                                    panic!("`{text}` is not an AssertionStatus")
                                });
                                seen.insert(format!("assertion_status={text}"));
                            }
                            "freshness" => {
                                seen.insert(format!("freshness={text}"));
                            }
                            _ => {}
                        }
                    }
                    walk(item, seen);
                }
                // `entities_by_kind` and `assertions_by_relation` are keyed by the vocabulary.
                for (key, item) in map {
                    if key == "entities_by_kind" {
                        for kind in item.as_object().into_iter().flatten().map(|(k, _)| k) {
                            kind.parse::<EntityKind>()
                                .unwrap_or_else(|_| panic!("`{kind}` is not an EntityKind"));
                            seen.insert(format!("kind={kind}"));
                        }
                    }
                    if key == "assertions_by_relation" {
                        for relation in item.as_object().into_iter().flatten().map(|(k, _)| k) {
                            relation
                                .parse::<Relation>()
                                .unwrap_or_else(|_| panic!("`{relation}` is not a Relation"));
                            seen.insert(format!("relation={relation}"));
                        }
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, seen)),
            _ => {}
        }
    }
    for body in &bodies {
        walk(body, &mut seen);
    }

    // The property the whole slice turns on, stated as data rather than as prose.
    let pinned: Vec<&str> = vec![
        "assertion_status=SUPPORTED",
        "directness=DIRECT",
        "directness=RESOLVED",
        "freshness=fresh",
        "kind=directory",
        "kind=document",
        "kind=file",
        "kind=repository",
        "kind=section",
        "kind=unresolved",
        "relation=CONTAINS",
        "relation=REFERENCES",
        "relation=SUPERSEDES",
        "source_type=DOCUMENT_STATED",
        "source_type=FILESYSTEM_OBSERVED",
    ];
    let actual: Vec<&str> = seen.iter().map(String::as_str).collect();
    assert_eq!(
        actual, pinned,
        "the vocabulary the API speaks changed. The gloss is presentation only: an API consumer \
         must keep receiving the canonical values."
    );

    // And no gloss sentence from either interface table appears in any response.
    let interface = format!("{}{}", vocab_ts(), format_ts());
    for sentence in [
        "The filesystem contains this",
        "A document asserts it",
        "A decision cannot replace itself",
    ] {
        assert!(
            interface.contains(sentence),
            "`{sentence}` is no longer an interface gloss, so this check is testing nothing"
        );
        for body in &bodies {
            assert!(
                !body.to_string().contains(sentence),
                "the gloss `{sentence}` reached the JSON API"
            );
        }
    }
}
