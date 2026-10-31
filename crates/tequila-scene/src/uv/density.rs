//! How fine the texture is on the part, and how much of the texture the
//! part uses: texel density, and the layout's coverage of its square.
//!
//! A density here is a length: layout units per metre of the part, the
//! square root of area on the layout over area on the part. In pixels per
//! centimetre at a texture's size it is that, times the size, over a
//! hundred - the part's own metres, as the editor's rulers read them.

use super::layout::UvLayout;
use crate::geometry::Mesh;

/// Layout units per metre of the part over some of its triangles; none
/// where they have no area on the part or none on the layout.
pub fn of_triangles(layout: &UvLayout, mesh: &Mesh, triangles: &[u32]) -> Option<f32> {
    let (mut flat, mut surface) = (0.0f64, 0.0f64);
    for &t in triangles {
        if t as usize >= layout.island_of.len() {
            continue;
        }
        let [a, b, c] = layout.triangle_uv(t);
        flat += super::flatten::facing(a, b, c).abs() as f64;
        let [p, q, r] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
        surface += super::flatten::surface_area(p, q, r) as f64;
    }
    (surface > 0.0 && flat > 0.0).then(|| (flat / surface).sqrt() as f32)
}

/// A density in pixels per centimetre on a texture `size` pixels a side.
pub fn pixels_per_cm(density: f32, size: u32) -> f32 {
    density * size as f32 / 100.0
}

/// The density that is `pixels` per centimetre on a texture `size` a side.
pub fn from_pixels_per_cm(pixels: f32, size: u32) -> f32 {
    pixels * 100.0 / size.max(1) as f32
}

/// How much of the layout's square the islands cover, overlaps counted
/// once, and how much of it more than one island covers: each a share of
/// the square, 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coverage {
    pub covered: f32,
    pub overlap: f32,
}

/// The coverage, found on a grid of `resolution` cells a side over the
/// square: a cell is covered where its middle is inside a triangle. A
/// thousand a side is within a tenth of a per cent.
pub fn coverage(layout: &UvLayout, resolution: usize) -> Coverage {
    let n = resolution.clamp(16, 8192);
    let mut count = vec![0u8; n * n];
    let mut stamp = vec![u32::MAX; n * n];
    let cell = 1.0 / n as f32;
    for (i, island) in layout.islands.iter().enumerate() {
        let i = i as u32;
        for &t in &island.triangles {
            let [a, b, c] = layout.triangle_uv(t);
            let (low, high) = (a.min(b).min(c), a.max(b).max(c));
            if !(low.is_finite() && high.is_finite()) || high.x < 0.0 || high.y < 0.0 || low.x > 1.0 || low.y > 1.0 {
                continue;
            }
            let span = |lo: f32, hi: f32| (((lo / cell) - 0.5).ceil().max(0.0) as usize, (((hi / cell) - 0.5).floor()).min(n as f32 - 1.0));
            let (x0, x1) = span(low.x, high.x);
            let (y0, y1) = span(low.y, high.y);
            if x1 < 0.0 || y1 < 0.0 {
                continue;
            }
            let area = super::flatten::facing(a, b, c);
            if area == 0.0 {
                continue;
            }
            for y in y0..=y1 as usize {
                for x in x0..=x1 as usize {
                    let p = tequila_core::math::vec2((x as f32 + 0.5) * cell, (y as f32 + 0.5) * cell);
                    let w = [super::flatten::facing(p, b, c), super::flatten::facing(a, p, c), super::flatten::facing(a, b, p)];
                    let inside = if area > 0.0 { w.iter().all(|&w| w >= 0.0) } else { w.iter().all(|&w| w <= 0.0) };
                    if inside {
                        let k = y * n + x;
                        if stamp[k] != i {
                            stamp[k] = i;
                            count[k] = count[k].saturating_add(1);
                        }
                    }
                }
            }
        }
    }
    let total = (n * n) as f32;
    Coverage { covered: count.iter().filter(|&&c| c >= 1).count() as f32 / total, overlap: count.iter().filter(|&&c| c >= 2).count() as f32 / total }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    /// Two squares of a part, 10 cm a side, laid out as squares of the
    /// layout `side` across, the second at `second`.
    fn two(side: f32, second: tequila_core::math::Vec2) -> Mesh {
        let positions = vec![
            vec3(0.0, 0.0, 0.0), vec3(0.1, 0.0, 0.0), vec3(0.1, 0.1, 0.0), vec3(0.0, 0.1, 0.0),
            vec3(0.2, 0.0, 0.0), vec3(0.3, 0.0, 0.0), vec3(0.3, 0.1, 0.0), vec3(0.2, 0.1, 0.0),
        ];
        let square = |at: tequila_core::math::Vec2| [at, at + vec2(side, 0.0), at + vec2(side, side), at + vec2(0.0, side)];
        let uv = square(vec2(0.0, 0.0)).into_iter().chain(square(second)).collect();
        Mesh::new(positions, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]).with_uvs(uv)
    }

    #[test]
    fn a_density_reads_in_pixels_per_centimetre() {
        let mesh = two(0.5, vec2(0.5, 0.5));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // 10 cm of the part is half the layout: 5 layout units a metre.
        let density = of_triangles(&layout, &mesh, &[0, 1]).unwrap();
        assert!((density - 5.0).abs() < 1e-4);
        // On a 2K texture, half is 1024 pixels over 10 cm.
        assert!((pixels_per_cm(density, 2048) - 102.4).abs() < 1e-2);
        assert!((from_pixels_per_cm(102.4, 2048) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn coverage_counts_overlaps_once_and_says_how_much_overlaps() {
        let apart = coverage(&UvLayout::new(&two(0.5, vec2(0.5, 0.5)), 0).unwrap(), 512);
        assert!((apart.covered - 0.5).abs() < 0.01 && apart.overlap < 0.01, "{apart:?}");
        // The second over a quarter of the first.
        let over = coverage(&UvLayout::new(&two(0.5, vec2(0.25, 0.25)), 0).unwrap(), 512);
        assert!((over.covered - (0.5 - 0.0625)).abs() < 0.01, "{over:?}");
        assert!((over.overlap - 0.0625).abs() < 0.01, "{over:?}");
        // Off the square counts for nothing.
        let off = coverage(&UvLayout::new(&two(0.5, vec2(1.5, 0.0)), 0).unwrap(), 512);
        assert!((off.covered - 0.25).abs() < 0.01);
    }
}
