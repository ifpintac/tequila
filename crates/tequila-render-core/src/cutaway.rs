//! Cutaways: volumes that remove what is inside them, and cap what they cut
//! (`docs/02` §3, "Cutaway").
//!
//! "Any surface inside the cutaway solid is not rendered; back-facing
//! surfaces of the cut object are capped and shaded with the assigned cap
//! material. Implemented as a per-ray CSG test against the cutaway
//! primitive(s) at intersection time, plus a cap-surface synthesis pass."
//!
//! ## What the cut object is
//!
//! The result is the object *minus* the union of the cutters. Its surface has
//! two parts: the object's own surface where it lies outside every cutter,
//! and the cutters' surface where it lies inside the object — the cap. So a
//! ray walks forward, dropping object hits that are inside a cutter, and
//! stopping at a cutter's boundary when that point is inside a solid.
//!
//! "Inside a solid" costs nothing extra to answer. At a boundary crossing the
//! ray has already been traced to the next surface beyond it, and when that
//! surface is met from behind, the crossing was inside the object that
//! surface bounds. This is the "back-facing surfaces" of the spec, asked at
//! the right place: the cap is drawn *at the cut*, where it belongs, rather
//! than on the far wall, where it would shade and shadow at the wrong depth.
//!
//! It asks the one thing of the geometry that every closed CAD solid gives:
//! faces wound outward. An open surface — a sheet, a decal plane — has no
//! inside, and is simply cut without a cap.
//!
//! ## The cap's normal
//!
//! Points out of the result solid, which is *into* the cutter. Seen from a
//! camera looking through the removed part, that faces the camera, which is
//! what makes the cap a lit surface rather than a hole.

use crate::intersect::Hit;
use crate::scene::SceneSnapshot;
use tequila_core::ids::MaterialId;
use tequila_core::math::{Ray, Transform, Vec2, Vec3};
use std::sync::Arc;

/// The primitive index a cap reports. Every per-triangle lookup is bounds
/// checked, so a cap falls back to the surface's flat values — no UVs, no
/// interpolated normal — which is what a synthesized surface has.
pub const CAP_PRIMITIVE: u32 = u32::MAX;

/// How many surfaces and boundaries one ray may step over. Beyond it the ray
/// is taken to have gone through, which errs towards seeing into the cut.
const MAX_STEPS: usize = 128;

/// The shape of a cutter, in its own space.
#[derive(Clone, Debug)]
pub enum CutterShape {
    /// Everything in front of the plane `z = 0`, along +Z.
    Plane,
    /// The cube from -1 to 1 on every axis.
    Box,
    /// The unit sphere.
    Sphere,
    /// Radius 1 about the Z axis, from z = -1 to 1.
    Cylinder,
    /// A closed mesh, already in world space.
    Mesh(Arc<CutMesh>),
}

/// One cutting volume.
#[derive(Clone, Debug)]
pub struct Cutter {
    pub shape: CutterShape,
    /// The cutter's own space to world. Unused by a mesh, which is in world
    /// space already.
    pub transform: Transform,
    /// What the cap is shaded with. `None` shades it with the material of the
    /// object that was cut, so a cut part looks solid all the way through.
    pub cap: Option<MaterialId>,
}

impl Cutter {
    pub fn new(shape: CutterShape, transform: Transform) -> Cutter {
        Cutter {
            shape,
            transform,
            cap: None,
        }
    }

    /// Whether `point` is inside the volume.
    pub fn contains(&self, point: Vec3) -> bool {
        if let CutterShape::Mesh(mesh) = &self.shape {
            return mesh.contains(point);
        }
        let p = self.transform.inv_point(point);
        match self.shape {
            CutterShape::Plane => p.z > 0.0,
            CutterShape::Box => p.x.abs() < 1.0 && p.y.abs() < 1.0 && p.z.abs() < 1.0,
            CutterShape::Sphere => p.length_squared() < 1.0,
            CutterShape::Cylinder => p.x * p.x + p.y * p.y < 1.0 && p.z.abs() < 1.0,
            CutterShape::Mesh(_) => unreachable!(),
        }
    }

    /// The first place after `t0` and before `t1` where the ray crosses the
    /// volume's boundary, with the boundary's outward normal there in world
    /// space.
    pub fn next_crossing(&self, ray: &Ray, t0: f32, t1: f32) -> Option<(f32, Vec3)> {
        if let CutterShape::Mesh(mesh) = &self.shape {
            return mesh.nearest(ray.origin, ray.direction, t0, t1);
        }
        // Into the cutter's space. The direction is not renormalized, so a
        // distance along the ray is the same number in both spaces.
        let o = self.transform.inv_point(ray.origin);
        let d = self.transform.inv_vector(ray.direction);
        let (t_in, n_in, t_out, n_out) = convex_interval(&self.shape, o, d)?;
        let (t, normal) = if t_in > t0 && t_in < t1 {
            (t_in, n_in)
        } else if t_out > t0 && t_out < t1 {
            (t_out, n_out)
        } else {
            return None;
        };
        Some((t, self.transform.normal(normal).try_normalize()?))
    }
}

/// Where a ray is inside a convex shape: from `t_in` to `t_out`, with the
/// outward normals at each (in the shape's own space). Infinite ends for a
/// half-space.
fn convex_interval(shape: &CutterShape, o: Vec3, d: Vec3) -> Option<(f32, Vec3, f32, Vec3)> {
    const INF: f32 = f32::INFINITY;
    let z = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
    match shape {
        CutterShape::Plane => {
            if d.z.abs() < 1e-12 {
                return (o.z > 0.0).then_some((-INF, -z, INF, -z));
            }
            let t = -o.z / d.z;
            // Moving towards +z enters the half-space there; the other way
            // leaves it.
            Some(if d.z > 0.0 { (t, -z, INF, -z) } else { (-INF, -z, t, -z) })
        }
        CutterShape::Sphere => {
            let a = d.length_squared();
            let b = o.dot(d);
            let c = o.length_squared() - 1.0;
            let disc = b * b - a * c;
            if disc <= 0.0 || a <= 0.0 {
                return None;
            }
            let root = disc.sqrt();
            let (t_in, t_out) = ((-b - root) / a, (-b + root) / a);
            Some((t_in, o + d * t_in, t_out, o + d * t_out))
        }
        CutterShape::Box => {
            let mut t_in = -INF;
            let mut t_out = INF;
            let (mut n_in, mut n_out) = (Vec3::ZERO, Vec3::ZERO);
            for axis in 0..3 {
                let (oa, da) = (o[axis], d[axis]);
                let mut unit = Vec3::ZERO;
                unit[axis] = 1.0;
                if da.abs() < 1e-12 {
                    if oa.abs() >= 1.0 {
                        return None;
                    }
                    continue;
                }
                let (mut near, mut far) = ((-1.0 - oa) / da, (1.0 - oa) / da);
                let (mut n_near, mut n_far) = (-unit, unit);
                if near > far {
                    std::mem::swap(&mut near, &mut far);
                    std::mem::swap(&mut n_near, &mut n_far);
                }
                if near > t_in {
                    t_in = near;
                    n_in = n_near;
                }
                if far < t_out {
                    t_out = far;
                    n_out = n_far;
                }
            }
            (t_in < t_out).then_some((t_in, n_in, t_out, n_out))
        }
        CutterShape::Cylinder => {
            // The infinite cylinder, then the slab between its end caps.
            let a = d.x * d.x + d.y * d.y;
            let (mut t_in, mut t_out, mut n_in, mut n_out);
            if a < 1e-12 {
                if o.x * o.x + o.y * o.y >= 1.0 {
                    return None;
                }
                (t_in, t_out, n_in, n_out) = (-INF, INF, Vec3::ZERO, Vec3::ZERO);
            } else {
                let b = o.x * d.x + o.y * d.y;
                let c = o.x * o.x + o.y * o.y - 1.0;
                let disc = b * b - a * c;
                if disc <= 0.0 {
                    return None;
                }
                let root = disc.sqrt();
                t_in = (-b - root) / a;
                t_out = (-b + root) / a;
                let side = |t: f32| {
                    let p = o + d * t;
                    Vec3 { x: p.x, y: p.y, z: 0.0 }
                };
                n_in = side(t_in);
                n_out = side(t_out);
            }
            if d.z.abs() < 1e-12 {
                if o.z.abs() >= 1.0 {
                    return None;
                }
            } else {
                let (mut near, mut far) = ((-1.0 - o.z) / d.z, (1.0 - o.z) / d.z);
                let (mut n_near, mut n_far) = (-z, z);
                if near > far {
                    std::mem::swap(&mut near, &mut far);
                    std::mem::swap(&mut n_near, &mut n_far);
                }
                if near > t_in {
                    t_in = near;
                    n_in = n_near;
                }
                if far < t_out {
                    t_out = far;
                    n_out = n_far;
                }
            }
            (t_in < t_out).then_some((t_in, n_in, t_out, n_out))
        }
        CutterShape::Mesh(_) => None,
    }
}

/// A closed mesh used as a cutter, in world space, with a bounding volume
/// hierarchy of its own. Cutters are few and usually coarse, so the
/// hierarchy is a plain median split rather than anything tuned.
#[derive(Debug)]
pub struct CutMesh {
    triangles: Vec<[Vec3; 3]>,
    nodes: Vec<BvhNode>,
}

#[derive(Debug, Clone, Copy)]
struct BvhNode {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle and count. Interior: the right child's index in
    /// `left`, with the left child following this node.
    left: u32,
    count: u32,
}

const LEAF_SIZE: usize = 4;

impl CutMesh {
    pub fn new(mut triangles: Vec<[Vec3; 3]>) -> CutMesh {
        let mut nodes = Vec::with_capacity(triangles.len().max(1) * 2 / LEAF_SIZE + 1);
        let count = triangles.len();
        build(&mut triangles, 0, count, &mut nodes);
        CutMesh { triangles, nodes }
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Inside, by the parity of crossings along a ray: an odd count is
    /// inside. The direction is deliberately not along an axis, so the ray
    /// does not graze the edges an axis-aligned CAD model is full of.
    pub fn contains(&self, point: Vec3) -> bool {
        let direction = Vec3 {
            x: 0.5773,
            y: 0.5774,
            z: 0.5775,
        };
        let mut crossings = 0u32;
        self.visit(point, direction, 0.0, f32::INFINITY, &mut |_, _| {
            crossings += 1;
            f32::INFINITY
        });
        crossings % 2 == 1
    }

    /// Whether the ray meets any triangle in `(t0, t1)`. Stops at the first.
    pub fn hits(&self, origin: Vec3, direction: Vec3, t0: f32, t1: f32) -> bool {
        let mut found = false;
        self.visit(origin, direction, t0, t1, &mut |_, _| {
            found = true;
            // Closing the interval ends the walk: nothing more fits in it.
            t0
        });
        found
    }

    /// The nearest crossing in `(t0, t1)`, with the face normal turned to
    /// point outward. Outward by the winding, confirmed by stepping off the
    /// surface: a mesh wound the wrong way round must still cap on the right
    /// side.
    pub fn nearest(&self, origin: Vec3, direction: Vec3, t0: f32, t1: f32) -> Option<(f32, Vec3)> {
        let mut best: Option<(f32, usize)> = None;
        self.visit(origin, direction, t0, t1, &mut |t, index| {
            if best.is_none_or(|(b, _)| t < b) {
                best = Some((t, index));
            }
            best.map_or(f32::INFINITY, |(b, _)| b)
        });
        let (t, index) = best?;
        let [a, b, c] = self.triangles[index];
        let mut normal = (b - a).cross(c - a).try_normalize()?;
        let point = origin + direction * t;
        let step = (point.abs().max_component().max(1.0)) * 1e-4;
        if self.contains(point + normal * step) {
            normal = -normal;
        }
        Some((t, normal))
    }

    /// Every triangle the ray crosses in `(t0, t1)`, nearest-first where the
    /// hierarchy allows. `hit` returns the new upper bound.
    fn visit(&self, origin: Vec3, direction: Vec3, t0: f32, mut t1: f32, hit: &mut impl FnMut(f32, usize) -> f32) {
        if self.nodes.is_empty() {
            return;
        }
        let inverse = Vec3 {
            x: 1.0 / direction.x,
            y: 1.0 / direction.y,
            z: 1.0 / direction.z,
        };
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = self.nodes[index];
            if !slab(node.min, node.max, origin, inverse, t0, t1) {
                continue;
            }
            if node.count > 0 {
                let first = node.left as usize;
                for i in first..first + node.count as usize {
                    if let Some(t) = triangle(&self.triangles[i], origin, direction) {
                        if t > t0 && t < t1 {
                            t1 = t1.min(hit(t, i));
                        }
                    }
                }
            } else {
                stack.push(node.left as usize);
                stack.push(index + 1);
            }
        }
    }
}

fn build(triangles: &mut [[Vec3; 3]], start: usize, end: usize, nodes: &mut Vec<BvhNode>) -> usize {
    let index = nodes.len();
    let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for tri in &triangles[start..end] {
        for v in tri {
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    nodes.push(BvhNode {
        min,
        max,
        left: start as u32,
        count: (end - start) as u32,
    });
    if end - start <= LEAF_SIZE {
        return index;
    }
    let extent = max - min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let centroid = |t: &[Vec3; 3]| (t[0][axis] + t[1][axis] + t[2][axis]) / 3.0;
    let middle = (start + end) / 2;
    triangles[start..end].select_nth_unstable_by(middle - start, |a, b| centroid(a).total_cmp(&centroid(b)));
    build(triangles, start, middle, nodes);
    let right = build(triangles, middle, end, nodes);
    nodes[index].left = right as u32;
    nodes[index].count = 0;
    index
}

fn slab(min: Vec3, max: Vec3, origin: Vec3, inverse: Vec3, t0: f32, t1: f32) -> bool {
    let a = (min - origin) * inverse;
    let b = (max - origin) * inverse;
    let near = a.min(b).max_component().max(t0);
    let far = a.max(b).min_component().min(t1);
    // NaN from 0 * inf on a face-parallel ray compares false either way; the
    // `!(near > far)` form lets it through rather than culling a real hit.
    !(near > far)
}

/// Möller–Trumbore, both faces.
fn triangle(tri: &[Vec3; 3], origin: Vec3, direction: Vec3) -> Option<f32> {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - tri[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(e2.dot(q) * inv)
}

/// Whether the cutters act on this surface at all. The ground is not
/// geometry of the product and a light's stand-in is not a solid.
fn cuttable(scene: &SceneSnapshot, hit: &Hit) -> bool {
    scene
        .instances
        .get(hit.instance.0 as usize)
        .is_none_or(|i| i.shadow_catcher.is_none() && !i.light_proxy)
}

/// A step along the ray that clears a surface at `t` in single precision.
fn step_past(ray: &Ray, t: f32) -> f32 {
    t + (t.abs().max(ray.origin.abs().max_component()).max(1.0)) * 2e-6
}

/// The nearest surface along `ray` once the scene's cutters have done their
/// work: `raw` is the backend's own closest-hit query.
pub fn intersect(scene: &SceneSnapshot, ray: &Ray, raw: impl Fn(&Ray) -> Option<Hit>) -> Option<Hit> {
    let cutters = scene.cutters();
    if cutters.is_empty() {
        return raw(ray);
    }
    let mut probe = *ray;
    for _ in 0..MAX_STEPS {
        let hit = raw(&probe);
        let limit = hit.as_ref().map_or(probe.t_max, |h| h.t);

        // The nearest boundary before that surface.
        let crossing = cutters
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.next_crossing(ray, probe.t_min, limit).map(|(t, n)| (t, n, i)))
            .min_by(|a, b| a.0.total_cmp(&b.0));

        if let Some((t, outward, index)) = crossing {
            let point = ray.at(t);
            // A boundary inside another cutter is not on the surface of
            // their union, and nothing there is left to cap.
            let exposed = !cutters
                .iter()
                .enumerate()
                .any(|(j, other)| j != index && other.contains(point));
            if let (true, Some(beyond)) = (exposed, &hit) {
                if cuttable(scene, beyond) && beyond.is_backface(ray.direction) {
                    let normal = -outward;
                    return Some(Hit {
                        t,
                        position: point,
                        geometric_normal: normal,
                        shading_normal: normal,
                        uv: Vec2::ZERO,
                        instance: beyond.instance,
                        material: cutters[index].cap.unwrap_or(beyond.material),
                        primitive: CAP_PRIMITIVE,
                    });
                }
            }
            probe.t_min = step_past(ray, t);
            continue;
        }

        let hit = hit?;
        if cuttable(scene, &hit) && cutters.iter().any(|c| c.contains(hit.position)) {
            probe.t_min = step_past(ray, hit.t);
            continue;
        }
        return Some(hit);
    }
    None
}

/// Whether anything is between two points once the cutters have done their
/// work.
pub fn occluded(
    scene: &SceneSnapshot,
    from: Vec3,
    from_normal: Vec3,
    to: Vec3,
    to_normal: Vec3,
    raw: impl Fn(&Ray) -> Option<Hit>,
) -> bool {
    let ray = crate::intersect::shadow_ray(from, from_normal, to, to_normal);
    intersect(scene, &ray, |r| {
        // A shadow ray may pass surfaces that do not cast shadows.
        let mut r = *r;
        loop {
            let hit = raw(&r)?;
            if scene.casts_shadows(hit.instance) {
                return Some(hit);
            }
            r.t_min = step_past(&r, hit.t);
        }
    })
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn ray(origin: Vec3, direction: Vec3) -> Ray {
        Ray::new(origin, direction.normalize())
    }

    /// Each analytic shape agrees with itself: a crossing it reports is on
    /// the boundary between a point it contains and one it does not, and its
    /// normal points out.
    #[test]
    fn crossings_lie_between_inside_and_outside() {
        let transform = Transform::translation(vec3(0.3, -0.2, 0.1));
        for shape in [CutterShape::Plane, CutterShape::Box, CutterShape::Sphere, CutterShape::Cylinder] {
            let cutter = Cutter::new(shape.clone(), transform);
            let mut checked = 0;
            for (o, d) in [
                (vec3(0.0, 0.0, -5.0), vec3(0.05, 0.02, 1.0)),
                (vec3(-5.0, 0.1, 0.0), vec3(1.0, 0.0, 0.03)),
                (vec3(0.2, 5.0, 0.3), vec3(0.0, -1.0, 0.1)),
                (vec3(0.3, -0.2, 0.1), vec3(0.3, 0.4, 0.8)),
            ] {
                let r = ray(o, d);
                let Some((t, normal)) = cutter.next_crossing(&r, 0.0, 100.0) else {
                    continue;
                };
                let p = r.at(t);
                assert!(
                    cutter.contains(p - normal * 1e-3) && !cutter.contains(p + normal * 1e-3),
                    "{shape:?}: the crossing at {p:?} with normal {normal:?} is not on the boundary"
                );
                checked += 1;
            }
            assert!(checked >= 2, "{shape:?} reported almost no crossings");
        }
    }

    fn cube_mesh(scale: f32) -> CutMesh {
        let v = |x: f32, y: f32, z: f32| vec3(x * scale, y * scale, z * scale);
        let corners = [
            v(-1.0, -1.0, -1.0),
            v(1.0, -1.0, -1.0),
            v(1.0, 1.0, -1.0),
            v(-1.0, 1.0, -1.0),
            v(-1.0, -1.0, 1.0),
            v(1.0, -1.0, 1.0),
            v(1.0, 1.0, 1.0),
            v(-1.0, 1.0, 1.0),
        ];
        let faces = [
            [0, 2, 1, 0, 3, 2],
            [4, 5, 6, 4, 6, 7],
            [0, 1, 5, 0, 5, 4],
            [3, 7, 6, 3, 6, 2],
            [0, 4, 7, 0, 7, 3],
            [1, 2, 6, 1, 6, 5],
        ];
        let mut triangles = Vec::new();
        for face in faces {
            for tri in face.chunks(3) {
                triangles.push([corners[tri[0]], corners[tri[1]], corners[tri[2]]]);
            }
        }
        CutMesh::new(triangles)
    }

    /// A mesh cutter behaves as the analytic box it has the shape of.
    #[test]
    fn a_mesh_cutter_matches_the_box_it_is() {
        let mesh = Cutter::new(CutterShape::Mesh(Arc::new(cube_mesh(1.0))), Transform::IDENTITY);
        let analytic = Cutter::new(CutterShape::Box, Transform::IDENTITY);
        for p in [
            vec3(0.0, 0.0, 0.0),
            vec3(0.9, -0.9, 0.5),
            vec3(1.1, 0.0, 0.0),
            vec3(0.2, 3.0, 0.1),
        ] {
            assert_eq!(mesh.contains(p), analytic.contains(p), "they disagree at {p:?}");
        }
        let r = ray(vec3(0.1, 0.2, -4.0), vec3(0.0, 0.0, 1.0));
        let (tm, nm) = mesh.next_crossing(&r, 0.0, 100.0).unwrap();
        let (ta, na) = analytic.next_crossing(&r, 0.0, 100.0).unwrap();
        assert!((tm - ta).abs() < 1e-4 && (nm - na).length() < 1e-4, "{tm} {nm:?} against {ta} {na:?}");
    }

    /// A mesh with its faces wound inward still reports outward normals.
    #[test]
    fn a_mesh_wound_the_wrong_way_still_faces_out() {
        let mut inward = cube_mesh(1.0);
        for tri in &mut inward.triangles {
            tri.swap(1, 2);
        }
        let r = ray(vec3(0.1, 0.2, -4.0), vec3(0.0, 0.0, 1.0));
        let (_, normal) = inward.nearest(r.origin, r.direction, 0.0, 100.0).unwrap();
        assert!(normal.z < -0.99, "the entry face's normal points {normal:?}");
    }
}
