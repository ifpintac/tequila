//! GGX, Fresnel and the energy tables on the card against
//! `tequila_materials` (`docs/07` R2.4's method): random inputs over the
//! ranges the processor's own tests use, the Rust as the oracle, agreement
//! to 1e-4 relative (1e-6 absolute near zero).

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::energy;
use tequila_materials::fresnel;
use tequila_materials::microfacet::Ggx;
use tequila_render_gpu::harness::Harness;

const N: usize = 20_000;

fn close(card: f32, processor: f32, absolute: f32, relative: f32) -> bool {
    (card - processor).abs() <= absolute + relative * processor.abs() || (card.is_nan() && processor.is_nan())
}

/// `relative` above 1e-6 absolute; a component of a unit vector drawn
/// through sin and cos is held to `absolute` instead (pass it negative to
/// mean the default).
fn check(name: &str, card: &[u32], expected: &[f32], relative: f32, inputs: impl Fn(usize) -> String) {
    let (absolute, relative) = if relative < 0.0 { (-relative, 0.0) } else { (1e-6, relative) };
    let mut wrong = Vec::new();
    for (i, (&c, &e)) in card.iter().zip(expected).enumerate() {
        let c = f32::from_bits(c);
        if !close(c, e, absolute, relative) {
            if wrong.len() < 6 {
                wrong.push(format!("  #{i} ({}): card {c}, processor {e}", inputs(i)));
            }
        }
    }
    assert!(wrong.is_empty(), "{name} differs:\n{}", wrong.join("\n"));
}

/// A direction in the upper hemisphere, now and then a grazing one.
fn direction(rng: &mut Pcg32) -> Vec3 {
    let mu = if rng.next_f32() < 0.1 { rng.next_f32() * 0.02 } else { rng.next_f32().max(1e-3) };
    let phi = std::f32::consts::TAU * rng.next_f32();
    let s = (1.0 - mu * mu).max(0.0).sqrt();
    vec3(s * phi.cos(), s * phi.sin(), mu)
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn ggx_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(1, 1);
    let mut inputs = Vec::new();
    let mut expected = Vec::new();
    let mut cases = Vec::new();
    for _ in 0..N {
        let roughness = rng.next_f32();
        let anisotropy = if rng.next_f32() < 0.5 { 0.0 } else { rng.next_f32() };
        let (wo, wi) = (direction(&mut rng), direction(&mut rng));
        let u = vec2(rng.next_f32(), rng.next_f32());
        let g = Ggx::from_roughness(roughness, anisotropy);
        inputs.extend(bits(&[roughness, anisotropy, wo.x, wo.y, wo.z, wi.x, wi.y, wi.z, u.x, u.y]));
        let wm = (wo + wi).normalize();
        let m = g.sample_visible_normal(wo, u);
        expected.extend([
            g.alpha_x, g.alpha_y, g.d(wm), g.lambda(wo), g.g1(wo), g.g2(wo, wi), g.d_visible(wo, wm), m.x, m.y, m.z,
            g.reflection_pdf(wo, wi),
        ]);
        cases.push(format!("roughness {roughness}, anisotropy {anisotropy}, wo {wo:?}, wi {wi:?}"));
    }
    let out = h.run_wgsl(
        &["microfacet"],
        "let g = ggx_from_roughness(inf(0u), inf(1u));
         let wo = vec3<f32>(inf(2u), inf(3u), inf(4u));
         let wi = vec3<f32>(inf(5u), inf(6u), inf(7u));
         let wm = mf_normalize(wo + wi);
         let m = ggx_sample_visible_normal(g, wo, vec2<f32>(inf(8u), inf(9u)));
         outf(0u, g.alpha_x); outf(1u, g.alpha_y); outf(2u, ggx_d(g, wm)); outf(3u, ggx_lambda(g, wo));
         outf(4u, ggx_g1(g, wo)); outf(5u, ggx_g2(g, wo, wi)); outf(6u, ggx_d_visible(g, wo, wm));
         outf(7u, m.x); outf(8u, m.y); outf(9u, m.z); outf(10u, ggx_reflection_pdf(g, wo, wi));",
        &inputs,
        10,
        11,
    );
    // The visible normal goes through sin and cos, which the card does not
    // round correctly: its components are held to 2e-4 absolute.
    let names = ["alpha_x", "alpha_y", "d", "lambda", "g1", "g2", "d_visible", "m.x", "m.y", "m.z", "reflection_pdf"];
    for (k, name) in names.iter().enumerate() {
        let card: Vec<u32> = out.iter().skip(k).step_by(11).copied().collect();
        let want: Vec<f32> = expected.iter().skip(k).step_by(11).copied().collect();
        let relative = if (7..=9).contains(&k) { -2e-4 } else { 1e-4 };
        check(name, &card, &want, relative, |i| cases[i].clone());
    }
}

#[test]
fn fresnel_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(2, 2);
    let mut inputs = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..N {
        let cos = rng.next_f32() * 2.0 - 1.0;
        let eta = 1.0 + rng.next_f32() * 2.0;
        let (n, k) = (0.1 + rng.next_f32() * 3.0, rng.next_f32() * 8.0);
        let inside = 0.3 + rng.next_f32() * 0.69;
        inputs.extend(bits(&[cos, eta, n, k, inside]));
        expected.extend([
            fresnel::dielectric(cos, eta),
            fresnel::conductor_scalar(cos.abs(), n, k),
            fresnel::dielectric_average(eta),
            fresnel::dielectric_average(inside),
        ]);
    }
    let out = h.run_wgsl(
        &["fresnel"],
        "outf(0u, fresnel_dielectric(inf(0u), inf(1u)));
         outf(1u, fresnel_conductor_scalar(abs(inf(0u)), inf(2u), inf(3u)));
         outf(2u, fresnel_dielectric_average(inf(1u)));
         outf(3u, fresnel_dielectric_average(inf(4u)));",
        &inputs,
        5,
        4,
    );
    for (k, name) in ["dielectric", "conductor", "dielectric_average", "dielectric_average below one"].iter().enumerate() {
        let card: Vec<u32> = out.iter().skip(k).step_by(4).copied().collect();
        let want: Vec<f32> = expected.iter().skip(k).step_by(4).copied().collect();
        check(name, &card, &want, 1e-4, |i| format!("{:?}", &inputs[i * 5..i * 5 + 5].iter().map(|b| f32::from_bits(*b)).collect::<Vec<_>>()));
    }
}

#[test]
fn the_energy_tables_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let table = energy::table();
    let mut rng = Pcg32::new(3, 3);
    let mut inputs = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..N {
        let cos = rng.next_f32();
        let alpha = rng.next_f32();
        let ior = 1.0 + rng.next_f32() * 2.0;
        let f0 = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
        let (e_o, e_i, e_avg) = (rng.next_f32(), rng.next_f32(), rng.next_f32());
        let inside = rng.next_f32() < 0.5;
        inputs.extend(bits(&[cos, alpha, ior, f0.r, f0.g, f0.b, e_o, e_i, e_avg]));
        inputs.push(inside as u32);
        let cd = table.conductor_directional(cos, alpha, f0);
        let ca = table.conductor_average(alpha, f0);
        let ms = energy::multiple_scattering_scale(f0, e_avg);
        expected.extend([
            table.white_directional(cos, alpha),
            table.white_average(alpha),
            cd.g,
            ca.b,
            table.dielectric_directional(cos, alpha, ior),
            table.dielectric_hemispherical(alpha, ior),
            table.interface_directional(cos, alpha, ior, inside),
            energy::multiple_scattering_value(e_o, e_i, e_avg),
            ms.r,
        ]);
    }
    let out = h.run_wgsl(
        &["energy"],
        "let cos = inf(0u); let alpha = inf(1u); let ior = inf(2u);
         let f0 = vec3<f32>(inf(3u), inf(4u), inf(5u));
         outf(0u, energy_white_directional(cos, alpha));
         outf(1u, energy_white_average(alpha));
         outf(2u, energy_conductor_directional(cos, alpha, f0).y);
         outf(3u, energy_conductor_average(alpha, f0).z);
         outf(4u, energy_dielectric_directional(cos, alpha, ior));
         outf(5u, energy_dielectric_hemispherical(alpha, ior));
         outf(6u, energy_interface_directional(cos, alpha, ior, inp(9u) != 0u));
         outf(7u, multiple_scattering_value(inf(6u), inf(7u), inf(8u)));
         outf(8u, multiple_scattering_scale(f0, inf(8u)).x);",
        &inputs,
        10,
        9,
    );
    let names = [
        "white_directional", "white_average", "conductor_directional", "conductor_average", "dielectric_directional",
        "dielectric_hemispherical", "interface_directional", "multiple_scattering_value", "multiple_scattering_scale",
    ];
    for (k, name) in names.iter().enumerate() {
        let card: Vec<u32> = out.iter().skip(k).step_by(9).copied().collect();
        let want: Vec<f32> = expected.iter().skip(k).step_by(9).copied().collect();
        check(name, &card, &want, 1e-4, |i| format!("{:?}", &inputs[i * 10..i * 10 + 9].iter().map(|b| f32::from_bits(*b)).collect::<Vec<_>>()));
    }
}
