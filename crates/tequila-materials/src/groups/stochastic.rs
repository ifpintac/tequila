//! Stochastic tiling: a picture repeated without its repeats showing.
//!
//! A picture tiled over a floor shows its own period: the one knot, the one
//! stain, in a row to the horizon. The cure is to lay it in patches, each
//! starting from a place of its own in the picture, and to hide where the
//! patches meet.
//!
//! The patches here are the squares of a grid, each moving the picture by
//! two numbers of its own. A second grid, set a third of a square over,
//! does the same with other numbers, and a third, two thirds over. Each is
//! counted by how far the point is from the edge of its square - nothing
//! at the edge, where the picture jumps, most at the middle - and the three
//! are blended by those counts. Two grids are not enough: where an edge of
//! one crosses an edge of the other neither counts, and the crossing shows
//! as a crease. With three, set over by thirds, the third is well inside a
//! square wherever the other two cross.
//!
//! The group gives the three sets of coordinates and the two blends that
//! bring them together; the picture is read three times, at each, by three
//! Image Textures, and two Mixes make one of them. A blend of places is
//! flatter than any one, so the counts are raised to a power first: the
//! higher, the narrower the band in which more than one shows.
//!
//! Built of Math nodes and three White Noises, so every engine renders it
//! as it renders those.

use super::{Expansion, GroupInput, GroupOutput};
use crate::graph::{NodeKind, SocketType, Value};
use crate::nodes::{MixData, ShaderNode};
use tequila_texture::cycles::ops::{BlendMode, MathOp};

pub(super) const INPUTS: &[GroupInput] = &[
    GroupInput { name: "Repeats", default: 4.0, min: 0.01, max: 100.0, tip: "How many times the picture repeats across the part's UVs" },
    GroupInput { name: "Patch Size", default: 0.6, min: 0.05, max: 4.0, tip: "How large each patch of the picture is, in pictures: smaller hides a repeat better and shows less of the picture whole" },
    GroupInput { name: "Sharpness", default: 2.0, min: 1.0, max: 16.0, tip: "How narrow the band is in which patches blend: higher keeps more of the picture's contrast, lower hides the patches better" },
    GroupInput { name: "Seed", default: 0.0, min: 0.0, max: 100.0, tip: "Another number, another arrangement" },
    GroupInput { name: "Plane", default: 0.0, min: 0.0, max: 3.0, tip: "0: the part's UVs. For a part without them - 1: laid flat, across the part's X and Z. 2: on a wall facing the front, X and Y. 3: on a wall facing the side, Z and Y" },
    GroupInput { name: "Size (mm)", default: 500.0, min: 1.0, max: 10000.0, tip: "How wide the picture is on a plane, where Repeats counts across UVs" },
];

pub(super) const OUTPUTS: &[GroupOutput] = &[
    GroupOutput { name: "Coordinates A", ty: SocketType::Vector, tip: "Where to read the picture the first time: into an Image Texture's Coordinates" },
    GroupOutput { name: "Coordinates B", ty: SocketType::Vector, tip: "Where to read it the second time: into the Coordinates of another Image Texture of the same picture" },
    GroupOutput { name: "Coordinates C", ty: SocketType::Vector, tip: "Where to read it the third time: into the Coordinates of a third Image Texture of the same picture" },
    GroupOutput { name: "Blend B", ty: SocketType::Float, tip: "How much of the second reading shows over the first: into the Factor of a Mix of the two" },
    GroupOutput { name: "Blend C", ty: SocketType::Float, tip: "How much of the third reading shows over that Mix: into the Factor of a second Mix, of the first Mix and the third reading" },
];

/// A value on its way through the group: a node's output, a number, or one
/// of the group's inputs.
#[derive(Clone, Copy)]
enum V {
    Node(u32, u8),
    Num(f32),
    In(usize),
}

struct Build {
    e: Expansion,
}

impl Build {
    fn feed(&mut self, node: u32, input: u8, value: V) {
        match value {
            V::Node(from, output) => self.e.link(from, output, node, input),
            V::Num(number) => self.e.set(node, input, Value::Float(number)),
            V::In(group_input) => self.e.read(group_input, node, input),
        }
    }

    fn math(&mut self, op: MathOp, args: &[V]) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::Math { op, clamp: false }), &[]);
        for (input, value) in args.iter().enumerate() {
            self.feed(node, input as u8, *value);
        }
        V::Node(node, 0)
    }

    /// `yes` where `when` is 1, `no` where it is 0.
    fn pick(&mut self, when: V, yes: V, no: V) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::Mix { data: MixData::Float, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false }), &[]);
        self.feed(node, 0, when);
        self.feed(node, 1, no);
        self.feed(node, 2, yes);
        V::Node(node, 0)
    }

    fn combine(&mut self, x: V, y: V, z: V) -> V {
        let node = self.e.node(NodeKind::Node(ShaderNode::CombineXyz), &[]);
        for (input, value) in [x, y, z].into_iter().enumerate() {
            self.feed(node, input as u8, value);
        }
        V::Node(node, 0)
    }

    fn split(&mut self, vector: V) -> (V, V, V) {
        let node = self.e.node(NodeKind::Node(ShaderNode::SeparateXyz), &[]);
        self.feed(node, 0, vector);
        (V::Node(node, 0), V::Node(node, 1), V::Node(node, 2))
    }
}

/// White Noise's Color output, and Texture Coordinate's UV and Object.
const NOISE_COLOR: u8 = 1;
const UV: u8 = 2;
const OBJECT: u8 = 3;

pub(super) fn build() -> Expansion {
    use MathOp::*;
    let mut b = Build { e: Expansion::new(INPUTS.len(), OUTPUTS.len()) };
    let (repeats, patch, sharpness, seed, plane, size_mm) = (V::In(0), V::In(1), V::In(2), V::In(3), V::In(4), V::In(5));

    // Where the point is in pictures: across the UVs, or across a plane of
    // the part's own space, in scene units.
    let coordinates = b.e.node(NodeKind::Node(ShaderNode::TextureCoordinate), &[]);
    let (u, v, _) = b.split(V::Node(coordinates, UV));
    let (x, y, z) = b.split(V::Node(coordinates, OBJECT));
    let planar = b.math(GreaterThan, &[plane, V::Num(0.5)]);
    let wall = b.math(GreaterThan, &[plane, V::Num(1.5)]);
    let side = b.math(GreaterThan, &[plane, V::Num(2.5)]);
    let across = b.pick(side, z, x);
    let up = b.pick(wall, y, z);
    let size = b.math(Multiply, &[size_mm, V::Num(0.001)]);
    let on_plane = (b.math(Divide, &[across, size]), b.math(Divide, &[up, size]));
    let on_uvs = (b.math(Multiply, &[u, repeats]), b.math(Multiply, &[v, repeats]));
    let px = b.pick(planar, on_plane.0, on_uvs.0);
    let py = b.pick(planar, on_plane.1, on_uvs.1);

    // One grid of patches: where to read the picture, and how far the point
    // is from the edge of its square, 0 to 1.
    let layer = |b: &mut Build, shift: f32, name: V| -> (V, V) {
        let qx = b.math(Divide, &[px, patch]);
        let qy = b.math(Divide, &[py, patch]);
        let (qx, qy) = (b.math(Add, &[qx, V::Num(shift)]), b.math(Add, &[qy, V::Num(shift)]));
        let (cx, cy) = (b.math(Floor, &[qx]), b.math(Floor, &[qy]));
        let (fx, fy) = (b.math(Fraction, &[qx]), b.math(Fraction, &[qy]));
        // The square's own two numbers: how far the picture is moved in it.
        let square = b.combine(cx, cy, name);
        let noise = b.e.node(NodeKind::Node(ShaderNode::WhiteNoise { dims: 3 }), &[]);
        b.feed(noise, 0, square);
        let (rx, ry, _) = b.split(V::Node(noise, NOISE_COLOR));
        let place = (b.math(Add, &[px, rx]), b.math(Add, &[py, ry]));
        let place = b.combine(place.0, place.1, V::Num(0.0));
        let (ox, oy) = (b.math(Subtract, &[V::Num(1.0), fx]), b.math(Subtract, &[V::Num(1.0), fy]));
        let (ex, ey) = (b.math(Minimum, &[fx, ox]), b.math(Minimum, &[fy, oy]));
        let edge = b.math(Minimum, &[ex, ey]);
        let edge = b.math(Multiply, &[edge, V::Num(2.0)]);
        let count = b.math(Power, &[edge, sharpness]);
        (place, count)
    };
    let second = b.math(Add, &[seed, V::Num(0.25)]);
    let third = b.math(Add, &[seed, V::Num(0.5)]);
    let (place_a, count_a) = layer(&mut b, 0.0, seed);
    let (place_b, count_b) = layer(&mut b, 1.0 / 3.0, second);
    let (place_c, count_c) = layer(&mut b, 2.0 / 3.0, third);

    // The second's share of the first two, and the third's of all three.
    // Where the first two grids cross neither counts, and it is the third
    // that is seen.
    let two = b.math(Add, &[count_a, count_b]);
    let three = b.math(Add, &[two, count_c]);
    let two = b.math(Maximum, &[two, V::Num(1e-6)]);
    let three = b.math(Maximum, &[three, V::Num(1e-6)]);
    let blend_b = b.math(Divide, &[count_b, two]);
    let blend_c = b.math(Divide, &[count_c, three]);

    for (output, value) in [place_a, place_b, place_c, blend_b, blend_c].into_iter().enumerate() {
        let V::Node(node, out) = value else { unreachable!("every output is a node's") };
        b.e.write(output, node, out);
    }
    b.e
}
