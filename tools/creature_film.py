"""Headless Blender: film one creature clip as HLH's follow camera sees it.

    blender -b --factory-startup -P tools/creature_film.py -- --glb RIG.glb --clip NAME --out PREFIX
        [--frames 8] [--loops 1] [--move auto|no] [--crop x0,x1,y0,y1] [--measure-only]
        [--video PREFIX.mp4 --ffmpeg PATH]

The camera is the game's (HLH `bevy/src/fx.rs`, `gameplay.rs`): 75 degrees
of vertical view, pitched 0.32 rad (18.3 degrees) down, 3.0 m behind the
player and looking at a point 1.535 m above the player's feet. The player
is not drawn; the creature stands in front of them, facing them, its
nearest point 1.5 m away, at the size its file gives. The frame is a 4K
screen (3840 x 2160) cut to the creature (`--crop`, as shares of the
frame, so two films can share one cut): the game's own view, with the
pixels a large screen gives it.

`--move auto`: a clip whose name has `walk` or `trot` in it is played in
place by the route (its feet are planted relative to a body that moves),
so the body is carried toward the player at the speed its planted feet go
back, measured from the clip; any other clip stays where it stands.

Prints `CREATURE_FILM {...}` (frames written, the crop, the speed).
Workbench: the painted colour, cavity and shadows, a plain floor.
"""
import json
import math
import os
import subprocess
import sys

import bpy
from mathutils import Matrix, Vector

FOV_DEG = 75.0
PITCH = 0.32
DISTANCE = 3.0
FOCUS = 1.535
GAP = 1.5
WIDTH, HEIGHT = 3840, 2160


def options():
    argv = sys.argv[sys.argv.index("--") + 1:]
    o = {"frames": "8", "loops": "1", "move": "auto", "fps": "24"}
    k = 0
    while k < len(argv):
        key = argv[k][2:]
        if key == "measure-only":
            o[key] = True
            k += 1
            continue
        o[key] = argv[k + 1]
        k += 2
    return o


def bind(arms, act):
    for arm in arms:
        if arm.animation_data is None:
            arm.animation_data_create()
        ad = arm.animation_data
        for tr in list(ad.nla_tracks):
            ad.nla_tracks.remove(tr)
        ad.action = act
        if act is not None and hasattr(ad, "action_suitable_slots") and len(ad.action_suitable_slots):
            slots = list(ad.action_suitable_slots)
            pick = next((s for s in slots if getattr(s, "name_display", "") in (arm.name, arm.data.name)), None)
            ad.action_slot = pick if pick is not None else slots[0]


def mesh_points(objs):
    dg = bpy.context.evaluated_depsgraph_get()
    pts = []
    for ob in objs:
        if ob.type != "MESH":
            continue
        e = ob.evaluated_get(dg)
        me = e.to_mesh()
        mw = e.matrix_world
        pts += [mw @ v.co for v in me.vertices]
        e.to_mesh_clear()
    return pts


def set_time(scene, f):
    fi = int(math.floor(f))
    scene.frame_set(fi, subframe=max(0.0, min(0.9999, f - fi)))


def foot_speed(scene, arm, f0, f1, ground, height):
    """How fast planted feet go back in an in-place clip (m/s): bones whose
    tails touch the ground, followed from one frame to the next."""
    fps = scene.render.fps
    prev, speeds = None, []
    f = f0
    while f <= f1:
        set_time(scene, f)
        tails = {pb.name: (arm.matrix_world @ pb.tail) for pb in arm.pose.bones}
        if prev is not None:
            for n, p in tails.items():
                q = prev.get(n)
                if q is not None and p.z <= ground + 0.03 * height and q.z <= ground + 0.03 * height:
                    speeds.append((p.y - q.y) * fps)
        prev = tails
        f += 1.0
    if not speeds:
        return 0.0
    speeds.sort()
    return max(0.0, speeds[len(speeds) // 2])


def main():
    o = options()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    fps = int(o["fps"])
    scene.render.fps = fps
    known = set(bpy.ops.import_scene.gltf.get_rna_type().properties.keys())
    kw = {"disable_bone_shape": True} if "disable_bone_shape" in known else {}
    bpy.ops.import_scene.gltf(filepath=os.path.abspath(o["glb"]), **kw)
    objs = [x for x in bpy.data.objects if x.type in ("MESH", "ARMATURE", "EMPTY")]
    arms = [x for x in objs if x.type == "ARMATURE"]
    acts = {a.name: a for a in bpy.data.actions}
    act = acts.get(o["clip"]) or next((a for n, a in acts.items() if n.split(".")[0] == o["clip"]), None)
    if act is None:
        print("CREATURE_FILM " + json.dumps({"error": f"no clip {o['clip']} (have {sorted(acts)})"}))
        sys.exit(3)
    bind(arms, act)
    f0, f1 = float(act.frame_range[0]), float(act.frame_range[1])
    # every object under one root we can carry
    roots = [x for x in objs if x.parent is None]
    carrier = bpy.data.objects.new("carrier", None)
    scene.collection.objects.link(carrier)
    for r in roots:
        r.parent = carrier

    set_time(scene, f0)
    rest = mesh_points(objs)
    lo = Vector((min(p.x for p in rest), min(p.y for p in rest), min(p.z for p in rest)))
    hi = Vector((max(p.x for p in rest), max(p.y for p in rest), max(p.z for p in rest)))
    height = max(hi.z - lo.z, 1e-3)
    # glTF +Z forward is Blender -Y: the creature faces -Y. The player
    # stands at the origin facing +Y; the creature's front is GAP away.
    carrier.location = Vector((-(lo.x + hi.x) / 2, GAP - lo.y, -lo.z))
    ground = 0.0

    loops = max(1, int(o["loops"]))
    span = (f1 - f0) * loops
    n = max(1, int(o["frames"]))
    times = [f0 + span * i / n for i in range(n)]
    speed = 0.0
    moving = o["move"] == "auto" and any(w in act.name.lower() for w in ("walk", "trot", "run", "gallop"))
    if moving and arms:
        speed = foot_speed(scene, arms[0], f0, f1, ground, height)
    start_y = carrier.location.y + (speed * span / fps if moving else 0.0)

    def place(f):
        t = (f - f0) / fps
        carrier.location.y = start_y - speed * t
        clip_f = f0 + ((f - f0) % max(f1 - f0, 1e-6)) if f > f1 else f
        set_time(scene, clip_f)

    # the game's camera
    cam_data = bpy.data.cameras.new("cam")
    cam_data.sensor_fit = "VERTICAL"
    cam_data.angle = math.radians(FOV_DEG)
    cam_data.clip_start = 0.05
    cam_data.clip_end = 500.0
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    focus = Vector((0.0, 0.0, FOCUS))
    eye = focus + Vector((0.0, -DISTANCE * math.cos(PITCH), DISTANCE * math.sin(PITCH)))
    fwd = (focus - eye).normalized()
    right = fwd.cross(Vector((0.0, 0.0, 1.0))).normalized()
    up = right.cross(fwd).normalized()
    cam.matrix_world = Matrix.Translation(eye) @ Matrix((right, up, -fwd)).transposed().to_4x4()
    scene.camera = cam
    scene.render.resolution_x, scene.render.resolution_y = WIDTH, HEIGHT
    scene.render.resolution_percentage = 100
    tan = math.tan(math.radians(FOV_DEG) / 2)
    aspect = WIDTH / HEIGHT

    def project(p):
        v = p - eye
        z = v.dot(fwd)
        x = v.dot(right) / (z * tan * aspect)
        y = v.dot(up) / (z * tan)
        return (x + 1) / 2, (y + 1) / 2

    if o.get("crop"):
        crop = [float(x) for x in o["crop"].split(",")]
    else:
        xs, ys = [], []
        for f in times + [f0 + span * (i + 0.5) / n for i in range(n)]:
            place(f)
            for p in mesh_points(objs):
                u, w = project(p)
                xs.append(u)
                ys.append(w)
        pad = 0.08 * max(max(xs) - min(xs), max(ys) - min(ys))
        crop = [max(0.0, min(xs) - pad), min(1.0, max(xs) + pad), max(0.0, min(ys) - pad), min(1.0, max(ys) + pad)]
    info = {"clip": act.name, "seconds": round((f1 - f0) / fps, 3), "loops": loops, "frames": n,
            "crop": [round(c, 5) for c in crop], "speed_mps": round(speed, 4), "height_m": round(height, 4),
            "crop_px": [round((crop[1] - crop[0]) * WIDTH), round((crop[3] - crop[2]) * HEIGHT)]}
    if o.get("measure-only"):
        print("CREATURE_FILM " + json.dumps(info))
        return

    # floor, light, look
    g = bpy.data.meshes.new("floor")
    s = 60.0
    g.from_pydata([(-s, -s, 0), (s, -s, 0), (s, s, 0), (-s, s, 0)], [], [(0, 1, 2, 3)])
    gm = bpy.data.materials.new("floor")
    gm.diffuse_color = (0.55, 0.56, 0.55, 1.0)
    g.materials.append(gm)
    scene.collection.objects.link(bpy.data.objects.new("floor", g))
    scene.render.engine = "BLENDER_WORKBENCH"
    sh = scene.display.shading
    sh.light = "STUDIO"
    sh.color_type = "TEXTURE"
    sh.show_shadows = True
    sh.show_cavity = True
    scene.display.render_aa = "8"
    w = bpy.data.worlds.new("sky")
    w.color = (0.78, 0.80, 0.83)
    scene.world = w
    scene.view_settings.view_transform = "Standard"
    scene.render.use_border = True
    scene.render.use_crop_to_border = True
    scene.render.border_min_x, scene.render.border_max_x = crop[0], crop[1]
    scene.render.border_min_y, scene.render.border_max_y = crop[2], crop[3]
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    out = []
    for i, f in enumerate(times):
        place(f)
        scene.render.filepath = f"{o['out']}_{i + 1:03d}.png"
        bpy.ops.render.render(write_still=True)
        out.append(scene.render.filepath)
    info["pngs"] = out
    if o.get("video"):
        # every frame of the clip, looped, at the game's speed, for people
        vdir = o["out"] + "_video"
        os.makedirs(vdir, exist_ok=True)
        total = int(round(span))
        for k in range(total):
            place(f0 + k)
            scene.render.filepath = os.path.join(vdir, f"{k:04d}.png")
            bpy.ops.render.render(write_still=True)
        ff = o.get("ffmpeg") or "ffmpeg"
        r = subprocess.run([ff, "-loglevel", "error", "-y", "-framerate", str(fps), "-i", os.path.join(vdir, "%04d.png"),
                            "-vf", "scale=trunc(iw/4)*2:trunc(ih/4)*2", "-pix_fmt", "yuv420p", o["video"]],
                           capture_output=True, text=True)
        info["video"] = o["video"] if r.returncode == 0 else None
        if r.returncode != 0:
            info["video_error"] = r.stderr[-200:]
    print("CREATURE_FILM " + json.dumps(info))


main()
