# `fixtures/gitobj` — Git object access (Slice 12a)

What `crates/nerve-index/src/gitobj/` must read, and what it must refuse. `expected.json` is the
machine-readable form of this file and `crates/nerve-index/tests/gitobj.rs` reads it; both were
written **before** the reader existed, which is this project's convention because a stated
expectation has repeatedly won a disagreement against an implementation.

## Provenance

| directory | how it got here |
|---|---|
| `packed/objects/pack/` | **a real packfile**, written by `git gc --aggressive` |
| `loose/objects/` | **real loose objects**, four of them, one of each type |
| `inventory.json` | **Git's own answer** for every object above: type, size, offset, delta depth, base |

Regenerate with `scripts/make_gitobj_fixtures.sh`. That script runs `git`; it is a development
tool, on the same side of the line as cloning the validation corpus. No Rust source references it,
and the gate asserts that (`no_rust_source_references_the_fixture_script`).

A hand-written pack would test a reading of the format rather than the format, which is why a real
one is committed. Byte-for-byte reproducibility is **not** claimed: a pack's name is a checksum of
its bytes, and its bytes depend on the zlib build and delta heuristics of the Git that wrote it.
What the script reproduces is the shape — the same commits, all four loose types, and a pack that
actually contains delta entries. The script **fails** rather than emitting a delta-free pack, so
the fixture cannot quietly stop exercising `OFS_DELTA`.

Committed size, measured: pack 2275 bytes, `.idx` 1604 bytes, four loose objects 496 bytes —
**4375 bytes total**.

Every per-object assertion is read out of `inventory.json` rather than written into the test, so
the expected type and byte length of each object is Git's claim and not Nerve's reader agreeing
with itself.

## What is committed, and what is generated

Three provenances, and the distinction is load-bearing:

- **Committed** — the real pack, the real `.idx`, the four real loose objects. These are the
  format, and nothing else can substitute for them.
- **Derived in the test** from those committed bytes — every corruption case. A corrupt `.idx` is
  the real `.idx` with four bytes changed, so it cannot drift away from the format it corrupts, and
  a reviewer can see exactly which bytes make it invalid.
- **Generated in the test from the bound it attacks** — the decompression bomb, the over-long
  delta chain, the pack-count overflow.

That last row is not a stylistic preference. Slice 11a-i measured what a committed placeholder
costs: `fixtures/trace-hostile` shipped four artifacts carrying tokens that nothing expanded, so
`__PAD_STRING__` reached the parser as fourteen ASCII bytes and four attacks tested nothing while a
green suite reported them as passing. A payload derived from `MAX_OBJECT_BYTES` is one byte past
whatever the bound currently says, so tightening the bound cannot disarm its own attack. See
`crates/nerve-index/tests/trace.rs::stage_hostile` for the pattern.

## The cases

`expected.json` carries the authoritative table. In summary, and grouped by what each one is for:

### It must read what Git wrote

| case | required |
|---|---|
| `real-pack-non-delta-entry` | every depth-0 entry reconstructs to Git's type and length; lookup goes through the `.idx` fanout |
| `real-pack-delta-entry` | every entry with `depth > 0` reconstructs, and the fixture must contain one at depth ≥ 2 |
| `loose-all-four-types` | commit, tree, blob and annotated tag headers all parsed |
| `object-absent-from-the-store` | `Ok(None)` — *not present* is neither an error nor a refusal |
| `worktree-dot-git-is-a-file` | resolves through `gitinfo`'s existing resolution, then follows `commondir` |
| `alternates-one-entry` | one hop followed, `alternates_followed == 1` |

### It must refuse what it cannot trust

| case | required | form |
|---|---|---|
| `truncated-pack` | refused, counted, no partial object | `pack-truncated` |
| `corrupt-idx-bad-magic` | pack skipped, store still opens | `idx-bad-magic` |
| `corrupt-idx-version-1` | refused **with the version stated** | `idx-unsupported-version` |
| `corrupt-idx-version-3` | refused, version stated | `idx-unsupported-version` |
| `corrupt-idx-fanout-not-monotonic` | refused, not clamped | `idx-fanout-not-monotonic` |
| `corrupt-idx-truncated` | declared count and file length must agree exactly | `idx-truncated` |
| `ref-delta-missing-base` | `Ok(None)` **and counted** | `delta-base-missing` |
| `delta-chain-past-max-depth` | refused at `MAX_DELTA_DEPTH` | `delta-depth-exceeded` |
| `delta-cycle` | terminates, by the same bound | `delta-depth-exceeded` |
| `decompression-bomb` | refused **during** inflate, peak allocation bounded | `object-too-large` |
| `loose-declared-size-disagrees` | refused rather than either value trusted | `loose-declared-size-disagrees` |
| `loose-header-malformed` | each refused with a stated reason | `loose-header-malformed`, `loose-unknown-type` |
| `pack-count-past-the-bound` | exactly `MAX_PACK_COUNT` loaded, excess counted | `pack-count-exceeded` |
| `alternates-chain` | second hop refused and counted | `alternate-chain-refused` |
| `alternates-outside-the-repository` | refused and counted | `alternate-escapes-repository-root` |
| `alternates-hostile-shape` | empty, control character, absent, self — each refused | `alternate-refused-shape` |
| `sha256-object-format` | `open()` refuses and **names the format** | `unsupported-object-format` |

### It must say what it cannot see

These produce no refusal. They populate `StoreLimits`, and the reason that is a separate category
is the one `bound`/`stale`/`unverified` exists for in Slice 11a and `CoverageEvidence::Absent` in
6b: *"there are no more commits"* and *"I cannot see further"* are different answers.

| case | required |
|---|---|
| `shallow-clone` | `StoreLimits.shallow` is `Some([<boundary oid>])` — a boundary, never a root |
| `no-shallow-file` | `StoreLimits.shallow` is `None`. `Some(vec![])` is a different claim and is never produced |
| `promisor-partial-clone` | `StoreLimits.promisor` is `true`, from either `.git/config` or a `.promisor` file |

## What this fixture deliberately does not contain

- **A `REF_DELTA` in the committed pack.** `git gc` writes `OFS_DELTA` (`pack.useOfsDelta`
  defaults to true), so a real pack from a real `gc` has none. `REF_DELTA` is exercised by packs
  built in the test, which is the only way to get one without hand-editing a real pack — and
  hand-editing it would make the *committed* artifact synthetic, which is precisely what
  committing a real pack was for.
- **A multi-pack-index or a commit-graph.** Both are caches of what the objects already say and
  Nerve ignores them; a fixture would assert an optimisation it does not have.
- **A SHA-256 repository's objects.** Only its `.git/config` is generated, because `open()` refuses
  before reading an object and the objects would therefore assert nothing.
