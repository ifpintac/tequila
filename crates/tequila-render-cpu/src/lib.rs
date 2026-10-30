//! The CPU rendering backend: Embree 4 for traversal, Rayon over tiles.
//!
//! `TEQUILA_SPEC.md` §2.1 on why the BVH is not ours to write: "Do not write
//! your own BVH; you will not beat it and it is not where the value is."
//!
//! This crate is one of two implementations of
//! [`Intersector`](tequila_render_core::intersect::Intersector). The
//! *integrator* may differ between backends because it is control flow; the
//! shading math may not (`docs/01` §10.1).

#![warn(missing_debug_implementations)]

pub mod embree;
mod embree_sys;
pub mod floor;
pub mod preview;
pub mod renderer;
pub mod silhouette;
pub mod wireframe;

pub use embree::{Device, EmbreeIntersector};
pub use preview::{
    render_thumbnail, render_thumbnail_graph, render_thumbnail_textured, PreviewSettings, PreviewShape, Thumbnail, PREVIEW_RADIUS,
};
pub use silhouette::Silhouette;
pub use renderer::{build_caustics, preview_pass, render_pass, render_pass_with_aovs, RenderContext};
