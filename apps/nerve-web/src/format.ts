/**
 * Formatting, and the plain-English glosses for Nerve's vocabulary.
 *
 * The evidence model deliberately has no `confidence: float` — it has source types, directness,
 * extractor identity and freshness. That is more truthful and less legible, so every one of those
 * terms is glossed here in one sentence, in the interface's own voice, and shown next to the term
 * rather than hidden in documentation. A reader who has never opened an ADR should still be able
 * to say what `AST_RESOLVED` means and why it is weaker than `AST_DIRECT`.
 */

export type Tone = 'fresh' | 'stale' | 'absent' | 'unknown' | 'quiet' | 'plain';

const FRESHNESS: Record<string, { label: string; tone: Tone; gloss: string }> = {
  fresh: {
    label: 'fresh',
    tone: 'fresh',
    gloss: 'The file still hashes to exactly what this observation was extracted from.',
  },
  stale: {
    label: 'stale',
    tone: 'stale',
    gloss: 'The file has changed since this was recorded. Re-index before trusting it.',
  },
  'file-missing': {
    label: 'file missing',
    tone: 'absent',
    gloss: 'The file this was read from no longer exists in the repository.',
  },
  refused: {
    label: 'refused',
    tone: 'absent',
    gloss: 'The path guard refused to re-read the file, so freshness could not be measured.',
  },
  unreadable: {
    label: 'unreadable',
    tone: 'absent',
    gloss: 'The file could not be read back as text, so freshness could not be measured.',
  },
};

export function freshness(value: string): { label: string; tone: Tone; gloss: string } {
  return (
    FRESHNESS[value] ?? {
      label: value,
      tone: 'quiet',
      gloss: 'This build does not have a description for that freshness value.',
    }
  );
}

const SOURCE_TYPES: Record<string, string> = {
  AST_DIRECT: 'The syntax tree literally contains this relationship.',
  AST_RESOLVED: 'Reached through import and module resolution, not stated in one place.',
  AST_HEURISTIC: 'Matched by name. Ambiguous by construction.',
  TYPE_RESOLVED: 'A type checker resolved it.',
  FRAMEWORK_RULE: 'A deterministic framework rule concluded it.',
  TEST_COVERAGE: 'A test executed this symbol. That is coverage, not a call.',
  TEST_CALL_TRACE: 'A call observed during a test run, through instrumentation.',
  RUNTIME_CALL_TRACE: 'A call observed at runtime.',
  DOCUMENT_STATED: 'A document asserts it.',
  HUMAN_CONFIRMED: 'A person confirmed it.',
  LLM_DERIVED: 'A language model suggested it.',
  FILESYSTEM_OBSERVED:
    'The filesystem contains this. Found by walking directories, never by reading a file.',
};

export function sourceTypeGloss(value: string): string {
  return SOURCE_TYPES[value] ?? 'This build has no description for that evidence source type.';
}

const DIRECTNESS: Record<string, string> = {
  DIRECT: 'The artifact states it outright.',
  RESOLVED: 'Derived through a resolution step.',
  INFERRED: 'A rule concluded it.',
};

export function directnessGloss(value: string): string {
  return DIRECTNESS[value] ?? 'This build has no description for that directness value.';
}

/**
 * Class suffix for the spine node, which encodes directness by fill rather than by colour.
 *
 * Every member of the vocabulary is named, and there is deliberately **no `default` arm**. This
 * function used to end in `default: return 'obs--inferred'`, which meant a directness the build
 * did not recognise was drawn as though a rule had concluded it — a statement about the strength
 * of the evidence that nothing had observed. An unknown value is now drawn as unknown, which is
 * the only honest thing to say about it.
 */
export function directnessClass(value: string): string {
  switch (value) {
    case 'DIRECT':
      return 'obs--direct';
    case 'RESOLVED':
      return 'obs--resolved';
    case 'INFERRED':
      return 'obs--inferred';
  }
  return 'obs--unrecognised';
}

const RELATION_VERB: Record<string, [string, string]> = {
  CONTAINS: ['contains', 'is contained by'],
  DEFINES: ['defines', 'is defined by'],
  IMPORTS: ['imports', 'is imported by'],
  EXPORTS: ['exports', 'is exported by'],
  CALLS: ['calls', 'is called by'],
  REFERENCES: ['references', 'is referenced by'],
  EXTENDS: ['extends', 'is extended by'],
  IMPLEMENTS: ['implements', 'is implemented by'],
  // Stored one way only — `A SUPERSEDES B` means A replaces B — so the incoming reading is a
  // query over the same edge rather than a second edge pointing the other way.
  SUPERSEDES: ['supersedes', 'is superseded by'],
};

/** How to read a relation from the subject's side. */
export function relationPhrase(relation: string, outgoing: boolean): string {
  const pair = RELATION_VERB[relation];
  if (!pair) return outgoing ? relation.toLowerCase() : `${relation.toLowerCase()} (incoming)`;
  return outgoing ? pair[0] : pair[1];
}

const STATUS_GLOSS: Record<string, string> = {
  SUPPORTED: 'At least one observation supports this and none contradicts it.',
  CONTRADICTED: 'Observations disagree about this relationship.',
  STALE: 'Last observed in an older state of the repository. Re-index to find out whether it survived.',
  UNRESOLVED: 'Supported, but one end is a reference Nerve could not connect to a declaration.',
  DELETED: 'The evidence that supported this is gone from the current state. Kept, not erased.',
};

export function statusGloss(value: string): string {
  return STATUS_GLOSS[value] ?? 'This build has no description for that assertion status.';
}

export function bytes(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return 'unknown';
  if (value < 1024) return `${value} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let size = value / 1024;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[unit]}`;
}

export function count(value: number): string {
  return value.toLocaleString('en-US');
}

/** A timestamp as the repository recorded it, trimmed to something a person can read. */
export function stamp(value: string | null): string {
  if (!value) return 'not recorded';
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return value;
  const date = new Date(parsed);
  const pad = (part: number) => String(part).padStart(2, '0');
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  );
}

export function ago(value: string | null, now = Date.now()): string {
  if (!value) return '';
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return '';
  const seconds = Math.max(0, Math.round((now - parsed) / 1000));
  if (seconds < 45) return 'moments ago';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? '' : 's'} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 36) return `${hours} hour${hours === 1 ? '' : 's'} ago`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? '' : 's'} ago`;
}

/** Shorten a hash for display. The full value always stays available as a `title`. */
export function shortHash(value: string | null, keep = 10): string {
  if (!value) return 'none';
  return value.length <= keep ? value : `${value.slice(0, keep)}…`;
}

/** Middle-elision, so both ends of a long qualified name survive. */
export function elide(value: string, max: number): string {
  if (value.length <= max) return value;
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${value.slice(0, head)}…${value.slice(value.length - tail)}`;
}

export function fileLine(path: string | null, line: number | null): string {
  if (!path) return 'no recorded location';
  return line === null ? path : `${path}:${line}`;
}

/** Render an arbitrary JSON blob as text. Never as markup. */
export function jsonText(value: unknown): string {
  if (value === null || value === undefined) return 'none';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/** Flatten a shallow object into label/value pairs for display; anything deeper stays JSON. */
export function pairs(value: unknown): [string, string][] | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0) return null;
  return entries.map(([key, item]) => [
    key,
    item === null || item === undefined
      ? 'null'
      : typeof item === 'object'
        ? jsonText(item)
        : String(item),
  ]);
}
