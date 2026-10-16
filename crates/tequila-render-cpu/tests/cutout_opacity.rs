//! Cutout opacity: a surface light passes through rather than scatters off.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.2's `geometry_opacity`. Not refraction — a
//! ray either meets the surface or carries straight on undeviated — which is
//! what makes it a hole rather than a pane of glass.
//!
//! The half that is easy to leave out is the shadow ray. A cutout that the
//! camera sees through but that still casts a solid shadow looks *worse* than
//! no feature at all: the object is visibly absent and its shadow visibly
//! present. So both directions are checked here, and the shadow one is checked
//! against a ground plane where the difference shows.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A sphere over a ground plane, lit by the studio dome.
///
/// `opacity` is the sphere's; `None` leaves the sphere out of the scene
/// entirely, which is the answer a fully transparent one has to match.
fn render(opacity: Option<f32>) -> Framebuffer {
    let studio = tequila_scene::demo::studio_environment();

    let meshes = vec![
        Arc::new(tequila_scene::demo::uv_sphere(1.0, 48, 24)),
        Arc::new(tequila_scene::demo::ground_plane(20.0)),
    ];
    let ground = Material {
        base_color: tequila_core::spectrum::Spectrum::splat(0.6),
        specular_roughness: 0.4,
        ..MaterialType::Plastic.defaults()
    };

    let mut instances = vec![
        // The ground sits below the sphere, so the sphere's shadow lands on it.
        Instance::new(InstanceId(0), MeshId(1), MaterialId(1))
            .with_transform(Transform::translation(vec3(0.0, -1.2, 0.0))),
    ];
    let mut materials = vec![
        Material {
            base_color: tequila_core::spectrum::Spectrum::splat(0.8),
            ..MaterialType::Plastic.defaults()
        },
        ground,
    ];
    if let Some(opacity) = opacity {
        materials[0].geometry_opacity = opacity;
        instances.push(
            Instance::new(InstanceId(1), MeshId(0), MaterialId(0))
                .with_transform(Transform::IDENTITY),
        );
    }

    let scene = SceneSnapshot::new(
        meshes,
        instances,
        materials,
        Arc::new(Environment::new(studio.image().clone(), 1.0)),
    );

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(80, 80);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 1.6, 5.0),
            vec3(0.0, -0.4, 0.0),
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        160,
    );
    framebuffer
}

/// Mean luminance over a box of the frame.
fn mean(image: &Framebuffer, x0: usize, y0: usize, x1: usize, y1: usize) -> f32 {
    let mut total = 0.0;
    let mut count = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            total += image.pixel(x, y).mean().luminance();
            count += 1;
        }
    }
    total / count as f32
}

/// A fully transparent surface is not there at all.
///
/// The strongest statement available: the render has to match the scene with
/// the object removed, not merely be lighter than the opaque one. That catches
/// a cutout that passes the camera ray but still blocks light, or still casts
/// a shadow, or still shows a rim — none of which "lighter than opaque" would.
#[test]
fn a_fully_transparent_surface_matches_one_that_is_not_there() {
    let absent = render(None);
    let clear = render(Some(0.0));
    let solid = render(Some(1.0));

    // Over the whole frame, so the shadow on the ground counts as much as the
    // sphere's own silhouette.
    let (a, c, s) = (
        mean(&absent, 0, 0, 80, 80),
        mean(&clear, 0, 0, 80, 80),
        mean(&solid, 0, 0, 80, 80),
    );
    let gap = (a - s).abs();
    assert!(
        gap > 0.02,
        "the opaque and absent renders are too alike ({a:.4} vs {s:.4}) for \
         this test to be able to tell anything apart"
    );
    assert!(
        (c - a).abs() < gap * 0.05,
        "a fully transparent sphere rendered {c:.4}, against {a:.4} with no \
         sphere at all and {s:.4} with a solid one — it is still doing \
         something"
    );
}

/// Half-open lets half the light past, in shadow as well as in view.
///
/// Checked on the ground rather than on the sphere, because the shadow is the
/// half a camera-ray-only implementation gets wrong, and it is the half that
/// makes the render look broken.
#[test]
fn a_half_open_surface_casts_half_a_shadow() {
    let absent = render(None);
    let half = render(Some(0.5));
    let solid = render(Some(1.0));

    // A box on the ground under the sphere, inside the shadow and clear of the
    // silhouette.
    let (x0, y0, x1, y1) = (30, 62, 50, 74);
    let lit = mean(&absent, x0, y0, x1, y1);
    let dark = mean(&solid, x0, y0, x1, y1);
    let middle = mean(&half, x0, y0, x1, y1);

    assert!(
        lit - dark > 0.01,
        "the sampled box is not in shadow ({lit:.4} unshadowed against \
         {dark:.4} shadowed), so it cannot show a partial one"
    );
    assert!(
        middle > dark && middle < lit,
        "a half-open sphere put {middle:.4} on the ground, outside the \
         {dark:.4}..{lit:.4} its shadow should fall between — a solid shadow \
         from a surface the camera sees through is the failure this checks"
    );
    // And roughly halfway, which distinguishes a real transmittance from a
    // shadow that merely leaks a little.
    let expected = 0.5 * (lit + dark);
    assert!(
        (middle - expected).abs() < 0.3 * (lit - dark),
        "a half-open shadow measured {middle:.4}, nowhere near the {expected:.4} \
         halfway between {dark:.4} and {lit:.4}"
    );
}

/// The flag that keeps every other scene on the fast path.
///
/// If this ever reads true for an ordinary scene, every shadow ray in the
/// renderer quietly changes from an any-hit query to a walk, and the only
/// symptom is that renders get slower.
#[test]
fn a_scene_with_nothing_transparent_reports_no_cutouts() {
    let plain = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 8, 4))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![MaterialType::Plastic.defaults()],
        Arc::new(tequila_scene::demo::studio_environment()),
    );
    assert!(!plain.has_cutouts(), "an opaque scene wants the fast path");

    let cut = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 8, 4))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![MaterialType::Cutaway.defaults()],
        Arc::new(tequila_scene::demo::studio_environment()),
    );
    assert!(cut.has_cutouts(), "Cutaway is a cutout, by name and by value");

    // Opacity that is a number is answered without shading; that is what lets
    // a shadow ray walk a cutout scene cheaply.
    assert_eq!(cut.opacity_of(MaterialId(0)), Some(0.0));
    assert_eq!(plain.opacity_of(MaterialId(0)), Some(1.0));
}

/// A half-open sphere's coverage is its expected value in every sample,
/// not a coin flip: each pixel through its middle has alpha 0.75 — half
/// held at the front, half of the rest at the back — after any number of
/// samples.
///
/// The coin flip decides where a camera path goes, and has to: the path
/// needs one place to carry on from. But it also decided the pixel's alpha,
/// which made a lightbulb's glass, held only a fifth present by its opacity
/// map, cut out as a grainy speckle where Cycles cuts it out smooth.
#[test]
fn a_half_open_surface_covers_its_pixels_smoothly() {
    let mut material = Material {
        base_color: tequila_core::spectrum::Spectrum::splat(0.8),
        ..MaterialType::Plastic.defaults()
    };
    material.geometry_opacity = 0.5;
    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 48, 24))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![material],
        Arc::new(tequila_scene::demo::studio_environment()),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(40, 40);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y),
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 4);
    let mut worst: f32 = 0.0;
    for y in 16..24 {
        for x in 16..24 {
            worst = worst.max((framebuffer.pixel(x, y).alpha() - 0.75).abs());
        }
    }
    assert!(
        worst < 0.02,
        "the middle of a half-open sphere has alpha 0.75 after any number of \
         samples; after 4 a pixel was {worst:.3} away from it"
    );
}

/// The same where a shadow-catching floor lies behind the surface: the
/// surface's share is settled, and only the floor's shadow, where a path
/// gets through to it, is left to chance. The floor behind a lightbulb's
/// glass made the glass grainy there again before this.
#[test]
fn a_half_open_surface_over_a_catcher_covers_its_pixels_smoothly() {
    let mut material = Material {
        base_color: tequila_core::spectrum::Spectrum::splat(0.8),
        ..MaterialType::Plastic.defaults()
    };
    material.geometry_opacity = 0.5;
    let floor = tequila_render_core::scene::ShadowCatcher::default();
    let scene = SceneSnapshot::new(
        vec![
            Arc::new(tequila_scene::demo::uv_sphere(1.0, 48, 24)),
            Arc::new(tequila_scene::demo::ground_plane(20.0)),
        ],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1))
                .with_transform(Transform::translation(vec3(0.0, -1.0, 0.0)))
                .with_shadow_catcher(floor),
        ],
        vec![material, MaterialType::Plastic.defaults()],
        Arc::new(tequila_scene::demo::studio_environment()),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(40, 40);
    let context = RenderContext {
        // Looking down a little, so the floor is behind the lower half of
        // the sphere.
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 1.0, 5.0), vec3(0.0, -0.2, 0.0), Vec3::Y),
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 8);
    // Neighbouring pixels in the lower half of the sphere, the floor behind.
    let mut rough = 0.0;
    let mut count = 0;
    for y in 22..27 {
        for x in 15..25 {
            rough += (framebuffer.pixel(x, y).alpha() - framebuffer.pixel(x + 1, y).alpha()).abs();
            count += 1;
        }
    }
    let rough = rough / count as f32;
    assert!(
        rough < 0.06,
        "the alpha of a half-open sphere over a catcher differs by {rough:.3} \
         between neighbouring pixels after 8 samples; settled, it is smooth"
    );
}
