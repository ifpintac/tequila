//! What every edit of a layout has in common: what it is refused for, and
//! the mesh it ends with.
//!
//! An edit works on the layout's UV vertices ([`super::UvLayout`]): it
//! moves them, or regroups the corners on them where it cuts or stitches.
//! The mesh it gives back has a vertex for each mesh vertex and UV vertex
//! its corners are on together: the mesh's own vertex where the corners on
//! it stay together, keeping its number, so a selection made before the edit
//! still names the same vertices after it, and a new vertex, after all the
//! others, for each place a cut parts them.

use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::Vec2;

/// Why a layout edit was refused. Nothing is changed when one is.
#[derive(Clone, Debug, PartialEq)]
pub enum UvError {
    /// The part has no such UV set.
    NoSuchSet(usize),
    /// The set does not cover the part's vertices, so it is not a layout.
    NotALayout(usize),
    NoSuchIsland(u32),
    NoSuchVertex(u32),
    /// The two vertices are not the ends of an edge of the part.
    NoSuchEdge([u32; 2]),
    /// Nothing was chosen to work on.
    NothingChosen,
    /// The edit would turn triangles over against the rest of their island:
    /// a fold, where artwork prints mirrored.
    WouldFold,
    /// The edit would flatten the layout to a line or a point.
    Degenerate,
    /// An island does not lie flat without folding, even projected: it
    /// needs cutting first.
    CannotFlatten(u32),
    /// The islands do not fit the region at any size, with that margin.
    NoRoom,
}

impl std::fmt::Display for UvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UvError::NoSuchSet(set) => write!(f, "the part has no UV set {set}"),
            UvError::NotALayout(set) => write!(f, "UV set {set} does not cover the part's vertices"),
            UvError::NoSuchIsland(island) => write!(f, "the layout has no island {island}"),
            UvError::NoSuchVertex(vertex) => write!(f, "the part has no vertex {vertex}"),
            UvError::NoSuchEdge([a, b]) => write!(f, "vertices {a} and {b} are not the ends of an edge of the part"),
            UvError::NothingChosen => write!(f, "no islands, vertices or edges were chosen"),
            UvError::WouldFold => write!(f, "that would fold the layout over on itself"),
            UvError::Degenerate => write!(f, "that would flatten the layout to a line"),
            UvError::CannotFlatten(island) => write!(f, "island {island} does not lie flat without folding: cut it first"),
            UvError::NoRoom => write!(f, "the islands do not fit there with that margin"),
        }
    }
}

impl std::error::Error for UvError {}

/// The layout of a set, or why there is none.
pub(crate) fn layout_of(mesh: &Mesh, set: usize) -> Result<super::UvLayout, UvError> {
    if set >= mesh.uv_sets.len() {
        return Err(UvError::NoSuchSet(set));
    }
    super::UvLayout::new(mesh, set).ok_or(UvError::NotALayout(set))
}

/// Islands named in an edit, checked and in order, each once.
pub(crate) fn chosen_islands(layout: &super::UvLayout, islands: &[u32]) -> Result<Vec<u32>, UvError> {
    let mut out = islands.to_vec();
    out.sort_unstable();
    out.dedup();
    if let Some(&bad) = out.iter().find(|&&i| i as usize >= layout.islands.len()) {
        return Err(UvError::NoSuchIsland(bad));
    }
    Ok(out)
}

/// The mesh with UV set `set` laid out anew: each corner on the UV vertex
/// `corner_vertex` gives it, each UV vertex at `uv`. Vertices split where
/// the corners on one are on two UV vertices; nothing else changes, but the
/// tangent frames where the set they follow, the first, moved.
pub(crate) fn finish(mesh: &Mesh, set: usize, corner_vertex: &[u32], uv: &[Vec2]) -> Mesh {
    let count = mesh.positions.len();
    let mut own = vec![u32::MAX; count];
    let mut extra: HashMap<(u32, u32), u32> = HashMap::new();
    let mut sources: Vec<u32> = Vec::new();
    let mut layout: Vec<Vec2> = mesh.uv_sets[set].clone();
    let mut indices = mesh.indices.clone();
    for (c, index) in indices.iter_mut().enumerate() {
        let v = mesh.indices[c];
        let g = corner_vertex[c];
        let slot = &mut own[v as usize];
        if *slot == u32::MAX {
            *slot = g;
            layout[v as usize] = uv[g as usize];
        } else if *slot != g {
            let next = (count + sources.len()) as u32;
            *index = *extra.entry((v, g)).or_insert_with(|| {
                sources.push(v);
                layout.push(uv[g as usize]);
                next
            });
        }
    }
    let mut out = mesh.clone();
    if !sources.is_empty() {
        let pick = |v: &u32| *v as usize;
        out.positions.extend(sources.iter().map(|v| mesh.positions[pick(v)]));
        if mesh.normals.len() == count {
            out.normals.extend(sources.iter().map(|v| mesh.normals[pick(v)]));
        }
        if mesh.colors.len() == count {
            out.colors.extend(sources.iter().map(|v| mesh.colors[pick(v)]));
        }
        if mesh.tangents.len() == count {
            out.tangents.extend(sources.iter().map(|v| mesh.tangents[pick(v)]));
        }
        for (i, other) in out.uv_sets.iter_mut().enumerate() {
            if i != set && other.len() == count {
                let values: Vec<Vec2> = sources.iter().map(|v| mesh.uv_sets[i][pick(v)]).collect();
                other.extend(values);
            }
        }
        out.indices = indices;
    }
    out.uv_sets[set] = layout;
    // The tangent frame follows the first set's layout.
    if set == 0 && !out.tangents.is_empty() && !out.generate_tangents() {
        out.tangents.clear();
    }
    out.recompute_bounds();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    #[test]
    fn a_vertex_on_two_places_is_split_and_the_rest_keep_their_numbers() {
        // Two triangles sharing an edge; their shared vertices put on two
        // UV vertices each, as a cut along the edge does.
        let positions = vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)];
        let mesh = Mesh::new(positions, vec![0, 1, 2, 0, 2, 3]).with_uvs(vec![Vec2::ZERO; 4]);
        let corner_vertex = [0, 1, 2, 3, 4, 5];
        let uv = [vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 1.5), vec2(1.0, 0.5), vec2(0.0, 0.5)];
        let out = finish(&mesh, 0, &corner_vertex, &uv);
        assert_eq!(out.positions.len(), 6);
        assert_eq!(&out.indices[..3], &[0, 1, 2], "the first triangle keeps its vertices");
        assert_eq!(out.indices[4], 5, "vertex 2 doubled for the second");
        for (c, &v) in out.indices.iter().enumerate() {
            assert_eq!(out.uv_sets[0][v as usize], uv[corner_vertex[c] as usize]);
            assert_eq!(out.positions[v as usize], mesh.positions[mesh.indices[c] as usize]);
        }
    }
}
