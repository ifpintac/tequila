//! GGX conductor (metal) reflection with Kulla-Conty energy compensation.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use crate::energy::{self, multiple_scattering_value};
use crate::fresnel::{self, ComplexIor};
use crate::thin_film::Film;
use crate::microfacet::Ggx;
use tequila_core::math::{shading, Vec2, Vec3};
use tequila_core::sampling::{cosine_hemisphere_pdf, sample_cosine_hemisphere};
use tequila_core::spectrum::Spectrum;

/// A rough metal surface: GGX microfacets with conductor Fresnel.
///
/// Two lobes, not one: the single-scattering microfacet term plus the
/// multiple-scattering compensation term of `docs/02` §2.3. `pdf` returns the
/// weighted sum of both, which is what MIS requires.
#[derive(Clone, Copy, Debug)]
pub struct Conductor {
    pub distribution: Ggx,
    pub ior: ComplexIor,
    /// §2.2's `base_color` for a metallic base: a tint on the measured
    /// reflectance.
    ///
    /// §3.1 gives the Metal preset both a Type (a measured n/k spectrum) and a
    /// Color, and they have to combine rather than compete. White here is the
    /// bare measured metal; anything else is the anodized or coated version of
    /// it. Multiplying keeps the metal's spectral character — the reason to
    /// ship measured data at all — while letting the colour control mean
    /// something.
    pub tint: Spectrum,
    /// An interference film over this interface, §2.2's thin film.
    ///
    /// `None` rather than a zero-weight film, so that the overwhelmingly
    /// common uncoated surface keeps the scalar Fresnel path and never pays
    /// for the complex arithmetic.
    pub film: Option<Film>,
}

impl Conductor {
    pub fn with_tint(mut self, tint: Spectrum) -> Self {
        self.tint = tint;
        self
    }

    pub fn new(distribution: Ggx, ior: ComplexIor) -> Self {
        Conductor {
            distribution,
            ior,
            tint: Spectrum::ONE,
            film: None,
        }
    }

    /// Put an interference film over this metal: anodised titanium, the heat
    /// colours on steel, oil on chrome.
    pub fn with_film(mut self, film: Option<Film>) -> Self {
        self.film = film.filter(|f| !f.is_negligible());
        self
    }

    /// Reflectance at a microfacet, coated or bare.
    #[inline]
    fn reflectance(&self, cos_theta: f32) -> Spectrum {
        match &self.film {
            Some(film) => film.over_conductor(cos_theta, self.ior),
            None => fresnel::conductor(cos_theta, self.ior.n, self.ior.k),
        }
    }

    /// Build from the artist-facing controls the Metal material exposes.
    pub fn from_artist_parameters(
        reflectivity: Spectrum,
        edge_tint: Spectrum,
        roughness: f32,
        anisotropy: f32,
    ) -> Self {
        Conductor {
            distribution: Ggx::from_roughness(roughness, anisotropy),
            ior: ComplexIor::from_reflectivity_and_edge_tint(reflectivity, edge_tint),
            // The colour is already in the derived `(n, k)` here, so tinting
            // again would apply it twice.
            tint: Spectrum::ONE,
            film: None,
        }
    }

    /// Probability of choosing the compensation lobe. Proportional to the
    /// energy that lobe carries, so that neither lobe is under-sampled.
    fn compensation_probability(&self, wo: Vec3) -> f32 {
        let alpha = self.distribution.average_alpha();
        let e_o = energy::table().white_directional(shading::abs_cos_theta(wo), alpha);
        (1.0 - e_o).clamp(0.0, 0.5)
    }

    /// The multiple-scattering term's spectral scale. Derived so that a
    /// perfectly reflective conductor (`F = 1`) conserves energy exactly, and
    /// so that a coloured one saturates with each additional bounce the way a
    /// real rough metal does.
    fn compensation_scale(&self, e_avg: f32) -> Spectrum {
        let f_avg = fresnel::conductor_average(self.ior.normal_incidence_reflectance());
        energy::multiple_scattering_scale(f_avg, e_avg)
    }
}

impl Bsdf for Conductor {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        if wo.z <= 0.0 || wi.z <= 0.0 || self.distribution.is_smooth() {
            return Spectrum::ZERO;
        }
        let Some(wm) = (wo + wi).try_normalize() else {
            return Spectrum::ZERO;
        };
        let wm = if wm.z < 0.0 { -wm } else { wm };

        let d = self.distribution.d(wm);
        let g2 = self.distribution.g2(wo, wi);
        let f = self.reflectance(wo.dot(wm).abs());
        let single = f * (d * g2 / (4.0 * wo.z * wi.z));

        let alpha = self.distribution.average_alpha();
        let table = energy::table();
        let e_o = table.white_directional(wo.z, alpha);
        let e_i = table.white_directional(wi.z, alpha);
        let e_avg = table.white_average(alpha);
        let multi = self.compensation_scale(e_avg) * multiple_scattering_value(e_o, e_i, e_avg);

        (single + multi) * self.tint
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if wo.z <= 0.0 || wi.z <= 0.0 || self.distribution.is_smooth() {
            return 0.0;
        }
        // Weighted sum over *both* lobes. Returning only the sampled lobe's
        // PDF here is the bug `docs/02` §2.4 warns about.
        let p_comp = self.compensation_probability(wo);
        (1.0 - p_comp) * self.distribution.reflection_pdf(wo, wi)
            + p_comp * cosine_hemisphere_pdf(wi.z)
    }

    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample> {
        if wo.z <= 0.0 {
            return None;
        }

        // Perfect mirror: a delta lobe, handled separately because the
        // microfacet expressions divide by a vanishing NDF width.
        if self.distribution.is_smooth() {
            let wi = Vec3 {
                x: -wo.x,
                y: -wo.y,
                z: wo.z,
            };
            let f = self.reflectance(wo.z);
            return Some(BsdfSample {
                wi,
                // Pre-divided by the cosine: the integrator multiplies by
                // |cos| and by 1/pdf, and for a delta lobe both are formally
                // absorbed into the value.
                f: f * self.tint / wi.z.abs(),
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
            let wi = -wo + wm * (2.0 * wo.dot(wm));
            if wi.z <= 0.0 {
                return None;
            }
            wi
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
        // The directional albedo of the compensated lobe is, by construction,
        // the Fresnel reflectance scaled to close the energy budget. Bounded
        // by 1 so Russian roulette keeps terminating.
        let mu = shading::abs_cos_theta(wo);
        if self.distribution.is_smooth() {
            return (self.reflectance(mu) * self.tint).clamp(0.0, 1.0);
        }
        let f0 = self.ior.normal_incidence_reflectance();
        let alpha = self.distribution.average_alpha();
        let table = energy::table();
        // The split-sum term accounts for the *microfacet* Fresnel rather
        // than the macro-surface one; on a rough surface at oblique
        // incidence the two diverge sharply. See `crate::energy`.
        let single = table.conductor_directional(mu, alpha, f0);
        let e_o = table.white_directional(mu, alpha);
        let e_avg = table.white_average(alpha);
        ((single + self.compensation_scale(e_avg) * (1.0 - e_o)) * self.tint).clamp(0.0, 1.0)
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
    use crate::fresnel::metals;
    use tequila_core::math::{vec2, vec3};
    use tequila_core::rng::Pcg32;

    fn wo_at(mu: f32) -> Vec3 {
        vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu)
    }

    /// White-furnace energy of the lobe against anisotropy, measured by
    /// sampling it.
    fn mean_energy(lobe: &Conductor, wo: tequila_core::math::Vec3) -> f32 {
        use tequila_core::math::Vec2;
        const N: usize = 16384;
        let mut total = 0.0;
        for i in 0..N {
            let u = Vec2 {
                x: (i as f32 + 0.5) / N as f32,
                y: ((i * 7919) % N) as f32 / N as f32,
            };
            let uc = ((i * 4099) % N) as f32 / N as f32;
            if let Some(sample) = lobe.sample(wo, u, uc) {
                if sample.pdf > 0.0 {
                    total += sample.f.luminance() * sample.wi.z.abs() / sample.pdf;
                }
            }
        }
        total / N as f32
    }

    /// Energy compensation holds across the anisotropy a person would use,
    /// and this records where it stops holding.
    ///
    /// §2.3's compensation is built on tables integrated for an *isotropic*
    /// GGX, and an anisotropic lobe looks them up with the geometric mean of
    /// its two roughnesses. That proxy is good while the surface is not
    /// stretched far and stops being good when it is. Measured on a perfect
    /// white mirror, where the answer must be 1 — worst error over three
    /// angles and both azimuths:
    ///
    /// ```text
    ///   anisotropy    0.0     0.2     0.3     0.5     0.7     0.9
    ///   r 0.1       0.0008  0.0008  0.0009  0.0012  0.0018  0.0026
    ///   r 0.3       0.0027  0.0034  0.0042  0.0073  0.0137  0.0307
    ///   r 0.5       0.0027  0.0031  0.0039  0.0069  0.0140  0.0410
    ///   r 0.8       0.0027  0.0048  0.0078  0.0196  0.0480  0.1436
    ///   r 1.0       0.0030  0.0042  0.0058  0.0120  0.0257  0.0673
    /// ```
    ///
    /// Under a percent to anisotropy 0.3 at any roughness, about two percent
    /// at 0.5, and fourteen in the far corner. The Anisotropic preset sits at
    /// 0.7 and roughness 0.25, which is a bit over one percent.
    ///
    /// **The obvious fix is worse, and that is why this is a test and not a
    /// TODO.** Handing each direction the roughness it is actually masked by
    /// — `sqrt((ax*wx)^2 + (ay*wy)^2)` over its projection, the alpha that
    /// reproduces that direction's Smith lambda exactly — was tried. It sends
    /// this measurement to 1.71 along the stretched axis at anisotropy 0.99
    /// while leaving 0.74 across it, because Kulla-Conty's construction needs
    /// `e_o`, `e_i` and `e_avg` to describe one surface, and feeding the
    /// first two from different ones breaks the identity that makes the term
    /// conserve at all. A real fix gives the table an anisotropy axis, which
    /// is a build-cost decision rather than a one-line one.
    ///
    /// The assertion covers the range where the proxy is sound. It fails if
    /// that degrades, and it fails if someone widens the range without
    /// re-measuring — which is the better of the two ways to find out.
    #[test]
    fn energy_compensation_holds_to_the_anisotropy_people_use() {
        let white = ComplexIor {
            n: Spectrum::splat(1.0),
            k: Spectrum::splat(1e6),
        };
        let mut worst = (0.0f32, String::new());
        for &roughness in &[0.1f32, 0.2, 0.3, 0.5, 0.8, 1.0] {
            for &anisotropy in &[0.0f32, 0.2, 0.3, 0.5] {
                let lobe = Conductor::new(Ggx::from_roughness(roughness, anisotropy), white);
                for &degrees in &[15.0f32, 45.0, 75.0] {
                    let theta: f32 = degrees.to_radians();
                    // Both azimuths: along the stretched axis and across it.
                    // The error has opposite signs on the two, so a test that
                    // looked at one would see half of it.
                    for (axis, phi) in [("along", 0.0f32), ("across", std::f32::consts::FRAC_PI_2)]
                    {
                        let wo = vec3(
                            theta.sin() * phi.cos(),
                            theta.sin() * phi.sin(),
                            theta.cos(),
                        );
                        let error = (mean_energy(&lobe, wo) - 1.0).abs();
                        if error > worst.0 {
                            worst = (
                                error,
                                format!(
                                    "roughness {roughness}, anisotropy {anisotropy}, \
                                     {axis} at {degrees} degrees: off by {error:.4}"
                                ),
                            );
                        }
                    }
                }
            }
        }
        assert!(
            worst.0 < 0.025,
            "a perfect mirror is not conserving energy: {}",
            worst.1
        );
    }

    #[test]
    fn rough_gold_keeps_its_energy_and_its_hue() {
        let mut rng = Pcg32::new(22, 22);
        let smooth = Conductor::new(Ggx::from_roughness(0.05, 0.0), metals::GOLD);
        let rough = Conductor::new(Ggx::from_roughness(0.9, 0.0), metals::GOLD);
        let wo = wo_at(0.8);

        let measure = |lobe: &Conductor, rng: &mut Pcg32| {
            let n = 300_000;
            let mut sum = Spectrum::ZERO;
            for _ in 0..n {
                let u = vec2(rng.next_f32(), rng.next_f32());
                if let Some(s) = lobe.sample(wo, u, rng.next_f32()) {
                    sum += s.throughput_weight();
                }
            }
            sum / n as f32
        };

        let polished = measure(&smooth, &mut rng);
        let compensated = measure(&rough, &mut rng);

        // What single-scattering GGX alone would have returned.
        let alpha = rough.distribution.average_alpha();
        let uncompensated = energy::table().conductor_directional(
            wo.z,
            alpha,
            metals::GOLD.normal_incidence_reflectance(),
        );

        assert!(
            compensated.luminance() > 1.5 * uncompensated.luminance(),
            "compensation barely helped: {:.4} vs uncompensated {:.4}",
            compensated.luminance(),
            uncompensated.luminance()
        );
        assert!(
            compensated.luminance() < polished.luminance() + 1e-3,
            "rough gold out-reflected polished gold: {:.4} vs {:.4}",
            compensated.luminance(),
            polished.luminance()
        );
        // And it must still read as gold: red-dominant, blue-poor.
        assert!(
            compensated.r > compensated.g && compensated.g > compensated.b,
            "rough gold turned {compensated:?}"
        );
        // Roughening deepens the tint rather than washing it out, because each
        // extra bounce multiplies by the same coloured Fresnel.
        assert!(
            compensated.b / compensated.r < polished.b / polished.r,
            "rough gold desaturated instead of saturating"
        );
    }

    #[test]
    fn smooth_metal_is_a_delta_lobe() {
        let lobe = Conductor::new(Ggx::isotropic(0.0), metals::SILVER);
        assert!(lobe.flags().is_pure_delta());
        let wo = wo_at(0.7);
        let s = lobe.sample(wo, vec2(0.3, 0.6), 0.5).unwrap();
        assert!(s.is_delta);
        // Mirror reflection about +Z.
        assert!((s.wi.z - wo.z).abs() < 1e-6);
        assert!((s.wi.x + wo.x).abs() < 1e-6);
        // A delta lobe contributes nothing to next-event estimation.
        assert!(lobe.eval(wo, s.wi).is_black());
        assert_eq!(lobe.pdf(wo, s.wi), 0.0);
    }

    #[test]
    fn albedo_never_exceeds_one() {
        for (name, ior) in metals::ALL {
            for roughness in [0.0f32, 0.3, 1.0] {
                let lobe = Conductor::new(Ggx::from_roughness(roughness, 0.0), ior);
                for mu in [0.05f32, 0.5, 1.0] {
                    let a = lobe.albedo(wo_at(mu));
                    assert!(
                        a.max_component() <= 1.0 && a.min_component() >= 0.0,
                        "{name} roughness {roughness} mu {mu}: albedo {a:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn is_reciprocal() {
        let mut rng = Pcg32::new(24, 24);
        for roughness in [0.2f32, 0.6, 1.0] {
            let lobe = Conductor::new(Ggx::from_roughness(roughness, 0.0), metals::ALUMINIUM);
            for _ in 0..2000 {
                let wo = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
                let wi = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
                let a = lobe.eval(wo, wi).g;
                let b = lobe.eval(wi, wo).g;
                assert!((a - b).abs() <= 1e-4 * a.max(b).max(1.0), "{a} vs {b}");
            }
        }
    }
}
