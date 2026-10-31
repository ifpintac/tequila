//! What the editor's Select menu finds on a layout: the islands of the same
//! shape as some, their border edges, the edges square to the layout, the
//! faces with no area on it, and the faces turned over against their
//! island. Each a list of the layout's own numbers - islands, edges,
//! triangles - for the editor to select.

use super::layout::{EdgeKind, UvLayout};
use super::similar::{similar, IslandShape};
use crate::geometry::Mesh;

/// The islands of the same piece and shape as any of `islands`, within
/// `tolerance` ([`super::similar`]); `islands` themselves among them.
pub fn similar_islands(layout: &UvLayout, mesh: &Mesh, islands: &[u32], tolerance: f32, mirrored: bool) -> Vec<u32> {
    let wanted: Vec<IslandShape> = islands.iter().filter(|&&i| (i as usize) < layout.islands.len()).map(|&i| IslandShape::of(layout, mesh, i)).collect();
    let counts: Vec<(usize, usize, usize)> = wanted.iter().map(IslandShape::counts).collect();
    let mut out: Vec<u32> = islands.to_vec();
    for i in 0..layout.islands.len() as u32 {
        if islands.contains(&i) {
            continue;
        }
        // The triangle count first: it rules most out without a shape.
        let triangles = layout.islands[i as usize].triangles.len();
        if !counts.iter().any(|c| c.0 == triangles) {
            continue;
        }
        let shape = IslandShape::of(layout, mesh, i);
        let own = shape.counts();
        if wanted.iter().zip(&counts).any(|(w, c)| *c == own && similar(w, &shape, tolerance, mirrored).is_some()) {
            out.push(i);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The edges round some islands - seams and the part's own edge - or round
/// every island where none are given.
pub fn border_edges(layout: &UvLayout, islands: &[u32]) -> Vec<u32> {
    let all: Vec<u32>;
    let islands = if islands.is_empty() {
        all = (0..layout.islands.len() as u32).collect();
        &all
    } else {
        islands
    };
    let mut out = Vec::new();
    for &i in islands {
        let Some(island) = layout.islands.get(i as usize) else { continue };
        out.extend(island.edges.clone().filter(|&e| layout.edges[e].kind != EdgeKind::Inner).map(|e| e as u32));
    }
    out
}

/// Which way a square edge runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Square {
    /// Across the layout, along u.
    Across,
    /// Down it, along v.
    Down,
    Either,
}

/// Of `edges`, those within `tolerance` radians of running across the
/// layout, or down it: a strip's lines, not the long diagonals of its
/// quads, which a long thin quad lays nearly along them too.
pub fn square_edges(layout: &UvLayout, mesh: &Mesh, edges: &[u32], way: Square, tolerance: f32) -> Vec<u32> {
    edges
        .iter()
        .copied()
        .filter(|&e| {
            let Some(edge) = layout.edges.get(e as usize) else { return false };
            let [a, b] = edge.ends.map(|v| layout.uv_vertices[v as usize].uv);
            let d = b - a;
            if d.length() <= 0.0 || super::vertices::is_diagonal(layout, mesh, e) {
                return false;
            }
            let angle = d.y.abs().atan2(d.x.abs());
            let across = angle <= tolerance;
            let down = angle >= std::f32::consts::FRAC_PI_2 - tolerance;
            match way {
                Square::Across => across,
                Square::Down => down,
                Square::Either => across || down,
            }
        })
        .collect()
}

/// The faces with no area on the layout, or next to none: squashed to less
/// than a ten-thousandth of what their island gives a face of their size on
/// the part, or to nothing at all. Artwork cannot land on them.
pub fn no_area_faces(layout: &UvLayout, mesh: &Mesh) -> Vec<u32> {
    let mut out = Vec::new();
    for island in &layout.islands {
        let density = island.density();
        for &t in &island.triangles {
            let [a, b, c] = layout.triangle_uv(t);
            let flat = super::flatten::facing(a, b, c).abs();
            let [p, q, r] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
            let surface = super::flatten::surface_area(p, q, r);
            if flat <= 1e-14 || flat <= density * surface * 1e-4 {
                out.push(t);
            }
        }
    }
    out
}

/// The faces turned over against the rest of their island: where artwork
/// prints mirrored ([`UvLayout::folded_triangles`]).
pub fn flipped_faces(layout: &UvLayout) -> Vec<u32> {
    layout.folded_triangles()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use tequila_core::math::vec2;

    #[test]
    fn a_box_s_sides_are_alike_and_bordered_by_seams() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        assert_eq!(similar_islands(&layout, &mesh, &[2], 0.05, true), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(border_edges(&layout, &[2]).len(), 4, "four seams round a side");
        assert_eq!(border_edges(&layout, &[]).len(), 24);
    }

    #[test]
    fn square_edges_squashed_faces_and_folds_are_found() {
        let flat = sheet(2, 2, |_, _| 0.0);
        let uv = flat.positions.iter().map(|p| vec2(0.1 + p.x * 0.5, 0.1 + p.z * 0.5)).collect();
        let mut mesh = flat.with_uvs(uv);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.edges.len() as u32).collect();
        // Twelve sides of the four squares across and down; four diagonals.
        assert_eq!(square_edges(&layout, &mesh, &all, Square::Either, 0.1).len(), 12);
        assert_eq!(square_edges(&layout, &mesh, &all, Square::Across, 0.1).len(), 6);
        assert!(no_area_faces(&layout, &mesh).is_empty());
        assert!(flipped_faces(&layout).is_empty());
        // The middle vertex pushed out past a corner turns faces over; onto
        // the top edge's middle, two faces lose their area.
        let middle = (0..mesh.positions.len()).find(|&v| (mesh.positions[v].x - 0.5).abs() < 1e-6 && (mesh.positions[v].z - 0.5).abs() < 1e-6).unwrap();
        mesh.uv_sets[0][middle] = vec2(0.75, 0.75);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        assert!(!flipped_faces(&layout).is_empty());
        mesh.uv_sets[0][middle] = vec2(0.35, 0.1);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        assert_eq!(no_area_faces(&layout, &mesh).len(), 2, "the faces on the edge it now lies along");
    }
}
