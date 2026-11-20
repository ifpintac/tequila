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
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut MeshCache::new());
    let camera = scene.camera(&document);
    let config = document.lighting.config();

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
    // RGB, width x height, to look at.
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
            std::fs::write(std::path::Path::new(&folder).join(format!("grey_product-{name}.f32")), bytes).expect("writes");
        }
    }
    let pixels = card.pixels().len();
    let mean = sum / (pixels * 3) as f64;
    let share = far as f64 / pixels as f64;
    eprintln!(
        "grey_product {}x{} at {} samples: card {:.2} s, processor {:.2} s; mean {:.5} against {:.5}; \
         mean |difference| {mean:.6}; {far} pixels ({:.3}%) more than 0.02 apart; worst {:.4} at pixel {}",
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
    assert!(mean < 0.002, "the mean absolute difference is {mean:.6}, not under 0.002");
    assert!(share <= 0.001, "{far} pixels ({:.3}%) are more than 0.02 apart, more than 0.1%", 100.0 * share);
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
