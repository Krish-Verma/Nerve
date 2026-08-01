/**
 * The one place this app talks to the server.
 *
 * The session token arrives in the URL that `nerve serve` prints and is held in memory for the
 * life of the tab. It is deliberately **not** copied into `localStorage` or `sessionStorage`:
 * `nerve serve` promises the token is never written to disk, and browsers persist both of those
 * for session restore. Leaving it in the address bar is the honest trade — it is already in the
 * shell history that produced it, and a reload still works.
 */

import type { Json } from './types';

/** Header the server accepts the session token on. */
const TOKEN_HEADER = 'X-Nerve-Token';

/** A refusal from the API, in the one shape every failure takes. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly detail: Json;

  constructor(status: number, code: string, message: string, detail: Json) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.detail = detail;
  }

  /** True when the session token is missing, wrong, or has died with the server process. */
  get isAuth(): boolean {
    return this.code === 'token_required' || this.code === 'token_invalid';
  }
}

/** The server was not reachable at all — usually because `nerve serve` has stopped. */
export class TransportError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TransportError';
  }
}

function readToken(): string | null {
  const value = new URLSearchParams(window.location.search).get('token');
  return value && value.length > 0 ? value : null;
}

let token = readToken();

/** The session token, or null when the page was opened without one. */
export function sessionToken(): string | null {
  return token;
}

/** Forget the token after the server has rejected it, so the app can ask for a fresh URL. */
export function forgetToken(): void {
  token = null;
}

export type Params = Record<string, string | number | boolean | string[] | null | undefined>;

/** Build a request target. Repeated values become repeated parameters, as the server expects. */
export function query(path: string, params: Params = {}): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === null || value === undefined || value === '') continue;
    if (Array.isArray(value)) {
      if (value.length > 0) search.append(key, value.join(','));
    } else {
      search.append(key, String(value));
    }
  }
  const text = search.toString();
  return text.length > 0 ? `${path}?${text}` : path;
}

interface Envelope {
  ok?: boolean;
  status?: number;
  error?: { code?: string; message?: string; detail?: Json };
}

/**
 * Perform one read.
 *
 * Two failure shapes are distinguished on purpose. A `TransportError` means the process is gone;
 * an `ApiError` means the process answered and refused, and its structured `detail` is often the
 * most useful thing on the screen — an ambiguous selector arrives with every candidate attached.
 */
export async function get<T>(path: string, params: Params = {}, signal?: AbortSignal): Promise<T> {
  if (token === null) {
    throw new ApiError(401, 'token_required', 'this page has no session token', null);
  }

  let response: Response;
  try {
    response = await fetch(query(path, params), {
      method: 'GET',
      headers: { [TOKEN_HEADER]: token, Accept: 'application/json' },
      credentials: 'omit',
      cache: 'no-store',
      redirect: 'error',
      signal: signal ?? null,
    });
  } catch (cause) {
    if (cause instanceof DOMException && cause.name === 'AbortError') throw cause;
    throw new TransportError('nerve serve did not answer. The process may have stopped.');
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    throw new TransportError(`the server answered ${response.status} with a body that is not JSON`);
  }

  const envelope = body as Envelope;
  if (!response.ok || envelope.ok === false) {
    const error = envelope.error ?? {};
    throw new ApiError(
      response.status,
      error.code ?? 'unknown',
      error.message ?? `request failed with ${response.status}`,
      error.detail ?? null,
    );
  }
  return body as T;
}
