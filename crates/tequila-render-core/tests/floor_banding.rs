//! Concentric contour lines on a reflective floor.
//!
//! Reported as "curved lines on the ground". Curved lines on a flat floor are
//! concentric circles about the camera's ground point, and a circle on the
//! floor is a set of points at one distance from the camera — which is a set
//! of points whose reflected direction has one *elevation*. So an artefact
//! that appears as rings is an artefact in the elevation axis of the
//! environment lookup, and nothing else can produce that shape.
//!
//! The suspect was bilinear interpolation, and it was a good suspect. Bilinear
//! is continuous in value but not in slope: at every texel boundary the
//! gradient changes abruptly. Normally that is invisible because one texel
//! spans about one pixel, but a floor at a grazing angle destroys that
//! assumption — the elevation of the reflected direction changes very slowly
//! with screen position, so one texel row of the environment can stretch
//! across tens of pixels and the slope breaks land far enough apart to be seen
//! individually. The eye is very good at finding derivative discontinuities in
//! a smooth gradient; that is what a Mach band is.
//!
//! **It was not the cause.** Measured here, a smooth gradient walked along a
//! floor's reflected directions has a worst-to-mean second difference of 7.3
//! with no spikes at all, and 5.0 at grazing angles. The rings came from the
//! last step instead — quantizing to eight bits with no dither, which
//! `tequila-color` now does. See `dither_threshold` there.
//!
//! This file stays because the hypothesis was a reasonable one and the next
//! person to see rings on a floor will have it too. It measures the lookup
//! without rendering anything, by walking the exact sequence of directions a
//! floor produces and taking the second difference of the radiance along it.
//! Smooth shading gives a small, even second difference; a slope break gives a
//! spike. Should someone change how the environment is filtered, this is what
//! says whether the change put structure into a floor that the environment
//! does not have.

use tequila_core::math::{vec3, Vec3};
use tequila_render_core::environment::Environment;
use tequila_texture::{ColorSpace, Image};
use std::sync::Arc;

/// A smooth vertical gradient — the easiest possible environment to
/// interpolate, and one with no detail of its own to be confused with an
/// artefact. Anything structured that comes out of it was put there by the
/// lookup.
fn gradient_environment(width: usize, height: usize) -> Environment {
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let v = (y as f32 + 0.5) / height as f32;
        // Bright above, dark below, with the falloff a studio dome has.
        let value = 0.05 + 2.5 * (1.0 - v).powf(1.5);
        for _ in 0..width {
            data.extend_from_slice(&[value, value, value]);
        }
    }
    Environment::new(
        Arc::new(Image {
            width,
            height,
            channels: 3,
            data,
            color_space: ColorSpace::LinearSrgb,
        }),
        1.0,
    )
}

/// The radiance a mirror floor shows, walking away from the camera.
///
/// Camera at `height` looking along -Z; the floor is y = 0. At distance `d`
/// the incoming direction has elevation `-atan(height / d)` and the mirror
/// reflection has `+atan(height / d)`.
fn floor_walk(environment: &Environment, height: f32, samples: usize) -> Vec<f32> {
    (0..samples)
        .map(|i| {
            // Distances spaced the way screen rows space them: a floor seen in
            // perspective compresses distance towards the horizon, so an even
            // walk in *screen* space is an uneven one in world space. Walking
            // 1/d evenly is that, near enough.
            let t = (i as f32 + 1.0) / samples as f32;
            let d = 1.0 / (t * 0.5);
            let elevation = (height / d).atan();
            let direction = vec3(0.0, elevation.sin(), -elevation.cos())
                .try_normalize()
                .unwrap_or(Vec3::Y);
            environment.radiance(direction).luminance()
        })
        .collect()
}

/// Spikes in the second difference, as a multiple of the mean.
fn banding(values: &[f32]) -> (f64, usize) {
    let second: Vec<f64> = (1..values.len() - 1)
        .map(|i| (values[i + 1] - 2.0 * values[i] + values[i - 1]).abs() as f64)
        .collect();
    let mean = second.iter().sum::<f64>() / second.len() as f64;
    let worst = second.iter().copied().fold(0.0f64, f64::max);
    let spikes = second.iter().filter(|v| **v > mean * 8.0).count();
    (if mean > 0.0 { worst / mean } else { 0.0 }, spikes)
}

/// The environment lookup must not put structure into a floor that the
/// environment does not have.
///
/// A smooth gradient reflected in a flat floor is a smooth gradient. If the
/// second difference along that walk has spikes, the lookup invented them, and
/// they will be visible as the rings this test is named for.
#[test]
fn a_smooth_environment_makes_a_smooth_floor() {
    let environment = gradient_environment(512, 256);
    let walk = floor_walk(&environment, 1.2, 900);
    let (ratio, spikes) = banding(&walk);
    println!("floor walk: worst/mean second difference {ratio:.1}, {spikes} spikes");
    assert!(
        ratio < 12.0,
        "the second difference spikes to {ratio:.0}x its mean along a floor \
         reflecting a smooth gradient, in {spikes} places; those are the \
         contour rings"
    );
}

/// And the same at the grazing angles near the horizon, where a texel row
/// stretches furthest across the screen and the artefact is worst.
#[test]
fn the_far_floor_is_smooth_too() {
    let environment = gradient_environment(512, 256);
    // A low camera: the whole visible floor is then at a grazing angle.
    let walk = floor_walk(&environment, 0.35, 900);
    let (ratio, spikes) = banding(&walk);
    println!("grazing walk: worst/mean second difference {ratio:.1}, {spikes} spikes");
    assert!(
        ratio < 12.0,
        "a low camera makes it worse: {ratio:.0}x the mean in {spikes} places"
    );
}
