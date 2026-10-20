//! Texture and image handling.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §5 is the specification this crate implements:
//! an OpenImageIO-backed tiled MIP cache, UDIM, and per-map colour-space
//! assignment, arriving with the material system at M2.
//!
//! At M0 it provides only what the environment loader needs: float image
//! buffers and readers for the two HDR formats.

#![warn(missing_debug_implementations)]

pub mod bump;
pub mod cache;
pub mod cycles;
pub mod image;
pub mod mip;
pub mod gradient;
pub mod noise;
pub mod procedural;
pub mod projection;
pub mod texture;
pub mod udim;
pub mod print_size;
pub mod psd;
pub mod radiance;
pub mod video;
pub mod volume;

pub use bump::{BumpGradient, TangentFrame};
pub use cache::{CacheStats, TextureCache};
pub use image::{ColorSpace, Image, ImageError};
pub use mip::{MipPyramid, Wrap};
pub use projection::{Mapping, Projection, SurfacePoint, TextureCoords};
pub use texture::{MapRole, Texture};
pub use udim::UdimSet;
