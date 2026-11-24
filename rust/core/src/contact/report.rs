//! Contact measurement, clip checks, and debug proxy dumps.

use super::body::{r4, Body, Capsule};
use super::slack::{hits_with_slack, SourceSlack, SRC_EXCESS_TOL};
use crate::json::Json;
use crate::math::Vec3;
use crate::rig::{Rig, Trs};

/// Per clip and arm: what the pass found and changed.
#[derive(Clone, Debug, Default)]
pub struct ArmReport {
    pub side: String,
    pub frames_before: usize,
    pub frames_after: usize,
    pub max_depth_before: f64,
    pub max_depth_after: f64,
    pub frames_changed: usize,
    pub max_shift: f64,
    /// Largest blade turn about the wrist (radians).
    pub max_turn: f64,
    /// Exact mesh check: frames with an arm vertex or blade point inside
    /// the body (beyond rest), and the deepest, before and after.
    pub mesh_frames_before: usize,
    pub mesh_frames_after: usize,
    pub mesh_depth_before: f64,
    pub mesh_depth_after: f64,
    /// Clip time of the deepest contact before the fix (for review).
    pub worst_s: f64,
    /// Proxies that were hit, e.g. "hand.R>spine.002.L".
    pub contacts: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ClipContact {
    pub weapon: bool,
    pub arms: Vec<ArmReport>,
    pub height: f64,
}

impl ClipContact {
    pub fn to_json(&self) -> Json {
        let cm = |m: f64| Json::num(((m * 1000.0).round()) / 10.0);
        Json::obj(vec![
            ("weapon", Json::Bool(self.weapon)),
            (
                "arms",
                Json::Arr(
                    self.arms
                        .iter()
                        .map(|a| {
                            Json::obj(vec![
                                ("side", Json::str(&a.side)),
                                (
                                    "frames_in_contact_before",
                                    Json::num(a.frames_before as f64),
                                ),
                                ("frames_in_contact_after", Json::num(a.frames_after as f64)),
                                ("max_depth_cm_before", cm(a.max_depth_before)),
                                ("max_depth_cm_after", cm(a.max_depth_after)),
                                (
                                    "mesh_frames_inside_before",
                                    Json::num(a.mesh_frames_before as f64),
                                ),
                                (
                                    "mesh_frames_inside_after",
                                    Json::num(a.mesh_frames_after as f64),
                                ),
                                ("mesh_depth_cm_before", cm(a.mesh_depth_before)),
                                ("mesh_depth_cm_after", cm(a.mesh_depth_after)),
                                ("worst_s", Json::num((a.worst_s * 1000.0).round() / 1000.0)),
                                ("frames_changed", Json::num(a.frames_changed as f64)),
                                ("max_wrist_shift_cm", cm(a.max_shift)),
                                (
                                    "max_blade_turn_deg",
                                    Json::num((a.max_turn.to_degrees() * 10.0).round() / 10.0),
                                ),
                                (
                                    "contacts",
                                    Json::Arr(a.contacts.iter().map(|c| Json::str(c)).collect()),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    /// Changed anything at all.
    pub fn changed(&self) -> bool {
        self.arms.iter().any(|a| a.frames_changed > 0)
    }
}

pub(super) fn measure_with_slack(
    body: &Body,
    ai: usize,
    rig: &Rig,
    frames: &[Vec<Option<Trs>>],
    weapon: bool,
    src_slack: Option<&SourceSlack>,
) -> (usize, f64, Vec<String>) {
    let mut count = 0;
    let mut worst: f64 = 0.0;
    let mut names: Vec<String> = Vec::new();
    let arm = &body.arms[ai];
    for (fi, f) in frames.iter().enumerate() {
        let world = rig.world(f);
        let hs = hits_with_slack(
            body,
            ai,
            &world,
            weapon,
            0.0,
            src_slack.map(|s| s.cap_raw[fi].as_slice()),
            src_slack.map(|s| s.cap_margin[fi].as_slice()),
            src_slack.map(|s| s.surf_raw[fi].as_slice()),
            src_slack.map(|s| s.surf_margin[fi].as_slice()),
        );
        if !hs.is_empty() {
            count += 1;
        }
        for h in hs {
            worst = worst.max(h.depth);
            let target = match body.static_parts.get(h.body) {
                Some(c) => c.name.clone(),
                None => format!(
                    "skin:{}",
                    rig.names[body.surf_bones[h.body - body.static_parts.len()]]
                        .trim_start_matches("DEF-")
                ),
            };
            let n = format!("{}>{}", arm.parts(weapon)[h.part].name, target);
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names.sort();
    (count, worst, names)
}

fn measure(
    body: &Body,
    ai: usize,
    rig: &Rig,
    frames: &[Vec<Option<Trs>>],
    weapon: bool,
) -> (usize, f64, Vec<String>) {
    measure_with_slack(body, ai, rig, frames, weapon, None)
}

/// Read-only contact check of one arm over a clip (no fix): what
/// `motion check` reports. Proxy contacts and the exact mesh check, the
/// same measures `fix_clip` reports as "before".
#[derive(Clone, Debug, Default)]
pub struct ArmCheck {
    pub side: String,
    pub proxy_frames: usize,
    pub proxy_depth: f64,
    /// Per frame: an arm vertex or blade point inside the body (beyond rest).
    pub mesh_inside: Vec<bool>,
    pub mesh_depth: f64,
    /// Frame of the deepest mesh contact.
    pub worst_frame: usize,
    pub contacts: Vec<String>,
}

/// [`ArmCheck`] for every arm; `weapon` adds the blade proxy where the
/// weapon arm has one.
pub fn check_frames(
    body: &Body,
    rig: &Rig,
    frames: &[Vec<Option<Trs>>],
    weapon: bool,
) -> Vec<ArmCheck> {
    (0..body.arms.len())
        .map(|ai| {
            let use_weapon = weapon && body.arms[ai].weapon.is_some();
            let (pf, pd, contacts) = measure(body, ai, rig, frames, use_weapon);
            let (inside, md, at) = mesh_measure(body, ai, rig, frames, use_weapon);
            ArmCheck {
                side: body.arms[ai].side.clone(),
                proxy_frames: pf,
                proxy_depth: pd,
                mesh_inside: inside,
                mesh_depth: md,
                worst_frame: at,
                contacts,
            }
        })
        .collect()
}

/// Debug: posed capsules of every frame, as JSON
/// `[[{"name", "a", "b", "r"}...] per frame]` in glTF world space.
pub fn dump_posed(body: &Body, rig: &Rig, frames: &[Vec<Option<Trs>>], weapon: bool) -> Json {
    let v = |p: Vec3| {
        Json::Arr(vec![
            Json::num(r4(p.x)),
            Json::num(r4(p.y)),
            Json::num(r4(p.z)),
        ])
    };
    Json::Arr(
        frames
            .iter()
            .map(|f| {
                let world = rig.world(f);
                let mut caps: Vec<&Capsule> = body.static_parts.iter().collect();
                for a in &body.arms {
                    caps.extend(a.parts(weapon));
                }
                Json::Arr(
                    caps.iter()
                        .map(|c| {
                            let (a, b) = c.posed(&body.inv_rest, &world);
                            Json::obj(vec![
                                ("name", Json::str(&c.name)),
                                ("a", v(a)),
                                ("b", v(b)),
                                ("r", Json::num(r4(c.r))),
                            ])
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

pub(super) type SourceFrames<'a> = (&'a Body, &'a Rig, &'a [Vec<Option<Trs>>]);
pub(super) type ArmMeshMeasure = (Vec<bool>, f64, usize, Vec<f64>);

/// Measure exact (mesh) contact over a clip for one arm on the initial frames
/// and compute per-frame source-body mesh slack (0.0 when target deepened the
/// penetration beyond the source body's own fault).
pub(super) fn mesh_measure_initial(
    body: &Body,
    ai: usize,
    rig: &Rig,
    frames: &[Vec<Option<Trs>>],
    weapon: bool,
    source: Option<SourceFrames<'_>>,
) -> ArmMeshMeasure {
    let arm = &body.arms[ai];
    let w = if weapon { arm.weapon.as_ref() } else { None };
    let mut at = 0;
    let rest_slack = body.mesh_slack.get(ai).copied().unwrap_or(0.0);
    let src_info = source.and_then(|(sb, srig, sfr)| {
        if sfr.len() != frames.len() {
            return None;
        }
        let sai = sb.arms.iter().position(|a| a.side == arm.side)?;
        let sw = if weapon {
            sb.arms[sai].weapon.as_ref()
        } else {
            None
        };
        let scale = body.height / sb.height.max(1e-6);
        Some((sb, srig, sfr, sai, sw, scale))
    });
    let mut inside = Vec::with_capacity(frames.len());
    let mut frame_slack = vec![0.0f64; frames.len()];
    let mut worst: f64 = 0.0;
    for (fi, f) in frames.iter().enumerate() {
        let world = rig.world(f);
        let (d, n) = body.mesh_depth(rig, &world, ai, w);
        let mut slack = rest_slack;
        if n > 0 && d > slack + 0.002 * body.height {
            if let Some((sb, srig, sfr, sai, sw, scale)) = src_info {
                let s_world = srig.world(&sfr[fi]);
                let (sd, sn) = sb.mesh_depth(srig, &s_world, sai, sw);
                if sn > 0 && d <= sd * scale + SRC_EXCESS_TOL {
                    let s_sl = sd * scale + 1e-4;
                    frame_slack[fi] = s_sl;
                    slack = slack.max(s_sl);
                }
            }
        }
        let deep = n > 0 && d > slack + 0.002 * body.height;
        if deep && d - slack > worst {
            worst = d - slack;
            at = inside.len();
        }
        inside.push(deep);
    }
    (inside, worst, at, frame_slack)
}

/// Exact (mesh) contact over a clip for one arm using precomputed per-frame
/// source mesh slack.
pub(super) fn mesh_measure_with_slack(
    body: &Body,
    ai: usize,
    rig: &Rig,
    frames: &[Vec<Option<Trs>>],
    weapon: bool,
    frame_slack: Option<&[f64]>,
) -> (Vec<bool>, f64, usize) {
    let arm = &body.arms[ai];
    let w = if weapon { arm.weapon.as_ref() } else { None };
    let mut at = 0;
    let rest_slack = body.mesh_slack.get(ai).copied().unwrap_or(0.0);
    let mut inside = Vec::with_capacity(frames.len());
    let mut worst: f64 = 0.0;
    for (fi, f) in frames.iter().enumerate() {
        let world = rig.world(f);
        let (d, n) = body.mesh_depth(rig, &world, ai, w);
        let extra_sl = frame_slack.and_then(|s| s.get(fi).copied()).unwrap_or(0.0);
        let slack = rest_slack.max(extra_sl);
        let deep = n > 0 && d > slack + 0.002 * body.height;
        if deep && d - slack > worst {
            worst = d - slack;
            at = inside.len();
        }
        inside.push(deep);
    }
    (inside, worst, at)
}

fn mesh_measure(
    body: &Body,
    ai: usize,
    rig: &Rig,
    frames: &[Vec<Option<Trs>>],
    weapon: bool,
) -> (Vec<bool>, f64, usize) {
    mesh_measure_with_slack(body, ai, rig, frames, weapon, None)
}
