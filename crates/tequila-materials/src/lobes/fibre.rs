//! A fibre's scattering: §4.4's Fuzz grown as real fibres, each a thin
//! dielectric cylinder that light reflects off, passes through and bounces
//! around inside.
//!
//! The model is Chiang et al.'s [Chiang, Bitterli, Tappan & Burley 2016, *A
//! practical and controllable hair and fur model for production path
//! tracing*], as pbrt-v3 published it and as Cycles' Principled Hair BSDF
//! has it (`bsdf_principled_hair_chiang.h`, Apache-2.0), from which the
//! constants and the order of operations here are ported - so a fibre is the
//! same fibre in both of Tequila's engines. Cycles' own fit maps the
//! roughness to the lobes' variance and scale, and a colour to the
//! absorption that gives a fibre of that colour once light has bounced
//! around a mass of them.
//!
//! # The four lobes
//!
//! Light meeting a fibre at `h` across its width (-1 at one edge, 0 straight
//! on, 1 at the other) reflects off the surface (R), passes through (TT),
//! reflects once inside and comes back out on the near side (TRT), and the
//! rest leaves after more bounces, spread around the fibre (TRRT and on).
//! Each lobe is a longitudinal spread about the cone of directions a smooth
//! cylinder sends the light into, times an azimuthal one about the
//! direction the lobe's path round the cross-section turns it, times what
//! the surface's Fresnel and the fibre's absorption leave of it.
//!
//! # The frame
//!
//! `x` runs along the fibre and `z`, the shading normal, faces the viewer
//! square to it; `y` completes the frame. The model reads angles from `x`
//! (how far a direction is off square to the fibre) and around it (`atan2(z,
//! y)`), and only differences of the second matter, so which way `y` points
//! is free as long as `h` is measured along it.
//!
//! # The cosine
//!
//! The model's value includes the cosine to the fibre; the integrator
//! multiplies by the cosine to the shading normal instead, so `eval`
//! divides by that one, as pbrt does. The two cancel exactly - the
//! integrator's cosine is this one, read from the same `wi.z` - and leave
//! the model's.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use crate::fresnel;
use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use std::f32::consts::{PI, TAU};

/// A fibre's index of refraction: polyester's, wool's and cotton's are all
/// about this, and it is Cycles' Principled Hair's default.
pub const FIBRE_IOR: f32 = 1.55;

/// One fibre, met at one point across its width.
#[derive(Clone, Copy, Debug)]
pub struct Fibre {
    /// Absorption across the fibre's diameter, per channel.
    sigma: Spectrum,
    /// The R lobe's longitudinal variance; TT's is a quarter of it, TRT's
    /// and the rest's four times.
    v: f32,
    /// The azimuthal lobes' logistic scale.
    s: f32,
    /// Where across the fibre the ray met it, -1 to 1 along `y`.
    h: f32,
    eta: f32,
    /// What the colour was, kept for the albedo guide.
    colour: Spectrum,
}

/// Chiang et al.'s fit (their eq. 9), as Cycles takes it: the absorption
/// that makes a mass of fibres look `colour`, at azimuthal roughness
/// `roughness`.
pub fn absorption_for(colour: Spectrum, roughness: f32) -> Spectrum {
    let x = roughness;
    let scale = (((((0.245 * x) + 5.574) * x - 10.73) * x + 2.532) * x - 0.215) * x + 5.969;
    // A white fibre absorbs nothing; black is clamped short of infinity, as
    // the logarithm of zero would be.
    let channel = |c: f32| {
        let a = c.clamp(1e-4, 1.0).ln() / scale;
        a * a
    };
    Spectrum { r: channel(colour.r), g: channel(colour.g), b: channel(colour.b) }
}

impl Fibre {
    /// A fibre of `colour` (the colour a mass of them shows, each channel 0
    /// to 1), with `roughness` 0 to 1 along and around it alike, met at `h`.
    pub fn new(colour: Spectrum, roughness: f32, h: f32) -> Fibre {
        let roughness = roughness.clamp(0.001, 1.0);
        let colour = colour.clamp(0.0, 1.0);
        // Cycles' mapping from a roughness to the variance and scale of the
        // lobes (`bsdf_hair_chiang_setup`).
        let r = roughness;
        let v = (0.726 * r + 0.812 * r * r + 3.7 * r.powi(20)).powi(2);
        let s = (0.265 * r + 1.194 * r * r + 5.372 * r.powi(22)) * (PI / 8.0).sqrt();
        Fibre {
            sigma: absorption_for(colour, roughness),
            v,
            s,
            h: h.clamp(-1.0, 1.0),
            eta: FIBRE_IOR,
            colour,
        }
    }

    /// The angles and attenuations every evaluation from `wo` shares.
    fn setup(&self, wo: Vec3) -> Option<Setup> {
        let sin_theta_o = wo.x.clamp(-1.0, 1.0);
        let cos_theta_o = (1.0 - sin_theta_o * sin_theta_o).max(0.0).sqrt();
        // Looking straight down the fibre there is nothing across it to
        // see.
        if cos_theta_o < 1e-4 {
            return None;
        }
        let phi_o = wo.z.atan2(wo.y);
        let sin_theta_t = sin_theta_o / self.eta;
        let cos_theta_t = (1.0 - sin_theta_t * sin_theta_t).max(0.0).sqrt();
        let sin_gamma_o = self.h;
        let cos_gamma_o = (1.0 - sin_gamma_o * sin_gamma_o).max(0.0).sqrt();
        let gamma_o = sin_gamma_o.clamp(-1.0, 1.0).asin();
        let sin_gamma_t = (sin_gamma_o * cos_theta_o / (self.eta * self.eta - sin_theta_o * sin_theta_o).sqrt()).clamp(-1.0, 1.0);
        let cos_gamma_t = (1.0 - sin_gamma_t * sin_gamma_t).max(0.0).sqrt();
        let gamma_t = sin_gamma_t.asin();
        // One pass through the fibre, along the refracted chord.
        let through = (self.sigma * (-2.0 * cos_gamma_t / cos_theta_t.max(1e-6))).exp();
        let f = fresnel::dielectric(cos_theta_o * cos_gamma_o, self.eta);
        // R, TT, TRT, and everything after, as Cycles' `hair_attenuation`.
        let mut ap = [Spectrum::ZERO; 4];
        ap[0] = Spectrum::splat(f);
        ap[1] = through * ((1.0 - f) * (1.0 - f));
        ap[2] = ap[1] * through * f;
        let tf = through * f;
        ap[3] = ap[2]
            * Spectrum {
                r: tf.r / (1.0 - tf.r).max(1e-6),
                g: tf.g / (1.0 - tf.g).max(1e-6),
                b: tf.b / (1.0 - tf.b).max(1e-6),
            };
        let mut energy = ap.map(|a| a.luminance().max(0.0));
        let total: f32 = energy.iter().sum();
        if !(total > 0.0) {
            return None;
        }
        for e in &mut energy {
            *e /= total;
        }
        Some(Setup { sin_theta_o, cos_theta_o, phi_o, gamma_o, gamma_t, ap, energy })
    }

    /// The model's value and its density, as luminance, for `wi` - Cycles'
    /// `F` and `F_energy`.
    fn evaluate(&self, setup: &Setup, wi: Vec3) -> (Spectrum, f32) {
        let sin_theta_i = wi.x.clamp(-1.0, 1.0);
        let cos_theta_i = (1.0 - sin_theta_i * sin_theta_i).max(0.0).sqrt();
        let phi_i = wi.z.atan2(wi.y);
        let phi = phi_i - setup.phi_o;
        let mut value = Spectrum::ZERO;
        let mut density = 0.0;
        for p in 0..3 {
            let mp = longitudinal(sin_theta_i, cos_theta_i, setup.sin_theta_o, setup.cos_theta_o, self.variance(p));
            let np = azimuthal(phi, p, self.s, setup.gamma_o, setup.gamma_t);
            value += setup.ap[p] * (mp * np);
            density += setup.energy[p] * mp * np;
        }
        let mp = longitudinal(sin_theta_i, cos_theta_i, setup.sin_theta_o, setup.cos_theta_o, self.variance(3));
        let np = 1.0 / TAU;
        value += setup.ap[3] * (mp * np);
        density += setup.energy[3] * mp * np;
        (value, density)
    }

    fn variance(&self, p: usize) -> f32 {
        match p {
            0 => self.v,
            1 => 0.25 * self.v,
            _ => 4.0 * self.v,
        }
    }

    /// Everything the fibre sends on from `wo`, over every direction: the
    /// four lobes' attenuations summed, the longitudinal and azimuthal
    /// spreads each integrating to one. Exactly what a furnace measures.
    pub fn scattered(&self, wo: Vec3) -> Spectrum {
        self.setup(wo).map_or(Spectrum::ZERO, |s| s.ap[0] + s.ap[1] + s.ap[2] + s.ap[3])
    }
}

/// What [`Fibre::setup`] works out once per outgoing direction.
struct Setup {
    sin_theta_o: f32,
    cos_theta_o: f32,
    phi_o: f32,
    gamma_o: f32,
    gamma_t: f32,
    ap: [Spectrum; 4],
    energy: [f32; 4],
}

impl Bsdf for Fibre {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        let Some(setup) = self.setup(wo) else {
            return Spectrum::ZERO;
        };
        let cos = wi.z.abs();
        if cos < 1e-7 {
            return Spectrum::ZERO;
        }
        self.evaluate(&setup, wi).0 * (1.0 / cos)
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        let Some(setup) = self.setup(wo) else {
            return 0.0;
        };
        self.evaluate(&setup, wi).1
    }

    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample> {
        let setup = self.setup(wo)?;
        // Which lobe, by its share of the energy; what is left of `uc` then
        // places the direction along the fibre.
        let mut pick = uc;
        let mut p = 0;
        while p < 3 && pick >= setup.energy[p] {
            pick -= setup.energy[p];
            p += 1;
        }
        let along = (pick / setup.energy[p].max(1e-12)).clamp(1e-5, 1.0);
        let v = self.variance(p);
        // About the cone a smooth fibre would send this lobe's light into
        // (Cycles' sampling of `Mp`), at an azimuth `u.y` round it.
        let fac = 1.0 + v * (along + (1.0 - along) * (-2.0 / v).exp()).ln();
        let sin_theta_i = (-fac * setup.sin_theta_o
            + (1.0 - fac * fac).max(0.0).sqrt() * (TAU * u.y).cos() * setup.cos_theta_o)
            .clamp(-1.0, 1.0);
        let cos_theta_i = (1.0 - sin_theta_i * sin_theta_i).max(0.0).sqrt();
        let phi = if p < 3 {
            delta_phi(p, setup.gamma_o, setup.gamma_t) + sample_trimmed_logistic(u.x, self.s)
        } else {
            TAU * u.x
        };
        let phi_i = setup.phi_o + phi;
        let wi = Vec3 { x: sin_theta_i, y: cos_theta_i * phi_i.cos(), z: cos_theta_i * phi_i.sin() };
        let (value, pdf) = self.evaluate(&setup, wi);
        let cos = wi.z.abs();
        if !(pdf > 0.0) || cos < 1e-7 {
            return None;
        }
        Some(BsdfSample {
            wi,
            f: value * (1.0 / cos),
            pdf,
            is_delta: false,
            // By the side of the shading normal it leaves on, which is what
            // the integrator offsets the next ray by: an R lobe off a
            // fibre's edge can leave behind it, and TT straight through.
            is_transmission: wi.z < 0.0,
            eta: 1.0,
            flags: BsdfFlags::REFLECTION | BsdfFlags::TRANSMISSION | BsdfFlags::GLOSSY,
        })
    }

    /// Everything the fibre sends on, toward the viewer or past it: for
    /// Russian roulette and the denoiser's guide, at most one.
    fn albedo(&self, wo: Vec3) -> Spectrum {
        if self.setup(wo).is_none() {
            return self.colour;
        }
        self.scattered(wo).clamp(0.0, 1.0)
    }

    fn flags(&self) -> BsdfFlags {
        BsdfFlags::REFLECTION | BsdfFlags::TRANSMISSION | BsdfFlags::GLOSSY
    }
}

/// How far round the fibre lobe `p`'s path turns the light: Cycles'
/// `delta_phi`.
#[inline]
fn delta_phi(p: usize, gamma_o: f32, gamma_t: f32) -> f32 {
    let p = p as f32;
    2.0 * p * gamma_t - 2.0 * gamma_o + p * PI
}

/// `a` wrapped into `[-pi, pi]`.
#[inline]
fn wrap(a: f32) -> f32 {
    (a + PI) - TAU * ((a + PI) / TAU).floor() - PI
}

#[inline]
fn logistic(x: f32, s: f32) -> f32 {
    let v = (-x.abs() / s).exp();
    v / (s * (1.0 + v) * (1.0 + v))
}

#[inline]
fn logistic_cdf(x: f32, s: f32) -> f32 {
    let arg = -x / s;
    if arg > 88.0 {
        return 0.0;
    }
    1.0 / (1.0 + arg.exp())
}

/// The logistic distribution kept to `[-pi, pi]` and renormalized there.
#[inline]
fn trimmed_logistic(x: f32, s: f32) -> f32 {
    let scale = 1.0 - 2.0 * logistic_cdf(-PI, s);
    if scale > 0.0 {
        logistic(x, s) / scale
    } else {
        0.0
    }
}

#[inline]
fn sample_trimmed_logistic(u: f32, s: f32) -> f32 {
    let below = logistic_cdf(-PI, s);
    let x = -s * (1.0 / (u * (1.0 - 2.0 * below) + below) - 1.0).ln();
    x.clamp(-PI, PI)
}

/// `Np`: the azimuthal spread of lobe `p` about where its path turns the
/// light.
#[inline]
fn azimuthal(phi: f32, p: usize, s: f32, gamma_o: f32, gamma_t: f32) -> f32 {
    trimmed_logistic(wrap(phi - delta_phi(p, gamma_o, gamma_t)), s)
}

/// The modified Bessel function of the first kind, order zero, by its
/// series (Cycles' `bessel_I0`).
fn bessel_i0(x: f32) -> f32 {
    let x = x * x;
    let mut value = 1.0 + 0.25 * x;
    let mut power = x * x;
    let mut factorial_squared = 1u64;
    let mut four_power = 16.0f32;
    for i in 2..10u64 {
        factorial_squared *= i * i;
        let next = value + power / (four_power * factorial_squared as f32);
        if next == value {
            return value;
        }
        value = next;
        power *= x;
        four_power *= 4.0;
    }
    value
}

fn log_bessel_i0(x: f32) -> f32 {
    if x > 12.0 {
        return x + 0.5 * (1.0 / (8.0 * x) - (TAU).ln() - x.ln());
    }
    bessel_i0(x).ln()
}

/// `Mp`: the longitudinal spread about the cone of mirror directions, a
/// von Mises-Fisher lobe of variance `v` (d'Eon et al.'s, as Cycles has
/// it). Normalized over the sphere with the cosine to the fibre's normal
/// plane.
fn longitudinal(sin_theta_i: f32, cos_theta_i: f32, sin_theta_o: f32, cos_theta_o: f32, v: f32) -> f32 {
    let inv_v = 1.0 / v;
    let cos_arg = cos_theta_i * cos_theta_o * inv_v;
    let sin_arg = sin_theta_i * sin_theta_o * inv_v;
    if v <= 0.1 {
        // In logarithms, where the exponentials alone would overflow.
        return (log_bessel_i0(cos_arg) - sin_arg - inv_v + 0.6931 + (0.5 * inv_v).ln()).exp();
    }
    ((-sin_arg).exp() * bessel_i0(cos_arg)) / ((inv_v).sinh() * 2.0 * v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_core::sobol;

    /// A direction `theta` off square to the fibre, `phi` round it.
    fn direction(theta: f32, phi: f32) -> Vec3 {
        vec3(theta.sin(), theta.cos() * phi.cos(), theta.cos() * phi.sin())
    }

    /// The viewer as the integrator places it: in the `xz` plane, `z`
    /// facing it.
    fn viewer(theta: f32) -> Vec3 {
        direction(theta, std::f32::consts::FRAC_PI_2)
    }

    /// The mean of `f |cos| / pdf` over the fibre's own samples, and of
    /// `f |cos|` over uniform directions: both estimate everything it sends
    /// on from `wo`.
    fn sent_on(fibre: &Fibre, wo: Vec3) -> (Spectrum, Spectrum) {
        const SAMPLES: u32 = 1 << 16;
        let (mut sampled, mut uniform) = ([0.0f64; 3], [0.0f64; 3]);
        for i in 0..SAMPLES {
            let u = Vec2 { x: sobol::sample(i, 0, 7), y: sobol::sample(i, 1, 7) };
            let uc = sobol::sample(i, 2, 7);
            if let Some(s) = fibre.sample(wo, u, uc) {
                let w = s.f * (s.wi.z.abs() / s.pdf);
                assert!(w.is_finite(), "weight {w:?}");
                sampled[0] += w.r as f64;
                sampled[1] += w.g as f64;
                sampled[2] += w.b as f64;
            }
            // Uniform over the sphere, density 1 / 4 pi.
            let z = 1.0 - 2.0 * sobol::sample(i, 3, 11);
            let r = (1.0 - z * z).max(0.0).sqrt();
            let a = TAU * sobol::sample(i, 4, 11);
            let wi = vec3(r * a.cos(), r * a.sin(), z);
            let w = fibre.eval(wo, wi) * (wi.z.abs() * 4.0 * PI);
            uniform[0] += w.r as f64;
            uniform[1] += w.g as f64;
            uniform[2] += w.b as f64;
        }
        let mean = |t: [f64; 3]| Spectrum { r: (t[0] / SAMPLES as f64) as f32, g: (t[1] / SAMPLES as f64) as f32, b: (t[2] / SAMPLES as f64) as f32 };
        (mean(sampled), mean(uniform))
    }

    /// The white furnace: a fibre that absorbs nothing sends on everything
    /// it meets, from any angle and anywhere across it, at any roughness -
    /// estimated by its own sampling and by plain uniform directions alike,
    /// which is `eval`, `pdf` and `sample` agreeing about the energy.
    #[test]
    fn a_white_fibre_sends_on_all_it_meets() {
        for roughness in [0.15, 0.3, 0.6, 0.9] {
            for h in [-0.9, -0.3, 0.0, 0.5, 0.95] {
                for theta in [-1.1, -0.4, 0.0, 0.7] {
                    let fibre = Fibre::new(Spectrum::ONE, roughness, h);
                    let wo = viewer(theta);
                    assert!((fibre.scattered(wo).g - 1.0).abs() < 1e-4, "attenuations sum to {:?}", fibre.scattered(wo));
                    let (sampled, uniform) = sent_on(&fibre, wo);
                    assert!((sampled.g - 1.0).abs() < 0.01, "roughness {roughness}, h {h}, theta {theta}: sampled {sampled:?}");
                    // Uniform directions find a tight lobe rarely, so they
                    // are held to it only where it is not tight.
                    if roughness >= 0.3 {
                        assert!((uniform.g - 1.0).abs() < 0.04, "roughness {roughness}, h {h}, theta {theta}: uniform {uniform:?}");
                    }
                }
            }
        }
    }

    /// A coloured fibre sends on what its attenuations say, channel by
    /// channel: the sampling's weights are the value over the density, not
    /// an approximation of it.
    #[test]
    fn a_coloured_fibre_sends_on_its_attenuations() {
        let fibre = Fibre::new(Spectrum { r: 0.8, g: 0.3, b: 0.05 }, 0.4, 0.2);
        let wo = viewer(0.3);
        let expected = fibre.scattered(wo);
        let (sampled, uniform) = sent_on(&fibre, wo);
        for (got, want) in [(sampled.r, expected.r), (sampled.g, expected.g), (sampled.b, expected.b)] {
            assert!((got - want).abs() < 0.01 * want.max(0.05), "sampled {sampled:?} against {expected:?}");
        }
        for (got, want) in [(uniform.r, expected.r), (uniform.g, expected.g), (uniform.b, expected.b)] {
            assert!((got - want).abs() < 0.04 * want.max(0.05), "uniform {uniform:?} against {expected:?}");
        }
        assert!(expected.r > expected.g && expected.g > expected.b, "a red fibre should send on most red: {expected:?}");
    }

    /// The density `sample` reports is `pdf` at the same direction, and its
    /// value is `eval` there.
    #[test]
    fn sample_agrees_with_eval_and_pdf() {
        let fibre = Fibre::new(Spectrum { r: 0.6, g: 0.5, b: 0.4 }, 0.35, -0.4);
        let wo = viewer(0.5);
        let mut checked = 0;
        for i in 0..512 {
            let u = Vec2 { x: sobol::sample(i, 0, 3), y: sobol::sample(i, 1, 3) };
            if let Some(s) = fibre.sample(wo, u, sobol::sample(i, 2, 3)) {
                let pdf = fibre.pdf(wo, s.wi);
                let f = fibre.eval(wo, s.wi);
                assert!((s.pdf - pdf).abs() <= 1e-4 * pdf.max(1.0), "{} against {pdf}", s.pdf);
                assert!((s.f.g - f.g).abs() <= 1e-4 * f.g.max(1.0));
                checked += 1;
            }
        }
        assert!(checked > 500);
    }

    /// The model's turn of the light agrees with the geometry it stands for.
    /// Met at `h` along `y`, the fibre's surface there faces `n`, and a
    /// smooth, nearly mirror fibre reflects the viewer about it: the R lobe
    /// peaks there and not at the mirror image about the other side.
    #[test]
    fn the_reflection_leaves_by_the_surface_it_met() {
        let h = 0.6f32;
        let fibre = Fibre::new(Spectrum::splat(0.01), 0.1, h);
        // Square to the fibre, so the reflection stays in its normal plane.
        let wo = viewer(0.0);
        // The point met: `h` of the way to the edge along `y`, on the
        // viewer's side.
        let n = vec3(0.0, h, (1.0 - h * h).sqrt());
        let mirror = n * (2.0 * n.dot(wo)) - wo;
        let wrong = vec3(0.0, -mirror.y, mirror.z);
        let at = |w: Vec3| fibre.eval(wo, w).g * w.z.abs();
        assert!(at(mirror) > 50.0 * at(wrong), "mirror {} against the other side {}", at(mirror), at(wrong));
    }

    /// A black fibre sends on its surface's reflection, and next to none of
    /// what goes inside it.
    #[test]
    fn a_black_fibre_mostly_reflects() {
        let fibre = Fibre::new(Spectrum::ZERO, 0.3, 0.0);
        let sent = fibre.scattered(viewer(0.0));
        let f0 = fresnel::dielectric(1.0, FIBRE_IOR);
        assert!(sent.g >= f0 && sent.g < f0 + 0.01, "{sent:?} against {f0}");
    }
}
