/**
 * The pieces every history screen is built from.
 *
 * They live in one file because they are shared by five screens, and because two of them carry
 * rules rather than layout:
 *
 * - `Availability` renders the block the API puts on **every** history answer. A history answer
 *   without it is a list of dates with no statement of what they are dates of — how much was
 *   read, where the reading stopped, and whether what was read still describes what is indexed
 *   now. It is not a footnote and it is not collapsed behind a disclosure.
 * - `CommitCard` never says a commit is the beginning of anything unless the response said it
 *   may. The permission is `may_claim_history_begins_here`, computed once in `nerve-core`; this
 *   file reads it and does not recompute it.
 *
 * Every string that came from the repository — a summary, a path, an identity, a refusal key — is
 * interpolated as a React child, which React escapes. `fixtures/history-hostile` carries a
 * script tag in a commit summary, and it renders here as the text it is.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';

import { ApiError } from '../api/client';
import type {
  HistoryBlock,
  HistoryChange,
  HistoryCommit,
  HistoryRename,
  Json,
} from '../api/types';
import { count, type Tone } from '../format';
import { changeCountReading, commitBoundary, fileMode, gitTime } from '../history';
import { useDebounced } from '../hooks';
import { Chip, Def, Defs, Failure, Figure, Tally } from '../ui/parts';
import {
  changeKindGloss,
  changesEnumeratedGloss,
  historyFreshnessGloss,
  parentCompletenessGloss,
  renameAmbiguityGloss,
  renameEvidenceGloss,
  walkTerminationGloss,
} from '../vocab';

/**
 * Hue for why the walk stopped.
 *
 * `commit_budget` is quiet rather than alarming on purpose: it is Nerve's own ceiling, and
 * colouring it as a fault would draw a decision this tool made as damage to the repository.
 * `missing_object` and `refused` are the two that want reading. There is no `default` arm — an
 * unrecognised value must not borrow a real member's colour.
 */
export function walkTone(value: string | null): Tone {
  switch (value) {
    case 'exhausted':
      return 'fresh';
    case 'commit_budget':
      return 'quiet';
    case 'shallow_boundary':
      return 'absent';
    case 'missing_object':
      return 'stale';
    case 'refused':
      return 'stale';
  }
  return 'unknown';
}

/** Hue for the history freshness verdict. `unverifiable` is never drawn as `current`. */
export function historyFreshnessTone(value: string): Tone {
  switch (value) {
    case 'current':
      return 'fresh';
    case 'stale':
      return 'stale';
    case 'unverifiable':
      return 'unknown';
    case 'no_history_ingested':
      return 'absent';
  }
  return 'unknown';
}

/** Hue for a parent situation. The two that mean "cannot see further" are not the same hue. */
export function completenessTone(value: string): Tone {
  switch (value) {
    case 'root':
      return 'fresh';
    case 'parents_available':
      return 'quiet';
    case 'shallow_boundary':
      return 'absent';
    case 'parents_missing':
      return 'stale';
    case 'parents_unverifiable':
      return 'unknown';
  }
  return 'unknown';
}

/** Hue for a change kind. `mode_changed` is not a content change and is not coloured as one. */
export function changeTone(value: string): Tone {
  switch (value) {
    case 'added':
      return 'fresh';
    case 'modified':
      return 'quiet';
    case 'deleted':
      return 'stale';
    case 'mode_changed':
      return 'absent';
  }
  return 'unknown';
}

/**
 * What was read, where the reading stopped, and whether it still describes what is indexed.
 *
 * The four tallies are `null` — not zero — when history has never been read, and this component
 * renders nothing in their place rather than a row of zeroes. That is the same discipline the
 * coverage screen applies to `totals: null`, in a second place: a zero is a measurement, and
 * nothing here was measured.
 */
export function Availability({ block }: { block: HistoryBlock }) {
  const totals = block.totals;
  const refusals = Object.entries(block.refusals ?? {}).sort((a, b) => b[1] - a[1]);
  const earlier = block.limitations.earlier_changes_may_exist;

  return (
    <section className="panel">
      <div className="panel__body" style={{ display: 'grid', gap: 12 }}>
        <div className="row row--wrap">
          <Chip
            tone={block.history_ingested ? 'quiet' : 'absent'}
            title={
              block.history_ingested
                ? 'A history ingest is recorded for this repository.'
                : 'No history ingest is recorded. That is an absence, not a repository without commits.'
            }
          >
            <span className="chip__dot" />
            {block.history_ingested ? 'history read' : 'history never read'}
          </Chip>
          {block.shallow === true ? (
            <Chip tone="absent" title={parentCompletenessGloss('shallow_boundary')}>
              shallow checkout
            </Chip>
          ) : null}
          {block.promisor === true ? (
            <Chip
              tone="absent"
              title="A partial clone: objects are fetched on demand, so an absent object here is not necessarily a fault."
            >
              promisor clone
            </Chip>
          ) : null}
          {block.walk_terminated_by === null ? null : (
            <Chip
              tone={walkTone(block.walk_terminated_by)}
              title={walkTerminationGloss(block.walk_terminated_by)}
            >
              stopped: {block.walk_terminated_by}
            </Chip>
          )}
          <Chip
            tone={historyFreshnessTone(block.freshness)}
            title={historyFreshnessGloss(block.freshness)}
          >
            {block.freshness}
          </Chip>
          <span className="spacer" />
          <span className="head__sub">{block.result_kind}</span>
        </div>

        {block.walk_terminated_note === null ? null : (
          <p className="prose">{block.walk_terminated_note}</p>
        )}
        <p className="prose">{block.freshness_note}</p>

        {totals === null ? (
          <p className="prose">
            No tally is shown because none was taken. Printing zeroes here would report &ldquo;never
            read&rdquo; as &ldquo;read, and found nothing&rdquo;, which are different answers.
          </p>
        ) : (
          <>
            <div className="figures">
              <Figures totals={totals} block={block} />
            </div>
            {Object.keys(totals.changes_by_kind).length > 0 ? (
              <Tally
                rows={Object.entries(totals.changes_by_kind).sort(
                  (a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1),
                )}
              />
            ) : null}
          </>
        )}

        <Defs>
          <Def term="earlier changes">
            {earlier === null
              ? 'Unanswerable: the question is about an ingest, and there has not been one.'
              : earlier
                ? 'Earlier commits may exist above what was read. The counts on this screen are a floor, never a total.'
                : 'The walk ran out of parents rather than stopping at a boundary, so nothing above what it read is hidden from these counts.'}
          </Def>
          {block.limitations.merges_in_repository === null ? null : (
            <Def term="merges">
              {count(block.limitations.merges_in_repository)} recorded. A merge enumerates no
              changes, so every change count here is short by whatever those merges did.
            </Def>
          )}
          {block.shallow_boundary === null || block.shallow_boundary.length === 0 ? null : (
            <Def term="boundary">
              {block.shallow_boundary.map((oid) => (
                <span key={oid} className="hash wrapany">
                  {oid}{' '}
                </span>
              ))}
            </Def>
          )}
          {refusals.length === 0 ? null : (
            <Def term="refusals">
              {/*
                Repository-derived keys, rendered as text. The durable record of a refusal is
                `changes_enumerated: refused` on the commit itself — these counts describe the
                ingest that produced them, so a re-sync with no tree work reports none.
              */}
              {refusals.map(([form, tally]) => (
                <span key={form} className="wrapany">
                  {form} · {count(tally)}{' '}
                </span>
              ))}
            </Def>
          )}
          <Def term="read by">
            {block.reader_version === null ? 'no reader has run here' : block.reader_version}
            {block.ingest_head_oid === null ? null : (
              <>
                {' '}
                at <span className="hash wrapany">{block.ingest_head_oid}</span>
              </>
            )}
          </Def>
          <Def term="indexed state">
            {block.current_repository_state.git_commit === null ? (
              'the newest indexed state records no commit'
            ) : (
              <span className="hash wrapany">{block.current_repository_state.git_commit}</span>
            )}
          </Def>
        </Defs>
      </div>
    </section>
  );
}

function Figures({ totals, block }: { totals: NonNullable<HistoryBlock['totals']>; block: HistoryBlock }) {
  return (
    <>
      <Figure label="commits" value={count(totals.commits)} note="recorded in this ingest" />
      <Figure label="changes" value={count(totals.changes)} note="rows across those commits" />
      <Figure label="renames" value={count(totals.renames)} note="hypotheses, never records" />
      <Figure
        label="budget"
        value={block.commit_budget === null ? 'none' : count(block.commit_budget)}
        note={
          block.commits_recorded === null
            ? 'no ingest'
            : `${count(block.commits_recorded)} read against it`
        }
      />
    </>
  );
}

/**
 * What this one answer covered, and what it could not offer.
 *
 * `truncated` is the server's own comparison against a counted total, never `len() === limit` —
 * which is wrong exactly when an answer ends on the boundary, the case a reader most needs to be
 * right. Where no continuation exists the API sends a sentence saying why, and it is shown rather
 * than replaced by a greyed-out button.
 */
export function AnswerScope({ block }: { block: HistoryBlock }) {
  const truncation = block.truncation;
  if (truncation === null) return null;
  return (
    <div className="row row--wrap">
      <Chip tone={truncation.truncated ? 'stale' : 'quiet'}>
        {count(truncation.returned)} shown
        {truncation.total === null ? '' : ` of ${count(truncation.total)}`}
      </Chip>
      {truncation.truncated ? (
        <Chip tone="stale" title={`One answer carries at most ${count(truncation.limit)} rows.`}>
          cut at {count(truncation.limit)}
        </Chip>
      ) : null}
      {block.continuation.supported ? null : (
        <span className="head__sub wrapany">{block.continuation.statement}</span>
      )}
    </div>
  );
}

/**
 * One commit.
 *
 * The summary is repository prose and the first free-form repository text Nerve stores — it is
 * attacker-influencable wherever contributions are accepted, and it is rendered as a child here
 * and nowhere as markup. The boundary label comes from `commitBoundary`, which reads the carried
 * permission; a shallow boundary therefore reads as the earliest thing visible in this checkout,
 * and never as a first commit.
 */
export function CommitCard({
  commit,
  href,
  aside,
}: {
  commit: HistoryCommit;
  href?: string;
  aside?: ReactNode;
}) {
  const boundary = commitBoundary(commit);
  const changes = changeCountReading(commit.changes);
  const body = (
    <>
      <div className="row row--wrap">
        <span className="hash">{commit.commit_oid.slice(0, 12)}</span>
        <Chip
          tone={completenessTone(commit.parent_completeness)}
          title={parentCompletenessGloss(commit.parent_completeness)}
        >
          {commit.parent_completeness}
        </Chip>
        {commit.is_merge ? (
          <Chip tone="absent" title={changesEnumeratedGloss('merge_not_enumerated')}>
            merge
          </Chip>
        ) : null}
        {boundary === null ? null : (
          <Chip
            tone={boundary.begins ? 'fresh' : 'absent'}
            title={commit.parent_completeness_note}
          >
            {boundary.label}
          </Chip>
        )}
        <span className="spacer" />
        {aside}
      </div>
      <div className="fact__value wrapany">{commit.summary}</div>
      <div className="hash wrapany">
        {gitTime(commit.committer_time, commit.committer_tz)}
        {commit.committer_ident === null ? '' : ` · ${commit.committer_ident}`}
      </div>
      <div className="row row--wrap">
        <Chip
          tone={changes.counted ? 'quiet' : 'absent'}
          title={changesEnumeratedGloss(commit.changes_enumerated)}
        >
          {changes.text}
        </Chip>
        <span className="head__sub wrapany">{commit.changes_enumerated_note}</span>
      </div>
      {commit.change === undefined ? null : <ChangeLine change={commit.change} />}
    </>
  );

  if (href === undefined) {
    return (
      <div className="obs obs--direct" style={{ gap: 6 }}>
        {body}
      </div>
    );
  }
  return (
    <a className="obs obs--direct" style={{ gap: 6 }} href={href}>
      {body}
    </a>
  );
}

/** One change, on one line — the shape a path's own history and a commit's change list share. */
export function ChangeLine({ change, showCommit }: { change: HistoryChange; showCommit?: boolean }) {
  return (
    <div className="row row--wrap">
      <Chip tone={changeTone(change.change_kind)} title={changeKindGloss(change.change_kind)}>
        {change.change_kind}
      </Chip>
      <span className="wrapany">{change.path}</span>
      {change.change_kind === 'mode_changed' ? (
        <span className="hash">
          {fileMode(change.prev_mode)} → {fileMode(change.mode)}
        </span>
      ) : null}
      {showCommit === true && change.commit_oid !== undefined ? (
        <span className="hash">{change.commit_oid.slice(0, 12)}</span>
      ) : null}
    </div>
  );
}

export function ChangeList({
  changes,
  showCommit,
}: {
  changes: HistoryChange[];
  showCommit?: boolean;
}) {
  return (
    <div className="spine">
      {changes.map((change, index) => (
        <div
          className="obs obs--direct"
          key={`${change.commit_oid ?? ''}-${change.path}-${index}`}
          style={{ gap: 4 }}
        >
          <ChangeLine change={change} showCommit={showCommit} />
          <div className="hash wrapany">
            {change.prev_blob_oid === null ? 'no previous blob' : change.prev_blob_oid.slice(0, 12)}{' '}
            → {change.blob_oid === null ? 'no blob' : change.blob_oid.slice(0, 12)}
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * Rename hypotheses.
 *
 * Every row says it is a hypothesis, carries what it rests on, and carries how many ways it could
 * have been drawn — and there is no score anywhere, because none exists. When one blob matches
 * several paths every pairing is recorded and none is promoted, so a `many_to` row must not be
 * drawn the way a `unique` one is.
 */
export function RenameList({ renames }: { renames: HistoryRename[] }) {
  return (
    <div className="spine">
      {renames.map((row, index) => (
        <div
          className={row.ambiguity === 'unique' ? 'obs obs--resolved' : 'obs obs--inferred'}
          key={`${row.commit_oid}-${row.from_path}-${row.to_path}-${index}`}
          style={{ gap: 5 }}
        >
          <div className="row row--wrap">
            <Chip tone="unknown" title="Git records no rename. This is a proposal drawn from identical content.">
              hypothesis
            </Chip>
            <Chip
              tone={row.ambiguity === 'unique' ? 'quiet' : 'absent'}
              title={renameAmbiguityGloss(row.ambiguity)}
            >
              {row.ambiguity}
            </Chip>
            <Chip tone="quiet" title={renameEvidenceGloss(row.evidence)}>
              {row.evidence}
            </Chip>
            <span className="hash">{row.commit_oid.slice(0, 12)}</span>
          </div>
          <div className="fact__value wrapany">
            {row.from_path} → {row.to_path}
          </div>
          <div className="head__sub wrapany">{row.ambiguity_note}</div>
          <div className="hash wrapany">blob {row.blob_oid.slice(0, 12)}</div>
        </div>
      ))}
    </div>
  );
}

function detailField(detail: Json, key: string): Json {
  if (detail === null || typeof detail !== 'object' || Array.isArray(detail)) return null;
  return (detail as Record<string, Json>)[key] ?? null;
}

/**
 * A history refusal, with the part of it that is easiest to misread stated in words.
 *
 * Two of these carry a flag whose whole purpose is to stop a refusal being read as an empty
 * answer: `this_is_not_an_empty_commit` on an unrecorded commit, and `nothing_was_looked_up` on a
 * symbol-shaped path. The generic failure card shows the code and the message; this adds the
 * sentence the refusal itself carries, and never offers a nearby path instead — answering with the
 * containing file would be a different claim wearing the same words.
 */
export function HistoryRefusal({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const detail = error instanceof ApiError ? error.detail : null;
  const statement = detailField(detail, 'reason_statement');
  const notEmpty = detailField(detail, 'this_is_not_an_empty_commit') === true;
  const nothingLookedUp = detailField(detail, 'nothing_was_looked_up') === true;

  return (
    <div className="stack" style={{ gap: 10 }}>
      <Failure error={error} onRetry={onRetry} />
      {typeof statement === 'string' ? <p className="prose">{statement}</p> : null}
      {notEmpty ? (
        <p className="prose">
          This is a commit Nerve never read, which is a different answer from a commit that changed
          nothing. Only the second of those is an empty change list.
        </p>
      ) : null}
      {nothingLookedUp ? (
        <p className="prose">
          Nothing was looked up and no path was guessed. History is keyed on a path, so the only
          dates available for a symbol would be its file&apos;s dates — a different claim in the
          same words.
        </p>
      ) : null}
    </div>
  );
}

/**
 * A text field that writes what was typed into the route once it settles.
 *
 * The route is replaced rather than pushed, so typing a path does not turn the back button into a
 * per-character undo — the same trade the search box makes.
 */
export function RouteField({
  label,
  placeholder,
  value,
  onSettled,
  wide,
}: {
  label: string;
  placeholder: string;
  value: string;
  onSettled: (next: string) => void;
  wide?: boolean;
}) {
  const [text, setText] = useState(value);
  const latest = useRef(onSettled);
  latest.current = onSettled;

  useEffect(() => {
    setText(value);
  }, [value]);

  const settled = useDebounced(text.trim(), 220);
  useEffect(() => {
    if (settled !== value) latest.current(settled);
  }, [settled, value]);

  return (
    <>
      <span className="micro">{label}</span>
      <div className="field" style={{ flex: wide === true ? '1 1 320px' : '1 1 180px' }}>
        <input
          type="text"
          value={text}
          spellCheck={false}
          autoComplete="off"
          aria-label={label}
          placeholder={placeholder}
          onChange={(event) => setText(event.target.value)}
        />
      </div>
    </>
  );
}
