#!/usr/bin/env bash
# Gate for motion-4 (plans/motion.md): close distal-mitten hand on the hilt.
#  - scope: worker edits only rust/core/src/attack/grip.rs (never readcam.rs,
#    tests, gates, tools/bench/, Cargo files, or docs);
#  - git diff empty on rust/core/src/readcam.rs (the measure must not change);
#  - no #[test] removed or #[ignore] added;
#  - cargo fmt --all --check;
#  - RUSTFLAGS="-D warnings" cargo build --locked --release --all-targets;
#  - detmath determinism tests pass;
#  - distal_mitten_cupped_hand_reaches_0_9_on_all_four_presets passes;
#  - ENTIRE cargo test --locked --release --no-fail-fast passes with 0 skips/failures;
#  - tools/bench/grip_gate.sh passes.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-4
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${2:-30}"; }
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find the commit that added the gate"; BASE=HEAD; }

echo "== scope and untouched measure"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(rust/core/src/attack/grip\.rs)?$' | sort -u)
if [ -z "$other" ]; then pass "only rust/core/src/attack/grip.rs changed"; else fail "other files changed"; echo "$other" | show; fi

if git diff --quiet "$BASE" -- rust/core/src/readcam.rs rust/core/tests rust/cli/tests tools/bench/grip_gate.sh; then
  pass "readcam.rs, tests, and grip_gate.sh unchanged since gate base"
else
  fail "readcam.rs, tests, or grip_gate.sh changed since gate base"
fi

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

(cd rust && RUSTFLAGS="-D warnings" cargo build --locked --release --all-targets) > "$T/build.out" 2>&1
if [ $? -eq 0 ]; then
  pass "RUSTFLAGS=-D warnings cargo build --locked --release --all-targets"
else
  fail "RUSTFLAGS=-D warnings cargo build --locked --release --all-targets"
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

echo "== motion-4 distal-mitten grip test"
(cd rust && cargo test --locked --release -p motion_core --test motion_4_grip -- --test-threads=1) > "$T/m4.out" 2>&1
if grep -qE '^test distal_mitten_cupped_hand_reaches_0_9_on_all_four_presets \.\.\. ok$' "$T/m4.out"; then
  pass "distal_mitten_cupped_hand_reaches_0_9_on_all_four_presets"
else
  fail "distal_mitten_cupped_hand_reaches_0_9_on_all_four_presets"
  show 20 < "$T/m4.out"
fi

echo "== full rust test suite (zero skips, zero failures)"
(cd rust && RUSTFLAGS="-D warnings" cargo test --locked --release --no-fail-fast) > "$T/suite.out" 2>&1
tc=$?
nfail=$(grep -E '^test result: FAILED' -c "$T/suite.out")
if [ $tc -eq 0 ] && [ "$nfail" -eq 0 ]; then
  pass "full rust test suite green"
else
  fail "full rust test suite green"
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
