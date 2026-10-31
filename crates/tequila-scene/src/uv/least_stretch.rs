//! Least stretch: a patch laid out as near as it can be to the surface's
//! own lengths, every triangle turned but neither stretched nor squashed.
//!
//! A conformal layout ([`super::flatten`]) keeps angles and lets areas
//! drift: a dome comes out with its middle small and its rim large. This
//! keeps lengths as well as it can, as-rigid-as-possible (Liu et al. 2008):
//! from a starting layout, each triangle is given the turn that lays its
//! part-shaped self best over where it is now (the local step), then every
//! vertex is placed to agree with all those turned triangles at once, as
//! well as it can (the global step, a cotangent Laplace solve); and again,
//! a fixed number of times, or until a step changes next to nothing.
//!
//! It works in the solve's own orientation, v up, where a triangle that
//! reads the right way turns counter-clockwise like the part's own: the
//! layout is mirrored into it and back, and an island laid out mirrored is
//! mirrored once more across, so the turns are turns.

use super::flatten::{facing, surface_area};
use super::shape::{conjugate_gradients, Sparse};
use tequila_core::math::{vec2, Vec2, Vec3};

/// How a patch is laid out flat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    /// Least-squares conformal maps: angles kept, areas let go. Quick, and
    /// what an unwrap has always done.
    #[default]
    Conformal,
    /// From the conformal layout, lengths kept as well as they can be: less
    /// stretch, a little slower.
    LeastStretch,
}

impl Method {
    pub fn key(self) -> &'static str {
        match self {
            Method::Conformal => "conformal",
            Method::LeastStretch => "least_stretch",
        }
    }

    pub fn from_key(key: &str) -> Option<Method> {
        match key {
            "conformal" => Some(Method::Conformal),
            "least_stretch" | "least-stretch" => Some(Method::LeastStretch),
            _ => None,
        }
    }
}

/// Which of a layout's coordinates a solve may change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axes {
    Both,
    /// Only u, across: v stays as it is.
    Across,
    /// Only v, down: u stays as it is.
    Down,
}

/// How many rounds of turning and placing at most, and how small a change
/// of the stretch from one round to the next ends them early.
pub const ROUNDS: usize = 60;
const SETTLED: f64 = 1e-5;

/// The patch - points on the part, triangles over them - laid out with
/// least stretch from `start`, a layout of it in the layout's own
/// orientation that reads the right way (or `mirrored`, all of it), at the
/// size it is to keep: the size the answer comes out at. `held` vertices
/// stay where `start` has them; with none, one does, which only fixes where
/// the answer is. Only the `axes` given change. The layout, and how many
/// rounds it took; None where it does not settle.
pub(crate) fn least_stretch(points: &[Vec3], faces: &[[usize; 3]], start: &[Vec2], mirrored: bool, held: &[usize], axes: Axes, rounds: usize) -> Option<(Vec<Vec2>, usize)> {
    let n = points.len();
    if n < 3 || faces.is_empty() || start.len() != n {
        return None;
    }
    // Into the solve's orientation: v up, and a mirrored patch across.
    let flip = |p: Vec2| vec2(if mirrored { -p.x } else { p.x }, -p.y);
    let mut uv: Vec<Vec2> = start.iter().map(|p| flip(*p)).collect();

    // Each triangle in its own plane, and its cotangents.
    struct Local {
        x: [Vec2; 3],
        cot: [f64; 3],
    }
    let locals: Vec<Local> = faces
        .iter()
        .map(|f| {
            let [p0, p1, p2] = f.map(|i| points[i]);
            let (e1, e2) = (p1 - p0, p2 - p0);
            let normal = e1.cross(e2);
            let (len, area2) = (e1.length(), normal.length());
            if len <= 0.0 || area2 <= 1e-20 {
                return Local { x: [Vec2::ZERO; 3], cot: [0.0; 3] };
            }
            let ax = e1 / len;
            let ay = (normal / area2).cross(ax);
            let x = [Vec2::ZERO, vec2(len, 0.0), vec2(e2.dot(ax), e2.dot(ay))];
            // The cotangent of the angle at corner k weighs the edge across
            // from it, from corner k+1 to k+2.
            let cot = [0, 1, 2].map(|k| {
                let (a, b) = (x[(k + 1) % 3] - x[k], x[(k + 2) % 3] - x[k]);
                let cross = (a.x * b.y - a.y * b.x) as f64;
                if cross.abs() > 1e-20 { (a.x * b.x + a.y * b.y) as f64 / cross } else { 0.0 }
            });
            Local { x, cot }
        })
        .collect();
    // An edge's weight, kept a little above nothing as everywhere here.
    let mean = locals.iter().flat_map(|l| l.cot.iter()).map(|c| c.abs()).sum::<f64>() / (locals.len() * 3).max(1) as f64;
    let least = (mean * 1e-3).max(1e-12);
    let weight = |c: f64| (0.5 * c).max(least);

    let mut fixed = vec![false; n];
    for &i in held {
        if i < n {
            fixed[i] = true;
        }
    }
    if !fixed.iter().any(|&f| f) {
        fixed[faces[0][0]] = true;
    }
    let mut column = vec![usize::MAX; n];
    let mut free = 0usize;
    for i in 0..n {
        if !fixed[i] {
            column[i] = free;
            free += 1;
        }
    }
    if free == 0 {
        return Some((start.to_vec(), 0));
    }
    // The global step's matrix, the same every round.
    let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); free];
    for (f, l) in faces.iter().zip(&locals) {
        for k in 0..3 {
            let (i, j) = (f[(k + 1) % 3], f[(k + 2) % 3]);
            let w = weight(l.cot[k]);
            for (p, q) in [(i, j), (j, i)] {
                if column[p] != usize::MAX {
                    rows[column[p]].push((column[p], w));
                    if column[q] != usize::MAX {
                        rows[column[p]].push((column[q], -w));
                    }
                }
            }
        }
    }
    let matrix = Sparse::from_rows(rows);
    let solve_u = axes != Axes::Down;
    let solve_v = axes != Axes::Across;
    let mut previous = f64::INFINITY;
    let mut done = 0;
    for round in 0..rounds.max(1) {
        // Local: each triangle's best turn onto where it is now.
        let mut turns: Vec<(f32, f32)> = Vec::with_capacity(faces.len());
        let mut energy = 0.0f64;
        for (f, l) in faces.iter().zip(&locals) {
            let (mut s00, mut s01, mut s10, mut s11) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for k in 0..3 {
                let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                let w = weight(l.cot[k]);
                let du = uv[f[i]] - uv[f[j]];
                let dx = l.x[i] - l.x[j];
                s00 += w * (du.x * dx.x) as f64;
                s01 += w * (du.x * dx.y) as f64;
                s10 += w * (du.y * dx.x) as f64;
                s11 += w * (du.y * dx.y) as f64;
            }
            let angle = (s10 - s01).atan2(s00 + s11);
            let (s, c) = (angle.sin() as f32, angle.cos() as f32);
            turns.push((c, s));
            for k in 0..3 {
                let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                let dx = l.x[i] - l.x[j];
                let r = vec2(c * dx.x - s * dx.y, s * dx.x + c * dx.y);
                energy += weight(l.cot[k]) * ((uv[f[i]] - uv[f[j]]) - r).length_squared() as f64;
            }
        }
        done = round;
        if (previous - energy).abs() <= SETTLED * energy.max(1e-30) {
            break;
        }
        previous = energy;
        // Global: every free vertex where the turned triangles want it.
        let mut b = vec![[0.0f64; 2]; free];
        for ((f, l), &(c, s)) in faces.iter().zip(&locals).zip(&turns) {
            for k in 0..3 {
                let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                let w = weight(l.cot[k]);
                let dx = l.x[i] - l.x[j];
                let r = vec2(c * dx.x - s * dx.y, s * dx.x + c * dx.y);
                for (p, q, sign) in [(f[i], f[j], 1.0f64), (f[j], f[i], -1.0f64)] {
                    let cp = column[p];
                    if cp == usize::MAX {
                        continue;
                    }
                    b[cp][0] += w * sign * r.x as f64;
                    b[cp][1] += w * sign * r.y as f64;
                    if column[q] == usize::MAX {
                        b[cp][0] += w * uv[q].x as f64;
                        b[cp][1] += w * uv[q].y as f64;
                    }
                }
            }
        }
        for (axis, wanted) in [(0usize, solve_u), (1usize, solve_v)] {
            if !wanted {
                continue;
            }
            // A coordinate held still on every vertex but this one's own:
            // the other coordinate's fixed vertices still pull.
            let rhs: Vec<f64> = b.iter().map(|v| v[axis]).collect();
            let mut x: Vec<f64> = (0..n).filter(|&i| column[i] != usize::MAX).map(|i| if axis == 0 { uv[i].x as f64 } else { uv[i].y as f64 }).collect();
            if !conjugate_gradients(&matrix, &rhs, &mut x, 1e-9, (free * 4).clamp(200, 20_000)) {
                return None;
            }
            for i in 0..n {
                if column[i] != usize::MAX {
                    if axis == 0 { uv[i].x = x[column[i]] as f32 } else { uv[i].y = x[column[i]] as f32 }
                }
            }
        }
        done = round + 1;
    }
    let out: Vec<Vec2> = uv.into_iter().map(flip).collect();
    out.iter().all(|p| p.is_finite()).then_some((out, done))
}

/// How far a layout of a patch is from the surface's own lengths, scaled
/// to its true area: the root mean square, by area, of how much each
/// triangle is stretched or squashed along the two ways it is most and
/// least. 0 is a layout with no stretch at all; 0.05 is five percent.
pub fn stretch_left(points: &[Vec3], faces: &[[usize; 3]], uv: &[Vec2]) -> f32 {
    let (surface, flat) = faces.iter().fold((0.0f64, 0.0f64), |(s, l), f| {
        (s + surface_area(points[f[0]], points[f[1]], points[f[2]]) as f64, l + facing(uv[f[0]], uv[f[1]], uv[f[2]]).abs() as f64)
    });
    if surface <= 0.0 || flat <= 0.0 {
        return 0.0;
    }
    let to_true = (surface / flat).sqrt();
    let (mut sum, mut total) = (0.0f64, 0.0f64);
    for f in faces {
        let [p0, p1, p2] = f.map(|i| points[i]);
        let (e1, e2) = (p1 - p0, p2 - p0);
        let area = e1.cross(e2).length() as f64 * 0.5;
        if area <= 1e-20 || e1.length() <= 0.0 {
            continue;
        }
        let x = e1.normalize();
        let y = e1.cross(e2).normalize().cross(x);
        let (p11, p12, p22) = (e1.length() as f64, e2.dot(x) as f64, e2.dot(y) as f64);
        let [a, b, c] = f.map(|i| uv[i]);
        let (q11, q21) = ((b.x - a.x) as f64 * to_true, (b.y - a.y) as f64 * to_true);
        let (q12, q22) = ((c.x - a.x) as f64 * to_true, (c.y - a.y) as f64 * to_true);
        let det_p = p11 * p22;
        let j11 = (q11 * p22) / det_p;
        let j12 = (q12 * p11 - q11 * p12) / det_p;
        let j21 = (q21 * p22) / det_p;
        let j22 = (q22 * p11 - q21 * p12) / det_p;
        let energy = j11 * j11 + j12 * j12 + j21 * j21 + j22 * j22;
        let det = (j11 * j22 - j12 * j21).abs();
        let (plus, minus) = ((energy + 2.0 * det).max(0.0).sqrt(), (energy - 2.0 * det).max(0.0).sqrt());
        let (most, least) = ((plus + minus) * 0.5, (plus - minus) * 0.5);
        sum += area * ((most - 1.0).powi(2) + (least - 1.0).powi(2)) * 0.5;
        total += area;
    }
    if total > 0.0 { (sum / total).sqrt() as f32 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::flatten::{conformal, folds, project, to_true_area};
    use tequila_core::math::vec3;

    /// A dome of quads, as points and faces.
    fn dome(n: usize) -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let mut points = Vec::new();
        for r in 0..=n {
            for c in 0..=n {
                let (x, z) = (c as f32 / n as f32 - 0.5, r as f32 / n as f32 - 0.5);
                points.push(vec3(x, 0.6 - (x * x + z * z) * 1.6, z));
            }
        }
        let at = |r: usize, c: usize| r * (n + 1) + c;
        let mut faces = Vec::new();
        for r in 0..n {
            for c in 0..n {
                faces.push([at(r, c), at(r + 1, c), at(r + 1, c + 1)]);
                faces.push([at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
            }
        }
        (points, faces)
    }

    #[test]
    fn a_dome_laid_out_with_least_stretch_has_less_than_conformally() {
        let (points, faces) = dome(12);
        let start = project(&points, Vec3::Y);
        let flat = to_true_area(&points, &faces, conformal(&points, &faces, &start, &[]).unwrap());
        assert!(!folds(&flat, &faces, Some(true)));
        let before = stretch_left(&points, &faces, &flat);
        let (least, rounds) = least_stretch(&points, &faces, &flat, false, &[], Axes::Both, ROUNDS).unwrap();
        let after = stretch_left(&points, &faces, &least);
        assert!(!folds(&least, &faces, Some(true)), "reads the right way, no folds");
        // A dome cannot lie flat unstretched; a third less is what keeping
        // lengths rather than angles gives here.
        assert!(after < before * 0.75, "least stretch {after} against conformal {before}");
        assert!(rounds > 1 && rounds <= ROUNDS);
        // Only across: every v is as it was.
        let (across, _) = least_stretch(&points, &faces, &flat, false, &[], Axes::Across, ROUNDS).unwrap();
        for (a, b) in across.iter().zip(&flat) {
            assert_eq!(a.y, b.y);
        }
    }

    #[test]
    fn a_flat_patch_has_no_stretch_and_keeps_it_mirrored() {
        let (points, faces) = dome(4);
        let flat_points: Vec<Vec3> = points.iter().map(|p| vec3(p.x, 0.0, p.z)).collect();
        let uv: Vec<Vec2> = flat_points.iter().map(|p| vec2(p.x, p.z)).collect();
        assert!(stretch_left(&flat_points, &faces, &uv) < 1e-4);
        // Laid out mirrored and squashed: it comes back mirrored, unsquashed.
        assert!(!folds(&uv, &faces, Some(true)), "seen from above it reads the right way");
        let squashed: Vec<Vec2> = uv.iter().map(|p| vec2(-p.x, p.y * 0.5)).collect();
        let (back, _) = least_stretch(&flat_points, &faces, &squashed, true, &[], Axes::Both, ROUNDS).unwrap();
        assert!(stretch_left(&flat_points, &faces, &back) < 1e-3);
        assert!(!folds(&back, &faces, Some(false)), "still mirrored, as it was");
    }
}
