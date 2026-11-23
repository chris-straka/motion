"""Regression tests for motion Wave 0 P0 ignored subprocess exit codes:

1. tools/creature_gates.py:417 must require exit code 0 on `creature-bench anatomy`
   even if anatomy.json contains `{"pass": true, "findings": []}`.
2. tools/tripo_rig.py:389 (`check()`) must not report pass when `weights check`
   exits non-zero (especially >= 2 even with JSON stdout).
3. tools/tripo_rig.py:370 (`weights sheet`) must check the return code and fail
   loudly (exit 2) if `weights sheet` exits >= 2 or omits the output PNG.
"""
import os
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(TOOLS)
sys.path.insert(0, TOOLS)

import creature_gates as cg  # noqa: E402
import tripo_rig  # noqa: E402


def write_exec(path, body):
    with open(path, "w") as f:
        f.write("#!/usr/bin/env python3\n" + body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


class CreatureGatesExitCodes(unittest.TestCase):
    def test_anatomy_nonzero_exit_fails_even_with_passing_json(self):
        with tempfile.TemporaryDirectory() as d:
            bins = os.path.join(d, "bins")
            os.makedirs(bins)
            rf = os.path.join(bins, "rfcheck")
            cr = os.path.join(bins, "creature-rig")
            cb = os.path.join(bins, "creature-bench")
            wf = os.path.join(bins, "weights")

            write_exec(rf, "import sys; print('{\"diags\": [], \"warns\": []}'); sys.exit(0)\n")
            write_exec(cr, """
import json, os, sys
sj = sys.argv[sys.argv.index("--json") + 1]
rom = sys.argv[sys.argv.index("--rom") + 1]
json.dump({"pass": True, "score": 1.0, "kind": "quadruped", "bones": 10, "findings": []}, open(sj, "w"))
open(rom, "w").write("PoseSet(name: \\"r\\", poses: [])")
sys.exit(0)
""")
            # Deceptive creature-bench: writes pass=True JSON yet exits 2!
            write_exec(cb, """
import json, sys
aj = sys.argv[sys.argv.index("--json") + 1]
json.dump({"pass": True, "findings": [], "limb_pairs": 2, "head": True}, open(aj, "w"))
sys.exit(2)
""")
            write_exec(wf, "import json, sys; print(json.dumps({'pass': True, 'score': 1.0, 'verts': 10, 'poses': 1, 'findings': []})); sys.exit(0)\n")

            old_env = {k: os.environ.get(k) for k in ("RFCHECK_BIN", "CREATURE_RIG_BIN", "CREATURE_BENCH_BIN", "WEIGHTS_BIN")}
            os.environ["RFCHECK_BIN"] = rf
            os.environ["CREATURE_RIG_BIN"] = cr
            os.environ["CREATURE_BENCH_BIN"] = cb
            os.environ["WEIGHTS_BIN"] = wf
            try:
                res = cg.check(os.path.join(cg.FIXTURES, "clean.glb"), os.path.join(d, "work"), quiet=True)
            finally:
                for k, v in old_env.items():
                    if v is None:
                        os.environ.pop(k, None)
                    else:
                        os.environ[k] = v

            anat = res["checks"]["anatomy"]
            self.assertFalse(anat["pass"], f"anatomy passed despite exit code 2: {anat}")
            self.assertEqual(anat["exit"], 2)


class TripoRigExitCodes(unittest.TestCase):
    def test_check_fails_on_nonzero_exit_despite_passing_json(self):
        with tempfile.TemporaryDirectory() as d:
            wf = os.path.join(d, "weights")
            write_exec(wf, """
import json, sys
print(json.dumps({"pass": True, "score": 99.0, "findings": [], "class": "humanoid"}))
sys.exit(2)
""")
            res = tripo_rig.check(wf, os.path.join(d, "dummy.glb"))
            self.assertFalse(res["pass"], f"check() passed despite weights exit 2: {res}")

    def test_sheet_failure_exits_two(self):
        with tempfile.TemporaryDirectory() as d:
            bins = os.path.join(d, "bins")
            os.makedirs(bins)
            bl = os.path.join(bins, "blender")
            sk = os.path.join(bins, "skintokens")
            mf = os.path.join(bins, "motionforge")
            wf = os.path.join(bins, "weights")
            rf = os.path.join(bins, "rfcheck")

            write_exec(bl, """
import json, sys
out = sys.argv[sys.argv.index("--") + 2]
open(out, "wb").write(b"glTF")
print("TRIPO_TO_GLB " + json.dumps({"kind": "biped", "decided_by": "test", "source_tris": 100, "tris": 100, "holes": {"found": 0, "filled": 0, "left": 0}}))
sys.exit(0)
""")
            write_exec(sk, """
import sys
open(sys.argv[3], "wb").write(b"glTF")
sys.exit(0)
""")
            write_exec(mf, """
import sys
out = sys.argv[sys.argv.index("--output") + 1]
open(out, "wb").write(b"glTF")
sys.exit(0)
""")
            write_exec(rf, """
import json, sys
if "--print-budget" in sys.argv:
    print(json.dumps({"max_tris_per_mesh": 15000}))
else:
    print(json.dumps({"diags": []}))
sys.exit(0)
""")
            write_exec(wf, """
import json, sys
cmd = sys.argv[1]
if cmd == "check":
    print(json.dumps({"pass": True, "score": 95.0, "findings": [], "class": "humanoid"}))
    sys.exit(0)
if cmd == "sheet":
    print("simulated weights sheet error", file=sys.stderr)
    sys.exit(2)
sys.exit(0)
""")
            model = os.path.join(d, "in.glb")
            with open(model, "wb") as fh:
                fh.write(b"glTF")
            out_dir = os.path.join(d, "out")
            r = subprocess.run([
                sys.executable, os.path.join(TOOLS, "tripo_rig.py"),
                model, out_dir, "--name", "hero", "--kind", "human", "--class", "hero",
                "--seeds", "1", "--blender", bl, "--skintokens", sk,
                "--motionforge", mf, "--weights", wf, "--rfcheck", rf,
            ], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2, f"tripo_rig ignored failing weights sheet (rc={r.returncode}):\n{r.stdout}\n{r.stderr}")


if __name__ == "__main__":
    unittest.main()
