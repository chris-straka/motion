"""Tests for Blender 5.2+ Action slot binding in tools/bench/motion_video_render.py."""
import ast
import types
import unittest
from pathlib import Path

RENDER_PY = Path(__file__).resolve().parents[1] / "bench" / "motion_video_render.py"


def load_slot_helpers():
    """Extract pure-Python `pick_armature_slot` and `bind_action` without importing bpy."""
    tree = ast.parse(RENDER_PY.read_text())
    wanted = {"pick_armature_slot", "bind_action"}
    mod = ast.Module(
        body=[n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in wanted],
        type_ignores=[],
    )
    ns = {}
    exec(compile(mod, str(RENDER_PY), "exec"), ns)
    return ns["pick_armature_slot"], ns["bind_action"]


class FakeAnimData:
    def __init__(self):
        self.action = None
        self.action_slot = None


class FakeObj:
    def __init__(self, name):
        self.name = name
        self.animation_data = None

    def animation_data_create(self):
        self.animation_data = FakeAnimData()
        return self.animation_data


class SlotBindingTests(unittest.TestCase):
    def test_picks_pose_bones_slot_over_empty_slot_zero(self):
        pick_armature_slot, bind_action = load_slot_helpers()
        s_finger = types.SimpleNamespace(name_display="Finger3_L", identifier="OBFinger3_L")
        s_arm = types.SimpleNamespace(name_display="Armature", identifier="OBArmature")
        cb_finger = types.SimpleNamespace(
            slot=s_finger,
            fcurves=[types.SimpleNamespace(data_path="rotation_quaternion")],
        )
        cb_arm = types.SimpleNamespace(
            slot=s_arm,
            fcurves=[types.SimpleNamespace(data_path='pose.bones["DEF-upper_arm.R"].rotation_quaternion')],
        )
        act = types.SimpleNamespace(
            slots=[s_finger, s_arm],
            layers=[types.SimpleNamespace(strips=[types.SimpleNamespace(channelbags=[cb_finger, cb_arm])])],
        )
        arm = FakeObj("Armature")
        finger = FakeObj("Finger3_L")

        self.assertIs(pick_armature_slot(arm, act), s_arm)
        bind_action(arm, act, [finger, arm])
        self.assertIs(arm.animation_data.action_slot, s_arm)
        self.assertIs(finger.animation_data.action_slot, s_finger)


if __name__ == "__main__":
    unittest.main()
