/**
 * The pure half of the impact view: the fragment, and every sentence this screen is able to get
 * wrong.
 *
 * One rule sits above the others and is the reason this file exists rather than a component with
 * some JSX in it. **A short answer here is the dangerous answer.** `results` is a reverse
 * dependency closure, and three rows read as *"three things depend on this, it is safe to change"* —
 * which is a claim about the whole repository built from a traversal that could only follow the
 * edges Nerve managed to resolve. Nerve has no type inference, so `shape.area()` is recorded as
 * unresolved rather than guessed at, and Slice 2a measured 38.1 % of call sites on the resolution
 * corpus as unresolved. On `fixtures/ts-basic`, `add` has three dependants beside **four**
 * unresolved sites: the caveat is larger than the answer, and that is the honest shape of the
 * repository rather than a defect in the response.
 *
 * So [`unresolvedReading`] has no branch that returns nothing. The zero case is the one where a
 * silent omission most invites the wrong conclusion, and it gets a sentence saying so explicitly.
 *
 * Two further rules, both about not overclaiming:
 *
 * - **The relation set is read off the answer, never assumed.** The server echoes `relations`, and
 *   an empty request means five specific relations rather than "every relation" — following
 *   `CONTAINS` would answer that every symbol impacts the repository. A list mirrored in this file
 *   would be a second copy of a decision that lives in `nerve-store`, free to drift.
 * - **This is not "affected tests".** `nerve affected` is refused rather than deferred: LCOV
 *   carries no per-test attribution (ADR-0008 §A.2). A test file in an impact set is there because
 *   code depends on code, and nothing on this screen may be labelled as test impact.
 */

import type { ImpactReport, ImpactRow, ImpactUnresolved } from './api/types';
import type { Tone } from './format';

/** The server's ceilings, mirrored only to keep the controls inside them. It clamps regardless. */
export const MAX_DEPTH = 32;
export const MAX_LIMIT = 500;

/** What the server applies when the request says nothing. */
export const DEFAULT_DEPTH = 6;
export const DEFAULT_LIMIT = 50;

/**
 * Build an impact fragment.
 *
 * Every value goes through `URLSearchParams` for the reason `memoryFragment` does: a subject is a
 * selector a person typed, it may legally hold a `#`, and a raw `#` in a URL starts the fragment —
 * so `?subject=src/app.ts#Thing` would arrive as `subject=src/app.ts` and be answered, correctly,
 * about a different entity. The `#` is the symbol separator in the selector grammar, so this is
 * the common case rather than a hostile one.
 */
export function impactFragment(options: Record<string, string>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(options)) {
    if (value !== '') search.set(key, value);
  }
  const text = search.toString();
  return text.length > 0 ? `#/impact?${text}` : '#/impact';
}

/**
 * Clamp a typed number the way the server does, and refuse to send nonsense.
 *
 * Returns `null` for anything that is not a positive integer, so the caller omits the parameter
 * and gets the documented default rather than sending `NaN` and receiving a `400` for a control
 * this app rendered.
 */
export function bounded(text: string, ceiling: number): number | null {
  if (!/^[0-9]+$/.test(text.trim())) return null;
  const value = Number(text.trim());
  if (!Number.isFinite(value) || value < 1) return null;
  return Math.min(value, ceiling);
}

/**
 * **The account of what this answer cannot see.** Both branches are sentences; neither is silence.
 *
 * The scope is repository-wide, restricted to the relations walked — a hidden edge could attach
 * anywhere, and narrowing that without name matching is not possible. Which is also why nothing
 * here lists the unresolved sites as suspect callers or matches their names against the subject's:
 * that is identity by coincidence, Nerve does not do it, and the API deliberately hands over no
 * data to do it with.
 */
export function unresolvedReading(unresolved: ImpactUnresolved): string {
  if (unresolved.sites > 0) {
    const sites = unresolved.sites === 1 ? '1 reference site' : `${unresolved.sites} reference sites`;
    return `${sites} in this repository resolved to nothing. Any of them could reach this symbol and this answer cannot rule them out.`;
  }
  return 'Every reference site Nerve indexed under these relations resolved, so no failed resolution is hiding a dependency from this answer.';
}

/**
 * Whether the account is a warning or a clearance, for hue only.
 *
 * `unknown` rather than `stale` when sites exist: nothing here has gone out of date, it was never
 * worked out in the first place. And `fresh` for the zero case is a real statement rather than a
 * default — it is the one case where the reassurance is earned.
 */
export function unresolvedTone(unresolved: ImpactUnresolved): Tone {
  return unresolved.sites > 0 ? 'unknown' : 'fresh';
}

/**
 * The three coarser counts, which are the same fact and must not read as three findings.
 *
 * `sites >= assertions >= targets` always, because they are one relationship counted at three
 * grains. A screen that put them side by side unlabelled would invite a reader to add them up.
 */
export function grainReading(unresolved: ImpactUnresolved): string {
  if (unresolved.sites === 0) return '';
  return `The same failure counted three ways: ${unresolved.sites} observed site(s), grouped into ${unresolved.assertions} assertion(s), naming ${unresolved.targets} distinct unnamed target(s). Not three findings.`;
}

/**
 * How much of the closure is on the screen.
 *
 * Read off `count` against `results_total` rather than off the `truncated` flag alone, and the
 * tallies are stated as exact because they are: `limit` caps rows and nothing else. A reader who
 * believed the totals had been capped too would treat the one trustworthy number on the screen as
 * a lower bound.
 */
export function pageReading(report: Pick<ImpactReport, 'count' | 'results_total' | 'truncated'>): string {
  if (report.results_total === 0) return '';
  if (!report.truncated && report.count >= report.results_total) {
    return `Every one of the ${report.results_total} is listed below.`;
  }
  return `${report.count} of ${report.results_total} are listed below. The tallies above are exact — the cap applies to rows only.`;
}

/**
 * The empty answer, which still is not silence.
 *
 * Names the relations that were walked, because "nothing depends on this" is only true relative to
 * them, and the unresolved account is rendered beside this rather than instead of it — an empty
 * closure with unresolved sites outstanding is exactly the case where the caveat carries the whole
 * answer.
 */
export function emptyReading(relations: string[]): string {
  const set = relations.length > 0 ? relations.join(', ') : 'the relations that were walked';
  return `Nothing in the index depends on this through ${set}.`;
}

/**
 * What the closure did **not** follow, stated once on the screen.
 *
 * `TEST_OBSERVED_CALL` is named because its absence is a decision rather than an oversight: a
 * trace says one run took an edge, so a blast radius built on it would grow and shrink with which
 * tests happened to have been run, and an artifact is untrusted input that must not be able to
 * change what Nerve tells a user to review. `CONTAINS` is named because following it would answer
 * that every symbol impacts the repository.
 */
export function exclusionReading(relations: string[]): string {
  const walked = new Set(relations);
  const notable = ['TEST_OBSERVED_CALL', 'CONTAINS', 'IMPORTS'].filter((name) => !walked.has(name));
  if (notable.length === 0) return '';
  return `Not followed: ${notable.join(', ')}. A trace edge says one run took it rather than that every run does, and containment would answer that every symbol impacts the whole repository.`;
}

/** Whether a relation the user could add is one the closure is deliberately built without. */
export function isOptInRelation(relation: string): boolean {
  return relation === 'TEST_OBSERVED_CALL';
}

/**
 * How stale evidence in the closure should be read.
 *
 * A stale row is not a wrong row: the edge was recorded against a file that has since changed, so
 * what is unknown is whether it survived. Saying "re-index" is the actionable half.
 */
export function staleReading(stale: number): string {
  if (stale === 0) return '';
  const rows = stale === 1 ? '1 of these was' : `${stale} of these were`;
  return `${rows} reached through evidence recorded against a file that has since changed. Re-index to find out whether the dependency survived.`;
}

/** Hue for one row's freshness. `null` is not measured, which is not the same as stale. */
export function rowTone(row: Pick<ImpactRow, 'evidence_freshness' | 'is_unresolved'>): Tone {
  if (row.is_unresolved) return 'unknown';
  switch (row.evidence_freshness) {
    case 'fresh':
      return 'fresh';
    case 'stale':
      return 'stale';
    case 'file-missing':
      return 'absent';
    case null:
    case undefined:
      return 'quiet';
  }
  return 'unknown';
}

/**
 * The depth tally, in depth order, from the array the server sends.
 *
 * It arrives as an array of `{depth, entities}` rather than an object precisely so this order
 * survives the wire — JSON object keys are strings, and `"10"` sorts before `"2"`. Sorting again
 * here is cheap and makes the guarantee local rather than remembered.
 */
export function depthRows(totals: ImpactReport['totals']): [string, number][] {
  return [...totals.by_depth]
    .sort((left, right) => left.depth - right.depth)
    .map(({ depth, entities }): [string, number] => [`depth ${depth}`, entities]);
}

/** A `Record<string, number>` tally as sorted rows: biggest first, then by name for stability. */
export function tallyRows(tally: Record<string, number>): [string, number][] {
  return Object.entries(tally).sort(
    (left, right) => right[1] - left[1] || left[0].localeCompare(right[0]),
  );
}
