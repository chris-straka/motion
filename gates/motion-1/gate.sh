#!/usr/bin/env bash
# Gate for motion-1 (plans/motion.md): grip measure read from the hand's skin.
#  - scope: worker edits only rust/core/src/ (never tests, tools/bench/grip_gate.sh,
#    gates/motion-1/, Cargo files, or docs);
#  - no #[test] removed or #[ignore] added;
#  - cargo fmt --all --check;
#  - cargo build --locked --release --all-targets with zero warnings;
#  - detmath determinism guard passes (no platform libm calls like .atan2()/.sin());
#  - all 4 skin-grip tests in rust/core/tests/skin_grip.rs pass;
#  - existing suite passes (skipping only motion-2's two known red tests);
#  - tools/bench/grip_gate.sh passes.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-1
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
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(rust/core/src/.*)?$' | sort -u)
if [ -z "$other" ]; then pass "only rust/core/src/ changed"; else fail "other files changed"; echo "$other" | show; fi

echo "== tests kept"
nt=$(git grep -c '#\[test\]' -- rust | awk -F: '{s+=$NF} END {print s+0}')
nb=$(git grep -c '#\[test\]' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
ni=$(git grep -c '#\[ignore' -- rust | awk -F: '{s+=$NF} END {print s+0}')
nib=$(git grep -c '#\[ignore' "$BASE" -- rust | awk -F: '{s+=$NF} END {print s+0}')
if [ "$nt" -ge "$nb" ] && [ "$ni" -le "$nib" ]; then pass "tests $nt (base $nb), ignored $ni (base $nib)"; else fail "tests $nt (base $nb), ignored $ni (base $nib)"; fi

echo "== format and build"
command -v cargo >/dev/null 2>&1 || { fail "no cargo on this machine"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT

if (cd rust && cargo fmt --all --check) > "$T/fmt.out" 2>&1; then
  pass "cargo fmt --all --check"
else
  fail "cargo fmt --all --check"
  show 10 < "$T/fmt.out"
fi

(cd rust && cargo build --locked --release --all-targets) > "$T/build.out" 2>&1
bc=$?
nw=$(grep -cE '^warning' "$T/build.out")
if [ $bc -eq 0 ] && [ "$nw" -eq 0 ]; then
  pass "cargo build --locked --release --all-targets, zero warnings"
else
  fail "cargo build --locked --release --all-targets"
  grep -E '^(warning|error)' -A3 "$T/build.out" | show 20
fi

echo "== detmath determinism check"
(cd rust && cargo test --locked --release -p motion_core --lib detmath) > "$T/det.out" 2>&1
if [ $? -eq 0 ] && grep -qE '^test result: ok\.' "$T/det.out"; then
  pass "detmath determinism tests"
else
  fail "detmath determinism tests"
  show 15 < "$T/det.out"
fi

echo "== skin_grip tests"
(cd rust && cargo test --locked --release -p motion_core --test skin_grip -- --test-threads=1) > "$T/sg.out" 2>&1
for t in skin_grip_pure_scoring_and_constants \
  skin_grip_measures_closed_vs_three_planted_bad_cases_and_json \
  skin_grip_flat_palm_without_fingers_fails_on_circumference_wrap \
  skin_grip_distal_mitten_reads_palm_skin_not_bone_ratio; do
  if grep -qE "^test $t \.\.\. ok$" "$T/sg.out"; then
    pass "$t"
  else
    fail "$t"
    awk -v t="---- $t stdout ----" '$0 == t {p = 1} p' "$T/sg.out" | show 10
  fi
done

echo "== existing test suite (excluding motion-2's two pre-existing red tests)"
(cd rust && cargo test --locked --release --no-fail-fast -- \
  --skip single_finger_chain_places_palm_socket_and_curls_round_the_handle \
  --skip distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket) > "$T/suite.out" 2>&1
tc=$?
nfail=$(grep -E '^test result: FAILED' -c "$T/suite.out")
if [ $tc -eq 0 ] && [ "$nfail" -eq 0 ]; then
  pass "existing rust test suite green"
else
  fail "existing rust test suite green"
  grep -E '^(test .* FAILED|failures:|error)' "$T/suite.out" | show 20
fi

echo "== grip_gate.sh"
if MOTIONFORGE_BIN="$CARGO_TARGET_DIR/release/motionforge" bash tools/bench/grip_gate.sh > "$T/gg.out" 2>&1; then
  pass "tools/bench/grip_gate.sh"
else
  fail "tools/bench/grip_gate.sh"
  show 20 < "$T/gg.out"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS failed)"
exit 1
