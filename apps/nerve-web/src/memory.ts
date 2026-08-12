/**
 * The pure part of the memory view: routing, the two kinds, and every decision about wording that
 * would otherwise be made inline in a component where nothing can test it.
 *
 * One distinction is load-bearing above all the others and it is the reason this file exists.
 * **A note's `status` is stored and its `views` are computed when it is read.** Four values live
 * in a column; `potentially_stale`, `conflicted` and `multiple_active` live nowhere, are worked out
 * from the anchor state and from what else is active on the same subject, and are true only of the
 * moment the answer was assembled. An interface that drew them as one row of identical chips would
 * undo the split the whole row was designed around, so the two kinds are separated here — as data,
 * in [`STORED_ON_A_RECORD`] and [`DERIVED_ON_A_RECORD`] — and `memory.test.mjs` holds the view to
 * rendering them apart.
 *
 * Nothing here derives a verdict. `potentially_stale` is the server's answer and arrives in
 * `views`; [`anchorReading`] states the two state ids beside each other and is forbidden from
 * concluding anything from them, in the manner `history.ts` reads `may_claim_created` rather than
 * recomputing it.
 */

import type { MemoryRecord } from './api/types';
import type { Tone } from './format';

/**
 * Build a memory fragment.
 *
 * Every value goes through `URLSearchParams`, and here that is not only tidiness. A `memory_id` is
 * caller-supplied text that may legally hold a `#`, a raw `#` in a URL starts the fragment, and the
 * browser would drop everything after it — so `?record=note#2` would arrive as `record=note` and be
 * answered about a different record. The same reasoning as `historyFragment`, one screen over.
 */
export function memoryFragment(options: Record<string, string>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(options)) {
    if (value !== '') search.set(key, value);
  }
  const text = search.toString();
  return text.length > 0 ? `#/memory?${text}` : '#/memory';
}

/**
 * Hue for a note's **stored** lifecycle.
 *
 * There is deliberately no `default` arm returning a settled colour. `superseded` and `invalidated`
 * are the pair a reader is choosing between — *something replaced it* against *nothing did* — so
 * they are never the same hue, and a value this build does not recognise is drawn as unrecognised
 * rather than as one of the four.
 */
export function statusTone(status: string): Tone {
  switch (status) {
    case 'active':
      return 'fresh';
    // On record and not settled. Muted rather than coloured, because a proposal is neither a
    // current claim nor a retired one.
    case 'proposed':
      return 'quiet';
    case 'superseded':
      return 'stale';
    case 'invalidated':
      return 'absent';
  }
  return 'unknown';
}

/**
 * Hue for a **derived** view.
 *
 * `conflicted` and `multiple_active` must never look alike: one says two notes answer the same
 * named question differently, and the other says several notes are about one subject, which is
 * ordinary. Drawing the second as a warning is the false claim the corrected design exists to
 * prevent.
 */
export function viewTone(view: string): Tone {
  switch (view) {
    case 'potentially_stale':
      return 'stale';
    case 'conflicted':
      return 'unknown';
    case 'multiple_active':
      return 'quiet';
  }
  return 'unknown';
}

/**
 * Hue for what a note's subject snapshot reaches now.
 *
 * `missing` and `repository_state_unavailable` are the pair that must not collapse: one is a
 * deletion that was observed and the other is a question nobody could ask, and rendering the second
 * as the first would claim a deletion nothing saw. They are `absent` and `unknown`, which is the
 * same separation `nerve check` draws between stale and unverified.
 */
export function subjectTone(resolution: string): Tone {
  switch (resolution) {
    case 'resolved':
      return 'fresh';
    // Resolved, and through a recorded rename rather than directly. Muted rather than confident,
    // because the attachment rests on a second record that the reader may want to look at.
    case 'resolved_through_identity_link':
      return 'quiet';
    case 'missing':
      return 'absent';
    case 'ambiguous':
      return 'unknown';
    case 'repository_state_unavailable':
      return 'unknown';
  }
  return 'unknown';
}

/**
 * The stored columns of a record, in the order a card shows them.
 *
 * A list rather than a convention, so `memory.test.mjs` can assert that no key is on both lists and
 * that the view labels each group. Every one of these was written by a command; nothing recomputes
 * one on a read.
 */
export const STORED_ON_A_RECORD: readonly string[] = [
  'memory_id',
  'status',
  'subject',
  'scope',
  'claim_key',
  'anchor_state_id',
  'content',
  'author_label',
  'created_at',
  'supersedes_memory_id',
  'invalidated_at',
  'invalidation_reason',
];

/**
 * The values worked out at the moment the record was read. **Nothing writes one.**
 *
 * `superseded_by_memory_id` is on this list and that is the point of it: supersession is stored on
 * the successor only, and a second column holding the inverse could disagree with nothing in the
 * schema to notice.
 */
export const DERIVED_ON_A_RECORD: readonly string[] = [
  'views',
  'subject_resolution',
  'subject_live_entity_ids',
  'current_state_id',
  'superseded_by_memory_id',
];

/**
 * How to head each of the three answers, and the two absences that are not the same absence.
 *
 * `no_memory_recorded` means nobody has ever written a note here; `no_memory_matches` means notes
 * exist and this question matches none of them. They have different next steps — write one, or
 * widen the question — and a screen that headed both "no results" would answer the first as the
 * second. The sentence under each heading is the server's `absence_statement`, printed rather than
 * paraphrased.
 */
export function absenceHeading(resultKind: string): string {
  switch (resultKind) {
    case 'no_memory_recorded':
      return 'Nothing has been written down here';
    case 'no_memory_matches':
      return 'These filters match none of the notes here';
  }
  return 'This answer holds no record';
}

/**
 * The two state ids, side by side, with **no verdict drawn from comparing them**.
 *
 * Whether a note is `potentially_stale` is the server's answer and arrives in `views`. Deciding it
 * here from `anchor !== current` would be a second copy of the one rule the derived half exists to
 * own, and the copy would be the one on screen.
 */
export function anchorReading(anchor: string, current: string | null): string {
  if (current === null) {
    return 'confirmed against a recorded state; nothing is indexed now, so there is no state to read it beside';
  }
  return anchor === current
    ? 'confirmed against the state this index still describes'
    : 'confirmed against a state this index has since moved on from';
}

/**
 * How much of the answer this page holds.
 *
 * Read off `returned` against `total` rather than off the `truncated` flag, and the difference is
 * a wrong sentence rather than a style choice: `truncated` means *this page was cut*, which is
 * false on the **last** page of a cut answer. A screen that read the flag would tell a reader
 * looking at records 51–55 of 55 that every one of them was in front of them.
 */
export function pageReading(truncation: { returned: number; total: number } | null): string {
  if (truncation === null) return '';
  // Nothing matched, so there is no page to describe. "Every one of them is on this page" is a
  // claim about a set with no members, and the absence panel below is where that answer belongs.
  if (truncation.total === 0) return '';
  return truncation.returned === truncation.total
    ? 'Every one of them is on this page.'
    : `This page holds ${truncation.returned} of them.`;
}

/**
 * Whether a pager belongs on this screen.
 *
 * True on **every** page of a cut answer, not only on the ones that were themselves cut. Gating it
 * on `truncated` alone leaves the last page with no way back, which is the version this screen
 * shipped to a viewport-controlled QA run before anything caught it.
 */
export function hasPager(truncation: { truncated: boolean } | null, offset: number): boolean {
  if (truncation === null) return false;
  return truncation.truncated || offset > 0;
}

/**
 * Both directions of a supersession, and which of the two the schema stores.
 *
 * The inverse is reported by the server and never written back. A client that stored it would be
 * keeping a second, independently writable copy of one fact.
 */
export function supersession(record: Pick<MemoryRecord, 'supersedes_memory_id' | 'superseded_by_memory_id'>): {
  replaces: string | null;
  replacedBy: string | null;
  isEndOfChain: boolean;
} {
  return {
    replaces: record.supersedes_memory_id,
    replacedBy: record.superseded_by_memory_id,
    isEndOfChain: record.superseded_by_memory_id === null,
  };
}
