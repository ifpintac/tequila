//! The card draws the processor's samples, bit for bit (`docs/07` R2.3).
//!
//! Same samples, same paths: a card picture can only be compared with the
//! processor's at a seed (R2.6) if both draw the same numbers for every
//! pixel, sample and dimension. `sampler.wgsl`, `sobol.wgsl` and `rng.wgsl`
//! are `tequila_core`'s sampler, Sobol′ and PCG32 (in 32-bit halves), and
//! the Sobol′ byte tables are the processor's own, uploaded.

use tequila_core::math::Vec2;
use tequila_core::rng::{hash_many, Pcg32};
use tequila_core::sampler::Sampler;
use tequila_core::sobol;
use tequila_render_gpu::harness::Harness;

/// The raw 32 bits the processor's sampler turns into its first draw of
/// `dimension`.
fn raw(pixel_x: u32, pixel_y: u32, sample: u32, scene_seed: u32, dimension: u32) -> u32 {
    let seed = hash_many(&[scene_seed, pixel_x, pixel_y]);
    if dimension < 64 {
        sobol::sample_u32(sample, dimension, seed)
    } else {
        Pcg32::new(hash_many(&[seed, sample, 0x5f3d]) as u64, hash_many(&[seed, sample, 0xa17c]) as u64).next_u32()
    }
}

#[test]
fn a_million_draws_are_the_processors_bit_for_bit() {
    let Some(harness) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(20261120, 7);
    const N: usize = 1_000_000;
    let mut inputs = Vec::with_capacity(N * 5);
    for _ in 0..N {
        let dimension = rng.next_u32() % 96;
        inputs.extend_from_slice(&[rng.next_u32() % 8192, rng.next_u32() % 8192, rng.next_u32(), rng.next_u32(), dimension]);
    }
    // The raw first draw, then the second draw as the sampler gives it: past
    // the Sobol′ limit the second is the PCG32 stream's next number.
    let out = harness.run_wgsl(
        &["sampler"],
        "var s = sampler_new(inp(0u), inp(1u), inp(2u), inp(3u));
         out(0u, sampler_get_u32(&s, inp(4u)));
         outf(1u, sampler_get(&s, inp(4u)));",
        &inputs,
        5,
        2,
    );
    let mut wrong = Vec::new();
    for (i, item) in inputs.chunks(5).enumerate() {
        let &[x, y, sample, seed, dimension] = item else { unreachable!() };
        let mut sampler = Sampler::new(x, y, sample, seed);
        let _ = sampler.get(dimension);
        let expected = (raw(x, y, sample, seed, dimension), sampler.get(dimension).to_bits());
        let got = (out[2 * i], out[2 * i + 1]);
        if got != expected && wrong.len() < 8 {
            wrong.push(format!("pixel ({x}, {y}) sample {sample} seed {seed} dimension {dimension}: card {got:08x?}, processor {expected:08x?}"));
        }
    }
    assert!(wrong.is_empty(), "the card's samples differ:\n{}", wrong.join("\n"));
}

/// A path's worth of draws, in a path's order: ten bounces of direction,
/// lobe, roulette, light and medium, which runs past the Sobol′ dimensions
/// into the PCG32 stream at the seventh bounce.
#[test]
fn a_paths_draws_come_in_the_processors_order() {
    let Some(harness) = Harness::for_test() else { return };
    const BOUNCES: u32 = 10;
    const PER: usize = 8;
    let mut rng = Pcg32::new(5, 11);
    let items: Vec<[u32; 4]> = (0..20_000).map(|_| [rng.next_u32() % 4096, rng.next_u32() % 4096, rng.next_u32() % 100_000, rng.next_u32()]).collect();
    let inputs: Vec<u32> = items.iter().flatten().copied().collect();
    let out = harness.run_wgsl(
        &["sampler"],
        &format!(
            "var s = sampler_new(inp(0u), inp(1u), inp(2u), inp(3u));
             for (var b = 0u; b < {BOUNCES}u; b++) {{
                 sampler_begin_bounce(&s, b);
                 let d = sampler_bsdf_direction(&s);
                 let base = b * {PER}u;
                 outf(base, d.x); outf(base + 1u, d.y);
                 outf(base + 2u, sampler_bsdf_lobe(&s));
                 outf(base + 3u, sampler_russian_roulette(&s));
                 let l = sampler_light_sample(&s);
                 outf(base + 4u, l.x); outf(base + 5u, l.y);
                 outf(base + 6u, sampler_light_select(&s));
                 outf(base + 7u, sampler_medium(&s));
             }}"
        ),
        &inputs,
        4,
        BOUNCES as usize * PER,
    );
    let stride = BOUNCES as usize * PER;
    for (i, &[x, y, sample, seed]) in items.iter().enumerate() {
        let mut s = Sampler::new(x, y, sample, seed);
        let mut expected = Vec::with_capacity(stride);
        for b in 0..BOUNCES {
            s.begin_bounce(b);
            let d: Vec2 = s.bsdf_direction();
            expected.extend([d.x, d.y, s.bsdf_lobe(), s.russian_roulette()]);
            let l = s.light_sample();
            expected.extend([l.x, l.y, s.light_select(), s.medium()]);
        }
        let expected: Vec<u32> = expected.iter().map(|v| v.to_bits()).collect();
        let got = &out[i * stride..(i + 1) * stride];
        if got != expected.as_slice() {
            let at = got.iter().zip(&expected).position(|(a, b)| a != b).unwrap();
            panic!(
                "pixel ({x}, {y}) sample {sample} seed {seed}: draw {at} (bounce {}, slot {}) is {:08x} on the card and {:08x} on the processor",
                at / PER,
                at % PER,
                got[at],
                expected[at]
            );
        }
    }
}
