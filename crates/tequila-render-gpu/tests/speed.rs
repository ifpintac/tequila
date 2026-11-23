//! Where a scene's time goes on the card (`docs/07` R3, G1.1): diagnostics
//! run by hand, outside the render path, each against the scene's own
//! variant at the bench's size (960 x 540), its settings as the bench reads
//! them. `cargo test --test speed -- --ignored --nocapture NAME`, with
//! `TEQUILA_SCENE` a bench scene's name or `headphones` (the default) and
//! `TEQUILA_REFLECTION=0` for the Headphones with the floor's reflection
//! turned off, the bench's like-for-like row.

use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::snapshot::{snapshot, MeshCache};

const WIDTH: usize = 960;
const HEIGHT: usize = 540;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// The scene `TEQUILA_SCENE` names, as the bench sets it up, with the seed.
fn shot(seed: u32) -> (SceneSnapshot, FrameSettings, String) {
    let name = std::env::var("TEQUILA_SCENE").unwrap_or_else(|_| "headphones".to_string());
    let reflection = env_or("TEQUILA_REFLECTION", 1u32) != 0;
    let (mut document, generated) = match tequila_scene::bench::BenchScene::from_name(&name) {
        Some(scene) => (scene.build(), true),
        None => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples/Headphones.tequila");
            (tequila_format::load(&path).expect("the Headphones sample loads").document, false)
        }
    };
    if !reflection {
        document.environment.ground.reflections = 0.0;
    }
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (mut scene, _) = snapshot(&document, environment, &mut MeshCache::new());
    scene.background = environments.background(&document.environment);
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let camera = if generated { tequila_scene::bench::product_camera(&document, aspect) } else { document.cameras.active().camera };
    let mut config = document.lighting.config();
    config.spectral = document.render.spectral;
    config.adaptive_sampling_threshold = 0.0;
    let (reject_fireflies, clamp_indirect, clamp_direct) =
        if generated { (true, None, None) } else { (document.render.reject_fireflies, document.render.clamp_indirect, document.render.clamp_direct) };
    let settings = FrameSettings { camera, config, scene_seed: seed, reject_fireflies, clamp_direct, clamp_indirect };
    let label = if reflection { name } else { format!("{name}, no floor reflection") };
    (scene, settings, label)
}

/// `from` replaced by `to` in the kernel's source, which must hold it once.
fn swap(source: &str, from: &str, to: &str) -> String {
    let n = source.matches(from).count();
    assert_eq!(n, 1, "{from:?} is in the kernel {n} times");
    source.replacen(from, to, 1)
}

/// The kernel's source with its lines ended by `\n` alone, which the edits
/// below are written against; the same SPIR-V, so the driver's cache holds
/// it as the unedited kernel.
fn plain(source: String) -> String {
    source.replace("\r\n", "\n")
}

/// The stand-ins a part is replaced by: the line, and what replaces it with
/// `{G}` the guard of the part's old code. Each is measured twice: the guard
/// `false`, so the compiler drops the part (what it costs to run and to
/// carry), and a guard the compiler cannot see is false (what it costs to
/// run alone). The picture is no longer the scene's.
const STAND_INS: &[(&str, &str, &str)] = &[
    // The material's program at the one place it runs, the material left as
    // its constants.
    (
        "program",
        "let shaded = material_at(at, surf, along, ray, spread, rounded);",
        "var shaded = material; shaded.material.geometry_opacity = 1.0; if ({G}) { shaded = material_at(at, surf, along, ray, spread, rounded); }",
    ),
    // The bump's two runs of the program.
    ("bump", "let runs = select(1u, 3u, (p.flags & PROGRAM_BUMPS) != 0u);", "let runs = select(1u, 3u, (p.flags & PROGRAM_BUMPS) != 0u && ({G}));"),
    // The light sample: direct_light's call, no light added.
    ("light sample", "let direct = direct_light(&s, light);", "var direct: Direct; if ({G}) { direct = direct_light(&s, light); }"),
    // The light sample's shadow rays, every one unblocked.
    (
        "shadow rays",
        "let v = walk_ray(walk);",
        "var v: WalkEnd; v.through = vec3<f32>(1.0); v.bent = false; if ({G}) { v = walk_ray(walk); }",
    ),
    // The lamps and sky a ray meets: meet_and_escape's call, nothing met.
    (
        "meet and escape",
        "let met = meet_and_escape(&st, hit, at_i, at_step == 1u, found);",
        "var met: Paused; met.wants = false; met.carried = vec3<f32>(0.0); if ({G}) { met = meet_and_escape(&st, hit, at_i, at_step == 1u, found); }",
    ),
    // The floor's shadow where the camera sees the floor.
    ("floor shadow", "                        if (c.shadows != 0u) {", "                        if (c.shadows != 0u && ({G})) {"),
    // The Noise Texture's instruction: its first operand times its second.
    (
        "noise node",
        "out = cy_noise_node(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x, a[6].x, a[7].x, a[8].x);",
        "out = vec4<f32>(a[0].x * a[1].x); if ({G}) { out = cy_noise_node(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x, a[6].x, a[7].x, a[8].x); }",
    ),
    // The Voronoi Texture's instruction: its operands as they are.
    (
        "voronoi node",
        "let cells = cy_voronoi_node(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x, a[6].x, a[7].x, a[8].x);",
        "var cells: CyVoronoiOut; cells.a = a[0]; cells.b = a[1]; if ({G}) { cells = cy_voronoi_node(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x, a[6].x, a[7].x, a[8].x); }",
    ),
    // One Perlin noise, wherever the fractals read one.
    ("perlin", "    let v = cy_perlin(w, dims);", "    var v = w.x * 0.1; if ({G}) { v = cy_perlin(w, dims); }"),
];

/// Each stand-in as an edit, dropped and kept, by name ("program",
/// "program kept"); and the source from a file, if one is named.
fn edits() -> Vec<(String, Box<dyn Fn(String) -> String>)> {
    let mut out: Vec<(String, Box<dyn Fn(String) -> String>)> = Vec::new();
    // The kernel's whole source from the file `TEQUILA_SOURCE_FILE` names:
    // an earlier revision's, joined (shader.rs's `source`), for comparing
    // what a change costs to compile.
    if let Ok(path) = std::env::var("TEQUILA_SOURCE_FILE") {
        let text = std::fs::read_to_string(path).expect("the source is there").replace("\r\n", "\n");
        out.push(("file".to_string(), Box::new(move |_: String| text.clone())));
    }
    for &(name, from, to) in STAND_INS {
        for (suffix, guard) in [("", "false"), (" kept", "cy_zero == 7u")] {
            let to = to.replace("{G}", guard);
            out.push((format!("{name}{suffix}"), Box::new(move |s: String| swap(&plain(s), from, &to))));
        }
    }
    out
}

/// What each part of the scene's variant costs to render: the variant
/// against itself with each stand-in in turn, the same samples, interleaved
/// rounds, the first render of each a warm-up. `TEQUILA_EDITS` a comma list
/// of the edits' names (all), `TEQUILA_ROUNDS` (3), `TEQUILA_SAMPLES` (64).
#[test]
#[ignore]
fn what_the_parts_cost_to_render() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let (scene, settings, label) = shot(2);
    let rounds = env_or("TEQUILA_ROUNDS", 3usize);
    let samples = env_or("TEQUILA_SAMPLES", 64u32);
    let wanted = std::env::var("TEQUILA_EDITS").ok();
    let all = edits();
    let chosen: Vec<_> = all.iter().filter(|(name, _)| wanted.as_ref().is_none_or(|w| w.split(',').any(|n| n.trim() == name))).collect();
    let mut renderer = GpuRenderer::new(gpu, &scene);
    let variant = renderer.features;
    eprintln!("{label}: its kernel keeps {}", variant.names());
    // 0 the variant itself, then each edit; and each one's first picture,
    // to say whether an edit changed it.
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); chosen.len() + 1];
    let mut pictures: Vec<Option<Vec<[f32; 3]>>> = vec![None; chosen.len() + 1];
    for round in 0..rounds {
        let mut order: Vec<usize> = (0..=chosen.len()).collect();
        let len = order.len();
        order.rotate_left(round % len);
        for which in order {
            if which == 0 {
                renderer.set_kernel_edited(variant, &plain);
            } else {
                renderer.set_kernel_edited(variant, &*chosen[which - 1].1);
            }
            let mut warm = Framebuffer::new(WIDTH, HEIGHT);
            renderer.render_pass(&mut warm, &settings, 4);
            let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
            let started = std::time::Instant::now();
            renderer.render_pass(&mut framebuffer, &settings, samples);
            times[which].push(started.elapsed().as_secs_f64());
            if pictures[which].is_none() {
                pictures[which] = Some(framebuffer.pixels().iter().map(|p| [p.sum.r, p.sum.g, p.sum.b]).collect());
            }
        }
    }
    let apart = |which: usize| {
        let (a, b) = (pictures[0].as_ref().unwrap(), pictures[which].as_ref().unwrap());
        let differ = a.iter().zip(b).filter(|(x, y)| x.iter().zip(y.iter()).any(|(u, v)| u.to_bits() != v.to_bits())).count();
        let worst = a.iter().zip(b).flat_map(|(x, y)| x.iter().zip(y.iter()).map(|(u, v)| (u - v).abs() / samples as f32)).fold(0.0f32, f32::max);
        format!("{differ} pixels apart from the variant's picture (worst {worst:.2e})")
    };
    let rate = |t: f64| (WIDTH * HEIGHT) as f64 * samples as f64 / t / 1e6;
    let base = median(&times[0]);
    eprintln!("{label}, {samples} samples: the variant {base:.3} s ({:.1} M samples/s), {:?}", rate(base), times[0]);
    for (i, (name, _)) in chosen.iter().enumerate() {
        let t = median(&times[i + 1]);
        eprintln!(
            "  {name}: {t:.3} s ({:.1} M samples/s), {:.1}% of the variant's time saved, {:?}; {}",
            rate(t),
            100.0 * (1.0 - t / base),
            times[i + 1],
            apart(i + 1)
        );
    }
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The counters a sample of the counting kernel hands back, in its six
/// channels: the segments its paths took (a turn of path()'s outer loop),
/// the turns of the segment's loop, the walks made at path()'s one place,
/// the light sample's shadow rays, the light samples, and the material
/// programs run.
const COUNTERS: [&str; 6] = ["segments", "turns", "walks", "shadow rays", "light samples", "programs"];

/// The kernel with each sample's counters as its radiance and background.
fn counting(source: String) -> String {
    let s = plain(source);
    let s = swap(&s, "fn path(x: u32, y: u32, sample_index: u32) -> PathOut {\n", "var<private> dbg: array<u32, 6>;\nfn path(x: u32, y: u32, sample_index: u32) -> PathOut {\n    dbg = array<u32, 6>(0u, 0u, 0u, 0u, 0u, 0u);\n");
    let s = swap(&s, "        var ended = true;\n", "        dbg[0] += 1u;\n        var ended = true;\n");
    let s = swap(&s, "            var found: WalkEnd;\n", "            var found: WalkEnd;\n            dbg[1] += 1u;\n");
    let s = swap(&s, "                found = walk_ray(w);\n", "                found = walk_ray(w);\n                dbg[2] += 1u;\n");
    let s = swap(
        &s,
        "let v = walk_ray(walk);",
        "let v = walk_ray(walk); dbg[3] += 1u;",
    );
    let s = swap(&s, "let direct = direct_light(&s, light);", "let direct = direct_light(&s, light); dbg[4] += 1u;");
    let s = swap(&s, "let shaded = material_at(at, surf, along, ray, spread, rounded);", "let shaded = material_at(at, surf, along, ray, spread, rounded); dbg[5] += 1u;");
    swap(
        &s,
        "    return out;\n}\n\n@compute",
        "    out.radiance = vec3<f32>(f32(dbg[0]), f32(dbg[1]), f32(dbg[2]));\n    out.background = vec3<f32>(f32(dbg[3]), f32(dbg[4]), f32(dbg[5]));\n    out.coverage = 1.0;\n    return out;\n}\n\n@compute",
    )
}

/// How the scene's paths run on the card: each counter's mean a sample, and
/// how evenly a warp's 32 threads (8 x 4 pixels of a workgroup's 8 x 8)
/// share the segment loop's turns - the share of the warp's turns its
/// threads are busy, with the samples each thread takes in turn as now (a
/// warp waits at each sample for its longest path) and as they would be
/// were a thread to start its next sample as soon as its path ends, over a
/// dispatch of 4 or 16 samples. `TEQUILA_SAMPLES` samples (16).
#[test]
#[ignore]
fn how_the_paths_run() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let (scene, mut settings, label) = shot(2);
    settings.reject_fireflies = false;
    settings.clamp_direct = None;
    settings.clamp_indirect = None;
    let samples = env_or("TEQUILA_SAMPLES", 16usize);
    let mut renderer = GpuRenderer::new(gpu, &scene);
    renderer.accumulate_on_card = false;
    let variant = renderer.features;
    renderer.set_kernel_edited(variant, &counting);
    let pixels = WIDTH * HEIGHT;
    // counts[k][sample][pixel]
    let mut counts = vec![vec![vec![0u32; pixels]; samples]; COUNTERS.len()];
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
    let mut before = vec![[0f32; 6]; pixels];
    for i in 0..samples {
        renderer.render_pass(&mut framebuffer, &settings, 1);
        for (p, pixel) in framebuffer.pixels().iter().enumerate() {
            let now = [pixel.sum.r, pixel.sum.g, pixel.sum.b, pixel.background_sum.r, pixel.background_sum.g, pixel.background_sum.b];
            for k in 0..6 {
                counts[k][i][p] = (now[k] - before[p][k]).round() as u32;
            }
            before[p] = now;
        }
    }
    eprintln!("{label}, {samples} samples, its kernel {}:", variant.names());
    let n = (samples * pixels) as f64;
    for (k, name) in COUNTERS.iter().enumerate() {
        let total: u64 = counts[k].iter().flatten().map(|&c| c as u64).sum();
        let most = counts[k].iter().flatten().copied().max().unwrap_or(0);
        eprintln!("  {name}: {:.3} a sample, at most {most}", total as f64 / n);
    }
    // Histogram of turns a sample.
    let turns = &counts[1];
    let mut histogram = std::collections::BTreeMap::new();
    for &t in turns.iter().flatten() {
        *histogram.entry(t.min(40)).or_insert(0u64) += 1;
    }
    eprintln!("  turns a sample (40 = 40 or more): {:?}", histogram.iter().map(|(t, c)| format!("{t}:{:.4}", *c as f64 / n)).collect::<Vec<_>>().join(" "));
    // Warps: 8 x 4 pixels, within each 8 x 8 workgroup.
    let warps: Vec<Vec<usize>> = (0..HEIGHT.div_ceil(4))
        .flat_map(|wy| (0..WIDTH / 8).map(move |wx| (wx, wy)))
        .map(|(wx, wy)| (0..4).flat_map(|dy| (0..8).map(move |dx| (wx * 8 + dx, wy * 4 + dy))).filter(|&(_, y)| y < HEIGHT).map(|(x, y)| y * WIDTH + x).collect())
        .collect();
    for group in [1usize, 4, 16] {
        if group > samples {
            continue;
        }
        let (mut busy, mut held) = (0u64, 0u64);
        let (mut busy_merged, mut held_merged) = (0u64, 0u64);
        for warp in &warps {
            for start in (0..samples - samples % group).step_by(group) {
                // As now: each sample, the warp held for its longest path.
                for i in start..start + group {
                    let most = warp.iter().map(|&p| turns[i][p]).max().unwrap_or(0) as u64;
                    busy += warp.iter().map(|&p| turns[i][p] as u64).sum::<u64>();
                    held += most * warp.len() as u64;
                }
                // Merged: each thread's samples one after another.
                let sums: Vec<u64> = warp.iter().map(|&p| (start..start + group).map(|i| turns[i][p] as u64).sum()).collect();
                busy_merged += sums.iter().sum::<u64>();
                held_merged += sums.iter().copied().max().unwrap_or(0) * warp.len() as u64;
            }
        }
        eprintln!(
            "  {group} samples a dispatch: threads busy {:.1}% of their warp's turns as now; {:.1}% were each to start its next sample at once",
            100.0 * busy as f64 / held as f64,
            100.0 * busy_merged as f64 / held_merged as f64
        );
    }
}

/// What the camera meets first at each pixel, one sample a pixel: the
/// instance (-1 for nothing) written to `TEQUILA_IDS` as little-endian i32s,
/// row by row, and each instance's material printed, for reading a noise
/// map by part.
#[test]
#[ignore]
fn what_the_camera_meets() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let (scene, mut settings, label) = shot(2);
    settings.reject_fireflies = false;
    settings.clamp_direct = None;
    settings.clamp_indirect = None;
    let mut renderer = GpuRenderer::new(gpu, &scene);
    renderer.accumulate_on_card = false;
    let variant = renderer.features;
    renderer.set_kernel_edited(variant, &|source: String| {
        let s = plain(source);
        let s = swap(&s, "fn path(x: u32, y: u32, sample_index: u32) -> PathOut {\n", "var<private> dbg_first: f32;\nfn path(x: u32, y: u32, sample_index: u32) -> PathOut {\n    dbg_first = -2.0;\n");
        let s = swap(
            &s,
            "            if (phase == PH_TRACED) {\n                let hit = seg_hit;\n",
            "            if (phase == PH_TRACED) {\n                let hit = seg_hit;\n                if (dbg_first == -2.0) { if (hit.kind == RAY_QUERY_INTERSECTION_NONE) { dbg_first = -1.0; } else { dbg_first = f32(scene_instances[hit.instance_custom_data].instance); } }\n",
        );
        swap(&s, "    return out;\n}\n\n@compute", "    out.radiance = vec3<f32>(dbg_first + 2.0, 0.0, 0.0);\n    out.background = vec3<f32>(0.0);\n    out.coverage = 1.0;\n    return out;\n}\n\n@compute")
    });
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
    renderer.render_pass(&mut framebuffer, &settings, 1);
    let ids: Vec<i32> = framebuffer.pixels().iter().map(|p| p.sum.r.round() as i32 - 2).collect();
    if let Ok(path) = std::env::var("TEQUILA_IDS") {
        std::fs::write(&path, ids.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).expect("the ids are written");
    }
    eprintln!("{label}: {} instances", scene.instances.len());
    let mut seen = std::collections::BTreeMap::new();
    for &id in &ids {
        *seen.entry(id).or_insert(0usize) += 1;
    }
    for (id, n) in seen {
        if id < 0 {
            eprintln!("  nothing: {n} pixels");
            continue;
        }
        let instance = &scene.instances[id as usize];
        let m = &scene.materials[instance.material.0 as usize];
        eprintln!(
            "  {id}: {n} pixels, material {} (program {}), base {:.3} {:.3} {:.3} metal {:.2} diffuse rough {:.2}, specular {:.2} rough {:.3} ior {:.2}, coat {:.2}, fuzz {:.2}",
            instance.material.0,
            scene.programs.get(instance.material.0 as usize).is_some_and(|p| p.is_some()),
            m.base_color.r,
            m.base_color.g,
            m.base_color.b,
            m.base_metalness,
            m.base_diffuse_roughness,
            m.specular_weight,
            m.specular_roughness,
            m.specular_ior,
            m.coat_weight,
            m.fuzz_weight
        );
    }
}

/// What the edits `TEQUILA_EDITS` names cost the cold compile, against the
/// kernel as it is: the whole kernel, and the scene's variant with
/// `TEQUILA_VARIANT=1`; interleaved rounds (`TEQUILA_ROUNDS`, 3), each build
/// salted so nothing is cached.
#[test]
#[ignore]
fn what_the_edits_cost_to_compile() {
    use tequila_render_gpu::transport::cold_compile_time_edited_of;
    use tequila_render_gpu::variants::KernelFeatures;
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let (scene, _, label) = shot(2);
    let rounds = env_or("TEQUILA_ROUNDS", 3usize);
    let wanted = std::env::var("TEQUILA_EDITS").ok();
    let all = edits();
    let chosen: Vec<_> = all.iter().filter(|(name, _)| wanted.as_ref().is_none_or(|w| w.split(',').any(|n| n.trim() == name))).collect();
    let renderer = GpuRenderer::new(gpu, &scene);
    let kernel = if env_or("TEQUILA_VARIANT", 0u32) != 0 { renderer.features } else { KernelFeatures::ALL };
    eprintln!("{label}: cold compiles of the kernel keeping {}", kernel.names());
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32);
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); chosen.len() + 1];
    let mut n = 0u32;
    for round in 0..rounds {
        let mut order: Vec<usize> = (0..=chosen.len()).collect();
        let len = order.len();
        order.rotate_left(round % len);
        for which in order {
            n += 1;
            let t = if which == 0 {
                cold_compile_time_edited_of(&renderer, salt.wrapping_add(n), &plain, kernel)
            } else {
                cold_compile_time_edited_of(&renderer, salt.wrapping_add(n), &*chosen[which - 1].1, kernel)
            };
            times[which].push(t.as_secs_f64());
            eprintln!("  round {round}: {} {:.1} s", if which == 0 { "as it is" } else { &chosen[which - 1].0 }, t.as_secs_f64());
        }
    }
    eprintln!("as it is: {:.1} s {:?}", median(&times[0]), times[0]);
    for (i, (name, _)) in chosen.iter().enumerate() {
        eprintln!("  {name}: {:.1} s {:?}", median(&times[i + 1]), times[i + 1]);
    }
}

/// The scene's variant built through a kernel cache in `TEQUILA_KEPT`, so
/// the driver's compiled pipeline (Vulkan's pipeline cache) is kept there
/// as a file, for reading what the driver made of it (its registers).
#[test]
#[ignore]
fn the_variant_kept() {
    use tequila_render_gpu::variants::KernelCache;
    let Ok(dir) = std::env::var("TEQUILA_KEPT") else { return };
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let (scene, _, label) = shot(2);
    let cache = KernelCache::new(&dir, &gpu.info);
    let renderer = GpuRenderer::with_cache(gpu, &scene, &cache);
    renderer.wait_for_aside();
    eprintln!("{label}: kernel {} kept in {dir}", renderer.kernel_now().names());
}
