/**
 * Hash routing.
 *
 * The server serves exactly one document and a fixed table of assets — there is no history
 * fallback and there should not be one, because a wildcard route is a filesystem surface. The
 * fragment never reaches the server, so `#/entity/<id>/evidence` is both bookmarkable and
 * impossible to mistake for a path the server must resolve.
 */

import { useCallback, useEffect, useState } from 'react';

import { historyFragment, isHistoryTab, type HistoryTab } from './history';

export type EntityTab = 'relations' | 'evidence' | 'graph' | 'source';

export const ENTITY_TABS: readonly EntityTab[] = ['relations', 'evidence', 'graph', 'source'];

/**
 * The screens.
 *
 * `unresolved` and `coverage` are deliberately two routes rather than one "gaps" route. They are
 * different questions with different evidence behind them — what Nerve could not work out, and
 * what the tests are not known to touch — and a single word covering both is how a reader ends up
 * reading one answer as the other. There is no `#/gaps` any more, in either sense.
 *
 * `history` carries a tab and free options for the same reason `entity` does: three of its five
 * questions are about a subject the user types — a path, or two commit oids — and an answer that
 * cannot be linked to is an answer that cannot be quoted in a review.
 */
export type Route =
  | { view: 'overview' }
  | { view: 'search'; q: string; kind: string | null }
  | { view: 'entity'; id: string; tab: EntityTab; options: Record<string, string> }
  | { view: 'unresolved' }
  | { view: 'coverage' }
  | { view: 'history'; tab: HistoryTab; options: Record<string, string> };

function parse(hash: string): Route {
  const raw = hash.startsWith('#') ? hash.slice(1) : hash;
  const [pathPart = '', queryPart = ''] = raw.split('?', 2);
  const segments = pathPart.split('/').filter((segment) => segment.length > 0);
  const search = new URLSearchParams(queryPart);

  switch (segments[0]) {
    case 'search':
      return { view: 'search', q: search.get('q') ?? '', kind: search.get('kind') };
    case 'entity': {
      const id = segments[1] ? decodeURIComponent(segments[1]) : '';
      if (id === '') return { view: 'overview' };
      const candidate = segments[2] as EntityTab | undefined;
      const tab = candidate && ENTITY_TABS.includes(candidate) ? candidate : 'relations';
      const options: Record<string, string> = {};
      search.forEach((value, key) => {
        options[key] = value;
      });
      return { view: 'entity', id, tab, options };
    }
    case 'unresolved':
      return { view: 'unresolved' };
    case 'coverage':
      return { view: 'coverage' };
    case 'history': {
      const tab = isHistoryTab(segments[1]) ? segments[1] : 'commits';
      const options: Record<string, string> = {};
      search.forEach((value, key) => {
        options[key] = value;
      });
      return { view: 'history', tab, options };
    }
    default:
      return { view: 'overview' };
  }
}

/** Build a fragment. Every caller goes through here so no route is spelled two ways. */
export function href(route: Route): string {
  switch (route.view) {
    case 'overview':
      return '#/overview';
    case 'search': {
      const search = new URLSearchParams();
      if (route.q) search.set('q', route.q);
      if (route.kind) search.set('kind', route.kind);
      const text = search.toString();
      return text ? `#/search?${text}` : '#/search';
    }
    case 'entity': {
      const search = new URLSearchParams(route.options);
      const text = search.toString();
      const base = `#/entity/${encodeURIComponent(route.id)}/${route.tab}`;
      return text ? `${base}?${text}` : base;
    }
    case 'unresolved':
      return '#/unresolved';
    case 'coverage':
      return '#/coverage';
    // Built in `history.ts` rather than here, because the encoding is the part that matters: a
    // path may hold a `#`, and a raw one would be dropped by the browser before the request left.
    case 'history':
      return historyFragment(route.tab, route.options);
  }
}

/** A shorthand for the common case: open an entity on its default tab. */
export function entityHref(id: string, tab: EntityTab = 'relations'): string {
  return href({ view: 'entity', id, tab, options: {} });
}

export function navigate(target: string): void {
  window.location.hash = target.startsWith('#') ? target.slice(1) : target;
}

/**
 * Change the route without adding a history entry.
 *
 * Typing into the search field rewrites the route on every keystroke. Pushing each one would turn
 * the back button into a per-character undo, so the query is replaced in place and the listeners
 * are notified by hand — `replaceState` does not fire `hashchange` on its own.
 */
export function replaceRoute(target: string): void {
  const url = new URL(window.location.href);
  url.hash = target.startsWith('#') ? target.slice(1) : target;
  if (url.href === window.location.href) return;
  window.history.replaceState(null, '', url);
  window.dispatchEvent(new HashChangeEvent('hashchange'));
}

export function useRoute(): [Route, (target: string) => void] {
  const [route, setRoute] = useState<Route>(() => parse(window.location.hash));

  useEffect(() => {
    const onChange = () => setRoute(parse(window.location.hash));
    window.addEventListener('hashchange', onChange);
    return () => window.removeEventListener('hashchange', onChange);
  }, []);

  const go = useCallback((target: string) => navigate(target), []);
  return [route, go];
}
