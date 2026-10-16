//! The Bubbles geometry node (`docs/02` §4.4): air bubbles inside a
//! transmissive part.
//!
//! ## Inside-out spheres of the part's own material
//!
//! A bubble is the boundary between the liquid and a pocket of air. Seen
//! from the liquid, its surface is the liquid's own surface seen from
//! inside — so a bubble is made as a sphere wound inside out, wearing the
//! part's own material. A ray reaching one meets the back of a liquid
//! surface and leaves the liquid into air, refracting by the liquid's IOR
//! the right way round, with the bright total-internal-reflection rim a
//! real bubble has; a ray leaving it re-enters the liquid, and the liquid's
//! absorption picks up again. No material with an IOR below one — which the
//! lobes do not model — and no second material to keep in step.
//!
//! The spec describes bubbles as a density field. That reads as fuzzy
//! specks; real bubbles read as edges, which only an interface gives.
//!
//! ## Placement
//!
//! Scattered through the part by a seeded generator, kept only where the
//! whole bubble is inside — tested by parity against the part's own
//! triangles, so the part must be closed — and where it touches no other.

use tequila_core::math::{vec3, Aabb, Vec3};
use tequila_materials::graph::BubbleRequest;
use tequila_render_core::cutaway::CutMesh;
use tequila_render_core::scene::Mesh as RenderMesh;

/// The most bubbles one part may hold.
pub const MOST: u32 = 5000;

/// Where each bubble goes, as a centre and a radius in world space.
pub fn place(triangles: Vec<[Vec3; 3]>, bounds: Aabb, request: &BubbleRequest) -> Vec<(Vec3, f32)> {
    let count = request.count.min(MOST) as usize;
    let radius = request.radius();
    if count == 0 || !(radius > 0.0) || bounds.is_empty() || triangles.is_empty() {
        return Vec::new();
    }
    let solid = CutMesh::new(triangles);
    let mut random = SplitMix(u64::from(request.seed) ^ 0x9e37_79b9_7f4a_7c15);
    let span = bounds.diagonal();
    let variation = request.variation().clamp(0.0, 1.0);
    let mut placed: Vec<(Vec3, f32)> = Vec::with_capacity(count);
    let offsets = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    for _ in 0..count * 40 {
        if placed.len() == count {
            break;
        }
        let centre = bounds.min + vec3(random.unit() * span.x, random.unit() * span.y, random.unit() * span.z);
        let r = radius * (1.0 + variation * (random.unit() * 2.0 - 1.0)).max(0.1);
        // Wholly inside: the centre and the six points a radius away.
        if !solid.contains(centre) || !offsets.iter().all(|o| solid.contains(centre + *o * r)) {
            continue;
        }
        if placed.iter().any(|(c, other)| (*c - centre).length() < r + other) {
            continue;
        }
        placed.push((centre, r));
    }
    placed
}

/// A unit sphere wound inside out, with its normals turned inward: the
/// surface of a pocket of air, seen from the liquid around it.
pub fn bubble_mesh() -> RenderMesh {
    let mut sphere = crate::demo::uv_sphere(1.0, 24, 12);
    for triangle in sphere.indices.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
    for n in &mut sphere.normals {
        *n = -*n;
    }
    sphere
}

/// A small seeded generator: bubbles land in the same places every time the
/// scene is built.
struct SplitMix(u64);

impl SplitMix {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bubble surface faces into the bubble: winding and normals agree,
    /// and both point at the centre.
    #[test]
    fn a_bubble_is_inside_out() {
        let mesh = bubble_mesh();
        for (t, triangle) in mesh.indices.chunks_exact(3).enumerate().step_by(37) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| mesh.positions[i as usize]);
            let face = (b - a).cross(c - a);
            let centre = (a + b + c) * (1.0 / 3.0);
            if face.length() > 1e-6 {
                assert!(face.dot(centre) < 0.0, "triangle {t} faces out");
            }
        }
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            assert!(n.dot(*p) < 0.0);
        }
    }
}
