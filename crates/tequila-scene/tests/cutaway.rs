//! Cutaways from the document to the snapshot (`docs/02` §3).
//!
//! The renderer's side is tested in `tequila-render-cpu`. These check what
//! the document hands it: a cutaway node becomes a cutter where the node is,
//! switching it off or hiding it takes it away, a cap colour becomes a
//! material, and a part given the Cutaway material stops being a surface and
//! becomes a cutter of its own shape.

use tequila_core::math::{vec3, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::cutaway::CutterShape;
use tequila_render_core::environment::Environment;
use tequila_scene::cutaway::{CapStyle, CutShape, CutawaySettings};
use tequila_scene::snapshot::{snapshot, MeshCache};
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus, SceneDocument};
use std::sync::Arc;

fn build(document: &SceneDocument) -> tequila_render_core::scene::SceneSnapshot {
    snapshot(
        document,
        Arc::new(Environment::uniform(Spectrum::ONE)),
        &mut MeshCache::default(),
    )
    .0
}

fn cutaway_node(document: &SceneDocument) -> tequila_scene::tree::NodeId {
    document
        .tree
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Cutaway { .. }))
        .map(|n| n.id)
        .expect("a cutaway node")
}

#[test]
fn a_cutaway_node_reaches_the_renderer_where_it_is() {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let mut bus = CommandBus::new();
    assert!(build(&document).cutters().is_empty());

    let placed = Transform::translation(vec3(0.0, 0.0, 0.25));
    bus.apply(
        &mut document,
        Command::AddCutaway {
            parent: None,
            name: "Section".into(),
            settings: Box::new(CutawaySettings {
                shape: CutShape::Box,
                ..Default::default()
            }),
            transform: placed,
        },
    )
    .unwrap();
    let scene = build(&document);
    assert_eq!(scene.cutters().len(), 1);
    let cutter = &scene.cutters()[0];
    assert!(matches!(cutter.shape, CutterShape::Box));
    assert_eq!(cutter.transform.matrix, placed.matrix, "the cutter is not where the node is");
    assert!(cutter.cap.is_none(), "an own-material cap was given a material");

    // One undo step takes it away again, placement and all.
    bus.undo(&mut document).unwrap();
    assert!(build(&document).cutters().is_empty());
}

#[test]
fn a_switched_off_or_hidden_cutaway_cuts_nothing() {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let mut bus = CommandBus::new();
    bus.apply(
        &mut document,
        Command::AddCutaway {
            parent: None,
            name: "Section".into(),
            settings: Box::default(),
            transform: Transform::IDENTITY,
        },
    )
    .unwrap();
    let node = cutaway_node(&document);

    bus.apply(
        &mut document,
        Command::SetCutaway {
            node,
            settings: Box::new(CutawaySettings {
                enabled: false,
                ..Default::default()
            }),
        },
    )
    .unwrap();
    assert!(build(&document).cutters().is_empty(), "a disabled cutaway still cut");
    bus.undo(&mut document).unwrap();
    assert_eq!(build(&document).cutters().len(), 1);

    bus.apply(&mut document, Command::set_visibility(node, false)).unwrap();
    assert!(build(&document).cutters().is_empty(), "a hidden cutaway still cut");
}

#[test]
fn a_cap_colour_becomes_a_material() {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let mut bus = CommandBus::new();
    let red = Spectrum { r: 0.8, g: 0.05, b: 0.05 };
    bus.apply(
        &mut document,
        Command::AddCutaway {
            parent: None,
            name: "Section".into(),
            settings: Box::new(CutawaySettings {
                cap: CapStyle::Color(red),
                ..Default::default()
            }),
            transform: Transform::IDENTITY,
        },
    )
    .unwrap();
    let scene = build(&document);
    let cap = scene.cutters()[0].cap.expect("a cap material");
    assert_eq!(scene.material(cap).base_color, red);

    // A colour that is not a number is refused, not rendered as NaN.
    let node = cutaway_node(&document);
    assert!(bus
        .apply(
            &mut document,
            Command::SetCutaway {
                node,
                settings: Box::new(CutawaySettings {
                    cap: CapStyle::Color(Spectrum::splat(f32::NAN)),
                    ..Default::default()
                }),
            },
        )
        .is_err());
}

/// The Cutaway material turns a part into a cutter: it leaves the scene as
/// a surface — nothing of it is drawn — and its closed mesh cuts instead.
#[test]
fn a_part_with_the_cutaway_material_becomes_a_cutter() {
    let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let mut bus = CommandBus::new();
    let before = build(&document);
    let material = document.materials.keys().next().expect("the demo's material");
    bus.apply(
        &mut document,
        Command::SetMaterialType {
            material,
            material_type: MaterialType::Cutaway,
        },
    )
    .unwrap();
    let after = build(&document);
    assert_eq!(after.cutters().len(), 1);
    assert!(matches!(after.cutters()[0].shape, CutterShape::Mesh(_)));
    assert_eq!(
        after.instances.len(),
        before.instances.len() - 1,
        "the cutter is still drawn as a surface"
    );
    // The demo's part is a sphere at the origin: its centre is inside.
    assert!(after.cutters()[0].contains(vec3(0.0, 0.0, 0.0)));
}
