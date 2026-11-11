//! The finish patterns keep to what they say: every output between nought
//! and one, a seed another arrangement, an amount more or less of the
//! thing, and pieces that are each one piece.

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::Param;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::projection::{PointWorld, SurfacePoint};

const FINISHES: [NodeGroup; 12] = [
    NodeGroup::Hammered,
    NodeGroup::OrangePeel,
    NodeGroup::MouldTexture,
    NodeGroup::Droplets,
    NodeGroup::Cracks,
    NodeGroup::Terrazzo,
    NodeGroup::Patches,
    NodeGroup::Speckle,
    NodeGroup::Stucco,
    NodeGroup::Dent,
    NodeGroup::Smoke,
    NodeGroup::Smudges,
];

/// A side of the patch looked at, in the group's own sizes, and samples
/// along it.
const SIZES: f32 = 30.0;
const SAMPLES: usize = 300;

/// One of a group's outputs over a flat patch `SIZES` of its first size
/// across.
fn read(group: NodeGroup, inputs: &[(&str, f32)], output: &str) -> Vec<f32> {
    let mut graph = MaterialGraph::from_material(Default::default());
    let mut node = Node::new(NodeKind::Group(group));
    for (name, value) in inputs {
        let index = group.inputs().iter().position(|i| i.name == *name).unwrap_or_else(|| panic!("{group:?} has no input {name}"));
        node = node.with_literal(Socket::Input(index as u8), Value::Float(*value));
    }
    let pattern = graph.add_node(node);
    let surface = graph.output();
    let index = group.outputs().iter().position(|o| o.name == output).unwrap_or_else(|| panic!("{group:?} has no output {output}"));
    graph.connect_output(pattern, index as u8, surface, Socket::Param(Param::SpecularRoughness));
    let program = graph.compile().expect("compiles");
    let textures = TextureTable::new();
    let mut registers = Registers::new();
    let mut scratch = tequila_materials::openpbr::Material::default();
    let patch = group.inputs()[0].default * 0.001 * SIZES;
    (0..SAMPLES * SAMPLES)
        .map(|i| {
            let at = |index: usize| (index as f32 + 0.5) / SAMPLES as f32 * patch;
            let position = vec3(at(i % SAMPLES), 0.013, at(i / SAMPLES));
            let point = SurfacePoint {
                position,
                normal: Vec3::Y,
                uv: vec2(0.0, 0.0),
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: PointWorld { position, normal: Vec3::Y, true_normal: Vec3::Y, incoming: Vec3::Y, ..Default::default() },
            };
            shade(&program, &textures, &point, &mut registers, &mut scratch).specular_roughness
        })
        .collect()
}

fn mean(values: &[f32]) -> f32 {
    values.iter().map(|v| f64::from(*v)).sum::<f64>() as f32 / values.len() as f32
}

#[test]
fn every_finish_keeps_between_nought_and_one_and_is_not_one_grey() {
    for group in FINISHES {
        for output in group.outputs() {
            let values = read(group, &[], output.name);
            assert!(values.iter().all(|v| (0.0..=1.0).contains(v)), "{group:?}'s {} leaves 0 to 1", output.name);
            let average = mean(&values);
            let spread = (values.iter().map(|v| (v - average) * (v - average)).sum::<f32>() / values.len() as f32).sqrt();
            assert!(spread > 0.02, "{group:?}'s {} is one grey ({average} throughout)", output.name);
        }
    }
}

#[test]
fn a_seed_is_another_arrangement_of_the_same_thing() {
    for group in FINISHES {
        if !group.inputs().iter().any(|i| i.name == "Seed") {
            continue;
        }
        let output = group.outputs()[0].name;
        let (first, again, other) = (read(group, &[("Seed", 0.0)], output), read(group, &[("Seed", 0.0)], output), read(group, &[("Seed", 7.0)], output));
        assert_eq!(first, again, "{group:?} is not the same twice");
        // Of where there is any of it in either, most is not where it was.
        let either = first.iter().zip(&other).filter(|(a, b)| **a > 0.05 || **b > 0.05).count().max(1);
        let moved = first.iter().zip(&other).filter(|(a, b)| (**a - **b).abs() > 0.05).count() as f32 / either as f32;
        assert!(moved > 0.5, "{group:?} with another seed is the same arrangement ({moved} of it moved)");
        // The same thing: about as much of it.
        assert!((mean(&first) - mean(&other)).abs() < 0.12, "{group:?}: {} of it, then {}", mean(&first), mean(&other));
    }
}

#[test]
fn an_amount_is_more_or_less_of_the_thing() {
    for (group, output) in [
        (NodeGroup::Droplets, "Drop"),
        (NodeGroup::Terrazzo, "Chip"),
        (NodeGroup::Patches, "Patch"),
        (NodeGroup::Speckle, "Speck"),
        (NodeGroup::Stucco, "Height"),
        (NodeGroup::Smudges, "Smudge"),
    ] {
        let covered = |amount: f32| mean(&read(group, &[("Amount", amount)], output));
        let (none, some, most) = (covered(0.0), covered(0.5), covered(1.0));
        assert!(none < 0.03, "{group:?} with none of it covers {none}");
        assert!(some > none + 0.02 && most > some * 1.25, "{group:?}: {none}, {some}, {most}");
        // Where it is a count of pieces, half as many are half as much.
        if matches!(group, NodeGroup::Droplets | NodeGroup::Terrazzo | NodeGroup::Speckle) {
            assert!((some / most - 0.5).abs() < 0.1, "{group:?}: half the pieces cover {some} of {most}");
        }
    }
    // Half the surface in patches is about half of it.
    let half = mean(&read(NodeGroup::Patches, &[("Amount", 0.5), ("Softness", 0.0)], "Patch"));
    assert!((half - 0.5).abs() < 0.12, "{half}");
    // More dented is lower on the whole.
    let height = |amount: f32| mean(&read(NodeGroup::Dent, &[("Amount", amount)], "Height"));
    assert!(height(0.0) > 0.9 && height(1.0) < height(0.0) - 0.15, "{} and {}", height(0.0), height(1.0));
}

#[test]
fn cracks_are_as_wide_as_they_say_and_a_piece_is_one_piece() {
    // Cracks a twentieth of a piece's width take a share of the surface -
    // more than a tenth, the pieces being cut through where the surface
    // happens to pass and their sides aslant to it - and twice as wide,
    // twice the share.
    let share = |width: f32| mean(&read(NodeGroup::Cracks, &[("Size (mm)", 10.0), ("Width (mm)", width), ("Wander", 0.0)], "Crack"));
    let (thin, wide) = (share(0.5), share(1.0));
    assert!(thin > 0.08 && thin < 0.25, "{thin}");
    assert!((wide / thin - 2.0).abs() < 0.35, "twice as wide is twice as much: {thin}, {wide}");
    // Depth is in the crack and nowhere else.
    // (Cracks wide enough here that none slips between two samples.)
    let wide = [("Width (mm)", 5.0)];
    let crack = read(NodeGroup::Cracks, &wide, "Crack");
    let depth = read(NodeGroup::Cracks, &wide, "Depth");
    assert!(crack.iter().zip(&depth).all(|(c, d)| (*c == 1.0) == (*d > 0.0)));
    // Between two samples side by side that no crack parts, the piece's
    // number is the same: a piece is one piece.
    let random = read(NodeGroup::Cracks, &wide, "Random");
    let (mut same, mut pairs) = (0, 0);
    for row in 0..SAMPLES {
        for column in 1..SAMPLES {
            let (a, b) = (row * SAMPLES + column - 1, row * SAMPLES + column);
            if crack[a] == 0.0 && crack[b] == 0.0 {
                pairs += 1;
                same += usize::from(random[a] == random[b]);
            }
        }
    }
    assert!(same as f32 > pairs as f32 * 0.995, "{same} of {pairs} neighbours off a crack are one piece");
    // A chip of terrazzo has one number too.
    let (chip, random) = (read(NodeGroup::Terrazzo, &[], "Chip"), read(NodeGroup::Terrazzo, &[], "Random"));
    let (mut same, mut pairs) = (0, 0);
    for row in 0..SAMPLES {
        for column in 1..SAMPLES {
            let (a, b) = (row * SAMPLES + column - 1, row * SAMPLES + column);
            if chip[a] == 1.0 && chip[b] == 1.0 {
                pairs += 1;
                same += usize::from(random[a] == random[b]);
            }
        }
    }
    assert!(pairs > 1000 && same == pairs, "{same} of {pairs} neighbours on a chip are one chip");
}
