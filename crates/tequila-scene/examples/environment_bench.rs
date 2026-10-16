//! What an environment rebuild costs, split by what actually changed.
//!
//! The cache is keyed so that the expensive part — generating a sky — is not
//! redone when only the cheap part changes. That is a claim about which branch
//! runs, and the honest way to check it is to time both.
//!
//! `cargo run --release -p tequila-scene --example environment_bench`

use tequila_render_core::pins::Pin;
use tequila_render_core::sky::Sky;
use tequila_scene::document::EnvironmentSettings;
use tequila_scene::environment::EnvironmentCache;
use std::time::Instant;

fn best_of<F: FnMut()>(times: usize, mut run: F) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..times {
        let start = Instant::now();
        run();
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    best
}

fn main() {
    let mut cache = EnvironmentCache::new();
    let mut settings = EnvironmentSettings {
        sky: Some(Sky::default()),
        pins: vec![
            Pin {
                name: "Key".into(),
                intensity: 12.0,
                show_in_background: false,
                ..Default::default()
            },
            Pin {
                name: "Fill".into(),
                intensity: 4.0,
                show_in_background: false,
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let cold = best_of(1, || {
        cache.clear();
        let _ = cache.environment(&settings);
    });
    println!("cold build (sky + pins + CDF): {cold:.1} ms");

    let cached = best_of(20, || {
        let _ = cache.environment(&settings);
    });
    println!("unchanged:                     {cached:.2} ms");

    // A pin's brightness, which is what a slider drag looks like. The sky must
    // not be regenerated here.
    let mut brightness = 12.0f32;
    let pin_drag = best_of(10, || {
        brightness += 0.37;
        settings.pins[0].intensity = brightness;
        let _ = cache.environment(&settings);
    });
    println!("pin slider (sky must be kept): {pin_drag:.1} ms");

    // The sky's own slider, which does have to regenerate it.
    let mut turbidity = 2.0f32;
    let sky_drag = best_of(5, || {
        turbidity += 0.11;
        settings.sky = Some(Sky {
            turbidity,
            ..Sky::default()
        });
        let _ = cache.environment(&settings);
    });
    println!("sky slider (rebuilds the sky): {sky_drag:.1} ms");

    // The built-in studio, which is what a scene with no environment image
    // is lit by: its own lights are pins too, over a larger base.
    settings.sky = None;
    let studio_cold = best_of(3, || {
        cache.clear();
        let _ = cache.environment(&settings);
    });
    println!("studio cold build:             {studio_cold:.1} ms");
    let studio_pin = best_of(10, || {
        brightness += 0.37;
        settings.pins[0].intensity = brightness;
        let _ = cache.environment(&settings);
    });
    println!("studio pin slider:             {studio_pin:.1} ms");
}
