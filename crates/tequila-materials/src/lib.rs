//! OpenPBR surface model, material graph, and procedural textures.
//!
//! `docs/02-MATERIAL-SYSTEM.md` is the specification this crate implements.
//! Its central decision: **one** shading model (OpenPBR Surface), with all 34
//! KeyShot-equivalent material types expressed as parameter presets over it,
//! rather than 34 hand-tuned implementations.
//!
//! At M0 this crate provides the [`Bsdf`](bsdf::Bsdf) contract and the three
//! lobes the renderer proof-of-concept needs. The full model, the graph IR,
//! and the texture pipeline land at M2.

#![warn(missing_debug_implementations)]

pub mod bsdf;
pub mod energy;
pub mod fresnel;
pub mod graph;
pub mod groups;
pub mod lobes;
pub mod material_type;
pub mod nodes;
pub mod microfacet;
pub mod openpbr;
pub mod thin_film;

pub use bsdf::{Bsdf, BsdfFlags, BsdfSample};
pub mod decal;
pub mod dispersion;
