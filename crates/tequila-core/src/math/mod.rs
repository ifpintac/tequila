//! Vectors, matrices, transforms, bases, rays, and bounds.

pub mod mat;
pub mod onb;
pub mod ray;
pub mod vec;

pub use mat::{Mat4, Transform};
pub use onb::{shading, Onb};
pub use ray::{offset_ray_origin, Aabb, Ray};
pub use vec::{vec2, vec3, vec4, Vec2, Vec3, Vec4};

/// Clamp a cosine into `[-1, 1]` before feeding it to `acos`, which returns
/// NaN for arguments a hair outside the range. Dot products of unit vectors
/// land outside that range often enough in f32 that this is not optional.
#[inline]
pub fn safe_acos(x: f32) -> f32 {
    x.clamp(-1.0, 1.0).acos()
}

#[inline]
pub fn safe_sqrt(x: f32) -> f32 {
    x.max(0.0).sqrt()
}

/// `x^2`, written out because `powi(2)` is not always folded and because it
/// reads better in the microfacet expressions.
#[inline]
pub fn sqr(x: f32) -> f32 {
    x * x
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// Solve `a*x^2 + b*x + c = 0` for real roots, ordered ascending.
///
/// Uses the numerically stable form that avoids catastrophic cancellation when
/// `b^2 >> 4ac` — the naive quadratic formula loses most of its precision on
/// grazing ray-sphere intersections, which is exactly where a renderer needs
/// it most.
pub fn solve_quadratic(a: f32, b: f32, c: f32) -> Option<(f32, f32)> {
    let discriminant = (b as f64) * (b as f64) - 4.0 * (a as f64) * (c as f64);
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let q = if b < 0.0 {
        -0.5 * (b as f64 - root)
    } else {
        -0.5 * (b as f64 + root)
    };
    let t0 = q / a as f64;
    let t1 = if q == 0.0 { t0 } else { c as f64 / q };
    let (lo, hi) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
    Some((lo as f32, hi as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_acos_tolerates_out_of_range_dot_products() {
        assert!(safe_acos(1.000_000_1).is_finite());
        assert!(safe_acos(-1.000_000_1).is_finite());
    }

    #[test]
    fn quadratic_is_stable_for_grazing_roots() {
        // A ray grazing a unit sphere from far away: the naive formula loses
        // the near root entirely here.
        let (a, b, c) = (1.0f32, 2000.0, 1.0);
        let (t0, t1) = solve_quadratic(a, b, c).unwrap();
        for t in [t0, t1] {
            let residual = a * t * t + b * t + c;
            assert!(residual.abs() < 1e-3, "root {t} has residual {residual}");
        }
    }

    #[test]
    fn quadratic_reports_no_real_roots() {
        assert!(solve_quadratic(1.0, 0.0, 1.0).is_none());
    }
}
