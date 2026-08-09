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

for command in init index coverage trace status check doctor search gaps impact path serve mcp why; do
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
