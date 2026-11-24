"""Tests for Can It Code 2-cam DINOv2 tracker integration in motion/python/creature."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
CREATURE_PY = os.path.join(REPO, "python", "creature")
FIXTURE_PROFILE = os.path.join(REPO, "tests", "fixtures", "creature", "fixture_biped.json")
sys.path.insert(0, os.path.join(REPO, "python"))


def _maybe_motionforge():
    from creature import exam

    try:
        return exam.find_motionforge("")
    except SystemExit as exc:
        if os.environ.get("MF_REQUIRE_CLI"):
            raise AssertionError(str(exc)) from exc
        return None


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

    def test_exam_worse_regression_and_cli_lookup(self):
        from creature import exam

        self.assertFalse(exam.worse({"mean_mm": 10.4}, {"mean_mm": 10.0}, "mean_mm", abs_floor=2.0))
        self.assertTrue(exam.worse({"mean_mm": 12.6}, {"mean_mm": 10.0}, "mean_mm", abs_floor=2.0))
        self.assertTrue(exam.worse({"mean_deg": 2.65}, {"mean_deg": 2.0}, "mean_deg", abs_floor=0.5))

        with self.assertRaises(SystemExit):
            exam.find_motionforge(os.path.join(REPO, "nonexistent_motionforge_bin"))

        with tempfile.TemporaryDirectory() as tmp:
            fake_bin = os.path.join(tmp, "mf-bin")
            with open(fake_bin, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\nexit 0\n")
            old_env = os.environ.get("MOTIONFORGE_BIN")
            try:
                os.environ["MOTIONFORGE_BIN"] = fake_bin
                self.assertEqual(exam.find_motionforge(""), fake_bin)
            finally:
                if old_env is None:
                    os.environ.pop("MOTIONFORGE_BIN", None)
                else:
                    os.environ["MOTIONFORGE_BIN"] = old_env

        r = subprocess.run([sys.executable, os.path.join(CREATURE_PY, "exam.py"), "--help"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("--align-views", r.stdout)
        self.assertIn("--align-time", r.stdout)

    def test_detect_split_views_and_finder_math(self):
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

        stacked = torch.zeros((3, 128, 64))
        stacked[:, :64, :] = 3.0
        stacked[:, 64:, :] = 4.0
        tb = detect.split_views(stacked, 2, layout="top-bottom")
        self.assertEqual(tuple(tb[0].shape), (3, 64, 64))
        self.assertAlmostEqual(float(tb[0].mean()), 3.0)
        self.assertAlmostEqual(float(tb[1].mean()), 4.0)

        with self.assertRaises(ValueError):
            detect.split_views(composite, 2, layout="diagonal")

        # Soft-argmax decode round-trip on Gaussian targets (no HuggingFace network call).
        uv = torch.tensor([[[112.0, 224.0], [336.0, 140.0]]])
        targets = finder.gaussian_targets(uv, sigma_px=6.0)
        logits = torch.log(targets.clamp_min(1e-9))
        u, v, conf = finder.decode(logits)
        self.assertLess((u - uv[..., 0]).abs().max().item(), 1.0)
        self.assertLess((v - uv[..., 1]).abs().max().item(), 1.0)
        self.assertGreater(float(conf.min()), 0.5)

        loss = finder.heatmap_loss(logits, uv, torch.tensor([[True, False]]))
        self.assertTrue(torch.isfinite(loss))

        r = subprocess.run([sys.executable, os.path.join(CREATURE_PY, "detect.py"), "--help"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("--layout", r.stdout)

    def test_two_camera_correspondence_and_alignment_cli(self):
        mf = _maybe_motionforge()
        if mf is None:
            self.skipTest("motionforge binary not built (set MF_REQUIRE_CLI=1 to enforce)")
        tmp = tempfile.mkdtemp(prefix="mf_creature_tracker_")
        try:
            glb = os.path.join(tmp, "fixture.glb")
            truth = os.path.join(tmp, "truth.json")
            truth_other = os.path.join(tmp, "truth_other.json")
            det = os.path.join(tmp, "det.json")
            det_other = os.path.join(tmp, "det_other.json")
            for argv in (
                ["fixture-glb", "--output", glb],
                ["creature-motion", "--creature", FIXTURE_PROFILE, "--output", truth, "--frames", "24", "--seed", "3"],
                ["creature-motion", "--creature", FIXTURE_PROFILE, "--output", truth_other, "--frames", "24", "--seed", "11"],
                ["creature-project", "--creature", FIXTURE_PROFILE, "--input", truth, "--output", det, "--noise-px", "1"],
                ["creature-project", "--creature", FIXTURE_PROFILE, "--input", truth_other, "--output", det_other, "--noise-px", "1"],
            ):
                subprocess.run([mf] + argv, check=True, capture_output=True, text=True)

            # 1) Consistent two-camera detections solve cleanly to clip + GLB and score accurately.
            solved = os.path.join(tmp, "solved.json")
            solved_glb = os.path.join(tmp, "solved.glb")
            report_path = os.path.join(tmp, "solve_report.json")
            scores_path = os.path.join(tmp, "scores.json")
            r_ok = subprocess.run(
                [mf, "creature-solve", "--creature", FIXTURE_PROFILE, "--detections", det,
                 "--output", solved, "--glb-in", glb, "--glb-out", solved_glb,
                 "--clip-name", "tracker_test", "--report", report_path],
                capture_output=True, text=True,
            )
            self.assertEqual(r_ok.returncode, 0, r_ok.stderr)
            with open(report_path, encoding="utf-8") as f:
                rep = json.load(f)
            self.assertTrue(rep["consistent"])
            self.assertLess(rep["median_gap_ratio"], 0.08)
            self.assertTrue(os.path.isfile(solved_glb) and os.path.getsize(solved_glb) > 0)

            subprocess.run(
                [mf, "creature-compare", "--creature", FIXTURE_PROFILE,
                 "--truth", truth, "--solved", solved, "--output", scores_path],
                check=True, capture_output=True, text=True,
            )
            with open(scores_path, encoding="utf-8") as f:
                scores = json.load(f)
            self.assertLess(scores["mean_mm"], 25.0)

            # 2) Mismatched two-camera motion (side camera from a different seed) is rejected.
            with open(det, encoding="utf-8") as f:
                det_a = json.load(f)
            with open(det_other, encoding="utf-8") as f:
                det_b = json.load(f)
            mismatched = dict(det_a)
            mismatched["frames"] = [[fr_a[0], fr_b[1]] for fr_a, fr_b in zip(det_a["frames"], det_b["frames"])]
            det_mismatch = os.path.join(tmp, "det_mismatch.json")
            with open(det_mismatch, "w", encoding="utf-8") as f:
                json.dump(mismatched, f)
            r_bad = subprocess.run(
                [mf, "creature-solve", "--creature", FIXTURE_PROFILE, "--detections", det_mismatch,
                 "--output", os.path.join(tmp, "bad.json")],
                capture_output=True, text=True,
            )
            self.assertNotEqual(r_bad.returncode, 0, "mismatched two-camera motions must fail correspondence check")

            # 3) Misaligned second camera (zoom + shift + 2-frame lag) recovers via --align-views --align-time auto.
            warped = dict(det_a)
            n_fr = len(det_a["frames"])
            lag = 2
            warped_frames = []
            for i in range(n_fr):
                src_i = max(0, min(n_fr - 1, i - lag))
                side = [[(u - 256.0) * 1.08 + 256.0 + 14.0, (v - 256.0) * 1.08 + 256.0 - 10.0, conf]
                        for u, v, conf in det_a["frames"][src_i][1]]
                warped_frames.append([det_a["frames"][i][0], side])
            warped["frames"] = warped_frames
            det_warped = os.path.join(tmp, "det_warped.json")
            with open(det_warped, "w", encoding="utf-8") as f:
                json.dump(warped, f)

            aligned_report = os.path.join(tmp, "aligned_report.json")
            r_align = subprocess.run(
                [mf, "creature-solve", "--creature", FIXTURE_PROFILE, "--detections", det_warped,
                 "--output", os.path.join(tmp, "aligned.json"),
                 "--align-views", "--align-time", "auto", "--report", aligned_report],
                capture_output=True, text=True,
            )
            self.assertEqual(r_align.returncode, 0, r_align.stderr)
            with open(aligned_report, encoding="utf-8") as f:
                arep = json.load(f)
            self.assertTrue(arep["consistent"])
            self.assertIn("align", arep)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
