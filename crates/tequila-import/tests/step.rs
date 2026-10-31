//! STEP import, on fixtures written by `tests/data/make_step_fixtures.py`:
//! hand-built B-reps, so every expectation below is exact geometry.

use tequila_core::math::{Vec3, vec3};
use tequila_core::units::LengthUnit;
use tequila_import::raw::{RawMesh, RawScene};
use tequila_import::{build, read_format, ImportError, ImportOptions};
use tequila_scene::{CommandBus, SceneDocument};

const TWO_BODIES: &[u8] = include_bytes!("data/two_bodies.step");
const INCHES: &[u8] = include_bytes!("data/inches.step");
const ASSEMBLY: &[u8] = include_bytes!("data/assembly.step");

fn read(bytes: &[u8]) -> RawScene {
    let scene = read_format(bytes, "step", "file", &|_| None).expect("the fixture reads");
    scene.validate().expect("the reader's output holds together");
    scene
}

fn mesh<'a>(scene: &'a RawScene, name: &str) -> &'a RawMesh {
    let node = scene
        .nodes
        .iter()
        .find(|n| n.name == name && n.mesh.is_some())
        .unwrap_or_else(|| panic!("no part named `{name}` in {:?}", scene.nodes.iter().map(|n| &n.name).collect::<Vec<_>>()));
    &scene.meshes[node.mesh.unwrap()]
}

fn triangles(mesh: &RawMesh) -> impl Iterator<Item = [Vec3; 3]> + '_ {
    mesh.indices
        .chunks(3)
        .map(|t| [mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]])
}

fn area(mesh: &RawMesh) -> f32 {
    triangles(mesh).map(|[a, b, c]| (b - a).cross(c - a).length() * 0.5).sum()
}

/// Every triangle wound so its face points away from `inside`, and its
/// vertex normals agree.
fn assert_outward(mesh: &RawMesh, inside: impl Fn(Vec3) -> Vec3, what: &str) {
    assert_eq!(mesh.normals.len(), mesh.positions.len(), "{what} has a normal per vertex");
    for (t, [a, b, c]) in mesh.indices.chunks(3).zip(triangles(mesh)) {
        let facing = (b - a).cross(c - a);
        let centre = (a + b + c) / 3.0;
        assert!(facing.dot(centre - inside(centre)) > 0.0, "{what}: a triangle at {centre:?} faces inward");
        for &i in t {
            assert!(mesh.normals[i as usize].dot(facing) > 0.0, "{what}: a vertex normal at {centre:?} disagrees with its triangle");
        }
    }
}

/// The two solids arrive as parts under the product, named as the file
/// names them, in the file's millimetres.
#[test]
fn bodies_are_named_parts_under_the_product() {
    let scene = read(TWO_BODIES);
    assert_eq!(scene.source_unit, Some(LengthUnit::Millimetre));
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);

    assert_eq!(scene.roots.len(), 1);
    let root = &scene.nodes[scene.roots[0]];
    assert_eq!(root.name, "Fixture");
    assert!(root.mesh.is_none());
    let children: Vec<&str> =
        scene.nodes.iter().filter(|n| n.parent == Some(scene.roots[0])).map(|n| n.name.as_str()).collect();
    assert_eq!(children, ["Box", "Pin"]);
}

/// The pin's circles and cylinder are placed along X with the reference
/// direction left out. truck-stepio 0.3 filled that in with +X whatever the
/// axis, so for an X axis it normalised a zero vector: every such circle
/// came out NaN, and meshing panicked. A real CAD file lost every hole and
/// shaft along X this way. The standard's default is +Y for an X axis.
#[test]
fn a_cylinder_along_x_without_a_reference_direction_meshes() {
    let scene = read(TWO_BODIES);
    let pin = mesh(&scene, "Pin");
    assert!(pin.positions.iter().all(|p| p.is_finite()), "the pin has non-finite vertices");

    let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for &p in &pin.positions {
        low = low.min(p);
        high = high.max(p);
        let radius = vec3(0.0, p.y - 50.0, p.z).length();
        assert!(radius <= 5.0 + 1e-3, "a vertex at {p:?} is {radius} from the axis");
    }
    // The polygon inscribed in each circle need not touch its extremes, but
    // comes within the chord tolerance, a thousandth of the pin's size.
    assert!((low - vec3(0.0, 45.0, -5.0)).abs().max_component() < 0.05, "starts at {low:?}");
    assert!((high - vec3(40.0, 55.0, 5.0)).abs().max_component() < 0.05, "ends at {high:?}");
    assert!(low.x.abs() < 1e-4 && (high.x - 40.0).abs() < 1e-4, "runs from x = {} to {}", low.x, high.x);

    // Side and two caps; the polygon inscribed in each circle is a little
    // short of it.
    let exact = std::f32::consts::TAU * 5.0 * 40.0 + 2.0 * std::f32::consts::PI * 25.0;
    let meshed = area(pin);
    assert!(meshed < exact && meshed > exact * 0.97, "the pin's area is {meshed}, not about {exact}");
    assert_outward(pin, |p| vec3(p.x.clamp(1.0, 39.0), 50.0, 0.0), "the pin");
}

/// Colours come from the body, and a face's own colour overrides it: the
/// box is red with a green top, as one part per colour, and the pin is the
/// pre-defined blue.
#[test]
fn colours_come_from_the_body_and_a_face_overrides_it() {
    let scene = read(TWO_BODIES);
    let colour = |mesh: &RawMesh| {
        let definition = &scene.materials[mesh.material.expect("coloured parts have a material")];
        (definition.name.clone(), definition.material().base_color.to_linear_srgb())
    };
    let near = |a: Vec3, b: Vec3| (a - b).abs().max_component() < 1e-3;

    let red = mesh(&scene, "Box (Colour #FF0000)");
    let green = mesh(&scene, "Box (Colour #00FF00)");
    let blue = mesh(&scene, "Pin");
    assert!(near(colour(red).1, vec3(1.0, 0.0, 0.0)), "{:?}", colour(red));
    assert!(near(colour(green).1, vec3(0.0, 1.0, 0.0)), "{:?}", colour(green));
    assert_eq!(colour(blue).0, "blue");
    assert!(near(colour(blue).1, vec3(0.0, 0.0, 1.0)), "{:?}", colour(blue));

    // The green part is the top face alone, and the red one the other five.
    assert!(green.positions.iter().all(|p| (p.z - 30.0).abs() < 1e-4));
    assert!((area(green) - 200.0).abs() < 1e-2, "top area {}", area(green));
    assert!((area(red) - 2000.0).abs() < 1e-1, "other faces' area {}", area(red));
    // Both halves of the box belong to a group named for it.
    let group = scene.nodes.iter().position(|n| n.name == "Box" && n.mesh.is_none()).expect("a Box group");
    assert_eq!(scene.nodes.iter().filter(|n| n.parent == Some(group)).count(), 2);

    for part in [red, green] {
        assert_outward(part, |_| vec3(5.0, 10.0, 15.0), "the box");
    }
}

/// The unit is the one the geometry context assigns. This file defines
/// millimetres too, as the base its inch is 25.4 of, and taking the first
/// length unit in the file would read it as millimetres.
#[test]
fn the_unit_is_the_one_the_context_assigns() {
    let scene = read(INCHES);
    assert_eq!(scene.source_unit, Some(LengthUnit::Inch));

    // A single body is the product, and takes its name.
    assert_eq!(scene.roots.len(), 1);
    let root = &scene.nodes[scene.roots[0]];
    assert_eq!(root.name, "Inch box");
    assert!(root.mesh.is_some());

    // And it reaches the document at its real size, in the metres a new
    // document is in: 1 x 2 x 3 inches.
    let mut document = SceneDocument::new();
    let (command, report) = build(&document, None, scene, &ImportOptions::default(), None).unwrap();
    assert!(!report.unit_was_guessed);
    CommandBus::new().apply(&mut document, command).unwrap();
    let bounds = document.bounds();
    let size = bounds.max - bounds.min;
    assert!((size - vec3(0.0254, 0.0508, 0.0762)).abs().max_component() < 1e-6, "the box measures {size:?}");
}

/// A document's unit is how its lengths read, not what they are stored in:
/// the inch box in a millimetre scene is the same 0.0254 x 0.0508 x 0.0762
/// metres, which that scene shows as 25.40 x 50.80 x 76.20 mm.
#[test]
fn a_stated_unit_converts_into_metres_whatever_the_document_shows() {
    let mut document = SceneDocument::new();
    document.units = tequila_scene::units::SceneUnits::new(LengthUnit::Millimetre);
    let (command, _) = build(&document, None, read(INCHES), &ImportOptions::default(), None).unwrap();
    CommandBus::new().apply(&mut document, command).unwrap();
    let bounds = document.bounds();
    let size = bounds.max - bounds.min;
    assert!((size - vec3(0.0254, 0.0508, 0.0762)).abs().max_component() < 1e-6, "the box measures {size:?}");
    let shown = tequila_scene::measure::format_length(size.z, &document.units);
    assert_eq!(shown, "76.20 mm");
}

/// An assembly places its components: each instance lands where the
/// assembly puts it, through nested sub-assemblies, and a part used twice
/// is meshed once and shared. The pin's placement is written parent-first,
/// which the standard allows, so its transformation has to be inverted.
#[test]
fn an_assembly_places_its_components() {
    let scene = read(ASSEMBLY);
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
    assert_eq!(scene.roots.len(), 1);
    assert_eq!(scene.nodes[scene.roots[0]].name, "Assembly");

    let world = |mut node: usize| {
        let mut transform = scene.nodes[node].transform;
        while let Some(parent) = scene.nodes[node].parent {
            transform = scene.nodes[parent].transform * transform;
            node = parent;
        }
        transform
    };
    let bounds = |node: usize| {
        let transform = world(node);
        let mesh = &scene.meshes[scene.nodes[node].mesh.expect("a part")];
        mesh.positions.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), &p| {
            let p = transform.point(p);
            (lo.min(p), hi.max(p))
        })
    };
    let parts = |name: &str| -> Vec<usize> {
        (0..scene.nodes.len()).filter(|&i| scene.nodes[i].name == name && scene.nodes[i].mesh.is_some()).collect()
    };
    let near = |(lo, hi): (Vec3, Vec3), want_lo: Vec3, want_hi: Vec3, slack: f32| {
        (lo - want_lo).abs().max_component() < slack && (hi - want_hi).abs().max_component() < slack
    };

    let boxes = parts("Box part");
    assert_eq!(boxes.len(), 2, "two instances of the box");
    assert_eq!(scene.nodes[boxes[0]].mesh, scene.nodes[boxes[1]].mesh, "sharing one mesh");
    assert_eq!(scene.meshes.len(), 2, "the box and the pin, each meshed once");
    let placed: Vec<_> = boxes.iter().map(|&b| bounds(b)).collect();
    // One moved to x = 100; one moved to y = 100 and turned so its 10 mm
    // side runs along y and its 20 mm side along -x.
    assert!(
        placed.iter().any(|&b| near(b, vec3(100.0, 0.0, 0.0), vec3(110.0, 20.0, 30.0), 1e-3)),
        "no box at x = 100: {placed:?}"
    );
    assert!(
        placed.iter().any(|&b| near(b, vec3(-20.0, 100.0, 0.0), vec3(0.0, 110.0, 30.0), 1e-3)),
        "no turned box at y = 100: {placed:?}"
    );

    // The pin: 5 mm along x inside Sub, and Sub 50 mm up.
    let pin = parts("Pin part");
    assert_eq!(pin.len(), 1);
    let sub = scene.nodes[pin[0]].parent.expect("the pin is in Sub");
    assert_eq!(scene.nodes[sub].name, "Sub");
    let placed = bounds(pin[0]);
    assert!(near(placed, vec3(5.0, -5.0, 45.0), vec3(45.0, 5.0, 55.0), 0.05), "the pin is at {placed:?}");

    // A colour on a part's body is still its colour in the assembly.
    let red = scene.materials[scene.meshes[scene.nodes[boxes[0]].mesh.unwrap()].material.expect("coloured")].material();
    assert!((red.base_color.to_linear_srgb() - vec3(1.0, 0.0, 0.0)).abs().max_component() < 1e-3);
}

/// Names are written with STEP's escapes for anything outside ASCII.
#[test]
fn escaped_names_are_decoded() {
    let text = String::from_utf8(TWO_BODIES.to_vec()).unwrap();
    let escaped = text.replace("PRODUCT('Fixture','Fixture'", r"PRODUCT('Caf\X2\00E9\X0\','Caf\X2\00E9\X0\ \X\E0 la carte'");
    assert_ne!(escaped, text);
    let scene = read(escaped.as_bytes());
    assert_eq!(scene.nodes[scene.roots[0]].name, "Café à la carte");
}

/// Part 21 writes an apostrophe as a doubled quote, and allows an empty
/// list. ruststep's parser, which this reader used first, rejected both,
/// failing the whole file over a part named "Sam's bracket".
#[test]
fn an_apostrophe_or_an_empty_list_does_not_fail_the_file() {
    let text = String::from_utf8(TWO_BODIES.to_vec()).unwrap();
    let quoted = text.replace("PRODUCT('Fixture','Fixture','',(#3))", "PRODUCT('Fixture','Sam''s bracket','',())");
    assert_ne!(quoted, text);
    let scene = read(quoted.as_bytes());
    assert_eq!(scene.nodes[scene.roots[0]].name, "Sam's bracket");
    assert_eq!(scene.meshes.len(), 3);
}

/// A file that is not STEP, or has no geometry, is refused with a reason
/// rather than a panic or an empty scene.
#[test]
fn a_file_with_nothing_to_mesh_is_refused() {
    let error = read_format(b"solid cube\nendsolid\n", "step", "x", &|_| None).unwrap_err();
    assert!(matches!(error, ImportError::Malformed(_)), "{error}");

    // In lines ending as a Unix checkout has them: a Windows checkout ends
    // them \r\n, and the end of the data section went unfound there.
    let text = String::from_utf8(TWO_BODIES.to_vec()).unwrap().replace("\r\n", "\n");
    let start = text.find("DATA;").unwrap() + "DATA;".len();
    let end = text.find("ENDSEC;\nEND-ISO").unwrap();
    let empty = format!("{}\n#1=APPLICATION_CONTEXT('automotive design');\n{}", &text[..start], &text[end..]);
    let error = read_format(empty.as_bytes(), "stp", "x", &|_| None).unwrap_err();
    assert!(matches!(error, ImportError::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("no solids"), "{error}");
}

/// The pin's edges are its B-rep's: the two circles where the caps meet the
/// side, and not the seam the cylinder's surface is closed along, which is
/// no edge of the part.
#[test]
fn a_parts_edges_are_its_b_rep_edges_without_seams() {
    let scene = read(TWO_BODIES);
    let pin = mesh(&scene, "Pin");
    assert!(pin.edge_lines.len() >= 2 * 16, "only {} edge points", pin.edge_lines.len());
    assert_eq!(pin.edge_lines.len() % 2, 0);
    for p in &pin.edge_lines {
        let radius = vec3(0.0, p.y - 50.0, p.z).length();
        assert!(p.x.abs() < 1e-3 || (p.x - 40.0).abs() < 1e-3, "an edge point at {p:?} is on neither end circle");
        assert!((radius - 5.0).abs() < 0.05, "an edge point at {p:?} is {radius} from the axis");
    }
    for segment in pin.edge_lines.chunks(2) {
        assert!((segment[0].x - segment[1].x).abs() < 1e-3, "a segment runs along the cylinder: the seam was drawn");
    }
}

/// Finer is more triangles, and the quality is recorded for re-dividing.
#[test]
fn a_finer_tessellation_divides_more_finely() {
    use tequila_import::step::{read_with, Tessellation};
    let coarse = read_with(TWO_BODIES, "file", Tessellation::Draft).unwrap();
    let fine = read_with(TWO_BODIES, "file", Tessellation::High).unwrap();
    let count = |s: &RawScene| mesh(s, "Pin").indices.len() / 3;
    assert!(count(&fine) > count(&coarse) * 2, "fine {} against coarse {}", count(&fine), count(&coarse));
    assert_eq!(fine.tessellation.as_deref(), Some("High"));
}

/// Dividing a model again at another quality, from its kept source, is an
/// update: the part keeps the material it was given, and takes the new
/// triangles.
#[test]
fn re_dividing_keeps_the_work_on_the_parts() {
    use tequila_import::step::Tessellation;
    use tequila_scene::tree::NodeKind;
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let model = tequila_import::read_cad(TWO_BODIES, "Fixture", None, Tessellation::Draft).unwrap();
    let model = tequila_import::ReadModel { brep: Some((tequila_scene::document::Sources::id_of(TWO_BODIES), std::sync::Arc::new(TWO_BODIES.to_vec()))), ..model };
    let (command, _) = tequila_import::build_read(&document, None, model, &ImportOptions::default()).unwrap();
    bus.apply(&mut document, command).unwrap();
    let root = document.tree.roots()[0];
    let metadata = &document.tree.get(root).unwrap().metadata;
    assert_eq!(metadata.get(tequila_import::step::TESSELLATION).map(String::as_str), Some("Draft"));
    assert!(metadata.contains_key(tequila_scene::document::BREP), "the source is named on the model");

    let pin = document.tree.iter().find(|n| n.name == "Pin").map(|n| n.id).unwrap();
    let gold = document.materials.insert(tequila_scene::material::MaterialDefinition::preset("Gold", tequila_materials::material_type::MaterialType::Metal));
    bus.apply(&mut document, tequila_scene::Command::AssignMaterial { nodes: vec![pin], material: gold }).unwrap();
    let triangles = |d: &SceneDocument| match d.tree.get(pin).unwrap().kind {
        NodeKind::Part { mesh } => d.geometry.get(mesh).unwrap().indices.len() / 3,
        _ => 0,
    };
    let before = triangles(&document);

    let finer = tequila_import::read_cad(TWO_BODIES, "Fixture", None, Tessellation::High).unwrap();
    let options = ImportOptions { place_on_ground: false, ..Default::default() };
    let (command, _) = tequila_import::build_read(&document, None, finer, &options).unwrap();
    let tequila_scene::Command::Import { payload } = command else { panic!("an import") };
    let (update, report) = tequila_scene::reimport::plan(&document, &[root], *payload);
    bus.apply(&mut document, update).unwrap();
    assert!(report.added.is_empty() && report.removed.is_empty(), "the same parts: {report:?}");
    assert_eq!(document.tree.get(pin).unwrap().material, Some(gold), "the pin lost its material");
    assert!(triangles(&document) > before * 2, "{} triangles, from {before}", triangles(&document));
    let metadata = &document.tree.get(root).unwrap().metadata;
    assert_eq!(metadata.get(tequila_import::step::TESSELLATION).map(String::as_str), Some("High"));
    assert!(metadata.contains_key(tequila_scene::document::BREP), "re-dividing forgot the source");
}

const LAYERS: &[u8] = include_bytes!("data/layers.step");

/// What a CAD package writes about a part besides its shape arrives as the
/// nodes' metadata: the product's part number, description and release and
/// a property of the user's own on the group; each body's layer, the
/// material on the first body; and a body on a layer that is switched off
/// arrives hidden. By OpenCASCADE where its helper is built, by truck
/// elsewhere; `step_fallback.rs` reads it with truck as well.
#[test]
fn layers_and_attributes_arrive_as_metadata() {
    let scene = read(LAYERS);
    let node = |name: &str| scene.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no node `{name}`"));
    let housing = node("Housing");
    assert_eq!(housing.metadata.get("Part number").map(String::as_str), Some("PN-1042"));
    assert_eq!(housing.metadata.get("Description").map(String::as_str), Some("Front housing"));
    assert_eq!(housing.metadata.get("Definition").map(String::as_str), Some("Production release"));
    assert_eq!(housing.metadata.get("Finish").map(String::as_str), Some("Mold-Tech 11010"));
    let (box_part, pin) = (node("Box"), node("Pin"));
    assert_eq!(box_part.layers, ["Outer"]);
    assert_eq!(box_part.metadata.get("Material").map(String::as_str), Some("ABS"));
    assert!(!box_part.hidden, "the box's layer is on");
    assert_eq!(pin.layers, ["Construction"]);
    assert!(pin.hidden, "the pin's layer is switched off");

    // And in the document: the pin hidden, the box shown, and the layer
    // there for a search or a rule to find.
    let mut document = SceneDocument::new();
    let (command, _) = build(&document, None, scene, &ImportOptions::default(), None).unwrap();
    let mut bus = CommandBus::new();
    bus.apply(&mut document, command).unwrap();
    let placed = |document: &SceneDocument, name: &str| {
        document.tree.iter().find(|n| n.name == name).cloned().unwrap_or_else(|| panic!("no `{name}` in the document"))
    };
    assert!(!placed(&document, "Pin").visible && placed(&document, "Box").visible);
    assert_eq!(placed(&document, "Box").layers, ["Outer"]);
    let query = tequila_scene::metadata::Query::parse("Layer = Outer").unwrap();
    assert_eq!(tequila_scene::metadata::find(&document, &query), [placed(&document, "Box").id]);

    // The file's layers are the document's, the switched-off one hidden,
    // and showing it shows its part.
    let layers = tequila_scene::layers::layers(&document);
    let shown: Vec<(&str, tequila_scene::layers::Shown)> = layers.iter().map(|l| (l.name.as_str(), l.shown())).collect();
    assert_eq!(shown, [("Outer", tequila_scene::layers::Shown::All), ("Construction", tequila_scene::layers::Shown::None)]);
    bus.apply(&mut document, tequila_scene::Command::SetLayerVisibility { layer: "Construction".into(), visible: true }).unwrap();
    assert!(placed(&document, "Pin").visible);
}

/// Where OpenCASCADE's helper is built (`native/occt/build-occt.bat`), it,
/// not truck, reads the layers and attributes above.
#[test]
fn opencascade_reads_the_layers_and_attributes_where_it_is_built() {
    let name = if cfg!(windows) { "tequila-occt.exe" } else { "tequila-occt" };
    let built = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../native/occt/build/occt").join(name);
    if std::env::var_os("TEQUILA_OCCT").is_some() || !built.is_file() {
        eprintln!("skipped: OpenCASCADE's helper is not built here");
        return;
    }
    let scene = read(LAYERS);
    assert!(scene.warnings.iter().all(|w| !w.contains("Tequila's own reader")), "the helper fell back: {:?}", scene.warnings);
    let node = |name: &str| scene.nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no node `{name}`"));
    assert_eq!(node("Housing").metadata.get("Finish").map(String::as_str), Some("Mold-Tech 11010"));
    assert_eq!(node("Box").layers, ["Outer"]);
    assert!(node("Pin").hidden);
}

/// A file that says nothing about its parts adds nothing: the product
/// whose id only repeats its name gets no part number.
#[test]
fn a_file_that_says_nothing_adds_no_metadata() {
    let scene = read(TWO_BODIES);
    for node in &scene.nodes {
        assert!(
            node.metadata.keys().all(|k| k == "Tessellation" || k == "B-rep" || k.starts_with("Source")),
            "{}: {:?}",
            node.name,
            node.metadata
        );
        assert!(!node.hidden);
    }
}
