//! The selection model.
//!
//! `docs/04-UX-SPEC.md` §4.1 gives selection an explicit **level**, and the
//! level decides what a single click means: the part under the cursor, the
//! group it belongs to, everything sharing its material, or one face of it.
//!
//! Kept here rather than in the app because §4.1 ends with a requirement that
//! is not a UI detail:
//!
//! > Selection is synchronized bidirectionally between viewport, Scene Tree,
//! > and Material tab at all times — no exceptions.
//!
//! Three panels cannot stay synchronized if each keeps its own idea of what is
//! selected. There is one [`Selection`], the panels read it and send it
//! changes, and "synchronized" stops being something anyone has to remember.

use crate::document::SceneDocument;
use crate::tree::{MaterialKey, NodeId, NodeKind};
use std::collections::{BTreeMap, BTreeSet};

/// What a click selects (§4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionLevel {
    /// The individual part. Plain click.
    #[default]
    Part,
    /// The top-level group containing it. `Alt`+click, or double-click to step
    /// up.
    Object,
    /// Every part sharing the clicked part's material. `Ctrl`+click.
    ///
    /// This is what makes "recolour every black trim piece" one gesture, and
    /// it is why the material level exists at all.
    Material,
    /// A single face. `F`+click.
    Face,
}

impl SelectionLevel {
    /// For the status bar, which §4.1 requires to show the current level.
    pub fn label(self) -> &'static str {
        match self {
            SelectionLevel::Part => "Part",
            SelectionLevel::Object => "Object",
            SelectionLevel::Material => "Material",
            SelectionLevel::Face => "Face",
        }
    }

    pub const ALL: [SelectionLevel; 4] = [
        SelectionLevel::Part,
        SelectionLevel::Object,
        SelectionLevel::Material,
        SelectionLevel::Face,
    ];
}

/// How a click combines with what is already selected (§4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionMode {
    /// Plain click: this and nothing else.
    #[default]
    Replace,
    /// `Shift`+click.
    Add,
    /// `Ctrl+Shift`+click.
    Remove,
    /// `Shift`+click in the tree, from the anchor to here.
    Range,
}

/// What is selected, and at what level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    level: SelectionLevel,
    nodes: BTreeSet<NodeId>,
    /// Face indices per node, used only at [`SelectionLevel::Face`].
    faces: BTreeMap<NodeId, BTreeSet<u32>>,
    /// Where a range select measures from: the last node picked by a
    /// non-range click.
    anchor: Option<NodeId>,
    /// The most recently added node, which the property panels show when the
    /// selection holds several.
    active: Option<NodeId>,
}

impl Selection {
    pub fn new() -> Selection {
        Selection::default()
    }

    pub fn level(&self) -> SelectionLevel {
        self.level
    }

    /// Change the level.
    ///
    /// The selection itself is kept: switching to Material level with a part
    /// already selected and then clicking is the natural gesture, and dropping
    /// the selection on every level change would make the toolbar toggle
    /// destructive.
    pub fn set_level(&mut self, level: SelectionLevel) {
        if self.level != level {
            self.level = level;
            if level != SelectionLevel::Face {
                self.faces.clear();
            }
        }
    }

    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().copied()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains(&id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node the property panels show. `None` when nothing is selected.
    pub fn active(&self) -> Option<NodeId> {
        self.active.filter(|id| self.nodes.contains(id))
    }

    /// Selected faces of a node, at [`SelectionLevel::Face`].
    pub fn faces(&self, id: NodeId) -> impl Iterator<Item = u32> + '_ {
        self.faces.get(&id).into_iter().flatten().copied()
    }

    /// `Esc`.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.faces.clear();
        self.anchor = None;
        self.active = None;
    }

    /// `Ctrl+A`. Selects parts, not the groups above them: selecting a group
    /// *and* its children would apply every subsequent operation twice.
    pub fn select_all(&mut self, document: &SceneDocument) {
        self.nodes = document
            .tree
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Part { .. }))
            .map(|node| node.id)
            .collect();
        self.faces.clear();
        self.active = self.nodes.iter().next_back().copied();
        self.anchor = self.active;
    }

    /// Apply a click on `clicked`, at the current level.
    ///
    /// Returns what the click resolved to, which the viewport uses to draw the
    /// highlight and the tree uses to scroll into view.
    pub fn click(
        &mut self,
        document: &SceneDocument,
        clicked: NodeId,
        mode: SelectionMode,
    ) -> Vec<NodeId> {
        let resolved = resolve(document, self.level, clicked);
        if resolved.is_empty() {
            return resolved;
        }

        match mode {
            SelectionMode::Replace => {
                self.nodes = resolved.iter().copied().collect();
                self.faces.clear();
                self.anchor = Some(clicked);
                self.active = Some(clicked);
            }
            SelectionMode::Add => {
                self.nodes.extend(resolved.iter().copied());
                self.anchor = Some(clicked);
                self.active = Some(clicked);
            }
            SelectionMode::Remove => {
                for id in &resolved {
                    self.nodes.remove(id);
                    self.faces.remove(id);
                }
                if self.active.is_some_and(|a| !self.nodes.contains(&a)) {
                    self.active = self.nodes.iter().next_back().copied();
                }
            }
            SelectionMode::Range => {
                let from = self.anchor.unwrap_or(clicked);
                for id in range(document, from, clicked) {
                    self.nodes.insert(id);
                }
                // The anchor deliberately stays put: dragging a range select
                // back and forth from one anchor is the standard behaviour,
                // and moving it would make each shift-click extend from the
                // last instead.
                self.active = Some(clicked);
            }
        }
        resolved
    }

    /// Add or remove one face, at [`SelectionLevel::Face`].
    ///
    /// A no-op at any other level: the level is what says whether a face is
    /// even a thing that can be selected, and silently selecting faces while
    /// the user is in Part mode would make the highlight disagree with the
    /// status bar.
    pub fn click_face(&mut self, node: NodeId, face: u32, mode: SelectionMode) {
        if self.level != SelectionLevel::Face {
            return;
        }
        match mode {
            SelectionMode::Replace => {
                self.nodes.clear();
                self.faces.clear();
                self.nodes.insert(node);
                self.faces.entry(node).or_default().insert(face);
            }
            SelectionMode::Add | SelectionMode::Range => {
                self.nodes.insert(node);
                self.faces.entry(node).or_default().insert(face);
            }
            SelectionMode::Remove => {
                if let Some(faces) = self.faces.get_mut(&node) {
                    faces.remove(&face);
                    if faces.is_empty() {
                        self.faces.remove(&node);
                        self.nodes.remove(&node);
                    }
                }
            }
        }
        self.anchor = Some(node);
        self.active = Some(node);
    }

    /// Replace the selection with the results of a box drag.
    pub fn select_many(&mut self, ids: impl IntoIterator<Item = NodeId>, mode: SelectionMode) {
        let ids: Vec<NodeId> = ids.into_iter().collect();
        match mode {
            SelectionMode::Replace => {
                self.nodes = ids.iter().copied().collect();
                self.faces.clear();
            }
            SelectionMode::Add | SelectionMode::Range => self.nodes.extend(ids.iter().copied()),
            SelectionMode::Remove => {
                for id in &ids {
                    self.nodes.remove(id);
                    self.faces.remove(id);
                }
            }
        }
        self.active = ids
            .last()
            .copied()
            .filter(|id| self.nodes.contains(id))
            .or_else(|| self.nodes.iter().next_back().copied());
        self.anchor = self.active;
    }

    /// Drop anything that is no longer in the document.
    ///
    /// Called after every command. Without it, deleting a selected part leaves
    /// its ID selected, and the next operation refers to a node that is not
    /// there — which is a crash in the viewport or, worse, silently applies to
    /// nothing.
    pub fn prune(&mut self, document: &SceneDocument) {
        self.nodes.retain(|id| document.tree.contains(*id));
        self.faces.retain(|id, _| document.tree.contains(*id));
        self.anchor = self.anchor.filter(|id| document.tree.contains(*id));
        self.active = self
            .active
            .filter(|id| self.nodes.contains(id))
            .or_else(|| self.nodes.iter().next_back().copied());
    }

    /// A short description for the status bar.
    pub fn summary(&self, document: &SceneDocument) -> String {
        match self.nodes.len() {
            0 => format!("{}: nothing selected", self.level.label()),
            1 => {
                let id = self.nodes.iter().next().copied().unwrap();
                let name = document
                    .tree
                    .get(id)
                    .map(|n| n.name.as_str())
                    .unwrap_or("(missing)");
                match self.faces.get(&id).map(|f| f.len()).unwrap_or(0) {
                    0 => format!("{}: {name}", self.level.label()),
                    1 => format!("{}: {name}, 1 face", self.level.label()),
                    n => format!("{}: {name}, {n} faces", self.level.label()),
                }
            }
            n => format!("{}: {n} selected", self.level.label()),
        }
    }
}

/// What clicking `clicked` selects, at `level`.
pub fn resolve(document: &SceneDocument, level: SelectionLevel, clicked: NodeId) -> Vec<NodeId> {
    if !document.tree.contains(clicked) {
        return Vec::new();
    }
    match level {
        SelectionLevel::Part | SelectionLevel::Face => vec![clicked],
        SelectionLevel::Object => vec![top_level_ancestor(document, clicked)],
        SelectionLevel::Material => match document.tree.get(clicked).and_then(|n| n.material) {
            Some(material) => document.tree.users_of_material(material),
            // A part with no material assigned shares "no material" with a lot
            // of unrelated parts; selecting all of them would be surprising.
            // Clicking one selects just it.
            None => vec![clicked],
        },
    }
}

/// The outermost group containing a node, which is what §4.1 means by "the
/// top-level group containing it".
///
/// A node with no parent is its own top level, so `Alt`+click on a
/// root-level part selects that part rather than nothing.
pub fn top_level_ancestor(document: &SceneDocument, id: NodeId) -> NodeId {
    document.tree.ancestors(id).last().copied().unwrap_or(id)
}

/// Every node between two, inclusive, in the order the tree displays them.
///
/// Display order rather than ID order: a `Shift`-click in the tree selects
/// what the user saw between the two rows, and after a reparent those are not
/// the same set.
pub fn range(document: &SceneDocument, from: NodeId, to: NodeId) -> Vec<NodeId> {
    let order = document.tree.depth_first();
    let (Some(a), Some(b)) = (
        order.iter().position(|id| *id == from),
        order.iter().position(|id| *id == to),
    ) else {
        return vec![to];
    };
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    order[low..=high].to_vec()
}

/// Which of a selection a transform should actually be applied to.
///
/// A node whose ancestor is also selected is dropped. Transforms compose down
/// the tree, so moving a group *and* a part inside it would apply the motion
/// to that part twice: it would travel at double speed and slide out of the
/// assembly. Selecting a whole assembly and then ctrl-clicking one of its
/// screws is an easy thing to do without meaning to, so this filters rather
/// than refuses.
///
/// Order follows the tree, not the selection, so the result is the same
/// whichever way round the user clicked.
pub fn transform_targets(document: &SceneDocument, selected: &[NodeId]) -> Vec<NodeId> {
    // Asked every frame the viewport draws: it walked the whole tree each
    // time, nothing selected or not. Now each selected node is placed by its
    // path from the root - which orders as depth-first order does - so the
    // cost is the selection's, not the assembly's.
    if selected.is_empty() {
        return Vec::new();
    }
    let chosen: BTreeSet<NodeId> = selected.iter().copied().collect();
    let tree = &document.tree;
    let mut kept: Vec<(Vec<usize>, NodeId)> = chosen
        .iter()
        .filter_map(|&id| {
            let mut path = Vec::new();
            let mut current = id;
            loop {
                let node = tree.get(current)?;
                if current != id && chosen.contains(&current) {
                    // An ancestor is selected: it moves this node with it.
                    return None;
                }
                match node.parent {
                    Some(parent) => {
                        path.push(tree.get(parent)?.children.iter().position(|&c| c == current)?);
                        current = parent;
                    }
                    None => {
                        path.push(tree.roots().iter().position(|&r| r == current)?);
                        break;
                    }
                }
            }
            path.reverse();
            Some((path, id))
        })
        .collect();
    kept.sort();
    kept.into_iter().map(|(_, id)| id).collect()
}

/// Parts sharing a material, for the Material tab's "used by" list.
pub fn users_of(document: &SceneDocument, material: MaterialKey) -> Vec<NodeId> {
    document.tree.users_of_material(material)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, ImportPayload};
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use crate::tree::SceneNode;
    use crate::CommandBus;
    use tequila_core::math::{vec3, Vec3};

    /// Assembly
    ///   Housing        (material 1)
    ///   Fasteners
    ///     Screw A      (material 2)
    ///     Screw B      (material 2)
    ///   Bracket        (material 1)
    fn assembly() -> (SceneDocument, CommandBus, Vec<NodeId>) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();

        let mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let (meshes, materials, ids) = crate::command::reserve_keys(&document, 1, 2, 6);

        let mut nodes = Vec::new();
        let mut node = SceneNode::new(ids[0], "Assembly", NodeKind::Group);
        node.children = vec![ids[1], ids[2], ids[5]];
        nodes.push(node);

        for (index, name, material) in [
            (1usize, "Housing", 0usize),
            (3, "Screw A", 1),
            (4, "Screw B", 1),
            (5, "Bracket", 0),
        ] {
            let mut node = SceneNode::new(ids[index], name, NodeKind::Part { mesh: meshes[0] });
            node.parent = Some(if index == 3 || index == 4 {
                ids[2]
            } else {
                ids[0]
            });
            node.material = Some(materials[material]);
            nodes.push(node);
        }
        // The Fasteners group, inserted in the right place in the list.
        let mut fasteners = SceneNode::new(ids[2], "Fasteners", NodeKind::Group);
        fasteners.parent = Some(ids[0]);
        fasteners.children = vec![ids[3], ids[4]];
        nodes.insert(2, fasteners);

        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: vec![
                        (materials[0], MaterialDefinition::plastic("Shell")),
                        (materials[1], MaterialDefinition::plastic("Steel")),
                    ],
                    nodes,
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        document.validate().unwrap();
        (document, bus, ids)
    }

    #[test]
    fn a_plain_click_selects_the_part_and_nothing_else() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();

        selection.click(&document, ids[3], SelectionMode::Replace);
        assert_eq!(selection.nodes().collect::<Vec<_>>(), vec![ids[3]]);

        selection.click(&document, ids[4], SelectionMode::Replace);
        assert_eq!(
            selection.nodes().collect::<Vec<_>>(),
            vec![ids[4]],
            "a plain click must replace, not add"
        );
    }

    /// §4.1: Object level selects "the top-level group containing it" — the
    /// outermost one, not the immediate parent. Clicking a screw two levels
    /// down selects the whole assembly.
    #[test]
    fn object_level_selects_the_outermost_group() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();
        selection.set_level(SelectionLevel::Object);

        selection.click(&document, ids[3], SelectionMode::Replace);
        assert_eq!(
            selection.nodes().collect::<Vec<_>>(),
            vec![ids[0]],
            "clicking a screw must select the assembly, not the Fasteners group"
        );
    }

    /// A part at the root has no group above it, and must select itself rather
    /// than nothing.
    #[test]
    fn object_level_on_a_root_part_selects_that_part() {
        let (mut document, mut bus, _) = assembly();
        bus.apply(
            &mut document,
            Command::AddGroup {
                parent: None,
                name: "Loose".into(),
            },
        )
        .unwrap();
        let loose = document.tree.iter().find(|n| n.name == "Loose").unwrap().id;

        let mut selection = Selection::new();
        selection.set_level(SelectionLevel::Object);
        selection.click(&document, loose, SelectionMode::Replace);
        assert_eq!(selection.nodes().collect::<Vec<_>>(), vec![loose]);
    }

    /// The gesture that makes "recolour every black trim piece" one click.
    #[test]
    fn material_level_selects_every_part_sharing_the_material() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();
        selection.set_level(SelectionLevel::Material);

        selection.click(&document, ids[3], SelectionMode::Replace);
        assert_eq!(
            selection.nodes().collect::<Vec<_>>(),
            vec![ids[3], ids[4]],
            "both screws share Steel"
        );

        selection.click(&document, ids[1], SelectionMode::Replace);
        assert_eq!(
            selection.nodes().collect::<Vec<_>>(),
            vec![ids[1], ids[5]],
            "the housing and the bracket share Shell"
        );
    }

    /// Parts with no material do not all share one: selecting every unassigned
    /// part in a 4,000-part assembly because the user clicked one would be a
    /// disaster.
    #[test]
    fn material_level_on_an_unassigned_part_selects_only_it() {
        let (mut document, mut bus, ids) = assembly();
        bus.apply(
            &mut document,
            Command::AddGroup {
                parent: None,
                name: "Bare".into(),
            },
        )
        .unwrap();
        let bare = document.tree.iter().find(|n| n.name == "Bare").unwrap().id;
        assert!(document.tree.get(bare).unwrap().material.is_none());

        let mut selection = Selection::new();
        selection.set_level(SelectionLevel::Material);
        selection.click(&document, bare, SelectionMode::Replace);
        assert_eq!(selection.nodes().collect::<Vec<_>>(), vec![bare]);
        assert!(
            !selection.contains(ids[0]),
            "the assembly is also unassigned"
        );
    }

    #[test]
    fn shift_adds_and_ctrl_shift_removes() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();

        selection.click(&document, ids[1], SelectionMode::Replace);
        selection.click(&document, ids[3], SelectionMode::Add);
        selection.click(&document, ids[4], SelectionMode::Add);
        assert_eq!(selection.len(), 3);

        selection.click(&document, ids[3], SelectionMode::Remove);
        assert_eq!(selection.len(), 2);
        assert!(!selection.contains(ids[3]));
    }

    /// Range select works in *display* order, not ID order: after a reparent
    /// those differ, and the user picked what they saw.
    #[test]
    fn a_range_select_follows_the_displayed_order() {
        let (mut document, mut bus, ids) = assembly();
        let mut selection = Selection::new();

        selection.click(&document, ids[1], SelectionMode::Replace);
        selection.click(&document, ids[4], SelectionMode::Range);
        // Housing, Fasteners, Screw A, Screw B.
        assert_eq!(selection.len(), 4);
        assert!(
            selection.contains(ids[2]),
            "the group between them is in range"
        );
        assert!(!selection.contains(ids[5]), "Bracket is past the end");

        // Move Bracket to the top and the same range means something else.
        bus.apply(
            &mut document,
            Command::Reparent {
                node: ids[5],
                parent: Some(ids[0]),
                index: Some(0),
            },
        )
        .unwrap();
        let mut selection = Selection::new();
        selection.click(&document, ids[5], SelectionMode::Replace);
        selection.click(&document, ids[1], SelectionMode::Range);
        assert_eq!(
            selection.nodes().collect::<Vec<_>>(),
            vec![ids[1], ids[5]],
            "Bracket now sits immediately before Housing"
        );
    }

    /// The anchor stays put across repeated shift-clicks, so dragging a range
    /// back and forth grows and shrinks from one end.
    #[test]
    fn repeated_range_clicks_measure_from_the_same_anchor() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();

        selection.click(&document, ids[1], SelectionMode::Replace);
        selection.click(&document, ids[3], SelectionMode::Range);
        let after_first = selection.len();
        selection.click(&document, ids[4], SelectionMode::Range);

        assert_eq!(after_first, 3);
        assert_eq!(selection.len(), 4);
        assert!(
            selection.contains(ids[1]),
            "the anchor must still be selected"
        );
    }

    #[test]
    fn select_all_takes_parts_not_the_groups_above_them() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();
        selection.select_all(&document);

        assert_eq!(selection.len(), 4);
        assert!(
            !selection.contains(ids[0]) && !selection.contains(ids[2]),
            "selecting a group and its children applies every operation twice"
        );
    }

    /// Deleting a selected part must not leave its ID selected, or the next
    /// command refers to a node that is not there.
    #[test]
    fn deleting_a_selected_node_prunes_it_from_the_selection() {
        let (mut document, mut bus, ids) = assembly();
        let mut selection = Selection::new();
        selection.click(&document, ids[3], SelectionMode::Replace);
        selection.click(&document, ids[4], SelectionMode::Add);

        bus.apply(
            &mut document,
            Command::DeleteNodes {
                nodes: vec![ids[3]],
            },
        )
        .unwrap();
        selection.prune(&document);

        assert_eq!(selection.nodes().collect::<Vec<_>>(), vec![ids[4]]);
        assert_eq!(selection.active(), Some(ids[4]));

        // And undoing brings the node back, but not the selection — that is
        // the app's business, and silently reselecting would surprise.
        bus.undo(&mut document).unwrap();
        selection.prune(&document);
        assert_eq!(selection.len(), 1);
    }

    /// Deleting a whole subtree prunes the children too, not just the node
    /// that was named.
    #[test]
    fn pruning_follows_a_deleted_subtree() {
        let (mut document, mut bus, ids) = assembly();
        let mut selection = Selection::new();
        selection.select_all(&document);

        bus.apply(
            &mut document,
            Command::DeleteNodes {
                nodes: vec![ids[2]],
            },
        )
        .unwrap();
        selection.prune(&document);

        assert!(!selection.contains(ids[3]));
        assert!(!selection.contains(ids[4]));
        assert_eq!(selection.len(), 2, "the housing and bracket survive");
    }

    #[test]
    fn faces_are_only_selectable_at_face_level() {
        let (_document, _, ids) = assembly();
        let mut selection = Selection::new();

        selection.click_face(ids[1], 3, SelectionMode::Replace);
        assert!(
            selection.is_empty(),
            "a face was selected while the status bar said Part"
        );

        selection.set_level(SelectionLevel::Face);
        selection.click_face(ids[1], 3, SelectionMode::Replace);
        selection.click_face(ids[1], 7, SelectionMode::Add);
        assert_eq!(selection.faces(ids[1]).collect::<Vec<_>>(), vec![3, 7]);

        selection.click_face(ids[1], 3, SelectionMode::Remove);
        assert_eq!(selection.faces(ids[1]).collect::<Vec<_>>(), vec![7]);

        // Removing the last face deselects the node with it.
        selection.click_face(ids[1], 7, SelectionMode::Remove);
        assert!(selection.is_empty());
    }

    /// Leaving Face level must not leave face selections behind to be applied
    /// by a later whole-part operation.
    #[test]
    fn leaving_face_level_drops_the_face_selection() {
        let (_document, _, ids) = assembly();
        let mut selection = Selection::new();
        selection.set_level(SelectionLevel::Face);
        selection.click_face(ids[1], 3, SelectionMode::Replace);

        selection.set_level(SelectionLevel::Part);
        assert_eq!(selection.faces(ids[1]).count(), 0);
        assert!(selection.contains(ids[1]), "the part itself stays selected");
    }

    #[test]
    fn the_status_bar_summary_names_the_level_and_what_is_selected() {
        let (document, _, ids) = assembly();
        let mut selection = Selection::new();
        assert_eq!(selection.summary(&document), "Part: nothing selected");

        selection.click(&document, ids[1], SelectionMode::Replace);
        assert_eq!(selection.summary(&document), "Part: Housing");

        selection.click(&document, ids[3], SelectionMode::Add);
        assert_eq!(selection.summary(&document), "Part: 2 selected");

        selection.clear();
        selection.set_level(SelectionLevel::Face);
        selection.click_face(ids[1], 3, SelectionMode::Replace);
        assert_eq!(selection.summary(&document), "Face: Housing, 1 face");
    }

    /// Transforms compose down the tree, so a part whose group is also
    /// selected must not be moved twice — it would travel at double speed and
    /// leave the assembly.
    #[test]
    fn a_transform_skips_nodes_whose_ancestor_is_also_selected() {
        let (document, _, ids) = assembly();

        // The Fasteners group and one of its screws.
        let targets = transform_targets(&document, &[ids[2], ids[3]]);
        assert_eq!(targets, vec![ids[2]], "the screw must be dropped");

        // The whole assembly plus everything under it.
        let everything: Vec<NodeId> = document.tree.iter().map(|n| n.id).collect();
        assert_eq!(
            transform_targets(&document, &everything),
            vec![ids[0]],
            "only the root should move"
        );

        // Siblings are all kept.
        assert_eq!(
            transform_targets(&document, &[ids[3], ids[4]]),
            vec![ids[3], ids[4]]
        );
    }

    /// Placed by their paths from the root, the targets are what walking the
    /// whole tree gave, for every selection the assembly allows - and an id
    /// that is not there is left out, as before.
    #[test]
    fn transform_targets_match_the_whole_tree_walk() {
        let (document, _, ids) = assembly();
        let walked = |selected: &[NodeId]| -> Vec<NodeId> {
            let chosen: BTreeSet<NodeId> = selected.iter().copied().collect();
            document
                .tree
                .depth_first()
                .into_iter()
                .filter(|id| chosen.contains(id))
                .filter(|id| !document.tree.ancestors(*id).iter().any(|a| chosen.contains(a)))
                .collect()
        };
        let all: Vec<NodeId> = document.tree.depth_first();
        assert!(all.len() >= ids.len() && all.len() <= 12, "the assembly is small enough to try every selection");
        for mask in 0u32..(1 << all.len()) {
            let mut selected: Vec<NodeId> = all.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, id)| *id).collect();
            selected.reverse();
            assert_eq!(transform_targets(&document, &selected), walked(&selected), "selection {selected:?}");
        }
        assert_eq!(transform_targets(&document, &[NodeId(9999), ids[3]]), vec![ids[3]]);
        assert!(transform_targets(&document, &[]).is_empty());
    }

    /// The result must not depend on the order the user clicked.
    #[test]
    fn transform_targets_do_not_depend_on_selection_order() {
        let (document, _, ids) = assembly();
        let forward = transform_targets(&document, &[ids[1], ids[3], ids[4]]);
        let backward = transform_targets(&document, &[ids[4], ids[3], ids[1]]);
        assert_eq!(forward, backward);
    }

    #[test]
    fn clicking_a_node_that_is_gone_changes_nothing() {
        let (document, _, _) = assembly();
        let mut selection = Selection::new();
        selection.click(&document, NodeId(9999), SelectionMode::Replace);
        assert!(selection.is_empty());
    }
}
