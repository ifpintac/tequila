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

/// Offset a ray origin off a surface to avoid self-intersection, using the
/// integer-magnitude method of [Wächter & Binder 2019, *A Fast and Robust
/// Method for Avoiding Self-Intersection*, Ray Tracing Gems].
///
/// A fixed epsilon cannot work here: `docs/01-RENDERER-CORE.md` §3.1 notes
/// that product CAD scenes span both ends of the scale range at once (a
/// millimetre fillet on a two-metre assembly), so the offset has to scale with
/// the magnitude of the intersection point. This does that by stepping a fixed
/// number of ULPs along the geometric normal.
#[inline]
pub fn offset_ray_origin(p: Vec3, geometric_normal: Vec3) -> Vec3 {
    const ORIGIN: f32 = 1.0 / 32.0;
    const FLOAT_SCALE: f32 = 1.0 / 65536.0;
    const INT_SCALE: f32 = 256.0;

    let of_i = [
        (INT_SCALE * geometric_normal.x) as i32,
        (INT_SCALE * geometric_normal.y) as i32,
        (INT_SCALE * geometric_normal.z) as i32,
    ];

    #[inline]
    fn bump(p: f32, of: i32) -> f32 {
        f32::from_bits(((p.to_bits() as i32) + if p < 0.0 { -of } else { of }) as u32)
    }

    let p_i = vec3(bump(p.x, of_i[0]), bump(p.y, of_i[1]), bump(p.z, of_i[2]));

    // Near the origin the integer step is smaller than float precision needs,
    // so fall back to an absolute offset there.
    vec3(
        if p.x.abs() < ORIGIN {
            p.x + FLOAT_SCALE * geometric_normal.x
        } else {
            p_i.x
        },
        if p.y.abs() < ORIGIN {
            p.y + FLOAT_SCALE * geometric_normal.y
        } else {
            p_i.y
        },
        if p.z.abs() < ORIGIN {
            p.z + FLOAT_SCALE * geometric_normal.z
        } else {
            p_i.z
        },
    )
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
