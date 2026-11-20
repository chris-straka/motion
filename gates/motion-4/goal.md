# Goal: motion-4 — Close the distal-mitten hand on the hilt (>= 0.9 under motion-1)

Update the grip solver in `rust/core/src/attack/grip.rs` so a distal-tip mitten rig (`palm_len > 2.0 * finger_len`) seats the sword handle inside the palm pocket (surrounded by hand skin, `skin_wrap >= 0.85`) and curls onto the hilt cleanly (`curl_deg` ~ 34 deg, scoring `>= 0.90` on all four presets under `motion-1`'s unchanged skin-based grip measure), while leaving five-finger and standard-mitten hands untouched.

## Scope

- Edit ONLY `rust/core/src/attack/grip.rs`.
- Do NOT edit `rust/core/src/readcam.rs` (the measure MUST stay untouched), `rust/core/tests/`, `rust/cli/`, `tools/bench/grip_gate.sh`, `gates/motion-4/`, `Cargo.toml`, `Cargo.lock`, or docs.
- Keep zero external crates (`--locked`), zero compiler warnings (`RUSTFLAGS="-D warnings"`), and strict byte-determinism across macOS arm64 and Linux x86_64 (`detmath` only).

## Design

In `rust/core/src/attack/grip.rs`:

1. **Why distal mittens currently score ~0.74 under `motion-1`**:
   - In `palm_hold`, the distal-mitten branch (`finger_len > 1e-9 && palm_len > 2.0 * finger_len`) places `base = hand_head + palm_vec.scale(DISTAL_MITTEN_PALM_SHARE)` (`0.55`), and then adds `palm.scale((HANDLE_R + FINGER_R) * unit)` (`+2.6 cm` along `palm`).
   - On a five-finger hand, the bones sit along the dorsal/knuckle line and articulated fingers wrap around the palm side of the handle; on a distal-tip mitten rig, the cupped hand mesh is already centered around the palm bone axis, so pushing `+2.6 cm` along `palm` shoves the handle out to the outer palm edge (`skin_wrap ~ 0.71–0.75`, leaving an open gap on the palm side).
   - Meanwhile `DISTAL_MITTEN_CURL = 0.55` only curls the finger chain to ~26.0 deg at `close = 0.9`, capping the curl component at `26.0 / 35.0 = 0.743`.

2. **Fix in `rust/core/src/attack/grip.rs`**:
   - Define `pub const DISTAL_MITTEN_PALM_OFFSET: f64 = 0.008;` (half the handle radius, `0.5 * HANDLE_R`, seating the handle cleanly inside the cupped palm pocket instead of shoving it `HANDLE_R + FINGER_R = 2.6 cm` out to the palm face).
   - In `palm_hold`'s single-chain branch, use `DISTAL_MITTEN_PALM_OFFSET` for the palm-normal offset when `distal_mitten` is true (keeping `(HANDLE_R + FINGER_R)` unchanged for standard mittens and five-finger hands!).
   - Update `DISTAL_MITTEN_CURL` from `0.55` to `0.72` (so at `close = 0.9`, mean curl is `~34.0 deg`, matching five-finger hands' `~33.8 deg` and staying gently below 36 deg while scoring `0.972 >= 0.90`).
   - Keep the doc comment on `DISTAL_MITTEN_PALM_SHARE` / `DISTAL_MITTEN_PALM_OFFSET` / `DISTAL_MITTEN_CURL` accurate and concise.
   - Do NOT change five-finger `palm_hold` or `fingers` behavior, and do NOT change standard-mitten (`palm_len <= 2.0 * finger_len`) behavior.
   - Add a unit test in `rust/core/src/attack/grip.rs` checking that `palm_hold` seats a distal mitten's socket inside the palm pocket (offset by `DISTAL_MITTEN_PALM_OFFSET`, not `HANDLE_R + FINGER_R`) and curls past 31.5 deg (while staying < 36 deg).

## Evidence of Done

- `gates/motion-4/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- `rust/core/src/readcam.rs` has empty `git diff`, all 236+ Rust tests pass with zero warnings, and `tools/bench/grip_gate.sh` passes.
