//! The doorway into subsurface scattering (`docs/01` §6.4).
//!
//! A random walk is transport, not a BSDF: light enters the surface, scatters
//! about inside the part as through a dense medium, and leaves somewhere else.
//! What the surface itself contributes is the way in and the way out, and
//! both are this lobe: a Lambertian *transmission*, sending light across the
//! surface cosine-weighted about its normal. It is white — the colour comes
//! from the walk, which is what makes it depend on how far light travels
//! inside and on the shape of the part — and its samples carry
//! [`BsdfFlags::SUBSURFACE`], which is how the integrator knows a path has
//! entered the part's medium, or left it.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use tequila_core::math::{Vec2, Vec3};
use tequila_core::sampling::{cosine_hemisphere_pdf, sample_cosine_hemisphere};
use tequila_core::spectrum::Spectrum;
use std::f32::consts::FRAC_1_PI;

#[derive(Clone, Copy, Debug)]
pub struct DiffuseTransmission {
    /// What the lobe passes. White for a doorway into a walk.
    tint: Spectrum,
    /// What the surface looks like overall — the colour the walk is tuned to
    /// return — for the denoiser's albedo guide and the stack's lobe choice.
    /// The lobe's value does not use it.
    appearance: Spectrum,
}

impl DiffuseTransmission {
    pub fn new(tint: Spectrum, appearance: Spectrum) -> Self {
        DiffuseTransmission { tint, appearance }
    }
}

impl Bsdf for DiffuseTransmission {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        if wo.z * wi.z < 0.0 {
            self.tint * FRAC_1_PI
        } else {
            Spectrum::ZERO
        }
    }

    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        if wo.z * wi.z < 0.0 {
            cosine_hemisphere_pdf(wi.z.abs())
        } else {
            0.0
        }
    }

    fn sample(&self, wo: Vec3, u: Vec2, _uc: f32) -> Option<BsdfSample> {
        let mut wi = sample_cosine_hemisphere(u);
        // Across the surface from where the light came.
        if wo.z > 0.0 {
            wi.z = -wi.z;
        }
        let pdf = cosine_hemisphere_pdf(wi.z.abs());
        if pdf <= 0.0 {
            return None;
        }
        Some(BsdfSample {
            wi,
            f: self.tint * FRAC_1_PI,
            pdf,
            is_delta: false,
            is_transmission: true,
            eta: 1.0,
            flags: self.flags(),
        })
    }

    fn albedo(&self, _wo: Vec3) -> Spectrum {
        self.appearance
    }

    fn flags(&self) -> BsdfFlags {
        BsdfFlags::TRANSMISSION | BsdfFlags::DIFFUSE | BsdfFlags::SUBSURFACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// The doorway passes all of the light that reaches it, to the other
    /// side and none back: the walk decides what comes out, not the door.
    #[test]
    fn the_doorway_passes_everything_across() {
        let lobe = DiffuseTransmission::new(Spectrum::ONE, Spectrum::splat(0.5));
        for wo in [vec3(0.0, 0.0, 1.0), vec3(0.6, 0.0, 0.8), vec3(0.0, 0.3, -0.95)] {
            let mut total = 0.0f64;
            let n = 4096;
            for i in 0..n {
                let u = Vec2 { x: (i as f32 + 0.5) / n as f32, y: ((i * 7919) % n) as f32 / n as f32 };
                let s = lobe.sample(wo, u, 0.5).expect("a sample");
                assert!(s.wi.z * wo.z < 0.0, "it sent light back");
                assert!((lobe.pdf(wo, s.wi) - s.pdf).abs() < 1e-5);
                total += s.throughput_weight().luminance() as f64;
            }
            let passed = total / n as f64;
            assert!((passed - 1.0).abs() < 1e-3, "{wo:?}: passed {passed}");
        }
    }
}
