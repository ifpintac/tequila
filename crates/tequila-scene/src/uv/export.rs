//! The layout as a picture: an SVG to draw artwork over in a drawing
//! program, and a PNG made from it.
//!
//! The unit square, `size` pixels a side, v down as the texture's rows run,
//! so artwork drawn over it lands where it prints. Every edge of the layout
//! as a thin line, its seams in colour over them, and the edge of the part
//! darker; the islands filled each in a colour of its own, or by how much
//! the layout stretches each triangle. Where the layout is as dense all
//! over, a note in the corner says how large the square is on the part.

use super::distortion::{distortion, heat, Measure};
use super::edit::{layout_of, UvError};
use super::layout::EdgeKind;
use crate::geometry::Mesh;
use std::collections::BTreeMap;
use std::fmt::Write;
use tequila_core::math::Vec2;

/// What the islands are filled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fill {
    /// Not filled: lines only.
    None,
    /// Each island a colour of its own.
    #[default]
    Islands,
    /// Each triangle by how much the layout stretches it.
    Distortion(Measure),
}

/// How a layout is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutPicture {
    /// The square's side, in pixels.
    pub size: u32,
    pub fill: Fill,
    /// Every edge, inside the islands too.
    pub edges: bool,
    /// The seams, in colour.
    pub seams: bool,
    /// The square's size on the part, where the layout says it.
    pub note: bool,
    /// The colour the lines are drawn in, sRGB - the edges, the part's edge,
    /// and the seams when they are not in colour - or the usual greys.
    pub line: Option<[u8; 3]>,
    /// No white square under the layout: a PNG is clear where nothing is
    /// drawn, to lay over artwork.
    pub transparent: bool,
}

impl Default for LayoutPicture {
    fn default() -> Self {
        LayoutPicture { size: 2048, fill: Fill::Islands, edges: true, seams: true, note: true, line: None, transparent: false }
    }
}

/// The largest picture made, a side.
pub const MOST_PIXELS: u32 = 16_384;

/// The layout of set `set` as an SVG document.
pub fn layout_svg(mesh: &Mesh, set: usize, picture: &LayoutPicture) -> Result<String, UvError> {
    let layout = layout_of(mesh, set)?;
    let size = picture.size.clamp(16, MOST_PIXELS);
    let s = size as f32;
    let at = |p: Vec2| (p.x * s, p.y * s);
    let stroke = (s / 1024.0).max(0.5);
    let mut out = String::with_capacity(layout.edges.len() * 40 + 4096);
    let _ = writeln!(out, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">"#);
    let _ = writeln!(out, "<title>UV layout, set {set}</title>");
    if !picture.transparent {
        let _ = writeln!(out, r##"<rect width="{size}" height="{size}" fill="#ffffff"/>"##);
    }
    // The lines: the usual greys, or one colour, the inner edges lighter.
    let (inner, outer, opacity) = match picture.line {
        Some(colour) => (hex(colour), hex(colour), 0.55),
        None => ("#5a5f66".to_string(), "#1d2024".to_string(), 1.0),
    };
    let triangle = |out: &mut String, t: u32| {
        let [a, b, c] = layout.triangle_uv(t).map(at);
        let _ = write!(out, "M{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}Z", a.0, a.1, b.0, b.1, c.0, c.1);
    };
    match picture.fill {
        Fill::None => {}
        Fill::Islands => {
            let _ = writeln!(out, r#"<g id="islands" fill-opacity="0.45">"#);
            for (i, island) in layout.islands.iter().enumerate() {
                let _ = write!(out, r#"<path fill="{}" d=""#, island_colour(i));
                for &t in &island.triangles {
                    triangle(&mut out, t);
                }
                let _ = writeln!(out, r#""/>"#);
            }
            let _ = writeln!(out, "</g>");
        }
        Fill::Distortion(measure) => {
            // One path for each colour, coarsened to a sixteenth a channel.
            let all = distortion(mesh, set)?;
            let mut by_colour: BTreeMap<[u8; 3], String> = BTreeMap::new();
            for (t, d) in all.iter().enumerate() {
                let colour = heat(d, measure).map(|c| (c / 16) * 16 + 8);
                triangle(by_colour.entry(colour).or_default(), t as u32);
            }
            let _ = writeln!(out, r#"<g id="distortion">"#);
            for (colour, path) in by_colour {
                let _ = writeln!(out, r#"<path fill="{}" d="{path}"/>"#, hex(colour));
            }
            let _ = writeln!(out, "</g>");
        }
    }
    let lines = |kind: EdgeKind| {
        let mut d = String::new();
        for edge in layout.edges.iter().filter(|e| e.kind == kind) {
            let [a, b] = edge.ends.map(|v| at(layout.uv_vertices[v as usize].uv));
            let _ = write!(d, "M{:.2} {:.2}L{:.2} {:.2}", a.0, a.1, b.0, b.1);
        }
        d
    };
    if picture.edges {
        let _ = writeln!(
            out,
            r##"<path id="edges" fill="none" stroke="{inner}" stroke-opacity="{opacity}" stroke-width="{:.2}" stroke-linecap="round" d="{}"/>"##,
            stroke * 0.6,
            lines(EdgeKind::Inner)
        );
    }
    let _ = writeln!(out, r##"<path id="boundary" fill="none" stroke="{outer}" stroke-width="{:.2}" stroke-linecap="round" d="{}"/>"##, stroke, lines(EdgeKind::Boundary));
    if picture.seams {
        let _ = writeln!(out, r##"<path id="seams" fill="none" stroke="#e8571f" stroke-width="{:.2}" stroke-linecap="round" d="{}"/>"##, stroke * 1.5, lines(EdgeKind::Seam));
    } else {
        let _ = writeln!(out, r##"<path id="seams" fill="none" stroke="{outer}" stroke-width="{:.2}" stroke-linecap="round" d="{}"/>"##, stroke, lines(EdgeKind::Seam));
    }
    let _ = writeln!(out, r##"<rect width="{size}" height="{size}" fill="none" stroke="#9aa0a6" stroke-width="{:.2}"/>"##, stroke * 2.0);
    if picture.note {
        if let Some(note) = size_note(layout.units_per_uv, size) {
            let font = (s / 56.0).max(9.0);
            let _ = writeln!(
                out,
                r##"<text x="{:.1}" y="{:.1}" font-family="sans-serif" font-size="{:.1}" fill="#1d2024" stroke="#ffffff" stroke-width="{:.1}" paint-order="stroke">{note}</text>"##,
                font * 0.6,
                s - font * 0.6,
                font,
                font * 0.25
            );
        }
    }
    out.push_str("</svg>\n");
    Ok(out)
}

/// The note on a layout as dense all over: how large its square is on the
/// part, and a pixel.
pub fn size_note(units_per_uv: Option<f32>, size: u32) -> Option<String> {
    let millimetres = units_per_uv? * 1000.0;
    if !(millimetres > 0.0) || !millimetres.is_finite() {
        return None;
    }
    let per_pixel = millimetres / size.max(1) as f32;
    Some(format!("Square: {} mm on the part \u{b7} {} mm a pixel", round(millimetres), round(per_pixel)))
}

/// Three figures, no more decimals than they need.
fn round(value: f32) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}

/// The layout of set `set` as a PNG, drawn from its SVG.
pub fn layout_png(mesh: &Mesh, set: usize, picture: &LayoutPicture) -> Result<Vec<u8>, String> {
    let svg = layout_svg(mesh, set, picture).map_err(|e| e.to_string())?;
    let size = picture.size.clamp(16, MOST_PIXELS);
    tequila_texture::svg::render_png(svg.as_bytes(), size, size)
}

/// A colour of its own for each island: hues a golden turn apart, so
/// neighbours by number are far apart in colour.
fn island_colour(i: usize) -> String {
    let hue = (i as f32 * 0.618_034).fract();
    let (r, g, b) = hsv(hue, 0.55, 0.92);
    hex([r, g, b])
}

fn hsv(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let byte = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;

    #[test]
    fn the_svg_has_every_island_seam_and_the_size_note() {
        let mesh = unwrapped(cube());
        let svg = layout_svg(&mesh, 0, &LayoutPicture { size: 512, ..Default::default() }).unwrap();
        assert!(svg.starts_with("<svg"));
        assert_eq!(svg.matches("<path fill=").count(), 6, "an island each");
        assert!(svg.contains(r#"id="seams""#));
        assert!(svg.contains("mm on the part"), "an unwrap is as dense all over");
        let heat = layout_svg(&mesh, 0, &LayoutPicture { fill: Fill::Distortion(Measure::Angle), ..Default::default() }).unwrap();
        assert!(heat.contains(r#"id="distortion""#));
        assert!(layout_svg(&mesh, 3, &LayoutPicture::default()).is_err());
    }

    #[test]
    fn the_png_is_the_size_asked_for() {
        let mesh = unwrapped(cube());
        let png = layout_png(&mesh, 0, &LayoutPicture { size: 256, note: false, ..Default::default() }).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // The header's width and height, big-endian after the IHDR tag.
        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((width, height), (256, 256));
    }

    /// Lines in a colour of their own, and no white square under them: a
    /// layout to lay over artwork.
    #[test]
    fn the_lines_take_a_colour_and_the_square_can_be_clear() {
        let mesh = unwrapped(cube());
        let picture = LayoutPicture { size: 256, line: Some([0x20, 0x60, 0xc0]), transparent: true, note: false, ..Default::default() };
        let svg = layout_svg(&mesh, 0, &picture).unwrap();
        assert!(!svg.contains(r##"fill="#ffffff""##), "no white square");
        assert!(svg.contains(r##"id="boundary" fill="none" stroke="#2060c0""##) || !svg.contains(r#"id="boundary""#));
        assert!(svg.contains(r##"id="edges" fill="none" stroke="#2060c0""##));
        let path = std::env::temp_dir().join(format!("tequila-clear-layout-{}.png", std::process::id()));
        std::fs::write(&path, layout_png(&mesh, 0, &picture).unwrap()).unwrap();
        let image = tequila_texture::image::Image::load_raw(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(image.channels, 4);
        let clear = image.data.chunks(4).filter(|p| p[3] == 0.0).count();
        assert!(clear > image.width * image.height / 10, "the square is clear between the islands: {clear}");
    }

    #[test]
    fn the_note_says_the_size_in_millimetres() {
        assert_eq!(size_note(Some(0.25), 1000).as_deref(), Some("Square: 250 mm on the part \u{b7} 0.250 mm a pixel"));
        assert_eq!(size_note(None, 1000), None);
    }
}
