//! `graph`: a material's node graph, a node at a time, and the catalogue
//! of nodes there are to add.
//!
//! `material.set_graph` replaces a material's whole graph: five to ten
//! thousand characters to read and send back to change one number, and
//! nothing to say what a node type's inputs, outputs and settings are, so
//! a node of a new type could only be guessed at. This is the same edit a
//! node at a time, as a person makes it in the Material Graph:
//!
//! - `catalogue`: the nodes there are, searched, a page at a time; one by
//!   its `name`, with every input, output, setting and field it has.
//! - `read`: a material's graph in brief, or one `node` of it whole.
//! - `add`: a node from the catalogue, by its name.
//! - `set`: a node's settings, fields, input values and label.
//! - `connect`, `disconnect`: one wire.
//! - `remove`: a node and its wires.
//!
//! Each change answers with what it did and is one `material.set_graph`
//! command, as an edit in the editor is: one step to undo. A graph that
//! would not compile is refused, in words, and nothing is changed.

use crate::document::LibraryOutcome;
use tequila_core::math::{vec2, Vec2};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, SocketType, Value as GraphValue, LAYER_INPUT};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::nodes::{Setting, ShaderNode};
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::MaterialKey;
use tequila_scene::Command;
use tequila_texture::procedural::Procedural;

/// What a `graph` request can do.
pub const ACTIONS: &[&str] = &["catalogue", "read", "add", "set", "connect", "disconnect", "remove"];

/// How many of the catalogue, or of a graph's nodes, one answer lists.
pub const PAGE: usize = 25;

/// One kind of node there is to add.
pub struct Entry {
    /// What `add` takes, and what a node read back is called.
    pub name: String,
    pub label: String,
    /// Where it is in the editor's Add menu, in a word.
    pub family: &'static str,
    /// What it is for.
    pub what: String,
    /// Other words a search finds it by.
    pub keywords: String,
    /// The node as it is added.
    pub node: Node,
}

/// What each general node is for. Tequila's own words: a node's key says
/// what it is called, not when to reach for it.
fn node_purpose(node: &ShaderNode) -> &'static str {
    match node {
        ShaderNode::TextureCoordinate => "The point's coordinates to lay a texture by: generated, object, UV, normal and the others. Wire one into a texture's Vector, or into a Mapping first.",
        ShaderNode::Geometry => "What the surface knows at the point: its position, normal and tangent, whether it is the back face, how pointed the shape is there.",
        ShaderNode::LayerWeight => "How far the surface turns away from the camera: edges against faces, for a rim, a velvet sheen, a pearl.",
        ShaderNode::Fresnel => "How mirror-like a surface of this IOR is at this angle: low facing the camera, 1 at grazing edges.",
        ShaderNode::ObjectInfo => "What the part says of itself: its own colour and number (scene.set_values), a random number for each part, where it is. One material, different on every part.",
        ShaderNode::LightPath => "Which kind of ray reached the point - the camera's, a shadow's, a reflection's, one through glass - to show each something different.",
        ShaderNode::Noise { .. } => "Fractal noise: soft clouds, as a number and as a colour. Mottling, roughness variation, a base for most organic looks. Scale counts repeats per metre of the part.",
        ShaderNode::Voronoi { .. } => "Cells: the distance to the nearest cell point, a colour for each cell, the cell's place. Stone, hammered metal, scales, anything that differs cell by cell.",
        ShaderNode::Wave { .. } => "Bands or rings, bent by a distortion: wood rings, stripes, ripples.",
        ShaderNode::Magic { .. } => "Swirls of interfering colour. Rarely right for a product.",
        ShaderNode::Brick { .. } => "Bricks or tiles in courses with mortar between: their colour, and a mask of the mortar.",
        ShaderNode::Checker => "A chequerboard of two colours.",
        ShaderNode::GradientTexture { .. } => "A ramp across the coordinates - linear, radial, spherical and others - to put through a Color Ramp.",
        ShaderNode::WhiteNoise { .. } => "A random number and colour for every different value it is given, with nothing smooth about it. Give it a cell's colour or place for a random of its own for each cell.",
        ShaderNode::Gabor { .. } => "Streaked noise with a direction: brushed grain, fibres, fine scratches.",
        ShaderNode::Mix { .. } => "Two colours - or numbers, or vectors - mixed by a factor, with the blend modes: multiply, overlay, screen and the rest.",
        ShaderNode::HueSaturationValue => "A colour's hue turned, its saturation and value scaled.",
        ShaderNode::BrightContrast => "A colour's brightness and contrast.",
        ShaderNode::Gamma => "A colour raised to a power: darker or lighter mid-tones.",
        ShaderNode::InvertColor => "A colour inverted, by a factor.",
        ShaderNode::Blackbody => "The colour of something glowing at a temperature in kelvin: 2700 a warm bulb, 6500 daylight.",
        ShaderNode::Math { .. } => "One sum on numbers - add, multiply, power, sine, greater than and many more: `operation` says which.",
        ShaderNode::VectorMath { .. } => "One sum on vectors - add, scale, cross, dot, length, normalize and more: `operation` says which.",
        ShaderNode::MapRange { .. } => "A number fitted from one range into another: a pattern's 0 to 1 made a roughness of 0.2 to 0.4.",
        ShaderNode::Clamp { .. } => "A number held between a least and a most.",
        ShaderNode::SeparateXyz => "A vector as its three numbers.",
        ShaderNode::CombineXyz => "Three numbers as a vector.",
        ShaderNode::SeparateColor { .. } => "A colour as its three numbers: red, green, blue, or hue, saturation, value.",
        ShaderNode::CombineColor { .. } => "Three numbers as a colour.",
        ShaderNode::Mapping { .. } => "Coordinates moved, turned and scaled before a texture reads them.",
    }
}

/// The catalogue: every node an assistant may add, in the Add menu's order.
pub fn catalogue() -> Vec<Entry> {
    let mut out = Vec::new();
    let mut plain = |name: &str, label: &str, family: &'static str, what: &str, keywords: &str, kind: NodeKind| {
        out.push(Entry { name: name.into(), label: label.into(), family, what: what.into(), keywords: keywords.into(), node: Node::new(kind) });
    };
    plain("value", "Value", "input", "A number, to feed several inputs from one place.", "constant number float", NodeKind::Constant(GraphValue::Float(0.5)));
    plain("rgb", "RGB", "input", "A colour, to feed several inputs from one place.", "constant colour color", NodeKind::Constant(GraphValue::Color(tequila_core::spectrum::Spectrum::splat(0.8))));
    plain("vertex_color", "Vertex Color", "input", "The colours painted on the model's own vertices, when the file brought any.", "vertex colour paint", NodeKind::VertexColor);
    plain(
        "mapping_2d",
        "Projection",
        "input",
        "How a picture is laid on the part: by its UVs or projected (planar, box, tri-planar, cylindrical, spherical), with its size, offset and turn. Wire it into an Image Texture's Coordinates.",
        "projection mapping uv triplanar box planar tiling",
        NodeKind::Mapping2d { mapping: Default::default() },
    );
    plain(
        "image_texture",
        "Image Texture",
        "texture",
        "A picture that is already in the project, by its texture id (tequila_read textures lists them). To bring a picture file in and wire it, material.set_texture does both.",
        "picture bitmap map image",
        NodeKind::ImageTexture { texture: tequila_materials::graph::TextureId(0), fallback: GraphValue::Color(tequila_core::spectrum::Spectrum::splat(0.18)) },
    );
    plain(
        "image_alpha",
        "Image Alpha",
        "texture",
        "A picture's transparency as a number: a cut-out mask for geometry_opacity.",
        "picture alpha mask cutout opacity",
        NodeKind::ImageAlpha { texture: tequila_materials::graph::TextureId(0), fallback: GraphValue::Float(1.0) },
    );
    plain(
        "gradient",
        "Color Ramp",
        "color",
        "A colour ramp: a number from 0 to 1 made colours through its stops. Put it after a pattern to colour it, or to sharpen a mask.",
        "ramp colour gradient stops",
        NodeKind::Gradient { gradient: Box::default() },
    );
    plain(
        "bump",
        "Bump",
        "vector",
        "A height (a number) made a bump, `depth` in metres. Wire it into the surface's bump.",
        "height relief normal emboss",
        NodeKind::Bump { depth: tequila_materials::graph::DEFAULT_BUMP_DEPTH },
    );
    plain(
        "curvature",
        "Curvature",
        "input",
        "The model's own edges and creases as a mask: convex (worn edges), concave (dirt in creases) or both, within a radius in metres.",
        "edges wear worn creases cavity VRayCurvature",
        NodeKind::Curvature { mode: tequila_materials::graph::CurvatureMode::Convex, radius: tequila_materials::graph::DEFAULT_CURVATURE_RADIUS, contrast: 1.0 },
    );
    plain(
        "occlusion",
        "Occlusion",
        "input",
        "How enclosed the point is: dark in crevices, white in the open, within a radius in metres. A mask for dirt and dust.",
        "ambient occlusion dirt crevice VRayDirt",
        NodeKind::Occlusion { radius: tequila_materials::graph::DEFAULT_OCCLUSION_RADIUS, contrast: 1.0 },
    );
    plain(
        "distance",
        "Distance",
        "input",
        "How close the point is to another part, within a radius in metres: dirt, wear or wetness where parts meet.",
        "proximity contact VRayDistanceTex",
        NodeKind::Distance { radius: tequila_materials::graph::DEFAULT_DISTANCE_RADIUS },
    );
    plain(
        "shader",
        "Shader",
        "shader",
        "Another whole surface with parameters of its own, to lay over the material's with a Mix Shader: lacquer over wood, dust over paint.",
        "layer coat second material blend",
        NodeKind::Shader(Box::new(MaterialType::Plastic.defaults())),
    );
    plain(
        "mix_shader",
        "Mix Shader",
        "shader",
        "Mixes the Shader wired into it over the surface whose Layer it feeds, by Fac: a number, or a mask from a pattern.",
        "blend layer mask VRayBlendMtl",
        NodeKind::MixShader { add: false },
    );
    plain("add_shader", "Add Shader", "shader", "Adds the Shader wired into it on top of the surface whose Layer it feeds.", "layer additive glow", NodeKind::MixShader { add: true });
    plain(
        "bubbles",
        "Bubbles",
        "shader",
        "Bubbles inside a clear material: how many, their radius in metres and how much it varies.",
        "glass resin liquid air",
        NodeKind::Bubbles { count: 200, radius: 0.002, variation: 0.5, seed: 0 },
    );
    plain(
        "fuzz",
        "Fuzz",
        "shader",
        "Real fibres standing on the surface: their length and thickness in metres, how many to the square centimetre, their lean and curl.",
        "fur hair fibres velvet felt carpet",
        NodeKind::Fuzz { length: 0.002, density: 400.0, thickness: 0.00002, lean: 0.35, curl: 0.3, seed: 0 },
    );
    plain("reroute", "Reroute", "layout", "A dot that passes a wire on, to tidy a graph.", "dot wire", NodeKind::Reroute { ty: SocketType::Color });
    // The simple nodes a recipe builds a graph from. The general ones of
    // the same name (mix, clamp, invert) do more.
    plain("remap", "Remap", "simple", "A number's range fitted into another: `from` [low, high] to `to` [low, high]. What a recipe's `range` builds.", "range fit", NodeKind::Remap { from: vec2(0.0, 1.0), to: vec2(0.0, 1.0) });
    plain("channel", "Channel", "simple", "One of a colour's three numbers: r, g or b.", "red green blue separate", NodeKind::Channel { channel: Default::default() });
    plain("multiply", "Multiply", "simple", "Two colours multiplied. What a recipe's `blend: multiply` builds.", "blend darken", NodeKind::Multiply);
    plain("add", "Add", "simple", "Two colours added. What a recipe's `blend: add` builds.", "blend lighten", NodeKind::Add);
    plain("mix_simple", "Mix (simple)", "simple", "Two colours mixed by a factor. What a recipe's layers build; `mix` has the blend modes.", "blend lerp", NodeKind::Mix);
    plain("clamp_simple", "Clamp (simple)", "simple", "A value held between `low` and `high`. `clamp` takes them as inputs.", "limit", NodeKind::Clamp { low: 0.0, high: 1.0 });
    plain("invert_simple", "Invert (simple)", "simple", "A value inverted: white for black. `invert` does it by a factor.", "negative", NodeKind::Invert);

    for node in ShaderNode::catalog() {
        let family = match node.category() {
            tequila_materials::nodes::NodeCategory::Input => "input",
            tequila_materials::nodes::NodeCategory::Texture => "texture",
            tequila_materials::nodes::NodeCategory::Color => "color",
            tequila_materials::nodes::NodeCategory::Converter => "converter",
            tequila_materials::nodes::NodeCategory::Vector => "vector",
        };
        out.push(Entry {
            name: node.key().into(),
            label: node.menu_label().into(),
            family,
            what: node_purpose(&node).into(),
            keywords: node.keywords().into(),
            node: Node::new(NodeKind::Node(node)),
        });
    }
    // The patterns `material.set_procedural` drives a parameter with, each
    // a node: what the material guide says of them.
    let guide = crate::material::guide();
    for procedural in Procedural::catalog() {
        let described = guide.get("patterns").map(Value::as_array).unwrap_or(&[]).iter().find(|p| p.get("kind").and_then(Value::as_str) == Some(procedural.key()));
        let say = |key: &str| described.and_then(|p| p.get(key)).and_then(Value::as_str).unwrap_or("").to_string();
        out.push(Entry {
            name: format!("pattern_{}", procedural.key()),
            label: procedural.label().into(),
            family: "pattern",
            what: format!("{} {} Sized by `frequency`: repeats per metre.", say("looks_like"), say("good_for")).trim().to_string(),
            keywords: "procedural pattern".into(),
            node: Node::new(NodeKind::Procedural { procedural: Box::new(procedural), seed: 0 }),
        });
    }
    for group in NodeGroup::ALL {
        out.push(Entry {
            name: group.key().into(),
            label: group.label().into(),
            family: "group",
            what: group.description().into(),
            keywords: group.keywords().into(),
            node: Node::new(NodeKind::Group(*group)),
        });
    }
    out
}

/// What a node of this kind is called in the catalogue.
pub fn name_of(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Surface(_) => "surface".into(),
        NodeKind::Shader(_) => "shader".into(),
        NodeKind::MixShader { add: false } => "mix_shader".into(),
        NodeKind::MixShader { add: true } => "add_shader".into(),
        NodeKind::Constant(GraphValue::Float(_) | GraphValue::Bool(_)) => "value".into(),
        NodeKind::Constant(_) => "rgb".into(),
        NodeKind::Mapping2d { .. } => "mapping_2d".into(),
        NodeKind::ImageTexture { .. } => "image_texture".into(),
        NodeKind::ImageAlpha { .. } => "image_alpha".into(),
        NodeKind::Gradient { .. } => "gradient".into(),
        NodeKind::Procedural { procedural, .. } => format!("pattern_{}", procedural.key()),
        NodeKind::Mix => "mix_simple".into(),
        NodeKind::Multiply => "multiply".into(),
        NodeKind::Add => "add".into(),
        NodeKind::Invert => "invert_simple".into(),
        NodeKind::Channel { .. } => "channel".into(),
        NodeKind::Remap { .. } => "remap".into(),
        NodeKind::Clamp { .. } => "clamp_simple".into(),
        NodeKind::Bump { .. } => "bump".into(),
        NodeKind::Curvature { .. } => "curvature".into(),
        NodeKind::Occlusion { .. } => "occlusion".into(),
        NodeKind::Distance { .. } => "distance".into(),
        NodeKind::VertexColor => "vertex_color".into(),
        NodeKind::VolumeMap { .. } => "volume_map".into(),
        NodeKind::Bubbles { .. } => "bubbles".into(),
        NodeKind::Fuzz { .. } => "fuzz".into(),
        NodeKind::Reroute { .. } => "reroute".into(),
        NodeKind::Node(node) => node.key().into(),
        NodeKind::Group(group) => group.key().into(),
    }
}

fn type_name(ty: SocketType) -> &'static str {
    match ty {
        SocketType::Float => "number",
        SocketType::Color => "colour",
        SocketType::Vector => "vector",
        SocketType::Normal => "normal",
        SocketType::Bool => "flag",
        SocketType::Shader => "shader",
    }
}

/// A graph value as a script writes one: a number, three numbers, a flag.
fn plain_value(value: GraphValue) -> Value {
    match value {
        GraphValue::Float(f) => Value::from(f),
        GraphValue::Color(c) => Value::from(vec![c.r, c.g, c.b]),
        GraphValue::Vector(v) => Value::from(vec![v.x, v.y, v.z]),
        GraphValue::Bool(b) => Value::Bool(b),
    }
}

/// What a script wrote, as the value a socket of this type takes: a
/// number, three numbers, a hex colour, a flag.
fn value_for(ty: SocketType, written: &Value, input: &str) -> Result<GraphValue, String> {
    let wrong = || format!("`{input}` takes a {}: {}", type_name(ty), match ty {
        SocketType::Float => "a number",
        SocketType::Color => "\"#rrggbb\" or [r, g, b]",
        SocketType::Vector | SocketType::Normal => "[x, y, z]",
        SocketType::Bool => "true or false",
        SocketType::Shader => "a wire, not a value",
    });
    let three = |v: &Value| v.as_f32_array().filter(|a| a.len() == 3 && a.iter().all(|n| n.is_finite()));
    match ty {
        SocketType::Float => written.as_f32().filter(|n| n.is_finite()).map(GraphValue::Float).ok_or_else(wrong),
        SocketType::Color => match written {
            Value::String(text) => tequila_format::hex_colour(text).map(GraphValue::Color).ok_or_else(wrong),
            Value::Number(_) => written.as_f32().map(|n| GraphValue::Color(tequila_core::spectrum::Spectrum::splat(n))).ok_or_else(wrong),
            other => three(other).map(|c| GraphValue::Color(tequila_core::spectrum::Spectrum { r: c[0], g: c[1], b: c[2] })).ok_or_else(wrong),
        },
        SocketType::Vector | SocketType::Normal => match written {
            Value::Number(_) => written.as_f32().map(|n| GraphValue::Vector(tequila_core::math::vec3(n, n, n))).ok_or_else(wrong),
            other => three(other).map(|v| GraphValue::Vector(tequila_core::math::vec3(v[0], v[1], v[2]))).ok_or_else(wrong),
        },
        SocketType::Bool => written.as_bool().map(GraphValue::Bool).ok_or_else(wrong),
        SocketType::Shader => Err(wrong()),
    }
}

/// A general node's settings, each with its value and what it takes.
fn settings_of(node: &ShaderNode) -> Value {
    let mut out = Object::new();
    for spec in node.settings() {
        let mut entry = Object::new();
        match spec.value {
            Setting::Choice(v) => {
                entry.set("value", v);
                entry.set("options", Value::Array(spec.options.iter().map(|(key, _)| Value::String((*key).into())).collect()));
            }
            Setting::Int(v) => {
                entry.set("value", v as f64).set("takes", "a whole number");
            }
            Setting::Float(v) => {
                entry.set("value", v).set("takes", "a number");
            }
            Setting::Bool(v) => {
                entry.set("value", v);
            }
        };
        out.set(spec.key, entry.build());
    }
    out.build()
}

/// The fields of a node that are its own kind's: what the file writes of it
/// beyond what every node has.
fn fields_of(node: &Node) -> Value {
    let Value::Object(mut fields) = tequila_format::graph_node_to_json(NodeId(0), node) else {
        return Value::Null;
    };
    for key in ["id", "kind", "type", "group", "settings", "literals", "position", "label", "muted", "collapsed"] {
        // A reroute's `type` is its own field; a general node's names it.
        if key == "type" && matches!(node.kind, NodeKind::Reroute { .. }) {
            continue;
        }
        fields.remove(key);
    }
    Value::Object(fields)
}

/// Where a wire comes from or goes to, in a few characters: `7.Distance`.
fn end_name(graph: &MaterialGraph, node: NodeId, name: &str) -> String {
    if node == graph.output() {
        format!("surface.{name}")
    } else {
        format!("{}.{name}", node.0)
    }
}

fn output_name(kind: &NodeKind, index: u8) -> String {
    kind.outputs().get(index as usize).map(|(name, _)| name.to_string()).unwrap_or_else(|| index.to_string())
}

/// An input's name as a wire's end says it: a surface's parameter by its
/// key, any other input by the name the editor shows.
fn input_name(kind: &NodeKind, socket: Socket) -> String {
    match socket {
        Socket::Param(param) => param.key().to_string(),
        other => kind.inputs().iter().find(|(s, _, _)| *s == other).map(|(_, _, name)| name.to_string()).unwrap_or_else(|| tequila_format::graph_socket_name(other)),
    }
}

/// One node whole: what it is, its settings and fields, each input with
/// its value or the wire into it, each output with where it goes. `graph`
/// is `None` for a catalogue entry, which is wired to nothing.
fn describe(id: Option<NodeId>, node: &Node, graph: Option<&MaterialGraph>) -> Value {
    let mut out = Object::new();
    if let Some(id) = id {
        out.set("id", id.0 as f64);
    }
    out.set("node", name_of(&node.kind)).set("label", node.label.clone().unwrap_or_else(|| node.kind.label().to_string()));
    if node.muted {
        out.set("muted", true);
    }
    if let NodeKind::Node(shader) = &node.kind {
        if !shader.settings().is_empty() {
            out.set("settings", settings_of(shader));
        }
    }
    let surface = matches!(node.kind, NodeKind::Surface(_) | NodeKind::Shader(_));
    if !matches!(node.kind, NodeKind::Surface(_)) {
        if let Value::Object(fields) = fields_of(node) {
            if !fields.is_empty() {
                out.set("fields", Value::Object(fields));
            }
        }
    }
    let mut inputs = Vec::new();
    for (index, (socket, ty, _)) in node.kind.inputs().into_iter().enumerate() {
        let wire = graph.zip(id).and_then(|(graph, id)| graph.links().iter().find(|link| link.to == id && link.to_socket == socket).copied());
        // A surface has every parameter as an input: only the wired ones,
        // and its Layer, say anything a parameter's own value does not.
        if surface && wire.is_none() && socket != LAYER_INPUT {
            continue;
        }
        if !surface && wire.is_none() && !node.literals.contains_key(&socket) && !node.kind.shows_input(index) {
            continue;
        }
        let mut entry = Object::new();
        entry.set("input", input_name(&node.kind, socket)).set("socket", tequila_format::graph_socket_name(socket)).set("type", type_name(ty));
        match (wire, graph) {
            (Some(link), Some(graph)) => {
                let from = graph.node(link.from).map(|n| output_name(&n.kind, link.from_socket)).unwrap_or_default();
                entry.set("from", end_name(graph, link.from, &from));
            }
            _ if ty == SocketType::Shader => {}
            _ => {
                let value = node.literals.get(&socket).copied().or_else(|| node.kind.input_default(index));
                match (value, node.kind.implicit_input(index)) {
                    (Some(value), _) if node.literals.contains_key(&socket) => {
                        entry.set("value", plain_value(value));
                    }
                    (_, Some(_)) => {
                        entry.set("value", "the surface's own, unless wired");
                    }
                    (Some(value), None) => {
                        entry.set("value", plain_value(value));
                    }
                    (None, None) => {}
                }
            }
        }
        if let NodeKind::Node(shader) = &node.kind {
            if let Some((low, high)) = shader.inputs().get(index).and_then(|spec| spec.range) {
                entry.set("range", Value::from(vec![low, high]));
            }
        }
        if let NodeKind::Group(group) = &node.kind {
            if let Some(input) = group.inputs().get(index) {
                entry.set("range", Value::from(vec![input.min, input.max]));
                if !input.tip.is_empty() {
                    entry.set("tip", input.tip);
                }
            }
        }
        inputs.push(entry.build());
    }
    if surface && graph.is_some() {
        out.set("inputs_note", "Only the wired inputs are listed. Every parameter of the material is an input, by its key (base_color, specular_roughness, bump...), and Layer takes a Mix Shader.");
    }
    out.set("inputs", Value::Array(inputs));
    let outputs: Vec<Value> = node
        .kind
        .outputs()
        .into_iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            let mut entry = Object::new();
            entry.set("output", name).set("type", type_name(ty));
            if let (Some(graph), Some(id)) = (graph, id) {
                let to: Vec<Value> = graph
                    .links()
                    .iter()
                    .filter(|link| link.from == id && link.from_socket as usize == index)
                    .filter_map(|link| graph.node(link.to).map(|target| Value::String(end_name(graph, link.to, &input_name(&target.kind, link.to_socket)))))
                    .collect();
                if !to.is_empty() {
                    entry.set("to", Value::Array(to));
                }
            }
            if let NodeKind::Group(group) = &node.kind {
                if let Some(tip) = group.outputs().get(index).map(|o| o.tip).filter(|tip| !tip.is_empty()) {
                    entry.set("tip", tip);
                }
            }
            entry.build()
        })
        .collect();
    out.set("outputs", Value::Array(outputs));
    out.build()
}

/// The first sentence of a description, for a list.
fn brief(text: &str) -> String {
    let end = text.find(". ").map(|at| at + 1).unwrap_or(text.len());
    let first = &text[..end];
    if first.chars().count() > 140 {
        format!("{}...", first.chars().take(137).collect::<String>().trim_end())
    } else {
        first.to_string()
    }
}

fn page_of(request: &Value) -> Result<usize, String> {
    match request.get("page") {
        None | Some(Value::Null) => Ok(1),
        Some(page) => page.as_index().filter(|page| *page >= 1).ok_or_else(|| "`page` is a whole number, from 1".to_string()),
    }
}

/// The catalogue, as a `catalogue` request asks for it: one node by its
/// `name`; or those matching every word of `search` and in `family`, a
/// page at a time - three or fewer in full, more as a line each.
pub fn catalogue_answer(request: &Value) -> Result<Value, String> {
    let entries = catalogue();
    let text = |key: &str| request.get(key).and_then(Value::as_str).map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty());
    let whole = |entry: &Entry| {
        let mut out = Object::new();
        out.set("name", entry.name.clone()).set("family", entry.family).set("what", entry.what.clone());
        if let Value::Object(fields) = describe(None, &entry.node, None) {
            for (key, value) in fields {
                if key != "node" {
                    out.set(&key, value);
                }
            }
        }
        // As the editor's Add menu calls it.
        out.set("label", entry.label.clone());
        out.build()
    };
    if let Some(name) = text("name") {
        let entry = entries.iter().find(|entry| entry.name == name).ok_or_else(|| {
            let near: Vec<&str> = entries.iter().filter(|entry| entry.name.contains(&name) || name.contains(&entry.name)).map(|entry| entry.name.as_str()).take(6).collect();
            match near.is_empty() {
                true => format!("there is no node called `{name}`; search the catalogue for it in words"),
                false => format!("there is no node called `{name}`; nearest: {}", near.join(", ")),
            }
        })?;
        return Ok(whole(entry));
    }
    let families: Vec<&str> = {
        let mut seen = Vec::new();
        for entry in &entries {
            if !seen.contains(&entry.family) {
                seen.push(entry.family);
            }
        }
        seen
    };
    let family = text("family");
    if let Some(family) = &family {
        if !families.contains(&family.as_str()) {
            return Err(format!("`{family}` is not a family of nodes; the families are {}", families.join(", ")));
        }
    }
    let words: Vec<String> = text("search").map(|search| search.split_whitespace().map(str::to_string).collect()).unwrap_or_default();
    let found: Vec<&Entry> = entries
        .iter()
        .filter(|entry| family.as_deref().is_none_or(|family| entry.family == family))
        .filter(|entry| {
            let hay = format!("{} {} {} {} {}", entry.name, entry.label, entry.family, entry.keywords, entry.what).to_lowercase();
            words.iter().all(|word| hay.contains(word.as_str()))
        })
        .collect();
    let page = page_of(request)?;
    let pages = found.len().div_ceil(PAGE).max(1);
    if page > pages {
        return Err(format!("there are {pages} page(s) of these"));
    }
    let mut out = Object::new();
    out.set("count", found.len());
    if found.is_empty() {
        out.set("note", format!("Nothing matches. Search with fewer words, or by family: {}.", families.join(", ")));
    } else if found.len() <= 3 {
        out.set("nodes", Value::Array(found.iter().map(|entry| whole(entry)).collect()));
    } else {
        let shown: Vec<Value> = found
            .iter()
            .skip((page - 1) * PAGE)
            .take(PAGE)
            .map(|entry| {
                let mut line = Object::new();
                line.set("name", entry.name.clone()).set("family", entry.family).set("what", brief(&entry.what));
                line.build()
            })
            .collect();
        out.set("page", page).set("pages", pages).set("nodes", Value::Array(shown));
        out.set("note", "A line each. Ask for one by `name` for its inputs, outputs, settings and fields before adding it.");
    }
    if words.is_empty() && family.is_none() {
        out.set("families", Value::Array(families.iter().map(|f| Value::String((*f).into())).collect()));
    }
    Ok(out.build())
}

fn material_of<'d>(document: &'d SceneDocument, request: &Value) -> Result<(MaterialKey, &'d tequila_scene::material::MaterialDefinition), String> {
    let id = request.get("material").and_then(Value::as_index).ok_or("say which material: its id, as `material` (tequila_materials lists them)")?;
    let key = MaterialKey(id as u64);
    let definition = document.materials.get(key).ok_or_else(|| format!("there is no material {id}; tequila_materials lists them"))?;
    Ok((key, definition))
}

/// A node of the graph, as a request names one: its id, or `surface`.
fn node_named(graph: &MaterialGraph, request: &Value, key: &str) -> Result<NodeId, String> {
    let named = request.get(key).ok_or_else(|| format!("say which node: its id, as `{key}`"))?;
    if named.as_str().is_some_and(|text| text.trim().eq_ignore_ascii_case("surface")) {
        return Ok(graph.output());
    }
    let id = NodeId(named.as_index().ok_or_else(|| format!("`{key}` is a node's id, a whole number, or \"surface\""))? as u32);
    if graph.node(id).is_none() {
        let ids: Vec<String> = graph.nodes().map(|(id, _)| id.0.to_string()).collect();
        return Err(format!("there is no node {} in this graph; its nodes are {} ({} is the surface)", id.0, ids.join(", "), graph.output().0));
    }
    Ok(id)
}

/// The input of a node a request names: by the name the editor shows, by
/// a parameter's key on a surface, or as the file writes it (`input:2`).
fn input_named(node: &Node, name: &str) -> Result<(usize, Socket, SocketType), String> {
    let inputs = node.kind.inputs();
    let wanted = name.trim();
    let position = |socket: Socket| inputs.iter().position(|(s, _, _)| *s == socket);
    if let Some(socket) = tequila_format::graph_socket_from_name(wanted) {
        if let Some(index) = position(socket) {
            return Ok((index, socket, inputs[index].1));
        }
    }
    if matches!(node.kind, NodeKind::Surface(_) | NodeKind::Shader(_)) {
        if let Some(index) = Param::from_key(&wanted.to_lowercase()).and_then(|param| position(Socket::Param(param))) {
            return Ok((index, inputs[index].0, inputs[index].1));
        }
    }
    let matching: Vec<usize> = inputs.iter().enumerate().filter(|(_, (_, _, n))| n.eq_ignore_ascii_case(wanted)).map(|(index, _)| index).collect();
    let shown: Vec<usize> = matching.iter().copied().filter(|index| node.kind.shows_input(*index)).collect();
    match (shown.as_slice(), matching.as_slice()) {
        ([index], _) | ([], [index]) => Ok((*index, inputs[*index].0, inputs[*index].1)),
        ([], []) => {
            let known: Vec<String> = match node.kind {
                NodeKind::Surface(_) | NodeKind::Shader(_) => vec!["a parameter's key (base_color, specular_roughness, bump...)".into(), "Layer".into()],
                _ => inputs.iter().enumerate().filter(|(index, _)| node.kind.shows_input(*index)).map(|(_, (_, _, n))| n.to_string()).collect(),
            };
            Err(match known.is_empty() {
                true => format!("a {} has no inputs", node.kind.label()),
                false => format!("a {} has no input `{wanted}`; its inputs are {}", node.kind.label(), known.join(", ")),
            })
        }
        _ => {
            let sockets: Vec<String> = matching.iter().map(|index| tequila_format::graph_socket_name(inputs[*index].0)).collect();
            Err(format!("a {} has several inputs called `{wanted}`; name one as the file does: {}", node.kind.label(), sockets.join(", ")))
        }
    }
}

/// The output of a node a request names: by its name, or its number.
fn output_named(node: &Node, named: Option<&Value>) -> Result<u8, String> {
    let outputs = node.kind.outputs();
    if outputs.is_empty() {
        return Err(format!("a {} has no outputs", node.kind.label()));
    }
    let names = || outputs.iter().map(|(name, _)| name.to_string()).collect::<Vec<_>>().join(", ");
    match named {
        None | Some(Value::Null) => Ok(0),
        Some(Value::String(name)) => outputs
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
            .map(|index| index as u8)
            .ok_or_else(|| format!("a {} has no output `{name}`; its outputs are {}", node.kind.label(), names())),
        Some(other) => other.as_index().filter(|index| *index < outputs.len()).map(|index| index as u8).ok_or_else(|| format!("a {} has outputs {}", node.kind.label(), names())),
    }
}

/// `over` laid on `under`: objects merged key by key, anything else
/// replaced.
fn merged(under: &Value, over: &Value) -> Value {
    match (under, over) {
        (Value::Object(under), Value::Object(over)) => {
            let mut out = under.clone();
            for (key, value) in over {
                let laid = match out.get(key) {
                    Some(existing) => merged(existing, value),
                    None => value.clone(),
                };
                out.insert(key.clone(), laid);
            }
            Value::Object(out)
        }
        _ => over.clone(),
    }
}

/// A constant's value as the file writes one, from a number, a hex colour
/// or three numbers: three numbers stay what the node already held.
fn written_value(current: Option<&Value>, given: &Value) -> Value {
    let wrap = |key: &str, value: Value| {
        let mut out = Object::new();
        out.set(key, value);
        out.build()
    };
    match given {
        Value::Number(_) => wrap("float", given.clone()),
        Value::String(_) => wrap("color", given.clone()),
        Value::Bool(_) => wrap("bool", given.clone()),
        Value::Array(_) if current.is_some_and(|c| c.get("vector").is_some()) => wrap("vector", given.clone()),
        Value::Array(_) => wrap("color", given.clone()),
        other => other.clone(),
    }
}

/// Lay a request's `settings`, `fields`, `inputs`, `label` and `muted` on a
/// node, each checked against what the node has.
fn change(node: &Node, request: &Value) -> Result<Node, String> {
    let label = node.kind.label();
    let mut written = tequila_format::graph_node_to_json(NodeId(0), node);
    if let Some(fields) = request.get("fields") {
        let Value::Object(fields) = fields else {
            return Err("`fields` is an object of the node's own fields".into());
        };
        let own = fields_of(node);
        for (key, value) in fields {
            // A ramp's interpolation is written only when it is not linear.
            let known = own.get(key).is_some() || (key == "interpolation" && matches!(node.kind, NodeKind::Gradient { .. }));
            if !known {
                let Value::Object(own) = &own else { unreachable!() };
                let names: Vec<&str> = own.keys().map(String::as_str).collect();
                return Err(match names.is_empty() {
                    true => format!("a {label} has no fields of its own; `{key}` is not read"),
                    false => format!("a {label} has no field `{key}`; its fields are {}", names.join(", ")),
                });
            }
            let value = if matches!(key.as_str(), "value" | "fallback") { written_value(own.get(key), value) } else { value.clone() };
            let laid = merged(written.get(key).unwrap_or(&Value::Null), &value);
            if let Value::Object(map) = &mut written {
                map.insert(key.clone(), laid);
            }
        }
    }
    if let Some(settings) = request.get("settings") {
        let (Value::Object(settings), NodeKind::Node(shader)) = (settings, &node.kind) else {
            return Err(match node.kind {
                NodeKind::Node(_) => "`settings` is an object of the node's settings".into(),
                _ => format!("a {label} has no settings; what it has to set are its `fields` and `inputs`"),
            });
        };
        // Every setting the node can have, whatever its others show now.
        let specs: Vec<_> = ShaderNode::from_key(shader.key()).map(|fresh| fresh.settings()).unwrap_or_default().into_iter().chain(shader.settings()).collect();
        for (key, value) in settings {
            let Some(spec) = specs.iter().find(|spec| spec.key == key) else {
                let names: Vec<&str> = shader.settings().iter().map(|spec| spec.key).collect();
                return Err(match names.is_empty() {
                    true => format!("a {label} has no settings"),
                    false => format!("a {label} has no setting `{key}`; its settings are {}", names.join(", ")),
                });
            };
            let fits = match spec.value {
                Setting::Choice(_) => value.as_str().is_some_and(|v| spec.options.iter().any(|(k, _)| *k == v)),
                Setting::Int(_) | Setting::Float(_) => value.as_f32().is_some_and(f32::is_finite),
                Setting::Bool(_) => value.as_bool().is_some(),
            };
            if !fits {
                return Err(match spec.value {
                    Setting::Choice(_) => format!("`{key}` is one of {}", spec.options.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")),
                    Setting::Bool(_) => format!("`{key}` is true or false"),
                    _ => format!("`{key}` is a number"),
                });
            }
        }
        let laid = merged(written.get("settings").unwrap_or(&Value::Null), &Value::Object(settings.clone()));
        if let Value::Object(map) = &mut written {
            map.insert("settings".into(), laid);
        }
    }
    let mut changed = tequila_format::graph_node_from_json(&written).map_err(|e| e.to_string())?;
    // A Mix told to mix numbers and, in the same breath, how to blend
    // colours: the reader sets each setting on the node as it then is.
    if let (NodeKind::Node(now), Some(Value::Object(settings))) = (&changed.kind, request.get("settings")) {
        for spec in now.settings() {
            let Some(asked) = settings.get(spec.key) else { continue };
            let took = match spec.value {
                Setting::Choice(v) => asked.as_str() == Some(v),
                Setting::Bool(v) => asked.as_bool() == Some(v),
                // A number is held to the node's own range.
                Setting::Int(_) | Setting::Float(_) => true,
            };
            if !took {
                return Err(format!("`{}` could not be set on this {label}", spec.key));
            }
        }
    }
    if let Some(inputs) = request.get("inputs") {
        let Value::Object(inputs) = inputs else {
            return Err("`inputs` is an object: an input's name and the value to give it".into());
        };
        if matches!(changed.kind, NodeKind::Surface(_)) {
            return Err("the surface's own values are the material's parameters: material.set_parameter".into());
        }
        if matches!(changed.kind, NodeKind::Shader(_)) {
            return Err("a Shader's values are its `parameters`, among its `fields`: {\"fields\": {\"parameters\": {\"base_color\": \"#202020\"}}}".into());
        }
        for (name, value) in inputs {
            let (index, socket, ty) = input_named(&changed, name)?;
            if matches!(value, Value::Null) {
                changed.literals.remove(&socket);
                continue;
            }
            let value = value_for(ty, value, name)?;
            // The value it has without being told is no change to write.
            if !changed.literals.contains_key(&socket) && changed.kind.input_default(index) == Some(value) {
                continue;
            }
            changed.literals.insert(socket, value);
        }
    }
    match request.get("label") {
        None => {}
        Some(Value::Null) => changed.label = None,
        Some(Value::String(text)) => changed.label = Some(text.trim().to_string()).filter(|t| !t.is_empty()),
        Some(_) => return Err("`label` is the node's name, in words".into()),
    }
    if let Some(muted) = request.get("muted") {
        changed.muted = muted.as_bool().ok_or("`muted` is true or false")?;
    }
    Ok(changed)
}

/// Whether nobody has laid the graph out: two nodes at one place, as a
/// recipe leaves every node. The editor arranges such a graph itself, and
/// goes on doing so while it stays that way.
fn unplaced(graph: &MaterialGraph) -> bool {
    let mut places = std::collections::HashSet::new();
    graph.len() > 1 && graph.nodes().any(|(_, node)| !places.insert((node.position.x.to_bits(), node.position.y.to_bits())))
}

/// How far apart nodes are set, in the editor's own units: a node and the
/// gap for its wires across, a tall node and a gap down.
const ACROSS: f32 = 260.0;
const DOWN: f32 = 180.0;

/// A place at or below `at` that no node but `moving` is on or beside.
fn free(graph: &MaterialGraph, moving: Option<NodeId>, mut at: Vec2) -> Vec2 {
    while graph.nodes().any(|(id, node)| Some(id) != moving && (node.position.x - at.x).abs() < ACROSS * 0.9 && (node.position.y - at.y).abs() < DOWN * 0.9) {
        at.y += DOWN;
    }
    at
}

/// Somewhere for a new node that no node is at. A graph nobody has laid
/// out stays that way, and the editor arranges it; in one that has been,
/// the node goes to the left of the rest, level with the surface.
fn place(graph: &MaterialGraph) -> Vec2 {
    let surface = graph.node(graph.output()).map(|n| n.position).unwrap_or(Vec2::ZERO);
    if unplaced(graph) {
        return surface;
    }
    let left = graph.nodes().map(|(_, node)| node.position.x).fold(f32::INFINITY, f32::min);
    free(graph, None, vec2(if left.is_finite() { left - ACROSS } else { -ACROSS }, surface.y))
}

/// After a wire is run into `node`: whatever feeds it stands to its left,
/// and what feeds those to theirs, so the graph reads from left to right
/// as one laid out by hand does. Only a node whose wire would run
/// backwards is moved.
fn keep_left(graph: &mut MaterialGraph, node: NodeId, depth: usize) {
    if depth > 32 || unplaced(graph) {
        return;
    }
    let Some(at) = graph.node(node).map(|n| n.position) else { return };
    let sources: Vec<NodeId> = graph.links().iter().filter(|link| link.to == node).map(|link| link.from).collect();
    for source in sources {
        let Some(position) = graph.node(source).map(|n| n.position) else { continue };
        if position.x > at.x - ACROSS * 0.9 {
            let moved = free(graph, Some(source), vec2(at.x - ACROSS, at.y));
            if let Some(found) = graph.node_mut(source) {
                found.position = moved;
            }
            keep_left(graph, source, depth + 1);
        }
    }
}

/// The whole graph in brief: every node on a line, with where it leads.
fn brief_graph(key: MaterialKey, definition: &tequila_scene::material::MaterialDefinition, request: &Value) -> Result<Value, String> {
    let graph = &definition.graph;
    let page = page_of(request)?;
    let nodes: Vec<(NodeId, &Node)> = graph.nodes().filter(|(id, _)| *id != graph.output()).collect();
    let pages = nodes.len().div_ceil(PAGE).max(1);
    if page > pages {
        return Err(format!("this graph has {pages} page(s) of nodes"));
    }
    let lines: Vec<Value> = nodes
        .iter()
        .skip((page - 1) * PAGE)
        .take(PAGE)
        .map(|(id, node)| {
            let mut line = Object::new();
            line.set("id", id.0 as f64).set("node", name_of(&node.kind));
            if let Some(label) = &node.label {
                line.set("label", label.clone());
            }
            if node.muted {
                line.set("muted", true);
            }
            let to: Vec<Value> = graph
                .links()
                .iter()
                .filter(|link| link.from == *id)
                .filter_map(|link| {
                    let target = graph.node(link.to)?;
                    let end = end_name(graph, link.to, &input_name(&target.kind, link.to_socket));
                    Some(Value::String(match node.kind.outputs().len() {
                        0 | 1 => end,
                        _ => format!("{} > {end}", output_name(&node.kind, link.from_socket)),
                    }))
                })
                .collect();
            line.set("to", Value::Array(to));
            line.build()
        })
        .collect();
    let driven: Vec<Value> = graph
        .links()
        .iter()
        .filter(|link| link.to == graph.output())
        .map(|link| Value::String(input_name(&graph.node(link.to).expect("the surface is there").kind, link.to_socket)))
        .collect();
    let mut out = Object::new();
    out.set("material", key.0 as f64)
        .set("name", definition.name.clone())
        .set("type", definition.material_type.id())
        .set("surface", graph.output().0 as f64)
        .set("count", nodes.len())
        .set("page", page)
        .set("pages", pages)
        .set("driven", Value::Array(driven))
        .set("nodes", Value::Array(lines));
    out.set(
        "note",
        match nodes.is_empty() {
            true => "No nodes yet: every parameter is its own value. Add one, then connect it to the surface.",
            false => "`to` is where each node's output goes: a node's id and input, or the surface and the parameter it drives. Read one node whole with `node`.",
        },
    );
    Ok(out.build())
}

/// Carry out a `graph` request (the module says what each action is).
pub fn request(request: &Value, document: &SceneDocument) -> Result<LibraryOutcome, String> {
    let action = request.get("action").and_then(Value::as_str).unwrap_or("").trim().to_ascii_lowercase();
    if action == "catalogue" || action == "catalog" {
        return catalogue_answer(request).map(LibraryOutcome::Done);
    }
    if !ACTIONS.contains(&action.as_str()) {
        return Err(format!("`{action}` is not something `graph` does; its actions are {}", ACTIONS.join(", ")));
    }
    let (key, definition) = material_of(document, request)?;
    if action == "read" {
        return match request.get("node") {
            None | Some(Value::Null) => brief_graph(key, definition, request).map(LibraryOutcome::Done),
            Some(_) => {
                let id = node_named(&definition.graph, request, "node")?;
                Ok(LibraryOutcome::Done(describe(Some(id), definition.graph.node(id).expect("it was found"), Some(&definition.graph))))
            }
        };
    }
    let mut graph = definition.graph.clone();
    let mut out = Object::new();
    out.set("material", key.0 as f64);
    match action.as_str() {
        "add" => {
            let name = request.get("node").and_then(Value::as_str).map(|n| n.trim().to_lowercase()).ok_or("say which node to add: its name in the catalogue, as `node`")?;
            let entry = catalogue().into_iter().find(|entry| entry.name == name).ok_or_else(|| format!("there is no node called `{name}`; search the catalogue (action catalogue)"))?;
            let mut node = change(&entry.node, request)?;
            node.position = place(&graph);
            let id = graph.add_node(node);
            out.set("added", id.0 as f64).set("node", describe(Some(id), graph.node(id).expect("just added"), Some(&graph)));
            out.set("note", "It drives nothing until one of its outputs is connected: action connect, from this node, to the surface or another node.");
        }
        "set" => {
            let id = node_named(&graph, request, "node")?;
            if !["settings", "fields", "inputs", "label", "muted"].iter().any(|key| request.get(key).is_some()) {
                return Err("say what to set: `settings`, `fields`, `inputs`, `label` or `muted`".into());
            }
            let changed = change(graph.node(id).expect("it was found"), request)?;
            let before = graph.node(id).expect("it was found").clone();
            if before == changed {
                return Err("that is what the node already has; nothing would change".into());
            }
            *graph.node_mut(id).expect("it was found") = Node { position: before.position, collapsed: before.collapsed, ..changed };
            out.set("set", id.0 as f64).set("node", describe(Some(id), graph.node(id).expect("it was found"), Some(&graph)));
        }
        "connect" => {
            let from = node_named(&graph, request, "from")?;
            let to = match request.get("to") {
                None => graph.output(),
                Some(_) => node_named(&graph, request, "to")?,
            };
            let source = graph.node(from).expect("it was found");
            let target = graph.node(to).expect("it was found");
            let output = output_named(source, request.get("output"))?;
            let input = request.get("input").and_then(Value::as_str).ok_or("say which input the wire goes into, as `input`: an input's name, or on the surface a parameter's key")?;
            let (_, socket, _) = input_named(target, input)?;
            let replaced = graph.links().iter().find(|link| link.to == to && link.to_socket == socket).map(|link| {
                let name = graph.node(link.from).map(|n| output_name(&n.kind, link.from_socket)).unwrap_or_default();
                end_name(&graph, link.from, &name)
            });
            let wire = format!("{} > {}", end_name(&graph, from, &output_name(&source.kind, output)), end_name(&graph, to, &input_name(&target.kind, socket)));
            if let (Socket::Param(param), NodeKind::Surface(_)) = (socket, &target.kind) {
                if !definition.material_type.exposed().contains(&param) {
                    out.set("note", format!("A {} material does not show `{}`: the wire is made, and may change nothing.", definition.material_type.label(), param.key()));
                }
            }
            graph.connect_output(from, output, to, socket);
            out.set("connected", wire);
            out.set_some("replaced", replaced);
        }
        "disconnect" => {
            let to = match request.get("node") {
                None => graph.output(),
                Some(_) => node_named(&graph, request, "node")?,
            };
            let target = graph.node(to).expect("it was found");
            let input = request.get("input").and_then(Value::as_str).ok_or("say which input to free, as `input`")?;
            let (_, socket, _) = input_named(target, input)?;
            let Some(link) = graph.links().iter().find(|link| link.to == to && link.to_socket == socket).copied() else {
                return Err(format!("nothing is wired into {}", end_name(&graph, to, &input_name(&target.kind, socket))));
            };
            let name = graph.node(link.from).map(|n| output_name(&n.kind, link.from_socket)).unwrap_or_default();
            out.set("disconnected", format!("{} > {}", end_name(&graph, link.from, &name), end_name(&graph, to, &input_name(&target.kind, socket))));
            graph.disconnect(to, socket);
        }
        "remove" => {
            let id = node_named(&graph, request, "node")?;
            if id == graph.output() {
                return Err("the surface is the material itself and cannot be removed".into());
            }
            let wires = graph.links().iter().filter(|link| link.from == id || link.to == id).count();
            let removed = graph.remove_node(id).expect("it was found");
            out.set("removed", id.0 as f64).set("node", name_of(&removed.kind)).set("wires_removed", wires);
        }
        _ => unreachable!("the actions were checked"),
    }
    graph.validate().map_err(|error| format!("that would leave a graph that does not work: {error}. Nothing was changed."))?;
    if action == "connect" {
        if let Ok(to) = match request.get("to") {
            None => Ok(graph.output()),
            Some(_) => node_named(&graph, request, "to"),
        } {
            keep_left(&mut graph, to, 0);
        }
    }
    out.set("nodes", graph.len() - 1);
    Ok(LibraryOutcome::Commands(vec![Command::SetGraph { material: key, graph: Box::new(graph) }], out.build()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::json;

    fn ask(text: &str) -> Value {
        json::parse(text.as_bytes()).expect("the test's request is JSON")
    }

    /// A material and the bus that changes it.
    fn a_material() -> (SceneDocument, tequila_scene::CommandBus, u64) {
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2);
        let key = document.materials.iter().next().map(|(key, _)| key.0).expect("the demo has a material");
        (document, tequila_scene::CommandBus::new(), key)
    }

    fn carry(document: &mut SceneDocument, bus: &mut tequila_scene::CommandBus, text: &str) -> Result<Value, String> {
        match request(&ask(text), document)? {
            LibraryOutcome::Commands(commands, answer) => {
                assert_eq!(commands.len(), 1, "a graph edit is one command");
                for command in commands {
                    bus.apply(document, command).map_err(|e| e.to_string())?;
                }
                bus.end_interaction();
                Ok(answer)
            }
            LibraryOutcome::Done(answer) => Ok(answer),
        }
    }

    /// Every entry has a name of its own, says what it is for, is called
    /// by that name when read back out of a graph, and answers whole in a
    /// size that is read.
    #[test]
    fn the_catalogue_names_every_node_once_and_describes_it() {
        let entries = catalogue();
        assert!(entries.len() > 80, "{} entries", entries.len());
        let mut names = std::collections::BTreeSet::new();
        for entry in &entries {
            assert!(names.insert(entry.name.clone()), "two nodes are called `{}`", entry.name);
            assert!(entry.what.len() > 20, "`{}` does not say what it is for", entry.name);
            assert_eq!(name_of(&entry.node.kind), entry.name, "a {} read back is called something else", entry.label);
            let whole = tequila_format::json_write::to_string(&catalogue_answer(&ask(&format!(r#"{{"name":"{}"}}"#, entry.name))).unwrap());
            assert!(whole.len() < 6_000, "`{}` answers with {} characters", entry.name, whole.len());
        }
        for node in ShaderNode::catalog() {
            assert!(names.contains(node.key()), "the catalogue has no {}", node.key());
        }
        for group in NodeGroup::ALL {
            assert!(names.contains(group.key()), "the catalogue has no {}", group.key());
        }
    }

    #[test]
    fn the_catalogue_is_searched_a_page_at_a_time() {
        let all = catalogue_answer(&ask("{}")).unwrap();
        let count = all.get("count").and_then(Value::as_index).unwrap();
        let pages = all.get("pages").and_then(Value::as_index).unwrap();
        assert_eq!(pages, count.div_ceil(PAGE));
        assert_eq!(all.get("nodes").map(Value::as_array).unwrap().len(), PAGE);
        for page in 1..=pages {
            let answer = tequila_format::json_write::to_string(&catalogue_answer(&ask(&format!(r#"{{"page":{page}}}"#))).unwrap());
            assert!(answer.len() < 8_000, "page {page} is {} characters", answer.len());
        }
        assert!(catalogue_answer(&ask(&format!(r#"{{"page":{}}}"#, pages + 1))).is_err());

        // A search that leaves a few answers with them whole.
        let voronoi = catalogue_answer(&ask(r#"{"search":"voronoi"}"#)).unwrap();
        let found = voronoi.get("nodes").map(Value::as_array).unwrap();
        assert_eq!(found[0].get("name").and_then(Value::as_str), Some("voronoi_texture"));
        assert!(found[0].get("outputs").map(Value::as_array).unwrap().iter().any(|o| o.get("output").and_then(Value::as_str) == Some("Distance")));
        assert!(found[0].get("settings").and_then(|s| s.get("feature")).and_then(|f| f.get("options")).is_some());

        let groups = catalogue_answer(&ask(r#"{"family":"group"}"#)).unwrap();
        assert_eq!(groups.get("count").and_then(Value::as_index), Some(NodeGroup::ALL.len()));
        assert!(catalogue_answer(&ask(r#"{"family":"shaders"}"#)).unwrap_err().contains("families are"));
        assert!(catalogue_answer(&ask(r#"{"name":"voronoi"}"#)).unwrap_err().contains("voronoi_texture"));
        assert_eq!(catalogue_answer(&ask(r#"{"search":"no such thing at all"}"#)).unwrap().get("count").and_then(Value::as_index), Some(0));
    }

    /// The round an assistant makes: add a node of a type it has never
    /// used, set it, wire it through a ramp into the surface, read it
    /// back, take a wire off, remove a node - each one step to undo.
    #[test]
    fn a_node_is_added_set_wired_read_and_removed_a_step_at_a_time() {
        let (mut document, mut bus, material) = a_material();
        let key = MaterialKey(material);
        let empty = carry(&mut document, &mut bus, &format!(r#"{{"action":"read","material":{material}}}"#)).unwrap();
        assert_eq!(empty.get("count").and_then(Value::as_index), Some(0));

        let added = carry(&mut document, &mut bus, &format!(r#"{{"action":"add","material":{material},"node":"voronoi_texture","settings":{{"feature":"f2"}},"inputs":{{"Scale":40}}}}"#)).unwrap();
        let cells = added.get("added").and_then(Value::as_index).unwrap();
        let node = added.get("node").unwrap();
        assert_eq!(node.get("settings").and_then(|s| s.get("feature")).and_then(|f| f.get("value")).and_then(Value::as_str), Some("f2"));
        let scale = node.get("inputs").map(Value::as_array).unwrap().iter().find(|i| i.get("input").and_then(Value::as_str) == Some("Scale")).unwrap();
        assert_eq!(scale.get("value").and_then(Value::as_f32), Some(40.0));

        let ramp = carry(&mut document, &mut bus, &format!(r##"{{"action":"add","material":{material},"node":"gradient","fields":{{"gradient":[{{"at":0,"color":"#101010"}},{{"at":1,"color":"#d0d0d0"}}]}}}}"##)).unwrap();
        let ramp = ramp.get("added").and_then(Value::as_index).unwrap();
        let wired = carry(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":{material},"from":{cells},"output":"Distance","to":{ramp},"input":"Position"}}"#)).unwrap();
        assert_eq!(wired.get("connected").and_then(Value::as_str), Some(format!("{cells}.Distance > {ramp}.Position").as_str()));
        let driven = carry(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":{material},"from":{ramp},"input":"base_color"}}"#)).unwrap();
        assert_eq!(driven.get("connected").and_then(Value::as_str), Some(format!("{ramp}.Color > surface.base_color").as_str()));
        let graph = &document.materials.get(key).unwrap().graph;
        assert!(graph.is_connected(Param::BaseColor));
        graph.compile().expect("what the requests built compiles");

        let brief = carry(&mut document, &mut bus, &format!(r#"{{"action":"read","material":{material}}}"#)).unwrap();
        assert_eq!(brief.get("count").and_then(Value::as_index), Some(2));
        assert_eq!(brief.get("driven").map(Value::as_array).unwrap()[0].as_str(), Some("base_color"));
        let whole = carry(&mut document, &mut bus, &format!(r#"{{"action":"read","material":{material},"node":{cells}}}"#)).unwrap();
        let distance = whole.get("outputs").map(Value::as_array).unwrap().iter().find(|o| o.get("output").and_then(Value::as_str) == Some("Distance")).unwrap();
        assert_eq!(distance.get("to").map(Value::as_array).unwrap()[0].as_str(), Some(format!("{ramp}.Position").as_str()));

        // One value, and nothing else of the node, changes.
        let before = document.materials.get(key).unwrap().graph.node(NodeId(cells as u32)).unwrap().clone();
        carry(&mut document, &mut bus, &format!(r#"{{"action":"set","material":{material},"node":{cells},"inputs":{{"scale":80}},"label":"Cells"}}"#)).unwrap();
        let after = document.materials.get(key).unwrap().graph.node(NodeId(cells as u32)).unwrap().clone();
        assert_eq!(after.kind, before.kind);
        assert_eq!(after.label.as_deref(), Some("Cells"));
        assert_eq!(after.literals.values().filter(|v| **v == GraphValue::Float(80.0)).count(), 1);

        // Each was a step of its own: one undo takes back the last alone.
        bus.undo(&mut document).unwrap();
        assert_eq!(document.materials.get(key).unwrap().graph.node(NodeId(cells as u32)).unwrap(), &before);
        bus.redo(&mut document).unwrap();

        let freed = carry(&mut document, &mut bus, &format!(r#"{{"action":"disconnect","material":{material},"input":"base_color"}}"#)).unwrap();
        assert!(freed.get("disconnected").and_then(Value::as_str).unwrap().ends_with("surface.base_color"));
        assert!(!document.materials.get(key).unwrap().graph.is_connected(Param::BaseColor));
        let removed = carry(&mut document, &mut bus, &format!(r#"{{"action":"remove","material":{material},"node":{cells}}}"#)).unwrap();
        assert_eq!(removed.get("wires_removed").and_then(Value::as_index), Some(1));
        assert_eq!(removed.get("nodes").and_then(Value::as_index), Some(1));
    }

    /// What is wrong is said in the words that put it right, and nothing
    /// is changed.
    #[test]
    fn a_wrong_request_is_refused_in_words() {
        let (mut document, mut bus, material) = a_material();
        let refused = |document: &mut SceneDocument, bus: &mut tequila_scene::CommandBus, text: &str| carry(document, bus, &text.replace("MATERIAL", &material.to_string())).unwrap_err();
        let noise = carry(&mut document, &mut bus, &format!(r#"{{"action":"add","material":{material},"node":"noise_texture"}}"#)).unwrap().get("added").and_then(Value::as_index).unwrap();
        let before = document.materials.get(MaterialKey(material)).unwrap().graph.clone();

        assert!(refused(&mut document, &mut bus, r#"{"action":"add","material":MATERIAL,"node":"perlin"}"#).contains("no node called `perlin`"));
        assert!(refused(&mut document, &mut bus, r#"{"action":"paint","material":MATERIAL}"#).contains("actions are"));
        assert!(refused(&mut document, &mut bus, r#"{"action":"read","material":999}"#).contains("no material 999"));
        assert!(refused(&mut document, &mut bus, r#"{"action":"remove","material":MATERIAL,"node":99}"#).contains("its nodes are"));
        assert!(refused(&mut document, &mut bus, r#"{"action":"remove","material":MATERIAL,"node":"surface"}"#).contains("cannot be removed"));
        let text = |tail: &str| format!(r#"{{"action":"set","material":MATERIAL,"node":{noise},{tail}}}"#);
        assert!(refused(&mut document, &mut bus, &text(r#""settings":{"kind":"fbm"}"#)).contains("its settings are dimensions, type, normalize"));
        assert!(refused(&mut document, &mut bus, &text(r#""settings":{"type":"cloudy"}"#)).contains("`type` is one of"));
        assert!(refused(&mut document, &mut bus, &text(r#""inputs":{"Size":3}"#)).contains("its inputs are"));
        assert!(refused(&mut document, &mut bus, &text(r#""inputs":{"Scale":"big"}"#)).contains("takes a number"));
        assert!(refused(&mut document, &mut bus, &text(r#""fields":{"depth":1}"#)).contains("no fields of its own"));
        // The value an input has without being told is no change.
        assert!(refused(&mut document, &mut bus, &text(r#""inputs":{"Scale":25}"#)).contains("already has"));
        // A number cannot be a whole surface, and a node cannot feed itself.
        assert!(refused(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":MATERIAL,"from":{noise},"input":"Layer"}}"#)).contains("Nothing was changed"));
        assert!(refused(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":MATERIAL,"from":{noise},"to":{noise},"input":"Scale"}}"#)).contains("cycle"));
        assert!(refused(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":MATERIAL,"from":{noise},"output":"Alpha","input":"base_color"}}"#)).contains("its outputs are"));
        assert!(refused(&mut document, &mut bus, r#"{"action":"disconnect","material":MATERIAL,"input":"bump"}"#).contains("nothing is wired"));
        assert_eq!(document.materials.get(MaterialKey(material)).unwrap().graph, before);
    }

    /// Every node of the catalogue can be added to a material by its
    /// name, and what is left compiles.
    #[test]
    fn every_node_of_the_catalogue_is_added_by_its_name() {
        let (mut document, mut bus, material) = a_material();
        for entry in catalogue() {
            let answer = carry(&mut document, &mut bus, &format!(r#"{{"action":"add","material":{material},"node":"{}"}}"#, entry.name)).unwrap_or_else(|e| panic!("{}: {e}", entry.name));
            assert_eq!(answer.get("node").and_then(|n| n.get("node")).and_then(Value::as_str), Some(entry.name.as_str()));
        }
        let graph = &document.materials.get(MaterialKey(material)).unwrap().graph;
        assert_eq!(graph.len() - 1, catalogue().len());
        graph.compile().expect("a graph of one of everything, wired to nothing, compiles");
    }

    /// A node added to a graph a person has laid out lands where no node
    /// is; one added to a recipe's graph, laid out by nobody, leaves it
    /// for the editor to arrange.
    #[test]
    fn a_new_node_is_never_put_on_another() {
        let (mut document, mut bus, material) = a_material();
        let key = MaterialKey(material);
        for _ in 0..3 {
            carry(&mut document, &mut bus, &format!(r#"{{"action":"add","material":{material},"node":"value"}}"#)).unwrap();
        }
        let graph = &document.materials.get(key).unwrap().graph;
        let mut places = std::collections::HashSet::new();
        assert!(graph.nodes().all(|(_, node)| places.insert((node.position.x.to_bits(), node.position.y.to_bits()))), "two nodes share a place");

        // Wired in the order a chain is built - the bump after the pattern
        // that feeds it - each node ends up to the left of what it feeds.
        let (mut document, mut bus, material) = a_material();
        let key = MaterialKey(material);
        let add = |document: &mut SceneDocument, bus: &mut tequila_scene::CommandBus, node: &str| carry(document, bus, &format!(r#"{{"action":"add","material":{material},"node":"{node}"}}"#)).unwrap().get("added").and_then(Value::as_index).unwrap();
        let knurl = add(&mut document, &mut bus, "knurl");
        let bump = add(&mut document, &mut bus, "bump");
        let scale = add(&mut document, &mut bus, "value");
        carry(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":{material},"from":{scale},"to":{knurl},"input":"Pitch (mm)"}}"#)).unwrap();
        carry(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":{material},"from":{knurl},"to":{bump},"input":"Height"}}"#)).unwrap();
        carry(&mut document, &mut bus, &format!(r#"{{"action":"connect","material":{material},"from":{bump},"input":"bump"}}"#)).unwrap();
        let graph = &document.materials.get(key).unwrap().graph;
        for link in graph.links() {
            let (from, to) = (graph.node(link.from).unwrap().position, graph.node(link.to).unwrap().position);
            assert!(from.x < to.x, "a wire runs backwards, from {from:?} to {to:?}");
        }
        let mut places = std::collections::HashSet::new();
        assert!(graph.nodes().all(|(_, node)| places.insert((node.position.x.to_bits(), node.position.y.to_bits()))), "two nodes share a place");
    }
}
