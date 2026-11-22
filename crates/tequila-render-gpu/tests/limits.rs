//! What the card renders otherwise, it says (`docs/07` R3): each sentence
//! of `part_limits` and `camera_limits` is said of a scene that has the
//! thing, and what the card now renders as the processor does (flakes, a
//! dome, a background) is not, and none of them of the parity
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
fn a_cutaway_is_not_said() {
    use tequila_core::math::vec3;
    let document = grey();
    // A box, and a part's own closed mesh made a cutter, are cut by on the
    // card as on the processor.
    let shape = Cutter::new(CutterShape::Box, tequila_core::math::Transform::IDENTITY);
    let scene = snapped(&document).with_cutters(vec![shape.clone()]);
    assert!(part_limits(&scene, &document.lighting.config()).is_empty(), "{:?}", part_limits(&scene, &document.lighting.config()));
    let tetrahedron = tequila_render_core::cutaway::CutMesh::new(vec![
        [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
        [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0)],
        [vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0)],
    ]);
    let mesh = Cutter::new(CutterShape::Mesh(std::sync::Arc::new(tetrahedron)), tequila_core::math::Transform::IDENTITY);
    let scene = snapped(&document).with_cutters(vec![shape, mesh]);
    assert!(part_limits(&scene, &document.lighting.config()).is_empty(), "{:?}", part_limits(&scene, &document.lighting.config()));
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

/// A dome of a size is the card's as the processor's, under a floor too.
#[test]
fn a_dome_of_a_size_is_not_said() {
    let document = with_environment(parity::scene("finite_dome").expect("the parity set has it").document(), EnvironmentParameter::scalar(EnvParam::Size, 6.0));
    let scene = snapped(&document);
    assert!(!scene.environment.dome().is_infinite(), "the dome has a size");
    assert!(part_limits(&scene, &document.lighting.config()).is_empty(), "{:?}", part_limits(&scene, &document.lighting.config()));
    assert!(floor_limits(&scene, &document.lighting.config()).is_empty(), "{:?}", floor_limits(&scene, &document.lighting.config()));
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

/// Glass that disperses light is the card's as the processor's, in an RGB
/// render (`dispersion.wgsl`) and by wavelength (`spectral.wgsl`), and so
/// is a render by wavelength of a scene that disperses nothing.
#[test]
fn dispersion_and_wavelengths_are_not_said() {
    use tequila_render_core::settings::SpectralMode;
    let document = with_material(grey(), "Body", &|m| {
        m.transmission_weight = 1.0;
        m.transmission_dispersion_abbe = 20.0;
    });
    let scene = snapped(&document);
    let rgb = IntegratorConfig { spectral: SpectralMode::Off, ..document.lighting.config() };
    assert!(glass_limits(&scene, &rgb).is_empty(), "{:?}", glass_limits(&scene, &rgb));
    let auto = IntegratorConfig { spectral: SpectralMode::Auto, ..document.lighting.config() };
    assert!(glass_limits(&scene, &auto).is_empty(), "{:?}", glass_limits(&scene, &auto));
    let grey = grey();
    let on = IntegratorConfig { spectral: SpectralMode::On, ..grey.lighting.config() };
    assert!(glass_limits(&snapped(&grey), &on).is_empty(), "{:?}", glass_limits(&snapped(&grey), &on));
}

/// A walk through a medium is the card's as the processor's in an RGB
/// render; by wavelength the processor walks each of the path's four in its
/// own medium, where the card's walk carries three channels: said there,
/// and only there.
#[test]
fn a_walk_by_wavelength_is_said() {
    use tequila_render_core::settings::SpectralMode;
    for name in ["subsurface_product", "scattering_volumes"] {
        let document = parity::scene(name).expect("the parity set has it").document();
        let scene = snapped(&document);
        for spectral in [SpectralMode::Off, SpectralMode::Auto] {
            let config = IntegratorConfig { spectral, ..document.lighting.config() };
            assert!(glass_limits(&scene, &config).is_empty(), "{name}: {:?}", glass_limits(&scene, &config));
        }
        let on = IntegratorConfig { spectral: SpectralMode::On, ..document.lighting.config() };
        let said = glass_limits(&scene, &on);
        assert!(said.iter().any(|s| s.contains("walked in three channels")), "{name}: {said:?}");
    }
}

/// A thin film is the card's as the processor's (`film.wgsl`): no limit.
#[test]
fn a_thin_film_is_not_said() {
    let document = with_material(grey(), "Body", &|m| {
        m.thin_film_weight = 1.0;
        m.thin_film_thickness = 400.0;
    });
    let scene = snapped(&document);
    assert!(glass_limits(&scene, &document.lighting.config()).is_empty(), "{:?}", glass_limits(&scene, &document.lighting.config()));
}

/// Flakes are the card's as the processor's (`flakes.wgsl`): no limit.
#[test]
fn flakes_are_not_said() {
    let document = with_material(grey(), "Body", &|m| m.flake_weight = 0.5);
    let scene = snapped(&document);
    assert!(glass_limits(&scene, &document.lighting.config()).is_empty(), "{:?}", glass_limits(&scene, &document.lighting.config()));
}

#[test]
fn the_camera_and_the_film_are_said() {
    let document = grey();
    let config = document.lighting.config();
    let pinhole = parity::scene("grey_product").unwrap().camera(&document);
    let said = |camera: Camera, config: &IntegratorConfig| camera_limits(&camera, config);
    // Depth of field is the card's, round or bladed.
    assert!(said(Camera { f_number: 2.8, ..pinhole }, &config).is_empty());
    assert!(said(Camera { f_number: 2.8, aperture: tequila_render_core::camera::Aperture::Bladed { blades: 5, rotation: 0.3 }, ..pinhole }, &config).is_empty());
    // And every projection.
    assert!(said(Camera { projection: Projection::Orthographic { scale: 1.0 }, ..pinhole }, &config).is_empty());
    assert!(said(Camera { projection: Projection::Equirectangular, ..pinhole }, &config).is_empty());
    assert!(said(Camera { projection: Projection::Shift { focal_length_mm: 50.0, shift_x: 0.1, shift_y: 0.0 }, ..pinhole }, &config).is_empty());
    assert!(said(Camera { projection: Projection::Shift { focal_length_mm: 50.0, shift_x: 0.0, shift_y: 0.0 }, ..pinhole }, &config).is_empty());
    // And every pixel filter.
    let soft = IntegratorConfig { pixel_filter: PixelFilter { kind: PixelFilterKind::Gaussian, width: 1.5 }, ..config.clone() };
    assert!(said(pinhole, &soft).is_empty());
    let wide = IntegratorConfig { pixel_filter: PixelFilter { kind: PixelFilterKind::Box, width: 2.0 }, ..config.clone() };
    assert!(said(pinhole, &wide).is_empty());
}
