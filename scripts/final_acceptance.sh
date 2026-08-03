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
for pair in "history:Slice 12 — Git history and the temporal layer" \
            "memory:Slice 14 — human-confirmed memory"; do
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
note "MANUAL" "Visual QA of apps/nerve-web: the frontend is frozen and owned by the user"

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
