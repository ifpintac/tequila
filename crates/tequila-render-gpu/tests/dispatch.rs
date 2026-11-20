//! How the card's work is cut into dispatches (`GpuRenderer::render_pass`):
//! each within the time a display allows the card, however big the frame,
//! and the picture the same however it is cut.

use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

/// The frame's mean colour.
fn mean(frame: &Framebuffer) -> [f32; 3] {
    let n = frame.pixels().len().max(1) as f32;
    frame.pixels().iter().fold([0.0; 3], |m, p| {
        let c = p.mean();
        [m[0] + c.r / n, m[1] + c.g / n, m[2] + c.b / n]
    })
}

/// The renderer for parity scene `name`, and its frame settings.
fn scene(name: &str) -> Option<(GpuRenderer, FrameSettings)> {
    let gpu = device::for_test(Needs::RayQueries)?;
    let scene = parity::scene(name).expect("the parity set has it");
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
    Some((GpuRenderer::new(gpu, &snapshot), settings))
}

/// A frame cut into one sample of every few rows at a time is the picture a
/// frame drawn whole is, to the bit, the card adding the samples up or the
/// processor.
#[test]
fn a_frame_cut_finely_is_the_same_picture() {
    let Some((mut renderer, settings)) = scene("bevelled_product") else { return };
    let (width, height, samples) = (96, 54, 6);
    for on_card in [false, true] {
        renderer.accumulate_on_card = on_card;
        let mut whole = Framebuffer::new(width, height);
        renderer.set_dispatch_work((width * height) as u64 * 16);
        renderer.render_pass(&mut whole, &settings, samples);
        let mut cut = Framebuffer::new(width, height);
        // Two rows' worth a dispatch to begin with: a stride of 27, growing
        // as the dispatches are timed, so the frame is cut several ways.
        renderer.set_dispatch_work(2 * width as u64);
        renderer.render_pass(&mut cut, &settings, samples);
        let mut apart = 0;
        for (a, b) in whole.pixels().iter().zip(cut.pixels()) {
            let same = a.samples == b.samples
                && a.sum.r.to_bits() == b.sum.r.to_bits()
                && a.sum.g.to_bits() == b.sum.g.to_bits()
                && a.sum.b.to_bits() == b.sum.b.to_bits()
                && a.background_sum.r.to_bits() == b.background_sum.r.to_bits()
                && a.coverage_sum.to_bits() == b.coverage_sum.to_bits()
                && a.clamped_energy.to_bits() == b.clamped_energy.to_bits();
            if !same {
                apart += 1;
            }
        }
        assert!(mean(&whole)[0] > 0.01, "the picture is drawn");
        assert_eq!(apart, 0, "{apart} of {} pixels differ when the frame is cut finely (on the card: {on_card})", width * height);
    }
}

/// A big frame of a heavy scene: 16 samples of 1280x720 in one dispatch ran
/// past Windows' two seconds on the RTX 2060 and the card was reset. Cut by
/// time, it finishes, and is a picture.
#[test]
fn a_big_frame_finishes_within_the_time_a_display_allows() {
    let Some((renderer, settings)) = scene("bevelled_product") else { return };
    let (width, height) = (1280, 720);
    let mut frame = Framebuffer::new(width, height);
    let started = std::time::Instant::now();
    renderer.render_pass(&mut frame, &settings, 16);
    eprintln!("1280x720 at 16 samples: {:.2} s, compile included", started.elapsed().as_secs_f64());
    assert!(frame.pixels().iter().all(|p| p.samples == 16), "every pixel has its 16 samples");
    let mean = mean(&frame);
    assert!(mean.iter().all(|c| *c > 0.05), "the picture is drawn, not black: {mean:?}");
}
