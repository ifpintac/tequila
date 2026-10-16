//! Vector types.
//!
//! Deliberately hand-written rather than pulled from a general-purpose math
//! crate: the renderer depends on exact, stable floating-point semantics for
//! determinism (`docs/01-RENDERER-CORE.md` §4.6), and on knowing precisely
//! which operations are `f32` and which widen to `f64`.

use std::ops::{
    Add, AddAssign, Div, DivAssign, Index, IndexMut, Mul, MulAssign, Neg, Sub, SubAssign,
};

/// Two-component vector. Used for UVs, sample points, and screen coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// Three-component vector. Doubles as a point, a direction, and an RGB triple
/// where a distinct type would cost more in conversion noise than it buys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Four-component vector, primarily for homogeneous coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[inline]
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

#[inline]
pub const fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

#[inline]
pub const fn vec4(x: f32, y: f32, z: f32, w: f32) -> Vec4 {
    Vec4 { x, y, z, w }
}

impl Vec2 {
    pub const ZERO: Self = vec2(0.0, 0.0);
    pub const ONE: Self = vec2(1.0, 1.0);

    #[inline]
    pub const fn splat(v: f32) -> Self {
        vec2(v, v)
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    #[inline]
    pub fn normalize(self) -> Self {
        self / self.length()
    }

    #[inline]
    pub fn min(self, o: Self) -> Self {
        vec2(self.x.min(o.x), self.y.min(o.y))
    }

    #[inline]
    pub fn max(self, o: Self) -> Self {
        vec2(self.x.max(o.x), self.y.max(o.y))
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Vec3 {
    pub const ZERO: Self = vec3(0.0, 0.0, 0.0);
    pub const ONE: Self = vec3(1.0, 1.0, 1.0);
    pub const X: Self = vec3(1.0, 0.0, 0.0);
    pub const Y: Self = vec3(0.0, 1.0, 0.0);
    pub const Z: Self = vec3(0.0, 0.0, 1.0);

    #[inline]
    pub const fn splat(v: f32) -> Self {
        vec3(v, v, v)
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Absolute value of the dot product. Used constantly for cosine terms on
    /// two-sided surfaces, where the sign carries no information.
    #[inline]
    pub fn abs_dot(self, o: Self) -> f32 {
        self.dot(o).abs()
    }

    #[inline]
    pub fn cross(self, o: Self) -> Self {
        // Computed in f64 to avoid the catastrophic cancellation that a naive
        // f32 cross product suffers on near-parallel vectors, which shows up as
        // shading-normal noise on tessellated CAD surfaces.
        let (ax, ay, az) = (self.x as f64, self.y as f64, self.z as f64);
        let (bx, by, bz) = (o.x as f64, o.y as f64, o.z as f64);
        vec3(
            (ay * bz - az * by) as f32,
            (az * bx - ax * bz) as f32,
            (ax * by - ay * bx) as f32,
        )
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    #[inline]
    pub fn normalize(self) -> Self {
        self / self.length()
    }

    /// Normalize, returning `None` for a zero or non-finite vector rather than
    /// silently producing NaNs that propagate into the framebuffer.
    #[inline]
    pub fn try_normalize(self) -> Option<Self> {
        let len2 = self.length_squared();
        if len2 > 0.0 && len2.is_finite() {
            Some(self / len2.sqrt())
        } else {
            None
        }
    }

    #[inline]
    pub fn abs(self) -> Self {
        vec3(self.x.abs(), self.y.abs(), self.z.abs())
    }

    #[inline]
    pub fn min(self, o: Self) -> Self {
        vec3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    #[inline]
    pub fn max(self, o: Self) -> Self {
        vec3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    #[inline]
    pub fn min_component(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }

    #[inline]
    pub fn max_component(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }

    /// Index of the largest component. Used to pick the dominant axis for
    /// spectral MIS and for BVH traversal orderings.
    #[inline]
    pub fn max_dimension(self) -> usize {
        if self.x > self.y {
            if self.x > self.z {
                0
            } else {
                2
            }
        } else if self.y > self.z {
            1
        } else {
            2
        }
    }

    #[inline]
    pub fn average(self) -> f32 {
        (self.x + self.y + self.z) * (1.0 / 3.0)
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    #[inline]
    pub fn is_black(self) -> bool {
        self.x == 0.0 && self.y == 0.0 && self.z == 0.0
    }

    #[inline]
    pub fn sqrt(self) -> Self {
        vec3(self.x.sqrt(), self.y.sqrt(), self.z.sqrt())
    }

    #[inline]
    pub fn exp(self) -> Self {
        vec3(self.x.exp(), self.y.exp(), self.z.exp())
    }

    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self * (1.0 - t) + o * t
    }

    #[inline]
    pub fn clamp(self, lo: f32, hi: f32) -> Self {
        vec3(
            self.x.clamp(lo, hi),
            self.y.clamp(lo, hi),
            self.z.clamp(lo, hi),
        )
    }

    /// Reflect `self` about `n`. Both are expected to be unit length and to
    /// point away from the surface.
    #[inline]
    pub fn reflect(self, n: Self) -> Self {
        -self + n * (2.0 * self.dot(n))
    }

    /// Flip `n` so that it lies in the same hemisphere as `v`.
    #[inline]
    pub fn face_forward(self, v: Self) -> Self {
        if self.dot(v) < 0.0 {
            -self
        } else {
            self
        }
    }

    /// Refract `self` (pointing away from the surface) about normal `n` with
    /// relative index of refraction `eta = ior_incident / ior_transmitted`.
    /// Returns `None` on total internal reflection.
    #[inline]
    pub fn refract(self, n: Self, eta: f32) -> Option<Self> {
        let cos_i = self.dot(n);
        let sin2_i = (1.0 - cos_i * cos_i).max(0.0);
        let sin2_t = eta * eta * sin2_i;
        if sin2_t >= 1.0 {
            return None; // total internal reflection
        }
        let cos_t = (1.0 - sin2_t).sqrt();
        Some(-self * eta + n * (eta * cos_i - cos_t))
    }
}

impl Vec4 {
    pub const ZERO: Self = vec4(0.0, 0.0, 0.0, 0.0);

    #[inline]
    pub const fn from_vec3(v: Vec3, w: f32) -> Self {
        vec4(v.x, v.y, v.z, w)
    }

    #[inline]
    pub const fn xyz(self) -> Vec3 {
        vec3(self.x, self.y, self.z)
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }
}

// ---------------------------------------------------------------------------
// Operator boilerplate
// ---------------------------------------------------------------------------

macro_rules! impl_vec_ops {
    ($t:ty, $ctor:ident, $($f:ident),+) => {
        impl Add for $t {
            type Output = Self;
            #[inline]
            fn add(self, o: Self) -> Self { $ctor($(self.$f + o.$f),+) }
        }
        impl Sub for $t {
            type Output = Self;
            #[inline]
            fn sub(self, o: Self) -> Self { $ctor($(self.$f - o.$f),+) }
        }
        impl Mul for $t {
            type Output = Self;
            #[inline]
            fn mul(self, o: Self) -> Self { $ctor($(self.$f * o.$f),+) }
        }
        impl Div for $t {
            type Output = Self;
            #[inline]
            fn div(self, o: Self) -> Self { $ctor($(self.$f / o.$f),+) }
        }
        impl Mul<f32> for $t {
            type Output = Self;
            #[inline]
            fn mul(self, s: f32) -> Self { $ctor($(self.$f * s),+) }
        }
        impl Mul<$t> for f32 {
            type Output = $t;
            #[inline]
            fn mul(self, v: $t) -> $t { $ctor($(v.$f * self),+) }
        }
        impl Div<f32> for $t {
            type Output = Self;
            #[inline]
            fn div(self, s: f32) -> Self { let inv = 1.0 / s; $ctor($(self.$f * inv),+) }
        }
        impl Neg for $t {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self { $ctor($(-self.$f),+) }
        }
        impl AddAssign for $t {
            #[inline]
            fn add_assign(&mut self, o: Self) { $(self.$f += o.$f;)+ }
        }
        impl SubAssign for $t {
            #[inline]
            fn sub_assign(&mut self, o: Self) { $(self.$f -= o.$f;)+ }
        }
        impl MulAssign<f32> for $t {
            #[inline]
            fn mul_assign(&mut self, s: f32) { $(self.$f *= s;)+ }
        }
        impl MulAssign for $t {
            #[inline]
            fn mul_assign(&mut self, o: Self) { $(self.$f *= o.$f;)+ }
        }
        impl DivAssign<f32> for $t {
            #[inline]
            fn div_assign(&mut self, s: f32) { let inv = 1.0 / s; $(self.$f *= inv;)+ }
        }
    };
}

impl_vec_ops!(Vec2, vec2, x, y);
impl_vec_ops!(Vec3, vec3, x, y, z);
impl_vec_ops!(Vec4, vec4, x, y, z, w);

impl Index<usize> for Vec3 {
    type Output = f32;
    #[inline]
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            _ => panic!("Vec3 index out of range: {i}"),
        }
    }
}

impl IndexMut<usize> for Vec3 {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut f32 {
        match i {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            _ => panic!("Vec3 index out of range: {i}"),
        }
    }
}

impl From<[f32; 3]> for Vec3 {
    #[inline]
    fn from(a: [f32; 3]) -> Self {
        vec3(a[0], a[1], a[2])
    }
}

impl From<Vec3> for [f32; 3] {
    #[inline]
    fn from(v: Vec3) -> Self {
        [v.x, v.y, v.z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refract_matches_snells_law() {
        // 45 degrees into glass (eta = 1/1.5) must obey sin(t) = eta * sin(i).
        let n = Vec3::Z;
        let theta_i = std::f32::consts::FRAC_PI_4;
        let wo = vec3(theta_i.sin(), 0.0, theta_i.cos());
        let eta = 1.0 / 1.5;
        let wt = wo.refract(n, eta).expect("no TIR entering a denser medium");
        let sin_t = (1.0 - wt.z * wt.z).sqrt();
        assert!((sin_t - eta * theta_i.sin()).abs() < 1e-5);
    }

    #[test]
    fn refract_reports_total_internal_reflection() {
        // Leaving glass at 60 degrees is past the ~41.8 degree critical angle.
        let theta = std::f32::consts::PI / 3.0;
        let wo = vec3(theta.sin(), 0.0, theta.cos());
        assert!(wo.refract(Vec3::Z, 1.5).is_none());
    }

    #[test]
    fn reflect_preserves_angle() {
        let wo = vec3(0.3, 0.4, 0.866_025_4).normalize();
        let wi = wo.reflect(Vec3::Z);
        assert!((wi.z - wo.z).abs() < 1e-6);
        assert!((wi.x + wo.x).abs() < 1e-6);
    }
}
