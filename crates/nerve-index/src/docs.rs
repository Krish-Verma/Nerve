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
//! rendering, and **no link is ever followed**: Slice 5c resolves a destination against the set
//! of indexed paths ([`crate::docref`]), which is a set-membership test, not an access.
//!
//! This module stays a **pure function of one file**. It records the destinations a document
//! wrote and where they were; deciding what they name needs the whole repository, so that
//! decision lives in [`crate::pipeline`], which is the only place that has it.

use std::collections::BTreeMap;

use nerve_core::ids;
use nerve_core::model::Span;
use nerve_core::vocab::{Directness, EvidenceSourceType};

use crate::markdown::{
    self, form, DocumentScan, HeadingStyle, RawLink, MAX_RAW_STATUS_BYTES,
    MAX_SUPERSESSION_TARGET_BYTES,
};

/// Extractor identity, recorded on every observation and on its `extractor_run` row.
pub const EXTRACTOR_ID: &str = "md-structural";

/// Extractor version. A change here re-extracts every document, by design.
///
/// `1.1.0` is Slice 5c: the same structure as `1.0.0`, plus `REFERENCES` edges for resolved
/// links. Output over identical bytes changed, so the version had to move with it — that is the
/// whole contract of the field, and it is what makes every document re-scan on the first run of
/// this build rather than keep a graph the current rules would not produce.
///
/// `1.2.0` is Slice 5d-ii: `SUPERSEDES` edges read out of the four explicit supersession fields.
/// The same argument applies — a document whose bytes never changed can now produce an edge it
/// did not produce before, so every document must be re-scanned once on this build.
pub const EXTRACTOR_VERSION: &str = "1.2.0";

/// The only evidence source type this extractor may emit (ADR-0003, THREAT-MODEL.md T7).
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::DocumentStated];

/// How directly a document states the structure this extractor reads out of it.
///
/// `Direct`: the file literally contains the heading whose span the observation cites.
pub const DIRECTNESS: Directness = Directness::Direct;

/// How directly a document states a claim that **link resolution** produced.
///
/// `Resolved`, on the same reading of ADR-0003 that gives an import `AST_RESOLVED`: the document
/// wrote `./util.ts#L13`, and a resolution step turned that into an entity. The *source type*
/// stays `DOCUMENT_STATED` — a resolved link is still only a document's claim — so
/// [`DECLARED_SOURCE_TYPES`] is unchanged and the THREAT-MODEL.md T7 query still has no
/// exceptions. Directness is the axis that says a step happened; source type is the axis that
/// says who said it, and only the first of those moved.
pub const RESOLVED_DIRECTNESS: Directness = Directness::Resolved;

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

/// Which way a supersession field points.
///
/// Both normalise to the **same** stored edge — `A SUPERSEDES B` means A replaces B — with the
/// endpoints swapped for [`SupersessionDirection::SupersededBy`]. The relation is stored one way
/// only, so a reverse lookup is a query rather than a second edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupersessionDirection {
    /// `**Supersedes:** <target>` — this document replaces `<target>`.
    Supersedes,
    /// `**Superseded by:** <target>` — `<target>` replaces this document.
    SupersededBy,
}

impl SupersessionDirection {
    /// Value recorded in observation details and in the extraction cache.
    pub fn as_str(self) -> &'static str {
        match self {
            SupersessionDirection::Supersedes => "supersedes",
            SupersessionDirection::SupersededBy => "superseded_by",
        }
    }
}

/// The two field labels, and the two section headings that carry the same meaning.
///
/// Closed. Nothing else is evidence of supersession — not a `Superseded` status with no target,
/// not adjacency of ADR numbers, not similarity of subject, not prose containing the word.
const SUPERSESSION_LABELS: [(&str, &str, SupersessionDirection); 2] = [
    (
        "**Supersedes:**",
        "Supersedes",
        SupersessionDirection::Supersedes,
    ),
    (
        "**Superseded by:**",
        "Superseded by",
        SupersessionDirection::SupersededBy,
    ),
];

/// Which of the two recognised places carried a supersession field.
pub mod supersession_form {
    /// A `**Supersedes:**` / `**Superseded by:**` field in the header block.
    pub const HEADER_LINE: &str = "header-line";
    /// The first non-empty line of a `## Supersedes` / `## Superseded by` section.
    pub const SECTION: &str = "supersession-section";
}

/// One supersession statement a document wrote, **uninterpreted**.
///
/// What the target *names* depends on every other file in the repository, so it is decided in
/// [`crate::docref`] and not here. This type records what the document said and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersessionStatement {
    /// Which of the two fields carried it.
    pub direction: SupersessionDirection,
    /// Which of the two places it was written in. See [`supersession_form`].
    pub form: &'static str,
    /// The field value exactly as written, trimmed. Repository content; inert.
    ///
    /// Empty when the field was present with nothing after it, and empty when the value was
    /// longer than [`MAX_SUPERSESSION_TARGET_BYTES`]. Both resolve to `unparsed`; the second is
    /// additionally counted under [`form::SUPERSESSION_TARGET_TOO_LONG`].
    pub raw_target: String,
    /// The destination of the Markdown link the value consists of, when it consists of exactly
    /// one.
    ///
    /// Taken from the link the scanner already recorded rather than re-parsed, so link syntax has
    /// one definition. `None` when the value is a bare identifier, is empty, or contains more
    /// than one link — a value naming two destinations names neither unambiguously.
    pub link_destination: Option<String>,
    /// The line the field was written on. This is the citation the observation carries.
    pub span: Span,
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
    /// Supersession fields the document wrote, in source order, uninterpreted.
    pub supersession: Vec<SupersessionStatement>,
    /// Front matter span, when the document opens with one.
    pub front_matter: Option<Span>,
    /// Link destinations the document wrote, in source order, uninterpreted.
    ///
    /// Uninterpreted here on purpose: what a destination *names* depends on every other file in
    /// the repository, and this function is given one file.
    pub links: Vec<RawLink>,
    /// Inline code spans that look like a bare identifier.
    ///
    /// Counted and never emitted. `parseConfig` in prose is not evidence that the document means
    /// the symbol `parseConfig`; that is name matching, which ADR-0002 refuses as a basis for
    /// identity. The count exists so that "Nerve saw this and declined" is visible rather than
    /// indistinguishable from "Nerve never looked".
    pub code_span_mentions: usize,
    /// Constructs the scanner refused, by form tag.
    pub unsupported: BTreeMap<String, usize>,
    /// The whole file, which is where the document entity is.
    pub file_span: Span,
}

impl DocumentExtraction {
    /// The innermost section whose span covers `byte`, or `None` when nothing does.
    ///
    /// `None` means the **document** is the container: a link written before the first heading
    /// belongs to no section, and inventing one for it would put an entity in the graph that the
    /// file does not contain.
    ///
    /// The answer cannot be ambiguous. Sections nest by construction and each one starts at its
    /// own heading, so no two share a start byte and any two that cover the same byte differ in
    /// length. The shortest covering span is therefore unique, and "innermost" is a total
    /// function rather than a tie-break on declaration order.
    pub fn owning_section(&self, byte: usize) -> Option<&SectionDef> {
        self.sections
            .iter()
            .filter(|section| {
                section.section_span.start_byte <= byte && byte < section.section_span.end_byte
            })
            .min_by_key(|section| {
                section
                    .section_span
                    .end_byte
                    .saturating_sub(section.section_span.start_byte)
            })
    }
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
pub fn adr_id_from_name(name: &str) -> Option<String> {
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

/// `ADR-<digits>` when the **whole** of `text` is one, in the same canonical spelling
/// [`adr_id_from_name`] produces.
///
/// Strict where the file-name reader is not: `ADR-0004` is an identifier, `ADR-0004 and others`
/// is prose. A supersession field's value is a target or it is unparsed, and taking a prefix of
/// it would be a guess about which of several things the author meant.
pub fn bare_adr_identifier(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let prefix = trimmed.get(..4)?;
    if !prefix.eq_ignore_ascii_case("adr-") {
        return None;
    }
    let digits = &trimmed[4..];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
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

/// End of the header block: the first level-2-or-deeper heading, or the end of the file.
///
/// The same rule the `**Status:**` reader uses, factored out so the two cannot drift: a field
/// beside the status on one line is in the header block exactly when the status is.
fn header_block_end(source: &str, scan: &DocumentScan) -> usize {
    scan.headings
        .iter()
        .find(|heading| heading.level >= 2)
        .map(|heading| heading.heading_span.start_byte)
        .unwrap_or(source.len())
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
    let header_end = header_block_end(source, scan);
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

/// `text` with `label` removed from its front, matched case-insensitively.
///
/// ASCII-only comparison, so `label.len()` is also the length of the match and the caller can use
/// it as a byte offset. `get` rather than slicing, so a multi-byte character straddling the
/// boundary is a non-match rather than a panic.
fn strip_field_label<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let prefix = text.get(..label.len())?;
    prefix
        .eq_ignore_ascii_case(label)
        .then(|| &text[label.len()..])
}

/// The `·`-separated fields of one header line, each with its byte offset within the line.
///
/// Nerve's own ADRs write `**Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 3b`, so a
/// `**Supersedes:**` field may sit beside other fields on one line and a value ends at the first
/// `·` as well as at the end of the line. That is the same rule [`status_from_header_line`]
/// applies; this splits so that the *second* field on a line can be read too.
fn header_fields(text: &str) -> Vec<(usize, &str)> {
    let mut fields = Vec::new();
    let mut offset = 0usize;
    for field in text.split('·') {
        fields.push((offset, field));
        offset += field.len() + '·'.len_utf8();
    }
    fields
}

/// Read one field value out of a line, returning it with its byte range within `text`.
///
/// The trailing `*` strip mirrors the status reader, so `**Supersedes:** ADR-1**` and
/// `**Supersedes:** ADR-1` are the same value.
fn supersession_field(text: &str, label: &str) -> Option<(usize, usize)> {
    for (field_offset, field) in header_fields(text) {
        let lead = field.len() - field.trim_start().len();
        let Some(rest) = strip_field_label(field.trim_start(), label) else {
            continue;
        };
        let rest_offset = field_offset + lead + label.len();
        let value_lead = rest.len() - rest.trim_start().len();
        let value = rest.trim().trim_end_matches('*').trim();
        let start = rest_offset + value_lead;
        return Some((start, start + value.len()));
    }
    None
}

/// Accumulates the supersession statements of one document while it is being read.
struct SupersessionScan<'a> {
    source: &'a str,
    links: &'a [RawLink],
    unsupported: &'a mut BTreeMap<String, usize>,
    statements: Vec<SupersessionStatement>,
}

impl SupersessionScan<'_> {
    /// Record one statement, refusing a value too long to be a target.
    fn push(
        &mut self,
        direction: SupersessionDirection,
        form_tag: &'static str,
        value: (usize, usize),
        line: markdown::ProseLine,
    ) {
        let raw = &self.source[value.0..value.1];
        let raw_target = if raw.len() > MAX_SUPERSESSION_TARGET_BYTES {
            *self
                .unsupported
                .entry(form::SUPERSESSION_TARGET_TOO_LONG.to_string())
                .or_insert(0) += 1;
            String::new()
        } else {
            raw.to_string()
        };

        // The link the scanner already recorded, when the value consists of exactly one. Two
        // destinations in one field name neither unambiguously, so the value stays a bare string
        // and resolves as unparsed.
        let mut inside = self
            .links
            .iter()
            .filter(|link| value.0 <= link.span.start_byte && link.span.start_byte < value.1);
        let link_destination = match (inside.next(), inside.next()) {
            (Some(link), None) if !raw_target.is_empty() => Some(link.destination.clone()),
            _ => None,
        };

        self.statements.push(SupersessionStatement {
            direction,
            form: form_tag,
            raw_target,
            link_destination,
            span: Span {
                start_byte: line.start,
                end_byte: line.end,
                start_line: line.number,
                start_col: 0,
                end_line: line.number,
                end_col: line.end - line.start,
            },
        });
    }
}

/// Read every supersession field a document wrote. Four forms, and nothing else.
///
/// | Form | Meaning |
/// |---|---|
/// | `**Supersedes:** <target>` in the header block | this document supersedes `<target>` |
/// | `**Superseded by:** <target>` in the header block | `<target>` supersedes this document |
/// | First non-empty line of a `## Supersedes` section | as above |
/// | First non-empty line of a `## Superseded by` section | as above |
///
/// Every line considered comes from [`DocumentScan::prose_lines`], which is exactly the set of
/// lines the block scan classified as prose. That is what suppresses a field written inside a
/// fenced code block: not a rule applied here, but a line this function is never shown.
///
/// Unlike the status reader this does **not** stop at the first match. A document that both
/// supersedes one decision and is superseded by another states two facts, and reading only the
/// first would silently drop one of them.
fn read_supersession(
    source: &str,
    scan: &DocumentScan,
    unsupported: &mut BTreeMap<String, usize>,
) -> Vec<SupersessionStatement> {
    let mut scanner = SupersessionScan {
        source,
        links: &scan.links,
        unsupported,
        statements: Vec::new(),
    };

    // Forms 1 and 2: fields in the header block, in source order.
    let header_end = header_block_end(source, scan);
    for line in scan
        .prose_lines
        .iter()
        .filter(|line| line.start < header_end)
    {
        let text = &source[line.start..line.end];
        for (label, _, direction) in SUPERSESSION_LABELS {
            let Some((start, end)) = supersession_field(text, label) else {
                continue;
            };
            scanner.push(
                direction,
                supersession_form::HEADER_LINE,
                (line.start + start, line.start + end),
                *line,
            );
        }
    }

    // Forms 3 and 4: the first non-empty prose line of a section named for the field.
    for (_, heading_text, direction) in SUPERSESSION_LABELS {
        let Some(section) = scan
            .headings
            .iter()
            .find(|heading| heading.text.trim().eq_ignore_ascii_case(heading_text))
        else {
            continue;
        };
        let body_start = section.heading_span.end_byte.min(source.len());
        let body_end = section.section_span.end_byte.min(source.len());
        let Some(line) = scan
            .prose_lines
            .iter()
            .find(|line| line.start >= body_start && line.end <= body_end)
        else {
            continue;
        };
        // A prose line is never blank by construction, so the first one found is the first
        // non-empty line of the section.
        let text = &source[line.start..line.end];
        let value = text.trim();
        let lead = text.len() - text.trim_start().len();
        scanner.push(
            direction,
            supersession_form::SECTION,
            (line.start + lead, line.start + lead + value.len()),
            *line,
        );
    }

    scanner.statements
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
    let mut unsupported = scan.counters.unsupported.clone();
    let supersession = read_supersession(source, &scan, &mut unsupported);
    DocumentExtraction {
        rel_path: rel_path.to_string(),
        entity_id: ids::document_id(project_id, rel_path),
        sections: build_sections(project_id, rel_path, &scan),
        adr: read_adr(rel_path, source, &scan),
        supersession,
        front_matter: scan.front_matter,
        links: scan.links.clone(),
        code_span_mentions: scan.code_span_mentions,
        unsupported,
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

    /// A link's source is the innermost section containing it, and the document when none does.
    #[test]
    fn a_link_belongs_to_the_innermost_section_containing_it() {
        let source = "Intro [a](./a.md)\n\n# Top\n\n[b](./b.md)\n\n## Child\n\n[c](./c.md)\n";
        let extraction = extract("docs/a.md", source);
        assert_eq!(extraction.links.len(), 3);

        let owner = |destination: &str| -> Option<String> {
            let link = extraction
                .links
                .iter()
                .find(|link| link.destination == destination)
                .expect(destination);
            extraction
                .owning_section(link.span.start_byte)
                .map(|section| section.name.clone())
        };
        assert_eq!(owner("./a.md"), None, "a link before the first heading");
        assert_eq!(owner("./b.md").as_deref(), Some("Top"));
        assert_eq!(
            owner("./c.md").as_deref(),
            Some("Child"),
            "the enclosing `Top` section covers it too; the innermost wins"
        );
    }

    fn supersession(source: &str) -> Vec<(&'static str, &'static str, String, Option<String>)> {
        extract("docs/decisions/ADR-0100-x.md", source)
            .supersession
            .into_iter()
            .map(|statement| {
                (
                    statement.direction.as_str(),
                    statement.form,
                    statement.raw_target,
                    statement.link_destination,
                )
            })
            .collect()
    }

    /// The exact shape the fixture corpus writes: the field sits beside the status on one line.
    #[test]
    fn a_supersedes_field_is_read_beside_other_fields_on_one_header_line() {
        assert_eq!(
            supersession("# T\n\n**Status:** Accepted · **Supersedes:** ADR-0004\n\ntext\n"),
            vec![(
                "supersedes",
                supersession_form::HEADER_LINE,
                "ADR-0004".to_string(),
                None
            )]
        );
        assert_eq!(
            supersession(
                "# T\n\n**Status:** Superseded · **Superseded by:** [ADR-2](ADR-2-x.md)\n"
            ),
            vec![(
                "superseded_by",
                supersession_form::HEADER_LINE,
                "[ADR-2](ADR-2-x.md)".to_string(),
                Some("ADR-2-x.md".to_string())
            )]
        );
    }

    /// A document may state both directions. Stopping at the first would drop one of them.
    #[test]
    fn both_directions_can_be_stated_by_one_document() {
        let read = supersession(
            "# T\n\n**Supersedes:** ADR-0001\n**Superseded by:** ADR-0003\n\n## Context\n",
        );
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].0, "supersedes");
        assert_eq!(read[1].0, "superseded_by");
    }

    #[test]
    fn a_section_supplies_the_target_when_no_header_field_does() {
        assert_eq!(
            supersession("# T\n\n**Status:** Superseded\n\n## Superseded by\n\nADR-0006\n"),
            vec![(
                "superseded_by",
                supersession_form::SECTION,
                "ADR-0006".to_string(),
                None
            )]
        );
        assert_eq!(
            supersession("# T\n\n## Supersedes\n\n[a](./a.md)\n")[0].3,
            Some("./a.md".to_string())
        );
    }

    /// The field is present and says nothing. That is a value, not an absence.
    #[test]
    fn an_empty_field_is_recorded_rather_than_skipped() {
        let read = supersession("# T\n\n**Status:** Accepted · **Supersedes:**\n\ntext\n");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].2, "");
        assert_eq!(read[0].3, None);
    }

    /// Block structure is resolved before this function runs, so a field inside a fence is a line
    /// it is never shown.
    #[test]
    fn a_field_inside_a_fenced_code_block_produces_no_statement() {
        let source = "# T\n\n**Status:** Accepted\n\n```markdown\n**Supersedes:** ADR-0001\n```\n";
        assert!(supersession(source).is_empty());
    }

    /// Prose containing the word, and a code span containing the field's text, are not fields.
    #[test]
    fn prose_and_a_code_span_are_not_evidence_of_supersession() {
        let source = "# T\n\n**Status:** Accepted\n\n\
                      This supersedes ADR-0001 in spirit, and `Supersedes: ADR-0001` is a span.\n";
        assert!(supersession(source).is_empty());
    }

    /// A field written after the header block is not a header field, and a section not named for
    /// one of the two labels is not a supersession section.
    #[test]
    fn a_field_outside_the_header_block_and_an_unrelated_section_produce_nothing() {
        let source = "# T\n\n## Context\n\n**Supersedes:** ADR-0001\n";
        assert!(supersession(source).is_empty());
    }

    #[test]
    fn a_value_too_long_to_be_a_target_is_refused_and_counted() {
        let long = "x".repeat(MAX_SUPERSESSION_TARGET_BYTES + 1);
        let source = format!("# T\n\n**Supersedes:** {long}\n");
        let extraction = extract("docs/decisions/ADR-0101-x.md", &source);
        assert_eq!(extraction.supersession.len(), 1);
        assert_eq!(extraction.supersession[0].raw_target, "");
        assert_eq!(
            extraction
                .unsupported
                .get(form::SUPERSESSION_TARGET_TOO_LONG),
            Some(&1)
        );
    }

    #[test]
    fn a_bare_identifier_is_the_whole_value_or_it_is_not_one() {
        assert_eq!(
            bare_adr_identifier("ADR-0004"),
            Some("ADR-0004".to_string())
        );
        assert_eq!(bare_adr_identifier(" adr-12 "), Some("ADR-12".to_string()));
        for text in [
            "ADR-0004 and others",
            "ADR-",
            "ADR",
            "",
            "the old note",
            "ADR-4x",
        ] {
            assert_eq!(bare_adr_identifier(text), None, "{text}");
        }
    }

    /// The scanner's block ordering, restated where the extractor consumes it: a destination
    /// inside a fence or a code span is not a link, so no attribution question arises.
    #[test]
    fn a_destination_inside_code_never_reaches_the_link_list() {
        let source = "# T\n\n```md\n[fenced](./fenced.md)\n```\n\n\
                      A `[span](./span.md)` in prose, and [real](./real.md).\n";
        let extraction = extract("docs/a.md", source);
        let destinations: Vec<&str> = extraction
            .links
            .iter()
            .map(|link| link.destination.as_str())
            .collect();
        assert_eq!(destinations, vec!["./real.md"]);
    }
}
