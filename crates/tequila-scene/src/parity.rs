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

use crate::bench::{lamp_lit, product_camera, simple_product, simple_product_mapped};
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
}, ParityScene {
    name: "lamp_lit",
    covers: "R3 tier A, the lamps: the bench's lamp-lit product, the environment turned down to nothing and three              lamps in its place - a rectangular key and a disc fill, each with its stand-in the camera sees (the disc's              a square), and a spot rim weak enough beside them that the light hierarchy chooses - next-event              estimation of one lamp chosen down the hierarchy by importance, the rectangle by its spherical              rectangle (by area near its plane), the disc by area, the spot as a delta; shadow rays that pass the              stand-ins; the bounces off sharp reflections that meet the panels along their way (a draw of the open              stream between roughness 0.5 and 0.8), shared with the light sample by its choice from there; the              mirrors' views of them; Filter Glossy's blur of the mirror-smooth screen and lens past a rough bounce",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: lamp_lit,
}, ParityScene {
    name: "studio_lamps",
    covers: "R3 tier A, a studio: the product (no floor) under the environment and the studio kit's softbox, strip              and card - panels in nits, falling off to their edges (0.55, 0.45, 0.2), out of the camera's sight so              no stand-ins, yet shown in mirrors - three lamps of like power chosen by the alias table; the sky's and              a lamp's light samples summed and clamped as one",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: studio_lamps,
}, ParityScene {
    name: "lamps_in_view",
    covers: "R3 tier A, the stand-ins: the product on its modelled ground under the environment, lit by lamp_lit's              key and by two lamps the camera sees - a softbox behind the product facing the camera, falling off to              its edges and fading down its face, its stand-in glowing as the lamp lights (and mirrored in the              screen, the bezel and the ground by the lamp itself, the stand-in passed), and a disc turned away,              whose stand-in the camera sees from behind: a square, unlit",
    width: 320,
    height: 180,
    samples: 256,
    seed: 7,
    build: lamps_in_view,
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

/// `simple_product` under the environment and lamp_lit's key, with two lamps
/// in the camera's view: a softbox behind the product facing the camera,
/// up to its right, and a disc up to its left turned away from it.
fn lamps_in_view() -> SceneDocument {
    use crate::lighting::{Intensity, LightSettings, LightShape};
    use tequila_core::math::{vec3, Mat4, Transform, Vec3};
    let mut document = simple_product();
    let mut bus = crate::command::CommandBus::new();
    let camera = product_camera(&document, 320.0 / 180.0);
    let eye = camera.position();
    let product: Vec<_> = document
        .tree
        .iter()
        .filter(|n| matches!(n.kind, crate::tree::NodeKind::Part { .. }))
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_none_or(|m| m.name != "Ground"))
        .map(|n| n.id)
        .collect();
    let bounds = crate::snapshot::visible_bounds_under(&document, &product);
    let (middle, radius) = (bounds.center(), (bounds.diagonal().length() * 0.5).max(1e-4));
    let toward = vec3(eye.x - middle.x, 0.0, eye.z - middle.z).normalize();
    let right = Vec3::Y.cross(toward).normalize();
    // A frame whose +Z (the way a panel shines) is `facing`.
    let placed = |at: Vec3, facing: Vec3| {
        let z = facing.normalize();
        let x = Vec3::Y.cross(z).normalize();
        let y = z.cross(x);
        Transform::from_matrix(Mat4::translation(at) * Mat4::from_columns(x, y, z)).expect("a frame")
    };
    let quarter = std::f32::consts::FRAC_PI_2;
    let softbox_at = middle - toward * (1.8 * radius) + right * (1.2 * radius) + Vec3::Y * (0.6 * radius);
    let disc_at = middle - toward * (1.5 * radius) - right * (1.3 * radius) + Vec3::Y * (0.5 * radius);
    let lamps = [
        (
            "Key",
            LightSettings {
                shape: LightShape::Area { width: 1.5, height: 1.0, disc: false, two_sided: false },
                intensity: Intensity::Lumens(400.0),
                ..LightSettings::default()
            },
            Transform::translation(vec3(0.9, 2.2, 1.1)) * Transform::rotation(Vec3::X, quarter),
        ),
        (
            "Softbox",
            LightSettings {
                shape: LightShape::Area { width: radius, height: 0.6 * radius, disc: false, two_sided: false },
                intensity: Intensity::Nits(8.0),
                edge_falloff: 0.55,
                fade: 0.3,
                ..LightSettings::default()
            },
            placed(softbox_at, eye - softbox_at),
        ),
        (
            "Turned away",
            LightSettings {
                shape: LightShape::Area { width: 0.5 * radius, height: 0.5 * radius, disc: true, two_sided: false },
                intensity: Intensity::Nits(20.0),
                ..LightSettings::default()
            },
            placed(disc_at, disc_at - eye),
        ),
    ];
    for (name, settings, transform) in lamps {
        let before: Vec<_> = document.tree.iter().filter(|n| n.kind.light().is_some()).map(|n| n.id).collect();
        bus.apply(&mut document, Command::AddLight { parent: None, name: name.into(), settings: Box::new(settings) }).expect("a lamp");
        let id = document.tree.iter().find(|n| n.kind.light().is_some() && !before.contains(&n.id)).expect("the lamp just added").id;
        bus.apply(&mut document, Command::set_transform(id, transform)).expect("the lamp placed");
    }
    document
}

/// `simple_product` with no floor at all, under the environment, lit by the
/// studio kit's softbox, strip light and reflector card, each placed for the
/// set's camera as Lighting › Add to the studio places it.
fn studio_lamps() -> SceneDocument {
    use crate::studio_kit::{add, Piece};
    let (mut document, mut bus) = simple_product_mapped(&|material| material);
    let modelled: Vec<_> = document
        .tree
        .iter()
        .filter(|n| n.material.and_then(|m| document.materials.get(m)).is_some_and(|m| m.name == "Ground"))
        .map(|n| n.id)
        .collect();
    bus.apply(&mut document, Command::DeleteNodes { nodes: modelled }).expect("the modelled ground goes");
    let eye = product_camera(&document, 320.0 / 180.0).position();
    for piece in [Piece::Softbox, Piece::Strip, Piece::Card] {
        let (commands, _) = add(&document, piece, eye);
        for command in commands {
            bus.apply(&mut document, command).expect("the studio kit's piece goes in");
        }
    }
    document
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

    /// The lamp scenes are lit as they say: lamp_lit by its three lamps
    /// alone, the hierarchy choosing; the studio by three panels out of the
    /// camera's sight, falling off to their edges, the alias table choosing.
    #[test]
    fn the_lamp_scenes_are_lit_as_they_say() {
        use tequila_render_core::light::{LightKind, SelectionStrategy};
        let lit = |name: &str| {
            let document = scene(name).unwrap().document();
            let mut environments = crate::environment::EnvironmentCache::new();
            let environment = environments.environment(&document.environment);
            crate::snapshot::snapshot(&document, environment, &mut crate::snapshot::MeshCache::new()).0
        };
        let lamps = lit("lamp_lit");
        assert_eq!(lamps.lights.len(), 3);
        assert_eq!(lamps.lights.strategy(), SelectionStrategy::Hierarchy);
        assert_eq!(lamps.instances.iter().filter(|i| i.light_proxy).count(), 2, "the key's and the fill's stand-ins");
        assert!(lamps.environment.intensity() <= 0.0);
        let studio = lit("studio_lamps");
        assert_eq!(studio.lights.len(), 3);
        assert_eq!(studio.lights.strategy(), SelectionStrategy::Alias);
        assert!(studio.instances.iter().all(|i| !i.light_proxy && i.shadow_catcher.is_none()));
        for light in studio.lights.lights() {
            let LightKind::Area(area) = &light.kind else { panic!("a panel") };
            assert!(!area.is_even() && !light.visibility.camera && light.visibility.specular);
        }
        assert!(studio.environment.intensity() > 0.0);
        let in_view = lit("lamps_in_view");
        assert_eq!(in_view.lights.len(), 3);
        assert_eq!(in_view.instances.iter().filter(|i| i.light_proxy).count(), 3);
        assert!(in_view.environment.intensity() > 0.0);
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
