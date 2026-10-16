//! Specular-over-diffuse layering.
//!
//! The first piece of the OpenPBR layer stack (`docs/02-MATERIAL-SYSTEM.md`
//! §2.1). It is what makes Plastic and Paint look like plastic and paint: a
//! dielectric specular interface over a diffuse substrate, with the substrate
//! attenuated by exactly the energy the interface kept.
//!
//! Getting the attenuation wrong is the "why is my coated material too dark"
//! bug §1 of that document calls out. There are three ways to get it wrong,
//! and this module avoids all three:
//!
//! 1. **Scaling by `1 - F(cos_theta_o)`.** Ignores that light must cross the
//!    interface twice, and uses the macro-surface Fresnel where the
//!    microfacet-relative one is what matters (see [`crate::energy`]).
//! 2. **Forgetting the outbound crossing.** The transmitted fraction enters
//!    squared, once for each direction.
//! 3. **Forgetting the light trapped between the layers.** Light that reaches
//!    the substrate, scatters back up, and reflects off the *underside* of the
//!    interface gets another try at the substrate. Summing that geometric
//!    series is the difference between a stack that conserves energy and one
//!    that quietly loses 7% at grazing angles.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use crate::lobes::{DielectricSpecular, OrenNayarDiffuse};
use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;

/// A dielectric specular layer over an Oren-Nayar diffuse substrate.
#[derive(Clone, Copy, Debug)]
pub struct SpecularOverDiffuse {
    pub specular: DielectricSpecular,
    pub diffuse: OrenNayarDiffuse,
}

impl SpecularOverDiffuse {
    /// The Plastic preset of `docs/02` §3.1: IOR 1.5, `specular_weight` 1.
    pub fn plastic(color: Spectrum, roughness: f32) -> Self {
        SpecularOverDiffuse {
            specular: DielectricSpecular::new(roughness, 1.5),
            diffuse: OrenNayarDiffuse::new(color, 0.0),
        }
    }

    /// Gain from light trapped between the substrate and the underside of the
    /// interface: `1 / (1 - rho * R_internal)`, the sum of the geometric
    /// series over repeated internal bounces.
    ///
    /// `R_internal` is approximated by the interface's *external*
    /// hemispherical reflectance. The true internal value is higher — total
    /// internal reflection has no external counterpart — so this slightly
    /// understates the gain for a bright substrate. It is exact for the
    /// white-furnace case that pins the energy budget, and the residual error
    /// elsewhere is well inside the tolerance of
    /// [`tests::white_furnace_passes_for_the_stack`].
    #[inline]
    fn internal_reflection_gain(&self) -> f32 {
        let r_internal = self.specular.hemispherical_albedo();
        let rho = self.diffuse.albedo.luminance().clamp(0.0, 1.0);
        1.0 / (1.0 - rho * r_internal).max(1e-3)
    }

    /// Fraction of energy reaching the substrate and returning, including the
    /// trapped-light gain.
    ///
    /// The transmitted fraction appears twice because light must cross the
    /// interface inbound and outbound. That double crossing is what makes a
    /// coated surface darken correctly towards grazing angles instead of
    /// staying flat.
    #[inline]
    fn substrate_weight(&self, wo: Vec3, wi: Vec3) -> f32 {
        (1.0 - self.specular.directional_albedo(wo))
            * (1.0 - self.specular.directional_albedo(wi))
            * self.internal_reflection_gain()
    }

    /// Probability of sampling the specular layer rather than the substrate.
    #[inline]
    fn specular_probability(&self, wo: Vec3) -> f32 {
        let spec = self.specular.directional_albedo(wo);
        let diff = self.diffuse.albedo.luminance() * (1.0 - spec);
        let total = spec + diff;
        if total <= 0.0 {
            0.0
        } else {
            // Bounded away from 0 and 1 so neither lobe is ever starved of
            // samples, which would spike variance on a nearly-black or
            // nearly-mirror material.
            (spec / total).clamp(0.05, 0.95)
        }
    }
}

impl Bsdf for SpecularOverDiffuse {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        if wo.z <= 0.0 || wi.z <= 0.0 {
            return Spectrum::ZERO;
        }
        self.specular.eval(wo, wi) + self.diffuse.eval(wo, wi) * self.substrate_weight(wo, wi)
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if wo.z <= 0.0 || wi.z <= 0.0 {
            return 0.0;
        }
        let p_spec = self.specular_probability(wo);
        p_spec * self.specular.pdf(wo, wi) + (1.0 - p_spec) * self.diffuse.pdf(wo, wi)
    }

    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample> {
        if wo.z <= 0.0 {
            return None;
        }
        let p_spec = self.specular_probability(wo);

        // Remap the lobe-selection sample back into [0,1) after using it, so
        // the sub-lobe still receives a stratified value instead of a value
        // biased towards one end of its range.
        let wi = if uc < p_spec {
            let remapped = (uc / p_spec).min(1.0 - f32::EPSILON / 2.0);
            let s = self.specular.sample(wo, u, remapped)?;
            if s.is_delta {
                // A smooth interface over a substrate: the specular part stays
                // a delta lobe, weighted by the probability of choosing it.
                return Some(BsdfSample { pdf: p_spec, ..s });
            }
            s.wi
        } else {
            let remapped = ((uc - p_spec) / (1.0 - p_spec)).min(1.0 - f32::EPSILON / 2.0);
            self.diffuse.sample(wo, u, remapped)?.wi
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
            flags: BsdfFlags::REFLECTION | BsdfFlags::GLOSSY | BsdfFlags::DIFFUSE,
        })
    }

    fn albedo(&self, wo: Vec3) -> Spectrum {
        let spec = self.specular.directional_albedo(wo);
        (self.specular.tint * spec + self.diffuse.albedo * (1.0 - spec)).clamp(0.0, 1.0)
    }

    fn flags(&self) -> BsdfFlags {
        let mut f = BsdfFlags::REFLECTION | BsdfFlags::DIFFUSE;
        if self.specular.distribution.is_smooth() {
            f |= BsdfFlags::DELTA;
        } else {
            f |= BsdfFlags::GLOSSY;
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_core::rng::Pcg32;
    use tequila_core::sampling::sample_cosine_hemisphere;

    fn wo_at(mu: f32) -> Vec3 {
        vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu)
    }

    fn measure(lobe: &SpecularOverDiffuse, wo: Vec3, rng: &mut Pcg32) -> Spectrum {
        let n = 400_000;
        let mut sum = Spectrum::ZERO;
        for _ in 0..n {
            let u = vec2(rng.next_f32(), rng.next_f32());
            if let Some(s) = lobe.sample(wo, u, rng.next_f32()) {
                sum += s.throughput_weight();
            }
        }
        sum / n as f32
    }

    /// White furnace over the whole stack. `docs/05` §3.1 asks specifically
    /// for the layered case: "layering is where energy is most often lost".
    ///
    /// A white substrate under a lossless interface must reflect *everything*:
    /// whatever the interface does not keep must reach the substrate, and
    /// whatever bounces back down off the interface's underside must get
    /// another try. Dropping either term shows up here and nowhere else.
    #[test]
    fn white_furnace_passes_for_the_stack() {
        let mut rng = Pcg32::new(41, 41);
        for spec_roughness in [0.0f32, 0.05, 0.3, 0.7, 1.0] {
            for diff_roughness in [0.0f32, 0.5, 1.0] {
                let lobe = SpecularOverDiffuse {
                    specular: DielectricSpecular::new(spec_roughness, 1.5),
                    diffuse: OrenNayarDiffuse::new(Spectrum::ONE, diff_roughness),
                };
                for mu in [0.15f32, 0.5, 0.9, 1.0] {
                    let result = measure(&lobe, wo_at(mu), &mut rng).g;
                    assert!(
                        (result - 1.0).abs() < 0.02,
                        "spec {spec_roughness} diff {diff_roughness} mu {mu}: \
                         furnace {result}"
                    );
                }
            }
        }
    }

    /// The stack must never return more energy than it received, at any
    /// albedo. The inter-reflection series is the term most likely to
    /// overshoot, since it is a division by a quantity approaching zero.
    #[test]
    fn never_reflects_more_than_it_receives() {
        let mut rng = Pcg32::new(45, 45);
        for rho in [0.0f32, 0.5, 0.9, 1.0] {
            for roughness in [0.0f32, 0.4, 1.0] {
                let lobe = SpecularOverDiffuse::plastic(Spectrum::splat(rho), roughness);
                for mu in [0.1f32, 0.5, 1.0] {
                    let r = measure(&lobe, wo_at(mu), &mut rng);
                    assert!(
                        r.max_component() <= 1.0 + 0.02,
                        "rho {rho} roughness {roughness} mu {mu}: reflected {r:?}"
                    );
                }
            }
        }
    }

    /// The substrate must dim as the interface reflects more. A stack that
    /// brightens as the coat strengthens is the classic double-counting bug.
    #[test]
    fn coating_attenuates_the_substrate() {
        let mut rng = Pcg32::new(43, 43);
        let wo = wo_at(0.2); // grazing, where the interface reflects most
        let substrate_only = |lobe: &SpecularOverDiffuse, rng: &mut Pcg32| {
            let n = 200_000;
            let mut sum = 0.0f64;
            for _ in 0..n {
                let u = vec2(rng.next_f32(), rng.next_f32());
                if let Some(s) = lobe.sample(wo, u, rng.next_f32()) {
                    let d = lobe.diffuse.eval(wo, s.wi)
                        * lobe.substrate_weight(wo, s.wi)
                        * (s.wi.z.abs() / s.pdf);
                    sum += d.g as f64;
                }
            }
            sum / n as f64
        };
        let uncoated = SpecularOverDiffuse {
            specular: DielectricSpecular::new(0.3, 1.5).with_weight(0.0),
            diffuse: OrenNayarDiffuse::new(Spectrum::splat(0.8), 0.0),
        };
        let coated = SpecularOverDiffuse {
            specular: DielectricSpecular::new(0.3, 1.5),
            diffuse: OrenNayarDiffuse::new(Spectrum::splat(0.8), 0.0),
        };
        let a = substrate_only(&uncoated, &mut rng);
        let b = substrate_only(&coated, &mut rng);
        assert!(b < a, "coating did not attenuate the substrate: {a} -> {b}");
    }

    /// A coated surface must get *more* specular towards grazing incidence,
    /// which is the visual signature of a real varnish.
    #[test]
    fn reflection_strengthens_towards_grazing() {
        let lobe = SpecularOverDiffuse::plastic(Spectrum::splat(0.5), 0.2);
        let near_normal = lobe.specular.directional_albedo(wo_at(1.0));
        let grazing = lobe.specular.directional_albedo(wo_at(0.1));
        assert!(
            grazing > near_normal * 2.0,
            "no Fresnel ramp: {near_normal} -> {grazing}"
        );
    }

    #[test]
    fn is_reciprocal() {
        let mut rng = Pcg32::new(44, 44);
        let lobe = SpecularOverDiffuse::plastic(Spectrum::splat(0.7), 0.4);
        for _ in 0..2000 {
            let wo = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
            let wi = sample_cosine_hemisphere(vec2(rng.next_f32(), rng.next_f32()));
            let a = lobe.eval(wo, wi).g;
            let b = lobe.eval(wi, wo).g;
            assert!((a - b).abs() <= 1e-4 * a.max(b).max(1.0), "{a} vs {b}");
        }
    }
}
