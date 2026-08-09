/**
 * One path's history, and the paths that changed alongside it.
 *
 * These two screens share a text field and a refusal, and both are keyed on a **path as a tree
 * recorded it** — matched literally against stored bytes, never resolved against the working tree,
 * because a historical path routinely does not exist on disk any more. A symbol-shaped selector is
 * refused rather than answered, and the containing file is deliberately not offered instead: a
 * symbol's dates would be its file's dates, which is a different claim in the same words.
 *
 * The first/last-observed block is the reason this screen is careful rather than convenient.
 * `first` is the earliest change Nerve can *see*, and on a shallow clone that is routinely not the
 * first change at all — so the only thing licensed to say "created" is `may_claim_created`, the
 * sentence beside it is the backend's, and the phrase this screen puts on the chip comes from
 * `firstObservedHeadline`, which cannot produce a creation claim from a value alone.
 */

import { useMemo } from 'react';

import type { HistoryCochangeReport, HistoryPathReport, HistoryPathChange } from '../api/types';
import { count } from '../format';
import { cochangeDisclaimer, firstObservedHeadline, historyFragment } from '../history';
import { useApi } from '../hooks';
import { replaceRoute } from '../routing';
import { Chip, Def, Defs, Empty, Loading, Panel } from '../ui/parts';
import { firstObservedGloss, walkTerminationGloss } from '../vocab';
import {
  AnswerScope,
  ChangeLine,
  CommitCard,
  HistoryRefusal,
  RenameList,
  RouteField,
  walkTone,
} from './HistoryParts';

const LIMIT = 50;

/** The field both screens use, plus the note about what a path here means. */
function PathBar({
  tab,
  path,
  options,
}: {
  tab: 'path' | 'cochange';
  path: string;
  options: Record<string, string>;
}) {
  return (
    <section className="panel">
      <div className="graph__controls">
        <RouteField
          label="path"
          placeholder="a path as a tree recorded it, such as src/app.ts"
          value={path}
          wide
          onSettled={(next) => replaceRoute(historyFragment(tab, { ...options, path: next }))}
        />
      </div>
      <div className="panel__body">
        <p className="prose">
          Matched literally against the bytes a tree recorded, and never looked up on disk — a path
          that has since been deleted still has a history, and canonicalising it would refuse every
          one of them. A symbol selector such as{' '}
          <span className="hash">src/app.ts#parse</span> is refused rather than answered.
        </p>
      </div>
    </section>
  );
}

export function HistoryPath({ options }: { options: Record<string, string> }) {
  const path = options['path'] ?? '';
  const params = useMemo(() => ({ path, limit: LIMIT }), [path]);
  const { state, reload } = useApi<HistoryPathReport>(path === '' ? null : '/api/history/path', params);

  return (
    <>
      <PathBar tab="path" path={path} options={options} />

      {path === '' ? (
        <Panel title="One path">
          <Empty
            title="Name a path"
            body="Nerve will report every recorded change to it, the rename hypotheses that name it, and which of six answers its first observation is — including the two that mean the path was never touched in what was read, which are not the same as no history."
          />
        </Panel>
      ) : state.status === 'loading' ? (
        <Loading label="Reading one path" />
      ) : state.status === 'error' ? (
        <HistoryRefusal error={state.error} onRetry={reload} />
      ) : (
        <PathAnswer report={state.data} />
      )}
    </>
  );
}

function PathAnswer({ report }: { report: HistoryPathReport }) {
  const observed = report.first_observed;
  // The permission is the gate and the value is only a lookup for the phrasings that are not a
  // creation. Composing a sentence from `kind` here is exactly what Entry 7 forbids.
  const headline = firstObservedHeadline(observed.kind, observed.may_claim_created);

  return (
    <div className="stack">
      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">First observed</h2>
          <span className="hash wrapany">{report.path}</span>
        </header>
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <div className="row row--wrap">
            <Chip
              tone={observed.may_claim_created ? 'fresh' : 'quiet'}
              title={firstObservedGloss(observed.kind)}
            >
              {headline}
            </Chip>
            <Chip tone="quiet">{observed.kind}</Chip>
            {observed.shallow ? <Chip tone="absent">shallow checkout</Chip> : null}
            {observed.walk_terminated_by === null ? null : (
              <Chip
                tone={walkTone(observed.walk_terminated_by)}
                title={walkTerminationGloss(observed.walk_terminated_by)}
              >
                stopped: {observed.walk_terminated_by}
              </Chip>
            )}
          </div>

          {/* The backend's own sentences. This screen renders them and writes none of its own. */}
          <p className="prose">{observed.kind_note}</p>
          <p className="prose">
            <span className="micro">the word created: </span>
            {observed.may_claim_created_note}
          </p>

          <Defs>
            <Def term="earlier, this path">
              {observed.earlier_history_unavailable === null
                ? 'Nothing is hidden above this path: its earliest recorded change has nothing before it that Nerve failed to reach.'
                : observed.earlier_history_unavailable_note}
            </Def>
            {/*
              A different scope, deliberately side by side. A shallow clone can hold a genuine root
              commit, so a path created at that root has nothing hidden above it while the
              repository still reports that earlier commits may exist. Showing only one would
              "resolve" a disagreement that is not one.
            */}
            <Def term="earlier, this repository">
              {observed.earlier_changes_may_exist
                ? 'The ingest may not have read everything, so earlier commits may exist. This is about the repository, not about this path.'
                : 'The ingest walked to exhaustion, so no earlier commit went unread.'}
            </Def>
            <Def term="changes recorded">
              {count(observed.changes_in_visible_history)} in visible history, of which{' '}
              {count(observed.additions_recorded)}{' '}
              {observed.additions_recorded === 1 ? 'is an addition' : 'are additions'}. More than
              one addition means the path was created more than once, and which came first is a
              question about ancestry that recorded times cannot answer.
            </Def>
            <Def term="merges">
              {count(observed.merges_in_repository)} recorded for this repository. A merge
              enumerates no changes, so a path created inside one has that event unrecorded.
            </Def>
            <Def term="current tree">
              {observed.current_tree.index_exists
                ? `${count(observed.current_tree.entities_at_path)} indexed ${
                    observed.current_tree.entities_at_path === 1 ? 'entity names' : 'entities name'
                  } this path, read from the ${observed.current_tree.basis}.`
                : 'No index has run here, so the current tree could not be consulted at all.'}
            </Def>
          </Defs>
        </div>
      </section>

      <div className="grid2">
        <Endpoint title="Earliest visible change" observed={observed.first} />
        <Endpoint title="Latest visible change" observed={observed.last} />
      </div>

      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">Every recorded change to this path</h2>
          <span className="hash">newest first</span>
        </header>
        <div className="panel__body">
          <AnswerScope block={report} />
        </div>
        {report.commits.length === 0 ? (
          <div className="panel__body">
            <Empty
              title="No commit Nerve read touched this path"
              body={observed.kind_note}
            />
          </div>
        ) : (
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
        )}
      </section>

      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">Rename hypotheses naming this path</h2>
          <span className="hash">
            {count(report.renames_count)}
            {report.renames_truncated ? ' shown, and cut' : ''}
          </span>
        </header>
        <div className="panel__body">
          {/*
            Which matcher the candidate-set record on each row below was looked up under. Several
            matchers may analyse one commit, so a completeness read without the matcher it belongs
            to describes a run that never happened.
          */}
          <div className="row row--wrap">
            <Chip tone="quiet" title="Two kinds of evidence, kept apart. One says two paths named the same bytes; the other says a named method measured how much two different blobs share. They are never blended, summed, or ranked against each other.">
              exact content and similar content are separate claims
            </Chip>
            <Chip tone="quiet" title="The primary key of the analysis table admits several matchers per commit, so the completeness shown on a row belongs to this matcher's run and to no other.">
              similarity analysed by {report.rename_analysis_matcher_id}
            </Chip>
          </div>
        </div>
        {report.renames.length === 0 ? (
          <div className="panel__body">
            <Empty
              title="No hypothesis names this path"
              body="No commit Nerve read deleted a path and added another whose content either matched it byte for byte or measured above the similarity threshold, where one of the two is this one. A move whose file was also heavily rewritten falls below that threshold and leaves no hypothesis — a published false negative, not evidence that nothing moved. A commit whose candidate set exceeded a bound records no hypothesis at all, and each hypothesis below carries which of those its commit was."
            />
          </div>
        ) : (
          <div className="panel__body panel__body--flush">
            <RenameList renames={report.renames} />
          </div>
        )}
      </section>
    </div>
  );
}

/** One end of the observed range — the change itself, beside the commit that made it. */
function Endpoint({ title, observed }: { title: string; observed: HistoryPathChange | null }) {
  if (observed === null) {
    return (
      <Panel title={title}>
        <Empty
          title="No change to show"
          body="No recorded commit touched this path, so there is no earliest or latest change to point at. Which of the four reasons that is, is stated above."
        />
      </Panel>
    );
  }
  return (
    <Panel title={title} flush>
      <div className="panel__body" style={{ display: 'grid', gap: 8 }}>
        <ChangeLine change={observed.change} />
      </div>
      <div className="panel__body panel__body--flush">
        <div className="spine">
          <CommitCard
            commit={observed.commit}
            href={historyFragment('commits', { commit: observed.commit.commit_oid })}
          />
        </div>
      </div>
    </Panel>
  );
}

/**
 * Which paths changed in the same commits as one path.
 *
 * The count is a raw shared-commit count and never a normalised affinity, because a normalised
 * number invites exactly the comparison the label forbids. The disclaimer the API sends is
 * printed verbatim, above the rows rather than under them: two files changing together is equally
 * consistent with coupling, a formatting sweep, a version bump, and one commit that did two
 * unrelated things.
 */
export function HistoryCochange({ options }: { options: Record<string, string> }) {
  const path = options['path'] ?? '';
  const params = useMemo(() => ({ path, limit: LIMIT }), [path]);
  const { state, reload } = useApi<HistoryCochangeReport>(
    path === '' ? null : '/api/history/cochange',
    params,
  );

  return (
    <>
      <PathBar tab="cochange" path={path} options={options} />

      {path === '' ? (
        <Panel title="Changed together">
          <Empty
            title="Name a path"
            body="Nerve will count how many recorded commits changed it and another path at the same time. That count is an observation about commits and nothing else — it is not a dependency, and this screen will not let it be read as one."
          />
        </Panel>
      ) : state.status === 'loading' ? (
        <Loading label="Counting shared commits" />
      ) : state.status === 'error' ? (
        <HistoryRefusal error={state.error} onRetry={reload} />
      ) : (
        <CochangeAnswer report={state.data} />
      )}
    </>
  );
}

function CochangeAnswer({ report }: { report: HistoryCochangeReport }) {
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Changed in the same commits</h2>
        <span className="hash wrapany">{report.path}</span>
      </header>
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        {/*
          The store's own sentence, carried on the response and printed unchanged. A paraphrase
          here would be a second copy of the one line that stops a shared-commit count reading as a
          dependency, and a paraphrase is where it softens.
        */}
        <p className="prose">{cochangeDisclaimer(report)}</p>
        <div className="row row--wrap">
          <Chip tone="quiet">{count(report.pairs_total)} pairs share at least one commit</Chip>
          <Chip
            tone="absent"
            title="A merge enumerates no changes, so no pair is ever observed inside one."
          >
            merges contribute none
          </Chip>
        </div>
        <AnswerScope block={report} />
      </div>
      {report.rows.length === 0 ? (
        <div className="panel__body">
          <Empty
            title="No other path changed in the same commit"
            body="Every recorded commit that touched this path touched nothing else Nerve read. On a repository whose history is shallow or bounded that is a statement about what was read, not about how the repository is written."
          />
        </div>
      ) : (
        <div className="panel__body panel__body--flush">
          <ul className="gaplist">
            {report.rows.map((row) => (
              <li key={`${row.path_a}::${row.path_b}`}>
                <span className="gap">
                  <span className="gap__name wrapany">
                    {row.path_a} · {row.path_b}
                  </span>
                  <span className="gap__meta">
                    {count(row.cochange_observations)}{' '}
                    {row.cochange_observations === 1 ? 'shared commit' : 'shared commits'}
                  </span>
                  <span className="gap__reason">
                    observed changing in the same commit that many times, and nothing more than that
                  </span>
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
