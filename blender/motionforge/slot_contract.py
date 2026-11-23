# SPDX-License-Identifier: GPL-3.0-or-later
"""Pure-Python Slotted Actions contract & GLB timing validator (no bpy).

Defines canonical slot names (hero, weapon, shield, helper), rejects
duplicate or auto-suffixed slot names (_1, _2, .001, .002, ...), and
enforces zero-desync frame range and cue timing parity across slots.
"""

import argparse
import json
import math
import re
import struct
import sys

CANONICAL_SLOTS = ("hero", "weapon", "shield", "helper")
DUPLICATE_SUFFIX_RE = re.compile(r"(?:_\d+|\.\d+)$")

_GLTF_MAGIC = 0x46546C67
_CHUNK_JSON = 0x4E4F534A
_CHUNK_BIN = 0x004E4942
_FLOAT = 5126


class SlotContractError(ValueError):
    """Raised when a slotted action or GLB violates the slot contract."""


def is_suffixed_slot_name(name):
    """Return True if `name` ends with a duplicate suffix like _1 or .001."""
    return isinstance(name, str) and bool(DUPLICATE_SUFFIX_RE.search(name))


def validate_slot_name(name, allowed_slots=None):
    """Validate one slot name; raise SlotContractError on bad/suffixed names."""
    if not isinstance(name, str) or not name or name != name.strip():
        raise SlotContractError(f"invalid empty or whitespace slot name: {name!r}")
    if is_suffixed_slot_name(name):
        raise SlotContractError(
            f"slot name '{name}' has a forbidden duplicate suffix (_N or .NNN)"
        )
    if allowed_slots is not None:
        allowed = tuple(allowed_slots)
        if name not in allowed:
            raise SlotContractError(
                f"slot name '{name}' is not in canonical slots {allowed}"
            )
    return name


def validate_slot_names(
    slot_names,
    expected_slots=None,
    allowed_slots=None,
    allow_extra=False,
):
    """Validate a collection of slot names for suffixes, duplicates, and expectation."""
    names = list(slot_names)
    if not names:
        raise SlotContractError("action has no slots")
    seen_exact = set()
    seen_folded = {}
    for name in names:
        validate_slot_name(name, allowed_slots=allowed_slots)
        if name in seen_exact:
            raise SlotContractError(f"duplicate slot name '{name}'")
        folded = name.casefold()
        if folded in seen_folded:
            raise SlotContractError(
                f"case-colliding duplicate slot names '{seen_folded[folded]}' and '{name}'"
            )
        seen_exact.add(name)
        seen_folded[folded] = name

    if expected_slots is not None:
        expected = [validate_slot_name(s, allowed_slots=allowed_slots) for s in expected_slots]
        expected_set = set(expected)
        missing = [s for s in expected if s not in seen_exact]
        if missing:
            raise SlotContractError(f"missing expected slot(s): {', '.join(missing)}")
        if not allow_extra:
            extra = [s for s in names if s not in expected_set]
            if extra:
                raise SlotContractError(
                    f"mismatched/unexpected slot(s): {', '.join(extra)} "
                    f"(expected {tuple(expected)})"
                )
    return names


def normalize_cue_frames(cue_frames):
    """Convert dict/iterable of cue timings into [(label, float_frame), ...]."""
    if cue_frames is None:
        return []
    items = cue_frames.items() if isinstance(cue_frames, dict) else (
        (f"frame_{v}", v) for v in cue_frames
    )
    out = []
    for label, val in items:
        try:
            f = float(val)
        except (TypeError, ValueError) as exc:
            raise SlotContractError(f"non-numeric cue frame for '{label}': {val!r}") from exc
        if not math.isfinite(f):
            raise SlotContractError(f"non-finite cue frame for '{label}': {val!r}")
        out.append((str(label), f))
    return out


def _dedup_sorted_frames(frames, slot_name, tol=1e-6):
    vals = []
    for raw in frames:
        try:
            f = float(raw)
        except (TypeError, ValueError) as exc:
            raise SlotContractError(f"slot '{slot_name}' has non-numeric frame {raw!r}") from exc
        if not math.isfinite(f):
            raise SlotContractError(f"slot '{slot_name}' has non-finite frame {raw!r}")
        vals.append(f)
    if not vals:
        raise SlotContractError(f"slot '{slot_name}' has no keyframes")
    vals.sort()
    dedup = [vals[0]]
    for f in vals[1:]:
        if f - dedup[-1] > tol:
            dedup.append(f)
    return dedup


def validate_slot_timings(
    slot_frames,
    cue_frames=None,
    require_shared_key_times=True,
    frame_tol=1e-4,
):
    """Verify frame-range parity and zero-desync keyframe timings across slots."""
    if not slot_frames:
        raise SlotContractError("no slot keyframes to validate")
    validate_slot_names(slot_frames.keys())
    cues = normalize_cue_frames(cue_frames)

    normalized = {
        name: _dedup_sorted_frames(frames, name)
        for name, frames in slot_frames.items()
    }
    names = list(normalized.keys())
    ref_name = names[0]
    ref_frames = normalized[ref_name]
    ref_start, ref_end = ref_frames[0], ref_frames[-1]

    for name in names[1:]:
        frames = normalized[name]
        start_f, end_f = frames[0], frames[-1]
        if abs(start_f - ref_start) > frame_tol or abs(end_f - ref_end) > frame_tol:
            raise SlotContractError(
                f"frame range desync: slot '{name}' spans {start_f:.4f}..{end_f:.4f} "
                f"while '{ref_name}' spans {ref_start:.4f}..{ref_end:.4f}"
            )

    for label, cue_f in cues:
        for name in names:
            frames = normalized[name]
            nearest = min(frames, key=lambda kf: abs(kf - cue_f))
            delta = abs(nearest - cue_f)
            if delta > frame_tol:
                raise SlotContractError(
                    f"timing desync at cue '{label}' (frame {cue_f}): "
                    f"slot '{name}' nearest key is {nearest:.4f} (drift {delta:.4f})"
                )

    if require_shared_key_times:
        for name in names[1:]:
            frames = normalized[name]
            if len(frames) != len(ref_frames):
                raise SlotContractError(
                    f"keyframe timing desync: slot '{name}' has {len(frames)} keys "
                    f"{frames} vs '{ref_name}' ({len(ref_frames)} keys {ref_frames})"
                )
            for fa, fb in zip(ref_frames, frames, strict=True):
                if abs(fa - fb) > frame_tol:
                    raise SlotContractError(
                        f"keyframe timing desync between '{ref_name}' ({fa:.4f}) "
                        f"and '{name}' ({fb:.4f})"
                    )

    return {
        "frame_range": (ref_start, ref_end),
        "slot_frames": normalized,
    }


def parse_glb_chunks(data):
    """Parse GLB bytes into (json_dict, bin_bytes) with bounds checks."""
    if len(data) < 20:
        raise SlotContractError("truncated GLB header")
    magic, version, total = struct.unpack_from("<III", data, 0)
    if magic != _GLTF_MAGIC or version != 2:
        raise SlotContractError("not a glTF 2.0 binary (GLB)")
    if total > len(data):
        raise SlotContractError("truncated GLB payload")
    pos = 12
    doc = None
    bin_bytes = b""
    while pos + 8 <= total:
        length, kind = struct.unpack_from("<II", data, pos)
        start = pos + 8
        end = start + length
        if end > total:
            raise SlotContractError("truncated GLB chunk")
        if kind == _CHUNK_JSON and doc is None:
            try:
                text = data[start:end].decode("utf-8").rstrip(" \0")
                doc = json.loads(text)
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise SlotContractError(f"invalid GLB JSON chunk: {exc}") from exc
        elif kind == _CHUNK_BIN and not bin_bytes:
            bin_bytes = data[start:end]
        pos = end
    if doc is None:
        raise SlotContractError("GLB has no JSON chunk")
    return doc, bin_bytes


def read_glb_metadata(path):
    """Read motionforge_slotted metadata + JSON tree from a GLB file."""
    with open(path, "rb") as fh:
        data = fh.read()
    doc, bin_bytes = parse_glb_chunks(data)
    meta = {}
    for scene in doc.get("scenes", ()):
        extras = scene.get("extras")
        if isinstance(extras, dict) and "motionforge_slotted" in extras:
            raw = extras["motionforge_slotted"]
            if isinstance(raw, str):
                meta = json.loads(raw)
            elif isinstance(raw, dict):
                meta = dict(raw)
            break
    node_slots = {}
    for idx, node in enumerate(doc.get("nodes", ())):
        extras = node.get("extras")
        if isinstance(extras, dict) and isinstance(extras.get("motionforge_slot"), str):
            node_slots[idx] = extras["motionforge_slot"]
    return doc, bin_bytes, meta, node_slots


def _read_scalar_floats(doc, bin_bytes, accessor_idx):
    accessors = doc.get("accessors", ())
    views = doc.get("bufferViews", ())
    if not 0 <= accessor_idx < len(accessors):
        raise SlotContractError(f"bad GLB accessor index {accessor_idx}")
    acc = accessors[accessor_idx]
    if acc.get("componentType") != _FLOAT or acc.get("type") != "SCALAR":
        raise SlotContractError("GLB animation input accessor must be SCALAR FLOAT")
    bv_idx = acc.get("bufferView")
    if bv_idx is None or not 0 <= bv_idx < len(views):
        raise SlotContractError("GLB accessor has invalid bufferView")
    bv = views[bv_idx]
    count = int(acc["count"])
    if count <= 0:
        return []
    stride = int(bv.get("byteStride") or 4)
    offset = int(bv.get("byteOffset", 0)) + int(acc.get("byteOffset", 0))
    if offset < 0 or stride < 4 or offset + (count - 1) * stride + 4 > len(bin_bytes):
        raise SlotContractError("GLB accessor reads past BIN chunk")
    return [struct.unpack_from("<f", bin_bytes, offset + i * stride)[0] for i in range(count)]


def _node_slot_resolver(doc, meta, node_slots):
    nodes = doc.get("nodes", ())
    parent = {}
    for i, nd in enumerate(nodes):
        for c in nd.get("children", ()):
            parent[c] = i
    meta_slots = meta.get("slots", {}) if isinstance(meta.get("slots"), dict) else {}

    def slot_for_node(node_idx):
        cur = node_idx
        visited = set()
        while cur is not None and cur not in visited:
            visited.add(cur)
            if cur in node_slots:
                return node_slots[cur]
            name = nodes[cur].get("name")
            if isinstance(name, str) and name in meta_slots:
                return meta_slots[name]
            cur = parent.get(cur)
        if 0 <= node_idx < len(nodes):
            return nodes[node_idx].get("name") or f"node_{node_idx}"
        return f"node_{node_idx}"

    return slot_for_node


def validate_glb_slot_timings(
    path,
    expected_slots=None,
    allowed_slots=None,
    cue_frames=None,
    require_shared_key_times=True,
    frame_tol=1e-4,
    fps=None,
    anim_name=None,
):
    """Validate slot names and cross-slot keyframe sync inside an exported GLB."""
    doc, bin_bytes, meta, node_slots = read_glb_metadata(path)
    anims = doc.get("animations", ())
    if not anims:
        raise SlotContractError(f"GLB '{path}' contains no animations")
    wanted_name = anim_name or meta.get("action")
    anim = None
    if wanted_name:
        anim = next((a for a in anims if a.get("name") == wanted_name), None)
    if anim is None:
        anim = anims[0]

    if fps is None:
        raw_fps = meta.get("fps", 30.0)
        raw_base = meta.get("fps_base", 1.0)
        effective_fps = float(raw_fps) / float(raw_base)
    else:
        effective_fps = float(fps)

    resolve = _node_slot_resolver(doc, meta, node_slots)
    samplers = anim.get("samplers", ())
    slot_times = {}
    for ch in anim.get("channels", ()):
        node_idx = ch.get("target", {}).get("node")
        sampler_idx = ch.get("sampler")
        if node_idx is None or sampler_idx is None:
            continue
        if not 0 <= sampler_idx < len(samplers):
            raise SlotContractError(f"bad sampler index {sampler_idx} in GLB")
        times = _read_scalar_floats(doc, bin_bytes, samplers[sampler_idx]["input"])
        slot_name = resolve(node_idx)
        slot_times.setdefault(slot_name, []).extend(t * effective_fps for t in times)

    if not slot_times:
        raise SlotContractError(f"GLB animation '{anim.get('name', '')}' has no targeted channels")

    validate_slot_names(
        list(slot_times.keys()),
        expected_slots=expected_slots,
        allowed_slots=allowed_slots,
    )
    timing = validate_slot_timings(
        slot_times,
        cue_frames=cue_frames,
        require_shared_key_times=require_shared_key_times,
        frame_tol=frame_tol,
    )
    return {
        "action": anim.get("name", ""),
        "fps": effective_fps,
        "slots": list(slot_times.keys()),
        **timing,
    }


def _parse_csv(text):
    return [item.strip() for item in text.split(",") if item.strip()]


def main(argv=None):
    parser = argparse.ArgumentParser(description="Validate slotted animation GLBs")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_val = sub.add_parser("validate", help="Validate slot names and timing parity in a GLB")
    p_val.add_argument("glb")
    p_val.add_argument("--expect", default="", help="Comma-separated expected slot names")
    p_val.add_argument("--cues", default="", help="Comma-separated cue frame numbers")
    p_val.add_argument("--canonical-only", action="store_true")
    p_val.add_argument("--fps", type=float, default=None)
    args = parser.parse_args(argv)

    try:
        if args.cmd == "validate":
            expected = _parse_csv(args.expect) if args.expect else None
            cues = [float(x) for x in _parse_csv(args.cues)] if args.cues else None
            allowed = CANONICAL_SLOTS if args.canonical_only else None
            info = validate_glb_slot_timings(
                args.glb,
                expected_slots=expected,
                allowed_slots=allowed,
                cue_frames=cues,
                fps=args.fps,
            )
            lo, hi = info["frame_range"]
            print(
                f"SLOTTED GLB OK: action={info['action']} slots={','.join(info['slots'])} "
                f"frames={lo:g}..{hi:g} fps={info['fps']:g}"
            )
            return 0
    except (SlotContractError, OSError, ValueError) as exc:
        print(f"SLOTTED GLB ERROR: {exc}", file=sys.stderr)
        return 1
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
