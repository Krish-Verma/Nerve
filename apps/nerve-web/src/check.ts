/**
 * The pure half of the trust screen: the fragment, the hues, and every sentence this screen is
 * able to get wrong.
 *
 * One rule sits above the others and is the reason this file exists rather than a component with
 * some conditionals in it. **There are five verdicts and a two-state screen would be a lie.**
 *
 * `stale` and `unverified` mean the same thing to a caller — do not rely on this index — and rest
 * on opposite evidence. `stale` is a *measurement*: a file changed, a file the index describes is
 * gone, or a file exists that no row describes. `unverified` is the *absence* of a measurement:
 * part of the tree was never looked at, because the sweep reached its cap or a path could not be
 * read. Nothing was observed to have changed. `nerve check` gives them one exit code because a
 * shell has one way to say "do not proceed"; a screen has room to say which, so it does. That is
 * the separation Slice 7c-i settled, and [`verdictTone`] has no shared arm and no `default`.
 *
 * `no_index` and `unusable` are the other two that a careless screen folds into "bad": one means
 * nothing was ever measured, and the other means an index exists that this build cannot read. They
 * have different remedies in the same command and different things to say to a reader.
 *
 * Two further rules, both about not claiming more than was measured:
 *
 * - **The verdict is read off the answer, never derived here.** `trustworthy` is the server's, the
 *   reason is the server's, and both families of counts are the server's. A second derivation on
 *   this side would be a second answer to *can I trust this?* — the exact duplication moving the
 *   judgement into `nerve-index` existed to remove.
 * - **A null tally is not a zero tally.** No sweep ran is not a sweep that found nothing, and
 *   [`sweepReading`] says which it is rather than printing zeroes.
 */

import type { TrustEvidence, TrustNotEstablished, TrustObserved, TrustSweep, TrustTree } from './api/types';
import type { Tone } from './format';

/** Build a check fragment. No options today; the shape matches every other builder. */
export function checkFragment(): string {
  return '#/check';
}

/**
 * Hue for one verdict.
 *
 * **Five arms, five hues, no `default`.** A value this build does not recognise is drawn as
 * unrecognised rather than reused from a member that means something else — the rule
 * `directnessClass` is held to, for the same reason: the strongest thing a screen says about the
 * evidence is exactly what it must never invent.
 *
 * `unverified` is `unknown` and `stale` is `stale`, and that pairing is the whole point. Nothing
 * went out of date in the unverified case; it was never established in the first place, which is
 * the same hue `Impact` gives an unresolved reference site.
 */
export function verdictTone(verdict: string): Tone {
  switch (verdict) {
    case 'current':
      return 'fresh';
    // Measured divergence. The index and the tree disagree about a file that was read.
    case 'stale':
      return 'stale';
    // Nothing was observed to change. Part of the tree was never looked at.
    case 'unverified':
      return 'unknown';
    // An index exists and cannot be read as it stands.
    case 'unusable':
      return 'absent';
    // Nothing was measured at all, which is not a finding about the repository.
    case 'no_index':
      return 'quiet';
  }
  return 'unknown';
}

/**
 * The heading one verdict gets, in the interface's own voice.
 *
 * Five distinct sentences. A heading shared between two verdicts would undo the split before a
 * reader reached the counts that explain it.
 */
export function verdictHeading(verdict: string): string {
  switch (verdict) {
    case 'current':
      return 'This index describes the repository as it is now';
    case 'stale':
      return 'The repository has moved on from this index';
    case 'unverified':
      return 'Part of the repository was never compared';
    case 'unusable':
      return 'This index cannot be used as it stands';
    case 'no_index':
      return 'There is nothing here to judge';
  }
  return 'This build does not recognise that verdict';
}

/**
 * Whether this verdict rests on something that was measured, or on something that was not.
 *
 * The answer a reader needs before they read a count. `stale` counts are observations; `unverified`
 * counts are the size of what nobody looked at, and reading the second as the first is how "I could
 * not check" becomes "it changed".
 */
export function verdictEvidenceKind(verdict: string): 'measured' | 'not-established' | 'none' {
  switch (verdict) {
    case 'current':
    case 'stale':
      return 'measured';
    case 'unverified':
      return 'not-established';
    case 'no_index':
    case 'unusable':
      return 'none';
  }
  return 'none';
}

/**
 * How the sweep should be read, including the case where there was no sweep.
 *
 * The null branch is not an empty string and is not silence: an unjudged index and an index that
 * swept clean are opposite findings, and printing zeroes for the first would report the second.
 */
export function sweepReading(sweep: TrustSweep | null): string {
  if (sweep === null) {
    return 'No sweep ran. Nothing was re-hashed, so there is no tally here — that is the absence of a measurement rather than a measurement of nothing.';
  }
  if (sweep.truncated) {
    return `${sweep.files_probed} of ${sweep.files_total} indexed files were re-hashed before the ${sweep.probe_cap}-file cap stopped the sweep. The rest were never looked at, so nothing here says they are unchanged.`;
  }
  return `All ${sweep.files_total} indexed ${sweep.files_total === 1 ? 'file was' : 'files were'} re-hashed and compared against what was extracted from them.`;
}

/**
 * How the tree walk should be read.
 *
 * Named separately from the sweep on the screen and here, because it is the measurement the sweep
 * cannot make: a file added since the last index has no row to compare, so a repository can grow a
 * hundred modules with every recorded hash still matching. Without this walk that index reports
 * itself current, which is the defect Slice 7c-i was written to fix.
 */
export function treeReading(tree: TrustTree | null): string {
  if (tree === null) {
    return 'The repository was not walked, because there was no index to compare it against.';
  }
  if (tree.added === 0) {
    return 'The repository holds no file the index has never seen. The freshness sweep cannot establish this — it only compares files the index already has a row for — so it is a separate walk of the tree.';
  }
  const files = tree.added === 1 ? '1 file has' : `${tree.added} files have`;
  return `${files} no row in the index at all. The freshness sweep cannot see these: it walks the index's own cache, so a file added since the last index has nothing to compare.`;
}

/** Why an unreadable new file is counted apart, said only when there is one. */
export function unindexableReading(tree: TrustTree | null): string {
  if (tree === null || tree.unindexable === 0) return '';
  const files = tree.unindexable === 1 ? '1 file' : `${tree.unindexable} files`;
  return `${files} in the repository could not be read by the indexer either — over the size ceiling, unreadable, or not UTF-8. They are counted here and not called additions: re-indexing would not produce a row for them, so counting them would make this screen say "stale" forever with nothing you could do about it.`;
}

/** The observed-divergence tally, as rows, or an empty list when nothing was measured. */
export function observedRows(observed: TrustObserved | null): [string, number][] {
  if (observed === null) return [];
  return [
    ['changed', observed.changed],
    ['removed', observed.removed],
    ['added', observed.added],
  ];
}

/**
 * The never-established tally, as rows. Deliberately not addable to the rows above.
 *
 * One-word labels, matching the observed family's, so the two panels read as two tallies of the
 * same grain rather than one detailed list beside one summary — and short enough that the tally
 * rows are legible at 380px, where a long label is truncated to an ellipsis.
 */
export function notEstablishedRows(unchecked: TrustNotEstablished | null): [string, number][] {
  if (unchecked === null) return [];
  return [
    ['refused', unchecked.refused],
    ['unreadable', unchecked.unreadable],
    ['never probed', unchecked.never_probed],
  ];
}

/**
 * Whether either family has anything in it, so a zero panel can still be drawn as a zero panel.
 *
 * Both panels are rendered on every answer regardless. This says whether the numbers in one are
 * worth drawing attention to, not whether the panel exists — an absent panel is indistinguishable
 * from a panel nobody wrote.
 */
export function familyTone(total: number, kind: 'observed' | 'not-established'): Tone {
  if (total === 0) return 'quiet';
  return kind === 'observed' ? 'stale' : 'unknown';
}

/** Whether the two families disagree about what happened, which is what makes the split visible. */
export function evidenceSplitReading(evidence: TrustEvidence): string {
  const observed = evidence.observed?.total ?? 0;
  const unchecked = evidence.not_established?.total ?? 0;
  if (evidence.observed === null || evidence.not_established === null) {
    return 'Neither family has a tally, because nothing was measured.';
  }
  if (observed > 0 && unchecked > 0) {
    return `Both at once: ${observed} measured divergence(s), and ${unchecked} file(s) nobody looked at. The verdict is "stale" because an observation outranks a gap in the observing — but the second number is not covered by the first.`;
  }
  if (observed > 0) {
    return `${observed} measured divergence(s), and every indexed file was compared. This verdict rests on something that was looked at.`;
  }
  if (unchecked > 0 || evidence.not_established.truncated) {
    return `Nothing was observed to have changed. ${unchecked} file(s) were never compared, so this verdict is the absence of a measurement rather than a finding of staleness.`;
  }
  return 'Nothing diverged and nothing went unchecked.';
}

/**
 * The one-line reading of an overview freshness sweep, for the rail and the overview chip.
 *
 * **Deliberately not a verdict.** `/api/overview` reports the freshness sweep only, and the sweep
 * cannot see an added file — so a chip there saying "index is current" would be claiming something
 * only `/api/check` measures. Three labels, each about the sweep and nothing more, and the screen
 * that carries them points at the verdict rather than substituting for it.
 */
export function sweepLabel(
  freshness: { stale: number; missing: number; refused: number; unreadable: number; truncated: boolean; files_probed: number } | null,
): { label: string; tone: Tone } {
  if (freshness === null || freshness.files_probed === 0) {
    return { label: 'no files were checked', tone: 'quiet' };
  }
  if (freshness.stale + freshness.missing > 0) {
    return { label: 'indexed files have drifted', tone: 'stale' };
  }
  if (freshness.truncated || freshness.refused + freshness.unreadable > 0) {
    return { label: 'not every file was checked', tone: 'unknown' };
  }
  return { label: 'every indexed file matches', tone: 'fresh' };
}
