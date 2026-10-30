//! An OpenColorIO v2 config, read and applied (`docs/01-RENDERER-CORE.md` §7).
//!
//! §7 asks for an OCIO v2 pipeline so that a studio can point Tequila at the
//! config the rest of its pipeline uses and get the same picture. That is the
//! whole value of OCIO and it is worth being precise about: the point is not to
//! have more tone curves, it is that the curve is *the one the compositor will
//! use*, read from the same file.
//!
//! # What this implements, and what it does not
//!
//! A config is a graph of colour spaces, each with a transform to or from a
//! common reference space, plus a list of (display, view) pairs naming the
//! space an image should be converted to for a given monitor and look. This
//! module reads that graph and evaluates it.
//!
//! An OCIO v2 config has two references, not one: scene-referred spaces are
//! defined against the scene reference (ACES2065-1 in an ACES config) and
//! display-referred ones, in `display_colorspaces`, against the display
//! connection space (CIE XYZ D65). A view crosses from one to the other
//! through a *view transform*, which is where the ACES output transforms live.
//! Both references, view transforms, shared views, looks, aliases, roles,
//! data spaces and the `active_displays` / `active_views` lists are read.
//!
//! Supported transforms: `FileTransform` (`.cube`, `.spi1d`, `.spimtx`),
//! `MatrixTransform`, `ExponentTransform`, `ExponentWithLinearTransform`,
//! `RangeTransform`, `LogTransform`, `LogAffineTransform`,
//! `LogCameraTransform`, `CDLTransform`, `GroupTransform`,
//! `ColorSpaceTransform` and `BuiltinTransform`, each in both directions
//! where OCIO defines both.
//!
//! `BuiltinTransform` is the registry OCIO v2 moved the ACES transforms into,
//! defined by the library rather than by any file; [`builtin`] implements it
//! from the ACES CTL and the camera vendors' published formulas. What it does
//! not implement — the ACES 2.0 output transforms of OCIO 2.4, chiefly — is
//! *reported*, not silently mis-rendered: [`Config::view_pipeline`] returns
//! the names of the transforms it could not build, the application says so,
//! and the analytic transforms of [`crate::DisplayTransform`] remain
//! available.
//!
//! The analytic transforms were written first for this reason. `docs/05` §1
//! note 3: "M0 implements the same display transforms analytically ... OCIO
//! arrives at M3; the analytic versions then serve as the reference to check
//! it against" — and
//! [`tests::a_config_that_encodes_srgb_matches_the_analytic_transform`] is
//! that check.

pub mod aces;
pub mod builtin;
pub mod curves;
pub mod lut;
pub mod primaries;
pub mod yaml;

use crate::ocio::aces::Tonescale;
use crate::ocio::curves::Curve;
use crate::ocio::lut::Lut;
use crate::ocio::yaml::Yaml;
use tequila_core::math::{vec3, Vec3};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One step of a colour conversion.
#[derive(Clone, Debug)]
pub enum Op {
    Lut(Arc<Lut>),
    Matrix { matrix: [f32; 9], offset: Vec3 },
    /// Per-channel power.
    Exponent { value: [f32; 3] },
    /// The "monitor curve": a power above a break point and a straight line
    /// below it. Every real gamma curve has this shape, sRGB included, and the
    /// straight segment is not a detail — a pure power has infinite slope at
    /// zero, which quantizes the darkest code values into a cliff.
    ///
    /// The forward direction is *encoded to linear*, which is how OCIO defines
    /// it; a config writes `direction: inverse` to get linear to encoded, which
    /// is what a display transform ends with.
    MonCurve {
        gamma: [f32; 3],
        offset: [f32; 3],
        inverse: bool,
    },
    Range {
        min_in: f32,
        max_in: f32,
        min_out: f32,
        max_out: f32,
        clamp: bool,
    },
    /// Linear to log, base `base`, as OCIO's `LogTransform` defines it; log
    /// to linear when `inverse`.
    Log { base: f32, inverse: bool },
    /// OCIO's `LogAffineTransform`, and `LogCameraTransform` when it has a
    /// linear segment.
    LogCamera(Box<LogCamera>),
    /// ASC CDL: slope, offset, power, saturation.
    Cdl {
        slope: [f32; 3],
        offset: [f32; 3],
        power: [f32; 3],
        saturation: f32,
    },
    /// Every channel clamped to `[lo, hi]`, either bound possibly infinite.
    /// What a clamp cuts off cannot be recovered, so its inverse is the same
    /// clamp, as in OCIO.
    Clamp { lo: f32, hi: f32 },
    /// One of the published curves of [`curves`], per channel.
    Curve { curve: Curve, to_linear: bool },
    /// The colour scaled by a power of its own luminance, `c * Y^(gamma - 1)`:
    /// a change of luminance at constant chromaticity. The ACES dim-surround
    /// compensation and BT.2100's HLG system gamma are both this.
    LumaPower {
        weights: [f32; 3],
        gamma: f32,
        /// The luminance is held at least this, so black stays black.
        floor: f32,
    },
    /// The ACES RRT's glow and red modifier, in ACES2065-1.
    AcesSweeteners { inverse: bool },
    /// An ACES tone scale, per channel in AP1.
    AcesTonescale {
        tonescale: Arc<Tonescale>,
        inverse: bool,
    },
    /// The ACES 1.3 Reference Gamut Compression, on linear AP1.
    AcesGamutCompress { inverse: bool },
    /// The ACES white roll-off for the DCI-white simulation outputs.
    RollWhite {
        new_white: f32,
        width: f32,
        inverse: bool,
    },
}

/// The parameters of a `LogAffineTransform` or `LogCameraTransform`.
///
/// Forward is linear to log, `log_slope * log_base(lin_slope * x +
/// lin_offset) + log_offset`, with a straight segment below `lin_break` for
/// the camera form. The straight segment's slope defaults to the one that
/// meets the log curve with a continuous derivative, as OCIO computes it.
#[derive(Clone, Debug, PartialEq)]
pub struct LogCamera {
    pub base: f32,
    pub log_slope: [f32; 3],
    pub log_offset: [f32; 3],
    pub lin_slope: [f32; 3],
    pub lin_offset: [f32; 3],
    pub lin_break: Option<[f32; 3]>,
    pub linear_slope: Option<[f32; 3]>,
    pub inverse: bool,
}

impl LogCamera {
    fn log(&self, x: f32, i: usize) -> f32 {
        self.log_slope[i] * (self.lin_slope[i] * x + self.lin_offset[i]).max(1e-10).ln() / self.base.ln()
            + self.log_offset[i]
    }

    fn antilog(&self, y: f32, i: usize) -> f32 {
        (self.base.powf((y - self.log_offset[i]) / self.log_slope[i]) - self.lin_offset[i])
            / self.lin_slope[i]
    }

    /// The break point and the straight segment's slope there.
    fn segment(&self, i: usize) -> Option<(f32, f32)> {
        let lin_break = self.lin_break?[i];
        let slope = match self.linear_slope {
            Some(s) => s[i],
            None => {
                self.log_slope[i] * self.lin_slope[i]
                    / ((self.lin_slope[i] * lin_break + self.lin_offset[i]) * self.base.ln())
            }
        };
        Some((lin_break, slope))
    }

    fn apply(&self, v: f32, i: usize) -> f32 {
        match (self.segment(i), self.inverse) {
            (None, false) => self.log(v, i),
            (None, true) => self.antilog(v, i),
            (Some((lin_break, slope)), false) => {
                if v <= lin_break {
                    slope * (v - lin_break) + self.log(lin_break, i)
                } else {
                    self.log(v, i)
                }
            }
            (Some((lin_break, slope)), true) => {
                let log_break = self.log(lin_break, i);
                if v <= log_break {
                    (v - log_break) / slope + lin_break
                } else {
                    self.antilog(v, i)
                }
            }
        }
    }
}

impl Op {
    pub fn apply(&self, c: Vec3) -> Vec3 {
        let per_channel = |f: &dyn Fn(f32) -> f32| vec3(f(c.x), f(c.y), f(c.z));
        match self {
            Op::Lut(lut) => lut.apply(c),
            Op::Matrix { matrix, offset } => vec3(
                matrix[0] * c.x + matrix[1] * c.y + matrix[2] * c.z + offset.x,
                matrix[3] * c.x + matrix[4] * c.y + matrix[5] * c.z + offset.y,
                matrix[6] * c.x + matrix[7] * c.y + matrix[8] * c.z + offset.z,
            ),
            Op::Exponent { value } => {
                // Mirrored about zero rather than clamped, so a negative value
                // — which an ACEScg colour outside the sRGB gamut genuinely has
                // — comes back negative instead of being quietly flattened.
                let channel = |v: f32, gamma: f32| {
                    if v < 0.0 {
                        -(-v).powf(gamma)
                    } else {
                        v.powf(gamma)
                    }
                };
                vec3(
                    channel(c.x, value[0]),
                    channel(c.y, value[1]),
                    channel(c.z, value[2]),
                )
            }
            Op::MonCurve {
                gamma,
                offset,
                inverse,
            } => vec3(
                mon_curve(c.x, gamma[0], offset[0], *inverse),
                mon_curve(c.y, gamma[1], offset[1], *inverse),
                mon_curve(c.z, gamma[2], offset[2], *inverse),
            ),
            Op::Range {
                min_in,
                max_in,
                min_out,
                max_out,
                clamp,
            } => {
                let span = (max_in - min_in).abs().max(1e-9);
                let scale = (max_out - min_out) / span;
                let map = |v: f32| {
                    let out = (v - min_in) * scale + min_out;
                    if *clamp {
                        out.clamp(min_out.min(*max_out), min_out.max(*max_out))
                    } else {
                        out
                    }
                };
                vec3(map(c.x), map(c.y), map(c.z))
            }
            Op::Log { base, inverse } => {
                let base = base.max(1.0001);
                if *inverse {
                    per_channel(&|v| base.powf(v))
                } else {
                    let ln_base = base.ln();
                    per_channel(&|v| v.max(1e-10).ln() / ln_base)
                }
            }
            Op::LogCamera(log) => vec3(log.apply(c.x, 0), log.apply(c.y, 1), log.apply(c.z, 2)),
            Op::Cdl {
                slope,
                offset,
                power,
                saturation,
            } => {
                let graded = |v: f32, i: usize| {
                    let t = (v * slope[i] + offset[i]).max(0.0);
                    t.powf(power[i])
                };
                let out = vec3(graded(c.x, 0), graded(c.y, 1), graded(c.z, 2));
                // Rec.709 luma, which is what the ASC CDL specifies.
                let luma = 0.2126 * out.x + 0.7152 * out.y + 0.0722 * out.z;
                vec3(
                    luma + saturation * (out.x - luma),
                    luma + saturation * (out.y - luma),
                    luma + saturation * (out.z - luma),
                )
            }
            Op::Clamp { lo, hi } => c.clamp(*lo, *hi),
            Op::Curve { curve, to_linear } => {
                if *to_linear {
                    per_channel(&|v| curve.to_linear(v))
                } else {
                    per_channel(&|v| curve.from_linear(v))
                }
            }
            Op::LumaPower {
                weights,
                gamma,
                floor,
            } => {
                let y = (weights[0] * c.x + weights[1] * c.y + weights[2] * c.z).max(*floor);
                c * y.powf(gamma - 1.0)
            }
            Op::AcesSweeteners { inverse } => {
                if *inverse {
                    aces::sweeteners_inverse(c)
                } else {
                    aces::sweeteners_forward(c)
                }
            }
            Op::AcesTonescale { tonescale, inverse } => {
                if *inverse {
                    per_channel(&|v| tonescale.inverse(v))
                } else {
                    per_channel(&|v| tonescale.forward(v))
                }
            }
            Op::AcesGamutCompress { inverse } => aces::gamut_compress(c, *inverse),
            Op::RollWhite {
                new_white,
                width,
                inverse,
            } => {
                if *inverse {
                    per_channel(&|v| aces::roll_white_inverse(v, *new_white, *width))
                } else {
                    per_channel(&|v| aces::roll_white_forward(v, *new_white, *width))
                }
            }
        }
    }

    /// The inverse, where there is one in closed form.
    ///
    /// A LUT has no closed-form inverse, and inverting one numerically is a
    /// different feature with its own accuracy questions. Returning `None` here
    /// is what lets [`Config::view_pipeline`] *report* that it cannot build a
    /// direction rather than build a wrong one.
    pub fn inverse(&self) -> Option<Op> {
        match self {
            Op::Matrix { matrix, offset } => {
                let inverse = invert3(matrix)?;
                let back = vec3(
                    -(inverse[0] * offset.x + inverse[1] * offset.y + inverse[2] * offset.z),
                    -(inverse[3] * offset.x + inverse[4] * offset.y + inverse[5] * offset.z),
                    -(inverse[6] * offset.x + inverse[7] * offset.y + inverse[8] * offset.z),
                );
                Some(Op::Matrix {
                    matrix: inverse,
                    offset: back,
                })
            }
            Op::Exponent { value } => Some(Op::Exponent {
                value: [1.0 / value[0], 1.0 / value[1], 1.0 / value[2]],
            }),
            Op::MonCurve {
                gamma,
                offset,
                inverse,
            } => Some(Op::MonCurve {
                gamma: *gamma,
                offset: *offset,
                inverse: !inverse,
            }),
            Op::Range {
                min_in,
                max_in,
                min_out,
                max_out,
                clamp,
            } => Some(Op::Range {
                min_in: *min_out,
                max_in: *max_out,
                min_out: *min_in,
                max_out: *max_in,
                clamp: *clamp,
            }),
            Op::Log { base, inverse } => Some(Op::Log {
                base: *base,
                inverse: !inverse,
            }),
            Op::LogCamera(log) => Some(Op::LogCamera(Box::new(LogCamera {
                inverse: !log.inverse,
                ..(**log).clone()
            }))),
            Op::Clamp { lo, hi } => Some(Op::Clamp { lo: *lo, hi: *hi }),
            Op::Curve { curve, to_linear } => Some(Op::Curve {
                curve: *curve,
                to_linear: !to_linear,
            }),
            Op::LumaPower {
                weights,
                gamma,
                floor,
            } => Some(Op::LumaPower {
                weights: *weights,
                gamma: 1.0 / gamma,
                floor: floor.powf(*gamma),
            }),
            Op::AcesSweeteners { inverse } => Some(Op::AcesSweeteners { inverse: !inverse }),
            Op::AcesTonescale { tonescale, inverse } => Some(Op::AcesTonescale {
                tonescale: tonescale.clone(),
                inverse: !inverse,
            }),
            Op::AcesGamutCompress { inverse } => Some(Op::AcesGamutCompress { inverse: !inverse }),
            Op::RollWhite {
                new_white,
                width,
                inverse,
            } => Some(Op::RollWhite {
                new_white: *new_white,
                width: *width,
                inverse: !inverse,
            }),
            Op::Lut(_) | Op::Cdl { .. } => None,
        }
    }
}

/// The monitor curve, forward (encoded to linear) or inverse.
///
/// The break point is `offset / (gamma - 1)` because that is the only place a
/// straight line can meet the power curve with *both* the value and the slope
/// continuous — a curve joined on value alone has a visible kink in the
/// shadows, and one joined on neither is not a monitor curve at all. The slope
/// follows from the join rather than being a second constant to keep in step,
/// which is why sRGB's famous 12.92 does not appear anywhere here: it falls out
/// of gamma 2.4 and offset 0.055.
fn mon_curve(v: f32, gamma: f32, offset: f32, inverse: bool) -> f32 {
    if gamma <= 1.0 || offset <= 0.0 {
        // Degenerates to a plain power, which is what a config asking for
        // gamma 1 or no offset means.
        let g = if inverse { 1.0 / gamma.max(1e-6) } else { gamma };
        return if v < 0.0 { -(-v).powf(g) } else { v.powf(g) };
    }
    let break_in = offset / (gamma - 1.0);
    let slope = ((break_in + offset) / (1.0 + offset)).powf(gamma) / break_in;
    let break_out = slope * break_in;

    if inverse {
        // Linear to encoded.
        if v < 0.0 {
            return v / slope;
        }
        if v >= break_out {
            (1.0 + offset) * v.powf(1.0 / gamma) - offset
        } else {
            v / slope
        }
    } else {
        // Encoded to linear.
        if v < 0.0 {
            return v * slope;
        }
        if v >= break_in {
            ((v + offset) / (1.0 + offset)).powf(gamma)
        } else {
            v * slope
        }
    }
}

fn invert3(m: &[f32; 9]) -> Option<[f32; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * inv,
        (m[2] * m[7] - m[1] * m[8]) * inv,
        (m[1] * m[5] - m[2] * m[4]) * inv,
        (m[5] * m[6] - m[3] * m[8]) * inv,
        (m[0] * m[8] - m[2] * m[6]) * inv,
        (m[2] * m[3] - m[0] * m[5]) * inv,
        (m[3] * m[7] - m[4] * m[6]) * inv,
        (m[1] * m[6] - m[0] * m[7]) * inv,
        (m[0] * m[4] - m[1] * m[3]) * inv,
    ])
}

/// A built conversion: a list of ops applied in order.
#[derive(Clone, Debug, Default)]
pub struct Processor {
    ops: Vec<Op>,
    /// Transforms in the config this build could not construct. Empty means the
    /// pipeline is complete.
    pub unsupported: Vec<String>,
}

impl Processor {
    pub fn apply(&self, c: Vec3) -> Vec3 {
        self.ops.iter().fold(c, |c, op| op.apply(c))
    }

    pub fn is_complete(&self) -> bool {
        self.unsupported.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Append an op, folding it into the previous one when both are matrices.
    ///
    /// Not a micro-optimization: an ACES view chains a colour space's matrix,
    /// the output transform's last matrices and the display's first, and the
    /// develop pass runs every op on two million pixels. Multiplying
    /// neighbouring matrices once here takes three or four matrix products
    /// per pixel down to one, and changes nothing else.
    pub fn push(&mut self, op: Op) {
        if let (
            Some(Op::Matrix {
                matrix: first,
                offset: first_offset,
            }),
            Op::Matrix {
                matrix: second,
                offset: second_offset,
            },
        ) = (self.ops.last_mut(), &op)
        {
            let a = *first;
            let b = second;
            let mut fused = [0.0f32; 9];
            for row in 0..3 {
                for col in 0..3 {
                    fused[row * 3 + col] = (0..3).map(|k| b[row * 3 + k] * a[k * 3 + col]).sum();
                }
            }
            let o = *first_offset;
            *first_offset = vec3(
                b[0] * o.x + b[1] * o.y + b[2] * o.z,
                b[3] * o.x + b[4] * o.y + b[5] * o.z,
                b[6] * o.x + b[7] * o.y + b[8] * o.z,
            ) + *second_offset;
            *first = fused;
            return;
        }
        self.ops.push(op);
    }

    fn extend(&mut self, other: Processor) {
        for op in other.ops {
            self.push(op);
        }
        self.unsupported.extend(other.unsupported);
    }
}

/// Which of a config's two references a colour space or view transform is
/// defined against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reference {
    /// The scene reference: ACES2065-1 in an ACES config, and the only
    /// reference an OCIO v1 config has.
    #[default]
    Scene,
    /// The display connection space: CIE XYZ D65 in an ACES config.
    Display,
}

/// One colour space in a config.
#[derive(Clone, Debug, Default)]
pub struct ColorSpace {
    pub name: String,
    pub family: String,
    pub aliases: Vec<String>,
    /// `isdata: true`: values that are not colours — normals, masks, the
    /// "Raw" space — which OCIO never converts.
    pub is_data: bool,
    pub reference: Reference,
    /// Written in the config as `to_..._reference` or `from_..._reference`;
    /// both are kept, because building a conversion always needs one of each
    /// direction and deriving the missing one is not always possible.
    to_reference: Option<Yaml>,
    from_reference: Option<Yaml>,
}

/// A view transform: the step of an OCIO v2 view that crosses from the scene
/// reference to the display reference, where a config's tone mapping lives.
#[derive(Clone, Debug, Default)]
pub struct ViewTransform {
    pub name: String,
    /// What it maps *from*: the scene reference for an ordinary view
    /// transform, the display reference for one that adjusts an already
    /// display-referred picture.
    pub reference: Reference,
    to_reference: Option<Yaml>,
    from_reference: Option<Yaml>,
}

/// A look: a creative or corrective transform applied in its own process
/// space before the view transform.
#[derive(Clone, Debug, Default)]
pub struct Look {
    pub name: String,
    pub process_space: String,
    transform: Option<Yaml>,
    inverse_transform: Option<Yaml>,
}

/// A parsed config.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub path: PathBuf,
    pub search_paths: Vec<PathBuf>,
    pub roles: BTreeMap<String, String>,
    /// Display name to its views, in the order the config presents them:
    /// filtered and ordered by `active_displays` and `active_views` when it
    /// lists them, as OCIO does, because those lists are the config author
    /// saying what an application should offer.
    pub displays: Vec<(String, Vec<View>)>,
    /// Every display with every view, active or not, so that a document which
    /// chose an inactive one still resolves.
    all_displays: Vec<(String, Vec<View>)>,
    spaces: BTreeMap<String, ColorSpace>,
    /// Lower-cased names and aliases, to the canonical name. OCIO looks
    /// colour spaces up without regard to case.
    names: BTreeMap<String, String>,
    view_transforms: Vec<ViewTransform>,
    default_view_transform: Option<String>,
    looks: Vec<Look>,
}

#[derive(Clone, Debug)]
pub struct View {
    pub name: String,
    /// Where the view ends: for an OCIO v1 view the colour space it converts
    /// to, for a v2 view its display colour space.
    pub colorspace: String,
    /// The view transform of an OCIO v2 view.
    pub view_transform: Option<String>,
    /// The looks applied before the view transform, as the config writes
    /// them: `+look`, `-look`, separated by commas.
    pub looks: Option<String>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Config, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
        let mut config = Config::parse(&text)?;
        config.path = path.to_path_buf();
        Ok(config)
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let root = Yaml::parse(text)?;
        let mut config = Config::default();

        if let Some(search) = root.get("search_path").and_then(|v| v.as_str()) {
            // Colon or semicolon separated, the way OCIO writes it on each
            // platform.
            for part in search.split([':', ';']) {
                let part = part.trim();
                if !part.is_empty() {
                    config.search_paths.push(PathBuf::from(part));
                }
            }
        }
        if let Yaml::Sequence(items) = root.get("search_path").unwrap_or(&Yaml::Null) {
            for item in items {
                if let Some(s) = item.as_str() {
                    config.search_paths.push(PathBuf::from(s));
                }
            }
        }

        if let Some(Yaml::Mapping(roles)) = root.get("roles") {
            for (role, target) in roles {
                if let Some(target) = target.as_str() {
                    config.roles.insert(role.clone(), target.to_string());
                }
            }
        }

        // Shared views are written once and named from each display with
        // `!<Views> [...]`; `<USE_DISPLAY_NAME>` in one stands for the name
        // of the display it is used on.
        let shared: Vec<&Yaml> = root
            .get("shared_views")
            .map(|v| v.as_sequence().iter().collect())
            .unwrap_or_default();
        if let Some(Yaml::Mapping(displays)) = root.get("displays") {
            for (display, items) in displays {
                let mut list = Vec::new();
                for item in items.as_sequence() {
                    if item.tag() == Some("Views") || matches!(item, Yaml::Sequence(_)) {
                        for name in item.as_sequence().iter().filter_map(|n| n.as_str()) {
                            let found = shared
                                .iter()
                                .find(|v| v.get("name").and_then(|n| n.as_str()) == Some(name));
                            if let Some(view) = found.and_then(|v| view_from(v, display)) {
                                list.push(view);
                            }
                        }
                    } else if let Some(view) = view_from(item, display) {
                        list.push(view);
                    }
                }
                config.all_displays.push((display.clone(), list));
            }
        }

        let active_displays = name_list(root.get("active_displays"));
        let active_views = name_list(root.get("active_views"));
        let ordered: Vec<(String, Vec<View>)> = if active_displays.is_empty() {
            config.all_displays.clone()
        } else {
            active_displays
                .iter()
                .filter_map(|name| config.all_displays.iter().find(|(d, _)| d == name).cloned())
                .collect()
        };
        config.displays = ordered
            .into_iter()
            .map(|(display, views)| {
                if active_views.is_empty() {
                    (display, views)
                } else {
                    let views = active_views
                        .iter()
                        .filter_map(|name| views.iter().find(|v| &v.name == name).cloned())
                        .collect();
                    (display, views)
                }
            })
            .filter(|(_, views)| !views.is_empty())
            .collect();

        for (key, reference) in [("colorspaces", Reference::Scene), ("display_colorspaces", Reference::Display)] {
            for entry in root.get(key).map(|v| v.as_sequence()).unwrap_or(&[]) {
                let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
                    continue;
                };
                let (to, from) = match reference {
                    // `to_reference` and `from_reference` are OCIO v1's
                    // spelling, and still read by v2.
                    Reference::Scene => (
                        entry.get("to_scene_reference").or_else(|| entry.get("to_reference")),
                        entry.get("from_scene_reference").or_else(|| entry.get("from_reference")),
                    ),
                    Reference::Display => (
                        entry.get("to_display_reference"),
                        entry.get("from_display_reference"),
                    ),
                };
                let space = ColorSpace {
                    name: name.to_string(),
                    family: entry
                        .get("family")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    aliases: name_list(entry.get("aliases")),
                    is_data: entry.get("isdata").and_then(|v| v.as_str()) == Some("true"),
                    reference,
                    to_reference: to.cloned(),
                    from_reference: from.cloned(),
                };
                config.names.insert(name.to_lowercase(), name.to_string());
                for alias in &space.aliases {
                    config.names.insert(alias.to_lowercase(), name.to_string());
                }
                config.spaces.insert(name.to_string(), space);
            }
        }

        for entry in root
            .get("view_transforms")
            .map(|v| v.as_sequence())
            .unwrap_or(&[])
        {
            let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let scene = (entry.get("to_scene_reference"), entry.get("from_scene_reference"));
            let display = (entry.get("to_display_reference"), entry.get("from_display_reference"));
            let (reference, (to, from)) = if scene.0.is_some() || scene.1.is_some() {
                (Reference::Scene, scene)
            } else {
                (Reference::Display, display)
            };
            config.view_transforms.push(ViewTransform {
                name: name.to_string(),
                reference,
                to_reference: to.cloned(),
                from_reference: from.cloned(),
            });
        }
        config.default_view_transform = root
            .get("default_view_transform")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        for entry in root.get("looks").map(|v| v.as_sequence()).unwrap_or(&[]) {
            let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            config.looks.push(Look {
                name: name.to_string(),
                process_space: entry
                    .get("process_space")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                transform: entry.get("transform").cloned(),
                inverse_transform: entry.get("inverse_transform").cloned(),
            });
        }

        if config.spaces.is_empty() {
            return Err("the config names no colour spaces".into());
        }
        Ok(config)
    }

    pub fn space_names(&self) -> Vec<&str> {
        self.spaces.keys().map(|s| s.as_str()).collect()
    }

    /// The colour space a role points at, following one level of indirection.
    pub fn role(&self, name: &str) -> Option<&str> {
        self.roles.get(name).map(|s| s.as_str())
    }

    /// The space the renderer's own values are in, if the config says.
    ///
    /// `scene_linear` is the role every config defines for exactly this, and
    /// Tequila renders in ACEScg — so a config whose `scene_linear` is not an
    /// ACEScg-equivalent space will convert, which is correct and is the point
    /// of asking rather than assuming.
    pub fn scene_linear(&self) -> Option<&str> {
        self.role("scene_linear")
    }

    /// A colour space by name, alias or role, as OCIO resolves one: exact
    /// name first, then any name or alias without regard to case, then a role.
    pub fn space(&self, name: &str) -> Option<&ColorSpace> {
        let mut name = name;
        // Roles can point at roles; four hops is more than any config uses
        // and stops a cycle from hanging the loader.
        for _ in 0..4 {
            if let Some(space) = self.spaces.get(name) {
                return Some(space);
            }
            if let Some(canonical) = self.names.get(&name.to_lowercase()) {
                return self.spaces.get(canonical);
            }
            let role = self
                .roles
                .iter()
                .find(|(role, _)| role.eq_ignore_ascii_case(name))
                .map(|(_, target)| target.as_str())?;
            name = role;
        }
        None
    }

    /// Build the conversion from `from` to `to`.
    ///
    /// Both directions go through the config's reference space, which is what
    /// the config is structured around: a space defines its relationship to the
    /// reference and nothing else, so any conversion is "out of one, into the
    /// other". When one space is scene-referred and the other display-referred
    /// the two references are bridged by the default view transform, which is
    /// what OCIO does.
    pub fn processor(&self, from: &str, to: &str) -> Processor {
        let mut processor = Processor::default();
        if from == to {
            return processor;
        }
        let (Some(source), Some(target)) = (self.space(from), self.space(to)) else {
            for name in [from, to] {
                if self.space(name).is_none() {
                    processor.unsupported.push(format!("unknown colour space `{name}`"));
                }
            }
            return processor;
        };
        // OCIO never converts data, in either direction.
        if source.name == target.name || source.is_data || target.is_data {
            return processor;
        }
        self.space_to_reference(source, &mut processor);
        self.bridge(source.reference, target.reference, &mut processor);
        self.space_from_reference(target, &mut processor);
        processor
    }

    /// The conversion the renderer needs: scene-linear to a (display, view).
    pub fn view_pipeline(&self, display: &str, view: &str) -> Processor {
        let mut processor = Processor::default();
        let source = self.scene_linear().unwrap_or("ACEScg").to_string();
        let Some(view) = self
            .all_displays
            .iter()
            .find(|(name, _)| name == display)
            .and_then(|(_, views)| views.iter().find(|v| v.name == view))
        else {
            processor
                .unsupported
                .push(format!("no view `{view}` on display `{display}`"));
            return processor;
        };

        let (Some(source), Some(target)) = (self.space(&source), self.space(&view.colorspace)) else {
            for name in [source.as_str(), view.colorspace.as_str()] {
                if self.space(name).is_none() {
                    processor.unsupported.push(format!("unknown colour space `{name}`"));
                }
            }
            return processor;
        };
        // A "Raw" view: the values are shown as they are.
        if source.is_data || target.is_data {
            return processor;
        }

        self.space_to_reference(source, &mut processor);
        self.bridge(source.reference, Reference::Scene, &mut processor);
        if let Some(looks) = &view.looks {
            self.apply_looks(looks, &mut processor);
        }
        match &view.view_transform {
            Some(name) => {
                let Some(transform) = self.view_transforms.iter().find(|t| &t.name == name) else {
                    processor
                        .unsupported
                        .push(format!("unknown view transform `{name}`"));
                    return processor;
                };
                // A display-referred view transform adjusts a picture that is
                // already display-referred, so the scene is brought there by
                // the default view transform first.
                self.bridge(Reference::Scene, transform.reference, &mut processor);
                self.leave(&transform.from_reference, &transform.to_reference, &mut processor);
                self.bridge(Reference::Display, target.reference, &mut processor);
            }
            None => self.bridge(Reference::Scene, target.reference, &mut processor),
        }
        self.space_from_reference(target, &mut processor);
        processor
    }

    /// The (display, view) an application should open a config on: the first
    /// in the config's own order whose pipeline builds completely, else the
    /// very first.
    ///
    /// For the ACES configs that is their default, ACES 1.0 SDR Video on
    /// sRGB. It matters for a config whose first view uses something not
    /// implemented here — an ACES 2.0 output transform — where taking the
    /// first view blindly would open on a pipeline that falls straight back
    /// to the built-in transform. The search stops at the first success,
    /// because building a view of a LUT-based config reads its LUT files.
    pub fn first_buildable_view(&self) -> Option<(String, String)> {
        let every = self
            .displays
            .iter()
            .flat_map(|(display, views)| views.iter().map(move |v| (display.clone(), v.name.clone())));
        every
            .clone()
            .find(|(display, view)| self.view_pipeline(display, view).is_complete())
            .or_else(|| every.clone().next())
    }

    /// Into the space's reference from the space.
    fn space_to_reference(&self, space: &ColorSpace, out: &mut Processor) {
        // A space with neither transform *is* its reference, and needs no
        // conversion.
        self.enter(&space.from_reference, &space.to_reference, out);
    }

    /// Out of the space's reference into the space.
    fn space_from_reference(&self, space: &ColorSpace, out: &mut Processor) {
        self.leave(&space.from_reference, &space.to_reference, out);
    }

    /// The direction that leaves a reference: `from_reference` forward, or
    /// `to_reference` inverted.
    fn leave(&self, from: &Option<Yaml>, to: &Option<Yaml>, out: &mut Processor) {
        if let Some(transform) = from {
            self.build(transform, false, out);
        } else if let Some(transform) = to {
            self.build(transform, true, out);
        }
    }

    /// The direction that enters a reference.
    fn enter(&self, from: &Option<Yaml>, to: &Option<Yaml>, out: &mut Processor) {
        if let Some(transform) = to {
            self.build(transform, false, out);
        } else if let Some(transform) = from {
            self.build(transform, true, out);
        }
    }

    /// Cross between the scene and display references, if `from` and `to`
    /// differ, through the default view transform — or, failing one, the
    /// first scene-referred view transform, which is OCIO's fallback too.
    fn bridge(&self, from: Reference, to: Reference, out: &mut Processor) {
        if from == to {
            return;
        }
        let transform = self
            .default_view_transform
            .as_deref()
            .and_then(|name| self.view_transforms.iter().find(|t| t.name == name))
            .filter(|t| t.reference == Reference::Scene)
            .or_else(|| self.view_transforms.iter().find(|t| t.reference == Reference::Scene));
        let Some(transform) = transform else {
            out.unsupported.push(
                "no view transform connects the scene-referred and display-referred spaces".into(),
            );
            return;
        };
        match to {
            Reference::Display => self.leave(&transform.from_reference, &transform.to_reference, out),
            Reference::Scene => self.enter(&transform.from_reference, &transform.to_reference, out),
        }
    }

    /// A view's looks, each in its own process space, from and back to the
    /// scene reference.
    fn apply_looks(&self, looks: &str, out: &mut Processor) {
        for item in looks.split([',', ':']) {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let (name, inverse) = match item.strip_prefix('-') {
                Some(name) => (name.trim(), true),
                None => (item.trim_start_matches('+').trim(), false),
            };
            let Some(look) = self.looks.iter().find(|l| l.name == name) else {
                out.unsupported.push(format!("unknown look `{name}`"));
                continue;
            };
            let process = self.space(&look.process_space);
            if let Some(space) = process {
                self.space_from_reference(space, out);
            }
            let (transform, flipped) = match (inverse, &look.transform, &look.inverse_transform) {
                (false, Some(t), _) => (t, false),
                (false, None, Some(t)) => (t, true),
                (true, _, Some(t)) => (t, false),
                (true, Some(t), None) => (t, true),
                (_, None, None) => continue,
            };
            self.build(transform, flipped, out);
            if let Some(space) = process {
                self.space_to_reference(space, out);
            }
        }
    }

    /// Turn one transform into ops, appending them to `out`.
    ///
    /// `invert` flips both the order of a group's children and the direction of
    /// each op, which is what makes a space defined in only one direction
    /// usable in both.
    fn build(&self, node: &Yaml, invert: bool, out: &mut Processor) {
        let tag = node.tag().unwrap_or("");
        // A transform carries its own direction as well, and the two compose:
        // an inverse transform read backwards is a forward one.
        let declared_inverse = node.get("direction").and_then(|v| v.as_str()) == Some("inverse");
        let invert = invert != declared_inverse;

        let numbers = |key: &str, fallback: Vec<f32>| -> Vec<f32> {
            match node.get(key) {
                Some(v) => {
                    let n = v.as_numbers();
                    if n.is_empty() {
                        fallback
                    } else {
                        n
                    }
                }
                None => fallback,
            }
        };
        let triple = |key: &str, fallback: f32| -> [f32; 3] {
            let n = numbers(key, vec![fallback; 3]);
            [
                *n.first().unwrap_or(&fallback),
                *n.get(1).unwrap_or(n.first().unwrap_or(&fallback)),
                *n.get(2).unwrap_or(n.first().unwrap_or(&fallback)),
            ]
        };

        let op = match tag {
            "GroupTransform" => {
                let children: Vec<&Yaml> = node
                    .get("children")
                    .map(|c| c.as_sequence().iter().collect())
                    .unwrap_or_default();
                if invert {
                    for child in children.into_iter().rev() {
                        self.build(child, true, out);
                    }
                } else {
                    for child in children {
                        self.build(child, false, out);
                    }
                }
                return;
            }
            "ColorSpaceTransform" => {
                let src = node.get("src").and_then(|v| v.as_str()).unwrap_or_default();
                let dst = node.get("dst").and_then(|v| v.as_str()).unwrap_or_default();
                let (src, dst) = if invert { (dst, src) } else { (src, dst) };
                out.extend(self.processor(src, dst));
                return;
            }
            "BuiltinTransform" => {
                let style = node
                    .get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed");
                match builtin::ops(style) {
                    Ok(ops) if invert => {
                        for op in ops.into_iter().rev() {
                            match op.inverse() {
                                Some(inverse) => out.push(inverse),
                                None => {
                                    out.unsupported
                                        .push(format!("BuiltinTransform `{style}` cannot be inverted"));
                                    return;
                                }
                            }
                        }
                    }
                    Ok(ops) => {
                        for op in ops {
                            out.push(op);
                        }
                    }
                    Err(error) => out.unsupported.push(error),
                }
                return;
            }
            "FileTransform" => {
                let Some(src) = node.get("src").and_then(|v| v.as_str()) else {
                    out.unsupported.push("a FileTransform with no src".into());
                    return;
                };
                match self.resolve(src) {
                    Some(path) => match Lut::load(&path) {
                        Ok(loaded) => Op::Lut(Arc::new(loaded)),
                        Err(error) => {
                            out.unsupported.push(error);
                            return;
                        }
                    },
                    None => {
                        out.unsupported
                            .push(format!("`{src}` was not found on the search path"));
                        return;
                    }
                }
            }
            "MatrixTransform" => {
                let m = numbers("matrix", vec![]);
                if m.len() < 16 {
                    out.unsupported
                        .push("a MatrixTransform without a 4x4 matrix".into());
                    return;
                }
                // OCIO writes a 4x4 and a separate offset. The fourth column
                // multiplies alpha, which is 1 for every pixel developed here,
                // so it acts as a further offset; the fourth row is alpha's
                // own and unused for colour.
                let offset = triple("offset", 0.0);
                Op::Matrix {
                    matrix: [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]],
                    offset: vec3(m[3] + offset[0], m[7] + offset[1], m[11] + offset[2]),
                }
            }
            "ExponentTransform" => Op::Exponent {
                value: triple("value", 1.0),
            },
            "ExponentWithLinearTransform" => Op::MonCurve {
                gamma: triple("gamma", 1.0),
                offset: triple("offset", 0.0),
                inverse: false,
            },
            "RangeTransform" => {
                let read = |key: &str, fallback: f32| {
                    node.get(key).and_then(|v| v.as_f32()).unwrap_or(fallback)
                };
                Op::Range {
                    min_in: read("min_in_value", 0.0),
                    max_in: read("max_in_value", 1.0),
                    min_out: read("min_out_value", 0.0),
                    max_out: read("max_out_value", 1.0),
                    clamp: node.get("style").and_then(|v| v.as_str()) != Some("noClamp"),
                }
            }
            "LogTransform" => Op::Log {
                base: node.get("base").and_then(|v| v.as_f32()).unwrap_or(2.0),
                inverse: false,
            },
            "LogAffineTransform" | "LogCameraTransform" => {
                let camera = tag == "LogCameraTransform";
                if camera && node.get("lin_side_break").is_none() {
                    out.unsupported
                        .push("a LogCameraTransform without lin_side_break".into());
                    return;
                }
                Op::LogCamera(Box::new(LogCamera {
                    base: node.get("base").and_then(|v| v.as_f32()).unwrap_or(2.0),
                    log_slope: triple("log_side_slope", 1.0),
                    log_offset: triple("log_side_offset", 0.0),
                    lin_slope: triple("lin_side_slope", 1.0),
                    lin_offset: triple("lin_side_offset", 0.0),
                    lin_break: camera.then(|| triple("lin_side_break", 0.0)),
                    linear_slope: (camera && node.get("linear_slope").is_some())
                        .then(|| triple("linear_slope", 1.0)),
                    inverse: false,
                }))
            }
            "CDLTransform" => Op::Cdl {
                slope: triple("slope", 1.0),
                offset: triple("offset", 0.0),
                power: triple("power", 1.0),
                saturation: node.get("sat").and_then(|v| v.as_f32()).unwrap_or(1.0),
            },
            "" => return,
            other => {
                out.unsupported.push(other.to_string());
                return;
            }
        };

        if invert {
            match op.inverse() {
                Some(inverse) => out.push(inverse),
                None => out
                    .unsupported
                    .push(format!("`{tag}` cannot be inverted in closed form")),
            }
        } else {
            out.push(op);
        }
    }

    /// Find a LUT on the config's search path.
    fn resolve(&self, src: &str) -> Option<PathBuf> {
        let base = self.path.parent().unwrap_or(Path::new("."));
        let direct = base.join(src);
        if direct.is_file() {
            return Some(direct);
        }
        for search in &self.search_paths {
            let candidate = base.join(search).join(src);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        let absolute = PathBuf::from(src);
        absolute.is_file().then_some(absolute)
    }
}

/// A view as a display lists it, with `<USE_DISPLAY_NAME>` resolved.
fn view_from(node: &Yaml, display: &str) -> Option<View> {
    let name = node.get("name").and_then(|v| v.as_str())?;
    let colorspace = node
        .get("display_colorspace")
        .or_else(|| node.get("colorspace"))
        .and_then(|v| v.as_str())?;
    let colorspace = if colorspace == "<USE_DISPLAY_NAME>" {
        display
    } else {
        colorspace
    };
    Some(View {
        name: name.to_string(),
        colorspace: colorspace.to_string(),
        view_transform: node
            .get("view_transform")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        looks: node
            .get("looks")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string),
    })
}

/// A list of names, written `[a, b]` or as one comma-separated string.
fn name_list(node: Option<&Yaml>) -> Vec<String> {
    match node {
        Some(Yaml::Sequence(items)) => items
            .iter()
            .filter_map(|i| i.as_str())
            .map(str::to_string)
            .collect(),
        Some(other) => other
            .as_str()
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DisplayTransform;
    use tequila_core::spectrum::Spectrum;

    /// A config small enough to read, exercising the shapes a real one uses:
    /// roles, displays with views, a group transform, a matrix, and a curve.
    const CONFIG: &str = r#"
ocio_profile_version: 2
search_path: luts
roles:
  scene_linear: ACEScg
  color_picking: sRGB - Texture

displays:
  sRGB:
    - !<View> {name: Un-tone-mapped, colorspace: sRGB - Display}
    - !<View> {name: Raw, colorspace: ACEScg}

colorspaces:
  - !<ColorSpace>
    name: ACEScg
    family: ACES

  - !<ColorSpace>
    name: sRGB - Display
    family: Display
    from_reference: !<GroupTransform>
      children:
        - !<MatrixTransform> {matrix: [1.7051, -0.6217, -0.0833, 0, -0.1302, 1.1408, -0.0106, 0, -0.0240, -0.1289, 1.1529, 0, 0, 0, 0, 1]}
        - !<ExponentWithLinearTransform> {gamma: 2.4, offset: 0.055, direction: inverse}

  - !<ColorSpace>
    name: sRGB - Texture
    family: Texture
    to_reference: !<ColorSpaceTransform> {src: sRGB - Display, dst: ACEScg}
"#;

    #[test]
    fn a_config_reads_its_roles_displays_and_spaces() {
        let config = Config::parse(CONFIG).unwrap();
        assert_eq!(config.scene_linear(), Some("ACEScg"));
        assert_eq!(config.displays.len(), 1);
        let (display, views) = &config.displays[0];
        assert_eq!(display, "sRGB");
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].name, "Un-tone-mapped");
        assert_eq!(config.space_names().len(), 3);
    }

    /// The check `docs/05` §1 asks for in so many words: the analytic transform
    /// written at M0 is the reference the OCIO path is measured against. A
    /// config encoding plain sRGB has to reproduce `DisplayTransform::Srgb`.
    ///
    /// Not to the last bit — the config's matrix is the rounded one a real
    /// config carries, and the curve is the standard's rather than the fit —
    /// but to well inside a code value, which is the tolerance that matters for
    /// a picture.
    #[test]
    fn a_config_that_encodes_srgb_matches_the_analytic_transform() {
        let config = Config::parse(CONFIG).unwrap();
        let processor = config.view_pipeline("sRGB", "Un-tone-mapped");
        assert!(
            processor.is_complete(),
            "the pipeline could not be built: {:?}",
            processor.unsupported
        );

        for probe in [0.0, 0.02, 0.18, 0.5, 0.9, 1.0] {
            let scene = Spectrum::from_linear_srgb(Vec3::splat(probe));
            let through_ocio = processor.apply(scene.to_rendering_rgb());
            let analytic = DisplayTransform::Srgb.apply(scene);
            for (a, b) in [
                (through_ocio.x, analytic.x),
                (through_ocio.y, analytic.y),
                (through_ocio.z, analytic.z),
            ] {
                assert!(
                    (a - b).abs() < 0.01,
                    "at {probe}: OCIO says {a}, the analytic transform says {b}"
                );
            }
        }
    }

    /// A round trip through the reference space has to come back where it
    /// started. This is what catches a direction read backwards, which is the
    /// single easiest mistake to make here and produces an image that is
    /// plausible and wrong.
    #[test]
    fn a_space_converts_to_itself_and_back() {
        let config = Config::parse(CONFIG).unwrap();
        let out = config.processor("ACEScg", "sRGB - Display");
        let back = config.processor("sRGB - Display", "ACEScg");
        assert!(out.is_complete() && back.is_complete());

        for probe in [0.05f32, 0.18, 0.6] {
            let start = vec3(probe, probe * 0.8, probe * 1.2);
            let round = back.apply(out.apply(start));
            assert!(
                (round.x - start.x).abs() < 1e-3
                    && (round.y - start.y).abs() < 1e-3
                    && (round.z - start.z).abs() < 1e-3,
                "{start:?} came back as {round:?}"
            );
        }
    }

    /// A `ColorSpaceTransform` resolves through the config, so a space defined
    /// in terms of another is usable without the user knowing it was.
    #[test]
    fn a_space_defined_by_another_resolves() {
        let config = Config::parse(CONFIG).unwrap();
        let direct = config.processor("sRGB - Display", "ACEScg");
        let indirect = config.processor("sRGB - Texture", "ACEScg");
        assert!(indirect.is_complete(), "{:?}", indirect.unsupported);
        let probe = vec3(0.3, 0.5, 0.7);
        let a = direct.apply(probe);
        let b = indirect.apply(probe);
        assert!(
            (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4,
            "{a:?} against {b:?}"
        );
    }

    /// What is still missing must come back *reported*, by name, so the
    /// application can say so and fall back — a silently empty pipeline would
    /// render the scene untransformed and look like a bug in the renderer.
    /// An ACES 2.0 output transform is the case that matters, and a style
    /// that is not in the registry at all the other.
    #[test]
    fn a_builtin_transform_that_is_not_implemented_is_reported_by_name() {
        for style in [
            "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0",
            "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65",
        ] {
            let text = format!(
                r#"
ocio_profile_version: 2
displays:
  sRGB:
    - !<View> {{name: ACES, colorspace: out}}
colorspaces:
  - !<ColorSpace>
    name: ACEScg
  - !<ColorSpace>
    name: out
    from_reference: !<BuiltinTransform> {{style: {style}}}
"#
            );
            let config = Config::parse(&text).unwrap();
            let processor = config.view_pipeline("sRGB", "ACES");
            assert!(!processor.is_complete());
            assert!(
                processor.unsupported[0].contains("BuiltinTransform")
                    && processor.unsupported[0].contains(style),
                "{:?}",
                processor.unsupported
            );
        }
    }

    /// A config whose first view is an ACES 2.0 one opens on the first view
    /// that builds instead, which is what the application selects on load.
    #[test]
    fn the_first_buildable_view_skips_one_that_will_not_build() {
        let text = r#"
ocio_profile_version: 2
displays:
  sRGB - Display:
    - !<View> {name: ACES 2.0 - SDR 100 nits (Rec.709), view_transform: ACES 2.0, display_colorspace: <USE_DISPLAY_NAME>}
    - !<View> {name: ACES 1.0 - SDR Video, view_transform: ACES 1.0, display_colorspace: <USE_DISPLAY_NAME>}
view_transforms:
  - !<ViewTransform>
    name: ACES 2.0
    from_scene_reference: !<BuiltinTransform> {style: ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0}
  - !<ViewTransform>
    name: ACES 1.0
    from_scene_reference: !<BuiltinTransform> {style: ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0}
display_colorspaces:
  - !<ColorSpace>
    name: sRGB - Display
    from_display_reference: !<BuiltinTransform> {style: DISPLAY - CIE-XYZ-D65_to_sRGB}
colorspaces:
  - !<ColorSpace>
    name: ACES2065-1
  - !<ColorSpace>
    name: ACEScg
    to_scene_reference: !<BuiltinTransform> {style: ACEScg_to_ACES2065-1}
roles:
  scene_linear: ACEScg
"#;
        let config = Config::parse(text).unwrap();
        let first = config.view_pipeline("sRGB - Display", "ACES 2.0 - SDR 100 nits (Rec.709)");
        assert!(first.unsupported[0].contains("ACES 2.0"), "{:?}", first.unsupported);
        assert_eq!(
            config.first_buildable_view(),
            Some(("sRGB - Display".to_string(), "ACES 1.0 - SDR Video".to_string()))
        );
        // And with nothing buildable it still offers the first, so the
        // application can say what is missing.
        let none = text.replace("SDR-VIDEO_1.0", "SDR-NOTHING");
        assert_eq!(
            Config::parse(&none).unwrap().first_buildable_view().map(|(_, v)| v),
            Some("ACES 2.0 - SDR 100 nits (Rec.709)".to_string())
        );
    }

    /// A view that does not exist is reported too, rather than returning an
    /// empty processor that would silently pass the image through.
    #[test]
    fn a_missing_view_is_reported() {
        let config = Config::parse(CONFIG).unwrap();
        let processor = config.view_pipeline("sRGB", "Nonexistent");
        assert!(!processor.is_complete());
    }

    /// The camera form of OCIO's log transform, checked against a curve it
    /// can express exactly: ARRI LogC3 at EI 800, written the way the ACES
    /// configs write camera curves that have no builtin. The linear
    /// segment's slope is left for the transform to derive, and has to come
    /// out as ARRI's published 5.367655.
    #[test]
    fn a_log_camera_transform_reproduces_logc3() {
        let text = r#"
ocio_profile_version: 2
colorspaces:
  - !<ColorSpace>
    name: linear
  - !<ColorSpace>
    name: logc
    to_reference: !<LogCameraTransform> {base: 10, log_side_slope: 0.24719, log_side_offset: 0.385537, lin_side_slope: 5.555556, lin_side_offset: 0.052272, lin_side_break: 0.010591, direction: inverse}
"#;
        let config = Config::parse(text).unwrap();
        let decode = config.processor("logc", "linear");
        let encode = config.processor("linear", "logc");
        assert!(decode.is_complete() && encode.is_complete());
        for code in [0.05f32, 0.1, 0.149, 0.2, 0.391, 0.6, 0.9] {
            let ours = decode.apply(Vec3::splat(code)).x;
            let arri = Curve::LogC3Ei800.to_linear(code);
            assert!((ours - arri).abs() < 2e-5, "at {code}: {ours} against {arri}");
            let back = encode.apply(Vec3::splat(ours)).x;
            assert!((back - code).abs() < 1e-5, "{code} came back as {back}");
        }
    }

    /// Fusing neighbouring matrices must not change the result.
    #[test]
    fn fused_matrices_apply_as_the_chain_did() {
        let a = Op::Matrix {
            matrix: [1.2, 0.1, -0.05, 0.02, 0.9, 0.1, -0.1, 0.2, 1.1],
            offset: vec3(0.01, -0.02, 0.03),
        };
        let b = Op::Matrix {
            matrix: [0.8, -0.2, 0.1, 0.3, 1.1, -0.1, 0.0, 0.05, 0.95],
            offset: vec3(-0.1, 0.0, 0.2),
        };
        let mut fused = Processor::default();
        fused.push(a.clone());
        fused.push(b.clone());
        assert_eq!(fused.len(), 1);
        let probe = vec3(0.3, 0.6, 0.9);
        let chained = b.apply(a.apply(probe));
        let once = fused.apply(probe);
        assert!((chained - once).abs().max_component() < 1e-6, "{chained:?} against {once:?}");
    }
}
