//! §4.4's fibres through the whole transport loop.
//!
//! The fibre BSDF's own tests hold `sample`, `eval` and `pdf` together at
//! one fibre. What they cannot see is the integrator around it: the frame
//! it is shaded in, the cosine it divides out, the side a path leaves by,
//! the rays that leave a fibre and must not meet it again. In a uniform
//! white environment a lossless object is invisible, fur and all: a white
//! ball grown over with white fibres reads the environment's radiance at
//! every pixel, whatever the fibres do to the paths, and a pixel that reads
//! less is light the transport lost.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::FuzzRequest;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A unit ball in `material`, with the fibres the material asks for, seen
/// whole from in front, in a white environment.
fn furred_ball(material: Material, config: IntegratorConfig) -> Framebuffer {
    let ball = tequila_scene::demo::uv_sphere(1.0, 96, 48);
    let mut meshes = vec![Arc::new(ball.clone())];
    let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))];
    if let Some(request) = FuzzRequest::of_material(&material) {
        let fur = tequila_scene::fuzz::grow(&ball, 1.0, &request).expect("fibres");
        meshes.push(Arc::new(fur));
        instances.push(Instance::new(InstanceId(1), MeshId(1), MaterialId(0)));
    }
    let scene = SceneSnapshot::new(meshes, instances, vec![material], Arc::new(Environment::uniform(Spectrum::ONE)));
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), scene).expect("builds");
    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: camera(),
        config,
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 256);
    framebuffer
}

/// The mean over the middle of the image, fur seen end on over the ball.
fn centre(image: &Framebuffer) -> Spectrum {
    let mut total = Spectrum::ZERO;
    for y in 18..30 {
        for x in 18..30 {
            total = total + image.pixel(x, y).mean();
        }
    }
    total * (1.0 / 144.0)
}

fn camera() -> tequila_render_core::camera::Camera {
    tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y)
}

/// The mean round the outline, fur seen side on against the sky.
fn rim(image: &Framebuffer) -> Spectrum {
    // Where the outline falls: the ball's edge seen from the camera, a
    // radius over at the distance where its tangent rays touch it.
    let edge = camera().project(vec3(0.9682, 0.0, 0.25), 1.0).expect("in view");
    let outline = (edge.x - 0.5).abs() * 48.0;
    let (mut total, mut n) = (Spectrum::ZERO, 0.0);
    for y in 0..48 {
        for x in 0..48 {
            let r = ((x as f32 + 0.5 - 24.0).powi(2) + (y as f32 + 0.5 - 24.0).powi(2)).sqrt();
            if (outline - 1.5..outline + 2.0).contains(&r) {
                total = total + image.pixel(x, y).mean();
                n += 1.0;
            }
        }
    }
    total * (1.0 / n)
}

fn white_fur(density: f32) -> Material {
    Material {
        base_color: Spectrum::ONE,
        specular_weight: 0.0,
        fuzz_color: Spectrum::ONE,
        fuzz_roughness: 0.4,
        fuzz_density: density,
        fuzz_length: 0.12,
        fuzz_thickness: 0.004,
        fuzz_lean: 0.4,
        fuzz_curl: 0.4,
        fuzz_color_variation: 0.0,
        ..Material::default()
    }
}

/// A white ball under white fur, as thick as a felt's - several fibres
/// deep wherever the camera looks - is as white as the sky round it, over
/// its face and at its outline, given the bounces to get out.
#[test]
fn white_fur_on_a_white_ball_loses_no_light() {
    let deep = IntegratorConfig { max_bounces: 128, max_transmission_bounces: 128, ..IntegratorConfig::product() };
    let bare = furred_ball(white_fur(0.0), deep.clone());
    let furred = furred_ball(white_fur(0.4), deep);
    let (face, outline) = (centre(&furred).luminance(), rim(&furred).luminance());
    println!("bare {:.4} {:.4}, furred face {face:.4}, outline {outline:.4}", centre(&bare).luminance(), rim(&bare).luminance());
    assert!((centre(&bare).luminance() - 1.0).abs() < 0.02, "the bare control is off");
    assert!((face - 1.0).abs() < 0.03, "a white furred ball reads {face:.4} over its face: light is lost in the fur");
    assert!((outline - 1.0).abs() < 0.03, "a white furred ball reads {outline:.4} at its outline");
}
