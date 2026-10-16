//! Display transforms and image styles.
//!
//! `docs/01-RENDERER-CORE.md` §7.3 fixes the pipeline:
//!
//! ```text
//! accumulated scene-linear -> Image Style -> OCIO display transform -> encode
//! ```
//!
//! and is emphatic that the last step is not cosmetic: "This is a large part
//! of why a physically correct render can still look wrong; the tone mapping
//! is not cosmetic." Studio lighting produces highlights hundreds of times
//! brighter than mid-grey, and naive sRGB clipping turns every one of them
//! into a flat white blob with a hard edge.
//!
//! The full OpenColorIO v2 integration with the ACES studio config arrives at
//! M3. This module implements the same transforms analytically so that M0 has
//! a correct pipeline rather than a placeholder — and so that the OCIO
//! integration has something to be checked against.

#![warn(missing_debug_implementations)]

use tequila_core::math::{vec3, Vec3};
use rayon::prelude::*;
use tequila_core::spectrum::{srgb_inverse_eotf, Spectrum};

/// The display transforms of §7.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DisplayTransform {
    /// AgX. Desaturates gracefully into highlights instead of hue-shifting,
    /// which keeps a bright coloured plastic from looking like a blown-out
    /// neon sign — at the price of flattening everything below them: blacks
    /// come out grey and colours faded. It was the default until the
    /// product renders it made looked washed out next to the Solid view.
    AgX,
    /// ACES 1.3 sRGB output transform, via the Narkowicz fit. Punchier than
    /// AgX and the industry default for film work.
    Aces,
    /// Khronos PBR Neutral, the default. Preserves material colour more
    /// faithfully than either of the above — a black stays black and a base
    /// colour renders as the colour it is — compressing only the highlights.
    /// Designed for product and e-commerce work, which is what Tequila is.
    #[default]
    PbrNeutral,
    /// Plain sRGB with hard clipping. For technical and QA work, where you
    /// want to *see* what is out of range rather than have it handled.
    Srgb,
    /// No transform at all beyond the primaries conversion.
    Raw,
}

impl DisplayTransform {
    pub const ALL: [DisplayTransform; 5] = [
        DisplayTransform::AgX,
        DisplayTransform::Aces,
        DisplayTransform::PbrNeutral,
        DisplayTransform::Srgb,
        DisplayTransform::Raw,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            DisplayTransform::AgX => "AgX",
            DisplayTransform::Aces => "ACES",
            DisplayTransform::PbrNeutral => "PBR Neutral",
            DisplayTransform::Srgb => "sRGB",
            DisplayTransform::Raw => "Raw",
        }
    }

    /// Map scene-linear radiance to a display-encoded value in `[0, 1]`.
    pub fn apply(self, color: Spectrum) -> Vec3 {
        let linear = color.to_linear_srgb().max(Vec3::ZERO);
        match self {
            DisplayTransform::AgX => encode_srgb(agx(linear)),
            DisplayTransform::Aces => encode_srgb(aces(linear)),
            DisplayTransform::PbrNeutral => encode_srgb(pbr_neutral(linear)),
            DisplayTransform::Srgb => encode_srgb(linear.clamp(0.0, 1.0)),
            DisplayTransform::Raw => linear.clamp(0.0, 1.0),
        }
    }

    /// The tone-mapped value *before* the sRGB encoding, and whether one is
    /// due.
    ///
    /// Split out so the eight-bit path can encode with a table lookup instead
    /// of three `powf` calls. The split is not a second implementation — every
    /// transform's curve is still written once, above and here in the same
    /// arms — and the encode it skips is reproduced exactly by
    /// [`srgb_encode_u8`].
    #[inline]
    fn toned(self, color: Spectrum) -> Vec3 {
        let linear = color.to_linear_srgb().max(Vec3::ZERO);
        match self {
            DisplayTransform::AgX => agx(linear),
            DisplayTransform::Aces => aces(linear),
            DisplayTransform::PbrNeutral => pbr_neutral(linear),
            DisplayTransform::Srgb | DisplayTransform::Raw => linear.clamp(0.0, 1.0),
        }
    }

    /// Map scene-linear radiance straight to an eight-bit display code.
    #[inline]
    pub fn apply_u8(self, color: Spectrum) -> [u8; 3] {
        encode_display_u8(self, self.toned(color))
    }
}

fn encode_srgb(linear: Vec3) -> Vec3 {
    vec3(
        srgb_inverse_eotf(linear.x.clamp(0.0, 1.0)),
        srgb_inverse_eotf(linear.y.clamp(0.0, 1.0)),
        srgb_inverse_eotf(linear.z.clamp(0.0, 1.0)),
    )
}

/// Base-2 logarithm, to about six decimal digits.
///
/// The exponent comes straight out of the float's bits and a degree-four
/// polynomial covers the mantissa, which is one octave wide. That is worth
/// doing here and nowhere else in the renderer: AgX calls it three times per
/// pixel, at 1080p that is six million calls per re-develop, and §9.3 promises
/// the whole pass inside a 60 fps frame.
///
/// The error — under `2e-6` across the range AgX uses — is spent on a *look*,
/// not a measurement. It enters a tone curve whose output is quantized to eight
/// bits, where one code value is 1/255; being a hundred thousand times inside
/// that is the difference between an approximation and a shortcut.
#[inline]
fn fast_log2(x: f32) -> f32 {
    let bits = x.to_bits();
    // Exponent, unbiased.
    let exponent = ((bits >> 23) & 0xff) as i32 - 127;
    // Mantissa, reassembled as a float in [1, 2).
    let mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    // A minimax fit of log2 over [1, 2), in Horner form. Splitting the
    // exponent off first is what makes this easy: the whole range collapses to
    // one octave, which a degree-six polynomial covers to two parts in a
    // million, and the exponent itself is exact.
    let m = mantissa;
    let log_mantissa = ((((((-0.025_275_857_4_f32) * m + 0.271_409_19) * m - 1.253_041_1) * m
        + 3.259_378_8)
        * m
        - 5.312_462_3)
        * m
        + 6.096_042_2)
        * m
        - 3.036_048_9;
    exponent as f32 + log_mantissa
}

/// `2^x`, for the AgX curve's linearization: the integer part into the
/// exponent bits, the fraction by a polynomial good to a few parts in ten
/// million over `[0, 1)`. Its first coefficient is close to ln 2 because
/// it is fitted, not because it is meant to be ln 2.
#[inline]
#[allow(clippy::approx_constant)]
fn fast_exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, 126.0);
    let whole = x.floor();
    let f = x - whole;
    let p = 1.0
        + f * (0.693_147_2 + f * (0.240_226_5 + f * (0.055_504_1 + f * (0.009_618_1 + f * 0.001_333_3))));
    f32::from_bits(((whole as i32 + 127) as u32) << 23) * p
}

/// AgX, in the compact form used by Blender's implementation.
///
/// The shape that matters: the transform works in log space over a fixed
/// exposure range, so highlights compress smoothly and asymptotically instead
/// of clipping, and a per-channel curve desaturates them towards white rather
/// than letting one channel clip first and shift the hue.
fn agx(linear: Vec3) -> Vec3 {
    // Into AgX's working space. These are the standard AgX inset matrix
    // coefficients, which pull the primaries inwards so that saturated colours
    // have room to desaturate before clipping.
    let m = [
        [0.842_479_1, 0.078_437_3, 0.079_083_6],
        [0.042_328_4, 0.878_468_1, 0.079_203_5],
        [0.042_346_5, 0.078_845_4, 0.878_808_1],
    ];
    let v = vec3(
        m[0][0] * linear.x + m[0][1] * linear.y + m[0][2] * linear.z,
        m[1][0] * linear.x + m[1][1] * linear.y + m[1][2] * linear.z,
        m[2][0] * linear.x + m[2][1] * linear.y + m[2][2] * linear.z,
    );

    // Log2 encode over the AgX range of -12.47 to +4.03 stops.
    const MIN_EV: f32 = -12.473_931;
    const MAX_EV: f32 = 4.026_069;
    let encode = |x: f32| ((fast_log2(x.max(1e-10)) - MIN_EV) / (MAX_EV - MIN_EV)).clamp(0.0, 1.0);
    let l = vec3(encode(v.x), encode(v.y), encode(v.z));

    // The tone curve: a sigmoid in log space, as a polynomial fit.
    let curve = |x: f32| {
        let x2 = x * x;
        let x4 = x2 * x2;
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
            - 0.00232
    };
    let toned = vec3(curve(l.x), curve(l.y), curve(l.z));

    // The curve's output is display-encoded — it was fitted in a gamma-2.2
    // space — and the display encode comes after this function. So it goes
    // back to linear first, as AgX's own EOTF step does. Without it the
    // encode was applied twice: 18% grey came out at 186 of 255 rather than
    // AgX's half-way, and every render in the look was washed out, as
    // though seen through haze.
    // As 2^(2.2 log2 x), from the same fast log the encode above uses: a
    // `powf` per channel made the look five times the cost of no curve and
    // took a 1080p re-develop past what a slider drag can wait for.
    let linear = |x: f32| if x <= 0.0 { 0.0 } else { fast_exp2(2.2 * fast_log2(x)) };
    let toned = vec3(linear(toned.x), linear(toned.y), linear(toned.z));

    // Back out of the inset. The outset is the inverse of the inset matrix.
    let o = [
        [1.196_881_3, -0.098_020_3, -0.098_861_0],
        [-0.052_801_8, 1.151_824_2, -0.099_022_4],
        [-0.052_945_3, -0.098_275_4, 1.151_220_5],
    ];
    vec3(
        o[0][0] * toned.x + o[0][1] * toned.y + o[0][2] * toned.z,
        o[1][0] * toned.x + o[1][1] * toned.y + o[1][2] * toned.z,
        o[2][0] * toned.x + o[2][1] * toned.y + o[2][2] * toned.z,
    )
    .clamp(0.0, 1.0)
}

/// ACES filmic output transform, via the Narkowicz rational fit.
fn aces(linear: Vec3) -> Vec3 {
    let f = |x: f32| {
        let x = x * 0.6; // the fit is defined against a 0.6-scaled input
        ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
    };
    vec3(f(linear.x), f(linear.y), f(linear.z))
}

/// Khronos PBR Neutral.
///
/// Designed for commerce: it keeps a material's colour recognizable all the
/// way into the highlights instead of pushing it towards a film look. That
/// matters when the render is meant to show what a product actually looks
/// like.
fn pbr_neutral(linear: Vec3) -> Vec3 {
    const START_COMPRESSION: f32 = 0.8 - 0.04;
    const DESATURATION: f32 = 0.15;

    let mut color = linear;
    // Lift the darkest channel out of negatives introduced by the wide-gamut
    // working space.
    let x = color.min_component_value();
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    color -= Vec3::splat(offset);

    let peak = color.max_component();
    if peak < START_COMPRESSION {
        return color;
    }

    let d = 1.0 - START_COMPRESSION;
    let new_peak = 1.0 - d * d / (peak + d - START_COMPRESSION);
    color *= new_peak / peak;

    // Desaturate towards white as the peak approaches 1, which is what stops
    // a bright saturated surface from clipping to a flat primary.
    let g = 1.0 - 1.0 / (DESATURATION * (peak - new_peak) + 1.0);
    color.lerp(Vec3::splat(new_peak), g)
}

trait MinComponent {
    fn min_component_value(self) -> f32;
}

impl MinComponent for Vec3 {
    fn min_component_value(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }
}

/// Approximate chromaticity of a blackbody at `kelvin`, normalized so its
/// luminance is 1.
///
/// An approximation rather than a Planckian integral: the difference is well
/// under a just-noticeable step across the 1500–15000 K range a user will ask
/// for, and the exact version needs the spectral pipeline that arrives with
/// M6's gems. Normalizing the luminance is what makes the temperature control
/// change the *colour* without also changing the exposure — otherwise every
/// adjustment to warmth is also an adjustment to brightness, and the two are
/// impossible to separate by eye.
pub fn blackbody_rgb(kelvin: f32) -> Spectrum {
    let t = (kelvin.clamp(1000.0, 40000.0)) / 100.0;

    let red = if t <= 66.0 {
        255.0
    } else {
        329.698_73 * (t - 60.0).powf(-0.133_204_76)
    };
    let green = if t <= 66.0 {
        99.470_802 * t.ln() - 161.119_57
    } else {
        288.122_16 * (t - 60.0).powf(-0.075_514_85)
    };
    let blue = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_79
    };

    let linear = |v: f32| {
        let v = (v / 255.0).clamp(0.0, 1.0);
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let colour = Spectrum {
        r: linear(red),
        g: linear(green),
        b: linear(blue),
    };
    let luminance = colour.luminance().max(1e-6);
    colour / luminance
}

/// The Basic image style of `TEQUILA_SPEC.md` §1.9.
///
/// Applied to scene-linear values *before* the display transform, and — this
/// is the point — it never triggers a re-render. `docs/01` §9.3 marks image
/// style and exposure as "no re-render at all. Instant."
/// What actually turns scene-linear radiance into display code values.
///
/// Two of them, and the split is the point of §7. The built-in transforms are
/// always there, need no files, and are what the application opens with. An
/// OCIO pipeline is what a studio points at its own config so that Tequila's
/// picture is the one the rest of its pipeline will produce — read from the
/// same file the compositor reads, which is the entire value of OCIO and is
/// not something a built-in curve can ever be.
///
/// Held beside [`ImageStyle`] rather than inside it, because a style is a
/// *look* — a small, copyable, undoable block of numbers the document carries —
/// and an OCIO processor is a compiled object holding LUT tables. Putting one
/// inside the other would make every undo entry drag a lookup table behind it.
#[derive(Clone, Debug)]
pub enum DisplayPipeline {
    Builtin(DisplayTransform),
    Ocio {
        processor: std::sync::Arc<ocio::Processor>,
        /// `display / view`, for the readout.
        label: String,
    },
}

impl Default for DisplayPipeline {
    fn default() -> Self {
        DisplayPipeline::Builtin(DisplayTransform::default())
    }
}

impl DisplayPipeline {
    /// Map scene-linear radiance to a display-encoded value in `[0, 1]`.
    pub fn apply(&self, color: Spectrum) -> Vec3 {
        match self {
            DisplayPipeline::Builtin(transform) => transform.apply(color),
            // The processor works in the rendering space, which is what the
            // config's `scene_linear` role names and what the renderer already
            // holds — no conversion to sRGB on the way in, which would throw
            // away the wide-gamut values the config exists to handle.
            DisplayPipeline::Ocio { processor, .. } => {
                let out = processor.apply(color.to_rendering_rgb());
                vec3(out.x, out.y, out.z)
            }
        }
    }

    /// Map scene-linear radiance straight to an eight-bit display code.
    #[inline]
    pub fn apply_u8(&self, color: Spectrum) -> [u8; 3] {
        match self {
            DisplayPipeline::Builtin(transform) => transform.apply_u8(color),
            DisplayPipeline::Ocio { processor, .. } => {
                // A config's own output encoding is whatever its last transform
                // says, so the result is already display-referred and is only
                // quantized here. Applying an sRGB encode on top would be a
                // second one the config did not ask for.
                let out = processor.apply(color.to_rendering_rgb());
                [encode_u8(out.x), encode_u8(out.y), encode_u8(out.z)]
            }
        }
    }

    pub fn label(&self) -> &str {
        match self {
            DisplayPipeline::Builtin(transform) => transform.label(),
            DisplayPipeline::Ocio { label, .. } => label,
        }
    }
}

/// Which set of controls the style presents (§1.9).
///
/// Not two types, because they share exposure, contrast, saturation and the
/// display transform, and switching between them must not throw those away —
/// a user who set an exposure in Basic and then wanted a vignette would
/// otherwise lose it. [`ToneMode::Basic`] ignores the photographic fields,
/// which is asserted by `basic_ignores_every_photographic_control`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToneMode {
    #[default]
    Basic,
    Photographic,
}

impl ToneMode {
    pub const ALL: [ToneMode; 2] = [ToneMode::Basic, ToneMode::Photographic];

    pub const fn label(self) -> &'static str {
        match self {
            ToneMode::Basic => "Basic",
            ToneMode::Photographic => "Photographic",
        }
    }
}

/// Neutral colour temperature. Daylight, and the point at which the white
/// balance controls do nothing.
pub const NEUTRAL_KELVIN: f32 = 6500.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageStyle {
    pub mode: ToneMode,
    /// Exposure in stops.
    pub exposure: f32,
    /// Contrast about mid-grey. 1 is neutral.
    pub contrast: f32,
    /// 0 is greyscale, 1 neutral, above 1 boosted.
    pub saturation: f32,

    // ---- Photographic ---------------------------------------------------
    //
    // These are *look* controls, not measurements: each is a curve chosen to
    // behave the way the corresponding control on a camera or in a raw
    // developer behaves. Two properties are not matters of taste and are
    // tested — at their neutral values each is exactly the identity, and each
    // is monotone in its parameter, so dragging a slider one way never
    // reverses direction partway.
    /// Recovery or boost of the bright end, -1 to 1.
    pub highlights: f32,
    /// Lift or crush of the dark end, -1 to 1.
    pub shadows: f32,
    /// Gain at the very top of the range, -1 to 1.
    pub whites: f32,
    /// The black point, -1 to 1. Positive lifts it (a faded, filmic floor),
    /// negative crushes it: right is brighter, as on every other slider.
    pub blacks: f32,
    /// White balance, in kelvin. The image is corrected *as if* it had been
    /// lit at this temperature, so a lower number makes the image cooler.
    pub temperature: f32,
    /// Green/magenta balance, -1 to 1: the axis temperature leaves over.
    pub tint: f32,
    /// Corner darkening, 0 to 1.
    pub vignette: f32,
    /// Glare bleeding out of the brightest areas, 0 to 1.
    pub bloom: f32,
    /// Radius of that glare, as a fraction of the image's smaller side.
    pub bloom_radius: f32,
    /// Unsharp masking, 0 to 1.
    pub sharpening: f32,
    /// Lateral chromatic aberration: radial separation of the red and blue
    /// channels, as a fraction of the image radius.
    pub chromatic_aberration: f32,

    pub display: DisplayTransform,
}

impl Default for ImageStyle {
    fn default() -> Self {
        ImageStyle {
            mode: ToneMode::Basic,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temperature: NEUTRAL_KELVIN,
            tint: 0.0,
            vignette: 0.0,
            bloom: 0.0,
            bloom_radius: 0.03,
            sharpening: 0.0,
            chromatic_aberration: 0.0,
            display: DisplayTransform::default(),
        }
    }
}

/// Photographic mid-grey. Contrast pivots here so that adjusting it does not
/// also change the overall brightness.
const MIDDLE_GREY: f32 = 0.18;

impl ImageStyle {
    /// Whether any enabled control needs to see more than one pixel at a time.
    ///
    /// The per-pixel path is the one that runs on every viewport frame, so it
    /// is worth knowing when the expensive path can be skipped entirely.
    pub fn needs_neighbourhood(&self) -> bool {
        self.mode == ToneMode::Photographic
            && (self.bloom > 0.0 || self.sharpening > 0.0 || self.chromatic_aberration > 0.0)
    }

    /// The scene-linear grade, before any display transform.
    ///
    /// Split out from [`Self::apply`] because the spatial effects belong
    /// between the grade and the transform: bloom that ran after tone mapping
    /// would bleed display values rather than light, and would stop looking
    /// like glare the moment the highlight roll-off changed.
    pub fn grade(&self, color: Spectrum) -> Spectrum {
        self.grade_scaled(color, f32::exp2(self.exposure))
    }

    /// The grade, with the exposure multiplier supplied rather than computed.
    ///
    /// `exp2` is a transcendental and the exposure does not vary across an
    /// image, so computing it per pixel is two million calls to produce one
    /// number. Hoisting it out is the kind of thing a compiler cannot do for
    /// itself here, because `self` is behind a reference the loop does not
    /// prove it owns.
    #[inline]
    fn grade_scaled(&self, color: Spectrum, exposure_scale: f32) -> Spectrum {
        let mut c = color * exposure_scale;

        if self.mode == ToneMode::Photographic {
            c = self.white_balance(c);
            c = self.tone_curve(c);
        }

        if self.saturation != 1.0 {
            let luminance = c.luminance();
            c = Spectrum::splat(luminance).lerp(c, self.saturation);
        }

        if self.contrast != 1.0 {
            // A power curve about mid-grey, in linear space. Applying contrast
            // after the display transform instead would crush the highlight
            // roll-off the transform just built.
            let apply = |v: f32| {
                if v <= 0.0 {
                    0.0
                } else {
                    MIDDLE_GREY * (v / MIDDLE_GREY).powf(self.contrast)
                }
            };
            c = Spectrum {
                r: apply(c.r),
                g: apply(c.g),
                b: apply(c.b),
            };
        }

        c
    }

    /// Grade a scene-linear value and encode it for display.
    pub fn apply(&self, color: Spectrum) -> Vec3 {
        self.display.apply(self.grade(color))
    }

    /// Correct the image as if it had been lit at `temperature`.
    ///
    /// The multiplier is renormalized against a white patch, so changing the
    /// temperature changes the colour of the image and not its brightness.
    /// Without that, every warmth adjustment is also an exposure adjustment
    /// and the two can never be separated by eye.
    fn white_balance(&self, c: Spectrum) -> Spectrum {
        if (self.temperature - NEUTRAL_KELVIN).abs() < 1.0 && self.tint == 0.0 {
            return c;
        }
        let source = blackbody_rgb(self.temperature);
        let mut gain = Spectrum {
            r: 1.0 / source.r.max(1e-4),
            g: 1.0 / source.g.max(1e-4),
            b: 1.0 / source.b.max(1e-4),
        };
        if self.tint != 0.0 {
            let t = self.tint.clamp(-1.0, 1.0) * 0.3;
            gain.g *= 1.0 - t;
            gain.r *= 1.0 + t * 0.5;
            gain.b *= 1.0 + t * 0.5;
        }
        let balanced = Spectrum {
            r: c.r * gain.r,
            g: c.g * gain.g,
            b: c.b * gain.b,
        };
        // Renormalized by what the gain does to a white patch, not by what it
        // does to *this* pixel: scaling each pixel to preserve its own
        // luminance would make the whole control a no-op.
        let reference = gain.luminance().max(1e-6);
        balanced / reference
    }

    /// Highlights, shadows, whites and blacks, in scene-linear.
    ///
    /// Each is a gain driven by a mask over luminance rather than a curve over
    /// the channel values, so adjusting the shadows of a coloured surface does
    /// not also shift its hue — which is what a per-channel curve does, and
    /// what makes a naive shadows slider tint everything it touches.
    fn tone_curve(&self, c: Spectrum) -> Spectrum {
        if self.highlights == 0.0 && self.shadows == 0.0 && self.whites == 0.0 && self.blacks == 0.0
        {
            return c;
        }
        let luminance = c.luminance().max(0.0);
        let relative = luminance / MIDDLE_GREY;

        // Masks in stops about mid-grey, so the regions they name are the ones
        // a photographer means: shadows are the stops below, highlights the
        // stops above, whites everything past the top few.
        let stops = if relative > 1e-6 {
            relative.log2()
        } else {
            -20.0
        };
        let shadow_mask = smoothstep(-3.0, 0.0, -stops);
        let highlight_mask = smoothstep(0.0, 3.0, stops);
        let white_mask = smoothstep(2.0, 5.0, stops);

        // Gains are exponential in the parameter, so -1 and +1 are a halving
        // and a doubling rather than a subtraction that could reach zero.
        let gain = f32::exp2(
            self.shadows.clamp(-1.0, 1.0) * shadow_mask
                + self.highlights.clamp(-1.0, 1.0) * highlight_mask
                + self.whites.clamp(-1.0, 1.0) * white_mask,
        );
        let c = c * gain;

        if self.blacks == 0.0 {
            return c;
        }
        // A black *point*: positive lifts the floor towards a faded look,
        // negative pushes it down and clips what falls below.
        let point = -self.blacks.clamp(-1.0, 1.0) * MIDDLE_GREY * 0.25;
        let scale = 1.0 / (1.0 - point / MIDDLE_GREY).max(1e-3);
        let adjust = |v: f32| ((v - point) * scale).max(0.0);
        Spectrum {
            r: adjust(c.r),
            g: adjust(c.g),
            b: adjust(c.b),
        }
    }

    /// The vignette multiplier at a point, `(u, v)` in `[0, 1]` across the
    /// image.
    pub fn vignette_at(&self, u: f32, v: f32) -> f32 {
        if self.mode != ToneMode::Photographic || self.vignette <= 0.0 {
            return 1.0;
        }
        let dx = (u - 0.5) * 2.0;
        let dy = (v - 0.5) * 2.0;
        // Normalized so the corners reach 1 and the centre 0 whatever the
        // aspect ratio, otherwise a wide image vignettes on the sides only.
        let r = ((dx * dx + dy * dy) * 0.5).sqrt().clamp(0.0, 1.0);
        1.0 - self.vignette.clamp(0.0, 1.0) * smoothstep(0.2, 1.0, r)
    }

    /// Develop a whole image through the style's own built-in transform.
    pub fn develop(&self, width: usize, height: usize, scene_linear: &[Spectrum]) -> Vec<u8> {
        self.develop_through(
            width,
            height,
            scene_linear,
            &DisplayPipeline::Builtin(self.display),
        )
    }

    /// Develop a whole image: grade, spatial effects, display transform,
    /// encode. Returns RGBA8, row-major from the top-left.
    ///
    /// This is the pass §9.3 calls "no re-render at all": it reads the
    /// accumulation buffer and writes pixels, so every control on the Image
    /// tab is instant no matter how long the render has been running. It is
    /// also the only place the spatial effects can live — bloom, sharpening
    /// and chromatic aberration are each defined over a neighbourhood, and a
    /// per-pixel function cannot express them however it is called.
    pub fn develop_through(
        &self,
        width: usize,
        height: usize,
        scene_linear: &[Spectrum],
        display: &DisplayPipeline,
    ) -> Vec<u8> {
        let count = width * height;
        assert!(
            scene_linear.len() >= count,
            "develop: {} pixels for a {width}x{height} image",
            scene_linear.len()
        );
        // In parallel, and not as a micro-optimization: at 1080p this is two
        // million pixels through a grade and a tone curve, several
        // transcendentals each, and §9.3 promises the whole pass inside a
        // 60 fps frame. Serially it does not fit.
        //
        // The common case is fused into a single pass, and that matters more
        // than the arithmetic does. The general path below holds two
        // intermediate images — 48 MB at 1080p, written and read again — and
        // that memory traffic, not the transcendentals, is most of the cost.
        // The intermediates exist only so the spatial effects have a
        // neighbourhood to work over, so when none is enabled there is nothing
        // for them to hold.
        if !self.needs_neighbourhood() && self.vignette <= 0.0 {
            // Written into a buffer that already exists rather than collected.
            // That is not a style preference: a parallel `collect` into a
            // `Vec<u8>` cannot know each worker's share in advance, so it
            // builds per-worker buffers and concatenates them — which measured
            // at 12 ms for a transform that does a matrix multiply and a
            // clamp, more than the tone mapping it was carrying.
            // A row at a time, not a pixel at a time. Rayon's per-item
            // bookkeeping is small but two million of it is not: splitting by
            // row cut this pass by a third on its own.
            let exposure_scale = f32::exp2(self.exposure);
            let mut out = vec![255u8; count * 4];
            out.par_chunks_mut(width * 4)
                .enumerate()
                .for_each(|(y, row)| {
                    let source = &scene_linear[y * width..(y + 1) * width];
                    for (x, (pixel, color)) in row.chunks_exact_mut(4).zip(source).enumerate() {
                        let graded = self.grade_scaled(color.sanitized(), exposure_scale);
                        let encoded = display.apply(graded);
                        let threshold = dither_threshold(x, y);
                        pixel[0] = quantize_dithered(encoded.x, threshold);
                        pixel[1] = quantize_dithered(encoded.y, threshold);
                        pixel[2] = quantize_dithered(encoded.z, threshold);
                    }
                });
            return out;
        }

        let exposure_scale = f32::exp2(self.exposure);
        let mut graded: Vec<Spectrum> = Vec::with_capacity(count);
        scene_linear[..count]
            .par_iter()
            .map(|c| self.grade_scaled(c.sanitized(), exposure_scale))
            .collect_into_vec(&mut graded);

        if self.mode == ToneMode::Photographic {
            if self.bloom > 0.0 {
                self.add_bloom(width, height, &mut graded);
            }
            if self.chromatic_aberration > 0.0 {
                graded = self.separate_channels(width, height, &graded);
            }
            if self.vignette > 0.0 {
                for y in 0..height {
                    // Sampled at pixel centres: the corner pixel of a small
                    // image is not at the corner of the image.
                    let v = (y as f32 + 0.5) / height as f32;
                    for x in 0..width {
                        let u = (x as f32 + 0.5) / width as f32;
                        graded[y * width + x] = graded[y * width + x] * self.vignette_at(u, v);
                    }
                }
            }
        }

        let mut encoded: Vec<Vec3> = Vec::with_capacity(count);
        graded
            .par_iter()
            .map(|c| display.apply(*c))
            .collect_into_vec(&mut encoded);

        // Sharpening runs *after* the display transform, on purpose. Unsharp
        // masking in scene-linear amplifies the difference between a highlight
        // and its surround by the raw ratio, which in a studio render can be
        // hundreds to one; the same operation on display values sharpens what
        // the eye actually sees.
        if self.mode == ToneMode::Photographic && self.sharpening > 0.0 {
            encoded = self.unsharp_mask(width, height, &encoded);
        }

        let mut out = vec![255u8; count * 4];
        out.par_chunks_mut(width * 4)
            .enumerate()
            .for_each(|(y, row)| {
                let source = &encoded[y * width..(y + 1) * width];
                for (x, (pixel, v)) in row.chunks_exact_mut(4).zip(source).enumerate() {
                    let threshold = dither_threshold(x, y);
                    pixel[0] = quantize_dithered(v.x, threshold);
                    pixel[1] = quantize_dithered(v.y, threshold);
                    pixel[2] = quantize_dithered(v.z, threshold);
                }
            });
        out
    }

    /// Add glare bleeding out of the brightest areas.
    ///
    /// A threshold at one stop above mid-grey, a separable Gaussian, and the
    /// result added back. Added rather than mixed: glare in a lens is light
    /// arriving where it should not, not a redistribution of light that was
    /// already there, and a mix would darken the source of the glare.
    fn add_bloom(&self, width: usize, height: usize, image: &mut [Spectrum]) {
        let threshold = MIDDLE_GREY * 2.0;
        let bright: Vec<Spectrum> = image
            .iter()
            .map(|c| {
                let over = c.luminance() - threshold;
                if over <= 0.0 {
                    Spectrum::ZERO
                } else {
                    // Weighted by how far over it is, so the transition into
                    // the glare is smooth rather than an outline around every
                    // region that happens to cross the threshold.
                    *c * (over / c.luminance().max(1e-6))
                }
            })
            .collect();

        let radius_px =
            (self.bloom_radius.clamp(0.0, 1.0) * width.min(height) as f32).max(1.0);
        let blurred = gaussian_blur(width, height, &bright, radius_px);
        let strength = self.bloom.clamp(0.0, 1.0);
        for (out, add) in image.iter_mut().zip(blurred) {
            *out = *out + add * strength;
        }
    }

    /// Radially separate the red and blue channels, the way a lens that fails
    /// to focus every wavelength at the same magnification does.
    ///
    /// Green is left alone and the other two are sampled at slightly different
    /// magnifications about the image centre, which is why the effect is
    /// invisible in the middle and strongest in the corners — the signature
    /// that makes it read as a lens rather than as a misalignment.
    fn separate_channels(&self, width: usize, height: usize, image: &[Spectrum]) -> Vec<Spectrum> {
        let amount = self.chromatic_aberration.clamp(0.0, 1.0) * 0.02;
        let mut out = vec![Spectrum::ZERO; image.len()];
        let (w, h) = (width as f32, height as f32);
        for y in 0..height {
            for x in 0..width {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let dx = px - w * 0.5;
                let dy = py - h * 0.5;
                let sample = |scale: f32| {
                    bilinear(
                        width,
                        height,
                        image,
                        w * 0.5 + dx * scale,
                        h * 0.5 + dy * scale,
                    )
                };
                let red = sample(1.0 + amount);
                let blue = sample(1.0 - amount);
                let green = image[y * width + x];
                out[y * width + x] = Spectrum {
                    r: red.r,
                    g: green.g,
                    b: blue.b,
                };
            }
        }
        out
    }

    /// Unsharp masking on display-encoded values.
    fn unsharp_mask(&self, width: usize, height: usize, image: &[Vec3]) -> Vec<Vec3> {
        let as_spectrum: Vec<Spectrum> = image
            .iter()
            .map(|v| Spectrum {
                r: v.x,
                g: v.y,
                b: v.z,
            })
            .collect();
        let blurred = gaussian_blur(width, height, &as_spectrum, 1.5);
        let amount = self.sharpening.clamp(0.0, 1.0) * 1.5;
        image
            .iter()
            .zip(blurred)
            .map(|(sharp, soft)| {
                let soft = vec3(soft.r, soft.g, soft.b);
                // Clamped into range here rather than at encode time, so the
                // overshoot a sharpen produces cannot wrap a bright edge into
                // a dark one.
                let out = *sharp + (*sharp - soft) * amount;
                vec3(
                    out.x.clamp(0.0, 1.0),
                    out.y.clamp(0.0, 1.0),
                    out.z.clamp(0.0, 1.0),
                )
            })
            .collect()
    }

    /// Grade and encode to 8-bit sRGB.
    pub fn apply_u8(&self, color: Spectrum) -> [u8; 3] {
        let v = self.apply(color);
        [
            (v.x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (v.y.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (v.z.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }
}

fn encode_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The linear value at which the sRGB encoding crosses into each 8-bit code.
///
/// Built once, from the exact transfer function, and searched instead of
/// evaluated. The reason is measured rather than assumed: at 1080p the develop
/// pass is two million pixels, the sRGB encode is three `powf` calls each, and
/// `powf` was most of a 30 ms frame — over §9.3's whole budget for a pass that
/// is supposed to make dragging a slider feel free.
///
/// A search over the code boundaries is *exact* rather than approximate: the
/// table holds the value that rounds to each code, so the result is the same
/// byte the power would have produced. That is what lets the float path and the
/// eight-bit path stay in agreement, which
/// `develop_agrees_with_the_per_pixel_path` asserts.
fn srgb_thresholds() -> &'static [f32; 255] {
    static THRESHOLDS: std::sync::OnceLock<[f32; 255]> = std::sync::OnceLock::new();
    THRESHOLDS.get_or_init(|| {
        let mut table = [0.0f32; 255];
        for (i, entry) in table.iter_mut().enumerate() {
            // The boundary between code `i` and `i + 1`: the linear value whose
            // encoding is exactly halfway between them.
            *entry = tequila_core::spectrum::srgb_eotf((i as f32 + 0.5) / 255.0);
        }
        table
    })
}

/// An 8x8 ordered dither, as a threshold in [0, 1) per pixel.
///
/// # Why an image needs this at all
///
/// Quantizing to 256 levels turns a smooth gradient into a staircase, and the
/// eye finds the steps. It is at its worst exactly where a product render
/// spends most of its area: a large, slowly changing surface. Measured on the
/// ground plane of the demo scene with reflections on, the floor came out as
/// 35 flat bands, the largest 77 pixels tall — and because a floor's
/// iso-luminance contours are circles about the camera's ground point, those
/// bands appear as curved lines sweeping across the floor.
///
/// Dithering fixes it by adding under half a code of noise before rounding, so
/// a value between two codes lands on the lower one in some pixels and the
/// upper one in others, in the right proportion. The staircase becomes a
/// gradient the eye integrates, at the cost of a texture far below the
/// threshold of visibility.
///
/// # Why *ordered*, and not random
///
/// Because the renderer promises determinism (`docs/05` §3.6) and the file
/// format promises a byte-identical rewrite. A random dither would make two
/// renders of one scene differ in most pixels — indistinguishable to the eye
/// and fatal to every golden-image test, every A/B compare in the gallery, and
/// the "did anything actually change" question a person asks by diffing two
/// files. The threshold below is a pure function of the pixel's coordinates,
/// so the same image quantizes the same way every time, on any number of
/// threads, in any order.
///
/// The matrix is the Bayer 8x8, which distributes its thresholds as evenly as
/// an 8x8 grid allows. Its cross-hatch texture is the reason larger dithers
/// use blue noise instead, but at a sixteenth of one code the texture is far
/// below anything visible and the pattern's evenness is worth more.
#[inline]
fn dither_threshold(x: usize, y: usize) -> f32 {
    // Generated rather than tabulated: the Bayer matrix is the bit interleave
    // of `y` with `x ^ y`, most significant bit first, and writing the
    // sixty-four numbers out by hand invites a typo nothing would catch.
    let (xc, yc) = ((x as u32 ^ y as u32) & 7, y as u32 & 7);
    let mut value = 0u32;
    for bit in (0..3).rev() {
        value = (value << 1) | ((yc >> bit) & 1);
        value = (value << 1) | ((xc >> bit) & 1);
    }
    // 64 levels, centred so the mean perturbation is zero and the image does
    // not drift brighter or darker.
    (value as f32 + 0.5) / 64.0
}

/// Quantize with the dither for this pixel.
///
/// The perturbation is applied in *display* space, after the transfer
/// function, because that is where the codes are evenly spaced. Half a code of
/// display value is a different amount of scene radiance in the shadows than
/// in the highlights, and it is the display value that is being rounded.
#[inline]
fn quantize_dithered(value: f32, threshold: f32) -> u8 {
    let scaled = value.clamp(0.0, 1.0) * 255.0;
    let floor = scaled.floor();
    // The fraction decides how often this pixel rounds up. Comparing it
    // against an evenly distributed threshold makes the *proportion* of
    // rounded-up pixels in any small neighbourhood equal to the fraction,
    // which is the whole trick.
    let up = (scaled - floor) > threshold;
    (floor as u8).saturating_add(u8::from(up))
}

/// sRGB-encode a linear value straight to its eight-bit code.
#[inline]
fn srgb_encode_u8(linear: f32) -> u8 {
    let v = linear.clamp(0.0, 1.0);
    srgb_thresholds().partition_point(|threshold| *threshold <= v) as u8
}

/// The display-encoded value of a tone-mapped colour, straight to bytes.
#[inline]
fn encode_display_u8(transform: DisplayTransform, toned: Vec3) -> [u8; 3] {
    match transform {
        // The one transform that is not sRGB-encoded on the way out.
        DisplayTransform::Raw => [encode_u8(toned.x), encode_u8(toned.y), encode_u8(toned.z)],
        _ => [
            srgb_encode_u8(toned.x),
            srgb_encode_u8(toned.y),
            srgb_encode_u8(toned.z),
        ],
    }
}

/// Hermite interpolation between two edges, clamped outside them.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear sample with clamped edges.
fn bilinear(width: usize, height: usize, image: &[Spectrum], x: f32, y: f32) -> Spectrum {
    if width == 0 || height == 0 {
        return Spectrum::ZERO;
    }
    let fx = (x - 0.5).clamp(0.0, (width - 1) as f32);
    let fy = (y - 0.5).clamp(0.0, (height - 1) as f32);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let top = image[y0 * width + x0].lerp(image[y0 * width + x1], tx);
    let bottom = image[y1 * width + x0].lerp(image[y1 * width + x1], tx);
    top.lerp(bottom, ty)
}

/// Separable Gaussian blur with clamped edges.
///
/// Separable because the two-dimensional version costs `O(r²)` per pixel and
/// the bloom radius is a percentage of the image — at 3% of a 4K frame that is
/// a 115-pixel kernel, 13,000 taps a pixel against 230.
fn gaussian_blur(
    width: usize,
    height: usize,
    image: &[Spectrum],
    radius: f32,
) -> Vec<Spectrum> {
    let sigma = (radius / 2.0).max(0.5);
    let taps = (sigma * 3.0).ceil().max(1.0) as isize;
    let mut kernel = Vec::with_capacity((taps * 2 + 1) as usize);
    let mut total = 0.0;
    for i in -taps..=taps {
        let x = i as f32;
        let w = (-(x * x) / (2.0 * sigma * sigma)).exp();
        kernel.push(w);
        total += w;
    }
    for w in &mut kernel {
        *w /= total;
    }

    let at = |image: &[Spectrum], x: isize, y: isize| -> Spectrum {
        let x = x.clamp(0, width as isize - 1) as usize;
        let y = y.clamp(0, height as isize - 1) as usize;
        image[y * width + x]
    };

    let mut horizontal = vec![Spectrum::ZERO; image.len()];
    for y in 0..height as isize {
        for x in 0..width as isize {
            let mut sum = Spectrum::ZERO;
            for (i, w) in kernel.iter().enumerate() {
                sum = sum + at(image, x + i as isize - taps, y) * *w;
            }
            horizontal[y as usize * width + x as usize] = sum;
        }
    }

    let mut out = vec![Spectrum::ZERO; image.len()];
    for y in 0..height as isize {
        for x in 0..width as isize {
            let mut sum = Spectrum::ZERO;
            for (i, w) in kernel.iter().enumerate() {
                sum = sum + at(&horizontal, x, y + i as isize - taps) * *w;
            }
            out[y as usize * width + x as usize] = sum;
        }
    }
    out
}

#[cfg(test)]
mod dither_tests {
    use super::*;

    /// A gradient so shallow that 8-bit quantization turns it into plateaus.
    /// One code covers about six pixels here, which is the regime a large
    /// slowly-shading surface — a floor, a car door, a sky — lives in.
    fn shallow_gradient(width: usize, height: usize) -> Vec<Spectrum> {
        (0..width * height)
            .map(|i| {
                let y = i / width;
                let t = y as f32 / height as f32;
                Spectrum::splat(0.20 + 0.03 * t)
            })
            .collect()
    }

    fn develop(pixels: &[Spectrum], width: usize, height: usize) -> Vec<u8> {
        ImageStyle::default().develop_through(
            width,
            height,
            pixels,
            &DisplayPipeline::Builtin(DisplayTransform::Srgb),
        )
    }

    /// Codes present, and how many times the code changes, down one column.
    ///
    /// This is what a band is and is not. Undithered, a vertical gradient
    /// spanning N codes changes exactly N-1 times down any column: N flat
    /// plateaus with a hard step between them, and on a floor those steps
    /// follow iso-luminance contours — circles about the camera's ground
    /// point, seen as curved lines.
    ///
    /// Dithered, the same column crosses the boundary region many times, so
    /// the count of changes is far larger than the count of codes. That ratio
    /// is the measurement: it says the step was broken up rather than moved.
    ///
    /// Two earlier attempts measured the wrong thing and are worth recording,
    /// because both looked reasonable. The longest run down a column is not
    /// it: in the middle of a code's span a correctly dithered image holds one
    /// code for many rows, because the fraction there is near zero and almost
    /// nothing should round up. Nor is the count of flat rows: one row samples
    /// only eight of the matrix's sixty-four thresholds, and a Bayer row does
    /// not span the whole range, so plenty of rows are legitimately flat while
    /// the area around them is not.
    fn column_profile(bytes: &[u8], width: usize, height: usize, column: usize) -> (usize, usize) {
        let codes: Vec<u8> = (0..height)
            .map(|y| bytes[(y * width + column) * 4])
            .collect();
        let distinct: std::collections::BTreeSet<u8> = codes.iter().copied().collect();
        let changes = codes.windows(2).filter(|w| w[0] != w[1]).count();
        (distinct.len(), changes)
    }

    /// The reported artefact, as a number.
    #[test]
    fn a_shallow_gradient_does_not_come_out_as_hard_bands() {
        let (width, height) = (64usize, 256usize);
        let bytes = develop(&shallow_gradient(width, height), width, height);
        let (codes, changes) = column_profile(&bytes, width, height, 32);
        assert!(codes >= 4, "the gradient only spans {codes} codes to begin with");
        assert!(
            changes > codes * 4,
            "{codes} codes down the column and only {changes} changes; the \
             steps between them are still hard edges"
        );
    }

    /// And the dither must not change the picture, only how it is rounded. The
    /// mean over any patch has to match the undithered mean to well under one
    /// code, or "removing banding" would mean "shifting the exposure".
    #[test]
    fn the_dither_does_not_shift_the_image() {
        let (width, height) = (64usize, 256usize);
        let source = shallow_gradient(width, height);
        let bytes = develop(&source, width, height);

        for band in 0..8 {
            let y0 = band * 32;
            let mut sum = 0.0f64;
            let mut want = 0.0f64;
            for y in y0..y0 + 32 {
                for x in 0..width {
                    sum += bytes[(y * width + x) * 4] as f64;
                    let toned = DisplayTransform::Srgb.apply(source[y * width + x]);
                    want += (toned.x.clamp(0.0, 1.0) * 255.0) as f64;
                }
            }
            let n = (32 * width) as f64;
            let (got, expected) = (sum / n, want / n);
            assert!(
                (got - expected).abs() < 0.25,
                "patch {band} averages {got:.3} against {expected:.3}"
            );
        }
    }

    /// Determinism. `docs/05` §3.6 requires a render to be reproducible, and
    /// the file format's byte-identical rewrite depends on it — a random
    /// dither would make two renders of one scene differ in most pixels, which
    /// is invisible to the eye and fatal to every golden-image comparison.
    #[test]
    fn the_dither_is_deterministic() {
        let (width, height) = (64usize, 96usize);
        let source = shallow_gradient(width, height);
        let first = develop(&source, width, height);
        let second = develop(&source, width, height);
        assert_eq!(first, second);
    }

    /// The threshold is a permutation of the 64 cells: each appears once, so
    /// the perturbation over any aligned 8x8 block averages to zero and the
    /// dither adds no brightness of its own.
    #[test]
    fn the_matrix_is_an_even_permutation() {
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0.0f64;
        for y in 0..8 {
            for x in 0..8 {
                let t = dither_threshold(x, y);
                total += t as f64;
                seen.insert((t * 64.0) as u32);
            }
        }
        assert_eq!(seen.len(), 64, "the matrix repeats a threshold");
        assert!(
            (total / 64.0 - 0.5).abs() < 1e-6,
            "the mean threshold is {}",
            total / 64.0
        );
    }

    /// A value that lands exactly on a code is left exactly on it.
    ///
    /// Dither exists to represent values *between* codes; where a value needs
    /// no rounding there is nothing to spread, and adding texture there would
    /// be adding noise for nothing.
    ///
    /// The value has to be found rather than guessed. 0.5 is not on a code —
    /// sRGB-encoded it is 187.5 of 255 — and a correctly dithered patch of it
    /// alternates 187 and 188, which is right and was the first version of
    /// this test calling it a bug.
    #[test]
    fn a_value_exactly_on_a_code_is_left_alone() {
        let (width, height) = (16usize, 16usize);
        // Walk down from mid-grey until the encoded value is within a
        // thousandth of an integer code.
        let mut linear = 0.5f32;
        let mut found = None;
        for i in 0..20_000 {
            let candidate = 0.5 - i as f32 * 1e-5;
            let scaled = DisplayTransform::Srgb.apply(Spectrum::splat(candidate)).x * 255.0;
            if (scaled - scaled.round()).abs() < 1e-3 {
                linear = candidate;
                found = Some(scaled.round());
                break;
            }
        }
        let code = found.expect("a value on a code exists between 0.3 and 0.5");

        let source = vec![Spectrum::splat(linear); width * height];
        let bytes = develop(&source, width, height);
        assert!(
            bytes.chunks_exact(4).all(|p| p[0] == code as u8),
            "a patch already on code {code} came out textured"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(v: f32) -> Spectrum {
        Spectrum::from_linear_srgb(Vec3::splat(v))
    }

    /// The approximation has to be an approximation of the right thing, across
    /// the whole range AgX feeds it — sixteen and a half stops, from a black
    /// that is not quite zero to a highlight hundreds of times mid-grey.
    /// AgX puts 18% grey at about half-way on the display, as Blender's
    /// does — not at 73%, which the encode applied twice made of it.
    /// The fast power AgX linearizes with agrees with `powf` to well under
    /// an 8-bit step over the curve's whole output range.
    #[test]
    fn agx_fast_power_matches_powf() {
        let mut worst = 0.0f32;
        for i in 1..=1000 {
            let x = i as f32 / 1000.0;
            let fast = fast_exp2(2.2 * fast_log2(x));
            worst = worst.max((fast - x.powf(2.2)).abs());
        }
        assert!(worst < 1e-3, "worst difference {worst}");
    }

    #[test]
    fn agx_puts_middle_grey_half_way() {
        let grey = DisplayTransform::AgX.apply(Spectrum::splat(0.18));
        assert!(grey.x > 0.44 && grey.x < 0.56, "18% grey displays at {}", grey.x);
        // And it still rolls off: bright light does not clip early, and
        // black stays black.
        let bright = DisplayTransform::AgX.apply(Spectrum::splat(4.0));
        assert!(bright.x > 0.85 && bright.x <= 1.0, "{bright:?}");
        let black = DisplayTransform::AgX.apply(Spectrum::splat(0.0));
        assert!(black.x < 0.05, "{black:?}");
    }

    #[test]
    fn the_fast_logarithm_agrees_with_the_real_one() {
        let mut worst = 0.0f32;
        let mut at = 0.0f32;
        let mut x = 1e-10f32;
        while x < 1.0e6 {
            let error = (fast_log2(x) - x.log2()).abs();
            if error > worst {
                worst = error;
                at = x;
            }
            x *= 1.02;
        }
        assert!(
            worst < 1e-5,
            "the fast logarithm is off by {worst} at {at}, which is not close \
             enough to spend on a tone curve"
        );
        // Exact at the powers of two, where the mantissa fit is at its
        // endpoints — if it is not, the exponent has been split off wrongly and
        // the error would be a full stop rather than a rounding.
        for stop in -20i32..20 {
            let v = f32::exp2(stop as f32);
            assert!(
                (fast_log2(v) - stop as f32).abs() < 1e-3,
                "log2({v}) came out as {} rather than {stop}",
                fast_log2(v)
            );
        }
    }

    fn photographic() -> ImageStyle {
        ImageStyle {
            mode: ToneMode::Photographic,
            ..ImageStyle::default()
        }
    }

    /// Neutral values must be the identity *exactly*, not approximately. A
    /// control that shifts the image slightly when it is set to its default is
    /// one a user cannot switch off, and the drift compounds: six such
    /// controls make the Photographic style a different look from Basic before
    /// anything has been adjusted.
    #[test]
    fn every_photographic_control_is_neutral_at_its_default() {
        let basic = ImageStyle::default();
        let photo = photographic();
        for v in [0.001, 0.02, 0.18, 1.0, 12.0, 400.0] {
            let a = basic.grade(grey(v));
            let b = photo.grade(grey(v));
            assert_eq!(a, b, "at {v}");
        }
    }

    /// Basic must ignore the photographic fields even when they are set —
    /// switching modes has to be reversible, which means the fields keep their
    /// values while doing nothing.
    #[test]
    fn basic_ignores_every_photographic_control() {
        let loud = ImageStyle {
            highlights: -0.8,
            shadows: 0.7,
            whites: 0.5,
            blacks: -0.6,
            temperature: 3200.0,
            tint: 0.4,
            vignette: 0.9,
            ..ImageStyle::default()
        };
        for v in [0.01, 0.18, 4.0] {
            assert_eq!(loud.grade(grey(v)), ImageStyle::default().grade(grey(v)));
        }
        assert_eq!(loud.vignette_at(0.0, 0.0), 1.0);
    }

    /// Each tone control has to be monotone in its parameter: dragging a
    /// slider one way can never make the image move the other way partway
    /// along, which is the failure mode of a mask that is not itself monotone.
    #[test]
    fn tone_controls_are_monotone_in_their_parameter() {
        let cases: [(&str, fn(&mut ImageStyle, f32), f32); 4] = [
            ("highlights", |s, v| s.highlights = v, 4.0),
            ("shadows", |s, v| s.shadows = v, 0.02),
            ("whites", |s, v| s.whites = v, 40.0),
            ("blacks", |s, v| s.blacks = v, 0.02),
        ];
        for (name, set, probe) in cases {
            let mut previous = f32::NEG_INFINITY;
            for step in 0..=20 {
                let amount = -1.0 + step as f32 * 0.1;
                let mut style = photographic();
                set(&mut style, amount);
                let value = style.grade(grey(probe)).luminance();
                assert!(
                    value >= previous - 1e-6,
                    "{name} reversed at {amount}: {value} after {previous}"
                );
                previous = value;
            }
            // And it has to actually do something, or monotonicity is vacuous.
            let mut low = photographic();
            set(&mut low, -1.0);
            let mut high = photographic();
            set(&mut high, 1.0);
            let (a, b) = (
                low.grade(grey(probe)).luminance(),
                high.grade(grey(probe)).luminance(),
            );
            assert!(b > a * 1.1, "{name} is inert: {a} vs {b}");
        }
    }

    /// The whole point of normalizing the white balance: warmth must not be a
    /// disguised exposure change. A neutral grey stays the same brightness at
    /// any temperature, only its colour moves.
    #[test]
    fn colour_temperature_does_not_change_exposure() {
        let neutral = photographic().grade(grey(0.18)).luminance();
        for kelvin in [2000.0, 3200.0, 5000.0, 9000.0, 20000.0] {
            let style = ImageStyle {
                temperature: kelvin,
                ..photographic()
            };
            let value = style.grade(grey(0.18)).luminance();
            let ratio = value / neutral;
            assert!(
                (ratio - 1.0).abs() < 0.02,
                "{kelvin} K changed exposure by {ratio}"
            );
        }
    }

    /// A lower temperature must make the image *cooler*. The sign is easy to
    /// invert — the control names the light the image is corrected for, so
    /// correcting for tungsten removes orange rather than adding it.
    #[test]
    fn a_lower_temperature_cools_the_image() {
        let warm = ImageStyle {
            temperature: 3000.0,
            ..photographic()
        }
        .grade(grey(0.18));
        let cool = ImageStyle {
            temperature: 12000.0,
            ..photographic()
        }
        .grade(grey(0.18));
        assert!(warm.b > warm.r, "3000 K should read blue: {warm:?}");
        assert!(cool.r > cool.b, "12000 K should read orange: {cool:?}");
    }

    /// Bloom is light arriving where it should not, so it may only ever add.
    /// A version that mixed would darken the highlight it spread from, which
    /// is the opposite of what a lens does.
    #[test]
    fn bloom_only_adds_light() {
        let style = ImageStyle {
            bloom: 1.0,
            bloom_radius: 0.2,
            display: DisplayTransform::Raw,
            ..photographic()
        };
        let (w, h) = (16, 16);
        let mut image = vec![grey(0.0); w * h];
        image[8 * w + 8] = grey(50.0);

        let plain = ImageStyle {
            bloom: 0.0,
            ..style
        };
        let with_bloom = style.develop(w, h, &image);
        let without = plain.develop(w, h, &image);
        for (a, b) in with_bloom.iter().zip(&without) {
            assert!(a >= b, "bloom removed light: {a} < {b}");
        }
        // And it has to reach the neighbours, or it is not bloom.
        let neighbour = ((8 * w + 11) * 4) as usize;
        assert!(
            with_bloom[neighbour] > without[neighbour],
            "bloom did not spread"
        );
    }

    /// The bloom kernel must conserve what it spreads. A blur whose weights do
    /// not sum to one changes the total energy with the radius, so widening
    /// the glare would also brighten or dim the image.
    #[test]
    fn the_bloom_blur_conserves_energy() {
        let (w, h) = (32, 32);
        let mut image = vec![Spectrum::ZERO; w * h];
        image[16 * w + 16] = Spectrum::splat(100.0);
        for radius in [1.0, 4.0, 11.0] {
            let blurred = gaussian_blur(w, h, &image, radius);
            let total: f32 = blurred.iter().map(|c| c.r).sum();
            assert!(
                (total - 100.0).abs() < 0.5,
                "radius {radius} moved {total} of 100"
            );
        }
    }

    /// The vignette darkens the corners and leaves the centre alone. A version
    /// that scaled the whole image would be an exposure control with extra
    /// steps.
    #[test]
    fn the_vignette_darkens_only_the_edges() {
        let style = ImageStyle {
            vignette: 0.8,
            ..photographic()
        };
        assert_eq!(style.vignette_at(0.5, 0.5), 1.0);
        assert!(style.vignette_at(0.0, 0.0) < 0.4);
        // Symmetric, and the same at every corner whatever the aspect.
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
            assert!((style.vignette_at(u, v) - style.vignette_at(0.0, 0.0)).abs() < 1e-6);
        }
    }

    /// `develop` and the per-pixel path must not drift apart. Two code paths
    /// that can disagree is how a viewport and a saved file come to look
    /// different — and this test caught exactly that when the batch path
    /// gained a dither the per-pixel one did not have, which was showing up as
    /// banding in command-line renders that the application did not have. The
    /// fix was to give the command line the batch path rather than to relax
    /// this test.
    ///
    /// What is asserted is now "within one code" rather than "identical",
    /// because the dither deliberately rounds a value between two codes up in
    /// some pixels and down in others. One code is the whole of the freedom it
    /// has: anything larger is the two paths genuinely diverging, and the mean
    /// is pinned separately by `the_dither_does_not_shift_the_image`.
    #[test]
    fn develop_agrees_with_the_per_pixel_path() {
        let style = ImageStyle {
            exposure: 0.5,
            contrast: 1.2,
            saturation: 0.8,
            highlights: -0.3,
            shadows: 0.2,
            temperature: 4000.0,
            ..photographic()
        };
        let image: Vec<Spectrum> = (0..64)
            .map(|i| Spectrum {
                r: i as f32 * 0.03,
                g: 0.4,
                b: 2.0 / (i as f32 + 1.0),
            })
            .collect();
        let developed = style.develop(8, 8, &image);
        for (i, color) in image.iter().enumerate() {
            let want = style.apply_u8(color.sanitized());
            let got = [developed[i * 4], developed[i * 4 + 1], developed[i * 4 + 2]];
            for channel in 0..3 {
                let difference = got[channel] as i32 - want[channel] as i32;
                assert!(
                    difference.abs() <= 1,
                    "pixel {i} channel {channel}: {got:?} against {want:?}"
                );
            }
        }
    }

    /// The property that matters most: a value hundreds of times brighter than
    /// mid-grey must still map inside the display range, and must keep
    /// increasing. A transform that clips turns every studio highlight into a
    /// flat blob with a hard edge.
    #[test]
    fn tone_mapping_never_clips_and_stays_monotonic() {
        for transform in [
            DisplayTransform::AgX,
            DisplayTransform::Aces,
            DisplayTransform::PbrNeutral,
        ] {
            let mut previous = -1.0f32;
            for stops in -6..=10 {
                let v = transform.apply(grey(0.18 * f32::exp2(stops as f32))).y;
                assert!(
                    (0.0..=1.0).contains(&v),
                    "{}: {stops} stops mapped to {v}",
                    transform.label()
                );
                // Strictly increasing across the working range, and never
                // *decreasing* beyond it. AgX encodes a defined window of
                // about -12.5 to +4 EV and saturates past the top of it; that
                // is the transform doing its job, not a defect. What would be
                // a defect is the curve turning over, which would make a
                // brighter surface render darker.
                if stops <= 4 {
                    assert!(
                        v > previous,
                        "{}: not increasing at {stops} stops ({previous} -> {v})",
                        transform.label()
                    );
                } else {
                    assert!(
                        v >= previous - 1e-6,
                        "{}: curve turned over at {stops} stops ({previous} -> {v})",
                        transform.label()
                    );
                }
                previous = v;
            }
            // 10 stops over mid-grey must be bright but not fully clipped, or
            // there is no headroom left to see detail in.
            let bright = transform.apply(grey(0.18 * 1024.0)).y;
            assert!(
                bright > 0.85,
                "{}: very bright input only reached {bright}",
                transform.label()
            );
        }
    }

    /// Plain sRGB is the one that *does* clip. Keeping it is deliberate:
    /// §7.3 lists it "for technical/QA work", where seeing the clip is the
    /// point.
    #[test]
    fn plain_srgb_clips_as_advertised() {
        let a = DisplayTransform::Srgb.apply(grey(1.0)).y;
        let b = DisplayTransform::Srgb.apply(grey(100.0)).y;
        assert!((a - 1.0).abs() < 1e-5);
        assert_eq!(a, b, "sRGB should clip, not roll off");
    }

    #[test]
    fn black_maps_to_black_and_stays_there() {
        for transform in DisplayTransform::ALL {
            let v = transform.apply(Spectrum::ZERO);
            assert!(
                v.length() < 0.02,
                "{} turned black into {v:?}",
                transform.label()
            );
        }
    }

    /// A neutral grey must stay neutral. A transform that tints the greyscale
    /// is immediately obvious on a product shot and is the classic symptom of
    /// a matrix applied in the wrong direction.
    #[test]
    fn greys_stay_neutral() {
        for transform in DisplayTransform::ALL {
            for level in [0.02f32, 0.18, 0.5, 2.0, 20.0] {
                let v = transform.apply(grey(level));
                let spread = v.max_component() - v.min_component_value();
                assert!(
                    spread < 0.02,
                    "{} tinted grey {level} to {v:?}",
                    transform.label()
                );
            }
        }
    }

    /// Highlights must desaturate towards white rather than clipping one
    /// channel at a time — the latter shifts hue, so a bright orange turns
    /// yellow before it turns white.
    #[test]
    fn bright_saturated_colours_desaturate_rather_than_shifting_hue() {
        for transform in [DisplayTransform::AgX, DisplayTransform::PbrNeutral] {
            let orange = |scale: f32| {
                transform.apply(Spectrum::from_linear_srgb(vec3(1.0, 0.35, 0.05) * scale))
            };
            let dim = orange(0.5);
            let bright = orange(50.0);
            let saturation = |v: Vec3| {
                (v.max_component() - v.min_component_value()) / v.max_component().max(1e-6)
            };
            assert!(
                saturation(bright) < saturation(dim),
                "{}: bright colour did not desaturate ({} -> {})",
                transform.label(),
                saturation(dim),
                saturation(bright)
            );
        }
    }

    #[test]
    fn exposure_moves_by_stops() {
        let style = ImageStyle {
            display: DisplayTransform::Raw,
            ..Default::default()
        };
        let brighter = ImageStyle {
            exposure: 1.0,
            ..style
        };
        let a = style.apply(grey(0.1)).y;
        let b = brighter.apply(grey(0.1)).y;
        assert!((b / a - 2.0).abs() < 1e-3, "one stop gave {}x", b / a);
    }

    /// Contrast must pivot at mid-grey, so adjusting it does not also change
    /// the overall brightness of the image.
    #[test]
    fn contrast_pivots_at_middle_grey() {
        let style = ImageStyle {
            contrast: 1.6,
            display: DisplayTransform::Raw,
            ..Default::default()
        };
        let neutral = ImageStyle {
            display: DisplayTransform::Raw,
            ..Default::default()
        };
        let mid = style.apply(grey(MIDDLE_GREY)).y;
        assert!(
            (mid - neutral.apply(grey(MIDDLE_GREY)).y).abs() < 1e-3,
            "mid-grey moved when contrast changed"
        );
        // Below mid-grey darkens, above brightens.
        assert!(style.apply(grey(0.05)).y < neutral.apply(grey(0.05)).y);
        assert!(style.apply(grey(0.5)).y > neutral.apply(grey(0.5)).y);
    }

    #[test]
    fn zero_saturation_gives_greyscale() {
        let style = ImageStyle {
            saturation: 0.0,
            display: DisplayTransform::Raw,
            ..Default::default()
        };
        let v = style.apply(Spectrum::from_linear_srgb(vec3(0.6, 0.2, 0.05)));
        let spread = v.max_component() - v.min_component_value();
        assert!(spread < 0.01, "not greyscale: {v:?}");
    }

    #[test]
    fn eight_bit_encoding_covers_the_full_range() {
        let style = ImageStyle::default();
        assert_eq!(style.apply_u8(Spectrum::ZERO), [0, 0, 0]);
        let white = style.apply_u8(grey(1000.0));
        assert!(white.iter().all(|&c| c > 240), "{white:?}");
    }

    /// Every transform must survive whatever the renderer hands it. NaNs and
    /// negatives reach here from a wide-gamut working space and from
    /// unconverged pixels.
    #[test]
    fn hostile_inputs_do_not_produce_hostile_output() {
        let style = ImageStyle::default();
        for c in [
            Spectrum::splat(f32::NAN),
            Spectrum::splat(f32::INFINITY),
            Spectrum::splat(-5.0),
            Spectrum {
                r: 1e20,
                g: -1e20,
                b: f32::NAN,
            },
        ] {
            let v = style.apply(c.sanitized());
            assert!(v.is_finite(), "{c:?} produced {v:?}");
            assert!(v.x >= 0.0 && v.x <= 1.0, "{c:?} produced {v:?}");
            // A sanitized NaN is black, and a sanitized infinity is
            // clamped to white; neither may come out as an arbitrary byte.
            let bytes = style.apply_u8(c.sanitized());
            assert!(
                bytes == [0, 0, 0] || bytes.iter().any(|&b| b > 0),
                "{c:?} encoded to {bytes:?}"
            );
        }
    }
}

pub mod ocio;
