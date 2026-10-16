//! Orthonormal bases and shading-space conversions.
//!
//! Every BSDF in Tequila evaluates in *shading space*, where the shading
//! normal is `+Z`. This keeps the lobe implementations free of normal-mapping
//! bookkeeping and makes the `docs/01-RENDERER-CORE.md` §3.3 contract easy to
//! test in isolation.

use super::vec::{vec3, Vec3};

/// A right-handed orthonormal basis with `n` as the third axis.
#[derive(Clone, Copy, Debug)]
pub struct Onb {
    pub t: Vec3,
    pub b: Vec3,
    pub n: Vec3,
}

impl Onb {
    /// Branchless basis construction from a single unit vector
    /// [Duff et al. 2017, *Building an Orthonormal Basis, Revisited*].
    ///
    /// The `sign`/`copysign` formulation is used rather than the older Frisvad
    /// method because Frisvad loses precision catastrophically as `n.z`
    /// approaches -1, which shows up as visible seams on spheres.
    #[inline]
    pub fn from_normal(n: Vec3) -> Self {
        let sign = (1.0f32).copysign(n.z);
        let a = -1.0 / (sign + n.z);
        let b = n.x * n.y * a;
        Onb {
            t: vec3(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x),
            b: vec3(b, sign + n.y * n.y * a, -n.y),
            n,
        }
    }

    /// Basis with a caller-supplied tangent direction, used for anisotropic
    /// lobes where the tangent field carries the anisotropy orientation.
    /// The tangent is Gram-Schmidt orthogonalized against `n`; if it is
    /// degenerate (parallel to `n`) an arbitrary basis is used instead.
    #[inline]
    pub fn from_normal_tangent(n: Vec3, tangent: Vec3) -> Self {
        let t = tangent - n * n.dot(tangent);
        match t.try_normalize() {
            Some(t) => Onb {
                t,
                b: n.cross(t),
                n,
            },
            None => Self::from_normal(n),
        }
    }

    /// World space -> shading space (normal becomes `+Z`).
    #[inline]
    pub fn to_local(&self, v: Vec3) -> Vec3 {
        vec3(v.dot(self.t), v.dot(self.b), v.dot(self.n))
    }

    /// Shading space -> world space.
    #[inline]
    pub fn to_world(&self, v: Vec3) -> Vec3 {
        self.t * v.x + self.b * v.y + self.n * v.z
    }
}

/// Trigonometric helpers for directions expressed in shading space, where the
/// normal is `+Z`. Written out rather than recomputed inline because these
/// appear in every microfacet expression and a sign error in one of them is
/// hard to spot in an image.
pub mod shading {
    use super::Vec3;

    #[inline]
    pub fn cos_theta(w: Vec3) -> f32 {
        w.z
    }

    #[inline]
    pub fn cos2_theta(w: Vec3) -> f32 {
        w.z * w.z
    }

    #[inline]
    pub fn abs_cos_theta(w: Vec3) -> f32 {
        w.z.abs()
    }

    #[inline]
    pub fn sin2_theta(w: Vec3) -> f32 {
        (1.0 - cos2_theta(w)).max(0.0)
    }

    #[inline]
    pub fn sin_theta(w: Vec3) -> f32 {
        sin2_theta(w).sqrt()
    }

    #[inline]
    pub fn tan2_theta(w: Vec3) -> f32 {
        sin2_theta(w) / cos2_theta(w)
    }

    #[inline]
    pub fn cos_phi(w: Vec3) -> f32 {
        let s = sin_theta(w);
        if s == 0.0 {
            1.0
        } else {
            (w.x / s).clamp(-1.0, 1.0)
        }
    }

    #[inline]
    pub fn sin_phi(w: Vec3) -> f32 {
        let s = sin_theta(w);
        if s == 0.0 {
            0.0
        } else {
            (w.y / s).clamp(-1.0, 1.0)
        }
    }

    /// True when `a` and `b` lie in the same hemisphere about `+Z`.
    #[inline]
    pub fn same_hemisphere(a: Vec3, b: Vec3) -> bool {
        a.z * b.z > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_orthonormal(onb: &Onb) {
        assert!((onb.t.length() - 1.0).abs() < 1e-5);
        assert!((onb.b.length() - 1.0).abs() < 1e-5);
        assert!(onb.t.dot(onb.b).abs() < 1e-5);
        assert!(onb.t.dot(onb.n).abs() < 1e-5);
        assert!(onb.b.dot(onb.n).abs() < 1e-5);
    }

    #[test]
    fn basis_is_orthonormal_everywhere_including_the_pole() {
        // The z = -1 pole is where the naive Frisvad construction breaks.
        for n in [
            Vec3::Z,
            -Vec3::Z,
            vec3(0.0, 0.0, -0.999_999_9),
            vec3(1.0, 0.0, 0.0),
            vec3(0.3, -0.5, 0.81).normalize(),
        ] {
            assert_orthonormal(&Onb::from_normal(n.normalize()));
        }
    }

    #[test]
    fn local_and_world_round_trip() {
        let onb = Onb::from_normal(vec3(0.2, 0.7, -0.68).normalize());
        let v = vec3(-0.4, 0.15, 0.9).normalize();
        let round = onb.to_world(onb.to_local(v));
        assert!((round - v).length() < 1e-5);
    }
}
