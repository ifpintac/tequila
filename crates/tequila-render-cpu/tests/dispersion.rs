//! Dispersion: each colour refracts by its own index (`docs/01` §7.4).
//!
//! Two claims. Dispersion splits light and must not tint it, so a dispersive
//! ball in a uniformly white world is as white and as bright as any other
//! clear ball. And it splits light, so a small white light seen through a
//! strongly dispersive ball breaks into colour — the fire a diamond has and
//! the renderer, refracting one index for all three channels, could not
//! show.

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

fn flint(abbe: f32) -> Material {
    Material {
        specular_ior: 1.72,
        transmission_dispersion_abbe: abbe,
        transmission_color: Spectrum::ONE,
        ..MaterialType::GlassSolid.defaults()
    }
}

fn render(material: Material, environment: Environment, samples: u32) -> Framebuffer {
    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 96, 48))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(environment),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.4), Vec3::ZERO, Vec3::Y),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
    framebuffer
}

#[test]
fn dispersion_does_not_tint_a_white_world() {
    let image = render(flint(20.0), Environment::uniform(Spectrum::ONE), 128);
    let mut total = Spectrum::ZERO;
    for y in 16..32 {
        for x in 16..32 {
            total = total + image.pixel(x, y).mean();
        }
    }
    let mean = (total * (1.0 / 256.0)).to_linear_srgb();
    for (name, c) in [("red", mean.x), ("green", mean.y), ("blue", mean.z)] {
        assert!((c - 1.0).abs() < 0.04, "{name} reads {c:.3} through a dispersive ball in a white world");
    }
}

/// Dark, but for one small bright light behind and to the side of the ball.
fn spot() -> Environment {
    let (width, height) = (256, 128);
    let towards = vec3(0.35, 0.15, -1.0).normalize();
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let d = tequila_render_core::pins::equirect_direction((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32);
            let lit = if d.dot(towards) > 0.995 { 40.0 } else { 0.0 };
            data.extend_from_slice(&[lit, lit, lit]);
        }
    }
    Environment::new(Arc::new(Image { width, height, channels: 3, data, color_space: ColorSpace::LinearSrgb }), 1.0)
}

/// How coloured the brightest parts of an image are: the most saturated
/// pixel among those bright enough to be the light seen through the ball.
fn fire(image: &Framebuffer) -> f32 {
    let mut most = 0.0f32;
    for y in 0..image.height {
        for x in 0..image.width {
            let c = image.pixel(x, y).mean().to_linear_srgb();
            let (high, low) = (c.x.max(c.y).max(c.z), c.x.min(c.y).min(c.z));
            if high > 0.5 {
                most = most.max((high - low) / high);
            }
        }
    }
    most
}

#[test]
fn a_dispersive_ball_breaks_white_light_into_colour() {
    let plain = fire(&render(flint(0.0), spot(), 256));
    let dispersive = fire(&render(flint(20.0), spot(), 256));
    assert!(plain < 0.15, "without dispersion the light should stay white; it is {plain:.3} saturated");
    assert!(dispersive > 0.5, "a strongly dispersive ball shows a light at most {dispersive:.3} saturated");
}
