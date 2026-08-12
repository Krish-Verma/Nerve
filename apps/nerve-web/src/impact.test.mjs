// The impact screen's one real failure mode is not a crash — it is a correct-looking answer that
// reads as a clearance. `results` is a reverse dependency closure, three rows read as "three things
// depend on this, it is safe to change", and that is a claim about the repository built from a
// traversal that could only follow the edges Nerve resolved. On `fixtures/ts-basic`, `add` has three
// dependants and four unresolved sites.
//
// So these are tests about wording and about what is unconditional, in two kinds, as in
// `memory.test.mjs`:
//
//   1. Assertions over the pure functions in `impact.ts` — both branches of the unresolved account,
//      the page reading, the tally ordering, and the tones.
//   2. Assertions over the **source of the view**, because the rules that matter most here are
//      structural and no behavioural test can see them: the unresolved panel must be rendered
//      unconditionally, and nothing on the screen may be labelled as test impact.
//
// Every assertion that something is absent is paired with a count of what was checked. A scan that
// found nothing and a scan that scanned nothing look identical otherwise.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  bounded,
  depthRows,
  emptyReading,
  exclusionReading,
  grainReading,
  impactFragment,
  isOptInRelation,
  MAX_DEPTH,
  MAX_LIMIT,
  pageReading,
  rowTone,
  staleReading,
  tallyRows,
  unresolvedReading,
  unresolvedTone,
} from './impact.ts';

const here = dirname(fileURLToPath(import.meta.url));
const read = (relative) => readFileSync(join(here, relative), 'utf8');

/** The source with its comment lines dropped: a file naming a rule in order to state it is fine. */
const withoutComments = (source) =>
  source
    .split('\n')
    .filter((line) => !/^\s*(\/\/|\*|\/\*|\{\/\*)/.test(line))
    .join('\n');

const none = { sites: 0, assertions: 0, targets: 0, by_category: {} };
const some = { sites: 4, assertions: 4, targets: 4, by_category: { value: 4 } };

test('the unresolved account has no silent branch — zero is a sentence, not an omission', () => {
  const zero = unresolvedReading(none);
  const four = unresolvedReading(some);

  assert.notEqual(zero, '', 'the zero case must say something');
  assert.notEqual(four, '');
  assert.notEqual(zero, four);

  // The zero case is where a silent omission most invites the wrong conclusion, so it has to make
  // the positive claim rather than merely not warn.
  assert.match(zero, /every reference site/i);
  assert.match(zero, /resolved/i);
  assert.match(zero, /hiding a dependency/i);

  // And the non-zero case must state that the answer cannot rule them out, rather than merely
  // reporting a number.
  assert.match(four, /4 reference sites/);
  assert.match(four, /cannot rule them out/i);
});

test('the account is worded from `sites`, which is the observation count', () => {
  // `sites >= assertions >= targets` always: one relationship counted at three grains. The headline
  // must be the largest and most honest of the three.
  const lopsided = { sites: 9, assertions: 4, targets: 2, by_category: { value: 9 } };
  assert.match(unresolvedReading(lopsided), /9 reference sites/);
  assert.doesNotMatch(unresolvedReading(lopsided), /\b4 reference\b/);

  // And the three grains, where they are stated, are stated as one fact rather than three findings.
  assert.match(grainReading(lopsided), /Not three findings/);
  assert.equal(grainReading(none), '', 'there is no grain to report when nothing is unresolved');
});

test('one reference site is singular, and the sentence stays grammatical', () => {
  assert.match(unresolvedReading({ ...some, sites: 1 }), /1 reference site\b/);
  assert.doesNotMatch(unresolvedReading({ ...some, sites: 1 }), /1 reference sites/);
});

test('the tone of the account separates a warning from an earned clearance', () => {
  assert.equal(unresolvedTone(some), 'unknown');
  assert.equal(unresolvedTone(none), 'fresh');
  // Never `stale`: nothing here went out of date, it was never worked out in the first place.
  assert.notEqual(unresolvedTone(some), 'stale');
});

test('the page reading distinguishes a cut page from a whole answer', () => {
  assert.equal(pageReading({ count: 3, results_total: 3, truncated: false }), 'Every one of the 3 is listed below.');

  const cut = pageReading({ count: 1, results_total: 3, truncated: true });
  assert.match(cut, /1 of 3/);
  // The tallies are exact whatever the cap removed, and a reader who believed otherwise would treat
  // the one trustworthy number on the screen as a lower bound.
  assert.match(cut, /tallies above are exact/i);

  // Nothing matched, so there is no page to describe. "Every one of them is listed" is a claim
  // about a set with no members.
  assert.equal(pageReading({ count: 0, results_total: 0, truncated: false }), '');
});

test('depth rows come back in numeric order, because "10" sorts before "2" as a string', () => {
  const rows = depthRows({ by_depth: [{ depth: 10, entities: 1 }, { depth: 2, entities: 5 }] });
  assert.deepEqual(rows, [['depth 2', 5], ['depth 10', 1]]);
});

test('a tally is ordered by size, and ties break by name so the order is stable', () => {
  assert.deepEqual(tallyRows({ CALLS: 3, REFERENCES: 7, EXTENDS: 3 }), [
    ['REFERENCES', 7],
    ['CALLS', 3],
    ['EXTENDS', 3],
  ]);
  assert.deepEqual(tallyRows({}), []);
});

test('an unmeasured freshness is not drawn as stale', () => {
  assert.equal(rowTone({ evidence_freshness: 'fresh', is_unresolved: false }), 'fresh');
  assert.equal(rowTone({ evidence_freshness: 'stale', is_unresolved: false }), 'stale');
  assert.equal(rowTone({ evidence_freshness: 'file-missing', is_unresolved: false }), 'absent');

  // `null` is "freshness was not measured", which is not a finding about the file.
  const unmeasured = rowTone({ evidence_freshness: null, is_unresolved: false });
  assert.equal(unmeasured, 'quiet');
  assert.notEqual(unmeasured, 'stale');

  // An unresolved edge outranks freshness: what is unknown about it is bigger than staleness.
  assert.equal(rowTone({ evidence_freshness: 'fresh', is_unresolved: true }), 'unknown');

  // A value this build does not know is drawn as unknown rather than as one of the four.
  assert.equal(rowTone({ evidence_freshness: 'invented', is_unresolved: false }), 'unknown');
});

test('the empty answer names the relations it is relative to', () => {
  const reading = emptyReading(['CALLS', 'REFERENCES']);
  assert.match(reading, /CALLS, REFERENCES/);
  // "Nothing depends on this" without the qualification is the sentence this screen must not print.
  assert.match(reading, /through/);
  assert.notEqual(emptyReading([]), '');
});

test('what was not followed is named, and a trace edge is the one that must be', () => {
  const walked = ['CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS', 'SERVED_BY'];
  const reading = exclusionReading(walked);
  assert.match(reading, /TEST_OBSERVED_CALL/);
  assert.match(reading, /CONTAINS/);
  // The reason, not merely the fact: a trace edge's absence is a decision.
  assert.match(reading, /one run took it/i);

  // Nothing to report when everything notable was walked.
  assert.equal(exclusionReading([...walked, 'TEST_OBSERVED_CALL', 'CONTAINS', 'IMPORTS']), '');

  assert.equal(isOptInRelation('TEST_OBSERVED_CALL'), true);
  assert.equal(isOptInRelation('CALLS'), false);
});

test('stale evidence is reported as unknown survival rather than as a wrong edge', () => {
  assert.equal(staleReading(0), '');
  assert.match(staleReading(1), /1 of these was/);
  assert.match(staleReading(3), /3 of these were/);
  assert.match(staleReading(3), /Re-index/);
});

test('a bounded control never sends nonsense, and clamps where the server clamps', () => {
  assert.equal(bounded('6', MAX_DEPTH), 6);
  assert.equal(bounded('999', MAX_DEPTH), MAX_DEPTH);
  assert.equal(bounded('9999', MAX_LIMIT), MAX_LIMIT);
  assert.equal(bounded('0', MAX_DEPTH), null);
  assert.equal(bounded('', MAX_DEPTH), null);
  assert.equal(bounded('-4', MAX_DEPTH), null);
  assert.equal(bounded('abc', MAX_DEPTH), null);
  // The one that would otherwise reach the server as `NaN` and earn a 400 for a control this app
  // rendered itself.
  assert.equal(bounded('6.5', MAX_DEPTH), null);
});

test('a subject holding a `#` survives the fragment, because `#` is the selector separator', () => {
  const fragment = impactFragment({ subject: 'src/app.ts#Thing' });
  // A raw `#` would start the fragment and the browser would drop everything after it, so the
  // request would be answered — correctly — about a different entity.
  assert.equal(fragment.indexOf('#'), 0, 'only the leading fragment marker may be a raw #');
  assert.match(fragment, /subject=src%2Fapp.ts%23Thing/);
  assert.equal(impactFragment({}), '#/impact');
  assert.equal(impactFragment({ subject: '' }), '#/impact');
});

// ---- the source of the view -------------------------------------------------------------------

test('the unresolved panel is rendered unconditionally, not behind a count', () => {
  const source = read('./views/Impact.tsx');

  // It is called once, from the body of the report, with no `&&` or ternary gating it on a count.
  // Rendering it only when `sites > 0` is the exact defect the whole panel exists to prevent: the
  // zero case would then be an absent panel, which is indistinguishable from a panel nobody wrote.
  const call = /<Unresolved report=\{report\} \/>/;
  assert.match(source, call);

  const gated = /(sites\s*>\s*0|unresolved\.sites)\s*(&&|\?)[^\n]*<Unresolved/;
  assert.doesNotMatch(source, gated, 'the account must not be conditional on there being any');
});

test('nothing on this screen is labelled as test impact', () => {
  // `nerve affected` is refused rather than deferred: LCOV carries no per-test attribution
  // (ADR-0008 §A.2). A test file in an impact set is there because code depends on code.
  const source = withoutComments(read('./views/Impact.tsx'));
  const forbidden = [/affected tests/i, /test impact/i, /tests affected/i, /impacted tests/i];
  let checked = 0;
  for (const pattern of forbidden) {
    checked += 1;
    assert.doesNotMatch(source, pattern, `the view must not say ${pattern}`);
  }
  assert.equal(checked, 4);
});

test('the view builds no markup out of repository text', () => {
  const source = read('./views/Impact.tsx');

  // The last character of each forbidden spelling is written as a one-member character class. It
  // matches exactly the same text, and it keeps this file from tripping `tools/lint.mjs`, which
  // bans the literal spellings in every source file and cannot tell a rule from a violation of it.
  const checked = [/dangerouslySetInnerHTM[L]/, /\.innerHTM[L]\b/, /insertAdjacentHTM[L]/];
  for (const pattern of checked) {
    assert.doesNotMatch(source, pattern, String(pattern));
  }
  assert.equal(checked.length, 3);
});

test('the view reads the relation set off the answer rather than mirroring one', () => {
  const source = withoutComments(read('./views/Impact.tsx'));
  // The set that was walked is on screen because "nothing depends on this" is only true relative
  // to it, and it comes from `report.relations` — a local list would be a second copy of a
  // decision that lives in `nerve-store`, free to drift.
  assert.match(source, /report\.relations\.map/);
  assert.doesNotMatch(
    source,
    /const\s+\w*RELATIONS\w*\s*[:=]\s*\[\s*'CALLS'/,
    'the default relation set must not be mirrored in the view',
  );
});
