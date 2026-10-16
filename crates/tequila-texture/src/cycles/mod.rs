//! Blender's shader-node textures and converters, ported from Cycles.
//!
//! Cycles is Blender's renderer and is licensed Apache-2.0, like Tequila
//! (Blender itself is GPL, and nothing here comes from it). These are
//! Rust translations of Cycles' SVM kernels - Copyright 2011-2022 Blender
//! Foundation, SPDX-License-Identifier: Apache-2.0 - so that a material
//! graph built from them shades the same in Tequila's renderer as in
//! Cycles: the same hashes, the same noise, the same cells, value for value.
//! Cycles' licence and notice ship in `scripts/notices/cycles`.

pub mod hash;
pub mod noise;
pub mod ops;
pub mod textures;
pub mod voronoi;
