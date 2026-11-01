//! Vertices of the layout put where a tool worked out they go.
//!
//! Most of the tools that reshape a layout - a path laid straight, an
//! island laid out as a grid or a rectangle, relaxed, welded, mirrored -
//! move its UV vertices to places no affine map gives. They work the places
//! out from the layout as it is, and this applies them: one edit, named by
//! the layout's own numbers for its UV vertices ([`super::UvLayout`]), which
//! the same mesh always numbers the same way, so the journal and a script
//! can say it again.
//!
//! A UV vertex moved onto another of the same welded vertex is not one any
//! more: where the two sides of an edge then agree, the layout is joined
//! there, as a stitch joins it (see [`super`]).

use super::edit::{finish, layout_of, UvError};
use super::transform::{check_no_new_folds, islands_of};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// The layout with some of its UV vertices - each by its number in
/// [`super::UvLayout::uv_vertices`] - put at new places; a vertex named
/// twice goes where it is named last. Refused where that would turn a
/// triangle over against its island, or flatten an island.
pub fn set_positions(mesh: &Mesh, set: usize, moves: &[(u32, Vec2)]) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    if moves.is_empty() {
        return Err(UvError::NothingChosen);
    }
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    for &(v, at) in moves {
        let slot = uv.get_mut(v as usize).ok_or(UvError::NoSuchVertex(v))?;
        if !at.is_finite() {
            return Err(UvError::Degenerate);
        }
        *slot = at;
    }
    let mut moved: Vec<u32> = moves.iter().map(|(v, _)| *v).collect();
    moved.sort_unstable();
    moved.dedup();
    check_no_new_folds(&layout, &islands_of(&layout, &moved), &uv)?;
    Ok(finish(mesh, set, &layout.corner_vertex, &uv))
}

/// [`set_positions`] with the vertices and their places in two lists, the
/// way the command carries them; refused where the lists differ in length.
pub fn set_positions_from_lists(mesh: &Mesh, set: usize, vertices: &[u32], uv: &[[f32; 2]]) -> Result<Mesh, UvError> {
    if vertices.len() != uv.len() {
        return Err(UvError::Degenerate);
    }
    let moves: Vec<(u32, Vec2)> = vertices.iter().zip(uv).map(|(&v, &[u, w])| (v, vec2(u, w))).collect();
    set_positions(mesh, set, &moves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::UvLayout;

    #[test]
    fn vertices_go_where_they_are_put_and_nothing_else_moves() {
        let mesh = unwrapped(sheet(3, 3, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // The vertex in the middle of the sheet, nudged.
        let middle = (0..layout.uv_vertices.len() as u32).min_by(|&a, &b| {
            let d = |v: u32| (layout.uv_vertices[v as usize].uv - layout.islands[0].centre).length();
            d(a).total_cmp(&d(b))
        });
        let middle = middle.unwrap();
        let to = layout.uv_vertices[middle as usize].uv + vec2(0.01, -0.005);
        let out = set_positions(&mesh, 0, &[(middle, to)]).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert_eq!(after.uv_vertices.len(), layout.uv_vertices.len());
        for (v, (a, b)) in layout.uv_vertices.iter().zip(&after.uv_vertices).enumerate() {
            let expected = if v as u32 == middle { to } else { a.uv };
            assert_eq!(b.uv, expected, "vertex {v}");
        }
        assert_eq!(out.positions.len(), mesh.positions.len(), "no vertex split");
    }

    #[test]
    fn a_fold_a_bad_number_or_nothing_is_refused() {
        let mesh = unwrapped(sheet(3, 3, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let corner = layout.islands[0].boundary[0][0];
        // A corner thrown across the whole island turns its triangle over.
        let across = layout.islands[0].centre * 2.0 - layout.uv_vertices[corner as usize].uv;
        assert_eq!(set_positions(&mesh, 0, &[(corner, across * 1.5)]), Err(UvError::WouldFold));
        assert_eq!(set_positions(&mesh, 0, &[(9999, Vec2::ZERO)]), Err(UvError::NoSuchVertex(9999)));
        assert_eq!(set_positions(&mesh, 0, &[(corner, vec2(f32::NAN, 0.0))]), Err(UvError::Degenerate));
        assert_eq!(set_positions(&mesh, 0, &[]), Err(UvError::NothingChosen));
        assert_eq!(set_positions(&mesh, 3, &[(corner, Vec2::ZERO)]), Err(UvError::NoSuchSet(3)));
        assert_eq!(set_positions_from_lists(&mesh, 0, &[corner, 0], &[[0.0, 0.0]]), Err(UvError::Degenerate));
    }

    /// Two islands of a cut sheet, the seam's sides put on each other: the
    /// layout joins them, as a stitch does.
    #[test]
    fn sides_of_a_seam_put_together_join() {
        let mesh = unwrapped(sheet(2, 4, |x, _| x * 0.2));
        let edges: Vec<[u32; 2]> = (0..mesh.triangle_count())
            .flat_map(|t| {
                let tri = mesh.triangle(t);
                (0..3).map(move |k| [tri[k], tri[(k + 1) % 3]])
            })
            .filter(|[a, b]| {
                let (pa, pb) = (mesh.positions[*a as usize], mesh.positions[*b as usize]);
                (pa.x - 0.5).abs() < 1e-5 && (pb.x - 0.5).abs() < 1e-5 && a < b
            })
            .collect();
        let parted = crate::uv::cut(&mesh, 0, &edges, &crate::uv::CutOptions { reflatten: false, gap: 0.05 }).unwrap();
        let layout = UvLayout::new(&parted, 0).unwrap();
        assert_eq!(layout.islands.len(), 2);
        // Every UV vertex of the second island moved by the gap between them.
        let (a, b) = (&layout.islands[0], &layout.islands[1]);
        let pairs: Vec<(u32, u32)> = a
            .uv_vertices
            .iter()
            .filter_map(|&va| b.uv_vertices.iter().find(|&&vb| layout.uv_vertices[vb as usize].weld == layout.uv_vertices[va as usize].weld).map(|&vb| (va, vb)))
            .collect();
        assert_eq!(pairs.len(), 3, "the seam's three vertices");
        let (va, vb) = pairs[0];
        let shift = layout.uv_vertices[va as usize].uv - layout.uv_vertices[vb as usize].uv;
        let moves: Vec<(u32, Vec2)> = b.uv_vertices.iter().map(|&v| (v, layout.uv_vertices[v as usize].uv + shift)).collect();
        let joined = set_positions(&parted, 0, &moves).unwrap();
        let after = UvLayout::new(&joined, 0).unwrap();
        assert_eq!(after.islands.len(), 1, "one island again");
        assert!(after.folded_triangles().is_empty());
    }
}
