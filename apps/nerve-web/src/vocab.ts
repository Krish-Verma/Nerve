/**
 * Plain-English readings of the reason codes the indexer records.
 *
 * These codes are Nerve's honest account of where it stopped. Printed raw they look like defects;
 * read properly most of them are statements about the *language*, not about the extractor — a
 * method call on a value whose type is not written down cannot be resolved by reading syntax, and
 * saying so is the correct answer rather than a failure to give one.
 *
 * Anything unrecognised falls through as the code itself. Making one up would be worse.
 */

const UNRESOLVED_REASON: Record<string, string> = {
  'import-module-unresolved':
    'The imported file could not be located from this import path, so nothing behind it is known.',
  'import-name-not-exported':
    'The module was found, but it does not export this name under this name.',
  'import-named': 'A named import whose target could not be pinned down.',
  'import-default': 'A default import whose target could not be pinned down.',
  'import-namespace': 'A namespace import; which member is meant is decided where it is used.',
  'namespace-member': 'A member reached through a namespace import that could not be followed.',
  'dynamic-import': 'The import path is computed at runtime, so it is not a fact about the source.',
  'name-not-in-scope': 'No declaration of this name is visible from here.',
  'local-binding-not-a-symbol':
    'The name is bound locally to something that is not a declared symbol.',
  'receiver-not-resolvable':
    'A method call on a value whose type is not written down. Resolving it needs type inference, which this build does not do.',
  'complex-receiver': 'The thing being called is an expression, not a name.',
  'method-not-declared-on-class': 'The class is known and does not declare this member.',
  'this-member': 'A member reached through `this` that is not declared on the enclosing class.',
  'this-not-in-class-method': '`this` was used outside a class method, so it names nothing fixed.',
  'this-rebound-by-nested-function':
    '`this` is rebound by an enclosing function, so what it refers to depends on the call.',
  'heritage-other': 'An extends or implements clause that is not a plain name.',
  'tagged-template': 'A tagged template whose tag could not be resolved.',

  // Document references. A link in prose and a supersession field are both a document naming
  // something, so they fail in the same small number of ways and are read here together.
  document_link_target_not_indexed:
    'The link points at a repository path that names no indexed file. This is what a stale documentation link looks like.',
  document_link_refused:
    'The path guard refused the destination, so the file was never opened. Nothing outside the repository is read.',
  document_anchor_no_symbol:
    'The `#L` line anchor resolved to no symbol — either nothing is declared on that line, or the line is past the end of the file.',
  document_supersedes_target_not_indexed:
    'The document says it supersedes something this repository does not contain as a document.',
  document_supersedes_target_ambiguous:
    'More than one indexed document answers to that identifier, so which one was meant is not decidable. It is refused rather than guessed.',
  document_supersedes_self: 'The document names itself. A decision cannot replace itself.',
  document_supersedes_unparsed:
    'A supersession field is present, but its value is empty, too long, or not in a form that names a target.',
};

/**
 * The reading of a reason.
 *
 * Not every reason is a code. The indexer also records reasons as prose — "relative specifier
 * does not name an indexed file" is already the explanation — and glossing those with "this build
 * has no description" would be telling the reader nothing while looking like a defect. So a value
 * that contains a space is returned as it stands.
 */
export function unresolvedReason(code: string): string {
  const known = UNRESOLVED_REASON[code];
  if (known) return known;
  return code.includes(' ') ? code : 'This build has no description for that reason code.';
}

/** A short label for a reason, for places where the full sentence will not fit. */
export function reasonLabel(code: string): string {
  return code.includes(' ') ? code.replace(/\s+/g, '-') : code;
}

const UNMODELLED_FORM: Record<string, string> = {
  'call-result': 'Calling the result of another call.',
  'complex-receiver': 'The thing being called is an expression rather than a name.',
  'computed-member': 'A member reached by a computed key, such as `object[name]()`.',
  'heritage-call': 'A class extending the result of a call — a mixin factory.',
  'heritage-other': 'An extends or implements clause that is not a plain name.',
  iife: 'A function expression invoked where it is written.',
  require: 'A CommonJS `require`, which this build does not model.',
  super: 'A `super` call.',
  'dynamic-import': 'An `import()` whose path is computed.',
  'tagged-template': 'A tagged template literal.',
  other: 'A call shape this extractor does not have a rule for.',
};

export function unmodelledForm(code: string): string {
  return UNMODELLED_FORM[code] ?? 'This build has no description for that call shape.';
}

/** What an entity kind is, for a reader who has not read the schema. */
const KIND_GLOSS: Record<string, string> = {
  repository: 'The indexed repository itself.',
  directory: 'A directory that holds indexed files.',
  file: 'One file on disk.',
  module: 'The module a file exports, as opposed to the file it is stored in.',
  function: 'A declared function.',
  method: 'A function declared on a class or interface.',
  class: 'A declared class.',
  interface: 'A declared interface.',
  document: 'A prose document Nerve read as text rather than as code. For Markdown, one per file.',
  section: 'A heading and everything under it, up to the next heading of the same level or higher.',
  unresolved:
    'A reference Nerve recorded but could not connect to a declaration. It is kept, not discarded.',
  coverage_run:
    'One coverage report Nerve was told to read, identified by its path and its contents. It stands for the whole test run, not for any single test — the report records no test names.',
  endpoint:
    'An entry point a framework declares, such as an HTTP route, named by the address the source gives it. It is the declaration, not a promise: it does not mean the route is reachable, that middleware allows it, or that configuration has not replaced it.',
};

export function kindGloss(kind: string): string {
  return KIND_GLOSS[kind] ?? 'This build has no description for that entity kind.';
}

/**
 * What the ingested coverage evidence says about one symbol.
 *
 * The two values that both mean "not covered" are deliberately kept apart, and the difference is
 * the whole reason this table is written out rather than reduced to a red dot. `uncovered` is a
 * measurement: a run instrumented the file and nothing inside this symbol ran. `unmeasured` is
 * silence: no coverage evidence names the file at all, so the symbol may be excluded from
 * instrumentation, may never be loaded by the suite, or may not be reachable — Nerve does not
 * know which. Rendering the second as the first would claim a measurement nobody took.
 */
const COVERAGE_STATE: Record<string, string> = {
  covered: 'Every instrumented line inside this symbol executed during an ingested coverage run.',
  partial:
    'Some instrumented lines inside this symbol ran and some did not. A line that ran proves the symbol was entered, not that it ran through, so this is counted as neither covered nor a gap.',
  uncovered:
    'A coverage run measured the file this symbol is in, and no line inside the symbol ran. The absence is a measurement.',
  unmeasured:
    'No coverage evidence names the file this symbol is in. The absence is silence rather than a measurement: the file may be excluded from instrumentation, or never loaded by the suite at all.',
};

export function coverageState(value: string): string {
  return COVERAGE_STATE[value] ?? 'This build has no description for that coverage state.';
}

/**
 * What an unresolved entity was standing in for.
 *
 * The category is part of the entity's identity, not decoration: an unresolvable import named
 * `parse` and an unresolvable call to `parse()` are different missing things, and collapsing
 * them would have Nerve claim a module and a value are one entity.
 */
const UNRESOLVED_CATEGORY: Record<string, string> = {
  module: 'A module specifier that named no indexed module.',
  value: 'A value or type name that no binding in scope could account for.',
  document_link: 'A link in a document — or its line anchor — that named nothing indexed.',
  document_supersedes: 'A supersession field in an ADR that named no single indexed document.',
};

export function unresolvedCategory(value: string): string {
  return UNRESOLVED_CATEGORY[value] ?? 'This build has no description for that unresolved category.';
}

/**
 * The status an ADR gives itself, read from its own header.
 *
 * `unparsed` is not an absence and not an error: it means the document *did* state a status and
 * the word was outside the vocabulary. Nerve records that rather than coercing it to the nearest
 * value, because the raw text is preserved and a guessed status would outrank the evidence.
 */
const ADR_STATUS: Record<string, string> = {
  Proposed: 'Put forward, not yet agreed.',
  Accepted: 'In force. This is the decision that currently governs.',
  Rejected: 'Considered and turned down. It never took effect.',
  Deprecated: 'No longer to be followed, with nothing named as its replacement.',
  Superseded: 'Replaced by a later decision.',
  unparsed:
    'The document states a status, and the word is not one of the five. The raw text is kept; Nerve does not guess which was meant.',
};

export function adrStatusGloss(value: string): string {
  return ADR_STATUS[value] ?? 'This build has no description for that ADR status.';
}

/*
 * ---- the history vocabularies -----------------------------------------------------------------
 *
 * Eight closed vocabularies arrived with Slices 12b and 12c-i-a and none of them was mirrored
 * here, so `crates/nerve-server/tests/ui_vocabulary.rs` could not fail for them — it did not know
 * they existed. The tables below are that mirror, and the guard gained an `every_*_is_glossed`
 * test for each in the same commit. Slice 5d-iii is what happens otherwise: an entire corrective
 * slice, and 120 sites rendering "this build has no description".
 *
 * **A gloss is not a note.** Each of these vocabularies also has a `note()` in Rust, and the API
 * sends that prose on the response — `walk_terminated_note`, `changes_enumerated_note`,
 * `kind_note`, `may_claim_created_note`. Those sentences are the backend's, they are owned in one
 * place, and `crates/nerve-cli/tests/history_wording.rs` fails a second copy of one by name. What
 * is written here is the other thing: a short reading of what a *value* means to somebody who has
 * not read the schema, in this interface's own voice. Where a view can show both, it shows both.
 */

/**
 * What one commit did to one path.
 *
 * There is deliberately no `renamed` member and this table must not grow one. Git records no
 * rename; a rename is proposed from identical content and carries its own evidence and ambiguity,
 * which is why it is a hypothesis with its own two vocabularies below rather than a fifth change
 * kind. `mode_changed` is kept apart from `modified` for the opposite reason: the bytes did not
 * move, and calling it a modification would claim content changed when only permission did.
 */
const CHANGE_KIND: Record<string, string> = {
  added: 'The path is in this commit and was not in the tree it was compared against.',
  modified: 'The path is in both trees and its content differs between them.',
  deleted: 'The path was in the compared tree and is not in this one.',
  mode_changed:
    'The file mode changed and the bytes did not. A real change to a tracked file, and not a content move.',
};

export function changeKindGloss(value: string): string {
  return CHANGE_KIND[value] ?? 'This build has no description for that change kind.';
}

/**
 * Why a commit has the parents it has — and, when none are visible, which reason that is.
 *
 * Two of these mean "cannot see further" and they are the reason the vocabulary exists. A
 * declared boundary is expected; an absent parent nobody declared is a fault; and the case where
 * Nerve could not tell which of those it was is a third answer rather than a rounding of either.
 * Only `root` licenses the sentence that history begins somewhere, and that permission is carried
 * on every commit as `may_claim_history_begins_here` rather than worked out again here.
 */
const PARENT_COMPLETENESS: Record<string, string> = {
  root: 'The commit names no parent and no boundary was declared, so nothing precedes it.',
  shallow_boundary:
    'The checkout declares that its copy of history stops at this commit. What lies above is out of reach here, which is not the same as absent.',
  parents_available: 'Every parent this commit names was found in the object store.',
  parents_missing:
    'A parent oid this commit lists was not found, and nothing declared it missing. That is a fault in the object store rather than a shallow checkout.',
  parents_unverifiable:
    'A listed parent was not found and the shallow declaration could not be read, so whether the absence was declared is undecided.',
};

export function parentCompletenessGloss(value: string): string {
  return (
    PARENT_COMPLETENESS[value] ?? 'This build has no description for that parent-completeness value.'
  );
}

/**
 * Which of four silences a commit with no change rows is.
 *
 * Reading a count of zero without this is the defect the column exists to prevent: two of the
 * four mean opposite things, one being "nothing happened" and the other "we cannot see what
 * happened".
 */
const CHANGES_ENUMERATED: Record<string, string> = {
  enumerated:
    'The diff against the single parent ran to the end, so a count of zero here means the commit really touched nothing.',
  merge_not_enumerated:
    'A merge. A change is only defined against one parent and a merge has several, so none were listed — by decision, not by failure.',
  parent_unavailable:
    'The parent tree could not be read, so there was nothing to compare against and no change was listed.',
  refused:
    'A bound stopped this commit being diffed, so no change was listed. The refusal is counted on the ingest that produced it.',
};

export function changesEnumeratedGloss(value: string): string {
  return CHANGES_ENUMERATED[value] ?? 'This build has no description for that enumeration state.';
}

/**
 * Why the walk that read this history stopped.
 *
 * `commit_budget` is the one that is Nerve's own doing. It says how far this read went and
 * nothing whatever about how far the repository goes back, and drawing it as a property of the
 * repository is the mistake the value exists to make visible.
 */
const WALK_TERMINATION: Record<string, string> = {
  exhausted:
    'The walk ran out of unvisited parents, so it saw everything reachable from where it started.',
  commit_budget:
    'Nerve stopped at its own ceiling. This is a fact about the read, and says nothing about how far the repository goes back.',
  shallow_boundary:
    'The walk reached a boundary this checkout declares. What lies above it is out of reach for this copy of the repository.',
  missing_object:
    'An object the walk needed was not in the store. A fault, and not a declared boundary.',
  refused: 'A bound refused an object the walk needed, so the walk stopped there rather than going past it.',
};

export function walkTerminationGloss(value: string): string {
  return WALK_TERMINATION[value] ?? 'This build has no description for that walk termination.';
}

/**
 * What a rename hypothesis rests on. There is no score here and none may be invented.
 *
 * The two values are never blended into one number. A blended figure would let a byte-identical
 * match and a 60%-similar one arrive at the same reading and become indistinguishable, which is
 * the whole reason this is a vocabulary rather than a percentage.
 */
const RENAME_EVIDENCE: Record<string, string> = {
  exact_content:
    'A path was deleted and another added in the same commit naming the same blob, so the content is byte-identical. No similarity was computed and no threshold was applied.',
  similar_content:
    'The two paths name different blobs, and a named method measured how much content they share. The measurement means nothing without that method, its version and the threshold it was admitted against, so those are shown beside it and never on their own.',
};

export function renameEvidenceGloss(value: string): string {
  return RENAME_EVIDENCE[value] ?? 'This build has no description for that rename evidence.';
}

/**
 * Whether a commit's similarity candidate set was measured in full.
 *
 * `refused_bound` is the value that cannot be a flag on a row, because there is no row: a commit
 * that exceeded a bound records no similarity hypothesis at all. Without this the empty result
 * reads as "nothing was renamed here", which is the opposite of what happened.
 */
const RENAME_ANALYSIS_COMPLETENESS: Record<string, string> = {
  complete:
    'Every candidate pair in this commit was measured, so the hypotheses shown are the full set for this matcher.',
  partial:
    'Some candidate pairs could not be measured. What is shown is not the full set, and the reasons are counted rather than summarised away.',
  refused_bound:
    'The candidate set exceeded a bound, so no similarity hypothesis was recorded for this commit at all. An empty result here is a refusal, not an absence of renames.',
  not_attempted:
    'The changes in this commit were not enumerated, so there was no candidate set to measure. That says nothing about whether the commit renamed anything.',
};

export function renameAnalysisCompletenessGloss(value: string): string {
  return (
    RENAME_ANALYSIS_COMPLETENESS[value] ??
    'This build has no description for that candidate-set completeness.'
  );
}

/**
 * Whether a stored commit summary is the whole first line or a cut one.
 *
 * Three values rather than a checkbox, and `unknown` is the one that earns the third. A summary of
 * exactly the stored bound is not truncated, so the length cannot be used to work the answer out
 * afterwards — a commit read before Nerve recorded this simply cannot be told either way.
 */
const SUMMARY_TRUNCATION: Record<string, string> = {
  complete: 'The whole first line of the commit message is here. Nothing was cut.',
  truncated:
    'The first line was longer than the stored bound and was cut, so this text ends where Nerve stopped rather than where the author did.',
  unknown:
    'This commit was recorded before Nerve stored whether a summary was cut, and it cannot be recovered. The length alone cannot tell a short first line from a cut one.',
};

export function summaryTruncationGloss(value: string): string {
  return SUMMARY_TRUNCATION[value] ?? 'This build has no description for that summary state.';
}

/**
 * Why a similarity candidate pair carries no measurement.
 *
 * Every one of these is an unanswered question rather than a negative answer. A pair that could not
 * be measured is not a pair that was measured and found unrelated, and reading it as one is the
 * mistake these reasons exist to prevent.
 */
const SIMILARITY_UNMEASURED: Record<string, string> = {
  'blob-absent':
    'The blob was not in the object store, so the pair could not be measured. An unanswered question, not a negative answer.',
  'blob-unreadable':
    'The blob was named by the tree and could not be read back, so the pair could not be measured.',
  'blob-too-large':
    'The blob was larger than the matcher will inflate, so it was refused rather than read. A bound Nerve set, not a property of the file.',
  'blob-binary':
    'The blob contains a NUL byte, so it has no lines, and a line ratio over it would be a number without a meaning.',
  'blob-too-small':
    'The blob has fewer lines than the floor beneath which a ratio is not a measurement. Two one-line files agreeing says nothing.',
};

export function similarityUnmeasuredGloss(value: string): string {
  return (
    SIMILARITY_UNMEASURED[value] ?? 'This build has no description for that unmeasured reason.'
  );
}

/**
 * How many ways a rename hypothesis could have been drawn.
 *
 * Identical content is ordinary — an empty file, a copied licence header, a barrel re-export — so
 * when one blob matches several paths every pairing is kept and none is ranked. A `many_to`
 * pairing drawn the way a `unique` one is drawn would show a guess as a record.
 */
const RENAME_AMBIGUITY: Record<string, string> = {
  unique:
    'One deleted path, one added path, one blob. The clearest shape available, and still a proposal rather than something Git recorded.',
  many_from:
    'Several deleted paths carry this blob, so more than one of them could be the origin. Every pairing is kept and none is ranked.',
  many_to:
    'Several added paths carry this blob, so more than one of them could be the destination. Every pairing is kept and none is ranked.',
  many_both:
    'Several paths on each side carry this blob. Every pairing is kept and none is ranked.',
};

export function renameAmbiguityGloss(value: string): string {
  return RENAME_AMBIGUITY[value] ?? 'This build has no description for that rename ambiguity.';
}

/**
 * What "when was this path first observed" actually answered.
 *
 * Six values, and exactly one of them is a creation. The permission to say so is
 * `may_claim_created` on the response and is never re-derived from the value — including here,
 * which is why the phrasing a view puts on screen comes from `firstObservedHeadline` in
 * `history.ts` and not from this table.
 */
const FIRST_OBSERVED_KIND: Record<string, string> = {
  created_in_visible_history:
    'An addition, with nothing out of reach above it and exactly one addition on record. This is the one answer of six that licenses the word created, and it rests on no clock.',
  earliest_visible_change:
    'The earliest change Nerve can see. Whether anything came before it is a separate question, and it is answered beside this one rather than assumed.',
  present_before_visible_history:
    'The path is in the current tree and no recorded commit touched it, so it is older than everything read. On a shallow checkout this is the ordinary answer, not an empty one.',
  absent_from_visible_history:
    'No recorded commit touched the path, and the current tree was consulted and does not hold it.',
  current_tree_unknown:
    'No recorded commit touched the path, and with no index the current tree could not be consulted, so whether it is there now is unknown rather than no.',
  no_history_ingested:
    'Nothing has been read here to answer from. That is a statement about this index, not about the path.',
};

export function firstObservedGloss(value: string): string {
  return FIRST_OBSERVED_KIND[value] ?? 'This build has no description for that first-observed answer.';
}

/**
 * Whether the recorded history still describes what is indexed now.
 *
 * `unverifiable` is not a cosmetic fourth value. Filing it under `current` is how an unfinished
 * comparison becomes a clean bill of health, which is the same distinction `nerve check` draws
 * between stale and unverified.
 */
const HISTORY_FRESHNESS: Record<string, string> = {
  current: 'The commit the ingest read as HEAD is the commit the newest indexed state records.',
  stale:
    'The ingest read a different HEAD from the one the newest indexed state records, so every fact here is true of that older commit. A qualification, not an error.',
  unverifiable:
    'The newest indexed state records no commit, so there was nothing to compare against. Unknown is a different answer from current.',
  no_history_ingested:
    'Nothing has been read here whose freshness could be judged. An absence rather than a verdict.',
};

export function historyFreshnessGloss(value: string): string {
  return HISTORY_FRESHNESS[value] ?? 'This build has no description for that history freshness.';
}

/*
 * ---- Slice 13a-i: the cross-repository vocabularies ------------------------------------------
 *
 * Four tables, declared one slice before any view imported them, and two more added in **13d** when
 * the Contracts view began rendering the rule and the ambiguity as bare tokens. All six are now on
 * `GLOSS_TABLES` in `crates/nerve-server/tests/ui_vocabulary.rs`, which is what makes the shipped
 * bundle carry their prose; `DECLARED_NOT_RENDERED` is empty as a result, and it stays as a list so
 * the next vocabulary declared ahead of its view has somewhere honest to sit.
 *
 * Declaring the gloss beside the vocabulary rather than waiting for the view is what Slice 12c-ii's
 * Pass A did, and the reason is the same: a vocabulary that reaches a surface before its gloss does
 * renders "this build has no description" to a real reader.
 */

/**
 * Whether a registered neighbour still counts.
 *
 * A removed entry is kept rather than deleted, and that is not tidiness: a link that resolved
 * through the entry has to be able to say which entry went away. A row that had been deleted could
 * not have reported its own ending.
 */
const REGISTRY_ENTRY_STATUS: Record<string, string> = {
  active:
    'This repository is registered as a neighbour and the entry still counts. Links resolved through it are current claims.',
  tombstoned:
    'The entry was removed and kept rather than deleted, so anything that rested on it can still say which neighbour went away, and when.',
};

export function registryEntryStatusGloss(value: string): string {
  return (
    REGISTRY_ENTRY_STATUS[value] ?? 'This build has no description for that registry entry status.'
  );
}

/**
 * Which stated declaration a cross-repository link was read out of.
 *
 * Every value names a file that says so. There is no value for a similar name, a matching endpoint
 * string, a nearby directory or a resemblance of any kind, because none of those is a declaration
 * and a link here is only ever quoted from one.
 */
const CONTRACT_RESOLUTION_METHOD: Record<string, string> = {
  manifest_declared:
    'A package manifest in this repository names the target directly. The link is quoted from that line, not inferred from a resemblance.',
  workspace_declared:
    'A workspace declaration lists the target as a member, so the two are stated to belong to one build rather than found beside each other on disk.',
  path_dependency_resolved:
    'A declared path dependency was followed to the directory it names, and the manifest in that directory was read. Both ends are stated in a file.',
  export_map_resolved:
    'An import specifier was resolved through the export map the target package declares, so the file at the far end is the one that package says the specifier means.',
};

export function contractResolutionMethodGloss(value: string): string {
  return (
    CONTRACT_RESOLUTION_METHOD[value] ??
    'This build has no description for that resolution method.'
  );
}

/**
 * Whether a recorded cross-repository link is still claimed.
 *
 * The same keep-rather-than-delete rule as a registry entry, one table over and for the same reason.
 */
const CONTRACT_LINK_STATUS: Record<string, string> = {
  active:
    'The declaration this link was drawn from was still in the file the last time this repository was read. That says nothing on its own about the state of the repository at the far end.',
  withdrawn:
    'The declaration is gone, or the registry entry it pointed through was removed. The record is kept so the ending can be shown at all.',
};

export function contractLinkStatusGloss(value: string): string {
  return CONTRACT_LINK_STATUS[value] ?? 'This build has no description for that link status.';
}

/**
 * The twelve ways a cross-repository link can have stopped describing the world.
 *
 * Twelve rather than one "stale", because a link has two repositories behind it and only one of
 * them is the repository being asked. Two pairs must not be read as one: a path that no longer
 * exists is not a path that now holds a different repository, and a part of the far side that was
 * never indexed is not a part that changed.
 */
const CONTRACT_FRESHNESS: Record<string, string> = {
  source_changed:
    'This repository has moved on since the link was worked out, so the declaration may no longer read the way it did. A remark about this end only.',
  target_changed:
    'The repository at the far end has moved on since the link was worked out, so what the recorded snapshot names may not be what is there now.',
  both_changed: 'Both repositories have moved on, so neither end of the link has been re-checked.',
  contract_version_mismatch:
    'The version this repository asks for and the version the other one declares are different numbers. Two recorded values that disagree, with no judgement about which is right.',
  target_repository_missing:
    'Nothing is at the registered path any more. This is the ordinary broken link, and it is a different situation from a path that now holds some other repository.',
  target_repository_moved:
    'Something is at the registered path and it is a different repository from the one registered. This is the dangerous one: every link through the entry would otherwise describe the wrong project.',
  contract_file_missing:
    'The manifest this link was quoted from is no longer in this repository, so there is nothing left to re-read the declaration out of.',
  duplicate_contract_identity:
    'More than one registered repository declares this contract under the same name, so which one is meant has not been established. Every candidate is shown and none is picked.',
  conflicting_definitions:
    'The contract is declared more than once and the declarations disagree, so there is no single stated fact to quote.',
  target_partially_indexed:
    'The other repository is readable, and the part this link names was never indexed there. Nothing was seen to change: this is unknown rather than out of date.',
  contract_deleted:
    'The declaration this link came from is gone from the manifest. The link is a record of something that used to be stated.',
  registry_entry_removed:
    'The registry entry this link went through was removed. The entry is kept as a marker precisely so this can be said.',
};

export function contractFreshnessGloss(value: string): string {
  return CONTRACT_FRESHNESS[value] ?? 'This build has no description for that contract freshness.';
}

/**
 * Which rule read the declaration a link was drawn from.
 *
 * Three, and the split that matters is not the language. Two of them link one repository to
 * another and name no file at either end; the third names a file inside the other repository, which
 * is why it is the only one that carries a snapshot of anything over there.
 */
const CONTRACT_KIND: Record<string, string> = {
  npm_local_dependency:
    'A dependency in package.json that states a path — a file: specifier, or a workspace member this repository lists itself. It links the two repositories and names no file at either end.',
  npm_export_resolution:
    'An import specifier resolved through the export map the other package declares, so the file at the far end is the one that package says the specifier means. This is the only rule that names a file over there.',
  python_path_dependency:
    'A local path dependency in pyproject.toml — PEP 621, Poetry or uv. It links the two repositories and names no file at either end.',
};

export function contractKindGloss(value: string): string {
  return CONTRACT_KIND[value] ?? 'This build has no description for that contract rule.';
}

/**
 * What was ambiguous about a resolution, when something was.
 *
 * Recorded on **every** row of the ambiguous identity, and none of them is preferred over another.
 * The evidence is that the manifest said the same thing twice; picking a winner would be inventing
 * the answer it declined to give.
 */
const CONTRACT_AMBIGUITY: Record<string, string> = {
  declared_more_than_once:
    'The same name is declared more than once and every declaration points at the same place. Nothing disagrees; the repetition is recorded rather than hidden.',
  conflicting_targets:
    'The same name is declared more than once and the declarations point at different places. Every candidate is kept and none is chosen, because the manifest did not choose either.',
};

export function contractAmbiguityGloss(value: string): string {
  return CONTRACT_AMBIGUITY[value] ?? 'This build has no description for that ambiguity.';
}

/*
 * ---- the memory vocabularies ------------------------------------------------------------------
 *
 * Three declared by Slice 14a and two by 14b-i, all five glossed in the change that declared them.
 * The first three sat on `DECLARED_NOT_RENDERED` in `crates/nerve-server/tests/ui_vocabulary.rs`
 * while row 14 had storage and no surface; **Slice 14d moved all five onto `GLOSS_TABLES` in the
 * same change that made `views/Memory.tsx` import them**, which is the rule that pair of lists
 * exists to enforce — a table left on the deferred list once a view imports it would make the
 * bundle-staleness check stop covering a gloss that is now on screen.
 *
 * Declaring the gloss beside the vocabulary rather than waiting for the view is the point: a
 * vocabulary that reaches a surface before its gloss does renders "this build has no description"
 * to a real reader, and Slice 5d-iii found 120 sites doing exactly that.
 */

/**
 * The four states a note is **stored** in.
 *
 * Four rather than six. `potentially_stale`, `conflicted` and `multiple_active` are worked out when
 * a note is read and are never written down, because keeping a stored copy true would need
 * something to keep writing it, and that something would be a question rather than an answer.
 *
 * Superseded and invalidated are the pair that must not be read as one: something replaced it, or
 * nothing did.
 */
const MEMORY_STATUS: Record<string, string> = {
  proposed:
    'Written down and not yet confirmed by a person at the command line. It is on record, and nothing here treats it as settled.',
  active:
    'Confirmed, and the current note about its subject. Whether it is still true of the code today is worked out when you read it, never stored as a score.',
  superseded:
    'A later note replaced this one, and that note says so. The words here are kept exactly as they were, so what was once believed can still be read.',
  invalidated:
    'It stopped being true and nothing replaced it — which is a different thing from being superseded, and usually the one you need, because there is no successor to read instead.',
};

export function memoryStatusGloss(value: string): string {
  return MEMORY_STATUS[value] ?? 'This build has no description for that memory status.';
}

/**
 * What is true of a note *right now*, worked out at the moment it is read.
 *
 * None of these is stored. The distinction between the last two is the one that matters: several
 * notes about one file is normal, and calling that a contradiction would be inventing a
 * disagreement out of two unrelated sentences. A contradiction needs both notes to be answering the
 * same named question.
 */
const MEMORY_VIEW: Record<string, string> = {
  potentially_stale:
    'The repository has moved on since this note was written, so nobody has checked it against the code as it is now. That is a reason to re-read it, not a sign that it is wrong.',
  conflicted:
    'Two confirmed notes answer the same named question about this subject and say different things. Nerve shows the disagreement and does not settle it; picking a winner would be inventing an answer neither note gave.',
  multiple_active:
    'Several notes are about this subject. That is ordinary — they answer no shared question, so nothing here says they disagree.',
};

export function memoryViewGloss(value: string): string {
  return MEMORY_VIEW[value] ?? 'This build has no description for that memory view.';
}

/**
 * What became of the thing a note is about.
 *
 * A note keeps a copy of its subject rather than a pointer to it, because indexing deletes entities
 * when files go away — so the note outlives what it was written about, and can still say what that
 * was. These five are how it says which of those situations you are in.
 *
 * "Could not check" is deliberately not "gone": reporting a subject as deleted when nothing was
 * ever looked at would be claiming something nobody observed.
 */
const MEMORY_SUBJECT_RESOLUTION: Record<string, string> = {
  resolved: 'The thing this note is about is in the index, so the note is attached to it directly.',
  resolved_through_identity_link:
    'The subject moved, and a recorded rename says where to. The note follows it because that record exists — never because two names looked similar.',
  missing:
    'The subject is no longer in the index and no recorded move leads anywhere. The note is kept and still readable: it carries a copy of what it was written about.',
  ambiguous:
    'More than one place the subject may have moved to, and none of them is preferred. Every candidate is shown, because choosing one would be deciding something nothing recorded.',
  repository_state_unavailable:
    'Nothing has been indexed here, so whether the subject is still there is unknown rather than answered. The note is readable either way.',
};

export function memorySubjectResolutionGloss(value: string): string {
  return (
    MEMORY_SUBJECT_RESOLUTION[value] ??
    'This build has no description for that subject resolution.'
  );
}

/**
 * Which facet of its subject a note's claim is about.
 *
 * Four, and the axis is neither *what kind of thing the subject is* — the subject snapshot holds
 * that — nor *what question the note answers*, which is the claim key. It is which side of the
 * subject the claim is about, and it earns its place by keeping two notes apart: "the payments team
 * owns this code" and "platform owns its deployment" answer the same named question about the same
 * file and are not a disagreement.
 *
 * A misfiled note is not a lost note. It is still found by its subject and by searching its text;
 * it only stops competing with a note filed elsewhere.
 */
const MEMORY_SCOPE: Record<string, string> = {
  implementation:
    'The claim is about what the code does and why it is written this way — the decision inside the thing, rather than the promise it makes outward.',
  interface:
    'The claim is about what the thing promises to whoever calls or consumes it. A fact about the boundary, not about what happens behind it.',
  operations:
    'The claim is about how it behaves when it runs: configuration, deployment, environment. A policy that is written in code belongs under implementation and the same policy configured belongs here, and a person may reasonably file it either way.',
  process:
    'The claim is about how people work on it: ownership, review, convention. Ownership is a facet here and never a scope of its own, because owner is a question a note answers rather than a side of the subject.',
};

export function memoryScopeGloss(value: string): string {
  return MEMORY_SCOPE[value] ?? 'This build has no description for that memory scope.';
}

/**
 * What one entry of a note's audit history records having been done.
 *
 * Five, and four of them are status changes. `cited` is the one that is not: attaching a citation
 * changes no status, so its entry arrives with the same status on both sides — which is
 * well-formed for this one operation and a defect for every other. That is the whole reason this is
 * a separate vocabulary from the stored statuses, and it is why an event carries a flag saying
 * whether it changed anything rather than leaving a reader to compare the two.
 *
 * There is no entry for a deletion, and there must never be one: nothing removes a record or an
 * entry, and a verb for it is how *history is preserved* would stop being true.
 */
const MEMORY_OPERATION: Record<string, string> = {
  proposed:
    'The note was written down. The entry that creates the record, so there is no status before it.',
  confirmed:
    'A person at the command line confirmed it. This is the act the whole feature turns on, and what makes it a human act is the surface it arrived on — there is no identity here to check.',
  superseded:
    'A later note replaced this one. The words are kept exactly as they were, so what was once believed can still be read.',
  invalidated:
    'It stopped being true and nothing replaced it. Every entry the record ever had is still here, including this one.',
  cited:
    'A passage was attached as a citation. The only entry that changes no status, so it arrives with the same status on both sides.',
};

export function memoryOperationGloss(value: string): string {
  return MEMORY_OPERATION[value] ?? 'This build has no description for that memory operation.';
}

/**
 * A reading for a key/value pair inside an extractor's `details` blob, where one exists.
 *
 * `details` is an open JSON object — most of it is paths, names and line numbers that need no
 * gloss. A few keys carry a closed vocabulary, and those are exactly the ones that would
 * otherwise be shown to a reader as a bare identifier such as
 * `document_supersedes_target_ambiguous`. Anything else returns `undefined` and is left alone;
 * inventing a sentence for an arbitrary key would be worse than saying nothing.
 */
export function detailGloss(key: string, value: string): string | undefined {
  // An absent value is not a vocabulary member. `reason` is `null` on every reference that
  // resolved, and glossing that with "this build has no description for that reason code" would
  // report a missing gloss where there is nothing to gloss — the loudest possible way to say
  // that nothing went wrong.
  if (value === 'null' || value === '') return undefined;

  switch (key) {
    case 'reason':
      return unresolvedReason(value);
    case 'status':
      return adrStatusGloss(value);
    case 'category':
      return UNRESOLVED_CATEGORY[value];
    case 'child_kind':
    case 'container_kind':
    case 'declaration_kind':
    case 'source_kind':
      return KIND_GLOSS[value];
    case 'call_form':
      return UNMODELLED_FORM[value];
    default:
      return undefined;
  }
}
