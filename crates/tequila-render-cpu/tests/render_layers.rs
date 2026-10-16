//! Parts hidden from the camera, which is what a render layer is made of
//! (`TEQUILA_SPEC.md` §1.10, "Render Layers").
//!
//! A layer's image holds only its own parts. Everything else is still in the
//! scene — casting shadows, bouncing light, showing in reflections — but the
//! camera looks straight through it, so the layers composite back together
//! without a hole where one part stood in front of another.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::intersect::Intersector;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A red sphere in front of a larger blue one, on the camera's axis.
fn scene(front_seen: bool) -> EmbreeIntersector {
    let meshes = vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 48, 24))];
    let materials = vec![
        Material {
            base_color: Spectrum { r: 0.8, g: 0.05, b: 0.05 },
            ..MaterialType::Plastic.defaults()
        },
        Material {
            base_color: Spectrum { r: 0.05, g: 0.1, b: 0.8 },
            ..MaterialType::Plastic.defaults()
        },
    ];
    let mut front = Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
        .with_transform(Transform::translation(vec3(0.0, 0.0, 1.5)));
    front.visible_to_camera = front_seen;
    let back = Instance::new(InstanceId(1), MeshId(0), MaterialId(1)).with_transform(
        Transform::from_matrix(
            tequila_core::math::Mat4::translation(vec3(0.0, 0.0, -2.0))
                * tequila_core::math::Mat4::scale(Vec3::splat(2.0)),
        )
        .unwrap(),
    );
    let snapshot = SceneSnapshot::new(
        meshes,
        vec![front, back],
        materials,
        Arc::new(Environment::uniform(Spectrum::ONE)),
    );
    EmbreeIntersector::build(&Device::shared().expect("Embree"), snapshot).expect("builds")
}

fn centre(intersector: &EmbreeIntersector) -> (Spectrum, f32) {
    let mut framebuffer = Framebuffer::new(32, 32);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y),
        ..RenderContext::default()
    };
    render_pass(intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 32);
    let pixel = framebuffer.pixel(16, 16);
    (pixel.mean(), pixel.alpha())
}

#[test]
fn the_camera_sees_through_a_hidden_part_and_nothing_else_does() {
    let (seen, _) = centre(&scene(true));
    assert!(seen.r > seen.b, "with both seen, the front sphere should be in front: {seen:?}");

    let hidden = scene(false);
    let (through, alpha) = centre(&hidden);
    assert!(through.b > 2.0 * through.r, "the camera did not see through the hidden sphere: {through:?}");
    assert!(alpha > 0.99, "the part behind it is solid: alpha {alpha}");

    // Everything but the camera still meets it: it still stands between the
    // back sphere and a light in front.
    assert!(
        hidden.occluded(vec3(0.0, 0.0, 1.0 - 2.0), Vec3::Z, vec3(0.0, 0.0, 6.0), -Vec3::Z),
        "the hidden sphere stopped blocking light"
    );
    let bounced = hidden.intersect(&tequila_core::math::Ray::new(vec3(0.0, 0.0, 0.0), Vec3::Z));
    assert!(bounced.is_some_and(|h| h.instance == InstanceId(0)), "a bounce ray passed through the hidden sphere");
}
