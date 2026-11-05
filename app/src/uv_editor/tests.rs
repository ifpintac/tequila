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

/// Opened from a label whose material two parts share, the editor shows
/// the one that is selected rather than the first.
#[test]
fn opening_on_a_shared_label_shows_the_selected_part() {
    let (mut project, first, material) = a_project(true);
    let (meshes, _, ids) = reserve_keys(project.document(), 1, 0, 1);
    let mesh = tequila_scene::Mesh::new(vec![vec3(0.0, 0.0, 0.1), vec3(0.2, 0.0, 0.1), vec3(0.2, 0.2, 0.1)], vec![0, 1, 2]);
    let mut node = SceneNode::new(ids[0], "Back", NodeKind::Part { mesh: meshes[0] });
    node.material = Some(material);
    let payload = ImportPayload { meshes: vec![(meshes[0], mesh)], textures: Vec::new(), materials: Vec::new(), nodes: vec![node], roots: vec![ids[0]] };
    project.apply(Command::Import { payload: Box::new(payload) }).unwrap();
    add_label(&mut project, material, vec2(0.5, 0.5), 0.2);
    project.selection.clear();
    let ctx = egui::Context::default();

    let mut editor = UvEditor::default();
    editor.edit(material, 0);
    frames(&mut editor, &mut project, &ctx, 2);
    assert_eq!(editor.part(&project).map(|p| p.node), Some(first), "with nothing selected, the first part");

    project.selection.select_many([ids[0]], tequila_scene::selection::SelectionMode::Replace);
    editor.edit(material, 0);
    frames(&mut editor, &mut project, &ctx, 2);
    assert_eq!(editor.part(&project).map(|p| p.node), Some(ids[0]), "the selected part");
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

// ---- the layout's own: islands, edges, vertices ----------------------------------

use tequila_scripting::protocol::{UvEditorChange, UvExport, UvPack};

/// A part of two squares a fifth of a metre across, each its own island
/// three tenths of the layout a side: one at the top left, one lower right.
/// Laid out as an unwrap lays one out, reading the right way.
fn two_islands() -> (Project, NodeId, MaterialKey) {
    let s = 0.2;
    let positions = vec![
        vec3(0.0, 0.0, 0.0), vec3(s, 0.0, 0.0), vec3(s, s, 0.0), vec3(0.0, s, 0.0),
        vec3(0.5, 0.0, 0.0), vec3(0.5 + s, 0.0, 0.0), vec3(0.5 + s, s, 0.0), vec3(0.5, s, 0.0),
    ];
    let square = |x: f32, y: f32| [vec2(x, y + 0.3), vec2(x + 0.3, y + 0.3), vec2(x + 0.3, y), vec2(x, y)];
    let mut mesh = tequila_scene::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
    mesh.uv_sets = vec![square(0.1, 0.1).into_iter().chain(square(0.55, 0.5)).collect()];
    part_with(mesh)
}

/// A strip of two squares side by side, one island across the edge they
/// share: vertices 1 and 4 are that edge's ends.
fn one_strip() -> (Project, NodeId, MaterialKey) {
    let positions = vec![
        vec3(0.0, 0.0, 0.0), vec3(0.1, 0.0, 0.0), vec3(0.2, 0.0, 0.0),
        vec3(0.0, 0.1, 0.0), vec3(0.1, 0.1, 0.0), vec3(0.2, 0.1, 0.0),
    ];
    let mut mesh = tequila_scene::Mesh::new(positions.clone(), vec![0, 1, 4, 0, 4, 3, 1, 2, 5, 1, 5, 4]);
    mesh.uv_sets = vec![positions.iter().map(|p| vec2(0.1 + p.x * 3.0, 0.7 - p.y * 3.0)).collect()];
    part_with(mesh)
}

fn part_with(mesh: tequila_scene::Mesh) -> (Project, NodeId, MaterialKey) {
    let document = SceneDocument::new();
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

fn mesh_of(project: &Project, node: NodeId) -> tequila_scene::Mesh {
    let key = project.document().tree.get(node).and_then(|n| n.mesh()).unwrap();
    project.document().geometry.get(key).unwrap().clone()
}

fn islands_of(project: &Project, node: NodeId, set: usize) -> tequila_scene::uv::UvLayout {
    tequila_scene::uv::UvLayout::new(&mesh_of(project, node), set).unwrap()
}

/// A script's change to the editor, a frame or two to carry it out.
fn ask(editor: &mut UvEditor, project: &mut Project, ctx: &egui::Context, change: UvEditorChange) {
    editor.script(&change, project).unwrap();
    frames(editor, project, ctx, 2);
}

/// An island dragged with the Move tool to near a guide lands on it by its
/// nearest edge, the drag shown while it is held and the layout changed
/// only when it is let go: one `uv.transform`, one step to undo. A label
/// on the island goes with it.
#[test]
fn an_island_dragged_near_a_guide_snaps_and_is_one_step() {
    let (mut project, node, material) = two_islands();
    add_label(&mut project, material, vec2(0.7, 0.65), 0.1);
    project.apply(Command::SetUvGuides { node, guides: vec![UvGuide::new(GuideAxis::U, 0.5)] }).unwrap();
    project.end_interaction();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let before = mesh_of(&project, node).uv_sets[0].clone();
    let steps = project.undo_step();

    editor.tool = tools::Tool::Move;
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![1]), ..Default::default() });
    assert_eq!(editor.mode, Mode::Islands);
    assert_eq!(chosen::islands(&editor), vec![1]);
    // Held: the left edge, 0.003 short of the guide, is pulled onto it.
    editor.scripted.drag = Some((vec2(0.7, 0.65), vec2(0.647, 0.65)));
    frames(&mut editor, &mut project, &ctx, 2);
    assert_eq!(editor.snap.note(), Some("guide"));
    assert!(matches!(editor.gesture, Some(tools::Gesture::Layout { .. })));
    assert_eq!(mesh_of(&project, node).uv_sets[0], before, "nothing changes while it is held");

    editor.scripted.release = true;
    frames(&mut editor, &mut project, &ctx, 2);
    let after = mesh_of(&project, node).uv_sets[0].clone();
    assert!((after[4].x - 0.5).abs() < 1e-5, "the left edge on the guide: {:?}", after[4]);
    assert!((after[5].x - 0.8).abs() < 1e-5 && (after[4].y - before[4].y).abs() < 1e-5);
    assert_eq!(&after[..4], &before[..4], "the other island stays");
    let label = centre_of_label(&project, material, 0);
    assert!((label - vec2(0.65, 0.65)).length() < 1e-4, "the label on it went with it: {label:?}");
    assert_eq!(chosen::islands(&editor), vec![1], "still selected");

    project.undo();
    assert_eq!(project.undo_step(), steps, "one step to undo");
    assert_eq!(mesh_of(&project, node).uv_sets[0], before);
    assert!((centre_of_label(&project, material, 0) - vec2(0.7, 0.65)).length() < 1e-5, "and the label back with it");
}

/// The strip cut along the edge its squares share comes apart in two
/// islands, the edge a seam; stitched along that seam it is one again,
/// whole. Each is one step to undo.
#[test]
fn a_cut_and_a_stitch_undo_each_other() {
    let (mut project, node, _) = one_strip();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    assert_eq!(islands_of(&project, node, 0).islands.len(), 1);
    let original = mesh_of(&project, node);

    ask(&mut editor, &mut project, &ctx, UvEditorChange { edges: Some(vec![[1, 4]]), cut: true, ..Default::default() });
    let cut = islands_of(&project, node, 0);
    assert_eq!(cut.islands.len(), 2, "{:?}", editor.message);
    assert_eq!(cut.edges.iter().filter(|e| e.kind == tequila_scene::uv::EdgeKind::Seam).count(), 2, "the cut is a seam, a side on each island");
    assert!(cut.folded_triangles().is_empty());
    assert!(editor.message.as_ref().is_some_and(|(text, refused, _)| !refused && text.starts_with("Cut along 1 edge")), "{:?}", editor.message);

    // The two islands selected, and the seam between them stitched.
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0, 1]), stitch: true, ..Default::default() });
    let stitched = islands_of(&project, node, 0);
    assert_eq!(stitched.islands.len(), 1, "{:?}", editor.message);
    assert!(stitched.edges.iter().all(|e| e.kind != tequila_scene::uv::EdgeKind::Seam));
    assert!(stitched.folded_triangles().is_empty());

    project.undo();
    assert_eq!(islands_of(&project, node, 0).islands.len(), 2, "the stitch was one step");
    project.undo();
    assert_eq!(mesh_of(&project, node), original, "the cut was one step");
}

/// A stitch that has nothing to stitch, and an edit the engine refuses,
/// say so in the status bar and change nothing.
#[test]
fn a_refusal_says_why_and_changes_nothing() {
    let (mut project, node, _) = two_islands();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let before = mesh_of(&project, node);
    let steps = project.undo_step();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0, 1]), stitch: true, ..Default::default() });
    assert!(editor.message.as_ref().is_some_and(|(text, refused, _)| *refused && text.contains("seams")), "{:?}", editor.message);
    // Squashed flat: the engine will not, and the editor says why.
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0]), transform: Some(tequila_scripting::protocol::UvTransform { scale: Some([1.0, 1e-9]), ..Default::default() }), ..Default::default() });
    assert!(editor.message.as_ref().is_some_and(|(text, refused, _)| *refused && text.starts_with("Could not move it")), "{:?}", editor.message);
    assert_eq!(mesh_of(&project, node), before);
    assert_eq!(project.undo_step(), steps, "nothing to undo");
}

/// Pinned vertices stay exactly where they are when their island, squashed,
/// is laid out flat again; the rest of it moves, and the pins are still
/// pins after. Pinning and unpinning are each a step to undo, kept on the
/// part in the document.
#[test]
fn pins_hold_through_a_reflatten() {
    let (mut project, node, _) = one_strip();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let pins = |project: &Project| project.document().tree.get(node).unwrap().uv_pins.get(&0).cloned().unwrap_or_default();
    // Squashed to half its height, so laying it out again changes it.
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0]), transform: Some(tequila_scripting::protocol::UvTransform { scale: Some([1.0, 0.5]), ..Default::default() }), ..Default::default() });
    let squashed = mesh_of(&project, node).uv_sets[0].clone();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(vec![0, 2]), pin: Some(true), ..Default::default() });
    assert_eq!(pins(&project), vec![0, 2]);
    assert_eq!(project.undo_step().map(|(name, _)| name), Some("uv.set_pins"), "pinning is a step of its own");
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0]), reflatten: true, ..Default::default() });
    let flat = mesh_of(&project, node).uv_sets[0].clone();
    assert_eq!((flat[0], flat[2]), (squashed[0], squashed[2]), "the pins held");
    assert!((flat[4] - squashed[4]).length() > 1e-3, "the rest laid out again: {:?} {:?}", flat[4], squashed[4]);
    let height = (flat[3] - flat[0]).length();
    assert!((height - 0.3).abs() < 0.02, "back to its true shape: {height}");
    assert_eq!(pins(&project), vec![0, 2], "the pins are still pins");
    ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(vec![0]), pin: Some(false), ..Default::default() });
    assert_eq!(pins(&project), vec![2]);
    // Undone, the unpinned vertex is pinned again; undone once more, the
    // re-flatten goes; redone, both come back.
    project.undo();
    frames(&mut editor, &mut project, &ctx, 2);
    assert_eq!(pins(&project), vec![0, 2]);
    assert_eq!(editor.state().get("pins").map(|p| p.as_array().len()), Some(2), "the editor shows the document's pins");
    project.undo();
    project.undo();
    assert!(pins(&project).is_empty(), "the pinning undone");
    project.redo();
    project.redo();
    project.redo();
    assert_eq!(pins(&project), vec![2]);
}

/// Packed between two guides down the layout and two across, every island
/// is inside the box they make, in one step.
#[test]
fn islands_pack_between_guides() {
    let (mut project, node, _) = two_islands();
    let guides = vec![UvGuide::new(GuideAxis::U, 0.6), UvGuide::new(GuideAxis::U, 0.95), UvGuide::new(GuideAxis::V, 0.1), UvGuide::new(GuideAxis::V, 0.3)];
    project.apply(Command::SetUvGuides { node, guides }).unwrap();
    project.end_interaction();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let steps = project.undo_step();
    let pack = UvPack { margin: Some(0.0), unit: Some("px".into()), into: Some("guides".into()), ..Default::default() };
    ask(&mut editor, &mut project, &ctx, UvEditorChange { mode: Some("islands".into()), pack: Some(pack), ..Default::default() });
    let uv = mesh_of(&project, node).uv_sets[0].clone();
    for p in &uv {
        assert!(p.x >= 0.6 - 1e-4 && p.x <= 0.95 + 1e-4 && p.y >= 0.1 - 1e-4 && p.y <= 0.3 + 1e-4, "{p:?} is outside the guides");
    }
    assert!(editor.message.as_ref().is_some_and(|(text, _, _)| text.contains("between the guides")), "{:?}", editor.message);
    project.undo();
    assert_eq!(project.undo_step(), steps);
    // Without two guides there is no box to pack into, and it says so.
    project.apply(Command::SetUvGuides { node, guides: vec![UvGuide::new(GuideAxis::U, 0.6)] }).unwrap();
    project.end_interaction();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { pack: Some(UvPack::default()), ..Default::default() });
    assert!(editor.message.as_ref().is_some_and(|(text, refused, _)| *refused && text.contains("two guides")), "{:?}", editor.message);
}

/// A set added after the first leaves a label on the first where it was;
/// with the label moved to the new set, the first deleted, the label is on
/// what is now the first set, which is the layout it was on.
#[test]
fn a_label_keeps_to_its_set_as_sets_come_and_go() {
    let (mut project, node, material) = two_islands();
    add_label(&mut project, material, vec2(0.25, 0.25), 0.1);
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    ask(&mut editor, &mut project, &ctx, UvEditorChange { set_action: Some("add".into()), ..Default::default() });
    let mesh = mesh_of(&project, node);
    assert_eq!(mesh.uv_sets.len(), 2, "{:?}", editor.message);
    assert_eq!(editor.shown_set, 1, "the new set is shown");
    let label = |project: &Project| project.document().materials.get(material).unwrap().graph.decals[0].mapping;
    assert_eq!(label(&project).uv_set, 0, "the label is on the set it was on");
    // The label to the new set, then the first set deleted.
    let mut decals = project.document().materials.get(material).unwrap().graph.decals.clone();
    decals[0].mapping.uv_set = 1;
    project.apply(Command::SetDecals { material, decals }).unwrap();
    project.end_interaction();
    let second = mesh.uv_sets[1].clone();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { set: Some(0), ..Default::default() });
    assert_eq!(editor.shown_set, 0);
    ask(&mut editor, &mut project, &ctx, UvEditorChange { set_action: Some("delete".into()), ..Default::default() });
    let mesh = mesh_of(&project, node);
    assert_eq!(mesh.uv_sets.len(), 1, "{:?}", editor.message);
    assert_eq!(label(&project).uv_set, 0, "the label followed its set down");
    assert_eq!(mesh.uv_sets[0].len(), second.len());
    assert_eq!(mesh.uv_sets[0], second, "and that set is the layout it was on");
    project.undo();
    assert_eq!(label(&project).uv_set, 1);
}

/// The layout written as a PNG and an SVG from a script, as the Export
/// dialog writes it.
#[test]
fn the_layout_is_exported_as_a_picture() {
    let (mut project, node, _) = two_islands();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let folder = std::env::temp_dir().join(format!("tequila-uv-export-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let png = folder.join("layout.png");
    let export = UvExport { path: png.display().to_string(), size: Some(256), fill: Some("islands".into()), line: Some("#204080".into()), transparent: Some(true) };
    ask(&mut editor, &mut project, &ctx, UvEditorChange { export: Some(export), ..Default::default() });
    let bytes = std::fs::read(&png).expect("the PNG was written");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]), 256);
    let svg = folder.join("layout.svg");
    ask(&mut editor, &mut project, &ctx, UvEditorChange { export: Some(UvExport { path: svg.display().to_string(), ..Default::default() }), ..Default::default() });
    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(text.starts_with("<svg") && text.contains("stroke=\"#204080\"") && !text.contains("fill=\"#ffffff\""), "the dialog's settings kept: {}", &text[..200.min(text.len())]);
    let _ = std::fs::remove_dir_all(&folder);
}

/// Faces picked on the part select their islands, and cut out, are an
/// island of their own.
#[test]
fn faces_picked_on_the_part_select_their_islands_and_cut_out() {
    let (mut project, node, _) = one_strip();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    ask(&mut editor, &mut project, &ctx, UvEditorChange { faces: Some(vec![2, 3]), ..Default::default() });
    assert_eq!(project.selection.faces(node).collect::<Vec<_>>(), vec![2, 3]);
    ask(&mut editor, &mut project, &ctx, UvEditorChange { from_faces: true, ..Default::default() });
    assert_eq!(chosen::islands(&editor), vec![0], "the strip's one island");
    ask(&mut editor, &mut project, &ctx, UvEditorChange { cut_faces: true, ..Default::default() });
    let layout = islands_of(&project, node, 0);
    assert_eq!(layout.islands.len(), 2, "{:?}", editor.message);
    assert_eq!(layout.island_of[2], layout.island_of[3], "the two faces are one island");
    assert_ne!(layout.island_of[0], layout.island_of[2]);
}

/// A label turned along an island runs the way Straighten reads the
/// island: straightened, the island lies square and the label on it, carried
/// with it, lies square too.
#[test]
fn along_the_island_agrees_with_straighten() {
    let (mut project, node, material) = one_strip();
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    // The strip turned 30 degrees anticlockwise, as the panel counts.
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0]), transform: Some(tequila_scripting::protocol::UvTransform { turn: Some(30.0), ..Default::default() }), ..Default::default() });
    let layout = islands_of(&project, node, 0);
    let centre = layout.islands[0].centre;
    add_label(&mut project, material, centre, 0.08);
    ask(&mut editor, &mut project, &ctx, UvEditorChange { select: Some(vec![0]), island: Some(0), place: Some("along".into()), ..Default::default() });
    let turn = |project: &Project| placement::turn(&project.document().materials.get(material).unwrap().graph.decals[0].mapping);
    let square = |angle: f32| {
        let folded = angle.rem_euclid(std::f32::consts::PI);
        folded.min(std::f32::consts::PI - folded)
    };
    assert!((square(turn(&project)) - 30f32.to_radians()).abs() < 0.02, "along the strip, 30 degrees off square: {}", turn(&project).to_degrees());
    ask(&mut editor, &mut project, &ctx, UvEditorChange { mode: Some("islands".into()), islands: Some(vec![0]), straighten: true, ..Default::default() });
    assert!(square(turn(&project)) < 0.02, "straightened, the label lies square with it: {}", turn(&project).to_degrees());
    let layout = islands_of(&project, node, 0);
    assert!(square(layout.islands[0].straightening_angle()) < 0.02);
}

/// The editor's own view of a selection, for the tests.
mod chosen {
    use super::*;
    pub fn islands(editor: &UvEditor) -> Vec<u32> {
        let (_, _, layout) = editor.layout.as_ref().unwrap();
        super::super::editing::islands(layout, &editor.selection)
    }
}
