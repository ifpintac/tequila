//! Re-import of an updated model (`docs/03` §2.3).
//!
//! A CAD model changes weekly, and the work done on it in Tequila — the
//! materials, labels, visibility, model sets, variants and names — is bound to
//! its parts by `NodeId`. Importing the new version as a second model would
//! throw that work away; this keeps it. Each imported node carries a *path
//! key*, its place in the source file as `parent path / name # occurrence`,
//! and an update matches the new file's nodes to the old by it:
//!
//! - a matched node keeps its id, and so everything that refers to it, and
//!   keeps its name, material, visibility, lock and pivot. It takes the new
//!   file's geometry, its placement within the model (the CAD file is the
//!   truth about where a part sits) and its CAD attributes. The model's top
//!   nodes keep their placement in the scene, which is the user's.
//! - a node only the new file has is added where the file puts it.
//! - a node only the old model has is hidden rather than deleted, and listed,
//!   so a part renamed in CAD does not silently lose its material; the user
//!   decides whether it goes.
//!
//! Planning is pure — it reads the document and returns a command — so an
//! update is one undo step, journalled and scriptable like any other.

use crate::command::{Command, ImportPayload};
use crate::tree::{MaterialKey, NodeId, NodeKind, SceneNode, SceneTree};
use crate::SceneDocument;
use tequila_core::math::Transform;
use std::collections::{BTreeMap, BTreeSet};

/// The metadata key an import's top nodes carry: the file they came from, so
/// the model can be reloaded from it.
pub const SOURCE_FILE: &str = "Source file";

/// The key of a model's one top node. Not its name: a CAD package names the
/// top of an export after the document, and the document is renamed with
/// every "save as" — Fusion's `Finger Wrestler v2` became `Finger Wrestler
/// (v2~recovered)` between two revisions of the same three parts — so a key
/// built on it matched nothing.
pub const MODEL_KEY: &str = "(model)";

/// A node's path key, from its parent's, its name, and which of its
/// same-named siblings it is (from 1).
pub fn key(parent: Option<&str>, name: &str, occurrence: usize) -> String {
    match parent {
        Some(parent) => format!("{parent}/{name}#{occurrence}"),
        None => format!("{name}#{occurrence}"),
    }
}

/// Path keys for a list of nodes given as `(name, parent index)`, in any
/// order. Siblings are counted in list order. A list with one top node keys
/// it as [`MODEL_KEY`], and everything else relative to it.
pub fn keys_of(nodes: &[(String, Option<usize>)]) -> Vec<String> {
    let mut keys: Vec<Option<String>> = vec![None; nodes.len()];
    let tops: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].1.is_none()).collect();
    if let [top] = tops[..] {
        keys[top] = Some(MODEL_KEY.to_string());
    }
    let mut seen: BTreeMap<(Option<usize>, String), usize> = BTreeMap::new();
    let mut occurrence = vec![0; nodes.len()];
    for (i, (name, parent)) in nodes.iter().enumerate() {
        let count = seen.entry((*parent, name.clone())).or_insert(0);
        *count += 1;
        occurrence[i] = *count;
    }
    fn resolve(
        i: usize,
        nodes: &[(String, Option<usize>)],
        occurrence: &[usize],
        keys: &mut Vec<Option<String>>,
        depth: usize,
    ) -> String {
        if let Some(key) = &keys[i] {
            return key.clone();
        }
        // A cycle would be a malformed file; cut it rather than recurse forever.
        let parent = nodes[i]
            .1
            .filter(|&p| p < nodes.len() && depth < nodes.len())
            .map(|p| resolve(p, nodes, occurrence, keys, depth + 1));
        let made = key(parent.as_deref(), &nodes[i].0, occurrence[i]);
        keys[i] = Some(made.clone());
        made
    }
    (0..nodes.len())
        .map(|i| resolve(i, nodes, &occurrence, &mut keys, 0))
        .collect()
}

/// What an update does to one node the old and new files share.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeUpdate {
    pub node: NodeId,
    /// A part's new mesh, or a group.
    pub kind: NodeKind,
    /// Where it now sits in its parent; `None` leaves it where it is.
    pub transform: Option<Transform>,
    pub material: Option<MaterialKey>,
    pub metadata: BTreeMap<String, String>,
    pub path_key: Option<String>,
    /// Its CAD layers, which the file decides: unlike a material or a note,
    /// they are not the person's work for an update to keep.
    pub layers: Vec<String>,
}

/// An update, as a command carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReimportPayload {
    /// What the new file adds: every new mesh, the materials and textures the
    /// added nodes and adopted materials need, and the added nodes, whose
    /// parents may be existing nodes.
    pub added: ImportPayload,
    pub updates: Vec<NodeUpdate>,
    /// Nodes the new file no longer has, hidden.
    pub hidden: Vec<NodeId>,
}

/// What an update found, for the reconciliation report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReimportReport {
    pub matched: usize,
    /// The top-most added nodes' names.
    pub added: Vec<String>,
    /// The top-most nodes no longer in the file, which the update hides.
    pub removed: Vec<(NodeId, String)>,
}

/// A name without the copy number a CAD package appends: `Bottom v3 (1)`
/// is `Bottom v3`.
fn without_copy_number(name: &str) -> String {
    let trimmed = name.trim_end();
    if let Some(open) = trimmed.rfind(" (") {
        let inside = &trimmed[open + 2..];
        if let Some(number) = inside.strip_suffix(')') {
            if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
                return trimmed[..open].to_string();
            }
        }
    }
    trimmed.to_string()
}

/// The nodes of an earlier import under `roots`, by path key.
///
/// Stored keys where the import recorded them. A model imported before keys
/// were recorded has none, and its keys are worked out from the tree as it
/// stands — right unless it has been renamed or regrouped since. Nodes the
/// user added (lights, cutaways, and in a keyed import any node without a
/// key) take no part.
fn existing(tree: &SceneTree, roots: &[NodeId]) -> BTreeMap<String, NodeId> {
    let keyed = roots
        .iter()
        .flat_map(|&r| tree.descendants_and_self(r))
        .any(|n| tree.get(n).is_some_and(|n| n.path_key.is_some()));
    let mut out = BTreeMap::new();
    let mut stack: Vec<(NodeId, Option<String>, usize)> = Vec::new();
    let mut root_seen: BTreeMap<String, usize> = BTreeMap::new();
    for &root in roots {
        let Some(node) = tree.get(root) else { continue };
        let count = root_seen.entry(node.name.clone()).or_insert(0);
        *count += 1;
        stack.push((root, None, *count));
    }
    let single = roots.len() == 1;
    while let Some((id, parent_key, occurrence)) = stack.pop() {
        let Some(node) = tree.get(id) else { continue };
        if !matches!(node.kind, NodeKind::Part { .. } | NodeKind::Group) {
            continue;
        }
        let own = match (&node.path_key, keyed) {
            (Some(stored), _) => Some(stored.clone()),
            (None, false) if single && roots.contains(&id) => Some(MODEL_KEY.to_string()),
            (None, false) => Some(key(parent_key.as_deref(), &node.name, occurrence)),
            (None, true) => None,
        };
        if let Some(own) = &own {
            out.entry(own.clone()).or_insert(id);
        }
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for &child in &node.children {
            let Some(child_node) = tree.get(child) else { continue };
            let count = seen.entry(child_node.name.clone()).or_insert(0);
            *count += 1;
            stack.push((child, own.clone(), *count));
        }
    }
    out
}

/// The import's top nodes that `node` belongs to: its top-most ancestor that
/// records a source file, with that ancestor's siblings from the same file
/// (a file with several top nodes imports as several). Without a recorded
/// file, the node's top-most ancestor alone.
pub fn import_roots(document: &SceneDocument, node: NodeId) -> Vec<NodeId> {
    let tree = &document.tree;
    let mut top = node;
    let mut sourced = None;
    let mut at = Some(node);
    while let Some(id) = at {
        let Some(n) = tree.get(id) else { break };
        top = id;
        if n.metadata.contains_key(SOURCE_FILE) {
            sourced = Some(id);
        }
        at = n.parent;
    }
    let Some(root) = sourced else {
        return vec![top];
    };
    let file = tree.get(root).and_then(|n| n.metadata.get(SOURCE_FILE)).cloned();
    let parent = tree.get(root).and_then(|n| n.parent);
    let siblings: Vec<NodeId> = match parent {
        Some(p) => tree.get(p).map(|n| n.children.clone()).unwrap_or_default(),
        None => tree.roots().to_vec(),
    };
    siblings
        .into_iter()
        .filter(|&s| tree.get(s).and_then(|n| n.metadata.get(SOURCE_FILE)) == file.as_ref())
        .collect()
}

/// The file an import came from, if it recorded one.
pub fn source_file(document: &SceneDocument, roots: &[NodeId]) -> Option<String> {
    roots
        .iter()
        .find_map(|&r| document.tree.get(r)?.metadata.get(SOURCE_FILE).cloned())
}

/// Plan the update of the import under `roots` to `new`, an import of the
/// new file built against `document` (with the old roots' parent as its
/// parent, and path keys on its nodes).
pub fn plan(document: &SceneDocument, roots: &[NodeId], new: ImportPayload) -> (Command, ReimportReport) {
    let tree = &document.tree;
    let old = existing(tree, roots);
    let old_roots: BTreeSet<NodeId> = roots.iter().copied().collect();

    // New node → the old node it becomes, by key; each old node taken once.
    let mut matched: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for node in &new.nodes {
        if let Some(&old_id) = node.path_key.as_ref().and_then(|k| old.get(k)) {
            if taken.insert(old_id) {
                matched.insert(node.id, old_id);
            }
        }
    }

    // Then by name, for what moved within the model: a part put into a
    // subassembly, or renumbered by the CAD package (`Bottom v3 (1)` in one
    // revision, `Bottom v3` in the next). Only a name that is unique among
    // the unmatched on both sides, parts with parts and groups with groups.
    for normalise in [(|name: &str| name.to_string()) as fn(&str) -> String, without_copy_number] {
        let mut olds: BTreeMap<(String, bool), Vec<NodeId>> = BTreeMap::new();
        for &id in old.values() {
            if taken.contains(&id) {
                continue;
            }
            if let Some(node) = tree.get(id) {
                olds.entry((normalise(&node.name), node.mesh().is_some())).or_default().push(id);
            }
        }
        let mut news: BTreeMap<(String, bool), Vec<NodeId>> = BTreeMap::new();
        for node in &new.nodes {
            if !matched.contains_key(&node.id) {
                news.entry((normalise(&node.name), node.mesh().is_some())).or_default().push(node.id);
            }
        }
        for node in &new.nodes {
            let name = (normalise(&node.name), node.mesh().is_some());
            if let (Some([new_id]), Some([old_id])) = (news.get(&name).map(|v| &v[..]), olds.get(&name).map(|v| &v[..])) {
                if *new_id == node.id && !matched.contains_key(new_id) && taken.insert(*old_id) {
                    matched.insert(*new_id, *old_id);
                }
            }
        }
    }

    let mut report = ReimportReport { matched: matched.len(), ..Default::default() };
    let mut updates = Vec::new();
    let mut added_nodes: Vec<SceneNode> = Vec::new();
    let added_ids: BTreeSet<NodeId> = new.nodes.iter().map(|n| n.id).filter(|id| !matched.contains_key(id)).collect();
    // Materials the result uses from the new file.
    let mut adopted: BTreeSet<MaterialKey> = BTreeSet::new();
    // A part the file now splits into a group of pieces - one per colour,
    // once a face of it is coloured in the CAD package - and the material it
    // had, which its pieces take: a part's material wins over the file's on
    // an update, and left on the group, which draws nothing, the part came
    // out in the file's colours.
    let mut split: BTreeMap<NodeId, MaterialKey> = BTreeMap::new();

    for node in &new.nodes {
        if let Some(&old_id) = matched.get(&node.id) {
            let Some(before) = tree.get(old_id) else { continue };
            let material = match node.kind {
                NodeKind::Part { .. } => before.material.or(node.material),
                _ => before.material,
            };
            if material.is_some() && material == node.material && before.material.is_none() {
                adopted.extend(material);
            }
            if let (NodeKind::Group, Some(NodeKind::Part { .. }), Some(material)) = (&node.kind, Some(&before.kind), before.material) {
                split.insert(node.id, material);
            }
            let mut metadata = before.metadata.clone();
            metadata.extend(node.metadata.clone());
            updates.push(NodeUpdate {
                node: old_id,
                kind: node.kind.clone(),
                transform: (!old_roots.contains(&old_id)).then_some(node.transform),
                material,
                metadata,
                path_key: node.path_key.clone(),
                layers: node.layers.clone(),
            });
        } else {
            let mut added = node.clone();
            // A new node under a matched one goes under the node it matched.
            added.parent = node.parent.map(|p| matched.get(&p).copied().unwrap_or(p));
            added.children = node.children.iter().copied().filter(|c| added_ids.contains(c)).collect();
            if let (Some(material), Some(_)) = (node.parent.and_then(|p| split.get(&p)), node.mesh()) {
                added.material = Some(*material);
                split.insert(node.id, *material);
            } else if let (Some(&material), None) = (node.parent.and_then(|p| split.get(&p)), node.mesh()) {
                // A group of pieces inside it hands the material down too.
                split.insert(node.id, material);
            }
            adopted.extend(added.material);
            if !added.parent.is_some_and(|p| added_ids.contains(&p)) {
                report.added.push(added.name.clone());
            }
            added_nodes.push(added);
        }
    }
    let added_roots: Vec<NodeId> = added_nodes
        .iter()
        .filter(|n| !n.parent.is_some_and(|p| added_ids.contains(&p)))
        .map(|n| n.id)
        .collect();

    // What the old model has and the new file does not: hidden, top-most
    // only, and never a node with a matched part inside it.
    let kept: BTreeSet<NodeId> = matched.values().copied().collect();
    let gone: BTreeSet<NodeId> = old.values().copied().filter(|id| !kept.contains(id)).collect();
    let mut hidden = Vec::new();
    for &id in &gone {
        let Some(node) = tree.get(id) else { continue };
        if node.parent.is_some_and(|p| gone.contains(&p)) {
            continue;
        }
        if tree.descendants_and_self(id).iter().any(|d| kept.contains(d)) {
            continue;
        }
        report.removed.push((id, node.name.clone()));
        if node.visible {
            hidden.push(id);
        }
    }

    let materials: Vec<_> = new.materials.into_iter().filter(|(k, _)| adopted.contains(k)).collect();
    let wanted_textures: BTreeSet<_> = materials
        .iter()
        .filter_map(|(_, d)| d.graph.compile().ok())
        .flat_map(|p| p.requirements.textures.into_iter().chain(p.requirements.volumes))
        .collect();
    let textures = new.textures.into_iter().filter(|(id, _)| wanted_textures.contains(id)).collect();
    let payload = ReimportPayload {
        added: ImportPayload {
            meshes: new.meshes,
            textures,
            materials,
            nodes: added_nodes,
            roots: added_roots,
        },
        updates,
        hidden,
    };
    (Command::Reimport { payload: Box::new(payload) }, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{reserve_keys, CommandBus};
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use tequila_core::math::vec3;
    use tequila_materials::material_type::MaterialType;

    /// An import of `(name, parent index, size)` nodes, as the importer
    /// builds one: keys reserved against the document, path keys on every
    /// node, a part for each size and a group where there is none, every
    /// part in one material of the file's, and the file on the top nodes.
    fn import(document: &SceneDocument, nodes: &[(&str, Option<usize>, Option<f32>)], parent: Option<NodeId>) -> ImportPayload {
        let parts = nodes.iter().filter(|n| n.2.is_some()).count();
        let (mesh_keys, material_keys, ids) = reserve_keys(document, parts, 1, nodes.len());
        let keys = keys_of(&nodes.iter().map(|(n, p, _)| (n.to_string(), *p)).collect::<Vec<_>>());
        let mut meshes = Vec::new();
        let mut built = Vec::new();
        for (i, (name, p, size)) in nodes.iter().enumerate() {
            let kind = match size {
                Some(size) => {
                    let key = mesh_keys[meshes.len()];
                    let s = *size;
                    meshes.push((key, Mesh::new(vec![vec3(0.0, 0.0, 0.0), vec3(s, 0.0, 0.0), vec3(0.0, s, 0.0)], vec![0, 1, 2])));
                    NodeKind::Part { mesh: key }
                }
                None => NodeKind::Group,
            };
            let mut node = SceneNode::new(ids[i], *name, kind);
            node.parent = p.map(|p| ids[p]).or(parent);
            node.path_key = Some(keys[i].clone());
            node.transform = Transform::translation(vec3(i as f32, 0.0, 0.0));
            if size.is_some() {
                node.material = Some(material_keys[0]);
            }
            if p.is_none() {
                node.metadata.insert(SOURCE_FILE.into(), "C:/cad/bracket.step".into());
            }
            node.metadata.insert("Revision".into(), "A".into());
            built.push(node);
        }
        for i in 0..built.len() {
            let children: Vec<NodeId> = nodes.iter().enumerate().filter(|(_, n)| n.1 == Some(i)).map(|(c, _)| ids[c]).collect();
            built[i].children = children;
        }
        ImportPayload {
            meshes,
            textures: Vec::new(),
            materials: vec![(material_keys[0], MaterialDefinition::new("From the file", MaterialType::Plastic, MaterialType::Plastic.defaults()))],
            roots: nodes.iter().enumerate().filter(|(_, n)| n.1.is_none()).map(|(i, _)| ids[i]).collect(),
            nodes: built,
        }
    }

    fn find(document: &SceneDocument, name: &str) -> NodeId {
        document.tree.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no {name}")).id
    }

    /// A part given a material of the person's, which the next revision of
    /// the file splits into pieces by colour, keeps that material on every
    /// piece rather than on the group the pieces are in.
    #[test]
    fn a_part_that_becomes_pieces_gives_them_its_material() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let first = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(1.0))], None);
        bus.apply(&mut document, Command::Import { payload: Box::new(first) }).unwrap();
        let root = find(&document, "Bracket");
        let housing = find(&document, "Housing");
        let own = crate::tree::MaterialKey(document.materials.next_key());
        bus.apply(&mut document, Command::Import { payload: Box::new(ImportPayload {
            meshes: Vec::new(), textures: Vec::new(), nodes: Vec::new(), roots: Vec::new(),
            materials: vec![(own, MaterialDefinition::new("Brushed aluminium", MaterialType::Metal, MaterialType::Metal.defaults()))],
        }) }).unwrap();
        bus.apply(&mut document, Command::AssignMaterial { nodes: vec![housing], material: own }).unwrap();

        // The housing is now a group of two pieces.
        let second = import(&document, &[("Bracket", None, None), ("Housing", Some(0), None), ("Housing (red)", Some(1), Some(1.0)), ("Housing (blue)", Some(1), Some(1.0))], document.tree.get(root).unwrap().parent);
        let (command, _) = plan(&document, &[root], second);
        bus.apply(&mut document, command).unwrap();

        let pieces: Vec<&SceneNode> = document.tree.iter().filter(|n| n.name.starts_with("Housing (")).collect();
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|p| p.material == Some(own)), "{:?}", pieces.iter().map(|p| p.material).collect::<Vec<_>>());
        assert!(pieces.iter().all(|p| p.parent == Some(housing)));
    }

    /// The work done on a model survives its update: a part renamed, given
    /// its own material and hidden keeps all three and takes the new shape;
    /// the model stays where it was put; a part the file dropped is hidden
    /// and listed; a part the file gained arrives in its place. Undo puts
    /// every bit of it back.
    #[test]
    fn an_update_keeps_the_work_and_takes_the_new_geometry() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let first = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(1.0)), ("Screw", Some(0), Some(0.1)), ("Screw", Some(0), Some(0.1))], None);
        bus.apply(&mut document, Command::Import { payload: Box::new(first) }).unwrap();

        let root = find(&document, "Bracket");
        let housing = find(&document, "Housing");
        let own = crate::tree::MaterialKey(document.materials.next_key());
        bus.apply(&mut document, Command::Import { payload: Box::new(ImportPayload {
            meshes: Vec::new(), textures: Vec::new(), nodes: Vec::new(), roots: Vec::new(),
            materials: vec![(own, MaterialDefinition::new("Anodised", MaterialType::Metal, MaterialType::Metal.defaults()))],
        }) }).unwrap();
        bus.apply(&mut document, Command::AssignMaterial { nodes: vec![housing], material: own }).unwrap();
        bus.apply(&mut document, Command::Rename { node: housing, name: "Case".into() }).unwrap();
        let placed = Transform::translation(vec3(0.0, 2.0, 0.0));
        bus.apply(&mut document, Command::SetTransform { transforms: vec![(root, placed)] }).unwrap();
        let before_mesh = document.tree.get(housing).unwrap().mesh();

        // The new revision: the housing is bigger, the second screw is gone,
        // and there is a lid.
        let second = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(2.0)), ("Screw", Some(0), Some(0.1)), ("Lid", Some(0), Some(1.0))], document.tree.get(root).unwrap().parent);
        let roots = import_roots(&document, housing);
        assert_eq!(roots, vec![root]);
        assert_eq!(source_file(&document, &roots).as_deref(), Some("C:/cad/bracket.step"));
        let (command, report) = plan(&document, &roots, second);
        assert_eq!(report.matched, 3);
        assert_eq!(report.added, vec!["Lid".to_string()]);
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].1, "Screw");
        let nodes_before = document.tree.iter().count();
        bus.apply(&mut document, command).unwrap();

        let case = document.tree.get(housing).expect("the housing kept its id");
        assert_eq!(case.name, "Case");
        assert_eq!(case.material, Some(own), "the housing lost its material");
        let mesh = document.geometry.get(case.mesh().unwrap()).unwrap();
        assert_eq!(mesh.positions[1].x, 2.0, "the housing kept its old shape");
        assert_eq!(document.tree.get(root).unwrap().transform, placed, "the model moved");
        let screws: Vec<&SceneNode> = document.tree.iter().filter(|n| n.name == "Screw").collect();
        assert_eq!(screws.iter().filter(|n| n.visible).count(), 1);
        let lid = document.tree.get(find(&document, "Lid")).unwrap();
        assert_eq!(lid.parent, Some(root));
        assert!(lid.material.is_some() && lid.material != Some(own));
        assert_eq!(document.tree.iter().count(), nodes_before + 1);

        bus.undo(&mut document).unwrap();
        let case = document.tree.get(housing).unwrap();
        assert_eq!(case.mesh(), before_mesh);
        assert_eq!(document.tree.iter().count(), nodes_before);
        assert!(document.tree.iter().filter(|n| n.name == "Screw").all(|n| n.visible));
        bus.redo(&mut document).unwrap();
        assert_eq!(document.tree.iter().count(), nodes_before + 1);
    }

    /// A model imported before keys were recorded is matched by its tree as
    /// it stands; the parts the user added to it are left alone.
    #[test]
    fn a_model_without_keys_is_matched_by_its_tree() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mut first = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(1.0))], None);
        for node in &mut first.nodes {
            node.path_key = None;
            node.metadata.clear();
        }
        bus.apply(&mut document, Command::Import { payload: Box::new(first) }).unwrap();
        let housing = find(&document, "Housing");
        let roots = import_roots(&document, housing);
        let second = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(3.0))], None);
        let (command, report) = plan(&document, &roots, second);
        assert_eq!(report.matched, 2);
        assert!(report.removed.is_empty() && report.added.is_empty());
        bus.apply(&mut document, command).unwrap();
        let part = document.tree.get(housing).unwrap();
        assert_eq!(document.geometry.get(part.mesh().unwrap()).unwrap().positions[1].x, 3.0);
        assert!(part.path_key.is_some(), "the update did not record the key for next time");
    }

    #[test]
    fn keys_count_same_named_siblings_under_their_parent() {
        let nodes = vec![
            ("Assembly".to_string(), None),
            ("Screw".to_string(), Some(0)),
            ("Housing".to_string(), Some(0)),
            ("Screw".to_string(), Some(0)),
            ("Screw".to_string(), Some(2)),
        ];
        assert_eq!(
            keys_of(&nodes),
            vec!["(model)", "(model)/Screw#1", "(model)/Housing#1", "(model)/Screw#2", "(model)/Housing#1/Screw#1"]
        );
        // Several top nodes are keyed by their names.
        let loose = vec![("Housing".to_string(), None), ("Screw".to_string(), None), ("Screw".to_string(), None)];
        assert_eq!(keys_of(&loose), vec!["Housing#1", "Screw#1", "Screw#2"]);
    }

    /// Two revisions of a real Fusion export, as their trees read: the top
    /// renamed after the document, a part renumbered. Every part matches.
    #[test]
    fn a_renamed_model_and_renumbered_parts_still_match() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let first = import(&document, &[("Finger Wrestler v2", None, None), ("Pocket v3", Some(0), Some(1.0)), ("Bottom v3 (1)", Some(0), Some(1.0)), ("Top v3 (1)", Some(0), Some(1.0))], None);
        bus.apply(&mut document, Command::Import { payload: Box::new(first) }).unwrap();
        let bottom = find(&document, "Bottom v3 (1)");
        let roots = import_roots(&document, bottom);
        let second = import(&document, &[("Finger Wrestler (v2~recovered)", None, None), ("Pocket v3", Some(0), Some(2.0)), ("Bottom v3", Some(0), Some(2.0)), ("Top v3", Some(0), Some(2.0))], None);
        let (_, report) = plan(&document, &roots, second);
        assert_eq!(report.matched, 4, "{report:?}");
        assert!(report.added.is_empty() && report.removed.is_empty(), "{report:?}");
        assert_eq!(without_copy_number("Bottom v3 (12)"), "Bottom v3");
        assert_eq!(without_copy_number("Bracket (left)"), "Bracket (left)");
    }

    /// A part's layers are the file's: an update takes the new file's, and
    /// undo puts the old ones back.
    #[test]
    fn an_update_takes_the_files_layers() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mut first = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(1.0))], None);
        first.nodes[1].layers = vec!["Outer".into()];
        bus.apply(&mut document, Command::Import { payload: Box::new(first) }).unwrap();
        let (root, housing) = (find(&document, "Bracket"), find(&document, "Housing"));
        let mut second = import(&document, &[("Bracket", None, None), ("Housing", Some(0), Some(1.0))], document.tree.get(root).unwrap().parent);
        second.nodes[1].layers = vec!["Shell".into(), "Outer".into()];
        let (command, _) = plan(&document, &[root], second);
        bus.apply(&mut document, command).unwrap();
        assert_eq!(document.tree.get(housing).unwrap().layers, ["Shell", "Outer"]);
        bus.undo(&mut document).unwrap();
        assert_eq!(document.tree.get(housing).unwrap().layers, ["Outer"]);
    }
}
