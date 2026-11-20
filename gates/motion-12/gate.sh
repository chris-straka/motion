#!/usr/bin/env bash
# Gate for motion-12: Code-health sweep under tools/gate/carbon.sh.
set -u
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)" || exit 2

FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${1:-40}"; }

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

echo "== 1. carbon.sh (guarded scope, fmt, offline release build -D warnings, all tests + byte-identical CLI goldens)"
if bash tools/gate/carbon.sh > "$T/carbon.out" 2>&1; then
  pass "tools/gate/carbon.sh"
else
  fail "tools/gate/carbon.sh"
  show 60 < "$T/carbon.out"
fi

echo "== 2. One distal-mitten predicate inside rust/core/src/attack/grip.rs"
mitten_hits=$(grep -En 'palm_len\s*>\s*2\.0\s*\*\s*finger_len' rust/core/src/attack/grip.rs | grep -v '^\s*[0-9]*:\s*//' | wc -l | tr -d ' ')
if [ "$mitten_hits" -eq 1 ]; then
  pass "single distal-mitten predicate in rust/core/src/attack/grip.rs (count=1)"
else
  fail "expected exactly 1 distal-mitten predicate in rust/core/src/attack/grip.rs, found $mitten_hits"
  grep -En 'palm_len\s*>\s*2\.0\s*\*\s*finger_len' rust/core/src/attack/grip.rs | show 10
fi

echo "== 3. Zero partial_cmp in unguarded Rust source files (use f64::total_cmp)"
if grep -rn 'partial_cmp' rust/core/src rust/cli/src \
  --exclude='motioncheck.rs' --exclude='readcam.rs' > "$T/pcmp.out" 2>&1; then
  fail "partial_cmp still present in unguarded Rust sources"
  show 30 < "$T/pcmp.out"
else
  pass "zero partial_cmp in unguarded Rust sources"
fi

echo "== 4. No DEF-forearm_twist special case in rust/core/src/contact.rs"
if grep -En 'DEF-forearm_twist' rust/core/src/contact.rs > "$T/ft.out" 2>&1; then
  fail "DEF-forearm_twist special case still present in rust/core/src/contact.rs"
  show 10 < "$T/ft.out"
else
  pass "contact.rs routes forearm_twist through helpers table"
fi

echo "== 5. helpers.rs caches rig mapping (at most 1 humanoid::map_rig call site, and reusable per-rig driver)"
map_sites=$(grep -En 'humanoid::map_rig' rust/core/src/helpers.rs | wc -l | tr -d ' ')
if [ "$map_sites" -le 1 ] && grep -Eq 'pub struct (Drivers|HelperMap|HelperSet|RigHelpers)' rust/core/src/helpers.rs; then
  pass "helpers.rs caches map_rig in a reusable per-rig helper struct (map_rig sites=$map_sites)"
else
  fail "helpers.rs must expose a reusable per-rig helper struct and call humanoid::map_rig in at most 1 helper (found map_rig sites=$map_sites)"
  grep -En 'humanoid::map_rig|pub struct ' rust/core/src/helpers.rs | show 15
fi

echo "== 6. No --on machine pin in pro_fleet.sh or docs/pro-bench.md"
if grep -En -- '--on ' tools/bench/pro_fleet.sh docs/pro-bench.md > "$T/on.out" 2>&1; then
  fail "--on machine pin still present"
  show 10 < "$T/on.out"
else
  pass "no --on machine pin in pro_fleet.sh or docs/pro-bench.md"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS)"
exit 1
