//! Character collision proxies (torso, head, thighs, arms) and exact mesh helpers.

use super::mesh::{
    clothing, cluster_data, dominant, fast_winding, percentile, seg_seg, skinned_mesh, Clusters,
};
use super::slack::{depth, surface_hits};
use super::socket::socket_frame;
use super::Options;
use crate::glb::{Affine, Document};
use crate::humanoid::{self, Mapping};
use crate::json::Json;
use crate::math::Vec3;
use crate::rig::Rig;

/// A capsule fixed to a bone: endpoints in the bone's rest-world frame.
#[derive(Clone, Debug)]
pub struct Capsule {
    pub name: String,
    pub bone: usize,
    pub a: Vec3,
    pub b: Vec3,
    pub r: f64,
}

impl Capsule {
    pub(super) fn posed(&self, _inv_rest: &[Affine], world: &[Affine]) -> (Vec3, Vec3) {
        let m = &world[self.bone];
        (m.apply(self.a), m.apply(self.b))
    }

    pub fn to_json(&self, rig: &Rig) -> Json {
        let v = |p: Vec3| {
            Json::Arr(vec![
                Json::num(r4(p.x)),
                Json::num(r4(p.y)),
                Json::num(r4(p.z)),
            ])
        };
        Json::obj(vec![
            ("name", Json::str(&self.name)),
            ("bone", Json::str(&rig.names[self.bone])),
            ("a", v(self.a)),
            ("b", v(self.b)),
            ("r", Json::num(r4(self.r))),
        ])
    }
}

pub(super) fn r4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

pub(super) const PART_UPPER: usize = 0;
pub(super) const PART_FOREARM: usize = 1;
pub(super) const PART_PALM: usize = 2;
pub(super) const PART_WEAPON: usize = 3;
pub(super) const N_ARM_PARTS: usize = 4;

/// One arm: its chain and moving parts.
#[derive(Clone, Debug)]
pub struct Arm {
    pub side: String,
    pub upper: usize,
    pub fore: usize,
    pub hand: usize,
    pub upper_arm: Capsule,
    pub forearm: Capsule,
    pub palm: Capsule,
    pub weapon: Option<Capsule>,
}

/// The character's collision proxies.
#[derive(Clone, Debug)]
pub struct Body {
    pub height: f64,
    /// The rest pose's floor: its lowest skinned vertex height, metres.
    pub floor: f64,
    pub static_parts: Vec<Capsule>,
    pub arms: Vec<Arm>,
    /// Rest-pose depth allowed per (arm index, part index, body index).
    pub(super) slack: Vec<f64>,
    /// Rest-pose arm-vs-arm depth allowed per (arm0 part, arm1 part) in 0..3 x 0..3.
    pub(super) arm_slack: Vec<f64>,
    pub(super) inv_rest: Vec<Affine>,
    /// Body surface points (torso, head, thighs) in their bone's rest
    /// frame: (bone, position, normal). They catch shallow contacts the
    /// inner capsules miss; the capsules catch deep ones.
    pub surface: Vec<(usize, Vec3, Vec3)>,
    /// Distinct bones of `surface`, and rest slack per (arm, part, bone).
    pub(super) surf_bones: Vec<usize>,
    pub(super) surf_slack: Vec<f64>,
    /// The mesh for the exact check: rest positions, influences, body
    /// triangles (torso/head/thighs), and per arm the vertices of its
    /// forearm and hand.
    pub(super) mesh_rest: Vec<Vec3>,
    pub(super) mesh_inf: Vec<Vec<(usize, f64)>>,
    pub(super) body_tris: Vec<[usize; 3]>,
    pub(super) arm_verts: Vec<Vec<usize>>,
    /// Rest-pose mesh depth per arm (slack for the exact check).
    pub(super) mesh_slack: Vec<f64>,
    pub(super) clusters: Clusters,
}

impl Body {
    /// Fit the proxies of a humanoid. `None` when the rig lacks the arm
    /// chains or a torso.
    pub fn build(doc: &Document, rig: &Rig, opts: &Options) -> Result<Option<Body>, String> {
        let map = humanoid::map_rig(rig);
        let (verts, tris) = skinned_mesh(doc, rig)?;
        if verts.is_empty() {
            return Ok(None);
        }
        let up = Vec3::new(0.0, 1.0, 0.0);
        let fwd = crate::animate::forward(rig, &map);
        let left = up.cross(fwd).normalized();
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for v in &verts {
            lo = lo.min(v.p.y);
            hi = hi.max(v.p.y);
        }
        let height = (hi - lo).max(1e-6);
        let owner = owners(rig, &map);
        let dom: Vec<usize> = verts
            .iter()
            .map(|v| owner[dominant(v).min(owner.len() - 1)])
            .collect();
        // Clothing worn over the body (a coat cut from the base body under it) is
        // cloth: it gives way and may overlap itself, so the proxies, the surface
        // points and the exact check are fitted to the skin under it. Fitting
        // them to a long outer coat pushed wrists the whole 12 % cap in every clip
        // (arms held out wide) to clear the sleeves from the coat.
        let torso_bones: Vec<usize> = [
            "DEF-spine",
            "DEF-spine.001",
            "DEF-spine.002",
            "DEF-spine.003",
        ]
        .iter()
        .filter_map(|n| map.node_of(n))
        .collect();
        let cloth = clothing(&verts, &tris, &|v| torso_bones.contains(&dom[v]));
        let skin: Vec<bool> = verts
            .iter()
            .map(|v| !cloth.get(v.prim).copied().unwrap_or(false))
            .collect();
        let dom: Vec<usize> = dom
            .iter()
            .zip(&skin)
            .map(|(&d, &s)| if s { d } else { usize::MAX })
            .collect();
        let head_of = |n: &str| map.node_of(n).map(|i| rig.head(i));
        let to_local = |bone: usize, p: Vec3| {
            rig.rest_world[bone]
                .inverse()
                .map(|m| m.apply(p))
                .unwrap_or(p)
        };

        let mut parts: Vec<Capsule> = Vec::new();
        // Torso: spine segments up to the neck.
        let chain: Vec<(String, usize)> = [
            "DEF-spine",
            "DEF-spine.001",
            "DEF-spine.002",
            "DEF-spine.003",
            "DEF-spine.004",
            "DEF-spine.005",
            "DEF-spine.006",
        ]
        .iter()
        .filter_map(|n| map.node_of(n).map(|i| (n.to_string(), i)))
        .collect();
        let neck_idx = chain
            .iter()
            .position(|(n, _)| n == "DEF-spine.004" || n == "DEF-spine.005");
        let head_node = map.node_of("DEF-spine.006");
        let torso_end = neck_idx.unwrap_or(chain.len().saturating_sub(1));
        for k in 0..torso_end {
            let (name, bone) = (&chain[k].0, chain[k].1);
            let (a, b) = (rig.head(bone), rig.head(chain[k + 1].1));
            let mut set: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == bone)
                .map(|(v, _)| v.p)
                .collect();
            if set.len() < 8 {
                set = verts
                    .iter()
                    .map(|v| v.p)
                    .filter(|p| {
                        let u = (b - a).normalized();
                        let t = (*p - a).dot(u);
                        t >= 0.0 && t <= (b - a).length()
                    })
                    .collect();
            }
            lozenge(&mut parts, name, bone, a, b, &set, left, fwd, &to_local);
        }
        // Head: joint to crown.
        if let Some(hn) = head_node {
            let set: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == hn)
                .map(|(v, _)| v.p)
                .collect();
            if set.len() >= 8 {
                let a = rig.head(hn);
                let top = percentile(set.iter().map(|p| (*p - a).dot(up)).collect(), 0.97);
                let mid = a + up.scale(top * 0.5);
                let r = percentile(
                    set.iter()
                        .map(|p| {
                            let q = *p - mid;
                            (q - up.scale(q.dot(up))).length()
                        })
                        .collect(),
                    0.6,
                );
                let a2 = a + up.scale(r.min(top * 0.5));
                let b2 = a + up.scale((top - r).max(r.min(top * 0.5)));
                parts.push(Capsule {
                    name: "head".into(),
                    bone: hn,
                    a: to_local(hn, a2),
                    b: to_local(hn, b2),
                    r: r * 0.95,
                });
            }
        }
        // Thighs.
        for side in ["L", "R"] {
            let (Some(th), Some(sh)) = (
                map.node_of(&format!("DEF-thigh.{side}")),
                head_of(&format!("DEF-shin.{side}")),
            ) else {
                continue;
            };
            let a = rig.head(th);
            let set: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == th)
                .map(|(v, _)| v.p)
                .collect();
            let r = radial(&set, a, sh, 0.5);
            if r > 0.0 {
                parts.push(Capsule {
                    name: format!("thigh.{side}"),
                    bone: th,
                    a: to_local(th, a + (sh - a).scale(0.15)),
                    b: to_local(th, sh),
                    r: r * 0.9,
                });
            }
        }
        // Shins.
        for side in ["L", "R"] {
            let (Some(sh), Some(ft)) = (
                map.node_of(&format!("DEF-shin.{side}")),
                head_of(&format!("DEF-foot.{side}")),
            ) else {
                continue;
            };
            let a = rig.head(sh);
            let set: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == sh)
                .map(|(v, _)| v.p)
                .collect();
            let r = radial(&set, a, ft, 0.5);
            if r > 0.0 {
                parts.push(Capsule {
                    name: format!("shin.{side}"),
                    bone: sh,
                    a: to_local(sh, a + (ft - a).scale(0.1)),
                    b: to_local(sh, ft),
                    r: r * 0.9,
                });
            }
        }
        if parts.is_empty() {
            return Ok(None);
        }
        // Arms.
        let mut arms = Vec::new();
        for side in ["L", "R"] {
            let (Some(up_n), Some(fo), Some(ha)) = (
                map.node_of(&format!("DEF-upper_arm.{side}")),
                map.node_of(&format!("DEF-forearm.{side}")),
                map.node_of(&format!("DEF-hand.{side}")),
            ) else {
                continue;
            };
            let (s_pos, e, w) = (rig.head(up_n), rig.head(fo), rig.head(ha));
            let uset: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == up_n)
                .map(|(v, _)| v.p)
                .collect();
            let ru = radial(&uset, s_pos, e, 0.5).max(0.01 * height);
            let upper_arm = Capsule {
                name: format!("upper_arm.{side}"),
                bone: up_n,
                a: to_local(up_n, s_pos + (e - s_pos).scale(0.25)),
                b: to_local(up_n, e),
                r: ru * 0.85,
            };
            let fset: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == fo)
                .map(|(v, _)| v.p)
                .collect();
            let rf = radial(&fset, e, w, 0.5).max(0.01 * height);
            let hset: Vec<Vec3> = verts
                .iter()
                .zip(&dom)
                .filter(|(_, &d)| d == ha)
                .map(|(v, _)| v.p)
                .collect();
            let hdir = hand_dir(rig, &map, side).unwrap_or((w - e).normalized());
            let hlen = percentile(hset.iter().map(|p| (*p - w).dot(hdir)).collect(), 0.95)
                .max(0.04 * height);
            let tip = w + hdir.scale(hlen);
            let rh = radial(&hset, w, tip, 0.5).max(0.01 * height);
            let forearm = Capsule {
                name: format!("forearm.{side}"),
                bone: fo,
                a: to_local(fo, e + (w - e).scale(0.2)),
                b: to_local(fo, w),
                r: rf * 0.85,
            };
            let palm = Capsule {
                name: format!("hand.{side}"),
                bone: ha,
                a: to_local(ha, w + hdir.scale(hlen * 0.2)),
                b: to_local(ha, w + hdir.scale((hlen - rh).max(hlen * 0.3))),
                r: rh * 0.85,
            };
            let weapon = if opts.weapon_side.as_deref() == Some(side) {
                let (origin, dir) = socket_frame(doc, rig, ha, side)
                    .unwrap_or_else(|| (w + hdir.scale(hlen * 0.45), fwd));
                let len = opts.weapon_length * height;
                Some(Capsule {
                    name: format!("weapon.{side}"),
                    bone: ha,
                    a: to_local(ha, origin + dir.scale(0.12 * len)),
                    b: to_local(ha, origin + dir.scale(len)),
                    r: 0.012 * height,
                })
            } else {
                None
            };
            arms.push(Arm {
                side: side.to_string(),
                upper: up_n,
                fore: fo,
                hand: ha,
                upper_arm,
                forearm,
                palm,
                weapon,
            });
        }
        if arms.is_empty() {
            return Ok(None);
        }
        let inv_rest: Vec<Affine> = rig
            .rest_world
            .iter()
            .map(|m| m.inverse().unwrap_or(Affine::IDENTITY))
            .collect();
        // Surface points: torso, head and thigh vertices that no arm bone
        // moves, kept in their dominant bone's rest frame.
        let mut body_bones: Vec<usize> = parts.iter().map(|c| c.bone).collect();
        body_bones.sort_unstable();
        body_bones.dedup();
        let arm_bone = |j: usize| {
            let o = owner[j.min(owner.len() - 1)];
            arms.iter()
                .any(|a| o == a.upper || o == a.fore || o == a.hand)
                || map
                    .canonical_of(o)
                    .is_some_and(|n| n.starts_with("DEF-shoulder"))
        };
        let mut surface = Vec::new();
        for (v, &d) in verts.iter().zip(&dom) {
            if v.inf.is_empty() || v.n.length() < 0.5 || !body_bones.contains(&d) {
                continue;
            }
            let wd: f64 = v
                .inf
                .iter()
                .filter(|(j, _)| owner[*j] == d)
                .map(|(_, w)| w)
                .sum();
            let wa: f64 = v
                .inf
                .iter()
                .filter(|(j, _)| arm_bone(*j))
                .map(|(_, w)| w)
                .sum();
            if wd < 0.6 || wa > 0.1 {
                continue;
            }
            let inv = rig.rest_world[d].inverse().unwrap_or(Affine::IDENTITY);
            surface.push((d, inv.apply(v.p), inv.apply_linear(v.n)));
        }
        let surf_bones = body_bones.clone();
        // Exact-check mesh: body triangles (all three corners owned by a
        // body bone and not moved by an arm), arm vertices per side.
        let is_body = |v: usize| {
            let vv = &verts[v];
            !vv.inf.is_empty()
                && body_bones.contains(&dom[v])
                && vv
                    .inf
                    .iter()
                    .filter(|(j, _)| arm_bone(*j))
                    .map(|(_, w)| w)
                    .sum::<f64>()
                    < 0.3
        };
        let body_tris: Vec<[usize; 3]> = tris
            .iter()
            .copied()
            .filter(|t| t.iter().all(|&v| is_body(v)))
            .collect();
        let body_tris_c = body_tris.clone();
        let arm_verts: Vec<Vec<usize>> = arms
            .iter()
            .map(|a| {
                (0..verts.len())
                    .filter(|&v| dom[v] == a.fore || dom[v] == a.hand)
                    .collect()
            })
            .collect();
        let mut body = Body {
            height,
            floor: lo,
            static_parts: parts,
            arms,
            slack: Vec::new(),
            arm_slack: Vec::new(),
            inv_rest,
            surface,
            surf_bones,
            surf_slack: Vec::new(),
            mesh_rest: verts.iter().map(|v| v.p).collect(),
            mesh_inf: verts.iter().map(|v| v.inf.clone()).collect(),
            body_tris,
            arm_verts,
            mesh_slack: Vec::new(),
            clusters: Clusters::new(
                &verts.iter().map(|v| v.p).collect::<Vec<_>>(),
                &body_tris_c,
                height / 10.0,
            ),
        };
        // Rest-pose slack.
        let world = rig.world(&vec![None; rig.names.len()]);
        let nb = body.static_parts.len();
        let mut slack = vec![0.0; body.arms.len() * N_ARM_PARTS * nb];
        for (ai, arm) in body.arms.iter().enumerate() {
            for (pi, part) in arm.parts(true).iter().enumerate() {
                for (bi, s) in body.static_parts.iter().enumerate() {
                    let d = depth(part, s, &body.inv_rest, &world, 0.0).0;
                    slack[(ai * N_ARM_PARTS + pi) * nb + bi] = d.max(0.0);
                }
            }
        }
        body.slack = slack;
        if body.arms.len() >= 2 {
            let mut arm_slack = vec![0.0; 9];
            let p0s = body.arms[0].body_parts();
            let p1s = body.arms[1].body_parts();
            for (i0, c0) in p0s.iter().enumerate() {
                for (i1, c1) in p1s.iter().enumerate() {
                    let d = depth(c0, c1, &body.inv_rest, &world, 0.0).0;
                    arm_slack[i0 * 3 + i1] = d.max(0.0);
                }
            }
            body.arm_slack = arm_slack;
        }
        let nsb = body.surf_bones.len();
        let mut ss: Vec<f64> = vec![0.0; body.arms.len() * N_ARM_PARTS * nsb];
        let posed = body.posed_surface(&world);
        for (ai, arm) in body.arms.iter().enumerate() {
            for (pi, part) in arm.parts(true).iter().enumerate() {
                for (k, d, _) in surface_hits(part, &posed, &body.inv_rest, &world, 0.0) {
                    if let Some(pos) = body.surf_bones.iter().position(|&b| b == body.surface[k].0)
                    {
                        let slot = (ai * N_ARM_PARTS + pi) * nsb + pos;
                        ss[slot] = ss[slot].max(d);
                    }
                }
            }
        }
        body.surf_slack = ss;
        body.mesh_slack = (0..body.arms.len())
            .map(|ai| body.mesh_depth(rig, &world, ai, None).0)
            .collect();
        Ok(Some(body))
    }

    pub fn to_json(&self, rig: &Rig) -> Json {
        let mut all: Vec<Json> = self.static_parts.iter().map(|c| c.to_json(rig)).collect();
        for arm in &self.arms {
            for p in arm.parts(true) {
                all.push(p.to_json(rig));
            }
        }
        Json::obj(vec![
            ("height", Json::num(r4(self.height))),
            ("capsules", Json::Arr(all)),
            ("surface_points", Json::num(self.surface.len() as f64)),
            ("body_triangles", Json::num(self.body_tris.len() as f64)),
            (
                "arm_vertices",
                Json::Arr(
                    self.arm_verts
                        .iter()
                        .map(|v| Json::num(v.len() as f64))
                        .collect(),
                ),
            ),
        ])
    }
}

impl Body {
    pub(super) fn posed_surface(&self, world: &[Affine]) -> Vec<(Vec3, Vec3)> {
        self.surface
            .iter()
            .map(|(b, p, n)| (world[*b].apply(*p), world[*b].apply_linear(*n).normalized()))
            .collect()
    }
}

impl Body {
    /// Height of the posed mesh's lowest vertex (the whole character:
    /// a body lying or kneeling touches the floor with more than its
    /// feet), metres.
    pub fn lowest(&self, world: &[Affine]) -> f64 {
        let mats: Vec<Affine> = (0..world.len())
            .map(|j| world[j].mul(&self.inv_rest[j]))
            .collect();
        let mut lo = f64::INFINITY;
        for (v, inf) in self.mesh_inf.iter().enumerate() {
            let mut y = 0.0;
            let mut tot = 0.0;
            for &(j, w) in inf {
                if let Some(m) = mats.get(j) {
                    y += m.apply(self.mesh_rest[v]).y * w;
                    tot += w;
                }
            }
            let y = if tot > 0.0 {
                y / tot
            } else {
                self.mesh_rest[v].y
            };
            lo = lo.min(y);
        }
        lo
    }

    /// Exact check for one arm in one pose: arm vertices (forearm, hand)
    /// and blade sample points inside the body mesh (generalized winding
    /// number over the body triangles, so the open cuts at the neck,
    /// shoulders and hips do not matter). Returns (deepest inside point's
    /// distance to the nearest body vertex, count of inside points).
    pub fn mesh_depth(
        &self,
        rig: &Rig,
        world: &[Affine],
        ai: usize,
        weapon: Option<&Capsule>,
    ) -> (f64, usize) {
        let mats: Vec<Affine> = (0..world.len())
            .map(|j| world[j].mul(&self.inv_rest[j]))
            .collect();
        let skin = |v: usize| {
            let mut acc = Vec3::ZERO;
            let mut tot = 0.0;
            for &(j, w) in &self.mesh_inf[v] {
                if let Some(m) = mats.get(j) {
                    acc = acc + m.apply(self.mesh_rest[v]).scale(w);
                    tot += w;
                }
            }
            if tot > 0.0 {
                acc.scale(1.0 / tot)
            } else {
                self.mesh_rest[v]
            }
        };
        let _ = rig;
        // Posed body triangles and their bounds.
        let mut used = vec![false; self.mesh_rest.len()];
        for t in &self.body_tris {
            for &v in t {
                used[v] = true;
            }
        }
        let mut posed = vec![Vec3::ZERO; self.mesh_rest.len()];
        let (mut lo, mut hi) = (
            Vec3::new(f64::MAX, f64::MAX, f64::MAX),
            Vec3::new(f64::MIN, f64::MIN, f64::MIN),
        );
        for v in 0..posed.len() {
            if used[v] {
                let p = skin(v);
                posed[v] = p;
                lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
                hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
            }
        }
        let Some(arm_vs) = self.arm_verts.get(ai) else {
            return (0.0, 0);
        };
        let mut pts: Vec<Vec3> = arm_vs.iter().map(|&v| skin(v)).collect();
        if let Some(w) = weapon {
            let (a, b) = w.posed(&self.inv_rest, world);
            for k in 0..=12 {
                pts.push(a + (b - a).scale(k as f64 / 12.0));
            }
        }
        let mut worst: f64 = 0.0;
        let mut count = 0;
        // Cheap reject first: a point far outside every body proxy (radius
        // x1.6 plus 4% of height, generous for bellies and chests) is not
        // inside the body; the winding number decides the rest.
        let caps: Vec<(Vec3, Vec3, f64)> = self
            .static_parts
            .iter()
            .map(|c| {
                let (a, b) = c.posed(&self.inv_rest, world);
                (a, b, c.r * 1.6 + 0.04 * self.height)
            })
            .collect();
        let mut cdata = None;
        for p in pts {
            if p.x < lo.x || p.y < lo.y || p.z < lo.z || p.x > hi.x || p.y > hi.y || p.z > hi.z {
                continue;
            }
            if !caps.iter().any(|(a, b, r)| seg_seg(*a, *b, p, p).2 < *r) {
                continue;
            }
            let cd =
                cdata.get_or_insert_with(|| cluster_data(&posed, &self.body_tris, &self.clusters));
            if fast_winding(&posed, &self.body_tris, cd, p).abs() < 0.5 {
                continue;
            }
            count += 1;
            let d = self
                .body_tris
                .iter()
                .flat_map(|t| t.iter())
                .map(|&v| (posed[v] - p).length())
                .fold(f64::MAX, f64::min);
            worst = worst.max(d);
        }
        (worst, count)
    }
}

impl Arm {
    pub(super) fn parts(&self, weapon: bool) -> Vec<&Capsule> {
        let mut v = vec![&self.upper_arm, &self.forearm, &self.palm];
        if weapon {
            if let Some(w) = &self.weapon {
                v.push(w);
            }
        }
        v
    }

    pub(super) fn body_parts(&self) -> [&Capsule; 3] {
        [&self.upper_arm, &self.forearm, &self.palm]
    }
}

/// Each joint's proxy owner: twist/helper bones and the pelvis/breast
/// count as their parents' region.
pub(super) fn owners(rig: &Rig, map: &Mapping) -> Vec<usize> {
    (0..rig.names.len())
        .map(|i| {
            let name = map.canonical_of(i).unwrap_or("");
            if let Some(owner) = crate::helpers::owner_of(name) {
                return map.node_of(&owner).unwrap_or(i);
            }
            if name.starts_with("DEF-pelvis") {
                return map.node_of("DEF-spine").unwrap_or(i);
            }
            if name.starts_with("DEF-breast") {
                return map
                    .node_of("DEF-spine.003")
                    .or_else(|| map.node_of("DEF-spine.002"))
                    .unwrap_or(i);
            }
            if name.starts_with("DEF-palm")
                || name.starts_with("DEF-f_")
                || name.starts_with("DEF-thumb")
            {
                let side = if name.ends_with(".L") { "L" } else { "R" };
                return map.node_of(&format!("DEF-hand.{side}")).unwrap_or(i);
            }
            // Unmapped joints (finger stubs) ride the nearest mapped ancestor.
            if map.canonical_of(i).is_none() {
                let mut cur = rig.parent[i];
                while let Some(p) = cur {
                    if map.canonical_of(p).is_some() {
                        return p;
                    }
                    cur = rig.parent[p];
                }
            }
            i
        })
        .collect()
}

/// Median-ish distance of `set` to segment a-b (quantile `q`).
fn radial(set: &[Vec3], a: Vec3, b: Vec3, q: f64) -> f64 {
    let ab = b - a;
    let l2 = ab.dot(ab).max(1e-12);
    percentile(
        set.iter()
            .map(|p| {
                let t = ((*p - a).dot(ab) / l2).clamp(0.0, 1.0);
                (*p - (a + ab.scale(t))).length()
            })
            .collect(),
        q,
    )
}

#[allow(clippy::too_many_arguments)]
fn lozenge(
    parts: &mut Vec<Capsule>,
    name: &str,
    bone: usize,
    a: Vec3,
    b: Vec3,
    set: &[Vec3],
    left: Vec3,
    fwd: Vec3,
    to_local: &dyn Fn(usize, Vec3) -> Vec3,
) {
    let u = (b - a).normalized();
    let len = (b - a).length();
    let lp = (left - u.scale(left.dot(u))).normalized();
    let fp = (fwd - u.scale(fwd.dot(u))).normalized();
    let pts: Vec<Vec3> = set
        .iter()
        .copied()
        .filter(|p| {
            let t = (*p - a).dot(u);
            t >= -0.1 * len && t <= 1.1 * len
        })
        .collect();
    if pts.len() < 8 {
        return;
    }
    let xs: Vec<f64> = pts.iter().map(|p| (*p - a).dot(lp)).collect();
    let ys: Vec<f64> = pts.iter().map(|p| (*p - a).dot(fp)).collect();
    let (x0, x1) = (percentile(xs.clone(), 0.08), percentile(xs, 0.92));
    let (y0, y1) = (percentile(ys.clone(), 0.08), percentile(ys, 0.92));
    let c = lp.scale((x0 + x1) * 0.5) + fp.scale((y0 + y1) * 0.5);
    let (hw, hd) = ((x1 - x0) * 0.5, (y1 - y0) * 0.5);
    let r = hd * 0.95;
    let off = (hw * 0.95 - r).max(0.0);
    let sides: Vec<(f64, &str)> = if off > 0.15 * r {
        vec![(-1.0, "R"), (1.0, "L")]
    } else {
        vec![(0.0, "")]
    };
    for (sgn, tag) in sides {
        let shift = c + lp.scale(sgn * off);
        let nm = if tag.is_empty() {
            name.trim_start_matches("DEF-").to_string()
        } else {
            format!("{}.{}", name.trim_start_matches("DEF-"), tag)
        };
        parts.push(Capsule {
            name: nm,
            bone,
            a: to_local(bone, a + shift),
            b: to_local(bone, b + shift),
            r,
        });
    }
}

pub(super) fn hand_dir(rig: &Rig, map: &Mapping, side: &str) -> Option<Vec3> {
    let h = rig.head(map.node_of(&format!("DEF-hand.{side}"))?);
    for c in [
        "DEF-f_middle.01",
        "DEF-f_index.01",
        "DEF-f_ring.01",
        "DEF-palm.02",
    ] {
        if let Some(n) = map.node_of(&format!("{c}.{side}")) {
            let d = rig.head(n) - h;
            if d.length() > 1e-6 {
                return Some(d.normalized());
            }
        }
    }
    // Unmapped hand children (finger stubs): their mean direction.
    let hn = map.node_of(&format!("DEF-hand.{side}"))?;
    let mut sum = Vec3::ZERO;
    for &c in &rig.children[hn] {
        let d = rig.head(c) - h;
        if d.length() > 1e-6 && rig.names[c] != format!("Socket_Hand_{side}") {
            sum = sum + d.normalized();
        }
    }
    (sum.length() > 1e-6).then(|| sum.normalized())
}
