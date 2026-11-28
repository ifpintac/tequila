//! The wear patterns (`tequila_texture::wear`) as Cycles nodes.
//!
//! Each pattern is written once, over `wear::Ops`; Tequila's renderer runs
//! it as numbers, and here each operation becomes the Cycles node it is
//! named for - Math, Vector Math, Map Range, Separate and Combine XYZ, White
//! Noise, Noise, Voronoi - so both renderers compute the same thing step by
//! step and a scratch lands on the same pixels in both. What is only
//! numbers (the settings, the keys) is worked out here and handed over as
//! constants, so it costs Cycles no nodes.
//!
//! Where the pattern reads the shading point: its place is the part's own
//! space in scene units, as every Tequila pattern reads it; its normal is
//! Texture Coordinate's Normal, the part's own space; which way it faces is
//! Geometry's Normal, world up; and the ray's footprint is Light Path's Ray
//! Length times the angle a pixel spans, which is how Tequila's renderer
//! measures it (`SceneSnapshot::shading_point_at`).
//!
//! # Room on the stack
//!
//! Scratches draw eight candidates per layer, some two hundred nodes each,
//! and Cycles' SVM has 255 places on its stack for everything a shader
//! holds at once. Two things about how Cycles (as Tequila ships it, Blender
//! 5.0.1's) compiles a graph decide whether that fits. It generates nodes in
//! sweeps in the order they were made, a node waiting for the next sweep if
//! anything it reads is not done yet; and a link between sockets of two
//! types gets a conversion node made then, after the node it feeds, so that
//! node waits a sweep. With a wait in each candidate every candidate gets as
//! far as its wait in one sweep, all eight holding what they have worked
//! out, and the shader runs out of room. So every conversion here is made
//! before the node it feeds ([`Nodes::as_kind`]), and the graph is
//! generated in one sweep, a candidate at a time. And White Noise keeps a
//! place for its Value whether it is read or not, for good, so the patterns
//! read every White Noise's Value as well as its Color.

use std::collections::HashMap;

use tequila_core::math::Vec3;
use tequila_materials::graph::Value;
use tequila_materials::nodes::PointInput;
use tequila_texture::procedural::Procedural;
use tequila_texture::projection::{Mapping, Projection as TextureProjection};
use tequila_texture::wear::{self, Eval, Ops, WearPoint};

use super::{Builder, Slot};
use crate::{Node, Shader};

/// A vector: worked out here, or a node's output.
#[derive(Clone, Copy, Debug)]
pub(super) enum Vector {
    Known(Vec3),
    Link(Node, &'static str),
}

/// What a socket holds, as Cycles types it: links between two kinds take a
/// conversion node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Vector,
    Point,
    Normal,
    Color,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Vector => "vector",
            Kind::Point => "point",
            Kind::Normal => "normal",
            Kind::Color => "color",
        }
    }

    /// A conversion node's socket of this kind.
    fn socket(self) -> &'static str {
        match self {
            Kind::Vector => "value_vector",
            Kind::Point => "value_point",
            Kind::Normal => "value_normal",
            Kind::Color => "value_color",
        }
    }

    /// The kind of the output `name` of the nodes these patterns read.
    fn of_output(name: &str) -> Kind {
        match name {
            "Color" | "value_color" => Kind::Color,
            "Normal" | "value_normal" => Kind::Normal,
            "Object" | "Position" | "UV" | "Generated" | "value_point" => Kind::Point,
            _ => Kind::Vector,
        }
    }
}

/// The patterns' arithmetic, as nodes of one shader.
struct Nodes<'b, 'a> {
    builder: &'b mut Builder<'a>,
    shader: Shader,
    /// Read through a mapping: on its plane, facing out of it, with no
    /// footprint, as Tequila's renderer reads a wear pattern there
    /// (`Procedural::evaluate_uv`).
    mapping: Option<&'b Mapping>,
    /// Each vector's Separate XYZ, made once.
    split: HashMap<(Node, &'static str), Node>,
    /// Each vector as another kind, converted once.
    converted: HashMap<(Node, &'static str, Kind), Node>,
    /// Outputs whose kind their name does not say.
    kinds: HashMap<(Node, &'static str), Kind>,
    /// The first thing that went wrong: the arithmetic carries on with
    /// stand-ins, and this is what the pattern answers.
    failed: Option<String>,
}

/// For folding: the same arithmetic as numbers.
fn numbers() -> Eval {
    Eval::new(WearPoint::flat(Vec3::ZERO))
}

fn known(slot: Slot) -> Option<f32> {
    match slot {
        Slot::Value(v) => Some(v.as_float()),
        Slot::Link(..) => None,
    }
}

impl Nodes<'_, '_> {
    fn check<T>(&mut self, result: Result<T, String>, fallback: T) -> T {
        match result {
            Ok(value) => value,
            Err(why) => {
                self.failed.get_or_insert(why);
                fallback
            }
        }
    }

    fn zero(&self) -> Slot {
        Slot::Value(Value::Float(0.0))
    }

    /// A Math node of `op`.
    fn math(&mut self, op: &str, a: Slot, b: Slot, c: Slot) -> Slot {
        let result = self.builder.float_op3(self.shader, op, a, b, c);
        self.check(result, Slot::Value(Value::Float(0.0)))
    }

    /// Folded where every input is a number, else a Math node.
    fn scalar(&mut self, op: &str, inputs: [Slot; 3], fold: impl FnOnce(&mut Eval, f32, f32, f32) -> f32) -> Slot {
        if let (Some(a), Some(b), Some(c)) = (known(inputs[0]), known(inputs[1]), known(inputs[2])) {
            return Slot::Value(Value::Float(fold(&mut numbers(), a, b, c)));
        }
        self.math(op, inputs[0], inputs[1], inputs[2])
    }

    /// Feed `vector` to `socket` of `node`.
    fn feed_vector(&mut self, node: Node, socket: &str, vector: Vector) {
        let result = match vector {
            Vector::Known(v) => self.builder.session.set_float3(node, socket, [v.x, v.y, v.z]),
            Vector::Link(from, output) => self.builder.session.connect(self.shader, from, output, node, socket),
        };
        self.check(result, ());
    }

    fn node(&mut self, kind: &str) -> Option<Node> {
        let result = self.builder.session.node(self.shader, kind).map(Some);
        self.check(result, None)
    }

    /// A Vector Math node of `op` on up to three vectors and a scale, its
    /// Vector output (or its Value, for those that give a number).
    fn vector_node(&mut self, op: &str, vectors: &[Vector], scale: Option<Slot>) -> Option<Node> {
        let vectors: Vec<Vector> = vectors.iter().map(|v| self.as_kind(*v, Kind::Vector)).collect();
        let node = self.node("vector_math")?;
        let result = self.builder.session.set_string(node, "math_type", op);
        self.check(result, ());
        for (vector, socket) in vectors.iter().zip(["Vector1", "Vector2", "Vector3"]) {
            self.feed_vector(node, socket, *vector);
        }
        if let Some(scale) = scale {
            let result = self.builder.feed(self.shader, node, "Scale", scale, false);
            self.check(result, ());
        }
        Some(node)
    }

    fn vector_op(&mut self, op: &str, vectors: &[Vector], scale: Option<Slot>) -> Vector {
        match self.vector_node(op, vectors, scale) {
            Some(node) => Vector::Link(node, "Vector"),
            None => Vector::Known(Vec3::ZERO),
        }
    }

    fn vector_value(&mut self, op: &str, vectors: &[Vector]) -> Slot {
        match self.vector_node(op, vectors, None) {
            Some(node) => Slot::Link(node, "Value"),
            None => self.zero(),
        }
    }

    /// The node and output a vector comes from, made into one if it is a
    /// number.
    fn linked(&mut self, v: Vector) -> Option<(Node, &'static str)> {
        match v {
            Vector::Link(node, output) => Some((node, output)),
            Vector::Known(v) => {
                let node = self.node("combine_xyz")?;
                for (socket, value) in [("X", v.x), ("Y", v.y), ("Z", v.z)] {
                    let result = self.builder.session.set_float(node, socket, value);
                    self.check(result, ());
                }
                Some((node, "Vector"))
            }
        }
    }

    fn point(&mut self, input: PointInput) -> Slot {
        let result = self.builder.point_input(self.shader, input);
        self.check(result, Slot::Value(Value::Float(0.0)))
    }
}

fn as_vector(slot: Slot) -> Vector {
    match slot {
        Slot::Value(v) => Vector::Known(v.as_vector()),
        Slot::Link(node, output) => Vector::Link(node, output),
    }
}

impl Ops for Nodes<'_, '_> {
    type S = Slot;
    type V = Vector;

    fn num(&mut self, x: f32) -> Slot {
        Slot::Value(Value::Float(x))
    }

    fn vector(&mut self, v: Vec3) -> Vector {
        Vector::Known(v)
    }

    fn position(&mut self) -> Vector {
        let result = self.builder.procedural_position(self.shader, self.mapping);
        match self.check(result.map(Some), None) {
            Some((node, output)) => {
                // Through a mapping, a Mapping node's Vector, which is a
                // point; else Vector Math's.
                self.kinds.insert((node, output), if self.mapped() { Kind::Point } else { Kind::Vector });
                Vector::Link(node, output)
            }
            None => Vector::Known(Vec3::ZERO),
        }
    }

    fn normal(&mut self) -> Vector {
        if self.mapped() {
            return Vector::Known(Vec3::Z);
        }
        let normal = self.point(PointInput::ObjectNormal);
        as_vector(normal)
    }

    fn up(&mut self) -> Slot {
        if self.mapped() {
            return Slot::Value(Value::Float(1.0));
        }
        // Cycles' world is Z up: the normal's z, as a dot product, which
        // (unlike Separate XYZ) keeps no place for what it does not give.
        let Some(geometry) = self.node("geometry") else { return self.zero() };
        self.dot(Vector::Link(geometry, "Normal"), Vector::Known(Vec3::Z))
    }

    fn footprint(&mut self) -> Slot {
        if self.mapped() {
            return self.zero();
        }
        let Some(path) = self.node("light_path") else { return self.zero() };
        let spread = self.builder.images.pixel_spread();
        self.math("multiply", Slot::Link(path, "Ray Length"), Slot::Value(Value::Float(spread)), self.zero())
    }

    fn convexity(&mut self, radius: f32) -> Slot {
        if self.mapped() {
            return self.zero();
        }
        let result = self.builder.curvature(self.shader, tequila_materials::graph::CurvatureMode::Convex, radius, 1.0);
        self.check(result, Slot::Value(Value::Float(0.0)))
    }

    fn add(&mut self, a: Slot, b: Slot) -> Slot {
        let z = self.zero();
        self.scalar("add", [a, b, z], |o, a, b, _| o.add(a, b))
    }
    fn sub(&mut self, a: Slot, b: Slot) -> Slot {
        let z = self.zero();
        self.scalar("subtract", [a, b, z], |o, a, b, _| o.sub(a, b))
    }
    fn mul(&mut self, a: Slot, b: Slot) -> Slot {
        let z = self.zero();
        self.scalar("multiply", [a, b, z], |o, a, b, _| o.mul(a, b))
    }
    fn div(&mut self, a: Slot, b: Slot) -> Slot {
        let z = self.zero();
        self.scalar("divide", [a, b, z], |o, a, b, _| o.div(a, b))
    }
    fn madd(&mut self, a: Slot, b: Slot, c: Slot) -> Slot {
        self.scalar("multiply_add", [a, b, c], |o, a, b, c| o.madd(a, b, c))
    }
    fn min(&mut self, a: Slot, b: Slot) -> Slot {
        let z = self.zero();
        self.scalar("minimum", [a, b, z], |o, a, b, _| o.min(a, b))
    }
    fn abs(&mut self, a: Slot) -> Slot {
        let z = self.zero();
        self.scalar("absolute", [a, z, z], |o, a, _, _| o.abs(a))
    }
    fn sqrt(&mut self, a: Slot) -> Slot {
        let z = self.zero();
        self.scalar("sqrt", [a, z, z], |o, a, _, _| o.sqrt(a))
    }
    fn inv_sqrt(&mut self, a: Slot) -> Slot {
        let z = self.zero();
        self.scalar("inversesqrt", [a, z, z], |o, a, _, _| o.inv_sqrt(a))
    }
    fn sin(&mut self, a: Slot) -> Slot {
        let z = self.zero();
        self.scalar("sine", [a, z, z], |o, a, _, _| o.sin(a))
    }
    fn cos(&mut self, a: Slot) -> Slot {
        let z = self.zero();
        self.scalar("cosine", [a, z, z], |o, a, _, _| o.cos(a))
    }
    fn less(&mut self, a: Slot, b: Slot) -> Slot {
        let z = self.zero();
        self.scalar("less_than", [a, b, z], |o, a, b, _| o.less(a, b))
    }

    fn smooth(&mut self, x: Slot, from: Slot, to: Slot, low: Slot, high: Slot) -> Slot {
        if let (Some(x), Some(from), Some(to), Some(low), Some(high)) = (known(x), known(from), known(to), known(low), known(high)) {
            return Slot::Value(Value::Float(numbers().smooth(x, from, to, low, high)));
        }
        let Some(node) = self.node("map_range") else { return self.zero() };
        let result = self.builder.session.set_string(node, "range_type", "smoothstep");
        self.check(result, ());
        for (socket, slot) in [("Value", x), ("From Min", from), ("From Max", to), ("To Min", low), ("To Max", high)] {
            let result = self.builder.feed(self.shader, node, socket, slot, false);
            self.check(result, ());
        }
        Slot::Link(node, "Result")
    }

    fn vadd(&mut self, a: Vector, b: Vector) -> Vector {
        match (a, b) {
            (Vector::Known(a), Vector::Known(b)) => Vector::Known(numbers().vadd(a, b)),
            _ => self.vector_op("add", &[a, b], None),
        }
    }
    fn vsub(&mut self, a: Vector, b: Vector) -> Vector {
        match (a, b) {
            (Vector::Known(a), Vector::Known(b)) => Vector::Known(numbers().vsub(a, b)),
            _ => self.vector_op("subtract", &[a, b], None),
        }
    }
    fn vmadd(&mut self, a: Vector, b: Vector, c: Vector) -> Vector {
        match (a, b, c) {
            (Vector::Known(a), Vector::Known(b), Vector::Known(c)) => Vector::Known(numbers().vmadd(a, b, c)),
            _ => self.vector_op("multiply_add", &[a, b, c], None),
        }
    }
    fn vscale(&mut self, v: Vector, s: Slot) -> Vector {
        match (v, known(s)) {
            (Vector::Known(v), Some(s)) => Vector::Known(numbers().vscale(v, s)),
            _ => self.vector_op("scale", &[v], Some(s)),
        }
    }
    fn vfloor(&mut self, v: Vector) -> Vector {
        match v {
            Vector::Known(v) => Vector::Known(numbers().vfloor(v)),
            _ => self.vector_op("floor", &[v], None),
        }
    }
    fn dot(&mut self, a: Vector, b: Vector) -> Slot {
        match (a, b) {
            (Vector::Known(a), Vector::Known(b)) => Slot::Value(Value::Float(numbers().dot(a, b))),
            _ => self.vector_value("dot_product", &[a, b]),
        }
    }
    fn cross(&mut self, a: Vector, b: Vector) -> Vector {
        match (a, b) {
            (Vector::Known(a), Vector::Known(b)) => Vector::Known(numbers().cross(a, b)),
            _ => self.vector_op("cross_product", &[a, b], None),
        }
    }
    fn length(&mut self, v: Vector) -> Slot {
        match v {
            Vector::Known(v) => Slot::Value(Value::Float(numbers().length(v))),
            _ => self.vector_value("length", &[v]),
        }
    }
    fn normalize(&mut self, v: Vector) -> Vector {
        match v {
            Vector::Known(v) => Vector::Known(numbers().normalize(v)),
            _ => self.vector_op("normalize", &[v], None),
        }
    }

    fn component(&mut self, v: Vector, axis: usize) -> Slot {
        let name = ["X", "Y", "Z"][axis.min(2)];
        match v {
            Vector::Known(v) => Slot::Value(Value::Float(numbers().component(v, axis))),
            Vector::Link(node, output) => match self.split_of((node, output)) {
                Some(split) => Slot::Link(split, name),
                None => self.zero(),
            },
        }
    }

    fn combine(&mut self, x: Slot, y: Slot, z: Slot) -> Vector {
        if let (Some(x), Some(y), Some(z)) = (known(x), known(y), known(z)) {
            return Vector::Known(numbers().combine(x, y, z));
        }
        let result = self.builder.combine(self.shader, x, y, z);
        match self.check(result.map(Some), None) {
            Some((node, output)) => Vector::Link(node, output),
            None => Vector::Known(Vec3::ZERO),
        }
    }

    fn white(&mut self, key: Vector) -> (Vector, Slot) {
        if let Vector::Known(k) = key {
            let (colour, value) = numbers().white(k);
            return (Vector::Known(colour), Slot::Value(Value::Float(value)));
        }
        let Some(key) = self.as_point(key) else { return (Vector::Known(Vec3::ZERO), self.zero()) };
        let result = self.builder.white(self.shader, key);
        match self.check(result.map(Some), None) {
            Some(node) => (Vector::Link(node, "Color"), Slot::Link(node, "Value")),
            None => (Vector::Known(Vec3::ZERO), self.zero()),
        }
    }

    fn noise(&mut self, p: Vector, detail: f32, roughness: f32) -> Slot {
        if let Vector::Known(p) = p {
            return Slot::Value(Value::Float(numbers().noise(p, detail, roughness)));
        }
        let p = self.as_kind(p, Kind::Point);
        let Some(node) = self.node("noise_texture") else { return self.zero() };
        for (socket, value) in [("scale", 1.0), ("detail", detail), ("roughness", roughness), ("lacunarity", 2.0)] {
            let result = self.builder.session.set_float(node, socket, value);
            self.check(result, ());
        }
        self.feed_vector(node, "Vector", p);
        Slot::Link(node, "Fac")
    }

    fn voronoi(&mut self, p: Vector) -> (Slot, Vector, Vector) {
        if let Vector::Known(p) = p {
            let (d, c, at) = numbers().voronoi(p);
            return (Slot::Value(Value::Float(d)), Vector::Known(c), Vector::Known(at));
        }
        let Some(placed) = self.as_point(p) else { return (self.zero(), Vector::Known(Vec3::ZERO), Vector::Known(Vec3::ZERO)) };
        let result = self.builder.voronoi(self.shader, placed, "euclidean", "f1");
        match self.check(result.map(Some), None) {
            Some(node) => (Slot::Link(node, "Distance"), Vector::Link(node, "Color"), Vector::Link(node, "Position")),
            None => (self.zero(), Vector::Known(Vec3::ZERO), Vector::Known(Vec3::ZERO)),
        }
    }
}

impl Nodes<'_, '_> {
    fn mapped(&self) -> bool {
        self.mapping.is_some_and(|m| m.projection != TextureProjection::TriPlanar)
    }

    /// A Separate XYZ of `from`, one a vector.
    fn split_of(&mut self, from: (Node, &'static str)) -> Option<Node> {
        if let Some(split) = self.split.get(&from) {
            return Some(*split);
        }
        // Separate XYZ reads a colour.
        let Vector::Link(node, output) = self.as_kind(Vector::Link(from.0, from.1), Kind::Color) else { return None };
        let result = self.builder.split(self.shader, (node, output));
        let split = self.check(result.map(Some), None)?;
        self.split.insert(from, split);
        Some(split)
    }

    fn kind(&self, node: Node, output: &'static str) -> Kind {
        self.kinds.get(&(node, output)).copied().unwrap_or_else(|| Kind::of_output(output))
    }

    /// `v` as a `kind`: through a conversion node made now, before the node
    /// it will feed, rather than the one Cycles would make when the two are
    /// linked, after it (see the module's notes).
    fn as_kind(&mut self, v: Vector, kind: Kind) -> Vector {
        let Vector::Link(node, output) = v else { return v };
        let from = self.kind(node, output);
        if from == kind {
            return v;
        }
        if let Some(convert) = self.converted.get(&(node, output, kind)) {
            return Vector::Link(*convert, kind.socket());
        }
        let Some(convert) = self.node(&format!("convert_{}_to_{}", from.name(), kind.name())) else { return v };
        let result = self.builder.session.connect(self.shader, node, output, convert, from.socket());
        self.check(result, ());
        self.converted.insert((node, output, kind), convert);
        Vector::Link(convert, kind.socket())
    }

    /// A node output holding `v` as a point, for a texture to read.
    fn as_point(&mut self, v: Vector) -> Option<(Node, &'static str)> {
        let linked = self.linked(v)?;
        match self.as_kind(Vector::Link(linked.0, linked.1), Kind::Point) {
            Vector::Link(node, output) => Some((node, output)),
            Vector::Known(_) => None,
        }
    }
}

impl Builder<'_> {
    /// A wear pattern, as Tequila's renderer evaluates it
    /// (`Procedural::evaluate_at`); `None` for any other pattern.
    pub(super) fn wear(&mut self, shader: Shader, procedural: &Procedural, seed: u32, mapping: Option<&Mapping>) -> Result<Option<Slot>, String> {
        let mut nodes = Nodes { builder: self, shader, mapping, split: HashMap::new(), converted: HashMap::new(), kinds: HashMap::new(), failed: None };
        let slot = match procedural {
            Procedural::WearScratches(s) => wear::scratches(&mut nodes, s, seed),
            Procedural::Dust(d) => wear::dust(&mut nodes, d, seed),
            Procedural::Fingerprints(s) => wear::fingerprints(&mut nodes, s, seed),
            Procedural::Scuffs(s) => wear::scuffs(&mut nodes, s, seed),
            Procedural::EdgeWear(e) => wear::edge_wear(&mut nodes, e, seed),
            _ => return Ok(None),
        };
        match nodes.failed {
            Some(why) => Err(why),
            None => Ok(Some(slot)),
        }
    }
}
