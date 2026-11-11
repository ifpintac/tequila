//! Patterns a product's surface is made with: the knurl on a knob, the
//! hexagons of a grille, dots, ribs, the lozenges of tread plate.
//!
//! Each is laid in a plane of the part's own space, in millimetres, so a
//! knurl of a millimetre and a half is that on a knob of any size; or
//! round the part's Y axis, as a knurl is cut; or across its UVs. Each
//! gives a mask and a height, for a colour and for a Bump, and where it is
//! made of pieces, a number of each piece's own.
//!
//! Built of Math nodes and a White Noise, so every engine renders them as
//! it renders those.

use super::build::{Build, PLANE, V};
use super::{Expansion, GroupInput, GroupOutput, Spec};
use crate::graph::SocketType;
use tequila_texture::cycles::ops::MathOp::*;

const HEIGHT: &str = "From 0 to 1: into a Bump's Height";

pub(super) const KNURL: Spec = Spec {
    key: "knurl",
    label: "Knurl",
    description: "The grip cut into a knob or a handle: two sets of grooves crossing, leaving rows of small pyramids, or at an \
                  angle of nought one set, leaving ridges. Wire Height into a Bump's Height. Plane 4 wraps it round the part's \
                  Y axis, as it is cut.",
    keywords: "knurling, diamond knurl, straight knurl, grip, knob, handle, crosshatch",
    inputs: &[
        GroupInput { name: "Pitch (mm)", default: 1.5, min: 0.1, max: 20.0, tip: "From one groove to the next" },
        GroupInput { name: "Angle", default: 30.0, min: 0.0, max: 60.0, tip: "How far the grooves lean either way, in degrees: 30 is the usual diamond, 0 a straight knurl" },
        GroupInput { name: "Flat", default: 0.1, min: 0.0, max: 0.9, tip: "How much of each point is cut flat" },
        PLANE,
    ],
    outputs: &[GroupOutput { name: "Height", ty: SocketType::Float, tip: HEIGHT }],
    build: knurl,
};

fn knurl() -> Expansion {
    let mut b = Build::new(KNURL.inputs.len(), KNURL.outputs.len());
    let (pitch, angle, flat, plane) = (V::In(0), V::In(1), V::In(2), V::In(3));
    let (u, v) = b.on_plane(plane);
    let pitch = b.metres(pitch);
    // One set of grooves leaning one way, the other the other: nought in a
    // groove, one half way to the next.
    let radians = b.math(Radians, &[angle]);
    let (c, s) = (b.math(Cosine, &[radians]), b.math(Sine, &[radians]));
    let (uc, vs) = (b.math(Multiply, &[u, c]), b.math(Multiply, &[v, s]));
    let ridge = |b: &mut Build, across: V| {
        let t = b.math(Divide, &[across, pitch]);
        let f = b.math(Fraction, &[t]);
        let f = b.math(MultiplyAdd, &[f, V::Num(2.0), V::Num(-1.0)]);
        let f = b.math(Absolute, &[f]);
        b.math(Subtract, &[V::Num(1.0), f])
    };
    let one = b.math(Add, &[uc, vs]);
    let other = b.math(Subtract, &[uc, vs]);
    let (one, other) = (ridge(&mut b, one), ridge(&mut b, other));
    let point = b.math(Minimum, &[one, other]);
    // The top cut flat.
    let left = b.math(Subtract, &[V::Num(1.0), flat]);
    let left = b.math(Maximum, &[left, V::Num(0.05)]);
    let height = b.clamped(Divide, &[point, left]);
    b.finish(&[height])
}

pub(super) const HEXAGONS: Spec = Spec {
    key: "hexagons",
    label: "Hexagons",
    description: "Hexagons side by side, as a honeycomb, a grille or a floor of hexagonal tiles. Tile is the mask of tile and \
                  joint; Random a number for each tile, for a Color Ramp; Edge how far in from the tile's edge, for a bevel; \
                  Centre runs from 0 at the tile's middle to 1 at its edge.",
    keywords: "honeycomb, hex, hexagonal tiles, grille, mesh, cells, scales",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 10.0, min: 0.5, max: 500.0, tip: "A hexagon's width, flat side to flat side" },
        GroupInput { name: "Gap (mm)", default: 1.0, min: 0.0, max: 50.0, tip: "The joint between hexagons" },
        PLANE,
    ],
    outputs: &[
        GroupOutput { name: "Tile", ty: SocketType::Float, tip: "1 on a hexagon, 0 in the joint between: a mask, or a bump's height" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the hexagon's own: through a Color Ramp, a colour for each" },
        GroupOutput { name: "Edge (mm)", ty: SocketType::Float, tip: "How far in from the hexagon's edge, in millimetres: 0 in the joint. For a bevel" },
        GroupOutput { name: "Centre", ty: SocketType::Float, tip: "0 at the hexagon's middle, 1 at its edge" },
    ],
    build: hexagons,
};

fn hexagons() -> Expansion {
    let mut b = Build::new(HEXAGONS.inputs.len(), HEXAGONS.outputs.len());
    let (size_mm, gap_mm, plane) = (V::In(0), V::In(1), V::In(2));
    let (u, v) = b.on_plane(plane);
    let size = b.metres(size_mm);
    let (px, py) = (b.math(Divide, &[u, size]), b.math(Divide, &[v, size]));
    // Hexagons a unit across lie on two grids of oblongs, 1 by root 3, one
    // set half an oblong over from the other: the point is in the hexagon
    // whose middle is nearer.
    let root3 = 3f32.sqrt();
    let nearest = |b: &mut Build, shift: f32| {
        let (sx, sy) = (b.math(Subtract, &[px, V::Num(shift * 0.5)]), b.math(Subtract, &[py, V::Num(shift * root3 * 0.5)]));
        let (mx, my) = (b.math(FlooredModulo, &[sx, V::Num(1.0)]), b.math(FlooredModulo, &[sy, V::Num(root3)]));
        let (gx, gy) = (b.math(Subtract, &[mx, V::Num(0.5)]), b.math(Subtract, &[my, V::Num(root3 * 0.5)]));
        let (xx, yy) = (b.math(Multiply, &[gx, gx]), b.math(Multiply, &[gy, gy]));
        let far = b.math(Add, &[xx, yy]);
        (gx, gy, far)
    };
    let (ax, ay, a_far) = nearest(&mut b, 0.0);
    let (bx, by, b_far) = nearest(&mut b, 1.0);
    let first = b.math(LessThan, &[a_far, b_far]);
    let (gx, gy) = (b.pick(first, ax, bx), b.pick(first, ay, by));
    // From the middle towards the nearest side, a half at the side.
    let (wide, tall) = (b.math(Absolute, &[gx]), b.math(Absolute, &[gy]));
    let slant = b.math(Multiply, &[wide, V::Num(0.5)]);
    let slant = b.math(MultiplyAdd, &[tall, V::Num(root3 * 0.5), slant]);
    let out = b.math(Maximum, &[wide, slant]);
    let centre = b.clamped(Multiply, &[out, V::Num(2.0)]);
    let edge = b.math(Subtract, &[V::Num(0.5), out]);
    let joint = b.math(Divide, &[gap_mm, size_mm]);
    let half = b.math(Multiply, &[joint, V::Num(0.5)]);
    let tile = b.math(GreaterThan, &[edge, half]);
    let inside = b.math(Subtract, &[edge, half]);
    let inside = b.math(Maximum, &[inside, V::Num(0.0)]);
    let edge_mm = b.math(Multiply, &[inside, size_mm]);
    // The hexagon's middle names it: whole numbers, doubled across and
    // counted in half oblongs up.
    let (cx, cy) = (b.math(Subtract, &[px, gx]), b.math(Subtract, &[py, gy]));
    let cx = b.math(Multiply, &[cx, V::Num(2.0)]);
    let cy = b.math(Multiply, &[cy, V::Num(2.0 / root3)]);
    let (cx, cy) = (b.math(Round, &[cx]), b.math(Round, &[cy]));
    let random = b.random(cx, cy, V::Num(0.0));
    b.finish(&[tile, random, edge_mm, centre])
}

pub(super) const DOTS: Spec = Spec {
    key: "dots",
    label: "Dots",
    description: "Round dots in rows: grip dots, dimples, perforations, a halftone. Dot is the mask; Dome the height of a round \
                  bump on each, for a Bump's Height (through an Invert, a dimple); Random a number for each dot.",
    keywords: "polka dots, dimples, bumps, grip dots, perforated, halftone, studs, pimples, braille",
    inputs: &[
        GroupInput { name: "Spacing (mm)", default: 5.0, min: 0.2, max: 500.0, tip: "From one dot's middle to the next" },
        GroupInput { name: "Size", default: 0.5, min: 0.05, max: 0.85, tip: "A dot's width, as a share of the spacing" },
        GroupInput { name: "Staggered", default: 1.0, min: 0.0, max: 1.0, tip: "1: every other row half a step along, the rows closer, as oranges are stacked. 0: a square grid" },
        PLANE,
    ],
    outputs: &[
        GroupOutput { name: "Dot", ty: SocketType::Float, tip: "1 on a dot, 0 between" },
        GroupOutput { name: "Dome", ty: SocketType::Float, tip: "The height of a round bump on each dot, from 0 at its rim to 1 at its middle" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the dot's own" },
    ],
    build: dots,
};

fn dots() -> Expansion {
    let mut b = Build::new(DOTS.inputs.len(), DOTS.outputs.len());
    let (spacing, size, staggered, plane) = (V::In(0), V::In(1), V::In(2), V::In(3));
    let (u, v) = b.on_plane(plane);
    let spacing = b.metres(spacing);
    let staggered = b.math(GreaterThan, &[staggered, V::Num(0.5)]);
    // Staggered rows sit closer, so each dot is as far from the six round
    // it.
    let row_height = b.pick(staggered, V::Num(0.866_025_4), V::Num(1.0));
    let qx = b.math(Divide, &[u, spacing]);
    let qy = b.math(Divide, &[v, spacing]);
    let qy = b.math(Divide, &[qy, row_height]);
    let row = b.math(Floor, &[qy]);
    let odd = b.math(FlooredModulo, &[row, V::Num(2.0)]);
    let shift = b.math(Multiply, &[odd, V::Num(0.5)]);
    let shift = b.math(Multiply, &[shift, staggered]);
    let qx = b.math(Add, &[qx, shift]);
    let column = b.math(Floor, &[qx]);
    // From the dot's middle, in spacings.
    let fx = b.math(Fraction, &[qx]);
    let fx = b.math(Subtract, &[fx, V::Num(0.5)]);
    let fy = b.math(Fraction, &[qy]);
    let fy = b.math(Subtract, &[fy, V::Num(0.5)]);
    let fy = b.math(Multiply, &[fy, row_height]);
    let (xx, yy) = (b.math(Multiply, &[fx, fx]), b.math(Multiply, &[fy, fy]));
    let far = b.math(Add, &[xx, yy]);
    // As a share of the dot's radius, squared.
    let radius = b.math(Multiply, &[size, V::Num(0.5)]);
    let radius = b.math(Multiply, &[radius, radius]);
    let out = b.math(Divide, &[far, radius]);
    let dot = b.math(LessThan, &[out, V::Num(1.0)]);
    let dome = b.math(Subtract, &[V::Num(1.0), out]);
    let dome = b.math(Maximum, &[dome, V::Num(0.0)]);
    let dome = b.math(Sqrt, &[dome]);
    let random = b.random(column, row, V::Num(0.0));
    b.finish(&[dot, dome, random])
}

pub(super) const STRIPES: Spec = Spec {
    key: "stripes",
    label: "Stripes",
    description: "Stripes of a width, at an angle: pinstripes, the ribs of a grip, grooves. Stripe is the mask; Round the height \
                  of a half-round rib on each, for a Bump's Height; Random a number for each stripe.",
    keywords: "ribs, ribbed, grooves, lines, pinstripe, bands, corrugated, fluted, reeded",
    inputs: &[
        GroupInput { name: "Width (mm)", default: 4.0, min: 0.1, max: 1000.0, tip: "From one stripe to the next" },
        GroupInput { name: "Share", default: 0.5, min: 0.02, max: 1.0, tip: "How much of that is the stripe, and not the gap" },
        GroupInput { name: "Angle", default: 0.0, min: -90.0, max: 90.0, tip: "How far the stripes are turned, in degrees" },
        PLANE,
    ],
    outputs: &[
        GroupOutput { name: "Stripe", ty: SocketType::Float, tip: "1 on a stripe, 0 between" },
        GroupOutput { name: "Round", ty: SocketType::Float, tip: "The height of a half-round rib on each stripe, from 0 at its sides to 1 along its middle" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the stripe's own" },
    ],
    build: stripes,
};

fn stripes() -> Expansion {
    let mut b = Build::new(STRIPES.inputs.len(), STRIPES.outputs.len());
    let (width, share, angle, plane) = (V::In(0), V::In(1), V::In(2), V::In(3));
    let (u, v) = b.on_plane(plane);
    let (across, _) = b.turned(u, v, angle);
    let width = b.metres(width);
    let t = b.math(Divide, &[across, width]);
    let index = b.math(Floor, &[t]);
    let f = b.math(Fraction, &[t]);
    let stripe = b.math(LessThan, &[f, share]);
    // Across the stripe from -1 to 1, and the half circle over it.
    let within = b.math(Divide, &[f, share]);
    let within = b.math(MultiplyAdd, &[within, V::Num(2.0), V::Num(-1.0)]);
    let squared = b.math(Multiply, &[within, within]);
    let round = b.math(Subtract, &[V::Num(1.0), squared]);
    let round = b.math(Maximum, &[round, V::Num(0.0)]);
    let round = b.math(Sqrt, &[round]);
    let round = b.math(Multiply, &[round, stripe]);
    let random = b.random(index, V::Num(0.0), V::Num(0.0));
    b.finish(&[stripe, round, random])
}

pub(super) const TREAD_PLATE: Spec = Spec {
    key: "tread_plate",
    label: "Tread Plate",
    description: "The raised lozenges of tread plate - checker plate, diamond plate - each at right angles to its neighbours. \
                  Wire Height into a Bump's Height; Lozenge is the mask.",
    keywords: "checker plate, diamond plate, chequer plate, floor plate, anti-slip, industrial, durbar",
    inputs: &[
        GroupInput { name: "Spacing (mm)", default: 30.0, min: 2.0, max: 500.0, tip: "From one lozenge's middle to the next" },
        GroupInput { name: "Length", default: 0.8, min: 0.1, max: 1.2, tip: "A lozenge's length, as a share of the spacing" },
        GroupInput { name: "Width", default: 0.22, min: 0.02, max: 0.6, tip: "A lozenge's width, as a share of the spacing" },
        PLANE,
    ],
    outputs: &[
        GroupOutput { name: "Height", ty: SocketType::Float, tip: HEIGHT },
        GroupOutput { name: "Lozenge", ty: SocketType::Float, tip: "1 on a lozenge, 0 on the plate between" },
    ],
    build: tread_plate,
};

fn tread_plate() -> Expansion {
    let mut b = Build::new(TREAD_PLATE.inputs.len(), TREAD_PLATE.outputs.len());
    let (spacing, length, width, plane) = (V::In(0), V::In(1), V::In(2), V::In(3));
    let (u, v) = b.on_plane(plane);
    let spacing = b.metres(spacing);
    let (qx, qy) = (b.math(Divide, &[u, spacing]), b.math(Divide, &[v, spacing]));
    let (column, row) = (b.math(Floor, &[qx]), b.math(Floor, &[qy]));
    let fx = b.math(Fraction, &[qx]);
    let fx = b.math(Subtract, &[fx, V::Num(0.5)]);
    let fy = b.math(Fraction, &[qy]);
    let fy = b.math(Subtract, &[fy, V::Num(0.5)]);
    // Every other square's lozenge leans the other way: along one diagonal
    // or the other.
    let sum = b.math(Add, &[column, row]);
    let odd = b.math(FlooredModulo, &[sum, V::Num(2.0)]);
    let way = b.math(MultiplyAdd, &[odd, V::Num(2.0), V::Num(-1.0)]);
    let leaning = b.math(Multiply, &[fy, way]);
    let along = b.math(Add, &[fx, leaning]);
    let along = b.math(Multiply, &[along, V::Num(std::f32::consts::FRAC_1_SQRT_2)]);
    let across = b.math(Multiply, &[fx, way]);
    let across = b.math(Subtract, &[fy, across]);
    let across = b.math(Multiply, &[across, V::Num(std::f32::consts::FRAC_1_SQRT_2)]);
    // An ellipse of that length and width: its height a round back.
    let half_length = b.math(Multiply, &[length, V::Num(0.5)]);
    let half_width = b.math(Multiply, &[width, V::Num(0.5)]);
    let (a, c) = (b.math(Divide, &[along, half_length]), b.math(Divide, &[across, half_width]));
    let (aa, cc) = (b.math(Multiply, &[a, a]), b.math(Multiply, &[c, c]));
    let out = b.math(Add, &[aa, cc]);
    let lozenge = b.math(LessThan, &[out, V::Num(1.0)]);
    let height = b.math(Subtract, &[V::Num(1.0), out]);
    let height = b.math(Maximum, &[height, V::Num(0.0)]);
    let height = b.math(Sqrt, &[height]);
    b.finish(&[height, lozenge])
}
