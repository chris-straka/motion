#!/usr/bin/env python3
"""A made-up four-legged creature as one closed mesh (stdlib only): the
fixture the creature gates are tested on (`tools/creature_gates.py
selftest`). No model, no corpus: capsules joined smoothly and meshed by
surface nets on a fixed grid, so the same command writes the same bytes.

    python3 tools/creature_fixture.py OUT.glb [--cell 0.03]

Body, four legs, neck and head, tail; about 1.1 m long, standing on the
ground at the origin, facing glTF +Z; positions, normals, UVs and one
plain material.
"""

import argparse
import json
import math
import struct

# (a, b, radius at a, radius at b): body, legs, neck, head, tail. Metres.
CAPSULES = [
    ((0.0, 0.56, -0.40), (0.0, 0.60, 0.38), 0.20, 0.21),
    ((0.15, 0.50, -0.34), (0.15, 0.03, -0.36), 0.075, 0.055),
    ((-0.15, 0.50, -0.34), (-0.15, 0.03, -0.36), 0.075, 0.055),
    ((0.15, 0.52, 0.30), (0.15, 0.03, 0.33), 0.075, 0.055),
    ((-0.15, 0.52, 0.30), (-0.15, 0.03, 0.33), 0.075, 0.055),
    ((0.0, 0.66, 0.45), (0.0, 0.84, 0.62), 0.10, 0.085),
    ((0.0, 0.88, 0.62), (0.0, 0.82, 0.86), 0.11, 0.07),
    ((0.0, 0.58, -0.55), (0.0, 0.46, -0.95), 0.06, 0.025),
]
BLEND = 0.06


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def capsule(p, a, b, ra, rb):
    ab, ap = sub(b, a), sub(p, a)
    t = max(0.0, min(1.0, dot(ap, ab) / dot(ab, ab)))
    q = (a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t)
    return math.dist(p, q) - (ra + (rb - ra) * t)


def smin(a, b, k):
    h = max(k - abs(a - b), 0.0) / k
    return min(a, b) - h * h * k * 0.25


def field(p):
    d = None
    for a, b, ra, rb in CAPSULES:
        c = capsule(p, a, b, ra, rb)
        d = c if d is None else smin(d, c, BLEND)
    return d


def surface_nets(cell):
    lo = (-0.35, -0.04, -1.05)
    n = (int(0.70 / cell) + 2, int(1.08 / cell) + 2, int(2.05 / cell) + 2)
    def at(i, j, k):
        return (lo[0] + i * cell, lo[1] + j * cell, lo[2] + k * cell)
    f = {}
    for i in range(n[0] + 1):
        for j in range(n[1] + 1):
            for k in range(n[2] + 1):
                f[i, j, k] = field(at(i, j, k))
    verts, index = [], {}
    corners = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)]
    edges = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)]
    for i in range(n[0]):
        for j in range(n[1]):
            for k in range(n[2]):
                vals = [f[i + c[0], j + c[1], k + c[2]] for c in corners]
                if all(v < 0 for v in vals) or all(v >= 0 for v in vals):
                    continue
                acc, m = [0.0, 0.0, 0.0], 0
                for e0, e1 in edges:
                    v0, v1 = vals[e0], vals[e1]
                    if (v0 < 0) != (v1 < 0):
                        t = v0 / (v0 - v1)
                        p0, p1 = at(i + corners[e0][0], j + corners[e0][1], k + corners[e0][2]), at(i + corners[e1][0], j + corners[e1][1], k + corners[e1][2])
                        for a in range(3):
                            acc[a] += p0[a] + (p1[a] - p0[a]) * t
                        m += 1
                index[i, j, k] = len(verts)
                verts.append(tuple(x / m for x in acc))
    quads = []
    for (i, j, k), v in list(f.items()):
        for axis in range(3):
            o = [0, 0, 0]
            o[axis] = 1
            w = (i + o[0], j + o[1], k + o[2])
            if w not in f or (v < 0) == (f[w] < 0):
                continue
            # the four cells round this edge
            u, t = [(1, 2), (0, 2), (0, 1)][axis]
            cells = []
            for du, dt in ((0, 0), (1, 0), (1, 1), (0, 1)):
                c = [i, j, k]
                c[u] -= du
                c[t] -= dt
                cells.append(tuple(c))
            if not all(c in index for c in cells):
                continue
            q = [index[c] for c in cells]
            if v < 0:
                q.reverse()
            if axis == 1:
                q.reverse()
            quads.append(q)
    tris = []
    for a, b, c, d in quads:
        tris += [(a, b, c), (a, c, d)]
    return verts, tris


def normal_at(p, e=1e-4):
    g = [(field((p[0] + (e if a == 0 else 0), p[1] + (e if a == 1 else 0), p[2] + (e if a == 2 else 0)))
          - field((p[0] - (e if a == 0 else 0), p[1] - (e if a == 1 else 0), p[2] - (e if a == 2 else 0)))) for a in range(3)]
    s = math.sqrt(sum(x * x for x in g)) or 1.0
    return tuple(x / s for x in g)


def write_glb(path, verts, tris):
    # Outward faces: flip any triangle whose normal points into the body.
    out = []
    for a, b, c in tris:
        pa, pb, pc = verts[a], verts[b], verts[c]
        nrm = (sub(pb, pa)[1] * sub(pc, pa)[2] - sub(pb, pa)[2] * sub(pc, pa)[1],
               sub(pb, pa)[2] * sub(pc, pa)[0] - sub(pb, pa)[0] * sub(pc, pa)[2],
               sub(pb, pa)[0] * sub(pc, pa)[1] - sub(pb, pa)[1] * sub(pc, pa)[0])
        cen = tuple((pa[k] + pb[k] + pc[k]) / 3 for k in range(3))
        out.append((a, c, b) if dot(nrm, normal_at(cen)) < 0 else (a, b, c))
    tris = out
    ys = [v[1] for v in verts]
    y0 = min(ys)
    verts = [(v[0], v[1] - y0, v[2]) for v in verts]
    normals = [normal_at((v[0], v[1] + y0, v[2])) for v in verts]
    uvs = [((v[0] + 0.35) / 0.7, (v[2] + 1.05) / 2.1) for v in verts]
    binb = bytearray()

    def view(data, target):
        while len(binb) % 4:
            binb.append(0)
        start = len(binb)
        binb.extend(data)
        bvs.append({"buffer": 0, "byteOffset": start, "byteLength": len(data), "target": target})
        return len(bvs) - 1

    bvs = []
    pos = view(b"".join(struct.pack("<3f", *v) for v in verts), 34962)
    nor = view(b"".join(struct.pack("<3f", *v) for v in normals), 34962)
    uv = view(b"".join(struct.pack("<2f", *v) for v in uvs), 34962)
    idx = view(b"".join(struct.pack("<3I", *t) for t in tris), 34963)
    j = {
        "asset": {"version": "2.0", "generator": "motion tools/creature_fixture.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "fixture_creature", "mesh": 0}],
        "meshes": [{"name": "fixture_creature", "primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
        "materials": [{"name": "fixture_skin", "pbrMetallicRoughness": {"baseColorFactor": [0.42, 0.36, 0.30, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.85}}],
        "accessors": [
            {"bufferView": pos, "componentType": 5126, "count": len(verts), "type": "VEC3",
             "min": [min(v[k] for v in verts) for k in range(3)], "max": [max(v[k] for v in verts) for k in range(3)]},
            {"bufferView": nor, "componentType": 5126, "count": len(verts), "type": "VEC3"},
            {"bufferView": uv, "componentType": 5126, "count": len(verts), "type": "VEC2"},
            {"bufferView": idx, "componentType": 5125, "count": 3 * len(tris), "type": "SCALAR"},
        ],
        "bufferViews": bvs,
        "buffers": [{"byteLength": 0}],
    }
    while len(binb) % 4:
        binb.append(0)
    j["buffers"][0]["byteLength"] = len(binb)
    js = json.dumps(j, separators=(",", ":")).encode()
    js += b" " * ((4 - len(js) % 4) % 4)
    with open(path, "wb") as fh:
        fh.write(struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(binb)))
        fh.write(struct.pack("<I", len(js)) + b"JSON" + js)
        fh.write(struct.pack("<I", len(binb)) + b"BIN\0" + bytes(binb))
    return len(verts), len(tris)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("out")
    ap.add_argument("--cell", type=float, default=0.03)
    a = ap.parse_args()
    verts, tris = surface_nets(a.cell)
    nv, nt = write_glb(a.out, verts, tris)
    print(f"creature fixture: {nv} vertices, {nt} triangles -> {a.out}")


if __name__ == "__main__":
    main()
