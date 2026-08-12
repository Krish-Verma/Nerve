/**
 * Trace evidence: reading it, and the four ways a view of it goes wrong while looking reasonable.
 *
 * A trace is **existential** evidence. It says *this run took these edges*, not *that every run
 * does*, and **absence of an edge is absence of observation** rather than evidence that no call
 * exists. An untraced repository has zero trace edges and a fully-traced one has some, so a screen
 * that drew "no observed calls" the way it draws "no callers" would turn missing instrumentation
 * into an apparent fact about the code. That is `gaps`' `null`-versus-`0` problem in a new place,
 * and it is the same rule as ADR-0005's: coverage is not a call graph, and neither is a trace.
 *
 * There is deliberately **no `/api/trace`**. Import is a write path and is CLI-only; trace
 * observations reach this app through `/api/why` and `/api/entity`, because they are observations
 * like any other and those endpoints are generic over the evidence model. Nothing here fetches.
 *
 * The four rules this file encodes, each of which is a way to be wrong while looking fine:
 *
 * 1. **A set of runs, not a run.** `idx_observation_identity` has no column that could hold a
 *    second row per test, so two tests reaching one callee from one line are *one* observation
 *    naming both. [`parseTraceEnvironment`] always yields a list.
 * 2. **The derived scalars are the weakest value across contributing runs**, not the first one's.
 *    They are read off the object and never recomputed here — a site observed by one complete run
 *    and one interrupted run reads as `partial`, and recomputing from `runs[0]` would lose that.
 * 3. **`repository_binding` has three values and `unverified` is not `stale`.** The absence of a
 *    check is not a failed check. A two-state badge here would be a lie in one direction or the
 *    other; see [`bindingTone`], which has no `default` arm returning something settled.
 * 4. **`count` is not a frequency.** It is how many times *one run* took that edge. Never labelled
 *    "called N times" without naming the run — see [`testCountReading`].
 *
 * Everything parsed here came out of an artifact a user's own tracer wrote. It is untrusted input
 * on exactly the terms the rest of this app treats repository content, and it is parsed rather
 * than trusted: a field of the wrong type is dropped, not coerced and not rendered as a hole.
 */

import type { Json, Observation, TraceEnvironment, TraceRun } from './api/types';
import type { Tone } from './format';

/**
 * Evidence source types whose observations are a watched execution.
 *
 * `TEST_CALL_TRACE` is what `nerve trace import` writes today. `RUNTIME_CALL_TRACE` is in the
 * vocabulary and nothing writes one yet; it is included because if something ever does, the
 * existential reading applies to it *more* strongly rather than less, and the failure mode of
 * leaving it out is a view that quietly renders a runtime observation as an ordinary static fact.
 */
export const TRACE_SOURCE_TYPES: readonly string[] = ['TEST_CALL_TRACE', 'RUNTIME_CALL_TRACE'];

/** Whether an observation is a watched execution rather than a reading of the source. */
export function isTraceObservation(observation: Pick<Observation, 'evidence_source_type'>): boolean {
  return TRACE_SOURCE_TYPES.includes(observation.evidence_source_type);
}

/**
 * The sentence the CLI prints, in this interface's own voice.
 *
 * A view needs its own version rather than a shortened one: every clause is load-bearing, and the
 * final one is the clause a reader is most likely to supply wrongly on their own.
 */
export const EXISTENTIAL_STATEMENT =
  'A trace is existential evidence: it says this run took these edges, not that every run does, and absence of an edge is absence of observation.';

/**
 * What it means that a symbol has no trace evidence at all.
 *
 * The single most important sentence on this surface, because it is the one a screen gets wrong by
 * saying nothing. "No observed calls" and "no callers" must never be drawn alike.
 */
export const NOT_OBSERVED_STATEMENT =
  'Nothing traced this. That is the absence of an observation, not the observation of an absence — a repository with no tracer run has no trace evidence anywhere, and it says nothing about whether these calls happen.';

function asString(value: Json): string | null {
  return typeof value === 'string' ? value : null;
}

function asNumber(value: Json): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function asObject(value: Json): Record<string, Json> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, Json>)
    : null;
}

/** `{test: count}`, keeping only the entries that are actually a name and a number. */
function testCounts(value: Json): Record<string, number> {
  const object = asObject(value);
  if (object === null) return {};
  const out: Record<string, number> = {};
  for (const [test, hits] of Object.entries(object)) {
    const count = asNumber(hits);
    if (count !== null) out[test] = count;
  }
  return out;
}

function stringList(value: Json): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
}

function parseRun(value: Json): TraceRun | null {
  const run = asObject(value);
  if (run === null) return null;
  return {
    run_id: asString(run['run_id'] ?? null),
    artifact_path: asString(run['artifact_path'] ?? null),
    artifact_content_hash: asString(run['artifact_content_hash'] ?? null),
    producer: asString(run['producer'] ?? null),
    producer_version: asString(run['producer_version'] ?? null),
    test_framework: asString(run['test_framework'] ?? null),
    runtime: asString(run['runtime'] ?? null),
    runtime_version: asString(run['runtime_version'] ?? null),
    platform: asString(run['platform'] ?? null),
    started_at: asString(run['started_at'] ?? null),
    completed_at: asString(run['completed_at'] ?? null),
    completion_state: asString(run['completion_state'] ?? null),
    partial_reason: asString(run['partial_reason'] ?? null),
    source_map_state: asString(run['source_map_state'] ?? null),
    repository_binding: asString(run['repository_binding'] ?? null),
    producer_limitations: stringList(run['producer_limitations'] ?? null),
    records: asNumber(run['records'] ?? null),
    observed_count: asNumber(run['observed_count'] ?? null),
    tests: testCounts(run['tests'] ?? null),
  };
}

/**
 * Parse `observation.environment` into the run set it holds.
 *
 * Returns `null` when the text is absent, is not JSON, or is JSON that carries no `runs` array —
 * which is the ordinary case for every non-trace observation in the database, where `environment`
 * is either null or a plain string. A caller distinguishes *this is not trace evidence* from *this
 * is trace evidence I could not read* by testing the source type first.
 *
 * **The derived scalars are copied, never computed.** Recomputing `completion_state` from the run
 * list would be a second implementation of the "weakest across contributing runs" rule, and the
 * copy would be the one on screen.
 */
export function parseTraceEnvironment(environment: string | null): TraceEnvironment | null {
  if (environment === null || environment === '') return null;
  let parsed: Json;
  try {
    parsed = JSON.parse(environment) as Json;
  } catch {
    return null;
  }
  const object = asObject(parsed);
  if (object === null) return null;
  const raw = object['runs'];
  if (!Array.isArray(raw)) return null;
  const runs = raw.map(parseRun).filter((run): run is TraceRun => run !== null);
  return {
    runs,
    completion_state: asString(object['completion_state'] ?? null),
    repository_binding: asString(object['repository_binding'] ?? null),
    tests: stringList(object['tests'] ?? null),
  };
}

/**
 * Hue for the three-valued repository binding.
 *
 * There is deliberately no `default` arm returning a settled colour, for the reason
 * `directnessClass` has none: a value this build does not recognise must be drawn as unrecognised
 * rather than as one of the three. And `unverified` is `quiet` rather than `stale`, because the two
 * are the pair that must not collapse — one says the artifact names a *different* tree, the other
 * says it named no tree at all, so nothing was checked.
 */
export function bindingTone(binding: string | null): Tone {
  switch (binding) {
    case 'bound':
      return 'fresh';
    case 'stale':
      return 'stale';
    case 'unverified':
      return 'quiet';
  }
  return 'unknown';
}

/**
 * Hue for how far the run got.
 *
 * `crashed` is `absent` rather than `stale`: nothing about it is out of date, the process died and
 * whatever it had not reached was never observed at all.
 */
export function completionTone(state: string | null): Tone {
  switch (state) {
    case 'complete':
      return 'fresh';
    case 'partial':
      return 'stale';
    case 'crashed':
      return 'absent';
  }
  return 'unknown';
}

/** Hue for whether recorded locations are original source or generated code. */
export function sourceMapTone(state: string | null): Tone {
  switch (state) {
    case 'none':
      return 'plain';
    case 'applied':
      return 'fresh';
    case 'unavailable':
      return 'unknown';
  }
  return 'unknown';
}

/**
 * How many runs contributed, and what that does to the derived scalars above.
 *
 * Stated because the weakest-across-runs rule is invisible otherwise: a reader looking at
 * `partial` beside two runs needs to know it may be one run's partiality, not both.
 */
export function runSetReading(environment: Pick<TraceEnvironment, 'runs'>): string {
  const runs = environment.runs.length;
  if (runs === 0) return 'No run is named on this observation.';
  if (runs === 1) return 'One run observed this site.';
  return `${runs} runs observed this site, and it is one observation naming all of them rather than ${runs} observations. The completion and binding above are the weakest value across them, so one interrupted run makes the pair read as interrupted.`;
}

/**
 * A test's count, **with the run named**, because a bare count is not a frequency.
 *
 * `count` is how many times one run took the edge. Two runs' counts are recorded per run rather
 * than summed into a total, so there is no number here that could honestly be labelled "called N
 * times" — and this function will not produce one without the run beside it.
 */
export function testCountReading(run: TraceRun, test: string): string {
  const hits = run.tests[test];
  const where = run.run_id ?? 'an unnamed run';
  if (hits === undefined) return `observed by ${where}`;
  return `${hits} time(s) during ${where}`;
}

/**
 * Every test named across every contributing run, as a list.
 *
 * Read off the union the writer already computed where it is present, and rebuilt from the runs
 * where it is not. A view that rendered one test name would be picking one member of a set.
 */
export function testsNamed(environment: TraceEnvironment): string[] {
  if (environment.tests.length > 0) return environment.tests;
  const union = new Set<string>();
  for (const run of environment.runs) {
    for (const test of Object.keys(run.tests)) union.add(test);
  }
  return [...union].sort();
}

/** One run, and how much of what is on screen named it. */
export interface TraceRunSummary {
  /** The run's own metadata, taken from the first observed site that named it. */
  run: TraceRun;
  /**
   * How many **observed sites** named this run.
   *
   * Sites, and the word is chosen rather than convenient. It is not a number of calls, not a
   * frequency, and not comparable between runs — a run that observed one site a thousand times
   * counts 1 here. Per-edge counts stay attached to the edge and to the run that produced them,
   * where [`testCountReading`] can name both.
   */
  sites: number;
  /** Every test this run named, across those sites. */
  tests: string[];
}

/**
 * Collapse the run entries scattered across many observations into the set of distinct runs.
 *
 * Keyed on `run_id`, because that is what identifies a run; the rest of the metadata is a property
 * of the run and is identical wherever it appears, so the first occurrence is taken rather than
 * merged. **Counts are deliberately not summed.** Adding one run's per-site counts together would
 * manufacture exactly the number this evidence cannot support — a total that reads as "this edge is
 * taken N times" — so what is counted here is sites, which is a fact about the observation set
 * rather than about the program.
 *
 * A run with no id is kept as a single anonymous group rather than dropped: an artifact that
 * omitted the field still observed something, and discarding it would understate the evidence.
 */
export function summariseRuns(environments: TraceEnvironment[]): TraceRunSummary[] {
  const byId = new Map<string, TraceRunSummary>();
  for (const environment of environments) {
    for (const run of environment.runs) {
      const key = run.run_id ?? '';
      const existing = byId.get(key);
      if (existing === undefined) {
        byId.set(key, { run, sites: 1, tests: [...Object.keys(run.tests)].sort() });
        continue;
      }
      existing.sites += 1;
      const tests = new Set([...existing.tests, ...Object.keys(run.tests)]);
      existing.tests = [...tests].sort();
    }
  }
  return [...byId.values()].sort((left, right) =>
    (left.run.run_id ?? '').localeCompare(right.run.run_id ?? ''),
  );
}

/**
 * The reading for a binding, spelled out where a chip cannot carry it.
 *
 * The `unverified` sentence is the one that matters: it is the absence of a check, and a reader who
 * takes it for a passed one has been told the artifact was verified against this tree when nothing
 * was compared at all.
 */
export function bindingReading(binding: string | null): string {
  switch (binding) {
    case 'bound':
      return 'The artifact names this exact tree, and every state field it declared agrees with the index.';
    case 'stale':
      return 'The artifact names a different tree from the one indexed here. It is still about this repository, but the code has moved since.';
    case 'unverified':
      return 'The artifact declared no state field, so nothing was checked. This is the absence of a check rather than a failed one — it is not stale, and it is not a pass.';
  }
  return 'This build has no description for that repository binding.';
}
