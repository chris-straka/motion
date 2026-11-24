//! Analytical two-bone arm IK preserving shoulder position and elbow hinge.

use crate::glb::canonical_quat;
use crate::math::{shortest_arc, Quat, Vec3};
use crate::rig::{Rig, Trs};

/// Two-bone IK on one arm: move the wrist to `target`, keep the shoulder,
/// the elbow's bend plane and the hand's world rotation.
pub fn solve_arm(
    rig: &Rig,
    locals: &mut [Option<Trs>],
    upper: usize,
    fore: usize,
    hand: usize,
    target: Vec3,
) {
    solve_arm_turn(rig, locals, upper, fore, hand, target, Quat::IDENTITY);
}

/// [`solve_arm`] with the hand's world rotation turned by `turn` (a held
/// blade swung clear of the body about the wrist).
#[allow(clippy::too_many_arguments)]
pub fn solve_arm_turn(
    rig: &Rig,
    locals: &mut [Option<Trs>],
    upper: usize,
    fore: usize,
    hand: usize,
    target: Vec3,
    turn: Quat,
) {
    let world = rig.world(locals);
    let n = world.len();
    if upper >= n
        || fore >= n
        || hand >= n
        || upper >= locals.len()
        || fore >= locals.len()
        || hand >= locals.len()
    {
        return;
    }
    let (s, e, w) = (world[upper].t, world[fore].t, world[hand].t);
    let a = (e - s).length();
    let b = (w - e).length();
    if a < 1e-9 || b < 1e-9 {
        return;
    }
    let ru_old = world[upper].rotation();
    let rf_old = world[fore].rotation();
    let rh_old = turn.mul(world[hand].rotation());
    let goal = target - s;
    if goal.length() < 1e-9 {
        return;
    }
    let reach = goal
        .length()
        .clamp((a - b).abs() + 1e-6, a + b - 1e-6 * (a + b));
    let u = e - s;
    let v = w - e;
    // The hinge axis, fixed in the upper bone: the anatomical one
    // ([`hinge_axis`]; a nearly straight limb's own cross product is noise,
    // and a hinge read from it flipped the elbow between frames), else the
    // current bend when that is clear.
    let rest_axis = hinge_axis(rig, upper, fore, hand)
        .map(|n| ru_old.rotate_vec(rig.rest_world[upper].rotation().conj().rotate_vec(n)));
    let bend = u.cross(v);
    let axis = rest_axis.or(if bend.length() > 0.14 * a * b {
        Some(bend.normalized())
    } else {
        None
    });
    let (ru_new, rf_new) = if let Some(axis) = axis {
        // A hinge: turn the lower bone about the axis until the limb's
        // reach is right, keeping the bend's side, then swing the whole
        // limb from the shoulder onto the target. The elbow never leaves
        // its hinge plane (solving in the plane through shoulder, old
        // elbow and target, as this used to, bent it sideways whenever the
        // target moved off the limb's own plane).
        let (u_par, v_par) = (axis.scale(u.dot(axis)), axis.scale(v.dot(axis)));
        let (u_perp, v_perp) = (u - u_par, v - v_par);
        let (lu, lv) = (u_perp.length(), v_perp.length());
        let qb = if lu > 1e-9 && lv > 1e-9 {
            let c = (u_perp.dot(v_perp) / (lu * lv)).clamp(-1.0, 1.0);
            let sign = if u_perp.cross(v_perp).dot(axis) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            let alpha = sign * crate::detmath::acos(c);
            let k = ((reach * reach - a * a - b * b) / 2.0 - u_par.dot(v_par)) / (lu * lv);
            let target_angle = sign * crate::detmath::acos(k.clamp(-1.0, 1.0));
            Quat::from_axis_angle(axis, target_angle - alpha)
        } else {
            Quat::IDENTITY
        };
        let w_bent = s + u + qb.rotate_vec(v);
        let qs = shortest_arc((w_bent - s).normalized(), goal.normalized());
        (qs.mul(ru_old), qs.mul(qb).mul(rf_old))
    } else {
        // Straight limb: no hinge to read. Bend in the plane of the
        // current elbow's offset from the shoulder-target line.
        let to = goal.normalized();
        let t = s + to.scale(reach);
        let ce = e - s;
        let mut pole = ce - to.scale(ce.dot(to));
        if pole.length() < 1e-6 * a {
            let cw = (w - e).normalized();
            pole = cw.scale(-1.0) - to.scale(-cw.dot(to));
            if pole.length() < 1e-9 {
                return;
            }
        }
        let pole = pole.normalized();
        let cos_a = ((a * a + reach * reach - b * b) / (2.0 * a * reach)).clamp(-1.0, 1.0);
        let ang = crate::detmath::acos(cos_a);
        let (sa, ca) = crate::detmath::sin_cos(ang);
        let e2 = s + to.scale(a * ca) + pole.scale(a * sa);
        let q1 = shortest_arc((e - s).normalized(), (e2 - s).normalized());
        let fdir = q1.rotate_vec((w - e).normalized());
        let q2 = shortest_arc(fdir, (t - e2).normalized());
        (q1.mul(ru_old), q2.mul(q1).mul(rf_old))
    };
    let parent_rot = rig.parent[upper]
        .map(|p| world[p].rotation())
        .unwrap_or(Quat::IDENTITY);
    let mut lu = locals[upper].unwrap_or(rig.rest[upper]);
    lu.r = canonical_quat(parent_rot.conj().mul(ru_new));
    locals[upper] = Some(lu);
    // The forearm's parent is the upper arm (anatomical chain); fall back
    // to whatever its parent is.
    let fore_parent = rig.parent[fore];
    let fp_rot = if fore_parent == Some(upper) {
        ru_new
    } else {
        fore_parent
            .map(|p| world[p].rotation())
            .unwrap_or(Quat::IDENTITY)
    };
    let mut lf = locals[fore].unwrap_or(rig.rest[fore]);
    lf.r = canonical_quat(fp_rot.conj().mul(rf_new));
    locals[fore] = Some(lf);
    let hand_parent = rig.parent[hand];
    let hp_rot = if hand_parent == Some(fore) {
        rf_new
    } else {
        hand_parent
            .map(|p| world[p].rotation())
            .unwrap_or(Quat::IDENTITY)
    };
    let mut lh = locals[hand].unwrap_or(rig.rest[hand]);
    lh.r = canonical_quat(hp_rot.conj().mul(rh_old));
    locals[hand] = Some(lh);
}

/// Rest-pose hinge axis of a limb (world), oriented so a positive turn of
/// the lower bone about it is the natural bend (knee: shin back; elbow:
/// forearm forward): `upper x facing`, the facing read from the feet
/// (heel to toe) of a standard (`DEF-`) skeleton. None when the rig has no
/// feet to read or the limb points along the facing.
pub fn hinge_axis(rig: &Rig, upper: usize, fore: usize, hand: usize) -> Option<Vec3> {
    let mut fwd = Vec3::ZERO;
    for side in ["L", "R"] {
        if let (Some(f), Some(t)) = (
            rig.find(&format!("DEF-foot.{side}")),
            rig.find(&format!("DEF-toe.{side}")),
        ) {
            let d = rig.head(t) - rig.head(f);
            fwd = fwd + Vec3::new(d.x, 0.0, d.z);
        }
    }
    if fwd.length() < 1e-9 {
        return None;
    }
    let fwd = fwd.normalized();
    let u = rig.head(fore) - rig.head(upper);
    let n = u.cross(fwd);
    if n.length() < 0.2 * u.length() {
        return None;
    }
    let n = n.normalized();
    let leg = rig.names[hand].contains("foot");
    let toward = if leg { -1.0 } else { 1.0 };
    let bent = u.normalized() + fwd.scale(0.2 * toward);
    Some(if u.cross(bent).dot(n) >= 0.0 {
        n
    } else {
        n.scale(-1.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_arm_turn_ignores_out_of_bounds_joints() {
        let doc = crate::fixture::humanoid(crate::fixture::Naming::Def, false, false).unwrap();
        let rig = Rig::from_doc(&doc).unwrap();
        let mut locals = vec![None; rig.names.len()];
        solve_arm_turn(
            &rig,
            &mut locals,
            usize::MAX,
            0,
            1,
            Vec3::new(0.0, 1.0, 0.0),
            Quat::IDENTITY,
        );
        assert!(locals.iter().all(|l| l.is_none()));
    }
}
