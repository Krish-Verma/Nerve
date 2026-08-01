/**
 * The one search field, in the bar, reachable from anywhere with `/`.
 *
 * It is a combobox rather than a form: there is no submit, because every keystroke is already a
 * query and the server is on loopback. The list it drops down is the *same* data the Search view
 * renders, so a reader who takes the keyboard route and a reader who takes the mouse route are
 * looking at one thing described one way.
 *
 * Selection is deliberately optimistic-free: nothing is highlighted until an arrow key is pressed,
 * so pressing Enter straight after typing opens the full result list rather than gambling on the
 * top hit being the one that was meant.
 */

import { useEffect, useId, useMemo, useRef, useState } from 'react';

import type { SearchResponse } from '../api/types';
import { useApi, useDebounced, useGlobalKey } from '../hooks';
import { href, navigate } from '../routing';
import { entityHref } from '../routing';

const SUGGESTIONS = 8;

export function Omnibox({ initial }: { initial: string }) {
  const [text, setText] = useState(initial);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const input = useRef<HTMLInputElement>(null);
  const listId = useId();

  // The field follows the route when the route changes underneath it — opening a search link
  // from elsewhere in the app should leave the box showing what is being searched for.
  useEffect(() => {
    setText(initial);
  }, [initial]);

  const settled = useDebounced(text.trim(), 140);
  const { state } = useApi<SearchResponse>(
    settled.length > 0 ? '/api/search' : null,
    useMemo(() => ({ q: settled, limit: SUGGESTIONS }), [settled]),
  );

  const hits = state.status === 'ready' ? state.data.results : [];
  const showing = open && settled.length > 0;

  useEffect(() => {
    setActive(-1);
  }, [settled]);

  useGlobalKey((event) => {
    if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
    event.preventDefault();
    input.current?.focus();
    input.current?.select();
  });

  const openSearch = (query: string) => {
    setOpen(false);
    navigate(href({ view: 'search', q: query, kind: null }));
  };

  return (
    <div className="omni">
      <div className="field">
        <span className="micro" aria-hidden="true">
          find
        </span>
        <input
          ref={input}
          type="text"
          value={text}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          role="combobox"
          aria-expanded={showing}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-label="Search indexed symbols"
          aria-activedescendant={active >= 0 ? `${listId}-${active}` : undefined}
          placeholder="symbol, file or scope"
          onChange={(event) => {
            setText(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          // A click inside the list must not close it before the click lands, so the blur is
          // deferred by one frame rather than handled synchronously.
          onBlur={() => window.setTimeout(() => setOpen(false), 120)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown') {
              event.preventDefault();
              setOpen(true);
              setActive((index) => (hits.length === 0 ? -1 : (index + 1) % hits.length));
            } else if (event.key === 'ArrowUp') {
              event.preventDefault();
              setActive((index) =>
                hits.length === 0 ? -1 : (index <= 0 ? hits.length : index) - 1,
              );
            } else if (event.key === 'Enter') {
              event.preventDefault();
              const chosen = active >= 0 ? hits[active] : undefined;
              if (chosen) {
                setOpen(false);
                navigate(entityHref(chosen.entity_id));
              } else if (text.trim().length > 0) {
                openSearch(text.trim());
              }
            } else if (event.key === 'Escape') {
              event.preventDefault();
              if (showing) {
                setOpen(false);
              } else {
                setText('');
                input.current?.blur();
              }
            }
          }}
        />
        <span className="kbd" aria-hidden="true">
          /
        </span>
      </div>

      {showing ? (
        <div className="omni__pop">
          <ul className="hits" id={listId} role="listbox" aria-label="Search results">
            {hits.map((hit, index) => (
              <li key={hit.entity_id} role="presentation">
                <a
                  id={`${listId}-${index}`}
                  role="option"
                  aria-selected={index === active}
                  className={hit.kind === 'unresolved' ? 'hit hit--unresolved' : 'hit'}
                  href={entityHref(hit.entity_id)}
                  onMouseEnter={() => setActive(index)}
                  onClick={() => setOpen(false)}
                >
                  <span className="hit__kind">{hit.kind}</span>
                  <span className="hit__name truncate">
                    {hit.scope_path ? <span className="hit__scope">{hit.scope_path}.</span> : null}
                    {hit.name}
                  </span>
                  <span className="hit__where">
                    {hit.file_path ?? 'no location'}
                    {hit.start_line === null ? '' : `:${hit.start_line}`}
                  </span>
                </a>
              </li>
            ))}
          </ul>
          <div className="omni__foot">
            {state.status === 'loading' ? (
              <span>searching…</span>
            ) : hits.length === 0 ? (
              <span>Nothing starts with that. Press Enter for ways to widen it.</span>
            ) : (
              <>
                <span>
                  {hits.length === SUGGESTIONS ? `first ${SUGGESTIONS}` : `${hits.length} found`}
                </span>
                <span className="spacer" />
                <span className="kbd">↑↓</span>
                <span>move</span>
                <span className="kbd">↵</span>
                <span>{active >= 0 ? 'open' : 'see all'}</span>
                <span className="kbd">esc</span>
                <span>dismiss</span>
              </>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
