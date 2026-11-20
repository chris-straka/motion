//! Width-first retargeting per body (`docs/width-refit.md`, motion-13).
//!
//! Clips authored on a slim library rig carry hand paths shaped for that
//! slim torso and hips. Played on a wider body, plain FK puts wrists and
//! forearms inside the wider chest, hips, and legs. This pass measures
//! source and target width/depth profiles from their collision proxies
//! (chest, waist, thighs, shins) and smoothly widens wrist positions inside
//! the body's width envelope before the contact pass runs.
//!
//! Invariants:
//! - Same-width target (within 1e-4) -> early exit, zero change.
//! - Arms out wide (outside the width envelope + blend band) -> zero change.
//! - Key count and clip timing are preserved; legs are never modified.
//! - Byte-deterministic (no HashMap, no platform libm).

use crate::animate::Mapped;
use crate::contact::{solve_arm, Body, Options};
use crate::glb::Affine;
use crate::math::Vec3;
use crate::rig::{Rig, Trs};

struct Profile {
    height: f64,
    /// Sorted by normalized height `u`: `(u, half_width_m, half_depth_m)`.
    stations: Vec<(f64, f64, f64)>,
}

impl Profile {
    fn from_body(body: &Body, mapped: &Mapped) -> Option<Profile> {
        let hips = mapped.map.node_of("DEF-spine")?;
        let origin = mapped.rig.head(hips);
        let up = Vec3::new(0.0, 1.0, 0.0);
        let fwd = mapped.forward;
        let left = up.cross(fwd).normalized();
        if left.length() < 1e-6 || body.height <= 1e-6 {
            return None;
        }

        let groups: [&dyn Fn(&str) -> bool; 4] = [
            &|n: &str| n.starts_with("shin."),
            &|n: &str| n.starts_with("thigh."),
            &|n: &str| {
                n == "spine"
                    || n.starts_with("spine.L")
                    || n.starts_with("spine.R")
                    || n.starts_with("spine.001")
            },
            &|n: &str| n.starts_with("spine.002") || n.starts_with("spine.003"),
        ];

        let mut stations: Vec<(f64, f64, f64)> = Vec::new();
        for pred in groups {
            let mut y_sum = 0.0;
            let mut count = 0usize;
            let mut hw: f64 = 0.0;
            let mut hd: f64 = 0.0;
            for c in &body.static_parts {
                if !pred(&c.name) {
                    continue;
                }
                let pa = mapped.rig.rest_world[c.bone].apply(c.a);
                let pb = mapped.rig.rest_world[c.bone].apply(c.b);
                y_sum += 0.5 * (pa.y + pb.y);
                count += 1;
                let la = (pa - origin).dot(left).abs();
                let lb = (pb - origin).dot(left).abs();
                let fa = (pa - origin).dot(fwd).abs();
                let fb = (pb - origin).dot(fwd).abs();
                hw = hw.max(la.max(lb) + c.r);
                hd = hd.max(fa.max(fb) + c.r);
            }
            if count > 0 && hw > 1e-6 && hd > 1e-6 {
                let y_mid = y_sum / count as f64;
                let u = ((y_mid - body.floor) / body.height).clamp(0.0, 1.0);
                stations.push((u, hw, hd));
            }
        }
        if stations.is_empty() {
            return None;
        }
        stations.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Profile {
            height: body.height,
            stations,
        })
    }

    fn sample(&self, u: f64) -> (f64, f64) {
        let s = &self.stations;
        if s.len() == 1 || u <= s[0].0 {
            return (s[0].1, s[0].2);
        }
        let last = s.len() - 1;
        if u >= s[last].0 {
            return (s[last].1, s[last].2);
        }
        for i in 0..last {
            let (u0, w0, d0) = s[i];
            let (u1, w1, d1) = s[i + 1];
            if u <= u1 {
                let span = (u1 - u0).max(1e-9);
                let t = ((u - u0) / span).clamp(0.0, 1.0);
                return (w0 + (w1 - w0) * t, d0 + (d1 - d0) * t);
            }
        }
        (s[last].1, s[last].2)
    }
}

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn spine_chain(mapped: &Mapped) -> Vec<usize> {
    [
        "DEF-spine",
        "DEF-spine.001",
        "DEF-spine.002",
        "DEF-spine.003",
    ]
    .iter()
    .filter_map(|n| mapped.map.node_of(n))
    .collect()
}

#[derive(Clone, Copy)]
struct BodyAxes {
    left: Vec3,
    up: Vec3,
    fwd: Vec3,
}

/// Posed body-local frame `(center, left, up, forward, norm_height_u)` at `p`.
fn body_frame_at(
    rig: &Rig,
    world: &[Affine],
    chain: &[usize],
    axes: BodyAxes,
    body: &Body,
    p: Vec3,
) -> (Vec3, Vec3, Vec3, Vec3, f64) {
    let hips = chain[0];
    let r_hips = rig.rest_world[hips].rotation();
    let q_hips = world[hips].rotation().mul(r_hips.conj());
    let h_up = q_hips.rotate_vec(axes.up).normalized();
    let h_pos = world[hips].t;
    let h_rest_y = rig.head(hips).y;
    let dy_hips = (p - h_pos).dot(h_up);
    if chain.len() == 1 || dy_hips <= 0.0 {
        let center = h_pos + h_up.scale(dy_hips);
        let b_left = q_hips.rotate_vec(axes.left).normalized();
        let b_fwd = q_hips.rotate_vec(axes.fwd).normalized();
        let u = ((h_rest_y + dy_hips - body.floor) / body.height).clamp(0.0, 1.0);
        return (center, b_left, h_up, b_fwd, u);
    }

    let mut best_k = 0usize;
    let mut best_t = 0.0f64;
    let mut best_d2 = f64::INFINITY;
    for k in 0..chain.len() - 1 {
        let a = world[chain[k]].t;
        let b = world[chain[k + 1]].t;
        let ab = b - a;
        let l2 = ab.dot(ab).max(1e-12);
        let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
        let c = a + ab.scale(t);
        let d2 = (p - c).length_sq();
        if d2 < best_d2 {
            best_d2 = d2;
            best_k = k;
            best_t = t;
        }
    }
    let bone = chain[best_k];
    let r_b = rig.rest_world[bone].rotation();
    let q_b = world[bone].rotation().mul(r_b.conj());
    let b_left = q_b.rotate_vec(axes.left).normalized();
    let b_up = q_b.rotate_vec(axes.up).normalized();
    let b_fwd = q_b.rotate_vec(axes.fwd).normalized();
    let last_seg = chain.len() - 2;
    let (center, y_ref) = if best_k == last_seg && best_t >= 1.0 {
        let top = chain[chain.len() - 1];
        let top_pos = world[top].t;
        let extra = (p - top_pos).dot(b_up).max(0.0);
        (top_pos + b_up.scale(extra), rig.head(top).y + extra)
    } else {
        let a = world[chain[best_k]].t;
        let b = world[chain[best_k + 1]].t;
        let ya = rig.head(chain[best_k]).y;
        let yb = rig.head(chain[best_k + 1]).y;
        (a + (b - a).scale(best_t), ya + (yb - ya) * best_t)
    };
    let u = ((y_ref - body.floor) / body.height).clamp(0.0, 1.0);
    (center, b_left, b_up, b_fwd, u)
}

/// Refit arm wrist paths from `src_body` onto `tgt_body` in place.
#[allow(clippy::too_many_arguments)]
pub fn refit_clip(
    src_body: &Body,
    src: &Mapped,
    src_frames: &[Vec<Option<Trs>>],
    tgt_body: &Body,
    tgt: &Mapped,
    tgt_frames: &mut [Vec<Option<Trs>>],
    clip: &str,
    opts: &Options,
) {
    if src_frames.len() != tgt_frames.len() || tgt_frames.is_empty() {
        return;
    }
    let (Some(src_prof), Some(tgt_prof)) = (
        Profile::from_body(src_body, src),
        Profile::from_body(tgt_body, tgt),
    ) else {
        return;
    };
    let src_chain = spine_chain(src);
    let tgt_chain = spine_chain(tgt);
    if src_chain.is_empty() || tgt_chain.is_empty() {
        return;
    }

    let h_scale = tgt_prof.height / src_prof.height.max(1e-6);
    // If source and target have effectively the same width & depth at every
    // station, leave the clip completely unchanged.
    let mut needs_refit = false;
    for &(u, _, _) in src_prof.stations.iter().chain(&tgt_prof.stations) {
        let (sw, sd) = src_prof.sample(u);
        let (tw, td) = tgt_prof.sample(u);
        let rw = tw / (sw * h_scale).max(1e-6);
        let rd = td / (sd * h_scale).max(1e-6);
        if rw > 1.0 + 1e-4 || rd > 1.0 + 1e-4 {
            needs_refit = true;
            break;
        }
    }
    if !needs_refit {
        return;
    }

    let up0 = Vec3::new(0.0, 1.0, 0.0);
    let src_fwd0 = src.forward;
    let src_left0 = up0.cross(src_fwd0).normalized();
    let tgt_fwd0 = tgt.forward;
    let tgt_left0 = up0.cross(tgt_fwd0).normalized();
    let src_axes = BodyAxes {
        left: src_left0,
        up: up0,
        fwd: src_fwd0,
    };
    let tgt_axes = BodyAxes {
        left: tgt_left0,
        up: up0,
        fwd: tgt_fwd0,
    };
    let src_hips = src_chain[0];
    let tgt_hips = tgt_chain[0];
    let weapon = opts.weapon_in(clip);

    // Pair target arms with matching source arms by side.
    let mut pairs: Vec<(usize, usize, f64, f64, bool)> = Vec::new();
    for (ti, t_arm) in tgt_body.arms.iter().enumerate() {
        let Some(si) = src_body.arms.iter().position(|a| a.side == t_arm.side) else {
            continue;
        };
        let s_arm = &src_body.arms[si];
        let arm_sgn = if t_arm.side == "L" { 1.0 } else { -1.0 };
        let d_s = src.rig.head(s_arm.upper) - src.rig.head(src_hips);
        let d_t = tgt.rig.head(t_arm.upper) - tgt.rig.head(tgt_hips);
        let sh_s = d_s.dot(src_left0).abs();
        let sh_t = d_t.dot(tgt_left0).abs();
        let sh_delta = sh_t - sh_s * h_scale;
        let use_weapon = weapon && t_arm.weapon.is_some();
        pairs.push((ti, si, arm_sgn, sh_delta, use_weapon));
    }
    if pairs.is_empty() {
        return;
    }

    let srig = &src.rig;
    let trig = &tgt.rig;
    for f in 0..tgt_frames.len() {
        let sw = srig.world(&src_frames[f]);
        let tw = trig.world(&tgt_frames[f]);
        for &(ti, si, arm_sgn, sh_delta, use_weapon) in &pairs {
            let s_arm = &src_body.arms[si];
            let t_arm = &tgt_body.arms[ti];
            let w_s = sw[s_arm.hand].t;
            let (c_s, l_s, _, f_s, u_s) =
                body_frame_at(srig, &sw, &src_chain, src_axes, src_body, w_s);
            let rel_s = w_s - c_s;
            let side_s = rel_s.dot(l_s);
            let fwd_s = rel_s.dot(f_s);
            let (hw_s, hd_s) = src_prof.sample(u_s);
            let (hw_t, hd_t) = tgt_prof.sample(u_s);

            // Smooth width/depth envelope: full refit inside body + inner margin,
            // smoothly blending to 0 outside outer margin (wide arms untouched).
            let inner_side = hw_s + 0.08 * src_body.height;
            let outer_side = hw_s + 0.20 * src_body.height;
            let inner_fwd = hd_s + 0.12 * src_body.height;
            let outer_fwd = hd_s + 0.24 * src_body.height;
            let bs = 1.0 - smoothstep((side_s.abs() - inner_side) / (outer_side - inner_side));
            let bf = 1.0 - smoothstep((fwd_s.abs() - inner_fwd) / (outer_fwd - inner_fwd));
            let blend = bs * bf;
            if blend <= 0.0 {
                continue;
            }

            let extra_side = ((hw_t - hw_s * h_scale) - sh_delta).max(0.0);
            let extra_fwd = (hd_t - hd_s * h_scale).max(0.0);
            if extra_side <= 1e-9 && extra_fwd <= 1e-9 {
                continue;
            }

            let side_factor = ((side_s * arm_sgn) / hw_s.max(1e-4)).clamp(0.0, 1.0);
            let fwd_factor = (fwd_s / hd_s.max(1e-4)).clamp(-1.0, 1.0);
            let w_t = tw[t_arm.hand].t;
            let (_, l_t, _, f_t, _) = body_frame_at(trig, &tw, &tgt_chain, tgt_axes, tgt_body, w_t);
            let shift = l_t.scale(arm_sgn * side_factor * extra_side * blend)
                + f_t.scale(fwd_factor * extra_fwd * blend);
            if shift.length() <= 1e-9 {
                continue;
            }

            let orig_hand = tgt_frames[f][t_arm.hand];
            let target = w_t + shift;
            let frame = &mut tgt_frames[f];
            solve_arm(trig, frame, t_arm.upper, t_arm.fore, t_arm.hand, target);
            if !use_weapon {
                tgt_frames[f][t_arm.hand] = orig_hand;
            }
        }
    }
}
