//! Cycles' bump looks nearly the same however small the part is in the frame.
//!
//! The twin of `tequila-render-cpu/tests/bump_scale.rs`. Cycles' bump node
//! measures a height's slope a tenth of the ray's footprint along by default,
//! which smooths relief smaller than a pixel: these tiny polished pyramids
//! read 0.74 from above at 32 pixels across and 0.37 at 256. Translated with
//! a filter width of a hundredth ([`translate::BUMP_FILTER_WIDTH`]), 0.36 and
//! 0.32.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, Value};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::nodes::ShaderNode;
use tequila_materials::openpbr::Material;
use tequila_render_core::camera::Camera;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cycles::{translate, Session};
use tequila_texture::cycles::ops::MathOp;

const PITCH: f32 = 0.0006;
const TILE: f32 = 0.048;
/// A hundredth of a footprint still smooths some of relief this small: from
/// above, 0.358 at 32 pixels across and 0.325 at 256.
const TOLERANCE: f32 = 0.15;

/// A flat tile in the XZ plane, facing up.
fn tile() -> Mesh {
    let h = TILE * 0.5;
    let positions = vec![vec3(-h, 0.0, -h), vec3(h, 0.0, -h), vec3(h, 0.0, h), vec3(-h, 0.0, h)];
    Mesh::new(positions, vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4])
        .with_uvs(vec![vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)])
}

fn math(graph: &mut MaterialGraph, op: MathOp, a: (NodeId, u8), b: Option<f32>, c: Option<f32>) -> NodeId {
    let mut node = Node::new(NodeKind::Node(ShaderNode::Math { op, clamp: false }));
    if let Some(b) = b {
        node = node.with_literal(Socket::Input(1), Value::Float(b));
    }
    if let Some(c) = c {
        node = node.with_literal(Socket::Input(2), Value::Float(c));
    }
    let id = graph.add_node(node);
    graph.connect_output(a.0, a.1, id, Socket::Input(0));
    id
}

/// Polished metal cut with a grid of square pyramids, PITCH apart, a third
/// of PITCH tall, as the library's Clous de Paris draws them.
fn hobnail() -> (Material, tequila_materials::graph::CompiledGraph) {
    let mut graph = MaterialGraph::from_material(Material {
        base_color: Spectrum::splat(0.9),
        base_metalness: 1.0,
        specular_roughness: 0.05,
        ..MaterialType::Metal.defaults()
    });
    let coordinates = graph.add_node(Node::new(NodeKind::Node(ShaderNode::TextureCoordinate)));
    let xyz = graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateXyz)));
    graph.connect_output(coordinates, 3, xyz, Socket::Input(0));
    // Each axis: how far from the middle of its cell, 0 to 0.5.
    let mut off = Vec::new();
    for axis in [0u8, 2] {
        let cells = math(&mut graph, MathOp::Multiply, (xyz, axis), Some(1.0 / PITCH), None);
        let within = math(&mut graph, MathOp::Fraction, (cells, 0), None, None);
        let centred = math(&mut graph, MathOp::Subtract, (within, 0), Some(0.5), None);
        off.push(math(&mut graph, MathOp::Absolute, (centred, 0), None, None));
    }
    let farther = math(&mut graph, MathOp::Maximum, (off[0], 0), None, None);
    graph.connect_output(off[1], 0, farther, Socket::Input(1));
    // 1 at a pyramid's tip, 0 round its foot.
    let height = math(&mut graph, MathOp::MultiplyAdd, (farther, 0), Some(-2.0), Some(1.0));
    let bump = graph.add_node(Node::new(NodeKind::Bump { depth: PITCH * 0.32 }));
    graph.connect(height, bump, Socket::Input(0));
    let output = graph.output();
    graph.connect(bump, output, Socket::Param(Param::Bump));
    let program = graph.compile().expect("compiles");
    (graph.material().clone(), program)
}

/// The middle of the frame, from `elevation` degrees above the tile.
fn brightness(elevation: f32, size: usize, samples: u32) -> f32 {
    let (material, program) = hobnail();
    let scene = SceneSnapshot::new(
        vec![Arc::new(tile())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![material],
        Arc::new(tequila_scene::demo::studio_environment()),
    )
    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
    let e = elevation.to_radians();
    let eye = vec3(0.0, e.sin(), e.cos()) * 0.06;
    let up = if elevation > 80.0 { vec3(0.0, 0.0, -1.0) } else { Vec3::Y };
    let lighting = IntegratorConfig::product();
    let view = translate::View {
        camera: Camera::look_at(eye, Vec3::ZERO, up),
        width: size,
        height: size,
        transparent: false,
        ground_illumination: false,
        light_materials: true,
        light_groups: false,
        shadow_glass: lighting.shadow_glass(),
        manifold_caustics: false,
    };
    let mut session = Session::new("CPU", 0, None).expect("a Cycles session");
    translate::build(&mut session, &scene, &view).expect("translates");
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, &lighting)).expect("configures");
    let rgba = session.render(size, size, samples, |_, _| {}).expect("renders");
    // The middle half: whole pixels at every size here, so each size
    // measures exactly the same part of the tile.
    let (lo, hi) = (size / 4, size * 3 / 4);
    let mut sum = 0.0f64;
    for y in lo..hi {
        for x in lo..hi {
            let i = (y * size + x) * 4;
            sum += (0.2126 * rgba[i] + 0.7152 * rgba[i + 1] + 0.0722 * rgba[i + 2]) as f64;
        }
    }
    (sum / ((hi - lo) * (hi - lo)) as f64) as f32
}

#[test]
fn tiny_relief_is_nearly_as_bright_far_off_as_close_up() {
    if tequila_render_cycles::available().is_err() {
        return;
    }
    for elevation in [85.0, 60.0] {
        // About half a pixel per pyramid, then about four.
        let far = brightness(elevation, 32, 1024);
        let near = brightness(elevation, 256, 16);
        assert!(
            (far / near - 1.0).abs() < TOLERANCE,
            "seen from {elevation} degrees, the pyramids read {far:.4} at 32 pixels across and {near:.4} at 256"
        );
    }
}
