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

/// Whether `scene`'s floor reflection takes its alpha from its brightness
/// (`ShadowCatcher::reflection_luminance_alpha`), so it is laid through
/// [`by_brightness`] first.
pub fn alpha_by_brightness(scene: &tequila_render_core::scene::SceneSnapshot) -> bool {
    scene.instances.iter().any(|i| i.shadow_catcher.is_some_and(|c| c.reflections > 0.0 && c.reflection_luminance_alpha))
}

/// A reflection whose alpha follows its brightness as displayed, rather than
/// its share of the pixel: `reflection` premultiplied by `share`, as the
/// floor pass leaves it, and the image's `exposure` in stops. Returns the
/// premultiplied colour and the alpha to [`lay_reflection`] with.
///
/// A dark reflection laid by its share alone is opaque dark: a grey band
/// under the product on whatever page the picture is placed on. Here the
/// brightest channel as displayed is the alpha and the colour keeps its hue,
/// so a black reflection is transparent, a bright one is as it was, and over
/// black the picture is the same one (the compositor step Blender needed for
/// a V-Ray matte floor). Worked in display primaries with the sRGB curve, the
/// way a viewer composites a PNG.
pub fn by_brightness(reflection: Spectrum, share: f32, exposure: f32) -> (Spectrum, f32) {
    use tequila_core::math::vec3;
    use tequila_core::spectrum::{acescg_to_srgb, srgb_eotf, srgb_inverse_eotf, srgb_to_acescg};
    let share = share.clamp(0.0, 1.0);
    if share <= 0.0 {
        return (Spectrum::ZERO, 0.0);
    }
    let gain = 2f32.powf(exposure);
    // As displayed: straight, exposed, in display primaries, encoded.
    let straight = acescg_to_srgb(reflection.to_rendering_rgb()) * (gain / share);
    let shown = |c: f32| srgb_inverse_eotf(c.clamp(0.0, 1.0));
    let displayed = vec3(shown(straight.x), shown(straight.y), shown(straight.z));
    let peak = displayed.x.max(displayed.y).max(displayed.z);
    if peak <= 1e-6 {
        return (Spectrum::ZERO, 0.0);
    }
    let alpha = share * peak;
    let colour = vec3(srgb_eotf(displayed.x / peak), srgb_eotf(displayed.y / peak), srgb_eotf(displayed.z / peak)) * (alpha / gain);
    (Spectrum::from_rendering_rgb(srgb_to_acescg(colour)), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_core::spectrum::{srgb_inverse_eotf, Spectrum};

    /// What a viewer shows for a straight-alpha pixel over black, encoded.
    fn over_black(colour: Spectrum, alpha: f32, exposure: f32) -> [f32; 3] {
        let gain = 2f32.powf(exposure);
        let straight = if alpha > 0.0 { colour.to_linear_srgb() * (gain / alpha) } else { vec3(0.0, 0.0, 0.0) };
        [straight.x, straight.y, straight.z].map(|c| srgb_inverse_eotf(c.clamp(0.0, 1.0)) * alpha)
    }

    /// A black reflection is transparent; a saturated blue keeps its colour
    /// and loses none of itself; and over black the picture is the one its
    /// share alone gives.
    #[test]
    fn a_reflection_by_brightness_is_transparent_where_dark() {
        let (_, alpha) = by_brightness(Spectrum::ZERO, 0.6, 0.0);
        assert_eq!(alpha, 0.0);

        let share = 0.5;
        let blue = Spectrum::from_linear_srgb(vec3(0.05, 0.35, 1.0));
        let (colour, alpha) = by_brightness(blue * share, share, 0.0);
        assert!((alpha - share).abs() < 1e-4, "a reflection whose brightest channel shows at full keeps its share: {alpha}");
        let back = colour.to_linear_srgb() * (1.0 / alpha);
        let wanted = blue.to_linear_srgb();
        for (got, want) in [back.x, back.y, back.z].into_iter().zip([wanted.x, wanted.y, wanted.z]) {
            assert!((got - want).abs() < 2e-3, "the colour kept its hue: {back:?} for {wanted:?}");
        }

        for (reflection, share, exposure) in [
            (Spectrum::from_linear_srgb(vec3(0.02, 0.08, 0.2)), 0.7, 0.0),
            (Spectrum::from_linear_srgb(vec3(0.3, 0.3, 0.3)), 0.25, 0.474),
            (blue, 1.0, -1.0),
        ] {
            let (laid, alpha) = by_brightness(reflection * share, share, exposure);
            let by_share = over_black(reflection * share, share, exposure);
            let by_light = over_black(laid, alpha, exposure);
            for (a, b) in by_share.into_iter().zip(by_light) {
                assert!((a - b).abs() < 2e-3, "over black it is the same picture: {by_share:?} and {by_light:?}");
            }
            assert!(alpha <= share + 1e-6, "never more opaque than its share");
        }
    }
}
