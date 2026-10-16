//! Render-side materials: the bridge from a stored material to a [`Bsdf`].
//!
//! `docs/02-MATERIAL-SYSTEM.md` §1 makes a decision this module exists to
//! honour: there is exactly **one** surface shading model, and every material
//! type is a parameter preset over it. A new material "type" is a data file,
//! not new code.
//!
//! At M0 this module owned a cut-down parameter block and a four-way
//! `MaterialKind` enum, with a note that M2 would replace them. M2 has: the
//! parameter block is now [`tequila_materials::openpbr::Material`], the full
//! §2.2 set of 35 parameters, and the four kinds are gone in favour of the
//! 34-entry catalog in [`MaterialType`]. The catalog entry is a *preset
//! identity* — it says which parameters the UI exposes (`docs/04` §3), not
//! how the surface shades. Two materials of different types with identical
//! parameters render identically, which is the whole point of §1.
//!
//! What survives from M0 is the seam: the integrator asks a material for a
//! BSDF and knows nothing else about it. That is what kept this replacement
//! from rippling into the integrator.

pub use tequila_materials::material_type::{Category, MaterialType, Param};
pub use tequila_materials::openpbr::{Material, SurfaceFrame};

/// The BSDF a material resolves to.
///
/// One type rather than an enum over lobes: [`OpenPbrBsdf`] is *itself* the
/// layer stack of §2.1, holding whichever lobes the parameters called for. The
/// M0 `MaterialBsdf` enum existed only because there were three fixed
/// configurations to dispatch between.
///
/// Still no allocation — the stack is a fixed-size array — which matters
/// because one of these is built at every path vertex on every core.
pub type MaterialBsdf = tequila_materials::lobes::OpenPbrBsdf;

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3, Vec3};
    use tequila_core::spectrum::Spectrum;
    use tequila_materials::fresnel::metals;
    use tequila_materials::Bsdf;

    fn wo() -> Vec3 {
        vec3(0.3, 0.0, 0.954).normalize()
    }

    /// Path regularization must be able to raise roughness but never lower it
    /// — a rough material does not become shiny deep in a path.
    #[test]
    fn the_roughness_floor_only_raises_roughness() {
        let smooth = Material::metal(metals::SILVER, 0.01);
        let regularized = smooth.bsdf(0.3);
        let untouched = Material::metal(metals::SILVER, 0.9).bsdf(0.3);

        // A floor of 0.3 on a mirror leaves a lobe that scatters over a
        // measurable solid angle; the rough one is unchanged by it.
        let wi = vec3(-0.3, 0.0, 0.954).normalize();
        assert!(
            regularized.pdf(wo(), wi) > 0.0,
            "the floor did not roughen a near-mirror"
        );
        assert!(!regularized.flags().contains(tequila_materials::BsdfFlags::DELTA));

        let floored = Material::metal(metals::SILVER, 0.9).bsdf(0.0);
        let a = untouched.pdf(wo(), wi);
        let b = floored.pdf(wo(), wi);
        assert!(
            (a - b).abs() < 1e-4,
            "the floor changed an already-rough material: {a} against {b}"
        );
    }

    /// An emissive surface must still scatter *something*, or it leaves a
    /// black hole where reflections of it should be. §3.1 on Flat: "renders as
    /// its colour, still writes depth/ID".
    #[test]
    fn emissive_materials_still_provide_a_bsdf() {
        let m = Material::emissive(Spectrum::ONE, 100.0);
        assert!(m.is_emissive());
        assert_eq!(m.emission(), Spectrum::splat(100.0));
        // No scattering lobes at all, so it declines to sample rather than
        // returning a bogus direction — and the integrator reads that as "this
        // path ends here", which is correct for a light.
        assert!(m.bsdf(0.0).sample(wo(), vec2(0.4, 0.6), 0.5).is_none());
    }

    /// The preset table and the parameter block are separate: a type picks
    /// defaults, and nothing downstream branches on which type it was.
    #[test]
    fn a_type_is_only_a_set_of_defaults() {
        let from_preset = Material::preset(MaterialType::Plastic);
        let by_hand = Material {
            base_color: from_preset.base_color,
            specular_roughness: from_preset.specular_roughness,
            ..from_preset
        };
        assert_eq!(from_preset, by_hand);

        // Two different catalog entries can be made identical, and then they
        // shade identically. Nothing remembers where the parameters came from.
        let plastic = Material::plastic(Spectrum::splat(0.5), 0.3);
        let generic = Material {
            base_color: Spectrum::splat(0.5),
            specular_roughness: 0.3,
            ..MaterialType::Plastic.defaults()
        };
        let wi = vec3(-0.2, 0.3, 0.93).normalize();
        assert_eq!(
            plastic.bsdf(0.0).eval(wo(), wi),
            generic.bsdf(0.0).eval(wo(), wi)
        );
    }

    #[test]
    fn a_default_material_is_a_plausible_mid_grey() {
        let m = Material::default();
        // 18% grey, the photographic reference.
        assert!((m.base_color.g - 0.18).abs() < 1e-6);
        assert!(!m.is_emissive());
    }
}
