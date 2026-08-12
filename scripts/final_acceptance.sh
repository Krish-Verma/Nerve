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

# A verb absent by decision, inside a command that exists. Same rule as `refused`, one level down:
# `$NERVE help <command>` cannot ask about a subcommand, so the verb is *typed* and the parser's
# rejection is what is checked.
refused_verb() {
  if "$NERVE" "$1" "$2" --help >/dev/null 2>&1; then
    printf '  [%s] nerve %s %s exists, but %s\n' "$(red FAIL)" "$1" "$2" "$3"
    FAIL=$((FAIL + 1))
    FAILED_CHECKS+=("nerve $1 $2 should not exist")
  else
    printf '  [%s] nerve %s %s — %s\n' "$(dim REFUSED)" "$1" "$2" "$3"
  fi
}
refused_verb memory delete \
  "the brief requires history preserved, and a delete verb is how that stops being true; \
invalidate records that a note ended, and keeps every event it had"

# The "unbuilt commands" loop that used to sit here is gone, and its removal is the point rather
# than a tidy-up. It awarded a PASS for a command's mere existence: `nerve history` moved 35 → 36
# checks by appearing, printing `PASS — nerve history exists — update this script`, which counted
# as a pass while checking nothing about what the command does. That row was replaced by eight real
# checks in `2dc3a7d`, and `nerve memory` was the last name left in the loop — so it is replaced by
# section 4f in the same commit that lands the command, not after. A placeholder that scores is
# worse than a gap that does not.

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
section "4e. The contract surfaces: HTTP, MCP, and the bytes on both sides"

# Behaviour on every surface Slice 13d ships. Not "the route exists" — a route that exists and
# answers nothing is worth nothing — but: can a link be read over HTTP *with its freshness*, does
# the MCP tool answer and refuse an argument its question does not take, does repository-derived
# text stay inside the untrusted field, and are BOTH databases byte-identical afterwards.
#
# `fixtures/contracts-exports` rather than `contracts-npm`, because it is the only fixture that
# produces a C2 link: an import specifier resolved through the neighbour's own export map to a file
# inside it. That is the only link with a target snapshot, and a snapshot is what the freshness
# verdict is read against.
if [ -d "$ROOT/fixtures/contracts-exports" ] && command -v python3 >/dev/null 2>&1; then
  SWORK=$(mktemp -d)
  trap 'rm -rf "${WORK:-}" "${HWORK:-}" "${RWORK:-}" "${CWORK:-}" "${SWORK:-}"' EXIT

  surface_repo() {
    local name="$1"
    cp -R "$ROOT/fixtures/contracts-exports/$name" "$SWORK/$name" || return 1
    rm -rf "$SWORK/$name/.nerve"
    "$NERVE" init "$SWORK/$name" >/dev/null 2>&1 || return 1
    "$NERVE" index "$SWORK/$name" >/dev/null 2>&1 || return 1
    return 0
  }

  # The display name is registered as an XSS payload on purpose. It is untrusted repository content
  # on T7's terms — a neighbour is a checkout that may have been cloned from anywhere — so the
  # question every surface has to answer is where it ends up, not whether it is accepted.
  HOSTILE_NAME='<img src=x onerror=alert(1)> neighbour'

  if surface_repo host && surface_repo pkg-map &&
     "$NERVE" repo add "$SWORK/pkg-map" --path "$SWORK/host" --id pkg-map --name "$HOSTILE_NAME" >/dev/null 2>&1 &&
     "$NERVE" repo scan --path "$SWORK/host" >/dev/null 2>&1; then

    HOST_DB="$SWORK/host/.nerve/nerve.db"
    MAP_DB="$SWORK/pkg-map/.nerve/nerve.db"
    BEFORE_HOST=$(shasum -a 256 "$HOST_DB" | cut -d' ' -f1)
    BEFORE_MAP=$(shasum -a 256 "$MAP_DB" | cut -d' ' -f1)

    # 1. HTTP. The server is started on an ephemeral port and prints its own url and token as JSON,
    #    so nothing here has to guess either.
    "$NERVE" serve "$SWORK/host" --json >"$SWORK/serve.json" 2>"$SWORK/serve.err" &
    SERVE_PID=$!
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
      grep -q '"token"' "$SWORK/serve.json" 2>/dev/null && break
      sleep 0.25
    done

    if grep -q '"token"' "$SWORK/serve.json" 2>/dev/null; then
      check "a contract link is readable over HTTP with its freshness" bash -c "
        python3 - '$SWORK/serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def get(route):
    request = urllib.request.Request(meta['base_url'] + route,
                                     headers={meta['token_header']: meta['token']})
    return json.load(urllib.request.urlopen(request, timeout=20))

answer = get('/api/contracts?limit=500')
assert answer['ok'] is True, answer
links = answer['links']
assert links, 'no link was served'
row = next(l for l in links
           if l['contract_kind'] == 'npm_export_resolution' and l['contract_identity'] == 'pkg-map/sub')
# The freshness verdict, and the snapshot it is read against.
assert row['freshness'] is None, row
assert row['is_current'] is True, row
assert row['relation_semantics'] == 'REFERENCES', row
assert row['resolution_method'] == 'export_map_resolved', row
assert row['target_path_snapshot'] == 'src/sub.ts', row
assert row['target_entity_id'], row
assert row['target_state_at_resolution'], row
assert row['observed_contract_version'] == '3.1.0', row
assert row['registry_entry']['availability'] == 'available', row
assert row['registry_entry']['availability_statement'], row
# A file the neighbour has and never indexed is unknown, not stale and not missing.
data = next(l for l in links if l['contract_identity'] == 'pkg-map/data')
assert data['freshness'] == 'target_partially_indexed', data
assert data['target_path_snapshot'] == 'src/data.json', data
assert data['target_entity_id'] is None, data
# The registry and the vocabulary answer too, and the declined forms are named rather than counted.
entries = get('/api/contracts/registry')['entries']
assert [e['registry_id'] for e in entries] == ['pkg-map'], entries
forms = [t['name'] for t in get('/api/contracts/vocabulary')['vocabulary']['unsupported_forms']]
assert len(forms) == 23, forms
assert 'npm_export_wildcard_subpath' in forms, forms
# Read-only: a POST is refused before anything is routed.
try:
    urllib.request.urlopen(urllib.request.Request(
        meta['base_url'] + '/api/contracts', data=b'x',
        headers={meta['token_header']: meta['token']}), timeout=20)
    raise AssertionError('a POST was answered')
except urllib.error.HTTPError as error:
    assert error.code in (405, 413), error.code
PY"

      check "the neighbour moving on is visible over HTTP as target_changed" bash -c "
        printf 'export function added(): number { return 3; }\n' > '$SWORK/pkg-map/src/added.ts'
        '$NERVE' index '$SWORK/pkg-map' >/dev/null 2>&1 || exit 1
        python3 - '$SWORK/serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
request = urllib.request.Request(meta['base_url'] + '/api/contracts?limit=500',
                                 headers={meta['token_header']: meta['token']})
links = json.load(urllib.request.urlopen(request, timeout=20))['links']
row = next(l for l in links if l['contract_identity'] == 'pkg-map/sub')
assert row['freshness'] == 'target_changed', row
assert row['is_current'] is False, row
assert row['freshness_note'], row
assert row['target_state_at_resolution'] != row['target_current_state'], row
PY"

      # 1b. Slice 13d-ii. The CLI reports a link's freshness, and it AGREES with what
      #     `/api/contracts` reports for the same link. Cross-surface agreement is the point of the
      #     command, so it is asserted rather than the command's existence — a `repo links` that ran
      #     and answered something else would pass an existence check and be worth nothing.
      #
      #     The comparison is keyed on `link_id`, which is the same row in the same table on both
      #     sides, and it covers every link rather than a chosen one. It runs *after* the check
      #     above re-indexed the neighbour, so the agreed verdicts include a real `target_changed`
      #     rather than a column of nulls agreeing with a column of nulls.
      check "the CLI reports a link's freshness, and agrees with /api/contracts" bash -c "
        '$NERVE' repo links --path '$SWORK/host' --limit 500 --json > '$SWORK/cli-links.json' || exit 1
        python3 - '$SWORK/serve.json' '$SWORK/cli-links.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
request = urllib.request.Request(meta['base_url'] + '/api/contracts?limit=500',
                                 headers={meta['token_header']: meta['token']})
http = json.load(urllib.request.urlopen(request, timeout=20))['links']
answer = json.load(open(sys.argv[2]))
cli = answer['links']

assert answer['result_kind'] == 'contract_links', answer['result_kind']
assert answer['links_total'] == len(http) == len(cli), (answer['links_total'], len(http), len(cli))
assert answer['truncated'] is False, answer

# Anti-vacuity: two different real verdicts are present, so 'they agree' is not 'both answered
# null for every row'.
verdicts = {l['freshness'] for l in cli if l['freshness']}
assert 'target_changed' in verdicts, verdicts
assert 'target_partially_indexed' in verdicts, verdicts

served = {l['link_id']: (l['freshness'], l['freshness_note'], l['is_current']) for l in http}
reported = {l['link_id']: (l['freshness'], l['freshness_note'], l['is_current']) for l in cli}
assert served == reported, [k for k in served if served[k] != reported.get(k)]

# And the one link the row is about, named, so a failure says which verdict moved.
row = next(l for l in cli if l['contract_identity'] == 'pkg-map/sub')
assert row['freshness'] == 'target_changed', row
assert row['is_current'] is False, row
assert row['target_state_at_resolution'] != row['target_current_state'], row
assert row['registry_entry']['registry_id'] == 'pkg-map', row['registry_entry']
# The neighbour's display name is the hostile one, and it arrives inert rather than raw.
assert '<img src=x onerror=alert(1)>' in row['registry_entry']['display_name'], row['registry_entry']
PY"
    else
      skip "the contract HTTP surface" "nerve serve did not report a url"
    fi
    kill "$SERVE_PID" >/dev/null 2>&1
    wait "$SERVE_PID" 2>/dev/null

    # The check above re-indexed the neighbour on purpose, which is a write to its database and the
    # only one in this section. The byte comparison below therefore starts from the state that
    # deliberate write left, so that "unchanged" means "no *read* wrote", which is the property.
    AFTER_MOVE_MAP=$(shasum -a 256 "$MAP_DB" | cut -d' ' -f1)

    # 2. MCP. Driven over the wire the way a client drives it, so the framing is part of what is
    #    checked. An argument a question does not take must be REFUSED rather than ignored:
    #    ignoring it would let a caller believe the registry list was narrowed when nothing narrowed
    #    it.
    check "the MCP contract tool answers, and refuses an argument its question does not take" bash -c "
      {
        printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"acceptance\",\"version\":\"1\"}}}'
        printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}'
        printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"nerve_contracts\",\"arguments\":{\"question\":\"links\",\"limit\":100}}}'
        printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"nerve_contracts\",\"arguments\":{\"question\":\"registry\",\"registry_id\":\"pkg-map\"}}}'
        printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"nerve_contracts\",\"arguments\":{\"question\":\"vocabulary\"}}}'
      } | '$NERVE' mcp --path '$SWORK/host' > '$SWORK/mcp.jsonl' 2>'$SWORK/mcp.err'
      python3 - '$SWORK/mcp.jsonl' <<'PY'
import json, sys
rows = {r['id']: r for r in (json.loads(line) for line in open(sys.argv[1]) if line.strip())}
assert set(rows) == {1, 2, 3, 4, 5}, sorted(rows)
names = [t['name'] for t in rows[2]['result']['tools']]
assert 'nerve_contracts' in names, names
# The whole advertised set rather than one tool, so a tool that stopped being advertised fails
# here as well as in cargo test. The number is mcp::TOOL_NAMES.len(), which a shell script cannot
# read, so it moves with the table: five in 8b-ii, six in 12c-iii-b, seven in 13d, eight in 14c.
assert 'nerve_memory' in names, names
assert len(names) == 8, names

answered = rows[3]['result']
assert answered['isError'] is False, answered
payload = answered['structuredContent']
contracts = payload['repository_content']['contracts']
assert contracts['result_kind'] == 'contract_links', contracts['result_kind']
assert contracts['links'], 'the tool answered with no link'
assert contracts['boundary']['read_only'] is True, contracts['boundary']
assert 'nerve repo scan' in contracts['boundary']['commands'], contracts['boundary']

# The refusal: an argument this question does not take, named, with the question's own set.
refused = rows[4]
assert 'error' in refused, refused
assert refused['error']['code'] == -32602, refused['error']
assert refused['error']['data']['argument'] == 'registry_id', refused['error']['data']
assert refused['error']['data']['question'] == 'registry', refused['error']['data']

forms = rows[5]['result']['structuredContent']['repository_content']['contracts']['vocabulary']['unsupported_forms']
assert len(forms) == 23, len(forms)
PY"

    check "repository-derived text stays inside repository_content" bash -c "
      python3 - '$SWORK/mcp.jsonl' <<'PY'
import json, sys
rows = {r['id']: r for r in (json.loads(line) for line in open(sys.argv[1]) if line.strip())}
payload = rows[3]['result']['structuredContent']
content = payload['repository_content']

def strings(value, at=''):
    if isinstance(value, str):
        yield at, value
    elif isinstance(value, list):
        for index, item in enumerate(value):
            yield from strings(item, f'{at}/{index}')
    elif isinstance(value, dict):
        for key, item in value.items():
            yield from strings(item, f'{at}/{key}')

inside = {text for _, text in strings(content)}
assert inside, 'nothing was labelled, so this check is vacuous'
# The hostile display name really reached the answer, or the scan below proves nothing.
payload_marker = '<img src=x onerror=alert(1)>'
assert any(payload_marker in text for text in inside), 'the hostile name never reached the answer'
# Every repository-derived field is present, and nothing outside the envelope repeats one.
link = next(l for l in content['contracts']['links'] if l['contract_identity'] == 'pkg-map/sub')
for field in ('contract_identity', 'observed_contract_version', 'source_path',
              'target_path_snapshot', 'target_name_snapshot', 'target_span_snapshot'):
    assert link[field], (field, link)
assert link['registry_entry']['display_name'], link['registry_entry']
assert link['registry_entry']['local_path'], link['registry_entry']

outside = {key: payload[key] for key in payload if key != 'repository_content'}
for at, text in strings(outside):
    if at.startswith('/query'):
        continue
    assert text not in inside, (at, text)
    assert payload_marker not in text, (at, text)
PY"

    check "both databases are byte-identical after every contract read" bash -c "
      after_host=\$(shasum -a 256 '$HOST_DB' | cut -d' ' -f1)
      after_map=\$(shasum -a 256 '$MAP_DB' | cut -d' ' -f1)
      [ \"\$after_host\" = '$BEFORE_HOST' ] || { echo \"host db changed\"; exit 1; }
      [ \"\$after_map\" = '$AFTER_MOVE_MAP' ] || { echo \"neighbour db changed\"; exit 1; }
      # Anti-vacuity: the reads really produced links, so 'unchanged' is not 'nothing ran'.
      python3 -c '
import json,sys
rows = {r[\"id\"]: r for r in (json.loads(l) for l in open(\"$SWORK/mcp.jsonl\") if l.strip())}
links = rows[3][\"result\"][\"structuredContent\"][\"repository_content\"][\"contracts\"][\"links\"]
assert len(links) >= 4, len(links)
'"
  else
    skip "the contract surfaces" "the export fixture pair could not be built"
  fi
else
  skip "the contract surfaces" "fixtures/contracts-exports is missing or python3 is unavailable"
fi

# ---------------------------------------------------------------------------------------------
section "4f. Human-confirmed memory, and the facts it must keep apart"

# Behaviour, not surface. Every check below asks something the answer to which would be wrong if the
# command merely existed: does a note persist, does confirming change the stored lifecycle, does
# superseding keep the predecessor *and every event it had*, is invalidation a different fact from
# supersession, does a read write, is the export byte-identical twice and does it contain what was
# just written, and is a misspelled scope refused rather than answered with an empty list.
#
# This runs in section 4's throwaway `git archive` checkout, which is already indexed — a memory
# record needs a repository state to anchor to, so an unindexed directory could not exercise any of
# it. `relationPhrase` is the same TypeScript symbol section 4 queries, for the same reason: Nerve
# does not index Rust, so its own source is not a usable subject.
if [ -n "${WORK:-}" ] && [ -f "$WORK/.nerve/nerve.db" ] && command -v python3 >/dev/null 2>&1; then

  check "a proposed note persists, and nothing treats it as settled" bash -c "
    '$NERVE' memory propose --subject relationPhrase --scope interface \
      --content 'the UI renders every relation through this helper' \
      --claim-key rendering --id acc1 --path '$WORK' --json |
    python3 -c '
import json,sys
r = json.load(sys.stdin)[\"records\"][0]
assert r[\"memory_id\"] == \"acc1\", r
assert r[\"status\"] == \"proposed\", r
assert r[\"anchor_state_id\"], r
assert r[\"subject\"][\"selector\"] == \"relationPhrase\", r
assert r[\"views\"] == [], r
assert r[\"events\"][0][\"operation\"] == \"proposed\", r
assert r[\"events\"][0][\"from_status\"] is None, r
'
    '$NERVE' memory show acc1 --path '$WORK' --json | python3 -c '
import json,sys
r = json.load(sys.stdin)[\"records\"][0]
assert r[\"content\"].startswith(\"the UI renders\"), r
assert r[\"status\"] == \"proposed\", r
'"

  check "confirming changes the stored lifecycle and records that it did" bash -c "
    '$NERVE' memory confirm acc1 --note 'checked at the acceptance gate' --path '$WORK' --json |
    python3 -c '
import json,sys
r = json.load(sys.stdin)[\"records\"][0]
assert r[\"status\"] == \"active\", r
events = r[\"events\"]
assert [e[\"operation\"] for e in events] == [\"proposed\", \"confirmed\"], events
assert events[1][\"from_status\"] == \"proposed\" and events[1][\"to_status\"] == \"active\", events
assert events[1][\"note\"] == \"checked at the acceptance gate\", events
'"

  # The row's own property. A supersession that kept the row but lost an event would pass a naive
  # \"the predecessor is still there\" check, so the events are compared as a list.
  check "superseding keeps the predecessor and every event it had" bash -c "
    '$NERVE' memory cite acc1 --file docs/ROADMAP.md --span 1:20 --path '$WORK' >/dev/null || exit 1
    '$NERVE' memory supersede acc1 --content 'the UI now renders relations in two places' \
      --id acc2 --note 'the interface grew' --path '$WORK' --json |
    python3 -c '
import json,sys
rows = {r[\"memory_id\"]: r for r in json.load(sys.stdin)[\"records\"]}
old, new = rows[\"acc1\"], rows[\"acc2\"]
assert old[\"status\"] == \"superseded\", old
assert old[\"content\"].startswith(\"the UI renders\"), old
assert old[\"superseded_by_memory_id\"] == \"acc2\", old
assert new[\"supersedes_memory_id\"] == \"acc1\", new
assert new[\"subject\"] == old[\"subject\"], (new, old)
assert [e[\"operation\"] for e in old[\"events\"]] == \
       [\"proposed\", \"confirmed\", \"cited\", \"superseded\"], old[\"events\"]
cited = old[\"events\"][2]
assert cited[\"from_status\"] == cited[\"to_status\"], cited
assert old[\"citations\"][0][\"cited_path\"] == \"docs/ROADMAP.md\", old[\"citations\"]
'"

  # Two retirements, two facts. Collapsing them loses \"what did we once believe and no longer do,
  # with no successor\" — the question a returning reader actually asks.
  check "invalidation is a different fact from supersession, in the same read" bash -c "
    '$NERVE' memory invalidate acc2 --reason 'the helper was inlined' --path '$WORK' >/dev/null || exit 1
    '$NERVE' memory list --path '$WORK' --json | python3 -c '
import json,sys
rows = {r[\"memory_id\"]: r for r in json.load(sys.stdin)[\"records\"]}
old, new = rows[\"acc1\"], rows[\"acc2\"]
assert old[\"status\"] == \"superseded\", old
assert new[\"status\"] == \"invalidated\", new
assert old[\"status\"] != new[\"status\"]
assert new[\"invalidated_at\"], new
assert new[\"invalidation_reason\"] == \"the helper was inlined\", new
assert new[\"superseded_by_memory_id\"] is None, new
assert old[\"invalidated_at\"] is None, old
'"

  check "every memory read leaves the database byte-identical" bash -c "
    before=\$(shasum -a 256 '$WORK/.nerve/nerve.db' | cut -d' ' -f1)
    '$NERVE' memory list --path '$WORK' >/dev/null || exit 1
    '$NERVE' memory show acc1 --path '$WORK' >/dev/null || exit 1
    '$NERVE' memory search 'renders every relation' --path '$WORK' >/dev/null || exit 1
    '$NERVE' memory events acc1 --path '$WORK' >/dev/null || exit 1
    '$NERVE' memory export --path '$WORK' >/dev/null || exit 1
    after=\$(shasum -a 256 '$WORK/.nerve/nerve.db' | cut -d' ' -f1)
    [ \"\$before\" = \"\$after\" ] || { echo \"\$before != \$after\"; exit 1; }
    # Anti-vacuity: the reads really answered, so 'unchanged' is not 'nothing ran'.
    '$NERVE' memory search 'renders every relation' --path '$WORK' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"count\"] == 1, d
'"

  check "the export carries the records just written, with their history" bash -c "
    '$NERVE' memory export --path '$WORK' | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"format\"] == \"nerve-memory-export\", d
assert d[\"format_version\"] == 1, d
assert d[\"schema_version\"] == 10, d
assert d[\"repo_id\"].startswith(\"repo_\"), d
rows = {r[\"memory_id\"]: r for r in d[\"records\"]}
assert d[\"record_count\"] == len(d[\"records\"]) == 2, d[\"record_count\"]
assert rows[\"acc1\"][\"content\"].startswith(\"the UI renders\"), rows[\"acc1\"]
assert [e[\"operation\"] for e in rows[\"acc1\"][\"events\"]] == \
       [\"proposed\", \"confirmed\", \"cited\", \"superseded\"], rows[\"acc1\"][\"events\"]
assert rows[\"acc2\"][\"invalidation_reason\"] == \"the helper was inlined\", rows[\"acc2\"]
# Memory is the one thing re-indexing cannot rebuild, so the export must be a whole record and not
# a summary of one.
assert sorted(rows[\"acc1\"]) == [\"anchor_state_id\",\"author_label\",\"citations\",\"claim_key\",
                                 \"content\",\"created_at\",\"events\",\"invalidated_at\",
                                 \"invalidation_reason\",\"memory_id\",\"scope\",\"status\",
                                 \"subject\",\"supersedes_memory_id\"], sorted(rows[\"acc1\"])
'"

  # §7.4e. Deterministic means byte-identical, which is why the document carries no timestamp of its
  # own — and no derived state, and no absolute path, each of which would also make it a claim
  # rather than a copy.
  check "the export is byte-identical twice, and dates and derives nothing" bash -c "
    '$NERVE' memory export --path '$WORK' > /tmp/nerve_memory_export_a.json || exit 1
    '$NERVE' memory export --out /tmp/nerve_memory_export_b.json --path '$WORK' >/dev/null || exit 1
    cmp -s /tmp/nerve_memory_export_a.json /tmp/nerve_memory_export_b.json ||
      { echo 'two exports of one database differ'; exit 1; }
    for forbidden in exported_at potentially_stale conflicted multiple_active current_state_id \
                     subject_resolution superseded_by; do
      grep -q \"\$forbidden\" /tmp/nerve_memory_export_a.json &&
        { echo \"the export carries \$forbidden\"; exit 1; }
    done
    grep -q '/Users/' /tmp/nerve_memory_export_a.json && { echo 'the export carries a home path'; exit 1; }
    grep -qF '$WORK' /tmp/nerve_memory_export_a.json && { echo 'the export carries the root'; exit 1; }
    exit 0"

  # `absence is not zero`. A misspelled scope must be refused at the point of entry, because against
  # a free-form column it would answer \"there are no notes\" when what is true is \"there is no such
  # scope\" — and it would also silently suppress a conflict report.
  check "a misspelled scope is refused, and a legal empty one is not" bash -c "
    out=\$('$NERVE' memory list --scope opertions --path '$WORK' --json); rc=\$?
    [ \"\$rc\" = 10 ] || { printf '%s\n' \"\$out\"; exit 1; }
    printf '%s' \"\$out\" | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"ok\"] is False, d
assert \"implementation, interface, operations, process\" in d[\"error\"], d
assert \"records\" not in d, d
'
    '$NERVE' memory list --scope process --path '$WORK' --json | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert d[\"count\"] == 0, d
'
    out=\$('$NERVE' memory list --status potentially_stale --path '$WORK' 2>&1); rc=\$?
    [ \"\$rc\" = 10 ] || { printf '%s\n' \"\$out\"; exit 1; }
    printf '%s' \"\$out\" | grep -q 'derived at read time'"
else
  skip "human-confirmed memory" "the indexed checkout from section 4 is unavailable, or python3 is"
fi

# ---------------------------------------------------------------------------------------------
section "4g. The memory read surfaces, and whether they agree with the command line"

# Slice 14c. The HTTP API reads memory and cannot write it, and what it reads is what `nerve memory`
# reads. Cross-surface agreement is the point of a second surface, so it is asserted on the values
# rather than on the routes existing — a `/api/memory` that answered something else would pass an
# existence check and be worth nothing. This is the shape 13d-ii used for a link's freshness, and it
# needs both surfaces running at once, which is why it lives here rather than in `cargo test`.
#
# It runs against section 4f's records, which is deliberate: `acc1` is superseded and `acc2` is
# invalidated, so the comparison covers the two retirements that must never collapse into one.
if [ -n "${WORK:-}" ] && [ -f "$WORK/.nerve/nerve.db" ] && command -v python3 >/dev/null 2>&1 &&
   "$NERVE" memory show acc1 --path "$WORK" >/dev/null 2>&1; then

  MEM_BEFORE=$(shasum -a 256 "$WORK/.nerve/nerve.db" | cut -d' ' -f1)
  "$NERVE" serve "$WORK" --json >"$WORK/memory-serve.json" 2>"$WORK/memory-serve.err" &
  MEM_SERVE_PID=$!
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    grep -q '"token"' "$WORK/memory-serve.json" 2>/dev/null && break
    sleep 0.25
  done

  if grep -q '"token"' "$WORK/memory-serve.json" 2>/dev/null; then
    check "every memory record is served, and the CLI and /api/memory agree field for field" bash -c "
      '$NERVE' memory list --path '$WORK' --json > '$WORK/cli-memory.json' || exit 1
      '$NERVE' memory show acc1 --path '$WORK' --json > '$WORK/cli-memory-one.json' || exit 1
      python3 - '$WORK/memory-serve.json' '$WORK/cli-memory.json' '$WORK/cli-memory-one.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def get(route):
    request = urllib.request.Request(meta['base_url'] + route,
                                     headers={meta['token_header']: meta['token']})
    return json.load(urllib.request.urlopen(request, timeout=20))

served = get('/api/memory?limit=200')
assert served['ok'] is True, served
assert served['result_kind'] == 'memory_records', served['result_kind']
http = {r['memory_id']: r for r in served['records']}
cli = {r['memory_id']: r for r in json.load(open(sys.argv[2]))['records']}

# Anti-vacuity: two different real stored statuses are present, so 'they agree' is not 'both
# answered the same thing about nothing'.
assert set(cli) == {'acc1', 'acc2'}, sorted(cli)
assert cli['acc1']['status'] == 'superseded', cli['acc1']
assert cli['acc2']['status'] == 'invalidated', cli['acc2']
assert cli['acc1']['status'] != cli['acc2']['status']
assert len(cli['acc1']['events']) == 4, cli['acc1']['events']
assert cli['acc1']['citations'], cli['acc1']

# Every record, every field, both directions.
assert http == cli, [k for k in set(http) | set(cli) if http.get(k) != cli.get(k)]
assert served['records_in_repository'] == len(cli) == served['records_matching']

# And one record read on its own is the same record again, not a summary of it.
one = get('/api/memory/record?memory_id=acc1')
assert one['result_kind'] == 'memory_record', one['result_kind']
assert one['records'] == [cli['acc1']], one['records']

# The derived half is served and marked as derived rather than stored.
assert served['records'][0]['views_are_derived'] is True, served['records'][0]
assert cli['acc1']['superseded_by_memory_id'] == 'acc2', cli['acc1']
assert cli['acc1']['superseded_by_is_derived'] is True, cli['acc1']

# The boundary is on the answer, and it names commands rather than offering a control.
assert served['boundary']['read_only'] is True, served['boundary']
commands = served['boundary']['commands']
assert any(c.startswith('nerve memory propose') for c in commands), commands
assert any(c.startswith('nerve memory confirm') for c in commands), commands
assert not any('delete' in c for c in commands), commands
PY"

    # `absence is not zero`, on the second surface. The CLI exits 10 for a misspelled scope; HTTP
    # answers 400 with the admitted set, and neither answers an empty list.
    check "an unknown scope or status is refused over HTTP with the admitted set" bash -c "
      python3 - '$WORK/memory-serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def refusal(route):
    request = urllib.request.Request(meta['base_url'] + route,
                                     headers={meta['token_header']: meta['token']})
    try:
        urllib.request.urlopen(request, timeout=20)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)
    raise AssertionError(route + ' was answered')

code, body = refusal('/api/memory?scope=opertions')
assert code == 400, code
assert body['error']['code'] == 'unknown_scope', body
assert body['error']['detail']['allowed'] == \
       ['implementation', 'interface', 'operations', 'process'], body['error']['detail']
assert body['error']['detail']['this_is_not_an_empty_list'] is True, body
assert 'records' not in body, body

code, body = refusal('/api/memory?status=potentially_stale')
assert code == 400, code
assert body['error']['detail']['named_a_derived_view'] is True, body

# A record that is not here is a refusal too, and it is not an empty record.
code, body = refusal('/api/memory/record?memory_id=nope')
assert code == 404, code
assert body['error']['detail']['this_is_not_an_empty_record'] is True, body

# And a legal filter that matches nothing is not a refusal, which is the other half.
request = urllib.request.Request(meta['base_url'] + '/api/memory?scope=process',
                                 headers={meta['token_header']: meta['token']})
answer = json.load(urllib.request.urlopen(request, timeout=20))
assert answer['records'] == [], answer['records']
assert answer['result_kind'] == 'no_memory_matches', answer['result_kind']
assert answer['records_in_repository'] == 2, answer
PY"

    # Acceptance criterion 7. Read-only is proved on the bytes, across a session that includes the
    # write attempts — not on the routes being absent.
    check "no memory route accepts a write verb, and the database is byte-identical after" bash -c "
      python3 - '$WORK/memory-serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
refused = 0
for route in ('/api/memory', '/api/memory/record?memory_id=acc1'):
    for method in ('POST', 'PUT', 'PATCH', 'DELETE'):
        request = urllib.request.Request(meta['base_url'] + route, data=b'x', method=method,
                                         headers={meta['token_header']: meta['token']})
        try:
            urllib.request.urlopen(request, timeout=20)
            raise AssertionError(method + ' ' + route + ' was answered')
        except urllib.error.HTTPError as error:
            assert error.code in (405, 413), (method, route, error.code)
            refused += 1
assert refused == 8, refused
PY
      after=\$(shasum -a 256 '$WORK/.nerve/nerve.db' | cut -d' ' -f1)
      [ \"\$after\" = '$MEM_BEFORE' ] || { echo \"the database changed: $MEM_BEFORE != \$after\"; exit 1; }"

    # Slice 14d, and row 14 §7.9's last surface. The interface is compiled into this binary, so it
    # is fetched from the running server rather than read off disk — a bundle that was rebuilt and
    # never re-embedded would pass a file check and ship the old screen, which is `82a6ff3`'s
    # defect and the reason the source-side gloss tests all passed while the binary served
    # something else.
    #
    # What is asserted is what the screen can *say*, not that a file exists: it can ask for the
    # records, it tells the two absences apart, it labels the stored half and the derived half
    # separately, it carries the prose for all five memory vocabularies, and it has no write verb
    # anywhere in it. A page that could not do those would render a note as a row of bare tokens
    # with no way to tell what was recorded from what was worked out.
    check "the shipped interface can display a note, and carries no way to write one" bash -c "
      python3 - '$WORK/memory-serve.json' <<'PY'
import json, re, sys, urllib.request
meta = json.load(open(sys.argv[1]))

def asset(route):
    # Deliberately unauthenticated: a browser cannot put a header on a <script src>, so the
    # interface's own files are the one exemption, and this is exercising it as a browser does.
    with urllib.request.urlopen(meta['base_url'] + route, timeout=20) as answer:
        assert answer.status == 200, (route, answer.status)
        return answer.read().decode('utf-8', 'replace')

page = asset('/')
bundle = asset('/assets/nerve.js')
style = asset('/assets/nerve.css')
assert '/assets/nerve.js' in page, page[:400]

# It can ask both memory routes, and it can tell the two absences apart. They are different
# absences with different next steps, and a screen that rendered them alike would report
# 'nobody has written anything here' as 'your filters matched nothing'.
for needle in ('/api/memory', '/api/memory/record', 'no_memory_recorded', 'no_memory_matches'):
    assert needle in bundle, needle

# Stored and derived are drawn as two labelled groups with different rules beside them. Both the
# label and the rule are checked: a label with no rule is a claim the stylesheet does not keep.
assert 'worked out when read' in bundle, 'the derived half is not labelled'
assert 'kind--derived' in bundle and 'kind--derived' in style, 'the derived half has no rule'
assert 'dashed' in style

# One sentence from each of the five memory vocabularies, so a gloss that reached the source and
# not the binary fails here as well as in cargo test.
for gloss in ('It stopped being true and nothing replaced it',
              'Several notes are about this subject',
              'The note follows it because that record exists',
              'The claim is about how people work on it',
              'The only entry that changes no status'):
    assert gloss in bundle, gloss

# And the shipped interface issues exactly one HTTP method. Asserted on the request the page
# actually builds rather than on the words, twice over: PUT occurs inside INPUT and DELETE inside
# DELETED, so a substring search would fail on React's own strings; and the quote characters are
# required rather than matched with a wildcard, because the gloss for a method entity begins
# 'method:\"A function declared on…' and a wildcard reads its first word as a verb.
methods = set(re.findall(r'method:\s*\"([A-Z]{3,7})\"', bundle))
assert methods == {'GET'}, methods
assert '<form' not in page and '<form' not in bundle, 'the interface ships a form'
PY"
  else
    skip "the memory HTTP surface" "nerve serve did not report a url"
  fi
  kill "$MEM_SERVE_PID" >/dev/null 2>&1
  wait "$MEM_SERVE_PID" 2>/dev/null

  # The agent surface, driven as an agent drives it. What is asserted is the boundary rather than
  # the tool: the notes are readable, and the answer names the command a human runs instead of
  # offering a confirmation the surface must not have.
  check "the MCP surface reads memory, cannot write it, and prints the command instead" bash -c "
    printf '%s\n' \
      '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"acceptance\",\"version\":\"1\"}}}' \
      '{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}' \
      '{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}' \
      '{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"nerve_memory\",\"arguments\":{\"limit\":100}}}' \
      '{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"nerve_memory\",\"arguments\":{\"memory_id\":\"acc1\"}}}' \
      '{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"nerve_memory_confirm\",\"arguments\":{\"memory_id\":\"acc1\"}}}' |
    '$NERVE' mcp '$WORK' > '$WORK/mcp-memory.jsonl' || exit 1
    python3 - '$WORK/mcp-memory.jsonl' <<'PY'
import json, sys
lines = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
answers = {line['id']: line for line in lines}
assert len(lines) == 5, lines

tools = [t['name'] for t in answers[2]['result']['tools']]
assert 'nerve_memory' in tools, tools
# No confirmation tool exists to call, and the name is refused with the closed set rather than
# defaulted into some other tool's answer.
assert not any('confirm' in name for name in tools), tools
assert answers[5]['error']['code'] == -32602, answers[5]
assert 'nerve_memory_confirm' not in answers[5]['error']['data']['tools'], answers[5]

listed = answers[3]['result']['structuredContent']
memory = listed['repository_content']['memory']
rows = {r['memory_id']: r for r in memory['records']}
assert set(rows) == {'acc1', 'acc2'}, sorted(rows)
assert rows['acc1']['status'] == 'superseded', rows['acc1']
assert rows['acc2']['status'] == 'invalidated', rows['acc2']
assert listed['evidence']['carries_assertions'] is False, listed['evidence']

# The substitute for proposing: the exact command, as static text a human runs.
commands = memory['boundary']['commands']
assert memory['boundary']['read_only'] is True, memory['boundary']
assert any(c.startswith('nerve memory confirm') for c in commands), commands

# Every hostile-in-principle string a human typed is inside the labelled field and nowhere else.
one = answers[4]['result']['structuredContent']
outside = json.dumps({k: v for k, v in one.items() if k != 'repository_content'})
note = one['repository_content']['memory']['records'][0]['content']
assert note, one['repository_content']['memory']['records'][0]
assert note not in outside, note
PY"
else
  skip "the memory read surfaces" "the indexed checkout from section 4 holds no memory record, or python3 is unavailable"
fi

# ---------------------------------------------------------------------------------------------
section "4h. The three surfaces the UI had no view for, and what each must not say"

# The functional UI parity slice. `/api/impact` had existed since Slice 7b with no view, selector
# alternatives had been reported on every answer since 8b-i and rendered nowhere, and Slice 11a's
# trace evidence had no surface at all. Each is checked on behaviour rather than on a route
# existing, and then on the *shipped bundle*, because the interface is compiled into the binary and
# a rebuilt-but-not-re-embedded screen passes every source-side test — `82a6ff3`'s defect.
#
# `fixtures/trace-basic` is the subject rather than this repository's own checkout, for three
# reasons that no other fixture gives at once: it holds a real `TEST_OBSERVED_CALL` edge, its
# `src/parse.py` is a path with **two readings** (a module and a file), and two of its artifacts
# bind differently — `stale.jsonl` names a tree that is not this one and `unverified.jsonl` names no
# tree at all. That pair is the distinction the whole trace surface turns on.
#
# The directory must be named `repo`: the artifacts declare `repository_root_name: "repo"`, and an
# import into a differently-named checkout is refused as `other-repository` — correctly.
if command -v python3 >/dev/null 2>&1; then
  UIWORK=$(mktemp -d)
  TRACED="$UIWORK/repo"
  cp -R "$ROOT/fixtures/trace-basic" "$TRACED"

  if (cd "$TRACED" && "$NERVE" init >/dev/null 2>&1 && "$NERVE" index >/dev/null 2>&1 &&
      "$NERVE" trace import trace/unverified.jsonl >/dev/null 2>&1 &&
      "$NERVE" trace import trace/stale.jsonl >/dev/null 2>&1); then

    "$NERVE" serve "$TRACED" --json >"$UIWORK/serve.json" 2>"$UIWORK/serve.err" &
    UI_SERVE_PID=$!
    for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
      grep -q '"token"' "$UIWORK/serve.json" 2>/dev/null && break
      sleep 0.25
    done

    if grep -q '"token"' "$UIWORK/serve.json" 2>/dev/null; then

      # ---- impact -----------------------------------------------------------------------------
      # The account of what the answer cannot see is a present object on *every* answer, and the
      # relation set that was walked is echoed. Both are what the view renders, so both are checked
      # here on the values rather than assumed from the endpoint answering at all.
      check "/api/impact states what it cannot see, and the caveat can exceed the answer" bash -c "
        python3 - '$UIWORK/serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def get(route):
    request = urllib.request.Request(meta['base_url'] + route,
                                     headers={meta['token_header']: meta['token']})
    return json.load(urllib.request.urlopen(request, timeout=20))

answer = get('/api/impact?subject=parse_all')

# Present, never null, never omitted — including when a count is zero. A short results array with
# no account beside it reads as 'few things depend on this, safe to change'.
assert 'unresolved' in answer, sorted(answer)
account = answer['unresolved']
assert account is not None
for field in ('sites', 'assertions', 'targets', 'by_category'):
    assert field in account, (field, account)
    assert account[field] is not None
# sites counts observations and is the number a view shows; it is never below the coarser grains.
assert account['sites'] >= account['assertions'] >= account['targets'], account

# The zero case is present rather than omitted, which is the case the panel exists for.
zero = get('/api/impact?subject=Parser')
assert 'unresolved' in zero and zero['unresolved'] is not None, zero

# The relation set actually walked, echoed. Empty means these five, NOT every relation — following
# CONTAINS would answer that every symbol impacts the whole repository.
subject = get('/api/impact?subject=function:parse')
assert subject['relations'] == ['CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS', 'SERVED_BY'], \
       subject['relations']

# Anti-vacuity, and the sharpest check in this section: this database *does* hold a
# TEST_OBSERVED_CALL edge — 4h imports two trace artifacts above — and it is still not in the
# closure. So the exclusion is a decision being kept, not an empty set being reported.
assert 'TEST_OBSERVED_CALL' not in subject['relations'], subject['relations']
why = get('/api/why?subject=function:parse')
assert any(a['relation'] == 'TEST_OBSERVED_CALL' for a in why['assertions']), \
       'the fixture must hold a trace edge for the exclusion above to mean anything'

# On this fixture the caveat is a real one rather than a zero: unresolved sites outstanding beside
# a non-empty closure is the shape the view is designed around.
assert subject['unresolved']['sites'] > 0, subject['unresolved']
assert subject['totals']['entities'] > 0, subject['totals']

# The cap applies to rows only; every tally stays exact. A view that presented the totals as capped
# would turn the one trustworthy number on the screen into a lower bound.
capped = get('/api/impact?subject=function:parse&limit=1')
assert capped['count'] == 1, capped['count']
assert capped['truncated'] is True, capped
assert capped['results_total'] == subject['results_total'], (capped, subject)
assert capped['totals'] == subject['totals'], 'a cap on rows changed a tally'
PY"

      # ---- selector alternatives --------------------------------------------------------------
      # 'content wins, container is reported' — and the report has to reach a human. `src/parse.py`
      # is both a module and a file, which is the case the field exists for.
      check "a path with two readings reports the one it passed over, on every selector surface" bash -c "
        python3 - '$UIWORK/serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def get(route):
    request = urllib.request.Request(meta['base_url'] + route,
                                     headers={meta['token_header']: meta['token']})
    return json.load(urllib.request.urlopen(request, timeout=20))

# Every endpoint that resolves a selector carries the object, keyed by the query parameter name.
for route, key in (('/api/entity?selector=src/parse.py', 'selector'),
                   ('/api/why?subject=src/parse.py', 'subject'),
                   ('/api/neighbourhood?selector=src/parse.py', 'selector'),
                   ('/api/impact?subject=src/parse.py', 'subject')):
    answer = get(route)
    assert 'selectors' in answer, (route, sorted(answer))
    note = answer['selectors'][key]
    assert note['matched_by'] == 'path', (route, note)

    # The container was passed over by a stated rule, and it is named rather than dropped.
    alternatives = note['alternatives']
    assert len(alternatives) == 1, (route, alternatives)
    assert alternatives[0]['kind'] == 'file', (route, alternatives)
    assert alternatives[0]['file_path'] == 'src/parse.py', (route, alternatives)

# The content won: the same path resolved to the module, not to the file it reported.
assert get('/api/entity?selector=src/parse.py')['entity']['kind'] == 'module'

# And the passed-over entity is addressable, so the offer a view makes is one that works.
assert get('/api/entity?selector=file:src/parse.py')['entity']['kind'] == 'file'

# The ordinary case stays empty rather than inventing a second reading.
plain = get('/api/impact?subject=Parser')
assert plain['selectors']['subject']['alternatives'] == [], plain['selectors']

# A refusal to choose is a different event from a choice made by rule, and keeps its own status.
# The bare name 'parse' names both a function and a module, and Nerve refuses rather than picking.
#
# No backticks anywhere in this heredoc. It sits inside a double-quoted bash -c argument, which the
# outer shell expands *before* the heredoc exists, so a backtick pair here is a command substitution
# that silently rewrites the Python below. It cost two spurious 'command not found' lines on the run
# that added this section, and the assertions still passed — which is what makes it worth a comment.
try:
    urllib.request.urlopen(urllib.request.Request(
        meta['base_url'] + '/api/why?subject=parse',
        headers={meta['token_header']: meta['token']}), timeout=20)
    raise AssertionError('an ambiguous selector was answered')
except urllib.error.HTTPError as error:
    assert error.code == 409, error.code
    body = json.load(error)
    assert body['error']['code'] == 'ambiguous_selector', body
    assert len(body['error']['detail']['candidates']) == 2, body['error']['detail']
PY"

      # ---- trace ------------------------------------------------------------------------------
      # No read route was added, and none is needed: a trace observation is an observation, and the
      # evidence endpoints are generic over the evidence model. What is checked is that the
      # existential evidence arrives intact through the routes that already exist.
      check "trace evidence reaches the UI through the evidence routes, with no /api/trace" bash -c "
        python3 - '$UIWORK/serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def get(route):
    request = urllib.request.Request(meta['base_url'] + route,
                                     headers={meta['token_header']: meta['token']})
    return json.load(urllib.request.urlopen(request, timeout=20))

# Import is a write path and is CLI-only. There is no trace read route, and its absence is checked
# so that adding one silently would be a decision somebody had to make rather than a drift.
for route in ('/api/trace', '/api/traces', '/api/trace/runs'):
    try:
        urllib.request.urlopen(urllib.request.Request(
            meta['base_url'] + route,
            headers={meta['token_header']: meta['token']}), timeout=20)
        raise AssertionError(route + ' answered; there is no trace read route')
    except urllib.error.HTTPError as error:
        assert error.code == 404, (route, error.code)

why = get('/api/why?subject=function:parse')
traced = [a for a in why['assertions'] if a['relation'] == 'TEST_OBSERVED_CALL']
assert traced, 'no trace edge reached /api/why'

# The relation is kept distinct from CALLS. A trace says one run took this edge; a static call says
# the source contains it. Neither implies the other, and the same pair of frames may hold both.
observations = [o for a in traced for o in a['observations']]
assert observations, traced
assert all(o['evidence_source_type'] == 'TEST_CALL_TRACE' for o in observations), observations

# A *set* of runs, not a run: two artifacts observed the same site, and one observation names both
# because idx_observation_identity has no column that could hold a second row per test.
environment = json.loads(observations[0]['environment'])
runs = environment['runs']
assert len(runs) == 2, runs
bindings = sorted(r['repository_binding'] for r in runs)
assert bindings == ['stale', 'unverified'], bindings

# The derived scalar is the WEAKEST claim across contributing runs. One run said nothing about
# which tree it ran against and one named a different tree; the answer is the worse of the two, and
# 'unverified' is never upgraded into a pass by sitting beside anything.
assert environment['repository_binding'] == 'stale', environment['repository_binding']
assert environment['completion_state'] == 'complete', environment['completion_state']

# The tests are a list, because two tests reaching one callee from one line are one observation.
assert isinstance(environment['tests'], list), environment['tests']
assert environment['tests'], environment

# A count belongs to one run and is never summed into a frequency across runs.
for run in runs:
    assert isinstance(run['tests'], dict), run['tests']
    assert all(isinstance(v, int) for v in run['tests'].values()), run['tests']
PY"

      # ---- the shipped interface --------------------------------------------------------------
      # Fetched from the running server rather than read off disk. The bundle is a tracked build
      # artifact compiled in with include_bytes!, so a screen that was rebuilt and never re-embedded
      # passes every source-side test and ships the old interface.
      check "the shipped interface can display all three, and hedges the trace edge" bash -c "
        python3 - '$UIWORK/serve.json' <<'PY'
import json, sys, urllib.request
meta = json.load(open(sys.argv[1]))
def asset(route):
    # Unauthenticated on purpose: a browser cannot put a header on a <script src>.
    with urllib.request.urlopen(meta['base_url'] + route, timeout=20) as answer:
        assert answer.status == 200, (route, answer.status)
        return answer.read().decode('utf-8', 'replace')

bundle = asset('/assets/nerve.js')

# It can ask the endpoint, and it has a route of its own to be linked to.
for needle in ('/api/impact', '#/impact'):
    assert needle in bundle, needle

# Both branches of the unresolved account are in the shipped screen. The zero branch is the one
# that matters: a build carrying only the warning would omit the panel exactly when its absence
# most invites the wrong conclusion.
assert 'cannot rule them out' in bundle, 'the unresolved warning is not shipped'
assert 'no failed resolution is hiding a dependency' in bundle, 'the zero case is not shipped'

# And the screen never relabels the closure as test impact. The 'nerve affected' command is
# refused rather than deferred: LCOV carries no per-test attribution (ADR-0008 A.2).
for forbidden in ('affected tests', 'test impact', 'impacted tests'):
    assert forbidden not in bundle.lower(), forbidden

# Selector alternatives reach the screen.
for needle in ('also at this path', 'content wins'):
    assert needle in bundle, needle

# The trace surface, and the wording that has to stay existential.
assert 'not that every run does' in bundle, 'the existential sentence is not shipped'
assert 'absence of an edge is absence of observation' in bundle, bundle[:0]
assert 'not the observation of an absence' in bundle, 'the empty trace case is not shipped'

# One sentence from each of the three trace vocabularies, so a gloss that reached the source and
# not the binary fails here as well as in cargo test.
for gloss in ('The run reached the end of the suite',
              'This is the absence of a check rather than a failed one',
              'these locations may be of generated code'):
    assert gloss in bundle, gloss

# The three-valued binding survives into the shipped build: unverified must be renderable as
# itself rather than as either of the other two.
for value in ('bound', 'stale', 'unverified'):
    assert value in bundle, value
PY"
    else
      skip "the impact, selector and trace UI surfaces" "nerve serve did not report a url"
    fi
    kill "$UI_SERVE_PID" >/dev/null 2>&1
    wait "$UI_SERVE_PID" 2>/dev/null
  else
    skip "the impact, selector and trace UI surfaces" "the trace fixture did not index and import"
  fi
  rm -rf "$UIWORK"
else
  skip "the impact, selector and trace UI surfaces" "python3 is unavailable"
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
