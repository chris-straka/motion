#!/usr/bin/env bash
# Gate for motion-13 (docs/width-refit.md): refit clips to a body's width +
# extended contact pass (shins, upper arms, arm-vs-arm, source slack).
#  - scope: worker edits ONLY rust/core/src/{animate,contact,width_refit,lib}.rs
#    and rust/cli/src/{main,glb_cmds}.rs;
#  - all 5 tests in rust/core/tests/width_refit.rs pass;
#  - tools/gate/carbon.sh passes (guarded paths untouched, test count preserved,
#    fmt, zero warnings, full Rust suite + CLI goldens, Python stdlib tests);
#  - tools/bench/width_refit_check.sh passes (including CLI --refit-width / --no-refit-width).
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-13
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
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(rust/core/src/(animate|contact|width_refit|lib)\.rs|rust/cli/src/(main|glb_cmds)\.rs)?$' | sort -u)
if [ -z "$other" ]; then pass "only allowed motion-13 source files changed"; else fail "other files changed"; echo "$other" | show; fi

echo "== motion-13 width_refit tests"
command -v cargo >/dev/null 2>&1 || { fail "no cargo on this machine"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT

(cd rust && cargo test --locked --release -p motion_core --test width_refit -- --test-threads=1) > "$T/wr.out" 2>&1
for t in \
  body_proxies_include_shins_and_upper_arms \
  wide_target_clears_torso_legs_and_arm_crossing_while_keeping_timing_and_ramp \
  arm_vs_shin_clears_without_moving_leg \
  arm_vs_arm_weapon_moves_free_hand_first \
  same_width_target_and_source_fault_stay_unchanged; do
  if grep -qE "^test $t \.\.\. ok$" "$T/wr.out"; then
    pass "$t"
  else
    fail "$t"
    awk -v t="---- $t stdout ----" '$0 == t {p = 1} p' "$T/wr.out" | show 12
  fi
done

echo "== guarded Carbon gate (tools/gate/carbon.sh)"
if GATE_BASE="$BASE" bash tools/gate/carbon.sh > "$T/carbon.out" 2>&1 && tail -n 1 "$T/carbon.out" | grep -q '^GATE PASS$'; then
  pass "tools/gate/carbon.sh (scope + fmt + build + full suite + goldens + python)"
else
  fail "tools/gate/carbon.sh"
  grep -E '^(FAIL|test .* FAILED|error)' "$T/carbon.out" | show 25
fi

echo "== width_refit_check.sh"
if bash tools/bench/width_refit_check.sh > "$T/wrc.out" 2>&1 && tail -n 1 "$T/wrc.out" | grep -q '^WIDTH REFIT CHECK PASS$'; then
  pass "tools/bench/width_refit_check.sh"
else
  fail "tools/bench/width_refit_check.sh"
  show 25 < "$T/wrc.out"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS failed)"
exit 1
