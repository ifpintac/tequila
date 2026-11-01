//! Grow or shrink border: every vertex on an island's outer border moved
//! out from the island, or in, by one distance on the layout, square to
//! the border where it is; and the inside - holes and all - carried along
//! smoothly, so an island made a little smaller keeps its shape. What makes
//! room between the rims of parts laid out against each other.
//!
//! A vertex's way out is square to the border edges round it, weighed by
//! their lengths over a short stretch, so a rim of many short edges
//! zigzagging a little is moved as the line it makes; and it goes far
//! enough that the edges either side move the whole distance - a corner
//! further, up to twice it - as an outline offset does.
//!
//! Where a part of an island is thinner than the move - a spike, a notch -
//! the border there would cross the triangles inside it: the move is eased
//! there, halved each time until nothing turns over, and the report says
//! how many vertices moved less. An island smaller across than twice the
//! distance is not shrunk at all, and is counted.

use super::shape::{cannot, carried, islands_of_pick, layout, Moves, Patch, Pick, ShapeError};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowReport {
    pub islands: usize,
    pub vertices: usize,
    /// Border vertices moved less than the distance, where the island is
    /// thinner than it.
    pub eased: usize,
    /// Islands left as they were: too small to shrink by that much, or
    /// turned over by any move at all.
    pub left: usize,
}

/// How many times a fold halves the moves near it before an island is
/// left as it was.
const EASINGS: usize = 8;

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
        let Some(ring) = island.boundary.first() else { continue };
        let size = island.max - island.min;
        if by < 0.0 && size.x.min(size.y) <= -2.0 * by || ring.len() < 3 {
            report.left += 1;
            continue;
        }
        let patch = Patch::new(mesh, &layout, &island.triangles);
        // Out of the island is to the left of its border as it runs, seen
        // on the layout, for an island that reads the right way.
        let outward = if island.mirrored { -1.0 } else { 1.0 };
        let n = ring.len();
        let at = |k: usize| layout.uv_vertices[ring[k % n] as usize].uv;
        let normal = |a: Vec2, b: Vec2| {
            let d = b - a;
            let length = d.length();
            if length > 0.0 { vec2(-d.y, d.x) * (outward / length) } else { Vec2::ZERO }
        };
        let edges: Vec<(Vec2, f32)> = (0..n).map(|k| (normal(at(k), at(k + 1)), (at(k + 1) - at(k)).length())).collect();
        // As far as twice the distance, or a sixteenth of the way round,
        // whichever is shorter: a small island's corners stay corners.
        let perimeter: f32 = edges.iter().map(|e| e.1).sum();
        let span = (by.abs() * 2.0).min(perimeter / 16.0);
        let mut ways: Vec<Vec2> = Vec::with_capacity(n);
        for k in 0..n {
            let (n1, n2) = (edges[(k + n - 1) % n].0, edges[k].0);
            let mut sum = n1 * edges[(k + n - 1) % n].1 + n2 * edges[k].1;
            for side in [1usize, n - 1] {
                let (mut j, mut walked) = (if side == 1 { (k + 1) % n } else { (k + n - 2) % n }, 0.0f32);
                for _ in 0..(n / 8).min(64) {
                    walked += edges[j].1;
                    if walked > span {
                        break;
                    }
                    sum = sum + edges[j].0 * edges[j].1;
                    j = (j + side) % n;
                }
            }
            let way = if sum.length() > 1e-12 { sum.normalize() } else { n1 };
            // Far enough that each edge moves the whole distance.
            let reach = way.dot(n1).max(way.dot(n2)).max(0.5);
            ways.push(way * (by / reach));
        }
        // Moved past itself all round: shrunk by more than it has.
        let turned = (0..n).filter(|&k| ((at(k + 1) + ways[(k + 1) % n]) - (at(k) + ways[k])).dot(at(k + 1) - at(k)) <= 0.0).count();
        if turned * 2 > n {
            return cannot(format!("island {}'s border would cross itself: grow or shrink it by less", i + 1));
        }
        let weights = patch.weights();
        let reads_right = !island.mirrored;
        let mut share = vec![1.0f32; n];
        let mut laid = None;
        for _ in 0..EASINGS {
            let held: Vec<(usize, Vec2)> = (0..n).map(|k| (patch.local[&ring[k]], at(k) + ways[k] * share[k])).collect();
            let Some(uv) = carried(&patch, &held, &weights) else { break };
            if !patch.folds(&uv, reads_right) {
                laid = Some(uv);
                break;
            }
            // Halve the moves of the border vertices on a turned triangle,
            // and of their neighbours along the border.
            let sign = if reads_right { 1.0 } else { -1.0 };
            let mut turned_over = vec![false; patch.len()];
            for f in &patch.faces {
                if super::flatten::facing(uv[f[0]], uv[f[1]], uv[f[2]]) * sign <= 0.0 {
                    f.iter().for_each(|&v| turned_over[v] = true);
                }
            }
            let mut eased = false;
            for k in 0..n {
                let near = [k + n - 1, k, k + 1].iter().any(|&j| turned_over[patch.local[&ring[j % n]]]);
                if near {
                    share[k] *= 0.5;
                    eased = true;
                }
            }
            if !eased {
                break;
            }
        }
        let Some(uv) = laid else {
            report.left += 1;
            continue;
        };
        report.islands += 1;
        report.vertices += n;
        report.eased += share.iter().filter(|&&s| s < 1.0).count();
        moves.extend(patch.moves(&uv));
    }
    if moves.is_empty() {
        return cannot(if report.left > 0 { "the islands are too small, or too thin, to move their borders by that much" } else { "nothing to move" });
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

    /// A disc whose rim zigzags in and out by less than the distance, as a
    /// CAD rim's short edges do: shrunk, it shrinks, and nothing turns over.
    #[test]
    fn a_zigzag_rim_shrinks_without_turning_over() {
        let n = 96;
        let mut positions = vec![tequila_core::math::vec3(0.0, 0.0, 0.0)];
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let r = if i % 2 == 0 { 1.0 } else { 0.99 };
            positions.push(tequila_core::math::vec3(a.cos() * r, 0.0, a.sin() * r));
        }
        // A ring inside, so the inside has vertices to follow.
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            positions.push(tequila_core::math::vec3(a.cos() * 0.6, 0.0, a.sin() * 0.6));
        }
        let rim = |i: usize| 1 + (i % n) as u32;
        let ring = |i: usize| 1 + n as u32 + (i % n) as u32;
        let mut indices = Vec::new();
        for i in 0..n {
            indices.extend_from_slice(&[0, ring(i + 1), ring(i)]);
            indices.extend_from_slice(&[ring(i), ring(i + 1), rim(i + 1), ring(i), rim(i + 1), rim(i)]);
        }
        let uv = positions.iter().map(|p| vec2(p.x * 0.4 + 0.5, p.z * 0.4 + 0.5)).collect();
        let mesh = Mesh::new(positions, indices).with_uvs(uv);
        assert!(UvLayout::new(&mesh, 0).unwrap().folded_triangles().is_empty());
        let before = UvLayout::new(&mesh, 0).unwrap().islands[0].max.x;
        let (moves, _) = grow_border(&mesh, 0, &Pick::default(), -0.02).unwrap();
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert!(after.folded_triangles().is_empty());
        assert!((before - after.islands[0].max.x - 0.02).abs() < 0.004, "{before} {}", after.islands[0].max.x);
    }
}
