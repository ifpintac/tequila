//! §4.4's Displace: geometry moved by a height from the material graph.
//!
//! Bump changes how a surface shades; displacement changes where it is. The
//! silhouette of a knurled knob, the shadow a tread casts into its own
//! grooves, the edge of a stitched seam against the backdrop — none of those
//! can come from a normal, because a normal does not move anything.
//!
//! The graph's **Displacement** socket takes a height in scene units — a
//! pattern or a height map through a Bump node, which gives it its depth,
//! exactly as for bump. When the renderer builds its copy of a mesh for a
//! part whose material drives that socket, the mesh is subdivided until its
//! edges are no longer than the material's **Displacement Detail**, then each
//! vertex is moved along the surface normal by the height evaluated there.
//!
//! - **Crack-free.** Subdivision shares the new vertex on an edge between the
//!   two triangles that meet there, and vertices at one position — a UV seam
//!   or a hard edge stored as split vertices — move along one welded normal,
//!   so a surface that was closed stays closed.
//! - **Bounded.** Subdivision stops at [`TRIANGLE_BUDGET`] whatever the detail
//!   asks for; a detail too fine for a large part renders coarser rather
//!   than exhausting memory.
//! - **In scene units, like everything else.** The detail and the height are
//!   both measured in the world, so a part imported at a fiftieth of its
//!   units' size displaces the same as one that was not.
//!
//! A detail of zero is §4.4's bump-only approximation: no new geometry, and
//! the height shades as a bump (`surface_bump`).

use crate::geometry::{tangent_frames, vertex_curvature};
use tequila_core::math::{Vec2, Vec3};
use tequila_materials::graph::{CompiledGraph, Register};
use tequila_materials::material_type::Param;
use tequila_render_core::material::Material;
use tequila_render_core::scene::Mesh as RenderMesh;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::projection::SurfacePoint;
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// The most triangles one displaced mesh may have. Two million is enough to
/// resolve a fine height map over a hand-sized part, and bounded well inside
/// what a scene can hold several of.
pub const TRIANGLE_BUDGET: usize = 2_000_000;

/// What computes a material's displacement: its program cut down to the
/// Displacement socket, and the register the height ends up in.
#[derive(Debug)]
pub struct Displacer {
    program: CompiledGraph,
    register: Register,
}

impl Displacer {
    /// `None` when the material does not drive Displacement.
    pub fn of(program: &CompiledGraph) -> Option<Displacer> {
        let register = program
            .driven
            .iter()
            .find(|(param, _)| *param == Param::Displacement)?
            .1;
        Some(Displacer {
            program: program.restricted_to(&[Param::Displacement]),
            register,
        })
    }

    /// Identifies the height field: the instructions that compute it and
    /// where the answer lands. An edit to the material that leaves these
    /// alone — a colour, a roughness — leaves the displaced mesh alone.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.program.instructions).hash(&mut hasher);
        self.register.0.hash(&mut hasher);
        hasher.finish()
    }

    /// The height at a point, in scene units.
    fn height(&self, point: &SurfacePoint, textures: &TextureTable, registers: &mut Registers, scratch: &mut Material) -> f32 {
        shade(&self.program, textures, point, registers, scratch);
        let h = registers.value(self.register).as_float();
        if h.is_finite() {
            h
        } else {
            0.0
        }
    }
}

/// `mesh`, subdivided and displaced.
///
/// `scale` is the part's size in the world per unit of its own coordinates,
/// and `detail` the edge length to subdivide to, in scene units.
pub fn displace(mesh: &RenderMesh, displacer: &Displacer, textures: &TextureTable, scale: f32, detail: f32) -> RenderMesh {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let fine = subdivide(mesh, detail / scale, TRIANGLE_BUDGET);
    let count = fine.positions.len();

    // One direction per position, so every vertex at a point moves together
    // and a seam cannot open.
    let welded = welded_normals(&fine);

    let heights: Vec<f32> = (0..count)
        .into_par_iter()
        .map_init(
            || (Registers::new(), Material::default()),
            |(registers, scratch), i| {
                let point = SurfacePoint {
                    position: fine.positions[i],
                    normal: welded[i],
                    uv: fine.uvs.get(i).copied().unwrap_or(Vec2::ZERO),
                    // Sampled at the spacing of the new vertices: finer
                    // detail than that cannot be represented, and a lookup
                    // that tried would alias.
                    footprint: (detail / scale) * 0.5,
                    scale,
                    uv_footprint: None,
                    curvature: 0.0,
                    occlusion: 0.0,
                    vertex_color: tequila_core::spectrum::Spectrum::ONE,
                    world: Default::default(),
                };
                displacer.height(&point, textures, registers, scratch)
            },
        )
        .collect();

    let positions: Vec<Vec3> = (0..count)
        .map(|i| fine.positions[i] + welded[i] * (heights[i] / scale))
        .collect();
    let normals = displaced_normals(&positions, &fine.indices, &fine.normals);

    let mut out = RenderMesh::new(positions, fine.indices.clone()).with_normals(normals);
    if fine.uvs.len() == count {
        out = out.with_uvs(fine.uvs.clone());
        if let Some(tangents) = tangent_frames(&out.positions, &out.normals, &out.uvs, &out.indices) {
            out.tangents = tangents;
        }
    }
    out.curvature = vertex_curvature(&out.positions, &out.normals, &out.indices);
    out
}

/// Split every triangle into four until no edge is longer than `target` (in
/// the mesh's own units) or another split would pass `budget` triangles.
///
/// Uniform rather than adaptive: every triangle splits the same number of
/// times, so a vertex added on an edge is shared by both triangles along it
/// and nothing is left with a T-junction to crack at.
pub fn subdivide(mesh: &RenderMesh, target: f32, budget: usize) -> RenderMesh {
    let mut positions = mesh.positions.clone();
    let mut normals = if mesh.normals.len() == mesh.positions.len() {
        mesh.normals.clone()
    } else {
        face_averaged_normals(&mesh.positions, &mesh.indices)
    };
    let mut uvs = if mesh.uvs.len() == mesh.positions.len() { mesh.uvs.clone() } else { Vec::new() };
    let mut colors = if mesh.colors.len() == mesh.positions.len() { mesh.colors.clone() } else { Vec::new() };
    let mut indices = mesh.indices.clone();

    let longest = |positions: &[Vec3], indices: &[u32]| {
        indices
            .chunks_exact(3)
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| (positions[a as usize] - positions[b as usize]).length())
            .fold(0.0f32, f32::max)
    };

    if target > 0.0 {
        while longest(&positions, &indices) > target && (indices.len() / 3) * 4 <= budget {
            let mut midpoint: HashMap<(u32, u32), u32> = HashMap::new();
            let mut next = Vec::with_capacity(indices.len() * 4);
            for t in indices.chunks_exact(3) {
                let mut mid = |a: u32, b: u32| -> u32 {
                    let key = (a.min(b), a.max(b));
                    *midpoint.entry(key).or_insert_with(|| {
                        let (i, j) = (key.0 as usize, key.1 as usize);
                        positions.push((positions[i] + positions[j]) * 0.5);
                        normals.push((normals[i] + normals[j]).try_normalize().unwrap_or(normals[i]));
                        if !uvs.is_empty() {
                            uvs.push((uvs[i] + uvs[j]) * 0.5);
                        }
                        if !colors.is_empty() {
                            colors.push((colors[i] + colors[j]) * 0.5);
                        }
                        (positions.len() - 1) as u32
                    })
                };
                let (a, b, c) = (t[0], t[1], t[2]);
                let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
                next.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
            }
            indices = next;
        }
    }

    let mut out = RenderMesh::new(positions, indices).with_normals(normals);
    if !uvs.is_empty() {
        out = out.with_uvs(uvs);
    }
    if colors.len() == out.positions.len() {
        out.colors = colors;
    }
    out
}

/// The average normal of every vertex at each position, handed to each of
/// them: the direction a point moves in, shared by all its copies.
fn welded_normals(mesh: &RenderMesh) -> Vec<Vec3> {
    let key = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    let mut sums: HashMap<[u32; 3], Vec3> = HashMap::new();
    for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
        *sums.entry(key(*p)).or_insert(Vec3::ZERO) += *n;
    }
    mesh.positions
        .iter()
        .zip(&mesh.normals)
        .map(|(p, n)| sums[&key(*p)].try_normalize().unwrap_or(*n))
        .collect()
}

/// Normals for the displaced surface: area-weighted face normals, shared by
/// the vertices at one position that were smooth before — a UV seam — and
/// kept apart where they were not — a hard edge stays hard.
fn displaced_normals(positions: &[Vec3], indices: &[u32], before: &[Vec3]) -> Vec<Vec3> {
    let group = |i: usize| {
        let (p, n) = (positions[i], before[i]);
        let q = |v: f32| (v * 1024.0).round() as i32;
        ([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()], [q(n.x), q(n.y), q(n.z)])
    };
    // Grouped by where the vertex was *before* it moved would be the same
    // groups, since vertices at one position moved together.
    let mut sums: HashMap<([u32; 3], [i32; 3]), Vec3> = HashMap::new();
    let keys: Vec<_> = (0..positions.len()).map(group).collect();
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let face = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        for v in [a, b, c] {
            *sums.entry(keys[v]).or_insert(Vec3::ZERO) += face;
        }
    }
    (0..positions.len())
        .map(|i| sums.get(&keys[i]).and_then(|n| n.try_normalize()).unwrap_or(before[i]))
        .collect()
}

fn face_averaged_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let face = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        for v in [a, b, c] {
            normals[v] += face;
        }
    }
    normals.into_iter().map(|n| n.try_normalize().unwrap_or(Vec3::Y)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
    use tequila_materials::material_type::MaterialType;

    fn plane(size: f32) -> RenderMesh {
        RenderMesh::new(
            vec![vec3(0.0, 0.0, 0.0), vec3(size, 0.0, 0.0), vec3(size, 0.0, size), vec3(0.0, 0.0, size)],
            vec![0, 2, 1, 0, 3, 2],
        )
        .with_normals(vec![Vec3::Y; 4])
        .with_uvs(vec![vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)])
    }

    /// A program whose Displacement is a pattern wired through a Bump node of
    /// `depth`.
    fn displacer(depth: f32, pattern: Option<tequila_texture::procedural::Procedural>) -> Displacer {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let procedural = pattern.expect("a varying height");
        let height = graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(procedural), seed: 1 }));
        let bump = graph.add_node(Node::new(NodeKind::Bump { depth }));
        graph.connect(height, bump, Socket::Input(0));
        let output = graph.output();
        graph.connect(bump, output, Socket::Param(Param::Displacement));
        let program = graph.compile().expect("compiles");
        Displacer::of(&program).expect("the material displaces")
    }

    /// Subdivision reaches the edge length asked for, four triangles for one
    /// each time, and shares the new vertex along a shared edge.
    #[test]
    fn subdivision_reaches_the_detail_and_shares_edges() {
        let coarse = plane(1.0);
        let fine = subdivide(&coarse, 0.1, TRIANGLE_BUDGET);
        let longest = fine
            .indices
            .chunks_exact(3)
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| (fine.positions[a as usize] - fine.positions[b as usize]).length())
            .fold(0.0f32, f32::max);
        assert!(longest <= 0.1, "longest edge {longest}");
        let levels = ((fine.indices.len() / 3) as f32 / 2.0).log(4.0);
        assert!((levels - levels.round()).abs() < 1e-4, "each level quadruples the triangles");
        // Shared edges share vertices: an n x n grid of quads has
        // (n + 1)^2 vertices; duplicates along the diagonal would add more.
        let n = 2f32.powi(levels.round() as i32) as usize;
        assert_eq!(fine.positions.len(), (n + 1) * (n + 1), "a shared edge was split twice");
    }

    /// The budget holds whatever the detail asks for.
    #[test]
    fn subdivision_stops_at_the_budget() {
        let fine = subdivide(&plane(1.0), 1e-6, 5000);
        assert!(fine.indices.len() / 3 <= 5000);
        assert!(fine.indices.len() / 3 > 5000 / 4, "stopped a level early");
    }

    /// A pattern's height moves the surface along its normal by the height,
    /// in scene units — also on a part scaled down fifty times.
    #[test]
    fn a_height_moves_the_surface_by_itself_in_scene_units() {
        use tequila_texture::procedural::{Axis, Procedural};
        // Straight contour lines along x: a height known at every point.
        let lines = Procedural::Contour { axis: Axis::X, frequency: 4.0, width: 0.5, softness: 1.0, distortion: 0.0 };
        let displacer = displacer(0.01, Some(lines));
        for scale in [1.0f32, 0.02] {
            // Half a metre across in the world, whatever the scale.
            let mesh = plane(0.5 / scale);
            let displaced = displace(&mesh, &displacer, &TextureTable::new(), scale, 0.02);
            let mut checked = 0;
            for p in &displaced.positions {
                let world_x = p.x * scale;
                let expected = lines.evaluate(vec3(world_x, 0.0, p.z * scale), 1) * 0.01;
                let moved = p.y * scale;
                assert!((moved - expected).abs() < 1e-4, "scale {scale}: at x {world_x} moved {moved}, expected {expected}");
                checked += 1;
            }
            assert!(checked > 100);
        }
    }

    /// A closed surface stays closed: every copy of a position moves alike,
    /// so a cube with split hard-edge vertices opens no cracks.
    #[test]
    fn displacement_opens_no_cracks() {
        use tequila_texture::procedural::Procedural;
        let noise = Procedural::catalog()[0];
        let displacer = displacer(0.05, Some(noise));
        let mut cube = crate::geometry::Mesh::new(
            vec![
                vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
                vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0),
            ],
            vec![0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5],
        );
        cube.generate_normals(30f32.to_radians());
        let mesh = RenderMesh::new(cube.positions.clone(), cube.indices.clone()).with_normals(cube.normals.clone());
        let displaced = displace(&mesh, &displacer, &TextureTable::new(), 1.0, 0.1);
        // Every original position's copies still coincide.
        let fine = subdivide(&mesh, 0.1, TRIANGLE_BUDGET);
        let mut at: HashMap<[u32; 3], Vec3> = HashMap::new();
        for (before, after) in fine.positions.iter().zip(&displaced.positions) {
            let key = [before.x.to_bits(), before.y.to_bits(), before.z.to_bits()];
            let first = *at.entry(key).or_insert(*after);
            assert!((first - *after).length() < 1e-6, "copies of {before:?} moved apart");
        }
        // And the surface did move.
        let moved = fine.positions.iter().zip(&displaced.positions).filter(|(a, b)| (**a - **b).length() > 1e-4).count();
        assert!(moved > fine.positions.len() / 2);
    }

    /// The fingerprint follows the height field and nothing else.
    #[test]
    fn the_fingerprint_ignores_what_does_not_displace() {
        use tequila_texture::procedural::Procedural;
        let noise = Procedural::catalog()[0];
        let a = displacer(0.01, Some(noise)).fingerprint();
        assert_eq!(a, displacer(0.01, Some(noise)).fingerprint());
        assert_ne!(a, displacer(0.02, Some(noise)).fingerprint(), "a deeper displacement kept its mesh");
    }
}
