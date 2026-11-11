//! Stochastic tiling hides what it says it hides: shaded along a line
//! across many patches, a smooth picture stays smooth - no seam where one
//! patch gives way to the next - and no longer comes round again a repeat
//! further on.

use std::sync::Arc;
use tequila_core::math::{vec2, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, TextureId, Value};
use tequila_materials::groups::NodeGroup;
use tequila_materials::material_type::Param;
use tequila_materials::nodes::ShaderNode;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::image::{ColorSpace, Image};
use tequila_texture::projection::{Mapping, PointWorld, SurfacePoint};
use tequila_texture::texture::{MapRole, Texture};

/// How many times the picture repeats across the UVs.
const REPEATS: f32 = 4.0;

/// A picture that tiles and has no edge in it: a slow wave across in red,
/// another up in green.
fn picture() -> Image {
    let n = 64usize;
    let mut data = Vec::with_capacity(n * n * 3);
    for y in 0..n {
        for x in 0..n {
            let (u, v) = ((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32);
            let tau = std::f32::consts::TAU;
            data.extend([0.5 + 0.4 * (tau * u).sin(), 0.5 + 0.4 * (tau * v).sin(), 0.5]);
        }
    }
    Image { width: n, height: n, channels: 3, color_space: ColorSpace::LinearSrgb, data }
}

/// The group, three readings of the picture where it says, and the two
/// Mixes that make one of them: the graph, the first reading, and the last
/// Mix.
fn tiled() -> (MaterialGraph, NodeId, NodeId) {
    let mut graph = MaterialGraph::from_material(Default::default());
    let tiling = graph.add_node(Node::new(NodeKind::Group(NodeGroup::StochasticTiling)).with_literal(Socket::Input(0), Value::Float(REPEATS)));
    let readings: Vec<NodeId> = (0..3u8)
        .map(|output| {
            let image = graph.add_node(Node::new(NodeKind::ImageTexture { texture: TextureId(1), fallback: Value::Float(0.0) }));
            graph.connect_output(tiling, output, image, Socket::Input(0));
            image
        })
        .collect();
    let mut under = readings[0];
    for index in 0..2u8 {
        let mix = graph.add_node(Node::new(NodeKind::Node(ShaderNode::from_key("mix").expect("the Mix node"))));
        graph.connect_output(tiling, 3 + index, mix, Socket::Input(0));
        graph.connect_output(under, 0, mix, Socket::Input(1));
        graph.connect_output(readings[index as usize + 1], 0, mix, Socket::Input(2));
        under = mix;
    }
    (graph, readings[0], under)
}

/// The colour `from` gives the surface at each of `uvs`.
fn colours(graph: &MaterialGraph, from: NodeId, uvs: &[Vec2]) -> Vec<Spectrum> {
    let mut graph = graph.clone();
    let surface = graph.output();
    graph.connect_output(from, 0, surface, Socket::Param(Param::BaseColor));
    let program = graph.compile().expect("compiles");
    let mut textures = TextureTable::new();
    textures.insert(TextureId(1), Arc::new(Texture::from_linear(picture(), MapRole::Color, Mapping::default())));
    let mut registers = Registers::new();
    let mut scratch = tequila_materials::openpbr::Material::default();
    uvs.iter()
        .map(|uv| {
            let point = SurfacePoint {
                position: Vec3::ZERO,
                normal: Vec3::Y,
                uv: *uv,
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: Spectrum::ONE,
                world: PointWorld { normal: Vec3::Y, true_normal: Vec3::Y, incoming: Vec3::Y, ..Default::default() },
            };
            shade(&program, &textures, &point, &mut registers, &mut scratch).base_color
        })
        .collect()
}

fn apart(a: Spectrum, b: Spectrum) -> f32 {
    (a.r - b.r).abs().max((a.g - b.g).abs()).max((a.b - b.b).abs())
}

#[test]
fn a_picture_tiled_stochastically_shows_no_seam() {
    // A line across the UVs, at a slant so it crosses the squares of all
    // three grids, in steps far smaller than a patch.
    let steps = 40_000;
    let line: Vec<Vec2> = (0..steps).map(|i| i as f32 / steps as f32).map(|t| vec2(0.03 + 0.94 * t, 0.07 + 0.71 * t)).collect();
    let (graph, first, mixed) = tiled();
    let jump = |colours: &[Spectrum]| colours.windows(2).map(|pair| apart(pair[0], pair[1])).fold(0.0f32, f32::max);

    // One reading alone jumps where a patch ends: the test would see a seam.
    let alone = jump(&colours(&graph, first, &line));
    assert!(alone > 0.1, "a single reading has no seam to hide ({alone})");
    // The three blended do not.
    let together = jump(&colours(&graph, mixed, &line));
    assert!(together < 0.01, "the blended readings jump by {together} between neighbouring points");
}

#[test]
fn a_picture_tiled_stochastically_does_not_come_round_again() {
    // Points over the UVs, and the same points one repeat of the picture
    // along: tiled plainly, each pair is the same place in the picture.
    let points: Vec<Vec2> = (0..400).map(|i| vec2(0.05 + 0.6 * ((i * 37 % 400) as f32 / 400.0), 0.05 + 0.9 * (i as f32 / 400.0))).collect();
    let along: Vec<Vec2> = points.iter().map(|uv| vec2(uv.x + 1.0 / REPEATS, uv.y)).collect();
    let (graph, _, mixed) = tiled();
    let (here, there) = (colours(&graph, mixed, &points), colours(&graph, mixed, &along));
    let mean = here.iter().zip(&there).map(|(a, b)| apart(*a, *b)).sum::<f32>() / points.len() as f32;
    assert!(mean > 0.08, "a repeat along, the picture is the same again (apart by {mean} on average)");
    // And it is still the picture: its colours, not a grey.
    let spread = here.iter().map(|c| (c.r - 0.5).abs()).sum::<f32>() / here.len() as f32;
    assert!(spread > 0.08, "the blend has flattened the picture to grey ({spread})");
}
