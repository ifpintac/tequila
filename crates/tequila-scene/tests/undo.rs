//! Undo/redo integrity.
//!
//! Half of M1's exit criterion: "undo/redo across 50 mixed operations leaves
//! the document identical to the start."
//!
//! Written as an integration test rather than a unit test because that is the
//! level the claim lives at — it is about the document as a whole, not about
//! any one command.
//!
//! "Identical" is [`SceneDocument::describes_the_same_scene_as`], not `==`.
//! The two differ in one respect only: a document carries its ID allocators,
//! and those are monotonic on purpose — §2.3 forbids reusing an ID, so undo
//! cannot rewind them. Two documents that differ only in how many IDs have
//! been spent are the same scene. That the counters *do* advance is itself
//! asserted below, so the no-reuse guarantee stays visible.

use tequila_core::math::{vec3, Transform};
use tequila_core::units::LengthUnit;
use tequila_render_core::settings::LightingPreset;
use tequila_materials::material_type::Param;
use tequila_scene::command::{
    EnvParam,
    reserve_keys, Command, EnvironmentParameter, ImportPayload, MaterialParameter,
};
use tequila_scene::geometry::Mesh;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneNode};
use tequila_scene::units::ConversionMode;
use tequila_scene::{CommandBus, SceneDocument};

/// A document with a small assembly, built the way an importer builds one —
/// through `Command::Import` — so the test never reaches around the command
/// bus to set up its own fixture.
fn assembly() -> (SceneDocument, Vec<NodeId>, MeshKey, MaterialKey) {
    let mut doc = SceneDocument::new();
    let mut bus = CommandBus::new();

    let mesh = Mesh::new(
        vec![
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
        ],
        vec![0, 1, 2],
    );
    let (meshes, materials, ids) = reserve_keys(&doc, 1, 1, 5);
    let (mesh_key, material) = (meshes[0], materials[0]);

    let mut nodes = vec![SceneNode::new(ids[0], "Assembly", NodeKind::Group)];
    for (i, &id) in ids.iter().enumerate().skip(1) {
        let mut node = SceneNode::new(
            id,
            format!("Part {}", i - 1),
            NodeKind::Part { mesh: mesh_key },
        );
        node.parent = Some(ids[0]);
        node.material = Some(material);
        nodes[0].children.push(id);
        nodes.push(node);
    }

    bus.apply(
        &mut doc,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(mesh_key, mesh)],
                textures: Vec::new(),
                materials: vec![(material, MaterialDefinition::plastic("Base"))],
                nodes,
                roots: vec![ids[0]],
            }),
        },
    )
    .unwrap();
    doc.validate().unwrap();
    (doc, ids, mesh_key, material)
}

/// Fifty mixed operations, deterministic and covering every command that has
/// an inverse.
fn fifty_operations(ids: &[NodeId], material: MaterialKey) -> Vec<Command> {
    let mut commands = Vec::new();
    for i in 0..50u32 {
        let node = ids[1 + (i as usize % 4)];
        commands.push(match i % 10 {
            0 => Command::Rename {
                node,
                name: format!("renamed {i}"),
            },
            1 => Command::set_transform(node, Transform::translation(vec3(i as f32, 0.0, 0.0))),
            2 => Command::set_visibility(node, i % 20 == 2),
            3 => Command::SetMetadata {
                node,
                key: "PartNumber".into(),
                value: Some(format!("HS-{i}")),
            },
            4 => Command::AddGroup {
                parent: Some(ids[0]),
                name: format!("group {i}"),
            },
            5 => Command::SetMaterialParameter {
                material,
                parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.1 + i as f32 * 0.01),
            },
            6 => Command::SetEnvironmentParameter {
                parameter: EnvironmentParameter::scalar(EnvParam::Rotation, i as f32 * 0.01),
            },
            7 => Command::set_locked(node, false),
            8 => Command::RenameMaterial {
                material,
                name: format!("Base {i}"),
            },
            _ => Command::SetLightingPreset {
                preset: if i % 20 == 9 {
                    LightingPreset::Interior
                } else {
                    LightingPreset::Product
                },
            },
        });
    }
    commands
}

/// Assert two documents describe the same scene, reporting *what* differs.
/// A bare `assert_eq!` on two documents prints both in full and names no
/// field, which is not a usable failure message for a fifty-node assembly.
#[track_caller]
fn same_scene(actual: &SceneDocument, expected: &SceneDocument, context: &str) {
    if let Some(difference) = actual.difference_from(expected) {
        panic!("{context}: {difference}");
    }
}

#[track_caller]
fn different_scene(actual: &SceneDocument, expected: &SceneDocument, context: &str) {
    assert!(actual.difference_from(expected).is_some(), "{context}");
}

/// The exit criterion, stated directly.
#[test]
fn fifty_operations_undo_back_to_the_starting_document() {
    let (mut doc, ids, _, material) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    for command in fifty_operations(&ids, material) {
        bus.apply(&mut doc, command).unwrap();
    }
    different_scene(&doc, &start, "the operations did not change anything");

    while bus.can_undo() {
        bus.undo(&mut doc).unwrap();
    }
    same_scene(&doc, &start, "after undoing every operation");
    doc.validate().unwrap();
}

/// And redo must take it back to exactly where it was.
#[test]
fn redoing_everything_restores_the_edited_document() {
    let (mut doc, ids, _, material) = assembly();
    let mut bus = CommandBus::new();

    for command in fifty_operations(&ids, material) {
        bus.apply(&mut doc, command).unwrap();
    }
    let edited = doc.clone();

    while bus.can_undo() {
        bus.undo(&mut doc).unwrap();
    }
    while bus.can_redo() {
        bus.redo(&mut doc).unwrap();
    }
    same_scene(&doc, &edited, "after redoing every operation");
    doc.validate().unwrap();
}

/// Undo and redo alternating, which is what a user actually does. A history
/// that only survives a single sweep in each direction is not much use.
#[test]
fn interleaved_undo_and_redo_stays_consistent() {
    let (mut doc, ids, _, material) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    for command in fifty_operations(&ids, material) {
        bus.apply(&mut doc, command).unwrap();
    }
    let edited = doc.clone();

    for _ in 0..12 {
        for _ in 0..7 {
            bus.undo(&mut doc).unwrap();
        }
        for _ in 0..5 {
            bus.redo(&mut doc).unwrap();
        }
        doc.validate().unwrap();
    }
    while bus.can_undo() {
        bus.undo(&mut doc).unwrap();
    }
    same_scene(&doc, &start, "after unwinding an interleaved history");
    while bus.can_redo() {
        bus.redo(&mut doc).unwrap();
    }
    same_scene(&doc, &edited, "after replaying an interleaved history");
}

/// Deleting a subtree and undoing must restore it with the *same* node IDs, or
/// every reference into it — materials, animation tracks, selection — dangles.
#[test]
fn deleting_and_undoing_restores_identical_nodes() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::DeleteNodes {
            nodes: vec![ids[0]],
        },
    )
    .unwrap();
    assert!(doc.tree.is_empty());

    bus.undo(&mut doc).unwrap();
    same_scene(
        &doc,
        &start,
        "the restored subtree differs from the original",
    );
    doc.validate().unwrap();
}

/// An import under a parent that is not there is refused, and leaves
/// nothing behind: no orphan node, no meshes, no materials.
#[test]
fn an_import_under_a_missing_parent_changes_nothing() {
    let (mut doc, _, _, _) = assembly();
    let before = doc.clone();
    let mesh = Mesh::new(vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
    let (meshes, materials, ids) = reserve_keys(&doc, 1, 1, 1);
    let mut node = SceneNode::new(ids[0], "Stray", NodeKind::Part { mesh: meshes[0] });
    node.parent = Some(NodeId(999_999));
    node.material = Some(materials[0]);
    let mut bus = CommandBus::new();
    let refused = bus.apply(
        &mut doc,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], mesh)],
                textures: Vec::new(),
                materials: vec![(materials[0], MaterialDefinition::plastic("Stray"))],
                nodes: vec![node],
                roots: vec![ids[0]],
            }),
        },
    );
    assert!(refused.is_err());
    same_scene(&doc, &before, "a refused import changed the document");
    assert_eq!(doc.tree.iter().count(), before.tree.iter().count(), "an orphan was left in the tree");
    assert!(doc.geometry.get(meshes[0]).is_none(), "its mesh was left behind");
}

/// An undone delete of a group with parts in it puts the group back where
/// it was among its siblings, not after them.
#[test]
fn an_undone_group_delete_keeps_its_place() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();
    for name in ["H", "I"] {
        bus.apply(&mut doc, Command::AddGroup { parent: None, name: name.into() }).unwrap();
    }
    let order = doc.tree.roots().to_vec();
    assert_eq!(order[0], ids[0], "the assembly comes first");

    bus.apply(&mut doc, Command::DeleteNodes { nodes: vec![ids[0]] }).unwrap();
    bus.undo(&mut doc).unwrap();
    assert_eq!(doc.tree.roots(), order.as_slice());
    doc.validate().unwrap();
}

/// Grouping and ungrouping are inverses, including the position the group
/// occupied among its siblings.
#[test]
fn grouping_and_ungrouping_round_trip() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::Group {
            nodes: vec![ids[2], ids[3]],
            name: "Sub".into(),
        },
    )
    .unwrap();
    doc.validate().unwrap();
    let grouped = doc.clone();
    different_scene(&grouped, &start, "grouping changed nothing");

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "after undoing a group");

    bus.redo(&mut doc).unwrap();
    same_scene(&doc, &grouped, "after redoing a group");

    // And the explicit ungroup command must do the same.
    let group = doc
        .tree
        .iter()
        .find(|n| n.name == "Sub")
        .expect("the group exists")
        .id;
    bus.apply(&mut doc, Command::Ungroup { group }).unwrap();
    doc.validate().unwrap();
    let ungrouped = doc.clone();
    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &grouped, "after undoing an ungroup");
    bus.redo(&mut doc).unwrap();
    same_scene(&doc, &ungrouped, "after redoing an ungroup");
}

/// Unlinking creates a material; undoing must remove it again, not leave an
/// orphan behind in the library.
#[test]
fn unlinking_and_undoing_leaves_no_orphan_material() {
    let (mut doc, ids, _, material) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.apply(&mut doc, Command::UnlinkMaterial { node: ids[1] })
        .unwrap();
    assert_eq!(doc.materials.len(), 2);
    assert_ne!(doc.tree.get(ids[1]).unwrap().material, Some(material));

    bus.undo(&mut doc).unwrap();
    assert_eq!(doc.materials.len(), 1, "the unlinked copy was left behind");
    same_scene(&doc, &start, "after undoing an unlink");
}

/// Dragging a slider must leave one undo entry, not two hundred (§8.1).
#[test]
fn continuous_parameter_edits_coalesce_into_one_step() {
    let (mut doc, _, _, material) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    for i in 0..200 {
        bus.apply(
            &mut doc,
            Command::SetMaterialParameter {
                material,
                parameter: MaterialParameter::scalar(Param::SpecularRoughness, i as f32 / 200.0),
            },
        )
        .unwrap();
    }
    assert_eq!(
        bus.history_names().len(),
        1,
        "a slider drag produced {} undo entries",
        bus.history_names().len()
    );

    // One undo must return all the way to before the drag.
    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "after undoing a coalesced drag");
}

/// Different parameters must *not* coalesce, or a user cannot undo just the
/// second of two deliberate edits.
#[test]
fn distinct_edits_stay_separate_undo_steps() {
    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.5),
        },
    )
    .unwrap();
    bus.apply(
        &mut doc,
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularIor, 1.7),
        },
    )
    .unwrap();
    assert_eq!(bus.history_names().len(), 2);

    bus.undo(&mut doc).unwrap();
    assert!((doc.materials.get(material).unwrap().material().specular_ior - 1.5).abs() < 1e-6);
    assert!((doc.materials.get(material).unwrap().material().specular_roughness - 0.5).abs() < 1e-6);
}

/// A new edit after an undo discards the redo branch — it no longer follows
/// from the current state.
#[test]
fn a_new_edit_after_undo_clears_the_redo_branch() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::Rename {
            node: ids[1],
            name: "first".into(),
        },
    )
    .unwrap();
    bus.apply(
        &mut doc,
        Command::Rename {
            node: ids[2],
            name: "second".into(),
        },
    )
    .unwrap();
    bus.undo(&mut doc).unwrap();
    assert!(bus.can_redo());

    bus.apply(
        &mut doc,
        Command::Rename {
            node: ids[3],
            name: "third".into(),
        },
    )
    .unwrap();
    assert!(!bus.can_redo(), "the stale redo branch survived a new edit");
    assert_eq!(doc.tree.get(ids[3]).unwrap().name, "third");
    assert_eq!(doc.tree.get(ids[2]).unwrap().name, "Part 1");
}

/// Locking exists to refuse edits. A locked node must reject them, and the
/// refusal must not half-apply anything.
#[test]
fn locked_nodes_refuse_edits_without_side_effects() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(&mut doc, Command::set_locked(ids[0], true))
        .unwrap();
    let locked = doc.clone();

    // A child of a locked group inherits the lock.
    let error = bus
        .apply(
            &mut doc,
            Command::Rename {
                node: ids[1],
                name: "nope".into(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, tequila_scene::CommandError::Locked(_)));
    same_scene(&doc, &locked, "a refused rename still changed the document");

    // Deleting a locked subtree is refused as a whole, leaving nothing removed.
    assert!(bus
        .apply(
            &mut doc,
            Command::DeleteNodes {
                nodes: vec![ids[1], ids[2]]
            }
        )
        .is_err());
    same_scene(&doc, &locked, "a refused delete still changed the document");
}

/// A unit change is one undoable step, per §3.3.
#[test]
fn a_unit_conversion_undoes_as_a_single_step() {
    let (mut doc, _, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::SetUnits {
            unit: LengthUnit::Millimetre,
            mode: ConversionMode::KeepSceneSize,
        },
    )
    .unwrap();
    assert_eq!(doc.units.unit, LengthUnit::Millimetre);
    assert_eq!(bus.history_names(), vec!["scene.set_units"]);

    bus.undo(&mut doc).unwrap();
    assert_eq!(doc, start);
}

/// Every command must produce a dirty classification, and a refused command
/// must not be journalled — replaying the journal would then diverge.
#[test]
fn refused_commands_are_not_journalled() {
    let (mut doc, _, _, _) = assembly();
    let mut bus = CommandBus::new();
    let before = bus.journal().len();

    assert!(bus
        .apply(
            &mut doc,
            Command::Rename {
                node: NodeId(9999),
                name: "ghost".into()
            }
        )
        .is_err());
    assert_eq!(bus.journal().len(), before);
}

/// A command that is not sampled below cannot be checked against the
/// catalog, and six were missing for months without anything noticing.
///
/// This match has no wildcard arm, so adding a variant to `Command` stops the
/// tests compiling until it is listed here — and the comment on each arm is
/// where to go next: add a sample to the catalog test below.
#[allow(dead_code)]
fn every_variant_must_be_sampled(command: &Command) {
    match command {
        Command::Import { .. }
        | Command::Reimport { .. }
        | Command::AddGroup { .. }
        | Command::DeleteNodes { .. }
        | Command::SplitIntoParts { .. }
        | Command::Rename { .. }
        | Command::SetTransform { .. }
        | Command::SetVisibility { .. }
        | Command::SetLocked { .. }
        | Command::SetPivot { .. }
        | Command::Reparent { .. }
        | Command::Group { .. }
        | Command::Ungroup { .. }
        | Command::SetMetadata { .. }
        | Command::SetUnits { .. }
        | Command::AssignMaterial { .. }
        | Command::UnlinkMaterial { .. }
        | Command::SetMaterialParameter { .. }
        | Command::AddDecal { .. }
        | Command::SetDecals { .. }
        | Command::SetMaterialType { .. }
        | Command::SetParameterTexture { .. }
        | Command::SetParameterProcedural { .. }
        | Command::SetParameterGradient { .. }
        | Command::SetTextureMapping { .. }
        | Command::RenameMaterial { .. }
        | Command::SetGraph { .. }
        | Command::SetLightingPreset { .. }
        | Command::SetLightingConfig { .. }
        | Command::SetEnvironmentParameter { .. }
        | Command::SetEnvironmentPins { .. }
        | Command::SetSky { .. }
        | Command::AddLight { .. }
        | Command::SetLight { .. }
        | Command::AddCutaway { .. }
        | Command::SetCutaway { .. }
        | Command::SetMeasurements { .. }
        | Command::SetMultiMaterials { .. }
        | Command::SetModelSets { .. }
        | Command::SetStudios { .. }
        | Command::ApplyStudio { .. }
        | Command::SetConfigurator { .. }
        | Command::Configure { .. }
        | Command::SetMaterialTemplates { .. }
        | Command::AddCamera { .. }
        | Command::RemoveCamera { .. }
        | Command::SelectCamera { .. }
        | Command::SetCamera { .. }
        | Command::RenameCamera { .. }
        | Command::SetColorManagement { .. }
        | Command::SetRenderSettings { .. }
        | Command::SetImageStyle { .. } => {}
    }
}

/// The catalog must stay in step with the enum. `docs/05` §3.8 checks each
/// entry is reachable from the UI and from Python; that check is worthless if
/// the list itself has drifted.
#[test]
fn the_catalog_lists_every_command_exactly_once() {
    let mut sorted = Command::CATALOG.to_vec();
    sorted.sort_unstable();
    let mut deduped = sorted.clone();
    deduped.dedup();
    assert_eq!(sorted, deduped, "the catalog contains a duplicate");

    // Every constructed command's name must appear in the catalog.
    let samples = [
        Command::AddDecal { material: MaterialKey(1), name: "Logo".into(), source: tequila_scene::textures::TextureSource::new("logo.png", tequila_texture::texture::MapRole::Color) },
        Command::SetDecals { material: MaterialKey(1), decals: Vec::new() },
        Command::AddGroup {
            parent: None,
            name: String::new(),
        },
        Command::DeleteNodes { nodes: vec![] },
        Command::SplitIntoParts { node: NodeId(1) },
        Command::Reimport {
            payload: Box::new(tequila_scene::reimport::ReimportPayload {
                added: tequila_scene::command::ImportPayload {
                    meshes: Vec::new(),
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: Vec::new(),
                    roots: Vec::new(),
                },
                updates: Vec::new(),
                hidden: Vec::new(),
            }),
        },
        Command::Rename {
            node: NodeId(1),
            name: String::new(),
        },
        Command::set_transform(NodeId(1), Transform::IDENTITY),
        Command::set_visibility(NodeId(1), true),
        Command::set_locked(NodeId(1), true),
        Command::Reparent {
            node: NodeId(1),
            parent: None,
            index: None,
        },
        Command::Group {
            nodes: vec![],
            name: String::new(),
        },
        Command::Ungroup { group: NodeId(1) },
        Command::SetMetadata {
            node: NodeId(1),
            key: String::new(),
            value: None,
        },
        Command::SetUnits {
            unit: LengthUnit::Metre,
            mode: ConversionMode::ScaleScene,
        },
        Command::AssignMaterial {
            nodes: vec![],
            material: MaterialKey(1),
        },
        Command::set_pivot(NodeId(1), Some(vec3(0.0, 1.0, 0.0))),
        Command::UnlinkMaterial { node: NodeId(1) },
        Command::SetMaterialParameter {
            material: MaterialKey(1),
            parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.0),
        },
        Command::RenameMaterial {
            material: MaterialKey(1),
            name: String::new(),
        },
        Command::SetGraph {
            material: MaterialKey(1),
            graph: Box::new(tequila_materials::graph::MaterialGraph::from_material(Default::default())),
        },
        Command::SetLightingPreset {
            preset: LightingPreset::Product,
        },
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::scalar(EnvParam::Rotation, 0.0),
        },
        Command::SetMaterialType {
            material: MaterialKey(1),
            material_type: tequila_materials::material_type::MaterialType::Velvet,
        },
        Command::SetParameterTexture {
            material: MaterialKey(1),
            param: Param::BaseColor,
            source: None,
        },
        Command::SetParameterProcedural {
            material: MaterialKey(1),
            param: Param::BaseColor,
            procedural: None,
            seed: 0,
        },
        Command::SetParameterGradient {
            material: MaterialKey(1),
            param: Param::BaseColor,
            gradient: Box::default(),
        },
        Command::SetTextureMapping {
            material: MaterialKey(1),
            param: Param::BaseColor,
            mapping: Box::default(),
        },
        Command::SetEnvironmentPins { pins: Vec::new() },
        Command::SetSky { sky: Box::new(None) },
        Command::SetLightingConfig {
            config: Box::new(tequila_render_core::settings::IntegratorConfig::interior()),
        },
        Command::AddLight {
            parent: None,
            name: "Key".into(),
            settings: Box::new(tequila_scene::lighting::LightSettings::default()),
        },
        Command::SetLight {
            node: NodeId(1),
            settings: Box::new(tequila_scene::lighting::LightSettings::default()),
        },
        Command::AddCutaway {
            parent: None,
            name: "Section".into(),
            settings: Box::default(),
            transform: tequila_core::math::Transform::IDENTITY,
        },
        Command::SetCutaway {
            node: NodeId(1),
            settings: Box::default(),
        },
        Command::SetMeasurements {
            measurements: Vec::new(),
        },
        Command::SetMultiMaterials {
            multi_materials: Vec::new(),
        },
        Command::SetModelSets {
            model_sets: Box::default(),
        },
        Command::SetStudios { studios: Vec::new() },
        Command::ApplyStudio { index: 0 },
        Command::SetConfigurator { configurator: Box::default() },
        Command::Configure { choices: Vec::new() },
        Command::SetMaterialTemplates { templates: Vec::new() },
        Command::AddCamera {
            camera: Box::new(tequila_scene::lighting::SavedCamera {
                name: "Hero".into(),
                camera: Default::default(),
                target: tequila_core::math::Vec3::ZERO,
            }),
        },
        Command::RemoveCamera { index: 1 },
        Command::SelectCamera { index: 0 },
        Command::SetCamera {
            index: 0,
            camera: Box::new(tequila_scene::lighting::SavedCamera {
                name: "Hero".into(),
                camera: Default::default(),
                target: tequila_core::math::Vec3::ZERO,
            }),
        },
        Command::RenameCamera {
            index: 0,
            name: "Hero".into(),
        },
        Command::SetImageStyle {
            style: Box::new(tequila_color::ImageStyle::default()),
        },
        Command::SetColorManagement { selection: None },
        Command::SetRenderSettings {
            settings: Box::new(tequila_scene::render::RenderSettings::default()),
        },
    ];
    for command in &samples {
        assert!(
            Command::CATALOG.contains(&command.name()),
            "{} is missing from the catalog",
            command.name()
        );
    }
    // `scene.import` is the one not constructed above; check it explicitly so
    // the count still lines up.
    assert!(Command::CATALOG.contains(&"scene.import"));
    assert_eq!(Command::CATALOG.len(), samples.len() + 1);
}

/// Assigning a material to several parts is one step, and it must record each
/// part's previous material rather than a single shared one.
#[test]
fn assigning_to_many_nodes_undoes_each_to_its_own_previous_material() {
    let (mut doc, ids, _, base) = assembly();
    let mut bus = CommandBus::new();
    // Give one part its own material, so undo has something distinct to
    // restore per node rather than one shared previous value.
    bus.apply(&mut doc, Command::UnlinkMaterial { node: ids[2] })
        .unwrap();
    bus.mark_saved();
    let start = doc.clone();

    bus.apply(
        &mut doc,
        Command::AssignMaterial {
            nodes: vec![ids[1], ids[2], ids[3]],
            material: base,
        },
    )
    .unwrap();
    assert!(doc
        .tree
        .iter()
        .filter(|n| n.material.is_some())
        .all(|n| n.material == Some(base)));

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "each node must return to its own material");
}

/// The other half of the equality story, asserted directly so it cannot rot.
///
/// §2.3: an ID is never reused. Undo therefore restores the *scene* without
/// rewinding the allocators, and a node created after an undo gets a fresh ID
/// rather than the one just handed back. If it did not, the material
/// assignment or animation track still pointing at the old ID would silently
/// re-attach to an unrelated part.
#[test]
fn undo_restores_the_scene_without_reusing_identifiers() {
    let (mut doc, ids, _, material) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::AddGroup {
            parent: None,
            name: "Doomed".into(),
        },
    )
    .unwrap();
    let doomed = doc
        .tree
        .iter()
        .find(|n| n.name == "Doomed")
        .expect("the group was added")
        .id;
    bus.apply(&mut doc, Command::UnlinkMaterial { node: ids[1] })
        .unwrap();

    let spent_nodes = doc.tree.next_id();
    let spent_materials = doc.materials.next_key();

    bus.undo(&mut doc).unwrap();
    bus.undo(&mut doc).unwrap();

    // The scene is back.
    same_scene(&doc, &start, "after undoing an add and an unlink");

    // The watermarks are not, and must not be.
    assert_eq!(
        doc.tree.next_id(),
        spent_nodes,
        "undo rewound the node allocator"
    );
    assert_eq!(
        doc.materials.next_key(),
        spent_materials,
        "undo rewound the material allocator"
    );

    // Which is the point: the next node is not the one just freed.
    bus.apply(
        &mut doc,
        Command::AddGroup {
            parent: None,
            name: "Replacement".into(),
        },
    )
    .unwrap();
    let replacement = doc
        .tree
        .iter()
        .find(|n| n.name == "Replacement")
        .expect("the group was added")
        .id;
    assert_ne!(
        replacement, doomed,
        "a freed ID was reissued to a different node"
    );

    bus.apply(&mut doc, Command::UnlinkMaterial { node: ids[1] })
        .unwrap();
    assert!(
        doc.materials
            .keys()
            .all(|key| key.0 != material.0 || key == material),
        "the material store reissued a key"
    );
    assert!(
        doc.materials.next_key() > spent_materials,
        "the material allocator did not advance"
    );
}

/// Redo must re-create a node under the ID it had, not a fresh one.
///
/// The whole-document tests catch this too, but only as "the hierarchy
/// differs". This states the consequence directly: anything holding a NodeId
/// across an undo — a selection, a material assignment, an animation track —
/// is pointing at a node that redo has to bring back as *itself*. Allocating a
/// fresh ID instead renumbers the scene silently, and the reference dangles.
#[test]
fn redo_re_creates_nodes_under_their_original_identifiers() {
    let (mut doc, ids, _, material) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::Group {
            nodes: vec![ids[2], ids[3]],
            name: "Fasteners".into(),
        },
    )
    .unwrap();
    let group = doc
        .tree
        .iter()
        .find(|n| n.name == "Fasteners")
        .expect("the group exists")
        .id;

    // A reference into the group, of the kind that has to survive.
    bus.apply(
        &mut doc,
        Command::AssignMaterial {
            nodes: vec![group],
            material,
        },
    )
    .unwrap();

    bus.undo(&mut doc).unwrap();
    bus.undo(&mut doc).unwrap();
    assert!(!doc.tree.contains(group), "undo left the group behind");

    bus.redo(&mut doc).unwrap();
    assert!(
        doc.tree.contains(group),
        "redo created a different node instead of restoring {group}"
    );
    assert_eq!(
        doc.tree.get(group).unwrap().name,
        "Fasteners",
        "redo reused the ID for something else"
    );

    // And the assignment recorded against that ID still lands on it.
    bus.redo(&mut doc).unwrap();
    assert_eq!(
        doc.tree.get(group).unwrap().material,
        Some(material),
        "the material assignment was lost across undo and redo"
    );
    doc.validate().unwrap();
}

/// The same guarantee for the material an unlink creates.
#[test]
fn redo_re_creates_unlinked_materials_under_their_original_key() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(&mut doc, Command::UnlinkMaterial { node: ids[1] })
        .unwrap();
    let created = doc.tree.get(ids[1]).unwrap().material.expect("unlinked");

    bus.undo(&mut doc).unwrap();
    assert!(doc.materials.get(created).is_none(), "undo kept the copy");

    bus.redo(&mut doc).unwrap();
    assert_eq!(
        doc.tree.get(ids[1]).unwrap().material,
        Some(created),
        "redo unlinked into a different key"
    );
    assert!(doc.materials.get(created).is_some());
    doc.validate().unwrap();
}

/// Moving a selection is one action, and has to be one undo step whether it
/// touched one part or forty.
#[test]
fn transforming_many_nodes_is_a_single_undo_step() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    let moved: Vec<(NodeId, Transform)> = ids[1..4]
        .iter()
        .map(|&id| (id, Transform::translation(vec3(5.0, 0.0, 0.0))))
        .collect();
    bus.apply(&mut doc, Command::SetTransform { transforms: moved })
        .unwrap();

    for &id in &ids[1..4] {
        assert_eq!(
            doc.tree.get(id).unwrap().transform,
            Transform::translation(vec3(5.0, 0.0, 0.0))
        );
    }
    assert_eq!(bus.history_names().len(), 1, "one drag, one step");

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "after undoing a multi-part move");
}

/// Each node returns to *its own* previous transform, not to a shared one.
#[test]
fn undoing_a_multi_node_move_restores_each_nodes_own_transform() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    // Give them different starting places first.
    let spread: Vec<(NodeId, Transform)> = ids[1..4]
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, Transform::translation(vec3(i as f32, 0.0, 0.0))))
        .collect();
    bus.apply(&mut doc, Command::SetTransform { transforms: spread })
        .unwrap();
    bus.end_interaction();
    let spread_out = doc.clone();

    let together: Vec<(NodeId, Transform)> = ids[1..4]
        .iter()
        .map(|&id| (id, Transform::translation(vec3(9.0, 9.0, 9.0))))
        .collect();
    bus.apply(
        &mut doc,
        Command::SetTransform {
            transforms: together,
        },
    )
    .unwrap();
    bus.undo(&mut doc).unwrap();

    same_scene(&doc, &spread_out, "each node must return to its own place");
}

/// A command that moved four parts and then refused on the fifth would leave
/// the document in a state the user never asked for and cannot undo, because
/// a refusal produces no undo record.
#[test]
fn a_locked_node_refuses_the_whole_multi_node_transform() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();
    bus.apply(&mut doc, Command::set_locked(ids[3], true))
        .unwrap();
    let locked = doc.clone();

    let transforms: Vec<(NodeId, Transform)> = ids[1..5]
        .iter()
        .map(|&id| (id, Transform::translation(vec3(5.0, 0.0, 0.0))))
        .collect();
    assert!(bus
        .apply(&mut doc, Command::SetTransform { transforms })
        .is_err());

    same_scene(&doc, &locked, "a refused move changed part of the document");
    assert_eq!(bus.journal().len(), 1, "a refused command was journalled");
}

/// Isolate and Hide Others hide everything but a handful. One command, so one
/// undo step — on a 4,000-part assembly the alternative is unusable.
#[test]
fn hiding_many_nodes_undoes_in_one_step() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::set_visibility_of(ids[1..].to_vec(), false),
    )
    .unwrap();
    assert!(ids[1..]
        .iter()
        .all(|&id| !doc.tree.get(id).unwrap().visible));

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "after undoing a bulk hide");
    assert!(!bus.can_undo(), "it should have been a single step");
}

/// Undo has to restore each node's own flag, not set them all back to one
/// value — some of them may have been hidden already.
#[test]
fn undoing_a_bulk_hide_restores_nodes_that_were_already_hidden() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();
    bus.apply(&mut doc, Command::set_visibility(ids[2], false))
        .unwrap();
    let mixed = doc.clone();

    bus.apply(&mut doc, Command::set_visibility_of(ids.clone(), true))
        .unwrap();
    bus.undo(&mut doc).unwrap();

    same_scene(
        &doc,
        &mixed,
        "the already-hidden node was un-hidden by undo",
    );
}

/// A drag merges frame to frame, but only while the same parts are being
/// moved. Merging across selections would fold two deliberate edits into one.
#[test]
fn transforms_merge_within_a_drag_and_not_across_selections() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    let drag = |at: f32, nodes: &[NodeId]| Command::SetTransform {
        transforms: nodes
            .iter()
            .map(|&id| (id, Transform::translation(vec3(at, 0.0, 0.0))))
            .collect(),
    };

    for step in 1..=5 {
        bus.apply(&mut doc, drag(step as f32, &ids[1..3])).unwrap();
    }
    assert_eq!(bus.history_names().len(), 1, "a drag is one step");

    // A different set of parts is a different edit.
    bus.apply(&mut doc, drag(1.0, &ids[3..5])).unwrap();
    assert_eq!(bus.history_names().len(), 2);

    // And the same set again resumes merging.
    bus.apply(&mut doc, drag(2.0, &ids[3..5])).unwrap();
    assert_eq!(bus.history_names().len(), 2);
}

/// §8.1's merge policy exists so "dragging a slider produces one undo entry,
/// not two hundred". It must not also swallow two deliberate edits — typing a
/// position, then typing another, is two things to undo.
#[test]
fn two_deliberate_edits_of_one_value_stay_separate() {
    let (mut doc, ids, _, material) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::set_transform(ids[1], Transform::translation(vec3(1.0, 0.0, 0.0))),
    )
    .unwrap();
    bus.end_interaction();
    bus.apply(
        &mut doc,
        Command::set_transform(ids[1], Transform::translation(vec3(2.0, 0.0, 0.0))),
    )
    .unwrap();

    assert_eq!(bus.history_names().len(), 2, "the first edit was swallowed");
    bus.undo(&mut doc).unwrap();
    assert_eq!(
        doc.tree.get(ids[1]).unwrap().transform,
        Transform::translation(vec3(1.0, 0.0, 0.0)),
        "undo skipped a step"
    );

    // The same for a material parameter, which is the case §8.1 names.
    bus.apply(
        &mut doc,
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.2),
        },
    )
    .unwrap();
    bus.end_interaction();
    bus.apply(
        &mut doc,
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.8),
        },
    )
    .unwrap();
    assert_eq!(bus.history_names().len(), 3);
}

/// And a command applied after an undo must not merge backwards into the
/// entry the undo just stepped over.
#[test]
fn an_edit_after_an_undo_does_not_merge_into_the_step_before_it() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::set_transform(ids[1], Transform::translation(vec3(1.0, 0.0, 0.0))),
    )
    .unwrap();
    bus.apply(
        &mut doc,
        Command::set_transform(ids[2], Transform::translation(vec3(1.0, 0.0, 0.0))),
    )
    .unwrap();
    bus.undo(&mut doc).unwrap();

    // This truncates the redo branch and must start a fresh step.
    bus.apply(
        &mut doc,
        Command::set_transform(ids[1], Transform::translation(vec3(7.0, 0.0, 0.0))),
    )
    .unwrap();
    assert_eq!(bus.history_names().len(), 2);

    bus.undo(&mut doc).unwrap();
    assert_eq!(
        doc.tree.get(ids[1]).unwrap().transform,
        Transform::translation(vec3(1.0, 0.0, 0.0)),
        "the edit merged into the step before the undo"
    );
}

/// "Isolate" both shows and hides, so a per-node flag is what lets it be one
/// command. Two commands would make the first Ctrl+Z appear to do nothing.
#[test]
fn isolating_shows_and_hides_in_a_single_step() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    // Hide something first, so undo has a mixed state to restore rather than
    // a uniform one it could fake.
    bus.apply(&mut doc, Command::set_visibility(ids[4], false))
        .unwrap();
    bus.end_interaction();
    let before = doc.clone();

    // Isolate ids[1]: it and its ancestors stay, everything else goes.
    let keep = ids[1];
    let states: Vec<(NodeId, bool)> = doc
        .tree
        .iter()
        .map(|node| {
            let visible = node.id == keep || doc.tree.ancestors(keep).contains(&node.id);
            (node.id, visible)
        })
        .collect();
    bus.apply(&mut doc, Command::SetVisibility { states })
        .unwrap();

    assert!(doc.tree.is_effectively_visible(keep));
    assert!(!doc.tree.is_effectively_visible(ids[3]));
    assert_eq!(bus.history_names().len(), 2, "isolate must be one step");

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &before, "one undo did not fully reverse isolate");

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "after undoing the earlier hide too");
}

/// Every control the Material tab can show must actually drive an edit.
///
/// The panel is generated from `MaterialType::exposed()`, so a parameter can
/// appear in it purely by being listed there. This walks the same list the
/// panel walks — all thirty-four types, every parameter each one exposes — and
/// requires that the command bus accepts it, that it changes the material, and
/// that undo puts it back.
///
/// A parameter that is exposed but not settable would render as a live-looking
/// control that silently does nothing, which is indistinguishable to the user
/// from a broken application.
#[test]
fn every_parameter_a_type_exposes_can_be_set_and_undone() {
    use tequila_materials::material_type::{MaterialType, ParamUi, ParamValue};

    for material_type in MaterialType::ALL {
        let mut doc = SceneDocument::new();
        let mut bus = CommandBus::new();
        let key = doc
            .materials
            .insert(MaterialDefinition::preset("Under test", material_type));

        for param in material_type.exposed() {
            // A texture slot has no value and the panel shows it disabled.
            if param.ui() == ParamUi::Slot {
                continue;
            }
            let before = doc.materials.get(key).cloned().expect("a definition");
            let current = param
                .read(before.material())
                .unwrap_or_else(|| panic!("{material_type:?} exposes `{}` with no value", param.key()));

            // A value that is definitely different from the current one, and
            // inside whatever bounds the control would offer.
            let next = match current {
                ParamValue::Scalar(v) => {
                    let range = param.range().unwrap_or_else(|| {
                        panic!("{material_type:?} exposes `{}` with no range", param.key())
                    });
                    let candidate = if (v - range.low).abs() > 1e-3 {
                        range.low
                    } else {
                        range.high
                    };
                    ParamValue::Scalar(candidate)
                }
                ParamValue::Color(c) => ParamValue::Color(tequila_core::spectrum::spectrum(
                    1.0 - c.r.min(1.0),
                    0.25,
                    0.75,
                )),
                ParamValue::Flag(f) => ParamValue::Flag(!f),
                ParamValue::Metal(ior) => {
                    let other = tequila_materials::fresnel::metals::ALL
                        .iter()
                        .map(|(_, i)| *i)
                        .find(|i| *i != ior)
                        .expect("the table has more than one metal");
                    ParamValue::Metal(other)
                }
            };
            assert_ne!(next, current, "`{}` test value matches the current one", param.key());

            bus.apply(
                &mut doc,
                Command::SetMaterialParameter {
                    material: key,
                    parameter: MaterialParameter::new(param, next),
                },
            )
            .unwrap_or_else(|e| {
                panic!("{material_type:?} exposes `{}` but setting it failed: {e}", param.key())
            });
            bus.end_interaction();

            let after = doc.materials.get(key).expect("a definition");
            assert_eq!(
                param.read(after.material()),
                Some(next),
                "{material_type:?}: setting `{}` did not take effect",
                param.key()
            );

            bus.undo(&mut doc).expect("undo applies");
            assert_eq!(
                doc.materials.get(key),
                Some(&before),
                "{material_type:?}: undoing `{}` did not restore the material",
                param.key()
            );
        }
    }
}

/// §4.2's texture toggle, as a command: attaching inserts the mapping and
/// texture nodes, and the parameter reads as driven.
#[test]
fn attaching_a_texture_inserts_the_nodes_and_drives_the_parameter() {
    use tequila_scene::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();

    assert!(!doc.materials.get(material).unwrap().graph.is_connected(Param::BaseColor));

    bus.apply(
        &mut doc,
        Command::SetParameterTexture {
            material,
            param: Param::BaseColor,
            source: Some(TextureSource::new("maps/panel.png", MapRole::Color)),
        },
    )
    .expect("attaches");

    let definition = doc.materials.get(material).unwrap();
    assert!(definition.graph.is_connected(Param::BaseColor));
    assert_eq!(
        definition.graph.len(),
        3,
        "expected the surface node plus a mapping and a texture"
    );
    assert!(definition.graph.validate().is_ok());
    assert_eq!(doc.textures.len(), 1);

    // The compiled program actually samples something now.
    let compiled = definition.graph.compile().expect("compiles");
    assert_eq!(compiled.requirements.textures.len(), 1);
    assert!(!compiled.instructions.is_empty());
}

/// The role comes from what the parameter is, not from the file — §5.2's rule,
/// at the point where it is decided.
#[test]
fn the_parameter_decides_the_maps_colour_space() {
    use tequila_scene::textures::TextureSource;
    use tequila_texture::image::ColorSpace;
    use tequila_texture::texture::MapRole;

    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();

    // The same file, once as a colour and once as a number.
    for (param, role) in [
        (Param::BaseColor, MapRole::Color),
        (Param::SpecularRoughness, MapRole::Data),
    ] {
        bus.apply(
            &mut doc,
            Command::SetParameterTexture {
                material,
                param,
                source: Some(TextureSource::new("maps/grey.png", role)),
            },
        )
        .expect("attaches");
        bus.end_interaction();
    }

    let spaces: Vec<ColorSpace> = doc
        .textures
        .iter()
        .map(|(_, s)| s.effective_color_space())
        .collect();
    assert!(
        spaces.contains(&ColorSpace::Srgb) && spaces.contains(&ColorSpace::Raw),
        "one file in two roles should decode two ways, got {spaces:?}"
    );
    assert_eq!(doc.textures.len(), 2, "one file, two roles, two references");
}

/// Detaching must clean up after itself. A mapping node left behind with
/// nothing downstream would appear in the graph editor as a node the user did
/// not add and cannot explain.
#[test]
fn detaching_a_texture_removes_the_nodes_it_added() {
    use tequila_scene::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();
    let before = doc.materials.get(material).unwrap().graph.len();

    bus.apply(
        &mut doc,
        Command::SetParameterTexture {
            material,
            param: Param::BaseColor,
            source: Some(TextureSource::new("maps/panel.png", MapRole::Color)),
        },
    )
    .expect("attaches");
    bus.end_interaction();

    bus.apply(
        &mut doc,
        Command::SetParameterTexture {
            material,
            param: Param::BaseColor,
            source: None,
        },
    )
    .expect("detaches");

    let definition = doc.materials.get(material).unwrap();
    assert_eq!(
        definition.graph.len(),
        before,
        "detaching left {} orphaned nodes behind",
        definition.graph.len() - before
    );
    assert!(!definition.graph.is_connected(Param::BaseColor));
    assert!(definition.graph.validate().is_ok());
}

/// Attaching and undoing must restore the document exactly — graph, texture
/// store and all. Restoring the graph but not the store would leave a texture
/// reference nothing points at; the reverse would leave a graph pointing at a
/// texture that is gone.
#[test]
fn undoing_a_texture_attachment_restores_everything() {
    use tequila_scene::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();
    let before = doc.clone();

    bus.apply(
        &mut doc,
        Command::SetParameterTexture {
            material,
            param: Param::BaseColor,
            source: Some(TextureSource::new("maps/panel.png", MapRole::Color)),
        },
    )
    .expect("attaches");
    bus.end_interaction();
    assert_ne!(doc.textures.len(), before.textures.len());

    bus.undo(&mut doc).expect("undoes");

    assert!(
        doc.describes_the_same_scene_as(&before),
        "undo left the document different: {:?}",
        doc.difference_from(&before)
    );
}

/// The literal a parameter had becomes the texture's fallback, so a file that
/// fails to load leaves the material looking as it did rather than black.
#[test]
fn the_previous_value_becomes_the_textures_fallback() {
    use tequila_materials::graph::{NodeKind, Value};
    use tequila_scene::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.42),
        },
    )
    .expect("sets");
    bus.end_interaction();

    bus.apply(
        &mut doc,
        Command::SetParameterTexture {
            material,
            param: Param::SpecularRoughness,
            source: Some(TextureSource::new("maps/rough.png", MapRole::Data)),
        },
    )
    .expect("attaches");

    let graph = &doc.materials.get(material).unwrap().graph;
    let fallback = graph
        .nodes()
        .find_map(|(_, n)| match &n.kind {
            NodeKind::ImageTexture { fallback, .. } => Some(*fallback),
            _ => None,
        })
        .expect("a texture node");
    assert_eq!(
        fallback,
        Value::Float(0.42),
        "the fallback should be the value the parameter had"
    );
}

/// Collecting garbage must drop a texture nothing refers to, and keep one that
/// is still in use.
#[test]
fn unreferenced_textures_are_collected() {
    use tequila_scene::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    let (mut doc, _, _, material) = assembly();
    let mut bus = CommandBus::new();

    bus.apply(
        &mut doc,
        Command::SetParameterTexture {
            material,
            param: Param::BaseColor,
            source: Some(TextureSource::new("maps/used.png", MapRole::Color)),
        },
    )
    .expect("attaches");
    bus.end_interaction();

    // One nothing points at.
    doc.textures
        .intern(TextureSource::new("maps/orphan.png", MapRole::Color));
    assert_eq!(doc.textures.len(), 2);

    doc.collect_garbage();

    assert_eq!(doc.textures.len(), 1, "the orphan survived collection");
    assert!(doc
        .textures
        .iter()
        .any(|(_, s)| s.display_name() == "used.png"));
}

/// A script's work is one step: every command applied inside a group comes
/// off with one undo and goes back with one redo.
#[test]
fn a_group_is_one_undo_step() {
    let (mut doc, ids, _, _) = assembly();
    let mut bus = CommandBus::new();

    // Something from before the script, which must survive undoing it.
    bus.apply(&mut doc, Command::Rename { node: ids[1], name: "Before".into() }).unwrap();
    let before_script = doc.clone();

    bus.begin_group();
    for (i, id) in ids.iter().enumerate().skip(2) {
        bus.apply(&mut doc, Command::Rename { node: *id, name: format!("Scripted {i}") }).unwrap();
    }
    bus.apply(&mut doc, Command::set_visibility(ids[1], false)).unwrap();
    bus.end_group();
    let after_script = doc.clone();

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &before_script, "one undo takes the whole script back");
    assert!(bus.can_undo(), "and only the script: the edit before it remains");

    bus.redo(&mut doc).unwrap();
    same_scene(&doc, &after_script, "one redo puts the whole script back");

    bus.undo(&mut doc).unwrap();
    bus.undo(&mut doc).unwrap();
    assert_eq!(doc.tree.get(ids[1]).unwrap().name, "Part 0", "the edit before the script undoes on its own");
}

/// The first command of a group must not merge into the edit before it, or
/// undoing the group would silently take that edit too.
#[test]
fn a_group_does_not_swallow_the_edit_before_it() {
    let (mut doc, ids, _, material) = assembly();
    let mut bus = CommandBus::new();
    let roughness = |value| Command::SetMaterialParameter {
        material,
        parameter: tequila_scene::command::MaterialParameter::scalar(Param::SpecularRoughness, value),
    };

    bus.apply(&mut doc, roughness(0.2)).unwrap();
    let before = doc.clone();
    bus.begin_group();
    // Same kind of edit to the same target: exactly what merges when it is
    // a slider drag.
    bus.apply(&mut doc, roughness(0.7)).unwrap();
    bus.apply(&mut doc, Command::Rename { node: ids[1], name: "In group".into() }).unwrap();
    bus.end_group();

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &before, "undoing the group keeps the edit made before it");
}

/// A batch that fails part-way is taken back entirely, and cannot be redone.
#[test]
fn an_abandoned_group_leaves_no_trace() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.begin_group();
    bus.apply(&mut doc, Command::Rename { node: ids[1], name: "Half".into() }).unwrap();
    bus.apply(&mut doc, Command::set_visibility(ids[2], false)).unwrap();
    assert!(bus.apply(&mut doc, Command::Rename { node: NodeId(9999), name: "ghost".into() }).is_err());
    let reverted = bus.abandon_group(&mut doc).unwrap();

    assert_eq!(reverted, 2);
    same_scene(&doc, &start, "the applied half of the batch is gone");
    assert!(!bus.can_redo(), "a failed batch is not offered for redo");
    assert!(!bus.in_group(), "and the group is closed");
}

/// A nested batch that fails takes back its own steps only: the batch
/// around it keeps what it did before, still open, and is one step after.
#[test]
fn an_abandoned_inner_group_keeps_the_outer_groups_work() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.begin_group();
    bus.apply(&mut doc, Command::Rename { node: ids[1], name: "Outer".into() }).unwrap();
    bus.begin_group();
    bus.apply(&mut doc, Command::Rename { node: ids[2], name: "Inner".into() }).unwrap();
    let reverted = bus.abandon_group(&mut doc).unwrap();
    assert_eq!(reverted, 1, "only the inner batch's step");
    assert_eq!(doc.tree.get(ids[1]).unwrap().name, "Outer");
    assert_eq!(doc.tree.get(ids[2]).unwrap().name, start.tree.get(ids[2]).unwrap().name);
    assert!(bus.in_group(), "the outer batch is still open");
    bus.apply(&mut doc, Command::Rename { node: ids[3], name: "After".into() }).unwrap();
    bus.end_group();
    assert!(!bus.in_group());

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "the outer batch undoes as one step");
}

/// Groups nest by counting, so a script that runs a script is one step.
#[test]
fn nested_groups_are_one_step() {
    let (mut doc, ids, _, _) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    bus.begin_group();
    bus.apply(&mut doc, Command::Rename { node: ids[1], name: "Outer".into() }).unwrap();
    bus.begin_group();
    bus.apply(&mut doc, Command::Rename { node: ids[2], name: "Inner".into() }).unwrap();
    bus.end_group();
    assert!(bus.in_group(), "the outer group is still open");
    bus.apply(&mut doc, Command::Rename { node: ids[3], name: "Outer again".into() }).unwrap();
    bus.end_group();

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "one undo for the lot");
    assert!(!bus.can_undo());
}

/// A graph edit is one command: applied whole, undone whole, and refused
/// whole if the graph would not compile.
#[test]
fn a_graph_edit_applies_undoes_and_refuses_a_broken_graph() {
    use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value};
    let (mut doc, _, _, material) = assembly();
    let start = doc.clone();
    let mut bus = CommandBus::new();

    let mut graph = doc.materials.get(material).unwrap().graph.clone();
    let constant = graph.add_node(Node::new(NodeKind::Constant(Value::Float(0.8))));
    let output = graph.output();
    graph.connect(constant, output, Socket::Param(Param::SpecularRoughness));
    bus.apply(&mut doc, Command::SetGraph { material, graph: Box::new(graph.clone()) }).unwrap();
    assert_eq!(doc.materials.get(material).unwrap().graph, graph);

    // A cycle between two nodes cannot compile: refused, and nothing changes.
    let mut broken: MaterialGraph = graph.clone();
    let a = broken.add_node(Node::new(NodeKind::Invert));
    let b = broken.add_node(Node::new(NodeKind::Invert));
    broken.connect(a, b, Socket::Input(0));
    broken.connect(b, a, Socket::Input(0));
    broken.connect(a, output, Socket::Param(Param::BaseWeight));
    let before = doc.clone();
    assert!(bus.apply(&mut doc, Command::SetGraph { material, graph: Box::new(broken) }).is_err());
    same_scene(&doc, &before, "a refused graph changes nothing");

    bus.undo(&mut doc).unwrap();
    same_scene(&doc, &start, "undo restores the graph as it was");
}
