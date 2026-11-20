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
                           vec3<f32>(inf(5u), inf(6u), inf(7u)), vec3<f32>(inf(8u), inf(9u), inf(10u)));
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
