// Fixture gate tests for motion-9 (non-quadruped creature locomotion:
// two-legged upright & hunched monsters with planted feet and <= 2.5 cm skate,
// plus legless creatures with a traveling body wave).

use motion_core::clip::{fk, Bone, Skeleton};
use motion_core::creature::anybody::{read, walk_stride};
use motion_core::creature::gait::Stride;
use motion_core::creature::profile::{build, skeleton_bounds, Creature};
use motion_core::creature::rigcheck::{from_glb, Options};
use motion_core::detmath;
use motion_core::fixture::{humanoid, Naming};
use motion_core::math::{Quat, Vec3};

fn left_axis(c: &Creature) -> Vec3 {
    Vec3::new(0.0, 0.0, 1.0).cross(c.forward).normalized()
}

fn bone_tail(sk: &Skeleton, posed: &[motion_core::clip::FkBone], b: usize) -> Vec3 {
    let len = sk.bone_length(b);
    posed[b].head + posed[b].rot.mul_vec(Vec3::new(0.0, len, 0.0))
}

fn turn_deg(q: Quat) -> f64 {
    (2.0 * detmath::acos(q.dot(Quat::IDENTITY).abs().min(1.0))).to_degrees()
}

fn upright_biped() -> Creature {
    let doc = humanoid(Naming::Mixamo, false, false).unwrap();
    from_glb(
        &doc,
        &Options {
            forward: None,
            view_size: 64,
            name: "biped".into(),
        },
    )
    .unwrap()
    .creature
}

/// A hunched, long-armed two-legged monster: forward-pitched spine, two bent
/// legs with feet at z = 0, two long arms hanging free.
fn hunched_monster() -> Creature {
    let mut bones = Vec::new();
    let mut add = |name: &str, parent: Option<usize>, h: [f64; 3], t: [f64; 3]| -> usize {
        let id = bones.len();
        bones.push(Bone {
            name: name.into(),
            parent,
            head: Vec3::new(h[0], h[1], h[2]),
            tail: Vec3::new(t[0], t[1], t[2]),
        });
        id
    };
    // Forward is -Y, Up is +Z (matching creature/gait.rs dog fixture).
    let hips = add("hips", None, [0.0, 0.0, 0.90], [0.0, -0.08, 1.02]);
    let s1 = add("spine1", Some(hips), [0.0, -0.08, 1.02], [0.0, -0.24, 1.18]);
    let chest = add("chest", Some(s1), [0.0, -0.24, 1.18], [0.0, -0.42, 1.30]);
    let neck = add("neck", Some(chest), [0.0, -0.42, 1.30], [0.0, -0.56, 1.36]);
    let _head = add("head", Some(neck), [0.0, -0.56, 1.36], [0.0, -0.72, 1.34]);

    // Two long arms hanging free (tails stay well above z = 0).
    for (side, x) in [("L", 0.28), ("R", -0.28)] {
        let ua = add(
            &format!("upper_arm.{side}"),
            Some(chest),
            [x, -0.38, 1.24],
            [x * 1.15, -0.44, 0.84],
        );
        let fa = add(
            &format!("forearm.{side}"),
            Some(ua),
            [x * 1.15, -0.44, 0.84],
            [x * 1.10, -0.58, 0.44],
        );
        let _claw = add(
            &format!("claw.{side}"),
            Some(fa),
            [x * 1.10, -0.58, 0.44],
            [x * 1.05, -0.66, 0.28],
        );
    }

    // Two bent monster legs ending on z = 0.
    let mut feet = Vec::new();
    for (side, x) in [("L", 0.16), ("R", -0.16)] {
        let th = add(
            &format!("thigh.{side}"),
            Some(hips),
            [x, 0.0, 0.86],
            [x, -0.14, 0.46],
        );
        let sh = add(
            &format!("shin.{side}"),
            Some(th),
            [x, -0.14, 0.46],
            [x, 0.02, 0.08],
        );
        let ft = add(
            &format!("foot.{side}"),
            Some(sh),
            [x, 0.02, 0.08],
            [x, -0.12, 0.0],
        );
        feet.push(ft);
    }

    let sk = Skeleton { bones };
    let (lo, hi) = skeleton_bounds(&sk);
    build(
        "hunched_monster",
        sk,
        feet,
        Vec3::new(0.0, -1.0, 0.0),
        lo,
        hi,
        256,
    )
}

/// A legless undulating creature (serpent / wyrm): 7-bone spine along +Y from
/// snout to tail (facing -Y), zero feet.
fn legless_serpent() -> Creature {
    let mut bones = Vec::new();
    let mut prev = None;
    for i in 0..7 {
        let y0 = 0.0 + 0.25 * i as f64;
        let y1 = y0 + 0.25;
        let id = bones.len();
        bones.push(Bone {
            name: format!("seg{i}"),
            parent: prev,
            head: Vec3::new(0.0, y0, 0.15),
            tail: Vec3::new(0.0, y1, 0.15),
        });
        prev = Some(id);
    }
    let sk = Skeleton { bones };
    let (lo, hi) = skeleton_bounds(&sk);
    build(
        "serpent",
        sk,
        Vec::new(),
        Vec3::new(0.0, -1.0, 0.0),
        lo,
        hi,
        256,
    )
}

/// World-space foot-tip track once the body moves forward at `s.speed`.
fn foot_track(c: &Creature, s: &Stride, leg: &[usize]) -> Vec<Vec3> {
    let foot = *leg.last().unwrap();
    let len = c.skeleton.bone_length(foot);
    s.clip
        .frames
        .iter()
        .enumerate()
        .map(|(f, frame)| {
            let posed = fk(&c.skeleton, frame);
            let tail = posed[foot].head + posed[foot].rot.mul_vec(Vec3::new(0.0, len, 0.0));
            tail + c.forward.scale(s.speed * f as f64 / s.clip.fps)
        })
        .collect()
}

fn assert_biped_walk_quality(c: &Creature, label: &str) {
    let body = read(c);
    assert_eq!(
        body.legs.len(),
        2,
        "{label}: expected 2 legs, got {:?}",
        body
    );
    let s =
        walk_stride(c, &body, 30.0).unwrap_or_else(|e| panic!("{label}: walk_stride failed: {e}"));
    let n = s.clip.frames.len();
    assert!(n >= 12, "{label}: too few frames ({n})");
    assert!(s.speed > 0.1, "{label}: speed too low ({})", s.speed);
    assert!(
        s.stance > 0.12 * c.height,
        "{label}: stance too short ({} vs h={})",
        s.stance,
        c.height
    );
    assert!((s.period - n as f64 / 30.0).abs() < 1e-9);

    let mut grounded_masks: Vec<Vec<bool>> = Vec::new();
    for (li, leg) in body.legs.iter().enumerate() {
        let track = foot_track(c, &s, leg);
        let foot = *leg.last().unwrap();
        let rest_z = c.skeleton.bones[foot].tail.z;

        // Grounded frames: within 1.5% of height of rest floor.
        let thresh = (0.015 * c.height).max(0.015);
        let grounded: Vec<bool> = track.iter().map(|p| p.z <= rest_z + thresh).collect();
        let grounded_count = grounded.iter().filter(|&&g| g).count();
        assert!(
            grounded_count >= n / 3,
            "{label} leg {li}: too few planted frames ({grounded_count}/{n})"
        );

        // Find contiguous stance runs (wrapping across loop boundary) and check skate <= 2.5 cm.
        let mut run_start = 0usize;
        if grounded[0] && grounded[n - 1] {
            while run_start < n && grounded[run_start] {
                run_start += 1;
            }
        }
        let mut cur_run: Vec<Vec3> = Vec::new();
        let mut max_skate = 0.0_f64;
        let mut checked_runs = 0usize;
        for step in 0..=n {
            let f = (run_start + step) % n;
            let lap = if run_start + step >= n {
                s.speed * s.period
            } else {
                0.0
            };
            if step < n && grounded[f] {
                cur_run.push(track[f] + c.forward.scale(lap));
            } else if cur_run.len() >= 3 {
                // Trim 1-frame liftoff/touchdown transitions if run is long enough.
                let slice = if cur_run.len() >= 5 {
                    &cur_run[1..cur_run.len() - 1]
                } else {
                    &cur_run[..]
                };
                let mean = slice
                    .iter()
                    .fold(Vec3::ZERO, |a, b| a + *b)
                    .scale(1.0 / slice.len() as f64);
                for p in slice {
                    let horiz = Vec3::new(p.x - mean.x, p.y - mean.y, 0.0).length();
                    if horiz > max_skate {
                        max_skate = horiz;
                    }
                }
                checked_runs += 1;
                cur_run.clear();
            } else {
                cur_run.clear();
            }
        }
        assert!(
            checked_runs >= 1,
            "{label} leg {li}: no stance run >= 3 frames"
        );
        let max_allowed_skate = (0.025 * (c.height / 1.75)).max(0.025);
        assert!(
            max_skate <= max_allowed_skate,
            "{label} leg {li}: planted foot skate {max_skate:.4}m > {max_allowed_skate:.4}m"
        );

        // Foot lifts cleanly during swing and never sinks below the floor.
        let top = track.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
        let low = track.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
        assert!(
            top > rest_z + 0.03 * c.height,
            "{label} leg {li}: swing lift too small ({:.4}m)",
            top - rest_z
        );
        assert!(
            low >= rest_z - 0.008 * c.height,
            "{label} leg {li}: foot sank below floor ({:.4}m)",
            rest_z - low
        );
        grounded_masks.push(grounded);
    }

    // Alternating support: never both feet airborne on the same frame.
    let both_air = (0..n)
        .filter(|&f| !grounded_masks[0][f] && !grounded_masks[1][f])
        .count();
    assert_eq!(
        both_air, 0,
        "{label}: both feet left the ground simultaneously on {both_air} frames"
    );

    // Exact bone lengths preserved across every frame.
    for frame in &s.clip.frames {
        let posed = fk(&c.skeleton, frame);
        for leg in &body.legs {
            for w in leg.windows(2) {
                let rest = (c.skeleton.bones[w[1]].head - c.skeleton.bones[w[0]].head).length();
                let now = (posed[w[1]].head - posed[w[0]].head).length();
                assert!(
                    (rest - now).abs() < 1e-6,
                    "{label}: bone length stretched ({rest} -> {now})"
                );
            }
        }
    }
}

#[test]
fn hunched_two_legged_monster_walks_with_planted_feet_and_arm_swing() {
    let c = hunched_monster();
    assert_biped_walk_quality(&c, "hunched_monster");

    // Arms must counter-swing visibly instead of freezing.
    let body = read(&c);
    let s = walk_stride(&c, &body, 30.0).unwrap();
    let max_arm_deg = body
        .arms
        .iter()
        .flat_map(|arm| {
            s.clip
                .frames
                .iter()
                .map(move |fr| turn_deg(fr.poses[arm[0]].quat))
        })
        .fold(0.0_f64, f64::max);
    assert!(
        max_arm_deg > 8.0,
        "hunched monster arms barely swung ({max_arm_deg:.2} deg)"
    );
}

#[test]
fn upright_biped_fixture_walks_with_planted_feet() {
    let c = upright_biped();
    assert_biped_walk_quality(&c, "upright_biped");
}

#[test]
fn legless_creature_moves_with_traveling_body_wave() {
    let c = legless_serpent();
    let body = read(&c);
    assert!(body.floats, "serpent must have no feet (floats=true)");
    let s = walk_stride(&c, &body, 30.0).expect("legless walk_stride must succeed");
    let n = s.clip.frames.len();
    assert!(n >= 12, "serpent cycle too short: {n}");
    assert!(
        s.speed > 0.1,
        "serpent forward speed too small: {}",
        s.speed
    );

    let left = left_axis(&c);
    let rest_fk = fk(
        &c.skeleton,
        &motion_core::clip::Frame {
            poses: vec![
                motion_core::clip::Pose {
                    loc: Vec3::ZERO,
                    quat: Quat::IDENTITY
                };
                c.skeleton.len()
            ],
        },
    );

    // Measure first-harmonic Fourier phase of lateral displacement at each spine bone.
    let mut amps = Vec::new();
    let mut phases = Vec::new();
    let tau = 2.0 * std::f64::consts::PI;
    for b in 0..c.skeleton.len() {
        let mut re = 0.0_f64;
        let mut im = 0.0_f64;
        let mut max_lat = 0.0_f64;
        let rest_tail = bone_tail(&c.skeleton, &rest_fk, b);
        for (f, frame) in s.clip.frames.iter().enumerate() {
            let posed = fk(&c.skeleton, frame);
            let lat = (bone_tail(&c.skeleton, &posed, b) - rest_tail).dot(left);
            if lat.abs() > max_lat {
                max_lat = lat.abs();
            }
            let ang = tau * (f as f64) / (n as f64);
            re += lat * detmath::cos(ang);
            im += lat * detmath::sin(ang);
        }
        amps.push(max_lat);
        // atan2(im, re) gives the phase of sin/cos; use detmath-safe phase comparison via unit phasor.
        let mag = (re * re + im * im).sqrt();
        phases.push(if mag > 1e-9 {
            (re / mag, im / mag)
        } else {
            (1.0, 0.0)
        });
    }

    let max_amp = amps.iter().copied().fold(0.0_f64, f64::max);
    assert!(
        max_amp > 0.02 * body.size,
        "legless serpent lateral wave too small: {max_amp:.4}m (size={:.3}m)",
        body.size
    );

    // Traveling wave check: phase must shift across the body (not an in-phase rigid wag!).
    // Compare phasor dot product between front third and rear third of the body.
    let (r0, i0) = phases[1];
    let (r1, i1) = phases[c.skeleton.len() - 2];
    let cos_dphi = (r0 * r1 + i0 * i1).clamp(-1.0, 1.0);
    let dphi_deg = detmath::acos(cos_dphi).to_degrees();
    assert!(
        dphi_deg > 25.0,
        "serpent wave must propagate with phase lag along the body, got only {dphi_deg:.1} deg"
    );

    // Smooth loop closure and zero bone stretching.
    for frame in &s.clip.frames {
        let posed = fk(&c.skeleton, frame);
        for b in 1..c.skeleton.len() {
            let rest = (c.skeleton.bones[b].head - c.skeleton.bones[b - 1].head).length();
            let now = (posed[b].head - posed[b - 1].head).length();
            assert!((rest - now).abs() < 1e-6, "serpent bone stretched");
        }
    }
    let step_dist = |a: usize, b: usize| {
        let fa = fk(&c.skeleton, &s.clip.frames[a]);
        let fb = fk(&c.skeleton, &s.clip.frames[b]);
        (0..c.skeleton.len())
            .map(|bi| (bone_tail(&c.skeleton, &fa, bi) - bone_tail(&c.skeleton, &fb, bi)).length())
            .sum::<f64>()
    };
    let mean_step: f64 = (0..n - 1).map(|i| step_dist(i, i + 1)).sum::<f64>() / (n - 1) as f64;
    let wrap_step = step_dist(n - 1, 0);
    assert!(
        wrap_step <= mean_step * 2.5 + 1e-6,
        "serpent wave does not loop smoothly: wrap_step={wrap_step:.4} vs mean={mean_step:.4}"
    );
}

#[test]
fn nonquad_walk_is_deterministic_and_rejects_degenerate_skeleton() {
    let c = hunched_monster();
    let body = read(&c);
    let a = walk_stride(&c, &body, 30.0).unwrap();
    let b = walk_stride(&c, &body, 30.0).unwrap();
    assert_eq!(a.clip.frames.len(), b.clip.frames.len());
    assert_eq!(a.speed.to_bits(), b.speed.to_bits());
    for (fa, fb) in a.clip.frames.iter().zip(&b.clip.frames) {
        for (pa, pb) in fa.poses.iter().zip(&fb.poses) {
            assert_eq!(pa.quat, pb.quat);
            assert_eq!(pa.loc, pb.loc);
        }
    }

    // Single-bone skeleton cannot walk or undulate.
    let tiny_sk = Skeleton {
        bones: vec![Bone {
            name: "root".into(),
            parent: None,
            head: Vec3::ZERO,
            tail: Vec3::new(0.0, 0.0, 0.2),
        }],
    };
    let (lo, hi) = skeleton_bounds(&tiny_sk);
    let tiny = build(
        "tiny",
        tiny_sk,
        Vec::new(),
        Vec3::new(0.0, -1.0, 0.0),
        lo,
        hi,
        64,
    );
    let tiny_body = read(&tiny);
    assert!(
        walk_stride(&tiny, &tiny_body, 30.0).is_err(),
        "1-bone skeleton must return Err from walk_stride"
    );
}
