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
import { useApi } from '../hooks';
import { entityHref, href } from '../routing';
import { Chip, Empty, Failure, Loading, Panel } from '../ui/parts';

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
            <Fact
              label="environment"
              value={observation.environment ?? 'none recorded'}
              note="The environment the observation was made in, where that changes what is true."
            />
            <Fact label="recorded" value={stamp(observation.created_at)} note="When the extractor ran." />
            <Fact
              label="repository state"
              value={shortHash(observation.state_id, 12)}
              note="The merkle of the whole indexed tree at the moment this was recorded."
            />
          </div>

          <FileCheck observation={observation} />

          {detail === null ? null : (
            <div>
              <div className="micro" style={{ marginBottom: 6 }}>
                what the extractor recorded
              </div>
              <div className="obs__facts">
                {detail.map(([key, value]) => (
                  <div className="fact" key={key}>
                    <span className="fact__key">{key.replace(/_/g, ' ')}</span>
                    <span className="fact__value fact__value--strong wrapany">{value}</span>
                  </div>
                ))}
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
