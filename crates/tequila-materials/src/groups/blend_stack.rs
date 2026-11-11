//! Blend Stack: the mix factors of coats laid one over another.
//!
//! A material is layered by a Mix Shader on its Layer input, and the surface
//! it brings in may be layered in its turn. That nests: the second coat is
//! mixed into the first, and shows only where the first does. What a
//! person stacking coats means is the other thing, as the blend material of
//! V-Ray and every paint program has it - each coat over everything under
//! it, by its own amount:
//!
//! ```text
//! result = mix(mix(mix(base, coat 1, a1), coat 2, a2), coat 3, a3)
//! ```
//!
//! The two say the same when the factors are these. With `P(k)` the share
//! left uncovered by coats `k` and up, `P(k) = (1 - a(k)) * P(k + 1)`, the
//! coats `k` and up together cover `S(k) = 1 - P(k)`; the Mix Shader that
//! brings in the first coat takes `S(1)`, and the one that brings coat `k`
//! into coat `k - 1` takes `S(k) / S(k - 1)`: of what the coats from
//! `k - 1` up cover, the share that is `k` and up.
//!
//! So this node takes the amounts and gives the factors, and the nesting
//! renders as a stack in every engine, none of which knows.

use super::{Expansion, GroupInput, GroupOutput};
use crate::graph::{NodeKind, SocketType, Value};
use crate::nodes::ShaderNode;
use tequila_texture::cycles::ops::MathOp;

/// How many coats it has factors for.
pub const COATS: usize = 4;

pub(super) const INPUTS: &[GroupInput] = &[
    GroupInput { name: "Coat 1", default: 0.5, min: 0.0, max: 1.0, tip: "How much of the first coat covers the material: a number, or a mask wired in" },
    GroupInput { name: "Coat 2", default: 0.0, min: 0.0, max: 1.0, tip: "How much of the second coat covers everything under it, the first coat too" },
    GroupInput { name: "Coat 3", default: 0.0, min: 0.0, max: 1.0, tip: "How much of the third coat covers everything under it" },
    GroupInput { name: "Coat 4", default: 0.0, min: 0.0, max: 1.0, tip: "How much of the fourth coat covers everything under it" },
];

pub(super) const OUTPUTS: &[GroupOutput] = &[
    GroupOutput { name: "Fac 1", ty: SocketType::Float, tip: "For the Mix Shader on the material's own Layer, which brings in the first coat" },
    GroupOutput { name: "Fac 2", ty: SocketType::Float, tip: "For the Mix Shader on the first coat's Layer, which brings in the second" },
    GroupOutput { name: "Fac 3", ty: SocketType::Float, tip: "For the Mix Shader on the second coat's Layer, which brings in the third" },
    GroupOutput { name: "Fac 4", ty: SocketType::Float, tip: "For the Mix Shader on the third coat's Layer, which brings in the fourth" },
];

pub(super) fn build() -> Expansion {
    let mut e = Expansion::new(INPUTS.len(), OUTPUTS.len());
    let math = |op: MathOp, clamp: bool| NodeKind::Node(ShaderNode::Math { op, clamp });
    // What each coat leaves uncovered, 1 - a, held to 0..1.
    let open: Vec<u32> = (0..COATS)
        .map(|coat| {
            let node = e.node(math(MathOp::Subtract, true), &[(0, Value::Float(1.0))]);
            e.read(coat, node, 1);
            node
        })
        .collect();
    // P(k), from the top coat down: what coats k and up leave uncovered.
    let mut uncovered = vec![0u32; COATS];
    uncovered[COATS - 1] = open[COATS - 1];
    for coat in (0..COATS - 1).rev() {
        let node = e.node(math(MathOp::Multiply, false), &[]);
        e.link(open[coat], 0, node, 0);
        e.link(uncovered[coat + 1], 0, node, 1);
        uncovered[coat] = node;
    }
    // S(k) = 1 - P(k): what coats k and up cover.
    let covered: Vec<u32> = (0..COATS)
        .map(|coat| {
            let node = e.node(math(MathOp::Subtract, true), &[(0, Value::Float(1.0))]);
            e.link(uncovered[coat], 0, node, 1);
            node
        })
        .collect();
    e.write(0, covered[0], 0);
    for coat in 1..COATS {
        // Nothing under it covered: nothing to be a share of, and none.
        let share = e.node(math(MathOp::Divide, true), &[]);
        e.link(covered[coat], 0, share, 0);
        e.link(covered[coat - 1], 0, share, 1);
        e.write(coat, share, 0);
    }
    e
}
