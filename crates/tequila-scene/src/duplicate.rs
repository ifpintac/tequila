//! Copies of parts and groups (Edit › Duplicate, Ctrl+D).
//!
//! A copy is new nodes and nothing else: each part of it shows the same mesh
//! and wears the same material as the part it was copied from, so a copy of
//! a million-triangle part costs a node, not another million triangles, and
//! the renderer draws the two as instances of one mesh. A group is copied
//! with everything under it. The copies go in as an import of those nodes
//! alone ([`Command::Import`](crate::Command::Import) with no meshes and no
//! materials), so one step to undo takes them away and leaves the meshes the
//! originals still use.

use crate::command::{reserve_keys, ImportPayload};
use crate::document::SceneDocument;
use crate::tree::NodeId;
use std::collections::{BTreeSet, HashMap};

/// The copies of `nodes`, each with everything under it, as an import to
/// apply, and the copies' own ids in the order of `nodes`. A node that is
/// under another of `nodes` comes with that one rather than twice. Each copy
/// sits beside its original, in the same place, its name marked as a copy.
/// `None` when none of `nodes` is in the document.
pub fn payload(document: &SceneDocument, nodes: &[NodeId]) -> Option<(ImportPayload, Vec<NodeId>)> {
    let tree = &document.tree;
    let chosen: BTreeSet<NodeId> = nodes.iter().copied().filter(|id| tree.contains(*id)).collect();
    let under_another = |id: NodeId| {
        let mut parent = tree.get(id).and_then(|n| n.parent);
        while let Some(p) = parent {
            if chosen.contains(&p) {
                return true;
            }
            parent = tree.get(p).and_then(|n| n.parent);
        }
        false
    };
    let mut roots: Vec<NodeId> = Vec::new();
    for &id in nodes {
        if chosen.contains(&id) && !under_another(id) && !roots.contains(&id) {
            roots.push(id);
        }
    }
    if roots.is_empty() {
        return None;
    }
    // Parent before child, as an import lists them.
    let originals: Vec<NodeId> = roots.iter().flat_map(|&root| tree.descendants_and_self(root)).collect();
    let (_, _, ids) = reserve_keys(document, 0, 0, originals.len());
    let copy_of: HashMap<NodeId, NodeId> = originals.iter().copied().zip(ids).collect();
    let copies = originals
        .iter()
        .filter_map(|old| {
            let mut node = tree.get(*old)?.clone();
            node.id = copy_of[old];
            node.parent = node.parent.map(|p| copy_of.get(&p).copied().unwrap_or(p));
            node.children = node.children.iter().filter_map(|c| copy_of.get(c).copied()).collect();
            // Not the file's part: an updated CAD file is matched to the
            // original alone (`crate::reimport`).
            node.path_key = None;
            // A pattern copied whole keeps being one, over its own copies.
            if let Some(pattern) = node.pattern.as_mut() {
                pattern.source = copy_of.get(&pattern.source).copied().unwrap_or(pattern.source);
                pattern.copies = pattern.copies.iter().filter_map(|c| copy_of.get(c).copied()).collect();
            }
            if roots.contains(old) {
                node.name = copy_name(&node.name);
            }
            Some(node)
        })
        .collect();
    let new_roots: Vec<NodeId> = roots.iter().map(|r| copy_of[r]).collect();
    Some((
        ImportPayload { meshes: Vec::new(), textures: Vec::new(), materials: Vec::new(), nodes: copies, roots: new_roots.clone() },
        new_roots,
    ))
}

/// A copy's name: the original's with "copy" after it, once.
fn copy_name(name: &str) -> String {
    if name.ends_with(" copy") || name.contains(" copy ") {
        name.to_string()
    } else {
        format!("{name} copy")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{reserve_keys, Command, ImportPayload};
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use crate::tree::{NodeKind, SceneNode};
    use crate::CommandBus;
    use tequila_core::math::{vec3, Transform, Vec3};

    /// A group holding two parts that share one mesh and one material.
    fn assembly() -> (SceneDocument, CommandBus, Vec<NodeId>) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mesh = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 3);
        let mut root = SceneNode::new(ids[0], "Housing", NodeKind::Group);
        root.children = ids[1..].to_vec();
        let mut nodes = vec![root];
        for (i, id) in ids[1..].iter().enumerate() {
            let mut node = SceneNode::new(*id, format!("Screw {i}"), NodeKind::Part { mesh: meshes[0] });
            node.parent = Some(ids[0]);
            node.transform = Transform::translation(vec3(i as f32, 0.0, 0.0));
            node.material = Some(materials[0]);
            node.path_key = Some(format!("Housing/Screw {i}"));
            nodes.push(node);
        }
        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: vec![(materials[0], MaterialDefinition::plastic("Steel"))],
                    nodes,
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        (document, bus, ids)
    }

    /// A part's copy is a node beside it, in the same place, showing the
    /// same mesh in the same material - no mesh or material is added - and
    /// one undo takes it away and leaves the original as it was.
    #[test]
    fn a_copy_of_a_part_shares_its_mesh_and_material() {
        let (mut document, mut bus, ids) = assembly();
        let meshes = document.geometry.len();
        let materials = document.materials.len();
        let (payload, copies) = payload(&document, &[ids[1]]).unwrap();
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        let (original, copy) = (document.tree.get(ids[1]).unwrap().clone(), document.tree.get(copies[0]).unwrap().clone());
        assert_eq!(copy.kind, original.kind, "the same mesh");
        assert_eq!(copy.material, original.material);
        assert_eq!(copy.transform, original.transform, "in the same place");
        assert_eq!(copy.parent, Some(ids[0]), "beside it, in its group");
        assert_eq!(copy.name, "Screw 0 copy");
        assert_eq!(copy.path_key, None, "a re-import matches the original alone");
        assert_eq!((document.geometry.len(), document.materials.len()), (meshes, materials), "nothing but a node was added");
        bus.undo(&mut document).unwrap();
        assert!(!document.tree.contains(copies[0]));
        assert!(document.tree.contains(ids[1]) && document.geometry.len() == meshes, "the original and its mesh stay");
    }

    /// A group is copied with everything under it, each copy under the
    /// group's copy; a part chosen with its group comes with the group, not
    /// twice.
    #[test]
    fn a_group_is_copied_whole_and_once() {
        let (mut document, mut bus, ids) = assembly();
        let (payload, copies) = payload(&document, &[ids[0], ids[2]]).unwrap();
        assert_eq!(copies.len(), 1, "the screw came with its group");
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        let group = document.tree.get(copies[0]).unwrap().clone();
        assert_eq!((group.name.as_str(), group.parent, group.children.len()), ("Housing copy", None, 2));
        for child in &group.children {
            let node = document.tree.get(*child).unwrap();
            assert_eq!(node.parent, Some(copies[0]));
            assert!(!ids.contains(child), "a new node, not the original");
            assert!(node.name.starts_with("Screw "), "children keep their names: {}", node.name);
        }
        assert_eq!(document.tree.get(ids[0]).unwrap().children.len(), 2, "the original group is untouched");
        assert!(payload_none(&document));
    }

    fn payload_none(document: &SceneDocument) -> bool {
        payload(document, &[NodeId(9_999)]).is_none()
    }
}
