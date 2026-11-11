//! Tiles laid herringbone and basketweave: planks, parquet, paving.
//!
//! Brick Texture lays its bricks in courses, which is a wall. A floor of
//! planks or blocks is laid in other bonds, and each is a pattern of
//! oblongs a whole number of widths long:
//!
//! - **Herringbone**: every plank at right angles to the next, stepping
//!   along a diagonal. On a grid of squares a width across, the squares of
//!   one diagonal band, `(column - row) mod 2n`, belong by turns to planks
//!   lying across (`n` in a row) and planks standing up (`n` in a column).
//! - **Basketweave**: blocks of `n` planks side by side, each block turned
//!   from its neighbours. The blocks are squares `n` widths across, lying
//!   or standing by `(block column + block row) mod 2`.
//!
//! Both say, at a point: whether it is on a tile or in the gap between,
//! how far it is from the tile's edge, a number of the tile's own, and
//! coordinates along and across that tile - which, wired into a wood's
//! Vector, run the grain along every plank whichever way it lies.
//!
//! Built of Math nodes, so every engine renders them as it renders those.

use super::{Expansion, GroupInput, GroupOutput};
use crate::graph::{NodeKind, SocketType, Value};
use crate::nodes::{MixData, ShaderNode};
use tequila_texture::cycles::ops::{BlendMode, MathOp};

pub(super) const INPUTS: &[GroupInput] = &[
    GroupInput { name: "Tile Width (mm)", default: 70.0, min: 1.0, max: 1000.0, tip: "A tile's short side" },
    GroupInput { name: "Length (widths)", default: 4.0, min: 2.0, max: 12.0, tip: "How many widths long a tile is: a whole number, two or more" },
    GroupInput { name: "Gap (mm)", default: 2.0, min: 0.0, max: 50.0, tip: "The joint between tiles" },
    GroupInput { name: "Plane", default: 0.0, min: 0.0, max: 2.0, tip: "0: laid flat, across the part's X and Z. 1: on a wall facing the front, X and Y. 2: on a wall facing the side, Z and Y" },
];

pub(super) const OUTPUTS: &[GroupOutput] = &[
    GroupOutput { name: "Tile", ty: SocketType::Float, tip: "1 on a tile, 0 in the gap between: a mask, or a bump's height" },
    GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the tile's own: through a Color Ramp, a colour for each" },
    GroupOutput { name: "Tile Coordinates", ty: SocketType::Vector, tip: "Along the tile and across it, in metres, each tile from a place of its own: wire it into a wood's Vector and the grain runs along every plank" },
    GroupOutput { name: "Edge (mm)", ty: SocketType::Float, tip: "How far in from the tile's edge, in millimetres: 0 in the gap. For a bevel or a worn edge" },
];

/// Which bond.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Bond {
    Herringbone,
    Basketweave,
}

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
}

pub(super) fn build(bond: Bond) -> Expansion {
    use MathOp::*;
    let mut b = Build { e: Expansion::new(INPUTS.len(), OUTPUTS.len()) };
    let (width_mm, length, gap_mm, plane) = (V::In(0), V::In(1), V::In(2), V::In(3));

    // The two axes of the plane the tiles are laid in, from the part's own
    // space, in tile widths.
    let coordinates = b.e.node(NodeKind::Node(ShaderNode::TextureCoordinate), &[]);
    let axes = b.e.node(NodeKind::Node(ShaderNode::SeparateXyz), &[]);
    b.e.link(coordinates, 3, axes, 0);
    let (x, y, z) = (V::Node(axes, 0), V::Node(axes, 1), V::Node(axes, 2));
    let wall = b.math(GreaterThan, &[plane, V::Num(0.5)]);
    let side = b.math(GreaterThan, &[plane, V::Num(1.5)]);
    let u = b.pick(side, z, x);
    let v = b.pick(wall, y, z);
    let width = b.math(Multiply, &[width_mm, V::Num(0.001)]);
    let qx = b.math(Divide, &[u, width]);
    let qy = b.math(Divide, &[v, width]);
    // A whole number of widths long, two at the least.
    let n = b.math(Round, &[length]);
    let n = b.math(Maximum, &[n, V::Num(2.0)]);

    // Where on its tile the point is, in widths (`along` 0 to n, `across`
    // 0 to 1), whether that tile lies across, and the three numbers that
    // are the tile's alone.
    let (along, across, lying, id) = match bond {
        Bond::Herringbone => {
            let (i, j) = (b.math(Floor, &[qx]), b.math(Floor, &[qy]));
            let (fx, fy) = (b.math(Fraction, &[qx]), b.math(Fraction, &[qy]));
            let band = b.math(Subtract, &[i, j]);
            let period = b.math(Multiply, &[n, V::Num(2.0)]);
            let k = b.math(FlooredModulo, &[band, period]);
            let lying = b.math(LessThan, &[k, n]);
            // Lying: the k-th square of its row.
            let along_lying = b.math(Add, &[k, fx]);
            let first = b.math(Subtract, &[i, k]);
            // Standing: square k - n counted down from its top.
            let down = b.math(Subtract, &[k, n]);
            let last = b.math(Subtract, &[n, V::Num(1.0)]);
            let up = b.math(Subtract, &[last, down]);
            let along_standing = b.math(Add, &[up, fy]);
            let top = b.math(Add, &[j, down]);
            let along = b.pick(lying, along_lying, along_standing);
            let across = b.pick(lying, fy, fx);
            let id_x = b.pick(lying, first, i);
            let id_y = b.pick(lying, j, top);
            (along, across, lying, [id_x, id_y, lying])
        }
        Bond::Basketweave => {
            let bx = b.math(Divide, &[qx, n]);
            let by = b.math(Divide, &[qy, n]);
            let (bi, bj) = (b.math(Floor, &[bx]), b.math(Floor, &[by]));
            let ox = b.math(Multiply, &[bi, n]);
            let oy = b.math(Multiply, &[bj, n]);
            let lx = b.math(Subtract, &[qx, ox]);
            let ly = b.math(Subtract, &[qy, oy]);
            let sum = b.math(Add, &[bi, bj]);
            let odd = b.math(FlooredModulo, &[sum, V::Num(2.0)]);
            let lying = b.math(LessThan, &[odd, V::Num(0.5)]);
            let (row, column) = (b.math(Floor, &[ly]), b.math(Floor, &[lx]));
            let (fy, fx) = (b.math(Fraction, &[ly]), b.math(Fraction, &[lx]));
            let along = b.pick(lying, lx, ly);
            let across = b.pick(lying, fy, fx);
            let within = b.pick(lying, row, column);
            (along, across, lying, [bi, bj, within])
        }
    };
    let _ = lying;

    // How far in from the nearest edge, in widths, and the half of the
    // joint that is this tile's.
    let far = b.math(Subtract, &[n, along]);
    let ends = b.math(Minimum, &[along, far]);
    let other = b.math(Subtract, &[V::Num(1.0), across]);
    let sides = b.math(Minimum, &[across, other]);
    let edge = b.math(Minimum, &[ends, sides]);
    let joint = b.math(Divide, &[gap_mm, width_mm]);
    let half = b.math(Multiply, &[joint, V::Num(0.5)]);
    let tile = b.math(GreaterThan, &[edge, half]);
    let inside = b.math(Subtract, &[edge, half]);
    let inside = b.math(Maximum, &[inside, V::Num(0.0)]);
    let edge_mm = b.math(Multiply, &[inside, width_mm]);

    // The tile's own number, from the three that name it.
    let name = b.combine(id[0], id[1], id[2]);
    let V::Node(name, _) = name else { unreachable!("combine makes a node") };
    let noise = b.e.node(NodeKind::Node(ShaderNode::WhiteNoise { dims: 3 }), &[]);
    b.e.link(name, 0, noise, 0);
    let random = V::Node(noise, 0);

    // Along and across in metres, each tile moved to a place of its own.
    let away = b.math(Multiply, &[random, V::Num(5.0)]);
    let along_m = b.math(MultiplyAdd, &[along, width, away]);
    let across_m = b.math(Multiply, &[across, width]);
    let place = b.combine(along_m, across_m, away);

    for (output, value) in [tile, random, place, edge_mm].into_iter().enumerate() {
        let V::Node(node, out) = value else { unreachable!("every output is a node's") };
        b.e.write(output, node, out);
    }
    b.e
}
