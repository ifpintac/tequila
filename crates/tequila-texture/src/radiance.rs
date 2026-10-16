//! Radiance RGBE (`.hdr`) decoder and encoder.
//!
//! The format almost every free HDRI is distributed in, and the one
//! `TEQUILA_SPEC.md` §1.5 lists first. It is small enough to implement
//! directly, which keeps M0 free of a native image dependency.
//!
//! Format: an ASCII header terminated by a blank line, a resolution line, then
//! scanlines of RGBE pixels — three 8-bit mantissas sharing one 8-bit exponent.
//! Scanlines are either flat or run-length encoded ("new RLE"), and both
//! appear in the wild, so both are handled.

use crate::image::{ColorSpace, Image, ImageError};

/// Decode a Radiance RGBE file.
pub fn decode(bytes: &[u8]) -> Result<Image, ImageError> {
    let mut pos = 0usize;

    let read_line = |pos: &mut usize| -> Result<String, ImageError> {
        let start = *pos;
        while *pos < bytes.len() && bytes[*pos] != b'\n' {
            *pos += 1;
        }
        if *pos >= bytes.len() {
            return Err(ImageError::Decode("truncated header".into()));
        }
        let line = String::from_utf8_lossy(&bytes[start..*pos]).into_owned();
        *pos += 1;
        Ok(line)
    };

    // Magic. Some writers emit "#?RGBE" rather than "#?RADIANCE".
    let magic = read_line(&mut pos)?;
    if !magic.starts_with("#?") {
        return Err(ImageError::Decode(format!(
            "not a Radiance file: {magic:?}"
        )));
    }

    // Header lines until a blank one.
    let mut format_seen = false;
    let mut exposure = 1.0f32;
    loop {
        let line = read_line(&mut pos)?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("FORMAT=") {
            let rest = rest.trim();
            if rest != "32-bit_rle_rgbe" && rest != "32-bit_rle_xyze" {
                return Err(ImageError::Decode(format!("unsupported FORMAT {rest:?}")));
            }
            format_seen = true;
        } else if let Some(rest) = line.strip_prefix("EXPOSURE=") {
            // Cumulative: the spec allows several EXPOSURE lines, and the
            // stored values are divided by their product.
            if let Ok(v) = rest.trim().parse::<f32>() {
                if v != 0.0 {
                    exposure *= v;
                }
            }
        }
    }
    if !format_seen {
        return Err(ImageError::Decode("missing FORMAT line".into()));
    }

    // Resolution. Only the standard -Y/+X orientation is produced in practice.
    let resolution = read_line(&mut pos)?;
    let parts: Vec<&str> = resolution.split_whitespace().collect();
    if parts.len() != 4 || parts[0] != "-Y" || parts[2] != "+X" {
        return Err(ImageError::Decode(format!(
            "unsupported orientation {resolution:?}"
        )));
    }
    let height: usize = parts[1]
        .parse()
        .map_err(|_| ImageError::Decode("bad height".into()))?;
    let width: usize = parts[3]
        .parse()
        .map_err(|_| ImageError::Decode("bad width".into()))?;
    if width == 0 || height == 0 || width > 65535 {
        return Err(ImageError::Decode(format!(
            "bad dimensions {width}x{height}"
        )));
    }

    let mut data = vec![0.0f32; width * height * 3];
    let mut rgbe = vec![0u8; width * 4];
    let inv_exposure = 1.0 / exposure;

    for y in 0..height {
        read_scanline(bytes, &mut pos, width, &mut rgbe)?;
        for x in 0..width {
            let e = rgbe[x * 4 + 3];
            let (r, g, b) = if e == 0 {
                (0.0, 0.0, 0.0)
            } else {
                // The shared exponent is biased by 128, and the mantissas are
                // interpreted with an implicit 0.5 offset so that the value
                // lands mid-bucket rather than at its floor.
                let scale = f32::exp2(e as f32 - (128.0 + 8.0)) * inv_exposure;
                (
                    (rgbe[x * 4] as f32 + 0.5) * scale,
                    (rgbe[x * 4 + 1] as f32 + 0.5) * scale,
                    (rgbe[x * 4 + 2] as f32 + 0.5) * scale,
                )
            };
            let i = (y * width + x) * 3;
            data[i] = r;
            data[i + 1] = g;
            data[i + 2] = b;
        }
    }

    Ok(Image {
        width,
        height,
        channels: 3,
        data,
        color_space: ColorSpace::LinearSrgb,
    })
}

/// Encode an image as Radiance RGBE, the other plain format `docs/03` §5
/// asks environment export to write.
///
/// Flat scanlines. A flat line could be mistaken for a run-length-encoded
/// one only if a pixel began 2, 2 with its next two bytes spelling the
/// width — but the largest component's mantissa is always at least 128, so
/// that third byte would put the width past the 32767 the encoded form can
/// describe.
pub fn encode(image: &Image) -> Vec<u8> {
    let (width, height) = (image.width, image.height);
    let mut out = Vec::with_capacity(width * height * 4 + 128);
    out.extend_from_slice(b"#?RADIANCE
FORMAT=32-bit_rle_rgbe
SOFTWARE=Tequila

");
    out.extend_from_slice(format!("-Y {height} +X {width}
").as_bytes());
    for i in 0..width * height {
        let at = |k: usize| {
            let v = image.data[i * image.channels + k.min(image.channels - 1)];
            if v.is_finite() { v.max(0.0) } else { 0.0 }
        };
        out.extend_from_slice(&to_rgbe(at(0), at(1), at(2)));
    }
    out
}

/// One pixel as three mantissas sharing an exponent. The inverse of the
/// decoder's mid-bucket reading, to within half a step of the largest
/// component.
fn to_rgbe(r: f32, g: f32, b: f32) -> [u8; 4] {
    let largest = r.max(g).max(b);
    if largest < 1e-32 {
        return [0, 0, 0, 0];
    }
    // largest = mantissa * 2^exponent with the mantissa in [0.5, 1).
    let mut exponent = largest.log2().floor() as i32 + 1;
    if largest / f32::exp2(exponent as f32) >= 1.0 {
        exponent += 1;
    }
    let exponent = exponent.clamp(-128, 127);
    let scale = 256.0 / f32::exp2(exponent as f32);
    let byte = |v: f32| (v * scale).floor().clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b), (exponent + 128) as u8]
}

/// Read one scanline, handling both flat and run-length-encoded forms.
fn read_scanline(
    bytes: &[u8],
    pos: &mut usize,
    width: usize,
    out: &mut [u8],
) -> Result<(), ImageError> {
    let need = |pos: usize, n: usize| -> Result<(), ImageError> {
        if pos + n > bytes.len() {
            Err(ImageError::Decode("truncated pixel data".into()))
        } else {
            Ok(())
        }
    };

    need(*pos, 4)?;
    let header = [
        bytes[*pos],
        bytes[*pos + 1],
        bytes[*pos + 2],
        bytes[*pos + 3],
    ];

    // New-style RLE: magic 2,2 followed by the width as a big-endian u16.
    let is_rle = header[0] == 2
        && header[1] == 2
        && ((header[2] as usize) << 8 | header[3] as usize) == width
        && (8..32768).contains(&width);

    if !is_rle {
        // Flat scanline: `width` RGBE quadruples, interleaved.
        need(*pos, width * 4)?;
        out[..width * 4].copy_from_slice(&bytes[*pos..*pos + width * 4]);
        *pos += width * 4;
        return Ok(());
    }

    *pos += 4;
    // RLE stores each of the four components as its own run-encoded plane.
    for component in 0..4 {
        let mut x = 0usize;
        while x < width {
            need(*pos, 1)?;
            let count = bytes[*pos];
            *pos += 1;
            if count > 128 {
                // A run: one value repeated `count - 128` times.
                let run = count as usize - 128;
                if x + run > width {
                    return Err(ImageError::Decode("RLE run overruns scanline".into()));
                }
                need(*pos, 1)?;
                let value = bytes[*pos];
                *pos += 1;
                for _ in 0..run {
                    out[x * 4 + component] = value;
                    x += 1;
                }
            } else {
                // A literal span of `count` values.
                let span = count as usize;
                if span == 0 {
                    return Err(ImageError::Decode("zero-length RLE span".into()));
                }
                if x + span > width {
                    return Err(ImageError::Decode("RLE span overruns scanline".into()));
                }
                need(*pos, span)?;
                for k in 0..span {
                    out[(x + k) * 4 + component] = bytes[*pos + k];
                }
                *pos += span;
                x += span;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is written reads back, to RGBE's precision.
    #[test]
    fn what_is_encoded_decodes() {
        for width in [5usize, 300] {
            let height = 3;
            let mut data = Vec::new();
            for i in 0..width * height {
                let t = i as f32 / (width * height) as f32;
                data.extend_from_slice(&[t * 40.0, 0.001 + t, if i % 7 == 0 { 0.0 } else { 0.5 }]);
            }
            let image = Image {
                width,
                height,
                channels: 3,
                data,
                color_space: ColorSpace::LinearSrgb,
            };
            let back = decode(&encode(&image)).expect("it reads back");
            assert_eq!((back.width, back.height), (width, height));
            for (i, (a, b)) in image.data.iter().zip(&back.data).enumerate() {
                let pixel = &image.data[i / 3 * 3..i / 3 * 3 + 3];
                let largest = pixel.iter().cloned().fold(0.0f32, f32::max);
                assert!(
                    (a - b).abs() <= largest / 128.0 + 1e-6,
                    "width {width}, value {i}: wrote {a}, read {b}"
                );
            }
        }
    }

    /// Build a minimal flat-scanline file with known pixel values.
    fn flat_file(width: usize, height: usize, pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n");
        v.extend_from_slice(format!("-Y {height} +X {width}\n").as_bytes());
        for p in pixels {
            v.extend_from_slice(p);
        }
        v
    }

    #[test]
    fn decodes_a_flat_scanline() {
        // Exponent 128 means a scale of 2^0 / 256, so mantissa 128 is ~0.502.
        let file = flat_file(2, 1, &[[128, 64, 32, 128], [255, 255, 255, 128]]);
        let img = decode(&file).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        let p0 = img.texel(0, 0);
        assert!((p0.x - 0.5019).abs() < 1e-3, "{p0:?}");
        assert!((p0.y - 0.2519).abs() < 1e-3, "{p0:?}");
        assert!((p0.z - 0.1269).abs() < 1e-3, "{p0:?}");
    }

    #[test]
    fn zero_exponent_decodes_to_black() {
        let file = flat_file(1, 1, &[[200, 200, 200, 0]]);
        let img = decode(&file).unwrap();
        assert_eq!(img.texel(0, 0), tequila_core::math::vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn exponent_doubles_the_value() {
        let a = decode(&flat_file(1, 1, &[[128, 128, 128, 128]])).unwrap();
        let b = decode(&flat_file(1, 1, &[[128, 128, 128, 129]])).unwrap();
        assert!((b.texel(0, 0).x / a.texel(0, 0).x - 2.0).abs() < 1e-4);
    }

    /// Run-length encoding is what real HDRIs actually use, and the plane
    /// interleaving is easy to get wrong: components are stored as four
    /// separate encoded planes, not as interleaved quadruples.
    #[test]
    fn decodes_a_run_length_encoded_scanline() {
        let width = 16usize;
        let mut v = Vec::new();
        v.extend_from_slice(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n");
        v.extend_from_slice(format!("-Y 1 +X {width}\n").as_bytes());
        v.extend_from_slice(&[2, 2, (width >> 8) as u8, (width & 0xff) as u8]);
        // R: a run of 16 x 100. G: 16 literals 0..16. B: run of 16 x 50.
        // E: run of 16 x 128.
        v.extend_from_slice(&[128 + 16, 100]);
        v.push(width as u8);
        v.extend((0..width).map(|i| i as u8));
        v.extend_from_slice(&[128 + 16, 50]);
        v.extend_from_slice(&[128 + 16, 128]);

        let img = decode(&v).unwrap();
        assert_eq!(img.width, width);
        let scale = 1.0f32 / 256.0;
        for x in 0..width {
            let p = img.texel(x, 0);
            assert!((p.x - (100.5 * scale)).abs() < 1e-4, "x={x} {p:?}");
            assert!(
                (p.y - ((x as f32 + 0.5) * scale)).abs() < 1e-4,
                "x={x} {p:?}"
            );
            assert!((p.z - (50.5 * scale)).abs() < 1e-4, "x={x} {p:?}");
        }
    }

    #[test]
    fn rejects_malformed_files_without_panicking() {
        // `docs/05` §3.7 wants graceful errors rather than panics on malformed
        // input; an HDRI is untrusted data like any other file.
        assert!(decode(b"").is_err());
        assert!(decode(b"not radiance at all").is_err());
        assert!(decode(b"#?RADIANCE\n\n-Y 1 +X 1\n").is_err()); // no FORMAT
        assert!(
            decode(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n+Y 1 +X 1\n\x00\x00\x00\x00").is_err()
        );
        // Header promises more pixels than the file contains.
        let mut truncated = Vec::new();
        truncated.extend_from_slice(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 4 +X 4\n");
        truncated.extend_from_slice(&[1, 2, 3, 128]);
        assert!(decode(&truncated).is_err());
    }

    #[test]
    fn honours_the_exposure_header() {
        let mut v = Vec::new();
        v.extend_from_slice(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\nEXPOSURE=2.0\n\n");
        v.extend_from_slice(b"-Y 1 +X 1\n");
        v.extend_from_slice(&[128, 128, 128, 128]);
        let img = decode(&v).unwrap();
        // Stored values are divided by the exposure to recover radiance.
        assert!((img.texel(0, 0).x - 0.5019 / 2.0).abs() < 1e-3);
    }
}
