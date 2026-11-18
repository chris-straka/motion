#!/usr/bin/env python3
"""`tools/creature.sh NAME MODEL OUT`'s driver (stdlib only): one Tripo
creature through the route and every gate of `~/Games/MONSTERS.md`.

    python3 tools/creature_run.py NAME MODEL OUT

The tools come from the environment `tools/creature_deps.sh` sets up
(`*_BIN`, `BLENDER`, `CREATURE_DEP_DIRS`). NAME is the creature's name in
the game (snake case); there are no other flags. The run's own setting is
`CREATURE_SET=dev`, which the dev-set runner sets: benchmark animals never
go into the game repo, so gate 9 is not run for them, and the run says so.

1. tests: each repo the route changed (motion, skintokens; charkit for
   gate 9) has its README's checks green, cargo with zero warnings. Run
   once per source tree (by git tree hash) and kept in the deps cache.
2-5. `tools/creature_gates.py check` on OUT/NAME.glb.
6. films at HLH's follow camera (`tools/creature_film.py`) and a fresh
   judge (`claude -p`, reading the pack folder only) that must call the
   can't-tell control can't-tell, catch the planted swapped limbs, and name
   no torn mesh, limb through the body or sliding foot on this creature,
   and see its attack.
7. the whole route again (OUT/run2) picks the same rig and gives the same
   numbers.
8. the committed planted fixtures (`creature_gates.py selftest`) and the
   same four plantings made from this creature's own result each make the
   check exit non-zero, naming the right check.
9. charkit's game step (`tools/line/creature_game.sh`) puts it on HLH's
   branch `character-line` and films it in the game.

OUT gets `gates.json` (each gate: pass, the commands, exit codes,
numbers), `NAME.glb`, the films and the judge's pack. Exit 0 only when
every gate passes; 1 otherwise; 2 when it could not start.
"""

import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import creature_gates as cg  # noqa: E402

JUDGE_SEED = 20261118


def sh(cmd, cwd=None, timeout=7200, env=None):
    t = time.time()
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env,
                           shell=isinstance(cmd, str))
        code, out, err = r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired as e:
        code, out, err = 124, (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or ""), f"timed out after {timeout} s"
    return {"cmd": cmd, "exit": code, "secs": round(time.time() - t, 1), "out": out, "err": err}


def tail(s, n=600):
    return (s or "").strip()[-n:]


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def dep_dirs():
    out = {}
    for part in os.environ.get("CREATURE_DEP_DIRS", "").split():
        k, _, v = part.partition("=")
        out[k] = v
    out.setdefault("motion", REPO)
    return out


# ------------------------------------------------------------- gate 1

def repo_checks(name, path):
    """The README's checks of one repo, as shell lines."""
    linux = sys.platform.startswith("linux")
    if name == "motion":
        return ["cd rust && cargo fmt --all --check",
                "cd rust && cargo build --locked --release --all-targets 2>&1 | tee /dev/stderr | (! grep -E '^warning')",
                "cd rust && cargo test --locked --release",
                "python3 -m unittest discover -s blender/tests -p 'test_pure.py'",
                "python3 -m unittest discover -s python/tests -p 'test_*.py'",
                "python3 -m unittest discover -s tools/tests -p 'test_*.py'",
                '"$BLENDER" --background --factory-startup --python blender/tests/test_headless.py',
                "python3 -m unittest blender.tests.test_batch"]
    if name == "skintokens":
        feat = " --no-default-features" if linux else ""
        return ["cargo fmt --all --check",
                f"cargo build --release --workspace --all-targets{feat} 2>&1 | tee /dev/stderr | (! grep -E '^warning')",
                f"cargo test --release --workspace{feat}",
                "python3 -m unittest discover -s tools -p 'test_*.py'"]
    if name == "charkit":
        return ["bin/charkit test"]
    return []


def gate_tests(cache, repos):
    res = {"pass": True, "repos": {}}
    for name, path in repos.items():
        tree = sh(["git", "-C", path, "rev-parse", "HEAD^{tree}"])["out"].strip()
        dirty = sh(["git", "-C", path, "status", "--porcelain", "--untracked-files=no"])["out"].strip()
        key = os.path.join(cache, "tests", f"{name}-{tree}.json")
        if tree and not dirty and os.path.exists(key):
            r = json.load(open(key))
            r["cached"] = True
        else:
            r = {"tree": tree, "commit": sh(["git", "-C", path, "rev-parse", "--short", "HEAD"])["out"].strip(),
                 "steps": [], "pass": True}
            for line in repo_checks(name, path):
                s = sh(line, cwd=path, timeout=10800)
                fails = [ln for ln in (s["out"] + s["err"]).splitlines() if ln.startswith(("test ", "FAIL", "ERROR")) and
                         ("FAILED" in ln or ln.startswith(("FAIL", "ERROR")))]
                r["steps"].append({"cmd": line, "exit": s["exit"], "secs": s["secs"], "failing": fails[:20],
                                   "tail": tail(s["out"] + s["err"], 400) if s["exit"] else ""})
                r["pass"] = r["pass"] and s["exit"] == 0
            if tree and not dirty:
                os.makedirs(os.path.dirname(key), exist_ok=True)
                json.dump(r, open(key, "w"), indent=1)
        res["repos"][name] = r
        res["pass"] = res["pass"] and r["pass"]
    return res


# ------------------------------------------------------------- the route

def route(name, model, out):
    """tripo_rig.py once into `out`, then the gate checks on its result."""
    os.makedirs(out, exist_ok=True)
    cmd = [sys.executable, os.path.join(HERE, "tripo_rig.py"), model, out, "--name", name,
           "--kind", "creature", "--class", "monster"]
    r = sh(cmd, timeout=6 * 3600)
    with open(os.path.join(out, "route.log"), "w") as f:
        f.write(r["out"] + "\n--- stderr\n" + r["err"])
    glb = os.path.join(out, name + ".glb")
    rep = {}
    try:
        rep = json.load(open(os.path.join(out, "report.json")))
    except (OSError, ValueError):
        pass
    res = {"cmd": cmd, "exit": r["exit"], "secs": r["secs"], "glb": glb if os.path.exists(glb) else None,
           "last_line": tail(r["out"], 400), "report": rep}
    if res["glb"]:
        res["check"] = cg.check(glb, os.path.join(out, "gates_work"), quiet=True)
    return res


def picked(rep):
    t = (rep.get("tries") or [{}])[0]
    return {"which": t.get("picked"), "seed": rep.get("best_seed"), "reason": (rep.get("pick") or {}).get("reason")}


def skeleton_of(glb):
    g = cg.Glb(glb)
    skin = g.j["skins"][0]
    ibm = g.read(skin["inverseBindMatrices"])
    names = [g.j["nodes"][j].get("name", str(j)) for j in skin["joints"]]
    places = [tuple(round(x, 5) for x in cg._inv4(m)[12:15]) for m in ibm]
    return names, places


def numbers(check):
    out = {}
    for k, c in (check or {}).get("checks", {}).items():
        out[k] = {x: v for x, v in c.items() if x not in ("cmd", "fails", "clips")}
    return out


def gate_repeat(r1, r2):
    res = {"pass": False}
    if not (r1.get("glb") and r2.get("glb")):
        res["why"] = "a run left no rig"
        return res
    p1, p2 = picked(r1["report"]), picked(r2["report"])
    s1, s2 = skeleton_of(r1["glb"]), skeleton_of(r2["glb"])
    n1, n2 = numbers(r1.get("check")), numbers(r2.get("check"))
    moved = 0.0
    if s1[0] == s2[0]:
        moved = max((max(abs(a - b) for a, b in zip(x, y)) for x, y in zip(s1[1], s2[1])), default=0.0)
    diffs = [k for k in n1 if n1[k] != n2.get(k)]
    res.update(first=p1, second=p2, same_pick=p1 == p2, same_joints=s1[0] == s2[0], joint_moved_m=round(moved, 6),
               numbers_differ=diffs, first_numbers=n1, second_numbers={k: n2[k] for k in diffs},
               same_file=sha256(r1["glb"]) == sha256(r2["glb"]))
    res["pass"] = res["same_pick"] and res["same_joints"] and moved <= 1e-5 and not diffs
    return res


# ------------------------------------------------------------- gate 8

def gate_planted(glb, work):
    res = {"pass": True, "selftest": {}, "own": {}}
    ok, rep = cg.selftest(os.path.join(work, "selftest"))
    res["selftest"] = {"pass": ok, "report": rep, "cmd": ["creature_gates.py", "selftest"]}
    res["pass"] = ok
    for kind, check_name in cg.PLANT_CHECK.items():
        dst = os.path.join(work, f"planted_{kind}.glb")
        try:
            info = cg.plant(kind, glb, dst)
        except (OSError, ValueError, KeyError, StopIteration) as e:
            res["own"][kind] = {"pass": False, "why": f"could not plant: {e}"}
            res["pass"] = False
            continue
        r = cg.check(dst, os.path.join(work, f"planted_{kind}"), quiet=True)
        named = check_name in r.get("failed", [])
        res["own"][kind] = {"pass": (not r.get("pass")) and named, "exit": 0 if r.get("pass") else 1,
                            "failed": r.get("failed"), "expected": check_name, "planted": info}
        res["pass"] = res["pass"] and res["own"][kind]["pass"]
    return res


# ------------------------------------------------------------- gate 6

def film(blender, glb, clip, prefix, crop=None, video=None, measure=False):
    cmd = [blender, "-b", "--factory-startup", "-P", os.path.join(HERE, "creature_film.py"), "--",
           "--glb", glb, "--clip", clip, "--out", prefix, "--frames", "8"]
    if crop:
        cmd += ["--crop", ",".join(str(c) for c in crop)]
    if video:
        cmd += ["--video", video]
    if measure:
        cmd.append("--measure-only")
    r = sh(cmd, timeout=3600)
    lines = [ln for ln in r["out"].splitlines() if ln.startswith("CREATURE_FILM ")]
    if r["exit"] != 0 or not lines:
        raise RuntimeError(f"film {clip} of {glb}: {tail(r['err'] or r['out'], 300)}")
    return json.loads(lines[-1][len("CREATURE_FILM "):])


def tile(pngs, out, cols=4):
    rows = (len(pngs) + cols - 1) // cols
    r = sh(["ffmpeg", "-loglevel", "error", "-y", "-i", pngs[0].rsplit("_", 1)[0] + "_%03d.png",
            "-vf", f"tile={cols}x{rows}:padding=6:color=white", "-frames:v", "1", out])
    if r["exit"] != 0:
        raise RuntimeError("ffmpeg tile: " + tail(r["err"]))


def side_by_side(left, right, out):
    r = sh(["ffmpeg", "-loglevel", "error", "-y", "-i", left, "-i", right, "-filter_complex",
            "[0:v]pad=iw+24:ih:0:0:color=white[a];[a][1:v]hstack", "-frames:v", "1", out])
    if r["exit"] != 0:
        raise RuntimeError("ffmpeg hstack: " + tail(r["err"]))


JUDGE_PROMPT = """You are checking creature animation for a video game, as a player would see it.
The pictures in this folder are item_01.png to item_{n:02d}.png. Each is a film strip: 8 frames of one
clip in time order, left to right, top row then bottom row, seen from the game's camera. Some items are
two strips side by side (a LEFT half and a RIGHT half, split by a white band). questions.json says which
kind each item is and what the clip is. Read questions.json, then open every picture and look closely.

For every item, list the defects you see. Name only these:
- "torn_mesh": skin torn, ripped open, or stretched into spikes, strings or long thin sheets;
- "limb_through_body": a leg, arm, wing, tail or head passing through the body or through another limb;
- "sliding_foot": a foot that should be standing on the ground sliding along it.
For a single strip also say whether the creature visibly attacks (strikes, lunges, bites, swipes) in it.
For a two-strip item say which half moves more cleanly: "left", "right", or "cant_tell" when they look
the same to you; and list each half's defects.

Answer with JSON only, no other text, in this form:
{{"items": {{"1": {{"kind": "single", "defects": [], "attacks": "yes"}},
            "2": {{"kind": "pair", "cleaner": "left", "left_defects": [], "right_defects": ["torn_mesh"]}}}}}}
Use an empty list when you see none. Do not open any file outside this folder."""


def gate_judge(name, glb, work, blender):
    """Gate 6: films of the creature's idle-or-walk and attack, a can't-tell
    control and a planted swapped-limb pair, judged blind."""
    res = {"pass": False}
    os.makedirs(work, exist_ok=True)
    pack, keyp = os.path.join(work, "pack"), os.path.join(work, "key.json")
    if os.path.exists(pack):
        if not os.path.exists(os.path.join(pack, "questions.json")):
            raise RuntimeError(f"{pack} exists and is not a judge pack")
        shutil.rmtree(pack)
    os.makedirs(pack)
    g = cg.Glb(glb)
    cl = cg.clips(g)
    move = cg.clip_named(cl, cg.WALK_WORDS) or cg.clip_named(cl, cg.IDLE_WORDS)
    att = cg.clip_named(cl, cg.ATTACK_WORDS)
    if not move or not att:
        res["why"] = "no idle or walk, or no attack, to film"
        return res
    frames = os.path.join(work, "frames")
    os.makedirs(frames, exist_ok=True)
    items = []
    films = {}
    for clip in (move["name"], att["name"]):
        info = film(blender, glb, clip, os.path.join(frames, clip), video=os.path.join(work, f"{name}_{clip}.mp4"))
        sheet = os.path.join(work, f"{name}_{clip}_sheet.png")
        tile(info["pngs"], sheet)
        films[clip] = {"sheet": sheet, "info": info}
        items.append({"category": "film", "clip": clip, "image": sheet, "attack_clip": clip == att["name"]})
    # the control: the same strip twice
    ctrl = os.path.join(work, "control.png")
    side_by_side(films[move["name"]]["sheet"], films[move["name"]]["sheet"], ctrl)
    items.append({"category": "control", "clip": move["name"], "image": ctrl, "expected": "cant_tell"})
    # the planted pair: this creature with its limbs swapped, on the clip that moves its limbs most
    planted = os.path.join(work, f"{name}_planted_swap.glb")
    cg.plant("swap", glb, planted)
    pclip = max((move, att), key=lambda c: c["turn_deg"])["name"]
    m1 = film(blender, glb, pclip, os.path.join(frames, "m1"), measure=True)
    m2 = film(blender, planted, pclip, os.path.join(frames, "m2"), measure=True)
    crop = [min(m1["crop"][0], m2["crop"][0]), max(m1["crop"][1], m2["crop"][1]),
            min(m1["crop"][2], m2["crop"][2]), max(m1["crop"][3], m2["crop"][3])]
    a = film(blender, glb, pclip, os.path.join(frames, "pc"), crop=crop)
    b = film(blender, planted, pclip, os.path.join(frames, "pp"), crop=crop)
    sa, sb = os.path.join(work, "planted_clean.png"), os.path.join(work, "planted_swap.png")
    tile(a["pngs"], sa)
    tile(b["pngs"], sb)
    rnd = random.Random(JUDGE_SEED)
    clean_left = rnd.random() < 0.5
    pair = os.path.join(work, "planted_pair.png")
    side_by_side(sa if clean_left else sb, sb if clean_left else sa, pair)
    items.append({"category": "planted", "clip": pclip, "image": pair, "expected": "left" if clean_left else "right"})
    rnd.shuffle(items)
    questions = {}
    for i, it in enumerate(items, 1):
        it["item"] = i
        shutil.copyfile(it["image"], os.path.join(pack, f"item_{i:02d}.png"))
        what = "attack" if it["category"] == "film" and it.get("attack_clip") else it["clip"]
        questions[str(i)] = {"image": f"item_{i:02d}.png",
                             "kind": "single" if it["category"] == "film" else "pair",
                             "clip": "an attack" if what == "attack" else f"the '{it['clip']}' clip"}
    json.dump({"items": questions}, open(os.path.join(pack, "questions.json"), "w"), indent=1)
    json.dump({"seed": JUDGE_SEED, "items": items}, open(keyp, "w"), indent=1)
    # the judge: a fresh session that sees the pack only
    prompt = JUDGE_PROMPT.format(n=len(items))
    jr = sh(["claude", "-p", prompt, "--model", "opus", "--allowedTools", "Read",
             "--disallowedTools", "Bash,Glob,Grep,Write,Edit,WebFetch,WebSearch,Task",
             "--output-format", "stream-json", "--verbose"], cwd=pack, timeout=1800)
    with open(os.path.join(work, "judge.jsonl"), "w") as f:
        f.write(jr["out"])
    reads, answer = [], None
    for ln in jr["out"].splitlines():
        try:
            ev = json.loads(ln)
        except ValueError:
            continue
        if ev.get("type") == "assistant":
            for c in ev.get("message", {}).get("content", []):
                if c.get("type") == "tool_use":
                    reads.append(c.get("input", {}).get("file_path", str(c.get("input"))))
        if ev.get("type") == "result":
            answer = ev.get("result")
    res["judge"] = {"cmd": "claude -p (opus, Read only) in the pack folder", "exit": jr["exit"], "reads": reads}
    outside = [p for p in reads if not os.path.abspath(os.path.join(pack, p)).startswith(os.path.abspath(pack) + os.sep)]
    res["judge"]["read_outside_pack"] = outside
    try:
        text = answer[answer.index("{"): answer.rindex("}") + 1]
        ans = json.loads(text)["items"]
    except (AttributeError, ValueError, KeyError, TypeError):
        res["why"] = "the judge gave no readable answer: " + tail(str(answer), 300)
        return res
    json.dump(ans, open(os.path.join(work, "answers.json"), "w"), indent=1)
    verdicts = []
    ok = not outside and jr["exit"] == 0
    for it in items:
        a = ans.get(str(it["item"]), {})
        if it["category"] == "control":
            good = a.get("cleaner") == "cant_tell"
            verdicts.append({"item": it["item"], "category": "control", "answer": a.get("cleaner"), "pass": good})
        elif it["category"] == "planted":
            good = a.get("cleaner") == it["expected"]
            verdicts.append({"item": it["item"], "category": "planted", "answer": a.get("cleaner"),
                             "expected": it["expected"], "pass": good})
        else:
            defects = [d for d in a.get("defects", []) if d in ("torn_mesh", "limb_through_body", "sliding_foot")]
            good = not defects and (not it.get("attack_clip") or a.get("attacks") == "yes")
            verdicts.append({"item": it["item"], "category": "film", "clip": it["clip"], "defects": defects,
                             "attacks": a.get("attacks"), "pass": good})
        ok = ok and good
    res.update({"pass": ok, "verdicts": verdicts, "films": {k: v["sheet"] for k, v in films.items()},
                "videos": [os.path.join(work, f"{name}_{c}.mp4") for c in films]})
    return res


# ------------------------------------------------------------- gate 9

def gate_game(name, glb, work):
    if os.environ.get("CREATURE_SET") == "dev":
        return {"pass": None, "why": "not run: dev set (benchmark animals never go into the game repo)"}
    ck = dep_dirs().get("charkit")
    script = os.path.join(ck, "tools/line/creature_game.sh") if ck else ""
    if not script or not os.path.exists(script):
        return {"pass": False, "why": f"charkit's creature game step not found ({script or 'no charkit checkout'})"}
    os.makedirs(work, exist_ok=True)
    r = sh(["bash", script, name, glb, work], timeout=4 * 3600)
    with open(os.path.join(work, "game.log"), "w") as f:
        f.write(r["out"] + "\n--- stderr\n" + r["err"])
    res = {"cmd": ["bash", "charkit/tools/line/creature_game.sh", name, glb, work], "exit": r["exit"],
           "secs": r["secs"], "pass": r["exit"] == 0}
    try:
        res.update(json.load(open(os.path.join(work, "game.json"))))
    except (OSError, ValueError):
        res["pass"] = False
        res["why"] = "the game step wrote no game.json: " + tail(r["err"] or r["out"], 300)
    return res


# ------------------------------------------------------------- main

def main():
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    name, model, out = sys.argv[1], os.path.abspath(sys.argv[2]), os.path.abspath(sys.argv[3])
    if not os.path.isfile(model):
        print(f"no model at {model}", file=sys.stderr)
        return 2
    blender = os.environ.get("BLENDER") or shutil.which("blender")
    os.makedirs(out, exist_ok=True)
    G = {"format": "creature-gates/1", "name": name, "model_sha256": sha256(model),
         "set": os.environ.get("CREATURE_SET", ""), "tools": os.environ.get("CREATURE_DEP_SHAS", "").split(),
         "machine": os.uname().nodename, "started": time.strftime("%Y-%m-%dT%H:%M:%S"), "gates": {}}
    gp = os.path.join(out, "gates.json")

    def save():
        with open(gp, "w") as f:
            json.dump(G, f, indent=1, default=str)

    save()
    cache = os.environ.get("CREATURE_DEPS") or os.path.join(os.path.expanduser("~"), ".cache", "creature-route")
    repos = {k: v for k, v in dep_dirs().items() if k in ("motion", "skintokens", "charkit")}
    G["gates"]["1"] = gate_tests(cache, repos)
    save()

    r1 = route(name, model, os.path.join(out, "run1"))
    G["route"] = {k: v for k, v in r1.items() if k not in ("report", "check")}
    G["route"]["picked"] = picked(r1["report"])
    clips_made = ((r1["report"].get("tries") or [{}])[0]).get("clips_made")
    if picked(r1["report"]).get("which") == "predicted":
        clips_made = (((r1["report"].get("tries") or [{}])[0]).get("predicted") or {}).get("clips_made")
    G["route"]["clips_made"] = clips_made
    final = os.path.join(out, name + ".glb")
    if r1.get("glb"):
        shutil.copyfile(r1["glb"], final)
        chk = r1["check"]
        for k in cg.CHECKS:
            c = chk["checks"][k]
            gate = str(c["gate"])
            G["gates"].setdefault(gate, {"pass": True, "checks": {}})
            G["gates"][gate]["checks"][k] = c
            G["gates"][gate]["pass"] = G["gates"][gate]["pass"] and c["pass"]
        if not chk["checks"]["idle_or_walk"]["pass"] or not chk["checks"]["attack"]["pass"]:
            G["gates"]["5"]["why"] = ("no clip maker made them: " + str((clips_made or {}).get("error")))
            if not clips_made:
                G["gates"]["5"]["why"] = "the route made no clips for this body"
    else:
        for gate in ("2", "3", "4", "5"):
            G["gates"][gate] = {"pass": False, "why": "the route left no rig: " + r1["last_line"]}
    save()

    r2 = route(name, model, os.path.join(out, "run2"))
    G["gates"]["7"] = gate_repeat(r1, r2)
    save()

    if r1.get("glb"):
        G["gates"]["8"] = gate_planted(final, os.path.join(out, "planted"))
        save()
        try:
            G["gates"]["6"] = gate_judge(name, final, os.path.join(out, "gate6"), blender)
        except (RuntimeError, OSError, ValueError) as e:
            G["gates"]["6"] = {"pass": False, "why": str(e)}
        save()
        G["gates"]["9"] = gate_game(name, final, os.path.join(out, "gate9"))
    else:
        G["gates"]["8"] = {"pass": False, "why": "no rig to plant from"}
        G["gates"]["6"] = {"pass": False, "why": "no rig to film"}
        G["gates"]["9"] = {"pass": False, "why": "no rig to put in the game"}
    G["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    counted = {k: v for k, v in G["gates"].items() if v.get("pass") is not None}
    G["pass"] = len(counted) > 0 and all(v["pass"] for v in counted.values())
    G["not_run"] = sorted(k for k, v in G["gates"].items() if v.get("pass") is None)
    G["failed"] = sorted(k for k, v in counted.items() if not v["pass"])
    save()
    for k in sorted(G["gates"], key=int):
        v = G["gates"][k]
        state = "not run" if v.get("pass") is None else ("pass" if v["pass"] else "FAIL")
        why = v.get("why") or ""
        if not why and "checks" in v:
            why = ", ".join(c for c, x in v["checks"].items() if not x["pass"])
        print(f"gate {k}: {state}{(' - ' + why) if why and state != 'pass' else ''}")
    print(f"{name}: {'PASS' if G['pass'] else 'FAIL'} -> {gp}")
    return 0 if G["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
