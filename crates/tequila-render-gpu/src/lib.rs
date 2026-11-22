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
//!   `@group(1)`: the Sobol′ sampler's, the GGX energy tables and the
//!   sheen's.
//! - [`material`]: a material as the card reads it, for the OpenPBR stack in
//!   `openpbr.wgsl` (R3: so far its opaque part).
//! - [`program`]: a material's graph as the card runs it (R3, stage 1): its
//!   compiled program lowered to the bytecode `program.wgsl` interprets,
//!   the nodes of `nodes.wgsl`, and the height bump; what the card cannot
//!   run yet it refuses, and says why.
//! - [`harness`]: running a WGSL function over a buffer of inputs, which is
//!   how every port is tested against the Rust it was ported from.
//! - [`scene`]: a snapshot's triangles, acceleration structures and the
//!   records a hit leads back to (R2.1), their layouts held to the WGSL's
//!   by test (R2.2).
//! - [`lights`]: the scene's analytic lamps and the way the processor
//!   chooses one, as `LightSet` built it (R3).
//! - [`emitters`]: the scene's glowing triangles as lights, the tree of
//!   them as `Emitters` built it (R3).
//! - [`cutaways`]: the scene's cutaway volumes, which every walk's query
//!   steps past, their caps met (R3).
//! - [`textures`]: the pictures a scene's programs sample, every level as
//!   `tequila-texture` keeps it, for `textures.wgsl` to filter as `mip.rs`
//!   does (R3).
//! - [`environment`] and [`transport`]: the transport (R2.5, R3), opaque
//!   OpenPBR surfaces under the environment and the lamps, on a shadow
//!   catcher's floor, the processor's path draw for draw, its samples
//!   accumulated by the processor's `Framebuffer`.
//! - [`variants`]: the transport kernel built with only the parts a scene
//!   uses, which the driver compiles in a fraction of the whole kernel's
//!   time, and gives the whole kernel's picture.

pub mod cutaways;
pub mod device;
pub mod emitters;
pub mod environment;
pub mod harness;
pub mod lights;
pub mod material;
pub mod program;
pub mod scene;
pub mod shader;
pub mod tables;
pub mod textures;
pub mod transport;
pub mod variants;

pub use device::{GpuDevice, Needs};
