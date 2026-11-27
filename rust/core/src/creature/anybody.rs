//! An idle and an attack for a creature of any body plan, read off its
//! profile by the skeleton's shape alone (no bone names): two-legged,
//! winged, many-limbed, legless, or four-legged where the planted-foot
//! gait (`gait.rs`) can't read it. What `creature-gait` is to a clean
//! four-legged body, this is to the rest, so that every creature has
//! something to stand in and something to strike with.
//!
//! The body (`read`):
//! - the centre line, followed by structure: from the hierarchy root, at
//!   each bone the child nearest the mirror plane goes on (of those about
//!   as near, the one reaching farthest forward and up), so a generated
//!   body's spine that wanders sideways is still its spine. That is the
//!   *front chain* (a quadruped's spine to its head, a biped's spine up to
//!   its head, a fish's body to its snout); the *back chain* is the centre
//!   branch reaching farthest back from the front chain's first third (a
//!   tail);
//! - side chains: every branch leaving the centre line sideways, from its
//!   first bone out to a leaf; one whose subtree holds a foot (a bone the
//!   creature stands on, from the profile) is a leg, the others are arms
//!   (arms, wings, fins);
//! - a bone with a foot anywhere below it is *held*: turning it would move
//!   a planted foot, so neither clip turns it. A body with no feet (a
//!   swimmer, a flyer) has nothing held, and may move as a whole.
//!
//! The clips start and end at rest; the idle loops. Everything is in the
//! profile's frame (Z up) through `gait`'s poser, so the clips go into the
//! GLB with `glbanim::clip_to_glb` like the gait's.

use super::gait::{
    apply_leg_pts, chain_toward, children, left_of, leg_rig_bones, make_frame, smooth, LegRig,
    Poser, Stride,
};
use super::profile::{skeleton_bounds, Creature};
use crate::clip::{fk, Clip};
use crate::detmath;
use crate::math::{Quat, Vec3};

const TAU: f64 = std::f64::consts::PI * 2.0;

/// A body read off its skeleton, as bone indices into the profile's.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// From the hierarchy root forward and up to the head (the root first).
    pub front: Vec<usize>,
    /// From the first part of the front chain backward to the tip.
    pub back: Vec<usize>,
    /// Side chains holding a foot, each from the body out.
    pub legs: Vec<Vec<usize>>,
    /// Side chains holding none (arms, wings, fins), each from the body out.
    pub arms: Vec<Vec<usize>>,
    /// Per bone: a planted foot is somewhere below it.
    pub held: Vec<bool>,
    /// The body stands on nothing (no feet in the profile).
    pub floats: bool,
    /// Longest side of the skeleton's bounds.
    pub size: f64,
}

impl Body {
    fn free(&self, bones: &[usize]) -> Vec<usize> {
        bones.iter().copied().filter(|&b| !self.held[b]).collect()
    }
    /// The free part of the front chain (what can breathe and strike).
    pub fn free_front(&self) -> Vec<usize> {
        self.free(&self.front)
    }
    pub fn free_back(&self) -> Vec<usize> {
        self.free(&self.back)
    }
    /// What strikes in the attack: the free side chains when they come as
    /// a pair or more (arms, wings); else, on a body of four limbs or more,
    /// the pair whose tips reach farthest forward (a spider's front legs, a
    /// knuckle-walker's hands), lifted off the ground; it stands on the
    /// others. A lone free chain (a leg whose foot the profile cut) does not
    /// make an attack.
    pub fn strikers(&self, c: &Creature) -> Vec<Vec<usize>> {
        if self.arms.len() >= 2 {
            return self.arms.clone();
        }
        let mut all: Vec<Vec<usize>> = self.legs.iter().chain(self.arms.iter()).cloned().collect();
        if all.len() < 4 {
            return self.arms.clone();
        }
        let up = Vec3::new(0.0, 0.0, 1.0);
        let ahead = |limb: &Vec<usize>| {
            let tip = limb.last().map_or(c.skeleton.bones[limb[0]].tail, |&t| {
                c.skeleton.bones[t].tail
            });
            let b = &c.skeleton.bones[limb[0]];
            let at = b.parent.map_or(b.head, |p| c.skeleton.bones[p].head);
            tip.dot(c.forward) + 0.5 * at.dot(up)
        };
        all.sort_by(|a, b| ahead(b).total_cmp(&ahead(a)).then(a[0].cmp(&b[0])));
        all.truncate(2);
        all
    }

    /// `held` with the feet of `lifted` chains taken off the ground.
    pub fn held_without(&self, c: &Creature, lifted: &[Vec<usize>]) -> Vec<bool> {
        let off_ground: Vec<usize> = lifted.iter().flat_map(|l| subtree(c, l[0])).collect();
        let mut held = vec![false; c.skeleton.len()];
        for &f in c.feet.iter().filter(|f| !off_ground.contains(f)) {
            let mut cur = Some(f);
            while let Some(b) = cur {
                held[b] = true;
                cur = c.skeleton.bones[b].parent;
            }
        }
        held
    }
}

fn subtree(c: &Creature, bone: usize) -> Vec<usize> {
    let mut out = vec![bone];
    let mut k = 0;
    while k < out.len() {
        let b = out[k];
        out.extend(children(c, b));
        k += 1;
    }
    out
}

/// Read the body. Never fails on a skeleton with at least one bone.
pub fn read(c: &Creature) -> Body {
    let n = c.skeleton.len();
    let (lo, hi) = skeleton_bounds(&c.skeleton);
    let ext = hi - lo;
    let size = ext.x.max(ext.y).max(ext.z).max(1e-6);
    let left = left_of(c);
    let up = Vec3::new(0.0, 0.0, 1.0);
    let root = c.root();
    // Anchor the mirror plane at the hierarchy root instead of the global
    // min/max across all bones: one longer or outstretched arm/wing tip
    // must not drag the plane away from the pelvis and spine.
    let mid = c.skeleton.bones[root].head.dot(left);
    // Off the mirror plane, by its head. The centre line is followed by
    // structure, not by a band: at each bone the child nearest the plane
    // goes on (a generated body's spine wanders a few percent sideways;
    // its limbs leave it in pairs, well out).
    let side = |b: usize| c.skeleton.bones[b].head.dot(left) - mid;
    let off = |b: usize| side(b).abs();
    let rel_side =
        |b: usize, cur: usize| (c.skeleton.bones[b].head - c.skeleton.bones[cur].head).dot(left);
    let sag_dir = |b: usize| {
        let v = c.skeleton.bones[b].tail - c.skeleton.bones[b].head;
        (v.dot(c.forward), v.dot(up), v.length())
    };
    // A child that has a mirror partner on the other side of its parent
    // (comparable lateral distance, height, and fore/aft position, not
    // pointing opposite ways in the sagittal plane) is one of a pair
    // (legs, arms, eyes): never the centre line.
    let paired = |k: usize, cur: usize, among: &[usize]| {
        let sk = rel_side(k, cur);
        let ak = sk.abs();
        if ak <= 0.03 * size {
            return false;
        }
        let hk = c.skeleton.bones[k].head;
        let (kf, ku, klen) = sag_dir(k);
        let k_sag = (kf * kf + ku * ku).sqrt();
        among.iter().any(|&o| {
            if o == k {
                return false;
            }
            let so = rel_side(o, cur);
            let ao = so.abs();
            if so * sk >= 0.0 || (ao - ak).abs() > 0.03 * size {
                return false;
            }
            let ho = c.skeleton.bones[o].head;
            if (hk - ho).dot(up).abs() > 0.12 * size || (hk - ho).dot(c.forward).abs() > 0.12 * size
            {
                return false;
            }
            let (of, ou, olen) = sag_dir(o);
            let o_sag = (of * of + ou * ou).sqrt();
            if k_sag > 0.25 * klen && o_sag > 0.25 * olen && (kf * of + ku * ou) <= 0.0 {
                return false;
            }
            true
        })
    };
    let foot_descendant = |b: usize| -> bool {
        let mut p = Some(b);
        while let Some(x) = p {
            if c.feet.contains(&x) {
                return true;
            }
            p = c.skeleton.bones[x].parent;
        }
        false
    };
    // A child branch whose subtree terminates solely in a single foot (and
    // optional toes below it) without climbing above its parent is a leg,
    // never a spine or tail. (A quadruped's spine holds two front feet and
    // a head/neck, so it is not a pure leg.)
    let is_pure_leg = |k: usize, cur: usize| -> bool {
        let sub = subtree(c, k);
        let foot_count = sub.iter().filter(|x| c.feet.contains(x)).count();
        if foot_count != 1 {
            return false;
        }
        let all_leaves_are_foot = sub
            .iter()
            .filter(|&&x| children(c, x).is_empty())
            .all(|&leaf| foot_descendant(leaf));
        if !all_leaves_are_foot {
            return false;
        }
        let cur_z = c.skeleton.bones[cur].head.z;
        sub.iter()
            .all(|&x| c.skeleton.bones[x].head.z <= cur_z + 0.05 * size)
    };
    let band = 0.10 * size;
    let reach = |b: usize, dir: Vec3| -> f64 {
        subtree(c, b)
            .iter()
            .map(|&x| c.skeleton.bones[x].tail.dot(dir))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let centre_chain = |start: usize, dir: Vec3, skip: &[usize]| -> Vec<usize> {
        let mut out = vec![start];
        let mut cur = start;
        loop {
            let all = children(c, cur);
            // an only child goes on however far it leans (a head held to one side)
            let kids: Vec<usize> = all
                .iter()
                .copied()
                .filter(|&k| {
                    !skip.contains(&k)
                        && !is_pure_leg(k, cur)
                        && (all.len() == 1 || (off(k) <= band && !paired(k, cur, &all)))
                })
                .collect();
            if kids.is_empty() {
                break;
            }
            let least = kids.iter().map(|&k| off(k)).fold(f64::INFINITY, f64::min);
            let Some(next) = kids
                .iter()
                .copied()
                .filter(|&k| off(k) <= least + 0.03 * size)
                .max_by(|a, b| reach(*a, dir).total_cmp(&reach(*b, dir)).then(b.cmp(a)))
            else {
                break;
            };
            out.push(next);
            cur = next;
        }
        out
    };
    let mut held = vec![false; n];
    for &f in &c.feet {
        let mut cur = Some(f);
        while let Some(b) = cur {
            held[b] = true;
            cur = c.skeleton.bones[b].parent;
        }
    }
    let front_dir = (c.forward + up).normalized();
    let front = centre_chain(root, front_dir, &[]);
    // The tail: the centre branch reaching farthest back from the front
    // chain's first third (the hips and what is next to them).
    let back_dir = (c.forward.scale(-1.0) + up.scale(-0.3)).normalized();
    let mut back: Vec<usize> = Vec::new();
    let mut best = f64::NEG_INFINITY;
    for &from in front.iter().take(front.len().div_ceil(3).max(1)) {
        let base = c.skeleton.bones[from].head.dot(back_dir);
        let all = children(c, from);
        let cands: Vec<usize> = all
            .iter()
            .copied()
            .filter(|&k| {
                !front.contains(&k)
                    && !is_pure_leg(k, from)
                    && off(k) <= band
                    && !paired(k, from, &all)
            })
            .collect();
        let least = cands.iter().map(|&k| off(k)).fold(f64::INFINITY, f64::min);
        for k in cands {
            if off(k) > least + 0.03 * size {
                continue;
            }
            let ch = centre_chain(k, back_dir, &front);
            if let Some(&tip_idx) = ch.last() {
                let tip = c.skeleton.bones[tip_idx].tail.dot(back_dir);
                if tip > base + 0.05 * size && tip > best {
                    best = tip;
                    back = ch;
                }
            }
        }
    }
    let mut on_line: Vec<usize> = front.clone();
    on_line.extend(&back);
    let mut legs = Vec::new();
    let mut arms = Vec::new();
    for &b in &on_line {
        for k in children(c, b) {
            if on_line.contains(&k) {
                continue;
            }
            let out = (c.skeleton.bones[k].tail - c.skeleton.bones[b].head).normalized();
            let ch = chain_toward(c, k, out, &on_line);
            let has_foot = subtree(c, k).iter().any(|x| c.feet.contains(x));
            if has_foot {
                legs.push(ch);
            } else {
                arms.push(ch);
            }
        }
    }
    Body {
        front,
        back,
        legs,
        arms,
        held,
        floats: c.feet.is_empty(),
        size,
    }
}

fn dir_of(c: &Creature, b: usize) -> Vec3 {
    let bone = &c.skeleton.bones[b];
    (bone.tail - bone.head).normalized()
}

/// The world axis turning bone `b` so its tip moves toward `want`; when the
/// bone already points along `want` (or against it), toward `fallback`.
fn axis_toward(c: &Creature, b: usize, want: Vec3, fallback: Vec3) -> Vec3 {
    let d = dir_of(c, b);
    let a = d.cross(want);
    if a.length() >= 0.25 {
        return a.normalized();
    }
    let f = d.cross(fallback);
    if f.length() > 1e-6 {
        f.normalized()
    } else {
        left_of(c)
    }
}

/// Standing and breathing, `frames` long at `fps`, looping: the free part
/// of the front chain rises and falls, the head looks about, a tail sways,
/// arms and wings hang and stir. A body that stands on nothing sways
/// along its whole length and beats its fins or wings.
pub fn idle(c: &Creature, body: &Body, frames: usize, fps: f64) -> Clip {
    let up = Vec3::new(0.0, 0.0, 1.0);
    let down = up.scale(-1.0);
    let front = body.free_front();
    let back = body.free_back();
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let t = f as f64 / frames as f64;
        let s1 = detmath::sin(TAU * t);
        let mut p = Poser::new(c);
        if let Some((head, rest)) = front.split_last() {
            if !rest.is_empty() {
                p.pitch_chain(rest, 3.0 * s1);
            }
            p.yaw(*head, 8.0 * detmath::sin(TAU * t + 0.6));
            p.pitch(*head, -2.5 * s1);
        }
        for (i, &b) in back.iter().enumerate() {
            let k = back.len() as f64;
            p.yaw(b, 14.0 / k * detmath::sin(TAU * t - 0.45 * i as f64));
        }
        if body.floats {
            // the whole body sways as it hangs in the water or the air
            for (i, &b) in body.front.iter().enumerate().skip(1) {
                p.yaw(b, 3.0 * detmath::sin(TAU * t + 0.5 * i as f64));
            }
        }
        for (k, arm) in body.arms.iter().enumerate() {
            let phase = if k % 2 == 0 { 0.0 } else { 0.5 };
            let top = arm[0];
            let amount = if body.floats { 14.0 } else { 4.0 };
            p.add(
                top,
                axis_toward(c, top, up, c.forward),
                amount * detmath::sin(TAU * (t + phase)),
            );
            if let Some(&elbow) = arm.get(1) {
                p.add(elbow, axis_toward(c, elbow, down, c.forward), 3.0 * s1);
            }
        }
        out.push(p.frame());
    }
    Clip {
        fps,
        skeleton: c.skeleton.clone(),
        frames: out,
    }
}

/// A strike, `frames` long at `fps`, from rest to rest: rear back (the
/// free front chain lifts, the arms draw back and up, a tail rises), then
/// throw forward (the front chain and head drive forward and down, the
/// arms swing through), and recover. A body standing on five legs or more
/// and no arms strikes with its front pair; a body on nothing lunges
/// forward as a whole.
pub fn attack(c: &Creature, body: &Body, frames: usize, fps: f64) -> Clip {
    let up = Vec3::new(0.0, 0.0, 1.0);
    let left = left_of(c);
    let strikers = body.strikers(c);
    // a striking leg leaves the ground, and what only it held is free
    let held = body.held_without(c, &strikers);
    let front: Vec<usize> = body.front.iter().copied().filter(|&b| !held[b]).collect();
    let back: Vec<usize> = body.back.iter().copied().filter(|&b| !held[b]).collect();
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let t = if frames > 1 {
            f as f64 / (frames - 1) as f64
        } else {
            0.0
        };
        let rear = smooth(t / 0.35) * (1.0 - smooth((t - 0.35) / 0.15));
        let strike = smooth((t - 0.35) / 0.15) * (1.0 - smooth((t - 0.55) / 0.45));
        let mut p = Poser::new(c);
        if let Some((head, rest)) = front.split_last() {
            if !rest.is_empty() {
                p.pitch_chain(rest, 18.0 * rear - 32.0 * strike);
            }
            p.pitch(*head, 12.0 * rear - 18.0 * strike);
        }
        if !back.is_empty() {
            // the tail goes against the body: down as it rears, up as it strikes
            p.pitch_chain(&back, -8.0 * rear + 14.0 * strike);
        }
        // The second striker of a pair follows a beat later (its strike
        // starts at 0.45, not 0.35) at 40% of the swing: one claw leads, as
        // a swipe does. A mirror-image strike also hid a left-right swap
        // from the blind judge (a held-out creature, 2026-11-18). Wings
        // still beat together.
        let strike_late = smooth((t - 0.45) / 0.15) * (1.0 - smooth((t - 0.65) / 0.35));
        for (k, arm) in strikers.iter().enumerate() {
            let top = arm[0];
            let tip_idx = arm.last().copied().unwrap_or(top);
            let tip = c.skeleton.bones[tip_idx].tail;
            let reach = (tip - c.skeleton.bones[top].head).normalized();
            let sideways = reach.dot(left).abs() > 0.55;
            let (gain, s) = if k == 0 || sideways {
                (1.0, strike)
            } else {
                (0.4, strike_late)
            };
            // Every striking limb swings in its own side's upright plane
            // (about the left-right axis), so it never crosses the body:
            // drawn back as the body rears, then swung through.
            p.pitch(top, gain * (-30.0 * rear + 60.0 * s));
            if let Some(&elbow) = arm.get(1) {
                p.pitch(elbow, gain * (25.0 * rear - 10.0 * s));
            }
            // A limb held out sideways (a wing, a fin) also beats: up as the
            // body rears, down through the strike.
            if sideways {
                p.add(
                    top,
                    axis_toward(c, top, up, c.forward),
                    45.0 * rear - 45.0 * strike,
                );
            }
        }
        if body.floats {
            // the whole body goes with the strike, and draws back before it
            p.shift = c.forward.scale(body.size * (0.22 * strike - 0.06 * rear));
            for &b in body.front.iter().skip(1) {
                p.pitch(b, -6.0 * strike);
            }
        }
        out.push(p.frame());
    }
    Clip {
        fps,
        skeleton: c.skeleton.clone(),
        frames: out,
    }
}

/// A test clip, never shipped: each side limb in turn (legs too, lifted
/// off the ground) swings forward and out to its own side, folds, and
/// comes back, `frames_each` frames a limb. Filmed beside the same rig with
/// its limbs swapped, it shows whether a swap can be seen at all (gate 6's
/// planted case): a limb's skin hung on its mirror partner's bones moves
/// just like the real thing when it only swings forward (the two pivots
/// differ only along that axis), and goes through the body when it swings
/// out.
pub fn sweep(c: &Creature, body: &Body, frames_each: usize, fps: f64) -> Clip {
    let up = Vec3::new(0.0, 0.0, 1.0);
    let left = left_of(c);
    let mid = c
        .skeleton
        .bones
        .iter()
        .map(|b| b.head.dot(left))
        .sum::<f64>()
        / c.skeleton.len().max(1) as f64;
    let mut limbs: Vec<&Vec<usize>> = body.legs.iter().chain(body.arms.iter()).collect();
    limbs.sort_by_key(|l| l[0]);
    let mut out = Vec::with_capacity(frames_each * limbs.len().max(1));
    for limb in &limbs {
        let Some(&tip_idx) = limb.last() else {
            continue;
        };
        let tip = c.skeleton.bones[tip_idx].tail;
        let outward = if tip.dot(left) >= mid {
            left
        } else {
            left.scale(-1.0)
        };
        for f in 0..frames_each {
            let t = f as f64 / frames_each as f64;
            let s = detmath::sin(std::f64::consts::PI * t);
            let mut p = Poser::new(c);
            let top = limb[0];
            p.add(top, axis_toward(c, top, c.forward, up), 30.0 * s);
            p.add(top, axis_toward(c, top, outward, up), 30.0 * s);
            if let Some(&mid_bone) = limb.get(1) {
                p.add(mid_bone, axis_toward(c, mid_bone, up, c.forward), 30.0 * s);
            }
            out.push(p.frame());
        }
    }
    if out.is_empty() {
        out.push(Poser::new(c).frame());
    }
    Clip {
        fps,
        skeleton: c.skeleton.clone(),
        frames: out,
    }
}

/// Trace a leg chain down to its actual foot bone in `c.feet` when the leg
/// subtree forks near the foot; otherwise keep `leg` as read.
fn leg_chain_to_foot(c: &Creature, leg: &[usize]) -> Vec<usize> {
    let top = leg[0];
    if let Some(&last) = leg.last() {
        if c.feet.contains(&last) {
            return leg.to_vec();
        }
    }
    let sub = subtree(c, top);
    if let Some(&foot) = c.feet.iter().filter(|f| sub.contains(f)).min_by(|a, b| {
        c.skeleton.bones[**a]
            .tail
            .z
            .total_cmp(&c.skeleton.bones[**b].tail.z)
            .then(a.cmp(b))
    }) {
        let mut chain = vec![foot];
        let mut cur = foot;
        while cur != top {
            let Some(p) = c.skeleton.bones[cur].parent else {
                break;
            };
            chain.push(p);
            cur = p;
        }
        chain.reverse();
        return chain;
    }
    leg.to_vec()
}

/// Index in `chain` where the rigid flat foot starts. A plantigrade 4+ bone
/// leg ending in `[..., Foot, ToeBase]` keeps both foot bones flat as one
/// block so the upper leg solves cleanly to the ankle.
fn foot_start_of(c: &Creature, chain: &[usize]) -> usize {
    let n = chain.len();
    if n < 4 {
        return n.saturating_sub(1);
    }
    let sk = &c.skeleton;
    let pen = &sk.bones[chain[n - 2]];
    let tip = &sk.bones[chain[n - 1]];
    let top = &sk.bones[chain[0]];
    let pen_len = (pen.tail - pen.head).length();
    let pen_dz = (pen.head.z - pen.tail.z).abs();
    let total_drop = (top.head.z - tip.tail.z).max(1e-6);
    let pen_above = pen.head.z - tip.tail.z;
    if pen_len > 1e-6 && pen_dz < 0.75 * pen_len && pen_above < 0.25 * total_drop {
        n - 2
    } else {
        n - 1
    }
}

/// Solve one biped/monster leg onto `contact` preserving every bone length:
/// exact 2-bone IK when `m == 2` (avoiding collinear knee lock on straight
/// rest legs) and bend-initialized FABRIK for longer chains, then aimed via
/// `gait::apply_leg_pts`.
fn solve_biped_leg(c: &Creature, rig: &LegRig, quats: &mut [Quat], shift: Vec3, contact: Vec3) {
    let sk = &c.skeleton;
    let m = rig.segs.len();
    if m == 0 {
        return;
    }
    for &b in &rig.bones {
        quats[b] = Quat::IDENTITY;
    }
    let posed = fk(sk, &make_frame(c, quats, shift));
    let mut pts: Vec<Vec3> = rig.bones[..=m].iter().map(|&b| posed[b].head).collect();
    let lens: Vec<f64> = rig.segs.iter().map(|s| s.length()).collect();
    let hip = pts[0];
    let mut ankle = contact - rig.paw_vec;
    let far = (ankle - hip).length();
    if far > 0.99 * rig.reach {
        ankle = hip + (ankle - hip).scale(0.99 * rig.reach / far);
    }

    let rest_pts: Vec<Vec3> = rig.bones[..=m].iter().map(|&b| sk.bones[b].head).collect();
    let rest_chord = rest_pts[m] - rest_pts[0];
    let rest_dir = rest_chord.normalized();
    let mut cum = vec![0.0; m + 1];
    for k in 0..m {
        cum[k + 1] = cum[k] + lens[k];
    }
    let total = cum[m].max(1e-9);
    let mut rest_bends = vec![Vec3::ZERO; m + 1];
    let mut max_bend = 0.0_f64;
    for k in 1..m {
        let frac = cum[k] / total;
        let on_line = rest_pts[0] + rest_chord.scale(frac);
        let raw = rest_pts[k] - on_line;
        let perp = raw - rest_dir.scale(raw.dot(rest_dir));
        max_bend = max_bend.max(perp.length());
        rest_bends[k] = perp;
    }
    let d = if (ankle - hip).length() > 1e-9 {
        (ankle - hip).normalized()
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    };
    let perp_to = |v: Vec3| -> Vec3 { v - d.scale(v.dot(d)) };

    if m == 1 {
        pts[0] = hip;
        pts[1] = hip + d.scale(lens[0]);
    } else if m == 2 {
        let (l0, l1) = (lens[0], lens[1]);
        let min_d = (l0 - l1).abs() + 1e-4;
        let max_d = (0.995 * (l0 + l1)).max(min_d);
        let dist = (ankle - hip).length().clamp(min_d, max_d);
        let a = (l0 * l0 - l1 * l1 + dist * dist) / (2.0 * dist);
        let h = (l0 * l0 - a * a).max(0.0).sqrt();
        let hint = if max_bend >= 0.03 * rig.reach {
            rest_bends[1]
        } else {
            c.forward
        };
        let p = perp_to(hint);
        let pole = if p.length() > 1e-6 {
            p.normalized()
        } else {
            let f = perp_to(c.forward);
            if f.length() > 1e-6 {
                f.normalized()
            } else {
                left_of(c)
            }
        };
        pts[0] = hip;
        pts[1] = hip + d.scale(a) + pole.scale(h);
        pts[2] = hip + d.scale(dist);
    } else {
        let fwd_perp = {
            let f = perp_to(c.forward);
            if f.length() > 1e-6 {
                f.normalized()
            } else {
                left_of(c)
            }
        };
        for k in 1..m {
            let frac = cum[k] / total;
            let offset = if max_bend >= 0.03 * rig.reach {
                perp_to(rest_bends[k])
            } else {
                fwd_perp.scale(0.10 * rig.reach * detmath::sin(std::f64::consts::PI * frac))
            };
            pts[k] = hip.lerp(ankle, frac) + offset;
        }
        for _ in 0..48 {
            pts[m] = ankle;
            for k in (0..m).rev() {
                pts[k] = pts[k + 1] + (pts[k] - pts[k + 1]).normalized().scale(lens[k]);
            }
            pts[0] = hip;
            for k in 0..m {
                pts[k + 1] = pts[k] + (pts[k + 1] - pts[k]).normalized().scale(lens[k]);
            }
        }
    }
    apply_leg_pts(c, rig, &posed, &pts, quats);
}

fn walk_biped(c: &Creature, body: &Body, fps: f64) -> Result<Stride, String> {
    let left = left_of(c);
    let up = Vec3::new(0.0, 0.0, 1.0);
    let mut legs: Vec<Vec<usize>> = body.legs.iter().map(|l| leg_chain_to_foot(c, l)).collect();
    for leg in &legs {
        if leg.len() < 2 {
            return Err("two-legged walk requires at least 2 bones per leg".into());
        }
    }
    // Order deterministically: left leg first, right leg second.
    legs.sort_by(|a, b| {
        let tip_dot = |ch: &Vec<usize>| {
            ch.last()
                .map_or(0.0, |&t| c.skeleton.bones[t].tail.dot(left))
        };
        tip_dot(b).total_cmp(&tip_dot(a)).then(a[0].cmp(&b[0]))
    });
    let rigs: Vec<LegRig> = legs
        .iter()
        .map(|l| leg_rig_bones(c, l, foot_start_of(c, l)))
        .collect();
    let hip = (rigs[0].drop + rigs[1].drop) * 0.5;
    if hip <= 1e-6 {
        return Err("two-legged walk requires legs with positive vertical drop".into());
    }

    let duty = 0.62;
    let offsets = [0.0, 0.5];
    let froude = 0.20;
    let stride_in_hips = 1.4;
    let full_speed = (froude * 9.81 * hip).sqrt();
    let frames = ((stride_in_hips * hip / full_speed) * fps)
        .round()
        .max(16.0) as usize;
    let period = frames as f64 / fps;

    let crouch = 0.075 * hip;
    let bob = 0.025 * hip;
    let lat_shift = 0.025 * hip;
    let phase_shift = 0.5 * duty - 0.25;
    let z_shift_at = |t: f64| -crouch - bob * detmath::cos(2.0 * TAU * (t - phase_shift));
    let lat_at = |t: f64| lat_shift * detmath::sin(TAU * (t - phase_shift));

    let mut half_reach = f64::INFINITY;
    for (rig, off) in rigs.iter().zip(offsets) {
        let top = rig.bones[0];
        let ankle_bone = rig.bones[rig.segs.len()];
        let rest_off = c.skeleton.bones[ankle_bone].head - c.skeleton.bones[top].head;
        let rest_fwd = rest_off.dot(c.forward);
        let rest_lat = rest_off.dot(left);
        for k in 0..=16 {
            let u = k as f64 / 16.0;
            let s = 1.0 - 2.0 * u;
            if s.abs() < 1e-6 {
                continue;
            }
            let phase = u * duty;
            let t = (phase - off).rem_euclid(1.0);
            let dz = (rig.drop + z_shift_at(t)).max(0.0);
            let dl = rest_lat - lat_at(t);
            let avail_sq = (0.97 * rig.reach).powi(2) - dz * dz - dl * dl;
            let avail = avail_sq.max(0.0).sqrt();
            let max_h = ((avail - s.signum() * rest_fwd) / s.abs()).max(0.0);
            half_reach = half_reach.min(max_h);
        }
    }
    let stance = (duty * stride_in_hips * hip).min(2.0 * half_reach);
    if stance <= 1e-6 {
        return Err("legs cannot clear a stride without stretching".into());
    }
    let speed = stance / (duty * period);
    let half = 0.5 * stance;
    let lift = (0.14 * hip).max(0.055 * c.height).min(0.45 * hip);
    let swing_time = (1.0 - duty) * period;

    let free_front = body.free_front();
    let free_back = body.free_back();
    let legs_mid = 0.5
        * (c.skeleton.bones[legs[0][0]].head.dot(left)
            + c.skeleton.bones[legs[1][0]].head.dot(left));

    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let t = f as f64 / frames as f64;
        let mut p = Poser::new(c);
        let step_cos = detmath::cos(TAU * t);
        let bob_cos = detmath::cos(2.0 * TAU * (t - phase_shift));
        if let Some((head, rest)) = free_front.split_last() {
            if !rest.is_empty() {
                p.yaw_chain(rest, 6.0 * step_cos);
                p.pitch_chain(rest, 2.0 * bob_cos);
            }
            p.yaw(*head, -4.0 * step_cos);
            p.pitch(*head, -2.5 * bob_cos);
        }
        for (i, &b) in free_back.iter().enumerate() {
            let k = free_back.len() as f64;
            p.yaw(b, 14.0 / k * detmath::sin(TAU * t - 0.4 * i as f64));
        }
        // Free arms counter-swing in opposite phase to their ipsilateral leg.
        for arm in &body.arms {
            let top = arm[0];
            let tip = arm.last().copied().unwrap_or(top);
            let arm_side = 0.5
                * (c.skeleton.bones[top].head.dot(left) + c.skeleton.bones[tip].tail.dot(left))
                - legs_mid;
            let leg_off = if arm_side >= 0.0 {
                offsets[0]
            } else {
                offsets[1]
            };
            let swing = -detmath::cos(TAU * (t + leg_off));
            p.add(top, axis_toward(c, top, c.forward, up), 20.0 * swing);
            if let Some(&elbow) = arm.get(1) {
                let elbow_deg = if arm.len() >= 4 {
                    10.0 * swing
                } else {
                    10.0 * swing.max(0.0) + 4.0 * swing
                };
                p.add(elbow, axis_toward(c, elbow, c.forward, up), elbow_deg);
            }
        }
        p.shift = left.scale(lat_at(t)) + up.scale(z_shift_at(t));
        let (mut quats, shift) = (p.quats, p.shift);
        for (rig, off) in rigs.iter().zip(offsets) {
            let phase = (t + off).fract();
            let (along, height) = if phase < duty {
                (half - stance * (phase / duty), 0.0)
            } else {
                let u = (phase - duty) / (1.0 - duty);
                let (u2, u3) = (u * u, u * u * u);
                let slope = -speed * swing_time;
                let x = (2.0 * u3 - 3.0 * u2 + 1.0) * -half
                    + (u3 - 2.0 * u2 + u) * slope
                    + (-2.0 * u3 + 3.0 * u2) * half
                    + (u3 - u2) * slope;
                (x, lift * 0.5 * (1.0 - detmath::cos(TAU * u)))
            };
            let contact = rig.contact + c.forward.scale(along) + up.scale(height);
            solve_biped_leg(c, rig, &mut quats, shift, contact);
        }
        out.push(make_frame(c, &quats, shift));
    }
    Ok(Stride {
        clip: Clip {
            fps,
            skeleton: c.skeleton.clone(),
            frames: out,
        },
        speed,
        period,
        duty,
        stance,
    })
}

fn walk_legless(c: &Creature, body: &Body, fps: f64) -> Result<Stride, String> {
    let mut chain: Vec<usize> = Vec::new();
    if !body.back.is_empty() {
        for &b in body.front.iter().rev() {
            chain.push(b);
        }
        for &b in &body.back {
            if !chain.contains(&b) {
                chain.push(b);
            }
        }
    } else if let (Some(&first), Some(&last)) = (body.front.first(), body.front.last()) {
        let first_fwd = c.skeleton.bones[first].head.dot(c.forward);
        let last_fwd = c.skeleton.bones[last].tail.dot(c.forward);
        if first_fwd > last_fwd + 1e-6 {
            chain.extend_from_slice(&body.front);
        } else {
            chain.extend(body.front.iter().rev().copied());
        }
    }
    let movable: usize = chain.iter().filter(|&&b| !body.held[b]).count();
    if chain.len() < 3 || movable < 2 {
        return Err(format!(
            "legless walk requires at least 3 body bones and 2 movable segments (found {} bones, {} movable)",
            chain.len(),
            movable
        ));
    }

    let up = Vec3::new(0.0, 0.0, 1.0);
    let left = left_of(c);
    let root = c.root();
    let speed = (0.6 * body.size).max(0.15);
    let target_period = (0.8 + 0.35 * body.size).clamp(0.75, 1.5);
    let frames = (target_period * fps).round().max(16.0) as usize;
    let period = frames as f64 / fps;
    let stance = speed * period;

    let wave_k = 0.9 * std::f64::consts::PI;
    let n = chain.len();
    let base_amp = (72.0 / n as f64).clamp(6.0, 18.0);

    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let t = f as f64 / frames as f64;
        let mut p = Poser::new(c);
        for (i, &b) in chain.iter().enumerate() {
            if body.held[b] {
                continue;
            }
            let s = i as f64 / (n - 1) as f64;
            let mut amp = base_amp * (0.3 + 0.9 * s);
            if b == root {
                amp = amp.min(14.0);
            }
            let angle = amp * detmath::sin(TAU * t - wave_k * s);
            let axis = axis_toward(c, b, left, up);
            p.add(b, axis, angle);
        }
        for arm in &body.arms {
            let top = arm[0];
            p.add(
                top,
                axis_toward(c, top, up, c.forward),
                20.0 * detmath::sin(TAU * t),
            );
            p.add(
                top,
                axis_toward(c, top, c.forward, up),
                10.0 * detmath::cos(TAU * t),
            );
            if let Some(&elbow) = arm.get(1) {
                p.add(
                    elbow,
                    axis_toward(c, elbow, up, c.forward),
                    12.0 * detmath::sin(TAU * t - 0.8),
                );
            }
        }
        out.push(p.frame());
    }
    Ok(Stride {
        clip: Clip {
            fps,
            skeleton: c.skeleton.clone(),
            frames: out,
        },
        speed,
        period,
        duty: 1.0,
        stance,
    })
}

/// One looping locomotion cycle on the spot for a non-quadruped creature:
/// alternating planted-foot walk for two-legged bodies (upright bipeds and
/// hunched long-armed monsters) or a traveling sinusoidal lateral S-wave for
/// legless bodies (serpents, wyrms, fish, cloaked spines).
pub fn walk_stride(c: &Creature, body: &Body, fps: f64) -> Result<Stride, String> {
    if !(fps > 0.0 && fps.is_finite()) {
        return Err("fps must be finite and > 0".into());
    }
    if body.legs.len() == 2 {
        walk_biped(c, body, fps)
    } else if body.floats || body.legs.is_empty() {
        walk_legless(c, body, fps)
    } else {
        Err(format!(
            "walk_stride supports two-legged or legless bodies; this body has {} legs",
            body.legs.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Bone, Frame, Pose, Skeleton};
    use crate::creature::rigcheck::{from_glb, Options};
    use crate::fixture::{humanoid, Naming};

    fn rest(n: usize) -> Frame {
        Frame {
            poses: vec![
                Pose {
                    loc: Vec3::ZERO,
                    quat: Quat::IDENTITY,
                };
                n
            ],
        }
    }

    fn turn_deg(q: Quat) -> f64 {
        (2.0 * detmath::acos(q.dot(Quat::IDENTITY).abs().min(1.0))).to_degrees()
    }

    fn biped() -> Creature {
        let doc = humanoid(Naming::Mixamo, false, false).unwrap();
        from_glb(
            &doc,
            &Options {
                forward: None,
                view_size: 64,
                name: "biped".into(),
            },
        )
        .unwrap()
        .creature
    }

    #[test]
    fn a_biped_reads_two_legs_two_arms_and_a_front_chain() {
        let c = biped();
        let b = read(&c);
        assert_eq!(b.legs.len(), 2, "{:?}", b);
        assert_eq!(b.arms.len(), 2, "{:?}", b);
        assert!(b.front.len() >= 3);
        // the hips hold the feet; the head doesn't
        assert!(b.held[b.front[0]]);
        assert!(!b.held[*b.front.last().unwrap()]);
    }

    #[test]
    fn the_clips_leave_planted_feet_where_they_are() {
        let c = biped();
        let b = read(&c);
        assert!(!c.feet.is_empty());
        let still = fk(&c.skeleton, &rest(c.skeleton.len()));
        for clip in [idle(&c, &b, 72, 24.0), attack(&c, &b, 24, 24.0)] {
            for frame in &clip.frames {
                let posed = fk(&c.skeleton, frame);
                for &foot in &c.feet {
                    let d = (posed[foot].head - still[foot].head).length();
                    assert!(d < 1e-9, "a foot moved {} m", d);
                    assert!(frame.poses[foot].quat.dot(Quat::IDENTITY).abs() > 1.0 - 1e-12);
                }
            }
        }
    }

    #[test]
    fn the_attack_turns_a_joint_far_and_starts_and_ends_at_rest() {
        let c = biped();
        let b = read(&c);
        let clip = attack(&c, &b, 24, 24.0);
        let first = &clip.frames[0];
        let last = &clip.frames[clip.frames.len() - 1];
        for (a, z) in first.poses.iter().zip(&last.poses) {
            assert!(turn_deg(a.quat) < 0.5 && turn_deg(z.quat) < 2.0);
        }
        let most = clip
            .frames
            .iter()
            .flat_map(|fr| fr.poses.iter())
            .map(|p| turn_deg(p.quat))
            .fold(0.0, f64::max);
        assert!(most > 30.0, "largest turn {}", most);
    }

    /// A made-up knuckle-walker: a spine that wanders 2-4% sideways, long
    /// arms whose hands stand on the ground, short legs, a head held to one
    /// side.
    fn knuckle_walker() -> Creature {
        let b = |name: &str, parent: Option<usize>, h: [f64; 3], t: [f64; 3]| Bone {
            name: name.into(),
            parent,
            head: Vec3::new(h[0], h[1], h[2]),
            tail: Vec3::new(t[0], t[1], t[2]),
        };
        let bones = vec![
            b("hips", None, [-0.03, 0.10, 0.30], [-0.02, 0.09, 0.33]),
            b("spine1", Some(0), [-0.02, 0.09, 0.42], [0.0, 0.07, 0.53]),
            b("spine2", Some(1), [0.0, 0.07, 0.53], [0.03, 0.05, 0.64]),
            b("chest", Some(2), [0.03, 0.05, 0.64], [0.01, 0.0, 0.74]),
            b("arm1.L", Some(3), [0.18, -0.02, 0.62], [0.29, -0.03, 0.47]),
            b("arm2.L", Some(4), [0.29, -0.03, 0.47], [0.30, -0.15, 0.20]),
            b("hand.L", Some(5), [0.30, -0.15, 0.20], [0.28, -0.25, 0.0]),
            b(
                "arm1.R",
                Some(3),
                [-0.14, -0.04, 0.66],
                [-0.25, -0.04, 0.50],
            ),
            b(
                "arm2.R",
                Some(7),
                [-0.25, -0.04, 0.50],
                [-0.30, -0.15, 0.20],
            ),
            b("hand.R", Some(8), [-0.30, -0.15, 0.20], [-0.30, -0.25, 0.0]),
            b("neck", Some(3), [0.01, -0.06, 0.80], [0.09, -0.10, 0.92]),
            b("head", Some(10), [0.09, -0.10, 0.92], [0.15, -0.12, 0.96]),
            b("leg1.L", Some(0), [0.08, 0.12, 0.27], [0.15, 0.13, 0.15]),
            b("leg2.L", Some(12), [0.15, 0.13, 0.15], [0.15, 0.10, 0.0]),
            b("leg1.R", Some(0), [-0.12, 0.05, 0.28], [-0.16, 0.0, 0.15]),
            b("leg2.R", Some(14), [-0.16, 0.0, 0.15], [-0.17, -0.05, 0.0]),
        ];
        let skeleton = Skeleton { bones };
        let (lo, hi) = skeleton_bounds(&skeleton);
        crate::creature::profile::build(
            "knuckles",
            skeleton,
            vec![6, 9, 13, 15],
            Vec3::new(0.0, -1.0, 0.0),
            lo,
            hi,
            64,
        )
    }

    #[test]
    fn a_wandering_spine_is_still_the_spine_and_knuckles_strike() {
        let c = knuckle_walker();
        let b = read(&c);
        let name = |i: usize| c.skeleton.bones[i].name.as_str();
        let front: Vec<&str> = b.front.iter().map(|&i| name(i)).collect();
        assert_eq!(
            front,
            ["hips", "spine1", "spine2", "chest", "neck", "head"],
            "{:?}",
            b
        );
        assert_eq!(b.legs.len(), 4, "{:?}", b);
        assert!(b.arms.is_empty());
        // the hands are feet, so the chest is held for standing ...
        assert!(b.held[3]);
        // ... the arms strike, and with them off the ground the chest is free
        let s = b.strikers(&c);
        let tops: Vec<&str> = s.iter().map(|l| name(l[0])).collect();
        assert_eq!(tops.len(), 2);
        assert!(
            tops.contains(&"arm1.L") && tops.contains(&"arm1.R"),
            "{:?}",
            tops
        );
        let held = b.held_without(&c, &s);
        assert!(!held[3] && !held[1] && held[0]);
        let clip = attack(&c, &b, 24, 24.0);
        let most = clip
            .frames
            .iter()
            .flat_map(|fr| fr.poses.iter())
            .map(|p| turn_deg(p.quat))
            .fold(0.0, f64::max);
        assert!(most > 30.0, "largest turn {}", most);
        // the legs it stands on stay put through the attack
        let still = fk(&c.skeleton, &rest(c.skeleton.len()));
        for frame in &clip.frames {
            let posed = fk(&c.skeleton, frame);
            for foot in [13, 15] {
                assert!((posed[foot].head - still[foot].head).length() < 1e-9);
            }
        }
    }

    #[test]
    fn the_idle_loops() {
        let c = biped();
        let b = read(&c);
        let clip = idle(&c, &b, 72, 24.0);
        // one step past the last frame is the first again: the change from
        // the last to the first is no bigger than from one frame to the next
        let step = |a: &Frame, z: &Frame| {
            a.poses
                .iter()
                .zip(&z.poses)
                .map(|(p, q)| 1.0 - p.quat.dot(q.quat).abs())
                .fold(0.0, f64::max)
        };
        let wrap = step(&clip.frames[71], &clip.frames[0]);
        let inner = step(&clip.frames[0], &clip.frames[1]);
        assert!(wrap <= inner * 1.5 + 1e-12, "wrap {} inner {}", wrap, inner);
    }

    fn foot_world_track(c: &Creature, s: &Stride, foot: usize) -> Vec<Vec3> {
        let len = c.skeleton.bone_length(foot);
        s.clip
            .frames
            .iter()
            .enumerate()
            .map(|(f, frame)| {
                let posed = fk(&c.skeleton, frame);
                let tail = posed[foot].head + posed[foot].rot.mul_vec(Vec3::new(0.0, len, 0.0));
                tail + c.forward.scale(s.speed * f as f64 / s.clip.fps)
            })
            .collect()
    }

    fn assert_bone_lengths_preserved(c: &Creature, clip: &Clip) {
        for frame in &clip.frames {
            let posed = fk(&c.skeleton, frame);
            for (i, bone) in c.skeleton.bones.iter().enumerate() {
                if let Some(p) = bone.parent {
                    let rest_d = (bone.head - c.skeleton.bones[p].head).length();
                    let now_d = (posed[i].head - posed[p].head).length();
                    assert!(
                        (rest_d - now_d).abs() < 1e-6,
                        "bone {} stretched: rest {} now {}",
                        bone.name,
                        rest_d,
                        now_d
                    );
                }
            }
        }
    }

    fn hunched_monster() -> Creature {
        let mk = |name: &str, parent: Option<usize>, h: [f64; 3], t: [f64; 3]| Bone {
            name: name.into(),
            parent,
            head: Vec3::new(h[0], h[1], h[2]),
            tail: Vec3::new(t[0], t[1], t[2]),
        };
        // Forward = -Y, left = +X, Z up. Hunched forward torso, 2 bent legs,
        // 2 long hanging arms (not feet), and a short tail.
        let bones = vec![
            mk("hips", None, [0.0, 0.0, 0.85], [0.0, -0.05, 0.95]),
            mk("spine1", Some(0), [0.0, -0.05, 0.95], [0.0, -0.18, 1.15]),
            mk("chest", Some(1), [0.0, -0.18, 1.15], [0.0, -0.32, 1.32]),
            mk("neck", Some(2), [0.0, -0.32, 1.32], [0.0, -0.44, 1.40]),
            mk("head", Some(3), [0.0, -0.44, 1.40], [0.0, -0.58, 1.42]),
            mk("tail0", Some(0), [0.0, 0.05, 0.84], [0.0, 0.25, 0.78]),
            mk("tail1", Some(5), [0.0, 0.25, 0.78], [0.0, 0.45, 0.70]),
            // Left & right legs (thigh -> shin -> foot), slightly bent knee
            mk("thigh.L", Some(0), [0.14, 0.0, 0.82], [0.14, -0.08, 0.44]),
            mk("shin.L", Some(7), [0.14, -0.08, 0.44], [0.14, 0.02, 0.08]),
            mk("foot.L", Some(8), [0.14, 0.02, 0.08], [0.14, -0.12, 0.0]),
            mk("thigh.R", Some(0), [-0.14, 0.0, 0.82], [-0.14, -0.08, 0.44]),
            mk(
                "shin.R",
                Some(10),
                [-0.14, -0.08, 0.44],
                [-0.14, 0.02, 0.08],
            ),
            mk("foot.R", Some(11), [-0.14, 0.02, 0.08], [-0.14, -0.12, 0.0]),
            // Long hanging arms (not in feet)
            mk("arm1.L", Some(2), [0.22, -0.28, 1.28], [0.26, -0.30, 0.84]),
            mk("arm2.L", Some(13), [0.26, -0.30, 0.84], [0.26, -0.36, 0.42]),
            mk("hand.L", Some(14), [0.26, -0.36, 0.42], [0.26, -0.40, 0.26]),
            mk(
                "arm1.R",
                Some(2),
                [-0.22, -0.28, 1.28],
                [-0.26, -0.30, 0.84],
            ),
            mk(
                "arm2.R",
                Some(16),
                [-0.26, -0.30, 0.84],
                [-0.26, -0.36, 0.42],
            ),
            mk(
                "hand.R",
                Some(17),
                [-0.26, -0.36, 0.42],
                [-0.26, -0.40, 0.26],
            ),
        ];
        let sk = Skeleton { bones };
        let (lo, hi) = skeleton_bounds(&sk);
        crate::creature::profile::build(
            "hunched_monster",
            sk,
            vec![9, 12],
            Vec3::new(0.0, -1.0, 0.0),
            lo,
            hi,
            64,
        )
    }

    fn serpent(with_fins: bool) -> Creature {
        let mk = |name: &str, parent: Option<usize>, h: [f64; 3], t: [f64; 3]| Bone {
            name: name.into(),
            parent,
            head: Vec3::new(h[0], h[1], h[2]),
            tail: Vec3::new(t[0], t[1], t[2]),
        };
        let mut bones = vec![
            mk("seg0", None, [0.0, -0.6, 0.3], [0.0, -0.4, 0.3]),
            mk("seg1", Some(0), [0.0, -0.4, 0.3], [0.0, -0.2, 0.3]),
            mk("seg2", Some(1), [0.0, -0.2, 0.3], [0.0, 0.0, 0.3]),
            mk("seg3", Some(2), [0.0, 0.0, 0.3], [0.0, 0.2, 0.3]),
            mk("seg4", Some(3), [0.0, 0.2, 0.3], [0.0, 0.4, 0.3]),
            mk("seg5", Some(4), [0.0, 0.4, 0.3], [0.0, 0.6, 0.3]),
        ];
        if with_fins {
            bones.extend([
                mk("fin1.L", Some(1), [0.05, -0.3, 0.3], [0.3, -0.25, 0.3]),
                mk("fin2.L", Some(6), [0.3, -0.25, 0.3], [0.5, -0.2, 0.3]),
                mk("fin1.R", Some(1), [-0.05, -0.3, 0.3], [-0.3, -0.25, 0.3]),
                mk("fin2.R", Some(8), [-0.3, -0.25, 0.3], [-0.5, -0.2, 0.3]),
            ]);
        }
        let sk = Skeleton { bones };
        let (lo, hi) = skeleton_bounds(&sk);
        crate::creature::profile::build(
            "serpent",
            sk,
            vec![],
            Vec3::new(0.0, -1.0, 0.0),
            lo,
            hi,
            64,
        )
    }

    #[test]
    fn biped_walk_plants_feet_clears_ground_and_preserves_bones() {
        let c = biped();
        let b = read(&c);
        let s = walk_stride(&c, &b, 30.0).unwrap();
        let n = s.clip.frames.len();
        assert!(n >= 16);
        assert!((s.period - n as f64 / 30.0).abs() < 1e-12);
        assert!(s.duty >= 0.60 && s.duty <= 0.65);
        assert!(s.speed > 0.1 && s.stance > 0.15 * c.height);

        let left = left_of(&c);
        let mut feet = c.feet.clone();
        feet.sort_by(|a, bb| {
            c.skeleton.bones[*bb]
                .tail
                .dot(left)
                .total_cmp(&c.skeleton.bones[*a].tail.dot(left))
        });
        let tracks: Vec<Vec<Vec3>> = feet.iter().map(|&f| foot_world_track(&c, &s, f)).collect();

        // Double support: never both feet airborne on the same frame.
        #[allow(clippy::needless_range_loop)]
        for f in 0..n {
            let any_down = feet.iter().enumerate().any(|(i, &foot)| {
                let rest_z = c.skeleton.bones[foot].tail.z;
                tracks[i][f].z <= rest_z + 0.005 * c.height
            });
            assert!(any_down, "both feet airborne on frame {f}");
        }

        for (i, (&foot, off)) in feet.iter().zip([0.0, 0.5]).enumerate() {
            let track = &tracks[i];
            let down: Vec<Vec3> = (0..n)
                .filter(|f| (*f as f64 / n as f64 + off).fract() < s.duty)
                .map(|f| {
                    let lap = ((f as f64 / n as f64 + off).floor()) * s.speed * s.period;
                    track[f] - c.forward.scale(lap)
                })
                .collect();
            assert!(down.len() >= 4);
            let mean = down
                .iter()
                .fold(Vec3::ZERO, |a, v| a + *v)
                .scale(1.0 / down.len() as f64);
            let slide = down
                .iter()
                .map(|d| (*d - mean).length())
                .fold(0.0, f64::max);
            assert!(
                slide < 0.005 * c.height && slide <= 0.025,
                "foot {i} skated {slide} m"
            );
            let rest_z = c.skeleton.bones[foot].tail.z;
            let top = track.iter().map(|v| v.z).fold(f64::NEG_INFINITY, f64::max);
            let low = track.iter().map(|v| v.z).fold(f64::INFINITY, f64::min);
            assert!(
                top > rest_z + 0.03 * c.height,
                "foot {i} lift {}",
                top - rest_z
            );
            assert!(
                low >= rest_z - 0.005 * c.height,
                "foot {i} low {}",
                low - rest_z
            );
        }
        assert_bone_lengths_preserved(&c, &s.clip);

        // Byte-deterministic across repeated calls.
        let s2 = walk_stride(&c, &b, 30.0).unwrap();
        for (a, z) in s.clip.frames.iter().zip(&s2.clip.frames) {
            for (pa, pz) in a.poses.iter().zip(&z.poses) {
                assert_eq!(pa.quat, pz.quat);
                assert_eq!(pa.loc, pz.loc);
            }
        }
    }

    #[test]
    fn hunched_long_armed_monster_walks_with_arm_counter_swing() {
        let c = hunched_monster();
        let b = read(&c);
        assert_eq!(b.legs.len(), 2);
        assert_eq!(b.arms.len(), 2);
        let s = walk_stride(&c, &b, 24.0).unwrap();
        let n = s.clip.frames.len();
        assert!(n >= 16);
        assert_bone_lengths_preserved(&c, &s.clip);

        // Planted feet & swing clearance.
        let left_foot = c.skeleton.index("foot.L").unwrap();
        let right_foot = c.skeleton.index("foot.R").unwrap();
        for (foot, off) in [(left_foot, 0.0), (right_foot, 0.5)] {
            let track = foot_world_track(&c, &s, foot);
            let down: Vec<Vec3> = (0..n)
                .filter(|f| (*f as f64 / n as f64 + off).fract() < s.duty)
                .map(|f| {
                    let lap = ((f as f64 / n as f64 + off).floor()) * s.speed * s.period;
                    track[f] - c.forward.scale(lap)
                })
                .collect();
            let mean = down
                .iter()
                .fold(Vec3::ZERO, |a, v| a + *v)
                .scale(1.0 / down.len() as f64);
            let slide = down
                .iter()
                .map(|d| (*d - mean).length())
                .fold(0.0, f64::max);
            assert!(slide < 0.005 * c.height, "monster foot slide {slide}");
            let rest_z = c.skeleton.bones[foot].tail.z;
            let top = track.iter().map(|v| v.z).fold(f64::NEG_INFINITY, f64::max);
            let low = track.iter().map(|v| v.z).fold(f64::INFINITY, f64::min);
            assert!(top > rest_z + 0.03 * c.height);
            assert!(low >= rest_z - 0.005 * c.height);
        }

        // Visible arm swing (> 10 deg) and anti-phase to ipsilateral leg.
        let arm_l = c.skeleton.index("arm1.L").unwrap();
        let hand_l = c.skeleton.index("hand.L").unwrap();
        let max_arm_deg = s
            .clip
            .frames
            .iter()
            .map(|fr| turn_deg(fr.poses[arm_l].quat))
            .fold(0.0, f64::max);
        assert!(max_arm_deg > 10.0, "arm swing {max_arm_deg} deg");

        let rest_foot_fwd = c.skeleton.bones[left_foot].tail.dot(c.forward);
        let rest_hand_fwd = c.skeleton.bones[hand_l].tail.dot(c.forward);
        let corr: f64 = s
            .clip
            .frames
            .iter()
            .map(|fr| {
                let posed = fk(&c.skeleton, fr);
                let foot_tail = crate::clip::tail_world(&c.skeleton, &posed, left_foot);
                let hand_tail = crate::clip::tail_world(&c.skeleton, &posed, hand_l);
                (foot_tail.dot(c.forward) - rest_foot_fwd)
                    * (hand_tail.dot(c.forward) - rest_hand_fwd)
            })
            .sum();
        assert!(
            corr < 0.0,
            "arm should counter-swing ipsilateral leg: {corr}"
        );
    }

    #[test]
    fn legless_serpent_produces_traveling_lateral_s_wave() {
        for with_fins in [false, true] {
            let c = serpent(with_fins);
            let b = read(&c);
            assert!(b.floats && b.legs.is_empty());
            let s = walk_stride(&c, &b, 24.0).unwrap();
            let n = s.clip.frames.len();
            assert!(n >= 16);
            assert!(s.speed > 0.1);
            assert!((s.period - n as f64 / 24.0).abs() < 1e-12);
            assert!((s.stance - s.speed * s.period).abs() < 1e-12);
            assert_bone_lengths_preserved(&c, &s.clip);

            let left = left_of(&c);
            // Clear lateral excursion (> 0.02 * body.size).
            let mut max_lat = 0.0_f64;
            let mut seg0_lat = Vec::with_capacity(n);
            let mut seg5_lat = Vec::with_capacity(n);
            for fr in &s.clip.frames {
                let posed = fk(&c.skeleton, fr);
                for i in 0..6 {
                    let tail = crate::clip::tail_world(&c.skeleton, &posed, i);
                    let lat = tail.dot(left);
                    max_lat = max_lat.max(lat.abs());
                    if i == 0 {
                        seg0_lat.push(lat);
                    }
                    if i == 5 {
                        seg5_lat.push(lat);
                    }
                }
            }
            assert!(
                max_lat > 0.02 * b.size,
                "lateral excursion {max_lat} vs size {}",
                b.size
            );

            // Traveling wave: tail peak lags head peak (not rigid in-phase wag).
            let peak_frame = |vals: &[f64]| -> usize {
                vals.iter()
                    .enumerate()
                    .max_by(|a, bb| a.1.total_cmp(bb.1))
                    .unwrap()
                    .0
            };
            let p0 = peak_frame(&seg0_lat);
            let p5 = peak_frame(&seg5_lat);
            let lag = (p5 + n - p0) % n;
            assert!(
                lag > 0 && lag < n / 2,
                "expected head-to-tail lag, got {lag}/{n}"
            );

            if with_fins {
                let fin_l = c.skeleton.index("fin1.L").unwrap();
                let fin_turn = s
                    .clip
                    .frames
                    .iter()
                    .map(|fr| turn_deg(fr.poses[fin_l].quat))
                    .fold(0.0, f64::max);
                assert!(fin_turn > 10.0, "fin turn {fin_turn}");
            }
        }
    }

    #[test]
    fn walk_stride_rejects_unsupported_and_degenerate_bodies() {
        // 4-legged body rejected by walk_stride (quadrupeds use gait::stride).
        let kw = knuckle_walker();
        assert!(walk_stride(&kw, &read(&kw), 24.0).is_err());

        // Single-legged body rejected.
        let mut one_leg = hunched_monster();
        one_leg.feet.truncate(1);
        assert!(walk_stride(&one_leg, &read(&one_leg), 24.0).is_err());

        // 1-bone and 2-bone legless skeletons rejected.
        for count in [1usize, 2] {
            let bones: Vec<Bone> = (0..count)
                .map(|i| Bone {
                    name: format!("b{i}"),
                    parent: if i == 0 { None } else { Some(i - 1) },
                    head: Vec3::new(0.0, i as f64 * 0.2, 0.2),
                    tail: Vec3::new(0.0, (i + 1) as f64 * 0.2, 0.2),
                })
                .collect();
            let sk = Skeleton { bones };
            let (lo, hi) = skeleton_bounds(&sk);
            let c = crate::creature::profile::build(
                "tiny",
                sk,
                vec![],
                Vec3::new(0.0, -1.0, 0.0),
                lo,
                hi,
                64,
            );
            assert!(walk_stride(&c, &read(&c), 24.0).is_err());
        }
    }

    #[test]
    fn asymmetric_arm_biped_keeps_spine_as_front_and_walks() {
        let mk = |name: &str, parent: Option<usize>, h: [f64; 3], t: [f64; 3]| Bone {
            name: name.into(),
            parent,
            head: Vec3::new(h[0], h[1], h[2]),
            tail: Vec3::new(t[0], t[1], t[2]),
        };
        // Forward = -Y, left = +X, Z up. Right arm reaches far sideways
        // (-0.42) while left arm stays tucked (+0.26), pulling global
        // min/max midpoint well off the hips (-0.01) and spine (~0.0).
        let bones = vec![
            mk("hips", None, [-0.01, 0.08, 0.35], [-0.01, 0.08, 0.36]),
            mk("spine1", Some(0), [0.00, 0.05, 0.46], [0.00, -0.04, 0.55]),
            mk("spine2", Some(1), [0.00, -0.04, 0.55], [0.01, -0.14, 0.62]),
            mk("chest", Some(2), [0.01, -0.14, 0.62], [0.00, -0.21, 0.61]),
            mk("arm1.L", Some(3), [0.12, -0.20, 0.59], [0.22, -0.16, 0.50]),
            mk("arm2.L", Some(4), [0.22, -0.16, 0.50], [0.26, -0.20, 0.20]),
            mk("hand.L", Some(5), [0.26, -0.20, 0.20], [0.24, -0.24, 0.08]),
            mk(
                "arm1.R",
                Some(3),
                [-0.12, -0.20, 0.61],
                [-0.26, -0.15, 0.54],
            ),
            mk(
                "arm2.R",
                Some(7),
                [-0.26, -0.15, 0.54],
                [-0.38, -0.17, 0.22],
            ),
            mk(
                "hand.R",
                Some(8),
                [-0.38, -0.17, 0.22],
                [-0.42, -0.22, 0.06],
            ),
            mk("tail1", Some(0), [0.00, 0.19, 0.35], [0.01, 0.34, 0.28]),
            mk("tail2", Some(10), [0.01, 0.34, 0.28], [0.00, 0.50, 0.14]),
            mk("thigh.L", Some(0), [0.09, 0.03, 0.29], [0.14, 0.04, 0.19]),
            mk("shin.L", Some(12), [0.14, 0.04, 0.19], [0.16, 0.16, 0.05]),
            mk("foot.L", Some(13), [0.16, 0.16, 0.05], [0.18, 0.12, 0.00]),
            mk("thigh.R", Some(0), [-0.11, 0.04, 0.30], [-0.16, 0.05, 0.20]),
            mk("shin.R", Some(15), [-0.16, 0.05, 0.20], [-0.18, 0.16, 0.04]),
            mk("foot.R", Some(16), [-0.18, 0.16, 0.04], [-0.20, 0.11, 0.00]),
        ];
        let sk = Skeleton { bones };
        let (lo, hi) = skeleton_bounds(&sk);
        let c = crate::creature::profile::build(
            "asym_biped",
            sk,
            vec![14, 17],
            Vec3::new(0.0, -1.0, 0.0),
            lo,
            hi,
            64,
        );
        let b = read(&c);
        let names = |ids: &[usize]| -> Vec<&str> {
            ids.iter()
                .map(|&i| c.skeleton.bones[i].name.as_str())
                .collect()
        };
        assert_eq!(names(&b.front), ["hips", "spine1", "spine2", "chest"]);
        assert_eq!(names(&b.back), ["tail1", "tail2"]);
        assert_eq!(b.legs.len(), 2, "{:?}", b);
        assert_eq!(b.arms.len(), 2, "{:?}", b);

        let stride = walk_stride(&c, &b, 24.0).expect("asymmetric biped should walk");
        assert!(stride.clip.frames.len() >= 16);
        assert_bone_lengths_preserved(&c, &stride.clip);
        let atk = attack(&c, &b, 24, 24.0);
        let most = atk
            .frames
            .iter()
            .flat_map(|fr| fr.poses.iter())
            .map(|p| turn_deg(p.quat))
            .fold(0.0, f64::max);
        assert!(most > 25.0, "expected active attack swing, got {most}");
    }

    #[test]
    fn off_center_root_never_lets_foot_leg_hijack_front_chain() {
        let mk = |name: &str, parent: Option<usize>, h: [f64; 3], t: [f64; 3]| Bone {
            name: name.into(),
            parent,
            head: Vec3::new(h[0], h[1], h[2]),
            tail: Vec3::new(t[0], t[1], t[2]),
        };
        // Root sits off-center (+0.11); one downward leg sits closer to x = 0
        // than the spine, and the spine leans slightly (+0.04) opposite a
        // leg (-0.04) relative to hips. Neither may pair nor hijack `front`.
        let bones = vec![
            mk("hips", None, [0.11, 0.02, 0.48], [0.11, 0.00, 0.52]),
            mk("spine1", Some(0), [0.15, 0.00, 0.58], [0.13, -0.06, 0.70]),
            mk("spine2", Some(1), [0.13, -0.06, 0.70], [0.11, -0.14, 0.80]),
            mk("chest", Some(2), [0.11, -0.14, 0.80], [0.11, -0.22, 0.86]),
            mk("arm1.L", Some(3), [0.28, -0.16, 0.80], [0.36, -0.20, 0.52]),
            mk("arm2.L", Some(4), [0.36, -0.20, 0.52], [0.38, -0.28, 0.24]),
            mk(
                "arm1.R",
                Some(3),
                [-0.08, -0.16, 0.78],
                [-0.20, -0.20, 0.50],
            ),
            mk(
                "arm2.R",
                Some(6),
                [-0.20, -0.20, 0.50],
                [-0.26, -0.26, 0.22],
            ),
            mk("thigh.L", Some(0), [0.23, 0.02, 0.42], [0.24, 0.08, 0.22]),
            mk("shin.L", Some(8), [0.24, 0.08, 0.22], [0.24, 0.04, 0.06]),
            mk("foot.L", Some(9), [0.24, 0.04, 0.06], [0.24, -0.04, 0.00]),
            mk("thigh.R", Some(0), [0.07, 0.00, 0.38], [0.02, 0.06, 0.20]),
            mk("shin.R", Some(11), [0.02, 0.06, 0.20], [0.01, 0.04, 0.05]),
            mk("foot.R", Some(12), [0.01, 0.04, 0.05], [0.01, -0.04, 0.00]),
        ];
        let sk = Skeleton { bones };
        let (lo, hi) = skeleton_bounds(&sk);
        let c = crate::creature::profile::build(
            "off_center_biped",
            sk,
            vec![10, 13],
            Vec3::new(0.0, -1.0, 0.0),
            lo,
            hi,
            64,
        );
        let b = read(&c);
        let names = |ids: &[usize]| -> Vec<&str> {
            ids.iter()
                .map(|&i| c.skeleton.bones[i].name.as_str())
                .collect()
        };
        assert_eq!(names(&b.front), ["hips", "spine1", "spine2", "chest"]);
        assert_eq!(b.legs.len(), 2, "{:?}", b);
        assert_eq!(b.arms.len(), 2, "{:?}", b);

        let stride = walk_stride(&c, &b, 24.0).expect("off-center biped should walk");
        assert!(stride.clip.frames.len() >= 16);
        assert_bone_lengths_preserved(&c, &stride.clip);
        let atk = attack(&c, &b, 24, 24.0);
        assert_bone_lengths_preserved(&c, &atk);
    }
}
