//! USDZ export (`tequila_format::usdz`): the package as Apple's devices need
//! it, the stage in it, its scale and place, its materials, its instancing,
//! and what Tequila's own importer makes of it again.
//!
//! Pixar's checker (`usdchecker --arkit`, `UsdUtils.ComplianceChecker`)
//! was run on these packages through the USD Blender 5.2 bundles; that is
//! outside this suite, and noted in `docs/03` §6.3.

use std::path::Path;
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_format::usdc::{self, Layer, SpecKind, Value};
use tequila_format::usdz::{export, UsdzOptions};
use tequila_import::{build, read_format, ImportOptions};
use tequila_materials::graph::NodeKind as GraphNode;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_scene::command::{reserve_keys, ImportPayload};
use tequila_scene::material::MaterialDefinition;
use tequila_scene::tree::{NodeKind, SceneNode};
use tequila_scene::{Command, CommandBus, SceneDocument};

fn demo() -> SceneDocument {
    tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3)
}

fn sample(name: &str) -> SceneDocument {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples").join(name);
    tequila_format::load(&path).unwrap_or_else(|e| panic!("{name}: {e}")).document
}

/// Each member of a ZIP as its local header has it: name, compression
/// method, and where its data starts.
fn members(zip: &[u8]) -> Vec<(String, u16, usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 30 <= zip.len() && zip[at..at + 4] == [0x50, 0x4b, 0x03, 0x04] {
        let u16_at = |i: usize| u16::from_le_bytes([zip[i], zip[i + 1]]) as usize;
        let method = u16_at(at + 8) as u16;
        let size = u32::from_le_bytes(zip[at + 18..at + 22].try_into().unwrap()) as usize;
        let name_length = u16_at(at + 26);
        let extra = u16_at(at + 28);
        let name = String::from_utf8_lossy(&zip[at + 30..at + 30 + name_length]).into_owned();
        let data = at + 30 + name_length + extra;
        out.push((name, method, data, size));
        at = data + size;
    }
    out
}

fn stage(zip: &[u8]) -> Layer {
    let (name, _, data, size) = members(zip).into_iter().next().expect("a first member");
    assert!(name.ends_with(".usdc"), "the stage comes first: {name}");
    usdc::read(&zip[data..data + size]).expect("the stage reads")
}

fn field<'a>(layer: &'a Layer, path: &str, name: &str) -> Option<&'a Value> {
    layer.spec(path).and_then(|s| s.field(name))
}

fn default<'a>(layer: &'a Layer, attribute: &str) -> Option<&'a Value> {
    field(layer, attribute, "default")
}

fn connection(layer: &Layer, attribute: &str) -> Option<String> {
    match field(layer, attribute, "connectionPaths") {
        Some(Value::Paths(op)) => op.items.first().cloned(),
        _ => None,
    }
}

/// The prims of a type, by path.
fn prims<'a>(layer: &'a Layer, kind: &str) -> Vec<&'a str> {
    layer
        .specs
        .iter()
        .filter(|s| s.kind == SpecKind::Prim && s.field("typeName") == Some(&Value::Token(kind.into())))
        .map(|s| s.path.as_str())
        .collect()
}

/// The material a prim binds.
fn bound(layer: &Layer, prim: &str) -> Option<String> {
    match field(layer, &format!("{prim}.material:binding"), "targetPaths") {
        Some(Value::Paths(op)) => op.items.first().cloned(),
        _ => None,
    }
}

/// Every member stored, not compressed, its data on a 64-byte boundary, the
/// binary stage first and only PNGs after it: what a USDZ is.
#[test]
fn the_package_is_stored_aligned_and_starts_with_the_stage() {
    let mut document = demo();
    wood(&mut document);
    let exported = export(&document, &UsdzOptions { bake_size: 128, ..Default::default() });
    let list = members(&exported.bytes);
    assert!(list.len() >= 2, "the stage and its baked pictures: {list:?}");
    for (name, method, data, _) in &list {
        assert_eq!(*method, 0, "{name} is stored");
        assert_eq!(data % 64, 0, "{name}'s data starts at {data}");
    }
    assert!(list[0].0.ends_with(".usdc"));
    assert_eq!(&exported.bytes[list[0].2..list[0].2 + 8], b"PXR-USDC");
    for (name, _, data, _) in &list[1..] {
        assert!(name.starts_with("textures/") && name.ends_with(".png"), "{name}");
        assert_eq!(&exported.bytes[*data..data + 4], b"\x89PNG");
    }
    // An ordinary ZIP reader opens it too, through its directory.
    let zip = tequila_format::zip::ZipArchive::read(&exported.bytes).expect("a valid ZIP");
    assert_eq!(zip.names(), exported.members.as_slice());
}

/// Metres, Y up, a default prim of kind component - and the product moved
/// to stand on the ground with its middle over the origin, at its own size.
#[test]
fn the_stage_is_real_size_and_stands_on_the_ground() {
    let document = sample("Perfume bottle.tequila");
    let exported = export(&document, &UsdzOptions { bake_size: 64, ..Default::default() });
    let layer = stage(&exported.bytes);
    assert_eq!(field(&layer, "/", "metersPerUnit"), Some(&Value::Double(1.0)));
    assert_eq!(field(&layer, "/", "upAxis"), Some(&Value::Token("Y".into())));
    let Some(Value::Token(name)) = field(&layer, "/", "defaultPrim") else { panic!("a default prim") };
    let root = format!("/{name}");
    assert_eq!(field(&layer, &root, "kind"), Some(&Value::Token("component".into())));
    assert_eq!(field(&layer, &root, "typeName"), Some(&Value::Token("Xform".into())));

    // The document's own box, in the world.
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for node in document.tree.iter() {
        let NodeKind::Part { mesh } = node.kind else { continue };
        if !document.is_shown(node.id) {
            continue;
        }
        let world = document.tree.world_transform(node.id);
        for p in &document.geometry.get(mesh).unwrap().positions {
            lo = lo.min(world.point(*p));
            hi = hi.max(world.point(*p));
        }
    }
    let size = hi - lo;
    assert!(size.y > 0.05 && size.y < 0.5, "a perfume bottle is a hand's height: {size:?} m");
    let Some(Value::Vecd(t)) = default(&layer, &format!("{root}.xformOp:translate")) else { panic!("the product is moved") };
    let moved = vec3(t[0] as f32, t[1] as f32, t[2] as f32);
    assert!((moved - vec3(-(lo.x + hi.x) / 2.0, -lo.y, -(lo.z + hi.z) / 2.0)).length() < 1e-5, "{moved:?}");
    let (a, b) = exported.bounds.expect("bounds");
    assert!(a.y.abs() < 1e-5, "standing on the ground: {a:?}");
    assert!((a.x + b.x).abs() < 1e-5 && (a.z + b.z).abs() < 1e-5, "over the origin: {a:?} {b:?}");
    assert!(((b - a) - size).length() < 1e-5, "at its own size");

    // Every part shown is a mesh with its normals and a material.
    let meshes = prims(&layer, "Mesh");
    let shown = document.tree.iter().filter(|n| matches!(n.kind, NodeKind::Part { .. }) && document.is_shown(n.id)).count();
    assert_eq!(meshes.len(), shown);
    for mesh in &meshes {
        assert!(matches!(default(&layer, &format!("{mesh}.points")), Some(Value::Vecfs(3, _))), "{mesh}");
        assert!(matches!(default(&layer, &format!("{mesh}.normals")), Some(Value::Vecfs(3, _))), "{mesh}");
        assert!(bound(&layer, mesh).is_some(), "{mesh} wears a material");
        let schemas = field(&layer, mesh, "apiSchemas");
        assert!(matches!(schemas, Some(Value::TokenList(op)) if op.items == ["MaterialBindingAPI"]), "{mesh}");
    }
}

/// A plain material is a preview surface of plain values: a metal's colour
/// its measured reflectance, glass mostly see-through, every material's
/// whole definition carried for Tequila.
#[test]
fn plain_materials_are_preview_surfaces() {
    let document = sample("Perfume bottle.tequila");
    let exported = export(&document, &UsdzOptions::default());
    assert_eq!(exported.members.len(), 1, "nothing to bake: {:?}", exported.members);
    let layer = stage(&exported.bytes);
    for material in prims(&layer, "Material") {
        let surface = format!("{material}/PreviewSurface");
        assert_eq!(default(&layer, &format!("{surface}.info:id")), Some(&Value::Token("UsdPreviewSurface".into())));
        assert_eq!(connection(&layer, &format!("{material}.outputs:surface")), Some(format!("{surface}.outputs:surface")));
        assert!(matches!(default(&layer, &format!("{material}.tequila:definition")), Some(Value::String(_))));
        let Some(Value::String(name)) = default(&layer, &format!("{material}.tequila:name")) else { panic!() };
        let definition = document.materials.iter().find(|(_, d)| &d.name == name).map(|(_, d)| d.clone()).expect("a material of the document");
        let m = definition.material();
        let float = |input: &str| match default(&layer, &format!("{surface}.inputs:{input}")) {
            Some(Value::Float(v)) => *v,
            other => panic!("{material} {input}: {other:?}"),
        };
        assert!((float("roughness") - m.specular_roughness).abs() < 1e-6, "{name}");
        assert!((float("metallic") - m.base_metalness).abs() < 1e-6, "{name}");
        let Some(Value::Vecf(colour)) = default(&layer, &format!("{surface}.inputs:diffuseColor")) else { panic!() };
        if m.base_metalness > 0.5 {
            // The measured metal's own colour, not the white its base is.
            let reflectance = m.solid_color().to_linear_srgb();
            assert!((colour[0] - reflectance.x).abs() < 1e-3 && (colour[2] - reflectance.z).abs() < 1e-3, "{name}: {colour:?}");
        }
        if m.transmission_weight > 0.99 {
            assert!(float("opacity") < 0.5, "{name} is see-through");
        }
    }
}

fn wood(document: &mut SceneDocument) {
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let key = document.tree.get(part).unwrap().material.unwrap();
    let wood = tequila_texture::procedural::Procedural::catalog().into_iter().find(|p| p.key() == "wood").unwrap();
    CommandBus::new()
        .apply(document, Command::SetParameterProcedural { material: key, param: Param::BaseColor, procedural: Some(Box::new(wood)), seed: 1 })
        .unwrap();
}

/// A procedural is baked: the colour into a picture on the part's UVs,
/// read through a UV texture in sRGB, while what does not vary - the
/// roughness - stays a plain value.
#[test]
fn a_procedural_is_baked_where_it_varies() {
    let mut document = demo();
    wood(&mut document);
    let exported = export(&document, &UsdzOptions { bake_size: 128, ..Default::default() });
    let layer = stage(&exported.bytes);
    let material = prims(&layer, "Material")[0].to_string();
    let surface = format!("{material}/PreviewSurface");
    let image = connection(&layer, &format!("{surface}.inputs:diffuseColor")).expect("the colour is a picture");
    let (shader, output) = image.split_once(".outputs:").unwrap();
    assert_eq!(output, "rgb");
    assert_eq!(default(&layer, &format!("{shader}.info:id")), Some(&Value::Token("UsdUVTexture".into())));
    assert_eq!(default(&layer, &format!("{shader}.inputs:sourceColorSpace")), Some(&Value::Token("sRGB".into())));
    let Some(Value::Asset(file)) = default(&layer, &format!("{shader}.inputs:file")) else { panic!("a file") };
    assert!(exported.members.contains(file), "{file} is in the package");
    assert!(matches!(default(&layer, &format!("{surface}.inputs:roughness")), Some(Value::Float(r)) if (r - 0.3).abs() < 0.01));
    assert_eq!(connection(&layer, &format!("{shader}.inputs:st")), Some(format!("{material}/TexCoord.outputs:result")));
    assert_eq!(default(&layer, &format!("{material}/TexCoord.inputs:varname")), Some(&Value::String("st".into())));
    let mesh = prims(&layer, "Mesh")[0];
    assert!(matches!(default(&layer, &format!("{mesh}.primvars:st")), Some(Value::Vecfs(2, _))));

    // The picture varies, as wood does.
    let zip = tequila_format::zip::ZipArchive::read(&exported.bytes).unwrap();
    let decoder = png::Decoder::new(zip.get(file).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    let (lo, hi) = pixels.iter().fold((255u8, 0u8), |(l, h), p| (l.min(*p), h.max(*p)));
    assert!(hi - lo > 30, "the baked wood is not flat: {lo}..{hi}");
}

/// A bump becomes a normal map, read raw and scaled to -1..1 as the
/// checker requires, in the frame of the UVs the mesh carries.
#[test]
fn a_bump_becomes_a_normal_map() {
    let mut document = demo();
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let key = document.tree.get(part).unwrap().material.unwrap();
    let pattern = tequila_texture::procedural::Procedural::catalog().into_iter().find(|p| p.key() == "cellular").unwrap();
    let mut bus = CommandBus::new();
    bus.apply(&mut document, Command::SetParameterProcedural { material: key, param: Param::Bump, procedural: Some(Box::new(pattern)), seed: 2 }).unwrap();
    // Deep, so the slopes of the few cells across the ten-centimetre ball
    // show plainly in eight bits.
    let mut graph = document.materials.get(key).unwrap().graph.clone();
    assert!(tequila_scene::command::set_bump_depth(&mut graph, Param::Bump, 0.05));
    bus.apply(&mut document, Command::SetGraph { material: key, graph: Box::new(graph) }).unwrap();
    let exported = export(&document, &UsdzOptions { bake_size: 128, ..Default::default() });
    let layer = stage(&exported.bytes);
    let material = prims(&layer, "Material")[0].to_string();
    let image = connection(&layer, &format!("{material}/PreviewSurface.inputs:normal")).expect("a normal map");
    let shader = image.split_once(".outputs:").unwrap().0;
    assert_eq!(default(&layer, &format!("{shader}.inputs:sourceColorSpace")), Some(&Value::Token("raw".into())));
    assert_eq!(default(&layer, &format!("{shader}.inputs:scale")), Some(&Value::Vecf(vec![2.0, 2.0, 2.0, 1.0])));
    assert_eq!(default(&layer, &format!("{shader}.inputs:bias")), Some(&Value::Vecf(vec![-1.0, -1.0, -1.0, 0.0])));
    // The colour did not vary, so it is not a picture.
    assert!(matches!(default(&layer, &format!("{material}/PreviewSurface.inputs:diffuseColor")), Some(Value::Vecf(_))));

    // The map tilts the normal both ways, and on the whole points out.
    let Some(Value::Asset(file)) = default(&layer, &format!("{shader}.inputs:file")) else { panic!("a file") };
    let zip = tequila_format::zip::ZipArchive::read(&exported.bytes).unwrap();
    let mut reader = png::Decoder::new(zip.get(file).unwrap()).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    let texels: Vec<&[u8]> = pixels.chunks_exact(3).collect();
    let (lo, hi) = texels.iter().fold((255u8, 0u8), |(l, h), p| (l.min(p[0]), h.max(p[0])));
    assert!(lo < 110 && hi > 146, "tilted both ways along u: {lo}..{hi}");
    let blue = texels.iter().map(|p| p[2] as f32).sum::<f32>() / texels.len() as f32;
    assert!(blue > 200.0, "out of the surface: {blue}");
}

/// Two parts sharing a mesh share it in the package too: one prototype,
/// referenced by an instanceable prim for each, each binding its own
/// material.
#[test]
fn shared_meshes_are_instanced() {
    let mut document = SceneDocument::new();
    let (meshes, materials, ids) = reserve_keys(&document, 1, 2, 3);
    let mut group = SceneNode::new(ids[0], "Bolts", NodeKind::Group);
    group.children = vec![ids[1], ids[2]];
    let mut nodes = vec![group];
    for (k, x) in [(1usize, -0.05f32), (2, 0.05)] {
        let mut bolt = SceneNode::new(ids[k], format!("Bolt {k}"), NodeKind::Part { mesh: meshes[0] });
        bolt.parent = Some(ids[0]);
        bolt.transform = Transform::translation(vec3(x, 0.01, 0.0));
        bolt.material = Some(materials[k - 1]);
        nodes.push(bolt);
    }
    let mut cube = tequila_scene::geometry::Mesh::new(
        vec![vec3(0.0, 0.0, 0.0), vec3(0.01, 0.0, 0.0), vec3(0.0, 0.01, 0.0), vec3(0.0, 0.0, 0.01)],
        vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
    );
    cube.normals.clear();
    CommandBus::new()
        .apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], cube)],
                    textures: Vec::new(),
                    materials: vec![
                        (materials[0], MaterialDefinition::preset("Red", MaterialType::Plastic)),
                        (materials[1], MaterialDefinition::preset("Steel", MaterialType::Metal)),
                    ],
                    nodes,
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
    let exported = export(&document, &UsdzOptions::default());
    let layer = stage(&exported.bytes);
    let meshes = prims(&layer, "Mesh");
    assert_eq!(meshes.len(), 1, "the mesh once: {meshes:?}");
    assert!(meshes[0].starts_with("/Prototypes/"));
    assert_eq!(field(&layer, "/Prototypes", "specifier"), Some(&Value::Specifier(usdc::Specifier::Class)));
    let instances: Vec<&str> = layer.specs.iter().filter(|s| s.field("instanceable") == Some(&Value::Bool(true))).map(|s| s.path.as_str()).collect();
    assert_eq!(instances.len(), 2);
    let materials: std::collections::BTreeSet<String> = instances.iter().map(|i| bound(&layer, i).expect("each binds a material")).collect();
    assert_eq!(materials.len(), 2, "each its own");
    for instance in instances {
        let Some(Value::References(op)) = field(&layer, instance, "references") else { panic!() };
        assert_eq!(op.items[0].prim, meshes[0].rsplit_once('/').unwrap().0);
    }

    // Read back, a bolt at each place.
    let scene = read_format(&exported.bytes, "usdz", "bolts", &|_| None).expect("reads");
    assert_eq!(scene.meshes.len(), 2);
    assert!(scene.meshes.iter().all(|m| m.material.is_some()));
}

/// What goes out comes back through Tequila's own importer: the parts where
/// they stood (moved onto the ground as the package moves them), and the
/// materials - a plain one whole, a baked one as its picture.
#[test]
fn tequila_reads_its_own_package_back() {
    let mut document = demo();
    wood(&mut document);
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    let exported = export(&document, &UsdzOptions { bake_size: 128, ..Default::default() });
    let scene = read_format(&exported.bytes, "usdz", "demo", &|_| None).expect("our own usdz reads");
    assert_eq!(scene.meshes.len(), 1);
    let mesh = &scene.meshes[0];
    let NodeKind::Part { mesh: key } = document.tree.get(part).unwrap().kind else { unreachable!() };
    assert_eq!(mesh.indices.len(), document.geometry.get(key).unwrap().indices.len());
    // Viewers see the wood baked; Tequila reads the wood itself, from the
    // definition the material carries.
    let material = &scene.materials[mesh.material.expect("bound")];
    let wood = material.graph.nodes().any(|(_, n)| matches!(n.kind, GraphNode::Procedural { .. }));
    assert!(wood, "the wood comes back as the wood");
    assert!(scene.textures.is_empty());

    let mut copy = SceneDocument::new();
    let options = ImportOptions { place_on_ground: false, ..Default::default() };
    let (command, _) = build(&copy, None, scene, &options, None).unwrap();
    CommandBus::new().apply(&mut copy, command).unwrap();
    let back = copy.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).unwrap();
    let NodeKind::Part { mesh: copied } = back.kind else { unreachable!() };
    let lowest = copy.geometry.get(copied).unwrap().positions.iter().map(|p| copy.tree.world_transform(back.id).point(*p).y).fold(f32::INFINITY, f32::min);
    assert!(lowest.abs() < 1e-4, "standing on the ground: {lowest}");
    let size = |doc: &SceneDocument, id, mesh| {
        let world = doc.tree.world_transform(id);
        let ys: Vec<f32> = doc.geometry.get(mesh).unwrap().positions.iter().map(|p| world.point(*p).y).collect();
        ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max) - ys.iter().cloned().fold(f32::INFINITY, f32::min)
    };
    assert!((size(&copy, back.id, copied) - size(&document, part, key)).abs() < 1e-5, "at its own size");
}

/// A plain material comes back whole, by the definition it carries.
#[test]
fn a_plain_material_comes_back_whole() {
    let document = sample("Perfume bottle.tequila");
    let exported = export(&document, &UsdzOptions::default());
    let scene = read_format(&exported.bytes, "usdz", "bottle", &|_| None).expect("reads");
    for original in document.materials.iter().map(|(_, d)| d) {
        let Some(back) = scene.materials.iter().find(|m| m.name == original.name) else { continue };
        assert_eq!(back.material_type, original.material_type, "{}", original.name);
        for param in Param::ALL {
            assert_eq!(param.read(back.material()), param.read(original.material()), "{} {}", original.name, param.key());
        }
    }
}

/// The samples export with their materials baked where they vary - the
/// headphones' bumped plastics and brushed steel as normal maps - and
/// within a size a phone downloads. With `TEQUILA_USDZ_SAMPLES` naming a
/// folder, each package is written there, and the `.usda` beside it, to
/// open in other tools.
#[test]
fn the_samples_export() {
    for name in ["Headphones.tequila", "palma P8.tequila", "Perfume bottle.tequila"] {
        let document = sample(name);
        let stem = name.trim_end_matches(".tequila").replace(' ', "_");
        let exported = export(&document, &UsdzOptions { name: stem.clone(), ..Default::default() });
        if let Some(folder) = std::env::var_os("TEQUILA_USDZ_SAMPLES") {
            let folder = Path::new(&folder);
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join(format!("{stem}.usdz")), &exported.bytes).unwrap();
            std::fs::write(folder.join(format!("{stem}.usda")), tequila_format::usd::export(&document)).unwrap();
        }
        let layer = stage(&exported.bytes);
        assert!(!prims(&layer, "Mesh").is_empty(), "{name}");
        assert!(exported.bytes.len() < 60_000_000, "{name}: {} bytes", exported.bytes.len());
        for (member, method, data, _) in members(&exported.bytes) {
            assert_eq!(method, 0, "{name} {member}");
            assert_eq!(data % 64, 0, "{name} {member}");
        }
        let textures = exported.members.len() - 1;
        assert!(textures > 0 || name.starts_with("Perfume"), "{name} bakes something");
        if name.starts_with("Headphones") {
            assert!(exported.members.iter().any(|m| m.ends_with("_normal.png")), "{name}: {:?}", exported.members);
        }
        // Read back, a part for every part shown, instances included.
        let back = read_format(&exported.bytes, "usdz", name, &|_| None).expect("reads back");
        let shown = document.tree.iter().filter(|n| matches!(n.kind, NodeKind::Part { .. }) && document.is_shown(n.id)).count();
        assert_eq!(back.meshes.len(), shown, "{name}");
        if name.starts_with("palma") {
            // Its printed circuit boards are pictures: they come back as
            // image nodes on the pictures from inside the package.
            assert!(!back.textures.is_empty() && back.textures.iter().all(|t| t.embedded.as_ref().is_some_and(|b| b.starts_with(b"\x89PNG"))));
            let imaged = back.materials.iter().filter(|m| m.graph.nodes().any(|(_, n)| matches!(n.kind, GraphNode::ImageTexture { .. }))).count();
            assert!(imaged >= 3, "{imaged} materials with pictures");
        }
    }
}

/// A layer Pixar's own library wrote (`data/make_pixar_written.py`) reads
/// field for field: compressed integers and floats in both codings, an
/// inlined vector, a matrix, list ops and a class prototype.
#[test]
fn a_layer_pixar_wrote_reads() {
    let bytes = include_bytes!("data/pixar_written.usdc");
    let layer = usdc::read(bytes).expect("reads");
    assert_eq!(field(&layer, "/", "defaultPrim"), Some(&Value::Token("Product".into())));
    assert_eq!(field(&layer, "/", "metersPerUnit"), Some(&Value::Double(1.0)));
    let part = "/Product/Group/Part";
    assert_eq!(default(&layer, &format!("{part}.points")), Some(&Value::Vecfs(3, vec![-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.5, -1.0, 1.0, 0.123])));
    let indices: Vec<i32> = [0, 1, 2, 0, 2, 3].repeat(10);
    assert_eq!(default(&layer, &format!("{part}.faceVertexIndices")), Some(&Value::Ints(indices)), "compressed integers");
    assert_eq!(default(&layer, &format!("{part}.faceVertexCounts")), Some(&Value::Ints(vec![3; 20])));
    let counts: Vec<f32> = (1..=20).map(|i| i as f32).collect();
    assert_eq!(default(&layer, &format!("{part}.counts")), Some(&Value::Floats(counts)), "floats that are whole numbers");
    assert_eq!(default(&layer, &format!("{part}.widths")), Some(&Value::Floats([0.5, 1.5, 2.0, 3.0].repeat(5))), "floats from a table");
    assert_eq!(default(&layer, &format!("{part}.primvars:st")), Some(&Value::Vecfs(2, vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])));
    assert_eq!(field(&layer, &format!("{part}.normals"), "interpolation"), Some(&Value::Token("vertex".into())));
    assert_eq!(default(&layer, &format!("{part}.doubleSided")), Some(&Value::Bool(true)));
    let Some(Value::Vecd(m)) = default(&layer, "/Product/Group.xformOp:transform") else { panic!("a matrix") };
    assert_eq!(&m[12..], &[0.25, 0.5, -1.5, 1.0]);
    assert_eq!(default(&layer, "/Product/Bolt_1.xformOp:translate"), Some(&Value::Vecd(vec![1.0, 2.0, 3.0])), "an inlined double3");
    assert_eq!(default(&layer, "/Product/Materials/Red/Normal.inputs:bias"), Some(&Value::Vecf(vec![-1.0, -1.0, -1.0, 0.0])), "an inlined float4");
    assert_eq!(default(&layer, "/Product/Materials/Red/Surface.inputs:metallic"), Some(&Value::Float(0.123456)));
    assert_eq!(default(&layer, "/Product/Materials/Red/Colour.inputs:file"), Some(&Value::Asset("textures/red.png".into())));
    assert_eq!(connection(&layer, "/Product/Materials/Red/Surface.inputs:diffuseColor"), Some("/Product/Materials/Red/Colour.outputs:rgb".into()));
    assert_eq!(bound(&layer, part), Some("/Product/Materials/Red".into()));
    let Some(Value::References(op)) = field(&layer, "/Product/Bolt_1", "references") else { panic!("a reference") };
    assert_eq!(op.items, vec![usdc::Reference { asset: String::new(), prim: "/Prototypes/Bolt".into() }]);
    assert_eq!(field(&layer, "/Prototypes", "specifier"), Some(&Value::Specifier(usdc::Specifier::Class)));
    assert!(matches!(field(&layer, part, "apiSchemas"), Some(Value::TokenList(op)) if op.items == ["MaterialBindingAPI"]));

    // And Tequila's importer makes a scene of it: the part, and a bolt
    // where the reference puts one; the class itself not drawn.
    let scene = read_format(bytes, "usdc", "pixar", &|_| None).expect("imports");
    assert_eq!(scene.meshes.len(), 2);
}
