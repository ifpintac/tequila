//! The fuzz lobe and the OpenPBR layer stack on the card against
//! `tequila_materials`: `Sheen` on its own, then `OpenPbrBsdf` for random
//! materials in the part of the stack the card has so far - fuzz, coat,
//! metal, dielectric specular and the diffuse substrate, with base weight
//! and occlusion, transmission, subsurface, film and flakes off - built
//! face on (`OpenPbrBsdf::new`) and aimed (`to_aim` then `aim`), at a
//! roughness floor of zero and above it. Each is evaluated, its density
//! taken, sampled and its albedo read, through the `Bsdf` trait.
//!
//! The tolerances are `tests/lobes.rs`'s: values to 1e-4 relative (1e-6
//! absolute), a sampled direction to 2e-4 absolute, a sample's value and
//! density to 2e-3 relative, and one item in a thousand may be looser, to
//! 5% (1e-5 absolute).

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::bsdf::{Bsdf, BsdfSample};
use tequila_materials::fresnel::ComplexIor;
use tequila_materials::lobes::{OpenPbrBsdf, Sheen};
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

/// How a word is held: a value, an exact flag, a sampled direction, or a
/// sample's value or density; `sampled` words are compared only where both
/// sides drew a sample.
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

/// Every word within its bound, but for at most one item in a thousand,
/// which must still be within 5% (1e-5 absolute). `ok` is the word saying
/// whether a sample was drawn. Returns how many items were looser than the
/// tight bound.
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

fn push_sample(e: &mut Vec<f32>, sample: Option<BsdfSample>) {
    match sample {
        Some(s) => e.extend([1.0, s.wi.x, s.wi.y, s.wi.z, s.f.r, s.f.g, s.f.b, s.pdf]),
        None => e.extend([0.0; 8]),
    }
}

const SAMPLE_OUT: &str = "
    if (s.ok) {
        outf(4u, 1.0);
        outf(5u, s.wi.x); outf(6u, s.wi.y); outf(7u, s.wi.z);
        outf(8u, s.f.x); outf(9u, s.f.y); outf(10u, s.f.z);
        outf(11u, s.pdf);
    } else {
        for (var k = 4u; k < 12u; k++) { outf(k, 0.0); }
    }";

/// Sheen's words: eval rgb, pdf, sampled, wi, f, pdf, albedo rgb,
/// transmittance seen from `wo`, hemispherical albedo.
const SHEEN_WORDS: [Word; 17] = [
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Flag, false),
    w(Kind::Direction, true),
    w(Kind::Direction, true),
    w(Kind::Direction, true),
    w(Kind::Sampled, true),
    w(Kind::Sampled, true),
    w(Kind::Sampled, true),
    w(Kind::Sampled, true),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
];

#[test]
fn the_sheen_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(21, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let color = match rng.next_f32() {
            x if x < 0.05 => Spectrum::ZERO,
            x if x < 0.25 => Spectrum::ONE,
            _ => Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() },
        };
        let roughness = if rng.next_f32() < 0.05 { 0.0 } else { rng.next_f32() };
        let weight = match rng.next_f32() {
            x if x < 0.05 => 0.0,
            x if x < 0.45 => 1.0,
            _ => rng.next_f32(),
        };
        let (wo, wi) = (direction(&mut rng, 0.1), direction(&mut rng, 0.15));
        let u = vec2(rng.next_f32(), rng.next_f32());
        let lobe = Sheen::new(color, roughness, weight);
        inputs.extend(bits(&[color.r, color.g, color.b, roughness, weight, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y]));
        let eval = lobe.eval(wo, wi);
        let mut e = vec![eval.r, eval.g, eval.b, lobe.pdf(wo, wi)];
        push_sample(&mut e, lobe.sample(wo, u, 0.5));
        let albedo = lobe.albedo(wo);
        e.extend([albedo.r, albedo.g, albedo.b, lobe.transmittance(wo.z), lobe.hemispherical_albedo()]);
        want.push(e);
        cases.push(format!("color {color:?} roughness {roughness} weight {weight} wo {wo:?} wi {wi:?} u {u:?}"));
    }
    let body = format!(
        "let l = sheen_new(vec3<f32>(inf(0u), inf(1u), inf(2u)), inf(3u), inf(4u));
         let wo = vec3<f32>(inf(5u), inf(6u), inf(7u)); let wi = vec3<f32>(inf(8u), inf(9u), inf(10u));
         let ev = sheen_eval(l, wo, wi);
         outf(0u, ev.x); outf(1u, ev.y); outf(2u, ev.z);
         outf(3u, sheen_pdf(l, wo, wi));
         let s = sheen_sample(l, wo, vec2<f32>(inf(11u), inf(12u)));{SAMPLE_OUT}
         let al = sheen_albedo(l, wo);
         outf(12u, al.x); outf(13u, al.y); outf(14u, al.z);
         outf(15u, sheen_transmittance(l, wo.z));
         outf(16u, sheen_hemispherical_albedo(l));"
    );
    let out = h.run_wgsl(&["sheen"], &body, &inputs, 13, SHEEN_WORDS.len());
    compare("Sheen", &out, &want, &SHEEN_WORDS, 4, &cases);
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

/// White half the time, as tints mostly are.
fn tint(rng: &mut Pcg32) -> Spectrum {
    if rng.next_f32() < 0.5 { Spectrum::ONE } else { colour(rng) }
}

/// Mirror-smooth a tenth of the time, nearly so another tenth (a delta
/// lobe, or a GGX lobe a thousandth wide), else anywhere.
fn roughness(rng: &mut Pcg32) -> f32 {
    match rng.next_f32() {
        x if x < 0.1 => 0.0,
        x if x < 0.2 => rng.next_f32() * 0.05,
        _ => rng.next_f32(),
    }
}

/// A material in the card's part of the stack, every parameter over its
/// range: coat over metal, fuzz over diffuse, half metals, zero weights
/// and their mixes all come up often.
fn material(rng: &mut Pcg32) -> Material {
    let m = Material {
        base_weight: unit(rng, 0.08, 0.6),
        base_color: colour(rng),
        base_metalness: unit(rng, 0.4, 0.25),
        base_diffuse_roughness: unit(rng, 0.3, 0.1),
        metal_ior: ComplexIor {
            n: Spectrum { r: 0.1 + rng.next_f32() * 2.5, g: 0.1 + rng.next_f32() * 2.5, b: 0.1 + rng.next_f32() * 2.5 },
            k: Spectrum { r: rng.next_f32() * 6.0, g: rng.next_f32() * 6.0, b: rng.next_f32() * 6.0 },
        },
        specular_weight: unit(rng, 0.1, 0.5),
        specular_color: tint(rng),
        specular_roughness: roughness(rng),
        specular_ior: 1.0 + rng.next_f32() * 1.5,
        specular_roughness_anisotropy: if rng.next_f32() < 0.6 { 0.0 } else { rng.next_f32() },
        coat_weight: unit(rng, 0.45, 0.25),
        coat_color: tint(rng),
        coat_roughness: roughness(rng),
        coat_ior: 1.0 + rng.next_f32() * 1.5,
        coat_darkening: unit(rng, 0.1, 0.6),
        fuzz_weight: unit(rng, 0.5, 0.2),
        fuzz_color: tint(rng),
        fuzz_roughness: unit(rng, 0.05, 0.05),
        geometry_occlusion: match rng.next_f32() {
            x if x < 0.03 => 0.0,
            x if x < 0.8 => 1.0,
            _ => rng.next_f32(),
        },
        // Off: the processor's stack is then the card's.
        transmission_weight: 0.0,
        subsurface_weight: 0.0,
        thin_film_weight: 0.0,
        flake_weight: 0.0,
        ..Default::default()
    };
    assert!(GpuMaterial::supports(&m));
    m
}

/// The stack's words: eval rgb, pdf, sampled, wi, f, pdf, delta, the
/// reflection's roughness (-1 for none), albedo rgb.
const STACK_WORDS: [Word; 17] = [
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Flag, false),
    w(Kind::Direction, true),
    w(Kind::Direction, true),
    w(Kind::Direction, true),
    w(Kind::Sampled, true),
    w(Kind::Sampled, true),
    w(Kind::Sampled, true),
    w(Kind::Sampled, true),
    w(Kind::Flag, true),
    w(Kind::Value, true),
    w(Kind::Value, false),
    w(Kind::Value, false),
    w(Kind::Value, false),
];

const MATERIAL_WORDS: usize = std::mem::size_of::<GpuMaterial>() / 4;

/// `openpbr.wgsl`'s GpuMaterial from the first words of the item.
fn material_from_words() -> String {
    let at = |k: usize| format!("inf({k}u)");
    let mut parts: Vec<String> = (0..6).map(|c| format!("vec4<f32>({}, {}, {}, {})", at(4 * c), at(4 * c + 1), at(4 * c + 2), at(4 * c + 3))).collect();
    parts.extend((24..38).map(at));
    parts.push(format!("vec2<f32>({}, {})", at(38), at(39)));
    format!("GpuMaterial({})", parts.join(", "))
}

fn the_stack(seed: u64, aimed: bool) {
    let Some(h) = Harness::for_test() else { return };
    assert_eq!(MATERIAL_WORDS, 40);
    let mut rng = Pcg32::new(seed, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    let (mut deltas, mut misses, mut floored) = (0, 0, 0);
    for _ in 0..N {
        let m = material(&mut rng);
        let floor = if rng.next_f32() < 0.6 { 0.0 } else { rng.next_f32() * 0.5 };
        let (wo, wi) = (direction(&mut rng, 0.03), direction(&mut rng, 0.05));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let bsdf = if aimed {
            let mut bsdf = OpenPbrBsdf::to_aim(&m, floor);
            bsdf.aim(wo);
            bsdf
        } else {
            OpenPbrBsdf::new(&m, floor)
        };
        let gpu = GpuMaterial::from_material(&m);
        inputs.extend_from_slice(bytemuck::cast_slice::<GpuMaterial, u32>(std::slice::from_ref(&gpu)));
        inputs.extend(bits(&[floor, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y, uc]));
        inputs.push(aimed as u32);

        let eval = bsdf.eval(wo, wi);
        let mut e = vec![eval.r, eval.g, eval.b, bsdf.pdf(wo, wi)];
        let sample = Bsdf::sample(&bsdf, wo, u, uc);
        push_sample(&mut e, sample);
        let reflection = bsdf.sample_reflection(wo, u, uc);
        e.push(sample.map_or(0.0, |s| s.is_delta as u8 as f32));
        e.push(reflection.and_then(|(_, r)| r).unwrap_or(-1.0));
        let albedo = bsdf.albedo(wo);
        e.extend([albedo.r, albedo.g, albedo.b]);
        want.push(e);
        deltas += sample.is_some_and(|s| s.is_delta) as usize;
        misses += sample.is_none() as usize;
        floored += (floor > 0.0) as usize;
        cases.push(format!("aimed {aimed} floor {floor} wo {wo:?} wi {wi:?} u {u:?} uc {uc} material {gpu:?}"));
    }
    let base = MATERIAL_WORDS;
    let body = format!(
        "let m = {material};
         let rf = inf({base}u);
         let wo = vec3<f32>(inf({}u), inf({}u), inf({}u));
         let wi = vec3<f32>(inf({}u), inf({}u), inf({}u));
         var b: OpenPbr;
         if (inp({}u) != 0u) {{ b = openpbr_aim(openpbr_to_aim(m, rf), wo); }} else {{ b = openpbr_new(m, rf); }}
         let ev = openpbr_eval(b, wo, wi);
         outf(0u, ev.x); outf(1u, ev.y); outf(2u, ev.z);
         outf(3u, openpbr_pdf(b, wo, wi));
         let r = openpbr_sample_reflection(b, wo, vec2<f32>(inf({}u), inf({}u)), inf({}u));
         let s = r.sample;{SAMPLE_OUT}
         outf(12u, select(0.0, 1.0, s.ok && s.is_delta));
         outf(13u, select(0.0, r.roughness, s.ok));
         let al = openpbr_albedo(b, wo);
         outf(14u, al.x); outf(15u, al.y); outf(16u, al.z);",
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
    let name = if aimed { "OpenPbrBsdf aimed" } else { "OpenPbrBsdf" };
    let out = h.run_wgsl(&["openpbr"], &body, &inputs, base + 11, STACK_WORDS.len());
    // The processor's roughness word for no sample is 0 here too.
    let want: Vec<Vec<f32>> = want
        .into_iter()
        .map(|mut e| {
            if e[4] == 0.0 {
                e[13] = 0.0;
            }
            e
        })
        .collect();
    println!("{name}: {deltas} delta samples, {misses} without a sample, {floored} at a roughness floor above zero");
    compare(name, &out, &want, &STACK_WORDS, 4, &cases);
}

#[test]
fn the_stack_is_the_processors() {
    the_stack(31, false);
}

#[test]
fn the_aimed_stack_is_the_processors() {
    the_stack(32, true);
}
