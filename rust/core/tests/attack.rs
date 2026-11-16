// Attack authoring on the procedural fixture: the sword is where the keys
// put it, planted feet stay put, the build is deterministic.

use motion_core::attack::{build, parse_spec, preset, solve, Ease, Solver, PRESETS};
use motion_core::fixture::{humanoid, Naming};
use motion_core::glb::Document;
use motion_core::keypose::fit::RigInfo;
use motion_core::math::Vec3;
use motion_core::standardize::{standardize, Outcome};

fn target() -> Document {
    let body = humanoid(Naming::Plain, false, false).unwrap();
    match standardize(&body, "humanoid").unwrap() {
        Outcome::Done(d, _) => Document::parse(&d.to_bytes().unwrap()).unwrap(),
        Outcome::Refused(_, why) => panic!("{why}"),
    }
}

/// A cut the fixture's arm reaches with room to spare.
const EASY: &str = r#"{"format":"motionforge-attack","version":1,"name":"easy","hand":"R","keys":[
 {"role":"ready","grip":[-0.2,0.55,0.12],"blade":[0,1,0.3]},
 {"role":"windup","t":0.2,"hold":0.1,"grip":[-0.22,0.8,-0.05],"blade":[-0.3,0.8,-0.5],
  "hips":{"turn":-30,"drop":0.05,"shift":[0,-0.02]},"torso":{"twist":-25,"lean":-5},
  "feet":{"R":[-0.04,-0.1,-30]}},
 {"role":"strike","t":0.1,"ease":"in","grip":[-0.05,0.6,0.3],"blade":[0.4,-0.2,0.9],
  "hips":{"turn":15,"drop":0.1,"shift":[0.01,0.04]},"torso":{"twist":15,"lean":20}},
 {"role":"follow","t":0.1,"hold":0.2,"ease":"out","grip":[0.1,0.5,0.2],"blade":[0.8,-0.4,0.1],
  "hips":{"turn":30,"drop":0.12,"shift":[0.02,0.05]},"torso":{"twist":25,"lean":25}}]}"#;

#[test]
fn the_sword_is_where_each_key_puts_it() {
    let doc = target();
    let info = RigInfo::new(&doc).unwrap();
    let spec = parse_spec(EASY).unwrap();
    let solver = Solver::new(&info, &spec).unwrap();
    let (_, frames, report) = solve(&info, &spec).unwrap();
    // 0.2 s + 0.1 held + 0.1 + 0.1 + 0.2 held at 30 fps, plus frame 0.
    assert_eq!(frames.len(), 1 + 6 + 3 + 3 + 3 + 6);
    let at: Vec<usize> = report.keys.iter().map(|k| k.0).collect();
    assert_eq!(at, vec![0, 6, 12, 15]);
    for (k, key) in spec.keys.iter().enumerate() {
        let m = report.key_miss[k];
        let (grip, blade) = solver.sword(&frames[at[k]]);
        let want = solver.point(key.grip);
        let dir = info.to_rig((key.tip - key.grip).normalized());
        // The grip is on its point at every key.
        assert!(m.grip < 1e-9, "key {k}: {m:?}");
        assert!(
            (grip - want).length() < 1e-6 * solver.h,
            "key {k}: grip {grip:?} vs {want:?}"
        );
        // The blade is along its direction (the windup and the strike
        // here), or off by exactly what the report says (the wrist's
        // limit: the ready pose and the follow-through here).
        let off = blade.dot(dir).clamp(-1.0, 1.0).acos().to_degrees();
        assert!((off - m.blade_deg).abs() < 1e-3, "key {k}: {off} vs {m:?}");
        assert!(m.blade_deg < 20.0, "key {k}: {m:?}");
        if k == 1 || k == 2 {
            assert!(m.blade_deg < 1e-3, "key {k}: {m:?}");
        }
    }
    // Held frames repeat the key exactly.
    assert_eq!(format!("{:?}", frames[6]), format!("{:?}", frames[9]));
}

#[test]
fn planted_feet_do_not_move_and_a_turning_foot_pivots_on_its_ball() {
    let doc = target();
    let info = RigInfo::new(&doc).unwrap();
    let spec = parse_spec(EASY).unwrap();
    let (_, frames, _) = solve(&info, &spec).unwrap();
    let rig = &info.rig;
    let ball = |f: usize, s: usize| rig.world(&frames[f])[info.toe[s].unwrap()].t;
    let ankle = |f: usize, s: usize| rig.world(&frames[f])[info.foot[s]].t;
    // The left foot never moves: its ball stays put all clip, while the
    // hips drop, shift and turn over it (its heel may come up a little
    // when the shin leans far over it; never under the floor).
    for f in 1..frames.len() {
        assert!((ball(f, 0) - ball(0, 0)).length() < 1e-9, "frame {f}");
        assert!(ankle(f, 0).y > ankle(0, 0).y - 1e-9, "frame {f}");
    }
    // The right foot steps back during the windup (it lifts on the way),
    // then stands: nothing moves from the windup key to the end.
    assert!(ankle(3, 1).y > ankle(0, 1).y + 0.02 * info.k, "no step");
    for f in 7..frames.len() {
        assert!((ball(f, 1) - ball(6, 1)).length() < 1e-9, "frame {f}");
    }
    // Turned 30 degrees about its ball: the ball is exactly the rest ball
    // moved by the key's offset, whatever the turn.
    let h = (rig.head(info.head.unwrap()).y - info.ground) / motion_core::readcam::HEAD_SHARE;
    let want = rig.head(info.toe[1].unwrap()) + info.to_rig(Vec3::new(-0.04, 0.0, -0.1)).scale(h);
    assert!(
        (ball(6, 1) - want).length() < 1e-9,
        "{:?} vs {want:?}",
        ball(6, 1)
    );
}

#[test]
fn every_preset_builds_twice_the_same_and_parses() {
    let doc = target();
    for name in PRESETS {
        let spec = parse_spec(preset(name).unwrap()).unwrap();
        assert_eq!(spec.name, name);
        let (a, ra) = build(&doc, &spec, None).unwrap();
        let (b, _) = build(&doc, &spec, None).unwrap();
        assert_eq!(a.to_bytes().unwrap(), b.to_bytes().unwrap(), "{name}");
        assert!(ra.frames > 20, "{name}");
        // No foot ever misses its place (the hips come down instead).
        assert!(ra.worst.foot < 1e-6, "{name}: {:?}", ra.worst);
    }
    // Building again replaces the clip instead of adding a second one.
    let spec = parse_spec(preset("sweep").unwrap()).unwrap();
    let (once, _) = build(&doc, &spec, None).unwrap();
    let (twice, _) = build(&once, &spec, None).unwrap();
    assert_eq!(
        once.array("animations").len(),
        twice.array("animations").len()
    );
}

#[test]
fn specs_inherit_and_refuse_nonsense() {
    let spec = parse_spec(EASY).unwrap();
    // The strike says nothing of the feet: it keeps the windup's.
    assert_eq!(spec.keys[2].feet[1].dz, -0.1);
    assert_eq!(spec.keys[3].body.turn, 30.0);
    for bad in [
        r#"{"format":"x","keys":[]}"#,
        r#"{"format":"motionforge-attack","keys":[{"grip":[0,0,0],"blade":[0,1,0]}]}"#,
        r#"{"format":"motionforge-attack","keys":[{"blade":[0,1,0]},{"t":0.1}]}"#,
        r#"{"format":"motionforge-attack","keys":[{"grip":[0,0,0],"blade":[0,1,0]},{"grip":[0,1,0]}]}"#,
        r#"{"format":"motionforge-attack","keys":[{"grip":[0,0,0],"blade":[0,0,0]},{"t":0.1}]}"#,
    ] {
        assert!(parse_spec(bad).is_err(), "{bad}");
    }
}

#[test]
fn eases_start_end_and_keep_their_middles_visible() {
    for e in [Ease::Linear, Ease::In, Ease::Out, Ease::InOut] {
        assert_eq!(e.at(0.0), 0.0);
        assert!((e.at(1.0) - 1.0).abs() < 1e-12);
        let mut last = 0.0;
        for i in 1..=10 {
            let v = e.at(i as f64 / 10.0);
            assert!(v > last, "{e:?}");
            last = v;
        }
    }
    // A three-frame strike shows its middle: a third of the way in time is
    // at least a fifth of the way there.
    assert!(Ease::In.at(1.0 / 3.0) > 0.19);
    assert!((Ease::In.at(0.3) + Ease::Out.at(0.7) - 1.0).abs() < 1e-12);
}

#[test]
fn a_hand_without_fingers_keeps_the_games_hold_and_its_socket() {
    // The procedural fixture has no finger bones: no palm hold, the
    // socket is not moved, no finger channels are written.
    let doc = target();
    let info = RigInfo::new(&doc).unwrap();
    assert!(motion_core::attack::grip::palm_hold(&info, 1, 1.0).is_none());
    let spec = parse_spec(EASY).unwrap();
    let (out, rep) = build(&doc, &spec, None).unwrap();
    assert!(!rep.socket_rewritten);
    let sock = info.socket[1].unwrap();
    assert_eq!(
        format!("{:?}", out.array("nodes")[sock]),
        format!("{:?}", doc.array("nodes")[sock])
    );
}

#[test]
fn overlap_puts_the_hips_ahead_and_the_sword_behind_landing_together() {
    let spec = parse_spec(EASY).unwrap();
    assert_eq!(spec.lead, [0.35, 0.2, 0.2]);
    let (params, at) = spec.frames();
    // Half-way through the strike (windup key -> strike key), the hips
    // have turned further than an even share and the sword less far; on
    // the key all are there.
    let (w, s) = (at[1], at[2]);
    let mid = w + 3 + (s - w - 3) / 2; // past the windup's hold
    let k = &spec.keys;
    let hips_share = (params[mid].body.turn - k[1].body.turn) / (k[2].body.turn - k[1].body.turn);
    let grip_share = (params[mid].grip - k[1].grip).length() / (k[2].grip - k[1].grip).length();
    assert!(
        hips_share > grip_share + 0.2,
        "{hips_share} vs {grip_share}"
    );
    assert!((params[s].body.turn - k[2].body.turn).abs() < 1e-12);
    assert!((params[s].grip - k[2].grip).length() < 1e-12);
}

#[test]
fn single_finger_chain_places_palm_socket_and_curls_round_the_handle() {
    use motion_core::fixture::humanoid_mitten;
    use motion_core::rig::{Animation, Rig};
    // Twisted A-pose single-finger rig: local +X is randomized on every bone,
    // so curling must follow the hand's world palm axis, not local +X.
    let raw = humanoid_mitten(Naming::Plain, true, 75.0).unwrap();
    let (std_doc, _) = match standardize(&raw, "humanoid").unwrap() {
        Outcome::Done(d, r) => (Document::parse(&d.to_bytes().unwrap()).unwrap(), r),
        Outcome::Refused(_, why) => panic!("{why}"),
    };
    let spec = parse_spec(EASY).unwrap();
    let (built, rep) = build(&std_doc, &spec, None).unwrap();
    assert!(rep.socket_rewritten);
    for (k, m) in rep.key_miss.iter().enumerate() {
        assert!(m.grip < 1e-6, "key {k}: {m:?}");
    }
    let rig = Rig::from_doc(&built).unwrap();
    let anim = &Animation::load_all(&built).unwrap()[0];
    let w = rig.world(&anim.sample(&rig, 0.4));
    let hand = rig.find("DEF-hand.R").unwrap();
    let f1 = rig.find("DEF-f_middle.01.R").unwrap();
    let f2 = rig.find("DEF-f_middle.02.R").unwrap();
    let f3 = rig.find("DEF-f_middle.03.R").unwrap();
    // Every segment turned substantially from rest, and the fingertip curled
    // toward the socket (the palm side), not away from it.
    for (p, c) in [(hand, f1), (f1, f2), (f2, f3)] {
        let turn = motion_core::readcam::joint_turn_deg(
            rig.rest_world[p].rotation(),
            rig.rest_world[c].rotation(),
            w[p].rotation(),
            w[c].rotation(),
        );
        assert!(
            turn > 30.0,
            "segment {} turned only {turn} deg",
            rig.names[c]
        );
    }
    let sock = rig.find("Socket_Hand_R").unwrap();
    let rest_tip_to_sock = (rig.head(f3) - rig.rest_world[sock].t).length();
    let posed_tip_to_sock = (w[f3].t - w[sock].t).length();
    assert!(
        posed_tip_to_sock < rest_tip_to_sock,
        "curled tip moved away from socket: {posed_tip_to_sock} vs {rest_tip_to_sock}"
    );

    // Also cover an already-exported rig where an unmapped 3-bone finger chain
    // sits under `DEF-hand.L/R` outside `skins[0].joints`: `place_sockets` and
    // `attack` still place the palm socket and key the finger nodes.
    let mut unskinned = raw.clone();
    for node in unskinned.array_mut("nodes").iter_mut() {
        let name = node
            .get("name")
            .and_then(motion_core::json::Json::as_str)
            .unwrap_or("")
            .to_string();
        if let Some(rest) = name.strip_prefix("Finger") {
            node.set("name", motion_core::json::Json::str(&format!("Claw{rest}")));
        }
    }
    let (mut unskinned_std, _) = match standardize(&unskinned, "humanoid").unwrap() {
        Outcome::Done(d, r) => (Document::parse(&d.to_bytes().unwrap()).unwrap(), r),
        Outcome::Refused(_, why) => panic!("{why}"),
    };
    let placed = motion_core::attack::grip::place_sockets(&mut unskinned_std).unwrap();
    assert_eq!(placed, vec!["Socket_Hand_L", "Socket_Hand_R"]);
    let (built_u, rep_u) = build(&unskinned_std, &spec, None).unwrap();
    assert!(rep_u.socket_rewritten);
    let rig_u = Rig::from_doc(&built_u).unwrap();
    let anim_u = &Animation::load_all(&built_u).unwrap()[0];
    let f1_u = rig_u.find("Claw1_R").unwrap();
    assert!(
        !rig_u.joints().contains(&f1_u),
        "Claw1_R should be outside skins[0].joints"
    );
    assert!(
        anim_u.animated_nodes().contains(&f1_u),
        "unskinned Claw1_R must still be keyed by attack::build"
    );
}
