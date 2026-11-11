//! How a surface was finished and what has happened to it since: hammered,
//! sprayed, moulded; wet, cracked, patched, smudged. And the procedurals a
//! 3ds Max user knows by name - Speckle, Stucco, Dent, Smoke - made here
//! of Tequila's own noise.
//!
//! These fill space, as stone does, so they need no plane: any part, of
//! any shape, without UVs. Sizes are in millimetres. A Seed, where there
//! is one, gives another arrangement of the same thing.
//!
//! Built of Noise and Voronoi Textures and Math, so every engine renders
//! them as it renders those.

use super::build::{Build, V};
use super::{GroupInput, GroupOutput, Spec};
use crate::graph::SocketType;
use tequila_texture::cycles::ops::MathOp::*;
use tequila_texture::cycles::ops::VectorMathOp;
use tequila_texture::cycles::voronoi::VoronoiFeature;

const HEIGHT: GroupOutput = GroupOutput { name: "Height", ty: SocketType::Float, tip: "From 0 to 1: into a Bump's Height" };
const SEED: GroupInput = GroupInput { name: "Seed", default: 0.0, min: 0.0, max: 100.0, tip: "Another number, another arrangement" };

/// How far the point is from the middle of the cell it is in, and three
/// numbers of that cell's own.
fn cell(b: &mut Build, scale: V, seed: V) -> (V, V) {
    let node = b.cells(VoronoiFeature::F1, None, scale, seed);
    (V::Node(node, 0), own(b, V::Node(node, 1)))
}

/// Three numbers of a cell's own from its Voronoi Color. The Color is
/// itself where in the cell its middle lies - Cycles makes both of the one
/// hash - so read as it is, a drop's size would go with how the surface
/// cuts it. Through a White Noise it goes with nothing.
fn own(b: &mut Build, colour: V) -> V {
    let noise = b.node(crate::graph::NodeKind::Node(crate::nodes::ShaderNode::WhiteNoise { dims: 3 }));
    b.feed(noise, 0, colour);
    V::Node(noise, 1)
}

pub(super) const HAMMERED: Spec = Spec {
    key: "hammered",
    label: "Hammered",
    description: "Metal beaten with a round hammer: shallow dishes side by side, each a little different. Wire Height into a \
                  Bump's Height; Random is a number for each dish, for a little roughness of its own.",
    keywords: "hammered metal, beaten, planished, dimpled, copper, hand forged, facets",
    inputs: &[GroupInput { name: "Size (mm)", default: 8.0, min: 0.5, max: 200.0, tip: "A hammer mark's width" }, SEED],
    outputs: &[HEIGHT, GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the mark's own" }],
    build: || {
        let mut b = Build::new(2, 2);
        let scale = b.per_metre(V::In(0));
        let (distance, colour) = cell(&mut b, scale, V::In(1));
        // A dish: lowest at the mark's middle, rising to the ridge where
        // it meets the next.
        let out = b.clamped(Multiply, &[distance, V::Num(1.35)]);
        let height = b.math(Multiply, &[out, out]);
        let (random, _, _) = b.split(colour);
        b.finish(&[height, random])
    },
};

pub(super) const ORANGE_PEEL: Spec = Spec {
    key: "orange_peel",
    label: "Orange Peel",
    description: "The gentle waviness of sprayed paint, powder coat and moulded plastic. Wire Height into a Bump's Height, the \
                  Bump shallow: a few hundredths of a millimetre.",
    keywords: "paint texture, powder coat, lacquer, clear coat waviness, moulded plastic, spray",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 1.5, min: 0.1, max: 50.0, tip: "From one rise to the next" },
        GroupInput { name: "Detail", default: 0.5, min: 0.0, max: 4.0, tip: "Finer waviness on the coarse" },
    ],
    outputs: &[HEIGHT],
    build: || {
        let mut b = Build::new(2, 1);
        let scale = b.per_metre(V::In(0));
        let (height, _) = b.noise(None, scale, V::Num(0.0), V::In(1), 0.4, 0.0);
        b.finish(&[height])
    },
};

pub(super) const MOULD_TEXTURE: Spec = Spec {
    key: "mould_texture",
    label: "Mould Texture",
    description: "The fine grain a mould leaves on plastic - spark-eroded, etched: small rounded grains, more or less even. Wire \
                  Height into a Bump's Height.",
    keywords: "spark erosion, EDM, VDI 3400, MT texture, mold tech, etched, textured plastic, grain, matte plastic",
    inputs: &[
        GroupInput { name: "Grain (mm)", default: 0.4, min: 0.02, max: 10.0, tip: "A grain's width" },
        GroupInput { name: "Unevenness", default: 0.4, min: 0.0, max: 1.0, tip: "How much the grains run together into coarser patches" },
    ],
    outputs: &[HEIGHT],
    build: || {
        let mut b = Build::new(2, 1);
        let scale = b.per_metre(V::In(0));
        let (distance, _) = cell(&mut b, scale, V::Num(0.0));
        // A rounded grain on each cell, and a coarser noise under them.
        let grain = b.clamped(Subtract, &[V::Num(1.0), distance]);
        let coarse = b.math(Multiply, &[scale, V::Num(0.35)]);
        let (under, _) = b.noise(None, coarse, V::Num(0.0), V::Num(2.0), 0.5, 0.0);
        let height = b.pick(V::In(1), under, grain);
        b.finish(&[height])
    },
};

pub(super) const DROPLETS: Spec = Spec {
    key: "droplets",
    label: "Droplets",
    description: "Drops of water on a surface: a cold can, a bottle from the fridge, rain on paint. Wire Height into a Bump's \
                  Height; Drop is the mask, for less roughness where it is wet.",
    keywords: "water drops, condensation, rain, dew, wet, beads, sweat, cold drink",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 4.0, min: 0.2, max: 100.0, tip: "The largest drops' width" },
        GroupInput { name: "Amount", default: 0.8, min: 0.0, max: 1.0, tip: "How many of the places a drop could be have one" },
        SEED,
    ],
    outputs: &[
        HEIGHT,
        GroupOutput { name: "Drop", ty: SocketType::Float, tip: "1 on a drop, 0 on the dry surface" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the drop's own" },
    ],
    build: || {
        let mut b = Build::new(3, 3);
        // A drop to a cell, about the cell's middle - which is somewhere
        // in space, not on the surface: the surface cuts each drop where
        // it happens to, so some are seen whole, some as a small cap, and
        // many not at all. The cells are sized so the largest drop, cut
        // through its middle, is the size asked for.
        let scale = b.per_metre(V::In(0));
        let (distance, colour) = cell(&mut b, scale, V::In(2));
        let (size, has, random) = b.split(colour);
        // Each its own size, from half of the largest.
        let radius = b.math(MultiplyAdd, &[size, V::Num(0.25), V::Num(0.25)]);
        let there = b.math(LessThan, &[has, V::In(1)]);
        let out = b.math(Divide, &[distance, radius]);
        let inside = b.math(LessThan, &[out, V::Num(1.0)]);
        let drop = b.math(Multiply, &[inside, there]);
        // A round back.
        let squared = b.math(Multiply, &[out, out]);
        let dome = b.math(Subtract, &[V::Num(1.0), squared]);
        let dome = b.math(Maximum, &[dome, V::Num(0.0)]);
        let dome = b.math(Sqrt, &[dome]);
        let height = b.math(Multiply, &[dome, there]);
        b.finish(&[height, drop, random])
    },
};

pub(super) const CRACKS: Spec = Spec {
    key: "cracks",
    label: "Cracks",
    description: "A net of cracks: dried mud, crazed glaze, old paint, ice. Crack is the mask; Depth runs from 0 at a crack's \
                  edge to 1 down its middle, for a Bump through an Invert; Random is a number for each piece between.",
    keywords: "crackle, craquelure, crazing, dried mud, broken, fractured, old paint, ice, shattered",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 15.0, min: 0.5, max: 1000.0, tip: "A piece's width, from crack to crack" },
        GroupInput { name: "Width (mm)", default: 0.4, min: 0.01, max: 50.0, tip: "A crack's width" },
        GroupInput { name: "Wander", default: 0.3, min: 0.0, max: 1.0, tip: "How far the cracks stray from straight" },
        SEED,
    ],
    outputs: &[
        GroupOutput { name: "Crack", ty: SocketType::Float, tip: "1 in a crack, 0 on the pieces between" },
        GroupOutput { name: "Depth", ty: SocketType::Float, tip: "0 at a crack's edge, 1 down its middle" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the piece's own" },
    ],
    build: || {
        let mut b = Build::new(4, 3);
        let (size_mm, width_mm, wander, seed) = (V::In(0), V::In(1), V::In(2), V::In(3));
        let scale = b.per_metre(size_mm);
        // The space pushed about by a noise, so the cells' straight sides
        // are not.
        let place = b.object();
        let fine = b.math(Multiply, &[scale, V::Num(1.7)]);
        let (_, push) = b.noise(None, fine, seed, V::Num(1.0), 0.5, 0.0);
        let push = b.vector(VectorMathOp::Subtract, push, V::Num(0.5));
        let size = b.metres(size_mm);
        let far = b.math(Multiply, &[size, wander]);
        let push = b.scaled(push, far);
        let place = b.vector(VectorMathOp::Add, place, push);
        let edges = b.cells(VoronoiFeature::DistanceToEdge, Some(place), scale, seed);
        let pieces = b.cells(VoronoiFeature::F1, Some(place), scale, seed);
        // From the nearest crack's middle, in millimetres.
        let from = b.math(Multiply, &[V::Node(edges, 0), size_mm]);
        let half = b.math(Multiply, &[width_mm, V::Num(0.5)]);
        let crack = b.math(LessThan, &[from, half]);
        let across = b.clamped(Divide, &[from, half]);
        let depth = b.math(Subtract, &[V::Num(1.0), across]);
        let numbers = own(&mut b, V::Node(pieces, 1));
        let (random, _, _) = b.split(numbers);
        b.finish(&[crack, depth, random])
    },
};

pub(super) const TERRAZZO: Spec = Spec {
    key: "terrazzo",
    label: "Terrazzo",
    description: "Chips of stone set in a ground: terrazzo, composite stone, recycled plastic, confetti. Chip is the mask of chip \
                  and ground; Random a number for each chip, for a Color Ramp of the stones' colours.",
    keywords: "chips, composite stone, quartz, speckled, confetti, recycled plastic, aggregate, flecks",
    inputs: &[
        GroupInput { name: "Chip (mm)", default: 8.0, min: 0.2, max: 500.0, tip: "A chip's width" },
        GroupInput { name: "Amount", default: 0.6, min: 0.0, max: 1.0, tip: "How many of the places a chip could be have one" },
        GroupInput { name: "Ground", default: 0.3, min: 0.02, max: 0.9, tip: "How much ground is left round each chip" },
        SEED,
    ],
    outputs: &[
        GroupOutput { name: "Chip", ty: SocketType::Float, tip: "1 on a chip, 0 on the ground between" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the chip's own" },
    ],
    build: || {
        let mut b = Build::new(4, 2);
        let scale = b.per_metre(V::In(0));
        let (_, colour) = cell(&mut b, scale, V::In(3));
        let edges = b.cells(VoronoiFeature::DistanceToEdge, None, scale, V::In(3));
        let (has, random, _) = b.split(colour);
        let there = b.math(LessThan, &[has, V::In(1)]);
        // A cell shrunk from its sides: an angular chip.
        let margin = b.math(Multiply, &[V::In(2), V::Num(0.5)]);
        let inside = b.math(GreaterThan, &[V::Node(edges, 0), margin]);
        let chip = b.math(Multiply, &[inside, there]);
        b.finish(&[chip, random])
    },
};

pub(super) const PATCHES: Spec = Spec {
    key: "patches",
    label: "Patches",
    description: "Irregular patches, by how much of the surface they cover: rust, moss, damp, peeling paint, a splat of another \
                  colour. Wire Patch into the Factor of a Mix of two colours, or of a Mix Shader.",
    keywords: "rust, moss, stains, blotches, splat, peeling paint, corrosion, patina, weathering, mask",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 40.0, min: 0.5, max: 5000.0, tip: "A patch's width" },
        GroupInput { name: "Amount", default: 0.4, min: 0.0, max: 1.0, tip: "How much of the surface is patch" },
        GroupInput { name: "Softness", default: 0.1, min: 0.0, max: 1.0, tip: "How gradually a patch gives way at its edge" },
        GroupInput { name: "Detail", default: 3.0, min: 0.0, max: 8.0, tip: "How ragged the edges are" },
        SEED,
    ],
    outputs: &[GroupOutput { name: "Patch", ty: SocketType::Float, tip: "1 on a patch, 0 off it" }],
    build: || {
        let mut b = Build::new(5, 1);
        let scale = b.per_metre(V::In(0));
        let (noise, _) = b.noise(None, scale, V::In(4), V::In(3), 0.55, 0.0);
        // The noise keeps near its middle, so the level it is cut at moves
        // less than the amount does.
        let level = b.math(MultiplyAdd, &[V::In(1), V::Num(-0.7), V::Num(0.85)]);
        let half = b.math(Multiply, &[V::In(2), V::Num(0.25)]);
        let (low, high) = (b.math(Subtract, &[level, half]), b.math(Add, &[level, half]));
        let patch = b.smooth(low, high, noise);
        b.finish(&[patch])
    },
};

pub(super) const SPECKLE: Spec = Spec {
    key: "speckle",
    label: "Speckle",
    description: "Small specks scattered over a ground: an egg, granite, a speckled glaze, flecked plastic. Speck is the mask; \
                  Random a number for each speck.",
    keywords: "specks, flecks, freckles, spots, granite, egg, spatter, splatter, dots random",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 1.5, min: 0.05, max: 200.0, tip: "The largest specks' width" },
        GroupInput { name: "Amount", default: 0.7, min: 0.0, max: 1.0, tip: "How many of the places a speck could be have one" },
        SEED,
    ],
    outputs: &[
        GroupOutput { name: "Speck", ty: SocketType::Float, tip: "1 on a speck, 0 on the ground" },
        GroupOutput { name: "Random", ty: SocketType::Float, tip: "A number from 0 to 1 that is the speck's own" },
    ],
    build: || {
        let mut b = Build::new(3, 2);
        // As Droplets are: a speck to a cell, cut by the surface where it
        // happens to be.
        let scale = b.per_metre(V::In(0));
        let (distance, colour) = cell(&mut b, scale, V::In(2));
        let (size, has, random) = b.split(colour);
        let radius = b.math(MultiplyAdd, &[size, V::Num(0.3), V::Num(0.2)]);
        let there = b.math(LessThan, &[has, V::In(1)]);
        let inside = b.math(LessThan, &[distance, radius]);
        let speck = b.math(Multiply, &[inside, there]);
        b.finish(&[speck, random])
    },
};

pub(super) const STUCCO: Spec = Spec {
    key: "stucco",
    label: "Stucco",
    description: "Plaster thrown on and flattened: raised flats with soft edges, and the wall showing between. Wire Height into \
                  a Bump's Height.",
    keywords: "plaster, render, wall texture, textured wall, trowelled, popcorn ceiling",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 12.0, min: 0.5, max: 1000.0, tip: "A flat's width" },
        GroupInput { name: "Amount", default: 0.45, min: 0.0, max: 1.0, tip: "How much of the wall is raised" },
        GroupInput { name: "Thickness", default: 0.15, min: 0.01, max: 1.0, tip: "How gradually a flat rises at its edge" },
    ],
    outputs: &[HEIGHT],
    build: || {
        let mut b = Build::new(3, 1);
        let scale = b.per_metre(V::In(0));
        let (noise, _) = b.noise(None, scale, V::Num(0.0), V::Num(1.0), 0.5, 0.0);
        let level = b.math(MultiplyAdd, &[V::In(1), V::Num(-0.7), V::Num(0.85)]);
        let half = b.math(Multiply, &[V::In(2), V::Num(0.25)]);
        let (low, high) = (b.math(Subtract, &[level, half]), b.math(Add, &[level, half]));
        let height = b.smooth(low, high, noise);
        b.finish(&[height])
    },
};

pub(super) const DENT: Spec = Spec {
    key: "dent",
    label: "Dent",
    description: "A surface knocked about: scattered dents and dings in what is otherwise flat. Wire Height into a Bump's Height.",
    keywords: "dents, dings, knocks, battered, damaged, pitted, worn metal",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 25.0, min: 0.5, max: 2000.0, tip: "A dent's width" },
        GroupInput { name: "Amount", default: 0.3, min: 0.0, max: 1.0, tip: "How much of the surface is dented" },
        GroupInput { name: "Detail", default: 1.0, min: 0.0, max: 8.0, tip: "Small dents in the large" },
    ],
    outputs: &[HEIGHT],
    build: || {
        let mut b = Build::new(3, 1);
        let scale = b.per_metre(V::In(0));
        let (noise, _) = b.noise(None, scale, V::Num(0.0), V::In(2), 0.5, 0.0);
        // Where the noise falls below a level there is a dent, deeper the
        // further below; above it the surface is flat. The noise keeps
        // near its middle, so a level well under that leaves most of it
        // alone.
        let level = b.math(MultiplyAdd, &[V::In(1), V::Num(0.3), V::Num(0.25)]);
        let floor = b.math(Subtract, &[level, V::Num(0.2)]);
        let height = b.smooth(floor, level, noise);
        b.finish(&[height])
    },
};

pub(super) const SMOKE: Spec = Spec {
    key: "smoke",
    label: "Smoke",
    description: "Wisps and billows: smoke, cloud, a mottled wash of colour, the bloom on a plum. Wire Smoke into the Factor of \
                  a Mix of two colours, or into Opacity.",
    keywords: "clouds, wisps, fog, mottled, turbulence, haze, marbling, vapour",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 60.0, min: 0.5, max: 5000.0, tip: "A billow's width" },
        GroupInput { name: "Detail", default: 5.0, min: 0.0, max: 10.0, tip: "Wisps on the billows" },
        GroupInput { name: "Exponent", default: 1.5, min: 0.2, max: 6.0, tip: "Higher: thinner smoke, more of it clear" },
    ],
    outputs: &[GroupOutput { name: "Smoke", ty: SocketType::Float, tip: "From 0, clear, to 1, thick" }],
    build: || {
        let mut b = Build::new(3, 1);
        let scale = b.per_metre(V::In(0));
        let (noise, _) = b.noise(None, scale, V::Num(0.0), V::In(1), 0.6, 1.2);
        // Stretched about its middle first, so thick is thick.
        let wide = b.clamped(MultiplyAdd, &[noise, V::Num(1.8), V::Num(-0.4)]);
        let smoke = b.math(Power, &[wide, V::In(2)]);
        b.finish(&[smoke])
    },
};

pub(super) const SMUDGES: Spec = Spec {
    key: "smudges",
    label: "Smudges",
    description: "Where a surface has been handled and wiped: uneven patches of dullness, finely streaked. Through a Map Range, \
                  into Roughness: nothing sells a polished surface like a little of this.",
    keywords: "fingerprints, grease, wipe marks, smears, imperfections, roughness map, handled, dust",
    inputs: &[
        GroupInput { name: "Size (mm)", default: 40.0, min: 1.0, max: 2000.0, tip: "A smudge's width" },
        GroupInput { name: "Amount", default: 0.5, min: 0.0, max: 1.0, tip: "How much of the surface is smudged" },
        GroupInput { name: "Streaks", default: 0.5, min: 0.0, max: 1.0, tip: "How strongly the smudges are streaked, as by a cloth" },
        SEED,
    ],
    outputs: &[GroupOutput { name: "Smudge", ty: SocketType::Float, tip: "From 0, clean, to 1, smudged" }],
    build: || {
        let mut b = Build::new(4, 1);
        let scale = b.per_metre(V::In(0));
        let (broad, _) = b.noise(None, scale, V::In(3), V::Num(2.0), 0.5, 0.6);
        let level = b.math(MultiplyAdd, &[V::In(1), V::Num(-0.7), V::Num(0.85)]);
        let (low, high) = (b.math(Subtract, &[level, V::Num(0.12)]), b.math(Add, &[level, V::Num(0.12)]));
        let patch = b.smooth(low, high, broad);
        // The cloth's streaks: a fine noise drawn out along X.
        let place = b.object();
        let stretch = b.combine(V::Num(0.08), V::Num(1.0), V::Num(1.0));
        let drawn = b.vector(VectorMathOp::Multiply, place, stretch);
        let fine = b.math(Multiply, &[scale, V::Num(14.0)]);
        let (streak, _) = b.noise(Some(drawn), fine, V::In(3), V::Num(1.0), 0.5, 0.0);
        let streak = b.math(MultiplyAdd, &[streak, V::Num(1.6), V::Num(-0.3)]);
        let streaked = b.pick(V::In(2), streak, V::Num(1.0));
        let smudge = b.clamped(Multiply, &[patch, streaked]);
        b.finish(&[smudge])
    },
};
