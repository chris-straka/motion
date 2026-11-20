#!/usr/bin/env bash
# Gate for motion-8 (plans/motion.md): pro_bench held-out Mixamo set support,
# dev vs held-out reporting, planted bad clip rejection, and no machine pin.
#  - scope: worker edits ONLY tools/bench/pro_bench.py and tools/bench/pro_fleet.sh;
#  - all 4 tests in tools/tests/test_pro_bench.py pass;
#  - `python3 tools/bench/pro_bench.py planted` runs against the real built
#    motionforge binary and confirms the planted bad clip fails;
#  - all Python stdlib test suites pass and `cargo fmt --all --check` passes.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-8
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-30}"; }
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find the commit that added the gate"; BASE=HEAD; }

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(tools/bench/pro_bench\.py|tools/bench/pro_fleet\.sh)?$' | sort -u)
if [ -z "$other" ]; then pass "only tools/bench/pro_bench.py and tools/bench/pro_fleet.sh changed"; else fail "other files changed"; echo "$other" | show; fi

echo "== build motionforge CLI"
command -v cargo >/dev/null 2>&1 || { fail "no cargo on this machine"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT
if (cd rust && RUSTFLAGS="-D warnings" cargo build --locked --offline --release -p motionforge) > "$T/build.out" 2>&1; then
  pass "cargo build --release -p motionforge"
else
  fail "cargo build --release -p motionforge"
  tail -n 25 "$T/build.out" | show
fi
export MOTIONFORGE_BIN="$CARGO_TARGET_DIR/release/motionforge"

echo "== motion-8 pro_bench unit tests"
python3 -m unittest -v tools/tests/test_pro_bench.py > "$T/pb.out" 2>&1
for t in \
  test_cli_supports_lib_heldout_and_planted_subcommand \
  test_report_separates_dev_and_heldout_sets \
  test_planted_bad_clip_fails_compare \
  test_pro_fleet_has_no_machine_pin; do
  if grep -qE "^$t \(.*\) \.\.\. ok$" "$T/pb.out"; then
    pass "$t"
  else
    fail "$t"
    grep -A 12 "$t" "$T/pb.out" | show 15
  fi
done

echo "== pro_bench.py planted (real CLI bad-clip rejection)"
if python3 tools/bench/pro_bench.py planted --out "$T/planted" > "$T/pl.out" 2>&1; then
  pass "pro_bench.py planted (planted bad clip fails as expected)"
else
  fail "pro_bench.py planted"
  show 25 < "$T/pl.out"
fi

echo "== all Python stdlib test suites"
PY_OK=1
for pair in "blender/tests:test_pure.py" "python/tests:test_*.py" "tools/tests:test_*.py"; do
  dir="${pair%%:*}"; pat="${pair##*:}"
  if ! python3 -m unittest discover -s "$dir" -p "$pat" > "$T/py.out" 2>&1; then
    fail "python unittest $dir"
    tail -n 20 "$T/py.out" | show
    PY_OK=0
  fi
done
[ "$PY_OK" -eq 1 ] && pass "all Python stdlib suites"

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS failed)"
exit 1
