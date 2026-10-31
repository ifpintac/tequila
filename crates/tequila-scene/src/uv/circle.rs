//! Circle: a closed loop of the layout laid on a perfect circle - picked
//! edges or vertices that close a loop, or the border of picked faces or of
//! an island - and the rest of its island laid out again round it.
//!
//! The circle is centred on the middle of the loop's area, as long round as
//! the loop was on the layout, its vertices spaced as their edges are long
//! on the part or evenly, starting where the loop's first vertex was and
//! running round the way the loop ran, so the inside does not turn over.

use super::shape::{border_loops, cannot, chain, island_round, layout, on_island, picked_chain, point, polygon_area, regions, Chain, Moves, Pick, ShapeError};
use super::layout::UvLayout;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CircleOptions {
    /// The vertices evenly round, rather than as their edges are long on
    /// the part.
    pub even: bool,
    /// The border of the picked faces or islands made the circle, even when
    /// edges or vertices are picked.
    pub whole_region: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CircleReport {
    pub circles: usize,
    pub vertices: usize,
}

/// The loop `pick` makes - its edges or vertices, or the border of its
/// faces or islands - laid on a circle; each island laid out again round
/// its circle with `pins` (mesh vertices) held too.
pub fn circle(mesh: &Mesh, set: usize, pick: &Pick, pins: &[u32], options: &CircleOptions) -> Result<(Moves, CircleReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let mut loops: Vec<(u32, Vec<u32>)> = Vec::new();
    if !options.whole_region && (!pick.edges.is_empty() || !pick.vertices.is_empty()) {
        let (island, pairs) = picked_chain(&layout, pick)?;
        match chain(&pairs) {
            Some(Chain::Loop(ring)) => loops.push((island, ring)),
            Some(Chain::Path(_)) => return cannot("the picked edges don't close a loop: Straight line lays a path out"),
            None => return cannot("the picked edges are not one loop: they branch, or are in pieces"),
        }
    } else {
        for region in regions(&layout, &Pick { vertices: Vec::new(), edges: Vec::new(), ..pick.clone() })? {
            let Some(outer) = border_loops(&layout, &region.triangles).into_iter().next() else { continue };
            loops.push((region.island, outer));
        }
    }
    if loops.is_empty() {
        return cannot("nothing picked has a border to lay round");
    }
    let mut report = CircleReport::default();
    let mut moves = Moves::new();
    for (island, ring) in loops {
        if ring.len() < 3 {
            return cannot("a loop needs three vertices at least");
        }
        let placed = round(mesh, &layout, &ring, options.even);
        let held = on_island(&layout, pins, island);
        moves.extend(island_round(mesh, &layout, island, &placed, &held, "round the circle")?);
        report.circles += 1;
        report.vertices += ring.len();
    }
    Ok((moves, report))
}

/// A loop of UV vertices on its circle.
fn round(mesh: &Mesh, layout: &UvLayout, ring: &[u32], even: bool) -> Vec<(u32, Vec2)> {
    let n = ring.len();
    let old: Vec<Vec2> = ring.iter().map(|&v| layout.uv_vertices[v as usize].uv).collect();
    let area = polygon_area(&old);
    // The middle of its area; of its vertices where it has none.
    let centre = if area.abs() > 1e-18 {
        let (mut cx, mut cy) = (0.0f64, 0.0f64);
        for k in 0..n {
            let (p, q) = (old[k], old[(k + 1) % n]);
            let cross = (q.x as f64) * (p.y as f64) - (p.x as f64) * (q.y as f64);
            cx += (p.x + q.x) as f64 * cross;
            cy += (p.y + q.y) as f64 * cross;
        }
        vec2((cx / (6.0 * area)) as f32, (cy / (6.0 * area)) as f32)
    } else {
        old.iter().fold(Vec2::ZERO, |s, p| s + *p) / n as f32
    };
    let perimeter: f32 = (0..n).map(|k| (old[(k + 1) % n] - old[k]).length()).sum();
    let radius = perimeter / std::f32::consts::TAU;
    let lengths: Vec<f32> = (0..n).map(|k| (point(mesh, layout, ring[(k + 1) % n]) - point(mesh, layout, ring[k])).length()).collect();
    let total: f32 = lengths.iter().sum();
    let mut fractions = Vec::with_capacity(n);
    let mut done = 0.0f32;
    for k in 0..n {
        fractions.push(if even || total <= 0.0 { k as f32 / n as f32 } else { done / total });
        done += lengths[k];
    }
    let start = (old[0] - centre).y.atan2((old[0] - centre).x);
    let lay = |way: f32| -> Vec<Vec2> {
        fractions.iter().map(|t| {
            let angle = start + way * t * std::f32::consts::TAU;
            centre + vec2(angle.cos(), angle.sin()) * radius
        }).collect()
    };
    let mut new = lay(1.0);
    if polygon_area(&new) * area < 0.0 {
        new = lay(-1.0);
    }
    ring.iter().copied().zip(new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, UvLayout};

    /// A flat square sheet's border made a circle: every border vertex as
    /// far from the middle, the inside laid out again within it.
    #[test]
    fn an_island_s_border_becomes_a_circle() {
        let mesh = unwrapped(sheet(6, 6, |_, _| 0.0));
        for even in [false, true] {
            let (moves, report) = circle(&mesh, 0, &Pick::default(), &[], &CircleOptions { even, whole_region: true }).unwrap();
            assert_eq!((report.circles, report.vertices), (1, 24));
            let out = set_positions(&mesh, 0, &moves).unwrap();
            let layout = UvLayout::new(&out, 0).unwrap();
            assert!(layout.folded_triangles().is_empty());
            let ring: Vec<Vec2> = layout.islands[0].boundary[0].iter().map(|&v| layout.uv_vertices[v as usize].uv).collect();
            let middle = ring.iter().fold(Vec2::ZERO, |s, p| s + *p) / ring.len() as f32;
            let radii: Vec<f32> = ring.iter().map(|p| (*p - middle).length()).collect();
            let (low, high) = radii.iter().fold((f32::INFINITY, 0.0f32), |(l, h), r| (l.min(*r), h.max(*r)));
            assert!(high - low < 1e-4 * high, "round: {low} to {high}");
        }
    }

    /// A cylinder's lid, cut off along its sharp rim, laid round: its border
    /// a circle as long round as it was.
    #[test]
    fn a_cylinder_s_lid_lies_round() {
        let mut mesh = tube(12, 2);
        let centre = mesh.positions.len() as u32;
        mesh.positions.push(tequila_core::math::vec3(0.0, 1.0, 0.0));
        for s in 0..12u32 {
            mesh.indices.extend_from_slice(&[centre, 2 * 13 + s + 1, 2 * 13 + s]);
        }
        let mesh = unwrapped(mesh);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let rim = crate::uv::angle_cut::sharp_edges(&mesh, 0, &[], 30.0).unwrap();
        let lid = if rim.is_empty() { mesh.clone() } else { crate::uv::cut(&mesh, 0, &rim, &crate::uv::CutOptions::default()).unwrap() };
        let layout = if rim.is_empty() { layout } else { UvLayout::new(&lid, 0).unwrap() };
        // The lid: the island of the fan's first triangle.
        let first_lid_face = (lid.triangle_count() - 12) as u32;
        let island = layout.island_of[first_lid_face as usize];
        let (moves, _) = circle(&lid, 0, &Pick::islands(vec![island]), &[], &CircleOptions::default()).unwrap();
        let out = set_positions(&lid, 0, &moves).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert!(after.folded_triangles().is_empty());
        let ring: Vec<Vec2> = after.islands[island as usize].boundary[0].iter().map(|&v| after.uv_vertices[v as usize].uv).collect();
        assert_eq!(ring.len(), 12);
        let middle = ring.iter().fold(Vec2::ZERO, |s, p| s + *p) / 12.0;
        let radii: Vec<f32> = ring.iter().map(|p| (*p - middle).length()).collect();
        assert!(radii.iter().all(|r| (r - radii[0]).abs() < 1e-4 * radii[0]), "{radii:?}");
    }

    /// A row of a sheet's vertices is a path, not a loop: refused.
    #[test]
    fn a_path_is_not_a_loop() {
        let mesh = unwrapped(sheet(3, 3, |_, _| 0.0));
        let row: Vec<u32> = (0..mesh.positions.len() as u32).filter(|&v| mesh.positions[v as usize].z.abs() < 1e-5).collect();
        let error = circle(&mesh, 0, &Pick::vertices(row), &[], &CircleOptions::default()).unwrap_err();
        assert!(matches!(&error, ShapeError::Cannot(why) if why.contains("loop")), "{error}");
    }
}
