//! A dome of a size on the card (`docs/07` R3): the sky looked up, sampled
//! and weighed from where a path is, against the processor's own
//! `Environment` on `finite_dome`'s dome - `radiance_from` and
//! `background_radiance_from`, `pdf_from`, `sample_from`,
//! `irradiance_bound` and `sample_irradiance` - through the transport's own
//! functions, as it calls them, on the renderer's uploaded scene.
//!
//! To 1e-4 relative (1e-6 absolute near nought), at most one item in a
//! thousand outside, and a draw found where the processor finds one.

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

/// Each item: the point, a direction, a normal and two draws in; out over
/// them, the radiance the lit image and the camera's give along the
/// direction, the density, a light sample (its direction, density,
/// radiance, distance), and the floor's draw (its direction, keep) and
/// bound, and whether each draw was found.
const KERNEL: &str = r#"
@compute @workgroup_size(64)
fn dome_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    let at = 6u * k;
    let origin = frame_samples[at].xyz;
    let direction = frame_samples[at + 1u].xyz;
    let normal = frame_samples[at + 2u].xyz;
    let u = frame_samples[at + 3u].xy;
    cy_zero = bitcast<u32>(frame_samples[at + 3u].z) & 0x80000000u;
    // radiance_from and background_radiance_from: past_the_scene's lookup.
    let towards = dome_lookup(origin, direction);
    let lit = env_radiance(false, towards);
    let seen = env_radiance(true, towards);
    let pdf = env_pdf_from(origin, direction, vec3<f32>(0.0));
    // sample_from: the whole sky from the centre, seen from the point.
    let e = env_sample_sky(vec3<f32>(0.0), u);
    var drawn = vec4<f32>(0.0);
    var drawn_radiance = vec4<f32>(0.0);
    var found = 0.0;
    if (e.ok) {
        let s = dome_from_centre(origin, e.direction);
        let p = e.pdf * s.jacobian;
        if (s.ok && p > 0.0) {
            drawn = vec4<f32>(s.direction, p);
            drawn_radiance = vec4<f32>(e.radiance, s.distance);
            found = 1.0;
        }
    }
    // irradiance_bound and sample_irradiance: the floor's shadow's.
    let spread = dome_spread_bound(origin);
    let bound = env_irradiance_bound(normal) * spread;
    let i = env_sample_irradiance(normal, u);
    var irradiance = vec4<f32>(0.0);
    var irradiance_found = 0.0;
    if (i.ok) {
        let s = dome_from_centre(origin, i.direction);
        if (s.ok) {
            irradiance = vec4<f32>(s.direction, min(cy_div(max(dot(normal, s.direction), 0.0), cy_exact(s.jacobian * spread)), 1.0));
            irradiance_found = s.distance;
        }
    }
    frame_samples[at] = vec4<f32>(lit, pdf);
    frame_samples[at + 1u] = vec4<f32>(seen, found);
    frame_samples[at + 2u] = drawn;
    frame_samples[at + 3u] = drawn_radiance;
    frame_samples[at + 4u] = irradiance;
    frame_samples[at + 5u] = vec4<f32>(bound, irradiance_found, 0.0, 0.0);
}
"#;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-6_f32.max(1e-4 * b.abs())
}

fn unit(rng: &mut Pcg32) -> Vec3 {
    let z = 1.0 - 2.0 * rng.next_f32();
    let phi = std::f32::consts::TAU * rng.next_f32();
    let r = (1.0 - z * z).max(0.0).sqrt();
    vec3(r * phi.cos(), z, r * phi.sin())
}

#[test]
fn the_dome_is_the_processors() {
    let Some(gpu) = device::for_test(Needs::RayQueries) else { return };
    let scene = parity::scene("finite_dome").expect("the parity set has it");
    // Its dome with the lower cap the ground too, which the parity scene
    // leaves round (a floor point on that plane draws its shadow along it,
    // where the processor's own rounding decides).
    let mut document = scene.document();
    tequila_scene::CommandBus::new()
        .apply(&mut document, tequila_scene::command::Command::SetEnvironmentParameter { parameter: tequila_scene::command::EnvironmentParameter::flag(tequila_scene::command::EnvParam::GroundFlatten, true) })
        .expect("the dome is flattened");
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    let dome = snap.environment.dome();
    let radius = dome.radius.expect("the dome has a size");
    assert!(dome.flatten, "the dome's lower cap is the ground");
    let renderer = GpuRenderer::new(gpu, &snap);
    let settings = FrameSettings { camera: scene.camera(&document), config: parity::config(&document), scene_seed: 7, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    let env = &snap.environment;

    struct Item {
        origin: Vec3,
        direction: Vec3,
        normal: Vec3,
        u: tequila_core::math::Vec2,
    }
    let mut rng = Pcg32::new(41, 9);
    let mut items = Vec::new();
    while items.len() < 20_000 {
        // Inside the dome, above the floor mostly, out to most of its
        // radius; a fifth of the normals straight up, as a floor's.
        let reach = radius * 0.9 * rng.next_f32().sqrt();
        let origin = vec3(0.0, dome.height, 0.0) + unit(&mut rng) * reach;
        if origin.y < -0.05 {
            continue;
        }
        let normal = if items.len() % 5 == 0 { Vec3::Y } else { unit(&mut rng) };
        items.push(Item { origin, direction: unit(&mut rng), normal, u: vec2(rng.next_f32(), rng.next_f32()) });
    }
    let count = items.len();
    let words: Vec<u32> = items
        .iter()
        .flat_map(|i| {
            let v = |x: f32, y: f32, z: f32| [x.to_bits(), y.to_bits(), z.to_bits(), 0];
            [v(i.origin.x, i.origin.y, i.origin.z), v(i.direction.x, i.direction.y, i.direction.z), v(i.normal.x, i.normal.y, i.normal.z), v(i.u.x, i.u.y, 0.0), v(0.0, 0.0, 0.0), v(0.0, 0.0, 0.0)]
        })
        .flatten()
        .collect();
    let kernel = KERNEL.replace("COUNTu", &format!("{count}u"));
    let out = renderer.run_on_scene(&settings, &kernel, "dome_test", &words, (count as u32).div_ceil(64));
    let f = |i: usize, v: usize, c: usize| f32::from_bits(out[(6 * i + v) * 4 + c]);

    let mut outside = [0usize; 5];
    let names = ["radiance", "density", "light sample", "floor's draw", "floor's bound"];
    let mut found = [0usize; 2];
    for (i, item) in items.iter().enumerate() {
        let mut fail = |k: usize, ok: bool, what: String| {
            if !ok {
                if outside[k] < 4 {
                    eprintln!("{} at {:?} along {:?}: {what}", names[k], item.origin, item.direction);
                }
                outside[k] += 1;
            }
        };
        let lit = env.radiance_from(item.origin, item.direction);
        let seen = env.background_radiance_from(item.origin, item.direction);
        let card_lit = [f(i, 0, 0), f(i, 0, 1), f(i, 0, 2)];
        let card_seen = [f(i, 1, 0), f(i, 1, 1), f(i, 1, 2)];
        fail(
            0,
            close(card_lit[0], lit.r) && close(card_lit[1], lit.g) && close(card_lit[2], lit.b) && close(card_seen[0], seen.r) && close(card_seen[1], seen.g) && close(card_seen[2], seen.b),
            format!("card {card_lit:?} {card_seen:?}, processor {lit:?} {seen:?}"),
        );
        let pdf = env.pdf_from(item.origin, item.direction);
        fail(1, close(f(i, 0, 3), pdf), format!("card {}, processor {pdf}", f(i, 0, 3)));
        let sample = env.sample_from(item.origin, item.u);
        let card_found = f(i, 1, 3) > 0.5;
        match sample {
            Some(s) => {
                found[0] += 1;
                let ok = card_found
                    && close(f(i, 2, 0), s.direction.x)
                    && close(f(i, 2, 1), s.direction.y)
                    && close(f(i, 2, 2), s.direction.z)
                    && close(f(i, 2, 3), s.pdf)
                    && close(f(i, 3, 0), s.radiance.r)
                    && close(f(i, 3, 1), s.radiance.g)
                    && close(f(i, 3, 2), s.radiance.b)
                    && close(f(i, 3, 3), s.distance);
                fail(2, ok, format!("card {:?} pdf {} distance {}, processor {:?} pdf {} distance {}", [f(i, 2, 0), f(i, 2, 1), f(i, 2, 2)], f(i, 2, 3), f(i, 3, 3), s.direction, s.pdf, s.distance));
            }
            None => fail(2, !card_found, "the card found a sample the processor did not".to_string()),
        }
        let irradiance = env.sample_irradiance(item.origin, item.normal, item.u);
        let card_distance = f(i, 5, 1);
        match irradiance {
            Some(s) => {
                found[1] += 1;
                let ok = card_distance > 0.0
                    && close(f(i, 4, 0), s.direction.x)
                    && close(f(i, 4, 1), s.direction.y)
                    && close(f(i, 4, 2), s.direction.z)
                    && close(f(i, 4, 3), s.keep)
                    && close(card_distance, s.distance);
                fail(3, ok, format!("card {:?} keep {} distance {card_distance}, processor {:?} keep {} distance {}", [f(i, 4, 0), f(i, 4, 1), f(i, 4, 2)], f(i, 4, 3), s.direction, s.keep, s.distance));
            }
            None => fail(3, card_distance == 0.0, "the card found a draw the processor did not".to_string()),
        }
        let bound = env.irradiance_bound(item.origin, item.normal);
        fail(4, close(f(i, 5, 0), bound), format!("card {}, processor {bound}", f(i, 5, 0)));
    }
    for (k, name) in names.iter().enumerate() {
        eprintln!("{name}: {} of {count} outside the bound", outside[k]);
    }
    eprintln!("light samples found {} times, the floor's draws {}", found[0], found[1]);
    assert!(found[0] > count / 2 && found[1] > count / 2, "the draws are found");
    assert!(outside.iter().all(|o| o * 1000 <= count), "not the processor's: {outside:?}");
}
