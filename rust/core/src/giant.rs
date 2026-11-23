//! Giant animation pass (`motionforge giant --scale k`):
//!
//! Scales a rigged GLB and its clips by `k` while applying Froude time
//! scaling (`sqrt(k)`), anticipation / hit-stop / heavy damped settle timing,
//! hit-stop & cancel-window event metadata in glTF animation `extras`, and an
//! optional pinned-pelvis or arms-only body mask.
//!
//! Deterministic: uses `crate::detmath` and `f64::sqrt` exclusively (no
//! platform libm transcendentals, no hash-map iteration in output paths).

use crate::clean::{clean_clip, written_nodes, CleanOptions, LOCK_RAMP};
use crate::contact::{Body, Options as ContactOptions};
use crate::detmath;
use crate::glb::{canonical_quat, clean, Document};
use crate::humanoid::{self, Mapping};
use crate::json::Json;
use crate::math::Vec3;
use crate::motioncheck::{self, ATTACK_WORDS, CONTACT_HEIGHT, PLANT_SPEED, REF_HEIGHT};
use crate::rig::{Animation, Rig, Trs};
use crate::stylize_body::{encode_clip, probe_nodes};

const DEFAULT_FPS: f64 = 30.0;

/// Which parts of the body animate in the giant pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMask {
    /// Full body animates.
    Full,
    /// Pelvis and all lower-body (leg) bones stay pinned at rest while upper
    /// spine, head, shoulders, arms, hands, fingers, and sockets animate.
    ArmsOnly,
}

/// Settings for [`giant_glb`].
#[derive(Clone, Debug)]
pub struct GiantOptions {
    /// Uniform rig, mesh, and root-translation scale factor `k` (> 0).
    pub scale: f64,
    /// Windup anticipation weighting (>= 0; 0 disables anticipation warp).
    pub anticipation: f64,
    /// Heavy damped settle weighting (>= 0; 0 disables settle warp).
    pub settle: f64,
    /// Hit-stop freeze duration at peak motion, seconds (>= 0).
    pub hit_stop_s: f64,
    /// Cancel window duration immediately after hit-stop, seconds (>= 0).
    pub cancel_window_s: f64,
    /// Body animation mask (`Full` or `ArmsOnly`).
    pub mask: BodyMask,
    /// Pin `DEF-spine` (pelvis/hips) at rest on every frame.
    pub pin_pelvis: bool,
    /// Re-plant / preserve grounded foot plants on full-body clips.
    pub replant_feet: bool,
}

impl Default for GiantOptions {
    fn default() -> Self {
        GiantOptions {
            scale: 4.0,
            anticipation: 0.35,
            settle: 0.35,
            hit_stop_s: 0.12,
            cancel_window_s: 0.25,
            mask: BodyMask::Full,
            pin_pelvis: false,
            replant_feet: true,
        }
    }
}

impl GiantOptions {
    fn validate(&self) -> Result<(), String> {
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return Err(format!("scale must be finite and > 0, got {}", self.scale));
        }
        if !self.anticipation.is_finite() || self.anticipation < 0.0 {
            return Err(format!(
                "anticipation must be finite and >= 0, got {}",
                self.anticipation
            ));
        }
        if !self.settle.is_finite() || self.settle < 0.0 {
            return Err(format!(
                "settle must be finite and >= 0, got {}",
                self.settle
            ));
        }
        if !self.hit_stop_s.is_finite() || self.hit_stop_s < 0.0 {
            return Err(format!(
                "hit_stop_s must be finite and >= 0, got {}",
                self.hit_stop_s
            ));
        }
        if !self.cancel_window_s.is_finite() || self.cancel_window_s < 0.0 {
            return Err(format!(
                "cancel_window_s must be finite and >= 0, got {}",
                self.cancel_window_s
            ));
        }
        Ok(())
    }
}

/// Summary of one clip processed by [`giant_glb`].
#[derive(Clone, Debug)]
pub struct GiantReport {
    pub name: String,
    pub source: String,
    pub scale: f64,
    pub source_duration_s: f64,
    pub output_duration_s: f64,
    pub root_travel_before_m: f64,
    pub root_travel_after_m: f64,
    pub hit_time_s: Option<f64>,
    pub hit_stop: Option<(f64, f64)>,
    pub cancel_window: Option<(f64, f64)>,
    pub mask: BodyMask,
    pub pin_pelvis: bool,
}

/// Monotone time-warp `[0, out_duration] -> [0, src_duration]` with
/// anticipation windup, impact snap, hit-stop hold, and damped settle.
#[derive(Clone, Copy, Debug)]
struct TimeWarp {
    src_duration: f64,
    out_duration: f64,
    src_hit: f64,
    hs_start: f64,
    hs_end: f64,
    anticipation: f64,
    settle: f64,
    plain: bool,
}

impl TimeWarp {
    fn new(src_duration: f64, src_hit: f64, opts: &GiantOptions) -> Self {
        let time_scale = f64::sqrt(opts.scale);
        let out_duration = src_duration * time_scale;
        let plain = opts.anticipation == 0.0 && opts.settle == 0.0 && opts.hit_stop_s == 0.0;
        if src_duration <= 0.0 || out_duration <= 0.0 {
            return TimeWarp {
                src_duration: 0.0,
                out_duration: 0.0,
                src_hit: 0.0,
                hs_start: 0.0,
                hs_end: 0.0,
                anticipation: opts.anticipation,
                settle: opts.settle,
                plain,
            };
        }
        let u_hit = (src_hit / src_duration).clamp(0.15, 0.85);
        let clamped_src_hit = u_hit * src_duration;
        if plain {
            let t_hit = u_hit * out_duration;
            return TimeWarp {
                src_duration,
                out_duration,
                src_hit: clamped_src_hit,
                hs_start: t_hit,
                hs_end: t_hit,
                anticipation: 0.0,
                settle: 0.0,
                plain: true,
            };
        }
        let hs_dur = if opts.hit_stop_s > 0.0 {
            opts.hit_stop_s.min(0.25 * out_duration)
        } else {
            0.0
        };
        let move_dur = (out_duration - hs_dur).max(0.0);
        let shift =
            (0.06 * opts.anticipation.min(1.0) - 0.04 * opts.settle.min(1.0)).clamp(-0.08, 0.10);
        let u_pre = (u_hit + shift).clamp(0.15, 0.82);
        let hs_start = u_pre * move_dur;
        let hs_end = (hs_start + hs_dur).min(out_duration);
        TimeWarp {
            src_duration,
            out_duration,
            src_hit: clamped_src_hit,
            hs_start,
            hs_end,
            anticipation: opts.anticipation,
            settle: opts.settle,
            plain: false,
        }
    }

    /// Evaluate `(t_src, counter_load_weight)` at output time `t_out`.
    fn eval(&self, t_out: f64) -> (f64, f64) {
        if self.out_duration <= 0.0 || self.src_duration <= 0.0 {
            return (0.0, 0.0);
        }
        let t = t_out.clamp(0.0, self.out_duration);
        if self.plain {
            return ((t / self.out_duration) * self.src_duration, 0.0);
        }
        if t < self.hs_start {
            let u = if self.hs_start > 1e-12 {
                (t / self.hs_start).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let alpha = (self.anticipation * 1.4).clamp(0.0, 0.85);
            let warped = (1.0 - alpha) * u + alpha * u * u * u;
            let cl = if self.anticipation > 0.0 {
                0.12 * self.anticipation.min(1.0) * detmath::sin(std::f64::consts::PI * u)
            } else {
                0.0
            };
            (warped * self.src_hit, cl)
        } else if t <= self.hs_end {
            (self.src_hit, 0.0)
        } else {
            let post_dur = self.out_duration - self.hs_end;
            let v = if post_dur > 1e-12 {
                ((t - self.hs_end) / post_dur).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let beta = (self.settle * 1.4).clamp(0.0, 0.85);
            let denom = 1.0 - detmath::exp(-4.0);
            let damp = (1.0 - detmath::exp(-4.0 * v)) / denom;
            let warped = (1.0 - beta) * v + beta * damp;
            (
                self.src_hit + warped * (self.src_duration - self.src_hit),
                0.0,
            )
        }
    }
}

fn smooth3(raw: &[f64]) -> Vec<f64> {
    let n = raw.len();
    (0..n)
        .map(|f| {
            let a = raw[f.saturating_sub(1)];
            let c = raw[(f + 1).min(n.saturating_sub(1))];
            (a + 2.0 * raw[f] + c) / 4.0
        })
        .collect()
}

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

fn is_strike_name(name: &str) -> bool {
    let low = name.to_ascii_lowercase();
    ATTACK_WORDS.iter().any(|w| low.contains(w))
        || [
            "cleave", "backcut", "sweep", "thrust", "smash", "slam", "chop", "heavy", "reap",
        ]
        .iter()
        .any(|w| low.contains(w))
}

fn detect_peak_time(
    rig: &Rig,
    map: &Mapping,
    hips: usize,
    height: f64,
    anim: &Animation,
) -> (f64, bool) {
    if anim.duration <= 0.0 {
        return (0.0, false);
    }
    let (times, frames) = motioncheck::sample(rig, anim, Some(DEFAULT_FPS));
    let n = frames.len();
    if n < 3 {
        return (0.5 * anim.duration, is_strike_name(&anim.name));
    }
    let world: Vec<_> = frames.iter().map(|f| rig.world(f)).collect();

    let mut hand_nodes = Vec::new();
    for name in ["DEF-hand.R", "DEF-hand.L"] {
        if let Some(nd) = map.node_of(name) {
            hand_nodes.push(nd);
        }
    }
    for name in ["Socket_Hand_R", "Socket_Hand_L"] {
        if let Some(nd) = rig.find(name) {
            if !hand_nodes.contains(&nd) {
                hand_nodes.push(nd);
            }
        }
    }

    let raw_hand: Vec<f64> = (0..n)
        .map(|f| {
            let (a, b) = (f.saturating_sub(1), (f + 1).min(n - 1));
            let dt = (times[b] - times[a]).max(1e-9);
            hand_nodes
                .iter()
                .map(|&h| {
                    let pa = world[a][h].t - world[a][hips].t;
                    let pb = world[b][h].t - world[b][hips].t;
                    (pb - pa).length() / dt
                })
                .fold(0.0_f64, f64::max)
        })
        .collect();
    let v_hand = smooth3(&raw_hand);

    let mut probes = probe_nodes(map);
    if probes.is_empty() {
        probes = rig.joints();
    }
    let raw_body: Vec<f64> = (0..n)
        .map(|f| {
            let (a, b) = (f.saturating_sub(1), (f + 1).min(n - 1));
            let dt = (times[b] - times[a]).max(1e-9);
            probes
                .iter()
                .map(|&p| {
                    let (pa, pb) = if p == hips {
                        (world[a][p].t, world[b][p].t)
                    } else {
                        (
                            world[a][p].t - world[a][hips].t,
                            world[b][p].t - world[b][hips].t,
                        )
                    };
                    (pb - pa).length() / dt
                })
                .sum()
        })
        .collect();
    let v_body = smooth3(&raw_body);

    let peak_hand = v_hand[1..n - 1].iter().copied().fold(0.0_f64, f64::max);
    let strike_like =
        !hand_nodes.is_empty() && (is_strike_name(&anim.name) || peak_hand > 0.6 * height);
    let sig = if strike_like { &v_hand } else { &v_body };

    let mut best_f = n / 2;
    let mut best_v = -1.0_f64;
    for (f, &v) in sig.iter().enumerate().take(n - 1).skip(1) {
        if v > best_v {
            best_v = v;
            best_f = f;
        }
    }
    let t_peak = if best_v <= 1e-9 {
        0.5 * anim.duration
    } else {
        times[best_f].clamp(0.15 * anim.duration, 0.85 * anim.duration)
    };
    (t_peak, strike_like)
}

struct LegInfo {
    th: usize,
    sh: usize,
    ft: usize,
    points: Vec<(usize, f64)>,
}

fn leg_infos(rig: &Rig, map: &Mapping) -> Vec<LegInfo> {
    let mut out = Vec::new();
    for side in ["L", "R"] {
        let get = |b: &str| map.node_of(&format!("DEF-{b}.{side}"));
        let (Some(th), Some(sh), Some(ft)) = (get("thigh"), get("shin"), get("foot")) else {
            continue;
        };
        let mut points = vec![(ft, rig.head(ft).y)];
        if let Some(t) = get("toe") {
            points.push((t, rig.head(t).y));
        }
        out.push(LegInfo { th, sh, ft, points });
    }
    out
}

fn lowest_point(leg: &LegInfo, world: &[crate::glb::Affine]) -> (usize, f64) {
    leg.points
        .iter()
        .map(|&(nd, y)| (nd, world[nd].t.y - y))
        .fold(
            (leg.ft, f64::INFINITY),
            |a, b| if b.1 < a.1 { b } else { a },
        )
}

/// Per leg, which source frames were planted under `motioncheck`'s thresholds.
fn source_planted_masks(
    rig: &Rig,
    map: &Mapping,
    hips: usize,
    height: f64,
    times: &[f64],
    frames: &[Vec<Option<Trs>>],
) -> Vec<Vec<bool>> {
    let legs = leg_infos(rig, map);
    let n = frames.len();
    if n < 2 || legs.is_empty() {
        return Vec::new();
    }
    let k = height / REF_HEIGHT;
    let dt = ((times[n - 1] - times[0]) / (n - 1) as f64).max(1e-9);
    let world: Vec<_> = frames.iter().map(|f| rig.world(f)).collect();
    let pos = |f: usize, nd: usize| world[f][nd].t;
    let belt = motioncheck::treadmill(
        &legs
            .iter()
            .map(|l| {
                (0..n)
                    .map(|f| {
                        l.points
                            .iter()
                            .map(|&(nd, y)| (pos(f, nd), pos(f, nd).y - y))
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<_>>(),
        &(0..n).map(|f| pos(f, hips)).collect::<Vec<_>>(),
        dt,
        k,
    );
    let t0 = times[0];
    let fpos = |f: usize, nd: usize| pos(f, nd) - belt.scale(times[f] - t0);
    legs.iter()
        .map(|leg| {
            let low: Vec<(usize, f64)> = (0..n).map(|f| lowest_point(leg, &world[f])).collect();
            (0..n)
                .map(|f| {
                    let nd = low[f].0;
                    let (a, b) = (f.saturating_sub(1), (f + 1).min(n - 1));
                    let spd = if a == b {
                        0.0
                    } else {
                        flat(fpos(b, nd) - fpos(a, nd)).length() / ((b - a) as f64 * dt)
                    };
                    low[f].1 < CONTACT_HEIGHT * k && spd < PLANT_SPEED * k
                })
                .collect()
        })
        .collect()
}

/// Give swinging feet that skimmed just under `CONTACT_HEIGHT` in the source
/// clip enough vertical clearance when Froude slowdown drops their horizontal
/// speed below `PLANT_SPEED`, so `motioncheck` does not misread swing phases
/// as skating plants. Source-planted frames are never lifted.
#[allow(clippy::too_many_arguments)]
fn clear_skimming_swings(
    rig: &Rig,
    map: &Mapping,
    hips: usize,
    height: f64,
    src_times: &[f64],
    src_planted: &[Vec<bool>],
    out_times: &[f64],
    out_src_times: &[f64],
    out_frames: &mut [Vec<Option<Trs>>],
) {
    let legs = leg_infos(rig, map);
    let n = out_frames.len();
    let n_src = src_times.len();
    if n < 2 || n_src < 2 || legs.len() != src_planted.len() {
        return;
    }
    let k = height / REF_HEIGHT;
    let dt = ((out_times[n - 1] - out_times[0]) / (n - 1) as f64).max(1e-9);
    let world: Vec<_> = out_frames.iter().map(|f| rig.world(f)).collect();
    let pos = |f: usize, nd: usize| world[f][nd].t;
    let belt = motioncheck::treadmill(
        &legs
            .iter()
            .map(|l| {
                (0..n)
                    .map(|f| {
                        l.points
                            .iter()
                            .map(|&(nd, y)| (pos(f, nd), pos(f, nd).y - y))
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<_>>(),
        &(0..n).map(|f| pos(f, hips)).collect::<Vec<_>>(),
        dt,
        k,
    );
    let t0 = out_times[0];
    let fpos = |f: usize, nd: usize| pos(f, nd) - belt.scale(out_times[f] - t0);
    let src_span = (src_times[n_src - 1] - src_times[0]).max(1e-9);
    let up = Vec3::new(0.0, 1.0, 0.0);

    for (li, leg) in legs.iter().enumerate() {
        let sp = &src_planted[li];
        let low: Vec<(usize, f64)> = (0..n).map(|f| lowest_point(leg, &world[f])).collect();
        let was_planted: Vec<bool> = (0..n)
            .map(|g| {
                let u = ((out_src_times[g] - src_times[0]) / src_span).clamp(0.0, 1.0);
                let pos_f = u * (n_src - 1) as f64;
                let f0 = (pos_f.floor() as usize).min(n_src - 1);
                let f1 = (f0 + 1).min(n_src - 1);
                sp[f0] || sp[f1]
            })
            .collect();
        let mut need = vec![0.0_f64; n];
        for g in 0..n {
            if was_planted[g] {
                continue;
            }
            let nd = low[g].0;
            let (a, b) = (g.saturating_sub(1), (g + 1).min(n - 1));
            let spd = if a == b {
                0.0
            } else {
                flat(fpos(b, nd) - fpos(a, nd)).length() / ((b - a) as f64 * dt)
            };
            if low[g].1 < CONTACT_HEIGHT * k && spd < PLANT_SPEED * k {
                need[g] = (1.15 * CONTACT_HEIGHT * k - low[g].1).max(0.0);
            }
        }
        let lift: Vec<f64> = (0..n)
            .map(|f| {
                if was_planted[f] {
                    return 0.0;
                }
                let lo = f.saturating_sub(LOCK_RAMP);
                let hi = (f + LOCK_RAMP).min(n - 1);
                (lo..=hi)
                    .map(|g| {
                        let d = f.abs_diff(g) as f64 / (LOCK_RAMP + 1) as f64;
                        need[g] * smoothstep(1.0 - d)
                    })
                    .fold(0.0, f64::max)
            })
            .collect();
        for (g, fr) in out_frames.iter_mut().enumerate() {
            if lift[g] < 1e-4 * k {
                continue;
            }
            let w = rig.world(fr);
            let foot_rot = w[leg.ft].rotation();
            let target = w[leg.ft].t + up.scale(lift[g]);
            crate::contact::solve_arm(rig, fr, leg.th, leg.sh, leg.ft, target);
            let w2 = rig.world(fr);
            let mut l = fr[leg.ft].unwrap_or(rig.rest[leg.ft]);
            l.r = canonical_quat(w2[leg.sh].rotation().conj().mul(foot_rot));
            fr[leg.ft] = Some(l);
        }
    }
}

fn horizontal_travel(rig: &Rig, frames: &[Vec<Option<Trs>>], hips: usize) -> f64 {
    let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
        return 0.0;
    };
    let a = rig.world(first)[hips].t;
    let b = rig.world(last)[hips].t;
    Vec3::new(b.x - a.x, 0.0, b.z - a.z).length()
}

fn rig_height(doc: &Document, rig: &Rig) -> Result<f64, String> {
    if let Some(body) = Body::build(doc, rig, &ContactOptions::default())? {
        return Ok(body.height);
    }
    let ys: Vec<f64> = rig.joints().iter().map(|&j| rig.head(j).y).collect();
    let lo = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok(((hi - lo) * 1.08).max(1e-6))
}

/// Uniformly scale all node rest translations, mesh `POSITION` accessors (and
/// morph target `POSITION`s), skin `inverseBindMatrices` translation columns,
/// and animation `translation` channel outputs by `k`.
fn scale_document(doc: &mut Document, k: f64) -> Result<(), String> {
    // 1. Node rest translations (keep rotation and scale unchanged).
    if let Some(nodes) = doc.json.get_mut("nodes").and_then(Json::as_arr_mut) {
        for node in nodes.iter_mut() {
            if let Some(mat) = node.get_mut("matrix").and_then(Json::as_arr_mut) {
                if mat.len() == 16 {
                    for idx in [12, 13, 14] {
                        if let Some(v) = mat[idx].as_f64() {
                            mat[idx] = Json::num(clean(v * k));
                        }
                    }
                }
            } else if let Some(t) = node.get_mut("translation").and_then(Json::as_arr_mut) {
                if t.len() == 3 {
                    for v in t.iter_mut() {
                        if let Some(x) = v.as_f64() {
                            *v = Json::num(clean(x * k));
                        }
                    }
                }
            }
        }
    }

    // 2. Mesh primitive POSITION accessors (including morph targets) + min/max.
    let mut pos_accs: Vec<usize> = Vec::new();
    for mesh in doc.array("meshes") {
        for prim in mesh.get("primitives").and_then(Json::as_arr).unwrap_or(&[]) {
            if let Some(acc) = prim
                .get("attributes")
                .and_then(|a| a.get("POSITION"))
                .and_then(Json::as_usize)
            {
                if !pos_accs.contains(&acc) {
                    pos_accs.push(acc);
                }
            }
            for target in prim.get("targets").and_then(Json::as_arr).unwrap_or(&[]) {
                if let Some(acc) = target.get("POSITION").and_then(Json::as_usize) {
                    if !pos_accs.contains(&acc) {
                        pos_accs.push(acc);
                    }
                }
            }
        }
    }
    let mut pos_remap: Vec<(usize, usize)> = Vec::new();
    for old_acc in pos_accs {
        let old_target = doc
            .array("accessors")
            .get(old_acc)
            .and_then(|a| a.get("bufferView"))
            .and_then(Json::as_usize)
            .and_then(|vi| doc.array("bufferViews").get(vi))
            .and_then(|v| v.get("target"))
            .and_then(Json::as_usize);
        let rows = doc.read_accessor(old_acc)?;
        let scaled: Vec<Vec<f64>> = rows
            .into_iter()
            .map(|r| r.into_iter().map(|v| v * k).collect())
            .collect();
        let new_acc = doc.push_float_accessor(&scaled, "VEC3", true);
        if let Some(t) = old_target {
            if let Some(vi) = doc
                .array("accessors")
                .get(new_acc)
                .and_then(|a| a.get("bufferView"))
                .and_then(Json::as_usize)
            {
                doc.array_mut("bufferViews")[vi].set("target", Json::num(t as f64));
            }
        }
        pos_remap.push((old_acc, new_acc));
    }
    let lookup = |map: &[(usize, usize)], old: usize| -> Option<usize> {
        map.iter().find(|&&(a, _)| a == old).map(|&(_, b)| b)
    };
    if let Some(meshes) = doc.json.get_mut("meshes").and_then(Json::as_arr_mut) {
        for mesh in meshes.iter_mut() {
            if let Some(prims) = mesh.get_mut("primitives").and_then(Json::as_arr_mut) {
                for prim in prims.iter_mut() {
                    if let Some(attrs) = prim.get_mut("attributes") {
                        if let Some(old) = attrs.get("POSITION").and_then(Json::as_usize) {
                            if let Some(new_acc) = lookup(&pos_remap, old) {
                                attrs.set("POSITION", Json::num(new_acc as f64));
                            }
                        }
                    }
                    if let Some(targets) = prim.get_mut("targets").and_then(Json::as_arr_mut) {
                        for target in targets.iter_mut() {
                            if let Some(old) = target.get("POSITION").and_then(Json::as_usize) {
                                if let Some(new_acc) = lookup(&pos_remap, old) {
                                    target.set("POSITION", Json::num(new_acc as f64));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Skin inverseBindMatrices translation column (col 3: indices 12, 13, 14).
    let mut ibm_accs: Vec<usize> = Vec::new();
    for skin in doc.array("skins") {
        if let Some(acc) = skin.get("inverseBindMatrices").and_then(Json::as_usize) {
            if !ibm_accs.contains(&acc) {
                ibm_accs.push(acc);
            }
        }
    }
    let mut ibm_remap: Vec<(usize, usize)> = Vec::new();
    for old_acc in ibm_accs {
        let mut rows = doc.read_accessor(old_acc)?;
        for row in rows.iter_mut() {
            if row.len() == 16 {
                row[12] *= k;
                row[13] *= k;
                row[14] *= k;
            }
        }
        let new_acc = doc.push_float_accessor(&rows, "MAT4", false);
        ibm_remap.push((old_acc, new_acc));
    }
    if let Some(skins) = doc.json.get_mut("skins").and_then(Json::as_arr_mut) {
        for skin in skins.iter_mut() {
            if let Some(old) = skin.get("inverseBindMatrices").and_then(Json::as_usize) {
                if let Some(new_acc) = lookup(&ibm_remap, old) {
                    skin.set("inverseBindMatrices", Json::num(new_acc as f64));
                }
            }
        }
    }

    // 4. All animation translation channel outputs.
    let n_anims = doc.array("animations").len();
    for ai in 0..n_anims {
        let mut trans_samplers: Vec<usize> = Vec::new();
        let mut other_samplers: Vec<usize> = Vec::new();
        {
            let anim = &doc.array("animations")[ai];
            for ch in anim.get("channels").and_then(Json::as_arr).unwrap_or(&[]) {
                let Some(path) = ch
                    .get("target")
                    .and_then(|t| t.get("path"))
                    .and_then(Json::as_str)
                else {
                    continue;
                };
                let Some(si) = ch.get("sampler").and_then(Json::as_usize) else {
                    continue;
                };
                if path == "translation" {
                    if !trans_samplers.contains(&si) {
                        trans_samplers.push(si);
                    }
                } else if !other_samplers.contains(&si) {
                    other_samplers.push(si);
                }
            }
        }
        let mut sampler_updates: Vec<(usize, usize, bool)> = Vec::new();
        for si in trans_samplers {
            let old_acc = doc.array("animations")[ai]
                .get("samplers")
                .and_then(Json::as_arr)
                .and_then(|s| s.get(si))
                .and_then(|s| s.get("output"))
                .and_then(Json::as_usize);
            let Some(old_acc) = old_acc else {
                continue;
            };
            let rows = doc.read_accessor(old_acc)?;
            let scaled: Vec<Vec<f64>> = rows
                .into_iter()
                .map(|r| r.into_iter().map(|v| v * k).collect())
                .collect();
            let new_acc = doc.push_float_accessor(&scaled, "VEC3", false);
            sampler_updates.push((si, new_acc, other_samplers.contains(&si)));
        }
        if !sampler_updates.is_empty() {
            let anim = &mut doc.array_mut("animations")[ai];
            let mut cloned_sampler_map: Vec<(usize, usize)> = Vec::new();
            if let Some(samplers) = anim.get_mut("samplers").and_then(Json::as_arr_mut) {
                for (si, new_acc, shared) in sampler_updates {
                    if shared {
                        if let Some(orig) = samplers.get(si).cloned() {
                            let mut copy = orig;
                            copy.set("output", Json::num(new_acc as f64));
                            let new_si = samplers.len();
                            samplers.push(copy);
                            cloned_sampler_map.push((si, new_si));
                        }
                    } else if let Some(s) = samplers.get_mut(si) {
                        s.set("output", Json::num(new_acc as f64));
                    }
                }
            }
            if !cloned_sampler_map.is_empty() {
                if let Some(channels) = anim.get_mut("channels").and_then(Json::as_arr_mut) {
                    for ch in channels.iter_mut() {
                        let is_trans = ch
                            .get("target")
                            .and_then(|t| t.get("path"))
                            .and_then(Json::as_str)
                            == Some("translation");
                        if is_trans {
                            if let Some(si) = ch.get("sampler").and_then(Json::as_usize) {
                                if let Some(new_si) = lookup(&cloned_sampler_map, si) {
                                    ch.set("sampler", Json::num(new_si as f64));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Nodes pinned at rest for `mask` / `pin_pelvis`.
fn pinned_nodes(
    rig: &Rig,
    map: &Mapping,
    hips: usize,
    mask: BodyMask,
    pin_pelvis: bool,
) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut add = |nd: usize| {
        if !out.contains(&nd) {
            out.push(nd);
        }
    };
    if pin_pelvis || mask == BodyMask::ArmsOnly {
        add(hips);
        let mut cur = rig.parent[hips];
        while let Some(p) = cur {
            add(p);
            cur = rig.parent[p];
        }
    }
    if mask == BodyMask::ArmsOnly {
        let mut leg_roots: Vec<usize> = Vec::new();
        for side in ["L", "R"] {
            for b in ["pelvis", "thigh", "thigh_twist", "shin", "foot", "toe"] {
                if let Some(nd) = map.node_of(&format!("DEF-{b}.{side}")) {
                    add(nd);
                    if !leg_roots.contains(&nd) {
                        leg_roots.push(nd);
                    }
                }
            }
        }
        // Any descendants of lower-body bones are also pinned.
        for nd in 0..rig.names.len() {
            if leg_roots.iter().any(|&root| rig.is_ancestor(root, nd)) {
                add(nd);
            }
        }
    }
    out.sort_unstable();
    out
}

/// Upper-body / arm nodes that receive subtle anticipation counter-load on
/// strike clips.
fn counter_load_nodes(map: &Mapping) -> Vec<usize> {
    let mut out = Vec::new();
    for name in [
        "DEF-spine.001",
        "DEF-spine.002",
        "DEF-spine.003",
        "DEF-shoulder.L",
        "DEF-shoulder.R",
        "DEF-upper_arm.L",
        "DEF-upper_arm.R",
        "DEF-forearm.L",
        "DEF-forearm.R",
    ] {
        if let Some(nd) = map.node_of(name) {
            if !out.contains(&nd) {
                out.push(nd);
            }
        }
    }
    out
}

fn giant_extras_json(
    opts: &GiantOptions,
    effective_pin_pelvis: bool,
    source_duration_s: f64,
    output_duration_s: f64,
    hit_time_s: Option<f64>,
    hit_stop: Option<(f64, f64)>,
    cancel_window: Option<(f64, f64)>,
) -> Json {
    let mask_str = match opts.mask {
        BodyMask::Full => "full",
        BodyMask::ArmsOnly => "arms-only",
    };
    let mut events = Vec::new();
    if let Some((s, e)) = hit_stop {
        events.push(Json::obj(vec![
            ("kind", Json::str("hit_stop")),
            ("start_s", Json::num(clean(s))),
            ("end_s", Json::num(clean(e))),
        ]));
    }
    if let Some((s, e)) = cancel_window {
        events.push(Json::obj(vec![
            ("kind", Json::str("cancel_window")),
            ("start_s", Json::num(clean(s))),
            ("end_s", Json::num(clean(e))),
        ]));
    }
    Json::obj(vec![
        ("scale", Json::num(clean(opts.scale))),
        ("mask", Json::str(mask_str)),
        ("pin_pelvis", Json::Bool(effective_pin_pelvis)),
        ("source_duration_s", Json::num(clean(source_duration_s))),
        ("duration_s", Json::num(clean(output_duration_s))),
        ("hit_time_s", Json::num(clean(hit_time_s.unwrap_or(0.0)))),
        ("events", Json::Arr(events)),
    ])
}

/// Apply the giant pass to a rigged GLB (`clips` empty = every clip).
///
/// Scales the rig, mesh, skins, and clip root translations by `opts.scale`,
/// retimes selected clips by Froude `sqrt(scale)` with anticipation /
/// hit-stop / heavy damped settle, writes `extras.motionforge_giant` event
/// metadata on each processed animation, and applies any body mask / pelvis
/// pin. With `suffix`, processed clips are appended as `<name><suffix>`;
/// without, they replace the originals.
pub fn giant_glb(
    doc: &Document,
    opts: &GiantOptions,
    clips: &[String],
    suffix: Option<&str>,
) -> Result<(Document, Vec<GiantReport>), String> {
    opts.validate()?;

    let orig_rig = Rig::from_doc(doc)?;
    let orig_map = humanoid::map_rig(&orig_rig);
    let orig_hips = orig_map.node_of("DEF-spine").ok_or("no hips bone found")?;
    let orig_height = rig_height(doc, &orig_rig)?;
    let orig_anims = Animation::load_all(doc)?;
    for c in clips {
        if !orig_anims.iter().any(|a| &a.name == c) {
            return Err(format!("no clip named '{}'", c));
        }
    }

    // Measure original root travel and source plant masks before scaling.
    let mut orig_travels: Vec<f64> = Vec::with_capacity(orig_anims.len());
    let mut orig_src_times: Vec<Vec<f64>> = Vec::with_capacity(orig_anims.len());
    let mut orig_src_planted: Vec<Vec<Vec<bool>>> = Vec::with_capacity(orig_anims.len());
    for anim in &orig_anims {
        let (times, frames) = motioncheck::sample(&orig_rig, anim, Some(DEFAULT_FPS));
        orig_travels.push(horizontal_travel(&orig_rig, &frames, orig_hips));
        orig_src_planted.push(source_planted_masks(
            &orig_rig,
            &orig_map,
            orig_hips,
            orig_height,
            &times,
            &frames,
        ));
        orig_src_times.push(times);
    }
    let orig_extras: Vec<Option<Json>> = doc
        .array("animations")
        .iter()
        .map(|a| a.get("extras").cloned())
        .collect();

    // Uniformly scale the entire document by k.
    let mut out = doc.clone();
    scale_document(&mut out, opts.scale)?;

    let rig = Rig::from_doc(&out)?;
    let helper_drivers = crate::helpers::Drivers::for_rig(&rig);
    let map = humanoid::map_rig(&rig);
    let hips = map.node_of("DEF-spine").ok_or("no hips bone found")?;
    let height = rig_height(&out, &rig)?;
    let anims = Animation::load_all(&out)?;

    let effective_pin_pelvis = opts.pin_pelvis || opts.mask == BodyMask::ArmsOnly;
    let pinned = pinned_nodes(&rig, &map, hips, opts.mask, effective_pin_pelvis);
    let upper_spine = (1..=6)
        .find_map(|i| map.node_of(&format!("DEF-spine.{:03}", i)))
        .filter(|&nd| !pinned.contains(&nd) && rig.parent[nd] == Some(hips));
    let cl_nodes = counter_load_nodes(&map);

    let mut reports = Vec::new();
    let mut new_anims: Vec<(usize, Json)> = Vec::new();

    for (ai, anim) in anims.iter().enumerate() {
        if !clips.is_empty() && !clips.contains(&anim.name) {
            continue;
        }
        let source_duration_s = anim.duration;
        let name = match suffix {
            Some(s) => format!("{}{}", anim.name, s),
            None => anim.name.clone(),
        };

        let (src_hit, is_strike) = detect_peak_time(&rig, &map, hips, height, anim);
        let warp = TimeWarp::new(source_duration_s, src_hit, opts);
        let output_duration_s = warp.out_duration;

        let (out_times, out_src_times, mut out_frames) = if output_duration_s <= 0.0 {
            let f0 = anim.sample(&rig, 0.0);
            (vec![0.0], vec![0.0], vec![f0])
        } else {
            let n_out = ((output_duration_s * DEFAULT_FPS).round() as usize).max(1) + 1;
            let times: Vec<f64> = (0..n_out)
                .map(|f| output_duration_s * f as f64 / (n_out - 1).max(1) as f64)
                .collect();
            let hit_pose = anim.sample(&rig, warp.src_hit);
            let mut src_t = Vec::with_capacity(n_out);
            let mut frames: Vec<Vec<Option<Trs>>> = Vec::with_capacity(n_out);
            for &t_out in &times {
                let (t_src, cl) = warp.eval(t_out);
                src_t.push(t_src);
                let mut pose = anim.sample(&rig, t_src);
                if is_strike && cl > 1e-9 {
                    for &nd in &cl_nodes {
                        if pinned.contains(&nd) {
                            continue;
                        }
                        let mut trs = pose[nd].unwrap_or(rig.rest[nd]);
                        let q_hit = hit_pose[nd].unwrap_or(rig.rest[nd]).r;
                        trs.r = trs.r.slerp(q_hit, -cl).normalized();
                        pose[nd] = Some(trs);
                    }
                }
                frames.push(pose);
            }
            (times, src_t, frames)
        };

        // Re-plant / preserve grounded foot plants on full-body clips.
        if opts.replant_feet && opts.mask == BodyMask::Full && out_frames.len() >= 2 {
            clear_skimming_swings(
                &rig,
                &map,
                hips,
                height,
                &orig_src_times[ai],
                &orig_src_planted[ai],
                &out_times,
                &out_src_times,
                &mut out_frames,
            );
            let clean_opts = CleanOptions {
                ground: !effective_pin_pelvis,
                lock: true,
                contact: None,
            };
            for _ in 0..2 {
                let rep = clean_clip(
                    &rig,
                    &map,
                    None,
                    height,
                    &mut out_frames,
                    &out_times,
                    &name,
                    &clean_opts,
                );
                if rep.plants_locked == 0 && rep.lift_frames == 0 {
                    break;
                }
            }
        }

        // Enforce pinned nodes; when the pelvis is pinned, carry its rotation
        // into the first upper-spine bone so upper-body attacks keep their arc.
        for f in out_frames.iter_mut() {
            let orig_sp_rot = if effective_pin_pelvis {
                upper_spine.map(|sp| rig.world(f)[sp].rotation())
            } else {
                None
            };
            for &nd in &pinned {
                f[nd] = Some(rig.rest[nd]);
            }
            if let (Some(sp), Some(target_rot)) = (upper_spine, orig_sp_rot) {
                let hips_rot = rig.world(f)[hips].rotation();
                let mut l = f[sp].unwrap_or(rig.rest[sp]);
                l.r = canonical_quat(hips_rot.conj().mul(target_rot));
                f[sp] = Some(l);
            }
            helper_drivers.drive(&rig, f);
            for &nd in &pinned {
                f[nd] = Some(rig.rest[nd]);
            }
        }

        let hit_time_s = if output_duration_s > 0.0 {
            Some(warp.hs_start)
        } else {
            None
        };
        let hit_stop = if output_duration_s > 0.0 && opts.hit_stop_s > 0.0 {
            let hs_e = warp.hs_end;
            if hs_e > warp.hs_start {
                Some((warp.hs_start, hs_e))
            } else {
                None
            }
        } else {
            None
        };
        let cancel_window = if output_duration_s > 0.0 && opts.cancel_window_s > 0.0 {
            let cw_s = hit_stop.map(|(_, e)| e).unwrap_or(warp.hs_start);
            let cw_e = (cw_s + opts.cancel_window_s).min(output_duration_s);
            if cw_e > cw_s {
                Some((cw_s, cw_e))
            } else {
                None
            }
        } else {
            None
        };

        let root_travel_after_m = horizontal_travel(&rig, &out_frames, hips);

        let mut nodes = written_nodes(&rig, &map, anim);
        for &nd in &pinned {
            if !nodes.contains(&nd) {
                nodes.push(nd);
            }
        }
        if let Some(sp) = upper_spine {
            if effective_pin_pelvis && !nodes.contains(&sp) {
                nodes.push(sp);
            }
        }
        if is_strike && opts.anticipation > 0.0 {
            for &nd in &cl_nodes {
                if !nodes.contains(&nd) {
                    nodes.push(nd);
                }
            }
        }
        nodes.sort_unstable();

        let mut anim_json = encode_clip(&mut out, &rig, &name, &out_times, &nodes, &out_frames);
        let giant_meta = giant_extras_json(
            opts,
            effective_pin_pelvis,
            source_duration_s,
            output_duration_s,
            hit_time_s,
            hit_stop,
            cancel_window,
        );
        let mut extras = match orig_extras.get(ai).and_then(|e| e.clone()) {
            Some(Json::Obj(pairs)) => Json::Obj(pairs),
            _ => Json::obj(vec![]),
        };
        extras.set("motionforge_giant", giant_meta);
        anim_json.set("extras", extras);

        new_anims.push((ai, anim_json));
        reports.push(GiantReport {
            name,
            source: anim.name.clone(),
            scale: opts.scale,
            source_duration_s,
            output_duration_s,
            root_travel_before_m: orig_travels[ai],
            root_travel_after_m,
            hit_time_s,
            hit_stop,
            cancel_window,
            mask: opts.mask,
            pin_pelvis: effective_pin_pelvis,
        });
    }

    let list = out.array_mut("animations");
    for (ai, j) in new_anims {
        match suffix {
            Some(_) => list.push(j),
            None => list[ai] = j,
        }
    }
    out.compact();
    Ok((out, reports))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_warp_is_monotone_and_flat_during_hit_stop() {
        let opts = GiantOptions::default();
        let warp = TimeWarp::new(1.0, 0.45, &opts);
        assert!((warp.out_duration - 2.0).abs() < 1e-12);
        assert!((warp.hs_end - warp.hs_start - 0.12).abs() < 1e-12);

        let (t0, cl0) = warp.eval(0.0);
        assert_eq!(t0, 0.0);
        assert_eq!(cl0, 0.0);

        let (t_end, cl_end) = warp.eval(warp.out_duration);
        assert!((t_end - 1.0).abs() < 1e-12);
        assert_eq!(cl_end, 0.0);

        let mut prev = -1.0_f64;
        for i in 0..=200 {
            let t = warp.out_duration * i as f64 / 200.0;
            let (s, _) = warp.eval(t);
            assert!(s >= prev - 1e-12, "non-monotone at i={i}: {s} < {prev}");
            prev = s;
        }

        // Inside hit-stop, source time stays pinned at src_hit and counter-load is zero.
        let (s_a, cl_a) = warp.eval(warp.hs_start + 0.01);
        let (s_b, cl_b) = warp.eval(warp.hs_end - 0.01);
        assert_eq!(s_a, warp.src_hit);
        assert_eq!(s_b, warp.src_hit);
        assert_eq!(cl_a, 0.0);
        assert_eq!(cl_b, 0.0);
    }

    #[test]
    fn plain_warp_matches_uniform_froude_slowdown() {
        let opts = GiantOptions {
            scale: 4.0,
            anticipation: 0.0,
            settle: 0.0,
            hit_stop_s: 0.0,
            cancel_window_s: 0.0,
            ..GiantOptions::default()
        };
        let warp = TimeWarp::new(1.5, 0.7, &opts);
        assert!((warp.out_duration - 3.0).abs() < 1e-12);
        for i in 0..=30 {
            let t_out = 3.0 * i as f64 / 30.0;
            let (t_src, cl) = warp.eval(t_out);
            assert!((t_src - t_out / 2.0).abs() < 1e-12);
            assert_eq!(cl, 0.0);
        }
    }

    #[test]
    fn validates_negative_params() {
        let mut opts = GiantOptions {
            anticipation: -0.1,
            ..Default::default()
        };
        assert!(opts.validate().is_err());

        opts = GiantOptions::default();
        opts.settle = f64::NAN;
        assert!(opts.validate().is_err());

        opts = GiantOptions::default();
        opts.hit_stop_s = -1.0;
        assert!(opts.validate().is_err());

        opts = GiantOptions::default();
        opts.cancel_window_s = f64::INFINITY;
        assert!(opts.validate().is_err());
    }

    #[test]
    fn giant_walk_scales_duration_travel_height_and_keeps_contacts() {
        let body = match crate::standardize::standardize(
            &crate::fixture::humanoid(crate::fixture::Naming::Plain, false, false).unwrap(),
            "humanoid",
        )
        .unwrap()
        {
            crate::standardize::Outcome::Done(d, _) => d,
            crate::standardize::Outcome::Refused(_, why) => panic!("standardize failed: {why}"),
        };
        let walk_src =
            crate::fixture::humanoid(crate::fixture::Naming::Plain, false, true).unwrap();
        let (animated, _) =
            crate::animate::retarget_clips(&body, &[("walk.glb".into(), walk_src)], 30.0).unwrap();
        let (cleaned, _) =
            crate::clean::clean_glb(&animated, &CleanOptions::default(), &[], None).unwrap();
        let src = Document::parse(&cleaned.to_bytes().unwrap()).unwrap();

        let (src_subj, src_scores) =
            motioncheck::check_doc(&src, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
        let src_score = &src_scores[0];

        let opts = GiantOptions {
            scale: 4.0,
            ..GiantOptions::default()
        };
        let (out_doc, reports) = giant_glb(&src, &opts, &[], None).unwrap();
        assert_eq!(reports.len(), 1);
        let rep = &reports[0];
        let out = Document::parse(&out_doc.to_bytes().unwrap()).unwrap();
        let (out_subj, out_scores) =
            motioncheck::check_doc(&out, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
        let out_score = &out_scores[0];

        assert!((rep.output_duration_s / rep.source_duration_s - 2.0).abs() < 0.06);
        assert!((rep.root_travel_after_m / rep.root_travel_before_m - 4.0).abs() < 0.20);
        assert!((out_subj.height / src_subj.height - 4.0).abs() < 0.20);
        assert!(out_score.feet.sink_frames <= src_score.feet.sink_frames + 2);
        assert!(out_score.feet.skate_m <= src_score.feet.skate_m + 0.03);
        let src_through = src_score.through.as_ref().map_or(0, |t| t.mesh_frames);
        let out_through = out_score.through.as_ref().map_or(0, |t| t.mesh_frames);
        assert!(out_through <= src_through + 2);
    }
}
