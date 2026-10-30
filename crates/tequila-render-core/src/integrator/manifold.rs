//! Manifold next event estimation [Hanika, Droske and Fascione 2015]: the
//! light a lamp sends through smooth glass onto what is around it, found by
//! solving for the path it takes rather than waiting for a bounce to take
//! it.
//!
//! A lamp is nothing a bounced ray can meet, so no path from the camera
//! ever finds the light it sends through a glass ball onto the floor, and
//! next event estimation's straight shadow ray stops at the glass. But
//! between a point on the floor and a point on the lamp, the way through
//! the glass is fixed by Snell's law at each surface it crosses: at each,
//! the generalized half vector of the two directions lies along the
//! surface's normal. So from the straight line between the two, which
//! crosses the glass about where the path will, a Newton walk moves each
//! crossing along its surface until the law holds at all of them (the
//! "specular manifold"), finding each on the mesh again as it goes. The
//! light that path brings is the lamp's, times each crossing's share,
//! times the generalized geometric term: how much of the floor's view one
//! unit of the lamp covers once the glass has bent it, which falls out of
//! the same derivatives as the walk's [Jakob and Marschner 2012]. The point
//! on the lamp is drawn as next event estimation draws it, so the estimate
//! is the light through the glass, and sharp from the first sample.
//!
//! What it finds, and what it leaves:
//!
//! - Lamps only: area, point, spot, IES and sun. No bounce can meet a lamp,
//!   so this is the only way its light through glass is counted, and it is
//!   weighed against nothing. The environment and glowing parts reach
//!   through glass by bounces, which can meet them; weighing a solved path
//!   against those would need the chance of a bounce landing on it, which
//!   nothing here has. With manifold caustics on, shadow rays stop at glass
//!   that bends light (`ShadowGlass::Unbent`), so light through it is
//!   counted here or by a bounce, never by both.
//! - Glass that bends light ([`crate::scene::Pane::bends`]): mirror-smooth,
//!   solid, on a mesh with vertex normals, whose turning the walk follows.
//!   A thin wall bends nothing and is passed straight through, here as by a
//!   shadow ray.
//! - One path per point on the lamp: the one the straight line's crossings
//!   lead to. Where two or more paths through the glass join the same two
//!   points - past a focus, in a ball's brightest spot - the others are
//!   missed, and the caustic there is darker than the true one; finding them
//!   too [Zeltner et al. 2020] costs many walks a sample. Cycles, whose
//!   method this is, misses them the same way.
//! - Refraction only: a path reflected inside the glass on its way, a
//!   mirror's caustic, light through frosted glass, are not found here.
//!
//! Ported from Cycles' `kernel/integrator/mnee.h` (Apache-2.0, Blender
//! Foundation; see THIRD_PARTY_LICENSES.md). Changed on the way: which side
//! of a crossing the receiver is on is read from the media the seed path
//! passes, as the path tracer reads it, rather than from the triangle's
//! winding, which a CAD part can have inside out; the tolerances are a
//! fraction of the scene's size rather than metres; the geometric term is
//! left unclamped, so the estimate stays unbiased; a sun's direction is
//! moved across the path's last leg rather than in angles about the
//! surface's normal, which are singular along it; and radiance is squeezed
//! by the square of the index at each crossing, as Tequila's refraction has
//! it and Cycles' does not.

use super::*;
use crate::light::{Light, LightKind};
use crate::scene::SceneSnapshot;
use tequila_core::ids::MaterialId;
use tequila_core::math::vec2;

/// Steps of the walk before it is given up.
const MAX_ITERATIONS: u32 = 64;

/// Surfaces a seed or a projection ray passes before it gives up.
const MAX_INTERSECTIONS: u32 = 10;

/// How far from Snell's law a path may be and count as found: the largest
/// part of any crossing's half vector along its surface.
const SOLVER_THRESHOLD: f32 = 1e-3;

/// The smallest share of a step the walk will take before giving up.
const MINIMUM_STEP_SIZE: f32 = 1e-4;

/// The most crossings a path may have: in and out of a bottle's wall, and
/// in and out of what it holds, with room to spare.
pub const MAX_CROSSINGS: usize = 6;

/// Below this a 2x2 block of the walk's derivatives is taken as singular.
const MIN_DETERMINANT: f32 = 1e-4;

/// How far past a tentative crossing its surface is looked for.
const PROJECTION_DISTANCE_MULTIPLIER: f32 = 2.0;

/// A 2x2 matrix, row by row: a block of the walk's derivatives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Mat2 {
    xx: f32,
    xy: f32,
    yx: f32,
    yy: f32,
}

impl Mat2 {
    const ZERO: Mat2 = Mat2 { xx: 0.0, xy: 0.0, yx: 0.0, yy: 0.0 };
    const IDENTITY: Mat2 = Mat2 { xx: 1.0, xy: 0.0, yx: 0.0, yy: 1.0 };

    /// The four dot products of `du` and `dv` with `s` and `t`.
    fn of(du: Vec3, dv: Vec3, s: Vec3, t: Vec3) -> Mat2 {
        Mat2 { xx: du.dot(s), xy: dv.dot(s), yx: du.dot(t), yy: dv.dot(t) }
    }

    fn times(self, o: Mat2) -> Mat2 {
        Mat2 {
            xx: self.xx * o.xx + self.xy * o.yx,
            xy: self.xx * o.xy + self.xy * o.yy,
            yx: self.yx * o.xx + self.yy * o.yx,
            yy: self.yx * o.xy + self.yy * o.yy,
        }
    }

    fn apply(self, v: Vec2) -> Vec2 {
        vec2(self.xx * v.x + self.xy * v.y, self.yx * v.x + self.yy * v.y)
    }

    fn minus(self, o: Mat2) -> Mat2 {
        Mat2 { xx: self.xx - o.xx, xy: self.xy - o.xy, yx: self.yx - o.yx, yy: self.yy - o.yy }
    }

    fn negated(self) -> Mat2 {
        Mat2 { xx: -self.xx, xy: -self.xy, yx: -self.yx, yy: -self.yy }
    }

    fn determinant(self) -> f32 {
        self.xx * self.yy - self.xy * self.yx
    }

    /// The inverse, or `None` where the block is too near singular for one.
    fn inverse(self) -> Option<Mat2> {
        let det = self.determinant();
        // Negated, so that a NaN is refused too.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(det.abs() >= MIN_DETERMINANT) {
            return None;
        }
        Some(Mat2 { xx: self.yy / det, xy: -self.xy / det, yx: -self.yx / det, yy: self.xx / det })
    }
}

/// A surface where a path crosses it, as the walk needs it: where it is,
/// two directions along it, its normals, and how the shading normal turns
/// along each of the two.
#[derive(Clone, Copy, Debug)]
struct Surface {
    p: Vec3,
    dp_du: Vec3,
    dp_dv: Vec3,
    n: Vec3,
    /// The triangle's own normal, by its winding.
    ng: Vec3,
    dn_du: Vec3,
    dn_dv: Vec3,
}

/// One crossing of glass on the path from the receiver to the lamp.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    at: Surface,
    instance: InstanceId,
    material: MaterialId,
    /// The index on the lamp's side over the index on the receiver's.
    eta: f32,
    /// Which way `at.ng` faced the receiver's side when the path was
    /// seeded: 1 towards it, -1 away. The walk keeps each crossing on its
    /// own object, and so on the same side of it.
    side: f32,
    /// Its half vector's part along the surface, which the walk drives to
    /// zero, and that part's derivatives in the crossings before (`a`) and
    /// after it (`c`) and in its own (`b`), each in their two directions.
    constraint: Vec2,
    a: Mat2,
    b: Mat2,
    c: Mat2,
}

impl Crossing {
    const EMPTY: Crossing = Crossing {
        at: Surface {
            p: Vec3::ZERO,
            dp_du: Vec3::ZERO,
            dp_dv: Vec3::ZERO,
            n: Vec3::ZERO,
            ng: Vec3::ZERO,
            dn_du: Vec3::ZERO,
            dn_dv: Vec3::ZERO,
        },
        instance: InstanceId(0),
        material: MaterialId(0),
        eta: 1.0,
        side: 1.0,
        constraint: Vec2::ZERO,
        a: Mat2::ZERO,
        b: Mat2::IDENTITY,
        c: Mat2::ZERO,
    };

    /// The index ratio here for a path arriving from `wi`: as seeded, or
    /// its inverse should the receiver's side have come round to the other.
    fn eta_towards(&self, wi: Vec3) -> f32 {
        if wi.dot(self.at.ng) * self.side >= 0.0 {
            self.eta
        } else {
            1.0 / self.eta
        }
    }
}

/// Where the path ends at the lamp.
#[derive(Clone, Copy, Debug)]
enum End {
    /// A point the light leaves from: on an area light, a point light's.
    Point(Vec3),
    /// The way back to a sun.
    Direction(Vec3),
}

impl End {
    /// Towards the lamp from `from`, not normalized for a point.
    fn towards(self, from: Vec3) -> Vec3 {
        match self {
            End::Point(y) => y - from,
            End::Direction(d) => d,
        }
    }
}

/// The surface of a triangle mesh at `hit`, with the derivatives the walk
/// takes along it. `None` off a mesh with vertex normals: a faceted surface
/// has no turning to follow, and a solved path on one is a facet's.
///
/// As Cycles sets a vertex up (`mnee_setup_manifold_vertex`): the
/// interpolated normal's derivatives across the triangle, carried into a
/// frame about the triangle's normal that every point of the surface shares
/// the rule for.
fn surface(scene: &SceneSnapshot, hit: &Hit) -> Option<Surface> {
    let instance = scene.instances.get(hit.instance.0 as usize)?;
    let mesh = scene.meshes.get(instance.mesh.index())?;
    if mesh.normals.len() != mesh.positions.len() || hit.primitive as usize >= mesh.triangle_count() {
        return None;
    }
    let [i0, i1, i2] = mesh.triangle(hit.primitive as usize).map(|i| i as usize);
    let transform = &instance.transform;
    let (v0, v1, v2) = (transform.point(mesh.positions[i0]), transform.point(mesh.positions[i1]), transform.point(mesh.positions[i2]));
    let (n0, n1, n2) = (
        transform.normal(mesh.normals[i0]).try_normalize()?,
        transform.normal(mesh.normals[i1]).try_normalize()?,
        transform.normal(mesh.normals[i2]).try_normalize()?,
    );

    // Where on the triangle, by its corners' weights.
    let (e1, e2, d) = (v1 - v0, v2 - v0, hit.position - v0);
    let (d11, d12, d22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let (d31, d32) = (d.dot(e1), d.dot(e2));
    let det = d11 * d22 - d12 * d12;
    if !(det.abs() > 1e-30) {
        return None;
    }
    let u = (d22 * d31 - d12 * d32) / det;
    let v = (d11 * d32 - d12 * d31) / det;
    let p = v0 + e1 * u + e2 * v;
    let ng = e1.cross(e2).try_normalize()?;

    // The shading normal, and its derivatives across the triangle: of
    // f / |f|, f / |f| times f's less f's own direction.
    let interpolated = n0 * (1.0 - u - v) + n1 * u + n2 * v;
    let length = interpolated.length();
    if !(length > 1e-12) {
        return None;
    }
    let n = interpolated / length;
    let mut dn_du = (n1 - n0) / length;
    let mut dn_dv = (n2 - n0) / length;
    dn_du -= n * n.dot(dn_du);
    dn_dv -= n * n.dot(dn_dv);

    // The triangle's two edges made orthonormal, and the normal's
    // derivatives carried along with them, so they are per unit of distance
    // along each.
    let inv_du = 1.0 / e1.length();
    let dp_du = e1 * inv_du;
    dn_du *= inv_du;
    let along = dp_du.dot(e2);
    let dp_dv = e2 - dp_du * along;
    dn_dv -= dn_du * along;
    let inv_dv = 1.0 / dp_dv.length();
    if !inv_dv.is_finite() || !inv_du.is_finite() {
        return None;
    }
    let dp_dv = dp_dv * inv_dv;
    dn_dv *= inv_dv;

    // Into a frame decided by the triangle's normal alone, so that the
    // directions a step is taken in turn smoothly from one triangle to the
    // next; the normal's derivatives turned with it.
    let frame_u = Onb::from_normal(ng).t;
    let frame_v = ng.cross(frame_u);
    let cos = dp_du.dot(frame_u);
    let sin = -dp_dv.dot(frame_u);
    Some(Surface {
        p,
        dp_du: frame_u,
        dp_dv: frame_v,
        n,
        ng,
        dn_du: dn_du * cos - dn_dv * sin,
        dn_dv: dn_du * sin + dn_dv * cos,
    })
}

/// Each crossing's constraint and its derivatives, for the path from
/// `receiver` through `crossings` to `end`: Cycles'
/// `mnee_compute_constraint_derivatives`. False where two points of the
/// path have come within `short` of each other.
fn constrain(crossings: &mut [Crossing], receiver: Vec3, end: End, short: f32) -> bool {
    let k = crossings.len();
    let fixed_direction = matches!(end, End::Direction(_));
    for i in 0..k {
        let v = crossings[i];
        let at = v.at;

        // Towards the receiver's side, and towards the lamp's.
        let previous = if i == 0 { receiver } else { crossings[i - 1].at.p };
        let mut wi = previous - at.p;
        let mut ili = wi.length();
        if ili < short {
            return false;
        }
        ili = 1.0 / ili;
        wi *= ili;
        let mut wo = if i == k - 1 { end.towards(at.p) } else { crossings[i + 1].at.p - at.p };
        let mut ilo = wo.length();
        if ilo < short {
            return false;
        }
        ilo = 1.0 / ilo;
        wo *= ilo;

        let eta = v.eta_towards(wi);
        let mut h = -(wi + wo * eta);
        let ilh = 1.0 / h.length();
        h *= ilh;
        ilo *= eta * ilh;
        ili *= ilh;

        // The shading frame the constraint is measured in.
        let dp_du_dot_n = at.dp_du.dot(at.n);
        let mut s = at.dp_du - at.n * dp_du_dot_n;
        let inv_len_s = 1.0 / s.length();
        s *= inv_len_s;
        let t = at.n.cross(s);

        // In the crossing before.
        let a = if i > 0 {
            let before = crossings[i - 1].at;
            let mut du = (before.dp_du - wi * wi.dot(before.dp_du)) * ili;
            let mut dv = (before.dp_dv - wi * wi.dot(before.dp_dv)) * ili;
            du -= h * du.dot(h);
            dv -= h * dv.dot(h);
            Mat2::of(-du, -dv, s, t)
        } else {
            Mat2::ZERO
        };

        // In its own.
        let (mut du, mut dv) = if i == k - 1 && fixed_direction {
            ((-at.dp_du + wi * wi.dot(at.dp_du)) * ili, (-at.dp_dv + wi * wi.dot(at.dp_dv)) * ili)
        } else {
            (
                -at.dp_du * (ili + ilo) + wi * (wi.dot(at.dp_du) * ili) + wo * (wo.dot(at.dp_du) * ilo),
                -at.dp_dv * (ili + ilo) + wi * (wi.dot(at.dp_dv) * ili) + wo * (wo.dot(at.dp_dv) * ilo),
            )
        };
        du -= h * du.dot(h);
        dv -= h * dv.dot(h);
        let (du, dv) = (-du, -dv);
        let mut ds_du = -(at.n * at.dp_du.dot(at.dn_du) + at.dn_du * dp_du_dot_n) * inv_len_s;
        let mut ds_dv = -(at.n * at.dp_du.dot(at.dn_dv) + at.dn_dv * dp_du_dot_n) * inv_len_s;
        ds_du -= s * s.dot(ds_du);
        ds_dv -= s * s.dot(ds_dv);
        let dt_du = at.dn_du.cross(s) + at.n.cross(ds_du);
        let dt_dv = at.dn_dv.cross(s) + at.n.cross(ds_dv);
        let b = Mat2 {
            xx: du.dot(s) + h.dot(ds_du),
            xy: dv.dot(s) + h.dot(ds_dv),
            yx: du.dot(t) + h.dot(dt_du),
            yy: dv.dot(t) + h.dot(dt_dv),
        };

        // In the crossing after.
        let c = if i < k - 1 {
            let after = crossings[i + 1].at;
            let mut du = (after.dp_du - wo * wo.dot(after.dp_du)) * ilo;
            let mut dv = (after.dp_dv - wo * wo.dot(after.dp_dv)) * ilo;
            du -= h * du.dot(h);
            dv -= h * dv.dot(h);
            Mat2::of(-du, -dv, s, t)
        } else {
            Mat2::ZERO
        };

        let crossing = &mut crossings[i];
        crossing.constraint = vec2(s.dot(h), t.dot(h));
        crossing.a = a;
        crossing.b = b;
        crossing.c = c;
    }
    true
}

/// The step that would bring every constraint to zero were the surfaces
/// their tangent planes: the block tridiagonal system of their derivatives
/// solved for it (Cycles' `mnee_solve_matrix_h_to_x`). False where it is
/// singular.
fn step(crossings: &[Crossing], dx: &mut [Vec2; MAX_CROSSINGS]) -> bool {
    let k = crossings.len();
    let mut inverses = [Mat2::ZERO; MAX_CROSSINGS];
    let mut carried = [Vec2::ZERO; MAX_CROSSINGS];
    let Some(first) = crossings[0].b.inverse() else { return false };
    inverses[0] = first;
    carried[0] = crossings[0].constraint;
    for j in 1..k {
        let a = crossings[j].a.times(inverses[j - 1]);
        let Some(inverse) = crossings[j].b.minus(a.times(crossings[j - 1].c)).inverse() else {
            return false;
        };
        inverses[j] = inverse;
        carried[j] = crossings[j].constraint - a.apply(carried[j - 1]);
    }
    dx[k - 1] = inverses[k - 1].apply(carried[k - 1]);
    for j in (0..k - 1).rev() {
        dx[j] = inverses[j].apply(carried[j] - crossings[j].c.apply(dx[j + 1]));
    }
    true
}

/// The surface of `instance` the way from `from` to `towards` meets, going
/// past anything else in the way: a tentative crossing, taken from its
/// surface's tangent plane back onto the surface. A path that is blocked is
/// still a path to solve for; whether it is blocked is asked once it is
/// found.
fn project(intersector: &dyn Intersector, from: Vec3, from_normal: Vec3, towards: Vec3, instance: InstanceId) -> Option<Hit> {
    let leaving = if (towards - from).dot(from_normal) >= 0.0 { from_normal } else { -from_normal };
    // Aimed from where the ray starts, off the surface, not from the point
    // it was offset from: near the origin the offset is a fixed few
    // hundredths of a millimetre, and a ray parallel to the line aimed along
    // lands that far beside the point every time - which the walk then
    // spends every step putting back, short of the answer for ever.
    let origin = offset_ray_origin(from, leaving);
    let offset = towards - origin;
    let distance = offset.length();
    if !(distance > 0.0) {
        return None;
    }
    let direction = offset / distance;
    let mut ray = Ray {
        origin,
        direction,
        t_min: 0.0,
        t_max: distance * PROJECTION_DISTANCE_MULTIPLIER,
        time: 0.0,
    };
    for _ in 0..MAX_INTERSECTIONS {
        let hit = intersector.intersect(&ray)?;
        if hit.instance == instance {
            return Some(hit);
        }
        let (geometric, _) = hit.facing_forward(direction);
        ray = Ray { origin: offset_ray_origin(hit.position, -geometric), t_max: ray.t_max - hit.t, ..ray };
        if ray.t_max <= 0.0 {
            return None;
        }
    }
    None
}

/// Walk the crossings along their surfaces until Snell's law holds at every
/// one of them, for the path from `receiver` (whose geometric normal,
/// facing the lamp's side, is `normal`) to `end`: Cycles'
/// `mnee_newton_solver`. Gently at first, as Cycles does: a seed far from
/// the answer sends a full Newton step further than the surfaces' local
/// shape can be trusted, so it starts at a tenth and doubles while the
/// steps land.
fn walk(intersector: &dyn Intersector, receiver: Vec3, normal: Vec3, end: End, crossings: &mut [Crossing], unit: f32) -> bool {
    let scene = intersector.scene();
    let k = crossings.len();
    let short = unit * 1e-6;
    let mut dx = [Vec2::ZERO; MAX_CROSSINGS];
    let mut tentative = [Crossing::EMPTY; MAX_CROSSINGS];
    let mut beta = 0.1f32;
    let mut resolve = true;
    for _ in 0..MAX_ITERATIONS {
        if resolve {
            if !constrain(crossings, receiver, end, short) {
                return false;
            }
            let worst = crossings.iter().map(|c| c.constraint.length()).fold(0.0f32, f32::max);
            if worst < SOLVER_THRESHOLD {
                return true;
            }
            if !step(crossings, &mut dx) {
                return false;
            }
        }

        // Each crossing moved along its tangent plane, and found on its
        // surface again from the one before.
        let mut reduce = false;
        let (mut moved, mut asked) = (0.0f32, 0.0f32);
        for i in 0..k {
            let current = crossings[i];
            let along = current.at.dp_du * dx[i].x + current.at.dp_dv * dx[i].y;
            asked = asked.max(along.length() * beta);
            let mut p = current.at.p - along * beta;
            // A first step can land below the receiver; the other way, then.
            if i == 0 && (p - receiver).dot(normal) <= 0.0 {
                p = current.at.p + along * beta;
            }
            let (from, from_normal) = if i == 0 { (receiver, normal) } else { (crossings[i - 1].at.p, crossings[i - 1].at.ng) };
            let Some(at) = project(intersector, from, from_normal, p, current.instance).and_then(|hit| surface(scene, &hit)) else {
                reduce = true;
                break;
            };
            moved = moved.max((at.p - current.at.p).length());
            tentative[i] = Crossing { at, ..current };
        }
        // None of them moving as far as asked: stuck against the edge of the
        // mesh, most likely. Measured against the step, which gets as small
        // as the answer is near; and over all of them, since one alone may
        // be where it belongs already, as the first crossing of a slab is
        // under the sun.
        if !reduce && moved < asked * 1e-3 {
            return false;
        }

        // Still through each surface, not turned back off it.
        if !reduce {
            for i in 0..k {
                let at = tentative[i].at;
                let wi = if i == 0 { receiver } else { tentative[i - 1].at.p } - at.p;
                let wo = if i == k - 1 { end.towards(at.p) } else { tentative[i + 1].at.p - at.p };
                if at.n.dot(wi) * at.n.dot(wo) >= 0.0 {
                    reduce = true;
                    break;
                }
            }
        }

        if reduce {
            resolve = false;
            beta *= 0.5;
            if beta < MINIMUM_STEP_SIZE {
                return false;
            }
            continue;
        }
        crossings.copy_from_slice(&tentative[..k]);
        resolve = true;
        beta = (2.0 * beta).min(1.0);
    }
    false
}

/// How much of the first crossing's tangent plane one unit of the lamp
/// covers, seen through the path: |dx1/dy|, from the constraints'
/// derivatives at the solution (Cycles' `mnee_compute_transfer_matrix`).
/// The lamp's unit is area in the plane `plane` spans, for a point on it;
/// for a sun, solid angle about its direction, which `plane` is across.
fn spread(crossings: &[Crossing], receiver: Vec3, end: End, plane: (Vec3, Vec3)) -> Option<f32> {
    let k = crossings.len();
    let mut forward = [Mat2::ZERO; MAX_CROSSINGS];
    let mut inverse = crossings[0].b.inverse()?;
    for j in 1..k {
        forward[j - 1] = inverse.times(crossings[j - 1].c);
        inverse = crossings[j].b.minus(crossings[j].a.times(forward[j - 1])).inverse()?;
    }

    // The last crossing's constraint, as the lamp's end moves.
    let last = &crossings[k - 1];
    let s = (last.at.dp_du - last.at.n * last.at.dp_du.dot(last.at.n)).try_normalize()?;
    let t = last.at.n.cross(s);
    let wi = (if k == 1 { receiver } else { crossings[k - 2].at.p } - last.at.p).try_normalize()?;
    let eta = last.eta_towards(wi);
    let (u, v) = plane;
    let moved = match end {
        End::Direction(wo) => {
            // The direction moved by a small angle across itself, along `u`
            // and along `v`: d(wo) is that direction, and the half vector's
            // unnormalized change is -eta times it.
            let h = -(wi + wo * eta);
            let ilh = 1.0 / h.length();
            let h = h * ilh;
            let dh = |e: Vec3| -(e - h * h.dot(e)) * (eta * ilh);
            Mat2::of(dh(u), dh(v), s, t)
        }
        End::Point(y) => {
            let mut wo = y - last.at.p;
            let mut ilo = 1.0 / wo.length();
            wo *= ilo;
            let h = -(wi + wo * eta);
            let ilh = 1.0 / h.length();
            let h = h * ilh;
            ilo *= eta * ilh;
            let mut du = (u - wo * wo.dot(u)) * ilo;
            let mut dv = (v - wo * wo.dot(v)) * ilo;
            du -= h * du.dot(h);
            dv -= h * dv.dot(h);
            Mat2::of(-du, -dv, s, t)
        }
    };

    // Back along the path to the first crossing.
    let mut transfer = inverse.times(moved).negated();
    for j in (0..k - 1).rev() {
        transfer = forward[j].times(transfer).negated();
    }
    let spread = transfer.determinant().abs();
    spread.is_finite().then_some(spread)
}

/// One lamp, drawn as next event estimation draws it.
struct Lamp<'a> {
    light: &'a Light,
    end: End,
    /// The plane an area light's point moves in. Any other's is across the
    /// path's last leg, known only once the path is.
    plane: Option<(Vec3, Vec3)>,
    /// The density the lamp and its point were drawn with: the selection's,
    /// times an area light's per unit of its area or a sun's per unit of
    /// solid angle about its direction.
    density: f32,
    /// A sun's light, which arrives along every path alike: its
    /// illuminance where it is a delta, its radiance where it has a disc.
    sun: Spectrum,
}

/// Draw a lamp and a point on it for a surface at `at` facing `normal`, on
/// the dimensions next event estimation draws them on: the same lamp and
/// point its own sample took, which is no matter - the two count different
/// paths - and keeps a caustic as well stratified as a shadow is.
fn lamp<'a>(scene: &'a SceneSnapshot, at: Vec3, normal: Vec3, receiver: InstanceId, sampler: &mut Sampler) -> Option<Lamp<'a>> {
    let (id, chosen) = scene.lights.select_at(at, normal, sampler.light_select())?;
    let light = scene.lights.get(id)?;
    let u = sampler.light_sample();
    // A lamp that casts no shadow passes glass already, unbent, and one kept
    // off this part lights it through nothing.
    if !light.enabled || !light.visibility.shadow || !light.visibility.lights_anything() || !light.links.lights(receiver.0) {
        return None;
    }
    if !(chosen > 0.0) {
        return None;
    }
    let point = |position: Vec3| Some(Lamp { light, end: End::Point(position), plane: None, density: chosen, sun: Spectrum::ZERO });
    match &light.kind {
        LightKind::Area(area) => {
            let sample = light.sample(at, u)?;
            let normal = area.normal();
            let cos = -sample.direction.dot(normal);
            let cos = if area.two_sided { cos.abs() } else { cos };
            if !(sample.pdf > 0.0 && cos > 1e-6 && sample.distance.is_finite()) {
                return None;
            }
            // Its density per unit of solid angle from here, as one per unit
            // of the light's area.
            let per_area = sample.pdf * cos / (sample.distance * sample.distance);
            let frame = Onb::from_normal(normal);
            Some(Lamp {
                light,
                end: End::Point(at + sample.direction * sample.distance),
                plane: Some((frame.t, frame.b)),
                density: chosen * per_area,
                sun: Spectrum::ZERO,
            })
        }
        // A point light with a radius is a point drawn on its sphere, as
        // its own sample draws one; a spot's and a profile's light leaves
        // from where they are, whichever way the path turns out to leave.
        LightKind::Point(p) if p.radius > 0.0 => {
            let sample = light.sample(at, u)?;
            point(at + sample.direction * sample.distance)
        }
        LightKind::Point(p) => point(p.position),
        LightKind::Spot(spot) => point(spot.position),
        LightKind::Ies(ies) => point(ies.position),
        LightKind::Distant(sun) => {
            let sample = light.sample(at, u)?;
            let (light_in, density) = if sample.is_delta { (sun.irradiance, chosen) } else { (sample.radiance, chosen * sample.pdf) };
            if !(density > 0.0) {
                return None;
            }
            Some(Lamp { light, end: End::Direction(sample.direction), plane: None, density, sun: light_in })
        }
    }
}

/// What a lamp's point sends along `towards` (from it to the path's last
/// crossing): an area light's radiance that way, a point light's intensity.
fn emitted(light: &Light, towards: Vec3) -> Spectrum {
    match &light.kind {
        LightKind::Area(area) => {
            let cos = towards.dot(area.normal());
            let cos = if area.two_sided { cos.abs() } else { cos };
            if cos <= 1e-6 {
                Spectrum::ZERO
            } else {
                area.radiance * area.spread_attenuation(cos)
            }
        }
        LightKind::Point(point) => point.intensity,
        LightKind::Spot(spot) => spot.intensity * spot.falloff_at(towards.dot(spot.direction)),
        LightKind::Ies(ies) => ies.intensity * ies.candela(towards),
        LightKind::Distant(_) => Spectrum::ZERO,
    }
}

/// `normal` turned to face `direction`.
fn facing(normal: Vec3, direction: Vec3) -> Vec3 {
    if normal.dot(direction) >= 0.0 {
        normal
    } else {
        -normal
    }
}

/// A lamp's light through glass onto one point, by one manifold sample.
pub(super) struct Caustic {
    /// From the point towards the path's first crossing.
    pub direction: Vec3,
    /// What arrives along it over the density it was drawn with, as next
    /// event estimation gives a lamp's: times the surface's cosine and
    /// response, the light the surface sends on.
    pub light: Spectrum,
    pub crossings: usize,
    pub lamp: crate::light::Visibility,
    pub layer: crate::lpe::LightGroup,
}

/// Solve for the path from a lamp through the glass in the way to the
/// point `at`, whose geometric normal facing the side the light is to come
/// from is `normal`, inside `media`, and return the light it brings.
/// `None` where there is no glass that bends light in the way, or no path
/// through it was found, or what it brings is blocked.
#[allow(clippy::too_many_arguments)]
pub(super) fn solve(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    at: &Hit,
    normal: Vec3,
    media: &MediumStack,
    bounce: u32,
    transmission_bounces: u32,
    sampler: &mut Sampler,
) -> Option<Caustic> {
    let scene = intersector.scene();
    if !config.shadows || scene.lights.is_empty() || !scene.has_panes() {
        return None;
    }
    let lamp = lamp(scene, at.position, normal, at.instance, sampler)?;
    let light = lamp.light;
    let unit = scene.bounds().bounding_radius().max(1e-3);

    // ---- The seed: the glass the straight line to the lamp crosses ------
    //
    // Everything else on the way is passed - a path around it may still
    // reach the lamp - and with it what changes the media the path is in,
    // so that each crossing's two indices are the ones the path tracer
    // would find there: water in a glass bends light by 1.33 to 1.5, not by
    // 1.33.
    let receiver = at.position;
    let (direction, reach) = match lamp.end {
        End::Point(y) => {
            let offset = y - receiver;
            let distance = offset.length();
            if !(distance > 0.0) {
                return None;
            }
            (offset / distance, distance * (1.0 - 1e-4))
        }
        End::Direction(d) => (d, f32::INFINITY),
    };
    if direction.dot(normal) <= 0.0 {
        return None;
    }
    let mut seeded = media.clone();
    // The medium each leg of the path runs through, where it absorbs: the
    // receiver's, then whatever each crossing leads into.
    let mut absorbing: [Option<InstanceId>; MAX_CROSSINGS + 1] = [None; MAX_CROSSINGS + 1];
    let absorbs = |media: &MediumStack| media.top(None).filter(|m| m.volume.is_some()).map(|m| m.instance);
    absorbing[0] = absorbs(&seeded);
    let mut crossings = [Crossing::EMPTY; MAX_CROSSINGS];
    let mut count = 0usize;
    let mut ray = Ray { origin: offset_ray_origin(receiver, normal), direction, t_min: 0.0, t_max: reach, time: 0.0 };
    for _ in 0..MAX_INTERSECTIONS {
        let Some(hit) = intersector.intersect(&ray) else { break };
        let Some(instance) = scene.instances.get(hit.instance.0 as usize) else { break };
        let base = scene.material(hit.material);
        let open = instance.light_proxy || !scene.casts_shadows(hit.instance) || !light.shadow_links.lights(hit.instance.0);
        if !open && is_boundary(base) {
            let present = seeded.top(None).map(|m| m.instance);
            let bends = scene.pane(hit.material).is_some_and(|pane| pane.bends());
            let there = seeded.beyond(hit.instance, base.medium_priority);
            let eta = seeded.ior_of(there, Some(base)) / seeded.ior_of(present, None);
            // Inside a medium that outranks it, no interface at all; glass
            // this does not solve through, or between two equal indices,
            // nothing to solve for, and only where the path is changes.
            if there != present && bends && (eta - 1.0).abs() > 1e-3 {
                if count == MAX_CROSSINGS {
                    return None;
                }
                // Glass without vertex normals has no path to walk to.
                let at = surface(scene, &hit)?;
                crossings[count] = Crossing {
                    at,
                    instance: hit.instance,
                    material: hit.material,
                    eta,
                    side: if (-direction).dot(at.ng) >= 0.0 { 1.0 } else { -1.0 },
                    ..Crossing::EMPTY
                };
                count += 1;
            }
            seeded.cross_with(hit.instance, base, scene.density(hit.instance));
            absorbing[count] = absorbs(&seeded);
        }
        let (geometric, _) = hit.facing_forward(ray.direction);
        ray = Ray { origin: offset_ray_origin(hit.position, -geometric), t_max: ray.t_max - hit.t, ..ray };
        if ray.t_max <= 0.0 {
            break;
        }
    }
    if count == 0 {
        return None;
    }
    // Within the path's limits, as though it had bounced its way through.
    if bounce + count as u32 >= config.max_bounces || transmission_bounces + count as u32 > config.max_transmission_bounces {
        return None;
    }

    // ---- The walk -------------------------------------------------------
    let crossings = &mut crossings[..count];
    if !walk(intersector, receiver, normal, lamp.end, crossings, unit) {
        return None;
    }

    // ---- What the path brings --------------------------------------------
    let first = crossings[0].at;
    let to_first = first.p - receiver;
    let first_distance = to_first.length();
    let direction = to_first / first_distance;
    if direction.dot(normal) <= 0.0 {
        return None;
    }
    let last = crossings[count - 1].at;
    let (light_in, plane, target, target_normal) = match lamp.end {
        End::Point(y) => {
            let towards = (last.p - y).try_normalize()?;
            let plane = lamp.plane.unwrap_or_else(|| {
                let frame = Onb::from_normal(towards);
                (frame.t, frame.b)
            });
            // An area light is reached on its surface, offset off it; any
            // other stopped just short of where it is.
            let (target, target_normal) = match &light.kind {
                LightKind::Area(area) => (y, facing(area.normal(), towards)),
                _ => (last.p + (y - last.p) * (1.0 - 1e-3), towards),
            };
            (emitted(light, towards), plane, target, target_normal)
        }
        End::Direction(d) => {
            let frame = Onb::from_normal(d);
            let far = scene.bounds().bounding_radius().max(1.0) * 1e4;
            (lamp.sun, (frame.t, frame.b), last.p + d * far, -d)
        }
    };
    if light_in.is_black() {
        return None;
    }

    // The generalized geometric term: the first crossing's area seen from
    // the receiver, per unit of it the lamp covers through the path.
    let spread = spread(crossings, receiver, lamp.end, plane)?;
    let geometry = first.ng.dot(direction).abs() / (first_distance * first_distance) * spread;
    if !(geometry > 0.0 && geometry.is_finite()) {
        return None;
    }

    // Each crossing's share, and radiance squeezed by the change of index:
    // by (n behind / n ahead)² on the way to the receiver, which over a
    // closed object in air comes to nothing.
    let mut through = Spectrum::ONE;
    let mut previous = receiver;
    for crossing in crossings.iter() {
        let wi = (previous - crossing.at.p).try_normalize()?;
        let eta = crossing.eta_towards(wi);
        let pane = scene.pane(crossing.material)?;
        through *= pane.refracted(wi.dot(crossing.at.n), eta) * (1.0 / (eta * eta));
        previous = crossing.at.p;
    }

    // What each leg's medium absorbs, and whether each leg is clear: past
    // what casts this lamp no shadow and through thin walls, as a shadow
    // ray goes, and stopped by anything else - other glass that bends light
    // included, through which this is not the path.
    let mut from = receiver;
    let mut from_normal = normal;
    for (i, crossing) in crossings.iter().enumerate() {
        let to = crossing.at.p;
        if let Some(medium) = absorbing[i] {
            through *= scene.material(scene.instance(medium).material).transmittance_over((to - from).length());
        }
        through *= shadow_transmittance(
            intersector,
            ShadowGlass::Unbent,
            from,
            facing(from_normal, to - from),
            to,
            facing(crossing.at.ng, from - to),
            &light.shadow_links,
        );
        if through.is_black() {
            return None;
        }
        from = to;
        from_normal = crossing.at.ng;
    }
    through *= shadow_transmittance(
        intersector,
        ShadowGlass::Unbent,
        last.p,
        facing(last.ng, target - last.p),
        target,
        target_normal,
        &light.shadow_links,
    );
    if through.is_black() {
        return None;
    }

    let light_out = light_in * through * (geometry / lamp.density);
    if !light_out.is_finite() {
        return None;
    }
    Some(Caustic {
        direction,
        light: light_out,
        crossings: count,
        lamp: light.visibility,
        layer: light.layer.min(crate::lpe::LAST_LAYER),
    })
}

/// Next event estimation's manifold sample at a surface: a lamp's light
/// through glass, off the surface towards `wo`, split by lobe as
/// [`Lit`] is, the lamp's layer, and how many crossings the path made (for
/// the passes, which see a specular transmission at each).
#[allow(clippy::too_many_arguments)]
pub(super) fn sample_caustic(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    bsdf: &impl Bsdf,
    onb: &Onb,
    wo: Vec3,
    geometric_normal: Vec3,
    media: &MediumStack,
    bounce: u32,
    transmission_bounces: u32,
    sampler: &mut Sampler,
    passes: bool,
) -> (Lit, crate::lpe::LightGroup, usize) {
    // Off the surface only, as Cycles has it: light reaching a surface
    // through its own far side is a transmission's, which this leaves be.
    let Some(caustic) = solve(intersector, config, hit, geometric_normal, media, bounce, transmission_bounces, sampler) else {
        return (Lit::default(), 0, 0);
    };
    let wi = onb.to_local(caustic.direction);
    if wi.z <= 0.0 {
        return (Lit::default(), 0, 0);
    }
    let f = bsdf.eval_selective(wo, wi, caustic.lamp.diffuse, caustic.lamp.specular);
    if f.is_black() {
        return (Lit::default(), 0, 0);
    }
    let diffuse = || bsdf.eval_selective(wo, wi, caustic.lamp.diffuse, false);
    (Lit::of(f, diffuse, caustic.light * wi.z, false, passes), caustic.layer, caustic.crossings)
}

/// A lamp's light through glass onto a shadow catcher, as a share of what
/// the floor would receive in the open: what brightens the backdrop there
/// as that light would a real floor, by the same measure the photons'
/// caustic on a catcher takes (`CausticMap::open_sky`).
#[allow(clippy::too_many_arguments)]
pub(super) fn on_catcher(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    normal: Vec3,
    media: &MediumStack,
    bounce: u32,
    sampler: &mut Sampler,
) -> Spectrum {
    let Some(caustic) = solve(intersector, config, hit, normal, media, bounce, 0, sampler) else {
        return Spectrum::ZERO;
    };
    // A lamp kept off the diffuse lights none of a matte floor.
    if !caustic.lamp.diffuse {
        return Spectrum::ZERO;
    }
    let irradiance = caustic.light * caustic.direction.dot(normal).max(0.0);
    let open = open_irradiance(intersector, hit.position, normal);
    if open > 0.0 {
        irradiance * (1.0 / open)
    } else {
        Spectrum::ZERO
    }
}

/// The irradiance, as luminance, the environment and the lamps would bring
/// to a point facing `normal` with nothing in the way. The environment's is
/// its bound, exact for a floor facing up under an infinite dome, which is
/// nearly every catcher.
fn open_irradiance(intersector: &dyn Intersector, position: Vec3, normal: Vec3) -> f32 {
    let scene = intersector.scene();
    let mut total = scene.environment.irradiance_bound(position, normal);
    for light in scene.lights.lights() {
        if let Some(sample) = light.sample(position, Vec2 { x: 0.5, y: 0.5 }) {
            let density = if sample.is_delta { 1.0 } else { sample.pdf };
            if density > 0.0 {
                total += sample.radiance.luminance() * sample.direction.dot(normal).max(0.0) / density;
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk's 2x2 algebra: a product, an inverse that is one, and a
    /// singular block refused rather than inverted into infinities.
    #[test]
    fn the_blocks_multiply_and_invert() {
        let m = Mat2 { xx: 2.0, xy: 1.0, yx: -1.0, yy: 3.0 };
        let product = m.times(m.inverse().expect("invertible"));
        for (got, want) in [(product.xx, 1.0), (product.xy, 0.0), (product.yx, 0.0), (product.yy, 1.0)] {
            assert!((got - want).abs() < 1e-6, "{product:?}");
        }
        let v = m.apply(vec2(1.0, 2.0));
        assert_eq!((v.x, v.y), (4.0, 5.0));
        assert!(Mat2 { xx: 1.0, xy: 2.0, yx: 2.0, yy: 4.0 }.inverse().is_none());
        assert!(Mat2 { xx: f32::NAN, ..Mat2::IDENTITY }.inverse().is_none());
    }
}
