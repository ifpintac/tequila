//! Subsurface scattering as a random walk (`docs/01` §6.4).
//!
//! Three claims, each against something known without the renderer: a white
//! part in a white world returns exactly what it receives, whatever happens
//! inside it; a part far thicker than the light's spread returns the colour
//! it was asked for, which is what the parameter conversion is for; and a
//! thin part lit from behind glows in front, which is what subsurface is for
//! and what the diffuse stand-in it replaces could never do.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::image::{ColorSpace, Image};
use std::sync::Arc;

fn render(mesh: Mesh, material: Material, environment: Environment, eye: Vec3, samples: u32) -> Framebuffer {
    let scene = SceneSnapshot::new(
        vec![Arc::new(mesh)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(environment),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(32, 32);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(eye, Vec3::ZERO, Vec3::Y),
        config: IntegratorConfig { max_bounces: 64, max_transmission_bounces: 64, ..IntegratorConfig::product() },
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
    framebuffer
}

fn centre(image: &Framebuffer) -> Spectrum {
    let mut total = Spectrum::ZERO;
    for y in 13..19 {
        for x in 13..19 {
            total = total + image.pixel(x, y).mean();
        }
    }
    total * (1.0 / 36.0)
}

/// Nothing but the walk: no reflection in front of it.
fn walker(colour: Spectrum, radius: f32) -> Material {
    Material {
        base_weight: 1.0,
        specular_weight: 0.0,
        subsurface_weight: 1.0,
        subsurface_color: colour,
        subsurface_radius: Spectrum::splat(radius),
        ..MaterialType::Diffuse.defaults()
    }
}

/// A white part in a uniformly white world is invisible: every path that
/// goes in comes out, however long it wanders. Loss here is energy the walk
/// drops; gain is energy it makes.
#[test]
fn a_white_walk_returns_what_it_receives() {
    let ball = tequila_scene::demo::uv_sphere(0.02, 64, 32);
    let image = render(ball, walker(Spectrum::ONE, 0.008), Environment::uniform(Spectrum::ONE), vec3(0.0, 0.0, 0.1), 64);
    let seen = centre(&image).luminance();
    assert!((seen - 1.0).abs() < 0.03, "a white subsurface ball in a white furnace reads {seen:.4}");
}

/// A part far thicker than the light's spread returns the colour it was set
/// to: the conversion from colour and radius to scattering coefficients
/// exists so that it does. Compared with a matte surface of that colour,
/// which by definition returns it.
#[test]
fn a_thick_part_returns_the_colour_it_was_set_to() {
    let colour = Spectrum { r: 0.8, g: 0.5, b: 0.2 };
    let ball = || tequila_scene::demo::uv_sphere(0.05, 64, 32);
    let walked = centre(&render(ball(), walker(colour, 0.001), Environment::uniform(Spectrum::ONE), vec3(0.0, 0.0, 0.25), 64));
    let matte = Material { base_color: colour, specular_weight: 0.0, ..MaterialType::Diffuse.defaults() };
    let flat = centre(&render(ball(), matte, Environment::uniform(Spectrum::ONE), vec3(0.0, 0.0, 0.25), 64));
    for (name, a, b) in [("red", walked.r, flat.r), ("green", walked.g, flat.g), ("blue", walked.b, flat.b)] {
        assert!((a - b).abs() < 0.12 * b.max(0.1), "{name}: the walk returned {a:.3} where the colour is {b:.3}");
    }
}

/// A world lit only from behind the part, dark everywhere else.
fn backlight() -> Environment {
    let (width, height) = (64, 32);
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            let d = tequila_render_core::pins::equirect_direction(u, v);
            let lit = if d.z < -0.5 { 8.0 } else { 0.0 };
            data.extend_from_slice(&[lit, lit, lit]);
        }
    }
    Environment::new(Arc::new(Image { width, height, channels: 3, data, color_space: ColorSpace::LinearSrgb }), 1.0)
}

/// A flat plate, 4 cm square and `thickness` thick, facing +Z.
fn plate(thickness: f32) -> Mesh {
    let (x, y, z) = (0.02, 0.02, thickness * 0.5);
    let corners = [
        vec3(-x, -y, -z), vec3(x, -y, -z), vec3(x, y, -z), vec3(-x, y, -z),
        vec3(-x, -y, z), vec3(x, -y, z), vec3(x, y, z), vec3(-x, y, z),
    ];
    let faces: [[usize; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [0, 4, 7, 3], [1, 2, 6, 5]];
    let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for face in faces {
        let base = positions.len() as u32;
        let (a, b, c) = (corners[face[0]], corners[face[1]], corners[face[2]]);
        let n = (b - a).cross(c - a).normalize();
        for i in face {
            positions.push(corners[i]);
            normals.push(n);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(positions, indices).with_normals(normals)
}

/// Light behind a thin part comes through it: a lampshade, a silicone key,
/// a leaf. The diffuse stand-in reflected light and let none through, so a
/// backlit subsurface part was as dark as a painted one.
#[test]
fn a_thin_part_glows_when_lit_from_behind() {
    let colour = Spectrum::splat(0.8);
    let eye = vec3(0.0, 0.0, 0.12);
    let walked = centre(&render(plate(0.002), walker(colour, 0.004), backlight(), eye, 64)).luminance();
    let painted = Material { base_color: colour, specular_weight: 0.0, ..MaterialType::Diffuse.defaults() };
    let matte = centre(&render(plate(0.002), painted, backlight(), eye, 64)).luminance();
    assert!(matte < 0.01, "the control is lit from the front: {matte}");
    assert!(walked > 0.3, "a thin subsurface plate lit from behind shows {walked:.3} in front");
}

/// A clear part whose volume scatters — milk, a smoky glass: `transmission_scatter`.
fn milky(scatter: f32) -> Material {
    Material {
        transmission_color: Spectrum::ONE,
        transmission_depth: 0.01,
        transmission_scatter: Spectrum::splat(scatter),
        ..MaterialType::ScatteringMedium.defaults()
    }
}

/// A volume that scatters and absorbs nothing, behind a glass surface, in a
/// white world, is invisible: the walk inside a clear part keeps its energy
/// as the one under a subsurface part does, and the part is left through its
/// refracting surface rather than a doorway.
#[test]
fn a_scattering_volume_that_absorbs_nothing_returns_what_it_receives() {
    let ball = tequila_scene::demo::uv_sphere(0.02, 64, 32);
    let seen = centre(&render(ball, milky(2.0), Environment::uniform(Spectrum::ONE), vec3(0.0, 0.0, 0.1), 64)).luminance();
    assert!((seen - 1.0).abs() < 0.03, "a non-absorbing milky ball in a white furnace reads {seen:.4}");
}

/// Scattering is what makes milk milk: lit from behind, a clear ball is a
/// lens that shows the light straight through, and a milky one spreads it.
#[test]
fn a_scattering_volume_is_not_clear() {
    let ball = || tequila_scene::demo::uv_sphere(0.02, 64, 32);
    let eye = vec3(0.0, 0.0, 0.1);
    let clear = centre(&render(ball(), milky(0.0), backlight(), eye, 64)).luminance();
    let cloudy = centre(&render(ball(), milky(3.0), backlight(), eye, 64)).luminance();
    assert!(clear > 1.0, "the clear control shows no backlight through it: {clear}");
    assert!(cloudy < clear * 0.6, "a milky ball passes {cloudy:.3} of the backlight where a clear one passes {clear:.3}");
}

/// Milk at real density, in a glass-sized ball, keeps its light. Its mean
/// free path is a third of a millimetre, so a path that wanders a few
/// centimetres in takes thousands of events to come back, and the walk used
/// to stop at 256 and count the rest as absorbed: a non-absorbing milky ball
/// in a white world read 0.83 to 0.86. Measured against the cap: 0.93 at
/// 1024, 0.97 at 4096, 0.98 to 1.00 at 16384 — for three times the time,
/// spent only on the paths that wander deep.
#[test]
fn dense_milk_keeps_its_light() {
    let material = Material {
        transmission_color: Spectrum::ONE,
        transmission_depth: 0.01,
        transmission_scatter: Spectrum { r: 25.5, g: 32.1, b: 37.7 },
        specular_ior: 1.35,
        ..MaterialType::Liquid.defaults()
    };
    let ball = tequila_scene::demo::uv_sphere(0.05, 64, 32);
    let seen = centre(&render(ball, material, Environment::uniform(Spectrum::ONE), vec3(0.0, 0.0, 0.25), 64));
    for (name, c) in [("red", seen.r), ("green", seen.g), ("blue", seen.b)] {
        assert!(c > 0.955, "a non-absorbing milk ball in a white world reads {c:.3} in {name}");
    }
}
