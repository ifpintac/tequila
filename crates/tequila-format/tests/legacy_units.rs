//! Files saved before lengths were always metres.
//!
//! Until 2026-10-13 a document could keep its lengths in its own unit, with
//! `metres_per_unit` saying what one was worth. The readouts honoured the
//! factor and the renderer never read it, so a millimetre scene rendered a
//! thousand times its size. Such a file now opens in metres, scaled the way
//! Scale Scene scales a scene: the model the size the file said, the shot
//! framed and lit as it was.

use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::units::LengthUnit;
use tequila_format::{read, write, zip, Preserved};
use tequila_render_core::camera::Camera;
use tequila_scene::command::{reserve_keys, ImportPayload};
use tequila_scene::lighting::{Intensity, LightSettings, SavedCamera};
use tequila_scene::{Command, CommandBus, ConversionMode, Mesh, NodeKind, SceneDocument, SceneNode};

/// A part 121 x 25 x 110 across, lit from 200 above and seen from 300 out,
/// shown in millimetres: as the old build kept a millimetre scene, the
/// numbers in millimetres.
fn millimetre_numbers() -> SceneDocument {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let (meshes, _, ids) = reserve_keys(&document, 1, 0, 1);
    let mesh = Mesh::new(
        vec![vec3(0.0, 0.0, 0.0), vec3(121.0, 0.0, 0.0), vec3(121.0, 25.0, 110.0)],
        vec![0, 1, 2],
    );
    let node = SceneNode::new(ids[0], "Hangboard", NodeKind::Part { mesh: meshes[0] });
    let commands = vec![
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], mesh)],
                textures: Vec::new(),
                materials: Vec::new(),
                nodes: vec![node],
                roots: vec![ids[0]],
            }),
        },
        Command::AddLight { parent: None, name: "Key".into(), settings: Box::new(LightSettings::default()) },
        Command::SetCamera {
            index: 0,
            camera: Box::new(SavedCamera {
                name: "Camera 1".into(),
                camera: Camera::look_at(vec3(300.0, 200.0, 300.0), vec3(60.0, 12.0, 55.0), Vec3::Y),
                target: vec3(60.0, 12.0, 55.0),
            }),
        },
        Command::SetUnits { unit: LengthUnit::Millimetre, mode: ConversionMode::KeepSceneSize },
    ];
    for command in commands {
        bus.apply(&mut document, command).unwrap();
    }
    let light = light(&document);
    bus.apply(&mut document, Command::set_transform(light, Transform::translation(vec3(60.0, 200.0, 55.0)))).unwrap();
    document
}

fn light(document: &SceneDocument) -> tequila_scene::NodeId {
    document.tree.iter().find(|n| n.kind.light().is_some()).unwrap().id
}

/// The document saved as a build from before kept it: the same file, with
/// `metres_per_unit` saying what one of its numbers is.
fn saved_the_old_way(document: &SceneDocument, metres_per_unit: f64) -> Vec<u8> {
    let bytes = write(document, &Preserved::default());
    let archive = zip::ZipArchive::read(&bytes).unwrap();
    let mut old = zip::ZipWriter::new();
    for name in archive.names() {
        let mut member = archive.get(name).unwrap().to_vec();
        if name == "scene.json" {
            let text = String::from_utf8(member).unwrap();
            let key = text.find("\"metres_per_unit\"").expect("the factor is written");
            let colon = key + text[key..].find(':').unwrap() + 1;
            let end = colon + text[colon..].find(|c: char| c == ',' || c == '}').unwrap();
            member = format!("{}{metres_per_unit}{}", &text[..colon], &text[end..]).into_bytes();
        }
        old.add(name.clone(), member);
    }
    old.finish()
}

#[test]
fn a_millimetre_file_opens_in_metres() {
    let document = millimetre_numbers();
    let loaded = read(&saved_the_old_way(&document, 0.001)).unwrap();
    let opened = &loaded.document;

    assert_eq!(opened.units.unit, LengthUnit::Millimetre);
    let bounds = tequila_scene::snapshot::visible_bounds(opened);
    let size = bounds.max - bounds.min;
    assert!((size - vec3(0.121, 0.025, 0.110)).abs().max_component() < 1e-6, "the part is {size:?} m");
    let saved = opened.cameras.active();
    assert!((saved.camera.position() - vec3(0.3, 0.2, 0.3)).length() < 1e-6, "{:?}", saved.camera.position());
    assert!((saved.target - vec3(0.06, 0.012, 0.055)).length() < 1e-6);
    let light = light(opened);
    assert!((opened.tree.world_transform(light).point(Vec3::ZERO) - vec3(0.06, 0.2, 0.055)).length() < 1e-6);
    // A thousandth the distance from a millionth the area: a millionth the
    // power lights the part as it was lit.
    let Intensity::Lumens(lumens) = opened.tree.get(light).unwrap().kind.light().unwrap().intensity else {
        panic!("still in lumens")
    };
    assert!((lumens - 1500.0e-6).abs() < 1e-9, "{lumens} lm");
    assert!(
        loaded.notes.iter().any(|n| n.contains("converted to metres")),
        "nothing said it was converted: {:?}",
        loaded.notes
    );

    // Saved again, it is in metres and opens as it was saved.
    let again = read(&write(opened, &loaded.preserved)).unwrap();
    assert_eq!(again.document.difference_from(opened), None);
    assert!(!again.notes.iter().any(|n| n.contains("converted")), "{:?}", again.notes);
}

/// A file whose factor is 1 - every file written since, and every file in
/// metres or with no unit before - is not touched.
#[test]
fn a_file_in_metres_opens_as_it_was() {
    let document = millimetre_numbers();
    let loaded = read(&saved_the_old_way(&document, 1.0)).unwrap();
    assert_eq!(loaded.document.difference_from(&document), None);
    assert!(!loaded.notes.iter().any(|n| n.contains("converted")), "{:?}", loaded.notes);
}
