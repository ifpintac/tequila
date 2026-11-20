//! Glass on the card against `tequila_materials`: `DielectricTransmission`
//! on its own - rough and smooth, solid and thin-walled, from outside and
//! from inside, index-matched - and then `OpenPbrBsdf` for random materials
//! that transmit, built face on and aimed, from outside and from inside, at
//! a roughness floor of zero and above it: coat over glass, the interface
//! and the diffuse sharing what the transmission leaves, the transmission
//! colour under the interface, and the sixth lobe a full stack drops. Each
//! is evaluated, its density taken, sampled (whether through or off, and the
//! crossing's index) and its albedo read; and the stack's `pdf_reflection`,
//! `eval_selective` and flags, for pairs of directions on both sides.
//!
//! The tolerances are `tests/openpbr.rs`'s: values to 1e-4 relative (1e-6
//! absolute), a sampled direction to 2e-4 absolute, a sample's value and
//! density to 2e-3 relative, and one item in a thousand may be looser, to
//! 5% (1e-5 absolute).

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use tequila_materials::fresnel::ComplexIor;
use tequila_materials::lobes::{DielectricTransmission, OpenPbrBsdf};
use tequila_materials::openpbr::Material;
use tequila_render_gpu::harness::Harness;
use tequila_render_gpu::material::GpuMaterial;

const N: usize = 20_000;

/// A direction in the upper hemisphere, now and then a grazing one; with
/// chance `below`, mirrored into the lower.
fn direction(rng: &mut Pcg32, below: f32) -> Vec3 {
    let mu = if rng.next_f32() < 0.1 { rng.next_f32() * 0.05 + 1e-3 } else { rng.next_f32().max(1e-3) };
    let phi = std::f32::consts::TAU * rng.next_f32();
    let s = (1.0 - mu * mu).max(0.0).sqrt();
    let sign = if rng.next_f32() < below { -1.0 } else { 1.0 };
    vec3(s * phi.cos(), s * phi.sin(), sign * mu)
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Value,
    Flag,
    Direction,
    Sampled,
}

struct Word {
    kind: Kind,
    sampled: bool,
}

const fn w(kind: Kind, sampled: bool) -> Word {
    Word { kind, sampled }
}

/// As `tests/openpbr.rs` compares: every word within its bound but for at
/// most one item in a thousand, which must still be within 5%. Returns how
/// many items were looser than the tight bound.
fn compare(name: &str, card: &[u32], want: &[Vec<f32>], words: &[Word], ok: usize, cases: &[String]) -> usize {
    let n = words.len();
    let mut wrong = Vec::new();
    let mut loose = Vec::new();
    let mut detail = Vec::new();
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = card[i * n..(i + 1) * n].iter().map(|b| f32::from_bits(*b)).collect();
        let mut item_loose = false;
        for (k, word) in words.iter().enumerate() {
            if word.sampled && (c[ok] == 0.0 || w[ok] == 0.0) {
                continue;
            }
            let (absolute, relative) = match word.kind {
                Kind::Value => (1e-6, 1e-4),
                Kind::Flag => (0.0, 0.0),
                Kind::Direction => (2e-4, 0.0),
                Kind::Sampled => (1e-6, 2e-3),
            };
            let d = (c[k] - w[k]).abs();
            if d <= absolute + relative * w[k].abs() {
                continue;
            }
            if word.kind != Kind::Flag && d <= 1e-5 + 5e-2 * w[k].abs() {
                if !item_loose && loose.len() < 5 {
                    detail.push(format!("  #{i} word {k}: card {}, processor {}", c[k], w[k]));
                }
                item_loose = true;
            } else if wrong.len() < 8 {
                wrong.push(format!("  #{i} word {k}: card {}, processor {} ({})", c[k], w[k], cases[i]));
            }
        }
        if item_loose {
            loose.push(i);
        }
    }
    assert!(wrong.is_empty(), "{name} differs:\n{}", wrong.join("\n"));
    assert!(
        loose.len() * 1000 <= want.len(),
        "{name}: {} items of {} outside the tight bound (first {:?}); at most one in a thousand may be",
        loose.len(),
        want.len(),
        &loose[..loose.len().min(5)]
    );
    println!("{name}: {} of {} items looser than the tight bound\n{}", loose.len(), want.len(), detail.join("\n"));
    loose.len()
}

/// A sample's words: drawn, wi, f, pdf, delta, through, eta.
fn push_sample(e: &mut Vec<f32>, sample: Option<BsdfSample>) {
    match sample {
        Some(s) => e.extend([1.0, s.wi.x, s.wi.y, s.wi.z, s.f.r, s.f.g, s.f.b, s.pdf, s.is_delta as u8 as f32, s.is_transmission as u8 as f32, s.eta]),
        None => e.extend([0.0; 11]),
    }
}

/// `s` (a LobeSample) into words 4 to 14.
const SAMPLE_OUT: &str = "
    if (s.ok) {
        outf(4u, 1.0);
        outf(5u, s.wi.x); outf(6u, s.wi.y); outf(7u, s.wi.z);
        outf(8u, s.f.x); outf(9u, s.f.y); outf(10u, s.f.z);
        outf(11u, s.pdf);
        outf(12u, select(0.0, 1.0, s.is_delta));
        outf(13u, select(0.0, 1.0, s.is_transmission));
        outf(14u, s.eta);
    } else {
        for (var k = 4u; k < 15u; k++) { outf(k, 0.0); }
    }";

/// eval rgb, pdf, then the sample's eleven words.
fn sample_words() -> Vec<Word> {
    let mut words = vec![w(Kind::Value, false); 4];
    words.push(w(Kind::Flag, false));
    words.extend([w(Kind::Direction, true), w(Kind::Direction, true), w(Kind::Direction, true)]);
    words.extend([w(Kind::Sampled, true), w(Kind::Sampled, true), w(Kind::Sampled, true), w(Kind::Sampled, true)]);
    words.extend([w(Kind::Flag, true), w(Kind::Flag, true), w(Kind::Value, true)]);
    words
}

impl Clone for Word {
    fn clone(&self) -> Word {
        Word { kind: self.kind, sampled: self.sampled }
    }
}

/// 0 with chance `zero`, 1 with chance `one`, else anywhere between.
fn unit(rng: &mut Pcg32, zero: f32, one: f32) -> f32 {
    let x = rng.next_f32();
    if x < zero {
        0.0
    } else if x < zero + one {
        1.0
    } else {
        rng.next_f32()
    }
}

fn colour(rng: &mut Pcg32) -> Spectrum {
    Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() }
}

fn tint(rng: &mut Pcg32) -> Spectrum {
    if rng.next_f32() < 0.5 { Spectrum::ONE } else { colour(rng) }
}

fn roughness(rng: &mut Pcg32) -> f32 {
    match rng.next_f32() {
        x if x < 0.25 => 0.0,
        x if x < 0.35 => rng.next_f32() * 0.05,
        _ => rng.next_f32(),
    }
}

/// An index of refraction: now and then matched to the outside (exactly 1,
/// or within the lobe's thousandth of it), else up to a diamond's and past.
fn ior(rng: &mut Pcg32) -> f32 {
    match rng.next_f32() {
        x if x < 0.04 => 1.0,
        x if x < 0.08 => 1.0 + rng.next_f32() * 1e-3,
        _ => 1.0 + rng.next_f32() * 1.6,
    }
}

/// The lobe's words: the sample's (`sample_words`), then its albedo rgb.
#[test]
fn the_transmission_lobe_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(41, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    let (mut through, mut deltas, mut misses) = (0, 0, 0);
    for _ in 0..N {
        let (r, n) = (roughness(&mut rng), ior(&mut rng));
        let t = tint(&mut rng);
        let thin = rng.next_f32() < 0.3;
        let inside = rng.next_f32() < 0.4;
        let (wo, wi) = (direction(&mut rng, 0.05), direction(&mut rng, 0.5));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let lobe = DielectricTransmission::new(r, n).with_tint(t).thin_walled(thin).from_inside(inside);
        inputs.extend(bits(&[r, n, t.r, t.g, t.b, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y, uc]));
        inputs.push(thin as u32 | (inside as u32) << 1);
        let eval = lobe.eval(wo, wi);
        let mut e = vec![eval.r, eval.g, eval.b, lobe.pdf(wo, wi)];
        let sample = lobe.sample(wo, u, uc);
        push_sample(&mut e, sample);
        let albedo = lobe.albedo(wo);
        e.extend([albedo.r, albedo.g, albedo.b]);
        want.push(e);
        through += sample.is_some_and(|s| s.is_transmission) as usize;
        deltas += sample.is_some_and(|s| s.is_delta) as usize;
        misses += sample.is_none() as usize;
        cases.push(format!("roughness {r} ior {n} tint {t:?} thin {thin} inside {inside} wo {wo:?} wi {wi:?} u {u:?} uc {uc}"));
    }
    let body = format!(
        "let l = transmission_new(inf(0u), inf(1u), vec3<f32>(inf(2u), inf(3u), inf(4u)), (inp(14u) & 1u) != 0u, (inp(14u) & 2u) != 0u);
         let wo = vec3<f32>(inf(5u), inf(6u), inf(7u)); let wi = vec3<f32>(inf(8u), inf(9u), inf(10u));
         let ev = transmission_eval(l, wo, wi);
         outf(0u, ev.x); outf(1u, ev.y); outf(2u, ev.z);
         outf(3u, transmission_pdf(l, wo, wi));
         let s = transmission_sample(l, wo, vec2<f32>(inf(11u), inf(12u)), inf(13u));{SAMPLE_OUT}
         let al = transmission_albedo(l);
         outf(15u, al.x); outf(16u, al.y); outf(17u, al.z);"
    );
    let mut words = sample_words();
    words.extend([w(Kind::Value, false), w(Kind::Value, false), w(Kind::Value, false)]);
    let out = h.run_wgsl(&["lobes"], &body, &inputs, 15, words.len());
    println!("DielectricTransmission: {through} samples through, {deltas} delta, {misses} without a sample");
    compare("DielectricTransmission", &out, &want, &words, 4, &cases);
}

/// A material that transmits, every other parameter of the card's stack
/// over its range: glass alone, glass under a coat, glass with a fuzz,
/// partly metal, partly transmitting, thin-walled or solid, with a depth or
/// without, and now and then the five lobes before the diffuse all there.
fn glass(rng: &mut Pcg32) -> Material {
    let full = rng.next_f32() < 0.1;
    let m = Material {
        base_weight: unit(rng, 0.05, 0.7),
        base_color: colour(rng),
        base_metalness: if full { 0.2 + 0.6 * rng.next_f32() } else { unit(rng, 0.7, 0.05) },
        base_diffuse_roughness: unit(rng, 0.3, 0.1),
        metal_ior: ComplexIor {
            n: Spectrum { r: 0.1 + rng.next_f32() * 2.5, g: 0.1 + rng.next_f32() * 2.5, b: 0.1 + rng.next_f32() * 2.5 },
            k: Spectrum { r: rng.next_f32() * 6.0, g: rng.next_f32() * 6.0, b: rng.next_f32() * 6.0 },
        },
        specular_weight: unit(rng, 0.1, 0.6),
        specular_color: tint(rng),
        specular_roughness: roughness(rng),
        specular_ior: ior(rng),
        specular_roughness_anisotropy: if rng.next_f32() < 0.7 { 0.0 } else { rng.next_f32() },
        transmission_weight: if full {
            0.3 + 0.6 * rng.next_f32()
        } else {
            match rng.next_f32() {
                x if x < 0.05 => 0.0,
                x if x < 0.45 => 1.0,
                x if x < 0.55 => 0.9,
                x if x < 0.6 => 0.999 + rng.next_f32() * 9e-4,
                _ => rng.next_f32(),
            }
        },
        transmission_color: colour(rng),
        transmission_tint: tint(rng),
        transmission_depth: if rng.next_f32() < 0.5 { 0.0 } else { rng.next_f32() },
        geometry_thin_walled: rng.next_f32() < 0.3,
        coat_weight: if full { 0.5 + 0.5 * rng.next_f32() } else { unit(rng, 0.6, 0.25) },
        coat_color: tint(rng),
        coat_roughness: roughness(rng),
        coat_ior: 1.0 + rng.next_f32() * 1.5,
        coat_darkening: unit(rng, 0.1, 0.6),
        fuzz_weight: if full { 0.5 + 0.5 * rng.next_f32() } else { unit(rng, 0.8, 0.1) },
        fuzz_color: tint(rng),
        fuzz_roughness: unit(rng, 0.05, 0.05),
        geometry_occlusion: match rng.next_f32() {
            x if x < 0.03 => 0.0,
            x if x < 0.8 => 1.0,
            _ => rng.next_f32(),
        },
        subsurface_weight: 0.0,
        thin_film_weight: 0.0,
        flake_weight: 0.0,
        ..Default::default()
    };
    assert!(GpuMaterial::supports(&m));
    m
}

const MATERIAL_WORDS: usize = std::mem::size_of::<GpuMaterial>() / 4;

/// `openpbr.wgsl`'s GpuMaterial from the first words of the item.
fn material_from_words() -> String {
    let at = |k: usize| format!("inf({k}u)");
    let mut parts: Vec<String> = (0..6).map(|c| format!("vec4<f32>({}, {}, {}, {})", at(4 * c), at(4 * c + 1), at(4 * c + 2), at(4 * c + 3))).collect();
    parts.extend((24..40).map(at));
    for v in [40, 44, 48] {
        parts.push(format!("vec4<f32>({}, {}, {}, {})", at(v), at(v + 1), at(v + 2), at(v + 3)));
    }
    parts.extend((52..55).map(at));
    parts.push("inp(55u)".into());
    parts.extend((56..62).map(at));
    // The edges' radius and the probes' count (a word).
    parts.push(at(62));
    parts.push("inp(63u)".into());
    parts.push(format!("vec4<f32>({}, {}, {}, {})", at(64), at(65), at(66), at(67)));
    format!("GpuMaterial({})", parts.join(", "))
}

/// How the stack is built: face on or to aim, from outside or inside.
fn build(m: &Material, floor: f32, aimed: bool, inside: bool, wo: Vec3) -> OpenPbrBsdf {
    match (aimed, inside) {
        (false, false) => OpenPbrBsdf::new(m, floor),
        (false, true) => OpenPbrBsdf::from_inside(m, floor),
        (true, false) => {
            let mut b = OpenPbrBsdf::to_aim(m, floor);
            b.aim(wo);
            b
        }
        (true, true) => {
            let mut b = OpenPbrBsdf::from_inside_to_aim(m, floor);
            b.aim(wo);
            b
        }
    }
}

/// The stack's words: eval rgb, pdf, the sample's eleven, the reflection's
/// roughness (-1 for none), albedo rgb; `pdf_reflection` at the
/// integrator's seeing share, `eval_selective` diffuse alone and the rest
/// alone, and whether the stack has a mirror-smooth lobe and transmits.
fn stack_words() -> Vec<Word> {
    let mut words = sample_words();
    words.push(w(Kind::Value, true));
    words.extend((0..3).map(|_| w(Kind::Value, false)));
    words.extend((0..7).map(|_| w(Kind::Value, false)));
    words.extend([w(Kind::Flag, false), w(Kind::Flag, false)]);
    words
}

fn seeing_share(roughness: f32) -> f32 {
    let t = ((roughness - 0.5) / (0.8 - 0.5)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

fn the_glass_stack(seed: u64, aimed: bool) {
    let Some(h) = Harness::for_test() else { return };
    assert_eq!(MATERIAL_WORDS, 68);
    let mut rng = Pcg32::new(seed, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    let (mut through, mut deltas, mut misses, mut dropped, mut inside_count) = (0, 0, 0, 0, 0);
    for _ in 0..N {
        let m = glass(&mut rng);
        let floor = if rng.next_f32() < 0.6 { 0.0 } else { rng.next_f32() * 0.5 };
        let inside = rng.next_f32() < 0.4;
        let (wo, wi) = (direction(&mut rng, 0.03), direction(&mut rng, 0.5));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let bsdf = build(&m, floor, aimed, inside, wo);
        let gpu = GpuMaterial::from_material(&m);
        inputs.extend_from_slice(bytemuck::cast_slice::<GpuMaterial, u32>(std::slice::from_ref(&gpu)));
        inputs.extend(bits(&[floor, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y, uc]));
        inputs.push(aimed as u32 | (inside as u32) << 1);

        let eval = bsdf.eval(wo, wi);
        let mut e = vec![eval.r, eval.g, eval.b, bsdf.pdf(wo, wi)];
        let reflection = bsdf.sample_reflection(wo, u, uc);
        push_sample(&mut e, reflection.map(|(s, _)| s));
        e.push(reflection.map_or(0.0, |(_, r)| r.unwrap_or(-1.0)));
        let albedo = bsdf.albedo(wo);
        e.extend([albedo.r, albedo.g, albedo.b]);
        e.push(bsdf.pdf_reflection(wo, wi, seeing_share));
        let diffuse = bsdf.eval_selective(wo, wi, true, false);
        let glossy = bsdf.eval_selective(wo, wi, false, true);
        e.extend([diffuse.r, diffuse.g, diffuse.b, glossy.r, glossy.g, glossy.b]);
        e.push(bsdf.flags().contains(BsdfFlags::DELTA) as u8 as f32);
        e.push(bsdf.flags().contains(BsdfFlags::TRANSMISSION) as u8 as f32);
        want.push(e);
        through += reflection.is_some_and(|(s, _)| s.is_transmission) as usize;
        deltas += reflection.is_some_and(|(s, _)| s.is_delta) as usize;
        misses += reflection.is_none() as usize;
        inside_count += inside as usize;
        // Fuzz, coat, metal, interface and transmission all pushed: the
        // diffuse is the sixth and is dropped.
        let dielectric = 1.0 - m.base_metalness;
        dropped += (m.fuzz_weight > 1e-4
            && m.coat_weight > 1e-4
            && m.base_metalness > 1e-4
            && m.specular_weight * (1.0 - m.transmission_weight) > 1e-4
            && m.transmission_weight > 1e-4
            && dielectric * (1.0 - m.transmission_weight) > 1e-4
            && m.base_weight > 0.0
            && m.geometry_occlusion > 0.0) as usize;
        cases.push(format!("aimed {aimed} inside {inside} floor {floor} wo {wo:?} wi {wi:?} u {u:?} uc {uc} material {gpu:?}"));
    }
    let base = MATERIAL_WORDS;
    let body = format!(
        "let m = {material};
         let rf = inf({base}u);
         let wo = vec3<f32>(inf({}u), inf({}u), inf({}u));
         let wi = vec3<f32>(inf({}u), inf({}u), inf({}u));
         let how = inp({}u);
         var b = openpbr_build(m, rf, (how & 1u) != 0u, (how & 2u) != 0u);
         if ((how & 1u) != 0u) {{ b = openpbr_aim(b, wo); }}
         let ev = openpbr_eval(b, wo, wi);
         outf(0u, ev.x); outf(1u, ev.y); outf(2u, ev.z);
         outf(3u, openpbr_pdf(b, wo, wi));
         let r = openpbr_sample_reflection(b, wo, vec2<f32>(inf({}u), inf({}u)), inf({}u));
         let s = r.sample;{SAMPLE_OUT}
         outf(15u, select(0.0, r.roughness, s.ok));
         let al = openpbr_albedo(b, wo);
         outf(16u, al.x); outf(17u, al.y); outf(18u, al.z);
         outf(19u, openpbr_pdf_reflection(b, wo, wi, 0.5, 0.8));
         let d = openpbr_eval_selective(b, wo, wi, true, false);
         outf(20u, d.x); outf(21u, d.y); outf(22u, d.z);
         let g = openpbr_eval_selective(b, wo, wi, false, true);
         outf(23u, g.x); outf(24u, g.y); outf(25u, g.z);
         outf(26u, select(0.0, 1.0, openpbr_has_delta(b)));
         outf(27u, select(0.0, 1.0, openpbr_transmits(b)));",
        base + 1,
        base + 2,
        base + 3,
        base + 4,
        base + 5,
        base + 6,
        base + 10,
        base + 7,
        base + 8,
        base + 9,
        material = material_from_words(),
    );
    let name = if aimed { "OpenPbrBsdf with glass, aimed" } else { "OpenPbrBsdf with glass" };
    let words = stack_words();
    let out = h.run_wgsl(&["openpbr"], &body, &inputs, base + 11, words.len());
    // The processor's roughness word for no sample is 0 here too.
    let want: Vec<Vec<f32>> = want
        .into_iter()
        .map(|mut e| {
            if e[4] == 0.0 {
                e[15] = 0.0;
            }
            e
        })
        .collect();
    println!(
        "{name}: {through} samples through, {deltas} delta, {misses} without a sample, {inside_count} from inside, {dropped} stacks dropping the diffuse"
    );
    compare(name, &out, &want, &words, 4, &cases);
}

#[test]
fn the_glass_stack_is_the_processors() {
    the_glass_stack(51, false);
}

#[test]
fn the_aimed_glass_stack_is_the_processors() {
    the_glass_stack(52, true);
}
