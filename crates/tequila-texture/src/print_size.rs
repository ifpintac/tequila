//! The size artwork was made to print at (`TEQUILA_SPEC.md` §1.11: "import
//! reads embedded DPI to preserve intended real-world label size").
//!
//! A label designed at 40 by 18 millimetres should arrive at 40 by 18
//! millimetres, not at whatever size fits the face it lands on. The file
//! says how many pixels make an inch — PNG in its `pHYs` chunk, in pixels
//! per metre; JPEG in its JFIF header, per inch or per centimetre; a PSD in
//! its resolution resource; a TIFF in its resolution tags; a BMP in its
//! header, per metre — and the pixel count divided by that is the print
//! size. An SVG says its size outright, when it gives it in millimetres,
//! inches or points ([`crate::svg::print_size`]).
//!
//! Read from the raw bytes rather than through a decoder: the density is a
//! few bytes near the start, and decoding a large image to find it would
//! cost far more than the answer is worth.

use std::path::Path;

/// Width and height in metres, if the file says what it was made to print
/// at. `None` when it does not, or says something no one means — a density
/// of one pixel per inch is a default, not a decision.
pub fn print_size(path: impl AsRef<Path>) -> Option<(f32, f32)> {
    let bytes = std::fs::read(path).ok()?;
    png(&bytes)
        .or_else(|| jpeg(&bytes))
        .or_else(|| crate::psd::resolution(&bytes).and_then(|(w, h, x, y)| sized(w, h, x, y)))
        .or_else(|| tiff(&bytes))
        .or_else(|| crate::bmp::resolution(&bytes).and_then(|(w, h, x, y)| sized(w, h, x, y)))
        .or_else(|| crate::svg::print_size(&bytes))
}

/// The same from a TIFF's first image: `XResolution` and `YResolution`,
/// per inch unless `ResolutionUnit` says centimetres, and turned with the
/// picture when its orientation lays it on its side.
fn tiff(bytes: &[u8]) -> Option<(f32, f32)> {
    use tiff::decoder::{ifd::Value, Decoder};
    use tiff::tags::Tag;
    if !(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")) {
        return None;
    }
    let mut decoder = Decoder::new(std::io::Cursor::new(bytes)).ok()?;
    let (width, height) = decoder.dimensions().ok()?;
    let mut density = |tag: Tag| match decoder.find_tag(tag).ok()?? {
        Value::Rational(n, d) if d != 0 => Some(n as f32 / d as f32),
        Value::Float(v) => Some(v),
        Value::Short(v) => Some(v as f32),
        Value::Unsigned(v) => Some(v as f32),
        _ => None,
    };
    let (x, y) = (density(Tag::XResolution)?, density(Tag::YResolution)?);
    let per_metre = match decoder.find_tag_unsigned::<u16>(Tag::ResolutionUnit).ok()?.unwrap_or(2) {
        2 => 1.0 / 0.0254,
        3 => 100.0,
        // 1 is an aspect ratio, with no size.
        _ => return None,
    };
    let lying = matches!(decoder.find_tag_unsigned::<u16>(Tag::Orientation).ok()?.unwrap_or(1), 5..=8);
    let (w, h, x, y) = if lying { (height, width, y, x) } else { (width, height, x, y) };
    sized(w as f32, h as f32, x * per_metre, y * per_metre)
}

/// Pixels per metre across and down, and the pixel size, from a PNG.
fn png(bytes: &[u8]) -> Option<(f32, f32)> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let be = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    // IHDR is always first.
    let (width, height) = (be(16) as f32, be(20) as f32);
    let mut at = 8;
    while at + 12 <= bytes.len() {
        let length = be(at) as usize;
        let kind = &bytes[at + 4..at + 8];
        if kind == b"pHYs" && length >= 9 && at + 8 + 9 <= bytes.len() {
            let (x, y, unit) = (be(at + 8) as f32, be(at + 12) as f32, bytes[at + 16]);
            // Unit 1 is metres; 0 is an aspect ratio only, with no size.
            if unit != 1 {
                return None;
            }
            return sized(width, height, x, y);
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        at += 12 + length;
    }
    None
}

/// The same from a JPEG's JFIF APP0 segment.
fn jpeg(bytes: &[u8]) -> Option<(f32, f32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut density: Option<(f32, f32)> = None;
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        let body = at + 4;
        if marker == 0xE0 && body + 12 <= bytes.len() && &bytes[body..body + 5] == b"JFIF\0" {
            let unit = bytes[body + 7];
            let x = u16::from_be_bytes([bytes[body + 8], bytes[body + 9]]) as f32;
            let y = u16::from_be_bytes([bytes[body + 10], bytes[body + 11]]) as f32;
            // Per inch, or per centimetre; into pixels per metre.
            density = match unit {
                1 => Some((x / 0.0254, y / 0.0254)),
                2 => Some((x * 100.0, y * 100.0)),
                _ => None,
            };
        }
        // The frame header carries the pixel size.
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) && body + 5 <= bytes.len() {
            let height = u16::from_be_bytes([bytes[body + 1], bytes[body + 2]]) as f32;
            let width = u16::from_be_bytes([bytes[body + 3], bytes[body + 4]]) as f32;
            let (x, y) = density?;
            return sized(width, height, x, y);
        }
        if marker == 0xDA {
            return None;
        }
        at += 2 + length;
    }
    None
}

fn sized(width: f32, height: f32, per_metre_x: f32, per_metre_y: f32) -> Option<(f32, f32)> {
    // Below 2 dpi is a placeholder, not a print size.
    let plausible = |d: f32| d.is_finite() && d > 2.0 / 0.0254;
    if !(plausible(per_metre_x) && plausible(per_metre_y)) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((width / per_metre_x, height / per_metre_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC, not checked here
        out
    }

    fn png_with(dpi: Option<f32>) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = 600u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&300u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        out.extend(chunk(b"IHDR", &ihdr));
        if let Some(dpi) = dpi {
            let per_metre = (dpi / 0.0254).round() as u32;
            let mut phys = per_metre.to_be_bytes().to_vec();
            phys.extend_from_slice(&per_metre.to_be_bytes());
            phys.push(1);
            out.extend(chunk(b"pHYs", &phys));
        }
        out.extend(chunk(b"IDAT", &[]));
        out
    }

    #[test]
    fn a_psd_says_its_print_size() {
        // A designer's label, 600 by 300 at 300 dpi: two inches by one.
        let pixels = vec![[255, 255, 255, 255]; 600 * 300];
        let path = std::env::temp_dir().join(format!("tequila-label-{}.psd", std::process::id()));
        std::fs::write(&path, crate::psd::tests::write(600, 300, &pixels, true, Some(300.0))).unwrap();
        let size = print_size(&path);
        std::fs::write(&path, crate::psd::tests::write(600, 300, &pixels, true, None)).unwrap();
        let unsized_ = print_size(&path);
        let _ = std::fs::remove_file(&path);
        let (w, h) = size.expect("a size");
        assert!((w - 0.0508).abs() < 1e-4 && (h - 0.0254).abs() < 1e-4, "{w} x {h}");
        assert!(unsized_.is_none());
    }

    #[test]
    fn a_png_says_its_print_size() {
        // 600 by 300 pixels at 300 dpi is two inches by one.
        let (w, h) = png(&png_with(Some(300.0))).expect("a size");
        assert!((w - 0.0508).abs() < 1e-4 && (h - 0.0254).abs() < 1e-4, "{w} x {h}");
        assert!(png(&png_with(None)).is_none());
        assert!(png(&png_with(Some(1.0))).is_none(), "a placeholder density was believed");
    }

    #[test]
    fn a_jpeg_says_its_print_size() {
        let mut out = vec![0xFF, 0xD8];
        // APP0 JFIF, 1.1, units per inch, 150 by 150.
        let mut app0 = b"JFIF\0".to_vec();
        app0.extend_from_slice(&[1, 1, 1]);
        app0.extend_from_slice(&150u16.to_be_bytes());
        app0.extend_from_slice(&150u16.to_be_bytes());
        app0.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&[0xFF, 0xE0]);
        out.extend_from_slice(&((app0.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app0);
        // SOF0: precision 8, height 75, width 300, one component.
        let sof = [8u8, 0, 75, 1, 44, 1, 1, 0x11, 0];
        out.extend_from_slice(&[0xFF, 0xC0]);
        out.extend_from_slice(&((sof.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&sof);
        let (w, h) = jpeg(&out).expect("a size");
        // 300 by 75 at 150 dpi: two inches by half an inch.
        assert!((w - 0.0508).abs() < 1e-4 && (h - 0.0127).abs() < 1e-4, "{w} x {h}");
    }

    #[test]
    fn a_tiff_says_its_print_size() {
        use tiff::encoder::{colortype, Rational, TiffEncoder};
        use tiff::tags::ResolutionUnit;
        let write = |unit: ResolutionUnit, per_unit: u32| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
            let mut image = encoder.new_image::<colortype::Gray8>(600, 300).unwrap();
            image.resolution(unit, Rational { n: per_unit, d: 1 });
            image.write_data(&vec![0u8; 600 * 300]).unwrap();
            bytes.into_inner()
        };
        let (w, h) = tiff(&write(ResolutionUnit::Inch, 300)).expect("a size");
        assert!((w - 0.0508).abs() < 1e-4 && (h - 0.0254).abs() < 1e-4, "{w} x {h}");
        // 600 pixels at 100 a centimetre is six centimetres.
        let (w, _) = tiff(&write(ResolutionUnit::Centimeter, 100)).expect("a size");
        assert!((w - 0.06).abs() < 1e-4, "{w}");
        assert!(tiff(&write(ResolutionUnit::None, 300)).is_none(), "an aspect ratio is not a size");
    }

    /// Through the path, as the Labels tab asks: a BMP's density and an
    /// SVG's millimetres.
    #[test]
    fn a_bmp_and_an_svg_say_their_print_sizes() {
        let mut bmp = b"BM".to_vec();
        let mut dib = vec![0u8; 40];
        dib[0] = 40;
        dib[4..8].copy_from_slice(&600i32.to_le_bytes());
        dib[8..12].copy_from_slice(&300i32.to_le_bytes());
        dib[14] = 24;
        // 11811 pixels a metre: 300 dpi.
        dib[24..28].copy_from_slice(&11811i32.to_le_bytes());
        dib[28..32].copy_from_slice(&11811i32.to_le_bytes());
        bmp.extend_from_slice(&[0; 8]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&dib);
        let path = std::env::temp_dir().join(format!("tequila-print-{}.bmp", std::process::id()));
        std::fs::write(&path, &bmp).unwrap();
        let size = print_size(&path);
        let svg = path.with_extension("svg");
        std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="18mm"/>"#).unwrap();
        let drawn = print_size(&svg);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&svg);
        let (w, h) = size.expect("a BMP's size");
        assert!((w - 0.0508).abs() < 1e-4 && (h - 0.0254).abs() < 1e-4, "{w} x {h}");
        let (w, h) = drawn.expect("an SVG's size");
        assert!((w - 0.040).abs() < 1e-6 && (h - 0.018).abs() < 1e-6, "{w} x {h}");
    }
}
