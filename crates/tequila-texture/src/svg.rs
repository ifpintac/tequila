//! SVG artwork (`.svg`, and `.svgz` gzipped), rasterized.
//!
//! A product's printed artwork - a label, a logo, a silkscreen's markings -
//! is very often drawn as vectors, and an SVG is how it leaves the drawing
//! program. The renderer samples pictures, so the drawing becomes one here,
//! through resvg: the whole of static SVG, with its viewBox, transparency
//! and text. resvg draws in sRGB as browsers do, so the result is treated
//! as an 8-bit sRGB picture like a PNG.
//!
//! The pixel size is chosen from the document rather than taken from it. A
//! drawing's nominal size is usually small - a 40 mm label is 151 pixels at
//! the 96 a CSS pixel is - and a label seen close fills the frame, so the
//! long side is brought up to [`DEFAULT_LONG_SIDE`] at least. A document
//! that asks for more gets it, up to [`MAX_LONG_SIDE`]: past that a single
//! picture costs over a gigabyte as floats, for detail no render resolves.
//!
//! Text needs fonts, and SVG names them rather than carrying them, so the
//! system's own are loaded once ([`fonts`]); a silkscreen's lettering
//! otherwise simply would not appear.
//!
//! A document whose width and height are physical (millimetres,
//! centimetres, inches, points, picas) says its print size, and a label
//! made from it arrives at that size ([`print_size`]), as a PSD's does by
//! its resolution.

use resvg::{tiny_skia, usvg};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// The long side a drawing is rasterized at when its own size is smaller.
pub const DEFAULT_LONG_SIDE: u32 = 4096;
/// The most a drawing is ever rasterized at.
pub const MAX_LONG_SIDE: u32 = 8192;

/// Rasterize an SVG file to 8-bit RGBA, unassociated, from the top row
/// down. Relative references (an embedded `<image>`'s file) resolve from
/// the file's own folder.
pub fn rasterize(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    rasterize_bytes(&bytes, path.parent())
}

pub(crate) fn rasterize_bytes(bytes: &[u8], folder: Option<&Path>) -> Result<(usize, usize, Vec<u8>), String> {
    let options = usvg::Options {
        resources_dir: folder.map(Path::to_path_buf),
        fontdb: fonts(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (width, height) = pixel_size(size.width(), size.height());
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("cannot make a {width}x{height} picture of this SVG"))?;
    let transform = tiny_skia::Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia keeps its pixels premultiplied; the renderer's images do not.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok((width as usize, height as usize, rgba))
}

/// The raster size for a drawing `width` by `height` CSS pixels: its long
/// side brought into [`DEFAULT_LONG_SIDE`]..=[`MAX_LONG_SIDE`], its
/// proportions kept.
pub fn pixel_size(width: f32, height: f32) -> (u32, u32) {
    let long = width.max(height).max(f32::MIN_POSITIVE);
    let target = long.round().clamp(DEFAULT_LONG_SIDE as f32, MAX_LONG_SIDE as f32);
    let scale = target / long;
    let side = |v: f32| ((v * scale).round() as u32).clamp(1, MAX_LONG_SIDE);
    (side(width), side(height))
}

/// The system's fonts, read once for the life of the process: reading
/// them takes a moment, and every SVG with text needs them.
///
/// SVG's generic families, and the serif usvg falls back to when nothing
/// named is installed, default to the Windows fonts; where those are
/// missing (most Linux systems) they are pointed at fonts that are there,
/// so that text renders in something rather than not at all.
pub fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            let installed = |name: &str| fonts.faces().any(|f| f.families.iter().any(|(family, _)| family == name));
            let pick = |wanted: &[&str]| {
                wanted
                    .iter()
                    .find(|name| installed(name))
                    .map(|name| name.to_string())
                    .or_else(|| fonts.faces().next().and_then(|f| f.families.first()).map(|(family, _)| family.clone()))
            };
            let serif = pick(&["Times New Roman", "DejaVu Serif", "Liberation Serif", "Noto Serif", "FreeSerif"]);
            let sans = pick(&["Arial", "Helvetica", "DejaVu Sans", "Liberation Sans", "Noto Sans", "FreeSans"]);
            let mono = pick(&["Courier New", "DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "FreeMono"]);
            if let Some(family) = serif {
                fonts.set_serif_family(family);
            }
            if let Some(family) = sans {
                fonts.set_sans_serif_family(family);
            }
            if let Some(family) = mono {
                fonts.set_monospace_family(family);
            }
            Arc::new(fonts)
        })
        .clone()
}

/// Width and height in metres, when the root element gives them in a
/// physical unit. Pixels, percentages and bare numbers say nothing about
/// print size.
pub fn print_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let text = if bytes.starts_with(&[0x1F, 0x8B]) {
        String::from_utf8(usvg::decompress_svgz(bytes).ok()?).ok()?
    } else {
        std::str::from_utf8(bytes).ok()?.to_owned()
    };
    let options = usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let document = usvg::roxmltree::Document::parse_with_options(&text, options).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return None;
    }
    Some((physical(root.attribute("width")?)?, physical(root.attribute("height")?)?))
}

/// A length in metres, if its unit is a physical one.
fn physical(length: &str) -> Option<f32> {
    const UNITS: [(&str, f32); 6] =
        [("mm", 0.001), ("cm", 0.01), ("in", 0.0254), ("pt", 0.0254 / 72.0), ("pc", 0.0254 / 6.0), ("Q", 0.00025)];
    let length = length.trim();
    let (number, metres) = UNITS.iter().find_map(|(unit, metres)| length.strip_suffix(unit).map(|n| (n, *metres)))?;
    let value = number.trim().parse::<f32>().ok()? * metres;
    (value.is_finite() && value > 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &(usize, usize, Vec<u8>), x: usize, y: usize) -> [u8; 4] {
        let i = (y * image.0 + x) * 4;
        image.2[i..i + 4].try_into().unwrap()
    }

    /// A 40 by 20 mm label: the viewBox maps four units across onto the
    /// whole width, and the long side comes out at the default size.
    const LABEL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="20mm" viewBox="0 0 4 2">
        <rect x="0" y="0" width="2" height="1" fill="#e01e1e"/>
        <rect x="2" y="0" width="2" height="1" fill="#1ec828" fill-opacity="0.5"/>
        <rect x="0" y="1" width="2" height="1" fill="#1e28dc"/>
    </svg>"##;

    #[test]
    fn the_view_box_fills_the_picture_and_transparency_is_kept() {
        let image = rasterize_bytes(LABEL.as_bytes(), None).unwrap();
        assert_eq!((image.0, image.1), (4096, 2048), "the long side at the default, proportions kept");
        assert_eq!(pixel(&image, 1024, 512), [0xE0, 0x1E, 0x1E, 255], "top left is red");
        let half = pixel(&image, 3072, 512);
        assert!((half[3] as i32 - 128).abs() <= 1, "half-transparent green keeps its alpha: {half:?}");
        assert!((half[1] as i32 - 0xC8).abs() <= 2, "and its colour, unpremultiplied: {half:?}");
        assert_eq!(pixel(&image, 1024, 1536), [0x1E, 0x28, 0xDC, 255], "bottom left is blue");
        assert_eq!(pixel(&image, 3072, 1536)[3], 0, "where nothing is drawn is transparent");
    }

    #[test]
    fn a_physical_size_is_a_print_size() {
        let (w, h) = print_size(LABEL.as_bytes()).expect("millimetres are a print size");
        assert!((w - 0.04).abs() < 1e-6 && (h - 0.02).abs() < 1e-6, "{w} x {h}");
        let inches = LABEL.replace("40mm", "2in").replace("20mm", "72pt");
        let (w, h) = print_size(inches.as_bytes()).unwrap();
        assert!((w - 0.0508).abs() < 1e-6 && (h - 0.0254).abs() < 1e-6, "{w} x {h}");
        let pixels = LABEL.replace("40mm", "400").replace("20mm", "200px");
        assert!(print_size(pixels.as_bytes()).is_none(), "pixels are not a print size");
        assert!(print_size(b"not an svg").is_none());
    }

    #[test]
    fn the_raster_size_is_chosen_from_the_document() {
        assert_eq!(pixel_size(151.0, 75.5), (4096, 2048), "small drawings come up to the default");
        assert_eq!(pixel_size(100.0, 400.0), (1024, 4096), "the long side is the one sized");
        assert_eq!(pixel_size(6000.0, 3000.0), (6000, 3000), "a larger document is kept at its size");
        assert_eq!(pixel_size(20000.0, 10000.0), (8192, 4096), "and never past the most");
        assert_eq!(pixel_size(10000.0, 1.0), (8192, 1), "a hairline stays at least a pixel");
    }

    /// Lettering needs the system's fonts. A machine with none at all
    /// cannot show it, and says so rather than failing.
    #[test]
    fn text_renders_with_the_system_fonts() {
        if fonts().is_empty() {
            eprintln!("no system fonts here; text cannot be drawn");
            return;
        }
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <text x="10" y="80" font-family="sans-serif" font-size="80" fill="black">WW</text>
        </svg>"#;
        let image = rasterize_bytes(svg.as_bytes(), None).unwrap();
        let inked = image.2.chunks_exact(4).filter(|p| p[3] > 128).count();
        assert!(inked > image.0 * image.1 / 50, "the text did not render: {inked} pixels inked");
    }

    #[test]
    fn a_gzipped_svg_reads_the_same() {
        let gz = gzip_stored(LABEL.as_bytes());
        let image = rasterize_bytes(&gz, None).unwrap();
        assert_eq!((image.0, image.1), (4096, 2048));
        assert_eq!(pixel(&image, 1024, 512), [0xE0, 0x1E, 0x1E, 255]);
        assert!(print_size(&gz).is_some());
    }

    /// A gzip stream of stored (uncompressed) deflate blocks, which needs
    /// no compressor to write.
    fn gzip_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
        let chunks: Vec<&[u8]> = data.chunks(65535).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            out.push(if i + 1 == chunks.len() { 1 } else { 0 });
            out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
            out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
            out.extend_from_slice(chunk);
        }
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }
}
