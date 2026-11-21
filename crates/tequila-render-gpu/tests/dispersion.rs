//! Dispersion on the card (`dispersion.wgsl`) against
//! `tequila_materials::dispersion`: the index at a wavelength, the colour a
//! wavelength stands for, and the wavelength a draw gives, at random
//! glasses, wavelengths and draws. The index and the draw to the bit but
//! for the card's division, the colour through the card's exponential, an
//! ulp or two from the processor's.

use tequila_core::rng::Pcg32;
use tequila_materials::dispersion;
use tequila_render_gpu::harness::Harness;

const N: usize = 20_000;

/// dispersion.rs's `normalization`, worked out as it works it out: each
/// channel's scale, so the wavelengths average to white.
fn normalization() -> [f32; 3] {
    let raw = |wavelength: f32| {
        let [x, y, z] = dispersion::cie_xyz(wavelength);
        [
            (3.2406 * x - 1.5372 * y - 0.4986 * z).max(0.0),
            (-0.9689 * x + 1.8758 * y + 0.0415 * z).max(0.0),
            (0.0557 * x - 0.2040 * y + 1.0570 * z).max(0.0),
        ]
    };
    let n = 4096;
    let mut sum = [0.0f64; 3];
    for i in 0..n {
        let wavelength = dispersion::SHORTEST + (dispersion::LONGEST - dispersion::SHORTEST) * (i as f32 + 0.5) / n as f32;
        let rgb = raw(wavelength);
        for c in 0..3 {
            sum[c] += rgb[c] as f64;
        }
    }
    [(n as f64 / sum[0]) as f32, (n as f64 / sum[1]) as f32, (n as f64 / sum[2]) as f32]
}

/// The kernel's scale is the processor's, to the bit.
#[test]
fn the_kernels_scale_is_the_processors() {
    let source = tequila_render_gpu::shader::source("dispersion");
    let line = source.lines().find(|l| l.starts_with("const DISPERSION_SCALE: vec3<f32> = vec3<f32>(")).expect("the kernel has it");
    let inner = line.trim_start_matches("const DISPERSION_SCALE: vec3<f32> = vec3<f32>(").trim_end_matches(");");
    let kernel: Vec<f32> = inner.split(',').map(|v| v.trim().parse::<f64>().expect("a number") as f32).collect();
    assert_eq!(kernel, normalization().to_vec(), "the kernel's dispersion scale is not the processor's");
}

#[test]
fn dispersion_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(51, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..N {
        let ior = 1.0 + rng.next_f32() * 2.0;
        let abbe = if rng.next_f32() < 0.05 { 0.0 } else { 5.0 + rng.next_f32() * 95.0 };
        let wavelength = if rng.next_f32() < 0.5 { dispersion::sample(rng.next_f32()) } else { 300.0 + rng.next_f32() * 500.0 };
        let u = if rng.next_f32() < 0.02 { 1.0 } else { rng.next_f32() };
        inputs.extend([ior, abbe, wavelength, u].map(f32::to_bits));
        let weight = dispersion::weight(wavelength);
        want.push([dispersion::ior_at(ior, abbe, wavelength), weight.r, weight.g, weight.b, dispersion::sample(u)]);
        cases.push(format!("ior {ior} abbe {abbe} wavelength {wavelength} u {u}"));
    }
    let body = "let w = dispersion_weight(inf(2u));
                outf(0u, dispersion_ior_at(inf(0u), inf(1u), inf(2u)));
                outf(1u, w.x); outf(2u, w.y); outf(3u, w.z);
                outf(4u, dispersion_sample(inf(3u)));";
    let out = h.run_wgsl(&["dispersion"], body, &inputs, 4, 5);
    let (mut loose, mut worst) = (0usize, [0.0f32; 5]);
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = out[i * 5..(i + 1) * 5].iter().map(|b| f32::from_bits(*b)).collect();
        let mut item_loose = false;
        for k in 0..5 {
            let d = (c[k] - w[k]).abs();
            worst[k] = worst[k].max(d / w[k].abs().max(1e-6));
            // The index and the draw to 2e-7 relative; the colour, through
            // the card's exponential, to 1e-5 relative (1e-7 absolute).
            let bound = if (1..4).contains(&k) { 1e-7 + 1e-5 * w[k].abs() } else { 2e-7 * w[k].abs() };
            if d > bound {
                assert!(d <= 1e-6 + 1e-3 * w[k].abs(), "#{i} word {k}: card {}, processor {} ({})", c[k], w[k], cases[i]);
                item_loose = true;
            }
        }
        loose += item_loose as usize;
    }
    println!("dispersion: {loose} of {N} items looser than the tight bound; worst relative: index {:.1e}, colour {:.1e} {:.1e} {:.1e}, draw {:.1e}", worst[0], worst[1], worst[2], worst[3], worst[4]);
    assert!(loose * 1000 <= N, "{loose} of {N} items outside the tight bound");
}
