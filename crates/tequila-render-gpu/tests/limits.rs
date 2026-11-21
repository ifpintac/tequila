//! What the card renders otherwise, it says (`docs/07` R3): each sentence
//! of `part_limits`, `camera_limits` and the flakes of `glass_limits` is
//! said of a scene that has the thing, and none of them of the parity
//! set's grey product, which has none (the parity test holds every scene of
//! the set to that). No card is needed: the limits are read off the scene.

use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::cutaway::{Cutter, CutterShape};
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::settings::{IntegratorConfig, PixelFilter, PixelFilterKind};
use tequila_render_gpu::transport::{camera_limits, floor_limits, glass_limits, part_limits};
use tequila_scene::command::{Command, EnvParam, EnvValue, EnvironmentParameter};
use tequila_scene::parity;
use tequila_scene::snapshot::{snapshot, MeshCache};
use tequila_scene::{CommandBus, SceneDocument};

/// `document`'s snapshot with its background, as the command line takes it.
fn snapped(document: &SceneDocument) -> SceneSnapshot {
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);
    let mut scene = snapshot(document, environment, &mut MeshCache::new()).0;
    scene.background = environments.background(&document.environment);
    scene
}

fn grey() -> SceneDocument {
    parity::scene("grey_product").expect("the parity set has it").document()
}

/// `document` with material `name`'s parameters changed by `change`.
fn with_material(mut document: SceneDocument, name: &str, change: &dyn Fn(&mut tequila_materials::openpbr::Material)) -> SceneDocument {
    let key = document.materials.iter().find(|(_, m)| m.name == name).map(|(k, _)| k).expect("the material");
    let mut definition = document.materials.get(key).expect("the material").clone();
    change(definition.graph.material_mut());
    document.materials.insert_with_key(key, definition);
    document
}

fn with_environment(mut document: SceneDocument, parameter: EnvironmentParameter) -> SceneDocument {
    CommandBus::new().apply(&mut document, Command::SetEnvironmentParameter { parameter }).expect("the environment is set");
    document
}

#[test]
fn the_grey_product_has_none_of_them() {
    let document = grey();
    let scene = snapped(&document);
    let config = document.lighting.config();
    let camera = parity::scene("grey_product").unwrap().camera(&document);
    assert!(part_limits(&scene, &config).is_empty(), "{:?}", part_limits(&scene, &config));
    assert!(camera_limits(&camera, &config).is_empty(), "{:?}", camera_limits(&camera, &config));
    assert!(glass_limits(&scene, &config).is_empty(), "{:?}", glass_limits(&scene, &config));
}

#[test]
fn a_cutaway_is_said() {
    let document = grey();
    let scene = snapped(&document).with_cutters(vec![Cutter::new(CutterShape::Box, tequila_core::math::Transform::IDENTITY)]);
    assert_eq!(part_limits(&scene, &document.lighting.config()), vec!["a cutaway is left out: the parts it cuts are whole"]);
}

#[test]
fn fibres_are_said() {
    let document = with_material(grey(), "Foot", &|m| {
        m.fuzz_density = 20.0;
        m.fuzz_length = 0.006;
    });
    let scene = snapped(&document);
    assert!(scene.meshes.iter().any(|m| m.fibres.is_some()), "the feet grow fibres");
    assert_eq!(part_limits(&scene, &document.lighting.config()), vec!["fibres are left out: the parts that grow them are bare"]);
}

#[test]
fn a_dome_of_a_size_is_said() {
    let document = with_environment(grey(), EnvironmentParameter::scalar(EnvParam::Size, 6.0));
    let scene = snapped(&document);
    assert!(!scene.environment.dome().is_infinite(), "the dome has a size");
    assert_eq!(part_limits(&scene, &document.lighting.config()), vec!["the environment is an infinitely far dome rather than one of its size"]);
}

/// A background colour, and a backplate, are the card's as the processor's
/// (`past_the_scene`): neither is a limit any more, with or without a floor.
#[test]
fn a_background_colour_or_backplate_is_not_said() {
    use tequila_scene::document::BackgroundMode;
    let colour = with_environment(grey(), EnvironmentParameter::new(EnvParam::Background, EnvValue::Mode(BackgroundMode::Color)));
    let scene = snapped(&colour);
    assert!(matches!(scene.background, tequila_render_core::environment::Background::Color(_)));
    assert!(part_limits(&scene, &colour.lighting.config()).is_empty(), "{:?}", part_limits(&scene, &colour.lighting.config()));
    let plate = parity::scene("backplate").expect("the parity set has it").document();
    let mut scene = snapped(&plate);
    scene.background = parity::background(&plate);
    assert!(matches!(scene.background, tequila_render_core::environment::Background::Backplate { .. }));
    assert!(part_limits(&scene, &plate.lighting.config()).is_empty(), "{:?}", part_limits(&scene, &plate.lighting.config()));
    assert!(floor_limits(&scene, &plate.lighting.config()).is_empty(), "{:?}", floor_limits(&scene, &plate.lighting.config()));
}

#[test]
fn flakes_are_said() {
    let document = with_material(grey(), "Body", &|m| m.flake_weight = 0.5);
    let scene = snapped(&document);
    assert_eq!(glass_limits(&scene, &document.lighting.config()), vec!["metallic flakes are left out"]);
}

#[test]
fn the_camera_and_the_film_are_said() {
    let document = grey();
    let config = document.lighting.config();
    let pinhole = parity::scene("grey_product").unwrap().camera(&document);
    let said = |camera: Camera, config: &IntegratorConfig| camera_limits(&camera, config);
    assert_eq!(said(Camera { f_number: 2.8, ..pinhole }, &config), vec!["no depth of field: everything is in focus, as through a pinhole"]);
    assert_eq!(
        said(Camera { projection: Projection::Orthographic { scale: 1.0 }, ..pinhole }, &config),
        vec!["an orthographic camera is not drawn: the card's camera is a perspective one"]
    );
    assert_eq!(
        said(Camera { projection: Projection::Equirectangular, ..pinhole }, &config),
        vec!["a 360-degree camera is not drawn: the card's camera is a perspective one"]
    );
    assert_eq!(said(Camera { projection: Projection::Shift { focal_length_mm: 50.0, shift_x: 0.1, shift_y: 0.0 }, ..pinhole }, &config), vec!["a shifted lens is left unshifted"]);
    assert!(said(Camera { projection: Projection::Shift { focal_length_mm: 50.0, shift_x: 0.0, shift_y: 0.0 }, ..pinhole }, &config).is_empty());
    let soft = IntegratorConfig { pixel_filter: PixelFilter { kind: PixelFilterKind::Gaussian, width: 1.5 }, ..config.clone() };
    assert_eq!(said(pinhole, &soft), vec!["the pixel filter is one pixel's box"]);
    let wide = IntegratorConfig { pixel_filter: PixelFilter { kind: PixelFilterKind::Box, width: 2.0 }, ..config.clone() };
    assert_eq!(said(pinhole, &wide), vec!["the pixel filter is one pixel's box"]);
}
