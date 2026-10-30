//! Studio presets through the command bus: applying one frames the product,
//! puts the studio in place in one undo step, and leaves the product's
//! geometry and materials as they are (`docs/studio-presets.md`).

use tequila_core::math::{vec3, Mat4, Transform, Vec3};
use tequila_scene::lighting::LightSettings;
use tequila_scene::preset::{capture, ApplyOptions, StudioPreset};
use tequila_scene::studio::{LightOverride, LightTarget, Studio};
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus, SceneDocument};

fn demo() -> SceneDocument {
    tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3)
}

/// A studio worth a preset: two lights, two cameras, two views (the second
/// switching a light off and turning the product), a ground and a look.
fn studio(document: &mut SceneDocument, bus: &mut CommandBus) {
    for (name, layer, at) in [("Key", 0u8, vec3(0.0, 0.5, 0.6)), ("Strip", 1, vec3(0.5, 0.3, 0.0))] {
        bus.apply(document, Command::AddLight { parent: None, name: name.into(), settings: Box::new(LightSettings { layer, ..Default::default() }) })
            .unwrap();
        let id = document.tree.iter().find(|n| n.name == name).unwrap().id;
        bus.apply(document, Command::set_transform(id, Transform::translation(at))).unwrap();
    }
    let mut main = document.cameras.active().clone();
    main.name = "Main".into();
    bus.apply(document, Command::SetCamera { index: document.cameras.active_index(), camera: Box::new(main.clone()) }).unwrap();
    let mut top = main.clone();
    top.name = "Top".into();
    top.camera.camera_to_world = Mat4::look_at(vec3(0.0, 2.0, 0.01), Vec3::ZERO, Vec3::Y);
    top.target = Vec3::ZERO;
    bus.apply(document, Command::AddCamera { camera: Box::new(top.clone()) }).unwrap();
    document.environment.ground.reflections = 1.0;
    document.environment.ground.reflection_fade_distance = 0.0075;
    let views = vec![
        Studio {
            name: "Main".into(),
            camera: Some(main),
            lights: vec![LightOverride { target: LightTarget::Layer(1), enabled: true }],
            product_yaw: Some(0.0),
            ..Studio::default()
        },
        Studio {
            name: "Top".into(),
            camera: Some(top),
            lights: vec![LightOverride { target: LightTarget::Layer(1), enabled: false }],
            product_yaw: Some(0.5),
            ..Studio::default()
        },
    ];
    bus.apply(document, Command::SetStudios { studios: views }).unwrap();
}

fn lights(document: &SceneDocument) -> Vec<(String, bool)> {
    let mut all: Vec<(String, bool)> = document
        .tree
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Light { settings } => Some((n.name.clone(), settings.enabled)),
            _ => None,
        })
        .collect();
    all.sort();
    all
}

fn part_materials(document: &SceneDocument) -> Vec<(u64, Option<u64>)> {
    document.tree.iter().filter(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| (n.id.0, n.material.map(|m| m.0))).collect()
}

/// Applied to a bare product, the preset frames it - centred, its base at
/// the offset - and brings its lights (in a group of their own), cameras,
/// views and settings; the product keeps its materials; one undo takes it
/// all back, and redo puts it back under the same identifiers.
#[test]
fn a_preset_frames_the_product_and_undoes_in_one_step() {
    let mut source = demo();
    let mut bus = CommandBus::new();
    studio(&mut source, &mut bus);
    let mut preset: StudioPreset = capture(&source, "test-studio", "Test Studio", "Tests", "Small");
    preset.framing.base_offset = 0.0003;

    let mut document = demo();
    let mut bus = CommandBus::new();
    let before = document.clone();
    let materials = part_materials(&document);
    bus.apply(&mut document, Command::ApplyStudioPreset { preset: Box::new(preset.clone()), options: ApplyOptions::default() }).unwrap();

    let bounds = tequila_scene::snapshot::visible_bounds(&document);
    assert!(bounds.center().x.abs() < 1e-4 && bounds.center().z.abs() < 1e-4, "centred: {:?}", bounds.center());
    assert!((bounds.min.y - 0.0003).abs() < 1e-5, "base at the offset: {}", bounds.min.y);
    assert_eq!(lights(&document), vec![("Key".to_string(), true), ("Strip".to_string(), true)]);
    let group = document
        .tree
        .iter()
        .find(|n| n.metadata.contains_key(tequila_scene::studio::STUDIO_LIGHTS))
        .expect("a studio lights group")
        .clone();
    assert_eq!(group.children.len(), 2);
    let names: Vec<String> = document.cameras.iter().map(|(_, c)| c.name.clone()).collect();
    assert_eq!(names, vec!["Main".to_string(), "Top".to_string()]);
    assert_eq!(document.studios.len(), 2);
    assert_eq!(document.environment.ground.reflection_fade_distance, 0.0075);
    assert_eq!(part_materials(&document), materials, "the product keeps its materials");
    assert_eq!(bus.undo_step(), Some(("studio_preset.apply", 1)));

    // A view of it: the Top view turns the product and switches the strip.
    bus.apply(&mut document, Command::ApplyStudio { index: 1 }).unwrap();
    assert_eq!(lights(&document), vec![("Key".to_string(), true), ("Strip".to_string(), false)]);

    bus.undo(&mut document).unwrap();
    bus.undo(&mut document).unwrap();
    assert!(document.describes_the_same_scene_as(&before), "one undo took the preset back");
    let ids_before: Vec<u64> = before.tree.iter().map(|n| n.id.0).collect();
    assert_eq!(document.tree.iter().map(|n| n.id.0).collect::<Vec<_>>(), ids_before);

    bus.redo(&mut document).unwrap();
    let again = document.tree.iter().find(|n| n.metadata.contains_key(tequila_scene::studio::STUDIO_LIGHTS)).expect("redone");
    assert_eq!(again.id, group.id, "redo brings the lights back under their ids");
}

/// Save a studio as a preset and apply it to the same product: the same
/// studio, light for light and view for view, cameras where they were.
#[test]
fn saving_and_applying_a_preset_is_lossless() {
    let mut document = demo();
    let mut bus = CommandBus::new();
    studio(&mut document, &mut bus);
    let preset = capture(&document, "same", "Same", "Tests", "Small");
    let (lights_before, studios_before, environment_before) = (lights(&document), document.studios.clone(), document.environment.clone());
    let cameras_before: Vec<_> = document.cameras.iter().map(|(_, c)| c.clone()).collect();
    let options = ApplyOptions { keep_placement: true, ..ApplyOptions::default() };
    bus.apply(&mut document, Command::ApplyStudioPreset { preset: Box::new(preset), options }).unwrap();
    assert_eq!(lights(&document), lights_before);
    assert_eq!(document.studios, studios_before);
    assert_eq!(document.environment, environment_before);
    let cameras: Vec<_> = document.cameras.iter().map(|(_, c)| c.clone()).collect();
    assert_eq!(cameras.len(), cameras_before.len());
    for (a, b) in cameras.iter().zip(&cameras_before) {
        assert_eq!(a.name, b.name);
        for r in 0..4 {
            for c in 0..4 {
                assert!((a.camera.camera_to_world.m[r][c] - b.camera.camera_to_world.m[r][c]).abs() < 1e-5, "{} moved", a.name);
            }
        }
    }
}

/// A product twice the size gets the cameras twice as far, and frames the
/// same.
#[test]
fn a_bigger_product_takes_the_cameras_further() {
    let mut source = demo();
    let mut bus = CommandBus::new();
    studio(&mut source, &mut bus);
    let preset = capture(&source, "sized", "Sized", "Tests", "Small");
    // The Top camera: the demo's own sits at the origin.
    let distance = |document: &SceneDocument| {
        let c = document.cameras.get(1).unwrap();
        (c.camera.camera_to_world.transform_point(Vec3::ZERO) - c.target).length()
    };
    let mut small = demo();
    CommandBus::new()
        .apply(&mut small, Command::ApplyStudioPreset { preset: Box::new(preset.clone()), options: ApplyOptions::default() })
        .unwrap();
    let mut big = tequila_scene::demo::demo_document_sized(tequila_scene::demo::DemoMaterial::Plastic, 0.3, 2.0);
    CommandBus::new().apply(&mut big, Command::ApplyStudioPreset { preset: Box::new(preset), options: ApplyOptions::default() }).unwrap();
    let ratio = distance(&big) / distance(&small);
    assert!((ratio - 2.0).abs() < 0.02, "cameras {ratio:.3} times as far for a product twice the size");
}
