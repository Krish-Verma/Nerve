/**
 * The shared vocabulary of the interface.
 *
 * Two of these are load-bearing rather than decorative:
 *
 * - `Chip` is the only component allowed to carry hue, because hue on this screen means a claim's
 *   state and nothing else.
 * - `Failure` renders a refusal as a *place to go next*. The API answers an ambiguous selector
 *   with every candidate attached and a missing one with suggestions; throwing that away and
 *   printing "404" would be discarding the most useful thing on the screen.
 *
 * Every value that came from the repository is interpolated as a React child or an attribute
 * value, which React escapes. Nothing in this app builds markup from a string.
 */

import type { ReactNode } from 'react';

import { ApiError, TransportError } from '../api/client';
import type { Entity, Json, SearchHit, SelectorNotes } from '../api/types';
import { count, elide, type Tone } from '../format';
import { entityHref } from '../routing';
import { alternativesFor, matchedBy, matchedByReading, passedOverReading, selectorFor } from '../selectors';

export function Chip({
  tone = 'plain',
  title,
  prose = false,
  children,
}: {
  tone?: Tone;
  title?: string;
  /**
   * This chip carries a **sentence** rather than a token, so it may wrap.
   *
   * `.chip` is `white-space: nowrap`, which is right for `similar_content` or `many_from` — a
   * stored value broken across two lines reads as two values. It is wrong for a chip whose content
   * is a clause: at a 380px viewport a 49-character sentence is wider than the panel that holds it,
   * and because `.row--wrap` wraps between chips rather than inside one, the chip overflowed its
   * container. Measured at 380px: `div.row--wrap` `scrollWidth` 379 against `clientWidth` 304.
   *
   * A modifier rather than a change to `.chip`, because the tokens must keep not wrapping.
   */
  prose?: boolean;
  children: ReactNode;
}) {
  const suffix = tone === 'plain' ? '' : ` chip--${tone}`;
  const wrap = prose ? ' chip--prose' : '';
  return (
    <span className={`chip${suffix}${wrap}`} title={title}>
      {children}
    </span>
  );
}

export function Panel({
  title,
  aside,
  flush,
  children,
}: {
  title: string;
  aside?: ReactNode;
  flush?: boolean;
  children: ReactNode;
}) {
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">{title}</h2>
        {aside}
      </header>
      <div className={flush ? 'panel__body panel__body--flush' : 'panel__body'}>{children}</div>
    </section>
  );
}

export function Figure({
  label,
  value,
  note,
  unknown,
}: {
  label: string;
  value: ReactNode;
  note?: ReactNode;
  unknown?: boolean;
}) {
  return (
    <div className="figure">
      <div className="micro">{label}</div>
      <div className={unknown ? 'figure__value figure__value--unknown' : 'figure__value'}>
        {value}
      </div>
      {note ? <div className="figure__note">{note}</div> : null}
    </div>
  );
}

export function Tally({
  rows,
  unknownKey,
}: {
  rows: [string, number][];
  unknownKey?: string;
}) {
  const peak = rows.reduce((most, [, value]) => Math.max(most, value), 0) || 1;
  return (
    <div className="tally">
      {rows.map(([name, value]) => (
        <div
          key={name}
          className={name === unknownKey ? 'tally__row tally__row--unknown' : 'tally__row'}
        >
          <span className="tally__name truncate" title={name}>
            {name}
          </span>
          <span className="tally__track">
            <span
              className="tally__fill"
              style={{ width: `${Math.max(2, Math.round((value / peak) * 100))}%` }}
            />
          </span>
          <span className="tally__value num">{count(value)}</span>
        </div>
      ))}
    </div>
  );
}

export function Def({ term, children }: { term: string; children: ReactNode }) {
  return (
    <>
      <dt>{term}</dt>
      <dd className="wrapany">{children}</dd>
    </>
  );
}

export function Defs({ children }: { children: ReactNode }) {
  return <dl className="defs">{children}</dl>;
}

/** A link to an entity. The label is repository text and is rendered as a child, never as HTML. */
export function EntityLink({
  entity,
  tab,
  max = 0,
  className = 'link',
}: {
  entity: Pick<Entity, 'entity_id' | 'qualified_name' | 'name' | 'kind'>;
  tab?: 'relations' | 'evidence' | 'graph' | 'source';
  max?: number;
  className?: string;
}) {
  const label = entity.qualified_name || entity.name;
  return (
    <a
      className={className}
      href={entityHref(entity.entity_id, tab ?? 'relations')}
      title={`${entity.kind} · ${label}`}
    >
      {max > 0 ? elide(label, max) : label}
    </a>
  );
}

/**
 * What the selector resolved to, and **what it passed over**.
 *
 * The server has reported this on every selector answer since Slice 8b-i and nothing rendered it,
 * which made the report worth as much as not making it. One path can hold two entities —
 * `src/app.ts` is both a `Module` and a `File`, `docs/architecture.md` both a `Document` and a
 * `File` — and the rule is content wins, container is reported. A reader who types the path and is
 * shown one entity has no way to learn the other exists unless this says so.
 *
 * Renders **nothing** in the ordinary case, and that is deliberate rather than a shortcut:
 * `alternatives` is empty for the overwhelming majority of selectors, so a permanent strip saying
 * "no alternatives" would be noise on every screen in the app in order to be information on almost
 * none. The absence of this block is not the absence of a check — the check is on every answer,
 * and this is what it looks like when it found nothing to report.
 *
 * Note this is **not** ambiguity. An ambiguous selector is a `409` with every candidate attached
 * and no choice made, which `Failure` renders. This is a choice made *by a stated rule*, and the
 * two must not read alike.
 */
export function SelectorReading({
  selectors,
  parameter,
  chosen,
}: {
  selectors: SelectorNotes | undefined;
  /** The query parameter name the server keyed this by: `subject`, `selector`, `from`, `to`. */
  parameter: string;
  /** What the selector resolved to, so the sentence can name the kind that won. */
  chosen: Pick<Entity, 'kind'>;
}) {
  const alternatives = alternativesFor(selectors, parameter);
  if (alternatives.length === 0) return null;

  const stage = matchedBy(selectors, parameter);
  const stageNote = matchedByReading(stage);

  return (
    <div className="stack" style={{ gap: 8 }}>
      <div className="row row--wrap">
        <span className="micro">also at this path</span>
        {alternatives.map((entity) => (
          <Chip key={entity.entity_id} tone="quiet" title={selectorFor(entity)}>
            {entity.kind} · <EntityLink entity={entity} max={40} />
          </Chip>
        ))}
      </div>
      <p className="prose">{passedOverReading(chosen.kind, alternatives)}</p>
      {stageNote === '' ? null : <p className="hash wrapany">{stageNote}</p>}
      <div className="row row--wrap">
        <span className="micro">ask for it by name</span>
        {alternatives.map((entity) => (
          <code key={entity.entity_id} className="hash wrapany">
            {selectorFor(entity)}
          </code>
        ))}
      </div>
    </div>
  );
}

export function Where({ path, line }: { path: string | null; line?: number | null }) {
  if (!path) return <span className="hash">no recorded location</span>;
  return (
    <span className="hash wrapany">
      {path}
      {line === null || line === undefined ? '' : `:${line}`}
    </span>
  );
}

export function Loading({ label = 'Reading the index' }: { label?: string }) {
  return (
    <div className="state" role="status" aria-live="polite">
      <div className="state__title">{label}…</div>
      <div className="signal" />
    </div>
  );
}

export function Empty({
  title,
  body,
  children,
}: {
  title: string;
  body: string;
  children?: ReactNode;
}) {
  return (
    <div className="state">
      <div className="state__title">{title}</div>
      <p className="state__body">{body}</p>
      {children}
    </div>
  );
}

/**
 * Pull the entity-shaped members out of an error's structured detail.
 *
 * The detail blob is `Json` — the server sends whatever the refusal needed to carry, and this
 * app must not assume a shape it did not check. So each member is tested for the one field that
 * makes it usable and cast through `unknown`; a value that fails the test is dropped rather than
 * rendered as a hole.
 */
function shaped<T>(value: Json): T[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (item) =>
      typeof item === 'object' &&
      item !== null &&
      !Array.isArray(item) &&
      typeof (item as Record<string, Json>)['entity_id'] === 'string',
  ) as unknown as T[];
}

function detailField(detail: Json, key: string): Json {
  if (detail === null || typeof detail !== 'object' || Array.isArray(detail)) return null;
  return (detail as Record<string, Json>)[key] ?? null;
}

/**
 * A refusal, rendered as something to do next.
 *
 * The wording never apologises and never guesses: it says which check fired and what the caller
 * can pick instead, because on this product a refusal is frequently the correct answer.
 */
export function Failure({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  if (error instanceof TransportError) {
    return (
      <div className="state state--error">
        <div className="state__title">nerve serve is not answering</div>
        <p className="state__body">
          The request never reached a server. The process that printed this page&apos;s URL has
          probably stopped. Start it again with <code>nerve serve</code> and open the new URL — the
          session token changes every time.
        </p>
        {onRetry ? (
          <button type="button" className="btn" onClick={onRetry}>
            Try again
          </button>
        ) : null}
      </div>
    );
  }

  if (!(error instanceof ApiError)) {
    return (
      <div className="state state--error">
        <div className="state__title">Something in this page failed</div>
        <p className="state__body">{error instanceof Error ? error.message : String(error)}</p>
        {onRetry ? (
          <button type="button" className="btn" onClick={onRetry}>
            Try again
          </button>
        ) : null}
      </div>
    );
  }

  const candidates = shaped<Entity>(detailField(error.detail, 'candidates'));
  const suggestions = shaped<SearchHit>(detailField(error.detail, 'suggestions'));
  const allowed = detailField(error.detail, 'allowed');

  return (
    <div className="state state--error">
      <div className="state__title">
        {error.status} · {error.code}
      </div>
      <p className="state__body wrapany">{error.message}</p>

      {candidates.length > 0 ? (
        <div className="stack" style={{ gap: 8, width: '100%' }}>
          <div className="micro">Pick one</div>
          <ul className="hits">
            {candidates.map((entity) => (
              <li key={entity.entity_id}>
                <a className="hit" href={entityHref(entity.entity_id)}>
                  <span className="hit__kind">{entity.kind}</span>
                  <span className="hit__name truncate">{entity.qualified_name || entity.name}</span>
                  <span className="hit__where">
                    {entity.file_path ?? '—'}
                    {entity.start_line === null ? '' : `:${entity.start_line}`}
                  </span>
                </a>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {suggestions.length > 0 ? (
        <div className="stack" style={{ gap: 8, width: '100%' }}>
          <div className="micro">Did you mean</div>
          <ul className="hits">
            {suggestions.map((hit) => (
              <li key={hit.entity_id}>
                <a className="hit" href={entityHref(hit.entity_id)}>
                  <span className="hit__kind">{hit.kind}</span>
                  <span className="hit__name truncate">
                    {hit.scope_path ? `${hit.scope_path}.` : ''}
                    {hit.name}
                  </span>
                  <span className="hit__where">{hit.file_path ?? '—'}</span>
                </a>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {Array.isArray(allowed) ? (
        <div className="row row--wrap">
          <span className="micro">Accepted</span>
          {allowed.map((value) => (
            <Chip key={String(value)} tone="quiet">
              {String(value)}
            </Chip>
          ))}
        </div>
      ) : null}

      {onRetry ? (
        <button type="button" className="btn" onClick={onRetry}>
          Try again
        </button>
      ) : null}
    </div>
  );
}
