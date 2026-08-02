/**
 * How two entities are connected.
 *
 * A path is a sequence, so it is drawn as one — a chain read left to right, each link labelled
 * with the relation that was followed and marked when it was followed *backwards*. That last
 * detail matters and is usually thrown away: "A is reachable from B" is a much weaker statement
 * than "A calls B", and a reader who cannot tell them apart will over-read the picture.
 *
 * The search is bounded and says so. "No path" from a bounded search means "none within this many
 * hops", never "none exists", and the empty state is worded to keep that distinction.
 */

import { useEffect, useMemo, useState } from 'react';

import type { FoundPath, PathReport, SearchResponse } from '../api/types';
import { count, relationPhrase } from '../format';
import { useApi, useDebounced } from '../hooks';
import { entityHref, href, replaceRoute } from '../routing';
import { Chip, Empty, EntityLink, Failure, Loading, Panel, Where } from '../ui/parts';

const DEPTHS = [3, 6, 10, 16];

export function PathFinder({ id, options }: { id: string; options: Record<string, string> }) {
  const target = options['to'] ?? '';
  const maxDepth = Number(options['max_depth'] ?? 6) || 6;
  const [text, setText] = useState(target);

  useEffect(() => {
    setText(target);
  }, [target]);

  const settled = useDebounced(text.trim(), 220);
  useEffect(() => {
    if (settled !== target) {
      replaceRoute(
        href({ view: 'entity', id, tab: 'graph', options: { ...options, to: settled } }),
      );
    }
    // The route is rewritten only when the settled text differs; `options` changing on every
    // render must not retrigger it, so it is deliberately not a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settled, target, id]);

  const set = (next: Record<string, string>) =>
    replaceRoute(href({ view: 'entity', id, tab: 'graph', options: { ...options, ...next } }));

  // The picker searches, so the reader can find the other end by name instead of pasting an id.
  const suggestParams = useMemo(() => ({ q: settled, limit: 6 }), [settled]);
  const suggestions = useApi<SearchResponse>(
    settled.length > 0 ? '/api/search' : null,
    suggestParams,
  );

  const params = useMemo(
    () => ({ from: id, to: settled, max_depth: maxDepth, limit: 5 }),
    [id, settled, maxDepth],
  );
  const { state, reload } = useApi<PathReport>(settled.length > 0 ? '/api/path' : null, params);

  // The picker is only useful when the selector did not resolve — an ambiguous name, or one that
  // matches nothing. Once the server has accepted it, leaving a list of alternatives on screen is
  // noise above the answer the reader asked for.
  const needsPicking = state.status === 'error';

  return (
    <>
      <section className="panel">
        <div className="graph__controls">
          <span className="micro">to</span>
          <div className="field" style={{ flex: '1 1 280px' }}>
            <input
              type="text"
              value={text}
              spellCheck={false}
              autoComplete="off"
              aria-label="The other end of the path"
              placeholder="a name, a qualified name, or an entity id"
              onChange={(event) => setText(event.target.value)}
            />
          </div>
          <span className="micro">within</span>
          <div className="seg" role="group" aria-label="Largest number of hops to search">
            {DEPTHS.map((value) => (
              <button
                key={value}
                type="button"
                aria-pressed={maxDepth === value}
                onClick={() => set({ max_depth: String(value) })}
              >
                {value} hops
              </button>
            ))}
          </div>
        </div>

        {settled.length > 0 && needsPicking && suggestions.state.status === 'ready' &&
        suggestions.state.data.results.length > 0 ? (
          <div className="panel__body panel__body--flush">
            <ul className="hits">
              {suggestions.state.data.results.map((hit) => (
                <li key={hit.entity_id}>
                  <button
                    type="button"
                    className="hit"
                    onClick={() => setText(hit.entity_id)}
                  >
                    <span className="hit__kind">{hit.kind}</span>
                    <span className="hit__name truncate">
                      {hit.scope_path ? <span className="hit__scope">{hit.scope_path}.</span> : null}
                      {hit.name}
                    </span>
                    <span className="hit__where">pick this one</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </section>

      {settled.length === 0 ? (
        <Panel title="Path">
          <Empty
            title="Name the other end"
            body="Type what you want to reach. Nerve walks assertions outwards from this entity and reports every distinct route it finds within the hop limit — including routes that had to be followed against the direction the assertion was recorded in."
          />
        </Panel>
      ) : state.status === 'loading' ? (
        <Loading label="Walking the graph" />
      ) : state.status === 'error' ? (
        <Failure error={state.error} onRetry={reload} />
      ) : (
        <Result report={state.data} maxDepth={maxDepth} />
      )}
    </>
  );
}

function Result({ report, maxDepth }: { report: PathReport; maxDepth: number }) {
  if (report.count === 0) {
    return (
      <Panel title="No route found">
        <div className="stack" style={{ gap: 12 }}>
          <p className="prose">
            Nothing connects <EntityLink entity={report.from} /> to{' '}
            <EntityLink entity={report.to} /> within {count(maxDepth)} hops. That is a statement
            about this search, not about the repository: a longer route may exist, and a route that
            depends on a reference Nerve could not resolve will never be found at all.
          </p>
          <div className="row row--wrap">
            <Chip tone="quiet">
              {count(report.expansions)}{' '}
              {report.expansions === 1 ? 'entity expanded' : 'entities expanded'}
            </Chip>
            {report.truncated ? (
              <Chip tone="absent" title="The search stopped at its ceiling before it ran out of graph.">
                search hit its ceiling
              </Chip>
            ) : null}
          </div>
          <p className="prose">
            Check <a className="link" href={href({ view: 'unresolved' })}>Unresolved</a> for references that were
            recorded but never connected — a broken chain there is the usual reason a route is
            missing.
          </p>
        </div>
      </Panel>
    );
  }

  return (
    <div className="stack">
      <section className="panel">
        <div className="panel__body">
          <p className="prose" style={{ fontSize: 14, color: 'var(--bone-2)' }}>
            {count(report.count)} {report.count === 1 ? 'route' : 'routes'} from{' '}
            <EntityLink entity={report.from} /> to <EntityLink entity={report.to} />, shortest
            first. {report.truncated ? 'The search stopped at its ceiling, so there may be more. ' : ''}
            {count(report.expansions)}{' '}
            {report.expansions === 1
              ? 'entity was expanded to find it.'
              : 'entities were expanded to find them.'}
          </p>
        </div>
      </section>

      {report.paths.map((path, index) => (
        <Route key={`${index}-${path.hops.map((hop) => hop.assertion_id).join('-')}`} path={path} />
      ))}
    </div>
  );
}

function Route({ path }: { path: FoundPath }) {
  return (
    <section className={path.traverses_unresolved ? 'claim claim--unresolved' : 'claim'}>
      <header className="claim__head">
        <div className="row row--wrap">
          <Chip tone="quiet">
            {count(path.length)} {path.length === 1 ? 'hop' : 'hops'}
          </Chip>
          {path.traverses_unresolved ? (
            <Chip
              tone="unknown"
              title="At least one step of this route passes through a reference Nerve could not connect to a declaration."
            >
              passes through an unresolved reference
            </Chip>
          ) : null}
          {path.hops.some((hop) => hop.traversed_backwards) ? (
            <Chip
              tone="absent"
              title="At least one step was followed against the direction the assertion was recorded in."
            >
              not all one way
            </Chip>
          ) : null}
        </div>
      </header>
      <div className="claim__body">
        <ol className="chain">
          {path.hops.map((hop, index) => (
            <li className="chain__step" key={`${hop.assertion_id}-${index}`}>
              <div className="chain__from wrapany">
                <EntityLink entity={hop.from} />
              </div>
              <div className="chain__link">
                <span className={hop.traversed_backwards ? 'chain__arrow chain__arrow--back' : 'chain__arrow'}>
                  {hop.traversed_backwards ? '←' : '→'}
                </span>
                <span className="claim__verb">
                  {hop.traversed_backwards
                    ? relationPhrase(hop.relation, false)
                    : relationPhrase(hop.relation, true)}
                </span>
                <a
                  className="hash"
                  href={href({
                    view: 'entity',
                    id: hop.from.entity_id,
                    tab: 'evidence',
                    options: { object: hop.to.entity_id },
                  })}
                >
                  {count(hop.observation_count)}{' '}
                  {hop.observation_count === 1 ? 'observation' : 'observations'} · why?
                </a>
                {hop.is_unresolved ? <Chip tone="unknown">unresolved</Chip> : null}
                <Where path={hop.file_path} line={hop.start_line} />
              </div>
              {index === path.hops.length - 1 ? (
                <div className="chain__from wrapany">
                  <a className="link" href={entityHref(hop.to.entity_id, 'evidence')}>
                    {hop.to.qualified_name || hop.to.name}
                  </a>
                </div>
              ) : null}
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}
