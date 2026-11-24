# Goal: motion-13 — Refit clips to a body's width & extend contact pass

Read `docs/width-refit.md` in full before editing. Clips authored on a slim clip-library body cut into a wider target body's torso, thighs, shins, and cross arm-through-arm when retargeted with plain FK + the existing partial contact pass.

Implement `motion-13` as designed in `docs/width-refit.md`: width-first retargeting per body plus the extended contact pass (shins, upper arms, arm-vs-arm, and source-body slack) wired into `animate` (`--refit-width`, default on for humanoids; `--no-refit-width` to disable).

## Scope

- Edit ONLY:
  - `rust/core/src/contact/` (originally `contact.rs`)
  - `rust/core/src/animate.rs`
  - `rust/core/src/width_refit.rs` (optional new module, wired from `rust/core/src/lib.rs`)
  - `rust/core/src/lib.rs` (only if adding `pub mod width_refit;`)
  - `rust/cli/src/main.rs` (register `--refit-width` and `--no-refit-width` boolean flags + help text)
  - `rust/cli/src/glb_cmds.rs` (wire `--refit-width` / `--no-refit-width` in `contact_opts` / `cmd_animate` / `adapter_animate`)
- Do NOT edit `rust/core/tests/`, `rust/cli/tests/`, `tests/fixtures/`, `rust/core/src/readcam.rs`, `rust/core/src/motioncheck.rs`, `tools/bench/`, `tools/gate/`, `gates/motion-13/`, `Cargo.toml`, `Cargo.lock`, or docs.
- Keep zero external crates (`--locked --offline`), zero warnings (`RUSTFLAGS="-D warnings"`), and byte-determinism across macOS arm64 and Linux x86_64 (`crate::detmath` only — never platform `f64` libm methods).
- Keep all existing CLI golden tests (`cli_contract.rs`) and `contact.rs` tests passing with zero drift.

## Design (from `docs/width-refit.md`)

1. **New proxies in `Body::build` (`rust/core/src/contact/body.rs`)**:
   - Add **shins** (`shin.L`, `shin.R`, from `DEF-shin.{side}` to `DEF-foot.{side}`) to `static_parts` (and body surface/mesh checks), sized from the skinned rest mesh just like thighs. Legs never move during contact resolution.
   - Add **upper arms** (`upper_arm.L`, `upper_arm.R`, from `DEF-upper_arm.{side}` to `DEF-forearm.{side}`) on `Arm` (and include them in `Body::to_json`), so `Body::to_json`'s `capsules` array contains `"shin.L"`, `"shin.R"`, `"upper_arm.L"`, `"upper_arm.R"`.
   - Keep `Options::default()` backwards compatible (add `pub refit_width: bool` defaulting to `true`; all existing callers use `..Options::default()`).

2. **Step 1 — Width first, as a retarget (in `animate::retarget_with`, after `pin_feet` and before `contact`)**:
   - When `contact` is `Some(opts)` and `opts.refit_width` is true, build the source rig's `Body` alongside the target rig's `Body` (gracefully skipping width-refit if the source has no mesh/proxies).
   - Measure source and target width profiles from their proxies (torso lozenge half-widths at shoulders/waist and hip/thigh capsule outer edges, interpolated by normalized height).
   - If the source and target have effectively the same width (ratio within ~1e-4 at every height), leave the clip **completely unchanged** (zero shift).
   - Otherwise, per frame and per arm: express the source wrist position in the posed body's local frame `(side, up, forward)` (centered on the torso/spine centerline). When the source wrist lies inside the source's width envelope plus a margin (e.g. a hand across or beside the torso/hips), scale its side offset (and optionally forward clearance where torso depth grew) smoothly by target-width / source-width at that height; outside the envelope + blend band (arms out wide), blend smoothly to 0 so wide-arm frames are left completely unchanged!
   - Re-solve with `contact::solve_arm` (keeping shoulder, elbow bend plane, and hand orientation/relative rotation), preserving clip timing (peak wrist speed frame unchanged) and key count.

3. **Step 2 — Extended contact clean-up (`contact/`)**:
   - **Source-body slack**: when `retarget_with` supplies the source `Body`, source `Rig`, and sampled source frames (`src_posed`), measure per-frame penetration depths on the source body using the same proxies (arm-vs-static and arm-vs-arm) and add that depth as per-frame slack on the target so faults already present on the source body itself are left alone (a same-width target with a source-body fault comes out unchanged!).
   - **Arm against arm**: test the two arms' `forearm` and `palm` (and `upper_arm`) capsules against each other per frame (respecting rest slack and source-body slack).
     - When neither arm holds a weapon (`!use_weapon` on both): push both wrists apart along the deepest arm-vs-arm contact normal, half each (smoothed with the same `ramp` smoothstep envelope and 12% height cap + rate-limit!).
     - When one hand holds a weapon (`use_weapon` on one arm, e.g. `R`, and the other arm `L` is free): move the **free hand first** (full separation on the free hand up to its cap; only move the weapon hand if the free hand alone cannot clear it), so the blade's line stays steady.
   - **Arm against legs (thighs + shins)**: arm moves, legs NEVER move.
   - **Caps & ramp**: keep the existing `0.12 * body.height` wrist cap and `opts.ramp` (`0.15 s`) smoothstep envelope so frames more than `ramp` away from any contact remain untouched and wrist peak-speed frame is preserved.

4. **CLI (`rust/cli/src/main.rs` and `rust/cli/src/glb_cmds.rs`)**:
   - Accept `--refit-width` (default on) and `--no-refit-width` boolean flags in `main.rs` flag parser and `glb_cmds.rs` `contact_opts` (error if both are passed together).

## Evidence of Done

- `gates/motion-13/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- All 5 tests in `rust/core/tests/width_refit.rs` pass (including the reachable crouched-leg `arm_vs_shin_clears_without_moving_leg` test), `tools/gate/carbon.sh` passes (all Rust tests, CLI goldens, fmt, zero warnings, Python suites), and `tools/bench/width_refit_check.sh` prints `WIDTH REFIT CHECK PASS`.
