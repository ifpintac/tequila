//! UV layouts: a part's surface laid out flat, so artwork can be placed on
//! it anywhere - a label on an angled face, a silkscreen round a fillet -
//! where a planar projection can only look down an axis; and every edit of
//! a layout the UV editor makes.
//!
//! # The layout
//!
//! A layout is one of a mesh's UV sets: a place on the flat for each vertex.
//! [`UvLayout`] reads it as islands, seams, edges and UV vertices, with
//! what the editor draws and hit-tests (see [`layout`]). Every operation
//! here reads it the same way, and every one that changes a layout is a
//! function from a mesh to a new mesh - the part's, with one set laid out
//! anew - that the commands in `crate::command` give the part as a mesh of
//! its own, undoably.
//!
//! Coordinates are the texture's: u across the picture and v down it, as
//! its rows run and the renderer samples it. A layout reads the right way
//! when, seen from the front of the surface, u runs to the right and v
//! down; an unwrap always lays a part out so, and an island the other way
//! round is [`Island::mirrored`]. Angles are radians in those coordinates:
//! a positive turn takes u toward v, which the editor, drawing v down,
//! shows clockwise.
//!
//! # Seams are where the layout is split
//!
//! There is no seam attribute on a mesh. An edge of the part - between two
//! welded vertices, its points in 3D - is a **seam** where the faces on its
//! two sides are laid out apart: the UVs at one of its ends differ between
//! them. Where they agree, the faces are joined, and an island is the faces
//! so joined. So a layout says by itself where it is cut, a file from
//! another program brings its seams with it, and there is nothing to keep
//! in step: what a mesh splits its vertices for - its normals at a hard
//! edge - is no seam where the layout runs on across.
//!
//! Which makes a cut ([`cut`]) part the layout along its edges: the pieces
//! of an island it divides are moved a gap apart, and laid out flat again
//! if asked, which also opens a cut that does not divide its island, a
//! slit. A stitch ([`stitch`]) brings the two sides of seams together: the
//! smaller island turned, scaled and moved onto the larger, and the seam's
//! ends put exactly on each other.
//!
//! # Naming things in a command
//!
//! A command has to name what it works on so that the journal and a script
//! can say it again, and so is written in numbers that last:
//!
//! - an **island** by its number: islands are numbered in the order of
//!   their first triangle, which every edit that does not cut or stitch
//!   keeps;
//! - a **vertex** by its mesh vertex number: edits keep every vertex's
//!   number, and add new vertices after the others where a cut splits them;
//!   a vertex stands for every corner on the same place of the layout;
//! - an **edge** ([`MeshEdge`]) by the mesh vertices at its two ends, on
//!   either side of it: they are welded to find the edge of the part, so
//!   an edge picked on the layout, on one island, and an edge picked on the
//!   part in the viewport name the same edge.
//!
//! # Cost
//!
//! Making a [`UvLayout`] is a pass over the triangles and a sort of their
//! edges; the edits are a layout, a pass, and a copy of the mesh. On a part
//! of 200,000 triangles each takes a fraction of a second - the test
//! `uv::tests::a_large_part_stays_interactive` measures them - and hit
//! tests take microseconds. Flattening is the costly step: it solves for
//! every vertex of an island at once. Laying out again an island of 51,200
//! triangles takes about a third of a second, and unwrapping the whole
//! 204,800 as one chart a few seconds (`uv::tests::flattening_a_large_part`,
//! ignored by default, times them).

pub mod again;
pub mod angle_cut;
pub mod circle;
pub mod distortion;
pub mod edit;
pub mod export;
pub mod flatten;
pub mod grid;
pub mod layout;
pub mod least_stretch;
pub mod offset;
pub mod pack;
pub mod positions;
pub mod project;
pub mod rectangle;
pub mod reflatten;
pub mod relax;
pub mod seam_sides;
pub mod seams;
pub mod sets;
pub mod shape;
pub mod straight;
pub mod symmetry;
pub mod transform;
pub mod unwrap;
pub mod weld;

pub use again::{unwrap_again, AgainOptions, AgainReport, Direction, Keep};
pub use distortion::{distortion, heat, Distortion, Measure};
pub use edit::UvError;
pub use export::{layout_png, layout_svg, Fill, LayoutPicture};
pub use layout::{weld, EdgeKind, Island, LayoutEdge, SurfaceHit, TriangleHit, UvLayout, UvVertex, NO_ISLAND};
pub use least_stretch::{stretch_left, Method};
pub use pack::{pack, PackOptions, PackReport, UvRect};
pub use positions::set_positions;
pub use reflatten::reflatten;
pub use seams::{cut, stitch, CutOptions};
pub use sets::SetChange;
pub use shape::{Moves, Pick, ShapeError};
pub use transform::{even_density, straighten, transform, Alignment, DensityTarget, Pivot, StraightenBy, UvAffine, UvSelection};
pub use unwrap::{unwrap, UnwrapOptions, UnwrapReport};

/// An edge of the part, named by the mesh vertices at its two ends on
/// either side of it.
pub type MeshEdge = [u32; 2];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Mesh;
    use std::time::Instant;
    use tequila_core::math::{vec2, vec3, Vec2};

    /// A wavy sheet of 400 tiles, each its own island of 512 triangles laid
    /// out at a size of its own: 204,800 triangles in all, the size of a
    /// detailed CAD part.
    fn large_part() -> Mesh {
        let (tiles, quads) = (20usize, 16usize);
        let mut positions = Vec::new();
        let mut uv = Vec::new();
        let mut indices = Vec::new();
        for ty in 0..tiles {
            for tx in 0..tiles {
                let base = positions.len() as u32;
                let scale = 0.6 + ((tx * 7 + ty * 13) % 10) as f32 * 0.08;
                for r in 0..=quads {
                    for c in 0..=quads {
                        let (x, z) = ((tx * quads + c) as f32 / (tiles * quads) as f32, (ty * quads + r) as f32 / (tiles * quads) as f32);
                        positions.push(vec3(x, (x * 9.0).sin() * 0.03 + (z * 7.0).cos() * 0.03, z));
                        uv.push(vec2(c as f32, r as f32) * (scale / quads as f32 / tiles as f32) + vec2(tx as f32, ty as f32) * (1.2 / tiles as f32));
                    }
                }
                let at = |r: usize, c: usize| base + (r * (quads + 1) + c) as u32;
                for r in 0..quads {
                    for c in 0..quads {
                        indices.extend_from_slice(&[at(r, c), at(r + 1, c), at(r + 1, c + 1), at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
                    }
                }
            }
        }
        Mesh::new(positions, indices).with_uvs(uv)
    }

    fn timed<T>(what: &str, limit: f32, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        let seconds = start.elapsed().as_secs_f32();
        eprintln!("{what}: {:.1} ms", seconds * 1000.0);
        assert!(seconds < limit, "{what} took {seconds:.2} s");
        out
    }

    /// Every operation the editor runs as a person works, on a part of
    /// 200,000 triangles, each in well under a second (`--nocapture` prints
    /// the times). The limits are loose so a busy machine does not fail
    /// them; the times are what matters.
    #[test]
    fn a_large_part_stays_interactive() {
        let mesh = large_part();
        assert_eq!(mesh.triangle_count(), 204_800);
        let layout = timed("layout", 3.0, || UvLayout::new(&mesh, 0).unwrap());
        assert_eq!(layout.islands.len(), 400);
        let points: Vec<Vec2> = (0..10_000).map(|i| vec2((i % 100) as f32 / 80.0, (i / 100) as f32 / 80.0)).collect();
        let hits = timed("10,000 triangle hit tests", 1.0, || points.iter().filter(|p| layout.triangle_at(**p).is_some()).count());
        assert!(hits > 1000);
        timed("10,000 nearest vertices", 1.0, || points.iter().filter_map(|p| layout.nearest_vertex(*p, 0.01)).count());
        timed("10,000 nearest edges", 1.0, || points.iter().filter_map(|p| layout.nearest_edge(*p, 0.01)).count());
        timed("surface lookup", 0.5, || layout.surface_at(&mesh, layout.islands[37].centroid).unwrap());
        timed("faces to islands", 0.5, || layout.islands_of_faces(&(0..50_000).collect::<Vec<_>>()));
        let all: Vec<u32> = (0..400).collect();
        let moved = timed("transform 400 islands", 3.0, || transform(&mesh, 0, &UvSelection::Islands(all.clone()), &UvAffine::rotation(0.3), Pivot::Islands).unwrap());
        timed("straighten 400 islands", 3.0, || straighten(&moved, 0, &all, StraightenBy::Axis, Alignment::Nearest).unwrap());
        timed("even density", 3.0, || even_density(&mesh, 0, &all, DensityTarget::Average).unwrap());
        let (packed, report) = timed("pack 400 islands", 5.0, || pack(&mesh, 0, None, &PackOptions::default()).unwrap());
        eprintln!("pack coverage {:.3}", report.coverage);
        assert!(UvLayout::new(&packed, 0).unwrap().folded_triangles().is_empty());
        timed("distortion", 2.0, || distortion(&mesh, 0).unwrap());
        let svg = timed("SVG", 3.0, || layout_svg(&mesh, 0, &LayoutPicture::default()).unwrap());
        eprintln!("SVG {} kB", svg.len() / 1024);
        // Cutting a tile in two along its middle, and stitching it back.
        let tile = &layout.islands[0];
        let middle: Vec<MeshEdge> = layout.edges[tile.edges.clone()]
            .iter()
            .filter(|e| e.kind == EdgeKind::Inner)
            .filter(|e| {
                let [a, b] = e.vertices.map(|v| mesh.positions[v as usize].x);
                (a - b).abs() < 1e-7 && (a - 8.0 / 320.0).abs() < 1e-6
            })
            .map(|e| e.vertices)
            .collect();
        assert_eq!(middle.len(), 16);
        let parted = timed("cut", 3.0, || cut(&mesh, 0, &middle, &CutOptions::default()).unwrap());
        let parted_layout = UvLayout::new(&parted, 0).unwrap();
        assert_eq!(parted_layout.islands.len(), 401);
        // The tile's left half begins with its first triangle; its right
        // half with the first triangle of its first row's ninth quad.
        let halves = parted_layout.islands_of_faces(&[0, 16]);
        assert_eq!(halves.len(), 2);
        let seams = parted_layout.shared_seams(&halves);
        assert_eq!(seams.len(), 16);
        timed("stitch", 3.0, || stitch(&parted, 0, &seams).unwrap());
        timed("reflatten one island", 2.0, || reflatten(&mesh, 0, &[5], &[], None).unwrap());
    }

    /// The costly steps, timed rather than tested - every vertex of an
    /// island solved for at once: `cargo test --release -p tequila-scene
    /// --lib uv::tests::flattening_a_large_part -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn flattening_a_large_part() {
        let mesh = large_part();
        let (unwrapped, report) = timed("unwrap 204,800 triangles", 600.0, || unwrap(&mesh, &UnwrapOptions::default()));
        eprintln!("{} charts, coverage {:.3}", report.charts, report.coverage);
        let kept = timed("unwrap keeping 400 islands", 600.0, || unwrap(&mesh, &UnwrapOptions { respect_cuts: true, ..Default::default() }).1);
        eprintln!("{} charts kept", kept.charts);
        let all: Vec<u32> = (0..400).collect();
        timed("reflatten 400 islands", 600.0, || reflatten(&mesh, 0, &all, &[], None).unwrap());
        // One island of 51,200 triangles: a quarter of the part, laid out
        // as seen from above and then flattened whole.
        let seen = mesh.positions.iter().map(|p| vec2(p.x, p.z)).collect();
        let quarter = Mesh::new(mesh.positions.clone(), mesh.indices[..51_200 * 3].to_vec()).with_uvs(seen);
        let one = timed("reflatten one island of 51,200 triangles", 600.0, || reflatten(&quarter, 0, &[0], &[], None));
        eprintln!("{}", if one.is_ok() { "laid out" } else { "would fold" });
        let _ = unwrapped;
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use crate::geometry::Mesh;
    use tequila_core::math::{vec3, Vec3};

    /// A unit cube split at its creases, four vertices a face.
    pub fn cube() -> Mesh {
        let faces: [[Vec3; 4]; 6] = [
            [vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0)],
            [vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0)],
            [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0)],
            [vec3(1.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 1.0, 1.0)],
            [vec3(0.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
            [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)],
        ];
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for quad in faces {
            let base = positions.len() as u32;
            positions.extend_from_slice(&quad);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Mesh::new(positions, indices)
    }

    /// A grid of quads over a function of x and z: a patch of surface,
    /// facing up.
    pub fn sheet(rows: usize, columns: usize, height: impl Fn(f32, f32) -> f32) -> Mesh {
        let mut positions = Vec::new();
        for r in 0..=rows {
            for c in 0..=columns {
                let (x, z) = (c as f32 / columns as f32, r as f32 / rows as f32);
                positions.push(vec3(x, height(x, z), z));
            }
        }
        let mut indices = Vec::new();
        let at = |r: usize, c: usize| (r * (columns + 1) + c) as u32;
        for r in 0..rows {
            for c in 0..columns {
                indices.extend_from_slice(&[at(r, c), at(r + 1, c), at(r + 1, c + 1), at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
            }
        }
        Mesh::new(positions, indices)
    }

    /// An open tube round Y, radius 0.5 and height 1, its seam column of
    /// vertices doubled as a mesh with a layout has it, facing out.
    pub fn tube(segments: usize, rings: usize) -> Mesh {
        let mut positions = Vec::new();
        for r in 0..=rings {
            for s in 0..=segments {
                let angle = s as f32 / segments as f32 * std::f32::consts::TAU;
                positions.push(vec3(angle.cos() * 0.5, r as f32 / rings as f32, angle.sin() * 0.5));
            }
        }
        let at = |r: usize, s: usize| (r * (segments + 1) + s) as u32;
        let mut indices = Vec::new();
        for r in 0..rings {
            for s in 0..segments {
                indices.extend_from_slice(&[at(r, s), at(r + 1, s), at(r + 1, s + 1), at(r, s), at(r + 1, s + 1), at(r, s + 1)]);
            }
        }
        Mesh::new(positions, indices)
    }

    /// A mesh unwrapped with the usual options.
    pub fn unwrapped(mesh: Mesh) -> Mesh {
        super::unwrap(&mesh, &super::UnwrapOptions::default()).0
    }
}

/// The commands, end to end through the bus: the part gets a mesh of its
/// own with the layout, and undo and redo put it back exactly.
#[cfg(test)]
mod command_tests {
    use crate::split::tests_support::one_cube;
    use crate::tree::NodeKind;
    use crate::Command;

    fn mesh_of(document: &crate::SceneDocument, node: crate::NodeId) -> crate::MeshKey {
        match document.tree.get(node).unwrap().kind {
            NodeKind::Part { mesh } => mesh,
            ref other => panic!("not a part: {other:?}"),
        }
    }

    #[test]
    fn unwrapping_gives_the_part_a_laid_out_mesh_and_undo_takes_it_back() {
        let (mut document, mut bus, node) = one_cube();
        let before = mesh_of(&document, node);
        let triangles = document.geometry.get(before).unwrap().triangle_count();
        bus.apply(&mut document, Command::UnwrapUvs { node, options: Default::default() }).expect("it unwraps");
        let after = mesh_of(&document, node);
        assert_ne!(after, before, "a mesh of its own");
        let unwrapped = document.geometry.get(after).unwrap();
        assert_eq!(unwrapped.triangle_count(), triangles);
        assert!(!unwrapped.uv_sets.is_empty() && unwrapped.uv_sets[0].len() == unwrapped.positions.len());

        bus.undo(&mut document).expect("undo");
        assert_eq!(mesh_of(&document, node), before, "the old mesh back");
        assert!(document.geometry.get(after).is_none(), "and the unwrapped one gone");

        bus.redo(&mut document).expect("redo");
        assert_eq!(mesh_of(&document, node), after, "redo comes back under the same key");
        assert!(document.geometry.get(after).is_some());
    }

    #[test]
    fn only_a_part_unwraps() {
        let (mut document, mut bus, node) = one_cube();
        let parent = document.tree.get(node).unwrap().parent.unwrap();
        assert!(bus.apply(&mut document, Command::UnwrapUvs { node: parent, options: Default::default() }).is_err());
    }

    use super::*;
    use crate::command::{reserve_keys, ImportPayload};
    use crate::material::MaterialDefinition;
    use crate::tree::{MaterialKey, NodeId, SceneNode};
    use crate::{CommandBus, SceneDocument};
    use tequila_core::math::vec2;
    use tequila_texture::projection::{Mapping, Projection};

    /// Two parts sharing an unwrapped cube with three UV sets, each wearing
    /// a material of its own with a label on set 1 and a texture mapping on
    /// set 2; or both one material.
    fn two_parts(shared_material: bool) -> (SceneDocument, CommandBus, [NodeId; 2], [MaterialKey; 2]) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, keys, ids) = reserve_keys(&document, 1, 2, 3);
        let mut mesh = super::tests_support::unwrapped(super::tests_support::cube());
        let first = mesh.uv_sets[0].clone();
        mesh.uv_sets.push(first.iter().map(|p| *p * 0.5).collect());
        mesh.uv_sets.push(first.iter().map(|p| *p + vec2(1.0, 0.0)).collect());
        let mut materials = Vec::new();
        for (i, &key) in keys.iter().enumerate() {
            let mut definition = MaterialDefinition::new(format!("Paint {i}"), tequila_materials::material_type::MaterialType::Plastic, Default::default());
            let mut label = tequila_materials::decal::Decal::new("Logo", tequila_materials::graph::TextureId(1));
            label.mapping.uv_set = 1;
            definition.graph.decals.push(label);
            definition.graph.add_node(tequila_materials::graph::Node::new(tequila_materials::graph::NodeKind::Mapping2d {
                mapping: Mapping { projection: Projection::Uv, uv_set: 2, ..Default::default() },
            }));
            materials.push((key, definition));
        }
        let mut root = SceneNode::new(ids[0], "Model", crate::NodeKind::Group);
        root.children = vec![ids[1], ids[2]];
        let mut nodes = vec![root];
        for (i, id) in [ids[1], ids[2]].into_iter().enumerate() {
            let mut part = SceneNode::new(id, format!("Part {i}"), crate::NodeKind::Part { mesh: meshes[0] });
            part.parent = Some(ids[0]);
            part.material = Some(if shared_material { keys[0] } else { keys[i] });
            nodes.push(part);
        }
        let payload = ImportPayload { meshes: vec![(meshes[0], mesh)], textures: Vec::new(), materials, nodes, roots: vec![ids[0]] };
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).expect("import");
        bus.mark_saved();
        (document, bus, [ids[1], ids[2]], [keys[0], keys[1]])
    }

    fn mesh_key(document: &SceneDocument, node: NodeId) -> crate::MeshKey {
        mesh_of(document, node)
    }

    /// Every UV command, applied, undone and redone: the part gets a mesh of
    /// its own and the other part keeps theirs; undo puts back the mesh key,
    /// the geometry and the materials exactly; redo makes the same mesh
    /// under the same key; and no edit leaves a fold.
    #[test]
    fn every_uv_command_undoes_exactly_and_redoes_the_same() {
        let commands = |node: NodeId| {
            let (document, _, _, _) = two_parts(false);
            let mesh = document.geometry.get(mesh_key(&document, node)).unwrap();
            let layout = UvLayout::new(mesh, 0).unwrap();
            let seam = layout.edges.iter().find(|e| e.kind == EdgeKind::Seam).unwrap().vertices;
            let inner = layout.edges.iter().find(|e| e.kind == EdgeKind::Inner).unwrap().vertices;
            // An inner edge's end, nudged a hair along the edge.
            let inner_edge = layout.edges.iter().find(|e| e.kind == EdgeKind::Inner).unwrap();
            let [a, b] = inner_edge.ends.map(|v| layout.uv_vertices[v as usize].uv);
            let inner_vertex = inner_edge.ends[0];
            let nudged = a + (b - a) * 0.01;
            let inner_place = [nudged.x, nudged.y];
            vec![
                Command::CutUvs { node, set: 0, edges: vec![inner], reflatten: true },
                Command::StitchUvs { node, set: 0, edges: vec![seam] },
                Command::TransformUvs { node, set: 0, selection: UvSelection::Islands(vec![1, 3]), transform: UvAffine::quarter_turns(1), pivot: Pivot::Islands },
                Command::TransformUvs { node, set: 1, selection: UvSelection::Vertices(vec![inner[0]]), transform: UvAffine::translation(vec2(0.001, 0.0)), pivot: Pivot::Origin },
                Command::ReflattenUvs { node, set: 0, islands: vec![0, 2], pins: Vec::new(), repack: false },
                Command::PackUvs { node, set: 0, islands: Vec::new(), options: PackOptions { margin: 0.02, ..Default::default() } },
                Command::StraightenUvs { node, set: 0, islands: vec![0], by: StraightenBy::Edge(seam), to: Alignment::Vertical },
                Command::EvenUvDensity { node, set: 0, islands: vec![0, 1, 2, 3, 4, 5], target: DensityTarget::Island(2) },
                Command::AddUvSet { node },
                Command::CopyUvSet { node, from: 2, to: 0 },
                Command::DeleteUvSet { node, set: 0 },
                Command::MoveUvSet { node, from: 2, to: 0 },
                Command::SetUvPositions { node, set: 0, vertices: vec![inner_vertex], uv: vec![inner_place] },
                Command::UnwrapUvs { node, options: UnwrapOptions { respect_cuts: true, ..Default::default() } },
                Command::UnwrapUvs { node, options: UnwrapOptions { respect_cuts: true, in_place: Some(1), ..Default::default() } },
            ]
        };
        let (_, _, [node, _], _) = two_parts(false);
        for command in commands(node) {
            let (mut document, mut bus, [node, other], _) = two_parts(false);
            let name = command.name();
            let before = document.clone();
            let (key, other_key) = (mesh_key(&document, node), mesh_key(&document, other));
            bus.apply(&mut document, command.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let edited = mesh_key(&document, node);
            assert_ne!(edited, key, "{name}: a mesh of its own");
            assert_eq!(mesh_key(&document, other), other_key, "{name}: the other part keeps its mesh");
            let after = document.geometry.get(edited).unwrap().clone();
            assert_eq!(after.triangle_count(), before.geometry.get(key).unwrap().triangle_count());
            for set in 0..after.uv_sets.len() {
                assert_eq!(after.uv_sets[set].len(), after.positions.len(), "{name}: set {set} covers the vertices");
                let layout = UvLayout::new(&after, set).unwrap();
                assert!(layout.folded_triangles().is_empty(), "{name}: set {set} folds");
            }
            let materials_after: Vec<_> = document.materials.iter().map(|(k, m)| (k, m.clone())).collect();

            bus.undo(&mut document).unwrap();
            assert!(document.describes_the_same_scene_as(&before), "{name}: undo did not restore the document");
            assert_eq!(mesh_key(&document, node), key);
            assert!(document.geometry.get(edited).is_none(), "{name}: the edited mesh goes");
            assert_eq!(document.geometry.get(key), before.geometry.get(key));
            for (k, m) in before.materials.iter() {
                assert_eq!(document.materials.get(k), Some(m), "{name}: material {k:?} restored");
            }

            bus.redo(&mut document).unwrap();
            assert_eq!(mesh_key(&document, node), edited, "{name}: redo under the same key");
            assert_eq!(document.geometry.get(edited), Some(&after), "{name}: redo makes the same mesh");
            let materials_redone: Vec<_> = document.materials.iter().map(|(k, m)| (k, m.clone())).collect();
            assert_eq!(materials_redone, materials_after);
        }
    }

    fn mappings(document: &SceneDocument, material: MaterialKey) -> (usize, usize) {
        let graph = &document.materials.get(material).unwrap().graph;
        let texture = graph
            .nodes()
            .find_map(|(_, n)| match &n.kind {
                tequila_materials::graph::NodeKind::Mapping2d { mapping } => Some(mapping.uv_set),
                _ => None,
            })
            .unwrap();
        (graph.decals[0].mapping.uv_set, texture)
    }

    #[test]
    fn labels_and_textures_follow_their_sets_when_sets_move() {
        let (mut document, mut bus, [node, other], [mine, theirs]) = two_parts(false);
        assert_eq!(mappings(&document, mine), (1, 2));
        // Set 0 deleted: the label's set 1 is now 0, the texture's 2 is 1.
        bus.apply(&mut document, Command::DeleteUvSet { node, set: 0 }).unwrap();
        assert_eq!(mappings(&document, mine), (0, 1));
        assert_eq!(mappings(&document, theirs), (1, 2), "the other part's material is its own");
        let mesh = document.geometry.get(mesh_key(&document, node)).unwrap();
        assert_eq!(mesh.uv_sets.len(), 2);
        // Moved to the front: the texture's set goes first, the label's after.
        bus.apply(&mut document, Command::MoveUvSet { node, from: 1, to: 0 }).unwrap();
        assert_eq!(mappings(&document, mine), (1, 0));
        // A copy put first moves both up.
        bus.apply(&mut document, Command::CopyUvSet { node, from: 1, to: 0 }).unwrap();
        assert_eq!(mappings(&document, mine), (2, 1));
        // Every step undone, every mapping back.
        for _ in 0..3 {
            bus.undo(&mut document).unwrap();
        }
        assert_eq!(mappings(&document, mine), (1, 2));
        let _ = other;
    }

    #[test]
    fn a_material_other_parts_wear_is_not_renumbered_for_one() {
        let (mut document, mut bus, [node, _], [shared, _]) = two_parts(true);
        let before = document.clone();
        let error = bus.apply(&mut document, Command::DeleteUvSet { node, set: 0 }).unwrap_err();
        assert!(matches!(&error, crate::CommandError::Uv(UvError::SharedMaterial(name)) if name == "Paint 0"), "{error}");
        assert!(document.describes_the_same_scene_as(&before), "refused, nothing changed");
        assert_eq!(mappings(&document, shared), (1, 2));
        // A change that moves no set the material reads is fine.
        bus.apply(&mut document, Command::AddUvSet { node }).unwrap();
        bus.apply(&mut document, Command::DeleteUvSet { node, set: 3 }).unwrap();
    }

    #[test]
    fn a_refused_edit_or_a_locked_part_changes_nothing() {
        let (mut document, mut bus, [node, _], _) = two_parts(false);
        let before = document.clone();
        let fold = Command::TransformUvs { node, set: 0, selection: UvSelection::Islands(vec![0]), transform: UvAffine::uniform(0.0), pivot: Pivot::Islands };
        assert!(bus.apply(&mut document, fold).is_err());
        assert!(bus.apply(&mut document, Command::CutUvs { node, set: 7, edges: vec![[0, 1]], reflatten: true }).is_err());
        assert!(document.describes_the_same_scene_as(&before));
        assert!(!bus.can_undo() || bus.depth() == 1, "nothing journalled for the refusals");
        bus.apply(&mut document, Command::set_locked(node, true)).unwrap();
        let locked = bus.apply(&mut document, Command::AddUvSet { node });
        assert!(matches!(locked, Err(crate::CommandError::Locked(_))));
    }
}
