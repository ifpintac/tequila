//! Grow or shrink border: every vertex on an island's border moved out
//! from the island, or in, by one distance on the layout, square to the
//! border where it is; and the inside carried along smoothly, so an island
//! made a little smaller keeps its shape. What makes room between the rims
//! of parts laid out against each other.
//!
//! A vertex's way out is square to the two border edges either side of it,
//! between them, and it goes far enough that both edges move the whole
//! distance - a corner further, up to twice it - as an outline offset does.
//! Holes go the other way to the outer border, as the island's own edge
//! does: an island grown has its holes smaller.

use super::shape::{cannot, follow, islands_of_pick, layout, Moves, Patch, Pick, ShapeError};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowReport {
    pub islands: usize,
    pub vertices: usize,
}

/// The borders of the islands `pick` is on - every island when it picks
/// nothing - moved out by `by` (in the layout's units; in where negative),
/// the insides following.
pub fn grow_border(mesh: &Mesh, set: usize, pick: &Pick, by: f32) -> Result<(Moves, GrowReport), ShapeError> {
    let layout = layout(mesh, set)?;
    if !by.is_finite() || by == 0.0 {
        return cannot("give a distance to grow or shrink by");
    }
    let islands = islands_of_pick(&layout, pick)?;
    let mut moves = Moves::new();
    let mut report = GrowReport::default();
    for i in islands {
        let island = &layout.islands[i as usize];
        let patch = Patch::new(mesh, &layout, &island.triangles);
        // Out of the island is to the left of its border as it runs, seen
        // on the layout, for an island that reads the right way.
        let outward = if island.mirrored { -1.0 } else { 1.0 };
        let mut held: Vec<(usize, Vec2)> = Vec::new();
        for ring in &island.boundary {
            let n = ring.len();
            if n < 2 {
                continue;
            }
            let at = |k: usize| layout.uv_vertices[ring[k % n] as usize].uv;
            let normal = |a: Vec2, b: Vec2| {
                let d = b - a;
                let length = d.length();
                if length > 0.0 { vec2(-d.y, d.x) * (outward / length) } else { Vec2::ZERO }
            };
            for k in 0..n {
                let (before, here, after) = (at(k + n - 1), at(k), at(k + 1));
                let (n1, n2) = (normal(before, here), normal(here, after));
                let sum = n1 + n2;
                let way = if sum.length() > 1e-9 { sum.normalize() } else { n1 };
                // Far enough that each edge moves the whole distance.
                let reach = way.dot(n1).max(way.dot(n2)).max(0.5);
                held.push((patch.local[&ring[k]], here + way * (by / reach)));
            }
        }
        // A border edge turned round has been moved past the one across
        // from it: the border crosses itself there.
        let mut new = patch.uv.clone();
        held.iter().for_each(|&(i, p)| new[i] = p);
        let crossed = island.boundary.iter().any(|ring| {
            let n = ring.len();
            (0..n).any(|k| {
                let (a, b) = (patch.local[&ring[k]], patch.local[&ring[(k + 1) % n]]);
                (new[b] - new[a]).dot(patch.uv[b] - patch.uv[a]) <= 0.0
            })
        });
        let laid = if crossed { None } else { follow(&patch, &held, &patch.weights()) };
        let Some(uv) = laid else {
            return cannot(format!("island {}'s border would cross itself: grow or shrink it by less", i + 1));
        };
        report.islands += 1;
        report.vertices += held.len();
        moves.extend(patch.moves(&uv));
    }
    Ok((moves, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, UvLayout};

    #[test]
    fn a_square_island_grows_and_shrinks_by_the_distance_all_round() {
        let mesh = unwrapped(cube());
        let before = UvLayout::new(&mesh, 0).unwrap();
        let side = before.islands[0].max - before.islands[0].min;
        for by in [0.01f32, -0.02] {
            let (moves, report) = grow_border(&mesh, 0, &Pick::islands(vec![0]), by).unwrap();
            assert_eq!((report.islands, report.vertices), (1, 4));
            let out = set_positions(&mesh, 0, &moves).unwrap();
            let after = UvLayout::new(&out, 0).unwrap();
            let size = after.islands[0].max - after.islands[0].min;
            assert!((size.x - (side.x + 2.0 * by)).abs() < 1e-5 && (size.y - (side.y + 2.0 * by)).abs() < 1e-5, "{by}: {size:?} from {side:?}");
            assert!((after.islands[0].centre - before.islands[0].centre).length() < 1e-5);
            assert!(after.folded_triangles().is_empty());
        }
        // Shrunk by more than it has: its border crosses itself.
        assert!(grow_border(&mesh, 0, &Pick::islands(vec![0]), -side.x).is_err());
    }
}
