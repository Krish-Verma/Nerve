/**
 * Impact — what depends on a symbol, and what the answer cannot see.
 *
 * `/api/impact` has existed since Slice 7b with no view. The endpoint is not hard to render; the
 * screen is hard to render *honestly*, and one rule decides the whole layout.
 *
 * **The caveat is not a footnote.** A reverse closure returning three rows reads as "three things
 * depend on this, it is safe to change" — a claim about the repository, drawn from a traversal that
 * could only follow the edges Nerve resolved. Nerve has no type inference, so `shape.area()` is
 * recorded as unresolved rather than guessed at; Slice 2a measured 38.1 % of call sites on the
 * resolution corpus as unresolved. On `fixtures/ts-basic`, `add` has three dependants beside four
 * unresolved sites. So the unresolved account sits **above** the results, is rendered on every
 * answer including the one where every count is zero, and its zero case is a sentence rather than
 * an omission — that is the case where silence most invites the wrong conclusion.
 *
 * Three more rules, each of which is a way to mislead while looking correct:
 *
 * - **The relation set is read off the answer.** An empty request means five specific relations,
 *   not "every relation"; following `CONTAINS` would answer that every symbol impacts the
 *   repository. The set that was actually walked is on screen, because "nothing depends on this"
 *   is only true relative to it.
 * - **Trace edges were not followed, and that is a decision.** A trace says one run took an edge,
 *   so a blast radius built on it would grow and shrink with which tests happened to run. Named on
 *   screen so its absence cannot be read as an oversight.
 * - **This is not "affected tests".** `nerve affected` is refused rather than deferred — LCOV
 *   carries no per-test attribution (ADR-0008 §A.2). A test file appears here because code depends
 *   on code, and nothing on this screen may be labelled as test impact.
 *
 * Every string is repository content: a symbol name, a file path, an unresolved category. All of it
 * is interpolated as a React child, which React escapes. Nothing here builds markup from a string.
 */

import { useEffect, useMemo, useState } from 'react';

import type { Entity, ImpactReport, ImpactRow } from '../api/types';
import { count, freshness, relationPhrase, shortHash } from '../format';
import { useApi, useDebounced } from '../hooks';
import {
  bounded,
  DEFAULT_DEPTH,
  DEFAULT_LIMIT,
  depthRows,
  emptyReading,
  exclusionReading,
  grainReading,
  MAX_DEPTH,
  MAX_LIMIT,
  pageReading,
  rowTone,
  staleReading,
  tallyRows,
  unresolvedReading,
  unresolvedTone,
} from '../impact';
import { entityHref, href, replaceRoute } from '../routing';
import {
  Chip,
  Empty,
  EntityLink,
  Failure,
  Figure,
  Loading,
  Panel,
  SelectorReading,
  Tally,
  Where,
} from '../ui/parts';
import { unresolvedCategory } from '../vocab';

/** The depths offered. The server clamps to 32 whatever is asked for, and echoes what it applied. */
const DEPTHS = [1, 2, 6, 12];

/** The page sizes offered. Caps rows only — every tally stays exact whatever this cuts. */
const LIMITS = [25, 50, 200];

export function Impact({ options }: { options: Record<string, string> }) {
  const subject = options['subject'] ?? '';
  const [text, setText] = useState(subject);

  useEffect(() => {
    setText(subject);
  }, [subject]);

  const settled = useDebounced(text.trim(), 220);
  useEffect(() => {
    if (settled !== subject) {
      replaceRoute(href({ view: 'impact', options: { ...options, subject: settled } }));
    }
    // Rewritten only when the settled text differs. `options` changes identity on every render and
    // must not retrigger this, which is the same exemption `PathFinder` takes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settled, subject]);

  const set = (next: Record<string, string>) =>
    replaceRoute(href({ view: 'impact', options: { ...options, ...next } }));

  const depth = bounded(options['max_depth'] ?? '', MAX_DEPTH) ?? DEFAULT_DEPTH;
  const limit = bounded(options['limit'] ?? '', MAX_LIMIT) ?? DEFAULT_LIMIT;

  const params = useMemo(
    () => ({ subject: settled, max_depth: depth, limit }),
    [settled, depth, limit],
  );
  const { state, reload } = useApi<ImpactReport>(
    settled.length > 0 ? '/api/impact' : null,
    params,
  );

  return (
    <div className="view">
      <section className="panel">
        <div className="graph__controls">
          <span className="micro">what depends on</span>
          <div className="field" style={{ flex: '1 1 280px' }}>
            <input
              type="text"
              value={text}
              spellCheck={false}
              autoComplete="off"
              aria-label="The symbol to measure the impact of"
              placeholder="a name, src/file.ts#Name, or an entity id"
              onChange={(event) => setText(event.target.value)}
            />
          </div>
          <span className="micro">within</span>
          <div className="seg" role="group" aria-label="How many hops of the closure to walk">
            {DEPTHS.map((value) => (
              <button
                key={value}
                type="button"
                aria-pressed={depth === value}
                onClick={() => set({ max_depth: String(value) })}
              >
                {value} deep
              </button>
            ))}
          </div>
          <span className="micro">showing</span>
          <div className="seg" role="group" aria-label="How many rows to list">
            {LIMITS.map((value) => (
              <button
                key={value}
                type="button"
                aria-pressed={limit === value}
                onClick={() => set({ limit: String(value) })}
              >
                {value} rows
              </button>
            ))}
          </div>
        </div>
      </section>

      {settled.length === 0 ? (
        <Panel title="Impact">
          <Empty
            title="Name a symbol"
            body="Nerve walks the assertion graph backwards from it and reports everything that reaches it — with the evidence for each edge, and with the account of the reference sites that resolved to nothing and could therefore be hiding a dependency this answer cannot see."
          />
        </Panel>
      ) : state.status === 'loading' ? (
        <Loading label="Walking the closure backwards" />
      ) : state.status === 'error' ? (
        <Failure error={state.error} onRetry={reload} />
      ) : (
        <Report report={state.data} />
      )}
    </div>
  );
}

function Report({ report }: { report: ImpactReport }) {
  // The rows on this page, by id, so an edge's far end can be named rather than printed as a hash.
  // Only the page — a truncated answer legitimately points at members that were cut, and those are
  // shown as the ids they are rather than guessed at.
  const onThisPage = useMemo(() => {
    const map = new Map<string, Entity>();
    for (const row of report.results) map.set(row.entity.entity_id, row.entity);
    return map;
  }, [report.results]);

  return (
    <>
      <Panel
        title="The symbol"
        aside={<span className="hash">{report.subject.kind}</span>}
      >
        <div className="stack" style={{ gap: 10 }}>
          <div className="row row--wrap">
            <EntityLink entity={report.subject} />
            <Where path={report.subject.file_path} line={report.subject.start_line} />
          </div>
          <SelectorReading
            selectors={report.selectors}
            parameter="subject"
            chosen={report.subject}
          />
        </div>
      </Panel>

      {/*
        Above the results, never below them. This is the panel that decides whether the number in
        the next panel may be read as a clearance, so a reader who stops after two panels has still
        been told the thing that changes the answer.
      */}
      <Unresolved report={report} />

      <Panel
        title="The closure"
        aside={
          <span className="hash">
            {count(report.totals.entities)} {report.totals.entities === 1 ? 'entity' : 'entities'}
          </span>
        }
      >
        <div className="stack" style={{ gap: 12 }}>
          <div className="figures">
            <Figure
              label="depend on it"
              value={count(report.totals.entities)}
              note="The size of the whole closure, not of the page below."
            />
            <Figure
              label="depth walked"
              value={count(report.max_depth)}
              note="As applied by the server after clamping, not as requested."
            />
            <Figure
              label="reached through stale evidence"
              value={count(report.totals.stale)}
              note="Recorded against a file that has since changed."
              unknown={report.totals.stale > 0}
            />
            <Figure
              label="files re-hashed"
              value={count(report.files_probed)}
              note="Freshness is measured by reading the repository again, not from a cache."
            />
          </div>

          <div className="row row--wrap">
            <span className="micro">relations walked</span>
            {report.relations.map((relation) => (
              <Chip key={relation} tone="quiet">
                {relation}
              </Chip>
            ))}
          </div>

          {exclusionReading(report.relations) === '' ? null : (
            <p className="prose">{exclusionReading(report.relations)}</p>
          )}
          {staleReading(report.totals.stale) === '' ? null : (
            <p className="prose">{staleReading(report.totals.stale)}</p>
          )}

          {report.totals.entities === 0 ? null : (
            <div className="stack" style={{ gap: 12 }}>
              <div>
                <div className="micro" style={{ marginBottom: 6 }}>
                  by how far away
                </div>
                <Tally rows={depthRows(report.totals)} />
              </div>
              <div>
                <div className="micro" style={{ marginBottom: 6 }}>
                  by the relation that reached it
                </div>
                <Tally rows={tallyRows(report.totals.by_relation)} />
              </div>
              <div>
                <div className="micro" style={{ marginBottom: 6 }}>
                  by kind
                </div>
                <Tally rows={tallyRows(report.totals.by_kind)} />
              </div>
            </div>
          )}
        </div>
      </Panel>

      <Panel
        title="What depends on this"
        aside={
          report.truncated ? (
            <Chip tone="stale">{count(report.count)} of {count(report.results_total)}</Chip>
          ) : (
            <span className="hash">{count(report.count)} listed</span>
          )
        }
        flush={report.results.length > 0}
      >
        {report.results.length === 0 ? (
          <Empty
            title="Nothing reaches it"
            body={emptyReading(report.relations)}
          >
            <p className="state__body">
              That is a statement about the relations above and about the edges Nerve managed to
              resolve. Read it beside the unresolved account — an empty closure with reference sites
              outstanding is exactly where the caveat carries the whole answer.
            </p>
          </Empty>
        ) : (
          <>
            <ul className="hits">
              {report.results.map((row) => (
                <Row
                  key={`${row.assertion_id}:${row.entity.entity_id}`}
                  row={row}
                  subject={report.subject}
                  onThisPage={onThisPage}
                />
              ))}
            </ul>
            {pageReading(report) === '' ? null : (
              <div className="panel__body">
                <p className="prose">{pageReading(report)}</p>
              </div>
            )}
          </>
        )}
      </Panel>
    </>
  );
}

/**
 * The account of what the answer cannot see. **Rendered on every answer, whatever the counts.**
 *
 * The zero branch is not an empty state and is not hidden. "No failed resolution is hiding a
 * dependency from this answer" is the only case on this screen where a reassurance is earned, and
 * it has to be said rather than inferred from an absent panel — an absent panel is indistinguishable
 * from a panel nobody wrote.
 *
 * There is deliberately **no list of the unresolved sites here**, and no matching of their names
 * against the subject's. That would be identity by coincidence; Nerve does not do it, and the API
 * hands over no data with which to do it.
 */
function Unresolved({ report }: { report: ImpactReport }) {
  const { unresolved } = report;
  const categories = tallyRows(unresolved.by_category);

  return (
    <Panel
      title="What this answer cannot see"
      aside={
        <Chip tone={unresolvedTone(unresolved)}>
          {count(unresolved.sites)} unresolved {unresolved.sites === 1 ? 'site' : 'sites'}
        </Chip>
      }
    >
      <div className="stack" style={{ gap: 12 }}>
        <p className="prose">{unresolvedReading(unresolved)}</p>

        {unresolved.sites === 0 ? null : (
          <>
            <p className="prose">
              Nerve does not infer types, so a call on a value whose type is not written down is
              recorded as unresolved rather than guessed at. The scope is this whole repository,
              restricted to the relations walked above — a hidden edge could attach anywhere, and
              narrowing that without matching names is not possible.
            </p>
            {grainReading(unresolved) === '' ? null : (
              <p className="hash wrapany">{grainReading(unresolved)}</p>
            )}
            {categories.length === 0 ? null : (
              <div>
                <div className="micro" style={{ marginBottom: 6 }}>
                  by category
                </div>
                <div className="stack" style={{ gap: 6 }}>
                  <Tally rows={categories} />
                  {categories.map(([category]) => (
                    <p key={category} className="hash wrapany">
                      <strong>{category}</strong> — {unresolvedCategory(category)}
                    </p>
                  ))}
                </div>
              </div>
            )}
            <p className="hash wrapany">
              These are not listed as suspect callers. Nothing here has been matched against the
              symbol&apos;s name, because a name that looks alike is not evidence of identity.
            </p>
          </>
        )}
      </div>
    </Panel>
  );
}

/**
 * One dependant, and the edge that reached it.
 *
 * The sentence reads from the dependant outwards — `describe calls add` — because `direction` on
 * an impact row is invariantly outgoing *read against the row's own entity*: a reverse closure can
 * only admit something through an edge that thing itself asserts. Rendering it any other way would
 * turn a dependency into its own inverse.
 */
function Row({
  row,
  subject,
  onThisPage,
}: {
  row: ImpactRow;
  subject: Entity;
  onThisPage: Map<string, Entity>;
}) {
  const reached =
    row.reached_entity_id === subject.entity_id ? subject : onThisPage.get(row.reached_entity_id);
  const measured = row.evidence_freshness === null ? null : freshness(row.evidence_freshness);

  return (
    <li>
      <div className={row.is_unresolved ? 'hit hit--unresolved' : 'hit'}>
        <span className="hit__kind">{row.entity.kind}</span>
        <span className="hit__name truncate">
          <EntityLink entity={row.entity} />
        </span>
        <span className="hit__where">
          <Where path={row.file_path} line={row.start_line} />
        </span>
      </div>
      <div className="row row--wrap" style={{ padding: '0 12px 10px' }}>
        <Chip tone="plain" title="How many edges away from the symbol this is.">
          depth {row.depth}
        </Chip>
        <span className="hash wrapany">
          {row.entity.name} {relationPhrase(row.relation, true)}{' '}
          {reached === undefined ? (
            <span title={row.reached_entity_id}>
              a closure member not on this page ({shortHash(row.reached_entity_id, 10)})
            </span>
          ) : (
            <EntityLink entity={reached} max={40} />
          )}
        </span>
        <Chip tone={rowTone(row)}>
          {measured === null ? 'freshness not measured' : measured.label}
        </Chip>
        {row.is_unresolved ? (
          <Chip tone="unknown" prose>
            this edge points at something Nerve could not name
          </Chip>
        ) : null}
        <span className="hash">
          {row.status} · {row.strongest_source_type} · {count(row.observation_count)}{' '}
          {row.observation_count === 1 ? 'observation' : 'observations'}
        </span>
        <a className="link" href={entityHref(row.entity.entity_id, 'evidence')}>
          the evidence
        </a>
      </div>
    </li>
  );
}
