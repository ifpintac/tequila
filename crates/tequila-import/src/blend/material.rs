//! Blender's shader node trees, as Tequila material graphs.
//!
//! Blender writes a material as a tree of nodes, and Tequila has most of
//! them as nodes of its own, evaluated with Cycles' algorithms
//! (`tequila_materials::nodes`): the textures, the maths, the colour and
//! vector nodes. So a tree is translated node for node, not baked or
//! guessed at - a powder coat made of a Voronoi and a noise is that Voronoi
//! and that noise in Tequila, at the same scale, and still editable there.
//!
//! What is not node for node:
//!
//! - **Shaders.** Tequila's surface is one OpenPBR material, and Blender's
//!   Principled BSDF is the same model under other names, so it maps
//!   parameter by parameter (`principled`). The other BSDFs become the
//!   OpenPBR material that renders as they do: a Glossy BSDF a metal of its
//!   colour, a Glass BSDF a glass. Mix and Add Shader become Tequila's own,
//!   rearranged into the nesting Tequila mixes in (`comb`); a Transparent
//!   BSDF mixed in becomes the other surface's opacity, and an Emission
//!   added on becomes the other surface's emission.
//! - **Groups, reroutes and muted nodes** are taken apart before anything is
//!   translated (`flatten`): a group is the nodes inside it, wired to what
//!   was wired to it.
//! - **Image textures** read the picture through the mesh's UVs where
//!   Blender does, and otherwise at the coordinates the tree works out, as
//!   Blender does. Blender's Clip, Extend and Mirror are built from its own
//!   maths nodes around the lookup, so both of Tequila's renderers do them.
//! - **Space.** Blender is Z up and Tequila Y up. A mesh arrives in its own
//!   object space, so Object, Generated and the object normal need nothing;
//!   a world-space position or direction (the Geometry node, Reflection) is
//!   turned into Blender's axes before it is used as a number, and left as
//!   it is where it is used as a normal.
//! - **Bump, Normal Map, Displacement and Bevel** are not values in
//!   Tequila but a surface's Bump and Edge radius, so they are read where a
//!   shader's Normal takes them.
//!
//! Anything with no counterpart is named once in the import's warnings, and
//! what it fed keeps the value typed into that socket in Blender.

use super::Notes;
use crate::raw::RawTexture;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_format::json::Value as Json;
use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, SocketType, TextureId, Value, LAYER_INPUT};
use tequila_materials::material_type::{MaterialType, Param, ParamValue};
use tequila_materials::nodes::{BlenderSetting, ShaderNode};
use tequila_materials::openpbr::Material;
use tequila_scene::material::MaterialDefinition;
use tequila_texture::cycles::ops::{MappingKind, MathOp, VectorMathOp};
use tequila_texture::gradient::{Gradient, Interpolation, Stop};
use tequila_texture::projection::{Mapping, Projection};
use tequila_texture::texture::MapRole;
use tequila_texture::Wrap;

/// Blender's emission strength to Tequila's luminance: the same radiance,
/// as glTF's emissive strength comes over.
const EMISSION: f32 = 1.0;

/// How deep groups may nest before the rest is left out.
const DEEPEST: usize = 32;

/// What the translation reads besides the tree.
pub(super) struct Context<'a> {
    pub trees: &'a [Json],
    pub images: &'a [Json],
    /// A packed image Blender wrote out, by its file name.
    pub packed: &'a dyn Fn(&str) -> Option<Arc<Vec<u8>>>,
    /// Blender units to metres.
    pub unit_scale: f32,
}

/// A Blender material as Tequila's.
pub(super) fn material(entry: &Json, cx: &Context<'_>, textures: &mut Vec<RawTexture>, notes: &mut Notes) -> MaterialDefinition {
    let name = entry.get("name").and_then(Json::as_str).unwrap_or("Material").to_string();
    let Some(tree) = entry.get("tree").and_then(Json::as_index).filter(|&t| t < cx.trees.len()) else {
        return viewport(&name, entry);
    };
    let flat = flatten(cx.trees, tree, notes, &name);
    let Some(output) = material_output(&flat) else {
        notes.add(format!("{name} has no Material Output; it came over as its viewport colour"));
        return viewport(&name, entry);
    };
    let mut t = Translator::new(cx, &flat, textures);
    let surface = t.shade_input(output, "Surface");
    let mut comb = t.comb(surface);
    // Blender's Displacement output, which renders as a bump unless the
    // material asks for true displacement.
    if let Some(Wire::Bump { height, depth, under }) = t.input(output, "Displacement") {
        if comb.first.has_bump {
            t.note("its Displacement was left out: the surface's own Normal already bumps it".into());
        } else {
            t.bump(&mut comb.first, Wire::Bump { height, depth, under });
        }
    }
    if t.input_linked(output, "Volume") {
        t.note("its Volume was left out: Tequila's media are the surface's own".into());
    }
    let said = std::mem::take(&mut t.notes);
    let definition = t.emit(comb, name.clone());
    for note in said {
        notes.add(format!("{name}: {note}"));
    }
    definition
}

/// A material with no nodes: a plastic of its viewport colour, as Blender
/// renders one.
fn viewport(name: &str, entry: &Json) -> MaterialDefinition {
    let colour = entry.get("color").and_then(Json::as_f32_array).unwrap_or_else(|| vec![0.8, 0.8, 0.8]);
    let mut m = MaterialType::Generic.defaults();
    m.base_color = linear(&colour);
    m.base_metalness = entry.get("metallic").and_then(Json::as_f32).unwrap_or(0.0).clamp(0.0, 1.0);
    m.specular_roughness = entry.get("roughness").and_then(Json::as_f32).unwrap_or(0.5).clamp(0.0, 1.0);
    let kind = if m.base_metalness > 0.5 { MaterialType::Metal } else { MaterialType::Plastic };
    MaterialDefinition::new(name, kind, m)
}

fn linear(rgb: &[f32]) -> Spectrum {
    let at = |i: usize| rgb.get(i).copied().unwrap_or(0.0).max(0.0);
    Spectrum::from_linear_srgb(vec3(at(0), at(1), at(2)))
}

// ---- Taking groups apart ------------------------------------------------

/// Where an input's value comes from, once groups, reroutes and muted nodes
/// are seen through.
#[derive(Clone, Copy, Debug)]
enum Source<'a> {
    /// An output of a node in the flat list, by its identifier.
    Output(usize, &'a str),
    /// The value typed into a socket: its entry, with its type.
    Typed(&'a Json),
    Nothing,
}

/// A node that does something, with where each of its inputs comes from.
struct Flat<'a> {
    node: &'a Json,
    inputs: Vec<Source<'a>>,
}

impl<'a> Flat<'a> {
    fn kind(&self) -> &'a str {
        self.node.get("type").and_then(Json::as_str).unwrap_or("")
    }
}

/// Nodes that are wiring, not work.
fn is_wiring(kind: &str) -> bool {
    matches!(kind, "NodeReroute" | "NodeGroupInput" | "NodeGroupOutput" | "ShaderNodeGroup" | "NodeFrame")
}

fn muted(node: &Json) -> bool {
    node.get("mute").and_then(Json::as_bool).unwrap_or(false)
}

fn sockets<'a>(node: &'a Json, side: &str) -> &'a [Json] {
    node.get(side).map(Json::as_array).unwrap_or(&[])
}

fn id_of(socket: &Json) -> &str {
    socket.get("id").and_then(Json::as_str).unwrap_or("")
}

fn name_of(socket: &Json) -> &str {
    socket.get("name").and_then(Json::as_str).unwrap_or("")
}

fn enabled(socket: &Json) -> bool {
    socket.get("enabled").and_then(Json::as_bool).unwrap_or(true)
}

/// Every node of `root` that does something, its groups opened up.
fn flatten<'a>(trees: &'a [Json], root: usize, notes: &mut Notes, name: &str) -> Vec<Flat<'a>> {
    let mut expander = Expander { trees, flat: Vec::new(), too_deep: false };
    expander.expand(root, BTreeMap::new(), 0);
    if expander.too_deep {
        notes.add(format!("{name}: its node groups nest more than {DEEPEST} deep; the deepest were left out"));
    }
    expander.flat
}

struct Expander<'a> {
    trees: &'a [Json],
    flat: Vec<Flat<'a>>,
    too_deep: bool,
}

/// One tree being opened: its nodes, what is wired into each socket, and
/// the flat place of each node that does something.
struct Scope<'a> {
    nodes: &'a [Json],
    incoming: HashMap<(usize, &'a str), (usize, &'a str)>,
    places: Vec<Option<usize>>,
    group_inputs: BTreeMap<&'a str, Source<'a>>,
    groups: HashMap<usize, BTreeMap<&'a str, Source<'a>>>,
    depth: usize,
}

impl<'a> Expander<'a> {
    /// Open `tree` with `group_inputs` wired to its Group Input, and say
    /// what its Group Output gives.
    fn expand(&mut self, tree: usize, group_inputs: BTreeMap<&'a str, Source<'a>>, depth: usize) -> BTreeMap<&'a str, Source<'a>> {
        let Some(tree) = self.trees.get(tree) else { return BTreeMap::new() };
        let nodes = tree.get("nodes").map(Json::as_array).unwrap_or(&[]);
        let mut incoming = HashMap::new();
        for link in tree.get("links").map(Json::as_array).unwrap_or(&[]) {
            let l = link.as_array();
            if let (Some(from), Some(from_socket), Some(to), Some(to_socket)) = (
                l.first().and_then(Json::as_index),
                l.get(1).and_then(Json::as_str),
                l.get(2).and_then(Json::as_index),
                l.get(3).and_then(Json::as_str),
            ) {
                if from < nodes.len() && to < nodes.len() {
                    incoming.insert((to, to_socket), (from, from_socket));
                }
            }
        }
        let mut places = vec![None; nodes.len()];
        for (i, node) in nodes.iter().enumerate() {
            let kind = node.get("type").and_then(Json::as_str).unwrap_or("");
            if !is_wiring(kind) && !muted(node) {
                places[i] = Some(self.flat.len());
                self.flat.push(Flat { node, inputs: Vec::new() });
            }
        }
        let mut scope = Scope { nodes, incoming, places, group_inputs, groups: HashMap::new(), depth };
        for i in 0..nodes.len() {
            let Some(place) = scope.places[i] else { continue };
            let inputs: Vec<Source<'a>> = sockets(&nodes[i], "inputs").iter().map(|s| self.source(&mut scope, i, id_of(s), s)).collect();
            self.flat[place].inputs = inputs;
        }
        // The Group Output in use, when this is a group.
        let output = nodes
            .iter()
            .position(|n| n.get("type").and_then(Json::as_str) == Some("NodeGroupOutput") && n.get("active").and_then(Json::as_bool).unwrap_or(true))
            .or_else(|| nodes.iter().position(|n| n.get("type").and_then(Json::as_str) == Some("NodeGroupOutput")));
        let mut out = BTreeMap::new();
        if let Some(o) = output {
            for socket in sockets(&nodes[o], "inputs") {
                let source = self.source(&mut scope, o, id_of(socket), socket);
                out.insert(id_of(socket), source);
            }
        }
        out
    }

    /// What feeds input `socket` of node `node`.
    fn source(&mut self, scope: &mut Scope<'a>, node: usize, socket: &'a str, entry: &'a Json) -> Source<'a> {
        match scope.incoming.get(&(node, socket)).copied() {
            None => Source::Typed(entry),
            Some((from, from_socket)) => self.output(scope, from, from_socket),
        }
    }

    /// What output `socket` of node `node` gives, seen through wiring.
    fn output(&mut self, scope: &mut Scope<'a>, node: usize, socket: &'a str) -> Source<'a> {
        let n = &scope.nodes[node];
        let kind = n.get("type").and_then(Json::as_str).unwrap_or("");
        let input = |id: &str| sockets(n, "inputs").iter().find(|s| id_of(s) == id);
        if kind == "NodeReroute" {
            let Some(entry) = sockets(n, "inputs").first() else { return Source::Nothing };
            return self.source(scope, node, id_of(entry), entry);
        }
        if kind == "NodeGroupInput" {
            return scope.group_inputs.get(socket).copied().unwrap_or(Source::Nothing);
        }
        if kind == "ShaderNodeGroup" {
            if !scope.groups.contains_key(&node) {
                let outputs = match n.get("group").and_then(Json::as_index) {
                    Some(tree) if scope.depth < DEEPEST && !muted(n) => {
                        let mut inputs = BTreeMap::new();
                        for s in sockets(n, "inputs") {
                            let source = self.source(scope, node, id_of(s), s);
                            inputs.insert(id_of(s), source);
                        }
                        self.expand(tree, inputs, scope.depth + 1)
                    }
                    Some(_) if !muted(n) => {
                        self.too_deep = true;
                        BTreeMap::new()
                    }
                    _ => BTreeMap::new(),
                };
                scope.groups.insert(node, outputs);
            }
            return scope.groups[&node].get(socket).copied().unwrap_or(Source::Nothing);
        }
        if muted(n) {
            // Blender's mute: each output passes on the input it is joined to.
            let through = n
                .get("internal_links")
                .map(Json::as_array)
                .unwrap_or(&[])
                .iter()
                .find(|l| l.index(1).and_then(Json::as_str) == Some(socket))
                .and_then(|l| l.index(0).and_then(Json::as_str));
            return match through.and_then(input) {
                Some(entry) => self.source(scope, node, id_of(entry), entry),
                None => Source::Nothing,
            };
        }
        match scope.places[node] {
            Some(place) => Source::Output(place, socket),
            None => Source::Nothing,
        }
    }
}

/// The Material Output a render uses: the active one for Cycles or all.
fn material_output(flat: &[Flat<'_>]) -> Option<usize> {
    let outputs: Vec<usize> = (0..flat.len()).filter(|&i| flat[i].kind() == "ShaderNodeOutputMaterial").collect();
    let target = |i: usize| flat[i].node.get("settings").and_then(|s| s.get("target")).and_then(Json::as_str).unwrap_or("ALL");
    let active = |i: usize| flat[i].node.get("active").and_then(Json::as_bool).unwrap_or(false);
    outputs
        .iter()
        .copied()
        .find(|&i| active(i) && target(i) != "EEVEE")
        .or_else(|| outputs.iter().copied().find(|&i| target(i) != "EEVEE"))
        .or_else(|| outputs.first().copied())
}

// ---- Values -------------------------------------------------------------

/// Which of Tequila's spaces a vector is in, when it is not Blender's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum World {
    No,
    /// A direction in Tequila's world.
    Direction,
    /// A position in Tequila's world.
    Position,
}

/// A value as it reaches an input.
#[derive(Clone, Debug)]
enum Wire {
    Link { node: NodeId, output: u8, world: World },
    Literal(Value),
    /// A Bump or Displacement node's normal: a height and its depth, and
    /// the normal it bends.
    Bump { height: Box<Wire>, depth: f32, under: Option<Box<Wire>> },
    /// A Normal Map node's normal: a normal map's lookup.
    NormalMap { node: NodeId },
    /// A Bevel node's normal: an edge rounded this much.
    Bevel { radius: f32, under: Option<Box<Wire>> },
}

impl Wire {
    fn link(node: NodeId) -> Wire {
        Wire::Link { node, output: 0, world: World::No }
    }

    fn literal(&self) -> Option<Value> {
        match self {
            Wire::Literal(v) => Some(*v),
            _ => None,
        }
    }

    fn number(&self) -> Option<f32> {
        self.literal().map(Value::as_float)
    }
}

/// A socket's own value, in the graph's terms.
fn typed(socket: &Json) -> Option<Value> {
    let value = socket.get("value")?;
    let ty = socket.get("type").and_then(Json::as_str).unwrap_or("VALUE");
    Some(match value {
        Json::Number(n) => Value::Float(*n as f32),
        Json::Bool(b) => Value::Float(if *b { 1.0 } else { 0.0 }),
        Json::Array(_) => {
            let v = value.as_f32_array()?;
            let at = |i: usize| v.get(i).copied().unwrap_or(0.0);
            match ty {
                "RGBA" => Value::Color(linear(&v)),
                _ => Value::Vector(vec3(at(0), at(1), at(2))),
            }
        }
        _ => return None,
    })
}

fn setting<'a>(node: &'a Json, key: &str) -> Option<BlenderSetting<'a>> {
    match node.get("settings")?.get(key)? {
        Json::String(s) => Some(BlenderSetting::Text(s)),
        Json::Bool(b) => Some(BlenderSetting::Bool(*b)),
        Json::Number(n) if n.fract() == 0.0 => Some(BlenderSetting::Int(*n as i32)),
        Json::Number(n) => Some(BlenderSetting::Float(*n as f32)),
        _ => None,
    }
}

fn text<'a>(node: &'a Json, key: &str) -> Option<&'a str> {
    node.get("settings")?.get(key)?.as_str()
}

// ---- Shaders --------------------------------------------------------------

/// A surface as Tequila holds one: the parameter block, and what drives
/// its parameters from the graph.
#[derive(Clone)]
struct Leaf {
    kind: MaterialType,
    material: Material,
    links: Vec<(Param, Wire)>,
    has_bump: bool,
    /// An Emission shader, which an Add Shader puts on top of another.
    emission_only: bool,
}

impl Leaf {
    fn new(kind: MaterialType) -> Leaf {
        Leaf { kind, material: MaterialType::Generic.defaults(), links: Vec::new(), has_bump: false, emission_only: false }
    }

    fn driven(&self, param: Param) -> Option<&Wire> {
        self.links.iter().rev().find(|(p, _)| *p == param).map(|(_, w)| w)
    }
}

/// A shader tree as Blender wires it.
enum Shade {
    Leaf(Box<Leaf>),
    /// A Transparent BSDF, of this colour.
    Transparent,
    Mix(Box<Shade>, Box<Shade>, Wire),
    Add(Box<Shade>, Box<Shade>),
}

/// The arrangement Tequila mixes surfaces in: the first, with each of the
/// rest mixed (or added) over the ones after it - `Mix(L0, Mix(L1, L2, f2),
/// f1)` is `first: L0, rest: [(f1, L1), (f2, L2)]`.
struct Comb {
    first: Leaf,
    rest: Vec<(Option<Wire>, Leaf)>,
}

impl Comb {
    fn leaves_mut(&mut self) -> impl Iterator<Item = &mut Leaf> {
        std::iter::once(&mut self.first).chain(self.rest.iter_mut().map(|(_, l)| l))
    }
}

struct Translator<'a, 'b> {
    cx: &'b Context<'a>,
    flat: &'b [Flat<'a>],
    graph: MaterialGraph,
    textures: &'b mut Vec<RawTexture>,
    notes: Vec<String>,
    wires: HashMap<(usize, &'a str), Option<Wire>>,
    nodes: HashMap<usize, (NodeId, ShaderNode)>,
    images: HashMap<(usize, bool), (Option<NodeId>, Option<NodeId>)>,
    coordinates: Option<NodeId>,
    turned: HashMap<(NodeId, u8), NodeId>,
}

impl<'a, 'b> Translator<'a, 'b> {
    fn new(cx: &'b Context<'a>, flat: &'b [Flat<'a>], textures: &'b mut Vec<RawTexture>) -> Self {
        Translator {
            cx,
            flat,
            graph: MaterialGraph::from_material(MaterialType::Generic.defaults()),
            textures,
            notes: Vec::new(),
            wires: HashMap::new(),
            nodes: HashMap::new(),
            images: HashMap::new(),
            coordinates: None,
            turned: HashMap::new(),
        }
    }

    fn note(&mut self, note: String) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn label(&self, id: usize) -> String {
        let node = self.flat[id].node;
        let label = node.get("label").and_then(Json::as_str).filter(|l| !l.is_empty());
        let name = node.get("name").and_then(Json::as_str).unwrap_or("?");
        label.unwrap_or(name).to_string()
    }

    /// Input `id` of flat node `node`: its place among the node's inputs.
    fn input_place(&self, node: usize, id: &str) -> Option<usize> {
        sockets(self.flat[node].node, "inputs").iter().position(|s| id_of(s) == id)
    }

    fn input_linked(&self, node: usize, id: &str) -> bool {
        self.input_place(node, id).is_some_and(|i| matches!(self.flat[node].inputs[i], Source::Output(..)))
    }

    /// What reaches input `id` of `node`: a wire, the value typed into it,
    /// or `None` for a socket that is not there or gets nothing usable.
    fn input(&mut self, node: usize, id: &str) -> Option<Wire> {
        let i = self.input_place(node, id)?;
        let source = self.flat[node].inputs[i];
        match source {
            Source::Typed(entry) => typed(entry).map(Wire::Literal),
            Source::Nothing => None,
            Source::Output(from, socket) => match self.output(from, socket) {
                Some(wire) => Some(wire),
                // Unreadable: the value typed into this socket, as if
                // nothing were wired to it.
                None => typed(&sockets(self.flat[node].node, "inputs")[i]).map(Wire::Literal),
            },
        }
    }

    /// What reaches the shader input `id` of `node`.
    fn shade_input(&mut self, node: usize, id: &str) -> Shade {
        let Some(i) = self.input_place(node, id) else { return self.black() };
        match self.flat[node].inputs[i] {
            Source::Output(from, _) => self.shade(from),
            _ => self.black(),
        }
    }

    /// Nothing wired to a shader renders black in Blender.
    fn black(&mut self) -> Shade {
        let mut leaf = Leaf::new(MaterialType::Diffuse);
        leaf.material.base_color = Spectrum::splat(0.0);
        leaf.material.specular_weight = 0.0;
        Shade::Leaf(Box::new(leaf))
    }

    fn shade(&mut self, id: usize) -> Shade {
        let kind = self.flat[id].kind();
        match kind {
            "ShaderNodeBsdfPrincipled" => Shade::Leaf(Box::new(self.principled(id))),
            "ShaderNodeBsdfDiffuse" => {
                let mut leaf = Leaf::new(MaterialType::Diffuse);
                leaf.material.specular_weight = 0.0;
                self.param_from(&mut leaf, id, "Color", Param::BaseColor);
                self.param_from(&mut leaf, id, "Roughness", Param::DiffuseRoughness);
                self.normal_from(&mut leaf, id, "Normal");
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeBsdfGlossy" | "ShaderNodeBsdfAnisotropic" | "ShaderNodeBsdfMetallic" => {
                let mut leaf = Leaf::new(MaterialType::Metal);
                leaf.material.base_metalness = 1.0;
                let colour = if kind == "ShaderNodeBsdfMetallic" { "Base Color" } else { "Color" };
                self.param_from(&mut leaf, id, colour, Param::BaseColor);
                if kind == "ShaderNodeBsdfMetallic" {
                    self.param_from(&mut leaf, id, "Edge Tint", Param::SpecularColor);
                    if text(self.flat[id].node, "fresnel_type") == Some("PHYSICAL_CONDUCTOR") {
                        self.note("a Metallic BSDF's measured IOR came over as its colour".into());
                    }
                }
                self.param_from(&mut leaf, id, "Roughness", Param::SpecularRoughness);
                self.anisotropy(&mut leaf, id);
                self.normal_from(&mut leaf, id, "Normal");
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeBsdfGlass" | "ShaderNodeBsdfRefraction" => {
                let mut leaf = Leaf::new(MaterialType::Glass);
                leaf.material.transmission_weight = 1.0;
                leaf.material.transmission_depth = 0.0;
                leaf.material.base_color = Spectrum::splat(1.0);
                self.param_from(&mut leaf, id, "Color", Param::TransmissionColor);
                self.param_from(&mut leaf, id, "Roughness", Param::SpecularRoughness);
                self.param_from(&mut leaf, id, "IOR", Param::SpecularIor);
                self.normal_from(&mut leaf, id, "Normal");
                if kind == "ShaderNodeBsdfRefraction" {
                    self.note("a Refraction BSDF came over as glass, which also reflects".into());
                }
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeBsdfTranslucent" => {
                let mut leaf = Leaf::new(MaterialType::Translucent);
                leaf.material.specular_weight = 0.0;
                leaf.material.geometry_thin_walled = true;
                leaf.material.subsurface_weight = 1.0;
                self.param_from(&mut leaf, id, "Color", Param::SubsurfaceColor);
                self.param_from(&mut leaf, id, "Color", Param::BaseColor);
                self.normal_from(&mut leaf, id, "Normal");
                self.note("a Translucent BSDF came over as a thin translucent surface".into());
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeBsdfSheen" | "ShaderNodeBsdfVelvet" => {
                let mut leaf = Leaf::new(MaterialType::Velvet);
                leaf.material.base_color = Spectrum::splat(0.0);
                leaf.material.specular_weight = 0.0;
                leaf.material.fuzz_weight = 1.0;
                self.param_from(&mut leaf, id, "Color", Param::FuzzColor);
                self.param_from(&mut leaf, id, "Roughness", Param::FuzzRoughness);
                self.normal_from(&mut leaf, id, "Normal");
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeSubsurfaceScattering" => {
                let mut leaf = Leaf::new(MaterialType::Generic);
                leaf.material.subsurface_weight = 1.0;
                self.param_from(&mut leaf, id, "Color", Param::SubsurfaceColor);
                self.param_from(&mut leaf, id, "Color", Param::BaseColor);
                self.subsurface_radius(&mut leaf, id);
                self.param_from(&mut leaf, id, "Roughness", Param::SpecularRoughness);
                self.param_from(&mut leaf, id, "IOR", Param::SpecularIor);
                self.normal_from(&mut leaf, id, "Normal");
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeEmission" => {
                let mut leaf = Leaf::new(MaterialType::Emissive);
                leaf.material.base_weight = 0.0;
                leaf.material.base_color = Spectrum::splat(0.0);
                leaf.material.specular_weight = 0.0;
                self.emission(&mut leaf, id, "Color", "Strength");
                leaf.emission_only = true;
                Shade::Leaf(Box::new(leaf))
            }
            "ShaderNodeBsdfTransparent" => Shade::Transparent,
            "ShaderNodeHoldout" => {
                self.note("a Holdout came over as transparent".into());
                Shade::Transparent
            }
            "ShaderNodeMixShader" => {
                let factor = self.input(id, "Fac").unwrap_or(Wire::Literal(Value::Float(0.5)));
                let a = self.shade_input(id, "Shader");
                let b = self.shade_input(id, "Shader_001");
                Shade::Mix(Box::new(a), Box::new(b), factor)
            }
            "ShaderNodeAddShader" => {
                let a = self.shade_input(id, "Shader");
                let b = self.shade_input(id, "Shader_001");
                Shade::Add(Box::new(a), Box::new(b))
            }
            "ShaderNodeBsdfToon" | "ShaderNodeEeveeSpecular" | "ShaderNodeBsdfHair" | "ShaderNodeBsdfHairPrincipled" => {
                let label = self.label(id);
                self.note(format!("the {label} shader has no counterpart; it came over as a plastic of its colour"));
                let mut leaf = Leaf::new(MaterialType::Plastic);
                let colour = if kind == "ShaderNodeEeveeSpecular" { "Base Color" } else { "Color" };
                self.param_from(&mut leaf, id, colour, Param::BaseColor);
                Shade::Leaf(Box::new(leaf))
            }
            _ => {
                let label = self.label(id);
                self.note(format!("the {label} node ({kind}) is not a surface Tequila reads; it came over black"));
                self.black()
            }
        }
    }

    /// Blender's Principled BSDF, parameter by parameter: the same model
    /// as OpenPBR under other names, and in places other scales.
    fn principled(&mut self, id: usize) -> Leaf {
        let mut leaf = Leaf::new(MaterialType::Generic);
        let base = self.input(id, "Base Color");
        self.set(&mut leaf, Param::BaseColor, base.clone());
        self.param_from(&mut leaf, id, "Metallic", Param::BaseMetalness);
        self.param_from(&mut leaf, id, "Roughness", Param::SpecularRoughness);
        self.param_from(&mut leaf, id, "IOR", Param::SpecularIor);
        self.param_from(&mut leaf, id, "Alpha", Param::Opacity);
        self.param_from(&mut leaf, id, "Diffuse Roughness", Param::DiffuseRoughness);
        // Blender's specular level is 0.5 at the IOR's own reflection.
        let level = self.input(id, "Specular IOR Level").map(|w| self.scaled(w, 2.0));
        self.set(&mut leaf, Param::SpecularWeight, level);
        self.param_from(&mut leaf, id, "Specular Tint", Param::SpecularColor);
        self.anisotropy(&mut leaf, id);

        let weight = |t: &mut Self, socket: &str| -> (bool, Option<Wire>) {
            let w = t.input(id, socket);
            let on = match &w {
                Some(Wire::Literal(v)) => v.as_float() > 0.0,
                Some(_) => true,
                None => false,
            };
            (on, w)
        };
        let (subsurface, w) = weight(self, "Subsurface Weight");
        self.set(&mut leaf, Param::SubsurfaceWeight, w);
        if subsurface {
            // Blender's subsurface is the base colour's.
            self.set(&mut leaf, Param::SubsurfaceColor, base.clone());
            self.subsurface_radius(&mut leaf, id);
            self.param_from(&mut leaf, id, "Subsurface Anisotropy", Param::SubsurfaceAnisotropy);
        }
        let (transmission, w) = weight(self, "Transmission Weight");
        self.set(&mut leaf, Param::TransmissionWeight, w);
        if transmission {
            // Blender tints what passes through by the base colour, at the
            // surface: no depth to absorb over.
            self.set(&mut leaf, Param::TransmissionColor, base.clone());
            leaf.material.transmission_depth = 0.0;
        }
        let (coat, w) = weight(self, "Coat Weight");
        self.set(&mut leaf, Param::CoatWeight, w);
        if coat {
            self.param_from(&mut leaf, id, "Coat Roughness", Param::CoatRoughness);
            self.param_from(&mut leaf, id, "Coat IOR", Param::CoatIor);
            self.param_from(&mut leaf, id, "Coat Tint", Param::CoatColor);
            if self.input_linked(id, "Coat Normal") {
                self.note("its Coat Normal was left out: the coat follows the surface's normal".into());
            }
        }
        let (sheen, w) = weight(self, "Sheen Weight");
        self.set(&mut leaf, Param::FuzzWeight, w);
        if sheen {
            self.param_from(&mut leaf, id, "Sheen Roughness", Param::FuzzRoughness);
            self.param_from(&mut leaf, id, "Sheen Tint", Param::FuzzColor);
        }
        self.emission(&mut leaf, id, "Emission Color", "Emission Strength");
        let (film, w) = weight(self, "Thin Film Thickness");
        if film {
            leaf.material.thin_film_weight = 1.0;
            self.set(&mut leaf, Param::ThinFilmThickness, w);
            self.param_from(&mut leaf, id, "Thin Film IOR", Param::ThinFilmIor);
        }
        self.normal_from(&mut leaf, id, "Normal");

        // What the Material tab calls it: the plain kind it is, or Generic
        // for anything with more to it.
        let number = |t: &mut Self, socket: &str| t.input(id, socket).map(|w| w.number());
        let metallic = number(self, "Metallic");
        let glass = number(self, "Transmission Weight");
        let extras = subsurface || coat || sheen || film || leaf.material.specular_roughness_anisotropy > 0.0 || leaf.driven(Param::Anisotropy).is_some();
        leaf.kind = match (metallic, glass) {
            _ if extras => MaterialType::Generic,
            (Some(Some(m)), Some(Some(g))) if g >= 0.5 && m < 0.5 => MaterialType::Glass,
            (Some(Some(m)), Some(Some(g))) if m >= 0.99 && g == 0.0 => MaterialType::Metal,
            (Some(Some(m)), Some(Some(g))) if m == 0.0 && g == 0.0 => MaterialType::Plastic,
            _ => MaterialType::Generic,
        };
        leaf
    }

    fn anisotropy(&mut self, leaf: &mut Leaf, id: usize) {
        let socket = if self.input_place(id, "Anisotropic").is_some() { "Anisotropic" } else { "Anisotropy" };
        self.param_from(leaf, id, socket, Param::Anisotropy);
        let amount = leaf.material.specular_roughness_anisotropy > 0.0 || leaf.driven(Param::Anisotropy).is_some();
        if !amount {
            return;
        }
        // Blender turns the direction a whole turn over 0 to 1; Tequila half
        // a turn, which is every direction an anisotropic lobe has.
        let rotation_socket = if self.input_place(id, "Anisotropic Rotation").is_some() { "Anisotropic Rotation" } else { "Rotation" };
        let rotation = self.input(id, rotation_socket).map(|w| match w {
            Wire::Literal(v) => Wire::Literal(Value::Float((v.as_float() * 2.0).rem_euclid(1.0))),
            other => self.scaled(other, 2.0),
        });
        self.set(leaf, Param::AnisotropyRotation, rotation);
        let tangent = self.input(id, "Tangent");
        if let Some(w @ Wire::Link { .. }) = tangent {
            leaf.links.push((Param::Tangent, w));
        }
    }

    fn subsurface_radius(&mut self, leaf: &mut Leaf, id: usize) {
        let scale = self.input(id, "Subsurface Scale").and_then(|w| w.number()).unwrap_or(1.0);
        let socket = if self.input_place(id, "Subsurface Radius").is_some() { "Subsurface Radius" } else { "Radius" };
        match self.input(id, socket) {
            // Per channel, as distances: not a colour to convert.
            Some(Wire::Literal(Value::Vector(r))) => leaf.material.subsurface_radius = Spectrum { r: r.x * scale, g: r.y * scale, b: r.z * scale },
            Some(Wire::Literal(v)) => leaf.material.subsurface_radius = Spectrum::splat(v.as_float() * scale),
            Some(_) => self.note("its Subsurface Radius is driven by nodes; it came over at Tequila's".into()),
            None => {}
        }
    }

    fn emission(&mut self, leaf: &mut Leaf, id: usize, colour: &str, strength: &str) {
        let s = self.input(id, strength);
        let on = match &s {
            Some(Wire::Literal(v)) => v.as_float() > 0.0,
            Some(_) => true,
            None => false,
        };
        if !on {
            leaf.material.emission_luminance = 0.0;
            return;
        }
        let s = s.map(|w| self.scaled(w, EMISSION));
        self.set(leaf, Param::EmissionLuminance, s);
        self.param_from(leaf, id, colour, Param::EmissionColor);
    }

    /// Input `socket` of `id` onto `param`.
    fn param_from(&mut self, leaf: &mut Leaf, id: usize, socket: &str, param: Param) {
        let wire = self.input(id, socket);
        self.set(leaf, param, wire);
    }

    /// `wire` onto `param`: a value into the block, a link to be wired.
    fn set(&mut self, leaf: &mut Leaf, param: Param, wire: Option<Wire>) {
        match wire {
            None => {}
            Some(Wire::Literal(v)) => {
                let value = match param.read(&leaf.material) {
                    Some(ParamValue::Scalar(_)) => {
                        let f = v.as_float();
                        ParamValue::Scalar(match param {
                            Param::SpecularIor | Param::CoatIor | Param::ThinFilmIor => f.clamp(1.0, 3.0),
                            Param::EmissionLuminance | Param::ThinFilmThickness | Param::SpecularWeight => f.max(0.0),
                            _ => f.clamp(0.0, 1.0),
                        })
                    }
                    Some(ParamValue::Color(_)) => ParamValue::Color(v.as_color()),
                    Some(ParamValue::Flag(_)) => ParamValue::Flag(v.as_float() > 0.5),
                    _ => return,
                };
                param.write(&mut leaf.material, value);
            }
            Some(w @ Wire::Link { .. }) => leaf.links.push((param, w)),
            Some(_) => {
                self.note(format!("a normal reached its {} and was left out", param.label()));
            }
        }
    }

    // ---- Normals ----------------------------------------------------------

    fn normal_from(&mut self, leaf: &mut Leaf, id: usize, socket: &str) {
        if let Some(wire) = self.input(id, socket) {
            self.bump(leaf, wire);
        }
    }

    /// A normal into a surface: its Bump, or its Edge radius.
    fn bump(&mut self, leaf: &mut Leaf, wire: Wire) {
        match wire {
            Wire::Literal(_) => {}
            Wire::Link { .. } => self.note("a normal worked out by nodes was left out: Tequila bends a surface by a height or a normal map".into()),
            Wire::NormalMap { node } => {
                leaf.links.push((Param::Bump, Wire::link(node)));
                leaf.has_bump = true;
            }
            Wire::Bevel { radius, under } => {
                leaf.material.edge_radius = radius.max(0.0);
                if let Some(under) = under {
                    self.bump(leaf, *under);
                }
            }
            Wire::Bump { height, depth, under } => {
                // A bump on a bump is their heights added.
                let mut total = self.scaled(*height, depth);
                let mut rest = under.map(|u| *u);
                while let Some(next) = rest.take() {
                    match next {
                        Wire::Bump { height, depth, under } => {
                            let more = self.scaled(*height, depth);
                            total = self.math(MathOp::Add, total, more);
                            rest = under.map(|u| *u);
                        }
                        Wire::Bevel { radius, under } => {
                            leaf.material.edge_radius = radius.max(0.0);
                            rest = under.map(|u| *u);
                        }
                        Wire::NormalMap { node } => {
                            self.note("a normal map under a bump was kept and the bump left out: Tequila takes one or the other".into());
                            leaf.links.push((Param::Bump, Wire::link(node)));
                            leaf.has_bump = true;
                            return;
                        }
                        _ => {}
                    }
                }
                if total.number().is_some_and(|h| h == 0.0) {
                    return;
                }
                let node = self.graph.add_node(Node::new(NodeKind::Bump { depth: 1.0 }));
                self.feed(&total, node, Socket::Input(0), SocketType::Float, false);
                leaf.links.push((Param::Bump, Wire::link(node)));
                leaf.has_bump = true;
            }
        }
    }

    // ---- Arranging shaders --------------------------------------------------

    fn comb(&mut self, shade: Shade) -> Comb {
        match shade {
            Shade::Leaf(leaf) => Comb { first: *leaf, rest: Vec::new() },
            Shade::Transparent => {
                let mut leaf = Leaf::new(MaterialType::Generic);
                leaf.material.geometry_opacity = 0.0;
                Comb { first: leaf, rest: Vec::new() }
            }
            Shade::Mix(a, b, f) => match (*a, *b) {
                // A cut-out: the other surface, there as much as the factor says.
                (Shade::Transparent, b) => {
                    let mut c = self.comb(b);
                    for leaf in c.leaves_mut().collect::<Vec<_>>() {
                        self.opacity_times(leaf, f.clone());
                    }
                    c
                }
                (a, Shade::Transparent) => {
                    let mut c = self.comb(a);
                    let rest = self.math(MathOp::Subtract, Wire::Literal(Value::Float(1.0)), f);
                    for leaf in c.leaves_mut().collect::<Vec<_>>() {
                        self.opacity_times(leaf, rest.clone());
                    }
                    c
                }
                // Mix(Mix(a1, a2, f1), b, f) is Mix(a1, Mix(a2, b, g), h)
                // with h = f1 + f - f1 f and g = f / h.
                (Shade::Mix(a1, a2, f1), b) => {
                    let both = self.math(MathOp::Multiply, f1.clone(), f.clone());
                    let sum = self.math(MathOp::Add, f1, f.clone());
                    let h = self.math(MathOp::Subtract, sum, both);
                    let g = self.math(MathOp::Divide, f, h.clone());
                    self.comb(Shade::Mix(a1, Box::new(Shade::Mix(a2, Box::new(b), g)), h))
                }
                (Shade::Add(a1, a2), b) => {
                    self.note("an Add Shader under a Mix Shader came over as its first shader".into());
                    let _ = a2;
                    self.comb(Shade::Mix(a1, Box::new(b), f))
                }
                (a @ Shade::Leaf(_), b) => {
                    let left = self.comb(a);
                    let right = self.comb(b);
                    let mut rest = vec![(Some(f), right.first)];
                    rest.extend(right.rest);
                    Comb { first: left.first, rest }
                }
            },
            Shade::Add(a, b) => {
                let emits = |s: &Shade| matches!(s, Shade::Leaf(l) if l.emission_only);
                if emits(&b) || emits(&a) {
                    let (glow, other) = if emits(&b) { (*b, *a) } else { (*a, *b) };
                    let Shade::Leaf(glow) = glow else { unreachable!() };
                    let mut c = self.comb(other);
                    for leaf in c.leaves_mut() {
                        if leaf.material.emission_luminance > 0.0 || leaf.driven(Param::EmissionLuminance).is_some() {
                            continue;
                        }
                        leaf.material.emission_color = glow.material.emission_color;
                        leaf.material.emission_luminance = glow.material.emission_luminance;
                        for (p, w) in &glow.links {
                            if matches!(p, Param::EmissionColor | Param::EmissionLuminance) {
                                leaf.links.push((*p, w.clone()));
                            }
                        }
                    }
                    return c;
                }
                match *a {
                    a @ Shade::Leaf(_) => {
                        let left = self.comb(a);
                        let right = self.comb(*b);
                        let mut rest = vec![(None, right.first)];
                        rest.extend(right.rest);
                        Comb { first: left.first, rest }
                    }
                    a => {
                        self.note("an Add Shader of mixed shaders came over as its first".into());
                        self.comb(a)
                    }
                }
            }
        }
    }

    fn opacity_times(&mut self, leaf: &mut Leaf, factor: Wire) {
        let current = match leaf.driven(Param::Opacity) {
            Some(w) => w.clone(),
            None => Wire::Literal(Value::Float(leaf.material.geometry_opacity)),
        };
        leaf.links.retain(|(p, _)| *p != Param::Opacity);
        let product = self.math(MathOp::Multiply, current, factor);
        self.set(leaf, Param::Opacity, Some(product));
    }

    fn emit(mut self, comb: Comb, name: String) -> MaterialDefinition {
        let output = self.graph.output();
        *self.graph.material_mut() = comb.first.material.clone();
        for (param, wire) in &comb.first.links {
            self.feed_param(wire, output, *param);
        }
        let mut under = output;
        for (factor, leaf) in comb.rest {
            let mix = self.graph.add_node(Node::new(NodeKind::MixShader { add: factor.is_none() }));
            if let Some(f) = factor {
                self.feed(&f, mix, Socket::Input(0), SocketType::Float, false);
            }
            let shader = self.graph.add_node(Node::new(NodeKind::Shader(Box::new(leaf.material.clone()))));
            for (param, wire) in &leaf.links {
                self.feed_param(wire, shader, *param);
            }
            self.graph.connect(shader, mix, Socket::Input(1));
            self.graph.connect(mix, under, LAYER_INPUT);
            under = shader;
        }
        MaterialDefinition { name, material_type: comb.first.kind, graph: self.graph, substance: None }
    }

    fn feed_param(&mut self, wire: &Wire, to: NodeId, param: Param) {
        let ty = if param == Param::Tangent { SocketType::Vector } else { SocketType::Float };
        self.feed(wire, to, Socket::Param(param), ty, false);
    }

    // ---- Wiring ---------------------------------------------------------------

    /// `wire` into input `socket` of `to`, which takes `ty`. `implicit`: the
    /// input reads the surface when nothing is wired, so a value has to
    /// arrive on a wire to count.
    fn feed(&mut self, wire: &Wire, to: NodeId, socket: Socket, ty: SocketType, implicit: bool) {
        match wire {
            Wire::Link { node, output, world } => {
                let (node, output) = if *world != World::No && ty != SocketType::Normal { (self.turn(*node, *output, *world), 0) } else { (*node, *output) };
                self.graph.connect_output(node, output, to, socket);
            }
            Wire::Literal(v) => {
                let v = v.convert(ty).unwrap_or(*v);
                if implicit {
                    let constant = self.graph.add_node(Node::new(NodeKind::Constant(v)));
                    self.graph.connect(constant, to, socket);
                } else if let Some(node) = self.graph.node_mut(to) {
                    node.literals.insert(socket, v);
                }
            }
            _ => self.note("a normal reached a socket that takes a value and was left out".into()),
        }
    }

    /// A vector in Tequila's world, in Blender's: a quarter turn about X
    /// takes Y up to Z up.
    fn turn(&mut self, node: NodeId, output: u8, world: World) -> NodeId {
        if let Some(&turned) = self.turned.get(&(node, output)) {
            return turned;
        }
        let kind = if world == World::Position { MappingKind::Point } else { MappingKind::Vector };
        let mut mapping = Node::new(NodeKind::Node(ShaderNode::Mapping { kind }));
        mapping.literals.insert(Socket::Input(1), Value::Vector(Vec3::ZERO));
        mapping.literals.insert(Socket::Input(2), Value::Vector(vec3(std::f32::consts::FRAC_PI_2, 0.0, 0.0)));
        mapping.literals.insert(Socket::Input(3), Value::Vector(Vec3::ONE));
        let turned = self.graph.add_node(mapping);
        self.graph.connect_output(node, output, turned, Socket::Input(0));
        self.turned.insert((node, output), turned);
        turned
    }

    /// `a op b`, worked out here when both are numbers.
    fn math(&mut self, op: MathOp, a: Wire, b: Wire) -> Wire {
        if let (Some(x), Some(y)) = (a.number(), b.number()) {
            let v = ShaderNode::Math { op, clamp: false }.evaluate(&[Value::Float(x), Value::Float(y), Value::Float(0.0)], 0);
            return Wire::Literal(v);
        }
        let node = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::Math { op, clamp: false })));
        self.feed(&a, node, Socket::Input(0), SocketType::Float, false);
        self.feed(&b, node, Socket::Input(1), SocketType::Float, false);
        Wire::link(node)
    }

    fn scaled(&mut self, wire: Wire, k: f32) -> Wire {
        if (k - 1.0).abs() < 1e-9 {
            return wire;
        }
        match wire {
            Wire::Literal(Value::Float(f)) => Wire::Literal(Value::Float(f * k)),
            Wire::Literal(v) => Wire::Literal(Value::Float(v.as_float() * k)),
            w => self.math(MathOp::Multiply, w, Wire::Literal(Value::Float(k))),
        }
    }

    /// The shared Texture Coordinate node.
    fn coordinates(&mut self) -> NodeId {
        if let Some(node) = self.coordinates {
            return node;
        }
        let node = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::TextureCoordinate)));
        self.coordinates = Some(node);
        node
    }

    fn generated(&mut self) -> Wire {
        Wire::Link { node: self.coordinates(), output: 0, world: World::No }
    }

    fn uv(&mut self) -> Wire {
        Wire::Link { node: self.coordinates(), output: 2, world: World::No }
    }

    // ---- Nodes ------------------------------------------------------------------

    /// What output `socket` of flat node `id` gives.
    fn output(&mut self, id: usize, socket: &'a str) -> Option<Wire> {
        if let Some(w) = self.wires.get(&(id, socket)) {
            return w.clone();
        }
        let wire = self.build(id, socket);
        self.wires.insert((id, socket), wire.clone());
        wire
    }

    fn build(&mut self, id: usize, socket: &str) -> Option<Wire> {
        let node = self.flat[id].node;
        let kind = self.flat[id].kind();
        let out_value = |socket: &str| sockets(node, "outputs").iter().find(|s| id_of(s) == socket).and_then(typed);
        match kind {
            "ShaderNodeValue" | "ShaderNodeRGB" => out_value(socket).map(Wire::Literal),
            "ShaderNodeTexImage" => self.image(id, socket),
            "ShaderNodeValToRGB" => self.ramp(id, socket),
            "ShaderNodeRGBCurve" | "ShaderNodeFloatCurve" => self.curves(id),
            "ShaderNodeBump" => {
                let height = self.input(id, "Height")?;
                let strength = self.input(id, "Strength").and_then(|w| w.number()).unwrap_or(1.0).clamp(0.0, 1.0);
                let distance = self.input(id, "Distance").and_then(|w| w.number()).unwrap_or(1.0);
                if self.input_linked(id, "Strength") || self.input_linked(id, "Distance") {
                    self.note("a Bump's Strength or Distance is driven by nodes; it came over at the values typed in".into());
                }
                let invert = node.get("settings").and_then(|s| s.get("invert")).and_then(Json::as_bool).unwrap_or(false);
                let depth = strength * distance * if invert { -1.0 } else { 1.0 };
                let under = self.input(id, "Normal").filter(|w| !matches!(w, Wire::Literal(_) | Wire::Link { .. })).map(Box::new);
                Some(Wire::Bump { height: Box::new(height), depth, under })
            }
            "ShaderNodeDisplacement" => {
                let height = self.input(id, "Height")?;
                let scale = self.input(id, "Scale").and_then(|w| w.number()).unwrap_or(1.0);
                Some(Wire::Bump { height: Box::new(height), depth: scale, under: None })
            }
            "ShaderNodeNormalMap" => {
                let strength = self.input(id, "Strength").and_then(|w| w.number()).unwrap_or(1.0);
                if (strength - 1.0).abs() > 1e-3 {
                    self.note(format!("a Normal Map's strength of {strength:.2} came over at full strength"));
                }
                if text(node, "space").is_some_and(|s| s != "TANGENT") {
                    self.note("an object- or world-space normal map was read as a tangent-space one".into());
                }
                let colour = sockets(node, "inputs").iter().position(|s| id_of(s) == "Color").map(|i| self.flat[id].inputs[i]);
                match colour {
                    Some(Source::Output(from, _)) if self.flat[from].kind() == "ShaderNodeTexImage" => {
                        let (normal, _) = self.image_nodes(from, true)?;
                        Some(Wire::NormalMap { node: normal? })
                    }
                    Some(Source::Output(..)) => {
                        self.note("a Normal Map fed by something other than an image was left out".into());
                        None
                    }
                    _ => None,
                }
            }
            "ShaderNodeBevel" => {
                let radius = self.input(id, "Radius").and_then(|w| w.number()).unwrap_or(0.05) * self.cx.unit_scale;
                let under = self.input(id, "Normal").filter(|w| !matches!(w, Wire::Literal(_) | Wire::Link { .. })).map(Box::new);
                Some(Wire::Bevel { radius, under })
            }
            "ShaderNodeTangent" => self.tangent(id),
            "ShaderNodeRGBToBW" => {
                let colour = self.input(id, "Color")?;
                Some(self.luminance(colour))
            }
            "ShaderNodeVertexColor" | "ShaderNodeAttribute" => {
                let wants_alpha = socket == "Alpha";
                if kind == "ShaderNodeAttribute" && text(node, "attribute_type").is_some_and(|t| t != "GEOMETRY") {
                    let label = self.label(id);
                    self.note(format!("the {label} attribute is an object's or a layer's, which Tequila does not have; it read zero"));
                    return None;
                }
                if wants_alpha {
                    return Some(Wire::Literal(Value::Float(1.0)));
                }
                let colour = Wire::link(self.graph.add_node(Node::new(NodeKind::VertexColor)));
                if socket == "Fac" {
                    return Some(self.luminance(colour));
                }
                if socket == "Vector" {
                    self.note("an attribute read as a vector came over as the mesh's colours".into());
                }
                Some(colour)
            }
            "ShaderNodeUVMap" => {
                if text(node, "uv_map").is_some_and(|m| !m.is_empty()) {
                    self.note("a UV Map node naming a map came over as the first one".into());
                }
                Some(self.uv())
            }
            "ShaderNodeAmbientOcclusion" => {
                let radius = self.input(id, "Distance").and_then(|w| w.number()).unwrap_or(1.0) * self.cx.unit_scale;
                let occlusion = self.graph.add_node(Node::new(NodeKind::Occlusion { radius: radius.max(1e-5), contrast: 1.0 }));
                // Blender's is how open the point is; Tequila's how closed.
                let open = self.graph.add_node(Node::new(NodeKind::Invert));
                self.graph.connect(occlusion, open, Socket::Input(0));
                if socket == "AO" {
                    return Some(Wire::link(open));
                }
                let colour = self.input(id, "Color").unwrap_or(Wire::Literal(Value::Color(Spectrum::ONE)));
                let product = self.graph.add_node(Node::new(NodeKind::Multiply));
                self.feed(&colour, product, Socket::Input(0), SocketType::Color, false);
                self.graph.connect(open, product, Socket::Input(1));
                Some(Wire::link(product))
            }
            _ => self.shader_node_output(id, socket),
        }
    }

    /// One of Blender's nodes that Tequila has, and the wire out of output
    /// `socket`.
    fn shader_node_output(&mut self, id: usize, socket: &str) -> Option<Wire> {
        let node = self.flat[id].node;
        let kind = self.flat[id].kind();
        let Some((graph_node, shader)) = self.shader_node(id) else {
            let label = self.label(id);
            if kind.starts_with("ShaderNodeBsdf") || matches!(kind, "ShaderNodeEmission" | "ShaderNodeMixShader" | "ShaderNodeAddShader") {
                self.note(format!("the {label} shader is wired where a value goes; it read as nothing"));
            } else {
                self.note(format!("the {label} node ({kind}) has no counterpart; what it fed kept its own value"));
            }
            return None;
        };
        let entry = sockets(node, "outputs").iter().find(|s| id_of(s) == socket)?;
        let outputs = shader.outputs();
        let candidates = names_for(kind, entry);
        let wanted = candidates.iter().copied().find(|n| outputs.iter().any(|o| o.name == *n)).unwrap_or(name_of(entry));
        let Some(index) = outputs.iter().position(|o| o.name == wanted) else {
            let label = self.label(id);
            self.note(format!("the {label} node's {wanted} output has no counterpart; it read zero"));
            return Some(Wire::Literal(Value::Float(0.0)));
        };
        let world = match (shader, wanted) {
            (ShaderNode::Geometry, "Position") | (ShaderNode::ObjectInfo, "Location") => World::Position,
            (ShaderNode::Geometry, "Normal" | "True Normal" | "Incoming") | (ShaderNode::TextureCoordinate, "Reflection") => World::Direction,
            _ => World::No,
        };
        let wire = Wire::Link { node: graph_node, output: index as u8, world };
        // Tequila's object space is in metres; Blender's in its own units.
        if shader == ShaderNode::TextureCoordinate && wanted == "Object" && (self.cx.unit_scale - 1.0).abs() > 1e-6 {
            let mut scale = Node::new(NodeKind::Node(ShaderNode::VectorMath { op: VectorMathOp::Scale }));
            scale.literals.insert(Socket::Input(3), Value::Float(1.0 / self.cx.unit_scale));
            let scaled = self.graph.add_node(scale);
            self.graph.connect_output(graph_node, index as u8, scaled, Socket::Input(0));
            return Some(Wire::link(scaled));
        }
        Some(wire)
    }

    /// The Tequila node for flat node `id`, its inputs wired.
    fn shader_node(&mut self, id: usize) -> Option<(NodeId, ShaderNode)> {
        if let Some(&found) = self.nodes.get(&id) {
            return Some(found);
        }
        let node = self.flat[id].node;
        let kind = self.flat[id].kind();
        let shader = ShaderNode::from_blender(kind, &|key| setting(node, key))?;
        if kind == "ShaderNodeMix" && text(node, "factor_mode") == Some("NON_UNIFORM") {
            self.note("a Mix of vectors by a vector factor came over mixed by its average".into());
        }
        if node.get("texture_mapping").and_then(|m| m.get("default")).and_then(Json::as_bool) == Some(false) {
            let label = self.label(id);
            self.note(format!("the {label} node's own mapping (in its sidebar) was left out"));
        }
        let graph_node = self.graph.add_node(Node::new(NodeKind::Node(shader)));
        self.nodes.insert(id, (graph_node, shader));
        let specs = shader.inputs();
        let blender = sockets(node, "inputs");
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (j, input) in blender.iter().enumerate() {
            if !enabled(input) {
                continue;
            }
            // By any of the names Blender has for it that Tequila's node
            // uses; a name used twice (Math's Values) by which one it is.
            let Some(name) = names_for(kind, input).into_iter().find(|n| specs.iter().any(|s| s.name == *n)) else { continue };
            let k = *seen.entry(name).and_modify(|k| *k += 1).or_insert(0);
            let Some(t) = (0..specs.len()).filter(|&i| specs[i].name == name && shader.shows(i)).nth(k) else { continue };
            let spec = specs[t];
            let source = self.flat[id].inputs[j];
            let texture_vector = kind.starts_with("ShaderNodeTex") && spec.cycles == "vector";
            let wire = match source {
                Source::Output(from, socket) => self.output(from, socket),
                // Blender's textures read Generated coordinates when nothing
                // is wired; Tequila's would read object space.
                _ if texture_vector => Some(self.generated()),
                // An unwired normal is the shading normal, which is what
                // Tequila's own reads too.
                _ if spec.ty == SocketType::Normal => None,
                Source::Typed(entry) => typed(entry).map(Wire::Literal),
                Source::Nothing => None,
            };
            let wire = match wire {
                Some(w) => w,
                None => match source {
                    // Unreadable: the value typed into the socket.
                    Source::Output(..) => match typed(input) {
                        Some(v) => Wire::Literal(v),
                        None => continue,
                    },
                    _ => continue,
                },
            };
            if matches!(wire, Wire::Bump { .. } | Wire::NormalMap { .. } | Wire::Bevel { .. }) {
                if spec.ty != SocketType::Normal {
                    self.note("a bump's normal reached a socket that takes a value and was left out".into());
                }
                continue;
            }
            self.feed(&wire, graph_node, Socket::Input(t as u8), spec.ty, spec.implicit.is_some());
        }
        Some((graph_node, shader))
    }

    fn luminance(&mut self, colour: Wire) -> Wire {
        if let Some(v) = colour.literal() {
            return Wire::Literal(Value::Float(v.as_color().luminance()));
        }
        // Tequila's colours are ACEScg: its luminance weights.
        let split = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateColor { model: tequila_texture::cycles::ops::ColorModel::Rgb })));
        self.feed(&colour, split, Socket::Input(0), SocketType::Color, false);
        let channel = |output: u8| Wire::Link { node: split, output, world: World::No };
        let b = self.scaled(channel(2), 0.053_689_5);
        let g = self.multiply_add(channel(1), 0.674_081_8, b);
        self.multiply_add(channel(0), 0.272_228_7, g)
    }

    fn multiply_add(&mut self, a: Wire, k: f32, c: Wire) -> Wire {
        let node = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::Math { op: MathOp::MultiplyAdd, clamp: false })));
        self.feed(&a, node, Socket::Input(0), SocketType::Float, false);
        self.feed(&Wire::Literal(Value::Float(k)), node, Socket::Input(1), SocketType::Float, false);
        self.feed(&c, node, Socket::Input(2), SocketType::Float, false);
        Wire::link(node)
    }

    /// Blender's Tangent node: radial about an axis, in the object's own
    /// space, as Cycles works it out from Generated coordinates; along the
    /// UVs, which is what Tequila does with no tangent wired.
    fn tangent(&mut self, id: usize) -> Option<Wire> {
        let node = self.flat[id].node;
        if text(node, "direction_type") == Some("UV_MAP") {
            return None;
        }
        let generated = self.generated();
        let split = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateXyz)));
        self.feed(&generated, split, Socket::Input(0), SocketType::Vector, false);
        let c = |output: u8| Wire::Link { node: split, output, world: World::No };
        let half = Wire::Literal(Value::Float(0.5));
        let zero = Wire::Literal(Value::Float(0.0));
        let (x, y, z) = match text(node, "axis") {
            Some("X") => (zero, self.math(MathOp::Subtract, half.clone(), c(2)), self.math(MathOp::Subtract, c(1), half)),
            Some("Y") => (self.math(MathOp::Subtract, half.clone(), c(0)), zero, self.math(MathOp::Subtract, c(2), half)),
            _ => (self.math(MathOp::Subtract, half.clone(), c(1)), self.math(MathOp::Subtract, c(0), half), zero),
        };
        let combine = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::CombineXyz)));
        for (i, part) in [x, y, z].iter().enumerate() {
            self.feed(part, combine, Socket::Input(i as u8), SocketType::Float, false);
        }
        Some(Wire::link(combine))
    }

    /// A Color Ramp: Tequila's ramp, and its alpha as a ramp of greys.
    fn ramp(&mut self, id: usize, socket: &str) -> Option<Wire> {
        let node = self.flat[id].node;
        let ramp = node.get("ramp")?;
        let interpolation = match ramp.get("interpolation").and_then(Json::as_str) {
            Some("CONSTANT") => Interpolation::Constant,
            Some("EASE") => Interpolation::Ease,
            Some("B_SPLINE") | Some("CARDINAL") => {
                self.note("a Color Ramp's spline came over eased".into());
                Interpolation::Ease
            }
            _ => Interpolation::Linear,
        };
        if ramp.get("color_mode").and_then(Json::as_str).is_some_and(|m| m != "RGB") {
            self.note("a Color Ramp blending in HSV or HSL came over blending in RGB".into());
        }
        let alpha = socket == "Alpha";
        let stops: Vec<Stop> = ramp
            .get("stops")
            .map(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|s| {
                let at = s.index(0)?.as_f32()?;
                let c = s.index(1)?.as_f32_array()?;
                let color = if alpha { Spectrum::splat(c.get(3).copied().unwrap_or(1.0)) } else { linear(&c) };
                Some(Stop { at, color })
            })
            .collect();
        let gradient = Gradient::new(stops).with_interpolation(interpolation);
        let graph_node = self.graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(gradient) }));
        let fac = self.input(id, "Fac").unwrap_or(Wire::Literal(Value::Float(0.5)));
        self.feed(&fac, graph_node, Socket::Input(0), SocketType::Float, false);
        Some(Wire::link(graph_node))
    }

    /// RGB Curves and Float Curve, as ramps of greys read from the curves
    /// Blender drew.
    fn curves(&mut self, id: usize) -> Option<Wire> {
        let node = self.flat[id].node;
        let kind = self.flat[id].kind();
        let curves = node.get("curves")?;
        let range = curves.get("range").and_then(Json::as_f32_array).filter(|r| r.len() == 2 && r[1] > r[0]).unwrap_or_else(|| vec![0.0, 1.0]);
        let samples: Vec<Vec<f32>> = curves.get("curves").map(Json::as_array).unwrap_or(&[]).iter().filter_map(Json::as_f32_array).filter(|s| s.len() >= 2).collect();
        let read = |s: &[f32], x: f32| -> f32 {
            let t = ((x - range[0]) / (range[1] - range[0])).clamp(0.0, 1.0) * (s.len() - 1) as f32;
            let i = (t.floor() as usize).min(s.len() - 2);
            s[i] + (s[i + 1] - s[i]) * (t - i as f32)
        };
        let steps = samples.first().map(Vec::len).unwrap_or(0);
        if steps < 2 {
            return None;
        }
        let ramp = |t: &mut Self, ys: Vec<f32>, input: Wire| -> Wire {
            let stops = ys.iter().enumerate().map(|(i, y)| Stop { at: i as f32 / (ys.len() - 1) as f32, color: Spectrum::splat(*y) }).collect();
            let gradient = t.graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(Gradient::new(stops)) }));
            let position = if range[0] == 0.0 && range[1] == 1.0 {
                input
            } else {
                let remap = t.graph.add_node(Node::new(NodeKind::Remap { from: tequila_core::math::vec2(range[0], range[1]), to: tequila_core::math::vec2(0.0, 1.0) }));
                t.feed(&input, remap, Socket::Input(0), SocketType::Color, false);
                Wire::link(remap)
            };
            t.feed(&position, gradient, Socket::Input(0), SocketType::Float, false);
            Wire::link(gradient)
        };
        let xs: Vec<f32> = (0..steps).map(|i| range[0] + (range[1] - range[0]) * i as f32 / (steps - 1) as f32).collect();
        let (curved, original, factor) = if kind == "ShaderNodeFloatCurve" {
            let value = self.input(id, "Value").unwrap_or(Wire::Literal(Value::Float(0.0)));
            let ys = xs.iter().map(|&x| read(&samples[0], x)).collect();
            (ramp(self, ys, value.clone()), value, self.input(id, "Factor"))
        } else {
            if samples.len() < 4 {
                return None;
            }
            // Blender's combined curve first, then each channel's own.
            let colour = self.input(id, "Color").unwrap_or(Wire::Literal(Value::Color(Spectrum::ONE)));
            let split = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateColor { model: tequila_texture::cycles::ops::ColorModel::Rgb })));
            self.feed(&colour, split, Socket::Input(0), SocketType::Color, false);
            let combine = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::CombineColor { model: tequila_texture::cycles::ops::ColorModel::Rgb })));
            for channel in 0..3u8 {
                let ys = xs.iter().map(|&x| read(&samples[channel as usize], read(&samples[3], x))).collect();
                let out = ramp(self, ys, Wire::Link { node: split, output: channel, world: World::No });
                self.feed(&out, combine, Socket::Input(channel), SocketType::Float, false);
            }
            (Wire::link(combine), colour, self.input(id, "Fac"))
        };
        match factor {
            Some(f) if f.number().is_some_and(|f| f >= 1.0) => Some(curved),
            None => Some(curved),
            Some(f) => {
                let mix = self.graph.add_node(Node::new(NodeKind::Mix));
                self.feed(&original, mix, Socket::Input(0), SocketType::Color, false);
                self.feed(&curved, mix, Socket::Input(1), SocketType::Color, false);
                self.feed(&f, mix, Socket::Input(2), SocketType::Float, false);
                Some(Wire::link(mix))
            }
        }
    }

    // ---- Images -------------------------------------------------------------

    fn image(&mut self, id: usize, socket: &str) -> Option<Wire> {
        let (colour, alpha) = self.image_nodes(id, false)?;
        let wanted = if socket == "Alpha" { alpha } else { colour };
        Some(wanted.map(Wire::link).unwrap_or(Wire::Literal(Value::Float(if socket == "Alpha" { 1.0 } else { 0.5 }))))
    }

    /// An Image Texture's colour and alpha, as nodes, each where something
    /// is wired to it; `normal` reads the picture as a normal map.
    fn image_nodes(&mut self, id: usize, normal: bool) -> Option<(Option<NodeId>, Option<NodeId>)> {
        if let Some(&found) = self.images.get(&(id, normal)) {
            return Some(found);
        }
        let node = self.flat[id].node;
        let Some(image) = node.get("image").and_then(Json::as_index).and_then(|i| self.cx.images.get(i)) else {
            let label = self.label(id);
            self.note(format!("the image texture {label} has no image"));
            return None;
        };
        let space = image.get("colorspace").and_then(Json::as_str).unwrap_or("sRGB");
        let role = if normal {
            MapRole::Normal
        } else if space.contains("sRGB") || space == "Display P3" {
            MapRole::Color
        } else {
            MapRole::Data
        };
        let texture = self.texture(image, role)?;
        if let Some(p) = text(node, "projection").filter(|p| *p != "FLAT") {
            self.note(format!("an image projected {} came over projected flat", p.to_lowercase()));
        }
        let extension = text(node, "extension").unwrap_or("REPEAT");
        let clip = extension == "CLIP";
        // Blender's Color is premultiplied unless the Alpha is used.
        let used = |output: &str| sockets(node, "outputs").iter().any(|s| id_of(s) == output && s.get("linked").and_then(Json::as_bool).unwrap_or(false));
        let alpha_used = used("Alpha");
        let colour_used = normal || used("Color");
        let has_alpha = image.get("channels").and_then(Json::as_index).unwrap_or(4) == 4 && image.get("alpha_mode").and_then(Json::as_str).unwrap_or("STRAIGHT") != "NONE";
        let premultiply = colour_used && !normal && has_alpha && !alpha_used && role == MapRole::Color;
        let wants_alpha = !normal && (alpha_used || premultiply);
        let place = sockets(node, "inputs").iter().position(|s| id_of(s) == "Vector");
        let source = place.map(|i| self.flat[id].inputs[i]).unwrap_or(Source::Nothing);
        // Through the mesh's UVs, where Blender reads them: nothing wired,
        // or the UVs themselves.
        let through_uvs = match source {
            Source::Output(from, socket) => {
                let k = self.flat[from].kind();
                (k == "ShaderNodeTexCoord" && socket == "UV") || k == "ShaderNodeUVMap"
            }
            _ => true,
        };
        let fallback_colour = Value::Color(Spectrum::splat(0.5));
        let colour = colour_used.then(|| self.graph.add_node(Node::new(NodeKind::ImageTexture { texture, fallback: fallback_colour })));
        let alpha = wants_alpha.then(|| self.graph.add_node(Node::new(NodeKind::ImageAlpha { texture, fallback: Value::Float(1.0) })));
        let place = if through_uvs {
            let wrap = match extension {
                "EXTEND" | "CLIP" => Wrap::Clamp,
                "MIRROR" => Wrap::Mirror,
                _ => Wrap::Repeat,
            };
            let mapping = Mapping { projection: Projection::Uv, uv_set: 0, wrap_u: wrap, wrap_v: wrap, ..Mapping::default() };
            let coords = self.graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
            for n in colour.into_iter().chain(alpha) {
                self.graph.connect(coords, n, Socket::Input(0));
            }
            clip.then(|| self.uv())
        } else {
            let place = match source {
                Source::Output(from, socket) => self.output(from, socket).unwrap_or_else(|| Wire::Literal(Value::Vector(Vec3::ZERO))),
                _ => Wire::Literal(Value::Vector(Vec3::ZERO)),
            };
            let at = match extension {
                "EXTEND" => self.per_axis(&place, |t, w| {
                    let clamp = t.graph.add_node(Node::new(NodeKind::Node(ShaderNode::Clamp { range: false })));
                    t.feed(&w, clamp, Socket::Input(0), SocketType::Float, false);
                    t.feed(&Wire::Literal(Value::Float(0.0)), clamp, Socket::Input(1), SocketType::Float, false);
                    t.feed(&Wire::Literal(Value::Float(1.0)), clamp, Socket::Input(2), SocketType::Float, false);
                    Wire::link(clamp)
                }),
                "MIRROR" => self.per_axis(&place, |t, w| t.math(MathOp::PingPong, w, Wire::Literal(Value::Float(1.0)))),
                _ => place.clone(),
            };
            for n in colour.into_iter().chain(alpha) {
                self.feed(&at, n, Socket::Input(0), SocketType::Vector, false);
            }
            Some(place)
        };
        let (mut colour, mut alpha) = (colour, alpha);
        if let (true, Some(c), Some(a)) = (premultiply, colour, alpha) {
            let product = self.graph.add_node(Node::new(NodeKind::Multiply));
            self.graph.connect(c, product, Socket::Input(0));
            self.graph.connect(a, product, Socket::Input(1));
            colour = Some(product);
        }
        // Clip: nothing outside the picture.
        if let (true, Some(place)) = (clip, place) {
            let inside = self.inside(&place);
            if let Some(c) = colour {
                let product = self.graph.add_node(Node::new(NodeKind::Multiply));
                self.graph.connect(c, product, Socket::Input(0));
                self.feed(&inside, product, Socket::Input(1), SocketType::Color, false);
                colour = Some(product);
            }
            if let Some(a) = alpha {
                if let Wire::Link { node, .. } = self.math(MathOp::Multiply, Wire::link(a), inside) {
                    alpha = Some(node);
                }
            }
        }
        self.images.insert((id, normal), (colour, alpha));
        Some((colour, alpha))
    }

    /// `f` applied to x and y of `place`, z kept.
    fn per_axis(&mut self, place: &Wire, f: impl Fn(&mut Self, Wire) -> Wire) -> Wire {
        let split = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateXyz)));
        self.feed(place, split, Socket::Input(0), SocketType::Vector, false);
        let x = f(self, Wire::Link { node: split, output: 0, world: World::No });
        let y = f(self, Wire::Link { node: split, output: 1, world: World::No });
        let combine = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::CombineXyz)));
        self.feed(&x, combine, Socket::Input(0), SocketType::Float, false);
        self.feed(&y, combine, Socket::Input(1), SocketType::Float, false);
        self.graph.connect_output(split, 2, combine, Socket::Input(2));
        Wire::link(combine)
    }

    /// 1 where x and y of `place` are both within 0 to 1, else 0.
    fn inside(&mut self, place: &Wire) -> Wire {
        let split = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateXyz)));
        self.feed(place, split, Socket::Input(0), SocketType::Vector, false);
        let mut within = Vec::new();
        for axis in 0..2u8 {
            let centred = self.math(MathOp::Subtract, Wire::Link { node: split, output: axis, world: World::No }, Wire::Literal(Value::Float(0.5)));
            let far = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::Math { op: MathOp::Absolute, clamp: false })));
            self.feed(&centred, far, Socket::Input(0), SocketType::Float, false);
            within.push(self.math(MathOp::LessThan, Wire::link(far), Wire::Literal(Value::Float(0.500_001))));
        }
        let y = within.pop().expect("two axes");
        let x = within.pop().expect("two axes");
        self.math(MathOp::Multiply, x, y)
    }

    /// The image as one of the scene's textures, for `role`.
    fn texture(&mut self, image: &Json, role: MapRole) -> Option<TextureId> {
        let name = image.get("name").and_then(Json::as_str).unwrap_or("image");
        let wanted = if let Some(file) = image.get("file").and_then(Json::as_str) {
            let Some(bytes) = (self.cx.packed)(file) else {
                self.note(format!("the image {name} could not be read back from Blender"));
                return None;
            };
            RawTexture { uri: file.to_string(), role, embedded: Some(bytes), elsewhere: None }
        } else if let Some(path) = image.get("path").and_then(Json::as_str) {
            RawTexture { uri: path.to_string(), role, embedded: None, elsewhere: None }
        } else {
            // Blender's own warning names it once already.
            return None;
        };
        let index = self.textures.iter().position(|t| t.same_image(&wanted)).unwrap_or_else(|| {
            self.textures.push(wanted);
            self.textures.len() - 1
        });
        Some(TextureId(index as u32))
    }
}

/// The names a Blender socket may go by in Tequila's node, likeliest
/// first: what Blender shows, its identifier, which does not change between
/// versions (Blender 5 shows "Factor" for the "Fac" it always was), that
/// without the type or count Blender adds to tell sockets apart
/// (`Factor_Float`, `Value_001`), and a legacy node's own names.
fn names_for<'n>(kind: &str, socket: &'n Json) -> Vec<&'n str> {
    let (name, id) = (name_of(socket), id_of(socket));
    let mut out = vec![blender_input_name(kind, name), blender_output_name(kind, name), name, id];
    if let Some((stem, tail)) = id.rsplit_once('_') {
        if tail.chars().all(|c| c.is_ascii_digit()) || matches!(tail, "Float" | "Vector" | "Color" | "Rotation" | "Int") {
            out.push(stem);
        }
    }
    for n in out.clone() {
        match n {
            "Factor" => out.push("Fac"),
            "Fac" => out.push("Factor"),
            _ => {}
        }
    }
    out
}

/// Tequila's name for a Blender node's input, where a legacy node's
/// differs.
fn blender_input_name<'n>(kind: &str, name: &'n str) -> &'n str {
    match (kind, name) {
        ("ShaderNodeMixRGB", "Fac") => "Factor",
        ("ShaderNodeMixRGB", "Color1") => "A",
        ("ShaderNodeMixRGB", "Color2") => "B",
        ("ShaderNodeSeparateRGB" | "ShaderNodeSeparateHSV", "Image" | "Color") => "Color",
        ("ShaderNodeCombineRGB", "R") | ("ShaderNodeCombineHSV", "H") => "Red",
        ("ShaderNodeCombineRGB", "G") | ("ShaderNodeCombineHSV", "S") => "Green",
        ("ShaderNodeCombineRGB", "B") | ("ShaderNodeCombineHSV", "V") => "Blue",
        (_, name) => name,
    }
}

/// Tequila's name for a Blender node's output, where it differs.
fn blender_output_name<'n>(kind: &str, name: &'n str) -> &'n str {
    match (kind, name) {
        ("ShaderNodeMixRGB", "Color") => "Result",
        ("ShaderNodeSeparateRGB", "R") | ("ShaderNodeSeparateHSV", "H") => "Red",
        ("ShaderNodeSeparateRGB", "G") | ("ShaderNodeSeparateHSV", "S") => "Green",
        ("ShaderNodeSeparateRGB", "B") | ("ShaderNodeSeparateHSV", "V") => "Blue",
        ("ShaderNodeCombineRGB", "Image") | ("ShaderNodeCombineHSV", "Color") => "Color",
        (_, name) => name,
    }
}

// ---- The world ------------------------------------------------------------------

/// What a Blender world lights the scene with.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Surroundings {
    /// One colour all round, at a strength.
    Colour { colour: Spectrum, strength: f32 },
    /// An HDR picture all round, at a strength, turned about the vertical
    /// by Blender's Mapping node (radians, Blender's own sense).
    Picture { path: String, strength: f32, turn: f32 },
}

/// The world `entry` as Tequila can light with it: its Background's colour
/// or picture and strength. Anything else is said, and comes over as the
/// world's viewport colour.
pub(super) fn world(entry: &Json, cx: &Context<'_>, notes: &mut Notes) -> Surroundings {
    let viewport = Surroundings::Colour { colour: linear(&entry.get("color").and_then(Json::as_f32_array).unwrap_or_else(|| vec![0.05, 0.05, 0.05])), strength: 1.0 };
    let Some(tree) = entry.get("tree").and_then(Json::as_index).filter(|&t| t < cx.trees.len()) else { return viewport };
    let flat = flatten(cx.trees, tree, notes, "The world");
    let outputs: Vec<usize> = (0..flat.len()).filter(|&i| flat[i].kind() == "ShaderNodeOutputWorld").collect();
    let target = |i: usize| text(flat[i].node, "target").unwrap_or("ALL");
    let active = |i: usize| flat[i].node.get("active").and_then(Json::as_bool).unwrap_or(false);
    let Some(output) = outputs.iter().copied().find(|&i| active(i) && target(i) != "EEVEE").or_else(|| outputs.first().copied()) else { return viewport };
    let input = |node: usize, id: &str| -> Option<Source<'_>> {
        let i = sockets(flat[node].node, "inputs").iter().position(|s| id_of(s) == id)?;
        Some(flat[node].inputs[i])
    };
    let Some(Source::Output(shader, _)) = input(output, "Surface") else { return viewport };
    if !matches!(flat[shader].kind(), "ShaderNodeBackground" | "ShaderNodeEmission") {
        notes.add(format!("the world is made with a {} node, not a Background; it came over as its viewport colour", flat[shader].kind().trim_start_matches("ShaderNode")));
        return viewport;
    }
    let strength = match input(shader, "Strength") {
        Some(Source::Typed(s)) => typed(s).map(Value::as_float).unwrap_or(1.0),
        Some(Source::Output(..)) => {
            notes.add("the world's strength is worked out by nodes; it came over at 1".into());
            1.0
        }
        _ => 1.0,
    }
    .max(0.0);
    match input(shader, "Color") {
        Some(Source::Typed(c)) => Surroundings::Colour { colour: typed(c).map(Value::as_color).unwrap_or(Spectrum::ONE), strength },
        Some(Source::Output(texture, _)) if flat[texture].kind() == "ShaderNodeTexEnvironment" => {
            let image = flat[texture].node.get("image").and_then(Json::as_index).and_then(|i| cx.images.get(i));
            let Some(path) = image.and_then(|i| i.get("path")).and_then(Json::as_str) else {
                if image.and_then(|i| i.get("file")).is_some() {
                    notes.add("the world's picture is packed inside the .blend; unpack it in Blender (File › External Data › Unpack Resources) to bring it over".into());
                }
                return Surroundings::Colour { colour: Spectrum::splat(0.5), strength };
            };
            // Turned by a Mapping node between it and its coordinates.
            let mut turn = 0.0;
            if let Some(Source::Output(mapping, _)) = input(texture, "Vector") {
                if flat[mapping].kind() == "ShaderNodeMapping" {
                    if let Some(Source::Typed(r)) = input(mapping, "Rotation") {
                        if let Some(Value::Vector(r)) = typed(r) {
                            if r.x.abs() > 1e-4 || r.y.abs() > 1e-4 {
                                notes.add("the world's picture is tilted; only its turn about the vertical came over".into());
                            }
                            turn = if text(flat[mapping].node, "vector_type") == Some("TEXTURE") { -r.z } else { r.z };
                        }
                    }
                }
            }
            Surroundings::Picture { path: path.to_string(), strength, turn }
        }
        Some(Source::Output(other, _)) => {
            notes.add(format!("the world's colour comes from a {} node; it came over as its viewport colour", flat[other].kind().trim_start_matches("ShaderNode")));
            match viewport {
                Surroundings::Colour { colour, .. } => Surroundings::Colour { colour, strength },
                picture => picture,
            }
        }
        _ => Surroundings::Colour { colour: Spectrum::splat(0.05), strength },
    }
}
