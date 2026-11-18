# Creature gate fixtures

A made-up four-legged creature (no model, no corpus) and the planted bad
inputs of gate 8 (`~/Games/MONSTERS.md`), made from it once on 2026-11-18
and pinned by sha256 in `expected.json`. `tools/creature_gates.py
selftest` checks that `clean.glb` passes every check and that each planted
file fails, naming its check. These files are never remade or edited to
get a pass.

How they were made (tools at their 2026-11-17 release builds on z:
skintokens' `creature-rig` and motion's `motionforge`):

    python3 tools/creature_fixture.py mesh.glb
    creature-rig rig mesh.glb rig.glb --rom rom.ron --blend 1.6
    motionforge creature-profile --rig rig.glb --output prof.json --clean-glb c.glb --report pr.json --name fixture
    motionforge creature-gait --creature prof.json --glb-in c.glb --glb-out clean.glb --rom gait_rom.ron
    python3 tools/creature_gates.py plant swap clean.glb swap.glb              # each limb's skin on its mirror partner's bones
    python3 tools/creature_gates.py plant hole clean.glb hole.glb              # the faces round the highest vertex cut
    python3 tools/creature_gates.py plant influence5 clean.glb influence5.glb  # one vertex given a fifth weight
    python3 tools/creature_gates.py plant overbudget clean.glb overbudget.glb  # every triangle split in four: 30,800

| file | expected | what `check` said when made |
| --- | --- | --- |
| clean.glb | every check passes | PASS (bend 89.5, skeleton 100, anatomy passes, walk and attack) |
| swap.glb | fails `anatomy` | anatomy (`B_SWAPPED` on all four legs), bend |
| hole.glb | fails `holes` | holes (one rim, 6 edges) |
| influence5.glb | fails `influences` | influences, contract (rfcheck also refuses a fifth influence) |
| overbudget.glb | fails `tris` | tris (30,800 over 20,000) |
