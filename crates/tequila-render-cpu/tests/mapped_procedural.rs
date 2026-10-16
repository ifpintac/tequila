//! A procedural pattern keeps its real-world size through every kind of
//! mapping, on a part whose mesh is in other units than the scene.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::procedural::{Procedural, WeavePattern};
use tequila_texture::projection::{Mapping, Projection};
use std::sync::Arc;

fn render(projection: Option<Projection>) -> Framebuffer {
    let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
    let output = graph.output();
    let pattern = graph.add_node(Node::new(NodeKind::Procedural {
        procedural: Box::new(Procedural::Weave { pattern: WeavePattern::Twill, frequency: 4.0, width: 0.8, variance: 0.0 }),
        seed: 1,
    }));
    if let Some(projection) = projection {
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping { projection, real_world_units: true, ..Default::default() },
        }));
        graph.connect(mapping, pattern, Socket::Input(0));
    }
    graph.connect(pattern, output, Socket::Param(Param::BaseColor));
    let program = Arc::new(graph.compile().expect("compiles"));

    let scene = SceneSnapshot::new(
        // Modelled in centimetres, placed at a hundredth: half a metre across
        // in the scene, as a glTF from a centimetre tool arrives.
        vec![Arc::new(tequila_scene::demo::uv_sphere(50.0, 64, 32))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::from_matrix(tequila_core::math::Mat4::scale(vec3(0.01, 0.01, 0.01))).unwrap())],
        vec![MaterialType::Diffuse.defaults()],
        Arc::new(Environment::uniform(Spectrum::ONE)),
    )
    .with_programs(vec![Some(program)], Arc::new(TextureTable::new()));
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.3, 0.4, 2.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)),
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 8);
    framebuffer
}

/// The spread of brightness over the middle of the sphere, in 4x4-pixel
/// blocks: a pattern the size asked for survives the averaging, and a
/// pattern far finer than a pixel — noise, however spread — does not.
fn spread(image: &Framebuffer) -> f32 {
    let values: Vec<f32> = (0..6)
        .flat_map(|by| (0..6).map(move |bx| (bx, by)))
        .map(|(bx, by)| {
            let mut sum = 0.0;
            for y in 0..4 {
                for x in 0..4 {
                    sum += image.pixel(12 + bx * 4 + x, 12 + by * 4 + y).mean().luminance();
                }
            }
            sum / 16.0
        })
        .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt() / mean.max(1e-6)
}

/// Four threads per scene unit are two across a half-metre sphere, however
/// its mesh is scaled. Through a real-world mapping the pattern was measured
/// in the mesh's own centimetres — four threads per centimetre, two hundred
/// across the sphere — and averaged to flat grey:
/// the Quest headset's strap weave vanished this way. Unmapped, the pattern
/// was right all along.
#[test]
fn a_pattern_keeps_its_size_through_every_mapping() {
    let unmapped = spread(&render(None));
    assert!(unmapped > 0.05, "the control shows no pattern: {unmapped}");
    for projection in [Projection::Planar, Projection::TriPlanar] {
        let mapped = spread(&render(Some(projection)));
        assert!(mapped > 0.05, "through a {projection:?} mapping the pattern spreads only {mapped}, against {unmapped} unmapped");
    }
}
