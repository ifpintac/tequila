//! A small language for writing material graphs as code: Blender's nodes by
//! their names, inputs by their labels, colours in sRGB as a person picks
//! them.

use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, Value};
use tequila_materials::material_type::Param;
use tequila_materials::nodes::{Setting, ShaderNode};
use tequila_materials::openpbr::Material;
use tequila_texture::gradient::{Gradient, Interpolation, Stop};

/// A colour from its sRGB hex code, as a swatch shows it, in the rendering
/// space.
pub fn srgb(hex: &str) -> Spectrum {
    let hex = hex.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex colour") as f32 / 255.0;
    let linear = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    Spectrum::from_linear_srgb(tequila_core::math::vec3(linear(channel(0)), linear(channel(2)), linear(channel(4))))
}

/// One of a node's outputs.
#[derive(Clone, Copy, Debug)]
pub struct Out(pub NodeId, pub u8);

/// A value for an input: a number or a wire.
#[derive(Clone, Copy)]
pub enum In {
    V(f32),
    O(Out),
}

impl From<f32> for In {
    fn from(v: f32) -> In {
        In::V(v)
    }
}

impl From<Out> for In {
    fn from(o: Out) -> In {
        In::O(o)
    }
}

pub trait IntoSetting {
    fn setting(self) -> Setting;
}
impl IntoSetting for &'static str {
    fn setting(self) -> Setting {
        Setting::Choice(self)
    }
}
impl IntoSetting for i32 {
    fn setting(self) -> Setting {
        Setting::Int(self)
    }
}
impl IntoSetting for f32 {
    fn setting(self) -> Setting {
        Setting::Float(self)
    }
}
impl IntoSetting for bool {
    fn setting(self) -> Setting {
        Setting::Bool(self)
    }
}

pub struct Recipe {
    pub graph: MaterialGraph,
}

/// Input `name` of `node`, by its label; "Value 2" for the second "Value".
fn input_index(node: &ShaderNode, name: &str) -> usize {
    let (label, nth) = match name.rsplit_once(' ') {
        Some((label, n)) if n.chars().all(|c| c.is_ascii_digit()) => (label, n.parse::<usize>().unwrap()),
        _ => (name, 1),
    };
    node.inputs()
        .iter()
        .enumerate()
        .filter(|(_, spec)| spec.name == label)
        .nth(nth - 1)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("{} has no input {name}: {:?}", node.label(), node.inputs().iter().map(|s| s.name).collect::<Vec<_>>()))
}

pub struct Builder<'a> {
    recipe: &'a mut Recipe,
    node: ShaderNode,
    literals: Vec<(usize, Value)>,
    wires: Vec<(usize, Out)>,
}

impl Builder<'_> {
    pub fn set(mut self, key: &str, value: impl IntoSetting) -> Self {
        let setting = value.setting();
        assert!(self.node.set(key, setting), "{} has no setting {key} = {setting:?}", self.node.label());
        self
    }

    pub fn f(mut self, name: &str, value: impl Into<In>) -> Self {
        let index = input_index(&self.node, name);
        match value.into() {
            In::V(v) => self.literals.push((index, Value::Float(v))),
            In::O(o) => self.wires.push((index, o)),
        }
        self
    }

    pub fn c(mut self, name: &str, colour: Spectrum) -> Self {
        let index = input_index(&self.node, name);
        self.literals.push((index, Value::Color(colour)));
        self
    }

    pub fn v(mut self, name: &str, v: [f32; 3]) -> Self {
        let index = input_index(&self.node, name);
        self.literals.push((index, Value::Vector(tequila_core::math::vec3(v[0], v[1], v[2]))));
        self
    }

    pub fn w(mut self, name: &str, from: Out) -> Self {
        let index = input_index(&self.node, name);
        self.wires.push((index, from));
        self
    }

    pub fn add(self) -> NodeId {
        let Builder { recipe, node, literals, wires } = self;
        let mut built = Node::new(NodeKind::Node(node));
        for (index, value) in literals {
            built.literals.insert(Socket::Input(index as u8), value);
        }
        let id = recipe.graph.add_node(built);
        for (index, from) in wires {
            recipe.graph.connect_output(from.0, from.1, id, Socket::Input(index as u8));
        }
        id
    }

    /// Add it and take output `name`.
    pub fn out(self, name: &str) -> Out {
        let node = self.node;
        let id = self.add();
        let index = node.outputs().iter().position(|o| o.name == name).unwrap_or_else(|| panic!("{} has no output {name}", node.label()));
        Out(id, index as u8)
    }
}

impl Recipe {
    pub fn new(material: Material) -> Recipe {
        Recipe { graph: MaterialGraph::from_material(material) }
    }

    pub fn node(&mut self, key: &str) -> Builder<'_> {
        let node = ShaderNode::from_key(key).unwrap_or_else(|| panic!("no node {key}"));
        Builder { recipe: self, node, literals: Vec::new(), wires: Vec::new() }
    }

    /// Wire into one of the surface's parameters.
    pub fn to(&mut self, param: Param, from: Out) {
        let output = self.graph.output();
        self.graph.connect_output(from.0, from.1, output, Socket::Param(param));
    }

    /// A colour ramp over `input`.
    pub fn ramp(&mut self, input: Out, interpolation: Interpolation, stops: &[(f32, Spectrum)]) -> Out {
        let gradient = Gradient::new(stops.iter().map(|(at, color)| Stop { at: *at, color: *color }).collect()).with_interpolation(interpolation);
        let id = self.graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(gradient) }));
        self.graph.connect_output(input.0, input.1, id, Socket::Input(0));
        Out(id, 0)
    }

    /// A height into the surface's bump, `depth` metres for its 1.
    pub fn bump(&mut self, height: Out, depth: f32) {
        let id = self.graph.add_node(Node::new(NodeKind::Bump { depth }));
        self.graph.connect_output(height.0, height.1, id, Socket::Input(0));
        self.to(Param::Bump, Out(id, 0));
    }

    /// `value` from one range to another, clamped.
    pub fn range(&mut self, value: impl Into<In>, from: (f32, f32), to: (f32, f32)) -> Out {
        self.node("map_range").f("Value", value).f("From Min", from.0).f("From Max", from.1).f("To Min", to.0).f("To Max", to.1).out("Result")
    }

    /// The same, eased at both ends.
    pub fn smooth(&mut self, value: impl Into<In>, from: (f32, f32), to: (f32, f32)) -> Out {
        self.node("map_range")
            .set("interpolation", "smoothstep")
            .f("Value", value)
            .f("From Min", from.0)
            .f("From Max", from.1)
            .f("To Min", to.0)
            .f("To Max", to.1)
            .out("Result")
    }

    pub fn math(&mut self, op: &'static str, a: impl Into<In>, b: impl Into<In>) -> Out {
        self.node("math").set("operation", op).f("Value", a).f("Value 2", b).out("Value")
    }

    /// A math operation of three: `multiply_add`, `compare`, `wrap`.
    pub fn math3(&mut self, op: &'static str, a: impl Into<In>, b: impl Into<In>, c: impl Into<In>) -> Out {
        self.node("math").set("operation", op).f("Value", a).f("Value 2", b).f("Value 3", c).out("Value")
    }

    /// Mix two colours by a factor.
    pub fn mix(&mut self, blend: &'static str, factor: impl Into<In>, a: Out, b: Out) -> Out {
        self.node("mix").set("blend_type", blend).f("Factor", factor).w("A", a).w("B", b).out("Result")
    }

    /// Mix two numbers by a factor.
    pub fn mix_float(&mut self, factor: impl Into<In>, a: impl Into<In>, b: impl Into<In>) -> Out {
        self.node("mix").set("data_type", "float").f("Factor", factor).f("A", a).f("B", b).out("Result")
    }

    /// Object space, scaled along each axis: a pattern stretched along one
    /// is grain, brushing, fibre.
    pub fn stretched(&mut self, scale: [f32; 3]) -> Out {
        let coords = self.node("texture_coordinate").out("Object");
        self.node("mapping").w("Vector", coords).v("Scale", scale).out("Vector")
    }

    /// The material as it stands, for a parameter not wired.
    pub fn material_mut(&mut self) -> &mut Material {
        self.graph.material_mut()
    }
}

/// Gentle variation in how glossy a surface is - handling, polishing,
/// the way light catches any real part - between `low` and `high`,
/// changing over a few centimetres.
pub fn wear(r: &mut Recipe, low: f32, high: f32, scale: f32) -> Out {
    let noise = r.node("noise_texture").f("Scale", scale).f("Detail", 4.0).f("Roughness", 0.55).out("Fac");
    r.smooth(noise, (0.3, 0.7), (low, high))
}

#[allow(unused_imports)]
pub use tequila_texture::gradient::Interpolation::{Constant, Ease, Linear};
