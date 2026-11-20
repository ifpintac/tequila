//! A material as the card reads it (`docs/07` R3): the parameters of
//! `tequila_materials::openpbr::Material` that `openpbr.wgsl`'s stack reads,
//! laid out for a storage buffer.
//!
//! For now the stack on the card is the opaque one - fuzz, coat, metal,
//! dielectric specular and the diffuse substrate - and this holds exactly
//! what that reads. Transmission, subsurface, thin film, flakes and
//! thin walls are not on the card yet, so their parameters are not here;
//! [`GpuMaterial::supports`] says whether a material is one the card's stack
//! is the processor's for. Cutout opacity is the integrator's, not the
//! BSDF's, and is not here either; emission is the integrator's too, and is
//! here for it to read (`transport.wgsl`: a lamp's stand-in glowing).

use bytemuck::{Pod, Zeroable};
use tequila_core::spectrum::Spectrum;
use tequila_materials::openpbr::Material;

/// `openpbr.wgsl`'s `GpuMaterial`, member for member: colours as four
/// floats (the fourth unused) so each starts on sixteen bytes as WGSL's
/// `vec4<f32>` does, then the scalars, then padding to WGSL's size.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuMaterial {
    pub base_color: [f32; 4],
    /// `metal_ior.n`.
    pub metal_n: [f32; 4],
    /// `metal_ior.k`.
    pub metal_k: [f32; 4],
    pub specular_color: [f32; 4],
    pub coat_color: [f32; 4],
    pub fuzz_color: [f32; 4],
    pub base_weight: f32,
    pub base_metalness: f32,
    pub base_diffuse_roughness: f32,
    pub specular_weight: f32,
    pub specular_roughness: f32,
    pub specular_roughness_anisotropy: f32,
    pub specular_ior: f32,
    pub coat_weight: f32,
    pub coat_roughness: f32,
    pub coat_ior: f32,
    pub coat_darkening: f32,
    pub fuzz_weight: f32,
    pub fuzz_roughness: f32,
    /// Baked occlusion, which scales every lobe of the stack.
    pub geometry_occlusion: f32,
    pub _pad: [f32; 2],
    /// `Material::emission`, in nits, and in `[3]` 1 where the material
    /// `is_emissive`.
    pub emission: [f32; 4],
}

fn colour(s: Spectrum) -> [f32; 4] {
    [s.r, s.g, s.b, 0.0]
}

impl GpuMaterial {
    /// The parameters the card's stack reads, as they are. What it does not
    /// read yet is left behind: see [`Self::supports`].
    pub fn from_material(m: &Material) -> GpuMaterial {
        GpuMaterial {
            base_color: colour(m.base_color),
            metal_n: colour(m.metal_ior.n),
            metal_k: colour(m.metal_ior.k),
            specular_color: colour(m.specular_color),
            coat_color: colour(m.coat_color),
            fuzz_color: colour(m.fuzz_color),
            base_weight: m.base_weight,
            base_metalness: m.base_metalness,
            base_diffuse_roughness: m.base_diffuse_roughness,
            specular_weight: m.specular_weight,
            specular_roughness: m.specular_roughness,
            specular_roughness_anisotropy: m.specular_roughness_anisotropy,
            specular_ior: m.specular_ior,
            coat_weight: m.coat_weight,
            coat_roughness: m.coat_roughness,
            coat_ior: m.coat_ior,
            coat_darkening: m.coat_darkening,
            fuzz_weight: m.fuzz_weight,
            fuzz_roughness: m.fuzz_roughness,
            geometry_occlusion: m.geometry_occlusion,
            _pad: [0.0; 2],
            emission: {
                let e = m.emission();
                [e.r, e.g, e.b, if m.is_emissive() { 1.0 } else { 0.0 }]
            },
        }
    }

    /// Whether `OpenPbrBsdf` builds `m` as the card's stack does: no
    /// transmission and no subsurface at all (even a share too small for
    /// their own lobes takes from the interface and the diffuse), no thin
    /// film the processor would put on, and no flake under this point.
    /// Thin walls change only the transmission and subsurface lobes.
    pub fn supports(m: &Material) -> bool {
        m.transmission_weight == 0.0
            && m.subsurface_weight == 0.0
            && m.thin_film_weight <= 1e-4
            && (m.flake_weight <= 1e-4 || m.flake_coverage <= 0.0)
    }
}
