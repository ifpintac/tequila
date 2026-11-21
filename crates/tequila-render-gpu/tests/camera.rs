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
