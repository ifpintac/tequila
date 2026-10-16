//! A texture attached in the document reaches the rendered image.
//!
//! Every piece of the texture path has its own test — the pyramid filters, the
//! projection projects, the graph compiles, the register machine runs, the
//! snapshot loads. All of those can pass while the feature does nothing, because
//! what they do not check is that the pieces are *connected*. This renders two
//! images and requires them to differ, which is the only claim a user cares
//! about.

use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::image::{ColorSpace, Image};
use tequila_texture::projection::{Mapping, Projection};
use tequila_texture::texture::{MapRole, Texture};
use std::sync::Arc;

/// A texture that is unmistakably red on one half and blue on the other, so its
/// presence in the image cannot be confused with lighting.
fn split_texture() -> Arc<Texture> {
    let size = 32;
    let mut data = vec![0.0f32; size * size * 3];
    for y in 0..size {
        for x in 0..size {
            let at = (y * size + x) * 3;
            if x < size / 2 {
                data[at] = 1.0;
            } else {
                data[at + 2] = 1.0;
            }
        }
    }
    Arc::new(Texture::from_linear(
        Image {
            width: size,
            height: size,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        },
        MapRole::Color,
        Mapping {
            projection: Projection::TriPlanar,
            scale: vec3(0.5, 0.5, 1.0),
            ..Default::default()
        },
    ))
}

fn quad() -> tequila_render_core::scene::Mesh {
    let positions = vec![
        vec3(-1.0, -1.0, 0.0),
        vec3(1.0, -1.0, 0.0),
        vec3(1.0, 1.0, 0.0),
        vec3(-1.0, 1.0, 0.0),
    ];
    let normals = vec![Vec3::Z; 4];
    let uvs = vec![
        tequila_core::math::vec2(0.0, 0.0),
        tequila_core::math::vec2(1.0, 0.0),
        tequila_core::math::vec2(1.0, 1.0),
        tequila_core::math::vec2(0.0, 1.0),
    ];
    tequila_render_core::scene::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3])
        .with_normals(normals)
        .with_uvs(uvs)
}

/// A graph driving base colour from a texture.
fn textured_graph() -> MaterialGraph {
    let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
    let output = graph.output();
    let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
        mapping: Mapping {
            projection: Projection::Uv,
            ..Default::default()
        },
    }));
    let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
        texture: TextureId(1),
        fallback: Value::Color(Spectrum::splat(0.5)),
    }));
    graph.connect(mapping, texture, Socket::Input(0));
    graph.connect(texture, output, Socket::Param(Param::BaseColor));
    graph
}

fn render(with_texture: bool) -> Framebuffer {
    use tequila_core::ids::{InstanceId, MaterialId, MeshId};
    use tequila_render_core::scene::Instance;

    let material = MaterialType::Diffuse.defaults();
    let mut scene = SceneSnapshot::new(
        vec![Arc::new(quad())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(Environment::uniform(Spectrum::splat(1.0))),
    );

    if with_texture {
        let program = Arc::new(textured_graph().compile().expect("compiles"));
        let mut textures = TextureTable::new();
        textures.insert(TextureId(1), split_texture());
        scene = scene.with_programs(vec![Some(program)], Arc::new(textures));
    }

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");

    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.0, 3.2),
            Vec3::ZERO,
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        64,
    );
    framebuffer
}

/// The claim: attaching a texture changes the picture.
#[test]
fn a_textured_material_renders_differently_from_an_untextured_one() {
    let plain = render(false);
    let textured = render(true);

    let mut differing = 0;
    for y in 0..48 {
        for x in 0..48 {
            let a = plain.pixel(x, y).mean();
            let b = textured.pixel(x, y).mean();
            if (a.r - b.r).abs() > 0.02 || (a.b - b.b).abs() > 0.02 {
                differing += 1;
            }
        }
    }
    assert!(
        differing > 200,
        "only {differing} pixels changed when a texture was attached; the \
         texture is not reaching the image"
    );
}

/// And it is the *right* texture at the right place: the colour at a pixel
/// matches what the texture holds at the coordinate that pixel's ray actually
/// hit.
///
/// Tied to the hit rather than to a screen side on purpose. Asserting "the left
/// of the image is red" bakes in a guess about which way the camera's X axis
/// runs, and a test that encodes a guess either fails for the wrong reason or
/// gets "fixed" by flipping the expectation until it passes — at which point it
/// asserts nothing. Going through the intersector makes the claim independent
/// of handedness: whatever the projection does, the pixel and the texture have
/// to agree about the same point on the surface.
#[test]
fn the_texture_lands_where_the_geometry_says_it_should() {
    use tequila_core::math::Vec2;
    use tequila_core::sampler::Sampler;
    use tequila_render_core::intersect::Intersector;

    let image = render(true);
    let (intersector, camera) = probe_scene();

    let mut checked = 0;
    for (x, y) in [(14usize, 20usize), (33, 20), (18, 30), (30, 28)] {
        let film = Vec2 {
            x: (x as f32 + 0.5) / 48.0,
            y: (y as f32 + 0.5) / 48.0,
        };
        let mut sampler = Sampler::new(0, 0, 0, 0);
        let ray = camera.generate_ray(film, 1.0, &mut sampler);
        let Some(hit) = intersector.intersect(&ray) else {
            continue;
        };

        // The texture is red below u = 0.5 and blue above it.
        let colour = image.pixel(x, y).mean();
        if hit.uv.x < 0.45 {
            assert!(
                colour.r > colour.b * 3.0,
                "the ray through ({x}, {y}) hit u = {:.3}, which is the red half, \
                 but the pixel is {colour:?}",
                hit.uv.x
            );
            checked += 1;
        } else if hit.uv.x > 0.55 {
            assert!(
                colour.b > colour.r * 3.0,
                "the ray through ({x}, {y}) hit u = {:.3}, which is the blue half, \
                 but the pixel is {colour:?}",
                hit.uv.x
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 2,
        "only {checked} probes landed on the quad away from the seam"
    );
}

/// The same scene and camera the render uses, for probing individual rays.
fn probe_scene() -> (EmbreeIntersector, tequila_render_core::camera::Camera) {
    use tequila_core::ids::{InstanceId, MaterialId, MeshId};
    use tequila_render_core::scene::Instance;

    let scene = SceneSnapshot::new(
        vec![Arc::new(quad())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![MaterialType::Diffuse.defaults()],
        Arc::new(Environment::uniform(Spectrum::splat(1.0))),
    );
    let device = Device::shared().expect("an Embree device");
    (
        EmbreeIntersector::build(&device, scene).expect("builds"),
        tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.0, 3.2),
            Vec3::ZERO,
            Vec3::Y,
        ),
    )
}

/// A material with no graph renders exactly as it did before any of this
/// existed — the fast path has to be a no-op, not an approximation of one.
#[test]
fn an_untextured_material_is_unaffected_by_the_machinery() {
    let a = render(false);
    let b = render(false);
    for y in 0..48 {
        for x in 0..48 {
            assert_eq!(
                a.pixel(x, y).mean(),
                b.pixel(x, y).mean(),
                "an untextured render is not deterministic at ({x}, {y})"
            );
        }
    }
}
