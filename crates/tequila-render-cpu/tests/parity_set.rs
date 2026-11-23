//! The parity set on the processor (`docs/07` R0.3).
//!
//! Each scene renders, comes out finite and not black, and comes out the
//! same twice from the same seed: what the graphics card's half of the
//! test (R2.6) will need of the processor's half to compare against.

use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_scene::parity::SET;
use tequila_scene::snapshot::snapshot;

/// A few of each scene's samples: enough to tell two renders apart.
const SAMPLES: u32 = 16;

fn render(scene: &tequila_scene::parity::ParityScene) -> Vec<[f32; 3]> {
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut scene.cache());
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snapshot).expect("builds");
    let context = RenderContext {
        camera: scene.camera(&document),
        config: document.lighting.config(),
        scene_seed: scene.seed,
        ..RenderContext::default()
    };
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let mut framebuffer = Framebuffer::new(scene.width, scene.height);
    render_pass(&intersector, &mut framebuffer, &context, &schedule, SAMPLES.min(scene.samples));
    framebuffer.pixels().iter().map(|p| {
        let c = p.mean();
        [c.r, c.g, c.b]
    }).collect()
}

#[test]
fn every_parity_scene_renders_the_same_twice() {
    for scene in SET {
        let first = render(scene);
        assert!(first.iter().flatten().all(|v| v.is_finite()), "{}: a pixel is not finite", scene.name);
        let mean = first.iter().flatten().map(|&v| v as f64).sum::<f64>() / (first.len() * 3) as f64;
        assert!(mean > 1e-3, "{}: black ({mean})", scene.name);
        assert!(first == render(scene), "{}: two renders from seed {} differ", scene.name, scene.seed);
    }
}
