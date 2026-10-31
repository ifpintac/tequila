//! Project from the faces' direction: faces laid flat as they are seen
//! straight on - down the average of their normals - the way a flat or
//! gently curved panel is best laid out, with nothing bent round.
//!
//! The faces picked on the part, or whole islands; each connected piece on
//! its own, or all of them seen down one direction; taking in, if asked,
//! the whole of each island they are on. A projection keeps the part's
//! proportions as seen from there, at the layout's density - times a scale
//! when one is given - turned to lie best over where the faces were and put
//! there. Where only part of an island is projected, the rest of it is laid
//! out again round them. Faces that turn away from the direction - a fold
//! seen from the front - are refused.

use super::flatten::project;
use super::shape::{best_fit, box_middle, cannot, island_round, layout, regions, scale_of, turned, Moves, Patch, Pick, ShapeError};
use super::layout::UvLayout;
use crate::geometry::Mesh;
use std::collections::{BTreeMap, HashMap};
use tequila_core::math::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectOptions {
    /// Each connected piece seen down its own direction.
    pub separately: bool,
    /// The whole of each island the faces are on.
    pub whole_island: bool,
    /// How large, times the layout's density.
    pub scale: f32,
}

impl Default for ProjectOptions {
    fn default() -> Self {
        ProjectOptions { separately: true, whole_island: false, scale: 1.0 }
    }
}

/// What projecting did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProjectReport {
    pub pieces: usize,
    pub faces: usize,
}

/// The faces `pick` names - picked on the part, else whole islands - laid
/// out as seen down their own direction.
pub fn project_faces(mesh: &Mesh, set: usize, pick: &Pick, options: &ProjectOptions) -> Result<(Moves, ProjectReport), ShapeError> {
    let layout = layout(mesh, set)?;
    if !(options.scale > 0.0 && options.scale.is_finite()) {
        return cannot("the scale has to be more than nothing");
    }
    let mut triangles: Vec<u32> = regions(&layout, pick)?.into_iter().flat_map(|r| if options.whole_island { layout.islands[r.island as usize].triangles.clone() } else { r.triangles }).collect();
    triangles.sort_unstable();
    triangles.dedup();
    let pieces = if options.separately { connected(&layout, &triangles) } else { vec![triangles.clone()] };
    let islands: Vec<u32> = {
        let mut list: Vec<u32> = triangles.iter().map(|&t| layout.island_of[t as usize]).collect();
        list.sort_unstable();
        list.dedup();
        list
    };
    let Some(scale) = scale_of(&layout, &islands) else { return cannot("the faces have no area on the layout") };
    let mut placed: Vec<(u32, Vec2)> = Vec::new();
    for piece in &pieces {
        let patch = Patch::new(mesh, &layout, piece);
        let direction = piece.iter().fold(Vec3::ZERO, |sum, &t| sum + mesh.geometric_normal_scaled(t as usize));
        if direction.length() <= 0.0 {
            return cannot("the faces have no area on the part");
        }
        let mirrored = patch.signed_area(&patch.uv) < 0.0;
        let mut seen: Vec<Vec2> = project(&patch.points, direction).into_iter().map(|p| p * (scale * options.scale)).collect();
        if mirrored {
            seen.iter_mut().for_each(|p| p.x = -p.x);
        }
        if patch.folds(&seen, !mirrored) {
            return cannot("some of the faces turn away from their direction: project fewer at a time, or each piece on its own");
        }
        let angle = best_fit(&seen, &patch.uv).0;
        let turned_points: Vec<Vec2> = seen.iter().map(|p| turned(*p, angle)).collect();
        let shift = box_middle(patch.uv.iter().copied()) - box_middle(turned_points.iter().copied());
        placed.extend(patch.vertices.iter().zip(turned_points).map(|(&v, p)| (v, p + shift)));
    }
    // Islands only partly projected: the rest laid out again round it.
    let mut by_island: BTreeMap<u32, usize> = BTreeMap::new();
    for &t in &triangles {
        *by_island.entry(layout.island_of[t as usize]).or_default() += 1;
    }
    let mut moves = Moves::new();
    let mut taken: HashMap<u32, bool> = HashMap::new();
    for (&island, &count) in &by_island {
        let on: Vec<(u32, Vec2)> = placed.iter().copied().filter(|(v, _)| layout.uv_vertices[*v as usize].island == island).collect();
        let laid = if count == layout.islands[island as usize].triangles.len() { on } else { island_round(mesh, &layout, island, &on, &[], "round the projected faces")? };
        for (v, p) in laid {
            if taken.insert(v, true).is_none() {
                moves.push((v, p));
            }
        }
    }
    Ok((moves, ProjectReport { pieces: pieces.len(), faces: triangles.len() }))
}

/// Some triangles in their connected pieces: joined across edges of the
/// layout, each piece in order.
pub(crate) fn connected(layout: &UvLayout, triangles: &[u32]) -> Vec<Vec<u32>> {
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, &t) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (layout.corner_vertex[t as usize * 3 + k], layout.corner_vertex[t as usize * 3 + (k + 1) % 3]);
            by_edge.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    let mut piece = vec![usize::MAX; triangles.len()];
    let mut out: Vec<Vec<u32>> = Vec::new();
    for seed in 0..triangles.len() {
        if piece[seed] != usize::MAX {
            continue;
        }
        let id = out.len();
        piece[seed] = id;
        let mut members = vec![triangles[seed]];
        let mut front = vec![seed];
        while let Some(i) = front.pop() {
            let t = triangles[i];
            for k in 0..3 {
                let (a, b) = (layout.corner_vertex[t as usize * 3 + k], layout.corner_vertex[t as usize * 3 + (k + 1) % 3]);
                for &n in &by_edge[&(a.min(b), a.max(b))] {
                    if piece[n] == usize::MAX {
                        piece[n] = id;
                        members.push(triangles[n]);
                        front.push(n);
                    }
                }
            }
        }
        members.sort_unstable();
        out.push(members);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, transform, Pivot, UvAffine, UvLayout, UvSelection};
    use tequila_core::math::vec2;

    /// A box's sides squashed on the layout, projected: each is a square
    /// again, as large as the layout's density makes it, where it was.
    #[test]
    fn a_squashed_side_seen_straight_on_is_square_again() {
        let mesh = unwrapped(cube());
        let squashed = transform(&mesh, 0, &UvSelection::Islands(vec![2]), &UvAffine::scale(vec2(1.0, 0.4)), Pivot::Islands).unwrap();
        let before = UvLayout::new(&squashed, 0).unwrap();
        let faces = before.islands[2].triangles.clone();
        let (moves, report) = project_faces(&squashed, 0, &Pick::faces(faces), &ProjectOptions::default()).unwrap();
        assert_eq!((report.pieces, report.faces), (1, 2));
        let out = set_positions(&squashed, 0, &moves).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        let island = &layout.islands[2];
        let size = island.max - island.min;
        assert!((size.x - size.y).abs() < 1e-4, "square: {size:?}");
        assert!((island.centre - before.islands[2].centre).length() < 1e-5, "where it was");
        assert!(layout.folded_triangles().is_empty());
        // Twice the scale, twice as wide.
        let (moves, _) = project_faces(&squashed, 0, &Pick::islands(vec![2]), &ProjectOptions { scale: 2.0, ..Default::default() }).unwrap();
        let twice = UvLayout::new(&set_positions(&squashed, 0, &moves).unwrap(), 0).unwrap();
        let wide = twice.islands[2].max - twice.islands[2].min;
        assert!((wide.x / size.x - 2.0).abs() < 1e-3);
    }

    /// Faces of a dome that turn right round from their average direction
    /// fold when seen down it, and are refused.
    #[test]
    fn faces_turned_away_are_refused() {
        let mesh = unwrapped(tube(12, 2));
        let all: Vec<u32> = (0..mesh.triangle_count() as u32).collect();
        let error = project_faces(&mesh, 0, &Pick::faces(all), &ProjectOptions { separately: false, ..Default::default() }).unwrap_err();
        assert!(matches!(error, ShapeError::Cannot(_)), "{error}");
    }

    #[test]
    fn faces_in_two_pieces_are_two_pieces() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let faces: Vec<u32> = layout.islands[0].triangles.iter().chain(&layout.islands[3].triangles).copied().collect();
        assert_eq!(connected(&layout, &faces).len(), 2);
    }
}
