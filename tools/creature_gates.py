#!/usr/bin/env python3
"""The creature route's gate checks, on a finished GLB (stdlib only).

    python3 tools/creature_gates.py check RIG.glb [--json OUT.json] [--work DIR]
    python3 tools/creature_gates.py plant swap|hole|influence5|overbudget IN.glb OUT.glb
    python3 tools/creature_gates.py selftest [--work DIR]

`check` runs gates 2 to 5 of `~/Games/MONSTERS.md` on one rigged creature
and prints one line per check. Every number is measured here, on the file
given, by this script or by the tool it names; nothing is read from the
route's own report. A tool that is missing or does not run is a failure of
its check, never a pass. Exit 0 only when every check passes; exit 1
otherwise; exit 2 when the file cannot be read.

| gate | check | what fails it |
| --- | --- | --- |
| 2 | `contract` | `rfcheck --class monster` exits non-zero |
| 2 | `tris` | more than 20,000 triangles in the scene, counted from the GLB here (rfcheck's monster budget; its `P_TRIS` is only a warning) |
| 2 | `holes` | any open rim, after welding seams: a monster's skin is closed, and an open rim is a see-through gap from some side (the clean step fills rims up to 8% of the largest side and calls longer ones meant openings; a monster has none) |
| 3 | `skeleton` | `creature-rig check` exits non-zero or does not pass |
| 3 | `anatomy` | `creature-bench anatomy` has any failing finding (`B_SWAPPED`, `B_NO_LEG`, ...) |
| 4 | `bend` | `weights check --poses ROM` (ROM from `creature-rig check --rom` on this file; the file's own clips are played too) exits non-zero or has a failing finding |
| 4 | `weight_sum` | a skinned vertex whose weights sum further than 0.001 from 1 |
| 4 | `influences` | a skinned vertex with more than 4 non-zero weights |
| 5 | `idle_or_walk` | no clip HLH would play as idle (`idle`, `breath`) or walk (`walk`) that lasts 0.25 s and turns a joint at least 2 degrees (idle) or 5 (walk) |
| 5 | `attack` | no clip HLH would play as an attack (`attack`, `lunge`, `bite`) that lasts 0.25 s and turns a joint at least 15 degrees |

Clips are found the way HLH's `rigged.rs` finds them: names sorted, the
first whose lower-cased name contains one of the words.

`plant` makes the planted bad inputs of gate 8 from a good rig: `swap`
gives each limb's skin to its mirror partner's bones (a plain left/right
swap), `hole` cuts the faces round one vertex on the back, `influence5`
gives one vertex a fifth weight (its four scaled so the sum stays 1),
`overbudget` splits every triangle in four until the scene passes the
budget. `selftest` checks the committed fixture (`tests/fixtures/creature_gates/`):
the clean rig passes every check and each planted one fails, naming the
check in `expected.json`.

The thresholds above are fixed (2026-11-18, before the first run) and are
not changed to make a creature pass.
"""

import argparse
import hashlib
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile

TRI_BUDGET = 20000
WEIGHT_SUM_TOL = 0.001
MAX_INFLUENCES = 4
CLIP_MIN_SECONDS = 0.25
IDLE_WORDS = ("idle", "breath")
WALK_WORDS = ("walk",)
ATTACK_WORDS = ("attack", "lunge", "bite")
IDLE_MIN_TURN = 2.0
WALK_MIN_TURN = 5.0
ATTACK_MIN_TURN = 15.0

GATE_OF = {"contract": 2, "tris": 2, "holes": 2, "skeleton": 3, "anatomy": 3, "bend": 4,
           "weight_sum": 4, "influences": 4, "idle_or_walk": 5, "attack": 5}
CHECKS = list(GATE_OF)

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
SIB = os.path.dirname(REPO)
FIXTURES = os.path.join(REPO, "tests", "fixtures", "creature_gates")


# ------------------------------------------------------------------ GLB

COMP = {5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2), 5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4)}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT2": 4, "MAT3": 9, "MAT4": 16}
NORM_DIV = {5120: 127.0, 5121: 255.0, 5122: 32767.0, 5123: 65535.0}


class Glb:
    def __init__(self, path):
        with open(path, "rb") as f:
            b = f.read()
        if len(b) < 20 or struct.unpack("<I", b[:4])[0] != 0x46546C67:
            raise ValueError(f"{path}: not a GLB")
        n = struct.unpack("<I", b[12:16])[0]
        self.j = json.loads(b[20:20 + n])
        off = 20 + n
        self.bin = bytearray()
        if off + 8 <= len(b):
            m = struct.unpack("<I", b[off:off + 4])[0]
            self.bin = bytearray(b[off + 8:off + 8 + m])
        for buf in self.j.get("buffers", []):
            if "uri" in buf:
                raise ValueError(f"{path}: external buffers are not read")

    def read(self, acc_index):
        a = self.j["accessors"][acc_index]
        if "sparse" in a:
            raise ValueError("sparse accessors are not read")
        fmt, size = COMP[a["componentType"]]
        nc = NCOMP[a["type"]]
        count = a["count"]
        if "bufferView" not in a:
            return [tuple([0] * nc) for _ in range(count)]
        bv = self.j["bufferViews"][a["bufferView"]]
        base = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = bv.get("byteStride") or size * nc
        st = struct.Struct("<" + fmt * nc)
        out = []
        for i in range(count):
            out.append(st.unpack_from(self.bin, base + i * stride))
        if a.get("normalized") and a["componentType"] in NORM_DIV:
            d = NORM_DIV[a["componentType"]]
            out = [tuple(max(-1.0, x / d) for x in t) for t in out]
        return out

    def add(self, values, ctype, typ, target=None, minmax=False, normalized=False):
        """Appends an accessor (and its buffer view) holding `values`;
        returns its index."""
        fmt, size = COMP[ctype]
        nc = NCOMP[typ]
        while len(self.bin) % 4:
            self.bin.append(0)
        start = len(self.bin)
        st = struct.Struct("<" + fmt * nc)
        for v in values:
            self.bin += st.pack(*v)
        bv = {"buffer": 0, "byteOffset": start, "byteLength": len(self.bin) - start}
        if target:
            bv["target"] = target
        self.j.setdefault("bufferViews", []).append(bv)
        acc = {"bufferView": len(self.j["bufferViews"]) - 1, "componentType": ctype, "count": len(values), "type": typ}
        if normalized:
            acc["normalized"] = True
        if minmax and values:
            acc["min"] = [min(v[k] for v in values) for k in range(nc)]
            acc["max"] = [max(v[k] for v in values) for k in range(nc)]
        self.j.setdefault("accessors", []).append(acc)
        return len(self.j["accessors"]) - 1

    def write(self, path):
        if not self.j.get("buffers"):
            self.j["buffers"] = [{}]
        while len(self.bin) % 4:
            self.bin.append(0)
        self.j["buffers"][0]["byteLength"] = len(self.bin)
        js = json.dumps(self.j, separators=(",", ":")).encode()
        js += b" " * ((4 - len(js) % 4) % 4)
        total = 12 + 8 + len(js) + 8 + len(self.bin)
        with open(path, "wb") as f:
            f.write(struct.pack("<III", 0x46546C67, 2, total))
            f.write(struct.pack("<I", len(js)) + b"JSON" + js)
            f.write(struct.pack("<I", len(self.bin)) + b"BIN\0" + bytes(self.bin))

    def triangles(self, prim):
        """Index triples of a primitive (mode 4 only; other modes give none)."""
        if prim.get("mode", 4) != 4:
            return []
        if "indices" in prim:
            idx = [t[0] for t in self.read(prim["indices"])]
        else:
            idx = list(range(self.j["accessors"][prim["attributes"]["POSITION"]]["count"]))
        return [(idx[i], idx[i + 1], idx[i + 2]) for i in range(0, len(idx) - 2, 3)]

    def mesh_instances(self):
        """Mesh index of every node that draws one, once per node."""
        return [n["mesh"] for n in self.j.get("nodes", []) if "mesh" in n]


# ------------------------------------------------------------ measures

def count_tris(g):
    per_mesh = {}
    for mi, m in enumerate(g.j.get("meshes", [])):
        per_mesh[mi] = sum(len(g.triangles(p)) for p in m.get("primitives", []))
    return sum(per_mesh.get(mi, 0) for mi in g.mesh_instances())


def welded(g, mesh):
    """Positions of all a mesh's primitives welded by place: returns
    (welded triangles, welded positions, largest side)."""
    pts, tris = [], []
    for p in mesh.get("primitives", []):
        pos = g.read(p["attributes"]["POSITION"])
        base = len(pts)
        pts += pos
        tris += [(a + base, b + base, c + base) for a, b, c in g.triangles(p)]
    if not pts:
        return [], [], 0.0
    lo = [min(p[k] for p in pts) for k in range(3)]
    hi = [max(p[k] for p in pts) for k in range(3)]
    size = max(hi[k] - lo[k] for k in range(3)) or 1.0
    q = size * 1e-5
    key_of, wid, wpts = {}, [], []
    for p in pts:
        k = (round(p[0] / q), round(p[1] / q), round(p[2] / q))
        if k not in key_of:
            key_of[k] = len(wpts)
            wpts.append(p)
        wid.append(key_of[k])
    wt = []
    for a, b, c in tris:
        a, b, c = wid[a], wid[b], wid[c]
        if a != b and b != c and a != c:
            wt.append((a, b, c))
    return wt, wpts, size


def open_rims(g):
    """Every open rim of every mesh: (length as a share of the mesh's
    largest side, edge count)."""
    rims = []
    for m in g.j.get("meshes", []):
        tris, pts, size = welded(g, m)
        count = {}
        for a, b, c in tris:
            for e in ((a, b), (b, c), (c, a)):
                k = (min(e), max(e))
                count[k] = count.get(k, 0) + 1
        open_edges = [e for e, n in count.items() if n == 1]
        at = {}
        for i, (a, b) in enumerate(open_edges):
            at.setdefault(a, []).append(i)
            at.setdefault(b, []).append(i)
        seen = set()
        for i in range(len(open_edges)):
            if i in seen:
                continue
            seen.add(i)
            stack, loop = [i], []
            while stack:
                k = stack.pop()
                loop.append(k)
                for v in open_edges[k]:
                    for o in at[v]:
                        if o not in seen:
                            seen.add(o)
                            stack.append(o)
            length = sum(math.dist(pts[open_edges[k][0]], pts[open_edges[k][1]]) for k in loop)
            rims.append((length / size, len(loop)))
    return rims


def skinned_prims(g):
    for m in g.j.get("meshes", []):
        for p in m.get("primitives", []):
            if "JOINTS_0" in p.get("attributes", {}) and "WEIGHTS_0" in p["attributes"]:
                yield p


def weight_sets(g, prim):
    sets, k = [], 0
    while f"JOINTS_{k}" in prim["attributes"] and f"WEIGHTS_{k}" in prim["attributes"]:
        sets.append((g.read(prim["attributes"][f"JOINTS_{k}"]), g.read(prim["attributes"][f"WEIGHTS_{k}"])))
        k += 1
    return sets


def weight_stats(g):
    verts, worst_sum, bad_sum, most, over = 0, 0.0, 0, 0, 0
    for p in skinned_prims(g):
        sets = weight_sets(g, p)
        for i in range(len(sets[0][1])):
            ws = [w for _, wl in sets for w in wl[i]]
            s = sum(ws)
            n = sum(1 for w in ws if w > 0.0)
            verts += 1
            worst_sum = max(worst_sum, abs(s - 1.0))
            bad_sum += abs(s - 1.0) > WEIGHT_SUM_TOL
            most = max(most, n)
            over += n > MAX_INFLUENCES
    return {"skinned_verts": verts, "worst_sum_error": round(worst_sum, 6), "verts_off_sum": bad_sum,
            "most_influences": most, "verts_over_4": over}


def quat_angle(a, b):
    d = abs(sum(x * y for x, y in zip(a, b)))
    na = math.sqrt(sum(x * x for x in a)) or 1.0
    nb = math.sqrt(sum(x * x for x in b)) or 1.0
    return math.degrees(2.0 * math.acos(min(1.0, d / (na * nb))))


def clips(g):
    """Each clip: name, length (s), largest turn of any joint from its first key."""
    out = []
    for k, an in enumerate(g.j.get("animations", [])):
        t0, t1, turn = None, None, 0.0
        for ch in an.get("channels", []):
            s = an["samplers"][ch["sampler"]]
            times = [t[0] for t in g.read(s["input"])]
            if times:
                t0 = min(times) if t0 is None else min(t0, min(times))
                t1 = max(times) if t1 is None else max(t1, max(times))
            if ch.get("target", {}).get("path") != "rotation":
                continue
            vals = g.read(s["output"])
            if s.get("interpolation") == "CUBICSPLINE":
                vals = vals[1::3]
            if vals:
                turn = max(turn, max(quat_angle(vals[0], v) for v in vals))
        out.append({"name": an.get("name", f"animation_{k}"), "seconds": round((t1 or 0.0) - (t0 or 0.0), 3),
                    "turn_deg": round(turn, 2)})
    return out


def clip_named(cl, words):
    names = sorted(c["name"] for c in cl)
    for w in words:
        for n in names:
            if w in n.lower():
                return next(c for c in cl if c["name"] == n)
    return None


# ---------------------------------------------------------------- tools

def find_tool(name, env, rel):
    for p in [os.environ.get(env), shutil.which(name)] + [os.path.join(SIB, r) for r in rel]:
        if p and os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    return None


def tools():
    return {
        "rfcheck": find_tool("rfcheck", "RFCHECK_BIN", ["rfcheck/target/release/rfcheck"]),
        "creature-rig": find_tool("creature-rig", "CREATURE_RIG_BIN", ["skintokens/target/release/creature-rig"]),
        "creature-bench": find_tool("creature-bench", "CREATURE_BENCH_BIN", ["skintokens/target/release/creature-bench"]),
        "weights": find_tool("weights", "WEIGHTS_BIN", ["meshweights/rust/target/release/weights"]),
    }


def run(cmd, timeout=1800):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return 124, "", f"timed out after {timeout} s"
    except OSError as e:
        return 127, "", str(e)


def tail(s, n=240):
    return (s or "").strip()[-n:]


# ---------------------------------------------------------------- check

def check(glb, work, quiet=False):
    """All checks on one GLB. Returns the result dict (`pass`, `checks`)."""
    os.makedirs(work, exist_ok=True)
    t = tools()
    res = {"file": os.path.abspath(glb), "tools": t, "checks": {}}
    C = res["checks"]

    def put(name, ok, cmd, code, **nums):
        C[name] = {"gate": GATE_OF[name], "pass": bool(ok), "cmd": cmd, "exit": code, **nums}

    try:
        g = Glb(glb)
    except (OSError, ValueError, KeyError) as e:
        res.update(ok=False, error=str(e))
        return res

    # Gate 2: contract, triangles, holes.
    if t["rfcheck"]:
        cmd = [t["rfcheck"], "--json", "--class", "monster", glb]
        code, out, err = run(cmd)
        fails = []
        try:
            d = json.loads(out)
            d = d[0] if isinstance(d, list) and d else d
            fails = [f"{x.get('code')}: {x.get('detail', x.get('message', ''))}" for x in d.get("diags", [])]
            warns = [x.get("code") for x in d.get("warns", [])]
        except ValueError:
            fails, warns = [tail(err or out)], []
        put("contract", code == 0, cmd, code, fails=fails, warns=warns)
    else:
        put("contract", False, ["rfcheck"], None, fails=["rfcheck not found"])
    n = count_tris(g)
    put("tris", n <= TRI_BUDGET, ["count triangles in the GLB"], 0, tris=n, budget=TRI_BUDGET)
    rims = open_rims(g)
    put("holes", not rims, ["open rims after welding"], 0, open_rims=len(rims), open_edges=sum(r[1] for r in rims),
        largest_rim_share=round(max((r[0] for r in rims), default=0.0), 4))

    # Gate 3: skeleton and anatomy.
    rom = os.path.join(work, "gate.rom.ron")
    if t["creature-rig"]:
        sj = os.path.join(work, "skeleton.json")
        for p in (sj, rom):
            if os.path.exists(p):
                os.remove(p)
        cmd = [t["creature-rig"], "check", glb, "--json", sj, "--rom", rom]
        code, out, err = run(cmd)
        try:
            with open(sj) as f:
                s = json.load(f)
            sf = [f"{x['code']} {x.get('part', '')}: {x.get('detail', '')}" for x in s.get("findings", []) if x.get("severity") == "fail"]
            put("skeleton", code == 0 and s.get("pass") is True, cmd, code, score=s.get("score"), kind=s.get("kind"),
                bones=s.get("bones"), fails=sf)
        except (OSError, ValueError):
            put("skeleton", False, cmd, code, fails=["did not run: " + tail(err or out)])
    else:
        put("skeleton", False, ["creature-rig"], None, fails=["creature-rig not found"])
    if t["creature-bench"]:
        aj = os.path.join(work, "anatomy.json")
        if os.path.exists(aj):
            os.remove(aj)
        cmd = [t["creature-bench"], "anatomy", glb, "--json", aj]
        code, out, err = run(cmd)
        try:
            with open(aj) as f:
                a = json.load(f)
            af = [f"{x.get('code')}: {x.get('detail', '')}" for x in a.get("findings", []) if x.get("severity") == "fail"]
            codes = sorted({x.split(":")[0] for x in af})
            ok = code == 0 and not af and a.get("pass") is True
            if code != 0 and not af:
                af = [f"creature-bench exited {code}" + (": " + tail(err or out) if (err or out).strip() else "")]
            put("anatomy", ok, cmd, code, codes=codes,
                fails=af, limb_pairs=a.get("limb_pairs"), head=a.get("head"))
        except (OSError, ValueError):
            put("anatomy", False, cmd, code, fails=["did not run: " + tail(err or out)])
    else:
        put("anatomy", False, ["creature-bench"], None, fails=["creature-bench not found"])

    # Gate 4: bend, weight sums, influences.
    if t["weights"] and os.path.exists(rom):
        cmd = [t["weights"], "check", glb, "--json", "--poses", rom]
        code, out, err = run(cmd)
        try:
            d = json.loads(out)
            bf = [f"{x.get('code')} {x.get('region')}: {x.get('detail', '')}" for x in d.get("findings", [])
                  if x.get("severity") == "fail"]
            put("bend", code == 0 and not bf and d.get("pass") is True, cmd, code, score=d.get("score"), fails=bf,
                poses=d.get("poses"), skipped_poses=len(d.get("skipped_poses", [])),
                tears_pct=round(100.0 * sum(x.get("verts", 0) for x in d.get("findings", []) if x.get("severity") == "fail"
                                            and x.get("code") in ("D_STRETCH", "D_VOLUME", "D_BLEED")) / max(1, d.get("verts", 1)), 2))
        except ValueError:
            put("bend", False, cmd, code, fails=["did not run: " + tail(err or out)])
    else:
        put("bend", False, ["weights check"], None,
            fails=["weights not found" if not t["weights"] else "no range of motion (creature-rig check did not write one)"])
    w = weight_stats(g)
    put("weight_sum", w["skinned_verts"] > 0 and w["verts_off_sum"] == 0, ["sum the weights in the GLB"], 0,
        skinned_verts=w["skinned_verts"], verts_off_sum=w["verts_off_sum"], worst_sum_error=w["worst_sum_error"],
        tolerance=WEIGHT_SUM_TOL)
    put("influences", w["skinned_verts"] > 0 and w["verts_over_4"] == 0, ["count non-zero weights in the GLB"], 0,
        most_influences=w["most_influences"], verts_over_4=w["verts_over_4"], limit=MAX_INFLUENCES)

    # Gate 5: clips HLH plays.
    cl = clips(g)
    idle, walk, att = clip_named(cl, IDLE_WORDS), clip_named(cl, WALK_WORDS), clip_named(cl, ATTACK_WORDS)

    def moves(c, turn):
        return bool(c) and c["seconds"] >= CLIP_MIN_SECONDS and c["turn_deg"] >= turn

    put("idle_or_walk", moves(idle, IDLE_MIN_TURN) or moves(walk, WALK_MIN_TURN), ["read the GLB's clips"], 0,
        idle=idle, walk=walk, clips=[c["name"] for c in cl])
    put("attack", moves(att, ATTACK_MIN_TURN), ["read the GLB's clips"], 0, attack=att)

    res["pass"] = all(c["pass"] for c in C.values())
    res["failed"] = [k for k in CHECKS if not C[k]["pass"]]
    if not quiet:
        for k in CHECKS:
            c = C[k]
            nums = {x: v for x, v in c.items() if x not in ("gate", "pass", "cmd", "exit", "fails", "clips")}
            why = ("; " + "; ".join(c.get("fails", [])[:3])) if not c["pass"] and c.get("fails") else ""
            print(f"gate {c['gate']} {k:12s} {'pass' if c['pass'] else 'FAIL'} {json.dumps(nums, separators=(',', ':'))}{why}")
        print(f"{os.path.basename(glb)}: {'PASS' if res['pass'] else 'FAIL: ' + ', '.join(res['failed'])}")
    return res


# ---------------------------------------------------------------- plant

def _inv4(m):
    """Inverse of a column-major 4x4 (glTF) matrix given as 16 floats."""
    a = [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    n = 4
    aug = [row[:] + [1.0 if i == j else 0.0 for j in range(n)] for i, row in enumerate(a)]
    for col in range(n):
        piv = max(range(col, n), key=lambda r: abs(aug[r][col]))
        aug[col], aug[piv] = aug[piv], aug[col]
        p = aug[col][col]
        if abs(p) < 1e-12:
            raise ValueError("singular bind matrix")
        aug[col] = [x / p for x in aug[col]]
        for r in range(n):
            if r != col:
                f = aug[r][col]
                aug[r] = [x - f * y for x, y in zip(aug[r], aug[col])]
    inv = [row[n:] for row in aug]
    return [inv[r][c] for c in range(4) for r in range(4)]


def mirror_partners(g):
    """Joint index (in the skin's list) -> its mirror partner across the
    body's mirror plane (x = the middle of the skinned mesh), by rest
    place, mutual nearest; joints on the plane are left out."""
    skin = g.j["skins"][0]
    ibm = g.read(skin["inverseBindMatrices"])
    places = [tuple(_inv4(m)[12:15]) for m in ibm]
    xs = [p[0] for pr in skinned_prims(g) for p in g.read(pr["attributes"]["POSITION"])]
    mid = (min(xs) + max(xs)) / 2.0
    size = max(max(p[k] for p in places) - min(p[k] for p in places) for k in range(3)) or 1.0
    side = [i for i, p in enumerate(places) if abs(p[0] - mid) > 0.02 * size]

    def nearest(i, cands):
        m = (2 * mid - places[i][0], places[i][1], places[i][2])
        return min(cands, key=lambda j: math.dist(m, places[j]))

    pairs = {}
    for i in side:
        other = [j for j in side if (places[j][0] - mid) * (places[i][0] - mid) < 0]
        if not other:
            continue
        j = nearest(i, other)
        back = [k for k in side if (places[k][0] - mid) * (places[j][0] - mid) < 0]
        if back and nearest(j, back) == i:
            pairs[i] = j
    return pairs


def plant_swap(g):
    pairs = mirror_partners(g)
    if not pairs:
        raise ValueError("no mirrored limb pairs to swap")
    for p in skinned_prims(g):
        js = g.read(p["attributes"]["JOINTS_0"])
        acc = g.j["accessors"][p["attributes"]["JOINTS_0"]]
        new = [tuple(pairs.get(j, j) for j in t) for t in js]
        ct = acc["componentType"] if max((max(t) for t in new), default=0) < (256 if acc["componentType"] == 5121 else 65536) else 5123
        p["attributes"]["JOINTS_0"] = g.add(new, ct, "VEC4", target=34962)
    return {"swapped_joints": len(pairs)}


def plant_hole(g):
    """Cuts every face round the vertex highest on the back (welded, so a
    seam doesn't leave half of it standing)."""
    best = None
    for mi, m in enumerate(g.j.get("meshes", [])):
        for pi, p in enumerate(m.get("primitives", [])):
            for vi, v in enumerate(g.read(p["attributes"]["POSITION"])):
                if best is None or v[1] > best[0][1]:
                    best = (v, mi, pi, vi)
    if best is None:
        raise ValueError("no vertices")
    v0 = best[0]
    m = g.j["meshes"][best[1]]
    _, _, size = welded(g, m)
    q = size * 1e-5
    cut = 0
    for p in m["primitives"]:
        pos = g.read(p["attributes"]["POSITION"])
        hit = {i for i, v in enumerate(pos) if all(abs(v[k] - v0[k]) <= q for k in range(3))}
        tris = g.triangles(p)
        keep = [t for t in tris if not (hit & set(t))]
        cut += len(tris) - len(keep)
        if len(keep) != len(tris):
            flat = [(i,) for t in keep for i in t]
            p["indices"] = g.add(flat, 5125, "SCALAR", target=34963)
    if not cut:
        raise ValueError("cut no faces")
    return {"faces_cut": cut}


def plant_influence5(g):
    for p in skinned_prims(g):
        js = g.read(p["attributes"]["JOINTS_0"])
        ws = g.read(p["attributes"]["WEIGHTS_0"])
        njoints = len(g.j["skins"][0]["joints"])
        # the vertex whose weight is most spread: least chance its fifth joint is one it has
        vi = max(range(len(ws)), key=lambda i: (sum(1 for w in ws[i] if w > 0), -max(ws[i]), -i))
        have = set(j for j, w in zip(js[vi], ws[vi]) if w > 0)
        fifth = next(j for j in range(njoints) if j not in have)
        ws2 = [tuple(float(x) for x in w) for w in ws]
        ws2[vi] = tuple(0.9 * x for x in ws2[vi])
        j1 = [(0, 0, 0, 0)] * len(ws)
        w1 = [(0.0, 0.0, 0.0, 0.0)] * len(ws)
        # four of its own scaled to 0.9, the fifth takes the rest: the sum stays 1
        j1[vi] = (fifth, 0, 0, 0)
        w1[vi] = (1.0 - sum(ws2[vi]), 0.0, 0.0, 0.0)
        p["attributes"]["WEIGHTS_0"] = g.add(ws2, 5126, "VEC4", target=34962)
        p["attributes"]["JOINTS_1"] = g.add(j1, 5123, "VEC4", target=34962)
        p["attributes"]["WEIGHTS_1"] = g.add(w1, 5126, "VEC4", target=34962)
        return {"vertex": vi, "fifth_joint": fifth}
    raise ValueError("no skinned primitive")


def _merge_skin(a, b):
    acc = {}
    for js, ws in (a, b):
        for j, w in zip(js, ws):
            if w > 0:
                acc[j] = acc.get(j, 0.0) + 0.5 * w
    top = sorted(acc.items(), key=lambda kv: (-kv[1], kv[0]))[:4]
    s = sum(w for _, w in top) or 1.0
    top += [(0, 0.0)] * (4 - len(top))
    return tuple(j for j, _ in top), tuple(w / s for _, w in top)


def plant_overbudget(g):
    rounds = 0
    while count_tris(g) <= TRI_BUDGET:
        rounds += 1
        if rounds > 6:
            raise ValueError("could not pass the budget")
        for m in g.j.get("meshes", []):
            for p in m.get("primitives", []):
                if p.get("mode", 4) != 4:
                    continue
                at = p["attributes"]
                data = {k: g.read(v) for k, v in at.items() if not k.startswith(("JOINTS_", "WEIGHTS_"))}
                skin = "JOINTS_0" in at and "WEIGHTS_0" in at
                if skin:
                    J, W = g.read(at["JOINTS_0"]), g.read(at["WEIGHTS_0"])
                tris = g.triangles(p)
                mid = {}

                def midpoint(a, b):
                    k = (min(a, b), max(a, b))
                    if k in mid:
                        return mid[k]
                    for name, vals in data.items():
                        vals.append(tuple((x + y) / 2.0 for x, y in zip(vals[a], vals[b])))
                    if skin:
                        j, w = _merge_skin((J[a], W[a]), (J[b], W[b]))
                        J.append(j)
                        W.append(w)
                    mid[k] = len(next(iter(data.values()))) - 1
                    return mid[k]

                new = []
                for a, b, c in tris:
                    ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
                    new += [(a, ab, ca), (ab, b, bc), (ca, bc, c), (ab, bc, ca)]
                for name, vals in data.items():
                    acc = g.j["accessors"][at[name]]
                    if name == "NORMAL":
                        vals = [tuple(x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v) for v in vals]
                    at[name] = g.add(vals, 5126, acc["type"], target=34962, minmax=(name == "POSITION"))
                if skin:
                    at["JOINTS_0"] = g.add(J, 5123, "VEC4", target=34962)
                    at["WEIGHTS_0"] = g.add(W, 5126, "VEC4", target=34962)
                for k in [k for k in at if k.startswith(("JOINTS_", "WEIGHTS_")) and k[-1] != "0"]:
                    del at[k]
                p.pop("targets", None)
                p["indices"] = g.add([(i,) for t in new for i in t], 5125, "SCALAR", target=34963)
    for m in g.j.get("meshes", []):
        m.pop("weights", None)
    return {"rounds": rounds, "tris": count_tris(g)}


PLANTS = {"swap": plant_swap, "hole": plant_hole, "influence5": plant_influence5, "overbudget": plant_overbudget}
PLANT_CHECK = {"swap": "anatomy", "hole": "holes", "influence5": "influences", "overbudget": "tris"}


def plant(kind, src, dst):
    g = Glb(src)
    info = PLANTS[kind](g)
    g.write(dst)
    return info


# ------------------------------------------------------------- selftest

def selftest(work):
    """The committed clean fixture passes every check; each committed
    planted fixture fails, naming its check."""
    exp = json.load(open(os.path.join(FIXTURES, "expected.json")))
    ok, report = True, {}
    pinned = [(exp["clean"], exp["clean_sha256"])] + [(v["file"], v["sha256"]) for v in exp["planted"].values()]
    for name, want in pinned:
        with open(os.path.join(FIXTURES, name), "rb") as f:
            got = hashlib.sha256(f.read()).hexdigest()
        if got != want:
            ok = False
            print(f"selftest: {name} is not the committed fixture (sha256 {got[:12]}, expected {want[:12]})")
    clean = check(os.path.join(FIXTURES, exp["clean"]), os.path.join(work, "clean"), quiet=True)
    report["clean"] = {"pass": clean.get("pass", False), "failed": clean.get("failed", [])}
    if not clean.get("pass"):
        ok = False
        print(f"selftest: the clean fixture fails {clean.get('failed') or clean.get('error')}")
    for kind, item in exp["planted"].items():
        r = check(os.path.join(FIXTURES, item["file"]), os.path.join(work, kind), quiet=True)
        named = item["check"] in r.get("failed", [])
        good = (not r.get("pass", False)) and named
        report[kind] = {"pass": r.get("pass", False), "failed": r.get("failed", []), "names_right_check": named}
        print(f"selftest: planted {kind:11s} -> {'exit 1' if not r.get('pass') else 'exit 0'}, failed {r.get('failed')}: "
              f"{'names ' + item['check'] if named else 'DOES NOT NAME ' + item['check']}")
        ok = ok and good
    print(f"selftest: {'PASS' if ok else 'FAIL'}")
    return ok, report


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("glb")
    c.add_argument("--json", default="")
    c.add_argument("--work", default="")
    p = sub.add_parser("plant")
    p.add_argument("kind", choices=sorted(PLANTS))
    p.add_argument("src")
    p.add_argument("dst")
    s = sub.add_parser("selftest")
    s.add_argument("--work", default="")
    s.add_argument("--json", default="")
    a = ap.parse_args()
    if a.cmd == "plant":
        try:
            info = plant(a.kind, a.src, a.dst)
        except (OSError, ValueError, KeyError, StopIteration) as e:
            print(f"plant {a.kind}: {e}", file=sys.stderr)
            return 2
        print(f"plant {a.kind}: {json.dumps(info)} -> {a.dst}")
        return 0
    work = a.work or tempfile.mkdtemp(prefix="creature_gates_")
    if a.cmd == "selftest":
        ok, rep = selftest(work)
        if a.json:
            json.dump(rep, open(a.json, "w"), indent=1)
        return 0 if ok else 1
    res = check(a.glb, work)
    if a.json:
        with open(a.json, "w") as f:
            json.dump(res, f, indent=1)
    if "error" in res:
        print(f"{a.glb}: {res['error']}", file=sys.stderr)
        return 2
    return 0 if res["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
