//! Cutaways through the real intersector (`docs/02` §3, "Cutaway").
//!
//! The unit tests in `render-core` check the shapes against themselves. These
//! check what a person sees: the part of a solid inside a cutter is gone, the
//! cut face is a surface at the cut rather than a view of the far wall, it is
//! lit from the side the camera is on, and the removed part no longer casts a
//! shadow.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Ray, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::cutaway::{Cutter, CutterShape, CAP_PRIMITIVE};
use tequila_render_core::environment::Environment;
use tequila_render_core::intersect::Intersector;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_cpu::{Device, EmbreeIntersector};
use std::sync::Arc;

/// A unit sphere at the origin, and the ground — a shadow catcher, as the
/// app makes it — under it.
fn scene(cutters: Vec<Cutter>) -> EmbreeIntersector {
    let meshes = vec![
        Arc::new(tequila_scene::demo::uv_sphere(1.0, 64, 32)),
        Arc::new(tequila_scene::demo::ground_plane(20.0)),
    ];
    let materials = vec![
        Material {
            base_color: Spectrum { r: 0.8, g: 0.1, b: 0.1 },
            ..MaterialType::Plastic.defaults()
        },
        MaterialType::Plastic.defaults(),
        // A cap colour of its own.
        Material {
            base_color: Spectrum { r: 0.9, g: 0.8, b: 0.1 },
            ..MaterialType::Plastic.defaults()
        },
    ];
    let instances = vec![
        Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
        Instance::new(InstanceId(1), MeshId(1), MaterialId(1))
            .with_transform(Transform::translation(vec3(0.0, -1.5, 0.0)))
            .with_shadow_catcher(Default::default()),
    ];
    let snapshot = SceneSnapshot::new(
        meshes,
        instances,
        materials,
        Arc::new(Environment::uniform(Spectrum::ONE)),
    )
    .with_cutters(cutters);
    EmbreeIntersector::build(&Device::shared().expect("Embree"), snapshot).expect("builds")
}

/// Everything in front of `z = 0` removed.
fn front_half() -> Cutter {
    Cutter::new(CutterShape::Plane, Transform::IDENTITY)
}

fn toward_the_sphere(x: f32, y: f32) -> Ray {
    Ray::new(vec3(x, y, 5.0), vec3(0.0, 0.0, -1.0))
}

/// Without a cutter the front of the sphere is seen; with one, the ray meets
/// a cap exactly at the cut, facing the camera, in the sphere's own material.
#[test]
fn a_plane_cut_leaves_a_cap_at_the_cut() {
    let whole = scene(Vec::new()).intersect(&toward_the_sphere(0.1, 0.2)).expect("the sphere");
    assert!((whole.t - (5.0 - (1.0f32 - 0.05).sqrt())).abs() < 0.02, "the whole sphere at {}", whole.t);

    let cut = scene(vec![front_half()])
        .intersect(&toward_the_sphere(0.1, 0.2))
        .expect("the cap");
    assert_eq!(cut.primitive, CAP_PRIMITIVE, "that was not a cap");
    assert!((cut.t - 5.0).abs() < 1e-3, "the cap is at depth {}, not at the cut", cut.t);
    assert!(cut.geometric_normal.z > 0.999, "the cap faces {:?}", cut.geometric_normal);
    assert!(!cut.is_backface(vec3(0.0, 0.0, -1.0)));
    assert_eq!(cut.material, MaterialId(0), "the cap took another material");
    assert_eq!(cut.instance, InstanceId(0));
}

/// A cap colour, when the cutter has one.
#[test]
fn a_cutter_can_give_the_cap_its_own_material() {
    let mut cutter = front_half();
    cutter.cap = Some(MaterialId(2));
    let cut = scene(vec![cutter]).intersect(&toward_the_sphere(0.0, 0.0)).expect("the cap");
    assert_eq!(cut.material, MaterialId(2));
}

/// Outside the solid, the cut leaves nothing: a ray past the sphere's edge
/// that would have grazed its removed front sees straight through to the
/// ground behind — here, nothing, as the ray is level.
#[test]
fn only_solids_are_capped() {
    let intersector = scene(vec![front_half()]);
    assert!(intersector.intersect(&toward_the_sphere(1.2, 0.0)).is_none());
    // The ground is not cut, even where it runs through the cutter.
    let down = Ray::new(vec3(0.0, 0.0, 5.0), vec3(0.0, -1.0, 0.0));
    let ground = intersector.intersect(&down).expect("the ground");
    assert!((ground.t - 1.5).abs() < 1e-3, "the ground was cut: {}", ground.t);
}

/// A sphere cutter takes a bite out of the side, capped with a concave face
/// that still faces the camera.
#[test]
fn a_sphere_cutter_takes_a_bite() {
    let bite = Cutter::new(
        CutterShape::Sphere,
        Transform::from_matrix(
            tequila_core::math::Mat4::translation(vec3(0.0, 0.0, 1.0))
                * tequila_core::math::Mat4::scale(vec3(0.6, 0.6, 0.6)),
        )
        .expect("invertible"),
    );
    let intersector = scene(vec![bite]);
    let centre = intersector.intersect(&toward_the_sphere(0.0, 0.0)).expect("the bite's floor");
    assert_eq!(centre.primitive, CAP_PRIMITIVE);
    // The bite's sphere reaches z = 0.4 on the axis.
    assert!((centre.t - 4.6).abs() < 1e-3, "the bite's floor is at {}", centre.t);
    assert!(centre.geometric_normal.z > 0.99);

    // Away from the bite, the sphere is whole.
    let side = intersector.intersect(&toward_the_sphere(0.0, 0.8)).expect("the sphere");
    assert_ne!(side.primitive, CAP_PRIMITIVE);
}

/// The removed part casts no shadow; what remains, including the cap, does.
#[test]
fn what_is_cut_away_casts_no_shadow() {
    // Light from straight above a point on the ground in front of the cut.
    let cut = scene(vec![front_half()]);
    let whole = scene(Vec::new());
    let under_the_front = vec3(0.0, -1.5, 0.5);
    let above = vec3(0.0, 10.0, 0.5);
    assert!(whole.occluded(under_the_front, vec3(0.0, 1.0, 0.0), above, vec3(0.0, -1.0, 0.0)));
    assert!(
        !cut.occluded(under_the_front, vec3(0.0, 1.0, 0.0), above, vec3(0.0, -1.0, 0.0)),
        "the cut-away front half still shadows the ground"
    );
    let under_the_back = vec3(0.0, -1.5, -0.5);
    let above_back = vec3(0.0, 10.0, -0.5);
    assert!(
        cut.occluded(under_the_back, vec3(0.0, 1.0, 0.0), above_back, vec3(0.0, -1.0, 0.0)),
        "the half that remains stopped casting its shadow"
    );
}

/// Rendered, the cap is a lit surface in the part's colour — not black, which
/// is what a cap facing the wrong way or shaded without a frame would be, and
/// not the background, which is what a cut without a cap would show.
#[test]
fn the_cap_renders_lit_in_the_parts_colour() {
    use tequila_render_core::framebuffer::Framebuffer;
    use tequila_render_core::scheduler::ProgressiveSchedule;
    use tequila_render_cpu::{render_pass, RenderContext};

    let render = |cutters: Vec<Cutter>| {
        let intersector = scene(cutters);
        let mut framebuffer = Framebuffer::new(48, 48);
        let context = RenderContext {
            camera: tequila_render_core::camera::Camera::look_at(
                vec3(0.0, 0.0, 6.0),
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            ),
            ..RenderContext::default()
        };
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 64);
        framebuffer
    };
    let centre = |image: &Framebuffer| {
        let mut sum = Spectrum::ZERO;
        for y in 20..28 {
            for x in 20..28 {
                sum = sum + image.pixel(x, y).mean();
            }
        }
        sum * (1.0 / 64.0)
    };

    let whole = centre(&render(Vec::new()));
    let cut = centre(&render(vec![front_half()]));
    // Lit, and red like the sphere's own face.
    assert!(cut.r > 0.1, "the cap is dark: {cut:?}");
    assert!(cut.r > 2.0 * cut.g && cut.r > 2.0 * cut.b, "the cap is not the part's red: {cut:?}");
    // Facing the camera, as the whole sphere's front does: similarly bright.
    assert!(
        cut.r > 0.5 * whole.r && cut.r < 2.0 * whole.r,
        "the cap ({cut:?}) and the uncut front ({whole:?}) are lit very differently"
    );
}
