//! Does a default sky expose like the studio it replaces?
use tequila_render_core::sky::{time_of_day, Sky};

fn mean_of(image: &tequila_texture::image::Image) -> f32 {
    let sum: f32 = image.data.iter().copied().sum();
    sum / image.data.len() as f32
}

fn main() {
    let studio = tequila_scene_stub::studio_mean();
    println!("built-in studio mean radiance: {studio:.4}");
    for (name, _, _, _) in tequila_render_core::sky::TIMES_OF_DAY {
        let sky = time_of_day(name).expect("a known time");
        let mean = mean_of(&sky.to_image(256, 128));
        println!("{name:<16} mean {mean:8.4}   ({:.1}x studio)", mean / studio);
    }
    let plain = mean_of(&Sky::default().to_image(256, 128));
    println!("{:<16} mean {plain:8.4}   ({:.1}x studio)", "Sky::default", plain / studio);
}

mod tequila_scene_stub {
    pub fn studio_mean() -> f32 {
        let environment = tequila_scene::demo::studio_environment();
        let image = environment.image();
        image.data.iter().copied().sum::<f32>() / image.data.len() as f32
    }
}
