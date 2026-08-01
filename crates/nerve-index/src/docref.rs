//! Document link resolution: turning a destination a document wrote into an indexed entity.
//!
//! The scanner ([`crate::markdown`]) records destinations and refuses to interpret them. This
//! module interprets them, and it is the only place that does. Two decisions handed over from
//! Slice 5b are settled here, and both are load-bearing rather than stylistic.
//!
//! # 1. Every destination goes through the path guard
//!
//! A destination is repository content, and repository content is attacker-controlled
//! (THREAT-MODEL.md, A1). Slice 5a proved that a `0x1f` in a path can forge the identity of an
//! entity in a *different* file, because `rel_path` is a field of every canonical tuple. The
//! scanner deliberately carries control bytes through rather than truncating them, precisely so
//! that the guard sees them and reports them — a refusal nobody sees is one nobody reports.
//!
//! So every destination is routed through [`crate::discover::canonical_child`], the same choke
//! point discovery uses. That is not belt-and-braces: it means a refusal rule added to the guard
//! later applies to document links automatically, instead of leaving this module to remember it.
//!
//! What the guard's *answer* is used for is narrow and deliberate:
//!
//! - `ControlCharacterInPath` / `NonUtf8Path` — the destination is **refused**. The file is never
//!   opened. (`canonical_child` reads directory metadata, never file contents, and this module
//!   opens nothing at all.)
//! - any other error — the candidate is not a real path under the root. That is either a missing
//!   file or a symlink pointing outside; both mean "not an indexed file", which is what the
//!   membership check below concludes anyway.
//! - success — ignored. Resolution is decided by membership of the **indexed path set**, not by
//!   the filesystem, so that resolution is a pure function of the inputs incremental indexing
//!   already tracks. A symlink is never in that set (discovery refuses to follow one), so a link
//!   through a symlink resolves to nothing rather than silently to the symlink's target.
//!
//! # 2. Percent-encoding is not decoded
//!
//! `./my%20file.md` names nothing here, even when `my file.md` exists and is indexed.
//!
//! Decoding would reintroduce exactly the hole the guard exists to close: `%1f` would smuggle a
//! unit separator past the scanner, which never sees a decoded byte, and into a path that the
//! guard was asked about *before* the decode. The rule is therefore "the bytes between the
//! delimiters are the path", with no transformation, and a document that means a path with a
//! space can write `[text](<./my file.md>)`, which the scanner already supports.
//!
//! # What is resolved, and what is refused
//!
//! - A destination whose path part names an indexed file resolves to that file.
//! - A `#L<n>` or `#L<n>-L<m>` fragment additionally resolves to the **innermost** symbol whose
//!   span covers line `n`. Two symbols with identical spans are ambiguous, and ambiguity is a
//!   refusal here for the same reason it is in move detection.
//! - An external destination (any URI scheme, or a protocol-relative `//host/…`) is a legitimate
//!   reference to something outside the repository. It is **counted and never fetched**, and it
//!   is not an `Unresolved` entity: nothing failed.
//! - A bare `#fragment` names a heading, and heading-anchor resolution is not modelled in this
//!   slice. It is counted, not entity-ised.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use nerve_core::model::Span;

use crate::discover;
use crate::error::IndexError;
use crate::markdown::{LinkForm, RawLink};

/// Closed reason vocabulary for an `Unresolved` entity of category `document_link`.
///
/// Closed, and recorded on both the entity and every observation, so that "why is this
/// unresolved?" has one of a small number of answers a reader can enumerate — the same contract
/// Slice 2a's [`crate::refs::UnresolvedReason`] provides for value references.
pub mod reason {
    /// The destination looks like a repository path and names no indexed file.
    ///
    /// The broken-documentation-link signal: this is what a stale `docs/` link produces.
    pub const TARGET_NOT_INDEXED: &str = "document_link_target_not_indexed";
    /// The path guard refused the destination. The file was never read.
    pub const REFUSED: &str = "document_link_refused";
    /// A `#L<n>` anchor that no symbol covers, or that lies past the end of the file.
    pub const ANCHOR_NO_SYMBOL: &str = "document_anchor_no_symbol";
}

/// Outcome tags counted per document and reported over the whole repository.
///
/// Counted rather than inferred from the graph, because three of the six produce no row at all:
/// an external destination, a bare fragment and a resolved file that also resolved an anchor
/// would otherwise be indistinguishable from a document Nerve never looked at.
pub mod outcome {
    /// Resolved to an indexed file.
    pub const RESOLVED_FILE: &str = "document_link_resolved_file";
    /// A `#L<n>` anchor that resolved to exactly one symbol.
    pub const RESOLVED_SYMBOL: &str = "document_link_resolved_symbol";
    /// An external reference. Counted, never fetched.
    pub const EXTERNAL: &str = "document_link_external";
    /// A destination that is only a `#fragment`.
    pub const FRAGMENT_ONLY: &str = "document_link_fragment_only";
}

/// A `#L<n>` or `#L<n>-L<m>` line anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineAnchor {
    /// First line named, 1-based. This is the line resolution uses.
    pub start: usize,
    /// Last line named, 1-based. Equal to `start` for a single-line anchor.
    pub end: usize,
}

/// One symbol's extent in a target file, reduced to what anchor resolution reads.
///
/// Carries bytes as well as lines because "innermost" is defined on the byte span: a method and
/// the class that holds it can begin and end on the same lines when both are written on one line,
/// and the byte span still separates them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolExtent {
    /// Entity id of the symbol.
    pub entity_id: String,
    /// Inclusive start byte.
    pub start_byte: usize,
    /// Exclusive end byte.
    pub end_byte: usize,
    /// 1-based first line.
    pub start_line: usize,
    /// 1-based last line.
    pub end_line: usize,
}

impl SymbolExtent {
    fn covers(&self, line: usize) -> bool {
        self.start_line <= line && line <= self.end_line
    }

    fn byte_len(&self) -> usize {
        self.end_byte.saturating_sub(self.start_byte)
    }
}

/// The whole-repository view resolution reads.
///
/// Every field spans the entire repository, not the files this run re-extracted: a document may
/// link to a file nothing touched, and the answer must not depend on which files were parsed.
#[derive(Debug, Clone, Copy)]
pub struct Corpus<'a> {
    /// Every indexed repository-relative path — code and documents alike.
    ///
    /// Wider than the set module resolution consults, and deliberately so: a README linking to
    /// another README is an ordinary and useful thing to record, whereas a *specifier* resolving
    /// to a document would invent a `Module` entity no extractor creates.
    pub indexed: &'a BTreeSet<String>,
    /// Symbol extents by repository-relative path.
    pub symbols: &'a BTreeMap<String, Vec<SymbolExtent>>,
    /// Content hash by repository-relative path, as this run observed it.
    pub content_hashes: &'a BTreeMap<String, String>,
}

/// What a `#L<n>` anchor resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorOutcome {
    /// Exactly one innermost symbol covers the line.
    Symbol {
        /// The symbol's entity id.
        entity_id: String,
        /// The anchor as written.
        anchor: LineAnchor,
        /// The **target file's** content hash at resolution time.
        ///
        /// Recorded so that a later `nerve why` can say the anchor was resolved against bytes the
        /// file no longer has. Deliberately recorded for the anchor edge only: putting it on the
        /// file edge as well would make every document that links to a file a dependent of that
        /// file's contents, and re-extract the documentation of a repository every time any file
        /// in it changed.
        target_content_hash: String,
    },
    /// No symbol covers the line, or two with identical spans do.
    NoSymbol {
        /// The anchor as written.
        anchor: LineAnchor,
    },
}

/// What one destination resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    /// An absolute or protocol-relative reference to something outside the repository.
    External {
        /// The scheme as written, or `//` for a protocol-relative destination.
        scheme: String,
    },
    /// A destination that is only a `#fragment`.
    FragmentOnly,
    /// The path guard refused it. The file was never read.
    Refused,
    /// A repository-shaped path naming no indexed file.
    NotIndexed {
        /// The normalized repository-relative path that named nothing.
        path: String,
    },
    /// An indexed file, and what the fragment did.
    File {
        /// Repository-relative path of the indexed file.
        path: String,
        /// Present only when the fragment was a line anchor.
        anchor: Option<AnchorOutcome>,
    },
}

impl LinkOutcome {
    /// The counter key this outcome contributes, plus the anchor's own key when it has one.
    fn counter_keys(&self) -> Vec<&'static str> {
        match self {
            LinkOutcome::External { .. } => vec![outcome::EXTERNAL],
            LinkOutcome::FragmentOnly => vec![outcome::FRAGMENT_ONLY],
            LinkOutcome::Refused => vec![reason::REFUSED],
            LinkOutcome::NotIndexed { .. } => vec![reason::TARGET_NOT_INDEXED],
            LinkOutcome::File { anchor, .. } => match anchor {
                None => vec![outcome::RESOLVED_FILE],
                Some(AnchorOutcome::Symbol { .. }) => {
                    vec![outcome::RESOLVED_FILE, outcome::RESOLVED_SYMBOL]
                }
                Some(AnchorOutcome::NoSymbol { .. }) => {
                    vec![outcome::RESOLVED_FILE, reason::ANCHOR_NO_SYMBOL]
                }
            },
        }
    }

    /// The reason recorded on an `Unresolved` entity, when this outcome produces one.
    pub fn unresolved_reason(&self) -> Option<&'static str> {
        match self {
            LinkOutcome::Refused => Some(reason::REFUSED),
            LinkOutcome::NotIndexed { .. } => Some(reason::TARGET_NOT_INDEXED),
            LinkOutcome::File {
                anchor: Some(AnchorOutcome::NoSymbol { .. }),
                ..
            } => Some(reason::ANCHOR_NO_SYMBOL),
            _ => None,
        }
    }
}

/// One resolved link site, ready for the graph builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSite {
    /// The link construct, so the citation points at the link and not at the line.
    pub span: Span,
    /// Which syntax carried the destination.
    pub form: LinkForm,
    /// The destination exactly as the document wrote it. Repository content; inert.
    pub raw_destination: String,
    /// What resolution concluded.
    pub outcome: LinkOutcome,
}

/// Everything resolution concluded about one document's links.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentLinks {
    /// Resolved sites, in source order.
    pub sites: Vec<LinkSite>,
    /// Outcome tallies by key. Every key is from [`outcome`] or [`reason`].
    pub outcomes: BTreeMap<String, usize>,
}

/// The URI scheme a destination opens with, per CommonMark's autolink rule.
///
/// A letter, then letters, digits, `+`, `.` or `-`, then a colon. `./a:b.md` has no scheme
/// because `.` is not a letter, which is what keeps an ordinary relative path with a colon in it
/// from being read as a URI.
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

/// The scheme of an external destination, or `None` when it is repository-shaped.
///
/// `//host/path` is protocol-relative and is external too: it names a host, not a path in this
/// repository, and reading it as a root-relative path would turn `//evil/x` into `evil/x`.
pub fn external_scheme(destination: &str) -> Option<&str> {
    if destination.starts_with("//") {
        return Some("//");
    }
    scheme_of(destination)
}

/// Split a destination into its path part and its fragment, at the first `#`.
pub fn split_fragment(destination: &str) -> (&str, Option<&str>) {
    match destination.find('#') {
        Some(index) => (&destination[..index], Some(&destination[index + 1..])),
        None => (destination, None),
    }
}

/// Parse `L<n>` or `L<n>-L<m>` out of a fragment.
///
/// Anything else — including `L0`, a number too large for a `usize`, and every ordinary heading
/// anchor — is not a line anchor. Lines are 1-based everywhere in Nerve, so `L0` names no line
/// and is not coerced into naming the first one.
pub fn parse_line_anchor(fragment: &str) -> Option<LineAnchor> {
    let number = |text: &str| -> Option<usize> {
        let digits = text.strip_prefix('L')?;
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        digits.parse::<usize>().ok().filter(|value| *value >= 1)
    };
    match fragment.split_once('-') {
        None => number(fragment).map(|start| LineAnchor { start, end: start }),
        Some((first, second)) => {
            let start = number(first)?;
            let end = number(second)?;
            if end < start {
                return None;
            }
            Some(LineAnchor { start, end })
        }
    }
}

/// Normalize `<document's directory> + <path part>` into a repository-relative path.
///
/// A leading `/` is root-relative; everything else is relative to the document's own directory.
/// Returns `None` when the path climbs above the repository root, which is a refusal.
///
/// Written here rather than shared with [`crate::resolve`] because the two rules genuinely
/// differ: module resolution has no root-relative form and appends extension and index-file
/// candidates, and a document link does neither. Sharing them would mean a `[text](./util)` in a
/// README silently resolving to `src/util.ts`, which the document did not say.
fn normalize(document_rel_path: &str, path_part: &str) -> Option<String> {
    let mut segments: Vec<&str> = if path_part.starts_with('/') {
        Vec::new()
    } else {
        match document_rel_path.rfind('/') {
            Some(index) => document_rel_path[..index].split('/').collect(),
            None => Vec::new(),
        }
    };
    if segments == [""] {
        segments.clear();
    }

    for part in path_part.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

/// The indexed file a destination names, ignoring any fragment, or `None`.
///
/// **Pure**: no filesystem access, no guard call. It exists so that change detection
/// ([`crate::incremental::classify`]) can ask "would this destination resolve differently now?"
/// without a repository root and without paying a `stat` per cached destination — exactly the way
/// it re-resolves cached import specifiers.
///
/// It agrees with [`resolve_destination`] on *which* file is named. It cannot disagree about a
/// refusal either: the only refusals the guard adds are control characters and non-UTF-8, and no
/// path carrying one can be in the indexed set, because discovery refused it first.
pub fn resolve_path(
    document_rel_path: &str,
    destination: &str,
    indexed: &BTreeSet<String>,
) -> Option<String> {
    if external_scheme(destination).is_some() {
        return None;
    }
    let (path_part, _) = split_fragment(destination);
    if path_part.is_empty() {
        return None;
    }
    let normalized = normalize(document_rel_path, path_part)?;
    indexed.contains(&normalized).then_some(normalized)
}

/// The line anchor a destination carries, if any. Used by change detection.
pub fn anchor_of(destination: &str) -> Option<LineAnchor> {
    if external_scheme(destination).is_some() {
        return None;
    }
    split_fragment(destination).1.and_then(parse_line_anchor)
}

/// The innermost symbol covering `line`, or `None` when nothing covers it or the answer is tied.
///
/// A method inside a class covers the same line as the class, and the innermost is the
/// deterministic answer to "which symbol is line `n` in?". Two candidates sharing the smallest
/// byte span are ambiguous — properly nested spans of equal length that both contain the same
/// line are identical — and ambiguity is refused rather than broken by declaration order.
pub fn innermost_covering(extents: &[SymbolExtent], line: usize) -> Option<&SymbolExtent> {
    let mut best: Option<&SymbolExtent> = None;
    let mut tied = false;
    for extent in extents.iter().filter(|extent| extent.covers(line)) {
        match best {
            None => best = Some(extent),
            Some(current) => {
                if extent.byte_len() < current.byte_len() {
                    best = Some(extent);
                    tied = false;
                } else if extent.byte_len() == current.byte_len() {
                    tied = true;
                }
            }
        }
    }
    if tied {
        return None;
    }
    best
}

/// Resolve one destination, with the path guard in the loop.
///
/// `root` is the canonical repository root. Nothing here opens the destination.
pub fn resolve_destination(
    root: &Path,
    document_rel_path: &str,
    destination: &str,
    corpus: &Corpus<'_>,
) -> LinkOutcome {
    if let Some(scheme) = external_scheme(destination) {
        return LinkOutcome::External {
            scheme: scheme.to_string(),
        };
    }
    let (path_part, fragment) = split_fragment(destination);
    if path_part.is_empty() {
        return LinkOutcome::FragmentOnly;
    }
    let Some(normalized) = normalize(document_rel_path, path_part) else {
        // Climbed above the repository root. Refused before anything reaches the filesystem.
        return LinkOutcome::Refused;
    };

    // The guard. Its refusals are the only part of its answer resolution acts on; see the module
    // documentation for why success is deliberately ignored.
    match discover::canonical_child(root, Path::new(&normalized)) {
        Err(IndexError::ControlCharacterInPath(_)) | Err(IndexError::NonUtf8Path(_)) => {
            return LinkOutcome::Refused
        }
        Err(_) | Ok(_) => {}
    }

    if !corpus.indexed.contains(&normalized) {
        return LinkOutcome::NotIndexed { path: normalized };
    }

    let anchor = fragment.and_then(parse_line_anchor).map(|anchor| {
        let extents = corpus.symbols.get(&normalized);
        match extents.and_then(|extents| innermost_covering(extents, anchor.start)) {
            Some(extent) => AnchorOutcome::Symbol {
                entity_id: extent.entity_id.clone(),
                anchor,
                target_content_hash: corpus
                    .content_hashes
                    .get(&normalized)
                    .cloned()
                    .unwrap_or_default(),
            },
            None => AnchorOutcome::NoSymbol { anchor },
        }
    });

    LinkOutcome::File {
        path: normalized,
        anchor,
    }
}

/// Resolve every link in one document.
pub fn resolve_document_links(
    root: &Path,
    document_rel_path: &str,
    links: &[RawLink],
    corpus: &Corpus<'_>,
) -> DocumentLinks {
    let mut resolved = DocumentLinks::default();
    for link in links {
        let outcome = resolve_destination(root, document_rel_path, &link.destination, corpus);
        for key in outcome.counter_keys() {
            *resolved.outcomes.entry(key.to_string()).or_insert(0) += 1;
        }
        resolved.sites.push(LinkSite {
            span: link.span,
            form: link.form,
            raw_destination: link.destination.clone(),
            outcome,
        });
    }
    resolved
}

/// Every destination a document wrote, deduplicated and sorted.
///
/// This is the document counterpart of [`crate::facts::ModuleFacts::import_specifiers`], and it
/// is cached for the same reason: adding, deleting or moving a file can change what a destination
/// resolves to without the document itself changing at all, and the comparison must not require
/// re-scanning every document to find out.
///
/// The **whole** destination is kept, fragment included, because the fragment is what says
/// whether the document depends on the target file's *contents* as well as its existence.
pub fn cached_destinations(links: &[RawLink]) -> Vec<String> {
    let unique: BTreeSet<String> = links.iter().map(|link| link.destination.clone()).collect();
    unique.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| (*path).to_string()).collect()
    }

    fn extent(
        entity_id: &str,
        start_byte: usize,
        end_byte: usize,
        lines: (usize, usize),
    ) -> SymbolExtent {
        SymbolExtent {
            entity_id: entity_id.to_string(),
            start_byte,
            end_byte,
            start_line: lines.0,
            end_line: lines.1,
        }
    }

    #[test]
    fn relative_destinations_resolve_against_the_documents_own_directory() {
        let set = indexed(&["docs/guide.md", "src/app.ts", "README.md"]);
        assert_eq!(
            resolve_path("docs/index.md", "./guide.md", &set),
            Some("docs/guide.md".to_string())
        );
        assert_eq!(
            resolve_path("docs/index.md", "guide.md", &set),
            Some("docs/guide.md".to_string())
        );
        assert_eq!(
            resolve_path("docs/index.md", "../src/app.ts", &set),
            Some("src/app.ts".to_string())
        );
        assert_eq!(
            resolve_path("README.md", "docs/guide.md", &set),
            Some("docs/guide.md".to_string())
        );
    }

    #[test]
    fn a_leading_slash_resolves_against_the_repository_root() {
        let set = indexed(&["src/app.ts"]);
        assert_eq!(
            resolve_path("docs/deep/notes.md", "/src/app.ts", &set),
            Some("src/app.ts".to_string())
        );
    }

    #[test]
    fn climbing_above_the_root_is_refused_rather_than_clamped() {
        assert_eq!(normalize("docs/hostile.md", "../../../etc/passwd"), None);
        assert_eq!(normalize("README.md", "../secret"), None);
        // One level up from `docs/` is the root, which is still inside the repository.
        assert_eq!(
            normalize("docs/a.md", "../README.md"),
            Some("README.md".to_string())
        );
    }

    /// The decision Slice 5b handed over. Decoding `%20` here would also decode `%1f`, and the
    /// scanner never sees a decoded byte, so the guard would be asked about the wrong bytes.
    #[test]
    fn percent_encoding_is_not_decoded() {
        let set = indexed(&["docs/my file.md"]);
        assert_eq!(resolve_path("README.md", "./docs/my%20file.md", &set), None);
        assert_eq!(
            resolve_path("README.md", "./docs/my file.md", &set),
            Some("docs/my file.md".to_string())
        );
    }

    #[test]
    fn external_destinations_are_recognised_and_never_resolved() {
        for destination in [
            "https://example.invalid/x",
            "http://example.invalid",
            "mailto:nobody@example.invalid",
            "javascript:alert(1)",
            "//example.invalid/x",
            "ftp://example.invalid",
        ] {
            assert!(
                external_scheme(destination).is_some(),
                "{destination} was not recognised as external"
            );
            assert_eq!(resolve_path("README.md", destination, &indexed(&[])), None);
        }
        for destination in ["./a.md", "../a.md", "a.md", "/a.md", "#L1"] {
            assert_eq!(external_scheme(destination), None, "{destination}");
        }
    }

    #[test]
    fn line_anchors_parse_only_in_the_two_supported_forms() {
        assert_eq!(
            parse_line_anchor("L12"),
            Some(LineAnchor { start: 12, end: 12 })
        );
        assert_eq!(
            parse_line_anchor("L12-L20"),
            Some(LineAnchor { start: 12, end: 20 })
        );
        for fragment in ["heading", "l12", "L", "L0", "L12-L3", "L1-2", "L1.5", ""] {
            assert_eq!(parse_line_anchor(fragment), None, "{fragment}");
        }
    }

    #[test]
    fn the_fragment_is_split_off_before_the_path_is_read() {
        let set = indexed(&["src/app.ts"]);
        assert_eq!(
            resolve_path("README.md", "./src/app.ts#L9", &set),
            Some("src/app.ts".to_string())
        );
        assert_eq!(
            anchor_of("./src/app.ts#L9-L11"),
            Some(LineAnchor { start: 9, end: 11 })
        );
        assert_eq!(anchor_of("./src/app.ts#usage"), None);
        assert_eq!(anchor_of("https://example.invalid/#L9"), None);
    }

    #[test]
    fn a_fragment_with_no_path_resolves_to_nothing() {
        assert_eq!(resolve_path("README.md", "#heading", &indexed(&[])), None);
        assert_eq!(split_fragment("#heading"), ("", Some("heading")));
    }

    /// The nesting rule: a method inside a class covers the same line, and the method wins.
    #[test]
    fn the_innermost_symbol_wins_when_spans_nest() {
        let extents = vec![
            extent("class", 0, 200, (5, 12)),
            extent("method", 40, 120, (8, 11)),
            extent("other", 300, 400, (20, 25)),
        ];
        assert_eq!(
            innermost_covering(&extents, 9).map(|e| e.entity_id.as_str()),
            Some("method")
        );
        assert_eq!(
            innermost_covering(&extents, 6).map(|e| e.entity_id.as_str()),
            Some("class")
        );
        assert!(innermost_covering(&extents, 15).is_none());
        assert!(innermost_covering(&[], 1).is_none());
    }

    #[test]
    fn two_symbols_with_identical_spans_are_ambiguous_and_resolve_to_neither() {
        let extents = vec![extent("a", 10, 50, (2, 4)), extent("b", 10, 50, (2, 4))];
        assert!(
            innermost_covering(&extents, 3).is_none(),
            "an ambiguous anchor must not be resolved by declaration order"
        );
    }

    #[test]
    fn cached_destinations_are_deduplicated_and_sorted() {
        let link = |destination: &str| RawLink {
            destination: destination.to_string(),
            form: LinkForm::Inline,
            span: Span::NONE,
        };
        assert_eq!(
            cached_destinations(&[link("./b.md"), link("./a.md"), link("./b.md")]),
            vec!["./a.md".to_string(), "./b.md".to_string()]
        );
    }

    #[test]
    fn outcome_counter_keys_and_reasons_agree() {
        assert_eq!(
            LinkOutcome::Refused.unresolved_reason(),
            Some(reason::REFUSED)
        );
        assert_eq!(
            LinkOutcome::NotIndexed {
                path: "x".to_string()
            }
            .unresolved_reason(),
            Some(reason::TARGET_NOT_INDEXED)
        );
        let resolved = LinkOutcome::File {
            path: "src/a.ts".to_string(),
            anchor: None,
        };
        assert_eq!(resolved.unresolved_reason(), None);
        assert_eq!(resolved.counter_keys(), vec![outcome::RESOLVED_FILE]);
        assert_eq!(
            LinkOutcome::External {
                scheme: "https".to_string()
            }
            .unresolved_reason(),
            None
        );
    }
}
