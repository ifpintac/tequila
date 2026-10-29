//! Reading and writing material graphs.
//!
//! The parameter block stays exactly where it was, written flat at the top of
//! the material's JSON. A `graph` object appears beside it only when there is
//! more than the surface node to record.
//!
//! That split is deliberate. A material with no textures is, by `docs/02` §4.2,
//! already a graph — one surface node holding the block as its literals — so
//! writing the block flat *is* writing that graph, and the file for such a
//! material is byte-for-byte what it was before graphs existed. Files that never
//! needed the machinery never pay for it, in bytes or in readability, and every
//! scene saved so far still opens.
//!
//! The other half of that arrangement: a graph read back must reconstruct the
//! surface node's literals from the flat block rather than from anything inside
//! the `graph` object. Writing the parameters twice would create exactly the
//! two-sources-of-truth problem §4.2 exists to prevent, one file format layer
//! down, where it would be discovered as "the material changed when I reopened
//! it".

use crate::json::Value;
use crate::json_write::Object;
use crate::{read_spectrum, FormatError};
use tequila_core::math::{vec2, vec3, Vec2};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{
    Link, MaterialGraph, Node, NodeId, NodeKind, Socket, TextureId, Value as GraphValue,
};
use tequila_materials::material_type::Param;
use tequila_materials::nodes::{Setting, ShaderNode};
use tequila_materials::openpbr::Material;
use tequila_texture::mip::Wrap;
use tequila_texture::projection::{Mapping, Projection};

/// The graph's non-surface nodes and its links, or `None` when there are none.
pub(crate) fn graph_to_json(graph: &MaterialGraph) -> Option<Value> {
    if graph.len() <= 1 && graph.links().is_empty() && graph.decals.is_empty() && graph.frames.is_empty() {
        return None;
    }

    let nodes: Vec<Value> = graph
        .nodes()
        .filter(|(id, _)| *id != graph.output())
        .map(|(id, node)| node_to_json(id, node))
        .collect();

    let links: Vec<Value> = graph
        .links()
        .iter()
        .map(|link| {
            let mut object = Object::new();
            object
                .set("from", link.from.0 as f64)
                .set("to", link.to.0 as f64)
                .set("socket", socket_name(link.to_socket));
            // Which of the node's outputs, for a node with more than one;
            // left out for the first, so older files read as they always did.
            if link.from_socket != 0 {
                object.set("output", link.from_socket as f64);
            }
            object.build()
        })
        .collect();

    let mut object = Object::new();
    object
        .set("output", graph.output().0 as f64)
        .set("nodes", Value::Array(nodes))
        .set("links", Value::Array(links));
    object.set("decals", decals_to_json(&graph.decals));
    if !graph.frames.is_empty() {
        object.set(
            "frames",
            Value::Array(
                graph
                    .frames
                    .iter()
                    .map(|frame| {
                        let mut entry = Object::new();
                        entry
                            .set("title", frame.title.clone())
                            .set("min", Value::from(vec![frame.min.x, frame.min.y]))
                            .set("max", Value::from(vec![frame.max.x, frame.max.y]));
                        entry.build()
                    })
                    .collect(),
            ),
        );
    }
    Some(object.build())
}

pub(crate) fn decals_to_json(decals: &[tequila_materials::decal::Decal]) -> Value {
    Value::Array(decals.iter().map(|d| {
        let mut o = Object::new();
        o.set("name", d.name.clone()).set("texture", d.texture.0 as f64)
            .set("mapping", mapping_to_json(&d.mapping)).set("opacity", d.opacity)
            .set("roughness", d.roughness).set("metallic", d.metallic).set("enabled", d.enabled);
        // Absent for a label on both sides, as every label was before.
        if d.side != tequila_materials::decal::DecalSide::Both {
            o.set("side", d.side.key());
        }
        if d.ink != tequila_core::spectrum::Spectrum::ONE {
            o.set("ink", Value::from(vec![d.ink.r, d.ink.g, d.ink.b]));
        }
        o.build()
    }).collect())
}

pub(crate) fn decals_from_json(value: Option<&Value>) -> Result<Vec<tequila_materials::decal::Decal>, FormatError> {
    let Some(value) = value else { return Ok(Vec::new()); };
    value.as_array().iter().map(|v| {
        let texture = v.get("texture").and_then(Value::as_f32)
            .ok_or_else(|| FormatError::Invalid("Label has no texture".into()))?;
        let mut d = tequila_materials::decal::Decal::new(
            v.get("name").and_then(Value::as_str).unwrap_or("Label"), TextureId(texture as u32));
        if let Some(m) = v.get("mapping") { d.mapping = mapping_from_json(m); }
        d.opacity = v.get("opacity").and_then(Value::as_f32).unwrap_or(1.0);
        d.roughness = v.get("roughness").and_then(Value::as_f32).unwrap_or(0.35);
        d.metallic = v.get("metallic").and_then(Value::as_bool).unwrap_or(false);
        d.enabled = v.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        if let Some(ink) = v.get("ink").and_then(Value::as_f32_array).filter(|c| c.len() == 3) {
            d.ink = tequila_core::spectrum::Spectrum { r: ink[0], g: ink[1], b: ink[2] };
        }
        if let Some(side) = v.get("side").and_then(Value::as_str) {
            d.side = tequila_materials::decal::DecalSide::from_key(side)
                .ok_or_else(|| FormatError::Invalid(format!("A label's side is `{side}`: both, positive or negative")))?;
        }
        if !d.is_valid() { return Err(FormatError::Invalid("Invalid label placement or finish".into())); }
        Ok(d)
    }).collect()
}

fn node_to_json(id: NodeId, node: &Node) -> Value {
    let mut object = Object::new();
    object.set("id", id.0 as f64).set("kind", kind_name(&node.kind));

    match &node.kind {
        NodeKind::Surface(_) => {}
        NodeKind::Constant(value) => {
            object.set("value", value_to_json(*value));
        }
        NodeKind::Mapping2d { mapping } => {
            object.set("mapping", mapping_to_json(mapping));
        }
        NodeKind::ImageTexture { texture, fallback }
        | NodeKind::ImageAlpha { texture, fallback } => {
            object
                .set("texture", texture.0 as f64)
                .set("fallback", value_to_json(*fallback));
        }
        NodeKind::Gradient { gradient } => {
            object.set("gradient", gradient_to_json(gradient));
            // Beside the stops rather than among them, so a linear ramp's
            // file is what it always was.
            if gradient.interpolation() != tequila_texture::gradient::Interpolation::Linear {
                object.set("interpolation", gradient.interpolation().key());
            }
        }
        NodeKind::Procedural { procedural, seed } => {
            object.set("procedural", procedural_to_json(procedural));
            object.set("seed", *seed as f64);
        }
        NodeKind::Remap { from, to } => {
            object
                .set("from", Value::from(vec![from.x, from.y]))
                .set("to", Value::from(vec![to.x, to.y]));
        }
        NodeKind::Clamp { low, high } => {
            object.set("low", *low).set("high", *high);
        }
        NodeKind::Channel { channel } => {
            object.set("channel", channel.id());
        }
        NodeKind::Bump { depth } => {
            object.set("depth", *depth);
        }
        NodeKind::Curvature { mode, radius, contrast } => {
            object
                .set("mode", mode.label().to_ascii_lowercase())
                .set("radius", *radius)
                .set("contrast", *contrast);
        }
        NodeKind::Reroute { ty } => {
            object.set("type", ty.label().to_ascii_lowercase());
        }
        NodeKind::Occlusion { radius, contrast } => {
            object.set("radius", *radius).set("contrast", *contrast);
        }
        NodeKind::VertexColor => {}
        NodeKind::VolumeMap { volume } => {
            object.set("volume", volume.0 as f64);
        }
        NodeKind::Bubbles { count, radius, variation, seed } => {
            object
                .set("count", *count as f64)
                .set("radius", *radius)
                .set("variation", *variation)
                .set("seed", *seed as f64);
        }
        NodeKind::Fuzz { length, density, thickness, lean, curl, seed } => {
            object
                .set("length", *length)
                .set("density", *density)
                .set("thickness", *thickness)
                .set("lean", *lean)
                .set("curl", *curl)
                .set("seed", *seed as f64);
        }
        NodeKind::Mix | NodeKind::Multiply | NodeKind::Add | NodeKind::Invert => {}
        // By name: what is inside is the group as this Tequila builds it.
        NodeKind::Group(group) => {
            object.set("group", group.key());
        }
        NodeKind::Node(shader) => {
            object.set("type", shader.key());
            let mut settings = Object::new();
            for spec in shader.settings() {
                match spec.value {
                    Setting::Choice(v) => settings.set(spec.key, v),
                    Setting::Int(v) => settings.set(spec.key, v as f64),
                    Setting::Float(v) => settings.set(spec.key, v),
                    Setting::Bool(v) => settings.set(spec.key, v),
                };
            }
            object.set("settings", settings.build());
        }
    }

    if !node.literals.is_empty() {
        let literals: Vec<Value> = node
            .literals
            .iter()
            .map(|(socket, value)| {
                let mut entry = Object::new();
                entry
                    .set("socket", socket_name(*socket))
                    .set("value", value_to_json(*value));
                entry.build()
            })
            .collect();
        object.set("literals", Value::Array(literals));
    }

    object.set(
        "position",
        Value::from(vec![node.position.x, node.position.y]),
    );
    if node.muted {
        object.set("muted", true);
    }
    if node.collapsed {
        object.set("collapsed", true);
    }
    if let Some(label) = &node.label {
        object.set("label", label.clone());
    }
    object.build()
}

/// Rebuild a graph, taking the surface node's literals from the flat block.
pub(crate) fn read_graph(
    value: Option<&Value>,
    material: Material,
    name: &str,
) -> Result<MaterialGraph, FormatError> {
    let mut graph = MaterialGraph::from_material(material);
    let Some(value) = value else {
        return Ok(graph);
    };

    // The output node's id in the file, so links naming it land on the surface
    // node this graph already has rather than on a second one.
    let stored_output = value
        .get("output")
        .and_then(|v| v.as_f32())
        .map(|v| NodeId(v as u32));

    let mut remapped: Vec<(NodeId, NodeId)> = Vec::new();
    if let Some(nodes) = value.get("nodes").map(|v| v.as_array()) {
        for node in nodes {
            let stored = node
                .get("id")
                .and_then(|v| v.as_f32())
                .map(|v| NodeId(v as u32))
                .ok_or_else(|| FormatError::Invalid(format!("{name}: a node has no id")))?;
            let built = read_node(node, name)?;
            let assigned = graph.add_node(built);
            remapped.push((stored, assigned));
        }
    }

    // Resolved up front rather than in a closure: `connect` needs the graph
    // mutably, and a closure capturing it to read `output()` would hold it
    // borrowed for the whole loop.
    let output = graph.output();
    let resolve = |id: NodeId| -> NodeId {
        if Some(id) == stored_output {
            return output;
        }
        remapped
            .iter()
            .find(|(stored, _)| *stored == id)
            .map(|(_, assigned)| *assigned)
            .unwrap_or(id)
    };

    if let Some(links) = value.get("links").map(|v| v.as_array()) {
        for link in links {
            let (Some(from), Some(to), Some(socket)) = (
                link.get("from").and_then(|v| v.as_f32()),
                link.get("to").and_then(|v| v.as_f32()),
                link.get("socket").and_then(|v| v.as_str()),
            ) else {
                return Err(FormatError::Invalid(format!("{name}: a link is incomplete")));
            };
            let Some(socket) = socket_from_name(socket) else {
                return Err(FormatError::Invalid(format!(
                    "{name}: `{socket}` is not a socket"
                )));
            };
            let output = link.get("output").and_then(|v| v.as_f32()).unwrap_or(0.0).clamp(0.0, 255.0) as u8;
            graph.connect_output(resolve(NodeId(from as u32)), output, resolve(NodeId(to as u32)), socket);
        }
    }

    graph.decals = decals_from_json(value.get("decals"))?;
    graph.frames = value
        .get("frames")
        .map(|frames| {
            frames
                .as_array()
                .iter()
                .filter_map(|entry| {
                    Some(tequila_materials::graph::Frame {
                        title: entry.get("title").and_then(|v| v.as_str()).unwrap_or("Group").to_string(),
                        min: read_vec2(entry.get("min"))?,
                        max: read_vec2(entry.get("max"))?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // A file describing a graph that will not compile is corrupt, and opening
    // it into a state the editor cannot show is worse than refusing it.
    graph
        .validate()
        .map_err(|e| FormatError::Invalid(format!("{name}: {e}")))?;
    Ok(graph)
}

fn read_node(value: &Value, name: &str) -> Result<Node, FormatError> {
    let kind_text = value
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| FormatError::Invalid(format!("{name}: a node has no kind")))?;

    let kind = match kind_text {
        "constant" => NodeKind::Constant(
            value
                .get("value")
                .and_then(value_from_json)
                .unwrap_or(GraphValue::Float(0.0)),
        ),
        "mapping_2d" => NodeKind::Mapping2d {
            mapping: value
                .get("mapping")
                .map(mapping_from_json)
                .unwrap_or_default(),
        },
        "gradient" => NodeKind::Gradient {
            gradient: Box::new(
                value
                    .get("gradient")
                    .and_then(gradient_from_json)
                    .unwrap_or_default()
                    .with_interpolation(
                        value
                            .get("interpolation")
                            .and_then(|v| v.as_str())
                            .and_then(tequila_texture::gradient::Interpolation::from_key)
                            .unwrap_or_default(),
                    ),
            ),
        },
        "procedural" => NodeKind::Procedural {
            procedural: Box::new(
                value
                    .get("procedural")
                    .and_then(procedural_from_json)
                    .unwrap_or_default(),
            ),
            seed: value.get("seed").and_then(|v| v.as_f32()).unwrap_or(0.0) as u32,
        },
        "image_texture" => NodeKind::ImageTexture {
            texture: TextureId(
                value
                    .get("texture")
                    .and_then(|v| v.as_f32())
                    .unwrap_or(0.0) as u32,
            ),
            fallback: value
                .get("fallback")
                .and_then(value_from_json)
                .unwrap_or(GraphValue::Color(Spectrum::splat(0.5))),
        },
        "image_alpha" => NodeKind::ImageAlpha {
            texture: TextureId(
                value
                    .get("texture")
                    .and_then(|v| v.as_f32())
                    .unwrap_or(0.0) as u32,
            ),
            // Solid, not clear: a mask that fails to load must leave the
            // surface present rather than delete the part.
            fallback: value
                .get("fallback")
                .and_then(value_from_json)
                .unwrap_or(GraphValue::Float(1.0)),
        },
        "mix" => NodeKind::Mix,
        "multiply" => NodeKind::Multiply,
        "add" => NodeKind::Add,
        "invert" => NodeKind::Invert,
        "channel" => NodeKind::Channel {
            channel: value
                .get("channel")
                .and_then(|v| v.as_str())
                .and_then(tequila_materials::graph::Channel::from_id)
                .unwrap_or_default(),
        },
        "remap" => NodeKind::Remap {
            from: read_vec2(value.get("from")).unwrap_or(vec2(0.0, 1.0)),
            to: read_vec2(value.get("to")).unwrap_or(vec2(0.0, 1.0)),
        },
        "clamp" => NodeKind::Clamp {
            low: value.get("low").and_then(|v| v.as_f32()).unwrap_or(0.0),
            high: value.get("high").and_then(|v| v.as_f32()).unwrap_or(1.0),
        },
        "curvature" => NodeKind::Curvature {
            mode: match value.get("mode").and_then(|v| v.as_str()).unwrap_or("") {
                "concave" => tequila_materials::graph::CurvatureMode::Concave,
                "both" => tequila_materials::graph::CurvatureMode::Both,
                _ => tequila_materials::graph::CurvatureMode::Convex,
            },
            radius: value
                .get("radius")
                .and_then(|v| v.as_f32())
                .filter(|v| v.is_finite())
                .unwrap_or(tequila_materials::graph::DEFAULT_CURVATURE_RADIUS),
            contrast: value
                .get("contrast")
                .and_then(|v| v.as_f32())
                .filter(|v| v.is_finite())
                .unwrap_or(1.0),
        },
        "vertex_color" => NodeKind::VertexColor,
        "volume_map" => NodeKind::VolumeMap {
            volume: TextureId(value.get("volume").and_then(|v| v.as_f32()).unwrap_or(0.0) as u32),
        },
        "bubbles" => NodeKind::Bubbles {
            count: value.get("count").and_then(|v| v.as_index()).unwrap_or(200).min(20_000) as u32,
            radius: value.get("radius").and_then(|v| v.as_f32()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.002),
            variation: value.get("variation").and_then(|v| v.as_f32()).filter(|v| v.is_finite()).unwrap_or(0.5),
            seed: value.get("seed").and_then(|v| v.as_index()).unwrap_or(0) as u32,
        },
        "fuzz" => {
            let positive = |key: &str, default: f32| value.get(key).and_then(|v| v.as_f32()).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(default);
            let unit = |key: &str, default: f32| value.get(key).and_then(|v| v.as_f32()).filter(|v| v.is_finite()).unwrap_or(default).clamp(0.0, 1.0);
            NodeKind::Fuzz {
                length: positive("length", 0.002),
                density: positive("density", 400.0),
                thickness: positive("thickness", 0.00002),
                lean: unit("lean", 0.35),
                curl: unit("curl", 0.3),
                seed: value.get("seed").and_then(|v| v.as_index()).unwrap_or(0) as u32,
            }
        }
        "occlusion" => NodeKind::Occlusion {
            radius: value
                .get("radius")
                .and_then(|v| v.as_f32())
                .filter(|v| v.is_finite())
                .unwrap_or(tequila_materials::graph::DEFAULT_OCCLUSION_RADIUS),
            contrast: value
                .get("contrast")
                .and_then(|v| v.as_f32())
                .filter(|v| v.is_finite())
                .unwrap_or(1.0),
        },
        "reroute" => NodeKind::Reroute {
            ty: match value.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "float" => tequila_materials::graph::SocketType::Float,
                "vector" => tequila_materials::graph::SocketType::Vector,
                "normal" => tequila_materials::graph::SocketType::Normal,
                "bool" => tequila_materials::graph::SocketType::Bool,
                _ => tequila_materials::graph::SocketType::Color,
            },
        },
        "group" => {
            let key = value.get("group").and_then(|v| v.as_str()).unwrap_or("");
            let Some(group) = tequila_materials::groups::NodeGroup::from_key(key) else {
                return Err(FormatError::Invalid(format!("{name}: `{key}` is not a node group this version has")));
            };
            NodeKind::Group(group)
        }
        "node" => {
            let key = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let Some(mut shader) = ShaderNode::from_key(key) else {
                return Err(FormatError::Invalid(format!("{name}: `{key}` is not a node type")));
            };
            if let Some(settings) = value.get("settings") {
                // Every setting the node can have, whichever its others make
                // it show: a Mix's blend is kept while it mixes numbers.
                for spec in shader.settings() {
                    let Some(stored) = settings.get(spec.key) else {
                        continue;
                    };
                    let setting = match spec.value {
                        Setting::Choice(_) => stored.as_str().map(|s| {
                            // Matched to the option's own `&'static str`.
                            spec.options.iter().find(|(k, _)| *k == s).map(|(k, _)| Setting::Choice(k))
                        }).flatten(),
                        Setting::Int(_) => stored.as_f32().map(|v| Setting::Int(v as i32)),
                        Setting::Float(_) => stored.as_f32().filter(|v| v.is_finite()).map(Setting::Float),
                        Setting::Bool(_) => stored.as_bool().map(Setting::Bool),
                    };
                    if let Some(setting) = setting {
                        shader.set(spec.key, setting);
                    }
                }
            }
            NodeKind::Node(shader)
        }
        "bump" => NodeKind::Bump {
            depth: value
                .get("depth")
                .and_then(|v| v.as_f32())
                .filter(|v| v.is_finite())
                .unwrap_or(tequila_materials::graph::DEFAULT_BUMP_DEPTH),
        },
        other => {
            return Err(FormatError::Invalid(format!(
                "{name}: `{other}` is not a node kind"
            )))
        }
    };

    let mut node = Node::new(kind);
    if let Some(literals) = value.get("literals").map(|v| v.as_array()) {
        for literal in literals {
            let (Some(socket), Some(value)) = (
                literal.get("socket").and_then(|v| v.as_str()),
                literal.get("value").and_then(value_from_json),
            ) else {
                continue;
            };
            if let Some(socket) = socket_from_name(socket) {
                node.literals.insert(socket, value);
            }
        }
    }
    if let Some(position) = read_vec2(value.get("position")) {
        node.position = position;
    }
    node.label = value
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    node.muted = value.get("muted").and_then(|v| v.as_bool()).unwrap_or(false);
    node.collapsed = value.get("collapsed").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(node)
}

fn read_vec2(value: Option<&Value>) -> Option<Vec2> {
    let v = value?.as_f32_array()?;
    (v.len() >= 2).then(|| vec2(v[0], v[1]))
}

// --- small vocabularies --------------------------------------------------

fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Surface(_) => "surface",
        NodeKind::Constant(_) => "constant",
        NodeKind::Mapping2d { .. } => "mapping_2d",
        NodeKind::ImageTexture { .. } => "image_texture",
        NodeKind::ImageAlpha { .. } => "image_alpha",
        NodeKind::Node(_) => "node",
        NodeKind::Group(_) => "group",
        NodeKind::Mix => "mix",
        NodeKind::Multiply => "multiply",
        NodeKind::Add => "add",
        NodeKind::Invert => "invert",
        NodeKind::Channel { .. } => "channel",
        NodeKind::Remap { .. } => "remap",
        NodeKind::Clamp { .. } => "clamp",
        NodeKind::Procedural { .. } => "procedural",
        NodeKind::Gradient { .. } => "gradient",
        NodeKind::Bump { .. } => "bump",
        NodeKind::Curvature { .. } => "curvature",
        NodeKind::Reroute { .. } => "reroute",
        NodeKind::Occlusion { .. } => "occlusion",
        NodeKind::VertexColor => "vertex_color",
        NodeKind::VolumeMap { .. } => "volume_map",
        NodeKind::Bubbles { .. } => "bubbles",
        NodeKind::Fuzz { .. } => "fuzz",
    }
}

/// A ramp, as its stops.
pub(crate) fn gradient_to_json(gradient: &tequila_texture::gradient::Gradient) -> Value {
    Value::Array(
        gradient
            .stops()
            .iter()
            .map(|stop| {
                let mut object = Object::new();
                object
                    .set("at", stop.at)
                    .set("color", crate::color(stop.color));
                object.build()
            })
            .collect(),
    )
}

pub(crate) fn gradient_from_json(value: &Value) -> Option<tequila_texture::gradient::Gradient> {
    use tequila_texture::gradient::{Gradient, Stop};
    let stops: Vec<Stop> = value
        .as_array()
        .iter()
        .filter_map(|entry| {
            Some(Stop {
                at: entry.get("at").and_then(|v| v.as_f32())?,
                color: entry.get("color").and_then(crate::read_spectrum)?,
            })
        })
        .collect();
    // Fewer than two is not a ramp. A file that has lost its stops gets the
    // default rather than a node that answers every position with one colour.
    (stops.len() >= 2).then(|| Gradient::new(stops))
}

/// A procedural, as its kind plus that kind's own parameters.
///
/// Written by name rather than as a tagged number: a file that says
/// `"scratches"` survives the day a variant is inserted in the middle of the
/// enum, and a number does not.
pub(crate) fn procedural_to_json(p: &tequila_texture::procedural::Procedural) -> Value {
    use tequila_texture::procedural::Procedural;
    let mut object = Object::new();
    object.set("kind", p.key());
    match p {
        Procedural::Noise { fractal, frequency } => {
            object
                .set("frequency", *frequency)
                .set("octaves", fractal.octaves as f64)
                .set("lacunarity", fractal.lacunarity)
                .set("gain", fractal.gain)
                .set("basis", basis_name(fractal.basis));
        }
        Procedural::Cellular {
            metric,
            output,
            frequency,
        } => {
            object
                .set("frequency", *frequency)
                .set("metric", metric_name(*metric))
                .set("output", cell_output_name(*output));
        }
        Procedural::Scratches {
            frequency,
            width,
            density,
            spread,
        } => {
            object
                .set("frequency", *frequency)
                .set("width", *width)
                .set("density", *density)
                .set("spread", *spread);
        }
        Procedural::Marble {
            axis,
            frequency,
            distortion,
            fractal,
        } => {
            object
                .set("axis", axis_name(*axis))
                .set("frequency", *frequency)
                .set("distortion", *distortion)
                .set("octaves", fractal.octaves as f64)
                .set("lacunarity", fractal.lacunarity)
                .set("gain", fractal.gain)
                .set("basis", basis_name(fractal.basis));
        }
        Procedural::Wood {
            axis,
            frequency,
            distortion,
            grain,
            fractal,
        } => {
            object
                .set("axis", axis_name(*axis))
                .set("frequency", *frequency)
                .set("distortion", *distortion)
                .set("grain", *grain)
                .set("octaves", fractal.octaves as f64)
                .set("lacunarity", fractal.lacunarity)
                .set("gain", fractal.gain)
                .set("basis", basis_name(fractal.basis));
        }
        Procedural::WoodAdvanced {
            axis,
            frequency,
            distortion,
            grain,
            fractal,
            ring_variation,
            latewood,
            contrast,
            flecks,
            knots,
            board_width,
        } => {
            object
                .set("axis", axis_name(*axis))
                .set("frequency", *frequency)
                .set("distortion", *distortion)
                .set("grain", *grain)
                .set("octaves", fractal.octaves as f64)
                .set("lacunarity", fractal.lacunarity)
                .set("gain", fractal.gain)
                .set("basis", basis_name(fractal.basis))
                .set("ring_variation", *ring_variation)
                .set("latewood", *latewood)
                .set("contrast", *contrast)
                .set("flecks", *flecks)
                .set("knots", *knots)
                .set("board_width", *board_width);
        }
        Procedural::Brushed { frequency, stretch } => {
            object.set("frequency", *frequency).set("stretch", *stretch);
        }
        Procedural::NoiseTexture {
            basis,
            frequency,
            contrast,
            bias,
        } => {
            object
                .set("frequency", *frequency)
                .set("contrast", *contrast)
                .set("bias", *bias)
                .set("basis", basis_name(*basis));
        }
        Procedural::Ridged { fractal, frequency } => {
            object
                .set("frequency", *frequency)
                .set("octaves", fractal.octaves as f64)
                .set("lacunarity", fractal.lacunarity)
                .set("gain", fractal.gain)
                .set("basis", basis_name(fractal.basis));
        }
        Procedural::Granite {
            frequency,
            speckle,
            fine,
        } => {
            object
                .set("frequency", *frequency)
                .set("speckle", *speckle)
                .set("fine", *fine);
        }
        Procedural::Leather {
            frequency,
            crack_width,
            wrinkle,
        } => {
            object
                .set("frequency", *frequency)
                .set("crack_width", *crack_width)
                .set("wrinkle", *wrinkle);
        }
        Procedural::Spots {
            frequency,
            coverage,
            variance,
            softness,
        } => {
            object
                .set("frequency", *frequency)
                .set("coverage", *coverage)
                .set("variance", *variance)
                .set("softness", *softness);
        }
        Procedural::Camouflage { frequency, layers } => {
            object.set("frequency", *frequency).set("layers", *layers as f64);
        }
        Procedural::Contour {
            axis,
            frequency,
            width,
            softness,
            distortion,
        } => {
            object
                .set("axis", axis_name(*axis))
                .set("frequency", *frequency)
                .set("width", *width)
                .set("softness", *softness)
                .set("distortion", *distortion);
        }
        Procedural::BrushedRadial {
            axis,
            frequency,
            stretch,
        } => {
            object
                .set("axis", axis_name(*axis))
                .set("frequency", *frequency)
                .set("stretch", *stretch);
        }
        Procedural::Weave {
            pattern,
            frequency,
            width,
            variance,
        } => {
            object
                .set("pattern", weave_pattern_name(*pattern))
                .set("frequency", *frequency)
                .set("width", *width)
                .set("variance", *variance);
        }
        Procedural::Mesh {
            shape,
            frequency,
            size,
            softness,
        } => {
            object
                .set("shape", hole_shape_name(*shape))
                .set("frequency", *frequency)
                .set("size", *size)
                .set("softness", *softness);
        }
    }
    object.build()
}

fn weave_pattern_name(pattern: tequila_texture::procedural::WeavePattern) -> &'static str {
    use tequila_texture::procedural::WeavePattern;
    match pattern {
        WeavePattern::Plain => "plain",
        WeavePattern::Twill => "twill",
        WeavePattern::Satin => "satin",
    }
}

fn hole_shape_name(shape: tequila_texture::procedural::HoleShape) -> &'static str {
    use tequila_texture::procedural::HoleShape;
    match shape {
        HoleShape::Round => "round",
        HoleShape::Square => "square",
        HoleShape::Hex => "hex",
    }
}

pub(crate) fn procedural_from_json(value: &Value) -> Option<tequila_texture::procedural::Procedural> {
    use tequila_texture::noise::{Basis, Fractal, Metric};
    use tequila_texture::procedural::{CellOutput, HoleShape, Procedural, WeavePattern};

    let number = |key: &str, fallback: f32| {
        value
            .get(key)
            .and_then(|v| v.as_f32())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    };
    let name = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or("");

    Some(match value.get("kind").and_then(|v| v.as_str())? {
        "noise" => Procedural::Noise {
            fractal: Fractal {
                octaves: number("octaves", 4.0).clamp(1.0, 12.0) as u32,
                lacunarity: number("lacunarity", 2.0),
                gain: number("gain", 0.5),
                basis: match name("basis") {
                    "value" => Basis::Value,
                    _ => Basis::Perlin,
                },
            },
            frequency: number("frequency", 4.0),
        },
        "cellular" => Procedural::Cellular {
            metric: match name("metric") {
                "manhattan" => Metric::Manhattan,
                "chebyshev" => Metric::Chebyshev,
                _ => Metric::Euclidean,
            },
            output: match name("output") {
                "edges" => CellOutput::Edges,
                "id" => CellOutput::Id,
                _ => CellOutput::Distance,
            },
            frequency: number("frequency", 6.0),
        },
        "scratches" => Procedural::Scratches {
            frequency: number("frequency", 8.0),
            width: number("width", 0.06),
            density: number("density", 0.3),
            spread: number("spread", 0.05),
        },
        "marble" => Procedural::Marble {
            axis: axis_from_name(name("axis")),
            frequency: number("frequency", 3.0),
            distortion: number("distortion", 2.5),
            fractal: fractal_from(value, 5),
        },
        "wood" => Procedural::Wood {
            axis: axis_from_name(name("axis")),
            frequency: number("frequency", 8.0),
            distortion: number("distortion", 0.35),
            grain: number("grain", 0.35),
            fractal: fractal_from(value, 3),
        },
        "wood_advanced" => Procedural::WoodAdvanced {
            axis: axis_from_name(name("axis")),
            frequency: number("frequency", 8.0),
            distortion: number("distortion", 0.3),
            grain: number("grain", 0.25),
            fractal: fractal_from(value, 3),
            ring_variation: number("ring_variation", 0.5),
            latewood: number("latewood", 0.3),
            contrast: number("contrast", 0.8),
            flecks: number("flecks", 0.3),
            knots: number("knots", 0.25),
            board_width: number("board_width", 0.0),
        },
        "brushed" => Procedural::Brushed {
            frequency: number("frequency", 60.0),
            stretch: number("stretch", 40.0),
        },
        "noise_texture" => Procedural::NoiseTexture {
            basis: match name("basis") {
                "value" => Basis::Value,
                _ => Basis::Perlin,
            },
            frequency: number("frequency", 8.0),
            contrast: number("contrast", 1.0),
            bias: number("bias", 0.0),
        },
        "ridged" => Procedural::Ridged {
            fractal: fractal_from(value, 5),
            frequency: number("frequency", 3.0),
        },
        "granite" => Procedural::Granite {
            frequency: number("frequency", 10.0),
            speckle: number("speckle", 0.15),
            fine: number("fine", 0.3),
        },
        "leather" => Procedural::Leather {
            frequency: number("frequency", 25.0),
            crack_width: number("crack_width", 0.12),
            wrinkle: number("wrinkle", 0.25),
        },
        "spots" => Procedural::Spots {
            frequency: number("frequency", 8.0),
            coverage: number("coverage", 0.5),
            variance: number("variance", 0.3),
            softness: number("softness", 0.1),
        },
        "camouflage" => Procedural::Camouflage {
            frequency: number("frequency", 3.0),
            layers: number("layers", 4.0).clamp(2.0, 5.0) as u32,
        },
        "contour" => Procedural::Contour {
            axis: axis_from_name(name("axis")),
            frequency: number("frequency", 10.0),
            width: number("width", 0.1),
            softness: number("softness", 0.05),
            distortion: number("distortion", 0.8),
        },
        "brushed_radial" => Procedural::BrushedRadial {
            axis: axis_from_name(name("axis")),
            frequency: number("frequency", 200.0),
            stretch: number("stretch", 60.0),
        },
        "weave" => Procedural::Weave {
            pattern: match name("pattern") {
                "twill" => WeavePattern::Twill,
                "satin" => WeavePattern::Satin,
                _ => WeavePattern::Plain,
            },
            frequency: number("frequency", 40.0),
            width: number("width", 0.85),
            variance: number("variance", 0.2),
        },
        "mesh" => Procedural::Mesh {
            shape: match name("shape") {
                "square" => HoleShape::Square,
                "hex" => HoleShape::Hex,
                _ => HoleShape::Round,
            },
            frequency: number("frequency", 20.0),
            size: number("size", 0.6),
            softness: number("softness", 0.05),
        },
        _ => return None,
    })
}

fn axis_name(axis: tequila_texture::procedural::Axis) -> &'static str {
    use tequila_texture::procedural::Axis;
    match axis {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

fn axis_from_name(name: &str) -> tequila_texture::procedural::Axis {
    use tequila_texture::procedural::Axis;
    match name {
        "x" => Axis::X,
        "z" => Axis::Z,
        // Y is the default and the fallback both: a file with a missing or
        // unrecognised axis gets the grain running up, which is how a plank
        // and a slab are usually modelled.
        _ => Axis::Y,
    }
}

/// The fractal block, shared by every node that carries one.
///
/// Read through the same keys the noise node writes, so a `fractal` means the
/// same thing wherever it appears — one vocabulary rather than three that
/// drift.
fn fractal_from(value: &Value, default_octaves: u32) -> tequila_texture::noise::Fractal {
    use tequila_texture::noise::{Basis, Fractal};
    let number = |key: &str, fallback: f32| {
        value
            .get(key)
            .and_then(|v| v.as_f32())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    };
    Fractal {
        octaves: number("octaves", default_octaves as f32).clamp(1.0, 12.0) as u32,
        lacunarity: number("lacunarity", 2.0),
        gain: number("gain", 0.5),
        basis: match value.get("basis").and_then(|v| v.as_str()).unwrap_or("") {
            "value" => Basis::Value,
            _ => Basis::Perlin,
        },
    }
}

fn basis_name(basis: tequila_texture::noise::Basis) -> &'static str {
    use tequila_texture::noise::Basis;
    match basis {
        Basis::Perlin => "perlin",
        Basis::Value => "value",
    }
}

fn metric_name(metric: tequila_texture::noise::Metric) -> &'static str {
    use tequila_texture::noise::Metric;
    match metric {
        Metric::Euclidean => "euclidean",
        Metric::Manhattan => "manhattan",
        Metric::Chebyshev => "chebyshev",
    }
}

fn cell_output_name(output: tequila_texture::procedural::CellOutput) -> &'static str {
    use tequila_texture::procedural::CellOutput;
    match output {
        CellOutput::Distance => "distance",
        CellOutput::Edges => "edges",
        CellOutput::Id => "id",
    }
}

/// `param:base_color` or `input:2`.
fn socket_name(socket: Socket) -> String {
    match socket {
        Socket::Param(p) => format!("param:{}", p.key()),
        Socket::Input(i) => format!("input:{i}"),
    }
}

fn socket_from_name(text: &str) -> Option<Socket> {
    let (tag, rest) = text.split_once(':')?;
    match tag {
        "param" => Param::from_key(rest).map(Socket::Param),
        "input" => rest.parse::<u8>().ok().map(Socket::Input),
        _ => None,
    }
}

fn value_to_json(value: GraphValue) -> Value {
    let mut object = Object::new();
    match value {
        GraphValue::Float(f) => {
            object.set("float", f);
        }
        GraphValue::Color(c) => {
            object.set("color", Value::from(vec![c.r, c.g, c.b]));
        }
        GraphValue::Vector(v) => {
            object.set("vector", Value::from(vec![v.x, v.y, v.z]));
        }
        GraphValue::Bool(b) => {
            object.set("bool", b);
        }
    }
    object.build()
}

fn value_from_json(value: &Value) -> Option<GraphValue> {
    if let Some(f) = value.get("float").and_then(|v| v.as_f32()) {
        return Some(GraphValue::Float(f));
    }
    if let Some(c) = value.get("color").and_then(read_spectrum) {
        return Some(GraphValue::Color(c));
    }
    if let Some(v) = value.get("vector").and_then(|v| v.as_f32_array()) {
        if v.len() >= 3 {
            return Some(GraphValue::Vector(vec3(v[0], v[1], v[2])));
        }
    }
    if let Some(b) = value.get("bool").and_then(|v| v.as_bool()) {
        return Some(GraphValue::Bool(b));
    }
    None
}

fn projection_name(projection: Projection) -> &'static str {
    match projection {
        Projection::Uv => "uv",
        Projection::Planar => "planar",
        Projection::Cylindrical => "cylindrical",
        Projection::Spherical => "spherical",
        Projection::TriPlanar => "tri_planar",
    }
}

fn projection_from_name(text: &str) -> Projection {
    match text {
        "planar" => Projection::Planar,
        "cylindrical" => Projection::Cylindrical,
        "spherical" => Projection::Spherical,
        "tri_planar" => Projection::TriPlanar,
        _ => Projection::Uv,
    }
}

fn wrap_name(wrap: Wrap) -> &'static str {
    match wrap {
        Wrap::Repeat => "repeat",
        Wrap::Clamp => "clamp",
        Wrap::Mirror => "mirror",
    }
}

fn wrap_from_name(text: &str) -> Wrap {
    match text {
        "clamp" => Wrap::Clamp,
        "mirror" => Wrap::Mirror,
        _ => Wrap::Repeat,
    }
}

fn mapping_to_json(mapping: &Mapping) -> Value {
    let mut object = Object::new();
    object
        .set("projection", projection_name(mapping.projection))
        .set("axis", axis_name(mapping.axis))
        .set("uv_set", mapping.uv_set as f64)
        .set(
            "scale",
            Value::from(vec![mapping.scale.x, mapping.scale.y, mapping.scale.z]),
        )
        .set(
            "offset",
            Value::from(vec![mapping.offset.x, mapping.offset.y]),
        )
        .set("rotation", mapping.rotation)
        .set(
            "pivot",
            Value::from(vec![mapping.pivot.x, mapping.pivot.y]),
        )
        .set("wrap_u", wrap_name(mapping.wrap_u))
        .set("wrap_v", wrap_name(mapping.wrap_v))
        .set("flip_u", mapping.flip_u)
        .set("flip_v", mapping.flip_v)
        .set("blend_sharpness", mapping.blend_sharpness)
        .set("real_world_units", mapping.real_world_units);
    object.build()
}

fn mapping_from_json(value: &Value) -> Mapping {
    let default = Mapping::default();
    let scalar = |key: &str, fallback: f32| {
        value
            .get(key)
            .and_then(|v| v.as_f32())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    };
    let flag = |key: &str, fallback: bool| {
        value.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
    };
    let wrap = |key: &str, fallback: Wrap| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(wrap_from_name)
            .unwrap_or(fallback)
    };

    let projection = value
        .get("projection")
        .and_then(|v| v.as_str())
        .map(projection_from_name)
        .unwrap_or(default.projection);

    Mapping {
        projection,
        // A file written before the axis existed has to keep meaning what it
        // meant. Planar always looked down Z and cylindrical always wrapped
        // about Y, so those are the fallbacks — per projection, because there
        // is no single axis that leaves both unchanged.
        axis: match value.get("axis").and_then(|v| v.as_str()) {
            Some(name) => axis_from_name(name),
            None if projection == Projection::Cylindrical => {
                tequila_texture::projection::Axis::Y
            }
            None => tequila_texture::projection::Axis::Z,
        },
        uv_set: scalar("uv_set", 0.0).max(0.0) as usize,
        scale: value
            .get("scale")
            .and_then(|v| v.as_f32_array())
            .filter(|v| v.len() >= 3)
            .map(|v| vec3(v[0], v[1], v[2]))
            .unwrap_or(default.scale),
        offset: read_vec2(value.get("offset")).unwrap_or(default.offset),
        rotation: scalar("rotation", default.rotation),
        pivot: read_vec2(value.get("pivot")).unwrap_or(default.pivot),
        wrap_u: wrap("wrap_u", default.wrap_u),
        wrap_v: wrap("wrap_v", default.wrap_v),
        flip_u: flag("flip_u", default.flip_u),
        flip_v: flag("flip_v", default.flip_v),
        blend_sharpness: scalar("blend_sharpness", default.blend_sharpness),
        real_world_units: flag("real_world_units", default.real_world_units),
    }
}

/// Not used directly, but keeps `Link` imported for readers of this module.
#[allow(dead_code)]
fn link_shape(_: Link) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec2;
    use tequila_materials::graph::{Node, Socket};
    use tequila_materials::material_type::MaterialType;

    /// A graph with one of everything worth serialising.
    fn textured() -> MaterialGraph {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let output = graph.output();

        let mut mapping_node = Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: Projection::TriPlanar,
                // Deliberately neither the struct default (Z) nor the axis
                // the file reader falls back to for a projection with none
                // recorded, so a dropped field cannot survive the round trip.
                axis: tequila_texture::projection::Axis::X,
                uv_set: 2,
                scale: vec3(3.0, 4.0, 1.0),
                offset: vec2(0.25, -0.5),
                rotation: 0.125,
                pivot: vec2(0.75, -0.25),
                wrap_u: Wrap::Mirror,
                wrap_v: Wrap::Clamp,
                flip_u: true,
                flip_v: false,
                blend_sharpness: 4.5,
                real_world_units: true,
            },
        });
        mapping_node.position = vec2(-300.0, 20.0);
        mapping_node.label = Some("Panel mapping".into());
        let mapping = graph.add_node(mapping_node);

        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(17),
            fallback: GraphValue::Color(Spectrum::splat(0.42)),
        }));

        let tint = graph.add_node(Node::new(NodeKind::Constant(GraphValue::Color(
            tequila_core::spectrum::spectrum(0.9, 0.3, 0.2),
        ))));
        let multiply = graph.add_node(Node::new(NodeKind::Multiply));

        let rough = graph.add_node(
            Node::new(NodeKind::Clamp {
                low: 0.05,
                high: 0.8,
            })
            .with_literal(Socket::Input(0), GraphValue::Float(0.6)),
        );

        graph.connect(mapping, texture, Socket::Input(0));
        graph.connect(texture, multiply, Socket::Input(0));
        graph.connect(tint, multiply, Socket::Input(1));
        graph.connect(multiply, output, Socket::Param(Param::BaseColor));
        graph.connect(rough, output, Socket::Param(Param::SpecularRoughness));
        graph
    }

    #[test]
    fn label_only_graph_preserves_order_placement_and_finish() {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let mut label = tequila_materials::decal::Decal::new("Logo", TextureId(7));
        label.opacity = 0.45;
        label.mapping.offset.x = -0.25;
        label.mapping.rotation = 0.125;
        graph.decals.push(label.clone());
        label.name = "Top print".into();
        label.enabled = false;
        label.metallic = true;
        label.side = tequila_materials::decal::DecalSide::Negative;
        label.ink = tequila_core::spectrum::Spectrum { r: 0.45, g: 0.46, b: 0.46 };
        graph.decals.push(label);
        assert_eq!(round_trip(&graph).decals, graph.decals);
        // A label on both sides writes nothing new: as a file from before.
        let written = crate::json_write::to_string(&decals_to_json(&graph.decals[..1]));
        assert!(!written.contains("side") && !written.contains("ink"), "{written}");
        let wrong = crate::json::parse(br#"[{"texture":1,"side":"inside"}]"#).unwrap();
        assert!(decals_from_json(Some(&wrong)).is_err());
    }

    /// Reroutes keep their type and their wiring, and groups their title and
    /// box, across a save; a graph that is only a group still saves.
    #[test]
    fn reroutes_and_groups_round_trip() {
        use tequila_materials::graph::{Frame, SocketType};
        let mut graph = textured();
        let hop = graph.add_node(Node::new(NodeKind::Reroute { ty: SocketType::Vector }));
        graph.frames.push(Frame {
            title: "Wood grain".into(),
            min: vec2(-40.0, 10.0),
            max: vec2(320.0, 260.0),
        });
        let back = round_trip(&graph);
        assert_eq!(back.frames, graph.frames);
        assert!(back
            .nodes()
            .any(|(_, n)| n.kind == NodeKind::Reroute { ty: SocketType::Vector }));
        let _ = hop;

        let mut only_a_group = MaterialGraph::from_material(Material::default());
        only_a_group.frames.push(Frame {
            title: "Empty".into(),
            min: vec2(0.0, 0.0),
            max: vec2(100.0, 100.0),
        });
        assert_eq!(round_trip(&only_a_group).frames.len(), 1);
    }

    fn round_trip(graph: &MaterialGraph) -> MaterialGraph {
        let json = graph_to_json(graph).expect("a graph with nodes serialises");
        read_graph(Some(&json), graph.material().clone(), "test")
            .expect("what was just written must read back")
    }

    /// A ramp keeps its interpolation.
    #[test]
    fn a_ramp_keeps_its_interpolation() {
        use tequila_texture::gradient::{Gradient, Interpolation};
        let mut graph = textured();
        let ramp = Gradient::default().with_interpolation(Interpolation::Constant);
        graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(ramp.clone()) }));
        let back = round_trip(&graph);
        assert!(back.nodes().any(|(_, n)| matches!(&n.kind, NodeKind::Gradient { gradient } if **gradient == ramp)));
    }

    /// Blender nodes come back as they were saved: their settings, the
    /// output each wire leaves from, their literals, muted and collapsed.
    #[test]
    fn blender_nodes_round_trip() {
        use tequila_materials::nodes::{Setting, ShaderNode};
        let mut graph = textured();
        let mut voronoi = ShaderNode::voronoi();
        assert!(voronoi.set("feature", Setting::Choice("smooth_f1")));
        assert!(voronoi.set("metric", Setting::Choice("manhattan")));
        assert!(voronoi.set("dimensions", Setting::Choice("4D")));
        let cells = graph.add_node(Node::new(NodeKind::Node(voronoi)).with_literal(Socket::Input(2), GraphValue::Float(7.5)));
        let mut mix = ShaderNode::catalog().into_iter().find(|n| n.key() == "mix").unwrap();
        assert!(mix.set("blend_type", Setting::Choice("overlay")));
        let mut mixer = Node::new(NodeKind::Node(mix));
        mixer.muted = true;
        mixer.collapsed = true;
        let mixer = graph.add_node(mixer);
        // The cells' colour, their second output, into the mix's A.
        graph.connect_output(cells, 1, mixer, Socket::Input(1));
        let back = round_trip(&graph);
        let find = |g: &MaterialGraph, key: &str| g.nodes().find(|(_, n)| matches!(&n.kind, NodeKind::Node(s) if s.key() == key)).map(|(id, n)| (id, n.clone())).unwrap();
        let ((cells_back, a), (mixer_back, b)) = (find(&back, "voronoi_texture"), find(&back, "mix"));
        assert_eq!(a.kind, NodeKind::Node(voronoi));
        assert_eq!(a.literals.get(&Socket::Input(2)), Some(&GraphValue::Float(7.5)));
        assert_eq!(b.kind, NodeKind::Node(mix));
        assert!(b.muted && b.collapsed);
        assert!(back.links().iter().any(|l| l.from == cells_back && l.from_socket == 1 && l.to == mixer_back && l.to_socket == Socket::Input(1)));
    }

    /// The whole point: a graph survives a save and reload unchanged.
    #[test]
    fn a_graph_round_trips() {
        let original = textured();
        let recovered = round_trip(&original);

        assert_eq!(
            original.len(),
            recovered.len(),
            "node count changed across the round trip"
        );
        assert_eq!(
            original.links().len(),
            recovered.links().len(),
            "link count changed across the round trip"
        );
        assert_eq!(
            original.content_hash(),
            recovered.content_hash(),
            "the graph came back different from how it went in"
        );
    }

    /// A Curvature node keeps its settings across a save.
    #[test]
    fn a_curvature_node_round_trips() {
        use tequila_materials::graph::CurvatureMode;
        let mut graph = textured();
        let node = graph.add_node(Node::new(NodeKind::Curvature {
            mode: CurvatureMode::Concave,
            radius: 0.0035,
            contrast: 2.5,
        }));
        let output = graph.output();
        graph.connect(node, output, Socket::Param(Param::SpecularRoughness));
        let recovered = round_trip(&graph);
        assert_eq!(recovered.content_hash(), graph.content_hash());
    }

    /// A Fuzz node keeps every setting across a save, and still asks the
    /// scene for its fibres.
    #[test]
    fn a_fuzz_node_round_trips() {
        let mut graph = textured();
        let fuzz = NodeKind::Fuzz { length: 0.0031, density: 640.0, thickness: 0.000017, lean: 0.4, curl: 0.65, seed: 9 };
        graph.add_node(Node::new(fuzz.clone()));
        let recovered = round_trip(&graph);
        assert_eq!(recovered.content_hash(), graph.content_hash());
        assert!(recovered.nodes().any(|(_, n)| n.kind == fuzz));
        assert!(recovered.compile().expect("compiles").requirements.fuzz.is_some());
    }

    /// A Bump node keeps its depth across a save.
    #[test]
    fn a_bump_node_round_trips_with_its_depth() {
        let mut graph = textured();
        let bump = graph.add_node(Node::new(NodeKind::Bump { depth: 0.00123 }));
        let output = graph.output();
        graph.connect(bump, output, Socket::Param(Param::Bump));
        let recovered = round_trip(&graph);
        assert_eq!(recovered.content_hash(), graph.content_hash());
        assert!(recovered
            .nodes()
            .any(|(_, n)| n.kind == NodeKind::Bump { depth: 0.00123 }));
    }

    /// Writing and reading twice must not drift — the second pass has to be
    /// identical to the first, or a file mutates a little every time it is
    /// opened and saved.
    #[test]
    fn a_second_round_trip_changes_nothing() {
        let once = round_trip(&textured());
        let twice = round_trip(&once);
        assert_eq!(once.content_hash(), twice.content_hash());
        assert_eq!(
            graph_to_json(&once).map(|v| crate::json_write::to_string(&v)),
            graph_to_json(&twice).map(|v| crate::json_write::to_string(&v)),
            "the serialised form drifted on a second pass"
        );
    }

    /// A material with no textures writes no graph at all, so its file is
    /// exactly what it was before graphs existed.
    #[test]
    fn a_textureless_material_writes_no_graph() {
        let plain = MaterialGraph::from_material(Material::default());
        assert!(
            graph_to_json(&plain).is_none(),
            "a one-node graph emitted a graph object, changing every existing file"
        );
    }

    /// A file with no graph object still opens, and gives back the one-node
    /// graph a plain material is.
    #[test]
    fn a_file_without_a_graph_still_opens() {
        let material = MaterialType::Metal.defaults();
        let graph = read_graph(None, material.clone(), "test").expect("opens");
        assert_eq!(graph.len(), 1);
        assert_eq!(graph.material(), &material);
    }

    /// The surface node's parameters come from the flat block, not from inside
    /// the graph object. Writing them twice is the two-sources-of-truth problem
    /// §4.2 exists to prevent, and it would show up as a material changing when
    /// the file is reopened.
    #[test]
    fn the_parameter_block_is_not_written_twice() {
        let graph = textured();
        let json = graph_to_json(&graph).expect("serialises");
        let text = crate::json_write::to_string(&json);
        for param in [Param::BaseColor, Param::SpecularIor, Param::CoatWeight] {
            assert!(
                !text.contains(&format!("\"{}\"", param.key())),
                "`{}` appears inside the graph object as well as in the flat block",
                param.key()
            );
        }
        // ...and the block does come back, from the material handed in.
        let distinctive = Material {
            coat_weight: 0.77,
            ..MaterialType::Plastic.defaults()
        };
        let recovered = read_graph(Some(&json), distinctive.clone(), "test").expect("reads");
        assert_eq!(recovered.material().coat_weight, 0.77);
    }

    /// Node ids are reassigned on load, so links have to be remapped with them.
    /// A link that still pointed at a file-relative id would attach to the
    /// wrong node, or to none.
    #[test]
    fn links_survive_the_renumbering_of_nodes() {
        let original = textured();
        let recovered = round_trip(&original);

        assert!(
            recovered.is_connected(Param::BaseColor),
            "the link into base colour was lost in renumbering"
        );
        assert!(recovered.is_connected(Param::SpecularRoughness));

        // The chain is intact: base colour is driven by a multiply, which is
        // fed by a texture, which is fed by a mapping.
        let multiply = recovered.driver_of(Param::BaseColor).expect("a driver");
        assert!(matches!(
            recovered.node(multiply).map(|n| &n.kind),
            Some(NodeKind::Multiply)
        ));
        let feeding: Vec<_> = recovered
            .links()
            .iter()
            .filter(|l| l.to == multiply)
            .map(|l| recovered.node(l.from).map(|n| n.kind.label()))
            .collect();
        assert_eq!(feeding.len(), 2, "the multiply lost an input");
        assert!(feeding.contains(&Some("Image Texture")));
        assert!(feeding.contains(&Some("Constant")));
    }

    /// Every field of a mapping survives, not just the projection. A silently
    /// dropped `real_world_units` would rescale every texture in the scene on
    /// reload.
    #[test]
    fn every_mapping_control_survives() {
        let recovered = round_trip(&textured());
        let mapping = recovered
            .nodes()
            .find_map(|(_, n)| match &n.kind {
                NodeKind::Mapping2d { mapping } => Some(*mapping),
                _ => None,
            })
            .expect("the mapping node came back");

        assert_eq!(mapping.projection, Projection::TriPlanar);
        assert_eq!(mapping.uv_set, 2);
        assert_eq!(mapping.scale.x, 3.0);
        assert_eq!(mapping.offset.y, -0.5);
        assert_eq!(mapping.rotation, 0.125);
        assert_eq!(mapping.wrap_u, Wrap::Mirror);
        assert_eq!(mapping.wrap_v, Wrap::Clamp);
        assert!(mapping.flip_u && !mapping.flip_v);
        assert_eq!(mapping.blend_sharpness, 4.5);
        assert!(mapping.real_world_units);
    }

    /// Editor-only state survives too — reopening a file must not scatter the
    /// nodes back to the origin.
    #[test]
    fn node_positions_and_labels_survive() {
        let recovered = round_trip(&textured());
        let placed = recovered
            .nodes()
            .find(|(_, n)| n.label.as_deref() == Some("Panel mapping"))
            .expect("the labelled node came back");
        assert_eq!(placed.1.position, vec2(-300.0, 20.0));
    }

    /// A file describing a graph that cannot compile is corrupt, and opening it
    /// into a state the editor cannot display is worse than refusing it.
    #[test]
    fn a_corrupt_graph_is_refused_rather_than_half_loaded() {
        let mut object = crate::json_write::Object::new();
        object
            .set("output", 1.0f64)
            .set("nodes", Value::Array(Vec::new()))
            .set(
                "links",
                Value::Array(vec![{
                    let mut link = crate::json_write::Object::new();
                    link.set("from", 99.0f64)
                        .set("to", 1.0f64)
                        .set("socket", "param:base_color");
                    link.build()
                }]),
            );
        let json = object.build();

        let result = read_graph(Some(&json), Material::default(), "broken");
        assert!(
            result.is_err(),
            "a link to a node that does not exist was accepted"
        );
    }

    #[test]
    fn an_unknown_node_kind_is_reported() {
        let mut node = crate::json_write::Object::new();
        node.set("id", 2.0f64).set("kind", "quantum_flux");
        let mut object = crate::json_write::Object::new();
        object
            .set("output", 1.0f64)
            .set("nodes", Value::Array(vec![node.build()]))
            .set("links", Value::Array(Vec::new()));

        let result = read_graph(Some(&object.build()), Material::default(), "future");
        match result {
            Err(FormatError::Invalid(message)) => {
                assert!(message.contains("quantum_flux"), "got: {message}")
            }
            other => panic!("expected a clear error, got {other:?}"),
        }
    }

    /// Socket names round-trip, including every parameter.
    #[test]
    fn every_socket_name_round_trips() {
        for param in Param::ALL {
            let socket = Socket::Param(param);
            assert_eq!(socket_from_name(&socket_name(socket)), Some(socket));
        }
        for i in [0u8, 1, 7, 255] {
            let socket = Socket::Input(i);
            assert_eq!(socket_from_name(&socket_name(socket)), Some(socket));
        }
        assert_eq!(socket_from_name("nonsense"), None);
        assert_eq!(socket_from_name("param:no_such_parameter"), None);
    }
}
