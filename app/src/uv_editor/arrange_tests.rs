//! The arranging tools in the editor as a whole, drawn frame after frame
//! without a window and driven as a script drives them: each one step to
//! undo, a refusal saying why, and what is only for looking - the 2D
//! cursor, the checker on the part - changing nothing in the document.

use super::*;
use std::sync::Arc;
use tequila_core::math::vec3;
use tequila_scene::command::{reserve_keys, ImportPayload};
use tequila_scene::uv::UvLayout;
use tequila_scene::{SceneDocument, SceneNode};
use tequila_scripting::protocol::{UvArrange, UvCursor, UvEditorChange};

/// A box a fifth of a metre a side, split at its creases, unwrapped: six
/// islands of one shape.
fn boxed() -> tequila_scene::Mesh {
    let faces: [[tequila_core::math::Vec3; 4]; 6] = [
        [vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0)],
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0)],
        [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0)],
        [vec3(1.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 1.0, 1.0)],
        [vec3(0.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
        [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)],
    ];
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for quad in faces {
        let base = positions.len() as u32;
        positions.extend(quad.iter().map(|p| *p * 0.2));
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    tequila_scene::uv::unwrap(&tequila_scene::Mesh::new(positions, indices), &Default::default()).0
}

/// A strip of eight squares laid out a little crooked and wavy: one island.
fn crooked_strip() -> tequila_scene::Mesh {
    let mut positions = Vec::new();
    for r in 0..=1 {
        for c in 0..=8 {
            positions.push(vec3(c as f32 * 0.025, 0.0, r as f32 * 0.025));
        }
    }
    let at = |r: u32, c: u32| r * 9 + c;
    let mut indices = Vec::new();
    for c in 0..8 {
        indices.extend_from_slice(&[at(0, c), at(1, c), at(1, c + 1), at(0, c), at(1, c + 1), at(0, c + 1)]);
    }
    let uv = positions.iter().map(|p| vec2(0.1 + p.x * 32.0 + p.z * 0.4, 0.4 + p.z * 4.0 + (p.x * 160.0).sin() * 0.004 + p.x * 0.8)).collect();
    tequila_scene::Mesh::new(positions, indices).with_uvs(uv)
}

fn part_with(mesh: tequila_scene::Mesh) -> (Project, NodeId, MaterialKey) {
    let document = SceneDocument::new();
    let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 1);
    let mut node = SceneNode::new(ids[0], "Box", NodeKind::Part { mesh: meshes[0] });
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

fn input(modifiers: egui::Modifiers, events: Vec<egui::Event>) -> egui::RawInput {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    egui::RawInput { screen_rect: Some(screen), modifiers, events, ..Default::default() }
}

fn frames(editor: &mut UvEditor, project: &mut Project, ctx: &egui::Context, count: usize) {
    for _ in 0..count {
        let _ = ctx.run(input(egui::Modifiers::NONE, Vec::new()), |ctx| show(ctx, editor, project));
    }
}

fn editor_on(project: &mut Project, node: NodeId, ctx: &egui::Context) -> UvEditor {
    let mut editor = UvEditor::default();
    editor.prefs.grid = false;
    editor.open_on(node);
    frames(&mut editor, project, ctx, 3);
    editor
}

fn mesh_of(project: &Project, node: NodeId) -> tequila_scene::Mesh {
    let key = project.document().tree.get(node).and_then(|n| n.mesh()).unwrap();
    project.document().geometry.get(key).unwrap().clone()
}

fn layout_of(project: &Project, node: NodeId) -> UvLayout {
    UvLayout::new(&mesh_of(project, node), 0).unwrap()
}

/// A script's change, and frames to carry it out: the arranging tools wait
/// for the end of a frame, so two.
fn ask(editor: &mut UvEditor, project: &mut Project, ctx: &egui::Context, change: UvEditorChange) {
    editor.script(&change, project).unwrap();
    frames(editor, project, ctx, 3);
}

fn arrange(change: UvArrange) -> UvEditorChange {
    UvEditorChange { arrange: change, ..Default::default() }
}

fn islands(list: &[usize]) -> UvEditorChange {
    UvEditorChange { islands: Some(list.to_vec()), ..Default::default() }
}

fn options() -> Option<tequila_format::json::Value> {
    Some(tequila_format::json::Value::Object(Default::default()))
}

fn said(editor: &UvEditor) -> (String, bool) {
    editor.message.as_ref().map(|(t, r, _)| (t.clone(), *r)).unwrap_or_default()
}

/// Islands lined up by a corner against the 2D cursor: each moved whole,
/// all of them one step, undone in one.
#[test]
fn islands_line_up_by_a_corner_against_the_cursor_in_one_step() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let original = mesh_of(&project, node);
    let steps = project.undo_step();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { cursor: Some(UvCursor::At([0.5, 0.5])), align_to: Some("cursor".into()), ..Default::default() }));
    assert_eq!(editor.cursor_of(&editor.part(&project).unwrap()), Some(vec2(0.5, 0.5)));
    assert_eq!(project.undo_step(), steps, "putting the cursor down is no edit");
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0, 1, 2]), align: Some("top_left".into()), ..Default::default() });
    let after = layout_of(&project, node);
    for i in 0..3 {
        assert!((after.islands[i].min - vec2(0.5, 0.5)).length() < 1e-5, "island {i}: {:?} ({:?})", after.islands[i].min, editor.message);
    }
    assert_eq!(project.undo_step().map(|(name, _)| name), Some("uv.place_islands"));
    project.undo();
    assert_eq!(mesh_of(&project, node), original, "one step");
    let state = editor.state();
    assert_eq!(state.get("cursor").map(|c| c.as_f32_array()), Some(Some(vec![0.5, 0.5])));
}

/// Vertices line up by a side of the box round them, or onto the line
/// between the outermost for the middles; each picked edge path on its own.
#[test]
fn vertices_and_edge_paths_line_up() {
    let (mut project, node, _) = part_with(crooked_strip());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let top: Vec<usize> = (0..9).collect();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(top.clone()), align: Some("top".into()), ..Default::default() });
    let uv = mesh_of(&project, node).uv_sets[0].clone();
    let lowest = crooked_strip().uv_sets[0][..9].iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    assert!(top.iter().all(|&v| (uv[v].y - lowest).abs() < 1e-6), "{:?}", editor.message);
    project.undo();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(top.clone()), align: Some("middle".into()), ..Default::default() });
    let uv = mesh_of(&project, node).uv_sets[0].clone();
    let (a, b) = (uv[0], uv[8]);
    for &v in &top {
        let p = uv[v];
        assert!(((b - a).x * (p - a).y - (b - a).y * (p - a).x).abs() < 1e-6, "on the line between the outermost");
    }
    assert!(said(&editor).0.contains("line between the outermost"), "{:?}", editor.message);
    project.undo();
    // The strip's top and bottom, each a path, each lined up to its own
    // middle.
    let edges: Vec<[usize; 2]> = (0..8).map(|c| [c, c + 1]).chain((0..8).map(|c| [9 + c, 10 + c])).collect();
    ask(&mut editor, &mut project, &ctx, UvEditorChange { edges: Some(edges), align: Some("middle".into()), ..Default::default() });
    let uv = mesh_of(&project, node).uv_sets[0].clone();
    assert!((0..9).all(|v| (uv[v].y - uv[0].y).abs() < 1e-6) && (9..18).all(|v| (uv[v].y - uv[9].y).abs() < 1e-6), "{:?}", editor.message);
    assert!((uv[9].y - uv[0].y).abs() > 0.05, "each on its own, not to one line");
}

/// Shift and a right click put the cursor down where it was clicked; a
/// right click alone does not.
#[test]
fn a_shift_right_click_puts_the_cursor_down() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let at = editor.view.xform(editor.canvas_rect).to_screen(vec2(0.47, 0.53));
    let click = |editor: &mut UvEditor, project: &mut Project, modifiers: egui::Modifiers| {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers };
        for events in [vec![egui::Event::PointerMoved(at)], vec![press(true)], vec![press(false)], Vec::new()] {
            let _ = ctx.run(input(modifiers, events), |ctx| show(ctx, editor, project));
        }
    };
    click(&mut editor, &mut project, egui::Modifiers::NONE);
    assert_eq!(editor.cursors.of(node), None, "a right click opens the menu");
    click(&mut editor, &mut project, egui::Modifiers::SHIFT);
    let cursor = editor.cursors.of(node).expect("put down");
    assert!((cursor - vec2(0.47, 0.53)).length() < 0.03, "{cursor:?}");
    assert!(said(&editor).0.starts_with("2D cursor at"), "{:?}", editor.message);
}

/// Every Arrange tool is one step to undo, and says what it did.
#[test]
fn every_arrange_tool_is_one_step() {
    let tools: Vec<(&str, UvArrange)> = vec![
        ("upright", UvArrange { upright: true, ..Default::default() }),
        ("sort", UvArrange { sort: options(), ..Default::default() }),
        ("match size", UvArrange { match_size: options(), ..Default::default() }),
        ("fit", UvArrange { fit: options(), ..Default::default() }),
        ("shuffle", UvArrange { shuffle: true, ..Default::default() }),
        ("as on the part", UvArrange { as_on_part: options(), ..Default::default() }),
        ("spread", UvArrange { distribute: Some("across".into()), spread: Some("centres".into()), ..Default::default() }),
    ];
    for (name, change) in tools {
        let (mut project, node, _) = part_with(boxed());
        let ctx = egui::Context::default();
        let mut editor = editor_on(&mut project, node, &ctx);
        // Turned and shrunk first, so there is something to do.
        ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0, 1, 2, 3, 4, 5]), transform: Some(tequila_scripting::protocol::UvTransform { quarter: Some(1), ..Default::default() }), ..Default::default() });
        ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![2]), transform: Some(tequila_scripting::protocol::UvTransform { scale: Some([0.5, 0.5]), ..Default::default() }), ..Default::default() });
        ask(&mut editor, &mut project, &ctx, islands(&[0, 1, 2, 3, 4, 5]));
        let before = mesh_of(&project, node);
        let steps = project.undo_step();
        ask(&mut editor, &mut project, &ctx, arrange(change));
        let (text, refused) = said(&editor);
        assert!(!refused, "{name}: {text}");
        assert_ne!(mesh_of(&project, node), before, "{name}: {text}");
        assert!(layout_of(&project, node).folded_triangles().is_empty(), "{name}");
        project.undo();
        assert_eq!(project.undo_step(), steps, "{name}: one step");
        assert_eq!(mesh_of(&project, node), before, "{name}: undone");
    }
}

/// Stacked, a box's sides lie exactly over the one clicked; a shape copied
/// and pasted onto another gives it that shape where it is.
#[test]
fn stacking_and_pasting_give_islands_the_same_shape() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![3]), transform: Some(tequila_scripting::protocol::UvTransform { turn: Some(20.0), ..Default::default() }), ..Default::default() });
    // Island 0 clicked last: what the others are laid over.
    ask(&mut editor, &mut project, &ctx, islands(&[3, 0]));
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { stack: options(), ..Default::default() }));
    let after = layout_of(&project, node);
    assert!((after.islands[3].min - after.islands[0].min).length() < 1e-5 && (after.islands[3].max - after.islands[0].max).length() < 1e-5, "{:?}", editor.message);
    assert!(said(&editor).0.starts_with("Stacked 1 island on island 1"), "{:?}", editor.message);
    project.undo();
    // Copied from 0, pasted onto 5 where 5 is.
    let before = layout_of(&project, node);
    ask(&mut editor, &mut project, &ctx, islands(&[0]));
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { copy_shape: true, ..Default::default() }));
    assert!(editor.arrange.copied.is_some());
    ask(&mut editor, &mut project, &ctx, islands(&[5]));
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { paste_shape: options(), ..Default::default() }));
    let after = layout_of(&project, node);
    assert!((after.islands[5].centre - before.islands[5].centre).length() < 1e-5, "kept its place");
    let size = |l: &UvLayout, i: usize| l.islands[i].max - l.islands[i].min;
    assert!((size(&after, 5) - size(&before, 0)).length() < 1e-5, "took the shape");
    // A paste onto nothing of that piece is refused, and says why.
    let mut other = editor.arrange.copied.clone().unwrap();
    other.1.triangles.pop();
    editor.arrange.copied = Some(other);
    let steps = project.undo_step();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { paste_shape: options(), ..Default::default() }));
    let (text, refused) = said(&editor);
    assert!(refused && text.starts_with("Could not paste the shape"), "{text}");
    assert_eq!(project.undo_step(), steps);
}

/// The Select menu: islands like the selected, their borders, the edges
/// running across, and the faces turned over, picked on the part.
#[test]
fn the_select_menu_finds_alike_islands_borders_edges_and_flipped_faces() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    ask(&mut editor, &mut project, &ctx, islands(&[2]));
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { select_similar: Some(None), ..Default::default() }));
    assert_eq!(editing::islands(editor.layout.as_ref().map(|(_, _, l)| l).unwrap(), &editor.selection).len(), 6, "{:?}", editor.message);
    ask(&mut editor, &mut project, &ctx, islands(&[2]));
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { select_borders: true, ..Default::default() }));
    assert_eq!(editor.mode, Mode::Edges);
    assert_eq!(editor.selection.edges().len(), 4, "a side's four seams");
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { select_square: Some("across".into()), ..Default::default() }));
    assert_eq!(editor.selection.edges().len(), 2, "of them, the two across");
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { select_flipped: true, ..Default::default() }));
    assert_eq!(said(&editor), ("No face is turned over against its island".to_string(), false));
    // A face turned over: its corner pulled across the diagonal.
    let mut folded = crooked_strip();
    folded.uv_sets[0][9 + 4] = folded.uv_sets[0][4] + (folded.uv_sets[0][5] - folded.uv_sets[0][4]) * 1.6 + vec2(0.0, -0.02);
    let (mut project, node, _) = part_with(folded);
    let mut editor = editor_on(&mut project, node, &ctx);
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { select_flipped: true, ..Default::default() }));
    let picked: Vec<u32> = project.selection.faces(node).collect();
    assert!(!picked.is_empty(), "{:?}", editor.message);
    assert_eq!(picked, layout_of(&project, node).folded_triangles());
}

/// The density read off the selected islands in pixels per centimetre at
/// a texture size, and set from the field: twice as fine, twice as large.
#[test]
fn a_texel_density_is_read_and_set() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    ask(&mut editor, &mut project, &ctx, islands(&[1]));
    let density_change = |json: &str| arrange(UvArrange { density: Some(tequila_format::json::parse(json.as_bytes()).unwrap()), ..Default::default() });
    ask(&mut editor, &mut project, &ctx, density_change(r#"{"texture": 1024, "get": true}"#));
    let layout = layout_of(&project, node);
    let side = layout.islands[1].max.x - layout.islands[1].min.x;
    // The side is 20 cm of the part.
    let expected = side * 1024.0 / 20.0;
    assert!((editor.texel.field - expected).abs() < expected * 1e-3, "{} against {expected} ({:?})", editor.texel.field, editor.message);
    let field = editor.texel.field;
    ask(&mut editor, &mut project, &ctx, density_change(&format!(r#"{{"apply": {}}}"#, field * 2.0)));
    let after = layout_of(&project, node);
    assert!(((after.islands[1].max.x - after.islands[1].min.x) / side - 2.0).abs() < 1e-3, "{:?}", editor.message);
    let state = editor.state();
    let coverage = state.get("density").and_then(|d| d.get("coverage")).and_then(|c| c.as_f32());
    assert!(coverage.is_some_and(|c| c > 0.0 && c <= 1.0), "{coverage:?}");
}

/// The checker is for looking only: on, the viewport is handed the part's
/// instance to draw it on, and the document, the material and the history
/// are as they were.
#[test]
fn the_checker_changes_nothing_in_the_document_or_its_history() {
    let (mut project, node, material) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    let before = project.document().clone();
    let steps = project.undo_step();
    let definition = project.document().materials.get(material).cloned();
    assert!(editor.checker_frame(&project).is_none(), "off by default");
    let on = tequila_format::json::parse(br#"{"squares": 6, "pattern": "numbers"}"#).unwrap();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { checker: Some(on), ..Default::default() }));
    let frame = editor.checker_frame(&project).expect("on");
    assert_eq!(frame.instances, vec![project.mapping().instance_of(node).unwrap().0]);
    assert_eq!(frame.image.0 % 6, 0);
    frames(&mut editor, &mut project, &ctx, 3);
    assert!(project.document().describes_the_same_scene_as(&before), "nothing in the document");
    assert_eq!(project.undo_step(), steps, "nothing in the history");
    assert_eq!(project.document().materials.get(material).cloned(), definition, "the material untouched");
    assert!(!project.can_undo() || project.undo_step() == steps);
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { checker: Some(tequila_format::json::Value::Bool(false)), ..Default::default() }));
    assert!(editor.checker_frame(&project).is_none());
}

/// The checker follows the UV set the editor shows: on the first, the
/// viewport draws it through the set its meshes carry; on a second, it is
/// handed that set's places, to the graphics card and the processor alike;
/// a set picked holds whatever the editor shows. Nothing of it reaches the
/// document or the history.
#[test]
fn the_checker_follows_the_uv_set_shown() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    project.apply(tequila_scene::Command::AddUvSet { node }).unwrap();
    project.end_interaction();
    let before = project.document().clone();
    let steps = project.undo_step();
    let on = tequila_format::json::parse(br#"{"squares": 8}"#).unwrap();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { checker: Some(on), ..Default::default() }));
    let frame = editor.checker_frame(&project).expect("on");
    assert!(frame.uvs.is_none(), "the first set, which the scene's meshes carry");
    assert!(editor.cpu_checker(&project).is_some_and(|(_, c)| c.uvs.is_none()));

    ask(&mut editor, &mut project, &ctx, UvEditorChange { set: Some(1), ..Default::default() });
    let mesh = mesh_of(&project, node);
    let frame = editor.checker_frame(&project).expect("on");
    let (_, places) = frame.uvs.expect("the second set handed beside it");
    assert_eq!(*places, mesh.uv_sets[1]);
    let (key, checker) = editor.cpu_checker(&project).expect("on");
    assert_eq!(checker.uvs.as_deref(), Some(&mesh.uv_sets[1]));
    assert_eq!(editor.cpu_checker(&project).map(|(k, _)| k), Some(key), "made once, kept while nothing changes");

    // Picked: the first set, whichever the editor shows.
    let first = tequila_format::json::parse(br#"{"set": 0}"#).unwrap();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { checker: Some(first), ..Default::default() }));
    assert!(editor.checker_frame(&project).expect("on").uvs.is_none());
    assert_eq!(editor.state().get("checker").and_then(|c| c.get("set")).and_then(|s| s.as_index()), Some(0));
    let shown = tequila_format::json::parse(br#"{"set": "shown"}"#).unwrap();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { checker: Some(shown), behind: Some("checker".into()), ..Default::default() }));
    assert!(editor.checker_frame(&project).expect("on").uvs.is_some(), "following the editor again");
    frames(&mut editor, &mut project, &ctx, 3);
    assert_eq!(editor.state().get("behind").and_then(|b| b.as_str().map(str::to_string)), Some("checker".to_string()), "behind the layout too");
    assert!(project.document().describes_the_same_scene_as(&before), "nothing in the document");
    assert_eq!(project.undo_step(), steps, "nothing in the history");
}

/// Islands closer than the padding are counted, and Use for Pack gives
/// Pack the same margin.
#[test]
fn the_padding_marks_islands_too_close_and_feeds_pack() {
    let (mut project, node, _) = part_with(boxed());
    let ctx = egui::Context::default();
    let mut editor = editor_on(&mut project, node, &ctx);
    // Two sides pushed up against each other.
    let layout = layout_of(&project, node);
    let gap = layout.islands[1].min.x - layout.islands[0].max.x;
    let _ = gap;
    ask(&mut editor, &mut project, &ctx, UvEditorChange { islands: Some(vec![0, 1]), align: Some("top".into()), ..Default::default() });
    let padding = tequila_format::json::parse(br#"{"texture": 1024, "px": 40, "use_for_pack": true}"#).unwrap();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { padding: Some(padding), ..Default::default() }));
    assert!(editor.prefs.padding);
    assert_eq!((editor.pack.margin, editor.pack.texture), (40.0, 1024));
    let close = editor.state().get("padding").and_then(|p| p.get("too_close")).and_then(|c| c.as_index());
    assert!(close.is_some_and(|c| c >= 2), "{close:?}");
    let suggest = tequila_format::json::parse(br#"{"texture": 4096, "suggest": true}"#).unwrap();
    ask(&mut editor, &mut project, &ctx, arrange(UvArrange { padding: Some(suggest), ..Default::default() }));
    assert_eq!(editor.prefs.padding_px, 16.0);
}

/// A script's bad key is refused with what it may be.
#[test]
fn a_bad_arranging_key_says_what_it_may_be() {
    let (project, node, _) = part_with(boxed());
    let mut editor = UvEditor::default();
    editor.open_on(node);
    let error = editor.script(&arrange(UvArrange { align_to: Some("moon".into()), ..Default::default() }), &project).unwrap_err();
    assert!(error.contains("selection, island, layout or cursor"), "{error}");
    let sort = tequila_format::json::parse(br#"{"by": "colour"}"#).unwrap();
    assert!(editor.script(&arrange(UvArrange { sort: Some(sort), ..Default::default() }), &project).is_err());
}
