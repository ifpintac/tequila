//! A seam said by where it goes: the edges of the part along the shortest
//! way, by its own edges, from one point of it to the next.
//!
//! `uv.cut` names an edge by the mesh vertices at its ends, which is what
//! the editor has when a person clicks one, and nothing a script has: it
//! knows the part's size and where its faces are, not how its mesh is
//! numbered. A path of points on the part is what it can say - down the
//! back of a bottle, from the top of its wall to the bottom - and this
//! finds the edges that follow it.

use super::edit::{layout_of, UvError};
use super::MeshEdge;
use crate::geometry::Mesh;
use std::collections::BinaryHeap;
use tequila_core::math::Vec3;

/// The edges a path follows, and how well the path was met.
#[derive(Clone, Debug, PartialEq)]
pub struct PathEdges {
    /// Each edge once, by the mesh vertices at its ends.
    pub edges: Vec<MeshEdge>,
    /// The path's length along the part, in its own space.
    pub length: f32,
    /// How far the point given furthest from any vertex was from the
    /// vertex it was taken to mean.
    pub furthest: f32,
}

/// Why no path was found.
#[derive(Clone, Debug, PartialEq)]
pub enum PathError {
    Uv(UvError),
    /// A path is two points or more.
    TooFewPoints,
    /// Two points that follow each other are on pieces of the part no edge
    /// joins, or on one vertex: the second's place in the list.
    NoWay(usize),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Uv(error) => write!(f, "{error}"),
            PathError::TooFewPoints => write!(f, "a path is two points or more"),
            PathError::NoWay(index) => write!(f, "points {} and {} of the path are the same place on the part, or on pieces of it that do not touch", index, index + 1),
        }
    }
}

/// A place in the queue: the nearer first.
#[derive(Clone, Copy, PartialEq)]
struct Reached {
    distance: f32,
    vertex: u32,
}

impl Eq for Reached {}

impl Ord for Reached {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.distance.total_cmp(&self.distance).then_with(|| other.vertex.cmp(&self.vertex))
    }
}

impl PartialOrd for Reached {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The edges of `mesh` along the shortest way, by its edges, through
/// `points`, each a place on the part in its own space and taken to mean
/// the vertex nearest it. The layout of `set` says where the islands end:
/// with `across`, a path that stops a little short of an island's edge -
/// in the rounding of a corner, a row of triangles from the rim - is
/// carried on to it at each end, since a cut that does not reach the edge
/// opens a slit and divides nothing. Only a little: an end more than a
/// quarter of the path's length from any edge is left where it was put.
pub fn edges_along(mesh: &Mesh, set: usize, points: &[Vec3], across: bool) -> Result<PathEdges, PathError> {
    let layout = layout_of(mesh, set).map_err(PathError::Uv)?;
    if points.len() < 2 {
        return Err(PathError::TooFewPoints);
    }
    // The welded vertices, each where one of its mesh vertices is, and the
    // edges out of each.
    let welded = layout.weld.iter().map(|w| *w as usize + 1).max().unwrap_or(0);
    let mut place = vec![Vec3::ZERO; welded];
    for (vertex, weld) in layout.weld.iter().enumerate() {
        place[*weld as usize] = mesh.positions[vertex];
    }
    let mut used = vec![false; welded];
    let mut ways: Vec<Vec<(u32, MeshEdge)>> = vec![Vec::new(); welded];
    for group in layout.table.groups() {
        let side = layout.table.sides[group.start];
        ways[side.lo as usize].push((side.hi, side.vertices));
        ways[side.hi as usize].push((side.lo, side.vertices));
        used[side.lo as usize] = true;
        used[side.hi as usize] = true;
    }
    // Where the layout is cut already, or the part ends.
    let mut border = vec![false; welded];
    for edge in &layout.edges {
        if edge.kind != super::EdgeKind::Inner {
            for vertex in edge.vertices {
                border[layout.weld[vertex as usize] as usize] = true;
            }
        }
    }
    let mut furthest = 0.0f32;
    let stops: Vec<u32> = points
        .iter()
        .map(|point| {
            let (nearest, distance) = (0..welded)
                .filter(|w| used[*w])
                .map(|w| (w as u32, (place[w] - *point).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap_or((0, 0.0));
            furthest = furthest.max(distance);
            nearest
        })
        .collect();
    // The shortest way from a vertex to the first that `arrived` says is
    // one to stop at: how far, where, and the edges on the way.
    let shortest = |from: u32, arrived: &dyn Fn(u32) -> bool| -> Option<(f32, u32, Vec<MeshEdge>)> {
        let mut best = vec![f32::INFINITY; welded];
        let mut came: Vec<Option<(u32, MeshEdge)>> = vec![None; welded];
        let mut queue = BinaryHeap::new();
        best[from as usize] = 0.0;
        queue.push(Reached { distance: 0.0, vertex: from });
        while let Some(Reached { distance, vertex }) = queue.pop() {
            if distance > best[vertex as usize] {
                continue;
            }
            if arrived(vertex) {
                let mut edges = Vec::new();
                let mut at = vertex;
                while let Some((before, edge)) = came[at as usize] {
                    edges.push(edge);
                    at = before;
                }
                return Some((distance, vertex, edges));
            }
            for &(next, edge) in &ways[vertex as usize] {
                let reached = distance + (place[next as usize] - place[vertex as usize]).length();
                if reached < best[next as usize] {
                    best[next as usize] = reached;
                    came[next as usize] = Some((vertex, edge));
                    queue.push(Reached { distance: reached, vertex: next });
                }
            }
        }
        None
    };
    let mut edges: Vec<MeshEdge> = Vec::new();
    let mut length = 0.0;
    let mut add = |found: Vec<MeshEdge>, edges: &mut Vec<MeshEdge>| {
        for edge in found {
            if !edges.contains(&edge) {
                edges.push(edge);
            }
        }
    };
    for (index, pair) in stops.windows(2).enumerate() {
        let (from, to) = (pair[0], pair[1]);
        if from == to {
            return Err(PathError::NoWay(index + 1));
        }
        let (distance, _, found) = shortest(from, &|vertex| vertex == to).ok_or(PathError::NoWay(index + 1))?;
        length += distance;
        add(found, &mut edges);
    }
    if across {
        let reach = length * 0.25;
        for end in [stops[0], stops[stops.len() - 1]] {
            if let Some((distance, _, found)) = shortest(end, &|vertex| border[vertex as usize]) {
                if distance > 0.0 && distance <= reach {
                    length += distance;
                    add(found, &mut edges);
                }
            }
        }
    }
    Ok(PathEdges { edges, length, furthest })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::{sheet, unwrapped};
    use crate::uv::{cut, CutOptions, UvLayout};
    use tequila_core::math::vec3;

    /// A flat sheet unwrapped is one island; a path across its middle,
    /// said by its two ends, cuts it in two along the row of edges there.
    #[test]
    fn a_path_across_a_sheet_cuts_it_in_two() {
        let sheet = unwrapped(sheet(8, 8, |_, _| 0.0));
        assert_eq!(UvLayout::new(&sheet, 0).unwrap().islands.len(), 1);
        // Not quite on the vertices at either end: the nearest are meant.
        let path = edges_along(&sheet, 0, &[vec3(-0.01, 0.0, 0.51), vec3(1.02, 0.0, 0.49)], false).expect("there is a way across");
        assert_eq!(path.edges.len(), 8, "one edge a column");
        assert!((path.length - 1.0).abs() < 1e-4, "{} long across a sheet 1 wide", path.length);
        assert!(path.furthest < 0.03, "{}", path.furthest);
        let halves = cut(&sheet, 0, &path.edges, &CutOptions::default()).expect("the path cuts");
        assert_eq!(UvLayout::new(&halves, 0).unwrap().islands.len(), 2);
    }

    /// Through a point on the way, the path bends there.
    #[test]
    fn a_path_goes_through_every_point_in_turn() {
        let sheet = unwrapped(sheet(4, 4, |_, _| 0.0));
        let bent = edges_along(&sheet, 0, &[vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0)], false).unwrap();
        assert_eq!(bent.edges.len(), 8);
        assert!((bent.length - 2.0).abs() < 1e-4);
    }

    /// A path that stops a row short of the sheet's edge at either end is
    /// carried on to it, and divides the sheet; asked not to, it opens a
    /// slit and the sheet stays one piece. An end far from any edge stays
    /// where it was put.
    #[test]
    fn a_path_a_little_short_of_the_edge_is_carried_on_to_it() {
        let sheet = unwrapped(sheet(8, 8, |_, _| 0.0));
        let short = [vec3(0.125, 0.0, 0.5), vec3(0.875, 0.0, 0.5)];
        let slit = edges_along(&sheet, 0, &short, false).unwrap();
        assert_eq!(slit.edges.len(), 6);
        assert_eq!(UvLayout::new(&cut(&sheet, 0, &slit.edges, &CutOptions::default()).unwrap(), 0).unwrap().islands.len(), 1, "a slit divides nothing");
        let whole = edges_along(&sheet, 0, &short, true).unwrap();
        assert_eq!(whole.edges.len(), 8);
        assert!((whole.length - 1.0).abs() < 1e-4);
        assert_eq!(UvLayout::new(&cut(&sheet, 0, &whole.edges, &CutOptions::default()).unwrap(), 0).unwrap().islands.len(), 2);
        // In the middle of the sheet, three rows from the nearest edge.
        let inside = edges_along(&sheet, 0, &[vec3(0.375, 0.0, 0.5), vec3(0.625, 0.0, 0.5)], true).unwrap();
        assert_eq!(inside.edges.len(), 2, "an end far from the edge is left");
    }

    #[test]
    fn a_path_of_one_place_is_refused() {
        let flat = unwrapped(sheet(4, 4, |_, _| 0.0));
        let corner = flat.positions[0];
        assert_eq!(edges_along(&flat, 0, &[corner], true), Err(PathError::TooFewPoints));
        assert_eq!(edges_along(&flat, 0, &[corner, corner], true), Err(PathError::NoWay(1)));
        let bare = sheet(4, 4, |_, _| 0.0);
        assert!(matches!(edges_along(&bare, 0, &[corner, vec3(1.0, 0.0, 1.0)], true), Err(PathError::Uv(_))));
    }
}
