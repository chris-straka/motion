// Known-answer gate tests for motion-7 (Giant pass: Froude time scaling,
// root travel x k, foot plants & contacts preserved, anticipation + settle +
// hit-stop distinct from plain slowdown, GLB extras events, and arms-only /
// pinned-pelvis body mask).

use motion_core::attack::{build, parse_spec, preset};
use motion_core::clean::{clean_glb, CleanOptions};
use motion_core::contact::Options as ContactOptions;
use motion_core::fixture::{humanoid, Naming};
use motion_core::giant::{giant_glb, BodyMask, GiantOptions};
use motion_core::glb::Document;
use motion_core::humanoid::map_rig;
use motion_core::math::Vec3;
use motion_core::motioncheck::{check_doc, sample};
use motion_core::rig::{Animation, Rig};
use motion_core::standardize::{standardize, Outcome};

fn standardized_body(walk: bool) -> Document {
    let body = humanoid(Naming::Plain, false, walk).unwrap();
    match standardize(&body, "humanoid").unwrap() {
        Outcome::Done(d, _) => Document::parse(&d.to_bytes().unwrap()).unwrap(),
        Outcome::Refused(_, why) => panic!("standardize failed: {why}"),
    }
}

fn grounded_walk_doc() -> Document {
    let raw = standardized_body(true);
    let (cleaned, _) = clean_glb(&raw, &CleanOptions::default(), &[], None).unwrap();
    Document::parse(&cleaned.to_bytes().unwrap()).unwrap()
}

fn cleave_attack_doc() -> Document {
    let body = standardized_body(false);
    let spec = parse_spec(preset("cleave").unwrap()).unwrap();
    let (attack_doc, _) = build(&body, &spec, Some("cleave")).unwrap();
    Document::parse(&attack_doc.to_bytes().unwrap()).unwrap()
}

fn clip_duration_and_root_travel(doc: &Document, clip_name: &str) -> (f64, f64, f64) {
    let rig = Rig::from_doc(doc).unwrap();
    let map = map_rig(&rig);
    let hips = map.node_of("DEF-spine").expect("hips node");
    let anims = Animation::load_all(doc).unwrap();
    let anim = anims
        .iter()
        .find(|a| a.name == clip_name)
        .unwrap_or_else(|| panic!("missing clip {clip_name}"));
    let (times, frames) = sample(&rig, anim, Some(30.0));
    let dur = *times.last().unwrap_or(&0.0) - *times.first().unwrap_or(&0.0);
    let first = rig.world(&frames[0])[hips].t;
    let last = rig.world(frames.last().unwrap())[hips].t;
    let travel = Vec3::new(last.x - first.x, 0.0, last.z - first.z).length();
    let (subj, _) = check_doc(doc, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
    (dur, travel, subj.height)
}

fn sampled_node_positions(
    doc: &Document,
    clip_name: &str,
    canonical: &str,
) -> (Vec<f64>, Vec<Vec3>) {
    let rig = Rig::from_doc(doc).unwrap();
    let map = map_rig(&rig);
    let node = map
        .node_of(canonical)
        .unwrap_or_else(|| panic!("missing bone {canonical}"));
    let anims = Animation::load_all(doc).unwrap();
    let anim = anims
        .iter()
        .find(|a| a.name == clip_name)
        .unwrap_or_else(|| panic!("missing clip {clip_name}"));
    let (times, frames) = sample(&rig, anim, Some(30.0));
    let pts = frames.iter().map(|f| rig.world(f)[node].t).collect();
    (times, pts)
}

#[test]
fn scale_4_doubles_duration_quadruples_root_travel_and_keeps_contacts() {
    let src = grounded_walk_doc();
    let (src_dur, src_travel, src_h) = clip_duration_and_root_travel(&src, "walk");
    assert!(src_dur > 0.5, "source walk duration too short: {src_dur}");
    assert!(
        src_travel > 0.5,
        "source walk must have real root travel: {src_travel}"
    );
    let (_, src_scores) =
        check_doc(&src, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
    let src_score = &src_scores[0];

    let opts = GiantOptions {
        scale: 4.0,
        ..GiantOptions::default()
    };
    let (out_doc, reports) = giant_glb(&src, &opts, &[], None).unwrap();
    assert_eq!(reports.len(), 1);
    let out = Document::parse(&out_doc.to_bytes().unwrap()).unwrap();
    let (out_dur, out_travel, out_h) = clip_duration_and_root_travel(&out, "walk");

    // k = 4 -> Froude time = sqrt(4) = 2x duration (within 1 frame / 5%).
    let dur_ratio = out_dur / src_dur;
    assert!(
        (dur_ratio - 2.0).abs() < 0.06,
        "k=4 must run twice as long: src={src_dur:.3}s out={out_dur:.3}s ratio={dur_ratio:.3}"
    );
    // k = 4 -> root travel = 4x (within 5%).
    let travel_ratio = out_travel / src_travel;
    assert!(
        (travel_ratio - 4.0).abs() < 0.20,
        "k=4 must quadruple root travel: src={src_travel:.3}m out={out_travel:.3}m ratio={travel_ratio:.3}"
    );
    // Rig height scales by 4x.
    let height_ratio = out_h / src_h;
    assert!(
        (height_ratio - 4.0).abs() < 0.20,
        "k=4 must scale rig height 4x: src={src_h:.3}m out={out_h:.3}m ratio={height_ratio:.3}"
    );

    // Contacts unchanged / not worsened on the giant walk.
    let (_, out_scores) =
        check_doc(&out, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
    let out_score = &out_scores[0];
    assert!(
        out_score.feet.sink_frames <= src_score.feet.sink_frames + 2,
        "giant walk introduced floor sink: src={} out={}",
        src_score.feet.sink_frames,
        out_score.feet.sink_frames
    );
    assert!(
        out_score.feet.skate_m <= src_score.feet.skate_m + 0.03,
        "giant walk worsened scaled foot skate: src={:.3}m out={:.3}m",
        src_score.feet.skate_m,
        out_score.feet.skate_m
    );
    let src_through = src_score.through.as_ref().map_or(0, |t| t.mesh_frames);
    let out_through = out_score.through.as_ref().map_or(0, |t| t.mesh_frames);
    assert!(
        out_through <= src_through + 2,
        "giant walk worsened through-body contacts: src={src_through} out={out_through}"
    );
}

#[test]
fn planted_attack_keeps_plants_and_differs_from_plain_slowdown() {
    let src = cleave_attack_doc();
    let (src_dur, _, src_h) = clip_duration_and_root_travel(&src, "cleave");
    let (_, src_scores) =
        check_doc(&src, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
    let src_score = &src_scores[0];
    assert!(
        src_score.feet.plants >= 1,
        "source cleave must have planted feet"
    );

    let weighted_opts = GiantOptions {
        scale: 4.0,
        anticipation: 0.35,
        settle: 0.35,
        hit_stop_s: 0.12,
        cancel_window_s: 0.25,
        ..GiantOptions::default()
    };
    let plain_opts = GiantOptions {
        scale: 4.0,
        anticipation: 0.0,
        settle: 0.0,
        hit_stop_s: 0.0,
        cancel_window_s: 0.0,
        ..GiantOptions::default()
    };

    let (weighted_doc, reports) = giant_glb(&src, &weighted_opts, &[], None).unwrap();
    let (plain_doc, _) = giant_glb(&src, &plain_opts, &[], None).unwrap();
    let weighted = Document::parse(&weighted_doc.to_bytes().unwrap()).unwrap();
    let plain = Document::parse(&plain_doc.to_bytes().unwrap()).unwrap();

    // Both scale duration by sqrt(4) = 2x.
    let (w_dur, _, _) = clip_duration_and_root_travel(&weighted, "cleave");
    let (p_dur, _, _) = clip_duration_and_root_travel(&plain, "cleave");
    assert!(
        (w_dur / src_dur - 2.0).abs() < 0.08,
        "weighted giant duration ratio off: {w_dur} vs {src_dur}"
    );
    assert!(
        (p_dur / src_dur - 2.0).abs() < 0.06,
        "plain slowdown duration ratio off: {p_dur} vs {src_dur}"
    );

    // Foot plants kept and contacts not broken on the weighted giant attack.
    let (_, w_scores) =
        check_doc(&weighted, &ContactOptions::default(), &[], &[], Some(30.0)).unwrap();
    let w_score = &w_scores[0];
    assert!(
        w_score.feet.plants >= 1,
        "giant attack lost foot plants: src={} out={}",
        src_score.feet.plants,
        w_score.feet.plants
    );
    assert!(
        w_score.feet.skate_m <= src_score.feet.skate_m + 0.03,
        "giant attack skated feet: src={:.3}m out={:.3}m",
        src_score.feet.skate_m,
        w_score.feet.skate_m
    );
    assert!(
        w_score.feet.sink_frames <= src_score.feet.sink_frames + 2,
        "giant attack sank feet: src={} out={}",
        src_score.feet.sink_frames,
        w_score.feet.sink_frames
    );

    // Weighted giant pass must differ measurably from plain uniform slow-down.
    let (w_times, w_hand) = sampled_node_positions(&weighted, "cleave", "DEF-hand.R");
    let (_, p_hand) = sampled_node_positions(&plain, "cleave", "DEF-hand.R");
    let n = w_hand.len().min(p_hand.len());
    let max_diff = (0..n)
        .map(|i| (w_hand[i] - p_hand[i]).length())
        .fold(0.0_f64, f64::max);
    assert!(
        max_diff > 0.05 * src_h * 4.0,
        "giant pass with anticipation+settle+hit-stop must differ visibly from plain slowdown (max_diff={max_diff:.3}m)"
    );

    // During hit-stop window, the sword hand must pause steadily.
    let rep = &reports[0];
    let (hs_start, hs_end) = rep.hit_stop.expect("attack giant must report hit_stop");
    assert!(
        hs_end - hs_start >= 0.08,
        "hit_stop window too short: ({hs_start}, {hs_end})"
    );
    let inside: Vec<usize> = w_times
        .iter()
        .enumerate()
        .filter(|(_, &t)| t >= hs_start + 1e-4 && t <= hs_end - 1e-4)
        .map(|(i, _)| i)
        .collect();
    assert!(
        inside.len() >= 2,
        "expected >=2 sampled frames inside hit_stop ({hs_start:.3}..{hs_end:.3}), got {}",
        inside.len()
    );
    let mut max_step = 0.0_f64;
    for pair in inside.windows(2) {
        let step = (w_hand[pair[1]] - w_hand[pair[0]]).length();
        if step > max_step {
            max_step = step;
        }
    }
    assert!(
        max_step < 0.02 * src_h * 4.0,
        "hand moved too far during hit_stop: max_step={max_step:.4}m"
    );
}

#[test]
fn glb_extras_record_hit_stop_and_cancel_window_roundtrip() {
    let src = cleave_attack_doc();
    let opts = GiantOptions {
        scale: 4.0,
        hit_stop_s: 0.12,
        cancel_window_s: 0.25,
        ..GiantOptions::default()
    };
    let (out_doc, _) = giant_glb(&src, &opts, &[], None).unwrap();
    let bytes = out_doc.to_bytes().unwrap();
    let parsed = Document::parse(&bytes).unwrap();

    let anims = parsed.array("animations");
    assert_eq!(anims.len(), 1);
    let giant = anims[0]
        .get("extras")
        .and_then(|e| e.get("motionforge_giant"))
        .expect("animations[0].extras.motionforge_giant must exist after round-trip");
    let scale = giant
        .get("scale")
        .and_then(|v| v.as_f64())
        .expect("extras.scale");
    assert!((scale - 4.0).abs() < 1e-6);
    let dur = giant
        .get("duration_s")
        .and_then(|v| v.as_f64())
        .expect("extras.duration_s");
    assert!(dur > 0.5);

    let events = giant
        .get("events")
        .and_then(|v| v.as_arr())
        .expect("extras.events array");
    let find_ev = |kind: &str| {
        events
            .iter()
            .find(|e| e.get("kind").and_then(|k| k.as_str()) == Some(kind))
            .unwrap_or_else(|| panic!("missing event kind={kind} in {events:?}"))
    };
    let hs = find_ev("hit_stop");
    let hs_s = hs.get("start_s").and_then(|v| v.as_f64()).unwrap();
    let hs_e = hs.get("end_s").and_then(|v| v.as_f64()).unwrap();
    assert!(hs_s >= 0.0 && hs_e <= dur + 1e-6);
    assert!(
        ((hs_e - hs_s) - 0.12).abs() < 0.05,
        "hit_stop duration mismatch: {hs_s}..{hs_e}"
    );

    let cw = find_ev("cancel_window");
    let cw_s = cw.get("start_s").and_then(|v| v.as_f64()).unwrap();
    let cw_e = cw.get("end_s").and_then(|v| v.as_f64()).unwrap();
    assert!(
        cw_s >= hs_e - 1e-6,
        "cancel_window must start at or after hit_stop end: cw_s={cw_s} hs_e={hs_e}"
    );
    assert!(cw_e > cw_s && cw_e <= dur + 1e-6);
}

#[test]
fn arms_only_mask_pins_pelvis_and_legs_while_arms_swing() {
    let src = cleave_attack_doc();
    let (_, _, src_h) = clip_duration_and_root_travel(&src, "cleave");
    let opts = GiantOptions {
        scale: 4.0,
        mask: BodyMask::ArmsOnly,
        pin_pelvis: true,
        ..GiantOptions::default()
    };
    let (out_doc, reports) = giant_glb(&src, &opts, &[], None).unwrap();
    assert_eq!(reports[0].mask, BodyMask::ArmsOnly);
    assert!(reports[0].pin_pelvis);
    let out = Document::parse(&out_doc.to_bytes().unwrap()).unwrap();

    // Check GLB extras record mask & pin_pelvis.
    let giant = out.array("animations")[0]
        .get("extras")
        .and_then(|e| e.get("motionforge_giant"))
        .expect("extras.motionforge_giant");
    assert_eq!(
        giant.get("mask").and_then(|v| v.as_str()),
        Some("arms-only")
    );
    assert_eq!(
        giant.get("pin_pelvis").and_then(|v| v.as_bool()),
        Some(true)
    );

    let rig = Rig::from_doc(&out).unwrap();
    let map = map_rig(&rig);
    let anim = Animation::load_all(&out)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let (_, frames) = sample(&rig, &anim, Some(30.0));

    // Pelvis and all leg bones stay pinned at rest on every frame.
    for pinned_bone in [
        "DEF-spine",
        "DEF-thigh.L",
        "DEF-thigh.R",
        "DEF-shin.L",
        "DEF-shin.R",
        "DEF-foot.L",
        "DEF-foot.R",
    ] {
        let Some(node) = map.node_of(pinned_bone) else {
            continue;
        };
        let rest_w = rig.rest_world[node];
        for (fi, frame) in frames.iter().enumerate() {
            let w = rig.world(frame)[node];
            let pos_err = (w.t - rest_w.t).length();
            let dot = w.rotation().dot(rest_w.rotation()).abs().clamp(0.0, 1.0);
            let ang_err = 2.0 * dot.acos();
            assert!(
                pos_err < 1e-3 && ang_err < 1e-2,
                "{pinned_bone} moved on frame {fi}: pos_err={pos_err:.5}m ang_err={ang_err:.5}rad"
            );
        }
    }

    // Arms still swing with substantial giant excursion.
    let (_, hand_pts) = sampled_node_positions(&out, "cleave", "DEF-hand.R");
    let mut max_span = 0.0_f64;
    for a in &hand_pts {
        for b in &hand_pts {
            let d = (*a - *b).length();
            if d > max_span {
                max_span = d;
            }
        }
    }
    assert!(
        max_span > 0.5 * src_h * 4.0,
        "arms-only giant hand barely moved: max_span={max_span:.3}m"
    );
}

#[test]
fn giant_is_byte_deterministic_and_rejects_invalid_scale() {
    let src = cleave_attack_doc();
    let opts = GiantOptions {
        scale: 4.0,
        ..GiantOptions::default()
    };
    let (a, _) = giant_glb(&src, &opts, &[], None).unwrap();
    let (b, _) = giant_glb(&src, &opts, &[], None).unwrap();
    assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap());

    for bad in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        let bad_opts = GiantOptions {
            scale: bad,
            ..GiantOptions::default()
        };
        assert!(
            giant_glb(&src, &bad_opts, &[], None).is_err(),
            "scale={bad} must be rejected"
        );
    }
}
