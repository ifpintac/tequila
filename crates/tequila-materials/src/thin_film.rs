//! Thin-film interference over a specular interface.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3: "Airy-summation interference over the
//! specular Fresnel term; spectral when possible, 3-wavelength fallback in RGB
//! mode". This is the fallback. `docs/01` §7.1 wants hero-wavelength spectral
//! sampling before the colours are strictly right, and until that lands the
//! film is evaluated at one wavelength per channel.
//!
//! # What the film does
//!
//! Light reaching a coated surface splits: some reflects off the top of the
//! film, some crosses it, reflects off the base, and comes back out having
//! travelled an extra `2 * n * d * cos(theta)`. The two add as waves rather
//! than as intensities, so they reinforce at wavelengths where that extra path
//! is a whole number of them and cancel where it is a half. The wavelengths
//! that survive are what the eye reads as the colour of a soap bubble, an oil
//! film, or anodised titanium.
//!
//! That is why the whole thing has to be done in *amplitudes*. Reflectance —
//! the number the rest of this crate passes around — is an intensity, and
//! intensities have thrown away the phase that the interference is made of.
//!
//! # The three-wavelength limitation, stated plainly
//!
//! One sample per channel is correct only while the reflectance varies slowly
//! across each channel's band. Past roughly 1000 nm of film the fringes are
//! packed closer together than the three samples can follow, and the colour
//! begins to alias: it keeps moving and stays saturated where a real film's
//! colours would have washed out to a pale silver as its fringes averaged
//! away within the eye's own bands. §2.2 allows thicknesses up to 2000 nm, so
//! the top half of that range renders a plausible iridescence rather than the
//! right one. Spectral rendering is the fix, not more RGB samples.

use crate::fresnel::ComplexIor;
use tequila_core::spectrum::{spectrum, Spectrum};

/// The wavelength each channel is evaluated at, in nanometres.
///
/// Roughly where the sRGB primaries sit: the peak of each of the eye's cone
/// responses as the display drives it, rather than the ends of the visible
/// range. Using the extremes instead exaggerates the hue sweep, because it
/// measures the film at wavelengths the channels barely respond to.
const WAVELENGTHS: [f32; 3] = [600.0, 550.0, 450.0];

/// A complex number, for amplitude reflection coefficients.
///
/// Local and minimal on purpose: this is the only place in the renderer that
/// needs one, and a general complex type would invite the rest of the crate to
/// start carrying phases it has no use for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    const ONE: Complex = Complex { re: 1.0, im: 0.0 };

    #[inline]
    fn new(re: f32, im: f32) -> Complex {
        Complex { re, im }
    }

    #[inline]
    fn real(re: f32) -> Complex {
        Complex { re, im: 0.0 }
    }

    /// `e^(i * theta)`.
    #[inline]
    fn unit(theta: f32) -> Complex {
        Complex {
            re: theta.cos(),
            im: theta.sin(),
        }
    }

    #[inline]
    fn add(self, other: Complex) -> Complex {
        Complex::new(self.re + other.re, self.im + other.im)
    }

    #[inline]
    fn sub(self, other: Complex) -> Complex {
        Complex::new(self.re - other.re, self.im - other.im)
    }

    #[inline]
    fn mul(self, other: Complex) -> Complex {
        Complex::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }

    #[inline]
    fn div(self, other: Complex) -> Complex {
        let denominator = other.re * other.re + other.im * other.im;
        if denominator <= 0.0 {
            return Complex::real(0.0);
        }
        Complex::new(
            (self.re * other.re + self.im * other.im) / denominator,
            (self.im * other.re - self.re * other.im) / denominator,
        )
    }

    /// Squared magnitude, which is the intensity an amplitude carries.
    #[inline]
    fn norm_squared(self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    /// The principal square root.
    #[inline]
    fn sqrt(self) -> Complex {
        if self.im == 0.0 && self.re >= 0.0 {
            return Complex::real(self.re.sqrt());
        }
        let modulus = (self.re * self.re + self.im * self.im).sqrt();
        let re = (0.5 * (modulus + self.re)).max(0.0).sqrt();
        let im = (0.5 * (modulus - self.re)).max(0.0).sqrt();
        Complex::new(re, if self.im < 0.0 { -im } else { im })
    }
}

/// The two polarizations, kept apart until the very end.
///
/// A film's fringes land at different angles for each, and averaging the
/// indices before the interference rather than the intensities after it
/// smears the fringes into a grey that never shows a colour at all.
#[derive(Clone, Copy, Debug)]
struct Polarized {
    s: Complex,
    p: Complex,
}

/// Amplitude reflection at an interface between two media, given the cosines
/// on each side. `eta` values may be complex; for a dielectric the imaginary
/// part is zero and the answers come out real.
fn interface(eta_i: Complex, cos_i: Complex, eta_t: Complex, cos_t: Complex) -> Polarized {
    let (ei_ci, et_ct) = (eta_i.mul(cos_i), eta_t.mul(cos_t));
    let (et_ci, ei_ct) = (eta_t.mul(cos_i), eta_i.mul(cos_t));
    Polarized {
        s: ei_ci.sub(et_ct).div(ei_ci.add(et_ct)),
        p: et_ci.sub(ei_ct).div(et_ci.add(ei_ct)),
    }
}

/// `cos(theta)` on the far side of an interface, by Snell's law.
#[inline]
fn refracted_cosine(eta_i: Complex, sin_i: f32, eta_t: Complex) -> Complex {
    let sin_t = Complex::real(sin_i).mul(eta_i).div(eta_t);
    Complex::ONE.sub(sin_t.mul(sin_t)).sqrt()
}

/// Reflectance of a film of `thickness` nanometres and index `film_ior`, on a
/// base of complex index `base`, seen from air at `cos_theta`.
///
/// One wavelength, one polarization pair, the whole Airy summation of every
/// bounce inside the film — which has a closed form because the bounces are a
/// geometric series.
fn airy(cos_theta: f32, film_ior: f32, base: Complex, thickness: f32, wavelength: f32) -> f32 {
    let cos_theta = cos_theta.clamp(0.0, 1.0);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();

    let air = Complex::ONE;
    let film = Complex::real(film_ior.max(1.0));
    let cos_film = refracted_cosine(air, sin_theta, film);

    let top = interface(air, Complex::real(cos_theta), film, cos_film);

    // Snell again, into the base. Measured from *air*, not from inside the
    // film: `n * sin(theta)` is conserved right across the stack, so the
    // angle in the base depends only on the angle light arrived at and on the
    // base itself — the film in between does not enter into it. Refracting a
    // second time out of the film instead multiplies by the film's index
    // twice over, and the base appears to be in total internal reflection at
    // angles where nothing of the sort is happening. It reads as a coated
    // surface that turns into a perfect mirror well before grazing.
    let cos_base = refracted_cosine(air, sin_theta, base);
    let bottom = interface(film, cos_film, base, cos_base);

    // Phase picked up crossing the film and coming back. `cos_film` is
    // complex only for an absorbing film, which this one is not, so its real
    // part is the whole of it.
    let phase = 4.0 * std::f32::consts::PI * film_ior * thickness * cos_film.re / wavelength;
    let shift = Complex::unit(-phase);

    let combine = |r_top: Complex, r_bottom: Complex| -> f32 {
        let delayed = r_bottom.mul(shift);
        let numerator = r_top.add(delayed);
        let denominator = Complex::ONE.add(r_top.mul(delayed));
        numerator.div(denominator).norm_squared()
    };

    // Unpolarized light is an equal mix, so the two intensities average.
    // Clamped because this is an intensity and the arithmetic that produced
    // it is not exact: at grazing every term in the stack is 1 and the
    // rounding lands a few ulps the wrong side of it.
    (0.5 * (combine(top.s, bottom.s) + combine(top.p, bottom.p))).clamp(0.0, 1.0)
}

/// Reflectance of a film over a **dielectric** base — the soap bubble, the oil
/// slick, the coating on a lens.
pub fn over_dielectric(cos_theta: f32, film_ior: f32, base_ior: f32, thickness: f32) -> Spectrum {
    let base = Complex::real(base_ior.max(1.0));
    per_channel(cos_theta, film_ior, base, thickness)
}

/// Reflectance of a film over a **conductor** — anodised titanium, heat
/// colours on steel, oil on chrome.
///
/// The base index is complex here, and that matters for more than absorption:
/// a metal shifts the phase of what it reflects by an amount that depends on
/// wavelength, so the fringes sit somewhere else than they would over glass.
/// A film over gold is not a film over glass with a gold tint.
pub fn over_conductor(
    cos_theta: f32,
    film_ior: f32,
    base: ComplexIor,
    thickness: f32,
) -> Spectrum {
    let channel = |n: f32, k: f32, wavelength: f32| {
        airy(
            cos_theta,
            film_ior,
            // Negative imaginary part: the `n - i*k` convention the rest of
            // the crate is written in.
            Complex::new(n, -k),
            thickness,
            wavelength,
        )
    };
    spectrum(
        channel(base.n.r, base.k.r, WAVELENGTHS[0]),
        channel(base.n.g, base.k.g, WAVELENGTHS[1]),
        channel(base.n.b, base.k.b, WAVELENGTHS[2]),
    )
}

/// A film over a specular interface, as §2.2 parameterizes one.
///
/// Carried by the reflective lobes rather than applied over them, because a
/// film does not sit *on* a Fresnel term — it replaces it. The reflectance of
/// a coated surface is not the reflectance of the bare one adjusted; it is a
/// different function of angle and wavelength, and at a quarter wave of the
/// right index it is zero where the bare surface reflects four percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Film {
    /// §2.2's `thin_film_ior`, 1.0 to 3.0.
    pub ior: f32,
    /// §2.2's `thin_film_thickness`, in nanometres, 0 to 2000.
    pub thickness: f32,
    /// §2.2's `thin_film_weight`: how much of the coated answer to use.
    ///
    /// A blend between two reflectances rather than between two thicknesses,
    /// so that turning the slider down fades the iridescence out evenly
    /// instead of sweeping its colour towards the bare surface on the way.
    pub weight: f32,
}

impl Film {
    /// True when the film would do nothing, so the caller can keep the plain
    /// Fresnel path and its scalar arithmetic.
    #[inline]
    pub fn is_negligible(&self) -> bool {
        self.weight <= 1.0e-4 || self.thickness <= 0.0
    }

    /// Reflectance over a dielectric base, blended by [`Self::weight`].
    pub fn over_dielectric(&self, cos_theta: f32, base_ior: f32) -> Spectrum {
        let bare = Spectrum::splat(crate::fresnel::dielectric(cos_theta, base_ior));
        if self.is_negligible() {
            return bare;
        }
        let coated = over_dielectric(cos_theta, self.ior, base_ior, self.thickness);
        bare.lerp(coated, self.weight.clamp(0.0, 1.0))
    }

    /// Reflectance over a conductor base, blended by [`Self::weight`].
    pub fn over_conductor(&self, cos_theta: f32, base: ComplexIor) -> Spectrum {
        let bare = crate::fresnel::conductor(cos_theta, base.n, base.k);
        if self.is_negligible() {
            return bare;
        }
        let coated = over_conductor(cos_theta, self.ior, base, self.thickness);
        bare.lerp(coated, self.weight.clamp(0.0, 1.0))
    }
}

fn per_channel(cos_theta: f32, film_ior: f32, base: Complex, thickness: f32) -> Spectrum {
    spectrum(
        airy(cos_theta, film_ior, base, thickness, WAVELENGTHS[0]),
        airy(cos_theta, film_ior, base, thickness, WAVELENGTHS[1]),
        airy(cos_theta, film_ior, base, thickness, WAVELENGTHS[2]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fresnel;

    /// No film, no interference: the stack has to collapse to the plain
    /// Fresnel of the base.
    ///
    /// This is the test that catches a sign error in either interface, because
    /// at zero thickness the two reflections add in phase and the Airy sum has
    /// a closed form that must equal the single-interface answer exactly.
    #[test]
    fn zero_thickness_is_the_bare_interface() {
        for &cos_theta in &[0.05f32, 0.3, 0.7, 1.0] {
            for &base_ior in &[1.33f32, 1.5, 2.4] {
                let film = over_dielectric(cos_theta, 1.4, base_ior, 0.0);
                let bare = fresnel::dielectric(cos_theta, base_ior);
                assert!(
                    (film.g - bare).abs() < 1e-4,
                    "cos {cos_theta}, ior {base_ior}: film {} against bare {bare}",
                    film.g
                );
            }
        }
    }

    /// The same, over metal, where the base reflection is complex and the
    /// cancellation is the one most likely to be got wrong.
    #[test]
    fn zero_thickness_over_metal_is_the_bare_metal() {
        for &(name, ior) in fresnel::metals::ALL.iter() {
            for &cos_theta in &[0.1f32, 0.5, 1.0] {
                let film = over_conductor(cos_theta, 1.4, ior, 0.0);
                let bare = fresnel::conductor(cos_theta, ior.n, ior.k);
                for (f, b) in [(film.r, bare.r), (film.g, bare.g), (film.b, bare.b)] {
                    assert!(
                        (f - b).abs() < 1e-3,
                        "{name} at cos {cos_theta}: film {f} against bare {b}"
                    );
                }
            }
        }
    }

    /// A film cannot reflect more than arrives. Interference moves energy
    /// between wavelengths; it does not make any.
    #[test]
    fn reflectance_stays_within_unity() {
        for thickness in [0.0f32, 80.0, 250.0, 550.0, 1300.0, 2000.0] {
            for &cos_theta in &[0.02f32, 0.25, 0.6, 1.0] {
                for film_ior in [1.0f32, 1.4, 2.0, 3.0] {
                    let over_glass = over_dielectric(cos_theta, film_ior, 1.5, thickness);
                    let over_metal =
                        over_conductor(cos_theta, film_ior, fresnel::metals::ALUMINIUM, thickness);
                    for value in [
                        over_glass.r, over_glass.g, over_glass.b, over_metal.r, over_metal.g,
                        over_metal.b,
                    ] {
                        assert!(
                            (0.0..=1.0).contains(&value),
                            "thickness {thickness}, cos {cos_theta}, film ior {film_ior}: \
                             reflectance {value}"
                        );
                    }
                }
            }
        }
    }

    /// The whole point: the film has to have a colour, and that colour has to
    /// change as it gets thicker.
    ///
    /// A film that reflected the same in all three channels would be a grey
    /// coating, which is exactly what this looks like if the phase term is
    /// dropped or the wavelengths are not actually used.
    #[test]
    fn thickness_sweeps_the_hue() {
        let saturation = |s: Spectrum| {
            let high = s.r.max(s.g).max(s.b);
            let low = s.r.min(s.g).min(s.b);
            high - low
        };

        let mut coloured = 0;
        let mut previous = over_dielectric(1.0, 1.4, 1.5, 100.0);
        let mut moved = 0;
        for thickness in [100.0f32, 200.0, 300.0, 400.0, 500.0, 600.0] {
            let value = over_dielectric(1.0, 1.4, 1.5, thickness);
            if saturation(value) > 0.01 {
                coloured += 1;
            }
            if (value.r - previous.r).abs() + (value.b - previous.b).abs() > 0.01 {
                moved += 1;
            }
            previous = value;
        }
        assert!(coloured >= 4, "a thin film came out grey at most thicknesses");
        assert!(moved >= 4, "the colour did not change with thickness");
    }

    /// Grazing incidence is a mirror whatever the coating: every Fresnel term
    /// in the stack goes to 1 there, and a film of 1 over a base of 1 is 1.
    #[test]
    fn grazing_incidence_reflects_everything() {
        let value = over_dielectric(1.0e-4, 1.4, 1.5, 420.0);
        for channel in [value.r, value.g, value.b] {
            assert!(
                channel > 0.99,
                "a coated surface stopped mirroring at grazing: {channel}"
            );
        }
    }

    /// A quarter-wave film of the *right* index is an anti-reflection coating,
    /// which is the one case with a textbook answer to check against.
    ///
    /// With `n_film = sqrt(n_base)` and a thickness of a quarter wavelength in
    /// the film, the two reflections cancel exactly and the surface reflects
    /// nothing at that wavelength. For glass that is an index of 1.2247 and,
    /// at 550 nm, a thickness of 112.3 nm — which is what real lens coatings
    /// are, give or take the materials that actually exist.
    #[test]
    fn a_quarter_wave_coating_cancels_its_own_reflection() {
        let base_ior = 1.5f32;
        let film_ior = base_ior.sqrt();
        let thickness = 550.0 / (4.0 * film_ior);
        let value = over_dielectric(1.0, film_ior, base_ior, thickness);
        assert!(
            value.g < 1e-4,
            "a quarter-wave coating still reflected {} at its design wavelength",
            value.g
        );
        // And the bare interface it is cancelling really does reflect.
        assert!(fresnel::dielectric(1.0, base_ior) > 0.03);
    }
}
