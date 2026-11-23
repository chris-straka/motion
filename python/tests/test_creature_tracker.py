"""Tests for Can It Code 2-cam DINOv2 tracker integration in motion/python/creature."""

import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
CREATURE_PY = os.path.join(REPO, "python", "creature")
sys.path.insert(0, os.path.join(REPO, "python"))


class CreatureTrackerIntegrationTests(unittest.TestCase):
    def test_module_files_and_docs_present(self):
        for fname in ("__init__.py", "finder.py", "detect.py", "exam.py", "render.py", "train.py"):
            self.assertTrue(os.path.isfile(os.path.join(CREATURE_PY, fname)), f"missing {fname}")
        with open(os.path.join(REPO, "docs", "creature.md"), encoding="utf-8") as f:
            doc = f.read()
        self.assertIn("python/creature", doc)
        self.assertIn("rust/core/src/creature", doc)
        self.assertIn("skintokens/tools/bench/exam.sh", doc)

    def test_finder_docstring_matches_128_heatmap_grid(self):
        with open(os.path.join(CREATURE_PY, "finder.py"), encoding="utf-8") as f:
            src = f.read()
        self.assertIn("128x128", src)
        self.assertNotIn("112x112", src)

    def test_exam_worse_regression_threshold(self):
        from creature import exam

        self.assertFalse(exam.worse({"mean_mm": 10.4}, {"mean_mm": 10.0}, "mean_mm", abs_floor=2.0))
        self.assertTrue(exam.worse({"mean_mm": 12.6}, {"mean_mm": 10.0}, "mean_mm", abs_floor=2.0))
        self.assertTrue(exam.worse({"mean_deg": 2.65}, {"mean_deg": 2.0}, "mean_deg", abs_floor=0.5))

    def test_detect_split_views_and_cli_validation(self):
        try:
            import torch
            from creature import detect, finder
        except ImportError:
            self.skipTest("torch not installed in this Python")
        self.assertEqual(finder.GRID, 32)
        self.assertEqual(finder.HEAT, 128)
        composite = torch.zeros((3, 64, 128))
        composite[:, :, :64] = 1.0
        composite[:, :, 64:] = 2.0
        views = detect.split_views(composite, 2)
        self.assertEqual(len(views), 2)
        self.assertEqual(tuple(views[0].shape), (3, 64, 64))
        self.assertAlmostEqual(float(views[0].mean()), 1.0)
        self.assertAlmostEqual(float(views[1].mean()), 2.0)

        r = subprocess.run([sys.executable, os.path.join(CREATURE_PY, "detect.py"), "--help"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
