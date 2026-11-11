//! The product patterns lay what they say: shaded over a patch of points,
//! a knurl is as high as its pyramids are, hexagons are all one size with
//! nothing between them, dots and lozenges cover the share their sizes
//! say, and a stripe is one stripe all along it.

use std::collections::HashMap;
use tequila_core::math::{vec2, vec3, Vec2, Vec3};
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::Param;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::projection::{PointWorld, SurfacePoint};

/// A side of the patch looked at, in metres, and samples along it.
const PATCH: f32 = 0.2;
const SAMPLES: usize = 400;

/// Two of a group's outputs at each of `points`: position and UV.
fn read(group: NodeGroup, inputs: &[(&str, f32)], outputs: [&str; 2], points: &[(Vec3, Vec2)]) -> Vec<(f32, f32)> {
    let mut graph = MaterialGraph::from_material(Default::default());
    let mut node = Node::new(NodeKind::Group(group));
    for (name, value) in inputs {
        let index = group.inputs().iter().position(|i| i.name == *name).unwrap_or_else(|| panic!("{group:?} has no input {name}"));
        node = node.with_literal(Socket::Input(index as u8), Value::Float(*value));
    }
    let pattern = graph.add_node(node);
    let surface = graph.output();
    for (name, param) in outputs.into_iter().zip([Param::SpecularRoughness, Param::BaseMetalness]) {
        let index = group.outputs().iter().position(|o| o.name == name).unwrap_or_else(|| panic!("{group:?} has no output {name}"));
        graph.connect_output(pattern, index as u8, surface, Socket::Param(param));
    }
    let program = graph.compile().expect("compiles");
    let textures = TextureTable::new();
    let mut registers = Registers::new();
    let mut scratch = tequila_materials::openpbr::Material::default();
    points
        .iter()
        .map(|(position, uv)| {
            let point = SurfacePoint {
                position: *position,
                normal: Vec3::Y,
                uv: *uv,
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: PointWorld { position: *position, normal: Vec3::Y, true_normal: Vec3::Y, incoming: Vec3::Y, ..Default::default() },
            };
            let material = shade(&program, &textures, &point, &mut registers, &mut scratch);
            (material.specular_roughness, material.base_metalness)
        })
        .collect()
}

/// The middle of each sample of the patch, on the floor: X across, Z down.
fn floor() -> Vec<(Vec3, Vec2)> {
    let at = |index: usize| (index as f32 + 0.5) / SAMPLES as f32 * PATCH - PATCH * 0.5;
    (0..SAMPLES * SAMPLES).map(|i| (vec3(at(i % SAMPLES), 0.0, at(i / SAMPLES)), vec2(0.0, 0.0))).collect()
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0f64, 0usize), |(sum, count), v| (sum + f64::from(v), count + 1));
    (sum / count as f64) as f32
}

#[test]
fn a_knurl_is_grooves_a_pitch_apart_and_pyramids_between() {
    // Straight: the height goes from a groove to a ridge and back in a
    // pitch, and is the same all along the groove.
    let line: Vec<(Vec3, Vec2)> = (0..=40).map(|i| (vec3(i as f32 * 0.0001, 0.0, 0.013), vec2(0.0, 0.0))).collect();
    let straight = read(NodeGroup::Knurl, &[("Pitch (mm)", 2.0), ("Angle", 0.0), ("Flat", 0.0)], ["Height", "Height"], &line);
    assert!(straight[0].0 < 0.02, "a groove at nought: {}", straight[0].0);
    assert!((straight[10].0 - 1.0).abs() < 0.02, "a ridge half a pitch on: {}", straight[10].0);
    assert!(straight[20].0 < 0.02 && (straight[30].0 - 1.0).abs() < 0.02, "and again a pitch on");
    assert!((straight[5].0 - 0.5).abs() < 0.02, "a straight flank between: {}", straight[5].0);

    // Crossed: pyramids, whose mean height is a third of their top's.
    let crossed = read(NodeGroup::Knurl, &[("Pitch (mm)", 2.0), ("Angle", 30.0), ("Flat", 0.0)], ["Height", "Height"], &floor());
    assert!(crossed.iter().all(|(h, _)| (0.0..=1.0).contains(h)));
    let average = mean(crossed.iter().map(|(h, _)| *h));
    assert!((average - 1.0 / 3.0).abs() < 0.02, "pyramids average a third: {average}");
    // Cut flat, more of each is at the top.
    let flat = read(NodeGroup::Knurl, &[("Pitch (mm)", 2.0), ("Angle", 30.0), ("Flat", 0.5)], ["Height", "Height"], &floor());
    let top = flat.iter().filter(|(h, _)| *h > 0.999).count() as f32 / flat.len() as f32;
    assert!((top - 0.25).abs() < 0.02, "half the height cut off leaves a quarter of the area flat: {top}");
}

#[test]
fn hexagons_are_all_one_size_with_nothing_between() {
    let size = 0.02;
    let samples = read(NodeGroup::Hexagons, &[("Size (mm)", size * 1000.0), ("Gap (mm)", 0.0)], ["Random", "Tile"], &floor());
    assert!(samples.iter().all(|(_, tile)| *tile == 1.0), "with no gap everything is hexagon");
    // The samples that share a number are one hexagon: those wholly inside
    // the patch all cover a hexagon's area.
    let mut boxes: HashMap<u32, (usize, usize, usize, usize, usize)> = HashMap::new();
    for (index, (random, _)) in samples.iter().enumerate() {
        let (column, row) = (index % SAMPLES, index / SAMPLES);
        let found = boxes.entry(random.to_bits()).or_insert((column, column, row, row, 0));
        *found = (found.0.min(column), found.1.max(column), found.2.min(row), found.3.max(row), found.4 + 1);
    }
    let whole: Vec<_> = boxes.values().filter(|b| b.0 > 0 && b.2 > 0 && b.1 < SAMPLES - 1 && b.3 < SAMPLES - 1).collect();
    assert!(whole.len() > 50, "only {} whole hexagons", whole.len());
    let sample = PATCH / SAMPLES as f32;
    let area = 3f32.sqrt() * 0.5 * size * size / (sample * sample);
    for b in &whole {
        assert!((b.4 as f32 - area).abs() < area * 0.06, "a hexagon of {} samples, not {area}", b.4);
        // A unit across the flats, and two over root three from point to
        // point.
        let (wide, tall) = ((b.1 - b.0 + 1) as f32 * sample, (b.3 - b.2 + 1) as f32 * sample);
        assert!((wide - size).abs() < 2.5 * sample && (tall - size * 2.0 / 3f32.sqrt()).abs() < 2.5 * sample, "{wide} by {tall}");
    }
    // A joint takes its share, and the edge is measured from it.
    let jointed = read(NodeGroup::Hexagons, &[("Size (mm)", 20.0), ("Gap (mm)", 4.0)], ["Tile", "Edge (mm)"], &floor());
    let share = mean(jointed.iter().map(|(tile, _)| *tile));
    assert!((share - 0.64).abs() < 0.02, "hexagons four fifths as wide cover 0.64: {share}");
    let deepest = jointed.iter().map(|(_, edge)| *edge).fold(0.0f32, f32::max);
    // (To within a sample: none lands on a hexagon's very middle.)
    assert!((deepest - 8.0).abs() < 0.4, "the middle is 8 mm in from the joint: {deepest}");
    assert!(jointed.iter().all(|(tile, edge)| (*tile == 1.0) == (*edge > 0.0)));
}

#[test]
fn dots_and_lozenges_cover_the_share_their_sizes_say() {
    let pi = std::f32::consts::PI;
    // Dots half a spacing wide: on a square grid, pi over sixteen.
    let square = read(NodeGroup::Dots, &[("Spacing (mm)", 10.0), ("Size", 0.5), ("Staggered", 0.0)], ["Dot", "Dome"], &floor());
    let share = mean(square.iter().map(|(dot, _)| *dot));
    assert!((share - pi / 16.0).abs() < 0.01, "{share}");
    // Staggered, the rows are closer by root three over two.
    let staggered = read(NodeGroup::Dots, &[("Spacing (mm)", 10.0), ("Size", 0.5), ("Staggered", 1.0)], ["Dot", "Dome"], &floor());
    let share = mean(staggered.iter().map(|(dot, _)| *dot));
    assert!((share - pi / 16.0 / 0.866).abs() < 0.01, "{share}");
    // The dome is on the dot and nowhere else, and reaches one.
    assert!(staggered.iter().all(|(dot, dome)| (*dot == 1.0) == (*dome > 0.0) && *dome <= 1.0));
    assert!(staggered.iter().map(|(_, dome)| *dome).fold(0.0f32, f32::max) > 0.99);
    // A dot has one number.
    let dotted = read(NodeGroup::Dots, &[("Spacing (mm)", 10.0), ("Size", 0.5), ("Staggered", 1.0)], ["Random", "Dot"], &floor());
    let numbers: std::collections::HashSet<u32> = dotted.iter().filter(|(_, dot)| *dot == 1.0).map(|(random, _)| random.to_bits()).collect();
    let dots = (PATCH / 0.01) * (PATCH / 0.00866);
    assert!((numbers.len() as f32 - dots).abs() < dots * 0.15, "{} numbers for about {dots} dots", numbers.len());

    // Tread plate: an ellipse to a square.
    let plate = read(NodeGroup::TreadPlate, &[("Spacing (mm)", 20.0), ("Length", 0.8), ("Width", 0.2)], ["Lozenge", "Height"], &floor());
    let share = mean(plate.iter().map(|(lozenge, _)| *lozenge));
    assert!((share - pi * 0.4 * 0.1).abs() < 0.01, "{share}");
    assert!(plate.iter().all(|(lozenge, height)| (*lozenge == 1.0) == (*height > 0.0)));
    // Its neighbours lean the other way: along one diagonal of a square
    // there is lozenge where, a square along, there is plate.
    let diagonal = |square: f32| -> Vec<(Vec3, Vec2)> { (1..10).map(|i| i as f32 * 0.02 * 0.1).map(|t| (vec3(square * 0.02 + t, 0.0, t), vec2(0.0, 0.0))).collect() };
    let lean = |square: f32| mean(read(NodeGroup::TreadPlate, &[("Spacing (mm)", 20.0), ("Length", 0.8), ("Width", 0.2)], ["Lozenge", "Height"], &diagonal(square)).iter().map(|(l, _)| *l));
    assert!((lean(0.0) - lean(1.0)).abs() > 0.4, "{} and {}", lean(0.0), lean(1.0));
}

#[test]
fn a_stripe_is_one_stripe_all_along_it() {
    let stripes = read(NodeGroup::Stripes, &[("Width (mm)", 10.0), ("Share", 0.3), ("Angle", 0.0)], ["Stripe", "Random"], &floor());
    let share = mean(stripes.iter().map(|(stripe, _)| *stripe));
    assert!((share - 0.3).abs() < 0.01, "{share}");
    // Unturned, a stripe runs down the patch: a column of samples is all
    // stripe or all gap, and all one number.
    for column in 0..SAMPLES {
        let first = stripes[column];
        assert!((0..SAMPLES).all(|row| stripes[row * SAMPLES + column] == first), "column {column} changes down its length");
    }
    // Turned a quarter, it runs across.
    let turned = read(NodeGroup::Stripes, &[("Width (mm)", 10.0), ("Share", 0.3), ("Angle", 90.0)], ["Stripe", "Random"], &floor());
    let changes = (0..SAMPLES).filter(|row| (1..SAMPLES).any(|column| turned[row * SAMPLES + column].0 != turned[row * SAMPLES].0)).count();
    assert!(changes < SAMPLES / 20, "{changes} rows change along their length");
    // The rib is round: full height along the stripe's middle.
    let ribs = read(NodeGroup::Stripes, &[("Width (mm)", 10.0), ("Share", 0.5), ("Angle", 0.0)], ["Round", "Stripe"], &floor());
    assert!(ribs.iter().all(|(round, stripe)| *round <= 1.0 && (*stripe == 1.0 || *round == 0.0)));
    let average = mean(ribs.iter().filter(|(_, stripe)| *stripe == 1.0).map(|(round, _)| *round));
    assert!((average - std::f32::consts::FRAC_PI_4).abs() < 0.01, "a half circle averages pi over four: {average}");
}

#[test]
fn a_pattern_goes_round_a_knob_and_across_its_uvs() {
    // Round the Y axis: on a cylinder, a straight knurl's grooves are a
    // pitch apart along the surface, whatever the cylinder's size.
    for radius in [0.01f32, 0.05] {
        let pitch = 0.002;
        let step = pitch / radius;
        let round: Vec<(Vec3, Vec2)> = [0.3f32, 0.3 + step * 0.5, 0.3 + step, 0.3 + step * 1.5]
            .into_iter()
            .map(|angle| (vec3(radius * angle.cos(), 0.004, radius * angle.sin()), vec2(0.0, 0.0)))
            .collect();
        let heights = read(NodeGroup::Knurl, &[("Pitch (mm)", pitch * 1000.0), ("Angle", 0.0), ("Flat", 0.0), ("Plane", 4.0)], ["Height", "Height"], &round);
        assert!((heights[0].0 - heights[2].0).abs() < 0.03, "a pitch round, the same height: {heights:?}");
        assert!((heights[1].0 - heights[3].0).abs() < 0.03, "{heights:?}");
        assert!((heights[0].0 + heights[1].0 - 1.0).abs() < 0.03, "half a pitch round, the other side of the flank: {heights:?}");
    }
    // Across the UVs, the pattern follows them and not where the point is.
    let by_uv = |position: Vec3, uv: Vec2| read(NodeGroup::Stripes, &[("Width (mm)", 100.0), ("Share", 0.5), ("Plane", 3.0)], ["Stripe", "Random"], &[(position, uv)])[0];
    assert_eq!(by_uv(vec3(0.0, 0.0, 0.0), vec2(0.52, 0.3)), by_uv(vec3(3.7, 1.0, -2.0), vec2(0.52, 0.3)));
    assert_ne!(by_uv(vec3(0.0, 0.0, 0.0), vec2(0.52, 0.3)).0, by_uv(vec3(0.0, 0.0, 0.0), vec2(0.58, 0.3)).0);
}
