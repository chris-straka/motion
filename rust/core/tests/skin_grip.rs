// Skin-based grip measure (motion-1): handle-to-palm-surface distance and
// handle circumference wrap read from the weapon hand's skinned triangles,
// with no bone-ratio heuristic.

use motion_core::attack::{build, parse_spec, preset};
use motion_core::contact::Options;
use motion_core::fixture::{humanoid_mitten, Naming};
use motion_core::glb::{node_trs, set_node_trs, Affine, Document};
use motion_core::json::Json;
use motion_core::math::{Mat3, Quat, Vec3};
use motion_core::motioncheck::{sample, Subject};
use motion_core::readcam::{
    skin_grip_of, BladePose, CamOptions, Stage, SKIN_PALM_BAD_M, SKIN_PALM_GOOD_M, SKIN_WRAP_BAD,
    SKIN_WRAP_GOOD,
};
use motion_core::rig::{Animation, Rig};
use motion_core::standardize::{standardize, Outcome};

/// Helper: add a skinned box aligned to `a..b` with half-widths `ry`, `rz`
/// (in the box's cross-section) or axis-aligned bounds, weighted 100% to `joint`.
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

/// Procedural canonical DEF humanoid with a thin anatomical palm slab (2 cm
/// thick) and optionally five articulated finger chains per hand oriented so
/// local +X curls each finger into the palm (matching Rigify / `attack::grip`).
fn synthetic_hand_humanoid(with_fingers: bool) -> Document {
    struct JointSpec {
        name: String,
        parent: Option<usize>,
        pos: Vec3,
        rot: Quat,
        mesh_seg: Option<(Vec3, f64)>,
        mesh_aabb: Option<(Vec3, Vec3)>,
    }

    let mut js: Vec<JointSpec> = Vec::new();
    let mut add_j = |name: &str,
                     parent: Option<usize>,
                     pos: [f64; 3],
                     rot: Quat,
                     mesh_seg: Option<(Vec3, f64)>,
                     mesh_aabb: Option<(Vec3, Vec3)>|
     -> usize {
        let id = js.len();
        js.push(JointSpec {
            name: name.to_string(),
            parent,
            pos: Vec3::new(pos[0], pos[1], pos[2]),
            rot,
            mesh_seg,
            mesh_aabb,
        });
        id
    };

    let root = add_j("root", None, [0.0, 0.0, 0.0], Quat::IDENTITY, None, None);
    let hips = add_j(
        "DEF-spine",
        Some(root),
        [0.0, 0.95, 0.0],
        Quat::IDENTITY,
        Some((Vec3::new(0.0, 1.05, 0.0), 0.07)),
        None,
    );
    let sp1 = add_j(
        "DEF-spine.001",
        Some(hips),
        [0.0, 1.05, 0.0],
        Quat::IDENTITY,
        Some((Vec3::new(0.0, 1.2, 0.0), 0.07)),
        None,
    );
    let sp2 = add_j(
        "DEF-spine.002",
        Some(sp1),
        [0.0, 1.2, 0.0],
        Quat::IDENTITY,
        Some((Vec3::new(0.0, 1.35, 0.0), 0.08)),
        None,
    );
    let chest = add_j(
        "DEF-spine.003",
        Some(sp2),
        [0.0, 1.35, 0.0],
        Quat::IDENTITY,
        Some((Vec3::new(0.0, 1.5, 0.0), 0.08)),
        None,
    );
    let neck = add_j(
        "DEF-spine.004",
        Some(chest),
        [0.0, 1.5, 0.0],
        Quat::IDENTITY,
        Some((Vec3::new(0.0, 1.6, 0.0), 0.04)),
        None,
    );
    let _head = add_j(
        "DEF-spine.006",
        Some(neck),
        [0.0, 1.6, 0.0],
        Quat::IDENTITY,
        Some((Vec3::new(0.0, 1.72, 0.0), 0.07)),
        None,
    );

    for (side, sx) in [("L", 1.0), ("R", -1.0)] {
        let sh = add_j(
            &format!("DEF-shoulder.{side}"),
            Some(chest),
            [0.04 * sx, 1.45, 0.0],
            Quat::IDENTITY,
            Some((Vec3::new(0.17 * sx, 1.45, 0.0), 0.04)),
            None,
        );
        let ua = add_j(
            &format!("DEF-upper_arm.{side}"),
            Some(sh),
            [0.17 * sx, 1.45, 0.0],
            Quat::IDENTITY,
            Some((Vec3::new(0.43 * sx, 1.45, 0.0), 0.04)),
            None,
        );
        let fa = add_j(
            &format!("DEF-forearm.{side}"),
            Some(ua),
            [0.43 * sx, 1.45, 0.0],
            Quat::IDENTITY,
            Some((Vec3::new(0.68 * sx, 1.45, 0.0), 0.035)),
            None,
        );
        let (px0, px1) = if sx < 0.0 {
            (-0.76, -0.68)
        } else {
            (0.68, 0.76)
        };
        let hand = add_j(
            &format!("DEF-hand.{side}"),
            Some(fa),
            [0.68 * sx, 1.45, 0.0],
            Quat::IDENTITY,
            None,
            Some((Vec3::new(px0, 1.44, -0.036), Vec3::new(px1, 1.46, 0.036))),
        );
        // Right-handed frame where local +X curls the outward finger (+sx, 0, 0)
        // downward toward the palm (0, -1, 0):
        //   R (sx=-1): local +X = (0,0,1), +Y = (-1,0,0), +Z = (0,-1,0)
        //   L (sx=+1): local +X = (0,0,-1), +Y = (1,0,0), +Z = (0,-1,0)
        let f_rot = Quat::from_mat3(Mat3::from_cols(
            Vec3::new(0.0, 0.0, -sx),
            Vec3::new(sx, 0.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
        ))
        .normalized();
        if with_fingers {
            for (fname, z) in [
                ("f_index", 0.028),
                ("f_middle", 0.010),
                ("f_ring", -0.009),
                ("f_pinky", -0.028),
            ] {
                let f1 = add_j(
                    &format!("DEF-{fname}.01.{side}"),
                    Some(hand),
                    [0.76 * sx, 1.45, z],
                    f_rot,
                    Some((Vec3::new(0.798 * sx, 1.45, z), 0.009)),
                    None,
                );
                let f2 = add_j(
                    &format!("DEF-{fname}.02.{side}"),
                    Some(f1),
                    [0.798 * sx, 1.45, z],
                    f_rot,
                    Some((Vec3::new(0.826 * sx, 1.45, z), 0.009)),
                    None,
                );
                let _f3 = add_j(
                    &format!("DEF-{fname}.03.{side}"),
                    Some(f2),
                    [0.826 * sx, 1.45, z],
                    f_rot,
                    Some((Vec3::new(0.848 * sx, 1.45, z), 0.008)),
                    None,
                );
            }
            // Thumb sits on the +Z edge and curls across the palm (-Z, -Y)
            // about its local +X = (sx * 0.6, -0.8, 0.0) when bent positive.
            let t_x = Vec3::new(0.55 * sx, -0.835, 0.0).normalized();
            let t_y = Vec3::new(0.65 * sx, 0.43, 0.62).normalized();
            let t_z = t_x.cross(t_y).normalized();
            let t_y = t_z.cross(t_x).normalized();
            let t_rot = Quat::from_mat3(Mat3::from_cols(t_x, t_y, t_z)).normalized();
            let t1 = add_j(
                &format!("DEF-thumb.01.{side}"),
                Some(hand),
                [0.72 * sx, 1.435, 0.036],
                t_rot,
                Some((Vec3::new(0.75 * sx, 1.425, 0.048), 0.010)),
                None,
            );
            let t2 = add_j(
                &format!("DEF-thumb.02.{side}"),
                Some(t1),
                [0.75 * sx, 1.425, 0.048],
                t_rot,
                Some((Vec3::new(0.775 * sx, 1.418, 0.056), 0.009)),
                None,
            );
            let _t3 = add_j(
                &format!("DEF-thumb.03.{side}"),
                Some(t2),
                [0.775 * sx, 1.418, 0.056],
                t_rot,
                Some((Vec3::new(0.795 * sx, 1.414, 0.062), 0.008)),
                None,
            );
        }

        let th = add_j(
            &format!("DEF-thigh.{side}"),
            Some(hips),
            [0.09 * sx, 0.92, 0.0],
            Quat::IDENTITY,
            Some((Vec3::new(0.09 * sx, 0.5, 0.0), 0.05)),
            None,
        );
        let sn = add_j(
            &format!("DEF-shin.{side}"),
            Some(th),
            [0.09 * sx, 0.5, 0.0],
            Quat::IDENTITY,
            Some((Vec3::new(0.09 * sx, 0.08, 0.0), 0.04)),
            None,
        );
        let ft = add_j(
            &format!("DEF-foot.{side}"),
            Some(sn),
            [0.09 * sx, 0.08, 0.0],
            Quat::IDENTITY,
            Some((Vec3::new(0.09 * sx, 0.02, 0.12), 0.035)),
            None,
        );
        let _toe = add_j(
            &format!("DEF-toe.{side}"),
            Some(ft),
            [0.09 * sx, 0.02, 0.12],
            Quat::IDENTITY,
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
                ("generator", Json::str("motionforge skin_grip fixture")),
            ]),
        )]),
        bin: Vec::new(),
    };

    let mut world = vec![Affine::IDENTITY; n];
    let mut nodes: Vec<Json> = Vec::with_capacity(n + 2);
    for (i, j) in js.iter().enumerate() {
        let want = Affine::from_trs(j.pos, j.rot, Vec3::new(1.0, 1.0, 1.0));
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

    // Add Socket_Hand_R under DEF-hand.R so both with-fingers and flat-palm
    // tests have a socket ready.
    let hand_r = js.iter().position(|j| j.name == "DEF-hand.R").unwrap();
    let sock_idx = nodes.len();
    let mut sock_node = Json::obj(vec![("name", Json::str("Socket_Hand_R"))]);
    // Default socket sitting right against the underside of the flat palm slab
    // (1.6 cm below y=1.44, blade along +Z).
    set_node_trs(
        &mut sock_node,
        Vec3::new(-0.04, -0.026, 0.0),
        Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2),
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
    while doc.bin.len() % 4 != 0 {
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

fn score_socket_clip(doc: &Document) -> motion_core::readcam::ReadCam {
    let subj = Subject::new(doc, &Options::default()).unwrap();
    let mut cam_opts = CamOptions::default();
    cam_opts.blade_pose = BladePose::Socket;
    let stage = Stage::new(doc, &subj, &cam_opts).unwrap();
    let anim = &Animation::load_all(doc).unwrap()[0];
    let (times, frames) = sample(&subj.rig, anim, None);
    let dt = (times[times.len() - 1] - times[0]) / (times.len() - 1) as f64;
    stage.read(&subj.rig, &frames, dt, &frames[0]).0
}

#[test]
fn skin_grip_pure_scoring_and_constants() {
    assert!(SKIN_PALM_GOOD_M > 0.02 && SKIN_PALM_GOOD_M < SKIN_PALM_BAD_M);
    assert!(SKIN_WRAP_BAD > 0.15 && SKIN_WRAP_BAD < SKIN_WRAP_GOOD && SKIN_WRAP_GOOD <= 1.0);
    // Full wrap, close to palm skin, closed fingers -> 1.0.
    assert!((skin_grip_of(0.02, 0.95, Some(40.0)) - 1.0).abs() < 1e-12);
    // Close to palm and wrapped, but fingers never curled -> fails.
    assert!(skin_grip_of(0.02, 0.95, Some(1.0)) < 0.1);
    // Close to a flat open palm with no wrap (even without finger bones) -> fails.
    assert_eq!(skin_grip_of(0.015, SKIN_WRAP_BAD, None), 0.0);
    assert!(skin_grip_of(0.015, SKIN_WRAP_BAD + 0.05, None) < 0.2);
    // Distant palm (fingertip socket or 10 cm off) -> fails even if curled.
    assert_eq!(skin_grip_of(SKIN_PALM_BAD_M + 0.01, 0.95, Some(50.0)), 0.0);
    // Mid-wrap without finger chains sits halfway.
    let mid = skin_grip_of(0.01, 0.5 * (SKIN_WRAP_BAD + SKIN_WRAP_GOOD), None);
    assert!((mid - 0.5).abs() < 1e-12, "mid={mid}");
}

#[test]
fn skin_grip_measures_closed_vs_three_planted_bad_cases_and_json() {
    let base = synthetic_hand_humanoid(true);
    let spec = parse_spec(preset("sweep").unwrap()).unwrap();

    // 1. Authored closed five-finger grip: palm close, high wrap, score >= 0.9.
    let (closed_doc, _) = build(&base, &spec, None).unwrap();
    let closed_doc = Document::parse(&closed_doc.to_bytes().unwrap()).unwrap();
    let closed_rc = score_socket_clip(&closed_doc);
    assert!(closed_rc.grip.skin_based, "{:?}", closed_rc.grip);
    assert!(
        closed_rc.grip.palm_surface_m <= SKIN_PALM_GOOD_M,
        "closed palm_surface_m too large: {:?}",
        closed_rc.grip
    );
    assert!(
        closed_rc.grip.skin_wrap >= 0.75,
        "closed skin_wrap too low: {:?}",
        closed_rc.grip
    );
    assert!(
        closed_rc.grip.score >= 0.90,
        "closed five-finger grip must score >= 0.90: {:?}",
        closed_rc.grip
    );

    // JSON exposes skin_wrap, palm_surface_cm, skin_based deterministically.
    let j1 = closed_rc.to_json();
    let j2 = score_socket_clip(&closed_doc).to_json();
    assert_eq!(format!("{j1:?}"), format!("{j2:?}"));
    let g_json = j1.get("grip").unwrap();
    assert_eq!(g_json.get("skin_based").and_then(Json::as_bool), Some(true));
    assert!(g_json.get("skin_wrap").and_then(Json::as_f64).unwrap() >= 0.75);
    assert!(
        g_json
            .get("palm_surface_cm")
            .and_then(Json::as_f64)
            .unwrap()
            <= 4.5
    );

    // 2. Planted bad case A: open hand (fingers barely curled).
    let mut open_spec = spec.clone();
    open_spec.close = 0.01;
    let (open_doc, _) = build(&base, &open_spec, None).unwrap();
    let open_doc = Document::parse(&open_doc.to_bytes().unwrap()).unwrap();
    let open_rc = score_socket_clip(&open_doc);
    assert!(
        open_rc.grip.score < 0.30,
        "planted open hand must fail (< 0.30): {:?}",
        open_rc.grip
    );

    // 3. Planted bad case B: fingertip socket (~9 cm past the palm along -X).
    let rig = Rig::from_doc(&closed_doc).unwrap();
    let sock_r = rig.find("Socket_Hand_R").unwrap();
    let mut tip_doc = closed_doc.clone();
    let (st, sr, ss) = node_trs(&tip_doc.array("nodes")[sock_r]);
    set_node_trs(
        &mut tip_doc.array_mut("nodes")[sock_r],
        st + Vec3::new(-0.095, 0.0, 0.0),
        sr,
        ss,
    );
    let tip_rc = score_socket_clip(&tip_doc);
    assert!(
        tip_rc.grip.palm_surface_m > 0.075 && tip_rc.grip.score < 0.30,
        "planted fingertip socket must fail (< 0.30): {:?}",
        tip_rc.grip
    );

    // 4. Planted bad case C: handle floating 10 cm off the palm (-Y).
    let mut off_doc = closed_doc.clone();
    set_node_trs(
        &mut off_doc.array_mut("nodes")[sock_r],
        st + Vec3::new(0.0, -0.10, 0.0),
        sr,
        ss,
    );
    let off_rc = score_socket_clip(&off_doc);
    assert!(
        off_rc.grip.palm_surface_m > 0.085
            && off_rc.grip.skin_wrap < 0.15
            && off_rc.grip.score < 0.30,
        "planted +10 cm off handle must fail (< 0.30): {:?}",
        off_rc.grip
    );
}

#[test]
fn skin_grip_flat_palm_without_fingers_fails_on_circumference_wrap() {
    // A thin flat palm slab with no fingers: the handle rests 1.6 cm below the
    // palm surface (so bone/palm distance alone would pass), yet hand skin
    // covers less than half the handle's circumference -> must score < 0.30.
    let base = synthetic_hand_humanoid(false);
    let spec = parse_spec(preset("sweep").unwrap()).unwrap();
    let (doc, _) = build(&base, &spec, None).unwrap();
    let doc = Document::parse(&doc.to_bytes().unwrap()).unwrap();
    let rc = score_socket_clip(&doc);
    assert!(rc.grip.skin_based, "{:?}", rc.grip);
    assert!(
        rc.grip.palm_surface_m < 0.03,
        "handle should sit right on the flat palm: {:?}",
        rc.grip
    );
    assert!(
        rc.grip.skin_wrap < 0.50,
        "flat palm should cover < 50% circumference: {:?}",
        rc.grip
    );
    assert!(
        rc.grip.score < 0.30,
        "flat palm without wrap must fail (< 0.30): {:?}",
        rc.grip
    );
}

#[test]
fn skin_grip_distal_mitten_reads_palm_skin_not_bone_ratio() {
    // Distal-tip mitten rig (Hand -> Finger1 = 0.184 m, over 3x Finger1..3):
    // palm-pocket socket sits against the palm skin box while a fingertip
    // socket at Finger1 sits > 8 cm away from the palm skin.
    let raw = humanoid_mitten(Naming::Plain, false, 0.0).unwrap();
    let mut base = match standardize(&raw, "humanoid").unwrap() {
        Outcome::Done(d, _) => Document::parse(&d.to_bytes().unwrap()).unwrap(),
        Outcome::Refused(_, why) => panic!("{why}"),
    };
    let rig0 = Rig::from_doc(&base).unwrap();
    for (side, sign) in [("L", 1.0), ("R", -1.0)] {
        let f1 = rig0.find(&format!("DEF-f_middle.01.{side}")).unwrap();
        let f2 = rig0.find(&format!("DEF-f_middle.02.{side}")).unwrap();
        let f3 = rig0.find(&format!("DEF-f_middle.03.{side}")).unwrap();
        let (_, r1, s1) = node_trs(&base.array("nodes")[f1]);
        set_node_trs(
            &mut base.array_mut("nodes")[f1],
            Vec3::new(sign * 0.184, 0.0, 0.0),
            r1,
            s1,
        );
        let (_, r2, s2) = node_trs(&base.array("nodes")[f2]);
        set_node_trs(
            &mut base.array_mut("nodes")[f2],
            Vec3::new(sign * 0.024, 0.0, 0.0),
            r2,
            s2,
        );
        let (_, r3, s3) = node_trs(&base.array("nodes")[f3]);
        set_node_trs(
            &mut base.array_mut("nodes")[f3],
            Vec3::new(sign * 0.024, 0.0, 0.009),
            r3,
            s3,
        );
    }
    let spec = parse_spec(preset("sweep").unwrap()).unwrap();
    let (palm_doc, _) = build(&base, &spec, None).unwrap();
    let palm_doc = Document::parse(&palm_doc.to_bytes().unwrap()).unwrap();
    let palm_rc = score_socket_clip(&palm_doc);
    assert!(palm_rc.grip.skin_based, "{:?}", palm_rc.grip);
    assert!(
        palm_rc.grip.palm_surface_m < SKIN_PALM_GOOD_M,
        "palm pocket should sit near palm skin despite 18.4 cm bone offset: {:?}",
        palm_rc.grip
    );
    assert!(
        palm_rc.grip.skin_wrap > SKIN_WRAP_BAD,
        "palm pocket should engage hand skin past flat-palm wrap: {:?}",
        palm_rc.grip
    );

    // Floating the socket out at Finger1 (fingertips) fails palm surface distance.
    let rig_p = Rig::from_doc(&palm_doc).unwrap();
    let sock_r = rig_p.find("Socket_Hand_R").unwrap();
    let mut tip_doc = palm_doc.clone();
    let (_, sr, ss) = node_trs(&tip_doc.array("nodes")[sock_r]);
    set_node_trs(
        &mut tip_doc.array_mut("nodes")[sock_r],
        Vec3::new(-0.184, -0.026, 0.0),
        sr,
        ss,
    );
    let tip_rc = score_socket_clip(&tip_doc);
    assert!(
        tip_rc.grip.palm_surface_m > SKIN_PALM_BAD_M
            && tip_rc.grip.score < 0.30
            && palm_rc.grip.score > tip_rc.grip.score,
        "fingertip socket on distal mitten must fail (< 0.30) and score below palm pocket: tip={:?} palm={:?}",
        tip_rc.grip,
        palm_rc.grip
    );
}
