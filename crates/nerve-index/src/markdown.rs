//! A deliberately restricted Markdown **block scanner**.
//!
//! This is not a CommonMark implementation and does not want to be. It answers one question —
//! *where are this document's headings, and what encloses them* — and reports everything it
//! cannot answer as unsupported and counted, never guessed at. `docs/plans/slice-05-document-
//! evidence.md` §2.5 records why no parser crate was adopted: `tree-sitter-md 0.5.3` requires
//! `tree-sitter 0.26` against this workspace's `0.25`, and the alternative was either bumping a
//! frozen, precision-gated TS/JS extraction path for a documentation slice or carrying two
//! copies of a C runtime.
//!
//! # The supported subset
//!
//! - **ATX headings**, `#` to `######`, including the optional closing hash run.
//! - **Setext headings**, a single-line paragraph underlined with `===` or `---`.
//! - **Fenced code**, ``` and `~~~`, including an unterminated fence (which runs to EOF).
//! - **Indented code blocks**, four spaces or a tab, where an indented run may begin.
//! - **Inline code spans**, so that a `#` inside one is not read as a closing hash run.
//! - **YAML front matter**, `---` delimited, recognised at byte 0 only.
//!
//! # What is deliberately outside it
//!
//! Everything else is prose as far as structure is concerned, with one class of exception that
//! is counted rather than ignored: a construct that *can legally contain a heading* which this
//! scanner does not descend into. A heading inside a block quote or a list item is a heading in
//! CommonMark and is not one here, so those lines are counted under
//! [`ScanCounters::unsupported`] and the reader is told the number rather than left to assume
//! there was nothing there.
//!
//! # Order of operations
//!
//! CommonMark resolves **block structure before inline structure**, and this scanner follows it.
//! An inline code span therefore cannot hide a block-level construct: in
//!
//! ```text
//! text `code
//! # heading` more
//! ```
//!
//! the second line is an ATX heading in CommonMark and here too. Code spans are consulted only
//! where they change the reading of a heading's own text.
//!
//! # Bounds
//!
//! Every bound refuses and counts (`docs/plans/slice-05-document-evidence.md` §3.6). Nothing is
//! silently truncated: a document that hits a bound says so in its counters, and the counters
//! reach `nerve index`'s report.

use std::collections::BTreeMap;

use nerve_core::model::Span;

/// Deepest heading level CommonMark defines. `#######` is paragraph text, so section nesting is
/// bounded by construction and the scanner cannot recurse.
pub const MAX_HEADING_LEVEL: usize = 6;

/// Headings a single document may contribute. A 2 MiB file of `#` lines is about a million.
pub const MAX_HEADINGS_PER_DOCUMENT: usize = 10_000;

/// Lines of YAML front matter scanned before the delimiter search is abandoned.
pub const MAX_FRONT_MATTER_LINES: usize = 1_000;

/// Bytes of raw status text preserved for an ADR whose status is outside the vocabulary.
///
/// A status value is one word. Anything longer is not a status, and the whole line is
/// attacker-controlled, so it is refused rather than stored at whatever length it happens to be.
pub const MAX_RAW_STATUS_BYTES: usize = 200;

/// Link destinations a single document may contribute (Slice 5b).
///
/// A destination becomes a cache entry, and every cache entry is re-resolved on every run, so an
/// unbounded list would let one file make every subsequent index slower. The bound refuses and
/// counts, like every other bound here.
pub const MAX_LINKS_PER_DOCUMENT: usize = 10_000;

/// Bytes of a link destination the scanner is willing to carry.
///
/// A destination becomes an entity name and a cache key. `PATH_MAX` is 4096 on Linux and 1024 on
/// macOS; nothing longer can name a file, so anything longer is refused rather than stored.
pub const MAX_LINK_DESTINATION_BYTES: usize = 1024;

/// Bytes of inline code span content examined when counting bare code mentions.
///
/// A code span is prose, and prose is unbounded. Only short spans can be an identifier, so the
/// rest are not examined at all rather than scanned to their end.
pub const MAX_CODE_SPAN_MENTION_BYTES: usize = 128;

/// Form tags used in [`ScanCounters::unsupported`]. Closed, so a reader can enumerate them.
pub mod form {
    /// Seven or more `#` — not a heading in CommonMark, and not one here.
    pub const ATX_OVER_MAX_LEVEL: &str = "atx-over-six-hashes";
    /// A setext underline under a paragraph of more than one line.
    pub const SETEXT_MULTILINE_PARAGRAPH: &str = "setext-multiline-paragraph";
    /// A fence that no closing fence ever matched.
    pub const UNTERMINATED_FENCE: &str = "unterminated-fence";
    /// A leading `---` with no closing delimiter.
    pub const UNTERMINATED_FRONT_MATTER: &str = "unterminated-front-matter";
    /// Front matter longer than [`super::MAX_FRONT_MATTER_LINES`].
    pub const FRONT_MATTER_TOO_LONG: &str = "front-matter-lines-exceeded";
    /// Headings past [`super::MAX_HEADINGS_PER_DOCUMENT`].
    pub const HEADINGS_EXCEEDED: &str = "headings-exceeded";
    /// A heading inside a block quote. A heading in CommonMark; not descended into here.
    pub const HEADING_IN_BLOCK_QUOTE: &str = "heading-in-block-quote";
    /// A heading inside a list item. A heading in CommonMark; not descended into here.
    pub const HEADING_IN_LIST_ITEM: &str = "heading-in-list-item";
    /// A raw HTML block. Inert data here; never rendered, never executed.
    pub const HTML_BLOCK: &str = "html-block";
    /// Link destinations past [`super::MAX_LINKS_PER_DOCUMENT`].
    pub const LINKS_EXCEEDED: &str = "links-exceeded";
    /// A destination longer than [`super::MAX_LINK_DESTINATION_BYTES`].
    pub const LINK_DESTINATION_TOO_LONG: &str = "link-destination-too-long";
    /// A link or image nested inside another link's text, which is not descended into.
    ///
    /// `[![diagram](./d.png)](./page.md)` writes two destinations and this scanner records the
    /// outer one. Descending would mean recursing into attacker-controlled nesting, and a
    /// document of ten thousand nested brackets would then be a stack overflow. Counted rather
    /// than ignored, on the same principle as `heading-in-block-quote`: the reader is told the
    /// number instead of being left to assume there was nothing there.
    pub const LINK_IN_LINK_TEXT: &str = "link-in-link-text";
}

/// One heading the scanner is willing to vouch for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1 to [`MAX_HEADING_LEVEL`].
    pub level: usize,
    /// Heading text exactly as written, minus the markers. Repository content: inert data.
    pub text: String,
    /// The heading line itself — for setext, the text line and its underline.
    pub heading_span: Span,
    /// Heading through to the byte before the next heading of the same or a lower level.
    pub section_span: Span,
    /// How the heading was written.
    pub style: HeadingStyle,
}

/// How a heading was spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadingStyle {
    /// `# Title`
    Atx,
    /// `Title` underlined with `===` or `---`.
    Setext,
}

impl HeadingStyle {
    /// Canonical tag recorded in observation details.
    pub fn as_str(self) -> &'static str {
        match self {
            HeadingStyle::Atx => "atx",
            HeadingStyle::Setext => "setext",
        }
    }
}

/// How a link destination was written.
///
/// Recorded because the three forms are not equally strong evidence of intent, and because a
/// reader asking "why did Nerve think this was a link?" is owed the syntax it matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkForm {
    /// `[text](destination)`.
    Inline,
    /// `[label]: destination`, a link reference definition at the start of a block.
    ReferenceDefinition,
    /// `<destination>`.
    AngleBracket,
}

impl LinkForm {
    /// Canonical tag recorded in observation details.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkForm::Inline => "inline",
            LinkForm::ReferenceDefinition => "reference-definition",
            LinkForm::AngleBracket => "angle-bracket",
        }
    }
}

/// One link destination, exactly as the document wrote it.
///
/// The scanner **never** interprets a destination: it does not normalize it, resolve it, stat
/// it, open it or fetch it. It records the bytes between the delimiters and where they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLink {
    /// Destination as written, with backslash escapes removed. Repository content; inert.
    pub destination: String,
    /// Which syntax carried it.
    pub form: LinkForm,
    /// The whole link construct, so a citation points at the link and not at the line.
    pub span: Span,
}

/// What the scan refused, and how often.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanCounters {
    /// Constructs outside the supported subset, by form tag.
    pub unsupported: BTreeMap<String, usize>,
}

impl ScanCounters {
    fn count(&mut self, tag: &'static str) {
        *self.unsupported.entry(tag.to_string()).or_insert(0) += 1;
    }

    /// Total refusals across every form.
    pub fn total(&self) -> usize {
        self.unsupported.values().sum()
    }
}

/// The result of scanning one document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentScan {
    /// Headings in source order.
    pub headings: Vec<Heading>,
    /// The front-matter block, delimiters included, when one was recognised.
    pub front_matter: Option<Span>,
    /// Link destinations in source order. Never followed, never resolved here.
    pub links: Vec<RawLink>,
    /// Inline code spans whose content is a bare identifier.
    ///
    /// **Counted, never emitted.** A `` `parseConfig` `` in prose is not evidence that the
    /// document means the symbol `parseConfig`: the same three-word rule that forbids fuzzy
    /// name matching for identity forbids it here. Entity-ising every code span would also add
    /// thousands of nodes to a repository's graph in exchange for a guess. The count is
    /// reported so that "Nerve saw these and refused them" is visible rather than inferred.
    pub code_span_mentions: usize,
    /// Lines of the document. Reported so a bound can be seen to have been near.
    pub line_count: usize,
    /// What the scan refused.
    pub counters: ScanCounters,
}

/// A line of the source, with its byte range and its 1-based number.
struct Line<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    number: usize,
}

/// Split `source` into lines, handling LF, CRLF and a final line without a terminator.
///
/// `text` excludes the terminator; `end` is the offset one past the last non-terminator byte, so
/// a span built from it never includes a `\r` that the author did not write.
fn lines(source: &str) -> Vec<Line<'_>> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut number = 1usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            out.push(Line {
                text: &source[start..end],
                start,
                end,
                number,
            });
            number += 1;
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    if start < bytes.len() {
        out.push(Line {
            text: &source[start..bytes.len()],
            start,
            end: bytes.len(),
            number,
        });
    }
    out
}

/// Leading spaces, counting a tab as four. More than three means an indented context.
fn indent_of(text: &str) -> usize {
    let mut indent = 0usize;
    for ch in text.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => break,
        }
    }
    indent
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// A run of the same character, after up to three spaces of indent, and nothing else.
fn is_run_of(text: &str, ch: char, minimum: usize) -> bool {
    if indent_of(text) > 3 {
        return false;
    }
    let trimmed = text.trim();
    trimmed.len() >= minimum && trimmed.chars().all(|c| c == ch)
}

/// An opening or closing code fence: the fence character and the run length.
fn fence_of(text: &str) -> Option<(char, usize)> {
    if indent_of(text) > 3 {
        return None;
    }
    let trimmed = text.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == marker).count();
    if run < 3 {
        return None;
    }
    // A backtick fence's info string may not contain a backtick; that is the rule that keeps
    // `` `a` `` from opening a fence. A tilde fence has no such restriction.
    if marker == '`' && trimmed[run..].contains('`') {
        return None;
    }
    Some((marker, run))
}

/// True when `text` closes a fence opened with `marker` at length `open_run`.
fn closes_fence(text: &str, marker: char, open_run: usize) -> bool {
    match fence_of(text) {
        Some((found, run)) => {
            found == marker && run >= open_run && text.trim_start()[run..].trim().is_empty()
        }
        None => false,
    }
}

/// One inline code span within a line: the whole construct, and the content between the ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodeSpan {
    outer: (usize, usize),
    inner: (usize, usize),
}

/// Inline code spans within one line.
///
/// Only within a line: a code span may cross lines in CommonMark, but block structure is
/// resolved first, so a multi-line span can never change whether a line *is* a heading. This is
/// used on heading text, where the question is whether a `#` is a closing marker or content,
/// and on prose lines, where the question is whether a `[` opens a link or is code.
fn code_spans(text: &str) -> Vec<CodeSpan> {
    let bytes = text.as_bytes();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let start = index;
            while index < bytes.len() && bytes[index] == b'`' {
                index += 1;
            }
            runs.push((start, index - start));
        } else {
            index += 1;
        }
    }

    let mut spans = Vec::new();
    let mut position = 0usize;
    while position < runs.len() {
        let (open_at, open_len) = runs[position];
        match runs[position + 1..]
            .iter()
            .position(|(_, len)| *len == open_len)
        {
            Some(offset) => {
                let (close_at, close_len) = runs[position + 1 + offset];
                spans.push(CodeSpan {
                    outer: (open_at, close_at + close_len),
                    inner: (open_at + open_len, close_at),
                });
                position = position + 1 + offset + 1;
            }
            None => position += 1,
        }
    }
    spans
}

/// Byte ranges of inline code spans within one line, delimiters included.
fn code_span_ranges(text: &str) -> Vec<(usize, usize)> {
    code_spans(text)
        .into_iter()
        .map(|span| span.outer)
        .collect()
}

fn inside_any(ranges: &[(usize, usize)], offset: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| offset >= *start && offset < *end)
}

/// End of the inline code span covering `offset`, when one does.
fn code_span_end(ranges: &[(usize, usize)], offset: usize) -> Option<usize> {
    ranges
        .iter()
        .find(|(start, end)| offset >= *start && offset < *end)
        .map(|(_, end)| *end)
}

/// True when a code span's content is a bare identifier, and therefore a *code mention*.
///
/// Counted and never emitted. `parseConfig` in prose is not evidence that the document means the
/// symbol `parseConfig` — that is name matching, which ADR-0002 refuses as a basis for identity.
/// The predicate is deliberately narrow: an identifier, optionally with an empty call suffix.
fn is_bare_identifier(text: &str) -> bool {
    let trimmed = text.trim();
    let core = trimmed.strip_suffix("()").unwrap_or(trimmed);
    let mut chars = core.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Remove CommonMark backslash escapes: a `\` before ASCII punctuation is the punctuation.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.peek() {
                if next.is_ascii_punctuation() {
                    out.push(*next);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(ch);
    }
    out
}

fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// Index of the `]` closing the `[` at `open`, honouring nesting, escapes and code spans.
fn matching_bracket(text: &str, open: usize, code: &[(usize, usize)]) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = open;
    while index < bytes.len() {
        if let Some(end) = code_span_end(code, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => {
                depth += 1;
                index += 1;
            }
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Read `<destination>` starting at the `<` at `open`. Returns the destination and the index
/// one past the `>`.
///
/// `allow_space` separates the two things CommonMark spells the same way and treats differently:
///
/// - a **bracketed destination** — the `<…>` inside `[t](<…>)` or after `[id]:` — *may* contain
///   spaces, and is the only way to write a path that has one;
/// - an **autolink** — a bare `<…>` in running text — may not, which is what keeps
///   `<div id=x>` and `Vec<T, U>` out of the link set.
///
/// A nested `<` refuses in both. A **control byte is carried through**, deliberately, so that
/// the path guard reports it as a refusal rather than the scanner dropping it silently.
fn angle_destination(text: &str, open: usize, allow_space: bool) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'>' => {
                if index == open + 1 {
                    return None;
                }
                return Some((unescape(&text[open + 1..index]), index + 1));
            }
            b'<' => return None,
            byte if is_space(byte) && !allow_space => return None,
            b'\\' => index += 2,
            _ => index += 1,
        }
    }
    None
}

/// Read a bare destination: a run ending at whitespace or at an unbalanced `)`.
fn bare_destination(text: &str, from: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let start = from;
    let mut index = from;
    let mut depth = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                index += 1;
            }
            // A control byte is carried into the destination rather than truncated away: the
            // path guard is where a hostile path is refused, and a refusal it never sees is a
            // refusal nobody reports.
            byte if is_space(byte) => break,
            _ => index += 1,
        }
    }
    let end = index.min(bytes.len());
    if end == start {
        return None;
    }
    Some((unescape(&text[start..end]), end))
}

/// Skip an optional link title — `"…"`, `'…'` — returning the index after it.
fn skip_title(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = from;
    while index < bytes.len() && is_space(bytes[index]) {
        index += 1;
    }
    if index >= bytes.len() || !matches!(bytes[index], b'"' | b'\'') {
        return Some(index);
    }
    let quote = bytes[index];
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            index += 1;
            while index < bytes.len() && is_space(bytes[index]) {
                index += 1;
            }
            return Some(index);
        }
        index += 1;
    }
    None
}

/// Read `(destination "title")` starting at the `(` at `open`.
fn inline_destination(text: &str, open: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut index = open + 1;
    while index < bytes.len() && is_space(bytes[index]) {
        index += 1;
    }
    let (destination, after) = if index < bytes.len() && bytes[index] == b'<' {
        angle_destination(text, index, true)?
    } else if index < bytes.len() && bytes[index] == b')' {
        // `[text]()` names nothing. Not a reference site, so not a link and not a refusal.
        return None;
    } else {
        bare_destination(text, index)?
    };
    let after = skip_title(text, after)?;
    if after < bytes.len() && bytes[after] == b')' {
        Some((destination, after + 1))
    } else {
        None
    }
}

/// Read the destination of a link reference definition, `[label]: destination "title"`.
///
/// The rest of the line must be the destination and an optional title and nothing else. A line
/// that continues into prose is a paragraph beginning with a bracket, not a definition.
fn definition_destination(text: &str, from: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut index = from;
    while index < bytes.len() && is_space(bytes[index]) {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }
    let (destination, after) = if bytes[index] == b'<' {
        angle_destination(text, index, true)?
    } else {
        bare_destination(text, index)?
    };
    let after = skip_title(text, after)?;
    if after >= bytes.len() {
        Some((destination, bytes.len()))
    } else {
        None
    }
}

/// True when the `[` at `open` begins a block rather than sitting inside prose.
fn opens_a_block(text: &str, open: usize) -> bool {
    text[..open].chars().all(|c| c == ' ' || c == '\t') && indent_of(text) <= 3
}

/// The URI scheme a destination opens with: a letter, then letters, digits, `+`, `.` or `-`,
/// then a colon. CommonMark's own autolink rule.
fn scheme_of(text: &str) -> Option<&str> {
    let colon = text.find(':')?;
    let scheme = &text[..colon];
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
        return None;
    }
    Some(scheme)
}

/// True when an angle-bracketed run is a link rather than raw HTML or ordinary prose.
///
/// A bracketed `[text](dest)` is unambiguously a link whatever `dest` looks like, but `<…>` is
/// not: `<br/>`, `<Foo>` and `<div id=x>` are raw HTML and a generic parameter, and recording
/// any of them would fabricate a reference site the document never wrote. CommonMark's autolink
/// is an absolute URI, which is the first arm here.
///
/// An explicitly relative path — `./` or `../` — is accepted as well. It is not an autolink in
/// CommonMark, but it states the author's intent unambiguously and nothing else does.
///
/// A **root-relative** `</src/a.ts>` is deliberately **not** accepted, even though `/src/a.ts`
/// is a perfectly good repository path: `</div>` is the closing half of every HTML tag pair,
/// and the two forms are indistinguishable without knowing whether `div` names a directory.
/// Ambiguity is a refusal here for the same reason it is in move detection — a confident wrong
/// answer is the failure mode this product exists to avoid. A document that means the path can
/// still write `[text](/src/a.ts)`, which is unambiguous.
fn is_angle_link_destination(text: &str) -> bool {
    text.starts_with("./") || text.starts_with("../") || scheme_of(text).is_some()
}

/// Accumulates the link findings of one document while the block scan walks it.
struct LinkScan {
    links: Vec<RawLink>,
    code_span_mentions: usize,
    refused_links: usize,
}

impl LinkScan {
    fn new() -> LinkScan {
        LinkScan {
            links: Vec::new(),
            code_span_mentions: 0,
            refused_links: 0,
        }
    }

    fn push(
        &mut self,
        counters: &mut ScanCounters,
        destination: String,
        form: LinkForm,
        span: Span,
    ) {
        if destination.is_empty() {
            return;
        }
        if destination.len() > MAX_LINK_DESTINATION_BYTES {
            counters.count(form::LINK_DESTINATION_TOO_LONG);
            return;
        }
        if self.links.len() >= MAX_LINKS_PER_DOCUMENT {
            self.refused_links += 1;
            return;
        }
        self.links.push(RawLink {
            destination,
            form,
            span,
        });
    }
}

/// Read one **prose** line for link destinations and bare code mentions.
///
/// Called only where the block scan has already decided the line is prose — never inside a
/// fence, an indented code block or front matter. That ordering is the whole reason a link in a
/// fenced block produces nothing: it is not that the link parser rejects it, it is that the
/// link parser never sees it.
fn scan_line_links(
    text: &str,
    line_start: usize,
    line_number: usize,
    links: &mut LinkScan,
    counters: &mut ScanCounters,
) {
    let spans = code_spans(text);
    for span in &spans {
        let inner = &text[span.inner.0..span.inner.1];
        if inner.len() <= MAX_CODE_SPAN_MENTION_BYTES && is_bare_identifier(inner) {
            links.code_span_mentions += 1;
        }
    }
    let code: Vec<(usize, usize)> = spans.iter().map(|span| span.outer).collect();

    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if let Some(end) = code_span_end(&code, index) {
            index = end;
            continue;
        }
        let found = match bytes[index] {
            b'\\' => {
                index += 2;
                continue;
            }
            b'[' => {
                let Some(close) = matching_bracket(text, index, &code) else {
                    index += 1;
                    continue;
                };
                let after = close + 1;
                let parsed = if after < bytes.len() && bytes[after] == b'(' {
                    inline_destination(text, after).map(|(dest, end)| (dest, LinkForm::Inline, end))
                } else if after < bytes.len() && bytes[after] == b':' && opens_a_block(text, index)
                {
                    definition_destination(text, after + 1)
                        .map(|(dest, end)| (dest, LinkForm::ReferenceDefinition, end))
                } else {
                    None
                };
                match parsed {
                    Some(hit) => {
                        // The outer link is the link. A nested one is counted, not descended
                        // into — `](` is its signature, and this is a counter, not an emission.
                        if hit.1 == LinkForm::Inline && text[index + 1..close].contains("](") {
                            counters.count(form::LINK_IN_LINK_TEXT);
                        }
                        Some(hit)
                    }
                    None => {
                        index = close + 1;
                        continue;
                    }
                }
            }
            b'<' => match angle_destination(text, index, false) {
                Some((dest, end)) if is_angle_link_destination(&dest) => {
                    Some((dest, LinkForm::AngleBracket, end))
                }
                _ => {
                    index += 1;
                    continue;
                }
            },
            _ => {
                index += 1;
                continue;
            }
        };

        let (destination, form, end) = found.expect("continue covers every non-hit path");
        links.push(
            counters,
            destination,
            form,
            Span {
                start_byte: line_start + index,
                end_byte: line_start + end,
                start_line: line_number,
                start_col: index,
                end_line: line_number,
                end_col: end,
            },
        );
        index = end;
    }
}

/// Strip an ATX heading's optional closing hash run.
///
/// CommonMark requires the run to be preceded by a space or to be the entire content, which is
/// what keeps `# C#` intact. A run inside an inline code span is content, not a marker.
fn strip_closing_hashes(content: &str) -> &str {
    let trimmed = content.trim_end();
    if trimmed.is_empty() {
        return trimmed;
    }
    let spans = code_span_ranges(trimmed);
    let hashes = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| *c == '#')
        .count();
    if hashes == 0 {
        return trimmed;
    }
    let cut = trimmed.len() - hashes;
    if inside_any(&spans, cut) {
        return trimmed;
    }
    if cut == 0 {
        return "";
    }
    if !trimmed[..cut].ends_with([' ', '\t']) {
        return trimmed;
    }
    trimmed[..cut].trim_end()
}

/// Parse an ATX heading line into `(level, text)`.
fn atx_heading(text: &str) -> Option<(usize, &str)> {
    if indent_of(text) > 3 {
        return None;
    }
    let trimmed = text.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > MAX_HEADING_LEVEL {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((level, strip_closing_hashes(rest.trim_start())))
}

/// True when the line looks like an ATX heading with more hashes than CommonMark allows.
fn over_deep_atx(text: &str) -> bool {
    if indent_of(text) > 3 {
        return false;
    }
    let trimmed = text.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    level > MAX_HEADING_LEVEL
        && (trimmed[level..].is_empty() || trimmed[level..].starts_with([' ', '\t']))
}

/// A heading marker that this scanner does not descend into, if the line carries one.
fn nested_heading_form(text: &str) -> Option<&'static str> {
    if indent_of(text) > 3 {
        return None;
    }
    let trimmed = text.trim_start();
    if trimmed.starts_with('>') {
        let inner = trimmed.trim_start_matches(['>', ' ']);
        return atx_heading(inner).map(|_| form::HEADING_IN_BLOCK_QUOTE);
    }
    let mut chars = trimmed.char_indices();
    let (_, first) = chars.next()?;
    let marker_len = if first == '-' || first == '*' || first == '+' {
        1
    } else if first.is_ascii_digit() {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        let delimiter = trimmed[digits..].chars().next()?;
        if delimiter != '.' && delimiter != ')' {
            return None;
        }
        digits + 1
    } else {
        return None;
    };
    let rest = &trimmed[marker_len..];
    if !rest.starts_with(' ') {
        return None;
    }
    atx_heading(rest.trim_start()).map(|_| form::HEADING_IN_LIST_ITEM)
}

/// True when the line opens a raw HTML block.
fn opens_html_block(text: &str) -> bool {
    indent_of(text) <= 3 && text.trim_start().starts_with('<')
}

/// Front matter, when the document opens with one. Returns the span and the line after it.
///
/// Recognised **at byte 0 only**: a `---` anywhere else is a thematic break or a setext
/// underline, and treating it as front matter would let a document hide its own tail.
fn front_matter(lines: &[Line<'_>], counters: &mut ScanCounters) -> (Option<Span>, usize) {
    let Some(first) = lines.first() else {
        return (None, 0);
    };
    if first.start != 0 || first.text != "---" {
        return (None, 0);
    }
    for (index, line) in lines.iter().enumerate().skip(1) {
        if index > MAX_FRONT_MATTER_LINES {
            counters.count(form::FRONT_MATTER_TOO_LONG);
            return (None, 0);
        }
        if line.text == "---" || line.text == "..." {
            return (
                Some(Span {
                    start_byte: first.start,
                    end_byte: line.end,
                    start_line: first.number,
                    start_col: 0,
                    end_line: line.number,
                    end_col: line.end - line.start,
                }),
                index + 1,
            );
        }
    }
    // No delimiter closed it. Refusing is the conservative reading: swallowing the rest of the
    // file as front matter would make one stray `---` erase a document's whole structure.
    counters.count(form::UNTERMINATED_FRONT_MATTER);
    (None, 0)
}

/// Scan one document for its heading structure.
///
/// Never panics and never guesses. `source` is repository content and is treated as hostile
/// throughout: nothing here executes, interprets, resolves or renders any of it.
pub fn scan(source: &str) -> DocumentScan {
    let lines = lines(source);
    let mut counters = ScanCounters::default();
    let (front_matter_span, start_index) = front_matter(&lines, &mut counters);

    let mut headings: Vec<Heading> = Vec::new();
    let mut links = LinkScan::new();
    let mut refused_headings = 0usize;
    // The immediately preceding line, when it is a paragraph line that a setext underline could
    // convert into a heading, plus whether the paragraph it belongs to is longer than one line.
    let mut paragraph: Option<usize> = None;
    let mut paragraph_lines = 0usize;
    let mut fence: Option<(char, usize)> = None;
    let mut indented_code = false;

    let mut index = start_index;
    while index < lines.len() {
        let line = &lines[index];

        if let Some((marker, run)) = fence {
            if closes_fence(line.text, marker, run) {
                fence = None;
            }
            index += 1;
            continue;
        }

        if is_blank(line.text) {
            paragraph = None;
            paragraph_lines = 0;
            indented_code = false;
            index += 1;
            continue;
        }

        if indented_code {
            if indent_of(line.text) >= 4 {
                index += 1;
                continue;
            }
            indented_code = false;
        }

        // An indented code block may begin only where a paragraph is not already open.
        if paragraph.is_none() && indent_of(line.text) >= 4 {
            indented_code = true;
            index += 1;
            continue;
        }

        if let Some((marker, run)) = fence_of(line.text) {
            fence = Some((marker, run));
            paragraph = None;
            paragraph_lines = 0;
            index += 1;
            continue;
        }

        // A setext underline converts the *preceding* paragraph into a heading. It is refused
        // when that paragraph is more than one line: taking the last line alone would be a
        // guess about which line the author meant to be the heading.
        if let Some(previous) = paragraph {
            let level = if is_run_of(line.text, '=', 1) {
                Some(1)
            } else if is_run_of(line.text, '-', 1) {
                Some(2)
            } else {
                None
            };
            if let Some(level) = level {
                if paragraph_lines > 1 {
                    counters.count(form::SETEXT_MULTILINE_PARAGRAPH);
                } else {
                    let text_line = &lines[previous];
                    push_heading(
                        &mut headings,
                        &mut refused_headings,
                        Heading {
                            level,
                            text: text_line.text.trim().to_string(),
                            heading_span: Span {
                                start_byte: text_line.start,
                                end_byte: line.end,
                                start_line: text_line.number,
                                start_col: 0,
                                end_line: line.number,
                                end_col: line.end - line.start,
                            },
                            section_span: Span::NONE,
                            style: HeadingStyle::Setext,
                        },
                    );
                }
                paragraph = None;
                paragraph_lines = 0;
                index += 1;
                continue;
            }
        }

        if let Some((level, text)) = atx_heading(line.text) {
            // A heading line is prose too: `## See [the resolver](../src/resolve.rs)` is a link
            // the document really wrote, and the section it belongs to is the one it introduces.
            scan_line_links(
                line.text,
                line.start,
                line.number,
                &mut links,
                &mut counters,
            );
            push_heading(
                &mut headings,
                &mut refused_headings,
                Heading {
                    level,
                    text: text.to_string(),
                    heading_span: Span {
                        start_byte: line.start,
                        end_byte: line.end,
                        start_line: line.number,
                        start_col: 0,
                        end_line: line.number,
                        end_col: line.end - line.start,
                    },
                    section_span: Span::NONE,
                    style: HeadingStyle::Atx,
                },
            );
            paragraph = None;
            paragraph_lines = 0;
            index += 1;
            continue;
        }

        if over_deep_atx(line.text) {
            counters.count(form::ATX_OVER_MAX_LEVEL);
        }
        if let Some(tag) = nested_heading_form(line.text) {
            counters.count(tag);
        }
        if opens_html_block(line.text) {
            counters.count(form::HTML_BLOCK);
        }

        scan_line_links(
            line.text,
            line.start,
            line.number,
            &mut links,
            &mut counters,
        );

        paragraph = Some(index);
        paragraph_lines += 1;
        index += 1;
    }

    if fence.is_some() {
        counters.count(form::UNTERMINATED_FENCE);
    }
    for _ in 0..refused_headings {
        counters.count(form::HEADINGS_EXCEEDED);
    }
    for _ in 0..links.refused_links {
        counters.count(form::LINKS_EXCEEDED);
    }

    assign_section_spans(&mut headings, source.len(), lines.len().max(1));

    DocumentScan {
        headings,
        front_matter: front_matter_span,
        links: links.links,
        code_span_mentions: links.code_span_mentions,
        line_count: lines.len(),
        counters,
    }
}

/// Accept a heading, or refuse it because the document has already contributed its allowance.
fn push_heading(headings: &mut Vec<Heading>, refused: &mut usize, heading: Heading) {
    if headings.len() >= MAX_HEADINGS_PER_DOCUMENT {
        *refused += 1;
        return;
    }
    headings.push(heading);
}

/// A section runs from its heading to the byte before the next heading of the same or a lower
/// level, or to the end of the document.
///
/// The byte range and the line range must describe the same region. A section that runs to EOF
/// therefore ends on the document's last line, not on its own heading line — a span that claimed
/// bytes it did not claim lines for would make a citation say one thing and a snippet another.
fn assign_section_spans(headings: &mut [Heading], source_len: usize, last_line: usize) {
    for position in 0..headings.len() {
        let level = headings[position].level;
        let next = headings[position + 1..]
            .iter()
            .find(|later| later.level <= level)
            .map(|later| later.heading_span);
        let start = headings[position].heading_span;
        let (end_byte, end_line) = match next {
            Some(span) => (span.start_byte, span.start_line.saturating_sub(1)),
            None => (source_len, last_line),
        };
        headings[position].section_span = Span {
            start_byte: start.start_byte,
            end_byte: end_byte.max(start.end_byte),
            start_line: start.start_line,
            start_col: 0,
            end_line: end_line.max(start.end_line),
            end_col: 0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(scan: &DocumentScan) -> Vec<(usize, String)> {
        scan.headings
            .iter()
            .map(|h| (h.level, h.text.clone()))
            .collect()
    }

    fn levels(source: &str) -> Vec<(usize, String)> {
        texts(&scan(source))
    }

    #[test]
    fn atx_headings_of_every_level() {
        let source = "# One\n## Two\n### Three\n#### Four\n##### Five\n###### Six\n";
        assert_eq!(
            levels(source),
            vec![
                (1, "One".into()),
                (2, "Two".into()),
                (3, "Three".into()),
                (4, "Four".into()),
                (5, "Five".into()),
                (6, "Six".into()),
            ]
        );
    }

    #[test]
    fn seven_hashes_is_not_a_heading_and_is_counted() {
        let scanned = scan("####### Too deep\n");
        assert!(scanned.headings.is_empty());
        assert_eq!(scanned.counters.unsupported[form::ATX_OVER_MAX_LEVEL], 1);
    }

    #[test]
    fn a_lone_hash_is_an_empty_heading_and_a_hashtag_is_not_a_heading() {
        assert_eq!(levels("#\n"), vec![(1, String::new())]);
        assert_eq!(levels("#hashtag\n"), Vec::new());
    }

    #[test]
    fn the_closing_hash_run_is_stripped_only_when_commonmark_says_so() {
        assert_eq!(levels("# Title #\n"), vec![(1, "Title".into())]);
        assert_eq!(levels("## Title ##\n"), vec![(2, "Title".into())]);
        assert_eq!(levels("# C#\n"), vec![(1, "C#".into())]);
        assert_eq!(levels("# ###\n"), vec![(1, String::new())]);
    }

    /// A `#` inside an inline code span is content, not a closing marker.
    #[test]
    fn a_hash_inside_an_inline_code_span_is_not_a_closing_marker() {
        assert_eq!(levels("# Use `#` here\n"), vec![(1, "Use `#` here".into())]);
        assert_eq!(levels("# Trailing `#`\n"), vec![(1, "Trailing `#`".into())]);
        assert_eq!(
            levels("# Mixed `a # b` #\n"),
            vec![(1, "Mixed `a # b`".into())]
        );
    }

    #[test]
    fn setext_headings_are_recognised() {
        assert_eq!(levels("Title\n=====\n"), vec![(1, "Title".into())]);
        assert_eq!(levels("Title\n-----\n"), vec![(2, "Title".into())]);
        assert_eq!(levels("Title\n=\n"), vec![(1, "Title".into())]);
    }

    /// A `---` or `===` with a blank line before it underlines nothing.
    #[test]
    fn a_setext_underline_after_a_blank_line_is_not_a_heading() {
        assert!(scan("Title\n\n-----\n").headings.is_empty());
        assert!(scan("Title\n\n=====\n").headings.is_empty());
        assert!(scan("-----\n").headings.is_empty());
    }

    #[test]
    fn a_setext_underline_under_a_multiline_paragraph_is_refused_and_counted() {
        let scanned = scan("One line\nAnother line\n-----\n");
        assert!(scanned.headings.is_empty());
        assert_eq!(
            scanned.counters.unsupported[form::SETEXT_MULTILINE_PARAGRAPH],
            1
        );
    }

    #[test]
    fn text_inside_a_fenced_code_block_is_not_structure() {
        let source = "# Real\n\n```\n# Fake\nTitle\n=====\n```\n\n## Also real\n";
        assert_eq!(
            levels(source),
            vec![(1, "Real".into()), (2, "Also real".into())]
        );
    }

    #[test]
    fn tilde_fences_work_and_can_contain_backtick_fences() {
        let source = "~~~\n# Fake\n```\n~~~\n\n# Real\n";
        assert_eq!(levels(source), vec![(1, "Real".into())]);
    }

    #[test]
    fn an_unterminated_fence_runs_to_the_end_and_is_counted() {
        let scanned = scan("# Real\n\n```\n# Swallowed\n");
        assert_eq!(texts(&scanned), vec![(1, "Real".into())]);
        assert_eq!(scanned.counters.unsupported[form::UNTERMINATED_FENCE], 1);
    }

    #[test]
    fn a_longer_closing_fence_closes_a_shorter_one_but_not_the_reverse() {
        assert_eq!(
            levels("```\ncode\n`````\n\n# After\n"),
            vec![(1, "After".into())]
        );
        assert!(scan("`````\ncode\n```\n\n# After\n").headings.is_empty());
    }

    #[test]
    fn an_indented_code_block_is_not_structure() {
        let source = "# Real\n\n    # Indented\n    Title\n    =====\n\n## Also real\n";
        assert_eq!(
            levels(source),
            vec![(1, "Real".into()), (2, "Also real".into())]
        );
    }

    /// Four spaces cannot open a code block in the middle of a paragraph, so the heading that
    /// follows the paragraph is still a heading.
    #[test]
    fn an_indented_line_inside_a_paragraph_does_not_open_a_code_block() {
        let source = "prose\n    continued\n\n# Real\n";
        assert_eq!(levels(source), vec![(1, "Real".into())]);
    }

    #[test]
    fn front_matter_is_recognised_only_at_byte_zero() {
        let scanned = scan("---\ntitle: A\n---\n\n# Real\n");
        assert_eq!(texts(&scanned), vec![(1, "Real".into())]);
        let span = scanned.front_matter.expect("front matter");
        assert_eq!(span.start_byte, 0);
        assert_eq!(span.start_line, 1);
        assert_eq!(span.end_line, 3);

        // The same delimiters later in the file are not front matter.
        let later = scan("# Real\n\n---\ntitle: A\n---\n");
        assert!(later.front_matter.is_none());
        assert_eq!(texts(&later), vec![(1, "Real".into())]);
    }

    #[test]
    fn unterminated_front_matter_is_refused_and_the_document_is_still_scanned() {
        let scanned = scan("---\ntitle: A\n\n# Real\n");
        assert!(scanned.front_matter.is_none());
        assert_eq!(
            scanned.counters.unsupported[form::UNTERMINATED_FRONT_MATTER],
            1
        );
        assert_eq!(texts(&scanned), vec![(1, "Real".into())]);
    }

    #[test]
    fn front_matter_longer_than_the_bound_is_refused_and_counted() {
        let mut source = String::from("---\n");
        for line in 0..MAX_FRONT_MATTER_LINES + 5 {
            source.push_str(&format!("key{line}: value\n"));
        }
        source.push_str("---\n\n# Real\n");
        let scanned = scan(&source);
        assert!(scanned.front_matter.is_none());
        assert_eq!(scanned.counters.unsupported[form::FRONT_MATTER_TOO_LONG], 1);
    }

    #[test]
    fn crlf_and_mixed_line_endings_are_handled() {
        let scanned = scan("# One\r\n## Two\n### Three\r\n");
        assert_eq!(
            texts(&scanned),
            vec![(1, "One".into()), (2, "Two".into()), (3, "Three".into()),]
        );
        // No heading text may carry a stray carriage return into an entity name.
        assert!(scanned.headings.iter().all(|h| !h.text.contains('\r')));
        assert_eq!(levels("Title\r\n=====\r\n"), vec![(1, "Title".into())]);
    }

    #[test]
    fn an_empty_document_and_one_without_headings_scan_cleanly() {
        let empty = scan("");
        assert!(empty.headings.is_empty());
        assert_eq!(empty.line_count, 0);
        assert_eq!(empty.counters.total(), 0);

        let prose = scan("Just some prose.\n\nAnd more of it.\n");
        assert!(prose.headings.is_empty());
        assert_eq!(prose.counters.total(), 0);
    }

    #[test]
    fn a_document_whose_first_heading_is_level_three_keeps_its_level() {
        let scanned = scan("### Deep first\n\ntext\n\n#### Deeper\n\n## Shallower\n");
        assert_eq!(
            texts(&scanned),
            vec![
                (3, "Deep first".into()),
                (4, "Deeper".into()),
                (2, "Shallower".into()),
            ]
        );
    }

    #[test]
    fn headings_inside_block_quotes_and_list_items_are_counted_not_claimed() {
        let scanned = scan("> # Quoted\n\n- # Listed\n\n1. # Numbered\n");
        assert!(scanned.headings.is_empty());
        assert_eq!(
            scanned.counters.unsupported[form::HEADING_IN_BLOCK_QUOTE],
            1
        );
        assert_eq!(scanned.counters.unsupported[form::HEADING_IN_LIST_ITEM], 2);
    }

    #[test]
    fn raw_html_is_counted_and_never_becomes_structure() {
        let scanned = scan("<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\n# Real\n");
        assert_eq!(texts(&scanned), vec![(1, "Real".into())]);
        assert_eq!(scanned.counters.unsupported[form::HTML_BLOCK], 2);
    }

    #[test]
    fn the_heading_bound_refuses_the_excess_and_counts_it() {
        let mut source = String::new();
        for line in 0..MAX_HEADINGS_PER_DOCUMENT + 7 {
            source.push_str(&format!("# H{line}\n"));
        }
        let scanned = scan(&source);
        assert_eq!(scanned.headings.len(), MAX_HEADINGS_PER_DOCUMENT);
        assert_eq!(scanned.counters.unsupported[form::HEADINGS_EXCEEDED], 7);
    }

    #[test]
    fn section_spans_run_to_the_next_heading_of_the_same_or_a_lower_level() {
        let source = "# One\nbody\n\n## Two\nbody\n\n# Three\n";
        let scanned = scan(source);
        let one = &scanned.headings[0];
        let two = &scanned.headings[1];
        let three = &scanned.headings[2];
        assert_eq!(one.section_span.start_byte, 0);
        assert_eq!(one.section_span.end_byte, three.heading_span.start_byte);
        assert_eq!(two.section_span.end_byte, three.heading_span.start_byte);
        assert_eq!(three.section_span.end_byte, source.len());
        // The line range must describe the same region as the byte range.
        assert_eq!(one.section_span.start_line, 1);
        assert_eq!(one.section_span.end_line, 6, "the line before `# Three`");
        assert_eq!(
            three.section_span.end_line, scanned.line_count,
            "a section running to EOF ends on the document's last line"
        );
        // Every span must lie inside the source and be well ordered.
        for heading in &scanned.headings {
            assert!(heading.section_span.start_byte <= heading.section_span.end_byte);
            assert!(heading.section_span.end_byte <= source.len());
            assert!(heading.section_span.start_line <= heading.section_span.end_line);
            assert!(heading.section_span.end_line <= scanned.line_count);
            assert!(source.is_char_boundary(heading.section_span.start_byte));
            assert!(source.is_char_boundary(heading.section_span.end_byte));
        }
    }

    /// Block structure is resolved before inline structure, so a code span cannot hide a
    /// heading. Pinned so the behaviour is a decision rather than an accident.
    #[test]
    fn an_inline_code_span_does_not_hide_a_block_level_heading() {
        assert_eq!(
            levels("text `code\n# heading` more\n"),
            vec![(1, "heading` more".into())]
        );
    }

    #[test]
    fn control_characters_in_a_heading_are_carried_through_as_inert_text() {
        // The scanner does not sanitize; identity does (`nerve_core::ids::strip_control`).
        let scanned = scan("# Before\u{1f}After\n");
        assert_eq!(texts(&scanned), vec![(1, "Before\u{1f}After".into())]);
    }

    #[test]
    fn scanning_never_panics_on_adversarial_input() {
        for source in [
            "",
            "#",
            "#######",
            "```",
            "~~~",
            "---",
            "---\n",
            "\r\n\r\n",
            "\u{0}\u{1f}\u{7f}",
            "# \u{feff}\u{202e}",
            "    \t    # deep indent",
            "> > > # nested quote",
            "`````````",
            "#\u{0}",
            "Title\n===",
            "\n\n\n=\n",
        ] {
            let scanned = scan(source);
            for heading in &scanned.headings {
                assert!(heading.level >= 1 && heading.level <= MAX_HEADING_LEVEL);
                assert!(heading.section_span.end_byte <= source.len());
            }
        }
    }

    #[test]
    fn line_splitting_handles_every_terminator_form() {
        assert_eq!(lines("").len(), 0);
        assert_eq!(lines("a").len(), 1);
        assert_eq!(lines("a\n").len(), 1);
        assert_eq!(lines("a\nb").len(), 2);
        assert_eq!(lines("a\r\nb\r\n").len(), 2);
        let split = lines("a\r\n");
        assert_eq!(split[0].text, "a");
        assert_eq!(split[0].end, 1);
    }

    // ---- link destinations -----------------------------------------------------------------
    //
    // The scanner **records** destinations. It does not normalize, resolve, stat, open or fetch
    // one, and none of these tests touches a filesystem. What is being pinned here is only
    // "which bytes did the document write, and where" — the question a later resolver is
    // entitled to ask, and the only one this module answers.

    fn destinations(source: &str) -> Vec<String> {
        scan(source)
            .links
            .into_iter()
            .map(|link| link.destination)
            .collect()
    }

    fn forms(source: &str) -> Vec<(String, &'static str)> {
        scan(source)
            .links
            .into_iter()
            .map(|link| (link.destination, link.form.as_str()))
            .collect()
    }

    #[test]
    fn an_inline_link_yields_its_destination() {
        assert_eq!(
            destinations("See [the resolver](../src/resolve.rs) for the rule.\n"),
            vec!["../src/resolve.rs".to_string()]
        );
        assert_eq!(
            forms("[a](./a.md) and [b](./b.md)\n"),
            vec![
                ("./a.md".to_string(), "inline"),
                ("./b.md".to_string(), "inline"),
            ]
        );
    }

    /// A title is not a destination, and neither quote style may swallow the closing paren.
    #[test]
    fn an_inline_link_title_is_not_part_of_the_destination() {
        assert_eq!(
            destinations("[a](./a.md \"the title\")\n"),
            vec!["./a.md".to_string()]
        );
        assert_eq!(
            destinations("[a](./a.md 'the title')\n"),
            vec!["./a.md".to_string()]
        );
        assert_eq!(
            destinations("[a](  ./a.md  \"t\"  )\n"),
            vec!["./a.md".to_string()]
        );
    }

    /// The angle-bracketed destination form, which is how a path containing a space is written.
    #[test]
    fn an_inline_link_may_bracket_its_destination() {
        assert_eq!(
            destinations("[a](<./docs/a file.md>)\n"),
            vec!["./docs/a file.md".to_string()]
        );
        assert_eq!(
            forms("[a](<./b.md> \"t\")\n"),
            vec![("./b.md".to_string(), "inline")]
        );
    }

    /// Balanced parentheses belong to the destination; the first unbalanced one closes the link.
    #[test]
    fn balanced_parentheses_stay_inside_the_destination() {
        assert_eq!(
            destinations("[a](./a(1).md)\n"),
            vec!["./a(1).md".to_string()]
        );
        assert_eq!(destinations("[a](./a.md) (aside)\n"), vec!["./a.md"]);
    }

    #[test]
    fn an_autolink_yields_its_destination() {
        assert_eq!(
            forms("Read <https://example.invalid/spec#L3> today.\n"),
            vec![(
                "https://example.invalid/spec#L3".to_string(),
                "angle-bracket"
            )]
        );
        assert_eq!(
            forms("<mailto:someone@example.invalid>\n"),
            vec![(
                "mailto:someone@example.invalid".to_string(),
                "angle-bracket"
            )]
        );
        assert_eq!(
            forms("<./docs/nearby.md>\n"),
            vec![("./docs/nearby.md".to_string(), "angle-bracket")]
        );
    }

    /// `<br/>`, `<Foo>` and an HTML attribute run are not links. Recording one would fabricate a
    /// reference site the document never wrote, which is the failure this scanner exists to
    /// avoid — not a cosmetic one.
    #[test]
    fn raw_html_and_generics_in_angle_brackets_are_not_links() {
        for source in [
            "A line break<br/>here.\n",
            "A `Map` is a <Foo> in the docs.\n",
            "<div id=x>inline</div>\n",
            "Vec<T, U> is a generic.\n",
            "<>\n",
            "a < b and b > c\n",
            // A closing tag and a root-relative path are indistinguishable in angle brackets.
            "</div>\n",
            "<script>alert(1)</script>\n",
        ] {
            assert!(
                destinations(source).is_empty(),
                "{source:?} produced {:?}",
                destinations(source)
            );
        }
    }

    #[test]
    fn a_reference_definition_yields_its_destination() {
        assert_eq!(
            forms("[spec]: ./docs/spec.md\n"),
            vec![("./docs/spec.md".to_string(), "reference-definition")]
        );
        assert_eq!(
            forms("[spec]: ./docs/spec.md \"The spec\"\n"),
            vec![("./docs/spec.md".to_string(), "reference-definition")]
        );
        assert_eq!(
            forms("[spec]: <./docs/a file.md>\n"),
            vec![("./docs/a file.md".to_string(), "reference-definition")]
        );
    }

    /// A definition is a whole block. A line that continues into prose is a paragraph that
    /// happens to start with a bracket, and guessing otherwise would invent a destination.
    #[test]
    fn a_bracketed_line_that_continues_into_prose_is_not_a_definition() {
        assert!(destinations("[note]: this is ordinary prose\n").is_empty());
        assert!(destinations("  text [note]: ./a.md\n").is_empty());
    }

    /// Fragments are carried through untouched. Splitting `#L12` from the path is resolution,
    /// and resolution is not this module's job.
    #[test]
    fn a_fragment_is_carried_through_untouched() {
        assert_eq!(
            destinations("[a](../src/pipeline.rs#L12)\n"),
            vec!["../src/pipeline.rs#L12".to_string()]
        );
        assert_eq!(
            destinations("[a](../src/pipeline.rs#L12-L20)\n"),
            vec!["../src/pipeline.rs#L12-L20".to_string()]
        );
        assert_eq!(
            destinations("[a](#a-heading-in-this-document)\n"),
            vec!["#a-heading-in-this-document".to_string()]
        );
    }

    /// Block structure is resolved before inline structure, so a fenced block is never even
    /// offered to the link parser. Both fence characters, and an unterminated fence.
    #[test]
    fn a_link_inside_a_fenced_code_block_produces_nothing() {
        assert!(destinations("```\n[a](./a.md)\n```\n").is_empty());
        assert!(destinations("~~~md\n[a](./a.md)\n~~~\n").is_empty());
        assert!(destinations("```\n[a](./a.md)\n").is_empty());
        assert_eq!(
            destinations("[before](./b.md)\n\n```\n[a](./a.md)\n```\n\n[after](./c.md)\n"),
            vec!["./b.md".to_string(), "./c.md".to_string()],
            "the fence must hide only what is inside it"
        );
    }

    #[test]
    fn a_link_inside_an_indented_code_block_produces_nothing() {
        assert!(destinations("text\n\n    [a](./a.md)\n").is_empty());
    }

    #[test]
    fn a_link_inside_front_matter_produces_nothing() {
        assert!(destinations("---\nsee: [a](./a.md)\n---\n\ntext\n").is_empty());
    }

    #[test]
    fn a_link_inside_an_inline_code_span_produces_nothing() {
        assert!(destinations("Write `[a](./a.md)` to link.\n").is_empty());
        assert!(destinations("Write ``[a](./a.md)`` to link.\n").is_empty());
        assert_eq!(
            destinations("`[a](./a.md)` but [b](./b.md) is real\n"),
            vec!["./b.md".to_string()],
            "a code span must hide only what is inside it"
        );
        assert!(
            destinations("An autolink `<https://example.invalid>` in code\n").is_empty(),
            "a code span hides an autolink too"
        );
    }

    /// A bare identifier in a code span is **counted**, never emitted. It is not evidence that
    /// the document means the symbol of that name.
    #[test]
    fn a_bare_code_span_identifier_is_counted_as_a_mention_and_not_a_link() {
        let scanned = scan("Call `parseConfig` before `run()`; see `_private` and `$id`.\n");
        assert!(scanned.links.is_empty());
        assert_eq!(scanned.code_span_mentions, 4);
    }

    /// The predicate is deliberately narrow: anything that is not an identifier is not a mention
    /// either, so the count cannot quietly become "code spans".
    #[test]
    fn a_code_span_that_is_not_an_identifier_is_not_a_mention() {
        let scanned = scan("Use `a.b`, `x/y`, `--flag`, `1two`, `a b` and `` ` ``.\n");
        assert_eq!(scanned.code_span_mentions, 0);
        assert!(scanned.links.is_empty());
    }

    /// A mention inside a heading counts, because a heading is prose; one inside a fence does
    /// not, because a fence is not.
    #[test]
    fn code_span_mentions_follow_the_same_block_rules_as_links() {
        assert_eq!(
            scan("# The `parseConfig` entry point\n").code_span_mentions,
            1
        );
        assert_eq!(scan("```\n`parseConfig`\n```\n").code_span_mentions, 0);
        assert_eq!(scan("    `parseConfig`\n").code_span_mentions, 0);
    }

    #[test]
    fn escaped_brackets_do_not_open_a_link() {
        assert!(destinations("\\[not a link\\](./a.md)\n").is_empty());
        assert_eq!(
            destinations("\\[not a link\\] but [real](./a.md)\n"),
            vec!["./a.md".to_string()]
        );
    }

    /// An escape inside a destination is removed, because the resolver must see the path the
    /// author meant rather than the bytes Markdown needed to write it.
    #[test]
    fn escapes_inside_a_destination_are_removed() {
        assert_eq!(
            destinations("[a](./a\\(1\\).md)\n"),
            vec!["./a(1).md".to_string()]
        );
    }

    #[test]
    fn unmatched_delimiters_produce_nothing() {
        for source in [
            "[unclosed link text\n",
            "[text](./a.md\n",
            "[text] (./a.md)\n",
            "](./a.md)\n",
            "[text]\n",
            "[text]()\n",
            "[text](   )\n",
            "<https://example.invalid\n",
        ] {
            assert!(
                destinations(source).is_empty(),
                "{source:?} produced {:?}",
                destinations(source)
            );
        }
    }

    /// Inline structure is resolved within a line. A bracket pair straddling a line end is not
    /// completed across it, so nothing is emitted for either half.
    #[test]
    fn a_bracket_pair_spanning_a_line_end_is_not_a_link() {
        assert!(destinations("[text\nspanning](./a.md)\n").is_empty());
        assert!(destinations("[text](./a.md\n)\n").is_empty());
    }

    #[test]
    fn nested_brackets_in_link_text_still_find_the_destination() {
        assert_eq!(
            destinations("[a [b] c](./a.md)\n"),
            vec!["./a.md".to_string()]
        );
        // Link text is not descended into. The outer destination is recorded and the nested one
        // is counted, so the under-report is visible rather than silent.
        let nested = scan("[![img](./i.png)](./a.md)\n");
        assert_eq!(destinations_of(&nested), vec!["./a.md".to_string()]);
        assert_eq!(nested.counters.unsupported[form::LINK_IN_LINK_TEXT], 1);
        assert_eq!(
            destinations("[a `]` b](./a.md)\n"),
            vec!["./a.md".to_string()],
            "a bracket inside a code span does not close the link text"
        );
    }

    #[test]
    fn a_link_in_a_heading_is_recorded_exactly_once() {
        let scanned = scan("## See [the plan](./plan.md)\n\nBody.\n");
        assert_eq!(
            texts(&scanned),
            vec![(2, "See [the plan](./plan.md)".into())]
        );
        assert_eq!(
            scanned
                .links
                .iter()
                .map(|link| link.destination.as_str())
                .collect::<Vec<_>>(),
            vec!["./plan.md"]
        );
    }

    /// A setext heading's text line is scanned as the paragraph it was, once and only once.
    #[test]
    fn a_link_on_a_setext_heading_line_is_recorded_exactly_once() {
        let scanned = scan("See [the plan](./plan.md)\n=========================\n");
        assert_eq!(scanned.headings.len(), 1);
        assert_eq!(destinations_of(&scanned), vec!["./plan.md".to_string()]);
    }

    fn destinations_of(scanned: &DocumentScan) -> Vec<String> {
        scanned
            .links
            .iter()
            .map(|link| link.destination.clone())
            .collect()
    }

    /// A block quote and a list item are prose. Documents put links in both constantly, and a
    /// scanner that dropped them would under-report broken links, which is the opposite of the
    /// failure this module is allowed to have.
    #[test]
    fn links_in_list_items_and_block_quotes_are_recorded() {
        assert_eq!(
            destinations("- see [a](./a.md)\n- see [b](./b.md)\n"),
            vec!["./a.md".to_string(), "./b.md".to_string()]
        );
        assert_eq!(
            destinations("> quoted [a](./a.md)\n"),
            vec!["./a.md".to_string()]
        );
    }

    #[test]
    fn the_link_bound_fires_and_is_counted() {
        let mut source = String::new();
        for index in 0..(MAX_LINKS_PER_DOCUMENT + 5) {
            source.push_str(&format!("[l](./f{index}.md)\n\n"));
        }
        let scanned = scan(&source);
        assert_eq!(scanned.links.len(), MAX_LINKS_PER_DOCUMENT);
        assert_eq!(scanned.counters.unsupported[form::LINKS_EXCEEDED], 5);
    }

    #[test]
    fn an_over_long_destination_is_refused_rather_than_stored() {
        let long = "a".repeat(MAX_LINK_DESTINATION_BYTES + 1);
        let scanned = scan(&format!("[l](./{long})\n"));
        assert!(scanned.links.is_empty());
        assert_eq!(
            scanned.counters.unsupported[form::LINK_DESTINATION_TOO_LONG],
            1
        );

        let allowed = "a".repeat(MAX_LINK_DESTINATION_BYTES - 2);
        assert_eq!(scan(&format!("[l](./{allowed})\n")).links.len(), 1);
    }

    /// A control byte in a destination is **carried through**, not truncated away: the path
    /// guard is where a hostile path is refused, and a refusal it never sees is one nobody
    /// reports. The scanner's job is to hand over exactly what was written.
    #[test]
    fn a_control_byte_in_a_destination_survives_to_the_guard() {
        assert_eq!(
            destinations("[a](./ev\u{1f}il.md)\n"),
            vec!["./ev\u{1f}il.md".to_string()]
        );
    }

    /// A citation must point at the link, not at the line that held it.
    #[test]
    fn a_links_span_points_at_the_text_it_came_from() {
        let source = "intro\n\nSee [the plan](./plan.md) now.\n";
        let scanned = scan(source);
        assert_eq!(scanned.links.len(), 1);
        let link = &scanned.links[0];
        assert_eq!(link.span.start_line, 3);
        assert_eq!(link.span.end_line, 3);
        assert_eq!(
            &source[link.span.start_byte..link.span.end_byte],
            "[the plan](./plan.md)"
        );
        assert_eq!(link.span.start_col, 4);
        assert_eq!(link.span.end_col, 25);
    }

    /// The same, for the two other forms and for a line that is not the first.
    #[test]
    fn every_link_form_spans_exactly_its_own_construct() {
        let source = "a\n\n[id]: ./ref.md\n\nb <https://example.invalid/x> c\n";
        let scanned = scan(source);
        let cut: Vec<&str> = scanned
            .links
            .iter()
            .map(|link| &source[link.span.start_byte..link.span.end_byte])
            .collect();
        assert_eq!(cut, vec!["[id]: ./ref.md", "<https://example.invalid/x>"]);
        assert_eq!(
            scanned
                .links
                .iter()
                .map(|link| link.span.start_line)
                .collect::<Vec<_>>(),
            vec![3, 5]
        );
    }

    /// Spans stay valid when the document is not ASCII, which byte offsets make easy to get
    /// wrong: a link after a multi-byte character must still slice on a character boundary.
    #[test]
    fn spans_are_correct_after_multi_byte_characters() {
        let source = "— naïve — see [a](./a.md)\n";
        let scanned = scan(source);
        assert_eq!(scanned.links.len(), 1);
        assert_eq!(
            &source[scanned.links[0].span.start_byte..scanned.links[0].span.end_byte],
            "[a](./a.md)"
        );
    }

    #[test]
    fn link_form_tags_are_stable() {
        assert_eq!(LinkForm::Inline.as_str(), "inline");
        assert_eq!(
            LinkForm::ReferenceDefinition.as_str(),
            "reference-definition"
        );
        assert_eq!(LinkForm::AngleBracket.as_str(), "angle-bracket");
    }

    #[test]
    fn scheme_detection_matches_commonmarks_rule() {
        assert_eq!(scheme_of("https://x"), Some("https"));
        assert_eq!(scheme_of("mailto:a@b"), Some("mailto"));
        assert_eq!(scheme_of("x-y.z+1:rest"), Some("x-y.z+1"));
        assert_eq!(scheme_of("./a.md"), None);
        assert_eq!(scheme_of("1http://x"), None);
        assert_eq!(scheme_of("no-colon"), None);
        assert_eq!(scheme_of(":empty"), None);
    }

    /// The hostile shapes already committed in `fixtures/md-docs/docs/hostile.md`. The scanner
    /// records them as inert destinations; it does not follow, execute or interpret any of them.
    #[test]
    fn hostile_link_shapes_are_recorded_as_inert_text() {
        assert_eq!(
            destinations("[click](javascript:alert(1))\n"),
            vec!["javascript:alert(1)".to_string()]
        );
        assert_eq!(
            destinations("[traversal](../../../etc/passwd)\n"),
            vec!["../../../etc/passwd".to_string()]
        );
        assert_eq!(
            destinations("[../../../etc/passwd](./real.md)\n"),
            vec!["./real.md".to_string()],
            "link *text* is not a destination"
        );
        assert!(
            destinations("<script>alert(1)</script>\n").is_empty(),
            "a script tag is raw HTML, not a link"
        );
        assert!(
            destinations("<img src=x onerror=alert(1)>\n").is_empty(),
            "an attribute run contains spaces and is not an autolink"
        );
    }

    #[test]
    fn link_scanning_never_panics_on_adversarial_input() {
        for source in [
            "[",
            "[[[[[[[[[[",
            "]]]]]]]]]]",
            "[](",
            "[](<",
            "[](<>)",
            "[]()",
            "[a](\\",
            "[a](./a.md\\",
            "<<<<<<<<<<",
            ">>>>>>>>>>",
            "`[a](./a.md)",
            "[a](`)`)",
            "[\u{1f}](\u{1f})",
            "[é](./é.md)",
            "[a]: ",
            "[a]:",
            "[a](  \t  )",
            "[a](<unterminated)\n",
            "\\",
            "\\\\[a](./a.md)",
        ] {
            let scanned = scan(source);
            for link in &scanned.links {
                assert!(
                    link.span.end_byte <= source.len(),
                    "{source:?} produced a span past the end"
                );
                assert!(link.span.start_byte <= link.span.end_byte);
                assert!(source.is_char_boundary(link.span.start_byte));
                assert!(source.is_char_boundary(link.span.end_byte));
                assert!(!link.destination.is_empty());
                assert!(link.destination.len() <= MAX_LINK_DESTINATION_BYTES);
            }
        }
    }

    /// Scanning is a pure function of the bytes: the same document twice is the same result.
    #[test]
    fn link_scanning_is_deterministic() {
        let source = "# T\n\n[a](./a.md) [b](<./b c.md> \"t\")\n\n[r]: ./r.md\n\n<./d.md>\n";
        assert_eq!(scan(source), scan(source));
        assert_eq!(
            destinations(source),
            vec![
                "./a.md".to_string(),
                "./b c.md".to_string(),
                "./r.md".to_string(),
                "./d.md".to_string(),
            ]
        );
    }
}
