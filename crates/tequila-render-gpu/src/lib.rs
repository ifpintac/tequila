//! Tequila's renderer on the graphics card (`docs/07-OWN-GPU-RENDERER.md`).
//!
//! Through wgpu's ray queries: Vulkan on Windows and Linux, Metal on the
//! Mac. The kernels are hand-written WGSL in `src/wgsl/`, one file per
//! module, each checked against its Rust twin in the processor renderer:
//! the processor renderer stays the reference, and a card picture is right
//! when it is the processor's picture.
//!
//! What is here so far (R2.1-R2.4):
//! - [`device`]: choosing the adapter, what it must have, and why one was
//!   refused.
//! - [`shader`]: the WGSL modules and the step that joins a module to the
//!   ones it includes.
//! - [`tables`]: the read-only tables every kernel shares, at
//!   `@group(1)`: today the Sobol′ sampler's.
//! - [`harness`]: running a WGSL function over a buffer of inputs, which is
//!   how every port is tested against the Rust it was ported from.

pub mod device;
pub mod harness;
pub mod shader;
pub mod tables;

pub use device::{GpuDevice, Needs};
