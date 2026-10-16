//! The OpenPBR `fuzz` layer: sheen.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3 asks for "Zeltner sheen (or Conty-Kulla
//! charlie sheen) with an energy-compensation LUT". This is the Zeltner one
//! [Zeltner, Burley & Chiang 2022]: the light a layer of fibres standing on
//! the surface sends back, multiple scattering and all, fitted as linearly
//! transformed cosines. It is the lobe Cycles' Principled BSDF uses, with
//! Cycles' own fitted tables (`sheen_ltc.rs`), so a velvet is the same velvet
//! in both of Tequila's engines.
//!
//! # Why not the charlie distribution any more
//!
//! This file used to be Conty-Kulla's charlie distribution [Estevez & Kulla
//! 2017] with Ashikhmin-Shirley visibility. It is a single-scattering lobe,
//! and a fibre layer is not: at high roughness most of what it returns has
//! bounced between fibres several times. Charlie therefore hardly brightened
//! as the fuzz roughened, where real pile does a great deal. On a crimson
//! velvet sphere under the library's studio light the two engines measured
//!
//! | fuzz roughness | 0.3 | 0.6 | 0.9 |
//! |---|---|---|---|
//! | Tequila, charlie | 107 | 118 | 124 |
//! | Cycles | 80 | 109 | 153 |
//!
//! and at 0.3 the two pictures correlated 0.26: charlie spread a tight fuzz
//! over the whole face, where the fibre layer keeps it to the rim. Cycles
//! renders Tequila's finals, so this renderer agreeing with it is what
//! makes a thumbnail a promise about the final image.
//!
//! # The lobe
//!
//! A cosine lobe, transformed by the matrix `[[a, 0, b], [0, a, 0], [0, 0,
//! 1]]` in the frame where the viewer leans towards `+x`, times the layer's
//! directional albedo. `a` narrows the lobe and `b` tips it back towards the
//! viewer — fibres retro-reflect. All three come from the table by the
//! viewing angle and the roughness. Tight fuzz seen face-on has no valid fit
//! there, and Cycles drops the closure; so does this, because a tight pile
//! seen end-on really does return nothing.
//!
//! Sampling is exact: a cosine sample pushed through the same matrix has the
//! lobe's own distribution as its density.
//!
//! # Layering
//!
//! The layer beneath sees `1 - weight * max(colour) * albedo` of the light,
//! which is Cycles' rule. The fibres reflect `colour` of what they catch, so
//! a black fuzz catches nothing to reflect and hides nothing either — an
//! earlier, colourless rule had a black-sheened velvet a fifth darker in
//! this renderer than in Cycles.

use super::sheen_ltc::{SIZE, TABLE};
use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use tequila_core::math::{shading, vec3, Vec2, Vec3};
use tequila_core::sampling::sample_cosine_hemisphere;
use tequila_core::spectrum::Spectrum;
use std::f32::consts::FRAC_1_PI;

/// Zeltner sheen over an arbitrary substrate.
#[derive(Clone, Copy, Debug)]
pub struct Sheen {
    pub color: Spectrum,
    /// OpenPBR `fuzz_roughness`. Clamped away from zero, as Cycles clamps
    /// it: the table's first row is a pile so tight nothing fits it.
    pub roughness: f32,
    /// OpenPBR `fuzz_weight`.
    pub weight: f32,
}

/// The fitted transform and the albedo, for one view and roughness.
#[derive(Clone, Copy, Debug)]
struct Fit {
    a: f32,
    b: f32,
    albedo: f32,
}

impl Sheen {
    pub fn new(color: Spectrum, roughness: f32, weight: f32) -> Sheen {
        Sheen {
            color,
            roughness: roughness.clamp(1e-3, 1.0),
            weight: weight.clamp(0.0, 1.0),
        }
    }

    /// The fit seen from `cos_o`, or nothing where there is no valid one —
    /// Cycles' test, so both engines drop the same closures.
    fn fit(&self, cos_o: f32) -> Option<Fit> {
        let fit = Fit {
            a: lookup(0, cos_o, self.roughness),
            b: lookup(1, cos_o, self.roughness),
            albedo: lookup(2, cos_o, self.roughness),
        };
        (fit.a.abs() >= 1e-5 && fit.albedo >= 1e-5).then_some(fit)
    }

    /// The layer's directional albedo seen from `cos_o`, before its colour
    /// and weight.
    fn layer_albedo(&self, cos_o: f32) -> f32 {
        self.fit(cos_o.abs()).map_or(0.0, |fit| fit.albedo)
    }

    /// The transformed cosine's density at `local`, a direction in the fit's
    /// frame.
    #[inline]
    fn density(fit: Fit, local: Vec3) -> f32 {
        let len_sqr = sqr(fit.a * local.x + fit.b * local.z) + sqr(fit.a * local.y) + sqr(local.z);
        if local.z <= 0.0 || len_sqr <= 0.0 {
            return 0.0;
        }
        FRAC_1_PI * local.z * sqr(fit.a / len_sqr)
    }

    /// Cosine-weighted hemispherical average of the directional albedo,
    /// with the colour and weight.
    ///
    /// What sits beneath this layer needs it for the light that gets past the
    /// fuzz, scatters back up, and meets the underside of it — the same
    /// quantity [`crate::lobes::DielectricSpecular::hemispherical_albedo`]
    /// provides for the coat.
    pub fn hemispherical_albedo(&self) -> f32 {
        (self.weight * self.color.max_component() * average_albedo(self.roughness)).clamp(0.0, 1.0)
    }

    /// How much of the light reaching this layer from `cos_o` passes through
    /// to whatever is beneath it.
    ///
    /// §2.1 has each layer attenuate what is under it, and sheen is the
    /// topmost. Without this a velvet's base would be fully lit *and* the
    /// sheen added on top, which is the classic too-bright cloth.
    pub fn transmittance(&self, cos_o: f32) -> f32 {
        (1.0 - self.weight * self.color.max_component() * self.layer_albedo(cos_o)).clamp(0.0, 1.0)
    }
}

impl Bsdf for Sheen {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        if self.weight <= 0.0 || !shading::same_hemisphere(wo, wi) {
            return Spectrum::ZERO;
        }
        let (wo, wi) = (upper(wo), upper(wi));
        if wo.z <= 1e-6 || wi.z <= 1e-6 {
            return Spectrum::ZERO;
        }
        let Some(fit) = self.fit(wo.z) else {
            return Spectrum::ZERO;
        };
        // The table's value is the lobe times the cosine, as Cycles keeps
        // its closures; this trait's is the lobe alone.
        let lobe = fit.albedo * Sheen::density(fit, to_fit_frame(wo, wi)) / wi.z;
        self.color * (self.weight * lobe)
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if self.weight <= 0.0 || !shading::same_hemisphere(wo, wi) {
            return 0.0;
        }
        let (wo, wi) = (upper(wo), upper(wi));
        match self.fit(wo.z) {
            Some(fit) if wo.z > 1e-6 => Sheen::density(fit, to_fit_frame(wo, wi)),
            _ => 0.0,
        }
    }

    fn sample(&self, wo: Vec3, u: Vec2, _uc: f32) -> Option<BsdfSample> {
        if self.weight <= 0.0 {
            return None;
        }
        let up = upper(wo);
        if up.z <= 1e-6 {
            return None;
        }
        let fit = self.fit(up.z)?;
        // A cosine sample through the transform, scaled by `a` so that a
        // tight fit cannot divide by a small number.
        let d = sample_cosine_hemisphere(u);
        let local = vec3(d.x - d.z * fit.b, d.y, d.z * fit.a).try_normalize()?;
        if local.z <= 1e-6 {
            return None;
        }
        let mut wi = from_fit_frame(up, local);
        if wo.z < 0.0 {
            wi.z = -wi.z;
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
            flags: self.flags(),
        })
    }

    fn albedo(&self, wo: Vec3) -> Spectrum {
        self.color * (self.weight * self.layer_albedo(shading::abs_cos_theta(wo)))
    }

    fn flags(&self) -> BsdfFlags {
        BsdfFlags::REFLECTION | BsdfFlags::GLOSSY
    }
}

#[inline]
fn sqr(x: f32) -> f32 {
    x * x
}

/// `w` mirrored into the upper hemisphere, where the fit lives.
#[inline]
fn upper(w: Vec3) -> Vec3 {
    vec3(w.x, w.y, w.z.abs())
}

/// The azimuth of `wo`, as a cosine and sine; any, seen face-on, where every
/// azimuth is alike and the fit's `b` is zero.
#[inline]
fn azimuth(wo: Vec3) -> (f32, f32) {
    let len = (wo.x * wo.x + wo.y * wo.y).sqrt();
    if len < 1e-7 {
        (1.0, 0.0)
    } else {
        (wo.x / len, wo.y / len)
    }
}

/// `w` in the fit's frame: the normal up, the viewer leaning towards `+x`.
#[inline]
fn to_fit_frame(wo: Vec3, w: Vec3) -> Vec3 {
    let (c, s) = azimuth(wo);
    vec3(c * w.x + s * w.y, -s * w.x + c * w.y, w.z)
}

/// The inverse of [`to_fit_frame`].
#[inline]
fn from_fit_frame(wo: Vec3, w: Vec3) -> Vec3 {
    let (c, s) = azimuth(wo);
    vec3(c * w.x - s * w.y, s * w.x + c * w.y, w.z)
}

/// One of the three tables at a view and roughness, bilinear on the uniform
/// grid exactly as Cycles' `lookup_table_read_2D` reads it.
fn lookup(table: usize, cos_o: f32, roughness: f32) -> f32 {
    let cell = |t: f32| {
        let x = t.clamp(0.0, 1.0) * (SIZE - 1) as f32;
        let i = (x as usize).min(SIZE - 1);
        (i, (i + 1).min(SIZE - 1), x - i as f32)
    };
    let (x0, x1, tx) = cell(cos_o);
    let (y0, y1, ty) = cell(roughness);
    let at = |row: usize, column: usize| TABLE[table * SIZE * SIZE + row * SIZE + column];
    let row = |r: usize| at(r, x0) * (1.0 - tx) + at(r, x1) * tx;
    row(y0) * (1.0 - ty) + row(y1) * ty
}

/// `2 * integral of E(mu) mu dmu` per roughness row, by the midpoint rule
/// over the same interpolation the lobe reads.
fn average_albedo(roughness: f32) -> f32 {
    static ROWS: std::sync::OnceLock<[f32; SIZE]> = std::sync::OnceLock::new();
    let rows = ROWS.get_or_init(|| {
        const STEPS: usize = 256;
        std::array::from_fn(|r| {
            let roughness = r as f32 / (SIZE - 1) as f32;
            let total: f32 = (0..STEPS)
                .map(|i| {
                    let mu = (i as f32 + 0.5) / STEPS as f32;
                    let fit_albedo = Sheen::new(Spectrum::ONE, roughness, 1.0).layer_albedo(mu);
                    fit_albedo * 2.0 * mu
                })
                .sum();
            (total / STEPS as f32).clamp(0.0, 1.0)
        })
    });
    let y = roughness.clamp(0.0, 1.0) * (SIZE - 1) as f32;
    let i = (y as usize).min(SIZE - 1);
    let j = (i + 1).min(SIZE - 1);
    let t = y - i as f32;
    rows[i] * (1.0 - t) + rows[j] * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn velvet() -> Sheen {
        Sheen::new(Spectrum::ONE, 0.3, 1.0)
    }

    fn direction(theta: f32) -> Vec3 {
        vec3(theta.sin(), 0.0, theta.cos())
    }

    /// The hemispherical integral of a lobe, done the slow and obvious way:
    /// both angles, a fine grid, no sampling and no table.
    fn integrate_directly(f: impl Fn(Vec3) -> f32) -> f32 {
        const THETA: usize = 512;
        const PHI: usize = 256;
        let d_theta = std::f32::consts::FRAC_PI_2 / THETA as f32;
        let d_phi = std::f32::consts::TAU / PHI as f32;
        let mut total = 0.0f64;
        for ti in 0..THETA {
            let theta = (ti as f32 + 0.5) * d_theta;
            let (sin_t, cos_t) = theta.sin_cos();
            for pi in 0..PHI {
                let phi = (pi as f32 + 0.5) * d_phi;
                let wi = vec3(sin_t * phi.cos(), sin_t * phi.sin(), cos_t);
                total += (f(wi) * cos_t * sin_t * d_theta * d_phi) as f64;
            }
        }
        total as f32
    }

    /// The table is read where Cycles reads it: its corners are its first
    /// and last entries, and between grid points it is the average of the
    /// two either side.
    #[test]
    fn the_table_is_read_as_cycles_reads_it() {
        assert_eq!(lookup(2, 1.0, 1.0), TABLE[3 * SIZE * SIZE - 1]);
        assert_eq!(lookup(0, 0.0, 0.0), TABLE[0]);
        let between = lookup(2, 0.5 / (SIZE - 1) as f32, 1.0);
        let (first, second) = (TABLE[2 * SIZE * SIZE + 31 * SIZE], TABLE[2 * SIZE * SIZE + 31 * SIZE + 1]);
        assert!((between - (first + second) / 2.0).abs() < 1e-6, "{between} against {first} and {second}");
    }

    /// Velvet is dark seen end on and bright towards the silhouette, and the
    /// rougher its pile, the more it returns — the two things charlie's
    /// single scattering got wrong.
    #[test]
    fn the_pile_is_bright_at_the_rim_and_brighter_for_being_rough() {
        for roughness in [0.3f32, 0.6, 0.9] {
            let sheen = Sheen::new(Spectrum::ONE, roughness, 1.0);
            let face = sheen.albedo(direction(0.0)).g;
            let rim = sheen.albedo(direction(80f32.to_radians())).g;
            assert!(rim > 2.0 * face, "roughness {roughness}: rim {rim:.3}, face {face:.3}");
        }
        let at = |roughness: f32| Sheen::new(Spectrum::ONE, roughness, 1.0).hemispherical_albedo();
        assert!(at(0.9) > 1.5 * at(0.3), "rough pile {:.3} against tight {:.3}", at(0.9), at(0.3));
    }

    /// Sampling is the lobe's own distribution: importance sampling it gives
    /// the same reflected total as integrating it directly. A frame turned
    /// the wrong way, or a transform inverted, fails here.
    #[test]
    fn sampling_agrees_with_the_lobe() {
        for roughness in [0.2f32, 0.5, 0.9] {
            for degrees in [20.0f32, 60.0, 82.0] {
                // Off the x axis, so the azimuth's frame is exercised.
                let theta = degrees.to_radians();
                let wo = vec3(theta.sin() * 0.6, theta.sin() * 0.8, theta.cos());
                let sheen = Sheen::new(Spectrum::ONE, roughness, 1.0);
                let direct = integrate_directly(|wi| sheen.eval(wo, wi).g);
                const N: usize = 128;
                let mut sum = 0.0f64;
                for i in 0..N {
                    for j in 0..N {
                        let u = Vec2 { x: (i as f32 + 0.5) / N as f32, y: (j as f32 + 0.5) / N as f32 };
                        if let Some(s) = sheen.sample(wo, u, 0.5) {
                            assert!((s.pdf - sheen.pdf(wo, s.wi)).abs() <= 1e-4 * s.pdf.max(1.0));
                            sum += (s.f.g * s.wi.z / s.pdf) as f64;
                        }
                    }
                }
                let sampled = (sum / (N * N) as f64) as f32;
                assert!(
                    (sampled - direct).abs() <= 0.02 * direct.max(0.05),
                    "roughness {roughness} at {degrees} degrees: sampled {sampled:.4}, integrated {direct:.4}"
                );
            }
        }
    }

    /// The lobe must not reflect more light than reaches it, and must
    /// reflect no more than its table says it does.
    #[test]
    fn the_lobe_never_reflects_more_than_arrives() {
        for &roughness in &[0.001f32, 0.05, 0.1, 0.3, 0.6, 1.0] {
            for &degrees in &[5.0f32, 45.0, 75.0, 85.0, 88.0, 89.5] {
                let lobe = Sheen::new(Spectrum::ONE, roughness, 1.0);
                let wo = direction(degrees.to_radians());
                let reflected = integrate_directly(|wi| lobe.eval(wo, wi).g);
                let table = lobe.albedo(wo).g;
                assert!(
                    reflected <= 1.0 && reflected <= table * 1.02 + 1e-4,
                    "roughness {roughness} at {degrees} degrees: reflected {reflected:.4}, table {table:.4}"
                );
            }
        }
    }

    /// Sheen plus whatever it lets through must not exceed what arrived.
    ///
    /// [`Sheen::transmittance`] is the other half of the albedo — §2.1 has
    /// each layer attenuate what is beneath it — so the two have to add up.
    /// If the albedo were under-reported the pair would sum past one, which
    /// is a fabric that emits.
    #[test]
    fn sheen_and_what_it_passes_on_do_not_exceed_one() {
        for &roughness in &[0.05f32, 0.3, 1.0] {
            for &weight in &[0.5f32, 1.0] {
                for colour in [Spectrum::ONE, tequila_core::spectrum::spectrum(0.9, 0.2, 0.1)] {
                    for &degrees in &[5.0f32, 45.0, 88.0] {
                        let lobe = Sheen::new(colour, roughness, weight);
                        let wo = direction(degrees.to_radians());
                        let through = lobe.transmittance(wo.z);
                        for channel in 0..3 {
                            let reflected = integrate_directly(|wi| {
                                let f = lobe.eval(wo, wi);
                                [f.r, f.g, f.b][channel]
                            });
                            assert!(
                                reflected + through <= 1.0 + 0.02,
                                "roughness {roughness}, weight {weight} at {degrees} \
                                 degrees: reflected {reflected:.4} plus {through:.4} \
                                 through is more than arrived"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_albedo_never_exceeds_one() {
        for roughness in [0.02, 0.1, 0.3, 0.6, 1.0] {
            let sheen = Sheen::new(Spectrum::ONE, roughness, 1.0);
            for degrees in [1.0f32, 15.0, 45.0, 75.0, 89.0] {
                let albedo = sheen.albedo(direction(degrees.to_radians())).luminance();
                assert!(
                    (0.0..=1.0).contains(&albedo),
                    "roughness {roughness}, {degrees} degrees: albedo {albedo}"
                );
            }
        }
    }

    /// §2.1: each layer attenuates what is beneath it. Together the sheen's
    /// own albedo and what it passes through must not exceed the light that
    /// arrived.
    #[test]
    fn what_passes_through_plus_what_is_reflected_conserves_energy() {
        let sheen = velvet();
        for degrees in [1.0f32, 30.0, 60.0, 85.0] {
            let wo = direction(degrees.to_radians());
            let reflected = sheen.albedo(wo).luminance();
            let through = sheen.transmittance(wo.z);
            assert!(
                reflected + through <= 1.0 + 1e-4,
                "{degrees} degrees: {reflected:.4} reflected + {through:.4} through"
            );
        }
    }

    /// A black fuzz reflects nothing, so it hides nothing: Cycles' rule, and
    /// the one a black-sheened velvet rendered a fifth darker without.
    #[test]
    fn a_black_fuzz_hides_nothing() {
        let sheen = Sheen::new(Spectrum::ZERO, 0.9, 1.0);
        for degrees in [0.0f32, 45.0, 85.0] {
            assert_eq!(sheen.transmittance(direction(degrees.to_radians()).z), 1.0);
        }
        assert_eq!(sheen.hemispherical_albedo(), 0.0);
    }

    /// A zero weight is a layer that is not there.
    #[test]
    fn zero_weight_scatters_nothing_and_hides_nothing() {
        let sheen = Sheen::new(Spectrum::ONE, 0.3, 0.0);
        let wo = direction(0.5);
        assert!(sheen.sample(wo, Vec2::splat(0.5), 0.5).is_none());
        assert_eq!(sheen.eval(wo, direction(0.7)), Spectrum::ZERO);
        assert!((sheen.transmittance(wo.z) - 1.0).abs() < 1e-6);
    }

    /// Sheen reflects; it must never scatter through the surface.
    #[test]
    fn nothing_is_scattered_into_the_far_hemisphere() {
        let sheen = velvet();
        let wo = direction(1.2);
        assert_eq!(sheen.eval(wo, vec3(0.0, 0.0, -1.0)), Spectrum::ZERO);
        assert_eq!(sheen.pdf(wo, vec3(0.3, 0.0, -0.95)), 0.0);

        let mut sampled = 0;
        for i in 0..64 {
            let u = Vec2 {
                x: (i as f32 + 0.5) / 64.0,
                y: ((i * 7) % 64) as f32 / 64.0,
            };
            if let Some(sample) = sheen.sample(wo, u, 0.5) {
                assert!(sample.wi.z > 0.0, "sampled through the surface: {sample:?}");
                sampled += 1;
            }
        }
        assert!(sampled > 0);
    }

    /// Roughness zero would be a pile no fit exists for, so it is clamped
    /// rather than allowed to produce NaNs.
    #[test]
    fn an_impossible_roughness_is_clamped_not_propagated() {
        let sheen = Sheen::new(Spectrum::ONE, 0.0, 1.0);
        assert!(sheen.roughness > 0.0);
        for degrees in [5.0f32, 60.0, 89.0] {
            let value = sheen.eval(direction(degrees.to_radians()), direction(0.9)).luminance();
            assert!(value.is_finite(), "{value}");
        }
    }
}
