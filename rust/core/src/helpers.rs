//! Twist/helper bones of the standard humanoid skeleton.
//!
//! Linear blend skinning collapses and stretches the armpit and groin
//! when an upper arm or thigh swings 90 degrees or more: a vertex shared
//! by the chest and the upper arm is averaged between two transforms
//! 90 degrees apart. A helper bone sits on the limb's joint, under the
//! limb's parent, and turns by a share (`SHARE`, half) of the limb's
//! rotation, swing and twist alike. Shared vertices then blend over two
//! 45 degree steps instead of one 90 degree step.
//!
//! The pipeline bakes the helper's rotation into every clip (`animate`)
//! and pose (`posetest`), so a game plays them as ordinary keyed joints
//! with no runtime constraint code. Each helper node carries
//! `extras.helper_bone = {"driver": <limb>, "share": 0.5}` so tools that
//! pose the rig (weightforge) can drive it the same way.

use crate::glb::{canonical_quat, node_trs, set_node_trs, Document};
use crate::humanoid;
use crate::json::Json;
use crate::keypose::fit::twist_angle;
use crate::math::Quat;
use crate::rig::{Rig, Trs};

/// Fraction of the driver's rotation the helper takes.
pub const SHARE: f64 = 0.5;

/// `(helper, driver)` for auto-inserted proximal limb helpers; the helper's
/// parent is the driver's parent.
pub fn table() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for side in ["L", "R"] {
        // Canonical table order (humanoid.rs): thigh before upper arm.
        out.push((
            format!("DEF-thigh_twist.{side}"),
            format!("DEF-thigh.{side}"),
        ));
        out.push((
            format!("DEF-upper_arm_twist.{side}"),
            format!("DEF-upper_arm.{side}"),
        ));
    }
    out
}

/// `(helper, owner)` for distal forearm twist helpers; the helper hangs
/// under `DEF-forearm.{L,R}` alongside `DEF-hand.{L,R}`.
pub fn forearm_table() -> Vec<(String, String)> {
    ["L", "R"]
        .into_iter()
        .map(|side| {
            (
                format!("DEF-forearm_twist.{side}"),
                format!("DEF-forearm.{side}"),
            )
        })
        .collect()
}

pub fn is_helper(name: &str) -> bool {
    owner_of(name).is_some()
}

/// Driver bone of an auto-inserted proximal helper (`table()`).
pub fn driver_of(name: &str) -> Option<String> {
    table().into_iter().find(|(h, _)| h == name).map(|(_, d)| d)
}

/// Owning limb bone for any helper bone (proximal limb twist or distal
/// forearm twist).
pub fn owner_of(name: &str) -> Option<String> {
    driver_of(name).or_else(|| {
        forearm_table()
            .into_iter()
            .find(|(h, _)| h == name)
            .map(|(_, o)| o)
    })
}

fn find_or_mapped(rig: &Rig, map: &Option<humanoid::Mapping>, name: &str) -> Option<usize> {
    rig.find(name)
        .or_else(|| map.as_ref().and_then(|m| m.node_of(name)))
}

/// Resolved helper-bone bindings for a single rig, built once so callers
/// driving many frames reuse the rig mapping across both proximal limb
/// helpers and distal forearm twists.
#[derive(Clone, Debug, Default)]
pub struct Drivers {
    proximal: Vec<(usize, usize)>,
    forearm: Vec<(usize, usize, usize)>,
}

impl Drivers {
    /// Resolve every proximal limb helper and distal forearm twist helper on
    /// `rig`. Unstandardized rigs are mapped once and shared across both.
    pub fn for_rig(rig: &Rig) -> Self {
        let map = rig
            .find("DEF-spine")
            .is_none()
            .then(|| humanoid::map_rig(rig));
        let proximal = table()
            .iter()
            .filter_map(|(h, d)| {
                let (hn, dn) = (find_or_mapped(rig, &map, h)?, find_or_mapped(rig, &map, d)?);
                (rig.parent[hn] == rig.parent[dn]).then_some((hn, dn))
            })
            .collect();
        let mut forearm = Vec::new();
        for side in ["L", "R"] {
            let (Some(hn), Some(fo), Some(ha)) = (
                find_or_mapped(rig, &map, &format!("DEF-forearm_twist.{side}")),
                find_or_mapped(rig, &map, &format!("DEF-forearm.{side}")),
                find_or_mapped(rig, &map, &format!("DEF-hand.{side}")),
            ) else {
                continue;
            };
            if rig.parent[hn] == Some(fo) && rig.parent[ha] == Some(fo) {
                forearm.push((hn, fo, ha));
            }
        }
        Self { proximal, forearm }
    }

    pub fn proximal(&self) -> &[(usize, usize)] {
        &self.proximal
    }

    pub fn forearm_twists(&self) -> &[(usize, usize, usize)] {
        &self.forearm
    }

    pub fn driven_nodes(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self.proximal.iter().map(|&(h, _)| h).collect();
        for &(h, _, _) in &self.forearm {
            if !out.contains(&h) {
                out.push(h);
            }
        }
        out
    }

    /// Set every present helper's local from its driver (posed or rest).
    pub fn drive(&self, rig: &Rig, locals: &mut [Option<Trs>]) {
        for &(h, d) in &self.proximal {
            let dl = locals[d].unwrap_or(rig.rest[d]);
            locals[h] = Some(helper_local(rig, h, d, dl));
        }
        for &(h, fo, ha) in &self.forearm {
            let hl = locals[ha].unwrap_or(rig.rest[ha]);
            locals[h] = Some(forearm_twist_local(rig, h, fo, ha, hl));
        }
    }

    /// Fill any unkeyed helper nodes from their drivers (for pose-sheet sampling).
    pub fn drive_missing(&self, rig: &Rig, locals: &mut [Option<Trs>]) {
        for &(h, d) in &self.proximal {
            if locals[h].is_none() {
                let dl = locals[d].unwrap_or(rig.rest[d]);
                locals[h] = Some(helper_local(rig, h, d, dl));
            }
        }
        for &(h, fo, ha) in &self.forearm {
            if locals[h].is_none() {
                let hl = locals[ha].unwrap_or(rig.rest[ha]);
                locals[h] = Some(forearm_twist_local(rig, h, fo, ha, hl));
            }
        }
    }
}

/// `(helper node, driver node)` for every proximal helper the rig has, when
/// the helper hangs from the driver's parent (the only layout the rule
/// knows). Also matches unstandardized twist names (`UpperArmTwist_R`, etc.).
pub fn present(rig: &Rig) -> Vec<(usize, usize)> {
    Drivers::for_rig(rig).proximal
}

/// `(forearm_twist node, forearm node, hand node)` for every distal forearm
/// twist helper hanging under `forearm` alongside `hand`.
pub fn forearm_twists(rig: &Rig) -> Vec<(usize, usize, usize)> {
    Drivers::for_rig(rig).forearm
}

/// Every helper node `drive` updates on `rig`.
pub fn driven_nodes(rig: &Rig) -> Vec<usize> {
    Drivers::for_rig(rig).driven_nodes()
}

/// The helper's local transform for a driver local transform: the rest
/// pose turned by `SHARE` of the driver's change from its own rest.
pub fn helper_local(rig: &Rig, helper: usize, driver: usize, driver_local: Trs) -> Trs {
    let delta = driver_local.r.mul(rig.rest[driver].r.conj());
    let part = Quat::IDENTITY.slerp(delta.normalized(), SHARE);
    let mut out = rig.rest[helper];
    out.r = part.mul(rig.rest[helper].r).normalized();
    // A translated driver (rare for limbs) moves its joint: follow it.
    out.t = rig.rest[helper].t + (driver_local.t - rig.rest[driver].t);
    out
}

/// Distal forearm twist helper local transform: turns about the forearm->hand
/// axis by `SHARE` of `hand`'s twist change from rest relative to `forearm`.
pub fn forearm_twist_local(
    rig: &Rig,
    helper: usize,
    forearm: usize,
    hand: usize,
    hand_local: Trs,
) -> Trs {
    let mut out = rig.rest[helper];
    let along = rig.rest_world[forearm]
        .rotation()
        .conj()
        .rotate_vec(rig.head(hand) - rig.head(forearm));
    if along.length() < 1e-9 {
        return out;
    }
    let axis_f = along.normalized();
    let delta = hand_local.r.mul(rig.rest[hand].r.conj());
    let tw = twist_angle(delta.normalized(), axis_f);
    out.r = canonical_quat(Quat::from_axis_angle(axis_f, SHARE * tw).mul(rig.rest[helper].r));
    out
}

/// Set every present helper's local from its driver (posed or rest).
pub fn drive(rig: &Rig, locals: &mut [Option<Trs>]) {
    Drivers::for_rig(rig).drive(rig, locals);
}

/// Fill any unkeyed helper nodes from their drivers (for pose-sheet sampling).
pub fn drive_missing(rig: &Rig, locals: &mut [Option<Trs>]) {
    Drivers::for_rig(rig).drive_missing(rig, locals);
}

/// What `insert` did.
#[derive(Clone, Debug, Default)]
pub struct Inserted {
    pub bones: Vec<String>,
}

/// Add the missing helpers of a standardized humanoid: a node at its
/// driver's joint with the driver's rest transform and a skin joint (last
/// in the list, so existing JOINTS indices stay valid) with the driver's
/// inverse bind matrix. They start with no weight: weightforge's fix
/// weights them (its helper band) when the deformation check needs it,
/// and SkinTokens skins around them. No-op for helpers already present.
pub fn insert(doc: &mut Document) -> Result<Inserted, String> {
    let mut done = Inserted::default();
    let rig = Rig::from_doc(doc)?;
    let mut added: Vec<(usize, usize)> = Vec::new(); // (helper node, driver node)
    for (helper, driver) in table() {
        if rig.find(&helper).is_some() {
            continue;
        }
        let Some(dn) = rig.find(&driver) else {
            continue;
        };
        let Some(parent) = rig.parent[dn] else {
            continue;
        };
        if !rig.joints().contains(&dn) {
            continue;
        }
        let (t, r, s) = node_trs(&doc.array("nodes")[dn]);
        let mut node = Json::obj(vec![
            ("name", Json::str(&helper)),
            (
                "extras",
                Json::obj(vec![(
                    "helper_bone",
                    Json::obj(vec![
                        ("driver", Json::str(&driver)),
                        ("share", Json::num(SHARE)),
                    ]),
                )]),
            ),
        ]);
        set_node_trs(&mut node, t, r, s);
        let nodes = doc.array_mut("nodes");
        nodes.push(node);
        let hn = nodes.len() - 1;
        let mut kids = nodes[parent]
            .get("children")
            .and_then(Json::as_arr)
            .map(|k| k.to_vec())
            .unwrap_or_default();
        kids.push(Json::num(hn as f64));
        nodes[parent].set("children", Json::Arr(kids));
        added.push((hn, dn));
        done.bones.push(helper);
    }
    if added.is_empty() {
        return Ok(done);
    }
    let rig = Rig::from_doc(doc)?;

    // Skins: append each helper with its driver's inverse bind matrix.
    let skin_count = doc.array("skins").len();
    for si in 0..skin_count {
        let mut joints = rig.skins[si].clone();
        let mut rows = match doc.array("skins")[si]
            .get("inverseBindMatrices")
            .and_then(Json::as_usize)
        {
            Some(a) => Some(doc.read_accessor(a)?),
            None => None,
        };
        for &(hn, dn) in &added {
            let Some(ds) = joints.iter().position(|&j| j == dn) else {
                continue;
            };
            joints.push(hn);
            if let Some(rows) = rows.as_mut() {
                let row = rows
                    .get(ds)
                    .cloned()
                    .ok_or("inverseBindMatrices shorter than joints")?;
                rows.push(row);
            }
        }
        let skins = doc.array_mut("skins");
        skins[si].set(
            "joints",
            Json::Arr(joints.iter().map(|&j| Json::num(j as f64)).collect()),
        );
        if let Some(rows) = rows {
            let acc = doc.push_float_accessor(&rows, "MAT4", false);
            doc.array_mut("skins")[si].set("inverseBindMatrices", Json::num(acc as f64));
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glb::Affine;
    use crate::math::Vec3;

    #[test]
    fn table_names() {
        assert!(is_helper("DEF-upper_arm_twist.L"));
        assert_eq!(
            driver_of("DEF-thigh_twist.R").as_deref(),
            Some("DEF-thigh.R")
        );
        assert!(!is_helper("DEF-upper_arm.L"));
    }

    #[test]
    fn owner_of_covers_proximal_and_forearm_twists() {
        for (helper, owner) in [
            ("DEF-thigh_twist.L", "DEF-thigh.L"),
            ("DEF-thigh_twist.R", "DEF-thigh.R"),
            ("DEF-upper_arm_twist.L", "DEF-upper_arm.L"),
            ("DEF-upper_arm_twist.R", "DEF-upper_arm.R"),
            ("DEF-forearm_twist.L", "DEF-forearm.L"),
            ("DEF-forearm_twist.R", "DEF-forearm.R"),
        ] {
            assert!(is_helper(helper), "{helper} should be a helper");
            assert_eq!(owner_of(helper).as_deref(), Some(owner));
        }
        assert_eq!(owner_of("DEF-forearm.L"), None);
        assert_eq!(owner_of("DEF-upper_arm.R"), None);
        assert_eq!(driver_of("DEF-forearm_twist.L"), None);
    }

    #[test]
    fn cached_drivers_match_free_functions_on_unstandardized_rig() {
        let specs: [(&str, Option<usize>, Vec3); 7] = [
            ("Hips", None, Vec3::new(0.0, 0.95, 0.0)),
            ("Shoulder_L", Some(0), Vec3::new(0.04, 1.45, 0.0)),
            ("UpperArm_L", Some(1), Vec3::new(0.17, 1.45, 0.0)),
            ("UpperArmTwist_L", Some(1), Vec3::new(0.17, 1.45, 0.0)),
            ("ForeArm_L", Some(2), Vec3::new(0.43, 1.45, 0.0)),
            ("ForeArmTwist_L", Some(4), Vec3::new(0.55, 1.45, 0.0)),
            ("Hand_L", Some(4), Vec3::new(0.68, 1.45, 0.0)),
        ];
        let n = specs.len();
        let mut children = vec![Vec::new(); n];
        for (i, &(_, p, _)) in specs.iter().enumerate() {
            if let Some(p) = p {
                children[p].push(i);
            }
        }
        let rest: Vec<Trs> = specs
            .iter()
            .map(|&(_, p, pos)| {
                let local_t = match p {
                    Some(pi) => pos - specs[pi].2,
                    None => pos,
                };
                Trs {
                    t: local_t,
                    r: Quat::IDENTITY,
                    s: Vec3::new(1.0, 1.0, 1.0),
                }
            })
            .collect();
        let rest_local: Vec<Affine> = rest
            .iter()
            .map(|t| Affine::from_trs(t.t, t.r, t.s))
            .collect();
        let rest_world: Vec<Affine> = specs
            .iter()
            .map(|&(_, _, pos)| Affine::from_trs(pos, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0)))
            .collect();
        let rig = Rig {
            names: specs.iter().map(|&(name, _, _)| name.to_string()).collect(),
            parent: specs.iter().map(|&(_, p, _)| p).collect(),
            children,
            rest,
            rest_local,
            rest_world,
            skins: vec![(0..n).collect()],
            order: (0..n).collect(),
        };
        assert!(rig.find("DEF-spine").is_none());

        let drivers = Drivers::for_rig(&rig);
        assert_eq!(drivers.proximal(), present(&rig).as_slice());
        assert_eq!(drivers.proximal(), &[(3, 2)]);
        assert_eq!(drivers.forearm_twists(), forearm_twists(&rig).as_slice());
        assert_eq!(drivers.forearm_twists(), &[(5, 4, 6)]);
        assert_eq!(drivers.driven_nodes(), driven_nodes(&rig));

        let mut posed = vec![None; n];
        let mut ua = rig.rest[2];
        ua.r = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 0.8);
        posed[2] = Some(ua);
        let mut hand = rig.rest[6];
        hand.r = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 1.0);
        posed[6] = Some(hand);

        let mut via_cached = posed.clone();
        let mut via_free = posed.clone();
        drivers.drive(&rig, &mut via_cached);
        drive(&rig, &mut via_free);
        for i in [3, 5] {
            let a = via_cached[i].unwrap();
            let b = via_free[i].unwrap();
            assert_eq!(a.t, b.t);
            assert_eq!(a.r, b.r);
            assert_eq!(a.s, b.s);
        }

        // drive_missing preserves an already-keyed helper and fills missing ones.
        let mut miss_cached = posed.clone();
        let custom = Trs {
            t: rig.rest[3].t,
            r: Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.25),
            s: rig.rest[3].s,
        };
        miss_cached[3] = Some(custom);
        let mut miss_free = miss_cached.clone();
        drivers.drive_missing(&rig, &mut miss_cached);
        drive_missing(&rig, &mut miss_free);
        assert_eq!(miss_cached[3].unwrap().r, custom.r);
        assert_eq!(miss_cached[3].unwrap().r, miss_free[3].unwrap().r);
        assert_eq!(miss_cached[5].unwrap().r, miss_free[5].unwrap().r);
    }

    #[test]
    fn nan_safe_sorting_does_not_panic() {
        let mut vals = [1.0, f64::NAN, -2.5, 0.0, f64::INFINITY, f64::NEG_INFINITY];
        vals.sort_by(f64::total_cmp);
        assert_eq!(vals[0], f64::NEG_INFINITY);
        assert_eq!(vals[1], -2.5);
        assert_eq!(vals[2], 0.0);
        assert_eq!(vals[3], 1.0);
        assert_eq!(vals[4], f64::INFINITY);
        assert!(vals[5].is_nan());
    }
}
