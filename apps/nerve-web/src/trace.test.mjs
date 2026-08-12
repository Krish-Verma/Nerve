// Trace evidence is existential, and every test here is about keeping it that way.
//
// A trace says *one run took this edge*, not *that every run does*, and absence of an edge is
// absence of observation. The handoff note for Slice 11a lists four ways a view can be wrong while
// looking entirely reasonable, and each of them has a test below:
//
//   1. rendering a run where a *set* of runs was recorded;
//   2. recomputing the derived scalars from `runs[0]` instead of reading the weakest across runs;
//   3. collapsing `unverified` into `stale`, or into a pass;
//   4. printing a `count` as though it were a frequency.
//
// Plus the fifth, which is not on that list because it is about the screen rather than the data:
// drawing "no observed calls" the way "no callers" is drawn, which turns missing instrumentation
// into an apparent fact about the code.
//
// Two kinds of assertion, as in `memory.test.mjs`: over the pure functions, and over the **source**
// of the view, because "this file never renders a trace edge as a static call" is structural and no
// behavioural test can see it.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  bindingReading,
  bindingTone,
  completionTone,
  EXISTENTIAL_STATEMENT,
  isTraceObservation,
  NOT_OBSERVED_STATEMENT,
  parseTraceEnvironment,
  runSetReading,
  sourceMapTone,
  summariseRuns,
  testCountReading,
  testsNamed,
  TRACE_SOURCE_TYPES,
} from './trace.ts';

const here = dirname(fileURLToPath(import.meta.url));
const read = (relative) => readFileSync(join(here, relative), 'utf8');

/** The source with its comment lines dropped: a file naming a rule in order to state it is fine. */
const withoutComments = (source) =>
  source
    .split('\n')
    .filter((line) => !/^\s*(\/\/|\*|\/\*|\{\/\*)/.test(line))
    .join('\n');

/** `TraceBinding::ALL`, in the Rust declaration order — weakest claim first. */
const BINDINGS = ['stale', 'unverified', 'bound'];

/** `CompletionState::ALL`. */
const COMPLETIONS = ['crashed', 'partial', 'complete'];

/** `SourceMapState::ALL`. */
const SOURCE_MAPS = ['none', 'applied', 'unavailable'];

/** One run entry, in the shape `trace_ingest.rs::run_entry` writes. */
const run = (over = {}) => ({
  run_id: 'run-1',
  artifact_path: 'nerve-trace/run-1.json',
  artifact_content_hash: 'abc',
  producer: 'nerve-tracer-python',
  producer_version: '0.1.0',
  test_framework: 'pytest',
  runtime: 'cpython',
  runtime_version: '3.12',
  platform: 'darwin',
  started_at: '2026-01-01T00:00:00Z',
  completed_at: '2026-01-01T00:00:10Z',
  completion_state: 'complete',
  partial_reason: null,
  source_map_state: 'none',
  repository_binding: 'bound',
  producer_limitations: [],
  records: 12,
  observed_count: 12,
  tests: { 'test_area': 3 },
  ...over,
});

const environment = (over = {}) =>
  JSON.stringify({
    runs: [run()],
    completion_state: 'complete',
    repository_binding: 'bound',
    tests: ['test_area'],
    ...over,
  });

test('a trace observation is recognised by its evidence source type', () => {
  assert.equal(isTraceObservation({ evidence_source_type: 'TEST_CALL_TRACE' }), true);
  assert.equal(isTraceObservation({ evidence_source_type: 'RUNTIME_CALL_TRACE' }), true);
  assert.equal(isTraceObservation({ evidence_source_type: 'AST_DIRECT' }), false);
  // `TEST_COVERAGE` is the neighbouring trap: also from a test run, also not a call.
  assert.equal(isTraceObservation({ evidence_source_type: 'TEST_COVERAGE' }), false);
  assert.equal(TRACE_SOURCE_TYPES.length, 2);
});

test('the environment parses into a list of runs, never into one run', () => {
  const parsed = parseTraceEnvironment(environment());
  assert.ok(Array.isArray(parsed.runs), 'runs must be an array');
  assert.equal(parsed.runs.length, 1);
  assert.equal(parsed.runs[0].run_id, 'run-1');
  assert.deepEqual(parsed.tests, ['test_area']);
  assert.deepEqual(parsed.runs[0].tests, { test_area: 3 });
});

test('two tests at one site are one observation naming both, and both survive parsing', () => {
  // `idx_observation_identity` has no column that could hold a second row per test, so this is the
  // shape the database forces. A view that rendered one name would be picking a set member.
  const parsed = parseTraceEnvironment(
    environment({
      runs: [run({ tests: { test_a: 1, test_b: 4 } })],
      tests: ['test_a', 'test_b'],
    }),
  );
  assert.deepEqual(testsNamed(parsed), ['test_a', 'test_b']);
});

test('the derived scalars are copied from the object, never recomputed from runs[0]', () => {
  // The rule: they are the *weakest* value across contributing runs. Here the first run is complete
  // and bound while the merged answer is partial and stale — reading `runs[0]` would report the
  // reassuring value, which is exactly the defect.
  const parsed = parseTraceEnvironment(
    environment({
      runs: [run(), run({ run_id: 'run-2', completion_state: 'partial', repository_binding: 'stale' })],
      completion_state: 'partial',
      repository_binding: 'stale',
    }),
  );
  assert.equal(parsed.completion_state, 'partial');
  assert.equal(parsed.repository_binding, 'stale');
  assert.equal(parsed.runs[0].completion_state, 'complete', 'the per-run value is untouched');
  assert.notEqual(parsed.completion_state, parsed.runs[0].completion_state);
});

test('unparseable, hostile and non-trace environments come back as null rather than as a shape', () => {
  let checked = 0;
  for (const value of [
    null,
    '',
    'not json at all',
    '"a plain string"',
    '42',
    '[]',
    '{}',
    '{"runs": "not an array"}',
    '{"runs": null}',
  ]) {
    checked += 1;
    assert.equal(parseTraceEnvironment(value), null, `${value} must not parse into a run set`);
  }
  assert.equal(checked, 9);
});

test('a field of the wrong type is dropped rather than coerced', () => {
  // The artifact is a user's own tracer's output: untrusted input, twice over.
  const parsed = parseTraceEnvironment(
    JSON.stringify({
      runs: [{ run_id: 42, tests: { good: 2, bad: 'lots' }, producer_limitations: ['a', 7] }, 'not an object'],
      completion_state: 99,
      repository_binding: null,
      tests: ['ok', 3],
    }),
  );
  assert.equal(parsed.runs.length, 1, 'a non-object run is dropped');
  assert.equal(parsed.runs[0].run_id, null, 'a numeric id is not coerced to a string');
  assert.deepEqual(parsed.runs[0].tests, { good: 2 }, 'a non-numeric count is dropped');
  assert.deepEqual(parsed.runs[0].producer_limitations, ['a']);
  assert.equal(parsed.completion_state, null);
  assert.deepEqual(parsed.tests, ['ok']);
});

test('`unverified` is neither `stale` nor a pass — the three-valued binding survives', () => {
  assert.equal(bindingTone('bound'), 'fresh');
  assert.equal(bindingTone('stale'), 'stale');
  assert.equal(bindingTone('unverified'), 'quiet');

  // The three that must never collapse into each other.
  const tones = new Set(BINDINGS.map(bindingTone));
  assert.equal(tones.size, 3, 'three values must have three tones');
  assert.notEqual(bindingTone('unverified'), bindingTone('stale'));
  assert.notEqual(bindingTone('unverified'), bindingTone('bound'));

  // And a value this build does not recognise is drawn as unrecognised rather than as one of them.
  assert.equal(bindingTone('invented'), 'unknown');
  assert.equal(bindingTone(null), 'unknown');
});

test('the binding readings say which of the three each is, and never reassure about unverified', () => {
  assert.match(bindingReading('bound'), /this exact tree/i);
  assert.match(bindingReading('stale'), /different tree/i);

  const unverified = bindingReading('unverified');
  assert.match(unverified, /absence of a check/i);
  assert.match(unverified, /not stale/i);
  assert.match(unverified, /not a pass/i);

  let checked = 0;
  for (const value of BINDINGS) {
    checked += 1;
    assert.notEqual(bindingReading(value), '', `${value} needs a reading`);
  }
  assert.equal(checked, 3);
});

test('completion and source-map tones name every member and no two collapse wrongly', () => {
  assert.equal(completionTone('complete'), 'fresh');
  assert.equal(completionTone('partial'), 'stale');
  // A crash is not staleness: nothing is out of date, the process died.
  assert.equal(completionTone('crashed'), 'absent');
  assert.notEqual(completionTone('crashed'), completionTone('partial'));
  assert.equal(completionTone('invented'), 'unknown');
  assert.equal(new Set(COMPLETIONS.map(completionTone)).size, 3);

  assert.equal(sourceMapTone('applied'), 'fresh');
  assert.equal(sourceMapTone('unavailable'), 'unknown');
  assert.equal(new Set(SOURCE_MAPS.map(sourceMapTone)).size, 3);
});

test('a count is never printed without the run that produced it', () => {
  const one = run({ run_id: 'run-7', tests: { test_area: 3 } });
  const reading = testCountReading(one, 'test_area');
  assert.match(reading, /3 time/);
  assert.match(reading, /run-7/, 'the run must be named beside the count');

  // A run with no id still names something rather than presenting a bare number.
  assert.match(testCountReading(run({ run_id: null }), 'test_area'), /unnamed run/);

  // A test this run did not name has no count to print, and none is invented — not even a zero,
  // which would assert that the run watched for this edge and never saw it.
  const missing = testCountReading(one, 'test_other');
  assert.doesNotMatch(missing, /time\(s\)/);
  assert.doesNotMatch(missing, /\b0\b/);
  assert.match(missing, /run-7/, 'it still says which run this is about');
});

test('runs are summarised by id, and per-site counts are never summed into a frequency', () => {
  const summaries = summariseRuns([
    { runs: [run({ tests: { a: 3 } })], completion_state: 'complete', repository_binding: 'bound', tests: ['a'] },
    { runs: [run({ tests: { b: 9 } })], completion_state: 'complete', repository_binding: 'bound', tests: ['b'] },
    { runs: [run({ run_id: 'run-2', tests: { c: 1 } })], completion_state: 'complete', repository_binding: 'bound', tests: ['c'] },
  ]);

  assert.equal(summaries.length, 2, 'two distinct run ids');
  const first = summaries.find((entry) => entry.run.run_id === 'run-1');

  // Sites, which is a fact about the observation set. Summing 3 + 9 would manufacture a "12" that
  // reads as a call frequency, which is the number this evidence cannot support.
  assert.equal(first.sites, 2);
  assert.deepEqual(first.tests, ['a', 'b']);
  assert.notEqual(first.sites, 12);

  assert.deepEqual(summariseRuns([]), []);
});

test('the run-set reading says what several runs do to the derived scalars', () => {
  assert.match(runSetReading({ runs: [] }), /No run/);
  assert.match(runSetReading({ runs: [run()] }), /One run/);

  const many = runSetReading({ runs: [run(), run({ run_id: 'run-2' })] });
  assert.match(many, /2 runs/);
  assert.match(many, /one observation naming all of them/i);
  assert.match(many, /weakest/i);
});

test('the two standing sentences keep every clause that makes them true', () => {
  assert.match(EXISTENTIAL_STATEMENT, /this run took these edges/i);
  assert.match(EXISTENTIAL_STATEMENT, /not that every run does/i);
  assert.match(EXISTENTIAL_STATEMENT, /absence of an edge is absence of observation/i);

  // The single most important sentence on this surface: it is the one a screen gets wrong by
  // saying nothing at all.
  assert.match(NOT_OBSERVED_STATEMENT, /absence of an observation/i);
  assert.match(NOT_OBSERVED_STATEMENT, /not the observation of an absence/i);
});

// ---- the source of the view -------------------------------------------------------------------

test('the evidence view renders the absence of trace evidence as an absence of observation', () => {
  const source = read('./views/Evidence.tsx');
  // The empty case renders a panel rather than nothing, and it prints the standing sentence.
  assert.match(source, /NOT_OBSERVED_STATEMENT/);
  assert.match(source, /EXISTENTIAL_STATEMENT/);
  assert.match(source, /environments\.length === 0/, 'the empty case must be handled explicitly');
});

test('a trace edge is never rendered as a static call', () => {
  const source = withoutComments(read('./views/Evidence.tsx'));

  // The claim row carries the existential qualification attached to it, rather than leaving a
  // reader to infer it from a verb.
  assert.match(source, /TEST_OBSERVED_CALL/);
  assert.match(source, /one run took this edge/i);

  // And no phrasing anywhere that turns a per-run count into a claim about calls in general.
  let checked = 0;
  for (const pattern of [/called \$\{/, /called [0-9N]+ times/i, /always calls/i]) {
    checked += 1;
    assert.doesNotMatch(source, pattern);
  }
  assert.equal(checked, 3);
});

test('the raw environment blob is not printed for an observation this app can read', () => {
  const source = read('./views/Evidence.tsx');
  // `environment` is a JSON document in a text column. Printed as the string it is, the whole of a
  // run's provenance lands on screen in a form nobody can read — which is what this field did.
  assert.match(source, /traced === null \? \(/, 'the raw field must be the fallback, not the default');
  assert.match(source, /<TraceEnvironmentFacts environment=\{traced\} \/>/);
});
