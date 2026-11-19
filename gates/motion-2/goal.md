# Goal: motion-2 — Make the attack wrist solver land exactly (< 1e-9) and turn CI green

Fix the wrist-placement residual in `rust/core/src/attack/` so `single_finger_chain_places_palm_socket_and_curls_round_the_handle` lands within its strict `1e-9` grip tolerance without loosening any test tolerance, turning the entire Rust test suite and CI green.

## Context

`distal_mitten_places_socket_in_palm_pocket_and_rejects_fingertip_socket` has already been updated to test against `motion-1`'s skin-based grip measure and passes.

The remaining failing test on `main` is `rust/core/tests/attack.rs::single_finger_chain_places_palm_socket_and_curls_round_the_handle`, which fails on key 2 with:
`Miss { grip: 3.5679523233734986e-8, blade_deg: 22.604..., drop: 0.0, foot: 2.48e-14 }`
against the strict `assert!(m.grip < 1e-9)` check.

## Scope

- Edit ONLY `rust/core/src/attack/` (`rust/core/src/attack/mod.rs` and/or `rust/core/src/attack/ik.rs`).
- Do NOT edit `rust/core/tests/`, `rust/cli/`, `rust/core/src/readcam.rs`, `tools/bench/`, `gates/motion-2/`, `Cargo.toml`, `Cargo.lock`, or docs.
- Keep zero external crates (`--locked` offline build), zero compiler warnings (`RUSTFLAGS="-D warnings"`), and strict byte-determinism across macOS arm64 and Linux x86_64 (`crate::detmath` only — never platform `f64` libm methods).

## Design

1. Diagnose why `Solver::place_hand` (`rust/core/src/attack/mod.rs`) leaves a `3.57e-8` residual on `at = w[ha].t + rot.rotate_vec(hold.grip).scale(blend)` for key 2 of `single_finger_chain_places_palm_socket_and_curls_round_the_handle` (twisted A-pose single-finger rig):
   - Check `ik::reach` (`rust/core/src/attack/ik.rs`): its fixed-point hinge refinement loop currently stops after only 4 iterations (`for _ in 0..4`) with threshold `1e-10`. On a twisted arm whose rest bones sit slightly off the hinge plane, 4 iterations may not reach `< 1e-10` — or `ik::reach` may stop with residual `~3.6e-8` while the target is still well inside reach.
   - Check the final lines of `Solver::place_hand` (`rust/core/src/attack/mod.rs:771-800`):
     - When `blend == 1.0`, `natural.slerp(rot, blend).normalized()` can introduce tiny quaternion drift compared to `rot` used for the wrist target; if `blend == 1.0`, `rot` should stay exact (and if `blend < 1.0`, an exact final `ik::reach` for the final blended `rot` keeps the grip point on target).
     - Check whether applying forearm pronation/supination twist (`Quat::from_axis_angle(axis_w, twist).mul(w[fo].rotation())` + `set_world_rot(rig, locals, fo, ...)`) shifts `w[ha].t` by rounding error if `axis_w` is not exact or if a final exact `ik::reach` step is needed before forearm twist (and note that rotating `fo` about `w[ha].t - w[fo].t` must preserve `w[ha].t` to machine precision).
2. Fix the root cause cleanly in `rust/core/src/attack/ik.rs` and/or `rust/core/src/attack/mod.rs`:
   - Preserve the damped 2-cycle stabilization near full arm extension (no elbow flip or wrist kink > `PALM_WRIST_MAX_DEG` + 2 deg).
   - Ensure that whenever the target wrist is within reach (`sol.miss <= 1e-9`), the final wrist reach lands to `< 1e-10` (so `m.grip < 1e-9` holds cleanly with room to spare).
   - Keep `every_preset_builds_twice_the_same_and_parses` and all CLI goldens passing and deterministic.
   - Add a focused unit test in `rust/core/src/attack/mod.rs` (or `ik.rs`) covering exact convergence (`< 1e-9`) through the damped wrist pass on a twisted arm so it never regresses.

## Evidence of Done

- `gates/motion-2/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- All 227+ Rust tests pass with zero skips and zero warnings (`RUSTFLAGS="-D warnings"`), `detmath` passes, `grip_gate.sh` passes, and Python stdlib tests pass.
