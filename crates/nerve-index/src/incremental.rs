//! Change detection, the invalidation set, and identity-link proposals.
//!
//! # Why "the file that changed" is not the answer
//!
//! Slice 2a resolves references through an export map and a transitive re-export closure. Given
//!
//! ```text
//! app.ts     import { helper } from './barrel'
//! barrel.ts  export * from './impl'
//! impl.ts    export function helper() {}
//! ```
//!
//! editing `impl.ts` changes what `app.ts` resolves, and `app.ts` never mentions `impl.ts`.
//! Re-extracting only the changed file would leave `app.ts` holding a resolution that is
//! silently wrong — which is worse than a slow re-index, because nothing reports it.
//!
//! The invalidation set is therefore the **reverse-reachable set over `IMPORTS`** from the
//! changed files, walked over the stored graph ([`nerve_store::importers_of`], backed by
//! `idx_assertion_target`). `IMPORTS` is emitted for `export ... from` as well as for `import`
//! and `require`, so the barrel is on that edge and the closure reaches `app.ts` in two hops.
//!
//! # Why the changed set is larger than "files whose bytes differ"
//!
//! Module resolution reads the *set of indexed paths*. Adding `impl.ts` can make a previously
//! unresolved `./impl` resolve, and adding `math.ts` can make `./math` stop resolving to
//! `math.js`, in files whose own bytes did not change and which have no `IMPORTS` edge to the
//! new file — the edge points at an `Unresolved` entity, or at the wrong module. So whenever the
//! path set changes, every cached module's specifiers are re-resolved against the old and the
//! new path set, and any module whose answer moved joins the seed. That comparison is exact and
//! costs no parsing, because the specifiers are cached.
//!
//! # Documents depend on the tree in two ways, and neither is an edge
//!
//! Slice 5c gives a document `REFERENCES` edges, and they rest on inputs the document does not
//! contain:
//!
//! - **the indexed path set** — `[guide](./guide.md)` resolves or does not resolve according to
//!   whether `guide.md` is indexed, so adding, deleting or moving *any* file can change a
//!   document's graph while its bytes stand still. Unlike a specifier, a destination is resolved
//!   against the **whole** path set, documents included: a README linking to a README is an
//!   ordinary thing to record.
//! - **the target file's contents**, but only under a line anchor. `./util.ts#L13` is resolved
//!   to the symbol covering line 13 and records the hash it was resolved at, so editing
//!   `util.ts` can move that answer. A link *without* an anchor depends only on the target
//!   existing, and is deliberately left alone — making every document that links to a file a
//!   dependent of that file's contents would re-scan the documentation on every source edit.
//!
//! Both comparisons are exact and cost no scanning, because the destinations are cached
//! ([`crate::facts::DocumentCounters::destinations`]) and both are answered by the pure
//! [`crate::docref::resolve_path`].

use std::collections::{BTreeMap, BTreeSet};

use nerve_core::ids;

use crate::docref;
use crate::error::Result;
use crate::facts::{CachedSymbol, ModuleFacts};
use crate::lang::{path_is_document, path_is_python};
use crate::pyresolve;
use crate::resolve;

/// How one path differs from the previously indexed tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    /// Present before and now, with identical content and extractor versions.
    Unchanged,
    /// Present before and now, with different content or a superseded extractor version.
    Modified,
    /// Not previously indexed.
    Added,
    /// Previously indexed and no longer present.
    Removed,
}

/// The classification of a tree against the previous index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Paths whose content and extractor versions are unchanged.
    pub unchanged: BTreeSet<String>,
    /// Paths whose content or extractor version changed.
    pub modified: BTreeSet<String>,
    /// Paths not previously indexed.
    pub added: BTreeSet<String>,
    /// Paths no longer present.
    pub removed: BTreeSet<String>,
    /// Paths whose bytes are unchanged but whose specifier resolution moved.
    pub resolution_changed: BTreeSet<String>,
}

impl ChangeSet {
    /// The files that must be re-extracted before the closure is applied.
    pub fn seed(&self) -> BTreeSet<String> {
        let mut seed = self.modified.clone();
        seed.extend(self.added.iter().cloned());
        seed.extend(self.removed.iter().cloned());
        seed.extend(self.resolution_changed.iter().cloned());
        seed
    }

    /// True when the tree is byte-identical to the previous index.
    pub fn is_unchanged(&self) -> bool {
        self.modified.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.resolution_changed.is_empty()
    }
}

/// What a previously indexed module recorded, as far as change detection is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousModule {
    /// Content hash it was extracted at.
    pub content_hash: String,
    /// Whether the cached payload is readable and produced by the current extractor versions.
    pub reusable: bool,
    /// Cached facts, when readable.
    pub facts: Option<ModuleFacts>,
}

/// The code files among a set of repository-relative paths.
///
/// Module resolution never sees a document (`crate::lang::FileKind`), so neither does anything
/// that reasons about what resolution would answer.
fn code_paths<'a>(paths: impl Iterator<Item = &'a String>) -> BTreeSet<String> {
    paths
        .filter(|path| !path_is_document(path))
        .cloned()
        .collect()
}

/// Classify the current tree against the previous index.
///
/// `force_full` marks every present path modified, which is what `nerve index --full` does: the
/// same code path then rebuilds everything, so a full run and a from-scratch run cannot diverge
/// through separate implementations.
pub fn classify(
    previous: &BTreeMap<String, PreviousModule>,
    current: &BTreeMap<String, String>,
    force_full: bool,
) -> ChangeSet {
    let mut set = ChangeSet::default();

    for (path, content_hash) in current {
        match previous.get(path) {
            None => {
                set.added.insert(path.clone());
            }
            Some(before) => {
                let same = before.reusable && &before.content_hash == content_hash;
                if same && !force_full {
                    set.unchanged.insert(path.clone());
                } else {
                    set.modified.insert(path.clone());
                }
            }
        }
    }

    for path in previous.keys() {
        if !current.contains_key(path) {
            set.removed.insert(path.clone());
        }
    }

    // Specifier re-resolution reads the set of **code** paths, because that is the only set
    // either resolver consults. Adding or deleting a document cannot move a specifier, so gating
    // on the whole path set would re-resolve every cached module every time a README changed.
    //
    // Which resolver is decided by the importer's own extension, not by the shape of the
    // specifier. A Python module's `pkg.util` and a TypeScript module's `./util` are answered by
    // different rules, and running one language's resolver over the other's specifiers would
    // report "unchanged" for a file whose imports really did move.
    let before: BTreeSet<String> = code_paths(previous.keys());
    let after: BTreeSet<String> = code_paths(current.keys());
    if before != after {
        for (path, module) in previous {
            if !after.contains(path) || set.modified.contains(path) {
                continue;
            }
            let Some(facts) = &module.facts else { continue };
            let python = path_is_python(path);
            let moved = facts.import_specifiers.iter().any(|specifier| {
                if python {
                    // Deliberately does **not** model the `sys.path` refusal the graph builder
                    // applies. That refusal forces a `None` whatever the file set holds, so a
                    // specifier this comparison calls "moved" can only ever be one the builder
                    // was going to record as `Unresolved` twice over: the error is an extra
                    // re-extraction, never a missed one.
                    pyresolve::resolve(path, specifier, &before)
                        != pyresolve::resolve(path, specifier, &after)
                } else {
                    resolve::resolve(path, specifier, &before)
                        != resolve::resolve(path, specifier, &after)
                }
            });
            if moved {
                set.unchanged.remove(path);
                set.resolution_changed.insert(path.clone());
            }
        }
    }

    // Documents. See the module documentation for why the path set here is the whole one and why
    // only an anchored destination depends on the target's bytes.
    let before_all: BTreeSet<String> = previous.keys().cloned().collect();
    let after_all: BTreeSet<String> = current.keys().cloned().collect();
    let path_set_moved = before_all != after_all;
    for (path, module) in previous {
        if !path_is_document(path) || !after_all.contains(path) || set.modified.contains(path) {
            continue;
        }
        let Some(facts) = &module.facts else { continue };
        let moved = facts.document.destinations.iter().any(|destination| {
            let now = docref::resolve_path(path, destination, &after_all);
            // A target added, removed or moved: the destination names a different file, or has
            // stopped naming one at all.
            if path_set_moved && now != docref::resolve_path(path, destination, &before_all) {
                return true;
            }
            // A target edited, under a line anchor.
            match (docref::anchor_of(destination), &now) {
                (Some(_), Some(target)) => {
                    previous
                        .get(target)
                        .map(|module| module.content_hash.as_str())
                        != current.get(target).map(String::as_str)
                }
                _ => false,
            }
        });
        // A **bare** supersession identifier (Slice 5d-ii) resolves against the ADR identifiers
        // parsed from indexed document file names, so adding, deleting or renaming a document can
        // move its answer — including into and out of ambiguity — with the citing document's
        // bytes unchanged. A link-form target needs nothing extra here: the supersession field's
        // link is one of the cached `destinations` above, and it resolves by the same rule.
        let identifier_moved = path_set_moved
            && facts
                .document
                .supersession
                .iter()
                .filter(|statement| statement.link.is_none())
                .any(|statement| {
                    docref::resolve_adr_identifier(&statement.target, &after_all)
                        != docref::resolve_adr_identifier(&statement.target, &before_all)
                });
        if moved || identifier_moved {
            set.unchanged.remove(path);
            set.resolution_changed.insert(path.clone());
        }
    }

    set
}

/// Files that must be re-extracted: the seed plus everything that imports it, transitively.
///
/// Removed files are seeds but never targets — they no longer exist to extract. The walk is over
/// module entity ids, which are a pure function of `(project_id, rel_path)`, so a removed file's
/// importers are still reachable even though its rows are about to be deleted.
pub fn invalidation_set(
    conn: &nerve_store::Connection,
    project_id: &str,
    seed: &BTreeSet<String>,
    known_paths: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    // Reverse map from module entity id to path, over every path either tree knows about.
    let mut path_of: BTreeMap<String, String> = BTreeMap::new();
    for path in known_paths.iter().chain(seed.iter()) {
        path_of.insert(ids::module_id(project_id, path), path.clone());
    }

    let mut visited: BTreeSet<String> = seed.clone();
    let mut frontier: Vec<String> = seed.iter().cloned().collect();

    while let Some(path) = frontier.pop() {
        let module_entity = ids::module_id(project_id, &path);
        for importer in nerve_store::importers_of(conn, &module_entity)? {
            let Some(importer_path) = path_of.get(&importer) else {
                // An `IMPORTS` source that is not a module of a path we know about. Nothing in
                // the current pipeline produces one; it is skipped rather than guessed at.
                continue;
            };
            if visited.insert(importer_path.clone()) {
                frontier.push(importer_path.clone());
            }
        }
    }

    Ok(visited)
}

/// A proposed file move, with the evidence that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveProposal {
    /// Path the file had.
    pub from_path: String,
    /// Path it now has.
    pub to_path: String,
    /// Symbols matching on `(kind, name, scope_path, body digest)`.
    pub matched: usize,
    /// Symbols the old file declared.
    pub from_symbols: usize,
    /// Symbols the new file declares.
    pub to_symbols: usize,
    /// Whether the two files are byte-identical.
    pub content_hash_equal: bool,
    /// The matching symbol pairs, old first.
    pub pairs: Vec<(CachedSymbol, CachedSymbol)>,
}

/// A file the identity-link producer is asked to consider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveCandidate {
    /// Repository-relative path.
    pub rel_path: String,
    /// BLAKE3 of the file's bytes.
    pub content_hash: String,
    /// Symbols it declares.
    pub symbols: Vec<CachedSymbol>,
}

/// Minimum share of symbols that must match for a pair of files to be proposed as a move.
///
/// Expressed as a fraction so the comparison stays in integer arithmetic; a float threshold on a
/// similarity score is exactly the kind of unfalsifiable number ADR-0003 refuses.
pub const MOVE_SIMILARITY_NUMERATOR: usize = 1;
/// Denominator of [`MOVE_SIMILARITY_NUMERATOR`].
pub const MOVE_SIMILARITY_DENOMINATOR: usize = 2;

fn matched_pairs(from: &MoveCandidate, to: &MoveCandidate) -> Vec<(CachedSymbol, CachedSymbol)> {
    let mut available: Vec<bool> = vec![true; to.symbols.len()];
    let mut pairs = Vec::new();
    for old in &from.symbols {
        let key = old.identity_key();
        if let Some(position) = to
            .symbols
            .iter()
            .enumerate()
            .position(|(index, new)| available[index] && new.identity_key() == key)
        {
            available[position] = false;
            pairs.push((old.clone(), to.symbols[position].clone()));
        }
    }
    pairs
}

/// Propose file moves between the files that vanished and the files that appeared.
///
/// Two rules, both deliberate (ADR-0002, ARCHITECTURE.md extension point 3):
///
/// - A symbol matches on `(kind, name, scope_path, body digest)`. **Name alone never matches.**
///   Two unrelated files that happen to declare `parse` are not evidence of anything, and the
///   whole point of an identity link is that it carries evidence.
/// - A file pair is proposed only when a majority of the larger side's symbols match, and only
///   when one candidate is strictly best. A tie is ambiguity, and ambiguity is a refusal here
///   for the same reason it is in selector resolution: guessing produces a confident wrong
///   answer, which is the failure mode this product exists to avoid.
///
/// The result is *proposals*. Nothing merges the two identities.
pub fn propose_moves(removed: &[MoveCandidate], added: &[MoveCandidate]) -> Vec<MoveProposal> {
    let mut proposals: Vec<MoveProposal> = Vec::new();
    let mut claimed: BTreeSet<String> = BTreeSet::new();

    for from in removed {
        let mut best: Option<(usize, usize, MoveProposal)> = None;
        let mut tied = false;

        for to in added {
            if claimed.contains(&to.rel_path) {
                continue;
            }
            let pairs = matched_pairs(from, to);
            let matched = pairs.len();
            if matched == 0 {
                continue;
            }
            let larger = from.symbols.len().max(to.symbols.len());
            if matched * MOVE_SIMILARITY_DENOMINATOR < larger * MOVE_SIMILARITY_NUMERATOR {
                continue;
            }

            let proposal = MoveProposal {
                from_path: from.rel_path.clone(),
                to_path: to.rel_path.clone(),
                matched,
                from_symbols: from.symbols.len(),
                to_symbols: to.symbols.len(),
                content_hash_equal: from.content_hash == to.content_hash,
                pairs,
            };

            match &best {
                None => best = Some((matched, larger, proposal)),
                Some((best_matched, best_larger, _)) => {
                    // Higher match count wins; on equal counts, the tighter fit (fewer unmatched
                    // symbols) wins. Exact equality on both is ambiguity.
                    let better = matched > *best_matched
                        || (matched == *best_matched && larger < *best_larger);
                    let same = matched == *best_matched && larger == *best_larger;
                    if better {
                        best = Some((matched, larger, proposal));
                        tied = false;
                    } else if same {
                        tied = true;
                    }
                }
            }
        }

        if tied {
            continue;
        }
        if let Some((_, _, proposal)) = best {
            claimed.insert(proposal.to_path.clone());
            proposals.push(proposal);
        }
    }

    proposals.sort_by(|a, b| (&a.from_path, &a.to_path).cmp(&(&b.from_path, &b.to_path)));
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, body: &str) -> CachedSymbol {
        CachedSymbol {
            entity_id: format!("fn_{name}_{body}"),
            kind: "function".to_string(),
            name: name.to_string(),
            scope_path: String::new(),
            body_hash: ids::content_hash(body.as_bytes()),
        }
    }

    fn candidate(path: &str, symbols: Vec<CachedSymbol>) -> MoveCandidate {
        MoveCandidate {
            rel_path: path.to_string(),
            content_hash: ids::content_hash(path.as_bytes()),
            symbols,
        }
    }

    fn previous(entries: &[(&str, &str)]) -> BTreeMap<String, PreviousModule> {
        entries
            .iter()
            .map(|(path, hash)| {
                (
                    (*path).to_string(),
                    PreviousModule {
                        content_hash: (*hash).to_string(),
                        reusable: true,
                        facts: Some(ModuleFacts::default()),
                    },
                )
            })
            .collect()
    }

    fn current(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(path, hash)| ((*path).to_string(), (*hash).to_string()))
            .collect()
    }

    #[test]
    fn classification_separates_the_four_outcomes() {
        let set = classify(
            &previous(&[("a.ts", "h1"), ("b.ts", "h2"), ("gone.ts", "h3")]),
            &current(&[("a.ts", "h1"), ("b.ts", "changed"), ("new.ts", "h4")]),
            false,
        );
        assert_eq!(set.unchanged, ["a.ts".to_string()].into());
        assert_eq!(set.modified, ["b.ts".to_string()].into());
        assert_eq!(set.added, ["new.ts".to_string()].into());
        assert_eq!(set.removed, ["gone.ts".to_string()].into());
        assert!(!set.is_unchanged());
    }

    #[test]
    fn an_unchanged_tree_seeds_nothing() {
        let set = classify(
            &previous(&[("a.ts", "h1")]),
            &current(&[("a.ts", "h1")]),
            false,
        );
        assert!(set.is_unchanged());
        assert!(set.seed().is_empty());
    }

    #[test]
    fn full_marks_every_present_file_modified() {
        let set = classify(
            &previous(&[("a.ts", "h1")]),
            &current(&[("a.ts", "h1"), ("b.ts", "h2")]),
            true,
        );
        assert!(set.unchanged.is_empty());
        assert_eq!(set.modified, ["a.ts".to_string()].into());
        assert_eq!(set.added, ["b.ts".to_string()].into());
    }

    #[test]
    fn a_superseded_extractor_version_counts_as_modified() {
        let mut before = previous(&[("a.ts", "h1")]);
        before.get_mut("a.ts").unwrap().reusable = false;
        let set = classify(&before, &current(&[("a.ts", "h1")]), false);
        assert_eq!(set.modified, ["a.ts".to_string()].into());
    }

    /// Adding a file can make a previously unresolved specifier resolve. The importer has no
    /// `IMPORTS` edge to the new file — its edge points at an `Unresolved` entity — so the graph
    /// walk cannot find it and the specifier comparison must.
    #[test]
    fn an_added_file_that_satisfies_a_dangling_specifier_seeds_its_importer() {
        let mut before = previous(&[("src/app.ts", "h1")]);
        before.get_mut("src/app.ts").unwrap().facts = Some(ModuleFacts {
            import_specifiers: vec!["./impl".to_string()],
            ..ModuleFacts::default()
        });
        let set = classify(
            &before,
            &current(&[("src/app.ts", "h1"), ("src/impl.ts", "h2")]),
            false,
        );
        assert_eq!(set.added, ["src/impl.ts".to_string()].into());
        assert_eq!(
            set.resolution_changed,
            ["src/app.ts".to_string()].into(),
            "the importer must be re-extracted even though its bytes did not change"
        );
        assert!(set.unchanged.is_empty());
    }

    /// Adding a higher-priority candidate changes where an already-resolved specifier points.
    #[test]
    fn an_added_file_that_outranks_the_current_resolution_seeds_its_importer() {
        let mut before = previous(&[("src/app.ts", "h1"), ("src/math.js", "h2")]);
        before.get_mut("src/app.ts").unwrap().facts = Some(ModuleFacts {
            import_specifiers: vec!["./math".to_string()],
            ..ModuleFacts::default()
        });
        let set = classify(
            &before,
            &current(&[
                ("src/app.ts", "h1"),
                ("src/math.js", "h2"),
                ("src/math.ts", "h3"),
            ]),
            false,
        );
        assert_eq!(set.resolution_changed, ["src/app.ts".to_string()].into());
    }

    /// A document is not in the set `resolve` consults, so adding or deleting one cannot move a
    /// specifier and must not seed a re-resolution of every cached module.
    #[test]
    fn adding_or_removing_a_document_never_seeds_a_resolution_recheck() {
        let mut before = previous(&[("src/app.ts", "h1"), ("docs/gone.md", "h9")]);
        before.get_mut("src/app.ts").unwrap().facts = Some(ModuleFacts {
            import_specifiers: vec!["./impl".to_string(), "./gone".to_string()],
            ..ModuleFacts::default()
        });
        let set = classify(
            &before,
            &current(&[("src/app.ts", "h1"), ("docs/added.md", "h2")]),
            false,
        );
        assert_eq!(set.added, ["docs/added.md".to_string()].into());
        assert_eq!(set.removed, ["docs/gone.md".to_string()].into());
        assert!(
            set.resolution_changed.is_empty(),
            "a document changed what a specifier resolves to: {set:?}"
        );
        assert_eq!(set.unchanged, ["src/app.ts".to_string()].into());
    }

    /// A document is a file like any other as far as change detection goes.
    #[test]
    fn a_changed_document_is_classified_like_any_other_file() {
        let set = classify(
            &previous(&[("docs/a.md", "h1"), ("docs/b.md", "h2")]),
            &current(&[("docs/a.md", "h1"), ("docs/b.md", "changed")]),
            false,
        );
        assert_eq!(set.unchanged, ["docs/a.md".to_string()].into());
        assert_eq!(set.modified, ["docs/b.md".to_string()].into());
        assert_eq!(set.seed(), ["docs/b.md".to_string()].into());
    }

    #[test]
    fn a_file_set_that_did_not_change_needs_no_resolution_recheck() {
        let mut before = previous(&[("src/app.ts", "h1"), ("src/math.ts", "h2")]);
        before.get_mut("src/app.ts").unwrap().facts = Some(ModuleFacts {
            import_specifiers: vec!["./math".to_string()],
            ..ModuleFacts::default()
        });
        let set = classify(
            &before,
            &current(&[("src/app.ts", "h1"), ("src/math.ts", "changed")]),
            false,
        );
        assert!(set.resolution_changed.is_empty());
        assert_eq!(set.modified, ["src/math.ts".to_string()].into());
    }

    /// Give a previously indexed document a cached destination list.
    fn with_destinations(
        previous: &mut BTreeMap<String, PreviousModule>,
        path: &str,
        destinations: &[&str],
    ) {
        previous.get_mut(path).unwrap().facts = Some(ModuleFacts {
            document: crate::facts::DocumentCounters {
                destinations: destinations.iter().map(|d| (*d).to_string()).collect(),
                ..crate::facts::DocumentCounters::default()
            },
            ..ModuleFacts::default()
        });
    }

    /// The four ways the *path set* can move a document link's answer.
    #[test]
    fn a_document_is_seeded_when_a_destination_would_now_resolve_differently() {
        // Added: `./guide.md` named nothing, and now names a file.
        let mut before = previous(&[("README.md", "h1")]);
        with_destinations(&mut before, "README.md", &["./docs/guide.md"]);
        let set = classify(
            &before,
            &current(&[("README.md", "h1"), ("docs/guide.md", "h2")]),
            false,
        );
        assert_eq!(set.resolution_changed, ["README.md".to_string()].into());
        assert!(set.unchanged.is_empty());

        // Removed: the link goes from resolved to broken, which is the signal, not the absence
        // of one.
        let mut before = previous(&[("README.md", "h1"), ("docs/guide.md", "h2")]);
        with_destinations(&mut before, "README.md", &["./docs/guide.md"]);
        let set = classify(&before, &current(&[("README.md", "h1")]), false);
        assert_eq!(set.resolution_changed, ["README.md".to_string()].into());

        // Moved: the destination stops naming the file it named.
        let mut before = previous(&[("README.md", "h1"), ("docs/guide.md", "h2")]);
        with_destinations(&mut before, "README.md", &["./docs/guide.md"]);
        let set = classify(
            &before,
            &current(&[("README.md", "h1"), ("docs/moved/guide.md", "h2")]),
            false,
        );
        assert_eq!(set.resolution_changed, ["README.md".to_string()].into());

        // Untouched: an unrelated file appearing must not re-resolve every document.
        let mut before = previous(&[("README.md", "h1"), ("docs/guide.md", "h2")]);
        with_destinations(&mut before, "README.md", &["./docs/guide.md"]);
        let set = classify(
            &before,
            &current(&[
                ("README.md", "h1"),
                ("docs/guide.md", "h2"),
                ("src/new.ts", "h3"),
            ]),
            false,
        );
        assert!(set.resolution_changed.is_empty(), "{set:?}");
        assert!(set.unchanged.contains("README.md"));
    }

    /// Give a previously indexed document a cached bare supersession identifier.
    fn with_supersedes(previous: &mut BTreeMap<String, PreviousModule>, path: &str, target: &str) {
        previous.get_mut(path).unwrap().facts = Some(ModuleFacts {
            document: crate::facts::DocumentCounters {
                supersession: vec![crate::facts::CachedSupersession {
                    direction: "supersedes".to_string(),
                    target: target.to_string(),
                    link: None,
                }],
                ..crate::facts::DocumentCounters::default()
            },
            ..ModuleFacts::default()
        });
    }

    /// A bare `ADR-<digits>` target resolves against the identifiers parsed from document file
    /// names, so the path set can move its answer — including into and out of ambiguity — while
    /// the citing document's bytes stand still.
    #[test]
    fn a_document_is_seeded_when_a_bare_supersession_identifier_would_resolve_differently() {
        let head = "docs/decisions/ADR-0900-head.md";
        let target = "docs/decisions/ADR-0901-target.md";
        let duplicate = "notes/ADR-0901-duplicate.md";

        // Added: `ADR-0901` named nothing, and now names exactly one document.
        let mut before = previous(&[(head, "h1")]);
        with_supersedes(&mut before, head, "ADR-0901");
        let set = classify(&before, &current(&[(head, "h1"), (target, "h2")]), false);
        assert_eq!(set.resolution_changed, [head.to_string()].into());

        // Removed: the edge goes from resolved to unresolved, which is the signal.
        let mut before = previous(&[(head, "h1"), (target, "h2")]);
        with_supersedes(&mut before, head, "ADR-0901");
        let set = classify(&before, &current(&[(head, "h1")]), false);
        assert_eq!(set.resolution_changed, [head.to_string()].into());

        // Ambiguous: a second document carrying the identifier must withdraw the edge, so the
        // document that cited it has to be re-extracted.
        let mut before = previous(&[(head, "h1"), (target, "h2")]);
        with_supersedes(&mut before, head, "ADR-0901");
        let set = classify(
            &before,
            &current(&[(head, "h1"), (target, "h2"), (duplicate, "h3")]),
            false,
        );
        assert_eq!(set.resolution_changed, [head.to_string()].into());

        // Untouched: an unrelated document appearing must not re-resolve every ADR.
        let mut before = previous(&[(head, "h1"), (target, "h2")]);
        with_supersedes(&mut before, head, "ADR-0901");
        let set = classify(
            &before,
            &current(&[(head, "h1"), (target, "h2"), ("docs/other.md", "h3")]),
            false,
        );
        assert!(set.resolution_changed.is_empty(), "{set:?}");
        assert!(set.unchanged.contains(head));
    }

    /// A **link**-form supersession target needs nothing of its own here: the field's link is one
    /// of the cached destinations, and it re-resolves by the rule above.
    #[test]
    fn a_link_form_supersession_target_is_covered_by_the_cached_destinations() {
        let head = "docs/decisions/ADR-0900-head.md";
        let mut before = previous(&[(head, "h1")]);
        before.get_mut(head).unwrap().facts = Some(ModuleFacts {
            document: crate::facts::DocumentCounters {
                destinations: vec!["ADR-0901-target.md".to_string()],
                supersession: vec![crate::facts::CachedSupersession {
                    direction: "supersedes".to_string(),
                    target: "[ADR-0901](ADR-0901-target.md)".to_string(),
                    link: Some("ADR-0901-target.md".to_string()),
                }],
                ..crate::facts::DocumentCounters::default()
            },
            ..ModuleFacts::default()
        });
        let set = classify(
            &before,
            &current(&[(head, "h1"), ("docs/decisions/ADR-0901-target.md", "h2")]),
            false,
        );
        assert_eq!(set.resolution_changed, [head.to_string()].into());
    }

    /// A line anchor is resolved against the target's symbols and records the hash it was
    /// resolved at, so editing the target must re-extract the document. A link with no anchor
    /// depends only on the file existing, and must not.
    #[test]
    fn only_an_anchored_destination_depends_on_the_targets_bytes() {
        let mut before = previous(&[("README.md", "h1"), ("src/util.ts", "h2")]);
        with_destinations(&mut before, "README.md", &["./src/util.ts#L13"]);
        let set = classify(
            &before,
            &current(&[("README.md", "h1"), ("src/util.ts", "edited")]),
            false,
        );
        assert_eq!(
            set.resolution_changed,
            ["README.md".to_string()].into(),
            "an anchored link survived an edit to the file it points into"
        );

        let mut before = previous(&[("README.md", "h1"), ("src/util.ts", "h2")]);
        with_destinations(&mut before, "README.md", &["./src/util.ts"]);
        let set = classify(
            &before,
            &current(&[("README.md", "h1"), ("src/util.ts", "edited")]),
            false,
        );
        assert!(
            set.resolution_changed.is_empty(),
            "an unanchored link made a document depend on a file's contents: {set:?}"
        );
        assert!(set.unchanged.contains("README.md"));
    }

    /// A destination that resolves to nothing in either tree is not a reason to do work.
    #[test]
    fn a_destination_that_names_nothing_either_way_seeds_nothing() {
        let mut before = previous(&[("README.md", "h1"), ("src/a.ts", "h2")]);
        with_destinations(
            &mut before,
            "README.md",
            &[
                "https://example.invalid/x",
                "#heading",
                "../../../etc/passwd",
                "./missing.md",
            ],
        );
        let set = classify(
            &before,
            &current(&[("README.md", "h1"), ("src/a.ts", "h2"), ("src/b.ts", "h3")]),
            false,
        );
        assert!(set.resolution_changed.is_empty(), "{set:?}");
    }

    #[test]
    fn an_identical_file_at_a_new_path_is_proposed_as_a_move() {
        let symbols = vec![symbol("alpha", "a"), symbol("beta", "b")];
        let proposals = propose_moves(
            &[candidate("src/old.ts", symbols.clone())],
            &[candidate("src/new.ts", symbols)],
        );
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].from_path, "src/old.ts");
        assert_eq!(proposals[0].to_path, "src/new.ts");
        assert_eq!(proposals[0].matched, 2);
        assert_eq!(proposals[0].pairs.len(), 2);
    }

    /// The negative case the plan requires: same name, different body, unrelated files.
    #[test]
    fn a_coincidental_name_match_is_not_a_move() {
        let proposals = propose_moves(
            &[candidate("src/old.ts", vec![symbol("parse", "one")])],
            &[candidate("src/unrelated.ts", vec![symbol("parse", "two")])],
        );
        assert!(
            proposals.is_empty(),
            "a name match with a different body is not evidence of identity"
        );
    }

    #[test]
    fn a_mostly_different_file_is_not_a_move() {
        let proposals = propose_moves(
            &[candidate(
                "src/old.ts",
                vec![symbol("shared", "s"), symbol("a", "a"), symbol("b", "b")],
            )],
            &[candidate(
                "src/other.ts",
                vec![symbol("shared", "s"), symbol("c", "c"), symbol("d", "d")],
            )],
        );
        assert!(proposals.is_empty(), "one symbol in three is not a move");
    }

    #[test]
    fn an_ambiguous_move_is_refused_rather_than_guessed() {
        let symbols = vec![symbol("alpha", "a")];
        let proposals = propose_moves(
            &[candidate("src/old.ts", symbols.clone())],
            &[
                candidate("src/one.ts", symbols.clone()),
                candidate("src/two.ts", symbols),
            ],
        );
        assert!(
            proposals.is_empty(),
            "two equally good candidates must not be resolved by file order"
        );
    }

    #[test]
    fn a_file_with_no_symbols_proposes_nothing() {
        let proposals = propose_moves(
            &[candidate("src/old.ts", vec![])],
            &[candidate("src/new.ts", vec![])],
        );
        assert!(proposals.is_empty());
    }

    #[test]
    fn each_added_file_is_claimed_at_most_once() {
        let symbols = vec![symbol("alpha", "a"), symbol("beta", "b")];
        let mut second = symbols.clone();
        second.push(symbol("gamma", "g"));
        let proposals = propose_moves(
            &[
                candidate("src/first.ts", symbols.clone()),
                candidate("src/second.ts", symbols.clone()),
            ],
            &[candidate("src/new.ts", symbols)],
        );
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].from_path, "src/first.ts");
    }
}
