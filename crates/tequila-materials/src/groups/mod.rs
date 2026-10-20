//! Node groups: a node made of other nodes, as Blender's node groups are.
//!
//! A group is one node in the graph, with its own inputs and outputs, and a
//! whole graph inside it built of the nodes every engine already has. Before
//! compiling, each group is replaced by what is inside it
//! ([`crate::graph::MaterialGraph::without_groups`]), so Tequila's renderer,
//! the GPU preview and Cycles all shade it, node for node, as they shade
//! anything else, and none of them knows groups exist.
//!
//! The groups are built in: each is a recipe in this module, not something a
//! file carries. A file names the group and sets its inputs, so a material
//! made with one opens in a later Tequila with the group as that version
//! builds it.

mod fdm_layer_lines;

use crate::graph::{Link, MaterialGraph, Node, NodeId, NodeKind, Socket, SocketType, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The groups Tequila has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeGroup {
    /// The surface of a print from a filament printer: its layer lines, the
    /// seam, the raster lines on the top and bottom skins. From the "3DP
    /// Layer Lines" group of the PLA 3D Print materials.
    FdmLayerLines,
}

/// One of a group's inputs. Every one is a number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupInput {
    pub name: &'static str,
    pub default: f32,
    /// The slider's range.
    pub min: f32,
    pub max: f32,
    pub tip: &'static str,
}

/// One of a group's outputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupOutput {
    pub name: &'static str,
    pub ty: SocketType,
    pub tip: &'static str,
}

impl NodeGroup {
    pub const ALL: &'static [NodeGroup] = &[NodeGroup::FdmLayerLines];

    /// Its stable name, in files and scripts.
    pub fn key(self) -> &'static str {
        match self {
            NodeGroup::FdmLayerLines => "fdm_layer_lines",
        }
    }

    pub fn from_key(key: &str) -> Option<NodeGroup> {
        NodeGroup::ALL.iter().copied().find(|g| g.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            NodeGroup::FdmLayerLines => "FDM Layer Lines",
        }
    }

    /// What it is for, in the Add menu and the sidebar.
    pub fn description(self) -> &'static str {
        match self {
            NodeGroup::FdmLayerLines => {
                "The surface of an FDM (filament) 3D print: layer lines up the walls, the seam, raster lines on the top \
                 and bottom. In millimetres, with the part's Y as the build direction. Wire Bump to Bump, and Tangent \
                 to Tangent for the sheen along the layers."
            }
        }
    }

    /// What the Add menu's search also finds it by, comma separated.
    pub fn keywords(self) -> &'static str {
        match self {
            NodeGroup::FdmLayerLines => "3D print, 3D printed, FFF, PLA, PETG, layer lines, filament, extrusion",
        }
    }

    pub fn inputs(self) -> &'static [GroupInput] {
        match self {
            NodeGroup::FdmLayerLines => fdm_layer_lines::INPUTS,
        }
    }

    pub fn outputs(self) -> &'static [GroupOutput] {
        match self {
            NodeGroup::FdmLayerLines => fdm_layer_lines::OUTPUTS,
        }
    }

    /// The nodes inside, built once.
    pub fn expansion(self) -> &'static Expansion {
        static FDM: OnceLock<Expansion> = OnceLock::new();
        match self {
            NodeGroup::FdmLayerLines => FDM.get_or_init(fdm_layer_lines::build),
        }
    }
}

/// What is inside a group: nodes, the links between them, and where the
/// group's own inputs and outputs meet them.
#[derive(Clone, Debug, Default)]
pub struct Expansion {
    nodes: Vec<Node>,
    /// From node and output, to node and input.
    links: Vec<(u32, u8, u32, u8)>,
    /// A group input, read by a node's input.
    reads: Vec<(usize, u32, u8)>,
    /// A group output, written by a node's output.
    writes: Vec<Option<(u32, u8)>>,
}

impl Expansion {
    pub(crate) fn new(_inputs: usize, outputs: usize) -> Expansion {
        Expansion { writes: vec![None; outputs], ..Expansion::default() }
    }

    /// Add a node with literals on some of its inputs.
    pub(crate) fn node(&mut self, kind: NodeKind, literals: &[(u8, Value)]) -> u32 {
        let mut node = Node::new(kind);
        for (index, value) in literals {
            node.literals.insert(Socket::Input(*index), *value);
        }
        self.nodes.push(node);
        self.nodes.len() as u32 - 1
    }

    pub(crate) fn link(&mut self, from: u32, output: u8, to: u32, input: u8) {
        self.links.push((from, output, to, input));
    }

    /// A literal on an input added since the node was.
    pub(crate) fn set(&mut self, node: u32, input: u8, value: Value) {
        self.nodes[node as usize].literals.insert(Socket::Input(input), value);
    }

    /// Group input `group_input` feeds `node`'s input `input`.
    pub(crate) fn read(&mut self, group_input: usize, node: u32, input: u8) {
        self.reads.push((group_input, node, input));
    }

    /// Group output `group_output` is `node`'s output `output`.
    pub(crate) fn write(&mut self, group_output: usize, node: u32, output: u8) {
        self.writes[group_output] = Some((node, output));
    }

    /// How many nodes it is.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl MaterialGraph {
    /// The same graph with every group replaced by the nodes inside it: a
    /// wire into a group's input goes on to every node that reads it (or,
    /// unwired, the group's literal or default is set on each of them), and
    /// a wire out of a group's output comes from the node inside that
    /// writes it.
    pub fn without_groups(&self) -> MaterialGraph {
        let groups: Vec<(NodeId, NodeGroup)> = self
            .nodes()
            .filter_map(|(id, node)| match node.kind {
                NodeKind::Group(group) => Some((id, group)),
                _ => None,
            })
            .collect();
        let mut out = self.clone();
        for (id, group) in groups {
            let outer = self.node(id).expect("listed above").clone();
            let inside = group.expansion();
            let ids: Vec<NodeId> = inside
                .nodes
                .iter()
                .map(|node| out.add_node(Node { position: outer.position, ..node.clone() }))
                .collect();
            for &(from, output, to, input) in &inside.links {
                out.connect_output(ids[from as usize], output, ids[to as usize], Socket::Input(input));
            }
            // What feeds each of the group's inputs.
            let wired: BTreeMap<u8, Link> = self
                .links()
                .iter()
                .filter(|l| l.to == id)
                .filter_map(|l| match l.to_socket {
                    Socket::Input(i) => Some((i, *l)),
                    Socket::Param(_) => None,
                })
                .collect();
            for &(group_input, node, input) in &inside.reads {
                let index = group_input as u8;
                let target = ids[node as usize];
                match wired.get(&index) {
                    Some(link) => out.connect_output(link.from, link.from_socket, target, Socket::Input(input)),
                    None => {
                        let value = outer
                            .literals
                            .get(&Socket::Input(index))
                            .copied()
                            .unwrap_or(Value::Float(group.inputs()[group_input].default));
                        if let Some(node) = out.node_mut(target) {
                            node.literals.insert(Socket::Input(input), value);
                        }
                    }
                }
            }
            // Wires out of the group, re-made from inside it.
            let leaving: Vec<Link> = out.links().iter().filter(|l| l.from == id).copied().collect();
            out.remove_node(id);
            for link in leaving {
                if let Some(Some((node, output))) = inside.writes.get(link.from_socket as usize) {
                    out.connect_output(ids[*node as usize], *output, link.to, link.to_socket);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Instruction;
    use crate::material_type::Param;
    use crate::openpbr::Material;

    fn layer_lines() -> (MaterialGraph, NodeId) {
        let mut graph = MaterialGraph::from_material(Material::default());
        let group = graph.add_node(Node::new(NodeKind::Group(NodeGroup::FdmLayerLines)));
        let output = graph.output();
        graph.connect_output(group, 0, output, Socket::Param(Param::Bump));
        (graph, group)
    }

    #[test]
    fn every_output_is_written_and_every_input_read() {
        for &group in NodeGroup::ALL {
            let inside = group.expansion();
            assert_eq!(inside.writes.len(), group.outputs().len());
            assert!(inside.writes.iter().all(Option::is_some), "{group:?} leaves an output unwritten");
            for input in 0..group.inputs().len() {
                assert!(inside.reads.iter().any(|r| r.0 == input), "{group:?} never reads {}", group.inputs()[input].name);
            }
        }
    }

    #[test]
    fn a_group_compiles_to_the_nodes_inside_it() {
        let (graph, _) = layer_lines();
        let program = graph.compile().expect("compiles");
        let applied = program.instructions.iter().filter(|i| matches!(i, Instruction::Apply { .. })).count();
        assert!(applied > 150, "only {applied} nodes evaluated");
        assert!(program.driven.iter().any(|(p, _)| *p == Param::Bump));
    }

    #[test]
    fn a_group_input_takes_its_literal_or_a_wire() {
        let (mut graph, group) = layer_lines();
        // Layer height, as a literal: the nodes that read it get it.
        graph.node_mut(group).unwrap().literals.insert(Socket::Input(0), Value::Float(0.12));
        let expanded = graph.without_groups();
        assert!(expanded.nodes().all(|(_, n)| !matches!(n.kind, NodeKind::Group(_))));
        let set = expanded
            .nodes()
            .filter(|(_, n)| n.literals.values().any(|v| *v == Value::Float(0.12)))
            .count();
        assert!(set >= 1);
        // Wired: the wire goes on to them instead.
        let constant = graph.add_node(Node::new(NodeKind::Constant(Value::Float(0.3))));
        graph.connect(constant, group, Socket::Input(0));
        let expanded = graph.without_groups();
        let fed = expanded.links().iter().filter(|l| l.from == constant).count();
        let readers = NodeGroup::FdmLayerLines.expansion().reads.iter().filter(|r| r.0 == 0).count();
        assert_eq!(fed, readers);
    }
}
