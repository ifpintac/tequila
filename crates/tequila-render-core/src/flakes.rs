//! The flake field of metallic paint (`docs/02-MATERIAL-SYSTEM.md` §4.4, and
//! §3.1's Metallic Paint).
//!
//! Metal platelets suspended in the paint under its clear coat. Each lies at a
//! slightly different tilt, so at any viewing angle a few of them happen to
//! mirror a light into the eye and the rest do not — which is the sparkle, and
//! the whole difference between metallic paint and a flat coloured metal.
//!
//! The surface is divided into Worley cells a flake across; a cell holds a
//! flake with probability `flake_weight`, so that is the share of the base the
//! flakes cover, and each flake takes a tilt from its own cell. Evaluated in
//! the part's object space, in scene units, like every pattern — a flake stays
//! where it is on the part as the part moves, and is the size it says whatever
//! scale the part was imported at.
//!
//! The field only says *where* the flakes are and how they lean; the layer
//! stack (`OpenPbrBsdf`) turns that into a metal lobe under the coat, in the
//! flake's own frame, in place of that share of the base.

use crate::material::Material;
use tequila_core::math::Vec3;
use tequila_texture::noise::{worley, Metric};
use tequila_texture::projection::SurfacePoint;

/// The steepest a flake can lean, at spread 1: sixty degrees.
const MAX_TILT: f32 = std::f32::consts::FRAC_PI_3;

/// `material` with this point's flake filled in: its coverage (0 or 1) and
/// its normal in the shading frame. Unchanged when the material has no
/// flakes.
pub fn apply(material: &Material, point: &SurfacePoint) -> Material {
    let mut out = material.clone();
    if material.flake_weight <= 1e-4 {
        return out;
    }
    let size = material.flake_size.max(1e-7);
    let cell = worley(point.position * (point.scale / size), 0x0f1a_4e5d, Metric::Euclidean);
    // Three independent numbers per cell, from the one the lookup gives.
    let seed = cell.id.to_bits();
    let (present, a, b) = (unit(seed, 1), unit(seed, 2), unit(seed, 3));
    if present >= material.flake_weight {
        out.flake_coverage = 0.0;
        return out;
    }
    // Uniform over the cap of directions within the spread: a flake is as
    // likely to lean one way as another, and to lean any amount up to the
    // limit.
    let cap = (material.flake_spread.clamp(0.0, 1.0) * MAX_TILT).cos();
    let cos_theta = 1.0 - a * (1.0 - cap);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = b * std::f32::consts::TAU;
    out.flake_coverage = 1.0;
    out.flake_normal = Vec3 {
        x: sin_theta * phi.cos(),
        y: sin_theta * phi.sin(),
        z: cos_theta,
    };
    out
}

/// A number in `[0, 1)` from a cell's bits and a stream index.
#[inline]
fn unit(bits: u32, stream: u32) -> f32 {
    let mut h = bits ^ stream.wrapping_mul(0x9e37_79b9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    fn point(x: f32, z: f32) -> SurfacePoint {
        SurfacePoint {
            position: vec3(x, 0.0, z),
            normal: Vec3::Y,
            uv: vec2(0.0, 0.0),
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        }
    }

    fn paint(weight: f32, spread: f32) -> Material {
        Material {
            flake_weight: weight,
            flake_spread: spread,
            flake_size: 0.001,
            ..Material::default()
        }
    }

    /// The amount is the share of the surface the flakes cover, and every
    /// flake leans no further than the spread allows.
    #[test]
    fn flakes_cover_their_share_and_lean_within_the_spread() {
        for (weight, spread) in [(0.25f32, 0.2f32), (0.6, 0.5), (1.0, 1.0)] {
            let material = paint(weight, spread);
            let limit = (spread * MAX_TILT).cos();
            let mut on = 0;
            let total = 40_000;
            for i in 0..total {
                let (x, z) = ((i % 200) as f32 * 0.00037, (i / 200) as f32 * 0.00041);
                let flaked = apply(&material, &point(x, z));
                if flaked.flake_coverage > 0.5 {
                    on += 1;
                    let n = flaked.flake_normal;
                    assert!((n.length() - 1.0).abs() < 1e-4);
                    assert!(n.z >= limit - 1e-4, "a flake leans past the spread: {n:?}");
                }
            }
            let share = on as f32 / total as f32;
            assert!((share - weight).abs() < 0.06, "amount {weight} covered {share}");
        }
    }

    /// A flake is the same flake everywhere inside it, and neighbours differ:
    /// the tilt belongs to the flake, not to the point.
    #[test]
    fn a_flake_keeps_its_tilt_across_itself() {
        let material = paint(1.0, 1.0);
        // A step of a fiftieth of a flake stays on the same flake unless it
        // happens to cross an edge, which a few in a hundred will.
        let mut same = 0;
        for i in 0..400 {
            let (x, z) = (0.0123 + i as f32 * 0.00173, 0.0071 + i as f32 * 0.00091);
            let a = apply(&material, &point(x, z)).flake_normal;
            let b = apply(&material, &point(x + 0.00002, z)).flake_normal;
            same += (a == b) as u32;
        }
        assert!(same > 380, "only {same} of 400 tiny steps stayed on their flake");
        let here = apply(&material, &point(0.0123, 0.0071)).flake_normal;
        let mut differ = 0;
        for i in 1..50 {
            let other = apply(&material, &point(0.01 + i as f32 * 0.003, 0.01)).flake_normal;
            if (other - here).length() > 1e-3 {
                differ += 1;
            }
        }
        assert!(differ > 40, "only {differ} of 49 distant flakes lean differently");
    }

    /// Size is what it says: flakes twice as big change tilt half as often
    /// along a line across them.
    #[test]
    fn flake_size_sets_how_often_the_tilt_changes() {
        let changes = |size: f32| {
            let material = Material { flake_size: size, ..paint(1.0, 1.0) };
            let mut previous = apply(&material, &point(0.0, 0.0)).flake_normal;
            let mut count = 0;
            for i in 1..4000 {
                let n = apply(&material, &point(i as f32 * 0.0001, 0.0137)).flake_normal;
                if n != previous {
                    count += 1;
                    previous = n;
                }
            }
            count as f32
        };
        let (small, large) = (changes(0.001), changes(0.002));
        let ratio = small / large;
        assert!((ratio - 2.0).abs() < 0.4, "1 mm flakes changed {small} times, 2 mm {large}: ratio {ratio}");
    }

    /// No amount, no flakes: the material comes back as it went in.
    #[test]
    fn no_amount_no_flakes() {
        let material = paint(0.0, 0.5);
        assert_eq!(apply(&material, &point(0.3, 0.2)), material);
    }
}
