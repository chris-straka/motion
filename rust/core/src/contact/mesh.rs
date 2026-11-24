//! Skinned rest mesh extraction, clothing separation, and winding queries.

use crate::glb::{Affine, Document};
use crate::json::Json;
use crate::math::Vec3;
use crate::rig::Rig;

/// Skinned rest vertex: world position and (joint node, weight).
pub struct SkinVert {
    pub p: Vec3,
    /// Area-weighted vertex normal at rest (zero when no triangle uses it).
    pub n: Vec3,
    pub inf: Vec<(usize, f64)>,
    /// Which mesh primitive it came from (in file order over skinned nodes).
    pub prim: usize,
}

/// Rest positions of every skinned vertex (through the inverse bind
/// matrices, so the mesh node's own transform does not matter).
pub fn skinned_rest(doc: &Document, rig: &Rig) -> Result<Vec<SkinVert>, String> {
    Ok(skinned_mesh(doc, rig)?.0)
}

/// [`skinned_rest`] plus the triangles (indices into the vertex list).
pub fn skinned_mesh(doc: &Document, rig: &Rig) -> Result<(Vec<SkinVert>, Vec<[usize; 3]>), String> {
    let mut out = Vec::new();
    let mut tris_out: Vec<[usize; 3]> = Vec::new();
    let mut prim_no = 0usize;
    for node in doc.array("nodes") {
        let (Some(mesh), Some(skin)) = (
            node.get("mesh").and_then(Json::as_usize),
            node.get("skin").and_then(Json::as_usize),
        ) else {
            continue;
        };
        let joints = rig
            .skins
            .get(skin)
            .ok_or("node references a missing skin")?;
        let skin_doc = doc
            .array("skins")
            .get(skin)
            .ok_or("node references a missing skin entry")?;
        let ibm: Vec<Affine> = match skin_doc.get("inverseBindMatrices").and_then(Json::as_usize) {
            Some(a) => doc
                .read_accessor(a)?
                .iter()
                .map(|r| Affine::from_gltf_matrix(r))
                .collect(),
            None => vec![Affine::IDENTITY; joints.len()],
        };
        let mut mats = Vec::with_capacity(joints.len().min(ibm.len()));
        for (&j, b) in joints.iter().zip(&ibm) {
            let rest = rig
                .rest_world
                .get(j)
                .ok_or("skin references out-of-range joint")?;
            mats.push(rest.mul(b));
        }
        let mesh_doc = doc
            .array("meshes")
            .get(mesh)
            .ok_or("node references a missing mesh")?;
        let prims = mesh_doc
            .get("primitives")
            .and_then(Json::as_arr)
            .unwrap_or(&[])
            .to_vec();
        for prim in prims {
            let Some(attrs) = prim.get("attributes") else {
                continue;
            };
            let (Some(pa), Some(ja), Some(wa)) = (
                attrs.get("POSITION").and_then(Json::as_usize),
                attrs.get("JOINTS_0").and_then(Json::as_usize),
                attrs.get("WEIGHTS_0").and_then(Json::as_usize),
            ) else {
                continue;
            };
            let (pos, jr, wr) = (
                doc.read_accessor(pa)?,
                doc.read_accessor(ja)?,
                doc.read_accessor(wa)?,
            );
            let base = out.len();
            let this_prim = prim_no;
            prim_no += 1;
            let count = pos.len().min(jr.len()).min(wr.len());
            for v in 0..count {
                let p = Vec3::new(pos[v][0], pos[v][1], pos[v][2]);
                let mut acc = Vec3::ZERO;
                let mut inf = Vec::new();
                let mut total = 0.0;
                for c in 0..4 {
                    let w = wr[v][c];
                    let k = jr[v][c] as usize;
                    if w <= 0.0 || k >= joints.len() {
                        continue;
                    }
                    acc = acc + mats[k].apply(p).scale(w);
                    total += w;
                    inf.push((joints[k], w));
                }
                out.push(SkinVert {
                    p: if total > 0.0 {
                        acc.scale(1.0 / total)
                    } else {
                        p
                    },
                    n: Vec3::ZERO,
                    inf,
                    prim: this_prim,
                });
            }
            // Normals from the triangles (mode 4 or unset only).
            let mode = prim.get("mode").and_then(Json::as_usize).unwrap_or(4);
            if mode == 4 {
                let idx: Vec<usize> = match prim.get("indices").and_then(Json::as_usize) {
                    Some(ia) => doc
                        .read_accessor(ia)?
                        .iter()
                        .map(|r| r[0] as usize)
                        .collect(),
                    None => (0..count).collect(),
                };
                for tri in idx.chunks(3) {
                    if tri.len() < 3 || tri.iter().any(|&i| i >= count) {
                        continue;
                    }
                    let (a, b, c) = (
                        out[base + tri[0]].p,
                        out[base + tri[1]].p,
                        out[base + tri[2]].p,
                    );
                    let fnrm = (b - a).cross(c - a);
                    for &i in tri {
                        out[base + i].n = out[base + i].n + fnrm;
                    }
                    tris_out.push([base + tri[0], base + tri[1], base + tri[2]]);
                }
                for v in &mut out[base..] {
                    if v.n.length() > 1e-12 {
                        v.n = v.n.normalized();
                    }
                }
            }
        }
    }
    Ok((out, tris_out))
}

/// Which primitives are clothing worn over another: a primitive whose surface
/// encloses most of another primitive's torso vertices (a coat over the base
/// body it was cut for). A character made as one fused mesh has none.
pub fn clothing(
    verts: &[SkinVert],
    tris: &[[usize; 3]],
    torso: &dyn Fn(usize) -> bool,
) -> Vec<bool> {
    let n = verts.iter().map(|v| v.prim + 1).max().unwrap_or(0);
    let pos: Vec<Vec3> = verts.iter().map(|v| v.p).collect();
    let mut out = vec![false; n];
    if n < 2 {
        return out;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in verts {
        lo = lo.min(v.p.y);
        hi = hi.max(v.p.y);
    }
    let cell = (hi - lo).max(1e-6) / 10.0;
    for (g, out_g) in out.iter_mut().enumerate() {
        let gt: Vec<[usize; 3]> = tris
            .iter()
            .copied()
            .filter(|t| verts[t[0]].prim == g)
            .collect();
        if gt.len() < 50 {
            continue;
        }
        let cl = Clusters::new(&pos, &gt, cell);
        let cd = cluster_data(&pos, &gt, &cl);
        for o in (0..n).filter(|&o| o != g) {
            let inner: Vec<usize> = (0..verts.len())
                .filter(|&v| verts[v].prim == o && torso(v))
                .collect();
            if inner.len() < 20 {
                continue;
            }
            let step = (inner.len() / 300).max(1);
            let (mut inside, mut all) = (0usize, 0usize);
            for &v in inner.iter().step_by(step) {
                all += 1;
                if fast_winding(&pos, &gt, &cd, pos[v]).abs() >= 0.5 {
                    inside += 1;
                }
            }
            if inside * 2 > all {
                *out_g = true;
            }
        }
    }
    out
}

pub(super) fn dominant(v: &SkinVert) -> usize {
    let mut best = (usize::MAX, -1.0);
    for &(j, w) in &v.inf {
        if w > best.1 || (w == best.1 && j < best.0) {
            best = (j, w);
        }
    }
    best.0
}

pub(super) fn percentile(mut xs: Vec<f64>, q: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(f64::total_cmp);
    let i = ((xs.len() - 1) as f64 * q).round() as usize;
    xs[i.min(xs.len() - 1)]
}

/// Closest points between segments p1-q1 and p2-q2: (s, t, distance).
pub fn seg_seg(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (f64, f64, f64) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (mut s, mut t);
    if a <= 1e-12 && e <= 1e-12 {
        return (0.0, 0.0, r.length());
    }
    if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            s = if denom > 1e-12 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
        }
    }
    let c1 = p1 + d1.scale(s);
    let c2 = p2 + d2.scale(t);
    (s, t, (c1 - c2).length())
}

fn atan2(y: f64, x: f64) -> f64 {
    let r = (x * x + y * y).sqrt();
    if r < 1e-300 {
        return 0.0;
    }
    let a = crate::detmath::acos((x / r).clamp(-1.0, 1.0));
    if y < 0.0 {
        -a
    } else {
        a
    }
}

/// Triangles grouped into spatial clusters for [`fast_winding`].
#[derive(Clone, Debug)]
pub struct Clusters {
    /// Triangle indices per cluster.
    pub tris: Vec<Vec<usize>>,
}

impl Clusters {
    /// Group triangles by a grid over their rest centroids (`cell` wide).
    pub fn new(pos: &[Vec3], tris: &[[usize; 3]], cell: f64) -> Clusters {
        let mut keys: Vec<((i64, i64, i64), usize)> = tris
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let c = (pos[t[0]] + pos[t[1]] + pos[t[2]]).scale(1.0 / 3.0);
                let k = |x: f64| (x / cell).floor() as i64;
                ((k(c.x), k(c.y), k(c.z)), i)
            })
            .collect();
        keys.sort();
        let mut out: Vec<Vec<usize>> = Vec::new();
        let mut last = None;
        for (k, i) in keys {
            if last == Some(k) {
                if let Some(bucket) = out.last_mut() {
                    bucket.push(i);
                }
            } else {
                out.push(vec![i]);
                last = Some(k);
            }
        }
        Clusters { tris: out }
    }
}

/// Generalized winding number with far clusters approximated by their
/// area-weighted normal (the first-order term of Barill et al. 2018,
/// "Fast winding numbers for soups and clouds"): a cluster farther than
/// 2.5 of its radii contributes `N . (c - p) / |c - p|^3`.
pub(super) fn fast_winding(
    pos: &[Vec3],
    tris: &[[usize; 3]],
    clusters: &[(Vec3, Vec3, f64, &[usize])],
    p: Vec3,
) -> f64 {
    let mut sum = 0.0;
    for (c, n, r, members) in clusters {
        let d = *c - p;
        let l = d.length();
        if l > 2.5 * r && l > 1e-9 {
            sum += n.dot(d) / (l * l * l);
            continue;
        }
        for &ti in *members {
            let t = tris[ti];
            let (a, b, cc) = (pos[t[0]] - p, pos[t[1]] - p, pos[t[2]] - p);
            let (la, lb, lc) = (a.length(), b.length(), cc.length());
            let num = a.dot(b.cross(cc));
            let den = la * lb * lc + a.dot(b) * lc + a.dot(cc) * lb + b.dot(cc) * la;
            sum += 2.0 * atan2(num, den);
        }
    }
    sum / (4.0 * std::f64::consts::PI)
}

/// Per-pose cluster summaries: (centroid, area vector, radius, members).
pub(super) fn cluster_data<'a>(
    pos: &[Vec3],
    tris: &[[usize; 3]],
    cl: &'a Clusters,
) -> Vec<(Vec3, Vec3, f64, &'a [usize])> {
    cl.tris
        .iter()
        .map(|members| {
            let mut area = 0.0;
            let mut c = Vec3::ZERO;
            let mut n = Vec3::ZERO;
            for &ti in members {
                let t = tris[ti];
                let cr = (pos[t[1]] - pos[t[0]]).cross(pos[t[2]] - pos[t[0]]);
                let a = cr.length() * 0.5;
                area += a;
                c = c + (pos[t[0]] + pos[t[1]] + pos[t[2]]).scale(a / 3.0);
                n = n + cr.scale(0.5);
            }
            let c = if area > 0.0 {
                c.scale(1.0 / area)
            } else if let Some(&ti) = members.first() {
                pos[tris[ti][0]]
            } else {
                Vec3::ZERO
            };
            let r = members
                .iter()
                .flat_map(|&ti| tris[ti].iter())
                .map(|&v| (pos[v] - c).length())
                .fold(0.0, f64::max);
            (c, n, r, members.as_slice())
        })
        .collect()
}

/// Generalized winding number of `p` (Jacobson et al. 2013): the summed
/// signed solid angle of the triangles over 4 pi.
pub(super) fn winding(pos: &[Vec3], tris: &[[usize; 3]], p: Vec3) -> f64 {
    let mut sum = 0.0;
    for t in tris {
        let (a, b, c) = (pos[t[0]] - p, pos[t[1]] - p, pos[t[2]] - p);
        let (la, lb, lc) = (a.length(), b.length(), c.length());
        let num = a.dot(b.cross(c));
        let den = la * lb * lc + a.dot(b) * lc + a.dot(c) * lb + b.dot(c) * la;
        sum += 2.0 * atan2(num, den);
    }
    sum / (4.0 * std::f64::consts::PI)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distance_cases() {
        let (s, t, d) = seg_seg(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.5, 1.0, -1.0),
            Vec3::new(0.5, 1.0, 1.0),
        );
        assert!((s - 0.5).abs() < 1e-12 && (t - 0.5).abs() < 1e-12 && (d - 1.0).abs() < 1e-12);
        // Parallel segments.
        let (_, _, d) = seg_seg(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
        );
        assert!((d - 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_shell_round_the_body_is_clothing() {
        // a small box (the body, prim 0) inside a larger one (a coat, prim 1)
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for (prim, size) in [(0usize, 1.0), (1usize, 1.6)] {
            let base = verts.len();
            for z in [-size, size] {
                for (x, y) in [(-size, -size), (size, -size), (size, size), (-size, size)] {
                    verts.push(SkinVert {
                        p: Vec3::new(x, y, z),
                        n: Vec3::ZERO,
                        inf: vec![(0, 1.0)],
                        prim,
                    });
                }
            }
            // a finer inner sample so the body has torso vertices to test
            for i in 0..30 {
                let t = i as f64 / 29.0 * 2.0 - 1.0;
                verts.push(SkinVert {
                    p: Vec3::new(t * size * 0.9, 0.0, size * 0.95),
                    n: Vec3::ZERO,
                    inf: vec![(0, 1.0)],
                    prim,
                });
            }
            let quads = [
                [0, 3, 2, 1],
                [4, 5, 6, 7],
                [0, 1, 5, 4],
                [1, 2, 6, 5],
                [2, 3, 7, 6],
                [3, 0, 4, 7],
            ];
            // enough triangles to count as a surface: repeat the box's faces
            for _ in 0..5 {
                for q in quads {
                    tris.push([base + q[0], base + q[1], base + q[2]]);
                    tris.push([base + q[0], base + q[2], base + q[3]]);
                }
            }
        }
        let cloth = clothing(&verts, &tris, &|_| true);
        assert_eq!(cloth, vec![false, true]);
    }

    #[test]
    fn winding_of_a_cube() {
        let mut pos = Vec::new();
        for z in [-1.0, 1.0] {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                pos.push(Vec3::new(x, y, z));
            }
        }
        let quads = [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut tris = Vec::new();
        for q in quads {
            tris.push([q[0], q[1], q[2]]);
            tris.push([q[0], q[2], q[3]]);
        }
        // Clustered (fast) winding agrees inside, outside and far away.
        let cl = Clusters::new(&pos, &tris, 0.7);
        let cd = cluster_data(&pos, &tris, &cl);
        for p in [
            Vec3::new(0.2, -0.3, 0.1),
            Vec3::new(2.5, 0.0, 0.0),
            Vec3::new(9.0, 4.0, -3.0),
        ] {
            let (e, f) = (winding(&pos, &tris, p), fast_winding(&pos, &tris, &cd, p));
            assert!((e - f).abs() < 0.05, "{p:?}: {e} vs {f}");
        }
        let inside = winding(&pos, &tris, Vec3::new(0.2, -0.3, 0.1));
        let outside = winding(&pos, &tris, Vec3::new(2.5, 0.0, 0.0));
        assert!((inside.abs() - 1.0).abs() < 1e-9, "{inside}");
        assert!(outside.abs() < 1e-9, "{outside}");
    }

    #[test]
    fn skinned_mesh_rejects_missing_mesh_without_panic() {
        let mut doc = crate::fixture::humanoid(crate::fixture::Naming::Def, false, false).unwrap();
        let rig = Rig::from_doc(&doc).unwrap();
        for node in doc.array_mut("nodes") {
            if node.get("mesh").is_some() {
                node.set("mesh", Json::num(999.0));
                break;
            }
        }
        let Err(err) = skinned_mesh(&doc, &rig) else {
            panic!("expected skinned_mesh to fail on missing mesh");
        };
        assert!(err.contains("missing mesh"), "{err}");
    }

    #[test]
    fn clusters_handle_empty_and_degenerate_triangles() {
        let cl = Clusters::new(&[], &[], 0.5);
        assert!(cl.tris.is_empty());
        assert!(cluster_data(&[], &[], &cl).is_empty());

        let pos = vec![Vec3::ZERO, Vec3::ZERO, Vec3::ZERO];
        let tris = vec![[0, 1, 2]];
        let cl = Clusters::new(&pos, &tris, 0.5);
        let cd = cluster_data(&pos, &tris, &cl);
        assert_eq!(cd.len(), 1);
        assert_eq!(cd[0].0, Vec3::ZERO);
    }
}
