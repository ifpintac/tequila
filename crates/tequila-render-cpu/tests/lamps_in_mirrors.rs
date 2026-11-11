//! A lamp seen in a mirror-smooth surface is the lamp: its shape, its
//! brightness along that way, on every path. On a path the camera saw along
//! it used to show only by its stand-in, a square at the lamp's place glowing
//! evenly - a disc lamp a square in every mirror, a softbox behind a grid as
//! bright from the side as head on - and a lamp the camera does not see had
//! no stand-in, and no mirror showed it.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::ComplexIor;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 48;

/// A metal that mirrors all of the light at every angle.
fn mirror() -> Material {
    let mut mirror = Material::metal(tequila_materials::fresnel::metals::ALUMINIUM, 0.0);
    mirror.metal_ior = ComplexIor { n: Spectrum::splat(1.0), k: Spectrum::splat(1e6) };
    mirror
}

fn glow(radiance: f32) -> Material {
    Material { emission_color: Spectrum::ONE, emission_luminance: radiance, ..MaterialType::Emissive.defaults() }
}

/// The sum of the picture's luminance.
fn total(framebuffer: &Framebuffer) -> f32 {
    let mut sum = 0.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            sum += framebuffer.pixel(x, y).mean().luminance();
        }
    }
    sum
}

fn render(scene: SceneSnapshot, samples: u32) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 1.0, 2.0), Vec3::ZERO, Vec3::Y),
        config: IntegratorConfig::product(),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    for _ in 0..samples {
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
    }
    framebuffer
}

fn floor() -> Mesh {
    Mesh::new(vec![vec3(-6.0, 0.0, -6.0), vec3(6.0, 0.0, -6.0), vec3(6.0, 0.0, 6.0), vec3(-6.0, 0.0, 6.0)], vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4])
}

/// A disc lamp the camera sees, mirrored in the floor, with its stand-in a
/// square as the scene gives it one: the mirror shows the disc, as bright in
/// all as a glowing disc in its place. The stand-in, which the mirror showed
/// before, glowed over the whole square: 4/pi of the disc's light.
#[test]
fn a_mirror_shows_a_disc_lamp_as_the_disc_it_is() {
    let mirrored = vec3(0.0, 1.0, -2.0).normalize();
    let at = mirrored * 2.5;
    let across = Vec3::X;
    let up = (-mirrored).cross(across).normalize();
    let (half_u, half_v) = if (across * 0.4).cross(up * 0.4).dot(-mirrored) > 0.0 { (across * 0.4, up * 0.4) } else { (up * 0.4, across * 0.4) };
    let lamp = AreaLight { position: at, half_u, half_v, radiance: Spectrum::splat(10.0), disc: true, two_sided: false, spread: std::f32::consts::PI, falloff: 0.0, fade: 0.0 };
    let lamp_scene = {
        let square = Mesh::new(vec![at - half_u - half_v, at + half_u - half_v, at + half_u + half_v, at - half_u + half_v], vec![0, 1, 2, 0, 2, 3])
            .with_normals(vec![-mirrored; 4]);
        SceneSnapshot::new(
            vec![Arc::new(floor()), Arc::new(square)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)), Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).as_light_proxy()],
            vec![mirror(), glow(10.0)],
            Arc::new(Environment::uniform(Spectrum::ZERO)),
        )
        .with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(lamp))])))
    };
    let disc_scene = {
        // A fan of 256 triangles, its corners on the lamp's rim.
        let segments = 256u32;
        let mut positions = vec![at];
        for i in 0..segments {
            let angle = i as f32 * std::f32::consts::TAU / segments as f32;
            positions.push(at + half_u * angle.cos() + half_v * angle.sin());
        }
        let indices = (0..segments).flat_map(|i| [0, 1 + i, 1 + (i + 1) % segments]).collect();
        let disc = Mesh::new(positions, indices).with_normals(vec![-mirrored; segments as usize + 1]);
        SceneSnapshot::new(
            vec![Arc::new(floor()), Arc::new(disc)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)), Instance::new(InstanceId(1), MeshId(1), MaterialId(1))],
            vec![mirror(), glow(10.0)],
            Arc::new(Environment::uniform(Spectrum::ZERO)),
        )
    };
    let (lamp, disc) = (total(&render(lamp_scene, 16)), total(&render(disc_scene, 16)));
    println!("the disc lamp in the mirror {lamp:.2} in all, a glowing disc {disc:.2}");
    assert!(disc > 100.0, "the mirror shows little of the glowing disc: {disc:.2}");
    assert!((lamp / disc - 1.0).abs() < 0.03, "the mirror shows the disc lamp at {lamp:.2} in all, a glowing disc at {disc:.2}");
}
