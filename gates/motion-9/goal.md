# motion-9: Non-quadruped creature locomotion (two-legged monsters + legless body wave)

Extend `motion_core::creature::anybody` and `motionforge creature-clips` so
creatures that are not four-legged (specifically two-legged upright and hunched
long-armed monsters, plus legless undulating creatures) generate real looping
locomotion clips (`walk`) instead of standing at rest.

## Required files & scope

Only touch unguarded source files:
- `rust/core/src/creature/anybody.rs` (add `walk_stride`, helper visibility if needed)
- `rust/core/src/creature/gait.rs` (only if making `leg_rig` / `place_leg` / `LegRig` `pub(crate)` for reuse — do NOT break any quadruped gait behavior or tests!)
- `rust/cli/src/creature_cmds.rs` (add `"walk"` support to `motionforge creature-clips --clips ...`)
- `rust/cli/src/main.rs` (update `creature-clips` help text if appropriate; do NOT break any CLI golden!)

Do NOT edit `rust/core/tests/`, `rust/cli/tests/`, `tests/fixtures/`,
`rust/core/src/readcam.rs`, `rust/core/src/motioncheck.rs`, `tools/bench/`,
`tools/gate/`, or `gates/`. All existing CLI contract goldens must remain
byte-identical (keep the default `--clips` value of `creature-clips` unchanged
if any test depends on it; explicitly accepting `walk` in `--clips idle,walk,attack`).

## Core API (`rust/core/src/creature/anybody.rs`)

Add:
```rust
pub fn walk_stride(c: &Creature, body: &Body, fps: f64) -> Result<Stride, String>
```
(reusing `crate::creature::gait::Stride`).

### 1. Two-legged creatures (`body.legs.len() == 2`)
Handles both upright bipeds and hunched long-armed monsters:
- Order legs deterministically (left first, right second using `left_of(c)`).
- Reuse `gait`'s FABRIK / leg IK (`leg_rig` + `place_leg`, or clean `pub(crate)`
  helpers from `gait.rs`) so bone lengths are preserved to `< 1e-6` on every frame.
- Alternating gait with double support (`offsets = [0.0, 0.5]`, `duty ~ 0.60..0.65`)
  so both feet are NEVER airborne simultaneously.
- Froude cadence from leg drop (`hip`), stride capped to what the legs can reach
  with a small crouch (`~0.06..0.08 * hip`) and subtle vertical bob (`~0.025 * hip`)
  plus subtle lateral weight shift toward the stance foot.
- Planted foot slides back under the body at exact treadmill `speed = stance / (duty * period)`
  during stance so world-space foot skate is `<= 2.5 cm` (ideally `< 0.005 * c.height`)!
- Swing phase clears the ground (`top > rest_z + 0.03 * c.height`) and never
  sinks below `rest_z - 0.005 * c.height`.
- Free spine/head sway gently with the step; free arms (`body.arms`) counter-swing
  in opposite phase to their ipsilateral leg (`> 10 deg` visible swing on long-armed
  hunched monsters!).
- Determinism: use `crate::detmath` for trig (zero platform `sin`/`cos`/`acos`).

### 2. Legless creatures (`body.floats` / `body.legs.is_empty()`)
Handles serpents, wyrms, fish, and cloaked/formless spine creatures:
- Require at least 3 bones along the body chain (`body.front` + `body.back` combined);
  if fewer than 2 movable body segments exist (e.g. a 1-bone skeleton), return `Err(...)`.
- Order the body chain head-to-tail along `-c.forward` and drive a **traveling
  sinusoidal lateral S-wave**:
  `yaw(s, t) = amp(s) * detmath::sin(TAU * t - wave_k * s)`
  with `wave_k` producing at least ~90–180° phase lag from front to rear (so the
  wave propagates head-to-tail rather than wagging rigidly in phase!), amplitude
  growing gently toward the tail, producing clear lateral excursion (`> 0.02 * body.size`).
- If `body.arms` is non-empty (fins/wings), add a synchronized smooth scull/flap.
- Set `speed > 0.1` proportional to `body.size` (e.g. `0.6 * body.size`),
  `frames >= 16` at `fps`, `period = frames / fps`, `stance = speed * period`.
- Loops smoothly with zero bone stretching.

### 3. Unsupported body plans
- Single-legged or degenerate (< 2 movable bones) skeletons return a clear `Err(...)`.

## CLI (`rust/cli/src/creature_cmds.rs`)
- In `cmd_clips` (`motionforge creature-clips`), handle `"walk"` in `--clips`:
  call `anybody::walk_stride(&c, &body, fps)` and write the `"walk"` animation into
  the output GLB via `clip_to_glb(&doc, &stride.clip, "walk")`, logging its frame
  count, fps, speed, and stance.
- Update the unknown-clip error message to list `(idle, walk, attack, sweep)`.
