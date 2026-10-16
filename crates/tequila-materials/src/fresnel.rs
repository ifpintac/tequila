//! Fresnel reflectance.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3 requires the **exact** Fresnel equations
//! rather than Schlick's approximation, because Schlick errs visibly at high
//! IOR — precisely the gem and dense-glass case this application exists to
//! render. Schlick appears here only inside the average-Fresnel approximations
//! used for energy compensation, where the error is second-order.

use tequila_core::math::sqr;
use tequila_core::spectrum::{spectrum, Spectrum};

/// Exact unpolarized Fresnel reflectance at a dielectric interface.
///
/// `cos_i` is the cosine of the incidence angle against the interface normal;
/// a negative value means the ray is arriving from inside the medium, and the
/// relative IOR is inverted accordingly. `eta` is `ior_transmitted /
/// ior_incident` for a ray arriving from outside.
///
/// Returns 1 on total internal reflection.
pub fn dielectric(cos_i: f32, eta: f32) -> f32 {
    let mut cos_i = cos_i.clamp(-1.0, 1.0);
    let mut eta = eta;

    if cos_i < 0.0 {
        // Arriving from inside: flip the interface.
        eta = 1.0 / eta;
        cos_i = -cos_i;
    }

    let sin2_i = (1.0 - cos_i * cos_i).max(0.0);
    let sin2_t = sin2_i / (eta * eta);
    if sin2_t >= 1.0 {
        return 1.0; // total internal reflection
    }
    let cos_t = (1.0 - sin2_t).max(0.0).sqrt();

    let r_parallel = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    let r_perpendicular = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    0.5 * (sqr(r_parallel) + sqr(r_perpendicular))
}

/// Exact unpolarized Fresnel reflectance at a conductor, from the complex
/// index of refraction `n - i*k`.
pub fn conductor_scalar(cos_i: f32, n: f32, k: f32) -> f32 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let cos2 = cos_i * cos_i;
    let sin2 = 1.0 - cos2;

    let n2 = n * n;
    let k2 = k * k;
    let t0 = n2 - k2 - sin2;
    let a2_plus_b2 = (t0 * t0 + 4.0 * n2 * k2).max(0.0).sqrt();
    let t1 = a2_plus_b2 + cos2;
    let a = (0.5 * (a2_plus_b2 + t0)).max(0.0).sqrt();
    let t2 = 2.0 * a * cos_i;
    let rs = (t1 - t2) / (t1 + t2);

    let t3 = cos2 * a2_plus_b2 + sqr(sin2);
    let t4 = t2 * sin2;
    let rp = rs * (t3 - t4) / (t3 + t4);

    0.5 * (rs + rp)
}

/// Per-channel conductor Fresnel.
#[inline]
pub fn conductor(cos_i: f32, n: Spectrum, k: Spectrum) -> Spectrum {
    spectrum(
        conductor_scalar(cos_i, n.r, k.r),
        conductor_scalar(cos_i, n.g, k.g),
        conductor_scalar(cos_i, n.b, k.b),
    )
}

/// Complex index of refraction for a conductor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComplexIor {
    pub n: Spectrum,
    pub k: Spectrum,
}

impl ComplexIor {
    /// Artist-friendly parameterization [Gulbrandsen 2014]: derive `(n, k)`
    /// from the reflectance at normal incidence and the reflectance at
    /// grazing ("edge tint"), which is how the Metal material's Color and
    /// Specular Color controls are defined (`docs/02` §2.3).
    ///
    /// This is invertible and stays inside physically realizable `(n, k)` for
    /// any inputs in `[0, 1)`, which is why it is preferred over letting users
    /// type complex IORs directly.
    pub fn from_reflectivity_and_edge_tint(reflectivity: Spectrum, edge_tint: Spectrum) -> Self {
        #[inline]
        fn channel(r: f32, g: f32) -> (f32, f32) {
            // Clamp just below 1: r = 1 is a perfect mirror with infinite n.
            let r = r.clamp(0.0, 0.999);
            let g = g.clamp(0.0, 1.0);
            let sqrt_r = r.sqrt();
            let n_min = (1.0 - r) / (1.0 + r);
            let n_max = (1.0 + sqrt_r) / (1.0 - sqrt_r);
            let n = g * n_min + (1.0 - g) * n_max;
            let k2 = (sqr(n + 1.0) * r - sqr(n - 1.0)) / (1.0 - r);
            (n, k2.max(0.0).sqrt())
        }
        let (nr, kr) = channel(reflectivity.r, edge_tint.r);
        let (ng, kg) = channel(reflectivity.g, edge_tint.g);
        let (nb, kb) = channel(reflectivity.b, edge_tint.b);
        ComplexIor {
            n: spectrum(nr, ng, nb),
            k: spectrum(kr, kg, kb),
        }
    }

    /// Reflectance at normal incidence.
    pub fn normal_incidence_reflectance(&self) -> Spectrum {
        conductor(1.0, self.n, self.k)
    }
}

/// Hemispherical-average Fresnel for a conductor.
///
/// Uses the closed-form average of Schlick's approximation,
/// `F_avg = F0 + (1 - F0) / 21`. The approximation is acceptable *here*
/// specifically because this quantity only ever scales the multiple-scattering
/// energy-compensation term, which is itself a small correction — an error of
/// a percent in `F_avg` is invisible, while the exact integral would need a
/// numerical quadrature per shading point.
#[inline]
pub fn conductor_average(f0: Spectrum) -> Spectrum {
    f0 + (Spectrum::ONE - f0) * (1.0 / 21.0)
}

/// Hemispherical-average Fresnel for a dielectric interface, from the rational
/// fit in [Kulla & Conty 2017]. Valid for `eta >= 1`; for `eta < 1` the
/// reciprocal relation `F_avg(1/eta) = 1 - (1 - F_avg(eta)) / eta^2` applies.
pub fn dielectric_average(eta: f32) -> f32 {
    if eta >= 1.0 {
        ((eta - 1.0) / (4.08567 + 1.00071 * eta)).clamp(0.0, 1.0)
    } else {
        let inv = 1.0 / eta.max(1e-4);
        let f = dielectric_average(inv);
        (1.0 - (1.0 - f) * eta * eta).clamp(0.0, 1.0)
    }
}

/// Named metal presets. Values are the complex IOR sampled at approximately
/// the sRGB primary wavelengths (600 / 550 / 450 nm) from the standard
/// refractive-index tables. RGB values are an approximation of a spectral
/// quantity; §7.4's spectral mode replaces them for dispersive work.
pub mod metals {
    use super::ComplexIor;
    use tequila_core::spectrum::spectrum;

    macro_rules! metal {
        ($name:ident, $doc:literal, [$nr:expr, $ng:expr, $nb:expr], [$kr:expr, $kg:expr, $kb:expr]) => {
            #[doc = $doc]
            pub const $name: ComplexIor = ComplexIor {
                n: spectrum($nr, $ng, $nb),
                k: spectrum($kr, $kg, $kb),
            };
        };
    }

    metal!(
        ALUMINIUM,
        "Aluminium",
        [1.345, 0.965, 0.617],
        [7.475, 6.400, 5.303]
    );
    metal!(
        SILVER,
        "Silver",
        [0.155, 0.116, 0.138],
        [4.820, 3.122, 2.146]
    );
    metal!(GOLD, "Gold", [0.143, 0.375, 1.442], [3.983, 2.386, 1.603]);
    metal!(
        COPPER,
        "Copper",
        [0.200, 0.924, 1.102],
        [3.912, 2.448, 2.303]
    );
    metal!(
        CHROMIUM,
        "Chromium",
        [3.134, 3.076, 2.353],
        [3.309, 3.334, 3.146]
    );
    metal!(IRON, "Iron", [2.911, 2.950, 2.585], [3.089, 2.932, 2.766]);
    metal!(
        NICKEL,
        "Nickel",
        [2.006, 1.822, 1.617],
        [3.851, 3.480, 3.075]
    );
    metal!(
        TITANIUM,
        "Titanium",
        [2.741, 2.542, 2.267],
        [3.812, 3.436, 3.033]
    );
    metal!(
        TUNGSTEN,
        "Tungsten",
        [3.638, 3.421, 3.099],
        [2.850, 2.741, 2.577]
    );
    metal!(
        PLATINUM,
        "Platinum",
        [2.376, 2.085, 1.845],
        [4.266, 3.715, 3.137]
    );

    /// Every preset with its display name, for the Metal material's Type
    /// dropdown (`docs/02` §3.1).
    pub const ALL: [(&str, ComplexIor); 10] = [
        ("Aluminium", ALUMINIUM),
        ("Silver", SILVER),
        ("Gold", GOLD),
        ("Copper", COPPER),
        ("Chromium", CHROMIUM),
        ("Iron", IRON),
        ("Nickel", NICKEL),
        ("Titanium", TITANIUM),
        ("Tungsten", TUNGSTEN),
        ("Platinum", PLATINUM),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dielectric_reflectance_at_normal_incidence_is_the_textbook_value() {
        // ((n1 - n2) / (n1 + n2))^2 = 0.04 for air/glass at n = 1.5.
        assert!((dielectric(1.0, 1.5) - 0.04).abs() < 1e-4);
        // Water.
        assert!((dielectric(1.0, 1.333) - 0.0204).abs() < 1e-3);
    }

    #[test]
    fn dielectric_reaches_one_at_grazing() {
        assert!(dielectric(1e-4, 1.5) > 0.99);
    }

    #[test]
    fn dielectric_detects_total_internal_reflection() {
        // Critical angle for n = 1.5 leaving glass is asin(1/1.5) ~ 41.8 deg.
        let cos_critical = (1.0f32 - (1.0 / 1.5f32).powi(2)).sqrt();
        // Just past critical (smaller cosine = larger angle).
        assert_eq!(dielectric(-(cos_critical * 0.9), 1.5), 1.0);
        // Just inside critical: below 1 but climbing.
        assert!(dielectric(-(cos_critical * 1.1), 1.5) < 1.0);
    }

    #[test]
    fn dielectric_is_symmetric_across_the_interface() {
        // Entering at cos_i and leaving at the matching cos_t must agree.
        let eta = 1.5;
        for cos_i in [0.2f32, 0.5, 0.8, 1.0] {
            let sin_t = (1.0 - cos_i * cos_i).sqrt() / eta;
            let cos_t = (1.0 - sin_t * sin_t).sqrt();
            let f_in = dielectric(cos_i, eta);
            let f_out = dielectric(-cos_t, eta);
            assert!((f_in - f_out).abs() < 1e-4, "{f_in} vs {f_out}");
        }
    }

    #[test]
    fn conductor_reflectance_is_high_and_bounded() {
        for (name, ior) in metals::ALL {
            let f0 = ior.normal_incidence_reflectance();
            for c in [f0.r, f0.g, f0.b] {
                assert!((0.0..=1.0).contains(&c), "{name}: F0 = {c}");
            }
            assert!(f0.luminance() > 0.4, "{name} is implausibly dark: {f0:?}");
            // Every metal approaches a perfect mirror at grazing incidence.
            let grazing = conductor(1e-4, ior.n, ior.k);
            assert!(grazing.luminance() > 0.97, "{name} grazing = {grazing:?}");
        }
    }

    #[test]
    fn gold_is_actually_gold_coloured() {
        let f0 = metals::GOLD.normal_incidence_reflectance();
        assert!(f0.r > f0.g && f0.g > f0.b, "gold F0 = {f0:?}");
    }

    /// The Gulbrandsen parameterization must round-trip: the `(n, k)` it
    /// derives from a reflectivity must reproduce that reflectivity at normal
    /// incidence, or the Metal material's Color control lies to the user.
    #[test]
    fn artist_parameterization_round_trips() {
        for r in [0.05f32, 0.3, 0.7, 0.95] {
            for g in [0.0f32, 0.5, 1.0] {
                let ior = ComplexIor::from_reflectivity_and_edge_tint(
                    Spectrum::splat(r),
                    Spectrum::splat(g),
                );
                let back = ior.normal_incidence_reflectance();
                assert!(
                    (back.g - r).abs() < 2e-3,
                    "r={r} g={g}: recovered {}",
                    back.g
                );
            }
        }
    }

    #[test]
    fn average_fresnel_is_bounded_and_monotonic() {
        let mut previous = 0.0;
        for eta in [1.0f32, 1.2, 1.5, 2.0, 2.42, 3.0] {
            let f = dielectric_average(eta);
            assert!((0.0..1.0).contains(&f), "eta {eta}: {f}");
            assert!(f >= previous, "not monotonic at eta {eta}");
            previous = f;
        }
        // Air/glass: roughly 9% of energy is reflected on average.
        assert!((dielectric_average(1.5) - 0.09).abs() < 0.02);
    }

    #[test]
    fn conductor_average_exceeds_normal_incidence() {
        let f0 = spectrum(0.9, 0.7, 0.4);
        let avg = conductor_average(f0);
        assert!(avg.r > f0.r && avg.g > f0.g && avg.b > f0.b);
        assert!(avg.max_component() <= 1.0);
    }
}
