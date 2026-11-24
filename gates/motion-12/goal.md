# motion-12: Code-health sweep

Clean up Code Health items 2-5 in `motion` under
`tools/gate/carbon.sh` while keeping every CLI golden byte-identical and
all tests passing with zero warnings (`RUSTFLAGS="-D warnings"`).

## Scope

Only edit unguarded Rust source files (`rust/core/src/` excluding guarded
`readcam.rs` and `motioncheck.rs`, and `rust/cli/src/`). Do NOT touch
`rust/core/tests/`, `rust/cli/tests/`, `tests/fixtures/`, `readcam.rs`,
`motioncheck.rs`, `tools/bench/`, `tools/gate/`, or `gates/`.

## Requirements

1. **One distal-mitten predicate in `rust/core/src/attack/grip.rs`**:
   - Extract a single helper function (e.g. `fn is_distal_mitten(palm_len: f64, finger_len: f64) -> bool`)
     containing `finger_len > 1e-9 && palm_len > 2.0 * finger_len` and call it at
     both sites where `distal_mitten` was previously computed inline.
   - Leave `readcam.rs` completely untouched (it is guarded by `carbon.sh`).
2. **Replace all `partial_cmp` with `f64::total_cmp` in unguarded Rust sources**:
   - Replace every `.partial_cmp(...)` in `rust/core/src/` (excluding guarded
     `motioncheck.rs` and `readcam.rs`) and `rust/cli/src/` with `.total_cmp(...)`
     so a NaN from a malformed GLB never panics.
   - Keep sort directions and tie-breakers identical so CLI goldens remain
     byte-for-byte identical.
3. **Route `contact.rs`'s `DEF-forearm_twist` lookup through `crate::helpers`**:
   - Add a helper table / `pub fn owner_of(name: &str) -> Option<String>` (or
     equivalent helper function) in `rust/core/src/helpers.rs` that covers both
     proximal helpers (`table()`) and distal forearm twist helpers
     (`DEF-forearm_twist.L/R` -> `DEF-forearm.L/R`).
   - Replace the `DEF-forearm_twist` special case in `rust/core/src/contact.rs`
     (`owners()`) with the `crate::helpers` lookup so zero `DEF-forearm_twist`
     strings remain in `contact.rs`.
4. **Cache `map_rig` in `rust/core/src/helpers.rs`**:
   - Add a reusable per-rig struct (`pub struct Drivers` or `HelperMap` / `HelperSet` /
     `RigHelpers`) built once via `for_rig(rig)` (calling `humanoid::map_rig` in
     at most ONE place in `helpers.rs` when `DEF-spine` is absent, sharing that
     mapping across both proximal helpers and forearm twists!).
   - Expose methods to drive frames (`drive(&self, rig, locals)` and
     `drive_missing(&self, rig, locals)`) and update frame-loop callers
     (`animate.rs`, `clean.rs`, `contact.rs`, `giant.rs`, `stylize_body.rs`,
     `posetest.rs`, etc.) to construct the helper struct once per rig instead of
     calling `helpers::drive(rig, ...)` inside per-frame loops.
   - Keep existing `pub fn present`, `forearm_twists`, `driven_nodes`, `drive`,
     `drive_missing` signatures working (delegating through the cached struct)
     for backwards compatibility.
   - Add unit tests in `helpers.rs` (and/or `grip.rs`) verifying `owner_of` /
     cached driving matches unstandardized rigs identically and NaN-safe sorting
     does not panic.
5. **All existing tests and CLI goldens pass byte-identically**:
   - `bash tools/gate/carbon.sh` must PASS with 0 warnings and 0 golden diffs.
