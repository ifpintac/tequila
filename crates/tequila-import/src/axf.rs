//! X-Rite AxF measured materials, as Tequila materials.
//!
//! An AxF file (HDF5, read by [`crate::hdf5`]) holds a material scanned off
//! a real sample, in one or more representations. Tequila reads the
//! spatially varying BRDF (`SVBRDF`, or its energy-preserving variant): a
//! diffuse colour, a specular colour, a GGX lobe's width, a Fresnel
//! reflectance and a normal map, each a picture of the sample or a single
//! value, over a tile of the sample's size. A car paint's own
//! representation (`CarPaint2`) is read for its measured flakes alone
//! ([`Flakes`]), fitted to Tequila's flakes; its tabulated colour is not,
//! and its SVBRDF, which every car paint file carries beside it, is the
//! rest of the material.

use crate::hdf5::{Dataset, File};
use std::path::{Path, PathBuf};

/// A map: one value, or a picture of the sample.
#[derive(Clone, Debug, PartialEq)]
pub enum Map {
    Constant(Vec<f32>),
    Image { width: usize, height: usize, channels: usize, data: Vec<f32> },
}

impl Map {
    /// Its mean, channel by channel.
    pub fn mean(&self) -> Vec<f32> {
        match self {
            Map::Constant(v) => v.clone(),
            Map::Image { channels, data, .. } => {
                let n = (data.len() / channels).max(1) as f64;
                (0..*channels).map(|c| (data.iter().skip(c).step_by(*channels).map(|v| *v as f64).sum::<f64>() / n) as f32).collect()
            }
        }
    }
}

/// What Tequila takes from an AxF file.
#[derive(Clone, Debug)]
pub struct Svbrdf {
    pub name: String,
    pub diffuse: Map,
    /// The specular colour, times its amount where there is one.
    pub specular: Map,
    /// GGX alpha.
    pub lobes: Map,
    /// Fresnel reflectance at normal incidence.
    pub fresnel: f32,
    pub normal: Option<Map>,
    /// The tile's size, in metres.
    pub size: (f32, f32),
    /// The file is a car paint: it has flakes the SVBRDF does not carry.
    pub car_paint: bool,
    /// Its measured flakes, where it has them and they read.
    pub flakes: Option<Flakes>,
}

/// A car paint's flakes as Tequila draws them, fitted to the flake
/// reflectance X-Rite measured (`CarPaint2`'s flake BTF).
///
/// The BTF is pictures of the sparkle, one for each pair of angles: the
/// half vector's from the normal, in steps of 90 / NumThetaF degrees, by
/// the light's from the half vector. Its mean over the picture is the
/// flakes' BRDF, added to the paint's (Unity's AxF renderer, `AxF.hlsl`).
/// Tequila's flakes cover a share `w` of the surface, with normals spread
/// evenly over a cone of half-angle `a` and the flakes' own reflectance
/// `F`. Seen with the light behind the eye (the first column of light
/// angles), they make a BRDF of `w F D(h) / (4 cos^3 h)`. So
/// `4 BRDF cos^3 h` integrated over the hemisphere is `w F`, and its mean
/// tilt is `2a/3`, the mean of a cone's. The measured flakes fall off with
/// a tail where Tequila's stop at the cone's edge: matched by their energy
/// and mean tilt, they glint as far out as the real ones on the whole.
#[derive(Clone, Debug, PartialEq)]
pub struct Flakes {
    /// The flakes' colour, linear sRGB, its strongest channel 1.
    pub colour: [f32; 3],
    /// Their reflectance times their coverage, `w F`.
    pub coverage_reflectance: f32,
    /// The half-angle of the cone their normals spread over, in degrees.
    pub cone_degrees: f32,
    /// A flake's diameter, in metres: one of the BTF's texels in area. Each
    /// sparkle in it is a texel, and as many sparkle as flakes of this size
    /// at their coverage would (two a square millimetre in TIGER's powder
    /// coats); flakes of Tequila's default size, a few hundredths of that,
    /// averaged into a sheen where the paint glitters.
    pub size: f32,
}

impl Flakes {
    /// Fit from the BRDF's mean at half-vector angles `0, step, 2 step, ...`
    /// degrees, the light behind the eye, and its colour at the first.
    pub fn fit(brdf: &[f32], step_degrees: f32, colour: [f32; 3], texel: f32) -> Option<Flakes> {
        let first = *brdf.first()?;
        if !(first > 0.0) || brdf.len() < 2 {
            return None;
        }
        let step = step_degrees.to_radians();
        let (mut energy, mut tilt) = (0.0f64, 0.0f64);
        for (j, value) in brdf.iter().enumerate() {
            // The bin around j steps: half a step either side.
            let centre = j as f32 * step;
            let (low, high) = ((centre - step / 2.0).max(0.0), centre + step / 2.0);
            let solid = std::f32::consts::TAU * (low.cos() - high.cos());
            let weight = (4.0 * value.max(0.0) * centre.cos().powi(3) * solid) as f64;
            energy += weight;
            tilt += weight * centre as f64;
        }
        if energy <= 0.0 {
            return None;
        }
        let mean = (tilt / energy) as f32;
        let strongest = colour.iter().copied().fold(0.0f32, f32::max);
        if !(strongest > 0.0) {
            return None;
        }
        Some(Flakes {
            colour: colour.map(|c| (c / strongest).max(0.0)),
            coverage_reflectance: energy as f32,
            cone_degrees: (1.5 * mean).to_degrees().clamp(1.0, 60.0),
            size: (2.0 * texel / std::f32::consts::PI.sqrt()).clamp(0.000_005, 0.000_5),
        })
    }
}

/// The flakes of the car paint representation `rep` of `material`, read
/// from its flake BTF.
fn flakes(file: &File, material: &str, rep: &str) -> Option<Flakes> {
    // The operand naming the BTF: its lookup table of patches by light and
    // half-vector angle, and how many of those angles.
    let mut operands = Vec::new();
    collect_operands(file, &format!("{rep}/com.xrite.Input"), &mut operands, 0);
    let operand = operands.iter().find(|o| file.has(&format!("{o}/PatchesLookupTable")))?;
    let table = file.dataset(&format!("{operand}/PatchesLookupTable")).ok()?;
    let columns = *table.shape.get(1)?;
    let lookup = table.numbers()?;
    let thetas = file.dataset(&format!("{operand}/NumThetaF")).ok()?.numbers()?.first().copied()? as f32;
    let locations = file.dataset(&format!("{operand}/com.xrite.Resources/Data/ResourceLocations")).ok()?;
    let crate::hdf5::Values::Strings(locations) = &locations.values else { return None };
    let resources = format!("{material}/com.xrite.Resources");
    // A texel's width, from the patches' size and the first's pixels.
    let first = locations.first()?.rsplit_once('/').map_or(String::new(), |(group, _)| group.to_string());
    let width_mm = file.dataset(&format!("{resources}/{first}/WidthMM")).ok()?.numbers()?.first().copied()? as f32;
    let mut texel = 0.0;
    // The first row: the light behind the eye, the half vector tilting.
    let mut brdf = Vec::new();
    let mut colour = [0.0f32; 3];
    for column in 0..columns {
        let Some(&patch) = lookup.get(column) else { break };
        if patch < 0.0 {
            break;
        }
        let location = locations.get(patch as usize)?;
        let data = file.dataset(&format!("{resources}/{location}/Data")).ok()?;
        let values = data.numbers()?;
        let value = |v: f64| if data.integer_bytes == Some(2) { crate::hdf5::half(v as u16) } else { v as f32 };
        let channels = data.shape.last().copied().unwrap_or(1).max(1);
        let count = (values.len() / channels).max(1) as f64;
        let mean: Vec<f32> =
            (0..channels).map(|c| (values.iter().skip(c).step_by(channels).map(|v| value(*v) as f64).sum::<f64>() / count) as f32).collect();
        if column == 0 {
            texel = width_mm * 0.001 / data.shape.get(1).copied().unwrap_or(1).max(1) as f32;
            for (k, slot) in colour.iter_mut().enumerate() {
                *slot = mean.get(k).or(mean.first()).copied().unwrap_or(0.0);
            }
        }
        brdf.push(mean.iter().sum::<f32>() / mean.len() as f32);
    }
    Flakes::fit(&brdf, 90.0 / thetas.max(1.0), colour, texel)
}

/// The first material's SVBRDF in an AxF file's bytes.
pub fn read(bytes: &[u8]) -> Result<Svbrdf, String> {
    let file = File::open(bytes)?;
    let materials = file.find("com.xrite.Materials")?;
    let (name, _) = file.members(materials)?.into_iter().next().ok_or("the AxF file holds no material")?;
    let material = format!("com.xrite.Materials/{name}");
    let representations = format!("{material}/com.xrite.Representations");
    let mut reps: Vec<String> = file.members(file.find(&representations)?)?.into_iter().map(|(n, _)| n).collect();
    reps.sort();
    let class_of = |rep: &str| file.dataset(&format!("{representations}/{rep}/RepresentationClass")).ok().and_then(|d| d.text().map(str::to_string)).unwrap_or_default();
    let car_paint = reps.iter().any(|r| class_of(r).contains("CarPaint"));
    let flakes = reps
        .iter()
        .find(|r| class_of(r).contains("CarPaint"))
        .and_then(|r| flakes(&file, &material, &format!("{representations}/{r}")));
    let preferred = file
        .dataset(&format!("{material}/IndexOfPreferredRepresentation"))
        .ok()
        .and_then(|d| d.numbers().and_then(|v| v.first().copied()))
        .map(|v| v as usize)
        .unwrap_or(0)
        .to_string();
    // The preferred representation if it is an SVBRDF, else the first that is.
    let rep = std::iter::once(preferred)
        .chain(reps.iter().cloned())
        .find(|r| class_of(r).contains("SVBRDF"))
        .ok_or("the AxF file has no SVBRDF; its other representations are not read")?;
    let rep_path = format!("{representations}/{rep}");
    let resources = format!("{material}/com.xrite.Resources");

    // The operands, wherever they are nested: the one with lobes is the
    // specular, the other with a colour the diffuse.
    let mut operands = Vec::new();
    collect_operands(&file, &format!("{rep_path}/com.xrite.Input"), &mut operands, 0);
    let resource = |operand: &str, kind: &str| -> Option<(Dataset, Option<(f32, f32)>, bool)> {
        let locations = file.dataset(&format!("{operand}/com.xrite.Resources/{kind}/ResourceLocations")).ok()?;
        let location = match &locations.values {
            crate::hdf5::Values::Strings(s) => s.first()?.clone(),
            _ => return None,
        };
        let place = format!("{resources}/{location}");
        let data = file.dataset(&format!("{place}/Data")).ok()?;
        let width = file.dataset(&format!("{place}/WidthMM")).ok().and_then(|d| d.numbers().and_then(|v| v.first().copied()));
        let height = file.dataset(&format!("{place}/HeightMM")).ok().and_then(|d| d.numbers().and_then(|v| v.first().copied()));
        let bottom = file.dataset(&format!("{place}/TextureOrigin")).ok().and_then(|d| d.text().map(|t| t.starts_with("BOTTOM"))).unwrap_or(false);
        let size = match (width, height) {
            (Some(w), Some(h)) => Some((w as f32 * 0.001, h as f32 * 0.001)),
            _ => None,
        };
        Some((data, size, bottom))
    };
    let has = |operand: &str, kind: &str| file.has(&format!("{operand}/com.xrite.Resources/{kind}"));
    let specular_op = operands.iter().find(|o| has(o, "Lobes")).cloned().ok_or("the AxF SVBRDF has no specular lobes")?;
    let diffuse_op = operands.iter().find(|o| **o != specular_op && has(o, "Color")).cloned();

    let mut size = None;
    let mut map_of = |found: Option<(Dataset, Option<(f32, f32)>, bool)>| -> Option<Map> {
        let (data, s, bottom) = found?;
        if size.is_none() {
            size = s;
        }
        to_map(&data, bottom)
    };
    let diffuse = diffuse_op.as_deref().and_then(|o| map_of(resource(o, "Color"))).unwrap_or(Map::Constant(vec![0.0; 3]));
    let colour = map_of(resource(&specular_op, "Color")).unwrap_or(Map::Constant(vec![1.0; 3]));
    let amount = map_of(resource(&specular_op, "SpecularAmount"));
    let lobes = map_of(resource(&specular_op, "Lobes")).ok_or("the AxF SVBRDF's lobes did not read")?;
    let normal = map_of(resource(&specular_op, "Normal")).or_else(|| diffuse_op.as_deref().and_then(|o| map_of(resource(o, "Normal"))));
    let fresnel = map_of(resource(&specular_op, "Fresnel"))
        .map(|m| {
            let mean = m.mean();
            mean.iter().sum::<f32>() / mean.len().max(1) as f32
        })
        .unwrap_or(0.04);
    // The amount scales the colour: one map of how strong the specular is.
    let specular = match (colour, amount) {
        (colour, None) => colour,
        (Map::Constant(c), Some(Map::Constant(a))) => Map::Constant(c.iter().map(|v| v * a.first().copied().unwrap_or(1.0)).collect()),
        (colour, Some(amount)) => {
            let a = amount.mean().first().copied().unwrap_or(1.0);
            match colour {
                Map::Constant(c) => Map::Constant(c.iter().map(|v| v * a).collect()),
                Map::Image { width, height, channels, data } => Map::Image { width, height, channels, data: data.iter().map(|v| v * a).collect() },
            }
        }
    };
    // A normal map of one flat normal is no normal map.
    let normal = normal.filter(|n| matches!(n, Map::Image { width, height, .. } if *width > 1 || *height > 1));
    Ok(Svbrdf { name, diffuse, specular, lobes, fresnel, normal, size: size.unwrap_or((0.1, 0.1)), car_paint, flakes })
}

fn collect_operands(file: &File, at: &str, out: &mut Vec<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    let operands = format!("{at}/com.xrite.Operands");
    let Ok(group) = file.find(&operands) else { return };
    let Ok(members) = file.members(group) else { return };
    for (name, _) in members {
        let path = format!("{operands}/{name}");
        out.push(path.clone());
        collect_operands(file, &path, out, depth + 1);
    }
}

/// A dataset as a map: a picture (height, width, channels), or a value.
fn to_map(data: &Dataset, bottom_up: bool) -> Option<Map> {
    let numbers: Vec<f32> = data.numbers()?.iter().map(|v| *v as f32).collect();
    match data.shape.as_slice() {
        [h, w, c] if *h > 1 || *w > 1 => {
            let (height, width, channels) = (*h, *w, *c);
            let mut data = numbers;
            if bottom_up {
                // Rows top first, as pictures are.
                let row = width * channels;
                let mut flipped = Vec::with_capacity(data.len());
                for y in (0..height).rev() {
                    flipped.extend_from_slice(&data[y * row..(y + 1) * row]);
                }
                data = flipped;
            }
            Some(Map::Image { width, height, channels, data })
        }
        [1, 1, _] => Some(Map::Constant(numbers)),
        _ => Some(Map::Constant(numbers)),
    }
}

/// How a map is written as a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// A colour, sRGB-encoded as photographs are.
    Colour,
    /// A number, the square root of a GGX alpha: a roughness.
    Roughness,
    /// A tangent-space normal, as normal maps store one.
    Normal,
}

/// `map` written to `path` as an 8-bit PNG.
pub fn write_png(map: &Map, encoding: Encoding, path: &Path) -> Result<PathBuf, String> {
    let Map::Image { width, height, channels, data } = map else {
        return Err("a single value is not a picture".into());
    };
    let (width, height, channels) = (*width, *height, *channels);
    let out_channels = if encoding == Encoding::Roughness { 1 } else { 3 };
    let mut bytes = Vec::with_capacity(width * height * out_channels);
    let srgb = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    for pixel in data.chunks_exact(channels) {
        match encoding {
            Encoding::Colour => {
                for c in 0..3 {
                    bytes.push((srgb(pixel[c.min(channels - 1)]) * 255.0).round() as u8);
                }
            }
            Encoding::Roughness => bytes.push((pixel[0].max(0.0).sqrt().clamp(0.0, 1.0) * 255.0).round() as u8),
            Encoding::Normal => {
                let (x, y, z) = (pixel[0], *pixel.get(1).unwrap_or(&0.0), *pixel.get(2).unwrap_or(&1.0));
                let length = (x * x + y * y + z * z).sqrt().max(1e-6);
                for v in [x / length, y / length, z / length] {
                    bytes.push(((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(if out_channels == 1 { png::ColorType::Grayscale } else { png::ColorType::Rgb });
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&bytes).map_err(|e| e.to_string())?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flakes made by Tequila's own model - a cone of normals, a coverage
    /// and a reflectance - measured as the BTF measures them, fit back to
    /// themselves.
    #[test]
    fn flakes_fit_back_to_the_cone_that_made_them() {
        let (coverage, reflectance, cone) = (0.03f32, 0.9f32, 24.0f32);
        let solid = std::f32::consts::TAU * (1.0 - cone.to_radians().cos());
        // Finely sampled, so the bins' own width hardly counts.
        let step = 0.5f32;
        let brdf: Vec<f32> = (0..120)
            .map(|j| {
                let h = (j as f32 * step).to_radians();
                if h.to_degrees() < cone {
                    coverage * reflectance / solid / (4.0 * h.cos().powi(3))
                } else {
                    0.0
                }
            })
            .collect();
        let fit = Flakes::fit(&brdf, step, [0.9, 0.81, 0.7], 0.0001).expect("a fit");
        assert!((fit.coverage_reflectance - coverage * reflectance).abs() < 0.002, "{fit:?}");
        assert!((fit.cone_degrees - cone).abs() < 1.0, "{fit:?}");
        assert_eq!(fit.colour[0], 1.0);
        assert!((fit.colour[2] - 0.7 / 0.9).abs() < 1e-5);
        // A texel's area: 0.1 mm square is a disc 0.113 mm across.
        assert!((fit.size - 0.000_112_8).abs() < 1e-6, "{fit:?}");
    }

    /// X-Rite keeps half floats as 16-bit integers; their bits read back.
    #[test]
    fn half_floats_read_from_their_bits() {
        assert_eq!(crate::hdf5::half(0x3c00), 1.0);
        assert_eq!(crate::hdf5::half(0xc000), -2.0);
        assert!((crate::hdf5::half(0x2e66) - 0.1).abs() < 1e-4);
        assert_eq!(crate::hdf5::half(0), 0.0);
    }

    /// AxF files named in `TEQUILA_AXF`: each reads to an SVBRDF.
    #[test]
    #[ignore = "reads files named in TEQUILA_AXF"]
    fn files_named_in_the_environment_read_as_svbrdfs() {
        let Ok(list) = std::env::var("TEQUILA_AXF") else { return };
        for path in list.split(';').filter(|p| !p.is_empty()) {
            let bytes = std::fs::read(path).unwrap();
            let svbrdf = read(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
            let kind = |m: &Map| match m {
                Map::Constant(v) => format!("constant {v:?}"),
                Map::Image { width, height, channels, .. } => format!("{width}x{height}x{channels} mean {:?}", m.mean()),
            };
            println!(
                "{path}: {} car paint {} flakes {:?} size {:?} fresnel {}\n  diffuse {}\n  specular {}\n  lobes {}\n  normal {}",
                svbrdf.name,
                svbrdf.car_paint,
                svbrdf.flakes,
                svbrdf.size,
                svbrdf.fresnel,
                kind(&svbrdf.diffuse),
                kind(&svbrdf.specular),
                kind(&svbrdf.lobes),
                svbrdf.normal.as_ref().map(kind).unwrap_or_default()
            );
        }
    }
}
