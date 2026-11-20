// Width-refit & extended contact gate (motion-13, docs/width-refit.md):
// Synthetic rigs and clips (no external meshes or owner assets):
//   - library rig with 0.30 m chest width (width_scale = 1.0)
//   - wide target rig 1.4x as wide (0.42 m chest width, 1.4x hips/thighs/shins)
//   - same-width target rig (width_scale = 1.0)
//
// Tests fail on main before motion-13 and pass once:
//   1. Body::build adds shin and upper_arm proxies;
//   2. animate's width-refit + extended contact pass clears torso, thighs, shins,
//      and arm-vs-arm contacts on the 1.4x target while preserving wrist
//      peak-speed frame, leaving frames > ramp away unchanged, and never moving legs;
//   3. arm-vs-arm contact moves the free hand first when one hand holds a weapon;
//   4. a target of the source's own width (and source-body faults via source slack)
//      comes out unchanged.

use motion_core::animate::{retarget_with, write_animation};
use motion_core::contact::{seg_seg, solve_arm, Body, Capsule, Options};
use motion_core::glb::{set_node_trs, Affine, Document};
use motion_core::json::Json;
use motion_core::math::{Quat, Vec3};
use motion_core::rig::{Animation, Rig, Trs};

struct BoneSpec {
    name: &'static str,
    parent: Option<&'static str>,
    head: [f64; 3],
    /// Half-extents `(hx, hz)` of the bone's skinned box (world X = side, Z = fwd,
    /// or perpendicular cross-section for horizontal arm bones).
    hx: f64,
    hz: f64,
}

/// Build a canonical `DEF-*` skinned humanoid GLB whose torso/hips/thighs/shins
/// scale horizontally by `width_scale` (at `1.0`: chest is 0.30 m wide, 0.18 m
/// deep; at `1.4`: chest is 0.42 m wide, 0.252 m deep). Pure synthetic geometry.
fn build_humanoid_with_width(width_scale: f64) -> Document {
    let ws = width_scale;
    // Chest scales by width_scale (0.30 m at 1.0 -> 0.42 m at 1.4). Shoulder
    // pivot widens slightly less than the bulky chest/hips so naive FK clips
    // cut into the wider chest and hips, while rest pose stays overlap-free.
    let chest_hw = 0.15 * ws;
    let shoulder_x = 0.175 + 0.1125 * (ws - 1.0);
    let hip_x = 0.09 * ws;
    let thigh_r = 0.060 * ws;
    let shin_r = 0.044 * ws;

    let mut specs: Vec<BoneSpec> = vec![
        BoneSpec {
            name: "root",
            parent: None,
            head: [0.0, 0.0, 0.0],
            hx: 0.0,
            hz: 0.0,
        },
        BoneSpec {
            name: "DEF-spine",
            parent: Some("root"),
            head: [0.0, 0.95, 0.0],
            hx: 0.13 * ws,
            hz: 0.088 * ws,
        },
        BoneSpec {
            name: "DEF-spine.001",
            parent: Some("DEF-spine"),
            head: [0.0, 1.08, 0.0],
            hx: 0.13 * ws,
            hz: 0.088 * ws,
        },
        BoneSpec {
            name: "DEF-spine.002",
            parent: Some("DEF-spine.001"),
            head: [0.0, 1.22, 0.0],
            hx: chest_hw,
            hz: 0.090 * ws,
        },
        BoneSpec {
            name: "DEF-spine.003",
            parent: Some("DEF-spine.002"),
            head: [0.0, 1.36, 0.0],
            hx: chest_hw,
            hz: 0.090 * ws,
        },
        BoneSpec {
            name: "DEF-spine.004",
            parent: Some("DEF-spine.003"),
            head: [0.0, 1.50, 0.0],
            hx: 0.045,
            hz: 0.045,
        },
        BoneSpec {
            name: "DEF-spine.006",
            parent: Some("DEF-spine.004"),
            head: [0.0, 1.60, 0.0],
            hx: 0.075,
            hz: 0.080,
        },
    ];
    for (s, dir) in [("L", 1.0), ("R", -1.0)] {
        let leak = |a: String| -> &'static str { Box::leak(a.into_boxed_str()) };
        let k = |b: &str| leak(format!("DEF-{b}.{s}"));
        specs.push(BoneSpec {
            name: k("shoulder"),
            parent: Some("DEF-spine.003"),
            head: [0.05 * ws * dir, 1.44, 0.0],
            hx: 0.032,
            hz: 0.032,
        });
        specs.push(BoneSpec {
            name: k("upper_arm"),
            parent: Some(k("shoulder")),
            head: [shoulder_x * dir, 1.44, 0.0],
            hx: 0.036,
            hz: 0.036,
        });
        specs.push(BoneSpec {
            name: k("forearm"),
            parent: Some(k("upper_arm")),
            head: [(shoulder_x + 0.26) * dir, 1.44, 0.0],
            hx: 0.028,
            hz: 0.028,
        });
        specs.push(BoneSpec {
            name: k("hand"),
            parent: Some(k("forearm")),
            head: [(shoulder_x + 0.51) * dir, 1.44, 0.0],
            hx: 0.024,
            hz: 0.024,
        });
        specs.push(BoneSpec {
            name: k("f_middle.01"),
            parent: Some(k("hand")),
            head: [(shoulder_x + 0.59) * dir, 1.44, 0.0],
            hx: 0.014,
            hz: 0.014,
        });
        specs.push(BoneSpec {
            name: k("thigh"),
            parent: Some("DEF-spine"),
            head: [hip_x * dir, 0.92, 0.0],
            hx: thigh_r,
            hz: thigh_r,
        });
        specs.push(BoneSpec {
            name: k("shin"),
            parent: Some(k("thigh")),
            head: [hip_x * dir, 0.50, 0.0],
            hx: shin_r,
            hz: shin_r,
        });
        specs.push(BoneSpec {
            name: k("foot"),
            parent: Some(k("shin")),
            head: [hip_x * dir, 0.08, 0.0],
            hx: 0.035,
            hz: 0.035,
        });
        specs.push(BoneSpec {
            name: k("toe"),
            parent: Some(k("foot")),
            head: [hip_x * dir, 0.02, 0.12],
            hx: 0.030,
            hz: 0.025,
        });
    }

    let idx = |name: &str| specs.iter().position(|s| s.name == name).unwrap();
    let n = specs.len();
    let mut doc = Document {
        json: Json::obj(vec![(
            "asset",
            Json::obj(vec![
                ("version", Json::str("2.0")),
                ("generator", Json::str("motionforge width_refit test")),
            ]),
        )]),
        bin: Vec::new(),
    };
    let mut world = vec![Affine::IDENTITY; n];
    let mut nodes = Vec::new();
    for (i, sp) in specs.iter().enumerate() {
        let want = Affine::from_trs(
            Vec3::new(sp.head[0], sp.head[1], sp.head[2]),
            Quat::IDENTITY,
            Vec3::new(1.0, 1.0, 1.0),
        );
        let pw = sp.parent.map(|p| world[idx(p)]).unwrap_or(Affine::IDENTITY);
        let local = pw.inverse().unwrap().mul(&want);
        world[i] = want;
        let (t, r, s) = local.decompose();
        let mut node = Json::obj(vec![("name", Json::str(sp.name))]);
        set_node_trs(&mut node, t, r, s);
        let kids: Vec<Json> = specs
            .iter()
            .enumerate()
            .filter(|(_, c)| c.parent == Some(sp.name))
            .map(|(k, _)| Json::num(k as f64))
            .collect();
        if !kids.is_empty() {
            node.set("children", Json::Arr(kids));
        }
        nodes.push(node);
    }

    // Closed rectangular prism per bone (with an extra mid-ring so lozenge &
    // radial percentiles have >= 12 vertices per bone).
    let mut positions: Vec<Vec<f64>> = Vec::new();
    let mut joints: Vec<[u16; 4]> = Vec::new();
    let mut weights: Vec<Vec<f64>> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for (i, sp) in specs.iter().enumerate() {
        if sp.name == "root" || sp.hx <= 0.0 {
            continue;
        }
        let a = Vec3::new(sp.head[0], sp.head[1], sp.head[2]);
        let child = specs
            .iter()
            .find(|c| c.parent == Some(sp.name))
            .map(|c| Vec3::new(c.head[0], c.head[1], c.head[2]));
        let b = child.unwrap_or_else(|| {
            if sp.name == "DEF-spine.006" {
                a + Vec3::new(0.0, 0.16, 0.0)
            } else {
                a + Vec3::new(0.0, 0.05, 0.0)
            }
        });
        // Keep shoulder/upper-arm box slightly inboard-trimmed so rest pose has
        // zero arm-vs-torso overlap.
        let a = if sp.name.starts_with("DEF-upper_arm") {
            a + (b - a).scale(0.12)
        } else {
            a
        };
        let d = (b - a).normalized();
        let (u, w) = if d.y.abs() > 0.7 {
            // Vertical bone (spine, head, thigh, shin): X = side, Z = front.
            (
                Vec3::new(1.0, 0.0, 0.0).scale(sp.hx),
                Vec3::new(0.0, 0.0, 1.0).scale(sp.hz),
            )
        } else {
            let up = Vec3::new(0.0, 1.0, 0.0);
            let u0 = d.cross(up);
            let u0 = if u0.length() > 1e-6 {
                u0.normalized()
            } else {
                Vec3::new(1.0, 0.0, 0.0)
            };
            let w0 = d.cross(u0).normalized();
            (u0.scale(sp.hx), w0.scale(sp.hz))
        };
        let base = positions.len() as u16;
        let ring_t = [0.0, 0.5, 1.0];
        for &rt in &ring_t {
            let p = a + (b - a).scale(rt);
            for (su, sw) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let v = p + u.scale(su) + w.scale(sw);
                positions.push(vec![v.x, v.y, v.z]);
                joints.push([i as u16, 0, 0, 0]);
                weights.push(vec![1.0, 0.0, 0.0, 0.0]);
            }
        }
        // Bottom cap (ring 0) and top cap (ring 2).
        let caps = [[0u16, 1, 2, 3], [8, 11, 10, 9]];
        for q in caps {
            for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                for k in tri {
                    indices.push(base + k);
                }
            }
        }
        // Side bands: ring 0->1 and ring 1->2.
        for band in 0u16..2 {
            let r0 = band * 4;
            let r1 = (band + 1) * 4;
            for side in 0u16..4 {
                let nxt = (side + 1) % 4;
                let q = [r0 + side, r1 + side, r1 + nxt, r0 + nxt];
                for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                    for k in tri {
                        indices.push(base + k);
                    }
                }
            }
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
            Json::Arr(vec![Json::num(0.0), Json::num(n as f64)]),
        )])]),
    );
    doc.json.set("scene", Json::num(0.0));
    Document::parse(&doc.to_bytes().unwrap()).unwrap()
}

fn smooth01(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Write `frames` of local transforms into `doc` as animation `name`.
fn attach_clip(
    doc: &mut Document,
    rig: &Rig,
    name: &str,
    times: &[f64],
    frames: &[Vec<Option<Trs>>],
) {
    let nodes: Vec<usize> = (0..rig.names.len())
        .filter(|&i| rig.names[i].starts_with("DEF-"))
        .collect();
    let mut tracks: Vec<Vec<Trs>> = vec![Vec::with_capacity(frames.len()); nodes.len()];
    for fr in frames {
        for (k, &node) in nodes.iter().enumerate() {
            tracks[k].push(fr[node].unwrap_or(rig.rest[node]));
        }
    }
    let hips = rig.find("DEF-spine");
    write_animation(doc, name, times, &nodes, &tracks, hips).unwrap();
}

/// Sample all frames of clip 0 in `doc` at `times`.
fn sample_frames(doc: &Document, rig: &Rig, times: &[f64]) -> Vec<Vec<Option<Trs>>> {
    let anims = Animation::load_all(doc).unwrap();
    assert!(!anims.is_empty(), "expected at least one animation");
    let a = &anims[0];
    times.iter().map(|&t| a.sample(rig, t)).collect()
}

/// Capsule-to-capsule penetration depth (> 0 means overlap).
fn cap_penetration(c1: &Capsule, c2: &Capsule, world: &[Affine]) -> f64 {
    let (p1, q1) = (world[c1.bone].apply(c1.a), world[c1.bone].apply(c1.b));
    let (p2, q2) = (world[c2.bone].apply(c2.a), world[c2.bone].apply(c2.b));
    let (_, _, d) = seg_seg(p1, q1, p2, q2);
    c1.r + c2.r - d
}

/// Index of the fastest-moving frame for `bone` (largest displacement from f-1 to f).
fn fastest_frame(rig: &Rig, frames: &[Vec<Option<Trs>>], bone: usize) -> usize {
    let mut best_f = 1usize;
    let mut best_d = -1.0f64;
    for f in 1..frames.len() {
        let p0 = rig.world(&frames[f - 1])[bone].t;
        let p1 = rig.world(&frames[f])[bone].t;
        let d = (p1 - p0).length();
        if d > best_d {
            best_d = d;
            best_f = f;
        }
    }
    best_f
}

/// Build the library clip specified in docs/width-refit.md:
///   - frames 0..=3 & 42..=45: arms out wide (well beyond 0.15 s ramp from any contact)
///   - frames 9..=21: both hands cross in front of the 0.30 m chest with a sharp
///     velocity peak on entry so each wrist has a single unambiguous fastest frame;
///     clean on the 0.30 m library rig, cuts into the 1.4x chest + arms cross on 1.4x
///   - frames 26..=36: right hand sweeps tightly beside the hip at thigh height
///     (clean on the 0.30 m library rig, cuts into the 1.4x hip/thigh)
fn build_cross_and_hip_source() -> (Document, Vec<f64>) {
    let mut src = build_humanoid_with_width(1.0);
    let rig = Rig::from_doc(&src).unwrap();
    let find = |n: &str| rig.find(n).unwrap();
    let (ul, fl, hl) = (
        find("DEF-upper_arm.L"),
        find("DEF-forearm.L"),
        find("DEF-hand.L"),
    );
    let (ur, fr, hr) = (
        find("DEF-upper_arm.R"),
        find("DEF-forearm.R"),
        find("DEF-hand.R"),
    );
    let n_frames = 46usize; // 0.0 .. 1.50 s at 30 fps
    let times: Vec<f64> = (0..n_frames).map(|k| k as f64 / 30.0).collect();
    let rest = vec![None; rig.names.len()];
    let w_l_wide = Vec3::new(0.62, 1.44, 0.03);
    let w_r_wide = Vec3::new(-0.62, 1.44, 0.03);
    // Chest-crossing targets: clean on 0.30 m chest (z_max = 0.090) and clear
    // of each other (dz = 0.076 > 2 * 0.034), while cutting deep into 1.4x chest
    // so pushing L forward out of the 1.4x chest drives L into R unless
    // width-refit + arm-vs-arm contact runs.
    let w_l_cross = Vec3::new(-0.04, 1.28, 0.140);
    let w_r_cross = Vec3::new(0.04, 1.26, 0.226);
    // Hip/thigh-height pass: straight down beside the hip at thigh height,
    // clear of 1.0x thigh + spine + margin (x = -0.208), cutting into the 1.4x
    // thigh/hip proxy under naive FK.
    let w_r_hip = Vec3::new(-0.208, 0.90, 0.02);

    let mut frames = Vec::with_capacity(n_frames);
    for (k, &t) in times.iter().enumerate() {
        let mut locals = rest.clone();
        // Fast crisp entry into chest cross over frames 10..=12 (t = 0.333..0.400)
        // so frame 11 is the unique fastest frame for BOTH wrists; gentle return.
        let cross_l = if t < 0.33 {
            0.0
        } else if t <= 0.40 {
            smooth01((t - 0.33) / 0.07)
        } else if t <= 0.56 {
            1.0
        } else if t <= 0.74 {
            1.0 - smooth01((t - 0.56) / 0.18)
        } else {
            0.0
        };
        let cross_r = cross_l;

        // Hip pass over t = 0.85..1.20 s (slower than the chest-cross entry so
        // it never competes for the fastest frame).
        let hip_r = if t < 0.85 || t > 1.20 {
            0.0
        } else if t <= 0.98 {
            smooth01((t - 0.85) / 0.13)
        } else if t <= 1.07 {
            1.0
        } else {
            1.0 - smooth01((t - 1.07) / 0.13)
        };

        let target_l = w_l_wide + (w_l_cross - w_l_wide).scale(cross_l);
        let target_r = if hip_r > 0.0 {
            w_r_wide + (w_r_hip - w_r_wide).scale(hip_r)
        } else {
            w_r_wide + (w_r_cross - w_r_wide).scale(cross_r)
        };
        let _ = k;
        solve_arm(&rig, &mut locals, ul, fl, hl, target_l);
        solve_arm(&rig, &mut locals, ur, fr, hr, target_r);
        frames.push(locals);
    }
    attach_clip(&mut src, &rig, "cross_hip", &times, &frames);
    (Document::parse(&src.to_bytes().unwrap()).unwrap(), times)
}

#[test]
fn body_proxies_include_shins_and_upper_arms() {
    let doc = build_humanoid_with_width(1.4);
    let rig = Rig::from_doc(&doc).unwrap();
    let opts = Options {
        weapon_side: None,
        ..Options::default()
    };
    let body = Body::build(&doc, &rig, &opts).unwrap().expect("proxies");
    let json = body.to_json(&rig);
    let names: Vec<String> = json
        .get("capsules")
        .and_then(Json::as_arr)
        .unwrap()
        .iter()
        .filter_map(|c| c.get("name").and_then(Json::as_str).map(|s| s.to_string()))
        .collect();
    for want in ["shin.L", "shin.R", "upper_arm.L", "upper_arm.R"] {
        assert!(
            names.iter().any(|n| n == want),
            "Body proxies must include {want}; got {names:?}"
        );
    }
}

#[test]
fn wide_target_clears_torso_legs_and_arm_crossing_while_keeping_timing_and_ramp() {
    let (src, times) = build_cross_and_hip_source();
    let tgt_wide = build_humanoid_with_width(1.4);
    let opts = Options {
        weapon_side: None,
        ..Options::default()
    };

    // Baseline: pure FK retarget (no contact / no width refit).
    let (raw_doc, _) = retarget_with(
        &tgt_wide,
        &[("lib.glb".into(), src.clone())],
        30.0,
        None,
        None,
        false,
    )
    .unwrap();
    // Refitted: width refit + extended contact pass.
    let (fit_doc, _) = retarget_with(
        &tgt_wide,
        &[("lib.glb".into(), src.clone())],
        30.0,
        None,
        Some(&opts),
        false,
    )
    .unwrap();

    let src_rig = Rig::from_doc(&src).unwrap();
    let src_body = Body::build(&src, &src_rig, &opts).unwrap().unwrap();
    let src_frames = sample_frames(&src, &src_rig, &times);

    let tgt_rig = Rig::from_doc(&fit_doc).unwrap();
    let tgt_body = Body::build(&fit_doc, &tgt_rig, &opts).unwrap().unwrap();
    let raw_frames = sample_frames(&raw_doc, &tgt_rig, &times);
    let fit_frames = sample_frames(&fit_doc, &tgt_rig, &times);

    // Sanity: the clip is genuinely clean on the 0.30 m source rig.
    for (f, sfr) in src_frames.iter().enumerate() {
        let sw = src_rig.world(sfr);
        for arm in &src_body.arms {
            for s in &src_body.static_parts {
                assert!(
                    cap_penetration(&arm.forearm, s, &sw) <= 0.004,
                    "source fixture must be clean: frame {f} {} vs {}",
                    arm.forearm.name,
                    s.name
                );
                assert!(
                    cap_penetration(&arm.palm, s, &sw) <= 0.004,
                    "source fixture must be clean: frame {f} {} vs {}",
                    arm.palm.name,
                    s.name
                );
            }
        }
        let pen_arms = cap_penetration(&src_body.arms[0].forearm, &src_body.arms[1].forearm, &sw);
        assert!(
            pen_arms <= 0.0,
            "source forearms must not collide (frame {f}: {pen_arms:.4} m)"
        );
    }

    // 1. After refit: no hand or forearm capsule inside any target torso or leg proxy.
    for (f, fr) in fit_frames.iter().enumerate() {
        let w = tgt_rig.world(fr);
        for arm in &tgt_body.arms {
            for s in &tgt_body.static_parts {
                let d_fo = cap_penetration(&arm.forearm, s, &w);
                let d_ha = cap_penetration(&arm.palm, s, &w);
                assert!(
                    d_fo <= 0.002,
                    "frame {f}: {} still penetrates {} by {:.1} mm",
                    arm.forearm.name,
                    s.name,
                    d_fo * 1000.0
                );
                assert!(
                    d_ha <= 0.002,
                    "frame {f}: {} still penetrates {} by {:.1} mm",
                    arm.palm.name,
                    s.name,
                    d_ha * 1000.0
                );
            }
        }
        // 2. Forearm capsules don't meet on any frame where the source's didn't.
        let sw = src_rig.world(&src_frames[f]);
        let src_arm_pen =
            cap_penetration(&src_body.arms[0].forearm, &src_body.arms[1].forearm, &sw);
        if src_arm_pen <= 0.0 {
            let tgt_arm_pen =
                cap_penetration(&tgt_body.arms[0].forearm, &tgt_body.arms[1].forearm, &w);
            assert!(
                tgt_arm_pen <= 0.002,
                "frame {f}: target forearms collide by {:.1} mm where source was clear",
                tgt_arm_pen * 1000.0
            );
        }
    }

    // 3. Each wrist's fastest frame is the same frame as the source's (timing kept).
    for side in ["L", "R"] {
        let sh = src_rig.find(&format!("DEF-hand.{side}")).unwrap();
        let th = tgt_rig.find(&format!("DEF-hand.{side}")).unwrap();
        let sf = fastest_frame(&src_rig, &src_frames, sh);
        let tf = fastest_frame(&tgt_rig, &fit_frames, th);
        assert_eq!(
            tf, sf,
            "wrist {side} fastest frame shifted from {sf} to {tf} (timing altered)"
        );
    }

    // 4. Frames more than the ramp (0.15 s) away from any contact/envelope are unchanged.
    for f in [0usize, 1, 2, 43, 44, 45] {
        let w_raw = tgt_rig.world(&raw_frames[f]);
        let w_fit = tgt_rig.world(&fit_frames[f]);
        for side in ["L", "R"] {
            let h = tgt_rig.find(&format!("DEF-hand.{side}")).unwrap();
            let diff = (w_fit[h].t - w_raw[h].t).length();
            assert!(
                diff < 1e-5,
                "frame {f} ({side}) is > ramp away from contact but wrist shifted by {:.3} mm",
                diff * 1000.0
            );
        }
    }
}

#[test]
fn arm_vs_shin_clears_without_moving_leg() {
    // Source bends left thigh & shin forward (crouch/dodge pose) and sweeps the
    // left hand just outside the 1.0x shin. On the 1.4x target the wider shin
    // catches the hand/forearm; refit must clear the shin without moving the leg.
    let mut src = build_humanoid_with_width(1.0);
    let rig = Rig::from_doc(&src).unwrap();
    let find = |n: &str| rig.find(n).unwrap();
    let (ul, fl, hl) = (
        find("DEF-upper_arm.L"),
        find("DEF-forearm.L"),
        find("DEF-hand.L"),
    );
    let (th_l, sh_l) = (find("DEF-thigh.L"), find("DEF-shin.L"));
    let n_frames = 31usize;
    let times: Vec<f64> = (0..n_frames).map(|k| k as f64 / 30.0).collect();

    // Base crouched-leg pose: lift left thigh forward -70 deg about +X, bend shin
    // +50 deg about +X so the left shin sits around y = 0.85..1.10, z = +0.35.
    let mut crouch = vec![None; rig.names.len()];
    let mut t_trs = rig.rest[th_l];
    t_trs.r = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -70f64.to_radians());
    crouch[th_l] = Some(t_trs);
    let mut s_trs = rig.rest[sh_l];
    s_trs.r = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 50f64.to_radians());
    crouch[sh_l] = Some(s_trs);

    let cw = rig.world(&crouch);
    let knee = cw[sh_l].t;
    let ankle = cw[find("DEF-foot.L")].t;
    let shin_mid = (knee + ankle).scale(0.5);
    // Clear of 1.0x shin (r_cap = 0.044*0.9 = 0.040, hand r = 0.020 -> sum 0.060)
    // at +0.072 m outboard in X; on 1.4x target shin is shifted +0.036 m in X
    // and thicker (r_cap = 0.055), so the hand lands straight inside shin.L!
    let w_far = Vec3::new(0.60, 1.44, 0.05);
    let w_near = Vec3::new(shin_mid.x + 0.072, shin_mid.y, shin_mid.z);

    let mut frames = Vec::with_capacity(n_frames);
    for &t in &times {
        let mut locals = crouch.clone();
        let s = if t < 0.30 || t > 0.70 {
            0.0
        } else if t <= 0.45 {
            smooth01((t - 0.30) / 0.15)
        } else if t <= 0.55 {
            1.0
        } else {
            1.0 - smooth01((t - 0.55) / 0.15)
        };
        solve_arm(
            &rig,
            &mut locals,
            ul,
            fl,
            hl,
            w_far + (w_near - w_far).scale(s),
        );
        frames.push(locals);
    }
    attach_clip(&mut src, &rig, "dodge_shin", &times, &frames);
    let src = Document::parse(&src.to_bytes().unwrap()).unwrap();

    let tgt_wide = build_humanoid_with_width(1.4);
    let opts = Options {
        weapon_side: None,
        ..Options::default()
    };
    let (raw_doc, _) = retarget_with(
        &tgt_wide,
        &[("lib.glb".into(), src.clone())],
        30.0,
        None,
        None,
        false,
    )
    .unwrap();
    let (fit_doc, _) = retarget_with(
        &tgt_wide,
        &[("lib.glb".into(), src)],
        30.0,
        None,
        Some(&opts),
        false,
    )
    .unwrap();

    let tgt_rig = Rig::from_doc(&fit_doc).unwrap();
    let tgt_body = Body::build(&fit_doc, &tgt_rig, &opts).unwrap().unwrap();
    let raw_frames = sample_frames(&raw_doc, &tgt_rig, &times);
    let fit_frames = sample_frames(&fit_doc, &tgt_rig, &times);

    let shin_l_cap = tgt_body
        .static_parts
        .iter()
        .find(|c| c.name == "shin.L")
        .expect("Body must have a shin.L proxy");
    let arm_l = tgt_body.arms.iter().find(|a| a.side == "L").unwrap();

    // Confirm raw FK really penetrated shin.L on the wide target.
    let raw_max_pen = raw_frames
        .iter()
        .map(|fr| {
            let w = tgt_rig.world(fr);
            cap_penetration(&arm_l.palm, shin_l_cap, &w).max(cap_penetration(
                &arm_l.forearm,
                shin_l_cap,
                &w,
            ))
        })
        .fold(0.0f64, f64::max);
    assert!(
        raw_max_pen > 0.005,
        "test setup must hit shin.L on raw FK (got {:.1} mm)",
        raw_max_pen * 1000.0
    );

    // After refit: arm clears shin.L on every frame, and leg joints never move.
    let tgt_th_l = tgt_rig.find("DEF-thigh.L").unwrap();
    let tgt_sh_l = tgt_rig.find("DEF-shin.L").unwrap();
    for (f, fr) in fit_frames.iter().enumerate() {
        let w = tgt_rig.world(fr);
        let pen = cap_penetration(&arm_l.palm, shin_l_cap, &w).max(cap_penetration(
            &arm_l.forearm,
            shin_l_cap,
            &w,
        ));
        assert!(
            pen <= 0.002,
            "frame {f}: left arm still penetrates shin.L by {:.1} mm",
            pen * 1000.0
        );
        for leg in [tgt_th_l, tgt_sh_l] {
            let r_raw = raw_frames[f][leg].unwrap_or(tgt_rig.rest[leg]).r;
            let r_fit = fr[leg].unwrap_or(tgt_rig.rest[leg]).r;
            assert!(
                r_raw.angle_to(r_fit) < 1e-5,
                "frame {f}: leg joint {} moved by {:.4} deg (legs must never move)",
                tgt_rig.names[leg],
                r_raw.angle_to(r_fit).to_degrees()
            );
        }
    }
}

#[test]
fn arm_vs_arm_weapon_moves_free_hand_first() {
    // Isolate arm-vs-arm contact well in front of the torso (z = +0.29 m, clear
    // of the chest) on a clip named "attack_cross" where R holds the weapon and
    // L is free. Both forearms collide in front of the chest; the free hand (L)
    // must take the lead in clearing so the weapon hand's blade line stays steady.
    let doc = build_humanoid_with_width(1.0);
    let rig = Rig::from_doc(&doc).unwrap();
    let find = |n: &str| rig.find(n).unwrap();
    let (ul, fl, hl) = (
        find("DEF-upper_arm.L"),
        find("DEF-forearm.L"),
        find("DEF-hand.L"),
    );
    let (ur, fr, hr) = (
        find("DEF-upper_arm.R"),
        find("DEF-forearm.R"),
        find("DEF-hand.R"),
    );
    let opts = Options {
        weapon_side: Some("R".into()),
        ..Options::default()
    };
    let body = Body::build(&doc, &rig, &opts).unwrap().unwrap();
    let n_frames = 21usize;
    let times: Vec<f64> = (0..n_frames).map(|k| k as f64 / 30.0).collect();
    let rest = vec![None; rig.names.len()];
    let w_l_wide = Vec3::new(0.55, 1.38, 0.10);
    let w_r_wide = Vec3::new(-0.55, 1.38, 0.10);
    // Both wrists cross at virtually identical (y, z) in front of the torso so
    // the two forearms intersect each other cleanly away from the chest.
    let w_l_hit = Vec3::new(-0.08, 1.30, 0.30);
    let w_r_hit = Vec3::new(0.08, 1.30, 0.305);

    let mut frames = Vec::with_capacity(n_frames);
    for &t in &times {
        let mut locals = rest.clone();
        let s = if t < 0.15 || t > 0.55 {
            0.0
        } else if t <= 0.30 {
            smooth01((t - 0.15) / 0.15)
        } else if t <= 0.40 {
            1.0
        } else {
            1.0 - smooth01((t - 0.40) / 0.15)
        };
        solve_arm(
            &rig,
            &mut locals,
            ul,
            fl,
            hl,
            w_l_wide + (w_l_hit - w_l_wide).scale(s),
        );
        solve_arm(
            &rig,
            &mut locals,
            ur,
            fr,
            hr,
            w_r_wide + (w_r_hit - w_r_wide).scale(s),
        );
        frames.push(locals);
    }
    let original = frames.clone();
    let _report =
        motion_core::contact::fix_clip(&body, &rig, &mut frames, &times, "attack_cross", &opts);

    // Forearms must be separated after fix_clip.
    let arm_l = body.arms.iter().find(|a| a.side == "L").unwrap();
    let arm_r = body.arms.iter().find(|a| a.side == "R").unwrap();
    for (f, fr) in frames.iter().enumerate() {
        let w = rig.world(fr);
        let pen = cap_penetration(&arm_l.forearm, &arm_r.forearm, &w);
        assert!(
            pen <= 0.002,
            "frame {f}: forearms still collide by {:.1} mm after arm-vs-arm contact",
            pen * 1000.0
        );
    }

    // Free hand (L) moved strictly more than the weapon hand (R).
    let max_shift = |hand: usize| -> f64 {
        frames
            .iter()
            .zip(&original)
            .map(|(a, b)| (rig.world(a)[hand].t - rig.world(b)[hand].t).length())
            .fold(0.0f64, f64::max)
    };
    let shift_l = max_shift(hl);
    let shift_r = max_shift(hr);
    assert!(
        shift_l > 0.01 && shift_l > shift_r * 1.5,
        "free hand L must move first when R holds weapon: shift_l={:.1} mm, shift_r={:.1} mm",
        shift_l * 1000.0,
        shift_r * 1000.0
    );
}

#[test]
fn same_width_target_and_source_fault_stay_unchanged() {
    let opts = Options {
        weapon_side: None,
        ..Options::default()
    };

    // Part A: clean clip on a target of the source's own width (1.0x) is unchanged.
    let (src, times) = build_cross_and_hip_source();
    let tgt_same = build_humanoid_with_width(1.0);
    let (raw_doc, _) = retarget_with(
        &tgt_same,
        &[("lib.glb".into(), src.clone())],
        30.0,
        None,
        None,
        false,
    )
    .unwrap();
    let (fit_doc, _) = retarget_with(
        &tgt_same,
        &[("lib.glb".into(), src)],
        30.0,
        None,
        Some(&opts),
        false,
    )
    .unwrap();
    let rig = Rig::from_doc(&fit_doc).unwrap();
    let raw_frames = sample_frames(&raw_doc, &rig, &times);
    let fit_frames = sample_frames(&fit_doc, &rig, &times);
    for (f, (r, a)) in raw_frames.iter().zip(&fit_frames).enumerate() {
        let wr = rig.world(r);
        let wa = rig.world(a);
        for side in ["L", "R"] {
            let h = rig.find(&format!("DEF-hand.{side}")).unwrap();
            let d = (wa[h].t - wr[h].t).length();
            assert!(
                d < 1e-5,
                "same-width clean clip frame {f} ({side}) drifted by {:.3} mm",
                d * 1000.0
            );
        }
    }

    // Part B: source-body slack — a clip where the source body ITSELF pushes
    // its left hand into its own chest must keep that depth as slack on a
    // same-width target (clip's own fault is not the body's).
    let mut fault_src = build_humanoid_with_width(1.0);
    let srig = Rig::from_doc(&fault_src).unwrap();
    let find = |n: &str| srig.find(n).unwrap();
    let (ul, fl, hl) = (
        find("DEF-upper_arm.L"),
        find("DEF-forearm.L"),
        find("DEF-hand.L"),
    );
    let ftimes: Vec<f64> = (0..15).map(|k| k as f64 / 30.0).collect();
    let rest = vec![None; srig.names.len()];
    let w_far = Vec3::new(0.60, 1.44, 0.02);
    let w_in_chest = Vec3::new(0.02, 1.28, 0.02); // deep inside source chest!
    let mut fframes = Vec::new();
    for &t in &ftimes {
        let mut locals = rest.clone();
        let s = if t < 0.10 || t > 0.37 {
            0.0
        } else {
            smooth01(1.0 - ((t - 0.233).abs() / 0.133))
        };
        solve_arm(
            &srig,
            &mut locals,
            ul,
            fl,
            hl,
            w_far + (w_in_chest - w_far).scale(s),
        );
        fframes.push(locals);
    }
    attach_clip(&mut fault_src, &srig, "source_fault", &ftimes, &fframes);
    let fault_src = Document::parse(&fault_src.to_bytes().unwrap()).unwrap();

    let (raw_fault, _) = retarget_with(
        &tgt_same,
        &[("lib.glb".into(), fault_src.clone())],
        30.0,
        None,
        None,
        false,
    )
    .unwrap();
    let (fit_fault, _) = retarget_with(
        &tgt_same,
        &[("lib.glb".into(), fault_src)],
        30.0,
        None,
        Some(&opts),
        false,
    )
    .unwrap();
    let rf = sample_frames(&raw_fault, &rig, &ftimes);
    let ff = sample_frames(&fit_fault, &rig, &ftimes);
    let hl_t = rig.find("DEF-hand.L").unwrap();
    for (f, (r, a)) in rf.iter().zip(&ff).enumerate() {
        let d = (rig.world(a)[hl_t].t - rig.world(r)[hl_t].t).length();
        assert!(
            d < 1e-4,
            "source-fault clip frame {f} was altered by {:.2} mm despite same-width source slack",
            d * 1000.0
        );
    }
}
