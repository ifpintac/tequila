//! Core types shared by every Tequila crate: math, sampling, radiometry,
//! units, and identifiers.
//!
//! This crate has no dependencies outside the standard library. Everything
//! here is on the renderer's hot path or is a type the rest of the system is
//! built from, so it is kept deliberately small and free of abstraction.
//!
//! See `docs/01-RENDERER-CORE.md` §4 for the sampling specification these
//! modules implement.

#![deny(clippy::float_cmp_const)]
#![warn(missing_debug_implementations)]

pub mod guard;
pub mod ids;
pub mod math;
pub mod rng;
pub mod sampler;
pub mod sampling;
pub mod sobol;
pub mod spectrum;
pub mod units;

/// The types used often enough that importing them individually is noise.
pub mod prelude {
    pub use crate::ids::*;
    pub use crate::math::{
        offset_ray_origin, safe_acos, safe_sqrt, shading, sqr, vec2, vec3, Aabb, Mat4, Onb, Ray,
        Transform, Vec2, Vec3,
    };
    pub use crate::sampler::Sampler;
    pub use crate::spectrum::{spectrum, Spectrum};
}
