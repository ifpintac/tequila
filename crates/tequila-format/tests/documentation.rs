//! `docs/file-format.md` against the code.
//!
//! A format document is only worth having if it is true, and prose does not
//! fail to compile. What rots it is not carelessness but ordinary work: a field
//! is added to the writer, the change is tested, the tests pass, and the
//! document quietly becomes a description of the previous version.
//!
//! So the document is read back and compared to what the writer actually emits.
//! Adding a field to either without the other fails here, with a message saying
//! which side is behind.

use tequila_core::math::{vec3, Transform, Vec3};
use tequila_format::json::{self, Value};
use tequila_format::zip::ZipArchive;
use tequila_format::{write, Preserved};
use tequila_scene::command::{reserve_keys, Command, ImportPayload};
use tequila_scene::geometry::Mesh;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::tree::{NodeKind, SceneNode};
use tequila_scene::{CommandBus, SceneDocument};
use std::collections::BTreeSet;

const DOCUMENT: &str = include_str!("../../../docs/file-format.md");

/// A document exercising every optional field, so nothing is missed by being
/// absent from the sample.
fn document_with_everything() -> SceneDocument {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();

    let mut mesh = Mesh::new(
        vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    );
    mesh.normals = vec![Vec3::Z; 3];
    let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 2);

    let mut root = SceneNode::new(ids[0], "Assembly", NodeKind::Group);
    root.children = vec![ids[1]];

    let mut part = SceneNode::new(ids[1], "Housing", NodeKind::Part { mesh: meshes[0] });
    part.parent = Some(ids[0]);
    part.material = Some(materials[0]);
    part.transform = Transform::translation(vec3(1.0, 2.0, 3.0));
    part.metadata.insert("PartNumber".into(), "HS-4410".into());
    // The optional ones, which a plainer fixture would leave out.
    part.path_key = Some("Assembly/Housing#1".into());
    part.pivot = Some(vec3(0.25, 0.0, -0.5));
    part.locked = true;
    part.visible = false;

    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], mesh)],
                textures: Vec::new(),
                materials: vec![(materials[0], textured_material())],
                nodes: vec![root, part],
                roots: vec![ids[0]],
            }),
        },
    )
    .unwrap();

    // A light, so the node table is checked against a node that has one.
    bus.apply(
        &mut document,
        Command::AddLight {
            parent: None,
            name: "Key".into(),
            settings: Box::new(tequila_scene::lighting::LightSettings::default()),
        },
    )
    .unwrap();

    // And a cutaway, for its own row, with the optional cap colour.
    bus.apply(
        &mut document,
        Command::AddCutaway {
            parent: None,
            name: "Section".into(),
            settings: Box::new(tequila_scene::cutaway::CutawaySettings {
                cap: tequila_scene::cutaway::CapStyle::Color(tequila_core::spectrum::Spectrum::splat(0.5)),
                ..Default::default()
            }),
            transform: Transform::IDENTITY,
        },
    )
    .unwrap();

    // Variants, model sets and a Studio, so the top level carries them.
    let material = document.tree.get(ids[1]).and_then(|n| n.material).expect("the part's material");
    document.multi_materials = vec![tequila_scene::studio::MultiMaterial {
        id: 1,
        name: "Colourways".into(),
        variants: vec![material],
        active: 0,
        parts: vec![ids[1]],
    }];
    document.model_sets = tequila_scene::studio::ModelSets {
        sets: vec![tequila_scene::studio::ModelSet {
            name: "Set".into(),
            nodes: vec![ids[0]],
            always_visible: false,
        }],
        active: Some(0),
    };
    document.studios = vec![tequila_scene::studio::Studio::capture("Hero", &document)];
    document.product_turn = tequila_scene::studio::ProductTurn { yaw: 0.9, pivot: [0.0, 0.0] };
    document.configurator = tequila_scene::configurator::Configurator::draft(&document);
    document.material_templates = vec![tequila_scene::templates::MaterialTemplate {
        name: "Fasteners".into(),
        rules: vec![tequila_scene::templates::Rule {
            condition: tequila_scene::templates::Condition::Name("bolt*".into()),
            material,
        }],
    }];

    // A measurement, so the top level carries its optional list.
    document.measurements = vec![tequila_scene::measure::Measurement::new(
        tequila_scene::measure::Anchor {
            node: Some(ids[1]),
            point: vec3(0.5, 0.0, 0.0),
        },
        tequila_scene::measure::Anchor {
            node: None,
            point: vec3(2.0, 0.0, 0.0),
        },
    )];

    // Optional fields have to be present, or the comparison silently excuses
    // whatever this fixture happens not to exercise.
    document.environment.image = Some("environments/studio.exr".into());
    document.environment.backplate = Some("plates/studio.jpg".into());
    document.environment.sky = Some(tequila_render_core::sky::Sky::default());
    // One pin of each kind, so every kind's own fields are written.
    document.environment.pins = vec![
        tequila_render_core::pins::softbox("Key", 30.0, 40.0, 0.8, 0.5, 10.0),
        tequila_render_core::pins::Pin::image(
            "Window",
            "pins/window.exr",
            tequila_render_core::pins::direction_from(1.0, 0.2),
            0.6,
        ),
        tequila_render_core::pins::Pin::backdrop_gradient(
            tequila_core::spectrum::Spectrum::splat(0.02),
            tequila_core::spectrum::Spectrum::splat(0.4),
        ),
    ];
    document.ocio = Some(tequila_scene::document::OcioSelection {
        config: "studio/config.ocio".into(),
        display: "sRGB".into(),
        view: "ACES 1.0 - SDR Video".into(),
    });
    document.textures.insert_with_id(
        tequila_materials::graph::TextureId(1),
        tequila_scene::textures::TextureSource {
            path: "maps/brushed.png".into(),
            role: tequila_texture::texture::MapRole::Data,
            color_space: Some(tequila_texture::image::ColorSpace::Raw),
        },
    );
    document
}

/// A material with a graph, so the comparison covers the optional `graph` key.
///
/// A plainer fixture would write no graph at all, and the table below would be
/// free to document anything about it without the writer ever disagreeing.
fn textured_material() -> MaterialDefinition {
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
    use tequila_materials::material_type::{MaterialType, Param};

    let mut graph = MaterialGraph::from_material(
        tequila_render_core::material::Material::metal(
            tequila_materials::fresnel::metals::ALUMINIUM,
            0.3,
        ),
    );
    let output = graph.output();
    let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
        mapping: tequila_texture::projection::Mapping::default(),
    }));
    let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
        texture: TextureId(1),
        fallback: Value::Color(tequila_core::spectrum::Spectrum::splat(0.5)),
    }));
    graph.connect(mapping, texture, Socket::Input(0));
    graph.connect(texture, output, Socket::Param(Param::SpecularRoughness));

    MaterialDefinition::from_graph("Anodized", MaterialType::Metal, graph)
}

/// Field names from a markdown table, taken from the first column's backticks.
///
/// The tables are the contract; the prose around them is free to say whatever
/// it needs to.
fn documented_fields(heading: &str) -> BTreeSet<String> {
    let section = DOCUMENT
        .split_once(heading)
        .unwrap_or_else(|| panic!("`{heading}` is not in docs/file-format.md"))
        .1;
    // Up to the next heading, so one section's table cannot answer for another.
    let section = section.split("\n## ").next().unwrap_or(section);
    let section = section.split("\n### ").next().unwrap_or(section);

    section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| line.split('`').nth(1))
        // `field?` in the table means optional; the name is the same.
        .map(|name| name.trim_end_matches('?').to_string())
        .collect()
}

fn keys_of(value: &Value) -> BTreeSet<String> {
    match value {
        Value::Object(map) => map.keys().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

fn compare(what: &str, documented: &BTreeSet<String>, written: &BTreeSet<String>) {
    let missing: Vec<&String> = written.difference(documented).collect();
    let extra: Vec<&String> = documented.difference(written).collect();
    assert!(
        missing.is_empty(),
        "{what}: the writer emits {missing:?}, which docs/file-format.md does not \
         document. Add them to the table."
    );
    assert!(
        extra.is_empty(),
        "{what}: docs/file-format.md documents {extra:?}, which the writer does not \
         emit. Either the field was removed, or it is spelled differently."
    );
}

#[test]
fn documented_fields_match_the_writer() {
    let document = document_with_everything();
    let bytes = write(&document, &Preserved::default());
    let archive = ZipArchive::read(&bytes).expect("the archive we just wrote");

    let manifest = json::parse(archive.get("manifest.json").expect("manifest")).unwrap();
    compare(
        "manifest.json",
        &documented_fields("## 2. `manifest.json`"),
        &keys_of(&manifest),
    );

    let scene = json::parse(archive.get("scene.json").expect("scene")).unwrap();
    compare(
        "scene.json",
        &documented_fields("### 3.1 Top level"),
        &keys_of(&scene),
    );

    compare(
        "scene.json `units`",
        &documented_fields("### 3.2 `units`"),
        &keys_of(scene.get("units").expect("units")),
    );
    compare(
        "scene.json `environment`",
        &documented_fields("### 3.3 `environment`"),
        &keys_of(scene.get("environment").expect("environment")),
    );

    // Every node's fields together, since `mesh` appears only on a Part and
    // `parent` only on a child.
    let mut pin_fields = BTreeSet::new();
    for pin in scene
        .get("environment")
        .and_then(|e| e.get("pins"))
        .expect("pins")
        .as_array()
    {
        pin_fields.extend(keys_of(pin));
    }
    compare(
        "a `pins` entry",
        &documented_fields("### 3.3.5 `pins` entries"),
        &pin_fields,
    );

    let mut node_fields = BTreeSet::new();
    for node in scene.get("tree").expect("tree").as_array() {
        node_fields.extend(keys_of(node));
    }
    compare(
        "a `tree` entry",
        &documented_fields("### 3.4 `tree` entries"),
        &node_fields,
    );

    let material_member = archive
        .names()
        .iter()
        .find(|n| n.starts_with("materials/"))
        .expect("a material member")
        .clone();
    let material = json::parse(archive.get(&material_member).unwrap()).unwrap();
    compare(
        "a material file",
        &documented_fields("## 4. `materials/<id>.json`"),
        &keys_of(&material),
    );
}

/// The document states the archive layout; a reader written from it has to find
/// those members where it says they are.
#[test]
fn the_documented_archive_layout_is_the_real_one() {
    let document = document_with_everything();
    let archive = ZipArchive::read(&write(&document, &Preserved::default())).unwrap();

    for member in ["manifest.json", "scene.json"] {
        assert!(
            archive.contains(member),
            "`{member}` is documented but absent"
        );
    }
    assert!(archive
        .names()
        .iter()
        .any(|n| n.starts_with("geometry/") && n.ends_with(".fsmesh")));
    assert!(archive
        .names()
        .iter()
        .any(|n| n.starts_with("materials/") && n.ends_with(".json")));

    // The id shapes the document promises, which anyone writing a reader will
    // match on.
    for name in archive.names() {
        if let Some(id) = name
            .strip_prefix("geometry/")
            .and_then(|n| n.strip_suffix(".fsmesh"))
        {
            assert!(
                id.starts_with("m_") && id.len() == 10,
                "`{id}` is not the documented `m_` plus eight hex digits"
            );
        }
        if let Some(id) = name
            .strip_prefix("materials/")
            .and_then(|n| n.strip_suffix(".json"))
        {
            assert!(
                id.starts_with("mat_") && id.len() == 12,
                "`{id}` is not the documented `mat_` plus eight hex digits"
            );
        }
    }
}

/// The header table in §5 is what someone writing a `.fsmesh` reader will work
/// from, so the offsets have to be the real ones.
#[test]
fn the_documented_fsmesh_header_matches_the_bytes() {
    let document = document_with_everything();
    let archive = ZipArchive::read(&write(&document, &Preserved::default())).unwrap();
    let member = archive
        .names()
        .iter()
        .find(|n| n.ends_with(".fsmesh"))
        .expect("a mesh")
        .clone();
    let bytes = archive.get(&member).unwrap();

    let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());

    assert_eq!(&bytes[0..8], b"FSMESH\0\0", "the magic moved");
    assert_eq!(u32_at(8), 1, "the version is not 1");
    assert_eq!(u32_at(12), 3, "vertex count is not at offset 12");
    assert_eq!(u32_at(16), 3, "index count is not at offset 16");
    assert_eq!(
        u32_at(20) & 1,
        1,
        "the normals flag is not bit 0 at offset 20"
    );
    assert_eq!(u32_at(24), 0, "the UV set count is not at offset 24");
    // 28 + 24 bytes of bounds = 52, the header size the document states.
    assert!(bytes.len() > 52);
}

/// The one part of the document that is a promise about behaviour rather than
/// about layout, and the easiest to let slip.
#[test]
fn saving_twice_really_is_byte_identical() {
    let document = document_with_everything();
    assert_eq!(
        write(&document, &Preserved::default()),
        write(&document, &Preserved::default()),
        "the document promises byte-identical saves"
    );
}
