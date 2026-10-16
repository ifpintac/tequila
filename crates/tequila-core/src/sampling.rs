//! Sampling warps and the estimator helpers built on them.
//!
//! Every warp is paired with its PDF in the same measure the renderer uses
//! (solid angle for directions, area for surfaces). The χ² tests in
//! `docs/05-ROADMAP-AND-TESTING.md` §3.2 check each warp against its PDF; a
//! warp added without a matching PDF is a build failure waiting to happen.

use crate::math::{vec2, vec3, Vec2, Vec3};
use std::f32::consts::{FRAC_1_PI, PI, TAU};

/// Uniform sample of a unit disk by the concentric (Shirley-Chiu) mapping.
///
/// Preferred over the polar mapping `(sqrt(u), 2*pi*v)` because it preserves
/// the stratification of the input samples — the polar map distorts strata
/// badly near the centre, which shows up as structured noise in depth of
/// field, the one place where disk sampling is most visible.
#[inline]
pub fn sample_uniform_disk_concentric(u: Vec2) -> Vec2 {
    let offset = vec2(2.0 * u.x - 1.0, 2.0 * u.y - 1.0);
    if offset.x == 0.0 && offset.y == 0.0 {
        return Vec2::ZERO;
    }
    let (r, theta) = if offset.x.abs() > offset.y.abs() {
        (offset.x, (PI / 4.0) * (offset.y / offset.x))
    } else {
        (offset.y, (PI / 2.0) - (PI / 4.0) * (offset.x / offset.y))
    };
    vec2(r * theta.cos(), r * theta.sin())
}

/// Uniform sample of a unit disk with a polygonal aperture of `blades` sides,
/// rotated by `rotation` radians. Used for the camera's bladed bokeh
/// (`docs/01` §8.1).
pub fn sample_polygon_aperture(u: Vec2, blades: u32, rotation: f32) -> Vec2 {
    debug_assert!(blades >= 3);
    let blades_f = blades as f32;
    // Pick a triangular wedge, then sample it uniformly. Rescaling `u.x` after
    // choosing the wedge keeps the sample stratified rather than throwing away
    // the low-discrepancy structure on a discrete choice.
    let wedge = (u.x * blades_f).floor().min(blades_f - 1.0);
    let ux = (u.x * blades_f - wedge).clamp(0.0, 1.0);

    let a0 = TAU * wedge / blades_f + rotation;
    let a1 = TAU * (wedge + 1.0) / blades_f + rotation;
    let v0 = vec2(a0.cos(), a0.sin());
    let v1 = vec2(a1.cos(), a1.sin());

    // Uniform point in the triangle (0, v0, v1) by the square-root warp.
    let (mut s, mut t) = (ux, u.y);
    if s + t > 1.0 {
        s = 1.0 - s;
        t = 1.0 - t;
    }
    v0 * s + v1 * t
}

/// Cosine-weighted hemisphere sample about `+Z`. PDF is
/// [`cosine_hemisphere_pdf`].
#[inline]
/// A direction scattered by a Henyey-Greenstein phase function about
/// `forward`, the way the light was travelling. Positive `g` keeps it going
/// forward, negative sends it back, zero is isotropic. Sampled exactly, so
/// the phase function's value over its density is one.
pub fn sample_henyey_greenstein(forward: Vec3, g: f32, u: Vec2) -> Vec3 {
    let cos_theta = if g.abs() < 1e-3 {
        1.0 - 2.0 * u.x
    } else {
        let s = (1.0 - g * g) / (1.0 - g + 2.0 * g * u.x);
        (1.0 + g * g - s * s) / (2.0 * g)
    }
    .clamp(-1.0, 1.0);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = TAU * u.y;
    let frame = crate::math::Onb::from_normal(forward);
    frame.to_world(vec3(sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta))
}

pub fn sample_cosine_hemisphere(u: Vec2) -> Vec3 {
    let d = sample_uniform_disk_concentric(u);
    let z = (1.0 - d.x * d.x - d.y * d.y).max(0.0).sqrt();
    vec3(d.x, d.y, z)
}

#[inline]
pub fn cosine_hemisphere_pdf(cos_theta: f32) -> f32 {
    cos_theta.max(0.0) * FRAC_1_PI
}

/// Uniform hemisphere sample about `+Z`.
#[inline]
pub fn sample_uniform_hemisphere(u: Vec2) -> Vec3 {
    let z = u.x;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = TAU * u.y;
    vec3(r * phi.cos(), r * phi.sin(), z)
}

pub const UNIFORM_HEMISPHERE_PDF: f32 = FRAC_1_PI * 0.5;

/// Uniform sphere sample.
#[inline]
pub fn sample_uniform_sphere(u: Vec2) -> Vec3 {
    let z = 1.0 - 2.0 * u.x;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = TAU * u.y;
    vec3(r * phi.cos(), r * phi.sin(), z)
}

pub const UNIFORM_SPHERE_PDF: f32 = FRAC_1_PI * 0.25;

/// Uniform sample of a cone of half-angle `acos(cos_theta_max)` about `+Z`.
/// Used for sphere lights and the sun disc.
#[inline]
pub fn sample_uniform_cone(u: Vec2, cos_theta_max: f32) -> Vec3 {
    let cos_theta = (1.0 - u.x) + u.x * cos_theta_max;
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = TAU * u.y;
    vec3(sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta)
}

#[inline]
pub fn uniform_cone_pdf(cos_theta_max: f32) -> f32 {
    1.0 / (TAU * (1.0 - cos_theta_max))
}

/// Uniform barycentric coordinates on a triangle. Returns `(b0, b1)`; the
/// third coordinate is `1 - b0 - b1`.
#[inline]
pub fn sample_uniform_triangle(u: Vec2) -> Vec2 {
    // Low-distortion mapping [Heitz 2019]: a fold rather than a square root,
    // which preserves stratification and avoids the precision loss of
    // `sqrt(u)` for small `u`.
    let (mut b0, mut b1) = (u.x * 0.5, u.y * 0.5);
    let offset = b1 - b0;
    if offset > 0.0 {
        b1 += offset;
    } else {
        b0 -= offset;
    }
    vec2(b0, b1)
}

/// Sample a discrete distribution given by unnormalized `weights`.
/// Returns `(index, pmf, remapped_u)`. The remapped sample lets the caller
/// reuse the same random number for a continuous decision afterwards without
/// losing stratification.
pub fn sample_discrete(weights: &[f32], u: f32) -> Option<(usize, f32, f32)> {
    let total: f32 = weights.iter().sum();
    // Written as a negated comparison deliberately: it rejects NaN as well as
    // zero and negative totals, which `total <= 0.0` would let through.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(total > 0.0) {
        return None;
    }
    let target = u * total;
    let mut cumulative = 0.0;
    for (i, &w) in weights.iter().enumerate() {
        let next = cumulative + w;
        if target < next || i + 1 == weights.len() {
            if w <= 0.0 {
                // The final-element fallback can land on a zero-weight entry
                // when `u` rounds up; scan back for a usable one.
                return weights
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, &w)| w > 0.0)
                    .map(|(i, &w)| (i, w / total, 0.5));
            }
            let remapped = ((target - cumulative) / w).clamp(0.0, 1.0 - f32::EPSILON / 2.0);
            return Some((i, w / total, remapped));
        }
        cumulative = next;
    }
    None
}

/// Balance heuristic, `beta = 1`.
#[inline]
pub fn mis_balance(pdf_a: f32, pdf_b: f32) -> f32 {
    if pdf_a.is_infinite() {
        return 1.0;
    }
    let denom = pdf_a + pdf_b;
    if denom <= 0.0 {
        0.0
    } else {
        pdf_a / denom
    }
}

/// Power heuristic with exponent `beta = 2` [Veach & Guibas 1995], the
/// combination rule `docs/01-RENDERER-CORE.md` §3.2 specifies.
///
/// An infinite `pdf_a` means the sampled strategy was a delta distribution:
/// the other strategy can never generate that direction, so the weight is 1.
#[inline]
pub fn mis_power(pdf_a: f32, pdf_b: f32) -> f32 {
    if pdf_a.is_infinite() {
        return 1.0;
    }
    // Squaring in f64 avoids overflow to infinity for large PDFs, which would
    // otherwise silently turn a legitimate weight into 1.0 or NaN.
    let a = pdf_a as f64;
    let b = pdf_b as f64;
    let (a2, b2) = (a * a, b * b);
    let denom = a2 + b2;
    if denom <= 0.0 {
        0.0
    } else {
        (a2 / denom) as f32
    }
}

/// Convert a PDF expressed per unit area on a light into a PDF per unit solid
/// angle at the shading point. The `d^2 / |cos|` factor is the single most
/// commonly dropped term in a path tracer, and dropping it produces images
/// that are wrong by a distance-dependent factor no visual inspection catches.
#[inline]
pub fn area_to_solid_angle_pdf(pdf_area: f32, distance_squared: f32, cos_light: f32) -> f32 {
    let c = cos_light.abs();
    if c <= 1e-9 {
        f32::INFINITY
    } else {
        pdf_area * distance_squared / c
    }
}

/// Equal-area mapping from a direction to equirectangular `(u, v)` in
/// `[0,1)^2`, with `+Y` up. `u` runs around the horizon from `-Z`.
#[inline]
pub fn direction_to_equirect(d: Vec3) -> Vec2 {
    let theta = d.y.clamp(-1.0, 1.0).acos(); // 0 at +Y
    let phi = d.x.atan2(-d.z); // 0 looking down -Z
    let u = (phi / TAU + 0.5).rem_euclid(1.0);
    let v = theta / PI;
    vec2(u, v)
}

/// Inverse of [`direction_to_equirect`].
#[inline]
pub fn equirect_to_direction(uv: Vec2) -> Vec3 {
    let phi = (uv.x - 0.5) * TAU;
    let theta = uv.y * PI;
    let sin_theta = theta.sin();
    vec3(sin_theta * phi.sin(), theta.cos(), -sin_theta * phi.cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Henyey-Greenstein's `g` is the mean cosine of the angle it turns
    /// light through, so sampled faithfully the samples average to it.
    #[test]
    fn henyey_greenstein_samples_average_to_g() {
        let forward = vec3(0.3, -0.5, 0.81).normalize();
        for g in [-0.6f32, 0.0, 0.4, 0.85] {
            let n = 20000;
            let mut total = 0.0f64;
            for i in 0..n {
                let u = vec2((i as f32 + 0.5) / n as f32, ((i * 7919) % n) as f32 / n as f32);
                let d = sample_henyey_greenstein(forward, g, u);
                assert!((d.length() - 1.0).abs() < 1e-4);
                total += d.dot(forward) as f64;
            }
            let mean = total / n as f64;
            assert!((mean - g as f64).abs() < 0.01, "g = {g}: mean cosine {mean}");
        }
    }
    use crate::rng::Pcg32;

    #[test]
    fn concentric_disk_stays_inside_the_unit_circle() {
        let mut r = Pcg32::new(1, 1);
        for _ in 0..100_000 {
            let p = sample_uniform_disk_concentric(vec2(r.next_f32(), r.next_f32()));
            assert!(p.length() <= 1.0 + 1e-6);
        }
    }

    /// Integrating 1/pdf over the warp must recover the measure of the domain.
    /// This is the cheap form of the χ² test and catches missing Jacobians.
    #[test]
    fn cosine_hemisphere_pdf_integrates_to_one() {
        let mut r = Pcg32::new(2, 2);
        let n = 400_000;
        let mut sum = 0.0f64;
        for _ in 0..n {
            let w = sample_cosine_hemisphere(vec2(r.next_f32(), r.next_f32()));
            let pdf = cosine_hemisphere_pdf(w.z);
            if pdf > 0.0 {
                // E[f/pdf] with f = pdf gives 1; use f = cos/pi -> integral of
                // the cosine lobe over the hemisphere, which is exactly 1.
                sum += (cosine_hemisphere_pdf(w.z) / pdf) as f64;
            }
        }
        assert!((sum / n as f64 - 1.0).abs() < 1e-6);
    }

    /// The cosine-weighted warp must actually be cosine weighted: estimate
    /// the projected solid angle of the hemisphere, which is exactly pi.
    #[test]
    fn cosine_hemisphere_estimates_projected_solid_angle() {
        let mut r = Pcg32::new(3, 3);
        let n = 500_000;
        let mut sum = 0.0f64;
        for _ in 0..n {
            let w = sample_cosine_hemisphere(vec2(r.next_f32(), r.next_f32()));
            sum += (w.z / cosine_hemisphere_pdf(w.z)) as f64;
        }
        let estimate = sum / n as f64;
        assert!(
            (estimate - PI as f64).abs() < 0.01,
            "got {estimate}, want pi"
        );
    }

    #[test]
    fn uniform_sphere_estimates_full_solid_angle() {
        let mut r = Pcg32::new(4, 4);
        let n = 400_000;
        let sum: f64 = (0..n)
            .map(|_| {
                let _w = sample_uniform_sphere(vec2(r.next_f32(), r.next_f32()));
                (1.0 / UNIFORM_SPHERE_PDF) as f64
            })
            .sum();
        assert!((sum / n as f64 - 4.0 * PI as f64).abs() < 1e-3);
    }

    #[test]
    fn triangle_barycentrics_are_valid_and_uniform() {
        let mut r = Pcg32::new(5, 5);
        let mut mean = Vec2::ZERO;
        let n = 200_000;
        for _ in 0..n {
            let b = sample_uniform_triangle(vec2(r.next_f32(), r.next_f32()));
            assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.y <= 1.0 + 1e-6);
            mean += b;
        }
        mean /= n as f32;
        // The centroid of the reference triangle is (1/3, 1/3).
        assert!((mean.x - 1.0 / 3.0).abs() < 0.003, "{}", mean.x);
        assert!((mean.y - 1.0 / 3.0).abs() < 0.003, "{}", mean.y);
    }

    #[test]
    fn cone_pdf_integrates_to_one() {
        let mut r = Pcg32::new(6, 6);
        let cos_max = 0.8;
        let n = 200_000;
        let sum: f64 = (0..n)
            .map(|_| {
                let w = sample_uniform_cone(vec2(r.next_f32(), r.next_f32()), cos_max);
                assert!(w.z >= cos_max - 1e-5);
                (uniform_cone_pdf(cos_max) / uniform_cone_pdf(cos_max)) as f64
            })
            .sum();
        assert!((sum / n as f64 - 1.0).abs() < 1e-9);
        // Solid angle of the cone.
        let expected = TAU * (1.0 - cos_max);
        assert!((1.0 / uniform_cone_pdf(cos_max) - expected).abs() < 1e-5);
    }

    #[test]
    fn equirect_mapping_round_trips() {
        let mut r = Pcg32::new(7, 7);
        for _ in 0..10_000 {
            let d = sample_uniform_sphere(vec2(r.next_f32(), r.next_f32()));
            let round = equirect_to_direction(direction_to_equirect(d));
            assert!((round - d).length() < 1e-4, "{d:?} -> {round:?}");
        }
    }

    #[test]
    fn mis_power_weights_sum_to_one() {
        for (a, b) in [(1.0f32, 3.0), (0.001, 900.0), (5.0, 5.0)] {
            assert!((mis_power(a, b) + mis_power(b, a) - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn mis_power_handles_delta_and_overflow() {
        assert_eq!(mis_power(f32::INFINITY, 4.0), 1.0);
        // Squaring 1e30 in f32 overflows; the f64 path must still be exact.
        assert!((mis_power(1e30, 1e30) - 0.5).abs() < 1e-6);
        assert_eq!(mis_power(0.0, 0.0), 0.0);
    }

    #[test]
    fn discrete_sampling_matches_its_pmf() {
        let weights = [1.0f32, 3.0, 0.0, 6.0];
        let mut counts = [0u32; 4];
        let n = 200_000;
        let mut r = Pcg32::new(8, 8);
        for _ in 0..n {
            let (i, pmf, remapped) = sample_discrete(&weights, r.next_f32()).unwrap();
            assert!((0.0..1.0).contains(&remapped));
            assert!((pmf - weights[i] / 10.0).abs() < 1e-6);
            counts[i] += 1;
        }
        for (i, &w) in weights.iter().enumerate() {
            let observed = counts[i] as f32 / n as f32;
            assert!((observed - w / 10.0).abs() < 0.01, "bucket {i}: {observed}");
        }
    }

    #[test]
    fn polygon_aperture_is_bounded_and_covers_all_blades() {
        let mut r = Pcg32::new(9, 9);
        let blades = 6;
        let mut wedge_hits = vec![0u32; blades as usize];
        for _ in 0..100_000 {
            let u = vec2(r.next_f32(), r.next_f32());
            let p = sample_polygon_aperture(u, blades, 0.0);
            assert!(p.length() <= 1.0 + 1e-6);
            let wedge = ((p.y.atan2(p.x).rem_euclid(TAU)) / TAU * blades as f32) as usize;
            wedge_hits[wedge.min(blades as usize - 1)] += 1;
        }
        assert!(wedge_hits.iter().all(|&c| c > 10_000), "{wedge_hits:?}");
    }
}
