#!/usr/bin/env bash
# Gate for motion-3 (plans/motion.md): a gate for Carbon (`tools/gate/carbon.sh`).
#  - scope: worker creates/edits ONLY tools/gate/carbon.sh (never planted.sh,
#    gates/motion-3/, rust/, tests/, or other files);
#  - tools/gate/carbon.sh exists and is executable;
#  - bash tools/gate/planted.sh prints PLANTED PASS (fails on every guarded path,
#    removed #[test], added #[ignore], and uncommitted guarded edit; passes on control);
#  - bash tools/gate/carbon.sh runs cleanly on the clean tree (fmt, full Rust suite
#    including CLI contract goldens, zero warnings) and prints GATE PASS.
# Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=motion-3
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
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^(tools/gate/carbon\.sh)?$' | sort -u)
if [ -z "$other" ]; then pass "only tools/gate/carbon.sh changed"; else fail "other files changed"; echo "$other" | show; fi

if git diff --quiet "$BASE" -- tools/gate/planted.sh gates/$SLUG rust tests; then
  pass "planted.sh, gate, rust/, and tests/ unchanged since gate base"
else
  fail "protected files changed since gate base"
fi

echo "== tools/gate/carbon.sh exists and is executable"
if [ -x tools/gate/carbon.sh ]; then
  pass "tools/gate/carbon.sh exists and is executable"
else
  fail "tools/gate/carbon.sh missing or not executable"
fi

echo "== planted-diff proof (tools/gate/planted.sh)"
if bash tools/gate/planted.sh > "$T/planted.out" 2>&1 && tail -n 1 "$T/planted.out" | grep -q '^PLANTED PASS$'; then
  pass "tools/gate/planted.sh (every guarded path fails, control passes)"
else
  fail "tools/gate/planted.sh"
  show 30 < "$T/planted.out"
fi

echo "== clean-tree full run (tools/gate/carbon.sh)"
command -v cargo >/dev/null 2>&1 || { fail "no cargo on this machine"; echo "GATE FAIL ($FAILS)"; exit 1; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/rust/target}/gate-$(git rev-parse --short=12 HEAD)-$$"
trap 'rm -rf "$T" "$CARGO_TARGET_DIR"' EXIT
if [ -x tools/gate/carbon.sh ]; then
  # Pass GATE_BASE=HEAD so the commit adding carbon.sh itself is not treated as a guarded edit
  if GATE_BASE="$(git rev-parse HEAD)" bash tools/gate/carbon.sh > "$T/carbon.out" 2>&1 && tail -n 1 "$T/carbon.out" | grep -q '^GATE PASS$'; then
    pass "tools/gate/carbon.sh passes on clean tree (fmt + full suite + CLI goldens)"
  else
    fail "tools/gate/carbon.sh failed on clean tree"
    show 30 < "$T/carbon.out"
  fi
else
  fail "tools/gate/carbon.sh failed on clean tree"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS failed)"
exit 1
