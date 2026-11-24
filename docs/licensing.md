# Licensing: what may train AutoPose, what may ship

Hard constraint from [PLAN.md](../PLAN.md), verified at build time (2026-10-01;
video-model terms verified 2026-11-19). The manifest gate (`tools/manifest.py --check`)
enforces both the license allowlist and that every clip's `source` has a
documented row in this file; shipping provenance is tracked per asset in the
target game project's asset log.

## Training data

| Source ID       | Source                            | Status                        | Terms (as verified 2026-10-01)                                                                                                                                                                                                                                                                                                                                                |
| --------------- | --------------------------------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cmu`           | CMU Graphics Lab mocap DB         | OK                            | "This data is free for use in research and commercial projects worldwide." Acknowledgment requested: cite `http://mocap.cs.cmu.edu` + "created with funding from NSF EIA-0196217". Not a formal CC0 grant — record the terms text as it stood on the download date in the manifest's `terms_snapshot` field. Fetched via the `una-dinosauria/cmu-mocap` BVH mirror or direct. |
| `owner`         | Owner's own animations/recordings | OK                            | Owner-authored; no third-party terms.                                                                                                                                                                                                                                                                                                                                         |
| `amass`         | AMASS / anything SMPL-based       | BLOCKED                       | SMPL is non-commercial without a paid license; HumanML3D inherits it.                                                                                                                                                                                                                                                                                                         |
| `research-only` | Research-only / NC-licensed data  | BLOCKED                       | Any license with a non-commercial or research-only clause.                                                                                                                                                                                                                                                                                                                    |
| `mixamo`        | Mixamo clips                      | Default EXCLUDE from training | Royalty-free in games (retargeting for in-game use and held-out benchmarking is fine), but training on it needs a terms check first. Pass `--allow-mixamo-training` only after the owner approves.                                                                                                                                                                            |
| `hunyuan`       | Hunyuan 3D output                 | BLOCKED everywhere            | Territory exclusion (EU/UK/South Korea); reference/placeholder only, never shipped or trained on.                                                                                                                                                                                                                                                                             |

## Video generation models (motion extraction / reference)

No clip extracted from AI-generated video may enter training or ship in a game
before its provider terms are read and recorded here. Every row below records
the terms URL, the verification date, and the verdicts for both ML training and
commercial shipping.

| Source ID                 | Model / Service                                   | Terms URL                                    | Read date  | Training verdict | Shipping verdict     | Notes                                                                                                                                                                                                                                                                                                                                          |
| ------------------------- | ------------------------------------------------- | -------------------------------------------- | ---------- | ---------------- | -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `veo`, `omni`, `veo-omni` | Google Veo / Gemini Omni (Gemini API & Vertex AI) | https://ai.google.dev/gemini-api/terms       | 2026-11-19 | BLOCKED          | PAID-GA-ONLY         | Gemini API Additional Terms bar using outputs to develop, train, or distill competing ML models. Google claims no ownership over outputs; GA paid/Vertex AI tiers permit commercial use, while Preview/Pre-GA models and unpaid tiers are barred from commercial deployment. Confirm GA status and paid tier before shipping extracted motion. |
| `seedance`                | ByteDance Seedance (BytePlus ModelArk)            | https://www.byteplus.com/en/terms-of-service | 2026-11-19 | BLOCKED          | PAID-ENTERPRISE-ONLY | Commercial use requires paid BytePlus ModelArk enterprise/resource-pack terms and custom/authorized inputs only; free/trial credits and platform reference assets are evaluation-only. Output may not be used to train competing models; preserve C2PA provenance where required.                                                              |
| `kling`                   | Kuaishou Kling AI                                 | https://klingai.com/terms-of-service         | 2026-11-19 | BLOCKED          | PAID-ONLY            | Terms of Paid Service grant paid subscribers commercial rights to watermark-free generated outputs; free-tier outputs are non-commercial. Explicitly bars using outputs to develop or train competitive AI models/services; user clears any third-party IP.                                                                                    |
| `minimax`                 | MiniMax Hailuo AI                                 | https://hailuoai.video/terms                 | 2026-11-19 | BLOCKED          | PAID-ONLY            | Paid subscription plans grant commercial use rights and watermark-free downloads; free/standard web use is personal non-commercial only. No grant for ML model training/distillation; retain provenance metadata where required and avoid third-party IP.                                                                                      |

## Shipping (game content)

- Tripo textures/meshes: shipping assets only while subscribed (paid plan =
  commercial rights); free plan is CC BY non-commercial.
- Suno: commercial rights only for songs made while subscribed (Pro/Premier).
- Steam AI disclosure + provenance log: record one row per shipped asset in
  the target project's provenance log.

## Tool licenses

This repo (`motion`, command `motionforge`): MIT (`rust/`, `python/`, `tools/`,
`tests/`, `docs/`), GPL-3.0-or-later (`blender/`). Content made with the tools
is unaffected. `gamerig` (formerly `rigforge`) is GPL (tools only, same deal).
