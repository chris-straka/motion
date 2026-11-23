"""Gate-first tests for motion/tools delegating auto-rigging cleanly to `autorig`."""

import os
import stat
import subprocess
import sys
import tempfile
import unittest

TOOLS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def _write_exe(path: str, body: str) -> None:
    with open(path, "w") as f:
        f.write("#!/usr/bin/env python3\n" + body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


class AutorigDispatchTests(unittest.TestCase):
    def test_creature_rig_delegates_to_autorig(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "autorig.log")
            fake_ar = os.path.join(d, "autorig")
            _write_exe(fake_ar, f"""
import json, sys
with open({log!r}, "a") as lf:
    lf.write(" ".join(sys.argv[1:]) + "\\n")
out = sys.argv[2]
with open(out, "wb") as f:
    f.write(b"glTF-ar")
if "--report" in sys.argv:
    with open(sys.argv[sys.argv.index("--report") + 1], "w") as rf:
        json.dump({{"best_seed": 2}}, rf)
""")
            fake_mf = os.path.join(d, "motionforge")
            _write_exe(fake_mf, """
import json, shutil, sys
clean = sys.argv[sys.argv.index("--clean-glb") + 1]
out = sys.argv[sys.argv.index("--output") + 1]
rep = sys.argv[sys.argv.index("--report") + 1]
shutil.copyfile(sys.argv[sys.argv.index("--rig") + 1], clean)
with open(out, "w") as f:
    json.dump({"views": []}, f)
with open(rep, "w") as f:
    json.dump({"score": 0.95, "checks": {"feet_on_ground": 4}}, f)
""")
            mesh = os.path.join(d, "wolf.glb")
            out_dir = os.path.join(d, "out")
            with open(mesh, "wb") as f:
                f.write(b"glTF")
            r = subprocess.run(
                [sys.executable, os.path.join(TOOLS, "creature_rig.py"),
                 mesh, out_dir, "--autorig", fake_ar, "--motionforge", fake_mf, "--seeds", "0,1"],
                capture_output=True, text=True,
            )
            self.assertEqual(r.returncode, 0, r.stderr + "\n" + r.stdout)
            with open(log) as lf:
                self.assertIn("--method auto", lf.read())
            self.assertTrue(os.path.isfile(os.path.join(out_dir, "clean.glb")))
            self.assertTrue(os.path.isfile(os.path.join(out_dir, "creature.json")))

    def test_tripo_rig_delegates_with_autorig_flag(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "ar.log")
            fake_ar = os.path.join(d, "autorig")
            _write_exe(fake_ar, f"""
import sys
with open({log!r}, "a") as lf:
    lf.write(" ".join(sys.argv[1:]) + "\\n")
with open(sys.argv[2], "wb") as f:
    f.write(b"glTF-rig")
""")
            bl = os.path.join(d, "blender")
            _write_exe(bl, """
import json, sys
with open(sys.argv[sys.argv.index("--") + 2], "wb") as f:
    f.write(b"glTF-clean")
print("TRIPO_TO_GLB " + json.dumps({"kind": "biped", "decided_by": "test", "source_tris": 100, "tris": 100, "holes": {"found": 0, "filled": 0, "left": 0}}))
""")
            mf = os.path.join(d, "motionforge")
            _write_exe(mf, """
import sys
with open(sys.argv[sys.argv.index("--output") + 1], "wb") as f:
    f.write(b"glTF-std")
""")
            rf = os.path.join(d, "rfcheck")
            _write_exe(rf, """
import json, sys
print(json.dumps({"max_tris_per_mesh": 15000} if "--print-budget" in sys.argv else {"diags": []}))
""")
            wf = os.path.join(d, "weights")
            _write_exe(wf, """
import json, shutil, sys
cmd = sys.argv[1]
if cmd == "fix":
    shutil.copyfile(sys.argv[2], sys.argv[sys.argv.index("--out") + 1])
elif cmd == "sheet":
    with open(sys.argv[sys.argv.index("--out") + 1], "wb") as f:
        f.write(b"\\x89PNG")
else:
    print(json.dumps({"pass": True, "score": 90.0, "findings": [], "class": "humanoid"}))
""")
            in_glb = os.path.join(d, "in.glb")
            with open(in_glb, "wb") as f:
                f.write(b"glTF")
            out_dir = os.path.join(d, "out")
            r = subprocess.run(
                [sys.executable, os.path.join(TOOLS, "tripo_rig.py"),
                 in_glb, out_dir, "--name", "hero", "--kind", "human", "--class", "hero",
                 "--method", "auto", "--autorig", fake_ar,
                 "--blender", bl, "--motionforge", mf, "--weights", wf, "--rfcheck", rf],
                capture_output=True, text=True,
            )
            self.assertEqual(r.returncode, 0, r.stderr + "\n" + r.stdout)
            with open(log) as lf:
                self.assertIn("--method auto", lf.read())


if __name__ == "__main__":
    unittest.main()
