//! The `md-structural` extractor: documents, sections, and ADR recognition.
//!
//! A document is a **witness, never an authority**. Everything this extractor produces carries
//! [`EvidenceSourceType::DocumentStated`] and nothing else — including `File CONTAINS Document`,
//! which is arguably a filesystem fact. That is deliberate (`docs/plans/slice-05-document-
//! evidence.md` §3.3): it makes the THREAT-MODEL.md **T7** separation a total function of the
//! source file rather than a per-claim judgement, which yields one invariant that can be checked
//! exhaustively over the database —
//!
//! > No observation whose `file_path` is a document has any `evidence_source_type` other than
//! > `DOCUMENT_STATED`.
//!
//! Nothing here interprets document text as an instruction. Prose reaches this module as bytes,
//! becomes an entity name and a byte span, and stops. There is no LLM in the product path, no
//! rendering, no link following, and in Slice 5a no resolution of any kind.

use std::collections::BTreeMap;

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::{Directness, EvidenceSourceType};

use crate::markdown::{self, DocumentScan, HeadingStyle, MAX_RAW_STATUS_BYTES};

/// Extractor identity, recorded on every observation and on its `extractor_run` row.
pub const EXTRACTOR_ID: &str = "md-structural";

/// Extractor version. A change here re-extracts every document, by design.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The only evidence source type this extractor may emit (ADR-0003, THREAT-MODEL.md T7).
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::DocumentStated];

/// How directly a document states the structure this extractor reads out of it.
///
/// `Direct`: the file literally contains the heading whose span the observation cites.
pub const DIRECTNESS: Directness = Directness::Direct;

/// Directory names that make every Markdown file inside them an ADR.
pub const ADR_DIRECTORIES: [&str; 3] = ["decisions", "adr", "adrs"];

/// The closed ADR status vocabulary. An unrecognised value is never coerced into one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdrStatus {
    /// Proposed, not yet decided.
    Proposed,
    /// In force.
    Accepted,
    /// Considered and refused.
    Rejected,
    /// No longer recommended.
    Deprecated,
    /// Replaced by a later decision.
    Superseded,
}

impl AdrStatus {
    /// Every status, in declaration order.
    pub const ALL: [AdrStatus; 5] = [
        AdrStatus::Proposed,
        AdrStatus::Accepted,
        AdrStatus::Rejected,
        AdrStatus::Deprecated,
        AdrStatus::Superseded,
    ];

    /// Canonical spelling recorded in entity metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            AdrStatus::Proposed => "Proposed",
            AdrStatus::Accepted => "Accepted",
            AdrStatus::Rejected => "Rejected",
            AdrStatus::Deprecated => "Deprecated",
            AdrStatus::Superseded => "Superseded",
        }
    }

    /// Parse a status word. Case-insensitive; anything else is not a status.
    pub fn parse(text: &str) -> Option<AdrStatus> {
        let needle = text.trim();
        AdrStatus::ALL
            .into_iter()
            .find(|status| status.as_str().eq_ignore_ascii_case(needle))
    }
}

/// The value recorded for a status outside the vocabulary.
///
/// Recorded as a value rather than dropped, for the same reason an unresolved reference is an
/// entity rather than an omission: "the document says something we do not model" is information,
/// and coercing it to `Proposed` would invent a decision nobody made.
pub const STATUS_UNPARSED: &str = "unparsed";

/// What a document says about being an ADR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdrFacts {
    /// Whether the document is an ADR at all.
    pub is_adr: bool,
    /// `ADR-<digits>` read from the file name, when the file name carries one.
    pub id: Option<String>,
    /// The parsed status, when it is in the closed vocabulary.
    pub status: Option<AdrStatus>,
    /// The raw status text, kept only when it could not be parsed and is short enough to cite.
    pub status_raw: Option<String>,
    /// 1-based line the status was read from.
    pub status_line: Option<usize>,
    /// Which of the two recognised forms carried the status.
    pub status_form: Option<&'static str>,
    /// True when a status was found but was too long to be a status word.
    pub status_refused_as_too_long: bool,
}

impl AdrFacts {
    /// The value recorded in the document entity's `meta`.
    ///
    /// `None` for a document that is not an ADR, `Some("unparsed")` for an ADR whose status text
    /// is outside the vocabulary, `Some(canonical)` otherwise. A recognised ADR with no status
    /// line at all records `None` — absent is not the same as unreadable.
    pub fn status_value(&self) -> Option<&str> {
        if !self.is_adr {
            return None;
        }
        match self.status {
            Some(status) => Some(status.as_str()),
            None if self.status_raw.is_some() || self.status_refused_as_too_long => {
                Some(STATUS_UNPARSED)
            }
            None => None,
        }
    }
}

/// One section of a document, with the identity it will carry into the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDef {
    /// Content-derived entity id.
    pub entity_id: String,
    /// Heading text exactly as written. Repository content; inert.
    pub name: String,
    /// Heading chain from the outermost enclosing heading down to this one, `>`-joined.
    ///
    /// **Display only.** Identity spreads the chain across tuple fields, because `>` is legal
    /// heading text and a joined field is forgeable (`nerve_core::ids::section_id`).
    pub heading_path: String,
    /// Heading level, 1 to 6.
    pub level: usize,
    /// Position among the siblings sharing this section's parent.
    pub sibling_ordinal: u32,
    /// The heading line, which is the evidence for the containment edge.
    pub heading_span: Span,
    /// The whole section, which is where the entity is.
    pub section_span: Span,
    /// Entity id of the enclosing section, or `None` when the document itself encloses it.
    pub parent: Option<String>,
    /// How the heading was written.
    pub style: HeadingStyle,
}

/// Everything `md-structural` read out of one document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentExtraction {
    /// Repository-relative path.
    pub rel_path: String,
    /// Document entity id.
    pub entity_id: String,
    /// Sections in source order.
    pub sections: Vec<SectionDef>,
    /// ADR recognition and status.
    pub adr: AdrFacts,
    /// Front matter span, when the document opens with one.
    pub front_matter: Option<Span>,
    /// Constructs the scanner refused, by form tag.
    pub unsupported: BTreeMap<String, usize>,
    /// The whole file, which is where the document entity is.
    pub file_span: Span,
}

fn last_segment(rel_path: &str) -> &str {
    match rel_path.rfind('/') {
        Some(index) => &rel_path[index + 1..],
        None => rel_path,
    }
}

fn parent_directory(rel_path: &str) -> Option<&str> {
    rel_path.rfind('/').map(|index| &rel_path[..index])
}

fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(index) if index > 0 => &name[..index],
        _ => name,
    }
}

/// `ADR-<digits>` read from the start of a file name, case-insensitively.
///
/// Returned in canonical upper case with the digits exactly as written, so `adr-0006-x.md` and
/// `ADR-0006-x.md` produce the same id and `ADR-6` and `ADR-0006` do not.
fn adr_id_from_name(name: &str) -> Option<String> {
    let stem = file_stem(name);
    let rest = stem.strip_prefix("ADR-").or_else(|| {
        stem.get(..4)
            .filter(|prefix| prefix.eq_ignore_ascii_case("adr-"))
            .map(|_| &stem[4..])
    })?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    Some(format!("ADR-{digits}"))
}

fn is_in_adr_directory(rel_path: &str) -> bool {
    let Some(parent) = parent_directory(rel_path) else {
        return false;
    };
    let directory = last_segment(parent);
    ADR_DIRECTORIES
        .iter()
        .any(|candidate| directory.eq_ignore_ascii_case(candidate))
}

/// The status value carried by a `**Status:** <word>` line.
///
/// Nerve's own ADRs write `**Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 3b`, so the
/// value ends at the first `·` as well as at the end of the line.
fn status_from_header_line(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("**Status:**")?;
    let value = rest.split('·').next().unwrap_or(rest).trim();
    let value = value.trim_end_matches('*').trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Record a status value, refusing one too long to be a status word.
fn record_status(facts: &mut AdrFacts, raw: &str, line: usize, form: &'static str) {
    facts.status_line = Some(line);
    facts.status_form = Some(form);
    if raw.len() > MAX_RAW_STATUS_BYTES {
        facts.status_refused_as_too_long = true;
        return;
    }
    match AdrStatus::parse(raw) {
        Some(status) => facts.status = Some(status),
        None => facts.status_raw = Some(raw.to_string()),
    }
}

/// Recognise an ADR and read its status.
///
/// Recognition is deterministic and closed: the file name carries `ADR-<digits>`, or the file
/// sits directly in a `decisions` / `adr` / `adrs` directory. Status comes from the first of the
/// two forms that matches, both of which occur in this repository.
fn read_adr(rel_path: &str, source: &str, scan: &DocumentScan) -> AdrFacts {
    let name = last_segment(rel_path);
    let id = adr_id_from_name(name);
    let is_adr = id.is_some() || is_in_adr_directory(rel_path);
    let mut facts = AdrFacts {
        is_adr,
        id,
        ..AdrFacts::default()
    };
    if !is_adr {
        return facts;
    }

    // Form 1: a `**Status:**` line in the header block, before the first level-2 heading.
    let header_end = scan
        .headings
        .iter()
        .find(|heading| heading.level >= 2)
        .map(|heading| heading.heading_span.start_byte)
        .unwrap_or(source.len());
    for (line_number, line) in (1usize..).zip(source[..header_end].split('\n')) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(value) = status_from_header_line(line) {
            record_status(&mut facts, value, line_number, "header-line");
            return facts;
        }
    }

    // Form 2: the first non-empty line of a `## Status` section.
    let Some(section) = scan
        .headings
        .iter()
        .find(|heading| heading.text.trim().eq_ignore_ascii_case("Status"))
    else {
        return facts;
    };
    let body_start = section.heading_span.end_byte.min(source.len());
    let body_end = section.section_span.end_byte.min(source.len());
    if body_start >= body_end {
        return facts;
    }
    for (number, line) in
        (section.heading_span.end_line..).zip(source[body_start..body_end].split('\n'))
    {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.trim().is_empty() {
            record_status(&mut facts, line.trim(), number, "status-section");
            return facts;
        }
    }
    facts
}

/// Turn the scanner's flat heading list into a nested section tree with stable identities.
///
/// Nesting is by heading level: a heading enters the section of the nearest preceding heading
/// with a strictly lower level. A document whose first heading is level 3 therefore has a
/// level-3 section directly under the document, which is what CommonMark's structure says and
/// what the plan's acceptance criterion 3 requires.
fn build_sections(project_id: &str, rel_path: &str, scan: &DocumentScan) -> Vec<SectionDef> {
    // Open ancestors: (level, entity_id, heading text, per-parent sibling counts).
    let mut stack: Vec<(usize, String, String)> = Vec::new();
    let mut ordinals: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut sections = Vec::with_capacity(scan.headings.len());

    for heading in &scan.headings {
        while stack
            .last()
            .is_some_and(|(level, _, _)| *level >= heading.level)
        {
            stack.pop();
        }
        let parent = stack.last().map(|(_, id, _)| id.clone());
        let parent_key = parent.clone().unwrap_or_default();

        let mut chain: Vec<&str> = stack.iter().map(|(_, _, text)| text.as_str()).collect();
        chain.push(heading.text.as_str());

        let ordinal = ordinals
            .entry((parent_key, heading.text.clone()))
            .or_insert(0);
        let sibling_ordinal = *ordinal;
        *ordinal += 1;

        let entity_id = ids::section_id(project_id, rel_path, &chain, sibling_ordinal);
        sections.push(SectionDef {
            entity_id: entity_id.clone(),
            name: heading.text.clone(),
            heading_path: chain.join(">"),
            level: heading.level,
            sibling_ordinal,
            heading_span: heading.heading_span,
            section_span: heading.section_span,
            parent,
            style: heading.style,
        });
        stack.push((heading.level, entity_id, heading.text.clone()));
    }

    sections
}

/// Scan one document and derive everything the graph builder needs from it.
///
/// Infallible by construction: a document Nerve cannot make sense of yields a document entity
/// with no sections and a set of counters saying why, never an error and never a panic.
pub fn extract_document(project_id: &str, rel_path: &str, source: &str) -> DocumentExtraction {
    let scan = markdown::scan(source);
    let line_count = scan.line_count.max(1);
    DocumentExtraction {
        rel_path: rel_path.to_string(),
        entity_id: ids::document_id(project_id, rel_path),
        sections: build_sections(project_id, rel_path, &scan),
        adr: read_adr(rel_path, source, &scan),
        front_matter: scan.front_matter,
        unsupported: scan.counters.unsupported.clone(),
        file_span: Span {
            start_byte: 0,
            end_byte: source.len(),
            start_line: 1,
            start_col: 0,
            end_line: line_count,
            end_col: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PID: &str = "00000000000000000000000000000001";

    fn extract(rel_path: &str, source: &str) -> DocumentExtraction {
        extract_document(PID, rel_path, source)
    }

    #[test]
    fn sections_nest_by_heading_level() {
        let source = "# Top\n\n## A\n\n### A1\n\n## B\n\n# Second\n";
        let extraction = extract("docs/a.md", source);
        let names: Vec<(&str, usize)> = extraction
            .sections
            .iter()
            .map(|s| (s.name.as_str(), s.level))
            .collect();
        assert_eq!(
            names,
            vec![("Top", 1), ("A", 2), ("A1", 3), ("B", 2), ("Second", 1)]
        );

        let by_name = |name: &str| {
            extraction
                .sections
                .iter()
                .find(|s| s.name == name)
                .expect(name)
        };
        assert!(by_name("Top").parent.is_none());
        assert_eq!(
            by_name("A").parent.as_deref(),
            Some(by_name("Top").entity_id.as_str())
        );
        assert_eq!(
            by_name("A1").parent.as_deref(),
            Some(by_name("A").entity_id.as_str())
        );
        assert_eq!(
            by_name("B").parent.as_deref(),
            Some(by_name("Top").entity_id.as_str())
        );
        assert!(by_name("Second").parent.is_none());
        assert_eq!(by_name("A1").heading_path, "Top>A>A1");
    }

    #[test]
    fn a_document_whose_first_heading_is_level_three_hangs_it_off_the_document() {
        let extraction = extract("docs/a.md", "### Deep\n\n#### Deeper\n");
        assert!(extraction.sections[0].parent.is_none());
        assert_eq!(extraction.sections[0].level, 3);
        assert_eq!(
            extraction.sections[1].parent.as_deref(),
            Some(extraction.sections[0].entity_id.as_str())
        );
    }

    /// Two siblings with identical heading text are two sections, not one.
    #[test]
    fn repeated_sibling_headings_get_distinct_ordinals_and_ids() {
        let extraction = extract("docs/a.md", "# Top\n\n## Notes\n\n## Notes\n");
        let notes: Vec<&SectionDef> = extraction
            .sections
            .iter()
            .filter(|s| s.name == "Notes")
            .collect();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].sibling_ordinal, 0);
        assert_eq!(notes[1].sibling_ordinal, 1);
        assert_ne!(notes[0].entity_id, notes[1].entity_id);
    }

    /// The ordinal is scoped to the parent, so an insertion elsewhere does not churn ids.
    #[test]
    fn ordinals_are_scoped_to_the_parent() {
        let before = extract("docs/a.md", "# A\n\n## Notes\n\n# B\n\n## Notes\n");
        let after = extract(
            "docs/a.md",
            "# A\n\n## Notes\n\n## Extra\n\n# B\n\n## Notes\n",
        );
        let last_before = before.sections.last().unwrap();
        let last_after = after.sections.last().unwrap();
        assert_eq!(last_before.name, "Notes");
        assert_eq!(last_after.name, "Notes");
        assert_eq!(
            last_before.entity_id, last_after.entity_id,
            "inserting a section under another parent must not renumber this one"
        );
    }

    /// The hostile case. A `0x1f` in one heading must not let it claim another's identity.
    #[test]
    fn a_control_character_in_a_heading_cannot_collide_two_sections() {
        let extraction = extract(
            "docs/a.md",
            "# Parent\n\n## Child\n\n# Parent\u{1f}Child\n\n# Plain\n",
        );
        let ids: Vec<&str> = extraction
            .sections
            .iter()
            .map(|s| s.entity_id.as_str())
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "two sections collided: {ids:?}");
    }

    #[test]
    fn an_angle_bracket_in_a_heading_cannot_collide_two_sections() {
        let extraction = extract("docs/a.md", "# A>B\n\n## C\n\n# A\n\n## B\n\n### C\n");
        let ids: Vec<&str> = extraction
            .sections
            .iter()
            .map(|s| s.entity_id.as_str())
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "two sections collided: {ids:?}");
    }

    #[test]
    fn adr_recognition_by_file_name_is_case_insensitive() {
        for name in [
            "docs/decisions/ADR-0006-state.md",
            "docs/adr-0006-state.md",
            "notes/Adr-12.md",
        ] {
            let extraction = extract(name, "# Title\n");
            assert!(extraction.adr.is_adr, "{name} should be an ADR");
            assert!(extraction.adr.id.is_some());
        }
        assert_eq!(
            extract("docs/adr-0006-state.md", "# T\n").adr.id.as_deref(),
            Some("ADR-0006")
        );
        assert!(!extract("docs/README.md", "# T\n").adr.is_adr);
        assert!(
            !extract("docs/adrenaline.md", "# T\n").adr.is_adr,
            "a word starting with `adr` is not an ADR id"
        );
    }

    #[test]
    fn a_document_in_a_decisions_directory_is_an_adr_without_an_id() {
        for path in [
            "docs/decisions/note.md",
            "docs/adr/note.md",
            "docs/ADRS/note.md",
        ] {
            let extraction = extract(path, "# Title\n");
            assert!(extraction.adr.is_adr, "{path}");
            assert!(extraction.adr.id.is_none());
        }
        assert!(!extract("docs/decisions/deeper/note.md", "# T\n").adr.is_adr);
    }

    /// The exact shape Nerve's own ADRs use.
    #[test]
    fn the_header_status_line_is_read_the_way_this_repository_writes_it() {
        let source = "# ADR-0006 — Occurrence identity\n\n\
                      **Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 3b\n\
                      **Amends:** ADR-0002\n\n## Context\n\ntext\n";
        let facts = extract("docs/decisions/ADR-0006-x.md", source).adr;
        assert_eq!(facts.status, Some(AdrStatus::Accepted));
        assert_eq!(facts.status_value(), Some("Accepted"));
        assert_eq!(facts.status_line, Some(3));
        assert_eq!(facts.status_form, Some("header-line"));
    }

    #[test]
    fn a_status_section_supplies_the_status_when_no_header_line_does() {
        let source = "# ADR-0009 — Something\n\n## Status\n\nProposed\n\n## Context\n\ntext\n";
        let facts = extract("docs/decisions/ADR-0009-x.md", source).adr;
        assert_eq!(facts.status, Some(AdrStatus::Proposed));
        assert_eq!(facts.status_form, Some("status-section"));
        assert_eq!(facts.status_line, Some(5));
    }

    #[test]
    fn every_status_in_the_vocabulary_parses_and_nothing_else_does() {
        for status in AdrStatus::ALL {
            assert_eq!(AdrStatus::parse(status.as_str()), Some(status));
            assert_eq!(
                AdrStatus::parse(&status.as_str().to_lowercase()),
                Some(status)
            );
        }
        assert_eq!(AdrStatus::parse("Accepted-ish"), None);
        assert_eq!(AdrStatus::parse(""), None);
    }

    #[test]
    fn an_unrecognised_status_is_recorded_unparsed_with_its_raw_text() {
        let source = "# ADR-0010\n\n**Status:** Mostly fine, probably\n\n## Context\n";
        let facts = extract("docs/decisions/ADR-0010-x.md", source).adr;
        assert_eq!(facts.status, None);
        assert_eq!(facts.status_raw.as_deref(), Some("Mostly fine, probably"));
        assert_eq!(facts.status_value(), Some(STATUS_UNPARSED));
    }

    #[test]
    fn an_absurdly_long_status_is_refused_rather_than_stored() {
        let long = "x".repeat(MAX_RAW_STATUS_BYTES + 1);
        let source = format!("# ADR-0011\n\n**Status:** {long}\n\n## Context\n");
        let facts = extract("docs/decisions/ADR-0011-x.md", &source).adr;
        assert!(facts.status_refused_as_too_long);
        assert_eq!(facts.status_raw, None);
        assert_eq!(facts.status_value(), Some(STATUS_UNPARSED));
    }

    #[test]
    fn an_adr_with_no_status_records_no_status_rather_than_guessing_one() {
        let facts = extract("docs/decisions/ADR-0012-x.md", "# ADR-0012\n\ntext\n").adr;
        assert_eq!(facts.status_value(), None);
        assert!(!facts.status_refused_as_too_long);
    }

    #[test]
    fn a_document_that_is_not_an_adr_has_no_status_even_if_it_says_status() {
        let facts = extract("docs/README.md", "# R\n\n**Status:** Accepted\n").adr;
        assert!(!facts.is_adr);
        assert_eq!(facts.status_value(), None);
    }

    #[test]
    fn an_empty_document_extracts_cleanly() {
        let extraction = extract("docs/empty.md", "");
        assert!(extraction.sections.is_empty());
        assert!(extraction.unsupported.is_empty());
        assert_eq!(extraction.file_span.start_byte, 0);
        assert_eq!(extraction.file_span.end_byte, 0);
    }

    #[test]
    fn declared_source_types_are_document_stated_and_nothing_else() {
        assert_eq!(DECLARED_SOURCE_TYPES, [EvidenceSourceType::DocumentStated]);
    }
}
