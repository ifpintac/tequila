//! A material as the card reads it (`docs/07` R3): the parameters of
//! `tequila_materials::openpbr::Material` that `openpbr.wgsl`'s stack reads,
//! laid out for a storage buffer.
//!
//! The stack on the card is fuzz, coat, a metal flake under the coat,
//! metal and dielectric specular (each under a thin film where the material
//! has one), the transmission lobe (glass, thin-walled or
//! solid) and the diffuse substrate, and this holds what that reads; and
//! what the integrator reads of glass besides - the medium a solid part is
//! (its priority, its index as the scene has it, whether its volume absorbs)
//! and the clear glass a shadow ray passes (`Pane`); and the thin film over
//! the metal and the interface. Subsurface is not on the card yet, so its
//! parameters are not here; [`GpuMaterial::supports`] says whether a
//! material is one the card's stack is the processor's for.
//! Cutout opacity and emission are the integrator's, not the BSDF's, and
//! are here for it to read (`transport.wgsl`: a cutout's holes, a glowing
//! surface), among the floats a program writes.

use bytemuck::{Pod, Zeroable};
use tequila_core::spectrum::Spectrum;
use tequila_materials::openpbr::Material;
use tequila_render_core::scene::Pane;

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
    /// The anisotropic frame's turn (`specular_rotation`), which the
    /// integrator reads rather than the stack.
    pub specular_rotation: f32,
    /// The cutout's opacity (`geometry_opacity`): 1 a wall, 0 a hole. The
    /// integrator's, not the stack's.
    pub geometry_opacity: f32,
    /// `emission_color`, and in `[3]` `emission_luminance`: the integrator
    /// reads `Material::emission` and `is_emissive` from the two, as a
    /// program writes them.
    pub emission_color: [f32; 4],
    /// `transmission_color`, and in `[3]` `transmission_weight`.
    pub transmission_color: [f32; 4],
    /// The transmission lobe's tint at each crossing, the square root of
    /// the colour the surface wears (`transmission_tint`, times
    /// `transmission_color` where there is no volume to absorb it), worked
    /// out here as the processor rounds it; and in `[3]`
    /// `transmission_depth`.
    pub transmission_tint: [f32; 4],
    pub volume_density: f32,
    pub medium_priority: f32,
    /// The index the scene's own material has (`SceneSnapshot::material`,
    /// not the program's), which a medium entered through a surface that is
    /// no interface records (integrator.rs's null crossing).
    pub base_ior: f32,
    /// [`GLASS_THIN`] and the rest.
    pub glass: u32,
    /// The clear glass a shadow ray passes (`SceneSnapshot::pane`), where
    /// [`GLASS_PANE`] says there is one: what reaches its transmission
    /// lobe, its cutout's opacity, its index, its coat and the coat's index,
    /// and the depth its volume reaches its colour in (`volume_depth`).
    pub pane_weight: f32,
    pub pane_opacity: f32,
    pub pane_ior: f32,
    pub pane_coat: f32,
    pub pane_coat_ior: f32,
    pub pane_depth: f32,
    /// Rounded edges (Cycles' Bevel, `bevel.rs`): the radius the normal is
    /// rounded over at the surface a path meets first, none at 0, and how
    /// many probes round it (`edge_samples as u32`, as the integrator
    /// counts them).
    pub edge_radius: f32,
    pub edge_samples: u32,
    /// What the volume behind the surface gives off (`emitted_over`): the
    /// glow, `volume_emission_color` times `volume_emission_strength` (none
    /// where the strength is none), and in `[3]` what it absorbs a metre
    /// before its colour, `volume_density` over `transmission_depth`.
    pub volume_glow: [f32; 4],
    /// Metallic paint's flakes (`flakes.rs`): `flake_color`, and in `[3]`
    /// `flake_weight`, how much of the surface they cover.
    pub flake_color: [f32; 4],
    /// `flake_size`, `flake_spread` and `flake_roughness`, and in `[3]` the
    /// point's `flake_coverage`: nought in a scene's record, filled in at
    /// each point by the kernel (`flakes.wgsl`) as the processor fills it
    /// in, wherever the weight is over 1e-4.
    pub flake: [f32; 4],
    /// The point's `flake_normal`, the flake's lean in the shading frame:
    /// straight up in a scene's record, the kernel's at a point. `[3]`
    /// unused.
    pub flake_normal: [f32; 4],
    /// The thin film over the metal and the dielectric interface
    /// (`thin_film.rs`): `thin_film_weight`, `thin_film_thickness` in
    /// nanometres and `thin_film_ior`; and in `[3]`
    /// `transmission_dispersion_abbe`, the glass's dispersion.
    pub film: [f32; 4],
}

/// [`GpuMaterial::glass`]: an infinitely thin shell (`geometry_thin_walled`).
pub const GLASS_THIN: u32 = 1;
/// The scene's own material is the boundary of a medium: transmissive and
/// not thin-walled (integrator.rs's `is_boundary`).
pub const GLASS_BOUNDARY: u32 = 2;
/// The medium behind the surface absorbs along the path (`Medium::of`'s
/// volume: a volume that does not scatter).
pub const GLASS_VOLUME: u32 = 4;
/// Clear glass a shadow ray passes (`Pane`).
pub const GLASS_PANE: u32 = 8;
/// Plain glass, which the camera splits at (`Pane::plain`).
pub const GLASS_PLAIN: u32 = 16;
/// Glass that bends the light through it (`Pane::bends`).
pub const GLASS_BENDS: u32 = 32;
/// The pane's volume absorbs between its way in and its way out
/// (`Pane::has_volume`).
pub const GLASS_PANE_VOLUME: u32 = 64;

fn colour(s: Spectrum) -> [f32; 4] {
    [s.r, s.g, s.b, 0.0]
}

impl GpuMaterial {
    /// The parameters the card's stack reads, as they are, with no clear
    /// glass and `m` its own scene material. What it does not read yet is
    /// left behind: see [`Self::supports`].
    pub fn from_material(m: &Material) -> GpuMaterial {
        GpuMaterial::with_glass(m, m, Pane::of(m).as_ref())
    }

    /// The same for a material of a scene: `m` the parameters its surface
    /// is shaded with (a program's constants), `scene` the scene's own
    /// material (`SceneSnapshot::material`, which the integrator asks which
    /// surfaces are a medium's boundary, and of the medium a surface that is
    /// no interface leads into), `pane` the clear glass it is to a shadow
    /// ray (`SceneSnapshot::pane`).
    pub fn with_glass(m: &Material, scene: &Material, pane: Option<&Pane>) -> GpuMaterial {
        // As OpenPbrBsdf::build has the transmission lobe's tint: the
        // surface's own tint where the volume takes the colour, both where
        // there is no volume, a square root at each crossing.
        let has_depth = !m.geometry_thin_walled && m.transmission_depth > 0.0;
        let c = if has_depth { m.transmission_tint } else { m.transmission_color * m.transmission_tint };
        let tint = [c.r.max(0.0).sqrt(), c.g.max(0.0).sqrt(), c.b.max(0.0).sqrt(), m.transmission_depth];
        let mut glass = 0;
        for (on, flag) in [
            (m.geometry_thin_walled, GLASS_THIN),
            (scene.is_transmissive() && !scene.geometry_thin_walled, GLASS_BOUNDARY),
            (m.has_volume() && m.volume_scattering().is_none(), GLASS_VOLUME),
            (pane.is_some(), GLASS_PANE),
            (pane.is_some_and(|p| p.plain), GLASS_PLAIN),
            (pane.is_some_and(|p| p.bends()), GLASS_BENDS),
            (pane.is_some_and(|p| p.has_volume()), GLASS_PANE_VOLUME),
        ] {
            if on {
                glass |= flag;
            }
        }
        let p = pane.copied();
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
            specular_rotation: m.specular_rotation,
            geometry_opacity: m.geometry_opacity,
            emission_color: [m.emission_color.r, m.emission_color.g, m.emission_color.b, m.emission_luminance],
            transmission_color: [m.transmission_color.r, m.transmission_color.g, m.transmission_color.b, m.transmission_weight],
            transmission_tint: tint,
            volume_density: m.volume_density,
            medium_priority: m.medium_priority,
            base_ior: scene.specular_ior,
            glass,
            pane_weight: p.map_or(0.0, |p| p.weight),
            pane_opacity: p.map_or(0.0, |p| p.opacity),
            pane_ior: p.map_or(1.0, |p| p.ior),
            pane_coat: p.map_or(0.0, |p| p.coat),
            pane_coat_ior: p.map_or(1.0, |p| p.coat_ior),
            pane_depth: p.and_then(|p| p.volume).map_or(0.0, |(_, depth)| depth),
            edge_radius: m.edge_radius,
            edge_samples: m.edge_samples as u32,
            volume_glow: {
                // As Material::emitted_over works them out.
                let glow = if m.volume_emission_strength > 0.0 { m.volume_emission_color * m.volume_emission_strength } else { Spectrum::ZERO };
                let per_metre = if m.transmission_depth > 0.0 { m.volume_density.max(0.0) / m.transmission_depth } else { 0.0 };
                [glow.r, glow.g, glow.b, per_metre]
            },
            flake_color: [m.flake_color.r, m.flake_color.g, m.flake_color.b, m.flake_weight],
            flake: [m.flake_size, m.flake_spread, m.flake_roughness, m.flake_coverage],
            flake_normal: [m.flake_normal.x, m.flake_normal.y, m.flake_normal.z, 0.0],
            film: [m.thin_film_weight, m.thin_film_thickness, m.thin_film_ior, m.transmission_dispersion_abbe],
        }
    }

    /// Whether the card renders `m` as the processor does: no subsurface at
    /// all (even a share too small for its own lobe takes from the diffuse);
    /// and as glass, none that scatters in its volume (a random walk) or
    /// that disperses light (a wavelength of its own). Flakes and thin films
    /// it renders, wherever they are.
    pub fn supports(m: &Material) -> bool {
        let glass = m.transmission_weight > 0.0;
        m.subsurface_weight == 0.0
            && !(glass && m.volume_scattering().is_some())
            && !(glass && m.transmission_dispersion_abbe > 0.0)
    }
}
