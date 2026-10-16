//! A floor's reflection on its own, for an image another renderer made.
//!
//! Cycles cannot make the floor reflection Tequila's renderer makes - the
//! product mirrored on an invisible floor, and never the sky - because its
//! shadow catcher reflects only as a ratio of the backdrop (on a black one
//! nothing shows), and a mirror of its own reflects the environment too.
//! So under Cycles the floor reflects nothing, and this renders the
//! reflection alone (`IntegratorConfig::floor_reflection_only`) through the
//! same camera, to lay over Cycles' image with [`lay_reflection`]: the same
//! reflection as Tequila's renderer, by the same code.

use crate::{render_pass, RenderContext};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::intersect::Intersector;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;

/// `samples` more samples of the floor's reflection, alone, into `layer`:
/// each pixel's mean is what the floor reflects there, its coverage the
/// share that takes of what was there, and both nothing off the floor.
pub fn render_floor_reflection(
    intersector: &dyn Intersector,
    layer: &mut Framebuffer,
    camera: Camera,
    config: &IntegratorConfig,
    samples: u32,
) {
    render_floor_reflection_until(intersector, layer, camera, config, samples, &|| false);
}

/// The same, a few samples at a time, ending early once `stop` says so -
/// after the first few, so there is always a reflection to lay over. One
/// call for all of them could not be stopped: a 4K frame at 512 samples
/// held a cancelled render, and quitting the application, for minutes.
pub fn render_floor_reflection_until(
    intersector: &dyn Intersector,
    layer: &mut Framebuffer,
    camera: Camera,
    config: &IntegratorConfig,
    samples: u32,
    stop: &dyn Fn() -> bool,
) {
    const CHUNK: u32 = 8;
    let context = RenderContext {
        camera,
        config: IntegratorConfig { floor_reflection_only: true, ..config.clone() },
        ..RenderContext::default()
    };
    let mut done = 0;
    while done < samples {
        let now = CHUNK.min(samples - done);
        render_pass(intersector, layer, &context, &ProgressiveSchedule::default(), now);
        done += now;
        if stop() {
            break;
        }
    }
}

/// A pixel's colour and alpha with the floor's reflection laid over it: the
/// reflection takes its share of what was there, as a glossy floor's does,
/// and counts toward the alpha.
pub fn lay_reflection(colour: Spectrum, alpha: f32, reflection: Spectrum, share: f32) -> (Spectrum, f32) {
    let share = share.clamp(0.0, 1.0);
    (colour * (1.0 - share) + reflection, alpha + share * (1.0 - alpha))
}

/// Whether `scene` has a floor that reflects.
pub fn reflects(scene: &tequila_render_core::scene::SceneSnapshot) -> bool {
    scene.instances.iter().any(|i| i.shadow_catcher.is_some_and(|c| c.reflections > 0.0))
}
