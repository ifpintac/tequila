//! A part's UV sets: added, copied, deleted and put in another order.
//!
//! A set is read by number - a label's or a texture's `Mapping.uv_set` -
//! so a change that renumbers them says, by [`SetChange::renumber`], which
//! number each old one has after it; the command that makes the change
//! renumbers the mappings of what the part wears with it.
//!
//! The first set is the one the tangent frames follow, and the one a
//! Blender-style Texture Coordinate node reads: a change that puts another
//! set first changes what those read.

use super::edit::UvError;
use crate::document::SceneDocument;
use crate::geometry::Mesh;
use crate::material::MaterialDefinition;
use crate::tree::{MaterialKey, NodeId};
use tequila_materials::graph::{MaterialGraph, NodeKind as GraphNode};
use tequila_texture::projection::{Mapping, Projection};

/// A change to a part's list of UV sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetChange {
    /// A new set at the end: the part unwrapped as `scene.unwrap_uvs` lays
    /// it out, a layout to start from that reads the right way and does not
    /// fold, where a projection of a closed part would lie over itself.
    Add,
    /// A copy of set `from`, put at `to` among them; those from there on
    /// move up one.
    Copy { from: usize, to: usize },
    /// Set `set` gone; those after it move down one.
    Delete { set: usize },
    /// Set `from` taken out and put back at `to`.
    Move { from: usize, to: usize },
}

impl SetChange {
    /// The number set `old` has after the change. A mapping on a deleted
    /// set reads the first set after it, which is what a part with no such
    /// set gave it anyway.
    pub fn renumber(&self, old: usize) -> usize {
        match *self {
            SetChange::Add => old,
            SetChange::Copy { to, .. } => if old >= to { old + 1 } else { old },
            SetChange::Delete { set } => {
                if old == set {
                    0
                } else if old > set {
                    old - 1
                } else {
                    old
                }
            }
            SetChange::Move { from, to } => {
                if old == from {
                    to
                } else if from < old && old <= to {
                    old - 1
                } else if to <= old && old < from {
                    old + 1
                } else {
                    old
                }
            }
        }
    }

    /// The mesh with its sets changed.
    pub fn apply(&self, mesh: &Mesh) -> Result<Mesh, UvError> {
        let count = mesh.uv_sets.len();
        let mut out = mesh.clone();
        let first_before = mesh.uv_sets.first().cloned();
        match *self {
            SetChange::Add => {
                // The unwrap puts its layout first, splitting vertices along
                // its seams and carrying the other sets through them; this
                // one goes last, and the first set stays first.
                let (mut unwrapped, _) = super::unwrap(mesh, &super::UnwrapOptions::default());
                let layout = unwrapped.uv_sets.remove(0);
                unwrapped.uv_sets.push(layout);
                // Tangent frames, where the part had them, for its first set.
                unwrapped.tangents.clear();
                if !mesh.tangents.is_empty() {
                    unwrapped.generate_tangents();
                }
                return Ok(unwrapped);
            }
            SetChange::Copy { from, to } => {
                if from >= count {
                    return Err(UvError::NoSuchSet(from));
                }
                if to > count {
                    return Err(UvError::NoSuchSet(to));
                }
                let copy = mesh.uv_sets[from].clone();
                out.uv_sets.insert(to, copy);
            }
            SetChange::Delete { set } => {
                if set >= count {
                    return Err(UvError::NoSuchSet(set));
                }
                out.uv_sets.remove(set);
            }
            SetChange::Move { from, to } => {
                if from >= count {
                    return Err(UvError::NoSuchSet(from));
                }
                if to >= count {
                    return Err(UvError::NoSuchSet(to));
                }
                let set = out.uv_sets.remove(from);
                out.uv_sets.insert(to, set);
            }
        }
        // The tangent frames follow the first set.
        if out.uv_sets.first() != first_before.as_ref() {
            if out.uv_sets.is_empty() {
                out.tangents.clear();
            } else if !mesh.tangents.is_empty() && !out.generate_tangents() {
                out.tangents.clear();
            }
        }
        Ok(out)
    }
}

/// The materials a part wears - its own, and every variant of a
/// multi-material it is in - with the mappings that read its UV sets by
/// number renumbered for `change`: those whose mappings change, as they
/// become.
///
/// Refused, naming it, for such a material another part wears too: the
/// other part's sets did not move, so no one numbering is right for both.
/// Unlinking it first gives this part its own. A material the part may be
/// given later - by a configurator, a Studio - is not looked at.
pub fn renumbered_materials(document: &SceneDocument, node: NodeId, change: SetChange) -> Result<Vec<(MaterialKey, MaterialDefinition)>, UvError> {
    let mut worn: Vec<MaterialKey> = document.tree.get(node).and_then(|n| n.material).into_iter().collect();
    for multi in document.multi_materials.iter().filter(|m| m.parts.contains(&node)) {
        worn.extend(multi.variants.iter().copied());
    }
    worn.sort_unstable();
    worn.dedup();
    let mut out = Vec::new();
    for key in worn {
        let Some(definition) = document.materials.get(key) else { continue };
        let mut renumbered = definition.clone();
        if !renumber(&mut renumbered.graph, change) {
            continue;
        }
        let elsewhere = document.tree.iter().any(|n| n.id != node && n.material == Some(key))
            || document.multi_materials.iter().any(|m| m.variants.contains(&key) && m.parts.iter().any(|&p| p != node));
        if elsewhere {
            return Err(UvError::SharedMaterial(definition.name.clone()));
        }
        out.push((key, renumbered));
    }
    Ok(out)
}

/// Every label's and texture's mapping in a graph that reads a UV set,
/// renumbered; whether any changed.
fn renumber(graph: &mut MaterialGraph, change: SetChange) -> bool {
    let mut changed = false;
    let mut follow = |mapping: &mut Mapping| {
        if mapping.projection == Projection::Uv {
            let set = change.renumber(mapping.uv_set);
            if set != mapping.uv_set {
                mapping.uv_set = set;
                changed = true;
            }
        }
    };
    for decal in &mut graph.decals {
        follow(&mut decal.mapping);
    }
    let mappings: Vec<_> = graph.nodes().filter(|(_, n)| matches!(n.kind, GraphNode::Mapping2d { .. })).map(|(id, _)| id).collect();
    for id in mappings {
        if let Some(GraphNode::Mapping2d { mapping }) = graph.node_mut(id).map(|n| &mut n.kind) {
            follow(mapping);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use tequila_core::math::vec2;

    fn with_sets(n: usize) -> Mesh {
        let mut mesh = cube();
        mesh.uv_sets = (0..n).map(|s| vec![vec2(s as f32, 0.0); mesh.positions.len()]).collect();
        mesh
    }

    #[test]
    fn sets_are_added_copied_deleted_and_moved() {
        let mesh = with_sets(3);
        let added = SetChange::Add.apply(&mesh).unwrap();
        assert_eq!(added.uv_sets.len(), 4);
        assert!(added.uv_sets[3].iter().all(|p| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y)));
        let copied = SetChange::Copy { from: 2, to: 0 }.apply(&mesh).unwrap();
        assert_eq!(copied.uv_sets.iter().map(|s| s[0].x).collect::<Vec<_>>(), vec![2.0, 0.0, 1.0, 2.0]);
        let deleted = SetChange::Delete { set: 1 }.apply(&mesh).unwrap();
        assert_eq!(deleted.uv_sets.iter().map(|s| s[0].x).collect::<Vec<_>>(), vec![0.0, 2.0]);
        let moved = SetChange::Move { from: 0, to: 2 }.apply(&mesh).unwrap();
        assert_eq!(moved.uv_sets.iter().map(|s| s[0].x).collect::<Vec<_>>(), vec![1.0, 2.0, 0.0]);
        assert_eq!(SetChange::Delete { set: 3 }.apply(&mesh), Err(UvError::NoSuchSet(3)));
        assert_eq!(SetChange::Move { from: 0, to: 3 }.apply(&mesh), Err(UvError::NoSuchSet(3)));
    }

    /// Every set that survives keeps being read where it went.
    #[test]
    fn renumbering_follows_each_set_to_its_new_place() {
        let mesh = with_sets(4);
        let changes = [
            SetChange::Add,
            SetChange::Copy { from: 1, to: 1 },
            SetChange::Copy { from: 3, to: 4 },
            SetChange::Delete { set: 0 },
            SetChange::Delete { set: 2 },
            SetChange::Move { from: 0, to: 3 },
            SetChange::Move { from: 3, to: 1 },
            SetChange::Move { from: 2, to: 2 },
        ];
        for change in changes {
            let out = change.apply(&mesh).unwrap();
            for old in 0..4 {
                if change == (SetChange::Delete { set: old }) {
                    continue;
                }
                let new = change.renumber(old);
                assert_eq!(out.uv_sets[new][0].x, old as f32, "{change:?}: set {old} went to {new}");
            }
        }
    }

    /// A new set is a layout to start from: unwrapped, reading the right way
    /// and without folds, and the sets the part had are as they were,
    /// corner by corner, with the first still first.
    #[test]
    fn a_new_set_is_an_unwrap_after_the_others() {
        let mut mesh = tube(16, 4);
        mesh.uv_sets = vec![(0..mesh.positions.len()).map(|i| vec2(i as f32 * 0.01, 0.5)).collect()];
        let out = SetChange::Add.apply(&mesh).unwrap();
        assert_eq!(out.uv_sets.len(), 2);
        let layout = crate::uv::UvLayout::new(&out, 1).unwrap();
        assert!(layout.folded_triangles().is_empty());
        assert!(layout.islands.iter().all(|i| !i.mirrored));
        for t in 0..out.triangle_count() {
            assert_eq!(out.triangle(t).map(|v| out.uv_sets[0][v as usize]), mesh.triangle(t).map(|v| mesh.uv_sets[0][v as usize]));
        }
        // And on a part with no sets, it is the first.
        mesh.uv_sets.clear();
        assert_eq!(SetChange::Add.apply(&mesh).unwrap().uv_sets.len(), 1);
    }
}
