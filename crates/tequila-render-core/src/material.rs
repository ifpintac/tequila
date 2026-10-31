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

/// What scatters light at a path vertex: a surface's layer stack, or one of
/// §4.4's fibres grown out of it.
///
/// Two, because a fibre is not a surface: it scatters all round itself, in
/// a frame that runs along it, and none of the stack's layering means
/// anything there. The integrator asks this for everything it asks a BSDF.
#[derive(Clone, Debug)]
pub enum Scattering {
    Surface(MaterialBsdf),
    Fibre(tequila_materials::lobes::Fibre),
}

impl tequila_materials::Bsdf for Scattering {
    fn eval(&self, wo: tequila_core::math::Vec3, wi: tequila_core::math::Vec3) -> tequila_core::spectrum::Spectrum {
        match self {
            Scattering::Surface(b) => b.eval(wo, wi),
            Scattering::Fibre(b) => b.eval(wo, wi),
        }
    }

    fn pdf(&self, wo: tequila_core::math::Vec3, wi: tequila_core::math::Vec3) -> f32 {
        match self {
            Scattering::Surface(b) => b.pdf(wo, wi),
            Scattering::Fibre(b) => b.pdf(wo, wi),
        }
    }

    fn sample(&self, wo: tequila_core::math::Vec3, u: tequila_core::math::Vec2, uc: f32) -> Option<tequila_materials::BsdfSample> {
        match self {
            Scattering::Surface(b) => b.sample(wo, u, uc),
            Scattering::Fibre(b) => b.sample(wo, u, uc),
        }
    }

    fn albedo(&self, wo: tequila_core::math::Vec3) -> tequila_core::spectrum::Spectrum {
        match self {
            Scattering::Surface(b) => b.albedo(wo),
            Scattering::Fibre(b) => b.albedo(wo),
        }
    }

    fn flags(&self) -> tequila_materials::BsdfFlags {
        match self {
            Scattering::Surface(b) => b.flags(),
            Scattering::Fibre(b) => b.flags(),
        }
    }

    fn eval_selective(&self, wo: tequila_core::math::Vec3, wi: tequila_core::math::Vec3, diffuse: bool, specular: bool) -> tequila_core::spectrum::Spectrum {
        match self {
            Scattering::Surface(b) => b.eval_selective(wo, wi, diffuse, specular),
            Scattering::Fibre(b) => b.eval_selective(wo, wi, diffuse, specular),
        }
    }
}

/// The fibre BSDF for a hit on one of `material`'s fibres: the fuzz's
/// colour, darkened by the fibre's own share of the colour variation, and
/// the fuzz's roughness, raised to `roughness_floor` as every lobe is deep
/// in a path.
pub fn fibre_bsdf(material: &Material, fibre: &crate::scene::FibreHit, roughness_floor: f32) -> tequila_materials::lobes::Fibre {
    tequila_materials::lobes::Fibre::new(
        fibre_colour(material, fibre.random),
        material.fuzz_roughness.max(roughness_floor),
        fibre.offset,
    )
}

/// The colour of the fibre with random number `random`: what Cycles'
/// translation computes from the same number at the same fibre.
pub fn fibre_colour(material: &Material, random: f32) -> tequila_core::spectrum::Spectrum {
    material.fuzz_color * (1.0 - material.fuzz_color_variation.clamp(0.0, 1.0) * random.clamp(0.0, 1.0))
}

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

    /// A fibre wears the fuzz colour, darkened by its own random number's
    /// share of the variation: none without variation, at most the whole
    /// variation for the darkest fibre, and the fuzz roughness raised to the
    /// path's floor.
    #[test]
    fn a_fibre_is_shaded_by_its_own_colour() {
        let velvet = Material {
            fuzz_color: Spectrum { r: 0.8, g: 0.1, b: 0.2 },
            fuzz_color_variation: 0.5,
            fuzz_roughness: 0.4,
            ..Material::default()
        };
        assert_eq!(fibre_colour(&velvet, 0.0), velvet.fuzz_color);
        assert!((fibre_colour(&velvet, 1.0).r - 0.4).abs() < 1e-6);
        assert!((fibre_colour(&velvet, 0.5).g - 0.075).abs() < 1e-6);
        let uniform = Material { fuzz_color_variation: 0.0, ..velvet.clone() };
        assert_eq!(fibre_colour(&uniform, 0.9), velvet.fuzz_color);

        // Darker fibres send on less of the light.
        let hit = |random| crate::scene::FibreHit { tangent: Vec3::X, facing: Vec3::Z, offset: 0.1, random };
        let wo = vec3(0.2, 0.0, 0.98).normalize();
        let light = fibre_bsdf(&velvet, &hit(0.0), 0.0).albedo(wo);
        let dark = fibre_bsdf(&velvet, &hit(1.0), 0.0).albedo(wo);
        assert!(dark.r < light.r, "{dark:?} against {light:?}");
    }

    #[test]
    fn a_default_material_is_a_plausible_mid_grey() {
        let m = Material::default();
        // 18% grey, the photographic reference.
        assert!((m.base_color.g - 0.18).abs() < 1e-6);
        assert!(!m.is_emissive());
    }
}
