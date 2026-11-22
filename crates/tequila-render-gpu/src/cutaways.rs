//! The scene's cutaway volumes on the card (`docs/07` R3, tier B's
//! cutaways): `tequila_render_core::cutaway`'s cutters as records
//! `transport.wgsl`'s `cut_judge` reads, packed in the lamps' links buffer
//! after the lamps' lists (no new binding): how many, then [`CUTTER_WORDS`]
//! words each - its shape, its cap's material, and its inverse transform's
//! three rows as f32 bits. `walk_ray`'s query steps past what they cut, and a
//! cutter's boundary inside a part it cuts is met as the part's cap, as
//! `cutaway::intersect` has it.
//!
//! A part's own closed mesh made a cutter (`CutterShape::Mesh`) follows the
//! records: its triangles, nine floats each, and its hierarchy's nodes,
//! eight words each (the box's corners, then for a leaf its first triangle
//! and count, for an interior node its right child and nought), as
//! `CutMesh` built them.

use tequila_render_core::cutaway::{Cutter, CutterShape};

/// [`Cutter::shape`] as the record has it.
pub const CUT_PLANE: u32 = 0;
pub const CUT_BOX: u32 = 1;
pub const CUT_SPHERE: u32 = 2;
pub const CUT_CYLINDER: u32 = 3;
pub const CUT_MESH: u32 = 4;
/// A cap of the cut part's own material (`Cutter::cap` none).
pub const CUT_OWN_CAP: u32 = u32::MAX;
/// The words a cutter's record takes: its shape, its cap, a mesh's
/// triangles' start and count and its nodes' start and count, two spare,
/// the inverse transform's three rows of four.
pub const CUTTER_WORDS: usize = 20;

/// Pack `cutters` onto the end of `words` and say where their block starts
/// (`FrameParams::cutters`): their count, then their records. Nought where
/// there are none; never nought where there are some, as the block follows
/// at least one word.
pub fn pack(cutters: &[Cutter], words: &mut Vec<u32>) -> u32 {
    if cutters.is_empty() {
        return 0;
    }
    if words.is_empty() {
        words.push(0);
    }
    let block = words.len();
    words.push(cutters.len() as u32);
    let start = words.len();
    words.resize(start + cutters.len() * CUTTER_WORDS, 0);
    for (i, cutter) in cutters.iter().enumerate() {
        let at = start + i * CUTTER_WORDS;
        words[at] = match &cutter.shape {
            CutterShape::Plane => CUT_PLANE,
            CutterShape::Box => CUT_BOX,
            CutterShape::Sphere => CUT_SPHERE,
            CutterShape::Cylinder => CUT_CYLINDER,
            CutterShape::Mesh(_) => CUT_MESH,
        };
        words[at + 1] = cutter.cap.map_or(CUT_OWN_CAP, |m| m.0);
        for (r, row) in cutter.transform.inverse.m.iter().take(3).enumerate() {
            for (c, v) in row.iter().enumerate() {
                words[at + 8 + r * 4 + c] = v.to_bits();
            }
        }
        if let CutterShape::Mesh(mesh) = &cutter.shape {
            let triangles = mesh.triangles();
            words[at + 2] = words.len() as u32;
            words[at + 3] = triangles.len() as u32;
            words.extend(triangles.iter().flatten().flat_map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]));
            let nodes_start = words.len() as u32;
            let mut nodes = 0u32;
            for (min, max, left, count) in mesh.nodes() {
                words.extend([min.x.to_bits(), min.y.to_bits(), min.z.to_bits(), max.x.to_bits(), max.y.to_bits(), max.z.to_bits(), left, count]);
                nodes += 1;
            }
            words[at + 4] = nodes_start;
            words[at + 5] = nodes;
        }
    }
    block as u32
}
