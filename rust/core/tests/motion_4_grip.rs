// Gate tests for motion-4: distal-tip mitten rigs hold the sword handle
// inside the palm pocket and curl to >= 0.90 on all four attack presets under
// motion-1's unchanged skin-based grip measure, without moving five-finger
// or standard-mitten grips.

use motion_core::attack::{build, parse_spec, preset, PRESETS};
use motion_core::contact::Options;
use motion_core::glb::{set_node_trs, Affine, Document};
use motion_core::json::Json;
use motion_core::math::{Quat, Vec3};
use motion_core::motioncheck::{sample, Subject};
use motion_core::readcam::{BladePose, CamOptions, Stage, SKIN_PALM_GOOD_M};
use motion_core::rig::Animation;

fn push_box(
    positions: &mut Vec<Vec<f64>>,
    joints: &mut Vec<[u16; 4]>,
    weights: &mut Vec<Vec<f64>>,
    indices: &mut Vec<u16>,
    pts: [Vec3; 8],
    joint: u16,
) {
    let base = positions.len() as u16;
    for p in pts {
        positions.push(vec![p.x, p.y, p.z]);
        joints.push([joint, 0, 0, 0]);
        weights.push(vec![1.0, 0.0, 0.0, 0.0]);
    }
    let quads = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [1, 5, 6, 2],
        [2, 6, 7, 3],
        [3, 7, 4, 0],
    ];
    for q in quads {
        for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
            for k in t {
                indices.push(base + k as u16);
            }
        }
    }
}

fn seg_box(a: Vec3, b: Vec3, r: f64) -> [Vec3; 8] {
    let d = (b - a).normalized();
    let helper = if d.y.abs() > 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = d.cross(helper).normalized().scale(r);
    let w = d.cross(u).normalized().scale(r);
    let mut out = [Vec3::ZERO; 8];
    let mut idx = 0;
    for p in [a, b] {
        for (su, sw) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            out[idx] = p + u.scale(su) + w.scale(sw);
            idx += 1;
        }
    }
    out
}

fn aabb_box(lo: Vec3, hi: Vec3) -> [Vec3; 8] {
    [
        Vec3::new(lo.x, lo.y, lo.z),
        Vec3::new(lo.x, hi.y, lo.z),
        Vec3::new(lo.x, hi.y, hi.z),
        Vec3::new(lo.x, lo.y, hi.z),
        Vec3::new(hi.x, lo.y, lo.z),
        Vec3::new(hi.x, hi.y, lo.z),
        Vec3::new(hi.x, hi.y, hi.z),
        Vec3::new(hi.x, lo.y, hi.z),
    ]
}

/// Synthetic canonical humanoid with a distal-tip mitten hand:
/// `Hand -> Finger1` spans 0.184 m (> 2x `Finger1..3` = 0.048 m), with a
/// realistic cupped hand mesh weighted to `DEF-hand` whose palm surface sits
/// 3.2 cm below the bone axis and dorsal surface 2.2 cm above it (so pushing
/// the handle 2.6 cm out along the palm normal leaves the bottom exposed,
/// whereas seating it inside the palm pocket wraps 360 deg).
fn synthetic_cupped_distal_mitten() -> Document {
    struct JointSpec {
        name: String,
        parent: Option<usize>,
        pos: Vec3,
        mesh_seg: Option<(Vec3, f64)>,
        mesh_aabb: Option<(Vec3, Vec3)>,
    }
    let mut js: Vec<JointSpec> = Vec::new();
    let mut add_j = |name: &str,
                     parent: Option<usize>,
                     pos: [f64; 3],
                     mesh_seg: Option<(Vec3, f64)>,
                     mesh_aabb: Option<(Vec3, Vec3)>|
     -> usize {
        let id = js.len();
        js.push(JointSpec {
            name: name.to_string(),
            parent,
            pos: Vec3::new(pos[0], pos[1], pos[2]),
            mesh_seg,
            mesh_aabb,
        });
        id
    };

    let root = add_j("root", None, [0.0, 0.0, 0.0], None, None);
    let hips = add_j(
        "DEF-spine",
        Some(root),
        [0.0, 0.95, 0.0],
        Some((Vec3::new(0.0, 1.05, 0.0), 0.07)),
        None,
    );
    let sp1 = add_j(
        "DEF-spine.001",
        Some(hips),
        [0.0, 1.05, 0.0],
        Some((Vec3::new(0.0, 1.2, 0.0), 0.07)),
        None,
    );
    let sp2 = add_j(
        "DEF-spine.002",
        Some(sp1),
        [0.0, 1.2, 0.0],
        Some((Vec3::new(0.0, 1.35, 0.0), 0.08)),
        None,
    );
    let chest = add_j(
        "DEF-spine.003",
        Some(sp2),
        [0.0, 1.35, 0.0],
        Some((Vec3::new(0.0, 1.5, 0.0), 0.08)),
        None,
    );
    let neck = add_j(
        "DEF-spine.004",
        Some(chest),
        [0.0, 1.5, 0.0],
        Some((Vec3::new(0.0, 1.6, 0.0), 0.04)),
        None,
    );
    let _head = add_j(
        "DEF-spine.006",
        Some(neck),
        [0.0, 1.6, 0.0],
        Some((Vec3::new(0.0, 1.72, 0.0), 0.07)),
        None,
    );

    for (side, sx) in [("L", 1.0), ("R", -1.0)] {
        let sh = add_j(
            &format!("DEF-shoulder.{side}"),
            Some(chest),
            [0.04 * sx, 1.45, 0.0],
            Some((Vec3::new(0.17 * sx, 1.45, 0.0), 0.04)),
            None,
        );
        let ua = add_j(
            &format!("DEF-upper_arm.{side}"),
            Some(sh),
            [0.17 * sx, 1.45, 0.0],
            Some((Vec3::new(0.43 * sx, 1.45, 0.0), 0.04)),
            None,
        );
        let fa = add_j(
            &format!("DEF-forearm.{side}"),
            Some(ua),
            [0.43 * sx, 1.45, 0.0],
            Some((Vec3::new(0.68 * sx, 1.45, 0.0), 0.035)),
            None,
        );
        let (hx0, hx1) = if sx < 0.0 {
            (-0.87, -0.68)
        } else {
            (0.68, 0.87)
        };
        let hand = add_j(
            &format!("DEF-hand.{side}"),
            Some(fa),
            [0.68 * sx, 1.45, 0.0],
            None,
            Some((Vec3::new(hx0, 1.418, -0.040), Vec3::new(hx1, 1.472, 0.040))),
        );
        let f1 = add_j(
            &format!("DEF-f_middle.01.{side}"),
            Some(hand),
            [0.864 * sx, 1.45, 0.0],
            None,
            None,
        );
        let f2 = add_j(
            &format!("DEF-f_middle.02.{side}"),
            Some(f1),
            [0.888 * sx, 1.45, 0.0],
            None,
            None,
        );
        let _f3 = add_j(
            &format!("DEF-f_middle.03.{side}"),
            Some(f2),
            [0.912 * sx, 1.45, 0.009],
            None,
            None,
        );

        let th = add_j(
            &format!("DEF-thigh.{side}"),
            Some(hips),
            [0.09 * sx, 0.92, 0.0],
            Some((Vec3::new(0.09 * sx, 0.5, 0.0), 0.05)),
            None,
        );
        let sn = add_j(
            &format!("DEF-shin.{side}"),
            Some(th),
            [0.09 * sx, 0.5, 0.0],
            Some((Vec3::new(0.09 * sx, 0.08, 0.0), 0.04)),
            None,
        );
        let ft = add_j(
            &format!("DEF-foot.{side}"),
            Some(sn),
            [0.09 * sx, 0.08, 0.0],
            Some((Vec3::new(0.09 * sx, 0.02, 0.12), 0.035)),
            None,
        );
        let _toe = add_j(
            &format!("DEF-toe.{side}"),
            Some(ft),
            [0.09 * sx, 0.02, 0.12],
            Some((Vec3::new(0.09 * sx, 0.02, 0.18), 0.025)),
            None,
        );
    }

    let n = js.len();
    let mut doc = Document {
        json: Json::obj(vec![(
            "asset",
            Json::obj(vec![
                ("version", Json::str("2.0")),
                ("generator", Json::str("motionforge motion_4 fixture")),
            ]),
        )]),
        bin: Vec::new(),
    };
    let mut world = vec![Affine::IDENTITY; n];
    let mut nodes: Vec<Json> = Vec::with_capacity(n + 2);
    for (i, j) in js.iter().enumerate() {
        let want = Affine::from_trs(j.pos, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        let pw = j.parent.map(|p| world[p]).unwrap_or(Affine::IDENTITY);
        let local = pw.inverse().unwrap().mul(&want);
        world[i] = want;
        let (t, r, s) = local.decompose();
        let mut node = Json::obj(vec![("name", Json::str(&j.name))]);
        set_node_trs(&mut node, t, r, s);
        let kids: Vec<Json> = js
            .iter()
            .enumerate()
            .filter(|(_, c)| c.parent == Some(i))
            .map(|(k, _)| Json::num(k as f64))
            .collect();
        if !kids.is_empty() {
            node.set("children", Json::Arr(kids));
        }
        nodes.push(node);
    }

    let hand_r = js.iter().position(|j| j.name == "DEF-hand.R").unwrap();
    let sock_idx = nodes.len();
    let mut sock_node = Json::obj(vec![("name", Json::str("Socket_Hand_R"))]);
    set_node_trs(
        &mut sock_node,
        Vec3::new(-0.08, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    );
    nodes.push(sock_node);
    let mut hand_kids = nodes[hand_r]
        .get("children")
        .and_then(Json::as_arr)
        .map(|k| k.to_vec())
        .unwrap_or_default();
    hand_kids.push(Json::num(sock_idx as f64));
    nodes[hand_r].set("children", Json::Arr(hand_kids));

    let mut positions = Vec::new();
    let mut joints = Vec::new();
    let mut weights = Vec::new();
    let mut indices = Vec::new();
    for (i, j) in js.iter().enumerate() {
        if let Some((b, r)) = j.mesh_seg {
            push_box(
                &mut positions,
                &mut joints,
                &mut weights,
                &mut indices,
                seg_box(j.pos, b, r),
                i as u16,
            );
        }
        if let Some((lo, hi)) = j.mesh_aabb {
            push_box(
                &mut positions,
                &mut joints,
                &mut weights,
                &mut indices,
                aabb_box(lo, hi),
                i as u16,
            );
        }
    }

    let pos_acc = doc.push_float_accessor(&positions, "VEC3", true);
    let j_acc = doc.push_joints_accessor(&joints);
    let w_acc = doc.push_float_accessor(&weights, "VEC4", false);
    let mut ibytes = Vec::new();
    for i in &indices {
        ibytes.extend_from_slice(&i.to_le_bytes());
    }
    while !doc.bin.len().is_multiple_of(4) {
        doc.bin.push(0);
    }
    let off = doc.bin.len();
    doc.bin.extend_from_slice(&ibytes);
    let views = doc.array_mut("bufferViews");
    views.push(Json::obj(vec![
        ("buffer", Json::num(0.0)),
        ("byteOffset", Json::num(off as f64)),
        ("byteLength", Json::num(ibytes.len() as f64)),
        ("target", Json::num(34963.0)),
    ]));
    let view = views.len() - 1;
    let accs = doc.array_mut("accessors");
    accs.push(Json::obj(vec![
        ("bufferView", Json::num(view as f64)),
        ("componentType", Json::num(5123.0)),
        ("count", Json::num(indices.len() as f64)),
        ("type", Json::str("SCALAR")),
    ]));
    let idx_acc = accs.len() - 1;
    let ibm_rows: Vec<Vec<f64>> = world
        .iter()
        .map(|w| w.inverse().unwrap().to_gltf_matrix())
        .collect();
    let ibm = doc.push_float_accessor(&ibm_rows, "MAT4", false);
    let mesh_node_idx = nodes.len();
    nodes.push(Json::obj(vec![
        ("name", Json::str("Body")),
        ("mesh", Json::num(0.0)),
        ("skin", Json::num(0.0)),
    ]));
    doc.json.set("nodes", Json::Arr(nodes));
    doc.json.set(
        "meshes",
        Json::Arr(vec![Json::obj(vec![
            ("name", Json::str("Body")),
            (
                "primitives",
                Json::Arr(vec![Json::obj(vec![
                    (
                        "attributes",
                        Json::obj(vec![
                            ("POSITION", Json::num(pos_acc as f64)),
                            ("JOINTS_0", Json::num(j_acc as f64)),
                            ("WEIGHTS_0", Json::num(w_acc as f64)),
                        ]),
                    ),
                    ("indices", Json::num(idx_acc as f64)),
                ])]),
            ),
        ])]),
    );
    doc.json.set(
        "skins",
        Json::Arr(vec![Json::obj(vec![
            (
                "joints",
                Json::Arr((0..n).map(|j| Json::num(j as f64)).collect()),
            ),
            ("inverseBindMatrices", Json::num(ibm as f64)),
        ])]),
    );
    doc.json.set(
        "scenes",
        Json::Arr(vec![Json::obj(vec![(
            "nodes",
            Json::Arr(vec![Json::num(0.0), Json::num(mesh_node_idx as f64)]),
        )])]),
    );
    doc.json.set("scene", Json::num(0.0));
    Document::parse(&doc.to_bytes().unwrap()).unwrap()
}

#[test]
fn distal_mitten_cupped_hand_reaches_0_9_on_all_four_presets() {
    let base = synthetic_cupped_distal_mitten();
    let cam_opts = CamOptions {
        blade_pose: BladePose::Socket,
        ..Default::default()
    };
    for name in PRESETS {
        let spec = parse_spec(preset(name).unwrap()).unwrap();
        let (built, rep) = build(&base, &spec, None).unwrap();
        assert!(rep.socket_rewritten, "{name}");
        let built = Document::parse(&built.to_bytes().unwrap()).unwrap();
        let subj = Subject::new(&built, &Options::default()).unwrap();
        let stage = Stage::new(&built, &subj, &cam_opts).unwrap();
        let anim = &Animation::load_all(&built).unwrap()[0];
        let (times, frames) = sample(&subj.rig, anim, None);
        let dt = (times[times.len() - 1] - times[0]) / (times.len() - 1) as f64;
        let (rc, _) = stage.read(&subj.rig, &frames, dt, &frames[0]);
        assert!(rc.grip.skin_based, "{name}: {:?}", rc.grip);
        assert!(
            rc.grip.palm_surface_m <= SKIN_PALM_GOOD_M,
            "{name}: palm_surface_m too large: {:?}",
            rc.grip
        );
        assert!(
            rc.grip.skin_wrap >= 0.85,
            "{name}: skin_wrap below 0.85: {:?}",
            rc.grip
        );
        assert!(
            rc.grip.curl_deg >= 31.5 && rc.grip.curl_deg < 36.0,
            "{name}: curl_deg should close onto hilt gently (31.5..36 deg), got {:?}",
            rc.grip
        );
        assert!(
            rc.grip.score >= 0.90,
            "{name}: distal-mitten grip score must be >= 0.90 under motion-1, got {:?}",
            rc.grip
        );
    }
}
