//! Rounded edges on the card (`docs/07` R3 tier B): `transport.wgsl`'s
//! `bevel_normal` against the processor's `bevel::rounded_normal`, at hits
//! the processor finds on `bevelled_product`'s sharp parts - the camera's,
//! and rays from every side at the parts' edges - each with a radius and a
//! count of probes of its own and the processor's sampler, on the scene's
//! own acceleration structure.
//!
//! The two draw the same numbers in the same order, cast the same probes
//! and weigh what they meet alike: they part only where a ray query and
//! Embree round a probe's hit on a triangle's edge, or its reach, apart,
//! or where a hit's normal lies on the line between leaning away and not.

use std::collections::HashMap;
use tequila_core::math::{vec2, vec3, Ray, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::sampler::Sampler;
use tequila_render_core::intersect::Intersector;
use tequila_render_cpu::{Device, EmbreeIntersector};
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

/// One item of the kernel: four vectors in, the rounded normal and the
/// sampler's next draw out, over the first.
const KERNEL: &str = r#"
@compute @workgroup_size(64)
fn bevel_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    // The part's slot, the probes, the radius; the point; the normal; the
    // sampler's pixel, sample and seed.
    let a = bitcast<vec4<u32>>(frame_samples[4u * k]);
    let p = frame_samples[4u * k + 1u].xyz;
    let n = frame_samples[4u * k + 2u].xyz;
    let d = bitcast<vec4<u32>>(frame_samples[4u * k + 3u]);
    var s = sampler_new(d.x, d.y, d.z, d.w);
    let rounded = bevel_normal(a.x, p, n, bitcast<f32>(a.z), a.y, &s);
    let next = sampler_next_unstructured(&s);
    frame_samples[4u * k] = vec4<f32>(rounded, next);
}
"#;

#[test]
fn the_rounded_normal_is_the_processors() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("bevelled_product").expect("the parity set has it");
    let document = scene.document();
    let camera = scene.camera(&document);
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snap);
    let slots: HashMap<u32, u32> = renderer.scene.instance_records.iter().enumerate().map(|(slot, r)| (r.instance, slot as u32)).collect();
    let settings = FrameSettings { camera, config: document.lighting.config(), scene_seed: 7, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snap).expect("builds");
    let scene_ref = intersector.scene();
    // The parts, not the ground: their bounds, which the rays from every
    // side aim inside.
    let parts: Vec<_> = scene_ref
        .instances
        .iter()
        .filter(|i| scene_ref.material(i.material).edge_radius > 0.0)
        .map(|i| scene_ref.meshes[i.mesh.index()].bounds().transformed(&i.transform))
        .collect();
    assert!(parts.len() >= 5, "the parts round their edges");

    struct Item {
        slot: u32,
        hit: tequila_render_core::intersect::Hit,
        normal: Vec3,
        radius: f32,
        samples: u32,
        pixel: [u32; 4],
    }
    let mut rng = Pcg32::new(91, 7);
    let mut items: Vec<Item> = Vec::new();
    let aspect = scene.width as f32 / scene.height as f32;
    while items.len() < 20_000 {
        let ray = if items.len() % 4 == 0 {
            let mut unused = Sampler::new(0, 0, 0, 0);
            camera.generate_ray(vec2(rng.next_f32(), rng.next_f32()), aspect, &mut unused)
        } else {
            // From a point well outside a part towards a point near one of
            // its box's edges, where the rounding shows.
            let b = &parts[(rng.next_u32() as usize) % parts.len()];
            let (lo, hi) = (b.min, b.max);
            let mut target = vec3(lo.x + (hi.x - lo.x) * rng.next_f32(), lo.y + (hi.y - lo.y) * rng.next_f32(), lo.z + (hi.z - lo.z) * rng.next_f32());
            // On two of the box's faces at once: one of its edges.
            let free = (rng.next_u32() % 3) as usize;
            for axis in 0..3 {
                if axis != free {
                    let (l, h) = [(lo.x, hi.x), (lo.y, hi.y), (lo.z, hi.z)][axis];
                    let v = if rng.next_f32() < 0.5 { l } else { h };
                    match axis {
                        0 => target.x = v,
                        1 => target.y = v,
                        _ => target.z = v,
                    }
                }
            }
            // From above the ground, which hides everything below it.
            let z = 0.05 + 0.95 * rng.next_f32();
            let phi = std::f32::consts::TAU * rng.next_f32();
            let r = (1.0 - z * z).max(0.0).sqrt();
            let from = vec3(r * phi.cos(), z, r * phi.sin());
            Ray::new(target + from * 2.0, -from)
        };
        let Some(hit) = intersector.intersect(&ray) else { continue };
        let material = scene_ref.material(hit.material);
        let (geometric, shading) = hit.facing_forward(ray.direction);
        let normal = if shading.dot(geometric) <= 0.0 { geometric } else { shading };
        // The part's own radius and probes half the time; else any.
        let (radius, samples) = if material.edge_radius > 0.0 && rng.next_f32() < 0.5 {
            (material.edge_radius, material.edge_samples as u32)
        } else {
            (0.002 + 0.04 * rng.next_f32(), 1 + rng.next_u32() % 24)
        };
        let pixel = [rng.next_u32() % 4096, rng.next_u32() % 4096, rng.next_u32() % 1024, rng.next_u32()];
        items.push(Item { slot: slots[&hit.instance.0], hit, normal, radius, samples, pixel });
    }

    let mut words: Vec<u32> = Vec::with_capacity(items.len() * 16);
    for item in &items {
        let p = item.hit.position;
        let n = item.normal;
        words.extend_from_slice(&[item.slot, item.samples, item.radius.to_bits(), 0]);
        words.extend_from_slice(&[p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), 0]);
        words.extend_from_slice(&[n.x.to_bits(), n.y.to_bits(), n.z.to_bits(), 0]);
        words.extend_from_slice(&item.pixel);
    }
    let kernel = KERNEL.replace("COUNT", &items.len().to_string());
    let card = renderer.run_on_scene(&settings, &kernel, "bevel_test", &words, (items.len() as u32).div_ceil(64));

    let (mut outside, mut moved, mut draws_apart, mut worst) = (0usize, 0usize, 0usize, 0.0f32);
    let mut cases = Vec::new();
    for (k, item) in items.iter().enumerate() {
        let mut sampler = Sampler::new(item.pixel[0], item.pixel[1], item.pixel[2], item.pixel[3]);
        let want = tequila_render_core::bevel::rounded_normal(&intersector, &item.hit, item.normal, item.radius, item.samples, &mut sampler);
        let next = sampler.next_unstructured();
        let got = vec3(f32::from_bits(card[16 * k]), f32::from_bits(card[16 * k + 1]), f32::from_bits(card[16 * k + 2]));
        if f32::from_bits(card[16 * k + 3]).to_bits() != next.to_bits() {
            draws_apart += 1;
        }
        if (want - item.normal).length() > 1e-3 {
            moved += 1;
        }
        let d = (got - want).abs().max_component();
        worst = worst.max(d);
        if !(d <= 1e-4) {
            outside += 1;
            if cases.len() < 8 {
                cases.push(format!("{k}: radius {} x{} at {:?}: card {got:?}, processor {want:?}", item.radius, item.samples, item.hit.position));
            }
        }
    }
    eprintln!(
        "rounded normals: {} hits, {moved} rounded away from their own normal; {outside} outside 1e-4 (worst {worst:.2e}); {draws_apart} left the sampler elsewhere",
        items.len()
    );
    for case in &cases {
        eprintln!("  {case}");
    }
    assert_eq!(draws_apart, 0, "the card draws as many numbers as the processor");
    assert!(moved * 6 >= items.len(), "a good share of the hits should be near enough an edge to round: {moved} of {}", items.len());
    assert!(outside * 1000 <= items.len(), "{outside} of {} rounded normals are outside 1e-4", items.len());
}

/// What rounding shows: `bevelled_product` on the card with its edges sharp
/// and rounded, at `TEQUILA_BEVEL_SCALE` (2 by default) times the parity
/// set's size, as PNGs in the folder `TEQUILA_BEVEL_PNG` names, exposed
/// alike. Run by hand, `TEQUILA_BEVEL_PNG=folder cargo test --test bevel --
/// --ignored`. At 3 times, a dispatch of 16 samples of this scene runs past
/// Windows' two seconds on the RTX 2060, sharp or rounded, and the device
/// is lost.
#[test]
#[ignore]
fn sharp_and_rounded_on_the_card() {
    let Ok(folder) = std::env::var("TEQUILA_BEVEL_PNG") else { return };
    let scene = parity::scene("bevelled_product").expect("the parity set has it");
    let scale: usize = std::env::var("TEQUILA_BEVEL_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let (width, height) = (scene.width * scale, scene.height * scale);
    let mut white = None;
    for rounded in [false, true] {
        let document = parity::bevelled_product_rounded(rounded);
        // The same aspect as the set's, so the same framing.
        let camera = scene.camera(&document);
        let mut environments = tequila_scene::environment::EnvironmentCache::new();
        let environment = environments.environment(&document.environment);
        let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
        let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
        let renderer = GpuRenderer::new(gpu, &snap);
        let settings = FrameSettings { camera, config: document.lighting.config(), scene_seed: scene.seed, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
        let mut film = tequila_render_core::framebuffer::Framebuffer::new(width, height);
        for _ in 0..8 {
            renderer.render_pass(&mut film, &settings, 64);
        }
        let mean = film.pixels().iter().map(|p| p.mean().max_component() as f64).sum::<f64>() / film.pixels().len() as f64;
        eprintln!("{}: {} samples a pixel, mean {mean:.4}", if rounded { "rounded" } else { "sharp" }, film.pixels()[0].samples);
        let white = *white.get_or_insert_with(|| {
            let mut levels: Vec<f32> = film.pixels().iter().map(|p| p.mean().max_component()).collect();
            levels.sort_by(f32::total_cmp);
            levels[levels.len() * 99 / 100].max(1e-6)
        });
        let byte = |v: f32| ((v / white).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8;
        let rgba: Vec<u8> = film.pixels().iter().flat_map(|p| {
            let c = p.mean();
            [byte(c.r), byte(c.g), byte(c.b), 255]
        }).collect();
        let frame = tequila_texture::video::Frame { width, height, rgba, seconds: 0.0 };
        let name = if rounded { "bevel-on.png" } else { "bevel-off.png" };
        tequila_texture::video::save_png(&frame, &std::path::Path::new(&folder).join(name)).expect("writes");
    }
}
