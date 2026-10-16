//! Per-vertex occlusion for the Occlusion node (`docs/02` §6.2).
//!
//! "Trace N short rays over the hemisphere at shading time, radius-limited;
//! return the occluded fraction. Cache per-vertex when the geometry is
//! static to avoid per-sample cost." The geometry is static between edits,
//! so this is the cached form throughout: measured once per vertex, in the
//! part's own space, against the part's own triangles, and interpolated at
//! shading time like curvature.
//!
//! Against the part itself only. The node is for what a part does to its
//! own surface — the grime in its recesses, the shadowed groove between two
//! panels of one moulding — and a part that got dirtier because another was
//! moved next to it would be a surprise rather than a feature.

use tequila_core::math::Vec3;
use tequila_render_core::cutaway::CutMesh;
use tequila_render_core::scene::Mesh as RenderMesh;
use rayon::prelude::*;

/// Rays per vertex. Enough for a smooth gradient into a corner once
/// interpolated across the triangles; the error left is below what a ramp
/// on base colour shows.
const RAYS: usize = 48;

/// The occluded fraction at every vertex of `mesh`, within `radius` of its
/// own units.
pub fn vertex_occlusion(mesh: &RenderMesh, radius: f32) -> Vec<f32> {
    if mesh.positions.is_empty() || radius <= 0.0 {
        return vec![0.0; mesh.positions.len()];
    }
    let triangles: Vec<[Vec3; 3]> = mesh
        .indices
        .chunks_exact(3)
        .filter(|t| t.iter().all(|&i| (i as usize) < mesh.positions.len()))
        .map(|t| [mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]])
        .collect();
    let hierarchy = CutMesh::new(triangles);
    let normals = vertex_normals(mesh);
    // Off the surface by a sliver of the radius, so a ray does not find the
    // triangles it starts on.
    let lift = radius * 1e-3 + mesh.bounds().diagonal().length() * 1e-6;

    (0..mesh.positions.len())
        .into_par_iter()
        .map(|i| {
            let n = normals[i];
            let Some(n) = n.try_normalize() else { return 0.0 };
            let origin = mesh.positions[i] + n * lift;
            let (t, b) = basis(n);
            // A different turn of the same pattern at each vertex, so
            // neighbours do not share their gaps.
            let turn = (i as f32 * 0.618_034).fract() * std::f32::consts::TAU;
            let mut blocked = 0usize;
            for k in 0..RAYS {
                // Cosine-weighted, Hammersley.
                let u = (k as f32 + 0.5) / RAYS as f32;
                let v = radical_inverse(k as u32);
                let r = u.sqrt();
                let phi = v * std::f32::consts::TAU + turn;
                let local = (r * phi.cos(), r * phi.sin(), (1.0 - u).max(0.0).sqrt());
                let direction = t * local.0 + b * local.1 + n * local.2;
                if hierarchy.hits(origin, direction, 0.0, radius) {
                    blocked += 1;
                }
            }
            blocked as f32 / RAYS as f32
        })
        .collect()
}

fn radical_inverse(mut bits: u32) -> f32 {
    bits = bits.reverse_bits();
    bits as f32 * (1.0 / 4_294_967_296.0)
}

fn basis(n: Vec3) -> (Vec3, Vec3) {
    let reference = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let t = reference.cross(n).normalize();
    (t, n.cross(t))
}

/// The mesh's own normals, or ones made from its faces when it has none.
fn vertex_normals(mesh: &RenderMesh) -> Vec<Vec3> {
    if mesh.normals.len() == mesh.positions.len() {
        return mesh.normals.clone();
    }
    let mut out = vec![Vec3::ZERO; mesh.positions.len()];
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        if a >= out.len() || b >= out.len() || c >= out.len() {
            continue;
        }
        let face = (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a]);
        for i in [a, b, c] {
            out[i] = out[i] + face;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// A floor beside a wall: the floor's edge a little way from the wall is
    /// occluded, the floor far from it is open, and a radius shorter than
    /// the gap sees nothing.
    #[test]
    fn a_corner_is_occluded_and_open_floor_is_not() {
        let positions = vec![
            // Floor, facing up, from a tenth off the wall out to 2.
            vec3(0.1, 0.0, 0.0),
            vec3(2.0, 0.0, 0.0),
            vec3(2.0, 0.0, 2.0),
            vec3(0.1, 0.0, 2.0),
            // Wall along x = 0, facing +x.
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 2.0),
            vec3(0.0, 2.0, 2.0),
            vec3(0.0, 2.0, 0.0),
        ];
        let indices = vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7];
        let mut mesh = RenderMesh::new(positions, indices);
        mesh.normals = vec![Vec3::Y, Vec3::Y, Vec3::Y, Vec3::Y, Vec3::X, Vec3::X, Vec3::X, Vec3::X];
        let near = vertex_occlusion(&mesh, 1.0);
        // Vertex 0 is by the wall, at its end: half the hemisphere faces the
        // wall and half of that has wall beside it, so about a quarter.
        // Vertex 2 is the far corner of the floor.
        assert!((0.18..0.32).contains(&near[0]), "beside the wall's end reads {}", near[0]);
        assert!(near[2] < 0.05, "open floor reads {}", near[2]);
        let short = vertex_occlusion(&mesh, 0.05);
        assert!(short.iter().all(|&o| o < 0.05), "a tiny radius still found the wall: {short:?}");
    }
}
