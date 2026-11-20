#!/usr/bin/env bash
# Forearm-wrap check (motion-6): verifies that twistless rigs never exceed the
# 80 deg forearm roll limit or collapse below 55% LBS cross-section area, and
# optionally measures forearm-band tears on any real rig without forearm twist
# bones using meshweights (`weights check --motion`).
#
# Usage:
#   tools/bench/forearm_wrap_check.sh [--rig <rig.glb>] [--weights <weights_bin>] [--max-forearm-cm2 <N>]
#
# Never hardcodes owner asset names or paths. Exits 0 only on pass.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RIG=""
WEIGHTS="${WEIGHTS_BIN:-}"
BIN="${MOTIONFORGE_BIN:-}"
MAX_FOREARM_CM2=""

while [ $# -gt 0 ]; do
  case "$1" in
    --rig) RIG="$2"; shift 2 ;;
    --weights) WEIGHTS="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --max-forearm-cm2) MAX_FOREARM_CM2="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }

echo "== synthetic forearm-wrap suite (cargo test --test motion_6_forearm)"
if (cd rust && cargo test --locked --release -p motion_core --test motion_6_forearm -- --test-threads=1); then
  pass "motion_6_forearm tests (<= 80 deg twistless roll, >= 55% LBS elbow ring area)"
else
  fail "motion_6_forearm tests"
fi

if [ -n "$RIG" ]; then
  if [ -z "$BIN" ]; then
    TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/rust/target}"
    BIN="$TARGET_DIR/release/motionforge"
    [ -x "$BIN" ] || (cd rust && cargo build --locked --release -p motionforge)
  fi
  if [ -z "$WEIGHTS" ]; then
    if command -v weights >/dev/null 2>&1; then
      WEIGHTS="$(command -v weights)"
    elif [ -x "$ROOT/../meshweights/rust/target/release/weights" ]; then
      WEIGHTS="$ROOT/../meshweights/rust/target/release/weights"
    fi
  fi
  if [ -z "$WEIGHTS" ] || [ ! -x "$WEIGHTS" ]; then
    fail "weights binary not found (pass --weights or set WEIGHTS_BIN)"
  elif [ ! -x "$BIN" ]; then
    fail "motionforge binary not found at $BIN"
  else
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    "$BIN" socket --input "$RIG" --output "$TMP/sock.glb" >/dev/null
    "$BIN" attack --rig "$TMP/sock.glb" --preset all --output "$TMP/atk.glb" >/dev/null
    "$WEIGHTS" check "$TMP/atk.glb" --motion "$TMP/atk.glb" --motion-clips cleave,backcut,sweep,thrust --json > "$TMP/w.json" || true
    if python3 - "$TMP/w.json" "$MAX_FOREARM_CM2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
max_cm2 = float(sys.argv[2]) if sys.argv[2] else None
h = d["headline"][0]
forearm_findings = [
    f for f in h["findings"]
    if any(w in f["part"].lower() for w in ("forearm", "wrist", "hand", "elbow"))
    and f["kind"] != "limb_through"
]
tot = sum(f["area_cm2"] for f in forearm_findings)
print(f"  tears_pct={h['tears_pct']:.2f}% forearm_non_through={tot:.1f}cm2 ({len(forearm_findings)} findings)")
for f in forearm_findings:
    print(f"    {f['code']:15s} {f['part']:28s} {f['area_cm2']:6.1f}cm2 {f['worst']}")
if max_cm2 is not None and tot > max_cm2 + 1e-6:
    print(f"  ERROR: forearm_non_through {tot:.1f} cm2 > limit {max_cm2:.1f} cm2")
    sys.exit(1)
PY
    then
      pass "real rig forearm-wrap check"
    else
      fail "real rig forearm-wrap check exceeded max forearm tears"
    fi
  fi
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "FOREARM WRAP CHECK PASS"
  exit 0
fi
echo "FOREARM WRAP CHECK FAIL ($FAILS failed)"
exit 1
