# Slice 12 — Git object access: the dependency analysis, before any library is chosen

2026-08-03. Written before the Slice 12 plan, because the brief is right that the choice must be
evidence-led rather than inherited from a continuation note.

**This document decides nothing about the historical model.** It answers one narrow question: *what
does Nerve have to be able to read, and what is the smallest correct way to read it?*

---

## 1. What the standard library and the current 101 crates provide: nothing

Measured, not assumed. Every package name in `Cargo.lock` was listed and searched. There is **no
compression library** — no `flate2`, no `miniz_oxide`, no `zlib`, no `libz-sys`, no `gix`, no `git2`.
The closest things present are `blake3` (a hash) and `rusqlite`/`libsqlite3-sys` (which bundles
SQLite, not zlib).

Rust's standard library has no inflate. So the question is not "which library is nicer" but **"can
Nerve read a Git object at all today?"** — and the answer is no.

## 2. Git objects are zlib-deflated. Measured on this repository

```
$ xxd -l 4 .git/objects/61/c291c117c56c00feb3530028732d029ba55b19
00000000: 7801 d55d
```

`78 01` is a zlib header (deflate, `FCHECK` valid, low compression). Every loose object is
zlib-wrapped; inside a packfile every entry's payload is deflate-compressed too. There is no
uncompressed path to a commit object, a tree object, or anything else.

**Consequence: a decompressor is mandatory, not a convenience.** No amount of clever plumbing-file
reading avoids it. `gitinfo.rs` gets away with zero dependencies today precisely because `.git/HEAD`
and `.git/packed-refs` are the *only* plain-text files in the object database's neighbourhood — they
are refs, not objects.

## 3. Loose versus packed: the measurement, and why this repository is misleading

```
Nerve's own .git:   1342 loose objects,  0 packfiles,  66 commits
```

**This repository is not representative, and treating it as such would be the mistake.** It has
never been cloned and never garbage-collected, so everything is loose. Two facts change the picture:

- **`git clone` always writes a packfile.** A user indexing a repository they cloned — the normal
  case — has history almost entirely packed, with only their own recent commits loose.
- **`git gc` packs, and runs automatically** via `gc.auto` once loose objects accumulate. This
  repository is 66 commits old; the threshold is 6700 loose objects by default.

Every repository in the validation corpus (`chalk`, `redux`, `zod`, `date-fns`, `vitest` — see
`docs/plans/slice-15-real-world-validation.md`) will be acquired by pinned-commit checkout, i.e.
cloned, i.e. packed.

**Consequence: loose-only support would work on Nerve's own repository and fail on essentially every
real one.** That failure mode is the worst available: it would pass the fixtures, pass a
self-referential smoke test, and then find no history in a user's repository. Shipping loose-only and
calling it "history support" is precisely the thing the brief forbids — *"do not claim full history
support while silently ignoring packed objects."*

## 4. What must therefore be supported

| form | required? | why |
|---|---|---|
| loose objects | **yes** | recent local commits |
| packfiles, non-delta entries | **yes** | a clone puts everything here |
| packfiles, `OFS_DELTA` / `REF_DELTA` | **yes** | most entries in a real pack are deltas; refusing them means refusing most of history |
| `.idx` v2 lookup | **yes** | finding an object in a pack without a linear scan |
| `.idx` v1 | no | superseded in 2007; refuse with a stated reason |
| multi-pack-index (`.midx`) | no | an optimisation; the individual `.idx` files remain valid |
| alternates (`objects/info/alternates`) | **read the file, follow one hop** | cheap, and a worktree of a shared object store is otherwise empty |
| worktrees (`.git` as a file) | **yes** | `gitinfo.rs` already handles this for HEAD; history must not regress it |
| shallow clones (`.git/shallow`) | **detect and report** | history is genuinely truncated; a boundary must be stated, not silently treated as a root |
| partial clones (`promisor` remotes) | **detect and refuse the missing objects** | fetching them requires network, which §2 of `CLAUDE.md` forbids |
| commit-graph (`.git/objects/info/commit-graph`) | no | an optimisation, and it is a cache of what the objects already say |

Shallow and partial clones are the two cases where the honest answer is *"history is incomplete and
here is why"* — the same three-valued shape as `bound / stale / unverified` in Slice 11a and
`CoverageEvidence::Absent` in Slice 6b. **Absence of history is not absence of change.**

## 5. The options, compared on evidence rather than preference

### Option A — `gix` (gitoxide)

Correct, actively maintained, MIT/Apache-2.0, pure Rust, no subprocess. It would deliver every row in
§4 including `.midx` and commit-graph.

**Rejected on dependency-delta grounds, and the number is the argument.** `gix` is a facade over
~30 sub-crates (`gix-object`, `gix-pack`, `gix-odb`, `gix-hash`, `gix-features`, `gix-ref`,
`gix-config`, `gix-url`, …) and pulls a substantial transitive set. Nerve is at **101** packages
total, deliberately: `CLAUDE.md` §12-equivalent discipline and `third_party/LICENSES.md` record every
one. A single dependency that grows the tree by roughly 50% is not "the smallest correct option" —
and it brings surface Nerve must never use, including `gix-url` and transport code, which is
outbound-network-adjacent in a product whose §2 invariant is that no product code has a network
client. Keeping a network-capable Git implementation in the tree and asserting by test that it is
never used is a weaker guarantee than not having it.

### Option B — `git2` (libgit2 bindings)

**Rejected.** A C library via `libgit2-sys`, which means a build-time C compilation and, more
importantly, a linked library that ships its own HTTP transport. Same objection as A, larger.

### Option C — `flate2` + an independent packfile reader

`flate2` with the `rust_backend` feature (`miniz_oxide`) is **pure Rust, no C, no build script, no
network, no process execution, no telemetry**, and MIT/Apache-2.0 — the same licence family as the
three tree-sitter grammars already present. Transitive cost is small: `flate2`, `miniz_oxide`, and
`adler2`. **Measured delta must be confirmed by `git diff Cargo.lock` at implementation time and
recorded in `third_party/LICENSES.md`; the estimate is +3, and the estimate is not the record.**

The cost is that Nerve writes the packfile reader itself: `.idx` v2 (a 256-entry fanout table, sorted
object ids, CRCs, offsets, and the 64-bit offset overflow table), pack entry headers, and
`OFS_DELTA` / `REF_DELTA` reconstruction with a bounded chain depth.

**This is the recommendation.** Three reasons, in order of weight:

1. **It is the smallest change that covers §4.** One decompressor plus a reader for a format Nerve
   genuinely needs, rather than an engine that also knows how to talk to a remote.
2. **The packfile format is a documented, stable, testable file format** — exactly the kind of thing
   this codebase already implements by hand and gates with fixtures: a Markdown block scanner
   (Slice 5b, which found 6 real defects), an LCOV reader (6a), and a `.git/HEAD` + `packed-refs`
   reader (`gitinfo.rs`). Delta reconstruction is the one genuinely intricate part, and it is
   arithmetic on bytes with a well-defined answer, which is the easiest kind of thing to gate.
3. **Clean-room is unaffected either way**, but worth stating: a decompressor is a foundational
   library in exactly the sense `CLAUDE.md` §1 permits, alongside parsers and hashers. Reading the
   *Git format* is reading a documented format, not reading a competitor's on-disk format — the
   prohibition in §1 is about CodeGraph/Graphify/GitNexus databases, and Git is neither a competitor
   nor a code-knowledge-graph product.

### Option D — shell out to `git cat-file --batch`

**Refused, and not for dependency reasons.** `crates/nerve-cli/tests/no_subprocess.rs` forbids
process creation in `crates/*/src/**`, and its module documentation names this exact case:

> "No package scripts, no build tools, no compilers, no test runners, **no `git` binary** — Git HEAD
> is read from `.git/HEAD` directly for exactly this reason."

The brief agrees independently: *"do not shell out to `git` from production core merely to avoid a
dependency if that violates the no-subprocess architecture."* It does violate it. Adding three
audited pure-Rust crates is a smaller cost than converting a structural invariant into a policy.

## 6. What this means for the slice's size, stated plainly

A packfile reader with `.idx` v2 lookup and delta reconstruction, gated with fixtures, is **on the
order of Slice 9's total size** — and that was two slices. It is object *access* only: it delivers no
historical entity, no query, and no user-visible answer on its own.

**So Slice 12 must split, and the split is forced by the same rule 8b and 9 were split under —
do not build the second half against a contract that is still moving:**

- **12a — object access.** `flate2` added and licence-recorded; loose objects; `.idx` v2; pack
  entries; `OFS_DELTA`/`REF_DELTA` with a bounded chain; alternates one hop; worktree `.git` files;
  shallow and partial detection reported as explicit states. Fixtures include a **real packfile**,
  committed as a fixture, plus a malformed-`.idx`, a truncated pack, a delta chain that exceeds the
  bound, and a `REF_DELTA` naming a missing base. Zero new entities, zero schema change: it is a
  reader, with its own unit gate, exactly as `coverage.rs` is a reader before `coverage_ingest.rs`
  writes anything.
- **12b — the historical model.** Commit entities, repository states, first/last seen, moves and
  rename *hypotheses* kept separate from identity links, change frequency, and explicitly labelled
  co-change. This is where the storage-strategy decision (delta vs selected-state vs on-demand
  reconstruction) belongs, and it must be measured rather than assumed — the brief is right that
  duplicating the whole graph per commit needs proof, not intuition.

## 7. Decisions recorded here so 12a does not relitigate them

1. **A decompressor is mandatory.** Git objects are zlib-deflated; measured `78 01` on this
   repository. There is no dependency-free path.
2. **Packed objects are mandatory.** A clone is packed; loose-only would pass Nerve's own repository
   and fail every corpus repository, which is the worst failure shape available.
3. **`flate2`/`rust_backend` over `gix` and `git2`**, on measured dependency delta and on keeping a
   network-capable Git implementation out of an offline-first product's tree.
4. **No `git` subprocess**, per `no_subprocess.rs` and the brief in agreement.
5. **Shallow and partial clones are reported states, not silent roots.**
6. **The dependency is not added until `git diff Cargo.lock` is measured and
   `third_party/LICENSES.md` records every package with its exact version and licence.** The +3
   estimate in §5 is an estimate.
