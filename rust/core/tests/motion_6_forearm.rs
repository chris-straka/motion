// Forearm roll & LBS elbow candy-wrap gate (motion-6):
// On a rig without forearm twist bones, linear blend skinning shrinks a 50/50
// elbow ring's cross-sectional area by cos^2(twist / 2):
//   - at 80 deg:  cos(40 deg)  = 0.766, area = 58.7% (stays above 50%)
//   - at 105 deg: cos(52.5 deg) = 0.609, area = 37.1% (collapses below half -> D_VOLUME)
// These tests fail when FOREARM_ROLL_DEG allows 105 deg on twistless rigs and
// pass once FOREARM_ROLL_DEG is restored to 80 deg.

use motion_core::attack::{parse_spec, preset, solve, FOREARM_ROLL_DEG, PRESETS};
use motion_core::fixture::{humanoid_mitten, Naming};
use motion_core::glb::Document;
use motion_core::keypose::fit::{twist_angle, RigInfo};
use motion_core::math::{Quat, Vec3};
use motion_core::standardize::{standardize, Outcome};

/// Attack spec that twists the sword blade strongly around the forearm axis
/// (pronation > 120 deg) so the solver hits its forearm-roll clamp.
const HIGH_PRONATION_SPEC: &str = r#"{"format":"motionforge-attack","version":1,"name":"roll_stress","hand":"R","keys":[
 {"role":"ready","grip":[-0.2,0.55,0.12],"blade":[0,1,0.3]},
 {"role":"windup","t":0.2,"grip":[-0.18,0.68,0.22],"blade":[0.2,-0.95,-0.25]},
 {"role":"strike","t":0.15,"grip":[-0.08,0.62,0.32],"blade":[-0.3,-0.90,0.30]},
 {"role":"follow","t":0.15,"grip":[0.05,0.55,0.25],"blade":[0.0,1.0,0.2]}]}"#;

fn twistless_rig() -> Document {
    let raw = humanoid_mitten(Naming::Plain, false, 0.0).unwrap();
    match standardize(&raw, "humanoid").unwrap() {
        Outcome::Done(d, _) => Document::parse(&d.to_bytes().unwrap()).unwrap(),
        Outcome::Refused(_, why) => panic!("{why}"),
    }
}

/// Measure peak axial twist (degrees) between `DEF-upper_arm.R` and
/// `DEF-forearm.R` relative to their rest relationship, across all frames of
/// `spec`.
fn peak_elbow_axial_twist_deg(info: &RigInfo, spec_json: &str) -> f64 {
    let spec = parse_spec(spec_json).unwrap();
    let (_, frames, _) = solve(info, &spec).unwrap();
    let rig = &info.rig;
    let (up, fo, ha) = (info.upper[1], info.fore[1], info.hand[1]);
    let rest_up = rig.rest_world[up].rotation();
    let rest_fo = rig.rest_world[fo].rotation();
    let mut peak = 0.0f64;
    for fr in &frames {
        let w = rig.world(fr);
        let axis_w = (w[ha].t - w[fo].t).normalized();
        // Forearm rotation relative to upper arm, compared to its rest pose on upper arm.
        let natural_fo = w[up].rotation().mul(rest_up.conj()).mul(rest_fo);
        let delta = w[fo].rotation().mul(natural_fo.conj());
        let deg = twist_angle(delta, axis_w).abs().to_degrees();
        if deg > peak {
            peak = deg;
        }
    }
    peak
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn forearm_roll_cap_is_at_most_80_deg_and_twistless_attacks_respect_it() {
    assert!(
        FOREARM_ROLL_DEG <= 80.0 + 1e-9,
        "FOREARM_ROLL_DEG ({FOREARM_ROLL_DEG}) must be <= 80 deg to avoid forearm candy-wrapping on twistless rigs"
    );
    let doc = twistless_rig();
    let info = RigInfo::new(&doc).unwrap();
    assert!(
        motion_core::helpers::forearm_twists(&info.rig).is_empty(),
        "fixture must have no forearm twist bones"
    );
    for name in PRESETS {
        let peak = peak_elbow_axial_twist_deg(&info, preset(name).unwrap());
        assert!(
            peak <= 80.5,
            "preset {name} twisted twistless forearm by {peak:.1} deg (> 80.5 deg)"
        );
    }
    let stress_peak = peak_elbow_axial_twist_deg(&info, HIGH_PRONATION_SPEC);
    assert!(
        stress_peak <= 80.5,
        "high-pronation attack twisted twistless forearm by {stress_peak:.1} deg (> 80.5 deg)"
    );
}

#[test]
fn lbs_elbow_ring_cross_section_does_not_collapse_below_half_area() {
    // Evaluate Linear Blend Skinning on an 8-vertex circular ring weighted
    // 50% DEF-upper_arm.R / 50% DEF-forearm.R at the elbow seam across the
    // high-pronation attack. Candy-wrapping collapses the ring area below
    // 0.50 (D_VOLUME threshold); at <= 80 deg twist, area stays >= 0.58.
    let doc = twistless_rig();
    let info = RigInfo::new(&doc).unwrap();
    let spec = parse_spec(HIGH_PRONATION_SPEC).unwrap();
    let (_, frames, _) = solve(&info, &spec).unwrap();
    let rig = &info.rig;
    let (up, fo, ha) = (info.upper[1], info.fore[1], info.hand[1]);
    let elbow = rig.head(fo);
    let rest_axis = (rig.head(ha) - elbow).normalized();
    let helper = if rest_axis.y.abs() > 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = rest_axis.cross(helper).normalized();
    let v = rest_axis.cross(u);
    let r0 = 0.04;
    let rest_ring: Vec<Vec3> = (0..8)
        .map(|k| {
            let a = std::f64::consts::TAU * (k as f64) / 8.0;
            let (s, c) = motion_core::detmath::sin_cos(a);
            elbow + u.scale(r0 * c) + v.scale(r0 * s)
        })
        .collect();
    let rest_up_inv = rig.rest_world[up].inverse().unwrap();
    let rest_fo_inv = rig.rest_world[fo].inverse().unwrap();

    let mut min_area_ratio = 1.0f64;
    for fr in &frames {
        let w = rig.world(fr);
        let m_up = w[up].mul(&rest_up_inv);
        let m_fo = w[fo].mul(&rest_fo_inv);
        // Measure twist-only LBS pinch (factor out hinge bend by comparing
        // against pure forearm-axis twist of the elbow seam).
        let axis_w = (w[ha].t - w[fo].t).normalized();
        let natural_fo = w[up]
            .rotation()
            .mul(rig.rest_world[up].rotation().conj())
            .mul(rig.rest_world[fo].rotation());
        let twist = twist_angle(w[fo].rotation().mul(natural_fo.conj()), axis_w);
        let q_twist = Quat::from_axis_angle(rest_axis, twist);
        let mut r_sum = 0.0;
        for &p in &rest_ring {
            let p_rel = p - elbow;
            // 50/50 LBS blend between un-twisted upper_arm and twisted forearm
            let p_lbs = p_rel.scale(0.5) + q_twist.rotate_vec(p_rel).scale(0.5);
            r_sum += p_lbs.length();
            // Also sanity check full posed vertices stay finite.
            let p_full = m_up.apply(p).scale(0.5) + m_fo.apply(p).scale(0.5);
            assert!(p_full.length().is_finite());
        }
        let mean_r_ratio = (r_sum / rest_ring.len() as f64) / r0;
        let area_ratio = mean_r_ratio * mean_r_ratio;
        if area_ratio < min_area_ratio {
            min_area_ratio = area_ratio;
        }
    }
    assert!(
        min_area_ratio >= 0.55,
        "LBS elbow ring collapsed to {:.1}% area (< 55% floor; candy-wrapping!)",
        min_area_ratio * 100.0
    );
}
