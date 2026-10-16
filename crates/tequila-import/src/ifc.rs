//! IFC (ISO 16739), the building model format, read in the subset a
//! product or interior render needs (`docs/03` §6).
//!
//! IFC is STEP's exchange format with a building schema, so the file is read
//! by the same Part 21 parser STEP uses. What comes in:
//!
//! * the spatial structure - project, site, building, storeys - as groups,
//!   and every element with a body (walls, slabs, windows, doors, furniture,
//!   proxies, …) as a part named as the model names it;
//! * the body geometry IFC files actually carry: extruded profiles
//!   (rectangles, circles, ellipses, polylines with voids, hollow sections,
//!   I and L shapes), faceted B-reps and surface models, IFC4's triangulated
//!   and polygonal face sets, and mapped items (one definition placed many
//!   times);
//! * openings cut: the holes models leave in walls and slabs for windows,
//!   doors and stairs, and walls trimmed to a roof by half-spaces, each
//!   cut closed with the reveal it leaves;
//! * each element's colour from its styled items, transparent ones as glass;
//! * its GlobalId, IFC type and property sets, as metadata - searchable in
//!   the Scene tab (`Pset_WallCommon.IsExternal = true`) and exported to CSV;
//! * the length unit; Z up turned Y up.
//!
//! Not read: swept disks, revolved solids and NURBS (`IfcAdvancedBrep`),
//! which a warning lists by count, and cuts by shapes that are neither
//! extrusions nor convex, which are drawn uncut with a warning. Spaces and
//! openings are left out as parts, being volumes rather than things.

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::step::{arguments, real, reference, references, string, Entities};
use crate::ImportError;
use tequila_core::math::{vec3, Mat4, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_materials::material_type::MaterialType;
use tequila_scene::material::MaterialDefinition;
use ruststep::ast::Parameter;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

/// A 4x4 transform in double precision: building coordinates can sit
/// kilometres from the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
struct M([[f64; 4]; 4]);

impl M {
    const IDENTITY: M = M([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    fn mul(&self, o: &M) -> M {
        let mut r = [[0.0; 4]; 4];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.0[i][k] * o.0[k][j]).sum();
            }
        }
        M(r)
    }

    fn point(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
            m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
            m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
        ]
    }

    fn vector(&self, v: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    /// From axes and an origin, as IFC placements give them.
    fn frame(x: [f64; 3], y: [f64; 3], z: [f64; 3], origin: [f64; 3]) -> M {
        M([
            [x[0], y[0], z[0], origin[0]],
            [x[1], y[1], z[1], origin[1]],
            [x[2], y[2], z[2], origin[2]],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// The inverse of a placement, which is a rotation and a move.
    fn rigid_inverse(&self) -> M {
        let m = &self.0;
        let mut r = M::IDENTITY;
        for i in 0..3 {
            for j in 0..3 {
                r.0[i][j] = m[j][i];
            }
        }
        for i in 0..3 {
            r.0[i][3] = -(0..3).map(|k| r.0[i][k] * m[k][3]).sum::<f64>();
        }
        r
    }

    /// In single precision, moved by `-shift`.
    fn to_transform(self, shift: [f64; 3]) -> Transform {
        let mut out = Mat4::IDENTITY;
        for r in 0..3 {
            for c in 0..3 {
                out.m[r][c] = self.0[r][c] as f32;
            }
            out.m[r][3] = (self.0[r][3] - shift[r]) as f32;
        }
        Transform::from_matrix(out).unwrap_or(Transform::IDENTITY)
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn normalized(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-12).then(|| scale(a, 1.0 / l))
}

/// Triangles in a part's own space, double precision.
type Triangles = Vec<[[f64; 3]; 3]>;

/// A closed 2D polygon and its holes, as triangles, counter-clockwise.
fn triangulate(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> Vec<[[f64; 2]; 3]> {
    let loops: Vec<Vec<[f64; 2]>> = std::iter::once(outer.to_vec()).chain(holes.iter().cloned()).collect();
    let (points, triangles) = crate::planar::triangulate(&loops, &[]);
    triangles.into_iter().map(|t| t.map(|i| points[i])).collect()
}

/// A planar polygon in 3D, with holes, as triangles facing the way the
/// outer loop winds.
fn polygon(outer: &[[f64; 3]], holes: &[Vec<[f64; 3]>]) -> Triangles {
    if outer.len() < 3 {
        return Vec::new();
    }
    // Newell's normal: robust for any simple polygon.
    let Some(n) = normalized(newell(outer)) else { return Vec::new() };
    if outer.len() == 3 && holes.is_empty() {
        return vec![[outer[0], outer[1], outer[2]]];
    }
    let helper = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = normalized(cross(helper, n)).unwrap_or([1.0, 0.0, 0.0]);
    let v = cross(n, u);
    let origin = outer[0];
    let flat = |p: &[f64; 3]| {
        let d = sub(*p, origin);
        [dot(d, u), dot(d, v)]
    };
    let back = |q: [f64; 2]| {
        [
            origin[0] + u[0] * q[0] + v[0] * q[1],
            origin[1] + u[1] * q[0] + v[1] * q[1],
            origin[2] + u[2] * q[0] + v[2] * q[1],
        ]
    };
    let outer2: Vec<[f64; 2]> = outer.iter().map(flat).collect();
    let holes2: Vec<Vec<[f64; 2]>> = holes.iter().map(|h| h.iter().map(flat).collect()).collect();
    // (u, v, n) is right-handed, so counter-clockwise in (u, v) faces n.
    triangulate(&outer2, &holes2).into_iter().map(|t| t.map(back)).collect()
}

fn signed_area(l: &[[f64; 2]]) -> f64 {
    (0..l.len())
        .map(|i| {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

/// A profile swept straight along `direction` for `depth`: two caps and
/// the sides, facing out, in the frame `position` places it.
fn extrude(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>], direction: [f64; 3], depth: f64, position: &M) -> Triangles {
    let lift = scale(direction, depth);
    let at = |p: [f64; 2], up: bool| {
        let base = [p[0], p[1], 0.0];
        position.point(if up { [base[0] + lift[0], base[1] + lift[1], base[2] + lift[2]] } else { base })
    };
    // Swept down through its own plane, the solid is the mirror image of
    // one swept up: every face turns over.
    let flip = lift[2] < 0.0;
    let mut out: Triangles = Vec::new();
    let mut push = |t: [[f64; 3]; 3]| out.push(if flip { [t[0], t[2], t[1]] } else { t });
    for [a, b, c] in triangulate(outer, holes) {
        push([at(a, true), at(b, true), at(c, true)]);
        push([at(a, false), at(c, false), at(b, false)]);
    }
    for (index, loop_) in std::iter::once(outer).chain(holes.iter().map(|h| h.as_slice())).enumerate() {
        // An outer loop counter-clockwise, or a hole clockwise, has the
        // solid on its left: its sides face right, out.
        let ccw = signed_area(loop_) > 0.0;
        let solid_on_left = if index == 0 { ccw } else { !ccw };
        for i in 0..loop_.len() {
            let (p, q) = (loop_[i], loop_[(i + 1) % loop_.len()]);
            if (p[0] - q[0]).abs() + (p[1] - q[1]).abs() <= 1e-9 {
                continue;
            }
            let (p0, q0, p1, q1) = (at(p, false), at(q, false), at(p, true), at(q, true));
            if solid_on_left {
                push([p0, q0, q1]);
                push([p0, q1, p1]);
            } else {
                push([p0, q1, q0]);
                push([p0, p1, q1]);
            }
        }
    }
    out
}

fn circle(radius: f64, segments: usize) -> Vec<[f64; 2]> {
    (0..segments)
        .map(|i| {
            let a = i as f64 / segments as f64 * std::f64::consts::TAU;
            [radius * a.cos(), radius * a.sin()]
        })
        .collect()
}

/// Whether a 2D loop turns one way all round.
fn is_convex(l: &[[f64; 2]]) -> bool {
    let n = l.len();
    let mut sign = 0.0f64;
    for i in 0..n {
        let (a, b, c) = (l[i], l[(i + 1) % n], l[(i + 2) % n]);
        let turn = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if turn.abs() < 1e-12 {
            continue;
        }
        if sign * turn < 0.0 {
            return false;
        }
        sign = turn;
    }
    n >= 3
}

// Cutting. Building models write walls whole and the openings for their
// windows and doors as separate volumes to take away, and trim walls to a
// roof by half-spaces; drawn uncut, every window is behind solid wall. What
// is taken away here is convex, or split into convex parts, which keeps the
// cut exact: a body's faces are clipped to outside each volume, and each
// volume's faces that lie inside the body are added, turned to face into
// the space left.

/// A plane, its normal facing out of the volume it bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Plane {
    n: [f64; 3],
    d: f64,
}

impl Plane {
    fn distance(&self, p: [f64; 3]) -> f64 {
        dot(self.n, p) - self.d
    }
}

/// A convex volume: where every plane's distance is negative.
type Cutter = Vec<Plane>;

type Polygon = Vec<[f64; 3]>;

fn bounds<'p>(points: impl IntoIterator<Item = &'p [f64; 3]>) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// Newell's normal, unnormalized: area-weighted, so it faces the way the
/// polygon winds.
fn newell(poly: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0; 3];
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

/// The planes of a convex solid's faces, facing out; `None` if it is not
/// convex.
fn convex_planes(triangles: &Triangles) -> Option<Cutter> {
    let (lo, hi) = bounds(triangles.iter().flatten());
    let eps = dot(sub(hi, lo), sub(hi, lo)).sqrt() * 1e-7;
    let mut planes: Cutter = Vec::new();
    for t in triangles {
        let Some(n) = normalized(cross(sub(t[1], t[0]), sub(t[2], t[0]))) else { continue };
        let d = dot(n, t[0]);
        if !planes.iter().any(|p| dot(p.n, n) > 1.0 - 1e-9 && (p.d - d).abs() <= eps) {
            planes.push(Plane { n, d });
        }
    }
    let contains_all = |planes: &[Plane]| triangles.iter().flatten().all(|p| planes.iter().all(|plane| plane.distance(*p) <= eps));
    if planes.len() < 4 {
        return None;
    }
    if contains_all(&planes) {
        return Some(planes);
    }
    // Wound inside out.
    let flipped: Cutter = planes.iter().map(|p| Plane { n: scale(p.n, -1.0), d: -p.d }).collect();
    contains_all(&flipped).then_some(flipped)
}

/// A convex polygon cut by a plane: the part inside (distance ≤ 0) and the
/// part outside. Distances within `eps` count as on the plane.
fn split(poly: &[[f64; 3]], plane: &Plane, eps: f64) -> (Polygon, Polygon) {
    let s: Vec<f64> = poly
        .iter()
        .map(|p| {
            let s = plane.distance(*p);
            if s.abs() <= eps { 0.0 } else { s }
        })
        .collect();
    let (mut inside, mut outside) = (Vec::new(), Vec::new());
    for i in 0..poly.len() {
        let j = (i + 1) % poly.len();
        let (a, b, sa, sb) = (poly[i], poly[j], s[i], s[j]);
        if sa <= 0.0 {
            inside.push(a);
        }
        if sa >= 0.0 {
            outside.push(a);
        }
        if (sa < 0.0 && sb > 0.0) || (sa > 0.0 && sb < 0.0) {
            let x = add(a, scale(sub(b, a), sa / (sa - sb)));
            inside.push(x);
            outside.push(x);
        }
    }
    (inside, outside)
}

/// What of a convex polygon lies outside a convex volume, as convex
/// polygons. A polygon lying in one of the volume's faces counts as inside
/// it where it faces the same way (the volume is behind it, as the body
/// is: an opening flush with a wall takes the wall's face) and outside
/// where it faces the other way (the volume only touches it: a wall
/// trimmed exactly at its top keeps its top).
fn subtract(poly: Polygon, normal: [f64; 3], cutter: &Cutter, eps: f64, out: &mut Vec<Polygon>) {
    let mut rest = poly;
    for plane in cutter {
        let (lo, hi) = rest.iter().map(|p| plane.distance(*p)).fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(s), hi.max(s)));
        if hi <= eps && lo >= -eps {
            if dot(normal, plane.n) > 0.0 {
                continue;
            }
            out.push(rest);
            return;
        }
        if hi <= eps {
            continue;
        }
        if lo >= -eps {
            out.push(rest);
            return;
        }
        let (inside, outside) = split(&rest, plane, eps);
        if outside.len() >= 3 {
            out.push(outside);
        }
        if inside.len() < 3 {
            return;
        }
        rest = inside;
    }
}

/// A convex volume's face on each of its planes, wound to face out, cut
/// from a square of half-size `reach` about `centre` so that an unbounded
/// volume (a half-space) has faces too.
fn cutter_faces(cutter: &Cutter, centre: [f64; 3], reach: f64) -> Vec<Polygon> {
    let mut out = Vec::new();
    for (i, plane) in cutter.iter().enumerate() {
        let helper = if plane.n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let Some(u) = normalized(cross(helper, plane.n)) else { continue };
        let v = cross(plane.n, u);
        let c = sub(centre, scale(plane.n, plane.distance(centre)));
        // (u, v, n) is right-handed: counter-clockwise in (u, v) faces n.
        let mut face: Polygon =
            [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|&(a, b)| add(c, add(scale(u, a * reach), scale(v, b * reach)))).collect();
        for (j, other) in cutter.iter().enumerate() {
            if i != j && face.len() >= 3 {
                face = split(&face, other, 0.0).0;
            }
        }
        if face.len() >= 3 {
            out.push(face);
        }
    }
    out
}

fn fan(poly: &[[f64; 3]], min_area: f64, out: &mut Triangles) {
    for i in 1..poly.len().saturating_sub(1) {
        let t = [poly[0], poly[i], poly[i + 1]];
        let c = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        if dot(c, c).sqrt() * 0.5 > min_area {
            out.push(t);
        }
    }
}

/// Whether a point is inside a closed body: rays three ways, by the parity
/// of their crossings, the majority deciding where a ray grazes an edge.
fn inside(p: [f64; 3], body: &Triangles) -> bool {
    const RAYS: [[f64; 3]; 3] = [[0.5772, 0.6513, 0.4925], [-0.4413, 0.2967, 0.8469], [0.6124, -0.5, 0.6124]];
    let odd = RAYS
        .iter()
        .filter(|&&r| {
            let dir = normalized(r).unwrap_or([0.0, 0.0, 1.0]);
            let crossings = body
                .iter()
                .filter(|t| {
                    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
                    let q = cross(dir, e2);
                    let det = dot(e1, q);
                    if det.abs() <= 1e-14 * dot(e1, e1).max(dot(e2, e2)) {
                        return false;
                    }
                    let s = sub(p, t[0]);
                    let u = dot(s, q) / det;
                    if !(0.0..=1.0).contains(&u) {
                        return false;
                    }
                    let r = cross(s, e1);
                    let v = dot(dir, r) / det;
                    v >= 0.0 && u + v <= 1.0 && dot(e2, r) / det > 0.0
                })
                .count();
            crossings % 2 == 1
        })
        .count();
    odd >= 2
}

/// Whether a point lies on one of a body's faces.
fn on_surface(p: [f64; 3], body: &Triangles, eps: f64) -> bool {
    body.iter().any(|t| {
        let Some(n) = normalized(cross(sub(t[1], t[0]), sub(t[2], t[0]))) else { return false };
        if dot(n, sub(p, t[0])).abs() > eps {
            return false;
        }
        // Inside all three edges, give or take.
        (0..3).all(|i| {
            let (a, b) = (t[i], t[(i + 1) % 3]);
            dot(cross(sub(b, a), sub(p, a)), n) >= -eps * dot(sub(b, a), sub(b, a)).sqrt()
        })
    })
}

/// A closed body with convex volumes taken away.
fn carve(body: Triangles, cutters: &[Cutter]) -> Triangles {
    if cutters.is_empty() || body.is_empty() {
        return body;
    }
    let (lo, hi) = bounds(body.iter().flatten());
    let diagonal = dot(sub(hi, lo), sub(hi, lo)).sqrt().max(1e-9);
    let eps = diagonal * 1e-7;
    let min_area = eps * eps;
    let mut out: Triangles = Vec::new();

    // The body's faces, cut back to outside every volume.
    for t in &body {
        let normal = newell(t);
        let mut pieces = vec![t.to_vec()];
        for cutter in cutters {
            let mut next = Vec::new();
            for piece in pieces {
                subtract(piece, normal, cutter, eps, &mut next);
            }
            pieces = next;
        }
        for piece in &pieces {
            fan(piece, min_area, &mut out);
        }
    }

    // The planes the body's faces lie in, with the bounds of the faces in
    // each, to divide the volumes' faces where they pass through it.
    let mut planes: Vec<(Plane, [f64; 3], [f64; 3])> = Vec::new();
    for t in &body {
        let Some(n) = normalized(cross(sub(t[1], t[0]), sub(t[2], t[0]))) else { continue };
        let plane = Plane { n, d: dot(n, t[0]) };
        let (tlo, thi) = bounds(t.iter());
        match planes.iter_mut().find(|(p, ..)| dot(p.n, n).abs() > 1.0 - 1e-9 && (p.distance(t[0])).abs() <= eps) {
            Some((_, plo, phi)) => {
                for k in 0..3 {
                    plo[k] = plo[k].min(tlo[k]);
                    phi[k] = phi[k].max(thi[k]);
                }
            }
            None => planes.push((plane, tlo, thi)),
        }
    }
    let centre = scale(add(lo, hi), 0.5);
    for (i, cutter) in cutters.iter().enumerate() {
        for mut face in cutter_faces(cutter, centre, diagonal * 2.0) {
            face.reverse();
            let normal = newell(&face);
            let (flo, fhi) = bounds(face.iter());
            let near: Vec<&Plane> = planes
                .iter()
                .filter(|(_, plo, phi)| (0..3).all(|k| plo[k] <= fhi[k] + eps && phi[k] >= flo[k] - eps))
                .map(|(p, ..)| p)
                .collect();
            // A body of many faces would divide a face into too many
            // pieces to test: its cut is left open.
            if near.len() > 64 {
                continue;
            }
            let mut fragments = vec![face];
            for plane in near {
                let mut next = Vec::new();
                for f in fragments {
                    let (lo_s, hi_s) = f.iter().map(|p| plane.distance(*p)).fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(s), hi.max(s)));
                    if hi_s > eps && lo_s < -eps {
                        let (a, b) = split(&f, plane, eps);
                        next.extend([a, b].into_iter().filter(|p| p.len() >= 3));
                    } else {
                        next.push(f);
                    }
                }
                fragments = next;
            }
            for fragment in fragments {
                let centroid = scale(fragment.iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / fragment.len() as f64);
                // On the body's surface it is the body's own face, drawn
                // already, or the mouth of an opening, which stays open.
                if on_surface(centroid, &body, eps * 10.0) || !inside(centroid, &body) {
                    continue;
                }
                let mut pieces = vec![fragment];
                for (j, other) in cutters.iter().enumerate() {
                    if j != i {
                        let mut next = Vec::new();
                        for piece in pieces {
                            subtract(piece, normal, other, eps, &mut next);
                        }
                        pieces = next;
                    }
                }
                for piece in &pieces {
                    fan(piece, min_area, &mut out);
                }
            }
        }
    }
    out
}

/// A surface style: its name, colour (sRGB) and transparency.
#[derive(Clone, Debug, PartialEq)]
struct Style {
    name: Option<String>,
    rgb: [f32; 3],
    transparency: f32,
}

struct Reader<'a> {
    entities: Entities<'a>,
    /// Styles by the representation item they style.
    styles: BTreeMap<u64, Style>,
    /// Kinds of geometry met and not read, with how often.
    skipped: RefCell<BTreeMap<String, usize>>,
    clipped: Cell<usize>,
}

impl<'a> Reader<'a> {
    fn args(&self, id: u64) -> Option<(&'a str, &'a [Parameter])> {
        let record = self.entities.first(id)?;
        Some((record.name.as_str(), arguments(record)))
    }

    fn skip(&self, kind: &str) {
        *self.skipped.borrow_mut().entry(kind.to_string()).or_insert(0) += 1;
    }

    fn coordinates(&self, id: u64, kind: &str) -> Option<[f64; 3]> {
        let (name, args) = self.args(id)?;
        if name != kind {
            return None;
        }
        let Parameter::List(list) = args.first()? else { return None };
        let c: Vec<f64> = list.iter().filter_map(real).collect();
        Some([c.first().copied()?, c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)])
    }

    fn point(&self, id: u64) -> Option<[f64; 3]> {
        self.coordinates(id, "IFCCARTESIANPOINT")
    }

    fn direction(&self, id: u64) -> Option<[f64; 3]> {
        normalized(self.coordinates(id, "IFCDIRECTION")?)
    }

    /// `IfcAxis2Placement3D` (or 2D, in the XY plane) as a frame.
    fn placement(&self, id: u64) -> M {
        let Some((name, args)) = self.args(id) else { return M::IDENTITY };
        let origin = args.first().and_then(reference).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
        match name {
            "IFCAXIS2PLACEMENT3D" => {
                let z = args.get(1).and_then(reference).and_then(|d| self.direction(d)).unwrap_or([0.0, 0.0, 1.0]);
                let fallback = if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                let x0 = args.get(2).and_then(reference).and_then(|d| self.direction(d)).unwrap_or(fallback);
                let x = normalized(sub(x0, scale(z, dot(x0, z)))).unwrap_or(fallback);
                M::frame(x, cross(z, x), z, origin)
            }
            "IFCAXIS2PLACEMENT2D" => {
                let x = args.get(1).and_then(reference).and_then(|d| self.direction(d)).unwrap_or([1.0, 0.0, 0.0]);
                M::frame(x, [-x[1], x[0], 0.0], [0.0, 0.0, 1.0], origin)
            }
            _ => M::IDENTITY,
        }
    }

    /// `IfcLocalPlacement`, all the way up its chain.
    fn local_placement(&self, id: u64, depth: usize) -> M {
        let Some(("IFCLOCALPLACEMENT", args)) = self.args(id) else { return M::IDENTITY };
        let relative = args.get(1).and_then(reference).map(|p| self.placement(p)).unwrap_or(M::IDENTITY);
        match args.first().and_then(reference) {
            Some(parent) if depth < 64 => self.local_placement(parent, depth + 1).mul(&relative),
            _ => relative,
        }
    }

    /// `IfcCartesianTransformationOperator3D`: axes, origin, uniform scale.
    fn operator(&self, id: u64) -> M {
        let Some((_, args)) = self.args(id) else { return M::IDENTITY };
        let x = args.first().and_then(reference).and_then(|d| self.direction(d)).unwrap_or([1.0, 0.0, 0.0]);
        let y = args.get(1).and_then(reference).and_then(|d| self.direction(d)).unwrap_or([0.0, 1.0, 0.0]);
        let origin = args.get(2).and_then(reference).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
        let s = args.get(3).and_then(real).unwrap_or(1.0);
        let z = args
            .get(4)
            .and_then(reference)
            .and_then(|d| self.direction(d))
            .or_else(|| normalized(cross(x, y)))
            .unwrap_or([0.0, 0.0, 1.0]);
        M::frame(scale(x, s), scale(y, s), scale(z, s), origin)
    }

    /// A curve in a profile's plane, as a closed polyline.
    fn curve(&self, id: u64) -> Option<Vec<[f64; 2]>> {
        let (name, args) = self.args(id)?;
        match name {
            "IFCPOLYLINE" => {
                Some(references(args.first()?).into_iter().filter_map(|p| self.point(p)).map(|p| [p[0], p[1]]).collect())
            }
            "IFCINDEXEDPOLYCURVE" => {
                let (_, list) = self.args(reference(args.first()?)?)?;
                let Parameter::List(points) = list.first()? else { return None };
                let all: Vec<[f64; 2]> = points
                    .iter()
                    .filter_map(|p| match p {
                        Parameter::List(c) => Some([real(c.first()?)?, real(c.get(1)?)?]),
                        _ => None,
                    })
                    .collect();
                // Segments, lines and arcs, index into the points; arcs are
                // drawn through their three points as straight runs.
                let Some(Parameter::List(segments)) = args.get(1) else { return Some(all) };
                if segments.is_empty() {
                    return Some(all);
                }
                let mut out = Vec::new();
                for segment in segments {
                    let indices = match segment {
                        Parameter::Typed { parameter, .. } => integers(parameter),
                        other => integers(other),
                    };
                    out.extend(indices.into_iter().filter_map(|i| all.get(i.wrapping_sub(1)).copied()));
                }
                Some(out)
            }
            "IFCCIRCLE" => {
                let frame = self.placement(reference(args.first()?)?);
                let r = real(args.get(1)?)?;
                Some(
                    circle(r, 32)
                        .into_iter()
                        .map(|p| {
                            let q = frame.point([p[0], p[1], 0.0]);
                            [q[0], q[1]]
                        })
                        .collect(),
                )
            }
            other => {
                self.skip(other);
                None
            }
        }
    }

    /// A profile definition as its outer loop and holes, in its own plane.
    fn profile(&self, id: u64) -> Option<(Vec<[f64; 2]>, Vec<Vec<[f64; 2]>>)> {
        let (name, args) = self.args(id)?;
        let dimension = |i: usize| args.get(i).and_then(real);
        let placed = |l: Vec<[f64; 2]>| -> Vec<[f64; 2]> {
            let frame = args.get(2).and_then(reference).map(|p| self.placement(p)).unwrap_or(M::IDENTITY);
            l.into_iter()
                .map(|p| {
                    let q = frame.point([p[0], p[1], 0.0]);
                    [q[0], q[1]]
                })
                .collect()
        };
        let rect = |w: f64, h: f64| vec![[-w / 2.0, -h / 2.0], [w / 2.0, -h / 2.0], [w / 2.0, h / 2.0], [-w / 2.0, h / 2.0]];
        let reversed = |mut l: Vec<[f64; 2]>| {
            l.reverse();
            l
        };
        Some(match name {
            "IFCRECTANGLEPROFILEDEF" | "IFCROUNDEDRECTANGLEPROFILEDEF" => (placed(rect(dimension(3)?, dimension(4)?)), Vec::new()),
            "IFCRECTANGLEHOLLOWPROFILEDEF" => {
                let (w, h, t) = (dimension(3)?, dimension(4)?, dimension(5)?);
                (placed(rect(w, h)), vec![placed(reversed(rect(w - 2.0 * t, h - 2.0 * t)))])
            }
            "IFCCIRCLEPROFILEDEF" => (placed(circle(dimension(3)?, 32)), Vec::new()),
            "IFCCIRCLEHOLLOWPROFILEDEF" => {
                let (r, t) = (dimension(3)?, dimension(4)?);
                (placed(circle(r, 32)), vec![placed(reversed(circle(r - t, 32)))])
            }
            "IFCELLIPSEPROFILEDEF" => {
                let (a, b) = (dimension(3)?, dimension(4)?);
                let points = (0..32)
                    .map(|i| {
                        let t = i as f64 / 32.0 * std::f64::consts::TAU;
                        [a * t.cos(), b * t.sin()]
                    })
                    .collect();
                (placed(points), Vec::new())
            }
            "IFCISHAPEPROFILEDEF" => {
                let (x, y, h, f) = (dimension(3)? / 2.0, dimension(4)? / 2.0, dimension(5)? / 2.0, dimension(6)?);
                let outline = vec![
                    [-x, -y], [x, -y], [x, -y + f], [h, -y + f], [h, y - f], [x, y - f],
                    [x, y], [-x, y], [-x, y - f], [-h, y - f], [-h, -y + f], [-x, -y + f],
                ];
                (placed(outline), Vec::new())
            }
            "IFCLSHAPEPROFILEDEF" => {
                let d = dimension(3)?;
                let (w, t) = (dimension(4).unwrap_or(d), dimension(5)?);
                let outline = [[0.0, 0.0], [w, 0.0], [w, t], [t, t], [t, d], [0.0, d]];
                (placed(outline.into_iter().map(|p| [p[0] - w / 2.0, p[1] - d / 2.0]).collect()), Vec::new())
            }
            "IFCARBITRARYCLOSEDPROFILEDEF" => (self.curve(reference(args.get(2)?)?)?, Vec::new()),
            "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                let outer = self.curve(reference(args.get(2)?)?)?;
                let holes = args.get(3).map(references).unwrap_or_default().into_iter().filter_map(|c| self.curve(c)).collect();
                (outer, holes)
            }
            other => {
                self.skip(other);
                return None;
            }
        })
    }

    /// Faces of a shell, each an outer loop and its holes.
    fn faces(&self, ids: Vec<u64>) -> Triangles {
        let mut out = Vec::new();
        for face in ids {
            let Some((_, args)) = self.args(face) else { continue };
            let mut outer: Option<Vec<[f64; 3]>> = None;
            let mut holes = Vec::new();
            for bound in args.first().map(references).unwrap_or_default() {
                let Some((kind, bound_args)) = self.args(bound) else { continue };
                let Some(loop_id) = bound_args.first().and_then(reference) else { continue };
                let Some(("IFCPOLYLOOP", loop_args)) = self.args(loop_id) else {
                    self.skip(self.args(loop_id).map_or("IFCLOOP", |(n, _)| n));
                    continue;
                };
                let mut points: Vec<[f64; 3]> =
                    loop_args.first().map(references).unwrap_or_default().into_iter().filter_map(|p| self.point(p)).collect();
                if matches!(bound_args.get(1), Some(Parameter::Enumeration(e)) if e == "F") {
                    points.reverse();
                }
                if kind == "IFCFACEOUTERBOUND" && outer.is_none() {
                    outer = Some(points);
                } else {
                    holes.push(points);
                }
            }
            // A face without a bound marked outer: its first is.
            let outer = match outer {
                Some(o) => o,
                None if !holes.is_empty() => holes.remove(0),
                None => continue,
            };
            out.extend(polygon(&outer, &holes));
        }
        out
    }

    /// The faces of each shell.
    fn shells(&self, ids: Vec<u64>) -> Triangles {
        let faces = ids
            .into_iter()
            .filter_map(|shell| self.args(shell))
            .flat_map(|(_, args)| args.first().map(references).unwrap_or_default())
            .collect();
        self.faces(faces)
    }

    /// An IFC4 tessellated item: triangulated or polygonal face set.
    fn face_set(&self, name: &str, args: &[Parameter]) -> Triangles {
        let points: Vec<[f64; 3]> = match args.first().and_then(reference).and_then(|c| self.args(c)) {
            Some((_, list_args)) => match list_args.first() {
                Some(Parameter::List(list)) => list
                    .iter()
                    .filter_map(|p| match p {
                        Parameter::List(c) => Some([real(c.first()?)?, real(c.get(1)?)?, c.get(2).and_then(real).unwrap_or(0.0)]),
                        _ => None,
                    })
                    .collect(),
                _ => return Vec::new(),
            },
            None => return Vec::new(),
        };
        let triangulated = name == "IFCTRIANGULATEDFACESET";
        // IFC4's optional remapping of indices into the points.
        let remap = args.get(if triangulated { 4 } else { 3 }).map(integers).unwrap_or_default();
        let point = |i: usize| -> Option<[f64; 3]> {
            let i = if remap.is_empty() { i } else { *remap.get(i.wrapping_sub(1))? };
            points.get(i.wrapping_sub(1)).copied()
        };
        let mut out = Vec::new();
        if triangulated {
            let Some(Parameter::List(triangles)) = args.get(3) else { return out };
            for t in triangles {
                if let [a, b, c] = integers(t)[..] {
                    if let (Some(a), Some(b), Some(c)) = (point(a), point(b), point(c)) {
                        out.push([a, b, c]);
                    }
                }
            }
        } else {
            for face in args.get(2).map(references).unwrap_or_default() {
                let Some((kind, face_args)) = self.args(face) else { continue };
                let outer: Vec<[f64; 3]> = face_args.first().map(integers).unwrap_or_default().into_iter().filter_map(point).collect();
                let holes: Vec<Vec<[f64; 3]>> = match (kind, face_args.get(1)) {
                    ("IFCINDEXEDPOLYGONALFACEWITHVOIDS", Some(Parameter::List(inner))) => {
                        inner.iter().map(|h| integers(h).into_iter().filter_map(point).collect()).collect()
                    }
                    _ => Vec::new(),
                };
                out.extend(polygon(&outer, &holes));
            }
        }
        out
    }

    /// One representation item, as triangles in the element's own space,
    /// and the style it carries.
    fn item(&self, id: u64, frame: &M, depth: usize) -> (Triangles, Option<Style>) {
        let style = self.styles.get(&id).cloned();
        let Some((name, args)) = self.args(id) else { return (Vec::new(), style) };
        let triangles = match name {
            "IFCEXTRUDEDAREASOLID" => {
                let Some((outer, holes)) = args.first().and_then(reference).and_then(|p| self.profile(p)) else {
                    return (Vec::new(), style);
                };
                let position = args.get(1).and_then(reference).map(|p| self.placement(p)).unwrap_or(M::IDENTITY);
                let direction = args.get(2).and_then(reference).and_then(|d| self.direction(d)).unwrap_or([0.0, 0.0, 1.0]);
                let depth = args.get(3).and_then(real).unwrap_or(0.0);
                extrude(&outer, &holes, direction, depth, &position)
            }
            "IFCFACETEDBREP" | "IFCFACETEDBREPWITHVOIDS" => {
                let outer = args.first().and_then(reference);
                let voids = args.get(1).map(references).unwrap_or_default();
                self.shells(outer.into_iter().chain(voids).collect())
            }
            "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => self.shells(args.first().map(references).unwrap_or_default()),
            "IFCCLOSEDSHELL" | "IFCOPENSHELL" | "IFCCONNECTEDFACESET" => self.faces(args.first().map(references).unwrap_or_default()),
            "IFCTRIANGULATEDFACESET" | "IFCPOLYGONALFACESET" => self.face_set(name, args),
            "IFCMAPPEDITEM" if depth < 16 => {
                let Some((_, map)) = args.first().and_then(reference).and_then(|m| self.args(m)) else { return (Vec::new(), style) };
                let origin = map.first().and_then(reference).map(|p| self.placement(p)).unwrap_or(M::IDENTITY);
                let target = args.get(1).and_then(reference).map(|o| self.operator(o)).unwrap_or(M::IDENTITY);
                let inner = frame.mul(&target).mul(&origin);
                let mut out = Vec::new();
                let mut inner_style = None;
                let items = map.get(1).and_then(reference).and_then(|r| self.args(r)).and_then(|(_, r)| r.get(3)).map(references);
                for item in items.unwrap_or_default() {
                    let (t, s) = self.item(item, &inner, depth + 1);
                    out.extend(t);
                    inner_style = inner_style.or(s);
                }
                // Already in the element's space.
                return (out, style.or(inner_style));
            }
            "IFCBOOLEANCLIPPINGRESULT" | "IFCBOOLEANRESULT" if depth < 16 => {
                let Some(first) = args.get(1).and_then(reference) else { return (Vec::new(), style) };
                let second = args.get(2).and_then(reference);
                let (mut t, s) = self.item(first, frame, depth + 1);
                match (args.first(), second) {
                    (Some(Parameter::Enumeration(op)), Some(second)) if op == "UNION" => {
                        t.extend(self.item(second, frame, depth + 1).0);
                    }
                    (Some(Parameter::Enumeration(op)), Some(second)) if op == "DIFFERENCE" => match self.cutters(second, frame, depth + 1) {
                        Some(cutters) => t = carve(t, &cutters),
                        None => self.clipped.set(self.clipped.get() + 1),
                    },
                    _ => self.clipped.set(self.clipped.get() + 1),
                }
                return (t, style.or(s));
            }
            other => {
                self.skip(other);
                Vec::new()
            }
        };
        (triangles.into_iter().map(|t| t.map(|p| frame.point(p))).collect(), style)
    }

    /// `IfcHalfSpaceSolid`'s plane, facing out of the half it fills.
    fn half_space(&self, args: &[Parameter], frame: &M) -> Option<Plane> {
        let Some(("IFCPLANE", plane)) = args.first().and_then(reference).and_then(|p| self.args(p)) else { return None };
        let at = frame.mul(&self.placement(reference(plane.first()?)?));
        let origin = at.point([0.0; 3]);
        let n = normalized(at.vector([0.0, 0.0, 1.0]))?;
        // The agreement flag is true when the normal faces away from the
        // half-space's material.
        let n = match args.get(1) {
            Some(Parameter::Enumeration(flag)) if flag == "F" => scale(n, -1.0),
            _ => n,
        };
        Some(Plane { n, d: dot(n, origin) })
    }

    /// The convex volumes an item takes away, in `frame`: a half-space, a
    /// half-space bounded by a polygon, or a solid, split into convex parts
    /// where it is not convex itself. `None` for what cannot be.
    fn cutters(&self, id: u64, frame: &M, depth: usize) -> Option<Vec<Cutter>> {
        let (name, args) = self.args(id)?;
        let convex_pieces = |outer: Vec<[f64; 2]>, holes: &[Vec<[f64; 2]>]| -> Vec<Vec<[f64; 2]>> {
            if holes.is_empty() && is_convex(&outer) {
                vec![outer]
            } else {
                triangulate(&outer, holes).into_iter().map(|t| t.to_vec()).collect()
            }
        };
        match name {
            "IFCHALFSPACESOLID" | "IFCBOXEDHALFSPACE" => Some(vec![vec![self.half_space(args, frame)?]]),
            "IFCPOLYGONALBOUNDEDHALFSPACE" => {
                let base = self.half_space(args, frame)?;
                let position = frame.mul(&self.placement(reference(args.get(2)?)?));
                let axis = normalized(position.vector([0.0, 0.0, 1.0]))?;
                let boundary = self.curve(reference(args.get(3)?)?)?;
                let cutters = convex_pieces(boundary, &[])
                    .into_iter()
                    .map(|piece| {
                        // Each edge and the axis make a side; edge × axis
                        // faces right of the edge, out of a loop wound
                        // counter-clockwise.
                        let outward = if signed_area(&piece) > 0.0 { 1.0 } else { -1.0 };
                        let mut cutter = vec![base];
                        for i in 0..piece.len() {
                            let (p, q) = (piece[i], piece[(i + 1) % piece.len()]);
                            let a = position.point([p[0], p[1], 0.0]);
                            let b = position.point([q[0], q[1], 0.0]);
                            if let Some(n) = normalized(cross(sub(b, a), axis)) {
                                let n = scale(n, outward);
                                cutter.push(Plane { n, d: dot(n, a) });
                            }
                        }
                        cutter
                    })
                    .collect();
                Some(cutters)
            }
            "IFCEXTRUDEDAREASOLID" => {
                let (outer, holes) = self.profile(reference(args.first()?)?)?;
                let position = frame.mul(&args.get(1).and_then(reference).map(|p| self.placement(p)).unwrap_or(M::IDENTITY));
                let direction = args.get(2).and_then(reference).and_then(|d| self.direction(d)).unwrap_or([0.0, 0.0, 1.0]);
                let depth = args.get(3).and_then(real)?;
                convex_pieces(outer, &holes).iter().map(|piece| convex_planes(&extrude(piece, &[], direction, depth, &position))).collect()
            }
            _ => convex_planes(&self.item(id, frame, depth + 1).0).map(|c| vec![c]),
        }
    }

    /// The items of a product's body: the representation called so, or
    /// any that is not an axis, a box, a footprint or an outline.
    fn body_items(&self, product: &[Parameter]) -> Vec<u64> {
        let representations: Vec<u64> =
            product.get(6).and_then(reference).and_then(|s| self.args(s)).and_then(|(_, a)| a.get(2)).map(references).unwrap_or_default();
        let identifier = |r: u64| self.args(r).and_then(|(_, a)| a.get(1).and_then(string));
        let body = representations.iter().copied().find(|&r| identifier(r).as_deref() == Some("Body")).or_else(|| {
            representations.iter().copied().find(|&r| {
                !matches!(identifier(r).as_deref(), Some("Axis" | "Box" | "FootPrint" | "Annotation" | "Profile" | "Clearance"))
            })
        });
        body.and_then(|b| self.args(b)).and_then(|(_, a)| a.get(3)).map(references).unwrap_or_default()
    }

    /// The colour a style resolves to, through presentation style
    /// assignments (IFC2x3) to the surface style and its shading.
    fn style(&self, id: u64, name: Option<String>, depth: usize) -> Option<Style> {
        if depth > 6 {
            return None;
        }
        let (kind, args) = self.args(id)?;
        match kind {
            "IFCSURFACESTYLESHADING" | "IFCSURFACESTYLERENDERING" => {
                let (_, c) = self.args(reference(args.first()?)?)?;
                let rgb = [real(c.get(1)?)? as f32, real(c.get(2)?)? as f32, real(c.get(3)?)? as f32];
                let transparency = args.get(1).and_then(real).unwrap_or(0.0) as f32;
                Some(Style { name, rgb, transparency: transparency.clamp(0.0, 1.0) })
            }
            "IFCSURFACESTYLE" => {
                let name = args.first().and_then(string).filter(|s| !s.trim().is_empty());
                args.get(2).map(references).unwrap_or_default().into_iter().find_map(|s| self.style(s, name.clone(), depth + 1))
            }
            _ => args.iter().flat_map(references).find_map(|r| self.style(r, name.clone(), depth + 1)),
        }
    }
}

/// A list of integers, as index lists are written.
fn integers(p: &Parameter) -> Vec<usize> {
    match p {
        Parameter::List(l) => l.iter().filter_map(real).map(|v| v as usize).collect(),
        other => real(other).map(|v| vec![v as usize]).unwrap_or_default(),
    }
}

/// An `IfcValue` as text: a label's string, a number, a boolean.
fn value_text(p: &Parameter) -> Option<String> {
    match p {
        Parameter::Typed { parameter, .. } => value_text(parameter),
        Parameter::String(_) => string(p),
        Parameter::Real(v) => Some(format!("{v}")),
        Parameter::Integer(v) => Some(format!("{v}")),
        Parameter::Enumeration(e) => Some(match e.as_str() {
            "T" => "true".into(),
            "F" => "false".into(),
            "U" => "unknown".into(),
            other => other.to_ascii_lowercase(),
        }),
        _ => None,
    }
}

/// `IFCWALLSTANDARDCASE` as `IfcWallStandardCase`, for the common names;
/// others as `IfcSomething`.
fn pretty(kind: &str) -> String {
    const KNOWN: &[&str] = &[
        "Wall", "WallStandardCase", "Slab", "Roof", "Window", "Door", "Beam", "Column", "Member", "Plate", "Stair",
        "StairFlight", "Ramp", "RampFlight", "Railing", "Covering", "CurtainWall", "Footing", "Pile", "BuildingElementProxy",
        "FurnishingElement", "Furniture", "FlowTerminal", "FlowSegment", "FlowFitting", "DistributionElement", "Site",
        "Building", "BuildingStorey", "Project", "Space", "Chimney", "Shading", "SanitaryTerminal", "LightFixture",
    ];
    let rest = kind.strip_prefix("IFC").unwrap_or(kind);
    if let Some(k) = KNOWN.iter().find(|k| k.eq_ignore_ascii_case(rest)) {
        return format!("Ifc{k}");
    }
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) => format!("Ifc{}{}", first, chars.as_str().to_ascii_lowercase()),
        None => "Ifc".into(),
    }
}

const SPATIAL: &[&str] = &["IFCPROJECT", "IFCSITE", "IFCBUILDING", "IFCBUILDINGSTOREY"];
/// Products that are volumes or annotation rather than things to render.
const LEFT_OUT: &[&str] = &["IFCSPACE", "IFCOPENINGELEMENT", "IFCVIRTUALELEMENT", "IFCANNOTATION", "IFCGRID", "IFCSPATIALZONE"];

/// Read an IFC file.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let text = String::from_utf8_lossy(bytes);
    let data = crate::part21::data_section(&text)
        .map_err(|e| ImportError::Malformed(format!("this is not an IFC file Tequila can read: {e}")))?;
    let entities = Entities::new(&data.entities);

    // The length unit.
    let mut source_unit = None;
    for (_, record) in entities.named("IFCSIUNIT") {
        let args = arguments(record);
        if matches!(args.get(1), Some(Parameter::Enumeration(e)) if e == "LENGTHUNIT") {
            source_unit = Some(match args.get(2) {
                Some(Parameter::Enumeration(p)) if p == "MILLI" => LengthUnit::Millimetre,
                Some(Parameter::Enumeration(p)) if p == "CENTI" => LengthUnit::Centimetre,
                _ => LengthUnit::Metre,
            });
        }
    }
    for (_, record) in entities.named("IFCCONVERSIONBASEDUNIT") {
        let args = arguments(record);
        if matches!(args.get(1), Some(Parameter::Enumeration(e)) if e == "LENGTHUNIT") {
            match args.get(2).and_then(string).map(|s| s.to_ascii_uppercase()).as_deref() {
                Some("FOOT" | "FT") => source_unit = Some(LengthUnit::Foot),
                Some("INCH" | "IN") => source_unit = Some(LengthUnit::Inch),
                _ => {}
            }
        }
    }

    let mut reader = Reader { entities, styles: BTreeMap::new(), skipped: RefCell::new(BTreeMap::new()), clipped: Cell::new(0) };
    // Styles, by the item they style.
    let mut styles = BTreeMap::new();
    for (_, record) in reader.entities.named("IFCSTYLEDITEM") {
        let args = arguments(record);
        let Some(item) = args.first().and_then(reference) else { continue };
        if let Some(style) = args.get(1).map(references).unwrap_or_default().into_iter().find_map(|s| reader.style(s, None, 0)) {
            styles.insert(item, style);
        }
    }
    reader.styles = styles;

    // What contains what: decomposition first, then containment.
    let mut parent_of: BTreeMap<u64, u64> = BTreeMap::new();
    for (_, record) in reader.entities.named("IFCRELAGGREGATES") {
        let args = arguments(record);
        if let Some(parent) = args.get(4).and_then(reference) {
            for child in args.get(5).map(references).unwrap_or_default() {
                parent_of.insert(child, parent);
            }
        }
    }
    for (_, record) in reader.entities.named("IFCRELCONTAINEDINSPATIALSTRUCTURE") {
        let args = arguments(record);
        if let Some(parent) = args.get(5).and_then(reference) {
            for child in args.get(4).map(references).unwrap_or_default() {
                parent_of.entry(child).or_insert(parent);
            }
        }
    }

    // Openings, by the elements they are cut from.
    let mut openings: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for (_, record) in reader.entities.named("IFCRELVOIDSELEMENT") {
        let args = arguments(record);
        if let (Some(element), Some(opening)) = (args.get(4).and_then(reference), args.get(5).and_then(reference)) {
            openings.entry(element).or_default().push(opening);
        }
    }
    let mut uncut = 0usize;

    // Property sets, by the objects they describe.
    let mut properties: BTreeMap<u64, Vec<(String, String)>> = BTreeMap::new();
    for (_, record) in reader.entities.named("IFCRELDEFINESBYPROPERTIES") {
        let args = arguments(record);
        let Some(("IFCPROPERTYSET", set)) = args.get(5).and_then(reference).and_then(|s| reader.args(s)) else { continue };
        let set_name = set.get(2).and_then(string).unwrap_or_default();
        let mut pairs = Vec::new();
        for property in set.get(4).map(references).unwrap_or_default() {
            let Some(("IFCPROPERTYSINGLEVALUE", p)) = reader.args(property) else { continue };
            if let (Some(key), Some(value)) = (p.first().and_then(string), p.get(2).and_then(value_text)) {
                pairs.push((format!("{set_name}.{key}"), value));
            }
        }
        for object in args.get(4).map(references).unwrap_or_default() {
            properties.entry(object).or_default().extend(pairs.iter().cloned());
        }
    }

    // Every object read: the spatial structure, and products placed and
    // shaped.
    let mut objects: Vec<(u64, &str)> = Vec::new();
    for id in reader.entities.ids() {
        let Some((kind, args)) = reader.args(id) else { continue };
        if SPATIAL.contains(&kind) {
            objects.push((id, kind));
            continue;
        }
        if LEFT_OUT.contains(&kind) || !kind.starts_with("IFC") {
            continue;
        }
        let is = |i: usize, want: &str| args.get(i).and_then(reference).and_then(|r| reader.args(r)).is_some_and(|(n, _)| n == want);
        if is(5, "IFCLOCALPLACEMENT") && is(6, "IFCPRODUCTDEFINITIONSHAPE") {
            objects.push((id, kind));
        }
    }
    let kinds: BTreeMap<u64, &str> = objects.iter().copied().collect();
    // The nearest container read: an element in a space belongs to the
    // space's storey.
    let container = |id: u64| {
        let mut at = id;
        for _ in 0..64 {
            at = *parent_of.get(&at)?;
            if kinds.contains_key(&at) {
                return Some(at);
            }
        }
        None
    };
    let depth_of = |id: u64| {
        let mut d = 0;
        let mut at = id;
        while let Some(parent) = container(at) {
            at = parent;
            d += 1;
            if d > 64 {
                break;
            }
        }
        d
    };
    // Parents before children.
    objects.sort_by_key(|(id, _)| (depth_of(*id), *id));

    let mut scene = RawScene { source_unit, ..RawScene::default() };
    // Z up to Y up, around everything.
    let mut top = RawNode::group(name);
    let mut turn = Mat4::IDENTITY;
    turn.m[1] = [0.0, 0.0, 1.0, 0.0];
    turn.m[2] = [0.0, -1.0, 0.0, 0.0];
    top.transform = Transform::from_matrix(turn).unwrap_or(Transform::IDENTITY);
    scene.nodes.push(top);
    scene.roots.push(0);

    let mut materials: BTreeMap<String, usize> = BTreeMap::new();
    let mut node_of: BTreeMap<u64, usize> = BTreeMap::new();
    // Where each element sits, for its children's.
    let mut placed_at: BTreeMap<u64, M> = BTreeMap::new();
    // A georeferenced model sits kilometres out, where f32 has millimetres
    // to spare at best: everything is moved by the first element's origin.
    let far = 1.0e4 / source_unit.unwrap_or(LengthUnit::Metre).to_metres();
    let mut shift: Option<[f64; 3]> = None;
    let mut warnings = Vec::new();

    for (id, kind) in objects {
        let Some((_, args)) = reader.args(id) else { continue };
        let label = args.get(2).and_then(string).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| pretty(kind));
        let parent = container(id);
        let parent_node = parent.and_then(|p| node_of.get(&p).copied()).unwrap_or(0);
        let mut metadata = BTreeMap::new();
        metadata.insert("IFC type".to_string(), pretty(kind));
        if let Some(g) = args.first().and_then(string) {
            metadata.insert("GlobalId".to_string(), g);
        }
        if let Some(t) = args.get(4).and_then(string).filter(|s| !s.is_empty()) {
            metadata.insert("ObjectType".to_string(), t);
        }
        for (k, v) in properties.get(&id).cloned().unwrap_or_default() {
            metadata.insert(k, v);
        }

        if SPATIAL.contains(&kind) {
            // Groups only: every element's placement already runs through
            // its storey's, building's and site's.
            let mut node = RawNode::group(label);
            node.parent = Some(parent_node);
            node.metadata = metadata;
            scene.nodes.push(node);
            node_of.insert(id, scene.nodes.len() - 1);
            continue;
        }
        if let Some(t) = args.get(7).and_then(string).filter(|s| !s.is_empty()) {
            metadata.insert("Tag".to_string(), t);
        }

        let placement = args.get(5).and_then(reference).map(|p| reader.local_placement(p, 0)).unwrap_or(M::IDENTITY);
        let shift = *shift.get_or_insert_with(|| {
            let o = placement.point([0.0; 3]);
            if dot(o, o).sqrt() > far {
                warnings.push("the model sits far from its origin and is brought to it".to_string());
                o
            } else {
                [0.0; 3]
            }
        });
        // An element inside another (a flight in a stair) sits relative to
        // it; one in the spatial structure, absolutely.
        let transform = match parent.and_then(|p| placed_at.get(&p)) {
            Some(outer) => outer.rigid_inverse().mul(&placement).to_transform([0.0; 3]),
            None => placement.to_transform(shift),
        };
        placed_at.insert(id, placement);

        let mut triangles: Triangles = Vec::new();
        let mut style = None;
        for item in reader.body_items(args) {
            let (t, s) = reader.item(item, &M::IDENTITY, 0);
            triangles.extend(t);
            style = style.or(s);
        }
        // Its openings, taken away.
        if let Some(list) = openings.get(&id).filter(|_| !triangles.is_empty()) {
            let inverse = placement.rigid_inverse();
            let mut cutters = Vec::new();
            for &opening in list {
                let Some((_, o)) = reader.args(opening) else { continue };
                let at = inverse.mul(&o.get(5).and_then(reference).map(|p| reader.local_placement(p, 0)).unwrap_or(M::IDENTITY));
                for item in reader.body_items(o) {
                    match reader.cutters(item, &at, 0) {
                        Some(c) => cutters.extend(c),
                        None => uncut += 1,
                    }
                }
            }
            triangles = carve(triangles, &cutters);
        }

        let mut node = if triangles.is_empty() {
            // Kept as a group for what it may contain.
            RawNode::group(label)
        } else {
            let mut mesh = RawMesh { name: label.clone(), ..RawMesh::default() };
            for p in triangles.iter().flatten() {
                mesh.indices.push(mesh.positions.len() as u32);
                mesh.positions.push(vec3(p[0] as f32, p[1] as f32, p[2] as f32));
            }
            mesh.material = Some(material_for(style, kind, &mut materials, &mut scene.materials));
            scene.meshes.push(mesh);
            RawNode::part(label, scene.meshes.len() - 1)
        };
        node.parent = Some(parent_node);
        node.transform = transform;
        node.metadata = metadata;
        scene.nodes.push(node);
        node_of.insert(id, scene.nodes.len() - 1);
    }

    let skipped = reader.skipped.borrow();
    if !skipped.is_empty() {
        let list: Vec<String> = skipped.iter().map(|(k, n)| format!("{} ×{n}", pretty(k))).collect();
        warnings.push(format!("geometry Tequila does not read was left out: {}", list.join(", ")));
    }
    if reader.clipped.get() > 0 {
        warnings.push(format!("{} boolean bodies are drawn uncut (a cut Tequila cannot make)", reader.clipped.get()));
    }
    if uncut > 0 {
        warnings.push(format!("{uncut} openings are not cut (their shape is not one Tequila can take away)"));
    }
    if scene.meshes.is_empty() {
        return Err(ImportError::Unsupported("the IFC file has no element bodies Tequila can read".into()));
    }
    scene.warnings = warnings;
    Ok(scene)
}

/// The material for an element: its style's, made once per style, or a
/// plain default for its kind.
fn material_for(style: Option<Style>, kind: &str, made: &mut BTreeMap<String, usize>, definitions: &mut Vec<MaterialDefinition>) -> usize {
    let style = style.unwrap_or_else(|| {
        let (name, rgb, transparency) = match kind {
            "IFCWINDOW" | "IFCPLATE" => ("Glazing", [0.72, 0.82, 0.86], 0.8),
            "IFCDOOR" | "IFCFURNISHINGELEMENT" | "IFCFURNITURE" => ("Timber", [0.62, 0.48, 0.34], 0.0),
            "IFCSLAB" | "IFCROOF" | "IFCSTAIR" | "IFCSTAIRFLIGHT" | "IFCFOOTING" => ("Concrete", [0.66, 0.65, 0.62], 0.0),
            "IFCBEAM" | "IFCCOLUMN" | "IFCMEMBER" | "IFCRAILING" => ("Structure", [0.78, 0.78, 0.76], 0.0),
            _ => ("Plaster", [0.88, 0.87, 0.84], 0.0),
        };
        Style { name: Some(name.into()), rgb, transparency }
    });
    let hex = format!("#{:02X}{:02X}{:02X}", to_byte(style.rgb[0]), to_byte(style.rgb[1]), to_byte(style.rgb[2]));
    let name = style.name.clone().unwrap_or_else(|| hex.clone());
    let key = format!("{name}\u{0}{hex}\u{0}{}", (style.transparency * 100.0).round());
    *made.entry(key).or_insert_with(|| {
        let colour = Spectrum::from_srgb_encoded(vec3(style.rgb[0], style.rgb[1], style.rgb[2]));
        let definition = if style.transparency > 0.3 {
            let mut d = MaterialDefinition::preset(name, MaterialType::Glass);
            d.material_mut().transmission_color = colour;
            d
        } else {
            let mut d = MaterialDefinition::preset(name, MaterialType::Plastic);
            let m = d.material_mut();
            m.base_color = colour;
            m.specular_roughness = 0.6;
            if style.transparency > 0.01 {
                m.geometry_opacity = 1.0 - style.transparency;
            }
            d
        };
        definitions.push(definition);
        definitions.len() - 1
    })
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::Vec3;

    /// A small house: one storey with a wall (an extruded rectangle), a
    /// floor with a hole (a polyline profile with a void), a blue window (a
    /// triangulated face set), and two chairs placed from one definition.
    /// Millimetres. The wall has a property set.
    const HOUSE: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('house.ifc','2026-10-11',(''),(''),'','','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('0proj',$,'Little house',$,$,$,$,$,#5);
#5=IFCUNITASSIGNMENT((#6));
#6=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#10=IFCCARTESIANPOINT((0.,0.,0.));
#11=IFCDIRECTION((0.,0.,1.));
#12=IFCDIRECTION((1.,0.,0.));
#13=IFCAXIS2PLACEMENT3D(#10,#11,#12);
#14=IFCLOCALPLACEMENT($,#13);
#20=IFCSITE('0site',$,'Site',$,$,#14,$,$,.ELEMENT.,$,$,$,$,$);
#21=IFCBUILDING('0bldg',$,'House',$,$,#14,$,$,.ELEMENT.,$,$,$);
#22=IFCBUILDINGSTOREY('0stor',$,'Ground floor',$,$,#14,$,$,.ELEMENT.,0.);
#23=IFCRELAGGREGATES('r1',$,$,$,#1,(#20));
#24=IFCRELAGGREGATES('r2',$,$,$,#20,(#21));
#25=IFCRELAGGREGATES('r3',$,$,$,#21,(#22));
#29=IFCCARTESIANPOINT((0.,0.,0.));
#30=IFCCARTESIANPOINT((1000.,0.,0.));
#31=IFCAXIS2PLACEMENT3D(#30,$,$);
#32=IFCLOCALPLACEMENT(#14,#31);
#38=IFCAXIS2PLACEMENT3D(#29,$,$);
#33=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);
#34=IFCEXTRUDEDAREASOLID(#33,#38,#11,2500.);
#35=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#34));
#36=IFCPRODUCTDEFINITIONSHAPE($,$,(#35));
#37=IFCWALL('0wall',$,'Front wall',$,$,#32,#36,'W-01',$);
#40=IFCCARTESIANPOINT((0.,0.));
#41=IFCCARTESIANPOINT((5000.,0.));
#42=IFCCARTESIANPOINT((5000.,4000.));
#43=IFCCARTESIANPOINT((0.,4000.));
#44=IFCPOLYLINE((#40,#41,#42,#43,#40));
#45=IFCCARTESIANPOINT((2000.,1500.));
#46=IFCCARTESIANPOINT((2000.,2500.));
#47=IFCCARTESIANPOINT((3000.,2500.));
#48=IFCCARTESIANPOINT((3000.,1500.));
#49=IFCPOLYLINE((#45,#46,#47,#48,#45));
#50=IFCARBITRARYPROFILEDEFWITHVOIDS(.AREA.,$,#44,(#49));
#51=IFCDIRECTION((0.,0.,-1.));
#52=IFCEXTRUDEDAREASOLID(#50,#38,#51,200.);
#53=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#52));
#54=IFCPRODUCTDEFINITIONSHAPE($,$,(#53));
#55=IFCSLAB('0slab',$,'Floor',$,$,#14,#54,$,.FLOOR.);
#60=IFCCARTESIANPOINTLIST3D(((0.,0.,1000.),(1000.,0.,1000.),(1000.,0.,2000.),(0.,0.,2000.)));
#61=IFCTRIANGULATEDFACESET(#60,$,$,((1,2,3),(1,3,4)),$);
#62=IFCSHAPEREPRESENTATION($,'Body','Tessellation',(#61));
#63=IFCPRODUCTDEFINITIONSHAPE($,$,(#62));
#64=IFCWINDOW('0wind',$,'Window',$,$,#14,#63,$,$,$,$,$,$);
#65=IFCCOLOURRGB($,0.2,0.4,0.9);
#66=IFCSURFACESTYLERENDERING(#65,0.,$,$,$,$,$,$,.NOTDEFINED.);
#67=IFCSURFACESTYLE('Blue panel',.BOTH.,(#66));
#68=IFCSTYLEDITEM(#61,(#67),$);
#70=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(400.,0.,0.),(400.,400.,0.),(0.,400.,0.)));
#71=IFCINDEXEDPOLYGONALFACE((1,2,3,4));
#72=IFCPOLYGONALFACESET(#70,$,(#71),$);
#73=IFCSHAPEREPRESENTATION($,'Body','Tessellation',(#72));
#74=IFCREPRESENTATIONMAP(#38,#73);
#75=IFCCARTESIANPOINT((500.,500.,0.));
#76=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#75,$,$);
#77=IFCMAPPEDITEM(#74,#76);
#78=IFCSHAPEREPRESENTATION($,'Body','MappedRepresentation',(#77));
#79=IFCPRODUCTDEFINITIONSHAPE($,$,(#78));
#80=IFCFURNISHINGELEMENT('0ch1',$,'Chair',$,$,#14,#79,$);
#81=IFCCARTESIANPOINT((2500.,500.,0.));
#82=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#81,$,$);
#83=IFCMAPPEDITEM(#74,#82);
#84=IFCSHAPEREPRESENTATION($,'Body','MappedRepresentation',(#83));
#85=IFCPRODUCTDEFINITIONSHAPE($,$,(#84));
#86=IFCFURNISHINGELEMENT('0ch2',$,'Chair',$,$,#14,#85,$);
#90=IFCRELCONTAINEDINSPATIALSTRUCTURE('c1',$,$,$,(#37,#55,#64,#80,#86),#22);
#91=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);
#92=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('REI 60'),$);
#93=IFCPROPERTYSET('ps1',$,'Pset_WallCommon',$,(#91,#92));
#94=IFCRELDEFINESBYPROPERTIES('d1',$,$,$,(#37),#93);
ENDSEC;
END-ISO-10303-21;
";

    fn node<'a>(scene: &'a RawScene, name: &str) -> &'a RawNode {
        scene.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no {name}"))
    }

    fn area(mesh: &RawMesh) -> f64 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                ((b - a).cross(c - a).length() * 0.5) as f64
            })
            .sum()
    }

    /// The volume a closed mesh encloses, by the divergence theorem:
    /// positive only when every face faces out.
    fn volume(mesh: &RawMesh) -> f64 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
                (a.dot(b.cross(c)) / 6.0) as f64
            })
            .sum()
    }

    #[test]
    fn a_small_house_reads() {
        let scene = read(HOUSE.as_bytes(), "house").expect("reads");
        scene.validate().expect("valid");
        assert_eq!(scene.source_unit, Some(LengthUnit::Millimetre));
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);

        // The structure: project, site, building, storey, then the elements.
        let mut chain = Vec::new();
        let mut at = scene.nodes.iter().position(|n| n.name == "Front wall");
        while let Some(i) = at {
            chain.push(scene.nodes[i].name.as_str());
            at = scene.nodes[i].parent;
        }
        assert_eq!(chain, ["Front wall", "Ground floor", "House", "Site", "Little house", "house"]);
        let wall = node(&scene, "Front wall");
        assert_eq!(wall.metadata.get("Pset_WallCommon.IsExternal").map(String::as_str), Some("true"));
        assert_eq!(wall.metadata.get("Pset_WallCommon.FireRating").map(String::as_str), Some("REI 60"));
        assert_eq!(wall.metadata.get("IFC type").map(String::as_str), Some("IfcWall"));
        assert_eq!(wall.metadata.get("GlobalId").map(String::as_str), Some("0wall"));
        assert_eq!(wall.metadata.get("Tag").map(String::as_str), Some("W-01"));

        // The wall: a 4000 x 200 x 2500 box, closed and facing out, at x = 1000.
        let wall_mesh = &scene.meshes[wall.mesh.unwrap()];
        let expected = 2.0 * (4000.0 * 200.0 + 4000.0 * 2500.0 + 200.0 * 2500.0);
        assert!((area(wall_mesh) - expected).abs() < expected * 1e-4, "wall area {} not {expected}", area(wall_mesh));
        let solid = 4000.0 * 200.0 * 2500.0;
        assert!((volume(wall_mesh) - solid).abs() < solid * 1e-5, "wall volume {}", volume(wall_mesh));
        assert!((wall.transform.point(Vec3::ZERO) - vec3(1000.0, 0.0, 0.0)).length() < 1e-3);

        // The floor: 5 x 4 m with a 1 x 1 m hole, 200 thick, downwards, and
        // still facing out.
        let floor = &scene.meshes[node(&scene, "Floor").mesh.unwrap()];
        let caps = 2.0 * (5000.0 * 4000.0 - 1000.0 * 1000.0);
        let sides = 200.0 * (2.0 * (5000.0 + 4000.0) + 4.0 * 1000.0);
        assert!((area(floor) - (caps + sides)).abs() < (caps + sides) * 1e-4, "floor area {}", area(floor));
        let solid = (5000.0 * 4000.0 - 1000.0 * 1000.0) * 200.0;
        assert!((volume(floor) - solid).abs() < solid * 1e-5, "floor volume {} not {solid}", volume(floor));
        assert!(floor.positions.iter().all(|p| p.z <= 1e-3 && p.z >= -200.0 - 1e-3), "the floor goes down from the storey");

        // The window: two triangles in its own style.
        let window = &scene.meshes[node(&scene, "Window").mesh.unwrap()];
        assert_eq!(window.indices.len(), 6);
        let material = &scene.materials[window.material.unwrap()];
        assert_eq!(material.name, "Blue panel");
        let blue = material.material().base_color.to_linear_srgb();
        assert!(blue.z > blue.x * 3.0, "the window is not blue: {blue:?}");

        // Two chairs, one definition, placed apart.
        let chairs: Vec<&RawNode> = scene.nodes.iter().filter(|n| n.name == "Chair").collect();
        assert_eq!(chairs.len(), 2);
        let first = &scene.meshes[chairs[0].mesh.unwrap()];
        let second = &scene.meshes[chairs[1].mesh.unwrap()];
        let corner = |m: &RawMesh| m.positions.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
        assert!((corner(first) - vec3(500.0, 500.0, 0.0)).length() < 1e-3, "{:?}", corner(first));
        assert!((corner(second) - vec3(2500.0, 500.0, 0.0)).length() < 1e-3, "{:?}", corner(second));
        assert!((area(first) - 400.0 * 400.0).abs() < 1.0);
    }

    #[test]
    fn a_face_with_a_hole_is_triangulated_around_it() {
        let outer = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let hole = vec![[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]];
        let triangles = triangulate(&outer, &[hole]);
        let signed = |[a, b, c]: &[[f64; 2]; 3]| ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) * 0.5;
        let area: f64 = triangles.iter().map(signed).sum();
        assert!((area - 12.0).abs() < 1e-9, "area {area}");
        assert!(triangles.iter().all(|t| signed(t) > 0.0), "all counter-clockwise");
    }

    #[test]
    fn every_profile_extrudes_to_a_closed_outward_solid() {
        let profiles = [
            "IFCRECTANGLEHOLLOWPROFILEDEF(.AREA.,$,$,100.,60.,5.,$,$)",
            "IFCCIRCLEPROFILEDEF(.AREA.,$,$,50.)",
            "IFCCIRCLEHOLLOWPROFILEDEF(.AREA.,$,$,50.,4.)",
            "IFCELLIPSEPROFILEDEF(.AREA.,$,$,80.,30.)",
            "IFCISHAPEPROFILEDEF(.AREA.,$,$,150.,300.,7.1,10.7,$,$,$)",
            "IFCLSHAPEPROFILEDEF(.AREA.,$,$,100.,80.,10.,$,$,$,$)",
        ];
        for profile in profiles {
            let file = format!(
                "ISO-10303-21;HEADER;FILE_SCHEMA(('IFC4'));ENDSEC;DATA;
#10=IFCCARTESIANPOINT((0.,0.,0.));#13=IFCAXIS2PLACEMENT3D(#10,$,$);#14=IFCLOCALPLACEMENT($,#13);
#11=IFCDIRECTION((0.,0.,1.));#33={profile};#34=IFCEXTRUDEDAREASOLID(#33,#13,#11,1000.);
#35=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#34));#36=IFCPRODUCTDEFINITIONSHAPE($,$,(#35));
#37=IFCMEMBER('m',$,'Member',$,$,#14,#36,$,$);ENDSEC;END-ISO-10303-21;"
            );
            let scene = read(file.as_bytes(), "member").unwrap_or_else(|e| panic!("{profile}: {e}"));
            let mesh = &scene.meshes[0];
            let v = volume(mesh);
            assert!(v > 0.0, "{profile}: faces inward, volume {v}");
            // Every edge used once each way: closed and consistently wound.
            let key = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            let mut edges: BTreeMap<([u32; 3], [u32; 3]), i32> = BTreeMap::new();
            for t in mesh.indices.chunks(3) {
                for i in 0..3 {
                    let (a, b) = (key(mesh.positions[t[i] as usize]), key(mesh.positions[t[(i + 1) % 3] as usize]));
                    *edges.entry((a.min(b), a.max(b))).or_insert(0) += if a < b { 1 } else { -1 };
                }
            }
            assert!(edges.values().all(|&n| n == 0), "{profile}: not closed");
        }
    }

    #[test]
    fn a_model_far_from_its_origin_is_brought_back() {
        let file = HOUSE.replace("#10=IFCCARTESIANPOINT((0.,0.,0.));", "#10=IFCCARTESIANPOINT((512000000.,6100000000.,30000.));");
        let scene = read(file.as_bytes(), "house").expect("reads");
        assert!(scene.warnings.iter().any(|w| w.contains("far from its origin")), "{:?}", scene.warnings);
        // Near the origin, and still a metre apart to the tenth of a millimetre.
        let wall = node(&scene, "Front wall").transform.point(Vec3::ZERO);
        let floor = node(&scene, "Floor").transform.point(Vec3::ZERO);
        assert!(wall.length() < 1e4 && floor.length() < 1e4, "{wall:?} {floor:?}");
        assert!((wall - floor - vec3(1000.0, 0.0, 0.0)).length() < 0.1, "{wall:?} {floor:?}");
    }

    /// A wall 4000 long (x -2000 to 2000), 200 thick (y -100 to 100) and
    /// 2500 high, its body `body` (the plain extrusion is #34), with
    /// `extra` entities. Millimetres.
    fn wall(body: &str, extra: &str) -> RawMesh {
        let file = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('IFC4'));ENDSEC;DATA;
#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#10=IFCCARTESIANPOINT((0.,0.,0.));#11=IFCDIRECTION((0.,0.,1.));#12=IFCDIRECTION((1.,0.,0.));
#13=IFCAXIS2PLACEMENT3D(#10,$,$);#14=IFCLOCALPLACEMENT($,#13);
#33=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);#34=IFCEXTRUDEDAREASOLID(#33,#13,#11,2500.);
#35=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',({body}));#36=IFCPRODUCTDEFINITIONSHAPE($,$,(#35));
#37=IFCWALL('w',$,'Wall',$,$,#14,#36,$,$);
{extra}
ENDSEC;END-ISO-10303-21;"
        );
        let scene = read(file.as_bytes(), "wall").expect("reads");
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        scene.meshes[0].clone()
    }

    /// An opening through the wall along +y from `y`, `depth` deep, of
    /// `profile` centred at x 0, z 1500.
    fn opening(profile: &str, y: f64, depth: f64) -> String {
        format!(
            "#50=IFCCARTESIANPOINT((0.,{y:.1},1500.));#51=IFCDIRECTION((0.,1.,0.));#52=IFCAXIS2PLACEMENT3D(#50,#51,#12);
#53={profile};#54=IFCEXTRUDEDAREASOLID(#53,#52,#11,{depth:.1});
#55=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#54));#56=IFCPRODUCTDEFINITIONSHAPE($,$,(#55));
#57=IFCLOCALPLACEMENT(#14,#13);#58=IFCOPENINGELEMENT('o',$,'Opening',$,$,#57,#56,$,.OPENING.);
#59=IFCRELVOIDSELEMENT('v',$,$,$,#37,#58);"
        )
    }

    fn close(what: &str, got: f64, want: f64) {
        assert!((got - want).abs() <= want.abs() * 1e-5, "{what}: {got} not {want}");
    }

    const WALL_AREA: f64 = 2.0 * (4000.0 * 200.0 + 4000.0 * 2500.0 + 200.0 * 2500.0);
    const WALL_VOLUME: f64 = 4000.0 * 200.0 * 2500.0;

    #[test]
    fn an_opening_is_cut_through_a_wall_with_its_reveals() {
        let window = "IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1000.,1200.)";
        // Deeper than the wall, as most tools write it, and exactly as deep.
        for (y, depth) in [(-200.0, 400.0), (-100.0, 200.0)] {
            let mesh = wall("#34", &opening(window, y, depth));
            close("volume", volume(&mesh), WALL_VOLUME - 1000.0 * 200.0 * 1200.0);
            // Two mouths gone, four reveals the wall's thickness deep.
            close("area", area(&mesh), WALL_AREA - 2.0 * 1000.0 * 1200.0 + 2.0 * (1000.0 + 1200.0) * 200.0);
        }
        // A niche, half the wall deep from its face: a mouth, four sides and a back.
        let mesh = wall("#34", &opening(window, -100.0, 100.0));
        close("niche volume", volume(&mesh), WALL_VOLUME - 1000.0 * 100.0 * 1200.0);
        close("niche area", area(&mesh), WALL_AREA + 2.0 * (1000.0 + 1200.0) * 100.0);
    }

    #[test]
    fn an_opening_that_is_not_convex_is_cut_in_convex_parts() {
        // An L, 800 wide and 1000 high with 200 legs: its parts' shared
        // faces must not show.
        let mesh = wall("#34", &opening("IFCLSHAPEPROFILEDEF(.AREA.,$,$,1000.,800.,200.,$,$,$,$)", -200.0, 400.0));
        let l = 800.0 * 200.0 + 800.0 * 200.0;
        close("volume", volume(&mesh), WALL_VOLUME - l * 200.0);
        close("area", area(&mesh), WALL_AREA - 2.0 * l + 2.0 * (1000.0 + 800.0) * 200.0);
    }

    #[test]
    fn a_wall_is_trimmed_by_a_half_space() {
        let clipped = |axis: &str, height: f64, flag: &str| {
            wall(
                "#60",
                &format!(
                    "#60=IFCBOOLEANCLIPPINGRESULT(.DIFFERENCE.,#34,#61);#61=IFCHALFSPACESOLID(#62,{flag});
#62=IFCPLANE(#63);#63=IFCAXIS2PLACEMENT3D(#64,#65,$);#64=IFCCARTESIANPOINT((0.,0.,{height:.1}));#65=IFCDIRECTION(({axis}));"
                ),
            )
        };
        // Flat at 2000: the material of the half-space is above, where the
        // normal points (agreement false).
        let mesh = clipped("0.,0.,1.", 2000.0, ".F.");
        close("volume", volume(&mesh), 4000.0 * 200.0 * 2000.0);
        close("area", area(&mesh), 2.0 * (4000.0 * 200.0 + 4000.0 * 2000.0 + 200.0 * 2000.0));
        // The same half-space by its other normal.
        let mesh = clipped("0.,0.,-1.", 2000.0, ".T.");
        close("flipped volume", volume(&mesh), 4000.0 * 200.0 * 2000.0);
        // Sloping, z = 2000 + x / 10: the same volume, by symmetry.
        let mesh = clipped("-0.1,0.,1.", 2000.0, ".F.");
        close("sloped volume", volume(&mesh), 4000.0 * 200.0 * 2000.0);
        // Exactly at the top: nothing to take, and the top stays.
        let mesh = clipped("0.,0.,1.", 2500.0, ".F.");
        close("flush volume", volume(&mesh), WALL_VOLUME);
        close("flush area", area(&mesh), WALL_AREA);
    }

    #[test]
    fn a_bounded_half_space_trims_only_within_its_polygon() {
        // Above 2000, but only for x from 0 to 3000: half the wall steps down.
        let mesh = wall(
            "#60",
            "#60=IFCBOOLEANCLIPPINGRESULT(.DIFFERENCE.,#34,#61);#61=IFCPOLYGONALBOUNDEDHALFSPACE(#62,.F.,#13,#70);
#62=IFCPLANE(#63);#63=IFCAXIS2PLACEMENT3D(#64,#11,$);#64=IFCCARTESIANPOINT((0.,0.,2000.));
#71=IFCCARTESIANPOINT((0.,-500.));#72=IFCCARTESIANPOINT((3000.,-500.));#73=IFCCARTESIANPOINT((3000.,500.));
#74=IFCCARTESIANPOINT((0.,500.));#70=IFCPOLYLINE((#71,#72,#73,#74,#71));",
        );
        close("volume", volume(&mesh), 2000.0 * 200.0 * 2500.0 + 2000.0 * 200.0 * 2000.0);
        let tops = 2000.0 * 200.0 * 2.0;
        let bottom = 4000.0 * 200.0;
        let faces = 2.0 * (2000.0 * 2500.0 + 2000.0 * 2000.0);
        let ends = 200.0 * 2500.0 + 200.0 * 2000.0;
        let step = 200.0 * 500.0;
        close("area", area(&mesh), tops + bottom + faces + ends + step);
    }
}
