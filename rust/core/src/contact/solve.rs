//! Contact solver: iterative arm IK clearance, temporal smoothing, and arm-vs-arm separation.

use super::body::{Body, PART_WEAPON};
use super::ik::{solve_arm, solve_arm_turn};
use super::report::{
    measure_with_slack, mesh_measure_initial, mesh_measure_with_slack, ArmReport, ClipContact,
    SourceFrames,
};
use super::slack::{arm_arm_hit, hits_with_slack, SourceSlack};
use super::Options;
use crate::math::{Quat, Vec3};
use crate::rig::{Rig, Trs};

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// One fix attempt on one arm with the arm proxies grown by `grow`
/// (radius multiplier). Returns the new frames and wrist offsets.
#[allow(clippy::too_many_arguments)]
fn fix_arm(
    body: &Body,
    rig: &Rig,
    ai: usize,
    original: &[Vec<Option<Trs>>],
    times: &[f64],
    use_weapon: bool,
    margin: f64,
    ramp: f64,
    grow: f64,
    src_slack: Option<&SourceSlack>,
) -> (Vec<Vec<Option<Trs>>>, Vec<Vec3>, Vec<Vec3>) {
    let mut grown = body.clone();
    {
        let arm = &mut grown.arms[ai];
        arm.forearm.r *= grow;
        arm.palm.r *= grow;
        if let Some(w) = arm.weapon.as_mut() {
            w.r *= grow;
        }
    }
    let body = &grown;
    let arm = &body.arms[ai];
    let n = original.len();
    if n == 0 {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut frames: Vec<Vec<Option<Trs>>> = original.to_vec();
    let wrist0: Vec<Vec3> = original.iter().map(|f| rig.world(f)[arm.hand].t).collect();
    let mut offset = vec![Vec3::ZERO; n];
    let mut turn_off = vec![Vec3::ZERO; n];
    // Limits that keep the swing's intent: the wrist moves at most 12% of
    // the character's height, a held blade turns at most 40 degrees.
    let max_shift = 0.12 * body.height;
    let max_turn = 40f64.to_radians();
    for _round in 0..3 {
        // 1. Per frame: the extra wrist offset that clears this frame.
        let mut need = vec![Vec3::ZERO; n];
        let mut any = false;
        let mut need_turn = vec![Vec3::ZERO; n];
        for f in 0..n {
            let mut locals = frames[f].clone();
            let mut extra = Vec3::ZERO;
            let mut turn = Vec3::ZERO; // rotation vector (axis * angle)
            let base = wrist0[f] + offset[f];
            for _ in 0..12 {
                let world = rig.world(&locals);
                let hs = hits_with_slack(
                    body,
                    ai,
                    &world,
                    use_weapon,
                    margin,
                    src_slack.map(|s| s.cap_raw[f].as_slice()),
                    src_slack.map(|s| s.cap_margin[f].as_slice()),
                    src_slack.map(|s| s.surf_raw[f].as_slice()),
                    src_slack.map(|s| s.surf_margin[f].as_slice()),
                );
                let Some(h) = hs
                    .iter()
                    .max_by(|a, b| (a.depth / a.reach).total_cmp(&(b.depth / b.reach)))
                else {
                    break;
                };
                let wrist = world[arm.hand].t;
                let lever = h.point - wrist;
                if h.part == PART_WEAPON && lever.length() > 1e-6 && turn.length() < max_turn {
                    // Blade: swing it clear about the wrist.
                    let axis = lever.cross(h.normal);
                    if axis.length() > 1e-9 {
                        let ang = ((h.depth + 0.5 * margin) / lever.length()).min(0.35);
                        turn = turn + axis.normalized().scale(ang);
                        if turn.length() > max_turn {
                            turn = turn.scale(max_turn / turn.length());
                        }
                    }
                } else {
                    extra = extra + h.normal.scale((h.depth + 0.5 * margin) / h.reach);
                    if extra.length() > max_shift {
                        extra = extra.scale(max_shift / extra.length());
                    }
                }
                locals = frames[f].clone();
                solve_arm_turn(
                    rig,
                    &mut locals,
                    arm.upper,
                    arm.fore,
                    arm.hand,
                    base + extra,
                    rotvec_quat(turn),
                );
                if !use_weapon {
                    locals[arm.hand] = original[f][arm.hand];
                }
            }
            if extra.length() > 0.0 || turn.length() > 0.0 {
                any = true;
            }
            need[f] = extra;
            need_turn[f] = turn;
        }
        if !any {
            break;
        }
        // 2. Smooth over time: max envelope with a smoothstep falloff.
        let smooth = envelope(&need, times, ramp);
        let smooth_turn = envelope(&need_turn, times, ramp);
        // 3. Apply.
        for f in 0..n {
            if smooth[f].length() <= 0.0 && smooth_turn[f].length() <= 0.0 {
                continue;
            }
            offset[f] = offset[f] + smooth[f];
            if offset[f].length() > max_shift {
                offset[f] = offset[f].scale(max_shift / offset[f].length());
            }
            turn_off[f] = turn_off[f] + smooth_turn[f];
            if turn_off[f].length() > max_turn {
                turn_off[f] = turn_off[f].scale(max_turn / turn_off[f].length());
            }
            let mut locals = original[f].clone();
            solve_arm_turn(
                rig,
                &mut locals,
                arm.upper,
                arm.fore,
                arm.hand,
                wrist0[f] + offset[f],
                rotvec_quat(turn_off[f]),
            );
            if !use_weapon {
                // A free hand turns with its forearm (see `fix_clip`).
                locals[arm.hand] = original[f][arm.hand];
            }
            frames[f] = locals;
        }
    }
    (frames, offset, turn_off)
}

/// Each vector's size kept, its direction smoothed: the binomial
/// [1 4 6 4 1]/16 mean of its neighbours (weighted by their sizes, ends
/// held), normalized.
fn steady_dir(v: &[Vec3]) -> Vec<Vec3> {
    if v.is_empty() {
        return Vec::new();
    }
    let n = v.len();
    let at = |i: isize| v[i.clamp(0, n as isize - 1) as usize];
    (0..n as isize)
        .map(|i| {
            let m = (at(i - 2) + at(i + 2) + (at(i - 1) + at(i + 1)).scale(4.0) + at(i).scale(6.0))
                .scale(1.0 / 16.0);
            let len = at(i).length();
            if m.length() > 1e-12 && len > 0.0 {
                m.normalized().scale(len)
            } else {
                at(i)
            }
        })
        .collect()
}

/// How fast a contact correction may change: wrist offset in character
/// heights per second, blade turn in radians per second.
pub const MAX_OFFSET_SPEED: f64 = 0.9;
pub const MAX_TURN_SPEED: f64 = 3.0;

/// Limit the change between neighbouring frames to `step`: each pair that
/// differs by more is pulled together, half each way, repeatedly (a jump
/// is spread over the frames around it; a sequence already within the
/// limit is returned unchanged).
fn rate_limit(v: &[Vec3], step: f64) -> Vec<Vec3> {
    let mut out = v.to_vec();
    for _ in 0..64 {
        let mut moved = false;
        for f in 0..out.len().saturating_sub(1) {
            let d = out[f + 1] - out[f];
            let len = d.length();
            if len > step * (1.0 + 1e-9) {
                let fix = d.scale((len - step) / (2.0 * len));
                out[f] = out[f] + fix;
                out[f + 1] = out[f + 1] - fix;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    out
}

/// Rotation vector (axis * angle) to quaternion.
fn rotvec_quat(v: Vec3) -> Quat {
    let a = v.length();
    if a < 1e-12 {
        Quat::IDENTITY
    } else {
        Quat::from_axis_angle(v.scale(1.0 / a), a)
    }
}

/// Max envelope with a smoothstep falloff of `ramp` seconds: every frame
/// gets at least the falloff of any nearby frame's need, so a correction
/// eases in before a contact and out after it. Direction is the
/// falloff-weighted mean of the needs.
fn envelope(need: &[Vec3], times: &[f64], ramp: f64) -> Vec<Vec3> {
    let n = need.len().min(times.len());
    let mut out = vec![Vec3::ZERO; n];
    for f in 0..n {
        let mut mag: f64 = 0.0;
        let mut dir = Vec3::ZERO;
        for g in 0..n {
            let len = need[g].length();
            if len <= 0.0 {
                continue;
            }
            let k = smoothstep(1.0 - (times[f] - times[g]).abs() / ramp.max(1e-6));
            if k <= 0.0 {
                continue;
            }
            mag = mag.max(len * k);
            dir = dir + need[g].scale(k);
        }
        if mag > 0.0 && dir.length() > 1e-12 {
            out[f] = dir.normalized().scale(mag);
        }
    }
    out
}

/// One fix attempt: frames, wrist offsets, frames still inside the mesh,
/// deepest residual, blade turns.
type Attempt = (Vec<Vec<Option<Trs>>>, Vec<Vec3>, usize, f64, Vec<Vec3>);

fn clamp_vec(v: Vec3, max_len: f64) -> Vec3 {
    let l = v.length();
    if l > max_len && l > 1e-12 {
        v.scale(max_len / l)
    } else {
        v
    }
}

/// Arm-against-arm separation pass: pushes colliding arms apart (half each
/// when both are free, or free arm first when one holds a weapon), while
/// avoiding driving either arm back into static body proxies.
#[allow(clippy::too_many_arguments)]
fn fix_arm_vs_arm(
    body: &Body,
    rig: &Rig,
    frames: &mut [Vec<Option<Trs>>],
    times: &[f64],
    weapon: bool,
    opts: &Options,
    src_slack: Option<&SourceSlack>,
    report: &mut ClipContact,
    mesh_slacks: &[Vec<f64>],
) {
    let margin = opts.margin * body.height;
    let ramp = opts.ramp;
    if body.arms.len() < 2 || frames.is_empty() {
        return;
    }
    let n = frames.len();
    let has_hit = (0..n).any(|f| {
        let world = rig.world(&frames[f]);
        arm_arm_hit(
            body,
            &world,
            0.0,
            src_slack.map(|s| s.arm_raw[f].as_slice()),
            src_slack.map(|s| s.arm_margin[f].as_slice()),
        )
        .is_some()
    });
    if !has_hit {
        return;
    }

    let use_w = [
        weapon && body.arms[0].weapon.is_some(),
        weapon && body.arms[1].weapon.is_some(),
    ];
    let base_frames: Vec<Vec<Option<Trs>>> = frames.to_vec();
    let wrist0: [Vec<Vec3>; 2] = [
        base_frames
            .iter()
            .map(|f| rig.world(f)[body.arms[0].hand].t)
            .collect(),
        base_frames
            .iter()
            .map(|f| rig.world(f)[body.arms[1].hand].t)
            .collect(),
    ];
    let mut offset = [vec![Vec3::ZERO; n], vec![Vec3::ZERO; n]];
    let max_shift = 0.12 * body.height;

    let apply_arm = |loc: &mut [Option<Trs>], ai: usize, target: Vec3, hand: Option<Trs>| {
        let arm = &body.arms[ai];
        solve_arm(rig, loc, arm.upper, arm.fore, arm.hand, target);
        if !use_w[ai] {
            loc[arm.hand] = hand;
        }
    };

    for _round in 0..3 {
        let mut need = [vec![Vec3::ZERO; n], vec![Vec3::ZERO; n]];
        let mut any = false;
        for f in 0..n {
            let mut locals = frames[f].clone();
            let mut extra = [Vec3::ZERO; 2];
            let base = [wrist0[0][f] + offset[0][f], wrist0[1][f] + offset[1][f]];
            let mut free_blocked = false;
            for _ in 0..16 {
                let world = rig.world(&locals);
                let Some(h) = arm_arm_hit(
                    body,
                    &world,
                    margin,
                    src_slack.map(|s| s.arm_raw[f].as_slice()),
                    src_slack.map(|s| s.arm_margin[f].as_slice()),
                ) else {
                    break;
                };
                let sep = h.depth + 0.5 * margin;
                if use_w[0] != use_w[1] {
                    let free = if !use_w[0] { 0 } else { 1 };
                    let wep = 1 - free;
                    let free_dir = if free == 0 {
                        h.normal
                    } else {
                        h.normal.scale(-1.0)
                    };
                    let reach_free = if free == 0 { h.reach0 } else { h.reach1 };
                    let reach_wep = if wep == 0 { h.reach0 } else { h.reach1 };
                    if !free_blocked {
                        let prev = extra[free];
                        let want = clamp_vec(prev + free_dir.scale(sep / reach_free), max_shift);
                        let achieved = (want - prev).dot(free_dir) * reach_free;
                        extra[free] = want;
                        let rem = (sep - achieved).max(0.0);
                        if rem > 1e-4 {
                            free_blocked = true;
                            let step = free_dir.scale(-rem / reach_wep);
                            extra[wep] = clamp_vec(extra[wep] + step, max_shift);
                        }
                    } else {
                        let step = free_dir.scale(-sep / reach_wep);
                        extra[wep] = clamp_vec(extra[wep] + step, max_shift);
                    }
                } else {
                    let prev0 = extra[0];
                    let prev1 = extra[1];
                    let want0 = clamp_vec(prev0 + h.normal.scale(0.5 * sep / h.reach0), max_shift);
                    let want1 = clamp_vec(prev1 + h.normal.scale(-0.5 * sep / h.reach1), max_shift);
                    let ach0 = (want0 - prev0).dot(h.normal) * h.reach0;
                    let ach1 = (want1 - prev1).dot(h.normal.scale(-1.0)) * h.reach1;
                    extra[0] = want0;
                    extra[1] = want1;
                    let rem0 = (0.5 * sep - ach0).max(0.0);
                    let rem1 = (0.5 * sep - ach1).max(0.0);
                    if rem0 > 1e-4 {
                        let step1 = h.normal.scale(-rem0 / h.reach1);
                        extra[1] = clamp_vec(extra[1] + step1, max_shift);
                    }
                    if rem1 > 1e-4 {
                        let step0 = h.normal.scale(rem1 / h.reach0);
                        extra[0] = clamp_vec(extra[0] + step0, max_shift);
                    }
                }
                locals = frames[f].clone();
                for ai in 0..2 {
                    if extra[ai].length() > 0.0 {
                        apply_arm(
                            &mut locals,
                            ai,
                            base[ai] + extra[ai],
                            base_frames[f][body.arms[ai].hand],
                        );
                    }
                }
                // Keep moved arms out of static body parts.
                let world2 = rig.world(&locals);
                for ai in 0..2 {
                    if extra[ai].length() <= 0.0 {
                        continue;
                    }
                    let hs = hits_with_slack(
                        body,
                        ai,
                        &world2,
                        use_w[ai],
                        margin,
                        src_slack.map(|s| s.cap_raw[f].as_slice()),
                        src_slack.map(|s| s.cap_margin[f].as_slice()),
                        src_slack.map(|s| s.surf_raw[f].as_slice()),
                        src_slack.map(|s| s.surf_margin[f].as_slice()),
                    );
                    if let Some(st) = hs
                        .iter()
                        .max_by(|a, b| (a.depth / a.reach).total_cmp(&(b.depth / b.reach)))
                    {
                        let step = st.normal.scale((st.depth + 0.5 * margin) / st.reach);
                        extra[ai] = clamp_vec(extra[ai] + step, max_shift);
                        if use_w[0] != use_w[1] && !use_w[ai] {
                            free_blocked = true;
                        }
                        apply_arm(
                            &mut locals,
                            ai,
                            base[ai] + extra[ai],
                            base_frames[f][body.arms[ai].hand],
                        );
                    }
                }
            }
            if extra[0].length() > 0.0 || extra[1].length() > 0.0 {
                any = true;
            }
            need[0][f] = extra[0];
            need[1][f] = extra[1];
        }
        if !any {
            break;
        }
        let smooth = [
            envelope(&need[0], times, ramp),
            envelope(&need[1], times, ramp),
        ];
        for f in 0..n {
            if smooth[0][f].length() <= 0.0 && smooth[1][f].length() <= 0.0 {
                continue;
            }
            let mut locals = base_frames[f].clone();
            for ai in 0..2 {
                if smooth[ai][f].length() > 0.0 {
                    offset[ai][f] = clamp_vec(offset[ai][f] + smooth[ai][f], max_shift);
                }
                if offset[ai][f].length() > 0.0 {
                    apply_arm(
                        &mut locals,
                        ai,
                        wrist0[ai][f] + offset[ai][f],
                        base_frames[f][body.arms[ai].hand],
                    );
                }
            }
            frames[f] = locals;
        }
    }

    let dt = if times.len() > 1 {
        ((times[times.len() - 1] - times[0]) / (times.len() - 1) as f64).max(1e-6)
    } else {
        1.0 / 30.0
    };
    let step = MAX_OFFSET_SPEED * body.height * dt;
    let final_off = [
        rate_limit(&steady_dir(&offset[0]), step),
        rate_limit(&steady_dir(&offset[1]), step),
    ];
    for f in 0..n {
        if final_off[0][f].length() <= 0.0 && final_off[1][f].length() <= 0.0 {
            continue;
        }
        let mut locals = base_frames[f].clone();
        for ai in 0..2 {
            if final_off[ai][f].length() > 0.0 {
                apply_arm(
                    &mut locals,
                    ai,
                    wrist0[ai][f] + final_off[ai][f],
                    base_frames[f][body.arms[ai].hand],
                );
            }
        }
        frames[f] = locals;
    }

    for ai in 0..2 {
        let changed = final_off[ai].iter().filter(|o| o.length() > 0.0).count();
        if changed == 0 {
            continue;
        }
        if let Some(rep) = report.arms.get_mut(ai) {
            let (fa, da, _) = measure_with_slack(body, ai, rig, frames, use_w[ai], src_slack);
            let msl = mesh_slacks.get(ai).map(Vec::as_slice);
            let (ma, mda, _) = mesh_measure_with_slack(body, ai, rig, frames, use_w[ai], msl);
            rep.frames_after = fa;
            rep.max_depth_after = da;
            rep.mesh_frames_after = ma.iter().filter(|&&x| x).count();
            rep.mesh_depth_after = mda;
            rep.frames_changed = rep.frames_changed.max(changed);
            let offs = &final_off[ai];
            let ms = offs.iter().map(|o| o.length()).fold(0.0, f64::max);
            rep.max_shift = rep.max_shift.max(ms);
        }
    }
}

/// Run the pass on one clip's frames (target locals per frame, sampled
/// at `times`). Edits `frames` in place. Proxies drive the fix; the exact
/// mesh check verifies it, and an arm that still cuts the mesh is
/// re-solved from the original with fatter proxies (x1.4, x1.9).
pub fn fix_clip(
    body: &Body,
    rig: &Rig,
    frames: &mut [Vec<Option<Trs>>],
    times: &[f64],
    clip: &str,
    opts: &Options,
) -> ClipContact {
    fix_clip_with_source(body, rig, frames, times, clip, opts, None)
}

/// [`fix_clip`] with optional source-body context so contacts already present
/// on the source body itself keep that depth as per-frame slack on the target.
pub fn fix_clip_with_source(
    body: &Body,
    rig: &Rig,
    frames: &mut [Vec<Option<Trs>>],
    times: &[f64],
    clip: &str,
    opts: &Options,
    source: Option<SourceFrames<'_>>,
) -> ClipContact {
    let weapon = opts.weapon_in(clip);
    let margin = opts.margin * body.height;
    let src_slack = source.and_then(|(sb, srig, sfr)| {
        if sfr.len() == frames.len() {
            Some(SourceSlack::build(
                body, rig, frames, sb, srig, sfr, weapon, opts,
            ))
        } else {
            None
        }
    });
    let mut report = ClipContact {
        weapon,
        arms: Vec::new(),
        height: body.height,
    };
    let mut mesh_slacks: Vec<Vec<f64>> = Vec::with_capacity(body.arms.len());
    for ai in 0..body.arms.len() {
        let arm = &body.arms[ai];
        let use_weapon = weapon && arm.weapon.is_some();
        let (fb, db, contacts) =
            measure_with_slack(body, ai, rig, frames, use_weapon, src_slack.as_ref());
        let (mb, mdb, worst_at, arm_mesh_slack) =
            mesh_measure_initial(body, ai, rig, frames, use_weapon, source);
        let mut rep = ArmReport {
            side: arm.side.clone(),
            frames_before: fb,
            max_depth_before: db,
            mesh_frames_before: mb.iter().filter(|&&x| x).count(),
            mesh_depth_before: mdb,
            worst_s: times.get(worst_at).copied().unwrap_or(0.0),
            contacts,
            ..Default::default()
        };
        if fb == 0 && rep.mesh_frames_before == 0 {
            mesh_slacks.push(arm_mesh_slack);
            report.arms.push(rep);
            continue;
        }
        // A free hand resting shallowly against the coat or a hip is left
        // where it is (pushing it out splayed the arm); the weapon arm is
        // always cleared (a forearm through the chest is shallow too).
        if !use_weapon && opts.min_depth > 0.0 && mdb < opts.min_depth * body.height {
            rep.frames_after = fb;
            rep.max_depth_after = db;
            rep.mesh_frames_after = rep.mesh_frames_before;
            rep.mesh_depth_after = mdb;
            mesh_slacks.push(arm_mesh_slack);
            report.arms.push(rep);
            continue;
        }
        let original: Vec<Vec<Option<Trs>>> = frames.to_vec();
        let mut best: Option<Attempt> = None;
        let sl = src_slack.as_ref();
        let msl = Some(arm_mesh_slack.as_slice());
        for grow in [1.0, 1.15, 1.3, 1.5, 1.75, 2.0, 2.4, 3.0] {
            let (fixed, offset, turns) = fix_arm(
                body, rig, ai, &original, times, use_weapon, margin, opts.ramp, grow, sl,
            );
            let (ma, mda, _) = mesh_measure_with_slack(body, ai, rig, &fixed, use_weapon, msl);
            let left = ma.iter().filter(|&&x| x).count();
            let better = best
                .as_ref()
                .is_none_or(|b| left < b.2 || (left == b.2 && mda < b.3));
            if better {
                best = Some((fixed, offset, left, mda, turns));
            }
            if left == 0 {
                break;
            }
        }
        let Some((_, offset, _, _, turns)) = best else {
            continue;
        };
        // The per-frame corrections may change only so fast: where the
        // body part to clear switches (a lying body's arm pushed out one
        // side, then the other), the chosen offset jumped tens of
        // centimetres in a frame or two, which reads as a twitch
        // (back-and-forth on the game's death: 0.2 -> 9 mm). Jumps are
        // spread out; corrections that already ease are left as they are.
        let dt = if times.len() > 1 {
            ((times[times.len() - 1] - times[0]) / (times.len() - 1) as f64).max(1e-6)
        } else {
            1.0 / 30.0
        };
        // The direction wobbled frame to frame within any speed limit (a
        // weighted mean of whichever surfaces were hit): smooth it, keep
        // each frame's size (the clearance).
        let offset = rate_limit(&steady_dir(&offset), MAX_OFFSET_SPEED * body.height * dt);
        let turns = rate_limit(&steady_dir(&turns), MAX_TURN_SPEED * dt);
        let arm_ = &body.arms[ai];
        let mut fixed = original.clone();
        for f in 0..fixed.len() {
            if offset[f].length() <= 0.0 && turns[f].length() <= 0.0 {
                continue;
            }
            let wrist = rig.world(&original[f])[arm_.hand].t;
            solve_arm_turn(
                rig,
                &mut fixed[f],
                arm_.upper,
                arm_.fore,
                arm_.hand,
                wrist + offset[f],
                rotvec_quat(turns[f]),
            );
            // A hand with no blade to aim turns with its forearm (keeping
            // its world rotation while the wrist moved up to 24 cm bent
            // wrists past their limits: on the game's clips, frames past a
            // limit 245 -> 509).
            if !use_weapon {
                fixed[f][arm_.hand] = original[f][arm_.hand];
            }
        }
        let (ma, mda, _) = mesh_measure_with_slack(body, ai, rig, &fixed, use_weapon, msl);
        let left = ma.iter().filter(|&&x| x).count();
        frames.clone_from_slice(&fixed);
        let (fa, da, _) = measure_with_slack(body, ai, rig, frames, use_weapon, src_slack.as_ref());
        rep.frames_after = fa;
        rep.max_depth_after = da;
        rep.mesh_frames_after = left;
        rep.mesh_depth_after = mda;
        rep.frames_changed = offset
            .iter()
            .zip(&turns)
            .filter(|(o, t)| o.length() > 0.0 || t.length() > 0.0)
            .count();
        rep.max_turn = turns.iter().map(|t| t.length()).fold(0.0, f64::max);
        rep.max_shift = offset.iter().map(|o| o.length()).fold(0.0, f64::max);
        mesh_slacks.push(arm_mesh_slack);
        report.arms.push(rep);
    }
    let sl = src_slack.as_ref();
    fix_arm_vs_arm(
        body,
        rig,
        frames,
        times,
        weapon,
        opts,
        sl,
        &mut report,
        &mesh_slacks,
    );
    report
}

#[cfg(test)]
mod tests {
    use super::super::ik::solve_arm;
    use super::super::slack::depth;
    use super::*;

    #[test]
    fn smoothstep_ends() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn arm_vs_shin_clears_reachable_crouch_without_moving_leg() {
        let doc = crate::fixture::humanoid(crate::fixture::Naming::Def, false, false).unwrap();
        let rig = Rig::from_doc(&doc).unwrap();
        let opts = Options {
            weapon_side: None,
            ..Options::default()
        };
        let body = Body::build(&doc, &rig, &opts).unwrap().unwrap();
        let shin_cap = body
            .static_parts
            .iter()
            .find(|c| c.name == "shin.L")
            .expect("shin.L proxy");
        let arm_l = body.arms.iter().find(|a| a.side == "L").unwrap();
        let sp = rig.find("DEF-spine.001").unwrap();
        let th = rig.find("DEF-thigh.L").unwrap();
        let sh = rig.find("DEF-shin.L").unwrap();
        let ft = rig.find("DEF-foot.L").unwrap();

        let mut crouch = vec![None; rig.names.len()];
        let mut s_trs = rig.rest[sp];
        s_trs.r = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 35f64.to_radians());
        crouch[sp] = Some(s_trs);
        let mut t_trs = rig.rest[th];
        t_trs.r = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -110f64.to_radians());
        crouch[th] = Some(t_trs);
        let mut k_trs = rig.rest[sh];
        k_trs.r = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 75f64.to_radians());
        crouch[sh] = Some(k_trs);

        let cw = rig.world(&crouch);
        let shin_mid = (cw[sh].t + cw[ft].t).scale(0.5);
        let w_far = cw[arm_l.hand].t;
        let times: Vec<f64> = (0..15).map(|i| i as f64 / 30.0).collect();
        let mut frames = Vec::with_capacity(times.len());
        for i in 0..times.len() {
            let mut loc = crouch.clone();
            let s = 1.0 - ((i as f64 - 7.0) / 7.0).abs();
            let target = w_far + (shin_mid - w_far).scale(s);
            solve_arm(&rig, &mut loc, arm_l.upper, arm_l.fore, arm_l.hand, target);
            frames.push(loc);
        }
        let orig = frames.clone();
        let pen_of = |fr: &[Option<Trs>]| -> f64 {
            let w = rig.world(fr);
            let p_fo = depth(&arm_l.forearm, shin_cap, &body.inv_rest, &w, 0.0).0;
            let p_ha = depth(&arm_l.palm, shin_cap, &body.inv_rest, &w, 0.0).0;
            p_fo.max(p_ha)
        };
        let before = frames.iter().map(|f| pen_of(f)).fold(0.0f64, f64::max);
        assert!(before > 0.02, "expected shin hit before fix, got {before}");

        fix_clip(&body, &rig, &mut frames, &times, "dodge_shin", &opts);
        for (i, fr) in frames.iter().enumerate() {
            let after = pen_of(fr);
            assert!(after <= 0.002, "frame {i}: shin pen {after} > 0.002");
            for leg in [th, sh] {
                let r0 = orig[i][leg].unwrap_or(rig.rest[leg]).r;
                let r1 = fr[leg].unwrap_or(rig.rest[leg]).r;
                assert!(r0.angle_to(r1) < 1e-6, "leg joint moved on frame {i}");
            }
        }
    }

    #[test]
    fn steady_dir_and_envelope_handle_empty_and_mismatched_inputs() {
        assert!(steady_dir(&[]).is_empty());
        assert!(rate_limit(&[], 0.1).is_empty());
        assert!(envelope(&[], &[], 0.15).is_empty());
        assert_eq!(envelope(&[Vec3::new(1.0, 0.0, 0.0)], &[], 0.15).len(), 0);
    }

    #[test]
    fn fix_clip_handles_empty_frames() {
        let doc = crate::fixture::humanoid(crate::fixture::Naming::Def, false, false).unwrap();
        let rig = Rig::from_doc(&doc).unwrap();
        let opts = Options::default();
        let body = Body::build(&doc, &rig, &opts).unwrap().unwrap();
        let mut frames: Vec<Vec<Option<Trs>>> = Vec::new();
        let report = fix_clip(&body, &rig, &mut frames, &[], "attack_01", &opts);
        assert!(!report.changed());
        assert!(frames.is_empty());
    }
}
