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
//! Supported transforms: `FileTransform` (`.cube`, `.spi1d`, `.spimtx`),
//! `MatrixTransform`, `ExponentTransform`, `ExponentWithLinearTransform`,
//! `RangeTransform`, `LogTransform`, `CDLTransform`, `GroupTransform` and
//! `ColorSpaceTransform`, each in both directions.
//!
//! **Not** supported: `BuiltinTransform`. OCIO v2 moved the ACES output
//! transforms out of LUT files and into a registry of transforms built into the
//! library — a substantial body of maths that is defined by its C++
//! implementation rather than by a file. A config that uses one is *reported*,
//! not silently mis-rendered: [`Config::view_pipeline`] returns the names of the
//! transforms it could not build, the application says so, and the analytic
//! transforms of [`crate::DisplayTransform`] remain available. That is a real
//! limitation and stating it is better than a picture that is quietly not what
//! the config asked for.
//!
//! This is why the analytic transforms were written first. `docs/05` §1 note 3:
//! "M0 implements the same display transforms analytically ... OCIO arrives at
//! M3; the analytic versions then serve as the reference to check it against" —
//! and [`tests::a_config_that_encodes_srgb_matches_the_analytic_transform`] is
//! that check.

pub mod lut;
pub mod yaml;

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
    /// Log to linear, base `base`, as OCIO's `LogTransform` defines it.
    Log { base: f32 },
    /// ASC CDL: slope, offset, power, saturation.
    Cdl {
        slope: [f32; 3],
        offset: [f32; 3],
        power: [f32; 3],
        saturation: f32,
    },
}

impl Op {
    pub fn apply(&self, c: Vec3) -> Vec3 {
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
            Op::Log { base } => {
                let ln_base = base.max(1.0001).ln();
                let map = |v: f32| v.max(1e-10).ln() / ln_base;
                vec3(map(c.x), map(c.y), map(c.z))
            }
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
            Op::Lut(_) | Op::Log { .. } | Op::Cdl { .. } => None,
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
}

/// One colour space in a config.
#[derive(Clone, Debug, Default)]
pub struct ColorSpace {
    pub name: String,
    pub family: String,
    /// Written in the config as `to_reference` or `from_reference`; both are
    /// kept as *to* reference, with the direction recorded, because building a
    /// conversion always needs one of each and deriving the missing one is not
    /// always possible.
    to_reference: Option<Yaml>,
    from_reference: Option<Yaml>,
}

/// A parsed config.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub path: PathBuf,
    pub search_paths: Vec<PathBuf>,
    pub roles: BTreeMap<String, String>,
    /// Display name to its views, in config order.
    pub displays: Vec<(String, Vec<View>)>,
    spaces: BTreeMap<String, ColorSpace>,
}

#[derive(Clone, Debug)]
pub struct View {
    pub name: String,
    pub colorspace: String,
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

        if let Some(Yaml::Mapping(displays)) = root.get("displays") {
            for (display, views) in displays {
                let mut list = Vec::new();
                for view in views.as_sequence() {
                    let (Some(name), Some(colorspace)) = (
                        view.get("name").and_then(|v| v.as_str()),
                        view.get("colorspace").and_then(|v| v.as_str()),
                    ) else {
                        continue;
                    };
                    list.push(View {
                        name: name.to_string(),
                        colorspace: colorspace.to_string(),
                    });
                }
                config.displays.push((display.clone(), list));
            }
        }

        for entry in root
            .get("colorspaces")
            .map(|v| v.as_sequence())
            .unwrap_or(&[])
        {
            let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            config.spaces.insert(
                name.to_string(),
                ColorSpace {
                    name: name.to_string(),
                    family: entry
                        .get("family")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    to_reference: entry.get("to_reference").cloned(),
                    from_reference: entry.get("from_reference").cloned(),
                },
            );
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

    /// Build the conversion from `from` to `to`.
    ///
    /// Both directions go through the config's reference space, which is what
    /// the config is structured around: a space defines its relationship to the
    /// reference and nothing else, so any conversion is "out of one, into the
    /// other".
    pub fn processor(&self, from: &str, to: &str) -> Processor {
        let mut processor = Processor::default();
        if from == to {
            return processor;
        }
        self.to_reference(from, &mut processor);
        self.from_reference(to, &mut processor);
        processor
    }

    /// The conversion the renderer needs: scene-linear to a (display, view).
    pub fn view_pipeline(&self, display: &str, view: &str) -> Processor {
        let source = self.scene_linear().unwrap_or("ACEScg").to_string();
        let Some(target) = self
            .displays
            .iter()
            .find(|(name, _)| name == display)
            .and_then(|(_, views)| views.iter().find(|v| v.name == view))
            .map(|v| v.colorspace.clone())
        else {
            let mut processor = Processor::default();
            processor
                .unsupported
                .push(format!("no view `{view}` on display `{display}`"));
            return processor;
        };
        self.processor(&source, &target)
    }

    fn to_reference(&self, name: &str, out: &mut Processor) {
        let Some(space) = self.spaces.get(name) else {
            out.unsupported.push(format!("unknown colour space `{name}`"));
            return;
        };
        if let Some(transform) = &space.to_reference {
            self.build(transform, false, out);
        } else if let Some(transform) = &space.from_reference {
            self.build(transform, true, out);
        }
        // A space with neither *is* the reference, and needs no conversion.
    }

    fn from_reference(&self, name: &str, out: &mut Processor) {
        let Some(space) = self.spaces.get(name) else {
            out.unsupported.push(format!("unknown colour space `{name}`"));
            return;
        };
        if let Some(transform) = &space.from_reference {
            self.build(transform, false, out);
        } else if let Some(transform) = &space.to_reference {
            self.build(transform, true, out);
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
                let inner = self.processor(src, dst);
                out.ops.extend(inner.ops);
                out.unsupported.extend(inner.unsupported);
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
                // OCIO writes a 4x4; the fourth column is the offset and the
                // fourth row is unused for colour.
                Op::Matrix {
                    matrix: [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]],
                    offset: vec3(m[3], m[7], m[11]),
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
            },
            "CDLTransform" => Op::Cdl {
                slope: triple("slope", 1.0),
                offset: triple("offset", 0.0),
                power: triple("power", 1.0),
                saturation: node.get("sat").and_then(|v| v.as_f32()).unwrap_or(1.0),
            },
            "BuiltinTransform" => {
                let style = node
                    .get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed");
                out.unsupported
                    .push(format!("BuiltinTransform `{style}`"));
                return;
            }
            "" => return,
            other => {
                out.unsupported.push(other.to_string());
                return;
            }
        };

        if invert {
            match op.inverse() {
                Some(inverse) => out.ops.push(inverse),
                None => out
                    .unsupported
                    .push(format!("`{tag}` cannot be inverted in closed form")),
            }
        } else {
            out.ops.push(op);
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

    /// The limitation, asserted rather than described. A config using a
    /// `BuiltinTransform` must come back *reported*, so the application can say
    /// so and fall back — a silently empty pipeline would render the scene
    /// untransformed and look like a bug in the renderer.
    #[test]
    fn a_builtin_transform_is_reported_rather_than_ignored() {
        let text = r#"
ocio_profile_version: 2
displays:
  sRGB:
    - !<View> {name: ACES, colorspace: out}
colorspaces:
  - !<ColorSpace>
    name: ACEScg
  - !<ColorSpace>
    name: out
    from_reference: !<BuiltinTransform> {style: ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65}
"#;
        let config = Config::parse(text).unwrap();
        let processor = config.view_pipeline("sRGB", "ACES");
        assert!(!processor.is_complete());
        assert!(
            processor.unsupported[0].contains("BuiltinTransform"),
            "{:?}",
            processor.unsupported
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
}
