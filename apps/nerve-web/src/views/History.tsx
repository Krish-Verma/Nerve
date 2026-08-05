/**
 * History — what Nerve read of this repository's recorded commits, and what it could not see.
 *
 * The default state of this screen on most repositories is that nothing has been read at all, and
 * that is not an empty result. `nerve history sync` is a separate, deliberate command because it
 * is the only part of this feature that opens `.git`; until it has run, `history_ingested` is
 * `false` and every tally is `null`. Rendering those as zeroes would report "we never looked" as
 * "we looked and there is nothing", which is the same failure the coverage screen exists to avoid
 * one floor down.
 *
 * Five questions, five tabs, one availability block above all of them. The block is not a
 * footnote: every number below it is a floor over *visible* history, bounded by where the walk
 * stopped and by whether the repository declares a boundary of its own — and which of those it
 * was is the difference between a fact about this repository and a fact about Nerve.
 *
 * Ingest is CLI-only and stays that way: `nerve serve` is read-only and proven so on the bytes, so
 * this screen prints the exact command rather than offering a disabled button that implies an
 * implementation is pending.
 */

import { useMemo } from 'react';

import type { HistoryBlock, HistoryCommitDetail, HistoryCommitLog } from '../api/types';
import { count } from '../format';
import { HISTORY_TABS, historyFragment, type HistoryTab } from '../history';
import { useApi } from '../hooks';
import { href } from '../routing';
import { Empty, Loading, Panel } from '../ui/parts';
import { HistoryCochange, HistoryPath } from './HistoryPath';
import { HistoryDiff, HistoryFrequency } from './HistoryDiff';
import {
  AnswerScope,
  Availability,
  ChangeList,
  CommitCard,
  HistoryRefusal,
} from './HistoryParts';

const TAB_LABEL: Record<HistoryTab, string> = {
  commits: 'Commits',
  path: 'One path',
  diff: 'Between two states',
  frequency: 'Change frequency',
  cochange: 'Changed together',
};

/** How many commits one page of the log asks for. The server clamps and reports what it applied. */
const PAGE = 50;

export function History({ tab, options }: { tab: HistoryTab; options: Record<string, string> }) {
  const availability = useApi<HistoryBlock>('/api/history');
  const block = availability.state.status === 'ready' ? availability.state.data : null;

  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">History</h1>
        <p className="head__sub">
          What Nerve read of this repository&apos;s commits, and what that reading could not reach.
          Every count here is over visible history — the commits one ingest walked — so each is a
          floor rather than a total, and where the walk stopped is stated rather than implied.
        </p>
      </div>

      <nav className="tabs" aria-label="History questions">
        {HISTORY_TABS.map((name) => (
          <a
            key={name}
            className="tab"
            href={href({ view: 'history', tab: name, options: {} })}
            aria-current={name === tab ? 'page' : undefined}
          >
            {TAB_LABEL[name]}
          </a>
        ))}
      </nav>

      <div className="stack">
        {availability.state.status === 'loading' ? (
          <Loading label="Asking what history is available" />
        ) : availability.state.status === 'error' ? (
          <HistoryRefusal error={availability.state.error} onRetry={availability.reload} />
        ) : (
          <Availability block={availability.state.data} />
        )}

        {block === null ? null : block.history_ingested ? (
          <Answer tab={tab} options={options} />
        ) : (
          <NeverRead />
        )}
      </div>
    </div>
  );
}

function Answer({ tab, options }: { tab: HistoryTab; options: Record<string, string> }) {
  if (tab === 'path') return <HistoryPath options={options} />;
  if (tab === 'cochange') return <HistoryCochange options={options} />;
  if (tab === 'diff') return <HistoryDiff options={options} />;
  if (tab === 'frequency') return <HistoryFrequency />;
  return <CommitLog options={options} />;
}

/**
 * The state that is not an empty list.
 *
 * "This repository has no commits" and "nobody has told Nerve to read them" are different answers,
 * and a log showing nothing would render the second as the first. So no list is rendered at all,
 * and the command that changes the situation is printed exactly.
 */
function NeverRead() {
  return (
    <section className="panel">
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <p style={{ fontSize: 17, lineHeight: 1.45 }}>
          No history has been read for this repository.
        </p>
        <p className="prose">
          That is an absence rather than a finding. This screen shows no commits, no dates and no
          tallies — not even zeroes — because listing nothing would say this repository has no
          history, and nothing here has established that either way.
        </p>
        <div className="gate__sample">$ nerve history sync</div>
        <p className="prose">
          Reading history is the one part of this feature that opens <code>.git</code>, so it is a
          command you run rather than a button here: <code>nerve serve</code> is read-only and is
          proven so on the database bytes. Run it and reload this page. It exits 3 when anything was
          refused, so a partial read is distinguishable from a complete one at the shell.
        </p>
      </div>
    </section>
  );
}

/** The recorded log, newest committer time first — or one commit, when the route names one. */
function CommitLog({ options }: { options: Record<string, string> }) {
  const chosen = options['commit'] ?? '';
  const offset = Number(options['offset'] ?? '0') || 0;

  const params = useMemo(() => ({ limit: PAGE, offset }), [offset]);
  const { state, reload } = useApi<HistoryCommitLog>(chosen === '' ? '/api/history/commits' : null, params);

  if (chosen !== '') return <CommitChanges oid={chosen} />;
  if (state.status === 'loading') return <Loading label="Reading the recorded log" />;
  if (state.status === 'error') return <HistoryRefusal error={state.error} onRetry={reload} />;

  const report = state.data;
  if (report.commits.length === 0) {
    return (
      <Panel title="Commit log">
        <Empty
          title="This page of the log is empty"
          body="History has been read here, and this offset holds no commits. Check the availability block above for how many were recorded — an offset past the end is an ordinary thing to ask for and an ordinary thing to answer."
        />
      </Panel>
    );
  }

  const continuation = report.continuation;
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Commit log</h2>
        <span className="hash">newest committer time first</span>
      </header>
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <AnswerScope block={report} />
        <p className="prose">
          Ordered by the time each commit records, which is not an ancestry order: a rebase or a
          fabricated clock can reorder these freely. Open one to see what it changed.
        </p>
      </div>
      <div className="panel__body panel__body--flush">
        <div className="spine">
          {report.commits.map((commit) => (
            <CommitCard
              key={commit.commit_oid}
              commit={commit}
              href={historyFragment('commits', { commit: commit.commit_oid })}
            />
          ))}
        </div>
      </div>
      <div className="graph__foot">
        <a
          className={offset === 0 ? 'btn btn--ghost' : 'btn'}
          aria-disabled={offset === 0 ? 'true' : undefined}
          href={
            offset === 0
              ? historyFragment('commits', {})
              : historyFragment('commits', { offset: String(Math.max(0, offset - PAGE)) })
          }
        >
          previous
        </a>
        <span>
          {count(offset + 1)}–{count(offset + report.commits.length)}
          {report.truncation === null || report.truncation.total === null
            ? ''
            : ` of ${count(report.truncation.total)}`}
        </span>
        <a
          className={continuation.next_offset === null ? 'btn btn--ghost' : 'btn'}
          aria-disabled={continuation.next_offset === null ? 'true' : undefined}
          href={
            continuation.next_offset === null
              ? historyFragment('commits', { offset: String(offset) })
              : historyFragment('commits', { offset: String(continuation.next_offset) })
          }
        >
          next
        </a>
      </div>
    </section>
  );
}

/**
 * What one commit did.
 *
 * A commit that is not recorded is a refusal, never an empty change list, and the refusal says so
 * in a field of its own. An empty list that *is* an answer still means one of four things, and
 * which one is on the commit as `changes_enumerated` — printed beside the count on every card.
 */
function CommitChanges({ oid }: { oid: string }) {
  const params = useMemo(() => ({ commit: oid }), [oid]);
  const { state, reload } = useApi<HistoryCommitDetail>('/api/history/commit', params);

  const back = (
    <a className="link" href={historyFragment('commits', {})}>
      back to the log
    </a>
  );

  if (state.status === 'loading') return <Loading label="Reading one commit" />;
  if (state.status === 'error') {
    return (
      <div className="stack">
        <HistoryRefusal error={state.error} onRetry={reload} />
        <div>{back}</div>
      </div>
    );
  }

  const report = state.data;
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">One commit</h2>
        {back}
      </header>
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <CommitCard commit={report.commit} />
        <AnswerScope block={report} />
      </div>
      <div className="panel__body panel__body--flush">
        {report.changes.length === 0 ? (
          <div className="panel__body">
            <Empty
              title="No change was listed for this commit"
              body={report.commit.changes_enumerated_note}
            />
          </div>
        ) : (
          <ChangeList changes={report.changes} />
        )}
      </div>
      {report.commit.parent_oids.length === 0 ? null : (
        <div className="panel__body">
          <p className="prose">
            Compared against{' '}
            {report.commit.parent_oids.map((parent) => (
              <span key={parent} className="hash wrapany">
                {parent.slice(0, 12)}{' '}
              </span>
            ))}
            {report.commit.is_merge
              ? '— several parents, so nothing was enumerated against any of them.'
              : '— the single parent a change is defined against.'}
          </p>
        </div>
      )}
    </section>
  );
}
