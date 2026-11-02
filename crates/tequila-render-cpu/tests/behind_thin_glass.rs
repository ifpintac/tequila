//! What lies just behind a thin cover - the mask round a handheld's screen,
//! a logo printed on the back of its lens - shows through it. On a model in
//! metres they sit microns behind the cover's back face: the palma P8's
//! logo 2 µm, its mask 4 µm. A ray carried on past the back face from
//! further off than that starts beyond them, and the lens showed what was
//! inside the mask, near black, where Cycles showed it red.

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

const SIZE: usize = 32;

/// A square facing the camera (+Z) at `z`, `half` across each way about
/// `centre`.
fn square(centre: Vec3, z: f32, half: f32) -> Mesh {
    let p = |x: f32, y: f32| vec3(centre.x + x * half, centre.y + y * half, z);
    Mesh::new(vec![p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0)], vec![0, 1, 2, 0, 2, 3]).with_normals(vec![Vec3::Z; 4])
}

/// A closed slab from `back` to `front` along Z, as the lens is.
fn slab(centre: Vec3, back: f32, front: f32, half: f32) -> Mesh {
    let p = |x: f32, y: f32, z: f32| vec3(centre.x + x * half, centre.y + y * half, z);
    let corners = vec![
        p(-1.0, -1.0, back), p(1.0, -1.0, back), p(1.0, 1.0, back), p(-1.0, 1.0, back),
        p(-1.0, -1.0, front), p(1.0, -1.0, front), p(1.0, 1.0, front), p(-1.0, 1.0, front),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // back, facing -Z
        4, 5, 6, 4, 6, 7, // front, facing +Z
        0, 1, 5, 0, 5, 4, // bottom
        1, 2, 6, 1, 6, 5, // right
        2, 3, 7, 2, 7, 6, // top
        3, 0, 4, 3, 4, 7, // left
    ];
    Mesh::new(corners, indices)
}

/// The palma P8's lens: a plastic all transmission, thin-walled, almost
/// mirror-smooth, faintly grey.
fn lens() -> tequila_materials::openpbr::Material {
    tequila_materials::openpbr::Material {
        transmission_weight: 1.0,
        transmission_color: Spectrum::splat(0.97),
        geometry_thin_walled: true,
        specular_roughness: 0.02,
        ..MaterialType::Plastic.defaults()
    }
}

/// The colour seen through the middle of a lens 0.3 mm thick whose front
/// face is at `z`, with red paint `gap` behind its back face; and the
/// paint's colour with no lens. The product stands on the floor as an
/// imported one does, its middle 10 cm up.
fn render(z: f32, gap: f32) -> (Spectrum, Spectrum) {
    let centre = vec3(0.0, 0.1, 0.0);
    let thickness = 0.0003;
    let red = tequila_materials::openpbr::Material { base_color: Spectrum { r: 0.8, g: 0.0, b: 0.0 }, ..MaterialType::Plastic.defaults() };
    let shot = |with_lens: bool| {
        let mut meshes = vec![Arc::new(square(centre, z - thickness - gap, 0.06))];
        let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)];
        let mut materials = vec![red.clone()];
        if with_lens {
            meshes.push(Arc::new(slab(centre, z - thickness, z, 0.07)));
            instances.push(Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(Transform::IDENTITY));
            materials.push(lens());
        }
        let scene = SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ONE)));
        let device = Device::shared().expect("an Embree device");
        let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
        let mut framebuffer = Framebuffer::new(SIZE, SIZE);
        let context = RenderContext {
            camera: tequila_render_core::camera::Camera::look_at(vec3(centre.x, centre.y, z + 0.25), vec3(centre.x, centre.y, z), Vec3::Y),
            config: IntegratorConfig::product(),
            reject_fireflies: false,
            ..RenderContext::default()
        };
        for _ in 0..16 {
            render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
        }
        middle(&framebuffer)
    };
    (shot(true), shot(false))
}

/// The mean colour of the middle of the picture, all of it on the paint.
fn middle(framebuffer: &Framebuffer) -> Spectrum {
    let mut sum = Spectrum::ZERO;
    let mut count = 0.0;
    for y in SIZE / 3..SIZE * 2 / 3 {
        for x in SIZE / 3..SIZE * 2 / 3 {
            sum += framebuffer.pixel(x, y).mean();
            count += 1.0;
        }
    }
    sum * (1.0 / count)
}

/// What the paint adds to the lens's own grey reflections: its red over
/// its green.
fn red(colour: Spectrum) -> f32 {
    colour.r - colour.g
}

#[test]
fn paint_just_behind_a_thin_lens_shows_through_it() {
    // At the world's origin, as the palma's lens is, and a hand's length
    // in front of it, where the coordinate is large.
    for z in [0.0, 0.3] {
        // A tenth of a millimetre behind, which every offset clears: how
        // the paint looks through the lens.
        let (reference, bare) = render(z, 1e-4);
        println!("lens at z {z}: paint 100 µm behind {reference:?}, bare {bare:?}");
        assert!(red(bare) > 0.5, "the paint alone is red: {bare:?}");
        // Lit through the lens and seen through it: two faces each way at
        // about four percent, and the grey of the lens.
        assert!(red(reference) > 0.6 * red(bare), "lens at z {z}: {reference:?} against {bare:?} bare");
        for gap in [2e-6, 4e-6] {
            let (through, _) = render(z, gap);
            println!("lens at z {z}: paint {:.0} µm behind {through:?}", gap * 1e6);
            assert!(
                (red(through) - red(reference)).abs() < 0.05 * red(reference),
                "lens at z {z}, paint {gap} behind its back face: {through:?}, against {reference:?} 100 µm behind"
            );
        }
    }
}
