//! How far a layout stretches the surface it lays out, triangle by
//! triangle: as data for a heatmap.
//!
//! Two measures, both 1 where the layout is perfect:
//!
//! - **Area**: the triangle's share of the layout against its share of the
//!   part - its texel density over the whole layout's. 2 is twice the
//!   texels it should have, 0.5 half.
//! - **Angle**: how much the layout squashes the triangle one way more than
//!   the other, the ratio of its stretch along the two directions it
//!   stretches most and least: 1 keeps its angles, 2 makes a circle on the
//!   part an ellipse twice as long as it is wide on the layout.

use super::edit::{layout_of, UvError};
use super::flatten::{facing, surface_area};
use crate::geometry::Mesh;

/// One triangle's distortion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Distortion {
    /// Its density over the layout's: 1 even.
    pub area: f32,
    /// Its stretch one way over the other, 1 or more: 1 conformal.
    pub angle: f32,
}

/// Which measure a heatmap shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Measure {
    #[default]
    Area,
    Angle,
}

/// Every triangle's distortion, in order. A triangle with no area on the
/// part reads as undistorted; one with area on the part and none on the
/// layout reads as squashed to nothing, its angle infinite.
pub fn distortion(mesh: &Mesh, set: usize) -> Result<Vec<Distortion>, UvError> {
    let layout = layout_of(mesh, set)?;
    let count = mesh.triangle_count();
    let mut flats = Vec::with_capacity(count);
    let mut surfaces = Vec::with_capacity(count);
    for t in 0..count {
        let [a, b, c] = layout.triangle_uv(t as u32);
        let [pa, pb, pc] = mesh.triangle(t).map(|v| mesh.positions[v as usize]);
        flats.push(facing(a, b, c).abs() as f64);
        surfaces.push(surface_area(pa, pb, pc) as f64);
    }
    let (flat, surface): (f64, f64) = (flats.iter().sum(), surfaces.iter().sum());
    let overall = if surface > 0.0 { flat / surface } else { 0.0 };
    let mut out = Vec::with_capacity(count);
    for t in 0..count {
        if surfaces[t] <= 0.0 || overall <= 0.0 {
            out.push(Distortion { area: 1.0, angle: 1.0 });
            continue;
        }
        let area = (flats[t] / surfaces[t] / overall) as f32;
        let [a, b, c] = layout.triangle_uv(t as u32);
        let [pa, pb, pc] = mesh.triangle(t).map(|v| mesh.positions[v as usize]);
        // The triangle in its own plane, x along its first edge.
        let (e1, e2) = (pb - pa, pc - pa);
        let x = e1.normalize();
        let y = e1.cross(e2).normalize().cross(x);
        let (p11, p12, p22) = (e1.length() as f64, e2.dot(x) as f64, e2.dot(y) as f64);
        // J = Q P^-1, P the part's edges in that plane, Q the layout's.
        let (q11, q21) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
        let (q12, q22) = ((c.x - a.x) as f64, (c.y - a.y) as f64);
        let det_p = p11 * p22;
        if det_p.abs() <= 1e-30 {
            out.push(Distortion { area, angle: 1.0 });
            continue;
        }
        // P^-1 = [[p22, -p12], [0, p11]] / det
        let j11 = (q11 * p22) / det_p;
        let j12 = (q12 * p11 - q11 * p12) / det_p;
        let j21 = (q21 * p22) / det_p;
        let j22 = (q22 * p11 - q21 * p12) / det_p;
        let energy = j11 * j11 + j12 * j12 + j21 * j21 + j22 * j22;
        let det = (j11 * j22 - j12 * j21).abs();
        let (plus, minus) = ((energy + 2.0 * det).max(0.0).sqrt(), (energy - 2.0 * det).max(0.0).sqrt());
        let (most, least) = ((plus + minus) * 0.5, (plus - minus) * 0.5);
        let angle = if least > 0.0 { (most / least) as f32 } else { f32::INFINITY };
        out.push(Distortion { area, angle: angle.max(1.0) });
    }
    Ok(out)
}

/// A heatmap's colour for a measure, sRGB: white where the layout is
/// perfect; for area, blue where the texture is coarser than the rest and
/// red where finer, full at a factor of four; for angle, yellow toward red,
/// full at twice as long one way as the other.
pub fn heat(distortion: &Distortion, measure: Measure) -> [u8; 3] {
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [u8; 3] {
        let t = t.clamp(0.0, 1.0);
        [0, 1, 2].map(|i| ((a[i] + (b[i] - a[i]) * t) * 255.0).round() as u8)
    };
    let white = [1.0, 1.0, 1.0];
    match measure {
        Measure::Area => {
            let a = distortion.area;
            if !(a > 0.0) || !a.is_finite() {
                return [40, 40, 160];
            }
            let log = a.log2() / 2.0;
            if log >= 0.0 { lerp(white, [0.85, 0.12, 0.10], log) } else { lerp(white, [0.12, 0.30, 0.85], -log) }
        }
        Measure::Angle => {
            let a = distortion.angle;
            if !a.is_finite() {
                return [140, 10, 10];
            }
            let t = (a - 1.0).clamp(0.0, 1.0);
            if t < 0.5 { lerp(white, [0.98, 0.80, 0.15], t * 2.0) } else { lerp([0.98, 0.80, 0.15], [0.85, 0.12, 0.10], (t - 0.5) * 2.0) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{transform, Pivot, UvAffine, UvSelection};
    use tequila_core::math::vec2;

    #[test]
    fn a_flat_unwrap_is_undistorted_and_a_squash_shows() {
        let mesh = unwrapped(sheet(4, 4, |_, _| 0.0));
        for d in distortion(&mesh, 0).unwrap() {
            assert!((d.area - 1.0).abs() < 1e-3 && (d.angle - 1.0).abs() < 1e-3, "{d:?}");
        }
        let squashed = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::scale(vec2(1.0, 0.5)), Pivot::Islands).unwrap();
        for d in distortion(&squashed, 0).unwrap() {
            assert!((d.angle - 2.0).abs() < 1e-3, "twice as long one way: {d:?}");
            assert!((d.area - 1.0).abs() < 1e-3, "one island, even on itself");
        }
    }

    #[test]
    fn a_larger_island_shows_as_denser() {
        let mesh = unwrapped(cube());
        let big = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::uniform(2.0), Pivot::Islands).unwrap();
        let layout = crate::uv::UvLayout::new(&big, 0).unwrap();
        let all = distortion(&big, 0).unwrap();
        let on_big = all[layout.islands[0].triangles[0] as usize];
        let on_other = all[layout.islands[1].triangles[0] as usize];
        assert!((on_big.area / on_other.area - 4.0).abs() < 1e-3);
        assert!((on_big.angle - 1.0).abs() < 1e-3, "scaled evenly, no angle lost");
        assert_eq!(heat(&Distortion { area: 1.0, angle: 1.0 }, Measure::Area), [255, 255, 255]);
        assert_ne!(heat(&on_big, Measure::Area), heat(&on_other, Measure::Area));
    }
}
