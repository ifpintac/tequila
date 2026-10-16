//! Laying a graph out tidily: columns flowing into the surface.

use super::view::{connected, layout, NodeLayout};
use tequila_core::math::Vec2 as GraphVec2;
use tequila_materials::graph::{MaterialGraph, NodeId};
use tequila_materials::material_type::Param;
use std::collections::HashMap;

/// A group's title bar, which is where it is picked up.
pub const FRAME_TITLE: f32 = 24.0;
const COLUMN_GAP: f32 = 64.0;
const ROW_GAP: f32 = 20.0;

pub fn layouts(graph: &MaterialGraph, exposed: &[Param], previews: bool) -> HashMap<NodeId, NodeLayout> {
    let wired = connected(graph);
    graph
        .nodes()
        .map(|(id, node)| (id, layout(node, id, &wired, exposed, previews, &|_| String::new())))
        .collect()
}

/// Whether any two nodes sit on top of each other — as every node of a graph
/// built by an importer does, all at the origin, since nothing placed them.
pub fn overlapping(graph: &MaterialGraph, exposed: &[Param]) -> bool {
    let rects: Vec<egui::Rect> = layouts(graph, exposed, true).into_values().map(|l| l.rect).collect();
    rects
        .iter()
        .enumerate()
        .any(|(i, a)| rects[i + 1..].iter().any(|b| a.shrink(4.0).intersects(*b)))
}

/// Everything a view should show: the nodes and the groups.
pub fn content_bounds(graph: &MaterialGraph, exposed: &[Param], previews: bool) -> Option<egui::Rect> {
    layouts(graph, exposed, previews)
        .into_values()
        .map(|l| l.rect)
        .chain(graph.frames.iter().map(|f| egui::Rect::from_min_max(egui::pos2(f.min.x, f.min.y), egui::pos2(f.max.x, f.max.y))))
        .reduce(|a, b| a.union(b))
}

/// Lay the graph out in columns flowing into the surface on the right: each
/// node one column left of the furthest node it feeds, and within a column
/// in the order of the sockets it feeds, so wires mostly run level. Groups
/// are fitted back round the nodes they held.
pub fn arrange(graph: &mut MaterialGraph, exposed: &[Param]) {
    let members: Vec<Vec<NodeId>> = graph
        .frames
        .iter()
        .map(|frame| graph.nodes().filter(|(_, n)| frame.contains(n.position)).map(|(id, _)| id).collect())
        .collect();
    arrange_nodes(graph, exposed);
    let placed = layouts(graph, exposed, true);
    for (index, held) in members.into_iter().enumerate() {
        let bounds = held.iter().filter_map(|id| placed.get(id).map(|l| l.rect)).reduce(|a, b| a.union(b));
        if let Some(bounds) = bounds {
            let around = bounds.expand(24.0);
            let frame = &mut graph.frames[index];
            frame.min = GraphVec2 { x: around.min.x, y: around.min.y - FRAME_TITLE };
            frame.max = GraphVec2 { x: around.max.x, y: around.max.y };
        }
    }
}

fn arrange_nodes(graph: &mut MaterialGraph, exposed: &[Param]) {
    let output = graph.output();
    let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id).collect();
    let mut column: HashMap<NodeId, usize> = HashMap::from([(output, 0)]);
    // Longest path to the surface. The graph compiles, so it has no loops
    // and this settles within one pass per node.
    for _ in 0..ids.len() {
        let mut changed = false;
        for link in graph.links() {
            if let Some(&to) = column.get(&link.to) {
                let at = column.entry(link.from).or_insert(0);
                if *at < to + 1 {
                    *at = to + 1;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let sizes = layouts(graph, exposed, true);
    let loose = column.values().max().copied().unwrap_or(0) + 1;
    let column_of = |id: &NodeId| column.get(id).copied().unwrap_or(loose);
    let columns = ids.iter().map(column_of).max().unwrap_or(0);
    // Each column as wide as its widest node.
    let widths: Vec<f32> = (0..=columns)
        .map(|c| ids.iter().filter(|id| column_of(id) == c).filter_map(|id| sizes.get(id).map(|l| l.rect.width())).fold(0.0, f32::max))
        .collect();
    let mut placed: HashMap<NodeId, f32> = HashMap::new();
    let mut right = 0.0;
    for c in 0..=columns {
        let mut members: Vec<(f32, NodeId)> = ids
            .iter()
            .filter(|id| column_of(id) == c)
            .map(|&id| {
                // Beside the first socket it feeds.
                let key = graph
                    .links()
                    .iter()
                    .filter(|l| l.from == id)
                    .filter_map(|l| {
                        let y = placed.get(&l.to)?;
                        let target = sizes.get(&l.to)?;
                        let at = target.input_at(l.to_socket)?;
                        Some(y + (at.y - target.rect.top()))
                    })
                    .fold(f32::INFINITY, f32::min);
                (key, id)
            })
            .collect();
        members.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1 .0.cmp(&b.1 .0)));
        let width = widths[c];
        let mut y: f32 = 0.0;
        for (key, id) in members {
            let Some(size) = sizes.get(&id).map(|l| l.rect.size()) else { continue };
            // Level with what it feeds where there is room.
            if key.is_finite() {
                y = y.max(key - 30.0);
            }
            if let Some(node) = graph.node_mut(id) {
                node.position = GraphVec2 { x: right - width, y };
            }
            placed.insert(id, y);
            y += size.y + ROW_GAP;
        }
        right -= width + COLUMN_GAP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::spectrum::Spectrum;
    use tequila_materials::graph::{Node, NodeKind, Socket, Value};

    /// An imported graph has every node at the origin; arranged, nothing
    /// overlaps and every wire runs right to left, into the surface.
    #[test]
    fn arranging_separates_stacked_nodes_and_flows_into_the_surface() {
        let mut graph = MaterialGraph::from_material(Default::default());
        let surface = graph.output();
        let a = graph.add_node(Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.5)))));
        let b = graph.add_node(Node::new(NodeKind::Invert));
        let c = graph.add_node(Node::new(NodeKind::Constant(Value::Float(0.3))));
        graph.connect(a, b, Socket::Input(0));
        graph.connect(b, surface, Socket::Param(Param::BaseColor));
        graph.connect(c, surface, Socket::Param(Param::SpecularRoughness));
        let exposed = tequila_materials::material_type::MaterialType::ALL[0].exposed();
        assert!(overlapping(&graph, &exposed), "all at the origin to begin with");
        arrange(&mut graph, &exposed);
        assert!(!overlapping(&graph, &exposed));
        for link in graph.links() {
            let (from, to) = (graph.node(link.from).unwrap().position, graph.node(link.to).unwrap().position);
            assert!(from.x < to.x, "a wire runs into a node to its right");
        }
    }

    /// Tidying moves the nodes; a group moves with the nodes it held and
    /// still surrounds them after, rather than being left behind.
    #[test]
    fn tidying_keeps_a_group_round_its_nodes() {
        use tequila_materials::graph::Frame;
        let mut graph = MaterialGraph::from_material(tequila_render_core::material::Material::plastic(Spectrum::splat(0.5), 0.3));
        let colour = graph.add_node(Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.4)))));
        let output = graph.output();
        graph.connect(colour, output, Socket::Param(Param::BaseColor));
        graph.node_mut(colour).unwrap().position = GraphVec2 { x: 900.0, y: 900.0 };
        graph.frames.push(Frame {
            title: "Source".into(),
            min: GraphVec2 { x: 880.0, y: 860.0 },
            max: GraphVec2 { x: 1100.0, y: 1050.0 },
        });
        arrange(&mut graph, &[Param::BaseColor]);
        let at = graph.node(colour).unwrap().position;
        assert!(graph.frames[0].contains(at), "the group was left behind at {:?} while its node went to {at:?}", graph.frames[0]);
        assert!(!graph.frames[0].contains(graph.node(output).unwrap().position), "the group grew round a node it never held");
    }
}
