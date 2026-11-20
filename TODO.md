# TODO — open follow-ups

Catch-up list as of 2026-11-19 (`motion` repo, command `motionforge`). Scope
and gates stay in `PLAN.md`; numbers and commands in `docs/measurements.md`,
`docs/pro-bench.md`, and `docs/log.md`.

## Done 2026-11-19 (CI green on Linux + macOS + Blender + Python)

- `motion-1` + `motion-4`: skin-based sword grip measure (`readcam::skin_grip_of`,
  `153822d`) and palm-pocket seating (`b296634`); single-finger hero `0.972` on
  all 4 presets, five-finger control `0.966`, 6 planted bad cases `0.000`
  (`tools/bench/grip_gate.sh`, fleet job `yhsgagzp`).
- `motion-2`: exact algebraic IK in `attack/ik.rs` + honest mitten test
  (`6d1d261`, CI `44644031463`).
- `motion-3`: guarded gate `tools/gate/carbon.sh`, verified 11/11 by
  `tools/gate/planted.sh` (`015c9de`, fleet job `xe3uhzr2`).
- `motion-6`: restored `FOREARM_ROLL_DEG = 80.0` after `weights check --motion`
  proved `105 deg` increased twistless forearm `D_VOLUME` +79% (`335da4b`,
  fleet job `q4zkw4ju`).
- `motion-13` (width refit): `width_refit.rs` + extended `contact.rs` (shins,
  upper arms, arm-vs-arm, source slack, `--refit-width`, `8413c42`, fleet
  audits `fphwu3tj`/`st2ake8d`, real-rig `8rygzvnw`).
- `motion-8`: pro bench held-out Mixamo set + `pro_bench.py planted` + machine-free
  `pro_fleet.sh` (`b2270cd`, `3cf2c1e`, fleet job `js23pxq6`).
- `motion-7`: giant pass `motionforge giant --scale k` (Froude `sqrt(k)` timing,
  root/rig `x k`, anticipation/hit-stop/settle, GLB `extras` events, `--mask
arms-only --pin-pelvis`, `42f6a74`, fleet audit `xxzx4eja`).
- `motion-9`: non-quadruped creature walks in `anybody::walk_stride` +
  `motionforge creature-clips --clips idle,walk,attack` for two-legged monsters
  and legless body-wave creatures (`8add1aa`, fleet audit `q7fh2jub`).
- `motion-12`: code-health sweep (`is_distal_mitten`, `total_cmp`,
  `helpers::owner_of`, cached `helpers::Drivers`, `ee69089`, fleet audit
  `mj9necpq`).
- `motion-11`: video-model terms in `docs/licensing.md` (Veo/Omni, Seedance,
  Kling, MiniMax) + `tools/manifest.py --check` source-row gate (`fe72a84`).

## Done 2026-10-03 (PRs #1–#5, all merged, CI green)

- #1 `detmath`: own sin/cos/asin/acos/exp, so outputs are
  byte-identical on macOS and Linux (platform libm differed by 1 ulp).
- #2 Fuzz fixes (deep-JSON stack overflow, quadratic non-ASCII parse,
  smoothing-flag hangs); Blender "Fix Balance" / "Max Lean"; CI on
  Linux + macOS.
- #3 Clip loads reject absurd magnitudes (|x| > 1e6 m, fps > 1e4,
  overflowing quats); CI runs the headless Blender tests (5.2.1).
- #4 Stylizer passes root travel through unchanged (a stylized walk's
  root used to freeze, sprint, overshoot and walk backwards).
- #5 AutoPose inputs are root-relative (weights tag
  `effector-pos-mask+lengths/root-relative`); old weights still work.

## Owner (needs you)

- [ ] Run `python/autopose/train.py` once locally: torch is not in CI,
      so the #5 training path is only compile-checked.
- [ ] Retrain AutoPose to get root-relative weights (ideally after the
      heading item below, to retrain only once).
- [ ] Look at a stylized walk in Blender after #4 (P2 gate: owner
      prefers stylized on 3 test clips).
- [ ] P1 gate: a Mixamo clip on `biped_hero` in the Bevy game with no foot slide.

## Creature motion follow-ups (2026-10-29, `docs/creature.md`)

- [ ] Real video through the pipeline: a paid clip (Seedance/Kling/
      MiniMax, release batch) of the wolf start frame with the strict
      prompt; measure the consistency check and the solve on it. The
      detector has only seen Blender renders plus degradation.
- [ ] Wan 2.2 TI2V-5B gave a clean but near-static clip at 20 steps /
      guidance 5 (`video_wan.py`); try 40 steps, guidance 7-9, a prompt
      that names a bigger move, or the 14B A14B model on a bigger card.
- [ ] Per-axis (swing/twist) joint limits (v2 limits format) so a knee
      cannot bend sideways; today's caps are scalar.
- [ ] Sideways-only disagreement between the two views is invisible to
      the consistency check (orthogonal cameras share only the height);
      a third camera or a learned prior would catch it.
- [ ] Occlusion-aware training targets (ray-cast visibility per joint)
      to feed `--min-conf`; today every joint is a target.

## Next code tasks (agent-ready)

- [ ] Motion check / stylizer follow-ups (2026-11-12, `docs/motion-check.md`,
      `docs/stylizer.md`): skate for in-place locomotion needs the game's
      travel speed added; a jitter measure that does not rank fast moves
      as rough; body-mode exaggeration should stop at the contact proxies
      and take keys from the weapon hand's extremes too.

- [x] Contact pass v2 (`docs/contact.md`, `docs/width-refit.md`): upper arm
      (armpit), shin proxies, arm-vs-arm contact with free-hand weapon priority,
      per-frame source slack, and width-first retargeting (landed 2026-11-19 in
      `8413c42`).
- [ ] Corrective shapes if the owner wants the deformation gate to pass
      real clips (`docs/adapters.md` "Shoulder helpers"): pose-space
      targets baked as glTF morph weights per clip, and weightforge
      evaluating morph targets.

- [ ] GLB adapters v2 (`docs/adapters.md` limits): foot pinning in
      `animate` when proportions differ; quadruped ROM poses and clips;
      CUBICSPLINE keys sampled as splines.

- [ ] **AutoPose heading invariance** (recommended next). Inputs are
      root-relative but not facing-relative: the same pose facing
      another way gives different inputs. Rotate effector offsets by
      the inverse of the root's yaw (about +Z) in
      `python/autopose/dataset.py::build_input` and
      `rust/core/src/autopose.rs::infer`; Blender `export_effectors`
      must also send the root's yaw/rotation. New weights `input` tag
      (keep both older tags working); open question: how the predicted
      root rotation is re-expressed. Mirror #5's tests (invariance,
      Python<->Rust cross-check golden, headless Blender check).
- [ ] **Retarget with unmapped intermediate bones** (design call).
      Transfer copies each bone's local motion (same as rigforge's
      reference), so an unmapped source bone's motion is dropped, not
      folded into the next mapped bone. Property check: full map
      matches world rotation deltas to 7e-16; skipping the middle bone
      of a 3-chain differs by up to 0.93. Only matters for targets
      with fewer bones than the source.
- [ ] Stylizer: bones whose angle never exceeds `--angle-threshold`
      keep only endpoint keys, so subtle motion (hip sway) is replaced
      by one eased transition. By design; revisit if it reads flat.
- [ ] Stylizer: exaggerating an extreme near 180 deg can wrap past pi
      (slerp extrapolation); rare for joints, unguarded.
- [ ] `tools/make_fixtures.py` uses Python's platform libm: regenerating
      on Linux shifts 14 numbers in `walk_src.json` by <=1.2e-16.
      Commit only intended files (note in its header).
- [ ] Windows (only if it becomes a target): extension's build-tree
      binary lookup lacks `.exe`; goldens need `.gitattributes`
      (`-text`) against CRLF conversion.
