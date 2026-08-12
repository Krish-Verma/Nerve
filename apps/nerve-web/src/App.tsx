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
import { Contracts } from './views/Contracts';
import { Coverage } from './views/Coverage';
import { Entity } from './views/Entity';
import { History } from './views/History';
import { Impact } from './views/Impact';
import { Memory } from './views/Memory';
import { Overview } from './views/Overview';
import { Search } from './views/Search';
import { Unresolved } from './views/Unresolved';

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
            note={data ? count(data.symbols_total) : undefined}
          />
        </div>

        {/*
          Two entries, not one, and the split is the point. "Unresolved" is what *Nerve* could not
          work out; "Coverage" is what the repository's tests are not known to touch. Both were
          once called Gaps, which meant a reader could ask one question and be answered the other.
        */}
        <div className="rail__group">
          <div className="rail__label micro">what is missing</div>
          <RailLink
            to={{ view: 'unresolved' }}
            current={route.view === 'unresolved'}
            label="Unresolved"
            note={data ? count(data.unresolved_entities) : undefined}
          />
          <RailLink
            to={{ view: 'coverage' }}
            current={route.view === 'coverage'}
            label="Coverage"
          />
        </div>

        {/*
          Grouped with "what is missing" rather than with the index, and the placement is an
          argument. An impact answer is mostly a number of dependants, but the half that changes a
          decision is the count of reference sites that resolved to nothing and could be hiding one
          more. It sits beside Unresolved because it is the same fact asked from the other end.

          No count beside it, because there is nothing to count until a symbol has been named.
        */}
        <div className="rail__group">
          <div className="rail__label micro">before you change something</div>
          <RailLink
            to={{ view: 'impact', options: {} }}
            current={route.view === 'impact'}
            label="Impact"
          />
        </div>

        {/*
          Its own group, because it answers a different kind of question from either of the two
          above. Those are about the index as it stands; this one is about what the repository has
          been doing, over whatever slice of its history somebody told Nerve to read. There is no
          count beside it on purpose — the number of commits read is meaningless without the
          statement of where the reading stopped, and that statement is on the screen itself.
        */}
        <div className="rail__group">
          <div className="rail__label micro">what changed</div>
          <RailLink
            to={{ view: 'history', tab: 'commits', options: {} }}
            current={route.view === 'history'}
            label="History"
          />
        </div>

        {/*
          Its own group, and the only screen in this interface that is about a repository other
          than this one. There is no count beside it for the same reason History has none: a number
          of links means nothing without the statement of how many of them still describe the
          world, and that statement is on the screen itself.
        */}
        <div className="rail__group">
          <div className="rail__label micro">other repositories</div>
          <RailLink
            to={{ view: 'contracts', tab: 'links', options: {} }}
            current={route.view === 'contracts'}
            label="Contracts"
          />
        </div>

        {/*
          Its own group, and the only screen here whose contents no extractor produced. Everything
          else in this rail was read off the repository; a note exists because a person typed one.
          There is no count beside it for the same reason History and Contracts have none — a
          number of notes says nothing without how many still describe the code, and that is
          decided when the notes are read rather than stored anywhere a rail could ask.
        */}
        <div className="rail__group">
          <div className="rail__label micro">what a person wrote</div>
          <RailLink
            to={{ view: 'memory', options: {} }}
            current={route.view === 'memory'}
            label="Memory"
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
        ) : route.view === 'coverage' ? (
          <Coverage />
        ) : route.view === 'history' ? (
          <History tab={route.tab} options={route.options} />
        ) : route.view === 'contracts' ? (
          <Contracts tab={route.tab} options={route.options} />
        ) : route.view === 'memory' ? (
          <Memory options={route.options} />
        ) : route.view === 'impact' ? (
          <Impact options={route.options} />
        ) : (
          <Unresolved />
        )}
      </main>
    </div>
  );
}
