//! Blender files: the scene Blender shows, its materials node for node, its
//! cameras.
//!
//! Read from the answer Blender gave for `data/make_blend_fixtures.py`'s
//! scene, kept in `data/blend`, so these run without Blender; and, where
//! Blender is installed, from the scene that script makes, read by Blender
//! itself (`blender_itself_reads_the_same`).

use std::path::{Path, PathBuf};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_import::raw::RawScene;
use tequila_materials::graph::{MaterialGraph, NodeKind, Socket, Value};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::nodes::ShaderNode;
use tequila_render_core::camera::Projection;
use tequila_scene::material::MaterialDefinition;
use tequila_texture::cycles::ops::{MappingKind, MathOp};
use tequila_texture::texture::MapRole;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/blend")
}

fn fixture() -> RawScene {
    tequila_import::blend::read_answer(&fixtures(), "Fixture").unwrap_or_else(|e| panic!("{e}"))
}

fn node<'a>(scene: &'a RawScene, name: &str) -> (usize, &'a tequila_import::raw::RawNode) {
    let found: Vec<_> = scene.nodes.iter().enumerate().filter(|(_, n)| n.name == name).collect();
    assert!(!found.is_empty(), "no node {name} in {:?}", scene.nodes.iter().map(|n| &n.name).collect::<Vec<_>>());
    found[0]
}

fn parent_name(scene: &RawScene, name: &str) -> String {
    let (_, n) = node(scene, name);
    scene.nodes[n.parent.expect("a parent")].name.clone()
}

fn material<'a>(scene: &'a RawScene, name: &str) -> &'a MaterialDefinition {
    scene.materials.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no material {name}"))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn same_colour(a: Spectrum, b: Spectrum) -> bool {
    close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b)
}

fn kinds(graph: &MaterialGraph) -> Vec<&NodeKind> {
    graph.nodes().map(|(_, n)| &n.kind).collect()
}

/// What drives `param` on the surface, if anything.
fn driver(graph: &MaterialGraph, param: Param) -> Option<&NodeKind> {
    let to = graph.output();
    let link = graph.links().iter().find(|l| l.to == to && l.to_socket == Socket::Param(param))?;
    graph.node(link.from).map(|n| &n.kind)
}

#[test]
fn the_scene_comes_over_as_blender_shows_it() {
    let scene = fixture();
    assert_eq!(scene.source_unit, Some(LengthUnit::Metre));
    assert_eq!(scene.roots, vec![0]);
    assert_eq!(scene.nodes[0].name, "Fixture");
    // Blender's Z up is Tequila's Y up, on the model's own node.
    let up = scene.nodes[0].transform.matrix.transform_vector(Vec3::Z);
    assert!((up - Vec3::Y).length() < 1e-6, "{up:?}");

    // Collections are groups; an empty one is left out.
    for (child, parent) in [("Product", "Fixture"), ("Studio", "Fixture"), ("Body", "Product"), ("Floor", "Studio"), ("Knob", "Body")] {
        assert_eq!(parent_name(&scene, child), parent, "{child}");
    }
    assert!(scene.nodes.iter().all(|n| n.name != "Empty"), "the empty collection came over");

    // Hidden where Blender's render leaves it out: switched off, or in an
    // excluded collection.
    for name in ["Hidden", "Spare", "Kit"] {
        assert!(node(&scene, name).1.hidden, "{name} is shown");
    }
    for name in ["Body", "Knob", "Product", "Dome"] {
        assert!(!node(&scene, name).1.hidden, "{name} is hidden");
    }
    assert!(node(&scene, "Spare part").1.hidden || node(&scene, "Spare").1.hidden);

    // Two materials on one mesh: a part for each, under the object.
    let (body, _) = node(&scene, "Body");
    let parts: Vec<&str> = scene.nodes.iter().filter(|n| n.parent == Some(body) && n.name.starts_with("Body")).map(|n| n.name.as_str()).collect();
    assert_eq!(parts, ["Body [Label]", "Body [Paint]"]);
    let top = scene.nodes.iter().find(|n| n.name == "Body [Label]").and_then(|n| n.mesh).unwrap();
    assert_eq!(scene.meshes[top].indices.len() / 3, 2, "the label part is the top face");

    // Modifiers applied: the knob's bevel rounded its edges.
    let knob = node(&scene, "Knob").1.mesh.unwrap();
    assert!(scene.meshes[knob].indices.len() / 3 > 124, "the bevel was not applied: {} triangles", scene.meshes[knob].indices.len() / 3);

    // A collection instanced twice: a screw under each holder, one mesh.
    let screws: Vec<usize> = ["Screw 1", "Screw 2"]
        .iter()
        .map(|holder| {
            let (h, _) = node(&scene, holder);
            let under: Vec<_> = scene.nodes.iter().filter(|n| n.parent == Some(h)).collect();
            assert_eq!(under.len(), 1, "{holder}");
            assert_eq!(under[0].name, "Screw");
            under[0].mesh.unwrap()
        })
        .collect();
    assert_eq!(screws[0], screws[1], "the instances do not share their mesh");

    // Text and a curve, as meshes, once each.
    for name in ["Words", "Cable"] {
        let (i, n) = node(&scene, name);
        assert!(n.mesh.is_some(), "{name} has no mesh");
        assert!(scene.nodes.iter().all(|c| c.parent != Some(i)), "{name} came over twice");
    }

    // Each part keeps the object it came from.
    assert_eq!(node(&scene, "Knob").1.metadata.get("Blender object").map(String::as_str), Some("Knob"));
    assert!(scene.warnings.iter().any(|w| w.contains("1 light left out")), "{:?}", scene.warnings);
    // Blender's v runs up the picture; Tequila's down.
    let plate = &scene.meshes[node(&scene, "Plate").1.mesh.unwrap()];
    assert_eq!(plate.uvs.len(), plate.positions.len());
}

#[test]
fn cameras_come_over_with_their_lenses() {
    let scene = fixture();
    let setup = scene.setup.as_ref().expect("the scene's cameras");
    let names: Vec<&str> = setup.cameras.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Hero", "Top"]);
    assert_eq!(setup.active_camera, Some(0));
    let hero = &setup.cameras[0];
    assert_eq!(hero.camera.projection, Projection::Perspective { focal_length_mm: 85.0 });
    assert!(close(hero.camera.f_number, 4.0) && close(hero.camera.focus_distance, 0.5));
    // At Blender's (0.3, -0.3, 0.25), Z up.
    assert!((hero.camera.position() - vec3(0.3, 0.25, 0.3)).length() < 1e-4, "{:?}", hero.camera.position());
    assert!(((hero.target - hero.camera.position()).length() - 0.5).abs() < 1e-4);
    // Blender's orthographic scale is across the wider side, 1200 by 800;
    // Tequila's is up it.
    let top = &setup.cameras[1];
    match top.camera.projection {
        Projection::Orthographic { scale } => assert!(close(scale, 0.2), "{scale}"),
        other => panic!("{other:?}"),
    }
    assert!((top.camera.forward() - vec3(0.0, -1.0, 0.0)).length() < 1e-5, "the top camera looks {:?}", top.camera.forward());
    let render = setup.render.as_ref().unwrap();
    assert_eq!((render.width, render.height), (1200, 800));
    assert_eq!(setup.image_style.unwrap().display, tequila_color::DisplayTransform::Srgb);
}

#[test]
fn principled_maps_onto_openpbr() {
    let scene = fixture();
    let paint = material(&scene, "Paint");
    let m = paint.graph.material();
    assert!(same_colour(m.base_color, Spectrum::from_linear_srgb(vec3(0.8, 0.1, 0.05))));
    assert!(close(m.specular_roughness, 0.35) && close(m.specular_ior, 1.45));
    // Blender's specular level of 0.25 is half its IOR's reflection.
    assert!(close(m.specular_weight, 0.5), "{}", m.specular_weight);
    assert!(close(m.coat_weight, 0.6) && close(m.coat_roughness, 0.1));
    assert_eq!(m.base_metalness, 0.0);

    let brushed = material(&scene, "Brushed");
    let m = brushed.graph.material();
    assert_eq!(m.base_metalness, 1.0);
    assert!(close(m.specular_roughness_anisotropy, 0.7));
    // A quarter of Blender's whole turn is half of Tequila's half turn.
    assert!(close(m.specular_rotation, 0.5), "{}", m.specular_rotation);
    // Radial about Z, from Generated coordinates.
    assert!(matches!(driver(&brushed.graph, Param::Tangent), Some(NodeKind::Node(ShaderNode::CombineXyz))));

    let glass = material(&scene, "Glass");
    assert_eq!(glass.material_type, MaterialType::Glass);
    assert_eq!(glass.graph.material().transmission_weight, 1.0);
    assert!(same_colour(glass.graph.material().transmission_color, Spectrum::from_linear_srgb(vec3(0.8, 0.9, 1.0))));
}

#[test]
fn a_label_is_projected_clipped_and_kept_to_the_top() {
    let scene = fixture();
    let label = material(&scene, "Label");
    let graph = &label.graph;
    // The picture, packed in the .blend, comes out with it: its alpha is
    // the ink, and its colour, which nothing uses, is not read.
    let texture = graph
        .nodes()
        .find_map(|(_, n)| match n.kind {
            NodeKind::ImageAlpha { texture, .. } => Some(texture),
            _ => None,
        })
        .expect("the label's picture");
    assert!(!kinds(graph).iter().any(|k| matches!(k, NodeKind::ImageTexture { .. })));
    let raw = &scene.textures[texture.0 as usize];
    assert_eq!(raw.role, MapRole::Color);
    assert!(raw.embedded.as_ref().is_some_and(|b| b.starts_with(b"\x89PNG")), "{}", raw.uri);
    // Projected from the object's own space through Blender's Mapping.
    let mapping = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Node(ShaderNode::Mapping { kind: MappingKind::Point })) && n.literals.get(&Socket::Input(3)) == Some(&Value::Vector(vec3(2.0, 4.0, 1.0))));
    assert!(mapping.is_some(), "the label's mapping");
    // Clipped: tested inside the picture on both axes.
    let tests = kinds(graph).iter().filter(|k| matches!(k, NodeKind::Node(ShaderNode::Math { op: MathOp::LessThan, .. }))).count();
    assert_eq!(tests, 2);
    // The face's normal, which Blender measures with Z up, turned to it.
    let turned = graph.nodes().any(|(_, n)| {
        matches!(n.kind, NodeKind::Node(ShaderNode::Mapping { kind: MappingKind::Vector })) && n.literals.get(&Socket::Input(2)) == Some(&Value::Vector(vec3(std::f32::consts::FRAC_PI_2, 0.0, 0.0)))
    });
    assert!(turned, "the geometry normal was not turned into Blender's axes");
    assert!(matches!(driver(graph, Param::BaseColor), Some(NodeKind::Node(ShaderNode::Mix { .. }))));
}

#[test]
fn groups_mutes_and_reroutes_are_seen_through() {
    let scene = fixture();
    let graph = &material(&scene, "Speckle").graph;
    assert!(!kinds(graph).iter().any(|k| matches!(k, NodeKind::Group(_) | NodeKind::Reroute { .. })));
    // The group's Voronoi at the scale given to the group.
    let voronoi = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Node(ShaderNode::Voronoi { .. }))).expect("the group's Voronoi");
    assert_eq!(voronoi.1.literals.get(&Socket::Input(2)), Some(&Value::Float(40.0)));
    // The muted Invert passes its colour on: the ramp reads the group's
    // multiply straight.
    let ramp = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Gradient { .. })).map(|(id, _)| id).unwrap();
    let into_ramp = graph.links().iter().find(|l| l.to == ramp).map(|l| &graph.node(l.from).unwrap().kind);
    assert!(matches!(into_ramp, Some(NodeKind::Node(ShaderNode::Math { op: MathOp::Multiply, .. }))), "{into_ramp:?}");
    assert!(!kinds(graph).iter().any(|k| matches!(k, NodeKind::Node(ShaderNode::InvertColor))));
    // The curves: a ramp for each channel, the combined curve in each.
    let ramps = kinds(graph).iter().filter(|k| matches!(k, NodeKind::Gradient { .. })).count();
    assert_eq!(ramps, 4);
    assert!(matches!(driver(graph, Param::BaseColor), Some(NodeKind::Node(ShaderNode::CombineColor { .. }))));
    // Bumped by the noise, Strength times Distance deep.
    assert!(matches!(driver(graph, Param::Bump), Some(NodeKind::Bump { .. })));
    let bump = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::Bump { .. })).map(|(id, _)| id).unwrap();
    let height = graph.links().iter().find(|l| l.to == bump).map(|l| l.from).unwrap();
    let depth = graph.node(height).and_then(|n| n.literals.get(&Socket::Input(1))).map(|v| v.as_float());
    assert!(depth.is_some_and(|d| (d - 0.001).abs() < 1e-7), "{depth:?}");
}

#[test]
fn shaders_mix_cut_out_and_glow() {
    let scene = fixture();
    let mixed = material(&scene, "Mixed");
    let graph = &mixed.graph;
    // The glossy over the diffuse, three tenths of the way.
    let mix = graph.nodes().find(|(_, n)| matches!(n.kind, NodeKind::MixShader { add: false })).expect("a Mix Shader");
    assert_eq!(mix.1.literals.get(&Socket::Input(0)), Some(&Value::Float(0.3)));
    let shader = graph.nodes().find_map(|(id, n)| match &n.kind {
        NodeKind::Shader(m) => Some((id, m)),
        _ => None,
    });
    let (shader, glossy) = shader.expect("the glossy");
    assert_eq!(glossy.base_metalness, 1.0);
    assert_eq!(mixed.graph.material().specular_weight, 0.0, "the diffuse reflects");
    // Cut out by the checker: opacity on both.
    assert!(driver(graph, Param::Opacity).is_some(), "the surface is not cut out");
    assert!(graph.links().iter().any(|l| l.to == shader && l.to_socket == Socket::Param(Param::Opacity)), "the glossy is not cut out");
    // And glowing: the Emission added is on both.
    let glow = Spectrum::from_linear_srgb(vec3(1.0, 0.5, 0.1));
    for (m, what) in [(graph.material(), "diffuse"), (glossy.as_ref(), "glossy")] {
        assert!(same_colour(m.emission_color, glow) && close(m.emission_luminance, 2.0), "the {what} does not glow");
    }
}

#[test]
fn a_normal_map_is_read_as_one() {
    let scene = fixture();
    let graph = &material(&scene, "Dimpled").graph;
    let Some(NodeKind::ImageTexture { texture, .. }) = driver(graph, Param::Bump) else { panic!("no normal map in Bump") };
    assert_eq!(scene.textures[texture.0 as usize].role, MapRole::Normal);
    assert_eq!(graph.len(), 3, "{:?}", kinds(graph));
}

#[test]
fn every_graph_compiles_with_nothing_left_over() {
    let scene = fixture();
    for m in &scene.materials {
        m.graph.compile().unwrap_or_else(|e| panic!("{}: {e:?}", m.name));
        // Every node feeds something: nothing made and left unwired.
        for (id, _) in m.graph.nodes() {
            if id != m.graph.output() {
                assert!(m.graph.links().iter().any(|l| l.from == id), "{}: {:?} feeds nothing", m.name, m.graph.node(id).unwrap().kind.label());
            }
        }
    }
}

#[test]
fn a_blender_file_builds_into_a_document() {
    let folder = std::env::temp_dir().join(format!("tequila-blend-document-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let mut document = tequila_scene::SceneDocument::new();
    let origin = tequila_import::Origin { directory: &folder, stem: "fixture" };
    let (command, report) = tequila_import::build(&document, None, fixture(), &Default::default(), Some(origin)).unwrap();
    tequila_scene::CommandBus::new().apply(&mut document, command).unwrap();
    document.validate().unwrap();
    assert!(document.tree.iter().any(|n| n.name == "Body [Label]"));
    // The packed pictures, written out beside the model.
    assert!(folder.join("fixture.textures").join("000-label.png").is_file(), "{:?}", report.warnings);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_damaged_answer_is_an_error_not_a_crash() {
    let scene = std::fs::read(fixtures().join("scene.json")).unwrap();
    let bin = std::fs::read(fixtures().join("scene.bin")).unwrap();
    let folder = std::env::temp_dir().join(format!("tequila-blend-damaged-{}", std::process::id()));
    let cases: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
        ("arrays cut short", scene.clone(), bin[..bin.len() / 2].to_vec()),
        ("no arrays", scene.clone(), Vec::new()),
        ("not an answer", b"{\"format\": \"something else\"}".to_vec(), bin.clone()),
        ("cut short", scene[..scene.len() / 3].to_vec(), bin.clone()),
        ("from a newer Tequila", String::from_utf8_lossy(&scene).replacen("\"version\": 1", "\"version\": 99", 1).into_bytes(), bin.clone()),
        ("no meshes", String::from_utf8_lossy(&scene).replacen("\"meshes\": [", "\"meshes\": [], \"was\": [", 1).into_bytes(), bin.clone()),
    ];
    for (what, json, arrays) in cases {
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("scene.json"), json).unwrap();
        std::fs::write(folder.join("scene.bin"), arrays).unwrap();
        assert!(tequila_import::blend::read_answer(&folder, "Damaged").is_err(), "{what} was read");
    }
    let _ = std::fs::remove_dir_all(&folder);
}

/// The fixture's scene, made and read by the Blender on this computer:
/// read the same as the answer kept for it. Says so and passes where there
/// is no Blender, as on the build machines.
#[test]
fn blender_itself_reads_the_same() {
    let Some(blender) = tequila_import::blend::blender() else {
        eprintln!("no Blender here; the kept answer stands in for it");
        return;
    };
    let folder = std::env::temp_dir().join(format!("tequila-blend-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("fixture.blend");
    let made = std::process::Command::new(&blender)
        .args(["--background", "--factory-startup", "--python"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/make_blend_fixtures.py"))
        .arg("--")
        .arg(&file)
        .output()
        .unwrap();
    assert!(file.is_file(), "Blender did not make the scene: {}", String::from_utf8_lossy(&made.stderr));
    let live = tequila_import::blend::read_with(&blender, &file, "Fixture").unwrap_or_else(|e| panic!("{e}"));
    let kept = fixture();
    let names = |s: &RawScene| s.nodes.iter().map(|n| (n.name.clone(), n.parent, n.hidden)).collect::<Vec<_>>();
    assert_eq!(names(&live), names(&kept));
    let counts = |s: &RawScene| s.meshes.iter().map(|m| m.indices.len()).collect::<Vec<_>>();
    assert_eq!(counts(&live), counts(&kept));
    let graphs = |s: &RawScene| s.materials.iter().map(|m| (m.name.clone(), m.graph.len(), m.graph.links().len())).collect::<Vec<_>>();
    assert_eq!(graphs(&live), graphs(&kept));
    // And a file Blender cannot read is said so, not read as Blender's
    // start-up scene.
    let junk = folder.join("junk.blend");
    std::fs::write(&junk, b"not a blend file").unwrap();
    let error = tequila_import::blend::read_with(&blender, &junk, "Junk").unwrap_err().to_string();
    assert!(error.contains("Blender could not read it"), "{error}");
    let _ = std::fs::remove_dir_all(&folder);
}
