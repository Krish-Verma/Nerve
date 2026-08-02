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
] as const;
