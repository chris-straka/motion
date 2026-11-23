# SPDX-License-Identifier: GPL-3.0-or-later
"""Pure tests for the bpy-free extension modules (quats, cli arg builders).

Run with system Python (no Blender, stdlib only):
    python3 -m unittest discover -s blender/tests -p "test_pure.py"
"""

import importlib.util
import json
import os
import stat
import struct
import tempfile
import unittest


def _load(name):
    path = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "motionforge", name + ".py"
    )
    spec = importlib.util.spec_from_file_location("motionforge_" + name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# Loaded by file path: importing the package would execute __init__.py,
# which needs bpy.
cli = _load("cli")
quats = _load("quats")
slot_contract = _load("slot_contract")


class TestQuats(unittest.TestCase):
    def assertRowsAlmostEqual(self, a, b, eps=1e-9):
        for ra, rb in zip(a, b):
            for x, y in zip(ra, rb):
                self.assertLess(abs(x - y), eps)

    def test_rest_basis_matches_rust(self):
        # Same empirical Blender 5.2 roll-0 bases the Rust core pins.
        cases = [
            ((1, 0, 0), ((0, -1, 0), (1, 0, 0), (0, 0, 1))),
            ((0, 1, 0), ((1, 0, 0), (0, 1, 0), (0, 0, 1))),
            ((0, -1, 0), ((-1, 0, 0), (0, -1, 0), (0, 0, 1))),
            ((0, 0, 1), ((1, 0, 0), (0, 0, 1), (0, -1, 0))),
            ((0, 0, -1), ((1, 0, 0), (0, 0, -1), (0, 1, 0))),
        ]
        for direction, cols in cases:
            rows = quats.rest_basis((0, 0, 0), direction)
            got_cols = tuple(tuple(rows[r][c] for r in range(3)) for c in range(3))
            for got, want in zip(got_cols, cols):
                for g, w in zip(got, want):
                    self.assertLess(abs(g - w), 1e-9)

    def test_max_abs_diff(self):
        ident = ((1, 0, 0), (0, 1, 0), (0, 0, 1))
        self.assertEqual(quats.max_abs_diff(ident, ident), 0.0)
        other = ((1, 0, 0), (0, 1, 0), (0, 0.5, 1))
        self.assertAlmostEqual(quats.max_abs_diff(ident, other), 0.5)


class TestCliBuilders(unittest.TestCase):
    def test_retarget_args(self):
        args = cli.build_retarget_args("s", "t", "m", "o", yaw="flip", pin=False)
        self.assertEqual(
            args,
            ["retarget", "--source", "s", "--target", "t", "--map", "m",
             "--output", "o", "--yaw", "flip", "--no-pin"],
        )

    def test_stylize_args(self):
        params = {
            "exaggeration": 1.5,
            "chain_factors": [("arm", 1.8)],
            "angle_threshold": 0.2,
            "min_spacing": 3,
            "hold": 1,
            "anticipation": 0.0,
            "anticipation_frames": 2,
            "overshoot": 0.0,
        }
        args = cli.build_stylize_args("i", "o", params)
        self.assertIn("--exaggeration", args)
        self.assertIn("arm:1.8", args)
        self.assertEqual(args[0], "stylize")
        self.assertNotIn("--keys-out", args)
        keyed = cli.build_stylize_args("i", "o", params, keys_out="k")
        self.assertIn("--keys-out", keyed)
        self.assertIn("k", keyed)

    def test_physics_args(self):
        params = {
            "root": "R", "feet": ["FL", "FR"], "contact_margin": 0.03,
            "foot_radius": 0.12, "balance_margin": 0.05, "min_air_frames": 4,
            "accel_limit": 25.0, "turn_deg": 40.0, "min_turn_speed": 0.5,
            "blend_frames": 2, "smooth_sigma": 1.0, "smooth_pad": 2,
            "fix_ballistic": True, "fix_momentum": False,
        }
        check = cli.build_physics_args("physics-check", "i", None, params)
        self.assertNotIn("--output", check)
        frame = cli.build_physics_args("physics-frame", "i", None, params, frame=3)
        self.assertIn("--frame", frame)
        self.assertIn("3", frame)
        with self.assertRaises(ValueError):
            cli.build_physics_args("physics-frame", "i", None, params)
        fix = cli.build_physics_args("physics-fix", "i", "o", params)
        self.assertIn("--no-momentum", fix)
        self.assertNotIn("--no-ballistic", fix)
        self.assertNotIn("--no-balance", fix)
        self.assertNotIn("--max-lean-deg", fix)
        self.assertIn("FL,FR", fix)
        fix = cli.build_physics_args("physics-fix", "i", "o",
                                     dict(params, fix_balance=False, max_lean_deg=12.5))
        self.assertIn("--no-balance", fix)
        self.assertEqual(fix[fix.index("--max-lean-deg") + 1], "12.5")
        # Balance flags are fix-only; check/frame never get them.
        check = cli.build_physics_args("physics-check", "i", None,
                                       dict(params, fix_balance=False, max_lean_deg=12.5))
        self.assertNotIn("--no-balance", check)
        self.assertNotIn("--max-lean-deg", check)

    def test_parsers(self):
        self.assertEqual(cli.parse_chain_factors(""), [])
        self.assertEqual(cli.parse_chain_factors("arm:1.8, spine:1.2"),
                         [("arm", 1.8), ("spine", 1.2)])
        with self.assertRaises(ValueError):
            cli.parse_chain_factors("arm")
        with self.assertRaises(ValueError):
            cli.parse_chain_factors("arm:xyz")
        self.assertEqual(cli.parse_feet("FL, FR"), ["FL", "FR"])
        with self.assertRaises(ValueError):
            cli.parse_feet("  ")

    def test_find_binary_explicit(self):
        with tempfile.NamedTemporaryFile(delete=False) as f:
            path = f.name
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
        try:
            self.assertEqual(cli.find_motionforge_binary(path), path)
        finally:
            os.unlink(path)
        # A bogus explicit path falls through to PATH/build-tree search,
        # never returned as-is.
        self.assertNotEqual(cli.find_motionforge_binary("/nonexistent"), "/nonexistent")


class TestRunCliStderr(unittest.TestCase):
    def _make_script(self, body):
        f = tempfile.NamedTemporaryFile("w", suffix=".py", delete=False)
        f.write("#!/usr/bin/env python3\n" + body)
        f.close()
        os.chmod(f.name, os.stat(f.name).st_mode | stat.S_IXUSR)
        return f.name

    def test_exit_zero_with_stderr_succeeds(self):
        script = self._make_script(
            "import sys\nsys.stdout.write('ok\\n')\nsys.stderr.write('wall: 1.2 ms\\n')\nsys.exit(0)\n"
        )
        try:
            out = cli.run_cli(script, [])
            self.assertEqual(out.strip(), "ok")
        finally:
            os.unlink(script)

    def test_nonzero_exit_raises_cli_error(self):
        script = self._make_script("import sys\nsys.stderr.write('boom\\n')\nsys.exit(2)\n")
        try:
            with self.assertRaises(cli.CliError):
                cli.run_cli(script, [])
        finally:
            os.unlink(script)


class TestSlotContract(unittest.TestCase):
    def test_canonical_and_suffix_rejection(self):
        self.assertEqual(slot_contract.CANONICAL_SLOTS, ("hero", "weapon", "shield", "helper"))
        for ok in ("hero", "weapon", "shield", "helper", "sword_prop"):
            self.assertFalse(slot_contract.is_suffixed_slot_name(ok))
            self.assertEqual(slot_contract.validate_slot_name(ok), ok)
        for bad in ("weapon_1", "weapon_2", "hero.001", "shield.002", "helper_01", "", " hero "):
            with self.assertRaises(slot_contract.SlotContractError):
                slot_contract.validate_slot_name(bad)
        with self.assertRaises(slot_contract.SlotContractError):
            slot_contract.validate_slot_name("custom_prop", allowed_slots=slot_contract.CANONICAL_SLOTS)

    def test_expected_and_duplicate_slots(self):
        self.assertEqual(
            slot_contract.validate_slot_names(
                ["hero", "weapon"],
                expected_slots=("hero", "weapon"),
                allowed_slots=slot_contract.CANONICAL_SLOTS,
            ),
            ["hero", "weapon"],
        )
        for bad_list in (
            ["hero", "Hero"],
            ["hero"],
            ["hero", "shield"],
            ["hero", "weapon", "weapon_1"],
        ):
            with self.assertRaises(slot_contract.SlotContractError):
                slot_contract.validate_slot_names(bad_list, expected_slots=("hero", "weapon"))

    def test_timing_parity_and_cue_sync(self):
        slot_contract.validate_slot_timings(
            {"hero": [1.0, 6.0, 12.0], "weapon": [1.0, 6.0, 12.0]},
            cue_frames={"start": 1, "hit": 6, "end": 12},
        )
        # Frame range drift (weapon shifted +1 frame)
        with self.assertRaises(slot_contract.SlotContractError):
            slot_contract.validate_slot_timings(
                {"hero": [1.0, 6.0, 12.0], "weapon": [2.0, 7.0, 13.0]},
                cue_frames={"start": 1, "hit": 6, "end": 12},
            )
        # Same endpoints, hit cue drifted (6 vs 7)
        with self.assertRaises(slot_contract.SlotContractError):
            slot_contract.validate_slot_timings(
                {"hero": [1.0, 6.0, 12.0], "weapon": [1.0, 7.0, 12.0]},
                cue_frames={"start": 1, "hit": 6, "end": 12},
            )
        # Empty slot rejected
        with self.assertRaises(slot_contract.SlotContractError):
            slot_contract.validate_slot_timings({"hero": [1.0, 6.0], "weapon": []})

    def _write_test_glb(self, path, hero_frames, weapon_frames, slot_names=("hero", "weapon")):
        fps = 30.0
        bin_buf = bytearray()

        def push_times(frames):
            off = len(bin_buf)
            for f in frames:
                bin_buf.extend(struct.pack("<f", float(f) / fps))
            return off, len(frames)

        off_h, cnt_h = push_times(hero_frames)
        off_w, cnt_w = push_times(weapon_frames)
        while len(bin_buf) % 4 != 0:
            bin_buf.append(0)
        doc = {
            "asset": {"version": "2.0"},
            "scenes": [{
                "nodes": [0, 1],
                "extras": {"motionforge_slotted": {"action": "Slash", "fps": 30, "fps_base": 1.0}},
            }],
            "nodes": [
                {"name": "HeroArm", "extras": {"motionforge_slot": slot_names[0]}},
                {"name": "WeaponProp", "extras": {"motionforge_slot": slot_names[1]}},
            ],
            "buffers": [{"byteLength": len(bin_buf)}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": off_h, "byteLength": cnt_h * 4},
                {"buffer": 0, "byteOffset": off_w, "byteLength": cnt_w * 4},
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": cnt_h, "type": "SCALAR"},
                {"bufferView": 1, "componentType": 5126, "count": cnt_w, "type": "SCALAR"},
            ],
            "animations": [{
                "name": "Slash",
                "samplers": [{"input": 0, "output": 0}, {"input": 1, "output": 1}],
                "channels": [
                    {"sampler": 0, "target": {"node": 0, "path": "rotation"}},
                    {"sampler": 1, "target": {"node": 1, "path": "translation"}},
                ],
            }],
        }
        jbytes = json.dumps(doc).encode("utf-8")
        while len(jbytes) % 4 != 0:
            jbytes += b" "
        total = 12 + 8 + len(jbytes) + 8 + len(bin_buf)
        with open(path, "wb") as fh:
            fh.write(struct.pack("<III", 0x46546C67, 2, total))
            fh.write(struct.pack("<II", len(jbytes), 0x4E4F534A))
            fh.write(jbytes)
            fh.write(struct.pack("<II", len(bin_buf), 0x004E4942))
            fh.write(bin_buf)

    def test_glb_slot_timings_and_cli_exit_codes(self):
        with tempfile.TemporaryDirectory() as td:
            ok_glb = os.path.join(td, "ok.glb")
            self._write_test_glb(ok_glb, [1, 6, 12], [1, 6, 12])
            self.assertEqual(
                slot_contract.main(["validate", ok_glb, "--expect", "hero,weapon", "--cues", "1,6,12", "--canonical-only"]),
                0,
            )

            bad_suffix = os.path.join(td, "bad_suffix.glb")
            self._write_test_glb(bad_suffix, [1, 6, 12], [1, 6, 12], slot_names=("hero", "weapon_1"))
            self.assertNotEqual(
                slot_contract.main(["validate", bad_suffix, "--expect", "hero,weapon"]),
                0,
            )

            bad_drift = os.path.join(td, "bad_drift.glb")
            self._write_test_glb(bad_drift, [1, 6, 12], [1, 7, 12])
            self.assertNotEqual(
                slot_contract.main(["validate", bad_drift, "--expect", "hero,weapon", "--cues", "1,6,12"]),
                0,
            )


if __name__ == "__main__":
    unittest.main()
