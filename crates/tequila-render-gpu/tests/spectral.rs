//! Spectral paths on the card (`spectral.wgsl`) against
//! `tequila_render_core::spectral`: a sample's four wavelengths and what the
//! film makes of each, how luminous it finds them, and every colour read at
//! them - a reflectance, an unbounded colour, the sigmoid polynomial the
//! table gives - at random draws and colours: greys, saturated colours,
//! weights above one, lights far above it, nought. The tables are the
//! processor's own (`transport::spectral_tables`), bound where the
//! transport finds them.

use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::spectral::{self, Sampled, Wavelengths};
use tequila_render_gpu::harness::Harness;

const N: usize = 20_000;
const OUT: usize = 29;

fn colour(rng: &mut Pcg32) -> Spectrum {
    let c = Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() };
    match rng.next_f32() {
        x if x < 0.05 => Spectrum::ZERO,
        x if x < 0.15 => Spectrum::splat(c.r),
        x if x < 0.2 => Spectrum { r: c.r, g: c.r, b: c.b },
        x if x < 0.3 => c * (1.0 + 9.0 * rng.next_f32()),
        x if x < 0.35 => c * 1000.0,
        x if x < 0.4 => c * 1e-3,
        x if x < 0.45 => Spectrum { r: c.r, g: 0.0, b: 0.0 },
        _ => c,
    }
}

#[test]
fn spectral_paths_are_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let tables = tequila_render_gpu::transport::spectral_tables();
    let mut rng = Pcg32::new(61, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let u = if rng.next_f32() < 0.02 { 0.0 } else { rng.next_f32() };
        let c = colour(&mut rng);
        // And nought for cy_zero, which the card must not see is.
        inputs.extend([u, c.r, c.g, c.b, 0.0].map(f32::to_bits));
        let w = Wavelengths::new(u);
        let mut e = Vec::with_capacity(OUT);
        e.extend(w.lambda);
        for i in 0..4 {
            let mut one = Sampled::ZERO;
            one.0[i] = 1.0;
            let f = w.to_rgb(one);
            e.extend([f.r, f.g, f.b]);
        }
        e.push(w.luminance(false));
        e.push(w.luminance(true));
        e.extend(w.reflectance(c).0);
        e.extend(w.unbounded(c).0);
        e.extend(spectral::table().sigmoid(c.to_array()).coefficients);
        want.push(e);
        cases.push(format!("u {u} colour {c:?}"));
    }
    let body = "cy_zero = inp(4u);
        spectral_base = 0u;
        spectral_begin(inf(0u));
        let rgb = vec3<f32>(inf(1u), inf(2u), inf(3u));
        outf(0u, spectral_lambda.x); outf(1u, spectral_lambda.y); outf(2u, spectral_lambda.z); outf(3u, spectral_lambda.w);
        let f0 = spectral_rgb(vec4<f32>(1.0, 0.0, 0.0, 0.0));
        let f1 = spectral_rgb(vec4<f32>(0.0, 1.0, 0.0, 0.0));
        let f2 = spectral_rgb(vec4<f32>(0.0, 0.0, 1.0, 0.0));
        let f3 = spectral_rgb(vec4<f32>(0.0, 0.0, 0.0, 1.0));
        outf(4u, f0.x); outf(5u, f0.y); outf(6u, f0.z);
        outf(7u, f1.x); outf(8u, f1.y); outf(9u, f1.z);
        outf(10u, f2.x); outf(11u, f2.y); outf(12u, f2.z);
        outf(13u, f3.x); outf(14u, f3.y); outf(15u, f3.z);
        outf(16u, spectral_hue(false)); outf(17u, spectral_hue(true));
        let r = spectral_reflectance(rgb);
        outf(18u, r.x); outf(19u, r.y); outf(20u, r.z); outf(21u, r.w);
        let b = spectral_unbounded(rgb);
        outf(22u, b.x); outf(23u, b.y); outf(24u, b.z); outf(25u, b.w);
        let s = spectral_sigmoid_of(rgb);
        outf(26u, s.x); outf(27u, s.y); outf(28u, s.z);";
    let out = h.run_wgsl_with(&["spectral"], body, &inputs, 5, OUT, &[(16, &tables)]);
    let mut loose = Vec::new();
    let mut detail = Vec::new();
    let mut worst = [0.0f64; OUT];
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = out[i * OUT..(i + 1) * OUT].iter().map(|b| f32::from_bits(*b)).collect();
        let mut item_loose = false;
        for k in 0..OUT {
            let (a, b) = (c[k], w[k]);
            if a == b || (a.is_infinite() && b.is_infinite() && a.signum() == b.signum()) {
                continue;
            }
            let d = (a - b).abs();
            worst[k] = worst[k].max((d / b.abs().max(1e-6)) as f64);
            // A wavelength to an ulp or two of its place in its bin, the
            // film and the colours to 1e-5 relative (1e-6 absolute); the
            // table's coefficients, which swing wide at the dark end, to
            // 1e-5 relative (1e-4 absolute).
            let bound = match k {
                0..=3 => 2e-7 * b.abs(),
                26..=28 => 1e-4 + 1e-5 * b.abs(),
                _ => 1e-6 + 1e-5 * b.abs(),
            };
            if d > bound {
                if detail.len() < 12 {
                    detail.push(format!("  #{i} word {k}: card {a}, processor {b} ({})", cases[i]));
                }
                assert!(d <= 1e-4 + 1e-2 * b.abs(), "#{i} word {k}: card {a}, processor {b} ({})", cases[i]);
                item_loose = true;
            }
        }
        if item_loose {
            loose.push(i);
        }
    }
    println!(
        "spectral: {} of {N} items looser than the tight bound (first {:?}); worst relative: wavelengths {:.1e}, film {:.1e}, hue {:.1e}, reflectance {:.1e}, unbounded {:.1e}, coefficients {:.1e}",
        loose.len(),
        loose.first().map(|&i| &cases[i]),
        worst[0..4].iter().cloned().fold(0.0, f64::max),
        worst[4..16].iter().cloned().fold(0.0, f64::max),
        worst[16..18].iter().cloned().fold(0.0, f64::max),
        worst[18..22].iter().cloned().fold(0.0, f64::max),
        worst[22..26].iter().cloned().fold(0.0, f64::max),
        worst[26..29].iter().cloned().fold(0.0, f64::max),
    );
    println!("{}", detail.join("
"));
    assert!(loose.len() * 1000 <= N, "{} of {N} items outside the tight bound", loose.len());
}
