//! The parity set (`docs/07` R0.3): the scenes Tequila's renderer on the
//! graphics card is held to its renderer on the processor with.
//!
//! Small, so they render in seconds on a processor and can be checked on
//! every pull request, and fixed - size, samples and seed - so the two
//! backends trace the same paths with the same random numbers and their
//! pictures can be held to a tight tolerance rather than a statistical
//! one (`docs/07` D7, R2.6). Between them they are to use every feature in
//! R3's tiers: R2 starts the set with one scene, and each R3 step adds the
//! scene for the feature it ports, here, with what it covers.
//!
//! Generated, as the benchmark scenes are (`bench`), so they are the same
//! on every machine and in every version.

use crate::bench::{product_camera, simple_product, simple_product_mapped};
use crate::SceneDocument;
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::camera::Camera;
use tequila_render_core::material::Material;

/// One scene of the set, and how it is rendered.
#[derive(Debug)]
pub struct ParityScene {
    pub name: &'static str,
    /// What in `docs/07` R3 it exercises.
    pub covers: &'static str,
    pub width: usize,
    pub height: usize,
    pub samples: u32,
    pub seed: u32,
    build: fn() -> SceneDocument,
}

impl ParityScene {
    pub fn document(&self) -> SceneDocument {
        (self.build)()
    }

    /// The camera it is seen from: framed on its product.
    pub fn camera(&self, document: &SceneDocument) -> Camera {
        product_camera(document, self.width as f32 / self.height as f32)
    }
}

/// The set.
pub const SET: &[ParityScene] = &[ParityScene {
    name: "grey_product",
    covers: "R2.5: the camera, instanced triangles, one grey diffuse surface, the environment sampled with next-event \
             estimation and MIS, Russian roulette and the bounce limit",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: grey_product,
}, ParityScene {
    name: "product_materials",
    covers: "R3 tier A, the surface: the OpenPBR stack's opaque lobes - plastic (a specular interface over a              diffuse substrate, with two mirror-smooth ones until the roughness floor), aluminium and gold              metal, a diffuse foot - with the stack's lobe choice and aim, mirror bounces' MIS and the              specular chain's clamps",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: simple_product,
}];

/// The scene of the set by `name`.
pub fn scene(name: &str) -> Option<&'static ParityScene> {
    SET.iter().find(|s| s.name == name)
}

/// `simple_product` with every material grey and diffuse, under the
/// default environment: what R2.5's first transport can render.
fn grey_product() -> SceneDocument {
    let grey = Material::diffuse(Spectrum::splat(0.5));
    simple_product_mapped(&|(name, _, _)| (name, MaterialType::Diffuse, grey.clone())).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_is_named_once_and_builds() {
        for (i, scene) in SET.iter().enumerate() {
            assert!(SET[..i].iter().all(|s| s.name != scene.name), "{} twice", scene.name);
            assert!(!scene.covers.is_empty());
            let document = scene.document();
            document.validate().unwrap();
            assert!(document.difference_from(&scene.document()).is_none(), "{} differs between builds", scene.name);
        }
        assert!(super::scene("grey_product").is_some());
        assert!(super::scene("product_materials").is_some());
    }

    #[test]
    fn the_grey_product_is_grey_and_diffuse_everywhere() {
        let document = scene("grey_product").unwrap().document();
        assert!(document.materials.iter().count() >= 8);
        for (_, material) in document.materials.iter() {
            assert_eq!(material.material_type, MaterialType::Diffuse, "{}", material.name);
            assert_eq!(material.material().base_color, Spectrum::splat(0.5), "{}", material.name);
        }
    }
}
