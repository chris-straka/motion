#!/usr/bin/env bash
# Gate for motion-2 (plans/motion.md): make the two red tests honest and CI green.
#  - scope: worker edits only rust/core/src/attack/ (never tests, readcam.rs,
#    gates/motion-2/, tools/bench/, Cargo files, or docs);
#  - no tolerance loosened (attack.rs keeps m.grip < 1e-9; tests/ and readcam.rs
#    unchanged since BASE);
#  - no #[test] removed or #[ignore] added;
#  - cargo fmt --all --check;
#  - RUSTFLAGS="-D warnings" cargo build --locked --release --all-targets;
#  - detmath determinism tests pass;
#  - single_finger_chain_places_palm_socket_and_curls_round_the_handle passes;
#  - distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket passes;
#  - ENTIRE cargo test --locked --release --no-fail-fast passes with 0 skipped, 0 failed;
#  - tools/bench/grip_gate.sh passes;
#  - Python stdlib test suites (matching .github/workflows/ci.yml) pass.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-2
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
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(rust/core/src/attack/.*)?$' | sort -u)
if [ -z "$other" ]; then pass "only rust/core/src/attack/ changed"; else fail "other files changed"; echo "$other" | show; fi

echo "== protected tolerances and tests"
if git diff --quiet "$BASE" -- rust/core/tests rust/cli/tests rust/core/src/readcam.rs; then
  pass "tests and readcam.rs unchanged since gate base"
else
  fail "tests or readcam.rs changed since gate base"
fi
if grep -q 'assert!(m.grip < 1e-9' rust/core/tests/attack.rs; then
  pass "attack.rs keeps strict 1e-9 grip tolerance"
else
  fail "attack.rs lost 1e-9 grip tolerance"
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

echo "== the two restored tests"
(cd rust && cargo test --locked --release -p motion_core --test attack single_finger_chain_places_palm_socket_and_curls_round_the_handle -- --exact) > "$T/t1.out" 2>&1
if grep -qE '^test single_finger_chain_places_palm_socket_and_curls_round_the_handle \.\.\. ok$' "$T/t1.out"; then
  pass "single_finger_chain_places_palm_socket_and_curls_round_the_handle"
else
  fail "single_finger_chain_places_palm_socket_and_curls_round_the_handle"
  show 15 < "$T/t1.out"
fi

(cd rust && cargo test --locked --release -p motion_core --test readcam distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket -- --exact) > "$T/t2.out" 2>&1
if grep -qE '^test distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket \.\.\. ok$' "$T/t2.out"; then
  pass "distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket"
else
  fail "distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket"
  show 15 < "$T/t2.out"
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

echo "== python stdlib tests (CI parity)"
if python3 -m unittest discover -s blender/tests -p "test_pure.py" > "$T/py.out" 2>&1 \
  && python3 -m unittest discover -s python/tests -p "test_*.py" >> "$T/py.out" 2>&1 \
  && python3 -m unittest discover -s tools/tests -p "test_*.py" >> "$T/py.out" 2>&1; then
  pass "python stdlib unittest suites"
else
  fail "python stdlib unittest suites"
  show 20 < "$T/py.out"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS failed)"
exit 1
