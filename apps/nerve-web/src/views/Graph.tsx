/**
 * The neighbourhood, and the path between two entities.
 *
 * This view is **never** "the repository". It is always a bounded expansion around one focused
 * entity, and the bound is part of the picture rather than a detail in a tooltip: the outer ring
 * is the edge of the query, and when the node budget cut the answer short a dashed arc sits
 * outside it saying how many entities are beyond. A graph that quietly drops nodes is worse than
 * no graph, because it looks like an answer.
 *
 * Distance from the centre is depth and nothing else — see `../graph/layout.ts` for why there is
 * no force simulation here. The layout is pure, so the same neighbourhood always draws the same
 * way and a screenshot of it means something.
 */

import { useMemo, useState } from 'react';

import type { Entity, NeighbourEdge, Neighbourhood } from '../api/types';
import { RELATIONS } from '../api/types';
import { count, elide, relationPhrase } from '../format';
import {
  edgePath,
  labelPlacement,
  layout,
  ringLabelPlacement,
  type LayoutEdge,
  type LayoutNode,
} from '../graph/layout';
import { useApi } from '../hooks';
import { entityHref, href, replaceRoute } from '../routing';
import { Chip, Empty, EntityLink, Failure, Loading, Panel, SelectorReading, Where } from '../ui/parts';
import { PathFinder } from './Path';

const WIDTH = 980;
const HEIGHT = 640;
const DEPTHS = [1, 2, 3, 4];
const BUDGETS = [25, 60, 150, 500];

type Selected = { kind: 'node'; id: string } | { kind: 'edge'; id: string } | null;

export function Graph({ id, options }: { id: string; options: Record<string, string> }) {
  const mode = options['to'] === undefined ? 'neighbourhood' : 'path';

  const set = (next: Record<string, string>) =>
    replaceRoute(href({ view: 'entity', id, tab: 'graph', options: next }));

  return (
    <div className="stack">
      <div className="row row--wrap">
        <div className="seg" role="group" aria-label="What to draw">
          <button
            type="button"
            aria-pressed={mode === 'neighbourhood'}
            onClick={() => {
              const next = { ...options };
              delete next['to'];
              set(next);
            }}
          >
            around this entity
          </button>
          <button
            type="button"
            aria-pressed={mode === 'path'}
            onClick={() => set({ ...options, to: options['to'] ?? '' })}
          >
            path to something
          </button>
        </div>
      </div>

      {mode === 'path' ? (
        <PathFinder id={id} options={options} />
      ) : (
        <Neighbours id={id} options={options} />
      )}
    </div>
  );
}

function Neighbours({ id, options }: { id: string; options: Record<string, string> }) {
  const depth = Number(options['depth'] ?? 1) || 1;
  const budget = Number(options['max_nodes'] ?? 60) || 60;
  const forward = options['direction'] === 'forward';
  const resolvedOnly = options['resolved_only'] === 'true';
  const relations = (options['relation'] ?? '').split(',').filter(Boolean);
  const [selected, setSelected] = useState<Selected>(null);

  const set = (next: Record<string, string>) => {
    const merged: Record<string, string> = { ...options, ...next };
    for (const key of Object.keys(merged)) {
      if (merged[key] === '') delete merged[key];
    }
    replaceRoute(href({ view: 'entity', id, tab: 'graph', options: merged }));
  };

  const params = useMemo(
    () => ({
      selector: id,
      depth,
      max_nodes: budget,
      direction: forward ? 'forward' : null,
      resolved_only: resolvedOnly ? 'true' : null,
      relation: relations.length > 0 ? relations : null,
    }),
    [id, depth, budget, forward, resolvedOnly, relations.join(',')],
  );
  const { state, reload } = useApi<Neighbourhood>('/api/neighbourhood', params);

  return (
    <>
      <section className="panel">
        <div className="graph__controls">
          <span className="micro">hops</span>
          <div className="seg" role="group" aria-label="How many hops to expand">
            {DEPTHS.map((value) => (
              <button
                key={value}
                type="button"
                aria-pressed={depth === value}
                onClick={() => set({ depth: String(value) })}
              >
                {value}
              </button>
            ))}
          </div>

          <span className="micro">node budget</span>
          <div className="seg" role="group" aria-label="Largest number of nodes to draw">
            {BUDGETS.map((value) => (
              <button
                key={value}
                type="button"
                aria-pressed={budget === value}
                onClick={() => set({ max_nodes: String(value) })}
              >
                {value}
              </button>
            ))}
          </div>

          <button
            type="button"
            className={forward ? 'btn btn--on' : 'btn'}
            aria-pressed={forward}
            title="Follow assertions only in the direction they were recorded."
            onClick={() => set({ direction: forward ? '' : 'forward' })}
          >
            follow direction
          </button>
          <button
            type="button"
            className={resolvedOnly ? 'btn btn--on' : 'btn'}
            aria-pressed={resolvedOnly}
            title="Leave out every assertion with an unresolved end."
            onClick={() => set({ resolved_only: resolvedOnly ? '' : 'true' })}
          >
            hide unresolved
          </button>
        </div>

        <div className="graph__controls">
          <span className="micro">relations</span>
          <button
            type="button"
            className={relations.length === 0 ? 'btn btn--on' : 'btn'}
            aria-pressed={relations.length === 0}
            onClick={() => set({ relation: '' })}
          >
            all
          </button>
          {RELATIONS.map((name) => {
            const on = relations.includes(name);
            return (
              <button
                key={name}
                type="button"
                className={on ? 'btn btn--on' : 'btn'}
                aria-pressed={on}
                onClick={() =>
                  set({
                    relation: (on
                      ? relations.filter((value) => value !== name)
                      : [...relations, name]
                    ).join(','),
                  })
                }
              >
                {name}
              </button>
            );
          })}
        </div>

        {state.status === 'ready' ? (
          <div className="panel__body">
            <SelectorReading
              selectors={state.data.selectors}
              parameter="selector"
              chosen={state.data.focus}
            />
          </div>
        ) : null}

        {state.status === 'loading' ? (
          <Loading label="Expanding the neighbourhood" />
        ) : state.status === 'error' ? (
          <div className="panel__body">
            <Failure error={state.error} onRetry={reload} />
          </div>
        ) : state.data.node_count <= 1 ? (
          <Empty
            title="Nothing within reach"
            body="No assertion connects this entity to another under these filters. Widen the relations, or turn off the direction and unresolved filters."
          />
        ) : (
          <Picture hood={state.data} selected={selected} onSelect={setSelected} />
        )}
      </section>

      {state.status === 'ready' && state.data.node_count > 1 ? (
        <Chosen hood={state.data} selected={selected} />
      ) : null}
    </>
  );
}

/** The drawing itself, plus the honest account of what it left out. */
function Picture({
  hood,
  selected,
  onSelect,
}: {
  hood: Neighbourhood;
  selected: Selected;
  onSelect: (value: Selected) => void;
}) {
  const nodes: LayoutNode[] = hood.nodes.map((node) => ({
    id: node.entity.entity_id,
    depth: node.depth,
    sortKey: `${node.entity.kind}\u0000${node.entity.qualified_name || node.entity.name}`,
  }));
  const edges: LayoutEdge[] = hood.edges.map((edge) => ({
    source: edge.source_entity_id,
    target: edge.target_entity_id,
  }));
  const placed = layout(nodes, edges, WIDTH, HEIGHT);
  const entities = new Map(hood.nodes.map((node) => [node.entity.entity_id, node.entity]));

  // What "lit" means: the selection and everything one edge away from it. Everything else dims,
  // so a selection reads as a subgraph rather than as a single highlighted dot.
  const lit = new Set<string>();
  const litEdges = new Set<string>();
  if (selected?.kind === 'node') {
    lit.add(selected.id);
    for (const edge of hood.edges) {
      if (edge.source_entity_id === selected.id || edge.target_entity_id === selected.id) {
        litEdges.add(edge.assertion_id);
        lit.add(edge.source_entity_id);
        lit.add(edge.target_entity_id);
      }
    }
  } else if (selected?.kind === 'edge') {
    const edge = hood.edges.find((item) => item.assertion_id === selected.id);
    if (edge) {
      litEdges.add(edge.assertion_id);
      lit.add(edge.source_entity_id);
      lit.add(edge.target_entity_id);
    }
  }
  const dimming = lit.size > 0;

  return (
    <div className="graph">
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        role="img"
        aria-label={`${hood.node_count} entities within ${hood.max_depth} hops of ${
          hood.focus.qualified_name || hood.focus.name
        }, drawn on ${hood.max_depth} rings`}
      >
        {placed.rings.map((ring) => {
          const at = ringLabelPlacement(ring, placed.nodes, placed.centre);
          return (
            <g key={ring.depth}>
              <circle
                className="graph__ring"
                cx={placed.centre.x}
                cy={placed.centre.y}
                r={ring.radius}
              />
              <text className="graph__ringlabel" x={at.x} y={at.y} textAnchor="middle">
                {ring.depth} {ring.depth === 1 ? 'hop' : 'hops'}
              </text>
            </g>
          );
        })}

        {hood.truncated ? (
          <>
            <circle
              className="graph__omitted"
              cx={placed.centre.x}
              cy={placed.centre.y}
              r={placed.extent + 22}
            />
            <text
              className="graph__ringlabel"
              x={placed.centre.x}
              y={placed.centre.y - placed.extent - 30}
              textAnchor="middle"
            >
              {count(hood.omitted_nodes)} more not drawn
            </text>
          </>
        ) : null}

        {hood.edges.map((edge) => {
          const from = placed.byId.get(edge.source_entity_id);
          const to = placed.byId.get(edge.target_entity_id);
          if (!from || !to) return null;
          const classes = ['graph__edge'];
          if (edge.is_unresolved) classes.push('graph__edge--unresolved');
          if (litEdges.has(edge.assertion_id)) classes.push('graph__edge--lit');
          else if (dimming) classes.push('graph__edge--dim');
          return (
            <path
              key={edge.assertion_id}
              className={classes.join(' ')}
              d={edgePath(from, to, placed.centre)}
              onClick={() => onSelect({ kind: 'edge', id: edge.assertion_id })}
            >
              <title>
                {`${entities.get(edge.source_entity_id)?.qualified_name ?? edge.source_entity_id} ${relationPhrase(edge.relation, true)} ${entities.get(edge.target_entity_id)?.qualified_name ?? edge.target_entity_id}`}
              </title>
            </path>
          );
        })}

        {placed.nodes.map((node) => {
          const entity = entities.get(node.id);
          if (!entity) return null;
          const focus = node.depth === 0;
          const label = entity.qualified_name || entity.name;
          const place = labelPlacement(node);
          const nodeClasses = ['graph__node'];
          if (focus) nodeClasses.push('graph__node--focus');
          if (entity.kind === 'unresolved') nodeClasses.push('graph__node--unresolved');
          if (lit.has(node.id) && !focus) nodeClasses.push('graph__node--lit');
          else if (dimming && !lit.has(node.id)) nodeClasses.push('graph__node--dim');
          const labelClasses = ['graph__label'];
          if (focus) labelClasses.push('graph__label--focus');
          if (dimming && !lit.has(node.id)) labelClasses.push('graph__label--dim');
          return (
            <g key={node.id}>
              {selected?.kind === 'node' && selected.id === node.id ? (
                <circle className="graph__halo" cx={node.x} cy={node.y} r={focus ? 13 : 10} />
              ) : null}
              <circle
                className={nodeClasses.join(' ')}
                cx={node.x}
                cy={node.y}
                r={focus ? 7.5 : 4.6}
                onClick={() => onSelect({ kind: 'node', id: node.id })}
                tabIndex={0}
                role="button"
                aria-label={`${entity.kind} ${label}`}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    onSelect({ kind: 'node', id: node.id });
                  }
                }}
              >
                <title>{`${entity.kind} · ${label}`}</title>
              </circle>
              <text
                className={labelClasses.join(' ')}
                x={place.x}
                y={place.y}
                textAnchor={place.anchor}
              >
                {elide(label, focus ? 40 : 26)}
              </text>
            </g>
          );
        })}
      </svg>

      <div className="graph__foot">
        <span>
          {count(hood.node_count)} of {count(hood.node_count + hood.omitted_nodes)} entities ·{' '}
          {count(hood.edge_count)} assertions · {count(hood.frontier_nodes)} on the outer ring
        </span>
        <span className="spacer" />
        <span className="legend">
          <svg width="26" height="8" aria-hidden="true">
            <path className="graph__edge" d="M1 4 L25 4" />
          </svg>
          resolved
        </span>
        <span className="legend">
          <svg width="26" height="8" aria-hidden="true">
            <path className="graph__edge graph__edge--unresolved" d="M1 4 L25 4" />
          </svg>
          unresolved end
        </span>
      </div>
    </div>
  );
}

/** What was clicked, described in words, with somewhere to go next. */
function Chosen({ hood, selected }: { hood: Neighbourhood; selected: Selected }) {
  if (selected === null) {
    return (
      <Panel title="Selection">
        <p className="prose">
          Click a node or a line to read what it is. The centre is{' '}
          <span className="hash">{hood.focus.qualified_name || hood.focus.name}</span>; each ring
          out is one more hop from it.
        </p>
      </Panel>
    );
  }

  if (selected.kind === 'node') {
    const node = hood.nodes.find((item) => item.entity.entity_id === selected.id);
    if (!node) return null;
    return (
      <Panel title="Selected entity">
        <div className="stack" style={{ gap: 12 }}>
          <div className="row row--wrap">
            <Chip tone={node.entity.kind === 'unresolved' ? 'unknown' : 'quiet'}>
              {node.entity.kind}
            </Chip>
            <Chip tone="quiet">
              {node.depth === 0 ? 'the focus' : `${node.depth} ${node.depth === 1 ? 'hop' : 'hops'} out`}
            </Chip>
            <Where path={node.entity.file_path} line={node.entity.start_line} />
          </div>
          <div style={{ fontSize: 15 }} className="wrapany">
            <EntityLink entity={node.entity} />
          </div>
          <div className="row row--wrap">
            <a className="btn" href={entityHref(node.entity.entity_id, 'evidence')}>
              Why does Nerve believe this?
            </a>
            <a className="btn" href={entityHref(node.entity.entity_id, 'graph')}>
              Centre the graph here
            </a>
          </div>
        </div>
      </Panel>
    );
  }

  const edge = hood.edges.find((item) => item.assertion_id === selected.id);
  if (!edge) return null;
  const source = hood.nodes.find((item) => item.entity.entity_id === edge.source_entity_id)?.entity;
  const target = hood.nodes.find((item) => item.entity.entity_id === edge.target_entity_id)?.entity;
  return (
    <Panel title="Selected assertion">
      <EdgeFacts edge={edge} source={source} target={target} />
    </Panel>
  );
}

export function EdgeFacts({
  edge,
  source,
  target,
}: {
  edge: NeighbourEdge;
  source: Entity | undefined;
  target: Entity | undefined;
}) {
  return (
    <div className="stack" style={{ gap: 12 }}>
      <div className="claim__sentence">
        {source ? <EntityLink entity={source} className="claim__party" /> : <span>{edge.source_entity_id}</span>}
        <span className="claim__verb">{relationPhrase(edge.relation, true)}</span>
        {target ? <EntityLink entity={target} className="claim__party" /> : <span>{edge.target_entity_id}</span>}
      </div>
      <div className="row row--wrap">
        <Chip tone={edge.status === 'SUPPORTED' ? 'fresh' : 'stale'}>{edge.status.toLowerCase()}</Chip>
        {edge.is_unresolved ? <Chip tone="unknown">one end unresolved</Chip> : null}
        <Chip tone="quiet">
          {count(edge.observation_count)}{' '}
          {edge.observation_count === 1 ? 'observation' : 'observations'}
        </Chip>
        <Chip tone="quiet">{edge.strongest_source_type}</Chip>
        <Where path={edge.file_path} line={edge.start_line} />
      </div>
      {source ? (
        <div className="row row--wrap">
          <a
            className="btn"
            href={href({
              view: 'entity',
              id: source.entity_id,
              tab: 'evidence',
              options: { object: edge.target_entity_id },
            })}
          >
            Open the evidence for this assertion
          </a>
        </div>
      ) : null}
    </div>
  );
}
