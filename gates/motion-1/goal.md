# Goal: motion-1 — Grip measure read from the hand's skin

Implement the skin-based sword grip measure in `rust/core/src/readcam.rs` so grip quality is measured directly from the weapon hand's skinned mesh triangles (handle-to-palm-surface distance and handle circumference wrap), without relying on a bone-ratio palm heuristic.

## Scope

- Edit ONLY `rust/core/src/` (primarily `rust/core/src/readcam.rs`).
- Do NOT edit `rust/core/tests/`, `rust/cli/`, `tools/bench/grip_gate.sh`, `gates/motion-1/`, `Cargo.toml`, `Cargo.lock`, or docs.
- Keep zero external crates (`--locked` offline build) and zero compiler warnings.
- Byte-deterministic across macOS arm64 and Linux x86_64: use `crate::detmath::{acos, sin_cos, ...}` — NEVER call platform `f64` libm methods (`.sin()`, `.cos()`, `.acos()`, `.asin()`, `.atan2()`, `.exp()`). Use `f64::total_cmp` in `median`.

## Design

In `rust/core/src/readcam.rs`:

1. **Public constants** (metres at the game's 2 m stature, GLB units divided by `unit`):
   - `pub const SKIN_PALM_GOOD_M: f64 = 0.040;` — handle axis within 4.0 cm of nearest palm skin surface gets full distance score (`1.0`).
   - `pub const SKIN_PALM_BAD_M: f64 = 0.085;` — handle axis `>= 8.5 cm` from palm skin gets `0.0`.
   - `pub const SKIN_WRAP_BAD: f64 = 0.30;` — circumference coverage `<= 30%` (flat palm on one side) gets `0.0`.
   - `pub const SKIN_WRAP_GOOD: f64 = 0.85;` — circumference coverage `>= 85%` (palm + curled fingers wrapping handle) gets `1.0`.
   - `pub const SKIN_WRAP_HALF_LEN_M: f64 = 0.055;` — axial half-band along the handle axis (`±5.5 cm`).
   - `pub const SKIN_WRAP_MIN_R_M: f64 = 0.004;` — minimum radial distance from handle axis.
   - `pub const SKIN_WRAP_MAX_R_M: f64 = 0.045;` — maximum radial reach from handle axis (`4.5 cm`).
   - `pub const SKIN_WRAP_BINS: usize = 24;` — 24 equal angular sectors (15° each) around the handle axis.

2. **`GripScore` fields** (keep all existing fields intact, add 3 new public fields with defaults `0.0`, `0.0`, `false` in `Default`):
   - `pub palm_surface_m: f64` — median over strike frames (`load..=settle`) of the exact minimum point-to-triangle distance from the handle's middle (`grip`) to the weapon hand's palm skin triangles, divided by `unit`.
   - `pub skin_wrap: f64` — median over strike frames (`load..=settle`) of the fraction (`0.0..=1.0`) of the `SKIN_WRAP_BINS` angular sectors around the handle axis covered by weapon-hand + finger skinned triangles.
   - `pub skin_based: bool` — `true` when the weapon hand has at least one skinned mesh triangle (`score` computed via `skin_grip_of`); `false` only on meshes with zero hand-skinned triangles (falls back to existing `grip_of_full`).

3. **Pure scoring function**:
   - `pub fn skin_grip_of(palm_surface_m: f64, skin_wrap: f64, curl_deg: Option<f64>) -> f64`
   - `palm_part = (1.0 - (palm_surface_m - SKIN_PALM_GOOD_M) / (SKIN_PALM_BAD_M - SKIN_PALM_GOOD_M)).clamp(0.0, 1.0)`
   - `wrap_part = ((skin_wrap - SKIN_WRAP_BAD) / (SKIN_WRAP_GOOD - SKIN_WRAP_BAD)).clamp(0.0, 1.0)`
   - `base = palm_part.min(wrap_part)`
   - When `curl_deg` is `Some(c)`: `base.min((c / GRIP_CURL_FULL_DEG).clamp(0.0, 1.0))` (note: uses `GRIP_CURL_FULL_DEG = 35.0`, no bone-ratio mitten special case!).
   - When `curl_deg` is `None`: `base`.

4. **Hand & palm skin extraction in `Stage::new`**:
   - Starting from `self.hand = Some(h)`:
     - Collect `hand_nodes`: `h` plus all descendant nodes of `h` in `subject.rig.children`, skipping `Socket_*` and `crate::helpers::is_helper` nodes.
     - Collect `palm_nodes`: `h` plus any direct/descendant `DEF-palm.*` nodes under `h`.
     - For each vertex `vi` in `mesh.verts`:
       - `is_hand[vi]` if sum of weights `w` for `(j, w)` with `j in hand_nodes` is `>= 0.5`.
       - `is_palm[vi]` if sum of weights `w` for `(j, w)` with `j in palm_nodes` is `>= 0.5`.
     - `hand_tris`: indices of triangles `[a, b, c]` in `mesh.tris` where `is_hand[a] || is_hand[b] || is_hand[c]`.
     - `palm_tris`: indices of triangles `[a, b, c]` in `mesh.tris` where `is_palm[a] || is_palm[b] || is_palm[c]`. If `palm_tris` is empty, fall back to `hand_tris`.
     - Precompute the compact set of vertex indices needed by `hand_tris` (or skin only those vertices per strike frame) so `Stage::read` stays fast.

5. **Per-frame skin geometry in `Stage::read`** (for each strike frame `f` in `load..=settle` when `!self.hand_tris.is_empty()`):
   - Skin the hand vertices for `world[f]` (same linear blend skinning as `Mesh::posed`, using `mesh.rest_inv`).
   - **`palm_surface_m`**: exact minimum point-to-triangle distance from `grip` to every posed triangle in `palm_tris`, divided by `self.unit`. Guard degenerate triangles (zero area / zero edge denominator) cleanly without dividing by zero or producing NaN.
   - **`skin_wrap`**:
     - Build deterministic orthonormal frame `(u, v)` perpendicular to `dir`:
       `ref_v = if dir.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) }`,
       `u = dir.cross(ref_v).normalized()`, `v = dir.cross(u)`.
     - Maintain `bins = [false; SKIN_WRAP_BINS]`.
     - For each posed triangle `(a, b, c)` in `hand_tris`, sample deterministic barycentric grid points with `steps = ((max_edge_len / (0.012 * self.unit)).ceil() as usize).clamp(2, 6)` so both coarse procedural box meshes and dense sculpted hands are sampled accurately without gaps:
       for `i in 0..=steps`, `j in 0..=(steps - i)`, `p = a * (1 - u_b - v_b) + b * u_b + c * v_b`.
     - For each sample point `p`:
       - `dv = p - grip`, `along = dv.dot(dir) / self.unit`. Skip if `along.abs() > SKIN_WRAP_HALF_LEN_M`.
       - `rad = dv - dir.scale(along * self.unit)`, `r = rad.length() / self.unit`. Skip if `r < SKIN_WRAP_MIN_R_M || r > SKIN_WRAP_MAX_R_M`.
       - Unit radial components `rx = rad.dot(u) / (r * self.unit)`, `ry = rad.dot(v) / (r * self.unit)`.
       - `let mut ang = detmath::acos(rx.clamp(-1.0, 1.0)); if ry < 0.0 { ang = std::f64::consts::TAU - ang; }`
       - `let bi = ((ang / std::f64::consts::TAU) * SKIN_WRAP_BINS as f64).floor() as usize % SKIN_WRAP_BINS;`
       - `bins[bi] = true;`
     - Frame wrap = `bins.iter().filter(|&&b| b).count() as f64 / SKIN_WRAP_BINS as f64`.
   - Take the median of per-frame `palm_surface_m` and `skin_wrap` over the strike frames.
   - Compute `score = skin_grip_of(palm_surface_m, skin_wrap, curl)`.
   - Keep `handle_m`, `axis_m`, `curl_deg`, `tip_m`, `fingers`, `frames` populated exactly as before.

6. **JSON & summary line (`ReadCam::to_json`, `ReadCam::line`)**:
   - In `to_json()`'s `"grip"` object, add:
     - `("palm_surface_cm", Json::num(r(self.grip.palm_surface_m * 100.0, 1)))`
     - `("skin_wrap", Json::num(r(self.grip.skin_wrap, 3)))`
     - `("skin_based", Json::Bool(self.grip.skin_based))`
   - In `line()`, when `self.grip.skin_based` is true, include `palm {:.1} cm, wrap {:.0}%` in the grip clause.

## Evidence of Done

- `gates/motion-1/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- All 4 new tests in `rust/core/tests/skin_grip.rs` pass, all existing tests (excluding the two motion-2 tests) pass, `cargo fmt --all --check` passes, zero warnings, `detmath` tests pass, and `tools/bench/grip_gate.sh` prints `GRIP GATE PASS`.
