//! Two-bone IK on plain points: mirrors a two-bone soft-reach runtime IK
//! solver in f64 with `detmath`, so an authored pose is solved the way the
//! engine solves reaches on top of it. The limb bends in the plane of its root, the target and a pole; the
//! middle joint points toward the pole.
//!
//! [`set_limb`] turns the answer into bone rotations as a true hinge: the
//! upper bone is rolled about itself until its anatomical hinge axis
//! (`contact::hinge_axis`) is square to the bend plane, and the lower bone
//! is its rest pose on the upper one turned about that axis alone. (The
//! game's `limb_turns` lays each bone along the answer the shortest way;
//! baked into a clip that left elbows bent out of their hinge plane.)

use crate::detmath;
use crate::keypose::fit::set_world_rot;
use crate::math::{Quat, Vec3};
use crate::rig::{Rig, Trs};

/// How far a hinge may bend, radians (0 = straight): the game's
/// `Hinge::LIMB`.
pub const MIN_BEND: f64 = 0.02;
pub const MAX_BEND: f64 = 2.6;

#[derive(Clone, Copy, Debug)]
pub struct TwoBone {
    pub mid: Vec3,
    pub end: Vec3,
    /// How far the end is from the target.
    pub miss: f64,
}

/// The reach a target at `d` gets under the soft limit: straight through
/// up to `full - soft`, then easing in so it only nears `full` far out.
pub fn soft_reach(d: f64, full: f64, soft: f64) -> f64 {
    if soft <= 0.0 {
        return d.min(full);
    }
    let start = full - soft;
    if d <= start {
        d
    } else {
        start + soft * (1.0 - detmath::exp(-(d - start) / soft))
    }
}

fn any_perp(v: Vec3) -> Vec3 {
    let other = if v.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    v.cross(other).normalized()
}

/// Solve a limb from `a` (its root, fixed) through `b` to `c` for
/// `target`; the middle joint points toward `pole`.
pub fn two_bone(a: Vec3, b: Vec3, c: Vec3, target: Vec3, pole: Vec3, soft: f64) -> TwoBone {
    let l1 = (b - a).length().max(1e-9);
    let l2 = (c - b).length().max(1e-9);
    let to = target - a;
    let dist = to.length();
    let dir = if dist > 1e-9 {
        to.scale(1.0 / dist)
    } else if (c - a).length() > 1e-9 {
        (c - a).normalized()
    } else {
        Vec3::new(0.0, -1.0, 0.0)
    };
    let mut d = soft_reach(dist, l1 + l2, soft).max((l1 - l2).abs() + 1e-6);
    let cos_mid = ((l1 * l1 + l2 * l2 - d * d) / (2.0 * l1 * l2)).clamp(-1.0, 1.0);
    let bend = std::f64::consts::PI - detmath::acos(cos_mid);
    let held = bend.clamp(MIN_BEND, MAX_BEND);
    if (held - bend).abs() > 1e-9 {
        d = (l1 * l1 + l2 * l2 + 2.0 * l1 * l2 * detmath::cos(held))
            .max(0.0)
            .sqrt();
    }
    let cos_root = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d.max(1e-9))).clamp(-1.0, 1.0);
    let sin_root = (1.0 - cos_root * cos_root).max(0.0).sqrt();
    let side = |p: Vec3| {
        let v = p - a;
        v - dir.scale(v.dot(dir))
    };
    let mut bend_dir = side(pole);
    if bend_dir.length() < 1e-6 {
        bend_dir = side(b);
    }
    let bend_dir = if bend_dir.length() < 1e-6 {
        any_perp(dir)
    } else {
        bend_dir.normalized()
    };
    let mid = a + (dir.scale(cos_root) + bend_dir.scale(sin_root)).scale(l1);
    let end = a + dir.scale(d);
    TwoBone {
        mid,
        end,
        miss: (end - target).length(),
    }
}

/// Shortest-arc quaternion taking unit `from` onto unit `to` with full
/// precision down to tiny angles (unlike `math::shortest_arc`, which snaps
/// angles below ~1.4e-6 rad to identity).
fn exact_shortest_arc(from: Vec3, to: Vec3) -> Quat {
    let d = from.dot(to);
    if d <= -1.0 + 1e-12 {
        let p = any_perp(from);
        return Quat::new(0.0, p.x, p.y, p.z);
    }
    let k = (from.length_sq() * to.length_sq()).sqrt() + d;
    let v = from.cross(to);
    Quat::new(k, v.x, v.y, v.z).normalized()
}

/// Rotation about unit `axis` taking `a`'s projection onto the plane
/// normal to `axis` onto `b`'s projection (half-angle form, no `acos`
/// precision loss near 0 or pi).
fn rot_about_axis(a: Vec3, b: Vec3, axis: Vec3) -> Quat {
    let ap = a - axis.scale(a.dot(axis));
    let bp = b - axis.scale(b.dot(axis));
    let la = ap.length();
    let lb = bp.length();
    if la < 1e-12 || lb < 1e-12 {
        return Quat::IDENTITY;
    }
    let au = ap.scale(1.0 / la);
    let bu = bp.scale(1.0 / lb);
    let c = au.dot(bu).clamp(-1.0, 1.0);
    let s = au.cross(bu).dot(axis);
    if c >= 0.0 {
        let k = 1.0 + c;
        Quat::new(k, axis.x * s, axis.y * s, axis.z * s).normalized()
    } else {
        let k = 1.0 - c;
        if s >= 0.0 {
            Quat::new(s, axis.x * k, axis.y * k, axis.z * k).normalized()
        } else {
            Quat::new(-s, -axis.x * k, -axis.y * k, -axis.z * k).normalized()
        }
    }
}

/// Write a [`two_bone`] answer onto the limb's bones as a hinge (module
/// doc). `axis_rest` is the limb's rest hinge axis in the world
/// (`contact::hinge_axis`); without one each bone turns the shortest way.
/// The end bone's rotation is the caller's to set.
pub fn set_limb(
    rig: &Rig,
    locals: &mut [Option<Trs>],
    upper: usize,
    lower: usize,
    end: usize,
    sol: &TwoBone,
    axis_rest: Option<Vec3>,
) {
    let world = rig.world(locals);
    let a = world[upper].t;
    let u_cur = (world[lower].t - a).normalized();
    let u_new = (sol.mid - a).normalized();
    let v_new = (sol.end - sol.mid).normalized();
    let mut ru = exact_shortest_arc(u_cur, u_new).mul(world[upper].rotation());
    let ru_rest = rig.rest_world[upper].rotation();
    let rl_rest = rig.rest_world[lower].rotation();
    let normal = u_new.cross(v_new);
    let hinge = match axis_rest {
        Some(ax) if normal.length() > 1e-4 => {
            let n = normal.normalized();
            // Roll the upper bone about itself: its hinge axis onto the
            // bend plane's normal.
            let h = ru.mul(ru_rest.conj()).rotate_vec(ax);
            ru = rot_about_axis(h, n, u_new).mul(ru);
            Some(n)
        }
        _ => None,
    };
    let ru = ru.normalized();
    set_world_rot(rig, locals, upper, ru);
    let rl = match hinge {
        Some(n) => {
            // The lower bone as it rests on the upper one, turned about the
            // hinge alone.
            let base = ru.mul(ru_rest.conj()).mul(rl_rest);
            let v_rest = rig.head(end) - rig.head(lower);
            let v_base = base.mul(rl_rest.conj()).rotate_vec(v_rest).normalized();
            rot_about_axis(v_base, v_new, n).mul(base)
        }
        None => {
            let w = rig.world(locals);
            let v_cur = (w[end].t - w[lower].t).normalized();
            exact_shortest_arc(v_cur, v_new).mul(w[lower].rotation())
        }
    };
    set_world_rot(rig, locals, lower, rl.normalized());
}

/// Solve and write a limb so its end lands on `target`: [`two_bone`] and
/// [`set_limb`], then aimed again past what the hinge left over (a limb
/// whose bones do not rest exactly in their hinge plane lands a few
/// millimetres off; a planted foot must not). Returns the end's distance
/// from the target.
#[allow(clippy::too_many_arguments)]
pub fn reach(
    rig: &Rig,
    locals: &mut [Option<Trs>],
    upper: usize,
    lower: usize,
    end: usize,
    target: Vec3,
    pole: Vec3,
    axis_rest: Option<Vec3>,
) -> f64 {
    let mut aim = target;
    let mut best_left = f64::INFINITY;
    let mut best_u = locals[upper];
    let mut best_l = locals[lower];
    for _ in 0..16 {
        let w = rig.world(locals);
        let sol = two_bone(w[upper].t, w[lower].t, w[end].t, aim, pole, 0.0);
        set_limb(rig, locals, upper, lower, end, &sol, axis_rest);
        let got = rig.world(locals)[end].t;
        let left = (got - target).length();
        if left < best_left {
            best_left = left;
            best_u = locals[upper];
            best_l = locals[lower];
        } else {
            locals[upper] = best_u;
            locals[lower] = best_l;
            break;
        }
        // Out of reach (the solve itself missed): nothing to correct.
        if left < 1e-12 || sol.miss > 1e-9 {
            break;
        }
        aim = aim + (target - got);
    }
    best_left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaches_a_target_inside_its_range_with_the_knee_at_the_pole() {
        let a = Vec3::new(0.0, 1.0, 0.0);
        let b = Vec3::new(0.0, 0.5, 0.0);
        let c = Vec3::new(0.0, 0.0, 0.0);
        let target = Vec3::new(0.0, 0.3, 0.2);
        let pole = Vec3::new(0.0, 0.6, 1.0);
        let s = two_bone(a, b, c, target, pole, 0.0);
        assert!(s.miss < 1e-9, "{s:?}");
        assert!(((s.mid - a).length() - 0.5).abs() < 1e-9);
        assert!(((s.end - s.mid).length() - 0.5).abs() < 1e-9);
        // The knee is on the pole's side of the hip-target line.
        assert!(s.mid.z > 0.1, "{s:?}");
        // Flip the pole: the knee flips.
        let s2 = two_bone(a, b, c, target, Vec3::new(0.0, 0.6, -1.0), 0.0);
        assert!(s2.mid.z < 0.0 && s2.miss < 1e-9, "{s2:?}");
    }

    #[test]
    fn a_target_out_of_reach_is_missed_by_the_overshoot_never_stretched() {
        let a = Vec3::new(0.0, 1.0, 0.0);
        let b = Vec3::new(0.0, 0.5, 0.0);
        let c = Vec3::new(0.0, 0.0, 0.0);
        let s = two_bone(
            a,
            b,
            c,
            Vec3::new(0.0, 1.0, 1.5),
            Vec3::new(0.0, 2.0, 0.5),
            0.0,
        );
        // Never quite straight (the least bend), so a hair under 1.0.
        let reach = (s.end - a).length();
        assert!(reach < 1.0 && reach > 0.999, "{reach}");
        assert!((s.miss - (1.5 - reach)).abs() < 1e-9);
        // The soft limit eases the last of the reach.
        assert!(soft_reach(0.95, 1.0, 0.1) < 0.95);
        assert!((soft_reach(0.5, 1.0, 0.1) - 0.5).abs() < 1e-12);
        assert!(soft_reach(5.0, 1.0, 0.1) <= 1.0 && soft_reach(1.05, 1.0, 0.1) < 1.0);
    }

    #[test]
    fn exact_rotations_have_no_small_angle_deadzone() {
        let axis = Vec3::new(0.3, -0.7, 0.6).normalized();
        let start = any_perp(axis);
        for ang in [
            0.0,
            1e-10,
            1e-7,
            1e-6,
            -1e-7,
            0.5,
            -0.5,
            std::f64::consts::FRAC_PI_2,
            2.5,
            -2.5,
            std::f64::consts::PI,
        ] {
            let want = Quat::from_axis_angle(axis, ang).rotate_vec(start);
            let q_arc = exact_shortest_arc(start, want);
            assert!(
                (q_arc.rotate_vec(start) - want).length() < 1e-12,
                "exact_shortest_arc failed at angle {ang}"
            );
            let q_ax = rot_about_axis(start, want, axis);
            assert!(
                (q_ax.rotate_vec(start) - want).length() < 1e-12,
                "rot_about_axis failed at angle {ang}"
            );
        }
    }

    #[test]
    fn reach_converges_on_small_refinement_and_off_plane_rest() {
        use crate::contact::hinge_axis;
        use crate::fixture::{humanoid_mitten, Naming};
        use crate::glb::{node_trs, set_node_trs, Document};
        use crate::keypose::fit::RigInfo;
        use crate::standardize::{standardize, Outcome};

        let raw = humanoid_mitten(Naming::Plain, true, 75.0).unwrap();
        let doc = match standardize(&raw, "humanoid").unwrap() {
            Outcome::Done(d, _) => Document::parse(&d.to_bytes().unwrap()).unwrap(),
            Outcome::Refused(_, why) => panic!("{why}"),
        };
        let info = RigInfo::new(&doc).unwrap();
        let (up, fo, ha) = (info.upper[1], info.fore[1], info.hand[1]);
        let ax = hinge_axis(&info.rig, up, fo, ha);
        let mut locals = vec![None; info.rig.names.len()];
        let sh = info.rig.head(up);
        let target = sh + Vec3::new(-0.15, -0.25, 0.20);
        let pole = sh + Vec3::new(-0.30, -0.50, -0.20);
        let m1 = reach(&info.rig, &mut locals, up, fo, ha, target, pole, ax);
        assert!(m1 < 1e-11, "initial reach miss {m1}");
        // Sub-micrometre refinement sits inside math::shortest_arc's 1.4e-6 rad
        // deadzone; reach must still land to < 1e-11.
        let target2 = target + Vec3::new(3e-8, -2e-8, 3.5e-8);
        let m2 = reach(&info.rig, &mut locals, up, fo, ha, target2, pole, ax);
        assert!(m2 < 1e-11, "small refinement miss {m2}");

        // Forearm resting off its hinge plane forces multi-step convergence.
        let mut off_doc = doc.clone();
        let (t, r, s) = node_trs(&off_doc.array("nodes")[ha]);
        let off_local = info.rig.rest_world[fo]
            .rotation()
            .conj()
            .rotate_vec(ax.unwrap().scale(0.015));
        set_node_trs(&mut off_doc.array_mut("nodes")[ha], t + off_local, r, s);
        let off_info = RigInfo::new(&off_doc).unwrap();
        let mut off_locals = vec![None; off_info.rig.names.len()];
        let off_ax = hinge_axis(&off_info.rig, up, fo, ha);
        let m3 = reach(
            &off_info.rig,
            &mut off_locals,
            up,
            fo,
            ha,
            target,
            pole,
            off_ax,
        );
        assert!(m3 < 1e-10, "off-plane rest reach miss {m3}");
    }
}
