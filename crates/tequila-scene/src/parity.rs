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
use crate::command::{Command, EnvParam, EnvValue, EnvironmentParameter};
use crate::SceneDocument;
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::camera::Camera;
use tequila_render_core::material::Material;
use tequila_render_core::settings::IntegratorConfig;

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
    covers: "R3 tier A, the surface: the OpenPBR stack's opaque lobes - plastic (a specular interface over a \
             diffuse substrate, with two mirror-smooth ones until the roughness floor), aluminium and gold \
             metal, a diffuse foot - with the stack's lobe choice and aim, mirror bounces' MIS and the \
             specular chain's clamps",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: simple_product,
}, ParityScene {
    name: "product_floor",
    covers: "R3 tier A, the floor: product_materials standing on the environment's ground, a shadow catcher with \
             the Headphones' settings - shadows in black, reflections at 0.35, 3 m across - its shadow drawn from \
             the sky's light onto a floor facing up, its mirror reflection a path of its own (three bounces, the \
             same sampler), the backdrop behind it and the coverage it takes, the catcher's view through mirrors \
             and near mirrors (a draw of the open stream), and a floor a bounce meets passed straight through",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor,
}, ParityScene {
    name: "product_floor_glossy",
    covers: "R3 tier A, the floor's other reflection: blurred (a GGX facet of roughness 0.3 drawn on the floor's \
             bounce dimensions), weakened by Fresnel, fading out by half the product's height, over a tinted \
             shadow",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor_glossy,
}, ParityScene {
    name: "product_floor_studio",
    covers: "R3 tier A, the studio floor: contact shadows by ambient occlusion rather than the sky, a bounce the \
             product sees and the camera does not (glowing, darker toward the contact), and a reflection fading by \
             how near the product is to the floor point",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor_studio,
}, ParityScene {
    name: "product_floor_lit",
    covers: "R3 tier A, ground illumination: the default ground's shadow, with the floor a bounce meets shaded as \
             the unassigned grey rather than passed",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: product_floor_lit,
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

/// `simple_product` in its own materials with its modelled ground taken
/// away, standing on the environment's ground instead, set as `ground` says.
/// The two floors lie in one plane, so only one may be there (see
/// `simple_product_mapped`).
fn on_the_environments_ground(ground: &[EnvironmentParameter]) -> SceneDocument {
    on_the_environments_ground_with(ground, None)
}

/// The same, and under `config` when given rather than the Product preset.
fn on_the_environments_ground_with(ground: &[EnvironmentParameter], config: Option<IntegratorConfig>) -> SceneDocument {
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let modelled: Vec<_> = document
        .tree
        .iter()
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_some_and(|m| m.name == "Ground"))
        .map(|n| n.id)
        .collect();
    assert_eq!(modelled.len(), 1, "simple_product has one modelled ground");
    bus.apply(&mut document, Command::DeleteNodes { nodes: modelled }).expect("the modelled ground goes");
    let on = [EnvironmentParameter::flag(EnvParam::GroundEnabled, true), EnvironmentParameter::flag(EnvParam::GroundShadows, true)];
    for parameter in on.into_iter().chain(ground.iter().cloned()) {
        bus.apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the environment's ground is set");
    }
    if let Some(config) = config {
        bus.apply(&mut document, Command::SetLightingConfig { config: Box::new(config) }).expect("the lighting is set");
    }
    document
}

/// The Headphones' floor: black shadows, reflections at 0.35, 3 m across.
fn product_floor() -> SceneDocument {
    on_the_environments_ground(&[
        EnvironmentParameter::color(EnvParam::GroundShadowColor, Spectrum::ZERO),
        EnvironmentParameter::scalar(EnvParam::GroundReflections, 0.35),
        EnvironmentParameter::scalar(EnvParam::GroundSize, 3.0),
    ])
}

/// A glossy floor: a blurred reflection, weakened by Fresnel, fading out by
/// half the product's height, over a tinted shadow.
fn product_floor_glossy() -> SceneDocument {
    on_the_environments_ground(&[
        EnvironmentParameter::color(EnvParam::GroundShadowColor, Spectrum { r: 0.05, g: 0.03, b: 0.02 }),
        EnvironmentParameter::scalar(EnvParam::GroundReflections, 0.8),
        EnvironmentParameter::scalar(EnvParam::GroundReflectionBlur, 0.3),
        EnvironmentParameter::scalar(EnvParam::GroundReflectionLength, 0.5),
        EnvironmentParameter::flag(EnvParam::GroundReflectionFresnel, true),
        EnvironmentParameter::scalar(EnvParam::GroundSize, 3.0),
    ])
}

/// A studio floor: contact shadows by ambient occlusion, a bounce glowing
/// for the product and darker toward the contact, and a reflection fading by
/// distance.
fn product_floor_studio() -> SceneDocument {
    on_the_environments_ground(&[
        EnvironmentParameter::flag(EnvParam::GroundOcclusion, true),
        EnvironmentParameter::flag(EnvParam::GroundBounce, true),
        EnvironmentParameter::scalar(EnvParam::GroundBounceStrength, 0.8),
        EnvironmentParameter::scalar(EnvParam::GroundBounceContactFalloff, 0.15),
        EnvironmentParameter::scalar(EnvParam::GroundReflections, 0.5),
        EnvironmentParameter::scalar(EnvParam::GroundReflectionFadeDistance, 0.08),
        EnvironmentParameter::new(EnvParam::GroundReflectionFadeBy, EnvValue::Choice("distance")),
        EnvironmentParameter::scalar(EnvParam::GroundSize, 3.0),
    ])
}

/// The default ground under the Product preset with ground illumination on.
fn product_floor_lit() -> SceneDocument {
    on_the_environments_ground_with(&[], Some(IntegratorConfig { ground_illumination: true, ..IntegratorConfig::product() }))
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
        assert!(super::scene("product_floor").is_some());
    }

    /// The floor scenes stand on the environment's ground alone, as set.
    #[test]
    fn the_floor_scenes_stand_on_the_environments_ground() {
        for name in ["product_floor", "product_floor_glossy", "product_floor_studio", "product_floor_lit"] {
            let document = scene(name).unwrap().document();
            let ground = &document.environment.ground;
            assert!(ground.enabled && ground.shadows, "{name}");
            assert!(
                document.tree.iter().all(|n| n.material.and_then(|m| document.materials.get(m)).is_none_or(|m| m.name != "Ground")),
                "{name} keeps no modelled ground"
            );
        }
        let floor = scene("product_floor").unwrap().document();
        assert_eq!((floor.environment.ground.reflections, floor.environment.ground.size), (0.35, 3.0));
        assert!(scene("product_floor_lit").unwrap().document().lighting.config().ground_illumination);
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
