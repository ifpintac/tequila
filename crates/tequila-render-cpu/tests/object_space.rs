//! A pattern belongs to the part, not to the room.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §6 says procedurals are evaluated in **object
//! space**, and the code says so too — `shade.rs` carries the comment "Object
//! space by default (§6), so a procedural does not swim across a part as the
//! part moves". Every unit test of the procedurals passes a position in
//! directly and so agrees with that by construction.
//!
//! None of them can see where the position comes from. This renders the same
//! part twice, moved between renders, with the camera moved by exactly the same
//! amount so the framing is identical. If the pattern lives in object space the
//! two images are the same picture. If it lives in world space the part slides
//! through its own grain.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Mat4, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::gradient::{Gradient, Stop};
use tequila_texture::noise::Fractal;
use tequila_texture::procedural::{Axis, Procedural};
use std::sync::Arc;

fn quad() -> tequila_render_core::scene::Mesh {
    let positions = vec![
        vec3(-1.0, -1.0, 0.0),
        vec3(1.0, -1.0, 0.0),
        vec3(1.0, 1.0, 0.0),
        vec3(-1.0, 1.0, 0.0),
    ];
    tequila_render_core::scene::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3])
        .with_normals(vec![Vec3::Z; 4])
        .with_uvs(vec![
            tequila_core::math::vec2(0.0, 0.0),
            tequila_core::math::vec2(1.0, 0.0),
            tequila_core::math::vec2(1.0, 1.0),
            tequila_core::math::vec2(0.0, 1.0),
        ])
}

/// Marble banded across X, with the turbulence off so the field is exactly
/// `sin(x * 5)` — stripes with a period of about 1.26 units, wide enough to see
/// at this resolution and narrow enough that a shift of a third of a unit lands
/// somewhere else entirely.
fn banded_graph() -> MaterialGraph {
    let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
    let output = graph.output();
    let node = graph.add_node(Node::new(NodeKind::Procedural {
        procedural: Box::new(Procedural::Marble {
            axis: Axis::X,
            frequency: 5.0,
            distortion: 0.0,
            fractal: Fractal::default(),
        }),
        seed: 7,
    }));
    let ramp = graph.add_node(Node::new(NodeKind::Gradient {
        gradient: Box::new(Gradient::new(vec![
            Stop {
                at: 0.0,
                color: Spectrum::splat(0.03),
            },
            Stop {
                at: 1.0,
                color: Spectrum::splat(0.95),
            },
        ])),
    }));
    graph.connect(node, ramp, Socket::Input(0));
    graph.connect(ramp, output, Socket::Param(Param::BaseColor));
    graph
}

/// Render the banded quad, with both the part and the camera displaced by
/// `shift`. The camera moves with the part, so the part occupies exactly the
/// same pixels in both renders and only the pattern can differ.
fn render_shifted(shift: Vec3) -> Framebuffer {
    let transform =
        Transform::from_matrix(Mat4::translation(shift)).expect("a translation is invertible");
    let scene = SceneSnapshot::new(
        vec![Arc::new(quad())],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(transform),
        ],
        vec![MaterialType::Diffuse.defaults()],
        Arc::new(Environment::uniform(Spectrum::splat(1.0))),
    )
    .with_programs(
        vec![Some(Arc::new(banded_graph().compile().expect("compiles")))],
        Arc::new(TextureTable::new()),
    );

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");

    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            shift + vec3(0.0, 0.0, 3.2),
            shift,
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        32,
    );
    framebuffer
}

/// The mean absolute difference between two renders of the same framing.
fn difference(a: &Framebuffer, b: &Framebuffer) -> f32 {
    let mut total = 0.0;
    for y in 0..48 {
        for x in 0..48 {
            total += (a.pixel(x, y).mean().r - b.pixel(x, y).mean().r).abs();
        }
    }
    total / (48.0 * 48.0)
}

/// Moving a part must not move the pattern through it.
///
/// A third of a unit is a quarter of a band: if the pattern is fixed to the
/// world, the stripes visibly march across the part while it moves. The
/// tolerance is for sampling noise between two independent renders, not for
/// drift — a world-space pattern differs here by a large fraction of the full
/// black-to-white range, an order of magnitude above it.
#[test]
fn a_procedural_does_not_swim_when_its_part_moves() {
    let still = render_shifted(Vec3::ZERO);
    let moved = render_shifted(vec3(0.37, 0.0, 0.0));

    // The control: the pattern really is visible in the image, so that
    // "identical" below cannot be satisfied by two identically blank renders.
    let mut lightest: f32 = 0.0;
    let mut darkest: f32 = 1.0;
    for y in 12..36 {
        for x in 12..36 {
            let v = still.pixel(x, y).mean().r;
            lightest = lightest.max(v);
            darkest = darkest.min(v);
        }
    }
    assert!(
        lightest - darkest > 0.2,
        "the bands are not visible in the render at all (range {darkest} to \
         {lightest}); this test would pass on a blank image"
    );

    let drift = difference(&still, &moved);
    assert!(
        drift < 0.02,
        "moving the part by 0.37 units changed the image by {drift} per pixel: \
         the pattern is anchored to the world, not to the part, and §6 says it \
         must be the part"
    );
}
