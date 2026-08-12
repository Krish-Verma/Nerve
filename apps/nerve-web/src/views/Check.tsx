/**
 * Trust — whether anything else in this interface may be believed.
 *
 * `nerve check` has answered this since Slice 7c-i and only a shell could ask. Every other screen
 * here could render a careful, well-qualified answer drawn from a graph built against a tree that
 * has since moved on, and nothing on the page said so. This screen is that answer, and one rule
 * decides its whole shape.
 *
 * **There are five verdicts and a two-state screen would be a lie.** `stale` and `unverified` mean
 * the same thing to a reader — do not rely on this index — and they rest on opposite evidence.
 * `stale` is a *measurement*: a file changed, a file the index describes is gone, or a file exists
 * that no row describes. `unverified` is the *absence* of a measurement: part of the tree was never
 * looked at. Nothing was observed to have changed. The command line gives them one exit code
 * because a shell has one way to say "do not proceed"; a screen has room to say which, so the two
 * get different hues, different headings, different glosses, and the two families of counts are
 * drawn as **two panels** that are never added together.
 *
 * Three more rules, each a way to be wrong while looking right:
 *
 * - **The added-file panel is not an afterthought.** The freshness sweep walks the index's own
 *   cache, so a file added since the last index has no row to compare and is invisible to it — a
 *   repository can grow a hundred modules with every recorded hash still matching. The separate
 *   walk is what sees it, and the panel says so on every answer including the one where the count
 *   is zero.
 * - **A null tally is not a zero tally.** No sweep ran and a sweep that found nothing are opposite
 *   findings, and the panels say which rather than printing zeroes.
 * - **The remedy is a command, not a control.** This API is read-only and proved so on the database
 *   bytes; re-indexing writes. So the screen prints `nerve index` the way the memory views print
 *   theirs, and there is no disabled button implying an implementation is pending.
 *
 * The repository root, the added paths and the reason are repository content. All of it is
 * interpolated as a React child, which React escapes. Nothing here builds markup from a string.
 */

import type { TrustEvidence, TrustReport, TrustSweep, TrustTree } from '../api/types';
import {
  evidenceSplitReading,
  familyTone,
  notEstablishedRows,
  observedRows,
  sweepReading,
  treeReading,
  unindexableReading,
  verdictEvidenceKind,
  verdictHeading,
  verdictTone,
} from '../check';
import { count } from '../format';
import { useApi } from '../hooks';
import { Chip, Def, Defs, Failure, Figure, Loading, Panel, Tally, Where } from '../ui/parts';
import { indexVerdictGloss } from '../vocab';

export function Check() {
  const { state, reload } = useApi<TrustReport>('/api/check');

  if (state.status === 'loading') {
    return <Loading label="Re-reading the repository to judge the index" />;
  }
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;
  return <Report report={state.data} />;
}

/**
 * The verdict itself, above everything else on the page.
 *
 * Both sentences are rendered: the vocabulary's own `verdict_note`, which is the server's and is
 * never paraphrased here, and this interface's gloss, which says what the value means to a reader
 * who has not read the schema. The measured particulars are the third line and are `reason`.
 */
function Verdict({ report }: { report: TrustReport }) {
  const tone = verdictTone(report.verdict);
  const kind = verdictEvidenceKind(report.verdict);

  return (
    <section className="panel">
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <div className="row row--wrap">
          <Chip tone={tone}>
            <span className="chip__dot" />
            {report.verdict}
          </Chip>
          <Chip tone={report.trustworthy ? 'fresh' : 'quiet'}>
            {report.trustworthy ? 'answers describe the tree as it is' : 'other answers need a caveat'}
          </Chip>
          {kind === 'not-established' ? (
            <Chip tone="unknown" prose>
              nothing was observed to change — part of the tree was never compared
            </Chip>
          ) : null}
          <span className="spacer" />
          <span className="head__sub">measured on this request, by reading the repository again</span>
        </div>
        <p style={{ fontSize: 17, lineHeight: 1.45 }}>{verdictHeading(report.verdict)}</p>
        <p className="prose">{indexVerdictGloss(report.verdict)}</p>
        <p className="hash wrapany">{report.reason}</p>
        <p className="prose">{report.verdict_note}</p>
      </div>
    </section>
  );
}

/**
 * The two families of counts, side by side and never summed.
 *
 * Both panels are drawn on every answer, including the ones where a family is empty. An absent
 * panel is indistinguishable from a panel nobody wrote, and the zero case is exactly where silence
 * invites the wrong reading: "nothing unchecked" is a real statement and has to be made.
 */
function Evidence({ evidence }: { evidence: TrustEvidence }) {
  const observed = evidence.observed;
  const unchecked = evidence.not_established;

  return (
    <>
      <p className="prose">{evidenceSplitReading(evidence)}</p>
      <div className="grid2">
        <Panel
          title="Divergence that was measured"
          aside={
            <Chip tone={familyTone(observed?.total ?? 0, 'observed')}>
              {observed === null ? 'not measured' : `${count(observed.total)} observed`}
            </Chip>
          }
        >
          {observed === null ? (
            <p className="state__body">
              No sweep ran, so there is no tally here. That is the absence of a measurement rather
              than a measurement of nothing.
            </p>
          ) : (
            <div className="stack" style={{ gap: 10 }}>
              <Tally rows={observedRows(observed)} />
              <p className="hash wrapany">
                Each of these is a file that was looked at. A count here is evidence that the index
                and the repository disagree.
              </p>
            </div>
          )}
        </Panel>

        <Panel
          title="Repository that was never compared"
          aside={
            <Chip tone={familyTone(unchecked?.total ?? 0, 'not-established')}>
              {unchecked === null ? 'not measured' : `${count(unchecked.total)} unchecked`}
            </Chip>
          }
        >
          {unchecked === null ? (
            <p className="state__body">
              No sweep ran, so nothing here was skipped either. There was nothing to skip.
            </p>
          ) : (
            <div className="stack" style={{ gap: 10 }}>
              <Tally rows={notEstablishedRows(unchecked)} />
              <p className="hash wrapany">
                None of these is a change. They are files nobody looked at, which is why a verdict
                that rests on them is <strong>unverified</strong> rather than <strong>stale</strong>
                {' '}— reporting them as staleness would claim a change nothing observed.
              </p>
              {unchecked.truncated ? (
                <Chip tone="unknown" prose>
                  the sweep stopped at its cap, so the files past it were never reached
                </Chip>
              ) : null}
            </div>
          )}
        </Panel>
      </div>
    </>
  );
}

/** What was re-hashed. `null` is a sentence rather than a row of zeroes. */
function Sweep({ sweep }: { sweep: TrustSweep | null }) {
  return (
    <Panel
      title="What was re-hashed"
      aside={
        sweep?.truncated ? (
          <Chip tone="unknown">
            partial sweep · {count(sweep.files_probed)} of {count(sweep.files_total)}
          </Chip>
        ) : (
          <span className="hash">{sweep === null ? 'no sweep' : `${count(sweep.files_probed)} files`}</span>
        )
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <p className="prose">{sweepReading(sweep)}</p>
        {sweep === null ? null : (
          <div className="figures">
            <Figure
              label="still match"
              value={count(sweep.fresh)}
              note="Re-read and hashing to what was extracted."
            />
            <Figure
              label="changed"
              value={count(sweep.stale)}
              note="The bytes are not what they were."
              unknown={sweep.stale > 0}
            />
            <Figure
              label="gone"
              value={count(sweep.missing)}
              note="The index still describes a file that is not there."
              unknown={sweep.missing > 0}
            />
            <Figure
              label="not compared"
              value={count(sweep.refused + sweep.unreadable)}
              note="Refused by the path check, or the bytes would not come."
              unknown={sweep.refused + sweep.unreadable > 0}
            />
            <Figure
              label="sweep cap"
              value={count(sweep.probe_cap)}
              note="Files re-hashed before a partial sweep is reported."
            />
          </div>
        )}
      </div>
    </Panel>
  );
}

/**
 * What the repository holds that the index has never seen.
 *
 * Its own panel, and the panel exists on every answer. This is the measurement the sweep above
 * cannot make, and an index that grew a hundred modules would report every recorded hash as
 * matching without it.
 */
function Untracked({ tree }: { tree: TrustTree | null }) {
  return (
    <Panel
      title="What the index has never seen"
      aside={
        <Chip tone={tree === null ? 'quiet' : tree.added > 0 ? 'stale' : 'fresh'}>
          {tree === null ? 'not walked' : `${count(tree.added)} untracked`}
        </Chip>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <p className="prose">{treeReading(tree)}</p>
        {tree === null || tree.added_paths.length === 0 ? null : (
          <div className="stack" style={{ gap: 6 }}>
            <div className="micro">files a re-index would add</div>
            {tree.added_paths.map((path) => (
              <Where key={path} path={path} />
            ))}
            {tree.added_paths_truncated ? (
              <p className="hash wrapany">
                {count(tree.added_paths_returned)} of {count(tree.added)} are named. The count is
                exact whatever the list was cut to.
              </p>
            ) : null}
          </div>
        )}
        {unindexableReading(tree) === '' ? null : (
          <p className="hash wrapany">{unindexableReading(tree)}</p>
        )}
      </div>
    </Panel>
  );
}

/**
 * The remedy, printed as the command it actually is.
 *
 * Not a button. This API is read-only — one `PRAGMA query_only` connection per worker, every route
 * a `GET`, proved on the database bytes — and re-indexing writes. A control that cannot work
 * implies an implementation is pending, and none is.
 */
function Remedy({ report }: { report: TrustReport }) {
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">{report.remedy.required ? 'Bringing this up to date' : 'Keeping this up to date'}</h2>
        <span className="hash">command line only</span>
      </header>
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <p className="prose">{report.remedy.statement}</p>
        <div className="gate__sample">
          {report.boundary.commands.map((command) => (
            <div key={command}>$ {command}</div>
          ))}
        </div>
        <p className="prose">{report.boundary.statement}</p>
      </div>
    </section>
  );
}

function Report({ report }: { report: TrustReport }) {
  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">Can this index be trusted?</h1>
        <p className="head__sub wrapany">
          {report.repository.root_path ?? 'no repository root recorded'}
        </p>
      </div>

      <div className="stack">
        <Verdict report={report} />
        <Evidence evidence={report.evidence} />
        <Sweep sweep={report.sweep} />
        <Untracked tree={report.tree} />

        <div className="grid2">
          <Panel title="The index itself">
            <Defs>
              <Def term="schema">
                v{report.schema.version ?? '—'}
                <span className="hash"> · this build supports v{report.schema.supported_version}</span>
              </Def>
              <Def term="readable">
                <Chip tone={report.schema.readable ? 'fresh' : 'absent'}>
                  {report.schema.readable ? 'yes' : 'no'}
                </Chip>
              </Def>
              <Def term="runs still open">
                <span className={report.runs_running > 0 ? 'figure__value--unknown' : ''}>
                  {count(report.runs_running)}
                </span>
              </Def>
              <Def term="state id">
                <span className="hash">{report.repository.state_id ?? 'not recorded'}</span>
              </Def>
              <Def term="git commit">
                <span className="hash">{report.repository.git_commit ?? 'not a git checkout'}</span>
              </Def>
            </Defs>
          </Panel>

          <Remedy report={report} />
        </div>

        <Panel title="What this answer does not claim">
          <div style={{ display: 'grid', gap: 8 }}>
            <p className="prose">{report.limitations.stale_is_not_unverified}</p>
            <p className="prose">{report.limitations.verdict_is_a_moment}</p>
            <p className="prose">{report.limitations.sweep_is_bounded}</p>
            <p className="prose">{report.limitations.added_is_a_separate_measurement}</p>
            <p className="prose">{report.limitations.unindexable_is_not_added}</p>
            <p className="prose">{report.status_is_not_the_verdict}</p>
          </div>
        </Panel>

        <Panel title="Every verdict this can be">
          <div style={{ display: 'grid', gap: 8 }}>
            {report.vocabulary.verdicts.map((term) => (
              <div key={term.verdict} className="row row--wrap">
                <Chip tone={verdictTone(term.verdict)}>{term.verdict}</Chip>
                <span className="hash wrapany">{indexVerdictGloss(term.verdict)}</span>
              </div>
            ))}
          </div>
        </Panel>
      </div>
    </div>
  );
}
