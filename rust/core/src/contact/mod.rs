//! Contact pass: keep the hands, forearms and a held weapon out of the
//! body in every clip.
//!
//! Retargeted or hand-keyed FK clips know nothing about the character's
//! surface, so a swing made on a slim rig cuts through a broad chest
//! (e.g. placeholder attack_1/attack_2 clips put the sword hand through the
//! torso). This pass gives the character simple collision proxies and
//! moves the arm out of the body per frame with two-bone IK, eased in
//! and out over time so the swing keeps its timing and arc.
//!
//! Proxies (all sized from the skinned rest mesh, never per character
//! tuning):
//! - torso: per spine segment (hips to neck), a "lozenge" of two
//!   capsules side by side fitted to the segment's cross-section (depth
//!   sets the radius, the extra width sets the side offset), so a flat
//!   chest is not modelled as a fat cylinder;
//! - head: a capsule from the head joint to the crown;
//! - thighs: one capsule each;
//! - arm parts that move: forearm and hand capsules per side, and a
//!   weapon capsule (blade) on the socket of the weapon hand.
//!
//! Pairs that already touch in the rest pose (a hand resting on the
//! thigh in an A-pose) keep that depth as slack, so only motion that
//! goes deeper than rest is changed.
//!
//! The fix, per clip and arm: for each frame, find the wrist offset that
//! clears every contact (iterating: push along the deepest contact's
//! normal, re-solve IK, measure again); then smooth the offsets over time
//! (a max-envelope with a smoothstep falloff of `ramp` seconds, so a
//! one-frame contact still eases in and out); then re-solve every frame
//! with the smoothed target. The shoulder stays put, the elbow stays in
//! its bend plane, and the hand keeps its world rotation, so a held
//! weapon keeps its angle and the hand's path keeps its shape, shifted
//! only where it would cut the body. Rounds repeat until clean (max 3).

mod body;
mod ik;
mod mesh;
mod report;
mod slack;
mod socket;
mod solve;

pub use body::{Arm, Body, Capsule};
pub use ik::{hinge_axis, solve_arm, solve_arm_turn};
pub use mesh::{clothing, seg_seg, skinned_mesh, skinned_rest, Clusters, SkinVert};
pub use report::{check_frames, dump_posed, ArmCheck, ArmReport, ClipContact};
pub use socket::{grip_search, insert_sockets, socket_frame};
pub use solve::{fix_clip, fix_clip_with_source, MAX_OFFSET_SPEED, MAX_TURN_SPEED};

/// Settings for the pass.
#[derive(Clone, Debug)]
pub struct Options {
    /// Hand that holds the weapon ("R", "L") or none.
    pub weapon_side: Option<String>,
    /// Blade length from the socket to the tip, in character heights
    /// (a reference sword: tip 1.5 units from the grip on a 2.0-unit hero).
    pub weapon_length: f64,
    /// Clip-name fragments (case-insensitive) that hold the weapon; other
    /// clips get no weapon proxy. Empty = every clip.
    pub weapon_clips: Vec<String>,
    /// Ease-in/out time of a correction, seconds.
    pub ramp: f64,
    /// Clearance kept between surfaces, in character heights.
    pub margin: f64,
    /// Leave a free arm (no blade in this clip) alone when its deepest mesh
    /// contact is shallower than this (character heights): a hand resting
    /// against a coat is not a limb through the body, and pushing it out
    /// splayed the arm. 0 = clear every contact.
    pub min_depth: f64,
    /// Width-first retargeting per body before contact clean-up (default on).
    pub refit_width: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            weapon_side: Some("R".into()),
            weapon_length: 0.75,
            weapon_clips: [
                "attack", "sword", "slash", "combo", "block", "parry", "stab", "swing",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            ramp: 0.15,
            margin: 0.004,
            min_depth: 0.0,
            refit_width: true,
        }
    }
}

impl Options {
    pub fn weapon_in(&self, clip: &str) -> bool {
        if self.weapon_side.is_none() {
            return false;
        }
        let low = clip.to_ascii_lowercase();
        self.weapon_clips.is_empty() || self.weapon_clips.iter().any(|f| low.contains(f.as_str()))
    }
}
