//! The light-transport engine: integrator, sampling, camera, environment, and
//! the render orchestrator.
//!
//! `docs/01-RENDERER-CORE.md` is the specification this crate implements. Its
//! five design principles, in priority order, govern every decision here:
//!
//! 1. **Interactivity is a correctness requirement, not an optimization.**
//! 2. **Physical correctness by default**; every shortcut is opt-in and named.
//! 3. **One implementation of the shading math** across backends.
//! 4. **Determinism**: same inputs, same seed, same output, bit for bit.
//! 5. **Progressive refinement**, never restarting when the edit did not
//!    invalidate the accumulated samples.
//!
//! The crate is backend-agnostic: it is written against the
//! [`Intersector`](intersect::Intersector) trait, so the same transport code
//! runs over Embree on the CPU and over hardware ray tracing on the GPU.

#![warn(missing_debug_implementations)]

pub mod aov;
pub mod bevel;
pub mod camera;
pub mod cryptomatte;
pub mod cutaway;
pub mod density;
pub mod emitters;
pub mod environment;
pub mod flakes;
pub mod framebuffer;
pub mod hosek;
mod hosek_data;
pub mod ies;
pub mod integrator;
pub mod intersect;
pub mod light;
pub mod lpe;
pub mod material;

pub mod pins;
pub mod sky;
pub mod preview;
pub mod scene;
pub mod shade;
pub mod surface_bump;
pub mod scheduler;
pub mod settings;
pub mod test_support;

pub use camera::{Aperture, Camera, Projection};
pub use environment::{Background, Environment};
pub use framebuffer::{Aov, Framebuffer, Pixel};
pub use integrator::{trace_path, PathResult};
pub use intersect::{Hit, Intersector};
pub use material::{Material, MaterialBsdf, MaterialType};
pub use scene::{Instance, Mesh, SceneSnapshot};
pub use scheduler::{Dirty, ProgressiveSchedule, Regime, SceneChange, Tile};
pub use settings::{IntegratorConfig, LightingPreset};
