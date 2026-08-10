/**
 * Contracts — which other repositories this one has been told about, and what it says about them.
 *
 * Three things make this screen different from every other one, and each of them is a rule rather
 * than a layout choice.
 *
 * **Nothing here is discovered.** A neighbour is in the registry because somebody ran a command
 * naming it. So an empty screen is an *absence* rather than a finding, and it says which command
 * changes that instead of listing nothing and letting a reader conclude the project has no
 * dependencies. A sibling checkout sitting next door is a filesystem accident, and Nerve refuses it
 * as evidence.
 *
 * **Every link carries the far side as a snapshot.** The repository at the other end can move,
 * change or vanish, and this index cannot vouch for it. So a link is never drawn as simply "true":
 * it is drawn with the verdict the server computed — current, or one of twelve named situations —
 * and the two pairs that look alike are kept visually apart, because a path that no longer exists
 * and a path that now holds a *different* repository have different remedies.
 *
 * **Registration and scanning are mutations, and `nerve serve` is read-only.** There is therefore
 * no button here and no disabled control implying one is coming. The screen shows the current data,
 * explains the boundary, and prints the exact command — which is the same decision the History
 * screen makes about `nerve history sync`, for the same reason.
 *
 * Every string that came out of a repository — a display name, a contract identity, a version, a
 * path, a target snapshot — is interpolated as a React child, which React escapes. Some of them
 * came out of a repository this one merely *names*, which is the widest untrusted surface in the
 * product.
 */

import { useMemo } from 'react';

import type {
  ContractLink,
  ContractLinkList,
  ContractRegistry,
  ContractTerm,
  ContractVocabulary,
  RegistryEntry,
} from '../api/types';
import { count, stamp, type Tone } from '../format';
import { useApi } from '../hooks';
import { href, type ContractTab } from '../routing';
import { Chip, Def, Defs, Empty, Failure, Loading, Panel, Where } from '../ui/parts';
import {
  contractAmbiguityGloss,
  contractFreshnessGloss,
  contractKindGloss,
  contractLinkStatusGloss,
  contractResolutionMethodGloss,
  registryEntryStatusGloss,
} from '../vocab';

/** How many links one page asks for. The server clamps and reports what it applied. */
const PAGE = 100;

export const CONTRACT_TABS: readonly ContractTab[] = ['links', 'registry', 'vocabulary'];

const TAB_LABEL: Record<ContractTab, string> = {
  links: 'Declared links',
  registry: 'Registered neighbours',
  vocabulary: 'What is read, and what is declined',
};

/**
 * Hue for a link's standing.
 *
 * There is deliberately **no `default` arm returning a calm colour**. An unrecognised verdict is
 * drawn as unknown, because the values a happy-path draft forgets are exactly the ones describing a
 * broken link — and falling back to "fine" would be silent in precisely the cases where silence
 * reads as "this link is current".
 */
export function freshnessTone(value: string | null): Tone {
  if (value === null) return 'fresh';
  switch (value) {
    case 'source_changed':
    case 'target_changed':
    case 'both_changed':
    case 'contract_version_mismatch':
      return 'stale';
    case 'target_repository_missing':
    case 'contract_file_missing':
    case 'contract_deleted':
    case 'registry_entry_removed':
      return 'absent';
    case 'target_repository_moved':
    case 'duplicate_contract_identity':
    case 'conflicting_definitions':
    case 'target_partially_indexed':
      return 'unknown';
  }
  return 'unknown';
}

/** Hue for what re-checking a registered path found. */
export function availabilityTone(value: string): Tone {
  switch (value) {
    case 'available':
      return 'fresh';
    case 'partially_indexed':
      return 'unknown';
    case 'entry_removed':
      return 'quiet';
    case 'missing':
      return 'absent';
    case 'moved':
      return 'stale';
    case 'refused':
      return 'unknown';
  }
  return 'unknown';
}

/**
 * The boundary, printed as the commands it actually is.
 *
 * Not a disabled button. A control that cannot work implies an implementation is pending, and none
 * is: this API is read-only and is proven so on the database bytes, so changing the registry is a
 * command and the useful thing to show is the command.
 */
function Boundary({ statement, commands }: { statement: string; commands: string[] }) {
  return (
    <section className="panel">
      <header className="panel__head">
        <h2 className="micro">Changing any of this</h2>
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
          Run one and reload this page. Nothing is discovered on its own: a directory sitting beside
          this one is a filesystem accident, and Nerve refuses it as evidence for a link.
        </p>
      </div>
    </section>
  );
}

/** The qualifications every contract answer carries, rendered rather than dropped. */
function Limitations({ block }: { block: ContractLinkList | ContractRegistry | ContractVocabulary }) {
  return (
    <Panel title="What this answer does not claim">
      <div style={{ display: 'grid', gap: 8 }}>
        <p className="prose">{block.limitations.link_is_directional_and_one_sided}</p>
        <p className="prose">{block.limitations.contract_version_verdict_is_not_derived}</p>
        <p className="prose">{block.limitations.no_link_is_reachable_from_a_local_graph_query}</p>
      </div>
    </Panel>
  );
}

/** One registered neighbour and what re-checking its path found. */
function EntryCard({ entry }: { entry: RegistryEntry }) {
  return (
    <div className="claim">
      <div className="claim__head">
        <div className="claim__sentence">
          <span className="claim__party claim__party--subject">{entry.display_name}</span>
          <Chip tone="quiet" title="The local id this entry is named by on the command line.">
            {entry.registry_id}
          </Chip>
        </div>
        <div className="claim__sentence">
          <Chip tone={availabilityTone(entry.availability)}>{entry.availability}</Chip>
          <Chip tone={entry.status === 'active' ? 'quiet' : 'absent'}>{entry.status}</Chip>
          {entry.freshness === null ? null : (
            <Chip tone={freshnessTone(entry.freshness)}>{entry.freshness}</Chip>
          )}
        </div>
      </div>
      <div className="claim__body" style={{ display: 'grid', gap: 8 }}>
        <p className="prose">{entry.availability_statement}</p>
        <p className="prose">{registryEntryStatusGloss(entry.status)}</p>
        {entry.refusal === null ? null : (
          <p className="prose">
            <strong>{entry.refusal}</strong> — {entry.refusal_statement}
          </p>
        )}
        {entry.observed_repository_id === null ? null : (
          <p className="prose">
            The repository found there is <span className="hash">{entry.observed_repository_id}</span>
            , which is not the one this entry records. Nothing resolved through it may be read until
            it is re-pointed or retired.
          </p>
        )}
        {entry.freshness === null ? null : (
          <p className="prose">{contractFreshnessGloss(entry.freshness)}</p>
        )}
        <Defs>
          <Def term="path">
            <span className="hash wrapany">{entry.local_path}</span>
          </Def>
          <Def term="repository id">
            <span className="hash wrapany">{entry.expected_repository_id}</span>
          </Def>
          <Def term="registered">{stamp(entry.added_at)}</Def>
          <Def term="last checked">{stamp(entry.availability_checked_at)}</Def>
          <Def term="last seen state">
            <span className="hash wrapany">{entry.last_seen_state ?? 'never recorded'}</span>
          </Def>
          {entry.withdrawn_at === null ? null : (
            <Def term="retired">{stamp(entry.withdrawn_at)}</Def>
          )}
          {entry.links_through_this_entry === undefined ? null : (
            <Def term="links through it">{count(entry.links_through_this_entry)}</Def>
          )}
        </Defs>
      </div>
    </div>
  );
}

/**
 * One declared link, with the far side shown as the snapshot it is.
 *
 * The target block is labelled "as recorded" rather than left to look like a live reading, because
 * every value in it was true of the neighbour at the state named beside it and may not be true now.
 */
function LinkCard({ link }: { link: ContractLink }) {
  const target = link.target_path_snapshot ?? link.target_name_snapshot;
  return (
    <div className={link.is_current ? 'claim' : 'claim claim--unresolved'}>
      <div className="claim__head">
        {/*
          The three parts of the claim, in the same sentence shape the evidence view uses: the two
          names either side are in the repositories' own voice and the verb between them is in
          Nerve's, so it stays visible which two parts Nerve did not write.
        */}
        <div className="claim__sentence">
          <span className="claim__party claim__party--subject">{link.contract_identity}</span>
          <span className="claim__verb">{link.relation_semantics}</span>
          <span className="claim__party">{link.registry_entry.display_name}</span>
        </div>
        <div className="claim__sentence">
          <Chip tone={freshnessTone(link.freshness)}>{link.freshness ?? 'current'}</Chip>
          <Chip tone="quiet" title={contractKindGloss(link.contract_kind)}>
            {link.contract_kind}
          </Chip>
          <Chip tone="quiet" title={link.resolution_method_note}>
            {link.resolution_method}
          </Chip>
          {link.status === 'active' ? null : <Chip tone="absent">{link.status}</Chip>}
          {link.ambiguity === null ? null : <Chip tone="unknown">{link.ambiguity}</Chip>}
        </div>
      </div>
      <div className="claim__body" style={{ display: 'grid', gap: 8 }}>
        <p className="prose">
          {link.freshness === null
            ? 'The registry entry is available, both repositories are still at the states this link was worked out at, and the file it was quoted from is still there.'
            : link.freshness_note}
        </p>
        <p className="prose">{contractKindGloss(link.contract_kind)}</p>
        <p className="prose">{contractResolutionMethodGloss(link.resolution_method)}</p>
        {link.ambiguity === null ? null : (
          <p className="prose">{contractAmbiguityGloss(link.ambiguity)}</p>
        )}
        {link.unsupported_reason === null ? null : (
          <p className="prose">
            One part of this declaration was recognised and declined, by name:{' '}
            <strong>{link.unsupported_reason}</strong>. A declined form is recorded rather than
            dropped.
          </p>
        )}

        <Defs>
          <Def term="declared in">
            <Where path={link.source_path} />
            <span className="hash"> · line {link.source_span}</span>
            {link.source_manifest_present ? null : (
              <span className="hash"> · that file is no longer in this repository</span>
            )}
          </Def>
          {/*
            The resolution method's own note, from the vocabulary that declares it, shown as a
            field rather than as a second paragraph. The gloss above is the interface's voice; this
            is the one the API carries, and both are on screen so neither has to be trusted to be
            a faithful copy of the other.
          */}
          <Def term="what was read">{link.resolution_method_note}</Def>
          <Def term="version asked for">
            {link.expected_contract_version ?? 'the declaration states none'}
          </Def>
          <Def term="version declared there">
            {link.observed_contract_version ?? 'the other manifest states none'}
          </Def>
          <Def term="through">
            {link.registry_entry.registry_id} ·{' '}
            <span className="hash wrapany">{link.registry_entry.local_path}</span>
          </Def>
          <Def term="target, as recorded">
            {target === null ? (
              'this rule links two repositories and names no file at either end'
            ) : (
              <>
                <span className="hash wrapany">{target}</span>
                {link.target_kind_snapshot === null ? null : (
                  <span className="hash"> · {link.target_kind_snapshot}</span>
                )}
                {link.target_span_snapshot === null ? null : (
                  <span className="hash"> · {link.target_span_snapshot}</span>
                )}
                {link.target_entity_id === null ? (
                  <span className="hash">
                    {' '}
                    · the other repository has this file and has never indexed it
                  </span>
                ) : null}
              </>
            )}
          </Def>
          <Def term="their state then">
            <span className="hash wrapany">
              {link.target_state_at_resolution ?? 'not recorded'}
            </span>
          </Def>
          <Def term="their state now">
            <span className="hash wrapany">{link.target_current_state ?? 'could not be read'}</span>
          </Def>
          <Def term="our state then">
            <span className="hash wrapany">{link.source_state_at_resolution}</span>
          </Def>
          <Def term="link status">
            {link.status} — {contractLinkStatusGloss(link.status)}
          </Def>
          <Def term="first seen">{stamp(link.first_seen_at)}</Def>
          <Def term="last seen">{stamp(link.last_seen_at)}</Def>
          {link.withdrawn_at === null ? null : (
            <Def term="withdrawn">{stamp(link.withdrawn_at)}</Def>
          )}
          <Def term="read by">
            {link.extractor_id} v{link.extractor_version}
          </Def>
        </Defs>
      </div>
    </div>
  );
}

/** One closed vocabulary, listed by name. */
function TermList({ title, note, terms }: { title: string; note: string; terms: ContractTerm[] }) {
  return (
    <Panel title={title} aside={<span className="hash">{count(terms.length)}</span>}>
      <div style={{ display: 'grid', gap: 10 }}>
        <p className="prose">{note}</p>
        <div className="row row--wrap">
          {terms.map((term) => (
            <Chip key={term.name} tone="quiet" title={term.note ?? term.rule ?? undefined}>
              {term.name}
            </Chip>
          ))}
        </div>
      </div>
    </Panel>
  );
}

export function Contracts({ tab, options }: { tab: ContractTab; options: Record<string, string> }) {
  return (
    <div className="view">
      <div className="head">
        <h1 className="head__title">Contracts</h1>
        <p className="head__sub">
          What this repository declares about other repositories it has been told about — and, on
          every link, whether that declaration still describes the world. A link is only ever quoted
          from an explicit declaration in a file. A similar name, a matching string and a directory
          next door are each refused as evidence.
        </p>
      </div>

      <nav className="tabs" aria-label="Contract questions">
        {CONTRACT_TABS.map((name) => (
          <a
            key={name}
            className="tab"
            href={href({ view: 'contracts', tab: name, options: {} })}
            aria-current={name === tab ? 'page' : undefined}
          >
            {TAB_LABEL[name]}
          </a>
        ))}
      </nav>

      <div className="stack">
        {tab === 'registry' ? (
          <RegistryView />
        ) : tab === 'vocabulary' ? (
          <VocabularyView />
        ) : (
          <LinkView options={options} />
        )}
      </div>
    </div>
  );
}

function LinkView({ options }: { options: Record<string, string> }) {
  const offset = Number(options['offset'] ?? '0') || 0;
  const registry = options['registry_id'] ?? '';
  const params = useMemo(
    () => (registry === '' ? { limit: PAGE, offset } : { limit: PAGE, offset, registry_id: registry }),
    [offset, registry],
  );
  const { state, reload } = useApi<ContractLinkList>('/api/contracts', params);

  if (state.status === 'loading') return <Loading label="Reading the declared links" />;
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;

  const report = state.data;
  const current = report.links.filter((link) => link.is_current).length;
  return (
    <>
      <Panel
        title="Declared links"
        aside={
          <span className="hash">
            {count(report.links.length)} shown of {count(report.links_total)}
          </span>
        }
      >
        <div style={{ display: 'grid', gap: 8 }}>
          <p className="prose">
            {report.links_total === 0
              ? 'This repository declares no link to any registered neighbour. That is an absence rather than a finding: either no neighbour has been registered, or no manifest here names one, or the manifests have not been read since one was added.'
              : `${count(current)} of ${count(report.links_total)} are current. The rest each carry a named situation saying what changed, and none of them is drawn as though it still described the world.`}
          </p>
          {registry === '' ? null : (
            <p className="prose">
              Narrowed to the entry <strong>{registry}</strong> —{' '}
              {count(report.links_matching_filter)} of {count(report.links_total)} match.{' '}
              <a className="link" href={href({ view: 'contracts', tab: 'links', options: {} })}>
                show all
              </a>
            </p>
          )}
          {report.links_without_registry_entry === 0 ? null : (
            <p className="prose">
              {count(report.links_without_registry_entry)} link(s) name a registry entry that could
              not be found, and are not listed above.
            </p>
          )}
        </div>
      </Panel>

      {report.links.length === 0 ? null : (
        <section className="panel">
          <div className="panel__body panel__body--flush">
            <div className="spine">
              {report.links.map((link) => (
                <LinkCard
                  key={`${link.link_id ?? 0}-${link.contract_identity}-${link.source_span}`}
                  link={link}
                />
              ))}
            </div>
          </div>
          {report.truncation === null || !report.truncation.truncated ? null : (
            <div className="graph__foot">
              <a
                className={offset === 0 ? 'btn btn--ghost' : 'btn'}
                aria-disabled={offset === 0 ? 'true' : undefined}
                href={href({
                  view: 'contracts',
                  tab: 'links',
                  options: { offset: String(Math.max(0, offset - PAGE)) },
                })}
              >
                previous
              </a>
              <span>
                {count(offset + 1)}–{count(offset + report.links.length)} of{' '}
                {count(report.truncation.total)}
              </span>
              <a
                className={report.continuation.next_offset === null ? 'btn btn--ghost' : 'btn'}
                aria-disabled={report.continuation.next_offset === null ? 'true' : undefined}
                href={href({
                  view: 'contracts',
                  tab: 'links',
                  options: { offset: String(report.continuation.next_offset ?? offset) },
                })}
              >
                next
              </a>
            </div>
          )}
        </section>
      )}

      <Limitations block={report} />
      <Boundary statement={report.boundary.statement} commands={report.boundary.commands} />
    </>
  );
}

function RegistryView() {
  const { state, reload } = useApi<ContractRegistry>('/api/contracts/registry', { limit: PAGE });

  if (state.status === 'loading') return <Loading label="Reading the registry" />;
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;

  const report = state.data;
  return (
    <>
      <Panel
        title="Registered neighbours"
        aside={<span className="hash">{count(report.registry_entries_total)}</span>}
      >
        {report.entries.length === 0 ? (
          <Empty
            title="No repository has been registered here"
            body="That is an absence rather than a finding. Nothing is discovered on its own — a checkout sitting beside this one is a filesystem accident, and a package name with no path is not a repository — so a neighbour exists here because a command named it."
          />
        ) : (
          <div style={{ display: 'grid', gap: 8 }}>
            <p className="prose">
              Each entry is re-checked against the filesystem every time this page is read, and the
              check is by the repository id recorded when it was registered rather than by its path.
              A path that now holds a different repository is the dangerous case, and it is the one
              a path comparison would miss.
            </p>
            <p className="prose">
              A retired entry stays listed. It is kept rather than deleted so that a link which
              rested on it can still say which neighbour went away, and when.
            </p>
          </div>
        )}
      </Panel>

      {report.entries.length === 0 ? null : (
        <section className="panel">
          <div className="panel__body panel__body--flush">
            <div className="spine">
              {report.entries.map((entry) => (
                <EntryCard key={entry.registry_id} entry={entry} />
              ))}
            </div>
          </div>
          <div className="panel__body">
            <p className="prose">
              <a
                className="link"
                href={href({ view: 'contracts', tab: 'links', options: {} })}
              >
                See what is declared through these entries
              </a>
            </p>
          </div>
        </section>
      )}

      <Limitations block={report} />
      <Boundary statement={report.boundary.statement} commands={report.boundary.commands} />
    </>
  );
}

function VocabularyView() {
  const { state, reload } = useApi<ContractVocabulary>('/api/contracts/vocabulary');

  if (state.status === 'loading') return <Loading label="Reading the vocabulary" />;
  if (state.status === 'error') return <Failure error={state.error} onRetry={reload} />;

  const { vocabulary } = state.data;
  return (
    <>
      <Panel title="What a link may be drawn from">
        <p className="prose">
          Every one of these names a file that says so. There is no entry for a similar name, a
          matching endpoint string, a nearby directory or a resemblance of any kind, because none of
          those is a declaration — and this list is the whole of what Nerve will read.
        </p>
      </Panel>
      <TermList
        title="Rules"
        note="Which reader produced a link, and which file it read."
        terms={vocabulary.rules}
      />
      <TermList
        title="Resolution methods"
        note="Which stated declaration a link was quoted from."
        terms={vocabulary.resolution_methods}
      />
      <TermList
        title="Declaration forms read"
        note="The syntax Nerve resolves. Anything outside this set is declined by name rather than dropped."
        terms={vocabulary.supported_forms}
      />
      <TermList
        title="Declaration forms declined, by name"
        note="Each of these was read, recognised and refused. None of them is fetched: a git or https specifier names a network resolution, and Nerve records that it saw one rather than performing it. Counts per run come from `nerve repo scan`, which is a command because it writes."
        terms={vocabulary.unsupported_forms}
      />
      <TermList
        title="Why a supported declaration reached nothing"
        note="The syntax was read in full and the target could not be reached. Not the same as a form Nerve declines: only one of the two has a remedy you can act on."
        terms={vocabulary.unresolved_reasons}
      />
      <TermList
        title="Situations a link can be in"
        note="Twelve, because a link has two repositories behind it and this index can only vouch for one. Two pairs must not be read as one."
        terms={vocabulary.freshness}
      />
      <TermList
        title="What a re-check of a registered path can find"
        note="Decided in one place and rendered here; no surface works this out for itself."
        terms={vocabulary.availability}
      />
      <TermList
        title="Why a registry command refuses"
        note="A refusal is a finding with a name, never a silent fallback."
        terms={vocabulary.registry_refusals}
      />
      <TermList
        title="Why a manifest was stopped"
        note="A bound reached is a stop with a name. A manifest read halfway would report the declarations before the cut and silently omit the rest."
        terms={vocabulary.manifest_refusals}
      />
      <TermList
        title="Other closed sets"
        note="Ambiguity, link status and registry entry status, each rendered on the cards above."
        terms={[
          ...vocabulary.ambiguity,
          ...vocabulary.link_statuses,
          ...vocabulary.registry_entry_statuses,
          ...vocabulary.scan_refusals,
        ]}
      />
      <Limitations block={state.data} />
    </>
  );
}
