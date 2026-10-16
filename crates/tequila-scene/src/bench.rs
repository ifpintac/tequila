//! The reference benchmark scenes of `docs/05` §4.1.
//!
//! `docs/01` §15 states every performance target "measured on the reference
//! benchmark scene", and calls them "requirements, asserted in CI, not
//! aspirations". A requirement measured against whatever scene happened to be
//! open is not a requirement, so the scenes are here, generated in code.
//!
//! # Why generated rather than checked in
//!
//! A benchmark scene has to be *identical* on every machine and in every
//! version, or a comparison over time measures the asset instead of the
//! renderer. A generated scene is byte-identical by construction, adds nothing
//! to the repository, and cannot be quietly re-exported by a different tool.
//! It also sidesteps §0.2's rule against bundling third-party content: nothing
//! here came from anywhere.
//!
//! The cost is that these are shapes, not products. What matters for the
//! numbers is the distribution of work — triangle count, instance count, how
//! many distinct materials the shader has to switch between, how much of the
//! frame is covered — and that can be built deliberately rather than found.

use crate::command::{reserve_keys, Command, ImportPayload};
use crate::demo::{ground_plane, to_document_mesh, uv_sphere};
use crate::geometry::Mesh;
use crate::material::MaterialDefinition;
use crate::tree::{NodeKind, SceneNode};
use crate::{CommandBus, SceneDocument};
use tequila_core::math::{vec2, vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::metals;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::material::Material;

/// The benchmark scenes that can be built today.
///
/// The rest of §4.1's table — `glass_jewelry`, `textured`, `interior`,
/// `animation` — measures features that do not exist yet. Adding empty
/// placeholders for them would produce numbers that look like measurements and
/// are not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchScene {
    /// §4.1: 200k triangles, 8 materials, 1 HDRI. "The interactivity
    /// baseline", and the scene §15's samples/sec target is stated against.
    SimpleProduct,
    /// §4.1: 5M triangles, 400 parts, heavy instancing. Built to exercise
    /// import, BVH construction and memory rather than shading.
    CadAssembly,
}

impl BenchScene {
    pub const ALL: [BenchScene; 2] = [BenchScene::SimpleProduct, BenchScene::CadAssembly];

    pub fn name(self) -> &'static str {
        match self {
            BenchScene::SimpleProduct => "simple_product",
            BenchScene::CadAssembly => "cad_assembly",
        }
    }

    pub fn from_name(name: &str) -> Option<BenchScene> {
        BenchScene::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn build(self) -> SceneDocument {
        match self {
            BenchScene::SimpleProduct => simple_product(),
            BenchScene::CadAssembly => cad_assembly(),
        }
    }
}

/// A box with rounded edges, tessellated by pushing a subdivided cube out onto
/// the rounded surface.
///
/// The shape products actually are, and the one that matters for a renderer:
/// the rounded edges are where the environment reflection reads, and they are
/// where most of the triangles go.
pub fn rounded_box(half: Vec3, radius: f32, subdivisions: usize) -> Mesh {
    let n = subdivisions.max(2);
    let radius = radius.min(half.x).min(half.y).min(half.z);
    let inner = half - Vec3::splat(radius);

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Six faces of a cube, each a grid, each vertex projected onto the rounded
    // surface: clamp the position into the inner box, then step out by the
    // corner radius along the direction from that clamped point.
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (-Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::Y, Vec3::X),
    ];

    for (axis, u, v) in faces {
        let base = positions.len() as u32;
        for j in 0..=n {
            for i in 0..=n {
                let (s, t) = (
                    i as f32 / n as f32 * 2.0 - 1.0,
                    j as f32 / n as f32 * 2.0 - 1.0,
                );
                let flat = axis * half.dot(axis.abs())
                    + u * s * u.dot(half.abs())
                    + v * t * v.dot(half.abs());
                let clamped = vec3(
                    flat.x.clamp(-inner.x, inner.x),
                    flat.y.clamp(-inner.y, inner.y),
                    flat.z.clamp(-inner.z, inner.z),
                );
                let outward = (flat - clamped).try_normalize().unwrap_or(axis);
                positions.push(clamped + outward * radius);
                normals.push(outward);
                uvs.push(vec2(i as f32 / n as f32, j as f32 / n as f32));
            }
        }
        let stride = (n + 1) as u32;
        for j in 0..n as u32 {
            for i in 0..n as u32 {
                let a = base + j * stride + i;
                // Counter-clockwise in (u, v), which each face's basis is
                // built to map onto its outward axis.
                indices.extend_from_slice(&[
                    a,
                    a + 1,
                    a + stride,
                    a + 1,
                    a + stride + 1,
                    a + stride,
                ]);
            }
        }
    }

    let mut mesh = Mesh::new(positions, indices);
    mesh.normals = normals;
    mesh.uv_sets = vec![uvs];
    mesh
}

/// A capped cylinder along +Y.
pub fn cylinder(radius: f32, height: f32, segments: usize) -> Mesh {
    let segments = segments.max(3);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();

    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let (c, s) = (a.cos(), a.sin());
        positions.push(vec3(c * radius, 0.0, s * radius));
        normals.push(vec3(c, 0.0, s));
        positions.push(vec3(c * radius, height, s * radius));
        normals.push(vec3(c, 0.0, s));
    }
    for i in 0..segments as u32 {
        let (b, next) = (i * 2, (i + 1) * 2);
        // Counter-clockwise from outside, so the geometric normals face out.
        indices.extend_from_slice(&[b, next + 1, next, b, b + 1, next + 1]);
    }

    // Caps, as fans around a centre vertex.
    for (y, up) in [(0.0f32, -Vec3::Y), (height, Vec3::Y)] {
        let centre = positions.len() as u32;
        positions.push(vec3(0.0, y, 0.0));
        normals.push(up);
        let rim = positions.len() as u32;
        for i in 0..=segments {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push(vec3(a.cos() * radius, y, a.sin() * radius));
            normals.push(up);
        }
        for i in 0..segments as u32 {
            // The rim runs anticlockwise about +Y, so a fan taken in rim order
            // faces -Y and has to be reversed for the top cap.
            if up.y > 0.0 {
                indices.extend_from_slice(&[centre, rim + i + 1, rim + i]);
            } else {
                indices.extend_from_slice(&[centre, rim + i, rim + i + 1]);
            }
        }
    }

    let mut mesh = Mesh::new(positions, indices);
    mesh.normals = normals;
    mesh
}

/// §4.1's `simple_product`: 200k triangles, 8 materials, 1 HDRI.
///
/// Built as a product rather than as a pile of spheres, because the numbers
/// depend on the shape of the work: most of the triangles are in the rounded
/// edges where reflections read, several distinct materials share the frame so
/// the integrator switches between them, and the object sits on a ground plane
/// that catches its shadow and bounces light back into it.
pub fn simple_product() -> SceneDocument {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();

    // Tuned to land near 200k triangles in total. Six faces of a subdivided
    // grid are 12n^2 triangles, so n = 96 gives about 110k for the body.
    let body = rounded_box(vec3(1.0, 0.14, 0.62), 0.08, 96);
    let bezel = rounded_box(vec3(0.92, 0.02, 0.55), 0.02, 64);
    let screen = rounded_box(vec3(0.88, 0.005, 0.51), 0.004, 40);
    let lens = to_document_mesh(&uv_sphere(0.055, 64, 32));
    let button = cylinder(0.028, 0.02, 48);
    let foot = cylinder(0.05, 0.02, 32);
    let ground = to_document_mesh(&ground_plane(40.0));

    let meshes = vec![body, bezel, screen, lens, button, foot, ground];
    // Eight, as §4.1 specifies: one per visible part plus the ground.
    let materials = vec![
        (
            "Body",
            MaterialType::Plastic,
            Material::plastic(Spectrum::from_srgb_encoded(vec3(0.16, 0.17, 0.19)), 0.28),
        ),
        ("Bezel", MaterialType::Metal, Material::metal(metals::ALUMINIUM, 0.14)),
        (
            "Screen",
            MaterialType::Plastic,
            Material::plastic(Spectrum::splat(0.02), 0.03),
        ),
        (
            "Lens",
            MaterialType::Glass,
            Material::plastic(Spectrum::splat(0.04), 0.02),
        ),
        (
            "Button",
            MaterialType::Plastic,
            Material::plastic(Spectrum::from_srgb_encoded(vec3(0.75, 0.22, 0.18)), 0.42),
        ),
        ("Trim", MaterialType::Metal, Material::metal(metals::GOLD, 0.22)),
        (
            "Foot",
            MaterialType::Diffuse,
            Material::diffuse(Spectrum::from_srgb_encoded(Vec3::splat(0.08))),
        ),
        (
            "Ground",
            MaterialType::Ground,
            Material::plastic(Spectrum::from_srgb_encoded(Vec3::splat(0.55)), 0.35),
        ),
    ];

    // Body, bezel, screen, lens, four buttons, four feet, ground, plus a root.
    let parts: Vec<(usize, usize, Transform)> = {
        let mut parts = vec![
            (0, 0, Transform::translation(vec3(0.0, 0.14, 0.0))),
            (1, 1, Transform::translation(vec3(0.0, 0.30, 0.0))),
            (2, 2, Transform::translation(vec3(0.0, 0.325, 0.0))),
            (3, 3, Transform::translation(vec3(0.72, 0.34, -0.36))),
        ];
        for (i, x) in [-0.62f32, -0.48, 0.48, 0.62].into_iter().enumerate() {
            // The last button is trim-coloured, so the frame holds all eight
            // materials at once.
            let material = if i == 3 { 5 } else { 4 };
            parts.push((4, material, Transform::translation(vec3(x, 0.28, 0.44))));
        }
        for (x, z) in [(-0.86f32, -0.5f32), (0.86, -0.5), (-0.86, 0.5), (0.86, 0.5)] {
            parts.push((5, 6, Transform::translation(vec3(x, 0.0, z))));
        }
        parts.push((6, 7, Transform::IDENTITY));
        parts
    };

    let (mesh_keys, material_keys, ids) =
        reserve_keys(&document, meshes.len(), materials.len(), parts.len() + 1);

    let mut root = SceneNode::new(ids[0], "Product", NodeKind::Group);
    root.children = ids[1..].to_vec();
    let mut nodes = vec![root];
    for (index, (mesh, material, transform)) in parts.iter().enumerate() {
        let mut node = SceneNode::new(
            ids[index + 1],
            format!("Part {}", index + 1),
            NodeKind::Part {
                mesh: mesh_keys[*mesh],
            },
        );
        node.parent = Some(ids[0]);
        node.transform = *transform;
        node.material = Some(material_keys[*material]);
        nodes.push(node);
    }

    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: mesh_keys.iter().copied().zip(meshes).collect(),
                textures: Vec::new(),
                materials: material_keys
                    .iter()
                    .copied()
                    .zip(
                        materials
                            .into_iter()
                            .map(|(name, material_type, material)| {
                                MaterialDefinition::new(name, material_type, material)
                            }),
                    )
                    .collect(),
                nodes,
                roots: vec![ids[0]],
            }),
        },
    )
    .expect("the benchmark scene is well formed");
    document
}

/// §4.1's `cad_assembly`: 5M triangles, 400 parts, heavy instancing.
///
/// The point is the *instancing*: §2.2's rule is that identical parts load as
/// one mesh and many transforms, and a benchmark that duplicated the geometry
/// would measure the wrong thing entirely — it would measure what the renderer
/// does when instancing has already failed.
///
/// So 400 parts share 20 distinct meshes. The five million triangles are what
/// the BVH must bound; a twentieth of that is what it must build.
pub fn cad_assembly() -> SceneDocument {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();

    // 20 meshes at ~12,500 triangles each, instanced 20 times: 5M triangles
    // across 400 parts.
    const DISTINCT: usize = 20;
    const COPIES: usize = 20;
    let meshes: Vec<Mesh> = (0..DISTINCT)
        .map(|i| to_document_mesh(&uv_sphere(0.4 + i as f32 * 0.01, 128, 49)))
        .collect();

    let materials = vec![
        ("Steel", MaterialType::Metal, Material::metal(metals::ALUMINIUM, 0.25)),
        (
            "Anodized",
            MaterialType::Paint,
            Material::plastic(Spectrum::from_srgb_encoded(vec3(0.2, 0.3, 0.6)), 0.3),
        ),
        (
            "Ground",
            MaterialType::Ground,
            Material::plastic(Spectrum::from_srgb_encoded(Vec3::splat(0.5)), 0.4),
        ),
    ];

    let parts = DISTINCT * COPIES;
    let (mesh_keys, material_keys, ids) =
        reserve_keys(&document, DISTINCT, materials.len(), parts + 1);

    let mut root = SceneNode::new(ids[0], "Assembly", NodeKind::Group);
    root.children = ids[1..].to_vec();
    let mut nodes = vec![root];

    for index in 0..parts {
        let (row, column) = (index / COPIES, index % COPIES);
        let mut node = SceneNode::new(
            ids[index + 1],
            format!("Part {index}"),
            NodeKind::Part {
                mesh: mesh_keys[row],
            },
        );
        node.parent = Some(ids[0]);
        node.transform = Transform::translation(vec3(
            (column as f32 - COPIES as f32 * 0.5) * 1.1,
            0.5,
            (row as f32 - DISTINCT as f32 * 0.5) * 1.1,
        ));
        node.material = Some(material_keys[index % 2]);
        nodes.push(node);
    }

    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: mesh_keys.iter().copied().zip(meshes).collect(),
                textures: Vec::new(),
                materials: material_keys
                    .iter()
                    .copied()
                    .zip(
                        materials
                            .into_iter()
                            .map(|(name, material_type, material)| {
                                MaterialDefinition::new(name, material_type, material)
                            }),
                    )
                    .collect(),
                nodes,
                roots: vec![ids[0]],
            }),
        },
    )
    .expect("the benchmark scene is well formed");
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4.1 states the scene's size, and a benchmark measured on something
    /// materially different is measuring something else. The tolerance is wide
    /// because the exact count depends on tessellation choices; an order of
    /// magnitude is what would invalidate the comparison.
    #[test]
    fn simple_product_is_the_size_the_specification_states() {
        let document = simple_product();
        let triangles = document.triangle_count();
        assert!(
            (140_000..=280_000).contains(&triangles),
            "simple_product has {triangles} triangles, against a stated 200k"
        );
        assert_eq!(document.materials.len(), 8, "§4.1 states 8 materials");
        document.validate().unwrap();
    }

    /// The frame has to *contain* all eight, or the benchmark measures a
    /// simpler scene than it claims.
    #[test]
    fn every_material_is_actually_used() {
        let document = simple_product();
        let used = document.referenced_materials();
        assert_eq!(
            used.len(),
            8,
            "only {} of 8 materials are assigned to anything",
            used.len()
        );
    }

    #[test]
    fn cad_assembly_is_five_million_triangles_across_four_hundred_parts() {
        let document = cad_assembly();
        let triangles = document.triangle_count();
        assert!(
            (4_000_000..=6_000_000).contains(&triangles),
            "cad_assembly has {triangles} triangles, against a stated 5M"
        );
        assert_eq!(
            document
                .tree
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Part { .. }))
                .count(),
            400
        );
        document.validate().unwrap();
    }

    /// The whole point of this scene. Storing 400 copies would measure what
    /// the renderer does when instancing has already failed.
    #[test]
    fn cad_assembly_instances_rather_than_duplicating() {
        let document = cad_assembly();
        assert_eq!(document.geometry.len(), 20, "the meshes were duplicated");
        // Which is what makes it affordable: a twentieth of the triangles are
        // actually stored.
        let stored: usize = document
            .geometry
            .iter()
            .map(|(_, mesh)| mesh.triangle_count())
            .sum();
        assert!(
            stored * 15 < document.triangle_count(),
            "stored {stored} against {} rendered",
            document.triangle_count()
        );
    }

    /// A benchmark that differs between runs measures the machine's mood. Both
    /// scenes are generated, so both must be identical every time.
    #[test]
    fn the_scenes_are_reproducible() {
        for scene in BenchScene::ALL {
            let a = scene.build();
            let b = scene.build();
            assert!(
                a.difference_from(&b).is_none(),
                "`{}` differs between builds: {:?}",
                scene.name(),
                a.difference_from(&b)
            );
        }
    }

    /// The rounded box is the shape most of the triangles are in, so its
    /// normals had better be right — a benchmark rendering black geometry
    /// would be very fast and completely useless.
    #[test]
    fn the_rounded_box_faces_outward() {
        let mesh = rounded_box(vec3(1.0, 0.5, 0.7), 0.15, 8);
        assert!(mesh.triangle_count() > 700);
        for t in 0..mesh.triangle_count() {
            let geometric = mesh.geometric_normal_scaled(t).try_normalize().unwrap();
            let [a, b, c] = mesh.triangle(t);
            let shading =
                (mesh.normals[a as usize] + mesh.normals[b as usize] + mesh.normals[c as usize])
                    .try_normalize()
                    .unwrap();
            assert!(
                geometric.dot(shading) > 0.0,
                "triangle {t} is wound against its vertex normals"
            );
        }
    }

    #[test]
    fn the_cylinder_faces_outward_and_is_capped() {
        let mesh = cylinder(0.5, 2.0, 16);
        for t in 0..mesh.triangle_count() {
            let geometric = mesh.geometric_normal_scaled(t).try_normalize().unwrap();
            let [a, b, c] = mesh.triangle(t);
            let shading =
                (mesh.normals[a as usize] + mesh.normals[b as usize] + mesh.normals[c as usize])
                    .try_normalize()
                    .unwrap();
            assert!(geometric.dot(shading) > 0.0, "triangle {t} is inside out");
        }
        // Walls plus two fans.
        assert_eq!(mesh.triangle_count(), 16 * 2 + 16 * 2);
    }
}
