//! The camera on the card (`docs/07` R3): what a camera ray that escapes
//! sees past the parts - a backplate read at the film point
//! (`environment.wgsl`'s `plate_bilinear`) against the processor's
//! `Background::radiance` - on the renderer's own uploaded scene, through
//! `GpuRenderer::run_on_scene`.
//!
//! Each holds the card to the processor's own function on the same inputs,
//! to 1e-4 relative (1e-6 absolute near nought), at most one item in a
//! thousand outside.

use tequila_core::math::vec2;
use tequila_core::rng::Pcg32;
use tequila_render_core::environment::Background;
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

/// A parity scene's renderer, its snapshot's background set as the parity
/// test sets it, and the settings it renders with.
fn renderer_for(name: &str) -> Option<(GpuRenderer, FrameSettings, tequila_render_core::scene::SceneSnapshot)> {
    let gpu = device::for_test(Needs::RayQueries)?;
    let scene = parity::scene(name).expect("the parity set has it");
    let document = scene.document();
    let camera = scene.camera(&document);
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (mut snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    snap.background = parity::background(&document);
    let renderer = GpuRenderer::new(gpu, &snap);
    let settings = FrameSettings { camera, config: document.lighting.config(), scene_seed: scene.seed, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    Some((renderer, settings, snap))
}

/// Whether `a` is `b` to 1e-4 relative, 1e-6 absolute near nought.
fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-6_f32.max(1e-4 * b.abs())
}

const PLATE_KERNEL: &str = r#"
@compute @workgroup_size(64)
fn plate_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    let film = frame_samples[k].xy;
    frame_samples[k] = vec4<f32>(plate_bilinear(film), 0.0);
}
"#;

/// The backplate's picture at a film point, inside the frame, on its edges
/// and corners, and outside it (clamped): the processor's bilinear lookup.
#[test]
fn the_backplate_is_read_as_the_processor_reads_it() {
    let Some((renderer, settings, snap)) = renderer_for("backplate") else { return };
    let Background::Backplate { image } = &snap.background else { panic!("the scene has a backplate") };
    let mut rng = Pcg32::new(17, 3);
    let mut films = Vec::new();
    for edge in [0.0f32, 1.0, 0.5] {
        films.push(vec2(edge, edge));
        films.push(vec2(edge, 1.0 - edge));
    }
    while films.len() < 20_000 {
        // A sixth outside the frame, where the processor clamps.
        films.push(vec2(rng.next_f32() * 1.2 - 0.1, rng.next_f32() * 1.2 - 0.1));
    }
    let count = films.len();
    let words: Vec<u32> = films.iter().flat_map(|f| [f.x.to_bits(), f.y.to_bits(), 0, 0]).collect();
    let kernel = PLATE_KERNEL.replace("COUNTu", &format!("{count}u"));
    let out = renderer.run_on_scene(&settings, &kernel, "plate_test", &words, (count as u32).div_ceil(64));
    let mut outside = 0;
    let mut worst = 0.0f32;
    for (i, film) in films.iter().enumerate() {
        let want = image.sample_bilinear_wrap_u(film.x.clamp(0.0, 1.0), film.y.clamp(0.0, 1.0));
        let got = [f32::from_bits(out[4 * i]), f32::from_bits(out[4 * i + 1]), f32::from_bits(out[4 * i + 2])];
        let want = [want.x, want.y, want.z];
        for c in 0..3 {
            worst = worst.max((got[c] - want[c]).abs());
        }
        if (0..3).any(|c| !close(got[c], want[c])) {
            if outside < 5 {
                eprintln!("film {film:?}: card {got:?}, processor {want:?}");
            }
            outside += 1;
        }
    }
    eprintln!("backplate: {outside} of {count} lookups outside the bound, worst {worst:.2e}");
    assert!(outside * 1000 <= count, "{outside} of {count} backplate lookups are not the processor's");
}

const CAMERA_KERNEL: &str = r#"
@compute @workgroup_size(64)
fn camera_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    // The film point, and a zero the compiler cannot see is one (cy_exact's,
    // as a program sets it); the sampler's pixel, sample and seed.
    let a = frame_samples[2u * k];
    let d = bitcast<vec4<u32>>(frame_samples[2u * k + 1u]);
    cy_zero = bitcast<u32>(a.z) & 0x80000000u;
    var s = sampler_new(d.x, d.y, d.z, d.w);
    let ray = camera_ray(a.xy, &s);
    frame_samples[2u * k] = vec4<f32>(ray.origin, sampler_next_unstructured(&s));
    frame_samples[2u * k + 1u] = vec4<f32>(ray.direction, 0.0);
}
"#;

/// `camera`'s rays on the card at `count` film points of a `width` by
/// `height` frame, each with a sampler of its own, against
/// `Camera::generate_ray`: how many are outside the bound, and the worst
/// difference.
fn rays_against_the_processor(renderer: &GpuRenderer, settings: &FrameSettings, camera: tequila_render_core::camera::Camera, (width, height): (usize, usize), count: usize, seed: u64) -> (usize, f32) {
    use tequila_core::sampler::Sampler;
    let settings = FrameSettings { camera, ..settings.clone() };
    let aspect = width as f32 / height as f32;
    let mut rng = Pcg32::new(seed, 11);
    let mut items = Vec::with_capacity(count);
    for i in 0..count {
        // The frame's corners and middle first; then anywhere on it, and a
        // little past it, where a wide filter places samples.
        let film = match i {
            0 => vec2(0.0, 0.0),
            1 => vec2(1.0, 1.0),
            2 => vec2(0.5, 0.5),
            3 => vec2(1.0, 0.0),
            _ => vec2(rng.next_f32() * 1.1 - 0.05, rng.next_f32() * 1.1 - 0.05),
        };
        let pixel = [rng.next_u32() % 4096, rng.next_u32() % 4096, rng.next_u32() % 65536, rng.next_u32()];
        items.push((film, pixel));
    }
    let words: Vec<u32> = items.iter().flat_map(|(f, p)| [f.x.to_bits(), f.y.to_bits(), 0, 0, p[0], p[1], p[2], p[3]]).collect();
    let kernel = CAMERA_KERNEL.replace("COUNTu", &format!("{count}u"));
    let out = renderer.run_on_scene_at(&settings, (width, height), &kernel, "camera_test", &words, (count as u32).div_ceil(64));
    let mut outside = 0;
    let mut worst = 0.0f32;
    for (i, (film, p)) in items.iter().enumerate() {
        let mut sampler = Sampler::new(p[0], p[1], p[2], p[3]);
        let ray = camera.generate_ray(*film, aspect, &mut sampler);
        let next = sampler.next_unstructured();
        let got = |k: usize| f32::from_bits(out[8 * i + k]);
        let card = [got(0), got(1), got(2), got(4), got(5), got(6)];
        let processor = [ray.origin.x, ray.origin.y, ray.origin.z, ray.direction.x, ray.direction.y, ray.direction.z];
        for c in 0..6 {
            worst = worst.max((card[c] - processor[c]).abs());
        }
        // The sampler left where the processor's is: the camera draws only
        // from its own dimensions, none from the open stream.
        let same_stream = got(3) == next;
        if !same_stream || (0..6).any(|c| !close(card[c], processor[c])) {
            if outside < 5 {
                eprintln!("film {film:?}, sampler {p:?}: card {card:?}, processor {processor:?}, stream {same_stream}");
            }
            outside += 1;
        }
    }
    (outside, worst)
}

/// The camera's rays: through the pinhole, and through a thin lens round
/// and bladed - three to eight blades, turned, wide open and stopped down,
/// focused near and far - shifted, with and without a lens; orthographic;
/// every way round; at two shapes of frame, each against
/// `Camera::generate_ray` with the same sampler.
#[test]
fn camera_rays_are_the_processors() {
    use tequila_render_core::camera::{Aperture, Camera, Projection};
    let Some((renderer, settings, _)) = renderer_for("shallow_focus") else { return };
    let framed = settings.camera;
    let cameras: Vec<(&str, Camera)> = vec![
        ("pinhole", Camera { f_number: f32::INFINITY, ..framed }),
        ("thin lens f/1.4", framed),
        ("thin lens f/8 focused far", Camera { f_number: 8.0, focus_distance: 25.0, ..framed }),
        ("thin lens f/0.95 focused near", Camera { f_number: 0.95, focus_distance: 0.05, ..framed }),
        ("three blades", Camera { aperture: Aperture::Bladed { blades: 3, rotation: 0.0 }, ..framed }),
        ("five blades turned", Camera { aperture: Aperture::Bladed { blades: 5, rotation: 0.7 }, ..framed }),
        ("six blades turned past a turn", Camera { aperture: Aperture::Bladed { blades: 6, rotation: 7.0 }, ..framed }),
        ("eight blades turned back", Camera { f_number: 2.0, aperture: Aperture::Bladed { blades: 8, rotation: -1.3 }, ..framed }),
        ("shifted", Camera { f_number: f32::INFINITY, projection: Projection::Shift { focal_length_mm: 35.0, shift_x: 0.1, shift_y: -0.2 }, ..framed }),
        ("shifted through a lens of five blades", Camera { projection: Projection::Shift { focal_length_mm: 85.0, shift_x: -0.3, shift_y: 0.05 }, aperture: Aperture::Bladed { blades: 5, rotation: 0.4 }, ..framed }),
        ("shifted, unshifted", Camera { f_number: f32::INFINITY, projection: Projection::Shift { focal_length_mm: 35.0, shift_x: 0.0, shift_y: 0.0 }, ..framed }),
        ("orthographic", Camera { projection: Projection::Orthographic { scale: 1.3 }, ..framed }),
        ("orthographic and tiny", Camera { projection: Projection::Orthographic { scale: 0.002 }, ..framed }),
        ("every way round", Camera { projection: Projection::Equirectangular, ..framed }),
    ];
    let mut failed = Vec::new();
    for (k, (name, camera)) in cameras.into_iter().enumerate() {
        for (j, size) in [(320usize, 180usize), (200, 300)].into_iter().enumerate() {
            let count = 20_000;
            let (outside, worst) = rays_against_the_processor(&renderer, &settings, camera, size, count, (k * 2 + j) as u64 + 1);
            eprintln!("{name} at {}x{}: {outside} of {count} rays outside the bound, worst {worst:.2e}", size.0, size.1);
            if outside * 1000 > count {
                failed.push(format!("{name} at {}x{}: {outside}", size.0, size.1));
            }
        }
    }
    assert!(failed.is_empty(), "rays not the processor's: {failed:?}");
}
