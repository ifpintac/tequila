//! Regions of a plane bounded by loops, divided into triangles: a building
//! model's profiles with their voids, a CAD face in its own parameters with
//! its trimming loops.
//!
//! Constrained Delaunay (spade) over the loops' edges, keeping the
//! triangles inside an odd number of loops - the even-odd rule, which
//! needs no telling which loop is the outer one and which are holes.
//! A loop that crosses another, as a carelessly written file's can, loses
//! the crossing edges as constraints rather than panicking, and the region
//! is still decided by the rule.

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

pub(crate) type P2 = [f64; 2];

/// A loop with its repeated points dropped: consecutive ones, and a last
/// that closes it on its first. Points closer than `eps` are repeats.
fn clean(points: &[P2], eps: f64) -> Vec<P2> {
    let same = |a: P2, b: P2| (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps;
    let mut out: Vec<P2> = Vec::with_capacity(points.len());
    for p in points {
        if p[0].is_finite() && p[1].is_finite() && out.last().is_none_or(|q| !same(*q, *p)) {
            out.push(*p);
        }
    }
    while out.len() > 1 && same(out[0], out[out.len() - 1]) {
        out.pop();
    }
    out
}

/// Whether a point is inside a closed polygon, by the crossings of a ray.
pub(crate) fn inside(p: P2, polygon: &[P2]) -> bool {
    let mut odd = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}

/// The region the loops enclose, by the even-odd rule, as triangles: the
/// points used (the loops' and the `extra` ones inside, which let a curved
/// surface's triangles follow its curve) and triangles indexing them,
/// counter-clockwise.
pub(crate) fn triangulate(loops: &[Vec<P2>], extra: &[P2]) -> (Vec<P2>, Vec<[usize; 3]>) {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in loops.iter().flatten().filter(|p| p[0].is_finite() && p[1].is_finite()) {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if lo[0] > hi[0] {
        return (Vec::new(), Vec::new());
    }
    let eps = (hi[0] - lo[0]).max(hi[1] - lo[1]) * 1e-10;
    let loops: Vec<Vec<P2>> = loops.iter().map(|l| clean(l, eps)).filter(|l| l.len() >= 3).collect();
    if loops.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
    for l in &loops {
        let handles: Vec<_> = l.iter().filter_map(|p| cdt.insert(Point2::new(p[0], p[1])).ok()).collect();
        for i in 0..handles.len() {
            let (a, b) = (handles[i], handles[(i + 1) % handles.len()]);
            if a != b {
                cdt.try_add_constraint(a, b);
            }
        }
    }
    for p in extra {
        if p[0].is_finite() && p[1].is_finite() {
            let _ = cdt.insert(Point2::new(p[0], p[1]));
        }
    }

    let mut points: Vec<P2> = Vec::new();
    let mut index: Vec<Option<usize>> = vec![None; cdt.num_vertices()];
    let mut triangles = Vec::new();
    for face in cdt.inner_faces() {
        let vertices = face.vertices();
        let [a, b, c] = vertices.map(|v| {
            let p = v.position();
            [p.x, p.y]
        });
        let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
        if loops.iter().filter(|l| inside(centre, l)).count() % 2 == 0 {
            continue;
        }
        let mut corner = |k: usize| {
            let v = vertices[k];
            *index[v.fix().index()].get_or_insert_with(|| {
                let p = v.position();
                points.push([p.x, p.y]);
                points.len() - 1
            })
        };
        let t = [corner(0), corner(1), corner(2)];
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        triangles.push(if area >= 0.0 { t } else { [t[0], t[2], t[1]] });
    }
    (points, triangles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(points: &[P2], triangles: &[[usize; 3]]) -> f64 {
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| points[i]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) * 0.5
            })
            .sum()
    }

    #[test]
    fn holes_are_left_out_whichever_way_they_wind() {
        let outer = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        for hole in [vec![[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]], vec![[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]]] {
            let (points, triangles) = triangulate(&[outer.clone(), hole], &[]);
            assert!((area(&points, &triangles) - 12.0).abs() < 1e-9);
        }
    }

    #[test]
    fn crossing_loops_do_not_panic() {
        // A bow tie, and a loop across the square's edge.
        let bow = vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]];
        let square = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let across = vec![[3.0, 1.0], [5.0, 1.0], [5.0, 3.0], [3.0, 3.0]];
        let (points, triangles) = triangulate(&[bow], &[]);
        assert!(area(&points, &triangles) > 0.0);
        let (points, triangles) = triangulate(&[square, across], &[]);
        // Inside one or the other, not both: 16 + 4 - 2 * 2.
        assert!((area(&points, &triangles) - 16.0).abs() < 1e-9, "{}", area(&points, &triangles));
    }

    #[test]
    fn extra_points_become_vertices_inside_only() {
        let square = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let (points, triangles) = triangulate(&[square], &[[0.5, 0.5], [0.25, 0.75]]);
        assert!(points.contains(&[0.5, 0.5]) && points.contains(&[0.25, 0.75]));
        assert!((area(&points, &triangles) - 1.0).abs() < 1e-12);
    }
}
