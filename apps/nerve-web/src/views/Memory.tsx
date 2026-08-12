/**
 * Memory — the notes a person wrote about this repository, and what is true of them right now.
 *
 * Four things make this screen different from every other one, and each is a rule rather than a
 * layout choice.
 *
 * **Nothing here was discovered.** A note exists because somebody ran a command and typed a
 * sentence. So an empty screen is an *absence* rather than a finding — and it is one of **two**
 * absences, which are not the same thing and are not headed alike: nothing has ever been written
 * here, or notes exist and this question matches none of them. The first is answered with the
 * command that changes it; the second with a way to widen the question.
 *
 * **Stored and derived are two kinds and are never one row of chips.** `status` is a column holding
 * one of four values. `potentially_stale`, `conflicted` and `multiple_active` are worked out at the
 * moment the record is read, from the anchor state and from what else is active on the same
 * subject, and nothing writes one. They are shown in separate, labelled groups with different
 * rules beside them, and each group says which kind it is. `subject_resolution` and
 * `superseded_by_memory_id` are on the derived side for the same reason.
 *
 * **The subject is a copy, not a pointer.** Indexing deletes entities when files go away, so a note
 * keeps a snapshot of what it was written about and the live subject is resolved when the note is
 * read. A moved subject is re-attached only where a recorded rename says so, never because two
 * names looked similar, and where more than one candidate exists all of them are shown and none is
 * chosen.
 *
 * **Writing is a command, and this API is read-only by design.** Proposing, confirming,
 * superseding, invalidating and citing all write, and `nerve serve` holds one `PRAGMA query_only`
 * connection per worker with the promise proved on the database bytes. There is therefore no
 * control here and no disabled one implying an implementation is pending — the screen shows the
 * data, states the boundary, and prints the commands. They are printed **verbatim, exactly as the
 * server sends them**, with the placeholder left in: a `memory_id` is caller-supplied text that may
 * hold any character, and substituting one into a line a reader is invited to paste into a shell
 * would be building a command out of untrusted text.
 *
 * Every string on a record — the note, the author label, the claim key, the reason it ended, an
 * event's note, a citation's path and every field of the subject snapshot — is free text a person
 * typed. All of it is interpolated as a React child, which React escapes. Nothing in this file
 * builds markup from a string.
 */

import { useMemo } from 'react';

import type { MemoryBlock, MemoryCitation, MemoryEvent, MemoryRecord } from '../api/types';
import { count, stamp } from '../format';
import { useApi } from '../hooks';
import {
  absenceHeading,
  anchorReading,
  hasPager,
  memoryFragment,
  pageReading,
  statusTone,
  subjectTone,
  supersession,
  viewTone,
} from '../memory';
import { Chip, Def, Defs, Empty, Failure, Loading, Panel, Where } from '../ui/parts';
import {
  memoryOperationGloss,
  memoryScopeGloss,
  memoryStatusGloss,
  memorySubjectResolutionGloss,
  memoryViewGloss,
} from '../vocab';

/** How many records one page asks for. The server clamps and reports what it applied. */
const PAGE = 50;

/**
 * The boundary, printed as the commands it actually is.
 *
 * Not a disabled button. A control that cannot work implies an implementation is pending, and none
 * is: this surface is read-only and is proven so on the database bytes, so changing a note is a
 * command and the useful thing to show is the command.
 */
function Boundary({ statement, commands }: { statement: string; commands: string[] }) {
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Writing any of this</h2>
        <span className="hash">command line only</span>
      </header>
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <p className="prose">{statement}</p>
        <div className="gate__sample">
          {commands.map((command) => (
            <div key={command}>$ {command}</div>
          ))}
        </div>
        <p className="prose">
          Run one and reload this page. The placeholders are left in on purpose: a note&apos;s id is
          text somebody typed, so this page prints the command it was given rather than assembling
          one around a value out of the index. Copy the id from the record below.
        </p>
      </div>
    </section>
  );
}

/** The qualifications every memory answer carries, rendered rather than dropped. */
function Limitations({ block }: { block: MemoryBlock }) {
  return (
    <Panel title="What this answer does not claim">
      <div style={{ display: 'grid', gap: 8 }}>
        <p className="prose">{block.limitations.views_are_derived}</p>
        <p className="prose">{block.limitations.subject_is_a_snapshot}</p>
        <p className="prose">{block.limitations.superseded_by_is_derived}</p>
        <p className="prose">{block.limitations.author_label_is_not_an_identity}</p>
        <p className="prose">{block.limitations.memory_is_not_evidence}</p>
        <p className="prose">{block.limitations.no_delete_verb}</p>
      </div>
    </Panel>
  );
}

/**
 * The filter row, built from the vocabulary the answer carried.
 *
 * Not from a list mirrored in this app. Two of the three closed sets are filterable and one is
 * not, and a control built from a local copy could offer a filter on a value nothing ever wrote —
 * which the server would answer with a `400`, correctly, after the user had already been shown it
 * as a choice. The derived views are listed beside the control as what they are: reported, never
 * filtered on.
 */
function Filters({ block, options }: { block: MemoryBlock; options: Record<string, string> }) {
  const rest = (key: string, value: string) => {
    const next: Record<string, string> = { ...options, [key]: value };
    delete next['offset'];
    delete next['record'];
    if (value === '') delete next[key];
    return memoryFragment(next);
  };
  const chosen = (key: string, value: string) =>
    (options[key] ?? '') === value ? 'btn btn--on' : 'btn';

  return (
    <Panel title="Narrow this">
      <div style={{ display: 'grid', gap: 10 }}>
        <div className="row row--wrap">
          <span className="micro">scope</span>
          <a className={chosen('scope', '')} href={rest('scope', '')}>
            any
          </a>
          {block.vocabulary.scopes.map((scope) => (
            <a
              key={scope}
              className={chosen('scope', scope)}
              href={rest('scope', scope)}
              title={memoryScopeGloss(scope)}
            >
              {scope}
            </a>
          ))}
        </div>
        <div className="row row--wrap">
          <span className="micro">stored status</span>
          <a className={chosen('status', '')} href={rest('status', '')}>
            any
          </a>
          {block.vocabulary.stored_statuses.map((status) => (
            <a
              key={status}
              className={chosen('status', status)}
              href={rest('status', status)}
              title={memoryStatusGloss(status)}
            >
              {status}
            </a>
          ))}
        </div>
        <div className="row row--wrap">
          <span className="micro">derived views</span>
          {block.vocabulary.derived_views.map((view) => (
            <Chip key={view} tone={viewTone(view)} title={memoryViewGloss(view)}>
              {view}
            </Chip>
          ))}
          <span className="hash">reported on each note, never a filter</span>
        </div>
        {block.requested.query === null ? null : (
          <p className="prose">
            Narrowed to notes whose text or claim key contains{' '}
            <strong className="wrapany">{block.requested.query}</strong> —{' '}
            {count(block.records_matching)} of {count(block.records_in_repository)} match.{' '}
            <a className="link" href={memoryFragment({})}>
              show all
            </a>
          </p>
        )}
        {block.requested.subject === null ? null : (
          <p className="prose">
            Narrowed to one subject —{' '}
            <span className="hash wrapany">{block.requested.subject}</span>.{' '}
            <a className="link" href={memoryFragment({})}>
              show all
            </a>
          </p>
        )}
      </div>
    </Panel>
  );
}

/** One citation: a passage a note quotes, kept as a copy for the same reason the subject is. */
function Citation({ citation }: { citation: MemoryCitation }) {
  return (
    <div className="claim__sentence">
      <Where path={citation.cited_path} />
      {citation.cited_span === null ? (
        <span className="hash">whole file</span>
      ) : (
        <span className="hash">lines {citation.cited_span}</span>
      )}
      {citation.cited_name === null ? null : (
        <Chip tone="quiet" title="What was there when the citation was taken. A copy, not a pointer.">
          {citation.cited_kind} · {citation.cited_name}
        </Chip>
      )}
      <span className="hash wrapany">at {citation.cited_at_state}</span>
    </div>
  );
}

/**
 * One entry of the audit history. Nothing deletes one, including invalidation.
 *
 * `changes_status` is read off the event rather than worked out by comparing the two statuses. An
 * event that changed no status and an event whose status came back the same would be
 * indistinguishable otherwise, and only one of them is a citation.
 */
function Event({ event }: { event: MemoryEvent }) {
  return (
    <div style={{ display: 'grid', gap: 6 }}>
      <div className="claim__sentence">
        <Chip tone="quiet" title={memoryOperationGloss(event.operation)}>
          {event.operation}
        </Chip>
        <span className="claim__verb">
          {event.changes_status
            ? `${event.from_status ?? 'nothing'} → ${event.to_status}`
            : 'no status changed'}
        </span>
        <span className="hash">{stamp(event.at)}</span>
      </div>
      <p className="prose">{event.operation_note}</p>
      <p className="prose">{memoryOperationGloss(event.operation)}</p>
      {event.note === null ? null : <p className="prose wrapany">“{event.note}”</p>}
    </div>
  );
}

/**
 * One note, with the stored half and the read-time half kept visibly apart.
 *
 * The head carries two labelled groups rather than one row of chips, and the body carries two
 * labelled definition lists. That repetition is the design: a reader has to be able to say, of any
 * value on this card, whether a command wrote it or a query worked it out — and on a screen where
 * a note may be years old, the difference between *this is what we recorded* and *this is what is
 * true of it now* is the whole reading.
 */
function RecordCard({ record, linked }: { record: MemoryRecord; linked: boolean }) {
  const chain = supersession(record);
  const id = (
    <span className="claim__party claim__party--subject wrapany">{record.memory_id}</span>
  );
  // Drawn from the one place that reads the stored vocabulary, rather than from a comparison
  // spelled here. A second copy of "which statuses are current" would be the one on screen.
  const tone = statusTone(record.status);
  return (
    <div className={tone === 'fresh' ? 'claim' : 'claim claim--unresolved'}>
      <div className="claim__head">
        <div className="claim__sentence">
          {linked ? (
            <a
              className="claim__party claim__party--subject wrapany"
              href={memoryFragment({ record: record.memory_id })}
            >
              {record.memory_id}
            </a>
          ) : (
            id
          )}
          <Chip tone="quiet" title={memoryScopeGloss(record.scope)}>
            {record.scope}
          </Chip>
          {record.claim_key === null ? (
            <span className="hash">answers no named question</span>
          ) : (
            <Chip
              tone="quiet"
              title="Two notes can only disagree when they answer the same named question about the same subject in the same scope."
            >
              answers “{record.claim_key}”
            </Chip>
          )}
        </div>

        {/*
          The split, drawn as two groups with different rules beside them rather than as two hues.
          Hue on this screen means a claim's state; what separates these is a different question —
          whether a command wrote the value or a query worked it out — so it is drawn as a
          different kind of line, and each group is labelled with which kind it is.
        */}
        <div className="kinds">
          <div className="kind kind--stored">
            <span className="kind__label micro">stored</span>
            <Chip tone={tone} title={record.status_note}>
              {record.status}
            </Chip>
          </div>
          <div className="kind kind--derived">
            <span className="kind__label micro">worked out when read</span>
            {record.views.length === 0 ? (
              <span className="hash">nothing qualifies this note right now</span>
            ) : (
              record.views.map((view) => (
                <Chip key={view.view} tone={viewTone(view.view)} title={view.note}>
                  {view.view}
                </Chip>
              ))
            )}
            <Chip
              tone={subjectTone(record.subject_resolution)}
              title={record.subject_resolution_note}
            >
              subject · {record.subject_resolution}
            </Chip>
          </div>
        </div>
      </div>

      <div className="claim__body" style={{ display: 'grid', gap: 10 }}>
        <div style={{ display: 'grid', gap: 4 }}>
          <div className="micro">the note, as it was written</div>
          <p className="prose wrapany">{record.content}</p>
        </div>

        <p className="prose">{record.status_note}</p>
        <p className="prose">{memoryStatusGloss(record.status)}</p>
        {record.views.map((view) => (
          <p className="prose" key={view.view}>
            <strong>{view.view}</strong> — {memoryViewGloss(view.view)}
          </p>
        ))}
        {record.scope_note === null ? (
          <p className="prose">
            This note stores a scope outside the four this build knows, so there is no sentence for
            it. It is shown as it was stored rather than corrected.
          </p>
        ) : (
          <p className="prose">{record.scope_note}</p>
        )}
        {record.invalidation_reason === null ? null : (
          <p className="prose wrapany">
            <strong>Why it ended:</strong> {record.invalidation_reason} — and nothing replaced it,
            which is a different thing from being superseded.
          </p>
        )}

        <div className="micro">stored — written by a command</div>
        <Defs>
          <Def term="subject, as recorded">
            <span className="hash wrapany">{record.subject.path || record.subject.name}</span>
            <span className="hash"> · {record.subject.kind}</span>
          </Def>
          <Def term="named as">
            <span className="hash wrapany">{record.subject.selector}</span>
          </Def>
          <Def term="its id then">
            <span className="hash wrapany">{record.subject.entity_id}</span>
          </Def>
          <Def term="scope">{record.scope}</Def>
          <Def term="claim key">{record.claim_key ?? 'none — this note answers no named question'}</Def>
          <Def term="confirmed against">
            <span className="hash wrapany">{record.anchor_state_id}</span>
          </Def>
          <Def term="written by">
            <span className="wrapany">{record.author_label}</span>
            <span className="hash"> · a label the caller typed, never an identity Nerve checked</span>
          </Def>
          <Def term="written">{stamp(record.created_at)}</Def>
          <Def term="replaces">
            {chain.replaces === null ? (
              'nothing — this note is not a replacement'
            ) : (
              <a className="link wrapany" href={memoryFragment({ record: chain.replaces })}>
                {chain.replaces}
              </a>
            )}
          </Def>
          {record.invalidated_at === null ? null : (
            <Def term="ended">{stamp(record.invalidated_at)}</Def>
          )}
        </Defs>

        <div className="micro">worked out when this answer was assembled — nothing stores these</div>
        <Defs>
          <Def term="subject now">
            {record.subject_resolution} — {record.subject_resolution_note}
          </Def>
          <Def term="what that means">
            {memorySubjectResolutionGloss(record.subject_resolution)}
          </Def>
          <Def term="reaches">
            {record.subject_live_entity_ids.length === 0 ? (
              'nothing in the index now'
            ) : (
              <>
                {record.subject_live_entity_ids.map((entity) => (
                  <span key={entity} className="hash wrapany">
                    {entity}{' '}
                  </span>
                ))}
                {record.subject_live_entity_ids.length > 1
                  ? '— every candidate is shown and none is preferred'
                  : ''}
              </>
            )}
          </Def>
          <Def term="index state now">
            <span className="hash wrapany">
              {record.current_state_id ?? 'nothing is indexed here'}
            </span>
          </Def>
          <Def term="read beside the anchor">
            {anchorReading(record.anchor_state_id, record.current_state_id)}
          </Def>
          <Def term="replaced by">
            {chain.replacedBy === null ? (
              // Read off the stored column rather than off the status word: a note that ended and
              // a note that is simply the newest in its chain are different answers, and only one
              // of them has an ending recorded.
              record.invalidated_at === null ? (
                'nothing — this is the latest note in its chain'
              ) : (
                'nothing — it was ended rather than replaced'
              )
            ) : (
              <a className="link wrapany" href={memoryFragment({ record: chain.replacedBy })}>
                {chain.replacedBy}
              </a>
            )}
          </Def>
          <Def term="views">
            {record.views.length === 0
              ? 'none right now'
              : record.views.map((view) => view.view).join(' · ')}
          </Def>
        </Defs>

        <div className="micro">
          citations — {count(record.citations.length)}
          {record.citations.length === 0 ? ' (none was attached)' : ''}
        </div>
        {record.citations.length === 0 ? null : (
          <div style={{ display: 'grid', gap: 6 }}>
            {record.citations.map((citation, index) => (
              <Citation key={citation.citation_id ?? index} citation={citation} />
            ))}
          </div>
        )}

        <div className="micro">
          history — {count(record.events.length)}, and nothing removes one
        </div>
        <div className="spine">
          {record.events.map((event, index) => (
            <Event key={event.event_id ?? index} event={event} />
          ))}
        </div>
      </div>
    </div>
  );
}

/** What the answer rests on, printed rather than implied. */
function AnswerScope({ block }: { block: MemoryBlock }) {
  return (
    <div style={{ display: 'grid', gap: 8 }}>
      <p className="prose">
        {count(block.records_matching)} of {count(block.records_in_repository)} note(s) in this
        repository match this question. {pageReading(block.truncation)}
      </p>
      <div className="row row--wrap">
        <span className="micro">repository</span>
        <span className="hash wrapany">{block.repository_id}</span>
        <span className="micro">state</span>
        <span className="hash wrapany">
          {block.current_repository_state ?? 'nothing is indexed here'}
        </span>
      </div>
    </div>
  );
}

/**
 * The two absences, headed differently and answered differently.
 *
 * A screen that headed both "no results" would report *nobody has written anything here* as
 * *your filters matched nothing*, which have opposite next steps.
 */
function Absence({ block }: { block: MemoryBlock }) {
  return (
    <Panel title="No note is on this answer">
      <Empty
        title={absenceHeading(block.result_kind)}
        body={block.absence_statement ?? 'This answer carries no record.'}
      >
        {block.result_kind === 'no_memory_matches' ? (
          <p className="state__body">
            {count(block.records_in_repository)} note(s) exist here and this question matches none
            of them.{' '}
            <a className="link" href={memoryFragment({})}>
              Show every note
            </a>
            .
          </p>
        ) : (
          <p className="state__body">
            Nothing is discovered here. A note exists because a person wrote one, so the way this
            screen stops being empty is the command below.
          </p>
        )}
      </Empty>
    </Panel>
  );
}

function RecordList({ options }: { options: Record<string, string> }) {
  const offset = Number(options['offset'] ?? '0') || 0;
  const scope = options['scope'] ?? '';
  const status = options['status'] ?? '';
  const q = options['q'] ?? '';
  const subject = options['subject'] ?? '';

  const params = useMemo(() => {
    const built: Record<string, string | number> = { limit: PAGE, offset };
    if (scope !== '') built['scope'] = scope;
    if (status !== '') built['status'] = status;
    if (q !== '') built['q'] = q;
    if (subject !== '') built['subject'] = subject;
    return built;
  }, [offset, scope, status, q, subject]);

  const { state, reload } = useApi<MemoryBlock>('/api/memory', params);

  if (state.status === 'loading') return <Loading label="Reading the notes" />;
  if (state.status === 'error') {
    return (
      <div className="stack">
        <Failure error={state.error} onRetry={reload} />
        <p className="prose">
          A scope or a status outside the closed set is refused by name rather than answered with an
          empty list, because an empty list would say <em>there are no notes</em> when what is true
          is <em>there is no such value</em>.{' '}
          <a className="link" href={memoryFragment({})}>
            Clear every filter
          </a>
          .
        </p>
      </div>
    );
  }

  const block = state.data;
  // Narrowed once, here, so the pager's own numbers cannot be read off a block that has none.
  const pager = hasPager(block.truncation, offset) ? block.truncation : null;
  return (
    <>
      <Filters block={block} options={options} />

      <Panel
        title="Notes"
        aside={
          <span className="hash">
            {count(block.records.length)} shown of {count(block.records_matching)}
          </span>
        }
      >
        <AnswerScope block={block} />
      </Panel>

      {block.records.length === 0 ? (
        <Absence block={block} />
      ) : (
        <section className="panel">
          <div className="panel__body panel__body--flush">
            <div className="spine">
              {block.records.map((record) => (
                <RecordCard key={record.memory_id} record={record} linked />
              ))}
            </div>
          </div>
          {/*
            On every page of a cut answer, not only on the ones that were themselves cut. The last
            page of a cut answer is not truncated, and gating the pager on that flag left a reader
            looking at records 51–55 with no way back — found by the viewport QA run, not by eye.
          */}
          {pager === null ? null : (
            <div className="graph__foot">
              <a
                className={offset === 0 ? 'btn btn--ghost' : 'btn'}
                aria-disabled={offset === 0 ? 'true' : undefined}
                href={memoryFragment({
                  ...options,
                  offset: String(Math.max(0, offset - PAGE)),
                })}
              >
                previous
              </a>
              <span>
                {count(offset + 1)}–{count(offset + block.records.length)} of {count(pager.total)}
              </span>
              <a
                className={block.continuation.next_offset === null ? 'btn btn--ghost' : 'btn'}
                aria-disabled={block.continuation.next_offset === null ? 'true' : undefined}
                href={memoryFragment({
                  ...options,
                  offset: String(block.continuation.next_offset ?? offset),
                })}
              >
                next
              </a>
            </div>
          )}
        </section>
      )}

      <Limitations block={block} />
      <Boundary statement={block.boundary.statement} commands={block.boundary.commands} />
    </>
  );
}

/**
 * One note in full.
 *
 * A note that is not here is a refusal and never an empty record: *there is no such note* and *the
 * note says nothing* are different answers, and only one of them is a `404`.
 */
function OneRecord({ id }: { id: string }) {
  const params = useMemo(() => ({ memory_id: id }), [id]);
  const { state, reload } = useApi<MemoryBlock>('/api/memory/record', params);

  const back = (
    <a className="link" href={memoryFragment({})}>
      back to every note
    </a>
  );

  if (state.status === 'loading') return <Loading label="Reading one note" />;
  if (state.status === 'error') {
    return (
      <div className="stack">
        <Failure error={state.error} onRetry={reload} />
        <div>{back}</div>
      </div>
    );
  }

  const block = state.data;
  const record = block.records[0];
  if (record === undefined) {
    return (
      <div className="stack">
        <Absence block={block} />
        <div>{back}</div>
      </div>
    );
  }

  return (
    <>
      <section className="panel">
        <header className="panel__head">
          <h2 className="micro">One note</h2>
          {back}
        </header>
        <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
          <AnswerScope block={block} />
          {block.continuation.statement === null ? null : (
            <p className="prose">{block.continuation.statement}</p>
          )}
        </div>
        <div className="panel__body panel__body--flush">
          <div className="spine">
            <RecordCard record={record} linked={false} />
          </div>
        </div>
      </section>

      <Limitations block={block} />
      <Boundary statement={block.boundary.statement} commands={block.boundary.commands} />
    </>
  );
}

export function Memory({ options }: { options: Record<string, string> }) {
  const chosen = options['record'] ?? '';
  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">Memory</h1>
        <p className="head__sub">
          What a person wrote down about this repository, kept beside the evidence and never mixed
          into it. Nothing here was discovered: every note exists because somebody typed it at the
          command line. Each one carries what was recorded and, separately, what is true of it now —
          because a note may be older than the code it is about.
        </p>
      </div>

      <div className="stack">
        {chosen === '' ? <RecordList options={options} /> : <OneRecord id={chosen} />}
      </div>
    </div>
  );
}
