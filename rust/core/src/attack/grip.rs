//! Holding the sword: where the handle sits in the hand and how the
//! fingers close round it.
//!
//! The game's hold (`tracks::held_pose`, constants in the hand socket's
//! space) was made for another rig's socket: on a hero with real hands it
//! laid the handle across the back of the wrist with the hand open (the
//! owner: "he's not holding the sword properly"). On a hand with finger
//! bones the hold is read from the hand itself:
//!
//! - the handle runs across the palm under the knuckles, from the little
//!   finger's side out past the index finger (a hammer grip), canted a
//!   little toward the fingers;
//! - its middle is a handle's and a finger's thickness to the palm side of
//!   the knuckle line, a little way down the fingers, so the first finger
//!   bones wrap over it;
//! - each finger then curls (every joint in the proportion of a fist, from
//!   a grips file when one is given) until it touches the handle; the
//!   thumb comes round the other side the same way.
//!
//! The hold is written into the hand socket (origin at the grip's middle,
//! +Y along the blade, +X the leading edge toward the knuckles), so a
//! sword parented to the socket with no further offset sits in the hand.

use crate::glb::{canonical_quat, Document};
use crate::json::Json;
use crate::keypose::fit::RigInfo;
use crate::math::{Mat3, Quat, Vec3};
use crate::readcam::HEAD_SHARE;
use crate::rig::{Rig, Trs};

/// Handle and finger half-thickness, metres on a 2 m hero.
pub const HANDLE_R: f64 = 0.016;
pub const FINGER_R: f64 = 0.010;
/// The handle's middle down the fingers from the knuckle line, metres.
pub const DOWN_FINGERS: f64 = 0.020;
/// How far the blade cants toward the fingers (share of the finger
/// direction added to the knuckle line).
pub const CANT: f64 = 0.25;
/// How far a free hand's fingers curl (share of a fist).
pub const RELAXED: f64 = 0.35;
/// On a distal-tip mitten rig (`palm_len > 2 * finger_len`, where `Hand ->
/// Finger1` starts at the wrist and runs nearly to the fingertips), the
/// cupped hand mesh sits around the palm bone axis: the grip pocket sits at
/// `DISTAL_MITTEN_PALM_SHARE` along `Hand -> Finger1`, offset by
/// `DISTAL_MITTEN_PALM_OFFSET` (`0.5 * HANDLE_R`) toward the palm so the
/// handle seats inside the cupped pocket instead of out at the palm edge,
/// and the short distal tip curls up to `DISTAL_MITTEN_CURL` of a full fist
/// (~34 deg at `close = 0.9`, matching five-finger hands without folding).
pub const DISTAL_MITTEN_PALM_SHARE: f64 = 0.55;
pub const DISTAL_MITTEN_PALM_OFFSET: f64 = 0.008;
pub const DISTAL_MITTEN_CURL: f64 = 0.72;

fn is_distal_mitten(palm_len: f64, finger_len: f64) -> bool {
    finger_len > 1e-9 && palm_len > 2.0 * finger_len
}

const FINGERS: [&str; 5] = ["f_index", "f_middle", "f_ring", "f_pinky", "thumb"];
/// A fist when no grips file says: radians per joint (root, middle, tip).
const DEFAULT_FIST: [[f64; 3]; 5] = [
    [0.84, 0.99, 0.70],
    [0.92, 1.08, 0.75],
    [0.79, 0.93, 0.64],
    [0.42, 0.50, 0.50],
    [0.26, 0.31, 0.21],
];

/// A hand's hold of the sword, in the hand's own rotation frame (GLB
/// units from the wrist joint).
#[derive(Clone, Copy, Debug)]
pub struct PalmHold {
    /// Blade direction (guard to tip).
    pub blade: Vec3,
    /// The grip's middle from the wrist joint.
    pub grip: Vec3,
    /// The socket's rotation: +Y the blade, +X the leading edge.
    pub socket: Quat,
}

fn chain(info: &RigInfo, finger: &str, s: usize) -> Option<[usize; 3]> {
    let side = ["L", "R"][s];
    // Through the name mapper, so a Mixamo or other rig's fingers count.
    let f = |k: usize| {
        let name = format!("DEF-{finger}.0{k}.{side}");
        info.map.node_of(&name).or_else(|| info.rig.find(&name))
    };
    Some([f(1)?, f(2)?, f(3)?])
}

fn bent(info: &RigInfo, node: usize, angle: f64) -> Trs {
    let mut t = info.rig.rest[node];
    t.r =
        t.r.mul(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), angle));
    t
}

/// Rotate `node` by `angle` about a world-space rest axis `world_axis`,
/// independent of how the rig rolled its joint local axes.
fn bent_world(info: &RigInfo, node: usize, world_axis: Vec3, angle: f64) -> Trs {
    let mut t = info.rig.rest[node];
    let local_axis = info.rig.rest_world[node]
        .rotation()
        .conj()
        .rotate_vec(world_axis)
        .normalized();
    t.r = canonical_quat(t.r.mul(Quat::from_axis_angle(local_axis, angle)));
    t
}

/// A single unified finger / mitten chain (2 or 3 bones, knuckle first) on
/// hand `s` when the hand does not have separate index and pinky chains:
/// first `DEF-f_middle` from the mapper, else a single child chain under
/// the hand node (for rigs whose finger chain was left outside the skin).
pub fn single_chain(info: &RigInfo, s: usize) -> Option<Vec<usize>> {
    if chain(info, "f_index", s).is_some() && chain(info, "f_pinky", s).is_some() {
        return None;
    }
    let side = ["L", "R"][s];
    let mapped: Vec<usize> = (1..=3)
        .filter_map(|k| {
            let name = format!("DEF-f_middle.0{k}.{side}");
            info.map.node_of(&name).or_else(|| info.rig.find(&name))
        })
        .collect();
    if mapped.len() >= 2 {
        return Some(mapped);
    }
    let any_other = ["f_index", "f_ring", "f_pinky"].iter().any(|finger| {
        (1..=2)
            .filter_map(|k| info.map.node_of(&format!("DEF-{finger}.0{k}.{side}")))
            .count()
            >= 2
    });
    if any_other {
        return None;
    }
    let hand = info.hand[s];
    let skip = |n: &str| {
        n.starts_with("Socket_") || n.starts_with("DEF-palm.") || crate::helpers::is_helper(n)
    };
    let mut found: Vec<Vec<usize>> = Vec::new();
    for &c in &info.rig.children[hand] {
        if skip(&info.rig.names[c]) {
            continue;
        }
        let mut ch = vec![c];
        while ch.len() < 3 {
            let cur = *ch.last().unwrap();
            let kids: Vec<usize> = info.rig.children[cur]
                .iter()
                .copied()
                .filter(|&k| !skip(&info.rig.names[k]))
                .collect();
            if kids.len() != 1 {
                break;
            }
            ch.push(kids[0]);
        }
        if ch.len() >= 2 {
            found.push(ch);
        }
    }
    (found.len() == 1).then(|| found.swap_remove(0))
}

/// Rest-world `(finger_dir, palm_normal, knuckle_line)` for a single
/// unified finger chain `chain` (2 or 3 bones, knuckle first):
/// - `finger_dir`: unit vector from `chain[0]` to `chain[1]`
/// - `palm_normal`: unit vector perpendicular to `finger_dir` pointing out
///   of the palm, derived from the standing hand's anatomical plane (`fwd`
///   and `finger_dir`) so a sideways rest phalanx kink never rolls the
///   blade; rest curl only flips the sign when strongly opposite (`< -0.85`,
///   a palm-up rest pose)
/// - `knuckle_line`: unit vector from the little-finger edge toward the
///   index/thumb edge.
pub fn single_finger_axes(
    rig: &Rig,
    chain: &[usize],
    right: bool,
    fwd: Vec3,
) -> Option<(Vec3, Vec3, Vec3)> {
    if chain.len() < 2 {
        return None;
    }
    let along = rig.head(chain[1]) - rig.head(chain[0]);
    if along.length() < 1e-9 {
        return None;
    }
    let fd = along.normalized();
    let fwd_p = fwd - fd.scale(fwd.dot(fd));
    let mut palm = if fwd_p.length() > 0.2 {
        let kn0 = fwd_p.normalized();
        if right {
            kn0.cross(fd).normalized()
        } else {
            fd.cross(kn0).normalized()
        }
    } else {
        let down = Vec3::new(0.0, -1.0, 0.0);
        let down_p = down - fd.scale(down.dot(fd));
        if down_p.length() < 1e-9 {
            return None;
        }
        down_p.normalized()
    };
    if chain.len() >= 3 {
        let d1 = rig.head(chain[2]) - rig.head(chain[1]);
        let d1_len = d1.length();
        if d1_len > 1e-9 {
            let perp = d1 - fd.scale(d1.dot(fd));
            if perp.length() > 0.0523 * d1_len && d1.dot(fd) > 0.0 {
                let curl_dir = perp.normalized();
                if curl_dir.dot(palm) < -0.85 {
                    palm = palm.scale(-1.0);
                }
            }
        }
    }
    let kn = if right {
        fd.cross(palm).normalized()
    } else {
        palm.cross(fd).normalized()
    };
    Some((fd, palm, kn))
}

/// Where the handle sits in hand `s` (module doc); None on a hand with no
/// finger bones. `unit` is GLB units per metre on a 2 m hero.
pub fn palm_hold(info: &RigInfo, s: usize, unit: f64) -> Option<PalmHold> {
    let rig = &info.rig;
    if let (Some(index), Some(pinky), Some(middle)) = (
        chain(info, "f_index", s),
        chain(info, "f_pinky", s),
        chain(info, "f_middle", s),
    ) {
        let (ki, kp) = (rig.head(index[0]), rig.head(pinky[0]));
        let along = rig.head(middle[1]) - rig.head(middle[0]);
        if along.length() < 1e-9 || (ki - kp).length() < 1e-9 {
            return None;
        }
        let fd = along.normalized();
        let kn = (ki - kp).normalized();
        // The palm's side: the thumb's root sits on it, below the knuckle
        // line. Without a thumb, where the middle finger's second joint goes
        // when its first curls (a Rigify finger's curl axis; other rigs curl
        // about other local axes, so the thumb comes first).
        let mut palm = fd.cross(kn).normalized();
        let toward = match chain(info, "thumb", s) {
            Some(thumb) => rig.head(thumb[0]) - (ki + kp).scale(0.5),
            None => {
                let mut l: Vec<Option<Trs>> = vec![None; rig.names.len()];
                l[middle[0]] = Some(bent(info, middle[0], 0.5));
                rig.world(&l)[middle[1]].t - rig.head(middle[1])
            }
        };
        if toward.dot(palm) < 0.0 {
            palm = palm.scale(-1.0);
        }
        let centre = (ki + kp).scale(0.5)
            + palm.scale((HANDLE_R + FINGER_R) * unit)
            + fd.scale(DOWN_FINGERS * unit);
        let blade = (kn + fd.scale(CANT)).normalized();
        let edge = (fd - blade.scale(fd.dot(blade))).normalized();
        let world = Quat::from_mat3(Mat3::from_cols(edge, blade, edge.cross(blade)));
        let hw = rig.rest_world[info.hand[s]];
        let inv = hw.rotation().conj();
        return Some(PalmHold {
            blade: inv.rotate_vec(blade),
            grip: inv.rotate_vec(centre - hw.t),
            socket: inv.mul(world).normalized(),
        });
    }
    let mid = single_chain(info, s)?;
    let (fd, palm, kn) = single_finger_axes(rig, &mid, s == 1, info.fwd)?;
    // When `Hand -> mid[0]` is over twice the finger chain's own length
    // (a distal-tip mitten rig whose hand bone spans the palm and proximal
    // fingers nearly to the tip), anchor the grip pocket inside the cupped
    // palm instead of floating it out at the fingertips or palm edge.
    let hand_head = rig.head(info.hand[s]);
    let palm_vec = rig.head(mid[0]) - hand_head;
    let palm_len = palm_vec.length();
    let finger_len = (rig.head(*mid.last().unwrap()) - rig.head(mid[0])).length();
    let distal_mitten = is_distal_mitten(palm_len, finger_len);
    let (base, palm_offset) = if distal_mitten {
        (
            hand_head + palm_vec.scale(DISTAL_MITTEN_PALM_SHARE),
            DISTAL_MITTEN_PALM_OFFSET,
        )
    } else {
        (
            rig.head(mid[0]) + fd.scale(DOWN_FINGERS * unit),
            HANDLE_R + FINGER_R,
        )
    };
    let centre = base + palm.scale(palm_offset * unit);
    let blade = (kn + fd.scale(CANT)).normalized();
    let edge = (fd - blade.scale(fd.dot(blade))).normalized();
    let world = Quat::from_mat3(Mat3::from_cols(edge, blade, edge.cross(blade)));
    let hw = rig.rest_world[info.hand[s]];
    let inv = hw.rotation().conj();
    Some(PalmHold {
        blade: inv.rotate_vec(blade),
        grip: inv.rotate_vec(centre - hw.t),
        socket: inv.mul(world).normalized(),
    })
}

/// Put each hand's `Socket_Hand_<side>` in its palm (`palm_hold`: the
/// grip's middle, +Y along the blade a fist holds, +X its leading edge),
/// adding the socket when missing. Hands without finger bones are left as
/// they are. Mocap sword clips (Mixamo's) are made for a fist round a
/// handle there: a socket elsewhere points the blade where the clip did
/// not mean it (the game's Quirin clips held the blade along the forearm).
/// Returns the sockets placed.
pub fn place_sockets(doc: &mut Document) -> Result<Vec<String>, String> {
    let info = RigInfo::new(doc)?;
    let rig = &info.rig;
    let head = info.head.ok_or("rig has no head joint")?;
    let h = (rig.head(head).y - info.ground) / HEAD_SHARE;
    let unit = h / 2.0;
    let mut placed = Vec::new();
    for (s, side) in ["L", "R"].iter().enumerate() {
        let Some(ph) = palm_hold(&info, s, unit) else {
            continue;
        };
        let hand = info.hand[s];
        let hw = rig.rest_world[hand];
        let world = hw.t + hw.rotation().rotate_vec(ph.grip);
        let inv = hw.inverse().ok_or("hand transform is singular")?;
        let (_, _, hs) = hw.decompose();
        let inv_s = Vec3::new(1.0 / hs.x, 1.0 / hs.y, 1.0 / hs.z);
        let name = format!("Socket_Hand_{side}");
        let node_index = match info.socket[s] {
            Some(n) => n,
            None => {
                let nodes = doc.array_mut("nodes");
                nodes.push(Json::obj(vec![("name", Json::str(&name))]));
                let sn = nodes.len() - 1;
                let mut kids = nodes[hand]
                    .get("children")
                    .and_then(Json::as_arr)
                    .map(|k| k.to_vec())
                    .unwrap_or_default();
                kids.push(Json::num(sn as f64));
                nodes[hand].set("children", Json::Arr(kids));
                sn
            }
        };
        let node = &mut doc.array_mut("nodes")[node_index];
        crate::glb::set_node_trs(node, inv.apply(world), ph.socket, inv_s);
        placed.push(name);
    }
    Ok(placed)
}

fn line_dist(p: Vec3, on: Vec3, dir: Vec3) -> f64 {
    let v = p - on;
    (v - dir.scale(v.dot(dir))).length()
}

/// Finger joint locals for hand `s`: curled `amount` of a fist, or (None)
/// each finger closed until it touches the handle of `hold`. `fist` gives
/// a joint's bend in a fist by its name (a grips file), else a default.
pub fn fingers(
    info: &RigInfo,
    s: usize,
    hold: Option<&PalmHold>,
    amount: Option<f64>,
    unit: f64,
    fist: &dyn Fn(&str) -> Option<f64>,
) -> Vec<(usize, Trs)> {
    let rig = &info.rig;
    let mut out = Vec::new();
    let hw = rig.rest_world[info.hand[s]];
    let handle = hold.map(|h| {
        (
            hw.t + hw.rotation().rotate_vec(h.grip),
            hw.rotation().rotate_vec(h.blade),
        )
    });
    if let Some(mid) = single_chain(info, s) {
        if let Some((fd, palm, kn)) = single_finger_axes(rig, &mid, s == 1, info.fwd) {
            let curl_axis = fd.cross(palm).normalized();
            let palm_len = (rig.head(mid[0]) - rig.head(info.hand[s])).length();
            let finger_len = (rig.head(*mid.last().unwrap()) - rig.head(mid[0])).length();
            let distal_mitten = is_distal_mitten(palm_len, finger_len);
            let cap = if distal_mitten {
                DISTAL_MITTEN_CURL
            } else {
                1.0
            };
            let bends: Vec<f64> = (0..mid.len())
                .map(|k| fist(&rig.names[mid[k]]).unwrap_or(DEFAULT_FIST[1][k]) * cap)
                .collect();
            let pose = |c: f64| -> Vec<Option<Trs>> {
                let mut l: Vec<Option<Trs>> = vec![None; rig.names.len()];
                for k in 0..mid.len() {
                    l[mid[k]] = Some(bent_world(info, mid[k], curl_axis, c * bends[k]));
                }
                l
            };
            let c = match (amount, handle) {
                (Some(a), _) => {
                    if distal_mitten {
                        a.min(1.0)
                    } else {
                        a
                    }
                }
                (None, None) => RELAXED,
                (None, Some(_)) if distal_mitten => 1.0,
                (None, Some((on, dir))) => {
                    let touch = (HANDLE_R + FINGER_R) * unit;
                    let mut c = 0.0;
                    let mut found = 1.0;
                    while c <= 1.6 {
                        let w = rig.world(&pose(c));
                        let last = mid.len() - 1;
                        let (p1, p2) = (w[mid[last - 1]].t, w[mid[last]].t);
                        let tip = p2 + (p2 - p1).scale(0.8);
                        let hit = [(p1, p2), (p2, tip)].iter().any(|(a, b)| {
                            (0..=4).any(|k| line_dist(a.lerp(*b, k as f64 / 4.0), on, dir) <= touch)
                        });
                        if hit {
                            found = c;
                            break;
                        }
                        c += 0.02;
                    }
                    found
                }
            };
            for k in 0..mid.len() {
                out.push((mid[k], bent_world(info, mid[k], curl_axis, c * bends[k])));
            }
            if let Some(thumb) = chain(info, "thumb", s) {
                let td = (rig.head(thumb[1]) - rig.head(thumb[0])).normalized();
                // Thumb closes across the palm toward the opposite side (`-kn` + `palm`).
                let toward = (palm - kn).normalized();
                let t_axis = td.cross(toward);
                if t_axis.length() > 1e-6 {
                    let t_axis = t_axis.normalized();
                    for (k, &jt) in thumb.iter().enumerate() {
                        let b = fist(&rig.names[jt]).unwrap_or(DEFAULT_FIST[4][k]);
                        out.push((jt, bent_world(info, jt, t_axis, c * b)));
                    }
                }
            }
            return out;
        }
    }
    for (fi, finger) in FINGERS.iter().enumerate() {
        let Some(joints) = chain(info, finger, s) else {
            continue;
        };
        let bends: Vec<f64> = (0..3)
            .map(|k| fist(&rig.names[joints[k]]).unwrap_or(DEFAULT_FIST[fi][k]))
            .collect();
        let pose = |c: f64| -> Vec<Option<Trs>> {
            let mut l: Vec<Option<Trs>> = vec![None; rig.names.len()];
            for k in 0..3 {
                l[joints[k]] = Some(bent(info, joints[k], c * bends[k]));
            }
            l
        };
        let c = match (amount, handle) {
            (Some(a), _) => a,
            (None, None) => RELAXED,
            (None, Some((on, dir))) => {
                // Close until any part of the finger touches the handle.
                let touch = (HANDLE_R + FINGER_R) * unit;
                let most = if *finger == "thumb" { 3.0 } else { 1.6 };
                let mut c = 0.0;
                let mut found = 1.0;
                while c <= most {
                    let w = rig.world(&pose(c));
                    let (p1, p2) = (w[joints[1]].t, w[joints[2]].t);
                    let tip = p2 + (p2 - p1).scale(0.8);
                    let hit = [(p1, p2), (p2, tip)].iter().any(|(a, b)| {
                        (0..=4).any(|k| line_dist(a.lerp(*b, k as f64 / 4.0), on, dir) <= touch)
                    });
                    if hit {
                        found = c;
                        break;
                    }
                    c += 0.02;
                }
                found
            }
        };
        for k in 0..3 {
            out.push((joints[k], bent(info, joints[k], c * bends[k])));
        }
    }
    out
}

/// The `fist` entry of a hand-tools grips file (`grips.ron`): every
/// `"joint name": radians` pair under it, both hands.
pub fn parse_grips(text: &str, entry: &str) -> Result<Vec<(String, f64)>, String> {
    let key = format!("\"{entry}\"");
    let at = text
        .find(&key)
        .ok_or_else(|| format!("no \"{entry}\" entry in the grips file"))?;
    let rest = &text[at + key.len()..];
    let open = rest.find('{').ok_or("grips file: no { after the entry")?;
    // The entry's own block: to its matching brace.
    let mut depth = 0usize;
    let mut end = None;
    for (i, ch) in rest[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let block = &rest[open..end.ok_or("grips file: unbalanced braces")?];
    let mut out = Vec::new();
    let mut parts = block.split('"');
    parts.next();
    while let (Some(name), Some(after)) = (parts.next(), parts.next()) {
        let after = after.trim_start();
        let Some(val) = after.strip_prefix(':') else {
            continue;
        };
        let num: String = val
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            .collect();
        if let Ok(v) = num.parse::<f64>() {
            if !v.is_finite() || v.abs() > 4.0 {
                return Err(format!("grips file: {name} bends {v} rad"));
            }
            out.push((name.to_string(), v));
        }
    }
    if out.is_empty() {
        return Err(format!("grips file: \"{entry}\" has no joints"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grips_file_gives_the_named_entrys_joints_only() {
        let text = r#"( grips: {
            "door": { "R": { "DEF-f_index.01.R": 0.1, }, },
            "fist": {
                "L": { "DEF-f_index.01.L": 0.8116, "DEF-thumb.03.L": 0.2142, },
                "R": { "DEF-f_index.01.R": 0.8378, },
            },
            "sword": { "R": { "DEF-f_index.01.R": 0.2880, }, },
        }, )"#;
        let g = parse_grips(text, "fist").unwrap();
        assert_eq!(
            g,
            vec![
                ("DEF-f_index.01.L".to_string(), 0.8116),
                ("DEF-thumb.03.L".to_string(), 0.2142),
                ("DEF-f_index.01.R".to_string(), 0.8378),
            ]
        );
        assert!(parse_grips(text, "claw").is_err());
        assert!(parse_grips(r#""fist": { "R": { "x": 9.0 } }"#, "fist").is_err());
    }

    #[test]
    fn single_finger_axes_match_anatomical_palm_and_rest_curl() {
        use crate::glb::Affine;
        let make = |pts: &[Vec3]| -> Rig {
            let n = pts.len();
            let rest = Trs {
                t: Vec3::ZERO,
                r: Quat::IDENTITY,
                s: Vec3::new(1.0, 1.0, 1.0),
            };
            Rig {
                names: (0..n).map(|i| format!("b{i}")).collect(),
                parent: (0..n).map(|i| (i > 0).then_some(i - 1)).collect(),
                children: (0..n)
                    .map(|i| if i + 1 < n { vec![i + 1] } else { vec![] })
                    .collect(),
                rest: vec![rest; n],
                rest_local: vec![Affine::IDENTITY; n],
                rest_world: pts
                    .iter()
                    .map(|&p| Affine::from_trs(p, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0)))
                    .collect(),
                skins: vec![(0..n).collect()],
                order: (0..n).collect(),
            }
        };
        let fwd = Vec3::new(0.0, 0.0, 1.0);
        // Straight T-pose: right (-X) and left (+X) both put palm down (-Y)
        // and knuckle line (pinky -> index/thumb) forward (+Z).
        let r_tpose = make(&[
            Vec3::new(-0.76, 1.45, 0.0),
            Vec3::new(-0.80, 1.45, 0.0),
            Vec3::new(-0.83, 1.45, 0.0),
        ]);
        let (fd, palm, kn) = single_finger_axes(&r_tpose, &[0, 1, 2], true, fwd).unwrap();
        assert!(fd.approx_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-9));
        assert!(palm.approx_eq(Vec3::new(0.0, -1.0, 0.0), 1e-9));
        assert!(kn.approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9));

        let l_tpose = make(&[
            Vec3::new(0.76, 1.45, 0.0),
            Vec3::new(0.80, 1.45, 0.0),
            Vec3::new(0.83, 1.45, 0.0),
        ]);
        let (fd, palm, kn) = single_finger_axes(&l_tpose, &[0, 1, 2], false, fwd).unwrap();
        assert!(fd.approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9));
        assert!(palm.approx_eq(Vec3::new(0.0, -1.0, 0.0), 1e-9));
        assert!(kn.approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9));

        // Arms hanging at sides with a slight inward rest curl (+X on R, -X on L):
        // rest curl and anatomical forward give the exact same palm and knuckle line.
        let r_hang = make(&[
            Vec3::new(-0.42, 0.80, 0.0),
            Vec3::new(-0.42, 0.78, 0.0),
            Vec3::new(-0.41, 0.75, 0.0),
        ]);
        let (fd, palm, kn) = single_finger_axes(&r_hang, &[0, 1, 2], true, fwd).unwrap();
        assert!(fd.approx_eq(Vec3::new(0.0, -1.0, 0.0), 1e-9));
        assert!(palm.approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9));
        assert!(kn.approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9));

        // A sideways rest kink on the distal phalanx (+Z) must NEVER roll the
        // palm normal or the blade away from the anatomical hand plane.
        let r_side_kink = make(&[
            Vec3::new(-0.42, 0.80, 0.0),
            Vec3::new(-0.42, 0.78, 0.0),
            Vec3::new(-0.42, 0.75, 0.015),
        ]);
        let (_, palm_k, kn_k) = single_finger_axes(&r_side_kink, &[0, 1, 2], true, fwd).unwrap();
        assert!(palm_k.approx_eq(Vec3::new(1.0, 0.0, 0.0), 1e-9));
        assert!(kn_k.approx_eq(Vec3::new(0.0, 0.0, 1.0), 1e-9));
    }

    #[test]
    fn distal_mitten_seats_socket_in_palm_pocket_and_curls_cleanly() {
        use crate::fixture::{humanoid_mitten, Naming};
        use crate::glb::{node_trs, set_node_trs};
        use crate::standardize::{standardize, Outcome};

        let raw = humanoid_mitten(Naming::Plain, false, 0.0).unwrap();
        let std_doc = match standardize(&raw, "humanoid").unwrap() {
            Outcome::Done(d, _) => Document::parse(&d.to_bytes().unwrap()).unwrap(),
            Outcome::Refused(_, why) => panic!("{why}"),
        };

        // Standard mitten (palm_len <= 2 * finger_len) keeps HANDLE_R + FINGER_R offset.
        let std_info = RigInfo::new(&std_doc).unwrap();
        let unit = 1.0;
        let std_mid = single_chain(&std_info, 1).unwrap();
        let (std_fd, std_palm, _) =
            single_finger_axes(&std_info.rig, &std_mid, true, std_info.fwd).unwrap();
        let std_ph = palm_hold(&std_info, 1, unit).unwrap();
        let std_hw = std_info.rig.rest_world[std_info.hand[1]];
        let std_world = std_hw.t + std_hw.rotation().rotate_vec(std_ph.grip);
        let std_expected = std_info.rig.head(std_mid[0])
            + std_fd.scale(DOWN_FINGERS * unit)
            + std_palm.scale((HANDLE_R + FINGER_R) * unit);
        assert!(std_world.approx_eq(std_expected, 1e-9));

        // Distal-tip mitten (Hand -> Finger1 = 0.184 m > 2 * 0.048 m):
        // seats inside the palm pocket (DISTAL_MITTEN_PALM_OFFSET) and curls 31.5..36 deg.
        let mut distal_doc = std_doc;
        let rig0 = Rig::from_doc(&distal_doc).unwrap();
        for (side, sign) in [("L", 1.0), ("R", -1.0)] {
            let f1 = rig0.find(&format!("DEF-f_middle.01.{side}")).unwrap();
            let f2 = rig0.find(&format!("DEF-f_middle.02.{side}")).unwrap();
            let f3 = rig0.find(&format!("DEF-f_middle.03.{side}")).unwrap();
            let (_, r1, s1) = node_trs(&distal_doc.array("nodes")[f1]);
            set_node_trs(
                &mut distal_doc.array_mut("nodes")[f1],
                Vec3::new(sign * 0.184, 0.0, 0.0),
                r1,
                s1,
            );
            let (_, r2, s2) = node_trs(&distal_doc.array("nodes")[f2]);
            set_node_trs(
                &mut distal_doc.array_mut("nodes")[f2],
                Vec3::new(sign * 0.024, 0.0, 0.0),
                r2,
                s2,
            );
            let (_, r3, s3) = node_trs(&distal_doc.array("nodes")[f3]);
            set_node_trs(
                &mut distal_doc.array_mut("nodes")[f3],
                Vec3::new(sign * 0.024, 0.0, 0.0),
                r3,
                s3,
            );
        }
        let info = RigInfo::new(&distal_doc).unwrap();
        let mid = single_chain(&info, 1).unwrap();
        let (_, palm, _) = single_finger_axes(&info.rig, &mid, true, info.fwd).unwrap();
        let ph = palm_hold(&info, 1, unit).unwrap();
        let hw = info.rig.rest_world[info.hand[1]];
        let world_grip = hw.t + hw.rotation().rotate_vec(ph.grip);
        let hand_head = info.rig.head(info.hand[1]);
        let palm_vec = info.rig.head(mid[0]) - hand_head;
        let expected = hand_head
            + palm_vec.scale(DISTAL_MITTEN_PALM_SHARE)
            + palm.scale(DISTAL_MITTEN_PALM_OFFSET * unit);
        assert!(world_grip.approx_eq(expected, 1e-9));
        let palm_proj = (world_grip - hand_head).dot(palm);
        assert!((palm_proj - DISTAL_MITTEN_PALM_OFFSET * unit).abs() < 1e-9);
        assert!((palm_proj - (HANDLE_R + FINGER_R) * unit).abs() > 0.01);

        let curled = fingers(&info, 1, Some(&ph), Some(0.9), unit, &|_| None);
        assert_eq!(curled.len(), mid.len());
        let mean_deg: f64 = curled
            .iter()
            .map(|&(node, trs)| info.rig.rest[node].r.angle_to(trs.r).to_degrees())
            .sum::<f64>()
            / curled.len() as f64;
        assert!(
            mean_deg >= 31.5 && mean_deg < 36.0,
            "distal mitten mean curl should be in 31.5..36 deg, got {mean_deg}"
        );
    }

    #[test]
    fn distal_mitten_predicate_bounds() {
        assert!(is_distal_mitten(0.184, 0.048));
        assert!(!is_distal_mitten(0.096, 0.048));
        assert!(!is_distal_mitten(0.05, 0.05));
        assert!(!is_distal_mitten(0.10, 0.0));
        assert!(!is_distal_mitten(0.10, 1e-12));
        assert!(!is_distal_mitten(f64::NAN, 0.05));
        assert!(!is_distal_mitten(0.20, f64::NAN));
    }
}
