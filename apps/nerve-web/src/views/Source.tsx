/**
 * The source behind an entity.
 *
 * An entity can occupy more than one span — a name declared once and re-exported elsewhere has
 * several occurrences — so this lists every one rather than picking a canonical location. Each
 * occurrence carries the content hash recorded for its file, and the snippet endpoint returns the
 * hash of that file as it is right now, so the two can be compared here as well.
 *
 * The server only serves files that are already in the index, and re-checks the path through the
 * same guard indexing used. Nothing here can ask for a file Nerve never read.
 */

import { useMemo } from 'react';

import type { EntityDetail, Occurrence, SourceSnippet } from '../api/types';
import { count } from '../format';
import { useApi } from '../hooks';
import { Chip, Empty, Failure, Loading, Panel } from '../ui/parts';

export function Source({ detail }: { detail: EntityDetail }) {
  if (detail.occurrence_count === 0) {
    return (
      <Panel title="Source">
        <Empty
          title="This entity has no recorded location"
          body="Unresolved references are named by what was written, not by where anything is declared — there is no declaration to show. The evidence tab holds the places that refer to it."
        />
      </Panel>
    );
  }

  return (
    <div className="stack">
      {detail.occurrences.map((occurrence) => (
        <Span key={occurrence.occurrence_id} occurrence={occurrence} />
      ))}
      {detail.occurrence_count > detail.occurrences.length ? (
        <Panel title="Not everything is shown">
          <p className="prose">
            {count(detail.occurrence_count - detail.occurrences.length)} further occurrences were
            not returned.
          </p>
        </Panel>
      ) : null}
    </div>
  );
}

function Span({ occurrence }: { occurrence: Occurrence }) {
  const params = useMemo(
    () => ({
      path: occurrence.file_path,
      start_line: Math.max(1, occurrence.start_line - 3),
      end_line: occurrence.end_line + 3,
    }),
    [occurrence],
  );
  const { state, reload } = useApi<SourceSnippet>('/api/source', params);

  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro wrapany">{occurrence.file_path}</h2>
        <span className="hash">
          lines {count(occurrence.start_line)}–{count(occurrence.end_line)}
        </span>
      </header>

      {state.status === 'loading' ? (
        <Loading label="Reading the file" />
      ) : state.status === 'error' ? (
        <div className="panel__body">
          <Failure error={state.error} onRetry={reload} />
        </div>
      ) : (
        <>
          <div className="panel__body" style={{ paddingBottom: 0 }}>
            <div className="row row--wrap">
              <Chip
                tone={state.data.content_hash === occurrence.content_hash ? 'fresh' : 'stale'}
                title={
                  state.data.content_hash === occurrence.content_hash
                    ? 'The file still hashes to what was indexed.'
                    : 'The file has changed since it was indexed, so these line numbers may no longer point at the same code.'
                }
              >
                <span className="chip__dot" />
                {state.data.content_hash === occurrence.content_hash
                  ? 'file unchanged since indexing'
                  : 'file changed since indexing'}
              </Chip>
              <span className="hash">{count(state.data.total_lines)} lines in the file</span>
              {state.data.truncated ? <Chip tone="absent">snippet truncated</Chip> : null}
            </div>
          </div>
          <div className="panel__body">
            <pre
              className="code"
              aria-label={`${state.data.path} lines ${state.data.start_line} to ${state.data.end_line}`}
            >
              {state.data.text.split('\n').map((line, index) => {
                const number = state.data.start_line + index;
                const marked =
                  number >= occurrence.start_line && number <= occurrence.end_line;
                return (
                  <div className={marked ? 'code__line code__line--marked' : 'code__line'} key={number}>
                    <span className="code__no num">{number}</span>
                    <span className="code__text">{line === '' ? ' ' : line}</span>
                  </div>
                );
              })}
            </pre>
          </div>
        </>
      )}
    </section>
  );
}
