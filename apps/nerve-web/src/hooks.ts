/**
 * Data-loading primitives.
 *
 * Every view in this app has four states and none of them is optional: loading, ready, empty and
 * refused. A hook that collapsed "refused" into "no data" would hide exactly the information this
 * product exists to show, so the error is carried through with its code and structured detail
 * intact and rendered, not swallowed.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { get, query, type Params } from './api/client';

export type Async<T> =
  | { status: 'loading' }
  | { status: 'ready'; data: T }
  | { status: 'error'; error: unknown };

export interface Resource<T> {
  state: Async<T>;
  reload: () => void;
}

/** Read one endpoint. Passing `null` for the path holds the hook idle without fetching. */
export function useApi<T>(path: string | null, params: Params = {}): Resource<T> {
  const key = path === null ? null : query(path, params);
  const [state, setState] = useState<Async<T>>({ status: 'loading' });
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    if (key === null) return;
    const controller = new AbortController();
    let live = true;
    setState({ status: 'loading' });
    get<T>(key, {}, controller.signal)
      .then((data) => {
        if (live) setState({ status: 'ready', data });
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === 'AbortError') return;
        if (live) setState({ status: 'error', error });
      });
    return () => {
      live = false;
      controller.abort();
    };
  }, [key, nonce]);

  const reload = useCallback(() => setNonce((value) => value + 1), []);
  return useMemo(() => ({ state, reload }), [state, reload]);
}

/** Delay a fast-changing value, so typing does not become one request per keystroke. */
export function useDebounced<T>(value: T, delay = 160): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(value), delay);
    return () => window.clearTimeout(timer);
  }, [value, delay]);
  return settled;
}

/** Run a callback on every keydown at the document level. Used for the global shortcuts. */
export function useGlobalKey(handler: (event: KeyboardEvent) => void): void {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => latest.current(event);
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
}
