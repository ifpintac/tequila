//! The OpenPBR `transmission` lobe: rough dielectric refraction.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3: "GGX with dielectric Fresnel and
//! refraction; medium attached for volumetric absorption/scattering", sampled
//! by "VNDF + Fresnel-weighted reflect/refract choice".
//!
//! This is what Glass, Liquid, Gem and Dielectric are made of. It is also the
//! lobe with the most ways to be subtly wrong, and three of them are worth
//! naming because each produces a plausible-looking image that is not right:
//!
//! 1. **Forgetting the `1/eta²` radiance compression.** Refraction into a
//!    denser medium squeezes a solid angle of directions into a smaller one,
//!    and radiance rises to match. Omit the factor and glass renders too dark
//!    from outside and too bright from inside — and, worse, the two disagree,
//!    so a closed object is not energy-conserving.
//!
//! 2. **Using the macro normal for Fresnel.** The Fresnel term belongs on the
//!    *microfacet*, `dot(wo, h)`, not on the surface, `dot(wo, n)`. Using the
//!    surface normal makes rough glass lose its grazing brightening.
//!
//! 3. **Sampling reflection and refraction with the wrong PDF split.** The
//!    discrete choice is made with probability `F`, so the PDF of a refracted
//!    direction carries a factor `1 - F`. Dropping it silently biases every
//!    MIS weight in the scene, and the error shows up as glass that is too
//!    bright next to a light and too dark away from one.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use crate::energy;
use crate::fresnel;
use crate::microfacet::Ggx;
use tequila_core::math::{shading, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;

/// Rough dielectric reflection and transmission at one interface.
#[derive(Clone, Copy, Debug)]
pub struct DielectricTransmission {
    pub distribution: Ggx,
    /// Index of refraction of the inside relative to the outside.
    pub ior: f32,
    /// Tint applied to transmitted light, OpenPBR `transmission_color`.
    ///
    /// The surface tint of the thin-walled case. A solid object tints by
    /// Beer-Lambert absorption through its volume instead, which is the
    /// medium's business rather than the interface's.
    pub tint: Spectrum,
    /// Whether the surface is an infinitely thin shell.
    ///
    /// A thin shell refracts twice over a negligible distance, so the two
    /// deflections cancel and the ray continues straight on. Modelling a
    /// window pane as a solid block instead bends the view through it, which
    /// looks wrong in exactly the way people notice.
    pub thin_walled: bool,
    /// Whether the ray arrives from inside the material.
    ///
    /// The side cannot be read off `wo` here. The integrator turns the
    /// shading frame to face whichever way the ray came from, so `wo.z` is
    /// positive at every vertex and a lobe asking it is told "outside" even
    /// when the path is on its way out of a glass block. The caller knows —
    /// it has the geometric normal and the ray — so it says.
    pub from_inside: bool,
}

/// Mirror  about .
///
/// Written here rather than pulled from the shading helpers because those are
/// about the *surface* frame, and this reflection is about a microfacet.
#[inline]
fn reflect(wo: Vec3, n: Vec3) -> Vec3 {
    -wo + n * (2.0 * wo.dot(n))
}

impl DielectricTransmission {
    pub fn new(roughness: f32, ior: f32) -> Self {
        // With no index contrast every microfacet passes light straight on,
        // so the surface is smooth whatever its roughness â€” and has to be
        // made so, because the refraction half vector is undefined there.
        let index_matched = ior <= 1.0 + 1e-3;
        DielectricTransmission {
            distribution: Ggx::from_roughness(if index_matched { 0.0 } else { roughness }, 0.0),
            ior: ior.max(1.0),
            tint: Spectrum::ONE,
            thin_walled: false,
            from_inside: false,
        }
    }

    pub fn with_tint(mut self, tint: Spectrum) -> Self {
        self.tint = tint;
        self
    }

    pub fn thin_walled(mut self, thin: bool) -> Self {
        self.thin_walled = thin;
        self
    }

    /// The relative IOR for the side the ray is arriving from.
    ///
    /// Light crossing inward sees `ior`; a ray already inside sees its
    /// reciprocal. Getting this backwards makes glass act as a diverging lens
    /// — and, worse, applies the `1/η²` radiance compression of entering a
    /// denser medium at every crossing with nothing ever undoing it, so a
    /// glass block swallows most of the light that enters it.
    ///
    /// Taken from [`Self::from_inside`] rather than from `wo.z`, which the
    /// shading frame has already made positive whichever side the ray is on.
    /// It is also what makes total internal reflection possible at all: that
    /// needs `η < 1`, which a lobe that thinks it is always entering can
    /// never see.
    #[inline]
    fn relative_ior(&self, _wo: Vec3) -> f32 {
        if self.from_inside {
            1.0 / self.ior
        } else {
            self.ior
        }
    }

    /// The same lobe, for a ray on its way out.
    #[inline]
    pub fn from_inside(mut self, inside: bool) -> Self {
        self.from_inside = inside;
        self
    }

    /// How much to brighten single scattering so the interface passes on all
    /// of the light that reaches it.
    ///
    /// A microfacet can send light into a neighbouring facet, and the
    /// single-scattering model has nowhere to put light that bounces twice,
    /// so it drops it. §2.3 makes correcting that mandatory. The reflective
    /// lobes do it by *adding* a Kulla-Conty lobe shaped like the missing
    /// energy, but that will not work here: what this interface loses has to
    /// come back split between a reflected and a refracted direction, in a
    /// ratio that depends on where the second bounce landed, and there is no
    /// closed form for the shape of it.
    ///
    /// So the whole lobe is scaled by the reciprocal of what it passes on
    /// [Turquin 2019, *Practical multiple scattering compensation for
    /// microfacet models*]. That puts the energy back in proportion to where
    /// the single-scattering lobe already sends light, rather than in the
    /// right shape — a good approximation precisely because a rough interface
    /// scatters broadly enough that the two are close.
    ///
    /// The scale depends on `wo` alone, so the lobe is no longer reciprocal.
    /// That is the accepted cost of the method and it is invisible to a path
    /// tracer that always starts at the camera; it would matter to
    /// bidirectional transport, which `docs/01` does not call for.
    #[inline]
    fn energy_scale(&self, wo: Vec3) -> f32 {
        if self.distribution.is_smooth() {
            // Nothing is shadowed on a flat interface, so nothing is missing.
            return 1.0;
        }
        let mu = shading::abs_cos_theta(wo);
        let alpha = self.distribution.average_alpha();
        let passed = if self.thin_walled {
            // A shell sends light out of the reflection half vector either
            // way, straight on or mirrored, so what it loses is exactly what
            // a white reflector loses and the reflection table already knows.
            energy::table().white_directional(mu, alpha)
        } else {
            energy::table().interface_directional(mu, alpha, self.ior, self.from_inside)
        };
        if passed <= 0.0 {
            return 1.0;
        }
        // Ceiling on the correction. The table bottoms out near 0.4 for a
        // fully rough diamond seen from inside, so 4 is well clear of every
        // real material; it is here so that a grazing sliver where the table
        // reads near zero cannot turn one sample into a firefly.
        (1.0 / passed).min(4.0)
    }

    /// Refraction through a microfacet.
    ///
    /// `eta` is the index of the medium *ahead* relative to the one behind,
    /// already resolved for the side `wi` arrives from. The normal is flipped
    /// to that side here; `eta` is not, because inverting it as well would
    /// undo the caller's choice and make a ray leaving glass behave as one
    /// entering it.
    pub(crate) fn refract(wi: Vec3, normal: Vec3, eta: f32) -> Option<Vec3> {
        let cos_i = normal.dot(wi);
        let (cos_i, normal) = if cos_i < 0.0 {
            (-cos_i, -normal)
        } else {
            (cos_i, normal)
        };
        let sin2_i = (1.0 - cos_i * cos_i).max(0.0);
        let sin2_t = sin2_i / (eta * eta);
        // Total internal reflection: no refracted direction exists.
        if sin2_t >= 1.0 {
            return None;
        }
        let cos_t = (1.0 - sin2_t).sqrt();
        Some(-wi / eta + normal * (cos_i / eta - cos_t))
    }
}

impl Bsdf for DielectricTransmission {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        if self.distribution.is_smooth() {
            // A smooth interface scatters into two delta directions, which
            // `eval` cannot represent and which NEE must never try to reach.
            return Spectrum::ZERO;
        }
        let (cos_o, cos_i) = (wo.z, wi.z);
        if cos_o == 0.0 || cos_i == 0.0 {
            return Spectrum::ZERO;
        }
        let eta = self.relative_ior(wo);
        let reflecting = cos_o * cos_i > 0.0;

        // A thin shell is treated as a reflection off the *mirrored* surface:
        // light enters and leaves parallel, so the exit direction is the
        // reflection seen from the other side. That reuses the reflection
        // half-vector and Jacobian exactly, which a hand-rolled "straight
        // through with roughness" cannot do correctly.
        let wi_effective = if !reflecting && self.thin_walled {
            Vec3 {
                x: wi.x,
                y: wi.y,
                z: -wi.z,
            }
        } else {
            wi
        };

        let h = if reflecting || self.thin_walled {
            wo + wi_effective
        } else {
            // The *generalized* half vector, which weights the two directions
            // by their indices. The plain sum is not the microfacet normal.
            wo + wi * eta
        };
        let Some(h) = h.try_normalize() else {
            return Spectrum::ZERO;
        };
        // Oriented to the *macrosurface* normal, not to `wo`.
        //
        // Reconstructing a half vector from a pair of directions always yields
        // something; whether an upward-facing microfacet could have produced
        // that pair is a separate question, and it is the one that matters.
        // Flipping towards `wo` answers it wrongly: a microfacet that points
        // up but away from `wo` gets turned into one that points down, and the
        // pair is then given a density no real facet could have generated. The
        // χ² test saw that as a density integrating to 1.25 over the sphere,
        // piled into the bins just below the horizon opposite `wo`, where the
        // refraction lobe cannot reach.
        let upward = if h.z < 0.0 { -h } else { h };
        // A microfacet takes part only if it faces both directions. Comparing
        // against the sign of each cosine rather than against zero keeps this
        // right for a ray already inside the medium, where "facing" means
        // pointing the other way.
        if upward.dot(wo) * cos_o <= 0.0 || upward.dot(wi_effective) * wi_effective.z <= 0.0 {
            return Spectrum::ZERO;
        }

        // The distribution, its visible-normal density and the masking term
        // are written against `wo`'s hemisphere, so from inside the medium
        // they want the facet pointing down. Only the test above needs the
        // macrosurface orientation.
        let h = if upward.dot(wo) < 0.0 { -upward } else { upward };

        let d = self.distribution.d(h);
        let f = fresnel::dielectric(wo.dot(h).abs(), eta);

        // Applied here rather than in `sample`, which reads its value from
        // this function: one place for both, and next-event estimation gets
        // the same interface the path sampler does.
        let compensation = self.energy_scale(wo);

        if reflecting {
            let g = self.distribution.g2(wo, wi);
            Spectrum::splat(compensation * f * d * g / (4.0 * (cos_o * cos_i).abs()))
        } else if self.thin_walled {
            let g = self.distribution.g2(wo, wi_effective);
            self.tint * (compensation * (1.0 - f) * d * g / (4.0 * (cos_o * cos_i).abs()))
        } else {
            let g = self.distribution.g2(wo, wi);
            let denominator = wo.dot(h) + eta * wi.dot(h);
            let denominator = denominator * denominator;
            if denominator <= 1e-9 {
                return Spectrum::ZERO;
            }
            // No explicit 1/eta^2 here: writing the denominator as
            // `(wo.h + eta * wi.h)^2` rather than `(wi.h + wo.h/eta)^2`
            // already carries it, and applying it twice is a factor of
            // eta^2 of missing light — which reads as clear glass that is
            // mysteriously dark.
            let value = compensation * (1.0 - f) * d * g * (wi.dot(h) * wo.dot(h)).abs()
                / (denominator * (cos_o * cos_i).abs());
            self.tint * value
        }
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if self.distribution.is_smooth() {
            return 0.0;
        }
        let (cos_o, cos_i) = (wo.z, wi.z);
        if cos_o == 0.0 || cos_i == 0.0 {
            return 0.0;
        }
        let eta = self.relative_ior(wo);
        let reflecting = cos_o * cos_i > 0.0;

        let wi_effective = if !reflecting && self.thin_walled {
            Vec3 {
                x: wi.x,
                y: wi.y,
                z: -wi.z,
            }
        } else {
            wi
        };

        let h = if reflecting || self.thin_walled {
            wo + wi_effective
        } else {
            wo + wi * eta
        };
        let Some(h) = h.try_normalize() else {
            return 0.0;
        };
        // Oriented to the *macrosurface* normal, not to `wo`.
        //
        // Reconstructing a half vector from a pair of directions always yields
        // something; whether an upward-facing microfacet could have produced
        // that pair is a separate question, and it is the one that matters.
        // Flipping towards `wo` answers it wrongly: a microfacet that points
        // up but away from `wo` gets turned into one that points down, and the
        // pair is then given a density no real facet could have generated. The
        // χ² test saw that as a density integrating to 1.25 over the sphere,
        // piled into the bins just below the horizon opposite `wo`, where the
        // refraction lobe cannot reach.
        let upward = if h.z < 0.0 { -h } else { h };
        // A microfacet takes part only if it faces both directions. Comparing
        // against the sign of each cosine rather than against zero keeps this
        // right for a ray already inside the medium, where "facing" means
        // pointing the other way.
        if upward.dot(wo) * cos_o <= 0.0 || upward.dot(wi_effective) * wi_effective.z <= 0.0 {
            return 0.0;
        }

        // The distribution, its visible-normal density and the masking term
        // are written against `wo`'s hemisphere, so from inside the medium
        // they want the facet pointing down. Only the test above needs the
        // macrosurface orientation.
        let h = if upward.dot(wo) < 0.0 { -upward } else { upward };

        let f = fresnel::dielectric(wo.dot(h).abs(), eta);
        let visible = self.distribution.d_visible(wo, h);
        let cos_oh = wo.dot(h).abs();

        // The discrete reflect-or-refract choice's probability is part of the
        // density, which is point 3 in this module's header.
        if reflecting {
            visible * f / (4.0 * cos_oh)
        } else if self.thin_walled {
            visible * (1.0 - f) / (4.0 * cos_oh)
        } else {
            let denominator = wo.dot(h) + eta * wi.dot(h);
            let denominator = denominator * denominator;
            if denominator <= 1e-9 {
                return 0.0;
            }
            visible * (1.0 - f) * (eta * eta * wi.dot(h)).abs() / denominator
        }
    }

    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample> {
        let eta = self.relative_ior(wo);

        if self.distribution.is_smooth() {
            let f = fresnel::dielectric(shading::abs_cos_theta(wo), eta);
            if uc < f {
                let wi = Vec3 {
                    x: -wo.x,
                    y: -wo.y,
                    z: wo.z,
                };
                return Some(BsdfSample {
                    wi,
                    // Divided by the probability of having chosen this branch
                    // as well as by the cosine, so `pdf` can be 1 like every
                    // other delta lobe here. Reporting the branch probability
                    // instead looks equivalent — `f * cos / pdf` is 1 either
                    // way — but the layer stack forces a delta sample's `pdf`
                    // to 1 without touching `f`, so a lobe that leaves its
                    // Fresnel factor in `f` has that factor silently applied
                    // twice over and its light thrown away.
                    f: Spectrum::splat(1.0 / shading::abs_cos_theta(wi)),
                    pdf: 1.0,
                    is_delta: true,
                    is_transmission: false,
                    eta: 1.0,
                    flags: BsdfFlags::REFLECTION | BsdfFlags::DELTA,
                });
            }
            let (wi, scale, crossing) = if self.thin_walled {
                // A *smooth* shell really does pass light straight through:
                // the two refractions cancel exactly.
                (-wo, 1.0, 1.0)
            } else {
                let wi = DielectricTransmission::refract(wo, Vec3::Z, eta)?;
                (wi, 1.0 / (eta * eta), eta)
            };
            return Some(BsdfSample {
                wi,
                // As above: the `1 - f` of having chosen refraction is divided
                // out here rather than reported as a PDF the stack discards.
                f: self.tint * (scale / shading::abs_cos_theta(wi)),
                pdf: 1.0,
                is_delta: true,
                is_transmission: true,
                eta: crossing,
                flags: BsdfFlags::TRANSMISSION | BsdfFlags::DELTA,
            });
        }

        let h = self.distribution.sample_visible_normal(wo, u);
        let f = fresnel::dielectric(wo.dot(h).abs(), eta);

        let (wi, is_transmission, crossing) = if uc < f {
            (reflect(wo, h), false, 1.0)
        } else if self.thin_walled {
            // Reflect, then mirror into the far hemisphere: the exit
            // direction of a thin shell, and the one `eval` and `pdf` above
            // are written against.
            let reflected = reflect(wo, h);
            (
                Vec3 {
                    x: reflected.x,
                    y: reflected.y,
                    z: -reflected.z,
                },
                true,
                1.0,
            )
        } else {
            match DielectricTransmission::refract(wo, h, eta) {
                Some(wi) => (wi, true, eta),
                // Total internal reflection: the refracted branch has no
                // direction, so the light reflects instead. Returning `None`
                // would lose it, which is where "dark rims on glass" comes
                // from.
                None => (reflect(wo, h), false, 1.0),
            }
        };

        // The direction has to end up on the side of the macrosurface that
        // the branch it came from implies.
        //
        // A microfacet normal drawn from a rough distribution can be tilted
        // far enough that reflecting off it sends the ray *into* the surface,
        // or that refracting through it sends the ray back out. Those samples
        // are shadowed: single-scattering microfacet theory has no direction
        // for them, and the energy they carry is what the compensation tables
        // put back.
        //
        // Keeping them is worse than losing them. `pdf` classifies a direction
        // by the side it is on, so a reflection that came out underneath gets
        // measured as though it were a refraction — reconstructed from a
        // different half vector, with a density unrelated to how the sample
        // was actually made. The χ² test caught this as a density integrating
        // to 1.25 over the sphere, concentrated in the bins either side of the
        // horizon, and growing with both roughness and grazing angle.
        let same_side = wi.z * wo.z > 0.0;
        if same_side == is_transmission {
            return None;
        }

        let pdf = self.pdf(wo, wi);
        if pdf <= 0.0 {
            return None;
        }
        let value = self.eval(wo, wi);
        if value.is_black() {
            return None;
        }
        Some(BsdfSample {
            wi,
            f: value,
            pdf,
            is_delta: false,
            is_transmission,
            eta: crossing,
            flags: if is_transmission {
                BsdfFlags::TRANSMISSION | BsdfFlags::GLOSSY
            } else {
                BsdfFlags::REFLECTION | BsdfFlags::GLOSSY
            },
        })
    }

    fn albedo(&self, _wo: Vec3) -> Spectrum {
        // A clear interface passes almost everything, tinted. Used only as a
        // Russian-roulette and denoiser guide, so it must not exceed one.
        Spectrum {
            r: self.tint.r.clamp(0.0, 1.0),
            g: self.tint.g.clamp(0.0, 1.0),
            b: self.tint.b.clamp(0.0, 1.0),
        }
    }

    fn flags(&self) -> BsdfFlags {
        let scattering = BsdfFlags::REFLECTION | BsdfFlags::TRANSMISSION;
        if self.distribution.is_smooth() {
            scattering | BsdfFlags::DELTA
        } else {
            scattering | BsdfFlags::GLOSSY
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn glass(roughness: f32) -> DielectricTransmission {
        DielectricTransmission::new(roughness, 1.5)
    }

    fn direction(theta: f32) -> Vec3 {
        vec3(theta.sin(), 0.0, theta.cos())
    }

    /// Snell's law, which is the one thing this lobe must get exactly right.
    #[test]
    fn refraction_follows_snells_law() {
        let eta = 1.5;
        for degrees in [5.0f32, 20.0, 40.0, 60.0] {
            let theta_i = degrees.to_radians();
            let wo = direction(theta_i);
            let wi = DielectricTransmission::refract(wo, Vec3::Z, eta).expect("refracts");

            // sin(theta_i) = eta * sin(theta_t)
            let sin_t = (1.0 - wi.z * wi.z).max(0.0).sqrt();
            assert!(
                (theta_i.sin() - eta * sin_t).abs() < 1e-4,
                "{degrees} degrees: sin_i {} against eta*sin_t {}",
                theta_i.sin(),
                eta * sin_t
            );
            assert!(wi.z < 0.0, "the refracted ray stayed on the incident side");
        }
    }

    /// Past the critical angle there is no refracted direction at all, and the
    /// light must reflect rather than vanish. Losing it is where dark rims on
    /// glass come from.
    #[test]
    fn total_internal_reflection_reflects_rather_than_absorbing() {
        // Sixty-four degrees off the normal, against a critical angle of about
        // 41.8 for an index of 1.5: there is no way out.
        //
        // `wo.z` is *positive* and the lobe is told it is on the inside,
        // because that is the arrangement the renderer produces — the shading
        // frame is turned to meet the incoming ray, so a path leaving glass
        // arrives with a positive cosine just like one entering it. This test
        // used to say "inside" by flipping `wo.z`, which the lobe read and no
        // caller ever sent; total internal reflection passed here and could
        // not happen in a render.
        let inside = vec3(0.9, 0.0, 0.436).normalize();
        assert!(
            DielectricTransmission::refract(inside, Vec3::Z, 1.0 / 1.5).is_none(),
            "a ray past the critical angle found a way out"
        );

        let lobe = glass(0.2).from_inside(true);
        let mut reflected = 0;
        for i in 0..64 {
            let u = Vec2 {
                x: (i as f32 + 0.5) / 64.0,
                y: ((i * 13) % 64) as f32 / 64.0,
            };
            if let Some(sample) = lobe.sample(inside, u, 0.99) {
                assert!(sample.f.is_finite(), "{sample:?}");
                if !sample.is_transmission {
                    reflected += 1;
                }
            }
        }
        assert!(reflected > 0, "nothing reflected past the critical angle");
    }

    /// A thin shell refracts twice over no distance, so the deflections cancel
    /// and the view through a window pane is undistorted.
    #[test]
    fn a_thin_shell_passes_light_straight_through() {
        let pane = glass(0.0).thin_walled(true);
        let wo = direction(0.7);
        let sample = pane
            .sample(wo, Vec2::splat(0.5), 0.99)
            .expect("transmits at a grazing-ish angle");

        assert!(sample.is_transmission);
        assert!(
            (sample.wi + wo).length() < 1e-5,
            "a pane deflected the ray: {:?} against {:?}",
            sample.wi,
            -wo
        );
        assert_eq!(sample.eta, 1.0, "a pane must not change the medium");
    }

    /// A solid block does bend it, which is the difference the flag exists for.
    #[test]
    fn a_solid_block_bends_the_ray() {
        let block = glass(0.0);
        let wo = direction(0.7);
        let sample = block.sample(wo, Vec2::splat(0.5), 0.99).expect("transmits");
        assert!(sample.is_transmission);
        assert!(
            (sample.wi + wo).length() > 0.1,
            "a solid block passed light straight through"
        );
        assert!(sample.eta > 1.0);
    }

    /// Throughput through a single refraction, measured by the ratio
    /// estimator: the mean of `f * cos / pdf` over sampled directions.
    fn mean_throughput(lobe: &DielectricTransmission, wo: Vec3) -> f32 {
        let mut total = 0.0;
        const SAMPLES: usize = 8192;
        for i in 0..SAMPLES {
            let u = Vec2 {
                x: (i as f32 + 0.5) / SAMPLES as f32,
                // A coprime stride, so the second dimension is not correlated
                // with the first.
                y: ((i * 7919) % SAMPLES) as f32 / SAMPLES as f32,
            };
            let uc = ((i * 4099) % SAMPLES) as f32 / SAMPLES as f32;
            if let Some(sample) = lobe.sample(wo, u, uc) {
                if sample.pdf > 0.0 {
                    let weight =
                        sample.f.luminance() * shading::abs_cos_theta(sample.wi) / sample.pdf;
                    assert!(weight.is_finite() && weight >= 0.0, "weight {weight}");
                    total += weight;
                }
            }
        }
        total / SAMPLES as f32
    }

    /// Entering a denser medium *compresses* radiance by `1/eta^2`, and
    /// leaving it expands by the same factor. A single refraction therefore
    /// does **not** have unit throughput, and a test asserting that it does is
    /// testing a misconception — the correct statement is that the two cancel.
    ///
    /// This is worth pinning down because both mistakes look plausible in a
    /// render. Drop the compression and glass glows; apply it twice and glass
    /// goes dark, which is the bug this lobe's header warns about first.
    #[test]
    fn entering_and_leaving_a_medium_cancel_exactly() {
        let eta = 1.5;
        let lobe = glass(0.15);

        // Near-normal on both sides: far from the critical angle, so the
        // measurement is about refraction rather than about total internal
        // reflection.
        //
        // The same direction both times, with only the declared side
        // differing — which is exactly what the renderer hands the lobe, the
        // shading frame having already turned to meet the ray.
        let wo = direction(0.2);
        let entering = mean_throughput(&lobe, wo);
        let leaving = mean_throughput(&glass(0.15).from_inside(true), wo);

        assert!(
            (entering - 1.0 / (eta * eta)).abs() < 0.06,
            "entering carried {entering:.4}, expected about {:.4}",
            1.0 / (eta * eta)
        );
        assert!(
            (leaving - eta * eta).abs() < 0.35,
            "leaving carried {leaving:.4}, expected about {:.4}",
            eta * eta
        );
        assert!(
            (entering * leaving - 1.0).abs() < 0.1,
            "a round trip through the surface carried {:.4}, not 1 — so a \
             closed glass object does not conserve energy",
            entering * leaving
        );
    }

    /// The whole lobe against a closed-form prediction.
    ///
    /// A ray leaving this interface either reflected, carrying its full
    /// radiance, or refracted, carrying `1/eta^2` of it. So the expected
    /// throughput is `F + (1 - F)/eta^2` — and checking against that number
    /// tests three things at once that are usually only tested by eye: that
    /// the Fresnel split is right, that the compression is applied once, and
    /// that the PDF carries the discrete choice's probability.
    ///
    /// The residual gap is GGX's single-scattering energy loss, which grows
    /// with roughness and is what the energy-compensation tables exist to put
    /// back for the reflection-only lobes.
    #[test]
    fn throughput_matches_the_fresnel_weighted_prediction() {
        let eta = 1.5;
        // Low roughness, so the multiple-scattering deficit is small enough
        // that the prediction is the thing being tested rather than the
        // deficit.
        let lobe = glass(0.1);

        for degrees in [5.0f32, 30.0, 55.0, 75.0] {
            let wo = direction(degrees.to_radians());
            let f = fresnel::dielectric(wo.z, eta);
            let expected = f + (1.0 - f) / (eta * eta);
            let measured = mean_throughput(&lobe, wo);

            assert!(
                (measured - expected).abs() < 0.05,
                "{degrees} degrees: carried {measured:.4}, predicted {expected:.4} \
                 (Fresnel {f:.3})"
            );
        }
    }

    /// A thin shell has no interior, so there is no compression either way:
    /// light enters and leaves parallel and the factors never arise.
    #[test]
    fn a_thin_shell_has_unit_throughput() {
        let pane = glass(0.2).thin_walled(true);
        for degrees in [10.0f32, 45.0, 70.0] {
            let throughput = mean_throughput(&pane, direction(degrees.to_radians()));
            assert!(
                (0.85..=1.05).contains(&throughput),
                "{degrees} degrees: a pane carried {throughput:.4}"
            );
        }
    }

    /// A smooth interface is delta and NEE must skip it: an `eval` that
    /// returned a value there would double-count.
    #[test]
    fn a_smooth_interface_evaluates_to_zero_and_is_flagged_delta() {
        let lobe = glass(0.0);
        assert!(lobe.flags().contains(BsdfFlags::DELTA));
        assert_eq!(lobe.eval(direction(0.5), direction(2.0)), Spectrum::ZERO);
        assert_eq!(lobe.pdf(direction(0.5), direction(2.0)), 0.0);
    }

    #[test]
    fn nothing_produces_a_non_finite_value() {
        for roughness in [0.0f32, 0.05, 0.5, 1.0] {
            let lobe = glass(roughness);
            for degrees in [0.1f32, 45.0, 89.9] {
                let wo = direction(degrees.to_radians());
                for i in 0..128 {
                    let u = Vec2 {
                        x: (i as f32 + 0.5) / 128.0,
                        y: ((i * 31) % 128) as f32 / 128.0,
                    };
                    for uc in [0.0f32, 0.5, 0.999] {
                        if let Some(sample) = lobe.sample(wo, u, uc) {
                            assert!(sample.f.is_finite(), "{sample:?}");
                            assert!(sample.pdf.is_finite() && sample.pdf >= 0.0);
                            assert!(sample.wi.is_finite());
                        }
                    }
                }
            }
        }
    }

    /// The *flux* a rough interface passes on, which must be all of it.
    ///
    /// Deliberately not [`mean_throughput`]. That measures radiance, and
    /// radiance is not conserved across a refraction: it compresses by
    /// `1/eta^2` going in and expands by `eta^2` coming out, so a correct
    /// interface reads 0.44 from one side and 2.25 from the other. Undoing
    /// that factor with `sample.eta` converts the measurement back into a
    /// fraction of the flux that arrived, which *is* conserved — and which
    /// therefore has one right answer, 1, on both sides and at every angle.
    fn flux_albedo(lobe: &DielectricTransmission, wo: Vec3) -> f32 {
        const SAMPLES: usize = 16384;
        let mut total = 0.0;
        for i in 0..SAMPLES {
            let u = Vec2 {
                x: (i as f32 + 0.5) / SAMPLES as f32,
                y: ((i * 7919) % SAMPLES) as f32 / SAMPLES as f32,
            };
            let uc = ((i * 4099) % SAMPLES) as f32 / SAMPLES as f32;
            if let Some(sample) = lobe.sample(wo, u, uc) {
                if sample.pdf > 0.0 {
                    let weight =
                        sample.f.luminance() * shading::abs_cos_theta(sample.wi) / sample.pdf;
                    total += weight * sample.eta * sample.eta;
                }
            }
        }
        total / SAMPLES as f32
    }

    /// Two media of one index meet at an interface that is not there: nothing
    /// bends and nothing reflects, however rough the surface, because every
    /// microfacet has the same index on both sides. The microfacet model
    /// cannot say so â€” the refraction half vector `-(wo + eta wi)` is zero at
    /// `eta = 1` â€” and sampled nothing, which ended every path that met a
    /// liquid index-matched to its glass once path regularization had made
    /// the surface rough.
    #[test]
    fn an_index_matched_rough_interface_passes_light_straight_on() {
        let wo = direction(0.4);
        for inside in [false, true] {
            let lobe = DielectricTransmission::new(0.3, 1.0).from_inside(inside);
            let sample = lobe
                .sample(wo, Vec2 { x: 0.37, y: 0.61 }, 0.5)
                .expect("an index-matched interface sampled nothing");
            assert!(sample.is_transmission, "it reflected");
            assert!((sample.wi + wo).length() < 1e-4, "it bent the light: {:?}", sample.wi);
            let weight = sample.f.luminance() * shading::abs_cos_theta(sample.wi) / sample.pdf;
            assert!((weight - 1.0).abs() < 1e-3, "it passed on {weight}");
        }
    }

    /// A clear interface must pass on every photon that reaches it.
    ///
    /// Single-scattering GGX does not: a microfacet can send light into
    /// another facet, and the model has nowhere to put light that bounces
    /// twice, so it drops it. §2.3 makes correcting that mandatory, and the
    /// reflective lobes were corrected long before this one — by tables that
    /// only know about reflection, where this lobe loses energy out of both
    /// of its halves.
    ///
    /// The loss is worst where the interface is roughest and worst of all
    /// from inside, where total internal reflection turns light back into the
    /// microsurface for a second chance the model does not simulate: before
    /// the compensation, a fully rough diamond seen from inside passed on
    /// 0.42 of what reached it.
    #[test]
    fn a_rough_interface_passes_on_all_of_its_light() {
        let mut worst = (0.0f32, String::new());
        for &roughness in &[0.1f32, 0.3, 0.6, 0.9] {
            for &ior in &[1.1f32, 1.33, 1.5, 2.4] {
                for &inside in &[false, true] {
                    // A shell loses its energy to the same shadowing but gets
                    // it back from a different table, so it is a case of its
                    // own rather than a variation on the one above.
                    for &thin in &[false, true] {
                        for &degrees in &[10.0f32, 45.0, 75.0] {
                            let lobe = DielectricTransmission::new(roughness, ior)
                                .from_inside(inside)
                                .thin_walled(thin);
                            let wo = direction(degrees.to_radians());
                            let albedo = flux_albedo(&lobe, wo);
                            let error = (albedo - 1.0).abs();
                            if error > worst.0 {
                                worst = (
                                    error,
                                    format!(
                                        "roughness {roughness}, ior {ior}, {}{} at \
                                         {degrees} degrees: {albedo:.4}",
                                        if inside { "inside" } else { "outside" },
                                        if thin { ", thin" } else { "" }
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(
            worst.0 < 0.02,
            "an interface that absorbs nothing lost light: {}",
            worst.1
        );
    }
}
