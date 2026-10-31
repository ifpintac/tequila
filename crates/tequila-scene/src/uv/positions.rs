//! UV vertices put where an edit worked out they go: the one move every
//! tool that reshapes a layout - lining vertices up, laying a shape over
//! another - comes down to, where no single affine map does it.
//!
//! The vertices are named by their number in the layout ([`super::UvLayout`]
//! numbers its UV vertices in the order of the corners on them), which a
//! mesh keeps until an edit cuts or stitches it: the layout the command is
//! applied to is the one the numbers were read from, as a redo applies it
//! to the same mesh again.

use super::edit::{finish, layout_of, UvError};
use super::transform::{check_no_new_folds, islands_of};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// Set `set` with UV vertex `vertices[i]` at `uv[i]`, every corner on it
/// with it, and nothing else moved. Refused where the lists differ in
/// length, name a vertex the layout does not have, hold a place that is not
/// a number, or would fold an island over on itself.
pub fn set_positions(mesh: &Mesh, set: usize, vertices: &[u32], uv: &[[f32; 2]]) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    if vertices.is_empty() {
        return Err(UvError::NothingChosen);
    }
    if vertices.len() != uv.len() {
        return Err(UvError::Degenerate);
    }
    if let Some(&bad) = vertices.iter().find(|&&v| v as usize >= layout.uv_vertices.len()) {
        return Err(UvError::NoSuchVertex(bad));
    }
    if uv.iter().flatten().any(|c| !c.is_finite()) {
        return Err(UvError::Degenerate);
    }
    let mut places: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    for (&v, &[u, w]) in vertices.iter().zip(uv) {
        places[v as usize] = vec2(u, w);
    }
    let touched = islands_of(&layout, vertices);
    check_no_new_folds(&layout, &touched, &places)?;
    Ok(finish(mesh, set, &layout.corner_vertex, &places))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::UvLayout;

    #[test]
    fn vertices_go_where_they_are_told_and_nothing_else_moves() {
        let mesh = unwrapped(sheet(4, 4, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let middle = (0..layout.uv_vertices.len() as u32).find(|&v| (layout.uv_vertices[v as usize].uv - layout.islands[0].centre).length() < 1e-3).expect("a middle vertex");
        let to = layout.uv_vertices[middle as usize].uv + vec2(0.01, -0.005);
        let out = set_positions(&mesh, 0, &[middle], &[[to.x, to.y]]).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert_eq!(after.uv_vertices.len(), layout.uv_vertices.len(), "the same vertices, numbered alike");
        for (v, (a, b)) in layout.uv_vertices.iter().zip(&after.uv_vertices).enumerate() {
            let want = if v as u32 == middle { to } else { a.uv };
            assert!((b.uv - want).length() < 1e-6, "vertex {v}");
        }
    }

    #[test]
    fn a_bad_list_or_a_fold_is_refused() {
        let mesh = unwrapped(sheet(4, 4, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let count = layout.uv_vertices.len() as u32;
        assert_eq!(set_positions(&mesh, 0, &[0, 1], &[[0.0, 0.0]]), Err(UvError::Degenerate));
        assert_eq!(set_positions(&mesh, 0, &[count], &[[0.0, 0.0]]), Err(UvError::NoSuchVertex(count)));
        assert_eq!(set_positions(&mesh, 0, &[0], &[[f32::NAN, 0.0]]), Err(UvError::Degenerate));
        assert_eq!(set_positions(&mesh, 0, &[], &[]), Err(UvError::NothingChosen));
        let middle = (0..count).find(|&v| (layout.uv_vertices[v as usize].uv - layout.islands[0].centre).length() < 1e-3).unwrap();
        let far = layout.islands[0].max + vec2(0.5, 0.5);
        assert_eq!(set_positions(&mesh, 0, &[middle], &[[far.x, far.y]]), Err(UvError::WouldFold));
    }
}
