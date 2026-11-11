//! What the pattern groups are built with: a value on its way through a
//! group, the Math that works on it, and the plane a pattern is laid in.

use super::{Expansion, GroupInput};
use crate::graph::{NodeKind, Value};
use crate::nodes::{MixData, ShaderNode};
use tequila_texture::cycles::noise::FractalKind;
use tequila_texture::cycles::ops::{BlendMode, MathOp, VectorMathOp};
use tequila_texture::cycles::voronoi::{VoronoiFeature, VoronoiMetric};

/// A value on its way through the group: a node's output, a number, or one
/// of the group's inputs.
#[derive(Clone, Copy)]
pub(super) enum V {
    Node(u32, u8),
    Num(f32),
    In(usize),
}

/// The input every pattern laid in a plane ends with: which plane.
pub(super) const PLANE: GroupInput = GroupInput {
    name: "Plane",
    default: 0.0,
    min: 0.0,
    max: 4.0,
    tip: "0: laid flat, across the part's X and Z. 1: on a wall facing the front, X and Y. 2: on a wall facing the side, Z and Y. 3: the part's UVs, a metre to the square. 4: wrapped round the part's Y axis, as round a knob",
};

/// Texture Coordinate's UV and Object, and White Noise's Value.
const UV: u8 = 2;
const OBJECT: u8 = 3;

pub(super) struct Build {
    pub e: Expansion,
}

impl Build {
    pub fn new(inputs: usize, outputs: usize) -> Build {
        Build { e: Expansion::new(inputs, outputs) }
    }

    pub fn feed(&mut self, node: u32, input: u8, value: V) {
        match value {
            V::Node(from, output) => self.e.link(from, output, node, input),
            V::Num(number) => self.e.set(node, input, Value::Float(number)),
            V::In(group_input) => self.e.read(group_input, node, input),
        }
    }

    pub fn math(&mut self, op: MathOp, args: &[V]) -> V {
        self.math_with(op, false, args)
    }

    /// The same, its result held between nought and one.
    pub fn clamped(&mut self, op: MathOp, args: &[V]) -> V {
        self.math_with(op, true, args)
    }

    fn math_with(&mut self, op: MathOp, clamp: bool, args: &[V]) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::Math { op, clamp }), &[]);
        for (input, value) in args.iter().enumerate() {
            self.feed(node, input as u8, *value);
        }
        V::Node(node, 0)
    }

    /// A node of that kind, nothing set on it.
    pub fn node(&mut self, kind: NodeKind) -> u32 {
        self.e.node(kind, &[])
    }

    /// `yes` where `when` is 1, `no` where it is 0.
    pub fn pick(&mut self, when: V, yes: V, no: V) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::Mix { data: MixData::Float, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false }), &[]);
        self.feed(node, 0, when);
        self.feed(node, 1, no);
        self.feed(node, 2, yes);
        V::Node(node, 0)
    }

    pub fn combine(&mut self, x: V, y: V, z: V) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::CombineXyz), &[]);
        for (input, value) in [x, y, z].into_iter().enumerate() {
            self.feed(node, input as u8, value);
        }
        V::Node(node, 0)
    }

    pub fn split(&mut self, vector: V) -> (V, V, V) {
        let node = self.e.node(NodeKind::Node(ShaderNode::SeparateXyz), &[]);
        self.feed(node, 0, vector);
        (V::Node(node, 0), V::Node(node, 1), V::Node(node, 2))
    }

    /// A number from nought to one that is the same wherever the three
    /// given are: a tile's own, from the numbers that name it.
    pub fn random(&mut self, a: V, b: V, c: V) -> V {
        let name = self.combine(a, b, c);
        let noise = self.e.node(NodeKind::Node(ShaderNode::WhiteNoise { dims: 3 }), &[]);
        self.feed(noise, 0, name);
        V::Node(noise, 0)
    }

    /// Where the point is in the plane the pattern is laid in, in scene
    /// units: across it and up it ([`PLANE`]).
    pub fn on_plane(&mut self, plane: V) -> (V, V) {
        use MathOp::*;
        let coordinates = self.e.node(NodeKind::Node(ShaderNode::TextureCoordinate), &[]);
        let (x, y, z) = self.split(V::Node(coordinates, OBJECT));
        let (s, t, _) = self.split(V::Node(coordinates, UV));
        let wall = self.math(GreaterThan, &[plane, V::Num(0.5)]);
        let side = self.math(GreaterThan, &[plane, V::Num(1.5)]);
        let uvs = self.math(GreaterThan, &[plane, V::Num(2.5)]);
        let round = self.math(GreaterThan, &[plane, V::Num(3.5)]);
        // Round the Y axis: how far round, as a length along the surface.
        let angle = self.math(Arctan2, &[z, x]);
        let (xx, zz) = (self.math(Multiply, &[x, x]), self.math(Multiply, &[z, z]));
        let radius = self.math(Add, &[xx, zz]);
        let radius = self.math(Sqrt, &[radius]);
        let arc = self.math(Multiply, &[angle, radius]);
        let u = self.pick(side, z, x);
        let u = self.pick(uvs, s, u);
        let u = self.pick(round, arc, u);
        let v = self.pick(wall, y, z);
        let v = self.pick(uvs, t, v);
        let v = self.pick(round, y, v);
        (u, v)
    }

    /// `(u, v)` turned by `degrees`: along the new direction and across it.
    pub fn turned(&mut self, u: V, v: V, degrees: V) -> (V, V) {
        use MathOp::*;
        let angle = self.math(Radians, &[degrees]);
        let (c, s) = (self.math(Cosine, &[angle]), self.math(Sine, &[angle]));
        let (uc, vs) = (self.math(Multiply, &[u, c]), self.math(Multiply, &[v, s]));
        let (us, vc) = (self.math(Multiply, &[u, s]), self.math(Multiply, &[v, c]));
        (self.math(Add, &[uc, vs]), self.math(Subtract, &[vc, us]))
    }

    /// How many of a size in millimetres go to a scene unit: a texture's
    /// Scale for features that size.
    pub fn per_metre(&mut self, millimetres: V) -> V {
        self.math(MathOp::Divide, &[V::Num(1000.0), millimetres])
    }

    /// A Noise Texture, in the part's own space or at `at`: its Fac, about
    /// a half, and its Color. `w` moves it to another noise altogether.
    pub fn noise(&mut self, at: Option<V>, scale: V, w: V, detail: V, roughness: f32, distortion: f32) -> (V, V) {
        let node = self.e.node(
            NodeKind::Node(ShaderNode::Noise { dims: 4, fractal: FractalKind::Fbm, normalize: true }),
            &[(4, Value::Float(roughness)), (8, Value::Float(distortion))],
        );
        if let Some(at) = at {
            self.feed(node, 0, at);
        }
        self.feed(node, 1, w);
        self.feed(node, 2, scale);
        self.feed(node, 3, detail);
        (V::Node(node, 0), V::Node(node, 1))
    }

    /// A Voronoi Texture of that feature, in the part's own space or at
    /// `at`: the node, whose outputs are the feature's.
    pub fn cells(&mut self, feature: VoronoiFeature, at: Option<V>, scale: V, w: V) -> u32 {
        let node = self.e.node(NodeKind::Node(ShaderNode::Voronoi { dims: 4, feature, metric: VoronoiMetric::Euclidean, normalize: false }), &[]);
        if let Some(at) = at {
            self.feed(node, 0, at);
        }
        self.feed(node, 1, w);
        self.feed(node, 2, scale);
        node
    }

    /// Nought below `low`, one above `high`, and an easy curve between.
    pub fn smooth(&mut self, low: V, high: V, x: V) -> V {
        use MathOp::*;
        let (above, span) = (self.math(Subtract, &[x, low]), self.math(Subtract, &[high, low]));
        let span = self.math(Maximum, &[span, V::Num(1e-5)]);
        let t = self.clamped(Divide, &[above, span]);
        let ease = self.math(MultiplyAdd, &[t, V::Num(-2.0), V::Num(3.0)]);
        let squared = self.math(Multiply, &[t, t]);
        self.math(Multiply, &[squared, ease])
    }

    /// Where the point is in the part's own space, in scene units.
    pub fn object(&mut self) -> V {
        let coordinates = self.e.node(NodeKind::Node(ShaderNode::TextureCoordinate), &[]);
        V::Node(coordinates, OBJECT)
    }

    /// Vector Math on two vectors.
    pub fn vector(&mut self, op: VectorMathOp, a: V, b: V) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::VectorMath { op }), &[]);
        self.feed_vector(node, 0, a);
        self.feed_vector(node, 1, b);
        V::Node(node, 0)
    }

    /// A vector times a number.
    pub fn scaled(&mut self, vector: V, by: V) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::VectorMath { op: VectorMathOp::Scale }), &[]);
        self.feed_vector(node, 0, vector);
        self.feed(node, 3, by);
        V::Node(node, 0)
    }

    /// As [`Build::feed`], a number given being the same along each axis.
    fn feed_vector(&mut self, node: u32, input: u8, value: V) {
        match value {
            V::Num(number) => self.e.set(node, input, Value::Vector(tequila_core::math::Vec3::splat(number))),
            other => self.feed(node, input, other),
        }
    }

    /// Millimetres as scene units.
    pub fn metres(&mut self, millimetres: V) -> V {
        self.math(MathOp::Multiply, &[millimetres, V::Num(0.001)])
    }

    /// The group, its outputs these in order.
    pub fn finish(mut self, outputs: &[V]) -> Expansion {
        for (output, value) in outputs.iter().enumerate() {
            let V::Node(node, out) = *value else { unreachable!("every output is a node's") };
            self.e.write(output, node, out);
        }
        self.e
    }
}
