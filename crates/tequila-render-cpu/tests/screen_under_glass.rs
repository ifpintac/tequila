//! A glowing screen under a glass cover - a phone, a dashboard, a device's
//! display - shows through it as it would through a window: the glass takes
//! its reflections, a few percent at each face, and no more.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 48;

/// A square facing up at `height`, `half` across each way.
fn square(height: f32, half: f32, up: bool) -> Mesh {
    let corners = vec![vec3(-half, height, -half), vec3(half, height, -half), vec3(half, height, half), vec3(-half, height, half)];
    let indices = if up { vec![0, 2, 1, 0, 3, 2] } else { vec![0, 1, 2, 0, 2, 3] };
    let normal = if up { Vec3::Y } else { -Vec3::Y };
    Mesh::new(corners, indices).with_normals(vec![normal; 4])
}

/// A closed slab of glass from `low` to `high`, wider than the screen.
fn slab(low: f32, high: f32, half: f32) -> Mesh {
    let p = |x: f32, y: f32, z: f32| vec3(x * half, y, z * half);
    let corners = vec![
        p(-1.0, low, -1.0), p(1.0, low, -1.0), p(1.0, low, 1.0), p(-1.0, low, 1.0),
        p(-1.0, high, -1.0), p(1.0, high, -1.0), p(1.0, high, 1.0), p(-1.0, high, 1.0),
    ];
    // Outward-facing triangles of the box.
    let indices = vec![
        0, 1, 2, 0, 2, 3, // bottom, facing down
        4, 6, 5, 4, 7, 6, // top, facing up
        0, 4, 5, 0, 5, 1, // back
        1, 5, 6, 1, 6, 2, // right
        2, 6, 7, 2, 7, 3, // front
        3, 7, 4, 3, 4, 0, // left
    ];
    Mesh::new(corners, indices)
}

fn render(cover: Option<tequila_materials::openpbr::Material>) -> f32 {
    let screen = MaterialType::Emissive.defaults();
    let screen = tequila_materials::openpbr::Material { emission_color: Spectrum::ONE, emission_luminance: 2.0, ..screen };
    let mut meshes = vec![Arc::new(square(0.0, 0.5, true))];
    let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)];
    let mut materials = vec![screen];
    if let Some(glass) = cover {
        meshes.push(Arc::new(slab(0.02, 0.06, 0.8)));
        instances.push(Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(Transform::IDENTITY));
        materials.push(glass);
    }
    let scene = SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ZERO)));
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 1.6, 0.01), Vec3::ZERO, Vec3::Y),
        config: IntegratorConfig::product(),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    for _ in 0..32 {
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
    }
    centre(&framebuffer)
}

/// The mean luminance of the middle of the picture, on the screen.
fn centre(framebuffer: &Framebuffer) -> f32 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in SIZE / 3..SIZE * 2 / 3 {
        for x in SIZE / 3..SIZE * 2 / 3 {
            sum += framebuffer.pixel(x, y).mean().luminance();
            count += 1.0;
        }
    }
    sum / count
}

#[test]
fn a_screen_shows_through_clear_glass_over_it() {
    let bare = render(None);
    assert!(bare > 1.5, "the screen alone glows: {bare}");
    for (name, glass) in [("solid glass", MaterialType::GlassSolid.defaults()), ("a thin pane", MaterialType::Glass.defaults())] {
        let covered = render(Some(glass));
        let through = covered / bare;
        println!("{name}: {covered:.3} against {bare:.3} bare, {through:.2} through");
        // Two faces at about four percent each, and some light kept between
        // them: near nine tenths.
        assert!(through > 0.8, "{name} let through {through:.2} of the screen");
    }
}
