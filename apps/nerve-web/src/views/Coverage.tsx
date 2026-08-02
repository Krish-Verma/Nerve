/**
 * Coverage — which symbols no ingested coverage run is known to touch.
 *
 * The default state of this screen, on almost every repository that opens it, is that the question
 * cannot be answered at all. `SELECT symbols WITHOUT a COVERS edge` returns *every* symbol in a
 * repository that has never run `nerve coverage`, and drawing that as a gap list says "your tests
 * cover nothing" when the truth is "Nerve has not been told what your tests cover". Those are
 * different answers, so when `coverage` is `absent` this screen renders neither a list nor a
 * tally — `totals` arrives as `null` and is never coerced to zeroes, because a `0` is a
 * measurement and nothing here was measured.
 *
 * When the question *is* answerable, the same discipline runs one scale down. `uncovered` and
 * `unmeasured` both mean "not covered" and are never merged: the first is a measurement (a run
 * instrumented the file and nothing in this symbol ran), the second is silence (no coverage
 * evidence names the file). They carry different words, different hues and their meanings are set
 * out on the screen rather than hidden in a tooltip.
 *
 * `partial` is neither, and is shown with its line counts exactly as recorded — a covered line
 * proves the symbol was entered, not that it ran through, and rounding it to either neighbour
 * would invent the difference.
 *
 * Freshness is a result, not a footnote. Coverage evidence cites the covered file's hash at
 * ingestion, the server re-hashes on this request, and every row says what came back.
 */

import { useMemo, useState } from 'react';

import type { CoverageRunRef, GapReport, GapRow, GapTotals } from '../api/types';
import { count, fileLine, freshness, shortHash, type Tone } from '../format';
import { useApi } from '../hooks';
import { entityHref } from '../routing';
import { Chip, Def, Defs, Empty, Failure, Figure, Loading, Panel } from '../ui/parts';
import { coverageState } from '../vocab';

/**
 * How many rows to ask for. The server clamps this to its own ceiling and reports the cap it
 * applied, so this number is a request rather than a promise and the screen states what it got.
 */
const LIMIT = 200;

/**
 * Hue is a claim, and here the claim is what the evidence says about the symbol.
 *
 * `uncovered` takes the same amber as stale evidence because it is a finding that wants reading;
 * `unmeasured` takes the grey reserved for absence, because that is exactly what it is. There is
 * no `default` arm on purpose: a state this build does not recognise must not borrow a real
 * member's colour and assert something nobody observed.
 */
function stateTone(state: string): Tone {
  switch (state) {
    case 'covered':
      return 'fresh';
    case 'partial':
      return 'quiet';
    case 'uncovered':
      return 'stale';
    case 'unmeasured':
      return 'absent';
  }
  return 'unknown';
}

/** The concrete claim behind one row, with the numbers the report actually recorded. */
function claim(row: GapRow): string {
  if (row.state === 'partial' || row.state === 'covered') {
    if (row.covered_lines === null || row.instrumented_lines === null) {
      return 'the report recorded no line counts for this symbol';
    }
    return `${count(row.covered_lines)} of ${count(row.instrumented_lines)} instrumented lines ran`;
  }
  if (row.state === 'uncovered') return 'a run measured this file; no line in this symbol ran';
  if (row.state === 'unmeasured') return 'no coverage evidence names this file';
  return coverageState(row.state);
}

export function Coverage() {
  const [includePartial, setIncludePartial] = useState(false);
  const [only, setOnly] = useState<string | null>(null);

  const params = useMemo(
    () => ({ limit: LIMIT, include_partial: includePartial ? 'true' : null }),
    [includePartial],
  );
  const { state, reload } = useApi<GapReport>('/api/gaps', params);

  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">Coverage</h1>
        <p className="head__sub">
          Which symbols no ingested coverage run is known to touch — and, before that, whether this
          repository has told Nerve anything about its tests at all. A coverage report says a line
          executed. It never says which test executed it, so nothing here is a call.
        </p>
      </div>

      {state.status === 'loading' ? (
        <Loading label="Reading what coverage says" />
      ) : state.status === 'error' ? (
        <Failure error={state.error} onRetry={reload} />
      ) : state.data.totals === null || !state.data.answerable ? (
        <Unanswerable report={state.data} />
      ) : (
        <Answer
          report={state.data}
          totals={state.data.totals}
          includePartial={includePartial}
          onIncludePartial={setIncludePartial}
          only={only}
          onOnly={setOnly}
        />
      )}
    </div>
  );
}

/**
 * The answer when there is no coverage evidence at all — the ordinary case, not an error.
 *
 * It must not render an empty table, which reads as "nothing is uncovered", and it must not render
 * every symbol as a gap, which reads as "no test covers anything". It says which of the two
 * questions has actually gone unanswered, and gives the one command that changes that.
 */
function Unanswerable({ report }: { report: GapReport }) {
  return (
    <div className="stack">
      <section className="panel">
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <div className="row row--wrap">
            <Chip tone="absent" title={`coverage evidence: ${report.coverage}`}>
              <span className="chip__dot" />
              coverage absent
            </Chip>
            <Chip tone="quiet">{count(report.symbols_in_scope)} symbols in scope</Chip>
            <span className="spacer" />
            <span className="head__sub">nothing to tally, so nothing is tallied</span>
          </div>

          <p style={{ fontSize: 17, lineHeight: 1.45 }}>
            Nerve has not been told what your tests cover.
          </p>

          <p className="prose">
            No coverage report has ever been ingested into this index, so the gap question is
            unanswerable here. This is not the same finding as &ldquo;no test covers your
            code&rdquo;: that would be a measurement, and nothing has been measured. Listing all{' '}
            {count(report.symbols_in_scope)} symbols in scope as gaps would report the second answer
            as the first, so this screen lists none of them and shows no totals — not even zeroes.
          </p>

          <div className="gate__sample">{'$ nerve coverage <lcov-report>'}</div>

          <p className="prose">
            Ingest an LCOV report with that command and open this screen again. Nerve reads the
            report as evidence — it records which lines a run executed, and never which test
            executed them.
          </p>
        </div>
      </section>

      {report.runs.length > 0 ? <Runs runs={report.runs} /> : null}
    </div>
  );
}

/** The answer when coverage evidence exists. */
function Answer({
  report,
  totals,
  includePartial,
  onIncludePartial,
  only,
  onOnly,
}: {
  report: GapReport;
  totals: GapTotals;
  includePartial: boolean;
  onIncludePartial: (value: boolean) => void;
  only: string | null;
  onOnly: (value: string | null) => void;
}) {
  const byState = new Map<string, number>();
  for (const row of report.results) byState.set(row.state, (byState.get(row.state) ?? 0) + 1);
  const states = [...byState.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));

  // A filter left selected after the row set changed would show an empty list under a button that
  // is no longer there. It falls back to every state rather than to nothing.
  const active = only !== null && byState.has(only) ? only : null;
  const rows = active === null ? report.results : report.results.filter((r) => r.state === active);

  const sentence =
    totals.gaps === 0
      ? `Every one of the ${count(report.symbols_in_scope)} symbols in scope was touched by an ingested coverage run.`
      : `${count(totals.gaps)} of ${count(report.symbols_in_scope)} symbols in scope are not known to be touched by any run — ${count(totals.uncovered)} measured and never entered, ${count(totals.unmeasured)} never measured at all.`;

  return (
    <div className="stack">
      <section className="panel">
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <div className="row row--wrap">
            <Chip tone={totals.gaps === 0 ? 'fresh' : 'stale'}>
              <span className="chip__dot" />
              {totals.gaps === 0 ? 'no gaps' : `${count(totals.gaps)} gaps`}
            </Chip>
            <Chip tone="quiet">
              {count(report.runs.length)} coverage {report.runs.length === 1 ? 'run' : 'runs'}
            </Chip>
            {totals.stale > 0 ? (
              <Chip
                tone="stale"
                title="These answers were computed from coverage taken against bytes the files no longer have."
              >
                {count(totals.stale)} stale
              </Chip>
            ) : null}
            <span className="spacer" />
            <span className="head__sub">
              {count(report.files_probed)} files re-hashed on this request
            </span>
          </div>
          <p style={{ fontSize: 17, lineHeight: 1.45 }}>{sentence}</p>
        </div>
      </section>

      <div className="figures">
        <Figure
          label="uncovered"
          value={count(totals.uncovered)}
          note="measured, never entered"
          unknown={totals.uncovered > 0}
        />
        <Figure
          label="unmeasured"
          value={count(totals.unmeasured)}
          note="no evidence names the file"
          unknown={totals.unmeasured > 0}
        />
        <Figure label="partial" value={count(totals.partial)} note="entered, not run through" />
        <Figure label="covered" value={count(totals.covered)} note="every instrumented line ran" />
        <Figure
          label="stale"
          value={count(totals.stale)}
          note={`over ${count(totals.stale_files)} of ${count(totals.measured_files)} measured files`}
          unknown={totals.stale > 0}
        />
      </div>

      <div className="grid2">
        <Panel title="Two ways of not being covered">
          <Defs>
            <Def term="uncovered">{coverageState('uncovered')}</Def>
            <Def term="unmeasured">{coverageState('unmeasured')}</Def>
            <Def term="partial">{coverageState('partial')}</Def>
            <Def term="stale">
              The coverage this answer rests on was taken against a file whose bytes have changed
              since. The claim is only as current as what it measured, so it is labelled rather than
              quietly reused.
            </Def>
          </Defs>
        </Panel>
        <Runs runs={report.runs} />
      </div>

      {report.results.length === 0 ? (
        <Panel title={includePartial ? 'Gaps, and the partly covered' : 'Symbols with a gap'}>
          <Empty
            title="Nothing in scope is a gap"
            body={
              totals.partial === 0
                ? 'Every symbol the ingested coverage could speak for was entered by it. Read that against the runs above: it is a statement about what those reports measured, not about every test you have.'
                : `Every symbol in scope was entered by an ingested run. ${count(totals.partial)} of them only partly — those are counted above and listed here if you ask for them.`
            }
          >
            {totals.partial > 0 ? (
              <button
                type="button"
                className={includePartial ? 'btn btn--on' : 'btn'}
                aria-pressed={includePartial}
                onClick={() => onIncludePartial(!includePartial)}
              >
                include partial
                <span className="rail__count num">{count(totals.partial)}</span>
              </button>
            ) : null}
          </Empty>
        </Panel>
      ) : (
        <section className="panel">
          <header className="panel__head">
            {/*
              The heading has to widen when partial rows are asked for. Partial is not a gap, and a
              panel headed "Symbols with a gap" that lists partly covered symbols would undo, in a
              title, the distinction the rest of the screen is built to keep.
            */}
            <h2 className="micro">
              {includePartial ? 'Gaps, and the partly covered' : 'Symbols with a gap'}
            </h2>
            <span className="hash">
              {count(report.count)} shown · {count(report.results_total)} matching
            </span>
          </header>

          <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
            <p className="prose">
              Each row is a symbol and the claim the evidence supports about it. Open one to read
              the observations underneath — including, where there are any, the coverage
              observations themselves.
            </p>
            <div className="row row--wrap" role="group" aria-label="Filter by coverage state">
              <button
                type="button"
                className={active === null ? 'btn btn--on' : 'btn'}
                aria-pressed={active === null}
                onClick={() => onOnly(null)}
              >
                every state
                <span className="rail__count num">{count(report.results.length)}</span>
              </button>
              {states.map(([code, tally]) => (
                <button
                  key={code}
                  type="button"
                  className={active === code ? 'btn btn--on' : 'btn'}
                  aria-pressed={active === code}
                  title={coverageState(code)}
                  onClick={() => onOnly(active === code ? null : code)}
                >
                  {code}
                  <span className="rail__count num">{count(tally)}</span>
                </button>
              ))}
              <span className="spacer" />
              <button
                type="button"
                className={includePartial ? 'btn btn--on' : 'btn'}
                aria-pressed={includePartial}
                title="Partial symbols are tallied either way. This only decides whether they are listed."
                onClick={() => onIncludePartial(!includePartial)}
              >
                include partial
                <span className="rail__count num">{count(totals.partial)}</span>
              </button>
            </div>
            {active === null ? null : <p className="prose">{coverageState(active)}</p>}
          </div>

          <div className="panel__body panel__body--flush">
            <ul className="gaplist">
              {rows.map((row) => (
                <Row key={row.entity.entity_id} row={row} />
              ))}
            </ul>
          </div>
        </section>
      )}

      {report.truncated ? (
        <Panel title="Not everything is listed">
          <p className="prose">
            {count(report.count)} of {count(report.results_total)} matching symbols are shown. One
            answer carries at most {count(report.limit)} rows, so the rest were cut here rather than
            summarised. The tallies above are exact over every symbol in scope regardless — they are
            counted before the cap is applied.
          </p>
        </Panel>
      ) : null}
    </div>
  );
}

/** One symbol, its location, and what the coverage evidence claims about it. */
function Row({ row }: { row: GapRow }) {
  const label = row.entity.qualified_name || row.entity.name;
  const reading = row.coverage_freshness === null ? null : freshness(row.coverage_freshness);
  return (
    <li>
      <a
        className="cov"
        href={entityHref(row.entity.entity_id, 'evidence')}
        title={`${row.entity.kind} · ${label}`}
      >
        <span className="cov__name wrapany">{label}</span>
        <span className="cov__meta">{fileLine(row.entity.file_path, row.entity.start_line)}</span>
        <span className="cov__claim">
          <Chip tone={stateTone(row.state)} title={coverageState(row.state)}>
            {row.state}
          </Chip>
          {reading === null ? (
            <Chip
              tone="quiet"
              title="No coverage observation cites this file, so there is no recorded hash to check it against."
            >
              no evidence to date
            </Chip>
          ) : (
            <Chip tone={reading.tone} title={reading.gloss}>
              {reading.label} coverage
            </Chip>
          )}
          <span className="cov__gloss">{claim(row)}</span>
          {row.covered_by.length > 0 ? (
            <span className="hash wrapany">from {row.covered_by.join(' · ')}</span>
          ) : null}
        </span>
      </a>
    </li>
  );
}

/**
 * The coverage runs the whole answer is relative to.
 *
 * Every number on this screen is relative to these reports and to when they were taken, so they
 * are shown rather than assumed: the path that was ingested, whether that file still hashes to
 * what was read, and how many source files the report said it described.
 */
function Runs({ runs }: { runs: CoverageRunRef[] }) {
  if (runs.length === 0) {
    return (
      <Panel title="Coverage runs">
        <Empty
          title="No run is recorded"
          body="No coverage report has been ingested here, so there is nothing for an answer to be relative to."
        />
      </Panel>
    );
  }

  return (
    <Panel
      title="What this is relative to"
      aside={
        <span className="hash">
          {count(runs.length)} {runs.length === 1 ? 'report' : 'reports'}
        </span>
      }
    >
      <div className="spine">
        {runs.map((run) => {
          const reading = run.freshness === null ? null : freshness(run.freshness);
          return (
            <div key={run.entity_id} className="obs obs--direct" style={{ gap: 5 }}>
              <div className="obs__line">
                <span className="obs__source wrapany">
                  {run.report_path ?? 'no report path was recorded'}
                </span>
                {reading === null ? (
                  <Chip
                    tone="quiet"
                    title="Without both a path and a recorded hash there is nothing to re-check the report against."
                  >
                    freshness unmeasurable
                  </Chip>
                ) : (
                  <Chip tone={reading.tone} title={reading.gloss}>
                    {reading.label}
                  </Chip>
                )}
              </div>
              <div className="fact__value">
                {run.source_files_in_report === null
                  ? 'the report did not record how many source files it described'
                  : `${count(run.source_files_in_report)} source ${
                      run.source_files_in_report === 1 ? 'file' : 'files'
                    } in the report`}
              </div>
              <div className="hash wrapany" title={run.report_content_hash ?? ''}>
                content hash {shortHash(run.report_content_hash)}
              </div>
            </div>
          );
        })}
      </div>
    </Panel>
  );
}
