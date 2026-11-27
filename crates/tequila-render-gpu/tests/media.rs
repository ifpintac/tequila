//! Media that scatter on the card (`docs/07` R3 tier B): `media.wgsl`'s
//! arithmetic of the random walk against the processor's own
//! (`integrator::walk_twins`) - the exponential and logarithm it reads,
//! Henyey-Greenstein's phase function and a turn drawn by it, a flight
//! drawn in a channel, what survives a flight and its density - and
//! `transport.wgsl`'s medium transmittance (`shadow_walk` in a medium) and
//! a collision's light sample (`direct_light` at a collision) against the
//! processor's `medium_transmittance` and `medium_direct_light` on
//! `scattering_volumes`, at points inside its two volumes, on the scene's
//! own acceleration structure.
//!
//! The two draw the same numbers in the same order and do the same
//! arithmetic, its products rounded as Rust rounds them, its quotients,
//! roots, exponentials, logarithms, powers and arctangents correctly
//! rounded; they part where the processor's expf, logf and powf are not
//! (a few times in ten thousand), where its sinf and cosf and the card's
//! port of Cephes' round apart (an ulp now and then), and where a ray query
//! and Embree round a hit apart.

use tequila_core::math::{vec3, Vec2, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::sampler::Sampler;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::integrator::walk_twins;
use tequila_render_core::intersect::Intersector;
use tequila_render_cpu::{Device, EmbreeIntersector};
use tequila_render_gpu::device::{self, Needs};
use tequila_render_gpu::harness::Harness;
use tequila_render_gpu::transport::{FrameSettings, GpuRenderer};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};

const N: usize = 20_000;

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

fn f(card: &[u32], k: usize) -> f32 {
    f32::from_bits(card[k])
}

/// How far apart two floats are, in units in the last place of `want`.
fn ulps(got: f32, want: f32) -> f32 {
    if got == want {
        return 0.0;
    }
    let ulp = (f32::from_bits(want.abs().to_bits() + 1) - want.abs()).max(f32::MIN_POSITIVE);
    (got - want).abs() / ulp
}

/// Within `relative` of `want`, or `absolute` of it.
fn near(got: f32, want: f32, relative: f32, absolute: f32) -> bool {
    (got - want).abs() <= absolute.max(relative * want.abs()) || (got.is_nan() && want.is_nan()) || got == want
}

/// The largest difference of two colours' channels.
fn apart(a: Spectrum, b: Spectrum) -> f32 {
    (a.r - b.r).abs().max((a.g - b.g).abs()).max((a.b - b.b).abs())
}

fn unit(rng: &mut Pcg32) -> Vec3 {
    let z = 2.0 * rng.next_f32() - 1.0;
    let phi = std::f32::consts::TAU * rng.next_f32();
    let r = (1.0 - z * z).max(0.0).sqrt();
    vec3(r * phi.cos(), r * phi.sin(), z)
}

/// `body` over `inputs` with `media.wgsl` and the pictures' tables it reads
/// its exponential, logarithm, power and arctangent from (an empty set of
/// pictures has them all).
fn run_media(h: &Harness, body: &str, inputs: &[u32], in_words: usize, out_words: usize) -> Vec<u32> {
    let mut pictures = tequila_render_gpu::textures::CardPictures::default();
    pictures.finish();
    // cy_exact's zero, read from a word of each item the compiler cannot
    // see is nought.
    let body = format!("cy_zero = inp({in_words}u);
{body}");
    let padded: Vec<u32> = inputs.chunks(in_words).flat_map(|item| item.iter().copied().chain(std::iter::once(0))).collect();
    h.run_wgsl_with(&["media"], &body, &padded, in_words + 1, out_words, &[(16, &pictures.texels), (17, &pictures.header)])
}

/// Said, and asserted: at most one item in a thousand outside its bound.
fn report(name: &str, outside: usize, items: usize, cases: &[String]) {
    eprintln!("{name}: {outside} of {items} outside the bound");
    for case in cases.iter().take(8) {
        eprintln!("  {case}");
    }
    assert!(outside * 1000 <= items, "{name}: {outside} of {items} outside the bound");
}

/// The exponential and logarithm the walk reads are the processor's to an
/// ulp, over the range a walk reaches.
#[test]
fn the_walks_exponential_and_logarithm_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(3, 11);
    let mut inputs = Vec::new();
    let mut xs = Vec::new();
    for i in 0..N {
        let e = match i % 4 {
            0 => -100.0 * rng.next_f32(),
            1 => 2.0 * rng.next_f32() - 1.0,
            2 => (2.0 * rng.next_f32() - 1.0) * 1e-3,
            _ => 20.0 * rng.next_f32() - 10.0,
        };
        let l = match i % 3 {
            0 => (1.0 - rng.next_f32()).max(1e-12),
            1 => 10f32.powf(24.0 * rng.next_f32() - 12.0),
            _ => 1.0 + (2.0 * rng.next_f32() - 1.0) * 1e-3,
        };
        xs.push((e, l));
        inputs.extend(bits(&[e, l]));
    }
    let card = run_media(&h, "outf(0u, md_exp(inf(0u))); outf(1u, md_ln(inf(1u)));", &inputs, 2, 2);
    let (mut worst_exp, mut worst_ln, mut apart, mut outside) = (0.0f32, 0.0f32, 0usize, 0usize);
    let mut cases = Vec::new();
    for (k, (e, l)) in xs.iter().enumerate() {
        let (a, b) = (f(&card, 2 * k), f(&card, 2 * k + 1));
        let (de, dl) = (ulps(a, e.exp()), ulps(b, l.ln()));
        // Results the card flushes to nought, as it does the subnormals.
        let de = if e.exp() < f32::MIN_POSITIVE && a == 0.0 { 0.0 } else { de };
        worst_exp = worst_exp.max(de);
        worst_ln = worst_ln.max(dl);
        if de > 0.0 || dl > 0.0 {
            apart += 1;
        }
        if de > 1.0 || dl > 1.0 {
            outside += 1;
            cases.push(format!("exp({e:e}) card {a:e} cpu {:e}; ln({l:e}) card {b:e} cpu {:e}", e.exp(), l.ln()));
        }
    }
    eprintln!("exp and ln: {apart} of {N} not the processor's bits; worst {worst_exp} ulp (exp), {worst_ln} ulp (ln)");
    report("exp and ln within an ulp", outside, N, &cases);
}

/// The sine and cosine a turn reads, and the doorway's cosine-weighted
/// draw (`sample_cosine_hemisphere`), within an ulp or two of the
/// processor's.
#[test]
fn the_walks_sines_and_cosines_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(53, 59);
    let mut inputs = Vec::new();
    let mut items = Vec::new();
    for _ in 0..N {
        let t = std::f32::consts::TAU * rng.next_f32();
        let u = Vec2 { x: rng.next_f32(), y: rng.next_f32() };
        items.push((t, u));
        inputs.extend(bits(&[t, u.x, u.y]));
    }
    let body = "let sc = md_sincos(inf(0u)); outf(0u, sc.x); outf(1u, sc.y);
        let d = md_cosine_hemisphere(vec2<f32>(inf(1u), inf(2u))); outf(2u, d.x); outf(3u, d.y); outf(4u, d.z);";
    let card = run_media(&h, body, &inputs, 3, 5);
    let (mut outside, mut apart, mut draws_apart, mut worst) = (0usize, 0usize, 0usize, 0.0f32);
    let mut cases = Vec::new();
    for (k, (t, u)) in items.iter().enumerate() {
        let want = [t.sin(), t.cos()];
        let d = tequila_core::sampling::sample_cosine_hemisphere(*u);
        let got = [f(&card, 5 * k), f(&card, 5 * k + 1)];
        let dir = vec3(f(&card, 5 * k + 2), f(&card, 5 * k + 3), f(&card, 5 * k + 4));
        let u_apart = ulps(got[0], want[0]).max(ulps(got[1], want[1]));
        apart += (got != want) as usize;
        draws_apart += (dir != d) as usize;
        let dd = (dir - d).abs().max_component();
        worst = worst.max(u_apart);
        // Near a zero of either, an ulp of the argument is many of the
        // result's.
        if !((u_apart <= 2.0 || (got[0] - want[0]).abs().max((got[1] - want[1]).abs()) <= 1e-7) && dd <= 1e-6) {
            outside += 1;
            cases.push(format!("{k}: t {t}: card {got:?} cpu {want:?}; draw {u:?} card {dir:?} cpu {d:?}"));
        }
    }
    eprintln!("sines and cosines: {apart} of {N} not the processor's bits, worst {worst} ulp; draws {draws_apart} not its bits");
    report("sines and cosines within two ulps, draws within 1e-6", outside, N, &cases);
}

/// Henyey-Greenstein's phase function, and a turn drawn by it.
#[test]
fn the_phase_function_and_its_turns_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(5, 13);
    let mut inputs = Vec::new();
    let mut items = Vec::new();
    for i in 0..N {
        let g = match i % 5 {
            0 => 0.0,
            1 => (2.0 * rng.next_f32() - 1.0) * 1e-3,
            2 => 0.95 * (2.0 * rng.next_f32() - 1.0),
            3 => if rng.next_f32() < 0.5 { 0.99 } else { -0.995 },
            _ => 2.0 * rng.next_f32() - 1.0,
        };
        let cos = 2.0 * rng.next_f32() - 1.0;
        let forward = unit(&mut rng);
        let u = Vec2 { x: rng.next_f32(), y: rng.next_f32() };
        items.push((g, cos, forward, u));
        inputs.extend(bits(&[g, cos, forward.x, forward.y, forward.z, u.x, u.y]));
    }
    let body = "let g = inf(0u);
        outf(0u, md_phase(inf(1u), g));
        let d = md_sample_phase(vec3<f32>(inf(2u), inf(3u), inf(4u)), g, vec2<f32>(inf(5u), inf(6u)));
        outf(1u, d.x); outf(2u, d.y); outf(3u, d.z);";
    let card = run_media(&h, body, &inputs, 7, 4);
    let (mut outside, mut worst_p, mut worst_d) = (0usize, 0.0f32, 0.0f32);
    let mut cases = Vec::new();
    for (k, (g, cos, forward, u)) in items.iter().enumerate() {
        let p = walk_twins::henyey_greenstein(*cos, *g);
        let d = tequila_core::sampling::sample_henyey_greenstein(*forward, *g, *u);
        let cp = f(&card, 4 * k);
        let cd = vec3(f(&card, 4 * k + 1), f(&card, 4 * k + 2), f(&card, 4 * k + 3));
        let dp = (cp - p).abs() / p.abs().max(1e-30);
        let dd = (cd - d).abs().max_component();
        worst_p = worst_p.max(dp);
        worst_d = worst_d.max(dd);
        if !(dp <= 1e-5) || !(dd <= 1e-5) {
            outside += 1;
            cases.push(format!("g {g} cos {cos}: phase card {cp} cpu {p}; turn card {cd:?} cpu {d:?}"));
        }
    }
    eprintln!("phase: worst {worst_p:.2e} relative; turns: worst {worst_d:.2e}");
    report("the phase function within 1e-5 and its turns within 1e-5", outside, N, &cases);
}

/// A flight: each channel's chance and the optical depth flown, and what
/// survives it and its density, plain - colliding at its end or flying
/// past it.
#[test]
fn a_walks_flight_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(7, 17);
    let mut inputs = Vec::new();
    let mut items = Vec::new();
    let colour = |rng: &mut Pcg32| -> Spectrum {
        let mut c = || match rng.next_f32() {
            x if x < 0.05 => 0.0,
            x if x < 0.1 => 1.0,
            _ => rng.next_f32() * 2.0,
        };
        Spectrum { r: c(), g: c(), b: c() }
    };
    for i in 0..N {
        let throughput = if i % 50 == 0 { Spectrum::ZERO } else { colour(&mut rng) };
        let albedo = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
        let albedo = if i % 7 == 0 { Spectrum::ZERO } else { albedo };
        let sigma = Spectrum { r: 10f32.powf(4.0 * rng.next_f32()), g: 10f32.powf(4.0 * rng.next_f32()), b: 10f32.powf(4.0 * rng.next_f32()) };
        let stretch = if i % 2 == 0 { 1.0 } else { 0.2 + 1.6 * rng.next_f32() };
        let (pick, along) = (rng.next_f32(), rng.next_f32());
        let t = 0.05 * rng.next_f32() * rng.next_f32();
        items.push((throughput, albedo, sigma, stretch, pick, along, t));
        inputs.extend(bits(&[throughput.r, throughput.g, throughput.b, albedo.r, albedo.g, albedo.b, sigma.r, sigma.g, sigma.b, stretch, pick, along, t]));
    }
    let body = "let sigma = vec3<f32>(inf(6u), inf(7u), inf(8u));
        let fl = md_walk_flight(vec3<f32>(inf(0u), inf(1u), inf(2u)), vec3<f32>(inf(3u), inf(4u), inf(5u)), sigma, inf(9u), inf(10u), inf(11u));
        outf(0u, fl.chance.x); outf(1u, fl.chance.y); outf(2u, fl.chance.z); outf(3u, fl.depth);
        let kept = md_survive(sigma, inf(12u));
        outf(4u, kept.x); outf(5u, kept.y); outf(6u, kept.z);
        // The density of ending there, by the walk's own end (no guide).
        outf(7u, md_walk_end(md_no_guide(), false, fl.chance, sigma, inf(12u), true).w);
        outf(8u, md_walk_end(md_no_guide(), false, fl.chance, sigma, inf(12u), false).w);";
    let card = run_media(&h, body, &inputs, 13, 9);
    let (mut outside, mut chances_apart) = (0usize, 0usize);
    let mut cases = Vec::new();
    for (k, (throughput, albedo, sigma, stretch, pick, along, t)) in items.iter().enumerate() {
        let (chance, depth) = walk_twins::walk_flight(*throughput, *albedo, *sigma, *stretch, (*pick, *along));
        let kept = Spectrum { r: (-sigma.r * t).exp(), g: (-sigma.g * t).exp(), b: (-sigma.b * t).exp() };
        let collided = chance.r * sigma.r * kept.r + chance.g * sigma.g * kept.g + chance.b * sigma.b * kept.b;
        let past = chance.r * kept.r + chance.g * kept.g + chance.b * kept.b;
        let c = &card[9 * k..9 * k + 9];
        let got: Vec<f32> = c.iter().map(|b| f32::from_bits(*b)).collect();
        if [got[0], got[1], got[2]] != [chance.r, chance.g, chance.b] {
            chances_apart += 1;
        }
        let ok = near(got[0], chance.r, 1e-6, 0.0)
            && near(got[1], chance.g, 1e-6, 0.0)
            && near(got[2], chance.b, 1e-6, 0.0)
            && near(got[3], depth, 1e-6, 0.0)
            && near(got[4], kept.r, 1e-6, 1e-30)
            && near(got[5], kept.g, 1e-6, 1e-30)
            && near(got[6], kept.b, 1e-6, 1e-30)
            && near(got[7], collided, 1e-5, 1e-30)
            && near(got[8], past, 1e-5, 1e-30);
        if !ok {
            outside += 1;
            cases.push(format!("{k}: card {got:?}; cpu {:?} {depth} {:?} {collided} {past}", chance, kept));
        }
    }
    eprintln!("flights: {chances_apart} of {N} chances not the processor's bits");
    report("flights within 1e-6, densities within 1e-5", outside, N, &cases);
}

/// A walk under a surface guided back towards where it came in
/// (`WalkGuide`): the guide made for a medium (its diffusion length, its
/// logarithm, its share of guided turns), the first flight's look ahead, a
/// turn guided or by the phase function - across to a surface opposite or
/// not - on the open stream, and the flight after it: its stretch, and its
/// density by every way it could have been drawn, colliding or flying
/// past.
#[test]
fn a_guided_walk_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(37, 41);
    let mut inputs = Vec::new();
    let mut items = Vec::new();
    while items.len() < N {
        let colour = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
        let radius = Spectrum { r: 0.001 + 0.2 * rng.next_f32(), g: 0.001 + 0.2 * rng.next_f32(), b: 0.001 + 0.2 * rng.next_f32() };
        let g = match items.len() % 4 {
            0 => 0.0,
            1 => 0.9 * (2.0 * rng.next_f32() - 1.0),
            2 => 0.3 * (2.0 * rng.next_f32() - 1.0),
            _ => 0.97,
        };
        let material = tequila_materials::openpbr::Material { subsurface_weight: 1.0, subsurface_color: colour, subsurface_radius: radius, subsurface_scatter_anisotropy: g, ..Default::default() };
        let Some(walk) = material.subsurface_medium() else { continue };
        let entry = unit(&mut rng) * 0.1;
        let normal = unit(&mut rng);
        let at = entry - normal * (0.05 * rng.next_f32()) + unit(&mut rng) * 0.02 * rng.next_f32();
        let forward = unit(&mut rng);
        let opposite = if rng.next_f32() < 0.5 { Some(0.01 + 0.05 * rng.next_f32()) } else { None };
        let pixel = [rng.next_u32() % 4096, rng.next_u32() % 4096, rng.next_u32() % 1024, rng.next_u32() % 65536];
        let chance = {
            let c = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
            let t = c.r + c.g + c.b;
            Spectrum { r: c.r / t, g: c.g / t, b: c.b / t }
        };
        let t = 0.05 * rng.next_f32() * rng.next_f32();
        items.push((walk, entry, normal, at, forward, opposite, pixel, chance, t));
        inputs.extend(bits(&[walk.albedo.r, walk.albedo.g, walk.albedo.b, walk.anisotropy, walk.extinction.r, walk.extinction.g, walk.extinction.b]));
        inputs.extend(bits(&[entry.x, entry.y, entry.z, normal.x, normal.y, normal.z, at.x, at.y, at.z, forward.x, forward.y, forward.z, opposite.unwrap_or(-1.0)]));
        inputs.extend_from_slice(&pixel);
        inputs.extend(bits(&[chance.r, chance.g, chance.b, t]));
    }
    let body = "let albedo = vec3<f32>(inf(0u), inf(1u), inf(2u));
        let g = inf(3u);
        let sigma = vec3<f32>(inf(4u), inf(5u), inf(6u));
        var guide = md_guide_new(7u, vec3<f32>(inf(7u), inf(8u), inf(9u)), vec3<f32>(inf(10u), inf(11u), inf(12u)), albedo, g);
        outf(0u, select(0.0, 1.0, guide.solid != NO_WALK));
        outf(1u, guide.diffusion); outf(2u, guide.phase_log); outf(3u, guide.share);
        outf(4u, md_guide_look(sigma));
        if (guide.solid != NO_WALK) {
            guide.opposite = inf(19u);
            var s = sampler_new(inp(20u), inp(21u), inp(22u), inp(23u));
            let next = md_guide_turn(&guide, vec3<f32>(inf(13u), inf(14u), inf(15u)), vec3<f32>(inf(16u), inf(17u), inf(18u)), g, &s);
            outf(5u, next.x); outf(6u, next.y); outf(7u, next.z);
            outf(8u, md_guide_stretch(guide));
            let chance = vec3<f32>(inf(24u), inf(25u), inf(26u));
            let kept = md_survive(sigma, inf(27u));
            outf(9u, md_guided_density(guide, chance, sigma, kept, inf(27u), true));
            outf(10u, md_guided_density(guide, chance, sigma, kept, inf(27u), false));
            outf(11u, select(0.0, 1.0, guide.guided));
            outf(12u, select(0.0, 1.0, guide.backward));
            outf(13u, sampler_next_unstructured(&s));
        }";
    let card = run_media(&h, body, &inputs, 28, 14);
    let (mut outside, mut guided, mut made, mut backward, mut draws_apart) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut cases = Vec::new();
    for (k, (walk, entry, normal, at, forward, opposite, pixel, chance, t)) in items.iter().enumerate() {
        let c: Vec<f32> = card[14 * k..14 * k + 14].iter().map(|b| f32::from_bits(*b)).collect();
        let look = walk_twins::Guide::look(walk.extinction);
        let mut ok = near(c[4], look, 1e-6, 0.0);
        let guide = walk_twins::Guide::new(7, *entry, *normal, walk);
        ok &= (c[0] == 1.0) == guide.is_some();
        if let Some(mut guide) = guide {
            made += 1;
            let (diffusion, phase_log, share) = guide.parts();
            ok &= near(c[1], diffusion, 1e-5, 0.0) && near(c[2], phase_log, 1e-5, 0.0) && near(c[3], share, 1e-5, 0.0);
            guide.set_opposite(*opposite);
            let mut sampler = Sampler::new(pixel[0], pixel[1], pixel[2], pixel[3]);
            let next = guide.turn(*at, *forward, walk.anisotropy, &mut sampler);
            let following = sampler.next_unstructured();
            if c[13].to_bits() != following.to_bits() {
                draws_apart += 1;
            }
            let (was_guided, was_backward, ..) = guide.last_turn().expect("a turn");
            guided += was_guided as usize;
            backward += was_backward as usize;
            ok &= (c[11] == 1.0) == was_guided && (c[12] == 1.0) == was_backward;
            ok &= (vec3(c[5], c[6], c[7]) - next).abs().max_component() <= 1e-4;
            ok &= near(c[8], guide.stretch(), 1e-4, 0.0);
            let collided = guide.density(*chance, walk.extinction, *t, true).expect("a turn");
            let past = guide.density(*chance, walk.extinction, *t, false).expect("a turn");
            ok &= near(c[9], collided, 1e-4, 1e-30) && near(c[10], past, 1e-4, 1e-30);
            if !ok {
                cases.push(format!("{k}: card {c:?}; cpu {diffusion} {phase_log} {share} {next:?} {} {collided} {past}", guide.stretch()));
            }
        } else if !ok {
            cases.push(format!("{k}: card {c:?}; cpu no guide, look {look}"));
        }
        outside += !ok as usize;
    }
    eprintln!("guided walks: {made} guides of {N}, {guided} turns guided, {backward} across; {draws_apart} left the sampler elsewhere");
    assert_eq!(draws_apart, 0, "the card draws as many numbers as the processor");
    report("guides, turns and their densities within 1e-4", outside, N, &cases);
}

/// What `scattering_volumes` puts in a test kernel's way: the renderer,
/// the processor's intersector over the same snapshot, its two volumes
/// (each a centre, a radius and its slot), the frame's settings.
struct Volumes {
    renderer: GpuRenderer,
    intersector: EmbreeIntersector,
    balls: Vec<(Vec3, f32, u32, walk_twins::SubsurfaceMedium)>,
    settings: FrameSettings,
}

fn volumes() -> Option<Volumes> {
    let gpu = device::for_test(Needs::RayQueries)?;
    let scene = parity::scene("scattering_volumes").expect("the parity set has it");
    let document = scene.document();
    let camera = scene.camera(&document);
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let (snap, _) = snapshot(&document, environment, &mut MeshCache::new());
    let renderer = GpuRenderer::new(gpu, &snap);
    let settings = FrameSettings { camera, config: parity::config(&document), scene_seed: 7, reject_fireflies: true, clamp_direct: None, clamp_indirect: None };
    let intersector = EmbreeIntersector::build(&Device::shared().expect("an Embree device"), snap).expect("builds");
    let scene_ref = intersector.scene();
    let balls: Vec<_> = scene_ref
        .instances
        .iter()
        .enumerate()
        .filter_map(|(n, i)| {
            let m = scene_ref.material(i.material);
            let walk = m.volume_scattering()?;
            let b = scene_ref.meshes[i.mesh.index()].bounds().transformed(&i.transform);
            Some((b.center(), 0.5 * (b.max.x - b.min.x), n as u32, walk))
        })
        .collect();
    assert_eq!(balls.len(), 2, "the haze and the wax");
    Some(Volumes { renderer, intersector, balls, settings })
}

/// A point inside one of the volumes, well inside or near its surface.
fn inside(rng: &mut Pcg32, balls: &[(Vec3, f32, u32, walk_twins::SubsurfaceMedium)]) -> (Vec3, usize) {
    let b = (rng.next_u32() as usize) % balls.len();
    let (centre, radius, _, _) = balls[b];
    let r = radius * 0.97 * rng.next_f32().cbrt();
    (centre + unit(rng) * r, b)
}

const TRANSMITTANCE_KERNEL: &str = r#"
@compute @workgroup_size(64)
fn transmittance_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    let a = frame_samples[3u * k];
    let b = frame_samples[3u * k + 1u];
    let c = frame_samples[3u * k + 2u];
    cy_zero = bitcast<u32>(c.w);
    let through = shadow_walk(GLASS_SOLID, ShadowRay(a.xyz, b.xyz, a.w), true, bitcast<u32>(b.w), c.xyz, NO_LIGHT).through;
    frame_samples[3u * k] = vec4<f32>(through, 0.0);
}
"#;

/// What survives from a point inside a volume to a reach along a way:
/// through its own surface, stopped by the product and the ground.
#[test]
fn what_survives_a_medium_towards_a_light_is_the_processors() {
    let Some(v) = volumes() else { return };
    let mut rng = Pcg32::new(19, 23);
    let mut items = Vec::new();
    let mut words = Vec::new();
    for i in 0..N {
        let (at, b) = inside(&mut rng, &v.balls);
        let direction = unit(&mut rng);
        let reach = if i % 3 == 0 { v.intersector.scene().bounds().bounding_radius().max(1.0) * 1e4 } else { 2.0 * rng.next_f32() };
        let (_, _, slot, walk) = v.balls[b];
        items.push((at, direction, reach, slot, walk));
        words.extend(bits(&[at.x, at.y, at.z, reach, direction.x, direction.y, direction.z]));
        words.push(slot);
        words.extend(bits(&[walk.extinction.r, walk.extinction.g, walk.extinction.b, 0.0]));
    }
    let kernel = TRANSMITTANCE_KERNEL.replace("COUNT", &items.len().to_string());
    let card = v.renderer.run_on_scene(&v.settings, &kernel, "transmittance_test", &words, (items.len() as u32).div_ceil(64));
    let (mut outside, mut blocked, mut worst) = (0usize, 0usize, 0.0f32);
    let mut cases = Vec::new();
    for (k, (at, direction, reach, slot, walk)) in items.iter().enumerate() {
        let mut sampler = Sampler::new(0, 0, 0, 0);
        let want = walk_twins::medium_transmittance(&v.intersector, *at, *direction, *reach, *slot, walk, &mut sampler);
        let got = Spectrum { r: f(&card, 12 * k), g: f(&card, 12 * k + 1), b: f(&card, 12 * k + 2) };
        if want.is_black() {
            blocked += 1;
        }
        let d = apart(got, want) / want.max_component().max(1e-6);
        worst = worst.max(d);
        if !(d <= 1e-4) {
            outside += 1;
            cases.push(format!("{k}: from {at:?} along {direction:?} to {reach}: card {got:?} cpu {want:?}"));
        }
    }
    eprintln!("transmittance: {blocked} of {N} stopped; worst {worst:.2e}");
    report("what survives a medium within 1e-4", outside, N, &cases);
}

const LIGHT_KERNEL: &str = r#"
@compute @workgroup_size(64)
fn medium_light_test(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.x;
    if (k >= COUNTu) {
        return;
    }
    let a = frame_samples[4u * k];
    let b = frame_samples[4u * k + 1u];
    let c = frame_samples[4u * k + 2u];
    cy_zero = bitcast<u32>(c.w);
    let d = bitcast<vec4<u32>>(frame_samples[4u * k + 3u]);
    var s = sampler_new(d.x, d.y, d.z, d.w & 0xFFFFu);
    sampler_begin_bounce(&s, d.w >> 16u);
    // A collision's light sample, after a throughput of one and unclamped:
    // the sky's and the lamp's summed.
    var here: LightSampleAt;
    here.wanted = true;
    here.medium = true;
    here.surf.position = a.xyz;
    here.wo = b.xyz;
    here.instance = bitcast<u32>(b.w);
    here.throughput = vec3<f32>(1.0);
    here.ceiling = 0.0;
    here.sigma = c.xyz;
    here.g = a.w;
    // Its draws, as the collision takes them before its turn (the sky's,
    // the lamp chosen by power and the lamp's): direct_light draws none.
    here.u_sky = sampler_light_sample(&s);
    if (FEATURE_LAMPS && frame.light_count > 0u) {
        here.choice = lamps_select(frame.light_count, sampler_light_select(&s));
        if (here.choice.ok && lamp_lights(scene_lights[here.choice.index], here.instance)) {
            here.u_lamp = sampler_light_sample(&s);
        }
    }
    let lit = direct_light(&s, here).total;
    let next = sampler_next_unstructured(&s);
    frame_samples[4u * k] = vec4<f32>(lit, next);
}
"#;

/// The lights a collision samples directly, in a medium whose surface bends
/// nothing: the whole sky's draw and a lamp chosen by power, each weighed
/// by the phase function and what of the medium lies between, summed; on
/// the bounces' dimensions, past the Sobol' limit the open stream's.
#[test]
fn the_lights_a_collision_samples_are_the_processors() {
    let Some(v) = volumes() else { return };
    let config = v.settings.config.clone();
    let mut rng = Pcg32::new(29, 31);
    let mut items = Vec::new();
    let mut words = Vec::new();
    for _ in 0..N {
        let (at, b) = inside(&mut rng, &v.balls);
        let forward = unit(&mut rng);
        let (_, _, slot, walk) = v.balls[b];
        let pixel = [rng.next_u32() % 4096, rng.next_u32() % 4096, rng.next_u32() % 1024, rng.next_u32() % 65536];
        let bounce = rng.next_u32() % 9;
        items.push((at, forward, slot, walk, pixel, bounce));
        words.extend(bits(&[at.x, at.y, at.z, walk.anisotropy, forward.x, forward.y, forward.z]));
        words.push(slot);
        words.extend(bits(&[walk.extinction.r, walk.extinction.g, walk.extinction.b, 0.0]));
        words.extend_from_slice(&[pixel[0], pixel[1], pixel[2], pixel[3] | (bounce << 16)]);
    }
    let kernel = LIGHT_KERNEL.replace("COUNT", &items.len().to_string());
    let card = v.renderer.run_on_scene(&v.settings, &kernel, "medium_light_test", &words, (items.len() as u32).div_ceil(64));
    let (mut outside, mut draws_apart, mut lit_sky, mut lit_lamps, mut worst) = (0usize, 0usize, 0usize, 0usize, 0.0f32);
    let mut cases = Vec::new();
    for (k, (at, forward, slot, walk, pixel, bounce)) in items.iter().enumerate() {
        let mut sampler = Sampler::new(pixel[0], pixel[1], pixel[2], pixel[3]);
        sampler.begin_bounce(*bounce);
        let (sky, lights) = walk_twins::medium_direct_light(&v.intersector, &config, *at, *forward, walk, *slot, &mut sampler);
        let next = sampler.next_unstructured();
        let got = Spectrum { r: f(&card, 16 * k), g: f(&card, 16 * k + 1), b: f(&card, 16 * k + 2) };
        if f(&card, 16 * k + 3).to_bits() != next.to_bits() {
            draws_apart += 1;
        }
        lit_sky += (!sky.is_black()) as usize;
        lit_lamps += (!lights.is_black()) as usize;
        let want = sky + lights;
        let d = apart(got, want) / want.max_component().max(1e-6);
        worst = worst.max(d);
        if !(d <= 1e-4) {
            outside += 1;
            cases.push(format!("{k}: at {at:?} bounce {bounce}: card {got:?} cpu {want:?} (sky {sky:?}, lamps {lights:?})"));
        }
    }
    eprintln!("medium light: {lit_sky} lit by the sky, {lit_lamps} by a lamp, of {N}; worst {worst:.2e}; {draws_apart} left the sampler elsewhere");
    assert_eq!(draws_apart, 0, "the card draws as many numbers as the processor");
    assert!(lit_sky * 4 > N && lit_lamps * 10 > N, "the sky and the lamps light a good share");
    report("the lights a collision samples within 1e-4", outside, N, &cases);
}
