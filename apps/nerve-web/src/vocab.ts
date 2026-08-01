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
  'computed-member': 'A member reached by a computed key, such as `object[name]()`.',
  'heritage-call': 'A class extending the result of a call — a mixin factory.',
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
  unresolved:
    'A reference Nerve recorded but could not connect to a declaration. It is kept, not discarded.',
};

export function kindGloss(kind: string): string {
  return KIND_GLOSS[kind] ?? 'This build has no description for that entity kind.';
}
