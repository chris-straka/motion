#!/usr/bin/env bash
# Width-refit & extended contact check (motion-13, docs/width-refit.md):
#   1. Runs the synthetic unit suite (rust/core/tests/width_refit.rs):
#      - shin + upper_arm proxies present;
#      - 0.30 m chest source -> 1.4x wide target clears torso, thighs, shins,
#        and arm-vs-arm contacts while keeping wrist peak-speed frame, ramp
#        locality, and untouched legs;
#      - weapon-hand arm-vs-arm moves the free hand first;
#      - same-width target + source-body faults stay unchanged.
#   2. Verifies CLI `motionforge animate` accepts `--refit-width` and
#      `--no-refit-width`.
#   3. Optional real-rig measurement with meshweights (`weights check --motion`):
#      tools/bench/width_refit_check.sh --rig <target.glb> --clips <src.glb> [--weights <bin>] [--max-through-cm2 <N>]
#
# Never hardcodes owner asset names or paths. Exits 0 only on pass.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RIG=""
CLIPS=""
WEIGHTS="${WEIGHTS_BIN:-}"
BIN="${MOTIONFORGE_BIN:-}"
MAX_THROUGH_CM2=""

while [ $# -gt 0 ]; do
  case "$1" in
    --rig) RIG="$2"; shift 2 ;;
    --clips) CLIPS="$2"; shift 2 ;;
    --weights) WEIGHTS="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --max-through-cm2) MAX_THROUGH_CM2="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }

echo "== synthetic width-refit suite (cargo test --test width_refit)"
if (cd rust && cargo test --locked --release -p motion_core --test width_refit -- --test-threads=1); then
  pass "width_refit core tests"
else
  fail "width_refit core tests"
fi

echo "== CLI --refit-width / --no-refit-width flags"
if [ -z "$BIN" ]; then
  TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/rust/target}"
  BIN="$TARGET_DIR/release/motionforge"
  [ -x "$BIN" ] || (cd rust && cargo build --locked --release -p motionforge)
fi
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
if [ ! -x "$BIN" ]; then
  fail "motionforge binary not found at $BIN"
else
  "$BIN" fixture-glb --naming def --walk --output "$TMP/src.glb" >/dev/null
  "$BIN" fixture-glb --naming def --output "$TMP/tgt.glb" >/dev/null
  if "$BIN" animate --input "$TMP/tgt.glb" --clips "$TMP/src.glb" --refit-width --output "$TMP/out_on.glb" >/dev/null 2>&1 \
    && "$BIN" animate --input "$TMP/tgt.glb" --clips "$TMP/src.glb" --no-refit-width --output "$TMP/out_off.glb" >/dev/null 2>&1; then
    pass "motionforge animate accepts --refit-width and --no-refit-width"
  else
    fail "motionforge animate rejected --refit-width or --no-refit-width"
  fi
fi

if [ -n "$RIG" ] && [ -n "$CLIPS" ]; then
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
    "$BIN" animate --input "$RIG" --clips "$CLIPS" --refit-width --output "$TMP/real_out.glb" >/dev/null
    "$WEIGHTS" check "$TMP/real_out.glb" --motion "$TMP/real_out.glb" --json > "$TMP/w.json" || true
    if python3 - "$TMP/w.json" "$MAX_THROUGH_CM2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
max_cm2 = float(sys.argv[2]) if sys.argv[2] else None
h = d["headline"][0]
through = [f for f in h["findings"] if f["kind"] == "limb_through"]
tot = sum(f["area_cm2"] for f in through)
print(f"  tears_pct={h['tears_pct']:.2f}% limb_through={tot:.1f}cm2 ({len(through)} findings)")
for f in through:
    print(f"    {f['code']:15s} {f['part']:28s} {f['area_cm2']:6.1f}cm2 {f['worst']}")
if max_cm2 is not None and tot > max_cm2 + 1e-6:
    print(f"  ERROR: limb_through {tot:.1f} cm2 > limit {max_cm2:.1f} cm2")
    sys.exit(1)
PY
    then
      pass "real rig width-refit limb-through check"
    else
      fail "real rig width-refit exceeded max limb-through"
    fi
  fi
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "WIDTH REFIT CHECK PASS"
  exit 0
fi
echo "WIDTH REFIT CHECK FAIL ($FAILS failed)"
exit 1
