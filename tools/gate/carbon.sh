#!/usr/bin/env bash
# Guarded gate for automated Carbon/Neon runs in motion (motionforge).
# Usage:
#   bash tools/gate/carbon.sh              # full gate (scope + fmt + build + rust tests/goldens + python stdlib tests)
#   bash tools/gate/carbon.sh --scope-only # scope & test-count checks only (used by tools/gate/planted.sh)
#
# Prints PASS/FAIL per check and ends with GATE PASS (exit 0) or GATE FAIL (<N>) (exit 1).
set -u
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)" || exit 2

SCOPE_ONLY=0
case "${1:-}" in
  "") ;;
  --scope-only) SCOPE_ONLY=1 ;;
  *)
    echo "FAIL  unknown argument: $1"
    echo "GATE FAIL (1)"
    exit 1
    ;;
esac

FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${1:-30}"; }

T=$(mktemp -d)
OWN_TARGET_DIR=""
cleanup() {
  rm -rf "$T"
  if [ -n "$OWN_TARGET_DIR" ]; then
    rm -rf "$OWN_TARGET_DIR"
  fi
}
trap cleanup EXIT

finish() {
  echo
  if [ "$FAILS" -eq 0 ]; then
    echo "GATE PASS"
    exit 0
  fi
  echo "GATE FAIL ($FAILS)"
  exit 1
}

echo "== base commit"
BASE="${GATE_BASE:-$(git log -1 --format=%H -- tools/gate)}"
if [ -z "$BASE" ]; then
  fail "cannot determine base commit (GATE_BASE unset and no commit touches tools/gate)"
  BASE=HEAD
elif ! git rev-parse --verify "${BASE}^{commit}" >/dev/null 2>&1; then
  fail "invalid base commit: $BASE"
  BASE=HEAD
else
  pass "base commit $(git rev-parse --short=12 "$BASE")"
fi

echo "== scope"
GUARDED_RE='^(rust/core/tests/|rust/cli/tests/|tests/fixtures/|rust/core/src/readcam\.rs$|rust/core/src/motioncheck\.rs$|tools/bench/|tools/gate/)'
touched=$(
  {
    for c in $(git log --format=%H --grep '^carbon [^ ]*: round' "$BASE..HEAD"); do
      git diff-tree --no-commit-id --name-only -r "$c"
    done
    git status --porcelain | cut -c4- | awk '{ n = split($0, a, " -> "); for (i = 1; i <= n; i++) print a[i] }'
  } | grep -v '^$' | sort -u
)
bad=""
if [ -n "$touched" ]; then
  bad=$(printf '%s\n' "$touched" | grep -E "$GUARDED_RE" || true)
fi
if [ -z "$bad" ]; then
  pass "no guarded paths touched"
else
  fail "guarded paths touched"
  printf '%s\n' "$bad" | show
fi

echo "== test count & ignore guard"
count_git_rs() {
  if [ -n "$2" ]; then
    git grep -F -c -e "$1" "$2" -- rust 2>/dev/null \
      | awk -F: 'NF >= 3 && $(NF-1) ~ /\.rs$/ { s += $NF } END { print s + 0 }'
  else
    git grep -F -c -e "$1" -- rust 2>/dev/null \
      | awk -F: 'NF >= 2 && $(NF-1) ~ /\.rs$/ { s += $NF } END { print s + 0 }'
  fi
}

count_wt_rs() {
  grep -F -r -c --include='*.rs' --exclude-dir=target -- "$1" rust 2>/dev/null \
    | awk -F: 'NF >= 2 { s += $NF } END { print s + 0 }'
}

base_tests=$(count_git_rs '#[test]' "$BASE")
head_tests=$(count_git_rs '#[test]' HEAD)
git_tests=$(count_git_rs '#[test]' "")
wt_tests=$(count_wt_rs '#[test]')

base_ign=$(count_git_rs '#[ignore' "$BASE")
head_ign=$(count_git_rs '#[ignore' HEAD)
git_ign=$(count_git_rs '#[ignore' "")
wt_ign=$(count_wt_rs '#[ignore')

if [ "$wt_tests" -lt "$base_tests" ] || [ "$head_tests" -lt "$base_tests" ] || [ "$git_tests" -lt "$base_tests" ]; then
  fail "#[test] count dropped below base (base=$base_tests head=$head_tests git=$git_tests wt=$wt_tests)"
else
  pass "#[test] count preserved (base=$base_tests head=$head_tests wt=$wt_tests)"
fi

if [ "$wt_ign" -gt "$base_ign" ] || [ "$head_ign" -gt "$base_ign" ] || [ "$git_ign" -gt "$base_ign" ]; then
  fail "#[ignore count increased above base (base=$base_ign head=$head_ign git=$git_ign wt=$wt_ign)"
else
  pass "#[ignore count not increased (base=$base_ign head=$head_ign wt=$wt_ign)"
fi

if [ "$SCOPE_ONLY" -eq 1 ]; then
  finish
fi

echo "== rust"
if ! command -v cargo >/dev/null 2>&1; then
  fail "cargo not found on PATH"
else
  if [ -z "${CARGO_TARGET_DIR:-}" ]; then
    OWN_TARGET_DIR="$PWD/rust/target/carbon-gate-$$"
    export CARGO_TARGET_DIR="$OWN_TARGET_DIR"
  fi

  if (cd rust && cargo fmt --all --check) > "$T/fmt.out" 2>&1; then
    pass "cargo fmt --all --check"
  else
    fail "cargo fmt --all --check"
    show 30 < "$T/fmt.out"
  fi

  if (cd rust && RUSTFLAGS="-D warnings" cargo build --locked --offline --release --all-targets) > "$T/build.out" 2>&1; then
    pass "cargo build --locked --offline --release --all-targets (zero warnings)"
  else
    fail "cargo build --locked --offline --release --all-targets"
    show 40 < "$T/build.out"
  fi

  (cd rust && env -u UPDATE_GOLDENS RUSTFLAGS="-D warnings" cargo test --locked --offline --release --no-fail-fast) > "$T/test.out" 2>&1
  test_rc=$?
  passed_total=$(awk '/^test result:/ { for (i = 1; i <= NF; i++) if ($i == "passed;" && i > 1) s += $(i-1) } END { print s + 0 }' "$T/test.out")
  failed_suites=$(awk '/test result: FAILED/ { c++ } END { print c + 0 }' "$T/test.out")
  if [ "$test_rc" -eq 0 ] && [ "$failed_suites" -eq 0 ] && [ "$passed_total" -gt 0 ]; then
    pass "cargo test --locked --offline --release --no-fail-fast ($passed_total passed, 0 failed)"
  else
    fail "cargo test --locked --offline --release --no-fail-fast (exit=$test_rc, passed=$passed_total, failed_suites=$failed_suites)"
    show 50 < "$T/test.out"
  fi
fi

echo "== python stdlib tests"
run_py_suite() {
  dir="$1"
  pat="$2"
  out="$T/py_$(printf '%s' "$dir" | tr '/' '_').out"
  if [ ! -d "$dir" ]; then
    fail "python suite directory missing: $dir"
    return
  fi
  python3 -m unittest discover -s "$dir" -p "$pat" > "$out" 2>&1
  py_rc=$?
  ran=$(awk '/^Ran [0-9]+ test/ { print $2; exit }' "$out")
  ran=${ran:-0}
  if [ "$py_rc" -eq 0 ] && [ "$ran" -gt 0 ]; then
    pass "python3 unittest $dir ($pat): $ran tests"
  else
    fail "python3 unittest $dir ($pat) (exit=$py_rc, ran=$ran)"
    show 30 < "$out"
  fi
}

if ! command -v python3 >/dev/null 2>&1; then
  fail "python3 not found on PATH"
else
  run_py_suite "blender/tests" "test_pure.py"
  run_py_suite "python/tests" "test_*.py"
  run_py_suite "tools/tests" "test_*.py"
fi

finish
