//! Weld nearby: picked vertices of the layout that lie within a distance of
//! each other moved together, to the middle of each group. Where the copies
//! of one vertex of the part come together on a seam they are one place
//! again, and the layout joins there as a stitch joins it; where every end
//! of a seam meets, its islands are one.
//!
//! Vertices of one triangle are never welded to each other: that would
//! squash the triangle to nothing, which is no join. A group they would be
//! in is left as it is, and counted.

use super::layout::UnionFind;
use super::shape::{cannot, edge_ends, layout, Moves, Pick, ShapeError};
use crate::geometry::Mesh;
use std::collections::{HashMap, HashSet};
use tequila_core::math::Vec2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeldReport {
    /// Vertices moved.
    pub welded: usize,
    /// The places they were moved to.
    pub places: usize,
    /// Places where copies of one vertex of the part came together.
    pub joined: usize,
    /// Groups left alone, as they hold two corners of one triangle.
    pub skipped: usize,
}

/// The vertices `pick` names - picked vertices, the ends of picked edges,
/// or every vertex of the picked islands - welded where they are within
/// `radius` of each other.
pub fn weld_nearby(mesh: &Mesh, set: usize, pick: &Pick, radius: f32) -> Result<(Moves, WeldReport), ShapeError> {
    let layout = layout(mesh, set)?;
    if !(radius > 0.0 && radius.is_finite()) {
        return cannot("give a distance to weld within");
    }
    let mut chosen: Vec<u32> = if !pick.vertices.is_empty() {
        layout.uv_vertices_of(&pick.vertices)
    } else if !pick.edges.is_empty() {
        let mut out = Vec::new();
        for &edge in &pick.edges {
            out.extend(edge_ends(&layout, edge)?);
        }
        out
    } else if !pick.islands.is_empty() || !pick.faces.is_empty() {
        let mut islands = super::edit::chosen_islands(&layout, &pick.islands)?;
        islands.extend(layout.islands_of_faces(&pick.faces));
        islands.iter().flat_map(|&i| layout.islands[i as usize].uv_vertices.iter().copied()).collect()
    } else {
        return cannot("pick the vertices to weld, or the islands whose vertices to weld");
    };
    chosen.sort_unstable();
    chosen.dedup();
    let index: HashMap<u32, usize> = chosen.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    // Corners of one triangle, which are never welded together.
    let mut sharing: HashSet<(u32, u32)> = HashSet::new();
    for t in 0..layout.island_of.len() {
        let corners = [0, 1, 2].map(|k| layout.corner_vertex[t * 3 + k]);
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            if index.contains_key(&a) && index.contains_key(&b) {
                sharing.insert((a.min(b), a.max(b)));
            }
        }
    }
    let place = |v: u32| layout.uv_vertices[v as usize].uv;
    let cell = |p: Vec2| ((p.x / radius).floor() as i64, (p.y / radius).floor() as i64);
    let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, &v) in chosen.iter().enumerate() {
        cells.entry(cell(place(v))).or_default().push(i);
    }
    let mut groups = UnionFind::new(chosen.len());
    for (i, &v) in chosen.iter().enumerate() {
        let (cx, cy) = cell(place(v));
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &j in cells.get(&(cx + dx, cy + dy)).map(|l| l.as_slice()).unwrap_or(&[]) {
                    let w = chosen[j];
                    if j > i && (place(w) - place(v)).length() <= radius && !sharing.contains(&(v.min(w), v.max(w))) {
                        groups.union(i, j);
                    }
                }
            }
        }
    }
    let mut members: HashMap<usize, Vec<u32>> = HashMap::new();
    for (i, &v) in chosen.iter().enumerate() {
        members.entry(groups.find(i)).or_default().push(v);
    }
    let mut report = WeldReport::default();
    let mut moves = Moves::new();
    let mut list: Vec<Vec<u32>> = members.into_values().filter(|m| m.len() > 1).collect();
    list.sort();
    for group in list {
        let squashes = group.iter().enumerate().any(|(k, &a)| group[k + 1..].iter().any(|&b| sharing.contains(&(a.min(b), a.max(b)))));
        if squashes {
            report.skipped += 1;
            continue;
        }
        let middle = group.iter().fold(Vec2::ZERO, |s, &v| s + place(v)) / group.len() as f32;
        let mut welds: Vec<u32> = group.iter().map(|&v| layout.uv_vertices[v as usize].weld).collect();
        welds.sort_unstable();
        if welds.windows(2).any(|w| w[0] == w[1]) {
            report.joined += 1;
        }
        report.welded += group.len();
        report.places += 1;
        moves.extend(group.iter().map(|&v| (v, middle)));
    }
    if moves.is_empty() {
        return cannot(if report.skipped > 0 { "the vertices that close share a face: welding them would squash it" } else { "no two of the picked vertices are that close" });
    }
    Ok((moves, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{cut, set_positions, CutOptions, UvLayout};

    /// A sheet cut in two, its halves a little apart: welded within the
    /// gap, the seam's copies meet and the halves are one island again.
    #[test]
    fn halves_a_gap_apart_weld_back_into_one() {
        let mesh = unwrapped(sheet(2, 4, |_, _| 0.0));
        let edges: Vec<[u32; 2]> = (0..mesh.triangle_count())
            .flat_map(|t| {
                let tri = mesh.triangle(t);
                (0..3).map(move |k| [tri[k], tri[(k + 1) % 3]])
            })
            .filter(|[a, b]| (mesh.positions[*a as usize].x - 0.5).abs() < 1e-5 && (mesh.positions[*b as usize].x - 0.5).abs() < 1e-5 && a < b)
            .collect();
        let parted = cut(&mesh, 0, &edges, &CutOptions { reflatten: false, gap: 0.004 }).unwrap();
        assert_eq!(UvLayout::new(&parted, 0).unwrap().islands.len(), 2);
        let (moves, report) = weld_nearby(&parted, 0, &Pick::islands(vec![0, 1]), 0.006).unwrap();
        assert_eq!((report.places, report.joined, report.welded), (3, 3, 6), "{report:?}");
        let out = set_positions(&parted, 0, &moves).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        assert_eq!(layout.islands.len(), 1, "one island again");
        assert!(layout.folded_triangles().is_empty());
        // Too small a distance welds nothing, and says so.
        assert!(weld_nearby(&parted, 0, &Pick::islands(vec![0, 1]), 0.001).is_err());
    }
}
