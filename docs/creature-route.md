# The creature route: from a Tripo creature to an HLH monster

The plan for `tools/creature.sh NAME MODEL OUT` (the goal is
`~/Games/MONSTERS.md`), written before any of it was built: the route as it
stands on 2026-11-18, which body plans get clips today, every hard-coded
host and path, and what the command adds.

## The route as it stands (2026-11-18)

`python3 tools/tripo_rig.py MODEL OUT --name NAME --kind creature --class monster`

1. **Clean** (`tools/tripo_to_glb.py`, headless Blender): one mesh on the
   ground at the origin, facing glTF +Z (`tools/facing.py`), holes with a
   rim under 8% of the model's largest side filled, triangles cut to the
   class budget with Blender's decimate (monster 18,000, a little under
   rfcheck's 20,000 per mesh), textures to PNG at 1024 at most.
2. **Hand-off.** When skintokens' `tools/creature_route.py` and its
   `creature-rig` binary are found, `tripo_rig.py` hands the cleaned mesh
   to it and returns its exit code. Otherwise (or with
   `--creature-route predicted`) it runs the older path: `skintokens rig`
   per seed, `motionforge standardize`, the gait, `weights fix`/`check`,
   rfcheck.
3. **skintokens' creature route** (`creature_route.py`):
   - `creature-rig rig`: the body's parts read from its volume (legs, arms,
     neck and head, tail, wings), a skeleton along them
     (`DEF-<part>.<nnn>.<side>`), weights through the body at band widths
     1.6, 1.0 and 2.5, and the body's range of motion as a pose set
     (`--rom`). It names the body: four-legged, two-legged, winged,
     many-limbed or legless.
   - Clips for a four-legged body only (below).
   - `weights fix` chooses per region among the three bands, the model's
     own weights for this skeleton (`skintokens skin`) and its repairs;
     `weights check --poses ROM` is the bend gate.
   - `creature-rig check` (the skeleton check: every limb has a chain,
     bones inside the body, sides match) and `creature-bench anatomy` (the
     no-artist check: `B_SWAPPED`, `B_NO_LEG`, `B_NO_HEAD`, `B_NO_ARMS`,
     `B_LOPSIDED`, `B_KIND`, `B_OFF_CENTRE`).
   - The model's own rig, `skintokens rig --class quadruped` on seeds
     1, 2, 3, standardized, judged the same way.
   - **The picker** (`tools/pick_score.py`, `pick_candidate`) keeps one of
     the two: anatomy first, then articulated limb pairs, then the learned
     rig unless the body-read rig tears more than 8 points less or clears
     90 by 3 where the learned one doesn't. It agrees with the artists on
     12 of 12 benchmark animals, measured on the same 12 it was tuned on
     (`skintokens/docs/creature-rig-log.md`): not a held-out number.
   - `rfcheck --class monster` on the kept rig; `P_*` (budget) codes are
     left out of the verdict.
   - Exit 0 when the kept rig passes the bend gate, the skeleton check and
     rfcheck. Out: `<name>.glb`, `<name>.rom.ron`, `<name>.sheet.png`,
     `<name>.skeleton.png`, `report.json`.
4. **The blind films** are not part of the route.
   `skintokens/tools/creature_blind.py make` builds same-frame A/B pair
   sheets (a can't-tell control, a planted swapped-limb rig from
   `creature-bench plant`, the key beside the judged folder) with
   `tools/bench/render_gamecam.py`, at its own camera (45 degrees of
   vertical view, 18.3 degrees down, 55 degrees round to the front three
   quarters). HLH's follow camera has 75 degrees of vertical view (motion
   `docs/motion-check.md`, `readcam.rs`), so that camera is not the game's.

### What it does not check today

- A missing `creature-bench` binary skips the anatomy check with no word,
  and `pick_score.anatomy_summary(None)` counts as a pass (audit
  2026-11-18).
- Triangles: nothing counts them from the finished GLB. rfcheck's budget
  overrun (`P_TRIS`) is a warning and is left out of the verdict.
- Weight sums and influences per vertex are not counted by the route
  (rfcheck checks `max_influences` per its contract only).
- Nothing says a creature has an idle or walk and an attack; a creature
  with no clips passes.
- Nothing runs the route twice to see that it repeats.
- Nothing plants a bad input to show a check can fail.
- Nothing puts the creature in HLH.

## Body plans and their clips today

Clips are made only by `motionforge creature-gait`
(`rust/core/src/creature/gait.rs`), and `gait::parts` refuses any profile
that does not have exactly four feet on the ground.

| Body (as `creature-rig` reads it) | Clips today | HLH plays |
| --- | --- | --- |
| four-legged | `walk`, `trot` (feet planted), `attack` (a lunge, forward kinematics) | walk, trot, attack; no idle (stands in the walk's first frame) |
| four-legged, but `creature-profile` drops a joint it takes for a stray | none | stands at rest |
| two-legged (bipeds, hunched long-armed monsters) | none | stands at rest |
| winged | none | stands at rest |
| many-limbed (more than four legs, a hand on its fingers) | none | stands at rest |
| legless (fish, cloaked, formless) | none | stands at rest |

On the predicted rig, the gait is tried only when the body-read rig was
four-legged. On the owner's 34 creatures (route8, 2026-11-13) 13 got clips,
all four-legged; furrow beast and husk, four-legged, got none.

HLH's creature path (`bevy/src/rigged.rs`, `creature_clips`) plays a
creature's own clips by words in their names: `idle`/`breath`, `walk`,
`trot`/`run`/`gallop`, `attack`/`lunge`/`bite`, `grab`/`reach`; `--rig
gen/<kind>.glb=<clip>@x,y,z` stands a rigged model in a level playing a
named clip.

So gate 5 fails today for every body that is not a clean four-legged one,
and has no idle anywhere. The command treats a body plan with no clip maker
as a fail with that reason.

## Hard-coded hosts and paths

In the route's code:

- `tools/tripo_rig.py`: `/Applications/Blender.app/Contents/MacOS/Blender`
  tried before `PATH`; the weights fallback is
  `../weightforge/rust/target/release/weights`, a folder renamed to
  `meshweights` on 2026-11-17, so it no longer resolves.
- `skintokens/tools/creature_route.py`: `~/rigwork/bench/bin/creature-bench`
  (exists on f only); `../motionforge/rust/target/release/motionforge`,
  `../motionforge/tools/tripo_to_glb.py` and
  `../weightforge/rust/target/release/weights` (old folder names: no longer
  resolve); the macOS Blender path.
- `skintokens/tools/creature_blind.py` and `tools/bench/score_artists.py`:
  `~/rigwork/bench/bin/creature-bench`, `~/rigwork/creatures/bin/creature-rig`,
  `~/rigwork/creatures/bin/weights` (f's old builds),
  `../weightforge/...`, `~/.local/bin/blender`, the macOS Blender path.
- `skintokens/tools/bench/run_routes.sh`: `TOOLS=/home/f/rigwork/creatures`.
- meshweights `weights fix --skintokens`: `$SKINTOKENS_BIN`, `PATH`, then
  `~/Games/_blender/skintokens/bin/skintokens`.

Outside the code:

- skintokens on f's NVIDIA card needs a CUDA build
  (`--no-default-features --features cuda`, `CUDA_ROOT` and an older gcc
  through `NVCC_PREPEND_FLAGS`); the one in use is
  `f:/home/f/rigwork/creatures/bin/skintokens`, built 2026-11-12.
- The benchmark animals: `f:/home/f/rigwork/bench/animals/` (12, prepared
  by `tools/bench/prep_truth.py`; sources under
  `f:/home/f/rigwork/third_party/bench/`). retopo's five bench creatures:
  `f:~/rfwork/set/src/`. The owner's backlog: `f:/home/f/rigwork/creatures/in/`
  and `f:/home/f/rigwork/in/`, listed in `f:/home/f/rigwork/bench/owner_set.tsv`.
- On z the tools' release binaries were last built 2026-11-17; on f the
  repos' own targets are older (motionforge 2026-11-07, no skintokens
  binaries in its target).

## What `tools/creature.sh` adds

`tools/creature.sh NAME MODEL OUT`: a name, a model, an output folder; no
other flags, no host names. Tools are found by environment variable, then
`PATH`, then each sibling repo's own build under its current folder name;
none by host path. It runs:

1. `tripo_rig.py MODEL OUT/run1 --name NAME --kind creature --class monster`
   (which hands off to skintokens' creature route).
2. The clips the body lacks, from a clip maker for its body plan; a body
   plan with none is a gate 5 fail with that reason.
3. `tools/creature_gates.py check` on the result: gates 2 to 5, each
   check run again on the finished GLB by the gate script, never read from
   the route's own report.
4. The same route again into `OUT/run2`, compared with the first (gate 7).
5. The planted inputs made from this creature's own result (swapped limbs,
   a hole, a vertex with five influences, over budget), each of which must
   make the gate script exit non-zero naming the right check (gate 8).
6. The blind films at HLH's follow camera and a fresh judge (gate 6).
7. The repos' checks at the commits it ran (gate 1).
8. `gates.json`: each gate's command, exit code and numbers; exit 0 only
   when every gate passes.

Gate 9 (the creature filmed in HLH by charkit's game step, committed only
to HLH branch `character-line`) runs after it, from an HLH worktree.

The gate script, its planted inputs and the input lists are committed
before the command, and are not edited afterwards to get a pass.
