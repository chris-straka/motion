#!/usr/bin/env bash
# Gate for motion-6 (plans/motion.md): forearm roll 105 deg measured -> restore 80 deg.
#  - scope: worker edits ONLY rust/core/src/attack/mod.rs;
#  - tools/gate/carbon.sh passes (guarded paths untouched, test count preserved,
#    fmt, zero warnings, full Rust suite + CLI goldens, Python stdlib tests);
#  - both tests in rust/core/tests/motion_6_forearm.rs pass;
#  - tools/bench/forearm_wrap_check.sh passes.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-6
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
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(rust/core/src/attack/mod\.rs)?$' | sort -u)
if [ -z "$other" ]; then pass "only rust/core/src/attack/mod.rs changed"; else fail "other files changed"; echo "$other" | show; fi

echo "== motion-6 forearm tests"
command -v cargo >/dev/null 2>&1 || { fail "no cargo on this machine"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT

(cd rust && cargo test --locked --release -p motion_core --test motion_6_forearm -- --test-threads=1) > "$T/m6.out" 2>&1
for t in forearm_roll_cap_is_at_most_80_deg_and_twistless_attacks_respect_it \
  lbs_elbow_ring_cross_section_does_not_collapse_below_half_area; do
  if grep -qE "^test $t \.\.\. ok$" "$T/m6.out"; then
    pass "$t"
  else
    fail "$t"
    awk -v t="---- $t stdout ----" '$0 == t {p = 1} p' "$T/m6.out" | show 10
  fi
done

echo "== guarded Carbon gate (tools/gate/carbon.sh)"
if GATE_BASE="$BASE" bash tools/gate/carbon.sh > "$T/carbon.out" 2>&1 && tail -n 1 "$T/carbon.out" | grep -q '^GATE PASS$'; then
  pass "tools/gate/carbon.sh (scope + fmt + build + full suite + goldens + python)"
else
  fail "tools/gate/carbon.sh"
  grep -E '^(FAIL|test .* FAILED|error)' "$T/carbon.out" | show 25
fi

echo "== forearm_wrap_check.sh"
if bash tools/bench/forearm_wrap_check.sh > "$T/fw.out" 2>&1 && tail -n 1 "$T/fw.out" | grep -q '^FOREARM WRAP CHECK PASS$'; then
  pass "tools/bench/forearm_wrap_check.sh"
else
  fail "tools/bench/forearm_wrap_check.sh"
  show 20 < "$T/fw.out"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS failed)"
exit 1
