# Goal: motion-8 — Pro bench held-out set, dev/held-out reporting, planted bad clip check, and no machine pin

UAL1+UAL2 are both the clip source and the pro benchmark. Extend `tools/bench/pro_bench.py` and `tools/bench/pro_fleet.sh` so the pro bench supports a held-out clip set (Mixamo clips), reports dev and held-out sets separately, verifies a planted bad clip fails, and never pins a fleet machine (`--on f`).

## Scope

- Edit ONLY `tools/bench/pro_bench.py` and `tools/bench/pro_fleet.sh`.
- Do NOT edit `rust/`, `tools/tests/`, `gates/motion-8/`, `docs/`, or any other file.
- Pure Python stdlib only (no external packages). Never hardcode owner paths, character names, or fleet machine names (`--on f`, etc.).

## Design

1. **Custom & held-out libraries in `tools/bench/pro_bench.py`**:
   - Add repeatable `--lib NAME=PATH.glb` to `run` and `calibrate` (merging into or overriding `LIBS`), plus auto-discovery of `mixamo` from `$FLEET_DATA_MOTION_MIXAMO_HELDOUT/mixamo_heldout.glb` or `$FORGE_BENCH/clips/mixamo/mixamo_heldout.glb` when present (so `--libs ual1,ual2,mixamo` works seamlessly with `--data motion-mixamo-heldout`).
   - Add `--heldout` (comma-separated library names, default `"mixamo"`) to `run` and store `"heldout": [...]` in `results.json` (defaulting to `["mixamo"]` when absent in older `results.json` files).
   - Update `report(out)` so for each target:
     - When clips come from both dev (`lib not in heldout`) and held-out (`lib in heldout`) sets, emit separate clearly labelled **dev** and **held-out** tables (plus per-library counts/names so e.g. `dev (ual1, ual2)` and `held-out (mixamo)` each have full numbers). When only one split is present, still clearly label it (`dev` or `held-out`).
     - Keep `recompare` and `rescore` working with custom/held-out libraries (reading library paths from `results.json` if stored, or falling back to `LIBS`).

2. **Planted bad clip check in `tools/bench/pro_bench.py`**:
   - Implement `check_planted_bad(out_dir=None) -> dict` and a CLI subcommand `pro_bench.py planted [--out DIR]`:
     - Uses the real `motionforge` CLI (`CLI`, respecting `MOTIONFORGE_BIN` env var) to generate a synthetic skinned walk GLB (`motionforge fixture-glb --naming def --walk --output ...`), scores it cleanly with `motion check --tracks` as the pro reference, then constructs a **planted bad clip** GLB by deliberately corrupting the clip (e.g. adding heavy root/foot skate and/or sinking feet `-0.12 m` under the floor and/or shifting pose angles by 45+ deg on the GLB animation accessors — pure stdlib `struct`/`json` GLB edit, no Blender needed!), scores the planted bad clip with `motion check --tracks`, and runs `compare(pro, planted_bad, "clean")`.
     - Verify `compare()` flags at least one real failure (`sink`, `skate`, `plants`, `hold`, or `pose`) on the planted bad clip, while an uncorrupted copy passes 0 failures (sanity control!).
     - Return `{"ok": bool, "failed_checks": [...], "control_failed_checks": [...], ...}` and exit 0 from `pro_bench.py planted` ONLY when the planted bad clip fails AND the clean control passes (exit 1 otherwise).

3. **No machine pin in `tools/bench/pro_fleet.sh`**:
   - Remove `--on f` and machine-specific comments from `tools/bench/pro_fleet.sh` (use capability flags like `--data motion-mixamo-heldout --cores 6`).
   - Automatically include held-out `mixamo` in `pro_bench.py run` when `$FLEET_DATA_MOTION_MIXAMO_HELDOUT/mixamo_heldout.glb` or `$FORGE_BENCH/clips/mixamo/mixamo_heldout.glb` is present, and run `pro_bench.py planted` as part of the fleet run.

## Evidence of Done

- `gates/motion-8/gate.sh` prints `GATE PASS` and exits 0 (verified via your `check` script).
- All 4 tests in `tools/tests/test_pro_bench.py` pass, `python3 tools/bench/pro_bench.py planted` passes against the real built `motionforge` binary, and all Python stdlib test suites pass.
