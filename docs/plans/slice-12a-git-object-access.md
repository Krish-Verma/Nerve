# Slice 12a — Git object access

2026-08-03. `docs/plans/slice-12-git-object-access-analysis.md` settled *which dependency* and *what
must be supported*. This settles the design. Read that first; nothing in it is relitigated here.

**12a creates no entity, writes no row, changes no schema and answers no user question.** It is a
reader, in the same sense `coverage.rs` is a reader before `coverage_ingest.rs` writes anything. That
is deliberate: 12b builds the historical model on top, and building it against a moving reader is the
mistake 8b and 9 were split to avoid.

---

## 1. Where it lives, and why not a crate

`crates/nerve-index/src/gitobj/`. Not a new crate, for one reason that settles it: `gitinfo.rs`
already reads `.git/HEAD` and `.git/packed-refs` and already lives in `nerve-index`. Git reading is an
indexing concern here, and a sixth crate whose only consumer is the fifth is structure for its own
sake.

Modules:

| module | reads |
|---|---|
| `oid.rs` | a 20-byte object id, hex parsing, ordering |
| `inflate.rs` | the one place `flate2` is called, with the output bound applied |
| `loose.rs` | `objects/ab/cdef…` |
| `packidx.rs` | `.idx` v2 — fanout, sorted oids, CRCs, offsets, the 64-bit overflow table |
| `pack.rs` | pack entry headers, `OFS_DELTA` / `REF_DELTA` reconstruction |
| `store.rs` | the façade: alternates, pack discovery, loose-then-pack lookup |
| `commit.rs` | a commit object's fields |
| `tree.rs` | a tree object's entries |

`commit.rs` and `tree.rs` are in 12a rather than 12b because they are **file formats with fixtures**,
which is what this slice is for. They produce plain structs, not entities.

## 2. The public shape

```rust
pub struct ObjectStore { /* … */ }

impl ObjectStore {
    /// `git_dir` is the resolved `.git` directory — `gitinfo.rs` already resolves the
    /// `.git`-is-a-file worktree case, and that resolution is reused rather than reimplemented.
    pub fn open(git_dir: &Path) -> Result<Self>;

    /// `Ok(None)` means *not present in this store*, which is a different fact from an error and
    /// from a refusal. A partial clone makes this the ordinary case, not an exception.
    pub fn read(&self, oid: &Oid) -> Result<Option<Object>>;

    /// What this store cannot see, and why. Never inferred by the caller from an empty result.
    pub fn limits(&self) -> &StoreLimits;
}

pub enum Object { Commit(Vec<u8>), Tree(Vec<u8>), Blob(Vec<u8>), Tag(Vec<u8>) }
```

**`StoreLimits` is the whole point of the slice's honesty.** It carries `shallow: Option<Vec<Oid>>`
(the boundary commits from `.git/shallow`), `promisor: bool` (a partial clone, so a missing object may
exist elsewhere), `unsupported_index_versions: Vec<u32>`, `alternates_followed: usize`,
`alternates_refused: usize`. A shallow clone's history genuinely ends, and *"there are no more
commits"* and *"I cannot see further"* are different answers — the same three-valued discipline as
`bound`/`stale`/`unverified` in 11a and `CoverageEvidence::Absent` in 6b.

## 3. Bounds, because `.git` is attacker-controlled input

This is the security substance of 12a and it is a **new** threat surface: until now Nerve read `.git`
only for two plain-text ref files. Compressed data with a self-described output size is a classic
amplification vector, and a delta chain is a graph a hostile pack can make cyclic.

| bound | value | why |
|---|---|---|
| `MAX_OBJECT_BYTES` | 64 MiB | one object's inflated size. The **inflate is bounded as it streams**, not checked afterwards — checking afterwards means having already allocated the bomb |
| `MAX_DELTA_DEPTH` | 64 | Git's own default `pack.depth` is 50. A chain past this is refused, not followed |
| `MAX_PACK_COUNT` | 256 | a directory of thousands of `.idx` files is not a repository Nerve needs to serve |
| declared-size disagreement | refuse | a loose object's header states its size; if the inflated stream disagrees, **refuse rather than trust either** |

A `REF_DELTA` naming a base that is absent is `Ok(None)` with a counted reason, never a panic and never
a partially-reconstructed object. A cyclic chain is caught by the depth bound rather than by cycle
detection, because the bound is needed anyway and two mechanisms for one hazard is one too many.

**Threat-model rows to add:** T9 gains `.git` object data as untrusted input alongside coverage and
trace artifacts, and the amplification bound is the control. T10 is unaffected in kind but the
dependency count moves.

## 4. Decisions this slice makes, with reasons

**SHA-256 repositories are detected and refused, not silently misread.** `extensions.objectFormat =
sha256` in `.git/config` means every oid is 32 bytes. Supporting both doubles the hash plumbing through
every module for a format that is still experimental and rare. Detected, reported through
`StoreLimits`, and refused with a named reason — the failure mode to avoid is reading a SHA-256
repository as if it were SHA-1 and producing 20-byte prefixes of real ids.

**Object content is not verified against its id.** Git verifies; Nerve will not, and the reason is
scope rather than laziness: it would mean adding a SHA-1 implementation to detect corruption that
`git fsck` exists for, in a repository Git itself is managing. **Recorded as a limitation in
`StoreLimits`' documentation**, because an unstated non-check is the kind of thing a later reader
assumes was done. The size-disagreement check above catches the cases that would otherwise produce
silently wrong bytes.

**`.idx` v1 is refused with a stated version**, not skipped silently. It was superseded in 2007; a
repository still carrying one gets told so.

**Multi-pack-index and commit-graph are ignored, not read.** Both are caches of what the objects
already say. Reading them would be an optimisation whose failure mode is disagreeing with the source.

**Alternates are followed exactly one hop.** `objects/info/alternates` can chain; one hop covers the
shared-object-store worktree case that motivates the feature, and an unbounded chain is a path-traversal
surface pointed at arbitrary directories. Each alternate goes through the **same** path guard as every
other path Nerve reads (`nerve_store::selector_shape` and the repository root check), and one refused
is counted rather than skipped.

## 5. Fixtures

Under `fixtures/gitobj/`. **A real packfile is committed**, and it must be: a hand-written pack would
test a reading of the format rather than the format.

Produced by a script (`scripts/make_gitobj_fixtures.sh`) that creates a tiny repository, commits a few
files, runs `git gc`, and copies the resulting `.pack`/`.idx` in. That script runs `git` — at
**fixture-creation** time, by a developer, exactly as the validation corpus is acquired by a developer.
No product code goes near it, and the script is committed so the fixture is reproducible rather than
mysterious.

Cases, each its own file and each with a stated expected outcome:

| fixture | asserts |
|---|---|
| a real pack + `.idx` | non-delta and delta entries both reconstruct; oid lookup goes through the fanout |
| loose objects, all four types | commit/tree/blob/tag headers parsed |
| truncated `.pack` | refused, counted, no partial object |
| corrupt `.idx` (bad magic, bad version, fanout not monotonic) | each refused with its own reason |
| `REF_DELTA` naming a missing base | `Ok(None)` and counted |
| a delta chain past `MAX_DELTA_DEPTH` | refused |
| a **decompression bomb** — small input, enormous declared output | refused *during* inflate, and the test asserts peak allocation stayed bounded |
| `.git` as a file (worktree) | resolves, reusing `gitinfo.rs` |
| `objects/info/alternates` with one entry, and with a chain | one hop followed, the second refused and counted |
| `.git/shallow` present | `StoreLimits.shallow` populated; **not** treated as a root |
| `extensions.objectFormat = sha256` | refused with the format named |

The bomb fixture must be **generated in the test from the bound**, not committed — same rule as 11a's
`stage_hostile`, and for the reason 11a-i measured: a committed placeholder that nothing expands is an
attack that tests nothing.

## 6. Acceptance

1. `flate2` with `rust_backend` added; `git diff Cargo.lock` **measured** and every added package
   recorded in `third_party/LICENSES.md` with its exact version and licence. The analysis estimated +3;
   the estimate is not the record.
2. `default-features = false, features = ["rust_backend"]` — asserted by a test that reads
   `Cargo.toml`, because the default feature set pulls a C backend and the point is that there is no C.
3. Every fixture row above passes, and the bomb fixture is generated from `MAX_OBJECT_BYTES`.
4. Zero new entity kinds, zero new relations, **`SCHEMA_VERSION` unchanged**, no migration.
5. `no_subprocess.rs` and `no_network.rs` byte-untouched; `scripts/make_gitobj_fixtures.sh` is not
   referenced by any Rust source.
6. A mutation probe per bound: removing each of the four bounds fails a named test.
7. The full gate.
