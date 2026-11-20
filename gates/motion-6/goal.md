# Goal: motion-6 — Restore `FOREARM_ROLL_DEG = 80.0` after forearm candy-wrap measurement

Empirical `weights check --motion` measurements on twistless character rigs across all four attack presets (`cleave`, `backcut`, `sweep`, `thrust`) showed that widening `FOREARM_ROLL_DEG` from `80.0` to `105.0` increases forearm candy-wrapping (`D_VOLUME` collapse area on `right forearm, front` jumped +79%, from 84.7 cm² to 151.4 cm²; total forearm non-through tears rose from 796.9 cm² to 874.7 cm²; LBS 50/50 elbow ring area collapses to `cos²(52.5°) = 37.1% < 50%` at 105° vs `cos²(40°) = 58.7%` at 80°).

Restore `FOREARM_ROLL_DEG` to `80.0` in `rust/core/src/attack/mod.rs`.

## Scope

- Edit ONLY `rust/core/src/attack/mod.rs`.
- Do NOT edit `rust/core/tests/`, `rust/cli/`, `tests/fixtures/`, `rust/core/src/readcam.rs`, `rust/core/src/motioncheck.rs`, `tools/bench/`, `tools/gate/`, `gates/motion-6/`, `Cargo.toml`, `Cargo.lock`, or docs.
- Keep zero external crates (`--locked --offline`), zero warnings (`RUSTFLAGS="-D warnings"`), and byte-determinism (`detmath` only).

## Design

In `rust/core/src/attack/mod.rs`:
1. Restore `pub const FOREARM_ROLL_DEG: f64 = 80.0;` (with an accurate doc comment explaining that 80° keeps LBS forearm/elbow twist above the 50% volume-collapse threshold on rigs without forearm twist bones, while rigs with a forearm twist helper already split roll 50/50 across forearm + twist bone).
2. Add a unit test inside `rust/core/src/attack/mod.rs` confirming `FOREARM_ROLL_DEG == 80.0` and that `place_hand` caps forearm roll at 80° on a twistless rig.

## Evidence of Done

- `gates/motion-6/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- Both tests in `rust/core/tests/motion_6_forearm.rs` pass, `tools/gate/carbon.sh` passes (all Rust tests, CLI goldens, fmt, zero warnings, Python suites), and `tools/bench/forearm_wrap_check.sh` prints `FOREARM WRAP CHECK PASS`.
