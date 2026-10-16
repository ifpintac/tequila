//! Multi-Materials, Model Sets and Studios (`TEQUILA_SPEC.md` §1.12,
//! `docs/02` §8.1): the variations of one product kept in one project.
//!
//! - A **Multi-Material** is an ordered set of material variants on a set of
//!   parts, one of them active — a colourway switch.
//! - A **Model Set** is a named selection of the scene tree — three chassis
//!   of the same product — of which one is shown at a time, and an **Always
//!   Visible** set is shown with every one: the backdrop and props.
//! - A **Studio** is a saved shot: a camera, an environment, an image style,
//!   a model set and a choice of variants, recalled in one step.
//!
//! ## The active variant is written into the parts
//!
//! Switching a Multi-Material assigns the active variant to its parts like
//! any assignment would, rather than having every reader of a part's material
//! ask the Multi-Material first. The renderer, the Material tab, the file and
//! the scripting API already agree on what a part's material is; one more
//! place to look would be one more place for them to disagree. The cost is
//! that assigning some other material to a member part by hand is allowed,
//! and the next switch puts the variant back — which is what a switch is.

use crate::document::{EnvironmentSettings, SceneDocument};
use crate::lighting::SavedCamera;
use crate::tree::{MaterialKey, NodeId, SceneTree};

/// Variants of the material on a set of parts.
#[derive(Clone, Debug, PartialEq)]
pub struct MultiMaterial {
    /// Stable within the document, so a Studio can name it across edits to
    /// the list.
    pub id: u64,
    pub name: String,
    pub variants: Vec<MaterialKey>,
    pub active: usize,
    pub parts: Vec<NodeId>,
}

impl MultiMaterial {
    /// The variant in use, if there is one.
    pub fn active_material(&self) -> Option<MaterialKey> {
        self.variants.get(self.active).copied()
    }
}

/// A named selection of the scene tree.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelSet {
    pub name: String,
    /// Its members. A member's descendants belong with it.
    pub nodes: Vec<NodeId>,
    /// Shown whichever set is active: staging geometry.
    pub always_visible: bool,
}

/// The document's model sets, and which is shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelSets {
    pub sets: Vec<ModelSet>,
    /// `None` shows everything, which is also what a document without sets
    /// does.
    pub active: Option<usize>,
}

impl ModelSets {
    /// Whether the active set lets this node be seen.
    ///
    /// A node in no set at all is always seen: lights, cameras' targets and
    /// whatever has not been sorted yet should not vanish because somebody
    /// made a set of something else. A node in some set is seen when one of
    /// its sets is the active one or is always visible.
    pub fn allows(&self, tree: &SceneTree, node: NodeId) -> bool {
        let Some(active) = self.active else {
            return true;
        };
        let mut chain = vec![node];
        chain.extend(tree.ancestors(node));
        let mut member_of_any = false;
        for (index, set) in self.sets.iter().enumerate() {
            if set.nodes.iter().any(|n| chain.contains(n)) {
                if index == active || set.always_visible {
                    return true;
                }
                member_of_any = true;
            }
        }
        !member_of_any
    }
}

/// A saved shot. Every part is optional, so a Studio can switch the camera
/// and leave the lighting alone, or the reverse.
#[derive(Clone, Debug, PartialEq)]
pub struct Studio {
    pub name: String,
    pub camera: Option<SavedCamera>,
    pub environment: Option<EnvironmentSettings>,
    pub image_style: Option<tequila_color::ImageStyle>,
    /// Which model set to show: `Some(None)` shows everything, `None` leaves
    /// it as it is.
    pub model_set: Option<Option<usize>>,
    /// Multi-Material id and the variant to make active.
    pub variants: Vec<(u64, usize)>,
}

impl Studio {
    /// A Studio of everything as it is now.
    pub fn capture(name: &str, document: &SceneDocument) -> Studio {
        Studio {
            name: name.into(),
            camera: Some(document.cameras.active().clone()),
            environment: Some(document.environment.clone()),
            image_style: Some(document.image_style),
            model_set: Some(document.model_sets.active),
            variants: document.multi_materials.iter().map(|m| (m.id, m.active)).collect(),
        }
    }
}

/// Assign each Multi-Material's active variant to its parts. Returns what
/// each changed part had before, for undo.
pub fn apply_variants(document: &mut SceneDocument) -> Vec<(NodeId, Option<MaterialKey>)> {
    let mut previous = Vec::new();
    let assignments: Vec<(NodeId, MaterialKey)> = document
        .multi_materials
        .iter()
        .filter_map(|m| m.active_material().map(|key| (m, key)))
        .flat_map(|(m, key)| m.parts.iter().map(move |&part| (part, key)))
        .collect();
    for (part, key) in assignments {
        if let Ok(node) = document.tree.get_mut(part) {
            if node.mesh().is_some() && node.material != Some(key) {
                previous.push((part, node.material));
                node.material = Some(key);
            }
        }
    }
    previous
}

/// Put parts' materials back as [`apply_variants`] found them, last change
/// first so a part changed twice ends at its first value.
pub fn restore_materials(document: &mut SceneDocument, previous: &[(NodeId, Option<MaterialKey>)]) {
    for (part, material) in previous.iter().rev() {
        if let Ok(node) = document.tree.get_mut(*part) {
            node.material = *material;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::NodeKind;

    #[test]
    fn a_node_in_no_set_is_always_seen_and_members_follow_the_active_set() {
        let mut document = SceneDocument::new();
        let a = document.add_node(None, "A", NodeKind::Group).unwrap();
        let b = document.add_node(None, "B", NodeKind::Group).unwrap();
        let child = document.add_node(Some(b), "B child", NodeKind::Group).unwrap();
        let stage = document.add_node(None, "Stage", NodeKind::Group).unwrap();
        let loose = document.add_node(None, "Loose", NodeKind::Group).unwrap();
        let sets = ModelSets {
            sets: vec![
                ModelSet {
                    name: "A".into(),
                    nodes: vec![a],
                    always_visible: false,
                },
                ModelSet {
                    name: "B".into(),
                    nodes: vec![b],
                    always_visible: false,
                },
                ModelSet {
                    name: "Stage".into(),
                    nodes: vec![stage],
                    always_visible: true,
                },
            ],
            active: Some(0),
        };
        let tree = &document.tree;
        assert!(sets.allows(tree, a));
        assert!(!sets.allows(tree, b));
        assert!(!sets.allows(tree, child), "a member's child showed with another set active");
        assert!(sets.allows(tree, stage), "an always-visible set was hidden");
        assert!(sets.allows(tree, loose), "a node in no set was hidden");

        let none = ModelSets {
            active: None,
            ..sets.clone()
        };
        assert!(none.allows(tree, b));
    }
}
