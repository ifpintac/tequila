//! A model made of several parts, from the file to a material on one of them.
//!
//! This is the workflow the application exists for. Somebody imports a product
//! — a housing, a lens, a bezel, forty fasteners — and the first thing they do
//! is give one part a different material from the rest. Every step has to hold
//! together: the file becomes a group of separately addressable parts rather
//! than one welded lump, a click resolves to the part under the cursor and not
//! to the whole model, and the assignment touches that part alone.
//!
//! Each step is implemented somewhere else — the importer, the selection
//! rules, the command bus — and each has its own tests. What none of them can
//! catch is the seam: a part correctly imported and correctly picked, then
//! assigned through a path that quietly widened to the parent. So this walks
//! the whole thing, on the bundled sample, which is the model most people will
//! try this on first.

use tequila_scene::selection::{Selection, SelectionLevel, SelectionMode};
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::{Command, CommandBus, SceneDocument};

fn sample() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the workspace root")
        .join("assets/samples/glasses.gltf")
}

/// The sample, imported and applied — the state the user is actually looking
/// at, rather than the command that would produce it.
fn imported() -> (SceneDocument, CommandBus) {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let (command, _) = tequila_import::import(
        &document,
        None,
        &sample(),
        &tequila_import::ImportOptions::default(),
    )
    .expect("the sample imports");
    bus.apply(&mut document, command).expect("it applies");
    (document, bus)
}

fn parts(document: &SceneDocument) -> Vec<(NodeId, String)> {
    document
        .tree
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Part { .. }))
        .map(|node| (node.id, node.name.clone()))
        .collect()
}

/// The parts arrive addressable and under one parent. Welding a multi-part
/// file into a single mesh would import faster and would make every step below
/// impossible; putting the parts loose in the scene would keep them selectable
/// and lose the model as a thing that can be moved at once.
#[test]
fn the_parts_are_separate_and_share_one_parent() {
    let (document, _) = imported();
    let parts = parts(&document);
    assert!(
        parts.len() >= 2,
        "a multi-part sample imported as {} part(s): {:?}",
        parts.len(),
        parts.iter().map(|(_, n)| n).collect::<Vec<_>>()
    );

    let parents: Vec<Option<NodeId>> = parts
        .iter()
        .map(|(id, _)| document.tree.get(*id).and_then(|n| n.parent))
        .collect();
    assert!(
        parents.iter().all(|p| p.is_some()),
        "a part has no group above it; the model is loose in the scene"
    );
    assert!(
        parents.windows(2).all(|w| w[0] == w[1]),
        "the parts landed under different parents: {parents:?}"
    );
}

/// A click resolves to the part under the cursor. This is the step that would
/// make the feature useless if it widened: the group is what the tree shows
/// and what the file named, so resolving to it is exactly what a plausible
/// implementation does.
#[test]
fn a_click_selects_one_part_and_not_the_whole_model() {
    let (document, _) = imported();
    let parts = parts(&document);

    for (id, name) in &parts {
        let mut selection = Selection::new();
        selection.set_level(SelectionLevel::Part);
        selection.click(&document, *id, SelectionMode::Replace);

        let chosen: Vec<NodeId> = selection.nodes().collect();
        assert_eq!(
            chosen,
            vec![*id],
            "clicking `{name}` selected {} node(s) instead of just it",
            chosen.len()
        );
        assert!(
            matches!(
                document.tree.get(chosen[0]).map(|n| &n.kind),
                Some(NodeKind::Part { .. })
            ),
            "clicking `{name}` selected a group"
        );
    }
}

/// And the Object level still takes the whole model, so somebody who wants to
/// move the thing as one piece can. Both have to work, or "you can select a
/// part" turns into "you can no longer select the model".
#[test]
fn the_object_level_still_takes_the_whole_model() {
    let (document, _) = imported();
    let (part, name) = parts(&document).remove(0);

    let mut selection = Selection::new();
    selection.set_level(SelectionLevel::Object);
    selection.click(&document, part, SelectionMode::Replace);

    let chosen: Vec<NodeId> = selection.nodes().collect();
    assert_eq!(chosen.len(), 1, "the Object level selected {chosen:?}");
    assert_ne!(
        chosen[0], part,
        "the Object level stayed on the part `{name}` it was given"
    );
    assert!(
        tequila_scene::selection::top_level_ancestor(&document, part) == chosen[0],
        "the Object level did not resolve to the model's root"
    );
}

/// The point of the whole exercise: a material on one part, and nothing else
/// touched.
#[test]
fn a_material_on_one_part_leaves_every_sibling_alone() {
    let (mut document, mut bus) = imported();
    let parts = parts(&document);
    let (target, target_name) = parts[0].clone();

    let before: Vec<_> = parts
        .iter()
        .map(|(id, _)| (*id, document.tree.get(*id).and_then(|n| n.material)))
        .collect();

    // A material that is not already in the document, so "it changed" cannot
    // be confused with "it was already that".
    let key = document
        .materials
        .insert(tequila_scene::material::MaterialDefinition::new(
            "Test Red",
            tequila_materials::material_type::MaterialType::Plastic,
            Default::default(),
        ));
    bus.apply(
        &mut document,
        Command::AssignMaterial {
            nodes: vec![target],
            material: key,
        },
    )
    .expect("the assignment applies");

    assert_eq!(
        document.tree.get(target).and_then(|n| n.material),
        Some(key),
        "the material did not land on `{target_name}`"
    );
    for (id, was) in before.iter().skip(1) {
        assert_eq!(
            document.tree.get(*id).and_then(|n| n.material),
            *was,
            "assigning a material to `{target_name}` also changed `{}`",
            document
                .tree
                .get(*id)
                .map(|n| n.name.clone())
                .unwrap_or_default()
        );
    }
}

/// Every part is reachable from the viewport, which means every part produced
/// an instance the renderer can be asked about. A part that renders but has no
/// instance in the mapping is a part that can be seen and never clicked.
#[test]
fn every_part_is_pickable() {
    let (document, _) = imported();
    let (_, mapping) = tequila_scene::snapshot::snapshot(
        &document,
        std::sync::Arc::new(tequila_render_core::environment::Environment::uniform(
            tequila_core::spectrum::Spectrum::ONE,
        )),
        &mut Default::default(),
    );

    for (id, name) in parts(&document) {
        let instance = mapping
            .instance_of(id)
            .unwrap_or_else(|| panic!("`{name}` produced no instance, so it cannot be clicked"));
        assert_eq!(
            mapping.node_of(instance),
            Some(id),
            "`{name}` maps to an instance that maps back to a different node"
        );
    }
}
