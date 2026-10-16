//! A clean backdrop with the pin lighting intact.
//!
//! Pins are painted onto the environment, so they light the scene *and* show up
//! behind it. For a product shot on a white sweep that second part is usually
//! not wanted, and the answer already exists: §1.5 decouples the background
//! from the lighting, so a solid background colour replaces what the camera
//! sees and not one photon landing on the product.
//!
//! That is a claim about behaviour, written into `docs/06` and a commit
//! message, so it is a test. Reading `Background::radiance` and the
//! `specular_chain` flag and concluding it must be so is how this project has
//! been wrong before.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::environment::{Background, Environment};
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::pins::{composite, Pin};
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::image::{ColorSpace, Image};
use std::sync::Arc;

/// A big softbox above and slightly in front, bright enough to dominate.
fn key_light() -> Pin {
    Pin {
        name: "Key".into(),
        direction: vec3(0.0, 0.7, 0.7),
        angular_size: vec2(1.1, 1.1),
        softness: 0.5,
        intensity: 25.0,
        ..Default::default()
    }
}

/// A second softbox out beyond the product, up and to one side.
///
/// The camera sees this one: it sits clear of the sphere's silhouette, in the
/// upper-left of the frame. A light that lights the camera-facing side of a
/// product is behind the camera and cannot be seen, so a test about what is in
/// the backdrop needs a light that is somewhere else.
fn visible_light() -> Pin {
    Pin {
        name: "Visible".into(),
        direction: vec3(-0.45, 0.38, -0.81),
        angular_size: vec2(0.45, 0.45),
        softness: 0.4,
        intensity: 20.0,
        ..Default::default()
    }
}

fn environment_with_pin() -> Environment {
    // A dim even sky, so the pin is the light that matters and the difference
    // below cannot come from the sky alone.
    let mut base = Image {
        width: 256,
        height: 128,
        channels: 3,
        data: vec![0.02; 256 * 128 * 3],
        color_space: ColorSpace::Raw,
    };
    base.color_space = ColorSpace::Raw;
    Environment::new(
        Arc::new(composite(&base, &[key_light(), visible_light()])),
        1.0,
    )
}

fn render(background: Background) -> Framebuffer {
    let sphere = tequila_scene::demo::uv_sphere(0.8, 48, 24);
    let mut scene = SceneSnapshot::new(
        vec![Arc::new(sphere)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![MaterialType::Diffuse.defaults()],
        Arc::new(environment_with_pin()),
    );
    scene.background = background;

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.0, 7.0),
            Vec3::ZERO,
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        48,
    );
    framebuffer
}

/// Mean brightness over a patch, so a few noisy pixels cannot decide anything.
fn patch(image: &Framebuffer, x0: usize, y0: usize, size: usize) -> f32 {
    let mut total = 0.0;
    for y in y0..y0 + size {
        for x in x0..x0 + size {
            total += image.pixel(x, y).mean().g;
        }
    }
    total / (size * size) as f32
}

/// The brightest pixel in a box. What a mirror needs: it reflects the softbox
/// as a small bright spot, and a patch mean around that spot washes it out.
fn brightest(image: &Framebuffer, x0: usize, y0: usize, x1: usize, y1: usize) -> f32 {
    let mut best = 0.0f32;
    for y in y0..y1 {
        for x in x0..x1 {
            best = best.max(image.pixel(x, y).mean().g);
        }
    }
    best
}

/// Switching the background to a solid colour changes the backdrop and leaves
/// the light on the product alone.
#[test]
fn a_solid_background_hides_the_pin_without_unlighting_the_product() {
    let showing = render(Background::Environment);
    let clean = render(Background::Color(Spectrum::splat(0.5)));

    // The corner is empty space: the backdrop, and it must change.
    let corner_showing = patch(&showing, 1, 1, 6);
    let corner_clean = patch(&clean, 1, 1, 6);
    assert!(
        (corner_clean - 0.5).abs() < 0.02,
        "the backdrop reads {corner_clean} rather than the colour asked for"
    );
    assert!(
        (corner_showing - corner_clean).abs() > 0.05,
        "the backdrop did not change ({corner_showing} against {corner_clean}), \
         so this test is comparing two identical pictures"
    );

    // The top of the sphere faces the key light. Its brightness comes from the
    // environment on both renders and must not move.
    let lit_showing = patch(&showing, 22, 19, 4);
    let lit_clean = patch(&clean, 22, 19, 4);
    assert!(
        lit_showing > 0.15,
        "the pin is not lighting the sphere at all ({lit_showing}), so there is \
         nothing for this test to protect"
    );
    assert!(
        (lit_showing - lit_clean).abs() < 0.02,
        "hiding the background changed the light on the product from \
         {lit_showing} to {lit_clean}; §1.5 says the two are decoupled"
    );
}

/// The exception worth knowing about: a mirror shows the backdrop, not the pin.
///
/// A path that has only ever bounced off delta surfaces is still one the camera
/// sees along, so §1.5's rule sends it to the background. That is deliberate —
/// a backplate photograph *should* appear in a chrome surface, or the product
/// looks pasted on — but it means a clean colour backdrop takes the softbox
/// reflections off chrome along with it, which is not what a product
/// photographer wants and is worth saying out loud rather than discovering.
#[test]
fn a_mirror_reflects_the_backdrop_rather_than_the_pin() {
    fn mirrored(background: Background) -> Framebuffer {
        let sphere = tequila_scene::demo::uv_sphere(0.8, 48, 24);
        let mut mirror = MaterialType::Metal.defaults();
        mirror.specular_roughness = 0.0;
        mirror.base_metalness = 1.0;

        let mut scene = SceneSnapshot::new(
            vec![Arc::new(sphere)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
                .with_transform(Transform::IDENTITY)],
            vec![mirror],
            Arc::new(environment_with_pin()),
        );
        scene.background = background;

        let device = Device::shared().expect("an Embree device");
        let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
        let mut framebuffer = Framebuffer::new(48, 48);
        let context = RenderContext {
            camera: tequila_render_core::camera::Camera::look_at(
                vec3(0.0, 0.0, 7.0),
                Vec3::ZERO,
                Vec3::Y,
            ),
            ..RenderContext::default()
        };
        render_pass(
            &intersector,
            &mut framebuffer,
            &context,
            &ProgressiveSchedule::default(),
            48,
        );
        framebuffer
    }

    let showing = mirrored(Background::Environment);
    let clean = mirrored(Background::Color(Spectrum::splat(0.05)));

    // The top of the sphere mirrors the sky above it, where the softbox is.
    let reflection_showing = brightest(&showing, 16, 14, 32, 30);
    let reflection_clean = brightest(&clean, 16, 14, 32, 30);
    assert!(
        reflection_showing > 0.2,
        "the mirror is not reflecting the softbox at all ({reflection_showing})"
    );
    assert!(
        reflection_clean < reflection_showing * 0.5,
        "a dark solid background left the softbox reflected in the mirror \
         ({reflection_clean} against {reflection_showing}); if that is now \
         true, the note in docs/06 about chrome losing its reflections is \
         wrong and should go"
    );
}

/// The arrangement neither background mode can give on its own: a clean
/// backdrop, a lit product, *and* the softbox still reflected in chrome.
///
/// Background::Colour hides the softbox from the backdrop and from chrome
/// together, because a path that has only ever bounced off mirrors is one the
/// camera sees along. A pin marked not to show is left out of the image the
/// camera reads and kept in the one the renderer samples, so the reflection
/// survives.
#[test]
fn a_hidden_pin_still_lights_and_still_reflects() {
    use tequila_render_core::pins::composite_where;

    fn scene_with(hidden: bool, material: tequila_materials::openpbr::Material) -> Framebuffer {
        // Both: the key lights the camera-facing side and is behind the
        // camera, the second one is what the camera can actually see. Hiding
        // applies to both, so the backdrop assertion is about the visible one
        // and the lighting assertion about the key.
        let pins = [
            Pin {
                show_in_background: !hidden,
                ..key_light()
            },
            Pin {
                show_in_background: !hidden,
                ..visible_light()
            },
        ];
        let base = Image {
            width: 256,
            height: 128,
            channels: 3,
            data: vec![0.02; 256 * 128 * 3],
            color_space: ColorSpace::Raw,
        };
        let mut environment =
            Environment::new(Arc::new(composite_where(&base, &pins, |_| true)), 1.0);
        if hidden {
            environment.set_camera_image(Some(Arc::new(composite_where(
                &base,
                &pins,
                |p| p.show_in_background,
            ))));
        }

        let sphere = tequila_scene::demo::uv_sphere(0.8, 48, 24);
        let scene = SceneSnapshot::new(
            vec![Arc::new(sphere)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
                .with_transform(Transform::IDENTITY)],
            vec![material],
            Arc::new(environment),
        );

        let device = Device::shared().expect("an Embree device");
        let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
        let mut framebuffer = Framebuffer::new(48, 48);
        let context = RenderContext {
            camera: tequila_render_core::camera::Camera::look_at(
                vec3(0.0, 0.0, 7.0),
                Vec3::ZERO,
                Vec3::Y,
            ),
            ..RenderContext::default()
        };
        render_pass(
            &intersector,
            &mut framebuffer,
            &context,
            &ProgressiveSchedule::default(),
            48,
        );
        framebuffer
    }

    let mut mirror = MaterialType::Metal.defaults();
    mirror.specular_roughness = 0.0;
    mirror.base_metalness = 1.0;

    // The backdrop: the pin must be gone from it.
    let shown = scene_with(false, MaterialType::Diffuse.defaults());
    let hidden = scene_with(true, MaterialType::Diffuse.defaults());
    let sky_shown = patch(&shown, 1, 1, 6);
    let sky_hidden = patch(&hidden, 1, 1, 6);
    assert!(
        sky_shown > sky_hidden * 2.0,
        "hiding the pin left the backdrop at {sky_hidden} against {sky_shown}; \
         it is still in the picture"
    );

    // The product: still lit, by the same amount.
    let lit_shown = patch(&shown, 22, 19, 4);
    let lit_hidden = patch(&hidden, 22, 19, 4);
    assert!(
        lit_shown > 0.15 && (lit_shown - lit_hidden).abs() < 0.02,
        "hiding the pin changed the light on the product from {lit_shown} to \
         {lit_hidden}"
    );

    // Chrome: still reflecting it. This is the part Background::Colour cannot
    // do, and the reason the flag exists rather than a note telling people to
    // use a solid background.
    let chrome_shown = brightest(&scene_with(false, mirror.clone()), 16, 14, 32, 30);
    let chrome_hidden = brightest(&scene_with(true, mirror), 16, 14, 32, 30);
    assert!(
        chrome_shown > 0.2,
        "the mirror is not reflecting the softbox at all ({chrome_shown})"
    );
    assert!(
        (chrome_shown - chrome_hidden).abs() < 0.02,
        "hiding the pin took it out of the mirror too ({chrome_hidden} against \
         {chrome_shown}), which is the whole thing this flag exists to avoid"
    );
}
