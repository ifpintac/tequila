//! Energy-preserving Oren-Nayar diffuse reflection.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3 specifies "Oren-Nayar (reduces to Lambert
//! at roughness 0); use the energy-preserving formulation". The classical
//! qualitative Oren-Nayar model is *not* energy preserving — it loses a
//! substantial fraction of energy at high roughness and would fail the white
//! furnace test of `docs/05` §3.1 outright.
//!
//! The formulation implemented here is EON [Portsmouth, Hill, Conty Estevez &
//! Hutchinson 2023, *EON: A practical energy-preserving rough diffuse BRDF*],
//! which is the model OpenPBR itself specifies for its diffuse lobe. It adds a
//! multiple-scattering term to Fujii's single-scattering approximation, with a
//! closed-form directional albedo, so no lookup table is needed.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use tequila_core::math::{safe_acos, Vec2, Vec3};
use tequila_core::sampling::{cosine_hemisphere_pdf, sample_cosine_hemisphere};
use tequila_core::spectrum::Spectrum;
use std::f32::consts::{FRAC_1_PI, PI};

/// `1/2 - 2/(3*pi)`, the constant in Fujii's normalization.
const FUJII_A: f32 = 0.5 - 2.0 / (3.0 * PI);
/// `2/3 - 28/(15*pi)`, the constant in the average-albedo expression.
const FUJII_B: f32 = 2.0 / 3.0 - 28.0 / (15.0 * PI);

/// Oren-Nayar diffuse reflection, EON formulation.
#[derive(Clone, Copy, Debug)]
pub struct OrenNayarDiffuse {
    /// Diffuse albedo in the rendering space. Must be in `[0, 1]` per channel
    /// or the multiple-scattering series diverges.
    pub albedo: Spectrum,
    /// Surface roughness in `[0, 1]`. Zero reduces exactly to Lambert.
    pub roughness: f32,
}

impl OrenNayarDiffuse {
    pub fn new(albedo: Spectrum, roughness: f32) -> Self {
        OrenNayarDiffuse {
            albedo: albedo.clamp(0.0, 1.0),
            roughness: roughness.clamp(0.0, 1.0),
        }
    }

    /// Directional albedo of the *single-scattering* Fujii lobe, in closed
    /// form. This is what makes EON cheap: the compensation term needs
    /// `E(mu)` at both directions, and computing it numerically per shading
    /// point would be prohibitive.
    fn single_scatter_albedo(&self, mu: f32) -> f32 {
        let r = self.roughness;
        let a = 1.0 / (1.0 + FUJII_A * r);
        if r <= 0.0 {
            return a;
        }
        let b = r * a;
        // Guard the grazing limit: the `sin/mu` term is finite in the limit
        // (it behaves like 3*mu/2) but is numerically 0/0 as mu -> 0.
        let mu = mu.clamp(1e-3, 1.0);
        let si = (1.0 - mu * mu).max(0.0).sqrt();
        let g =
            si * (safe_acos(mu) - si * mu) + (2.0 / 3.0) * ((si / mu) * (1.0 - si * si * si) - si);
        (a + (b * FRAC_1_PI) * g).clamp(0.0, 1.0)
    }

    /// Cosine-weighted hemispherical average of the single-scattering albedo.
    #[inline]
    fn average_single_scatter_albedo(&self) -> f32 {
        (1.0 / (1.0 + FUJII_A * self.roughness)) * (1.0 + FUJII_B * self.roughness)
    }
}

impl Bsdf for OrenNayarDiffuse {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        let (mu_o, mu_i) = (wo.z, wi.z);
        if mu_o <= 0.0 || mu_i <= 0.0 {
            return Spectrum::ZERO;
        }
        let r = self.roughness;
        let a = 1.0 / (1.0 + FUJII_A * r);

        // Single scattering (Fujii's qualitative Oren-Nayar, normalized).
        // `s` is sin_i * sin_o * cos(delta_phi); the asymmetric division by
        // the *larger* cosine is what produces Oren-Nayar's characteristic
        // flat, retro-reflective look.
        let s = wi.dot(wo) - mu_i * mu_o;
        let s_over_t = if s > 0.0 { s / mu_i.max(mu_o) } else { s };
        let f_ss = self.albedo * (FRAC_1_PI * a * (1.0 + r * s_over_t));

        if r <= 0.0 {
            return f_ss;
        }

        // Multiple scattering: the energy the single-scattering lobe fails to
        // reflect, redistributed so that the total is exactly conserved for a
        // white albedo.
        let e_o = self.single_scatter_albedo(mu_o);
        let e_i = self.single_scatter_albedo(mu_i);
        let e_avg = self.average_single_scatter_albedo();

        // Geometric series over repeated inter-reflections at albedo rho.
        let denom = Spectrum::ONE - self.albedo * (1.0 - e_avg);
        let rho_ms = (self.albedo * self.albedo) * e_avg / denom;

        const EPS: f32 = 1e-7;
        let f_ms = rho_ms
            * (FRAC_1_PI * (1.0 - e_o).max(EPS) * (1.0 - e_i).max(EPS) / (1.0 - e_avg).max(EPS));

        f_ss + f_ms
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if wo.z <= 0.0 || wi.z <= 0.0 {
            return 0.0;
        }
        cosine_hemisphere_pdf(wi.z)
    }

    fn sample(&self, wo: Vec3, u: Vec2, _uc: f32) -> Option<BsdfSample> {
        if wo.z <= 0.0 {
            return None;
        }
        let wi = sample_cosine_hemisphere(u);
        let pdf = cosine_hemisphere_pdf(wi.z);
        if pdf <= 0.0 {
            return None;
        }
        Some(BsdfSample {
            wi,
            f: self.eval(wo, wi),
            pdf,
            is_delta: false,
            is_transmission: false,
            eta: 1.0,
            flags: BsdfFlags::REFLECTION | BsdfFlags::DIFFUSE,
        })
    }

    fn albedo(&self, _wo: Vec3) -> Spectrum {
        self.albedo
    }

    fn flags(&self) -> BsdfFlags {
        BsdfFlags::REFLECTION | BsdfFlags::DIFFUSE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_core::rng::Pcg32;

    fn wo_at(mu: f32) -> Vec3 {
        vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu)
    }

    /// White furnace: a perfectly white, energy-conserving BRDF under uniform
    /// unit radiance must return exactly 1 at every angle and roughness
    /// (`docs/05` §3.1). This is the test the classical Oren-Nayar model fails.
    #[test]
    fn white_furnace_passes_at_every_roughness() {
        let mut rng = Pcg32::new(11, 11);
        for roughness in [0.0f32, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0] {
            let lobe = OrenNayarDiffuse::new(Spectrum::ONE, roughness);
            for mu in [0.05f32, 0.2, 0.5, 0.8, 1.0] {
                let wo = wo_at(mu);
                let n = 200_000;
                let mut sum = 0.0f64;
                for _ in 0..n {
                    let u = vec2(rng.next_f32(), rng.next_f32());
                    if let Some(s) = lobe.sample(wo, u, rng.next_f32()) {
                        sum += s.throughput_weight().g as f64;
                    }
                }
                let result = sum / n as f64;
                assert!(
                    (result - 1.0).abs() < 0.005,
                    "roughness {roughness}, mu {mu}: furnace = {result}"
                );
            }
        }
    }

    #[test]
    fn reduces_to_lambert_at_zero_roughness() {
        let lobe = OrenNayarDiffuse::new(Spectrum::splat(0.8), 0.0);
        let wo = wo_at(0.6);
        let wi = wo_at(0.4);
        let f = lobe.eval(wo, wi);
        assert!((f.g - 0.8 * FRAC_1_PI).abs() < 1e-6, "{}", f.g);
    }

    /// Reciprocity: `f(wo, wi) == f(wi, wo)` for a non-transmissive lobe
    /// (`docs/05` §3.3).
    #[test]
    fn is_reciprocal() {
        let mut rng = Pcg32::new(12, 12);
        for roughness in [0.0f32, 0.3, 1.0] {
            let lobe = OrenNayarDiffuse::new(Spectrum::splat(0.6), roughness);
            for _ in 0..2000 {
                let wo = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
                let wi = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
                let a = lobe.eval(wo, wi).g;
                let b = lobe.eval(wi, wo).g;
                assert!((a - b).abs() < 1e-5, "roughness {roughness}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn never_returns_negative_or_non_finite_values() {
        let mut rng = Pcg32::new(13, 13);
        for roughness in [0.0f32, 0.5, 1.0] {
            let lobe = OrenNayarDiffuse::new(Spectrum::splat(0.9), roughness);
            for _ in 0..20_000 {
                let wo = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
                let wi = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
                let f = lobe.eval(wo, wi);
                assert!(f.is_finite() && f.r >= 0.0, "{f:?}");
            }
        }
    }

    #[test]
    fn rejects_directions_below_the_surface() {
        let lobe = OrenNayarDiffuse::new(Spectrum::ONE, 0.5);
        assert!(lobe.eval(wo_at(0.5), vec3(0.0, 0.0, -1.0)).is_black());
        assert_eq!(lobe.pdf(wo_at(0.5), vec3(0.0, 0.0, -1.0)), 0.0);
        assert!(lobe
            .sample(vec3(0.0, 0.0, -1.0), vec2(0.5, 0.5), 0.5)
            .is_none());
    }

    /// A smooth Lambertian surface reflects exactly its albedo. A *rough*
    /// one reflects less: light bounces between microfacets, and every extra
    /// bounce is another chance to be absorbed. Asserting `reflectance == rho`
    /// at high roughness would be asserting a physical error.
    #[test]
    fn reflectance_tracks_albedo_and_falls_with_roughness() {
        let mut rng = Pcg32::new(14, 14);
        let measure = |lobe: &OrenNayarDiffuse, rng: &mut Pcg32| {
            let wo = wo_at(0.7);
            let n = 200_000;
            let mut sum = 0.0f64;
            for _ in 0..n {
                let u = vec2(rng.next_f32(), rng.next_f32());
                if let Some(s) = lobe.sample(wo, u, 0.0) {
                    sum += s.throughput_weight().g as f64;
                }
            }
            sum / n as f64
        };

        for rho in [0.2f32, 0.5, 0.9] {
            // Smooth: exactly the albedo.
            let smooth = OrenNayarDiffuse::new(Spectrum::splat(rho), 0.0);
            let r_smooth = measure(&smooth, &mut rng);
            assert!(
                (r_smooth - rho as f64).abs() < 0.005,
                "albedo {rho} at roughness 0 reflected {r_smooth}"
            );

            // Rough: less, but not by much, and never more.
            let mut previous = r_smooth;
            for roughness in [0.3f32, 0.6, 1.0] {
                let lobe = OrenNayarDiffuse::new(Spectrum::splat(rho), roughness);
                let r = measure(&lobe, &mut rng);
                assert!(
                    r <= previous + 0.005,
                    "albedo {rho} roughness {roughness}: reflectance rose to {r}"
                );
                assert!(
                    r > rho as f64 * 0.75,
                    "albedo {rho} roughness {roughness}: lost too much, {r}"
                );
                previous = r;
            }
        }
    }
}
