//! Metallic paint's flake field on the card (`flakes.wgsl`'s flake_at)
//! against `tequila_render_core::flakes::apply`: at random points of parts
//! of random scales, for flakes from a few microns to a centimetre across,
//! covering none to all of the surface and leaning from not at all to sixty
//! degrees, whether the point is on a flake and how that flake leans.
//!
//! The cells are Tequila's Worley noise, ported bit for bit, so a point's
//! cell and its three numbers are the processor's; the lean goes through
//! the card's cosine and sine, an ulp or so from the processor's. A point
//! within a rounding of the boundary between two cells may fall in the
//! other: at most one in a thousand.

use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::rng::Pcg32;
use tequila_materials::openpbr::Material;
use tequila_render_gpu::harness::Harness;
use tequila_texture::projection::SurfacePoint;

const N: usize = 20_000;

fn point(position: Vec3, scale: f32) -> SurfacePoint {
    SurfacePoint {
        position,
        normal: Vec3::Y,
        uv: vec2(0.0, 0.0),
        footprint: 0.0,
        scale,
        uv_footprint: None,
        curvature: 0.0,
        occlusion: 0.0,
        vertex_color: tequila_core::spectrum::Spectrum::ONE,
        world: Default::default(),
    }
}

#[test]
fn the_flake_field_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(41, 1);
    let (mut inputs, mut want, mut cases) = (Vec::new(), Vec::new(), Vec::new());
    let mut on = 0;
    for _ in 0..N {
        // From five microns to a centimetre, evenly in the logarithm.
        let size = 10f32.powf(-5.3 + 3.3 * rng.next_f32());
        let weight = match rng.next_f32() {
            x if x < 0.05 => 0.0,
            x if x < 0.25 => 1.0,
            _ => rng.next_f32(),
        };
        let spread = match rng.next_f32() {
            x if x < 0.1 => 0.0,
            x if x < 0.2 => 1.0,
            _ => rng.next_f32(),
        };
        let scale = if rng.next_f32() < 0.5 { 1.0 } else { 0.01 + 3.0 * rng.next_f32() };
        let reach = if rng.next_f32() < 0.5 { 0.05 } else { 2.0 };
        let position = vec3(rng.next_f32() - 0.5, rng.next_f32() - 0.5, rng.next_f32() - 0.5) * reach;
        let material = Material { flake_weight: weight, flake_size: size, flake_spread: spread, ..Material::default() };
        let flaked = tequila_render_core::flakes::apply(&Material { flake_weight: weight.max(2e-4), ..material.clone() }, &point(position, scale));
        let weight = weight.max(2e-4);
        inputs.extend([position.x, position.y, position.z, scale, size, weight, spread].map(f32::to_bits));
        want.push([flaked.flake_normal.x, flaked.flake_normal.y, flaked.flake_normal.z, flaked.flake_coverage]);
        on += (flaked.flake_coverage > 0.5) as usize;
        cases.push(format!("position {position:?} scale {scale} size {size} weight {weight} spread {spread}"));
    }
    let body = "let at = flake_at(vec3<f32>(inf(0u), inf(1u), inf(2u)), inf(3u), inf(4u), inf(5u), inf(6u));
                outf(0u, at.x); outf(1u, at.y); outf(2u, at.z); outf(3u, at.w);";
    let out = h.run_wgsl(&["flakes"], body, &inputs, 7, 4);
    let (mut other_cell, mut worst) = (Vec::new(), 0.0f32);
    for (i, w) in want.iter().enumerate() {
        let c: Vec<f32> = out[i * 4..(i + 1) * 4].iter().map(|b| f32::from_bits(*b)).collect();
        if c[3] != w[3] {
            other_cell.push(i);
            continue;
        }
        if w[3] > 0.5 {
            let d = (0..3).map(|k| (c[k] - w[k]).abs()).fold(0.0f32, f32::max);
            worst = worst.max(d);
            assert!(d <= 2e-6, "#{i}: the card's flake leans {:?}, the processor's {:?} ({})", &c[..3], &w[..3], cases[i]);
        }
    }
    println!("flake field: {on} of {N} points on a flake; {} on the other side of a cell's edge; worst lean apart {worst:.2e}", other_cell.len());
    assert!(other_cell.len() * 1000 <= N, "{} of {N} points fell on another flake: first {:?}", other_cell.len(), other_cell.first().map(|&i| &cases[i]));
}
