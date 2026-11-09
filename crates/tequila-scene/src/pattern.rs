//! Patterns: a part repeated along a line or around a circle.
//!
//! A pattern is a group holding the part and its copies, with the
//! pattern's settings on the group ([`SceneNode::pattern`]). The copies are
//! parts like any other - picked, exported, framed and rendered as the
//! rest are - that share the original's mesh and material: a pattern of a
//! hundred screws holds one screw's triangles, and a material dropped on
//! any of them is on all of them. Changing the settings
//! (`Command::SetPattern`) takes the copies away and lays them out again,
//! from the original as it is then, so the pattern stays editable.

use crate::document::SceneDocument;
use crate::tree::{NodeId, NodeKind, SceneNode};
use crate::CommandError;
use tequila_core::math::{vec3, Mat4, Transform, Vec3};

/// The most copies a pattern makes, the original included: enough for a
/// grille, and a mistyped zero short of a frozen window.
pub const MOST: u32 = 500;

/// A pattern's settings, kept on its group.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    /// How many there are, the original included: 2 to [`MOST`].
    pub count: u32,
    /// The part repeated, a child of the group.
    pub source: NodeId,
    /// The copies, children of the group, in order out from the original:
    /// the ids `Command::SetPattern` gives them, so a redo and a replayed
    /// journal make the same nodes.
    pub copies: Vec<NodeId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PatternKind {
    /// Each copy `step` on from the one before, in the group's space.
    Line { step: Vec3 },
    /// Around `axis` through `centre`, in the group's space, `angle`
    /// radians from the original to the last copy - or a full turn, evenly
    /// around it, without a copy on top of the original.
    Circle { centre: Vec3, axis: Vec3, angle: f32 },
}

impl PatternKind {
    pub fn label(&self) -> &'static str {
        match self {
            PatternKind::Line { .. } => "Line pattern",
            PatternKind::Circle { .. } => "Circle pattern",
        }
    }
}

impl Pattern {
    /// Where copy `index` (1 is the first copy, `count - 1` the last) is,
    /// as a transform applied over the original's, in the group's space.
    pub fn offset(&self, index: u32) -> Mat4 {
        let i = index as f32;
        match self.kind {
            PatternKind::Line { step } => Mat4::translation(step * i),
            PatternKind::Circle { centre, axis, angle } => {
                let axis = axis.try_normalize().unwrap_or(Vec3::Y);
                let full = (angle.abs() - std::f32::consts::TAU).abs() < 1e-3;
                let spaces = if full { self.count } else { self.count.saturating_sub(1) }.max(1) as f32;
                Mat4::translation(centre) * Mat4::rotation(axis, angle * i / spaces) * Mat4::translation(-centre)
            }
        }
    }

    /// The count, held between 2 and [`MOST`].
    pub fn clamped(mut self) -> Pattern {
        self.count = self.count.clamp(2, MOST);
        self
    }
}

/// A new pattern of `part`, with settings that show it at once: five along
/// the part's width with a fifth of it between each, or six around a
/// circle beside it, about the vertical. Ids for the copies are reserved
/// from `document`; the group comes from `Command::Group` before it.
pub fn starting(document: &SceneDocument, part: NodeId, circle: bool) -> Option<Pattern> {
    let node = document.tree.get(part)?;
    node.mesh()?;
    let bounds = crate::snapshot::visible_bounds_under(document, &[part]);
    let size = if bounds.is_empty() { vec3(0.1, 0.1, 0.1) } else { bounds.diagonal() };
    let width = size.x.max(size.z).max(1e-3);
    let local = node.transform.point(Vec3::ZERO);
    let (kind, count) = if circle {
        // The circle's radius twice the part's width, so six copies stand
        // clear of each other; its centre that far to the part's left.
        (PatternKind::Circle { centre: local - vec3(width * 2.0, 0.0, 0.0), axis: Vec3::Y, angle: std::f32::consts::TAU }, 6)
    } else {
        (PatternKind::Line { step: vec3(size.x.max(1e-3) * 1.2, 0.0, 0.0) }, 5)
    };
    Some(Pattern { kind, count, source: part, copies: Vec::new() }.with_ids(document, None))
}

impl Pattern {
    /// The same settings with ids for its copies: the ones it has that are
    /// still copies in `group` (or free), then fresh ones from `document`.
    /// A copy dragged out of the pattern keeps its id and is left alone.
    pub fn with_ids(mut self, document: &SceneDocument, group: Option<NodeId>) -> Pattern {
        let wanted = self.count.clamp(2, MOST) as usize - 1;
        self.copies.retain(|&id| document.tree.get(id).is_none_or(|n| group.is_some() && n.parent == group));
        self.copies.truncate(wanted);
        let mut next = document.tree.next_id().max(self.copies.iter().map(|id| id.0 + 1).max().unwrap_or(0));
        while self.copies.len() < wanted {
            self.copies.push(NodeId(next));
            next += 1;
        }
        self
    }
}

/// The copies `pattern` makes in `group`, as nodes ready to insert: each
/// the original's mesh and material, its visibility and rays, at its
/// offset, named after it with its number. Checked first: the group is a
/// group, the original a part in it, the ids free or the group's copies'.
pub fn copies(document: &SceneDocument, group: NodeId, pattern: &Pattern) -> Result<Vec<SceneNode>, CommandError> {
    let holder = document.tree.try_get(group)?;
    if holder.kind != NodeKind::Group {
        return Err(CommandError::InconsistentHistory("a pattern belongs to a group"));
    }
    let source = document.tree.try_get(pattern.source)?;
    if source.parent != Some(group) || source.mesh().is_none() {
        return Err(CommandError::InconsistentHistory("a pattern repeats a part in its group"));
    }
    if !(2..=MOST).contains(&pattern.count) || pattern.copies.len() != pattern.count as usize - 1 {
        return Err(CommandError::InconsistentHistory("a pattern has a copy id for each copy"));
    }
    let current = current_copies(document, group);
    let mut seen = std::collections::HashSet::new();
    for &id in &pattern.copies {
        if id == pattern.source || id == group || !seen.insert(id) || (document.tree.contains(id) && !current.contains(&id)) {
            return Err(CommandError::InconsistentHistory("a pattern's copy ids must be free"));
        }
    }
    let mut out = Vec::with_capacity(pattern.copies.len());
    for (i, &id) in pattern.copies.iter().enumerate() {
        let mut copy = SceneNode::new(id, format!("{} {}", source.name, i + 2), source.kind.clone());
        copy.parent = Some(group);
        copy.transform = Transform::from_matrix(pattern.offset(i as u32 + 1) * source.transform.matrix).unwrap_or(source.transform);
        copy.material = source.material;
        copy.visible = source.visible;
        copy.rays = source.rays;
        copy.layers = source.layers.clone();
        out.push(copy);
    }
    Ok(out)
}

/// The copies `group`'s pattern has now that are still in it: what a change
/// to the pattern takes away. One dragged out of the group is a part of its
/// own by then, and stays.
pub fn current_copies(document: &SceneDocument, group: NodeId) -> Vec<NodeId> {
    let Some(pattern) = document.tree.get(group).and_then(|g| g.pattern.as_ref()) else { return Vec::new() };
    pattern.copies.iter().copied().filter(|&id| document.tree.get(id).is_some_and(|n| n.parent == Some(group))).collect()
}

/// The pattern a node belongs to: the group itself, or the group of the
/// original or a copy.
pub fn pattern_of(document: &SceneDocument, node: NodeId) -> Option<NodeId> {
    let n = document.tree.get(node)?;
    if n.pattern.is_some() {
        return Some(node);
    }
    let parent = n.parent?;
    document.tree.get(parent)?.pattern.as_ref().filter(|p| p.source == node || p.copies.contains(&node)).map(|_| parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{reserve_keys, Command, ImportPayload};
    use crate::material::MaterialDefinition;
    use crate::CommandBus;

    /// A document with one 2 cm part at (1, 0, 0), grouped, and the group.
    fn scene() -> (SceneDocument, CommandBus, NodeId, NodeId) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 1);
        let mesh = crate::geometry::Mesh::new(vec![Vec3::ZERO, vec3(0.02, 0.0, 0.0), vec3(0.0, 0.02, 0.0)], vec![0, 1, 2]);
        let mut part = SceneNode::new(ids[0], "Screw", NodeKind::Part { mesh: meshes[0] });
        part.transform = Transform::translation(vec3(1.0, 0.0, 0.0));
        part.material = Some(materials[0]);
        let payload = ImportPayload { meshes: vec![(meshes[0], mesh)], textures: Vec::new(), materials: vec![(materials[0], MaterialDefinition::plastic("Steel"))], nodes: vec![part], roots: vec![ids[0]] };
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        bus.apply(&mut document, Command::Group { nodes: vec![ids[0]], name: "Line pattern".into() }).unwrap();
        let group = document.tree.get(ids[0]).unwrap().parent.unwrap();
        (document, bus, group, ids[0])
    }

    fn set(bus: &mut CommandBus, document: &mut SceneDocument, group: NodeId, pattern: Pattern) {
        let pattern = pattern.with_ids(document, Some(group));
        bus.apply(document, Command::SetPattern { group, pattern: Some(Box::new(pattern)) }).unwrap();
    }

    fn positions(document: &SceneDocument, group: NodeId) -> Vec<Vec3> {
        document.tree.get(group).unwrap().children.iter().map(|&c| document.tree.world_transform(c).point(Vec3::ZERO)).collect()
    }

    /// Five along a line: the original and four copies a step apart, each
    /// sharing its mesh and its material; a count of three takes two away;
    /// one undo puts four back; clearing it leaves the part in its group.
    #[test]
    fn a_line_pattern_lays_out_shared_copies_and_undoes() {
        let (mut document, mut bus, group, part) = scene();
        let step = vec3(0.05, 0.0, 0.0);
        set(&mut bus, &mut document, group, Pattern { kind: PatternKind::Line { step }, count: 5, source: part, copies: Vec::new() });
        let at = positions(&document, group);
        assert_eq!(at.len(), 5);
        for (i, p) in at.iter().enumerate() {
            assert!((*p - vec3(1.0 + 0.05 * i as f32, 0.0, 0.0)).length() < 1e-5, "{i}: {p:?}");
        }
        let original = document.tree.get(part).unwrap().clone();
        for &copy in &document.tree.get(group).unwrap().children[1..] {
            let copy = document.tree.get(copy).unwrap();
            assert_eq!((copy.mesh(), copy.material), (original.mesh(), original.material), "shares the mesh and material");
        }
        assert_eq!(document.tree.get(document.tree.get(group).unwrap().children[1]).unwrap().name, "Screw 2");
        let geometry = document.geometry.len();
        bus.end_interaction();

        let copies = document.tree.get(group).unwrap().pattern.clone().unwrap().copies;
        set(&mut bus, &mut document, group, Pattern { kind: PatternKind::Line { step }, count: 3, source: part, copies });
        assert_eq!(positions(&document, group).len(), 3, "a count of three");
        assert_eq!(document.geometry.len(), geometry, "no mesh made for a copy");

        bus.end_interaction();
        bus.undo(&mut document).unwrap();
        assert_eq!(positions(&document, group).len(), 5, "undone, five again");
        bus.redo(&mut document).unwrap();
        assert_eq!(positions(&document, group).len(), 3, "redone, three");

        bus.apply(&mut document, Command::SetPattern { group, pattern: None }).unwrap();
        assert_eq!(document.tree.get(group).unwrap().children, vec![part], "cleared: the part alone in its group");
        assert!(document.tree.get(group).unwrap().pattern.is_none());
    }

    /// Counts dragged one after another merge into one step, and undoing it
    /// leaves the pattern as it was before the drag - however many times
    /// the copies were replaced in between.
    #[test]
    fn a_dragged_count_is_one_step_to_undo() {
        let (mut document, mut bus, group, part) = scene();
        let line = |count| Pattern { kind: PatternKind::Line { step: vec3(0.05, 0.0, 0.0) }, count, source: part, copies: Vec::new() };
        set(&mut bus, &mut document, group, line(2));
        bus.end_interaction();
        for count in [3, 4, 6, 9, 7] {
            let copies = document.tree.get(group).unwrap().pattern.clone().unwrap().copies;
            set(&mut bus, &mut document, group, Pattern { copies, ..line(count) });
        }
        assert_eq!(positions(&document, group).len(), 7);
        bus.undo(&mut document).unwrap();
        assert_eq!(positions(&document, group).len(), 2, "back to before the drag");
        assert_eq!(document.tree.iter().count(), 3, "the group, the part and one copy: nothing left over");
    }

    /// A full circle of six: sixty degrees apart about the centre, none on
    /// top of the original; a half circle of five ends at the half.
    #[test]
    fn a_circle_pattern_goes_evenly_round() {
        let (mut document, mut bus, group, part) = scene();
        set(&mut bus, &mut document, group, Pattern { kind: PatternKind::Circle { centre: Vec3::ZERO, axis: Vec3::Y, angle: std::f32::consts::TAU }, count: 6, source: part, copies: Vec::new() });
        let at = positions(&document, group);
        assert_eq!(at.len(), 6);
        for (i, p) in at.iter().enumerate() {
            let turn = (i as f32) * std::f32::consts::TAU / 6.0;
            let want = vec3(turn.cos(), 0.0, -turn.sin());
            assert!((*p - want).length() < 1e-4, "{i}: {p:?}, not {want:?}");
        }
        let copies = document.tree.get(group).unwrap().pattern.clone().unwrap().copies;
        set(&mut bus, &mut document, group, Pattern { kind: PatternKind::Circle { centre: Vec3::ZERO, axis: Vec3::Y, angle: std::f32::consts::PI }, count: 5, source: part, copies });
        let last = *positions(&document, group).last().unwrap();
        assert!((last - vec3(-1.0, 0.0, 0.0)).length() < 1e-4, "the last copy at the half turn: {last:?}");
    }

    /// A copy dragged out of the pattern is a part of its own: a change to
    /// the pattern lays out the rest and leaves it where it was put.
    #[test]
    fn a_copy_taken_out_of_the_pattern_is_left_alone() {
        let (mut document, mut bus, group, part) = scene();
        let line = |count, copies| Pattern { kind: PatternKind::Line { step: vec3(0.05, 0.0, 0.0) }, count, source: part, copies };
        set(&mut bus, &mut document, group, line(4, Vec::new()));
        let taken = document.tree.get(group).unwrap().children[2];
        bus.apply(&mut document, Command::Reparent { node: taken, parent: None, index: None }).unwrap();
        let copies = document.tree.get(group).unwrap().pattern.clone().unwrap().copies;
        set(&mut bus, &mut document, group, line(4, copies));
        assert!(document.tree.contains(taken) && document.tree.get(taken).unwrap().parent.is_none(), "still there, at the top");
        assert_eq!(positions(&document, group).len(), 4, "and the pattern whole again without it");
    }

    /// Duplicated, a pattern is a pattern of its own copies.
    #[test]
    fn a_duplicated_pattern_repeats_its_own_part() {
        let (mut document, mut bus, group, part) = scene();
        set(&mut bus, &mut document, group, Pattern { kind: PatternKind::Line { step: vec3(0.05, 0.0, 0.0) }, count: 3, source: part, copies: Vec::new() });
        let (payload, roots) = crate::duplicate::payload(&document, &[group]).unwrap();
        bus.apply(&mut document, Command::Import { payload: Box::new(payload) }).unwrap();
        let twin = document.tree.get(roots[0]).unwrap();
        let pattern = twin.pattern.clone().unwrap();
        assert_eq!(pattern.source, twin.children[0]);
        assert_eq!(pattern.copies, twin.children[1..].to_vec());
    }
}
