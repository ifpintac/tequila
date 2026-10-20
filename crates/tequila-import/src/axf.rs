//! X-Rite AxF measured materials, as Tequila materials.
//!
//! An AxF file (HDF5, read by [`crate::hdf5`]) holds a material scanned off
//! a real sample, in one or more representations. Tequila reads the
//! spatially varying BRDF (`SVBRDF`, or its energy-preserving variant): a
//! diffuse colour, a specular colour, a GGX lobe's width, a Fresnel
//! reflectance and a normal map, each a picture of the sample or a single
//! value, over a tile of the sample's size. A car paint's own
//! representation (`CarPaint2`: tabulated colour and measured flakes) is
//! not read; its SVBRDF, which every car paint file carries beside it, is.

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
    Ok(Svbrdf { name, diffuse, specular, lobes, fresnel, normal, size: size.unwrap_or((0.1, 0.1)), car_paint })
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
                "{path}: {} car paint {} size {:?} fresnel {}\n  diffuse {}\n  specular {}\n  lobes {}\n  normal {}",
                svbrdf.name,
                svbrdf.car_paint,
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
