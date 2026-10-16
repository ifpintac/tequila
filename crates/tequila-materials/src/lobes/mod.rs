//! BSDF lobes.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §1: Tequila implements **one** surface
//! shading model and expresses every material type as a preset over it. These
//! are the lobes that model composes. A new material "type" is a data file,
//! not a new lobe.
//!
//! M0 implements the three the renderer proof-of-concept needs (`docs/05` §1),
//! plus the first layering case. The remaining OpenPBR lobes — transmission,
//! subsurface, coat, fuzz, thin film — arrive at M2.

pub mod conductor;
pub mod dielectric;
pub mod diffuse;
pub mod layered;
pub mod openpbr;
pub mod sheen;
mod sheen_ltc;
pub mod subsurface;
pub mod transmission;

pub use conductor::Conductor;
pub use dielectric::DielectricSpecular;
pub use diffuse::OrenNayarDiffuse;
pub use layered::SpecularOverDiffuse;
pub use openpbr::OpenPbrBsdf;
pub use sheen::Sheen;
pub use subsurface::DiffuseTransmission;
pub use transmission::DielectricTransmission;
