# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender Slotted Actions contract & zero-desync hero+prop GLB round-trip.

Supports Blender 4.4+ / 5.x Slotted Actions (action.slots, layers, strips,
channelbags) with canonical slot naming (hero, weapon, shield, helper),
strict validation, and exact-timing GLB export/import preservation.
"""

import argparse
import contextlib
import json
import os
import sys

import bpy
from bpy_extras import anim_utils

if __package__:
    from . import clip_io, slot_contract
else:
    _BLENDER_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    if _BLENDER_DIR not in sys.path:
        sys.path.insert(0, _BLENDER_DIR)
    from motionforge import clip_io, slot_contract

CANONICAL_SLOTS = slot_contract.CANONICAL_SLOTS


class SlottedActionError(clip_io.MotionforgeError, slot_contract.SlotContractError):
    """Raised when a Blender slotted action violates the slot/timing contract."""


def iter_fcurves(action, slot=None):
    """Iterate FCurves for an action (or a single slot), Blender 5.x or legacy."""
    return clip_io._iter_fcurves(action, slot=slot)


def find_slot(action, slot_name):
    """Return the ActionSlot whose `name_display` equals `slot_name`, or None."""
    for slot in getattr(action, "slots", None) or ():
        if slot.name_display == slot_name:
            return slot
    return None


def ensure_slot(action, slot_name, id_type="OBJECT", allowed_slots=None):
    """Find or create a canonical named slot on `action`, rejecting suffixes."""
    if action is None:
        raise SlottedActionError("no action provided")
    slots = getattr(action, "slots", None)
    if slots is None:
        raise SlottedActionError(f"action '{action.name}' does not support Blender slots")
    try:
        slot_contract.validate_slot_name(slot_name, allowed_slots=allowed_slots)
    except slot_contract.SlotContractError as exc:
        raise SlottedActionError(str(exc)) from exc

    for existing in slots:
        ename = existing.name_display
        if slot_contract.is_suffixed_slot_name(ename):
            raise SlottedActionError(
                f"action '{action.name}' already contains forbidden suffixed slot '{ename}'"
            )
        if ename != slot_name and ename.casefold() == slot_name.casefold():
            raise SlottedActionError(
                f"action '{action.name}' has case-colliding slot '{ename}' vs '{slot_name}'"
            )

    existing = find_slot(action, slot_name)
    if existing is not None:
        if getattr(existing, "target_id_type", id_type) != id_type:
            raise SlottedActionError(
                f"slot '{slot_name}' has target_id_type '{existing.target_id_type}', "
                f"expected '{id_type}'"
            )
        return existing

    slot = slots.new(id_type=id_type, name=slot_name)
    if slot.name_display != slot_name:
        slots.remove(slot)
        raise SlottedActionError(
            f"Blender auto-suffixed slot to '{slot.name_display}' instead of '{slot_name}'"
        )
    return slot


def ensure_channelbag(action, slot):
    """Ensure `action` has a layer/keyframe strip/channelbag for `slot`."""
    if not hasattr(action, "layers"):
        return None
    bag = anim_utils.action_get_channelbag_for_slot(action, slot)
    if bag is not None:
        return bag
    return anim_utils.action_ensure_channelbag_for_slot(action, slot)


def resolve_object_slot(obj, action, preferred_name=None):
    """Resolve the slot in `action` belonging to `obj` (never blind slots[0] on multi-slot)."""
    if action is None:
        return None
    slots = list(getattr(action, "slots", None) or ())
    if not slots:
        return None
    if preferred_name is not None:
        return find_slot(action, preferred_name)
    if obj.type == "ARMATURE":
        return clip_io._resolve_armature_slot(obj, action)
    ad = obj.animation_data
    if ad is not None and ad.action == action and getattr(ad, "action_slot", None) in slots:
        return ad.action_slot
    tag = obj.get("motionforge_slot") if hasattr(obj, "get") else None
    for cand in (tag, obj.name, obj.data.name if obj.data else None):
        if cand:
            found = find_slot(action, cand)
            if found is not None:
                return found
    return slots[0] if len(slots) == 1 else None


def bind_slot(obj, action, slot_name, allowed_slots=None, create_bag=True):
    """Bind `obj` to canonical slot `slot_name` inside `action`."""
    if obj is None:
        raise SlottedActionError(f"cannot bind None object to slot '{slot_name}'")
    if action is None:
        raise SlottedActionError("cannot bind to None action")
    slot = ensure_slot(action, slot_name, id_type="OBJECT", allowed_slots=allowed_slots)
    ad = obj.animation_data or obj.animation_data_create()
    ad.action = action
    if hasattr(ad, "action_slot"):
        ad.action_slot = slot
    if create_bag:
        ensure_channelbag(action, slot)
    return slot


def bind_slotted_action(action, bindings, allowed_slots=None, create_bag=True):
    """Bind all `{slot_name: obj}` entries to ONE shared `action` with canonical slots."""
    items = list(bindings.items()) if isinstance(bindings, dict) else list(bindings)
    if not items:
        raise SlottedActionError("bindings cannot be empty")
    try:
        slot_contract.validate_slot_names([name for name, _ in items], allowed_slots=allowed_slots)
    except slot_contract.SlotContractError as exc:
        raise SlottedActionError(str(exc)) from exc
    seen_objs = {}
    for name, obj in items:
        if obj is None:
            raise SlottedActionError(f"object for slot '{name}' is None")
        obj_id = id(obj)
        if obj_id in seen_objs:
            raise SlottedActionError(
                f"object '{obj.name}' bound to multiple slots: '{seen_objs[obj_id]}' and '{name}'"
            )
        seen_objs[obj_id] = name
    out = {}
    for name, obj in items:
        out[name] = bind_slot(obj, action, name, allowed_slots=allowed_slots, create_bag=create_bag)
    return out


def slot_keyframe_frames(action, slot):
    """Sorted unique keyframe frame numbers inside `slot`'s channelbags."""
    frames = sorted({
        float(kp.co.x)
        for fc in iter_fcurves(action, slot=slot)
        for kp in fc.keyframe_points
    })
    return frames


def validate_slotted_action(
    action,
    expected_slots=None,
    bindings=None,
    allowed_slots=None,
    cue_frames=None,
    require_shared_key_times=True,
    frame_tol=1e-4,
    allow_extra=False,
):
    """Strictly validate slot names, shared Action bindings, and zero-desync timings."""
    if action is None:
        raise SlottedActionError("no action to validate")
    slots = list(getattr(action, "slots", None) or ())
    if not slots:
        raise SlottedActionError(f"action '{action.name}' has no slots")

    slot_names = [s.name_display for s in slots]
    try:
        slot_contract.validate_slot_names(
            slot_names,
            expected_slots=expected_slots,
            allowed_slots=allowed_slots,
            allow_extra=allow_extra,
        )
    except slot_contract.SlotContractError as exc:
        raise SlottedActionError(f"action '{action.name}': {exc}") from exc

    if bindings is not None:
        bmap = dict(bindings)
        if expected_slots is not None:
            missing_b = [s for s in expected_slots if s not in bmap]
            if missing_b:
                raise SlottedActionError(
                    f"bindings missing expected slot(s): {', '.join(missing_b)}"
                )
        for slot_name, obj in bmap.items():
            if obj is None:
                raise SlottedActionError(f"binding for slot '{slot_name}' is None")
            ad = getattr(obj, "animation_data", None)
            if ad is None or ad.action != action:
                got = ad.action.name if (ad and ad.action) else None
                raise SlottedActionError(
                    f"object '{obj.name}' (slot '{slot_name}') does not share action "
                    f"'{action.name}' (has {got!r})"
                )
            bound_slot = getattr(ad, "action_slot", None)
            if bound_slot is None or bound_slot not in slots:
                raise SlottedActionError(
                    f"object '{obj.name}' has invalid/unbound action_slot on '{action.name}'"
                )
            if bound_slot.name_display != slot_name:
                raise SlottedActionError(
                    f"object '{obj.name}' bound to slot '{bound_slot.name_display}', "
                    f"expected '{slot_name}'"
                )

    slot_frames = {s.name_display: slot_keyframe_frames(action, s) for s in slots}
    try:
        timing = slot_contract.validate_slot_timings(
            slot_frames,
            cue_frames=cue_frames,
            require_shared_key_times=require_shared_key_times,
            frame_tol=frame_tol,
        )
    except slot_contract.SlotContractError as exc:
        raise SlottedActionError(f"action '{action.name}': {exc}") from exc

    return {
        "action": action.name,
        "slots": slot_names,
        **timing,
    }


def _collect_export_objects(roots):
    keep = []
    seen = set()

    def add(obj):
        if obj is None or obj in seen:
            return
        seen.add(obj)
        keep.append(obj)
        for child in obj.children:
            add(child)

    for obj in roots:
        add(obj)
    return keep


def _save_props(objects, scene):
    obj_prev = {
        obj: ("motionforge_slot" in obj, obj.get("motionforge_slot"))
        for obj in objects
    }
    scene_prev = (
        "motionforge_slotted" in scene,
        scene.get("motionforge_slotted"),
    )
    return obj_prev, scene_prev


def _restore_props(obj_prev, scene_prev, scene):
    for obj, (had, val) in obj_prev.items():
        if had:
            obj["motionforge_slot"] = val
        elif "motionforge_slot" in obj:
            del obj["motionforge_slot"]
    had_s, val_s = scene_prev
    if had_s:
        scene["motionforge_slotted"] = val_s
    elif "motionforge_slotted" in scene:
        del scene["motionforge_slotted"]


def export_slotted_glb(
    filepath,
    action,
    bindings,
    expected_slots=None,
    allowed_slots=None,
    cue_frames=None,
    require_shared_key_times=True,
    frame_tol=1e-4,
    bake_samples=True,
):
    """Validate and export a slotted hero+prop Action to GLB with zero timing drift."""
    bmap = dict(bindings)
    wanted_slots = list(expected_slots) if expected_slots is not None else list(bmap.keys())
    summary = validate_slotted_action(
        action,
        expected_slots=wanted_slots,
        bindings=bmap,
        allowed_slots=allowed_slots,
        cue_frames=cue_frames,
        require_shared_key_times=require_shared_key_times,
        frame_tol=frame_tol,
    )
    scene = bpy.context.scene
    prev_mode = bpy.context.mode
    if prev_mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    prev_active = bpy.context.view_layer.objects.active
    prev_selected = list(bpy.context.selected_objects)
    prev_start, prev_end = scene.frame_start, scene.frame_end
    obj_prev, scene_prev = _save_props(bmap.values(), scene)

    try:
        for slot_name, obj in bmap.items():
            obj["motionforge_slot"] = slot_name
        meta = {
            "action": action.name,
            "fps": scene.render.fps,
            "fps_base": scene.render.fps_base,
            "frame_start": summary["frame_range"][0],
            "frame_end": summary["frame_range"][1],
            "slots": {obj.name: slot_name for slot_name, obj in bmap.items()},
            "require_shared_key_times": require_shared_key_times,
        }
        if cue_frames is not None:
            meta["cues"] = dict(slot_contract.normalize_cue_frames(cue_frames))
        scene["motionforge_slotted"] = json.dumps(meta, sort_keys=True)

        export_objs = _collect_export_objects(bmap.values())
        bpy.ops.object.select_all(action="DESELECT")
        for obj in export_objs:
            obj.select_set(True)
        bpy.context.view_layer.objects.active = next(iter(bmap.values()))

        export_kw = {
            "filepath": filepath,
            "export_format": "GLB",
            "use_selection": True,
            "export_extras": True,
            "export_animations": True,
            "export_animation_mode": "ACTIVE_ACTIONS",
            "export_nla_strips_merged_animation_name": action.name,
            "export_anim_slide_to_zero": False,
            "export_optimize_animation_size": False,
            "export_optimize_animation_keep_anim_armature": True,
            "export_optimize_animation_keep_anim_object": True,
            "export_force_sampling": bake_samples,
        }
        known = set(bpy.ops.export_scene.gltf.get_rna_type().properties.keys())
        if "export_merge_animation" in known:
            export_kw["export_merge_animation"] = "ACTION"
        bpy.ops.export_scene.gltf(**export_kw)
    finally:
        _restore_props(obj_prev, scene_prev, scene)
        scene.frame_start, scene.frame_end = prev_start, prev_end
        bpy.ops.object.select_all(action="DESELECT")
        for obj in prev_selected:
            if obj.name in bpy.data.objects:
                obj.select_set(True)
        if prev_active is not None and prev_active.name in bpy.data.objects:
            bpy.context.view_layer.objects.active = prev_active
        if prev_mode != "OBJECT" and prev_active is not None:
            with contextlib.suppress(RuntimeError):
                bpy.ops.object.mode_set(mode=prev_mode)
    return summary


def _rename_slots_canonically(action, slot_assignments):
    """Rename slots on `action` to canonical names without Blender .001 collisions."""
    # First assign temporary unique names to avoid collisions if swapping names.
    for idx, (slot, _) in enumerate(slot_assignments):
        slot.name_display = f"__mf_tmp_slot_{idx}"
    for slot, canonical_name in slot_assignments:
        slot.name_display = canonical_name
        if slot.name_display != canonical_name:
            raise SlottedActionError(
                f"failed to restore canonical slot name '{canonical_name}' "
                f"(got '{slot.name_display}')"
            )


def import_slotted_glb(
    filepath,
    expected_slots=None,
    allowed_slots=None,
    cue_frames=None,
    require_shared_key_times=None,
    frame_tol=1e-4,
    action_name=None,
):
    """Import a slotted hero+prop GLB, restore canonical slot names, and validate sync."""
    _, _, meta, _ = slot_contract.read_glb_metadata(filepath)
    scene = bpy.context.scene
    if "fps" in meta:
        scene.render.fps = int(meta["fps"])
    if "fps_base" in meta:
        scene.render.fps_base = float(meta["fps_base"])

    before_objs = set(bpy.data.objects)
    before_acts = set(bpy.data.actions)
    bpy.ops.import_scene.gltf(filepath=filepath)
    new_objs = [o for o in bpy.data.objects if o not in before_objs]
    new_acts = [a for a in bpy.data.actions if a not in before_acts]

    wanted_action = action_name or meta.get("action")
    action = None
    if wanted_action:
        action = next((a for a in new_acts if a.name == wanted_action), None)
    if action is None:
        action = next((a for a in new_acts if len(getattr(a, "slots", ())) > 0), None)
    if action is None and new_acts:
        action = new_acts[0]
    if action is None:
        raise SlottedActionError(f"no action imported from '{filepath}'")

    meta_slots = meta.get("slots", {}) if isinstance(meta.get("slots"), dict) else {}
    bindings = {}
    renames = []
    for obj in new_objs:
        ad = getattr(obj, "animation_data", None)
        if ad is None or ad.action != action:
            continue
        cur_slot = getattr(ad, "action_slot", None)
        if cur_slot is None:
            continue
        canonical = obj.get("motionforge_slot") if "motionforge_slot" in obj else meta_slots.get(obj.name)
        if not canonical:
            canonical = cur_slot.name_display
        slot_contract.validate_slot_name(canonical, allowed_slots=allowed_slots)
        if canonical in bindings:
            raise SlottedActionError(f"duplicate imported canonical slot '{canonical}'")
        bindings[canonical] = obj
        if cur_slot.name_display != canonical:
            renames.append((cur_slot, canonical))

    if renames:
        _rename_slots_canonically(action, renames)
        for canonical, obj in bindings.items():
            obj.animation_data.action = action
            obj.animation_data.action_slot = find_slot(action, canonical)

    req_shared = (
        bool(meta.get("require_shared_key_times", True))
        if require_shared_key_times is None
        else require_shared_key_times
    )
    validate_slotted_action(
        action,
        expected_slots=expected_slots,
        bindings=bindings if bindings else None,
        allowed_slots=allowed_slots,
        cue_frames=cue_frames,
        require_shared_key_times=req_shared,
        frame_tol=frame_tol,
    )
    return action, bindings


def _mat_rows(mat):
    return tuple(tuple(float(mat[r][c]) for c in range(4)) for r in range(4))


def sample_transforms(bindings, frames, bone_names=None):
    """Sample world/local transforms for bound objects (and armature pose bones) at `frames`."""
    scene = bpy.context.scene
    prev_frame = scene.frame_current
    out = {}
    try:
        for f in frames:
            scene.frame_set(int(f))
            bpy.context.view_layer.update()
            per_slot = {}
            for slot_name, obj in bindings.items():
                entry = {
                    "loc": tuple(float(v) for v in obj.location),
                    "quat": tuple(float(v) for v in obj.matrix_local.to_quaternion()),
                    "world": _mat_rows(obj.matrix_world),
                }
                if obj.type == "ARMATURE":
                    names = list(bone_names) if bone_names is not None else [b.name for b in obj.pose.bones]
                    entry["bones"] = {
                        b: _mat_rows(obj.pose.bones[b].matrix_basis)
                        for b in names
                        if b in obj.pose.bones
                    }
                per_slot[slot_name] = entry
            out[int(f)] = per_slot
    finally:
        scene.frame_set(prev_frame)
        bpy.context.view_layer.update()
    return out


def verify_transforms_match(expected, actual, tol=1e-4):
    """Assert every sampled frame, slot, and pose bone matches within `tol`."""
    for f, exp_slots in expected.items():
        if f not in actual:
            raise SlottedActionError(f"missing sampled frame {f}")
        act_slots = actual[f]
        for slot_name, exp in exp_slots.items():
            if slot_name not in act_slots:
                raise SlottedActionError(f"frame {f} missing slot '{slot_name}'")
            act = act_slots[slot_name]
            for r in range(4):
                for c in range(4):
                    diff = abs(exp["world"][r][c] - act["world"][r][c])
                    if diff > tol:
                        raise SlottedActionError(
                            f"world transform desync at frame {f} slot '{slot_name}' "
                            f"(diff={diff:.6g} > tol={tol})"
                        )
            for bone_name, exp_m in exp.get("bones", {}).items():
                act_m = act.get("bones", {}).get(bone_name)
                if act_m is None:
                    raise SlottedActionError(
                        f"frame {f} slot '{slot_name}' missing bone '{bone_name}'"
                    )
                for r in range(4):
                    for c in range(4):
                        diff = abs(exp_m[r][c] - act_m[r][c])
                        if diff > tol:
                            raise SlottedActionError(
                                f"bone '{bone_name}' transform desync at frame {f} "
                                f"(diff={diff:.6g} > tol={tol})"
                            )
    return True


def collect_scene_action_bindings(action, scene=None):
    """Collect `{slot_name: obj}` for all scene objects bound to `action`."""
    scn = scene or bpy.context.scene
    bindings = {}
    for obj in scn.objects:
        ad = getattr(obj, "animation_data", None)
        if ad is None or ad.action != action:
            continue
        slot = getattr(ad, "action_slot", None)
        if slot is None:
            continue
        name = slot.name_display
        if name in bindings:
            raise SlottedActionError(
                f"multiple scene objects bound to slot '{name}' on action '{action.name}': "
                f"'{bindings[name].name}' and '{obj.name}'"
            )
        bindings[name] = obj
    return bindings


def main(argv=None):
    if argv is None:
        argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser(description="Validate Blender Slotted Actions")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_val = sub.add_parser("validate", help="Validate slotted action in scene or GLB")
    p_val.add_argument("--action", default="", help="Action name in current .blend")
    p_val.add_argument("--glb", default="", help="Optional GLB file to import and validate")
    p_val.add_argument("--expect", default="", help="Comma-separated expected slots")
    p_val.add_argument("--cues", default="", help="Comma-separated cue frame numbers")
    p_val.add_argument("--canonical-only", action="store_true")
    p_val.add_argument("--allow-extra", action="store_true")
    args = parser.parse_args(argv)

    try:
        expected = [s.strip() for s in args.expect.split(",") if s.strip()] or None
        cues = [float(s.strip()) for s in args.cues.split(",") if s.strip()] or None
        allowed = CANONICAL_SLOTS if args.canonical_only else None
        if args.glb:
            action, _ = import_slotted_glb(
                args.glb,
                expected_slots=expected,
                allowed_slots=allowed,
                cue_frames=cues,
            )
            info = validate_slotted_action(
                action,
                expected_slots=expected,
                allowed_slots=allowed,
                cue_frames=cues,
                allow_extra=args.allow_extra,
            )
        else:
            act = bpy.data.actions.get(args.action) if args.action else None
            if act is None and not args.action and len(bpy.data.actions) == 1:
                act = bpy.data.actions[0]
            if act is None:
                raise SlottedActionError(f"action '{args.action}' not found")
            bindings = collect_scene_action_bindings(act)
            info = validate_slotted_action(
                act,
                expected_slots=expected,
                bindings=bindings if bindings else None,
                allowed_slots=allowed,
                cue_frames=cues,
                allow_extra=args.allow_extra,
            )
        lo, hi = info["frame_range"]
        print(
            f"SLOTTED ACTION OK: action={info['action']} slots={','.join(info['slots'])} "
            f"frames={lo:g}..{hi:g}"
        )
        return 0
    except (SlottedActionError, clip_io.MotionforgeError, slot_contract.SlotContractError, ValueError) as exc:
        print(f"SLOTTED ACTION ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    import traceback

    try:
        raise SystemExit(main())
    except SystemExit:
        raise
    except BaseException:
        traceback.print_exc()
        raise SystemExit(1) from None
