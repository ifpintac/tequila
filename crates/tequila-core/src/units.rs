//! Scene units.
//!
//! `TEQUILA_SPEC.md` §1.3 flags this as "critical, and easy to get wrong":
//! units drive model scale, **light intensity**, **material colour density and
//! absorption**, and **texture mapping scale**. Getting them wrong produces
//! physically wrong lighting that a user cannot diagnose from the image, which
//! is why the unit is a first-class type rather than a float somewhere.
//!
//! The two conversion modes of `docs/03-SCENE-AND-IO.md` §3 land in
//! `tequila-scene` at M1; this module owns the unit itself and the conversion
//! factors everything else is expressed against.

/// A scene's length unit. The renderer works in a single internal length unit
/// (metres); this type records what the user's numbers mean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    /// A new, empty scene has no established unit yet. Importing geometry or
    /// setting it explicitly resolves it. Rendering with an unspecified unit
    /// is allowed but light intensities are then only relative.
    #[default]
    Unspecified,
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
}

impl LengthUnit {
    /// Metres per one of this unit.
    pub const fn to_metres(self) -> f64 {
        match self {
            // An unspecified unit is treated as metres so that arithmetic is
            // well-defined; the scene records that it is unspecified so the UI
            // can prompt, and so an import can set it without a conversion.
            LengthUnit::Unspecified | LengthUnit::Metre => 1.0,
            LengthUnit::Millimetre => 0.001,
            LengthUnit::Centimetre => 0.01,
            LengthUnit::Inch => 0.0254,
            LengthUnit::Foot => 0.3048,
        }
    }

    /// The short form a person reads, for labels and readouts.
    ///
    /// An unspecified unit has no symbol: saying "m" for it would be a claim
    /// the document has not made, and a scene whose scale is unknown should
    /// show a bare number rather than a wrong one.
    pub const fn symbol(self) -> &'static str {
        match self {
            LengthUnit::Unspecified => "",
            LengthUnit::Millimetre => "mm",
            LengthUnit::Centimetre => "cm",
            LengthUnit::Metre => "m",
            LengthUnit::Inch => "in",
            LengthUnit::Foot => "ft",
        }
    }

    /// Scale factor converting a length expressed in `self` into `target`.
    pub fn conversion_to(self, target: LengthUnit) -> f64 {
        self.to_metres() / target.to_metres()
    }

    pub const fn abbreviation(self) -> &'static str {
        match self {
            LengthUnit::Unspecified => "",
            LengthUnit::Millimetre => "mm",
            LengthUnit::Centimetre => "cm",
            LengthUnit::Metre => "m",
            LengthUnit::Inch => "in",
            LengthUnit::Foot => "ft",
        }
    }

    pub const ALL: [LengthUnit; 5] = [
        LengthUnit::Millimetre,
        LengthUnit::Centimetre,
        LengthUnit::Metre,
        LengthUnit::Inch,
        LengthUnit::Foot,
    ];
}

/// Quantities whose numeric value depends on the scene unit.
///
/// Enumerated explicitly because `docs/05` §6 lists "scene units mishandled"
/// as a risk whose mitigation is *enumerating* the unit-dependent quantities
/// rather than remembering them. Anything added to the renderer that scales
/// with length belongs in this list and in the conversion code that matches on
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitDependent {
    /// Vertex positions and instance translations. Scale by `k`.
    Geometry,
    /// Camera position, focus distance, and clipping. Scale by `k`.
    CameraDistance,
    /// Light emission: radiant intensity falls off with `1/r^2`, so a point
    /// light's power must scale by `k^2` to keep the same illuminance on a
    /// surface that also moved.
    LightIntensity,
    /// Beer-Lambert absorption and scattering distances (`transmission_depth`,
    /// `subsurface_radius`). Scale by `k`.
    MediumDistance,
    /// Texture projection scale for non-UV projections. Scale by `k`.
    TextureMappingScale,
    /// Environment dome size and ground size. Scale by `k`.
    EnvironmentSize,
    /// Displacement magnitude and bump height. Scale by `k`.
    DisplacementHeight,
}

impl UnitDependent {
    /// The exponent `n` in "multiply this quantity by `k^n`" when the scene's
    /// length scale changes by `k`.
    pub const fn scale_exponent(self) -> i32 {
        match self {
            UnitDependent::LightIntensity => 2,
            _ => 1,
        }
    }

    pub const ALL: [UnitDependent; 7] = [
        UnitDependent::Geometry,
        UnitDependent::CameraDistance,
        UnitDependent::LightIntensity,
        UnitDependent::MediumDistance,
        UnitDependent::TextureMappingScale,
        UnitDependent::EnvironmentSize,
        UnitDependent::DisplacementHeight,
    ];

    /// Factor to apply to this quantity for a length-scale change of `k`.
    pub fn factor(self, k: f64) -> f64 {
        k.powi(self.scale_exponent())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        for a in LengthUnit::ALL {
            for b in LengthUnit::ALL {
                let round = a.conversion_to(b) * b.conversion_to(a);
                assert!((round - 1.0).abs() < 1e-12, "{a:?} <-> {b:?}");
            }
        }
    }

    #[test]
    fn known_conversions_are_right() {
        assert!((LengthUnit::Metre.conversion_to(LengthUnit::Millimetre) - 1000.0).abs() < 1e-9);
        assert!((LengthUnit::Inch.conversion_to(LengthUnit::Millimetre) - 25.4).abs() < 1e-9);
        assert!((LengthUnit::Foot.conversion_to(LengthUnit::Inch) - 12.0).abs() < 1e-9);
    }

    /// Light intensity is the one quantity that scales with the *square* of
    /// the length change. Getting this wrong is the documented failure mode of
    /// "Keep Scene Size" unit conversion.
    #[test]
    fn light_intensity_scales_quadratically() {
        assert_eq!(UnitDependent::LightIntensity.scale_exponent(), 2);
        assert!((UnitDependent::LightIntensity.factor(1000.0) - 1e6).abs() < 1.0);
        for q in UnitDependent::ALL {
            if q != UnitDependent::LightIntensity {
                assert_eq!(q.scale_exponent(), 1, "{q:?}");
            }
        }
    }
}
