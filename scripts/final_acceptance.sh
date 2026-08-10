#!/usr/bin/env bash
#
# Nerve final acceptance. Run from the repository root:
#
#     scripts/final_acceptance.sh
#
# Exits 0 only if every check below passed. Prints one line per check, and a summary that
# distinguishes four outcomes that are routinely conflated:
#
#   PASS      the check ran and succeeded
#   FAIL      the check ran and failed
#   REFUSED   a command does not exist *by decision*, with the decision named. Not a gap.
#   NOT BUILT a command does not exist yet because its slice is not done. A gap, and named as one.
#   SKIPPED   the check could not run here, with the reason. Never counted as a pass.
#
# The REFUSED row exists because the alternative is worse. `nerve affected` is absent because
# ADR-0008 concluded LCOV carries no per-test attribution, so "which tests would my change affect?"
# cannot be answered from coverage evidence; `nerve trace-tests` is absent because Nerve must not run
# a repository's test runner. A script that listed those as missing features would be reporting two
# deliberate security and evidence decisions as incompleteness, and would create pressure to "fix"
# them.
#
# Indexing happens in a throwaway `git archive` checkout, not in your working tree, so this never
# touches an existing `.nerve/`.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
ROOT="$PWD"
export PATH="$HOME/.cargo/bin:$PATH"

PASS=0
FAIL=0
SKIP=0
FAILED_CHECKS=()

green() { printf '\033[32m%s\033[0m' "$1"; }
red()   { printf '\033[31m%s\033[0m' "$1"; }
dim()   { printf '\033[2m%s\033[0m' "$1"; }

check() {
  local label="$1"; shift
  if "$@" >/tmp/nerve_acceptance_out 2>&1; then
    printf '  [%s] %s\n' "$(green PASS)" "$label"
    PASS=$((PASS + 1))
  else
    printf '  [%s] %s\n' "$(red FAIL)" "$label"
    sed 's/^/        /' /tmp/nerve_acceptance_out | tail -15
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("$label")
  fi
}

skip() {
  printf '  [%s] %s — %s\n' "$(dim SKIPPED)" "$1" "$2"
  SKIP=$((SKIP + 1))
}

note() { printf '  [%s] %s\n' "$(dim "$1")" "$2"; }

section() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# ---------------------------------------------------------------------------------------------
section "1. Verification gate"

check "cargo fmt --all -- --check" cargo fmt --all -- --check
check "cargo clippy --workspace --all-targets -- -D warnings" \
  cargo clippy --workspace --all-targets -- -D warnings
check "cargo build --release" cargo build --release

# --no-fail-fast is not optional. Measured on this project in Slice 7b: the default reported 3
# failures where there were 16, because the first failing target stopped the run.
printf '  ... running the full suite (this takes minutes)\n'
if cargo test --workspace --no-fail-fast >/tmp/nerve_acceptance_tests 2>&1; then
  TOTALS=$(grep -E '^test result' /tmp/nerve_acceptance_tests \
    | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}')
  printf '  [%s] cargo test --workspace --no-fail-fast — %s\n' "$(green PASS)" "$TOTALS"
  PASS=$((PASS + 1))
else
  printf '  [%s] cargo test --workspace --no-fail-fast\n' "$(red FAIL)"
  grep -E '^test result|^failures:|panicked' /tmp/nerve_acceptance_tests | tail -25 | sed 's/^/        /'
  FAIL=$((FAIL + 1))
  FAILED_CHECKS+=("cargo test")
fi

NERVE="$ROOT/target/release/nerve"

# ---------------------------------------------------------------------------------------------
section "2. Security invariants — the ones the architecture is built on"

check "no repository code is executed during indexing (no_subprocess)" \
  cargo test -p nerve-cli --test no_subprocess
check "no outbound network client in product source (no_network)" \
  cargo test -p nerve-cli --test no_network

# A trace producer lives in this repository. No Rust source may name it, because product code that
# knows the tracer's name is one step from knowing how to launch it.
if [ -d "$ROOT/tracers" ]; then
  check "no Rust source references the trace producer" \
    bash -c '! grep -rqn --include="*.rs" "nerve_trace\|tracers/" crates/*/src/'
else
  skip "no Rust source references the trace producer" "tracers/ does not exist yet (Slice 11b)"
fi

# ---------------------------------------------------------------------------------------------
section "3. Command surface"

for command in init index coverage trace status check doctor search gaps impact path serve mcp why repo; do
  check "nerve $command exists" bash -c "'$NERVE' help $command >/dev/null 2>&1"
done

# Absent by decision. Each names the decision, because an unexplained absence reads as an oversight.
refused() {
  if "$NERVE" help "$1" >/dev/null 2>&1; then
    printf '  [%s] nerve %s exists, but %s\n' "$(red FAIL)" "$1" "$2"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("nerve $1 should not exist")
  else
    printf '  [%s] nerve %s — %s\n' "$(dim REFUSED)" "$1" "$2"
  fi
}
refused affected \
  "ADR-0008: LCOV carries no per-test attribution, so this is unanswerable from coverage evidence"
refused trace-tests \
  "Nerve must not run a repository's test runner (THREAT-MODEL T1, no_subprocess.rs)"

# Absent because unbuilt. A gap, and named as one rather than blended into the row above.
#
# `history` left this list in Slice 12b. It is now exercised for real in section 4b, which is the
# update the old row was asking for: while it sat here, the script printed
# `PASS — nerve history exists — update this script`, which counted as a pass while checking nothing
# about what the command does. A placeholder that scores is worse than a gap that does not.
for pair in "memory:Slice 14 — human-confirmed memory"; do
  command="${pair%%:*}"; reason="${pair#*:}"
  if "$NERVE" help "$command" >/dev/null 2>&1; then
    printf '  [%s] nerve %s exists — update this script\n' "$(green PASS)" "$command"
    PASS=$((PASS + 1))
  else
    printf '  [%s] nerve %s — %s\n' "$(dim "NOT BUILT")" "$command" "$reason"
  fi
done

# ---------------------------------------------------------------------------------------------
section "4. End to end, on a clean checkout of this repository"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
if git -C "$ROOT" archive HEAD | tar -x -C "$WORK" 2>/dev/null; then
  check "nerve init"   bash -c "cd '$WORK' && '$NERVE' init"
  check "nerve index"  bash -c "cd '$WORK' && '$NERVE' index"
  check "nerve status" bash -c "cd '$WORK' && '$NERVE' status"
  check "nerve doctor" bash -c "cd '$WORK' && '$NERVE' doctor"
  # The selectors below are **TypeScript**, and that is not incidental. The first version of this
  # script queried `parse_trace`, a Rust function, and both queries correctly refused: Nerve indexes
  # TypeScript, JavaScript and Python, and does not index Rust. Nerve's own Rust source is therefore
  # not a usable self-test subject for a symbol query, and `apps/nerve-web` is what makes this
  # repository able to index itself at all.
  check "nerve search" bash -c "cd '$WORK' && '$NERVE' search relationPhrase"
  check "nerve gaps"   bash -c "cd '$WORK' && '$NERVE' gaps"
  check "nerve impact" bash -c "cd '$WORK' && '$NERVE' impact relationPhrase"
  check "nerve why"    bash -c "cd '$WORK' && '$NERVE' why relationPhrase"
  check "nerve path"   bash -c "cd '$WORK' && '$NERVE' path relationPhrase sourceTypeGloss"
  # `check` exits non-zero by design when an index has honest problems, so its *exit code* is not
  # the assertion — that it produces a verdict at all is.
  check "nerve check produces a verdict" \
    bash -c "cd '$WORK' && '$NERVE' check 2>&1 | grep -qiE 'trust|stale|fresh|verdict|index'"

  # A read-only surface must not mutate. Byte-compare the database around a query.
  check "read-only queries do not mutate the database" bash -c "
    cd '$WORK'
    before=\$(shasum -a 256 .nerve/nerve.db | cut -d' ' -f1)
    '$NERVE' search parse_trace >/dev/null 2>&1
    '$NERVE' impact parse_trace >/dev/null 2>&1
    '$NERVE' gaps >/dev/null 2>&1
    after=\$(shasum -a 256 .nerve/nerve.db | cut -d' ' -f1)
    [ \"\$before\" = \"\$after\" ]"
else
  skip "end-to-end on a clean checkout" "git archive HEAD failed"
fi

# ---------------------------------------------------------------------------------------------
section "4b. Git history, and the sentence it must never print"

# The committed fixtures are used rather than this repository's own `.git`, for two reasons: a
# shallow checkout cannot be produced from the repository under test without network, and the
# fixtures carry an `inventory.json` whose values are Git's own answers. Each stores its git
# directory under the plain name `gitdir/`, because Git will not track a nested `.git`.
history_repo() {
  local fixture="$1" dest="$2"
  cp -R "$ROOT/fixtures/$fixture" "$dest" || return 1
  mv "$dest/gitdir" "$dest/.git" || return 1
  "$NERVE" init "$dest" >/dev/null 2>&1
}

if [ -d "$ROOT/fixtures/history-basic" ] && [ -d "$ROOT/fixtures/history-shallow" ]; then
  HWORK=$(mktemp -d)
  trap 'rm -rf "$WORK" "$HWORK"' EXIT

  check "nerve history sync reads a repository's own commits" bash -c "
    $(declare -f history_repo)
    ROOT='$ROOT' NERVE='$NERVE'
    history_repo history-basic '$HWORK/basic' &&
    '$NERVE' history sync '$HWORK/basic' >/dev/null &&
    '$NERVE' history log --path '$HWORK/basic' --json | grep -q '\"answerable\": *true'"

  check "nerve history log counts what Git counted" bash -c "
    declared=\$(python3 -c \"import json;print(len(json.load(open('$ROOT/fixtures/history-basic/inventory.json'))['commits']))\")
    got=\$('$NERVE' history log --path '$HWORK/basic' --json | python3 -c 'import json,sys;print(json.load(sys.stdin)[\"totals\"][\"commits\"])')
    [ \"\$declared\" = \"\$got\" ]"

  # A path that no longer exists must still have a history. Under a guard that canonicalized — which
  # is what `discover::canonical_child` does — every deleted path would have been refused, and the
  # refusal counted as a path-safety success. This is the check that would have caught that.
  check "a deleted path still has a history" bash -c "
    '$NERVE' history file src/app/util.rs --path '$HWORK/basic' --json |
      python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d[\"count\"]>0 and any(c[\"change\"][\"change_kind\"]==\"deleted\" for c in d[\"commits\"]) else 1)'"

  check "history reads leave the database byte-identical" bash -c "
    before=\$(shasum -a 256 '$HWORK/basic/.nerve/nerve.db' | cut -d' ' -f1)
    '$NERVE' history log --path '$HWORK/basic' >/dev/null 2>&1
    '$NERVE' history file README.md --path '$HWORK/basic' >/dev/null 2>&1
    after=\$(shasum -a 256 '$HWORK/basic/.nerve/nerve.db' | cut -d' ' -f1)
    [ \"\$before\" = \"\$after\" ]"

  # **The product assertion of row 12b, on the shipped binary.** A shallow boundary means "history
  # before this point is unavailable to this repository", never "the project's history begins here".
  # Diffing a boundary against the empty tree would report every file in it as newly added — the
  # claim, stated as data. Both directions are checked: the boundary must be *named* (or a command
  # that printed nothing would pass) and the forbidden phrasing must be absent.
  check "a shallow boundary is never described as the start of history" bash -c "
    $(declare -f history_repo)
    ROOT='$ROOT' NERVE='$NERVE'
    history_repo history-shallow '$HWORK/shallow' || exit 1
    '$NERVE' history sync '$HWORK/shallow' >/dev/null || exit 1
    boundary=\$(python3 -c \"import json;print(json.load(open('$ROOT/fixtures/history-shallow/inventory.json'))['shallow']['boundary_oids'][0])\")
    out=\$('$NERVE' history log --path '$HWORK/shallow')
    printf '%s' \"\$out\" | grep -q \"\$boundary\" || exit 1
    printf '%s' \"\$out\" | grep -q 'unavailable to this repository' || exit 1
    ! printf '%s' \"\$out\" | grep -qiE \"history begins here|first commit in project|beginning of repository history\""

  check "a shallow boundary enumerates no changes, and says why" bash -c "
    '$NERVE' history log --path '$HWORK/shallow' --json | python3 -c '
import json,sys
rows = json.load(sys.stdin)[\"commits\"]
b = [r for r in rows if r[\"parent_completeness\"] == \"shallow_boundary\"]
sys.exit(0 if len(b) == 1
         and b[0][\"changes\"] == 0
         and b[0][\"changes_enumerated\"] == \"parent_unavailable\"
         and b[0][\"may_claim_history_begins_here\"] is False
         and len(b[0][\"parent_oids\"]) == 1
         else 1)'"

  # Nerve stopping is not the repository ending. A different reason, and it must read differently.
  check "a bounded ingest is a different reason from a shallow boundary" bash -c "
    $(declare -f history_repo)
    ROOT='$ROOT' NERVE='$NERVE'
    history_repo history-basic '$HWORK/bounded' || exit 1
    '$NERVE' history sync '$HWORK/bounded' --max-commits 1 --json |
      python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d[\"walk_terminated_by\"]==\"commit_budget\" and d[\"shallow\"] is False else 1)'"

  check "an over-large --max-commits is refused with the clamp stated" bash -c "
    out=\$('$NERVE' history sync '$HWORK/bounded' --max-commits 999999 2>&1); rc=\$?
    [ \$rc -eq 10 ] && printf '%s' \"\$out\" | grep -q 5000"

  # ---- Slice 12c-ii: similarity evidence, on the shipped binary -----------------------------
  #
  # Every check below exercises behaviour rather than existence. The numbers are read out of
  # `fixtures/history-similar/ground_truth.json`, which is hand-written, predates the matcher, and
  # is never produced by running Nerve — so a check that passed because the matcher agreed with
  # itself would not pass here. The one thing a similarity hypothesis must never be is a
  # percentage with no method, no version and no threshold beside it.
  if [ -d "$ROOT/fixtures/history-similar" ]; then
    history_repo history-similar "$HWORK/similar" >/dev/null 2>&1
    SIMILAR_SYNC=$("$NERVE" history sync "$HWORK/similar" 2>&1)
    printf '%s' "$SIMILAR_SYNC" > /tmp/nerve_acceptance_similar_sync

    check "a similarity hypothesis reports its matcher, its measurement and its threshold" bash -c "
      '$NERVE' history file mod/alpha-renamed.txt --path '$HWORK/similar' --json |
      python3 -c '
import json,sys
truth = json.load(open(\"$ROOT/fixtures/history-similar/ground_truth.json\"))
matcher = truth[\"matcher\"]
pair = next(p for p in truth[\"similar_content_pairs\"]
            if p[\"to_path\"] == \"mod/alpha-renamed.txt\" and p[\"admitted\"])
rows = [r for r in json.load(sys.stdin)[\"renames\"] if r[\"evidence\"] == \"similar_content\"]
assert len(rows) == 1, rows
r = rows[0]
assert r[\"matcher_id\"] == matcher[\"id\"], r
assert r[\"matcher_version\"] == matcher[\"version\"], r
assert r[\"match_numerator\"] == pair[\"numerator\"], r
assert r[\"match_denominator\"] == pair[\"denominator\"], r
assert isinstance(r[\"match_numerator\"], int) and isinstance(r[\"match_denominator\"], int), r
a = r[\"analysis\"]
assert a[\"threshold_numerator\"] == matcher[\"threshold_numerator\"], a
assert a[\"threshold_denominator\"] == matcher[\"threshold_denominator\"], a
assert a[\"completeness\"] == \"complete\", a
assert r[\"from_blob_oid\"] != r[\"to_blob_oid\"], r
'"

    # A hypothesis is a proposal. Both halves are checked: the label must be present (or a command
    # that printed nothing would pass) and no affirmative phrasing may appear anywhere.
    check "a similarity hypothesis is labelled a hypothesis and never a confirmed rename" bash -c "
      out=\$('$NERVE' history file mod/alpha-renamed.txt --path '$HWORK/similar')
      printf '%s' \"\$out\" | grep -q 'similar_content' || exit 1
      printf '%s' \"\$out\" | grep -q 'rename hypothesis — Git recorded no rename' || exit 1
      printf '%s' \"\$out\" | grep -qE '[0-9]+ of [0-9]+ line\(s\) shared' || exit 1
      printf '%s' \"\$out\" | grep -qE 'threshold +[0-9]+ of [0-9]+' || exit 1
      ! printf '%s' \"\$out\" | grep -qiE 'confirmed rename|was renamed to|git renamed|rename recorded by git'"

    # Two kinds of evidence, two counts, and no line anywhere that adds them together.
    check "exact and similar rename counts are reported separately and never blended" bash -c "
      '$NERVE' history sync '$HWORK/similar' --json >/dev/null 2>&1
      python3 -c '
import json,re,sys
truth = json.load(open(\"$ROOT/fixtures/history-similar/ground_truth.json\"))
exact = truth[\"totals\"][\"exact_content_pairs\"]
similar = truth[\"totals\"][\"admitted\"]
text = open(\"/tmp/nerve_acceptance_similar_sync\").read()
assert \"%d exact-content hypothesis\" % exact in text, text
assert \"%d similar-content hypothesis\" % similar in text, text
blended = exact + similar
assert not re.search(r\"renames +%d hypothes\" % blended, text), text
assert \"never added to the line above\" in text, text
'"

    # A commit that could measure nothing is the case a per-row flag cannot state, so the commit
    # carries it. Reporting it as complete would be the quiet failure the analysis table exists for.
    check "an unmeasurable candidate pair is named and never reported as complete" bash -c "
      out=\$('$NERVE' history file bin/other.bin --path '$HWORK/similar')
      printf '%s' \"\$out\" | grep -q 'candidates     partial' || exit 1
      printf '%s' \"\$out\" | grep -q 'unmeasured     blob-binary' || exit 1
      ! printf '%s' \"\$out\" | grep -q 'candidates     complete'"

    check "similarity reads leave the database byte-identical" bash -c "
      before=\$(shasum -a 256 '$HWORK/similar/.nerve/nerve.db' | cut -d' ' -f1)
      '$NERVE' history file mod/alpha-renamed.txt --path '$HWORK/similar' >/dev/null 2>&1
      '$NERVE' history file bin/other.bin --path '$HWORK/similar' --json >/dev/null 2>&1
      '$NERVE' history log --path '$HWORK/similar' >/dev/null 2>&1
      after=\$(shasum -a 256 '$HWORK/similar/.nerve/nerve.db' | cut -d' ' -f1)
      [ \"\$before\" = \"\$after\" ]"
  else
    skip "similarity renames" "fixtures/history-similar is missing"
  fi

  # A summary is bounded at 512 bytes and the repository-level tally cannot say *which* one was
  # cut. `fixtures/history-hostile` carries a 600-byte single-line summary, so this exercises the
  # `truncated` value rather than asserting it — and every other commit in the same answer has to
  # carry the flag too, or its absence becomes the claim "nothing was cut".
  if [ -d "$ROOT/fixtures/history-hostile" ]; then
    history_repo history-hostile "$HWORK/hostile" >/dev/null 2>&1
    "$NERVE" history sync "$HWORK/hostile" >/dev/null 2>&1

    check "no summary is rendered without saying whether it was cut" bash -c "
      '$NERVE' history log --path '$HWORK/hostile' --limit 100 --json |
      python3 -c '
import json,sys
inv = json.load(open(\"$ROOT/fixtures/history-hostile/inventory.json\"))
cut = inv[\"attacks\"][\"summary-over-512-bytes\"][\"commit_oid\"]
assert inv[\"attacks\"][\"summary-over-512-bytes\"][\"summary_bytes\"] > 512
rows = json.load(sys.stdin)[\"commits\"]
assert len(rows) > 1, rows
for r in rows:
    assert isinstance(r[\"summary\"], str), r
    assert r[\"summary_truncation\"] in (\"complete\", \"truncated\", \"unknown\"), r
    assert isinstance(r[\"summary_truncation_note\"], str), r
row = next(r for r in rows if r[\"commit_oid\"] == cut)
assert row[\"summary_truncation\"] == \"truncated\", row
assert len(row[\"summary\"]) == 512, len(row[\"summary\"])
'"

    check "the human surface prints one truncation flag per printed summary" bash -c "
      out=\$('$NERVE' history log --path '$HWORK/hostile' --limit 100)
      s=\$(printf '%s\n' \"\$out\" | grep -c '^  summary        ')
      f=\$(printf '%s\n' \"\$out\" | grep -c '^  summary_state  ')
      [ \"\$s\" -gt 1 ] && [ \"\$s\" = \"\$f\" ] &&
        printf '%s' \"\$out\" | grep -q 'summary_state  truncated'"
  else
    skip "per-summary truncation" "fixtures/history-hostile is missing"
  fi
else
  skip "git history" "fixtures/history-* are missing"
fi

# ---------------------------------------------------------------------------------------------
section "4c. The cross-repository registry, and the second repository it reads"

# Every check here exercises behaviour. `nerve repo exists` is already covered by section 3 and is
# worth nothing on its own: the questions that matter are whether a *real* second checkout is
# registered and read, whether a retired entry is still listed, whether a relocation onto the wrong
# repository is refused, and whether the neighbour's bytes survive all of it.
#
# Three separate fixture copies, each `nerve init`-ed on its own, so each has its own `project_id`
# and therefore its own `repo_id`. Two copies sharing an identity would make the relocate refusal
# pass for the wrong reason.
if [ -d "$ROOT/fixtures/ts-basic" ] && [ -d "$ROOT/fixtures/ts-resolution" ]; then
  RWORK=$(mktemp -d)
  trap 'rm -rf "$WORK" "$HWORK" "$RWORK"' EXIT

  registry_repo() {
    local fixture="$1" dest="$2" index="$3"
    cp -R "$ROOT/fixtures/$fixture" "$dest" || return 1
    rm -rf "$dest/.nerve"
    "$NERVE" init "$dest" >/dev/null 2>&1 || return 1
    [ "$index" = "index" ] && { "$NERVE" index "$dest" >/dev/null 2>&1 || return 1; }
    return 0
  }

  if registry_repo ts-basic "$RWORK/a" index &&
     registry_repo ts-resolution "$RWORK/b" index &&
     registry_repo ts-resolution "$RWORK/other" index &&
     registry_repo ts-resolution "$RWORK/sibling" index; then

    # 1. Nothing is discovered. Three checkouts sit beside `a` and none of them is registered.
    check "no sibling checkout is registered without being named" bash -c "
      '$NERVE' repo list --path '$RWORK/a' --json |
      python3 -c 'import json,sys; d=json.load(sys.stdin); assert d[\"entries\"] == [], d'"

    # 2. A real second repository is registered, and read.
    check "a second repository is registered and reported available" bash -c "
      '$NERVE' repo add '$RWORK/b' --path '$RWORK/a' --id neighbour --json |
      python3 -c '
import json,sys
e = json.load(sys.stdin)[\"entries\"][0]
assert e[\"registry_id\"] == \"neighbour\", e
assert e[\"availability\"] == \"available\", e
assert e[\"freshness\"] is None, e
assert e[\"expected_repository_id\"].startswith(\"repo_\"), e
assert e[\"local_path\"].endswith(\"/b\"), e
'"

    check "the registered neighbour is listed" bash -c "
      '$NERVE' repo list --path '$RWORK/a' --json |
      python3 -c '
import json,sys
rows = json.load(sys.stdin)[\"entries\"]
assert len(rows) == 1, rows
assert rows[0][\"registry_id\"] == \"neighbour\", rows
assert rows[0][\"status\"] == \"active\", rows
'"

    # 3. Relocation onto a different repository is refused, with the reason named, and the entry
    #    does not move. Without this check, relocation is the silent re-pointing the row plan calls
    #    the dangerous case, performed by Nerve on request.
    check "relocating onto a different repository is refused by identity" bash -c "
      out=\$('$NERVE' repo relocate neighbour '$RWORK/other' --path '$RWORK/a' --json)
      status=\$?
      [ \"\$status\" = 10 ] || { printf '%s\n' \"\$out\"; exit 1; }
      printf '%s' \"\$out\" | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"ok\"] is False, d
assert d[\"refusal\"] == \"target_repository_moved\", d
'
      '$NERVE' repo list --path '$RWORK/a' --json | python3 -c '
import json,sys
row = json.load(sys.stdin)[\"entries\"][0]
assert row[\"local_path\"].endswith(\"/b\"), row
'"

    # 4. Removal is a tombstone. The entry is still listed, marked, and carries the one freshness
    #    state that only a kept row can report.
    check "a retired entry is still listed, and says so" bash -c "
      '$NERVE' repo remove neighbour --path '$RWORK/a' >/dev/null &&
      '$NERVE' repo list --path '$RWORK/a' --json | python3 -c '
import json,sys
rows = json.load(sys.stdin)[\"entries\"]
assert len(rows) == 1, rows
assert rows[0][\"status\"] == \"tombstoned\", rows
assert rows[0][\"availability\"] == \"entry_removed\", rows
assert rows[0][\"freshness\"] == \"registry_entry_removed\", rows
assert rows[0][\"withdrawn_at\"], rows
'"

    # 5. The neighbour's database is byte-identical after every one of those reads. A *different*
    #    neighbour, because the first one is now a tombstone and a tombstoned entry opens nothing —
    #    hashing a file nobody read would be the vacuous version of this check.
    check "the neighbour's database is byte-identical after every read" bash -c "
      before=\$(shasum -a 256 '$RWORK/other/.nerve/nerve.db' | cut -d' ' -f1)
      '$NERVE' repo add '$RWORK/other' --path '$RWORK/a' --id again >/dev/null || exit 1
      '$NERVE' repo list --path '$RWORK/a' >/dev/null
      '$NERVE' repo list --path '$RWORK/a' --json >/dev/null
      '$NERVE' repo relocate again '$RWORK/other' --path '$RWORK/a' >/dev/null || exit 1
      after=\$(shasum -a 256 '$RWORK/other/.nerve/nerve.db' | cut -d' ' -f1)
      [ \"\$before\" = \"\$after\" ] || { echo \"\$before != \$after\"; exit 1; }
      # Anti-vacuity: the reads really produced an answer, so 'unchanged' is not 'nothing ran'.
      '$NERVE' repo list --path '$RWORK/a' --json | python3 -c '
import json,sys
rows = {r[\"registry_id\"]: r for r in json.load(sys.stdin)[\"entries\"]}
assert set(rows) == {\"neighbour\", \"again\"}, rows
assert rows[\"again\"][\"availability\"] == \"available\", rows[\"again\"]
'"

    # 6. A registered path that no longer exists and one that now holds another repository are two
    #    different answers. Collapsing them is refutation 6 of the row plan.
    check "a missing neighbour and a swapped one stay distinct" bash -c "
      '$NERVE' repo add '$RWORK/sibling' --path '$RWORK/a' --id swapped >/dev/null || exit 1
      rm -rf '$RWORK/other'
      rm -rf '$RWORK/sibling'
      cp -R '$ROOT/fixtures/ts-basic' '$RWORK/sibling'
      rm -rf '$RWORK/sibling/.nerve'
      '$NERVE' init '$RWORK/sibling' >/dev/null 2>&1
      '$NERVE' index '$RWORK/sibling' >/dev/null 2>&1
      '$NERVE' repo list --path '$RWORK/a' --json | python3 -c '
import json,sys
rows = {r[\"registry_id\"]: r for r in json.load(sys.stdin)[\"entries\"]}
assert rows[\"again\"][\"freshness\"] == \"target_repository_missing\", rows[\"again\"]
assert rows[\"again\"][\"refusal\"] == \"path_does_not_exist\", rows[\"again\"]
assert rows[\"swapped\"][\"freshness\"] == \"target_repository_moved\", rows[\"swapped\"]
assert rows[\"swapped\"][\"observed_repository_id\"], rows[\"swapped\"]
assert rows[\"swapped\"][\"observed_repository_id\"] != rows[\"swapped\"][\"expected_repository_id\"]
assert rows[\"again\"][\"freshness\"] != rows[\"swapped\"][\"freshness\"]
'"

    # 7. Every refusal names itself rather than falling back to a narrower answer.
    check "every registration refusal names its own reason" bash -c "
      seen=''
      for target in '$RWORK/nowhere' '$RWORK/a'; do
        out=\$('$NERVE' repo add \"\$target\" --path '$RWORK/a' --json)
        [ \$? = 10 ] || { printf '%s\n' \"\$out\"; exit 1; }
        seen=\"\$seen \$(printf '%s' \"\$out\" | python3 -c 'import json,sys; print(json.load(sys.stdin)[\"refusal\"])')\"
      done
      printf '%s' \"\$seen\" | grep -q path_does_not_exist &&
      printf '%s' \"\$seen\" | grep -q same_repository"
  else
    skip "cross-repository registry" "the fixture repositories could not be built"
  fi
else
  skip "cross-repository registry" "fixtures/ts-basic or fixtures/ts-resolution is missing"
fi

# ---------------------------------------------------------------------------------------------
section "4d. The two contract rules, and the links they refuse to invent"

# Behaviour, not surface. Every check below runs the real extractor over a real fixture pair and
# asks a question the answer to which would be wrong if the rule were guessing: does an explicit
# `file:` declaration produce a link with the right resolution_method, does a specifier Nerve does
# not read get *named* rather than dropped, do two same-named packages that declare nothing about
# each other produce nothing at all, and is the neighbour's database identical afterwards.
#
# The two rules are exercised separately and their numbers are never added together.
if [ -d "$ROOT/fixtures/contracts-npm" ] && [ -d "$ROOT/fixtures/contracts-python" ]; then
  CWORK=$(mktemp -d)
  trap 'rm -rf "${WORK:-}" "${HWORK:-}" "${RWORK:-}" "${CWORK:-}"' EXIT

  contract_repo() {
    local fixture="$1" name="$2" dest="$3"
    cp -R "$ROOT/fixtures/$fixture/$name" "$dest" || return 1
    rm -rf "$dest/.nerve"
    "$NERVE" init "$dest" >/dev/null 2>&1 || return 1
    "$NERVE" index "$dest" >/dev/null 2>&1 || return 1
    return 0
  }

  if contract_repo contracts-npm app "$CWORK/app" &&
     contract_repo contracts-npm lib-core "$CWORK/lib-core" &&
     contract_repo contracts-npm lib-extra "$CWORK/lib-extra" &&
     contract_repo contracts-npm unregistered "$CWORK/unregistered" &&
     "$NERVE" repo add "$CWORK/lib-core" --path "$CWORK/app" --id lib-core >/dev/null 2>&1 &&
     "$NERVE" repo add "$CWORK/lib-extra" --path "$CWORK/app" --id lib-extra >/dev/null 2>&1; then

    # 1. C1: the declared links, each with the resolution_method its form implies. `file:` is a
    #    path the manifest states; `workspace:` is a path the workspaces array states. Two stated
    #    declarations, two different methods, and the response says which.
    check "C1 records the declared npm links with the right resolution_method" bash -c "
      '$NERVE' repo scan --path '$CWORK/app' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
links = {(l[\"section\"], l[\"identity\"]): l for l in d[\"links\"]}
assert d[\"links_recorded\"] == 5, d
assert links[(\"dependencies\",\"lib-core\")][\"registry_id\"] == \"lib-core\", links
assert links[(\"dependencies\",\"lib-core\")][\"resolution_method\"] == \"manifest_declared\", links
assert links[(\"dependencies\",\"lib-extra\")][\"registry_id\"] == \"lib-extra\", links
assert links[(\"dependencies\",\"lib-extra\")][\"resolution_method\"] == \"workspace_declared\", links
assert links[(\"peerDependencies\",\"lib-peer\")][\"registry_id\"] == \"lib-extra\", links
# One identity declared twice, naming two repositories. Both recorded, neither promoted.
assert links[(\"dependencies\",\"ambiguous-dep\")][\"ambiguity\"] == \"conflicting_targets\", links
assert links[(\"devDependencies\",\"ambiguous-dep\")][\"ambiguity\"] == \"conflicting_targets\", links
assert links[(\"dependencies\",\"ambiguous-dep\")][\"registry_id\"] != links[(\"devDependencies\",\"ambiguous-dep\")][\"registry_id\"]
'"

    # 2. §9.1: an unsupported specifier is recorded with its form named, asserted by a tally.
    #    A registry range, a git specifier, a URL and an alias are four different refusals and the
    #    response says which is which — never one \"could not read that\".
    check "an unsupported specifier is recorded with its form named, never dropped" bash -c "
      '$NERVE' repo scan --path '$CWORK/app' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
tally = {row[\"form\"]: row[\"count\"] for row in d[\"unsupported_tally\"]}
for form in (\"npm_registry_range\",\"npm_git_specifier\",\"npm_url_specifier\",
             \"npm_alias_specifier\",\"npm_unsupported_protocol\",\"npm_workspace_glob_pattern\"):
    assert tally.get(form, 0) >= 1, (form, tally)
assert sum(tally.values()) == len(d[\"unsupported\"]) == 9, (tally, d[\"unsupported\"])
# Every declaration is accounted for: nothing was read and then forgotten.
assert d[\"declarations\"] == len(d[\"links\"]) + len(d[\"unresolved\"]) + len(d[\"unsupported\"])
'"

    # 3. An explicit path to a real, indexed, adjacent repository that nobody registered produces
    #    no link and registers nothing. This is the refusal the whole row is built on.
    check "an unregistered neighbour is named as unresolved and never auto-registered" bash -c "
      '$NERVE' repo scan --path '$CWORK/app' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
reasons = {r[\"identity\"]: r[\"reason\"] for r in d[\"unresolved\"]}
assert reasons[\"lib-unregistered\"] == \"target_not_registered\", reasons
assert reasons[\"lib-missing\"] == \"declared_path_missing\", reasons
assert reasons[\"lib-inside\"] == \"declared_path_in_same_repository\", reasons
assert not any(l[\"identity\"] == \"lib-unregistered\" for l in d[\"links\"]), d[\"links\"]
'
      '$NERVE' repo list --path '$CWORK/app' --json | python3 -c '
import json,sys
ids = sorted(r[\"registry_id\"] for r in json.load(sys.stdin)[\"entries\"])
assert ids == [\"lib-core\", \"lib-extra\"], ids
'"

    # 4. Re-running writes nothing new. The unique index on the logical identity is what makes a
    #    re-scan a no-op rather than a duplicate, and a table that grew on every run would be a
    #    registry that says a declaration was made twice.
    check "re-running the scan records nothing new and duplicates nothing" bash -c "
      '$NERVE' repo scan --path '$CWORK/app' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"links_recorded\"] == 0, d
assert d[\"links_unchanged\"] == 5, d
'
      python3 -c '
import sqlite3
rows = sqlite3.connect(\"$CWORK/app/.nerve/nerve.db\").execute(
    \"select count(*), count(distinct link_id) from contract_link\").fetchone()
assert rows == (5, 5), rows
'"

    # 5. The neighbour's database is byte-identical after extraction. Anti-vacuity first: the scan
    #    really produced links through that neighbour, so \"unchanged\" is not \"nothing ran\".
    check "the neighbour's database is byte-identical after extraction" bash -c "
      before=\$(shasum -a 256 '$CWORK/lib-core/.nerve/nerve.db' | cut -d' ' -f1)
      '$NERVE' repo scan --path '$CWORK/app' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert any(l[\"registry_id\"] == \"lib-core\" for l in d[\"links\"]), d[\"links\"]
'
      after=\$(shasum -a 256 '$CWORK/lib-core/.nerve/nerve.db' | cut -d' ' -f1)
      [ \"\$before\" = \"\$after\" ] || { echo \"\$before != \$after\"; exit 1; }"
  else
    skip "C1 npm contract extraction" "the fixture repositories could not be built"
  fi

  # 6. §9.7: fuzzy linking is asserted absent. Same package name on both sides, adjacent
  #    directories, a registered neighbour — and no declaration between them.
  if contract_repo contracts-fuzzy left "$CWORK/left" &&
     contract_repo contracts-fuzzy right "$CWORK/right" &&
     "$NERVE" repo add "$CWORK/right" --path "$CWORK/left" --id neighbour >/dev/null 2>&1; then
    check "same-named packages that declare nothing produce zero links" bash -c "
      '$NERVE' repo list --path '$CWORK/left' --json | python3 -c '
import json,sys
rows = json.load(sys.stdin)[\"entries\"]
assert len(rows) == 1 and rows[0][\"availability\"] == \"available\", rows
'
      '$NERVE' repo scan --path '$CWORK/left' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"manifests_read\"] == 2, d
assert d[\"links\"] == [], d[\"links\"]
assert d[\"links_recorded\"] == 0, d
'
      grep -q shared-name '$CWORK/left/package.json' &&
      grep -q shared-name '$CWORK/right/package.json'"
  else
    skip "fuzzy linking is absent" "the fuzzy fixture pair could not be built"
  fi

  # 6b. C2 — the one rule in this row that reaches a file entity inside the target. Three
  #     behaviours, each of which would be wrong if the rule were guessing: does an import
  #     specifier resolve through the neighbour's own `exports` to a real file entity over there,
  #     is every export shape Nerve declines *named* rather than dropped, and does an ordinary
  #     local traversal stay out of the link entirely.
  if contract_repo contracts-exports host "$CWORK/host" &&
     contract_repo contracts-exports pkg-map "$CWORK/pkg-map" &&
     contract_repo contracts-exports pkg-string "$CWORK/pkg-string" &&
     contract_repo contracts-exports pkg-legacy "$CWORK/pkg-legacy" &&
     contract_repo contracts-exports twin-a "$CWORK/twin-a" &&
     contract_repo contracts-exports twin-b "$CWORK/twin-b" &&
     contract_repo contracts-exports pkg-unregistered "$CWORK/pkg-unregistered" &&
     "$NERVE" repo add "$CWORK/pkg-map" --path "$CWORK/host" --id pkg-map >/dev/null 2>&1 &&
     "$NERVE" repo add "$CWORK/pkg-string" --path "$CWORK/host" --id pkg-string >/dev/null 2>&1 &&
     "$NERVE" repo add "$CWORK/pkg-legacy" --path "$CWORK/host" --id pkg-legacy >/dev/null 2>&1 &&
     "$NERVE" repo add "$CWORK/twin-a" --path "$CWORK/host" --id twin-a >/dev/null 2>&1 &&
     "$NERVE" repo add "$CWORK/twin-b" --path "$CWORK/host" --id twin-b >/dev/null 2>&1; then

    # 1. The entity-to-entity link, and the whole evidence chain behind it. The target entity id
    #    is a row in pkg-map's database and in no row of host's, which is what `contract_link`
    #    exists for: `assertion.target_entity_id` is a foreign key and would have refused it.
    check "C2 resolves an import specifier to a file entity in the neighbour" bash -c "
      '$NERVE' repo scan --path '$CWORK/host' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
c2 = {l[\"identity\"]: l for l in d[\"links\"] if l[\"rule\"] == \"npm_export_resolution\"}
assert len(c2) == 7, sorted(c2)
sub = c2[\"pkg-map/sub\"]
assert sub[\"resolution_method\"] == \"export_map_resolved\", sub
assert sub[\"relation_semantics\"] == \"REFERENCES\", sub
assert sub[\"registry_id\"] == \"pkg-map\", sub
assert sub[\"target_path\"] == \"src/sub.ts\", sub
assert sub[\"source_entity_id\"], sub
assert sub[\"target_entity_id\"], sub
# The condition order is documented and taken: import beats require.
assert c2[\"pkg-map/cond\"][\"target_path\"] == \"src/esm.ts\", c2[\"pkg-map/cond\"]
# The legacy order is documented and taken: module beats main.
assert c2[\"pkg-legacy\"][\"form\"] == \"npm_legacy_module\", c2[\"pkg-legacy\"]
assert c2[\"pkg-legacy\"][\"target_path\"] == \"src/mod.ts\", c2[\"pkg-legacy\"]
# A file the neighbour has and never indexed: a path, no entity. Not a missing target.
assert c2[\"pkg-map/data\"][\"target_path\"] == \"src/data.json\", c2[\"pkg-map/data\"]
assert c2[\"pkg-map/data\"][\"target_entity_id\"] is None, c2[\"pkg-map/data\"]
# An unregistered neighbour is imported by name and still produces nothing.
reasons = {r[\"identity\"]: r[\"reason\"] for r in d[\"unresolved\"]}
assert reasons[\"pkg-unregistered\"] == \"target_not_registered\", reasons
assert reasons[\"pkg-aliased/thing\"] == \"package_name_not_declared\", reasons
assert reasons[\"pkg-map/gone\"] == \"export_target_missing\", reasons
'
      python3 -c '
import sqlite3
host = sqlite3.connect(\"$CWORK/host/.nerve/nerve.db\")
rows = host.execute(
    \"select target_entity_id, target_path_snapshot from contract_link \"
    \"where contract_kind = ?1 and target_entity_id is not null\", (\"npm_export_resolution\",)
).fetchall()
assert len(rows) == 7, rows
for entity_id, path in rows:
    local = host.execute(\"select count(*) from entity where entity_id = ?1\", (entity_id,)).fetchone()[0]
    assert local == 0, (entity_id, path, \"a proxy entity was created for a foreign target\")
found = sqlite3.connect(\"$CWORK/pkg-map/.nerve/nerve.db\").execute(
    \"select count(*) from entity where entity_id = ?1\", (rows[0][0],)).fetchone()[0]
# Anti-vacuity: at least one of those ids really is a row over there.
ids = [r[0] for r in rows]
hit = 0
for db in (\"pkg-map\", \"pkg-string\", \"pkg-legacy\", \"twin-a\", \"twin-b\"):
    conn = sqlite3.connect(\"$CWORK/\" + db + \"/.nerve/nerve.db\")
    for i in ids:
        hit += conn.execute(\"select count(*) from entity where entity_id = ?1\", (i,)).fetchone()[0]
assert hit == len(ids), (hit, len(ids))
'"

    # 2. Every export shape the rule declines is named. A wildcard subpath, a null block, an
    #    unsupported condition, an escaping path, an undeclared subpath and a legacy probe are six
    #    different refusals and the response says which is which.
    check "an unsupported export form is recorded with its form named, never dropped" bash -c "
      '$NERVE' repo scan --path '$CWORK/host' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
tally = {row[\"form\"]: row[\"count\"] for row in d[\"unsupported_tally\"]}
for form in (\"npm_export_wildcard_subpath\",\"npm_export_blocked\",
             \"npm_export_unsupported_condition\",\"npm_export_path_escapes_target\",
             \"npm_export_subpath_not_declared\",\"npm_legacy_subpath_probe\"):
    assert tally.get(form, 0) >= 1, (form, tally)
c2 = [r for r in d[\"unsupported\"] if r[\"rule\"] == \"npm_export_resolution\"]
assert len(c2) == 6, c2
# The wildcard case names a file that really exists over there, so declining it is a published
# false negative rather than an absence.
assert any(r[\"identity\"] == \"pkg-map/deep\" for r in c2), c2
'
      test -f '$CWORK/pkg-map/src/deep.ts'"

    # 3. §9.3b, asserted negatively: crossing repositories is opt-in at the contract surface, so an
    #    ordinary local query answers identically with the links present and with them deleted.
    check "a path and an impact query do not traverse a contract link" bash -c "
      '$NERVE' repo scan --path '$CWORK/host' >/dev/null 2>&1
      before_path=\$('$NERVE' path --path '$CWORK/host' src/app.ts src/local.ts --json)
      before_impact=\$('$NERVE' impact --path '$CWORK/host' src/local.ts --relation IMPORTS --json)
      printf '%s' \"\$before_path\" | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"paths\"], d
' || exit 1
      python3 -c '
import sqlite3
conn = sqlite3.connect(\"$CWORK/host/.nerve/nerve.db\")
n = conn.execute(\"select count(*) from contract_link\").fetchone()[0]
assert n >= 8, n
conn.execute(\"delete from contract_link\")
conn.commit()
'
      after_path=\$('$NERVE' path --path '$CWORK/host' src/app.ts src/local.ts --json)
      after_impact=\$('$NERVE' impact --path '$CWORK/host' src/local.ts --relation IMPORTS --json)
      [ \"\$before_path\" = \"\$after_path\" ] || { echo 'path traversed a contract link'; exit 1; }
      [ \"\$before_impact\" = \"\$after_impact\" ] || { echo 'impact traversed a contract link'; exit 1; }"
  else
    skip "C2 npm export resolution" "the contracts-exports fixture repositories could not be built"
  fi

  # 7. C3, on its own fixture and with its own methods. The PEP 621 direct reference is an
  #    absolute URL, so the fixture ships a placeholder — an absolute path cannot be committed.
  if cp -R "$ROOT/fixtures/contracts-python/service" "$CWORK/service" &&
     cp -R "$ROOT/fixtures/contracts-python/pkg-core" "$CWORK/pkg-core" &&
     cp -R "$ROOT/fixtures/contracts-python/pkg-extra" "$CWORK/pkg-extra" &&
     cp -R "$ROOT/fixtures/contracts-python/unregistered" "$CWORK/py-unregistered"; then
    CORE_PATH=$(cd "$CWORK/pkg-core" && pwd -P)
    python3 - "$CWORK/service/pyproject.toml" "$CORE_PATH" <<'SUBST'
import sys
path, core = sys.argv[1], sys.argv[2]
with open(path) as handle:
    text = handle.read()
with open(path, "w") as handle:
    handle.write(text.replace("{{PKG_CORE_PATH}}", core))
SUBST
    for d in service pkg-core pkg-extra py-unregistered; do
      "$NERVE" init "$CWORK/$d" >/dev/null 2>&1
      "$NERVE" index "$CWORK/$d" >/dev/null 2>&1
    done
    "$NERVE" repo add "$CWORK/pkg-core" --path "$CWORK/service" --id pkg-core >/dev/null 2>&1
    "$NERVE" repo add "$CWORK/pkg-extra" --path "$CWORK/service" --id pkg-extra >/dev/null 2>&1

    check "C3 records PEP 621, Poetry and uv path dependencies, each by its own method" bash -c "
      '$NERVE' repo scan --path '$CWORK/service' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
links = {(l[\"section\"], l[\"identity\"]): l for l in d[\"links\"]}
assert d[\"links_recorded\"] == 5, d
assert links[(\"project.dependencies\",\"pkg-core\")][\"resolution_method\"] == \"manifest_declared\", links
assert links[(\"tool.poetry.dependencies\",\"pkg-extra\")][\"resolution_method\"] == \"path_dependency_resolved\", links
assert links[(\"tool.uv.sources\",\"pkg-uv\")][\"resolution_method\"] == \"path_dependency_resolved\", links
assert links[(\"tool.uv.sources\",\"pkg-uv\")][\"registry_id\"] == \"pkg-core\", links
tally = {row[\"form\"]: row[\"count\"] for row in d[\"unsupported_tally\"]}
for form in (\"python_version_specifier\",\"python_unsupported_direct_reference\",
             \"python_git_source\",\"python_url_source\",\"python_workspace_source\"):
    assert tally.get(form, 0) >= 1, (form, tally)
reasons = {r[\"identity\"]: r[\"reason\"] for r in d[\"unresolved\"]}
assert reasons[\"pkg-unregistered\"] == \"target_not_registered\", reasons
'"
  else
    skip "C3 python contract extraction" "the python fixture pair could not be built"
  fi
else
  skip "contract extraction" "fixtures/contracts-npm or fixtures/contracts-python is missing"
fi

# ---------------------------------------------------------------------------------------------
section "5. Supply chain"

LOCKED=$(grep -c '^name = ' "$ROOT/Cargo.lock")
printf '  [%s] Cargo.lock records %s packages\n' "$(dim INFO)" "$LOCKED"
check "third_party/LICENSES.md exists and is non-trivial" \
  bash -c "[ -f '$ROOT/third_party/LICENSES.md' ] && [ \$(grep -c '^|' '$ROOT/third_party/LICENSES.md') -gt 50 ]"
# Not a bare grep for "GPL". The first version of this check failed, and the dependency was fine:
# `r-efi` offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later` and the record says "we take MIT". A
# disjunction containing a copyleft option is not a copyleft dependency. So the rule is: any line
# naming GPL/AGPL/SSPL must also name a permissive licence, i.e. must be a choice we can take.
check "every copyleft mention is a disjunction with a permissive option we take" bash -c "
  awk '/GPL|AGPL|SSPL/ && !/MIT|Apache|BSD|ISC|Zlib|Unlicense/ { print; found=1 }
       END { exit found ? 1 : 0 }' '$ROOT/third_party/LICENSES.md'"

# Clean-room. Named products only — a generic word like "graph" would match constantly.
check "no competitor product is referenced in source or docs" \
  bash -c "! grep -rqiE 'codegraph|graphify|gitnexus' --include='*.rs' --include='*.toml' crates/ Cargo.toml"

# ---------------------------------------------------------------------------------------------
section "6. What this script cannot check, and does not pretend to"

note "MANUAL" "Real-world accuracy: docs/plans/slice-15-real-world-validation.md needs a corpus, two oracles, and network to acquire them"
note "MANUAL" "The Python tracer end to end: scripts/trace_python_e2e.sh needs pytest in a venv, which needs network"
note "MANUAL" "Browser QA of apps/nerve-web: no headless browser is assumed here. The freeze was lifted for *function* on 2026-08-03 — the interface must expose every finalized capability — so this is a real gap, not a deferral"

# ---------------------------------------------------------------------------------------------
printf '\n\033[1mSummary\033[0m\n'
printf '  passed  %s\n  failed  %s\n  skipped %s\n' "$PASS" "$FAIL" "$SKIP"
if [ "$FAIL" -gt 0 ]; then
  printf '\n  failed checks:\n'
  for name in "${FAILED_CHECKS[@]}"; do printf '    - %s\n' "$name"; done
fi
printf '\n  The roadmap is authoritative about completeness: docs/ROADMAP.md.\n'
printf '  This script gates what is built. It does not claim the product is finished.\n\n'

[ "$FAIL" -eq 0 ]
