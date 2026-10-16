//! Working out what a material drop will affect (`docs/04-UX-SPEC.md` §3.1).
//!
//! §3 opens by calling itself normative: "Get any of it wrong and the
//! application feels unlike the thing it is meant to replace." The part that
//! carries the most weight is §3.1's modifier preview, which the spec
//! introduces with an instruction rather than a description — *"do this — it
//! removes an entire category of mistakes"*.
//!
//! The category is this. Materials are shared instances (§8.2), so dropping one
//! onto a part can change seven other parts at the same time, and the person
//! dropping it usually has no way to know that before they let go. Undo is
//! there, but by the time they notice, three more edits have happened. Stating
//! the consequence in the floating label *before* the drop is what turns a
//! surprising model into an obvious one.
//!
//! So this module exists to answer one question — which nodes does this drop
//! touch? — and to answer it identically for the preview and for the drop
//! itself. Two code paths would eventually disagree, and the label would
//! promise something the drop did not do, which is worse than no label.

use crate::document::SceneDocument;
use crate::tree::{MaterialKey, NodeId, NodeKind};

/// Modifiers held at the moment of the drop (§3.1 step 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropModifiers {
    /// Give this part its own copy instead of sharing.
    pub alt: bool,
    /// Every part in the target's group.
    pub shift: bool,
    /// Every part currently sharing the target's material.
    pub ctrl: bool,
}

impl DropModifiers {
    pub const NONE: DropModifiers = DropModifiers {
        alt: false,
        shift: false,
        ctrl: false,
    };
}

/// What a drop would do, resolved before it happens.
#[derive(Clone, Debug, PartialEq)]
pub struct DropPlan {
    /// The node under the cursor.
    pub target: NodeId,
    /// Every node the drop assigns to, in tree order.
    pub affected: Vec<NodeId>,
    /// Whether the drop unlinks rather than sharing.
    pub unlink: bool,
    /// How many parts currently share the target's material, including it.
    pub shared_with: usize,
    /// The sentence for the floating label (§3.1 step 3).
    pub description: String,
}

/// Resolve a drop (§3.1).
///
/// Modifiers are checked in the order Alt, Shift, Ctrl so that holding two
/// gives a defined result rather than whichever branch happened to come first.
/// Alt wins because it is the only one that changes the *kind* of operation
/// rather than its extent.
pub fn plan_drop(
    document: &SceneDocument,
    target: NodeId,
    modifiers: DropModifiers,
) -> Option<DropPlan> {
    let node = document.tree.get(target)?;
    let name = node.name.clone();
    let current = node.material;

    let shared_with = current
        .map(|key| document.tree.users_of_material(key).len())
        .unwrap_or(0);

    // A group has no material of its own; dropping on one means everything
    // beneath it, which is §3.3's rule for the scene tree and the only sensible
    // reading in the viewport too.
    let is_group = matches!(node.kind, NodeKind::Group);

    let (affected, description) = if modifiers.alt {
        (
            vec![target],
            format!("Apply to: `{name}` only (unlink)"),
        )
    } else if modifiers.shift || is_group {
        let group = enclosing_group(document, target);
        let parts = parts_under(document, group);
        let group_name = document
            .tree
            .get(group)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "scene".into());
        (
            parts.clone(),
            format!(
                "Apply to: all {} part{} in group `{group_name}`",
                parts.len(),
                if parts.len() == 1 { "" } else { "s" }
            ),
        )
    } else if modifiers.ctrl {
        match current {
            Some(key) => {
                let sharing = document.tree.users_of_material(key);
                let material_name = document
                    .materials
                    .get(key)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "material".into());
                (
                    sharing.clone(),
                    format!(
                        "Apply to: all {} parts with `{material_name}`",
                        sharing.len()
                    ),
                )
            }
            None => (
                vec![target],
                format!("Apply to: `{name}` (it has no material to match)"),
            ),
        }
    } else {
        // The plain case, and the one the warning is for.
        let sentence = match shared_with {
            0 | 1 => format!("Apply to: `{name}`"),
            n => format!("Apply to: `{name}` ({n} parts share this material)"),
        };
        (vec![target], sentence)
    };

    Some(DropPlan {
        target,
        affected,
        unlink: modifiers.alt,
        shared_with,
        description,
    })
}

/// Resolve a drop knowing what is selected (§3.1).
///
/// A plain drop on a selected part, or on a part inside a selected group,
/// goes on everything selected. Otherwise putting one material on twenty
/// parts took twenty drops: a CAD model's body often arrives as a piece per
/// face colour, and each drop covered one piece. A drop on something not
/// selected, or with a modifier held, is [`plan_drop`]'s; the plain one's
/// label then says how to reach the whole selection.
pub fn plan_drop_in(
    document: &SceneDocument,
    target: NodeId,
    modifiers: DropModifiers,
    selected: &[NodeId],
) -> Option<DropPlan> {
    let mut plan = plan_drop(document, target, modifiers)?;
    if modifiers != DropModifiers::NONE || selected.is_empty() {
        return Some(plan);
    }
    let parts = parts_of(document, selected.iter().copied());
    let in_selection = std::iter::successors(Some(target), |&node| document.tree.get(node).and_then(|n| n.parent))
        .any(|node| selected.contains(&node));
    if in_selection && parts.len() > 1 {
        plan.description = format!("Apply to: the {} selected parts", parts.len());
        plan.affected = parts;
    } else if !in_selection && parts.len() > 1 {
        plan.description.push_str(&format!("\nDrop on a selected part for all {}", parts.len()));
    }
    Some(plan)
}

/// The group a node belongs to: itself if it is one, otherwise its parent.
fn enclosing_group(document: &SceneDocument, node: NodeId) -> NodeId {
    let Some(entry) = document.tree.get(node) else {
        return node;
    };
    if matches!(entry.kind, NodeKind::Group) {
        return node;
    }
    entry.parent.unwrap_or(node)
}

/// Every part at or beneath any of `nodes`, once each: what putting a
/// material on a selection means when it holds a group. A group's own
/// material is never drawn, so assigning to the group itself changed
/// nothing anyone could see.
pub fn parts_of(document: &SceneDocument, nodes: impl IntoIterator<Item = NodeId>) -> Vec<NodeId> {
    let mut parts: Vec<NodeId> = nodes.into_iter().flat_map(|node| parts_under(document, node)).collect();
    parts.sort_by_key(|id| id.0);
    parts.dedup();
    parts
}

/// Every part at or beneath a node, in tree order.
pub fn parts_under(document: &SceneDocument, root: NodeId) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = document.tree.get(id) else {
            continue;
        };
        if matches!(node.kind, NodeKind::Part { .. }) {
            found.push(id);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    found.sort_by_key(|id| id.0);
    found
}

/// The material a plan would leave on its target, for the caller that needs to
/// select it afterwards (§3.1 step 4).
pub fn resulting_selection(plan: &DropPlan) -> Vec<NodeId> {
    plan.affected.clone()
}

/// Whether two plans would do the same thing, so a hover preview can skip
/// re-rendering its label every frame.
pub fn same_effect(a: &DropPlan, b: &DropPlan) -> bool {
    a.target == b.target && a.unlink == b.unlink && a.affected == b.affected
}

/// A material's reference count, for §3.2's badge.
pub fn share_count(document: &SceneDocument, material: MaterialKey) -> usize {
    document.tree.users_of_material(material).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandBus, ImportPayload};
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use crate::tree::{MeshKey, SceneNode};
    use tequila_core::math::vec3;
    use tequila_materials::material_type::MaterialType;

    /// Two groups, each with two parts. The first group's parts share one
    /// material; the second group's part has its own.
    fn assembly() -> (SceneDocument, Vec<NodeId>, MaterialKey, MaterialKey) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();

        let mesh = Mesh::new(
            vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let mesh_key = MeshKey(1);
        let (shared, own) = (MaterialKey(1), MaterialKey(2));
        let ids: Vec<NodeId> = (1..=6).map(NodeId).collect();

        let mut nodes = vec![
            SceneNode::new(ids[0], "Assembly", NodeKind::Group),
            SceneNode::new(ids[1], "Front", NodeKind::Group),
            SceneNode::new(ids[2], "Rear", NodeKind::Group),
        ];
        nodes[1].parent = Some(ids[0]);
        nodes[2].parent = Some(ids[0]);

        for (i, (&id, parent)) in ids[3..]
            .iter()
            .zip([ids[1], ids[1], ids[2]])
            .enumerate()
        {
            let mut part = SceneNode::new(id, format!("Part {i}"), NodeKind::Part { mesh: mesh_key });
            part.parent = Some(parent);
            part.material = Some(if i < 2 { shared } else { own });
            nodes.push(part);
        }

        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(mesh_key, mesh)],
                    textures: Vec::new(),
                    materials: vec![
                        (shared, MaterialDefinition::preset("Shared", MaterialType::Plastic)),
                        (own, MaterialDefinition::preset("Own", MaterialType::Metal)),
                    ],
                    nodes,
                    roots: vec![ids[0]],
                }),
            },
        )
        .expect("builds");

        (document, ids, shared, own)
    }

    /// The plain drop, and the sentence that removes the surprise: it says how
    /// many other parts are about to change.
    #[test]
    fn a_plain_drop_says_how_many_parts_share_the_material() {
        let (document, ids, _, _) = assembly();
        let plan = plan_drop(&document, ids[3], DropModifiers::NONE).expect("plans");

        assert_eq!(plan.affected, vec![ids[3]]);
        assert!(!plan.unlink);
        assert_eq!(plan.shared_with, 2);
        assert!(
            plan.description.contains("2 parts share"),
            "the label must warn that other parts change: {:?}",
            plan.description
        );
    }

    /// A part nothing else shares needs no warning, and must not get one — a
    /// caveat on every drop is a caveat nobody reads.
    #[test]
    fn a_sole_user_gets_no_warning() {
        let (document, ids, _, _) = assembly();
        let plan = plan_drop(&document, ids[5], DropModifiers::NONE).expect("plans");
        assert_eq!(plan.shared_with, 1);
        assert!(
            !plan.description.contains("share"),
            "an unshared material was described as shared: {:?}",
            plan.description
        );
    }

    /// Alt unlinks, and says so.
    #[test]
    fn alt_unlinks_only_the_target() {
        let (document, ids, _, _) = assembly();
        let plan = plan_drop(
            &document,
            ids[3],
            DropModifiers {
                alt: true,
                ..DropModifiers::NONE
            },
        )
        .expect("plans");

        assert!(plan.unlink);
        assert_eq!(plan.affected, vec![ids[3]]);
        assert!(plan.description.contains("unlink"), "{:?}", plan.description);
    }

    /// Shift widens to the group.
    #[test]
    fn shift_applies_to_the_whole_group() {
        let (document, ids, _, _) = assembly();
        let plan = plan_drop(
            &document,
            ids[3],
            DropModifiers {
                shift: true,
                ..DropModifiers::NONE
            },
        )
        .expect("plans");

        assert_eq!(plan.affected, vec![ids[3], ids[4]], "both parts of `Front`");
        assert!(plan.description.contains("Front"), "{:?}", plan.description);
    }

    /// Ctrl widens to everything sharing the current material — which may span
    /// groups, and that is the point of it.
    #[test]
    fn ctrl_applies_to_everything_sharing_the_material() {
        let (mut document, ids, shared, _) = assembly();
        // Give a part in the other group the shared material too.
        document.tree.get_mut(ids[5]).unwrap().material = Some(shared);

        let plan = plan_drop(
            &document,
            ids[3],
            DropModifiers {
                ctrl: true,
                ..DropModifiers::NONE
            },
        )
        .expect("plans");

        assert_eq!(plan.affected.len(), 3, "got {:?}", plan.affected);
        assert!(plan.affected.contains(&ids[5]), "did not cross the group");
        assert!(plan.description.contains("Shared"), "{:?}", plan.description);
    }

    /// Holding two modifiers must give a defined answer rather than whichever
    /// branch was written first. Alt wins, because it is the only one that
    /// changes what kind of operation this is.
    #[test]
    fn alt_wins_over_the_other_modifiers() {
        let (document, ids, _, _) = assembly();
        let plan = plan_drop(
            &document,
            ids[3],
            DropModifiers {
                alt: true,
                shift: true,
                ctrl: true,
            },
        )
        .expect("plans");

        assert!(plan.unlink);
        assert_eq!(plan.affected, vec![ids[3]]);
    }

    /// §3.3: dropping on a group applies to everything beneath it, with no
    /// modifier needed — a group has no material of its own to replace.
    #[test]
    fn dropping_on_a_group_covers_everything_beneath_it() {
        let (document, ids, _, _) = assembly();
        let plan = plan_drop(&document, ids[0], DropModifiers::NONE).expect("plans");
        assert_eq!(plan.affected.len(), 3, "every part in the assembly");

        let front = plan_drop(&document, ids[1], DropModifiers::NONE).expect("plans");
        assert_eq!(front.affected, vec![ids[3], ids[4]]);
    }

    /// Ctrl on a part with no material has nothing to match, and has to say so
    /// rather than silently applying to everything or to nothing.
    #[test]
    fn ctrl_on_an_unassigned_part_explains_itself() {
        let (mut document, ids, _, _) = assembly();
        document.tree.get_mut(ids[5]).unwrap().material = None;

        let plan = plan_drop(
            &document,
            ids[5],
            DropModifiers {
                ctrl: true,
                ..DropModifiers::NONE
            },
        )
        .expect("plans");

        assert_eq!(plan.affected, vec![ids[5]]);
        assert!(
            plan.description.contains("no material"),
            "{:?}",
            plan.description
        );
    }

    /// The preview and the drop must resolve identically, or the label promises
    /// something the drop does not do.
    #[test]
    fn planning_twice_gives_the_same_answer() {
        let (document, ids, _, _) = assembly();
        for modifiers in [
            DropModifiers::NONE,
            DropModifiers { alt: true, ..DropModifiers::NONE },
            DropModifiers { shift: true, ..DropModifiers::NONE },
            DropModifiers { ctrl: true, ..DropModifiers::NONE },
        ] {
            let a = plan_drop(&document, ids[3], modifiers).expect("plans");
            let b = plan_drop(&document, ids[3], modifiers).expect("plans");
            assert_eq!(a, b);
            assert!(same_effect(&a, &b));
        }
    }

    /// A drop on a selected part goes on the whole selection; one on a part
    /// inside a selected group, on everything selected; and one beside the
    /// selection on the part alone, its label saying how to reach them all.
    #[test]
    fn a_drop_on_the_selection_covers_all_of_it() {
        let (document, ids, _, _) = assembly();
        // Two parts in different groups.
        let selected = [ids[3], ids[5]];
        let plan = plan_drop_in(&document, ids[5], DropModifiers::NONE, &selected).expect("plans");
        assert_eq!(plan.affected, vec![ids[3], ids[5]]);
        assert!(plan.description.contains("2 selected parts"), "{:?}", plan.description);

        // A group selected, the drop on a part in it.
        let plan = plan_drop_in(&document, ids[4], DropModifiers::NONE, &[ids[1]]).expect("plans");
        assert_eq!(plan.affected, vec![ids[3], ids[4]], "both of `Front`'s parts");

        // Beside the selection: the part alone.
        let plan = plan_drop_in(&document, ids[4], DropModifiers::NONE, &selected).expect("plans");
        assert_eq!(plan.affected, vec![ids[4]]);
        assert!(plan.description.contains("selected part for all 2"), "{:?}", plan.description);

        // A modifier says what it always said, selection or not.
        let alt = DropModifiers { alt: true, ..DropModifiers::NONE };
        assert_eq!(plan_drop_in(&document, ids[5], alt, &selected), plan_drop(&document, ids[5], alt));
        let shift = DropModifiers { shift: true, ..DropModifiers::NONE };
        assert_eq!(plan_drop_in(&document, ids[5], shift, &selected), plan_drop(&document, ids[5], shift));
        // Only the target selected, or nothing: the plain drop, warning and all.
        assert_eq!(plan_drop_in(&document, ids[3], DropModifiers::NONE, &[ids[3]]), plan_drop(&document, ids[3], DropModifiers::NONE));
        assert_eq!(plan_drop_in(&document, ids[3], DropModifiers::NONE, &[]), plan_drop(&document, ids[3], DropModifiers::NONE));
    }

    #[test]
    fn a_drop_on_nothing_plans_nothing() {
        let (document, _, _, _) = assembly();
        assert!(plan_drop(&document, NodeId(9999), DropModifiers::NONE).is_none());
    }
}
