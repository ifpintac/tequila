//! Splitting one mesh into the separate shells it is made of.
//!
//! A model that is three parts does not always arrive as three parts. STL has
//! no concept of a part at all — it is a bag of triangles — and an OBJ with no
//! `g`/`o`/`usemtl` markers, or a glTF authored as a single primitive, arrives
//! the same way. The importer splits on whatever the *file* says (see
//! `tequila-import`), and when the file says nothing there is nothing to
//! split on.
//!
//! What is left is the geometry itself: a housing, a lens and a bezel that
//! never touch each other are three connected components of the triangle
//! graph, and finding them recovers the parts the file lost.
//!
//! # Welding is the whole problem
//!
//! Two triangles are adjacent when they share a vertex. The obvious
//! implementation compares vertex *indices*, and on the formats that need this
//! most it finds nothing at all: an STL writes three fresh vertices per
//! triangle, so no two triangles share an index and every triangle is its own
//! component. A model of two hundred thousand triangles becomes two hundred
//! thousand parts.
//!
//! So adjacency is by *position*, matched on the exact bits. Exact rather than
//! within a tolerance because a tolerance needs a scale to be a tolerance of,
//! the scale is the scene's and this code does not know it, and a wrong guess
//! either welds a gap shut or fails to weld a seam — both silently. Exporters
//! that duplicate a vertex write the same bytes for it, which is the case that
//! matters; where an exporter has genuinely moved a vertex by a hair, the two
//! surfaces were not sharing that vertex in the first place.

use crate::geometry::Mesh;
use tequila_core::math::Vec3;
use std::collections::HashMap;

/// Disjoint sets over the vertices, by index into the welded list.
struct Sets {
    parent: Vec<u32>,
}

impl Sets {
    fn new(count: usize) -> Sets {
        Sets {
            parent: (0..count as u32).collect(),
        }
    }

    fn find(&mut self, mut of: u32) -> u32 {
        while self.parent[of as usize] != of {
            // Path halving: shortens the chain as it walks it, which keeps
            // this near-constant without a second pass.
            let grandparent = self.parent[self.parent[of as usize] as usize];
            self.parent[of as usize] = grandparent;
            of = grandparent;
        }
        of
    }

    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a as usize] = b;
        }
    }
}

#[inline]
fn bits(v: f32) -> u32 {
    // Negative zero and positive zero are the same point, and an exporter that
    // writes one where another writes the other must not break a weld.
    if v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

/// How many separate shells a mesh is made of, without building them.
///
/// The menu needs this to say "Split into 3 parts" rather than "Split", and to
/// grey itself out when there is only one — an action that reports "the model
/// is already one part" after being clicked is an action that should have said
/// so before.
pub fn part_count(mesh: &Mesh) -> usize {
    components(mesh).len()
}

/// The triangles of each connected component, as indices into the mesh.
fn components(mesh: &Mesh) -> Vec<Vec<usize>> {
    let triangles = mesh.triangle_count();
    if triangles == 0 {
        return Vec::new();
    }

    // Weld: every distinct position gets one id, and every vertex maps to it.
    let mut welded: HashMap<[u32; 3], u32> = HashMap::with_capacity(mesh.positions.len());
    let mut vertex_to_weld = vec![0u32; mesh.positions.len()];
    for (v, position) in mesh.positions.iter().enumerate() {
        let key = [bits(position.x), bits(position.y), bits(position.z)];
        let next = welded.len() as u32;
        vertex_to_weld[v] = *welded.entry(key).or_insert(next);
    }

    let mut sets = Sets::new(welded.len());
    for t in 0..triangles {
        let [a, b, c] = mesh.triangle(t);
        let (a, b, c) = (
            vertex_to_weld[a as usize],
            vertex_to_weld[b as usize],
            vertex_to_weld[c as usize],
        );
        sets.union(a, b);
        sets.union(a, c);
    }

    // Group the triangles by the root of their first vertex. Keyed in first
    // -appearance order rather than by root id, so the parts come out in the
    // order they appear in the file — which is the order a person expects, and
    // is stable between runs where a hash map's order is not.
    let mut order: HashMap<u32, usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for t in 0..triangles {
        let root = sets.find(vertex_to_weld[mesh.triangle(t)[0] as usize]);
        let slot = *order.entry(root).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[slot].push(t);
    }
    groups
}

/// Split a mesh into one mesh per connected shell.
///
/// Returns `None` when there is nothing to split — one component, or none —
/// so a caller cannot accidentally replace a part with a group of one.
pub fn into_parts(mesh: &Mesh) -> Option<Vec<Mesh>> {
    let groups = components(mesh);
    if groups.len() < 2 {
        return None;
    }

    let mut parts = Vec::with_capacity(groups.len());
    for group in groups {
        // Each part keeps only the vertices its own triangles use, renumbered
        // from zero. Carrying the whole vertex list into every part would make
        // three copies of the model and leave each one's bounds wrong, which
        // breaks framing, the ground fit, and the BVH's split heuristic.
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv_sets: Vec<Vec<tequila_core::math::Vec2>> =
            mesh.uv_sets.iter().map(|_| Vec::new()).collect();
        let mut colors = Vec::new();
        let mut indices = Vec::with_capacity(group.len() * 3);

        for t in group {
            for v in mesh.triangle(t) {
                let next = positions.len() as u32;
                let slot = *remap.entry(v).or_insert_with(|| {
                    positions.push(mesh.positions[v as usize]);
                    if let Some(n) = mesh.normals.get(v as usize) {
                        normals.push(*n);
                    }
                    for (set, source) in uv_sets.iter_mut().zip(&mesh.uv_sets) {
                        if let Some(uv) = source.get(v as usize) {
                            set.push(*uv);
                        }
                    }
                    if let Some(c) = mesh.colors.get(v as usize) {
                        colors.push(*c);
                    }
                    next
                });
                indices.push(slot);
            }
        }

        let mut part = Mesh::new(positions, indices);
        // Authored normals are carried across, never regenerated. §6.1 point 3
        // is explicit: CAD tessellation carries correct normals and
        // regenerating them makes cylinders faceted.
        if normals.len() == part.positions.len() {
            part.normals = normals;
        }
        if colors.len() == part.positions.len() {
            part.colors = colors;
        }
        for set in uv_sets {
            if set.len() == part.positions.len() {
                part.uv_sets.push(set);
            }
        }
        part.recompute_bounds();
        parts.push(part);
    }
    Some(parts)
}

/// A name for each part, ordered as [`into_parts`] returns them.
///
/// Numbered rather than guessed at. A part's identity is not recoverable from
/// its geometry, and a name like "Lens" invented by the application would be a
/// claim it cannot support — the person doing the splitting knows which is
/// which and can rename them in the tree.
pub fn part_names(base: &str, count: usize) -> Vec<String> {
    // Sorted by size would put the housing first, which is tempting and wrong:
    // it makes the numbering change when the geometry does, so a saved project
    // that referred to "Part 2" now refers to something else.
    (1..=count).map(|i| format!("{base} part {i}")).collect()
}

/// Where a set of parts sits relative to each other, for a sanity check.
pub fn centre_of(mesh: &Mesh) -> Vec3 {
    mesh.bounds().center()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// A cube at `offset`, written the way an STL writes one: every triangle
    /// with its own three vertices, nothing shared.
    fn unwelded_cube(offset: Vec3, size: f32) -> (Vec<Vec3>, Vec<u32>) {
        let h = size * 0.5;
        let corners = [
            vec3(-h, -h, -h),
            vec3(h, -h, -h),
            vec3(h, h, -h),
            vec3(-h, h, -h),
            vec3(-h, -h, h),
            vec3(h, -h, h),
            vec3(h, h, h),
            vec3(-h, h, h),
        ];
        let faces = [
            [0, 1, 2, 3],
            [5, 4, 7, 6],
            [4, 0, 3, 7],
            [1, 5, 6, 2],
            [4, 5, 1, 0],
            [3, 2, 6, 7],
        ];
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for face in faces {
            for tri in [[face[0], face[1], face[2]], [face[0], face[2], face[3]]] {
                for corner in tri {
                    indices.push(positions.len() as u32);
                    positions.push(corners[corner] + offset);
                }
            }
        }
        (positions, indices)
    }

    pub(super) fn cubes_for_document(count: usize) -> Mesh {
        cubes(count)
    }

    fn cubes(count: usize) -> Mesh {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for i in 0..count {
            // Far enough apart that they share no vertex.
            let (p, idx) = unwelded_cube(vec3(i as f32 * 10.0, 0.0, 0.0), 1.0);
            let base = positions.len() as u32;
            positions.extend(p);
            indices.extend(idx.into_iter().map(|v| v + base));
        }
        Mesh::new(positions, indices)
    }

    /// The case the feature exists for, and the one an index-based
    /// implementation gets catastrophically wrong: three separate solids in one
    /// mesh, with no vertex shared *by index* anywhere — an STL, in other
    /// words. Index adjacency would find one component per triangle, which is
    /// thirty-six parts per cube.
    #[test]
    fn three_unwelded_solids_are_three_parts() {
        let mesh = cubes(3);
        assert_eq!(mesh.triangle_count(), 36);
        assert_eq!(part_count(&mesh), 3);

        let parts = into_parts(&mesh).expect("it splits");
        assert_eq!(parts.len(), 3);
        for (i, part) in parts.iter().enumerate() {
            assert_eq!(part.triangle_count(), 12, "part {i}");
            // Each keeps only its own vertices, renumbered.
            assert_eq!(part.positions.len(), 36, "part {i}");
            // And lands where its cube was, in file order.
            let centre = centre_of(part);
            assert!(
                (centre.x - i as f32 * 10.0).abs() < 1e-4,
                "part {i} centred at {centre:?}"
            );
        }
    }

    /// One solid stays one part, and says so by returning nothing rather than a
    /// group of one. A "split" that wrapped a part in a pointless group would
    /// be a change the user has to undo.
    #[test]
    fn a_single_solid_does_not_split() {
        let mesh = cubes(1);
        assert_eq!(part_count(&mesh), 1);
        assert!(into_parts(&mesh).is_none());
    }

    /// Two solids that touch at a single shared vertex are one part. This is
    /// the definition doing its job rather than a limitation: connected means
    /// connected, and a tolerance that guessed otherwise would split welded
    /// assemblies the user considers one thing.
    #[test]
    fn solids_that_touch_are_one_part() {
        let (mut positions, mut indices) = unwelded_cube(Vec3::ZERO, 1.0);
        let (second, second_indices) = unwelded_cube(vec3(1.0, 0.0, 0.0), 1.0);
        let base = positions.len() as u32;
        positions.extend(second);
        indices.extend(second_indices.into_iter().map(|v| v + base));
        // The cubes are one unit apart and one unit wide, so their faces meet
        // exactly and their corner vertices coincide.
        let mesh = Mesh::new(positions, indices);
        assert_eq!(part_count(&mesh), 1);
    }

    /// Splitting loses no geometry. Every triangle ends up in exactly one part,
    /// so the parts together are the model — not most of it.
    #[test]
    fn every_triangle_survives_exactly_once() {
        let mesh = cubes(4);
        let parts = into_parts(&mesh).expect("it splits");
        let total: usize = parts.iter().map(|p| p.triangle_count()).sum();
        assert_eq!(total, mesh.triangle_count());

        // And the parts cover the original's bounds between them.
        let mut union = tequila_core::math::Aabb::EMPTY;
        for part in &parts {
            union.extend(&part.bounds());
        }
        let original = mesh.bounds();
        assert!((union.min - original.min).length() < 1e-4);
        assert!((union.max - original.max).length() < 1e-4);
    }

    /// Authored normals travel with their vertices. §6.1 point 3: CAD
    /// tessellation carries correct normals and regenerating them makes
    /// cylinders faceted, so a split must move them rather than drop them.
    #[test]
    fn authored_normals_are_carried_across() {
        let mut mesh = cubes(2);
        // A recognisable normal per vertex, so a mix-up is visible.
        mesh.normals = mesh
            .positions
            .iter()
            .map(|p| p.try_normalize().unwrap_or(Vec3::Y))
            .collect();

        let parts = into_parts(&mesh).expect("it splits");
        for part in &parts {
            assert_eq!(part.normals.len(), part.positions.len());
            for (position, normal) in part.positions.iter().zip(&part.normals) {
                let expected = position.try_normalize().unwrap_or(Vec3::Y);
                assert!(
                    (*normal - expected).length() < 1e-5,
                    "a normal moved to the wrong vertex"
                );
            }
        }
    }

    /// An empty mesh is not a part and does not crash anything.
    #[test]
    fn an_empty_mesh_has_no_parts() {
        let mesh = Mesh::new(Vec::new(), Vec::new());
        assert_eq!(part_count(&mesh), 0);
        assert!(into_parts(&mesh).is_none());
    }

    /// Names are numbered in file order, and stay that way. Sorting by size
    /// would be friendlier once and wrong afterwards: the numbering would move
    /// when the geometry did, and a saved project that referred to "part 2"
    /// would refer to something else.
    #[test]
    fn names_are_numbered_in_order() {
        assert_eq!(
            part_names("Housing", 3),
            vec!["Housing part 1", "Housing part 2", "Housing part 3"]
        );
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use crate::command::{reserve_keys, ImportPayload};
    use crate::tree::{NodeId, NodeKind, SceneNode};
    use crate::{Command, CommandBus, SceneDocument};

    fn document_with(mesh: crate::geometry::Mesh) -> (SceneDocument, CommandBus, NodeId) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, _, ids) = reserve_keys(&document, 1, 0, 2);

        let mut root = SceneNode::new(ids[0], "Model", NodeKind::Group);
        root.children = vec![ids[1]];
        let mut part = SceneNode::new(ids[1], "Widget", NodeKind::Part { mesh: meshes[0] });
        part.parent = Some(ids[0]);

        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: vec![root, part],
                    roots: vec![ids[0]],
                }),
            },
        )
        .expect("import");
        (document, bus, ids[1])
    }

    pub fn three_cubes() -> (SceneDocument, CommandBus, NodeId) {
        document_with(super::tests::cubes_for_document(3))
    }

    pub fn one_cube() -> (SceneDocument, CommandBus, NodeId) {
        document_with(super::tests::cubes_for_document(1))
    }
}

/// The command, end to end through the bus.
///
/// The algorithm is tested above against meshes. This is about the *document*:
/// that the part keeps its identity, that the children inherit what they
/// should, and above all that undo and redo put things back exactly — a
/// destructive edit whose undo is approximate is worse than no undo, because
/// people trust it.
#[cfg(test)]
mod command_tests {
    use super::tests_support::*;
    use crate::tree::NodeKind;
    use crate::Command;

    #[test]
    fn splitting_turns_one_part_into_a_group_of_parts() {
        let (mut document, mut bus, node) = three_cubes();

        bus.apply(&mut document, Command::SplitIntoParts { node })
            .expect("it splits");

        let parent = document.tree.get(node).expect("the node survives");
        assert!(matches!(parent.kind, NodeKind::Group), "{:?}", parent.kind);
        assert_eq!(parent.children.len(), 3);
        assert_eq!(parent.name, "Widget", "the node was renamed");

        for child in &parent.children {
            let child = document.tree.get(*child).expect("a child");
            assert!(matches!(child.kind, NodeKind::Part { .. }));
            assert_eq!(child.parent, Some(node));
        }
        assert_eq!(document.geometry.len(), 4, "3 parts plus the original");
    }

    /// The children inherit the material, so splitting changes what the model
    /// is made of and not how it looks. Then each can be given its own — which
    /// is the entire point of splitting.
    #[test]
    fn the_parts_inherit_the_material() {
        let (mut document, mut bus, node) = three_cubes();
        let material = document
            .materials
            .insert(crate::material::MaterialDefinition::new(
                "Shell",
                tequila_materials::material_type::MaterialType::Plastic,
                Default::default(),
            ));
        bus.apply(
            &mut document,
            Command::AssignMaterial {
                nodes: vec![node],
                material,
            },
        )
        .expect("assign");

        bus.apply(&mut document, Command::SplitIntoParts { node })
            .expect("it splits");

        let children = document.tree.get(node).unwrap().children.clone();
        for child in children {
            assert_eq!(
                document.tree.get(child).unwrap().material,
                Some(material),
                "a part lost the material"
            );
        }
    }

    /// Undo restores the part exactly: the same node, the same mesh, no
    /// leftover children, and no orphaned geometry left in the store to be
    /// written into the file.
    #[test]
    fn undo_puts_the_part_back() {
        let (mut document, mut bus, node) = three_cubes();
        let before_geometry = document.geometry.len();
        let before_kind = document.tree.get(node).unwrap().kind.clone();
        let before_tree = document.tree.len();

        bus.apply(&mut document, Command::SplitIntoParts { node })
            .expect("it splits");
        bus.undo(&mut document).expect("undo");

        assert_eq!(document.tree.len(), before_tree, "nodes were left behind");
        assert_eq!(
            document.geometry.len(),
            before_geometry,
            "meshes were left behind"
        );
        let restored = document.tree.get(node).expect("the node is back");
        assert_eq!(restored.kind, before_kind);
        assert!(restored.children.is_empty());
        document.validate().expect("the document is still valid");
    }

    /// And redo reuses the same identifiers rather than allocating a second
    /// set. Fresh ones would leave the redone parts unreachable from anything
    /// that referred to them, and would grow the id space every time somebody
    /// pressed undo and redo.
    #[test]
    fn redo_reuses_the_same_identifiers() {
        let (mut document, mut bus, node) = three_cubes();

        bus.apply(&mut document, Command::SplitIntoParts { node })
            .expect("it splits");
        let first: Vec<_> = document.tree.get(node).unwrap().children.clone();
        let first_meshes: Vec<_> = document.geometry.keys().collect();

        bus.undo(&mut document).expect("undo");
        bus.redo(&mut document).expect("redo");

        let second: Vec<_> = document.tree.get(node).unwrap().children.clone();
        assert_eq!(first, second, "redo produced different nodes");
        let second_meshes: Vec<_> = document.geometry.keys().collect();
        assert_eq!(first_meshes, second_meshes, "redo produced different meshes");
        document.validate().expect("valid");
    }

    /// A mesh that is one shell refuses rather than wrapping the part in a
    /// pointless group the user then has to undo.
    #[test]
    fn a_single_shell_refuses_to_split() {
        let (mut document, mut bus, node) = one_cube();
        assert!(bus
            .apply(&mut document, Command::SplitIntoParts { node })
            .is_err());
        assert!(matches!(
            document.tree.get(node).unwrap().kind,
            NodeKind::Part { .. }
        ));
    }

    /// A group is not a part and cannot be split. Nor can a locked node —
    /// locking exists to stop exactly this kind of edit.
    #[test]
    fn only_an_unlocked_part_can_be_split() {
        let (mut document, mut bus, node) = three_cubes();
        let group = document
            .tree
            .get(node)
            .unwrap()
            .parent
            .expect("a root group");
        assert!(bus
            .apply(&mut document, Command::SplitIntoParts { node: group })
            .is_err());

        bus.apply(
            &mut document,
            Command::SetLocked {
                states: vec![(node, true)],
            },
        )
        .expect("lock");
        assert!(bus
            .apply(&mut document, Command::SplitIntoParts { node })
            .is_err());
    }
}
