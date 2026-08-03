#!/usr/bin/env bash
#
# Regenerate `fixtures/gitobj/` from a real Git repository.
#
# ---------------------------------------------------------------------------------------------
# THIS IS A DEVELOPMENT TOOL AND IT RUNS `git`.
#
# `crates/nerve-cli/tests/no_subprocess.rs` forbids process creation in `crates/*/src/**`, and
# `docs/plans/slice-12-git-object-access-analysis.md` §5 Option D refuses shelling out to `git`
# from product code for exactly that reason. This script is on the other side of that line: it is
# run by a developer, once, to *acquire a fixture*, in the same category as the validation corpus
# being cloned by a developer. No Rust source references this file, and
# `crates/nerve-index/tests/gitobj.rs::no_rust_source_references_the_fixture_script` asserts so.
# ---------------------------------------------------------------------------------------------
#
# What it produces, and why each piece exists:
#
#   fixtures/gitobj/loose/objects/<xx>/<38>   one loose object of each of the four types.
#   fixtures/gitobj/packed/objects/pack/*     a real packfile and its `.idx` v2.
#   fixtures/gitobj/inventory.json            what **Git** says those objects are.
#
# `inventory.json` is the important one. Every per-object assertion in the gate is read out of it,
# so the expected values come from Git rather than from Nerve's own reader agreeing with itself.
# It is produced by `git cat-file --batch-check` and `git verify-pack -v`, and the script **fails**
# if the pack contains no delta entry — a fixture that quietly stopped exercising `OFS_DELTA`
# would leave the hardest half of `pack.rs` untested while the suite still went green.
#
# Byte-for-byte reproducibility is not claimed: a packfile's name is a checksum of its contents and
# its contents depend on the zlib implementation and the delta heuristics of the Git that wrote it.
# What is reproducible is the *shape* — the same commits, the same four loose types, a pack with
# delta entries — which is what the gate asserts.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/.." && pwd)"
out="$repo_root/fixtures/gitobj"

if ! command -v git >/dev/null 2>&1; then
  echo "git is required to create these fixtures (it is not required to use them)" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required to write inventory.json" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A fixed identity and a fixed clock, so re-running this produces the same object ids for the same
# content. The two GIT_CONFIG_* redirections keep the developer's own ~/.gitconfig — signing keys,
# hooks, `core.autocrlf`, template directories — out of the fixture.
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_NAME="Nerve Fixture"
export GIT_AUTHOR_EMAIL="fixture@nerve.invalid"
export GIT_COMMITTER_NAME="Nerve Fixture"
export GIT_COMMITTER_EMAIL="fixture@nerve.invalid"
export GIT_AUTHOR_DATE="2026-01-01T00:00:00+0000"
export GIT_COMMITTER_DATE="2026-01-01T00:00:00+0000"
export TZ=UTC

cd "$work"
git init -q -b main repo
cd repo
git config gc.auto 0
git config commit.gpgsign false
git config tag.gpgsign false
git config core.autocrlf false

# `data.txt` is 200 near-identical lines. Small files give Git's delta compressor nothing to work
# with, and a pack of whole entries would leave `OFS_DELTA` reconstruction — the one genuinely
# intricate part of the format — unexercised. Three commits each edit a handful of lines, so the
# three blobs are highly similar and delta compression is the cheaper representation.
write_data() {
  local marker="$1"
  : >data.txt
  local i
  for i in $(seq 0 199); do
    printf 'row %04d: the quick brown fox jumps over the lazy dog\n' "$i" >>data.txt
  done
  printf 'marker: %s\n' "$marker" >>data.txt
}

printf 'A fixture repository. Not a product artifact.\n' >README.md
write_data one
git add README.md data.txt
git commit -q -m "first"

write_data two
printf 'row 0007: a second edit, to make the blobs differ in the middle\n' >>data.txt
git add data.txt
git commit -q -m "second"

git tag -a v1 -m "an annotated tag, so the fixture carries all four object types"

mkdir -p src
printf 'export const value = 1;\n' >src/lib.ts
write_data three
git add src/lib.ts data.txt
git commit -q -m "third"

# Two more revisions of the same file, so the delta compressor has enough near-identical blobs to
# build a chain rather than a flat set of depth-1 deltas.
for marker in four five; do
  write_data "$marker"
  printf 'revision %s adds a line at the end as well\n' "$marker" >>data.txt
  git add data.txt
  git commit -q -m "$marker"
done

# ---- loose objects, one of each type, copied before anything is packed ------------------------

rm -rf "$out/loose"
mkdir -p "$out/loose/objects"

copy_loose() {
  local oid="$1"
  local dir="${oid:0:2}"
  local rest="${oid:2}"
  mkdir -p "$out/loose/objects/$dir"
  cp ".git/objects/$dir/$rest" "$out/loose/objects/$dir/$rest"
}

loose_commit="$(git rev-parse HEAD)"
loose_tree="$(git rev-parse 'HEAD^{tree}')"
loose_blob="$(git rev-parse 'HEAD:README.md')"
loose_tag="$(git rev-parse v1)"

for oid in "$loose_commit" "$loose_tree" "$loose_blob" "$loose_tag"; do
  copy_loose "$oid"
done

# ---- the pack ---------------------------------------------------------------------------------

git gc --aggressive --prune=now -q

pack_idx="$(echo .git/objects/pack/pack-*.idx)"
pack_pack="${pack_idx%.idx}.pack"
if [ ! -f "$pack_idx" ] || [ ! -f "$pack_pack" ]; then
  echo "git gc produced no packfile; the fixture cannot be built" >&2
  exit 1
fi
if [ "$(ls -1 .git/objects/pack/*.idx | wc -l | tr -d ' ')" != "1" ]; then
  echo "expected exactly one pack after gc" >&2
  exit 1
fi

rm -rf "$out/packed"
mkdir -p "$out/packed/objects/pack"
cp "$pack_pack" "$pack_idx" "$out/packed/objects/pack/"

# ---- what Git says all of this is -------------------------------------------------------------

git verify-pack -v "$pack_idx" >"$work/verify.txt"
git cat-file --batch-all-objects --batch-check >"$work/check.txt"

python3 - "$work/verify.txt" "$work/check.txt" "$out/inventory.json" \
  "$(basename "$pack_pack")" "$(basename "$pack_idx")" \
  "$loose_commit" "$loose_tree" "$loose_blob" "$loose_tag" <<'PY'
import json, os, sys

verify_path, check_path, out_path, pack_name, idx_name = sys.argv[1:6]
loose_commit, loose_tree, loose_blob, loose_tag = sys.argv[6:10]

# `git cat-file --batch-check` prints "<oid> <type> <size>".
types = {}
with open(check_path) as handle:
    for line in handle:
        parts = line.split()
        if len(parts) == 3:
            types[parts[0]] = (parts[1], int(parts[2]))

# `git verify-pack -v` prints, per object:
#   <oid> <type> <size> <size-in-pack> <offset> [<depth> <base-oid>]
# followed by chain-length histogram lines and a summary. Only the per-object lines have an oid.
#
# Two traps in that format, both measured rather than assumed. First, the `<type>` column is the
# object's *logical* type — a delta entry says `blob`, never `ofs-delta` — so a delta is identified
# by the presence of the trailing depth and base oid, not by the type word. Second, the `<size>`
# column for a delta entry is the size of the delta representation, not of the object. Both `type`
# and `size` are therefore taken from `--batch-check`, which resolves the delta first.
entries = []
with open(verify_path) as handle:
    for line in handle:
        parts = line.split()
        if len(parts) < 5 or len(parts[0]) != 40:
            continue
        oid = parts[0]
        object_type, object_size = types[oid]
        entry = {
            "oid": oid,
            "type": object_type,
            "size": object_size,
            "offset": int(parts[4]),
            "depth": int(parts[5]) if len(parts) >= 7 else 0,
        }
        if len(parts) >= 7:
            entry["base_oid"] = parts[6]
        entries.append(entry)

entries.sort(key=lambda e: e["oid"])

deltas = [e for e in entries if e["depth"] > 0]
if not deltas:
    sys.exit(
        "the pack contains no delta entry, so the fixture would not exercise delta "
        "reconstruction at all; adjust the repository content and re-run"
    )

inventory = {
    "note": "Generated by scripts/make_gitobj_fixtures.sh. Every value here is Git's own answer.",
    "pack": {"pack": pack_name, "idx": idx_name},
    "packed_objects": entries,
    "delta_entry_count": len(deltas),
    "max_delta_depth": max(e["depth"] for e in entries),
    "loose_objects": [
        {"oid": oid, "type": types[oid][0], "size": types[oid][1]}
        for oid in (loose_commit, loose_tree, loose_blob, loose_tag)
    ],
}

with open(out_path, "w") as handle:
    json.dump(inventory, handle, indent=2, sort_keys=True)
    handle.write("\n")

print("packed objects:", len(entries), "delta entries:", len(deltas))
PY

echo "fixture sizes:"
find "$out/packed" "$out/loose" -type f -exec ls -l {} \; | awk '{print $5, $9}'
echo "total bytes:"
find "$out/packed" "$out/loose" -type f -print0 | xargs -0 wc -c | tail -1
