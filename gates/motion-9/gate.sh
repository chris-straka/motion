#!/usr/bin/env bash
# Gate for motion-9: Non-quadruped creature locomotion (two-legged monsters + legless body wave).
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

echo "== CLI smoke test (creature-clips --clips idle,walk,attack on biped fixture)"
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
  if "$CLI" fixture-glb --naming mixamo --output "$T/biped.glb" > "$T/fx.out" 2>&1 \
    && "$CLI" creature-profile --rig "$T/biped.glb" --output "$T/biped.json" > "$T/prof.out" 2>&1 \
    && "$CLI" creature-clips --creature "$T/biped.json" --glb-in "$T/biped.glb" --glb-out "$T/biped_clips.glb" --clips idle,walk,attack > "$T/clips.out" 2>&1; then
    pass "motionforge creature-clips idle,walk,attack on biped GLB"
  else
    fail "motionforge creature-clips idle,walk,attack on biped GLB"
    cat "$T/fx.out" "$T/prof.out" "$T/clips.out" 2>/dev/null | show 40
  fi

  if python3 - "$T/biped_clips.glb" > "$T/verify.out" 2>&1 <<'PY'
import json, struct, sys

data = open(sys.argv[1], "rb").read()
clen, ctype = struct.unpack_from("<II", data, 12)
assert ctype == 0x4E4F534A
gltf = json.loads(data[20:20 + clen].decode("utf-8"))
names = [a.get("name") for a in gltf.get("animations", [])]
assert set(names) >= {"idle", "walk", "attack"}, f"missing clips: {names}"
print("clips present:", names)
PY
  then
    pass "biped_clips.glb contains idle, walk, and attack animations"
  else
    fail "biped_clips.glb animation check"
    show 20 < "$T/verify.out"
  fi
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GATE PASS"
  exit 0
fi
echo "GATE FAIL ($FAILS)"
exit 1
