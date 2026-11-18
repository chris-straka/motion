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

use super::gait::{chain_toward, children, left_of, smooth, Poser};
use super::profile::{skeleton_bounds, Creature};
use crate::clip::Clip;
use crate::detmath;
use crate::math::Vec3;

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
    /// What strikes in the attack: the free side chains; on a body with
    /// none and four legs or more, the pair of legs that leaves the body
    /// highest and farthest forward (a knuckle-walker's arms, a spider's
    /// front pair), lifted off the ground; it stands on the others.
    pub fn strikers(&self, c: &Creature) -> Vec<Vec<usize>> {
        if !self.arms.is_empty() || self.legs.len() < 4 {
            return self.arms.clone();
        }
        let up = Vec3::new(0.0, 0.0, 1.0);
        let at = |leg: &Vec<usize>| {
            let b = &c.skeleton.bones[leg[0]];
            let p = b.parent.map_or(b.head, |p| c.skeleton.bones[p].head);
            p.dot(up) + p.dot(c.forward)
        };
        let mut legs = self.legs.clone();
        legs.sort_by(|a, b| at(b).total_cmp(&at(a)).then(a[0].cmp(&b[0])));
        legs.truncate(2);
        legs
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
    let sides: Vec<f64> = c
        .skeleton
        .bones
        .iter()
        .flat_map(|b| [b.head.dot(left), b.tail.dot(left)])
        .collect();
    let mid = (sides.iter().cloned().fold(f64::INFINITY, f64::min)
        + sides.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        * 0.5;
    // Off the mirror plane, by its head. The centre line is followed by
    // structure, not by a band: at each bone the child nearest the plane
    // goes on (a generated body's spine wanders a few percent sideways;
    // its limbs leave it in pairs, well out).
    let side = |b: usize| c.skeleton.bones[b].head.dot(left) - mid;
    let off = |b: usize| side(b).abs();
    // A child that has a partner on the other side, about as far out, is
    // one of a pair (legs, arms, eyes): never the centre line.
    let paired = |k: usize, among: &[usize]| {
        off(k) > 0.03 * size
            && among.iter().any(|&o| {
                o != k && side(o) * side(k) < 0.0 && (off(o) - off(k)).abs() <= 0.03 * size
            })
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
                .filter(|k| {
                    !skip.contains(k) && (all.len() == 1 || (off(*k) <= band && !paired(*k, &all)))
                })
                .collect();
            if kids.is_empty() {
                break;
            }
            let least = kids.iter().map(|&k| off(k)).fold(f64::INFINITY, f64::min);
            let next = kids
                .iter()
                .copied()
                .filter(|&k| off(k) <= least + 0.03 * size)
                .max_by(|a, b| reach(*a, dir).total_cmp(&reach(*b, dir)).then(b.cmp(a)))
                .unwrap();
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
    let root = c.root();
    let front_dir = (c.forward + up).normalized();
    let front = centre_chain(root, front_dir, &[]);
    // The tail: the centre branch reaching farthest back from the front
    // chain's first third (the hips and what is next to them).
    let back_dir = (c.forward.scale(-1.0) + up.scale(-0.3)).normalized();
    let mut back: Vec<usize> = Vec::new();
    let mut best = f64::NEG_INFINITY;
    for &from in front.iter().take(((front.len() + 2) / 3).max(1)) {
        let base = c.skeleton.bones[from].head.dot(back_dir);
        let all = children(c, from);
        let least = all.iter().map(|&k| off(k)).fold(f64::INFINITY, f64::min);
        for k in all.iter().copied() {
            if front.contains(&k)
                || off(k) > band
                || off(k) > least + 0.03 * size
                || paired(k, &all)
            {
                continue;
            }
            let ch = centre_chain(k, back_dir, &front);
            let tip = c.skeleton.bones[*ch.last().unwrap()].tail.dot(back_dir);
            if tip > base + 0.05 * size && tip > best {
                best = tip;
                back = ch;
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
        if !front.is_empty() {
            let (head, rest) = front.split_last().unwrap();
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
        if !front.is_empty() {
            let (head, rest) = front.split_last().unwrap();
            if !rest.is_empty() {
                p.pitch_chain(rest, 18.0 * rear - 32.0 * strike);
            }
            p.pitch(*head, 12.0 * rear - 18.0 * strike);
        }
        if !back.is_empty() {
            // the tail goes against the body: down as it rears, up as it strikes
            p.pitch_chain(&back, -8.0 * rear + 14.0 * strike);
        }
        for arm in &strikers {
            let top = arm[0];
            // Every striking limb swings in its own side's upright plane
            // (about the left-right axis), so it never crosses the body:
            // drawn back as the body rears, then swung through.
            p.pitch(top, -30.0 * rear + 60.0 * strike);
            if let Some(&elbow) = arm.get(1) {
                p.pitch(elbow, 25.0 * rear - 10.0 * strike);
            }
            // A limb held out sideways (a wing, a fin) also beats: up as the
            // body rears, down through the strike.
            let tip = c.skeleton.bones[*arm.last().unwrap()].tail;
            let reach = (tip - c.skeleton.bones[top].head).normalized();
            if reach.dot(left).abs() > 0.55 {
                p.add(
                    top,
                    axis_toward(c, top, up, c.forward),
                    30.0 * rear - 40.0 * strike,
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
        let tip = c.skeleton.bones[*limb.last().unwrap()].tail;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{fk, Frame, Pose};
    use crate::creature::rigcheck::{from_glb, Options};
    use crate::fixture::{humanoid, Naming};
    use crate::math::Quat;

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
        use crate::clip::{Bone, Skeleton};
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
}
