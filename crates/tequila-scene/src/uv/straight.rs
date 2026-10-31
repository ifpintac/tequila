//! Straight line: a path of picked edges or vertices laid on a straight
//! line across or down the layout, whichever is nearer the way it runs end
//! to end, spaced as its edges are long on the part and as long as the path
//! is on the part at its island's density; and the rest of the island laid
//! out again round it, as true to the surface as it can be, its pins held
//! too where they are kept.

use super::shape::{cannot, chain, island_round, layout, on_island, picked_chain, point, Chain, Moves, Pick, ShapeError};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// What laying a path straight did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StraightReport {
    /// The vertices on the line.
    pub vertices: usize,
    /// Across the layout; else down it.
    pub across: bool,
}

/// The path `pick`'s edges or vertices make laid straight, its island laid
/// out again round it with `pins` (mesh vertices) held as well.
pub fn straight_line(mesh: &Mesh, set: usize, pick: &Pick, pins: &[u32]) -> Result<(Moves, StraightReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let (island, pairs) = picked_chain(&layout, pick)?;
    let path = match chain(&pairs) {
        Some(Chain::Path(path)) => path,
        Some(Chain::Loop(_)) => return cannot("the picked edges close a loop: Circle lays a loop out round"),
        None => return cannot("the picked edges are not one path: they branch, or are in pieces"),
    };
    let place = |v: u32| layout.uv_vertices[v as usize].uv;
    let lengths: Vec<f32> = path.windows(2).map(|w| (point(mesh, &layout, w[1]) - point(mesh, &layout, w[0])).length()).collect();
    let total: f32 = lengths.iter().sum();
    let density = layout.islands[island as usize].density();
    if total <= 0.0 || density <= 0.0 {
        return cannot("the path has no length on the part");
    }
    let scale = density.sqrt();
    // Across or down, whichever the path runs nearer, and the same way
    // round as it runs.
    let (start, end) = (place(path[0]), place(*path.last().expect("a path")));
    let run = end - start;
    let across = run.x.abs() >= run.y.abs();
    let along = |p: Vec2| if across { p.x } else { p.y };
    let side = |p: Vec2| if across { p.y } else { p.x };
    let sign = if along(run) < 0.0 { -1.0 } else { 1.0 };
    let middle = (along(start) + along(end)) * 0.5;
    let level = path.iter().map(|&v| side(place(v))).sum::<f32>() / path.len() as f32;
    let mut done = 0.0f32;
    let mut placed = Vec::with_capacity(path.len());
    for (k, &v) in path.iter().enumerate() {
        if k > 0 {
            done += lengths[k - 1];
        }
        let a = middle + sign * (done - total * 0.5) * scale;
        placed.push((v, if across { vec2(a, level) } else { vec2(level, a) }));
    }
    let held = on_island(&layout, pins, island);
    let moves = island_round(mesh, &layout, island, &placed, &held, "round the line")?;
    Ok((moves, StraightReport { vertices: path.len(), across }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, UvLayout};

    /// A wavy sheet's row of vertices along x at z = 0.5: a path that is
    /// not straight on the layout.
    #[test]
    fn a_wavy_path_lies_straight_at_its_length_on_the_part() {
        let mesh = unwrapped(sheet(6, 8, |x, z| (x * 6.0).sin() * 0.08 + z * z * 0.3));
        let mut row: Vec<u32> = (0..mesh.positions.len() as u32).filter(|&v| (mesh.positions[v as usize].z - 0.5).abs() < 1e-5).collect();
        row.sort_by(|&a, &b| mesh.positions[a as usize].x.total_cmp(&mesh.positions[b as usize].x));
        assert_eq!(row.len(), 9);
        let (moves, report) = straight_line(&mesh, 0, &Pick::vertices(row.clone()), &[]).unwrap();
        assert_eq!(report.vertices, 9);
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        assert!(layout.folded_triangles().is_empty());
        let places: Vec<Vec2> = row.iter().map(|&v| out.uv_sets[0][v as usize]).collect();
        let level = if report.across { places[0].y } else { places[0].x };
        for p in &places {
            assert!(((if report.across { p.y } else { p.x }) - level).abs() < 1e-5, "on one line: {p:?}");
        }
        // As long as the path is on the part, at the island's density.
        let before = UvLayout::new(&mesh, 0).unwrap();
        let length_3d: f32 = row.windows(2).map(|w| (mesh.positions[w[1] as usize] - mesh.positions[w[0] as usize]).length()).sum();
        let laid = places.windows(2).map(|w| (w[1] - w[0]).length()).sum::<f32>();
        assert!((laid / (length_3d * before.islands[0].density().sqrt()) - 1.0).abs() < 1e-3, "{laid}");
    }

    #[test]
    fn a_branch_or_a_loop_is_refused() {
        let mesh = unwrapped(sheet(3, 3, |_, _| 0.0));
        // An unwrap numbers the vertices anew: found by where they are.
        let at = |r: u32, c: u32| {
            let p = tequila_core::math::vec3(c as f32 / 3.0, 0.0, r as f32 / 3.0);
            mesh.positions.iter().position(|q| (*q - p).length() < 1e-5).unwrap() as u32
        };
        // The four edges round a quad: they close a loop.
        let quad = vec![[at(1, 1), at(1, 2)], [at(1, 2), at(2, 2)], [at(2, 2), at(2, 1)], [at(2, 1), at(1, 1)]];
        let refused = straight_line(&mesh, 0, &Pick::edges(quad), &[]).unwrap_err();
        assert!(matches!(&refused, ShapeError::Cannot(why) if why.contains("loop")), "{refused}");
        // Two vertices that share no edge.
        let apart = straight_line(&mesh, 0, &Pick::vertices(vec![at(0, 0), at(3, 3)]), &[]).unwrap_err();
        assert!(matches!(&apart, ShapeError::Cannot(_)), "{apart}");
        assert!(straight_line(&mesh, 0, &Pick::default(), &[]).is_err());
    }
}
