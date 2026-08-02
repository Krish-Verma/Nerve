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
};

export function kindGloss(kind: string): string {
  return KIND_GLOSS[kind] ?? 'This build has no description for that entity kind.';
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
