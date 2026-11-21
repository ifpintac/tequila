//! The transport kernel's variants (`docs/07` R3, G1.6;
//! `tequila_render_gpu::variants`): which parts each scene's kernel keeps,
//! and what leaving the rest out does to the render's speed. That a
//! scene's variant gives the whole kernel's picture to the bit is held by
//! every scene of the parity test (`tests/parity.rs`).

use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_render_gpu::variants::KernelFeatures;
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

fn headphones() -> tequila_scene::SceneDocument {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
    tequila_format::load(&path).expect("the Headphones sample loads").document
}

/// The parts the renderer finds a scene reaches, the whole kernel built.
fn features_of(document: &tequila_scene::SceneDocument, mut cache: MeshCache) -> Option<KernelFeatures> {
    let gpu = device::for_test(Needs::RayQueries)?;
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (mut snapshot, _) = snapshot(document, environment, &mut cache);
    // What the camera sees past the parts, as the parity test sets it.
    snapshot.background = parity::background(document);
    let renderer = GpuRenderer::with_kernel(gpu, &snapshot, Some(KernelFeatures::ALL));
    assert_eq!(renderer.kernel, KernelFeatures::ALL);
    Some(renderer.features)
}

/// Each scene of the parity set, and the Headphones, keeps the parts it
/// uses and no others: the grey product none, the lamps their lamps and
/// their stand-ins' glow, glass its glass, and so on.
#[test]
fn each_scene_keeps_the_parts_it_reaches() {
    use KernelFeatures as F;
    let mut chosen = Vec::new();
    for scene in parity::SET {
        let Some(features) = features_of(&scene.document(), scene.cache()) else { return };
        eprintln!("{}: {}", scene.name, features.names());
        chosen.push((scene.name, features));
    }
    let Some(features) = features_of(&headphones(), MeshCache::new()) else { return };
    eprintln!("headphones: {}", features.names());
    chosen.push(("headphones", features));
    let of = |name: &str| chosen.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no {name}")).1;
    // Exactly what each needs: the Headphones' programs hold Point Input,
    // Math, Vector Math, Map Range, Mix, Noise, Voronoi, Mapping, Gradient
    // and Tequila's own nodes, the textured product's only pictures.
    let headphones_ops = [1, 2, 3, 4, 5, 6, 7, 9, 12, 13].into_iter().fold(F::NONE, |f, op| f | F::op(op));
    for (name, wanted) in [
        ("grey_product", F::NONE),
        ("product_materials", F::NONE),
        ("product_floor", F::FLOOR),
        ("lamp_lit", F::LAMPS | F::GLOW),
        ("glass_ball", F::GLASS | F::PANES),
        ("liquid_in_glass", F::GLASS | F::PANES),
        ("textured_product", F::PROGRAMS | F::op(24)),
        ("headphones", F::FLOOR | F::PROGRAMS | headphones_ops),
    ] {
        assert_eq!(of(name), wanted, "{name}");
    }
    // And only a scene that rounds its edges keeps the bevel.
    for (name, features) in &chosen {
        assert_eq!(features.contains(F::BEVEL), name.starts_with("bevel"), "{name}: {}", features.names());
    }
    // Only a scene with a background colour or a backplate keeps the
    // background, and only the backplate's its picture; the camera's parts
    // are a frame's, which no scene keeps (`frame_parts_are_the_cameras`).
    for (name, features) in &chosen {
        assert_eq!(features.contains(F::BACKGROUND), ["background_colour", "backplate"].contains(name), "{name}: {}", features.names());
        assert_eq!(features.contains(F::BACKPLATE), *name == "backplate", "{name}: {}", features.names());
        assert_eq!(features.contains(F::DOME), *name == "finite_dome", "{name}: {}", features.names());
        let frame_parts = F::THIN_LENS | F::BLADES | F::SHIFT | F::ORTHOGRAPHIC | F::EQUIRECTANGULAR | F::FILTER;
        assert!(F::of_frame(u32::MAX).contains(frame_parts) && !F::of_frame(u32::MAX).contains(F::DOME));
        assert!(features.without(frame_parts) == *features, "{name}: {}", features.names());
    }
    // At least what each is in the set for.
    for (name, wanted) in [
        ("bumped_product", F::PROGRAMS | F::TEXTURES | F::op(24)),
        ("cutout_grille", F::CUTOUTS | F::CUT_PROGRAMS | F::PROGRAMS),
        ("glowing_screen", F::EMITTERS | F::GLOW),
        ("glowing_volume", F::GLASS | F::VOLUME_GLOW),
        ("glow_under_glass", F::GLASS | F::PANES | F::EMITTERS | F::GLOW),
        ("bevelled_product", F::BEVEL | F::LAMPS),
        ("driven_film", F::GLASS | F::PROGRAMS),
        ("hidden_parts", F::UNSEEN | F::FLOOR | F::GLASS),
        ("shadow_links", F::SHADOW_LINKS | F::PASSES_PARTS | F::LAMPS),
        ("shadowless_parts", F::PASSES_PARTS | F::FLOOR | F::LAMPS | F::EMITTERS | F::GLOW),
        ("floor_lamps", F::FLOOR | F::LAMPS | F::SHADOW_LINKS),
        ("ies_lamps", F::IES | F::FLOOR | F::LAMPS),
    ] {
        assert!(of(name).contains(wanted), "{name} keeps {}, not all of {}", of(name).names(), wanted.names());
    }
    // Only the scenes with hidden parts, shadow links, parts that cast no
    // shadow or IES lamps keep those parts.
    for (name, features) in &chosen {
        assert_eq!(features.contains(F::UNSEEN), *name == "hidden_parts", "{name}: {}", features.names());
        assert_eq!(features.contains(F::SHADOW_LINKS), ["shadow_links", "floor_lamps"].contains(name), "{name}: {}", features.names());
        assert_eq!(features.contains(F::PASSES_PARTS), ["shadow_links", "shadowless_parts"].contains(name), "{name}: {}", features.names());
        assert_eq!(features.contains(F::IES), *name == "ies_lamps", "{name}: {}", features.names());
    }
    // And no scene that is not glass keeps glass.
    for (name, features) in &chosen {
        if !["glass", "liquid", "pane", "cut_glass", "glow_under_glass", "glowing_volume", "film", "transmission", "background", "backplate", "all_round", "dome"].iter().any(|g| name.contains(g)) {
            assert!(!features.contains(F::GLASS), "{name} keeps glass");
        }
    }
}

/// The parts a frame reaches through its camera and film, which a renderer
/// widens its kernel to: each parity scene's camera, as framed and through
/// its lens.
#[test]
fn frame_parts_are_the_cameras() {
    use KernelFeatures as F;
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("grey_product").expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snapshot, _) = snapshot(&document, environment, &mut scene.cache());
    let renderer = GpuRenderer::with_kernel(gpu, &snapshot, Some(KernelFeatures::ALL));
    for scene in parity::SET {
        let document = scene.document();
        let settings = FrameSettings { camera: scene.camera(&document), config: parity::config(&document), scene_seed: 7, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
        let parts = renderer.frame_features(&settings, scene.width, scene.height);
        let wanted = match scene.name {
            "shallow_focus" => F::THIN_LENS,
            "bladed_bokeh" => F::THIN_LENS | F::BLADES,
            "orthographic" => F::ORTHOGRAPHIC,
            "all_round" => F::EQUIRECTANGULAR,
            "shifted_lens" => F::SHIFT,
            "soft_filter" | "wide_box" => F::FILTER,
            _ => F::NONE,
        };
        eprintln!("{}: the frame reaches {}", scene.name, parts.names());
        assert_eq!(parts, wanted, "{}", scene.name);
    }
}

/// What a scene's variant does to the time a render takes, against the
/// whole kernel: the bench's scenes at its size (960 x 540), the same
/// samples on both, which give the same picture, so the time to a clean
/// picture moves as the render's time does. Interleaved, the first render
/// of each kernel a warm-up. Run by hand: `cargo test --test variants --
/// --ignored --nocapture render`; `TEQUILA_ROUNDS` rounds (5),
/// `TEQUILA_SAMPLES` samples (64), `TEQUILA_SCENES` a comma list.
#[test]
#[ignore]
fn what_the_variants_cost_to_render() {
    let rounds: usize = std::env::var("TEQUILA_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let samples: u32 = std::env::var("TEQUILA_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let names = std::env::var("TEQUILA_SCENES").unwrap_or_else(|_| "simple_product,cad_assembly,lamp_lit,headphones".to_string());
    let (width, height) = (960usize, 540usize);
    for name in names.split(',').map(str::trim) {
        let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
        let (document, generated) = match tequila_scene::bench::BenchScene::from_name(name) {
            Some(scene) => (scene.build(), true),
            None => (headphones(), false),
        };
        let mut environments = tequila_scene::environment::EnvironmentCache::new();
        let environment = environments.environment(&document.environment);
        let (mut scene, _) = snapshot(&document, environment, &mut MeshCache::new());
        scene.background = environments.background(&document.environment);
        let aspect = width as f32 / height as f32;
        let camera = if generated { tequila_scene::bench::product_camera(&document, aspect) } else { document.cameras.active().camera };
        let mut config = document.lighting.config();
        config.spectral = document.render.spectral;
        config.adaptive_sampling_threshold = 0.0;
        let (reject_fireflies, clamp_indirect, clamp_direct) =
            if generated { (true, None, None) } else { (document.render.reject_fireflies, document.render.clamp_indirect, document.render.clamp_direct) };
        let settings = FrameSettings { camera, config, scene_seed: 2, reject_fireflies, clamp_direct, clamp_indirect };
        let mut renderer = GpuRenderer::with_kernel(gpu, &scene, Some(KernelFeatures::ALL));
        let variant = renderer.features;
        let mut times: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
        for round in 0..rounds {
            let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
            for which in order {
                renderer.set_kernel(if which == 0 { KernelFeatures::ALL } else { variant });
                let mut warm = Framebuffer::new(width, height);
                renderer.render_pass(&mut warm, &settings, 4);
                let mut framebuffer = Framebuffer::new(width, height);
                let started = std::time::Instant::now();
                renderer.render_pass(&mut framebuffer, &settings, samples);
                times[which].push(started.elapsed().as_secs_f64());
            }
        }
        let median = |v: &Vec<f64>| {
            let mut v = v.clone();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let spread = |v: &Vec<f64>| v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min);
        let rate = |t: f64| (width * height) as f64 * samples as f64 / t / 1e6;
        let (full, lean) = (median(&times[0]), median(&times[1]));
        eprintln!(
            "{name} ({}), {samples} samples at {width}x{height}: whole kernel {full:.3} s ({:.1} M samples/s, spread {:.3} s), its variant {lean:.3} s ({:.1} M samples/s, spread {:.3} s): {:.3} of the whole kernel's time",
            variant.names(),
            rate(full),
            spread(&times[0]),
            rate(lean),
            spread(&times[1]),
            lean / full
        );
    }
}

/// A parity scene's snapshot, settings and size.
fn parity_shot(name: &str) -> (tequila_render_core::scene::SceneSnapshot, FrameSettings, usize, usize) {
    let scene = parity::scene(name).expect("the parity set has it");
    let document = scene.document();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (mut snapshot, _) = snapshot(&document, environment, &mut scene.cache());
    snapshot.background = parity::background(&document);
    let settings = FrameSettings {
        camera: scene.camera(&document),
        config: parity::config(&document),
        scene_seed: scene.seed,
        reject_fireflies: true,
        clamp_direct: None,
        clamp_indirect: None,
    };
    (snapshot, settings, scene.width, scene.height)
}

/// The kernels a machine has built, kept between runs (`KernelCache`): on
/// a machine that has built nothing, a scene starts with its own variant
/// and the whole kernel is built aside; the next scene, whose variant the
/// machine has not built, starts at once with the whole kernel and takes
/// its variant up once it is built - mid-render, with the picture the
/// variant alone gives; and a scene whose variant is kept starts with it.
#[test]
fn a_kept_kernel_starts_the_next_scene_at_once() {
    use tequila_render_gpu::variants::KernelCache;
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let dir = std::env::temp_dir().join(format!("tequila-kernels-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cache = KernelCache::new(&dir, &gpu.info);

    // Nothing built: the grey product's own variant, the whole kernel aside.
    let (grey, _, _, _) = parity_shot("grey_product");
    let first = GpuRenderer::with_cache(gpu, &grey, &cache);
    assert_eq!(first.kernel, KernelFeatures::NONE);
    first.wait_for_aside();
    assert!(cache.knows(KernelFeatures::NONE) && cache.knows(KernelFeatures::ALL));
    eprintln!("nothing kept: the grey product's variant in {:.2} s", first.kernel_seconds);
    drop(first);

    // The lamps' variant not built: the whole kernel at once, then theirs.
    let (lamps, settings, width, height) = parity_shot("lamp_lit");
    let gpu = device::for_test(Needs::RayQueries).expect("a card");
    let second = GpuRenderer::with_cache(gpu, &lamps, &cache);
    assert_eq!(second.kernel, KernelFeatures::ALL);
    let variant = second.features;
    assert_eq!(variant, KernelFeatures::LAMPS | KernelFeatures::GLOW);
    eprintln!("the whole kernel kept: the lamps start with it in {:.2} s", second.kernel_seconds);
    let mut switched = Framebuffer::new(width, height);
    second.render_pass(&mut switched, &settings, 16);
    second.wait_for_aside();
    assert_eq!(second.kernel_now(), variant);
    assert!(cache.knows(variant));
    second.render_pass(&mut switched, &settings, 16);
    let gpu = second.gpu;

    // Kept: the lamps' own variant at once, and its picture is the one the
    // render that switched kernels halfway gave.
    let third = GpuRenderer::with_cache(gpu, &lamps, &cache);
    assert_eq!(third.kernel, variant);
    assert_eq!(third.kernel_now(), variant);
    eprintln!("the lamps' variant kept: built in {:.2} s", third.kernel_seconds);
    let mut lean = Framebuffer::new(width, height);
    third.render_pass(&mut lean, &settings, 16);
    third.render_pass(&mut lean, &settings, 16);
    let same = switched.pixels().iter().zip(lean.pixels()).all(|(a, b)| {
        a.sum.r.to_bits() == b.sum.r.to_bits() && a.sum.g.to_bits() == b.sum.g.to_bits() && a.sum.b.to_bits() == b.sum.b.to_bits() && a.samples == b.samples
    });
    assert!(same, "the render that switched kernels is not the variant's picture");
    let _ = std::fs::remove_dir_all(&dir);
}

/// How long a first picture waits for its kernel in a fresh process, as a
/// second launch of the application would: the scene's renderer built with
/// its own variant, the whole kernel, or through a kept cache. Run by hand,
/// once per process: `TEQUILA_SCENE=lamp_lit TEQUILA_KERNEL=variant|full|kept
/// TEQUILA_KERNEL_CACHE=dir cargo test --test variants -- --ignored
/// --nocapture launch`.
#[test]
#[ignore]
fn a_launch_waits_this_long_for_its_kernel() {
    use tequila_render_gpu::variants::KernelCache;
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let name = std::env::var("TEQUILA_SCENE").unwrap_or_else(|_| "lamp_lit".to_string());
    let which = std::env::var("TEQUILA_KERNEL").unwrap_or_else(|_| "variant".to_string());
    let snapshot = if name == "headphones" {
        let document = headphones();
        let mut environments = tequila_scene::environment::EnvironmentCache::new();
        let environment = environments.environment(&document.environment);
        snapshot(&document, environment, &mut MeshCache::new()).0
    } else {
        parity_shot(&name).0
    };
    let started = std::time::Instant::now();
    let renderer = match which.as_str() {
        "full" => GpuRenderer::with_kernel(gpu, &snapshot, Some(KernelFeatures::ALL)),
        "kept" => {
            let dir = std::env::var("TEQUILA_KERNEL_CACHE").expect("TEQUILA_KERNEL_CACHE names the cache's folder");
            let cache = KernelCache::new(dir, &gpu.info);
            GpuRenderer::with_cache(gpu, &snapshot, &cache)
        }
        _ => GpuRenderer::with_kernel(gpu, &snapshot, None),
    };
    eprintln!(
        "{name}, {which}: the kernel keeping {} built in {:.3} s ({:.3} s with the upload)",
        renderer.kernel.names(),
        renderer.kernel_seconds,
        started.elapsed().as_secs_f64()
    );
    renderer.wait_for_aside();
}

/// Which of the interpreter's instructions each scene's programs hold, by
/// opcode: what gating each arm of `prog_run` would leave out. Run by hand:
/// `cargo test --test variants -- --ignored --nocapture instructions`.
#[test]
#[ignore]
fn the_instructions_each_scene_holds() {
    let mut scenes: Vec<(String, tequila_scene::SceneDocument, MeshCache)> = parity::SET.iter().map(|s| (s.name.to_string(), s.document(), s.cache())).collect();
    scenes.push(("headphones".into(), headphones(), MeshCache::new()));
    for (name, document, mut cache) in scenes {
        let mut environments = tequila_scene::environment::EnvironmentCache::new();
        let environment = environments.environment(&document.environment);
        let (snapshot, _) = snapshot(&document, environment, &mut cache);
        let lowered = tequila_render_gpu::transport::lower_programs(&snapshot);
        let table = tequila_render_gpu::program::ProgramTable::new(&lowered.iter().map(|l| l.clone().ok().flatten()).collect::<Vec<_>>());
        let mut ops = std::collections::BTreeMap::new();
        for h in &table.headers {
            for (start, end) in [(h.main_start, h.main_end), (h.bump_start, h.bump_end)] {
                let mut pc = start as usize;
                while pc < end as usize {
                    let head = table.words[pc];
                    *ops.entry(head & 0xFF).or_insert(0) += 1;
                    pc += 2 + (head >> 24) as usize;
                }
            }
        }
        eprintln!("{name}: {ops:?}");
    }
}

/// Which part a scene's variant leaves out moves its picture, if one does:
/// the scene rendered with its variant, with the whole kernel (twice, to
/// see the card repeat itself), and with its variant and each part it
/// leaves out put back, each against the whole kernel. Run by hand:
/// `TEQUILA_SCENE=liquid_in_glass cargo test --test variants -- --ignored
/// --nocapture moves`.
#[test]
#[ignore]
fn which_part_left_out_moves_a_picture() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let name = std::env::var("TEQUILA_SCENE").unwrap_or_else(|_| "liquid_in_glass".to_string());
    let samples: u32 = std::env::var("TEQUILA_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let (snapshot, settings, width, height) = parity_shot(&name);
    let mut renderer = GpuRenderer::with_kernel(gpu, &snapshot, Some(KernelFeatures::ALL));
    renderer.accumulate_on_card = false;
    let render = |renderer: &mut GpuRenderer, kernel: KernelFeatures| {
        renderer.set_kernel(kernel);
        let mut f = Framebuffer::new(width, height);
        renderer.render_pass(&mut f, &settings, samples);
        f
    };
    let whole = render(&mut renderer, KernelFeatures::ALL);
    let apart = |a: &Framebuffer, b: &Framebuffer| {
        let mut n = 0;
        let mut worst = 0.0f32;
        let mut sum = 0.0f64;
        for (p, q) in a.pixels().iter().zip(b.pixels()) {
            if p.sum.r.to_bits() != q.sum.r.to_bits() || p.sum.g.to_bits() != q.sum.g.to_bits() || p.sum.b.to_bits() != q.sum.b.to_bits() {
                n += 1;
            }
            let (p, q) = (p.mean(), q.mean());
            let d = (p.r - q.r).abs().max((p.g - q.g).abs()).max((p.b - q.b).abs());
            worst = worst.max(d);
            sum += d as f64;
        }
        format!("{n} pixels apart, mean |difference| {:.2e}, worst {:.2e}", sum / a.pixels().len() as f64, worst)
    };
    let again = render(&mut renderer, KernelFeatures::ALL);
    eprintln!("{name}: the whole kernel again: {}", apart(&again, &whole));
    let own = renderer.features;
    let variant = render(&mut renderer, own);
    eprintln!("{name}: its variant ({}): {}", own.names(), apart(&variant, &whole));
    let left_out = KernelFeatures::ALL.without(own);
    for (part, label) in KernelFeatures::NAMED {
        if left_out.contains(part) && !part.is_empty() {
            let with = render(&mut renderer, own | part);
            eprintln!("{name}: its variant and {label}: {}", apart(&with, &whole));
        }
    }
    if left_out.ops != 0 {
        let with = render(&mut renderer, own | KernelFeatures { parts: 0, ops: u32::MAX });
        eprintln!("{name}: its variant and every instruction: {}", apart(&with, &whole));
    }
}

/// Which place a part is left out at moves a picture: the scene's variant
/// with the part's constant put back (true) at one of its places at a
/// time, each against the whole kernel. Run by hand: `TEQUILA_SCENE=..
/// TEQUILA_GATE=FEATURE_EMITTERS cargo test --test variants -- --ignored
/// --nocapture places`.
#[test]
#[ignore]
fn which_place_a_part_is_left_out_at_moves_a_picture() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let name = std::env::var("TEQUILA_SCENE").unwrap_or_else(|_| "liquid_in_glass".to_string());
    let gate = std::env::var("TEQUILA_GATE").unwrap_or_else(|_| "FEATURE_EMITTERS".to_string());
    let samples: u32 = std::env::var("TEQUILA_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let (snapshot, settings, width, height) = parity_shot(&name);
    let mut renderer = GpuRenderer::with_kernel(gpu, &snapshot, Some(KernelFeatures::ALL));
    renderer.accumulate_on_card = false;
    let mut whole = Framebuffer::new(width, height);
    renderer.render_pass(&mut whole, &settings, samples);
    let own = renderer.features;
    let source = tequila_render_gpu::shader::source("transport");
    let declaration = format!("override {gate}:");
    let places: Vec<usize> = source.match_indices(&gate).map(|(i, _)| i).filter(|&i| !source[..i + gate.len() + 1].ends_with(&declaration)).collect();
    for (k, at) in places.iter().enumerate() {
        let line_start = source[..*at].rfind('\n').map_or(0, |i| i + 1);
        let line_end = source[*at..].find('\n').map_or(source.len(), |i| at + i);
        let at = *at;
        let edit = move |s: String| format!("{}true{}", &s[..at], &s[at + gate_len(&s[at..])..]);
        renderer.set_kernel_edited(own, &edit);
        let mut f = Framebuffer::new(width, height);
        renderer.render_pass(&mut f, &settings, samples);
        let apart = f.pixels().iter().zip(whole.pixels()).filter(|(p, q)| p.sum.r.to_bits() != q.sum.r.to_bits() || p.sum.g.to_bits() != q.sum.g.to_bits() || p.sum.b.to_bits() != q.sum.b.to_bits()).count();
        eprintln!("{name}: {gate} put back at place {k} ({}): {apart} pixels apart", source[line_start..line_end].trim());
    }
}

/// The length of the identifier at the start of `s`.
fn gate_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len())
}

/// What each part costs a render that does not use it: a bench scene's
/// variant with one part it leaves out put back, against the variant alone
/// and the whole kernel, the same samples. Where a part the scene never
/// runs slows it, the kernel's size is what costs (its registers), not the
/// work. Run by hand: `TEQUILA_SCENE=simple_product cargo test --test
/// variants -- --ignored --nocapture each_part_costs`.
#[test]
#[ignore]
fn what_each_part_costs_to_render() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let name = std::env::var("TEQUILA_SCENE").unwrap_or_else(|_| "simple_product".to_string());
    let samples: u32 = std::env::var("TEQUILA_SAMPLES").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let (width, height) = (960usize, 540usize);
    let scene = tequila_scene::bench::BenchScene::from_name(&name).expect("a bench scene");
    let document = scene.build();
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    let camera = tequila_scene::bench::product_camera(&document, width as f32 / height as f32);
    let mut config = document.lighting.config();
    config.adaptive_sampling_threshold = 0.0;
    let settings = FrameSettings { camera, config, scene_seed: 2, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    let mut renderer = GpuRenderer::with_kernel(gpu, &snap, Some(KernelFeatures::ALL));
    let own = renderer.features;
    let time = |renderer: &mut GpuRenderer, kernel: KernelFeatures| {
        renderer.set_kernel(kernel);
        let mut warm = Framebuffer::new(width, height);
        renderer.render_pass(&mut warm, &settings, 4);
        let mut best = f64::MAX;
        for _ in 0..3 {
            let mut f = Framebuffer::new(width, height);
            let started = std::time::Instant::now();
            renderer.render_pass(&mut f, &settings, samples);
            best = best.min(started.elapsed().as_secs_f64());
        }
        best
    };
    let lean = time(&mut renderer, own);
    let whole = time(&mut renderer, KernelFeatures::ALL);
    eprintln!("{name}: its variant ({}) {lean:.3} s, the whole kernel {whole:.3} s, for {samples} samples (the best of three)", own.names());
    let left_out = KernelFeatures::ALL.without(own);
    for (part, label) in KernelFeatures::NAMED {
        if left_out.contains(part) {
            let t = time(&mut renderer, own | part);
            eprintln!("{name}: and {label}: {t:.3} s ({:.2} of its variant's)", t / lean);
        }
    }
    let t = time(&mut renderer, own | KernelFeatures { parts: 0, ops: u32::MAX });
    eprintln!("{name}: and every instruction's arm: {t:.3} s ({:.2} of its variant's)", t / lean);
    let t = time(&mut renderer, KernelFeatures::ALL.without(KernelFeatures::CUT_PROGRAMS));
    eprintln!("{name}: the whole kernel but cut programs: {t:.3} s ({:.2} of its variant's)", t / lean);
}
