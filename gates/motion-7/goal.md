# motion-7: Giant pass (`motionforge giant --scale k`)

Implement the giant animation pass so giant enemies (slow-motion giant reapers,
upside-down bridge-pinned railway giant with arms-only attacks, and giant
walkers) have physical weight, Froude timing, hit-stop / cancel-window event
metadata in GLB `extras`, and a pinned-pelvis / arms-only body mask — rather
than a naive uniform slow-down.

## Required files & scope

Only touch unguarded source files:
- `rust/core/src/giant.rs` (new module)
- `rust/core/src/lib.rs` (export `pub mod giant;`)
- `rust/cli/src/main.rs` (add `giant` subcommand & help)
- `rust/cli/src/glb_cmds.rs` (add `cmd_giant`)

Do NOT edit `rust/core/tests/`, `rust/cli/tests/`, `tests/fixtures/`,
`rust/core/src/readcam.rs`, `rust/core/src/motioncheck.rs`, `tools/bench/`,
`tools/gate/`, or `gates/`. All CLI contract goldens must remain byte-identical.

## Core API (`rust/core/src/giant.rs`)

1. `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum BodyMask { Full, ArmsOnly }`
2. `#[derive(Clone, Debug)] pub struct GiantOptions`:
   - `pub scale: f64` (default `4.0`; must be finite and `> 0.0`, else return `Err`)
   - `pub anticipation: f64` (default `0.35`, `>= 0.0`)
   - `pub settle: f64` (default `0.35`, `>= 0.0`)
   - `pub hit_stop_s: f64` (default `0.12`, `>= 0.0`)
   - `pub cancel_window_s: f64` (default `0.25`, `>= 0.0`)
   - `pub mask: BodyMask` (default `BodyMask::Full`)
   - `pub pin_pelvis: bool` (default `false`)
   - `pub replant_feet: bool` (default `true`)
   Implement `Default for GiantOptions`.
3. `#[derive(Clone, Debug)] pub struct GiantReport`:
   - `pub name: String`
   - `pub source: String`
   - `pub scale: f64`
   - `pub source_duration_s: f64`
   - `pub output_duration_s: f64`
   - `pub root_travel_before_m: f64`
   - `pub root_travel_after_m: f64`
   - `pub hit_time_s: Option<f64>`
   - `pub hit_stop: Option<(f64, f64)>`
   - `pub cancel_window: Option<(f64, f64)>`
   - `pub mask: BodyMask`
   - `pub pin_pelvis: bool`
4. `pub fn giant_glb(doc: &Document, opts: &GiantOptions, clips: &[String], suffix: Option<&str>) -> Result<(Document, Vec<GiantReport>), String>`

## Mathematical & physics behavior

1. **Uniform rig & root scaling (`scale = k`)**:
   - Scale every node's rest `translation` by `k` (keeping `rotation` and `scale`
     unchanged), every mesh primitive's `POSITION` accessor vertices (and `min`/`max`)
     by `k`, every skin's `inverseBindMatrices` translation column `(col 3: indices 12,13,14)`
     by `k`, and every animation `translation` channel keyframe by `k`.
   - Result: character height scales by `k`, root horizontal travel scales by `k`
     (`k = 4` => `4x` root travel), floor stays at `y = 0`, and foot plants / contacts
     remain physically consistent without introducing sink or skate!
2. **Froude time scaling (`sqrt(k)`)**:
   - Output clip duration is `T_out = T_src * detmath::sqrt(k)` (so `k = 4` runs
     `2x` as long), sampled cleanly at 30 fps (or source fps).
   - Use `crate::detmath` exclusively — ZERO platform libm calls (`.sqrt()`, `.sin()`,
     `.cos()`, `.acos()`, `.atan2()`, etc. are forbidden by the gate).
3. **Anticipation, hit-stop, and heavy settle (distinct from plain slow-down)**:
   - Detect the clip's peak motion / strike moment (e.g. peak hand/tip speed on
     attack clips or peak body speed on general clips).
   - When `anticipation == 0.0 && settle == 0.0 && hit_stop_s == 0.0`, resample
     with plain Froude time mapping `t_src = t_out / sqrt(k)`.
   - When `anticipation > 0.0 || settle > 0.0 || hit_stop_s > 0.0`:
     - Build a monotone time-warp `t_src(t_out)` spanning `[0, T_out] -> [0, T_src]`
       that:
       (a) lengthens the anticipation windup before peak speed (with an optional
           subtle counter-load on upper-body/arm rotations that ramps cleanly to 0
           at clip ends and impact),
       (b) snaps into the impact time `t_hit` faster than uniform slowdown,
       (c) holds `t_src` constant at the impact pose across the `hit_stop` window
           `[hs_start, hs_end]` of duration `min(hit_stop_s, 0.25 * T_out)` so the
           striking hand pauses steadily (`< 0.02 * height` per frame),
       (d) eases heavily with a damped settle curve through recovery to `T_src`
           at `T_out`.
     - Re-plant / preserve grounded foot plants when `opts.replant_feet && opts.mask == BodyMask::Full`
       (ensure foot skate and sink do not worsen vs source).
4. **Hit-stop & cancel-window events in GLB `extras`**:
   - Compute `hit_stop = (hs_start, hs_end)` (when `hit_stop_s > 0.0`) and
     `cancel_window = (cw_start, cw_end)` starting at `hs_end` (or `t_hit` if
     `hit_stop_s == 0.0`) with duration `min(cancel_window_s, T_out - cw_start)`
     when `cancel_window_s > 0.0`.
   - Attach an `"extras"` JSON object on each processed glTF animation entry
     under `"motionforge_giant"`:
     - `"scale"`: `opts.scale`
     - `"mask"`: `"full"` or `"arms-only"`
     - `"pin_pelvis"`: `bool` (`opts.pin_pelvis || opts.mask == BodyMask::ArmsOnly`)
     - `"source_duration_s"`: `f64`
     - `"duration_s"`: `f64`
     - `"hit_time_s"`: `f64`
     - `"events"`: JSON array of `{"kind": "hit_stop", "start_s": ..., "end_s": ...}`
       and `{"kind": "cancel_window", "start_s": ..., "end_s": ...}`.
5. **Body mask (`BodyMask::ArmsOnly` and `pin_pelvis`)**:
   - When `opts.pin_pelvis || opts.mask == BodyMask::ArmsOnly`:
     - Keep `DEF-spine` (hips/pelvis) pinned at its rest translation and rotation
       on every frame (root travel = 0).
   - When `opts.mask == BodyMask::ArmsOnly`:
     - Also keep all lower-body bones (`DEF-thigh.L/R`, `DEF-shin.L/R`,
       `DEF-foot.L/R`, `DEF-toe.L/R` and any descendants of those leg bones)
       pinned at rest on every frame, while upper spine (`DEF-spine.001+`),
       neck/head, shoulders, arms, hands, fingers, and hand sockets animate
       normally!
6. **CLI subcommand (`motionforge giant`)**:
   - Flags: `--input <glb>` (required), `--output <glb>` (required),
     `--scale <f64>` (default `4.0`), `--clip <name,...>` (optional repeatable),
     `--suffix <str>` (optional), `--anticipation <f64>` (default `0.35`),
     `--settle <f64>` (default `0.35`), `--hit-stop <f64>` (default `0.12`),
     `--cancel-window <f64>` (default `0.25`), `--mask full|arms-only` (default `full`),
     `--pin-pelvis`, `--no-replant`.
   - Deterministic output (`to_bytes()` byte-for-byte identical across runs) and
     zero compiler warnings (`RUSTFLAGS="-D warnings"`).
