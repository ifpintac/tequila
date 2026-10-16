//! A diagnostic render of the ground plane, written to a PNG for inspection.
//!
//! Ignored by default: it is a picture to look at, not an assertion. Run with
//! `cargo test -p tequila-render-cpu --test ground_look -- --ignored
//! --nocapture`.

use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};

#[test]
#[ignore = "a picture to look at, not an assertion"]
fn render_the_ground() {
    for reflections in [0.0f32, 0.7] {
        one(reflections);
    }
}

fn one(reflections: f32) {
    let mut document =
        tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Metal, 0.15);
    document.environment.ground.reflections = reflections;
    document.environment.ground.shadows = true;

    let (scene, _) = tequila_scene::snapshot::snapshot(
        &document,
        std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        &mut Default::default(),
    );

    let device = Device::shared().expect("Embree");
    let intersector = EmbreeIntersector::build(&device, scene).expect("BVH");

    let (width, height) = (700usize, 400usize);
    // Low and close, the way the reported render was framed.
    let context = RenderContext {
        camera: Camera::look_at(
            tequila_core::math::vec3(0.0, 1.1, 5.0),
            tequila_core::math::vec3(0.0, 0.9, 0.0),
            tequila_core::math::Vec3::Y,
        ),
        ..Default::default()
    };

    let mut fb = Framebuffer::new(width, height);
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(10_000);
    for _ in 0..24 {
        render_pass(&intersector, &mut fb, &context, &schedule, 4);
    }

    let style = tequila_color::ImageStyle::default();
    let mut linear = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            linear.push(fb.pixel(x, y).mean().sanitized());
        }
    }
    let pixels = style.develop_through(
        width,
        height,
        &linear,
        &tequila_color::DisplayPipeline::default(),
    );

    // A column well away from the sphere, top to bottom: the seam shows as a
    // step in these numbers, and its size says whether it is worth chasing.
    println!("--- reflections {reflections} ---");
    // Run lengths of identical 8-bit codes down a column of the floor. A run
    // is one band; a long run means a wide, visible band.
    let column = 40usize;
    let floor_start = (0..height)
        .find(|&y| fb.pixel(column, y).first_instance != u32::MAX)
        .unwrap_or(height);
    let mut runs: Vec<usize> = Vec::new();
    let mut current = 1usize;
    for y in (floor_start + 1)..height {
        let a = pixels[(y * width + column) * 4];
        let b = pixels[((y - 1) * width + column) * 4];
        if a == b {
            current += 1;
        } else {
            runs.push(current);
            current = 1;
        }
    }
    runs.push(current);
    let longest = runs.iter().copied().max().unwrap_or(0);
    let mean = runs.iter().sum::<usize>() as f64 / runs.len().max(1) as f64;
    println!(
        "  floor from y={floor_start}: {} bands, mean {mean:.1} px, longest {longest} px",
        runs.len()
    );

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/ground_look_{reflections}.png"));
    let file = std::fs::File::create(&path).expect("create");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("header")
        .write_image_data(&pixels)
        .expect("data");
    println!("wrote {}", path.display());
}
