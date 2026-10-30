//! The UV editor: a part's surface laid out flat (`tequila_scene::uv`), and a
//! label placed on it by hand - dragged, scaled and turned where it prints,
//! or put square on an island of the layout in one click. What a planar
//! projection cannot do, a label on an angled face or round a fillet, is
//! done here.
//!
//! The layout is drawn as the texture sees it: `v` down, as an image's rows
//! are, so artwork placed square looks the way it does in a picture.

use crate::project::Project;
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2};
use tequila_materials::graph::TextureId;
use tequila_scene::{Command, MaterialKey, MeshKey, NodeId, NodeKind};
use tequila_texture::projection::{Mapping, Projection};

/// How much of an island a fitted label fills, so its edge does not run
/// onto the seam.
const FIT: f32 = 0.9;

/// Where the Labels tab leaves a request to open the editor on a label:
/// the material and the label's place, for the window to take next frame.
pub const OPEN: &str = "uv-editor-label";

pub struct UvEditor {
    pub open: bool,
    /// The label being placed: its material and its place among the
    /// material's labels.
    pub label: Option<(MaterialKey, usize)>,
    zoom: f32,
    pan: egui::Vec2,
    cone: f32,
    layout: Option<(MeshKey, usize, Islands)>,
    artwork: HashMap<TextureId, Option<egui::TextureHandle>>,
    island: Option<usize>,
    drag: Option<Drag>,
}

impl Default for UvEditor {
    fn default() -> Self {
        UvEditor {
            open: false,
            label: None,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            cone: tequila_scene::uv::UnwrapOptions::default().cone_degrees,
            layout: None,
            artwork: HashMap::new(),
            island: None,
            drag: None,
        }
    }
}

impl UvEditor {
    /// Open on a label, as the Labels tab's button asks.
    pub fn edit(&mut self, material: MaterialKey, label: usize) {
        self.open = true;
        self.label = Some((material, label));
        self.island = None;
        self.drag = None;
    }
}

#[derive(Clone, Copy, Debug)]
enum Grab {
    Move,
    Scale,
    Turn,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    grab: Grab,
    /// Where the pointer was, in the layout's coordinates.
    from: Vec2,
    original: Mapping,
    current: Mapping,
}

// ---- a label's placement on the layout ---------------------------------------

/// The mapping's scale as repeats per unit of the layout, whichever way the
/// mapping states it.
fn repeats(mapping: &Mapping) -> Vec2 {
    let (u, v) = mapping.repeats();
    vec2(u, v)
}

fn with_repeats(mut mapping: Mapping, repeats: Vec2) -> Mapping {
    let repeats = vec2(repeats.x.max(1e-4), repeats.y.max(1e-4));
    if mapping.real_world_units {
        mapping.scale.x = 1.0 / repeats.x;
        mapping.scale.y = 1.0 / repeats.y;
    } else {
        mapping.scale.x = repeats.x;
        mapping.scale.y = repeats.y;
    }
    mapping
}

/// Where a point of the layout lands on the artwork, as every renderer
/// works it out.
#[cfg(test)]
fn artwork_at(mapping: &Mapping, uv: Vec2) -> Vec2 {
    let (u, v) = mapping.apply_to(uv.x, uv.y);
    vec2(u, v)
}

/// The point of the layout a point of the artwork lands on.
fn layout_at(mapping: &Mapping, art: Vec2) -> Vec2 {
    let (u, v) = mapping.unapply(art.x, art.y);
    vec2(u, v)
}

/// The artwork's corners on the layout, from its top left round.
fn corners(mapping: &Mapping) -> [Vec2; 4] {
    [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)].map(|c| layout_at(mapping, c))
}

/// The point of the layout the artwork's middle is on.
fn centre(mapping: &Mapping) -> Vec2 {
    layout_at(mapping, vec2(0.5, 0.5))
}

/// The same placement with the artwork's middle at `at`, turning about its
/// middle: the pivot put there, as the Labels tab places a label.
fn centred(mut mapping: Mapping, at: Vec2) -> Mapping {
    // Placed by hand, it turns as a whole: a wide label turned stays wide.
    mapping.rigid = true;
    let s = repeats(&mapping);
    mapping.pivot = vec2(at.x * s.x, at.y * s.y);
    let middle = vec2(if mapping.flip_u { -0.5 } else { 0.5 }, if mapping.flip_v { -0.5 } else { 0.5 });
    mapping.offset = middle - mapping.pivot;
    mapping
}

/// The artwork's size on the layout.
fn size(mapping: &Mapping) -> Vec2 {
    let s = repeats(mapping);
    vec2(1.0 / s.x, 1.0 / s.y)
}

/// Its turn on the layout, in radians from a half turn back to a half turn
/// on, counter to the rotation field's.
fn turn(mapping: &Mapping) -> f32 {
    let half = std::f32::consts::PI;
    (-mapping.rotation * std::f32::consts::TAU + half).rem_euclid(2.0 * half) - half
}

fn turned(mut mapping: Mapping, angle: f32) -> Mapping {
    let at = centre(&mapping);
    mapping.rotation = (-angle / std::f32::consts::TAU).rem_euclid(1.0);
    centred(mapping, at)
}

fn resized(mapping: Mapping, to: Vec2) -> Mapping {
    let at = centre(&mapping);
    centred(with_repeats(mapping, vec2(1.0 / to.x.max(1e-5), 1.0 / to.y.max(1e-5))), at)
}

/// Inside the island, as large as it goes keeping the artwork's shape, in
/// the label's own turn, square on the island's middle.
fn fitted(mapping: Mapping, island: &[Vec2]) -> Mapping {
    let angle = turn(&mapping);
    let (sin, cos) = angle.sin_cos();
    // The island in the label's frame.
    let local: Vec<Vec2> = island.iter().map(|p| vec2(p.x * cos + p.y * sin, -p.x * sin + p.y * cos)).collect();
    let low = local.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
    let high = local.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
    let room = (high - low) * FIT;
    let shape = size(&mapping);
    let k = (room.x / shape.x.max(1e-9)).min(room.y / shape.y.max(1e-9));
    let middle = (low + high) * 0.5;
    let at = vec2(middle.x * cos - middle.y * sin, middle.x * sin + middle.y * cos);
    centred(with_repeats(mapping, vec2(1.0 / (shape.x * k).max(1e-5), 1.0 / (shape.y * k).max(1e-5))), at)
}

/// Square on the island's middle, as it is turned.
fn centred_on(mapping: Mapping, island: &[Vec2]) -> Mapping {
    let angle = turn(&mapping);
    let (sin, cos) = angle.sin_cos();
    let local: Vec<Vec2> = island.iter().map(|p| vec2(p.x * cos + p.y * sin, -p.x * sin + p.y * cos)).collect();
    let low = local.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
    let high = local.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
    let middle = (low + high) * 0.5;
    centred(mapping, vec2(middle.x * cos - middle.y * sin, middle.x * sin + middle.y * cos))
}

/// Turned to run along the island's length, the nearer of the two ways.
fn along(mapping: Mapping, axis: f32) -> Mapping {
    let now = turn(&mapping);
    let half = std::f32::consts::PI;
    let mut best = axis;
    for candidate in [axis, axis + half, axis - half] {
        let distance = |a: f32| ((a - now + half).rem_euclid(2.0 * half) - half).abs();
        if distance(candidate) < distance(best) {
            best = candidate;
        }
    }
    turned(mapping, best)
}

// ---- the layout's islands --------------------------------------------------------

/// A part's layout as the editor draws it: every edge once, which ones are
/// seams, and the islands they bound.
struct Islands {
    triangles: Vec<[Vec2; 3]>,
    island_of: Vec<u32>,
    edges: Vec<[Vec2; 2]>,
    seams: Vec<[Vec2; 2]>,
    points: Vec<Vec<Vec2>>,
    axes: Vec<f32>,
    /// Scene units of the part per unit of the layout, where the layout is
    /// as dense all over as an unwrap makes it.
    units_per_uv: f32,
}

impl Islands {
    fn of(mesh: &tequila_scene::Mesh, set: usize) -> Option<Islands> {
        let uv = mesh.uv_sets.get(set)?;
        if uv.len() != mesh.positions.len() {
            return None;
        }
        let count = mesh.triangle_count();
        // Islands: triangles joined by the vertices they share; an unwrap
        // splits its vertices along every seam.
        let mut parent: Vec<u32> = (0..mesh.positions.len() as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for t in 0..count {
            let [a, b, c] = mesh.triangle(t);
            for (p, q) in [(a, b), (b, c)] {
                let (rp, rq) = (find(&mut parent, p), find(&mut parent, q));
                if rp != rq {
                    parent[rp as usize] = rq;
                }
            }
        }
        let mut ids: HashMap<u32, u32> = HashMap::new();
        let mut island_of = Vec::with_capacity(count);
        let mut triangles = Vec::with_capacity(count);
        let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
        let (mut surface, mut flat) = (0.0f64, 0.0f64);
        for t in 0..count {
            let [a, b, c] = mesh.triangle(t);
            let root = find(&mut parent, a);
            let next = ids.len() as u32;
            island_of.push(*ids.entry(root).or_insert(next));
            let corners = [uv[a as usize], uv[b as usize], uv[c as usize]];
            triangles.push(corners);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                *uses.entry((p.min(q), p.max(q))).or_default() += 1;
            }
            let (pa, pb, pc) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
            surface += ((pb - pa).cross(pc - pa).length() * 0.5) as f64;
            let [ua, ub, uc] = corners;
            flat += (((ub.x - ua.x) * (uc.y - ua.y) - (ub.y - ua.y) * (uc.x - ua.x)) * 0.5).abs() as f64;
        }
        let mut edges = Vec::new();
        let mut seams = Vec::new();
        for (&(p, q), &n) in &uses {
            let edge = [uv[p as usize], uv[q as usize]];
            if n == 1 {
                seams.push(edge);
            } else {
                edges.push(edge);
            }
        }
        let mut points: Vec<Vec<Vec2>> = vec![Vec::new(); ids.len()];
        for (t, corners) in triangles.iter().enumerate() {
            points[island_of[t] as usize].extend_from_slice(corners);
        }
        let axes = points.iter().map(|p| principal_axis(p)).collect();
        let units_per_uv = if flat > 0.0 { (surface / flat).sqrt() as f32 } else { 0.0 };
        Some(Islands { triangles, island_of, edges, seams, points, axes, units_per_uv })
    }

    /// The island under a point of the layout.
    fn at(&self, uv: Vec2) -> Option<usize> {
        self.triangles.iter().position(|t| inside(uv, t)).map(|t| self.island_of[t] as usize)
    }
}

fn principal_axis(points: &[Vec2]) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let n = points.len() as f32;
    let mean = points.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
    let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
    for p in points {
        let d = *p - mean;
        xx += d.x * d.x;
        xy += d.x * d.y;
        yy += d.y * d.y;
    }
    0.5 * (2.0 * xy).atan2(xx - yy)
}

fn inside(p: Vec2, t: &[Vec2; 3]) -> bool {
    let side = |a: Vec2, b: Vec2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let (d1, d2, d3) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

// ---- the window -----------------------------------------------------------------

/// The part to show: the label's part, or the selected one.
fn part(project: &Project, label: Option<(MaterialKey, usize)>) -> Option<(NodeId, MeshKey)> {
    let document = project.document();
    let node = match label {
        Some((material, _)) => tequila_scene::decal_fit::part_of(document, material).map(|(node, _)| node),
        None => project.selection.active(),
    }?;
    match document.tree.get(node)?.kind {
        NodeKind::Part { mesh } => Some((node, mesh)),
        _ => None,
    }
}

pub fn show(ctx: &egui::Context, editor: &mut UvEditor, project: &mut Project) {
    if !editor.open {
        return;
    }
    let mut open = editor.open;
    egui::Window::new("UV editor")
        .open(&mut open)
        .default_size([720.0, 640.0])
        .min_size([420.0, 360.0])
        .show(ctx, |ui| body(ui, editor, project));
    editor.open = open;
}

fn body(ui: &mut egui::Ui, editor: &mut UvEditor, project: &mut Project) {
    // A label that is gone - removed, its material deleted - lets go.
    if let Some((material, index)) = editor.label {
        let still = project.document().materials.get(material).is_some_and(|m| index < m.graph.decals.len());
        if !still {
            editor.label = None;
        }
    }
    let Some((node, mesh_key)) = part(project, editor.label) else {
        ui.weak("Select a part, or open a label's UV editor from the Labels tab.");
        return;
    };
    let name = project.document().tree.get(node).map(|n| n.name.clone()).unwrap_or_default();
    let label = editor.label.and_then(|(material, index)| {
        project.document().materials.get(material).and_then(|m| m.graph.decals.get(index)).map(|d| (material, index, d.clone()))
    });
    let set = label.as_ref().map(|(_, _, d)| d.mapping.uv_set).unwrap_or(0);

    // The islands, worked out once a mesh and set.
    if editor.layout.as_ref().map(|(key, s, _)| (*key, *s)) != Some((mesh_key, set)) {
        editor.island = None;
        editor.layout = project.document().geometry.get(mesh_key).and_then(|mesh| Islands::of(mesh, set)).map(|i| (mesh_key, set, i));
    }

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&name).strong());
        if let Some((_, _, decal)) = &label {
            ui.label(egui::RichText::new(format!("· {}", decal.name)).color(crate::theme::dim()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let unwrap = ui
                .button("Unwrap")
                .on_hover_text("Lay the part's surface out flat as its first UV set: its faces apart at sharp edges, curved surfaces in a few even pieces. The set it had moves up one.");
            ui.add(egui::DragValue::new(&mut editor.cone).range(15.0..=85.0).suffix("°").speed(0.5))
                .on_hover_text("How far a surface may turn within one piece of the layout: smaller makes more, flatter pieces");
            if unwrap.clicked() {
                let options = tequila_scene::uv::UnwrapOptions { cone_degrees: editor.cone, ..Default::default() };
                match project.apply(Command::UnwrapUvs { node, options }) {
                    Ok(_) => project.status = format!("Unwrapped {name}"),
                    Err(error) => project.status = format!("Could not unwrap {name}: {error}"),
                }
                editor.layout = None;
            }
        });
    });

    let Some((_, _, islands)) = &editor.layout else {
        ui.add_space(8.0);
        ui.weak("This part has no UV layout yet. Unwrap it to lay its surface out flat, then place labels on it here.");
        return;
    };

    // Label tools.
    let mut change: Option<Mapping> = None;
    if let Some((_, _, decal)) = &label {
        ui.horizontal_wrapped(|ui| {
            if decal.mapping.projection != Projection::Uv {
                if ui
                    .button("Place it on the UV layout")
                    .on_hover_text("Print the label by the part's UV layout instead of projecting it, so it can sit on any face")
                    .clicked()
                {
                    let mut mapping = decal.mapping;
                    mapping.projection = Projection::Uv;
                    mapping.uv_set = 0;
                    mapping.real_world_units = false;
                    mapping.rotation = 0.0;
                    let biggest = (0..islands.points.len()).max_by(|&a, &b| islands.points[a].len().cmp(&islands.points[b].len()));
                    mapping = with_repeats(mapping, vec2(4.0, 4.0));
                    change = Some(match biggest {
                        Some(island) => fitted(mapping, &islands.points[island]),
                        None => centred(mapping, vec2(0.5, 0.5)),
                    });
                }
                ui.weak("This label is projected flat, not placed by the UV layout.");
            } else {
                let selected = editor.island.filter(|&i| i < islands.points.len());
                ui.add_enabled_ui(selected.is_some(), |ui| {
                    let island = selected.map(|i| (&islands.points[i], islands.axes[i]));
                    if ui.button("Fit to island").on_hover_text("As large as the island takes, keeping its shape").on_disabled_hover_text("Click an island of the layout first").clicked() {
                        if let Some((points, _)) = island {
                            change = Some(fitted(decal.mapping, points));
                        }
                    }
                    if ui.button("Centre on island").on_disabled_hover_text("Click an island of the layout first").clicked() {
                        if let Some((points, _)) = island {
                            change = Some(centred_on(decal.mapping, points));
                        }
                    }
                    if ui.button("Along the island").on_hover_text("Turned to run along the island's length").on_disabled_hover_text("Click an island of the layout first").clicked() {
                        if let Some((_, axis)) = island {
                            change = Some(along(decal.mapping, axis));
                        }
                    }
                });
                if ui.button("Square").on_hover_text("Not turned").clicked() {
                    change = Some(turned(decal.mapping, 0.0));
                }
                // The size, in the part's own units where the layout says.
                let current = size(&decal.mapping);
                if islands.units_per_uv > 0.0 {
                    let millimetres = |uv: f32| uv * islands.units_per_uv * 1000.0;
                    let mut width = millimetres(current.x);
                    ui.label("Width");
                    if ui
                        .add(egui::DragValue::new(&mut width).speed(0.1).range(0.1..=10_000.0).suffix(" mm").max_decimals(1))
                        .on_hover_text("The label's width on the part, keeping its shape")
                        .changed()
                    {
                        let k = width / millimetres(current.x).max(1e-6);
                        change = Some(resized(decal.mapping, current * k));
                    }
                }
            }
        });
    } else {
        ui.weak("Open a label's UV editor from the Labels tab to place it here.");
    }

    // The canvas.
    let available = ui.available_size();
    let side = available.x.min(available.y).max(120.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(available.x.max(120.0), available.y.max(120.0)), egui::Sense::click_and_drag());
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let before = editor.zoom;
            editor.zoom = (editor.zoom * (1.0 + scroll * 0.0015)).clamp(0.25, 64.0);
            // About the pointer, so what is under it stays there.
            if let Some(pointer) = response.hover_pos() {
                let origin = rect.center() + editor.pan;
                editor.pan += (pointer - origin) * (1.0 - editor.zoom / before);
            }
        }
    }
    if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
        editor.pan += response.drag_delta();
    }
    let scale = side * 0.9 * editor.zoom;
    let origin = rect.center() + editor.pan - egui::vec2(scale * 0.5, scale * 0.5);
    let to_screen = |p: Vec2| egui::pos2(origin.x + p.x * scale, origin.y + p.y * scale);
    let to_layout = |p: egui::Pos2| vec2((p.x - origin.x) / scale, (p.y - origin.y) / scale);

    let painter = ui.painter_at(rect);
    let palette = crate::theme::p();
    painter.rect_filled(rect, 0.0, palette.panel);
    // The unit square and a grid of tenths.
    let square = egui::Rect::from_min_max(to_screen(Vec2::ZERO), to_screen(Vec2::splat(1.0)));
    painter.rect_filled(square, 0.0, palette.canvas);
    for i in 1..10 {
        let f = i as f32 / 10.0;
        let stroke = egui::Stroke::new(1.0_f32, palette.line.gamma_multiply(0.5));
        painter.line_segment([to_screen(vec2(f, 0.0)), to_screen(vec2(f, 1.0))], stroke);
        painter.line_segment([to_screen(vec2(0.0, f)), to_screen(vec2(1.0, f))], stroke);
    }
    painter.rect_stroke(square, 0.0, egui::Stroke::new(1.0_f32, palette.line));

    // The label, under the layout's lines so they show through it.
    let shown = editor.drag.map(|d| d.current).or_else(|| label.as_ref().map(|(_, _, d)| d.mapping));
    let mut corners_on_screen = None;
    if let (Some(mapping), Some((_, _, decal))) = (shown, &label) {
        if mapping.projection == Projection::Uv {
            let quad = corners(&mapping).map(to_screen);
            corners_on_screen = Some(quad);
            let texture = artwork(&mut editor.artwork, project, decal.texture, ui.ctx());
            let mut mesh = match &texture {
                Some(texture) => egui::Mesh::with_texture(texture.id()),
                None => egui::Mesh::default(),
            };
            let tint = if texture.is_some() { egui::Color32::WHITE.gamma_multiply(0.9) } else { palette.accent.gamma_multiply(0.35) };
            for (corner, uv) in quad.iter().zip([egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)]) {
                mesh.vertices.push(egui::epaint::Vertex { pos: *corner, uv, color: tint });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
        }
    }

    // The selected island, filled.
    if let Some(island) = editor.island {
        let mut fill = egui::Mesh::default();
        let colour = palette.accent.gamma_multiply(0.25);
        for (t, corners) in islands.triangles.iter().enumerate() {
            if islands.island_of[t] as usize == island {
                let base = fill.vertices.len() as u32;
                for p in corners {
                    fill.colored_vertex(to_screen(*p), colour);
                }
                fill.add_triangle(base, base + 1, base + 2);
            }
        }
        painter.add(egui::Shape::mesh(fill));
    }
    // Every edge, faint, when there are few enough to see; the seams always.
    if islands.edges.len() <= 120_000 {
        let stroke = egui::Stroke::new(0.6_f32, palette.text_dim.gamma_multiply(0.45));
        for edge in &islands.edges {
            painter.line_segment([to_screen(edge[0]), to_screen(edge[1])], stroke);
        }
    }
    let seam = egui::Stroke::new(1.2_f32, palette.text);
    for edge in &islands.seams {
        painter.line_segment([to_screen(edge[0]), to_screen(edge[1])], seam);
    }

    // The label's handles: its outline, a corner to scale, a stalk to turn.
    let handle = 6.0;
    let mut turn_at = None;
    if let Some(quad) = corners_on_screen {
        let outline = egui::Stroke::new(1.5_f32, palette.accent);
        for i in 0..4 {
            painter.line_segment([quad[i], quad[(i + 1) % 4]], outline);
        }
        painter.rect_filled(egui::Rect::from_center_size(quad[2], egui::Vec2::splat(handle * 2.0)), 2.0, palette.accent);
        let top = quad[0] + (quad[1] - quad[0]) * 0.5;
        let middle = quad[0] + (quad[2] - quad[0]) * 0.5;
        let outward = (top - middle).normalized() * 24.0;
        let stalk = top + outward;
        painter.line_segment([top, stalk], outline);
        painter.circle_filled(stalk, handle, palette.accent);
        turn_at = Some(stalk);
    }

    // The pointer: grab a handle or the label, or pick an island.
    if response.drag_started_by(egui::PointerButton::Primary) {
        if let (Some(pointer), Some((_, _, decal))) = (response.interact_pointer_pos(), &label) {
            let at = to_layout(pointer);
            let near = |p: egui::Pos2| (p - pointer).length() <= handle * 2.0;
            let grab = match (corners_on_screen, turn_at) {
                (Some(quad), _) if near(quad[2]) => Some(Grab::Scale),
                (_, Some(stalk)) if near(stalk) => Some(Grab::Turn),
                (Some(quad), _) if inside(at, &[to_layout(quad[0]), to_layout(quad[1]), to_layout(quad[2])]) || inside(at, &[to_layout(quad[0]), to_layout(quad[2]), to_layout(quad[3])]) => Some(Grab::Move),
                _ => None,
            };
            if let Some(grab) = grab {
                editor.drag = Some(Drag { grab, from: at, original: decal.mapping, current: decal.mapping });
            }
        }
    }
    if let (Some(drag), Some(pointer)) = (editor.drag.as_mut(), response.interact_pointer_pos()) {
        let at = to_layout(pointer);
        let middle = centre(&drag.original);
        drag.current = match drag.grab {
            Grab::Move => centred(drag.original, middle + (at - drag.from)),
            Grab::Scale => {
                let k = (at - middle).length() / (drag.from - middle).length().max(1e-6);
                resized(drag.original, size(&drag.original) * k.max(0.01))
            }
            Grab::Turn => {
                let angle = |p: Vec2| (p.y - middle.y).atan2(p.x - middle.x);
                turned(drag.original, turn(&drag.original) + angle(at) - angle(drag.from))
            }
        };
    }
    if response.drag_stopped() {
        if let Some(drag) = editor.drag.take() {
            if drag.current != drag.original {
                change = Some(drag.current);
            }
        }
    }
    if response.clicked() {
        if let Some(pointer) = response.interact_pointer_pos() {
            editor.island = islands.at(to_layout(pointer));
        }
    }
    if editor.drag.is_some() {
        ui.ctx().request_repaint();
    }

    // One step to undo, the whole of an edit.
    if let (Some(mapping), Some((material, index, _))) = (change, &label) {
        if let Some(definition) = project.document().materials.get(*material) {
            let mut decals = definition.graph.decals.clone();
            if let Some(decal) = decals.get_mut(*index) {
                decal.mapping = mapping;
                if decal.is_valid() {
                    let _ = project.apply(Command::SetDecals { material: *material, decals });
                }
            }
        }
    }
}

/// The label's artwork as a texture to draw, read once: at most a thousand
/// pixels across, which is finer than the editor shows it.
fn artwork(cache: &mut HashMap<TextureId, Option<egui::TextureHandle>>, project: &Project, id: TextureId, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    if let Some(cached) = cache.get(&id) {
        return cached.clone();
    }
    let loaded = project.document().textures.get(id).and_then(|source| {
        let image = tequila_texture::image::Image::load_raw(&source.path).ok()?;
        let step = (image.width.max(image.height) as f32 / 1024.0).ceil().max(1.0) as usize;
        let (width, height) = (image.width.div_ceil(step), image.height.div_ceil(step));
        let mut pixels = Vec::with_capacity(width * height);
        let channel = |x: usize, y: usize, c: usize| image.data[(y * image.width + x) * image.channels + c.min(image.channels - 1)];
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = ((x * step).min(image.width - 1), (y * step).min(image.height - 1));
                let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let (r, g, b) = if image.channels >= 3 { (channel(sx, sy, 0), channel(sx, sy, 1), channel(sx, sy, 2)) } else { let l = channel(sx, sy, 0); (l, l, l) };
                let a = if image.channels == 4 || image.channels == 2 { channel(sx, sy, image.channels - 1) } else { 1.0 };
                pixels.push(egui::Color32::from_rgba_unmultiplied(byte(r), byte(g), byte(b), byte(a)));
            }
        }
        let image = egui::ColorImage { size: [width, height], pixels };
        Some(ctx.load_texture(format!("uv-editor-artwork-{}", id.0), image, egui::TextureOptions::LINEAR))
    });
    cache.insert(id, loaded.clone());
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label() -> Mapping {
        let mut mapping = Mapping::default();
        mapping.projection = Projection::Uv;
        with_repeats(mapping, vec2(4.0, 8.0))
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn the_artwork_lands_where_its_corners_say() {
        let mapping = turned(centred(label(), vec2(0.3, 0.6)), 0.7);
        for corner in [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0), vec2(0.5, 0.5)] {
            assert!(near(artwork_at(&mapping, layout_at(&mapping, corner)), corner), "{corner:?}");
        }
        assert!(near(centre(&mapping), vec2(0.3, 0.6)), "turning keeps its middle where it was");
        let flipped = Mapping { flip_u: true, ..mapping };
        let flipped = centred(flipped, vec2(0.25, 0.25));
        assert!(near(artwork_at(&flipped, vec2(0.25, 0.25)), vec2(0.5, 0.5)), "a flipped label's middle too");
    }

    #[test]
    fn a_label_moves_scales_and_turns_about_its_middle() {
        let mapping = centred(label(), vec2(0.5, 0.5));
        assert!(near(size(&mapping), vec2(0.25, 0.125)));
        let moved = centred(mapping, vec2(0.6, 0.4));
        assert!(near(centre(&moved), vec2(0.6, 0.4)));
        let bigger = resized(moved, vec2(0.5, 0.25));
        assert!(near(centre(&bigger), vec2(0.6, 0.4)) && near(size(&bigger), vec2(0.5, 0.25)));
        let quarter = turned(bigger, std::f32::consts::FRAC_PI_2);
        let [a, b, ..] = corners(&quarter);
        // The top edge, a half wide, now runs down the layout.
        assert!(near(b - a, vec2(0.0, 0.5)) || near(b - a, vec2(0.0, -0.5)), "{:?}", b - a);
    }

    #[test]
    fn a_label_fits_an_island_keeping_its_shape() {
        let island = [vec2(0.1, 0.1), vec2(0.5, 0.1), vec2(0.5, 0.3), vec2(0.1, 0.3)];
        let fitted = fitted(label(), &island);
        let s = size(&fitted);
        assert!((s.x / s.y - 2.0).abs() < 1e-3, "two wide to one high, as the artwork is");
        assert!((s.x - 0.4 * FIT).abs() < 1e-4 || (s.y - 0.2 * FIT).abs() < 1e-4, "{s:?}");
        assert!(near(centre(&fitted), vec2(0.3, 0.2)));
        // Along a tilted island, the nearer way round.
        let along = along(fitted, 0.1);
        assert!((turn(&along) - 0.1).abs() < 1e-4);
    }
}
