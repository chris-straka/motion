//! Weapon hand socket insertion and clearance search.

use super::body::{hand_dir, owners};
use super::mesh::{dominant, percentile, skinned_mesh, skinned_rest, winding};
use crate::glb::{canonical_quat, Affine, Document};
use crate::humanoid;
use crate::json::Json;
use crate::math::{Quat, Vec3};
use crate::rig::{Rig, Trs};

/// Blade test mesh for [`grip_search`]: every triangle except the holding
/// hand's, posed per frame.
struct GripMesh {
    rest: Vec<Vec3>,
    inf: Vec<Vec<(usize, f64)>>,
    tris: Vec<[usize; 3]>,
}

impl GripMesh {
    fn posed(&self, world: &[Affine], inv_rest: &[Affine]) -> Vec<Vec3> {
        let mats: Vec<Affine> = (0..world.len())
            .map(|j| world[j].mul(&inv_rest[j]))
            .collect();
        self.rest
            .iter()
            .zip(&self.inf)
            .map(|(p, inf)| {
                let mut acc = Vec3::ZERO;
                let mut tot = 0.0;
                for &(j, w) in inf {
                    acc = acc + mats[j].apply(*p).scale(w);
                    tot += w;
                }
                if tot > 0.0 {
                    acc.scale(1.0 / tot)
                } else {
                    *p
                }
            })
            .collect()
    }
}

/// Find the grip (blade direction in the hand) that a set of weapon clips
/// was animated for: the animator kept the prop out of the body, the
/// other arm, its own forearm and the floor, and swords reach away from
/// the body. Each of 98 directions (a Fibonacci sphere in rest world
/// space) is scored over every frame: blade points inside the mesh
/// (holding hand excluded) or below the lowest rest vertex count as hits;
/// fewer hits win, then more clearance (mean distance of the blade to the
/// nearest vertex, capped at 15% of height). Returns (direction, hits,
/// clearance), best first.
pub fn grip_search(
    doc: &Document,
    rig: &Rig,
    hand: usize,
    origin: Vec3,
    length: f64,
    clips: &[Vec<Vec<Option<Trs>>>],
) -> Result<Vec<(Vec3, usize, f64)>, String> {
    let (verts, tris) = skinned_mesh(doc, rig)?;
    let map = humanoid::map_rig(rig);
    let owner = owners(rig, &map);
    let held = |v: usize| {
        verts[v]
            .inf
            .iter()
            .filter(|(j, _)| owner[*j] == hand)
            .map(|(_, w)| w)
            .sum::<f64>()
            > 0.5
    };
    let floor = verts.iter().map(|v| v.p.y).fold(f64::MAX, f64::min);
    let height = verts.iter().map(|v| v.p.y).fold(f64::MIN, f64::max) - floor;
    let mesh = GripMesh {
        rest: verts.iter().map(|v| v.p).collect(),
        inf: verts.iter().map(|v| v.inf.clone()).collect(),
        tris: tris
            .into_iter()
            .filter(|t| !t.iter().any(|&v| held(v)))
            .collect(),
    };
    let inv_rest: Vec<Affine> = rig
        .rest_world
        .iter()
        .map(|m| m.inverse().unwrap_or(Affine::IDENTITY))
        .collect();
    let n = 98;
    let ga = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    let dirs: Vec<Vec3> = (0..n)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let (s, c) = crate::detmath::sin_cos(ga * i as f64);
            Vec3::new(c * r, y, s * r)
        })
        .collect();
    let Some(&inv_hand) = inv_rest.get(hand) else {
        return Err(format!("hand node index {} out of range", hand));
    };
    let local: Vec<Vec<Vec3>> = dirs
        .iter()
        .map(|d| {
            (3..=12)
                .map(|k| inv_hand.apply(origin + d.scale(length * k as f64 / 12.0)))
                .collect()
        })
        .collect();
    let mut hits = vec![0usize; n];
    let mut clear = vec![0.0f64; n];
    let mut samples = 0usize;
    let cap = 0.15 * height;
    for clip in clips {
        for f in clip {
            let world = rig.world(f);
            let posed = mesh.posed(&world, &inv_rest);
            let hw = world[hand];
            for (di, pts) in local.iter().enumerate() {
                for lp in pts {
                    let p = hw.apply(*lp);
                    let near = mesh
                        .tris
                        .iter()
                        .flat_map(|t| t.iter())
                        .map(|&v| (posed[v] - p).length())
                        .fold(f64::MAX, f64::min);
                    let inside =
                        p.y < floor || (near < cap && winding(&posed, &mesh.tris, p).abs() >= 0.5);
                    if inside {
                        hits[di] += 1;
                    }
                    clear[di] += near.min(cap);
                }
            }
            samples += 10;
        }
    }
    let mut out: Vec<(Vec3, usize, f64)> = (0..n)
        .map(|i| (dirs[i], hits[i], clear[i] / samples.max(1) as f64))
        .collect();
    out.sort_by(|x, y| x.1.cmp(&y.1).then(y.2.total_cmp(&x.2)));
    Ok(out)
}

/// Add `Socket_Hand_L/R` under each hand when missing: a plain node (not a
/// joint) where a game attaches a held weapon, the convention from
/// a hand-built hero rig: 45% along the hand, local +Y (the blade)
/// pointing at the character's front in the rest pose, local +Z down.
/// Returns the sockets added.
pub fn insert_sockets(doc: &mut Document) -> Result<Vec<String>, String> {
    let rig = Rig::from_doc(doc)?;
    let map = humanoid::map_rig(&rig);
    let verts = skinned_rest(doc, &rig)?;
    let fwd = crate::animate::forward(&rig, &map);
    let up = Vec3::new(0.0, 1.0, 0.0);
    let left = up.cross(fwd).normalized();
    let mut added = Vec::new();
    for side in ["L", "R"] {
        let name = format!("Socket_Hand_{side}");
        if rig.find(&name).is_some() {
            continue;
        }
        let (Some(hand), Some(fore)) = (
            map.node_of(&format!("DEF-hand.{side}")),
            map.node_of(&format!("DEF-forearm.{side}")),
        ) else {
            continue;
        };
        let w = rig.head(hand);
        let hdir = hand_dir(&rig, &map, side).unwrap_or((w - rig.head(fore)).normalized());
        let owner = owners(&rig, &map);
        let pts: Vec<f64> = verts
            .iter()
            .filter(|v| !v.inf.is_empty() && owner[dominant(v).min(owner.len() - 1)] == hand)
            .map(|v| (v.p - w).dot(hdir))
            .collect();
        let hlen = if pts.len() >= 4 {
            percentile(pts, 0.95)
        } else {
            (w - rig.head(fore)).length() * 0.35
        };
        let pos = w + hdir.scale(0.45 * hlen.max(1e-4));
        // Blade: across the palm from the little finger's knuckle to the
        // index finger's (the axis a fist closes around) when the hand has
        // fingers; else the character's front (the hand-built hero rule).
        let knuckles = (
            map.node_of(&format!("DEF-f_index.01.{side}")),
            map.node_of(&format!("DEF-f_pinky.01.{side}")),
        );
        let blade = match knuckles {
            (Some(i), Some(p)) if (rig.head(i) - rig.head(p)).length() > 1e-6 => {
                (rig.head(i) - rig.head(p)).normalized()
            }
            _ => fwd,
        };
        // Socket frame: +Y blade, +X across (blade x hand direction), +Z = X x Y.
        let mut x = blade.cross(hdir);
        if x.length() < 1e-6 {
            x = left;
        }
        let x = x.normalized();
        let m = if blade.dot(fwd) > 0.999 {
            crate::math::Mat3::from_cols(left, fwd, up.scale(-1.0))
        } else {
            crate::math::Mat3::from_cols(x, blade, x.cross(blade))
        };
        let world_rot = Quat::from_mat3(m);
        let hw = rig.rest_world[hand];
        let inv = hw.inverse().ok_or("hand transform is singular")?;
        let local_t = inv.apply(pos);
        let local_r = canonical_quat(hw.rotation().conj().mul(world_rot));
        let mut node = Json::obj(vec![("name", Json::str(&name))]);
        let (_, _, hs) = hw.decompose();
        let inv_s = Vec3::new(1.0 / hs.x, 1.0 / hs.y, 1.0 / hs.z);
        crate::glb::set_node_trs(&mut node, local_t, local_r, inv_s);
        let nodes = doc.array_mut("nodes");
        nodes.push(node);
        let sn = nodes.len() - 1;
        if let Some(hand_node) = nodes.get_mut(hand) {
            let mut kids = hand_node
                .get("children")
                .and_then(Json::as_arr)
                .map(|k| k.to_vec())
                .unwrap_or_default();
            kids.push(Json::num(sn as f64));
            hand_node.set("children", Json::Arr(kids));
        }
        added.push(name);
    }
    Ok(added)
}

/// Socket node under the hand (`Socket_Hand_<side>`): origin and blade
/// direction (its local +Y) at rest.
pub fn socket_frame(doc: &Document, rig: &Rig, hand: usize, side: &str) -> Option<(Vec3, Vec3)> {
    let _ = doc;
    let s = rig.children[hand]
        .iter()
        .copied()
        .find(|&c| rig.names[c] == format!("Socket_Hand_{side}"))?;
    let m = rig.rest_world[s];
    Some((m.t, m.apply_linear(Vec3::new(0.0, 1.0, 0.0)).normalized()))
}
