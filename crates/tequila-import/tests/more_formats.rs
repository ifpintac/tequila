//! PLY, 3MF and FBX, imported from files into a document: each at the right
//! size in metres, standing the right way up, where the file placed it,
//! with its materials and its part names.
//!
//! The PLY and 3MF files are written here; the FBX ones are Blender's
//! export of a synthetic scene (`data/make_fbx_fixtures.py`, which says
//! what is in it).

use std::path::{Path, PathBuf};
use tequila_core::math::{vec3, Vec3};
use tequila_core::units::LengthUnit;
use tequila_import::{import, ImportOptions, ImportReport};
use tequila_materials::graph::NodeKind as GraphNode;
use tequila_scene::tree::{NodeKind, SceneNode};
use tequila_scene::{CommandBus, SceneDocument};

/// A folder of this test's own, emptied.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tequila-more-formats-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Import a file where the file put it (not set on the ground), as a
/// document.
fn open(path: &Path, options: ImportOptions) -> (SceneDocument, ImportReport) {
    let mut document = SceneDocument::new();
    let (command, report) = import(&document, None, path, &options).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    CommandBus::new().apply(&mut document, command).unwrap();
    document.validate().unwrap();
    (document, report)
}

fn where_it_was() -> ImportOptions {
    ImportOptions { place_on_ground: false, ..Default::default() }
}

fn part<'a>(document: &'a SceneDocument, name: &str) -> &'a SceneNode {
    document
        .tree
        .iter()
        .find(|n| n.name == name && matches!(n.kind, NodeKind::Part { .. }))
        .unwrap_or_else(|| panic!("no part `{name}` among {:?}", document.tree.iter().map(|n| &n.name).collect::<Vec<_>>()))
}

/// A part's box in the world: its lowest and highest corners.
fn world_box(document: &SceneDocument, node: &SceneNode) -> (Vec3, Vec3) {
    let NodeKind::Part { mesh } = node.kind else { panic!("{} is not a part", node.name) };
    let world = document.tree.world_transform(node.id);
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for &p in &document.geometry.get(mesh).unwrap().positions {
        let p = world.point(p);
        low = vec3(low.x.min(p.x), low.y.min(p.y), low.z.min(p.z));
        high = vec3(high.x.max(p.x), high.y.max(p.y), high.z.max(p.z));
    }
    (low, high)
}

fn centre(document: &SceneDocument, name: &str) -> Vec3 {
    let (low, high) = world_box(document, part(document, name));
    (low + high) * 0.5
}

fn size(document: &SceneDocument, name: &str) -> Vec3 {
    let (low, high) = world_box(document, part(document, name));
    high - low
}

fn near(a: Vec3, b: Vec3, tolerance: f32) -> bool {
    (a - b).length() <= tolerance
}

fn material_of<'a>(document: &'a SceneDocument, name: &str) -> &'a tequila_scene::material::MaterialDefinition {
    let key = part(document, name).material.unwrap_or_else(|| panic!("`{name}` has no material"));
    document.materials.get(key).unwrap()
}

fn has_picture(definition: &tequila_scene::material::MaterialDefinition) -> bool {
    definition.graph.nodes().any(|(_, n)| matches!(n.kind, GraphNode::ImageTexture { .. }))
}

// ---------------------------------------------------------------- PLY

/// A box 40 x 20 x 10 (millimetres, as the user says when asked), painted
/// red, with its eight corners shared by six quads.
fn ply_box(format: &str) -> Vec<u8> {
    let corners = [[0.0f32, 0.0, 0.0], [40.0, 0.0, 0.0], [40.0, 20.0, 0.0], [0.0, 20.0, 0.0], [0.0, 0.0, 10.0], [40.0, 0.0, 10.0], [40.0, 20.0, 10.0], [0.0, 20.0, 10.0]];
    let faces: [[u32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [3, 0, 4, 7]];
    let mut out = format!(
        "ply\nformat {format} 1.0\ncomment a red box\nelement vertex 8\nproperty float x\nproperty float y\nproperty float z\n\
         property uchar red\nproperty uchar green\nproperty uchar blue\nelement face 6\nproperty list uchar int vertex_indices\nend_header\n"
    )
    .into_bytes();
    let big = format == "binary_big_endian";
    let float = |out: &mut Vec<u8>, v: f32| out.extend_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
    let int = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
    if format == "ascii" {
        for c in corners {
            out.extend_from_slice(format!("{} {} {} 230 20 10\n", c[0], c[1], c[2]).as_bytes());
        }
        for f in faces {
            out.extend_from_slice(format!("4 {} {} {} {}\n", f[0], f[1], f[2], f[3]).as_bytes());
        }
    } else {
        for c in corners {
            for v in c {
                float(&mut out, v);
            }
            out.extend_from_slice(&[230, 20, 10]);
        }
        for f in faces {
            out.push(4);
            for i in f {
                int(&mut out, i);
            }
        }
    }
    out
}

#[test]
fn a_ply_lands_at_its_size_with_its_colours_in_every_encoding() {
    let dir = folder("ply");
    for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
        let path = dir.join(format!("{format}.ply"));
        std::fs::write(&path, ply_box(format)).unwrap();

        // PLY carries no unit: guessed from the size, and flagged.
        let (_, report) = open(&path, where_it_was());
        assert!(report.unit_was_guessed, "{format}");

        let (document, report) = open(&path, ImportOptions { source_unit: Some(LengthUnit::Millimetre), ..Default::default() });
        assert_eq!(report.triangles, 12, "{format}");
        let box_size = size(&document, format);
        assert!(near(box_size, vec3(0.04, 0.02, 0.01), 1e-6), "{format}: {box_size:?}");
        // Set on the ground, as written otherwise: PLY has no up axis.
        let (low, _) = world_box(&document, part(&document, format));
        assert!(low.y.abs() < 1e-6, "{format}: the box's bottom is at {}", low.y);
        // Its colours, shown: a material that reads them.
        let definition = material_of(&document, format);
        assert!(definition.graph.nodes().any(|(_, n)| matches!(n.kind, GraphNode::VertexColor)), "{format}: the colours are not shown");
        let mesh = document.geometry.iter().next().unwrap().1;
        assert!(mesh.colors.iter().all(|c| (c.x - 0.791).abs() < 0.01 && c.y < 0.01), "{format}: {:?}", mesh.colors.first());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- 3MF

fn png_2x2() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 255, 255, 0, 128, 128, 0, 128, 128, 255, 255, 255]).unwrap();
    }
    out
}

/// A box's vertices and triangles, `w` x `d` x `h` along X, Y and Z (Z up,
/// as a printer's bed is), with `pid` on every triangle when given.
fn box_mesh(w: f32, d: f32, h: f32, triangle: impl Fn(usize) -> String) -> String {
    let v = [[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, d, 0.0], [0.0, d, 0.0], [0.0, 0.0, h], [w, 0.0, h], [w, d, h], [0.0, d, h]];
    let t = [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [2, 3, 7], [2, 7, 6], [1, 2, 6], [1, 6, 5], [3, 0, 4], [3, 4, 7]];
    let mut out = String::from("<mesh><vertices>");
    for p in v {
        out += &format!(r#"<vertex x="{}" y="{}" z="{}"/>"#, p[0], p[1], p[2]);
    }
    out += "</vertices><triangles>";
    for (i, c) in t.iter().enumerate() {
        out += &format!(r#"<triangle v1="{}" v2="{}" v3="{}" {}/>"#, c[0], c[1], c[2], triangle(i));
    }
    out + "</triangles></mesh>"
}

fn three_mf_package() -> Vec<u8> {
    let red_box = box_mesh(40.0, 20.0, 10.0, |_| String::new());
    // Its top and bottom blue, its sides red.
    let two_tone = box_mesh(20.0, 20.0, 20.0, |i| if i < 4 { r#"pid="1" p1="1""#.to_string() } else { String::new() });
    let tile = box_mesh(30.0, 30.0, 2.0, |i| match i % 2 {
        0 => r#"pid="4" p1="0" p2="2" p3="1""#.to_string(),
        _ => r#"pid="4" p1="0" p2="3" p3="2""#.to_string(),
    });
    let support = box_mesh(5.0, 5.0, 5.0, |_| String::new());
    let model = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:m="http://schemas.microsoft.com/3dmanufacturing/material/2015/02">
 <metadata name="Title">Print plate</metadata>
 <resources>
  <basematerials id="1"><base name="Red PLA" displaycolor="#FF0000"/><base name="Blue PLA" displaycolor="#0000FFFF"/></basematerials>
  <m:texture2d id="3" path="/3D/Textures/tile.png" contenttype="image/png"/>
  <m:texture2dgroup id="4" texid="3"><m:tex2coord u="0" v="0"/><m:tex2coord u="1" v="0"/><m:tex2coord u="1" v="1"/><m:tex2coord u="0" v="1"/></m:texture2dgroup>
  <object id="10" type="model" name="Box" pid="1" pindex="0">{red_box}</object>
  <object id="11" type="model" name="Two tone" pid="1" pindex="0">{two_tone}</object>
  <object id="12" type="model" name="Tile">{tile}</object>
  <object id="13" type="support" name="Support">{support}</object>
  <object id="20" type="model" name="Pair"><components>
   <component objectid="10" transform="1 0 0 0 1 0 0 0 1 0 50 0"/>
   <component objectid="10" transform="0 1 0 -1 0 0 0 0 1 0 100 0"/>
  </components></object>
 </resources>
 <build>
  <item objectid="10" transform="1 0 0 0 1 0 0 0 1 100 0 0" partnumber="BX-1"/>
  <item objectid="11" transform="1 0 0 0 1 0 0 0 1 0 0 30"/>
  <item objectid="12"/>
  <item objectid="13"/>
  <item objectid="20"/>
 </build>
</model>"##
    );
    let mut zip = tequila_format::zip::ZipWriter::new();
    zip.add("[Content_Types].xml", br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/><Default Extension="png" ContentType="image/png"/></Types>"#.to_vec());
    zip.add("_rels/.rels", br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#.to_vec());
    zip.add("3D/3dmodel.model", model.into_bytes());
    zip.add("3D/Textures/tile.png", png_2x2());
    zip.finish()
}

#[test]
fn a_3mf_lands_y_up_in_metres_with_its_objects_named_and_coloured() {
    let dir = folder("3mf");
    let path = dir.join("plate.3mf");
    std::fs::write(&path, three_mf_package()).unwrap();
    let (document, report) = open(&path, where_it_was());

    assert_eq!(report.source_unit, LengthUnit::Millimetre);
    assert!(!report.unit_was_guessed);
    assert!(report.warnings.iter().any(|w| w.contains("support")), "{:?}", report.warnings);

    // 40 x 20 x 10 mm, Z up: 40 along X, 10 up Y, 20 along Z, in metres.
    let size_of_box = size(&document, "Box");
    assert!(near(size_of_box, vec3(0.04, 0.01, 0.02), 1e-6), "{size_of_box:?}");
    // Its build item moved it 100 mm along X; 3MF's Y is the document's -Z.
    let at = world_box(&document, document.tree.iter().find(|n| n.name == "Box" && n.metadata.contains_key("Part number")).unwrap());
    assert!(near(at.0, vec3(0.1, 0.0, -0.02), 1e-6), "{:?}", at.0);

    // An object of two looks is a group of a part each, named by material.
    let two_tone = document.tree.iter().find(|n| n.name == "Two tone").unwrap();
    assert!(matches!(two_tone.kind, NodeKind::Group));
    let red = part(&document, "Red PLA");
    let blue = part(&document, "Blue PLA");
    assert_eq!(red.parent, Some(two_tone.id));
    assert_eq!(blue.parent, Some(two_tone.id));
    // Raised 30 mm: up the document's Y.
    let (low, high) = world_box(&document, blue);
    assert!((low.y - 0.03).abs() < 1e-6 && (high.y - 0.05).abs() < 1e-6, "{low:?} {high:?}");
    let colour = material_of(&document, "Box").material().base_color.to_linear_srgb();
    assert!(colour.x > 0.99 && colour.y < 0.01 && colour.z < 0.01, "{colour:?}");
    let colour = material_of(&document, "Blue PLA").material().base_color.to_linear_srgb();
    assert!(colour.z > 0.99 && colour.x < 0.01, "{colour:?}");

    // The texture group: the package's picture on the tile, written out
    // beside the model, and its texture coordinates.
    let tile = material_of(&document, "Tile");
    assert!(has_picture(tile), "the tile has no picture");
    assert_eq!(report.textures, 1);
    let (_, source) = document.textures.iter().next().unwrap();
    assert!(source.path.exists() && source.path.starts_with(dir.join("plate.textures")), "{}", source.path.display());
    let NodeKind::Part { mesh } = part(&document, "Tile").kind else { unreachable!() };
    assert!(!document.geometry.get(mesh).unwrap().uv_sets.is_empty(), "the tile has no texture coordinates");

    // Components: a group of the box twice, one mesh for all three boxes.
    let pair = document.tree.iter().find(|n| n.name == "Pair").unwrap();
    let boxes: Vec<_> = document.tree.iter().filter(|n| n.name == "Box").collect();
    assert_eq!(boxes.len(), 3);
    assert_eq!(boxes.iter().filter(|n| n.parent == Some(pair.id)).count(), 2);
    let meshes: std::collections::BTreeSet<_> = boxes.iter().map(|n| match n.kind { NodeKind::Part { mesh } => mesh, _ => unreachable!() }).collect();
    assert_eq!(meshes.len(), 1, "an object placed three times is one mesh");
    // The second component turned 90 degrees about Z: its 40 mm side along
    // the package's Y, which is the document's Z.
    let turned = boxes.iter().filter(|n| n.parent == Some(pair.id)).map(|n| world_box(&document, n)).find(|(l, h)| (h.z - l.z) > 0.03).expect("the turned box");
    assert!(near(turned.1 - turned.0, vec3(0.02, 0.01, 0.04), 1e-6), "{:?}", turned.1 - turned.0);

    // The top group carries the package's title.
    let top = document.tree.iter().find(|n| n.parent.is_none()).unwrap();
    assert_eq!(top.name, "plate");
    assert_eq!(top.metadata.get("Title").map(String::as_str), Some("Print plate"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- FBX

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

/// The fixtures, copied where an import may write beside them, into a
/// folder of the test's own (`test`), since tests run side by side.
fn fbx_copy(name: &str, test: &str) -> (PathBuf, PathBuf) {
    let dir = folder(test);
    std::fs::copy(fixture(name), dir.join(name)).unwrap();
    std::fs::copy(fixture("checker.png"), dir.join("checker.png")).unwrap();
    (dir.join(name), dir)
}

/// Where Blender put each object's centre (the script prints them), in
/// millimetres, Z up, as the document has them: metres, Y up.
fn blender(x: f32, y: f32, z: f32) -> Vec3 {
    vec3(x, z, -y) * 0.001
}

fn check_fbx(document: &SceneDocument, report: &ImportReport, file: &str) {
    // Placed where Blender placed them, through the hierarchy: the Ball is
    // a child of the tilted Column.
    for (name, expected) in [
        ("Plinth", blender(0.0, 0.0, 6.0)),
        ("Column", blender(-15.0, 0.0, 37.0)),
        ("Ball", blender(-2.3453, 0.0, 71.7686)),
        ("Ball copy", blender(22.0, 15.0, 24.0)),
    ] {
        let at = centre(document, name);
        assert!(near(at, expected, 2e-6), "{file}: {name} is at {at:?}, not {expected:?}");
    }
    // At its size in metres, standing up: the plinth is 12 mm tall.
    let plinth = size(document, "Plinth");
    assert!(near(plinth, vec3(0.08, 0.012, 0.06), 1e-6), "{file}: {plinth:?}");
    let ball = size(document, "Ball copy");
    assert!((ball.y - 0.012).abs() < 1e-5, "{file}: the half-size ball is {ball:?}");

    // The hierarchy, and nothing but the meshes: the camera and the light
    // are left out, and said to be.
    let plinth = part(document, "Plinth");
    assert!(plinth.parent.is_none(), "{file}: one top node needs no group");
    assert_eq!(part(document, "Column").parent, Some(plinth.id));
    assert_eq!(part(document, "Ball").parent, Some(part(document, "Column").id));
    assert_eq!(document.tree.len(), 4, "{file}: {:?}", document.tree.iter().map(|n| &n.name).collect::<Vec<_>>());
    assert!(report.warnings.iter().any(|w| w.contains("1 camera") && w.contains("1 light")), "{file}: {:?}", report.warnings);

    // The ball placed twice is one mesh.
    assert_eq!(document.geometry.len(), 3, "{file}");
    assert_eq!(part(document, "Ball").kind, part(document, "Ball copy").kind);

    // Materials, by name, the checker with its picture.
    let red = material_of(document, "Plinth");
    assert_eq!(red.name, "Red plastic");
    let base = red.material().base_color.to_linear_srgb();
    assert!(near(base, vec3(0.8, 0.05, 0.03), 2e-3), "{file}: {base:?}");
    let checker = material_of(document, "Column");
    assert_eq!(checker.name, "Checker");
    assert!(has_picture(checker), "{file}: the checker has no picture");
    assert_eq!(report.textures, 1, "{file}");
    let (_, source) = document.textures.iter().next().unwrap();
    assert!(source.path.exists(), "{file}: {}", source.path.display());
    assert_eq!(material_of(document, "Ball").name, "Steel");
}

#[test]
fn a_binary_fbx_lands_in_millimetres_with_blenders_materials_and_its_packed_picture() {
    let (path, dir) = fbx_copy("primitives.fbx", "fbx-binary");
    let (document, report) = open(&path, where_it_was());
    assert_eq!(report.source_unit, LengthUnit::Millimetre);
    assert!(!report.unit_was_guessed);
    check_fbx(&document, &report, "primitives.fbx");

    // Blender's export carries its Principled BSDF recoverably: the
    // roughness it had, and the steel a metal.
    let red = material_of(&document, "Plinth").material();
    assert!((red.specular_roughness - 0.35).abs() < 1e-3, "{}", red.specular_roughness);
    let steel = material_of(&document, "Ball");
    assert_eq!(steel.material_type, tequila_materials::material_type::MaterialType::Metal);
    assert!((steel.material().base_metalness - 1.0).abs() < 1e-4);
    // The packed picture, written out beside the model.
    let (_, source) = document.textures.iter().next().unwrap();
    assert!(source.path.starts_with(dir.join("primitives.textures")), "{}", source.path.display());
    assert_eq!(source.path.file_name().unwrap(), "checker.png");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_ascii_fbx_z_up_in_centimetres_lands_the_same_with_phong_mapped_by_the_formula() {
    let (path, dir) = fbx_copy("primitives_ascii.fbx", "fbx-ascii");
    let (document, report) = open(&path, where_it_was());
    assert_eq!(report.source_unit, LengthUnit::Centimetre);
    check_fbx(&document, &report, "primitives_ascii.fbx");

    // A plain Phong: roughness = sqrt(2 / (exponent + 2)), where Blender
    // wrote the exponent as ((1 - 0.35) * 10)^2.
    let exponent = ((1.0f32 - 0.35) * 10.0).powi(2);
    let expected = (2.0 / (exponent + 2.0)).sqrt();
    let red = material_of(&document, "Plinth").material();
    assert!((red.specular_roughness - expected).abs() < 1e-3, "{} against {expected}", red.specular_roughness);
    // Named by its relative path, found beside the model.
    let (_, source) = document.textures.iter().next().unwrap();
    assert_eq!(source.path, dir.join("checker.png"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An FBX whose relative path leads nowhere still finds its picture at the
/// absolute path it names, and failing both, beside the model by name.
#[test]
fn an_fbx_picture_is_looked_for_relative_then_absolute_then_beside() {
    let dir = folder("fbx-paths");
    let elsewhere = dir.join("library");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::copy(fixture("checker.png"), elsewhere.join("checker.png")).unwrap();
    let text = std::fs::read_to_string(fixture("primitives_ascii.fbx")).unwrap();
    let absolute = elsewhere.join("checker.png").display().to_string();
    let text = text
        .replace(r#"RelativeFilename: "checker.png""#, r#"RelativeFilename: "maps\checker.png""#)
        .replace(r"D:\tequila-fbx-fixtures\checker.png", &absolute);
    let path = dir.join("moved.fbx");
    std::fs::write(&path, text).unwrap();
    let (document, _) = open(&path, where_it_was());
    let (_, source) = document.textures.iter().next().unwrap();
    assert_eq!(source.path, elsewhere.join("checker.png"));

    // Neither: the same name beside the model.
    std::fs::remove_file(elsewhere.join("checker.png")).unwrap();
    std::fs::copy(fixture("checker.png"), dir.join("checker.png")).unwrap();
    let (document, _) = open(&path, where_it_was());
    let (_, source) = document.textures.iter().next().unwrap();
    assert_eq!(source.path, dir.join("checker.png"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Set on the ground, the FBX's lowest point, the plinth's underside, is at
/// height zero, as it was in Blender.
#[test]
fn an_fbx_set_on_the_ground_stands_as_it_did() {
    let (path, dir) = fbx_copy("primitives.fbx", "fbx-ground");
    let (document, report) = open(&path, ImportOptions::default());
    assert!(report.lifted.abs() < 1e-6, "lifted {}", report.lifted);
    let (low, _) = world_box(&document, part(&document, "Plinth"));
    assert!(low.y.abs() < 1e-6);
    let _ = std::fs::remove_dir_all(&dir);
}
