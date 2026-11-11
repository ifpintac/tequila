//! Per-vertex closeness of other parts, for the Distance node.
//!
//! Occlusion measures what a part does to its own surface; this measures
//! what its neighbours do: at each vertex, how far away the nearest other
//! part is, within the reach the node asked for. The mark a foot leaves on
//! the shelf it stands on, the clean stripe under a strap, the wear where
//! two panels rub.
//!
//! Measured as occlusion is: once per vertex, by rays over the hemisphere
//! above it, kept on the part's mesh and interpolated at shading time. The
//! rays go to the other parts' triangles where they stand in the scene, so
//! the measure moves with the parts; the nearest thing any ray meets is
//! the distance. A neighbour that no ray meets - one edge-on, far off to
//! the side - is not seen, which errs toward "further", never toward a
//! mark where there is nothing.

use tequila_core::math::{Aabb, Transform, Vec3};
use tequila_render_core::cutaway::CutMesh;
use tequila_render_core::scene::Mesh as RenderMesh;
use rayon::prelude::*;

/// Rays per vertex: the hemisphere, and one straight out.
const RAYS: usize = 32;

/// Another part, where it stands: its bounds in the scene and its
/// triangles there.
pub struct Neighbour {
    pub bounds: Aabb,
    pub triangles: CutMesh,
}

impl std::fmt::Debug for Neighbour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Neighbour").field("bounds", &self.bounds).finish_non_exhaustive()
    }
}

impl Neighbour {
    /// `mesh` as `transform` places it.
    pub fn new(mesh: &RenderMesh, transform: &Transform) -> Neighbour {
        let mut bounds = Aabb::EMPTY;
        let placed: Vec<Vec3> = mesh.positions.iter().map(|p| transform.point(*p)).collect();
        for p in &placed {
            bounds.extend_point(*p);
        }
        let triangles = mesh
            .indices
            .chunks_exact(3)
            .filter(|t| t.iter().all(|&i| (i as usize) < placed.len()))
            .map(|t| [placed[t[0] as usize], placed[t[1] as usize], placed[t[2] as usize]])
            .collect();
        Neighbour { bounds, triangles: CutMesh::new(triangles) }
    }

    /// Whether any of it could be within `reach` of `point`.
    fn near(&self, point: Vec3, reach: f32) -> bool {
        let (low, high) = (self.bounds.min, self.bounds.max);
        point.x >= low.x - reach && point.x <= high.x + reach && point.y >= low.y - reach && point.y <= high.y + reach && point.z >= low.z - reach && point.z <= high.z + reach
    }
}

/// How close the nearest of `others` is at every vertex of `mesh`, placed
/// by `transform`: 1 touching, 0 for none within `radius` scene units.
pub fn vertex_closeness(mesh: &RenderMesh, transform: &Transform, others: &[&Neighbour], radius: f32) -> Vec<f32> {
    if mesh.positions.is_empty() || radius <= 0.0 || others.is_empty() {
        return vec![0.0; mesh.positions.len()];
    }
    let normals = crate::occlusion::vertex_normals(mesh);
    // Off the surface by a sliver of the reach: a part resting on this one
    // starts at no distance, and a ray from exactly there can miss it.
    let lift = radius * 1e-3;
    (0..mesh.positions.len())
        .into_par_iter()
        .map(|i| {
            let Some(n) = transform.normal(normals[i]).try_normalize() else { return 0.0 };
            let at = transform.point(mesh.positions[i]);
            let near: Vec<&&Neighbour> = others.iter().filter(|other| other.near(at, radius)).collect();
            if near.is_empty() {
                return 0.0;
            }
            // From a little under the surface, so a part lying on it is met.
            let origin = at - n * lift;
            let (t, b) = crate::occlusion::basis(n);
            let turn = (i as f32 * 0.618_034).fract() * std::f32::consts::TAU;
            let mut nearest = f32::INFINITY;
            for k in 0..=RAYS {
                let direction = if k == RAYS {
                    n
                } else {
                    // Even over the hemisphere, Hammersley: a neighbour to
                    // the side is as much a neighbour as one above.
                    let u = (k as f32 + 0.5) / RAYS as f32;
                    let v = crate::occlusion::radical_inverse(k as u32);
                    let (z, phi) = (u, v * std::f32::consts::TAU + turn);
                    let r = (1.0 - z * z).max(0.0).sqrt();
                    t * (r * phi.cos()) + b * (r * phi.sin()) + n * z
                };
                for other in &near {
                    if let Some((distance, _)) = other.triangles.nearest(origin, direction, 0.0, nearest.min(radius + lift)) {
                        nearest = nearest.min(distance);
                    }
                }
            }
            (1.0 - (nearest - lift).max(0.0) / radius).clamp(0.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Mat4};

    /// A square in the XZ plane, facing up, a metre across, of a grid of
    /// vertices.
    fn sheet(cells: usize) -> RenderMesh {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for row in 0..=cells {
            for column in 0..=cells {
                positions.push(vec3(column as f32 / cells as f32 - 0.5, 0.0, row as f32 / cells as f32 - 0.5));
            }
        }
        let at = |row: usize, column: usize| (row * (cells + 1) + column) as u32;
        for row in 0..cells {
            for column in 0..cells {
                // Wound to face +Y.
                indices.extend([at(row, column), at(row + 1, column), at(row, column + 1), at(row, column + 1), at(row + 1, column), at(row + 1, column + 1)]);
            }
        }
        RenderMesh::new(positions, indices)
    }

    fn moved(by: Vec3) -> Transform {
        Transform::from_matrix(Mat4::translation(by)).expect("invertible")
    }

    /// A small plate held over the middle of a sheet marks the sheet under
    /// it, by how near it is, and nothing further off than the reach.
    #[test]
    fn a_part_held_over_another_marks_it_by_how_near_it_is() {
        let floor = sheet(20);
        let plate = sheet(4);
        let small = Transform::from_matrix(Mat4::translation(vec3(0.0, 0.0, 0.0)) * Mat4::scale(vec3(0.2, 1.0, 0.2))).expect("invertible");
        let over = |height: f32| {
            let place = Transform::from_matrix(Mat4::translation(vec3(0.0, height, 0.0)) * Mat4::scale(vec3(0.2, 1.0, 0.2))).expect("invertible");
            let neighbour = Neighbour::new(&plate, &place);
            vertex_closeness(&floor, &Transform::IDENTITY, &[&neighbour], 0.05)
        };
        let _ = small;
        let middle = 10 * 21 + 10;
        let corner = 0;
        // Lying on it: touching. A fifth of the reach up: four fifths
        // close. At the reach and past it: nothing.
        let touching = over(0.0);
        assert!(touching[middle] > 0.97, "under a plate lying on it, {}", touching[middle]);
        let near = over(0.01);
        assert!((near[middle] - 0.8).abs() < 0.05, "a fifth of the reach under it, {}", near[middle]);
        let far = over(0.06);
        assert_eq!(far[middle], 0.0, "past the reach");
        // Out from under it, a long way off across the sheet: nothing.
        for measured in [&touching, &near] {
            assert_eq!(measured[corner], 0.0, "at the sheet's corner, half a metre from the plate");
        }
        // Nearer the plate's edge than its middle is no nearer: under it
        // all the way across.
        let under_edge = 10 * 21 + 11;
        assert!((near[under_edge] - near[middle]).abs() < 0.08, "{} and {}", near[under_edge], near[middle]);

        // Moved with the part: the sheet set a metre to the side is clear.
        let neighbour = Neighbour::new(&plate, &Transform::from_matrix(Mat4::scale(vec3(0.2, 1.0, 0.2))).expect("invertible"));
        let aside = vertex_closeness(&floor, &moved(vec3(1.5, 0.0, 0.0)), &[&neighbour], 0.05);
        assert!(aside.iter().all(|c| *c == 0.0));
        // And with no neighbours, or no reach, nothing is close.
        assert!(vertex_closeness(&floor, &Transform::IDENTITY, &[], 0.05).iter().all(|c| *c == 0.0));
        assert!(vertex_closeness(&floor, &Transform::IDENTITY, &[&neighbour], 0.0).iter().all(|c| *c == 0.0));
    }
}
