/**
 * Two answers about ranges rather than about one thing: what lies between two recorded states,
 * and which paths changed most often in what was read.
 *
 * The diff is by **ancestry and never by a time range**, and the distinction is not pedantry: a
 * merge brings in commits whose recorded time precedes it, and a rebase reorders them freely, so a
 * time window answers a different question and fails silently when it does.
 *
 * Five outcomes, four of which are refusals — and none of the four is an empty diff. The API
 * carries every diff-shaped key as `null` on those four, so `commits: null` means no range was
 * computed and `commits: []` means one was and holds nothing. This screen branches on
 * `diffReading`, which is the one place that distinction is made, so that treating them alike
 * would take an edit rather than an oversight.
 */

import { useMemo } from 'react';

import type { HistoryDiffReport, HistoryFrequencyReport } from '../api/types';
import { count } from '../format';
import { diffReading, historyFragment } from '../history';
import { useApi } from '../hooks';
import { replaceRoute } from '../routing';
import { Chip, Def, Defs, Empty, Loading, Panel, Tally } from '../ui/parts';
import { changesEnumeratedGloss, parentCompletenessGloss } from '../vocab';
import { AnswerScope, ChangeList, CommitCard, HistoryRefusal, RouteField } from './HistoryParts';

const LIMIT = 50;

/** What each refusal means for the reader, beside the sentence the response carries. */
const REFUSAL_READING: Record<string, string> = {
  state_not_recorded:
    'One of the two oids is not a commit this ingest recorded, so there was nothing to walk between. Whether each was recorded is stated below.',
  not_an_ancestor:
    'The first oid is not an ancestor of the second, so there is no range from one to the other. That is a shape of question this data cannot answer, not an empty answer.',
  ancestry_incomplete:
    'The walk reached a commit whose parents could not be followed before it reached the first oid, so any range reported would have been a fragment presented as a whole.',
  walk_budget_exhausted:
    'The ancestry pass hit its own ceiling. Nerve stopped; the repository did not, and a partial ancestor set would prune the range wrongly rather than narrowly.',
};

export function HistoryDiff({ options }: { options: Record<string, string> }) {
  const from = options['from'] ?? '';
  const to = options['to'] ?? '';
  const params = useMemo(() => ({ from, to, limit: LIMIT }), [from, to]);
  const ready = from !== '' && to !== '';
  const { state, reload } = useApi<HistoryDiffReport>(ready ? '/api/history/diff' : null, params);

  return (
    <>
      <section className="panel">
        <div className="graph__controls">
          <RouteField
            label="from"
            placeholder="a recorded commit oid, excluded from the range"
            value={from}
            onSettled={(next) => replaceRoute(historyFragment('diff', { ...options, from: next }))}
          />
          <RouteField
            label="to"
            placeholder="a recorded commit oid, included in the range"
            value={to}
            onSettled={(next) => replaceRoute(historyFragment('diff', { ...options, to: next }))}
          />
        </div>
        <div className="panel__body">
          <p className="prose">
            Walked by ancestry, not by date. A merge brings in commits recorded as older than
            itself and a rebase reorders them freely, so a time window would answer a different
            question and look like this one while doing it. Pick oids from the commit log.
          </p>
        </div>
      </section>

      {!ready ? (
        <Panel title="Between two states">
          <Empty
            title="Name both ends"
            body="The first oid is excluded from the range and the second is included. Both have to be commits this ingest recorded — an oid Nerve never read is refused rather than treated as an empty range."
          />
        </Panel>
      ) : state.status === 'loading' ? (
        <Loading label="Walking ancestry between two states" />
      ) : state.status === 'error' ? (
        <HistoryRefusal error={state.error} onRetry={reload} />
      ) : (
        <DiffAnswer report={state.data} />
      )}
    </>
  );
}

function DiffAnswer({ report }: { report: HistoryDiffReport }) {
  const reading = diffReading(report);

  if (reading.outcome === 'refused') {
    return (
      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">No range was computed</h2>
          <span className="hash">{reading.kind}</span>
        </header>
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <p style={{ fontSize: 17, lineHeight: 1.45 }}>
            This is not an empty diff. Nothing was compared.
          </p>
          <p className="prose">
            {REFUSAL_READING[reading.kind] ??
              'This build has no reading for that outcome. Nothing was compared either way — an absent range is never the same answer as an empty one.'}
          </p>
          <Defs>
            <Def term="from">
              <span className="hash wrapany">{report.from}</span>
              {report.from_recorded === null
                ? ''
                : report.from_recorded
                  ? ' · recorded'
                  : ' · not a recorded commit'}
            </Def>
            <Def term="to">
              <span className="hash wrapany">{report.to}</span>
              {report.to_recorded === null
                ? ''
                : report.to_recorded
                  ? ' · recorded'
                  : ' · not a recorded commit'}
            </Def>
            {report.commits_walked === null ? null : (
              <Def term="commits walked">
                {count(report.commits_walked)}
                {report.walk_limit === null ? '' : ` against a ceiling of ${count(report.walk_limit)}`}
              </Def>
            )}
            {report.stopped_at === null ? null : (
              <Def term="stopped at">
                <span className="hash wrapany">{report.stopped_at}</span>
                {report.stopped_at_parent_completeness === null ? null : (
                  <>
                    {' · '}
                    <span title={parentCompletenessGloss(report.stopped_at_parent_completeness)}>
                      {report.stopped_at_parent_completeness}
                    </span>
                  </>
                )}
                {report.stopped_at_parent_completeness_note === null ? null : (
                  <div className="head__sub wrapany">
                    {report.stopped_at_parent_completeness_note}
                  </div>
                )}
              </Def>
            )}
          </Defs>
        </div>
      </section>
    );
  }

  const enumerated = Object.entries(report.changes_enumerated ?? {}).sort(
    (a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1),
  );

  return (
    <div className="stack">
      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">The range</h2>
          <span className="hash wrapany">
            {report.from.slice(0, 12)} → {report.to.slice(0, 12)}
          </span>
        </header>
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <div className="row row--wrap">
            <Chip tone="quiet">
              {count(reading.commitsInRange)}{' '}
              {reading.commitsInRange === 1 ? 'commit' : 'commits'} in the range
            </Chip>
            {reading.mergesInRange > 0 ? (
              <Chip tone="absent" title={changesEnumeratedGloss('merge_not_enumerated')}>
                {count(reading.mergesInRange)} of them merges
              </Chip>
            ) : null}
            {reading.changesTruncated ? (
              <Chip tone="stale">the change list is not all of them</Chip>
            ) : null}
          </div>
          <AnswerScope block={report} />
          {reading.mergesInRange > 0 ? (
            <p className="prose">
              A merge enumerates no changes, so a range holding merges carries fewer change rows
              than commits by design. Few changes here is expected rather than quiet.
            </p>
          ) : null}
          {report.ancestry_incomplete_at === null ? null : (
            <p className="prose">
              One commit inside this range had parents that could not be followed —{' '}
              <span className="hash wrapany">
                {report.ancestry_incomplete_at.commit_oid.slice(0, 12)}
              </span>
              . {report.ancestry_incomplete_at.parent_completeness_note} The range is therefore a
              floor rather than the whole of it.
            </p>
          )}
          {enumerated.length === 0 ? null : (
            <Tally rows={enumerated} />
          )}
        </div>
      </section>

      {reading.commits.length === 0 ? (
        <Panel title="Commits in the range">
          <Empty
            title="A range was computed and it holds nothing"
            body="These two states have no commit between them. This is the empty answer, and it is a different answer from the four refusals — those compute no range at all and say so."
          />
        </Panel>
      ) : (
        <Panel title="Commits in the range" flush>
          <div className="spine">
            {reading.commits.map((commit) => (
              <CommitCard
                key={commit.commit_oid}
                commit={commit}
                href={historyFragment('commits', { commit: commit.commit_oid })}
              />
            ))}
          </div>
        </Panel>
      )}

      {reading.changes.length === 0 ? null : (
        <Panel title="What those commits changed" flush>
          <ChangeList changes={reading.changes} showCommit />
        </Panel>
      )}
    </div>
  );
}

/**
 * Which paths changed most often in what was read.
 *
 * Every count is a floor twice over: over visible history rather than over the repository's whole
 * log, and short by whatever the merges did, because a merge enumerates no changes at all.
 */
export function HistoryFrequency() {
  const params = useMemo(() => ({ limit: LIMIT }), []);
  const { state, reload } = useApi<HistoryFrequencyReport>('/api/history/frequency', params);

  if (state.status === 'loading') return <Loading label="Counting changes per path" />;
  if (state.status === 'error') return <HistoryRefusal error={state.error} onRetry={reload} />;

  const report = state.data;
  if (report.rows.length === 0) {
    return (
      <Panel title="Change frequency">
        <Empty
          title="No path has a recorded change"
          body="History has been read here and no commit in it enumerated a change. Check the availability block: a walk made entirely of merges, or one that stopped at a boundary, records commits without recording what they did."
        />
      </Panel>
    );
  }

  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Change frequency</h2>
        <span className="hash">{count(report.paths_total)} paths have a recorded change</span>
      </header>
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <AnswerScope block={report} />
        <p className="prose">
          Commits that touched each path, most first. A floor rather than a lifetime count: it is
          over the commits this ingest read, and a merge enumerates no changes, so whatever the
          merges did is missing from every row.
        </p>
        <Tally rows={report.rows.map((row) => [row.path, row.commits])} />
      </div>
    </section>
  );
}
