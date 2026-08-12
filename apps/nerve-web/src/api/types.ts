/**
 * The wire shapes, transcribed from `crates/nerve-server/src/shapes.rs`.
 *
 * Every string field here originates in the repository being indexed and is therefore hostile:
 * a file can legally be named `<img src=x onerror=alert(1)>.ts`. These types exist to make that
 * explicit at the boundary — nothing in this app ever treats one of these values as markup.
 */

/** A JSON value of unknown shape, such as an extractor's `details` blob. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface Entity {
  entity_id: string;
  kind: string;
  name: string;
  scope_path: string;
  qualified_name: string;
  language: string | null;
  file_path: string | null;
  start_line: number | null;
  end_line: number | null;
}

export interface Occurrence {
  occurrence_id: string;
  file_path: string;
  start_byte: number;
  end_byte: number;
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
  content_hash: string;
}

export interface SearchHit {
  entity_id: string;
  kind: string;
  name: string;
  scope_path: string;
  language: string | null;
  file_path: string | null;
  start_line: number | null;
  end_line: number | null;
  score: number;
}

export interface SearchResponse {
  query: string;
  kind: string | null;
  limit: number;
  count: number;
  results: SearchHit[];
}

export interface RunSummary {
  run_id: number;
  state_id: string;
  extractor_id: string;
  extractor_version: string;
  started_at: string;
  finished_at: string | null;
  files_processed: number;
  files_failed: number;
  status: string;
}

export interface FreshnessReport {
  files_total: number;
  files_probed: number;
  fresh: number;
  stale: number;
  missing: number;
  refused: number;
  unreadable: number;
  truncated: boolean;
}

export interface Overview {
  schema_version: number | null;
  supported_schema_version: number;
  healthy: boolean;
  project_id: string | null;
  root_path: string | null;
  state_id: string | null;
  git_commit: string | null;
  database_bytes: number | null;
  /** Every entity kind, symbols included. Not the number to print beside the word "symbols". */
  entities_total: number;
  /** Functions, methods, classes and interfaces. Always at most `entities_total`. */
  symbols_total: number;
  entities_by_kind: Record<string, number>;
  assertions_total: number;
  assertions_by_relation: Record<string, number>;
  occurrences_total: number;
  observations_total: number;
  assertion_states_total: number;
  unresolved_entities: number;
  unresolved_assertions: number;
  last_run: RunSummary | null;
  runs: RunSummary[];
  freshness: FreshnessReport | null;
}

export interface NeighbourEdge {
  assertion_id: string;
  relation: string;
  source_entity_id: string;
  target_entity_id: string;
  is_unresolved: boolean;
  status: string;
  strongest_source_type: string;
  observation_count: number;
  file_path: string | null;
  start_line: number | null;
}

export interface NeighbourNode {
  depth: number;
  entity: Entity;
}

export interface Neighbourhood {
  focus: Entity;
  max_depth: number;
  max_nodes: number;
  truncated: boolean;
  omitted_nodes: number;
  frontier_nodes: number;
  node_count: number;
  edge_count: number;
  nodes: NeighbourNode[];
  edges: NeighbourEdge[];
  direction?: string;
  relations?: string[];
  resolved_only?: boolean;
}

/** One traversed step. `traversed_backwards` means the edge was followed against its recorded direction. */
export interface PathHop {
  relation: string;
  assertion_id: string;
  from: Entity;
  to: Entity;
  traversed_backwards: boolean;
  is_unresolved: boolean;
  status: string;
  strongest_source_type: string;
  observation_count: number;
  file_path: string | null;
  start_line: number | null;
}

export interface FoundPath {
  length: number;
  traverses_unresolved: boolean;
  hops: PathHop[];
}

export interface PathReport {
  from: Entity;
  to: Entity;
  max_depth: number;
  /** True when the search hit its ceiling before it had finished looking. */
  truncated: boolean;
  /** How many nodes the search expanded before it stopped. */
  expansions: number;
  count: number;
  paths: FoundPath[];
  limit?: number;
  direction?: string;
  relations?: string[];
  resolved_only?: boolean;
}

export interface EntityDetail {
  entity: Entity;
  occurrence_count: number;
  occurrences: Occurrence[];
  relation_counts: {
    outgoing: Record<string, number>;
    incoming: Record<string, number>;
  };
  defining_edges: Neighbourhood;
}

/** One observation and its whole evidence profile. There is no `confidence: float` by design. */
export interface Observation {
  observation_id: number;
  evidence_source_type: string;
  directness: string;
  extractor_id: string;
  extractor_version: string;
  match_quality: string | null;
  state_id: string;
  file_path: string | null;
  start_line: number | null;
  end_line: number | null;
  content_hash: string | null;
  environment: string | null;
  details: Json;
  created_at: string;
  /** Recomputed per request by re-hashing the file: `fresh`, `stale`, `file-missing`, … */
  freshness: string;
}

export interface Assertion {
  assertion_id: string;
  relation: string;
  /** `outgoing` or `incoming`, relative to the subject that was asked about. */
  direction: string;
  source: Entity;
  target: Entity;
  status: string;
  is_unresolved: boolean;
  observation_count: number;
  strongest_source_type: string;
  observations: Observation[];
}

export interface WhyReport {
  subject: Entity;
  object: Entity | null;
  files_probed: number;
  count: number;
  assertions: Assertion[];
  direction?: string;
  relations?: string[];
}

export interface UnresolvedRow {
  entity_id: string;
  name: string;
  scope_path: string;
  meta: Json;
  referencing_assertions: number;
}

export interface UnresolvedReport {
  limit: number;
  offset: number;
  unresolved_entities_total: number;
  unresolved_assertions_total: number;
  count: number;
  results: UnresolvedRow[];
}

export interface PartialParseRow {
  rel_path: string;
  language: string;
  content_hash: string;
  dynamic_imports_without_specifier: number;
  unmodelled_call_sites: number;
  unmodelled_by_form: Record<string, number>;
}

export interface PartialParseReport {
  count: number;
  results: PartialParseRow[];
}

/** One coverage run a gap answer is relative to. */
export interface CoverageRunRef {
  entity_id: string;
  /** Repository-relative path of the report that was ingested. */
  report_path: string | null;
  report_content_hash: string | null;
  /** Whether the report file still hashes to what was ingested. `null` when it cannot be checked. */
  freshness: string | null;
  /** How many source files the report described, as recorded at ingestion. */
  source_files_in_report: number | null;
}

/** The exact tally over every symbol in scope. Present only when coverage evidence exists. */
export interface GapTotals {
  covered: number;
  partial: number;
  uncovered: number;
  unmeasured: number;
  /** `uncovered + unmeasured`. `partial` is never counted here. */
  gaps: number;
  stale: number;
  measured_files: number;
  stale_files: number;
}

/** One symbol and what coverage says about it. `state` is a `SymbolCoverage` member. */
export interface GapRow {
  entity: Entity;
  state: string;
  /** `null` for `unmeasured`: there is no evidence to be fresh or stale about. */
  coverage_freshness: string | null;
  covered_lines: number | null;
  instrumented_lines: number | null;
  covered_by: string[];
}

/**
 * The whole gap answer, including the state that says it cannot be answered.
 *
 * `totals` is `null` — not a row of zeroes — when no coverage was ever ingested, and this app must
 * never coerce it: reading `uncovered: 0` off a tally that was never computed states the opposite
 * of the truth. `coverage: "absent"` means the question is unanswerable here, and `results` is
 * empty for the same reason rather than because nothing is missing.
 */
export interface GapReport {
  /** `absent` or `present`. */
  coverage: string;
  answerable: boolean;
  runs: CoverageRunRef[];
  symbols_in_scope: number;
  totals: GapTotals | null;
  /** The row cap the server applied. The tallies are exact regardless. */
  limit: number;
  count: number;
  results_total: number;
  truncated: boolean;
  files_probed: number;
  results: GapRow[];
  under: string | null;
  kind: string | null;
  include_partial: boolean;
}

// ---- history ---------------------------------------------------------------------------------
//
// Transcribed from `crates/nerve-server/src/api/history.rs`. Three shapes here are load-bearing
// and are annotated where they are declared rather than in a comment nobody reads twice:
//
//   * every optional field of `HistoryBlock` is `null` when history has never been read, and that
//     is not the same fact as zero;
//   * `HistoryCommit.changes` is `null` where the answer counted rows for a path rather than for
//     the commit, which is not an empty commit;
//   * every diff-shaped field of `HistoryDiffReport` is `null` on the four outcomes that computed
//     no range, which is not an empty range.

/** A bounded list's truncation, as the server measured it — never `len() === limit`. */
export interface HistoryTruncation {
  returned: number;
  /** `null` where no total exists to compare against, as for one path's own history. */
  total: number | null;
  truncated: boolean;
  limit: number;
}

/** An offset the query honours, or the statement that there is none to offer. */
export interface HistoryContinuation {
  supported: boolean;
  offset: number | null;
  next_offset: number | null;
  /** Present exactly when `supported` is false, saying why. */
  statement: string | null;
}

/** Whole-repository tallies over visible history. `null` where nothing was read. */
export interface HistoryTotals {
  commits: number;
  changes: number;
  renames: number;
  merges: number;
  /** Keyed by `ChangeKind`, with a zero for every kind that has none. */
  changes_by_kind: Record<string, number>;
}

export interface HistoryLimitations {
  /** From `nerve_store::earlier_changes_may_exist`. Never re-derived in this app. */
  earlier_changes_may_exist: boolean | null;
  merges_in_repository: number | null;
  /** Always true: a merge enumerates no changes, so every change count is a floor. */
  merges_enumerate_no_changes: boolean;
  counts_are_visible_history_only: boolean;
}

/**
 * The availability block, carried by every history answer.
 *
 * It is not decoration. Without it a history answer is a set of dates with no statement of what
 * they are dates *of* — how much was read, where the reading stopped, whether the reading still
 * describes what is indexed now.
 */
export interface HistoryBlock {
  repository_id: string;
  current_repository_state: { state_id: string | null; git_commit: string | null };
  requested_subject: Json;
  /** `false` is "history has never been read here", which is not "this project has no history". */
  history_ingested: boolean;
  shallow: boolean | null;
  shallow_boundary: string[] | null;
  promisor: boolean | null;
  /** A `WalkTermination` member. */
  walk_terminated_by: string | null;
  walk_terminated_note: string | null;
  commits_recorded: number | null;
  commit_budget: number | null;
  refusals: Record<string, number> | null;
  refusals_total: number | null;
  reader_version: string | null;
  totals: HistoryTotals | null;
  /** Which answer this is, including `no_history_ingested`. */
  result_kind: string;
  /** A `HistoryFreshness` member. */
  freshness: string;
  freshness_note: string;
  ingest_head_oid: string | null;
  truncation: HistoryTruncation | null;
  continuation: HistoryContinuation;
  limitations: HistoryLimitations;
}

/** One change to one path. A rename is not here: Git records none, so one is a hypothesis. */
export interface HistoryChange {
  path: string;
  /** A `ChangeKind` member. */
  change_kind: string;
  blob_oid: string | null;
  prev_blob_oid: string | null;
  mode: number | null;
  prev_mode: number | null;
  /** Present on a state diff, where changes span several commits. */
  commit_oid?: string;
}

export interface HistoryCommit {
  commit_oid: string;
  tree_oid: string;
  parent_oids: string[];
  is_merge: boolean;
  /** A `ParentCompleteness` member. */
  parent_completeness: string;
  parent_completeness_note: string;
  /**
   * Carried off `ParentCompleteness::may_claim_history_begins_here`, true for a root commit and
   * nothing else. **Never re-derive this from `parent_completeness`** — that would be a second
   * copy of the one rule the historical model exists to protect.
   */
  may_claim_history_begins_here: boolean;
  /** A `ChangesEnumerated` member: which of four silences a commit with no changes is. */
  changes_enumerated: string;
  changes_enumerated_note: string;
  /** `null` where the rows were counted for a path rather than for the commit. Not `0`. */
  changes: number | null;
  author_time: number;
  author_tz: string;
  committer_time: number;
  committer_tz: string;
  author_ident: string | null;
  committer_ident: string | null;
  /** Repository prose, and the first free-form repository text Nerve stores. Text, never markup. */
  summary: string;
  /**
   * A `SummaryTruncation` member, and **never render `summary` without it**. The repository-level
   * refusal tally says some summary was cut, not which one; the text alone cannot tell a short
   * first line from a cut one, and a first line of exactly the stored bound is `complete`. `unknown`
   * is what every commit recorded before schema v7 carries, so on an upgraded index it is the
   * common value rather than the rare one.
   */
  summary_truncation: string;
  summary_truncation_note: string;
  /** Present on a path's history, where each commit carries what it did to that path. */
  change?: HistoryChange;
  /**
   * Present on a path's history. The commit's own similarity candidate-set record, which is the
   * only thing that can qualify an **absent** rename hypothesis: a commit whose candidate set a
   * bound refused records no row at all, so without this its silence reads as "nothing moved".
   * `null` where this matcher never analysed the commit, and `rename_analysis_absent_note` says so.
   */
  rename_analysis?: HistoryRenameAnalysis | null;
  rename_analysis_absent_note?: string | null;
}

/**
 * One matcher's candidate set for one commit, and how much of it was measured.
 *
 * The **threshold** is why this travels beside every hypothesis. A measurement without the number
 * it was admitted against is a ratio the reader has to compare with a constant they guessed — and
 * the constant belongs to the run, not to this build, so a row measured under an older threshold
 * must still show the one it was actually judged by.
 */
export interface HistoryRenameAnalysis {
  commit_oid: string;
  matcher_id: string;
  matcher_version: string;
  /** Two integers, like the measurement. Admitted when `numerator × denom ≥ threshold_num × den`. */
  threshold_numerator: number;
  threshold_denominator: number;
  deletions_considered: number;
  additions_considered: number;
  pairs_considered: number;
  pairs_measured: number;
  /** A `RenameAnalysisCompleteness` member. Three of its four values mean "not the full set". */
  completeness: string;
  completeness_note: string;
  /** Keyed by `SimilarityUnmeasured`. An unmeasured pair is an unanswered question, not a "no". */
  unmeasured: Record<string, number>;
  unmeasured_notes: Record<string, string>;
}

/** A proposal that one path became another. There is no score and none may be invented. */
export interface HistoryRename {
  commit_oid: string;
  from_path: string;
  to_path: string;
  /** A `RenameEvidence` member. */
  evidence: string;
  evidence_note: string;
  /**
   * The blob each side names. Two, not one, since schema v7: a similarity pair has two blobs, and
   * for an `exact_content` hypothesis they are equal — that equality is the evidence.
   */
  from_blob_oid: string;
  to_blob_oid: string;
  /** Which method produced this row, and its version. A measurement without them means nothing. */
  matcher_id: string;
  matcher_version: string;
  /**
   * The measurement, as an exact rational rather than a float. `null` on both for `exact_content`,
   * which computes no similarity at all — not a perfect score, no score.
   */
  match_numerator: number | null;
  match_denominator: number | null;
  /** A `RenameAmbiguity` member. Not decoration: `many_to` is not `unique`. */
  ambiguity: string;
  ambiguity_note: string;
  /** Always true. Carried per row rather than in a footnote a client can drop. */
  is_hypothesis: boolean;
  /** Always false. The other half of the same statement, so neither has to be inferred. */
  is_confirmed_rename: boolean;
  /**
   * The candidate-set record for this row's commit, joined on by the backend. `null` in two cases
   * that are not the same, which is why `analysis_absent_note` says which — an exact-content
   * hypothesis has no candidate set to be complete about, while a similarity hypothesis without one
   * has a completeness that is unknown rather than complete. Never render the absence as a blank.
   */
  analysis: HistoryRenameAnalysis | null;
  analysis_absent_note: string | null;
}

export interface HistoryPathChange {
  commit: HistoryCommit;
  change: HistoryChange;
}

/** What the current tree said about a path, and whether it was in a position to say anything. */
export interface HistoryCurrentTree {
  basis: string;
  index_exists: boolean;
  entities_at_path: number;
}

/**
 * When a path was first and last *observed* changing, and which of six answers that is.
 *
 * `may_claim_created` is the only permission to use the word "created", it comes from
 * `FirstObservedKind::may_claim_created`, and `may_claim_created_note` is the sentence the
 * backend wrote for the case. Render the note; do not compose one from `kind`.
 */
export interface HistoryFirstObserved {
  path: string;
  /** A `FirstObservedKind` member. */
  kind: string;
  kind_note: string;
  may_claim_created: boolean;
  may_claim_created_note: string;
  first: HistoryPathChange | null;
  last: HistoryPathChange | null;
  changes_in_visible_history: number;
  additions_recorded: number;
  merges_in_repository: number;
  /**
   * An `EarlierHistoryUnavailable` member, about **this path**. `null` means nothing is hidden
   * above it. Not the same question as `earlier_changes_may_exist`, which is about the ingest.
   */
  earlier_history_unavailable: string | null;
  earlier_history_unavailable_note: string | null;
  earlier_changes_may_exist: boolean;
  walk_terminated_by: string | null;
  walk_terminated_note: string | null;
  shallow: boolean;
  current_tree: HistoryCurrentTree;
}

export interface HistoryCommitLog extends HistoryBlock {
  commits: HistoryCommit[];
}

export interface HistoryCommitDetail extends HistoryBlock {
  commit: HistoryCommit;
  changes: HistoryChange[];
}

export interface HistoryPathReport extends HistoryBlock {
  path: string;
  first_observed: HistoryFirstObserved;
  /** Each carries the `change` it made to this path. */
  commits: HistoryCommit[];
  renames: HistoryRename[];
  renames_count: number;
  renames_truncated: boolean;
  /** Which matcher each row's `analysis` was looked up under. Several may analyse one commit. */
  rename_analysis_matcher_id: string;
}

/**
 * What lies between two recorded states, by ancestry and never by a time range.
 *
 * Five outcomes, four of which are refusals. **Every field below that describes a range is `null`
 * on those four**, so an absent list is a refusal and an empty list is an answer.
 */
export interface HistoryDiffReport extends HistoryBlock {
  from: string;
  to: string;
  max_walk: number;
  max_changes: number;
  ancestry_not_a_time_range: boolean;
  from_recorded: boolean | null;
  to_recorded: boolean | null;
  commits_walked: number | null;
  walk_limit: number | null;
  stopped_at: string | null;
  stopped_at_parent_completeness: string | null;
  stopped_at_parent_completeness_note: string | null;
  commits: HistoryCommit[] | null;
  commits_in_range: number | null;
  commits_truncated: boolean | null;
  changes: HistoryChange[] | null;
  changes_truncated: boolean | null;
  merges_in_range: number | null;
  /** Keyed by `ChangesEnumerated`, counting commits in the range in each state. */
  changes_enumerated: Record<string, number> | null;
  ancestry_incomplete_at: {
    commit_oid: string;
    parent_completeness: string;
    parent_completeness_note: string;
  } | null;
  this_is_not_an_empty_diff: boolean | null;
}

export interface HistoryFrequencyRow {
  path: string;
  commits: number;
}

export interface HistoryFrequencyReport extends HistoryBlock {
  paths_total: number;
  rows: HistoryFrequencyRow[];
}

/** A raw shared-commit count. Never a normalised affinity, and never labelled as coupling. */
export interface HistoryCochangeRow {
  path_a: string;
  path_b: string;
  cochange_observations: number;
}

export interface HistoryCochangeReport extends HistoryBlock {
  path: string;
  pairs_total: number;
  /** `nerve_store::COCHANGE_IS_NOT_A_DEPENDENCY`, verbatim. Print it. */
  disclaimer: string;
  rows: HistoryCochangeRow[];
}

export interface SourceSnippet {
  path: string;
  start_line: number;
  end_line: number;
  total_lines: number;
  truncated: boolean;
  content_hash: string;
  max_lines: number;
  max_bytes: number;
  text: string;
}

/**
 * The closed vocabularies, mirrored from `nerve-core::vocab`.
 *
 * These two are data rather than prose: `ENTITY_KINDS` is the search filter and `RELATIONS` the
 * graph filter, so a member missing here is not a missing sentence — it is a question the user
 * cannot ask. Order matches the Rust declaration order because that is the order they render in.
 *
 * `crates/nerve-server/tests/ui_vocabulary.rs` asserts both lists against `EntityKind::ALL` and
 * `Relation::ALL` exactly, so neither can fall behind the backend again without a test failing.
 */
export const ENTITY_KINDS = [
  'repository',
  'directory',
  'file',
  'module',
  'function',
  'method',
  'class',
  'interface',
  'document',
  'section',
  'unresolved',
  'coverage_run',
  // Appended in Slice 10a.
  'endpoint',
] as const;

export const RELATIONS = [
  'CONTAINS',
  'DEFINES',
  'IMPORTS',
  'EXPORTS',
  'CALLS',
  'REFERENCES',
  'EXTENDS',
  'IMPLEMENTS',
  'SUPERSEDES',
  'COVERS',
  // Appended in Slice 10a.
  'SERVED_BY',
  // Appended in Slice 11a.
  'TEST_OBSERVED_CALL',
] as const;

/*
 * ---- Slice 13d: the cross-repository contract surfaces -----------------------------------------
 *
 * Every string below originated in a repository — this one, or a *neighbouring* one whose index
 * Nerve opened read-only. A display name, a contract identity, a version string, a manifest path
 * and every target snapshot field are all repository content on exactly the terms the rest of this
 * file sets, and none of them is ever treated as markup.
 */

/** One member of a closed contract vocabulary, with whatever it owns beside its name. */
export interface ContractTerm {
  name: string;
  /** The sentence this value owns, where it has one. */
  note: string | null;
  /** The rule it belongs to, or — for a rule — the manifest file that rule reads. */
  rule: string | null;
}

/** Where the registry is changed. Every contract answer carries it. */
export interface ContractBoundary {
  read_only: boolean;
  statement: string;
  commands: string[];
}

export interface ContractTruncation {
  returned: number;
  total: number;
  truncated: boolean;
  limit: number;
}

export interface ContractContinuation {
  supported: boolean;
  offset: number | null;
  next_offset: number | null;
  statement: string | null;
}

/** The block every `/api/contracts*` answer carries, assembled in one place on the server. */
export interface ContractBlock {
  repository_id: string | null;
  source_state: string | null;
  /** Which answer this is, including `no_registered_neighbours` and `no_contract_links`. */
  result_kind: string;
  registry_entries_total: number;
  links_total: number;
  /** Structurally zero. Counted rather than assumed, so a dropped row could be reported. */
  links_without_registry_entry: number;
  truncation: ContractTruncation | null;
  continuation: ContractContinuation;
  boundary: ContractBoundary;
  limitations: {
    link_is_directional_and_one_sided: string;
    contract_version_verdict_is_not_derived: string;
    no_link_is_reachable_from_a_local_graph_query: string;
  };
}

/** One registered neighbour, and what re-checking its path found. */
export interface RegistryEntry {
  registry_id: string;
  expected_repository_id: string;
  display_name: string;
  /** User-specific and absolute. Served only over the loopback API; never tracked by Git. */
  local_path: string;
  added_at: string;
  /** A `RegistryEntryStatus` member. */
  status: string;
  status_note: string;
  withdrawn_at: string | null;
  last_seen_state: string | null;
  last_seen_at: string | null;
  availability_checked_at: string | null;
  /** A `RegistryAvailability` verdict, re-derived from the filesystem on every read. */
  availability: string;
  availability_statement: string;
  /** A `RegistryRefusal` member, where a check fired. */
  refusal: string | null;
  refusal_statement: string | null;
  /** The repository id actually found at the path, when it was the wrong one. */
  observed_repository_id: string | null;
  usable: boolean;
  /** A `ContractFreshness` member, or null where the entry puts no qualification on a link. */
  freshness: string | null;
  freshness_note: string | null;
  /** Present on `/api/contracts/registry` only. Counted over every link, not over the page. */
  links_through_this_entry?: number;
}

/** One recorded cross-repository link. */
export interface ContractLink {
  link_id: number | null;
  /** `DEPENDS_ON` or `REFERENCES`. Never a row in the assertion graph. */
  relation_semantics: string;
  /** A `ContractRule` member. */
  contract_kind: string;
  contract_identity: string;
  /** A `ContractResolutionMethod` member. */
  resolution_method: string;
  resolution_method_note: string;
  /** Both recorded. Neither is compared: range satisfaction is not string inequality. */
  expected_contract_version: string | null;
  observed_contract_version: string | null;
  source_repository_id: string;
  source_state_at_resolution: string;
  source_entity_id: string | null;
  source_kind_snapshot: string | null;
  source_path: string;
  source_span: string;
  source_manifest_present: boolean;
  expected_target_repository_id: string;
  /** The state the neighbour was at when the link was resolved. */
  target_state_at_resolution: string | null;
  /** The state it is at now, where one could be read. */
  target_current_state: string | null;
  target_entity_id: string | null;
  target_kind_snapshot: string | null;
  target_name_snapshot: string | null;
  target_path_snapshot: string | null;
  target_span_snapshot: string | null;
  extractor_id: string;
  extractor_version: string;
  evidence_details: string | null;
  /** An `Ambiguity` member. Recorded on every row of an ambiguous identity; none is promoted. */
  ambiguity: string | null;
  /** An `UnsupportedForm` member, where a declined form still named a registered neighbour. */
  unsupported_reason: string | null;
  /** A `ContractLinkStatus` member. A withdrawn link is kept so its ending can be reported. */
  status: string;
  status_note: string;
  first_seen_at: string;
  last_seen_at: string;
  withdrawn_at: string | null;
  /** A `ContractFreshness` member, or null for *no qualification*. Never re-derived here. */
  freshness: string | null;
  freshness_note: string | null;
  /** True only when nothing qualifies the link. Stated so a null is not read as "unknown". */
  is_current: boolean;
  registry_entry: RegistryEntry;
}

export interface ContractLinkList extends ContractBlock {
  registry_id: string | null;
  links_matching_filter: number;
  links: ContractLink[];
}

export interface ContractRegistry extends ContractBlock {
  nothing_is_auto_registered: boolean;
  entries: RegistryEntry[];
}

export interface ContractVocabulary extends ContractBlock {
  vocabulary: {
    rules: ContractTerm[];
    resolution_methods: ContractTerm[];
    link_statuses: ContractTerm[];
    registry_entry_statuses: ContractTerm[];
    freshness: ContractTerm[];
    availability: ContractTerm[];
    registry_refusals: ContractTerm[];
    ambiguity: ContractTerm[];
    supported_forms: ContractTerm[];
    unsupported_forms: ContractTerm[];
    unresolved_reasons: ContractTerm[];
    manifest_refusals: ContractTerm[];
    scan_refusals: ContractTerm[];
  };
}

/*
 * ---- Slice 14d: human-confirmed memory ---------------------------------------------------------
 *
 * A memory record is the one thing in this database a human wrote, and almost every string on it
 * is therefore free text a person typed: the note itself, the author label, the claim key, the
 * reason a note ended, an event's note, a citation's path, and every field of the subject snapshot.
 * They are hostile on exactly the terms the head of this file sets and none of them is ever treated
 * as markup.
 *
 * **The two kinds are separate types on purpose.** `status` is stored and holds one of four values;
 * `views` are computed at read time and nothing writes one. They arrive under different keys, and
 * this file keeps them under different keys, because a mirror that merged them would be the first
 * place the distinction quietly healed itself.
 */

/** Where memory is written. Every memory answer carries it, and it names commands. */
export interface MemoryBoundary {
  /** Always `true`. `nerve serve` is read-only and is proven so on the database bytes. */
  read_only: boolean;
  statement: string;
  /**
   * Every `nerve memory` verb, as static text the server owns.
   *
   * Rendered **verbatim**, never with a record's own id substituted into one. A `memory_id` is
   * caller-supplied text that may hold any character, and a command assembled from one is a line a
   * reader is invited to paste into a shell.
   */
  commands: string[];
}

export interface MemoryTruncation {
  returned: number;
  /** What the store handed over before the window was taken. */
  total: number;
  truncated: boolean;
  limit: number;
}

export interface MemoryContinuation {
  supported: boolean;
  offset: number | null;
  next_offset: number | null;
  /** Why there is no continuation, when there is none. Present on the single-record route. */
  statement: string | null;
}

/**
 * The three closed sets, carried on every answer.
 *
 * A filter control is built from these rather than from a list mirrored in this app: two of them
 * are filterable and one is not, and a client that hard-coded the difference could offer a filter
 * on a value nothing ever wrote.
 */
export interface MemoryVocabulary {
  scopes: string[];
  stored_statuses: string[];
  derived_views: string[];
}

/** What the caller asked for, echoed verbatim, so an answer is never read as a wider one. */
export interface MemoryRequested {
  memory_id: string | null;
  scope: string | null;
  status: string | null;
  query: string | null;
  subject: string | null;
}

/** The subject as it was when the note was written. Every field is a copy; none is a pointer. */
export interface MemorySubjectSnapshot {
  entity_id: string;
  kind: string;
  name: string;
  /** Repository-relative. Empty for the repository entity, which is no file. */
  path: string;
  /** The selector the human named it with, verbatim. */
  selector: string;
}

export interface MemoryCitation {
  citation_id: number | null;
  cited_entity_id: string | null;
  cited_kind: string | null;
  cited_name: string | null;
  cited_path: string;
  cited_span: string | null;
  cited_at_state: string;
  created_at: string;
}

/** One entry of the append-only audit history. Nothing deletes one, including invalidation. */
export interface MemoryEvent {
  event_id: number | null;
  at: string;
  /** A `MemoryOperation` member. */
  operation: string;
  operation_note: string;
  /**
   * Carried off the vocabulary, never inferred from the two statuses being equal.
   *
   * Exactly one operation answers `false`, and an event whose `from_status` equals its `to_status`
   * is well-formed for that one and a defect for every other.
   */
  changes_status: boolean;
  from_status: string | null;
  to_status: string;
  note: string | null;
}

/** One record: everything stored about it, and everything true of it right now. */
export interface MemoryRecord {
  memory_id: string;
  /** **Stored.** A `MemoryStatus` member — one of four. */
  status: string;
  status_note: string;
  /** **Derived at read time.** A `MemoryView` member with the sentence it owns. */
  views: { view: string; note: string }[];
  /** Always `true`. Sent so no client has to decide which kind `views` is. */
  views_are_derived: boolean;
  subject: MemorySubjectSnapshot;
  /** **Derived.** A `MemorySubjectResolution` member: what the snapshot reaches now. */
  subject_resolution: string;
  subject_resolution_note: string;
  /** **Derived.** Every candidate, where the subject may have moved to more than one place. */
  subject_live_entity_ids: string[];
  /** A `MemoryScope` member. */
  scope: string;
  /** The scope's own sentence, or `null` where the stored value is outside the vocabulary. */
  scope_note: string | null;
  claim_key: string | null;
  /** The repository state the record was confirmed against. */
  anchor_state_id: string;
  /** **Derived.** The state this index describes now, or `null` when nothing is indexed. */
  current_state_id: string | null;
  content: string;
  author_label: string;
  /** Always `false`. There are no accounts, so the label is never an authentication. */
  author_label_is_an_identity: boolean;
  created_at: string;
  /** **Stored**, and the only writable direction of supersession. */
  supersedes_memory_id: string | null;
  /** **Derived** from the column above. There is no such column. */
  superseded_by_memory_id: string | null;
  superseded_by_is_derived: boolean;
  invalidated_at: string | null;
  invalidation_reason: string | null;
  citations: MemoryCitation[];
  events: MemoryEvent[];
}

/** The block every `/api/memory*` answer carries, assembled in one place on the server. */
export interface MemoryBlock {
  repository_id: string;
  current_repository_state: string | null;
  requested: MemoryRequested;
  /**
   * Which answer this is. **Three values, and two of them are absences that differ.**
   *
   * `no_memory_recorded` — nothing has ever been written here. `no_memory_matches` — records
   * exist and this question matches none. A client that rendered them alike would report the
   * second as the first.
   */
  result_kind: string;
  records_in_repository: number;
  records_matching: number;
  truncation: MemoryTruncation | null;
  continuation: MemoryContinuation;
  boundary: MemoryBoundary;
  vocabulary: MemoryVocabulary;
  limitations: {
    views_are_derived: string;
    superseded_by_is_derived: string;
    author_label_is_not_an_identity: string;
    subject_is_a_snapshot: string;
    no_delete_verb: string;
    memory_is_not_evidence: string;
  };
  /** The sentence for whichever absence this is, or `null` when records were returned. */
  absence_statement: string | null;
  records: MemoryRecord[];
}
