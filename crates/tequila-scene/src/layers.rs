//! CAD layers: the groups a CAD file put its parts in, shown and hidden
//! together (`docs/03` §6.5).
//!
//! A layer is not a node of its own. Each node keeps the names of the
//! layers the file put it on ([`SceneNode::layers`]), and a layer is every
//! node that names it. Whether a layer is shown is whether those nodes are,
//! so hiding one hides them: one undo step, through the visibility the
//! renderers and the Scene tab's eyes already follow, and nothing to fall
//! out of step when a part on it is shown again by hand.

use crate::document::SceneDocument;
use crate::tree::{NodeId, SceneNode};

/// One layer, as the document has it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub name: String,
    /// The nodes on it, in tree order.
    pub nodes: Vec<NodeId>,
    /// How many of them are shown.
    pub shown: usize,
}

/// Whether a layer's nodes are shown: all of them, none, or some, which is
/// what a part shown or hidden by hand leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    All,
    None,
    Some,
}

impl Shown {
    pub fn label(self) -> &'static str {
        match self {
            Shown::All => "all",
            Shown::None => "none",
            Shown::Some => "some",
        }
    }
}

impl Layer {
    pub fn shown(&self) -> Shown {
        match self.shown {
            0 => Shown::None,
            n if n == self.nodes.len() => Shown::All,
            _ => Shown::Some,
        }
    }
}

/// Whether `node` is on `layer`. Names match exactly, as the file wrote
/// them: two layers a CAD package kept apart stay apart.
pub fn is_on(node: &SceneNode, layer: &str) -> bool {
    node.layers.iter().any(|l| l == layer)
}

/// Every layer some node is on, in the order the tree first meets them,
/// which for an imported model is the file's.
pub fn layers(document: &SceneDocument) -> Vec<Layer> {
    let mut out: Vec<Layer> = Vec::new();
    for id in document.tree.depth_first() {
        let Some(node) = document.tree.get(id) else { continue };
        for name in &node.layers {
            let at = match out.iter().position(|l| &l.name == name) {
                Some(at) => at,
                None => {
                    out.push(Layer { name: name.clone(), nodes: Vec::new(), shown: 0 });
                    out.len() - 1
                }
            };
            // A node that names a layer twice is on it once.
            if out[at].nodes.last() != Some(&id) {
                out[at].nodes.push(id);
                out[at].shown += node.visible as usize;
            }
        }
    }
    out
}

/// The nodes on `layer`, in tree order.
pub fn members(document: &SceneDocument, layer: &str) -> Vec<NodeId> {
    document
        .tree
        .depth_first()
        .into_iter()
        .filter(|&id| document.tree.get(id).is_some_and(|node| is_on(node, layer)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, ImportPayload};
    use crate::geometry::Mesh;
    use crate::tree::NodeKind;
    use crate::CommandBus;
    use tequila_core::math::{vec3, Vec3};

    /// Model
    ///   Housing    Outer
    ///   Rib        Outer, Construction
    ///   Datum      Construction
    ///   Screw      (none)
    fn model() -> (SceneDocument, CommandBus, Vec<NodeId>) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mesh = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        let (meshes, _, ids) = crate::command::reserve_keys(&document, 1, 0, 5);
        let mut root = SceneNode::new(ids[0], "Model", NodeKind::Group);
        root.children = ids[1..].to_vec();
        let mut nodes = vec![root];
        for (index, name, layers) in [
            (1, "Housing", vec!["Outer"]),
            (2, "Rib", vec!["Outer", "Construction"]),
            (3, "Datum", vec!["Construction"]),
            (4, "Screw", vec![]),
        ] {
            let mut node = SceneNode::new(ids[index], name, NodeKind::Part { mesh: meshes[0] });
            node.parent = Some(ids[0]);
            node.layers = layers.into_iter().map(String::from).collect();
            nodes.push(node);
        }
        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes,
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        (document, bus, ids)
    }

    #[test]
    fn layers_are_listed_in_the_order_the_tree_meets_them() {
        let (document, _, ids) = model();
        let layers = layers(&document);
        let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Outer", "Construction"]);
        assert_eq!(layers[0].nodes, [ids[1], ids[2]]);
        assert_eq!(layers[1].nodes, [ids[2], ids[3]]);
        assert!(layers.iter().all(|l| l.shown() == Shown::All));
        assert_eq!(members(&document, "Construction"), [ids[2], ids[3]]);
        assert!(members(&document, "construction").is_empty(), "names match exactly");
    }

    /// Hiding a layer hides every part on it as one step; a part on two
    /// layers goes with either, and the other layer then reads as partly
    /// shown. Undo brings each part back as it was.
    #[test]
    fn a_layer_hides_and_shows_its_parts_as_one_step() {
        let (mut document, mut bus, ids) = model();
        let visible = |d: &SceneDocument, i: usize| d.tree.get(ids[i]).unwrap().visible;
        bus.apply(&mut document, Command::SetLayerVisibility { layer: "Construction".into(), visible: false }).unwrap();
        assert!(!visible(&document, 2) && !visible(&document, 3));
        assert!(visible(&document, 1) && visible(&document, 4));
        let outer = layers(&document).into_iter().find(|l| l.name == "Outer").unwrap();
        assert_eq!(outer.shown(), Shown::Some);

        bus.apply(&mut document, Command::SetLayerVisibility { layer: "Outer".into(), visible: true }).unwrap();
        assert!(visible(&document, 2), "the rib is on Outer as well");
        assert!(!visible(&document, 3));

        bus.undo(&mut document).unwrap();
        assert!(!visible(&document, 2));
        bus.undo(&mut document).unwrap();
        assert!((1..5).all(|i| visible(&document, i)));
    }

    #[test]
    fn a_layer_nothing_is_on_is_refused() {
        let (mut document, mut bus, _) = model();
        let error = bus.apply(&mut document, Command::SetLayerVisibility { layer: "Hatching".into(), visible: false }).unwrap_err();
        assert!(error.to_string().contains("Hatching"), "{error}");
    }
}
