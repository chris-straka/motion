//! Readability at the game camera: can a viewer tell the move from his
//! silhouette, seen from where the player sees him?
//!
//! Per frame the skinned body and the blade (a ribbon from the guard to
//! the tip, held at the hand socket as the game holds it) are projected
//! through the game's follow camera and filled into a small mask. The
//! camera follows the root the way the game's does (the hips' path
//! across the floor, smoothed over 9 frames, once he travels 0.25 m;
//! `tools/bench/motion_video_render.py` `root_path`), so a lunge stays
//! framed instead of walking out of a fixed shot. The blade counts only
//! where it is visible: every body pixel carries its depth, and a blade
//! pixel behind the body (past a 2 cm grace for the fist round the
//! guard) is not sweep, not pose change, and its tip is not arc. From
//! the masks:
//!
//! - **Pose change**: the largest `1 - IoU` between a frame of the strike
//!   (load to settle) and the neutral silhouette (the clip's own first
//!   frame, the stance it starts from; or a given clip's): how far the
//!   strike's key pose reads from where it began.
//! - **Body change**: the same against the neutral body alone (no
//!   blade), from the windup's start to the settle: whether the body
//!   takes part or only a thin blade moves (the owner could not read
//!   HLH's attack_1, where only the arm and blade move). The score uses
//!   this one.
//! - **Sweep**: the screen area the blade covers over the strike (union of
//!   its masks), over the neutral body's own area.
//! - **Arc**: the blade tip's screen path over the strike, in neutral body
//!   heights, times how consistently it turns one way
//!   (`|sum of signed turns| / sum of turns`; 1 for an arc or a straight
//!   cut, near 0 for a wiggle).
//! - **Load hold**: the longest run of frames in the second before the
//!   strike's fastest frame whose silhouette changes less than
//!   `STILL_SHARE` from the frame before: a held anticipation. It counts
//!   in the score only as far as the arc after it reads.
//!
//! The score is the weakest of the body, sweep and arc parts (each
//! against `READ_FULL`, capped at 1): an average let a wide blade sweep
//! hide a body that never moves, which is the attack the owner could
//! not read. A held load is a bonus of at most a fifth on top of that
//! weakest part, never a requirement (the readable reference attacks
//! include some with no held load at all). The grip (below) then caps
//! the score the same way: a strike that reads but is not held scores
//! as a grip, not as an attack.
//!
//! - **Grip**: over the strike (load to settle), is the sword actually
//!   held? The handle's middle (the guard backed off by the guard
//!   distance) against the palm (the mean finger root, else the hand
//!   joint); the fingers' mean bend; and the fingertips' distance to
//!   the handle's line. A good grip puts the handle in the palm and
//!   bends the fingers round it; an open hand or a socket floating off
//!   the palm fails however clear the silhouette is. Combined by the
//!   weaker part again (handle in the palm, fingers closed).
//!
//! The strike's frames are `motion check`'s: the fastest tip frame, the
//! load before it ([`crate::motioncheck::strike_timing`]); the settle is
//! the first frame after the peak with the tip under 15% of its peak
//! speed. Deterministic: f64 arithmetic in a fixed order, `detmath` for
//! the trigonometry.

use crate::contact::{skinned_mesh, SkinVert};
use crate::detmath;
use crate::glb::{Affine, Document};
use crate::json::Json;
use crate::math::{Quat, Vec3};
use crate::motioncheck::{strike_timing, Subject, HOLD_SHARE};
use crate::rig::{Rig, Trs};

/// A frame whose silhouette changed less than this (1 - IoU with the
/// frame before) is held still.
pub const STILL_SHARE: f64 = 0.04;
/// Head joint height over stature: Andras is 2 m with his head joint at
/// 1.6836 m (HLH `character.rs`, `RigRest::stature`).
pub const HEAD_SHARE: f64 = 1.6836 / 2.0;
/// HLH's `tracks::held_pose` (Bevy, in the hand socket's space): the
/// blade's rotation (x, y, z, w) and the grip's middle.
pub const HELD_ROT: [f64; 4] = [0.30965, -0.67445, 0.00552, 0.67023];
pub const HELD_GRIP: [f64; 3] = [0.0, 0.0428, -0.0850];

/// How the blade sits in the hand socket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BladePose {
    /// As the game holds it (`HELD_ROT`, `HELD_GRIP`).
    Game,
    /// Straight out of the fist along the socket's +Y from its origin.
    Socket,
}

/// The camera and the blade. Defaults are HLH's follow camera in a
/// source level (`bevy/src/fx.rs` `camera_follow`, recorded by the game in
/// `proportions.rs`): focus 0.535 m above the body's centre, itself 1.0 m
/// above the feet; 3.0 m back at pitch -0.32 rad; 75 deg vertical field of
/// view; straight behind him. Masks are 480x270 (the game's 1280x720 at
/// three eighths).
#[derive(Clone, Debug)]
pub struct CamOptions {
    pub focus_m: f64,
    pub distance_m: f64,
    pub pitch_rad: f64,
    pub fov_deg: f64,
    /// Round him from behind, toward his right, degrees.
    pub yaw_deg: f64,
    pub width: usize,
    pub height: usize,
    /// Scale him so his head joint is at `HEAD_SHARE` of this (metres); 0
    /// keeps the GLB's units as metres.
    pub stature_m: f64,
    /// The blade from the grip: guard and tip, metres; its width.
    pub blade_from_m: f64,
    pub blade_to_m: f64,
    pub blade_width_m: f64,
    pub blade_pose: BladePose,
}

impl Default for CamOptions {
    fn default() -> Self {
        CamOptions {
            focus_m: 1.535,
            distance_m: 3.0,
            pitch_rad: -0.32,
            fov_deg: 75.0,
            yaw_deg: 0.0,
            width: 480,
            height: 270,
            stature_m: 2.0,
            // The props kit's sword (grip at its origin, guard 0.11 m and
            // tip 0.91 m down the blade).
            blade_from_m: 0.11,
            blade_to_m: 0.91,
            blade_width_m: 0.05,
            blade_pose: BladePose::Game,
        }
    }
}

/// A pinhole camera: screen x right, y down, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Points from the scene to the eye (the view looks along -back).
    pub back: Vec3,
    /// Focal length in pixels.
    pub f: f64,
    pub w: usize,
    pub h: usize,
}

impl Camera {
    /// At `eye` looking at `at`, Y up, `fov_deg` vertical.
    pub fn look(eye: Vec3, at: Vec3, fov_deg: f64, w: usize, h: usize) -> Camera {
        let back = (eye - at).normalized();
        let right = Vec3::new(0.0, 1.0, 0.0).cross(back).normalized();
        let up = back.cross(right);
        let (s, c) = detmath::sin_cos(fov_deg.to_radians() * 0.5);
        Camera {
            eye,
            right,
            up,
            back,
            f: h as f64 * 0.5 * c / s,
            w,
            h,
        }
    }

    /// The game's follow camera on a character standing at the origin
    /// facing `forward` (horizontal), in units of `unit` per metre.
    pub fn game(forward: Vec3, opts: &CamOptions, unit: f64) -> Camera {
        let upv = Vec3::new(0.0, 1.0, 0.0);
        let his_right = forward.cross(upv);
        let (sy, cy) = detmath::sin_cos(opts.yaw_deg.to_radians());
        let look = forward.scale(cy) + his_right.scale(sy);
        let focus = upv.scale(opts.focus_m * unit);
        let (sp, cp) = detmath::sin_cos(-opts.pitch_rad);
        let d = opts.distance_m * unit;
        let eye = focus - look.scale(d * cp) + upv.scale(d * sp);
        Camera::look(eye, focus, opts.fov_deg, opts.width, opts.height)
    }

    /// Screen position and depth; `None` behind the eye.
    pub fn project(&self, p: Vec3) -> Option<(f64, f64, f64)> {
        let v = p - self.eye;
        let z = -v.dot(self.back);
        if z < 1e-6 {
            return None;
        }
        Some((
            self.w as f64 * 0.5 + self.f * v.dot(self.right) / z,
            self.h as f64 * 0.5 - self.f * v.dot(self.up) / z,
            z,
        ))
    }
}

/// A coverage mask: a pixel is in when its centre is inside a shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub px: Vec<bool>,
}

fn edge(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Mask {
        Mask {
            w,
            h,
            px: vec![false; w * h],
        }
    }

    pub fn area(&self) -> usize {
        self.px.iter().filter(|&&b| b).count()
    }

    /// Fill a triangle (either winding): every pixel whose centre is
    /// inside or on an edge.
    pub fn fill_tri(&mut self, a: (f64, f64), b: (f64, f64), c: (f64, f64)) {
        let area = edge(a, b, c);
        if area.abs() < 1e-12 {
            return;
        }
        let (lx, hx) = (a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0));
        let (ly, hy) = (a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1));
        let x0 = ((lx - 0.5).ceil() as i64).max(0);
        let x1 = ((hx - 0.5).floor() as i64).min(self.w as i64 - 1);
        let y0 = ((ly - 0.5).ceil() as i64).max(0);
        let y1 = ((hy - 0.5).floor() as i64).min(self.h as i64 - 1);
        if x0 > x1 || y0 > y1 {
            return;
        }
        for y in y0..=y1 {
            let py = y as f64 + 0.5;
            for x in x0..=x1 {
                let p = (x as f64 + 0.5, py);
                let (e0, e1, e2) = (edge(a, b, p), edge(b, c, p), edge(c, a, p));
                let inside = if area > 0.0 {
                    e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0
                } else {
                    e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0
                };
                if inside {
                    self.px[y as usize * self.w + x as usize] = true;
                }
            }
        }
    }

    /// Fill a thick segment: half width `ra` at `a`, `rb` at `b` (pixels).
    /// A segment seen end on is a square of half width the larger.
    pub fn fill_segment(&mut self, a: (f64, f64), b: (f64, f64), ra: f64, rb: f64) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            let r = ra.max(rb);
            let (p, q) = ((a.0 - r, a.1 - r), (a.0 + r, a.1 + r));
            self.fill_tri(p, (q.0, p.1), q);
            self.fill_tri(p, q, (p.0, q.1));
            return;
        }
        let (nx, ny) = (-dy / len, dx / len);
        let p0 = (a.0 + nx * ra, a.1 + ny * ra);
        let p1 = (b.0 + nx * rb, b.1 + ny * rb);
        let p2 = (b.0 - nx * rb, b.1 - ny * rb);
        let p3 = (a.0 - nx * ra, a.1 - ny * ra);
        self.fill_tri(p0, p1, p2);
        self.fill_tri(p0, p2, p3);
    }

    /// Fill a triangle and record each covered pixel's interpolated
    /// depth in `depth` (nearest wins; `depth` starts at infinity).
    pub fn fill_tri_depth(
        &mut self,
        a: (f64, f64),
        b: (f64, f64),
        c: (f64, f64),
        za: f64,
        zb: f64,
        zc: f64,
        depth: &mut [f64],
    ) {
        let area = edge(a, b, c);
        if area.abs() < 1e-12 {
            return;
        }
        let (lx, hx) = (a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0));
        let (ly, hy) = (a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1));
        let x0 = ((lx - 0.5).ceil() as i64).max(0);
        let x1 = ((hx - 0.5).floor() as i64).min(self.w as i64 - 1);
        let y0 = ((ly - 0.5).ceil() as i64).max(0);
        let y1 = ((hy - 0.5).floor() as i64).min(self.h as i64 - 1);
        if x0 > x1 || y0 > y1 {
            return;
        }
        for y in y0..=y1 {
            let py = y as f64 + 0.5;
            for x in x0..=x1 {
                let p = (x as f64 + 0.5, py);
                let (e0, e1, e2) = (edge(a, b, p), edge(b, c, p), edge(c, a, p));
                let inside = if area > 0.0 {
                    e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0
                } else {
                    e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0
                };
                if inside {
                    let i = y as usize * self.w + x as usize;
                    let z = (e1 * za + e2 * zb + e0 * zc) / area;
                    if z < depth[i] {
                        depth[i] = z;
                        self.px[i] = true;
                    }
                }
            }
        }
    }

    /// Fill a thick segment with depth: `za` at `a`, `zb` at `b`.
    pub fn fill_segment_depth(
        &mut self,
        a: (f64, f64),
        b: (f64, f64),
        ra: f64,
        rb: f64,
        za: f64,
        zb: f64,
        depth: &mut [f64],
    ) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            let r = ra.max(rb);
            let (p, q) = ((a.0 - r, a.1 - r), (a.0 + r, a.1 + r));
            self.fill_tri_depth(p, (q.0, p.1), q, za, za, za, depth);
            self.fill_tri_depth(p, q, (p.0, q.1), za, za, za, depth);
            return;
        }
        let (nx, ny) = (-dy / len, dx / len);
        let p0 = (a.0 + nx * ra, a.1 + ny * ra);
        let p1 = (b.0 + nx * rb, b.1 + ny * rb);
        let p2 = (b.0 - nx * rb, b.1 - ny * rb);
        let p3 = (a.0 - nx * ra, a.1 - ny * ra);
        self.fill_tri_depth(p0, p1, p2, za, zb, zb, depth);
        self.fill_tri_depth(p0, p2, p3, za, zb, za, depth);
    }

    /// Drop the pixels that sit behind another surface: a pixel stays
    /// only where its own depth is not past `other`'s by more than
    /// `bias` (the fist round a held guard is that deep).
    pub fn cull_behind(&mut self, own: &[f64], other: &[f64], bias: f64) {
        for i in 0..self.px.len() {
            if self.px[i] && own[i] > other[i] + bias {
                self.px[i] = false;
            }
        }
    }

    pub fn union_with(&mut self, o: &Mask) {
        for (a, b) in self.px.iter_mut().zip(&o.px) {
            *a |= *b;
        }
    }

    /// Intersection over union (1 when both are empty).
    pub fn iou(&self, o: &Mask) -> f64 {
        let (mut i, mut u) = (0usize, 0usize);
        for (a, b) in self.px.iter().zip(&o.px) {
            i += (*a && *b) as usize;
            u += (*a || *b) as usize;
        }
        if u == 0 {
            1.0
        } else {
            i as f64 / u as f64
        }
    }

    /// Rows from the first to the last covered one (0 when empty).
    pub fn rows(&self) -> usize {
        let mut lo = None;
        let mut hi = 0;
        for y in 0..self.h {
            if self.px[y * self.w..(y + 1) * self.w].iter().any(|&b| b) {
                lo.get_or_insert(y);
                hi = y;
            }
        }
        lo.map(|l| hi - l + 1).unwrap_or(0)
    }

    /// Binary PGM: body black, `blade` grey, the rest white.
    pub fn to_pgm(&self, blade: Option<&Mask>) -> Vec<u8> {
        let mut out = format!("P5\n{} {}\n255\n", self.w, self.h).into_bytes();
        for i in 0..self.px.len() {
            out.push(if blade.is_some_and(|b| b.px[i]) {
                128
            } else if self.px[i] {
                0
            } else {
                255
            });
        }
        out
    }
}

/// Screen path of a point: its length (pixels), how consistently it turns
/// one way (`|sum of signed turns| / sum of |turns|`, each turn weighted
/// by the shorter of its two steps; 1 when it turns under 10 deg a step
/// on average), and the total turn (degrees, unweighted). Steps under
/// half a pixel are skipped.
pub fn path_arc(pts: &[(f64, f64)]) -> (f64, f64, f64) {
    let mut kept: Vec<(f64, f64)> = Vec::new();
    let mut len = 0.0;
    for &p in pts {
        match kept.last() {
            None => kept.push(p),
            Some(&q) => {
                let d = ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt();
                if d >= 0.5 {
                    len += d;
                    kept.push(p);
                }
            }
        }
    }
    let (mut signed, mut total, mut weight, mut turn) = (0.0, 0.0, 0.0, 0.0);
    for w in kept.windows(3) {
        let a = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let b = (w[2].0 - w[1].0, w[2].1 - w[1].1);
        let (la, lb) = (
            (a.0 * a.0 + a.1 * a.1).sqrt(),
            (b.0 * b.0 + b.1 * b.1).sqrt(),
        );
        let cos = ((a.0 * b.0 + a.1 * b.1) / (la * lb)).clamp(-1.0, 1.0);
        let ang = detmath::acos(cos).to_degrees();
        let cross = a.0 * b.1 - a.1 * b.0;
        // A turn counts by the shorter of its two steps: the tip's
        // jitter where it slows at either end is not a wiggle.
        let wgt = la.min(lb);
        signed += if cross >= 0.0 { ang * wgt } else { -ang * wgt };
        total += ang * wgt;
        weight += wgt;
        turn += ang;
    }
    // Under 10 deg a step on average (weighted): it barely turns.
    let consistency = if total < 10.0 * weight || total <= 0.0 {
        1.0
    } else {
        signed.abs() / total
    };
    (len, consistency, turn)
}

/// The skinned mesh, ready to pose.
pub struct Mesh {
    verts: Vec<SkinVert>,
    tris: Vec<[usize; 3]>,
    rest_inv: Vec<Affine>,
}

impl Mesh {
    pub fn build(doc: &Document, rig: &Rig) -> Result<Mesh, String> {
        let (verts, tris) = skinned_mesh(doc, rig)?;
        let rest_inv = rig
            .rest_world
            .iter()
            .map(|m| m.inverse().unwrap_or(Affine::IDENTITY))
            .collect();
        Ok(Mesh {
            verts,
            tris,
            rest_inv,
        })
    }

    pub fn triangles(&self) -> usize {
        self.tris.len()
    }

    fn posed(&self, world: &[Affine]) -> Vec<Vec3> {
        let skin: Vec<Affine> = world
            .iter()
            .zip(&self.rest_inv)
            .map(|(w, r)| w.mul(r))
            .collect();
        self.verts
            .iter()
            .map(|v| {
                let mut acc = Vec3::ZERO;
                let mut total = 0.0;
                for &(j, w) in &v.inf {
                    acc = acc + skin[j].apply(v.p).scale(w);
                    total += w;
                }
                if total > 0.0 {
                    acc.scale(1.0 / total)
                } else {
                    v.p
                }
            })
            .collect()
    }

    fn posed_subset(&self, world: &[Affine], vert_ids: &[usize]) -> Vec<Vec3> {
        let skin: Vec<Affine> = world
            .iter()
            .zip(&self.rest_inv)
            .map(|(w, r)| w.mul(r))
            .collect();
        vert_ids
            .iter()
            .map(|&vi| {
                let v = &self.verts[vi];
                let mut acc = Vec3::ZERO;
                let mut total = 0.0;
                for &(j, w) in &v.inf {
                    acc = acc + skin[j].apply(v.p).scale(w);
                    total += w;
                }
                if total > 0.0 {
                    acc.scale(1.0 / total)
                } else {
                    v.p
                }
            })
            .collect()
    }
}

/// Readability of one clip at the game camera.
#[derive(Clone, Debug, Default)]
pub struct ReadCam {
    pub peak_frame: usize,
    pub load_frame: usize,
    pub settle_frame: usize,
    /// The frame of the strike furthest from neutral, and how far.
    pub key_frame: usize,
    pub pose_change: f64,
    /// The same for the body alone (no blade), from the windup's start to
    /// the settle, and its frame: does the body take part, or only a thin
    /// blade?
    pub body_change: f64,
    pub body_frame: usize,
    pub sweep: f64,
    /// Tip path in neutral body heights, its turn consistency and total
    /// turn (degrees), and their product.
    pub arc_len: f64,
    pub arc_consistency: f64,
    pub arc_turn_deg: f64,
    pub arc: f64,
    pub load_hold_frames: usize,
    /// Neutral body height on screen (pixels) and area.
    pub body_px: usize,
    pub body_area: usize,
    pub blade: bool,
    /// 0..1: the silhouette alone (body, sweep, arc and the held load,
    /// before the grip caps it).
    pub read_score: f64,
    /// The grip over the strike, when the rig holds a blade.
    pub grip: GripScore,
    /// Whether a blade (and so a grip) was there to judge.
    pub grip_applies: bool,
    /// 0..1: the weaker of the silhouette and the grip.
    pub score: f64,
}

/// Is the sword held, over one strike? Distances are metres at the
/// game's stature (GLB units divided out), so one threshold fits every
/// character.
#[derive(Clone, Debug)]
pub struct GripScore {
    /// 0..1, the weaker of the handle and finger parts.
    pub score: f64,
    /// Handle's middle to the palm, median over the strike.
    pub handle_m: f64,
    /// Palm to the handle's line, median over the strike.
    pub axis_m: f64,
    /// Mean finger turn from the rest pose, degrees (0 for a hand
    /// the clip never closes).
    pub curl_deg: f64,
    /// Fingertips to the handle's line, median over the strike.
    pub tip_m: f64,
    /// The rig has finger chains long enough to bend measurably.
    pub fingers: bool,
    /// Frames judged (load to settle).
    pub frames: usize,
    /// Minimum point-to-triangle distance from the handle's middle to the
    /// weapon hand's palm skin triangles, median over the strike.
    pub palm_surface_m: f64,
    /// Fraction (`0.0..=1.0`) of angular sectors around the handle axis
    /// covered by weapon-hand + finger skinned triangles, median over the
    /// strike.
    pub skin_wrap: f64,
    /// `true` when the weapon hand has skinned mesh triangles (`score`
    /// computed via [`skin_grip_of`]).
    pub skin_based: bool,
}

impl Default for GripScore {
    fn default() -> Self {
        GripScore {
            score: 1.0,
            handle_m: 0.0,
            axis_m: 0.0,
            curl_deg: 0.0,
            tip_m: 0.0,
            fingers: false,
            frames: 0,
            palm_surface_m: 0.0,
            skin_wrap: 0.0,
            skin_based: false,
        }
    }
}

/// Handle axis within this distance of nearest palm skin surface gets full
/// distance score (metres at the game's 2 m stature).
pub const SKIN_PALM_GOOD_M: f64 = 0.040;
/// Handle axis at or past this distance from palm skin scores 0.
pub const SKIN_PALM_BAD_M: f64 = 0.085;
/// Circumference coverage at or below this share (flat palm) scores 0.
pub const SKIN_WRAP_BAD: f64 = 0.40;
/// Circumference coverage at or above this share (wrapped fist) scores 1.
pub const SKIN_WRAP_GOOD: f64 = 0.85;
/// Axial half-band along the handle axis for wrap sampling (metres).
pub const SKIN_WRAP_HALF_LEN_M: f64 = 0.055;
/// Minimum radial distance from handle axis for wrap sampling (metres).
pub const SKIN_WRAP_MIN_R_M: f64 = 0.004;
/// Maximum radial reach from handle axis for wrap sampling (metres).
pub const SKIN_WRAP_MAX_R_M: f64 = 0.045;
/// Number of equal angular sectors around the handle axis (15 deg each).
pub const SKIN_WRAP_BINS: usize = 24;

/// Skin-based grip score from palm-surface distance, circumference wrap, and
/// finger curl (`None` when no measurable finger chains exist).
pub fn skin_grip_of(palm_surface_m: f64, skin_wrap: f64, curl_deg: Option<f64>) -> f64 {
    let palm_span = SKIN_PALM_BAD_M - SKIN_PALM_GOOD_M;
    let palm_part = (1.0 - (palm_surface_m - SKIN_PALM_GOOD_M) / palm_span).clamp(0.0, 1.0);
    let wrap_span = SKIN_WRAP_GOOD - SKIN_WRAP_BAD;
    let wrap_part = ((skin_wrap - SKIN_WRAP_BAD) / wrap_span).clamp(0.0, 1.0);
    let base = palm_part.min(wrap_part);
    match curl_deg {
        Some(c) => base.min((c / GRIP_CURL_FULL_DEG).clamp(0.0, 1.0)),
        None => base,
    }
}

/// Handle distance in the palm at this many metres; past
/// `GRIP_HANDLE_BAD_M` the handle is not in the hand at all. A palm
/// grip (`attack::grip::palm_hold`) puts the handle's middle about
/// 3.3 cm from the knuckle line (a handle's and a finger's radius,
/// a little down the fingers), so the good bound sits just past it.
pub const GRIP_HANDLE_GOOD_M: f64 = 0.045;
pub const GRIP_HANDLE_BAD_M: f64 = 0.115;
/// Mean finger turn of a closed grip, degrees: a fist turns each
/// joint roughly 30-60 deg from rest (`attack::grip`'s fist table);
/// a hand the clip never closes turns 0.
pub const GRIP_CURL_FULL_DEG: f64 = 35.0;
/// Mean finger turn of a closed distal-tip mitten chain (`palm_len > 2 *
/// finger_len`). The measure's own value, kept apart from the solver's
/// `attack::grip::DISTAL_MITTEN_CURL`: it is the full-curl value the
/// blind-judged measure had before 2026-11-16 (35 deg, as for any hand).
/// Change it only after a blind same-frame judge backs the change.
pub const GRIP_MITTEN_CURL_FULL_DEG: f64 = 35.0;
/// Where the measure puts a distal mitten's palm, as a share of hand
/// head -> finger root. The measure's own value, kept apart from the
/// solver's `attack::grip::DISTAL_MITTEN_PALM_SHARE`: 1.0 is the finger
/// root, the anchor the blind-judged measure had before 2026-11-16.
/// Change it only after a blind same-frame judge backs the change.
pub const GRIP_MITTEN_PALM_SHARE: f64 = 1.0;

/// The grip's score from the handle's distance to the palm and the
/// fingers' bend (`None` when the rig has no measurable fingers: the
/// handle alone is judged, and `GripScore::fingers` says so). Each
/// part 0..1, combined by the weaker.
pub fn grip_of(handle_m: f64, curl_deg: Option<f64>) -> f64 {
    grip_of_full(handle_m, curl_deg, GRIP_CURL_FULL_DEG)
}

fn grip_of_full(handle_m: f64, curl_deg: Option<f64>, full_deg: f64) -> f64 {
    let handle = (1.0 - (handle_m - GRIP_HANDLE_GOOD_M) / (GRIP_HANDLE_BAD_M - GRIP_HANDLE_GOOD_M))
        .clamp(0.0, 1.0);
    match curl_deg {
        Some(c) => handle.min((c / full_deg).clamp(0.0, 1.0)),
        None => handle,
    }
}

/// One frame's grip measures: the handle's distance to the palm, the
/// palm's distance to the handle's line and the fingertips' distance
/// to the handle's line. `palm` is the mean finger root (or the hand
/// joint); `grip` the handle's middle; `dir` the blade's direction;
/// `chains` are finger joint positions, hand first.
pub fn grip_frame(palm: Vec3, grip: Vec3, dir: Vec3, chains: &[Vec<Vec3>]) -> (f64, f64, f64) {
    let handle = (grip - palm).length();
    let along = (palm - grip).dot(dir);
    let axis = ((palm - grip) - dir.scale(along)).length();
    let mut tips = Vec::new();
    for ch in chains {
        if ch.len() >= 2 {
            let tip = ch[ch.len() - 1] + (ch[ch.len() - 1] - ch[ch.len() - 2]);
            let t = (tip - grip).dot(dir);
            tips.push(((tip - grip) - dir.scale(t)).length());
        }
    }
    let tip = if tips.is_empty() {
        0.0
    } else {
        tips.iter().sum::<f64>() / tips.len() as f64
    };
    (handle, axis, tip)
}

/// A finger joint's turn from its rest pose, degrees: its rotation
/// relative to its parent now, against the same at rest. Joint
/// positions cannot say this (a rig may rest with curved fingers --
/// Lysan's rest hand already bends 44 deg joint to joint, so an open
/// authored hand and a closed one read the same by position), the
/// joint's own turn can: a hand the clip never closes turns 0.
pub fn joint_turn_deg(
    rest_parent: Quat,
    rest_node: Quat,
    posed_parent: Quat,
    posed_node: Quat,
) -> f64 {
    let rest = rest_parent.conj().mul(rest_node);
    let posed = posed_parent.conj().mul(posed_node);
    rest.angle_to(posed).to_degrees()
}

/// The weapon hand's mean finger turn from rest over one posed frame
/// (degrees, `None` when no chain has a joint to turn): every finger
/// joint of every chain, hand first in each chain.
pub fn finger_turn_deg(rig: &Rig, world: &[Affine], chains: &[Vec<usize>]) -> Option<f64> {
    let mut sum = 0.0;
    let mut count = 0usize;
    for ch in chains {
        for w in ch.windows(2) {
            sum += joint_turn_deg(
                rig.rest_world[w[0]].rotation(),
                rig.rest_world[w[1]].rotation(),
                world[w[0]].rotation(),
                world[w[1]].rotation(),
            );
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f64)
}

fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let m = s.len() / 2;
    if s.len() % 2 == 1 {
        s[m]
    } else {
        0.5 * (s[m - 1] + s[m])
    }
}

/// Exact squared distance from point `p` to triangle `(a, b, c)`, guarding
/// degenerate triangles cleanly without NaN or division by zero.
fn point_tri_dist_sq(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f64 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length_sq();
    }

    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length_sq();
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let denom = d1 - d3;
        let v = if denom > 1e-18 { d1 / denom } else { 0.0 };
        return (p - (a + ab.scale(v))).length_sq();
    }

    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length_sq();
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let denom = d2 - d6;
        let w = if denom > 1e-18 { d2 / denom } else { 0.0 };
        return (p - (a + ac.scale(w))).length_sq();
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let denom = (d4 - d3) + (d5 - d6);
        let w = if denom > 1e-18 {
            (d4 - d3) / denom
        } else {
            0.0
        };
        return (p - (b + (c - b).scale(w))).length_sq();
    }

    let sum = va + vb + vc;
    if sum <= 1e-18 {
        let seg_sq = |u: Vec3, v: Vec3| -> f64 {
            let e = v - u;
            let len_sq = e.length_sq();
            let t = if len_sq > 1e-18 {
                ((p - u).dot(e) / len_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (p - (u + e.scale(t))).length_sq()
        };
        return seg_sq(a, b).min(seg_sq(b, c)).min(seg_sq(c, a));
    }
    let denom = 1.0 / sum;
    let v = vb * denom;
    let w = vc * denom;
    (p - (a + ab.scale(v) + ac.scale(w))).length_sq()
}

/// The silhouette score: the weakest of the body, sweep and arc parts
/// (each against `READ_FULL`, capped at 1), with a held load worth at
/// most a fifth on top of that weakest part, and only as far as the
/// arc after it reads (a still pose is not a held load).
pub fn score_of(body: f64, sweep: f64, arc: f64, hold: usize) -> f64 {
    let p = |v: f64, i: usize| (v / READ_FULL[i]).min(1.0);
    let weak = p(body, 0).min(p(sweep, 1)).min(p(arc, 2));
    weak * (0.8 + 0.2 * p(hold as f64, 3) * p(arc, 2))
}

/// What a clearly readable attack reaches in each part (calibrated on
/// HLH's attacks against its idle: `docs/motion-check.md`).
pub const READ_FULL: [f64; 4] = [0.7, 0.8, 2.0, 4.0];

impl ReadCam {
    pub fn to_json(&self) -> Json {
        let r = |v: f64, p: i32| {
            let k = 10f64.powi(p);
            (v * k).round() / k
        };
        Json::obj(vec![
            ("peak_frame", Json::num(self.peak_frame as f64)),
            ("load_frame", Json::num(self.load_frame as f64)),
            ("settle_frame", Json::num(self.settle_frame as f64)),
            ("key_frame", Json::num(self.key_frame as f64)),
            ("pose_change", Json::num(r(self.pose_change, 3))),
            ("body_change", Json::num(r(self.body_change, 3))),
            ("body_frame", Json::num(self.body_frame as f64)),
            ("sweep", Json::num(r(self.sweep, 3))),
            ("arc_len", Json::num(r(self.arc_len, 3))),
            ("arc_consistency", Json::num(r(self.arc_consistency, 3))),
            ("arc_turn_deg", Json::num(r(self.arc_turn_deg, 1))),
            ("arc", Json::num(r(self.arc, 3))),
            ("load_hold_frames", Json::num(self.load_hold_frames as f64)),
            ("body_px", Json::num(self.body_px as f64)),
            ("body_area", Json::num(self.body_area as f64)),
            ("blade", Json::Bool(self.blade)),
            ("read_score", Json::num(r(self.read_score, 3))),
            (
                "grip",
                if self.grip_applies {
                    Json::obj(vec![
                        ("score", Json::num(r(self.grip.score, 3))),
                        ("handle_cm", Json::num(r(self.grip.handle_m * 100.0, 1))),
                        ("axis_cm", Json::num(r(self.grip.axis_m * 100.0, 1))),
                        ("curl_deg", Json::num(r(self.grip.curl_deg, 1))),
                        ("tip_cm", Json::num(r(self.grip.tip_m * 100.0, 1))),
                        ("fingers", Json::Bool(self.grip.fingers)),
                        ("frames", Json::num(self.grip.frames as f64)),
                        (
                            "palm_surface_cm",
                            Json::num(r(self.grip.palm_surface_m * 100.0, 1)),
                        ),
                        ("skin_wrap", Json::num(r(self.grip.skin_wrap, 3))),
                        ("skin_based", Json::Bool(self.grip.skin_based)),
                        ("ok", Json::Bool(self.grip.score >= 0.5)),
                    ])
                } else {
                    Json::Null
                },
            ),
            ("score", Json::num(r(self.score, 3))),
        ])
    }

    pub fn line(&self) -> String {
        let grip = if self.grip_applies {
            if self.grip.skin_based {
                format!(
                    ", grip {:.2} (palm {:.1} cm, wrap {:.0}%, handle {:.1} cm, curl {:.0} deg{})",
                    self.grip.score,
                    self.grip.palm_surface_m * 100.0,
                    self.grip.skin_wrap * 100.0,
                    self.grip.handle_m * 100.0,
                    self.grip.curl_deg,
                    if self.grip.fingers {
                        ""
                    } else {
                        ", no fingers"
                    }
                )
            } else {
                format!(
                    ", grip {:.2} (handle {:.1} cm, curl {:.0} deg{})",
                    self.grip.score,
                    self.grip.handle_m * 100.0,
                    self.grip.curl_deg,
                    if self.grip.fingers {
                        ""
                    } else {
                        ", no fingers"
                    }
                )
            }
        } else {
            String::new()
        };
        format!(
            "{:<18} read at game camera {:.2}: body {:.2} (f{}), pose {:.2} (f{}), sweep {:.2}, arc {:.2} ({:.2} heights x {:.2}), load held {} fr; strike f{}-{}-{}{}\n",
            "",
            self.score,
            self.body_change,
            self.body_frame,
            self.pose_change,
            self.key_frame,
            self.sweep,
            self.arc,
            self.arc_len,
            self.arc_consistency,
            self.load_hold_frames,
            self.load_frame,
            self.peak_frame,
            self.settle_frame,
            grip
        )
    }
}

/// Metres to the GLB's units: his head joint goes to `HEAD_SHARE` of the
/// stature.
pub fn unit_of(subject: &Subject, opts: &CamOptions) -> f64 {
    if opts.stature_m <= 0.0 {
        return 1.0;
    }
    match subject.map.node_of("DEF-spine.006") {
        Some(h) if subject.rig.head(h).y > 1e-6 => {
            subject.rig.head(h).y / (opts.stature_m * HEAD_SHARE)
        }
        _ => 1.0,
    }
}

/// The blade's guard and tip in the hand socket's space (GLB units).
fn blade_local(opts: &CamOptions, unit: f64) -> (Vec3, Vec3) {
    let (grip, dir) = match opts.blade_pose {
        BladePose::Game => {
            let q = Quat::new(HELD_ROT[3], HELD_ROT[0], HELD_ROT[1], HELD_ROT[2]).normalized();
            (
                Vec3::new(HELD_GRIP[0], HELD_GRIP[1], HELD_GRIP[2]),
                q.rotate_vec(Vec3::new(0.0, 1.0, 0.0)),
            )
        }
        BladePose::Socket => (Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)),
    };
    (
        (grip + dir.scale(opts.blade_from_m)).scale(unit),
        (grip + dir.scale(opts.blade_to_m)).scale(unit),
    )
}

/// One frame's silhouettes at the camera, and the tip on screen.
pub struct Shot {
    pub body: Mask,
    pub blade: Option<Mask>,
    pub tip: Option<(f64, f64)>,
}

/// Everything needed to film a character.
pub struct Stage {
    pub cam: Camera,
    pub unit: f64,
    mesh: Mesh,
    socket: Option<usize>,
    hand: Option<usize>,
    hips: Option<usize>,
    blade: (Vec3, Vec3),
    width: f64,
    /// Guard distance from the grip, metres (the handle's middle is
    /// the guard backed off by this along the blade).
    from_m: f64,
    /// Finger roots `(node, share_from_hand)` defining the palm pocket and
    /// chains (hand first) of the weapon hand, for the grip.
    palm_roots: Vec<(usize, f64)>,
    finger_chains: Vec<Vec<usize>>,
    distal_mitten: bool,
    /// Compact vertex indices (into `mesh.verts`) needed by `hand_tris`, plus
    /// weapon-hand and palm triangles indexed into `hand_verts`.
    hand_verts: Vec<usize>,
    hand_tris: Vec<[usize; 3]>,
    palm_tris: Vec<[usize; 3]>,
}

/// How far he must travel across the floor (metres at the game's
/// stature) before the camera follows his root, and the smoothing
/// window in frames: the renderer's own numbers
/// (`motion_video_render.py` `root_path`).
pub const FOLLOW_MIN_M: f64 = 0.25;
pub const FOLLOW_WINDOW: usize = 9;

/// Per-frame camera offsets that follow the root: the hips' horizontal
/// travel from the first frame, smoothed over `FOLLOW_WINDOW` frames
/// and re-zeroed at the first, or all zero when he never travels
/// `FOLLOW_MIN_M`. Pure, so the following itself has a known answer.
pub fn follow_offsets(hips: &[Vec3], unit: f64) -> Vec<Vec3> {
    let n = hips.len();
    if n == 0 {
        return Vec::new();
    }
    let base = Vec3::new(hips[0].x, 0.0, hips[0].z);
    let raw: Vec<Vec3> = hips
        .iter()
        .map(|h| Vec3::new(h.x, 0.0, h.z) - base)
        .collect();
    let furthest = raw.iter().map(|o| o.length()).fold(0.0, f64::max);
    if unit <= 0.0 || furthest < FOLLOW_MIN_M * unit {
        return vec![Vec3::ZERO; n];
    }
    let half = FOLLOW_WINDOW / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (i.saturating_sub(half), (i + half + 1).min(n));
        let mut sum = Vec3::ZERO;
        for o in &raw[a..b] {
            sum = sum + *o;
        }
        out.push(sum.scale(1.0 / (b - a) as f64));
    }
    let first = out[0];
    out.iter().map(|o| *o - first).collect()
}

impl Stage {
    pub fn new(doc: &Document, subject: &Subject, opts: &CamOptions) -> Result<Stage, String> {
        let unit = unit_of(subject, opts);
        let side = subject
            .opts
            .weapon_side
            .clone()
            .unwrap_or_else(|| "R".to_string());
        let hand = subject.map.node_of(&format!("DEF-hand.{side}"));
        let socket = hand.and_then(|h| {
            subject.rig.children[h]
                .iter()
                .copied()
                .find(|&c| subject.rig.names[c] == format!("Socket_Hand_{side}"))
        });
        let socket = if subject.opts.weapon_side.is_some() {
            socket
        } else {
            None
        };
        // The grip's fingers: the canonical chains when the mapper
        // found them, else the hand's own child chains (a single
        // three-bone finger, as the game's rig has). Palm bones and
        // the socket are not fingers.
        let mut palm_roots: Vec<(usize, f64)> = Vec::new();
        let mut finger_chains: Vec<Vec<usize>> = Vec::new();
        // Per palm root: is it a distal mitten (hand bone over twice the
        // finger chain)? Kept apart from the share so the measure's
        // mitten curl follows the rig's shape, not the anchor's value.
        let mut mitten_roots: Vec<bool> = Vec::new();
        if let Some(h) = hand {
            let is_mitten = |c0: usize, c_last: usize| -> bool {
                let palm_len = (subject.rig.head(c0) - subject.rig.head(h)).length();
                let finger_len = (subject.rig.head(c_last) - subject.rig.head(c0)).length();
                finger_len > 1e-9 && palm_len > 2.0 * finger_len
            };
            let root_share = |m: bool| -> f64 {
                if m {
                    GRIP_MITTEN_PALM_SHARE
                } else {
                    1.0
                }
            };
            for finger in ["f_index", "f_middle", "f_ring", "f_pinky", "thumb"] {
                let chain: Vec<usize> = (1..=3)
                    .filter_map(|seg| {
                        subject
                            .map
                            .node_of(&format!("DEF-{}.{:02}.{}", finger, seg, side))
                    })
                    .collect();
                if chain.len() >= 2 {
                    let m = is_mitten(chain[0], *chain.last().unwrap());
                    palm_roots.push((chain[0], root_share(m)));
                    mitten_roots.push(m);
                    let mut full = vec![h];
                    full.extend(chain);
                    finger_chains.push(full);
                } else if let Some(&root) = chain.first() {
                    palm_roots.push((root, 1.0));
                    mitten_roots.push(false);
                }
            }
            if finger_chains.is_empty() {
                let skip = |n: &str| {
                    n.starts_with("Socket_")
                        || n.starts_with("DEF-palm.")
                        || crate::helpers::is_helper(n)
                };
                for &c in &subject.rig.children[h] {
                    if skip(&subject.rig.names[c]) {
                        continue;
                    }
                    let mut chain = vec![h, c];
                    let mut cur = c;
                    loop {
                        let next = subject.rig.children[cur]
                            .iter()
                            .copied()
                            .find(|&k| !skip(&subject.rig.names[k]));
                        match next {
                            Some(k) => {
                                chain.push(k);
                                cur = k;
                            }
                            None => break,
                        }
                    }
                    if chain.len() >= 3 {
                        let m = is_mitten(c, *chain.last().unwrap());
                        palm_roots.push((c, root_share(m)));
                        mitten_roots.push(m);
                        finger_chains.push(chain);
                    }
                }
            }
        }
        let distal_mitten = !mitten_roots.is_empty() && mitten_roots.iter().all(|&m| m);
        let mesh = Mesh::build(doc, &subject.rig)?;
        let n_nodes = subject.rig.names.len();
        let mut in_hand = vec![false; n_nodes];
        let mut in_palm = vec![false; n_nodes];
        if let Some(h) = hand {
            let mut stack = vec![h];
            in_hand[h] = true;
            in_palm[h] = true;
            while let Some(cur) = stack.pop() {
                for &c in &subject.rig.children[cur] {
                    let name = &subject.rig.names[c];
                    if name.starts_with("Socket_") || crate::helpers::is_helper(name) {
                        continue;
                    }
                    if !in_hand[c] {
                        in_hand[c] = true;
                        if name.starts_with("DEF-palm.") {
                            in_palm[c] = true;
                        }
                        stack.push(c);
                    }
                }
            }
        }
        let is_hand: Vec<bool> = mesh
            .verts
            .iter()
            .map(|v| {
                let w: f64 = v
                    .inf
                    .iter()
                    .filter(|&&(j, _)| in_hand.get(j).copied().unwrap_or(false))
                    .map(|&(_, w)| w)
                    .sum();
                w >= 0.5
            })
            .collect();
        let is_palm: Vec<bool> = mesh
            .verts
            .iter()
            .map(|v| {
                let mut w_palm = 0.0;
                let mut w_finger = 0.0;
                for &(j, w) in &v.inf {
                    if in_palm.get(j).copied().unwrap_or(false) {
                        w_palm += w;
                    } else if in_hand.get(j).copied().unwrap_or(false) {
                        w_finger += w;
                    }
                }
                w_palm >= 0.5 && w_palm > w_finger
            })
            .collect();
        let mut remap = vec![usize::MAX; mesh.verts.len()];
        let mut hand_verts = Vec::new();
        let mut hand_tris = Vec::new();
        let mut palm_tris = Vec::new();
        for &[a, b, c] in &mesh.tris {
            if is_hand[a] || is_hand[b] || is_hand[c] {
                let mut tri = [0usize; 3];
                for (slot, vi) in tri.iter_mut().zip([a, b, c]) {
                    if remap[vi] == usize::MAX {
                        remap[vi] = hand_verts.len();
                        hand_verts.push(vi);
                    }
                    *slot = remap[vi];
                }
                hand_tris.push(tri);
                if is_palm[a] || is_palm[b] || is_palm[c] {
                    palm_tris.push(tri);
                }
            }
        }
        if palm_tris.is_empty() {
            palm_tris = hand_tris.clone();
        }
        Ok(Stage {
            cam: Camera::game(subject.forward(), opts, unit),
            unit,
            mesh,
            socket,
            hand,
            hips: subject.map.node_of("DEF-spine"),
            blade: blade_local(opts, unit),
            width: opts.blade_width_m * unit,
            from_m: opts.blade_from_m,
            palm_roots,
            finger_chains,
            distal_mitten,
            hand_verts,
            hand_tris,
            palm_tris,
        })
    }

    /// The blade's guard and tip in the world, posed.
    pub fn blade_at(&self, world: &[Affine]) -> Option<(Vec3, Vec3)> {
        self.socket
            .map(|s| (world[s].apply(self.blade.0), world[s].apply(self.blade.1)))
    }

    /// The camera translated by a root-follow offset (same aim).
    pub fn cam_at(&self, offset: Vec3) -> Camera {
        Camera {
            eye: self.cam.eye + offset,
            ..self.cam
        }
    }

    pub fn shoot(&self, world: &[Affine]) -> Shot {
        self.shoot_cam(world, &self.cam)
    }

    /// One frame at `cam`: the body with its depth, and only the
    /// visible blade (pixels not hidden behind the body, and a tip
    /// that is on screen and not behind the body; a blade with no
    /// socket falls back to the hand as its tip).
    pub fn shoot_cam(&self, world: &[Affine], cam: &Camera) -> Shot {
        let mut body = Mask::new(cam.w, cam.h);
        let mut body_depth = vec![f64::INFINITY; cam.w * cam.h];
        let pts: Vec<Option<(f64, f64, f64)>> = self
            .mesh
            .posed(world)
            .into_iter()
            .map(|p| cam.project(p))
            .collect();
        for t in &self.mesh.tris {
            if let (Some(a), Some(b), Some(c)) = (pts[t[0]], pts[t[1]], pts[t[2]]) {
                body.fill_tri_depth(
                    (a.0, a.1),
                    (b.0, b.1),
                    (c.0, c.1),
                    a.2,
                    b.2,
                    c.2,
                    &mut body_depth,
                );
            }
        }
        // The fist round the guard is about this deep: a blade pixel
        // only that far behind the body still reads as held.
        let bias = 0.02 * self.unit;
        let mut tip = None;
        let blade = self.blade_at(world).and_then(|(g, t)| {
            let (pg, pt) = (cam.project(g)?, cam.project(t)?);
            let mut m = Mask::new(cam.w, cam.h);
            let mut blade_depth = vec![f64::INFINITY; cam.w * cam.h];
            let half = |z: f64| cam.f * self.width * 0.5 / z;
            m.fill_segment_depth(
                (pg.0, pg.1),
                (pt.0, pt.1),
                half(pg.2),
                half(pt.2),
                pg.2,
                pt.2,
                &mut blade_depth,
            );
            m.cull_behind(&blade_depth, &body_depth, bias);
            let on_screen =
                pt.0 >= 0.0 && pt.0 < cam.w as f64 && pt.1 >= 0.0 && pt.1 < cam.h as f64;
            let hidden = if on_screen {
                let i = pt.1 as usize * cam.w + pt.0 as usize;
                pt.2 > body_depth[i] + bias
            } else {
                true
            };
            if on_screen && !hidden {
                tip = Some((pt.0, pt.1));
            }
            Some(m)
        });
        if blade.is_none() {
            tip = self
                .hand
                .and_then(|h| cam.project(world[h].t))
                .filter(|(x, y, _)| {
                    *x >= 0.0 && *x < cam.w as f64 && *y >= 0.0 && *y < cam.h as f64
                })
                .map(|(x, y, _)| (x, y));
        }
        Shot { body, blade, tip }
    }

    /// Readability of a clip (`frames` at `dt`), against `neutral`.
    pub fn read(
        &self,
        rig: &Rig,
        frames: &[Vec<Option<Trs>>],
        dt: f64,
        neutral: &[Option<Trs>],
    ) -> (ReadCam, Vec<Shot>) {
        let n = frames.len();
        let world: Vec<Vec<Affine>> = frames.iter().map(|f| rig.world(f)).collect();
        // The game camera follows the root: each frame is shot from
        // the base camera shifted by the hips' smoothed travel.
        let hips_p: Vec<Vec3> = (0..n)
            .map(|f| self.hips.map(|h| world[f][h].t).unwrap_or(Vec3::ZERO))
            .collect();
        let offsets = follow_offsets(&hips_p, self.unit);
        let shots: Vec<Shot> = world
            .iter()
            .zip(&offsets)
            .map(|(w, o)| self.shoot_cam(w, &self.cam_at(*o)))
            .collect();
        let full = |s: &Shot| {
            let mut m = s.body.clone();
            if let Some(b) = &s.blade {
                m.union_with(b);
            }
            m
        };
        let ns = self.shoot(&rig.world(neutral));
        let neutral_full = full(&ns);
        let body_area = ns.body.area().max(1);
        let body_px = ns.body.rows().max(1);
        let mut out = ReadCam {
            body_px,
            body_area,
            blade: self.socket.is_some(),
            ..ReadCam::default()
        };
        if n < 3 {
            return (out, shots);
        }
        let hand_p: Vec<Vec3> = (0..n)
            .map(|f| self.hand.map(|h| world[f][h].t).unwrap_or(Vec3::ZERO))
            .collect();
        let tip_p: Vec<Vec3> = (0..n)
            .map(|f| match self.blade_at(&world[f]) {
                Some((_, t)) => t,
                None => hand_p[f],
            })
            .collect();
        let (peak, load, windup) = strike_timing(&hand_p, &tip_p, dt);
        let speed = |f: usize| {
            let (a, b) = (f.saturating_sub(1), (f + 1).min(n - 1));
            (tip_p[b] - tip_p[a]).length() / ((b - a) as f64 * dt)
        };
        let top = speed(peak);
        let mut settle = n - 1;
        for f in peak + 1..n {
            if speed(f) < HOLD_SHARE * top {
                settle = f;
                break;
            }
        }
        out.peak_frame = peak;
        out.load_frame = load;
        out.settle_frame = settle;
        let fulls: Vec<Mask> = shots.iter().map(full).collect();
        let mut sweep = Mask::new(self.cam.w, self.cam.h);
        let mut path = Vec::new();
        for f in load..=settle {
            let d = 1.0 - fulls[f].iou(&neutral_full);
            if d > out.pose_change {
                out.pose_change = d;
                out.key_frame = f;
            }
            if let Some(b) = &shots[f].blade {
                sweep.union_with(b);
            }
            if let Some(t) = shots[f].tip {
                path.push(t);
            }
        }
        if out.pose_change == 0.0 {
            out.key_frame = load;
        }
        out.body_frame = windup;
        for f in windup..=settle {
            let d = 1.0 - shots[f].body.iou(&ns.body);
            if d > out.body_change {
                out.body_change = d;
                out.body_frame = f;
            }
        }
        out.sweep = sweep.area() as f64 / body_area as f64;
        let (len, cons, turn) = path_arc(&path);
        out.arc_len = len / body_px as f64;
        out.arc_consistency = cons;
        out.arc_turn_deg = turn;
        out.arc = out.arc_len * cons;
        // Held load: the longest run of near-still frames in the second
        // before the peak.
        let win = ((1.0 / dt).round() as usize).max(1);
        let mut run = 0;
        for f in peak.saturating_sub(win).max(1)..peak {
            if 1.0 - fulls[f].iou(&fulls[f - 1]) < STILL_SHARE {
                run += 1;
                out.load_hold_frames = out.load_hold_frames.max(run);
            } else {
                run = 0;
            }
        }
        out.read_score = score_of(out.body_change, out.sweep, out.arc, out.load_hold_frames);
        // The grip over the strike, capped into the score by its
        // weaker part: a readable strike that is not held is a grip.
        out.grip_applies = self.socket.is_some();
        if out.grip_applies {
            let (mut handles, mut axes, mut curls, mut tips) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let skin_based = !self.hand_tris.is_empty();
            let (mut palm_surfaces, mut skin_wraps) = (Vec::new(), Vec::new());
            for f in load..=settle {
                let Some((guard, tip)) = self.blade_at(&world[f]) else {
                    continue;
                };
                let blade_vec = tip - guard;
                if blade_vec.length() < 1e-9 {
                    continue;
                }
                let dir = blade_vec.normalized();
                let grip = guard - dir.scale(self.from_m * self.unit);
                let palm = if self.palm_roots.is_empty() {
                    hand_p[f]
                } else {
                    self.palm_roots
                        .iter()
                        .fold(Vec3::ZERO, |s, &(r, sh)| {
                            s + hand_p[f].lerp(world[f][r].t, sh)
                        })
                        .scale(1.0 / self.palm_roots.len() as f64)
                };
                let chains: Vec<Vec<Vec3>> = self
                    .finger_chains
                    .iter()
                    .map(|ch| ch.iter().map(|&nd| world[f][nd].t).collect())
                    .collect();
                let (handle, axis, tip_d) = grip_frame(palm, grip, dir, &chains);
                handles.push(handle / self.unit);
                axes.push(axis / self.unit);
                if let Some(c) = finger_turn_deg(rig, &world[f], &self.finger_chains) {
                    curls.push(c);
                }
                tips.push(tip_d / self.unit);
                if skin_based {
                    let hand_pts = self.mesh.posed_subset(&world[f], &self.hand_verts);
                    let mut min_d2 = f64::INFINITY;
                    for &[ia, ib, ic] in &self.palm_tris {
                        let d2 = point_tri_dist_sq(grip, hand_pts[ia], hand_pts[ib], hand_pts[ic]);
                        if d2 < min_d2 {
                            min_d2 = d2;
                        }
                    }
                    palm_surfaces.push(min_d2.sqrt() / self.unit);

                    let ref_v = if dir.x.abs() < 0.9 {
                        Vec3::new(1.0, 0.0, 0.0)
                    } else {
                        Vec3::new(0.0, 1.0, 0.0)
                    };
                    let u = dir.cross(ref_v).normalized();
                    let v = dir.cross(u);
                    let mut bins = [false; SKIN_WRAP_BINS];
                    let step_len = 0.012 * self.unit;
                    for &[ia, ib, ic] in &self.hand_tris {
                        let (a, b, c) = (hand_pts[ia], hand_pts[ib], hand_pts[ic]);
                        let max_edge_len =
                            (b - a).length().max((c - b).length()).max((a - c).length());
                        let steps = ((max_edge_len / step_len).ceil() as usize).clamp(2, 6);
                        let inv_steps = 1.0 / steps as f64;
                        for i in 0..=steps {
                            let u_b = i as f64 * inv_steps;
                            for j in 0..=(steps - i) {
                                let v_b = j as f64 * inv_steps;
                                let w_b = 1.0 - u_b - v_b;
                                let p = a.scale(w_b) + b.scale(u_b) + c.scale(v_b);
                                let dv = p - grip;
                                let along = dv.dot(dir) / self.unit;
                                if along.abs() > SKIN_WRAP_HALF_LEN_M {
                                    continue;
                                }
                                let rad = dv - dir.scale(along * self.unit);
                                let r = rad.length() / self.unit;
                                if r < SKIN_WRAP_MIN_R_M || r > SKIN_WRAP_MAX_R_M {
                                    continue;
                                }
                                let r_unit = r * self.unit;
                                let rx = rad.dot(u) / r_unit;
                                let ry = rad.dot(v) / r_unit;
                                let mut ang = detmath::acos(rx.clamp(-1.0, 1.0));
                                if ry < 0.0 {
                                    ang = std::f64::consts::TAU - ang;
                                }
                                let bin_f = (ang / std::f64::consts::TAU) * SKIN_WRAP_BINS as f64;
                                let bi = bin_f.floor() as usize % SKIN_WRAP_BINS;
                                bins[bi] = true;
                            }
                        }
                    }
                    let wrap = bins.iter().filter(|&&b| b).count() as f64 / SKIN_WRAP_BINS as f64;
                    skin_wraps.push(wrap);
                }
            }
            let curl = if curls.is_empty() {
                None
            } else {
                Some(curls.iter().sum::<f64>() / curls.len() as f64)
            };
            let full_deg = if self.distal_mitten {
                GRIP_MITTEN_CURL_FULL_DEG
            } else {
                GRIP_CURL_FULL_DEG
            };
            let palm_surface_m = if skin_based {
                median(&palm_surfaces)
            } else {
                0.0
            };
            let skin_wrap = if skin_based { median(&skin_wraps) } else { 0.0 };
            let score = if skin_based {
                skin_grip_of(palm_surface_m, skin_wrap, curl)
            } else {
                grip_of_full(median(&handles), curl, full_deg)
            };
            out.grip = GripScore {
                score,
                handle_m: median(&handles),
                axis_m: median(&axes),
                curl_deg: curl.unwrap_or(0.0),
                tip_m: median(&tips),
                fingers: !curls.is_empty(),
                frames: handles.len(),
                palm_surface_m,
                skin_wrap,
                skin_based,
            };
            out.score = out.read_score.min(out.grip.score);
        } else {
            out.score = out.read_score;
        }
        (out, shots)
    }
}
