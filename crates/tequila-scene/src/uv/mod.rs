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
//! every vertex at once, and a single island of a hundred thousand vertices
//! takes seconds.

pub mod distortion;
pub mod edit;
pub mod export;
pub mod flatten;
pub mod layout;
pub mod pack;
pub mod reflatten;
pub mod seams;
pub mod sets;
pub mod transform;
pub mod unwrap;

pub use distortion::{distortion, heat, Distortion, Measure};
pub use edit::UvError;
pub use export::{layout_png, layout_svg, Fill, LayoutPicture};
pub use layout::{weld, EdgeKind, Island, LayoutEdge, SurfaceHit, TriangleHit, UvLayout, UvVertex, NO_ISLAND};
pub use pack::{pack, PackOptions, PackReport, UvRect};
pub use reflatten::reflatten;
pub use seams::{cut, stitch, CutOptions};
pub use sets::SetChange;
pub use transform::{even_density, straighten, transform, Alignment, DensityTarget, Pivot, StraightenBy, UvAffine, UvSelection};
pub use unwrap::{unwrap, UnwrapOptions, UnwrapReport};

/// An edge of the part, named by the mesh vertices at its two ends on
/// either side of it.
pub type MeshEdge = [u32; 2];

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
}
