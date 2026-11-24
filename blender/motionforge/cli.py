# SPDX-License-Identifier: GPL-3.0-or-later
"""Subprocess bridge to the motionforge CLI (no bpy).

The GPL extension talks to the MIT core only through this module:
argv in, report text + output files out. All paths are passed in by the
caller (which resolves them via bpy.path); nothing here imports bpy,
so plain-python tests can exercise the arg builders.
"""

import os
import shutil
import subprocess


class CliError(Exception):
    """A CLI run that failed (message already includes stderr)."""


def find_motionforge_binary(explicit_path=""):
    """Locate the `motionforge` CLI: explicit preference, $MOTIONFORGE_BIN,
    PATH, $CARGO_TARGET_DIR, then the build tree relative to this extension."""
    env_bin = os.environ.get("MOTIONFORGE_BIN", "")
    for cand in (explicit_path, env_bin):
        if cand and os.path.isfile(cand) and os.access(cand, os.X_OK):
            return cand
    on_path = shutil.which("motionforge")
    if on_path:
        return on_path
    here = os.path.dirname(os.path.abspath(__file__))
    candidates = [
        os.path.join(here, "..", "..", "rust", "target", "release", "motionforge"),
        os.path.join(here, "..", "..", "rust", "target", "debug", "motionforge"),
    ]
    target_dir = os.environ.get("CARGO_TARGET_DIR", "")
    if target_dir:
        candidates.extend([
            os.path.join(target_dir, "release", "motionforge"),
            os.path.join(target_dir, "debug", "motionforge"),
        ])
    for candidate in candidates:
        candidate = os.path.normpath(candidate)
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    return ""


def run_cli(binary, args, timeout=300):
    """Run the CLI; return stdout on success, raise CliError otherwise."""
    try:
        proc = subprocess.run(
            [binary] + args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
    except OSError as exc:
        raise CliError(f"cannot run {binary}: {exc}") from exc
    stdout = proc.stdout.decode("utf-8", "replace")
    stderr = proc.stderr.decode("utf-8", "replace")
    if proc.returncode != 0:
        raise CliError(f"motionforge exited {proc.returncode}: {stderr.strip() or stdout.strip()}")
    return stdout


def build_retarget_args(source, target, bonemap, output, yaw="auto", pin=True):
    args = [
        "retarget",
        "--source", source,
        "--target", target,
        "--map", bonemap,
        "--output", output,
        "--yaw", yaw,
    ]
    args.append("--pin" if pin else "--no-pin")
    return args


def build_stylize_args(input_path, output, params, keys_out=None):
    """params: dict with the StylizeParams keys (chain_factors a list of
    (substr, factor) tuples). keys_out requests the keys sidecar file."""
    args = ["stylize", "--input", input_path, "--output", output]
    if keys_out is not None:
        args += ["--keys-out", keys_out]
    for key in (
        "exaggeration",
        "angle_threshold",
        "min_spacing",
        "hold",
        "anticipation",
        "anticipation_frames",
        "overshoot",
    ):
        args += ["--" + key.replace("_", "-"), repr(params[key])]
    for substr, factor in params.get("chain_factors", []):
        args += ["--chain", f"{substr}:{factor!r}"]
    return args


def build_physics_args(command, input_path, output, params, frame=None):
    """command: 'physics-check', 'physics-fix', or 'physics-frame'.
    params: dict with the PhysicsParams keys (feet a list of names).
    physics-frame needs the clip frame index."""
    args = [command, "--input", input_path]
    if command == "physics-frame":
        if frame is None:
            raise ValueError("physics-frame needs a frame index")
        args += ["--frame", repr(frame)]
    if output is not None:
        args += ["--output", output]
    args += ["--root", params["root"], "--feet", ",".join(params["feet"])]
    for key in (
        "contact_margin",
        "foot_radius",
        "balance_margin",
        "min_air_frames",
        "accel_limit",
        "turn_deg",
        "min_turn_speed",
    ):
        args += ["--" + key.replace("_", "-"), repr(params[key])]
    if command == "physics-fix":
        for key in ("blend_frames", "smooth_sigma", "smooth_pad"):
            args += ["--" + key.replace("_", "-"), repr(params[key])]
        if not params.get("fix_ballistic", True):
            args.append("--no-ballistic")
        if not params.get("fix_momentum", True):
            args.append("--no-momentum")
        if not params.get("fix_balance", True):
            args.append("--no-balance")
        if "max_lean_deg" in params:
            args += ["--max-lean-deg", repr(params["max_lean_deg"])]
    return args


def build_autopose_args(model, effectors, output):
    return ["autopose", "--model", model, "--effectors", effectors, "--output", output]


def parse_chain_factors(text):
    """Parse 'substr:factor, ...' into [(substr, factor)]. Empty -> []."""
    text = (text or "").strip()
    if not text:
        return []
    out = []
    for chunk in text.split(","):
        chunk = chunk.strip()
        if ":" not in chunk:
            raise ValueError(f"chain factors must be 'substr:factor', got '{chunk}'")
        substr, _, factor = chunk.partition(":")
        substr, factor = substr.strip(), factor.strip()
        if not substr:
            raise ValueError(f"chain factors need a bone substring, got '{chunk}'")
        try:
            value = float(factor)
        except ValueError as exc:
            raise ValueError(f"chain factor must be a number, got '{chunk}'") from exc
        out.append((substr, value))
    return out


def parse_feet(text):
    """Parse 'a, b, ...' into [names]. Empty -> ValueError."""
    names = [n.strip() for n in (text or "").split(",") if n.strip()]
    if not names:
        raise ValueError("feet needs at least one bone name")
    return names


def build_creature_solve_args(creature, detections, output, glb_in="", glb_out="", clip_name="creature", pin=True):
    """argv for `motionforge creature-solve` (two-view detections -> clip, + GLB)."""
    args = ["creature-solve", "--creature", creature, "--detections", detections, "--output", output]
    if glb_in:
        args += ["--glb-in", glb_in, "--glb-out", glb_out, "--clip-name", clip_name]
    if not pin:
        args.append("--no-pin")
    return args
