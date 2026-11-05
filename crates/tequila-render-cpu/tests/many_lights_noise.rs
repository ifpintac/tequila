//! How noisy a scene of many lamps of very different power is at equal
//! time, the way each strategy of choosing a lamp to sample takes it: one
//! big softbox over a product, and a ring of small, weak accent lamps, each
//! lighting its own patch of floor, where it outshines the softbox several
//! times over. By power alone an accent lamp is chosen one sample in forty,
//! and its patch, which it lights nearly alone, is that starved.
//!
//! Run by hand (`cargo test -p tequila-render-cpu --release --test
//! many_lights_noise -- --ignored --nocapture`); the environment variables
//! it reads are listed on the test. What it measured is in `docs/01` §5.3.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet, SelectionStrategy};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: (usize, usize) = (240, 160);

/// A square lamp `half` across at `at`, facing `facing`.
fn lamp(at: Vec3, facing: Vec3, half: f32, radiance: f32) -> Light {
    let facing = facing.normalize();
    let across = if facing.y.abs() < 0.9 { Vec3::Y.cross(facing).normalize() } else { Vec3::X.cross(facing).normalize() };
    let up = facing.cross(across).normalize();
    let (half_u, half_v) = if across.cross(up).dot(facing) > 0.0 { (across * half, up * half) } else { (up * half, across * half) };
    Light::new(LightKind::Area(AreaLight { position: at, half_u, half_v, radiance: Spectrum::splat(radiance), disc: false, two_sided: false, spread: std::f32::consts::PI }))
}

/// Where accent lamp `i` of `count` stands, and the patch of floor it
/// lights.
pub fn accent(i: usize, count: usize) -> (Vec3, Vec3) {
    let angle = std::f32::consts::PI * (0.1 + 0.8 * i as f32 / (count.max(2) - 1) as f32);
    let out = vec3(angle.cos(), 0.0, -angle.sin().abs() * 0.6 + 0.3);
    let at = out * 2.0 + vec3(0.0, 0.3, 0.0);
    let patch = out * 2.4;
    (at, patch)
}

/// The studio: a floor and a back wall, a ball, a big softbox above and in
/// front, and `accents` small lamps a twentieth of its power or less, low
/// round the back, each aimed down at its own patch of floor.
pub fn studio(accents: usize, strategy: Option<SelectionStrategy>) -> SceneSnapshot {
    let floor = Mesh::new(vec![vec3(-6.0, 0.0, -3.0), vec3(6.0, 0.0, -3.0), vec3(6.0, 0.0, 6.0), vec3(-6.0, 0.0, 6.0)], vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4]);
    let wall = Mesh::new(vec![vec3(-6.0, 0.0, -3.0), vec3(6.0, 0.0, -3.0), vec3(6.0, 4.0, -3.0), vec3(-6.0, 4.0, -3.0)], vec![0, 1, 2, 0, 2, 3])
        .with_normals(vec![Vec3::Z; 4]);
    let ball = tequila_scene::demo::uv_sphere(0.45, 64, 32);
    let mut grey = MaterialType::Diffuse.defaults();
    grey.base_color = Spectrum::splat(0.6);
    let plastic = Material::plastic(Spectrum { r: 0.6, g: 0.15, b: 0.1 }, 0.3);
    let scene = SceneSnapshot::new(
        vec![Arc::new(floor), Arc::new(wall), Arc::new(ball)],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(0)),
            Instance::new(InstanceId(2), MeshId(2), MaterialId(1)).with_transform(Transform::translation(vec3(0.0, 0.45, 0.0))),
        ],
        vec![grey, plastic],
        Arc::new(Environment::uniform(Spectrum::splat(0.02))),
    );
    let mut lamps = vec![lamp(vec3(0.0, 2.8, 1.2), vec3(0.0, -1.0, -0.4), 0.8, 6.0)];
    for i in 0..accents {
        let (at, patch) = accent(i, accents);
        lamps.push(lamp(at, patch - at, 0.04, 60.0));
    }
    let set = match strategy {
        Some(strategy) => LightSet::with_strategy(lamps, strategy),
        None => LightSet::new(lamps),
    };
    scene.with_lights(Arc::new(set))
}

pub fn camera() -> tequila_render_core::camera::Camera {
    tequila_render_core::camera::Camera::look_at(vec3(0.0, 3.2, 5.0), vec3(0.0, 0.2, -0.6), Vec3::Y)
}

/// What one render accumulates, a sample a pass.
struct Run {
    context: RenderContext,
    framebuffer: Framebuffer,
    seconds: f32,
    samples: u32,
}

impl Run {
    fn new(seed: u32) -> Self {
        Run {
            context: RenderContext { camera: camera(), config: IntegratorConfig::product(), reject_fireflies: false, scene_seed: seed, ..RenderContext::default() },
            framebuffer: Framebuffer::new(SIZE.0, SIZE.1),
            seconds: 0.0,
            samples: 0,
        }
    }

    fn go(&mut self, intersector: &EmbreeIntersector, seconds: f32, samples: u32) {
        let start = std::time::Instant::now();
        let mut done = 0;
        while done < samples && start.elapsed().as_secs_f32() < seconds {
            render_pass(intersector, &mut self.framebuffer, &self.context, &Default::default(), 1);
            done += 1;
        }
        self.samples += done;
        self.seconds += start.elapsed().as_secs_f32();
    }
}

fn linear(framebuffer: &Framebuffer) -> Vec<Spectrum> {
    let mut out = Vec::with_capacity(SIZE.0 * SIZE.1);
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            out.push(framebuffer.pixel(x, y).mean().sanitized());
        }
    }
    out
}

fn develop(image: &[Spectrum]) -> Vec<u8> {
    tequila_color::ImageStyle::default().develop_through(SIZE.0, SIZE.1, image, &tequila_color::DisplayPipeline::default())
}

fn shown(image: &[Spectrum]) -> Vec<f64> {
    develop(image).chunks_exact(4).map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])).collect()
}

fn picture(folder: &std::path::Path, name: &str, image: &[Spectrum]) {
    let pixels = develop(image);
    let file = std::fs::File::create(folder.join(format!("{name}.png"))).expect("create");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIZE.0 as u32, SIZE.1 as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("header").write_image_data(&pixels).expect("data");
}

fn save(path: &std::path::Path, image: &[Spectrum]) {
    let mut bytes = Vec::new();
    for s in image {
        for v in [s.r, s.g, s.b] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).expect("writes the reference");
}

fn load(path: &std::path::Path) -> Option<Vec<Spectrum>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() != SIZE.0 * SIZE.1 * 12 {
        return None;
    }
    let value = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    Some((0..SIZE.0 * SIZE.1).map(|i| Spectrum { r: value(i * 3), g: value(i * 3 + 1), b: value(i * 3 + 2) }).collect())
}

/// The pixel boxes measured: the whole picture, and 12 by 12 about each
/// accent lamp's patch the camera sees.
fn boxes(accents: usize) -> Vec<(usize, usize, usize, usize)> {
    let mut out = vec![(0, 0, SIZE.0, SIZE.1)];
    let aspect = SIZE.0 as f32 / SIZE.1 as f32;
    for i in 0..accents {
        let (_, patch) = accent(i, accents);
        if let Some(film) = camera().project(patch, aspect) {
            let (x, y) = ((film.x * SIZE.0 as f32) as i64, (film.y * SIZE.1 as f32) as i64);
            if x >= 6 && y >= 6 && x + 6 <= SIZE.0 as i64 && y + 6 <= SIZE.1 as i64 {
                out.push((x as usize - 6, y as usize - 6, x as usize + 6, y as usize + 6));
            }
        }
    }
    out
}

/// RMS from `reference` over `area`, of `values`.
fn rms(values: &[f64], reference: &[f64], area: (usize, usize, usize, usize)) -> f64 {
    let (x0, y0, x1, y1) = area;
    let mut squares = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            squares += (values[y * SIZE.0 + x] - reference[y * SIZE.0 + x]).powi(2);
        }
    }
    (squares / ((x1 - x0) * (y1 - y0)) as f64).sqrt()
}

fn mean(values: &[f64], area: (usize, usize, usize, usize)) -> f64 {
    let (x0, y0, x1, y1) = area;
    let mut sum = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            sum += values[y * SIZE.0 + x];
        }
    }
    sum / ((x1 - x0) * (y1 - y0)) as f64
}

/// The strategies, rendered for the same time in turns of half a second
/// each, so that what else the machine is doing slows them alike, and
/// measured against a long reference: RMS as shown (0 to 255) over the
/// whole picture and over the accent lamps' patches, and the relative RMS
/// of the linear luminance there. `TEQUILA_NOISE_SECONDS` sets the time
/// (10 s), `TEQUILA_ACCENTS` the counts of accent lamps to measure, by
/// commas (6 and 12: a set of seven lamps, and of thirteen), `TEQUILA_NOISE_REFERENCE`
/// the reference's samples (4096), `TEQUILA_NOISE_CACHE` a folder to keep
/// it in, `TEQUILA_NOISE_PICTURES` one to write the pictures to, named after
/// each strategy with `TEQUILA_NOISE_LABEL` in front.
#[test]
#[ignore]
fn many_lamps_of_different_power_at_equal_time() {
    let number = |name: &str, default: f32| std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default);
    let seconds = number("TEQUILA_NOISE_SECONDS", 10.0);
    let counts: Vec<usize> = std::env::var("TEQUILA_ACCENTS").unwrap_or_else(|_| "6,12".into()).split(',').filter_map(|s| s.trim().parse().ok()).collect();
    for accents in counts {
        measure(accents, seconds);
    }
}

fn measure(accents: usize, seconds: f32) {
    let number = |name: &str, default: f32| std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default);
    let reference_samples = number("TEQUILA_NOISE_REFERENCE", 4096.0) as u32;
    let label = format!("{}{accents}-", std::env::var("TEQUILA_NOISE_LABEL").unwrap_or_default());
    let device = Device::shared().expect("an Embree device");

    let cache = std::env::var_os("TEQUILA_NOISE_CACHE").map(std::path::PathBuf::from);
    let cached = cache.as_ref().map(|c| c.join(format!("many-lights-{accents}-{reference_samples}.bin")));
    let reference = match cached.as_deref().and_then(load) {
        Some(reference) => reference,
        None => {
            let intersector = EmbreeIntersector::build(&device, studio(accents, None)).expect("builds");
            let mut run = Run::new(1234);
            run.go(&intersector, f32::INFINITY, reference_samples);
            let image = linear(&run.framebuffer);
            if let Some(path) = &cached {
                std::fs::create_dir_all(path.parent().unwrap()).ok();
                save(path, &image);
            }
            println!("reference: {reference_samples} samples in {:.0} s", run.seconds);
            image
        }
    };
    let pictures = std::env::var_os("TEQUILA_NOISE_PICTURES").map(std::path::PathBuf::from);
    if let Some(folder) = &pictures {
        std::fs::create_dir_all(folder).ok();
        picture(folder, &format!("{label}reference"), &reference);
    }
    let reference_shown = shown(&reference);
    let reference_luminance: Vec<f64> = reference.iter().map(|s| f64::from(s.luminance())).collect();

    let strategies = [("table", SelectionStrategy::Alias), ("tree", SelectionStrategy::Hierarchy)];
    let scenes: Vec<_> = strategies.iter().map(|(_, s)| EmbreeIntersector::build(&device, studio(accents, Some(*s))).expect("builds")).collect();
    let mut runs: Vec<Run> = strategies.iter().map(|_| Run::new(7)).collect();
    let turn = 0.5;
    let mut spent = 0.0;
    while spent < seconds {
        for (run, scene) in runs.iter_mut().zip(&scenes) {
            run.go(scene, turn, u32::MAX);
        }
        spent += turn;
    }
    let areas = boxes(accents);
    println!("{accents} accent lamps, {seconds} s each, {} patches in view", areas.len() - 1);
    for ((name, _), run) in strategies.iter().zip(&runs) {
        let image = linear(&run.framebuffer);
        let image_shown = shown(&image);
        let luminance: Vec<f64> = image.iter().map(|s| f64::from(s.luminance())).collect();
        let picture_rms = rms(&image_shown, &reference_shown, areas[0]);
        let patches_rms = (areas[1..].iter().map(|a| rms(&image_shown, &reference_shown, *a).powi(2)).sum::<f64>() / (areas.len() - 1) as f64).sqrt();
        let relative = (areas[1..].iter().map(|a| (rms(&luminance, &reference_luminance, *a) / mean(&reference_luminance, *a)).powi(2)).sum::<f64>() / (areas.len() - 1) as f64).sqrt();
        let bias = areas[1..].iter().map(|a| mean(&luminance, *a) / mean(&reference_luminance, *a)).sum::<f64>() / (areas.len() - 1) as f64;
        println!(
            "{name}: {} samples in {:.1} s; RMS as shown {picture_rms:.2} over the picture, {patches_rms:.2} in the patches; relative RMS in the patches {relative:.3}; patches' mean {bias:.3} of the reference's",
            run.samples, run.seconds
        );
        if let Some(folder) = &pictures {
            picture(folder, &format!("{label}{name}"), &image);
        }
    }
}
