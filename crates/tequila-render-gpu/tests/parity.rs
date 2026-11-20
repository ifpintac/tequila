//! The card's picture is the processor's (`docs/07` R2.6).
//!
//! `grey_product` from the parity set (R0.3), at its size, samples and seed,
//! through both renderers. They draw the same samples (R2.3) and trace the
//! same paths, so they differ only where floating point sends a path a
//! different way: a ray query and Embree round a triangle's edges
//! differently, the card's division, square root and trigonometry are not
//! correctly rounded, and an FMA here and there. The mean absolute
//! difference must be under 0.002 in scene-linear values, and at most 0.1%
//! of pixels more than 0.02 apart.

use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

#[test]
fn the_grey_product_is_the_processors_picture() {
    same_picture("grey_product");
}

/// R3 tier A's surface: plastics, metals and a diffuse part, with mirror
/// bounces and the stack's lobe choice.
#[test]
fn the_product_in_its_materials_is_the_processors_picture() {
    same_picture("product_materials");
}

/// R3 tier A's floor: the environment's ground as a shadow catcher, its
/// shadow, its mirror reflection and the coverage it takes.
#[test]
fn the_product_on_its_floor_is_the_processors_picture() {
    same_picture("product_floor");
}

/// The floor's blurred reflection, weakened by Fresnel and fading by height,
/// over a tinted shadow.
#[test]
fn the_product_on_a_glossy_floor_is_the_processors_picture() {
    same_picture("product_floor_glossy");
}

/// A studio floor: occlusion shadows, the bounce the product sees, a
/// reflection fading by distance.
#[test]
fn the_product_on_a_studio_floor_is_the_processors_picture() {
    same_picture("product_floor_studio");
}

/// Ground illumination: the floor a bounce meets is shaded, not passed.
#[test]
fn the_product_on_a_lit_floor_is_the_processors_picture() {
    same_picture("product_floor_lit");
}

/// The lamps: a key, a fill and a rim, chosen down the light hierarchy, two
/// with stand-ins; the bounces that meet them, the mirrors that show them,
/// Filter Glossy.
#[test]
fn the_lamp_lit_product_is_the_processors_picture() {
    same_picture("lamp_lit");
}

/// A studio: the sky and three softboxes falling off to their edges, out of
/// the camera's sight, chosen by the alias table.
#[test]
fn the_product_in_a_studio_is_the_processors_picture() {
    same_picture("studio_lamps");
}

/// Lamps the camera sees: a softbox's stand-in glowing as it lights, and
/// mirrored by the lamp itself; a disc's stand-in seen from behind.
#[test]
fn lamps_in_view_are_the_processors_picture() {
    same_picture("lamps_in_view");
}

fn same_picture(name: &str) {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene(name).expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let camera = scene.camera(&document);
    let config = document.lighting.config();

    // Nothing in the set is a case the card renders otherwise.
    let mut limits = tequila_render_gpu::transport::floor_limits(&snapshot, &config);
    limits.extend(tequila_render_gpu::lights::lamp_limits(&snapshot, &config));
    assert!(limits.is_empty(), "{name}: the card renders these otherwise: {limits:?}");

    let started = std::time::Instant::now();
    let mut renderer = GpuRenderer::new(gpu, &snapshot);
    // The processor adds the card's samples up, as it adds up its own: the
    // comparison is of the paths alone.
    renderer.accumulate_on_card = false;
    let mut card = Framebuffer::new(scene.width, scene.height);
    let settings = FrameSettings {
        camera,
        config: config.clone(),
        scene_seed: scene.seed,
        reject_fireflies: true,
        clamp_direct: None,
        clamp_indirect: None,
    };
    renderer.render_pass(&mut card, &settings, scene.samples);
    let card_time = started.elapsed();

    let started = std::time::Instant::now();
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snapshot).expect("builds");
    let context = RenderContext { camera, config, scene_seed: scene.seed, ..RenderContext::default() };
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let mut processor = Framebuffer::new(scene.width, scene.height);
    render_pass(&intersector, &mut processor, &context, &schedule, scene.samples);
    let processor_time = started.elapsed();

    let mut sum = 0.0f64;
    let mut far = 0usize;
    let mut worst = (0.0f32, 0usize);
    let (mut card_mean, mut processor_mean) = (0.0f64, 0.0f64);
    // The alpha too: a shadow catcher's coverage is the floor's shadow and
    // reflection, which the card has to find as the processor does.
    let mut alpha_sum = 0.0f64;
    let mut alpha_worst = 0.0f32;
    for (a, b) in card.pixels().iter().zip(processor.pixels()) {
        let d = (a.alpha() - b.alpha()).abs();
        alpha_sum += d as f64;
        alpha_worst = alpha_worst.max(d);
    }
    for (i, (a, b)) in card.pixels().iter().zip(processor.pixels()).enumerate() {
        let (a, b) = (a.mean(), b.mean());
        let d = [(a.r - b.r).abs(), (a.g - b.g).abs(), (a.b - b.b).abs()];
        sum += (d[0] + d[1] + d[2]) as f64;
        card_mean += (a.r + a.g + a.b) as f64;
        processor_mean += (b.r + b.g + b.b) as f64;
        let largest = d[0].max(d[1]).max(d[2]);
        if largest > 0.02 {
            far += 1;
        }
        if largest > worst.0 {
            worst = (largest, i);
        }
    }
    // TEQUILA_PARITY_DUMP=folder: both pictures as raw little-endian f32
    // RGB, width x height, to look at, and their alphas.
    if let Ok(folder) = std::env::var("TEQUILA_PARITY_DUMP") {
        for (name, buffer) in [("card", &card), ("processor", &processor)] {
            let bytes: Vec<u8> = buffer
                .pixels()
                .iter()
                .flat_map(|p| {
                    let c = p.mean();
                    [c.r, c.g, c.b]
                })
                .flat_map(f32::to_le_bytes)
                .collect();
            std::fs::write(std::path::Path::new(&folder).join(format!("{}-{name}.f32", scene.name)), bytes).expect("writes");
            // And the alpha, one float a pixel: a shadow catcher's shadow
            // and reflection.
            let alpha: Vec<u8> = buffer.pixels().iter().flat_map(|p| p.alpha().to_le_bytes()).collect();
            std::fs::write(std::path::Path::new(&folder).join(format!("{}-{name}-alpha.f32", scene.name)), alpha).expect("writes");
        }
    }
    let pixels = card.pixels().len();
    let mean = sum / (pixels * 3) as f64;
    let share = far as f64 / pixels as f64;
    let alpha_mean = alpha_sum / pixels as f64;
    eprintln!(
        "{} {}x{} at {} samples: card {:.2} s, processor {:.2} s; mean {:.5} against {:.5}; \
         mean |difference| {mean:.6}; {far} pixels ({:.3}%) more than 0.02 apart; worst {:.4} at pixel {}; \
         alpha mean |difference| {alpha_mean:.6}, worst {alpha_worst:.4}",
        scene.name,
        scene.width,
        scene.height,
        scene.samples,
        card_time.as_secs_f64(),
        processor_time.as_secs_f64(),
        card_mean / (pixels * 3) as f64,
        processor_mean / (pixels * 3) as f64,
        100.0 * share,
        worst.0,
        worst.1
    );
    assert!(mean < 0.002, "{name}: the mean absolute difference is {mean:.6}, not under 0.002");
    assert!(share <= 0.001, "{name}: {far} pixels ({:.3}%) are more than 0.02 apart, more than 0.1%", 100.0 * share);
    // A sample's coverage is all or nothing where a shadow catcher's shadow
    // ray is (one direction, blocked or not), so the same paths give the
    // same alpha but where a ray query and Embree round an edge apart:
    // drawing the shadow from another bounce's dimensions, statistically
    // the same shadow, is 0.0015 here and passes the colour's bar.
    assert!(alpha_mean < 1e-4, "{name}: the alpha's mean absolute difference is {alpha_mean:.6}, not under 0.0001");
}

/// The card adding its samples up, firefly rejection and all, gives the
/// picture the processor's `Framebuffer` gives adding the same samples up:
/// the two differ only in the squared luminance the rejection judges by,
/// an f64 on the processor and two f32s on the card.
#[test]
fn adding_up_on_the_card_is_adding_up_on_the_processor() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let settings = FrameSettings {
        camera: scene.camera(&document),
        config: document.lighting.config(),
        scene_seed: scene.seed,
        reject_fireflies: true,
        clamp_direct: None,
        clamp_indirect: None,
    };
    let mut renderer = GpuRenderer::new(gpu, &snapshot);
    let mut on_card = Framebuffer::new(scene.width, scene.height);
    // In two passes, so the sums go to the card and come back between them.
    renderer.render_pass(&mut on_card, &settings, scene.samples / 2);
    renderer.render_pass(&mut on_card, &settings, scene.samples - scene.samples / 2);
    renderer.accumulate_on_card = false;
    let mut on_processor = Framebuffer::new(scene.width, scene.height);
    renderer.render_pass(&mut on_processor, &settings, scene.samples);
    let mut sum = 0.0f64;
    let mut worst = 0.0f32;
    for (a, b) in on_card.pixels().iter().zip(on_processor.pixels()) {
        assert_eq!(a.samples, b.samples);
        let (a, b) = (a.mean(), b.mean());
        let d = (a.r - b.r).abs().max((a.g - b.g).abs()).max((a.b - b.b).abs());
        sum += d as f64;
        worst = worst.max(d);
    }
    let mean = sum / on_card.pixels().len() as f64;
    eprintln!("on the card against on the processor: mean |difference| {mean:.2e}, worst {worst:.2e}");
    assert!(mean < 1e-5 && worst < 0.01, "adding up on the card differs: mean {mean:.2e}, worst {worst:.2e}");
}
