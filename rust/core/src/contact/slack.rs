//! Penetration depth, surface hits, and source-body slack comparison.

use super::body::{Body, Capsule, N_ARM_PARTS, PART_FOREARM, PART_PALM, PART_UPPER, PART_WEAPON};
use super::mesh::seg_seg;
use super::Options;
use crate::glb::Affine;
use crate::humanoid;
use crate::math::Vec3;
use crate::rig::{Rig, Trs};

/// Surface points inside `part` (grown by `grow`): (index, depth, normal).
/// Depth is how far the part's axis must move along the point's normal
/// to clear it by the part's radius (tangent-plane estimate).
pub(super) fn surface_hits(
    part: &Capsule,
    posed: &[(Vec3, Vec3)],
    inv_rest: &[Affine],
    world: &[Affine],
    grow: f64,
) -> Vec<(usize, f64, Vec3)> {
    let (a, b) = part.posed(inv_rest, world);
    let r = part.r + grow;
    let ab = b - a;
    let l2 = ab.dot(ab).max(1e-12);
    let mut out = Vec::new();
    for (k, (p, n)) in posed.iter().enumerate() {
        let t = ((*p - a).dot(ab) / l2).clamp(0.0, 1.0);
        let c = a + ab.scale(t);
        if (c - *p).length() >= r {
            continue;
        }
        let s = (c - *p).dot(*n);
        out.push((k, r - s, *n));
    }
    out
}

/// Penetration of `part` into `body` beyond `slack`: (depth, normal from
/// body to part, param along part).
pub(super) fn depth(
    part: &Capsule,
    body: &Capsule,
    inv_rest: &[Affine],
    world: &[Affine],
    slack: f64,
) -> (f64, Vec3, f64) {
    let (p1, q1) = part.posed(inv_rest, world);
    let (p2, q2) = body.posed(inv_rest, world);
    let (s, t, d) = seg_seg(p1, q1, p2, q2);
    let c1 = p1 + (q1 - p1).scale(s);
    let c2 = p2 + (q2 - p2).scale(t);
    let mut n = c1 - c2;
    if n.length() < 1e-9 {
        // Axes cross: push perpendicular to the body axis.
        let ax = (q2 - p2).normalized();
        let side = (q1 - p1).cross(ax);
        n = if side.length() > 1e-9 {
            side
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
    }
    (part.r + body.r - d - slack, n.normalized(), s)
}

/// Tolerance (metres) above source-body penetration before a target contact is
/// treated as worsened by the target body's extra width (absorbs float32 GLB
/// and retarget rounding on same-width targets).
pub(super) const SRC_EXCESS_TOL: f64 = 0.001;

/// Per-frame penetration slack measured on the source body (scaled to the
/// target's height) so faults already present on the source body itself are
/// left untouched when the target body does not worsen them.
pub(super) struct SourceSlack {
    pub(super) cap_raw: Vec<Vec<f64>>,
    pub(super) cap_margin: Vec<Vec<f64>>,
    pub(super) surf_raw: Vec<Vec<f64>>,
    pub(super) surf_margin: Vec<Vec<f64>>,
    pub(super) arm_raw: Vec<Vec<f64>>,
    pub(super) arm_margin: Vec<Vec<f64>>,
}

impl SourceSlack {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build(
        tgt_body: &Body,
        tgt_rig: &Rig,
        tgt_frames: &[Vec<Option<Trs>>],
        src_body: &Body,
        src_rig: &Rig,
        src_frames: &[Vec<Option<Trs>>],
        weapon: bool,
        opts: &Options,
    ) -> SourceSlack {
        let n = src_frames.len();
        let nb = tgt_body.static_parts.len();
        let nsb = tgt_body.surf_bones.len();
        let na = tgt_body.arms.len();
        let scale = tgt_body.height / src_body.height.max(1e-6);
        let src_margin = opts.margin * src_body.height;

        let arm_map: Vec<Option<usize>> = tgt_body
            .arms
            .iter()
            .map(|ta| src_body.arms.iter().position(|sa| sa.side == ta.side))
            .collect();
        let sp = &src_body.static_parts;
        let static_map: Vec<Option<usize>> = tgt_body
            .static_parts
            .iter()
            .map(|ts| sp.iter().position(|ss| ss.name == ts.name))
            .collect();

        let tgt_map = humanoid::map_rig(tgt_rig);
        let src_map = humanoid::map_rig(src_rig);
        let src_surf_to_tgt: Vec<Option<usize>> = src_body
            .surface
            .iter()
            .map(|(sb, _, _)| {
                let cname = src_map.canonical_of(*sb).unwrap_or(&src_rig.names[*sb]);
                tgt_body.surf_bones.iter().position(|&tb| {
                    let tname = tgt_map.canonical_of(tb).unwrap_or(&tgt_rig.names[tb]);
                    tname == cname
                })
            })
            .collect();
        let tgt_surf_slot: Vec<usize> = tgt_body
            .surface
            .iter()
            .map(|(tb, _, _)| {
                tgt_body
                    .surf_bones
                    .iter()
                    .position(|&b| b == *tb)
                    .unwrap_or(0)
            })
            .collect();

        let mut cap_raw = Vec::with_capacity(n);
        let mut cap_margin = Vec::with_capacity(n);
        let mut surf_raw = Vec::with_capacity(n);
        let mut surf_margin = Vec::with_capacity(n);
        let mut arm_raw = Vec::with_capacity(n);
        let mut arm_margin = Vec::with_capacity(n);

        for (fi, sf) in src_frames.iter().enumerate() {
            let sw = src_rig.world(sf);
            let tw = tgt_rig.world(&tgt_frames[fi]);
            let src_posed = src_body.posed_surface(&sw);
            let tgt_posed = tgt_body.posed_surface(&tw);
            let mut cr = vec![0.0f64; na * N_ARM_PARTS * nb];
            let mut cm = vec![0.0f64; na * N_ARM_PARTS * nb];
            let mut sr = vec![0.0f64; na * N_ARM_PARTS * nsb];
            let mut sm = vec![0.0f64; na * N_ARM_PARTS * nsb];

            for (ai, &src_ai_opt) in arm_map.iter().enumerate() {
                let Some(src_ai) = src_ai_opt else {
                    continue;
                };
                let s_arm = &src_body.arms[src_ai];
                let t_arm = &tgt_body.arms[ai];
                let s_use_w = weapon && s_arm.weapon.is_some();
                let t_use_w = weapon && t_arm.weapon.is_some();
                let s_parts = s_arm.parts(s_use_w);
                let t_parts = t_arm.parts(t_use_w);
                for (pi, part) in s_parts.iter().enumerate() {
                    let t_part = t_parts.get(pi).copied();
                    for (bi, &src_bi_opt) in static_map.iter().enumerate() {
                        let Some(src_bi) = src_bi_opt else {
                            continue;
                        };
                        let ss = &src_body.static_parts[src_bi];
                        let ts = &tgt_body.static_parts[bi];
                        let raw_d = depth(part, ss, &src_body.inv_rest, &sw, 0.0).0;
                        let s_raw = raw_d.max(0.0) * scale;
                        let s_mar = (raw_d + src_margin).max(0.0) * scale;
                        let t_raw = t_part
                            .map(|tp| depth(tp, ts, &tgt_body.inv_rest, &tw, 0.0).0.max(0.0))
                            .unwrap_or(0.0);
                        let idx = (ai * N_ARM_PARTS + pi) * nb + bi;
                        if s_raw > 0.0 && t_raw <= s_raw + SRC_EXCESS_TOL {
                            cr[idx] = s_raw;
                            cm[idx] = s_mar;
                        } else if s_raw == 0.0 && t_raw <= SRC_EXCESS_TOL {
                            cm[idx] = s_mar;
                        }
                    }
                    let mut s_sr = vec![0.0f64; nsb];
                    let mut s_sm = vec![0.0f64; nsb];
                    let sh = surface_hits(part, &src_posed, &src_body.inv_rest, &sw, src_margin);
                    for (k, d, _) in sh {
                        let Some(tslot) = src_surf_to_tgt[k] else {
                            continue;
                        };
                        s_sm[tslot] = s_sm[tslot].max(d * scale);
                        s_sr[tslot] = s_sr[tslot].max((d - src_margin).max(0.0) * scale);
                    }
                    let mut t_sr = vec![0.0f64; nsb];
                    if s_sm.iter().any(|&v| v > 0.0) {
                        if let Some(tp) = t_part {
                            for (k, d, _) in
                                surface_hits(tp, &tgt_posed, &tgt_body.inv_rest, &tw, 0.0)
                            {
                                let tslot = tgt_surf_slot[k];
                                t_sr[tslot] = t_sr[tslot].max(d);
                            }
                        }
                    }
                    for tslot in 0..nsb {
                        let idx = (ai * N_ARM_PARTS + pi) * nsb + tslot;
                        if s_sr[tslot] > 0.0 && t_sr[tslot] <= s_sr[tslot] + SRC_EXCESS_TOL {
                            sr[idx] = s_sr[tslot];
                            sm[idx] = s_sm[tslot];
                        } else if s_sr[tslot] == 0.0 && t_sr[tslot] <= SRC_EXCESS_TOL {
                            sm[idx] = s_sm[tslot];
                        }
                    }
                }
            }

            let mut ar = vec![0.0f64; 9];
            let mut am = vec![0.0f64; 9];
            if na >= 2 {
                if let (Some(sa0), Some(sa1)) = (arm_map[0], arm_map[1]) {
                    let p0s = src_body.arms[sa0].body_parts();
                    let p1s = src_body.arms[sa1].body_parts();
                    let tp0s = tgt_body.arms[0].body_parts();
                    let tp1s = tgt_body.arms[1].body_parts();
                    for (i0, c0) in p0s.iter().enumerate() {
                        for (i1, c1) in p1s.iter().enumerate() {
                            let raw_d = depth(c0, c1, &src_body.inv_rest, &sw, 0.0).0;
                            let s_raw = raw_d.max(0.0) * scale;
                            let s_mar = (raw_d + src_margin).max(0.0) * scale;
                            let t_raw = depth(tp0s[i0], tp1s[i1], &tgt_body.inv_rest, &tw, 0.0)
                                .0
                                .max(0.0);
                            let idx = i0 * 3 + i1;
                            if s_raw > 0.0 && t_raw <= s_raw + SRC_EXCESS_TOL {
                                ar[idx] = s_raw;
                                am[idx] = s_mar;
                            } else if s_raw == 0.0 && t_raw <= SRC_EXCESS_TOL {
                                am[idx] = s_mar;
                            }
                        }
                    }
                }
            }

            cap_raw.push(cr);
            cap_margin.push(cm);
            surf_raw.push(sr);
            surf_margin.push(sm);
            arm_raw.push(ar);
            arm_margin.push(am);
        }

        SourceSlack {
            cap_raw,
            cap_margin,
            surf_raw,
            surf_margin,
            arm_raw,
            arm_margin,
        }
    }
}

/// One contact found in a frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    pub(super) depth: f64,
    pub(super) normal: Vec3,
    /// How much of a wrist move reaches the contact point (1 for the hand
    /// and weapon, the param along the forearm for the forearm).
    pub(super) reach: f64,
    pub(super) part: usize,
    pub(super) body: usize,
    /// Contact point on the part (world).
    pub(super) point: Vec3,
}

fn reach_of(pi: usize, at: f64) -> f64 {
    match pi {
        PART_UPPER => ((0.25 + 0.75 * at) * 0.5).max(0.35),
        PART_FOREARM => (0.2 + 0.8 * at).max(0.35),
        PART_PALM | PART_WEAPON => 1.0,
        _ => 1.0,
    }
}

fn effective_capsule_slack(r: f64, src_raw: f64, src_margin: f64, margin: f64) -> f64 {
    if margin <= 0.0 {
        let sl = r.max(src_raw);
        let eps = if src_raw > 0.0 { 1e-4 } else { 0.0 };
        if sl > 0.0 {
            sl + eps
        } else {
            0.0
        }
    } else {
        let rest_m = if r > 0.0 { r + margin } else { 0.0 };
        let m_sl = rest_m.max(src_margin);
        let eps = if src_margin > 0.0 { 1e-4 } else { 0.0 };
        if m_sl > 0.0 {
            m_sl - margin + eps
        } else {
            -margin
        }
    }
}

fn eff_surf_depth(d: f64, r: f64, src_raw: f64, src_margin: f64, margin: f64) -> f64 {
    if margin <= 0.0 {
        let sl = r.max(src_raw);
        let eps = if src_raw > 0.0 { 1e-4 } else { 0.0 };
        if sl > 0.0 {
            d - sl - eps
        } else {
            d
        }
    } else {
        let rest_m = if r > 0.0 { r + margin } else { 0.0 };
        let m_sl = rest_m.max(src_margin);
        let eps = if src_margin > 0.0 { 1e-4 } else { 0.0 };
        if m_sl > 0.0 {
            d - m_sl - eps
        } else {
            d
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn hits_with_slack(
    body: &Body,
    ai: usize,
    world: &[Affine],
    weapon: bool,
    margin: f64,
    src_cap_raw: Option<&[f64]>,
    src_cap_margin: Option<&[f64]>,
    src_surf_raw: Option<&[f64]>,
    src_surf_margin: Option<&[f64]>,
) -> Vec<Hit> {
    let arm = &body.arms[ai];
    let nb = body.static_parts.len();
    let mut out = Vec::new();
    let posed = body.posed_surface(world);
    let nsb = body.surf_bones.len();
    for (pi, part) in arm.parts(weapon).iter().enumerate() {
        // Upper arm is anchored at the fixed shoulder socket: tested in
        // arm-vs-arm, not against its own adjacent torso/armpit.
        if pi == PART_UPPER {
            continue;
        }
        let (pa, pb) = part.posed(&body.inv_rest, world);
        // Deepest surface point per body bone.
        let mut best: Vec<Option<(f64, Vec3, usize)>> = vec![None; nsb];
        for (k, d, n) in surface_hits(part, &posed, &body.inv_rest, world, margin) {
            let Some(slot) = body.surf_bones.iter().position(|&b| b == body.surface[k].0) else {
                continue;
            };
            let idx = (ai * N_ARM_PARTS + pi) * nsb + slot;
            let sr = src_surf_raw.map(|v| v[idx]).unwrap_or(0.0);
            let sm = src_surf_margin.map(|v| v[idx]).unwrap_or(0.0);
            let d = eff_surf_depth(d, body.surf_slack[idx], sr, sm, margin);
            if d > 0.0 && best[slot].is_none_or(|x| d > x.0) {
                best[slot] = Some((d, n, k));
            }
        }
        for (slot, b) in best.into_iter().enumerate() {
            if let Some((d, n, k)) = b {
                let p = posed[k].0;
                let ab = pb - pa;
                let at = ((p - pa).dot(ab) / ab.dot(ab).max(1e-12)).clamp(0.0, 1.0);
                out.push(Hit {
                    depth: d,
                    normal: n,
                    reach: reach_of(pi, at),
                    part: pi,
                    body: nb + slot,
                    point: pa + ab.scale(at),
                });
            }
        }
    }
    for (pi, part) in arm.parts(weapon).iter().enumerate() {
        if pi == PART_UPPER {
            continue;
        }
        for (bi, s) in body.static_parts.iter().enumerate() {
            let idx = (ai * N_ARM_PARTS + pi) * nb + bi;
            let sr = src_cap_raw.map(|v| v[idx]).unwrap_or(0.0);
            let sm = src_cap_margin.map(|v| v[idx]).unwrap_or(0.0);
            let eff_slack = effective_capsule_slack(body.slack[idx], sr, sm, margin);
            let (d, n, at) = depth(part, s, &body.inv_rest, world, eff_slack);
            if d > 0.0 {
                let (pa, pb) = part.posed(&body.inv_rest, world);
                out.push(Hit {
                    depth: d,
                    normal: n,
                    reach: reach_of(pi, at),
                    part: pi,
                    body: bi,
                    point: pa + (pb - pa).scale(at),
                });
            }
        }
    }
    out
}

/// Deepest arm-against-arm contact in one frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct ArmArmHit {
    pub(super) depth: f64,
    /// Unit normal pointing from arm 1's capsule toward arm 0's capsule.
    pub(super) normal: Vec3,
    pub(super) reach0: f64,
    pub(super) reach1: f64,
}

pub(super) fn arm_arm_hit(
    body: &Body,
    world: &[Affine],
    margin: f64,
    src_raw: Option<&[f64]>,
    src_margin: Option<&[f64]>,
) -> Option<ArmArmHit> {
    if body.arms.len() < 2 {
        return None;
    }
    let p0s = body.arms[0].body_parts();
    let p1s = body.arms[1].body_parts();
    let mut best: Option<ArmArmHit> = None;
    for (i0, c0) in p0s.iter().enumerate() {
        let (a0, b0) = c0.posed(&body.inv_rest, world);
        for (i1, c1) in p1s.iter().enumerate() {
            let (a1, b1) = c1.posed(&body.inv_rest, world);
            let (s0, s1, dist) = seg_seg(a0, b0, a1, b1);
            let idx = i0 * 3 + i1;
            let rest_sl = body.arm_slack.get(idx).copied().unwrap_or(0.0);
            let sr = src_raw.map(|v| v[idx]).unwrap_or(0.0);
            let sm = src_margin.map(|v| v[idx]).unwrap_or(0.0);
            let eff_slack = effective_capsule_slack(rest_sl, sr, sm, margin);
            let d = c0.r + c1.r - dist - eff_slack;
            if d > 0.0 && best.is_none_or(|b| d > b.depth) {
                let pt0 = a0 + (b0 - a0).scale(s0);
                let pt1 = a1 + (b1 - a1).scale(s1);
                let mut n = pt0 - pt1;
                if n.length() < 1e-9 {
                    let cr = (b0 - a0).cross(b1 - a1);
                    n = if cr.length() > 1e-9 {
                        cr
                    } else {
                        Vec3::new(0.0, 0.0, 1.0)
                    };
                }
                best = Some(ArmArmHit {
                    depth: d,
                    normal: n.normalized(),
                    reach0: reach_of(i0, s0),
                    reach1: reach_of(i1, s1),
                });
            }
        }
    }
    best
}
