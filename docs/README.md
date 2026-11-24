# motionforge documentation

Active design specs, contracts, benchmarks, and archived R&D logs for `motion` (`motionforge`).

## Active Documentation

### Overview & Architecture
- [README](../README.md) — Quick start, CLI overview, build instructions, and verification commands.
- [Build Plan](../PLAN.md) — Pipeline architecture, phases, licensing policy, and subsystem roadmap.
- [Agent Notes](../AGENTS.md) — Determinism rules, repository layout, and testing conventions.

### Formats, Adapters & Licensing
- [Clip & Skeleton Format](clip-format.md) — JSON schemas for clips, skeletons, retarget maps, AutoPose effectors, and weights.
- [GLB Rig Adapters](adapters.md) — `standardize`, `pose-test`, and `animate` asset-pipeline contracts and humanoid mapping.
- [Joint Limits](limits.md) — Target joint swing/twist bounds and how adapters/solvers check them.
- [Licensing Policy](licensing.md) — Approved/blocked mocap and AI-video data sources enforced by the manifest gate.

### Retargeting & Motion Cleanup
- [Retargeting](retarget.md) — Bone mapping, rotation transfer math, and foot pinning.
- [Contact Pass & Hand Sockets](contact.md) — Proxy fitting, two-bone arm IK clearance, weapon sockets, and winding-number checks ([`rust/core/src/contact/`](../rust/core/src/contact/mod.rs)).
- [Width-First Arm Refit](width-refit.md) — Per-body width scaling and extended limb/shin/arm-arm clearance (`--refit-width`).
- [Clip Clean-Up](clean.md) — Automated ground alignment, floor penetration recovery, and foot-plant locking (`motionforge clean`).
- [Motion Stylizer](stylizer.md) — Key reduction, exaggeration, anticipation/overshoot, and body-mode timing (`motionforge stylize`).

### Pose Authoring, Readability & Attacks
- [AutoPose](autopose.md) — Root-relative ML pose completion from sparse effectors (`motionforge autopose`).
- [Keypose Fitting](keypose.md) — Two-view drawn-pose solver (`motionforge keypose`).
- [Attack Authoring](attack.md) — Sword-path-first attack solver (`motionforge attack`) and grip placement.
- [Motion Check & Camera Readability](motion-check.md) — Automated clip quality checks, silhouette readability, and grip scoring (`motionforge motion check`).
- [Physics Pass](physics.md) — Balance, ballistic arcs, and angular momentum checks (`motionforge physics-check` / `physics-fix`).

### Creature Pipeline & Auto-Rigging
- [Creature Motion Pipeline](creature.md) — Creature profiling, pose sweeps, camera alignment, two-view solve, gaits, and non-quadruped clips.
- [Multi-Seed Auto-Rigging](tripo-rig.md) — Headless mesh cleanup, facing normalization, and multi-seed rig selection (`tools/tripo_rig.py`).

### Benchmarks & Measurements
- [Pro Clip Benchmark](pro-bench.md) — Calibration and held-out benchmark evaluation against CC0 motion libraries (`tools/bench/pro_bench.py`).
- [Measurements](measurements.md) — Recorded timings, error bounds, and benchmark figures with reproducing commands.

## Archived R&D / Spike Logs

Historical task lists, chronological R&D logs, and point-in-time evaluations live under `docs/archive/` for reference:

- [Historical TODO List](archive/TODO.md) — Early sprint catch-up list and completed milestone log.
- [Chronological R&D Log](archive/log.md) — Iteration-by-iteration benchmark runs, A/B notes, and ablation tables.
- [Code & Clip Review (2026-11-13)](archive/review-2026-11-13.md) — Point-in-time audit of clip-JSON vs GLB gaps and defect triage.
- [Auto-Rigger Comparison](archive/rig-rivals.md) — Head-to-head evaluation against external rigging models.
- [Creature Route Spike Log](archive/creature-route.md) — Early design ledger for the multi-tool creature rigging route (superseded by [Creature Motion Pipeline](creature.md)).
