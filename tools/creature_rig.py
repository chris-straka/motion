#!/usr/bin/env python3
"""Choose the cleanest creature skeleton across seeds and build `creature.json`.

    python3 tools/creature_rig.py MESH.glb OUT_DIR [--seeds 1,2,3] [--class custom]
        [--forward auto|+x|-x|+y|-y|...] [--name NAME]

Delegates multi-seed auto-rigging and CanItCode pruning to `skintokens/bin/autorig`
(`AUTORIG_BIN` or `--autorig`) when available, then runs `motionforge creature-profile`
to emit `OUT_DIR/creature.json` and `clean.glb`. Falls back cleanly to direct
`--skintokens` when invoked standalone without sibling `skintokens`.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys


def which(name, explicit, fallbacks):
    for p in [explicit, os.environ.get(name.upper() + "_BIN"), shutil.which(name)] + fallbacks:
        if p and os.path.exists(p):
            return p
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mesh", help="cleaned bare mesh GLB")
    ap.add_argument("out", help="output directory (gets creature.json, clean.glb, tries)")
    ap.add_argument("--seeds", default="1,2,3")
    ap.add_argument("--class", dest="klass", default="custom")
    ap.add_argument("--forward", default="auto")
    ap.add_argument("--name", default="")
    ap.add_argument("--autorig", default="")
    ap.add_argument("--skintokens", default="")
    ap.add_argument("--motionforge", default="")
    args = ap.parse_args()

    home = os.path.expanduser("~")
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    sib = os.path.dirname(repo)
    ar = which(
        "autorig",
        args.autorig,
        [os.path.join(sib, "skintokens/bin/autorig"), os.path.join(home, "Games/_blender/skintokens/bin/autorig")],
    )
    sk = which(
        "skintokens",
        args.skintokens,
        [os.path.join(sib, "skintokens/bin/skintokens"), os.path.join(home, "Games/_blender/skintokens/bin/skintokens")],
    )
    mf = which(
        "motionforge",
        args.motionforge,
        [
            os.path.join(repo, "rust/target/release/motionforge"),
            os.path.join(os.environ.get("CARGO_TARGET_DIR", ""), "release/motionforge"),
        ],
    )
    if not mf:
        raise SystemExit("motionforge not found; build rust/ or pass --motionforge")
    name = args.name or os.path.splitext(os.path.basename(args.mesh))[0]
    os.makedirs(args.out, exist_ok=True)

    if ar:
        ar_glb = os.path.join(args.out, "autorig.glb")
        ar_rep = os.path.join(args.out, "autorig.report.json")
        ar_cmd = [
            ar,
            args.mesh,
            ar_glb,
            "--method",
            "auto",
            "--class",
            args.klass if args.klass in ("hero", "npc", "monster", "humanoid", "quadruped", "custom") else "monster",
            "--seeds",
            args.seeds,
            "--report",
            ar_rep,
            "--no-fix",
        ]
        if sk:
            ar_cmd += ["--skintokens", sk]
        r = subprocess.run(ar_cmd, capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit(f"autorig failed ({r.returncode}): {(r.stderr or r.stdout).strip()[-400:]}")
        prof = os.path.join(args.out, "creature.json")
        clean = os.path.join(args.out, "clean.glb")
        chk = os.path.join(args.out, "report.json")
        rp = subprocess.run(
            [mf, "creature-profile", "--rig", ar_glb, "--output", prof, "--clean-glb", clean, "--report", chk, "--forward", args.forward, "--name", name],
            capture_output=True,
            text=True,
        )
        if rp.returncode != 0:
            raise SystemExit(f"creature-profile failed ({rp.returncode}): {(rp.stderr or rp.stdout).strip()[-400:]}")
        with open(chk) as f:
            checks = json.load(f)
        best_seed = 0
        if os.path.isfile(ar_rep):
            with open(ar_rep) as f:
                best_seed = json.load(f).get("best_seed", 0)
        with open(os.path.join(args.out, "tries.json"), "w") as f:
            json.dump({"mesh": os.path.abspath(args.mesh), "class": args.klass, "best_seed": best_seed, "via": "autorig", "checks": checks}, f, indent=2)
        print(f"best: via autorig (score {checks.get('score', 0.0):.4f}) -> {args.out}/creature.json, clean.glb")
        return

    if not sk:
        raise SystemExit("neither autorig nor skintokens binary found; pass --autorig or --skintokens")
    tries = []
    for seed in [int(s) for s in args.seeds.split(",") if s]:
        rig = os.path.join(args.out, f"try_s{seed}.glb")
        rep = os.path.join(args.out, f"try_s{seed}.skintokens.json")
        r = subprocess.run([sk, "rig", args.mesh, rig, "--report", rep, "--class", args.klass, "--seed", str(seed)], capture_output=True, text=True)
        if r.returncode != 0:
            print(f"seed {seed}: skintokens failed: {r.stderr.strip()[-300:]}")
            tries.append({"seed": seed, "ok": False, "error": r.stderr.strip()[-300:]})
            continue
        prof = os.path.join(args.out, f"try_s{seed}.creature.json")
        clean = os.path.join(args.out, f"try_s{seed}.clean.glb")
        chk = os.path.join(args.out, f"try_s{seed}.report.json")
        r = subprocess.run([mf, "creature-profile", "--rig", rig, "--output", prof, "--clean-glb", clean, "--report", chk, "--forward", args.forward, "--name", name], capture_output=True, text=True)
        if r.returncode != 0:
            print(f"seed {seed}: creature-profile failed: {r.stderr.strip()[-300:]}")
            tries.append({"seed": seed, "ok": False, "error": r.stderr.strip()[-300:]})
            continue
        with open(chk) as f:
            checks = json.load(f)
        checks.update({"seed": seed, "ok": True})
        tries.append(checks)
    good = [t for t in tries if t.get("ok")]
    if not good:
        raise SystemExit("no rig try succeeded")
    best = sorted(good, key=lambda t: (-t["score"], t["seed"]))[0]
    for src, dst in (("creature.json", "creature.json"), ("clean.glb", "clean.glb"), ("report.json", "report.json")):
        shutil.copyfile(os.path.join(args.out, f"try_s{best['seed']}.{src}"), os.path.join(args.out, dst))
    with open(os.path.join(args.out, "tries.json"), "w") as f:
        json.dump({"mesh": os.path.abspath(args.mesh), "class": args.klass, "best_seed": best["seed"], "tries": tries}, f, indent=2)
    print(f"best: seed {best['seed']} (score {best['score']:.4f}) -> {args.out}/creature.json, clean.glb")


if __name__ == "__main__":
    main()
