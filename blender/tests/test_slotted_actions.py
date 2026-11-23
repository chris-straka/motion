# SPDX-License-Identifier: GPL-3.0-or-later
"""Headless Blender tests for MotionForge Slotted Actions contract & GLB round-trip.

    /Applications/Blender.app/Contents/MacOS/Blender --background \
        --factory-startup --python-exit-code 1 \
        --python blender/tests/test_slotted_actions.py
"""

import os
import shutil
import subprocess
import sys
import tempfile

import bpy
from mathutils import Matrix

REPO = os.path.normpath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
)
sys.path.insert(0, os.path.join(REPO, "blender"))

from motionforge import clip_io, slot_contract, slotted  # noqa: E402


def check(condition, message):
    print(("PASS" if condition else "FAIL") + ": " + message)
    if not condition:
        raise SystemExit(1)


def expect_error(label, fn):
    try:
        fn()
    except (clip_io.MotionforgeError, slot_contract.SlotContractError, ValueError) as exc:
        print(f"PASS: {label} rejected ({exc})")
        return exc
    check(False, f"{label} should have failed loudly")


def clean_scene():
    if bpy.context.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for coll in (
        bpy.data.meshes,
        bpy.data.armatures,
        bpy.data.actions,
        bpy.data.objects,
    ):
        for item in list(coll):
            coll.remove(item)


def make_hero_armature(name="HeroRig"):
    bpy.ops.object.armature_add(enter_editmode=False, location=(0, 0, 0))
    arm = bpy.context.view_layer.objects.active
    arm.name = name
    bpy.ops.object.mode_set(mode="EDIT")
    ed = arm.data.edit_bones
    for b in list(ed):
        ed.remove(b)
    specs = [
        ("Root", (0, 0, 1.0), (0, 0, 1.5), None),
        ("ArmR", (0.2, 0, 1.5), (0.6, 0, 1.5), "Root"),
        ("HandR", (0.6, 0, 1.5), (0.8, 0, 1.5), "ArmR"),
    ]
    made = {}
    for bname, head, tail, parent in specs:
        eb = ed.new(bname)
        eb.head, eb.tail = head, tail
        eb.roll = 0.0
        made[bname] = eb
        if parent is not None:
            eb.parent = made[parent]
    bpy.ops.object.mode_set(mode="POSE")
    for pb in arm.pose.bones:
        pb.rotation_mode = "QUATERNION"
    bpy.ops.object.mode_set(mode="OBJECT")
    return arm


def make_prop_mesh(name, parent=None, parent_bone=None):
    bpy.ops.mesh.primitive_cube_add(size=0.2)
    obj = bpy.context.view_layer.objects.active
    obj.name = name
    obj.rotation_mode = "QUATERNION"
    if parent is not None and parent_bone is not None:
        obj.parent = parent
        obj.parent_type = "BONE"
        obj.parent_bone = parent_bone
    elif parent is not None:
        obj.parent = parent
    return obj


def build_synced_hero_prop_scene():
    """Create hero armature + weapon + shield + helper sharing one Action."""
    clean_scene()
    scene = bpy.context.scene
    scene.render.fps = 30
    scene.render.fps_base = 1.0
    scene.frame_start = 1
    scene.frame_end = 12

    arm = make_hero_armature("AndrasRig")
    weapon = make_prop_mesh("SwordProp", parent=arm, parent_bone="HandR")
    shield = make_prop_mesh("ShieldProp")
    helper = bpy.data.objects.new("HelperEmpty", None)
    helper.rotation_mode = "QUATERNION"
    scene.collection.objects.link(helper)

    action = bpy.data.actions.new("SlashCombo")
    bindings = {
        "hero": arm,
        "weapon": weapon,
        "shield": shield,
        "helper": helper,
    }
    slotted.bind_slotted_action(
        action,
        bindings,
        allowed_slots=slotted.CANONICAL_SLOTS,
    )

    cues = {"start": 1, "hit": 6, "end": 12}
    poses = [
        (1, 0.0, 0.0, (0.0, 0.0, 0.0), (0.0, 0.3, 1.2), (0.1, 0.0, 0.0)),
        (6, 1.15, -0.85, (0.05, -0.25, 0.15), (0.0, 0.3, 1.2), (0.4, 0.2, -0.1)),
        (12, 0.2, -0.1, (0.0, -0.05, 0.0), (0.0, 0.3, 1.2), (0.15, 0.05, 0.0)),
    ]
    for f, arm_ang, wpn_ang, wpn_loc, shd_loc, hlp_loc in poses:
        arm.pose.bones["Root"].location = (0.0, 0.02 * (f - 1), 0.0)
        arm.pose.bones["ArmR"].rotation_quaternion = Matrix.Rotation(
            arm_ang, 4, "X"
        ).to_quaternion()
        arm.pose.bones["HandR"].rotation_quaternion = Matrix.Rotation(
            arm_ang * 0.5, 4, "Z"
        ).to_quaternion()
        for pb in arm.pose.bones:
            pb.keyframe_insert("location", frame=f)
            pb.keyframe_insert("rotation_quaternion", frame=f)

        weapon.location = wpn_loc
        weapon.rotation_quaternion = Matrix.Rotation(wpn_ang, 4, "Y").to_quaternion()
        weapon.keyframe_insert("location", frame=f)
        weapon.keyframe_insert("rotation_quaternion", frame=f)

        # Constant shield across the action: must still be preserved on GLB export!
        shield.location = shd_loc
        shield.keyframe_insert("location", frame=f)

        helper.location = hlp_loc
        helper.rotation_quaternion = Matrix.Rotation(arm_ang * 0.25, 4, "Z").to_quaternion()
        helper.keyframe_insert("location", frame=f)
        helper.keyframe_insert("rotation_quaternion", frame=f)

    bpy.context.view_layer.update()
    return action, bindings, cues


def test_reject_suffixed_and_mismatched_slots():
    clean_scene()
    arm = make_hero_armature("Hero")
    weapon = make_prop_mesh("Weapon")
    action = bpy.data.actions.new("BadSlots")

    for bad_name in ("weapon_1", "weapon_2", "hero.001", "shield.002", "helper_01"):
        expect_error(
            f"bind suffixed slot {bad_name}",
            lambda n=bad_name: slotted.bind_slot(weapon, action, n),
        )

    # Manually inject a suffixed slot in Blender directly to verify validator rejects it.
    s_hero = action.slots.new(id_type="OBJECT", name="hero")
    s_bad = action.slots.new(id_type="OBJECT", name="weapon_1")
    layer = action.layers.new("Layer")
    strip = layer.strips.new(type="KEYFRAME")
    strip.channelbags.new(s_hero)
    strip.channelbags.new(s_bad)
    arm.animation_data_create()
    arm.animation_data.action = action
    arm.animation_data.action_slot = s_hero
    weapon.animation_data_create()
    weapon.animation_data.action = action
    weapon.animation_data.action_slot = s_bad
    for f in (1, 6, 12):
        arm.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
        weapon.keyframe_insert("location", frame=f)

    expect_error(
        "validate action with suffixed slot weapon_1",
        lambda: slotted.validate_slotted_action(action, expected_slots=("hero", "weapon")),
    )

    # Now create hero + hero.001 on the same Action.
    clean_scene()
    arm = make_hero_armature("Hero")
    weapon = make_prop_mesh("Weapon")
    act2 = bpy.data.actions.new("DupSlots")
    s1 = act2.slots.new(id_type="OBJECT", name="hero")
    s2 = act2.slots.new(id_type="OBJECT", name="hero")  # Blender auto-suffixes hero.001
    check(s2.name_display.endswith(".001"), f"Blender auto-suffixed duplicate to {s2.name_display}")
    layer2 = act2.layers.new("Layer")
    strip2 = layer2.strips.new(type="KEYFRAME")
    strip2.channelbags.new(s1)
    strip2.channelbags.new(s2)
    arm.animation_data_create()
    arm.animation_data.action = act2
    arm.animation_data.action_slot = s1
    weapon.animation_data_create()
    weapon.animation_data.action = act2
    weapon.animation_data.action_slot = s2
    for f in (1, 6, 12):
        arm.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
        weapon.keyframe_insert("location", frame=f)

    expect_error(
        "validate action with Blender auto-suffixed hero.001",
        lambda: slotted.validate_slotted_action(act2),
    )

    # Missing expected slot & mismatched slot names.
    clean_scene()
    arm = make_hero_armature("Hero")
    act3 = bpy.data.actions.new("MissingSlot")
    slotted.bind_slot(arm, act3, "hero")
    for f in (1, 6, 12):
        arm.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
    expect_error(
        "missing expected slot weapon",
        lambda: slotted.validate_slotted_action(act3, expected_slots=("hero", "weapon")),
    )

    # Separate actions (not unified in one Action).
    clean_scene()
    arm = make_hero_armature("Hero")
    weapon = make_prop_mesh("Weapon")
    a_hero = bpy.data.actions.new("HeroOnly")
    a_wpn = bpy.data.actions.new("WeaponOnly")
    slotted.bind_slot(arm, a_hero, "hero")
    slotted.bind_slot(weapon, a_wpn, "weapon")
    for f in (1, 6, 12):
        arm.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
        weapon.keyframe_insert("location", frame=f)
    expect_error(
        "separate actions for hero and weapon",
        lambda: slotted.validate_slotted_action(
            a_hero,
            expected_slots=("hero", "weapon"),
            bindings={"hero": arm, "weapon": weapon},
        ),
    )


def test_reject_keyframe_timing_desync():
    # Case 1: Frame range drift (weapon shifted +1 frame).
    clean_scene()
    arm = make_hero_armature("Hero")
    weapon = make_prop_mesh("Weapon")
    action = bpy.data.actions.new("RangeDrift")
    slotted.bind_slotted_action(action, {"hero": arm, "weapon": weapon})
    for f in (1, 6, 12):
        arm.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
    for f in (2, 7, 13):
        weapon.keyframe_insert("location", frame=f)
    expect_error(
        "frame range drift across slots",
        lambda: slotted.validate_slotted_action(
            action,
            expected_slots=("hero", "weapon"),
            cue_frames={"start": 1, "hit": 6, "end": 12},
        ),
    )

    # Case 2: Endpoints match (1..12) but combat hit cue drifted from frame 6 to frame 7!
    clean_scene()
    arm = make_hero_armature("Hero")
    weapon = make_prop_mesh("Weapon")
    action2 = bpy.data.actions.new("HitDrift")
    slotted.bind_slotted_action(action2, {"hero": arm, "weapon": weapon})
    for f in (1, 6, 12):
        arm.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
    for f in (1, 7, 12):
        weapon.keyframe_insert("location", frame=f)
    expect_error(
        "hit cue drift (frame 6 vs 7)",
        lambda: slotted.validate_slotted_action(
            action2,
            expected_slots=("hero", "weapon"),
            cue_frames={"start": 1, "hit": 6, "end": 12},
        ),
    )
    expect_error(
        "hit cue drift detected even without explicit cues",
        lambda: slotted.validate_slotted_action(
            action2,
            expected_slots=("hero", "weapon"),
        ),
    )


def test_glb_roundtrip_zero_desync():
    action, bindings, cues = build_synced_hero_prop_scene()
    summary = slotted.validate_slotted_action(
        action,
        expected_slots=("hero", "weapon", "shield", "helper"),
        bindings=bindings,
        allowed_slots=slotted.CANONICAL_SLOTS,
        cue_frames=cues,
    )
    check(summary["frame_range"] == (1.0, 12.0), f"validated frame range {summary['frame_range']}")

    sample_frames = (1, 6, 12)
    orig_transforms = slotted.sample_transforms(
        bindings,
        sample_frames,
        bone_names=("Root", "ArmR", "HandR"),
    )

    tmp = tempfile.mkdtemp(prefix="mf_slotted_")
    try:
        glb_path = os.path.join(tmp, "hero_slash.glb")
        slotted.export_slotted_glb(
            glb_path,
            action,
            bindings,
            expected_slots=("hero", "weapon", "shield", "helper"),
            allowed_slots=slotted.CANONICAL_SLOTS,
            cue_frames=cues,
        )
        check(os.path.isfile(glb_path) and os.path.getsize(glb_path) > 500, "GLB exported")

        # Pure-Python GLB inspection also confirms 0 desync.
        glb_info = slot_contract.validate_glb_slot_timings(
            glb_path,
            expected_slots=("hero", "weapon", "shield", "helper"),
            cue_frames=cues,
        )
        check(
            set(glb_info["slots"]) == {"hero", "weapon", "shield", "helper"},
            f"GLB metadata preserved canonical slots: {glb_info['slots']}",
        )

        # Clear scene and set non-matching 24 fps to ensure import restores 30 fps & 0 desync.
        clean_scene()
        bpy.context.scene.render.fps = 24
        bpy.context.scene.render.fps_base = 1.0
        imported_action, imported_bindings = slotted.import_slotted_glb(
            glb_path,
            expected_slots=("hero", "weapon", "shield", "helper"),
            allowed_slots=slotted.CANONICAL_SLOTS,
            cue_frames=cues,
        )
        check(
            set(imported_bindings) == {"hero", "weapon", "shield", "helper"},
            f"re-imported canonical bindings: {sorted(imported_bindings)}",
        )
        check(
            all(obj.animation_data.action is imported_action for obj in imported_bindings.values()),
            "all re-imported objects share one Action",
        )
        check(
            [s.name_display for s in imported_action.slots]
            and all(not slot_contract.is_suffixed_slot_name(s.name_display) for s in imported_action.slots),
            f"re-imported slot names clean: {[s.name_display for s in imported_action.slots]}",
        )

        # Verify 0 frame desync across sampled start/hit/end timestamps.
        for slot_name in ("hero", "weapon", "shield", "helper"):
            slot = slotted.find_slot(imported_action, slot_name)
            frames = slotted.slot_keyframe_frames(imported_action, slot)
            check(abs(frames[0] - 1.0) <= 1e-4 and abs(frames[-1] - 12.0) <= 1e-4,
                  f"slot {slot_name} exact start/end parity ({frames[0]}..{frames[-1]})")
            for cue_name, cue_f in cues.items():
                check(
                    any(abs(kf - cue_f) <= 1e-4 for kf in frames),
                    f"slot {slot_name} has exact key at cue '{cue_name}' (frame {cue_f})",
                )

        actual_transforms = slotted.sample_transforms(
            imported_bindings,
            sample_frames,
            bone_names=("Root", "ArmR", "HandR"),
        )
        slotted.verify_transforms_match(orig_transforms, actual_transforms, tol=1e-4)
        print("PASS: GLB round-trip transforms match within 1e-4 at start/hit/end")

        # Validate & export operators pass on the imported scene too.
        bpy.context.view_layer.objects.active = imported_bindings["hero"]
        res = bpy.ops.motionforge.validate_slotted(
            expected_slots="hero,weapon,shield,helper",
            cue_frames="1,6,12",
            canonical_only=True,
        )
        check("FINISHED" in res, "motionforge.validate_slotted operator finishes on valid action")
        op_glb = os.path.join(tmp, "op_export.glb")
        res_exp = bpy.ops.motionforge.export_slotted_glb(
            filepath=op_glb,
            expected_slots="hero,weapon,shield,helper",
            cue_frames="1,6,12",
            canonical_only=True,
        )
        check("FINISHED" in res_exp and os.path.isfile(op_glb), "motionforge.export_slotted_glb exports GLB")
        op_info = slot_contract.validate_glb_slot_timings(
            op_glb,
            expected_slots=("hero", "weapon", "shield", "helper"),
            cue_frames=cues,
        )
        op_lo, op_hi = op_info["frame_range"]
        check(
            abs(op_lo - 1.0) <= 1e-4 and abs(op_hi - 12.0) <= 1e-4,
            f"operator-exported GLB keeps 1..12 parity ({op_lo:.6f}..{op_hi:.6f})",
        )

        # Positive CLI check on exported GLB.
        ok_proc = subprocess.run(
            [
                bpy.app.binary_path,
                "--background",
                "--factory-startup",
                "--python-exit-code",
                "1",
                "--python",
                os.path.join(REPO, "blender", "motionforge", "slotted.py"),
                "--",
                "validate",
                "--glb",
                op_glb,
                "--expect",
                "hero,weapon,shield,helper",
                "--cues",
                "1,6,12",
                "--canonical-only",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=120,
            check=False,
        )
        check(ok_proc.returncode == 0, f"slotted.py CLI passes on valid GLB (rc={ok_proc.returncode})")

        # Verify Blender CLI script exits non-zero when validating a broken GLB/action.
        bad_blend = os.path.join(tmp, "bad.blend")
        clean_scene()
        arm2 = make_hero_armature("Hero")
        wpn2 = make_prop_mesh("Weapon")
        bad_act = bpy.data.actions.new("BadCli")
        s_h = bad_act.slots.new(id_type="OBJECT", name="hero")
        s_w = bad_act.slots.new(id_type="OBJECT", name="weapon_1")
        l2 = bad_act.layers.new("Layer")
        st2 = l2.strips.new(type="KEYFRAME")
        st2.channelbags.new(s_h)
        st2.channelbags.new(s_w)
        arm2.animation_data_create()
        arm2.animation_data.action = bad_act
        arm2.animation_data.action_slot = s_h
        wpn2.animation_data_create()
        wpn2.animation_data.action = bad_act
        wpn2.animation_data.action_slot = s_w
        for f in (1, 6, 12):
            arm2.pose.bones["Root"].keyframe_insert("rotation_quaternion", frame=f)
            wpn2.keyframe_insert("location", frame=f)
        bpy.ops.wm.save_as_mainfile(filepath=bad_blend)

        proc = subprocess.run(
            [
                bpy.app.binary_path,
                "--background",
                "--factory-startup",
                "--python-exit-code",
                "1",
                bad_blend,
                "--python",
                os.path.join(REPO, "blender", "motionforge", "slotted.py"),
                "--",
                "validate",
                "--action",
                "BadCli",
                "--expect",
                "hero,weapon",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=120,
            check=False,
        )
        check(proc.returncode != 0, f"slotted.py CLI exits non-zero on invalid slot (rc={proc.returncode})")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def test_clip_io_compat_with_multislot_and_single_slot():
    """Ensure clip_io.export_action/import_clip works when action has weapon slot first."""
    action, bindings, _ = build_synced_hero_prop_scene()
    arm = bindings["hero"]
    # Simulate non-matching active action on arm before calling export_action(arm, action, ...)
    other = bpy.data.actions.new("Unrelated")
    arm.animation_data.action = other
    clip = clip_io.export_action(arm, action, 1, 12)
    check(len(clip["frames"]) == 12, "clip_io.export_action exports 12 frames from slotted action")
    rot_at_6 = clip["frames"][5]["ArmR"]["quat"]
    expected_q = Matrix.Rotation(1.15, 4, "X").to_quaternion()
    diff = max(abs(rot_at_6[i] - expected_q[i]) for i in range(4))
    check(diff < 1e-4, f"clip_io.export_action resolved hero slot over weapon slot (diff={diff:.2e})")

    # Legacy single-slot round-trip remains untouched.
    imported = clip_io.import_clip(arm, clip, "SingleSlotRT", 1)
    reexported = clip_io.export_action(arm, imported, 1, 12)
    check(len(reexported["frames"]) == 12, "single-slot import_clip/export_action workflow intact")


def run_tests(manage_addon=True):
    if manage_addon:
        bpy.ops.preferences.addon_enable(module="motionforge")
    try:
        test_reject_suffixed_and_mismatched_slots()
        test_reject_keyframe_timing_desync()
        test_glb_roundtrip_zero_desync()
        test_clip_io_compat_with_multislot_and_single_slot()
        print("ALL SLOTTED ACTION TESTS PASSED")
    finally:
        if manage_addon:
            bpy.ops.preferences.addon_disable(module="motionforge")


if __name__ == "__main__":
    import traceback

    try:
        run_tests()
    except SystemExit:
        raise
    except BaseException:
        traceback.print_exc()
        raise SystemExit(1) from None
