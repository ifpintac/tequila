//! A coarse curved mesh, smoothed by its vertex normals, seen close to its
//! silhouette.
//!
//! At a grazing angle the reflection about an interpolated normal can point
//! into the triangle it was taken on. The path that drew it then ended, black,
//! and a dark line ran along every facet at the rim: the model's triangles
//! showing through smooth shading, on curved glass above all, in close-ups
//! and in a mirror floor. Cycles turns such a normal towards the geometric one
//! just far enough that the reflection stays above the surface, and so does
//! Tequila now.
//!
//! Measured in a furnace: clear glass under an even white sky gives back all
//! the light it is given, so every pixel, sphere or sky, is white, and a
//! pixel that is not is light the integrator lost.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 96;

fn clear_glass() -> Material {
    Material {
        transmission_weight: 1.0,
        transmission_color: Spectrum::ONE,
        transmission_depth: 0.0,
        geometry_thin_walled: false,
        specular_roughness: 0.0,
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    }
}

fn render(mesh: Mesh) -> Framebuffer {
    let scene = SceneSnapshot::new(
        vec![Arc::new(mesh)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![clear_glass()],
        Arc::new(Environment::uniform(Spectrum::ONE)),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.2), vec3(0.0, 0.0, -1.0), Vec3::Y),
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 64);
    framebuffer
}

/// How many pixels came out dark, and the darkest of them.
fn dark(image: &Framebuffer) -> (usize, f32) {
    let mut count = 0;
    let mut darkest = f32::INFINITY;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let value = image.pixel(x, y).mean().luminance();
            darkest = darkest.min(value);
            if value < 0.6 {
                count += 1;
            }
        }
    }
    (count, darkest)
}

/// A glass ball of 24 by 12 segments, smooth-shaded, in a furnace: no dark
/// outline round its silhouette, and nothing on it much darker than on a
/// finely tessellated ball. Before the normal was turned, five pixels on the
/// rim of this one fell under 0.6, the darkest to 0.53 - 61% of the fine
/// ball's darkest; turned, none do, and the darkest is 0.76, 88% of it.
#[test]
fn a_coarse_glass_ball_has_no_dark_outline() {
    let (coarse_dark, coarse_darkest) = dark(&render(tequila_scene::demo::uv_sphere(1.0, 24, 12)));
    let (fine_dark, fine_darkest) = dark(&render(tequila_scene::demo::uv_sphere(1.0, 256, 128)));
    assert_eq!(fine_dark, 0, "the fine ball itself lost light (darkest {fine_darkest:.3})");
    assert!(
        coarse_dark == 0 && coarse_darkest > 0.75 * fine_darkest,
        "{coarse_dark} pixels of the coarse ball came out dark, the darkest {coarse_darkest:.3} \
         against the fine ball's {fine_darkest:.3}: reflections off its smoothed facets are \
         going into the glass and ending there"
    );
}
