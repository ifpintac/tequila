//! What a sky costs to build, which decides whether its sliders are usable.
//!
//! The first version of the model took 1.6 seconds for a 512x256 sky and
//! rebuilt on every slider frame, which would have made the controls
//! unusable. Nothing in the test suite could have told me that — tests say
//! whether an answer is right, not whether it arrives in time — so this
//! exists to be run when the model changes.
//!
//! `cargo run --release -p tequila-render-core --example sky_bench`

use tequila_render_core::sky::Sky;
use std::time::Instant;

fn main() {
    let sky = Sky::default();
    for (width, height) in [(128usize, 64usize), (256, 128), (512, 256)] {
        // Warmed first: the optical-depth table is built once per process,
        // and timing its construction as though it were per-image would
        // report a cost nobody pays twice.
        let _ = sky.to_image(width, height);

        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let start = Instant::now();
            let image = sky.to_image(width, height);
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(&image);
        }
        println!("sky {width}x{height}: {best:.1} ms");

        let image = sky.to_image(width, height);
        let start = Instant::now();
        let environment =
            tequila_render_core::environment::Environment::new(std::sync::Arc::new(image), 1.0);
        println!("  + CDF: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&environment);
    }
}
