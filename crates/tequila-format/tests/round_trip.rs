//! The save/reopen half of M1's exit criterion:
//!
//! > import a multi-part glTF assembly, reorganize it, save, reopen, and get a
//! > byte-identical document
//!
//! "Byte-identical" is taken literally: the bytes of a second save must equal
//! the bytes of the first. That is a stronger claim than "the document is
//! equal", and it is the useful one — it means a saved project in version
//! control does not churn, and it catches every source of nondeterminism at
//! once (map iteration order, timestamps, float formatting) that document
//! equality would let through.

use tequila_core::math::{vec3, Transform};
use tequila_core::units::LengthUnit;
use tequila_format::{read, write, FormatError, Preserved};
use tequila_import::{build, read_format, ImportOptions};
use tequila_scene::command::Command;
use tequila_scene::tree::NodeId;
use tequila_scene::units::ConversionMode;
use tequila_scene::{CommandBus, SceneDocument};

/// Positions and indices for a triangle, base64 in a data URI.
fn assembly_gltf() -> String {
    let mut buffer = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            buffer.extend_from_slice(&c.to_le_bytes());
        }
    }
    let index_offset = buffer.len();
    for i in [0u16, 1, 2] {
        buffer.extend_from_slice(&i.to_le_bytes());
    }
    let total = buffer.len();

    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in buffer.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                encoded.push(TABLE[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
            } else {
                encoded.push('=');
            }
        }
    }

    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
"nodes":[
  {{"name":"Assembly","children":[1,2,3]}},
  {{"name":"Housing","mesh":0,"translation":[0.0,0.0,0.0],
    "extras":{{"PartNumber":"HS-4410","Material":"ABS"}}}},
  {{"name":"Screw A","mesh":0,"translation":[10.0,0.0,0.0],
    "rotation":[0.0,0.3826834,0.0,0.9238795]}},
  {{"name":"Screw B","mesh":0,"translation":[20.0,0.0,0.0],"scale":[2.0,1.0,0.5]}}],
"meshes":[{{"name":"Body","primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
"materials":[{{"name":"Anodized","pbrMetallicRoughness":{{
  "baseColorFactor":[0.2,0.4,0.8,1.0],"metallicFactor":1.0,"roughnessFactor":0.3}}}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
             {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}],
"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{index_offset}}},
               {{"buffer":0,"byteOffset":{index_offset},"byteLength":{}}}],
"buffers":[{{"byteLength":{total},"uri":"data:application/octet-stream;base64,{encoded}"}}]}}"#,
        total - index_offset
    )
}

/// Import the assembly, then reorganize it the way a user would: group the
/// fasteners, rename, hide something, lock something, unlink a material,
/// change units.
fn imported_and_reorganized() -> SceneDocument {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();

    let scene = read_format(assembly_gltf().as_bytes(), "gltf", "assembly", &|_| None)
        .expect("the assembly reads");
    let (command, report) =
        build(&document, None, scene, &ImportOptions::default(), None).expect("the assembly builds");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    bus.apply(&mut document, command).unwrap();

    let by_name = |document: &SceneDocument, name: &str| -> NodeId {
        document
            .tree
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no node named `{name}`"))
            .id
    };

    let (a, b) = (by_name(&document, "Screw A"), by_name(&document, "Screw B"));
    bus.apply(
        &mut document,
        Command::Group {
            nodes: vec![a, b],
            name: "Fasteners".into(),
        },
    )
    .unwrap();

    let housing = by_name(&document, "Housing");
    for command in [
        Command::Rename {
            node: housing,
            name: "Housing, upper".into(),
        },
        Command::set_visibility(b, false),
        Command::set_locked(a, true),
        Command::set_transform(housing, Transform::translation(vec3(1.5, -2.25, 0.125))),
        Command::SetMetadata {
            node: b,
            key: "Torque_Nm".into(),
            value: Some("1.2".into()),
        },
        // §04 §5 requires pivots to persist. A pivot that survived every
        // other check but not the file would be the one failure the word
        // "persist" rules out.
        Command::set_pivot(housing, Some(vec3(0.4, -0.2, 0.75))),
        Command::UnlinkMaterial { node: housing },
        Command::SetUnits {
            unit: LengthUnit::Millimetre,
            mode: ConversionMode::KeepSceneSize,
        },
    ] {
        bus.apply(&mut document, command).unwrap();
    }

    document.validate().unwrap();
    document
}

/// The exit criterion, stated directly.
#[test]
fn a_reorganized_assembly_saves_and_reopens_byte_identically() {
    let document = imported_and_reorganized();

    let first = write(&document, &Preserved::default());
    let reopened = read(&first).expect("the file we just wrote must read");
    let second = write(&reopened.document, &reopened.preserved);

    assert_eq!(
        first.len(),
        second.len(),
        "the second save is a different size"
    );
    assert!(
        first == second,
        "the second save differs from the first at byte {}",
        first
            .iter()
            .zip(&second)
            .position(|(a, b)| a != b)
            .unwrap_or(0)
    );
    assert!(reopened.notes.is_empty(), "{:?}", reopened.notes);
}

/// And the scene itself must survive, not merely the bytes. A writer and
/// reader that both dropped the same field would pass the test above.
#[test]
fn every_part_of_the_scene_survives_the_round_trip() {
    let document = imported_and_reorganized();
    let reopened = read(&write(&document, &Preserved::default()))
        .unwrap()
        .document;

    if let Some(difference) = reopened.difference_from(&document) {
        panic!("the reopened document differs: {difference}");
    }

    // Spot-check the things a lazy serializer loses, by name rather than by
    // trusting the comparison above.
    let node = |name: &str| {
        reopened
            .tree
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no node named `{name}`"))
    };
    assert!(!node("Screw B").visible, "visibility was lost");
    assert!(node("Screw A").locked, "the lock was lost");
    assert_eq!(
        node("Screw B")
            .metadata
            .get("Torque_Nm")
            .map(|s| s.as_str()),
        Some("1.2"),
        "metadata was lost"
    );
    assert_eq!(
        node("Housing, upper")
            .metadata
            .get("PartNumber")
            .map(|s| s.as_str()),
        Some("HS-4410"),
        "imported CAD metadata was lost"
    );
    assert_eq!(
        node("Housing, upper").pivot,
        Some(vec3(0.4, -0.2, 0.75)),
        "the custom pivot did not survive the file"
    );
    assert_eq!(reopened.units.unit, LengthUnit::Millimetre);
    assert_eq!(
        reopened
            .tree
            .get(node("Fasteners").id)
            .unwrap()
            .children
            .len(),
        2
    );

    // A non-uniform scale composed with a rotation is exactly the transform a
    // TRS decomposition would round off.
    let scaled = node("Screw B");
    assert_eq!(
        scaled.transform,
        document
            .tree
            .iter()
            .find(|n| n.name == "Screw B")
            .unwrap()
            .transform,
        "the transform did not survive exactly"
    );
}

/// §2.3: an ID is never reused. The watermarks are not derivable from the
/// content — the highest node may have been deleted — so they must be written.
#[test]
fn the_id_allocators_survive_so_ids_are_not_reissued() {
    let mut document = imported_and_reorganized();
    let mut bus = CommandBus::new();

    // Delete the highest-numbered node, so the watermark is now above anything
    // the file contains.
    let last = document.tree.iter().map(|n| n.id).max().unwrap();
    bus.apply(&mut document, Command::DeleteNodes { nodes: vec![last] })
        .unwrap();
    let watermark = document.tree.next_id();
    assert!(watermark > last.0);

    let mut reopened = read(&write(&document, &Preserved::default()))
        .unwrap()
        .document;
    assert_eq!(
        reopened.tree.next_id(),
        watermark,
        "the node allocator was rewound by a save and reopen"
    );

    // Which is the point: the next node added is not the one just deleted.
    let mut bus = CommandBus::new();
    bus.apply(
        &mut reopened,
        Command::AddGroup {
            parent: None,
            name: "New".into(),
        },
    )
    .unwrap();
    let added = reopened.tree.iter().find(|n| n.name == "New").unwrap().id;
    assert_ne!(added, last, "a deleted ID was reissued after a reopen");
}

/// §4.4: "Newer minor: load, warn about ignored unknown fields, **preserve
/// them on save**." This is what stops an older build silently deleting a
/// newer build's work.
#[test]
fn unknown_fields_from_a_newer_build_survive_a_load_and_save() {
    let document = imported_and_reorganized();
    let original = write(&document, &Preserved::default());

    // Rewrite the archive as a newer minor version would: a bumped version, a
    // new top-level field, a new field on a node, and a whole new member.
    let newer = rewrite_as_newer(&original);

    let loaded = read(&newer).expect("a newer minor version must load");
    assert!(!loaded.preserved.is_empty());
    assert!(
        loaded.notes.iter().any(|n| n.contains("preserved")),
        "the user was not told: {:?}",
        loaded.notes
    );

    // Save it back and confirm the newer build's data is still there.
    let saved = write(&loaded.document, &loaded.preserved);
    let text = String::from_utf8_lossy(&saved);
    assert!(text.contains("depth_of_field"), "a scene field was dropped");
    assert!(text.contains("surface_finish"), "a node field was dropped");
    assert!(text.contains("animation.json"), "a member was dropped");

    // And the whole thing still round-trips byte-identically from there.
    let reloaded = read(&saved).unwrap();
    assert_eq!(saved, write(&reloaded.document, &reloaded.preserved));
}

/// Splice fields into a written archive the way a newer build would have
/// written them.
fn rewrite_as_newer(bytes: &[u8]) -> Vec<u8> {
    use tequila_format::json;
    use tequila_format::json_write;
    use tequila_format::zip::{ZipArchive, ZipWriter};

    let archive = ZipArchive::read(bytes).unwrap();
    let mut scene = json::parse(archive.get("scene.json").unwrap()).unwrap();

    let json::Value::Object(map) = &mut scene else {
        panic!("scene.json is an object")
    };
    map.insert("format_version".into(), json::Value::String("1.7.0".into()));
    map.insert("depth_of_field".into(), json::Value::Number(2.8));
    if let Some(json::Value::Array(nodes)) = map.get_mut("tree") {
        if let Some(json::Value::Object(node)) = nodes.first_mut() {
            node.insert(
                "surface_finish".into(),
                json::Value::String("bead blast".into()),
            );
        }
    }

    let mut out = ZipWriter::new();
    for name in archive.names() {
        if name == "scene.json" {
            out.add(name.clone(), json_write::to_string(&scene).into_bytes());
        } else {
            out.add(name.clone(), archive.get(name).unwrap().to_vec());
        }
    }
    out.add("animation.json", br#"{"tracks":[]}"#.to_vec());
    out.finish()
}

/// §4.4: "Newer major: refuse with a clear message."
#[test]
fn a_newer_major_version_is_refused_by_name() {
    let document = imported_and_reorganized();
    let bytes = write(&document, &Preserved::default());
    let bumped = with_version(&bytes, "2.0.0");

    match read(&bumped) {
        Err(FormatError::TooNew { file, supported }) => {
            assert_eq!(file, "2.0.0");
            assert_eq!(supported, "1.0.0");
        }
        other => panic!("expected a refusal, got {:?}", other.map(|l| l.notes)),
    }
}

/// An older major version runs the migration hook and says so.
#[test]
fn an_older_major_version_logs_its_migration() {
    let document = imported_and_reorganized();
    let bytes = write(&document, &Preserved::default());
    let loaded = read(&with_version(&bytes, "0.9.0")).expect("an older file must load");
    assert!(
        loaded
            .notes
            .iter()
            .any(|n| n.contains("migrated from format 0.9.0")),
        "{:?}",
        loaded.notes
    );
}

fn with_version(bytes: &[u8], version: &str) -> Vec<u8> {
    use tequila_format::json;
    use tequila_format::json_write;
    use tequila_format::zip::{ZipArchive, ZipWriter};

    let archive = ZipArchive::read(bytes).unwrap();
    let mut scene = json::parse(archive.get("scene.json").unwrap()).unwrap();
    let json::Value::Object(map) = &mut scene else {
        panic!()
    };
    map.insert(
        "format_version".into(),
        json::Value::String(version.to_string()),
    );

    let mut out = ZipWriter::new();
    for name in archive.names() {
        if name == "scene.json" {
            out.add(name.clone(), json_write::to_string(&scene).into_bytes());
        } else {
            out.add(name.clone(), archive.get(name).unwrap().to_vec());
        }
    }
    out.finish()
}

/// §4.2: "Reject-with-explanation beats silently misparsing." A scene whose
/// cross-references do not hold together must not load as a broken document.
#[test]
fn an_inconsistent_scene_is_refused_rather_than_half_loaded() {
    let document = imported_and_reorganized();
    let bytes = write(&document, &Preserved::default());

    let broken = mutate_scene(&bytes, &|map| {
        // A node claiming a mesh that is not in the archive.
        if let Some(tequila_format::json::Value::Array(nodes)) = map.get_mut("tree") {
            for node in nodes.iter_mut() {
                if let tequila_format::json::Value::Object(fields) = node {
                    if fields.get("kind").and_then(|v| v.as_str()) == Some("Part") {
                        fields.insert(
                            "mesh".into(),
                            tequila_format::json::Value::String("m_deadbeef".into()),
                        );
                        break;
                    }
                }
            }
        }
    });
    let error = read(&broken).unwrap_err();
    assert!(
        error.to_string().contains("missing mesh"),
        "unhelpful message: {error}"
    );

    // A parent/child disagreement, which would make a walk down and a walk up
    // see different scenes.
    let broken = mutate_scene(&bytes, &|map| {
        if let Some(tequila_format::json::Value::Array(nodes)) = map.get_mut("tree") {
            if let Some(tequila_format::json::Value::Object(fields)) = nodes.get_mut(1) {
                fields.insert(
                    "children".into(),
                    tequila_format::json::Value::Array(vec![
                        tequila_format::json::Value::Number(999.0),
                    ]),
                );
            }
        }
    });
    let error = read(&broken).unwrap_err();
    assert!(
        error.to_string().contains("999"),
        "unhelpful message: {error}"
    );
}

fn mutate_scene(
    bytes: &[u8],
    edit: &dyn Fn(&mut std::collections::BTreeMap<String, tequila_format::json::Value>),
) -> Vec<u8> {
    use tequila_format::json;
    use tequila_format::json_write;
    use tequila_format::zip::{ZipArchive, ZipWriter};

    let archive = ZipArchive::read(bytes).unwrap();
    let mut scene = json::parse(archive.get("scene.json").unwrap()).unwrap();
    let json::Value::Object(map) = &mut scene else {
        panic!()
    };
    edit(map);

    let mut out = ZipWriter::new();
    for name in archive.names() {
        if name == "scene.json" {
            out.add(name.clone(), json_write::to_string(&scene).into_bytes());
        } else {
            out.add(name.clone(), archive.get(name).unwrap().to_vec());
        }
    }
    out.finish()
}

/// The archive members are the ones §4.1 lists, in the places it puts them.
#[test]
fn the_archive_has_the_layout_the_specification_describes() {
    use tequila_format::zip::ZipArchive;

    let document = imported_and_reorganized();
    let archive = ZipArchive::read(&write(&document, &Preserved::default())).unwrap();

    assert!(archive.contains("manifest.json"));
    assert!(archive.contains("scene.json"));
    assert!(
        archive
            .names()
            .iter()
            .any(|n| n.starts_with("geometry/") && n.ends_with(".fsmesh")),
        "geometry is not split out: {:?}",
        archive.names()
    );
    assert!(
        archive
            .names()
            .iter()
            .any(|n| n.starts_with("materials/") && n.ends_with(".json")),
        "materials are not split out: {:?}",
        archive.names()
    );

    // scene.json must stay human-readable and diffable, which is §4.1's whole
    // reason for the split.
    let scene = String::from_utf8(archive.get("scene.json").unwrap().to_vec()).unwrap();
    assert!(scene.contains("\n  \""), "scene.json is not pretty-printed");
    assert!(scene.contains("\"Housing, upper\""));
}

/// A damaged file must be reported, never partially loaded.
#[test]
fn a_damaged_file_is_refused() {
    let document = imported_and_reorganized();
    let bytes = write(&document, &Preserved::default());

    // Every single-byte truncation.
    for cut in [0, 1, 32, bytes.len() / 2, bytes.len() - 1] {
        assert!(read(&bytes[..cut]).is_err(), "truncating to {cut} loaded");
    }

    // A flipped bit anywhere in a member is caught by the archive checksum.
    let mut damaged = bytes.clone();
    let at = damaged.len() / 3;
    damaged[at] ^= 0xff;
    assert!(read(&damaged).is_err(), "a corrupted file loaded");
}

/// An empty document is a real case — File > New, then Save — and its round
/// trip must hold too.
#[test]
fn an_empty_document_round_trips() {
    let document = SceneDocument::new();
    let first = write(&document, &Preserved::default());
    let reopened = read(&first).unwrap();

    assert!(reopened.document.difference_from(&document).is_none());
    assert_eq!(first, write(&reopened.document, &reopened.preserved));
}

/// Every environment parameter survives a save and a reopen, with a value that
/// is not its default.
///
/// Both the writer and the reader are driven by `EnvParam::ALL`, so a new
/// control is meant to be carried automatically — and two doc comments in the
/// codebase said so, each naming a test that checked it. Neither test existed.
/// A claim about what is safe, with a named test behind it that was never
/// written, is worse than no claim: it is the reason nobody looked.
///
/// The non-default value matters. A round trip that wrote nothing, read
/// nothing, and left the field at its default would pass against defaults and
/// silently drop every setting a user had changed — which is precisely the
/// failure this is for.
#[test]
fn every_environment_parameter_survives_a_round_trip() {
    use tequila_scene::command::{EnvParam, EnvValue};
    use tequila_scene::document::BackgroundMode;

    let mut document = SceneDocument::new();

    // A value distinguishable from the default, per parameter kind.
    let changed = |param: EnvParam, current: &EnvValue| -> EnvValue {
        match current {
            EnvValue::Scalar(v) => {
                // Inside the parameter's own range, so it is not clamped back.
                let (low, high) = param.ui().range;
                let candidate = if *v == low { high } else { low };
                EnvValue::Scalar(if candidate == *v { v + 0.25 } else { candidate })
            }
            EnvValue::Flag(v) => EnvValue::Flag(!v),
            EnvValue::Color(c) => EnvValue::Color(tequila_core::spectrum::spectrum(
                (c.r + 0.3).fract(),
                (c.g + 0.5).fract(),
                (c.b + 0.7).fract(),
            )),
            EnvValue::Mode(m) => EnvValue::Mode(if *m == BackgroundMode::Color {
                BackgroundMode::Backplate
            } else {
                BackgroundMode::Color
            }),
            EnvValue::Path(_) => EnvValue::Path(Some("assets/somewhere/thing.hdr".to_string())),
            // Another of its options.
            EnvValue::Choice(c) => EnvValue::Choice(
                param.choices().iter().map(|(key, _)| *key).find(|key| key != c).expect("a choice of one"),
            ),
        }
    };

    let mut expected = Vec::new();
    for param in EnvParam::ALL {
        let before = param.read(&document.environment);
        let after = changed(param, &before);
        param.write(&mut document.environment, &after);
        expected.push((param, after));
    }

    let reopened = read(&write(&document, &Preserved::default()))
        .expect("the file we just wrote must read");

    for (param, wanted) in expected {
        let got = param.read(&reopened.document.environment);
        assert_eq!(
            got,
            wanted,
            "`{}` did not survive the round trip",
            param.key()
        );
    }
}

/// Variants, model sets and Studios survive a save and a reopen, every
/// optional part of a Studio included — both "show every set" and "leave
/// the set alone", which are different answers.
#[test]
fn studios_survive_a_round_trip() {
    use tequila_scene::studio::{ModelSet, ModelSets, MultiMaterial, Studio};
    let mut document = SceneDocument::new();
    document.multi_materials = vec![MultiMaterial {
        id: 3,
        name: "Colourways".into(),
        variants: vec![tequila_scene::MaterialKey(1), tequila_scene::MaterialKey(2)],
        active: 1,
        parts: vec![NodeId(4), NodeId(9)],
    }];
    document.model_sets = ModelSets {
        sets: vec![
            ModelSet {
                name: "Chassis A".into(),
                nodes: vec![NodeId(2)],
                always_visible: false,
            },
            ModelSet {
                name: "Stage".into(),
                nodes: vec![NodeId(7), NodeId(8)],
                always_visible: true,
            },
        ],
        active: Some(0),
    };
    let mut full = Studio::capture("Hero", &document);
    full.environment.as_mut().unwrap().rotation = 1.5;
    let mut all_sets = Studio {
        name: "Every set".into(),
        camera: None,
        environment: None,
        image_style: None,
        model_set: Some(None),
        variants: Vec::new(),
    };
    let untouched = Studio {
        model_set: None,
        name: "Leaves the set".into(),
        ..all_sets.clone()
    };
    all_sets.variants = vec![(3, 0)];
    document.studios = vec![full, all_sets, untouched];

    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.multi_materials, document.multi_materials);
    assert_eq!(reopened.document.model_sets, document.model_sets);
    assert_eq!(reopened.document.studios, document.studios);
}

/// The configurator survives a save and a reopen: every kind of option, a
/// swatch given and one left to the variant, both kinds of rule, the
/// layout and the configuration on show.
#[test]
fn a_configurator_survives_a_round_trip() {
    use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Drives, Placement, Rule, RuleKind, Style};
    let mut document = SceneDocument::new();
    let choice = |label: &str, target, swatch| Choice { label: label.into(), target, swatch };
    document.configurator = Configurator {
        title: "Kettle K2".into(),
        subtitle: "Choose your finish".into(),
        placement: Placement::Left,
        options: vec![
            ConfigOption {
                id: 3,
                name: "Body".into(),
                drives: Drives::Material { multi_material: 8 },
                choices: vec![choice("Red", 0, Some([0.6, 0.05, 0.04])), choice("Cream", 1, None)],
                style: Style::Swatches,
            },
            ConfigOption { id: 5, name: "Base".into(), drives: Drives::ModelSet, choices: vec![choice("Cord", 0, None), choice("Cordless", 1, None)], style: Style::List },
            ConfigOption { id: 6, name: "View".into(), drives: Drives::Studio, choices: vec![choice("Hero", 2, None)], style: Style::Buttons },
        ],
        rules: vec![
            Rule { option: 3, choice: 1, kind: RuleKind::Excludes, other: 5, other_choice: 0 },
            Rule { option: 5, choice: 1, kind: RuleKind::Requires, other: 6, other_choice: 0 },
        ],
        current: vec![1, 1, 0],
    };
    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.configurator, document.configurator);
}

/// Render layers survive a save and a reopen, a switched-off one included.
#[test]
fn render_layers_survive_a_round_trip() {
    use tequila_scene::render::RenderLayer;
    let mut document = SceneDocument::new();
    document.render.layers = vec![
        RenderLayer {
            name: "Body".into(),
            nodes: vec![NodeId(2), NodeId(5)],
            enabled: true,
        },
        RenderLayer {
            name: "Buttons".into(),
            nodes: vec![NodeId(9)],
            enabled: false,
        },
    ];
    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.render.layers, document.render.layers);
}

/// The finer Cycles controls survive a save and a reopen - the lighting's
/// Light Paths and the render's direct clamp, pixel filter and adaptive
/// threshold - and a project that sets none of them writes no Light Paths
/// at all, so it is the same bytes it always was.
#[test]
fn the_finer_cycles_controls_survive_a_round_trip() {
    use tequila_render_core::settings::{CyclesTransport, IntegratorConfig, LightingSettings, PixelFilter, PixelFilterKind};
    let plain = SceneDocument::new();
    let text = write(&plain, &Preserved::default());
    assert!(!String::from_utf8_lossy(&text).contains("\"cycles\""), "Light Paths written for a project that sets none");

    let mut document = SceneDocument::new();
    let mut custom = IntegratorConfig { max_bounces: 16, max_transmission_bounces: 12, ..IntegratorConfig::product() };
    custom.cycles = CyclesTransport {
        diffuse_bounces: Some(4),
        glossy_bounces: Some(8),
        transparent_bounces: Some(16),
        volume_bounces: Some(0),
        filter_glossy: Some(0.5),
        caustics_reflective: Some(false),
        caustics_refractive: Some(true),
    };
    document.lighting = LightingSettings { preset: None, custom };
    document.render.clamp_direct = Some(0.0);
    document.render.clamp_indirect = Some(10.0);
    document.render.pixel_filter = Some(PixelFilter { kind: PixelFilterKind::BlackmanHarris, width: 1.5 });
    document.render.adaptive_threshold = Some(0.01);
    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.lighting.custom.cycles, document.lighting.custom.cycles);
    assert_eq!(reopened.document.render, document.render);
}

/// Measurements survive a save and a reopen, ends on parts and in the
/// world alike, hidden ones included.
#[test]
fn measurements_survive_a_round_trip() {
    use tequila_scene::measure::{Anchor, Measurement};
    let mut document = SceneDocument::new();
    document.measurements = vec![
        Measurement::new(
            Anchor {
                node: Some(NodeId(3)),
                point: vec3(0.125, -2.5, 7.0),
            },
            Anchor {
                node: None,
                point: vec3(1.0, 2.0, 3.0),
            },
        ),
        Measurement {
            visible: false,
            ..Measurement::new(
                Anchor {
                    node: None,
                    point: vec3(0.0, 0.0, 0.0),
                },
                Anchor {
                    node: Some(NodeId(1)),
                    point: vec3(-1.0, 0.0, 0.5),
                },
            )
        },
    ];
    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.measurements, document.measurements);

    // And none written for none, so an older project's bytes are unchanged.
    let empty = write(&SceneDocument::new(), &Preserved::default());
    let archive = tequila_format::zip::ZipArchive::read(&empty).unwrap();
    let scene = String::from_utf8_lossy(archive.get("scene.json").unwrap()).into_owned();
    assert!(!scene.contains("measurements"));
}

/// A cutaway node survives a save and a reopen: its shape, its switch, its
/// cap colour, and its placement.
#[test]
fn a_cutaway_survives_a_round_trip() {
    use tequila_scene::cutaway::{CapStyle, CutShape, CutawaySettings};
    let mut document = SceneDocument::new();
    let mut bus = tequila_scene::CommandBus::new();
    let settings = CutawaySettings {
        shape: CutShape::Cylinder,
        cap: CapStyle::Color(tequila_core::spectrum::Spectrum { r: 0.7, g: 0.1, b: 0.2 }),
        enabled: false,
    };
    bus.apply(
        &mut document,
        tequila_scene::Command::AddCutaway {
            parent: None,
            name: "Bore".into(),
            settings: Box::new(settings.clone()),
            transform: Transform::translation(vec3(0.5, 1.0, -2.0)),
        },
    )
    .unwrap();

    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert!(
        reopened.document.difference_from(&document).is_none(),
        "{:?}",
        reopened.document.difference_from(&document)
    );
    // Read as ours, not as a newer build's field kept for it.
    assert!(reopened.notes.is_empty(), "{:?}", reopened.notes);
    let node = reopened
        .document
        .tree
        .iter()
        .find(|n| n.name == "Bore")
        .expect("the cutaway came back");
    assert_eq!(
        node.kind,
        tequila_scene::tree::NodeKind::Cutaway {
            settings: Box::new(settings)
        }
    );
}

/// All three kinds of pin survive a save and a reopen, each with the fields
/// that only it has: an image pin's picture, a gradient's stops and shape,
/// and the blend and lighting switches that make a backdrop a backdrop.
#[test]
fn every_kind_of_pin_survives_a_round_trip() {
    use tequila_core::spectrum::Spectrum;
    use tequila_render_core::pins::{direction_from, GradientStop, Pin, PinKind};

    let mut document = SceneDocument::new();
    let mut linear = Pin::backdrop_gradient(Spectrum::splat(0.05), Spectrum::splat(0.6));
    linear.kind = PinKind::Gradient {
        stops: vec![
            GradientStop {
                t: 0.2,
                color: Spectrum { r: 0.1, g: 0.2, b: 0.3 },
            },
            GradientStop {
                t: 0.5,
                color: Spectrum { r: 0.9, g: 0.8, b: 0.7 },
            },
            GradientStop {
                t: 0.95,
                color: Spectrum::splat(0.4),
            },
        ],
        radial: false,
    };
    document.environment.pins = vec![
        tequila_render_core::pins::softbox("Key", 40.0, 30.0, 0.8, 0.5, 12.0),
        Pin::image("Window", "assets/window.exr", direction_from(1.0, 0.3), 0.7),
        Pin::backdrop_gradient(Spectrum::splat(0.02), Spectrum { r: 0.3, g: 0.35, b: 0.4 }),
        linear,
    ];

    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.environment.pins, document.environment.pins);
}

/// Every procedural survives a save and a reopen, with parameters that are not
/// its defaults.
///
/// The non-default values are the point. A writer and a reader that both
/// dropped the parameters would pass a defaults-only test and silently reset
/// every procedural a person had tuned — which is the same failure the
/// environment round-trip guards against, in a place that had no guard at all.
#[test]
fn every_procedural_survives_a_round_trip() {
    use tequila_materials::material_type::Param;
    use tequila_texture::noise::{Basis, Fractal, Metric};
    use tequila_texture::procedural::{Axis, CellOutput, HoleShape, Procedural, WeavePattern};

    let tuned = [
        Procedural::Noise {
            fractal: Fractal {
                octaves: 7,
                lacunarity: 2.7,
                gain: 0.37,
                basis: Basis::Value,
            },
            frequency: 13.5,
        },
        Procedural::Cellular {
            metric: Metric::Chebyshev,
            output: CellOutput::Edges,
            frequency: 21.0,
        },
        Procedural::Scratches {
            frequency: 11.0,
            width: 0.11,
            density: 0.42,
            spread: 0.17,
        },
        Procedural::Brushed {
            frequency: 95.0,
            stretch: 23.0,
        },
        // Every field off its default, including the axis and the whole
        // fractal block — a writer that dropped the fractal would still round
        // -trip a marble whose fractal happened to be the default one.
        Procedural::Marble {
            axis: Axis::Z,
            frequency: 7.25,
            distortion: 4.5,
            fractal: Fractal {
                octaves: 6,
                lacunarity: 2.3,
                gain: 0.61,
                basis: Basis::Value,
            },
        },
        Procedural::Wood {
            axis: Axis::X,
            frequency: 14.0,
            distortion: 0.82,
            grain: 0.55,
            fractal: Fractal {
                octaves: 2,
                lacunarity: 3.1,
                gain: 0.44,
                basis: Basis::Value,
            },
        },
        Procedural::WoodAdvanced {
            axis: Axis::Z,
            frequency: 11.0,
            distortion: 0.44,
            grain: 0.15,
            fractal: Fractal {
                octaves: 4,
                lacunarity: 2.3,
                gain: 0.61,
                basis: Basis::Perlin,
            },
            ring_variation: 0.72,
            latewood: 0.21,
            contrast: 0.37,
            flecks: 0.63,
            knots: 0.18,
            board_width: 0.42,
        },
        Procedural::NoiseTexture {
            basis: Basis::Value,
            frequency: 17.0,
            contrast: 2.5,
            bias: -0.15,
        },
        Procedural::Ridged {
            fractal: Fractal {
                octaves: 3,
                lacunarity: 2.2,
                gain: 0.7,
                basis: Basis::Value,
            },
            frequency: 5.5,
        },
        Procedural::Granite {
            frequency: 12.5,
            speckle: 0.33,
            fine: 0.61,
        },
        Procedural::Leather {
            frequency: 31.0,
            crack_width: 0.07,
            wrinkle: 0.45,
        },
        Procedural::Spots {
            frequency: 6.5,
            coverage: 0.72,
            variance: 0.55,
            softness: 0.31,
        },
        Procedural::Camouflage {
            frequency: 2.25,
            layers: 3,
        },
        Procedural::Contour {
            axis: Axis::Z,
            frequency: 7.0,
            width: 0.18,
            softness: 0.12,
            distortion: 1.4,
        },
        Procedural::BrushedRadial {
            axis: Axis::X,
            frequency: 150.0,
            stretch: 35.0,
        },
        Procedural::Weave {
            pattern: WeavePattern::Satin,
            frequency: 55.0,
            width: 0.7,
            variance: 0.4,
        },
        Procedural::Mesh {
            shape: HoleShape::Hex,
            frequency: 14.0,
            size: 0.45,
            softness: 0.2,
        },
    ];
    // One of every kind, so a kind added without a writer and reader fails
    // here rather than resetting to noise the first time a file is reopened.
    let kinds: std::collections::BTreeSet<&str> = tuned.iter().map(|p| p.key()).collect();
    assert_eq!(kinds.len(), Procedural::catalog().len(), "a procedural kind is missing from this test");

    for procedural in tuned {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let key = document
            .materials
            .insert(tequila_scene::material::MaterialDefinition::new(
                "Tuned",
                tequila_materials::material_type::MaterialType::Metal,
                Default::default(),
            ));
        bus.apply(
            &mut document,
            Command::SetParameterProcedural {
                material: key,
                param: Param::SpecularRoughness,
                procedural: Some(Box::new(procedural)),
                seed: 4242,
            },
        )
        .expect("attach");

        let reopened = read(&write(&document, &Preserved::default()))
            .expect("the file we just wrote must read");
        let graph = &reopened
            .document
            .materials
            .get(key)
            .expect("the material survives")
            .graph;

        let (read_back, seed) =
            tequila_scene::command::procedural_of(graph, Param::SpecularRoughness)
                .unwrap_or_else(|| panic!("{} lost its procedural", procedural.label()));
        assert_eq!(
            read_back,
            procedural,
            "{} came back changed",
            procedural.label()
        );
        assert_eq!(seed, 4242, "{} lost its seed", procedural.label());
    }
}

/// A procedural on a *colour* parameter gets a ramp between the two, and both
/// survive a round trip.
///
/// The ramp is inserted on the user's behalf rather than asked for, so it is
/// exactly the kind of machinery that can go missing without anybody noticing
/// until a saved wood material reopens as a grey field.
#[test]
fn a_colour_procedural_keeps_its_ramp() {
    use tequila_materials::material_type::Param;
    use tequila_texture::gradient::Gradient;
    use tequila_texture::procedural::Procedural;

    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let key = document
        .materials
        .insert(tequila_scene::material::MaterialDefinition::new(
            "Grained",
            tequila_materials::material_type::MaterialType::Plastic,
            Default::default(),
        ));

    bus.apply(
        &mut document,
        Command::SetParameterProcedural {
            material: key,
            param: Param::BaseColor,
            procedural: Some(Box::new(Procedural::default())),
            seed: 17,
        },
    )
    .expect("attach");

    // A ramp appeared, unasked for, because a colour needs one.
    let graph = &document.materials.get(key).unwrap().graph;
    assert!(
        tequila_scene::command::gradient_of(graph, Param::BaseColor).is_some(),
        "a colour parameter got a procedural with no ramp to read it through"
    );
    // And the procedural is still findable *through* it.
    assert!(
        tequila_scene::command::procedural_of(graph, Param::BaseColor).is_some(),
        "the ramp hid the procedural behind it"
    );

    let wood = Gradient::presets()
        .into_iter()
        .find(|(name, _)| *name == "Wood")
        .map(|(_, g)| g)
        .expect("the Wood preset");
    bus.apply(
        &mut document,
        Command::SetParameterGradient {
            material: key,
            param: Param::BaseColor,
            gradient: Box::new(wood.clone()),
        },
    )
    .expect("set the ramp");

    let reopened =
        read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    let graph = &reopened.document.materials.get(key).unwrap().graph;
    assert_eq!(
        tequila_scene::command::gradient_of(graph, Param::BaseColor),
        Some(wood),
        "the ramp did not survive the round trip"
    );

    // A scalar parameter gets no ramp: one number is the whole answer there,
    // and a ramp would be a control with nothing to do.
    bus.apply(
        &mut document,
        Command::SetParameterProcedural {
            material: key,
            param: Param::SpecularRoughness,
            procedural: Some(Box::new(Procedural::default())),
            seed: 17,
        },
    )
    .expect("attach");
    let graph = &document.materials.get(key).unwrap().graph;
    assert!(
        tequila_scene::command::gradient_of(graph, Param::SpecularRoughness).is_none(),
        "a roughness was given a colour ramp"
    );
}

/// A CAD source kept with the project is saved while a model names it and
/// read back as it was; one no model names any more is left out.
#[test]
fn kept_cad_sources_survive_a_round_trip_while_named() {
    use tequila_scene::document::{Sources, BREP};
    let mut document = SceneDocument::new();
    let named = b"ISO-10303-21; named".to_vec();
    let orphan = b"ISO-10303-21; orphan".to_vec();
    let (named_id, orphan_id) = (Sources::id_of(&named), Sources::id_of(&orphan));
    document.sources.insert(named_id.clone(), std::sync::Arc::new(named.clone()));
    document.sources.insert(orphan_id.clone(), std::sync::Arc::new(orphan));
    let mut bus = tequila_scene::CommandBus::new();
    bus.apply(&mut document, tequila_scene::Command::AddGroup { parent: None, name: "Model".into() }).unwrap();
    let group = document.tree.roots()[0];
    bus.apply(&mut document, tequila_scene::Command::SetMetadata { node: group, key: BREP.into(), value: Some(named_id.clone()) }).unwrap();

    // An IGES file, kept as one.
    let iges = format!("{:<72}S{:>7}\n", "IGES source", 1).into_bytes();
    let iges_id = Sources::id_of(&iges);
    document.sources.insert(iges_id.clone(), std::sync::Arc::new(iges.clone()));
    bus.apply(&mut document, tequila_scene::Command::AddGroup { parent: None, name: "Surfaces".into() }).unwrap();
    let second = document.tree.roots()[1];
    bus.apply(&mut document, tequila_scene::Command::SetMetadata { node: second, key: BREP.into(), value: Some(iges_id.clone()) }).unwrap();

    let reopened = read(&write(&document, &Preserved::default())).expect("the file we just wrote must read");
    assert_eq!(reopened.document.sources.get(&named_id).map(|b| b.as_slice()), Some(named.as_slice()));
    assert_eq!(reopened.document.sources.get(&iges_id).map(|b| b.as_slice()), Some(iges.as_slice()));
    assert!(reopened.document.sources.get(&orphan_id).is_none(), "a source no model names was kept");
    assert!(reopened.preserved.is_empty(), "the source came back as an unknown member");
}
