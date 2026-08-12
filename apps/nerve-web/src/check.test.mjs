// The trust screen has one real failure mode and it is not a crash. It is a screen with two states
// where the answer has five — because `stale` and `unverified` mean the same thing to a reader and
// rest on opposite evidence, and a screen that drew them alike would report a tree nobody looked at
// as a tree that changed. `nerve check` gives them one exit code because a shell has one way to say
// "do not proceed"; a screen has room to say which, and these tests are what hold it to that.
//
// Two kinds of assertion, as in `impact.test.mjs` and `memory.test.mjs`:
//
//   1. Assertions over the pure functions in `check.ts` — every verdict gets its own hue, its own
//      heading and its own evidence kind, the two families of counts are never merged, and the null
//      branches are sentences rather than zeroes.
//   2. Assertions over the **source of the views**, because the rules that matter most here are
//      structural and no behavioural test can see them: both evidence panels must be rendered
//      unconditionally, the remedy must be a printed command rather than a control, and the
//      overview must not claim a verdict it does not measure.
//
// Every assertion that something is absent is paired with a count of what was checked. A scan that
// found nothing and a scan that scanned nothing look identical otherwise.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  checkFragment,
  evidenceSplitReading,
  familyTone,
  notEstablishedRows,
  observedRows,
  sweepLabel,
  sweepReading,
  treeReading,
  unindexableReading,
  verdictEvidenceKind,
  verdictHeading,
  verdictTone,
} from './check.ts';
import { indexVerdictGloss } from './vocab.ts';

const here = dirname(fileURLToPath(import.meta.url));
const read = (relative) => readFileSync(join(here, relative), 'utf8');

/** The five verdicts, which are the vocabulary `nerve-index` declares. */
const VERDICTS = ['current', 'no_index', 'unusable', 'stale', 'unverified'];

function sweep(overrides = {}) {
  return {
    files_total: 12,
    files_probed: 12,
    fresh: 12,
    stale: 0,
    missing: 0,
    refused: 0,
    unreadable: 0,
    truncated: false,
    probe_cap: 5000,
    ...overrides,
  };
}

function tree(overrides = {}) {
  return {
    added: 0,
    added_paths: [],
    added_paths_returned: 0,
    added_paths_truncated: false,
    added_paths_limit: 200,
    unindexable: 0,
    ...overrides,
  };
}

// ---- the five verdicts are five ----------------------------------------------------------------

test('every verdict has its own hue, and stale and unverified never share one', () => {
  const tones = VERDICTS.map(verdictTone);
  assert.equal(new Set(tones).size, VERDICTS.length, `two verdicts share a hue: ${tones}`);
  assert.equal(verdictTone('current'), 'fresh');
  assert.equal(verdictTone('stale'), 'stale');
  // Nothing went out of date here; it was never established. The same hue an unresolved reference
  // site gets on the impact screen, and deliberately not the stale one.
  assert.equal(verdictTone('unverified'), 'unknown');
  assert.notEqual(verdictTone('unverified'), verdictTone('stale'));
  // And an unrecognised value is drawn as unrecognised rather than reusing a real member's hue.
  assert.equal(verdictTone('something_this_build_has_never_heard_of'), 'unknown');
});

test('every verdict has its own heading and its own gloss', () => {
  const headings = VERDICTS.map(verdictHeading);
  assert.equal(new Set(headings).size, VERDICTS.length, `two verdicts share a heading: ${headings}`);
  for (const verdict of VERDICTS) {
    assert.ok(verdictHeading(verdict).length > 10, verdict);
    assert.ok(indexVerdictGloss(verdict).length > 20, verdict);
  }
  const glosses = VERDICTS.map(indexVerdictGloss);
  assert.equal(new Set(glosses).size, VERDICTS.length, 'two verdicts share a gloss');

  // The pair the whole vocabulary exists to keep apart says, in words, which is which.
  assert.match(indexVerdictGloss('stale'), /measured/);
  assert.match(indexVerdictGloss('unverified'), /never compared/);
  assert.match(verdictHeading('unverified'), /never compared/);

  // The other pair a careless screen folds together.
  assert.notEqual(indexVerdictGloss('no_index'), indexVerdictGloss('unusable'));
  assert.notEqual(verdictHeading('no_index'), verdictHeading('unusable'));

  // A value from outside the vocabulary is named as unglossed rather than given a neighbour's.
  assert.match(indexVerdictGloss('banana'), /no description/);
});

test('a verdict says whether it rests on a measurement or on the absence of one', () => {
  assert.equal(verdictEvidenceKind('current'), 'measured');
  assert.equal(verdictEvidenceKind('stale'), 'measured');
  assert.equal(verdictEvidenceKind('unverified'), 'not-established');
  assert.equal(verdictEvidenceKind('no_index'), 'none');
  assert.equal(verdictEvidenceKind('unusable'), 'none');
  assert.notEqual(verdictEvidenceKind('stale'), verdictEvidenceKind('unverified'));
});

// ---- the two families of counts ----------------------------------------------------------------

test('the two evidence families are separate rows and are never summed', () => {
  const observed = { changed: 1, removed: 2, added: 3, total: 6 };
  const unchecked = { refused: 4, unreadable: 5, never_probed: 6, truncated: true, total: 9 };

  const left = observedRows(observed).map(([name]) => name);
  const right = notEstablishedRows(unchecked).map(([name]) => name);
  assert.deepEqual(left, ['changed', 'removed', 'added']);
  assert.equal(right.length, 3);
  // No label appears in both, so the two tallies can never be read as one list.
  for (const name of left) assert.ok(!right.includes(name), name);

  // A null family is an empty list rather than a row of zeroes: the panel says "not measured".
  assert.deepEqual(observedRows(null), []);
  assert.deepEqual(notEstablishedRows(null), []);
});

test('the hue of a family says which family it is, not merely that it is non-zero', () => {
  assert.equal(familyTone(0, 'observed'), 'quiet');
  assert.equal(familyTone(0, 'not-established'), 'quiet');
  assert.equal(familyTone(3, 'observed'), 'stale');
  assert.equal(familyTone(3, 'not-established'), 'unknown');
  assert.notEqual(familyTone(3, 'observed'), familyTone(3, 'not-established'));
});

test('the split reading names both numbers when both are non-zero', () => {
  const both = evidenceSplitReading({
    observed: { changed: 1, removed: 0, added: 0, total: 1 },
    not_established: { refused: 2, unreadable: 0, never_probed: 0, truncated: false, total: 2 },
    statement: '',
    families_are_separate: '',
  });
  assert.match(both, /1 measured divergence/);
  assert.match(both, /2 file\(s\) nobody looked at/);
  assert.match(both, /not covered by the first/);

  const measured = evidenceSplitReading({
    observed: { changed: 1, removed: 0, added: 0, total: 1 },
    not_established: { refused: 0, unreadable: 0, never_probed: 0, truncated: false, total: 0 },
    statement: '',
    families_are_separate: '',
  });
  assert.match(measured, /rests on something that was looked at/);

  const notEstablished = evidenceSplitReading({
    observed: { changed: 0, removed: 0, added: 0, total: 0 },
    not_established: { refused: 1, unreadable: 0, never_probed: 0, truncated: false, total: 1 },
    statement: '',
    families_are_separate: '',
  });
  assert.match(notEstablished, /absence of a measurement rather than a finding of staleness/);
  assert.notEqual(measured, notEstablished);

  const nothing = evidenceSplitReading({
    observed: null,
    not_established: null,
    statement: '',
    families_are_separate: '',
  });
  assert.match(nothing, /nothing was measured/);
});

// ---- null is not zero --------------------------------------------------------------------------

test('no sweep and a clean sweep are different sentences', () => {
  const none = sweepReading(null);
  const clean = sweepReading(sweep());
  assert.match(none, /absence of a measurement/);
  assert.match(clean, /re-hashed and compared/);
  assert.notEqual(none, clean);

  // A truncated sweep names the cap that bit, and never claims the rest are unchanged.
  const cut = sweepReading(sweep({ files_total: 40000, files_probed: 5000, truncated: true }));
  assert.match(cut, /5000-file cap/);
  assert.match(cut, /never looked at/);
});

test('the tree walk is described as the measurement the sweep cannot make', () => {
  const clean = treeReading(tree());
  assert.match(clean, /no file the index has never seen/);
  // Even the zero case explains why it is a separate walk, because that is where the reader would
  // otherwise conclude the sweep had covered it.
  assert.match(clean, /separate walk/);

  const grown = treeReading(tree({ added: 3, added_paths: ['a.ts', 'b.ts', 'c.ts'] }));
  assert.match(grown, /3 files have/);
  assert.match(grown, /cannot see these/);
  // The singular is a different sentence rather than the plural with a number swapped in.
  assert.match(treeReading(tree({ added: 1, added_paths: ['a.ts'] })), /^1 file has /);

  assert.match(treeReading(null), /was not walked/);
});

test('an unreadable new file is counted apart from an added one, and only when there is one', () => {
  assert.equal(unindexableReading(tree()), '');
  assert.equal(unindexableReading(null), '');
  const said = unindexableReading(tree({ unindexable: 2 }));
  assert.match(said, /2 files/);
  assert.match(said, /not called additions/);
});

// ---- the sweep label is not a verdict ----------------------------------------------------------

test('the overview label says what was swept and never claims a verdict', () => {
  const fresh = sweepLabel({
    stale: 0,
    missing: 0,
    refused: 0,
    unreadable: 0,
    truncated: false,
    files_probed: 12,
  });
  assert.equal(fresh.label, 'every indexed file matches');
  assert.equal(fresh.tone, 'fresh');
  // It must not say "current": that is a verdict, and this endpoint cannot see an added file.
  for (const value of ['current', 'trusted', 'up to date']) {
    assert.ok(!fresh.label.includes(value), `${fresh.label} claims a verdict`);
  }

  const drifted = sweepLabel({
    stale: 1,
    missing: 0,
    refused: 0,
    unreadable: 0,
    truncated: false,
    files_probed: 12,
  });
  assert.equal(drifted.tone, 'stale');

  // Refused and unreadable files are not drift. They are the third label, with its own hue.
  const unchecked = sweepLabel({
    stale: 0,
    missing: 0,
    refused: 1,
    unreadable: 0,
    truncated: false,
    files_probed: 12,
  });
  assert.equal(unchecked.label, 'not every file was checked');
  assert.equal(unchecked.tone, 'unknown');
  assert.notEqual(unchecked.tone, drifted.tone);
  assert.notEqual(unchecked.label, drifted.label);

  assert.equal(sweepLabel(null).tone, 'quiet');
  assert.equal(
    sweepLabel({ stale: 0, missing: 0, refused: 0, unreadable: 0, truncated: false, files_probed: 0 })
      .tone,
    'quiet',
  );
});

test('the fragment is the route the shell links to', () => {
  assert.equal(checkFragment(), '#/check');
  const routing = read('./routing.ts');
  assert.ok(routing.includes("case 'check':"), 'the parser has no check arm');
  assert.ok(routing.includes('checkFragment()'), 'the builder is not wired to the route');
});

// ---- the views, read as source ------------------------------------------------------------------

test('both evidence panels are rendered unconditionally, including when a family is empty', () => {
  const view = read('./views/Check.tsx');

  // The panels are drawn by one component that takes the whole evidence block, and it is rendered
  // with no guard around it. An absent panel is indistinguishable from a panel nobody wrote.
  assert.ok(view.includes('<Evidence evidence={report.evidence} />'), 'the split is conditional');
  assert.ok(
    !/\{[^}]*\?\s*<Evidence/.test(view),
    'the evidence panels are behind a conditional',
  );

  // Both titles exist and neither names the other family's word.
  assert.ok(view.includes('Divergence that was measured'));
  assert.ok(view.includes('Repository that was never compared'));

  // And the untracked panel is unconditional too — it is the measurement the sweep cannot make.
  assert.ok(view.includes('<Untracked tree={report.tree} />'));
  assert.ok(view.includes('<Sweep sweep={report.sweep} />'));
});

test('the screen renders every state and every verdict', () => {
  const view = read('./views/Check.tsx');
  let checked = 0;
  for (const needle of ['<Loading', '<Failure', 'state.status === \'loading\'', 'state.status === \'error\'']) {
    assert.ok(view.includes(needle), `the ${needle} state is not rendered`);
    checked += 1;
  }
  assert.equal(checked, 4);

  // Every verdict reaches the screen through the vocabulary the answer carried, so a sixth one
  // renders as itself rather than being dropped by a hard-coded list.
  assert.ok(view.includes('report.vocabulary.verdicts.map'), 'the vocabulary is not rendered');
  assert.ok(view.includes('verdictTone(term.verdict)'));
  assert.ok(view.includes('indexVerdictGloss(term.verdict)'));
});

test('the remedy is a printed command and there is no control that writes', () => {
  const view = read('./views/Check.tsx');
  assert.ok(view.includes('report.boundary.commands.map'), 'the commands are not printed');
  assert.ok(view.includes('$ {command}'), 'the commands are not printed as commands');
  assert.ok(view.includes('gate__sample'), 'the commands are not in the command block');

  // No button, no form, no fetch verb other than the read the hook performs.
  let scanned = 0;
  for (const forbidden of ['<button', '<form', 'method:', 'onSubmit', 'POST']) {
    assert.ok(!view.includes(forbidden), `the trust screen ships ${forbidden}`);
    scanned += 1;
  }
  assert.equal(scanned, 5);
});

test('the shell offers the screen and the overview does not claim its verdict', () => {
  const app = read('./App.tsx');
  assert.ok(app.includes("to={{ view: 'check' }}"), 'the rail has no way to reach the verdict');
  assert.ok(app.includes('<Check />'), 'the shell does not render the view');
  assert.ok(app.includes('sweepLabel('), 'the rail still derives its own reading');

  // The rail no longer prints a verdict-shaped sentence off the sweep alone.
  let scanned = 0;
  for (const forbidden of ['every file still matches', 'files have drifted', 'index is current']) {
    assert.ok(!app.includes(forbidden), `the rail still says "${forbidden}"`);
    scanned += 1;
  }
  assert.equal(scanned, 3);

  const overview = read('./views/Overview.tsx');
  assert.ok(
    !overview.includes('index is current'),
    'the overview claims a verdict its endpoint cannot measure',
  );
  assert.ok(overview.includes('sweepLabel('), 'the overview chip is not the sweep label');
  assert.ok(
    overview.includes("href({ view: 'check' })"),
    'the overview does not point at the verdict',
  );
});
