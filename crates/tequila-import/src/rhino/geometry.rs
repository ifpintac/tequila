//! The objects a 3dm file's geometry is made of, read: meshes, B-reps with
//! the curves and surfaces under them, and extrusions.
//!
//! Each reader follows the field order of the matching openNURBS `Read`
//! (`ON_Mesh::Read`, `ON_Brep::Read` and its arrays, `ON_NurbsSurface::Read`
//! and so on), version gates and all; the module documentation of
//! `rhino/mod.rs` lists what is read and what is not.

use super::archive::{uuid, Object, Plane, Reader, Uuid, NIL, R};
use std::sync::Arc;

pub(super) const ON_MESH: Uuid = uuid("4ED7D4E4-E947-11d3-BFE5-0010830122F0");
pub(super) const ON_BREP: Uuid = uuid("60B5DBC5-E660-11d3-BFE4-0010830122F0");
pub(super) const ON_EXTRUSION: Uuid = uuid("36F53175-72B8-4d47-BF1F-B4E6FC24F4B9");
pub(super) const ON_NURBS_CURVE: Uuid = uuid("4ED7D4DD-E947-11d3-BFE5-0010830122F0");
pub(super) const ON_NURBS_SURFACE: Uuid = uuid("4ED7D4DE-E947-11d3-BFE5-0010830122F0");
pub(super) const ON_PLANE_SURFACE: Uuid = uuid("4ED7D4DF-E947-11d3-BFE5-0010830122F0");
const ON_CLIPPING_PLANE_SURFACE: Uuid = uuid("DBC5A584-CE3F-4170-98A8-497069CA5C36");
pub(super) const ON_REV_SURFACE: Uuid = uuid("A16220D3-163B-11d4-8000-0010830122F0");
pub(super) const ON_SUM_SURFACE: Uuid = uuid("C4CD5359-446D-4690-9FF5-29059732472B");
pub(super) const ON_LINE_CURVE: Uuid = uuid("4ED7D4DB-E947-11d3-BFE5-0010830122F0");
pub(super) const ON_ARC_CURVE: Uuid = uuid("CF33BE2A-09B4-11d4-BFFB-0010830122F0");
pub(super) const ON_POLYLINE_CURVE: Uuid = uuid("4ED7D4E6-E947-11d3-BFE5-0010830122F0");
pub(super) const ON_POLY_CURVE: Uuid = uuid("4ED7D4E0-E947-11d3-BFE5-0010830122F0");
/// The obsolete Rhino 5 user data an extrusion kept its meshes in.
const ON_V5_EXTRUSION_DISPLAY_MESH_CACHE: Uuid = uuid("A8130A3E-E4F3-4CB0-BB8A-F10A473912D0");

type P3 = [f64; 3];

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: P3, k: f64) -> P3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: P3, b: P3) -> P3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn length(a: P3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: P3) -> Option<P3> {
    let l = length(a);
    (l > 1e-300 && l.is_finite()).then(|| scale(a, 1.0 / l))
}

// Curves.

/// A curve as openNURBS stores it, in two or three dimensions (a trim's
/// curve is in its surface's parameters, with z zero).
#[derive(Clone, Debug)]
pub(super) enum Curve {
    Nurbs(Nurbs),
    Line { from: P3, to: P3, domain: [f64; 2] },
    /// Points and the parameter at each.
    Polyline { points: Vec<P3>, params: Vec<f64> },
    Arc { centre: P3, x: P3, y: P3, radius: f64, angle: [f64; 2], domain: [f64; 2] },
    /// Segments end to end, and the parameters where each begins and the
    /// last ends: each segment's own domain is laid linearly over its share.
    Poly { segments: Vec<Curve>, params: Vec<f64> },
}

/// A NURBS curve with openNURBS's knots made whole: openNURBS leaves off
/// the first and last knot, which no evaluation reads, and they are put
/// back as copies of their neighbours. Control points are homogeneous,
/// `[x·w, y·w, z·w, w]`, as openNURBS stores rational ones.
#[derive(Clone, Debug)]
pub(super) struct Nurbs {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub points: Vec<[f64; 4]>,
}

impl Nurbs {
    pub fn domain(&self) -> [f64; 2] {
        [self.knots[self.degree], self.knots[self.points.len()]]
    }

    pub fn at(&self, t: f64) -> P3 {
        let (span, weights) = crate::iges::basis(&self.knots, self.degree, self.points.len(), t);
        let mut h = [0.0; 4];
        for (j, w) in weights.iter().enumerate() {
            let p = self.points[span - self.degree + j];
            for c in 0..4 {
                h[c] += w * p[c];
            }
        }
        if h[3].abs() < 1e-300 {
            return [0.0; 3];
        }
        [h[0] / h[3], h[1] / h[3], h[2] / h[3]]
    }

    /// The parameters between `t0` and `t1` where its knot spans meet.
    fn breaks(&self, t0: f64, t1: f64) -> Vec<f64> {
        let (lo, hi) = (t0.min(t1), t0.max(t1));
        let mut out: Vec<f64> = self.knots.iter().copied().filter(|k| *k > lo && *k < hi).collect();
        out.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * (1.0 + b.abs()));
        out
    }
}

/// Whether a spline's numbers hold together: a knot for every point and
/// order, rising, all finite, and every weight positive.
fn sound(degree: usize, knots: &[f64], points: &[[f64; 4]]) -> bool {
    degree >= 1
        && points.len() > degree
        && knots.len() == points.len() + degree + 1
        && knots.iter().all(|k| k.is_finite())
        && knots.windows(2).all(|w| w[1] >= w[0])
        && knots[points.len()] > knots[degree]
        && points.iter().all(|p| p.iter().all(|c| c.is_finite()) && p[3] > 0.0)
}

impl Curve {
    pub fn domain(&self) -> [f64; 2] {
        match self {
            Curve::Nurbs(n) => n.domain(),
            Curve::Line { domain, .. } | Curve::Arc { domain, .. } => *domain,
            Curve::Polyline { params, .. } | Curve::Poly { params, .. } => [params[0], params[params.len() - 1]],
        }
    }

    pub fn at(&self, t: f64) -> P3 {
        match self {
            Curve::Nurbs(n) => n.at(t),
            Curve::Line { from, to, domain } => {
                let f = (t - domain[0]) / (domain[1] - domain[0]);
                add(*from, scale(sub(*to, *from), f))
            }
            Curve::Polyline { points, params } => {
                let i = segment(params, t);
                let f = (t - params[i]) / (params[i + 1] - params[i]);
                add(points[i], scale(sub(points[i + 1], points[i]), f))
            }
            Curve::Arc { centre, x, y, radius, angle, domain } => {
                let a = angle[0] + (t - domain[0]) / (domain[1] - domain[0]) * (angle[1] - angle[0]);
                add(*centre, add(scale(*x, radius * a.cos()), scale(*y, radius * a.sin())))
            }
            Curve::Poly { segments, params } => {
                let i = segment(params, t);
                let d = segments[i].domain();
                let f = (t - params[i]) / (params[i + 1] - params[i]);
                segments[i].at(d[0] + f * (d[1] - d[0]))
            }
        }
    }

    /// The curve from `t0` to `t1` (either way round) as pieces that are
    /// each smooth - a knot span, a straight segment, a quarter turn - with
    /// how many points each needs at least.
    pub fn pieces(&self, t0: f64, t1: f64) -> Vec<(f64, f64, usize)> {
        let (breaks, min): (Vec<f64>, usize) = match self {
            Curve::Nurbs(n) => (n.breaks(t0, t1), (2 * n.degree).saturating_sub(1).max(1)),
            Curve::Line { .. } => (Vec::new(), 1),
            Curve::Polyline { params, .. } => (inside(params, t0, t1), 1),
            Curve::Arc { angle, domain, .. } => {
                // A piece every quarter turn at most.
                let turn = (angle[1] - angle[0]).abs() * ((t1 - t0) / (domain[1] - domain[0])).abs();
                let n = (turn / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
                let breaks = (1..n).map(|i| t0.min(t1) + (t1 - t0).abs() * i as f64 / n as f64).collect();
                (breaks, 4)
            }
            Curve::Poly { segments, params } => {
                // Each segment's own pieces, carried into the polycurve's
                // parameters.
                let (lo, hi) = (t0.min(t1), t0.max(t1));
                let mut out = Vec::new();
                for (i, s) in segments.iter().enumerate() {
                    let (a, b) = (params[i].max(lo), params[i + 1].min(hi));
                    if b <= a {
                        continue;
                    }
                    let d = s.domain();
                    let to_segment = |t: f64| d[0] + (t - params[i]) / (params[i + 1] - params[i]) * (d[1] - d[0]);
                    let from_segment = |s: f64| params[i] + (s - d[0]) / (d[1] - d[0]) * (params[i + 1] - params[i]);
                    for (p, q, m) in s.pieces(to_segment(a), to_segment(b)) {
                        out.push((from_segment(p), from_segment(q), m));
                    }
                }
                if t1 < t0 {
                    out.reverse();
                    for piece in &mut out {
                        *piece = (piece.1, piece.0, piece.2);
                    }
                }
                return out;
            }
        };
        let mut cuts = vec![t0.min(t1)];
        cuts.extend(breaks);
        cuts.push(t0.max(t1));
        let mut out: Vec<(f64, f64, usize)> = cuts.windows(2).filter(|w| w[1] > w[0]).map(|w| (w[0], w[1], min)).collect();
        if t1 < t0 {
            out.reverse();
            for piece in &mut out {
                *piece = (piece.1, piece.0, piece.2);
            }
        }
        out
    }

    /// How many smooth pieces it is over its whole domain, times their
    /// degree: the grid a surface swept along it needs at least.
    fn spans(&self) -> usize {
        let d = self.domain();
        self.pieces(d[0], d[1]).iter().map(|p| p.2.max(1)).sum::<usize>().max(1)
    }

    /// The curve in the plane a transform takes it to: points mapped by
    /// `f`. Only lines, polylines and NURBS stay what they are under any
    /// affine map; an arc is mapped as a frame.
    fn mapped(&self, f: &dyn Fn(P3) -> P3, v: &dyn Fn(P3) -> P3) -> Curve {
        match self {
            Curve::Nurbs(n) => Curve::Nurbs(Nurbs {
                degree: n.degree,
                knots: n.knots.clone(),
                points: n
                    .points
                    .iter()
                    .map(|p| {
                        let q = f([p[0] / p[3], p[1] / p[3], p[2] / p[3]]);
                        [q[0] * p[3], q[1] * p[3], q[2] * p[3], p[3]]
                    })
                    .collect(),
            }),
            Curve::Line { from, to, domain } => Curve::Line { from: f(*from), to: f(*to), domain: *domain },
            Curve::Polyline { points, params } => Curve::Polyline { points: points.iter().map(|p| f(*p)).collect(), params: params.clone() },
            Curve::Arc { centre, x, y, radius, angle, domain } => Curve::Arc { centre: f(*centre), x: v(*x), y: v(*y), radius: *radius, angle: *angle, domain: *domain },
            Curve::Poly { segments, params } => Curve::Poly { segments: segments.iter().map(|s| s.mapped(f, v)).collect(), params: params.clone() },
        }
    }
}

/// Which segment of `params` holds `t`, the ends included in the first and
/// last.
fn segment(params: &[f64], t: f64) -> usize {
    let last = params.len() - 2;
    params[1..params.len() - 1].iter().position(|p| t < *p).unwrap_or(last)
}

fn inside(params: &[f64], t0: f64, t1: f64) -> Vec<f64> {
    let (lo, hi) = (t0.min(t1), t0.max(t1));
    params.iter().copied().filter(|p| *p > lo && *p < hi).collect()
}

/// A curve object, read by its class. `Ok(None)` for a kind of curve not
/// read here, which the caller names in a warning.
pub(super) fn curve(object: &Object<'_>, depth: usize) -> R<Option<Curve>> {
    let mut r = object.data;
    let class = object.class;
    if depth > 8 {
        return Err("curves nested more than eight deep".into());
    }
    let need_major = |v: (u8, u8)| if v.0 == 1 { Ok(()) } else { Err(format!("a curve of version {}.{}", v.0, v.1)) };
    let curve = match class {
        ON_NURBS_CURVE => {
            need_major(r.version()?)?;
            let dim = r.i32()?;
            let rational = r.i32()? != 0;
            let order = r.i32()?;
            let count = r.i32()?;
            r.skip(8)?;
            r.bbox()?;
            if !(1..=3).contains(&dim) || !(2..=32).contains(&order) || count < order {
                return Err(format!("a NURBS curve of dimension {dim}, order {order} and {count} points"));
            }
            let (dim, order, count) = (dim as usize, order as usize, count as usize);
            let knot_count = r.count(8)?;
            if knot_count != order + count - 2 {
                return Err(format!("a NURBS curve with {knot_count} knots for {count} points of order {order}"));
            }
            let knots: Vec<f64> = (0..knot_count).map(|_| r.f64()).collect::<R<_>>()?;
            let stored = r.count(8)?;
            if stored != count {
                return Err(format!("a NURBS curve that lists {stored} of its {count} points"));
            }
            let size = if rational { dim + 1 } else { dim };
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                let mut v = [0.0f64; 4];
                for slot in v.iter_mut().take(size) {
                    *slot = r.f64()?;
                }
                let w = if rational { v[dim] } else { 1.0 };
                let mut p = [0.0, 0.0, 0.0, w];
                p[..dim].copy_from_slice(&v[..dim]);
                points.push(p);
            }
            let mut full = Vec::with_capacity(knots.len() + 2);
            full.push(knots[0]);
            full.extend_from_slice(&knots);
            full.push(knots[knots.len() - 1]);
            if !sound(order - 1, &full, &points) {
                return Err("a NURBS curve whose numbers do not hold together".into());
            }
            Curve::Nurbs(Nurbs { degree: order - 1, knots: full, points })
        }
        ON_LINE_CURVE => {
            need_major(r.version()?)?;
            let (from, to, domain) = (r.point()?, r.point()?, r.interval()?);
            if !(domain[1] > domain[0]) {
                return Err("a line over an empty interval".into());
            }
            Curve::Line { from, to, domain }
        }
        ON_POLYLINE_CURVE => {
            need_major(r.version()?)?;
            let n = r.count(24)?;
            let points: Vec<P3> = (0..n).map(|_| r.point()).collect::<R<_>>()?;
            let params = r.double_array()?;
            if n < 2 || params.len() != n || !params.windows(2).all(|w| w[1] > w[0]) {
                return Err("a polyline whose parameters do not rise point by point".into());
            }
            Curve::Polyline { points, params }
        }
        ON_ARC_CURVE => {
            need_major(r.version()?)?;
            let plane = r.plane()?;
            let radius = r.f64()?;
            r.skip(72)?;
            let (angle, domain) = (r.interval()?, r.interval()?);
            if !(radius > 0.0) || !(angle[1] > angle[0]) || !(domain[1] > domain[0]) {
                return Err("an arc with no radius or no sweep".into());
            }
            Curve::Arc { centre: plane.origin, x: plane.x, y: plane.y, radius, angle, domain }
        }
        ON_POLY_CURVE => {
            need_major(r.version()?)?;
            let n = r.count(1)?;
            r.skip(8)?;
            r.bbox()?;
            let params = r.double_array()?;
            if n == 0 || params.len() != n + 1 || !params.windows(2).all(|w| w[1] > w[0]) {
                return Err("a polycurve whose parameters do not rise segment by segment".into());
            }
            let mut segments = Vec::with_capacity(n);
            for _ in 0..n {
                let object = r.object()?;
                match curve(&object, depth + 1)? {
                    Some(c) => segments.push(c),
                    None => return Ok(None),
                }
            }
            Curve::Poly { segments, params }
        }
        _ => return Ok(None),
    };
    Ok(Some(curve))
}

// Surfaces.

/// A surface a B-rep face lies on.
#[derive(Clone, Debug)]
pub(super) enum Surface {
    /// Control points homogeneous, u varying fastest.
    Nurbs { degree: [usize; 2], knots: [Vec<f64>; 2], count: [usize; 2], points: Vec<[f64; 4]> },
    /// The plane's parameters run over `domain` and map linearly onto
    /// `extents`, lengths along its axes.
    Plane { plane: Plane, domain: [[f64; 2]; 2], extents: [[f64; 2]; 2] },
    /// `profile` turned about the line through `origin` along `axis`; the
    /// turn's parameter runs over `t` and maps onto `angle`. Unless
    /// `transposed`, u is the turn and v the profile.
    Revolution { profile: Curve, origin: P3, axis: P3, angle: [f64; 2], t: [f64; 2], transposed: bool },
    /// `curves[0](u) + curves[1](v) + base`.
    Sum { curves: [Curve; 2], base: P3 },
}

impl Surface {
    pub fn domain(&self) -> [f64; 4] {
        match self {
            Surface::Nurbs { degree, knots, count, .. } => [knots[0][degree[0]], knots[0][count[0]], knots[1][degree[1]], knots[1][count[1]]],
            Surface::Plane { domain, .. } => [domain[0][0], domain[0][1], domain[1][0], domain[1][1]],
            Surface::Revolution { profile, t, transposed, .. } => {
                let p = profile.domain();
                if *transposed { [p[0], p[1], t[0], t[1]] } else { [t[0], t[1], p[0], p[1]] }
            }
            Surface::Sum { curves, .. } => {
                let (a, b) = (curves[0].domain(), curves[1].domain());
                [a[0], a[1], b[0], b[1]]
            }
        }
    }

    pub fn at(&self, u: f64, v: f64) -> P3 {
        match self {
            Surface::Nurbs { degree, knots, count, points } => {
                let (su, bu) = crate::iges::basis(&knots[0], degree[0], count[0], u);
                let (sv, bv) = crate::iges::basis(&knots[1], degree[1], count[1], v);
                let mut h = [0.0; 4];
                for (j, wv) in bv.iter().enumerate() {
                    for (i, wu) in bu.iter().enumerate() {
                        let p = points[(su - degree[0] + i) + (sv - degree[1] + j) * count[0]];
                        let w = wu * wv;
                        for c in 0..4 {
                            h[c] += w * p[c];
                        }
                    }
                }
                if h[3].abs() < 1e-300 {
                    return [0.0; 3];
                }
                [h[0] / h[3], h[1] / h[3], h[2] / h[3]]
            }
            Surface::Plane { plane, domain, extents } => {
                let map = |s: f64, d: [f64; 2], e: [f64; 2]| e[0] + (s - d[0]) / (d[1] - d[0]) * (e[1] - e[0]);
                let (s, t) = (map(u, domain[0], extents[0]), map(v, domain[1], extents[1]));
                add(plane.origin, add(scale(plane.x, s), scale(plane.y, t)))
            }
            Surface::Revolution { profile, origin, axis, angle, t, transposed } => {
                let (turn, along) = if *transposed { (v, u) } else { (u, v) };
                let a = angle[0] + (turn - t[0]) / (t[1] - t[0]) * (angle[1] - angle[0]);
                rotate(profile.at(along), *origin, *axis, a)
            }
            Surface::Sum { curves, base } => add(add(curves[0].at(u), curves[1].at(v)), *base),
        }
    }

    /// The grid divisions its shape needs at least, each way.
    pub fn spans(&self) -> [usize; 2] {
        match self {
            Surface::Nurbs { degree, knots, count, .. } => {
                let distinct = |k: &[f64], d: usize, n: usize| k[d..=n].windows(2).filter(|w| w[1] > w[0]).count().max(1);
                [distinct(&knots[0], degree[0], count[0]) * degree[0].max(1), distinct(&knots[1], degree[1], count[1]) * degree[1].max(1)]
            }
            Surface::Plane { .. } => [1, 1],
            Surface::Revolution { profile, angle, transposed, .. } => {
                let turn = ((angle[1] - angle[0]).abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize * 2;
                let along = profile.spans();
                if *transposed { [along, turn] } else { [turn, along] }
            }
            Surface::Sum { curves, .. } => [curves[0].spans(), curves[1].spans()],
        }
    }
}

/// `p` turned by `angle` about the line through `origin` along unit `axis`,
/// right-handed (Rodrigues).
fn rotate(p: P3, origin: P3, axis: P3, angle: f64) -> P3 {
    let v = sub(p, origin);
    let (s, c) = angle.sin_cos();
    let turned = add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)));
    add(origin, turned)
}

/// A surface object, read by its class; `Ok(None)` for one not read here.
pub(super) fn surface(object: &Object<'_>) -> R<Option<Surface>> {
    let mut r = object.data;
    let surface = match object.class {
        ON_NURBS_SURFACE => {
            let (major, _) = r.version()?;
            if major != 1 {
                return Err(format!("a NURBS surface of version {major}"));
            }
            let dim = r.i32()?;
            let rational = r.i32()? != 0;
            let order = [r.i32()?, r.i32()?];
            let count = [r.i32()?, r.i32()?];
            r.skip(8)?;
            r.bbox()?;
            if !(1..=3).contains(&dim) || order.iter().any(|o| !(2..=32).contains(o)) || count[0] < order[0] || count[1] < order[1] {
                return Err(format!("a NURBS surface of dimension {dim}, orders {order:?} and points {count:?}"));
            }
            let dim = dim as usize;
            let order = [order[0] as usize, order[1] as usize];
            let count = [count[0] as usize, count[1] as usize];
            let mut knots: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
            for k in 0..2 {
                let n = r.count(8)?;
                if n != order[k] + count[k] - 2 {
                    return Err("a NURBS surface with the wrong number of knots".into());
                }
                let read: Vec<f64> = (0..n).map(|_| r.f64()).collect::<R<_>>()?;
                let mut full = Vec::with_capacity(n + 2);
                full.push(read[0]);
                full.extend_from_slice(&read);
                full.push(read[n - 1]);
                knots[k] = full;
            }
            let stored = r.count(8)?;
            if stored != count[0] * count[1] {
                return Err("a NURBS surface that lists the wrong number of points".into());
            }
            let size = if rational { dim + 1 } else { dim };
            // The file runs v fastest; the evaluator wants u fastest.
            let mut points = vec![[0.0f64; 4]; count[0] * count[1]];
            for i in 0..count[0] {
                for j in 0..count[1] {
                    let mut v = [0.0f64; 4];
                    for slot in v.iter_mut().take(size) {
                        *slot = r.f64()?;
                    }
                    let w = if rational { v[dim] } else { 1.0 };
                    let mut p = [0.0, 0.0, 0.0, w];
                    p[..dim].copy_from_slice(&v[..dim]);
                    points[i + j * count[0]] = p;
                }
            }
            let degree = [order[0] - 1, order[1] - 1];
            let row: Vec<[f64; 4]> = points.iter().take(count[0]).copied().collect();
            let column: Vec<[f64; 4]> = (0..count[1]).map(|j| points[j * count[0]]).collect();
            if !sound(degree[0], &knots[0], &row) || !sound(degree[1], &knots[1], &column) || !points.iter().all(|p| p.iter().all(|c| c.is_finite()) && p[3] > 0.0) {
                return Err("a NURBS surface whose numbers do not hold together".into());
            }
            Surface::Nurbs { degree, knots, count, points }
        }
        ON_PLANE_SURFACE => {
            let (major, minor) = r.version()?;
            if major != 1 {
                return Err(format!("a plane surface of version {major}"));
            }
            let plane = r.plane()?;
            let domain = [r.interval()?, r.interval()?];
            let extents = if minor >= 1 { [r.interval()?, r.interval()?] } else { domain };
            if !(domain[0][1] > domain[0][0] && domain[1][1] > domain[1][0]) {
                return Err("a plane surface over an empty domain".into());
            }
            Surface::Plane { plane, domain, extents }
        }
        ON_REV_SURFACE => {
            let (major, _) = r.version()?;
            if major != 1 && major != 2 {
                return Err(format!("a surface of revolution of version {major}"));
            }
            let (from, to) = (r.point()?, r.point()?);
            let angle = r.interval()?;
            let t = if major >= 2 { r.interval()? } else { angle };
            r.bbox()?;
            let transposed = r.i32()? != 0;
            if !r.bool()? {
                return Err("a surface of revolution with no profile".into());
            }
            let axis = unit(sub(to, from)).ok_or("a surface of revolution about no axis")?;
            if !(angle[1] > angle[0]) || !(t[1] > t[0]) {
                return Err("a surface of revolution of no angle".into());
            }
            let Some(profile) = curve(&r.object()?, 0)? else { return Ok(None) };
            Surface::Revolution { profile, origin: from, axis, angle, t, transposed }
        }
        ON_SUM_SURFACE => {
            let (major, _) = r.version()?;
            if major != 1 {
                return Err(format!("a sum surface of version {major}"));
            }
            let base = r.point()?;
            r.bbox()?;
            let (Some(a), Some(b)) = (curve(&r.object()?, 0)?, curve(&r.object()?, 0)?) else { return Ok(None) };
            Surface::Sum { curves: [a, b], base }
        }
        _ => return Ok(None),
    };
    Ok(Some(surface))
}

// Meshes.

/// A mesh as read: openNURBS's float vertices (or its double ones where it
/// kept them), and the faces split into triangles.
#[derive(Clone, Debug, Default)]
pub(super) struct MeshData {
    pub positions: Vec<P3>,
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates as Rhino has them, v up.
    pub uvs: Vec<[f32; 2]>,
    /// Per vertex, as stored: red, green, blue, and an alpha nothing reads.
    pub colors: Vec<[u8; 4]>,
    pub triangles: Vec<[u32; 3]>,
}

/// `ON_Mesh::Read`: version 1 keeps its vertex arrays as they are, version
/// 3 (Rhino 4 on) as compressed buffers; the faces are four indices each,
/// a triangle repeating its third.
pub(super) fn mesh(object: &Object<'_>) -> R<MeshData> {
    let mut r = object.data;
    let (major, minor) = r.version()?;
    if major != 1 && major != 3 {
        return Err(format!("a mesh of version {major}.{minor}"));
    }
    let vcount = r.count_at_most(1 << 28)?;
    let fcount = r.count_at_most(1 << 28)?;
    // Packed texture and surface domains, the surface scale, and the
    // vertex, normal and texture boxes.
    r.skip(64 + 16 + 24 + 24 + 16)?;
    r.i32()?;
    // Meshing parameters and four curvature statistics, each behind a flag.
    for _ in 0..5 {
        if r.u8()? != 0 {
            r.chunk()?;
        }
    }
    let size = r.i32()?;
    let width = match size {
        1 | 2 | 4 => size as usize,
        _ => return Err(format!("mesh faces of {size}-byte indices")),
    };
    if fcount.saturating_mul(4 * width) > r.remaining() {
        return Err(format!("a mesh claims {fcount} faces, more than the data holds"));
    }
    let raw = r.take(fcount * 4 * width)?;
    let index = |i: usize| -> u32 {
        match width {
            1 => raw[i] as u32,
            2 => u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]) as u32,
            _ => u32::from_le_bytes([raw[4 * i], raw[4 * i + 1], raw[4 * i + 2], raw[4 * i + 3]]),
        }
    };

    let floats = |bytes: &[u8]| -> Vec<f32> { bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() };
    let (mut v, mut n, mut t, mut c) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    if major == 1 {
        // `ReadArray`s: a count, then the items as they are.
        let mut array = |item: usize| -> R<Vec<u8>> {
            let count = r.count(item)?;
            Ok(r.take(count * item)?.to_vec())
        };
        v = array(12)?;
        n = array(12)?;
        t = array(8)?;
        array(16)?;
        c = array(4)?;
    } else if vcount > 0 {
        v = r.compressed(vcount * 12, "mesh vertices")?.unwrap_or_default();
        n = r.compressed(vcount * 12, "mesh normals")?.unwrap_or_default();
        t = r.compressed(vcount * 8, "mesh texture coordinates")?.unwrap_or_default();
        r.compressed(vcount * 16, "mesh curvatures")?;
        c = r.compressed(vcount * 4, "mesh colours")?.unwrap_or_default();
    }
    if v.len() != vcount * 12 {
        return Err(format!("a mesh of {vcount} vertices with {} stored", v.len() / 12));
    }
    let mut out = MeshData::default();
    out.positions = floats(&v).chunks_exact(3).map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]).collect();
    if n.len() == vcount * 12 {
        out.normals = floats(&n).chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
    }
    if t.len() == vcount * 8 {
        out.uvs = floats(&t).chunks_exact(2).map(|p| [p[0], p[1]]).collect();
    }
    if c.len() == vcount * 4 {
        out.colors = c.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    }

    // What follows only refines what is already in hand - double precision
    // vertices from Rhino 6 on - so a problem past here costs nothing.
    let _ = double_vertices(&mut r, major, minor, vcount, &mut out.positions);

    for f in 0..fcount {
        let q = [index(4 * f), index(4 * f + 1), index(4 * f + 2), index(4 * f + 3)];
        if q.iter().any(|&i| i as usize >= vcount) {
            return Err(format!("mesh face {f} names a vertex the mesh does not have"));
        }
        if q[2] == q[3] {
            out.triangles.push([q[0], q[1], q[2]]);
        } else {
            // A quad on its shorter diagonal, as Rhino splits one.
            let p = |i: u32| out.positions[i as usize];
            if length(sub(p(q[0]), p(q[2]))) <= length(sub(p(q[1]), p(q[3]))) {
                out.triangles.push([q[0], q[1], q[2]]);
                out.triangles.push([q[0], q[2], q[3]]);
            } else {
                out.triangles.push([q[0], q[1], q[3]]);
                out.triangles.push([q[1], q[2], q[3]]);
            }
        }
    }
    Ok(out)
}

/// A mesh's fields after its vertex arrays, as far as its double precision
/// vertices, which replace the float ones when they are all there.
fn double_vertices(r: &mut Reader<'_>, major: u8, minor: u8, vcount: usize, positions: &mut Vec<P3>) -> R<()> {
    if minor >= 2 {
        r.i32()?;
    }
    if major == 3 && minor >= 3 {
        r.uuid()?;
        if vcount > 0 {
            r.compressed(vcount * 16, "mesh surface parameters")?;
        }
    }
    if minor < 4 || r.opennurbs < 200_606_010 {
        return Ok(());
    }
    // The texture mapping tag.
    r.chunk()?;
    if minor >= 5 {
        r.skip(3)?;
    }
    if minor >= 6 && r.bool()? {
        // N-gons: the faces are already in hand.
        r.chunk()?;
    }
    if minor >= 7 && r.bool()? {
        let (mut body, major, _) = r.anonymous("a mesh's double precision vertices")?;
        if major != 1 {
            return Ok(());
        }
        let count = body.count(0)?;
        if count == vcount {
            if let Some(bytes) = body.compressed(vcount * 24, "double precision vertices")? {
                let doubles: Vec<P3> = bytes
                    .chunks_exact(24)
                    .map(|c| {
                        let f = |k: usize| f64::from_le_bytes(c[8 * k..8 * k + 8].try_into().unwrap());
                        [f(0), f(1), f(2)]
                    })
                    .collect();
                // Only where they agree with the floats, as openNURBS checks.
                let agree = doubles.iter().zip(positions.iter()).all(|(d, f)| {
                    d.iter().all(|x| x.is_finite()) && (0..3).all(|k| (d[k] - f[k]).abs() <= 1e-5 * (1.0 + d[k].abs()))
                });
                if agree {
                    *positions = doubles;
                }
            }
        }
    }
    Ok(())
}

// B-reps.

/// One trim of a face's loop: the stretch of its curve, in the surface's
/// parameters, from `from` to `to` (either way round).
#[derive(Clone, Debug)]
pub(super) struct Trim {
    pub curve: Arc<Curve>,
    pub from: f64,
    pub to: f64,
}

#[derive(Clone, Debug)]
pub(super) struct Face {
    pub surface: Option<Arc<Surface>>,
    /// Its loops, the outer first, each of trims end to end.
    pub loops: Vec<Vec<Trim>>,
    pub reversed: bool,
    /// The render mesh Rhino kept for it, when the file was saved with them.
    pub mesh: Option<MeshData>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Brep {
    pub faces: Vec<Face>,
    /// Its edges as 3D curve stretches, seams and points left out: what
    /// View > Model edges draws.
    pub edges: Vec<(Arc<Curve>, f64, f64)>,
    pub vertices: Vec<P3>,
    /// Kinds of curve and surface met and not read.
    pub unread: Vec<String>,
}

/// `ON_Brep::Read`, version 3: the 2D curves, 3D curves and surfaces, then
/// the vertices, edges, trims, loops and faces, then (from minor version 1)
/// a render mesh for each face that has one.
pub(super) fn brep(object: &Object<'_>) -> R<Brep> {
    let mut r = object.data;
    let (major, minor) = r.version()?;
    if major != 3 {
        return Err(format!("a B-rep of version {major}.{minor}, older than Rhino 3"));
    }
    let mut out = Brep::default();
    let mut unread = std::collections::BTreeSet::new();
    let c2 = curve_array(&mut r, &mut unread)?;
    let c3 = curve_array(&mut r, &mut unread)?;
    let surfaces: Vec<Option<Arc<Surface>>> = {
        let mut a = array(&mut r, "the surfaces")?;
        let n = a.count(4)?;
        let mut list = Vec::with_capacity(n);
        for _ in 0..n {
            if a.i32()? != 1 {
                list.push(None);
                continue;
            }
            let object = a.object()?;
            match surface(&object) {
                Ok(Some(s)) => list.push(Some(Arc::new(s))),
                Ok(None) => {
                    unread.insert(class_name(object.class));
                    list.push(None);
                }
                Err(e) => {
                    unread.insert(format!("a surface ({e})"));
                    list.push(None);
                }
            }
        }
        list
    };
    // Vertices.
    let mut a = array(&mut r, "the vertices")?;
    for _ in 0..a.count(4)? {
        a.i32()?;
        out.vertices.push(a.point()?);
        a.int_array()?;
        a.f64()?;
    }
    // Edges: their 3D curve and the stretch of it.
    let gated = r.archive >= 3 && r.opennurbs >= 200_206_180;
    let mut a = array(&mut r, "the edges")?;
    let mut edges = Vec::new();
    for _ in 0..a.count(4)? {
        a.i32()?;
        let c3i = a.i32()?;
        let reversed = a.i32()? != 0;
        let domain = a.interval()?;
        a.skip(8)?;
        a.int_array()?;
        a.f64()?;
        if gated {
            a.interval()?;
        }
        edges.push((c3i, reversed, domain));
    }
    // Trims.
    struct T {
        c2i: i32,
        ei: i32,
        kind: i32,
        domain: [f64; 2],
        reversed: bool,
    }
    let mut a = array(&mut r, "the trims")?;
    let mut trims = Vec::new();
    for _ in 0..a.count(4)? {
        a.i32()?;
        let c2i = a.i32()?;
        let domain = a.interval()?;
        let ei = a.i32()?;
        a.skip(8)?;
        a.i32()?;
        let kind = a.i32()?;
        a.i32()?;
        a.i32()?;
        a.skip(16)?;
        let mut reversed = false;
        if gated {
            a.interval()?;
            let flags = a.take(8)?;
            reversed = flags[0] == 1;
            a.skip(24)?;
        } else {
            a.skip(48)?;
        }
        a.skip(16)?;
        trims.push(T { c2i, ei, kind, domain, reversed });
    }
    // Loops.
    let mut a = array(&mut r, "the loops")?;
    let mut loops = Vec::new();
    for _ in 0..a.count(4)? {
        a.i32()?;
        let ti = a.int_array()?;
        a.i32()?;
        a.i32()?;
        loops.push(ti);
    }
    // Faces.
    let mut a = array(&mut r, "the faces")?;
    for _ in 0..a.count(4)? {
        a.i32()?;
        let li = a.int_array()?;
        let si = a.i32()?;
        let reversed = a.i32()? != 0;
        a.i32()?;
        let surface = usize::try_from(si).ok().and_then(|i| surfaces.get(i).cloned().flatten());
        let mut face_loops = Vec::new();
        for l in li {
            let Some(ti) = usize::try_from(l).ok().and_then(|i| loops.get(i)) else { continue };
            let mut list = Vec::new();
            for &t in ti {
                let Some(trim) = usize::try_from(t).ok().and_then(|i| trims.get(i)) else { continue };
                let Some(curve) = usize::try_from(trim.c2i).ok().and_then(|i| c2.get(i).cloned().flatten()) else {
                    list.clear();
                    break;
                };
                let (from, to) = if trim.reversed { (trim.domain[1], trim.domain[0]) } else { (trim.domain[0], trim.domain[1]) };
                list.push(Trim { curve, from, to });
            }
            if !list.is_empty() {
                face_loops.push(list);
            }
        }
        out.faces.push(Face { surface, loops: face_loops, reversed, mesh: None });
    }
    r.bbox()?;
    if minor >= 1 {
        // Each face's render mesh, behind a flag; a mesh that does not read
        // leaves its face to be divided.
        let mut m = array_chunk(&mut r)?;
        for face in &mut out.faces {
            if m.u8()? != 0 {
                let object = m.object()?;
                if object.class == ON_MESH {
                    face.mesh = mesh(&object).ok().filter(|m| !m.triangles.is_empty());
                }
            }
        }
    }

    // The edges worth drawing: not a seam, which a closed surface's face
    // runs along twice, and with a curve to draw.
    let seam: std::collections::HashSet<i32> = trims.iter().filter(|t| t.kind == 3).map(|t| t.ei).collect();
    for (i, (c3i, reversed, domain)) in edges.iter().enumerate() {
        if seam.contains(&(i as i32)) {
            continue;
        }
        if let Some(curve) = usize::try_from(*c3i).ok().and_then(|i| c3.get(i).cloned().flatten()) {
            let (a, b) = if *reversed { (domain[1], domain[0]) } else { (domain[0], domain[1]) };
            out.edges.push((curve, a, b));
        }
    }
    out.unread = unread.into_iter().collect();
    Ok(out)
}

/// An array chunk of a B-rep: anonymous, a version byte of major 1, then
/// what the caller counts.
fn array<'a>(r: &mut Reader<'a>, what: &str) -> R<Reader<'a>> {
    let mut body = array_chunk(r)?;
    let (major, _) = body.version()?;
    if major != 1 {
        return Err(format!("{what} of version {major}"));
    }
    Ok(body)
}

fn array_chunk<'a>(r: &mut Reader<'a>) -> R<Reader<'a>> {
    Ok(r.chunk_of(super::archive::TCODE_ANONYMOUS_CHUNK, "a B-rep array")?.body)
}

/// `ON_CurveArray::Read`: a count, then for each a flag and, when it is
/// 1, the curve.
fn curve_array(r: &mut Reader<'_>, unread: &mut std::collections::BTreeSet<String>) -> R<Vec<Option<Arc<Curve>>>> {
    let mut a = array(r, "the curves")?;
    let n = a.count(4)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if a.i32()? != 1 {
            out.push(None);
            continue;
        }
        let object = a.object()?;
        match curve(&object, 0) {
            Ok(Some(c)) => out.push(Some(Arc::new(c))),
            Ok(None) => {
                unread.insert(class_name(object.class));
                out.push(None);
            }
            Err(e) => {
                unread.insert(format!("a curve ({e})"));
                out.push(None);
            }
        }
    }
    Ok(out)
}

/// What a class is called, for a warning about one not read.
pub(super) fn class_name(class: Uuid) -> String {
    const NAMES: [(Uuid, &str); 4] = [
        (uuid("4ED7D4D9-E947-11d3-BFE5-0010830122F0"), "curve proxies"),
        (uuid("4ED7D4D8-E947-11d3-BFE5-0010830122F0"), "curves on surfaces"),
        (uuid("00C61749-D430-4ecc-83A8-29130A20CF9C"), "offset surfaces"),
        (ON_CLIPPING_PLANE_SURFACE, "clipping planes"),
    ];
    match NAMES.iter().find(|(c, _)| *c == class) {
        Some((_, name)) => name.to_string(),
        None if class == NIL => "empty objects".into(),
        None => {
            let b = class;
            format!(
                "objects of class {:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
            )
        }
    }
}

// Extrusions.

/// `ON_Extrusion`: profiles in a plane, swept straight along a path, with
/// caps, and the meshes Rhino kept for it.
#[derive(Clone, Debug)]
pub(super) struct Extrusion {
    /// The outer profile first, then the holes, each closed (or one open
    /// profile, a swept sheet), in the plane of `x` and `y` at `start`.
    pub profiles: Vec<Curve>,
    pub start: P3,
    pub end: P3,
    pub x: P3,
    pub y: P3,
    pub caps: [bool; 2],
    pub mitered: bool,
    pub meshes: Vec<MeshData>,
}

pub(super) fn extrusion(object: &Object<'_>) -> R<Extrusion> {
    let mut data = object.data;
    let (mut r, major, minor) = data.anonymous("an extrusion")?;
    if major != 1 {
        return Err(format!("an extrusion of version {major}.{minor}"));
    }
    let profile = curve(&r.object()?, 0)?.ok_or("an extrusion whose profile is a kind of curve not read")?;
    let (from, to) = (r.point()?, r.point()?);
    let t = r.interval()?;
    let up = r.point()?;
    let miters = [r.bool()?, r.bool()?];
    r.skip(48)?;
    r.interval()?;
    r.bool()?;
    let count = if minor >= 1 { r.i32()?.max(1) as usize } else { 1 };
    let caps = if minor >= 2 { Some([r.bool()?, r.bool()?]) } else { None };
    let mut meshes = Vec::new();
    if minor >= 3 {
        meshes = mesh_cache(&mut r).unwrap_or_default();
    } else if let Some(cache) = object.userdata.iter().find(|u| u.class == ON_V5_EXTRUSION_DISPLAY_MESH_CACHE) {
        let mut p = cache.payload;
        for _ in 0..2 {
            let Ok(object) = p.object() else { break };
            if object.class == ON_MESH {
                if let Ok(m) = mesh(&object) {
                    meshes.push(m);
                }
            }
        }
    }

    let tangent = unit(sub(to, from)).ok_or("an extrusion along no path")?;
    let y = unit(sub(up, scale(tangent, dot(up, tangent)))).ok_or("an extrusion with no up direction")?;
    let x = cross(y, tangent);
    let profiles = match (count, profile) {
        (1, p) => vec![p],
        (n, Curve::Poly { segments, .. }) if segments.len() == n => segments,
        _ => return Err("an extrusion whose profiles do not match their count".into()),
    };
    let closed = profiles.iter().all(|p| {
        let d = p.domain();
        length(sub(p.at(d[0]), p.at(d[1]))) <= 1e-9 * (1.0 + length(p.at(d[0])))
    });
    // Before minor version 2 a closed extrusion was capped.
    let caps = caps.unwrap_or([closed, closed]);
    let caps = if closed { caps } else { [false, false] };
    let span = sub(to, from);
    Ok(Extrusion {
        profiles,
        start: add(from, scale(span, t[0])),
        end: add(from, scale(span, t[1])),
        x,
        y,
        caps,
        mitered: miters[0] || miters[1],
        meshes,
    })
}

/// Rhino 6's mesh cache: items, each behind a 1, ended by a 0; an item is
/// an anonymous chunk holding the kind of mesh (a UUID) and the mesh.
fn mesh_cache(r: &mut Reader<'_>) -> R<Vec<MeshData>> {
    let (mut cache, major, _) = r.anonymous("a mesh cache")?;
    if major != 1 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    while cache.u8()? == 1 {
        let (mut item, _, _) = cache.anonymous("a mesh cache item")?;
        item.uuid()?;
        let object = item.object()?;
        if object.class == ON_MESH {
            if let Ok(m) = mesh(&object) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

impl Extrusion {
    /// Its faces as surfaces with loops: the sides, swept from each
    /// profile, and the caps, planes trimmed by every profile.
    pub fn faces(&self) -> Vec<(Surface, Vec<Vec<Trim>>, bool)> {
        let path = sub(self.end, self.start);
        let mut out = Vec::new();
        for p in &self.profiles {
            // Lifted into space at the start, then swept: a sum of the
            // profile and the path. The outer profile runs anticlockwise
            // about the path and the holes clockwise, so along the profile
            // then the path faces out of the solid either way.
            let (start, x, y) = (self.start, self.x, self.y);
            let lifted = p.mapped(&|q: P3| add(start, add(scale(x, q[0]), scale(y, q[1]))), &|q: P3| add(scale(x, q[0]), scale(y, q[1])));
            let line = Curve::Line { from: [0.0; 3], to: path, domain: [0.0, 1.0] };
            out.push((Surface::Sum { curves: [lifted, line], base: [0.0; 3] }, Vec::new(), false));
        }
        for (cap, at) in [(0, self.start), (1, self.end)] {
            if !self.caps[cap] {
                continue;
            }
            // The plane's parameters are the profile's own coordinates, so
            // the profiles are its trims as they stand.
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for p in &self.profiles {
                let d = p.domain();
                for i in 0..=64 {
                    let q = p.at(d[0] + (d[1] - d[0]) * i as f64 / 64.0);
                    for k in 0..2 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                }
            }
            if !(hi[0] > lo[0] && hi[1] > lo[1]) {
                continue;
            }
            let pad = [(hi[0] - lo[0]) * 0.01, (hi[1] - lo[1]) * 0.01];
            let domain = [[lo[0] - pad[0], hi[0] + pad[0]], [lo[1] - pad[1], hi[1] + pad[1]]];
            let plane = Plane { origin: at, x: self.x, y: self.y };
            let loops = self
                .profiles
                .iter()
                .map(|p| {
                    let d = p.domain();
                    vec![Trim { curve: Arc::new(p.clone()), from: d[0], to: d[1] }]
                })
                .collect();
            // Its normal is along the path, out of the solid at the end
            // and into it at the start.
            out.push((Surface::Plane { plane, domain, extents: domain }, loops, cap == 0));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rational_quarter_circle_is_round() {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let n = Nurbs { degree: 2, knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0], points: vec![[1.0, 0.0, 0.0, 1.0], [h, h, 0.0, h], [0.0, 1.0, 0.0, 1.0]] };
        for i in 0..=10 {
            let p = n.at(i as f64 / 10.0);
            assert!((length(p) - 1.0).abs() < 1e-12, "{p:?}");
        }
    }

    #[test]
    fn a_polycurve_lays_its_segments_over_its_own_parameters() {
        let a = Curve::Line { from: [0.0; 3], to: [1.0, 0.0, 0.0], domain: [0.0, 1.0] };
        let b = Curve::Line { from: [1.0, 0.0, 0.0], to: [1.0, 2.0, 0.0], domain: [5.0, 7.0] };
        let poly = Curve::Poly { segments: vec![a, b], params: vec![0.0, 1.0, 3.0] };
        assert_eq!(poly.at(0.5), [0.5, 0.0, 0.0]);
        assert_eq!(poly.at(2.0), [1.0, 1.0, 0.0]);
        let pieces = poly.pieces(3.0, 0.0);
        assert_eq!(pieces.len(), 2);
        assert_eq!((pieces[0].0, pieces[0].1), (3.0, 1.0), "backwards, the last segment first");
    }

    #[test]
    fn a_surface_of_revolution_turns_its_profile_about_its_axis() {
        let profile = Curve::Line { from: [2.0, 0.0, 0.0], to: [2.0, 0.0, 3.0], domain: [0.0, 3.0] };
        let s = Surface::Revolution { profile, origin: [0.0; 3], axis: [0.0, 0.0, 1.0], angle: [0.0, std::f64::consts::TAU], t: [0.0, 1.0], transposed: false };
        let p = s.at(0.25, 1.0);
        assert!((p[0]).abs() < 1e-12 && (p[1] - 2.0).abs() < 1e-12 && (p[2] - 1.0).abs() < 1e-12, "{p:?}");
        assert_eq!(s.domain(), [0.0, 1.0, 0.0, 3.0]);
    }
}
