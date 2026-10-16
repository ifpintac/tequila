//! USD: Tequila's `.usda` export, read back by Tequila's `.usda` reader.
//! What goes out comes back - placement, geometry, and materials whole.

use tequila_import::{build, read_format, ImportOptions};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus, SceneDocument};

fn demo() -> (SceneDocument, CommandBus) {
    (tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3), CommandBus::new())
}

/// The part comes back where it was, with its triangles, and its material
/// with every parameter: a measured metal included.
#[test]
fn a_part_and_its_material_round_trip() {
    let (mut document, mut bus) = demo();
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let key = document.tree.get(part).unwrap().material.unwrap();
    bus.apply(&mut document, Command::SetMaterialType { material: key, material_type: MaterialType::Metal }).unwrap();
    let original = document.materials.get(key).unwrap().clone();

    let text = tequila_format::usd::export(&document);
    assert!(text.starts_with("#usda 1.0"));
    assert!(text.contains("UsdPreviewSurface") && text.contains(tequila_format::usd::OPEN_PBR));
    let scene = read_format(text.as_bytes(), "usda", "demo", &|_| None).expect("our own usda reads");

    let NodeKind::Part { mesh } = document.tree.get(part).unwrap().kind else { unreachable!() };
    let source = document.geometry.get(mesh).unwrap();
    let back = scene.meshes.iter().find(|m| m.indices.len() == source.indices.len()).expect("the part's triangles");
    let material = &scene.materials[back.material.expect("bound")];
    assert_eq!(material.name, original.name);
    assert_eq!(material.material_type, MaterialType::Metal);
    for param in Param::ALL {
        let (a, b) = (param.read(original.material()), param.read(material.material()));
        let close = match (a, b) {
            (Some(tequila_materials::material_type::ParamValue::Scalar(x)), Some(tequila_materials::material_type::ParamValue::Scalar(y))) => (x - y).abs() < 1e-4,
            (Some(tequila_materials::material_type::ParamValue::Color(x)), Some(tequila_materials::material_type::ParamValue::Color(y))) => (x.r - y.r).abs() < 1e-4 && (x.g - y.g).abs() < 1e-4 && (x.b - y.b).abs() < 1e-4,
            (Some(tequila_materials::material_type::ParamValue::Metal(x)), Some(tequila_materials::material_type::ParamValue::Metal(y))) => (x.n.r - y.n.r).abs() < 1e-3 && (x.k.b - y.k.b).abs() < 1e-3,
            (x, y) => x == y,
        };
        assert!(close, "{} did not survive: {:?} against {:?}", param.key(), a, b);
    }

    // Built into a document, the part stands where it stood.
    let mut copy = SceneDocument::new();
    let options = ImportOptions { place_on_ground: false, ..Default::default() };
    let (command, _) = build(&copy, None, scene, &options, None).unwrap();
    CommandBus::new().apply(&mut copy, command).unwrap();
    let placed = copy.tree.iter().find(|n| n.name == document.tree.get(part).unwrap().name && matches!(n.kind, NodeKind::Part { .. })).unwrap().id;
    let a = document.tree.world_transform(part).point(source.positions[0]);
    let NodeKind::Part { mesh: copied } = copy.tree.get(placed).unwrap().kind else { unreachable!() };
    let b = copy.tree.world_transform(placed).point(copy.geometry.get(copied).unwrap().positions[0]);
    let nearest = copy.geometry.get(copied).unwrap().positions.iter().map(|p| (copy.tree.world_transform(placed).point(*p) - a).length()).fold(f32::INFINITY, f32::min);
    assert!(nearest < 1e-4, "the part moved: {a:?} against {b:?}");
}

/// A procedural material comes back as the procedural, not its average.
#[test]
fn a_procedural_material_comes_back_whole() {
    let (mut document, mut bus) = demo();
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let key = document.tree.get(part).unwrap().material.unwrap();
    let wood = tequila_texture::procedural::Procedural::catalog().into_iter().find(|p| p.key() == "wood").unwrap();
    bus.apply(&mut document, Command::SetParameterProcedural { material: key, param: Param::BaseColor, procedural: Some(Box::new(wood)), seed: 4 }).unwrap();
    let text = tequila_format::usd::export(&document);
    assert!(text.contains("tequila:definition"));
    let scene = read_format(text.as_bytes(), "usda", "demo", &|_| None).unwrap();
    let back = scene.materials.iter().find(|m| m.name == document.materials.get(key).unwrap().name).unwrap();
    assert!(back.graph.len() > 1, "the wood came back as a flat colour");
    assert_eq!(back.graph.compile().unwrap().driven.len(), document.materials.get(key).unwrap().graph.compile().unwrap().driven.len());
}

/// A file nested far deeper than any scene is written is let go of, not
/// followed until the stack runs out and the application ends.
#[test]
fn a_value_nested_too_deep_is_not_followed() {
    let deep = format!("#usda 1.0\ndef Xform \"A\"\n{{\n    float x = {}1{}\n}}\n", "[".repeat(100_000), "]".repeat(100_000));
    let read = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || tequila_import::read_format(deep.as_bytes(), "usda", "deep", &|_| None).map(|_| ()).map_err(|e| e.to_string()))
        .unwrap()
        .join();
    assert!(read.is_ok(), "the reader overflowed its stack");
}
