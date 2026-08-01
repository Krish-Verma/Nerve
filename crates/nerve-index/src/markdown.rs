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

/// Byte ranges of inline code spans within one line.
///
/// Only within a line: a code span may cross lines in CommonMark, but block structure is
/// resolved first, so a multi-line span can never change whether a line *is* a heading. This is
/// used on heading text, where the question is whether a `#` is a closing marker or content.
fn code_span_ranges(text: &str) -> Vec<(usize, usize)> {
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
                spans.push((open_at, close_at + close_len));
                position = position + 1 + offset + 1;
            }
            None => position += 1,
        }
    }
    spans
}

fn inside_any(ranges: &[(usize, usize)], offset: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| offset >= *start && offset < *end)
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

    assign_section_spans(&mut headings, source.len(), lines.len().max(1));

    DocumentScan {
        headings,
        front_matter: front_matter_span,
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
}
