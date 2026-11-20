#!/usr/bin/env bash
# Gate for motion-7: Giant pass (`motionforge giant --scale k`).
set -u
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)" || exit 2

FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n "${1:-40}"; }

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

echo "== carbon.sh (scope, fmt, offline release build, all Rust & Python tests + CLI goldens)"
if bash tools/gate/carbon.sh > "$T/carbon.out" 2>&1; then
  pass "tools/gate/carbon.sh"
else
  fail "tools/gate/carbon.sh"
  show 60 < "$T/carbon.out"
fi

echo "== determinism check (no platform libm in rust/core/src/giant.rs)"
if [ ! -f rust/core/src/giant.rs ]; then
  fail "rust/core/src/giant.rs missing"
else
  # Reject direct f64 libm transcendental methods; require crate::detmath.
  if grep -En '\.(sqrt|sin|cos|tan|asin|acos|atan|atan2|powf|exp|ln)\(' rust/core/src/giant.rs > "$T/libm.out" 2>&1; then
    fail "rust/core/src/giant.rs calls platform libm (use crate::detmath)"
    show 20 < "$T/libm.out"
  else
    pass "rust/core/src/giant.rs uses detmath (zero platform libm calls)"
  fi
fi

echo "== CLI smoke test (motionforge giant + GLB extras + motion check)"
CLI="${CARGO_TARGET_DIR:-$PWD/rust/target}/release/motionforge"
if [ ! -x "$CLI" ]; then
  if (cd rust && cargo build --locked --offline --release --bin motionforge) > "$T/cli_build.out" 2>&1; then
    pass "built motionforge CLI"
  else
    fail "build motionforge CLI"
    show 30 < "$T/cli_build.out"
  fi
fi

if [ -x "$CLI" ]; then
  if "$CLI" fixture-glb --naming def --walk --output "$T/in.glb" > "$T/fx.out" 2>&1 \
    && "$CLI" giant --input "$T/in.glb" --output "$T/out.glb" --scale 4 --hit-stop 0.12 --cancel-window 0.25 > "$T/giant.out" 2>&1 \
    && "$CLI" giant --input "$T/in.glb" --output "$T/arms.glb" --scale 4 --mask arms-only --pin-pelvis > "$T/arms.out" 2>&1 \
    && "$CLI" motion check "$T/out.glb" > "$T/mc.out" 2>&1; then
    pass "motionforge giant CLI (full + arms-only + motion check)"
  else
    fail "motionforge giant CLI smoke test"
    cat "$T/fx.out" "$T/giant.out" "$T/arms.out" "$T/mc.out" 2>/dev/null | show 40
  fi

  if python3 - "$T/out.glb" "$T/arms.glb" > "$T/extras.out" 2>&1 <<'PY'
import json, struct, sys

def read_gltf(path):
    data = open(path, "rb").read()
    clen, ctype = struct.unpack_from("<II", data, 12)
    assert ctype == 0x4E4F534A
    return json.loads(data[20:20 + clen].decode("utf-8"))

g = read_gltf(sys.argv[1])
ex = g["animations"][0]["extras"]["motionforge_giant"]
assert abs(ex["scale"] - 4.0) < 1e-6, ex
kinds = {e["kind"] for e in ex["events"]}
assert "hit_stop" in kinds and "cancel_window" in kinds, ex

a = read_gltf(sys.argv[2])
aex = a["animations"][0]["extras"]["motionforge_giant"]
assert aex["mask"] == "arms-only" and aex["pin_pelvis"] is True, aex
print("extras verified:", ex["events"], "arms mask:", aex["mask"])
PY
  then
    pass "GLB extras.motionforge_giant verified on CLI outputs"
  else
    fail "GLB extras.motionforge_giant verification"
    show 20 < "$T/extras.out"
  fi
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS)"
exit 1
