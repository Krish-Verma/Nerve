// Every mistake available in history data is a wording mistake, so these are wording tests.
//
// Two kinds live here and they catch different things:
//
//   1. Assertions over the pure functions in `history.ts`. These are the decisions — is this a
//      creation, is this a beginning, is this a range or a refusal — and they are functions rather
//      than inline expressions in a component precisely so that something can hold them to it.
//   2. Assertions over the **source of the views**. Three of the rules this row exists for are
//      about what reaches the screen: the co-change disclaimer must be printed, a `null` commit
//      list must not be rendered as an empty one, and no view may hold a copy of a note the
//      backend already sends. A behavioural test cannot see any of those — the decision function
//      would still pass while the component ignored it — so the file is read as text, the way
//      `crates/nerve-server/tests/ui_vocabulary.rs` reads this app's vocabulary.
//
// Every assertion that something is absent is paired with a count of what was checked. A scan
// that found nothing and a scan that scanned nothing look identical otherwise.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  changeCountReading,
  cochangeDisclaimer,
  commitBoundary,
  diffReading,
  fileMode,
  firstObservedHeadline,
  gitTime,
  historyFragment,
  isHistoryTab,
  HISTORY_TABS,
} from './history.ts';

const here = dirname(fileURLToPath(import.meta.url));
const read = (relative) => readFileSync(join(here, relative), 'utf8');

/**
 * The source with its comment lines dropped.
 *
 * A file that names a forbidden word *in order to forbid it* is not committing the offence — the
 * co-change view's own doc comment says the count is never an affinity, and a scan that could not
 * tell the two apart would force the rule to go undocumented in the file it governs. This is the
 * same allowance `tools/lint.mjs` makes for the spellings it bans, decided the same way: by where
 * the line starts.
 */
const withoutComments = (source) =>
  source
    .split('\n')
    .filter((line) => !/^\s*(\/\/|\*|\/\*|\{\/\*)/.test(line))
    .join('\n');

/** `FirstObservedKind::ALL`, in the Rust declaration order. */
const FIRST_OBSERVED_KINDS = [
  'created_in_visible_history',
  'earliest_visible_change',
  'present_before_visible_history',
  'absent_from_visible_history',
  'current_tree_unknown',
  'no_history_ingested',
];

test('only the carried permission produces the word created', () => {
  assert.match(firstObservedHeadline('created_in_visible_history', true), /created/);

  // Every one of the six, without the permission. Not one may claim a creation — including the
  // value whose own name reads like one, which is the case a `kind === '…'` branch would get
  // wrong the moment the backend's rule and this app's copy of it drifted.
  let checked = 0;
  for (const kind of FIRST_OBSERVED_KINDS) {
    const headline = firstObservedHeadline(kind, false);
    assert.doesNotMatch(headline, /creat/i, `${kind} claimed a creation`);
    assert.doesNotMatch(headline, /first ever|first commit|beginning of history/i, kind);
    assert.ok(headline.length > 0, kind);
    checked += 1;
  }
  assert.equal(checked, 6, 'all six values must be exercised');
});

test('each first-observed value is refused the reading Entry 7 forbids it', () => {
  // The "never" column of the handoff's table, value by value.
  const forbidden = {
    earliest_visible_change: /first|creat/i,
    present_before_visible_history: /no history/i,
    absent_from_visible_history: /deleted/i,
    current_tree_unknown: /does not exist/i,
    no_history_ingested: /^no history$/i,
  };
  let checked = 0;
  for (const [kind, pattern] of Object.entries(forbidden)) {
    assert.doesNotMatch(firstObservedHeadline(kind, false), pattern, kind);
    checked += 1;
  }
  assert.equal(checked, 5);
});

test('an unknown first-observed value borrows no phrasing at all', () => {
  const headline = firstObservedHeadline('some_seventh_value', false);
  assert.doesNotMatch(headline, /creat/i);
  assert.match(headline, /no phrasing/);
});

test('only a root commit is a beginning, and a boundary is not one', () => {
  const root = commitBoundary({
    may_claim_history_begins_here: true,
    parent_completeness: 'root',
    parent_oids: [],
  });
  assert.equal(root.begins, true);
  assert.match(root.label, /begins/);

  let checked = 0;
  for (const completeness of ['shallow_boundary', 'parents_missing', 'parents_unverifiable']) {
    const boundary = commitBoundary({
      may_claim_history_begins_here: false,
      parent_completeness: completeness,
      parent_oids: [],
    });
    assert.equal(boundary.begins, false, completeness);
    assert.doesNotMatch(boundary.label, /begins|first|root/i, completeness);
    assert.match(boundary.label, /visible/, completeness);
    checked += 1;
  }
  assert.equal(checked, 3);

  // An ordinary commit is not labelled at all, so the label means something where it appears.
  assert.equal(
    commitBoundary({
      may_claim_history_begins_here: false,
      parent_completeness: 'parents_available',
      parent_oids: ['a'.repeat(40)],
    }),
    null,
  );
});

test('a change count that was never taken is not a zero', () => {
  const absent = changeCountReading(null);
  assert.equal(absent.counted, false);
  assert.doesNotMatch(absent.text, /\b0\b/);

  const measured = changeCountReading(0);
  assert.equal(measured.counted, true);
  assert.equal(measured.text, '0 changes');
  assert.equal(changeCountReading(1).text, '1 change');
  assert.equal(changeCountReading(4).text, '4 changes');
});

test('a diff refusal is never an empty range', () => {
  const refusals = [
    'state_not_recorded',
    'not_an_ancestor',
    'ancestry_incomplete',
    'walk_budget_exhausted',
  ];
  let checked = 0;
  for (const kind of refusals) {
    // Every diff-shaped key is null on a refusal, which is the shape being relied on.
    const reading = diffReading({
      result_kind: kind,
      commits: null,
      changes: null,
      commits_in_range: null,
      commits_truncated: null,
      changes_truncated: null,
      merges_in_range: null,
    });
    assert.equal(reading.outcome, 'refused', kind);
    assert.equal(reading.kind, kind);
    assert.equal(reading.commits, undefined, 'a refusal carries no commit list at all');
    checked += 1;
  }
  assert.equal(checked, 4, 'all four refusals must be exercised');

  // And the empty range, which is a different answer and is reachable.
  const empty = diffReading({
    result_kind: 'diff',
    commits: [],
    changes: [],
    commits_in_range: 0,
    commits_truncated: false,
    changes_truncated: false,
    merges_in_range: 0,
  });
  assert.equal(empty.outcome, 'range');
  assert.deepEqual(empty.commits, []);
  assert.equal(empty.commitsInRange, 0);
});

test('the co-change disclaimer is carried through unchanged', () => {
  const sent =
    'a shared-commit count is an observation, not a dependency: two paths changing together is ' +
    'equally consistent with coupling, a formatting sweep, a version bump, and one commit that ' +
    'did two unrelated things';
  assert.equal(cochangeDisclaimer({ disclaimer: sent }), sent);

  // A missing one is refused rather than replaced with a softer sentence of our own.
  for (const missing of [{}, { disclaimer: null }, { disclaimer: '   ' }]) {
    const fallback = cochangeDisclaimer(missing);
    assert.match(fallback, /nothing here may be read as a dependency/);
  }
});

test('a commit time is shown in the offset the commit recorded', () => {
  // 2026-01-01T00:00:00+0000, the first commit of fixtures/history-basic.
  assert.equal(gitTime(1767225600, '+0000'), '2026-01-01 00:00:00 +0000');
  // The same instant, recorded by somebody five and a half hours ahead.
  assert.equal(gitTime(1767225600, '+0530'), '2026-01-01 05:30:00 +0530');
  assert.equal(gitTime(1767225600, '-0800'), '2025-12-31 16:00:00 -0800');
  // A malformed offset is reported as what it is rather than silently treated as UTC.
  assert.match(gitTime(1767225600, 'nonsense'), /1767225600 seconds/);
});

test('a file mode is octal, and an absent one is not zero', () => {
  assert.equal(fileMode(0o100644), '100644');
  assert.equal(fileMode(0o100755), '100755');
  assert.equal(fileMode(null), 'none recorded');
});

test('a path with a # survives as %23, because a raw # never leaves the browser', () => {
  const fragment = historyFragment('path', { path: 'src/app.ts#parse' });
  assert.match(fragment, /%23/);
  assert.doesNotMatch(fragment.slice('#/history/path'.length), /#/);

  // And it round-trips: what the server is asked is what the user typed.
  const query = new URLSearchParams(fragment.split('?')[1]);
  assert.equal(query.get('path'), 'src/app.ts#parse');

  assert.equal(historyFragment('commits', {}), '#/history/commits');
  assert.equal(historyFragment('commits', { commit: '' }), '#/history/commits');
});

test('the tab vocabulary is closed', () => {
  assert.equal(HISTORY_TABS.length, 5);
  for (const tab of HISTORY_TABS) assert.ok(isHistoryTab(tab));
  assert.equal(isHistoryTab('renames'), false);
  assert.equal(isHistoryTab(undefined), false);
});

// ---- what reaches the screen -----------------------------------------------------------------

test('the co-change view prints the disclaimer the response carried', () => {
  const source = read('./views/HistoryPath.tsx');
  assert.ok(
    source.includes('cochangeDisclaimer(report)'),
    'the co-change view must render the disclaimer the API sent; it is the one line that stops a shared-commit count reading as a dependency',
  );

  // And it must not relabel the count as the thing the disclaimer denies. A server test fails on
  // these words; the interface holds the same line.
  const rendered = withoutComments(source).toLowerCase();
  let checked = 0;
  for (const forbidden of ['coupled', 'affinity', 'depends on', 'related to']) {
    assert.ok(!rendered.includes(forbidden), `the view labels co-change as "${forbidden}"`);
    checked += 1;
  }
  assert.equal(checked, 4);
  assert.ok(source.includes('shared commit'), 'the count must be named for what was observed');
});

test('the diff view asks which outcome it has rather than iterating the array', () => {
  const source = read('./views/HistoryDiff.tsx');
  assert.ok(
    source.includes('diffReading(report)'),
    'the diff view must branch on diffReading, which is where null-is-not-empty is decided',
  );
  assert.ok(source.includes("reading.outcome === 'refused'"), 'the refusal branch must exist');

  // Coercing the absent list into an empty one is the defect the shape exists to prevent.
  let checked = 0;
  for (const coercion of ['report.commits ?? []', 'report.commits || []', '(report.commits ?? [])']) {
    assert.ok(!source.includes(coercion), `the diff view coerces an absent range with \`${coercion}\``);
    checked += 1;
  }
  assert.equal(checked, 3);
});

test('no history view composes a claim from a kind instead of reading the permission', () => {
  let checked = 0;
  for (const view of ['./views/HistoryPath.tsx', './views/History.tsx', './views/HistoryParts.tsx']) {
    const source = read(view);
    assert.ok(
      !source.includes("=== 'created_in_visible_history'"),
      `${view} branches on the kind; the permission is may_claim_created and it is carried`,
    );
    assert.ok(
      !source.includes('may_claim_history_begins_here ='),
      `${view} recomputes a permission the response already carried`,
    );
    checked += 1;
  }
  assert.equal(checked, 3);

  // The positive half: the permission and its sentence do reach the screen.
  const path = read('./views/HistoryPath.tsx');
  assert.ok(path.includes('may_claim_created_note'), 'the backend sentence must be rendered');
  assert.ok(path.includes('observed.kind_note'), 'the backend sentence must be rendered');
});
