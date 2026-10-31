//! How noisy manifold caustics are beside the photons' at equal time,
//! measured against one reference both are trying to reach: brute-force
//! path tracing of the same studio with its lamps made glowing geometry,
//! which bounces through the glass can meet, so the light through it is
//! found with nothing solved for and nothing blurred.
//!
//! Run by hand (`cargo test -p tequila-render-cpu --release --test
//! manifold_noise -- --ignored --nocapture`); the environment variables it
//! reads are listed on the test. The reference takes some 25 minutes on a
//! desktop processor, so it can be kept between runs. What it measured is
//! in `docs/01` §6.5.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet, PointLight};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::{CausticsMethod, IntegratorConfig};
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: (usize, usize) = (320, 210);

/// The boxes measured, in pixels: the whole picture, the ball's caustic and
/// the flacon's.
const BOXES: [(&str, (usize, usize, usize, usize)); 3] = [("picture", (0, 0, 320, 210)), ("ball", (75, 120, 145, 165)), ("flacon", (160, 108, 300, 140))];

fn lathe(profile: &[(f32, f32)], around: usize) -> Mesh {
    let last = profile.len() - 1;
    let mut positions = Vec::new();
    let mut rings = Vec::new();
    for (i, &(r, y)) in profile.iter().enumerate() {
        rings.push(positions.len() as u32);
        if i == 0 || i == last {
            positions.push(vec3(0.0, y, 0.0));
        } else {
            for j in 0..around {
                let phi = j as f32 / around as f32 * std::f32::consts::TAU;
                positions.push(vec3(r * phi.cos(), y, r * phi.sin()));
            }
        }
    }
    let at = |ring: usize, j: usize| if ring == 0 || ring == last { rings[ring] } else { rings[ring] + j as u32 };
    let mut indices = Vec::new();
    for i in 0..last {
        for j in 0..around {
            let k = (j + 1) % around;
            let (a, b, c, d) = (at(i, j), at(i, k), at(i + 1, j), at(i + 1, k));
            if i != 0 {
                indices.extend_from_slice(&[a, c, b]);
            }
            if i + 1 != last {
                indices.extend_from_slice(&[b, c, d]);
            }
        }
    }
    let mut mesh = Mesh::new(positions, indices);
    mesh.compute_smooth_normals();
    mesh
}

fn arc(c: (f32, f32), r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    (0..=6)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / 6.0).to_radians();
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// The caustics studio of the pictures (`app` test `caustics_pictures`) -
/// a glass ball and a perfume flacon on a pale floor under a softbox and a
/// small round lamp where the spot is there - with its lamps as lamps, or
/// with them `glowing`: a panel and a ball of light as bright, which a
/// bounce can meet.
fn studio(glowing: bool) -> SceneSnapshot {
    let floor = Mesh::new(vec![vec3(-3.0, 0.0, -3.0), vec3(3.0, 0.0, -3.0), vec3(3.0, 0.0, 3.0), vec3(-3.0, 0.0, 3.0)], vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4]);
    let ball = tequila_scene::demo::uv_sphere(0.1, 160, 80);
    let mut outer = vec![(0.0, 0.0)];
    outer.extend(arc((0.05, 0.012), 0.012, -90.0, 0.0));
    outer.extend(arc((0.05, 0.128), 0.012, 0.0, 90.0));
    outer.extend([(0.03, 0.142), (0.016, 0.148), (0.016, 0.16), (0.0, 0.16)]);
    let mut inner = vec![(0.0, 0.016)];
    inner.extend(arc((0.034, 0.03), 0.014, -90.0, 0.0));
    inner.extend(arc((0.034, 0.1), 0.014, 0.0, 90.0));
    inner.push((0.0, 0.114));
    let clear = Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, ..MaterialType::GlassSolid.defaults() };
    let scent = Material { transmission_color: Spectrum { r: 0.95, g: 0.55, b: 0.35 }, transmission_depth: 0.05, specular_ior: 1.36, ..MaterialType::Liquid.defaults() };
    let pale = Material { base_color: Spectrum::splat(0.75), ..MaterialType::Diffuse.defaults() };
    let mut meshes = vec![Arc::new(floor), Arc::new(ball), Arc::new(lathe(&outer, 128)), Arc::new(lathe(&inner, 128))];
    let mut materials = vec![pale, clear.clone(), clear, scent];
    let at = |x: f32, y: f32, z: f32| Transform::translation(vec3(x, y, z));
    let mut instances = vec![
        Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
        Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(at(-0.17, 0.1, 0.02)),
        Instance::new(InstanceId(2), MeshId(2), MaterialId(2)).with_transform(at(0.13, 0.0, -0.03)),
        Instance::new(InstanceId(3), MeshId(3), MaterialId(3)).with_transform(at(0.13, 0.0, -0.03)),
    ];
    // A round lamp 6 cm across, as bright as the spot it stands in for.
    let (lamp_at, radius, intensity) = (vec3(-0.45, 0.62, -0.6), 0.03f32, 1.6f32);
    let towards = |from: Vec3, to: Vec3| (to - from).normalize();
    let box_at = vec3(0.55, 0.5, -0.55);
    let facing = towards(box_at, vec3(0.13, 0.05, -0.03));
    let side = facing.cross(Vec3::Y).normalize();
    let up = side.cross(facing).normalize();
    let (half_u, half_v, radiance) = (up * 0.07, side * 0.07, 60.0f32);
    let mut lights = Vec::new();
    if glowing {
        let mut add = |mesh: Mesh, luminance: f32| {
            let i = meshes.len() as u32;
            meshes.push(Arc::new(mesh));
            materials.push(Material { emission_color: Spectrum::ONE, emission_luminance: luminance, ..MaterialType::Emissive.defaults() });
            instances.push(Instance::new(InstanceId(i), MeshId(i), MaterialId(i)));
        };
        let mut sphere = tequila_scene::demo::uv_sphere(radius, 48, 24);
        for p in &mut sphere.positions {
            *p += lamp_at;
        }
        // A ball of radiance L has intensity L pi r^2 every way.
        add(sphere, intensity / (std::f32::consts::PI * radius * radius));
        let normal = half_u.cross(half_v).normalize();
        let corners = vec![box_at - half_u - half_v, box_at + half_u - half_v, box_at + half_u + half_v, box_at - half_u + half_v];
        add(Mesh::new(corners, vec![0, 1, 2, 0, 2, 3]).with_normals(vec![normal; 4]), radiance);
    } else {
        lights.push(Light::new(LightKind::Point(PointLight { position: lamp_at, intensity: Spectrum::splat(intensity), radius })));
        lights.push(Light::new(LightKind::Area(AreaLight { position: box_at, half_u, half_v, radiance: Spectrum::splat(radiance), disc: false, two_sided: false, spread: std::f32::consts::PI })));
    }
    SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::splat(0.025)))).with_lights(Arc::new(LightSet::new(lights)))
}

fn camera() -> tequila_render_core::camera::Camera {
    tequila_render_core::camera::Camera::look_at(vec3(-0.02, 0.5, 0.82), vec3(-0.02, 0.02, 0.03), Vec3::Y)
}

/// What one render accumulates, a sample a pass.
struct Run {
    context: RenderContext,
    framebuffer: Framebuffer,
    seconds: f32,
}

impl Run {
    fn new(config: IntegratorConfig, seed: u32) -> Self {
        Run {
            context: RenderContext { camera: camera(), config, reject_fireflies: false, scene_seed: seed, ..RenderContext::default() },
            framebuffer: Framebuffer::new(SIZE.0, SIZE.1),
            seconds: 0.0,
        }
    }

    /// Passes until `seconds` more have gone, or `samples` have been taken.
    fn go(&mut self, intersector: &EmbreeIntersector, seconds: f32, samples: u32) {
        let start = std::time::Instant::now();
        let mut done = 0;
        while done < samples && start.elapsed().as_secs_f32() < seconds {
            render_pass(intersector, &mut self.framebuffer, &self.context, &Default::default(), 1);
            done += 1;
        }
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

fn luminance(image: &[Spectrum]) -> Vec<f64> {
    image.iter().map(|s| f64::from(s.luminance())).collect()
}

/// `image` developed as the app's default look develops it: 8-bit RGBA.
fn develop(image: &[Spectrum]) -> Vec<u8> {
    tequila_color::ImageStyle::default().develop_through(SIZE.0, SIZE.1, image, &tequila_color::DisplayPipeline::default())
}

/// The luminance of each pixel of `image` as it is shown, 0 to 255.
fn shown(image: &[Spectrum]) -> Vec<f64> {
    develop(image).chunks_exact(4).map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])).collect()
}

/// RMS from `reference` in each box, as shown, 0 to 255.
fn shown_rms(image: &[f64], reference: &[f64]) -> Vec<f64> {
    BOXES
        .iter()
        .map(|(_, (x0, y0, x1, y1))| {
            let mut squares = 0.0;
            for y in *y0..*y1 {
                for x in *x0..*x1 {
                    squares += (image[y * SIZE.0 + x] - reference[y * SIZE.0 + x]).powi(2);
                }
            }
            (squares / ((x1 - x0) * (y1 - y0)) as f64).sqrt()
        })
        .collect()
}

/// `image` developed as the app's default look develops it, to `folder`.
fn picture(folder: &std::path::Path, name: &str, image: &[Spectrum]) {
    let pixels = develop(image);
    let path = folder.join(format!("{name}.png"));
    let file = std::fs::File::create(&path).expect("create");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIZE.0 as u32, SIZE.1 as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("header").write_image_data(&pixels).expect("data");
}

fn save(path: &std::path::Path, halves: &[Vec<Spectrum>; 2]) {
    let mut bytes = Vec::new();
    for half in halves {
        for s in half {
            for v in [s.r, s.g, s.b] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    std::fs::write(path, bytes).expect("writes the reference");
}

fn load(path: &std::path::Path) -> Option<[Vec<Spectrum>; 2]> {
    let bytes = std::fs::read(path).ok()?;
    let count = SIZE.0 * SIZE.1;
    if bytes.len() != count * 2 * 12 {
        return None;
    }
    let value = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    let half = |h: usize| (0..count).map(|i| Spectrum { r: value((h * count + i) * 3), g: value((h * count + i) * 3 + 1), b: value((h * count + i) * 3 + 2) }).collect();
    Some([half(0), half(1)])
}

/// RMS from `reference` in each box, over the box's mean of the reference.
fn relative_rms(image: &[f64], reference: &[f64]) -> Vec<f64> {
    BOXES
        .iter()
        .map(|(_, (x0, y0, x1, y1))| {
            let (mut squares, mut sum, mut count) = (0.0, 0.0, 0usize);
            for y in *y0..*y1 {
                for x in *x0..*x1 {
                    let i = y * SIZE.0 + x;
                    squares += (image[i] - reference[i]).powi(2);
                    sum += reference[i];
                    count += 1;
                }
            }
            (squares / count as f64).sqrt() / (sum / count as f64)
        })
        .collect()
}

/// The box means, for the bias each method has against the reference.
fn means(image: &[f64]) -> Vec<f64> {
    BOXES
        .iter()
        .map(|(_, (x0, y0, x1, y1))| {
            let mut sum = 0.0;
            for y in *y0..*y1 {
                for x in *x0..*x1 {
                    sum += image[y * SIZE.0 + x];
                }
            }
            sum / ((x1 - x0) * (y1 - y0)) as f64
        })
        .collect()
}

/// Brute-force path tracing as the reference takes it: no lamp, so no
/// manifold sample, and glass that bends light stopping the glowing
/// lamps' light samples, so light through it is the bounces' alone, each
/// counted whole; and no roughness floor, which would blur it.
fn reference_config() -> IntegratorConfig {
    IntegratorConfig { caustics: true, caustics_method: CausticsMethod::Manifold, regularization_roughness: 0.0, ..IntegratorConfig::product() }
}

/// The three ways of finding caustics, rendered for the same time in turns
/// of half a second each, so that what else the machine is doing slows
/// them alike, and measured against the reference in the whole picture and
/// in the two caustics. `TEQUILA_NOISE_SECONDS` sets the time (10 s),
/// `TEQUILA_NOISE_REFERENCE` the reference's samples over the picture
/// (8192) and `TEQUILA_NOISE_REFERENCE_BOXES` in the caustics (131072),
/// `TEQUILA_NOISE_METHODS` which of off, photons and manifold to render,
/// `TEQUILA_NOISE_CACHE` a folder to keep the reference in between runs,
/// `TEQUILA_NOISE_PICTURES` one to write the pictures to, each named after
/// its method with `TEQUILA_NOISE_LABEL` in front, and `TEQUILA_NOISE_SEED`
/// the methods' seed (7).
#[test]
#[ignore]
fn manifold_caustics_against_photons_at_equal_time() {
    let number = |name: &str, default: f32| std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default);
    let seconds = number("TEQUILA_NOISE_SECONDS", 10.0);
    let reference_samples = number("TEQUILA_NOISE_REFERENCE", 8192.0) as u32;
    let box_samples = number("TEQUILA_NOISE_REFERENCE_BOXES", 131072.0) as u32;
    let methods = std::env::var("TEQUILA_NOISE_METHODS").unwrap_or_else(|_| "off,photons,manifold".into());
    let pictures = std::env::var_os("TEQUILA_NOISE_PICTURES").map(std::path::PathBuf::from);
    let label = std::env::var("TEQUILA_NOISE_LABEL").unwrap_or_default();
    let seed = number("TEQUILA_NOISE_SEED", 7.0) as u32;
    if let Some(folder) = &pictures {
        std::fs::create_dir_all(folder).expect("a pictures folder");
    }
    let device = Device::shared().expect("an Embree device");
    let lamps = EmbreeIntersector::build(&device, studio(false)).expect("builds");
    let with = |method: CausticsMethod| IntegratorConfig { caustics: true, caustics_method: method, ..IntegratorConfig::product() };

    // The reference, in two halves of their own seeds: what is left of
    // its own noise is their distance apart over the square root of two,
    // and the halves' mean has a square root of two less again. A bounce
    // finds a lamp through the glass so seldom that the caustics take many
    // more samples than the rest, so they are rendered on their own.
    let cache = std::env::var_os("TEQUILA_NOISE_CACHE").map(|folder| {
        std::path::PathBuf::from(folder).join(format!("manifold-noise-reference-{reference_samples}-{box_samples}.bin"))
    });
    let halves = match cache.as_deref().and_then(load) {
        Some(halves) => halves,
        None => {
            let glowing = EmbreeIntersector::build(&device, studio(true)).expect("builds");
            let halves = [1u32, 2].map(|seed| {
                let mut run = Run::new(reference_config(), seed);
                run.go(&glowing, f32::INFINITY, reference_samples / 2);
                println!("reference half {seed}: {} samples in {:.0} s", reference_samples / 2, run.seconds);
                let mut image = linear(&run.framebuffer);
                for (name, (x0, y0, x1, y1)) in &BOXES[1..] {
                    let mut run = Run::new(reference_config(), seed + 2);
                    run.context.region = Some(tequila_render_core::scheduler::Tile { x0: *x0, y0: *y0, x1: *x1, y1: *y1 });
                    run.go(&glowing, f32::INFINITY, box_samples / 2);
                    println!("  its {name}: {} samples in {:.0} s", box_samples / 2, run.seconds);
                    let inside = linear(&run.framebuffer);
                    for y in *y0..*y1 {
                        for x in *x0..*x1 {
                            image[y * SIZE.0 + x] = inside[y * SIZE.0 + x];
                        }
                    }
                }
                image
            });
            if let Some(path) = &cache {
                save(path, &halves);
            }
            halves
        }
    };
    let (first, second) = (luminance(&halves[0]), luminance(&halves[1]));
    let reference: Vec<f64> = first.iter().zip(&second).map(|(a, b)| 0.5 * (a + b)).collect();
    let own: Vec<f64> = relative_rms(&first, &second).iter().map(|d| d / 2.0).collect();
    println!(
        "reference: {reference_samples} samples by path tracing glowing lamps, {box_samples} in the caustics; its own relative RMS {own:.4?}, taken out below"
    );
    // The same as shown: what the eye judges, where a highlight a thousand
    // times the floor's brightness counts for no more than white.
    let mean: Vec<Spectrum> = halves[0].iter().zip(&halves[1]).map(|(a, b)| (*a + *b) * 0.5).collect();
    let shown_reference = shown(&mean);
    let shown_own: Vec<f64> = shown_rms(&shown(&halves[0]), &shown(&halves[1])).iter().map(|d| d / 2.0).collect();
    println!("as shown (0 to 255), its own RMS {shown_own:.2?}");
    if let Some(folder) = &pictures {
        picture(folder, "reference", &mean);
    }

    let mut runs: Vec<(&str, Run)> = [
        ("off", IntegratorConfig::product()),
        ("photons", with(CausticsMethod::Photons)),
        ("manifold", with(CausticsMethod::Manifold)),
    ]
    .into_iter()
    .filter(|(name, _)| methods.split(',').any(|m| m == *name))
    .map(|(name, config)| (name, Run::new(config, seed)))
    .collect();
    while runs.iter().any(|(_, run)| run.seconds < seconds) {
        for (_, run) in &mut runs {
            if run.seconds < seconds {
                run.go(&lamps, 0.5f32.min(seconds - run.seconds), u32::MAX);
            }
        }
    }
    println!("boxes: {}", BOXES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" / "));
    for (name, run) in &runs {
        let image = linear(&run.framebuffer);
        if let Some(folder) = &pictures {
            picture(folder, &format!("{label}{name}"), &image);
        }
        let display: Vec<f64> = shown_rms(&shown(&image), &shown_reference).iter().zip(&shown_own).map(|(r, o)| (r * r - o * o).max(0.0).sqrt()).collect();
        let image = luminance(&image);
        // Each method's distance from the truth: what it is from the
        // reference, less the reference's own noise, which adds to it.
        let rms: Vec<f64> = relative_rms(&image, &reference).iter().zip(&own).map(|(r, o)| (r * r - o * o).max(0.0).sqrt()).collect();
        let (mean, reference_mean) = (means(&image), means(&reference));
        let bias: Vec<f64> = mean.iter().zip(&reference_mean).map(|(m, r)| m / r - 1.0).collect();
        println!(
            "{name}: {} samples in {:.1} s; RMS as shown {display:.2?}, relative {rms:.3?}; mean against the reference {bias:+.3?}",
            run.framebuffer.pixel(0, 0).samples, run.seconds
        );
    }
}
