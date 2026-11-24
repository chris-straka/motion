# motion (command `motionforge`) — agent notes

Folder and GitHub repo: `motion` (`chris-straka/motion`); CLI command and
Blender extension: `motionforge`.

One rules file only: this `AGENTS.md` is it.

Game-animation sidekick for Bevy games: retarget assist, motion stylizer,
physics pass, and ML AutoPose: an MIT-licensed Rust core + CLI, driven as a
subprocess over files by a GPL Blender extension. Training lives in
`python/` (PyTorch, local).

Scope and gates: read [PLAN.md](PLAN.md) first; documentation index:
[docs/README.md](docs/README.md) (active specs in `docs/`, historical notes
in `docs/archive/`). Every claim in docs carries a measured number and the
command behind it ([docs/measurements.md](docs/measurements.md)).

## Standing rules

- The owner's game assets, rigs, and any mocap (CMU, Mixamo,
  recordings) must never be committed — not even file names or paths
  in tracked files. Datasets enter this repo as manifests only
  (`dataset manifest` = clip id + source + license + checksum).
  `data/`, `models/`, and `bench/` are gitignored.
- Licensing is the hard constraint (see `docs/licensing.md`):
  commercial-safe sources only. CMU mocap is OK (terms verified
  2026-10-01, recorded in `docs/licensing.md`); owner's own clips OK.
  BLOCKED: AMASS / SMPL-based data (non-commercial), anything
  research-only, Hunyuan output (territory exclusion). Mixamo clips
  may be retargeted for in-game use but are excluded from training
  data by default — ask the owner before loosening any gate.
- Determinism: the Rust core is byte-deterministic (same input bytes
  -> same output bytes). No hash-map iteration in output paths, no
  timestamps or temp paths in outputs, no unseeded RNG, and no
  platform libm: call `motion_core::detmath::{sin, cos, sin_cos, asin,
acos, exp}` instead of the `f64` methods (Apple's and glibc's libm
  differ in the last ulp; a unit test rejects `.sin()` & co. in core
  code). `sqrt`, `floor`, `powi`, `to_degrees` are exact and fine. CLI
  contract goldens pin this; regenerate deliberately and review the
  diff.
- Never launch GUI binaries without explicit user approval. Headless
  Blender (`--background --factory-startup`) is fine.
- Blender binary: `/Applications/Blender.app/Contents/MacOS/Blender` on macOS, or `blender` on `PATH`.

## Build

- `cd rust && cargo build --locked --release` produces
  `rust/target/release/motionforge`. Zero external crates (offline
  build); keep it that way unless the maintainer approves a dependency.
- Python training needs torch (not vendored):
  `pip install -r python/requirements.txt`. Manifest tooling is
  stdlib-only.

## Checks

- `cd rust && cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test --locked --release`
  must be green with zero warnings. CI (`.github/workflows/ci.yml`, green on
  `main`) runs it on Linux x86_64 and macOS arm64 — including
  `python3 tools/bench/pro_bench.py planted` — so goldens must be
  byte-identical on both.
- Guarded Carbon/Neon gate: `bash tools/gate/carbon.sh` verifies scope
  (blocking edits to tests, fixtures, `readcam.rs`, `motioncheck.rs`,
  `tools/bench/`, `tools/gate/`), `#[test]`/`#[ignore` counts, formatting,
  `-D warnings` offline release build, full Rust & CLI golden tests, and Python
  stdlib tests (`tools/gate/planted.sh` verifies the guard).
- CLI contract goldens (`rust/cli/tests/` + `tests/fixtures/`): after
  an intended CLI output change, regenerate with
  `UPDATE_GOLDENS=1 cargo test --locked --release -p motionforge`
  and review the diff before committing.
- Blender smoke (headless, procedural armature, no gamerig needed):
  `blender --background --factory-startup --python blender/tests/test_headless.py`.
  CI runs it (and `blender.tests.test_batch`, which takes `$BLENDER`)
  on Linux with Blender 5.2.1.
  With gamerig + a real clip, the live checks are in
  `docs/measurements.md`.
- Manifest gate: `python3 tools/manifest.py --check data/manifest.json`
  rejects blocked licenses and any clip whose `source` has no documented row
  in `docs/licensing.md`; every training run records its manifest checksum in
  the run log.

## Layout

- `rust/core/` engine (`motion_core` lib), `rust/cli/` the
  `motionforge` binary + CLI contract test.
- `blender/motionforge/` = Blender extension (GPL, talks to the CLI
  as a subprocess over clip JSON). `blender/tests/` = headless smoke.
- `python/autopose/` = training (torch) + weights export to the
  Rust-readable format. `tools/` = stdlib-only helpers (fixtures,
  manifest checks).
- `tests/fixtures/` = procedural synthetic clips/skeletons (generated,
  committed; these are test vectors, not mocap).
- `docs/` = format + feature docs. `data/`, `models/` = gitignored
  local-only inputs/outputs (never committed).

## Integration notes

- Pairs with Rigify-compatible humanoid skeletons (`biped_hero` /
  `quadruped_stalker` deform bones); the Rust retarget module implements
  rotation transfer cleanly onto clip JSON without importing external rigs.
- Deform-only glTF export targets any Bevy/Rust game runtime (standardized
  skeleton + baked helper bone keys requiring zero runtime constraint code).
