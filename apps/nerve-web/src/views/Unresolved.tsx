/**
 * Unresolved — what Nerve could not resolve, and which files it could not fully parse.
 *
 * This screen exists because the alternative is worse. A tool that silently drops what it cannot
 * work out produces a graph that looks complete and is not, and the reader has no way to tell.
 * So every unresolved reference is kept as an entity, counted, and given a reason here.
 *
 * It is deliberately not styled as a warning. Most of these are statements about the language —
 * a method call on an untyped receiver is unresolvable by reading syntax, full stop — and dressing
 * them in alarm colours would train the reader to ignore the ones that matter.
 *
 * It was called "Gaps" until Slice 7a-ii, which was one word doing two jobs: `nerve gaps` answers
 * *"which symbols does no test touch?"*, and this screen answers *"what could Nerve not work
 * out?"*. Those are different questions with different evidence behind them, and a reader who
 * asked one and was shown the other would have no way of knowing. The coverage question now has
 * its own screen; this one is named after what it actually reads.
 */

import { useMemo, useState } from 'react';

import type { PartialParseReport, UnresolvedReport } from '../api/types';
import { count, jsonText } from '../format';
import { useApi } from '../hooks';
import { entityHref } from '../routing';
import { Chip, Empty, Failure, Loading, Panel } from '../ui/parts';
import { reasonLabel, unmodelledForm, unresolvedReason } from '../vocab';

const PAGE = 100;

function reasonOf(meta: unknown): string {
  if (meta === null || typeof meta !== 'object' || Array.isArray(meta)) return '';
  const value = (meta as Record<string, unknown>)['reason'];
  return typeof value === 'string' ? value : '';
}

function importerOf(meta: unknown): string | null {
  if (meta === null || typeof meta !== 'object' || Array.isArray(meta)) return null;
  const value = (meta as Record<string, unknown>)['importer'];
  return typeof value === 'string' ? value : null;
}

export function Unresolved() {
  const [offset, setOffset] = useState(0);
  const [reason, setReason] = useState<string | null>(null);

  const params = useMemo(() => ({ limit: PAGE, offset }), [offset]);
  const unresolved = useApi<UnresolvedReport>('/api/unresolved', params);
  const partial = useApi<PartialParseReport>('/api/partial-parses');

  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">Unresolved</h1>
        <p className="head__sub">
          References Nerve recorded and could not connect to a declaration, and files it could only
          parse in part. Both are kept rather than discarded, so the graph never looks more complete
          than it is. These are Nerve&apos;s own gaps in knowledge, not your repository&apos;s gaps
          in testing — for those, see Coverage.
        </p>
      </div>

      <div className="stack">
        {unresolved.state.status === 'loading' ? (
          <Loading label="Reading what could not be resolved" />
        ) : unresolved.state.status === 'error' ? (
          <Failure error={unresolved.state.error} onRetry={unresolved.reload} />
        ) : (
          <References
            report={unresolved.state.data}
            offset={offset}
            onOffset={setOffset}
            reason={reason}
            onReason={setReason}
          />
        )}

        {partial.state.status === 'loading' ? null : partial.state.status === 'error' ? (
          <Failure error={partial.state.error} onRetry={partial.reload} />
        ) : (
          <PartialParses report={partial.state.data} />
        )}
      </div>
    </div>
  );
}

function References({
  report,
  offset,
  onOffset,
  reason,
  onReason,
}: {
  report: UnresolvedReport;
  offset: number;
  onOffset: (value: number) => void;
  reason: string | null;
  onReason: (value: string | null) => void;
}) {
  const byReason = new Map<string, number>();
  for (const row of report.results) {
    const code = reasonOf(row.meta) || 'unrecorded';
    byReason.set(code, (byReason.get(code) ?? 0) + 1);
  }
  const reasons = [...byReason.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
  const rows = reason === null
    ? report.results
    : report.results.filter((row) => (reasonOf(row.meta) || 'unrecorded') === reason);

  if (report.unresolved_entities_total === 0) {
    return (
      <Panel title="Unresolved references">
        <Empty
          title="Everything resolved"
          body="Every reference this index recorded was connected to a declaration. On a repository of any size that is unusual — check that indexing covered what you expected."
        />
      </Panel>
    );
  }

  const last = offset + report.count >= report.unresolved_entities_total;

  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Unresolved references</h2>
        <span className="hash">
          {count(report.unresolved_entities_total)}{' '}
          {report.unresolved_entities_total === 1 ? 'reference' : 'references'} ·{' '}
          {count(report.unresolved_assertions_total)} assertions reach them
        </span>
      </header>

      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <p className="prose">
          Each of these is a name Nerve saw being used and could not tie to a definition. They are
          real entities in the graph: you can open one and see every place that refers to it.
        </p>
        <div className="row row--wrap" role="group" aria-label="Filter by reason">
          <button
            type="button"
            className={reason === null ? 'btn btn--on' : 'btn'}
            aria-pressed={reason === null}
            onClick={() => onReason(null)}
          >
            every reason
            <span className="rail__count num">{count(report.count)}</span>
          </button>
          {reasons.map(([code, tally]) => (
            <button
              key={code}
              type="button"
              className={reason === code ? 'btn btn--on' : 'btn'}
              aria-pressed={reason === code}
              title={unresolvedReason(code)}
              onClick={() => onReason(reason === code ? null : code)}
            >
              {reasonLabel(code)}
              <span className="rail__count num">{count(tally)}</span>
            </button>
          ))}
        </div>
        {reason === null ? null : <p className="prose">{unresolvedReason(reason)}</p>}
      </div>

      {/*
        These rows do not use the shared hit layout. A reason is not an entity kind: kinds are one
        short word from a closed list, reasons run to a full clause. Putting one in a column sized
        for the other wrapped every row six lines deep, so the name leads here and the reason sits
        under it where it has the width to be read.
      */}
      <div className="panel__body panel__body--flush">
        <ul className="gaplist">
          {rows.map((row) => {
            const code = reasonOf(row.meta) || 'unrecorded';
            const importer = importerOf(row.meta);
            return (
              <li key={row.entity_id}>
                <a className="gap" href={entityHref(row.entity_id, 'evidence')}>
                  {/*
                    A reference can genuinely have no name: an ADR that writes `**Supersedes:**`
                    and then nothing named nothing, and that emptiness is the finding. Rendering
                    it as a blank line loses the row entirely, so the absence is stated.
                  */}
                  <span
                    className={row.name === '' ? 'gap__name gap__name--empty' : 'gap__name wrapany'}
                    title={row.name === '' ? 'the reference names nothing at all' : row.name}
                  >
                    {row.name === '' ? 'nothing was named' : row.name}
                  </span>
                  <span className="gap__meta">
                    {importer ?? row.scope_path} · {count(row.referencing_assertions)}{' '}
                    {row.referencing_assertions === 1 ? 'reference' : 'references'}
                  </span>
                  <span className="gap__reason">{unresolvedReason(code)}</span>
                </a>
              </li>
            );
          })}
        </ul>
      </div>

      {report.unresolved_entities_total > PAGE ? (
        <div className="graph__foot">
          <button
            type="button"
            className="btn"
            disabled={offset === 0}
            onClick={() => onOffset(Math.max(0, offset - PAGE))}
          >
            previous
          </button>
          <span>
            {count(offset + 1)}–{count(offset + report.count)} of{' '}
            {count(report.unresolved_entities_total)}
          </span>
          <button
            type="button"
            className="btn"
            disabled={last}
            onClick={() => onOffset(offset + PAGE)}
          >
            next
          </button>
        </div>
      ) : null}
    </section>
  );
}

function PartialParses({ report }: { report: PartialParseReport }) {
  if (report.count === 0) {
    return (
      <Panel title="Partial parses">
        <Empty
          title="Every file parsed cleanly"
          body="No indexed file produced a syntax error, so nothing on this index was extracted from a partly understood file."
        />
      </Panel>
    );
  }

  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Partial parses</h2>
        <span className="hash">
          {count(report.count)} {report.count === 1 ? 'file' : 'files'}
        </span>
      </header>
      <div className="panel__body">
        <p className="prose">
          These files produced syntax errors. The parser recovers and carries on, so they did
          contribute to the graph — but read anything extracted from them with suspicion, because
          whole regions may have been skipped.
        </p>
      </div>
      <div className="panel__body panel__body--flush">
        {report.results.map((row) => {
          const forms = Object.entries(row.unmodelled_by_form).sort(
            (a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1),
          );
          return (
            <div key={row.rel_path} className="relgroup">
              <div className="relgroup__head">
                <span className="relgroup__verb wrapany">{row.rel_path}</span>
                <span className="relgroup__gloss">{row.language}</span>
              </div>
              <div className="panel__body" style={{ paddingTop: 0, display: 'grid', gap: 10 }}>
                <div className="row row--wrap">
                  <Chip tone="stale">syntax error</Chip>
                  {row.dynamic_imports_without_specifier > 0 ? (
                    <Chip
                      tone="absent"
                      title="An import() whose path is computed at runtime cannot be followed by reading the source."
                    >
                      {count(row.dynamic_imports_without_specifier)} computed imports
                    </Chip>
                  ) : null}
                  {row.unmodelled_call_sites > 0 ? (
                    <Chip tone="absent">
                      {count(row.unmodelled_call_sites)} unmodelled call sites
                    </Chip>
                  ) : null}
                </div>
                {forms.length > 0 ? (
                  <div className="obs__facts">
                    {forms.map(([form, tally]) => (
                      <div className="fact" key={form}>
                        <span className="fact__key">
                          {form} · {count(tally)}
                        </span>
                        <span className="fact__value">{unmodelledForm(form)}</span>
                      </div>
                    ))}
                  </div>
                ) : null}
                <div className="hash wrapany">content hash {jsonText(row.content_hash)}</div>
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
