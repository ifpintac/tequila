//! Radiometric values and the working colour space.
//!
//! `docs/01-RENDERER-CORE.md` §7.1: all internal computation is scene-linear
//! in **ACEScg (AP1 primaries)**. sRGB-encoded values never appear inside the
//! renderer. `Spectrum` is the type that carries that invariant — it is an
//! RGB triple *in the rendering space*, and the only ways to build one from
//! user-facing colour go through an explicit input transform.
//!
//! The type is deliberately distinct from [`Vec3`] so that "an RGB colour the
//! user typed" and "a radiance value" cannot be confused at a call site.

use crate::math::{vec3, Vec3};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub};

/// A radiometric quantity in the ACEScg rendering space.
///
/// Spectral mode (§7.4) will carry wavelengths alongside these coefficients;
/// the type is kept opaque so that change does not ripple through the
/// integrator.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Spectrum {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[inline]
pub const fn spectrum(r: f32, g: f32, b: f32) -> Spectrum {
    Spectrum { r, g, b }
}

impl Spectrum {
    pub const ZERO: Self = spectrum(0.0, 0.0, 0.0);
    pub const ONE: Self = spectrum(1.0, 1.0, 1.0);

    #[inline]
    pub const fn splat(v: f32) -> Self {
        spectrum(v, v, v)
    }

    /// Build from coefficients already in the rendering space.
    #[inline]
    pub const fn from_rendering_rgb(v: Vec3) -> Self {
        spectrum(v.x, v.y, v.z)
    }

    /// Build from a linear sRGB (Rec.709 primaries) triple, as read from a
    /// texture after its transfer function has been removed, or as typed into
    /// a colour picker.
    #[inline]
    pub fn from_linear_srgb(v: Vec3) -> Self {
        Self::from_rendering_rgb(srgb_to_acescg(v))
    }

    /// Build from an sRGB-*encoded* triple in `[0,1]`, i.e. a colour picker
    /// value or an 8-bit texel. Applies the sRGB EOTF and then the primaries
    /// conversion.
    #[inline]
    pub fn from_srgb_encoded(v: Vec3) -> Self {
        Self::from_linear_srgb(vec3(srgb_eotf(v.x), srgb_eotf(v.y), srgb_eotf(v.z)))
    }

    #[inline]
    pub const fn to_rendering_rgb(self) -> Vec3 {
        vec3(self.r, self.g, self.b)
    }

    #[inline]
    pub fn to_linear_srgb(self) -> Vec3 {
        acescg_to_srgb(self.to_rendering_rgb())
    }

    /// Relative luminance in the rendering space. Used for Russian roulette,
    /// light importance, firefly detection, and adaptive sampling — all of
    /// which need a scalar "how bright is this" and would be subtly wrong with
    /// a plain average.
    #[inline]
    pub fn luminance(self) -> f32 {
        // ACEScg (AP1) luminance coefficients.
        0.272_228_7 * self.r + 0.674_081_8 * self.g + 0.053_689_5 * self.b
    }

    #[inline]
    pub fn max_component(self) -> f32 {
        self.r.max(self.g).max(self.b)
    }

    #[inline]
    pub fn min_component(self) -> f32 {
        self.r.min(self.g).min(self.b)
    }

    #[inline]
    pub fn is_black(self) -> bool {
        self.r == 0.0 && self.g == 0.0 && self.b == 0.0
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite()
    }

    /// Replace NaN and negative components with zero. Applied at the boundary
    /// where samples enter the accumulation buffer: one NaN otherwise poisons
    /// a pixel permanently, since `NaN + anything` stays NaN.
    #[inline]
    pub fn sanitized(self) -> Self {
        #[inline]
        fn clean(v: f32) -> f32 {
            if v.is_finite() && v > 0.0 {
                v
            } else {
                0.0
            }
        }
        spectrum(clean(self.r), clean(self.g), clean(self.b))
    }

    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self * (1.0 - t) + o * t
    }

    #[inline]
    pub fn sqrt(self) -> Self {
        spectrum(self.r.sqrt(), self.g.sqrt(), self.b.sqrt())
    }

    #[inline]
    pub fn exp(self) -> Self {
        spectrum(self.r.exp(), self.g.exp(), self.b.exp())
    }

    #[inline]
    pub fn clamp(self, lo: f32, hi: f32) -> Self {
        spectrum(
            self.r.clamp(lo, hi),
            self.g.clamp(lo, hi),
            self.b.clamp(lo, hi),
        )
    }

    #[inline]
    pub fn to_array(self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }
}

impl Add for Spectrum {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        spectrum(self.r + o.r, self.g + o.g, self.b + o.b)
    }
}
impl Sub for Spectrum {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        spectrum(self.r - o.r, self.g - o.g, self.b - o.b)
    }
}
impl Mul for Spectrum {
    type Output = Self;
    #[inline]
    fn mul(self, o: Self) -> Self {
        spectrum(self.r * o.r, self.g * o.g, self.b * o.b)
    }
}
impl Div for Spectrum {
    type Output = Self;
    #[inline]
    fn div(self, o: Self) -> Self {
        spectrum(self.r / o.r, self.g / o.g, self.b / o.b)
    }
}
impl Mul<f32> for Spectrum {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        spectrum(self.r * s, self.g * s, self.b * s)
    }
}
impl Mul<Spectrum> for f32 {
    type Output = Spectrum;
    #[inline]
    fn mul(self, s: Spectrum) -> Spectrum {
        s * self
    }
}
impl Div<f32> for Spectrum {
    type Output = Self;
    #[inline]
    fn div(self, s: f32) -> Self {
        let inv = 1.0 / s;
        spectrum(self.r * inv, self.g * inv, self.b * inv)
    }
}
impl AddAssign for Spectrum {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        self.r += o.r;
        self.g += o.g;
        self.b += o.b;
    }
}
impl MulAssign for Spectrum {
    #[inline]
    fn mul_assign(&mut self, o: Self) {
        self.r *= o.r;
        self.g *= o.g;
        self.b *= o.b;
    }
}
impl MulAssign<f32> for Spectrum {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        self.r *= s;
        self.g *= s;
        self.b *= s;
    }
}
impl DivAssign<f32> for Spectrum {
    #[inline]
    fn div_assign(&mut self, s: f32) {
        let inv = 1.0 / s;
        self.r *= inv;
        self.g *= inv;
        self.b *= inv;
    }
}

// ---------------------------------------------------------------------------
// Colour space conversions
// ---------------------------------------------------------------------------

/// sRGB electro-optical transfer function: encoded -> linear.
#[inline]
pub fn srgb_eotf(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB inverse EOTF: linear -> encoded.
#[inline]
pub fn srgb_inverse_eotf(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear Rec.709/sRGB primaries -> ACEScg (AP1), both D65-adapted.
#[inline]
pub fn srgb_to_acescg(v: Vec3) -> Vec3 {
    vec3(
        0.613_097_4 * v.x + 0.339_523_4 * v.y + 0.047_379_2 * v.z,
        0.070_193_6 * v.x + 0.916_353_5 * v.y + 0.013_452_9 * v.z,
        0.020_615_8 * v.x + 0.109_569_6 * v.y + 0.869_814_6 * v.z,
    )
}

/// ACEScg (AP1) -> linear Rec.709/sRGB primaries. The inverse of
/// [`srgb_to_acescg`]; note it can produce negative components for saturated
/// ACEScg colours outside the sRGB gamut, which is correct and is the display
/// transform's problem to resolve, not this function's.
#[inline]
pub fn acescg_to_srgb(v: Vec3) -> Vec3 {
    vec3(
        1.704_858_7 * v.x - 0.621_716_2 * v.y - 0.083_142_4 * v.z,
        -0.130_076_7 * v.x + 1.140_804_5 * v.y - 0.010_727_8 * v.z,
        -0.024_003_2 * v.x - 0.128_968_8 * v.y + 1.152_972 * v.z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_transfer_round_trips() {
        for i in 0..=100 {
            let v = i as f32 / 100.0;
            assert!((srgb_inverse_eotf(srgb_eotf(v)) - v).abs() < 1e-5);
        }
    }

    #[test]
    fn primaries_round_trip() {
        for c in [
            vec3(1.0, 1.0, 1.0),
            vec3(0.8, 0.2, 0.05),
            vec3(0.0, 0.5, 0.9),
        ] {
            let round = acescg_to_srgb(srgb_to_acescg(c));
            assert!((round - c).length() < 2e-3, "{c:?} -> {round:?}");
        }
    }

    /// White must stay white across the primaries conversion: both spaces are
    /// D65-adapted, so (1,1,1) maps to (1,1,1). A chromatic-adaptation mistake
    /// shows up here first and would tint every render.
    #[test]
    fn white_is_preserved() {
        let w = srgb_to_acescg(Vec3::ONE);
        assert!((w - Vec3::ONE).length() < 2e-3, "white drifted to {w:?}");
    }

    #[test]
    fn luminance_of_white_is_one() {
        let white = Spectrum::from_linear_srgb(Vec3::ONE);
        assert!((white.luminance() - 1.0).abs() < 3e-3);
    }

    #[test]
    fn sanitize_removes_nan_and_negatives() {
        let s = spectrum(f32::NAN, -1.0, f32::INFINITY).sanitized();
        assert_eq!(s, Spectrum::ZERO);
        assert_eq!(spectrum(0.5, 2.0, 0.0).sanitized(), spectrum(0.5, 2.0, 0.0));
    }
}
