//! glTF import, against assemblies built here rather than against fixture
//! files.
//!
//! Built in code because the interesting cases are the encodings a real
//! exporter chooses and a hand-written fixture would not: strided buffer
//! views, quantized and `normalized` attributes, quaternion rotations,
//! multi-primitive meshes, and the GLB container. A checked-in `.glb` would
//! test one exporter's choices; this tests the reader's.

use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_import::{build, read_format, ImportOptions};
use tequila_materials::material_type::Param;
use tequila_scene::tree::NodeKind;
use tequila_scene::{CommandBus, SceneDocument};

/// A unit triangle's positions, as raw little-endian f32.
fn triangle_positions() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

fn indices() -> Vec<u8> {
    [0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A two-part assembly: one mesh, two nodes, under a named group. This is the
/// shape M1's exit criterion is written against.
fn assembly_gltf() -> String {
    let mut buffer = triangle_positions();
    let index_offset = buffer.len();
    buffer.extend_from_slice(&indices());
    let total = buffer.len();

    format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [0]}}],
  "nodes": [
    {{"name": "Assembly", "children": [1, 2]}},
    {{"name": "Left", "mesh": 0, "translation": [-5.0, 0.0, 0.0]}},
    {{"name": "Right", "mesh": 0, "translation": [5.0, 0.0, 0.0],
      "rotation": [0.0, 0.0, 0.7071068, 0.7071068],
      "extras": {{"PartNumber": "M3-8", "Quantity": 2}}}}
  ],
  "meshes": [
    {{"name": "Bracket", "primitives": [
      {{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}
    ]}}
  ],
  "materials": [
    {{"name": "Anodized", "pbrMetallicRoughness": {{
      "baseColorFactor": [0.2, 0.4, 0.8, 1.0],
      "metallicFactor": 1.0, "roughnessFactor": 0.3}}}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": {index_offset}}},
    {{"buffer": 0, "byteOffset": {index_offset}, "byteLength": {}}}
  ],
  "buffers": [{{"byteLength": {total}, "uri": "data:application/octet-stream;base64,{}"}}]
}}"#,
        total - index_offset,
        base64(&buffer)
    )
}

fn import(text: &str, extension: &str) -> SceneDocument {
    import_with(text.as_bytes(), extension, &ImportOptions::default())
}

fn import_with(bytes: &[u8], extension: &str, options: &ImportOptions) -> SceneDocument {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let scene = read_format(bytes, extension, "file", &|_| None).expect("the file should read");
    let (command, _) = build(&document, None, scene, options, None).expect("the scene should build");
    bus.apply(&mut document, command).unwrap();
    document.validate().expect("the imported document is valid");
    document
}

#[test]
fn a_multi_part_assembly_keeps_its_hierarchy_and_transforms() {
    let document = import(&assembly_gltf(), "gltf");

    let by_name = |name: &str| {
        document
            .tree
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no node named `{name}`"))
    };

    let assembly = by_name("Assembly");
    let left = by_name("Left");
    let right = by_name("Right");
    assert_eq!(left.parent, Some(assembly.id));
    assert_eq!(right.parent, Some(assembly.id));
    assert_eq!(
        assembly.children,
        vec![left.id, right.id],
        "children must be listed on the parent, and in file order"
    );

    // Transforms, in world space.
    let at = |id| document.tree.world_transform(id).point(Vec3::ZERO);
    assert!((at(left.id) - vec3(-5.0, 0.0, 0.0)).length() < 1e-5);
    assert!((at(right.id) - vec3(5.0, 0.0, 0.0)).length() < 1e-5);

    // The quaternion is a 90° turn about +Z, so the local +X axis ends up
    // along +Y. A dropped or transposed rotation is invisible at the origin
    // and obvious here.
    let corner = document
        .tree
        .world_transform(right.id)
        .point(vec3(1.0, 0.0, 0.0));
    assert!(
        (corner - vec3(5.0, 1.0, 0.0)).length() < 1e-4,
        "the rotation was not applied: {corner:?}"
    );
}

/// Both nodes name mesh 0, so the document must hold one mesh. glTF says this
/// outright; failing to honour it is the 4,000-screw problem of §2.2.
#[test]
fn a_mesh_referenced_twice_is_stored_once() {
    let document = import(&assembly_gltf(), "gltf");

    assert_eq!(document.geometry.len(), 1);
    let parts: Vec<_> = document
        .tree
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Part { .. }))
        .collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].kind, parts[1].kind, "both must share the mesh");
}

#[test]
fn materials_and_extras_survive_the_import() {
    let document = import(&assembly_gltf(), "gltf");

    assert_eq!(document.materials.len(), 1);
    let (_, material) = document.materials.iter().next().unwrap();
    assert_eq!(material.name, "Anodized");
    // metallicFactor 1.0 is a conductor.
    assert_eq!(
        material.material_type,
        tequila_materials::material_type::MaterialType::Metal
    );
    assert!((material.material().specular_roughness - 0.3).abs() < 1e-6);

    let right = document.tree.iter().find(|n| n.name == "Right").unwrap();
    assert_eq!(
        right.metadata.get("PartNumber").map(|s| s.as_str()),
        Some("M3-8")
    );
    assert_eq!(
        right.metadata.get("Quantity").map(|s| s.as_str()),
        Some("2")
    );
}

/// glTF's colour factors are linear sRGB, as its textures and vertex colours
/// are, and a part renders as the colour its file states. They were taken as
/// rendering-space (ACEScg) numbers, so every imported colour came out more
/// saturated than it was modelled, and a textured material's factor and its
/// texture were in two different spaces.
#[test]
fn colour_factors_render_as_the_colour_the_file_states() {
    let document = import(&assembly_gltf(), "gltf");
    let (_, material) = document.materials.iter().next().unwrap();
    let base = material.material().base_color.to_linear_srgb();
    for (got, stated) in [(base.x, 0.2), (base.y, 0.4), (base.z, 0.8)] {
        assert!((got - stated).abs() < 1e-3, "baseColorFactor [0.2, 0.4, 0.8] renders as {base:?}");
    }
    let sheen = material_with_extensions(
        "ext-sheen-colour",
        r#"{ "KHR_materials_sheen": { "sheenColorFactor": [0.9, 0.1, 0.1] } }"#,
    );
    let fuzz = (sheen.fuzz_color * sheen.fuzz_weight).to_linear_srgb();
    for (got, stated) in [(fuzz.x, 0.9), (fuzz.y, 0.1), (fuzz.z, 0.1)] {
        assert!((got - stated).abs() < 2e-3, "sheenColorFactor [0.9, 0.1, 0.1] renders as {fuzz:?}");
    }
}

/// glTF states metres, so nothing is guessed (§3.4).
#[test]
fn the_unit_comes_from_the_format_not_a_guess() {
    let scene = read_format(assembly_gltf().as_bytes(), "gltf", "file", &|_| None).unwrap();
    let document = SceneDocument::new();
    let (_, report) = build(&document, None, scene, &ImportOptions::default(), None).unwrap();

    assert_eq!(report.source_unit, LengthUnit::Metre);
    assert!(!report.unit_was_guessed);
}

/// The same document as a GLB: the reader must find the JSON and BIN chunks
/// and read the buffer from the binary chunk rather than from a URI.
#[test]
fn glb_reads_the_same_scene_as_gltf() {
    let mut buffer = triangle_positions();
    let index_offset = buffer.len();
    buffer.extend_from_slice(&indices());
    let total = buffer.len();

    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"Part","mesh":0}}],
"meshes":[{{"name":"Bracket","primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
             {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{index_offset}}},
               {{"buffer":0,"byteOffset":{index_offset},"byteLength":{}}}],
"buffers":[{{"byteLength":{total}}}]}}"#,
        total - index_offset
    );

    let mut json_chunk = json.into_bytes();
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' '); // JSON chunks pad with spaces
    }
    let mut bin_chunk = buffer;
    while bin_chunk.len() % 4 != 0 {
        bin_chunk.push(0);
    }

    let length = 12 + 8 + json_chunk.len() + 8 + bin_chunk.len();
    let mut glb = Vec::with_capacity(length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(length as u32).to_le_bytes());
    glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&(bin_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin_chunk);

    let document = import_with(&glb, "glb", &ImportOptions::default());
    assert_eq!(document.tree.len(), 1);
    assert_eq!(document.geometry.len(), 1);
    let mesh = document.geometry.iter().next().unwrap().1;
    assert_eq!(mesh.triangle_count(), 1);
    assert_eq!(mesh.positions[1], vec3(1.0, 0.0, 0.0));
}

/// Exporters interleave attributes into one buffer view with a `byteStride`.
/// Reading it as tightly packed silently returns the wrong vertices.
#[test]
fn strided_and_normalized_attributes_are_decoded() {
    // Per vertex: 3 f32 positions (12 bytes) + 2 u16 UVs (4 bytes) = 16.
    let mut buffer = Vec::new();
    for (p, uv) in [
        ([0.0f32, 0.0, 0.0], [0u16, 0]),
        ([1.0, 0.0, 0.0], [65535, 0]),
        ([0.0, 1.0, 0.0], [0, 65535]),
    ] {
        for c in p {
            buffer.extend_from_slice(&c.to_le_bytes());
        }
        for c in uv {
            buffer.extend_from_slice(&c.to_le_bytes());
        }
    }
    let index_offset = buffer.len();
    buffer.extend_from_slice(&indices());
    let total = buffer.len();

    let text = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"Part","mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2}}]}}],
"accessors":[
  {{"bufferView":0,"byteOffset":0,"componentType":5126,"count":3,"type":"VEC3"}},
  {{"bufferView":0,"byteOffset":12,"componentType":5123,"normalized":true,"count":3,"type":"VEC2"}},
  {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}],
"bufferViews":[
  {{"buffer":0,"byteOffset":0,"byteLength":{index_offset},"byteStride":16}},
  {{"buffer":0,"byteOffset":{index_offset},"byteLength":{}}}],
"buffers":[{{"byteLength":{total},"uri":"data:application/octet-stream;base64,{}"}}]}}"#,
        total - index_offset,
        base64(&buffer)
    );

    let document = import(&text, "gltf");
    let mesh = document.geometry.iter().next().unwrap().1;

    assert_eq!(mesh.positions.len(), 3);
    assert_eq!(
        mesh.positions[2],
        vec3(0.0, 1.0, 0.0),
        "the stride was ignored"
    );

    // 65535 with `normalized` means 1.0, not 65535.
    let uvs = &mesh.uv_sets[0];
    assert!(
        (uvs[1].x - 1.0).abs() < 1e-4,
        "uv {:?} was not denormalized",
        uvs[1]
    );
    assert!((uvs[2].y - 1.0).abs() < 1e-4);
}

/// COLOR_0 comes in: normalised bytes read as 0 to 1, linear, with alpha.
#[test]
fn vertex_colours_are_read() {
    let mut buffer = triangle_positions();
    let colours_at = buffer.len();
    for rgba in [[255u8, 0, 0, 255], [0, 255, 0, 128], [0, 0, 255, 255]] {
        buffer.extend_from_slice(&rgba);
    }
    let index_offset = buffer.len();
    buffer.extend_from_slice(&indices());
    let total = buffer.len();
    let text = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"Part","mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"COLOR_0":1}},"indices":2}}]}}],
"accessors":[
  {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
  {{"bufferView":1,"componentType":5121,"normalized":true,"count":3,"type":"VEC4"}},
  {{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}}],
"bufferViews":[
  {{"buffer":0,"byteOffset":0,"byteLength":{colours_at}}},
  {{"buffer":0,"byteOffset":{colours_at},"byteLength":12}},
  {{"buffer":0,"byteOffset":{index_offset},"byteLength":{}}}],
"buffers":[{{"byteLength":{total},"uri":"data:application/octet-stream;base64,{}"}}]}}"#,
        total - index_offset,
        base64(&buffer)
    );
    let document = import(&text, "gltf");
    let mesh = document.geometry.iter().next().unwrap().1;
    assert_eq!(mesh.colors.len(), 3, "the colours were dropped");
    let find = |x: f32, y: f32| mesh.colors.iter().copied().find(|c| (c.x - x).abs() < 1e-3 && (c.y - y).abs() < 1e-3);
    assert!(find(1.0, 0.0).is_some_and(|c| c.w == 1.0));
    assert!(find(0.0, 1.0).is_some_and(|c| (c.w - 128.0 / 255.0).abs() < 1e-3));
}

/// A mesh with several primitives cannot be one node, because a node carries
/// one material. It becomes a group with a child per primitive.
#[test]
fn a_multi_primitive_mesh_becomes_a_group_of_parts() {
    let mut buffer = triangle_positions();
    let index_offset = buffer.len();
    buffer.extend_from_slice(&indices());
    let total = buffer.len();

    let text = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"Housing","mesh":0}}],
"meshes":[{{"name":"Housing","primitives":[
  {{"attributes":{{"POSITION":0}},"indices":1,"material":0}},
  {{"attributes":{{"POSITION":0}},"indices":1,"material":1}}]}}],
"materials":[{{"name":"Shell"}},{{"name":"Window"}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
             {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{index_offset}}},
               {{"buffer":0,"byteOffset":{index_offset},"byteLength":{}}}],
"buffers":[{{"byteLength":{total},"uri":"data:application/octet-stream;base64,{}"}}]}}"#,
        total - index_offset,
        base64(&buffer)
    );

    let document = import(&text, "gltf");

    let housing = document.tree.iter().find(|n| n.name == "Housing").unwrap();
    assert_eq!(housing.kind, NodeKind::Group);
    assert_eq!(housing.children.len(), 2);

    // Each child carries its own material, which is the reason for the split.
    let materials: Vec<_> = housing
        .children
        .iter()
        .map(|&id| document.tree.get(id).unwrap().material)
        .collect();
    assert_eq!(materials.len(), 2);
    assert_ne!(materials[0], materials[1]);
    assert!(materials.iter().all(|m| m.is_some()));
}

/// Every one of these is a file someone could send. None may panic, and the
/// message must say what was wrong.
#[test]
fn broken_files_are_refused_with_an_explanation() {
    let cases: [(&str, &str); 7] = [
        (r#"{"asset":{"version":"1.0"}}"#, "1.0"),
        (
            r#"{"asset":{"version":"2.0"},"nodes":[{"name":"a"}],"scenes":[{"nodes":[9]}],"scene":0}"#,
            "no nodes",
        ),
        (
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
               "nodes":[{"mesh":0}],
               "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
               "accessors":[{"bufferView":0,"componentType":5126,"count":1000,"type":"VEC3"}],
               "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":12}],
               "buffers":[{"byteLength":12,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAA"}]}"#,
            "past the end",
        ),
        (
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
               "nodes":[{"mesh":0}],
               "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
               "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],
               "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":12}],
               "buffers":[{"byteLength":12,"uri":"external.bin"}]}"#,
            "external.bin",
        ),
        // A count whose size overflows: it read as needing no bytes, and the
        // reader then reserved room for 2^57 positions and aborted.
        (
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
               "nodes":[{"mesh":0}],
               "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
               "accessors":[{"bufferView":0,"componentType":5126,"count":144115188075855872,"type":"VEC3"}],
               "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":12,"byteStride":252}],
               "buffers":[{"byteLength":12,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAA"}]}"#,
            "past the end",
        ),
        (r#"{"asset":{"version":"2.0"}"#, "malformed"),
        (r#"{"asset":{"version":"2.0"},"nodes":[]}"#, "no nodes"),
    ];

    for (text, expected) in cases {
        let error = read_format(text.as_bytes(), "gltf", "file", &|_| None)
            .expect_err("this file should have been refused");
        let message = error.to_string();
        assert!(
            message.contains(expected),
            "message {message:?} does not mention {expected:?}"
        );
    }
}

/// A `children` cycle is not legal glTF, but nothing stops a writer emitting
/// one, and following it would not terminate.
#[test]
fn a_cyclic_hierarchy_terminates() {
    let text = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
      "nodes":[{"name":"a","children":[1]},{"name":"b","children":[0]}]}"#;

    let scene = read_format(text.as_bytes(), "gltf", "file", &|_| None).unwrap();
    assert_eq!(scene.nodes.len(), 2);
    assert!(scene.warnings.iter().any(|w| w.contains("more than once")));
    scene.validate().unwrap();
}

/// A truncated GLB must be an error, not a slice panic.
#[test]
fn a_truncated_glb_is_an_error() {
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&9999u32.to_le_bytes()); // claims more than it has
    assert!(read_format(&glb, "glb", "file", &|_| None).is_err());

    // A chunk whose declared length runs past the file.
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&24u32.to_le_bytes());
    glb.extend_from_slice(&9999u32.to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(b"{}");
    assert!(read_format(&glb, "glb", "file", &|_| None).is_err());
}

/// A triangle with UVs, a base-colour map and a normal map.
///
/// Written out here rather than checked in, for the reason the header gives:
/// what is interesting is the encodings an exporter chooses, and the image
/// references are the part this is about.
fn textured_gltf(base_color_uri: &str, normal_uri: &str) -> String {
    let mut buffer = triangle_positions();
    let uv_offset = buffer.len();
    for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
        for c in uv {
            buffer.extend_from_slice(&c.to_le_bytes());
        }
    }
    let index_offset = buffer.len();
    buffer.extend_from_slice(&indices());
    let total = buffer.len();

    format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "mesh": 0, "name": "Part" }}],
  "meshes": [{{ "primitives": [{{
      "attributes": {{ "POSITION": 0, "TEXCOORD_0": 1 }},
      "indices": 2,
      "material": 0
  }}] }}],
  "materials": [{{
      "name": "Painted",
      "pbrMetallicRoughness": {{
          "baseColorFactor": [0.8, 0.2, 0.2, 1.0],
          "baseColorTexture": {{ "index": 0 }}
      }},
      "normalTexture": {{ "index": 1 }}
  }}],
  "textures": [{{ "source": 0 }}, {{ "source": 1 }}],
  "images": [{{ "uri": "{base_color_uri}" }}, {{ "uri": "{normal_uri}" }}],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
       "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {uv_offset} }},
    {{ "buffer": 0, "byteOffset": {uv_offset}, "byteLength": {uv_len} }},
    {{ "buffer": 0, "byteOffset": {index_offset}, "byteLength": 6 }}
  ],
  "buffers": [{{ "byteLength": {total}, "uri": "data:application/octet-stream;base64,{data}" }}]
}}"#,
        uv_len = index_offset - uv_offset,
        data = base64(&buffer),
    )
}

/// A directory holding the glTF and whatever images it names.
fn write_model(name: &str, gltf: &str, images: &[&str]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tequila-gltf-textures-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    for image in images {
        // Not a real PNG: the importer records a path and the renderer decodes
        // it later, so what matters here is that the file exists.
        std::fs::write(dir.join(image), b"not really a png").expect("an image file");
    }
    let path = dir.join("scene.gltf");
    std::fs::write(&path, gltf).expect("the model");
    path
}

/// A glTF's images arrive as textures on the material that asked for them.
///
/// The importer read UVs and ignored every image in the file, so a textured
/// model — which is what a downloaded asset almost always is — imported as
/// flat colour. Nothing said so.
#[test]
fn a_textured_gltf_brings_its_images_in() {
    let path = write_model(
        "basic",
        &textured_gltf("colour.png", "bumps.png"),
        &["colour.png", "bumps.png"],
    );

    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &path, &ImportOptions::default())
            .expect("the model imports");
    assert_eq!(report.textures, 2, "both images should have come in");

    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("the import applies");

    // The document knows about them...
    assert_eq!(document.textures.len(), 2);
    let names: Vec<String> = document
        .textures
        .iter()
        .map(|(_, source)| source.display_name())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("colour")),
        "the base colour map is not in the document: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("bumps")),
        "the normal map is not in the document: {names:?}"
    );

    // ...and the material's graph refers to them, by ids the document
    // recognises. An id the store does not have would render as the fallback
    // and look exactly like no texture at all.
    let material = document
        .materials
        .iter()
        .next()
        .map(|(_, m)| m.clone())
        .expect("a material came in");
    let used = material.graph.compile().expect("compiles").requirements.textures;
    assert_eq!(used.len(), 2, "the graph refers to {} textures", used.len());
    for id in used {
        assert!(
            document.textures.get(id).is_some(),
            "the graph refers to {id:?}, which the document does not have"
        );
    }
}

/// An emissive map says where a surface glows: it drives the emission
/// colour. Without it the factor lit the whole surface, and a bulb's screw
/// base shone as brightly as its filament.
#[test]
fn an_emissive_map_says_where_a_surface_glows() {
    use tequila_materials::material_type::Param;
    let gltf = textured_gltf("colour.png", "bumps.png").replace(
        r#""normalTexture": { "index": 1 }"#,
        r#""normalTexture": { "index": 1 }, "emissiveTexture": { "index": 0 }, "emissiveFactor": [1, 1, 1]"#,
    );
    assert!(gltf.contains("emissiveTexture"), "the fixture did not take the emissive map");
    let path = write_model("emissive", &gltf, &["colour.png", "bumps.png"]);
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default()).expect("imports");
    CommandBus::new().apply(&mut document, command).expect("applies");
    let material = document.materials.iter().next().map(|(_, m)| m.clone()).expect("a material");
    let program = material.graph.compile().expect("compiles");
    assert!(
        program.driven.iter().any(|(param, _)| *param == Param::EmissionColor),
        "nothing drives the emission colour"
    );
}

/// Transmissive glass is tinted by its base colour, as glTF and Blender
/// tint it — factor and map alike. A dirty bulb's grime is base colour, and
/// left out the glass came in spotless.
#[test]
fn transmissive_glass_is_tinted_by_its_base_colour() {
    use tequila_materials::material_type::Param;
    let gltf = textured_gltf("colour.png", "bumps.png").replace(
        r#""normalTexture": { "index": 1 }"#,
        r#""normalTexture": { "index": 1 }, "extensions": { "KHR_materials_transmission": { "transmissionFactor": 1.0 } }"#,
    );
    assert!(gltf.contains("KHR_materials_transmission"), "the fixture did not take the transmission");
    let path = write_model("transmission-tint", &gltf, &["colour.png", "bumps.png"]);
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default()).expect("imports");
    CommandBus::new().apply(&mut document, command).expect("applies");
    let material = document.materials.iter().next().map(|(_, m)| m.clone()).expect("a material");
    assert_eq!(material.material().transmission_color, material.material().base_color, "the factor does not tint the glass");
    let program = material.graph.compile().expect("compiles");
    assert!(
        program.driven.iter().any(|(param, _)| *param == Param::TransmissionColor),
        "the base colour map does not tint the glass"
    );
}

/// Transmission with no volume is a thin shell, as the specification says:
/// a bulb, a window. Read as solid, a bulb's one-surface glass refracted
/// like a lens.
#[test]
fn transmission_without_a_volume_is_thin_walled() {
    let m = material_with_extensions(
        "ext-transmission-thin",
        r#"{ "KHR_materials_transmission": { "transmissionFactor": 1.0 } }"#,
    );
    assert!(m.geometry_thin_walled, "transmission without a volume should be a thin shell");
}

/// Undoing an import takes its textures back out.
#[test]
fn undoing_an_import_removes_its_textures() {
    let path = write_model(
        "undo",
        &textured_gltf("colour.png", "bumps.png"),
        &["colour.png", "bumps.png"],
    );
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    assert_eq!(document.textures.len(), 2);

    bus.undo(&mut document).expect("undo");
    assert!(
        document.textures.is_empty(),
        "undo left {} textures behind",
        document.textures.len()
    );
    bus.redo(&mut document).expect("redo");
    assert_eq!(document.textures.len(), 2, "redo did not bring them back");
}

/// An image the model names and does not ship is a warning, not a failure.
///
/// A downloaded model with a missing map should still import: the geometry is
/// the part that cannot be replaced, and §5.1's placeholder rule means the
/// material keeps its own colour where the map would have been.
#[test]
fn a_missing_image_warns_and_imports_anyway() {
    let path = write_model(
        "missing",
        &textured_gltf("colour.png", "gone.png"),
        &["colour.png"],
    );
    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &path, &ImportOptions::default())
            .expect("a missing image must not fail the import");
    assert!(
        report.warnings.iter().any(|w| w.contains("gone.png")),
        "nothing warned about the missing image: {:?}",
        report.warnings
    );
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    assert_eq!(document.tree.len(), 1, "the geometry should still arrive");
}

/// An image URI is a URI, so a space arrives percent-encoded.
#[test]
fn a_percent_encoded_image_name_is_decoded() {
    let path = write_model(
        "encoded",
        &textured_gltf("base%20colour.png", "bumps.png"),
        &["base colour.png", "bumps.png"],
    );
    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &path, &ImportOptions::default())
            .expect("imports");
    assert!(
        report.warnings.is_empty(),
        "a percent-encoded name was not resolved: {:?}",
        report.warnings
    );
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    assert!(document
        .textures
        .iter()
        .any(|(_, s)| s.display_name().contains("base colour")));
}

/// The same triangle with an ORM map: one image carrying occlusion, roughness
/// and metalness, referenced twice.
///
/// This is what an exported asset actually looks like. `occlusionTexture` and
/// `metallicRoughnessTexture` point at one file, because occlusion is in its
/// red channel and the other two are in green and blue.
fn orm_gltf(strength: Option<f32>) -> String {
    let occlusion = match strength {
        None => r#"{ "index": 2 }"#.to_string(),
        Some(s) => format!(r#"{{ "index": 2, "strength": {s} }}"#),
    };
    textured_gltf("colour.png", "bumps.png")
        .replace(
            r#""normalTexture": { "index": 1 }"#,
            &format!(
                r#""normalTexture": {{ "index": 1 }},
      "occlusionTexture": {occlusion}"#
            ),
        )
        .replace(
            r#""baseColorTexture": { "index": 0 }"#,
            r#""baseColorTexture": { "index": 0 },
          "metallicRoughnessTexture": { "index": 2 }"#,
        )
        .replace(
            r#""textures": [{ "source": 0 }, { "source": 1 }]"#,
            r#""textures": [{ "source": 0 }, { "source": 1 }, { "source": 2 }]"#,
        )
        .replace(
            r#""images": [{ "uri": "colour.png" }, { "uri": "bumps.png" }]"#,
            r#""images": [{ "uri": "colour.png" }, { "uri": "bumps.png" }, { "uri": "orm.png" }]"#,
        )
}

/// Walk back from a parameter on the surface node, listing the nodes that feed
/// it in order.
fn chain_into(graph: &tequila_materials::graph::MaterialGraph, param: Param) -> Vec<String> {
    use tequila_materials::graph::Socket;

    let mut at = graph
        .links()
        .iter()
        .find(|l| l.to == graph.output() && l.to_socket == Socket::Param(param))
        .map(|l| l.from);
    let mut out = Vec::new();
    while let Some(id) = at {
        let node = graph.node(id).expect("a link to a node that exists");
        out.push(format!("{:?}", node.kind));
        at = graph
            .links()
            .iter()
            .find(|l| l.to == id && l.to_socket == Socket::Input(0))
            .map(|l| l.from);
    }
    out
}

/// A model's baked occlusion arrives, out of the red channel of the image it
/// shares with the metallic-roughness map.
///
/// The importer read `metallicRoughnessTexture` and ignored `occlusionTexture`
/// beside it, so every crease an exporter had baked — the recesses, the panel
/// gaps, the channel a strap sits in, none of which is in the mesh — was
/// thrown away, and the model lit as if it were smooth.
#[test]
fn an_orm_packed_gltf_reads_occlusion_from_the_red_channel() {
    let path = write_model(
        "orm",
        &orm_gltf(None),
        &["colour.png", "bumps.png", "orm.png"],
    );
    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &path, &ImportOptions::default())
            .expect("imports");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    // Three files, not four: occlusion and metallic-roughness name one image
    // and it should be decoded once.
    assert_eq!(
        report.textures, 3,
        "the ORM image was brought in more than once"
    );

    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document
        .materials
        .iter()
        .next()
        .map(|(_, m)| m.clone())
        .expect("a material came in");

    let chain = chain_into(&material.graph, Param::Occlusion);
    assert!(
        chain.iter().any(|n| n.contains("Channel") && n.contains("Red")),
        "occlusion is not fed by the red channel: {chain:?}"
    );
    assert!(
        chain.iter().any(|n| n.starts_with("ImageTexture")),
        "occlusion is not fed by an image at all: {chain:?}"
    );

    // And it is the *same* image the other two channels come from, by id.
    let image_of = |param| {
        chain_into(&material.graph, param)
            .into_iter()
            .find(|n| n.starts_with("ImageTexture"))
            .unwrap_or_else(|| panic!("{param:?} is not fed by an image"))
    };
    assert_eq!(
        image_of(Param::Occlusion),
        image_of(Param::SpecularRoughness),
        "occlusion and roughness came from different textures"
    );
    assert_eq!(
        image_of(Param::Occlusion),
        image_of(Param::BaseMetalness),
        "occlusion and metalness came from different textures"
    );
}

/// `strength` fades the map towards unoccluded, which is not a multiply.
///
/// The specification defines it as `lerp(1, occlusion, strength)`. Scaling the
/// channel instead would run the control backwards: lowering the strength
/// would darken the model, and strength 0 would render it black rather than
/// turning the map off.
#[test]
fn occlusion_strength_fades_towards_unoccluded() {
    let path = write_model(
        "orm-strength",
        &orm_gltf(Some(0.25)),
        &["colour.png", "bumps.png", "orm.png"],
    );
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document
        .materials
        .iter()
        .next()
        .map(|(_, m)| m.clone())
        .expect("a material came in");

    let chain = chain_into(&material.graph, Param::Occlusion);
    let remap = chain
        .iter()
        .find(|n| n.starts_with("Remap"))
        .unwrap_or_else(|| panic!("strength did not reach the graph: {chain:?}"));
    // 0..1 onto 0.75..1, so a fully occluded texel arrives at 0.75 rather than
    // at 0.25, which is what the multiply would have produced.
    assert!(
        remap.contains("0.75"),
        "strength 0.25 should remap onto 0.75..1, and the node reads {remap}"
    );

    // The other direction: no strength at all leaves the chain bare.
    let path = write_model(
        "orm-plain",
        &orm_gltf(None),
        &["colour.png", "bumps.png", "orm.png"],
    );
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document.materials.iter().next().map(|(_, m)| m.clone()).unwrap();
    assert!(
        !chain_into(&material.graph, Param::Occlusion)
            .iter()
            .any(|n| n.starts_with("Remap")),
        "a model with no strength should not carry a node that does nothing"
    );
}

/// A map in a format the renderer cannot decode says so at import.
///
/// This was the quieter of the two texture failures and the worse one: the
/// file is present, so the existence check passed, the map appeared in the
/// panel and the graph referred to it — and the render came out flat, because
/// the decode failed much later and the node quietly fell back to the
/// parameter's own colour. Nothing between those two points had anywhere to
/// put a complaint.
#[test]
fn a_map_in_an_unreadable_format_is_reported_at_import() {
    // HEIC: its decoders are C libraries, so it stays unread.
    let path = write_model(
        "unreadable",
        &textured_gltf("colour.heic", "bumps.png"),
        &["colour.heic", "bumps.png"],
    );
    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &path, &ImportOptions::default())
            .expect("an unreadable map must not fail the import");

    let complaint = report
        .warnings
        .iter()
        .find(|w| w.contains("colour.heic"))
        .unwrap_or_else(|| panic!("nothing warned about the .heic: {:?}", report.warnings));
    assert!(
        complaint.contains("format"),
        "the warning does not say what is wrong: {complaint}"
    );
    assert!(
        complaint.contains("png") && complaint.contains("jpg"),
        "the warning should say which formats do work: {complaint}"
    );

    // The geometry still arrives, and so does the map that is readable.
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    assert_eq!(document.tree.len(), 1, "the geometry should still arrive");
    assert!(
        !report.warnings.iter().any(|w| w.contains("bumps.png")),
        "the readable map was complained about too: {:?}",
        report.warnings
    );
}

/// JPEG is the format a downloaded model's colour map usually is, so a model
/// that ships one must import without a complaint.
///
/// The pairing matters as much as the case above: a list of readable formats
/// that has drifted from what the decoder does would either warn about a map
/// that works or stay quiet about one that does not, and only checking both
/// directions catches the drift.
#[test]
fn a_jpeg_map_imports_without_complaint() {
    let path = write_model(
        "jpeg",
        &textured_gltf("colour.jpg", "bumps.JPEG"),
        &["colour.jpg", "bumps.JPEG"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report.warnings.is_empty(),
        "a JPEG map was refused: {:?}",
        report.warnings
    );
    assert_eq!(report.textures, 2);
}

/// `EXT_texture_webp` names its WebP inside the texture, with the plain
/// `source` left as a PNG for readers without it. This one has it, so the
/// WebP is used and the extension is not reported as unread.
#[test]
fn ext_texture_webp_is_read_for_its_webp() {
    let gltf = textured_gltf("fallback.png", "bumps.png")
        .replacen(
            r#""textures": [{ "source": 0 }"#,
            r#""extensionsUsed": ["EXT_texture_webp"],
  "textures": [{ "source": 0, "extensions": { "EXT_texture_webp": { "source": 2 } } }"#,
            1,
        )
        .replacen(r#"{ "uri": "bumps.png" }]"#, r#"{ "uri": "bumps.png" }, { "uri": "colour.webp" }]"#, 1);
    assert!(gltf.contains("colour.webp") && gltf.contains("EXT_texture_webp"), "the helper's JSON changed shape");
    let path = write_model("webp", &gltf, &["fallback.png", "bumps.png", "colour.webp"]);
    let mut document = SceneDocument::new();
    let (command, report) = tequila_import::import(&document, None, &path, &ImportOptions::default()).expect("imports");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    CommandBus::new().apply(&mut document, command).expect("applies");
    let names: Vec<String> = document.textures.iter().map(|(_, source)| source.display_name()).collect();
    assert!(names.iter().any(|n| n.contains("colour")), "the WebP was not used: {names:?}");
    assert!(!names.iter().any(|n| n.contains("fallback")), "the fallback was used: {names:?}");
}

/// The same for the formats game-engine and texture-painting exports bring:
/// TGA above all, and TIFF, WebP, BMP and DDS.
#[test]
fn game_engine_map_formats_import_without_complaint() {
    for (colour, bumps) in [("colour.tga", "bumps.TIF"), ("colour.webp", "bumps.dds"), ("colour.bmp", "bumps.tiff")] {
        let path = write_model("engine", &textured_gltf(colour, bumps), &[colour, bumps]);
        let (_, report) = tequila_import::import(&SceneDocument::new(), None, &path, &ImportOptions::default())
            .expect("imports");
        assert!(report.warnings.is_empty(), "{colour} or {bumps} was refused: {:?}", report.warnings);
        assert_eq!(report.textures, 2);
    }
}

/// A one-pixel PNG, as bytes.
///
/// Written out rather than checked in so the test is self-contained, and a
/// real one rather than a placeholder because the importer now decides an
/// embedded image's file extension from its magic bytes when the model does
/// not say.
fn one_pixel_png() -> Vec<u8> {
    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(tag);
        out.extend_from_slice(body);
        let mut crc = 0xFFFF_FFFFu32;
        for byte in tag.iter().chain(body) {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
        out
    }

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&1u32.to_be_bytes()); // width
    ihdr.extend_from_slice(&1u32.to_be_bytes()); // height
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour
    png.extend(chunk(b"IHDR", &ihdr));
    // One scanline: filter byte 0, then one RGB pixel. Stored uncompressed in
    // a zlib wrapper, which every decoder accepts.
    let raw = [0u8, 200, 40, 40];
    let mut idat = vec![0x78, 0x01, 0x01, 4, 0, 0xFB, 0xFF];
    idat.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in raw {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    idat.extend_from_slice(&((b << 16) | a).to_be_bytes());
    png.extend(chunk(b"IDAT", &idat));
    png.extend(chunk(b"IEND", &[]));
    png
}

/// A GLB whose colour map lives in its own binary chunk.
///
/// The format most models are downloaded as, and the reason this case matters:
/// a `.glb` is one file, so there is no image beside it to find.
fn glb_with_embedded_image(name: Option<&str>) -> Vec<u8> {
    let mut geometry = triangle_positions();
    let uv_offset = geometry.len();
    for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
        for c in uv {
            geometry.extend_from_slice(&c.to_le_bytes());
        }
    }
    let index_offset = geometry.len();
    geometry.extend_from_slice(&indices());
    let image_offset = geometry.len();
    let image = one_pixel_png();
    geometry.extend_from_slice(&image);
    let total = geometry.len();

    let named = match name {
        Some(n) => format!(r#""name":"{n}","#),
        None => String::new(),
    };
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"Part","mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
"materials":[{{"name":"Painted","pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
"textures":[{{"source":0}}],
"images":[{{{named}"bufferView":3,"mimeType":"image/png"}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
             {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}},
             {{"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{uv_offset}}},
               {{"buffer":0,"byteOffset":{uv_offset},"byteLength":{uv_len}}},
               {{"buffer":0,"byteOffset":{index_offset},"byteLength":6}},
               {{"buffer":0,"byteOffset":{image_offset},"byteLength":{image_len}}}],
"buffers":[{{"byteLength":{total}}}]}}"#,
        uv_len = index_offset - uv_offset,
        image_len = image.len(),
    );

    let mut json_chunk = json.into_bytes();
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' ');
    }
    let mut bin_chunk = geometry;
    while bin_chunk.len() % 4 != 0 {
        bin_chunk.push(0);
    }
    let length = 12 + 8 + json_chunk.len() + 8 + bin_chunk.len();
    let mut glb = Vec::with_capacity(length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(length as u32).to_le_bytes());
    glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&(bin_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin_chunk);
    glb
}

/// A GLB's own images are written out beside it and used.
///
/// They used to be skipped with a warning, so the single-file format almost
/// everything is downloaded as imported with no textures at all. The document
/// stores paths and never pixels (`docs/file-format.md` §3.1), so an embedded
/// image has to become a file before it can be referred to.
#[test]
fn a_glb_extracts_the_images_it_carries() {
    let dir = std::env::temp_dir().join("tequila-glb-embedded");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let model = dir.join("headset.glb");
    std::fs::write(&model, glb_with_embedded_image(Some("Quest3 baseColor"))).expect("the model");

    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &model, &ImportOptions::default())
            .expect("the model imports");
    assert!(
        report.warnings.is_empty(),
        "an embedded image should no longer be a complaint: {:?}",
        report.warnings
    );
    assert_eq!(report.textures, 1, "the embedded image did not come in");

    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let (_, source) = document.textures.iter().next().expect("a texture");

    // Beside the model, in a folder named after it, so two models sharing a
    // download folder cannot write over each other.
    assert_eq!(
        source.path.parent().map(|p| p.to_path_buf()),
        Some(dir.join("headset.textures")),
        "the image went somewhere unexpected: {}",
        source.path.display()
    );
    assert!(
        source.path.exists(),
        "{} was recorded but not written",
        source.path.display()
    );
    // The glTF's own name for the image, reduced to something safe to put in a
    // path, and the extension its bytes call for.
    let file = source.path.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        file.starts_with("Quest3_baseColor") && file.ends_with(".png"),
        "the extracted file is called {file}"
    );

    // And the bytes are the image, not a truncated view of the buffer.
    let written = std::fs::read(&source.path).expect("the extracted file reads");
    assert_eq!(written, one_pixel_png(), "the extracted bytes are not the image");
    assert!(
        tequila_texture::image::Image::load(&source.path).is_ok(),
        "the extracted file does not decode"
    );
}

/// An unnamed image still gets a usable filename.
#[test]
fn an_unnamed_embedded_image_is_numbered() {
    let dir = std::env::temp_dir().join("tequila-glb-unnamed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let model = dir.join("part.glb");
    std::fs::write(&model, glb_with_embedded_image(None)).expect("the model");

    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &model, &ImportOptions::default())
            .expect("imports");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let (_, source) = document.textures.iter().next().expect("a texture");
    assert_eq!(
        source.path.file_name().unwrap().to_string_lossy(),
        "image_0.png"
    );
    assert!(source.path.exists());
}

/// A `.gltf` can embed its images as `data:` URIs instead, which is what
/// "embed textures" in an exporter usually produces.
#[test]
fn a_data_uri_image_is_extracted_too() {
    let dir = std::env::temp_dir().join("tequila-datauri");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");

    let encoded = format!(
        "data:image/png;base64,{}",
        base64(&one_pixel_png())
    );
    let gltf = textured_gltf(&encoded, "bumps.png");
    std::fs::write(dir.join("bumps.png"), one_pixel_png()).expect("the normal map");
    let model = dir.join("scene.gltf");
    std::fs::write(&model, gltf).expect("the model");

    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &model, &ImportOptions::default())
            .expect("imports");
    assert!(
        report.warnings.is_empty(),
        "a data URI should no longer be a complaint: {:?}",
        report.warnings
    );
    assert_eq!(report.textures, 2);

    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let extracted: Vec<_> = document
        .textures
        .iter()
        .map(|(_, s)| s.path.clone())
        .filter(|p| p.parent() == Some(&dir.join("scene.textures")))
        .collect();
    assert_eq!(
        extracted.len(),
        1,
        "exactly the embedded one should have been written out, not {extracted:?}"
    );
    assert!(extracted[0].exists());
    // The map that was already a file stays where it is.
    assert!(document
        .textures
        .iter()
        .any(|(_, s)| s.path == dir.join("bumps.png")));
}

/// An embedded image named something hostile cannot climb out of the folder.
///
/// The same rule the sibling-file resolver applies: a model from a stranger
/// must not be able to choose where the importer writes.
#[test]
fn an_embedded_image_cannot_escape_its_folder() {
    let dir = std::env::temp_dir().join("tequila-glb-escape");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let model = dir.join("hostile.glb");
    std::fs::write(
        &model,
        glb_with_embedded_image(Some("....//....//escaped")),
    )
    .expect("the model");

    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &model, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let (_, source) = document.textures.iter().next().expect("a texture");

    assert_eq!(
        source.path.parent().map(|p| p.to_path_buf()),
        Some(dir.join("hostile.textures")),
        "the image escaped to {}",
        source.path.display()
    );
    let file = source.path.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        !file.contains('/') && !file.contains('\\') && !file.contains(".."),
        "the filename kept something dangerous: {file}"
    );
}

/// A GLB's padding is padding, not surplus data to complain about.
///
/// The binary chunk is aligned to four bytes, and the specification allows its
/// length to exceed the declared `buffer.byteLength` by up to three because of
/// it. Treating that as a surplus meant every GLB whose buffer was not already
/// a multiple of four imported with a warning it had done nothing to earn —
/// and the existing GLB test never saw it, because its buffer happened to be
/// aligned. Both directions are here so the slack cannot quietly widen into
/// ignoring a real mismatch.
#[test]
fn a_glb_is_not_accused_of_its_own_padding() {
    // Three bytes of payload, so the chunk is padded by exactly one.
    let quiet = minimal_glb(3, 3);
    let scene = read_format(&quiet, "glb", "file", &|_| None).expect("reads");
    assert!(
        scene.warnings.is_empty(),
        "padding was mistaken for surplus: {:?}",
        scene.warnings
    );

    // Four bytes beyond the declaration is past any padding rule.
    let loud = minimal_glb(3, 8);
    let scene = read_format(&loud, "glb", "file", &|_| None).expect("reads");
    assert!(
        scene.warnings.iter().any(|w| w.contains("longer than")),
        "a genuinely over-long buffer went unmentioned: {:?}",
        scene.warnings
    );
}

/// A GLB with no geometry, whose buffer declares `declared` bytes and whose
/// binary chunk actually carries `actual` (before four-byte padding).
fn minimal_glb(declared: usize, actual: usize) -> Vec<u8> {
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"Empty"}}],
"buffers":[{{"byteLength":{declared}}}]}}"#
    );
    let mut json_chunk = json.into_bytes();
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' ');
    }
    let mut bin_chunk = vec![7u8; actual];
    while bin_chunk.len() % 4 != 0 {
        bin_chunk.push(0);
    }

    let length = 12 + 8 + json_chunk.len() + 8 + bin_chunk.len();
    let mut glb = Vec::with_capacity(length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(length as u32).to_le_bytes());
    glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&(bin_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin_chunk);
    glb
}

/// A glTF whose base-colour texture carries `KHR_texture_transform`.
fn transformed_gltf(extension: &str) -> String {
    textured_gltf("colour.png", "bumps.png").replace(
        r#""baseColorTexture": { "index": 0 }"#,
        &format!(
            r#""baseColorTexture": {{ "index": 0, "extensions": {{ "KHR_texture_transform": {extension} }} }}"#
        ),
    )
}

/// The mapping node a material's base colour is sampled through.
fn base_colour_mapping(
    graph: &tequila_materials::graph::MaterialGraph,
) -> tequila_texture::projection::Mapping {
    use tequila_materials::graph::{NodeKind, Socket};

    // Output <- image <- mapping.
    let image = graph
        .links()
        .iter()
        .find(|l| l.to == graph.output() && l.to_socket == Socket::Param(Param::BaseColor))
        .map(|l| l.from)
        .expect("base colour is fed by something");
    let coords = graph
        .links()
        .iter()
        .find(|l| l.to == image && l.to_socket == Socket::Input(0))
        .map(|l| l.from)
        .expect("the image is fed by a mapping");
    match &graph.node(coords).expect("the node exists").kind {
        NodeKind::Mapping2d { mapping } => *mapping,
        other => panic!("base colour is sampled through {other:?}"),
    }
}

/// `KHR_texture_transform`'s scale and offset reach the mapping node.
///
/// The extension is what an exporter writes whenever it atlases or tiles, and
/// ignoring it does not fail: it renders the whole atlas onto every part, or
/// tiles once where the model asked for four. Silent wrongness, which is the
/// kind this importer has been shedding.
#[test]
fn a_texture_transform_reaches_the_mapping() {
    let path = write_model(
        "transform",
        &transformed_gltf(r#"{ "scale": [4, 2], "offset": [0.25, 0.5] }"#),
        &["colour.png", "bumps.png"],
    );
    let mut document = SceneDocument::new();
    let (command, report) =
        tequila_import::import(&document, None, &path, &ImportOptions::default())
            .expect("imports");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document.materials.iter().next().map(|(_, m)| m.clone()).unwrap();
    let mapping = base_colour_mapping(&material.graph);

    assert!((mapping.scale.x - 4.0).abs() < 1e-6, "scale.x is {}", mapping.scale.x);
    assert!((mapping.scale.y - 2.0).abs() < 1e-6, "scale.y is {}", mapping.scale.y);
    assert!((mapping.offset.x - 0.25).abs() < 1e-6, "offset.x is {}", mapping.offset.x);
    assert!((mapping.offset.y - 0.5).abs() < 1e-6, "offset.y is {}", mapping.offset.y);
    assert!(
        !mapping.real_world_units,
        "glTF's scale is repeats, not a distance, so real-world units would \
         invert its meaning"
    );

    // A model without the extension must be left alone rather than given an
    // identity transform that differs from the default in some other field.
    let path = write_model(
        "plain",
        &textured_gltf("colour.png", "bumps.png"),
        &["colour.png", "bumps.png"],
    );
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document.materials.iter().next().map(|(_, m)| m.clone()).unwrap();
    assert_eq!(
        base_colour_mapping(&material.graph),
        tequila_texture::projection::Mapping::default()
    );
}

/// The rotation carries over as an angle in turns, in the same direction.
///
/// This is the one assumption in reading the extension, so it is written down
/// as a test rather than left in a comment. The specification gives its
/// matrices as flat arrays, which are column-major: the first column
/// `(cos, sin, 0)` and the second `(-sin, cos, 0)` make the same matrix
/// `Mapping` applies, so the angle transfers unchanged but for the unit. If
/// that reading is ever shown wrong, this is the test that has to change, and
/// the sign is deliberately checked against a quarter turn — where a flipped
/// sign is a quarter turn the other way rather than a rounding difference.
#[test]
fn a_texture_transform_rotation_is_turns_in_the_same_direction() {
    let path = write_model(
        "rotated",
        &transformed_gltf(r#"{ "rotation": 1.5707963267948966 }"#),
        &["colour.png", "bumps.png"],
    );
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document.materials.iter().next().map(|(_, m)| m.clone()).unwrap();
    let mapping = base_colour_mapping(&material.graph);

    assert!(
        (mapping.rotation - 0.25).abs() < 1e-6,
        "a quarter turn arrived as {} turns",
        mapping.rotation
    );
    // The scale must not have been disturbed by a transform that only rotates.
    assert!((mapping.scale.x - 1.0).abs() < 1e-6);
}

/// Only the first UV set reaches the shading, so a model asking for another
/// one is told rather than quietly textured from coordinates it did not pick.
#[test]
fn a_second_uv_set_is_reported() {
    let path = write_model(
        "texcoord",
        &transformed_gltf(r#"{ "scale": [1, 1], "texCoord": 1 }"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report.warnings.iter().any(|w| w.contains("UV set")),
        "nothing mentioned the second UV set: {:?}",
        report.warnings
    );
}

/// An extension the file says it cannot do without is reported.
///
/// `extensionsRequired` is the file stating outright that ignoring the
/// extension gives the wrong answer. Importing anyway is right — the geometry
/// is usually fine, and a model on screen can be judged where a refusal cannot
/// — but doing it silently leaves a render that is wrong with nothing to
/// account for it.
///
/// The second half is what keeps the list honest: an extension that *is*
/// implemented must not be reported, or the warning becomes noise people learn
/// to ignore.
#[test]
fn a_required_extension_that_is_not_implemented_is_reported() {
    let required = |list: &str| {
        textured_gltf("colour.png", "bumps.png").replace(
            r#""asset": { "version": "2.0" },"#,
            &format!(r#""asset": {{ "version": "2.0" }}, "extensionsRequired": {list},"#),
        )
    };

    // `KHR_materials_dispersion` stands in for the unimplemented, and is a
    // deliberate choice rather than an arbitrary one: it needs spectral
    // rendering, which `Param::Abbe` says the renderer does not have, so it
    // will stay unimplemented for as long as that is true. This fixture named
    // `KHR_materials_iridescence` until the day iridescence was implemented,
    // at which point it failed — correctly, and for a reason that took a
    // minute to understand rather than a second.
    let path = write_model(
        "required",
        &required(r#"["KHR_materials_dispersion", "EXT_mesh_gpu_instancing"]"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("a required extension must not fail the import");
    let complaint = report
        .warnings
        .iter()
        .find(|w| w.contains("KHR_materials_dispersion"))
        .unwrap_or_else(|| panic!("nothing reported it: {:?}", report.warnings));
    assert!(
        complaint.contains("EXT_mesh_gpu_instancing"),
        "only one of the two was named: {complaint}"
    );

    // And one that is implemented stays quiet.
    let path = write_model(
        "required-ok",
        &required(r#"["KHR_texture_transform"]"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report.warnings.is_empty(),
        "an implemented extension was reported as missing: {:?}",
        report.warnings
    );
}

/// A glTF whose base colour carries an alpha mode.
fn alpha_gltf(mode: &str, extra: &str) -> String {
    textured_gltf("colour.png", "bumps.png").replace(
        r#""name": "Painted","#,
        &format!(r#""name": "Painted", "alphaMode": "{mode}"{extra},"#),
    )
}

/// `alphaMode` wires the colour image's own alpha to opacity.
///
/// This is where an alpha-tested model keeps its holes — a leaf, a grille — and
/// until the graph could carry alpha at all, none of it arrived. `OPAQUE` is
/// checked alongside because the specification says alpha is ignored entirely
/// in that mode, factor included, and a model that happens to carry a
/// transparent base colour must still render solid.
#[test]
fn an_alpha_mode_drives_opacity_from_the_colour_image() {
    use tequila_materials::graph::{NodeKind, Socket};

    let drives_opacity = |gltf: &str, name: &str| -> bool {
        let path = write_model(name, gltf, &["colour.png", "bumps.png"]);
        let mut document = SceneDocument::new();
        let (command, _) =
            tequila_import::import(&document, None, &path, &ImportOptions::default())
                .expect("imports");
        let mut bus = CommandBus::new();
        bus.apply(&mut document, command).expect("applies");
        let material = document.materials.iter().next().map(|(_, m)| m.clone()).unwrap();
        let graph = &material.graph;
        let Some(feeding) = graph
            .links()
            .iter()
            .find(|l| l.to == graph.output() && l.to_socket == Socket::Param(Param::Opacity))
            .map(|l| l.from)
        else {
            return false;
        };
        // Either the alpha node directly, or a multiply by the factor.
        let mut at = Some(feeding);
        while let Some(id) = at {
            match &graph.node(id).expect("the node exists").kind {
                NodeKind::ImageAlpha { .. } => return true,
                _ => {
                    at = graph
                        .links()
                        .iter()
                        .find(|l| l.to == id && l.to_socket == Socket::Input(0))
                        .map(|l| l.from)
                }
            }
        }
        false
    };

    assert!(
        drives_opacity(&alpha_gltf("MASK", ""), "alpha-mask"),
        "MASK did not reach opacity"
    );
    assert!(
        drives_opacity(&alpha_gltf("BLEND", ""), "alpha-blend"),
        "BLEND did not reach opacity"
    );
    assert!(
        !drives_opacity(&alpha_gltf("OPAQUE", ""), "alpha-opaque"),
        "OPAQUE must ignore alpha entirely"
    );
    assert!(
        !drives_opacity(
            &textured_gltf("colour.png", "bumps.png"),
            "alpha-unstated"
        ),
        "no alphaMode at all means OPAQUE"
    );
}

/// The factor's alpha is honoured, and only when the mode asks for it.
#[test]
fn the_base_colour_factors_alpha_follows_the_mode() {
    let with_factor = |mode: &str, name: &str| -> f32 {
        let gltf = alpha_gltf(mode, "").replace(
            r#""baseColorFactor": [0.8, 0.2, 0.2, 1.0]"#,
            r#""baseColorFactor": [0.8, 0.2, 0.2, 0.25]"#,
        );
        let path = write_model(name, &gltf, &["colour.png", "bumps.png"]);
        let mut document = SceneDocument::new();
        let (command, _) =
            tequila_import::import(&document, None, &path, &ImportOptions::default())
                .expect("imports");
        let mut bus = CommandBus::new();
        bus.apply(&mut document, command).expect("applies");
        let opacity = document
            .materials
            .iter()
            .next()
            .map(|(_, m)| m.material().geometry_opacity)
            .unwrap();
        opacity
    };

    assert!(
        (with_factor("BLEND", "factor-blend") - 0.25).abs() < 1e-6,
        "BLEND ignored the factor's alpha"
    );
    assert!(
        (with_factor("OPAQUE", "factor-opaque") - 1.0).abs() < 1e-6,
        "OPAQUE must ignore the factor's alpha, which the specification is \
         explicit about"
    );
}

/// A cutoff the renderer does not apply is reported rather than silently
/// producing a softer edge than the file asked for.
#[test]
fn a_non_default_alpha_cutoff_is_reported() {
    let path = write_model(
        "cutoff",
        &alpha_gltf("MASK", r#", "alphaCutoff": 0.9"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report.warnings.iter().any(|w| w.contains("alphaCutoff")),
        "nothing mentioned the cutoff: {:?}",
        report.warnings
    );

    // The default is what every exporter writes, and saying so every time
    // would train people to ignore the warning.
    let path = write_model(
        "cutoff-default",
        &alpha_gltf("MASK", r#", "alphaCutoff": 0.5"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report.warnings.is_empty(),
        "the default cutoff should be silent: {:?}",
        report.warnings
    );
}

/// The imported material for a glTF whose material carries `extensions`.
fn material_with_extensions(name: &str, extensions: &str) -> tequila_materials::openpbr::Material {
    let gltf = textured_gltf("colour.png", "bumps.png").replace(
        r#""name": "Painted","#,
        &format!(r#""name": "Painted", "extensions": {extensions},"#),
    );
    let path = write_model(name, &gltf, &["colour.png", "bumps.png"]);
    let mut document = SceneDocument::new();
    let (command, _) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).expect("applies");
    let material = document
        .materials
        .iter()
        .next()
        .map(|(_, m)| m.material().clone())
        .expect("a material came in");
    material
}

/// The `KHR_materials_*` extensions this renderer can honour reach the
/// parameters they name.
///
/// glTF's core material is metallic-roughness and nothing else, so glass, a
/// clear coat and a measured IOR all arrive as extensions. Ignoring them is
/// not a failure with a message — it is a model that imports looking like
/// plastic.
#[test]
fn the_material_extensions_reach_their_parameters() {
    let m = material_with_extensions(
        "ext-ior",
        r#"{ "KHR_materials_ior": { "ior": 2.4 } }"#,
    );
    assert!((m.specular_ior - 2.4).abs() < 1e-6, "ior is {}", m.specular_ior);

    let m = material_with_extensions(
        "ext-transmission",
        r#"{ "KHR_materials_transmission": { "transmissionFactor": 0.85 } }"#,
    );
    assert!(
        (m.transmission_weight - 0.85).abs() < 1e-6,
        "transmission is {}",
        m.transmission_weight
    );

    let m = material_with_extensions(
        "ext-coat",
        r#"{ "KHR_materials_clearcoat": { "clearcoatFactor": 0.7, "clearcoatRoughnessFactor": 0.2 } }"#,
    );
    assert!((m.coat_weight - 0.7).abs() < 1e-6, "coat is {}", m.coat_weight);
    assert!((m.coat_roughness - 0.2).abs() < 1e-6);

    let m = material_with_extensions(
        "ext-specular",
        r#"{ "KHR_materials_specular": { "specularFactor": 0.3, "specularColorFactor": [1.0, 0.8, 0.5] } }"#,
    );
    assert!((m.specular_weight - 0.3).abs() < 1e-6);
    assert!((m.specular_color.to_linear_srgb().y - 0.8).abs() < 1e-3);

    let m = material_with_extensions(
        "ext-aniso",
        r#"{ "KHR_materials_anisotropy": { "anisotropyStrength": 0.6 } }"#,
    );
    assert!((m.specular_roughness_anisotropy - 0.6).abs() < 1e-6);

    let m = material_with_extensions(
        "ext-iridescence",
        r#"{ "KHR_materials_iridescence": { "iridescenceFactor": 0.9,
             "iridescenceIor": 2.4, "iridescenceThicknessMinimum": 100,
             "iridescenceThicknessMaximum": 550 } }"#,
    );
    assert!((m.thin_film_weight - 0.9).abs() < 1e-6, "film is {}", m.thin_film_weight);
    assert!((m.thin_film_ior - 2.4).abs() < 1e-6, "film ior is {}", m.thin_film_ior);
    // The maximum, not the minimum and not the middle: with no thickness map
    // the specification says the film is its maximum thickness everywhere.
    assert!(
        (m.thin_film_thickness - 550.0).abs() < 1e-3,
        "film thickness is {}",
        m.thin_film_thickness
    );

    // glTF defaults a film's index to 1.3, which is not the 1.5 it defaults
    // most of its other indices to, and not the 1.4 this renderer starts
    // from. A material that gives a factor and nothing else has to get 1.3.
    let m = material_with_extensions(
        "ext-iridescence-defaults",
        r#"{ "KHR_materials_iridescence": { "iridescenceFactor": 1.0 } }"#,
    );
    assert!((m.thin_film_ior - 1.3).abs() < 1e-6, "film ior is {}", m.thin_film_ior);
    assert!(
        (m.thin_film_thickness - 400.0).abs() < 1e-3,
        "film thickness is {}",
        m.thin_film_thickness
    );

    // A material with no extensions at all must be left exactly as the core
    // fields describe it.
    let plain = material_with_extensions("ext-none", "{}");
    assert!((plain.specular_ior - 1.5).abs() < 1e-6);
    assert_eq!(plain.transmission_weight, 0.0);
    assert_eq!(plain.coat_weight, 0.0);
}

/// A film painted by a texture has to say so.
///
/// glTF's thickness map is how an exporter paints the *pattern* of an oil
/// slick or a tempered finish, so a model carrying one is a model whose
/// iridescence is meant to vary across the surface. Taking the maximum
/// everywhere is a reasonable stand-in, and it is also a different material
/// from the one in the file — which is the sort of difference an importer
/// has to admit to rather than let someone discover by eye.
#[test]
fn an_iridescence_thickness_map_is_reported_rather_than_silently_flattened() {
    let gltf = textured_gltf("colour.png", "bumps.png").replace(
        r#""name": "Painted","#,
        r#""name": "Painted", "extensions": { "KHR_materials_iridescence": {
            "iridescenceFactor": 1.0,
            "iridescenceThicknessMaximum": 650,
            "iridescenceThicknessTexture": { "index": 0 } } },"#,
    );
    let path = write_model("iridescence-map", &gltf, &["colour.png", "bumps.png"]);
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("a map this importer cannot wire must not fail the import");

    let complaint = report
        .warnings
        .iter()
        .find(|w| w.contains("film thickness"))
        .unwrap_or_else(|| panic!("nothing warned: {:?}", report.warnings));
    assert!(
        complaint.contains("650"),
        "the warning does not say what was used instead: {complaint}"
    );
}

/// Emissive strength is the scale the core factor does not carry.
///
/// `emissiveFactor` is a 0..1 colour with no units; `emission_luminance` is in
/// nits. The extension is what connects them, and it only applies where there
/// is emission to scale.
#[test]
fn emissive_strength_scales_an_emitter_and_nothing_else() {
    let emissive = |name: &str, extensions: &str| {
        let gltf = textured_gltf("colour.png", "bumps.png")
            .replace(
                r#""name": "Painted","#,
                &format!(
                    r#""name": "Painted", "emissiveFactor": [1.0, 0.5, 0.2], "extensions": {extensions},"#
                ),
            );
        let path = write_model(name, &gltf, &["colour.png", "bumps.png"]);
        let mut document = SceneDocument::new();
        let (command, _) =
            tequila_import::import(&document, None, &path, &ImportOptions::default())
                .expect("imports");
        let mut bus = CommandBus::new();
        bus.apply(&mut document, command).expect("applies");
        let lum = document
            .materials
            .iter()
            .next()
            .map(|(_, m)| m.material().emission_luminance)
            .unwrap();
        lum
    };

    assert!(
        (emissive("emit-strength", r#"{ "KHR_materials_emissive_strength": { "emissiveStrength": 40.0 } }"#) - 40.0).abs() < 1e-6
    );
    // Without the extension the importer's own scale stands.
    assert!(emissive("emit-plain", "{}") > 0.0);

    // And a material with no emission is not turned into a light by a strength
    // that has nothing to multiply.
    let m = material_with_extensions(
        "emit-none",
        r#"{ "KHR_materials_emissive_strength": { "emissiveStrength": 40.0 } }"#,
    );
    assert_eq!(
        m.emission_luminance, 0.0,
        "a strength with no emissiveFactor behind it must not light the scene"
    );
}

/// glTF folds sheen's strength into its colour; §2.2 keeps them apart.
///
/// The translation is the only one in this batch that is not a direct copy, so
/// it gets its own check: the brightest channel becomes the weight, and the
/// colour is normalized against it so the tint survives.
#[test]
fn sheen_colour_is_split_into_a_weight_and_a_tint() {
    let m = material_with_extensions(
        "ext-sheen",
        r#"{ "KHR_materials_sheen": { "sheenColorFactor": [0.4, 0.2, 0.1], "sheenRoughnessFactor": 0.8 } }"#,
    );
    assert!((m.fuzz_weight - 0.4).abs() < 1e-3, "weight is {}", m.fuzz_weight);
    let tint = m.fuzz_color.to_linear_srgb();
    assert!((tint.x - 1.0).abs() < 1e-3, "tint red is {}", tint.x);
    assert!((tint.y - 0.5).abs() < 1e-3, "tint green is {}", tint.y);
    assert!((m.fuzz_roughness - 0.8).abs() < 1e-6);

    // Black sheen is no sheen, and must not divide by its own zero.
    let m = material_with_extensions(
        "ext-sheen-black",
        r#"{ "KHR_materials_sheen": { "sheenColorFactor": [0.0, 0.0, 0.0] } }"#,
    );
    assert_eq!(m.fuzz_weight, 0.0);
    assert!(m.fuzz_color.r.is_finite() && m.fuzz_color.g.is_finite());
}

/// An extension the file merely *uses* is reported too, not only a required
/// one — an unread material extension changes the render either way.
#[test]
fn a_used_but_unimplemented_extension_is_reported() {
    let used = |list: &str| {
        textured_gltf("colour.png", "bumps.png").replace(
            r#""asset": { "version": "2.0" },"#,
            &format!(r#""asset": {{ "version": "2.0" }}, "extensionsUsed": {list},"#),
        )
    };

    let path = write_model(
        "used-unknown",
        &used(r#"["KHR_materials_dispersion"]"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("KHR_materials_dispersion") && w.contains("uses")),
        "nothing reported the used extension: {:?}",
        report.warnings
    );

    // One that is implemented stays quiet, or the warning becomes noise.
    let path = write_model(
        "used-known",
        &used(r#"["KHR_materials_ior", "KHR_texture_transform"]"#),
        &["colour.png", "bumps.png"],
    );
    let document = SceneDocument::new();
    let (_, report) = tequila_import::import(&document, None, &path, &ImportOptions::default())
        .expect("imports");
    assert!(
        report.warnings.is_empty(),
        "implemented extensions were reported as missing: {:?}",
        report.warnings
    );
}

/// `KHR_materials_volume` is the other half of glass.
///
/// `transmission` says light passes through; this says what happens to it on
/// the way. §2.2 spells the two parameters the same way glTF does — a colour
/// reached at a distance — so they transfer directly.
#[test]
fn a_volume_extension_becomes_absorption() {
    // Paired with transmission on purpose: a volume nothing transmits into
    // has no interior for light to be absorbed in, and `has_volume` says so.
    let m = material_with_extensions(
        "ext-volume",
        r#"{ "KHR_materials_transmission": { "transmissionFactor": 1.0 },
             "KHR_materials_volume": { "thicknessFactor": 1.0, "attenuationDistance": 0.4, "attenuationColor": [0.2, 0.9, 0.6] } }"#,
    );
    assert!(!m.geometry_thin_walled, "a thickness means a solid, not a pane");
    assert!((m.transmission_depth - 0.4).abs() < 1e-6, "depth is {}", m.transmission_depth);
    let colour = m.transmission_color.to_linear_srgb();
    assert!((colour.x - 0.2).abs() < 1e-3);
    assert!((colour.y - 0.9).abs() < 1e-3);
    assert!(m.has_volume(), "the material should carry a volume");

    // And it absorbs per channel, deepening with distance.
    let near = m.transmittance_over(0.4);
    assert!((near.to_linear_srgb().x - 0.2).abs() < 1e-3, "one depth should reach the colour");
    let far = m.transmittance_over(0.8);
    assert!(far.r < near.r && far.g < near.g, "absorption must deepen");
}

/// `thicknessFactor` of zero is the specification's way of saying thin-walled,
/// and an absent `attenuationDistance` is infinite — neither absorbs.
#[test]
fn a_thin_walled_volume_absorbs_nothing() {
    let thin = material_with_extensions(
        "ext-volume-thin",
        r#"{ "KHR_materials_transmission": { "transmissionFactor": 1.0 },
             "KHR_materials_volume": { "thicknessFactor": 0.0, "attenuationDistance": 0.4, "attenuationColor": [0.2, 0.9, 0.6] } }"#,
    );
    assert!(thin.geometry_thin_walled, "no thickness is a pane");
    assert_eq!(
        thin.transmission_depth, 0.0,
        "a pane has no interior to absorb in, however short the distance says"
    );
    assert!(!thin.has_volume());

    // Infinite is the default, and means light crosses any thickness untouched.
    let endless = material_with_extensions(
        "ext-volume-endless",
        r#"{ "KHR_materials_transmission": { "transmissionFactor": 1.0 },
             "KHR_materials_volume": { "thicknessFactor": 1.0, "attenuationColor": [0.2, 0.9, 0.6] } }"#,
    );
    assert_eq!(
        endless.transmission_depth, 0.0,
        "an absent attenuationDistance is infinite, which is no absorption"
    );
    assert_eq!(endless.transmittance_over(1000.0), Spectrum::ONE);
}
