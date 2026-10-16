//! glTF export (`tequila_format::gltf`), read back by Tequila's own glTF
//! reader: what goes out comes back.

use tequila_format::gltf::{export, GltfOptions};
use tequila_format::json::{self, Value};
use tequila_import::read_format;
use tequila_materials::material_type::Param;
use tequila_scene::studio::MultiMaterial;
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus, NodeId, SceneDocument};

fn demo() -> (SceneDocument, CommandBus, NodeId) {
    let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let part = document.tree.iter().find(|n| matches!(n.kind, NodeKind::Part { .. })).map(|n| n.id).unwrap();
    (document, CommandBus::new(), part)
}

/// The JSON chunk of a `.glb`.
fn gltf_json(glb: &[u8]) -> Value {
    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize, glb.len(), "the header's length is the file's");
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    assert_eq!(&glb[16..20], b"JSON");
    json::parse(&glb[20..20 + length]).unwrap()
}

/// The part's geometry and its plain material come back as they went.
#[test]
fn a_plain_part_round_trips() {
    let (document, _, part) = demo();
    let exported = export(&document, &GltfOptions::default());
    assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
    assert!(exported.nodes.contains_key(&part));
    let scene = read_format(&exported.glb, "glb", "demo", &|_| None).expect("our own glb reads");
    let key = document.tree.get(part).unwrap().material.unwrap();
    let original = document.materials.get(key).unwrap().material().clone();
    let NodeKind::Part { mesh } = document.tree.get(part).unwrap().kind else { unreachable!() };
    let source = document.geometry.get(mesh).unwrap();
    let back = scene.meshes.iter().find(|m| m.positions.len() == source.positions.len()).expect("the part's mesh");
    assert_eq!(back.indices.len(), source.indices.len());
    let material = &scene.materials[back.material.expect("a material")];
    let colour = material.material().base_color;
    let want = original.base_color * original.base_weight;
    assert!((colour.r - want.r).abs() < 0.01 && (colour.g - want.g).abs() < 0.01 && (colour.b - want.b).abs() < 0.01, "{colour:?} for {want:?}");
    assert!((material.material().specular_roughness - original.specular_roughness).abs() < 0.01);
}

/// A procedural is baked: into the mesh's own UVs when it has a clean
/// unwrap, into an atlas of its own when it has none - and either way the
/// baked colours vary as the pattern does.
#[test]
fn a_procedural_is_baked_into_the_uvs_or_an_atlas() {
    for with_uvs in [true, false] {
        let (mut document, mut bus, part) = demo();
        if !with_uvs {
            let NodeKind::Part { mesh } = document.tree.get(part).unwrap().kind else { unreachable!() };
            let mut bare = document.geometry.get(mesh).unwrap().clone();
            bare.uv_sets.clear();
            bare.tangents.clear();
            document.geometry.insert_with_key(mesh, bare);
        }
        let key = document.tree.get(part).unwrap().material.unwrap();
        let wood = tequila_texture::procedural::Procedural::catalog().into_iter().find(|p| p.key() == "wood").unwrap();
        bus.apply(&mut document, Command::SetParameterProcedural { material: key, param: Param::BaseColor, procedural: Some(Box::new(wood)), seed: 1 }).unwrap();
        let exported = export(&document, &GltfOptions { bake_size: 256, ..Default::default() });
        let root = gltf_json(&exported.glb);
        let material = &root.get("materials").unwrap().as_array()[exported.materials[&(part, key)]];
        let texture = material.get("pbrMetallicRoughness").and_then(|p| p.get("baseColorTexture")).expect("a baked base colour");
        let mesh = &root.get("meshes").unwrap().as_array()[exported.nodes[&part]];
        let attributes = mesh.get("primitives").unwrap().as_array()[0].get("attributes").unwrap();
        if with_uvs {
            assert_eq!(texture.get("texCoord").and_then(|v| v.as_index()), None, "on the mesh's own UVs");
            assert!(attributes.get("TEXCOORD_1").is_none());
        } else {
            assert_eq!(texture.get("texCoord").and_then(|v| v.as_index()), Some(1), "on an atlas's coordinates");
            assert!(attributes.get("TEXCOORD_1").is_some());
        }

        // The image, out of the binary chunk, decoded.
        let image = &root.get("images").unwrap().as_array()[texture.get("index").unwrap().as_index().unwrap()];
        let view = &root.get("bufferViews").unwrap().as_array()[image.get("bufferView").unwrap().as_index().unwrap()];
        let json_length = u32::from_le_bytes(exported.glb[12..16].try_into().unwrap()) as usize;
        let bin = 20 + json_length + 8;
        let (offset, length) = (view.get("byteOffset").unwrap().as_index().unwrap(), view.get("byteLength").unwrap().as_index().unwrap());
        let path = std::env::temp_dir().join(format!("tequila-baked-{}-{with_uvs}.png", std::process::id()));
        std::fs::write(&path, &exported.glb[bin + offset..bin + offset + length]).unwrap();
        let decoded = tequila_texture::image::Image::load_raw(&path);
        let _ = std::fs::remove_file(&path);
        let decoded = decoded.expect("a PNG");
        assert_eq!((decoded.width, decoded.height), (256, 256));
        // Texels the bake covered or dilated into: alpha 1. Among them the
        // pattern varies.
        let covered: Vec<f32> = decoded.data.chunks_exact(4).filter(|p| p[3] > 0.5).map(|p| p[0]).collect();
        assert!(covered.len() > 256 * 256 / 3, "only {} texels baked", covered.len());
        let mean = covered.iter().sum::<f32>() / covered.len() as f32;
        let spread = (covered.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / covered.len() as f32).sqrt();
        assert!(spread > 0.02, "the wood baked flat: spread {spread}");
    }
}

/// A Multi-Material's variants are written as `KHR_materials_variants`:
/// one named variant each, mapped onto the part.
#[test]
fn variants_are_written_for_other_viewers() {
    let (mut document, mut bus, part) = demo();
    let first = document.tree.get(part).unwrap().material.unwrap();
    bus.apply(&mut document, Command::UnlinkMaterial { node: part }).unwrap();
    let second = document.tree.get(part).unwrap().material.unwrap();
    bus.apply(&mut document, Command::SetMultiMaterials {
        multi_materials: vec![MultiMaterial { id: 5, name: "Body".into(), variants: vec![first, second], active: 1, parts: vec![part] }],
    })
    .unwrap();
    let exported = export(&document, &GltfOptions::default());
    let root = gltf_json(&exported.glb);
    let names: Vec<String> = root
        .get("extensions").and_then(|e| e.get("KHR_materials_variants")).and_then(|v| v.get("variants")).unwrap()
        .as_array().iter().map(|v| v.get("name").unwrap().as_str().unwrap().to_string()).collect();
    assert_eq!(names.len(), 2);
    assert!(names[0].starts_with("Body: "), "{names:?}");
    let primitive = &root.get("meshes").unwrap().as_array()[exported.nodes[&part]].get("primitives").unwrap().as_array()[0];
    let mappings = primitive.get("extensions").and_then(|e| e.get("KHR_materials_variants")).and_then(|v| v.get("mappings")).unwrap().as_array();
    assert_eq!(mappings.len(), 2);
    // The part wears the active variant until a viewer switches it.
    assert_eq!(primitive.get("material").and_then(|m| m.as_index()), Some(exported.materials[&(part, second)]));
    assert!(root.get("extensionsUsed").unwrap().as_array().iter().any(|v| v.as_str() == Some("KHR_materials_variants")));
}
