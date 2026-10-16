//! LUT files an OCIO config can reference.
//!
//! Three formats, which between them cover what a `FileTransform` in a
//! LUT-based config points at:
//!
//! - `.cube` — Iridas/Resolve, 1D or 3D, the format almost every vendor and
//!   every grading tool writes.
//! - `.spi1d` — Sony Pictures Imageworks 1D, which the ACES configs use for
//!   their shaper and inverse-shaper curves.
//! - `.spimtx` — a 3x3 matrix with an offset, which is how those configs carry
//!   primary conversions.
//!
//! The interpolation is the part worth being careful about. A 3D LUT sampled
//! with nearest-neighbour looks fine on a gradient and posterizes visibly on a
//! smooth product highlight — exactly the image this application exists to
//! make — so the 3D path is trilinear, and the 1D path is linear with clamped
//! ends rather than extrapolated, because extrapolating a tone curve past its
//! domain produces values that swing the wrong way.

use tequila_core::math::{vec3, Vec3};
use std::path::Path;

/// A loaded lookup table.
#[derive(Clone, Debug, PartialEq)]
pub enum Lut {
    /// A per-channel curve over `[domain.0, domain.1]`.
    OneD {
        /// `size` entries per channel, channel-major.
        channels: Vec<Vec<f32>>,
        domain: (f32, f32),
    },
    /// A cube of `size³` RGB entries, red varying fastest.
    ThreeD { size: usize, data: Vec<Vec3> },
    /// A 3x3 matrix with an offset.
    Matrix { matrix: [f32; 9], offset: Vec3 },
}

impl Lut {
    /// Read whichever format the extension names.
    pub fn load(path: &Path) -> Result<Lut, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("cube") => parse_cube(&text),
            Some("spi1d") => parse_spi1d(&text),
            Some("spimtx") => parse_spimtx(&text),
            other => Err(format!(
                "{}: unsupported LUT format {:?}",
                path.display(),
                other.unwrap_or("")
            )),
        }
    }

    pub fn apply(&self, c: Vec3) -> Vec3 {
        match self {
            Lut::OneD { channels, domain } => {
                let sample = |curve: &Vec<f32>, v: f32| -> f32 {
                    if curve.is_empty() {
                        return v;
                    }
                    let (lo, hi) = *domain;
                    let span = (hi - lo).abs().max(1e-9);
                    // Clamped, not extrapolated: a tone curve past its domain
                    // has no defined shape, and continuing its last slope sends
                    // highlights the wrong way.
                    let t = ((v - lo) / span).clamp(0.0, 1.0) * (curve.len() - 1) as f32;
                    let i = t.floor() as usize;
                    let j = (i + 1).min(curve.len() - 1);
                    let f = t - i as f32;
                    curve[i] * (1.0 - f) + curve[j] * f
                };
                let pick = |index: usize| -> &Vec<f32> {
                    channels.get(index).unwrap_or(&channels[0])
                };
                vec3(
                    sample(pick(0), c.x),
                    sample(pick(1), c.y),
                    sample(pick(2), c.z),
                )
            }
            Lut::ThreeD { size, data } => trilinear(*size, data, c),
            Lut::Matrix { matrix, offset } => vec3(
                matrix[0] * c.x + matrix[1] * c.y + matrix[2] * c.z + offset.x,
                matrix[3] * c.x + matrix[4] * c.y + matrix[5] * c.z + offset.y,
                matrix[6] * c.x + matrix[7] * c.y + matrix[8] * c.z + offset.z,
            ),
        }
    }
}

/// Trilinear sample of a cube, red varying fastest — the ordering every one of
/// these formats writes.
fn trilinear(size: usize, data: &[Vec3], c: Vec3) -> Vec3 {
    if size < 2 || data.len() < size * size * size {
        return c;
    }
    let last = (size - 1) as f32;
    let at = |r: usize, g: usize, b: usize| -> Vec3 { data[(b * size + g) * size + r] };

    let coord = |v: f32| -> (usize, usize, f32) {
        let t = v.clamp(0.0, 1.0) * last;
        let i = t.floor() as usize;
        let j = (i + 1).min(size - 1);
        (i, j, t - i as f32)
    };
    let (r0, r1, fr) = coord(c.x);
    let (g0, g1, fg) = coord(c.y);
    let (b0, b1, fb) = coord(c.z);

    let lerp = |a: Vec3, b: Vec3, t: f32| a * (1.0 - t) + b * t;
    let c00 = lerp(at(r0, g0, b0), at(r1, g0, b0), fr);
    let c10 = lerp(at(r0, g1, b0), at(r1, g1, b0), fr);
    let c01 = lerp(at(r0, g0, b1), at(r1, g0, b1), fr);
    let c11 = lerp(at(r0, g1, b1), at(r1, g1, b1), fr);
    lerp(lerp(c00, c10, fg), lerp(c01, c11, fg), fb)
}

fn parse_cube(text: &str) -> Result<Lut, String> {
    let mut size_1d = None;
    let mut size_3d = None;
    let mut domain = (0.0f32, 1.0f32);
    let mut entries: Vec<Vec3> = Vec::new();

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(first) = parts.next() else { continue };
        match first.to_ascii_uppercase().as_str() {
            "TITLE" => {}
            "LUT_1D_SIZE" => {
                size_1d = parts.next().and_then(|v| v.parse::<usize>().ok());
            }
            "LUT_3D_SIZE" => {
                size_3d = parts.next().and_then(|v| v.parse::<usize>().ok());
            }
            "DOMAIN_MIN" => {
                if let Some(v) = parts.next().and_then(|v| v.parse::<f32>().ok()) {
                    domain.0 = v;
                }
            }
            "DOMAIN_MAX" => {
                if let Some(v) = parts.next().and_then(|v| v.parse::<f32>().ok()) {
                    domain.1 = v;
                }
            }
            _ => {
                // A data row: three numbers.
                let values: Vec<f32> = line
                    .split_whitespace()
                    .filter_map(|v| v.parse::<f32>().ok())
                    .collect();
                if values.len() >= 3 {
                    entries.push(vec3(values[0], values[1], values[2]));
                }
            }
        }
    }

    if let Some(size) = size_3d {
        if entries.len() != size * size * size {
            return Err(format!(
                "a 3D cube of size {size} needs {} entries and has {}",
                size * size * size,
                entries.len()
            ));
        }
        return Ok(Lut::ThreeD { size, data: entries });
    }
    if let Some(size) = size_1d {
        if entries.len() != size {
            return Err(format!(
                "a 1D cube of size {size} needs {size} entries and has {}",
                entries.len()
            ));
        }
        let channels = vec![
            entries.iter().map(|v| v.x).collect(),
            entries.iter().map(|v| v.y).collect(),
            entries.iter().map(|v| v.z).collect(),
        ];
        return Ok(Lut::OneD { channels, domain });
    }
    Err("a .cube file with neither LUT_1D_SIZE nor LUT_3D_SIZE".into())
}

fn parse_spi1d(text: &str) -> Result<Lut, String> {
    let mut domain = (0.0f32, 1.0f32);
    let mut components = 1usize;
    let mut values: Vec<Vec<f32>> = Vec::new();
    let mut in_data = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "{" {
            in_data = true;
            continue;
        }
        if line == "}" {
            in_data = false;
            continue;
        }
        if !in_data {
            let mut parts = line.split_whitespace();
            let Some(key) = parts.next() else { continue };
            match key.to_ascii_lowercase().as_str() {
                "from" => {
                    let numbers: Vec<f32> =
                        parts.filter_map(|v| v.parse::<f32>().ok()).collect();
                    if numbers.len() >= 2 {
                        domain = (numbers[0], numbers[1]);
                    }
                }
                "components" => {
                    if let Some(v) = parts.next().and_then(|v| v.parse::<usize>().ok()) {
                        components = v.clamp(1, 3);
                        values = vec![Vec::new(); components];
                    }
                }
                _ => {}
            }
            continue;
        }
        if values.is_empty() {
            values = vec![Vec::new(); components];
        }
        let numbers: Vec<f32> = line
            .split_whitespace()
            .filter_map(|v| v.parse::<f32>().ok())
            .collect();
        for (i, channel) in values.iter_mut().enumerate() {
            // One component means one curve applied to all three channels,
            // which is how a shaper is written.
            channel.push(*numbers.get(i.min(numbers.len().saturating_sub(1))).unwrap_or(&0.0));
        }
    }

    if values.is_empty() || values[0].is_empty() {
        return Err("a .spi1d file with no data".into());
    }
    // A single component is broadcast, so the sampler can always index by
    // channel without asking how many there were.
    while values.len() < 3 {
        values.push(values[0].clone());
    }
    Ok(Lut::OneD {
        channels: values,
        domain,
    })
}

fn parse_spimtx(text: &str) -> Result<Lut, String> {
    let numbers: Vec<f32> = text
        .split_whitespace()
        .filter_map(|v| v.parse::<f32>().ok())
        .collect();
    if numbers.len() < 12 {
        return Err(format!(
            "a .spimtx file holds twelve numbers and this one holds {}",
            numbers.len()
        ));
    }
    // Rows of `[m00 m01 m02 offset]`, and the offsets are in the file's own
    // scale — 1.0 rather than 0..255, which is what OCIO writes.
    Ok(Lut::Matrix {
        matrix: [
            numbers[0], numbers[1], numbers[2], numbers[4], numbers[5], numbers[6], numbers[8],
            numbers[9], numbers[10],
        ],
        offset: vec3(numbers[3], numbers[7], numbers[11]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3D cube must interpolate, not snap. Nearest-neighbour sampling looks
    /// correct on a ramp and posterizes a smooth product highlight, which is
    /// the one image this application exists to make.
    #[test]
    fn a_three_d_cube_interpolates() {
        // A 2x2x2 identity cube.
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{r}.0 {g}.0 {b}.0\n"));
                }
            }
        }
        let lut = parse_cube(&text).unwrap();
        for probe in [0.0, 0.25, 0.5, 0.9, 1.0] {
            let out = lut.apply(vec3(probe, probe, probe));
            assert!(
                (out.x - probe).abs() < 1e-5 && (out.y - probe).abs() < 1e-5,
                "identity cube moved {probe} to {out:?}"
            );
        }
        // And it really is sampling the cube rather than passing values
        // through: an inverted cube has to invert.
        let inverted = text.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE 2");
        let _ = inverted;
        let doubled: String = text
            .lines()
            .map(|l| {
                if l.starts_with("LUT") {
                    l.to_string()
                } else {
                    l.split_whitespace()
                        .map(|v| format!("{}", v.parse::<f32>().unwrap() * 0.5))
                        .collect::<Vec<_>>()
                        .join(" ")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let half = parse_cube(&doubled).unwrap();
        let out = half.apply(vec3(1.0, 1.0, 1.0));
        assert!((out.x - 0.5).abs() < 1e-5, "{out:?}");
    }

    /// Red varies fastest. Getting the axis order backwards produces a LUT that
    /// is subtly wrong everywhere and exactly right on the grey axis, which is
    /// where anyone would check it.
    #[test]
    fn the_cube_axis_order_is_red_fastest() {
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    // Encode the index so a mis-ordered read is visible.
                    text.push_str(&format!("{}.0 {}.0 {}.0\n", r, g * 2, b * 4));
                }
            }
        }
        let lut = parse_cube(&text).unwrap();
        let out = lut.apply(vec3(1.0, 0.0, 0.0));
        assert!((out.x - 1.0).abs() < 1e-5, "red axis: {out:?}");
        let out = lut.apply(vec3(0.0, 0.0, 1.0));
        assert!((out.z - 4.0).abs() < 1e-5, "blue axis: {out:?}");
    }

    /// A 1D curve is clamped at its domain rather than extrapolated.
    #[test]
    fn a_one_d_curve_clamps_outside_its_domain() {
        let text = "Version 1\nFrom 0.0 1.0\nLength 3\nComponents 1\n{\n0.0\n0.5\n1.0\n}\n";
        let lut = parse_spi1d(text).unwrap();
        assert!((lut.apply(vec3(0.5, 0.5, 0.5)).x - 0.5).abs() < 1e-5);
        assert!((lut.apply(vec3(4.0, 4.0, 4.0)).x - 1.0).abs() < 1e-5);
        assert!((lut.apply(vec3(-2.0, -2.0, -2.0)).x - 0.0).abs() < 1e-5);
    }

    #[test]
    fn a_matrix_file_applies_its_offset() {
        let text = "1 0 0 0.1\n0 1 0 0.2\n0 0 1 0.3\n";
        let lut = parse_spimtx(text).unwrap();
        let out = lut.apply(vec3(1.0, 1.0, 1.0));
        assert!((out.x - 1.1).abs() < 1e-5 && (out.z - 1.3).abs() < 1e-5, "{out:?}");
    }

    /// A truncated file is an error, not a silently wrong transform.
    #[test]
    fn a_malformed_lut_is_reported() {
        assert!(parse_cube("LUT_3D_SIZE 4\n0 0 0\n").is_err());
        assert!(parse_spimtx("1 0 0\n").is_err());
        assert!(parse_cube("TITLE nothing\n").is_err());
    }
}
