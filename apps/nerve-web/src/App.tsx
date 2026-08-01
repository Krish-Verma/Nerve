/**
 * The shell.
 *
 * Three fixed regions: a bar that is always the way in (search), a rail that is always the way
 * around, and one scrolling region for whatever is being read. The rail carries the counts that
 * change how everything else should be read — how much is unresolved, whether the index is still
 * current — because those are the facts a reader needs *before* they trust a screen, not after.
 *
 * The overview is fetched once, here, and handed to the view that renders it. Freshness is
 * measured by re-hashing files on the server, so asking for it twice on one screen would be
 * asking the machine to do real work to tell us something we already knew.
 */

import { useEffect } from 'react';

import { sessionToken } from './api/client';
import type { Overview as OverviewData } from './api/types';
import { count } from './format';
import { useApi } from './hooks';
import { href, useRoute, type Route } from './routing';
import { Omnibox } from './ui/Omnibox';
import { Entity } from './views/Entity';
import { Gaps } from './views/Gaps';
import { Overview } from './views/Overview';
import { Search } from './views/Search';

function basename(path: string | null | undefined): string {
  if (!path) return 'this repository';
  const parts = path.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/**
 * What the page shows when it was opened without the token.
 *
 * This is not an error state — it is the normal consequence of bookmarking the page, because the
 * token dies with the process that printed it. So it explains rather than complains.
 */
function Gate() {
  return (
    <div className="gate">
      <div className="gate__card">
        <div className="bar__mark">
          <Mark />
          <span className="bar__word">nerve</span>
        </div>
        <h1 className="gate__title">This page has no session token</h1>
        <p className="state__body">
          Every request to the index has to carry a token that is generated when{' '}
          <code>nerve serve</code> starts, is never written to disk, and dies with the process. The
          address bar has none, so this tab cannot read anything.
        </p>
        <p className="state__body">
          Start the server and open the address it prints. A bookmark will not work twice — the
          token is different every run.
        </p>
        <div className="gate__sample">
          $ nerve serve .<br />
          http://127.0.0.1:PORT/?token=…
        </div>
      </div>
    </div>
  );
}

/** The mark: a signal crossing a junction. Drawn, not imported — no icon package ships here. */
function Mark() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true" focusable="false">
      <path
        d="M1 13 L6 13 L9 4 L12 13 L17 13"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="square"
      />
      <circle cx="9" cy="4" r="1.9" fill="currentColor" />
    </svg>
  );
}

function RailLink({ to, current, label, note }: {
  to: Route;
  current: boolean;
  label: string;
  note?: string;
}) {
  return (
    <a className="rail__link" href={href(to)} aria-current={current ? 'page' : undefined}>
      {label}
      {note === undefined ? null : <span className="rail__count num">{note}</span>}
    </a>
  );
}

export function App() {
  const [route] = useRoute();
  const overview = useApi<OverviewData>(sessionToken() === null ? null : '/api/overview');
  const data = overview.state.status === 'ready' ? overview.state.data : null;

  const name = basename(data?.root_path);
  useEffect(() => {
    document.title = data ? `${name} · nerve` : 'nerve';
  }, [data, name]);

  if (sessionToken() === null) return <Gate />;

  const drifted =
    data?.freshness && data.freshness.stale + data.freshness.missing > 0
      ? data.freshness.stale + data.freshness.missing
      : 0;

  return (
    <div className="shell">
      <header className="bar">
        <a className="bar__mark" href={href({ view: 'overview' })} aria-label="Nerve overview">
          <Mark />
          <span className="bar__word">nerve</span>
        </a>
        <div className="bar__search">
          <Omnibox initial={route.view === 'search' ? route.q : ''} />
        </div>
        <span className="spacer" />
        <div className="bar__meta">
          <span className="hash truncate" title={data?.root_path ?? ''}>
            {name}
          </span>
        </div>
      </header>

      <nav className="rail" aria-label="Sections">
        <div className="rail__group">
          <div className="rail__label micro">the index</div>
          <RailLink
            to={{ view: 'overview' }}
            current={route.view === 'overview'}
            label="Overview"
          />
          <RailLink
            to={{ view: 'search', q: '', kind: null }}
            current={route.view === 'search' || route.view === 'entity'}
            label="Symbols"
            note={data ? count(data.entities_total) : undefined}
          />
        </div>

        <div className="rail__group">
          <div className="rail__label micro">what is missing</div>
          <RailLink
            to={{ view: 'gaps' }}
            current={route.view === 'gaps'}
            label="Gaps"
            note={data ? count(data.unresolved_entities) : undefined}
          />
        </div>

        {data ? (
          <div className="rail__focus">
            <div className="micro">index state</div>
            <div className="fact__value">
              {drifted > 0 ? `${count(drifted)} files have drifted` : 'every file still matches'}
            </div>
            <div className="hash">
              {count(data.assertions_total)} assertions · {count(data.observations_total)}{' '}
              observations
            </div>
          </div>
        ) : null}
      </nav>

      <main className="main">
        {route.view === 'overview' ? (
          <Overview resource={overview} />
        ) : route.view === 'search' ? (
          <Search q={route.q} kind={route.kind} />
        ) : route.view === 'entity' ? (
          <Entity id={route.id} tab={route.tab} options={route.options} />
        ) : (
          <Gaps />
        )}
      </main>
    </div>
  );
}
