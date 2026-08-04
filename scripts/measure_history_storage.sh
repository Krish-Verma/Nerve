#!/usr/bin/env bash
#
# Measure the storage cost of the two candidate history designs, on a real repository.
#
# Slice 12b had to choose between storing a full snapshot of repository membership at every
# commit and storing only what each commit changed. The roadmap row required the choice to be
# *measured* rather than assumed, and this script is that measurement, kept so the numbers in
# docs/plans/slice-12b-historical-model.md §4 can be re-derived instead of quoted.
#
#   snapshot path-rows  =  sum over commits of |tree(commit)|
#   delta change-rows   =  sum over commits of |paths changed by commit|
#
# The ratio between them is the quantity that decides the design, and it is not a constant: the
# snapshot cost is O(commits x tree_size) while the delta cost is O(total churn), so the ratio
# grows with history depth. Measuring one repository would have been misleading.
#
# READ-ONLY. This script writes nothing inside the repository it measures, and reads no file
# contents -- only object *counts*. It is safe to point at a repository you care about.
#
# It shells out to `git`, which product code may never do. That distinction is deliberate:
# `crates/nerve-cli/tests/no_subprocess.rs` forbids process creation in Nerve's Rust product
# code, and Nerve's own history reader is the independent implementation in
# `crates/nerve-index/src/gitobj/`. This is a measurement harness establishing ground truth
# about repository shape, in the same position as `scripts/make_gitobj_fixtures.sh`.
#
# Usage:
#   scripts/measure_history_storage.sh [REPO_PATH ...]
#
# With no argument it measures this repository.

set -euo pipefail

if [ "$#" -eq 0 ]; then
    set -- "$(git rev-parse --show-toplevel)"
fi

printf '%-28s %10s %10s %14s %14s %9s\n' \
    REPOSITORY COMMITS FILES SNAPSHOT-ROWS DELTA-ROWS RATIO
printf '%-28s %10s %10s %14s %14s %9s\n' \
    ---------------------------- ---------- ---------- -------------- -------------- ---------

for repo in "$@"; do
    if [ ! -d "$repo/.git" ] && ! git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
        printf '%-28s %s\n' "$(basename "$repo")" "not a git repository -- skipped"
        continue
    fi

    if ! git -C "$repo" rev-parse --verify HEAD >/dev/null 2>&1; then
        printf '%-28s %s\n' "$(basename "$repo")" "no commits on HEAD -- skipped"
        continue
    fi

    commits=$(git -C "$repo" rev-list --count HEAD)
    files=$(git -C "$repo" ls-tree -r --name-only HEAD | wc -l | tr -d ' ')

    # One `git ls-tree` per commit is deliberately naive. It is what the snapshot design would
    # have to store, counted honestly, rather than an estimate derived from HEAD's tree size --
    # which would overstate early history in a repository that grew.
    snapshot=$(git -C "$repo" rev-list HEAD | while read -r commit; do
        git -C "$repo" ls-tree -r --name-only "$commit" | wc -l
    done | awk '{ total += $1 } END { print total + 0 }')

    # `diff-tree` with no `-m` emits *nothing* for a merge commit -- not a first-parent diff, as
    # an earlier version of this comment claimed. That is measured, not assumed, and it is kept
    # deliberately because it is exactly what the chosen design does: Slice 12b enumerates no
    # changes for a merge, because a diff is only defined against one parent and attributing a
    # merge's changes to the first parent double-counts every change the branch already recorded.
    # So this column measures the design as specified.
    #
    # It does mean the delta figure omits merges, which flatters it slightly. Nerve has no merge
    # commits at all, so its ratio is unaffected; on a repository with 55 merges in 1,214 commits
    # the correction was about 5%, against a ratio of 177x. Report both if it ever matters.
    #
    # A root commit reports its whole tree, which is correct: against the empty tree every path in
    # a root commit really is an addition.
    delta=$(git -C "$repo" rev-list HEAD | while read -r commit; do
        git -C "$repo" diff-tree -r --no-commit-id --name-only "$commit" 2>/dev/null | wc -l
    done | awk '{ total += $1 } END { print total + 0 }')

    if [ "$delta" -gt 0 ]; then
        ratio=$(awk -v s="$snapshot" -v d="$delta" 'BEGIN { printf "%.1fx", s / d }')
    else
        ratio="n/a"
    fi

    # Only the basename is printed. An absolute path would put the operator's directory
    # layout into any transcript this measurement is pasted into.
    printf '%-28s %10s %10s %14s %14s %9s\n' \
        "$(basename "$repo")" "$commits" "$files" "$snapshot" "$delta" "$ratio"
done

cat <<'NOTE'

  snapshot-rows  what a per-commit membership snapshot would store
  delta-rows     what storing only each commit's changes stores
  ratio          snapshot / delta -- grows with history depth, so compare repositories of
                 different ages rather than trusting a single number

  Slice 12b chose the delta design on this evidence. See
  docs/plans/slice-12b-historical-model.md section 4.
NOTE
