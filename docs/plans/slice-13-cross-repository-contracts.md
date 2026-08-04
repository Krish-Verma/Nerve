# Row 13 — cross-repository contracts

**Status:** planned, not started
**Depends on:** row 12 complete (registry freshness reuses `repository_state`; contract staleness
reuses the shape 12c-i's `history_freshness` establishes)
**Schema:** v6 → **v7**. Additive: a registry table and a contract-link table. See §4.
**Roadmap row:** 13

---

## 1. What this row is not

"Cross-repository" is not global code search, and the brief says so. The rule this plan is built
around, restated from `CLAUDE.md` §3 and from two prior accepted refusals:

> A trusted link is created from an **explicit stated declaration** and from nothing else.

Refused outright, each with a reason rather than a deferral:

| refused source of a link | why |
|---|---|
| similar names | `CLAUDE.md` §3 — identity is never established by fuzzy name matching alone |
| a matching endpoint string alone | the same string in two repositories is not a contract between them; the contract is the *document* that declares it (§3.4) |
| embedding similarity | there is no model in this product and there will not be one (`CLAUDE.md` §2 — never call an external LLM from product code) |
| directory proximity | a sibling checkout is a filesystem accident |
| a package name with no registry and version context | `left-pad` in two repositories may be two packages |

Nerve has already refused a *weaker* version of this inference twice, and both refusals are load
bearing precedent: `ADR_DESCRIBES_COMPONENT` was refused because no deterministic rule separates
"describes" from "mentions" (`CONTINUATION.md:495`), and a bare code-span name in prose is never a
link (`CONTINUATION.md:498`).

---

## 2. The contract set, chosen on dependency evidence

The brief says *"Do not optimize for format count. Prefer a small set with measurable high
precision."* The set below was chosen by asking which formats can be read with **zero new
dependencies** and which have entities on **both** ends for Nerve to link.

`Cargo.toml` at the workspace root already carries `serde_json = "1"` and `toml = "0.9"`, and
`nerve-index` already depends on both. So:

| # | contract | file | parser | new deps | both ends indexed? |
|---|---|---|---|---|---|
| **C1** | npm local / workspace dependency | `package.json` `dependencies`, `devDependencies`, `peerDependencies` with `file:` or `workspace:` | `serde_json` | **none** | yes — TS/JS is row 1–2 |
| **C2** | npm package export resolution | `package.json` `name` + `exports` / `main` / `module` / `types` | `serde_json` | **none** | yes — resolves a specifier to a *file entity* in the target |
| **C3** | Python local path dependency | `pyproject.toml` PEP 621 / Poetry / uv path dependencies, plus `project.name` | `toml` | **none** | yes — Python is row 9 |

C1 and C3 are repository-to-repository. **C2 is the one that produces an entity-to-entity link**,
and it is why the set is worth shipping: repo A's `import { x } from 'pkg-b/sub'` becomes
`A/src/app.ts REFERENCES B/src/sub.ts` with the evidence chain *this import specifier · this
`file:` dependency · this `name` · this `exports` entry*, every step a stated declaration in a
file, none of them a guess.

### 2.1 Deferred, with the cost named

- **OpenAPI.** The valuable pairing exists and it is specific: Nerve already extracts `Endpoint`
  entities from FastAPI, Flask and Express (rows 10a/10b), so an OpenAPI document declaring
  `GET /users/{id}` against a handler serving `GET /users/{user_id}` is a real cross-repository
  contract with a real endpoint on the far side. Two costs stop it being in the first cut. **YAML
  needs a new dependency** — most published OpenAPI is YAML, and JSON-only support would ship a
  format that usually cannot be read. And **path-template matching is a resolution rule, not a
  quotation**: `{id}` and `{user_id}` are the same path only under a normalisation Nerve would be
  inventing, so it needs its own measured precision gate. Row **13d**, on its own evidence.
- **GraphQL, Protobuf, event and queue schemas** — each needs a parser, so each needs a dependency
  justified on its own. Not refused; costed and deferred.
- **Deployment manifests, shared database schemas, generated-client metadata** — a link from a
  deployment manifest asserts a runtime topology, not a code contract, and Nerve's evidence model has
  no source type for "this is how it is deployed". Refused for row 13 rather than deferred.

---

## 3. Sub-slices

Row 13 is four sub-slices, for the reason recorded five times in this repository
(`ROADMAP.md:251`): a slice bundling a store layer and a surface has cost this project five agents.

| | scope | independently testable at |
|---|---|---|
| **13a** | The registry — schema v7, `nerve repo add/list/remove/relocate`, availability, states | a registry with a missing, moved and stale entry, no contracts at all |
| **13b** | C1 + C3 extraction and linking, precision measured **per rule** | fixture pairs with ground truth written first |
| **13c** | C2 — specifier → export → target file entity, its own precision gate | the entity-to-entity link |
| **13d** | Surfaces: CLI, `/api/contracts*`, MCP, reference UI | contract tests per surface |

OpenAPI, if taken, is **13e** and needs its own dependency review.

---

## 4. Schema v7 — two tables, and the decision behind them

### 4.1 Where a cross-repository link lives, when there are two databases

This is the architectural question of the row and it has a wrong answer that looks natural.

Nerve's database is **per repository** (`.nerve/nerve.db` at the repository root). A cross-repository
link has one end in each of two databases. Three candidate placements:

| | rejected because |
|---|---|
| write the link into **both** databases | two writable copies of one fact, free to diverge, and indexing B would have to write into A — Nerve has never written outside the repository it was pointed at |
| a **separate global** registry database | a new on-disk location outside any repository, needing its own path policy, its own permissions and its own reset story, for a product whose whole storage story is "one directory in your repository" |

**Chosen:** the registry and the links live in the database of the repository the command is run
from, and they are *that repository's stated view of its neighbours*. The link is an assertion made
by A's index about a contract A itself declares. Reading B's details opens B's database
**read-only** — which is how freshness (§6) is established, and is the same `query_only=ON`
connection `nerve check` already proves read-only on the bytes (Slice 7c-i).

The consequence is stated rather than hidden: **a link is directional and one-sided.** A's database
knows A depends on B. B's database does not know it is depended upon until B registers A. That is
honest about what was observed, and the alternative is writing into a repository nobody asked Nerve
to touch.

### 4.2 The tables

```
repo_registry(
  repo_id, registry_id, display_name, local_path, added_at,
  last_seen_state, last_seen_at, capabilities, availability_checked_at
)
contract_link(
  repo_id, registry_id, contract_kind, contract_identity, contract_version,
  source_path, source_span, source_entity_id,
  target_path, target_entity_ref,
  resolution_method, extractor_id, extractor_version,
  target_state_at_resolution, ambiguity, unsupported_reason
)
```

`local_path` is the one field that is **user-specific and absolute**. It lives only in
`.nerve/nerve.db`, which `.gitignore:2` already covers, and §9 makes that a tested property rather
than a convention.

### 4.3 No new `Relation`, and C2 is the exception that proves it

C1 and C3 emit **no** `Relation` and no `assertion` row: a repository-to-repository dependency has no
entity on either end that Nerve models, so it lives in `contract_link` only — the same reasoning that
kept commits out of the entity table in 12b.

C2 does have entities on both ends, and it is therefore the one that must be decided rather than
assumed. **It does not get a new relation name either.** `REFERENCES` already means "this names
that", and `CLAUDE.md` §3's rule that relation names are endpoint-kind-agnostic
(`CONTINUATION.md:494`) says the cross-repository-ness belongs in the *evidence*, not in the relation.
What C2 adds is a new `EvidenceSourceType` — the first since 5d-i's `FILESYSTEM_OBSERVED` — because
"a package manifest in another repository states this" is genuinely a new kind of evidence and
appending is safe (`ordinal()` is a position in `ALL`, `mask_bit()` is `1 << ordinal`, and the mask is
stored: `CONTINUATION.md:504`).

**Appending is load-bearing and is asserted, not assumed.** All twelve existing source types are
individually pinned, so an insertion fails where it is written.

---

## 5. Precision, measured per rule and never summed

Rows 2a, 9b, 10a and 10b each established this and it is not reopened: **two rules, two tables, no
combined number.** C1, C2 and C3 each get their own precision table, each with ground truth written
**before** the resolver, and none of the three is summed with another. 10b's `Views.handle` is the
precedent for the other half: a rule that declines a shape declares a **false negative** rather than
quietly resolving it, because resolving it on one language only would make the link mean different
things per language.

The gate: **FP = 0.** Recall is reported, not gated — the same split rows 2a and 9b use, and for the
same reason: a recall gate is met by guessing.

---

## 6. Freshness, and the twelve states that must stay distinct

A cross-repository link is stale in more ways than any fact Nerve has stored before, because it has
two repository states and either can move. The brief lists twelve situations. All twelve must be
distinguishable in the response, and none may be rendered as a current link:

`source_changed` · `target_changed` · `both_changed` · `contract_version_mismatch` ·
`generated_client_stale` · `target_repository_missing` · `target_repository_moved` ·
`contract_file_missing` · `duplicate_contract_identity` · `conflicting_definitions` ·
`target_partially_indexed` · `contract_deleted` · `registry_entry_removed`

Two of them are the ones a first draft collapses:

- **`target_repository_missing` is not `target_repository_moved`.** A path that no longer exists and a
  path that now holds a *different* repository are different facts with different remedies, and the
  second is the dangerous one — a registry entry silently re-pointed at another checkout would make
  every link about the wrong repository. Identity is checked against the recorded `repo_id`, not
  against the path.
- **`target_partially_indexed` is not `target_changed`.** This is Slice 7c-i's `Stale` / `Unverified`
  distinction, third instance: nothing was *observed* to change, part of the target was never looked
  at. Reporting "unknown" as "current" is how a truncated sweep becomes a clean bill.

---

## 7. Security, and the new part

The new part is that **Nerve reads a directory the user did not point it at.** Every prior row read
the repository given on the command line; row 13 reads a second repository named by a registry entry.

Controls, each with a test:

1. Registration is an **explicit mutating command**. Nothing is auto-discovered from sibling
   directories — that is the "directory proximity" link §1 refuses, one layer down.
2. The target is opened **read-only** (`query_only=ON`), byte-verified before and after, as 7c-i does.
3. The target's **path is validated at registration and re-validated at every use** — a registry row
   is untrusted input once written, because the file it points at can change.
4. Nerve reads the target's **database and its manifest files**, and nothing else. It does not index
   the target as a side effect: doing so would write into a repository the user pointed at only as a
   dependency.
5. `display_name`, `contract_identity`, `contract_version` and every manifest string are **untrusted
   repository content** on exactly the terms T7 already sets, and are confined to MCP's
   `repository_content` envelope by the existing property test.
6. A registry entry whose target is a **symlink out of the user's control** is refused with the
   existing symlink-escape guard, not followed.

`docs/THREAT-MODEL.md` gains **T12** for the second-repository read. It is a new trust boundary and it
does not fit inside T2's path safety, which is about paths *within* one repository.

---

## 8. What must not regress

- `no_subprocess.rs` and `no_network.rs` **byte-untouched**. A registry is not a package manager: Nerve
  never runs `npm`, `pip`, `poetry` or `git`, and never resolves a dependency from a network registry.
  A `dependencies` entry that is not `file:` or `workspace:` is **recorded as unsupported**, never
  fetched.
- `symbols_total`, `entities_total`, `entity_fts` and selector resolution: C1 and C3 add no entities.
  C2 adds no entity either — it links two that already exist.
- `Cargo.lock` at **106** packages for 13a–13d. If 13e takes YAML, the delta is measured and recorded
  in `third_party/LICENSES.md` before the code is written, per 12a's finding that the *measured* delta
  was +5 against an estimated +3.

---

## 9. Acceptance criteria for row 13

1. The supported contract set is explicit, and an unsupported form is **recorded as unsupported with
   its form named**, never silently dropped. Asserted by a tally, not by inspection.
2. Precision measured **per rule** (C1, C2, C3), ground truth written first, FP = 0, recall reported.
   No combined number exists anywhere in the output or the docs.
3. All thirteen freshness situations in §6 are producible and individually pinned, with
   `target_repository_moved` and `target_partially_indexed` each distinguished from its neighbour.
4. Registration is explicit; no sibling directory is ever auto-registered — asserted by a negative
   test with a sibling checkout present.
5. The target database is byte-identical after every read, verified by hash.
6. **No user-specific absolute path is tracked by Git.** Asserted by a test that runs
   `git ls-files` equivalents over the repository and fails on a path resembling a home directory,
   with an anti-vacuity floor so a scan that finds nothing is distinguishable from one that scanned
   nothing.
7. Fuzzy linking is absent, and *asserted* absent: a fixture pair with same-named packages, no
   `file:` dependency between them, and adjacent directories produces **zero** links.
8. `no_subprocess.rs` / `no_network.rs` byte-untouched; `Cargo.lock` unchanged for 13a–13d.
9. T12 written into `docs/THREAT-MODEL.md` and attack-verified, not merely documented.
10. Every surface reads one shared service. A source scan proves no surface computes freshness,
    ambiguity or availability independently — the same guard shape 12c-i introduces for history
    wording.
11. Full gate: `fmt`, `clippy -D warnings`, `cargo test --workspace --no-fail-fast`,
    `cargo build --release`, Python tracer suite, `scripts/final_acceptance.sh`.

---

## 10. Refutations of this plan's own first draft

1. **A global registry database was drafted first**, because it is where a registry "obviously" goes.
   It puts a new writable location outside every repository, for a product whose storage story is one
   gitignored directory. §4.1.
2. **The link was drafted as bidirectional.** Writing A's dependency into B's database means indexing
   one repository writes into another, which Nerve has never done. Directional and one-sided, with the
   asymmetry stated. §4.1.
3. **A new `DEPENDS_ON` relation was drafted for C1.** There is no entity on either end — a
   repository-to-repository dependency has no `entity` row to be the endpoint of, which is 12b's
   commits-are-not-entities argument in a new place. §4.3.
4. **A `CROSS_REPO_REFERENCES` relation was drafted for C2**, and it violates the recorded rule that
   relation names are endpoint-kind-agnostic. The cross-repository-ness is *evidence*, so it is a new
   `EvidenceSourceType` and `REFERENCES` is reused. §4.3.
5. **OpenAPI was drafted into the first cut** on the strength of the endpoint pairing, which is
   genuinely the most valuable link in the row. It needs a YAML dependency and a path-template
   normalisation rule that is a resolution Nerve would be inventing. Both need their own gate. §2.1.
6. **`target_repository_missing` and `target_repository_moved` were one state.** The second is the
   dangerous one, and collapsing them means a registry entry re-pointed at a different checkout makes
   every link describe the wrong repository. §6.
</content>
