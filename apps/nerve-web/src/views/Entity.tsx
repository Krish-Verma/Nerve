/**
 * One entity: what it is, where it is, what it connects to, and why any of that is believed.
 *
 * The four tabs are four different questions, not four slices of one table:
 *
 *   relations — what is this connected to
 *   evidence  — why does Nerve believe any of it
 *   graph     — what does the neighbourhood look like
 *   source    — what does the file actually say
 *
 * The header is the only thing shared between them, and it is deliberately generous: a qualified
 * name is the single most important string on the screen and it comes out of the repository
 * verbatim, so it is set in the literal voice and given room to be long.
 */

import { useMemo } from 'react';

import type { EntityDetail, Neighbourhood } from '../api/types';
import { count, relationPhrase } from '../format';
import { useApi } from '../hooks';
import { ENTITY_TABS, href, type EntityTab } from '../routing';
import { Chip, Empty, EntityLink, Failure, Loading, Panel, SelectorReading, Where } from '../ui/parts';
import { kindGloss } from '../vocab';
import { Evidence } from './Evidence';
import { Graph } from './Graph';
import { Source } from './Source';

const TAB_LABEL: Record<EntityTab, string> = {
  relations: 'Relations',
  evidence: 'Evidence',
  graph: 'Neighbourhood',
  source: 'Source',
};

export function Entity({
  id,
  tab,
  options,
}: {
  id: string;
  tab: EntityTab;
  options: Record<string, string>;
}) {
  const params = useMemo(() => ({ selector: id }), [id]);
  const { state, reload } = useApi<EntityDetail>('/api/entity', params);

  if (state.status === 'loading') return <Loading label="Reading this entity" />;
  if (state.status === 'error') {
    return (
      <div className="view">
        <Failure error={state.error} onRetry={reload} />
      </div>
    );
  }

  const detail = state.data;
  const entity = detail.entity;
  const label = entity.qualified_name || entity.name;
  const outgoing = Object.values(detail.relation_counts.outgoing).reduce((a, b) => a + b, 0);
  const incoming = Object.values(detail.relation_counts.incoming).reduce((a, b) => a + b, 0);

  const tabCount: Record<EntityTab, number | undefined> = {
    relations: outgoing + incoming,
    evidence: undefined,
    graph: undefined,
    source: detail.occurrence_count,
  };

  return (
    <div className={tab === 'graph' ? 'view view--wide' : 'view'}>
      <div className="head">
        <div className="row row--wrap">
          <Chip tone={entity.kind === 'unresolved' ? 'unknown' : 'quiet'} title={kindGloss(entity.kind)}>
            {entity.kind}
          </Chip>
          {entity.language ? <Chip tone="quiet">{entity.language}</Chip> : null}
          <Where path={entity.file_path} line={entity.start_line} />
        </div>
        <h1 className={label.length > 48 ? 'head__title head__title--long wrapany' : 'head__title wrapany'}>
          {label}
        </h1>
        {entity.kind === 'unresolved' ? (
          <p className="head__sub">{kindGloss('unresolved')}</p>
        ) : entity.scope_path ? (
          <p className="head__sub wrapany">
            declared inside <span className="hash">{entity.scope_path}</span>
          </p>
        ) : null}
        {/*
          Usually renders nothing: this screen is normally reached with an entity id, which names
          exactly one thing. It matters when a path was typed — `src/app.ts` is both a module and a
          file, and without this the reader is shown one of them with no sign the other exists.
        */}
        <SelectorReading selectors={detail.selectors} parameter="selector" chosen={entity} />
      </div>

      <nav className="tabs" aria-label="Views of this entity">
        {ENTITY_TABS.map((name) => (
          <a
            key={name}
            className="tab"
            href={href({ view: 'entity', id, tab: name, options: {} })}
            aria-current={name === tab ? 'page' : undefined}
          >
            {TAB_LABEL[name]}
            {tabCount[name] === undefined ? null : (
              <span className="tab__count num">{count(tabCount[name])}</span>
            )}
          </a>
        ))}
      </nav>

      {tab === 'relations' ? (
        <Relations id={id} detail={detail} />
      ) : tab === 'evidence' ? (
        <Evidence id={id} options={options} />
      ) : tab === 'graph' ? (
        <Graph id={id} options={options} />
      ) : (
        <Source detail={detail} />
      )}
    </div>
  );
}

/**
 * What this is connected to, read as sentences from this entity's side.
 *
 * The counts come from the entity endpoint; the actual other ends come from a depth-1
 * neighbourhood, which is the same bounded query the graph tab uses. Asking a second, unbounded
 * question here would let one popular entity produce an unbounded screen.
 */
function Relations({ id, detail }: { id: string; detail: EntityDetail }) {
  const params = useMemo(() => ({ selector: id, depth: 1, max_nodes: 200 }), [id]);
  const { state, reload } = useApi<Neighbourhood>('/api/neighbourhood', params);

  const totals = { ...detail.relation_counts.outgoing };
  for (const [relation, value] of Object.entries(detail.relation_counts.incoming)) {
    totals[relation] = (totals[relation] ?? 0) + value;
  }
  const anything = Object.values(totals).some((value) => value > 0);

  if (!anything) {
    return (
      <Panel title="Relations">
        <Empty
          title="Nothing connects to this"
          body="No assertion in the index has this entity at either end. That can mean it is genuinely isolated, or that the file it lives in was only partly parsed — check Unresolved."
        />
      </Panel>
    );
  }

  if (state.status === 'loading') return <Loading label="Reading one hop out" />;
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;

  const hood = state.data;
  const entities = new Map(hood.nodes.map((node) => [node.entity.entity_id, node.entity]));

  // Grouped by relation, then by which way it points relative to the entity being read, so the
  // heading can be a verb — "calls" and "is called by" are different questions.
  const groups = new Map<string, { outgoing: boolean; edges: typeof hood.edges }>();
  for (const edge of hood.edges) {
    const outgoing = edge.source_entity_id === id;
    if (!outgoing && edge.target_entity_id !== id) continue;
    const key = `${edge.relation}:${outgoing ? 'out' : 'in'}`;
    const bucket = groups.get(key);
    if (bucket) bucket.edges.push(edge);
    else groups.set(key, { outgoing, edges: [edge] });
  }

  const ordered = [...groups.entries()].sort((a, b) => {
    const left = a[0].split(':')[0] ?? '';
    const right = b[0].split(':')[0] ?? '';
    return left < right ? -1 : left > right ? 1 : a[0] < b[0] ? -1 : 1;
  });

  return (
    <div className="stack">
      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">Relations, one hop</h2>
          <span className="hash">
            {count(hood.edge_count)} {hood.edge_count === 1 ? 'assertion' : 'assertions'}
          </span>
        </header>
        <div className="panel__body panel__body--flush">
          {ordered.map(([key, group]) => {
            const relation = key.split(':')[0] ?? '';
            return (
              <div className="relgroup" key={key}>
                <div className="relgroup__head">
                  <span className="relgroup__verb">{relationPhrase(relation, group.outgoing)}</span>
                  <span className="relgroup__gloss">
                    {count(group.edges.length)}{' '}
                    {group.edges.length === 1 ? 'assertion' : 'assertions'}
                  </span>
                </div>
                <ul className="hits">
                  {group.edges.map((edge) => {
                    const otherId = group.outgoing ? edge.target_entity_id : edge.source_entity_id;
                    const other = entities.get(otherId);
                    return (
                      <li key={edge.assertion_id}>
                        <a
                          className={edge.is_unresolved ? 'hit hit--unresolved' : 'hit'}
                          href={href({
                            view: 'entity',
                            id,
                            tab: 'evidence',
                            options: { object: otherId, relation },
                          })}
                          title="Open the evidence for this one assertion"
                        >
                          <span className="hit__kind">{other?.kind ?? 'unknown'}</span>
                          <span className="hit__name truncate">
                            {other ? other.qualified_name || other.name : otherId}
                          </span>
                          <span className="hit__where">
                            {edge.observation_count}{' '}
                            {edge.observation_count === 1 ? 'observation' : 'observations'} ·{' '}
                            {edge.strongest_source_type}
                          </span>
                        </a>
                      </li>
                    );
                  })}
                </ul>
              </div>
            );
          })}
        </div>
      </section>

      {hood.truncated ? (
        <Panel title="Not everything is shown">
          <p className="prose">
            {count(hood.omitted_nodes)} more entities are connected to this one and were left out to
            keep the query bounded. Open the neighbourhood to raise the budget.
          </p>
        </Panel>
      ) : null}

      <Panel title="Defined by" aside={<span className="hash">structure</span>}>
        {detail.defining_edges.edges.length === 0 ? (
          <p className="prose">Nothing in the index declares this entity.</p>
        ) : (
          <ul className="hits">
            {detail.defining_edges.edges.map((edge) => {
              const other = detail.defining_edges.nodes.find(
                (node) =>
                  node.entity.entity_id ===
                  (edge.source_entity_id === detail.entity.entity_id
                    ? edge.target_entity_id
                    : edge.source_entity_id),
              );
              if (!other) return null;
              return (
                <li key={edge.assertion_id}>
                  <span className="hit">
                    <span className="hit__kind">{edge.relation}</span>
                    <span className="hit__name truncate">
                      <EntityLink entity={other.entity} />
                    </span>
                    <span className="hit__where">
                      {edge.file_path ?? '—'}
                      {edge.start_line === null ? '' : `:${edge.start_line}`}
                    </span>
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </Panel>
    </div>
  );
}
