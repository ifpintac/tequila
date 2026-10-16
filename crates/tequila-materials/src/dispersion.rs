//! Dispersion: glass that bends each colour by its own amount (`docs/01` §7.4).
//!
//! A path that meets a dispersive material is given one wavelength, and from
//! then on refracts as light of that wavelength does. Across many paths the
//! wavelengths cover the visible range and a white light splits into its
//! colours — a diamond's fire, a prism's rainbow.
//!
//! §7.4 asks for hero-wavelength sampling [Wilkie et al. 2014]: four
//! wavelengths per path, the three companions dropped at the first dispersive
//! refraction, where they would each go a different way. What is here is
//! that scheme after its first dispersive event, which is where it matters:
//! one wavelength, carried as the RGB its colour would register as, and
//! normalized so the wavelengths average to exact white. The difference is
//! colour noise before a path reaches a gem, and the spectral upsampling of
//! RGB surfaces [Jakob & Hanika 2019], which needs a precomputed table this
//! build does not carry; surfaces met after the split filter the path by
//! their RGB, which is exact for neutral ones.

use tequila_core::spectrum::Spectrum;
use std::sync::OnceLock;

/// The visible range sampled, in nanometres.
pub const SHORTEST: f32 = 380.0;
pub const LONGEST: f32 = 730.0;

/// The Fraunhofer lines the Abbe number is defined against: the helium d
/// line, where `specular_ior` is stated, and hydrogen F and C.
const D_LINE: f32 = 587.6;
const F_LINE: f32 = 486.1;
const C_LINE: f32 = 656.3;

/// The index of refraction at `wavelength` nanometres of a glass whose index
/// is `ior` at the d line and whose Abbe number is `abbe`, by the two-term
/// Cauchy law `n = A + B / λ²` fitted to both. An Abbe number of zero is no
/// dispersion.
pub fn ior_at(ior: f32, abbe: f32, wavelength: f32) -> f32 {
    if abbe <= 0.0 {
        return ior;
    }
    // V = (n_d - 1) / (n_F - n_C), and n_F - n_C = B (1/λF² - 1/λC²).
    let b = (ior - 1.0) / (abbe * (1.0 / (F_LINE * F_LINE) - 1.0 / (C_LINE * C_LINE)));
    let a = ior - b / (D_LINE * D_LINE);
    (a + b / (wavelength * wavelength)).max(1.0)
}

/// The CIE 1931 colour matching functions, by the multi-lobe Gaussian fit of
/// Wyman, Sloan and Shirley (2013).
pub fn cie_xyz(wavelength: f32) -> [f32; 3] {
    let g = |x: f32, mu: f32, lo: f32, hi: f32| {
        let t = (x - mu) / if x < mu { lo } else { hi };
        (-0.5 * t * t).exp()
    };
    let l = wavelength;
    let x = 1.056 * g(l, 599.8, 37.9, 31.0) + 0.362 * g(l, 442.0, 16.0, 26.7) - 0.065 * g(l, 501.1, 20.4, 26.2);
    let y = 0.821 * g(l, 568.8, 46.9, 40.5) + 0.286 * g(l, 530.9, 16.3, 31.1);
    let z = 1.217 * g(l, 437.0, 11.8, 36.0) + 0.681 * g(l, 459.0, 26.0, 13.8);
    [x, y, z]
}

/// Linear sRGB of a wavelength, clipped to what sRGB can show.
fn raw_rgb(wavelength: f32) -> [f32; 3] {
    let [x, y, z] = cie_xyz(wavelength);
    [
        (3.2406 * x - 1.5372 * y - 0.4986 * z).max(0.0),
        (-0.9689 * x + 1.8758 * y + 0.0415 * z).max(0.0),
        (0.0557 * x - 0.2040 * y + 1.0570 * z).max(0.0),
    ]
}

/// Per-channel scale that makes the wavelengths average to white.
fn normalization() -> [f32; 3] {
    static SCALE: OnceLock<[f32; 3]> = OnceLock::new();
    *SCALE.get_or_init(|| {
        let n = 4096;
        let mut sum = [0.0f64; 3];
        for i in 0..n {
            let wavelength = SHORTEST + (LONGEST - SHORTEST) * (i as f32 + 0.5) / n as f32;
            let rgb = raw_rgb(wavelength);
            for c in 0..3 {
                sum[c] += rgb[c] as f64;
            }
        }
        [
            (n as f64 / sum[0]) as f32,
            (n as f64 / sum[1]) as f32,
            (n as f64 / sum[2]) as f32,
        ]
    })
}

/// The colour a path carrying `wavelength` stands for, as a throughput
/// weight: over wavelengths drawn uniformly across the visible range it
/// averages to exactly white, so dispersion splits light without tinting it.
pub fn weight(wavelength: f32) -> Spectrum {
    let rgb = raw_rgb(wavelength);
    let scale = normalization();
    Spectrum::from_linear_srgb(tequila_core::math::vec3(
        rgb[0] * scale[0],
        rgb[1] * scale[1],
        rgb[2] * scale[2],
    ))
}

/// A wavelength, uniformly across the visible range, from `u` in [0, 1).
pub fn sample(u: f32) -> f32 {
    SHORTEST + (LONGEST - SHORTEST) * u.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index at the three lines reproduces the Abbe number it was given,
    /// and the stated index at the d line.
    #[test]
    fn the_index_follows_the_abbe_number() {
        for (ior, abbe) in [(1.5168f32, 64.17f32), (2.417, 55.0), (1.77, 72.0)] {
            let (d, f, c) = (ior_at(ior, abbe, D_LINE), ior_at(ior, abbe, F_LINE), ior_at(ior, abbe, C_LINE));
            assert!((d - ior).abs() < 1e-5, "{d} at the d line for {ior}");
            let measured = (d - 1.0) / (f - c);
            assert!((measured - abbe).abs() < 0.05 * abbe, "an Abbe number of {abbe} came out {measured}");
            assert!(ior_at(ior, abbe, 420.0) > ior_at(ior, abbe, 680.0), "blue must bend more than red");
        }
        assert_eq!(ior_at(1.5, 0.0, 420.0), 1.5, "zero is no dispersion");
    }

    /// Averaged over the visible range, the weights are white — dispersion
    /// splits light and must not tint it — and each end of the range is the
    /// colour it should be.
    #[test]
    fn the_wavelengths_average_to_white() {
        let n = 20000;
        let mut total = Spectrum::ZERO;
        for i in 0..n {
            total = total + weight(sample((i as f32 + 0.5) / n as f32));
        }
        let mean = (total * (1.0 / n as f32)).to_linear_srgb();
        for c in [mean.x, mean.y, mean.z] {
            assert!((c - 1.0).abs() < 0.01, "the wavelengths average to {mean:?}");
        }
        let blue = weight(450.0).to_linear_srgb();
        let red = weight(650.0).to_linear_srgb();
        assert!(blue.z > blue.x && blue.z > blue.y, "450 nm is {blue:?}");
        assert!(red.x > red.y && red.x > red.z, "650 nm is {red:?}");
    }
}
