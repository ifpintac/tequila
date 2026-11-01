//! Rays, axis-aligned bounds, and robust ray offsetting.

use super::vec::{vec3, Vec3};

/// A ray with an inclusive-exclusive parametric range `[t_min, t_max)`.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
    pub t_min: f32,
    pub t_max: f32,
    /// Shutter time in `[0, 1)` for motion blur (`docs/01` §8.4).
    pub time: f32,
}

impl Ray {
    #[inline]
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Ray {
            origin,
            direction,
            t_min: 0.0,
            t_max: f32::INFINITY,
            time: 0.0,
        }
    }

    #[inline]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb {
    pub const EMPTY: Self = Aabb {
        min: vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY),
        max: vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
    };

    #[inline]
    pub fn from_points(a: Vec3, b: Vec3) -> Self {
        Aabb {
            min: a.min(b),
            max: a.max(b),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    #[inline]
    pub fn extend_point(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    #[inline]
    pub fn extend(&mut self, o: &Aabb) {
        if !o.is_empty() {
            self.min = self.min.min(o.min);
            self.max = self.max.max(o.max);
        }
    }

    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn diagonal(&self) -> Vec3 {
        if self.is_empty() {
            Vec3::ZERO
        } else {
            self.max - self.min
        }
    }

    /// Radius of the bounding sphere. Used to frame the camera on load and to
    /// pick a default ground-plane and environment size.
    #[inline]
    pub fn bounding_radius(&self) -> f32 {
        self.diagonal().length() * 0.5
    }

    /// Transform the box by an affine matrix, taking the bounds of the eight
    /// transformed corners. Cheaper closed forms exist, but this is called
    /// once per instance at scene build, not per ray.
    pub fn transformed(&self, t: &super::mat::Transform) -> Aabb {
        if self.is_empty() {
            return *self;
        }
        let mut out = Aabb::EMPTY;
        for i in 0..8 {
            let c = vec3(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            );
            out.extend_point(t.point(c));
        }
        out
    }
}

/// Offset a ray origin off a surface to avoid self-intersection: a step along
/// the geometric normal sized to the precision single floats have at the
/// point.
///
/// A fixed epsilon cannot work here: `docs/01-RENDERER-CORE.md` §3.1 notes
/// that product CAD scenes span both ends of the scale range at once (a
/// millimetre fillet on a two-metre assembly), so the offset has to scale with
/// the magnitude of the intersection point. The step is 2^-18 of the point's
/// largest coordinate, 32 to 64 ULPs of it: enough to clear the few ULPs a hit
/// point computed from its triangle's corners is out by (`EmbreeIntersector`),
/// and well short of the gaps a model leaves between parts. A point is only
/// as precise as the numbers it was worked out from, though, and a hit's may
/// be far larger than its own coordinates - a part modelled a metre from
/// where it stands - so a hit takes the step for those
/// ([`offset_ray_origin_at`], `Hit::offset`). The step is never less than it
/// is at 1/8, 0.48 µm in metres: for a point that is not a hit, which says
/// nothing of what it was worked out from, that covers parts modelled up to a
/// metre and a half from where they stand.
///
/// It was the integer-magnitude step of [Wächter & Binder 2019, *A Fast and
/// Robust Method for Avoiding Self-Intersection*, Ray Tracing Gems]: 256 ULPs
/// of each coordinate, and a fixed 1/65536 for one within 1/32 of zero -
/// constants for a scene a unit across. In metres, with an imported product
/// standing at the origin, most of it is within 1/32 of some axis plane, and
/// the step there was 15 µm: past the logo printed 2 µm behind a handheld's
/// lens and into the mask 4 µm behind it, so the lens showed the inside of
/// the mask, near black. A coordinate near zero is computed from terms the
/// size of the others, so it is out by a few ULPs of the largest coordinate,
/// not of itself, and the step is sized to that.
#[inline]
pub fn offset_ray_origin(p: Vec3, geometric_normal: Vec3) -> Vec3 {
    offset_ray_origin_at(p, geometric_normal, 0.0)
}

/// [`offset_ray_origin`] for a point worked out from numbers as large as
/// `magnitude`: a hit, from its triangle's corners and its part's placement.
/// A part modelled a metre and a half from where it stands has its points only
/// to a few ULPs of that, even at the world's origin; and in the part's own
/// coordinates, where the ray tracer meets the next ray with it, they are that
/// large.
#[inline]
pub fn offset_ray_origin_at(p: Vec3, geometric_normal: Vec3, magnitude: f32) -> Vec3 {
    const ORIGIN: f32 = 1.0 / 8.0;
    const SCALE: f32 = 1.0 / 262_144.0;
    let magnitude = p.x.abs().max(p.y.abs()).max(p.z.abs()).max(magnitude).max(ORIGIN);
    p + geometric_normal * (magnitude * SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_moves_along_the_normal_at_every_scale() {
        // Both ends of the CAD scale range, plus the near-origin fallback.
        for scale in [0.0f32, 1e-3, 1.0, 1e3, 1e6] {
            let p = vec3(scale, scale * 0.5, -scale);
            let n = vec3(0.0, 1.0, 0.0);
            let o = offset_ray_origin(p, n);
            assert!(o.y > p.y, "no offset applied at scale {scale}");
            // The offset must stay small relative to the point's magnitude,
            // or large scenes develop visible light leaks at contact shadows.
            let rel = (o.y - p.y) / scale.max(1.0);
            assert!(rel < 1e-3, "offset too coarse at scale {scale}: {rel}");
        }
    }

    #[test]
    fn on_a_product_in_metres_the_step_is_under_a_micron() {
        // The palma P8's lens, its back face 0.3 mm behind the world's
        // origin and 8 to 13 cm up, a logo printed 2 µm behind it; the
        // floor under the product; and a part a hand's length away.
        let ulp = |x: f32| f32::from_bits(x.abs().to_bits() + 1) - x.abs();
        for p in [vec3(0.03, 0.08, -0.0003), vec3(-0.037, 0.134, -0.0003), vec3(0.0, 0.0, 0.0), vec3(0.3, 0.1, 0.3)] {
            let step = p.z - offset_ray_origin(p, -Vec3::Z).z;
            let largest = p.x.abs().max(p.y.abs()).max(p.z.abs());
            assert!(step > 16.0 * ulp(largest), "{p:?}: a step of {step} does not clear the point's precision");
            assert!(step < 1.5e-6, "{p:?}: a step of {step} passes what lies 2 µm behind");
        }
    }

    #[test]
    fn a_point_worked_out_from_large_numbers_steps_by_them() {
        // At the world's origin, on a part modelled 3 m from there.
        let p = vec3(0.01, 0.0, 0.02);
        let near = offset_ray_origin(p, Vec3::Y).y - p.y;
        let far = offset_ray_origin_at(p, Vec3::Y, 3.0).y - p.y;
        assert!(far > 16.0 * f32::EPSILON * 3.0, "a step of {far} does not clear 3 m's precision");
        assert!(far > 20.0 * near, "{far} against {near}");
        // And no less than its own coordinates ask for.
        assert_eq!(offset_ray_origin_at(p, Vec3::Y, 0.0), offset_ray_origin(p, Vec3::Y));
    }

    #[test]
    fn offset_is_signed_by_the_normal() {
        let p = vec3(100.0, 100.0, 100.0);
        let up = offset_ray_origin(p, Vec3::Y);
        let down = offset_ray_origin(p, -Vec3::Y);
        assert!(up.y > p.y && down.y < p.y);
    }

    #[test]
    fn transformed_bounds_contain_transformed_corners() {
        use super::super::mat::{Mat4, Transform};
        let b = Aabb::from_points(vec3(-1.0, -1.0, -1.0), vec3(1.0, 2.0, 3.0));
        let t = Transform::from_matrix(
            Mat4::translation(vec3(5.0, 0.0, 0.0)) * Mat4::rotation(Vec3::Y, 0.7),
        )
        .unwrap();
        let tb = b.transformed(&t);
        for i in 0..8 {
            let c = vec3(
                if i & 1 == 0 { b.min.x } else { b.max.x },
                if i & 2 == 0 { b.min.y } else { b.max.y },
                if i & 4 == 0 { b.min.z } else { b.max.z },
            );
            let p = t.point(c);
            assert!(p.x >= tb.min.x - 1e-4 && p.x <= tb.max.x + 1e-4);
            assert!(p.y >= tb.min.y - 1e-4 && p.y <= tb.max.y + 1e-4);
            assert!(p.z >= tb.min.z - 1e-4 && p.z <= tb.max.z + 1e-4);
        }
    }
}
