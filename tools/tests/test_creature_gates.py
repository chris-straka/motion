"""The gate script's own measures on the committed fixtures (no tools
needed): triangles, holes, weight sums and influences, clips, and that
each planted file breaks what it says it breaks."""
import hashlib
import json
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import creature_gates as cg  # noqa: E402

FX = cg.FIXTURES


def glb(name):
    return cg.Glb(os.path.join(FX, name))


class Fixtures(unittest.TestCase):
    def test_pinned(self):
        with open(os.path.join(FX, "expected.json")) as fh:
            exp = json.load(fh)
        files = [(exp["clean"], exp["clean_sha256"])] + [(v["file"], v["sha256"]) for v in exp["planted"].values()]
        for name, want in files:
            with open(os.path.join(FX, name), "rb") as fh:
                self.assertEqual(hashlib.sha256(fh.read()).hexdigest(), want, name)

    def test_clean_measures(self):
        g = glb("clean.glb")
        self.assertEqual(cg.count_tris(g), 7700)
        self.assertEqual(cg.open_rims(g), [])
        w = cg.weight_stats(g)
        self.assertEqual((w["verts_off_sum"], w["verts_over_4"]), (0, 0))
        self.assertLessEqual(w["most_influences"], 4)
        cl = cg.clips(g)
        self.assertEqual(cg.clip_named(cl, cg.WALK_WORDS)["name"], "walk")
        self.assertGreaterEqual(cg.clip_named(cl, cg.ATTACK_WORDS)["turn_deg"], cg.ATTACK_MIN_TURN)

    def test_planted_measures(self):
        self.assertEqual(len(cg.open_rims(glb("hole.glb"))), 1)
        self.assertEqual(cg.weight_stats(glb("influence5.glb"))["verts_over_4"], 1)
        self.assertEqual(cg.weight_stats(glb("influence5.glb"))["verts_off_sum"], 0)
        self.assertGreater(cg.count_tris(glb("overbudget.glb")), cg.TRI_BUDGET)
        self.assertEqual(len(cg.mirror_partners(glb("clean.glb"))), 10)

    def test_plant_round_trip(self):
        # planting again from the clean fixture gives the same measures
        with tempfile.TemporaryDirectory() as d:
            for kind in ("hole", "influence5", "overbudget", "swap"):
                out = os.path.join(d, kind + ".glb")
                cg.plant(kind, os.path.join(FX, "clean.glb"), out)
                g = cg.Glb(out)
                if kind == "hole":
                    self.assertTrue(cg.open_rims(g))
                if kind == "influence5":
                    self.assertEqual(cg.weight_stats(g)["verts_over_4"], 1)
                if kind == "overbudget":
                    self.assertGreater(cg.count_tris(g), cg.TRI_BUDGET)
                    self.assertEqual(cg.weight_stats(g)["verts_off_sum"], 0)

    def test_game_clip_rule(self):
        cl = [{"name": "walk_spot", "seconds": 1, "turn_deg": 9}, {"name": "Walk", "seconds": 1, "turn_deg": 9},
              {"name": "bite_2", "seconds": 1, "turn_deg": 20}]
        self.assertEqual(cg.clip_named(cl, cg.WALK_WORDS)["name"], "Walk")
        self.assertEqual(cg.clip_named(cl, cg.ATTACK_WORDS)["name"], "bite_2")
        self.assertIsNone(cg.clip_named(cl, cg.IDLE_WORDS))


if __name__ == "__main__":
    unittest.main()
