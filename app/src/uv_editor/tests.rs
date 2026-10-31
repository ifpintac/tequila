//! The editor as a whole, drawn frame after frame without a window: a drag
//! held and let go as a script holds it, the edits it makes and how they
//! undo.

use super::*;
use std::sync::Arc;
use tequila_core::math::vec3;
use tequila_scene::command::{reserve_keys, ImportPayload};
use tequila_scene::guides::GuideAxis;
use tequila_scene::{SceneDocument, SceneNode};

/// A part a fifth of a metre square, its layout the unit square or none,
/// with a material: the project, the part and the material.
fn a_project(laid_out: bool) -> (Project, NodeId, MaterialKey) {
    let document = SceneDocument::new();
    let mut mesh = tequila_scene::Mesh::new(vec![vec3(0.0, 0.0, 0.0), vec3(0.2, 0.0, 0.0), vec3(0.2, 0.2, 0.0), vec3(0.0, 0.2, 0.0)], vec![0, 1, 2, 0, 2, 3]);
    if laid_out {
        mesh.uv_sets = vec![vec![vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 0.0)]];
    }
    let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 1);
    let mut node = SceneNode::new(ids[0], "Panel", NodeKind::Part { mesh: meshes[0] });
    node.material = Some(materials[0]);
    let payload = ImportPayload {
        meshes: vec![(meshes[0], mesh)],
        textures: Vec::new(),
        materials: vec![(materials[0], tequila_scene::MaterialDefinition::plastic("Shell"))],
        nodes: vec![node],
        roots: vec![ids[0]],
    };
    let mut project = Project::with_document(document, Arc::new(tequila_scene::demo::studio_environment()));
    project.apply(Command::Import { payload: Box::new(payload) }).unwrap();
    project.end_interaction();
    (project, ids[0], materials[0])
}

/// A small picture to print, written once for the tests.
fn artwork() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("tequila-uv-editor-logo-{}.png", std::process::id()));
    if !path.exists() {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 8, 4);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[200u8; 8 * 4 * 3]).unwrap();
    }
    path
}

/// A label on the layout, centred at `at`, `side` across.
fn add_label(project: &mut Project, material: MaterialKey, at: Vec2, side: f32) {
    let (command, _) = crate::decal_tab::label_from_file(material, &artwork()).unwrap();
    project.apply(command).unwrap();
    let mut decals = project.document().materials.get(material).unwrap().graph.decals.clone();
    let label = decals.last_mut().unwrap();
    let mapping = Mapping { projection: Projection::Uv, ..label.mapping };
    label.mapping = placement::centred(placement::with_repeats(mapping, Vec2::splat(1.0 / side)), at);
    project.end_interaction();
    project.apply(Command::SetDecals { material, decals }).unwrap();
    project.end_interaction();
}

fn input(modifiers: egui::Modifiers) -> egui::RawInput {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    egui::RawInput { screen_rect: Some(screen), modifiers, ..Default::default() }
}

fn frames(editor: &mut UvEditor, project: &mut Project, ctx: &egui::Context, count: usize) {
    for _ in 0..count {
        let _ = ctx.run(input(egui::Modifiers::NONE), |ctx| show(ctx, editor, project));
    }
}

fn centre_of_label(project: &Project, material: MaterialKey, index: usize) -> Vec2 {
    placement::centre(&project.document().materials.get(material).unwrap().graph.decals[index].mapping)
}

/// An editor open on the part, the grid off so only the guide and the
/// layout's own lines are there to snap to.
fn editor_on(project: &mut Project, node: NodeId, ctx: &egui::Context) -> UvEditor {
    let mut editor = UvEditor::default();
    editor.prefs.grid = false;
    editor.open_on(node);
    frames(&mut editor, project, ctx, 3);
    editor
}

/// The editor lays out round a part's labels, and a label dragged with the
/// Move tool to near a guide lands on it by its nearest edge: one step to
/// undo, and while it is held the editor says what it snapped to.
#[test]
fn a_label_dragged_near_a_guide_lands_on_it() {
    let (mut project, node, material) = a_project(true);
    add_label(&mut project, material, vec2(0.25, 0.3), 0.2);
    project.apply(Command::SetUvGuides { node, guides: vec![UvGuide::new(GuideAxis::U, 0.65)] }).unwrap();
    project.end_interaction();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    assert!(editor.layout.is_some(), "the part's layout is read");
    assert!(editor.canvas_rect.width() > 400.0, "the canvas has the room: {:?}", editor.canvas_rect);
    assert_eq!(editor.scale.unit, units::Unit::Millimetres, "a layout of one size reads in millimetres");
    assert!((editor.scale.per_uv - 200.0).abs() < 1e-2, "the part is 200 mm across its layout: {}", editor.scale.per_uv);

    // Held: the right edge, 0.007 short of the guide, is pulled onto it.
    editor.tool = tools::Tool::Move;
    editor.selection.set(Item::Label(0));
    editor.scripted.drag = Some((vec2(0.25, 0.3), vec2(0.543, 0.3)));
    frames(&mut editor, &mut project, &ctx, 2);
    assert_eq!(editor.snap.note(), Some("guide"));
    assert!((centre_of_label(&project, material, 0) - vec2(0.25, 0.3)).length() < 1e-5, "nothing changes while it is held");

    editor.scripted.release = true;
    frames(&mut editor, &mut project, &ctx, 2);
    let moved = centre_of_label(&project, material, 0);
    assert!((moved - vec2(0.55, 0.3)).length() < 1e-4, "{moved:?}");
    project.undo();
    assert!((centre_of_label(&project, material, 0) - vec2(0.25, 0.3)).length() < 1e-5, "one step to undo");
}

/// With Ctrl held, the same drag ends where it was let go.
#[test]
fn ctrl_drags_freely() {
    let (mut project, node, material) = a_project(true);
    add_label(&mut project, material, vec2(0.25, 0.3), 0.2);
    project.apply(Command::SetUvGuides { node, guides: vec![UvGuide::new(GuideAxis::U, 0.65)] }).unwrap();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    editor.tool = tools::Tool::Move;
    editor.selection.set(Item::Label(0));
    editor.scripted.drag = Some((vec2(0.25, 0.3), vec2(0.543, 0.3)));
    editor.scripted.release = true;
    let _ = ctx.run(input(egui::Modifiers::COMMAND), |ctx| show(ctx, &mut editor, &mut project));
    let moved = centre_of_label(&project, material, 0);
    assert!((moved - vec2(0.543, 0.3)).length() < 1e-4, "{moved:?}");
}

/// A measure drawn with the Measure tool stays, its length on the part.
#[test]
fn a_measure_reads_on_the_part() {
    let (mut project, node, _) = a_project(true);
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    editor.prefs.snapping = false;
    editor.tool = tools::Tool::Measure;
    editor.scripted.drag = Some((vec2(0.1, 0.2), vec2(0.4, 0.6)));
    editor.scripted.release = true;
    frames(&mut editor, &mut project, &ctx, 2);
    let (from, to) = editor.measure.expect("the measure stays");
    assert!(((to - from).length() - 0.5).abs() < 1e-4);
    assert_eq!(editor.scale.format((to - from).length()), "100.0 mm");
}

/// A guide dragged out of the left ruler is a guide down the layout, saved
/// with the part; dragged back onto the ruler, it goes.
#[test]
fn a_guide_comes_out_of_a_ruler_and_goes_back() {
    let (mut project, node, _) = a_project(true);
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let part = editor.part(&project).unwrap();
    let mut edits = Edits::default();
    editor.gesture = Some(tools::Gesture::Guide { index: None, across: true, at: 0.3, removing: false });
    editor.finish(&part.labels, &part, &mut edits, editor.scale);
    editor.apply(&mut project, &part, edits, false);
    assert_eq!(project.document().tree.get(node).unwrap().uv_guides, [UvGuide::new(GuideAxis::U, 0.3)]);
    let part = editor.part(&project).unwrap();
    let mut edits = Edits::default();
    editor.gesture = Some(tools::Gesture::Guide { index: Some(0), across: true, at: 0.3, removing: true });
    editor.finish(&part.labels, &part, &mut edits, editor.scale);
    editor.apply(&mut project, &part, edits, false);
    assert!(project.document().tree.get(node).unwrap().uv_guides.is_empty());
}

/// Opened from a label on a part with no layout, the part is unwrapped, as
/// the Labels tab's button says: one step, with a word in the status line,
/// and the label selected.
#[test]
fn opening_on_a_label_unwraps_a_part_without_a_layout() {
    let (mut project, node, material) = a_project(false);
    let (command, _) = crate::decal_tab::label_from_file(material, &artwork()).unwrap();
    project.apply(command).unwrap();
    project.end_interaction();
    let before = project.undo_step();
    let ctx = egui::Context::default();
    let mut editor = UvEditor::default();
    editor.edit(material, 0);
    frames(&mut editor, &mut project, &ctx, 2);
    let mesh = project.document().tree.get(node).and_then(|n| n.mesh()).and_then(|m| project.document().geometry.get(m)).unwrap();
    assert!(!mesh.uv_sets.is_empty(), "the part was unwrapped");
    assert!(project.status.starts_with("Unwrapped Panel"), "{}", project.status);
    assert!(editor.layout.is_some());
    assert_eq!(editor.selection.labels(), [0]);
    project.undo();
    assert_eq!(project.undo_step(), before, "the unwrap was one step");
}

/// Align, duplicate and delete each change the labels, the selection
/// follows, and all of a frame's edits are one step to undo.
#[test]
fn the_selection_s_labels_are_aligned_duplicated_and_deleted() {
    let (mut project, node, material) = a_project(true);
    add_label(&mut project, material, vec2(0.2, 0.2), 0.1);
    add_label(&mut project, material, vec2(0.6, 0.5), 0.2);
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let part = editor.part(&project).unwrap();
    let layout = editor.layout.take().unwrap();

    let mut edits = Edits::default();
    editor.selection.set_labels([0, 1]);
    editor.align_to = align::Target::Selection;
    editor.align(&part, &mut edits, &layout.2, align::Align::Left);
    let lefts: Vec<f32> = edits.labels.as_ref().unwrap().iter().map(|l| placement::bounds(&l.mapping).0.x).collect();
    assert!((lefts[0] - lefts[1]).abs() < 1e-5 && (lefts[0] - 0.15).abs() < 1e-5, "{lefts:?}");

    editor.duplicate_selected(&part, &mut edits);
    assert_eq!(edits.labels.as_ref().unwrap().len(), 4);
    assert_eq!(editor.selection.labels(), [2, 3], "the copies are selected");
    assert!(edits.labels.as_ref().unwrap()[2].name.ends_with("copy"));
    editor.selection.set(Item::Label(0));
    editor.delete_selected(&part, &mut edits);
    assert_eq!(edits.labels.as_ref().unwrap().len(), 3);
    assert!(editor.selection.is_empty());
    editor.layout = Some(layout);
    editor.apply(&mut project, &part, edits, false);
    assert_eq!(project.document().materials.get(material).unwrap().graph.decals.len(), 3);
    project.undo();
    assert_eq!(project.document().materials.get(material).unwrap().graph.decals.len(), 2, "all of it one step to undo");
}

/// A locked label is not picked or moved, and a hidden one not picked.
#[test]
fn locked_and_hidden_labels_are_left_alone() {
    let (mut project, node, material) = a_project(true);
    add_label(&mut project, material, vec2(0.3, 0.3), 0.2);
    add_label(&mut project, material, vec2(0.7, 0.7), 0.2);
    let mut decals = project.document().materials.get(material).unwrap().graph.decals.clone();
    decals[0].locked = true;
    decals[1].enabled = false;
    project.apply(Command::SetDecals { material, decals }).unwrap();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let part = editor.part(&project).unwrap();
    let xform = editor.view.xform(editor.canvas_rect);
    for at in [vec2(0.3, 0.3), vec2(0.7, 0.7)] {
        assert!(!matches!(editor.pick(xform.to_screen(at), &xform, &part.labels, &part), canvas::Hit::Item(_)), "{at:?} was picked");
    }
    editor.select_all(&part.labels);
    assert!(editor.selection.is_empty());
}
