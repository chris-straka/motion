# Refitting clips to a body's width (design, motion-13)

Written 2026-11-19. Not built. Work list item `motion-13`: arms clear the
torso, hands never enter the body, arms don't pass through each other on a
wide chest.

## The fault

The game's clips were made on the clip library's slim body. Played on a
wider one, the hands follow the slim body's paths and go into the wider
one. The hero's A6 body in his tunic (bodygraft's shared notes, the route's
limbs-through frames):

| clip  | moment | what                              |
| ----- | ------ | --------------------------------- |
| death | 0.53   | left arm into the torso           |
| death | 0.60   | right arm into the torso          |
| death | 0.80   | arms through each other           |
| dodge | 0.20   | left arm into the left leg        |
| run   | 0.63   | deep hip bend (not a width fault) |

The run's hip bend is a pose fault of the clip, not of the body's width;
it is listed here so it isn't mistaken for one, and is left out.

## What exists, and why it doesn't reach the hero

`animate`'s contact pass (`contact.md`) already clears hands, forearms and a
held blade from torso, head and thigh capsules, per frame, with two-bone
arm IK, a max-envelope ramp and an exact winding-number check. Two gaps:

1. Its proxies are torso, head and thighs only. The other arm, the upper
   arms and the shins are not proxies (`contact.md`, "Limits"), so arms
   through each other (death 0.80) and a hand into a leg below the thigh
   capsule (dodge 0.20) are never seen.
2. HLH doesn't play `animate`'s output. The game retargets the library's
   clips onto each body when it appears (`hlh/bevy/src/clip_retarget.rs`),
   so no contact pass ever runs on the hero's clips. `motion-5` (today's
   clips into HLH) bakes clips into his body's GLB; the refit rides on
   that: a clip baked by `animate` is played as baked.

## The step

A retargeting step in `animate`, after the retarget and before `clean`,
per body: `--refit-width` (default on for humanoids).

1. **Width first, as a retarget.** Measure the source and target chests
   from their proxies: the torso lozenge's half-width at the shoulders and
   at the waist, and the hip capsules' outer edges. Each frame, each
   wrist's position relative to its shoulder is split into the body's
   frame (side, up, forward). Where the source wrist lies inside the
   source's own width envelope plus a margin (a hand across or beside the
   body), its side component is scaled by the target's width over the
   source's at that height; outside the envelope (an arm out wide) it is
   left alone, with a smooth blend between. The elbow keeps its bend
   plane (the existing two-bone solve), the hand its world rotation. This
   is a retarget of the arm's path to the body, not a correction: it
   changes no timing and no keys' count, and a target the source's width
   gets the clip unchanged.
2. **Then contact, as clean-up.** The existing pass runs on the result,
   with new proxies: each upper arm (shoulder to elbow, the armpit), the
   other arm's forearm and hand, and the shins. New pairs:
   - arm against arm: both wrists move apart along the deepest contact's
     normal, half each; when one hand holds the weapon, the free hand
     moves first (the blade's line matters more);
   - hand or forearm against a leg: the arm moves, the leg never does
     (the legs carry the plants and the root);
   - pairs that already touch at rest, and moments where the source body
     itself passes an arm through (measured on the source with the same
     proxies), keep that depth as slack: the clip's own faults are not
     the body's.
3. Caps and ramp as today (wrist at most 12 % of the height; 0.15 s
   smoothstep), so a swing's arc and timing are kept.

## Its test

Written before the code; the first two must fail at today's HEAD.

1. **Gate (the item's).** On the hero's A6 body with the game's clips put
   on by `animate`: meshweights' `weights check --motion` limb-through
   area (`through_cm2`, the route's gate 5 measure, bodygraft
   `tools/humanoid_gates.py`) at or under the base body's own amount on
   the same clips plus 25 % (the route's allowance), and no `limb_through`
   fail finding within 0.05 of any of the four width moments above. Today
   it fails on those four.
2. **Unit, in `rust/core/tests/width_refit.rs`** (no mesh, proxies only):
   a library rig with a 0.30 m chest and a target 1.4 times as wide; a
   clip whose hands cross in front of the chest at the library's width,
   one hand passing the hip at thigh height. After the step: no hand or
   forearm sample inside a target torso or leg proxy; the two forearm
   capsules don't meet on any frame where the source's didn't; each
   wrist's fastest frame is the same frame as the source's (timing kept);
   frames more than the ramp away from any contact are unchanged; and the
   same clip on a target of the source's own width comes out unchanged.
3. **Pictures.** The five moments at the game camera, before and after,
   judged by eye (the refit must still read as the same move).
