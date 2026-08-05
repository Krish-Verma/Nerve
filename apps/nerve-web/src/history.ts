/**
 * The pure part of the history views: routing, the two clocks, and every decision about wording
 * that would otherwise be made inline in a component where nothing can test it.
 *
 * Everything a history answer is capable of getting wrong is a wording mistake. "The earliest
 * change Nerve can see" and "the first commit" are the same row of the same table read two ways,
 * and only one of them is true; a refusal to compute a range and a range that came back empty are
 * the same JSON key holding `null` and `[]`. So the rules live here, as functions over plain
 * values, and `history.test.mjs` holds them to it.
 *
 * Nothing in this file derives a permission. `may_claim_created` and
 * `may_claim_history_begins_here` are computed in `nerve-core` and carried on the response; the
 * functions below *read* them. A `kind === 'created_in_visible_history'` written here would be a
 * second copy of the one rule the whole historical model exists to protect.
 */

import type { HistoryChange, HistoryCommit, HistoryDiffReport } from './api/types';

/** The five questions the history endpoints answer, as screens. */
export type HistoryTab = 'commits' | 'path' | 'diff' | 'frequency' | 'cochange';

export const HISTORY_TABS: readonly HistoryTab[] = [
  'commits',
  'path',
  'diff',
  'frequency',
  'cochange',
];

export function isHistoryTab(value: string | undefined): value is HistoryTab {
  return value !== undefined && (HISTORY_TABS as readonly string[]).includes(value);
}

/**
 * Build a history fragment.
 *
 * Every parameter goes through `URLSearchParams`, which is not a tidiness preference: a path may
 * legally contain a `#`, a raw `#` in a URL is a fragment, and the browser drops everything after
 * it before the request is sent. `?path=README.md#parse` would arrive at the server as
 * `path=README.md` and be answered — correctly, and about a different thing than was asked. The
 * symbol-selector refusal only fires if the `#` survives as `%23`.
 */
export function historyFragment(tab: HistoryTab, options: Record<string, string>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(options)) {
    if (value !== '') search.set(key, value);
  }
  const text = search.toString();
  return text.length > 0 ? `#/history/${tab}?${text}` : `#/history/${tab}`;
}

/**
 * The phrase for each first-observed answer — with the creation phrase deliberately absent.
 *
 * Entry 7 of the UI/backend handoff fixes both what each of the six values may be rendered as and
 * what it must never be rendered as. This is that column minus one row: `created_in_visible_history`
 * has no entry here, so no lookup by value can ever produce a creation claim.
 */
const WITHOUT_A_CREATION_CLAIM: Record<string, string> = {
  earliest_visible_change: 'the earliest change Nerve can see',
  present_before_visible_history: 'in the tree now, untouched in visible history',
  absent_from_visible_history: 'not in the tree, and no recorded commit touched it',
  current_tree_unknown: 'no index, so the current tree could not be consulted',
  no_history_ingested: 'history has never been read here',
};

/** The one phrase that claims a creation, reachable only through the permission. */
const A_CREATION_CLAIM = 'created at this change';

/**
 * How to head a first-observed answer.
 *
 * The permission is the gate and the value is only ever a lookup for the *other* five phrasings.
 * A response that somehow carried `created_in_visible_history` without the permission gets no
 * phrasing at all rather than a borrowed one, because every remaining phrase would be a claim
 * about a different answer.
 */
export function firstObservedHeadline(kind: string, mayClaimCreated: boolean): string {
  if (mayClaimCreated) return A_CREATION_CLAIM;
  return WITHOUT_A_CREATION_CLAIM[kind] ?? 'an answer this build has no phrasing for';
}

/** The three parent situations that mean "cannot see past this", none of which is a beginning. */
const BOUNDARY_COMPLETENESS = new Set([
  'shallow_boundary',
  'parents_missing',
  'parents_unverifiable',
]);

/**
 * Whether a commit is a boundary, and whether it is the *beginning*.
 *
 * `may_claim_history_begins_here` comes off the response and is true for a root commit and
 * nothing else. Everything below it is the earliest thing visible from here, which is a statement
 * about this checkout rather than about the project — so it never reads as a first commit.
 */
export function commitBoundary(commit: {
  may_claim_history_begins_here: boolean;
  parent_completeness: string;
  parent_oids: string[];
}): { label: string; begins: boolean } | null {
  if (commit.may_claim_history_begins_here) {
    return { label: 'history begins here', begins: true };
  }
  if (commit.parent_oids.length === 0 || BOUNDARY_COMPLETENESS.has(commit.parent_completeness)) {
    return { label: 'earliest visible in this checkout', begins: false };
  }
  return null;
}

/**
 * How many changes an answer counted — keeping `null` apart from `0`.
 *
 * `null` is what a path's commit list carries, where the rows counted belong to one path rather
 * than to the commit. Rendering it as `0` would say the commit changed nothing, which is a claim
 * the answer never made.
 */
export function changeCountReading(changes: number | null): { text: string; counted: boolean } {
  if (changes === null) return { text: 'changes not counted in this answer', counted: false };
  return { text: `${changes} ${changes === 1 ? 'change' : 'changes'}`, counted: true };
}

/** A range that was computed, or a refusal to compute one. Never both, and never neither. */
export type DiffReading =
  | {
      outcome: 'range';
      commits: HistoryCommit[];
      changes: HistoryChange[];
      commitsInRange: number;
      commitsTruncated: boolean;
      changesTruncated: boolean;
      mergesInRange: number;
    }
  | { outcome: 'refused'; kind: string };

/**
 * Which of the five outcomes a diff answer is.
 *
 * Four of them compute no range at all, and on those **every diff-shaped key is `null`** rather
 * than empty. So `commits === null` is a refusal and `commits === []` is a range that holds
 * nothing, and a reader that treated absent and empty alike would turn "we could not work out
 * what lies between these two states" into "nothing changed between them".
 */
export function diffReading(report: HistoryDiffReport): DiffReading {
  if (report.commits === null) return { outcome: 'refused', kind: report.result_kind };
  return {
    outcome: 'range',
    commits: report.commits,
    changes: report.changes ?? [],
    commitsInRange: report.commits_in_range ?? report.commits.length,
    commitsTruncated: report.commits_truncated ?? false,
    changesTruncated: report.changes_truncated ?? false,
    mergesInRange: report.merges_in_range ?? 0,
  };
}

/**
 * The co-change disclaimer, exactly as the API sent it.
 *
 * It is `nerve_store::COCHANGE_IS_NOT_A_DEPENDENCY` verbatim and is on the response rather than
 * in documentation precisely so that no surface has to decide whether to include it. This
 * function will not paraphrase it; if it is ever missing the count is shown with a refusal to
 * interpret it rather than with a sentence of this interface's own invention.
 */
export function cochangeDisclaimer(report: { disclaimer?: string | null }): string {
  const text = report.disclaimer;
  if (typeof text === 'string' && text.trim().length > 0) return text;
  return 'this answer arrived without the disclaimer it is required to carry, so nothing here may be read as a dependency';
}

/**
 * A commit time in the offset the commit itself recorded, not the one this browser is in.
 *
 * Git stores the epoch second and the author's UTC offset separately, and both are kept. Showing
 * the reader's local time instead would silently restate somebody else's afternoon as this
 * machine's morning, and the offset — which the fixtures deliberately vary — would be lost.
 */
export function gitTime(epoch: number, tz: string): string {
  const sign = tz.startsWith('-') ? -1 : 1;
  const hours = Number(tz.slice(1, 3));
  const minutes = Number(tz.slice(3, 5));
  if (!Number.isFinite(epoch) || !Number.isFinite(hours) || !Number.isFinite(minutes)) {
    return `${epoch} seconds, offset ${tz}`;
  }
  const shifted = new Date((epoch + sign * (hours * 3600 + minutes * 60)) * 1000);
  if (Number.isNaN(shifted.getTime())) return `${epoch} seconds, offset ${tz}`;
  const pad = (part: number) => String(part).padStart(2, '0');
  return (
    `${shifted.getUTCFullYear()}-${pad(shifted.getUTCMonth() + 1)}-${pad(shifted.getUTCDate())} ` +
    `${pad(shifted.getUTCHours())}:${pad(shifted.getUTCMinutes())}:${pad(shifted.getUTCSeconds())} ${tz}`
  );
}

/** A file mode as the tree recorded it, in the octal a reader expects. `null` where none was. */
export function fileMode(mode: number | null): string {
  if (mode === null || !Number.isFinite(mode)) return 'none recorded';
  return mode.toString(8).padStart(6, '0');
}
