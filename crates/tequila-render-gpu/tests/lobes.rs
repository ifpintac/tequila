//! The opaque lobes on the card against `tequila_materials::lobes`: the
//! diffuse (EON), the dielectric specular interface and the conductor, each
//! evaluated, its density taken, and sampled, on random lobes and
//! directions. Values to 1e-4 relative; a sample, which goes through the
//! card's sin and cos, to 2e-4 absolute in direction and 2e-3 relative in
//! value and density; one item in a thousand may be looser, to 5%.

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::bsdf::Bsdf;
use tequila_materials::fresnel::ComplexIor;
use tequila_materials::lobes::{Conductor, DielectricSpecular, OrenNayarDiffuse};
use tequila_materials::microfacet::Ggx;
use tequila_materials::thin_film::Film;
use tequila_render_gpu::harness::Harness;

const N: usize = 20_000;

fn direction(rng: &mut Pcg32) -> Vec3 {
    let mu = if rng.next_f32() < 0.1 { rng.next_f32() * 0.05 + 1e-3 } else { rng.next_f32().max(1e-3) };
    let phi = std::f32::consts::TAU * rng.next_f32();
    let s = (1.0 - mu * mu).max(0.0).sqrt();
    vec3(s * phi.cos(), s * phi.sin(), mu)
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

/// Expected words an item: eval rgb, pdf, sampled (1/0), wi xyz, f rgb, pdf,
/// albedo (one channel or the lobe's scalar).
const OUT: usize = 13;

fn expected(eval: Spectrum, pdf: f32, sample: Option<tequila_materials::bsdf::BsdfSample>, albedo: f32) -> [f32; OUT] {
    let mut e = [eval.r, eval.g, eval.b, pdf, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, albedo];
    if let Some(s) = sample {
        e[4] = 1.0;
        e[5..8].copy_from_slice(&[s.wi.x, s.wi.y, s.wi.z]);
        e[8..11].copy_from_slice(&[s.f.r, s.f.g, s.f.b]);
        e[11] = s.pdf;
    }
    e
}

const BODY_TAIL: &str = "
    let ev = EVAL;
    outf(0u, ev.x); outf(1u, ev.y); outf(2u, ev.z);
    outf(3u, PDF);
    let s = SAMPLE;
    if (s.ok) {
        outf(4u, 1.0);
        outf(5u, s.wi.x); outf(6u, s.wi.y); outf(7u, s.wi.z);
        outf(8u, s.f.x); outf(9u, s.f.y); outf(10u, s.f.z);
        outf(11u, s.pdf);
    } else {
        for (var k = 4u; k < 12u; k++) { outf(k, 0.0); }
    }
    outf(12u, ALBEDO);";

/// Every word within its bound, but for at most one item in a thousand,
/// which must still be within 5% (1e-5 absolute): floating point that is
/// not correctly rounded on the card meets an ill-conditioned spot now and
/// then (a GGX lobe a thousandth wide seen at grazing, a sample at the rim
/// of the disk), where a porting mistake is wrong everywhere.
fn compare(name: &str, card: &[u32], want: &[[f32; OUT]], cases: &[String]) {
    let mut wrong = Vec::new();
    let mut loose = Vec::new();
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = card[i * OUT..(i + 1) * OUT].iter().map(|b| f32::from_bits(*b)).collect();
        let mut item_loose = false;
        for k in 0..OUT {
            let (absolute, relative) = match k {
                5..=7 => (2e-4, 0.0),
                8..=11 => (1e-6, 2e-3),
                _ => (1e-6, 1e-4),
            };
            // A sample that went another way is only a difference if the
            // other side drew one too.
            if (5..=11).contains(&k) && (c[4] == 0.0 || w[4] == 0.0) {
                continue;
            }
            let d = (c[k] - w[k]).abs();
            if d <= absolute + relative * w[k].abs() {
                continue;
            }
            if d <= 1e-5 + 5e-2 * w[k].abs() {
                item_loose = true;
            } else if wrong.len() < 8 {
                wrong.push(format!("  #{i} word {k}: card {}, processor {} ({})", c[k], w[k], cases[i]));
            }
        }
        if item_loose {
            loose.push(i);
        }
    }
    assert!(wrong.is_empty(), "{name} differs:
{}", wrong.join("
"));
    assert!(
        loose.len() * 1000 <= want.len(),
        "{name}: {} items of {} outside the tight bound (first {:?}); at most one in a thousand may be",
        loose.len(),
        want.len(),
        &loose[..loose.len().min(5)]
    );
    println!("{name}: {} of {} items looser than the tight bound", loose.len(), want.len());
}

#[test]
fn the_diffuse_lobe_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(11, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let albedo = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
        let roughness = if rng.next_f32() < 0.3 { 0.0 } else { rng.next_f32() };
        let (wo, wi) = (direction(&mut rng), direction(&mut rng));
        let u = vec2(rng.next_f32(), rng.next_f32());
        let lobe = OrenNayarDiffuse::new(albedo, roughness);
        inputs.extend(bits(&[albedo.r, albedo.g, albedo.b, roughness, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y]));
        want.push(expected(lobe.eval(wo, wi), lobe.pdf(wo, wi), lobe.sample(wo, u, 0.5), lobe.albedo(wo).g));
        cases.push(format!("albedo {albedo:?} roughness {roughness} wo {wo:?} wi {wi:?}"));
    }
    let body = format!(
        "let l = diffuse_new(vec3<f32>(inf(0u), inf(1u), inf(2u)), inf(3u));
         let wo = vec3<f32>(inf(4u), inf(5u), inf(6u)); let wi = vec3<f32>(inf(7u), inf(8u), inf(9u));{}",
        BODY_TAIL
            .replace("EVAL", "diffuse_eval(l, wo, wi)")
            .replace("PDF", "diffuse_pdf(l, wo, wi)")
            .replace("SAMPLE", "diffuse_sample(l, wo, vec2<f32>(inf(10u), inf(11u)))")
            .replace("ALBEDO", "l.albedo.y")
    );
    let out = h.run_wgsl(&["lobes"], &body, &inputs, 12, OUT);
    compare("OrenNayarDiffuse", &out, &want, &cases);
}

#[test]
fn the_specular_interface_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(12, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let roughness = if rng.next_f32() < 0.1 { 0.0 } else { rng.next_f32() };
        let ior = 1.0 + rng.next_f32() * 1.5;
        let weight = rng.next_f32();
        let (wo, wi) = (direction(&mut rng), direction(&mut rng));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let lobe = DielectricSpecular::new(roughness, ior).with_weight(weight);
        inputs.extend(bits(&[roughness, ior, weight, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y, uc]));
        want.push(expected(lobe.eval(wo, wi), lobe.pdf(wo, wi), lobe.sample(wo, u, uc), lobe.directional_albedo(wo)));
        cases.push(format!("roughness {roughness} ior {ior} weight {weight} wo {wo:?} wi {wi:?}"));
    }
    let body = format!(
        "var l = specular_new(inf(0u), inf(1u)); l.weight = clamp(inf(2u), 0.0, 1.0);
         let wo = vec3<f32>(inf(3u), inf(4u), inf(5u)); let wi = vec3<f32>(inf(6u), inf(7u), inf(8u));{}",
        BODY_TAIL
            .replace("EVAL", "specular_eval(l, wo, wi)")
            .replace("PDF", "specular_pdf(l, wo, wi)")
            .replace("SAMPLE", "specular_sample(l, wo, vec2<f32>(inf(9u), inf(10u)), inf(11u))")
            .replace("ALBEDO", "specular_directional_albedo(l, wo)")
    );
    let out = h.run_wgsl(&["lobes"], &body, &inputs, 12, OUT);
    compare("DielectricSpecular", &out, &want, &cases);
}

#[test]
fn the_conductor_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(13, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let roughness = if rng.next_f32() < 0.1 { 0.0 } else { rng.next_f32() };
        let anisotropy = if rng.next_f32() < 0.5 { 0.0 } else { rng.next_f32() };
        let n = Spectrum { r: 0.1 + rng.next_f32() * 2.5, g: 0.1 + rng.next_f32() * 2.5, b: 0.1 + rng.next_f32() * 2.5 };
        let k = Spectrum { r: rng.next_f32() * 6.0, g: rng.next_f32() * 6.0, b: rng.next_f32() * 6.0 };
        let tint = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
        let (wo, wi) = (direction(&mut rng), direction(&mut rng));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let lobe = Conductor::new(Ggx::from_roughness(roughness, anisotropy), ComplexIor { n, k }).with_tint(tint);
        inputs.extend(bits(&[
            roughness, anisotropy, n.r, n.g, n.b, k.r, k.g, k.b, tint.r, tint.g, tint.b, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x,
            u.y, uc,
        ]));
        want.push(expected(lobe.eval(wo, wi), lobe.pdf(wo, wi), lobe.sample(wo, u, uc), lobe.albedo(wo).r));
        cases.push(format!("roughness {roughness} anisotropy {anisotropy} n {n:?} k {k:?} wo {wo:?} wi {wi:?}"));
    }
    let body = format!(
        "let l = Conductor(ggx_from_roughness(inf(0u), inf(1u)), vec3<f32>(inf(2u), inf(3u), inf(4u)),
                           vec3<f32>(inf(5u), inf(6u), inf(7u)), vec3<f32>(inf(8u), inf(9u), inf(10u)), vec4<f32>(0.0));
         let wo = vec3<f32>(inf(11u), inf(12u), inf(13u)); let wi = vec3<f32>(inf(14u), inf(15u), inf(16u));{}",
        BODY_TAIL
            .replace("EVAL", "conductor_eval(l, wo, wi)")
            .replace("PDF", "conductor_pdf(l, wo, wi)")
            .replace("SAMPLE", "conductor_sample(l, wo, vec2<f32>(inf(17u), inf(18u)), inf(19u))")
            .replace("ALBEDO", "conductor_albedo(l, wo).x")
    );
    let out = h.run_wgsl(&["lobes"], &body, &inputs, 20, OUT);
    compare("Conductor", &out, &want, &cases);
}

/// A thin film (`thin_film.rs`'s `Film`) over its whole range: now and then
/// none (a weight at or below 1e-4, or no thickness), else any weight, from
/// nothing to 2 um thick, of index 1 to 3.
fn film(rng: &mut Pcg32) -> Film {
    Film {
        weight: match rng.next_f32() {
            x if x < 0.08 => rng.next_f32() * 1e-4,
            x if x < 0.5 => 1.0,
            _ => rng.next_f32(),
        },
        thickness: if rng.next_f32() < 0.05 { 0.0 } else { rng.next_f32() * 2000.0 },
        ior: 1.0 + rng.next_f32() * 2.0,
    }
}

/// The film's reflectance alone, over a dielectric and over a metal, at
/// every angle: `Film::over_dielectric` and `Film::over_conductor`, the
/// Airy sum at 600, 550 and 450 nm blended with the bare interface.
#[test]
fn a_thin_films_reflectance_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(14, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let f = film(&mut rng);
        let cos = if rng.next_f32() < 0.1 { rng.next_f32() * 0.02 } else { rng.next_f32() };
        let ior = 1.0 + rng.next_f32() * 1.5;
        let n = Spectrum { r: 0.1 + rng.next_f32() * 2.5, g: 0.1 + rng.next_f32() * 2.5, b: 0.1 + rng.next_f32() * 2.5 };
        let k = Spectrum { r: rng.next_f32() * 6.0, g: rng.next_f32() * 6.0, b: rng.next_f32() * 6.0 };
        inputs.extend(bits(&[f.weight, f.thickness, f.ior, cos, ior, n.r, n.g, n.b, k.r, k.g, k.b]));
        let (d, c) = (f.over_dielectric(cos, ior), f.over_conductor(cos, ComplexIor { n, k }));
        let mut e = [0.0f32; OUT];
        e[..6].copy_from_slice(&[d.r, d.g, d.b, c.r, c.g, c.b]);
        want.push(e);
        cases.push(format!("film {f:?} cos {cos} ior {ior} n {n:?} k {k:?}"));
    }
    let body = "let f = vec4<f32>(inf(0u), inf(1u), inf(2u), 0.0); let c = inf(3u); let ior = inf(4u);
         let n = vec3<f32>(inf(5u), inf(6u), inf(7u)); let k = vec3<f32>(inf(8u), inf(9u), inf(10u));
         var d = vec3<f32>(fresnel_dielectric(c, ior));
         var m = fresnel_conductor(c, n, k);
         if (film_on(f)) { d = film_over(d, c, f, vec3<f32>(max(ior, 1.0)), vec3<f32>(0.0)); m = film_over(m, c, f, n, -k); }
         outf(0u, d.x); outf(1u, d.y); outf(2u, d.z); outf(3u, m.x); outf(4u, m.y); outf(5u, m.z);
         for (var w = 6u; w < 13u; w++) { outf(w, 0.0); }";
    let out = h.run_wgsl(&["lobes"], body, &inputs, 11, OUT);
    compare("Film", &out, &want, &cases);
}

#[test]
fn the_specular_interface_under_a_film_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(15, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let roughness = if rng.next_f32() < 0.2 { 0.0 } else { rng.next_f32() };
        let ior = 1.0 + rng.next_f32() * 1.5;
        let weight = rng.next_f32();
        let f = film(&mut rng);
        let (wo, wi) = (direction(&mut rng), direction(&mut rng));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let lobe = DielectricSpecular::new(roughness, ior).with_weight(weight).with_film(Some(f));
        inputs.extend(bits(&[roughness, ior, weight, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y, uc, f.weight, f.thickness, f.ior]));
        want.push(expected(lobe.eval(wo, wi), lobe.pdf(wo, wi), lobe.sample(wo, u, uc), lobe.directional_albedo(wo)));
        cases.push(format!("roughness {roughness} ior {ior} weight {weight} film {f:?} wo {wo:?} wi {wi:?}"));
    }
    let body = format!(
        "var l = specular_new(inf(0u), inf(1u)); l.weight = clamp(inf(2u), 0.0, 1.0);
         l.film = vec4<f32>(inf(12u), inf(13u), inf(14u), 0.0);
         let wo = vec3<f32>(inf(3u), inf(4u), inf(5u)); let wi = vec3<f32>(inf(6u), inf(7u), inf(8u));{}",
        BODY_TAIL
            .replace("EVAL", "specular_eval(l, wo, wi)")
            .replace("PDF", "specular_pdf(l, wo, wi)")
            .replace("SAMPLE", "specular_sample(l, wo, vec2<f32>(inf(9u), inf(10u)), inf(11u))")
            .replace("ALBEDO", "specular_directional_albedo(l, wo)")
    );
    let out = h.run_wgsl(&["lobes"], &body, &inputs, 15, OUT);
    compare("DielectricSpecular under a film", &out, &want, &cases);
}

#[test]
fn the_conductor_under_a_film_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(16, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let roughness = if rng.next_f32() < 0.2 { 0.0 } else { rng.next_f32() };
        let anisotropy = if rng.next_f32() < 0.5 { 0.0 } else { rng.next_f32() };
        let n = Spectrum { r: 0.1 + rng.next_f32() * 2.5, g: 0.1 + rng.next_f32() * 2.5, b: 0.1 + rng.next_f32() * 2.5 };
        let k = Spectrum { r: rng.next_f32() * 6.0, g: rng.next_f32() * 6.0, b: rng.next_f32() * 6.0 };
        let tint = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
        let f = film(&mut rng);
        let (wo, wi) = (direction(&mut rng), direction(&mut rng));
        let (u, uc) = (vec2(rng.next_f32(), rng.next_f32()), rng.next_f32());
        let lobe = Conductor::new(Ggx::from_roughness(roughness, anisotropy), ComplexIor { n, k }).with_tint(tint).with_film(Some(f));
        inputs.extend(bits(&[
            roughness, anisotropy, n.r, n.g, n.b, k.r, k.g, k.b, tint.r, tint.g, tint.b, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x,
            u.y, uc, f.weight, f.thickness, f.ior,
        ]));
        want.push(expected(lobe.eval(wo, wi), lobe.pdf(wo, wi), lobe.sample(wo, u, uc), lobe.albedo(wo).r));
        cases.push(format!("roughness {roughness} anisotropy {anisotropy} n {n:?} k {k:?} film {f:?} wo {wo:?} wi {wi:?}"));
    }
    let body = format!(
        "let l = Conductor(ggx_from_roughness(inf(0u), inf(1u)), vec3<f32>(inf(2u), inf(3u), inf(4u)),
                           vec3<f32>(inf(5u), inf(6u), inf(7u)), vec3<f32>(inf(8u), inf(9u), inf(10u)),
                           vec4<f32>(inf(20u), inf(21u), inf(22u), 0.0));
         let wo = vec3<f32>(inf(11u), inf(12u), inf(13u)); let wi = vec3<f32>(inf(14u), inf(15u), inf(16u));{}",
        BODY_TAIL
            .replace("EVAL", "conductor_eval(l, wo, wi)")
            .replace("PDF", "conductor_pdf(l, wo, wi)")
            .replace("SAMPLE", "conductor_sample(l, wo, vec2<f32>(inf(17u), inf(18u)), inf(19u))")
            .replace("ALBEDO", "conductor_albedo(l, wo).x")
    );
    let out = h.run_wgsl(&["lobes"], &body, &inputs, 23, OUT);
    compare("Conductor under a film", &out, &want, &cases);
}
