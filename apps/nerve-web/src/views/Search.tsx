/**
 * Search.
 *
 * The index matches **prefixes, term by term, ANDed** — that is the rule `nerve-store` implements
 * and this screen states it out loud rather than letting the reader infer it from silence. When
 * nothing matches, the empty state is the most useful part of the view: it says why, and offers
 * other queries built from the reader's own words (see `../search.ts`). No result is ever
 * invented to soften a miss.
 */

import { useEffect, useMemo, useRef, useState } from 'react';

import { ENTITY_KINDS, type SearchResponse } from '../api/types';
import { count } from '../format';
import { useApi, useDebounced } from '../hooks';
import { entityHref, href, navigate, replaceRoute } from '../routing';
import { widenings } from '../search';
import { Chip, Empty, Failure, Loading } from '../ui/parts';

const LIMIT = 60;

export function Search({ q, kind }: { q: string; kind: string | null }) {
  const [text, setText] = useState(q);
  const [active, setActive] = useState(-1);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    setText(q);
  }, [q]);

  const settled = useDebounced(text.trim(), 160);

  // The route is the source of truth for what is being searched, so it is kept in step with the
  // field — replaced rather than pushed, or the back button would become a per-character undo.
  useEffect(() => {
    if (settled !== q) replaceRoute(href({ view: 'search', q: settled, kind }));
  }, [settled, q, kind]);

  const params = useMemo(() => ({ q: settled, kind, limit: LIMIT }), [settled, kind]);
  const { state, reload } = useApi<SearchResponse>(settled.length > 0 ? '/api/search' : null, params);

  const hits = state.status === 'ready' && settled.length > 0 ? state.data.results : [];

  useEffect(() => {
    setActive(-1);
  }, [settled, kind]);

  useEffect(() => {
    if (active < 0) return;
    const node = list.current?.querySelectorAll('a')[active];
    node?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (hits.length === 0) return;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActive((index) => (index + 1) % hits.length);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActive((index) => (index <= 0 ? hits.length : index) - 1);
    } else if (event.key === 'Enter' && active >= 0) {
      event.preventDefault();
      const chosen = hits[active];
      if (chosen) navigate(entityHref(chosen.entity_id));
    } else if (event.key === 'Escape') {
      setActive(-1);
    }
  };

  return (
    <div className="view" onKeyDown={onKeyDown}>
      <div className="head">
        <h1 className="head__title">Symbols</h1>
        <p className="head__sub">
          Every entity Nerve named while indexing — files, modules, classes, functions, methods,
          and the references it could not resolve.
        </p>
      </div>

      <div className="stack">
        <div className="row row--wrap">
          <div className="field" style={{ flex: '1 1 320px' }}>
            <span className="micro" aria-hidden="true">
              find
            </span>
            <input
              type="text"
              value={text}
              spellCheck={false}
              autoComplete="off"
              aria-label="Search indexed symbols"
              placeholder="start of a name, or start of a scope"
              onChange={(event) => setText(event.target.value)}
            />
          </div>
        </div>

        <div className="row row--wrap" role="group" aria-label="Filter by kind">
          <span className="micro">kind</span>
          <a
            className={kind === null ? 'btn btn--on' : 'btn'}
            href={href({ view: 'search', q: settled, kind: null })}
          >
            any
          </a>
          {ENTITY_KINDS.map((name) => (
            <a
              key={name}
              className={kind === name ? 'btn btn--on' : 'btn'}
              href={href({ view: 'search', q: settled, kind: name })}
            >
              {name}
            </a>
          ))}
        </div>

        {settled.length === 0 ? (
          <Empty
            title="Type to search"
            body="Matching is by prefix: each word you type must start a word in the symbol's name or its scope. Searching area finds Circle.area; searching rea finds nothing."
          >
            <div className="row row--wrap">
              <span className="kbd">/</span>
              <span className="state__body">focuses the search box from anywhere</span>
            </div>
          </Empty>
        ) : state.status === 'loading' ? (
          <Loading label="Searching" />
        ) : state.status === 'error' ? (
          <Failure error={state.error} onRetry={reload} />
        ) : hits.length === 0 ? (
          <NoMatches query={settled} kind={kind} />
        ) : (
          <section className="panel">
            <header className="panel__head">
              <h2 className="micro">
                {hits.length === LIMIT
                  ? `first ${count(LIMIT)} matches`
                  : `${count(hits.length)} ${hits.length === 1 ? 'match' : 'matches'}`}
              </h2>
              <span className="hash">
                <span className="kbd">↑↓</span> move · <span className="kbd">↵</span> open
              </span>
            </header>
            <div className="panel__body panel__body--flush">
              <ul className="hits" ref={list}>
                {hits.map((hit, index) => (
                  <li key={hit.entity_id}>
                    <a
                      className={hit.kind === 'unresolved' ? 'hit hit--unresolved' : 'hit'}
                      href={entityHref(hit.entity_id)}
                      aria-selected={index === active}
                      onMouseEnter={() => setActive(index)}
                    >
                      <span className="hit__kind">{hit.kind}</span>
                      <span className="hit__name truncate">
                        {hit.scope_path ? (
                          <span className="hit__scope">{hit.scope_path}.</span>
                        ) : null}
                        {hit.name}
                      </span>
                      <span className="hit__where">
                        {hit.file_path ?? 'no recorded location'}
                        {hit.start_line === null ? '' : `:${hit.start_line}`}
                      </span>
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          </section>
        )}
      </div>
    </div>
  );
}

/**
 * The empty result, which is where this screen earns its keep.
 *
 * It names the rule that produced the miss and offers alternatives built from the reader's own
 * string. Each one is a visible query, not a hidden rewrite.
 */
function NoMatches({ query, kind }: { query: string; kind: string | null }) {
  const others = widenings(query);
  return (
    <Empty
      title="Nothing starts with that"
      body={
        kind === null
          ? 'Terms match from the start of a word, and every term has to match. A word from the middle of a name will not find it.'
          : `No ${kind} matches, though another kind might. Terms match from the start of a word, and every term has to match.`
      }
    >
      {kind === null ? null : (
        <a className="btn" href={href({ view: 'search', q: query, kind: null })}>
          Search every kind
        </a>
      )}
      {others.length > 0 ? (
        <div className="stack" style={{ gap: 8, width: '100%' }}>
          <div className="micro">try instead</div>
          <ul className="hits">
            {others.map((other) => (
              <li key={other.query}>
                <a className="hit" href={href({ view: 'search', q: other.query, kind })}>
                  <span className="hit__kind">query</span>
                  <span className="hit__name truncate">{other.query}</span>
                  <span className="hit__where">{other.why}</span>
                </a>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <Chip tone="quiet">searched for {query}</Chip>
    </Empty>
  );
}
