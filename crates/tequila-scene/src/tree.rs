//! The scene tree.
//!
//! `docs/03-SCENE-AND-IO.md` §1 states the two properties this module has to
//! guarantee:
//!
//! - **The Scene Document is the single source of truth.** Only the command
//!   bus writes to it, so every mutating method here is `pub(crate)` or takes
//!   a token that only a command can produce.
//! - **IDs are stable across sessions.** A `NodeId` is assigned once and never
//!   reused. §1 point 4 explains the stakes: if IDs move, material assignments
//!   break every time the model changes, "which is the number-one workflow
//!   failure in CAD visualization".
//!
//! # Determinism
//!
//! M1's exit criterion is that a document survives save, reopen and comparison
//! **byte for byte**. That rules out `HashMap` anywhere the contents are
//! serialized: its iteration order varies between runs, so the same document
//! would write different bytes each time. Node storage is a `BTreeMap` and
//! child lists are explicit `Vec`s, so ordering is a property of the document
//! rather than of the allocator.

use crate::lighting::LightSettings;
use tequila_core::math::{Aabb, Transform, Vec3};
use std::collections::{BTreeMap, BTreeSet};

/// A stable node identifier.
///
/// `u64` rather than the `u32` of the render-side IDs: these must never be
/// reused within a document, so the space has to outlast every edit a long-
/// lived project sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "node{}", self.0)
    }
}

/// Reference to a mesh in the geometry store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshKey(pub u64);

/// Reference to a material in the material store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialKey(pub u64);

/// What a node is.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// An organizational container with no geometry of its own.
    Group,
    /// Geometry. Several parts may reference one mesh — that *is* instancing
    /// (§2.2), not an optimization applied later.
    Part { mesh: MeshKey },
    /// A physical light (§01 §5.1).
    ///
    /// A node rather than an entry in a separate light list, because that is
    /// what gives a light selection, the transform gizmo, grouping, hiding,
    /// locking, undo and serialization without any of those being written a
    /// second time. Its *placement* is the node's transform — the settings
    /// here are only what a transform cannot express.
    Light { settings: Box<LightSettings> },
    /// A cutaway volume (`docs/02` §3): removes what is inside it from the
    /// render and caps what it cuts. A node for the same reasons a light is
    /// one — the gizmo, grouping, hiding, undo and saving come with it — and
    /// its size and place are the node's transform.
    Cutaway { settings: Box<crate::cutaway::CutawaySettings> },
}

impl NodeKind {
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Group => "Group",
            NodeKind::Part { .. } => "Part",
            NodeKind::Light { settings } => settings.shape.label(),
            NodeKind::Cutaway { .. } => "Cutaway",
        }
    }

    /// The light this node is, if it is one.
    pub fn light(&self) -> Option<&LightSettings> {
        match self {
            NodeKind::Light { settings } => Some(settings),
            _ => None,
        }
    }
}

/// Which rays see a part, and everything under it: the camera, the light
/// others' surfaces gather (diffuse), their reflections (glossy), what shows
/// through glass (transmission), and shadows. And holdout, which cuts the
/// part out of the picture with no alpha, for compositing. A part none of
/// whose rays see it still blocks nothing and lights nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rays {
    pub camera: bool,
    pub diffuse: bool,
    pub glossy: bool,
    pub transmission: bool,
    pub shadow: bool,
    pub holdout: bool,
}

impl Default for Rays {
    fn default() -> Self {
        Rays { camera: true, diffuse: true, glossy: true, transmission: true, shadow: true, holdout: false }
    }
}

impl Rays {
    pub fn is_default(&self) -> bool {
        *self == Rays::default()
    }

    /// A node's own under its parent's: seen only by what sees both, held
    /// out if either is.
    pub fn under(self, parent: Rays) -> Rays {
        Rays {
            camera: self.camera && parent.camera,
            diffuse: self.diffuse && parent.diffuse,
            glossy: self.glossy && parent.glossy,
            transmission: self.transmission && parent.transmission,
            shadow: self.shadow && parent.shadow,
            holdout: self.holdout || parent.holdout,
        }
    }
}

/// One node of the scene tree.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub name: String,
    pub kind: NodeKind,
    /// Local transform, relative to the parent.
    pub transform: Transform,
    pub visible: bool,
    pub locked: bool,
    /// The rays that see it (`Rays`); its parent's limit it too.
    pub rays: Rays,
    pub material: Option<MaterialKey>,
    /// CAD attributes and user annotations. Ordered, so the document
    /// serializes identically every time (§4.1).
    pub metadata: BTreeMap<String, String>,
    /// The CAD layers the file put it on (an IGES file's levels), in the
    /// file's order: the groups `crate::layers` shows and hides together.
    /// Kept apart from `metadata` because a layer's name is the file's and
    /// may hold any character, a comma included, and because a part may be
    /// on several.
    pub layers: Vec<String>,
    pub children: Vec<NodeId>,
    /// A custom pivot, in the node's own local space.
    ///
    /// Stored on the node rather than held by the gizmo because §04 §5 requires
    /// pivots to persist — they are what an animation track rotates about, so
    /// one that lived only in the UI would be lost the moment the file was
    /// saved.
    ///
    /// Local space, so it travels with the part when the part moves. `None`
    /// means the node has no custom pivot and the gizmo falls back to one of
    /// the derived modes.
    ///
    /// §03 §2.4 describes *several* named pivots per node, each usable by a
    /// different animation track. That arrives with animation in M5; one
    /// unnamed pivot is what a transform gizmo needs today.
    pub pivot: Option<Vec3>,
    /// The persistent path key of §2.3: `(parent path, name, occurrence)` from
    /// the source file. Re-import matches on this to preserve material
    /// assignments across an updated CAD file.
    pub path_key: Option<String>,
}

impl SceneNode {
    /// Build a detached node.
    ///
    /// Public because importers construct whole subtrees before handing them
    /// to `Command::Import` — the node has to exist before there is a document
    /// to insert it into. It is inert until inserted.
    pub fn new(id: NodeId, name: impl Into<String>, kind: NodeKind) -> Self {
        SceneNode {
            id,
            parent: None,
            name: name.into(),
            kind,
            transform: Transform::IDENTITY,
            visible: true,
            locked: false,
            rays: Rays::default(),
            material: None,
            metadata: BTreeMap::new(),
            layers: Vec::new(),
            children: Vec::new(),
            pivot: None,
            path_key: None,
        }
    }

    #[inline]
    pub fn mesh(&self) -> Option<MeshKey> {
        match self.kind {
            NodeKind::Part { mesh } => Some(mesh),
            NodeKind::Group | NodeKind::Light { .. } | NodeKind::Cutaway { .. } => None,
        }
    }

    /// The light this node is, if it is one.
    #[inline]
    pub fn light(&self) -> Option<&LightSettings> {
        self.kind.light()
    }
}

/// Why a tree operation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    NoSuchNode(NodeId),
    /// Reparenting a node beneath itself or one of its own descendants would
    /// detach the subtree from the tree entirely and leak it.
    WouldCycle {
        node: NodeId,
        parent: NodeId,
    },
    /// The node is locked, and locking exists precisely to refuse edits.
    Locked(NodeId),
}

impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TreeError::NoSuchNode(id) => write!(f, "no such node: {id}"),
            TreeError::WouldCycle { node, parent } => {
                write!(
                    f,
                    "cannot reparent {node} beneath its own descendant {parent}"
                )
            }
            TreeError::Locked(id) => write!(f, "{id} is locked"),
        }
    }
}

impl std::error::Error for TreeError {}

/// The hierarchy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneTree {
    nodes: BTreeMap<NodeId, SceneNode>,
    roots: Vec<NodeId>,
    next_id: u64,
}

impl SceneTree {
    pub fn new() -> Self {
        SceneTree {
            nodes: BTreeMap::new(),
            roots: Vec::new(),
            next_id: 1,
        }
    }

    /// Allocate the next identifier.
    ///
    /// Monotonic and never rewound, even by deletion: an ID freed and reissued
    /// would let a stale material assignment or animation track silently
    /// re-attach to an unrelated part.
    fn allocate_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    /// The highest ID ever issued. Persisted with the document so that reopening
    /// it does not restart the counter and collide with existing nodes.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Restore the allocator's position when loading a document.
    pub fn set_next_id(&mut self, next: u64) {
        self.next_id = self.next_id.max(next);
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn get(&self, id: NodeId) -> Option<&SceneNode> {
        self.nodes.get(&id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn try_get(&self, id: NodeId) -> Result<&SceneNode, TreeError> {
        self.nodes.get(&id).ok_or(TreeError::NoSuchNode(id))
    }

    pub(crate) fn get_mut(&mut self, id: NodeId) -> Result<&mut SceneNode, TreeError> {
        self.nodes.get_mut(&id).ok_or(TreeError::NoSuchNode(id))
    }

    /// Every node, in ID order. Deterministic, which is what the format needs.
    pub fn iter(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.values()
    }

    /// Insert a node under `parent`, or at the root when `parent` is `None`.
    pub(crate) fn insert(
        &mut self,
        parent: Option<NodeId>,
        name: impl Into<String>,
        kind: NodeKind,
    ) -> Result<NodeId, TreeError> {
        if let Some(parent) = parent {
            if !self.contains(parent) {
                return Err(TreeError::NoSuchNode(parent));
            }
        }
        let id = self.allocate_id();
        self.insert_with_id(id, parent, name, kind)
    }

    /// Insert a node with a chosen ID rather than a freshly allocated one.
    ///
    /// Redoing a command that created a node must re-create it under the *same*
    /// ID: a material assignment, animation track or selection recorded against
    /// the original would otherwise point at nothing after undo-then-redo. The
    /// allocator watermark is raised past `id` so nothing can later collide
    /// with it.
    pub(crate) fn insert_with_id(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        name: impl Into<String>,
        kind: NodeKind,
    ) -> Result<NodeId, TreeError> {
        if let Some(parent) = parent {
            if !self.contains(parent) {
                return Err(TreeError::NoSuchNode(parent));
            }
        }
        self.next_id = self.next_id.max(id.0 + 1);
        let mut node = SceneNode::new(id, name, kind);
        node.parent = parent;
        self.nodes.insert(id, node);
        match parent {
            Some(parent) => self.nodes.get_mut(&parent).unwrap().children.push(id),
            None => self.roots.push(id),
        }
        Ok(id)
    }

    /// Rebuild a whole tree from stored nodes, for a loader.
    ///
    /// Public, unlike the other mutators, and the distinction is deliberate:
    /// the command bus owns every *edit* to a live document, but a loader does
    /// not edit anything — it constructs a document that did not exist a
    /// moment ago, from a file. Routing that through eighteen commands would
    /// produce an undo history of a load, which is not a thing a user can undo.
    ///
    /// The nodes are trusted for their content and not for their structure:
    /// parent and child links come from a file that may have been edited by
    /// hand or damaged, so they are checked against each other before the tree
    /// is accepted. A cycle here would hang the first depth-first walk.
    pub fn restore(&mut self, nodes: Vec<SceneNode>, roots: Vec<NodeId>) -> Result<(), String> {
        self.nodes.clear();
        self.roots.clear();

        let mut highest = 0;
        for node in nodes {
            highest = highest.max(node.id.0);
            if self.nodes.insert(node.id, node).is_some() {
                return Err("two nodes share an ID".into());
            }
        }
        self.roots = roots;
        self.next_id = self.next_id.max(highest + 1);

        // Parent and child links must agree, or a walk down and a walk up
        // disagree about the shape of the scene.
        for (&id, node) in &self.nodes {
            match node.parent {
                Some(parent) => {
                    let parent_node = self
                        .nodes
                        .get(&parent)
                        .ok_or_else(|| format!("{id} names missing parent {parent}"))?;
                    if !parent_node.children.contains(&id) {
                        return Err(format!(
                            "{id} claims {parent} as parent, which does not list it"
                        ));
                    }
                }
                None => {
                    if !self.roots.contains(&id) {
                        return Err(format!("{id} has no parent but is not a root"));
                    }
                }
            }
            for &child in &node.children {
                let child_node = self
                    .nodes
                    .get(&child)
                    .ok_or_else(|| format!("{id} lists missing child {child}"))?;
                if child_node.parent != Some(id) {
                    return Err(format!(
                        "{id} lists {child}, which names a different parent"
                    ));
                }
            }
        }
        for &root in &self.roots {
            if !self.nodes.contains_key(&root) {
                return Err(format!("root {root} does not exist"));
            }
        }

        // `validate` catches the rest: cycles, duplicate listings, and nodes
        // unreachable from any root.
        self.validate()
    }

    /// Re-insert a node with a specific ID and position.
    ///
    /// Undoing a deletion has to restore the *same* IDs, or every reference
    /// into the deleted subtree — material assignments, animation tracks,
    /// selection — would dangle after the undo.
    pub(crate) fn reinsert(&mut self, node: SceneNode, index: usize) -> Result<(), TreeError> {
        let id = node.id;
        let parent = node.parent;
        // The parent first: inserted and then refused, the node stayed in
        // the map with nothing pointing at it.
        if let Some(parent) = parent {
            if !self.nodes.contains_key(&parent) {
                return Err(TreeError::NoSuchNode(parent));
            }
        }
        self.next_id = self.next_id.max(id.0 + 1);
        self.nodes.insert(id, node);
        let siblings = match parent {
            Some(parent) => {
                &mut self
                    .nodes
                    .get_mut(&parent)
                    .ok_or(TreeError::NoSuchNode(parent))?
                    .children
            }
            None => &mut self.roots,
        };
        // A removed subtree keeps its child lists, so restoring a parent
        // already restores its links. Only the subtree's own root needs
        // re-attaching; appending the rest again would list them twice.
        if !siblings.contains(&id) {
            siblings.insert(index.min(siblings.len()), id);
        }
        Ok(())
    }

    /// Position of a node among its siblings.
    pub fn sibling_index(&self, id: NodeId) -> Option<usize> {
        let node = self.get(id)?;
        let siblings = match node.parent {
            Some(parent) => &self.get(parent)?.children,
            None => &self.roots,
        };
        siblings.iter().position(|&child| child == id)
    }

    /// Detach a node from its parent's child list, leaving it in the map.
    fn detach(&mut self, id: NodeId) -> Result<(), TreeError> {
        let parent = self.try_get(id)?.parent;
        let siblings = match parent {
            Some(parent) => &mut self.get_mut(parent)?.children,
            None => &mut self.roots,
        };
        siblings.retain(|&child| child != id);
        Ok(())
    }

    /// Remove a node and everything under it, returning the removed nodes in
    /// an order that [`Self::reinsert`] can replay.
    pub(crate) fn remove_subtree(&mut self, id: NodeId) -> Result<Vec<SceneNode>, TreeError> {
        self.try_get(id)?;
        let ids = self.descendants_and_self(id);
        self.detach(id)?;
        // Deepest first, so re-inserting in reverse order always finds the
        // parent already present.
        let mut removed = Vec::with_capacity(ids.len());
        for node_id in ids.iter().rev() {
            if let Some(node) = self.nodes.remove(node_id) {
                removed.push(node);
            }
        }
        Ok(removed)
    }

    /// Move a node under a new parent.
    pub(crate) fn reparent(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        index: Option<usize>,
    ) -> Result<(), TreeError> {
        self.try_get(id)?;
        if let Some(parent) = new_parent {
            if !self.contains(parent) {
                return Err(TreeError::NoSuchNode(parent));
            }
            // A node cannot become its own descendant's child: the subtree
            // would detach from the root and become unreachable while still
            // holding IDs.
            if parent == id || self.is_descendant_of(parent, id) {
                return Err(TreeError::WouldCycle { node: id, parent });
            }
        }
        self.detach(id)?;
        self.get_mut(id)?.parent = new_parent;
        let siblings = match new_parent {
            Some(parent) => &mut self.get_mut(parent)?.children,
            None => &mut self.roots,
        };
        let at = index.unwrap_or(siblings.len()).min(siblings.len());
        siblings.insert(at, id);
        Ok(())
    }

    /// Whether `candidate` lies anywhere beneath `ancestor`.
    pub fn is_descendant_of(&self, candidate: NodeId, ancestor: NodeId) -> bool {
        let mut current = self.get(candidate).and_then(|n| n.parent);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.get(id).and_then(|n| n.parent);
        }
        false
    }

    /// The rays that see `id`: its own, limited by every ancestor's.
    pub fn rays_of(&self, id: NodeId) -> Rays {
        let mut rays = Rays::default();
        let mut current = Some(id);
        while let Some(node) = current.and_then(|c| self.get(c)) {
            rays = rays.under(node.rays);
            current = node.parent;
        }
        rays
    }

    /// `id` followed by every descendant, parents before children.
    pub fn descendants_and_self(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if let Some(node) = self.get(current) {
                out.push(current);
                // Reversed, so the pop order matches the child order.
                stack.extend(node.children.iter().rev().copied());
            }
        }
        out
    }

    /// Every node, depth-first from the roots in document order.
    pub fn depth_first(&self) -> Vec<NodeId> {
        let mut out = Vec::with_capacity(self.nodes.len());
        for &root in &self.roots {
            out.extend(self.descendants_and_self(root));
        }
        out
    }

    /// Where a drag-and-drop should put `dragged`, as the `(parent, index)` a
    /// `Reparent` command needs.
    ///
    /// `inside` drops it into `onto` as a child; otherwise it lands
    /// immediately after `onto` among its siblings.
    ///
    /// The index accounts for the node being removed before it is re-inserted.
    /// Dragging a node *downward* past its own position in the same parent
    /// shifts everything after it up by one, so an index measured before the
    /// move lands one place too far — the classic off-by-one in every tree
    /// view, and invisible until someone drags a part one row down and it goes
    /// two.
    pub fn drop_position(
        &self,
        dragged: NodeId,
        onto: NodeId,
        inside: bool,
    ) -> Result<(Option<NodeId>, usize), TreeError> {
        self.try_get(dragged)?;
        let target = self.try_get(onto)?;
        if dragged == onto || self.is_descendant_of(onto, dragged) {
            return Err(TreeError::WouldCycle {
                node: dragged,
                parent: onto,
            });
        }

        if inside {
            // Appended, so a drop onto a collapsed group does not silently
            // reorder what is inside it.
            return Ok((Some(onto), self.get(onto).map_or(0, |n| n.children.len())));
        }

        let parent = target.parent;
        let mut index = self.sibling_index(onto).unwrap_or(0) + 1;
        let same_parent = self.get(dragged).and_then(|n| n.parent) == parent;
        if same_parent {
            if let Some(from) = self.sibling_index(dragged) {
                if from < index {
                    index -= 1;
                }
            }
        }
        Ok((parent, index))
    }

    /// The chain from a node up to its root, nearest ancestor first.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut current = self.get(id).and_then(|n| n.parent);
        while let Some(node) = current {
            out.push(node);
            current = self.get(node).and_then(|n| n.parent);
        }
        out
    }

    /// The accumulated transform from the document root down to `id`.
    pub fn world_transform(&self, id: NodeId) -> Transform {
        let mut chain = self.ancestors(id);
        chain.reverse();
        chain.push(id);
        let mut world = Transform::IDENTITY;
        for node in chain {
            if let Some(node) = self.get(node) {
                world = world * node.transform;
            }
        }
        world
    }

    /// Whether a node renders: it and every ancestor must be visible.
    ///
    /// Hiding a group has to hide its contents — a per-node flag consulted in
    /// isolation would leave children of a hidden group still rendering.
    pub fn is_effectively_visible(&self, id: NodeId) -> bool {
        if !self.get(id).map(|n| n.visible).unwrap_or(false) {
            return false;
        }
        self.ancestors(id)
            .into_iter()
            .all(|a| self.get(a).map(|n| n.visible).unwrap_or(false))
    }

    /// Whether a node may be edited: it and every ancestor must be unlocked.
    pub fn is_effectively_locked(&self, id: NodeId) -> bool {
        if self.get(id).map(|n| n.locked).unwrap_or(false) {
            return true;
        }
        self.ancestors(id)
            .into_iter()
            .any(|a| self.get(a).map(|n| n.locked).unwrap_or(false))
    }

    /// The path from the root, as a `/`-separated string of names. The basis
    /// of the persistent path key of §2.3.
    pub fn path_of(&self, id: NodeId) -> String {
        let mut chain = self.ancestors(id);
        chain.reverse();
        chain.push(id);
        chain
            .into_iter()
            .filter_map(|n| self.get(n).map(|n| n.name.as_str()))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Nodes whose name contains `needle`, case-insensitively. The Scene tab's
    /// live filter (`docs/04` §4.2).
    pub fn search(&self, needle: &str) -> Vec<NodeId> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle = needle.to_lowercase();
        self.depth_first()
            .into_iter()
            .filter(|&id| {
                self.get(id)
                    .map(|n| n.name.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
            .collect()
    }

    /// The matches of a search plus every ancestor, so the tree can keep the
    /// path to each match expanded while filtering (§4.2).
    pub fn search_with_ancestors(&self, needle: &str) -> BTreeSet<NodeId> {
        let mut out = BTreeSet::new();
        for id in self.search(needle) {
            out.insert(id);
            out.extend(self.ancestors(id));
        }
        out
    }

    /// World-space bounds of a subtree, given a way to look up mesh bounds.
    pub fn bounds(&self, id: NodeId, mesh_bounds: &dyn Fn(MeshKey) -> Option<Aabb>) -> Aabb {
        let mut bounds = Aabb::EMPTY;
        for node_id in self.descendants_and_self(id) {
            let Some(node) = self.get(node_id) else {
                continue;
            };
            // A light has no mesh but does have a place, and "Focus" on a
            // selected light has to frame something. Its emitter extent is
            // that something.
            // A cutaway's extent is its canonical shape's, which is the unit
            // cube for all four — a plane included, so "Focus" frames the
            // handle rather than an infinite sheet.
            if let NodeKind::Cutaway { .. } = &node.kind {
                bounds.extend(
                    &Aabb::from_points(-Vec3::ONE, Vec3::ONE).transformed(&self.world_transform(node_id)),
                );
                continue;
            }
            if let NodeKind::Light { settings } = &node.kind {
                let extent = settings.emitter_extent();
                bounds.extend(
                    &Aabb::from_points(-extent, extent)
                        .transformed(&self.world_transform(node_id)),
                );
                continue;
            }
            let Some(mesh) = node.mesh() else { continue };
            let Some(local) = mesh_bounds(mesh) else {
                continue;
            };
            bounds.extend(&local.transformed(&self.world_transform(node_id)));
        }
        bounds
    }

    /// Every node referencing a given mesh — the instances of it.
    pub fn instances_of(&self, mesh: MeshKey) -> Vec<NodeId> {
        self.depth_first()
            .into_iter()
            .filter(|&id| self.get(id).and_then(|n| n.mesh()) == Some(mesh))
            .collect()
    }

    /// Every node assigned a given material. Backs "select all parts sharing
    /// this material" (`docs/04` §4.1, material-level selection).
    pub fn users_of_material(&self, material: MaterialKey) -> Vec<NodeId> {
        self.depth_first()
            .into_iter()
            .filter(|&id| self.get(id).and_then(|n| n.material) == Some(material))
            .collect()
    }

    /// Check every structural invariant. Used by tests and after loading a
    /// document, where a malformed file must be rejected rather than
    /// half-loaded.
    pub fn validate(&self) -> Result<(), String> {
        for (&id, node) in &self.nodes {
            if node.id != id {
                return Err(format!("{id} is stored under the wrong key"));
            }
            if node.id.0 >= self.next_id {
                return Err(format!("{id} is at or beyond the ID allocator"));
            }
            match node.parent {
                Some(parent) => {
                    let Some(parent_node) = self.get(parent) else {
                        return Err(format!("{id} has missing parent {parent}"));
                    };
                    if !parent_node.children.contains(&id) {
                        return Err(format!("{id} is not among {parent}'s children"));
                    }
                }
                None => {
                    if !self.roots.contains(&id) {
                        return Err(format!("{id} has no parent but is not a root"));
                    }
                }
            }
            for &child in &node.children {
                let Some(child_node) = self.get(child) else {
                    return Err(format!("{id} lists missing child {child}"));
                };
                if child_node.parent != Some(id) {
                    return Err(format!("{child} does not point back at {id}"));
                }
            }
        }
        for &root in &self.roots {
            if !self.contains(root) {
                return Err(format!("root {root} does not exist"));
            }
        }
        // Every node must be reachable, or a subtree is leaking.
        let reachable = self.depth_first();
        if reachable.len() != self.nodes.len() {
            let mut unique = reachable.clone();
            unique.sort_unstable();
            unique.dedup();
            return Err(if unique.len() != reachable.len() {
                format!(
                    "{} nodes are listed twice in the hierarchy",
                    reachable.len() - unique.len()
                )
            } else {
                format!(
                    "{} nodes are unreachable from the roots",
                    self.nodes.len() - reachable.len()
                )
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Vec3};

    /// A small assembly: two groups, four parts, two of them sharing a mesh.
    fn assembly() -> (SceneTree, Vec<NodeId>) {
        let mut tree = SceneTree::new();
        let root = tree.insert(None, "Assembly", NodeKind::Group).unwrap();
        let housing = tree
            .insert(Some(root), "Housing", NodeKind::Part { mesh: MeshKey(1) })
            .unwrap();
        let screws = tree.insert(Some(root), "Screws", NodeKind::Group).unwrap();
        let a = tree
            .insert(Some(screws), "Screw A", NodeKind::Part { mesh: MeshKey(2) })
            .unwrap();
        let b = tree
            .insert(Some(screws), "Screw B", NodeKind::Part { mesh: MeshKey(2) })
            .unwrap();
        (tree, vec![root, housing, screws, a, b])
    }

    #[test]
    fn a_fresh_assembly_is_well_formed() {
        let (tree, ids) = assembly();
        tree.validate().unwrap();
        assert_eq!(tree.len(), 5);
        assert_eq!(tree.roots(), &[ids[0]]);
        assert_eq!(tree.depth_first(), ids);
    }

    /// IDs must never be reused. A recycled ID would let a stale material
    /// assignment silently re-attach to an unrelated part.
    #[test]
    fn ids_are_never_reused_after_deletion() {
        let (mut tree, ids) = assembly();
        let before = tree.next_id();
        tree.remove_subtree(ids[2]).unwrap();
        let fresh = tree.insert(None, "New", NodeKind::Group).unwrap();
        assert!(fresh.0 >= before, "{fresh} reuses a retired identifier");
        assert!(!ids.contains(&fresh));
        tree.validate().unwrap();
    }

    #[test]
    fn removing_a_subtree_takes_its_descendants() {
        let (mut tree, ids) = assembly();
        let removed = tree.remove_subtree(ids[2]).unwrap();
        assert_eq!(removed.len(), 3, "group plus its two screws");
        assert_eq!(tree.len(), 2);
        assert!(!tree.contains(ids[3]) && !tree.contains(ids[4]));
        tree.validate().unwrap();
    }

    /// Undo of a deletion must restore the same IDs at the same positions, or
    /// every reference into the subtree dangles.
    #[test]
    fn a_removed_subtree_restores_exactly() {
        let (mut tree, ids) = assembly();
        let before = tree.clone();
        let index = tree.sibling_index(ids[2]).unwrap();
        let removed = tree.remove_subtree(ids[2]).unwrap();

        // Shallowest first, which is the reverse of removal order.
        for (offset, node) in removed.into_iter().rev().enumerate() {
            let at = if offset == 0 { index } else { usize::MAX };
            tree.reinsert(node, at).unwrap();
        }
        tree.validate().unwrap();
        assert_eq!(
            tree, before,
            "restoring the subtree did not reproduce the tree"
        );
    }

    #[test]
    fn reparenting_moves_a_node_and_its_children() {
        let (mut tree, ids) = assembly();
        tree.reparent(ids[3], Some(ids[1]), None).unwrap();
        assert_eq!(tree.get(ids[3]).unwrap().parent, Some(ids[1]));
        assert!(tree.get(ids[1]).unwrap().children.contains(&ids[3]));
        assert!(!tree.get(ids[2]).unwrap().children.contains(&ids[3]));
        tree.validate().unwrap();
    }

    /// Dropping a group onto one of its own children would detach the whole
    /// subtree from the root — reachable only through itself — and leak it.
    #[test]
    fn reparenting_into_a_descendant_is_refused() {
        let (mut tree, ids) = assembly();
        let error = tree.reparent(ids[2], Some(ids[3]), None).unwrap_err();
        assert_eq!(
            error,
            TreeError::WouldCycle {
                node: ids[2],
                parent: ids[3]
            }
        );
        assert_eq!(
            tree.reparent(ids[2], Some(ids[2]), None).unwrap_err(),
            TreeError::WouldCycle {
                node: ids[2],
                parent: ids[2]
            }
        );
        tree.validate().unwrap();
    }

    #[test]
    fn reparenting_honours_the_requested_index() {
        let (mut tree, ids) = assembly();
        tree.reparent(ids[1], Some(ids[2]), Some(0)).unwrap();
        assert_eq!(tree.get(ids[2]).unwrap().children[0], ids[1]);
        tree.reparent(ids[1], Some(ids[2]), Some(99)).unwrap();
        assert_eq!(*tree.get(ids[2]).unwrap().children.last().unwrap(), ids[1]);
        tree.validate().unwrap();
    }

    /// The off-by-one every tree view has: a node dragged downward is removed
    /// before it is re-inserted, so an index measured beforehand overshoots.
    #[test]
    fn a_drop_lands_where_it_was_dropped_in_both_directions() {
        let mut tree = SceneTree::new();
        let root = tree.insert(None, "Root", NodeKind::Group).unwrap();
        let ids: Vec<NodeId> = ["A", "B", "C", "D"]
            .iter()
            .map(|name| tree.insert(Some(root), *name, NodeKind::Group).unwrap())
            .collect();

        // Drag A down onto C: expected A directly after C.
        let (parent, index) = tree.drop_position(ids[0], ids[2], false).unwrap();
        assert_eq!(parent, Some(root));
        tree.reparent(ids[0], parent, Some(index)).unwrap();
        assert_eq!(
            tree.get(root).unwrap().children,
            vec![ids[1], ids[2], ids[0], ids[3]],
            "the node overshot its drop position"
        );

        // And back up: drag A onto B, expecting A directly after B.
        let (parent, index) = tree.drop_position(ids[0], ids[1], false).unwrap();
        tree.reparent(ids[0], parent, Some(index)).unwrap();
        assert_eq!(
            tree.get(root).unwrap().children,
            vec![ids[1], ids[0], ids[2], ids[3]]
        );
    }

    #[test]
    fn a_drop_into_a_group_appends_to_it() {
        let mut tree = SceneTree::new();
        let a = tree.insert(None, "A", NodeKind::Group).unwrap();
        let b = tree.insert(None, "B", NodeKind::Group).unwrap();
        let inner = tree.insert(Some(b), "Inner", NodeKind::Group).unwrap();

        let (parent, index) = tree.drop_position(a, b, true).unwrap();
        assert_eq!(parent, Some(b));
        tree.reparent(a, parent, Some(index)).unwrap();
        assert_eq!(
            tree.get(b).unwrap().children,
            vec![inner, a],
            "dropping into a group reordered what was already in it"
        );
    }

    /// Dropping a node into its own subtree would detach it from the root, so
    /// the position is refused rather than computed.
    #[test]
    fn a_drop_into_its_own_subtree_is_refused() {
        let mut tree = SceneTree::new();
        let outer = tree.insert(None, "Outer", NodeKind::Group).unwrap();
        let inner = tree.insert(Some(outer), "Inner", NodeKind::Group).unwrap();

        assert!(tree.drop_position(outer, inner, true).is_err());
        assert!(tree.drop_position(outer, inner, false).is_err());
        assert!(tree.drop_position(outer, outer, false).is_err());
    }

    /// Between two different parents there is nothing to compensate for.
    #[test]
    fn a_drop_across_parents_uses_the_plain_index() {
        let mut tree = SceneTree::new();
        let left = tree.insert(None, "Left", NodeKind::Group).unwrap();
        let right = tree.insert(None, "Right", NodeKind::Group).unwrap();
        let moving = tree.insert(Some(left), "Moving", NodeKind::Group).unwrap();
        let first = tree.insert(Some(right), "First", NodeKind::Group).unwrap();
        let second = tree.insert(Some(right), "Second", NodeKind::Group).unwrap();

        let (parent, index) = tree.drop_position(moving, first, false).unwrap();
        tree.reparent(moving, parent, Some(index)).unwrap();
        assert_eq!(
            tree.get(right).unwrap().children,
            vec![first, moving, second]
        );
    }

    #[test]
    fn world_transforms_accumulate_down_the_chain() {
        let (mut tree, ids) = assembly();
        tree.get_mut(ids[0]).unwrap().transform = Transform::translation(vec3(10.0, 0.0, 0.0));
        tree.get_mut(ids[2]).unwrap().transform = Transform::translation(vec3(0.0, 5.0, 0.0));
        tree.get_mut(ids[3]).unwrap().transform = Transform::translation(vec3(0.0, 0.0, 2.0));

        let world = tree.world_transform(ids[3]).point(Vec3::ZERO);
        assert!(
            (world - vec3(10.0, 5.0, 2.0)).length() < 1e-5,
            "world transform is {world:?}"
        );
    }

    /// Hiding a group must hide everything inside it. A per-node flag read in
    /// isolation would leave the children rendering.
    #[test]
    fn visibility_and_locking_inherit_from_ancestors() {
        let (mut tree, ids) = assembly();
        assert!(tree.is_effectively_visible(ids[3]));
        assert!(!tree.is_effectively_locked(ids[3]));

        tree.get_mut(ids[2]).unwrap().visible = false;
        assert!(!tree.is_effectively_visible(ids[3]));
        assert!(
            tree.get(ids[3]).unwrap().visible,
            "the child's own flag must be untouched, so unhiding restores it"
        );

        tree.get_mut(ids[0]).unwrap().locked = true;
        assert!(tree.is_effectively_locked(ids[3]));
    }

    #[test]
    fn paths_read_from_the_root_down() {
        let (tree, ids) = assembly();
        assert_eq!(tree.path_of(ids[3]), "Assembly/Screws/Screw A");
        assert_eq!(tree.path_of(ids[0]), "Assembly");
    }

    #[test]
    fn search_is_case_insensitive_and_keeps_ancestors() {
        let (tree, ids) = assembly();
        // "Screws" matches too — a substring search over names, as the
        // Scene tab's live filter is.
        assert_eq!(tree.search("screw"), vec![ids[2], ids[3], ids[4]]);
        assert_eq!(tree.search("SCREW A"), vec![ids[3]]);
        assert!(tree.search("nothing").is_empty());
        assert!(tree.search("").is_empty());

        // Ancestors come along so the tree can stay expanded to each match.
        let visible = tree.search_with_ancestors("Screw A");
        assert!(
            visible.contains(&ids[0]) && visible.contains(&ids[2]) && visible.contains(&ids[3])
        );
        assert!(!visible.contains(&ids[1]));
    }

    /// Instancing is a first-class relationship (§2.2): two parts sharing a
    /// mesh must be discoverable as instances of it.
    #[test]
    fn instances_and_material_users_are_discoverable() {
        let (mut tree, ids) = assembly();
        assert_eq!(tree.instances_of(MeshKey(2)), vec![ids[3], ids[4]]);
        assert_eq!(tree.instances_of(MeshKey(1)), vec![ids[1]]);
        assert!(tree.instances_of(MeshKey(99)).is_empty());

        tree.get_mut(ids[1]).unwrap().material = Some(MaterialKey(7));
        tree.get_mut(ids[3]).unwrap().material = Some(MaterialKey(7));
        assert_eq!(tree.users_of_material(MaterialKey(7)), vec![ids[1], ids[3]]);
    }

    #[test]
    fn bounds_cover_the_subtree_in_world_space() {
        let (mut tree, ids) = assembly();
        tree.get_mut(ids[3]).unwrap().transform = Transform::translation(vec3(5.0, 0.0, 0.0));
        let unit = Aabb::from_points(vec3(-1.0, -1.0, -1.0), Vec3::ONE);
        let bounds = tree.bounds(ids[2], &|_| Some(unit));
        assert!((bounds.min.x + 1.0).abs() < 1e-5, "{bounds:?}");
        assert!((bounds.max.x - 6.0).abs() < 1e-5, "{bounds:?}");
    }

    #[test]
    fn operations_on_missing_nodes_are_refused() {
        let (mut tree, _) = assembly();
        let ghost = NodeId(999);
        assert_eq!(
            tree.try_get(ghost).unwrap_err(),
            TreeError::NoSuchNode(ghost)
        );
        assert_eq!(
            tree.remove_subtree(ghost).unwrap_err(),
            TreeError::NoSuchNode(ghost)
        );
        assert_eq!(
            tree.reparent(ghost, None, None).unwrap_err(),
            TreeError::NoSuchNode(ghost)
        );
        assert_eq!(
            tree.insert(Some(ghost), "x", NodeKind::Group).unwrap_err(),
            TreeError::NoSuchNode(ghost)
        );
        tree.validate().unwrap();
    }

    /// Iteration order must be a property of the document, not of the
    /// allocator: M1's exit criterion is a byte-identical round trip.
    #[test]
    fn iteration_order_is_deterministic() {
        let build = || {
            let mut tree = SceneTree::new();
            let root = tree.insert(None, "R", NodeKind::Group).unwrap();
            for i in 0..64 {
                tree.insert(
                    Some(root),
                    format!("part{i}"),
                    NodeKind::Part { mesh: MeshKey(i) },
                )
                .unwrap();
            }
            tree
        };
        let a = build();
        let b = build();
        assert_eq!(a.depth_first(), b.depth_first());
        assert_eq!(
            a.iter().map(|n| n.id).collect::<Vec<_>>(),
            b.iter().map(|n| n.id).collect::<Vec<_>>()
        );
        assert_eq!(a, b);
    }

    #[test]
    fn validation_catches_a_corrupted_tree() {
        let (mut tree, ids) = assembly();
        tree.validate().unwrap();
        // Sever a back-pointer, as a bad loader might.
        tree.get_mut(ids[3]).unwrap().parent = None;
        assert!(tree.validate().is_err());
    }
}
