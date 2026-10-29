//! Caustics (`docs/01` §6.5): light a glass focuses onto the floor, traced
//! from the lights as photons and read by the camera path.
//!
//! Three claims, each one way the pass can be wrong: it must add the light
//! the path tracer leaves out and not count what it already counts (the
//! uniform-sky comparison against a long plain render), it must bring the
//! light of a lamp through glass at all (which the path tracer never does —
//! a lamp is never hit by a bounced ray), and it must not depend on how many
//! threads traced it.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet};
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 40;

/// A solid glass ball over a white floor, under `environment`, with
/// `lights`. Solid: the Glass type is a thin pane, which passes light
/// straight through and focuses nothing.
fn scene(environment: Environment, lights: Vec<Light>) -> SceneSnapshot {
    scene_with(MaterialType::GlassSolid.defaults(), environment, lights)
}

fn scene_with(glass: tequila_materials::openpbr::Material, environment: Environment, lights: Vec<Light>) -> SceneSnapshot {
    let sphere = tequila_scene::demo::uv_sphere(0.6, 64, 32);
    let floor = tequila_render_core::scene::Mesh::new(
        vec![vec3(-4.0, -0.75, -4.0), vec3(4.0, -0.75, -4.0), vec3(4.0, -0.75, 4.0), vec3(-4.0, -0.75, 4.0)],
        vec![0, 2, 1, 0, 3, 2],
    )
    .with_normals(vec![Vec3::Y; 4]);
    let mut diffuse = MaterialType::Diffuse.defaults();
    diffuse.base_color = Spectrum::splat(0.8);
    SceneSnapshot::new(
        vec![Arc::new(sphere), Arc::new(floor)],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(Transform::IDENTITY),
        ],
        vec![glass, diffuse],
        Arc::new(environment),
    )
    .with_lights(Arc::new(LightSet::new(lights)))
}

/// A small square lamp above and to one side of the ball, facing it, so the
/// caustic falls on open floor beside the ball rather than under it, where
/// the camera could see it only through the glass.
fn lamp() -> Light {
    let towards = vec3(-1.0, -1.0, 0.0).normalize();
    let side = vec3(0.0, 0.0, 1.0);
    let across = side.cross(towards).normalize();
    Light::new(LightKind::Area(AreaLight {
        position: vec3(3.0, 3.0, 0.0),
        // Wound so the lamp faces the ball.
        half_u: across * 0.1,
        half_v: side * 0.1,
        radiance: Spectrum::splat(400.0),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    }))
}

fn render(scene: SceneSnapshot, caustics: bool, passes: u32) -> Framebuffer {
    render_from(scene, caustics, passes, tequila_render_core::camera::Camera::look_at(vec3(0.0, 2.6, 2.4), vec3(0.0, -0.75, 0.0), Vec3::Y))
}

fn render_from(scene: SceneSnapshot, caustics: bool, passes: u32, camera: tequila_render_core::camera::Camera) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera,
        config: IntegratorConfig { caustics, ..IntegratorConfig::product() },
        reject_fireflies: false,
        ..RenderContext::default()
    };
    // One sample a pass, as the viewport renders: every pass a fresh set of
    // photons and a smaller radius.
    for _ in 0..passes {
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
    }
    framebuffer
}

fn luminance(framebuffer: &Framebuffer, x: usize, y: usize) -> f32 {
    let c = framebuffer.pixel(x, y).mean();
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// The mean over 8 × 8 blocks of the image.
fn blocks(framebuffer: &Framebuffer) -> Vec<f32> {
    let mut out = Vec::new();
    for by in (0..SIZE).step_by(8) {
        for bx in (0..SIZE).step_by(8) {
            let mut sum = 0.0;
            for y in by..by + 8 {
                for x in bx..bx + 8 {
                    sum += luminance(framebuffer, x, y);
                }
            }
            out.push(sum / 64.0);
        }
    }
    out
}

/// Under a uniform sky every path is one the path tracer can find, so a
/// long plain render is the answer. With caustics on, the light through the
/// ball onto the floor comes from photons instead, and the path tracer stops
/// counting it: the two must agree. Counted twice, the floor round the ball
/// comes out too bright; left out, too dark.
#[test]
fn caustics_neither_add_nor_lose_light_under_a_uniform_sky() {
    let sky = || Environment::uniform(Spectrum::splat(1.0));
    let reference = blocks(&render(scene(sky(), Vec::new()), false, 2048));
    let with = blocks(&render(scene(sky(), Vec::new()), true, 256));
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (a, b) = (mean(&reference), mean(&with));
    assert!((a - b).abs() / a < 0.03, "the whole image: {a} without, {b} with caustics");
    for (i, (r, w)) in reference.iter().zip(&with).enumerate() {
        assert!((r - w).abs() / r.max(1e-3) < 0.12, "block {i}: {r} without, {w} with caustics");
    }
}

/// The same for glass that is not quite smooth, as a CAD or OBJ export's
/// usually is: its light is traced as photons too, and the ball's own
/// surface stops sampling the lights past where they are read, so nothing
/// is counted twice there either.
#[test]
fn caustics_through_slightly_rough_glass_neither_add_nor_lose_light() {
    let sky = || Environment::uniform(Spectrum::splat(1.0));
    let rough = || tequila_materials::openpbr::Material { specular_roughness: 0.2, ..MaterialType::GlassSolid.defaults() };
    let reference = blocks(&render(scene_with(rough(), sky(), Vec::new()), false, 2048));
    let with = blocks(&render(scene_with(rough(), sky(), Vec::new()), true, 256));
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (a, b) = (mean(&reference), mean(&with));
    assert!((a - b).abs() / a < 0.03, "the whole image: {a} without, {b} with caustics");
    for (i, (r, w)) in reference.iter().zip(&with).enumerate() {
        assert!((r - w).abs() / r.max(1e-3) < 0.12, "block {i}: {r} without, {w} with caustics");
    }
}

/// A lamp through slightly rough glass, where the lamp's light through the
/// ball is most of what the floor beside it receives. The path tracer alone
/// finds that light here — the ball is rough enough for its surface to
/// sample the lamp — so a long plain render is the answer again, and a
/// caustic counted by the photons and by the ball's own light sampling as
/// well shows up as a patch of floor too bright.
#[test]
fn a_lamp_through_rough_glass_is_counted_once() {
    let dark = || Environment::uniform(Spectrum::splat(0.02));
    let rough = || tequila_materials::openpbr::Material { specular_roughness: 0.25, ..MaterialType::GlassSolid.defaults() };
    let camera = || tequila_render_core::camera::Camera::look_at(vec3(-2.6, 2.0, 2.2), vec3(-0.9, -0.75, 0.0), Vec3::Y);
    let reference = blocks(&render_from(scene_with(rough(), dark(), vec![lamp()]), false, 2048, camera()));
    let with = blocks(&render_from(scene_with(rough(), dark(), vec![lamp()]), true, 512, camera()));
    let brightest = reference.iter().cloned().fold(0.0f32, f32::max);
    for (i, (r, w)) in reference.iter().zip(&with).enumerate() {
        // Where there is light enough to compare.
        if *r > brightest * 0.2 {
            assert!((r - w).abs() / r < 0.15, "block {i}: {r} without, {w} with caustics");
        }
    }
}

/// A lamp's light through the ball lands as a focused spot on the open floor
/// beside it: a patch several pixels wide, brighter than the floor the lamp
/// lights directly, and even — not the few blazing pixels the path tracer
/// finds by chance, which is all there is with caustics off.
#[test]
fn a_lamp_through_glass_makes_a_bright_spot() {
    let dark = || Environment::uniform(Spectrum::splat(0.02));
    // Looking at the floor on the far side of the ball from the lamp.
    let camera = tequila_render_core::camera::Camera::look_at(vec3(-2.6, 2.0, 2.2), vec3(-0.9, -0.75, 0.0), Vec3::Y);
    let on = render_from(scene(dark(), vec![lamp()]), true, 64, camera);
    // The lamp has no geometry to be seen, so the brightest pixel is the
    // caustic's.
    let (mut best, mut at) = (0.0f32, (0, 0));
    for y in 1..SIZE - 1 {
        for x in 1..SIZE - 1 {
            let l = luminance(&on, x, y);
            if l > best {
                (best, at) = (l, (x, y));
            }
        }
    }
    let mut around = 0.0;
    for y in at.1 - 1..=at.1 + 1 {
        for x in at.0 - 1..=at.0 + 1 {
            around += luminance(&on, x, y) / 9.0;
        }
    }
    // Floor the lamp lights directly, in front of the ball.
    let open = luminance(&on, SIZE - 4, 4);
    assert!(open > 0.0, "the open floor is lit");
    assert!(around > open * 3.0, "no caustic: {around} around the brightest pixel against open floor {open}");
    assert!(around > best * 0.25, "the brightest pixel ({best}) stands alone: a firefly, not a spot ({around})");
}

/// The same photons however many threads trace them, so a render is the same
/// on any machine (`docs/01` §4.6).
#[test]
fn caustics_are_the_same_on_any_number_of_threads() {
    let picture = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| render(scene(Environment::uniform(Spectrum::splat(0.02)), vec![lamp()]), true, 3))
    };
    let (one, four) = (picture(1), picture(4));
    for y in 0..SIZE {
        for x in 0..SIZE {
            assert_eq!(one.pixel(x, y).mean(), four.pixel(x, y).mean(), "pixel {x},{y}");
        }
    }
}


/// A shadow-catching floor with its shadows off leaves the picture alone
/// - it is there for its reflection, if for anything - and that is true of
/// the light a glass focuses onto it too, as Cycles has it, where such a
/// floor is not seen at all. With shadows on, the caustic is on the floor.
#[test]
fn a_floor_without_shadows_shows_no_caustic() {
    let dark = || Environment::uniform(Spectrum::splat(0.02));
    let camera = tequila_render_core::camera::Camera::look_at(vec3(-2.6, 2.0, 2.2), vec3(-0.9, -0.75, 0.0), Vec3::Y);
    let on_catcher = |shadows: bool| {
        let mut scene = scene(dark(), vec![lamp()]);
        scene.instances[1].shadow_catcher = Some(tequila_render_core::scene::ShadowCatcher { shadows, ..Default::default() });
        scene
    };
    let brightest = |framebuffer: &Framebuffer| {
        (1..SIZE - 1).flat_map(|y| (1..SIZE - 1).map(move |x| (x, y))).map(|(x, y)| luminance(framebuffer, x, y)).fold(0.0f32, f32::max)
    };
    // The floor's own picture without photons, to measure against.
    let plain = brightest(&render_from(on_catcher(false), false, 32, camera));
    let without = brightest(&render_from(on_catcher(false), true, 32, camera));
    let with = brightest(&render_from(on_catcher(true), true, 32, camera));
    assert!(without < plain * 1.5 + 0.05, "a floor without shadows caught a caustic: {without} against {plain} without photons");
    assert!(with > without * 3.0, "a floor with shadows lost its caustic: {with} against {without}");
}
