//! The tile groups lay what they say: shaded over a grid of points, every
//! tile is a whole oblong a width across and its length long, lying or
//! standing, with nothing left over between them.

use std::collections::HashMap;
use tequila_core::math::{vec2, vec3, Vec3};
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::Param;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::projection::{PointWorld, SurfacePoint};

/// Tile widths to a side of the patch looked at, and samples to a width.
const WIDTHS: usize = 24;
const SAMPLES: usize = 8;
/// A tile's width, in metres, and how many widths long it is.
const WIDTH: f32 = 0.05;
const LENGTH: usize = 3;

/// The group's Random and Tile at every sample of a patch of floor: with
/// no gap, so that every sample is on a tile.
fn laid(group: NodeGroup) -> Vec<(f32, f32)> {
    let mut graph = MaterialGraph::from_material(Default::default());
    let tiles = graph.add_node(
        Node::new(NodeKind::Group(group))
            .with_literal(Socket::Input(0), Value::Float(WIDTH * 1000.0))
            .with_literal(Socket::Input(1), Value::Float(LENGTH as f32))
            .with_literal(Socket::Input(2), Value::Float(0.0)),
    );
    let surface = graph.output();
    // Random into one parameter and Tile into another, read back below.
    graph.connect_output(tiles, 1, surface, Socket::Param(Param::SpecularRoughness));
    graph.connect_output(tiles, 0, surface, Socket::Param(Param::BaseMetalness));
    let program = graph.compile().expect("compiles");
    let textures = TextureTable::new();
    let mut registers = Registers::new();
    let mut scratch = tequila_materials::openpbr::Material::default();
    let count = WIDTHS * SAMPLES;
    let mut out = Vec::with_capacity(count * count);
    for row in 0..count {
        for column in 0..count {
            // The middle of each sample, on the floor: X across, Z down.
            let at = |index: usize| (index as f32 + 0.5) / SAMPLES as f32 * WIDTH - WIDTHS as f32 * WIDTH * 0.5;
            let position = vec3(at(column), 0.0, at(row));
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
            let material = shade(&program, &textures, &point, &mut registers, &mut scratch);
            out.push((material.specular_roughness, material.base_metalness));
        }
    }
    out
}

/// Every tile that lies wholly inside the patch, as the box of the samples
/// that share its number, in samples: (wide, tall, how many).
fn whole_tiles(samples: &[(f32, f32)]) -> Vec<(usize, usize, usize)> {
    let count = WIDTHS * SAMPLES;
    let mut boxes: HashMap<u32, (usize, usize, usize, usize, usize)> = HashMap::new();
    for (index, (random, _)) in samples.iter().enumerate() {
        let (column, row) = (index % count, index / count);
        let found = boxes.entry(random.to_bits()).or_insert((column, column, row, row, 0));
        *found = (found.0.min(column), found.1.max(column), found.2.min(row), found.3.max(row), found.4 + 1);
    }
    boxes
        .into_values()
        .filter(|b| b.0 > 0 && b.2 > 0 && b.1 < count - 1 && b.3 < count - 1)
        .map(|b| (b.1 - b.0 + 1, b.3 - b.2 + 1, b.4))
        .collect()
}

#[test]
fn herringbone_and_basketweave_are_whole_tiles_with_nothing_between() {
    for group in [NodeGroup::Herringbone, NodeGroup::Basketweave] {
        let samples = laid(group);
        assert!(samples.iter().all(|(_, tile)| *tile == 1.0), "{group:?}: with no gap, every point is on a tile");
        let tiles = whole_tiles(&samples);
        assert!(tiles.len() > 100, "{group:?}: only {} whole tiles in the patch", tiles.len());
        let (long, short) = (LENGTH * SAMPLES, SAMPLES);
        let (mut lying, mut standing) = (0, 0);
        for (wide, tall, count) in &tiles {
            // A whole oblong, every sample of it and no more: no tile is a
            // piece of one, an L, or two that share a number.
            assert_eq!(*count, long * short, "{group:?}: a tile of {count} samples in a box {wide} by {tall}");
            match (*wide, *tall) {
                (w, t) if w == long && t == short => lying += 1,
                (w, t) if w == short && t == long => standing += 1,
                other => panic!("{group:?}: a tile {other:?} samples, not {long} by {short} either way"),
            }
        }
        // Half lie and half stand, in both bonds.
        assert!(lying * 3 > standing * 2 && standing * 3 > lying * 2, "{group:?}: {lying} lying, {standing} standing");
    }
}

/// The two bonds are different floors. Of the planks lying across, how
/// many have a plank beside their long side that lies exactly along them,
/// end to end: in basketweave all but the last of each block, and in
/// herringbone none, where every plank is stepped a width along from the
/// one beside it.
#[test]
fn the_two_bonds_are_not_the_same_floor() {
    let count = WIDTHS * SAMPLES;
    let stacked = |group: NodeGroup| {
        let samples = laid(group);
        // Each tile's first and last column and first row.
        let mut spans: HashMap<u32, (usize, usize, usize)> = HashMap::new();
        for (index, (random, _)) in samples.iter().enumerate() {
            let (column, row) = (index % count, index / count);
            let span = spans.entry(random.to_bits()).or_insert((column, column, row));
            *span = (span.0.min(column), span.1.max(column), span.2.min(row));
        }
        let (mut lying, mut alongside) = (0, 0);
        for (low, high, top) in spans.values().copied() {
            let whole = high - low + 1 == LENGTH * SAMPLES;
            if !whole || low == 0 || high >= count - 1 || top < SAMPLES || top + 2 * SAMPLES >= count {
                continue;
            }
            lying += 1;
            // The tile a width further down, under this one's first column.
            let under = samples[(top + SAMPLES) * count + low].0.to_bits();
            let (under_low, under_high, _) = spans[&under];
            alongside += usize::from((under_low, under_high) == (low, high));
        }
        assert!(lying > 30, "{group:?}: {lying} planks lying across to judge by");
        alongside as f32 / lying as f32
    };
    let (herringbone, basketweave) = (stacked(NodeGroup::Herringbone), stacked(NodeGroup::Basketweave));
    assert!(herringbone < 0.05, "herringbone planks lie alongside one another {herringbone} of the time");
    let wanted = (LENGTH - 1) as f32 / LENGTH as f32;
    assert!((basketweave - wanted).abs() < 0.1, "basketweave planks lie alongside one another {basketweave} of the time, not {wanted}");
}

/// With a joint, the gap is the gap: its share of the floor is what a
/// joint that wide round tiles that size takes.
#[test]
fn the_joint_takes_its_share_of_the_floor() {
    let mut graph = MaterialGraph::from_material(Default::default());
    let gap = 10.0;
    let tiles = graph.add_node(
        Node::new(NodeKind::Group(NodeGroup::Herringbone))
            .with_literal(Socket::Input(0), Value::Float(WIDTH * 1000.0))
            .with_literal(Socket::Input(1), Value::Float(LENGTH as f32))
            .with_literal(Socket::Input(2), Value::Float(gap)),
    );
    let surface = graph.output();
    graph.connect_output(tiles, 0, surface, Socket::Param(Param::BaseMetalness));
    let program = graph.compile().expect("compiles");
    let textures = TextureTable::new();
    let mut registers = Registers::new();
    let mut scratch = tequila_materials::openpbr::Material::default();
    let (mut on, mut all) = (0usize, 0usize);
    for row in 0..400 {
        for column in 0..400 {
            let position = vec3(column as f32 * 0.00137 - 0.27, 0.0, row as f32 * 0.00151 - 0.3);
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
            on += usize::from(shade(&program, &textures, &point, &mut registers, &mut scratch).base_metalness > 0.5);
            all += 1;
        }
    }
    // A tile 50 by 150 mm less 5 mm all round, of the 50 by 150 it is laid in.
    let wanted = (WIDTH * 1000.0 - gap) * (WIDTH * 1000.0 * LENGTH as f32 - gap) / (WIDTH * 1000.0 * WIDTH * 1000.0 * LENGTH as f32);
    let got = on as f32 / all as f32;
    assert!((got - wanted).abs() < 0.02, "{got} of the floor is tile, and {wanted} should be");
}
