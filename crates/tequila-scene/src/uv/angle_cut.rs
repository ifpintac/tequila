//! Cut by angle: the edges of the part where the faces either side meet
//! more sharply than an angle - a box's edges, a cylinder's rims - found
//! so the layout can be cut along them, and unwrapped again keeping the
//! cuts. Only edges inside an island are found: a seam is cut already.

use super::edit::{chosen_islands, layout_of, UvError};
use super::MeshEdge;
use crate::geometry::Mesh;

/// The edges inside `islands` - every island when none are named - whose
/// faces meet at more than `degrees` from flat, each named by the mesh
/// vertices at its ends on one side.
pub fn sharp_edges(mesh: &Mesh, set: usize, islands: &[u32], degrees: f32) -> Result<Vec<MeshEdge>, UvError> {
    let layout = layout_of(mesh, set)?;
    let chosen = chosen_islands(&layout, islands)?;
    let inside = |island: u32| chosen.is_empty() || chosen.binary_search(&island).is_ok();
    let limit = degrees.clamp(0.0, 180.0).to_radians().cos();
    let normal = |t: u32| mesh.geometric_normal_scaled(t as usize).try_normalize();
    let mut out = Vec::new();
    for group in layout.table.groups() {
        let sides = &layout.table.sides[group];
        // An edge of two faces only: three or more is no surface to fold.
        let [a, b] = sides else { continue };
        let (ta, tb) = (a.triangle, b.triangle);
        let island = layout.island_of[ta as usize];
        if island != layout.island_of[tb as usize] || !inside(island) {
            continue;
        }
        // Joined on the layout: the two sides' corners on the same places.
        let cv = &layout.corner_vertex;
        if cv[a.lo_corner()] != cv[b.lo_corner()] || cv[a.hi_corner()] != cv[b.hi_corner()] {
            continue;
        }
        let (Some(na), Some(nb)) = (normal(ta), normal(tb)) else { continue };
        if na.dot(nb) < limit {
            out.push(a.vertices);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{cut, CutOptions, UvLayout};
    use tequila_core::math::{vec2, vec3, Vec2};

    /// A cylinder with a lid, laid out as one piece: the rim where the lid
    /// meets the side is sharp, the side's own edges are not.
    fn lidded(segments: usize) -> Mesh {
        let mut mesh = tube(segments, 3);
        let base = mesh.positions.len() as u32;
        // The lid: a fan from its middle round the top ring.
        mesh.positions.push(vec3(0.0, 1.0, 0.0));
        let top = |s: usize| (3 * (segments + 1) + s) as u32;
        for s in 0..segments {
            mesh.indices.extend_from_slice(&[base, top(s + 1), top(s)]);
        }
        mesh
    }

    #[test]
    fn a_lid_s_rim_is_sharp_and_the_side_is_not() {
        let mesh = lidded(16);
        // One island: every vertex laid out by where it is on the part, so
        // every edge is inside it.
        let uv: Vec<Vec2> = mesh.positions.iter().map(|p| vec2(p.x + p.y * 2.0, p.z)).collect();
        let laid = mesh.clone().with_uvs(uv);
        let layout = UvLayout::new(&laid, 0).unwrap();
        let rim = sharp_edges(&laid, 0, &[], 30.0).unwrap();
        // Every rim edge, as the side's columns are 22.5 degrees apart.
        assert_eq!(rim.len(), 16, "{rim:?}");
        for [a, b] in &rim {
            assert!((laid.positions[*a as usize].y - 1.0).abs() < 1e-5 && (laid.positions[*b as usize].y - 1.0).abs() < 1e-5);
        }
        // Steeper than the side's own turn: the side's edges too.
        assert!(sharp_edges(&laid, 0, &[], 20.0).unwrap().len() > 16);
        // Cut along them, the lid comes off as an island of its own.
        let cut_off = cut(&laid, 0, &rim, &CutOptions::default()).unwrap();
        assert!(UvLayout::new(&cut_off, 0).unwrap().islands.len() > layout.islands.len());
    }
}
