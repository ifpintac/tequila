//! Anisotropy has a direction, and the direction belongs to the surface.
//!
//! An anisotropic metal reflects a small light as a streak along the
//! direction it is rougher in. That direction runs along the part's own
//! tangents — where its UVs run, as a brushed finish follows the part — and
//! §3.2's Rotation turns it about the normal. Before, the lobe was drawn in a
//! frame built from the normal alone: the streak ran whichever way that frame
//! happened to point, turning the part did not turn it, and Rotation did
//! nothing.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::image::{ColorSpace, Image};
use std::sync::Arc;

const SIZE: usize = 64;

fn brushed(rotation: f32) -> Material {
    Material {
        base_color: Spectrum::ONE,
        base_metalness: 1.0,
        specular_roughness: 0.4,
        specular_roughness_anisotropy: 0.95,
        specular_rotation: rotation,
        ..MaterialType::Anisotropic.defaults()
    }
}

/// Dark, but for one small light just above the camera.
fn spot() -> Environment {
    let (width, height) = (512, 256);
    let towards = vec3(0.0, 0.1, 1.0).normalize();
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let d = tequila_render_core::pins::equirect_direction((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32);
            let lit = if d.dot(towards) > 0.9995 { 400.0 } else { 0.0 };
            data.extend_from_slice(&[lit, lit, lit]);
        }
    }
    Environment::new(Arc::new(Image { width, height, channels: 3, data, color_space: ColorSpace::LinearSrgb }), 1.0)
}

fn render(material: Material, transform: Transform) -> Framebuffer {
    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 128, 64))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(transform)],
        vec![material],
        Arc::new(spot()),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.4), Vec3::ZERO, Vec3::Y),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 64);
    framebuffer
}

/// Which way the highlight runs: the ratio of its spread across the image to
/// its spread down it, from the second moments of its light. Above one it
/// runs across; below one, up and down.
fn across_over_down(image: &Framebuffer) -> f32 {
    let (mut total, mut mx, mut my) = (0.0f64, 0.0f64, 0.0f64);
    let mut weights = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let w = image.pixel(x, y).mean().luminance() as f64;
            weights.push((x as f64, y as f64, w));
            total += w;
            mx += w * x as f64;
            my += w * y as f64;
        }
    }
    assert!(total > 0.0, "the sphere shows no highlight at all");
    let (mx, my) = (mx / total, my / total);
    let (mut sxx, mut syy) = (0.0f64, 0.0f64);
    for (x, y, w) in weights {
        sxx += w * (x - mx) * (x - mx);
        syy += w * (y - my) * (y - my);
    }
    (sxx / syy) as f32
}

#[test]
fn rotation_turns_the_highlight() {
    let unturned = across_over_down(&render(brushed(0.0), Transform::IDENTITY));
    let quarter = across_over_down(&render(brushed(0.5), Transform::IDENTITY));
    assert!(unturned > 2.0, "along the UVs, round the sphere, the streak should run across; it is {unturned:.2}");
    assert!(quarter < 0.5, "a rotation of 0.5 is a quarter turn and should stand the streak up; it is {quarter:.2}");
}

#[test]
fn the_direction_turns_with_the_part() {
    let upright = across_over_down(&render(brushed(0.0), Transform::IDENTITY));
    let turned = across_over_down(&render(
        brushed(0.0),
        Transform::rotation(Vec3::Z, std::f32::consts::FRAC_PI_2),
    ));
    assert!(upright > 2.0, "the upright sphere's streak should run across; it is {upright:.2}");
    assert!(turned < 0.5, "turning the sphere a quarter turn should turn its streak; it is {turned:.2}");
}

/// A brushed sphere with no UVs - a CAD body - whose graph draws the
/// direction it is brushed in: `direction`, in the part's own space.
fn render_drawn(direction: Vec3) -> Framebuffer {
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
    use tequila_materials::material_type::Param;
    let mut bare = tequila_scene::demo::uv_sphere(1.0, 128, 64);
    bare.uvs.clear();
    bare.tangents.clear();
    let mut graph = MaterialGraph::from_material(brushed(0.0));
    let constant = graph.add_node(Node::new(NodeKind::Constant(Value::Vector(direction))));
    let output = graph.output();
    graph.connect(constant, output, Socket::Param(Param::Tangent));
    let program = graph.compile().expect("compiles");
    let scene = SceneSnapshot::new(
        vec![Arc::new(bare)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![graph.material().clone()],
        Arc::new(spot()),
    )
    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()));
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.4), Vec3::ZERO, Vec3::Y),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 64);
    framebuffer
}

/// A part with no UVs is brushed the way its material's graph says, not
/// the way its normal happens to frame it: the streak runs along the drawn
/// direction, up the sphere or across it. Before, a part without UVs had
/// only the normal's frame - across, at the front of this sphere - and a
/// brushed pattern's lines and its highlight could run at right angles.
#[test]
fn a_drawn_tangent_points_the_streak() {
    let up = across_over_down(&render_drawn(Vec3::Y));
    let across = across_over_down(&render_drawn(Vec3::X));
    assert!(up < 0.5, "drawn up the part, the streak should stand up; it is {up:.2}");
    assert!(across > 2.0, "drawn across the part, the streak should run across; it is {across:.2}");
}
