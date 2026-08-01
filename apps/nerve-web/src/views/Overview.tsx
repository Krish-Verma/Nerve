/**
 * Overview — the first screen.
 *
 * Its job is to orient, not to enumerate. The largest thing on the page is therefore a sentence
 * about whether the graph still describes the repository, not a count: a stale index is the one
 * fact that changes how every other number on this screen should be read.
 */

import type { Resource } from '../hooks';
import type { FreshnessReport, Overview as OverviewData, RunSummary } from '../api/types';
import { ago, bytes, count, stamp } from '../format';
import { Chip, Def, Defs, Empty, Failure, Figure, Loading, Panel, Tally } from '../ui/parts';

function basename(path: string | null): string {
  if (!path) return 'this repository';
  const parts = path.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function sorted(counts: Record<string, number>): [string, number][] {
  return Object.entries(counts).sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
}

/** The one-line reading of the freshness sweep, in the interface's voice. */
function verdict(freshness: FreshnessReport | null): { text: string; ok: boolean } {
  if (!freshness) {
    return { text: 'Freshness has not been measured for this index.', ok: false };
  }
  const drifted = freshness.stale + freshness.missing;
  if (freshness.files_probed === 0) {
    return { text: 'No indexed files were available to check.', ok: false };
  }
  if (drifted === 0 && freshness.refused === 0 && freshness.unreadable === 0) {
    return {
      text: `All ${count(freshness.files_probed)} indexed ${
        freshness.files_probed === 1 ? 'file' : 'files'
      } still hash to what the graph was built from.`,
      ok: true,
    };
  }
  if (drifted === 0) {
    return {
      text: `${count(freshness.fresh)} files match. ${count(
        freshness.refused + freshness.unreadable,
      )} could not be checked.`,
      ok: false,
    };
  }
  return {
    text: `${count(drifted)} of ${count(freshness.files_probed)} indexed files have changed or gone since indexing. Everything derived from them may be out of date.`,
    ok: false,
  };
}

function Gauge({ freshness }: { freshness: FreshnessReport }) {
  const total = Math.max(1, freshness.files_probed);
  const bands: [number, string][] = [
    [freshness.fresh, 'var(--fresh)'],
    [freshness.stale, 'var(--stale)'],
    [freshness.missing, 'var(--absent)'],
    [freshness.refused, 'var(--bone-4)'],
    [freshness.unreadable, 'var(--bone-4)'],
  ];
  return (
    <div
      className="gauge"
      role="img"
      aria-label={`${freshness.fresh} fresh, ${freshness.stale} stale, ${freshness.missing} missing of ${freshness.files_probed} probed`}
    >
      {bands.map(([value, colour], index) =>
        value > 0 ? (
          <span
            key={index}
            style={{ width: `${(value / total) * 100}%`, background: colour }}
          />
        ) : null,
      )}
    </div>
  );
}

function Run({ run, latest }: { run: RunSummary; latest: boolean }) {
  return (
    <div className="obs obs--direct" style={{ gap: 5 }}>
      <div className="obs__line">
        <span className="obs__source">
          {run.extractor_id}@{run.extractor_version}
        </span>
        {latest ? <Chip tone="quiet">latest</Chip> : null}
        <Chip tone={run.status === 'complete' ? 'fresh' : 'stale'}>{run.status}</Chip>
      </div>
      <div className="fact__value">
        {count(run.files_processed)} processed · {count(run.files_failed)} failed ·{' '}
        {stamp(run.finished_at ?? run.started_at)}
      </div>
    </div>
  );
}

export function Overview({ resource }: { resource: Resource<OverviewData> }) {
  const { state, reload } = resource;

  if (state.status === 'loading') return <Loading label="Measuring the index" />;
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;

  const data = state.data;
  const reading = verdict(data.freshness);
  const kinds = sorted(data.entities_by_kind);
  const relations = sorted(data.assertions_by_relation);

  if (data.entities_total === 0) {
    return (
      <div className="view">
        <div className="head">
          <h1 className="head__title">{basename(data.root_path)}</h1>
          <div className="head__sub wrapany">{data.root_path ?? 'no repository root recorded'}</div>
        </div>
        <Empty
          title="This index is empty"
          body="Nerve has a database here but no entities in it. Run nerve index in the repository root to parse the files and build the graph, then reload this page."
        >
          <div className="row row--wrap">
            <Chip tone="quiet">schema v{data.schema_version ?? '—'}</Chip>
            <Chip tone={data.healthy ? 'fresh' : 'stale'}>
              {data.healthy ? 'healthy' : 'unhealthy'}
            </Chip>
          </div>
        </Empty>
      </div>
    );
  }

  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">{basename(data.root_path)}</h1>
        <p className="head__sub wrapany">{data.root_path ?? 'no repository root recorded'}</p>
      </div>

      <div className="stack">
        <section className="panel">
          <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
            <div className="row row--wrap">
              <Chip tone={reading.ok ? 'fresh' : 'stale'}>
                <span className="chip__dot" />
                {reading.ok ? 'index is current' : 'index has drifted'}
              </Chip>
              {data.freshness?.truncated ? (
                <Chip tone="quiet">
                  partial sweep · {count(data.freshness.files_probed)} of{' '}
                  {count(data.freshness.files_total)} files
                </Chip>
              ) : null}
              <span className="spacer" />
              <span className="head__sub">
                measured on this request, by re-hashing the files
              </span>
            </div>
            <p style={{ fontSize: 17, lineHeight: 1.45 }}>{reading.text}</p>
            {data.freshness ? <Gauge freshness={data.freshness} /> : null}
          </div>
        </section>

        <div className="figures">
          <Figure label="entities" value={count(data.entities_total)} note="named things" />
          <Figure
            label="assertions"
            value={count(data.assertions_total)}
            note="relationships held"
          />
          <Figure
            label="observations"
            value={count(data.observations_total)}
            note="pieces of evidence"
          />
          <Figure
            label="occurrences"
            value={count(data.occurrences_total)}
            note="source spans"
          />
          <Figure
            label="unresolved"
            value={count(data.unresolved_entities)}
            note={`${count(data.unresolved_assertions)} assertions reach them`}
            unknown={data.unresolved_entities > 0}
          />
        </div>

        <div className="grid2">
          <Panel title="Entities by kind" aside={<span className="hash">{kinds.length} kinds</span>}>
            <Tally rows={kinds} unknownKey="unresolved" />
          </Panel>
          <Panel
            title="Assertions by relation"
            aside={<span className="hash">{relations.length} relations</span>}
          >
            <Tally rows={relations} />
          </Panel>
        </div>

        <div className="grid2">
          <Panel title="Index state">
            <Defs>
              <Def term="schema">
                v{data.schema_version ?? '—'}
                <span className="hash">
                  {' '}
                  · this build supports v{data.supported_schema_version}
                </span>
              </Def>
              <Def term="health">
                <Chip tone={data.healthy ? 'fresh' : 'stale'}>
                  {data.healthy ? 'healthy' : 'unhealthy'}
                </Chip>
              </Def>
              <Def term="database">{bytes(data.database_bytes)}</Def>
              <Def term="project id">
                <span className="hash">{data.project_id ?? 'not recorded'}</span>
              </Def>
              <Def term="state id">
                <span className="hash">{data.state_id ?? 'not recorded'}</span>
              </Def>
              <Def term="git commit">
                <span className="hash">{data.git_commit ?? 'not a git checkout'}</span>
              </Def>
              <Def term="assertion states">{count(data.assertion_states_total)}</Def>
            </Defs>
          </Panel>

          <Panel
            title="Extractor runs"
            aside={
              data.last_run ? (
                <span className="hash">{ago(data.last_run.finished_at ?? data.last_run.started_at)}</span>
              ) : null
            }
          >
            {data.runs.length === 0 ? (
              <p className="state__body">No extractor has run against this database yet.</p>
            ) : (
              <div className="spine">
                {data.runs.map((run) => (
                  <Run
                    key={run.run_id}
                    run={run}
                    latest={data.last_run?.run_id === run.run_id}
                  />
                ))}
              </div>
            )}
          </Panel>
        </div>
      </div>
    </div>
  );
}
