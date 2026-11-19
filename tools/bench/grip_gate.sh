#!/usr/bin/env bash
# Grip gate (motion-1): verifies the skin-based grip measure on synthetic
# procedural rigs (closed five-finger, flat palm, distal mitten, and planted
# bad cases: open hand, fingertip socket, handle 10 cm off) and optionally on
# real character rigs supplied via flags or env vars:
#
#   tools/bench/grip_gate.sh [--five-finger <rig.glb>] [--single-chain <rig.glb>]
#
# Never hardcodes owner asset names or paths. Exits 0 only when every check
# passes (five-finger closed >= 0.90; every planted bad case < 0.30).
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

FIVE_FINGER="${GRIP_FIVE_FINGER:-}"
SINGLE_CHAIN="${GRIP_SINGLE_CHAIN:-}"
BIN="${MOTIONFORGE_BIN:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --five-finger) FIVE_FINGER="$2"; shift 2 ;;
    --single-chain) SINGLE_CHAIN="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }

echo "== synthetic skin-grip suite (cargo test --test skin_grip)"
if (cd rust && cargo test --locked --release -p motion_core --test skin_grip -- --test-threads=1); then
  pass "skin_grip unit suite (closed >= 0.90, planted bad cases < 0.30, flat palm, distal mitten)"
else
  fail "skin_grip unit suite"
fi

if [ -z "$BIN" ]; then
  TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/rust/target}"
  BIN="$TARGET_DIR/release/motionforge"
  if [ ! -x "$BIN" ]; then
    (cd rust && cargo build --locked --release -p motionforge)
  fi
fi

if [ ! -x "$BIN" ]; then
  fail "motionforge binary not found at $BIN"
  echo "GRIP GATE FAIL ($FAILS)"
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "== CLI JSON schema check on procedural fixture"
"$BIN" fixture-glb --output "$TMP/fix.glb" --naming plain --walk >/dev/null
"$BIN" standardize --input "$TMP/fix.glb" --output "$TMP/std.glb" >/dev/null
"$BIN" attack --rig "$TMP/std.glb" --preset sweep --output "$TMP/atk.glb" >/dev/null
"$BIN" motion check --input "$TMP/atk.glb" --clip sweep --camera game --blade-pose socket --no-through --json > "$TMP/fix_check.json"
if python3 - "$TMP/fix_check.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
g = d["clips"][0]["read_cam"]["grip"]
for k in ("score", "palm_surface_cm", "skin_wrap", "skin_based"):
    assert k in g, f"missing key {k} in grip json: {g}"
assert g["skin_based"] is True, f"expected skin_based=True: {g}"
print(f"fixture grip json ok: score={g['score']} palm_surface_cm={g['palm_surface_cm']} skin_wrap={g['skin_wrap']}")
PY
then
  pass "CLI JSON exposes palm_surface_cm, skin_wrap, skin_based"
else
  fail "CLI JSON missing skin-grip fields"
fi

check_real_rig() {
  local label="$1"
  local rig="$2"
  local min_closed="$3"
  local dir="$TMP/$label"
  mkdir -p "$dir"
  "$BIN" socket --input "$rig" --output "$dir/sock.glb" >/dev/null
  "$BIN" attack --rig "$dir/sock.glb" --preset sweep --output "$dir/closed.glb" >/dev/null
  # Planted cases built via stdlib Python (open hand spec, fingertip socket, +10 cm off)
  python3 - "$BIN" "$dir" <<'PY'
import json, math, os, struct, subprocess, sys

BIN, DIR = sys.argv[1], sys.argv[2]

def load_glb(path):
    data = open(path, "rb").read()
    jlen = struct.unpack_from("<I", data, 12)[0]
    g = json.loads(data[20:20+jlen])
    boff = 20 + jlen
    blen = struct.unpack_from("<I", data, boff)[0]
    buf = data[boff+8:boff+8+blen]
    return g, buf

def save_glb(path, g, buf):
    jb = json.dumps(g, separators=(",", ":")).encode("utf-8")
    while len(jb) % 4: jb += b" "
    bb = buf
    while len(bb) % 4: bb += b"\x00"
    total = 12 + 8 + len(jb) + 8 + len(bb)
    out = bytearray()
    out += struct.pack("<4sII", b"glTF", 2, total)
    out += struct.pack("<I4s", len(jb), b"JSON") + jb
    out += struct.pack("<I4s", len(bb), b"BIN\x00") + bb
    open(path, "wb").write(out)

def read_trs_mat(nd):
    t = nd.get("translation", [0.0, 0.0, 0.0])
    r = nd.get("rotation", [0.0, 0.0, 0.0, 1.0])
    s = nd.get("scale", [1.0, 1.0, 1.0])
    x,y,z,w = r; sx,sy,sz = s
    return [(1-2*(y*y+z*z))*sx, (2*(x*y+z*w))*sx, (2*(x*z-y*w))*sx, 0,
            (2*(x*y-z*w))*sy, (1-2*(x*x+z*z))*sy, (2*(y*z+x*w))*sy, 0,
            (2*(x*z+y*w))*sz, (2*(y*z-x*w))*sz, (1-2*(x*x+y*y))*sz, 0,
            t[0], t[1], t[2], 1]

def mat_mul(A, B):
    return [sum(A[k*4+r]*B[c*4+k] for k in range(4)) for c in range(4) for r in range(4)]

def qrot(q, v):
    x,y,z,w = q; vx,vy,vz = v
    tx = 2*(y*vz - z*vy); ty = 2*(z*vx - x*vz); tz = 2*(x*vy - y*vx)
    return [vx + w*tx + y*tz - z*ty, vy + w*ty + z*tx - x*tz, vz + w*tz + x*ty - y*tx]

# 1. Open hand: remove finger animation channels from closed.glb so fingers stay at rest (curl = 0)
g, buf = load_glb(os.path.join(DIR, "closed.glb"))
nodes = g["nodes"]
hand_i = next(i for i, nd in enumerate(nodes) if nd.get("name") == "DEF-hand.R")
def desc(i):
    s = set()
    for c in nodes[i].get("children", []):
        if not nodes[c].get("name", "").startswith("Socket_"):
            s.add(c); s |= desc(c)
    return s
finger_nodes = desc(hand_i)
for anim in g.get("animations", []):
    anim["channels"] = [ch for ch in anim["channels"] if ch["target"]["node"] not in finger_nodes]
save_glb(os.path.join(DIR, "planted_open.glb"), g, buf)

# Compute unit (GLB units per metre) from head joint height
n = len(nodes)
parents = [None]*n
for i, nd in enumerate(nodes):
    for c in nd.get("children", []): parents[c] = i
world = [None]*n
def get_w(i):
    if world[i] is not None: return world[i]
    M = read_trs_mat(nodes[i])
    if parents[i] is not None: M = mat_mul(get_w(parents[i]), M)
    world[i] = M; return M
for i in range(n): get_w(i)
head_i = next(i for i, nd in enumerate(nodes) if nd.get("name") == "DEF-spine.006")
unit = world[head_i][13] / (2.0 * (1.6836 / 2.0))

# Hand scale for converting world delta into hand-local translation
hw = world[hand_i]
hs = math.sqrt(hw[0]**2 + hw[1]**2 + hw[2]**2)
if hs < 1e-9: hs = 1.0

# 2. Planted fingertip socket: shift socket +0.10 m along its +X (toward fingertips)
g2, buf2 = load_glb(os.path.join(DIR, "closed.glb"))
sock_i = next(i for i, nd in enumerate(g2["nodes"]) if nd.get("name") == "Socket_Hand_R")
sr = g2["nodes"][sock_i].get("rotation", [0.0, 0.0, 0.0, 1.0])
st = list(g2["nodes"][sock_i].get("translation", [0.0, 0.0, 0.0]))
ex = qrot(sr, [1.0, 0.0, 0.0])
shift_tip = 0.10 * unit / hs
g2["nodes"][sock_i]["translation"] = [st[k] + ex[k] * shift_tip for k in range(3)]
save_glb(os.path.join(DIR, "planted_fingertip.glb"), g2, buf2)

# 3. Planted +10 cm off handle: shift socket +0.11 m along its +Z (out of palm)
g3, buf3 = load_glb(os.path.join(DIR, "closed.glb"))
ez = qrot(sr, [0.0, 0.0, 1.0])
shift_off = 0.11 * unit / hs
g3["nodes"][sock_i]["translation"] = [st[k] + ez[k] * shift_off for k in range(3)]
save_glb(os.path.join(DIR, "planted_10cm_off.glb"), g3, buf3)
PY

  for case in closed planted_open planted_fingertip planted_10cm_off; do
    "$BIN" motion check --input "$dir/$case.glb" --clip sweep --camera game --blade-pose socket --no-through --json > "$dir/$case.json"
  done

  if python3 - "$label" "$dir" "$min_closed" <<'PY'
import json, os, sys
label, d, min_closed = sys.argv[1], sys.argv[2], float(sys.argv[3])
ok = True
for case in ("closed", "planted_open", "planted_fingertip", "planted_10cm_off"):
    j = json.load(open(os.path.join(d, f"{case}.json")))
    g = j["clips"][0]["read_cam"]["grip"]
    print(f"  {label:14s} {case:18s}: score={g['score']:.3f} palm={g['palm_surface_cm']:.1f}cm wrap={g['skin_wrap']:.3f} curl={g['curl_deg']:.1f}deg skin={g['skin_based']}")
    if not g["skin_based"]:
        print(f"    ERROR: skin_based is False on {label} {case}")
        ok = False
    if case == "closed" and min_closed > 0:
        if g["score"] < min_closed:
            print(f"    ERROR: closed score {g['score']} < {min_closed}")
            ok = False
    elif case.startswith("planted_"):
        if g["score"] >= 0.30:
            print(f"    ERROR: planted case {case} scored {g['score']} >= 0.30")
            ok = False
sys.exit(0 if ok else 1)
PY
  then
    pass "real rig $label (closed + planted cases)"
  else
    fail "real rig $label (closed + planted cases)"
  fi
}

if [ -n "$FIVE_FINGER" ]; then
  echo "== real five-finger rig check"
  check_real_rig "five_finger" "$FIVE_FINGER" "0.90"
fi

if [ -n "$SINGLE_CHAIN" ]; then
  echo "== real single-chain rig check"
  check_real_rig "single_chain" "$SINGLE_CHAIN" "0.0"
fi

echo
if [ "$FAILS" -eq 0 ]; then
  echo "GRIP GATE PASS"
  exit 0
fi
echo "GRIP GATE FAIL ($FAILS failed)"
exit 1
