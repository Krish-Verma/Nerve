/**
 * The evidence inspector.
 *
 * This is the screen the product exists for. Every other code-graph tool will tell you that A
 * calls B; the question this one answers is *why anyone should believe that*, and the answer is
 * not a number between 0 and 1. It is a chain: an assertion, the observations that support it,
 * the extractor that made each observation and the version it was, the exact bytes it read, and
 * whether those bytes are still there.
 *
 * Three deliberate choices:
 *
 * 1. **An assertion is written as a sentence.** "Circle defines Circle.area" is what the graph
 *    says; a row in a table with columns `source`, `relation`, `target` says the same thing in a
 *    form nobody reads aloud, and reading it aloud is how you notice it is wrong.
 * 2. **Freshness is shown as an arithmetic fact, not a badge.** The server recomputes it by
 *    re-hashing, so this view shows both hashes — the one recorded when the observation was made
 *    and the one the file has right now — and lets the reader see they are the same string. A
 *    badge asks for trust; two hashes and a verdict give the reader the check itself.
 * 3. **Nothing is a raw JSON dump.** The extractor's `details` blob is rendered as labelled
 *    facts when it is shallow enough to be read that way, and only falls back to formatted text
 *    when it genuinely is a nested structure.
 */

import { useMemo, useState } from 'react';

import type { Assertion, Entity, Observation, SourceSnippet, WhyReport } from '../api/types';
import {
  count,
  directnessClass,
  directnessGloss,
  freshness as freshnessOf,
  jsonText,
  pairs,
  relationPhrase,
  shortHash,
  sourceTypeGloss,
  stamp,
  statusGloss,
} from '../format';
import { detailGloss, traceBindingGloss, traceCompletionGloss, traceSourceMapGloss } from '../vocab';
import { useApi } from '../hooks';
import { entityHref, href } from '../routing';
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
} from '../trace';
import type { TraceEnvironment } from '../api/types';
import { Chip, Empty, Failure, Loading, Panel, SelectorReading } from '../ui/parts';

export function Evidence({ id, options }: { id: string; options: Record<string, string> }) {
  const object = options['object'] ?? null;
  const params = useMemo(() => ({ subject: id, object }), [id, object]);
  const { state, reload } = useApi<WhyReport>('/api/why', params);
  const [relation, setRelation] = useState<string | null>(null);
  const [direction, setDirection] = useState<'both' | 'outgoing' | 'incoming'>('both');

  if (state.status === 'loading') return <Loading label="Gathering the evidence" />;
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;

  const report = state.data;
  const all = report.assertions;

  const relations = [...new Set(all.map((item) => item.relation))].sort();
  const shown = all.filter(
    (item) =>
      (relation === null || item.relation === relation) &&
      (direction === 'both' || item.direction === direction),
  );

  if (all.length === 0) {
    return (
      <Panel title="Evidence">
        <Empty
          title="No assertion involves this entity"
          body="Nerve recorded this entity but holds no relationship at either end of it. There is nothing to explain."
        />
      </Panel>
    );
  }

  const observations = all.reduce((total, item) => total + item.observation_count, 0);

  return (
    <div className="stack">
      <section className="panel">
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <p className="prose" style={{ fontSize: 14, color: 'var(--bone-2)' }}>
            {count(all.length)} {all.length === 1 ? 'relationship involves' : 'relationships involve'}{' '}
            this entity, held up by {count(observations)}{' '}
            {observations === 1 ? 'observation' : 'observations'}. Each observation names the
            extractor that made it, the exact source it read, and whether that source has changed
            since — measured on this request, by re-hashing the file.
          </p>

          <SelectorReading
            selectors={report.selectors}
            parameter="subject"
            chosen={report.subject}
          />

          {object === null ? null : (
            <div className="row row--wrap">
              <Chip tone="quiet">narrowed to one relationship</Chip>
              <a className="btn" href={href({ view: 'entity', id, tab: 'evidence', options: {} })}>
                Show everything about this entity
              </a>
            </div>
          )}

          <div className="row row--wrap">
            <span className="micro">direction</span>
            <div className="seg" role="group" aria-label="Filter by direction">
              {(['both', 'outgoing', 'incoming'] as const).map((value) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={direction === value}
                  onClick={() => setDirection(value)}
                >
                  {value === 'both' ? 'either way' : value === 'outgoing' ? 'from here' : 'to here'}
                </button>
              ))}
            </div>

            {relations.length > 1 ? (
              <>
                <span className="micro">relation</span>
                <button
                  type="button"
                  className={relation === null ? 'btn btn--on' : 'btn'}
                  aria-pressed={relation === null}
                  onClick={() => setRelation(null)}
                >
                  any
                </button>
                {relations.map((name) => (
                  <button
                    key={name}
                    type="button"
                    className={relation === name ? 'btn btn--on' : 'btn'}
                    aria-pressed={relation === name}
                    onClick={() => setRelation(relation === name ? null : name)}
                  >
                    {name}
                  </button>
                ))}
              </>
            ) : null}
          </div>
        </div>
      </section>

      <WatchedExecution report={report} />

      {shown.length === 0 ? (
        <Panel title="Nothing matches that filter">
          <p className="prose">
            This entity has {count(all.length)}{' '}
            {all.length === 1 ? 'relationship' : 'relationships'}, none of them in that direction
            with that relation.
          </p>
        </Panel>
      ) : (
        // Opening an observation re-reads its file to compare hashes. On a focused screen that is
        // exactly what the reader came for; on an entity with fifty relationships it would be
        // fifty file reads nobody asked for, so the spine starts closed once the list itself is
        // the information.
        shown.map((assertion) => (
          <Claim
            key={assertion.assertion_id}
            assertion={assertion}
            subjectId={id}
            expand={shown.length <= 3}
          />
        ))
      )}
    </div>
  );
}

/**
 * Trace evidence, as its own surface — and it is fed by `/api/why`, not by a route of its own.
 *
 * **There is deliberately no `/api/trace`.** Import is a write path and is CLI-only; a trace
 * observation is an observation like any other, and the evidence endpoints are generic over the
 * evidence model, so the data is already here. Adding a read route would have created a second way
 * to ask one question, with a second chance to answer it differently.
 *
 * What this panel exists to prevent is a specific misreading. `TEST_OBSERVED_CALL` is **existential**
 * evidence: one run took this edge. It is not a `CALLS` edge, it must never be drawn as one, and
 * the neighbouring precedent is ADR-0005 — coverage is not a call graph, for the same reason.
 *
 * The empty case is the one that matters most, and it is why this panel renders even when there is
 * nothing to show. A repository nobody has traced has no trace evidence anywhere; drawing that the
 * way "no callers" is drawn would turn missing instrumentation into an apparent fact about the
 * code. So the absence is named as an absence of observation.
 */
function WatchedExecution({ report }: { report: WhyReport }) {
  const environments = useMemo(() => {
    const found: TraceEnvironment[] = [];
    for (const assertion of report.assertions) {
      for (const observation of assertion.observations) {
        if (!isTraceObservation(observation)) continue;
        const parsed = parseTraceEnvironment(observation.environment);
        if (parsed !== null) found.push(parsed);
      }
    }
    return found;
  }, [report.assertions]);

  const runs = useMemo(() => summariseRuns(environments), [environments]);

  if (environments.length === 0) {
    return (
      <Panel title="Watched execution" aside={<Chip tone="unknown">nothing observed</Chip>}>
        <div className="stack" style={{ gap: 10 }}>
          <p className="prose">{NOT_OBSERVED_STATEMENT}</p>
          <p className="hash wrapany">{EXISTENTIAL_STATEMENT}</p>
          <p className="hash wrapany">
            Nerve runs no tests. A trace arrives from a tracer you ran yourself, imported with{' '}
            <code>nerve trace import</code>.
          </p>
        </div>
      </Panel>
    );
  }

  return (
    <Panel
      title="Watched execution"
      aside={
        <Chip tone="quiet">
          {count(runs.length)} {runs.length === 1 ? 'run' : 'runs'}
        </Chip>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <p className="prose">{EXISTENTIAL_STATEMENT}</p>
        <p className="hash wrapany">
          These edges are recorded as <code>TEST_OBSERVED_CALL</code> and are kept apart from{' '}
          <code>CALLS</code> throughout. A call Nerve read out of the source and a call a tracer
          watched happen are different claims, and neither one implies the other.
        </p>

        {runs.map((summary) => (
          <div key={summary.run.run_id ?? 'unnamed'} className="stack" style={{ gap: 8 }}>
            <div className="row row--wrap">
              <span className="fact__value fact__value--strong">
                {summary.run.run_id ?? 'a run that declared no id'}
              </span>
              <Chip
                tone={completionTone(summary.run.completion_state)}
                title={traceCompletionGloss(summary.run.completion_state ?? '')}
              >
                {summary.run.completion_state ?? 'completion not declared'}
              </Chip>
              <Chip
                tone={bindingTone(summary.run.repository_binding)}
                title={traceBindingGloss(summary.run.repository_binding ?? '')}
              >
                {summary.run.repository_binding ?? 'binding not declared'}
              </Chip>
              <Chip
                tone={sourceMapTone(summary.run.source_map_state)}
                title={traceSourceMapGloss(summary.run.source_map_state ?? '')}
              >
                source map: {summary.run.source_map_state ?? 'not declared'}
              </Chip>
            </div>

            <p className="prose">{bindingReading(summary.run.repository_binding)}</p>

            {summary.run.partial_reason === null ? null : (
              <p className="hash wrapany">
                It stopped early, and said why: {summary.run.partial_reason}
              </p>
            )}

            <div className="row row--wrap">
              <span className="hash">
                {summary.run.producer ?? 'an unnamed producer'}
                {summary.run.producer_version === null ? '' : ` ${summary.run.producer_version}`}
              </span>
              {summary.run.test_framework === null ? null : (
                <span className="hash">framework: {summary.run.test_framework}</span>
              )}
              {summary.run.runtime === null ? null : (
                <span className="hash">
                  runtime: {summary.run.runtime}
                  {summary.run.runtime_version === null ? '' : ` ${summary.run.runtime_version}`}
                </span>
              )}
              {summary.run.platform === null ? null : (
                <span className="hash">platform: {summary.run.platform}</span>
              )}
              <span className="hash">{stamp(summary.run.started_at)}</span>
            </div>

            {/*
              Sites, not calls. A run that took one edge a thousand times counts once here, and
              that is the honest number: per-edge counts belong beside the run that produced them,
              which is where the observation below prints them.
            */}
            <span className="hash">
              named by {count(summary.sites)} observed{' '}
              {summary.sites === 1 ? 'site' : 'sites'} on this screen — a count of sites, not of
              calls
            </span>

            {summary.tests.length === 0 ? null : (
              <div className="row row--wrap">
                <span className="micro">tests</span>
                {summary.tests.map((test) => (
                  <Chip key={test} tone="quiet">
                    {test}
                  </Chip>
                ))}
              </div>
            )}

            {summary.run.producer_limitations.length === 0 ? null : (
              <div className="stack" style={{ gap: 4 }}>
                <div className="micro">the producer said it could not see</div>
                {summary.run.producer_limitations.map((limitation) => (
                  <p key={limitation} className="hash wrapany">
                    {limitation}
                  </p>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </Panel>
  );
}

/**
 * The run set behind one trace observation, rendered where its raw JSON used to be printed.
 *
 * `environment` is a text column holding a JSON document, and this screen used to render it as the
 * string it is — a wall of braces in a field labelled "environment". Everything the handoff note
 * calls load-bearing was in there and none of it was legible.
 *
 * A **list**, always, because two tests reaching one callee from one line are one observation
 * naming both: `idx_observation_identity` has no column that could hold a second row per test. And
 * the derived scalars are printed from the object rather than recomputed from `runs[0]`, because
 * they are the weakest value across contributing runs — a site observed by one complete run and one
 * interrupted run reads as `partial`, and reading the first run would report it as complete.
 */
function TraceEnvironmentFacts({ environment }: { environment: TraceEnvironment }) {
  const tests = testsNamed(environment);
  return (
    <div className="stack" style={{ gap: 10 }}>
      <div className="row row--wrap">
        <Chip
          tone={completionTone(environment.completion_state)}
          title={traceCompletionGloss(environment.completion_state ?? '')}
        >
          {environment.completion_state ?? 'completion not declared'}
        </Chip>
        <Chip
          tone={bindingTone(environment.repository_binding)}
          title={traceBindingGloss(environment.repository_binding ?? '')}
        >
          {environment.repository_binding ?? 'binding not declared'}
        </Chip>
        <span className="hash">weakest value across every contributing run</span>
      </div>

      <p className="prose">{runSetReading(environment)}</p>

      {environment.runs.map((run, index) => (
        <div key={`${run.run_id ?? 'unnamed'}:${index}`} className="stack" style={{ gap: 4 }}>
          <span className="fact__value">{run.run_id ?? 'a run that declared no id'}</span>
          {Object.keys(run.tests).length === 0 ? (
            <span className="hash">This run named no test at this site.</span>
          ) : (
            <div className="row row--wrap">
              {Object.keys(run.tests)
                .sort()
                .map((test) => (
                  <Chip key={test} tone="quiet" prose title={testCountReading(run, test)}>
                    {test} — {testCountReading(run, test)}
                  </Chip>
                ))}
            </div>
          )}
        </div>
      ))}

      {tests.length === 0 ? null : (
        <span className="hash wrapany">
          A count is how many times that one run took this edge. It is not a frequency, and two
          runs&apos; counts are recorded per run rather than added together.
        </span>
      )}
    </div>
  );
}

/** One party in the sentence. The subject is marked so the reader can see whose side they are on. */
function Party({ entity, subject }: { entity: Entity; subject: boolean }) {
  const label = entity.qualified_name || entity.name;
  const classes = [
    'claim__party',
    subject ? 'claim__party--subject' : '',
    entity.kind === 'unresolved' ? 'claim__party--unknown' : '',
  ]
    .filter(Boolean)
    .join(' ');
  if (subject) {
    return (
      <span className={classes} title={`${entity.kind} · ${label}`}>
        {label}
      </span>
    );
  }
  return (
    <a className={classes} href={entityHref(entity.entity_id, 'evidence')} title={`${entity.kind} · ${label}`}>
      {label}
    </a>
  );
}

function Claim({
  assertion,
  subjectId,
  expand,
}: {
  assertion: Assertion;
  subjectId: string;
  expand: boolean;
}) {
  const outgoing = assertion.source.entity_id === subjectId;
  return (
    <section className={assertion.is_unresolved ? 'claim claim--unresolved' : 'claim'}>
      <header className="claim__head">
        <div className="claim__sentence">
          <Party entity={assertion.source} subject={assertion.source.entity_id === subjectId} />
          <span className="claim__verb">{relationPhrase(assertion.relation, true)}</span>
          <Party entity={assertion.target} subject={assertion.target.entity_id === subjectId} />
        </div>
        <div className="row row--wrap">
          <Chip
            tone={assertion.status === 'SUPPORTED' ? 'fresh' : 'stale'}
            title={statusGloss(assertion.status)}
          >
            {assertion.status.toLowerCase()}
          </Chip>
          {assertion.is_unresolved ? (
            <Chip
              tone="unknown"
              title="One end of this relationship is a reference Nerve could not connect to a declaration."
            >
              one end unresolved
            </Chip>
          ) : null}
          <Chip tone="quiet">
            {count(assertion.observation_count)}{' '}
            {assertion.observation_count === 1 ? 'observation' : 'observations'}
          </Chip>
          {/*
            An existential edge is labelled on the claim itself, not only in the panel above. The
            sentence one line up reads "A was observed calling B", which is already hedged — but a
            reader skimming a list of claims needs the qualification attached to the row rather
            than inferred from a verb, because the row beside it may be a static `CALLS` and the
            two must never be read as the same strength of statement.
          */}
          {assertion.relation === 'TEST_OBSERVED_CALL' ? (
            <Chip tone="unknown" prose title={EXISTENTIAL_STATEMENT}>
              one run took this edge — not that every run does
            </Chip>
          ) : null}
          <span className="spacer" />
          <span className="hash">
            {outgoing ? 'recorded from this entity outwards' : 'recorded towards this entity'}
          </span>
        </div>
      </header>
      <div className="claim__body">
        <div className="spine">
          {assertion.observations.map((observation) => (
            <Observed
              key={observation.observation_id}
              observation={observation}
              alone={expand && assertion.observations.length === 1}
            />
          ))}
        </div>
      </div>
    </section>
  );
}

/**
 * One observation, and everything known about how it came to exist.
 *
 * A single observation is opened by default — hiding the only thing on the page behind a control
 * is a disclosure pattern applied where there is nothing to progress from. Where several stack up
 * they start closed, because then the list itself is the information.
 */
function Observed({ observation, alone }: { observation: Observation; alone: boolean }) {
  const [open, setOpen] = useState(alone);
  const reading = freshnessOf(observation.freshness);
  const detail = pairs(observation.details);
  // Null for every observation that is not a watched execution, and also for a trace observation
  // whose environment could not be parsed — the source type is what separates those two, and the
  // second is left to fall through to the raw field rather than be rendered as an empty run set.
  const traced = isTraceObservation(observation)
    ? parseTraceEnvironment(observation.environment)
    : null;

  return (
    <div className={`obs ${directnessClass(observation.directness)}`}>
      <div className="obs__line">
        <span className="obs__source">
          {observation.extractor_id}
          <span className="hash">@{observation.extractor_version}</span>
        </span>
        <Chip tone="quiet" title={sourceTypeGloss(observation.evidence_source_type)}>
          {observation.evidence_source_type}
        </Chip>
        <Chip tone={reading.tone} title={reading.gloss}>
          <span className="chip__dot" />
          {reading.label}
        </Chip>
        <span className="spacer" />
        <button
          type="button"
          className="btn btn--ghost"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? 'less' : 'how it was read'}
        </button>
      </div>

      <div className="obs__line">
        <span className="fact__value">{sourceTypeGloss(observation.evidence_source_type)}</span>
      </div>

      <div className="obs__line">
        <span className="hash wrapany">
          {observation.file_path ?? 'no file recorded'}
          {observation.start_line === null ? '' : `:${observation.start_line}`}
          {observation.end_line === null || observation.end_line === observation.start_line
            ? ''
            : `–${observation.end_line}`}
        </span>
      </div>

      {open ? (
        <div className="stack reveal" style={{ gap: 12 }}>
          <div className="obs__facts">
            <Fact label="directness" value={observation.directness} note={directnessGloss(observation.directness)} />
            <Fact
              label="extractor"
              value={`${observation.extractor_id} ${observation.extractor_version}`}
              note="The code that made this observation, and the version it was. A different version is a different witness."
            />
            <Fact
              label="match quality"
              value={observation.match_quality ?? 'not applicable'}
              note={
                observation.match_quality === null
                  ? 'This kind of evidence is not matched by name, so there is no match quality to record.'
                  : 'How the name was matched to a declaration.'
              }
            />
            {/*
              A trace observation's `environment` is a JSON document rather than a word, and it is
              rendered as the run set it is — see `TraceEnvironmentFacts` below the fold. Printing
              it here as the raw string would put the whole of the run provenance on screen in a
              form nobody can read, which is what this field did before.
            */}
            {traced === null ? (
              <Fact
                label="environment"
                value={observation.environment ?? 'none recorded'}
                note="The environment the observation was made in, where that changes what is true."
              />
            ) : null}
            <Fact label="recorded" value={stamp(observation.created_at)} note="When the extractor ran." />
            <Fact
              label="repository state"
              value={shortHash(observation.state_id, 12)}
              note="The merkle of the whole indexed tree at the moment this was recorded."
            />
          </div>

          <FileCheck observation={observation} />

          {traced === null ? null : (
            <div>
              <div className="micro" style={{ marginBottom: 6 }}>
                the runs that observed this, and the tests they were running
              </div>
              <TraceEnvironmentFacts environment={traced} />
            </div>
          )}

          {detail === null ? null : (
            <div>
              <div className="micro" style={{ marginBottom: 6 }}>
                what the extractor recorded
              </div>
              {/*
                A few of these keys carry a closed vocabulary — a reason code, an ADR status —
                and printed bare they are the raw identifiers the rest of the interface exists to
                translate. Where a reading exists it is shown under the value; where none does,
                the value stands alone rather than being given an invented sentence.
              */}
              <div className="obs__facts">
                {detail.map(([key, value]) => {
                  const reading = detailGloss(key, value);
                  return (
                    <div className="fact" key={key}>
                      <span className="fact__key">{key.replace(/_/g, ' ')}</span>
                      <span className="fact__value fact__value--strong wrapany">{value}</span>
                      {reading === undefined ? null : (
                        <span className="fact__note">{reading}</span>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
          {detail === null && observation.details !== null ? (
            <pre className="details">{jsonText(observation.details)}</pre>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Fact({ label, value, note }: { label: string; value: string; note: string }) {
  return (
    <div className="fact" title={note}>
      <span className="fact__key">{label}</span>
      <span className="fact__value fact__value--strong wrapany">{value}</span>
    </div>
  );
}

/**
 * The freshness check, shown as the comparison it actually is.
 *
 * The server never stores a "stale" flag — it re-reads the file and re-hashes it on every
 * request, which catches a change that was made and never indexed. That is a strictly stronger
 * claim than a stored flag, and it is invisible unless the two hashes are put side by side.
 */
function FileCheck({ observation }: { observation: Observation }) {
  const path = observation.file_path;
  const from = observation.start_line ?? 1;
  const to = observation.end_line ?? from;
  const params = useMemo(
    () => ({ path: path ?? '', start_line: Math.max(1, from - 2), end_line: to + 2 }),
    [path, from, to],
  );
  const { state } = useApi<SourceSnippet>(path === null ? null : '/api/source', params);

  if (path === null) {
    return (
      <p className="prose">
        This observation records no file, so there is nothing to re-read and freshness cannot be
        measured.
      </p>
    );
  }

  const recorded = observation.content_hash;

  return (
    <div className="stack" style={{ gap: 10 }}>
      <div className="micro">is the source still what was read?</div>

      {state.status === 'loading' ? (
        <p className="hash">re-reading {path}…</p>
      ) : state.status === 'error' ? (
        <div className="check">
          <p className="prose">
            The file could not be read back, so the two hashes cannot be compared. The server
            reported this as <span className="hash">{freshnessOf(observation.freshness).label}</span>.
          </p>
          <p className="prose">{freshnessOf(observation.freshness).gloss}</p>
        </div>
      ) : (
        <>
          <div className="check">
            <div className="check__row">
              <span className="fact__key">recorded</span>
              <span className="check__hash wrapany" title={recorded ?? ''}>
                {recorded ?? 'no hash recorded'}
              </span>
            </div>
            <div className="check__row">
              <span className="fact__key">on disk now</span>
              <span className="check__hash wrapany" title={state.data.content_hash}>
                {state.data.content_hash}
              </span>
            </div>
            <div
              className={
                recorded === state.data.content_hash
                  ? 'check__verdict check__verdict--same'
                  : 'check__verdict check__verdict--differs'
              }
            >
              {recorded === state.data.content_hash
                ? `identical — ${path} still holds exactly the bytes this was extracted from`
                : `different — ${path} has changed since this was recorded, so this observation may no longer describe it`}
            </div>
          </div>

          <Snippet snippet={state.data} from={from} to={to} />
        </>
      )}
    </div>
  );
}

/** The lines the observation was read from, with the observed range marked. */
function Snippet({ snippet, from, to }: { snippet: SourceSnippet; from: number; to: number }) {
  const lines = snippet.text.split('\n');
  return (
    <pre className="code" aria-label={`${snippet.path} lines ${snippet.start_line} to ${snippet.end_line}`}>
      {lines.map((line, index) => {
        const number = snippet.start_line + index;
        const marked = number >= from && number <= to;
        return (
          <div className={marked ? 'code__line code__line--marked' : 'code__line'} key={number}>
            <span className="code__no num">{number}</span>
            <span className="code__text">{line === '' ? ' ' : line}</span>
          </div>
        );
      })}
    </pre>
  );
}
