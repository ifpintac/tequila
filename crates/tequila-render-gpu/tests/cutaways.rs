//! Cutaways on the card (`docs/07` R3 tier B): `transport.wgsl`'s
//! `walk_ray` with its cutters (`cut_judge`) - the closest surface
//! (WALK_FIRST) - against the processor's `cutaway::intersect` through
//! `EmbreeIntersector::intersect`, on the parity set's `cutaways` scene - a
//! box, a cylinder, a sphere, a plane and a mesh of the Cutaway material:
//! the camera's rays, rays from around the cuts every way, and rays leaving
//! the caps the processor finds, as a bounce or a shadow ray leaves them,
//! within a reach or past everything. Each the surface met, its part,
//! whether it is a cap, its material, its distance, point and normal.
//!
//! The two meet the same surfaces in the same order: they part only where a
//! ray query and Embree round a triangle's edge apart, or a point on a
//! cutter's boundary falls inside it on one and outside on the other.

use std::collections::HashMap;
use tequila_core::math::{vec2, vec3, Ray, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::sampler::Sampler;
use tequila_render_core::cutaway::CAP_PRIMITIVE;
use tequila_render_core::intersect::Intersector;
use tequila_render_cpu::{Device, EmbreeIntersector};
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

/// One item: the ray in (its origin and reach, nought for past everything;
/// its direction), the surface out. cy_exact's nought from the reach's sign,
/// which the compiler cannot see.
const KERNEL: &str = r#"
@compute @workgroup_size(64)
fn cut_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    let a = frame_samples[3u * k];
    let d = frame_samples[3u * k + 1u].xyz;
    cy_zero = bitcast<u32>(a.w) & 0x80000000u;
    let hit = walk_ray(walk_new(WALK_FIRST, a.xyz, d, select(3.0e38, a.w, a.w > 0.0))).hit;
    if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
        frame_samples[3u * k] = vec4<f32>(0.0);
        frame_samples[3u * k + 1u] = vec4<f32>(0.0);
        frame_samples[3u * k + 2u] = vec4<f32>(0.0);
        return;
    }
    let s = hit_place(hit);
    frame_samples[3u * k] = vec4<f32>(s.position, hit.t);
    frame_samples[3u * k + 1u] = vec4<f32>(s.geometric, bitcast<f32>(hit_material(hit)));
    let cap = select(0u, 1u, hit.primitive_index == CAP_PRIMITIVE);
    frame_samples[3u * k + 2u] = bitcast<vec4<f32>>(vec4<u32>(1u, hit.instance_custom_data, cap, 0u));
}
"#;

#[test]
fn a_cutaway_query_is_the_processors() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("cutaways").expect("the parity set has it");
    let document = scene.document();
    let camera = scene.camera(&document);
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    assert!(snap.cutters().len() >= 5, "the scene has its cutters");
    let renderer = GpuRenderer::new(gpu, &snap);
    let slots: HashMap<u32, u32> = renderer.scene.instance_records.iter().enumerate().map(|(slot, r)| (r.instance, slot as u32)).collect();
    let settings = FrameSettings { camera, config: document.lighting.config(), scene_seed: 7, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snap).expect("builds");

    let mut rng = Pcg32::new(97, 3);
    let aspect = scene.width as f32 / scene.height as f32;
    let unit = |rng: &mut Pcg32| {
        let z = 1.0 - 2.0 * rng.next_f32();
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = std::f32::consts::TAU * rng.next_f32();
        vec3(r * phi.cos(), r * phi.sin(), z)
    };
    let mut rays: Vec<(Ray, f32)> = Vec::new();
    let mut caps = Vec::new();
    while rays.len() < 20_000 {
        let pick = rays.len() % 4;
        let ray = if pick == 0 || caps.is_empty() {
            let mut unused = Sampler::new(0, 0, 0, 0);
            camera.generate_ray(vec2(rng.next_f32(), rng.next_f32()), aspect, &mut unused)
        } else if pick == 1 {
            // From around the cut block and ball, any way.
            let from = vec3(0.1 + 1.0 * rng.next_f32(), 0.02 + 0.4 * rng.next_f32(), 0.65 + 0.6 * rng.next_f32());
            Ray::new(from, unit(&mut rng))
        } else {
            // Leaving a cap, off its face, into its side.
            let hit: &tequila_render_core::intersect::Hit = &caps[(rng.next_u32() as usize) % caps.len()];
            let mut d = unit(&mut rng);
            if d.dot(hit.geometric_normal) < 0.0 {
                d = -d;
            }
            Ray::new(hit.offset(hit.geometric_normal), d)
        };
        // Half the rays leaving caps within a reach, as a shadow ray to a
        // lamp is.
        let reach = if pick == 3 { 0.05 + rng.next_f32() } else { 0.0 };
        let mut limited = ray;
        if reach > 0.0 {
            limited.t_max = reach;
        }
        if let Some(hit) = intersector.intersect(&limited) {
            if hit.primitive == CAP_PRIMITIVE && caps.len() < 4096 {
                caps.push(hit);
            }
        }
        rays.push((ray, reach));
    }
    let mut words: Vec<u32> = Vec::with_capacity(rays.len() * 12);
    for (ray, reach) in &rays {
        let (o, d) = (ray.origin, ray.direction);
        words.extend_from_slice(&[o.x.to_bits(), o.y.to_bits(), o.z.to_bits(), reach.to_bits()]);
        words.extend_from_slice(&[d.x.to_bits(), d.y.to_bits(), d.z.to_bits(), 0]);
        words.extend_from_slice(&[0; 4]);
    }
    let kernel = KERNEL.replace("COUNT", &rays.len().to_string());
    let card = renderer.run_on_scene(&settings, &kernel, "cut_test", &words, (rays.len() as u32).div_ceil(64));

    let (mut met, mut capped, mut outside, mut exact) = (0usize, 0usize, 0usize, 0usize);
    let mut cases = Vec::new();
    for (k, (ray, reach)) in rays.iter().enumerate() {
        let w = &card[12 * k..12 * (k + 1)];
        let f = |i: usize| f32::from_bits(w[i]);
        let mut limited = *ray;
        if *reach > 0.0 {
            limited.t_max = *reach;
        }
        let want = intersector.intersect(&limited);
        let card_met = w[8] == 1;
        let agree = match &want {
            None => !card_met,
            Some(hit) => {
                met += 1;
                let cap = hit.primitive == CAP_PRIMITIVE;
                capped += cap as usize;
                let close = |a: Vec3, b: Vec3, tolerance: f32| (a - b).abs().max_component() <= tolerance * (1.0 + b.abs().max_component());
                let same = card_met && w[9] == slots[&hit.instance.0] && (w[10] == 1) == cap && w[7] == hit.material.0;
                exact += (same && f(3) == hit.t && vec3(f(0), f(1), f(2)) == hit.position) as usize;
                same && (f(3) - hit.t).abs() <= 1e-4 * (1.0 + hit.t) && close(vec3(f(0), f(1), f(2)), hit.position, 1e-4) && close(vec3(f(4), f(5), f(6)), hit.geometric_normal, 1e-4)
            }
        };
        if !agree {
            outside += 1;
            if cases.len() < 10 {
                cases.push(format!(
                    "{k}: ray {:?} along {:?} reach {reach}: card met {card_met} t {} slot {} cap {} material {} at {:?} normal {:?}; processor {:?}",
                    ray.origin,
                    ray.direction,
                    f(3),
                    w[9],
                    w[10],
                    w[7],
                    vec3(f(0), f(1), f(2)),
                    vec3(f(4), f(5), f(6)),
                    want.map(|h| (h.t, h.instance.0, h.primitive == CAP_PRIMITIVE, h.material.0, h.position, h.geometric_normal))
                ));
            }
        }
    }
    eprintln!("cutaway queries: {} rays, {met} met a surface, {capped} of them a cap, {exact} of them the processor's to the bit (its distance and point); {outside} apart", rays.len());
    for case in &cases {
        eprintln!("  {case}");
    }
    assert!(capped * 20 >= met, "a good share of the rays should meet a cap: {capped} of {met}");
    assert!(outside * 1000 <= rays.len(), "{outside} of {} cutaway queries are apart", rays.len());
}

/// One item: a shadow ray's two ends and their normals in, whether light
/// gets through out (WALK_SHADOW where nothing is cut out and no glass is let
/// by: the closest surface past the cutters, cutaway::occluded).
const SHADOW_KERNEL: &str = r#"
@compute @workgroup_size(64)
fn cut_shadow_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    let a = frame_samples[4u * k];
    let n = frame_samples[4u * k + 1u].xyz;
    let b = frame_samples[4u * k + 2u].xyz;
    let m = frame_samples[4u * k + 3u].xyz;
    cy_zero = bitcast<u32>(a.w) & 0x80000000u;
    let v = shadow_walk(GLASS_SOLID, shadow_ray(a.xyz, n, b, m), false, NO_SOLID, vec3<f32>(0.0), NO_LIGHT);
    frame_samples[4u * k] = vec4<f32>(v.through, 0.0);
}
"#;

/// The card's shadow rays past the cutters against the processor's
/// `cutaway::occluded` (`EmbreeIntersector::occluded`), on the same scene:
/// from the surfaces and caps the camera's rays meet, and from points around
/// the cuts, to points around them and far off, every way.
#[test]
fn a_cutaway_shadow_ray_is_the_processors() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("cutaways").expect("the parity set has it");
    let document = scene.document();
    let camera = scene.camera(&document);
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snap);
    let settings = FrameSettings { camera, config: document.lighting.config(), scene_seed: 7, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snap).expect("builds");
    let mut rng = Pcg32::new(53, 11);
    let aspect = scene.width as f32 / scene.height as f32;
    let around = |rng: &mut Pcg32| vec3(0.1 + 1.0 * rng.next_f32(), 0.02 + 0.4 * rng.next_f32(), 0.65 + 0.6 * rng.next_f32());
    let mut pairs: Vec<(Vec3, Vec3, Vec3, Vec3)> = Vec::new();
    while pairs.len() < 20_000 {
        let (from, normal) = if pairs.len() % 2 == 0 {
            let mut unused = Sampler::new(0, 0, 0, 0);
            let ray = camera.generate_ray(vec2(rng.next_f32(), rng.next_f32()), aspect, &mut unused);
            let Some(hit) = intersector.intersect(&ray) else { continue };
            (hit.position, hit.facing_forward(ray.direction).0)
        } else {
            let p = around(&mut rng);
            (p, vec3(0.0, 1.0, 0.0))
        };
        let to = if pairs.len() % 3 == 0 { vec3(-2.0 + 4.0 * rng.next_f32(), 3.0, -2.0 + 4.0 * rng.next_f32()) } else { around(&mut rng) };
        let to_normal = (from - to).normalize();
        pairs.push((from, normal, to, to_normal));
    }
    let mut words: Vec<u32> = Vec::with_capacity(pairs.len() * 16);
    for (a, n, b, m) in &pairs {
        for v in [a, n, b, m] {
            words.extend_from_slice(&[v.x.to_bits(), v.y.to_bits(), v.z.to_bits(), 0]);
        }
    }
    let kernel = SHADOW_KERNEL.replace("COUNT", &pairs.len().to_string());
    let card = renderer.run_on_scene(&settings, &kernel, "cut_shadow_test", &words, (pairs.len() as u32).div_ceil(64));
    let (mut blocked, mut apart) = (0usize, 0usize);
    let mut cases = Vec::new();
    for (k, (a, n, b, m)) in pairs.iter().enumerate() {
        let through = f32::from_bits(card[16 * k]);
        let processor = intersector.occluded(*a, *n, *b, *m);
        blocked += processor as usize;
        if (through == 0.0) != processor {
            apart += 1;
            if cases.len() < 10 {
                cases.push(format!("{k}: from {a:?} ({n:?}) to {b:?}: card lets {through} by, the processor's is {}", if processor { "blocked" } else { "open" }));
            }
        }
    }
    eprintln!("cutaway shadow rays: {}, {blocked} blocked on the processor; {apart} apart", pairs.len());
    for case in &cases {
        eprintln!("  {case}");
    }
    assert!(blocked * 10 >= pairs.len() && blocked * 10 <= pairs.len() * 9, "the rays should be blocked and open both: {blocked} of {}", pairs.len());
    assert!(apart * 1000 <= pairs.len(), "{apart} of {} cutaway shadow rays are apart", pairs.len());
}
