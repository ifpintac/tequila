//! GGX dielectric specular reflection with energy compensation.
//!
//! This is the OpenPBR `specular` lobe: the glossy reflection off a dielectric
//! interface (plastic, paint, varnish, glass seen from outside). Transmission
//! through that interface is a separate lobe, added with the medium machinery
//! at M2.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use crate::energy::{self, multiple_scattering_value};
use crate::fresnel;
use crate::microfacet::Ggx;
use crate::thin_film::Film;
use tequila_core::math::{shading, Vec2, Vec3};
use tequila_core::sampling::{cosine_hemisphere_pdf, sample_cosine_hemisphere};
use tequila_core::spectrum::Spectrum;

/// Rough dielectric reflection.
#[derive(Clone, Copy, Debug)]
pub struct DielectricSpecular {
    pub distribution: Ggx,
    /// Index of refraction of the substrate relative to the medium outside.
    pub ior: f32,
    /// Tint on the reflection. Physically this is white; it exists because the
    /// Plastic material exposes a Specular Color control.
    pub tint: Spectrum,
    /// Overall lobe strength, the OpenPBR `specular_weight`.
    pub weight: f32,
    /// An interference film over this interface, §2.2's thin film.
    ///
    /// `None` rather than a zero-weight film, so that the overwhelmingly
    /// common uncoated surface keeps the scalar Fresnel path and never pays
    /// for the complex arithmetic.
    pub film: Option<Film>,
}

impl DielectricSpecular {
    pub fn new(roughness: f32, ior: f32) -> Self {
        DielectricSpecular {
            distribution: Ggx::from_roughness(roughness, 0.0),
            ior: ior.max(1.0),
            tint: Spectrum::ONE,
            weight: 1.0,
            film: None,
        }
    }

    /// Put an interference film over this interface.
    pub fn with_film(mut self, film: Option<Film>) -> Self {
        self.film = film.filter(|f| !f.is_negligible());
        self
    }

    /// Reflectance at a microfacet, coated or bare.
    #[inline]
    fn reflectance(&self, cos_theta: f32) -> Spectrum {
        match &self.film {
            Some(film) => film.over_dielectric(cos_theta, self.ior),
            None => Spectrum::splat(fresnel::dielectric(cos_theta, self.ior)),
        }
    }

    pub fn with_tint(mut self, tint: Spectrum) -> Self {
        self.tint = tint;
        self
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight.clamp(0.0, 1.0);
        self
    }

    fn compensation_probability(&self, wo: Vec3) -> f32 {
        let alpha = self.distribution.average_alpha();
        let e_o = energy::table().white_directional(shading::abs_cos_theta(wo), alpha);
        (1.0 - e_o).clamp(0.0, 0.5)
    }

    fn compensation_scale(&self, e_avg: f32) -> f32 {
        let f_avg = fresnel::dielectric_average(self.ior);
        energy::multiple_scattering_scale(Spectrum::splat(f_avg), e_avg).g
    }

    /// Fraction of incident energy this lobe reflects for a given outgoing
    /// direction. The layered materials need it to know how much energy is
    /// left for the lobes underneath (`docs/02` §2.1 albedo scaling).
    ///
    /// This uses the split-sum table rather than `F(cos_theta_o)`: the
    /// Fresnel term that matters is the one at the *microfacet*, and on a
    /// rough surface at oblique incidence the two diverge sharply. See the
    /// module documentation of [`crate::energy`].
    pub fn directional_albedo(&self, wo: Vec3) -> f32 {
        let mu = shading::abs_cos_theta(wo);
        let alpha = self.distribution.average_alpha();
        let table = energy::table();
        let single = table.dielectric_directional(mu, alpha, self.ior);
        let e_o = table.white_directional(mu, alpha);
        let e_avg = table.white_average(alpha);
        (self.weight * (single + self.compensation_scale(e_avg) * (1.0 - e_o))).clamp(0.0, 1.0)
    }

    /// Cosine-weighted hemispherical average of [`Self::directional_albedo`].
    ///
    /// A layered material needs this for the light trapped between its
    /// substrate and the underside of this interface.
    pub fn hemispherical_albedo(&self) -> f32 {
        let alpha = self.distribution.average_alpha();
        let table = energy::table();
        let e_avg = table.white_average(alpha);
        (self.weight
            * (table.dielectric_hemispherical(alpha, self.ior)
                + self.compensation_scale(e_avg) * (1.0 - e_avg)))
            .clamp(0.0, 1.0)
    }
}

impl Bsdf for DielectricSpecular {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        if wo.z <= 0.0 || wi.z <= 0.0 || self.distribution.is_smooth() || self.weight <= 0.0 {
            return Spectrum::ZERO;
        }
        let Some(wm) = (wo + wi).try_normalize() else {
            return Spectrum::ZERO;
        };
        let wm = if wm.z < 0.0 { -wm } else { wm };

        let d = self.distribution.d(wm);
        let g2 = self.distribution.g2(wo, wi);
        let single = self.reflectance(wo.dot(wm).abs()) * (d * g2 / (4.0 * wo.z * wi.z));

        let alpha = self.distribution.average_alpha();
        let table = energy::table();
        let e_o = table.white_directional(wo.z, alpha);
        let e_i = table.white_directional(wi.z, alpha);
        let e_avg = table.white_average(alpha);
        let multi = self.compensation_scale(e_avg) * multiple_scattering_value(e_o, e_i, e_avg);

        self.tint * ((single + Spectrum::splat(multi)) * self.weight)
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if wo.z <= 0.0 || wi.z <= 0.0 || self.distribution.is_smooth() || self.weight <= 0.0 {
            return 0.0;
        }
        let p_comp = self.compensation_probability(wo);
        (1.0 - p_comp) * self.distribution.reflection_pdf(wo, wi)
            + p_comp * cosine_hemisphere_pdf(wi.z)
    }

    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample> {
        if wo.z <= 0.0 || self.weight <= 0.0 {
            return None;
        }

        if self.distribution.is_smooth() {
            let wi = Vec3 {
                x: -wo.x,
                y: -wo.y,
                z: wo.z,
            };
            let f = self.reflectance(wo.z) * self.weight;
            return Some(BsdfSample {
                wi,
                f: self.tint * f * (1.0 / wi.z.abs()),
                pdf: 1.0,
                is_delta: true,
                is_transmission: false,
                eta: 1.0,
                flags: BsdfFlags::REFLECTION | BsdfFlags::DELTA,
            });
        }

        let p_comp = self.compensation_probability(wo);
        let wi = if uc < p_comp {
            sample_cosine_hemisphere(u)
        } else {
            let wm = self.distribution.sample_visible_normal(wo, u);
            -wo + wm * (2.0 * wo.dot(wm))
        };
        if wi.z <= 0.0 {
            return None;
        }

        let pdf = self.pdf(wo, wi);
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
            flags: BsdfFlags::REFLECTION | BsdfFlags::GLOSSY,
        })
    }

    fn albedo(&self, wo: Vec3) -> Spectrum {
        self.tint * self.directional_albedo(wo)
    }

    fn flags(&self) -> BsdfFlags {
        if self.distribution.is_smooth() {
            BsdfFlags::REFLECTION | BsdfFlags::DELTA
        } else {
            BsdfFlags::REFLECTION | BsdfFlags::GLOSSY
        }
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

    fn measure_reflectance(lobe: &DielectricSpecular, wo: Vec3, rng: &mut Pcg32) -> f32 {
        let n = 300_000;
        let mut sum = 0.0f64;
        for _ in 0..n {
            let u = vec2(rng.next_f32(), rng.next_f32());
            if let Some(s) = lobe.sample(wo, u, rng.next_f32()) {
                sum += s.throughput_weight().g as f64;
            }
        }
        (sum / n as f64) as f32
    }

    /// A dielectric interface reflects only its Fresnel fraction, so the
    /// furnace value is not 1. The energy-conservation statement that *does*
    /// hold is that total reflectance must never exceed 1, and — this is the
    /// point of the compensation term — must be almost independent of
    /// Reflectance must stay in range at every angle and roughness, and must
    /// *fall* as the surface roughens.
    ///
    /// This is the opposite of the conductor case and it is easy to mistake
    /// for a bug. On a rough dielectric, light that a microfacet shadows goes
    /// on to strike another microfacet, where `F` is small and it mostly
    /// transmits. So roughening a dielectric genuinely moves energy out of the
    /// reflection lobe — the Kulla-Conty term restores only the small fraction
    /// that reflects again, scaled by `F_avg^2`. The energy is not lost from
    /// the *material*: `SpecularOverDiffuse` hands it to the substrate.
    #[test]
    fn reflectance_falls_as_the_surface_roughens() {
        let mut rng = Pcg32::new(31, 31);
        for mu in [0.2f32, 0.6, 1.0] {
            let wo = wo_at(mu);
            let mut previous = 1.01;
            for roughness in [0.0f32, 0.2, 0.5, 0.8, 1.0] {
                let lobe = DielectricSpecular::new(roughness, 1.5);
                let r = measure_reflectance(&lobe, wo, &mut rng);
                assert!(
                    (0.0..=1.0).contains(&r),
                    "roughness {roughness} mu {mu}: reflectance {r}"
                );
                assert!(
                    r < previous + 0.01,
                    "roughness {roughness} mu {mu}: reflectance rose to {r}"
                );
                previous = r;
            }
        }
    }

    /// A smooth interface must reproduce the exact Fresnel equations. This is
    /// the anchor for the whole lobe: whatever the rough case does, the smooth
    /// limit has a closed-form answer and must match it.
    #[test]
    fn smooth_limit_matches_exact_fresnel() {
        let mut rng = Pcg32::new(32, 32);
        for ior in [1.33f32, 1.5, 1.8] {
            for mu in [0.15f32, 0.4, 0.7, 1.0] {
                let lobe = DielectricSpecular::new(0.02, ior);
                let measured = measure_reflectance(&lobe, wo_at(mu), &mut rng);
                let exact = fresnel::dielectric(mu, ior);
                assert!(
                    (measured - exact).abs() < 0.01,
                    "ior {ior} mu {mu}: measured {measured}, exact {exact}"
                );
            }
        }
    }

    #[test]
    fn directional_albedo_agrees_with_a_measurement() {
        let mut rng = Pcg32::new(34, 34);
        for roughness in [0.1f32, 0.5, 0.9] {
            let lobe = DielectricSpecular::new(roughness, 1.5);
            for mu in [0.3f32, 0.7, 1.0] {
                let wo = wo_at(mu);
                let measured = measure_reflectance(&lobe, wo, &mut rng);
                let predicted = lobe.directional_albedo(wo);
                assert!(
                    (measured - predicted).abs() < 0.02,
                    "roughness {roughness} mu {mu}: measured {measured}, predicted {predicted}"
                );
            }
        }
    }

    #[test]
    fn smooth_dielectric_is_a_delta_lobe_with_the_right_fresnel() {
        let lobe = DielectricSpecular::new(0.0, 1.5);
        let wo = vec3(0.0, 0.0, 1.0);
        let s = lobe.sample(wo, vec2(0.5, 0.5), 0.5).unwrap();
        assert!(s.is_delta);
        // At normal incidence a 1.5-IOR interface reflects 4%.
        assert!((s.throughput_weight().g - 0.04).abs() < 1e-3);
    }
}
