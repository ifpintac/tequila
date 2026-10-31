//! GGX / Trowbridge-Reitz microfacet distribution with height-correlated
//! Smith masking-shadowing and visible-normal sampling.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3 specifies GGX with height-correlated
//! Smith and VNDF sampling [Heitz 2018] for every glossy lobe. VNDF sampling
//! is not an optimization: sampling the full NDF generates microfacets that
//! are backfacing or shadowed, which are then rejected, and the resulting
//! variance at high roughness is severe.

use tequila_core::math::{shading, sqr, vec3, Vec2, Vec3};

/// An anisotropic GGX distribution.
///
/// `alpha` is the *slope-space* roughness, related to the artist-facing
/// roughness by `alpha = roughness^2` (the perceptual mapping of `docs/02`
/// §2.2). The conversion belongs to the material, not here.
#[derive(Clone, Copy, Debug)]
pub struct Ggx {
    pub alpha_x: f32,
    pub alpha_y: f32,
}

/// Below this alpha the lobe is treated as a perfect mirror. Chosen so that
/// the reflected cone is narrower than a pixel at any plausible resolution,
/// while staying well above the point where the NDF overflows to infinity.
pub const SMOOTH_ALPHA_THRESHOLD: f32 = 1e-3;

impl Ggx {
    #[inline]
    pub fn isotropic(alpha: f32) -> Self {
        Ggx {
            alpha_x: alpha,
            alpha_y: alpha,
        }
    }

    /// Build from artist-facing roughness and anisotropy.
    ///
    /// `anisotropy` in `[0, 1)` stretches the highlight along the tangent
    /// direction while preserving the overall highlight area, which is what
    /// keeps a material's apparent brightness stable as the user drags the
    /// anisotropy slider.
    pub fn from_roughness(roughness: f32, anisotropy: f32) -> Self {
        let alpha = sqr(roughness.clamp(0.0, 1.0));
        let aniso = anisotropy.clamp(0.0, 0.99);
        if aniso <= 0.0 {
            return Ggx::isotropic(alpha);
        }
        let ratio = (1.0 - aniso * aniso).sqrt();
        Ggx {
            alpha_x: (alpha / ratio.sqrt()).min(1.0),
            alpha_y: (alpha * ratio.sqrt()).max(0.0),
        }
    }

    /// True when the distribution is narrow enough to treat as a delta lobe.
    #[inline]
    pub fn is_smooth(&self) -> bool {
        self.alpha_x < SMOOTH_ALPHA_THRESHOLD && self.alpha_y < SMOOTH_ALPHA_THRESHOLD
    }

    /// Impose a roughness floor, for path regularization (`docs/01` §3.5).
    #[inline]
    pub fn regularized(&self, min_alpha: f32) -> Self {
        Ggx {
            alpha_x: self.alpha_x.max(min_alpha),
            alpha_y: self.alpha_y.max(min_alpha),
        }
    }

    /// The artist-facing roughness along the wider of the two directions:
    /// how blurred the widest part of a reflection off it is.
    #[inline]
    pub fn roughness(&self) -> f32 {
        self.alpha_x.max(self.alpha_y).max(0.0).sqrt()
    }

    /// Effective isotropic roughness, for LUT lookups and heuristics.
    #[inline]
    pub fn average_alpha(&self) -> f32 {
        (self.alpha_x * self.alpha_y).sqrt()
    }

    /// The normal distribution function `D(wm)`.
    #[inline]
    pub fn d(&self, wm: Vec3) -> f32 {
        let cos2 = shading::cos2_theta(wm);
        if cos2 <= 0.0 {
            return 0.0;
        }
        let denom = sqr(wm.x / self.alpha_x) + sqr(wm.y / self.alpha_y) + sqr(wm.z);
        if denom <= 0.0 {
            return 0.0;
        }
        1.0 / (std::f32::consts::PI * self.alpha_x * self.alpha_y * sqr(denom))
    }

    /// Smith's `Lambda`, the ratio of masked to visible microfacet area.
    #[inline]
    pub fn lambda(&self, w: Vec3) -> f32 {
        let cos2 = shading::cos2_theta(w);
        if cos2 <= 0.0 {
            return 0.0;
        }
        let alpha2_tan2 = (sqr(self.alpha_x * w.x) + sqr(self.alpha_y * w.y)) / cos2;
        if alpha2_tan2 <= 0.0 {
            return 0.0;
        }
        0.5 * ((1.0 + alpha2_tan2).sqrt() - 1.0)
    }

    /// Single-direction masking `G1`.
    #[inline]
    pub fn g1(&self, w: Vec3) -> f32 {
        1.0 / (1.0 + self.lambda(w))
    }

    /// Height-correlated Smith masking-shadowing `G2`.
    ///
    /// The height-correlated form is used rather than the separable
    /// `G1(wo) * G1(wi)` because the separable form loses noticeable energy at
    /// grazing angles [Heitz 2014], which reads as a dark rim on rough
    /// surfaces.
    #[inline]
    pub fn g2(&self, wo: Vec3, wi: Vec3) -> f32 {
        1.0 / (1.0 + self.lambda(wo) + self.lambda(wi))
    }

    /// Distribution of visible normals, `D_v(wm)`, in solid-angle measure
    /// about the microfacet normal.
    ///
    /// The `wo . wm` factor is **clamped to zero**, not made absolute: a
    /// microfacet whose normal faces away from `wo` is not visible from `wo`
    /// and carries no probability. Using the absolute value instead makes the
    /// distribution integrate to more than 1 — by 12% at `alpha = 0.9` — which
    /// would show up as glossy surfaces that are quietly too dark under MIS,
    /// since the inflated PDF divides the contribution down.
    #[inline]
    pub fn d_visible(&self, wo: Vec3, wm: Vec3) -> f32 {
        let cos_o = shading::abs_cos_theta(wo);
        if cos_o <= 0.0 {
            return 0.0;
        }
        // `wm` is expected on the same side as `wo`, which is how both
        // `sample_visible_normal` and `reflection_pdf` orient it.
        let cos_d = wo.dot(wm);
        if cos_d <= 0.0 {
            return 0.0;
        }
        self.g1(wo) * cos_d * self.d(wm) / cos_o
    }

    /// Sample a visible microfacet normal [Heitz 2018, *Sampling the GGX
    /// Distribution of Visible Normals*]. `wo` must be in shading space; it is
    /// flipped into the upper hemisphere internally, and the returned normal
    /// is flipped back to match.
    pub fn sample_visible_normal(&self, wo: Vec3, u: Vec2) -> Vec3 {
        let flip = wo.z < 0.0;
        let wo_up = if flip { -wo } else { wo };

        // 1. Warp to the hemisphere configuration (alpha = 1).
        let vh = vec3(self.alpha_x * wo_up.x, self.alpha_y * wo_up.y, wo_up.z).normalize();

        // 2. Orthonormal basis about vh.
        let len2 = vh.x * vh.x + vh.y * vh.y;
        let t1 = if len2 > 0.0 {
            vec3(-vh.y, vh.x, 0.0) / len2.sqrt()
        } else {
            Vec3::X
        };
        let t2 = vh.cross(t1);

        // 3. Uniform point on the projected disk, then the affine squash that
        //    accounts for the projection of the hemisphere onto the disk.
        let r = u.x.max(0.0).sqrt();
        let phi = std::f32::consts::TAU * u.y;
        let p1 = r * phi.cos();
        let mut p2 = r * phi.sin();
        let s = 0.5 * (1.0 + vh.z);
        p2 = (1.0 - s) * (1.0 - p1 * p1).max(0.0).sqrt() + s * p2;

        // 4. Project onto the hemisphere and unwarp.
        let nh = t1 * p1 + t2 * p2 + vh * (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
        let wm = vec3(self.alpha_x * nh.x, self.alpha_y * nh.y, nh.z.max(1e-6)).normalize();

        if flip {
            -wm
        } else {
            wm
        }
    }

    /// PDF of a reflection direction `wi` produced by sampling a visible
    /// normal and reflecting `wo` about it. The `1 / (4 |wo . wm|)` factor is
    /// the Jacobian of the half-vector-to-direction reflection mapping.
    #[inline]
    pub fn reflection_pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        let wm = wo + wi;
        let Some(wm) = wm.try_normalize() else {
            return 0.0;
        };
        // Orient the half vector to `wo`'s side, not unconditionally upwards:
        // `d_visible` is defined relative to the viewing hemisphere.
        let wm = if wm.dot(wo) < 0.0 { -wm } else { wm };
        let dot = wo.dot(wm);
        if dot <= 0.0 {
            return 0.0;
        }
        self.d_visible(wo, wm) / (4.0 * dot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec2;
    use tequila_core::rng::Pcg32;

    fn spherical(theta: f32, phi: f32) -> Vec3 {
        vec3(
            theta.sin() * phi.cos(),
            theta.sin() * phi.sin(),
            theta.cos(),
        )
    }

    /// Numerically integrate `f` over the upper hemisphere by tensor-product
    /// midpoint quadrature in `(theta, phi)`.
    ///
    /// Deterministic quadrature rather than Monte Carlo: the NDF at low
    /// roughness is a spike a few hundredths of a radian wide, and uniform
    /// sampling of a spike gives an estimate with several percent of noise —
    /// enough to mask exactly the normalization errors this is meant to catch.
    fn integrate_hemisphere(f: impl Fn(Vec3) -> f32) -> f64 {
        const THETA_STEPS: usize = 2000;
        const PHI_STEPS: usize = 128;
        let d_theta = std::f32::consts::FRAC_PI_2 / THETA_STEPS as f32;
        let d_phi = std::f32::consts::TAU / PHI_STEPS as f32;
        let mut sum = 0.0f64;
        for ti in 0..THETA_STEPS {
            let theta = (ti as f32 + 0.5) * d_theta;
            let sin_theta = theta.sin();
            for pi in 0..PHI_STEPS {
                let phi = (pi as f32 + 0.5) * d_phi;
                sum += (f(spherical(theta, phi)) * sin_theta) as f64;
            }
        }
        sum * (d_theta * d_phi) as f64
    }

    /// The NDF must integrate to 1 against the projected solid angle:
    /// `integral D(wm) cos(theta_m) dwm = 1`. A missing normalization constant
    /// scales every glossy surface's brightness, and it is invisible in
    /// isolation because it looks like a roughness change.
    #[test]
    fn ndf_is_normalized() {
        for alpha in [0.05f32, 0.2, 0.5, 1.0] {
            let ggx = Ggx::isotropic(alpha);
            let integral = integrate_hemisphere(|w| ggx.d(w) * w.z);
            assert!(
                (integral - 1.0).abs() < 5e-3,
                "alpha {alpha}: integral = {integral}"
            );
        }
    }

    /// The anisotropic NDF must be normalized too — the `1 / (pi ax ay)`
    /// factor is easy to get wrong when the isotropic case still looks right.
    #[test]
    fn anisotropic_ndf_is_normalized() {
        for (ax, ay) in [(0.4f32, 0.1f32), (0.8, 0.2), (0.15, 0.6)] {
            let ggx = Ggx {
                alpha_x: ax,
                alpha_y: ay,
            };
            let integral = integrate_hemisphere(|w| ggx.d(w) * w.z);
            assert!(
                (integral - 1.0).abs() < 5e-3,
                "alpha ({ax}, {ay}): integral = {integral}"
            );
        }
    }

    /// The distribution of visible normals must be a normalized probability
    /// density over the hemisphere. Integrating to more than 1 divides every
    /// glossy contribution down under MIS; less than 1 brightens it.
    #[test]
    fn vndf_is_a_normalized_density() {
        for alpha in [0.1f32, 0.4, 0.9] {
            for theta_o in [0.1f32, 0.7, 1.3] {
                let ggx = Ggx::isotropic(alpha);
                let wo = spherical(theta_o, 0.3);
                let integral = integrate_hemisphere(|w| ggx.d_visible(wo, w));
                assert!(
                    (integral - 1.0).abs() < 5e-3,
                    "alpha {alpha}, theta_o {theta_o}: VNDF integral = {integral}"
                );
            }
        }
    }

    /// `sample_visible_normal` must produce exactly the density `d_visible`
    /// describes. Binning the samples and comparing against the integral of
    /// the density over each bin is the χ² test of `docs/05` §3.2 in
    /// miniature, restricted to the one distribution everything else is built
    /// on.
    #[test]
    fn vndf_sampling_matches_its_density() {
        const BINS: usize = 16;
        for alpha in [0.15f32, 0.5, 0.95] {
            let ggx = Ggx::isotropic(alpha);
            let wo = spherical(0.6, 0.0);

            let mut observed = [0.0f64; BINS];
            let n = 400_000;
            let mut rng = Pcg32::new(61, alpha.to_bits() as u64);
            for _ in 0..n {
                let wm = ggx.sample_visible_normal(wo, vec2(rng.next_f32(), rng.next_f32()));
                // Bin by cos(theta_m), where the density varies most.
                let bin = ((1.0 - wm.z) * BINS as f32) as usize;
                observed[bin.min(BINS - 1)] += 1.0;
            }

            // Expected mass per bin, by quadrature over the same partition.
            let mut expected = [0.0f64; BINS];
            const STEPS: usize = 4000;
            const PHI_STEPS: usize = 256;
            let d_theta = std::f32::consts::FRAC_PI_2 / STEPS as f32;
            let d_phi = std::f32::consts::TAU / PHI_STEPS as f32;
            for ti in 0..STEPS {
                let theta = (ti as f32 + 0.5) * d_theta;
                let bin = (((1.0 - theta.cos()) * BINS as f32) as usize).min(BINS - 1);
                for pi in 0..PHI_STEPS {
                    let phi = (pi as f32 + 0.5) * d_phi;
                    let w = spherical(theta, phi);
                    expected[bin] +=
                        (ggx.d_visible(wo, w) * theta.sin()) as f64 * (d_theta * d_phi) as f64;
                }
            }

            for b in 0..BINS {
                let o = observed[b] / n as f64;
                let e = expected[b];
                if e < 1e-4 && o < 1e-4 {
                    continue;
                }
                assert!(
                    (o - e).abs() < 0.005 + 0.05 * e,
                    "alpha {alpha} bin {b}: sampled {o:.5}, density says {e:.5}"
                );
            }
        }
    }

    #[test]
    fn sampled_normals_are_in_the_upper_hemisphere_and_visible() {
        let mut rng = Pcg32::new(62, 62);
        for alpha in [0.01f32, 0.3, 1.0] {
            let ggx = Ggx::isotropic(alpha);
            for _ in 0..10_000 {
                let wo = spherical(rng.next_f32() * 1.5, rng.next_f32() * 6.0);
                let wm = ggx.sample_visible_normal(wo, vec2(rng.next_f32(), rng.next_f32()));
                assert!(wm.z > 0.0, "backfacing microfacet at alpha {alpha}");
                assert!((wm.length() - 1.0).abs() < 1e-4);
                assert!(wo.dot(wm) > -1e-4, "invisible microfacet sampled");
            }
        }
    }

    #[test]
    fn masking_is_bounded_and_ordered() {
        let mut rng = Pcg32::new(63, 63);
        for alpha in [0.05f32, 0.5, 1.0] {
            let ggx = Ggx::isotropic(alpha);
            for _ in 0..2000 {
                let wo = spherical(rng.next_f32() * 1.55, rng.next_f32() * 6.0);
                let wi = spherical(rng.next_f32() * 1.55, rng.next_f32() * 6.0);
                let (g1, g2) = (ggx.g1(wo), ggx.g2(wo, wi));
                assert!((0.0..=1.0).contains(&g1));
                assert!((0.0..=1.0).contains(&g2));
                // Two-direction masking can never exceed one-direction.
                assert!(g2 <= g1 + 1e-5);
            }
        }
    }

    /// A microfacet whose normal faces away from the viewer contributes no
    /// probability. Taking the absolute value of `wo . wm` instead of clamping
    /// inflates the VNDF by 12% at high roughness.
    #[test]
    fn backfacing_microfacets_carry_no_density() {
        let ggx = Ggx::isotropic(0.9);
        let wo = spherical(1.2, 0.0);
        let wm = vec3(-wo.x, -wo.y, 0.05).normalize();
        assert!(wm.z > 0.0, "test normal must be in the upper hemisphere");
        assert!(wm.dot(wo) < 0.0, "test normal must face away from wo");
        assert_eq!(ggx.d_visible(wo, wm), 0.0);
    }

    #[test]
    fn smooth_surfaces_are_flagged() {
        assert!(Ggx::isotropic(0.0).is_smooth());
        assert!(!Ggx::isotropic(0.01).is_smooth());
        assert!(Ggx::from_roughness(0.0, 0.0).is_smooth());
    }

    #[test]
    fn anisotropy_stretches_without_changing_the_average() {
        let iso = Ggx::from_roughness(0.4, 0.0);
        let aniso = Ggx::from_roughness(0.4, 0.8);
        assert!(aniso.alpha_x > aniso.alpha_y);
        assert!(
            (aniso.average_alpha() - iso.average_alpha()).abs() < 1e-3,
            "anisotropy changed the effective roughness"
        );
    }

    #[test]
    fn regularization_only_raises_roughness() {
        let ggx = Ggx {
            alpha_x: 0.02,
            alpha_y: 0.5,
        };
        let r = ggx.regularized(0.09);
        assert_eq!(r.alpha_x, 0.09);
        assert_eq!(r.alpha_y, 0.5);
    }
}
