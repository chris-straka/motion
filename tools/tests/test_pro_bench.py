"""Tests for tools/bench/pro_bench.py (motion-8: held-out set + planted bad clip + no machine pin)."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BENCH_DIR = ROOT / "tools" / "bench"
PRO_BENCH = BENCH_DIR / "pro_bench.py"
PRO_FLEET = BENCH_DIR / "pro_fleet.sh"

sys.path.insert(0, str(BENCH_DIR))
import pro_bench  # noqa: E402


def _clean_row(stage="clean"):
    return {
        "plant_iou": 0.96,
        "hold_cm": 1.2,
        "hold_pro_cm": 1.5,
        "hold_worst_cm": 0.0,
        "skate_cm": 1.0,
        "skate_pro_cm": 1.2,
        "sink_new": 0,
        "float_extra": 0,
        "ground_cm": 0.1,
        "through_new": 0,
        "limits_new": 0,
        "limits_which": {},
        "sink_cm": 0.0,
        "rough_mm": 0.4,
        "rough_pro_mm": 0.5,
        "keys_frames": 0.0,
        "lag_frames": 0,
        "pose_deg": 2.0,
        "pose_joint": "knee.L",
        "fail": [],
    }


class ProBenchMotion8Tests(unittest.TestCase):
    def test_cli_supports_lib_heldout_and_planted_subcommand(self):
        r_run = subprocess.run(
            [sys.executable, str(PRO_BENCH), "run", "--help"],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertIn("--lib", r_run.stdout)
        self.assertIn("--heldout", r_run.stdout)
        r_pl = subprocess.run(
            [sys.executable, str(PRO_BENCH), "planted", "--help"],
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(r_pl.returncode, 0, r_pl.stderr)

    def test_report_separates_dev_and_heldout_sets(self):
        with tempfile.TemporaryDirectory() as td:
            res = {
                "commit": "abc1234",
                "date": "2026-11-19T21:00:00",
                "stages": ["raw", "retarget", "clean"],
                "heldout": ["mixamo"],
                "tol": pro_bench.TOL,
                "targets": {
                    "mannequin_f": {
                        "height_m": 1.81,
                        "clips": {
                            "ual1/Walk_Loop": {
                                "raw": _clean_row("raw"),
                                "retarget": _clean_row("retarget"),
                                "clean": _clean_row("clean"),
                            },
                            "mixamo/Walking": {
                                "raw": _clean_row("raw"),
                                "retarget": _clean_row("retarget"),
                                "clean": _clean_row("clean"),
                            },
                        },
                    }
                },
            }
            (Path(td) / "results.json").write_text(json.dumps(res))
            pro_bench.report(td)
            md = (Path(td) / "report.md").read_text()
            low = md.lower()
            self.assertIn("dev", low, "report.md must include a dev-set section")
            self.assertIn("held-out", low, "report.md must include a held-out section")
            self.assertIn("mixamo", low, "report.md must report the held-out mixamo set")

    def test_planted_bad_clip_fails_compare(self):
        self.assertTrue(
            hasattr(pro_bench, "check_planted_bad"),
            "pro_bench.py must expose check_planted_bad(out_dir=None) that verifies a planted bad clip fails",
        )
        with tempfile.TemporaryDirectory() as td:
            res = pro_bench.check_planted_bad(td)
            self.assertIsInstance(res, dict)
            self.assertTrue(res.get("ok"), f"planted bad clip check failed: {res}")
            self.assertTrue(res.get("failed_checks"), f"planted bad clip must fail >= 1 check: {res}")

    def test_pro_fleet_has_no_machine_pin(self):
        text = PRO_FLEET.read_text()
        self.assertNotIn("--on f", text, "pro_fleet.sh must not pin --on f")
        self.assertNotIn("--on ", text, "pro_fleet.sh must not pin any machine with --on")


if __name__ == "__main__":
    unittest.main()
